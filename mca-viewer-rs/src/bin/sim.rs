// simulating the workload the workload in a simplified way for profiling
// WARN: single threaded simulation

use std::{hint::black_box, path::PathBuf};

use clap::Parser;
use mca_viewer_rs::{ColorLookup, Region, get_palette, parse_regions_in_dir, render_region};

#[derive(clap::Parser)]
struct Args {
    /// Path to a directory containing region files (*.mca)
    #[arg(long)]
    path: PathBuf,

    #[arg(long)]
    newdes: bool,
}
struct Worker {
    img: image::RgbaImage,
    loader: mca_viewer_rs::McaLoader,
    color_lookup: ColorLookup,
}

impl Worker {
    fn default() -> Self {
        Self {
            img: image::RgbaImage::new(512, 512),
            loader: mca_viewer_rs::McaLoader::new(),
            color_lookup: ColorLookup::default(),
        }
    }

    fn render_region(&mut self, r: &Region) -> anyhow::Result<()> {
        black_box(render_region(
            r,
            &mut self.img,
            &mut self.loader,
            &mut self.color_lookup,
            true,
            true,
        ))
        .map(|_| ())
    }
}

fn main() -> anyhow::Result<()> {
    _ = get_palette(); // to init the OnceLock

    let args = Args::parse();

    let regions = parse_regions_in_dir(&args.path)?;

    black_box(render_all(black_box(&regions)));

    Ok(())
}

fn render_all(regions: &[Region]) {
    let mut worker = Worker::default();
    for r in regions {
        _ = worker.render_region(r);
    }
}
