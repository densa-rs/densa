use etagere::{size2, AllocId, Allocation, AtlasAllocator};
use rustc_hash::FxHashMap;

use crate::{rasterize, Font, FontId, GlyphId};

/// Subpixel positions per pixel: glyphs are rasterized at 0, 1/4, 1/2 and 3/4 px offsets.
pub const SUBPIXEL_STEPS: u8 = 4;

/// Transparent border around every glyph in the atlas, so sampling never bleeds into a
/// neighbour.
const PADDING: u32 = 1;

/// Splits a horizontal position into a whole pixel and a subpixel bin (`0..SUBPIXEL_STEPS`),
/// rounding to the nearest bin. Draw the glyph cached for `bin` at the returned pixel.
pub fn quantize_x(x: f32) -> (i32, u8) {
    let steps = i32::from(SUBPIXEL_STEPS);
    // Lossless enough: screen coordinates are far below 2^31 / SUBPIXEL_STEPS.
    let scaled = (x * steps as f32).round() as i32;
    // Lossless: rem_euclid keeps the value in 0..SUBPIXEL_STEPS.
    (scaled.div_euclid(steps), scaled.rem_euclid(steps) as u8)
}

/// A glyph ready to draw from the atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CachedGlyph {
    /// `[u0, v0, u1, v1]`, normalized atlas coordinates.
    pub uv: [f32; 4],
    /// Bitmap size in pixels; `[0, 0]` for glyphs without ink.
    pub size: [f32; 2],
    /// Offset from the pen position (x) and baseline (y, negative = up) to the bitmap's
    /// top-left corner, in pixels.
    pub offset: [f32; 2],
}

impl CachedGlyph {
    const EMPTY: Self = Self {
        uv: [0.0; 4],
        size: [0.0; 2],
        offset: [0.0; 2],
    };
}

/// Pixels a renderer must copy into its atlas texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphUpload {
    /// Left edge in the atlas, in pixels.
    pub x: u32,
    /// Top edge in the atlas, in pixels.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major coverage, `width` bytes per row.
    pub coverage: Vec<u8>,
}

/// Cumulative cache counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheStats {
    /// Lookups answered from the cache.
    pub hits: u64,
    /// Lookups that rasterized a glyph.
    pub misses: u64,
    /// Glyphs removed to make room.
    pub evictions: u64,
    /// Glyphs that did not fit even after evicting everything not used this frame.
    pub dropped: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: FontId,
    glyph: GlyphId,
    /// Size in 1/64 px, so nearly equal float sizes share entries.
    size_64: u32,
    subpixel: u8,
}

#[derive(Debug)]
struct Entry {
    glyph: CachedGlyph,
    allocation: Option<AllocId>,
    last_used: u64,
}

/// Rasterized glyphs packed into one square coverage atlas, with LRU eviction.
///
/// Call [`begin_frame`](Self::begin_frame) once per frame, [`get`](Self::get) for every
/// glyph drawn, then copy [`drain_uploads`](Self::drain_uploads) into the GPU texture before
/// drawing. Glyphs used in the current frame are never evicted, so every UV handed out
/// during a frame stays valid until the next `begin_frame`.
pub struct GlyphCache {
    atlas: AtlasAllocator,
    atlas_size: u32,
    entries: FxHashMap<GlyphKey, Entry>,
    uploads: Vec<GlyphUpload>,
    frame: u64,
    generation: u64,
    stats: CacheStats,
}

