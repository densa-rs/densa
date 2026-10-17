use crate::{GlyphAtlas, Instance, ATLAS_HEIGHT, ATLAS_WIDTH};
use bytemuck::{Pod, Zeroable};
use std::mem::size_of;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Globals {
    screen: [f32; 2],
    _pad: [f32; 2],
}

/// GPU resources for drawing a frame of [`Instance`]s in one draw call.
///
/// Everything is created up front; per frame only [`GpuGrid::prepare`] (two buffer
/// writes) and [`GpuGrid::render`] (one pass, one draw) run.
#[derive(Debug)]
pub struct GpuGrid {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    instances: wgpu::Buffer,
    capacity: usize,
}

impl GpuGrid {
    /// `format` must match the color target `render` will draw into; use an sRGB format
    /// (or an sRGB view of the surface) so linear shader colors display correctly.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        capacity: usize,
    ) -> Self {
        let atlas_view = upload_atlas(device, queue, &GlyphAtlas::font8x8());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("densa atlas sampler"),
            // Nearest: the bitmap font is drawn at integer scales and must stay crisp.
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("densa globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("densa instances"),
            size: (capacity.max(1) * size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = bind_group_layout(device);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("densa bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let pipeline = pipeline(device, &layout, format);

        Self {
            pipeline,
            bind_group,
            globals,
            instances,
            capacity,
        }
    }

    /// Instances the GPU buffer holds; more are clipped by [`GpuGrid::prepare`].
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Uploads the frame's instances for a `screen` of (width, height) physical pixels.
    /// Returns how many will be drawn (clipped to capacity).
    pub fn prepare(&self, queue: &wgpu::Queue, screen: (u32, u32), instances: &[Instance]) -> u32 {
        let globals = Globals {
            screen: [screen.0.max(1) as f32, screen.1.max(1) as f32],
            _pad: [0.0; 2],
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        let drawn = &instances[..instances.len().min(self.capacity)];
        if !drawn.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(drawn));
        }
        // Lossless: capacity is bounded by the GPU buffer size, far below u32::MAX.
        drawn.len() as u32
    }

    /// Clears `target` to `clear` and draws the first `count` prepared instances.
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clear: wgpu::Color,
        count: u32,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("densa grid"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..4, 0..count);
    }
}

fn upload_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &GlyphAtlas,
) -> wgpu::TextureView {
    let size = wgpu::Extent3d {
        width: ATLAS_WIDTH,
        height: ATLAS_HEIGHT,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("densa glyph atlas"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        atlas.pixels(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(ATLAS_WIDTH),
            rows_per_image: Some(ATLAS_HEIGHT),
        },
        size,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("densa bind group layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("densa grid shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("densa pipeline layout"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("densa grid pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<Instance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &ATTRIBUTES,
            })],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
