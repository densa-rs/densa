use crate::Instance;
use bytemuck::{Pod, Zeroable};
use densa_text::GlyphUpload;
use std::mem::size_of;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Globals {
    screen: [f32; 2],
    _pad: [f32; 2],
}

/// GPU resources for drawing a frame of [`Instance`]s in one draw call.
///
/// Everything is created up front; per frame only [`GpuGrid::upload_glyphs`] (only when
/// new glyphs appeared), [`GpuGrid::prepare`] (two buffer writes) and [`GpuGrid::render`]
/// (one pass, one draw) run.
#[derive(Debug)]
pub struct GpuGrid {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    instances: wgpu::Buffer,
    atlas: wgpu::Texture,
    capacity: usize,
}

impl GpuGrid {
    /// A grid drawing up to `capacity` instances with glyphs from an empty
    /// `atlas_size` x `atlas_size` coverage atlas, filled by [`GpuGrid::upload_glyphs`].
    ///
    /// Pass the painter's `GridPainter::atlas_size()`: glyph UVs are normalized by the cache's
    /// atlas size, so a different texture size samples the wrong texels.
    ///
    /// `format` must match the color target `render` will draw into; use an sRGB format
    /// (or an sRGB view of the surface) so linear shader colors display correctly.
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        capacity: usize,
        atlas_size: u32,
    ) -> Self {
        let atlas = create_atlas(device, atlas_size);
        let atlas_view = atlas.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("densa atlas sampler"),
            // Nearest: glyphs are drawn texel-for-pixel at whole-pixel positions.
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
            atlas,
            capacity,
        }
    }

    /// Instances the GPU buffer holds; more are clipped by [`GpuGrid::prepare`].
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Side of the square glyph atlas in pixels.
    pub fn atlas_size(&self) -> u32 {
        self.atlas.width()
    }

    /// Copies newly rasterized glyphs into the atlas, in order. Returns how many were
    /// written; uploads that do not lie inside the atlas are skipped.
    pub fn upload_glyphs(
        &self,
        queue: &wgpu::Queue,
        uploads: impl IntoIterator<Item = GlyphUpload>,
    ) -> usize {
        let side = self.atlas_size();
        let mut written = 0;
        for upload in uploads {
            let inside = upload
                .x
                .checked_add(upload.width)
                .is_some_and(|r| r <= side)
                && upload
                    .y
                    .checked_add(upload.height)
                    .is_some_and(|b| b <= side);
            let complete =
                upload.coverage.len() as u64 == u64::from(upload.width) * u64::from(upload.height);
            debug_assert!(inside && complete, "malformed glyph upload {upload:?}");
            if upload.width == 0 || upload.height == 0 || !inside || !complete {
                continue;
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: upload.x,
                        y: upload.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &upload.coverage,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(upload.width),
                    rows_per_image: Some(upload.height),
                },
                wgpu::Extent3d {
                    width: upload.width,
                    height: upload.height,
                    depth_or_array_layers: 1,
                },
            );
            written += 1;
        }
        written
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

/// An R8 coverage atlas; wgpu zero-initializes it, so unwritten texels are transparent.
fn create_atlas(device: &wgpu::Device, side: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("densa glyph atlas"),
        size: wgpu::Extent3d {
            width: side.max(1),
            height: side.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
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
