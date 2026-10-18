//! Real text for densa: fonts, glyph rasterization and a glyph atlas cache.
//!
//! Domain-agnostic: any widget that draws text goes through these three pieces.
//!
//! - [`Font`] wraps an OpenType/TrueType font (skrifa): character mapping, advances, line
//!   metrics. [`Font::bundled_mono`] is JetBrains Mono, shipped under the SIL OFL 1.1.
//! - [`rasterize`] turns a glyph outline into an 8-bit coverage bitmap (zeno), optionally
//!   shifted by a fraction of a pixel.
//! - [`GlyphCache`] packs rasterized glyphs into a texture atlas (etagere), evicting the least
//!   recently used ones when full, and queues the pixels the GPU must upload.
//!
//! A warm cache answers lookups without allocating, so per-frame text drawing stays
//! allocation-free; rasterizing a glyph seen for the first time allocates, once.

mod cache;
mod font;
mod raster;

pub use cache::{quantize_x, CacheStats, CachedGlyph, GlyphCache, GlyphUpload, SUBPIXEL_STEPS};
pub use font::{Font, FontError, FontId, GlyphId, LineMetrics, BUNDLED_MONO};
pub use raster::{rasterize, GlyphBitmap};
