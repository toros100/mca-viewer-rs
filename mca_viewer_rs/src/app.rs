mod slint_conv;
mod slint_gen;

use mca_viewer_rs::block_id::BlockId;
use mca_viewer_rs::palette::BlockPalette;
use slint::ModelExt;
use slint::language::{KeyboardModifiers, PointerEventKind};
use slint::platform::PointerEventButton;
use slint::wgpu_30::wgpu;
use std::cell::RefMut;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::{cell::RefCell, collections::HashMap, error::Error, rc::Rc};
use vello::kurbo;
use vello::kurbo::Point;

use slint::{ComponentHandle, ModelRc, SharedString, ToSharedString, VecModel, Weak};

use vello::kurbo::Affine;

use mca_viewer_rs::region::{Region, RegionPosition, RegionRange, parse_regions_in_dir};
use mca_viewer_rs::renderer::{GpuTile, Renderer};
use mca_viewer_rs::worker::{self};
use mca_viewer_rs::worker::{ManagerHandle, RegionTile, RenderMode, RenderOptions, View};

#[derive(Clone)]
pub struct PaletteEntry {
    id: BlockId,
    name: SharedString,
    col: slint::Color,
}

#[derive(Clone)]
pub struct NotifyApp {
    window: Weak<slint_gen::MainWindow>,
    notified: Arc<AtomicBool>,
}

impl worker::Notify for NotifyApp {
    fn notify(&self) {
        self.notify();
    }
}
type Color = vello::peniko::color::AlphaColor<vello::peniko::color::Srgb>;

impl NotifyApp {
    pub fn new(window: Weak<slint_gen::MainWindow>) -> Self {
        Self {
            window,
            notified: Arc::default(),
        }
    }
    pub fn notify(&self) {
        if !self
            .notified
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            _ = self.window.upgrade_in_event_loop(|w| {
                w.global::<slint_gen::Notify>().invoke_notify();
            });
        }
    }

    fn clear(&self) {
        self.notified
            .swap(false, std::sync::atomic::Ordering::AcqRel);
    }
}

pub struct AppHandle {
    app: Rc<RefCell<App>>,
    window: slint_gen::MainWindow,
}

impl AppHandle {
    pub fn run(&self) -> anyhow::Result<()> {
        Ok(self.window.run()?)
    }

    pub fn borrow_mut(&self) -> RefMut<'_, App> {
        self.app.borrow_mut()
    }
}

pub fn new(num_workers: NonZeroUsize, palette: Arc<BlockPalette>) -> anyhow::Result<AppHandle> {
    let window = slint_gen::MainWindow::new()?;

    let app = App::new(&window, num_workers, palette);

    app.borrow().register_callbacks()?;
    app.borrow_mut().refresh_palette_model();

    Ok(AppHandle { app, window })
}

pub struct App {
    self_ref: std::rc::Weak<RefCell<Self>>,
    renderer: Option<Renderer>,
    window: Weak<slint_gen::MainWindow>,
    tiles: HashMap<RegionPosition, RegionTile<BlockPalette, GpuTile>>,
    render_options: RenderOptions,
    overlay_scene: vello::Scene,
    canvas_dims: (f32, f32),
    view_affine: kurbo::Affine,
    pan_enter: Option<kurbo::Point>,
    handle: Option<ManagerHandle<BlockPalette, GpuTile>>,
    view: View,
    notify: NotifyApp,
    regions: Arc<[Region]>,
    palette: Arc<BlockPalette>,
    num_workers: NonZeroUsize,
    needs_redraw: bool,
    show_region_grid: bool,
    min_scale_pow: i8,
    curr_scale_pow: i8,
    max_scale_pow: i8,
    pal_model_internal: Rc<VecModel<PaletteEntry>>,
}

