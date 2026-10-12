use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use mca_viewer_rs::{
    palette::{BlockPalette, TomlPalette},
    region::{Region, RegionRange, parse_regions_in_dir},
    worker::{Notify, RenderCtx, RenderOptions, View},
};
use std::num::NonZeroUsize;
use std::sync::Arc;

#[derive(clap::Parser)]
struct Args {
    /// Path to a directory containing region files (*.mca)
    #[arg(long, short)]
    path: PathBuf,

    /// Number of worker threads (omit to use std::thread::available_parallelism())
    #[arg(long, short)]
    num_workers: Option<NonZeroUsize>,

    /// Number of repetitions
    #[arg(long, short, default_value_t = NonZeroUsize::new(1).unwrap())]
    repeat: NonZeroUsize,

    #[arg(long, short)]
    slice_height: Option<u16>,
}

#[derive(Clone)]
struct DummyRenderCtx;
impl RenderCtx<()> for DummyRenderCtx {
    fn upload_tile(&self, _: &[u8], _: u32, _: u32) {}
}

#[derive(Clone)]
struct DummyNotify;
impl Notify for DummyNotify {
    fn notify(&self) {}
}

fn main() -> anyhow::Result<()> {
    let palette = Arc::new(BlockPalette::new(
        toml::from_str::<TomlPalette>(include_str!("../../palette.toml"))?,
        [255, 0, 255, 255],
    ));

    let args = Args::parse();
    let regions = parse_regions_in_dir(&args.path)?.collect::<Arc<[_]>>();

    if regions.is_empty() {
        anyhow::bail!("no regions found");
    }

    let p = regions[0].pos();

    let (min_x, max_x, min_z, max_z) = regions
        .iter()
        .map(Region::pos)
        .fold((p.x, p.x, p.z, p.z), |a, r| {
            (a.0.min(r.x), a.1.max(r.x), a.2.min(r.z), a.3.max(r.z))
        });

    let rg = RegionRange::new(min_x, max_x, min_z, max_z);
    let view = View::new(rg, rg);

    let num_workers = args.num_workers.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .unwrap_or(const { std::num::NonZeroUsize::new(1).unwrap() })
    });

    println!("{} regions", regions.len());
    println!("num_workers: {:?}", num_workers.get());
    println!("repeat: {:?}", args.repeat.get());

    let (mut h, mut m) = mca_viewer_rs::worker::Manager::<
        DummyNotify,
        BlockPalette,
        DummyRenderCtx,
        (),
    >::new(DummyNotify, DummyRenderCtx, num_workers);

    let _ = std::thread::spawn(move || m.run());

    let opt = if let Some(h) = args.slice_height {
        RenderOptions {
            mode: mca_viewer_rs::worker::RenderMode::Slice,
            slice_height: h,
            ..Default::default()
        }
    } else {
        RenderOptions::default()
    };

    let mut timings = Vec::with_capacity(regions.len());

    for i in 0..args.repeat.get() {
        let mut expecting = regions.len();
        h.send_task(regions.clone(), opt, view, palette.clone(), true);

        timings.clear();
        let before = Instant::now();
        while expecting > 0 {
            match h.recv_result() {
                Err(crossbeam_channel::RecvError) => {
                    anyhow::bail!("channel disconnected")
                }
                Ok(r) => {
                    if r.inner.is_ok() {
                        timings.push(r.time)
                    }
                    expecting -= 1;
                }
            }
        }

        let after = before.elapsed();

        timings.sort();
        let m = timings[timings.len() / 2];

        println!("[{i}] total duration: {after:?}, median region duration: {m:?}");
    }

    Ok(())
}
