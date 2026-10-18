use crate::atlas::{AtlasError, GpuAtlas};
use bytemuck::{Pod, Zeroable};
use densa_core::Color;
use densa_draw::Instance;
use densa_text::AtlasUpdate;
use std::mem::size_of;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Globals {
    screen: [f32; 2],
    _pad: [f32; 2],
}

/// GPU resources for drawing a frame of [`Instance`]s in one draw call.
///
/// Everything is created up front; per frame only [`GpuRenderer::apply_atlas_updates`] (work
/// only when new glyphs appeared), [`GpuRenderer::prepare`] (two buffer writes) and
/// [`GpuRenderer::render`] (one pass, one draw) run.
#[derive(Debug)]
pub struct GpuRenderer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    globals: wgpu::Buffer,
    instances: wgpu::Buffer,
    atlas: GpuAtlas,
    /// `atlas.version()` that `bind_group` refers to.
    bound_atlas: u64,
    /// The first atlas failure; once set the atlas no longer mirrors the cache.
    failed: Option<AtlasError>,
    capacity: usize,
}

impl GpuRenderer {
    /// A renderer drawing up to `capacity` instances. Its glyph atlas starts empty and follows
    /// the glyph cache through [`GpuRenderer::apply_atlas_updates`].
    ///
    /// `format` must match the color target `render` will draw into; use an sRGB format
    /// (or an sRGB view of the surface) so linear shader colors display correctly.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, capacity: usize) -> Self {
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
        let atlas = GpuAtlas::new(device);
        let bind_group_layout = bind_group_layout(device);
        let bind_group = bind_group(device, &bind_group_layout, &globals, &atlas, &sampler);
        let pipeline = pipeline(device, &bind_group_layout, format);

        Self {
            pipeline,
            bind_group_layout,
            bind_group,
            sampler,
            globals,
            instances,
            bound_atlas: atlas.version(),
            failed: None,
            atlas,
            capacity,
        }
    }

    /// Instances the GPU buffer holds; more are clipped by [`GpuRenderer::prepare`].
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The largest atlas side `device` supports; pass it to
    /// `GlyphCache::limit_side` so the cache never outgrows the device.
    pub fn max_atlas_side(device: &wgpu::Device) -> u32 {
        device.limits().max_texture_dimension_2d
    }

    /// The atlas failure latched by [`apply_atlas_updates`](Self::apply_atlas_updates), if
    /// any: then the renderer draws nothing until it is recreated.
    pub fn atlas_error(&self) -> Option<AtlasError> {
        self.failed
    }

    /// Current glyph atlas size in texels, `(width, height)`.
    pub fn atlas_size(&self) -> (u32, u32) {
        self.atlas.size()
    }

    /// Applies the glyph cache's atlas changes in order (`GlyphCache::drain_updates`):
    /// grows the texture, keeping its texels, and copies new glyphs in. Returns how many
    /// glyph writes were applied.
    ///
    /// # Errors
    /// [`AtlasError`] if the texture can no longer mirror the cache (too large for the device,
    /// or a malformed update). The error is latched: later updates are not applied, every
    /// later call returns it, and [`prepare`](Self::prepare) draws nothing, since glyphs
    /// would sample the wrong texels. Recreate the renderer and the glyph cache to recover.
    pub fn apply_atlas_updates(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        updates: impl IntoIterator<Item = AtlasUpdate>,
    ) -> Result<usize, AtlasError> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        let result = self.atlas.apply(device, queue, updates);
        self.failed = result.err();
        // Rebuilt even on error: a resize before the failing update replaced the texture.
        if self.atlas.version() != self.bound_atlas {
            self.bind_group = bind_group(
                device,
                &self.bind_group_layout,
                &self.globals,
                &self.atlas,
                &self.sampler,
            );
            self.bound_atlas = self.atlas.version();
        }
        result
    }

    /// Uploads the frame's instances for a `screen` of (width, height) physical pixels.
    /// Returns how many will be drawn: clipped to capacity, and 0 after an atlas failure.
    pub fn prepare(&self, queue: &wgpu::Queue, screen: (u32, u32), instances: &[Instance]) -> u32 {
        if self.failed.is_some() {
            return 0;
        }
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
        clear: Color,
        count: u32,
    ) {
        let clear = wgpu::Color {
            r: f64::from(clear.r),
            g: f64::from(clear.g),
            b: f64::from(clear.b),
            a: f64::from(clear.a),
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("densa instances"),
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

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    globals: &wgpu::Buffer,
    atlas: &GpuAtlas,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    let view = atlas.view();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("densa bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
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
                // Vertex too: the vertex shader normalizes texel UVs by the atlas size.
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
        label: Some("densa shader"),
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
        label: Some("densa pipeline"),
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
