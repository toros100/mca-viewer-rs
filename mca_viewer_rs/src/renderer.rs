use slint::wgpu_30::wgpu;
use vello::kurbo::Rect;
mod wgpu_render;
pub use wgpu_render::{GpuTile, WgpuRenderCtx, WgpuRenderer};

pub const TEX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub const VELLO_USAGES: wgpu::TextureUsages =
    wgpu::TextureUsages::STORAGE_BINDING.union(wgpu::TextureUsages::TEXTURE_BINDING);

pub const SLINT_USAGES: wgpu::TextureUsages =
    wgpu::TextureUsages::TEXTURE_BINDING.union(wgpu::TextureUsages::RENDER_ATTACHMENT);

pub struct Renderer {
    texture_dimensions: (u32, u32),
    device: wgpu::Device,
    queue: wgpu::Queue,

    vello_renderer: vello::Renderer,

    vello_usages: wgpu::TextureUsages,
    dst_usages: wgpu::TextureUsages,

    vello_tex: wgpu::Texture,
    vello_view: wgpu::TextureView,
    vello_tile: GpuTile,

    dst_tex: wgpu::Texture,
    dst_view: wgpu::TextureView,

    wgpu_renderer: wgpu_render::WgpuRenderer,
}

impl Renderer {
    pub fn ctx(&self) -> WgpuRenderCtx {
        self.wgpu_renderer.ctx()
    }

    // only required for headless binary (needs COPY_SRC to read back rgba data)
    pub fn new_with_usages(
        device: wgpu::Device,
        queue: wgpu::Queue,
        vello_usages: wgpu::TextureUsages,
        dst_usages: wgpu::TextureUsages,
    ) -> Self {
        let vello_tex = make_texture(&device, "vello", vello_usages, 1, 1);
        let vello_view = vello_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let dst_tex = make_texture(&device, "dst", dst_usages, 1, 1);
        let dst_view = dst_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let wgpu_renderer = wgpu_render::WgpuRenderer::new(&device, &queue, TEX_FORMAT);

        let vello_tile = wgpu_renderer.tile_from_view(&vello_view, 1, 1);

        let vello_renderer = vello::Renderer::new(
            &device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::area_only(),
                ..Default::default()
            },
        )
        .unwrap();

        Self {
            vello_usages,
            dst_usages,
            vello_tile,
            vello_renderer,
            texture_dimensions: (1, 1),
            device,
            queue,
            vello_tex,
            vello_view,
            dst_tex,
            dst_view,
            wgpu_renderer,
        }
    }

    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self::new_with_usages(device, queue, VELLO_USAGES, SLINT_USAGES)
    }

    fn resize_textures(&mut self, w: f32, h: f32, s: f32) {
        let (tw, th) = texture_dimensions(w, h, s);
        if (tw, th) == self.texture_dimensions {
            return;
        }

        self.texture_dimensions = (tw, th);

        self.vello_tex = make_texture(&self.device, "vello_target", self.vello_usages, tw, th);
        self.vello_view = self
            .vello_tex
            .create_view(&wgpu::TextureViewDescriptor::default());

        self.vello_tile = self.wgpu_renderer.tile_from_view(&self.vello_view, tw, th);

        self.dst_tex = make_texture(&self.device, "slint_target", self.dst_usages, tw, th);
        self.dst_view = self
            .dst_tex
            .create_view(&wgpu::TextureViewDescriptor::default());
    }

    pub fn render<'a>(
        &'a mut self,
        tiles: impl Iterator<Item = (Rect, &'a GpuTile)>,
        scene: Option<&vello::Scene>,
        w: f32,
        h: f32,
        s: f32,
    ) -> wgpu::Texture {
        self.resize_textures(w, h, s);

        let (tw, th) = self.texture_dimensions;

        let mut encoder = self.device.create_command_encoder(&Default::default());

        if let Some(scene) = scene {
            self.vello_renderer
                .render_to_texture(
                    &self.device,
                    &self.queue,
                    scene,
                    &self.vello_view,
                    &vello::RenderParams {
                        base_color: vello::peniko::color::palette::css::TRANSPARENT,
                        width: tw,
                        height: th,
                        antialiasing_method: vello::AaConfig::Area,
                    },
                )
                .unwrap();

            let full_screen = Rect::from_origin_size((0., 0.), (tw as f64, th as f64));

            let overlay = (full_screen, &self.vello_tile);

            self.wgpu_renderer.render(
                &mut encoder,
                &self.dst_view,
                tw,
                th,
                tiles.chain(std::iter::once(overlay)),
            );
        } else {
            self.wgpu_renderer
                .render(&mut encoder, &self.dst_view, tw, th, tiles)
        }

        self.queue.submit(Some(encoder.finish()));
        self.dst_tex.clone()
    }
}

pub fn make_texture<'a>(
    device: &wgpu::Device,
    label: impl Into<Option<&'a str>>,
    usage: wgpu::TextureUsages,
    w: u32,
    h: u32,
) -> wgpu::Texture {
    debug_assert_ne!(w, 0);
    debug_assert_ne!(h, 0);

    device.create_texture(&wgpu::TextureDescriptor {
        label: label.into(),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TEX_FORMAT,
        usage,
        view_formats: &[],
    })
}

pub fn texture_dimensions(
    logical_width: f32,
    logical_height: f32,
    scale_factor: f32,
) -> (u32, u32) {
    let actual_w = (logical_width as f64 * scale_factor as f64)
        .round()
        .max(1.0) as u32;
    let actual_h = (logical_height as f64 * scale_factor as f64)
        .round()
        .max(1.0) as u32;
    (actual_w, actual_h)
}
