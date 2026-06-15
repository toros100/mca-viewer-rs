use std::{
    ops::RangeInclusive,
    sync::{Arc, Barrier, atomic::AtomicBool},
};

use crate::{ColorLookup, block_map::PackedBlockMap, render_slice};

use crossbeam_channel::{RecvError, TryRecvError, select};
use tracing::{error, info};

use crate::{Region, loader, render_region};

#[derive(Debug, Clone)]
pub struct View {
    visible: TileSpaceRectangle,
    keep: TileSpaceRectangle,
    overflow: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum RenderMode {
    #[default]
    Normal,
    Slice(u16),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderOptions {
    pub mode: RenderMode,
    pub depth_darken: bool,
    pub with_block_map: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            mode: RenderMode::default(),
            depth_darken: true,
            with_block_map: false,
        }
    }
}

impl View {
    pub fn new(visible: TileSpaceRectangle, keep: TileSpaceRectangle) -> Self {
        Self {
            visible,
            keep,
            overflow: false,
        }
    }

    // HACK: ugly
    pub fn eq(&self, other: &View) -> bool {
        self.visible.eq(&other.visible) && self.keep.eq(&other.keep)
    }

    pub fn keep(&self, x: i32, z: i32) -> bool {
        self.keep.contains(TileSpacePos { x, z })
    }

    fn empty() -> Self {
        Self {
            visible: TileSpaceRectangle::empty(),
            keep: TileSpaceRectangle::empty(),
            overflow: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TileSpaceRectangle {
    x_range: RangeInclusive<i32>,
    z_range: RangeInclusive<i32>,
}

#[derive(Eq, Hash, PartialEq, Clone, Copy, Debug)]
struct TileSpacePos {
    x: i32,
    z: i32,
}

impl TileSpaceRectangle {
    pub fn new(x_range: RangeInclusive<i32>, z_range: RangeInclusive<i32>) -> Self {
        Self { x_range, z_range }
    }

    fn empty() -> Self {
        Self {
            x_range: -1..=0,
            z_range: -1..=0,
        }
    }

    // HACK: ugly
    fn eq(&self, other: &TileSpaceRectangle) -> bool {
        self.x_range.start() == other.x_range.start()
            && self.x_range.end() == other.x_range.end()
            && self.z_range.start() == other.z_range.start()
            && self.z_range.end() == other.z_range.end()
    }

    fn contains(&self, pos: TileSpacePos) -> bool {
        self.x_range.contains(&pos.x) && self.z_range.contains(&pos.z)
    }
}

pub struct TileRendererHandle {
    view_update_tx: crossbeam_channel::Sender<View>,
    view_update_rx: crossbeam_channel::Receiver<View>,
    regions_update_tx: crossbeam_channel::Sender<Vec<Region>>,
    regions_update_rx: crossbeam_channel::Receiver<Vec<Region>>,
    render_result_rx: crossbeam_channel::Receiver<RenderResult>,
    options_update_tx: crossbeam_channel::Sender<RenderOptions>,
    options_update_rx: crossbeam_channel::Receiver<RenderOptions>,
    renderer_busy: Arc<AtomicBool>,
}

pub enum TileRendererError {
    Dead,
    Busy,
}

impl TileRendererHandle {
    fn is_busy(&self) -> bool {
        self.renderer_busy
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn try_recv(&self) -> Option<RenderResult> {
        self.render_result_rx.try_recv().ok()
    }

    pub fn update_view(&self, mut view: View) -> Result<(), TileRendererError> {
        // this will never actually loop more than once because there will be just one producer

        loop {
            match self.view_update_tx.try_send(view) {
                Ok(()) => break Ok(()),
                Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                    break Err(TileRendererError::Dead);
                }
                Err(crossbeam_channel::TrySendError::Full(v)) => {
                    view = v;
                    while self.view_update_rx.try_recv().is_ok() {}
                    view.overflow = true;
                }
            }
        }
    }

    pub fn update_render_options(&self, mut opt: RenderOptions) -> Result<(), TileRendererError> {
        loop {
            match self.options_update_tx.try_send(opt) {
                Ok(()) => break Ok(()),
                Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                    break Err(TileRendererError::Dead);
                }
                Err(crossbeam_channel::TrySendError::Full(o)) => {
                    opt = o;
                    while self.options_update_rx.try_recv().is_ok() {}
                }
            }
        }
    }

    pub fn render_regions(&self, mut regions: Vec<Region>) -> Result<(), TileRendererError> {
        if self.is_busy() {
            return Err(TileRendererError::Busy);
        }

        loop {
            match self.regions_update_tx.try_send(regions) {
                Ok(()) => break Ok(()),
                Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                    break Err(TileRendererError::Dead);
                }
                Err(crossbeam_channel::TrySendError::Full(r)) => {
                    regions = r;
                    while self.regions_update_rx.try_recv().is_ok() {}
                }
            }
        }
    }

    #[allow(unused)]
    fn stop_rendering(&mut self) {
        todo!()
        // need to be able to stop the renderer
        // the purpose is:
        // when switching which regions should be rendered (e.g. switching worlds),
        // we do not want to accidentally receive stale rendered tiles of the previous world.
    }
}

enum WorkerTask {
    Render(Region, RenderOptions),
    BarrierWait,
}

pub struct TileRenderer {
    ctx: egui::Context,

