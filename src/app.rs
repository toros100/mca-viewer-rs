use std::collections::HashMap;

use crate::{Region, render_mca};

// (adapted from https://github.com/emilk/eframe_template/ )
// (will clean up later)

/// We derive Deserialize/Serialize so we can persist app state on shutdown.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)] // if we add new fields, give them default values when deserializing old state
pub struct App {
    scale: f32,

    show_region_zx: bool,

    boxes: bool,

    #[serde(skip)]
    texture_handle: Option<egui::TextureHandle>,

    #[serde(skip)]
    tiles: HashMap<(i32, i32), egui::TextureHandle>,

    offset: egui::Vec2,

    #[serde(skip)]
    render_results_tx: crossbeam_channel::Sender<RenderResult>,

    #[serde(skip)]
    render_results_rx: crossbeam_channel::Receiver<RenderResult>,

    #[serde(skip)]
    render_tasks: Vec<Region>,

    #[serde(skip)]
    hover_pos: Option<egui::Pos2>,

    #[serde(skip)]
    box_texture: Option<egui::TextureHandle>,
}

pub struct RenderResult {
    pub region_x: i32,
    pub region_z: i32,
    pub img: Option<image::RgbaImage>,
}

impl Default for App {
    fn default() -> Self {
        let (render_results_tx, render_results_rx) = crossbeam_channel::bounded(100);

        Self {
            boxes: false,
            scale: 2.0,
            show_region_zx: false,
            texture_handle: None,
            offset: egui::vec2(0., 0.),
            tiles: HashMap::new(),
            render_results_rx,
            render_results_tx,
            render_tasks: Vec::new(),
            hover_pos: None,
            box_texture: None,
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

        a.render_tasks = regions;

        a
    }
}

impl eframe::App for App {
    /// Called by the framework to save state before shutdown.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    /// Called each time the UI needs repainting, which may be many times per second.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Put your widgets into a `SidePanel`, `TopBottomPanel`, `CentralPanel`, `Window` or `Area`.
        // For inspiration and more examples, go to https://emilk.github.io/egui
        //

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
            while let Ok(r) = self.render_results_rx.try_recv() {
                if let Some(img) = r.img {
                    let handle = ui.ctx().load_texture(
                        format!("r.{}.{}.mca", r.region_x, r.region_z),
                        egui::ColorImage::from_rgba_premultiplied([512, 512], img.as_raw()),
                        egui::TextureOptions::NEAREST,
                    );
                    self.tiles.insert((r.region_x, r.region_z), handle);
                }
            }

            let raw_scroll_delta = ui.input(raw_scroll_delta);

            ui.separator();

            ui.horizontal(|ui| {
                if ui.button("test render").clicked() {
                    while let Some(t) = self.render_tasks.pop() {
                        let res_tx = self.render_results_tx.clone();
                        std::thread::spawn(move || -> anyhow::Result<()> {
                            let f = std::fs::File::open(t.path)?;

                            let img = render_mca(f)?;

                            let res = RenderResult {
                                region_x: t.x,
                                region_z: t.z,
                                img: Some(img),
                            };

                            res_tx.try_send(res)?;

                            Ok(())
                        });
                    }
                }

                ui.label(if let Some(p) = self.hover_pos {
                    format!("cursor block: {:.0}/{:.0}", p.x, p.y)
                } else {
                    "cursor xz: ---".into()
                });

                ui.label(if let Some(p) = self.hover_pos {
                    format!(
                        "cursor region: {}/{}",
                        (p.x.round() as i32).div_euclid(512),
                        (p.y.round() as i32).div_euclid(512)
                    )
                } else {
                    "cursor region: ---".into()
                });

                ui.label(format!("scale: {:.2}", self.scale));

                if ui.button("toggle region pos").clicked() {
                    self.show_region_zx = !self.show_region_zx
                }

                if ui.button("toggle boxes").clicked() {
                    self.boxes = !self.boxes
                }
            });

            let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
            self.offset += response.drag_delta();

            self.hover_pos = ui.input(|i| {
                i.pointer
                    .hover_pos()
                    .map(|p| (p - self.offset) / self.scale)
            });

            if raw_scroll_delta.y != 0.0 {
                let screen_center_x = rect.width() / 2.0 + rect.min.x;
                let screen_center_y = rect.height() / 2.0 + rect.min.y;

                let world_center_x = (screen_center_x - self.offset.x) / self.scale;
                let world_center_y = (screen_center_y - self.offset.y) / self.scale;
                if raw_scroll_delta.y > 0.0 {
                    self.scale = (self.scale + 0.5).clamp(0.5, 4.0)
                } else if raw_scroll_delta.y < 0.0 {
                    self.scale = (self.scale - 0.5).clamp(0.5, 4.0)
                }

                let o_x = screen_center_x - world_center_x * self.scale;
                let o_y = screen_center_y - world_center_y * self.scale;

                self.offset = egui::vec2(o_x, o_y);
            }
            let painter = ui.painter_at(rect);
            for ((x, y), handle) in self.tiles.iter() {
                let pos = egui::pos2(*x as f32 * 512. * self.scale, *y as f32 * 512. * self.scale)
                    + self.offset.round();

                let t_rect = egui::Rect::from_min_size(
                    pos,
                    egui::vec2(512. * self.scale, 512. * self.scale),
                );

                let t_uv = egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.));
                painter.image(handle.id(), t_rect, t_uv, egui::Color32::WHITE);

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
        });
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
