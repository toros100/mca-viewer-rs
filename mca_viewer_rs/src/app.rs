mod models;
mod slint_conv;
mod slint_gen;

use mca_viewer_rs::block_id::BlockId;
use mca_viewer_rs::palette::{BlockPalette, PaletteCtx};
use mca_viewer_rs::worlds_scan::{WorldInfo, scan_for_worlds};
use slint::language::{KeyboardModifiers, PointerEventKind};
use slint::platform::PointerEventButton;
use slint::wgpu_30::wgpu;
use slint_conv::ColorConv;
use std::cell::RefMut;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::range::RangeInclusive;
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
                w.window().request_redraw();
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
    #[expect(unused)]
    app: Rc<RefCell<App>>,
    window: slint_gen::MainWindow,
}

impl AppHandle {
    pub fn run(&self) -> anyhow::Result<()> {
        Ok(self.window.run()?)
    }

    #[expect(unused)]
    pub fn borrow_mut(&self) -> RefMut<'_, App> {
        self.app.borrow_mut()
    }
}

pub fn new(num_workers: NonZeroUsize, palette: Arc<BlockPalette>) -> anyhow::Result<AppHandle> {
    let window = slint_gen::MainWindow::new()?;

    let app = App::new(&window, num_workers, palette);

    app.borrow().register_callbacks()?;

    Ok(AppHandle { app, window })
}

pub struct TileWithId {
    tile: RegionTile<GpuTile>,
    task_id: usize,
}

pub struct App {
    self_ref: std::rc::Weak<RefCell<Self>>,
    renderer: Option<Renderer>,
    window: Weak<slint_gen::MainWindow>,
    tiles: HashMap<RegionPosition, TileWithId>,
    overlay_scene: vello::Scene,
    canvas_dims: (f32, f32),
    pan_enter: Option<kurbo::Point>,
    handle: Option<ManagerHandle<BlockPalette, GpuTile>>,
    notify: NotifyApp,
    active_world: Option<World>,
    palette: Arc<BlockPalette>,
    num_workers: NonZeroUsize,
    needs_redraw: bool,
    show_region_grid: bool,
    palette_model: Rc<models::PaletteModel>,
    worlds_model: Rc<models::WorldsModel>,
    dimensions_model: Rc<VecModel<SharedString>>,
    world_fd_open: bool,
}

pub struct Dimension {
    pub name: SharedString,
    pub y_range: std::range::RangeInclusive<i16>,
    pub opt: RenderOptions,
    pub regions: Arc<[Region]>,
    pub view_state: ViewState,
}

#[derive(Clone, Copy)]
pub struct ViewState {
    pub min_scale_pow: i8,
    pub curr_scale_pow: i8,
    pub max_scale_pow: i8,
    pub affine: kurbo::Affine,
    pub view: View,
}

impl ViewState {
    fn screen_to_world(&self, x: f32, y: f32) -> kurbo::Point {
        let screen_pos = kurbo::Point::from((x, y));
        self.affine.inverse() * screen_pos
    }

    fn current_scale(&self) -> f64 {
        // WARN: only works because i only use scaling and translation
        let coeffs = self.affine.as_coeffs();
        let xs = coeffs[0];
        let ys = coeffs[3];
        debug_assert_eq!(xs, ys);
        xs
    }

    fn set_scale(&mut self, pow: i8, fixed_screen_point: kurbo::Point) -> bool {
        let p = pow.clamp(self.min_scale_pow, self.max_scale_pow);

        if self.curr_scale_pow == p {
            return false;
        }

        self.curr_scale_pow = p;

        let f = 2f64.powi(p as i32);

        let cs = self.current_scale();
        self.affine = self.affine.then_scale_about(1. / cs, fixed_screen_point);
        self.affine = self.affine.then_scale_about(f, fixed_screen_point);
        true
    }
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            min_scale_pow: -4,
            max_scale_pow: 8,
            curr_scale_pow: 0,
            affine: Affine::IDENTITY,
            view: View::EMPTY,
        }
    }
}