    // not really caching the tiles themselves, just which tiles we already sent to the "frontend"
    // to avoid rerendering them unnecessarily
    cached_tiles: rustc_hash::FxHashSet<TileSpacePos>,
    render_task_tx: crossbeam_channel::Sender<WorkerTask>,
    render_task_rx: crossbeam_channel::Receiver<WorkerTask>,
    render_result_tx: crossbeam_channel::Sender<RenderResult>,
    render_result_rx: crossbeam_channel::Receiver<RenderResult>,
    view_update_rx: crossbeam_channel::Receiver<View>,
    options_update_rx: crossbeam_channel::Receiver<RenderOptions>,
    regions_update_rx: crossbeam_channel::Receiver<Vec<Region>>,
    renderer_busy: Arc<AtomicBool>,
    barrier: Arc<Barrier>,
    num_worker_threads: usize,
    target_regions: Vec<Region>,
    view: View,
    opt: RenderOptions,
    filtered_regions: Vec<Region>,
}

fn box_dist(r: &Region, target: TileSpacePos) -> u32 {
    r.x.abs_diff(target.x).max(r.z.abs_diff(target.z))
}

impl TileRenderer {
    fn new(
        ctx: egui::Context,
        num_worker_threads: usize,
        view_update_rx: crossbeam_channel::Receiver<View>,
        regions_update_rx: crossbeam_channel::Receiver<Vec<Region>>,
        options_update_rx: crossbeam_channel::Receiver<RenderOptions>,
    ) -> Self {
        let cached_tiles = rustc_hash::FxHashSet::default();

        let (render_task_tx, render_task_rx) = crossbeam_channel::bounded(num_worker_threads);
        let (render_result_tx, render_result_rx) = crossbeam_channel::bounded(num_worker_threads);

        let renderer_busy = Arc::new(AtomicBool::new(false));

        let barrier = Arc::new(Barrier::new(num_worker_threads + 1));

        let t = Self {
            cached_tiles,
            barrier,
            ctx,
            render_result_tx,
            options_update_rx,
            render_task_rx,
            render_task_tx,
            render_result_rx,
            view_update_rx,
            regions_update_rx,
            renderer_busy,
            num_worker_threads,
            opt: RenderOptions::default(),
            target_regions: Vec::new(),
            filtered_regions: Vec::new(),
            view: View::empty(),
        };

        t.spawn_workers(num_worker_threads);

        t
    }

    fn handle_view_update(&mut self, v: View) {
        if v.overflow {
            self.cached_tiles.clear();
        } else {
            self.cached_tiles.retain(|t| v.keep(t.x, t.z));
        }
        self.view = v;
        self.update_filtered_regions();
        // need to synchronize workers here too?
    }

    fn handle_regions_update(&mut self, r: Vec<Region>) {
        self.reset_and_synchronize_workers();
        self.cached_tiles.clear();
        self.target_regions = r;
        self.update_filtered_regions();
    }