impl App {
    fn new(
        window: &slint_gen::MainWindow,
        num_workers: NonZeroUsize,
        palette: Arc<BlockPalette>,
    ) -> Rc<RefCell<Self>> {
        let notify = NotifyApp::new(window.as_weak());
        Rc::new_cyclic(|e| {
            RefCell::new(Self {
                palette,
                self_ref: e.clone(),
                num_workers,
                renderer: None,
                render_options: RenderOptions::default(),
                regions: Arc::default(),
                view: View::EMPTY,
                handle: None,
                window: window.as_weak(),
                tiles: HashMap::default(),
                overlay_scene: vello::Scene::default(),
                canvas_dims: (1., 1.),
                view_affine: Affine::IDENTITY,
                pan_enter: None,
                notify,
                needs_redraw: true,
                show_region_grid: false,
                min_scale_pow: -4,
                max_scale_pow: 8,
                curr_scale_pow: 0,
                pal_model_internal: Default::default(),
            })
        })
    }
    fn refresh_palette_model(&mut self) {
        self.pal_model_internal.clear();
        self.pal_model_internal
            .extend(
                self.palette
                    .iter_entries()
                    .map(|(id, name, col)| PaletteEntry {
                        id,
                        name: String::from_utf8_lossy(name).to_shared_string(),
                        col: slint::Color::from_rgb_u8(col[0], col[1], col[2]),
                    }),
            );

        let m: ModelRc<PaletteEntry> = ModelRc::new(self.pal_model_internal.clone());

        let Some(w) = self.window.upgrade() else {
            return;
        };
        w.set_palette(ModelRc::new(m.map(|e| slint_gen::PaletteEntry {
            id: usize::from(e.id) as i32,
            name: e.name,
            color: e.col,
        })));
    }

    fn fix_palette(&mut self) {
        let mut pal = self.palette.deep_clone();
        pal.fix();
        self.palette = Arc::new(pal);
        self.refresh_palette_model();
        self.rerender();
    }

    fn dump_dyn_palette(&self) {
        let pal = self.palette.deep_clone();

        let dy = pal.interner.dyn_interner.read();

        let n = dy.bytes_to_id.len();
        println!("--- {n} entries ---");
        for (id, bs) in dy.id_to_bytes.iter() {
            let bs = String::from_utf8_lossy(bs);
            println!("[{id:?}] {bs}")
        }
    }

    pub fn update_render_options(&mut self, opt: RenderOptions) {
        if opt == self.render_options {
            return;
        };

        self.render_options = opt;
        if self.regions.is_empty() {
            return;
        };

        self.send_task(false);
        if let Some(w) = self.window.upgrade() {
            w.set_render_options(slint_gen::RenderOptions::from(opt));
        }
    }

    fn send_task(&mut self, bump_id: bool) {
        let Some(handle) = self.handle.as_mut() else {
            return;
        };

        handle.send_task(
            self.regions.clone(),
            self.render_options,
            self.view,
            self.palette.clone(),
            bump_id,
        );
    }

    pub fn handle_notification(&mut self) {
        let Some(handle) = self.handle.as_ref() else {
            return;
        };

        self.notify.clear();

        while let Ok(r) = handle.try_recv_result() {
            if let Ok(s) = r.inner
                && self.view.keep.contains(s.pos())
            {
                self.tiles.insert(s.pos(), s);
                self.needs_redraw = true;
                if let Some(w) = self.window.upgrade() {
                    w.window().request_redraw();
                }
            }
        }
    }

    fn set_world_position(&self, pos: kurbo::Point) {
        let Some(window) = self.window.upgrade() else {
            return;
        };

        let kurbo::Point { x, y } = pos;

        let block_x = x.floor() as i32;
        let block_z = y.floor() as i32;

        let region_x = block_x.div_euclid(512);
        let region_z = block_z.div_euclid(512);

        let block_x_in_region = block_x.rem_euclid(512);
        let block_z_in_region = block_z.rem_euclid(512);

        let pos = slint_gen::WorldPosition {
            block_x,
            block_z,
            region_x,
            region_z,
            block_x_in_region,
            block_z_in_region,
        };

        if let Some(t) = self.tiles.get(&RegionPosition::new(region_x, region_z))
            && let Some(b) = &t.block_map
        {
            let (id, bs) = b.get(block_x_in_region as usize, block_z_in_region as usize);

            let has_color = self.palette.has_color(id);

            if let Some(bytes) = bs {
                // could maybe keep these shared strings around to avoid re-allocating them each
                // time, but it probably doesnt matter at all
                let name = String::from_utf8_lossy(bytes).to_shared_string();
                window.set_block_at_cursor(slint_gen::BlockAtCursor { name, has_color });
            } else {
                window.set_block_at_cursor(slint_gen::BlockAtCursor {
                    name: "".into(),
                    has_color: false,
                });
            }
        }

        window.set_world_position(pos);
    }

