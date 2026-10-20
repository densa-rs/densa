//! The window's surface and the renderer that draws a frame's instances to it.

use std::sync::Arc;

use densa_core::Color;
use densa_draw::Instance;
use densa_render::GpuRenderer;
use densa_text::GlyphCache;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::{Backend, WindowError};

/// The graphics APIs `backend` names, unless `from_env` (`WGPU_BACKEND`, read by the caller)
/// names others: the variable stays the way to try another API without rebuilding.
pub(crate) fn backends(backend: Backend, from_env: Option<wgpu::Backends>) -> wgpu::Backends {
    if let Some(chosen) = from_env.filter(|b| !b.is_empty()) {
        return chosen;
    }
    match backend {
        Backend::Auto => wgpu::Backends::all(),
        Backend::Vulkan => wgpu::Backends::VULKAN,
        Backend::Metal => wgpu::Backends::METAL,
        Backend::Dx12 => wgpu::Backends::DX12,
        Backend::Gl => wgpu::Backends::GL,
    }
}

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
    /// `instances` rectangles and glyphs a frame, through `backend` (or what `WGPU_BACKEND`
    /// names). When those give no device, every API is tried before giving up. Limits
    /// `cache` to the GPU's texture size.
    pub(crate) fn new(
        event_loop: &ActiveEventLoop,
        window: &Arc<Window>,
        instances: usize,
        cache: &mut GlyphCache,
        backend: Backend,
    ) -> Result<Self, WindowError> {
        let chosen = backends(backend, wgpu::Backends::from_env());
        match Self::open(event_loop, window, instances, cache, chosen) {
            Err(_) if chosen != wgpu::Backends::all() => {
                Self::open(event_loop, window, instances, cache, wgpu::Backends::all())
            }
            opened => opened,
        }
    }

    fn open(
        event_loop: &ActiveEventLoop,
        window: &Arc<Window>,
        instances: usize,
        cache: &mut GlyphCache,
        backends: wgpu::Backends,
    ) -> Result<Self, WindowError> {
        let gpu = |e: &dyn std::fmt::Display| WindowError::Gpu(e.to_string());
        let mut descriptor = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
            event_loop.owned_display_handle(),
        ));
        descriptor.backends = backends;
        let instance = wgpu::Instance::new(descriptor);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_backend_names_its_api_and_auto_names_every_one() {
        assert_eq!(backends(Backend::Auto, None), wgpu::Backends::all());
        assert_eq!(backends(Backend::Vulkan, None), wgpu::Backends::VULKAN);
        assert_eq!(backends(Backend::Metal, None), wgpu::Backends::METAL);
        assert_eq!(backends(Backend::Dx12, None), wgpu::Backends::DX12);
        assert_eq!(backends(Backend::Gl, None), wgpu::Backends::GL);
    }

    #[test]
    fn the_environment_overrides_the_application_unless_it_names_nothing() {
        let from_env = Some(wgpu::Backends::VULKAN | wgpu::Backends::GL);

        assert_eq!(
            backends(Backend::Dx12, from_env),
            wgpu::Backends::VULKAN | wgpu::Backends::GL
        );
        assert_eq!(
            backends(Backend::Gl, Some(wgpu::Backends::empty())),
            wgpu::Backends::GL
        );
    }

    #[test]
    fn a_window_draws_with_the_api_its_options_name() {
        assert_eq!(crate::WindowOptions::new("t").backend, Backend::Auto);
        assert_eq!(
            crate::WindowOptions::new("t")
                .with_backend(Backend::Gl)
                .backend,
            Backend::Gl
        );
    }
}