    fn update_filtered_regions(&mut self) {
        // TODO: do better

        let view_center_x =
            (self.view.visible.x_range.end() + self.view.visible.x_range.start()) / 2;
        let view_center_z =
            (self.view.visible.z_range.end() + self.view.visible.z_range.start()) / 2;

        let target = TileSpacePos {
            x: view_center_x,
            z: view_center_z,
        };

        let vis = self.view.visible.clone();
        self.filtered_regions = self
            .target_regions
            .iter()
            .filter(|r| vis.contains(TileSpacePos { x: r.x, z: r.z }))
            .cloned()
            .collect();

        self.filtered_regions
            .sort_by_key(|r1| -(box_dist(r1, target) as i64));

        // info!(
        //     "target: ({}, {}), view: {:?}, num filtered regions: {}",
        //     target.x,
        //     target.z,
        //     self.view,
        //     self.filtered_regions.len()
        // );
    }

    fn handle_event(&mut self, ev: RendererEvent) {
        match ev {
            RendererEvent::Regions(r) => self.handle_regions_update(r),
            RendererEvent::View(v) => self.handle_view_update(v),
            RendererEvent::Options(o) => self.handle_options_update(o),
        }
    }

    fn handle_options_update(&mut self, mut opt: RenderOptions) {
        info!("options update: {:?}", opt);
        while let Ok(o) = self.options_update_rx.try_recv() {
            opt = o;
        }

        if self.opt == opt {
            return;
        };
        self.reset_and_synchronize_workers();
        self.cached_tiles.clear();
        self.opt = opt;
        self.update_filtered_regions();
    }

    fn recv_event(&self) -> Result<RendererEvent, RecvError> {
        select! {
            recv(self.view_update_rx) -> res => res.map(|v| {RendererEvent::View(v)}),
            recv(self.regions_update_rx) -> res => res.map(|r| {RendererEvent::Regions(r)}),
            recv(self.options_update_rx) -> res => res.map(|o| {RendererEvent::Options(o)}),
        }
    }

    fn try_recv_event(&self) -> Result<RendererEvent, TryRecvError> {
        select! {
            recv(self.view_update_rx) -> res => res.map(|v| {RendererEvent::View(v)}).map_err(|_| TryRecvError::Disconnected),
            recv(self.regions_update_rx) -> res => res.map(|r| {RendererEvent::Regions(r)}).map_err(|_| TryRecvError::Disconnected),
            recv(self.options_update_rx) -> res => res.map(|r| {RendererEvent::Options(r)}).map_err(|_| TryRecvError::Disconnected),
            default => Err(TryRecvError::Empty)
        }
    }

    fn run(&mut self) -> anyhow::Result<()> {
        // NOTE: use select?
        loop {
            self.handle_event(self.recv_event()?);
            while let Ok(r) = self.try_recv_event() {
                self.handle_event(r);
            }

            while let Some(r) = self.filtered_regions.pop() {
                let r_pos = TileSpacePos { x: r.x, z: r.z };

                if self.cached_tiles.contains(&r_pos) {
                    continue;
                }

                self.render_task_tx
                    .send(WorkerTask::Render(r, self.opt.clone()))
                    .map_err(|_| anyhow::anyhow!("render_task_tx disconnected"))?;
                self.cached_tiles.insert(r_pos);

                // drain events ch
                while let Ok(r) = self.try_recv_event() {
                    self.handle_event(r);
                }
            }
        }
    }

    fn reset_and_synchronize_workers(&self) {
        while self.render_task_rx.try_recv().is_ok() {}

        for _ in 0..self.num_worker_threads {
            // TODO: handle errors (any would be fatal)
            _ = self.render_task_tx.send(WorkerTask::BarrierWait);
        }
        self.barrier.wait();
    }

