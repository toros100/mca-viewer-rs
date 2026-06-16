use std::{hint::black_box, path::PathBuf};

use clap::Parser;
use mca_viewer_rs::{get_palette, unpack_from_u64_bytes_width_4};

#[derive(clap::Parser)]
struct Args {
    /// Path to a directory containing region files (*.mca)
    #[arg(long)]
    path: PathBuf,
}

fn main() -> anyhow::Result<()> {
    black_box(unpack_from_u64_bytes_width_4(
        black_box(&[0u8; 8]),
        black_box(0),
    ));
    _ = get_palette(); // to init the OnceLock

    let args = Args::parse();

    let regions = mca_viewer_rs::parse_regions_in_dir(&args.path)?;

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            "warn,mca_viewer_rs=debug",
        ))
        .init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([400.0, 300.0]),
        // .with_icon(
        //     // NOTE: Adding an icon is optional
        //     eframe::icon_data::from_png_bytes(
        //         &include_bytes!("../assets/favicon-512x512.png")[..],
        //     )
        //     .expect("Failed to load icon"),
        // ),
        ..Default::default()
    };
    eframe::run_native(
        "mca-viewer-rs",
        native_options,
        Box::new(|cc| Ok(Box::new(mca_viewer_rs::App::new(cc, regions)))),
    )
    .map_err(|e| anyhow::anyhow!(e))
}
