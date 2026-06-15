use std::{borrow::Cow, cell::OnceCell, time::Duration};

use egui::{Rect, emath::GuiRounding};
use tracing::info;

use crate::{
    Region, RenderMode, RenderOptions, RenderResult, TileRendererHandle, TileSpaceRectangle, View,
    get_interner, init_tile_renderer,
};

// (adapted from https://github.com/emilk/eframe_template/ )
// (will clean up later)

/// We derive Deserialize/Serialize so we can persist app state on shutdown.

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)] // if we add new fields, give them default values when deserializing old state
pub struct App {
    scale: f32,

    show_region_zx: bool,

    boxes: bool,

    screen_center_region: Option<(i32, i32)>,

    #[serde(skip)]
    render_rect: Option<egui::Rect>,

    #[serde(skip)]
    view: View,

    #[serde(skip)]
    renderer_handle: OnceCell<TileRendererHandle>,

    #[serde(skip)]
    tiles: std::collections::HashMap<(i32, i32), RenderResult>,

    offset: egui::Vec2,

    #[serde(skip)]
    regions: Option<Vec<Region>>,

    #[serde(skip)]
    hover_pos: Option<egui::Pos2>,

    #[serde(skip)]
    box_texture: Option<egui::TextureHandle>,

    #[serde(skip)]
    render_opts: RenderOptions,

    #[serde(skip)]
    mode_update: bool,

    #[serde(skip)]
    curr_block: Option<Cow<'static, str>>,
}

fn tight_view(screen_center_region: (i32, i32)) -> View {
    let (x, z) = screen_center_region;
    let vis = TileSpaceRectangle::new(x - 5..=x + 5, z - 5..=z + 5);
    let keep = TileSpaceRectangle::new(x - 7..=x + 7, z - 7..=z + 7);
    View::new(vis, keep)
}

impl Default for App {
    fn default() -> Self {
        Self {
            screen_center_region: Some((0, 0)),
            view: tight_view((0, 0)),
            renderer_handle: OnceCell::new(),
            render_rect: None,
            boxes: false,
            scale: 2.0,
            show_region_zx: false,
            offset: egui::vec2(0., 0.),
            tiles: std::collections::HashMap::new(),
            regions: Some(Vec::new()),
            hover_pos: None,
            box_texture: None,
            render_opts: RenderOptions::default(),
            mode_update: false,
            curr_block: None,
        }
    }
}

impl App {
    /// Called once before the first frame.
    pub fn new(cc: &eframe::CreationContext<'_>, regions: Vec<Region>) -> Self {
        // This is also where you can customize the look and feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Load previous app state (if any).
        // Note that you must enable the `persistence` feature for this to work.
        let mut a: App = if let Some(storage) = cc.storage {
            eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default()
        } else {
            Default::default()
        };

        a.regions = Some(regions);

        a
    }
    fn update_view(&mut self) -> bool {
        if let Some(ref render_rect) = self.render_rect
            && let Some(ref scr) = self.screen_center_region
        {
            // clamping above zero here because technically the width/height can be negative, that would produce
            // nonsense values here, and i don't care to investigate when exactly it would be
            // negative for this specicic usage
            let world_width = render_rect.width().max(0.) / self.scale;
            let world_height = render_rect.height().max(0.) / self.scale;

            let w_fit = (0.5 * world_width / 512.0).ceil() as i32;
            let h_fit = (0.5 * world_height / 512.0).ceil() as i32;

            let (c_x, c_y) = scr;

            let vis = TileSpaceRectangle::new(
                (c_x - w_fit)..=(c_x + w_fit),
                (c_y - h_fit)..=(c_y + h_fit),
            );
            let keep = vis.clone();

            let new_view = View::new(vis, keep);

            if !self.view.eq(&new_view) {
                self.view = new_view;
                true
            } else {
                false
            }
        } else {
            false
        }
    }
}

