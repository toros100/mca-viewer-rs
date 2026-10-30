use crate::block_map::{BlockMap, PackedBlockMap};
use crate::draw_region::{self, DrawError, HeightShadeValues};
use crate::palette::PaletteCtx;
use crate::region::{Region, RegionPosition, RegionRange};
use crossbeam_channel::{Receiver, Sender, select_biased};
use mca_tools::loader::McaLoader;
use std::collections::HashSet;

use nbt::bumpalo;

use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderOptions {
    pub depth_darken: bool,
    pub block_map: bool,
    pub mode: RenderMode,
    // keeping slice height as a field rather than in the RenderMode enum to make it persistent when
    // choosing a mode other than Slice and then going back to Slice
    pub slice_height: i16,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            depth_darken: true,
            block_map: true,

            mode: RenderMode::Normal,

            // ends up being sea level for overworld, spiritually sea level for the end and the midway
            // height for the nether, seems like a reasonable default in each case
            slice_height: 62,
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
    palette: std::sync::Weak<P>,
}

#[derive(Debug)]
pub struct RegionTask<P> {
    region: Region,
    options: RenderOptions,
    task_id: usize,
    palette: Weak<P>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub visible: RegionRange,
    pub keep: RegionRange,
}

impl Default for View {
    fn default() -> Self {
        Self::EMPTY
    }
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

pub struct RegionResult<O> {
    pub task_id: usize,
    pub time: Duration,
    pub inner: Result<RegionTile<O>, RegionError>,
}

pub struct RegionTile<O> {
    pub x: i32,
    pub z: i32,
    pub tile: O,
    pub block_map: Option<crate::block_map::PackedBlockMap>,
}

pub struct RegionError {
    pub x: i32,
    pub z: i32,
    pub tile: DrawError,
}

impl<O> RegionTile<O> {
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

pub trait RenderCtx<O>
where
    Self: Send + Clone + 'static,
    O: Send + 'static,
{
    fn upload_tile(&self, data: &[u8], width: u32, height: u32) -> O;
}

// need this abstraction in mod draw_region to be able to call the functions in tests with miri.
// can't use McaLoader, because that uses ffi calls (libdeflater)
pub trait ChunkAccessor {
    type Error: std::error::Error;
    /// should return the (decompressed) chunk data for chunk idx < 1024
    fn get_chunk_data(&mut self, idx: usize) -> Result<&[u8], Self::Error>;
}

impl ChunkAccessor for McaLoader {
    type Error = mca_tools::loader::McaError;
    fn get_chunk_data(&mut self, idx: usize) -> Result<&[u8], Self::Error> {
        McaLoader::get_chunk_data(self, idx)
    }
}

struct Worker<N, P, R, O> {
    img: Box<[u8; 512 * 512 * 4]>,
    water_img: Box<[u8; 512 * 512 * 4]>,
    darken: Box<[u8; 512 * 512]>,
    loader: mca_tools::loader::McaLoader,
    block_map: BlockMap,
    ms: HeightShadeValues,
    notify: N,
    task_rx: Receiver<RegionTask<P>>,
    results_tx: Sender<RegionResult<O>>,
    ctx: R,
    bump: bumpalo::Bump,
}

impl<N, P, R, O> Worker<N, P, R, O>
where
    N: Notify,
    P: PaletteCtx,
    O: Send + 'static,
    R: RenderCtx<O>,
{
    fn new(
        notify: N,
        task_rx: Receiver<RegionTask<P>>,
        results_tx: Sender<RegionResult<O>>,
        ctx: R,
    ) -> Self {
        Self {
            bump: bumpalo::Bump::new(),
            darken: vec![0u8; 512 * 512].try_into().unwrap(),
            ms: HeightShadeValues::default(),
            task_rx,
            results_tx,
            notify,
            img: vec![0u8; 512 * 512 * 4].try_into().unwrap(),
            water_img: vec![0u8; 512 * 512 * 4].try_into().unwrap(),
            loader: mca_tools::loader::McaLoader::default(),
            block_map: BlockMap::default(),
            ctx,
        }
    }

    pub fn run(&mut self) {
        while let Ok(task) = self.task_rx.recv() {
            let res = self.render_region(&task);
            self.send_result(res);
        }
    }

    pub fn send_result(&self, t: RegionResult<O>) {
        if self.results_tx.send(t).is_ok() {
            self.notify.notify();
        }
    }

    pub fn render_region(&mut self, task: &RegionTask<P>) -> RegionResult<O> {
        let opt = task.options;
        let before = Instant::now();

        let Some(ctx) = task.palette.upgrade() else {
            return RegionResult {
                task_id: task.task_id,
                time: before.elapsed(),
                inner: Err(RegionError {
                    x: task.region.x,
                    z: task.region.z,
                    tile: DrawError::PaletteInvalidated,
                }),
            };
        };

        let f = match std::fs::File::open(&task.region.path) {
            Ok(f) => f,
            Err(e) => {
                return RegionResult {
                    task_id: task.task_id,
                    time: before.elapsed(),
                    inner: Err(RegionError {
                        x: task.region.x,
                        z: task.region.z,
                        tile: e.into(),
                    }),
                };
            }
        };

        if let Err(e) = self.loader.load_mca(f) {
            return RegionResult {
                task_id: task.task_id,
                time: before.elapsed(),
                inner: Err(RegionError {
                    x: task.region.x,
                    z: task.region.z,
                    tile: e.into(),
                }),
            };
        }

        let res = match opt.mode {
            RenderMode::Normal => draw_region::draw_region(
                &mut self.img,
                &mut self.water_img,
                &mut self.ms,
                &mut self.darken,
                &mut self.loader,
                &mut self.block_map,
                opt.depth_darken,
                opt.block_map,
                &ctx,
                &mut self.bump,
            ),
            RenderMode::Slice => draw_region::draw_slice(
                &mut self.img,
                &mut self.water_img,
                &mut self.ms,
                &mut self.darken,
                &mut self.loader,
                &mut self.block_map,
                opt.slice_height,
                opt.depth_darken,
                opt.block_map,
                &ctx,
                &mut self.bump,
            ),
        };

        let blocks = if res.is_ok() && opt.block_map {
            Some(PackedBlockMap::new(&self.block_map))
        } else {
            None
        };

        let time = before.elapsed();

        RegionResult {
            task_id: task.task_id,
            time,
            inner: match res {
                Ok(()) => Ok(RegionTile {
                    x: task.region.x,
                    z: task.region.z,
                    block_map: blocks,
                    tile: self.ctx.upload_tile(self.img.as_slice(), 512, 512),
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

pub struct Manager<N, P, R, O> {
    render_task_rx: Receiver<RenderTask<P>>,
    view_update_rx: Receiver<ViewUpdate>,
    already_rendered: HashSet<RegionPosition>,
    results_tx: Sender<RegionResult<O>>,
    current_task: Option<RenderTask<P>>,
    regions: Vec<usize>,
    next_region: usize,
    notify: N,
    worker_task_tx: Sender<RegionTask<P>>,
    worker_task_rx: Receiver<RegionTask<P>>,
    _render_ctx: PhantomData<R>,
}

pub struct ManagerHandle<P, O> {
    render_task_tx: Sender<RenderTask<P>>,
    view_update_tx: Sender<ViewUpdate>,
    view_update_rx: Receiver<ViewUpdate>,
    results_rx: Receiver<RegionResult<O>>,
    task_id: usize,
}

impl<N, P, R, O> Manager<N, P, R, O>
where
    N: Notify,
    P: PaletteCtx + Send + Sync + 'static,
    O: Send + 'static,
    R: RenderCtx<O>,
{
    pub fn new(notify: N, ctx: R, num_workers: NonZeroUsize) -> (ManagerHandle<P, O>, Self) {
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
            _render_ctx: PhantomData,
        };

        manager.spawn_workers(num_workers, ctx);

        (handle, manager)
    }

    #[expect(clippy::needless_pass_by_value)]
    fn spawn_workers(&self, num_workers: NonZeroUsize, ctx: R) {
        for _ in 0..num_workers.get() {
            std::thread::spawn({
                let task_rx = self.worker_task_rx.clone();
                let results_tx = self.results_tx.clone();
                let notify = self.notify.clone();

                let ctx = ctx.clone();

                move || {
                    let mut w = Worker::<N, P, R, O>::new(notify, task_rx, results_tx, ctx);
                    w.run()
                }
            });
        }
    }

    fn idle(&mut self) -> Result<(), crossbeam_channel::RecvError> {
        select_biased!(
                recv(self.render_task_rx) -> res => {
                    self.handle_task(res?);
                }
                recv(self.view_update_rx) -> res => {
                    self.handle_view_update(res?);
                }
        );
        Ok(())
    }

    pub fn run(&mut self) {
        loop {
            // i do not really like that this kind of hides control flow in the condition
            if self.current_task.is_none() && self.idle().is_err() {
                return;
            };

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
            } else if self.idle().is_err() {
                return;
            };
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

impl<P: PaletteCtx, O> ManagerHandle<P, O> {
    pub fn send_task(
        &mut self,
        regions: Arc<[Region]>,
        opt: RenderOptions,
        view: View,
        palette: Weak<P>,
    ) -> usize {
        let task_id = self.task_id;
        self.task_id = self.task_id.wrapping_add(1);
        let task = RenderTask {
            regions,
            options: opt,
            view,
            task_id,
            palette,
        };
        self.render_task_tx.send(task).unwrap();
        task_id
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

    pub fn recv_result(&self) -> Result<RegionResult<O>, crossbeam_channel::RecvError> {
        self.results_rx.recv()
    }

    pub fn try_recv_result(&self) -> Result<RegionResult<O>, crossbeam_channel::TryRecvError> {
        self.results_rx.try_recv()
    }
}

#[cfg(test)]
mod tests {

    #[cfg(miri)]
    #[test]
    fn draw_some_chunks() {
        // pretty untargeted and slow
        //
        // i guess it is still worth something that miri does not find anything wrong with all the
        // unsafe involved

        use crate::{
            block_map::{self, BlockMap},
            draw_region::{HeightShadeValues, draw_slice},
            palette::DummyPalette,
            worker::ChunkAccessor,
        };
        use mca_tools::loader::McaError;
        use nbt::bumpalo;

        const CHUNK0: &[u8] = include_bytes!("../test_data/decompressed_chunks/chunk_0.dat");
        const CHUNK1: &[u8] = include_bytes!("../test_data/decompressed_chunks/chunk_1.dat");
        const CHUNK2: &[u8] = include_bytes!("../test_data/decompressed_chunks/chunk_2.dat");
        const CHUNK3: &[u8] = include_bytes!("../test_data/decompressed_chunks/chunk_3.dat");

        struct PreDecompressedChunks;
        impl ChunkAccessor for PreDecompressedChunks {
            type Error = McaError;
            fn get_chunk_data(&mut self, idx: usize) -> Result<&[u8], Self::Error> {
                match idx {
                    0 => Ok(CHUNK0),
                    1 => Ok(CHUNK1),
                    31 => Ok(CHUNK0),
                    30 => Ok(CHUNK1),
                    29 => Ok(CHUNK2),
                    28 => Ok(&CHUNK0[..500]), // will error during deserialization
                    27 => Ok(CHUNK3),
                    900 => Ok(CHUNK1),
                    _ => Err(McaError::ChunkNotFound),
                }
            }
        }

        let mut img = vec![0u8; 4 * 512 * 512].try_into().unwrap();
        let mut water_img = vec![0u8; 4 * 512 * 512].try_into().unwrap();
        let mut block_map = BlockMap::default();
        let mut height_shade = HeightShadeValues::default();
        let mut darken_data = vec![0u8; 512 * 512].try_into().unwrap();
        let mut bump = bumpalo::Bump::new();

        draw_slice(
            &mut img,
            &mut water_img,
            &mut height_shade,
            &mut darken_data,
            &mut PreDecompressedChunks,
            &mut block_map,
            150, // above sea level, so a lot of blocks will be accessed
            true,
            false,
            &DummyPalette,
            &mut bump,
        )
        .unwrap();
    }
}