    fn spawn_workers(&self, num_worker_threads: usize) {
        for _ in 0..num_worker_threads {
            let mut w = Worker::new(
                self.render_task_rx.clone(),
                self.render_result_tx.clone(),
                self.barrier.clone(),
                self.ctx.clone(),
            );
            std::thread::spawn(move || {
                if let Err(e) = w.run() {
                    error!("worker exited with {:?}", e)
                }
            });
        }
    }
}

enum RendererEvent {
    View(View),
    Regions(Vec<Region>),
    Options(RenderOptions),
}

pub fn init_tile_renderer(ctx: egui::Context, num_worker_threads: usize) -> TileRendererHandle {
    if num_worker_threads == 0 {
        panic!("num_worker_threads must be > 0");
    };

    let (view_update_tx, view_update_rx) = crossbeam_channel::bounded(100);
    let (options_update_tx, options_update_rx) = crossbeam_channel::bounded(100);
    let (regions_update_tx, regions_update_rx) = crossbeam_channel::bounded(1);

    let mut t = TileRenderer::new(
        ctx,
        num_worker_threads,
        view_update_rx.clone(),
        regions_update_rx.clone(),
        options_update_rx.clone(),
    );

    let render_result_rx = t.render_result_rx.clone(); // TODO: cleanup

    let renderer_busy = t.renderer_busy.clone();

    _ = std::thread::spawn(move || t.run());

    TileRendererHandle {
        view_update_rx,
        view_update_tx,
        regions_update_rx,
        regions_update_tx,
        renderer_busy,
        render_result_rx,
        options_update_rx,
        options_update_tx,
    }
}

pub struct RenderResult {
    pub region_x: i32,
    pub region_z: i32,
    pub tex_handle: egui::TextureHandle,
    pub lookup: Option<PackedBlockMap>,
    pub dur: std::time::Duration,
}

struct Worker {
    render_task_rx: crossbeam_channel::Receiver<WorkerTask>,
    render_result_tx: crossbeam_channel::Sender<RenderResult>,
    ctx: egui::Context,
    img: image::RgbaImage,
    barrier: Arc<Barrier>,
    loader: loader::McaLoader,
    lookup: ColorLookup,
}

impl Worker {
    fn new(
        render_task_rx: crossbeam_channel::Receiver<WorkerTask>,
        render_result_tx: crossbeam_channel::Sender<RenderResult>,
        barrier: Arc<Barrier>,
        ctx: egui::Context,
    ) -> Self {
        let img = image::RgbaImage::new(512, 512);
        let loader = loader::McaLoader::new();
        let lookup = ColorLookup::default();

        Self {
            render_result_tx,
            render_task_rx,
            ctx,
            barrier,
            img,
            loader,
            lookup,
        }
    }
}

impl Worker {
    fn run(&mut self) -> anyhow::Result<()> {
        loop {
            let task = match self.render_task_rx.recv() {
                Ok(r) => r,
                Err(_) => anyhow::bail!("bye"),
            };

            match task {
                WorkerTask::BarrierWait => _ = self.barrier.wait(),
                WorkerTask::Render(task, opt) => {
                    let before = std::time::Instant::now();
                    match self.process(&task, &opt) {
                        Ok(lookup) => {
                            let after = std::time::Instant::now();
                            let dur = after.duration_since(before);
                            let col_img = egui::ColorImage::from_rgba_unmultiplied(
                                [512, 512],
                                self.img.as_raw(),
                            );
                            let tex = self.ctx.load_texture(
                                format!("r.{}.{}.mca", task.x, task.z),
                                col_img,
                                egui::TextureOptions::NEAREST,
                            );

                            let res = RenderResult {
                                region_x: task.x,
                                region_z: task.z,
                                tex_handle: tex,
                                lookup,
                                dur,
                            };

                            if self.render_result_tx.send(res).is_err() {
                                anyhow::bail!("render_result_tx disconnected")
                            }

                            // this is necessary, otherwise the ui fn will not even run to check the
                            // channel (if there is no other thing triggering a repaint).
                            // it works out in practice, but technically i don't think there is any
                            // guarantee that this is correct w.r.t. ordering? is there anything that
                            // prevents the ui from receiving the repaint signal and checking the
                            // channel before the message is actually receivable, thus going to sleep
                            // again without actually receiving the message?
                            self.ctx.request_repaint();
                        }
                        Err(_) => {
                            // NOTE: could send result with an error?
                        }
                    }
                }
            }
        }
    }

    fn process(
        &mut self,
        task: &Region,
        opt: &RenderOptions,
    ) -> anyhow::Result<Option<PackedBlockMap>> {
        // NOTE:
        // i want to test the performance diff for turning some of the boolean flag options (mostly
        // with_block_map) into a const param on the function and then manually dispatching either
        // variant
        // would be surprised if it was even measurable here though

        match opt.mode {
            RenderMode::Normal => render_region(
                task,
                &mut self.img,
                &mut self.loader,
                &mut self.lookup,
                opt.depth_darken,
                opt.with_block_map,
            ),
            RenderMode::Slice(slice_height) => render_slice(
                task,
                &mut self.img,
                &mut self.loader,
                &mut self.lookup,
                slice_height,
                opt.depth_darken,
                opt.with_block_map,
            ),
        }
    }
}