impl eframe::App for App {
    /// Called by the framework to save state before shutdown.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    /// Called each time the UI needs repainting, which may be many times per second.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let _ = self.renderer_handle.get_or_init(|| {
            init_tile_renderer(
                ui.ctx().clone(),
                std::thread::available_parallelism().unwrap().get(),
            )
        });

        // Put your widgets into a `SidePanel`, `TopBottomPanel`, `CentralPanel`, `Window` or `Area`.
        // For inspiration and more examples, go to https://emilk.github.io/egui
        //

        if ui.input_mut(|r| r.consume_key(egui::Modifiers::NONE, egui::Key::F1)) {
            match self.render_opts.mode {
                RenderMode::Normal => self.render_opts.mode = RenderMode::Slice(100),
                RenderMode::Slice(_) => self.render_opts.mode = RenderMode::Normal,
            }
            self.mode_update = true;
        };

        if ui.input_mut(|r| r.consume_key(egui::Modifiers::NONE, egui::Key::F2)) {
            self.render_opts.depth_darken = !self.render_opts.depth_darken;
            self.mode_update = true;
        };

        if ui.input_mut(|r| r.consume_key(egui::Modifiers::NONE, egui::Key::F3)) {
            self.render_opts.with_block_map = !self.render_opts.with_block_map;
            self.mode_update = true;
        };

        if ui.input_mut(|r| r.consume_key(egui::Modifiers::NONE, egui::Key::F4)) {
            let mut timings: Vec<std::time::Duration> =
                self.tiles.values().map(|r| r.dur).collect();
            timings.sort();
            let median_idx = timings.len() / 2;
            if !timings.is_empty() {
                let median_dur = timings[median_idx];
                info!("median of {} durations: {:.3?}", timings.len(), median_dur);
            }

            let total = timings.iter().fold(Duration::ZERO, |acc, x| acc + *x);
            info!("total: {:.3?}", total);
        }

        // if ui.input_mut(|r| r.consume_key(egui::Modifiers::NONE, egui::Key::F5)) {
        //     let int = get_interner();
        //     let r = int.dyn_interner.read().unwrap();
        //     println!("STATIC LOOKUP");
        //     for (&bs, idx) in int.static_interner.bs_to_idx.iter() {
        //         println!(
        //             "[{}]: {}",
        //             u16::from(BlockId::from(*idx)),
        //             String::from_utf8_lossy(bs)
        //         )
        //     }
        //     println!("----------------");
        //     println!("DYN LOOKUP");
        //     for (&bs, idx) in r.bs_to_idx.iter() {
        //         println!(
        //             "[{}]: {}",
        //             u16::from(BlockId::from(*idx)),
        //             String::from_utf8_lossy(bs)
        //         )
        //     }
        // };

        if self.box_texture.is_none() {
            let mut img = image::RgbaImage::new(512, 512);
            let pixel = image::Rgba::from([255, 0, 0, 100]);
            for i in 0..512 {
                img.put_pixel(i, 0, pixel);
                img.put_pixel(i, 1, pixel);
                img.put_pixel(i, 511, pixel);
                img.put_pixel(i, 510, pixel);
                img.put_pixel(0, i, pixel);
                img.put_pixel(1, i, pixel);
                img.put_pixel(510, i, pixel);
                img.put_pixel(511, i, pixel);
            }

            let img = egui::ColorImage::from_rgba_unmultiplied([512, 512], img.as_raw());
            let t = ui
                .ctx()
                .load_texture("debug_box", img, egui::TextureOptions::NEAREST);
            self.box_texture = Some(t)
        }

        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            ui.send_viewport_cmd(egui::ViewportCommand::Close);
        };

        egui::Panel::top("top_panel").show_inside(ui, |ui| {
            // The top panel is often a good place for a menu bar:

            egui::MenuBar::new().ui(ui, |ui| {
                // NOTE: no File->Quit on web pages!
                let is_web = cfg!(target_arch = "wasm32");
                if !is_web {
                    ui.menu_button("File", |ui| {
                        if ui.button("Quit").clicked() {
                            ui.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    ui.add_space(16.0);
                }

                egui::widgets::global_theme_preference_buttons(ui);
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let renderer = self.renderer_handle.get().unwrap();

            while let Some(res) = renderer.try_recv() {
                if self.view.keep(res.region_x, res.region_z) {
                    self.tiles.insert((res.region_x, res.region_z), res);
                    ui.ctx().request_repaint();
                }
            }

            let raw_scroll_delta = ui.input(raw_scroll_delta);

            ui.separator();

            if let Some(rs) = self.regions.take() {
                _ = renderer.render_regions(rs)
            }

            ui.horizontal(|ui| {
                ui.label(if let Some(p) = self.hover_pos {
                    format!("cursor block: {:.0}/{:.0}", p.x, p.y)
                } else {
                    "cursor xz: ---".into()
                });

                let cursor_reg_loc = self.hover_pos.map(|p| {
                    (
                        // WARN: need to round properly here lol
                        // need to correct for pixel size and round towards center of pixel
                        (p.x - 512. * (p.x).div_euclid(512.0)).floor() as i32,
                        (p.y - 512. * (p.y).div_euclid(512.0)).floor() as i32,
                    )
                });

                let cursor_reg = self.hover_pos.map(|p| {
                    (
                        p.x.div_euclid(512.0).floor() as i32,
                        p.y.div_euclid(512.0).floor() as i32,
                    )
                });

                let p = if let Some((x, y)) = cursor_reg_loc
                    && let Some(rp) = cursor_reg
                    && let Some(r) = self.tiles.get(&rp)
                    && let Some(ref lookup) = r.lookup
                {
                    let ux = x.clamp(0, 511) as usize;
                    let uy = y.clamp(0, 511) as usize;
                    let idx = ux + 512 * uy;

                    let id = lookup.get(ux, uy);
                    let name_bytes = get_interner().get_bytes(id);
                    Some((name_bytes, idx))
                } else {
                    None
                };

                let b = match p {
                    Some((c, _)) => c.map(String::from_utf8_lossy),
                    None => None,
                };

                self.curr_block = b;

                ui.label(if let Some(p) = self.hover_pos {
                    format!(
                        "cursor region: {}/{}",
                        (p.x.round() as i32).div_euclid(512),
                        (p.y.round() as i32).div_euclid(512)
                    )
                } else {
                    "cursor region: ---".into()
                });

                ui.label(if let Some(p) = cursor_reg_loc {
                    format!("region local: {}/{}", p.0, p.1)
                } else {
                    "region local: ---".into()
                });

                ui.label(format!("scale: {:.3}", self.scale));

                ui.label(format!(
                    "screen center region: {:?}",
                    self.screen_center_region
                ));

                if ui.button("toggle region pos").clicked() {
                    self.show_region_zx = !self.show_region_zx
                }

                if ui.button("toggle boxes").clicked() {
                    self.boxes = !self.boxes
                }
            });

            let (render_rect, response) =
                ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
            self.render_rect = Some(render_rect);
            let painter = ui.painter_at(render_rect);
            self.offset += response.drag_delta();

            // TODO: mapping of cursor pos into world space is slightly bogged
            // have to think about it and do it properly

            self.hover_pos = ui.input(|i| {
                i.pointer
                    .hover_pos()
                    .map(|p| (p - self.offset) / self.scale)
                    .map(|p| p.round_to_pixel_center(ui.pixels_per_point() * self.scale))
            });

            self.screen_center_region = Some({
                let screen_center_x = render_rect.width() / 2.0 + render_rect.min.x;
                let screen_center_y = render_rect.height() / 2.0 + render_rect.min.y;

                let world_center_x = (screen_center_x - self.offset.x) / self.scale;
                let world_center_y = (screen_center_y - self.offset.y) / self.scale;

                (
                    ((world_center_x.signum() * world_center_x.abs().ceil()) as i32)
                        .div_euclid(512),
                    ((world_center_y.signum() * world_center_y.abs().ceil()) as i32)
                        .div_euclid(512),
                )
            });

            if raw_scroll_delta.y != 0.0 {
                if ui.input(|r| r.modifiers.command) {
                    if let RenderMode::Slice(ref mut h) = self.render_opts.mode {
                        if raw_scroll_delta.y < 0.0 {
                            *h = h.saturating_add(1)
                        } else {
                            *h = h.saturating_sub(1)
                        }
                        self.mode_update = true;
                    }
                } else {
                    // WARN: this should NOT change which world position is at the "screen" center
                    let screen_center_x = render_rect.width() / 2.0 + render_rect.min.x;
                    let screen_center_y = render_rect.height() / 2.0 + render_rect.min.y;

                    let world_center_x = (screen_center_x - self.offset.x) / self.scale;
                    let world_center_y = (screen_center_y - self.offset.y) / self.scale;
                    let scale_fac = if raw_scroll_delta.y > 0.0 { 2. } else { 0.5 };

                    self.scale = (self.scale * scale_fac).clamp(0.0625, 8.0);

                    let o_x = screen_center_x - world_center_x * self.scale;
                    let o_y = screen_center_y - world_center_y * self.scale;

                    self.offset = egui::vec2(o_x, o_y);
                }
            }

            if self.update_view() {
                // HACK: i do not like this
                self.tiles.retain(|p, _| self.view.keep(p.0, p.1));
                _ = self
                    .renderer_handle
                    .get()
                    .unwrap()
                    .update_view(self.view.clone());
            }

            for ((x, y), handle) in self.tiles.iter() {
                let pos = egui::pos2(*x as f32 * 512. * self.scale, *y as f32 * 512. * self.scale)
                    + self.offset.round();

                let t_rect = egui::Rect::from_min_size(
                    pos,
                    egui::vec2(512. * self.scale, 512. * self.scale),
                );

                let t_uv = egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.));
                painter.image(handle.tex_handle.id(), t_rect, t_uv, egui::Color32::WHITE);

                if self.boxes
                    && let Some(ref t) = self.box_texture
                {
                    painter.image(t.id(), t_rect, t_uv, egui::Color32::WHITE);
                }

                if self.show_region_zx {
                    painter.text(
                        pos + egui::vec2(0.5 * 512. * self.scale, 0.5 * 512. * self.scale),
                        egui::Align2::CENTER_CENTER,
                        format!("({}, {})", x, y),
                        egui::FontId::proportional(24.0 * self.scale),
                        egui::Color32::WHITE,
                    );
                }
            }

            if let Some(ref a) = self.curr_block {
                let layout = painter.layout_no_wrap(
                    a.to_string(),
                    egui::FontId::proportional(24.0),
                    egui::Color32::WHITE,
                );

                let pos = egui::pos2(10., 70.);
                let rect = Rect::from_min_size(pos, layout.size()).expand(12.0);
                painter.rect_filled(rect, 0., egui::Color32::from_black_alpha(180u8));
                painter.galley(pos, layout, egui::Color32::WHITE);

                // _ = painter.text(
                //     egui::pos2(20., 60.),
                //     egui::Align2::LEFT_TOP,
                //     a,
                //     egui::FontId::proportional(24.0),
                //     egui::Color32::B,
                // )
            };
        });

        if self.mode_update {
            _ = self
                .renderer_handle
                .get()
                .unwrap()
                .update_render_options(self.render_opts.clone());
            self.mode_update = false;
        }
    }
}

fn raw_scroll_delta(input_state: &egui::InputState) -> egui::Vec2 {
    // HACK: (?) just ignoring other fields in the event, might not make sense for touch controls
    // i want this so that we can use the sign of the y scroll to modify the scale/zoom level in
    // discrete steps.
    // this is probably not a good idea in general, even ignoring touch controls, because i am
    // pretty sure there are mice that do more continuous scrolling (not just one "event" per detent
    // on the mouse wheel, if there even are physical detents at all).
    input_state
        .events
        .iter()
        .filter_map(|e| {
            if let egui::Event::MouseWheel { delta, .. } = e {
                Some(delta)
            } else {
                None
            }
        })
        .fold(egui::Vec2::ZERO, |acc, v| acc + *v)
}