pub struct World {
    #[expect(unused)]
    pub info: WorldInfo,
    pub dimensions: Vec<Dimension>,
    pub active_dimension_idx: Option<usize>,
}
impl World {
    fn active_dimension(&self) -> Option<&Dimension> {
        self.dimensions.get(self.active_dimension_idx?)
    }
    fn active_dimension_mut(&mut self) -> Option<&mut Dimension> {
        self.dimensions.get_mut(self.active_dimension_idx?)
    }
}

impl App {
    fn new(
        window: &slint_gen::MainWindow,
        num_workers: NonZeroUsize,
        palette: Arc<BlockPalette>,
    ) -> Rc<RefCell<Self>> {
        let notify = NotifyApp::new(window.as_weak());
        let app = Rc::new_cyclic(|e| {
            RefCell::new(Self {
                palette,
                self_ref: e.clone(),
                num_workers,
                renderer: None,
                handle: None,
                window: window.as_weak(),
                tiles: HashMap::default(),
                overlay_scene: vello::Scene::default(),
                canvas_dims: (1., 1.),
                pan_enter: None,
                notify,
                needs_redraw: true,
                show_region_grid: false,
                palette_model: Default::default(),
                worlds_model: Default::default(),
                dimensions_model: Default::default(),
                active_world: None,
                world_fd_open: false,
            })
        });
        app.borrow_mut().load_palette_model();
        app.borrow_mut()
            .set_worlds_directory("/home/tobias/.minecraft/".into());
        app
    }
    fn load_palette_model(&mut self) {
        self.palette_model.load(
            self.palette
                .iter_entries()
                .map(|(id, name, col, is_mapped)| {
                    let color = slint::Color::from_rgb_u8(col[0], col[1], col[2]);
                    // HACK: not nice
                    models::PaletteEntry {
                        id,
                        name: String::from_utf8_lossy(name).to_shared_string(),
                        color,
                        new_color: color,
                        is_mapped,
                    }
                }),
        );
    }

    fn fix_palette(&mut self) {
        // using Arc::make_mut to try to skip a deep clone on self.palette
        //
        // the worker threads only get Weak<BlockPalette> and try to upgrade it before actually
        // drawing a region, so if everything is "at rest", the wrong count will always be 1 and we
        // will be able to mutate the palette without the deep clone
        //
        // when this is hit while a worker does have a strong reference, Arc::make_mut will clone
        // the BlockPalette to a new allocation
        //
        // WARN: this can block because the impl of Clone on BlockPalette requires a read lock,
        // but that should be pretty rare and brief (will block only if a worker has a write
        // lock, which only happens when a previously unknown block identifier &[u8] is encountered
        // and interned)
        //
        let p = Arc::make_mut(&mut self.palette);
        p.fix();
        self.load_palette_model();
        self.rerender(false);
    }

    pub fn update_render_options(&mut self, opt: RenderOptions) {
        let Some(dim) = self
            .active_world
            .as_mut()
            .and_then(World::active_dimension_mut)
        else {
            return;
        };

        if opt == dim.opt {
            return;
        };

        dim.opt = opt;

        self.send_task();

        if let Some(w) = self.window.upgrade() {
            w.set_render_options(slint_gen::RenderOptions::from(opt));
        }
    }

    fn send_task(&mut self) {
        let Some(handle) = self.handle.as_mut() else {
            return;
        };

        if let Some(w) = self.active_world.as_ref()
            && let Some(d) = w.active_dimension()
        {
            let regions = d.regions.clone();

            handle.send_task(
                regions,
                d.opt,
                d.view_state.view,
                Arc::downgrade(&self.palette),
            );
        }
    }

    pub fn handle_notification(&mut self) {
        let Some(handle) = self.handle.as_ref() else {
            return;
        };

        self.notify.clear();

        let Some(d) = self.active_world.as_ref().and_then(World::active_dimension) else {
            // if no dimension set, any incoming thing is stale garbage
            // still has to be drained
            while handle.try_recv_result().is_ok() {}
            return;
        };

        while let Ok(r) = handle.try_recv_result() {
            if let Ok(s) = r.inner
                && d.view_state.view.keep.contains(s.pos())
            {
                if let Some(t) = self.tiles.get_mut(&s.pos()) {
                    if r.task_id == t.task_id {}
                    if t.task_id < r.task_id {
                        t.task_id = r.task_id;
                        t.tile = s;
                        self.needs_redraw = true
                    }
                } else {
                    self.tiles.insert(
                        s.pos(),
                        TileWithId {
                            tile: s,
                            task_id: r.task_id,
                        },
                    );
                    self.needs_redraw = true;
                }

                if self.needs_redraw
                    && let Some(w) = self.window.upgrade()
                {
                    w.window().request_redraw();
                }
            }
        }
    }

