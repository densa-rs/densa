//! A frame's display list, independent of any GPU API.
//!
//! Painters draw through a [`Canvas`]: filled rectangles and glyphs (from a shared
//! `densa_text::GlyphCache`) become [`Instance`]s in an [`InstanceBuffer`], one quad each, in
//! the vertex layout `densa-render` uploads and draws in a single instanced draw call. Keeping
//! the list GPU-free makes every painter testable without a GPU, and lets other backends
//! draw the same list.
//!
//! Drawing allocates nothing once the frame's glyphs are cached.

mod canvas;
mod instance;

pub use canvas::Canvas;
pub use densa_core::{Color, Point, Rect};
pub use instance::{Instance, InstanceBuffer};
