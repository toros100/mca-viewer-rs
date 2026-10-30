#![recursion_limit = "256"]

use std::path::PathBuf;

use mca_viewer_rs::renderer::{GpuTile, Renderer, VELLO_USAGES, WgpuRenderCtx};
use vello::{kurbo, wgpu};

use clap::Parser;
use mca_viewer_rs::{
    palette::{BlockPalette, TomlPalette},
    region::{Region, RegionRange, parse_regions_in_dir},
    worker::{Notify, RenderOptions, View},
};
use std::num::NonZeroUsize;
use std::sync::Arc;

#[derive(clap::Parser)]
struct Args {
    /// Path to a directory containing region files (*.mca)
    #[arg(long, short)]
    path: PathBuf,

    /// output image path
    #[arg(long, short)]
    out: PathBuf,

    /// Number of worker threads (omit to use std::thread::available_parallelism())
    #[arg(long, short)]
    num_workers: Option<NonZeroUsize>,

    #[arg(long, short, allow_negative_numbers = true)]
    slice_height: Option<i16>,

    /// scale exponent (scale factor will be 2^scale)
    #[arg(long, short, allow_negative_numbers = true, default_value_t = 0)]
    scale: i8,

    /// world position (block coordinates) of the top left corner
    #[arg(long, short, num_args = 2, action = clap::ArgAction::Set, value_names = ["X", "Z"], allow_negative_numbers = true)]
    top_left: Option<Vec<i32>>,

    /// size of the output image
    #[arg(long, short, required = true, action = clap::ArgAction::Set, num_args = 2, value_names = ["width", "height"])]
    size: Vec<u32>,

    #[arg(long, short, default_value_t = false)]
    region_borders: bool,
}

#[derive(Clone)]
struct DummyNotify;
impl Notify for DummyNotify {
    fn notify(&self) {}
}

// NOTE: would be nicer to render without wgpu
// could easily just implement RenderCtx where the "result" is just image::ImageRgba,
// but i would like it to be identical with the wgpu result
//
// would have to take into account that the wgpu sampler has
// mag_filter: wgpu::FilterMode::Nearest,
// min_filter: wgpu::FilterMode::Linear,

fn main() -> anyhow::Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))?;

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: None,
        required_limits: wgpu::Limits::defaults(),
        ..Default::default()
    }))?;

    let mut renderer = Renderer::new_with_usages(
        device.clone(),
        queue.clone(),
        VELLO_USAGES,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );

    let palette = Arc::new(BlockPalette::new(
        toml::from_str::<TomlPalette>(include_str!("../../palette.toml"))?,
        [255, 0, 255, 255],
    ));

    let args = Args::parse();

    let scale_fac = 2f64.powi(args.scale.clamp(-5, 5) as i32);

    let (w, h) = (args.size[0], args.size[1]);

    let blocks_w = ((w as f64) / scale_fac).ceil() as u32;
    let blocks_h = ((h as f64) / scale_fac).ceil() as u32;

    let (min_x, min_z) = args.top_left.map_or((0, 0), |v| (v[0], v[1]));

    let (min_region_x, min_region_z) = (min_x.div_euclid(512), min_z.div_euclid(512));

    let (max_region_x, max_region_z) = (
        min_region_x + (blocks_w / 512) as i32,
        min_region_z + (blocks_h / 512) as i32,
    );

    let affine =
        kurbo::Affine::scale(scale_fac).pre_translate((-min_x as f64, -min_z as f64).into());

    let rounded_translation = affine.translation().round();
    let rounded_affine = affine.with_translation(rounded_translation);

    let regions = parse_regions_in_dir(&args.path)?
        .filter_map(|r| {
            let p = r.pos();
            if (min_region_x..=max_region_x).contains(&p.x)
                && (min_region_z..=max_region_z).contains(&p.z)
            {
                Some(r)
            } else {
                None
            }
        })
        .collect::<Arc<[_]>>();

    println!("regions in view: {}", regions.len());

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

    let (mut handle, mut manager) = mca_viewer_rs::worker::Manager::<
        DummyNotify,
        BlockPalette,
        WgpuRenderCtx,
        GpuTile,
    >::new(DummyNotify, renderer.ctx(), num_workers);

    let _ = std::thread::spawn(move || manager.run());

    let opt = if let Some(slice_height) = args.slice_height {
        RenderOptions {
            mode: mca_viewer_rs::worker::RenderMode::Slice,
            slice_height,
            ..Default::default()
        }
    } else {
        RenderOptions::default()
    };

    let palette_weak = Arc::downgrade(&palette);

    let mut expecting = regions.len();
    handle.send_task(regions.clone(), opt, view, palette_weak.clone());

    let mut tiles = vec![];

    while expecting > 0 {
        match handle.recv_result() {
            Err(crossbeam_channel::RecvError) => {
                anyhow::bail!("channel disconnected")
            }
            Ok(r) => {
                if let Ok(t) = r.inner {
                    tiles.push(t)
                }
                expecting -= 1;
            }
        }
    }

    let scene = if args.region_borders {
        let mut scene = vello::Scene::new();
        for t in tiles.iter() {
            let r = rounded_affine.transform_rect_bbox(t.pos().rect()) + kurbo::Vec2::splat(0.5);

            let origin = r.origin();
            let size = (r.width() - 1., r.height() - 1.);
            let r2 = kurbo::Rect::from_origin_size(origin, size);

            scene.stroke(
                &kurbo::Stroke::new(1.),
                kurbo::Affine::IDENTITY,
                vello::peniko::Color::from_rgba8(255, 255, 255, 150),
                None,
                &r2,
            );
        }
        Some(scene)
    } else {
        None
    };

    let tex = renderer.render(
        tiles
            .iter()
            .map(|t| (rounded_affine.transform_rect_bbox(t.pos().rect()), &t.tile)),
        scene.as_ref(),
        w as f32,
        h as f32,
        1.,
    );

    let mut res = vec![];
    read_rgba_blocking(&device, &queue, &tex, &mut res)?;
    let img = image::RgbaImage::from_raw(w, h, res).unwrap();

    img.save(&args.out)?;

    Ok(())
}

pub fn read_rgba_blocking(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tex: &wgpu::Texture,
    dst: &mut Vec<u8>,
) -> anyhow::Result<()> {
    let (w, h) = (tex.width(), tex.height());

    let unpadded_w = w * 4;
    let padded_w = unpadded_w.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);

    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: padded_w as u64 * h as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_w),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);

    let (tx, rx) = std::sync::mpsc::sync_channel(1);

    slice.map_async(wgpu::MapMode::Read, move |r| _ = tx.send(r));

    device.poll(wgpu::PollType::wait_indefinitely())?;

    rx.try_recv()
        .map_err(|_| anyhow::anyhow!("buffer not mapped"))??;

    let data = slice.get_mapped_range()?;

    dst.reserve(unpadded_w as usize * h as usize);
    for row in data.chunks_exact(padded_w as usize) {
        dst.extend_from_slice(&row[..unpadded_w as usize]);
    }

    Ok(())
}
