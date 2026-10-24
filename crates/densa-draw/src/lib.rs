//! A frame's display list, independent of any GPU API.
//!
//! Painters draw through a [`Canvas`]: filled rectangles and glyphs (from a shared
//! `densa_text::GlyphCache`) become [`Instance`]s in an [`InstanceBuffer`], one quad each, in
//! the vertex layout `densa-render` uploads and draws in a single instanced draw call. Keeping
//! the list GPU-free makes every painter testable without a GPU, and lets other backends
//! draw the same list.
//!
//! Drawing allocates nothing once the frame's glyphs are cached.
//!
//! ```
//! use densa_draw::{Canvas, Color, InstanceBuffer, Rect};
//! use densa_text::{AtlasConfig, GlyphCache};
//!
//! // One glyph cache and one instance buffer per frame loop, reused every frame.
//! let mut cache = GlyphCache::new(AtlasConfig::default());
//! let mut instances = InstanceBuffer::with_capacity(1024);
//!
//! let mut canvas = Canvas::begin(&mut cache, &mut instances);
//! canvas.fill_rect(Rect::new(0.0, 0.0, 200.0, 20.0), Color::srgb(0x202020));
//! let saved = canvas.state();
//! canvas.clip(Rect::new(0.0, 0.0, 100.0, 100.0));
//! // Cut to the clip on the CPU: the frame stays one draw call.
//! canvas.fill_rect(Rect::new(50.0, 0.0, 200.0, 20.0), Color::WHITE);
//! canvas.restore(saved);
//!
//! assert_eq!(instances.len(), 2);
//! let cut = instances.as_slice()[1];
//! assert_eq!(cut.rect, [50.0, 0.0, 50.0, 20.0]); // x, y, width, height
//! ```

mod canvas;
mod instance;

pub use canvas::{Canvas, CanvasState};
pub use densa_core::{Color, Point, Rect, Size};
pub use instance::{Instance, InstanceBuffer};