    fn screen_to_world(&self, x: f32, y: f32) -> kurbo::Point {
        let screen_pos = kurbo::Point::from((x, y));
        self.view_affine.inverse() * screen_pos
    }

    fn current_scale(&self) -> f64 {
        // WARN: only works because
        let coeffs = self.view_affine.as_coeffs();
        let xs = coeffs[0];
        let ys = coeffs[3];
        debug_assert_eq!(xs, ys);
        xs
    }

    fn update_view(&mut self) {
        let Some(handle) = self.handle.as_ref() else {
            return;
        };

        let half_width = self.canvas_dims.0 / 2.;
        let half_height = self.canvas_dims.1 / 2.;
        let screen_center_world = self.screen_to_world(half_width, half_height);
        let screen_center_region_x = (screen_center_world.x.floor() as i32).div_euclid(512);
        let screen_center_region_z = (screen_center_world.y.floor() as i32).div_euclid(512);

        let tile_size_world = 512. * self.current_scale();

        let x_fit = (half_width as f64 / tile_size_world).ceil();
        let z_fit = (half_height as f64 / tile_size_world).ceil();

        let center = RegionRange::single(screen_center_region_x, screen_center_region_z);

        let visible = center.grow(x_fit as i32, z_fit as i32);
        let keep = center.grow((x_fit * 1.5) as i32, (z_fit * 1.5) as i32);

        let v = View { visible, keep };

        self.needs_redraw = true;

        if v != self.view {
            self.view = v;
            handle.send_view_update(v);
            self.tiles.retain(|pos, _| self.view.keep.contains(*pos));
        }
        if let Some(w) = self.window.upgrade() {
            w.window().request_redraw();
        }
    }

    fn handle_pointer_event(
        &mut self,
        kind: PointerEventKind,
        button: PointerEventButton,
        _: KeyboardModifiers,
        x: f32,
        y: f32,
    ) {
        let screen_pos = kurbo::Point::from((x, y));
        let world_pos = self.screen_to_world(x, y);

        self.set_world_position(world_pos);

        match (button, kind) {
            (PointerEventButton::Left, PointerEventKind::Down) => {
                self.pan_enter = Some(screen_pos);
            }

            (_, PointerEventKind::Move) if let Some(pan_enter_screen_pos) = self.pan_enter => {
                let diff = screen_pos - pan_enter_screen_pos;
                self.view_affine = self.view_affine.then_translate(diff);
                self.update_view();
                self.pan_enter = Some(screen_pos);
            }

            (_, PointerEventKind::Up | PointerEventKind::Cancel) => {
                self.pan_enter = None;
            }

            _ => {}
        }
    }

