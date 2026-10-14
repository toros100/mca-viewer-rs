mod slint_conv;
mod slint_gen;

use mca_viewer_rs::block_id::BlockId;
use mca_viewer_rs::palette::{BlockPalette, PaletteCtx};
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

use slint::{ComponentHandle, ModelRc, SharedString, ToSharedString, Weak};

use vello::kurbo::Affine;

use mca_viewer_rs::region::{Region, RegionPosition, RegionRange, parse_regions_in_dir};
use mca_viewer_rs::renderer::{GpuTile, Renderer};
use mca_viewer_rs::worker::{self};
use mca_viewer_rs::worker::{ManagerHandle, RegionTile, RenderMode, RenderOptions, View};

#[derive(Clone)]
pub struct PaletteEntry {
    id: BlockId,
    name: SharedString,
    new_color: slint::Color,
    color: slint::Color,
    is_mapped: bool,
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

    Ok(AppHandle { app, window })
}

#[derive(Default)]
struct BlockFilter {
    group: slint_gen::FilterGroup,
    status: slint_gen::FilterStatus,
    matcher: nucleo_matcher::Matcher,
    pattern: nucleo_matcher::pattern::Pattern,
    scratch: Vec<char>,
}

impl BlockFilter {
    fn set_pattern(&mut self, p: &str) {
        self.pattern = nucleo_matcher::pattern::Pattern::new(
            p,
            nucleo_matcher::pattern::CaseMatching::Ignore,
            nucleo_matcher::pattern::Normalization::Never,
            nucleo_matcher::pattern::AtomKind::Fuzzy,
        );
    }

    fn set_filter_group(&mut self, f: slint_gen::FilterGroup) {
        self.group = f;
    }

    fn set_filter_status(&mut self, s: slint_gen::FilterStatus) {
        self.status = s;
    }

    fn score(&mut self, e: &PaletteEntry) -> Option<u32> {
        match self.group {
            slint_gen::FilterGroup::Unmapped if e.is_mapped => return None,
            slint_gen::FilterGroup::Mapped if !e.is_mapped => return None,
            _ => {}
        }

        match self.status {
            slint_gen::FilterStatus::Modified if e.color == e.new_color => return None,
            slint_gen::FilterStatus::Unmodified if e.color != e.new_color => return None,
            _ => {}
        }

        self.pattern.score(
            nucleo_matcher::Utf32Str::new(&e.name, &mut self.scratch),
            &mut self.matcher,
        )
    }
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
    palette_model: Rc<PaletteModel>,
}

#[derive(Default)]
struct PaletteModelInner {
    filter: BlockFilter,
    entries: Vec<PaletteEntry>,
    indices: Vec<EntryRef>,
}

#[derive(Default)]
struct PaletteModel {
    inner: RefCell<PaletteModelInner>,

    // TODO: currently just calling notify.reset() on any change, could do dirty tracking
    notify: slint::ModelNotify,
}

struct EntryRef {
    idx: usize,
    score: u32,
}

impl PaletteModelInner {
    fn load(&mut self, it: impl Iterator<Item = PaletteEntry>) {
        let n = self.entries.len();
        for entry in it {
            if self.entries[..n]
                .binary_search_by_key(&entry.id, |e| e.id)
                .is_err()
            {
                self.entries.push(entry)
            }
        }
        self.entries.sort_by_key(|e| e.id);
        self.rescore();
    }

    fn discard_changes(&mut self) {
        for entry in self.entries.iter_mut() {
            entry.new_color = entry.color
        }
        self.rescore();
    }

    fn set_color(&mut self, id: BlockId, col: slint::Color) {
        if let Ok(idx) = self.entries.binary_search_by_key(&id, |e| e.id) {
            self.entries[idx].new_color = col;
        }
        self.rescore();
    }

    fn reset_color(&mut self, id: BlockId) {
        if let Ok(idx) = self.entries.binary_search_by_key(&id, |e| e.id) {
            self.entries[idx].new_color = self.entries[idx].color;
        }
        self.rescore();
    }

    fn set_pattern(&mut self, s: &str) {
        self.filter.set_pattern(s);
        self.rescore();
    }

    fn set_filter_group(&mut self, g: slint_gen::FilterGroup) {
        self.filter.set_filter_group(g);
        self.rescore();
    }

    fn set_filter_status(&mut self, s: slint_gen::FilterStatus) {
        self.filter.set_filter_status(s);
        self.rescore();
    }

    fn rescore(&mut self) {
        self.indices.clear();
        for (idx, e) in self.entries.iter_mut().enumerate() {
            let score = self.filter.score(e);
            if let Some(score) = score {
                self.indices.push(EntryRef { idx, score })
            }
        }

        self.indices
            .sort_by(|r1, r2| r2.score.cmp(&r1.score).then(r2.idx.cmp(&r1.idx)));
    }
}