    fn select_world(&mut self, w: WorldInfo) {
        let mut dims = vec![];

        // (x=0, z=0) at screen center
        let affine = Affine::translate((
            (self.canvas_dims.0 / 2.) as f64,
            (self.canvas_dims.1 / 2.) as f64,
        ));

        if let Some(r) = w.overworld_regions.clone()
            && let Ok(it) = parse_regions_in_dir(&r.path)
        {
            dims.push(Dimension {
                regions: it.collect(),
                name: "Overworld".into(),
                view_state: ViewState {
                    affine,
                    ..Default::default()
                },
                y_range: RangeInclusive {
                    start: -64,
                    last: 384,
                },
                opt: RenderOptions::default(),
            });
        };

        if let Some(r) = w.nether_regions.clone()
            && let Ok(it) = parse_regions_in_dir(&r.path)
        {
            dims.push(Dimension {
                regions: it.collect(),
                // NOTE: "normal" mode in nether just shows the roof, probably not what you want
                opt: RenderOptions {
                    mode: RenderMode::Slice,
                    ..Default::default()
                },
                name: "The Nether".into(),
                y_range: RangeInclusive {
                    start: 0,
                    last: 127,
                },
                view_state: ViewState {
                    affine,
                    ..Default::default()
                },
            });
        };

        if let Some(r) = w.end_regions.clone()
            && let Ok(it) = parse_regions_in_dir(&r.path)
        {
            dims.push(Dimension {
                regions: it.collect(),
                view_state: ViewState {
                    affine,
                    ..Default::default()
                },
                name: "The End".into(),
                y_range: RangeInclusive {
                    start: 0,
                    last: 127,
                },
                opt: RenderOptions::default(),
            });
        };

        self.dimensions_model.clear();
        for d in dims.iter() {
            self.dimensions_model.push(d.name.clone());
        }

        let dim_idx = if !dims.is_empty() { 0 } else { -1 };

        let wo = World {
            info: w,
            active_dimension_idx: None,
            dimensions: dims,
        };

        self.active_world = Some(wo);
        self.select_dimension(dim_idx);
    }

