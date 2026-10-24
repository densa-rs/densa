//! Real text for densa: fonts, glyph rasterization and a glyph atlas cache.
//!
//! Domain-agnostic: any widget that draws text goes through these three pieces.
//!
//! - [`Font`] wraps an OpenType/TrueType font (skrifa): character mapping, advances, line
//!   metrics. [`Font::bundled_mono`] is JetBrains Mono, shipped under the SIL OFL 1.1.
//! - [`rasterize`] turns a glyph outline into an 8-bit coverage bitmap (zeno), optionally
//!   shifted by a fraction of a pixel.
//! - [`GlyphCache`] packs rasterized glyphs into a paged texture atlas (etagere) that grows a
//!   page at a time and, once at its limit, evicts the least recently used glyphs (never the
//!   current frame's); it queues the [`AtlasUpdate`]s the GPU texture must follow.
//!
//! A warm cache answers lookups without allocating, so per-frame text drawing stays
//! allocation-free; rasterizing a glyph seen for the first time allocates, once.
//!
//! ```
//! use densa_text::{quantize_x, AtlasConfig, Font, GlyphCache, GlyphMap};
//!
//! let mut chars = GlyphMap::new(Font::bundled_mono()?, 14.0);
//! let mut cache = GlyphCache::new(AtlasConfig::default());
//!
//! cache.begin_frame();
//! let a = chars.get('A').expect("the bundled font has A");
//! let (_, subpixel) = quantize_x(10.25); // a pen at x = 10.25 px
//! let placed = cache.get(chars.font(), a.id, chars.px(), subpixel)?;
//! assert!(placed.size[0] > 0.0, "rasterized into the atlas");
//! assert_eq!(cache.cached_glyphs(), 1);
//!
//! // Asked again (next frame, say): found, not rasterized again.
//! cache.begin_frame();
//! cache.get(chars.font(), a.id, chars.px(), subpixel)?;
//! assert_eq!(cache.cached_glyphs(), 1);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod cache;
mod font;
mod glyph_map;
mod raster;
mod run;

pub use cache::{
    quantize_x, AtlasConfig, AtlasLimitError, AtlasUpdate, CacheStats, CachedGlyph, GlyphCache,
    GlyphUnavailable, GlyphUpload, SUBPIXEL_STEPS,
};
pub use font::{Font, FontBytes, FontError, FontId, GlyphId, LineMetrics, BUNDLED_MONO};
pub use glyph_map::{GlyphMap, MappedGlyph, MAX_CACHED_CHARS};
pub use raster::{rasterize, GlyphBitmap};
pub use run::{GlyphRun, PositionedGlyph};
