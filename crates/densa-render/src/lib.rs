//! wgpu renderer for densa.
//!
//! [`GpuRenderer`] draws a frame's `densa_draw::Instance`s, as built by a `Canvas` from any
//! painter (grids, paragraphs, your own), with one instanced draw call: each quad is a solid
//! rectangle or a glyph sampled from the atlas. It keeps the atlas texture in step with the
//! glyph cache's updates, growing it on the GPU as the cache grows.
//!
//! Applications built on `densa-window` never call it. A frame loop of your own does:
//!
//! ```no_run
//! use densa_draw::{Canvas, InstanceBuffer, Rect};
//! use densa_render::{AtlasError, Color, GpuRenderer};
//! use densa_text::{AtlasConfig, GlyphCache};
//!
//! /// Once per device: the renderer, and a glyph cache kept within the device's texture limit.
//! fn setup(device: &wgpu::Device, format: wgpu::TextureFormat) -> (GpuRenderer, GlyphCache) {
//!     let renderer = GpuRenderer::new(device, format, 65_536);
//!     let mut cache = GlyphCache::new(AtlasConfig::default());
//!     let _ = cache.limit_side(GpuRenderer::max_atlas_side(device));
//!     (renderer, cache)
//! }
//!
//! /// Each frame: paint through a canvas, follow the glyph atlas, draw in one call.
//! fn frame(
//!     device: &wgpu::Device,
//!     queue: &wgpu::Queue,
//!     (renderer, cache): (&mut GpuRenderer, &mut GlyphCache),
//!     instances: &mut InstanceBuffer,
//!     target: &wgpu::TextureView,
//!     size: (u32, u32),
//! ) -> Result<(), AtlasError> {
//!     let mut canvas = Canvas::begin(cache, instances);
//!     canvas.fill_rect(Rect::new(10.0, 10.0, 100.0, 20.0), Color::WHITE);
//!     // Grids, paragraphs and charts paint onto the same canvas here.
//!     renderer.apply_atlas_updates(device, queue, cache.drain_updates())?;
//!     let count = renderer.prepare(queue, size, instances.as_slice());
//!     let mut encoder = device.create_command_encoder(&Default::default());
//!     renderer.render(&mut encoder, target, Color::BLACK, count);
//!     queue.submit([encoder.finish()]);
//!     Ok(())
//! }
//! ```

mod atlas;
mod gpu;

pub use atlas::AtlasError;
pub use densa_core::Color;
pub use gpu::GpuRenderer;
