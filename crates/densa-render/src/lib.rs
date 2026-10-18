//! wgpu renderer for densa.
//!
//! [`GpuRenderer`] draws a frame's `densa_draw::Instance`s, as built by a `Canvas` from any
//! painter (grids, paragraphs, your own), with one instanced draw call: each quad is a solid
//! rectangle or a glyph sampled from the atlas. It keeps the atlas texture in step with the
//! glyph cache's updates, growing it on the GPU as the cache grows.

mod atlas;
mod gpu;

pub use atlas::AtlasError;
pub use densa_core::Color;
pub use gpu::GpuRenderer;
