//! The window's surface and the renderer that draws a frame's instances to it.

use std::sync::Arc;

use densa_core::Color;
use densa_draw::Instance;
use densa_render::GpuRenderer;
use densa_text::GlyphCache;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::WindowError;

pub(crate) struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    view_format: wgpu::TextureFormat,
    renderer: GpuRenderer,
}

impl Gpu {
    /// A surface on `window`, presenting with V-Sync, and a renderer drawing up to
    /// `instances` rectangles and glyphs a frame. Limits `cache` to the GPU's texture size.
    pub(crate) fn new(
        event_loop: &ActiveEventLoop,
        window: &Arc<Window>,
        instances: usize,
        cache: &mut GlyphCache,
    ) -> Result<Self, WindowError> {
        let gpu = |e: &dyn std::fmt::Display| WindowError::Gpu(e.to_string());
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(event_loop.owned_display_handle()),
        ));
        let surface = instance
            .create_surface(Arc::clone(window))
            .map_err(|e| gpu(&e))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| gpu(&e))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("densa"),
            ..Default::default()
        }))
        .map_err(|e| gpu(&e))?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| WindowError::Gpu("the GPU cannot present to this window".into()))?;
        // FIFO is V-Sync: presentation waits for the display's refresh and never tears.
        config.present_mode = wgpu::PresentMode::Fifo;
        let view_format = config.format.add_srgb_suffix();
        if view_format != config.format {
            config.view_formats.push(view_format);
        }
        surface.configure(&device, &config);
        cache
            .limit_side(GpuRenderer::max_atlas_side(&device))
            .map_err(|e| gpu(&e))?;
        let renderer = GpuRenderer::new(&device, view_format, instances);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            view_format,
            renderer,
        })
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return; // Minimized: keep the configuration until restored.
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Uploads new glyphs, draws `instances` over `background` and presents. `Ok(false)`:
    /// the frame was skipped (the surface was busy, hidden or out of date).
    pub(crate) fn draw(
        &mut self,
        window: &Window,
        cache: &mut GlyphCache,
        instances: &[Instance],
        background: Color,
    ) -> Result<bool, WindowError> {
        self.renderer
            .apply_atlas_updates(&self.device, &self.queue, cache.drain_updates())
            .map_err(|e| WindowError::Gpu(e.to_string()))?;
        let size = (self.config.width, self.config.height);
        let count = self.renderer.prepare(&self.queue, size, instances);
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                // Present this one, then reconfigure for the next.
                self.surface.configure(&self.device, &self.config);
                frame
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(WindowError::Gpu(
                    "the surface texture failed validation".into(),
                ));
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.view_format),
            ..Default::default()
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.renderer.render(&mut encoder, &view, background, count);
        self.queue.submit([encoder.finish()]);
        window.pre_present_notify();
        self.queue.present(frame);
        Ok(true)
    }
}
