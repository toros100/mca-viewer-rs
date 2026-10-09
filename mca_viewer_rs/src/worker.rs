use crate::block_map::{BlockMap, PackedBlockMap};
use crate::draw_region::{self};
use crate::palette::PaletteCtx;
use crate::region::{Region, RegionPosition, RegionRange};
use crate::renderer::{GpuTile, RenderCtx};
use crossbeam_channel::{Receiver, Sender, select_biased};
use std::collections::HashSet;

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const TILE_SIZE: u32 = 512;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderOptions {
    pub depth_darken: bool,
    pub block_map: bool,
    pub mode: RenderMode,
    pub slice_height: u16,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            depth_darken: true,
            block_map: true,
            mode: RenderMode::Normal,
            slice_height: 127,
        }
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub enum RenderMode {
    #[default]
    Normal,
    Slice,
}

#[derive(Debug)]
pub struct RenderTask<P> {
    regions: Arc<[Region]>,
    options: RenderOptions,
    view: View,
    task_id: usize,
    palette: Arc<P>,
}

#[derive(Debug)]
pub struct RegionTask<P> {
    region: Region,
    options: RenderOptions,
    task_id: usize,
    palette: Arc<P>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub visible: RegionRange,
    pub keep: RegionRange,
}

impl View {
    pub const EMPTY: View = View {
        visible: RegionRange::new(0, -1, 0, -1),
        keep: RegionRange::new(0, -1, 0, -1),
    };
    pub fn single(x: i32, z: i32) -> Self {
        let rg = RegionRange::new(x, x, z, z);
        Self {
            visible: rg,
            keep: rg,
        }
    }

    pub fn new(visible: RegionRange, keep: RegionRange) -> Self {
        Self { visible, keep }
    }

