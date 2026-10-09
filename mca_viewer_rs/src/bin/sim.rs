// simulating the workload the workload in a simplified way for profiling
// WARN: single threaded simulation

use std::{hint::black_box, path::PathBuf};

use clap::Parser;
use mca_tools::loader::McaLoader;
use mca_viewer_rs::block_map::BlockMap;
use mca_viewer_rs::draw_region::draw_region;
use mca_viewer_rs::palette::BlockPalette;
use mca_viewer_rs::palette::TomlPalette;
use mca_viewer_rs::region::Region;
use mca_viewer_rs::region::parse_regions_in_dir;

#[derive(clap::Parser)]
struct Args {
    /// Path to a directory containing region files (*.mca)
    #[arg(long)]
    path: PathBuf,
}

struct Worker {
    img: image::RgbaImage,
    loader: McaLoader,
    color_lookup: BlockMap,
}

impl Worker {
    fn default() -> Self {
        Self {
            img: image::RgbaImage::new(512, 512),
            loader: McaLoader::default(),
            color_lookup: Default::default(),
        }
    }

    fn render_region(&mut self, r: &Region, ctx: &BlockPalette) -> anyhow::Result<()> {
        black_box(draw_region(
            r,
            &mut self.img,
            &mut self.loader,
            &mut self.color_lookup,
            true,
            true,
            ctx,
        ))
        .map(|_| ())
    }
}

fn main() -> anyhow::Result<()> {
    let ctx = BlockPalette::new(
        toml::from_str::<TomlPalette>(include_str!("../../palette.toml"))?,
        [255, 0, 255, 255],
    );

    let args = Args::parse();

    let regions = parse_regions_in_dir(&args.path)?;

    render_all(black_box(regions), &ctx);

    Ok(())
}

fn render_all(regions: impl Iterator<Item = Region>, ctx: &BlockPalette) {
    let mut worker = Worker::default();
    for r in regions {
        _ = worker.render_region(&r, ctx);
    }
}
