use slint::wgpu_30::wgpu;
use vello::kurbo::Rect;
mod wgpu_render;
pub use wgpu_render::{GpuTile, RenderCtx};

pub const TEX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

const VELLO_USAGE: wgpu::TextureUsages =
    wgpu::TextureUsages::STORAGE_BINDING.union(wgpu::TextureUsages::TEXTURE_BINDING);

const SLINT_USAGE: wgpu::TextureUsages =
    wgpu::TextureUsages::TEXTURE_BINDING.union(wgpu::TextureUsages::RENDER_ATTACHMENT);

pub struct Renderer {
    texture_dimensions: (u32, u32),
    device: wgpu::Device,
    queue: wgpu::Queue,

    vello_renderer: vello::Renderer,

    vello_tex: wgpu::Texture,
    vello_view: wgpu::TextureView,
    vello_tile: GpuTile,

    slint_tex: wgpu::Texture,
    slint_view: wgpu::TextureView,

    wgpu_renderer: wgpu_render::WgpuRenderer,
}

impl Renderer {
    pub fn ctx(&self) -> RenderCtx {
        self.wgpu_renderer.ctx()
    }

    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let vello_tex = make_texture(&device, "vello_target", VELLO_USAGE, 1, 1);
        let vello_view = vello_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let slint_tex = make_texture(&device, "slint_target", SLINT_USAGE, 1, 1);
        let slint_view = slint_tex.create_view(&wgpu::TextureViewDescriptor::default());

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
            vello_tile,
            vello_renderer,
            texture_dimensions: (1, 1),
            device,
            queue,
            vello_tex,
            vello_view,
            slint_tex,
            slint_view,
            wgpu_renderer,
        }
    }

    fn resize_textures(&mut self, w: f32, h: f32, s: f32) {
        let (tw, th) = texture_dimensions(w, h, s);
        if (tw, th) == self.texture_dimensions {
            return;
        }

        self.texture_dimensions = (tw, th);

        self.vello_tex = make_texture(&self.device, "vello_target", VELLO_USAGE, tw, th);
        self.vello_view = self
            .vello_tex
            .create_view(&wgpu::TextureViewDescriptor::default());

        self.vello_tile = self.wgpu_renderer.tile_from_view(&self.vello_view, tw, th);

        self.slint_tex = make_texture(&self.device, "slint_target", SLINT_USAGE, tw, th);
        self.slint_view = self
            .slint_tex
            .create_view(&wgpu::TextureViewDescriptor::default());
    }

    pub fn render<'a>(
        &'a mut self,
        tiles: impl Iterator<Item = (Rect, &'a GpuTile)>,
        scene: &vello::Scene,
        w: f32,
        h: f32,
        s: f32,
    ) -> wgpu::Texture {
        self.resize_textures(w, h, s);

        let (tw, th) = self.texture_dimensions;

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

        let mut encoder = self.device.create_command_encoder(&Default::default());

        let full_screen = Rect::from_origin_size((0., 0.), (tw as f64, th as f64));

        let overlay = (full_screen, &self.vello_tile);

        self.wgpu_renderer.render(
            &mut encoder,
            &self.slint_view,
            tw,
            th,
            tiles.chain(std::iter::once(overlay)),
        );

        self.queue.submit(Some(encoder.finish()));
        self.slint_tex.clone()
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