    pub fn iter_visible(&self) -> impl Iterator<Item = RegionPosition> {
        self.visible.iter_positions()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TileError {
    #[error("todo")]
    SomeErrorTodo,
}

pub struct RegionResult<P> {
    task_id: usize,
    pub time: Duration,
    pub inner: Result<RegionTile<P>, RegionError>,
}

pub struct RegionTile<P> {
    pub x: i32,
    pub z: i32,
    pub tile: GpuTile,
    pub block_map: Option<crate::block_map::PackedBlockMap<P>>,
}

pub struct RegionError {
    pub x: i32,
    pub z: i32,
    pub tile: anyhow::Error,
}

impl<P> RegionTile<P> {
    pub fn pos(&self) -> RegionPosition {
        RegionPosition::new(self.x, self.z)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ViewUpdate {
    view: View,
    overflowed: bool,
}

// used to notify after something is sent through the results channel
//
// i need this to wake up slint, otherwise the channel will not be drained if nothing else wakes
// the event loop
pub trait Notify: Send + Clone + 'static {
    fn notify(&self);
}

struct Worker<N, P> {
    img: image::RgbaImage,
    loader: mca_tools::loader::McaLoader,
    lookup: BlockMap,
    notify: N,
    task_rx: Receiver<RegionTask<P>>,
    results_tx: Sender<RegionResult<P>>,
    ctx: RenderCtx,
}

impl<N: Notify, P: PaletteCtx> Worker<N, P> {
    fn new(
        notify: N,
        task_rx: Receiver<RegionTask<P>>,
        results_tx: Sender<RegionResult<P>>,
        ctx: RenderCtx,
    ) -> Self {
        let img = image::RgbaImage::new(TILE_SIZE, TILE_SIZE);
        let loader = mca_tools::loader::McaLoader::default();
        let lookup = BlockMap::default();
        Self {
            task_rx,
            results_tx,
            notify,
            img,
            loader,
            lookup,
            ctx,
        }
    }

    pub fn run(&mut self) {
        while let Ok(task) = self.task_rx.recv() {
            let res = self.render_region(task);
            self.send_result(res);
        }
    }

    pub fn send_result(&self, t: RegionResult<P>) {
        if self.results_tx.send(t).is_ok() {
            self.notify.notify();
        }
    }

    pub fn render_region(&mut self, task: RegionTask<P>) -> RegionResult<P> {
        let opt = task.options;
        let before = Instant::now();
        let res = match opt.mode {
            RenderMode::Normal => draw_region::draw_region(
                &task.region,
                &mut self.img,
                &mut self.loader,
                &mut self.lookup,
                opt.depth_darken,
                opt.block_map,
                task.palette.as_ref(),
            ),
            RenderMode::Slice => draw_region::draw_slice(
                &task.region,
                &mut self.img,
                &mut self.loader,
                &mut self.lookup,
                opt.slice_height,
                opt.depth_darken,
                opt.block_map,
                task.palette.as_ref(),
            ),
        };

        let blocks = if res.is_ok() && opt.block_map {
            Some(PackedBlockMap::new(&self.lookup, task.palette))
        } else {
            None
        };

        let after = Instant::now();
        let time = after.duration_since(before);

        RegionResult {
            task_id: task.task_id,
            time,
            inner: match res {
                Ok(()) => Ok(RegionTile {
                    x: task.region.x,
                    z: task.region.z,
                    block_map: blocks,
                    tile: self.ctx.upload_tile(&self.img, TILE_SIZE, TILE_SIZE),
                }),
                Err(e) => Err(RegionError {
                    x: task.region.x,
                    z: task.region.z,
                    tile: e,
                }),
            },
        }
    }
}

pub struct Manager<N, P> {
    render_task_rx: Receiver<RenderTask<P>>,
    view_update_rx: Receiver<ViewUpdate>,
    already_rendered: HashSet<RegionPosition>,
    results_tx: Sender<RegionResult<P>>,
    current_task: Option<RenderTask<P>>,
    regions: Vec<usize>,
    next_region: usize,
    notify: N,
    worker_task_tx: Sender<RegionTask<P>>,
    worker_task_rx: Receiver<RegionTask<P>>,
}

pub struct ManagerHandle<P> {
    render_task_tx: Sender<RenderTask<P>>,
    view_update_tx: Sender<ViewUpdate>,
    view_update_rx: Receiver<ViewUpdate>,
    results_rx: Receiver<RegionResult<P>>,
    task_id: usize,
}

impl<N: Notify, P: PaletteCtx + Send + Sync + 'static> Manager<N, P> {
    pub fn new(notify: N, ctx: &RenderCtx, num_workers: NonZeroUsize) -> (ManagerHandle<P>, Self) {
        let (render_task_tx, render_task_rx) = crossbeam_channel::bounded(100);

        let (view_update_tx, view_update_rx) = crossbeam_channel::bounded(100);

        let (results_tx, results_rx) = crossbeam_channel::bounded(100);

        let handle = ManagerHandle {
            render_task_tx,
            view_update_tx,
            view_update_rx: view_update_rx.clone(),
            results_rx,
            task_id: 0,
        };

        let (worker_task_tx, worker_task_rx) = crossbeam_channel::bounded(num_workers.get());

        let manager = Self {
            already_rendered: HashSet::default(),
            results_tx,
            render_task_rx,
            view_update_rx,
            current_task: None,
            regions: vec![],
            next_region: 0,
            notify,
            worker_task_tx,
            worker_task_rx,
        };

        manager.spawn_workers(num_workers, ctx);

        (handle, manager)
    }

    fn spawn_workers(&self, num_workers: NonZeroUsize, ctx: &RenderCtx) {
        for _ in 0..num_workers.get() {
            std::thread::spawn({
                let task_rx = self.worker_task_rx.clone();
                let results_tx = self.results_tx.clone();
                let notify = self.notify.clone();
                let ctx = ctx.clone();

                move || {
                    let mut w = Worker::<N, P>::new(notify, task_rx, results_tx, ctx);
                    w.run()
                }
            });
        }
    }

    fn idle(&mut self) {
        select_biased!(
                recv(self.render_task_rx) -> res => {
                    match res {
                        Ok(s) => self.handle_task(s),
                        Err(e) => {
                            eprintln!("{e:?}")
                        }
                    }
                }
                recv(self.view_update_rx) -> res => {
                    match res {
                        Ok(s) => self.handle_view_update(s),
                        Err(e) => {
                            eprintln!("{e:?}")
                        }
                    }
                }
        )
    }

    pub fn run(&mut self) {
        loop {
            if self.current_task.is_none() {
                println!("no task");
                self.idle();
            }

            select_biased! {
                recv(self.render_task_rx) -> res => {
                    match res {
                        Ok(s) => self.handle_task(s),
                        Err(_) => return
                    }
                }
                recv(self.view_update_rx) -> res => {
                    match res {
                        Ok(s) => self.handle_view_update(s),
                        Err(_) => return
                    }
                }
                default => {}
            };

            if self.next_region < self.regions.len()
                && let Some(task) = self.current_task.as_ref()
            // .map(|t| t.regions[self.regions[self.next_region]].clone())
            {
                let id = task.task_id;
                let region = &task.regions[self.regions[self.next_region]];
                let pos = region.pos();
                let opt = task.options;

                self.next_region += 1;

                if self.already_rendered.contains(&pos) || !task.view.visible.contains(pos) {
                    continue;
                }

                _ = self.worker_task_tx.send(RegionTask {
                    region: region.clone(),
                    options: opt,
                    task_id: id,
                    palette: task.palette.clone(),
                });

                self.already_rendered.insert(pos);
            } else {
                self.idle();
            }
        }
    }

    fn handle_view_update(&mut self, v: ViewUpdate) {
        if v.overflowed {
            self.already_rendered.clear();
        } else {
            self.already_rendered.retain(|e| v.view.keep.contains(*e));
        }

        let Some(center) = v.view.visible.center() else {
            return;
        };

        if let Some(RenderTask { regions, view, .. }) = self.current_task.as_mut() {
            self.regions.clear();
            self.regions.extend(0..regions.len());
            self.regions.sort_unstable_by(|i, j| {
                let i_pos = regions[*i].pos();
                let j_pos = regions[*j].pos();

                center.box_dist(i_pos).cmp(&center.box_dist(j_pos))
            });
            self.next_region = 0;
            *view = v.view;
        }
    }

    fn handle_task(&mut self, t: RenderTask<P>) {
        let v = t.view;
        self.current_task = Some(t);
        self.handle_view_update(ViewUpdate {
            view: v,
            overflowed: true,
        });
    }
}

impl<P: PaletteCtx> ManagerHandle<P> {
    pub fn send_task(
        &mut self,
        regions: Arc<[Region]>,
        opt: RenderOptions,
        view: View,
        palette: Arc<P>,
        bump_id: bool,
    ) {
        if bump_id {
            self.task_id = self.task_id.wrapping_add(1);
        }
        let task = RenderTask {
            regions,
            options: opt,
            view,
            task_id: self.task_id,
            palette,
        };
        self.render_task_tx.send(task).unwrap()
    }
    pub fn send_view_update(&self, v: View) {
        let mut upd = ViewUpdate {
            view: v,
            overflowed: false,
        };
        loop {
            match self.view_update_tx.try_send(upd) {
                Ok(()) => break,
                Err(crossbeam_channel::TrySendError::Full(u)) => {
                    while self.view_update_rx.try_recv().is_ok() {}
                    upd = u;
                    upd.overflowed = true;
                }
                Err(crossbeam_channel::TrySendError::Disconnected(_)) => break,
            }
        }
    }

    pub fn try_recv_result(&self) -> Result<RegionResult<P>, crossbeam_channel::TryRecvError> {
        loop {
            match self.results_rx.try_recv() {
                Ok(s) => {
                    if s.task_id == self.task_id {
                        break Ok(s);
                    }
                }
                Err(e) => break Err(e),
            }
        }
    }
}