    fn next_frame(&mut self) -> Option<slint::Image> {
        if !self.needs_redraw {
            return None;
        }

        let r = self.renderer.as_mut()?;

        let (w, h, s) = {
            let window = self.window.upgrade()?;
            (
                self.canvas_dims.0,
                self.canvas_dims.1,
                window.window().scale_factor(),
            )
        };

        let rounded_translation = self.view_affine.translation().round();
        let rounded_affine = self.view_affine.with_translation(rounded_translation);

        // NOTE:
        // could prepare scene in world coordinates, then pass the affine into self.renderer.render
        // below to transform the entire scene without having to rebuild it (keep another scene,
        // reset that one, append the world coordinate scene with the affine).
        // but then the scene would have to contain more than what we know to be visible this frame,
        // and it might be harder to get everything snapped to the pixel grid at variable scale.

        self.overlay_scene.reset();

        if self.show_region_grid {
            for pos in self.tiles.keys() {
                if self.view.visible.contains(*pos) {
                    if self.curr_scale_pow > -2 {
                        for chunk_x in 0..32 {
                            for chunk_z in 0..32 {
                                let x_off = (chunk_x * 16) as f64;
                                let z_off = (chunk_z * 16) as f64;

                                let chunk_box_origin =
                                    pos.rect().origin() + kurbo::Vec2::new(x_off, z_off);

                                let chunk_box =
                                    kurbo::Rect::from_origin_size(chunk_box_origin, (16., 16.));

                                let r = rounded_affine.transform_rect_bbox(chunk_box)
                                    + kurbo::Vec2::splat(0.5);

                                self.overlay_scene.stroke(
                                    &kurbo::Stroke::new(1.0),
                                    Affine::IDENTITY,
                                    Color::from_rgba8(255, 255, 255, 50),
                                    None,
                                    &r,
                                );
                            }
                        }
                    }

                    // pixel alignment to get very nice crisp boxes
                    let r =
                        rounded_affine.transform_rect_bbox(pos.rect()) + kurbo::Vec2::splat(0.5);

                    let origin = r.origin() + kurbo::Vec2::splat(1.);
                    let size = (r.width() - 2., r.height() - 2.);
                    let r2 = kurbo::Rect::from_origin_size(origin, size);

                    self.overlay_scene.stroke(
                        &kurbo::Stroke::new(1.),
                        Affine::IDENTITY,
                        Color::from_rgba8(255, 255, 255, 150),
                        None,
                        &r2,
                    );

                    self.overlay_scene.stroke(
                        &kurbo::Stroke::new(1.),
                        Affine::IDENTITY,
                        Color::from_rgba8(0, 0, 0, 150),
                        None,
                        &r,
                    );
                }
            }
        }

        let tex = r.render(
            self.tiles
                .iter()
                .map(|(pos, tile)| (rounded_affine.transform_rect_bbox(pos.rect()), &tile.tile)),
            &self.overlay_scene,
            w,
            h,
            s,
        );

        Some(slint::Image::try_from(tex).unwrap())
    }

