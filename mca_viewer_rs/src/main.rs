use clap::Parser;
use mca_viewer_rs::palette::{BlockPalette, TomlPalette};
use std::{num::NonZeroUsize, path::PathBuf, sync::Arc};

mod app;

// NOTE: should accept palette as arg
const PALETTE_TOML_STR: &str = include_str!("../palette.toml");

#[derive(clap::Parser)]
struct Args {
    #[arg(long, short)]
    path: PathBuf,

    #[arg(long, short)]
    num_workers: Option<NonZeroUsize>,
}

fn main() -> anyhow::Result<()> {
    let mut settings = slint::wgpu_29::WGPUSettings::default();
    settings.device_required_limits = slint::wgpu_29::wgpu::Limits::defaults();

    slint::BackendSelector::new()
        .require_wgpu_29(slint::wgpu_29::WGPUConfiguration::Automatic(settings))
        .select()?;

    let args = Args::parse();

    let num_workers = args.num_workers.unwrap_or_else(|| {
        std::thread::available_parallelism().unwrap_or(const { NonZeroUsize::new(1).unwrap() })
    });

    println!("num_workers={}", num_workers);

    let ctx = BlockPalette::new(TomlPalette::try_from(PALETTE_TOML_STR)?, [255, 0, 255, 255]);

    let app = app::new(num_workers, Arc::new(ctx))?;

    app.borrow_mut().load_dir(&args.path)?;

    app.run()
}