impl slint::Model for PaletteModel {
    type Data = slint_gen::PaletteEntry;
    fn row_count(&self) -> usize {
        self.inner.borrow().indices.len()
    }
    fn row_data(&self, row: usize) -> Option<Self::Data> {
        let inner = self.inner.borrow();
        inner.indices.get(row).map(|i| {
            let e = &inner.entries[i.idx];

            slint_gen::PaletteEntry {
                color: e.new_color,
                id: (u16::from(e.id) as i32),
                name: e.name.clone(),
                is_dirty: e.new_color != e.color,
                is_mapped: e.is_mapped,
            }
        })
    }

    fn model_tracker(&self) -> &dyn slint::ModelTracker {
        &self.notify
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
                palette_model: Default::default(),
            })
        });
        app.borrow_mut().load_palette_model();
        app
    }
    fn load_palette_model(&mut self) {
        self.palette_model
            .inner
            .borrow_mut()
            .load(
                self.palette
                    .iter_entries()
                    .map(|(id, name, col, is_mapped)| {
                        let color = slint::Color::from_rgb_u8(col[0], col[1], col[2]);
                        // HACK: not nice
                        PaletteEntry {
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
        let mut pal = self.palette.deep_clone();
        pal.fix();
        self.palette = Arc::new(pal);
        self.load_palette_model();
        self.rerender(false);
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

    fn rerender(&mut self, bump_id: bool) {
        println!("rendering {} regions", self.regions.len());
        if bump_id {
            self.tiles.clear();
        }
        self.send_task(bump_id);
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
                app.borrow_mut().rerender(true);
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

        window.on_set_color({
            let app = app.clone();
            move |id, c| {
                // TODO: kind of annoying to deep clone the palette for every single color change
                //
                // could batch changes first, e.g. have a working copy (already have a slint
                // model), changes are written to that, and then separately committed to the real
                // palette with a single deep clone

                let id = if id <= 1 || id >= u16::MAX as i32 {
                    return;
                } else {
                    BlockId::from(id as u16)
                };

                let app = app.borrow_mut();

                app.palette_model.inner.borrow_mut().set_color(id, c);
                app.palette_model.notify.reset();
            }
        });

        window.on_apply_palette({
            let app = app.clone();
            move || {
                let new_palette = {
                    let app = app.borrow();
                    let mut p = app.palette.deep_clone();
                    let mut pp = app.palette_model.inner.borrow_mut();
                    for e in pp.entries.iter_mut() {
                        if e.color != e.new_color {
                            let c = [
                                e.new_color.red(),
                                e.new_color.green(),
                                e.new_color.blue(),
                                255,
                            ];
                            p.set_color(e.id, c);
                            e.color = e.new_color;
                            e.is_mapped = true;
                        }
                    }
                    p.fix();
                    p
                };
                app.borrow_mut().palette = Arc::new(new_palette);
                app.borrow_mut().rerender(false);
                app.borrow().palette_model.notify.reset();
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

                let app = app.borrow_mut();

                app.palette_model.inner.borrow_mut().reset_color(id);
                app.palette_model.notify.reset();
            }
        });

        window.on_set_search_pattern({
            let app = app.clone();
            move |s| {
                let app = app.borrow();
                app.palette_model.inner.borrow_mut().set_pattern(&s);
                app.palette_model.notify.reset();
            }
        });

        window.on_reload_palette({
            let app = app.clone();
            move || {
                app.borrow_mut().fix_palette();

                let app = app.borrow();
                app.palette_model
                    .inner
                    .borrow_mut()
                    .load(app.palette.iter_entries().map(|(id, name, c, is_mapped)| {
                        let color = slint::Color::from_rgb_u8(c[0], c[1], c[2]);
                        let name = String::from_utf8_lossy(name).to_shared_string();
                        PaletteEntry {
                            id,
                            name,
                            new_color: color,
                            color,
                            is_mapped,
                        }
                    }));

                app.palette_model.notify.reset();
            }
        });

        window.set_palette(ModelRc::new(self.palette_model.clone()));

        window.on_set_filter_group({
            let app = app.clone();
            move |f| {
                let app = app.borrow();
                app.palette_model.inner.borrow_mut().set_filter_group(f);

                app.palette_model.notify.reset();
            }
        });

        window.on_set_filter_status({
            let app = app.clone();
            move |s| {
                let app = app.borrow();
                app.palette_model.inner.borrow_mut().set_filter_status(s);
                app.palette_model.notify.reset();
            }
        });

        window.on_discard_palette({
            let app = app.clone();
            move || {
                let app = app.borrow();
                app.palette_model.inner.borrow_mut().discard_changes();
                app.palette_model.notify.reset();
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
