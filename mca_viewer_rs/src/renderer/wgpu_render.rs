use slint::wgpu_30::wgpu;
use vello::kurbo::Rect;

use crate::worker::RenderCtx;

const TILES_MODULE_DESCR: wgpu::ShaderModuleDescriptor = wgpu::include_wgsl!("tiles.wgsl");

const TILE_USAGE: wgpu::TextureUsages =
    wgpu::TextureUsages::TEXTURE_BINDING.union(wgpu::TextureUsages::COPY_DST);

const INSTANCE_ATTRS: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2];

#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Instance {
    min: [f32; 2],
    size: [f32; 2],
}

#[derive(Clone)]
#[allow(unused)]
pub struct GpuTile {
    texture: Option<wgpu::Texture>, // None when wrapping an external view
    bind_group: wgpu::BindGroup,
    width: u32,
    height: u32,
}

#[derive(Clone)]
pub struct WgpuRenderCtx {
    device: wgpu::Device,
    queue: wgpu::Queue,
    tile_bgl: wgpu::BindGroupLayout,
}

impl WgpuRenderCtx {
    pub fn upload_tile(&self, rgba: &[u8], width: u32, height: u32) -> GpuTile {
        WgpuRenderer::upload_tile_inner(
            &self.tile_bgl,
            &self.device,
            &self.queue,
            rgba,
            width,
            height,
        )
    }
    pub fn update_tile(&self, tile: &GpuTile, rgba: &[u8]) {
        WgpuRenderer::update_tile(&self.queue, tile, rgba);
    }
}

impl RenderCtx<GpuTile> for WgpuRenderCtx {
    fn upload_tile(&self, data: &[u8], width: u32, height: u32) -> GpuTile {
        WgpuRenderCtx::upload_tile(self, data, width, height)
    }
}

pub struct WgpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,

    tile_bgl: wgpu::BindGroupLayout,

    _sampler_bgl: wgpu::BindGroupLayout,
    sampler_bg: wgpu::BindGroup,
    _sampler: wgpu::Sampler,

    pipeline: wgpu::RenderPipeline,

    instances: wgpu::Buffer,
    instance_cap: u64,

    instances_scratch: Vec<Instance>,
    bg_scratch: Vec<&'static wgpu::BindGroup>,
}

impl WgpuRenderer {
    pub fn ctx(&self) -> WgpuRenderCtx {
        WgpuRenderCtx {
            device: self.device.clone(),
            queue: self.queue.clone(),
            tile_bgl: self.tile_bgl.clone(),
        }
    }

    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let module = device.create_shader_module(TILES_MODULE_DESCR);

        let _sampler_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sampler bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            }],
        });

        let _sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let sampler_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sampler bg"),
            layout: &_sampler_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Sampler(&_sampler),
            }],
        });

        let tile_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tile texture"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tiles"),
            bind_group_layouts: &[Some(&_sampler_bgl), Some(&tile_bgl)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tiles"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &INSTANCE_ATTRS,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                cull_mode: None, // y-flip / mirrored cameras change winding
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    // NOTE: should this be BlendState::PREMULTIPLIED_ALPHA_BLENDING?
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let instance_cap_initial = 256;
        let instances = Self::make_instance_buffer(device, instance_cap_initial);

        Self {
            queue: queue.clone(),
            pipeline,
            _sampler_bgl,
            tile_bgl,
            _sampler,
            sampler_bg,
            instances,
            bg_scratch: vec![],
            instance_cap: instance_cap_initial,
            instances_scratch: Vec::with_capacity(instance_cap_initial as usize),
            device: device.clone(),
        }
    }

    fn make_instance_buffer(device: &wgpu::Device, cap: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tile instances"),
            size: cap * std::mem::size_of::<Instance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn upload_tile_inner(
        tile_bgl: &wgpu::BindGroupLayout,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> GpuTile {
        let texture = super::make_texture(device, "tile", TILE_USAGE, width, height);
        write_rgba(queue, &texture, rgba);
        let view = texture.create_view(&Default::default());
        let mut tile = Self::tile_from_view_inner(tile_bgl, device, &view, width, height);
        tile.texture = Some(texture);
        tile
    }

    fn update_tile(queue: &wgpu::Queue, tile: &GpuTile, rgba: &[u8]) {
        // TODO: should not silently do nothing
        // if the GpuTile does not have a texture stored, we can't assume it has COPY_DST usage
        if let Some(t) = &tile.texture {
            write_rgba(queue, t, rgba);
        }
    }

    // NOTE: should probably take w/h, since they cant be obtained from the view in general
    pub fn tile_from_view(&self, view: &wgpu::TextureView, width: u32, height: u32) -> GpuTile {
        Self::tile_from_view_inner(&self.tile_bgl, &self.device, view, width, height)
    }

    // wrap TextureView in GpuTile (for the separately rendered vello overlay)
    fn tile_from_view_inner(
        tile_bgl: &wgpu::BindGroupLayout,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> GpuTile {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tile (from view)"),
            layout: tile_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            }],
        });

        GpuTile {
            texture: None,
            bind_group,
            width,
            height,
        }
    }

    fn take_bg_scratch<'a>(&mut self) -> Vec<&'a wgpu::BindGroup> {
        std::mem::take(&mut self.bg_scratch)
    }

    fn replace_bg_scratch(&mut self, mut v: Vec<&wgpu::BindGroup>) {
        v.clear();
        let (ptr, _, capacity) = v.into_raw_parts();
        self.bg_scratch = unsafe { Vec::from_raw_parts(ptr.cast(), 0, capacity) }
    }

    pub fn render<'a>(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        target_w: u32,
        target_h: u32,
        tiles: impl Iterator<Item = (Rect, &'a GpuTile)>,
    ) {
        self.instances_scratch.clear();
        let mut bind_groups = self.take_bg_scratch();
        bind_groups.clear();

        fn to_clip_space(rect: Rect, w: f64, h: f64) -> Instance {
            let r = rect;
            let (x, y) = r.origin().into();
            let (rw, rh) = (r.width(), r.height());

            let min = [(x / w * 2.0 - 1.0) as f32, (1.0 - y / h * 2.0) as f32];
            let size = [(rw / w * 2.0) as f32, (-rh / h * 2.0) as f32];
            Instance { min, size }
        }

        for (r, t) in tiles {
            self.instances_scratch
                .push(to_clip_space(r, target_w as f64, target_h as f64));
            bind_groups.push(&t.bind_group)
        }

        if self.instances_scratch.is_empty() {
            self.replace_bg_scratch(bind_groups);
            return;
        }

        if self.instances_scratch.len() as u64 > self.instance_cap {
            // "inheriting" vec growth factor for the wgpu buffer
            self.instance_cap = self.instances_scratch.capacity() as u64;
            self.instances = Self::make_instance_buffer(&self.device, self.instance_cap)
        }

        self.queue.write_buffer(
            &self.instances,
            0,
            bytemuck::cast_slice(&self.instances_scratch),
        );

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("tiles"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.set_bind_group(0, &self.sampler_bg, &[]);

        for (i, &bg) in bind_groups.iter().enumerate() {
            let i = i as u32;
            pass.set_bind_group(1, bg, &[]);
            pass.draw(0..4, i..i + 1);
        }
    }
}

fn write_rgba(queue: &wgpu::Queue, texture: &wgpu::Texture, rgba: &[u8]) {
    let size = texture.size();
    debug_assert_eq!(rgba.len(), (size.width * size.height * 4) as usize);
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * size.width),
            rows_per_image: Some(size.height),
        },
        size,
    );
}