impl std::fmt::Debug for GlyphCache {
    // Hand-written: etagere's allocator does not implement Debug.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphCache")
            .field("atlas_size", &self.atlas_size)
            .field("glyphs", &self.entries.len())
            .field("pending_uploads", &self.uploads.len())
            .field("frame", &self.frame)
            .field("generation", &self.generation)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl GlyphCache {
    /// An empty cache whose atlas is `atlas_size` x `atlas_size` pixels.
    pub fn new(atlas_size: u32) -> Self {
        let atlas_size = atlas_size.max(1);
        // Lossless enough: atlas sizes are bounded by GPU texture limits (≤ 16384).
        let side = atlas_size.min(i32::MAX as u32) as i32;
        Self {
            atlas: AtlasAllocator::new(size2(side, side)),
            atlas_size,
            entries: FxHashMap::default(),
            uploads: Vec::new(),
            frame: 0,
            generation: 0,
            stats: CacheStats::default(),
        }
    }

    /// Side length of the square atlas in pixels.
    pub fn atlas_size(&self) -> u32 {
        self.atlas_size
    }

    /// Starts a frame. Glyphs used in earlier frames become eligible for eviction.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// Increments whenever glyphs are evicted: UVs from earlier frames may now point at
    /// other glyphs.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Cumulative counters.
    pub fn stats(&self) -> CacheStats {
        self.stats
    }

    /// The atlas placement of `glyph` at `px` pixels per em and subpixel bin `subpixel`
    /// (see [`quantize_x`]), rasterizing it on first use.
    ///
    /// `None` if the atlas is full of glyphs already used this frame; the glyph is counted
    /// in [`CacheStats::dropped`] and simply not drawn.
    pub fn get(
        &mut self,
        font: &Font,
        glyph: GlyphId,
        px: f32,
        subpixel: u8,
    ) -> Option<CachedGlyph> {
        let key = GlyphKey {
            font: font.id(),
            glyph,
            // Lossless enough: font sizes are small positive numbers.
            size_64: (px * 64.0).round().max(0.0) as u32,
            subpixel: subpixel % SUBPIXEL_STEPS,
        };
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = self.frame;
            self.stats.hits += 1;
            return Some(entry.glyph);
        }
        self.stats.misses += 1;

        let x_offset = f32::from(key.subpixel) / f32::from(SUBPIXEL_STEPS);
        let bitmap = rasterize(font, glyph, px, x_offset);
        if bitmap.width == 0 {
            self.insert(key, CachedGlyph::EMPTY, None);
            return Some(CachedGlyph::EMPTY);
        }

        let Some(allocation) =
            self.allocate(bitmap.width + 2 * PADDING, bitmap.height + 2 * PADDING)
        else {
            self.stats.dropped += 1;
            return None;
        };
        // Lossless: etagere rectangles lie inside the atlas, so they are non-negative.
        let x = allocation.rectangle.min.x as u32 + PADDING;
        let y = allocation.rectangle.min.y as u32 + PADDING;
        let side = self.atlas_size as f32;
        let cached = CachedGlyph {
            uv: [
                x as f32 / side,
                y as f32 / side,
                (x + bitmap.width) as f32 / side,
                (y + bitmap.height) as f32 / side,
            ],
            size: [bitmap.width as f32, bitmap.height as f32],
            offset: [bitmap.left as f32, bitmap.top as f32],
        };
        self.uploads.push(GlyphUpload {
            x,
            y,
            width: bitmap.width,
            height: bitmap.height,
            coverage: bitmap.coverage,
        });
        self.insert(key, cached, Some(allocation.id));
        Some(cached)
    }

    /// Takes the pixels queued since the last call, in the order they must be applied.
    pub fn drain_uploads(&mut self) -> std::vec::Drain<'_, GlyphUpload> {
        self.uploads.drain(..)
    }

    fn insert(&mut self, key: GlyphKey, glyph: CachedGlyph, allocation: Option<AllocId>) {
        let entry = Entry {
            glyph,
            allocation,
            last_used: self.frame,
        };
        self.entries.insert(key, entry);
    }

    /// Allocates atlas space, evicting least recently used glyphs from earlier frames
    /// until it fits. Runs only on cache misses, so it may allocate.
    fn allocate(&mut self, width: u32, height: u32) -> Option<Allocation> {
        // Lossless enough: glyph bitmaps are far smaller than i32::MAX.
        let size = size2(width as i32, height as i32);
        if let Some(allocation) = self.atlas.allocate(size) {
            return Some(allocation);
        }

        let mut evictable: Vec<(u64, GlyphKey)> = self
            .entries
            .iter()
            .filter(|(_, e)| e.last_used < self.frame && e.allocation.is_some())
            .map(|(key, e)| (e.last_used, *key))
            .collect();
        evictable.sort_unstable_by_key(|(last_used, _)| *last_used);

        for (_, key) in evictable {
            if let Some(Entry {
                allocation: Some(id),
                ..
            }) = self.entries.remove(&key)
            {
                self.atlas.deallocate(id);
                self.stats.evictions += 1;
                self.generation += 1;
            }
            if let Some(allocation) = self.atlas.allocate(size) {
                return Some(allocation);
            }
        }
        None
    }
}