    fn init_renderer(
        &mut self,
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Result<(), Box<dyn Error>> {
        let renderer = Renderer::new(device, queue);
        let ctx = renderer.ctx();
        self.renderer = Some(renderer);

        let (handle, mut manager) =
            worker::Manager::new(self.notify.clone(), ctx, self.num_workers);

        self.handle = Some(handle);

        std::thread::spawn(move || {
            manager.run();
        });

        self.update_view();
        if !self.regions.is_empty() {
            self.send_task(false);
        }

        Ok(())
    }

    fn set_scale(&mut self, pow: i8, fixed_screen_point: Point) {
        debug_assert!(self.min_scale_pow < self.max_scale_pow);

        let p = pow.clamp(self.min_scale_pow, self.max_scale_pow);

        if self.curr_scale_pow == p {
            return;
        }

        self.curr_scale_pow = p;

        let f = 2f64.powi(p as i32);

        let cs = self.current_scale();
        self.view_affine = self
            .view_affine
            .then_scale_about(1. / cs, fixed_screen_point);
        self.view_affine = self.view_affine.then_scale_about(f, fixed_screen_point);
        self.update_view();

        if let Some(w) = self.window.upgrade() {
            w.window().request_redraw();
        }
    }

    fn rerender(&mut self) {
        println!("rendering {} regions", self.regions.len());
        self.tiles.clear();
        self.send_task(true);
    }

    fn increase_scale(&mut self, fixed_screen_point: Point) {
        self.set_scale(self.curr_scale_pow + 1, fixed_screen_point);
    }

    fn decrease_scale(&mut self, fixed_screen_point: Point) {
        self.set_scale(self.curr_scale_pow - 1, fixed_screen_point);
    }

    pub fn load_dir(&mut self, path: &std::path::Path) -> anyhow::Result<()> {
        let rs = parse_regions_in_dir(path)?;
        let regions = Arc::from_iter(rs.into_iter().map(|r| Region {
            x: r.x,
            z: r.z,
            path: r.path,
        }));

        self.regions = regions;

        Ok(())
    }

    fn reset_scale(&mut self) {
        self.view_affine = Affine::scale(1.).then_translate(self.view_affine.translation());
        self.update_view();
        if let Some(w) = self.window.upgrade() {
            w.window().request_redraw();
        }
    }

    pub fn register_callbacks(&self) -> anyhow::Result<()> {
        let window = self.window.unwrap();
        let app = self.self_ref.upgrade().unwrap();

        window.set_render_options(app.borrow().render_options.into());

        let notify = window.global::<slint_gen::Notify>();
        notify.on_notify({
            let app = app.clone();
            move || {
                app.borrow_mut().handle_notification();
            }
        });

        window.on_fix_palette({
            let app = app.clone();
            move || {
                app.borrow_mut().fix_palette();
            }
        });

        window.on_canvas_size_changed({
            let app = app.clone();
            move |w, h| {
                let mut app = app.borrow_mut();
                app.canvas_dims = (w, h);
                app.update_view();
            }
        });

        window.on_pointer_event({
            let app = app.clone();

            move |k, b, m, x, y| {
                app.borrow_mut().handle_pointer_event(k, b, m, x, y);
            }
        });

        window.on_render({
            let app = app.clone();
            move || {
                app.borrow_mut().rerender();
            }
        });

        window.on_change_slice_height({
            let app = app.clone();

            move |e| {
                let mut app = app.borrow_mut();

                let mut opt = app.render_options;

                if app.render_options.mode == RenderMode::Slice {
                    opt.slice_height =
                        (opt.slice_height as i32 + e).clamp(0, u16::MAX as i32) as u16;
                } else {
                    return;
                }
                app.update_render_options(opt);
            }
        });

        window.on_increase_scale({
            let app = app.clone();
            move |x, y, m| {
                let mut app = app.borrow_mut();
                let fixed = if m {
                    Point::from((x, y))
                } else {
                    Point::from((app.canvas_dims.0 / 2., app.canvas_dims.1 / 2.))
                };
                app.increase_scale(fixed);
            }
        });

        window.on_set_render_options({
            let app = app.clone();
            move |o| {
                let opt = RenderOptions::from(o);
                app.borrow_mut().update_render_options(opt);
            }
        });

        window.on_decrease_scale({
            let app = app.clone();
            move |x, y, m| {
                let mut app = app.borrow_mut();
                let fixed = if m {
                    Point::from((x, y))
                } else {
                    Point::from((app.canvas_dims.0 / 2., app.canvas_dims.1 / 2.))
                };
                app.decrease_scale(fixed);
            }
        });

        window.on_reset_scale({
            let app = app.clone();
            move || {
                app.borrow_mut().reset_scale();
            }
        });

        window.on_toggle_grid({
            let app = app.clone();
            let window = window.as_weak();
            move || {
                let mut app = app.borrow_mut();
                app.show_region_grid = !app.show_region_grid;
                app.needs_redraw = true;
                if let Some(w) = window.upgrade() {
                    w.window().request_redraw();
                }
            }
        });

        window.window().set_rendering_notifier({
            let app = app.clone();
            let window = window.as_weak();
            move |state, graphics_api| match state {
                slint::RenderingState::RenderingSetup => {
                    let slint::GraphicsAPI::WGPU30 { device, queue, .. } = graphics_api else {
                        panic!("unexpected graphics API (expected wgpu30)");
                    };

                    app.borrow_mut()
                        .init_renderer(device.clone(), queue.clone())
                        .unwrap();
                }
                slint::RenderingState::BeforeRendering => {
                    if let Some(window) = window.upgrade() {
                        let mut app = app.borrow_mut();
                        if let Some(img) = app.next_frame() {
                            app.needs_redraw = false;
                            window.set_canvas_image(img);
                        }
                    }
                }
                _ => {}
            }
        })?;

        Ok(())
    }
}