    fn select_dimension(&mut self, i: i32) {
        let Some(win) = self.window.upgrade() else {
            return;
        };

        if let Some(w) = self.active_world.as_mut() {
            if i >= 0 && (i as usize) < w.dimensions.len() {
                w.active_dimension_idx = Some(i as usize);
                let d = &w.dimensions[i as usize];
                win.set_render_options(d.opt.into());
                win.set_dimension_choice(i);
                win.set_y_range(slint_gen::YRange {
                    max: d.y_range.last as i32,
                    min: d.y_range.start as i32,
                });
            } else {
                w.active_dimension_idx = None;
                win.set_dimension_choice(-1);
            }
            self.update_view(true);
            self.rerender(true);
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
            && let Some(b) = &t.tile.block_map
        {
            let id = b.get(block_x_in_region as usize, block_z_in_region as usize);
            let bs = self.palette.get_block_name(id);

            let has_color = self.palette.has_color(id);

            let color = self.palette.get_color(id);

            let c = slint::Color::from_rgb_u8(color[0], color[1], color[2]);

            if let Some(bytes) = bs {
                // could maybe keep these shared strings around to avoid re-allocating them each
                // time, but it probably doesnt matter at all
                let name = String::from_utf8_lossy(bytes).to_shared_string();

                let id = usize::from(id) as i32;

                window.set_block_at_cursor(slint_gen::BlockAtCursor {
                    name,
                    has_color,
                    color: c,
                    id,
                });
            } else {
                window.set_block_at_cursor(slint_gen::BlockAtCursor {
                    name: "".into(),
                    has_color: false,
                    color: c,
                    id: 0,
                });
            }
        }

        window.set_world_position(pos);
    }

    fn center_at(&mut self, world_position: kurbo::Point) {
        if let Some(d) = self
            .active_world
            .as_mut()
            .and_then(World::active_dimension_mut)
            .map(|d| &mut d.view_state)
        {
            let p = d.screen_to_world(self.canvas_dims.0 / 2., self.canvas_dims.1 / 2.);
            d.affine = d.affine.pre_translate(p - world_position);
        }
    }

    fn update_view(&mut self, send_update: bool) {
        let Some(handle) = self.handle.as_ref() else {
            return;
        };

        let Some(vs) = self
            .active_world
            .as_mut()
            .and_then(World::active_dimension_mut)
            .map(|d| &mut d.view_state)
        else {
            return;
        };

        let half_width = self.canvas_dims.0 / 2.;
        let half_height = self.canvas_dims.1 / 2.;

        let screen_center_world = vs.screen_to_world(half_width, half_height);
        let screen_center_region_x = (screen_center_world.x.floor() as i32).div_euclid(512);
        let screen_center_region_z = (screen_center_world.y.floor() as i32).div_euclid(512);

        let tile_size_world = 512. * vs.current_scale();

        let x_fit = (half_width as f64 / tile_size_world).ceil();
        let z_fit = (half_height as f64 / tile_size_world).ceil();

        let center = RegionRange::single(screen_center_region_x, screen_center_region_z);

        let visible = center.grow(x_fit as i32, z_fit as i32);
        let keep = center.grow((x_fit * 1.5) as i32, (z_fit * 1.5) as i32);

        let v = View { visible, keep };

        self.needs_redraw = true;

        if v != vs.view {
            vs.view = v;
            if send_update {
                handle.send_view_update(v);
            }
            self.tiles.retain(|pos, _| vs.view.keep.contains(*pos));
        }
        if let Some(w) = self.window.upgrade() {
            w.window().request_redraw();
        }
    }

    fn set_color(&mut self, id: BlockId, color: slint::Color) {
        // WARN: see comment in App::fix_palette
        let pal = Arc::make_mut(&mut self.palette);
        self.palette_model.set_color_if_id_present(id, color);
        pal.set_color_if_id_present(id, color.conv_into());
        self.palette_model.notify_all();
        self.rerender(false);
    }

    fn handle_pointer_event(
        &mut self,
        kind: PointerEventKind,
        button: PointerEventButton,
        _: KeyboardModifiers,
        x: f32,
        y: f32,
    ) {
        let Some(vs) = self
            .active_world
            .as_ref()
            .and_then(World::active_dimension)
            .map(|d| d.view_state)
        else {
            return;
        };

        let screen_pos = kurbo::Point::from((x, y));
        let world_pos = vs.screen_to_world(x, y);

        self.set_world_position(world_pos);

        match (button, kind) {
            (PointerEventButton::Left, PointerEventKind::Down) => {
                self.pan_enter = Some(screen_pos);
            }

            (_, PointerEventKind::Move) if let Some(pan_enter_screen_pos) = self.pan_enter => {
                if let Some(dim) = self
                    .active_world
                    .as_mut()
                    .and_then(World::active_dimension_mut)
                {
                    // NOTE: slightly ugly to have the pan enter live in App but apply to whichever
                    // dimension is active, but letting each dimension have its own pan enter is probably
                    // overkill

                    let diff = screen_pos - pan_enter_screen_pos;
                    dim.view_state.affine = dim.view_state.affine.then_translate(diff);

                    self.update_view(true);
                    self.pan_enter = Some(screen_pos);
                }
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

        let Some(dim) = self.active_world.as_ref().and_then(World::active_dimension) else {
            self.needs_redraw = false;
            return None;
        };

        let r = self.renderer.as_mut()?;

        let (w, h, s) = {
            let window = self.window.upgrade()?;
            (
                self.canvas_dims.0,
                self.canvas_dims.1,
                window.window().scale_factor(),
            )
        };

        let rounded_translation = dim.view_state.affine.translation().round();
        let rounded_affine = dim.view_state.affine.with_translation(rounded_translation);

        // NOTE:
        // could prepare scene in world coordinates, then pass the affine into self.renderer.render
        // below to transform the entire scene without having to rebuild it (keep another scene,
        // reset that one, append the world coordinate scene with the affine).
        // but then the scene would have to contain more than what we know to be visible this frame,
        // and it might be harder to get everything snapped to the pixel grid at variable scale.

        self.overlay_scene.reset();

        if self.show_region_grid {
            for pos in self.tiles.keys() {
                if dim.view_state.view.visible.contains(*pos) {
                    if dim.view_state.curr_scale_pow > -2 {
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
            self.tiles.iter().map(|(pos, tile)| {
                (
                    rounded_affine.transform_rect_bbox(pos.rect()),
                    &tile.tile.tile,
                )
            }),
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

        Ok(())
    }

    fn set_scale(&mut self, pow: i8, fixed_screen_point: Point) {
        let Some(vs) = self
            .active_world
            .as_mut()
            .and_then(World::active_dimension_mut)
            .map(|d| &mut d.view_state)
        else {
            return;
        };

        if !vs.set_scale(pow, fixed_screen_point) {
            return;
        };

        self.update_view(true);

        if let Some(w) = self.window.upgrade() {
            w.window().request_redraw();
        }
    }

    fn rerender(&mut self, clear: bool) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        if clear {
            self.tiles.clear();
        }
        self.needs_redraw = true;
        self.send_task();
        w.window().request_redraw();
    }

    fn increase_scale(&mut self, fixed_screen_point: Point) {
        let Some(curr_scale_pow) = self
            .active_world
            .as_ref()
            .and_then(World::active_dimension)
            .map(|d| d.view_state.curr_scale_pow)
        else {
            return;
        };

        self.set_scale(curr_scale_pow.saturating_add(1), fixed_screen_point);
    }

    fn decrease_scale(&mut self, fixed_screen_point: Point) {
        let Some(curr_scale_pow) = self
            .active_world
            .as_ref()
            .and_then(World::active_dimension)
            .map(|d| d.view_state.curr_scale_pow)
        else {
            return;
        };

        self.set_scale(curr_scale_pow.saturating_sub(1), fixed_screen_point);
    }

    fn reset_scale(&mut self) {
        let Some(vs) = self
            .active_world
            .as_mut()
            .and_then(World::active_dimension_mut)
            .map(|d| &mut d.view_state)
        else {
            return;
        };

        vs.affine = Affine::scale(1.).then_translate(vs.affine.translation());

        self.update_view(true);
        if let Some(w) = self.window.upgrade() {
            w.window().request_redraw();
        }
    }

    fn set_worlds_directory(&mut self, path: PathBuf) {
        let ws = scan_for_worlds(path, 3);

        println!("found {} worlds", ws.len());

        self.worlds_model.load(ws);
        self.worlds_model.notify_all();
    }

    pub fn register_callbacks(&self) -> anyhow::Result<()> {
        let window = self.window.unwrap();
        let app = self.self_ref.upgrade().unwrap();

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
                if app.canvas_dims != (w, h) {
                    app.canvas_dims = (w, h);
                    app.update_view(true);
                }
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
                app.borrow_mut().rerender(true);
            }
        });

        window.on_set_slice_height({
            let app = app.clone();
            move |h| {
                let mut app = app.borrow_mut();

                let Some(dim) = app.active_world.as_ref().and_then(World::active_dimension) else {
                    return;
                };

                let mut opt = dim.opt;

                if opt.mode == RenderMode::Slice {
                    opt.slice_height =
                        h.clamp(dim.y_range.start as i32, dim.y_range.last as i32) as i16;
                } else {
                    return;
                }
                app.update_render_options(opt);
            }
        });

        window.on_change_slice_height({
            let app = app.clone();

            move |e| {
                let mut app = app.borrow_mut();

                let Some(dim) = app.active_world.as_ref().and_then(World::active_dimension) else {
                    return;
                };

                let mut opt = dim.opt;

                if opt.mode == RenderMode::Slice {
                    opt.slice_height = ((opt.slice_height as i32).saturating_add(e))
                        .clamp(dim.y_range.start as i32, dim.y_range.last as i32)
                        as i16;
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

        window.on_set_color({
            let app = app.clone();
            move |id, c| {
                let id = if id <= 1 || id >= u16::MAX as i32 {
                    return;
                } else {
                    BlockId::from(id as u16)
                };

                app.borrow_mut().set_color(id, c);
            }
        });

        // TODO: this should be "save palette" (and offer a way to save to file)
        window.on_apply_palette({
            let app = app.clone();
            move || {
                app.borrow_mut().palette_model.apply_changes(|_, _| {});
                app.borrow().palette_model.notify_all();
            }
        });

        window.on_reset_color({
            let app = app.clone();
            move |id| {
                let id = if id <= 1 || id >= u16::MAX as i32 {
                    return;
                } else {
                    BlockId::from(id as u16)
                };

                let mut app = app.borrow_mut();

                if let Some(c) = app.palette_model.reset_color(id) {
                    app.set_color(id, c);
                    app.palette_model.notify_all();
                }
            }
        });

        window.on_set_search_pattern({
            let app = app.clone();
            move |s| {
                let app = app.borrow();
                app.palette_model.set_pattern(&s);
                app.palette_model.notify_all();
            }
        });

        window.on_reload_palette({
            let app = app.clone();
            move || {
                app.borrow_mut().fix_palette();

                let app = app.borrow();
                app.palette_model.load(app.palette.iter_entries().map(
                    |(id, name, c, is_mapped)| {
                        let color = c.conv_into();
                        let name = String::from_utf8_lossy(name).to_shared_string();
                        models::PaletteEntry {
                            id,
                            name,
                            new_color: color,
                            color,
                            is_mapped,
                        }
                    },
                ));

                app.palette_model.notify_all();
            }
        });

        window.set_palette(ModelRc::new(self.palette_model.clone()));
        window.set_worlds(ModelRc::new(self.worlds_model.clone()));

        window.on_select_world({
            let app = app.clone();
            move |i| {
                if i < 0 {
                    return;
                }
                let i = i as usize;

                let mut app = app.borrow_mut();

                let Some(world) = app.worlds_model.get(i) else {
                    return;
                };

                app.select_world(world);
                app.rerender(true);
            }
        });

        window.on_set_filter_group({
            let app = app.clone();
            move |f| {
                let app = app.borrow();
                app.palette_model.set_filter_group(f);
                app.palette_model.notify_all();
            }
        });

        window.on_set_filter_status({
            let app = app.clone();
            move |s| {
                let app = app.borrow();
                app.palette_model.set_filter_status(s);
                app.palette_model.notify_all();
            }
        });

        window.on_discard_palette({
            let app = app.clone();
            move || {
                let app = app.borrow();
                app.palette_model.discard_changes();
                app.palette_model.notify_all();
            }
        });

        window.on_pick_dir({
            let app = app.clone();
            let window = window.as_weak();
            move || {
                let app = app.clone();
                {
                    let mut app = app.borrow_mut();
                    if app.world_fd_open {
                        return;
                    }
                    app.world_fd_open = true;
                }

                slint::spawn_local({
                    let app = app.clone();
                    let window = window.clone();
                    async move {
                        let Some(window_handle) =
                            window.upgrade().map(|w| w.window().window_handle())
                        else {
                            return;
                        };
                        let res = rfd::AsyncFileDialog::new()
                            .set_parent(&window_handle)
                            .pick_folder()
                            .await;

                        let mut app = app.borrow_mut();
                        app.world_fd_open = false;

                        let Some(h) = res else { return };

                        let path = h.path().to_owned();
                        app.set_worlds_directory(path);
                    }
                })
                .unwrap();
            }
        });

        window.set_dimensions(ModelRc::new(self.dimensions_model.clone()));

        window.on_select_dimension({
            let app = app.clone();
            move |i| {
                app.borrow_mut().select_dimension(i);
            }
        });

        window.on_center_origin({
            let app = app.clone();
            move || {
                app.borrow_mut().center_at((0., 0.).into());
                app.borrow_mut().update_view(true);
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
