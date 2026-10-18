use etagere::{size2, AllocId, Allocation, AtlasAllocator};
use rustc_hash::FxHashMap;
use std::fmt;

use crate::{rasterize, Font, FontId, GlyphId};

/// Subpixel positions per pixel: glyphs are rasterized at 0, 1/4, 1/2 and 3/4 px offsets.
pub const SUBPIXEL_STEPS: u8 = 4;

/// Transparent border written around every glyph, so sampling at a glyph's edge (even with
/// bilinear filtering) never picks up a neighbour or a previous occupant of the space.
const PADDING: u32 = 1;

/// Pixels antialiasing and subpixel shifts may add to an outline's bounding box.
const RASTER_SLACK: f32 = 2.0;

/// Largest atlas side in texels: within every desktop GPU's 2D texture limit.
const MAX_ATLAS_SIDE: u32 = 8192;

/// Entries holding no atlas space (whitespace, too large, waiting) kept before the ones not
/// used this frame are forgotten, so ever-changing sizes cannot grow the cache without bound.
const MAX_UNPLACED: usize = 4096;

/// Splits a horizontal position into a whole pixel and a subpixel bin (`0..SUBPIXEL_STEPS`),
/// rounding to the nearest bin. Draw the glyph cached for `bin` at the returned pixel.
pub fn quantize_x(x: f32) -> (i32, u8) {
    let steps = i32::from(SUBPIXEL_STEPS);
    // Lossless enough: screen coordinates are far below 2^31 / SUBPIXEL_STEPS.
    let scaled = (x * steps as f32).round() as i32;
    // Lossless: rem_euclid keeps the value in 0..SUBPIXEL_STEPS.
    (scaled.div_euclid(steps), scaled.rem_euclid(steps) as u8)
}

/// Shape of the glyph atlas: square pages stacked vertically in one texture.
///
/// The atlas starts as one page and grows a page at a time, up to `max_pages`; growing never
/// moves a glyph. Once at its limit, the least recently used glyphs are evicted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AtlasConfig {
    /// Side of one page in texels; also the largest glyph the atlas holds.
    pub page_size: u32,
    /// Pages the atlas may grow to.
    pub max_pages: u32,
}

impl AtlasConfig {
    /// 1024 x 1024 pages, up to 4 (a 1024 x 4096 texture, 4 MiB).
    pub const DEFAULT: Self = Self::new(1024, 4);

    /// `max_pages` pages of `page_size` x `page_size` texels.
    pub const fn new(page_size: u32, max_pages: u32) -> Self {
        Self {
            page_size,
            max_pages,
        }
    }

    /// The config clamped to what the cache supports: a page of 1 to 4096 texels, at least
    /// one page, and a texture no taller than 8192 texels.
    pub fn normalized(self) -> Self {
        let page_size = self.page_size.clamp(1, 4096);
        Self {
            page_size,
            max_pages: self.max_pages.clamp(1, MAX_ATLAS_SIDE / page_size),
        }
    }
}

impl Default for AtlasConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A glyph ready to draw from the atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CachedGlyph {
    /// `[x0, y0, x1, y1]` of the glyph's coverage in atlas texels (not normalized: stays
    /// valid when the atlas grows).
    pub atlas_rect: [f32; 4],
    /// Bitmap size in pixels; `[0, 0]` for glyphs without ink.
    pub size: [f32; 2],
    /// Offset from the pen position (x) and baseline (y, negative = up) to the bitmap's
    /// top-left corner, in pixels.
    pub offset: [f32; 2],
}

impl CachedGlyph {
    const EMPTY: Self = Self {
        atlas_rect: [0.0; 4],
        size: [0.0; 2],
        offset: [0.0; 2],
    };
}

/// Why [`GlyphCache::get`] has no glyph to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GlyphUnavailable {
    /// No room this frame: everything evictable was evicted and it still did not fit. The
    /// rasterized glyph is kept and placed by a later lookup once there is room.
    NoRoom,
    /// The glyph at this size is larger than an atlas page; it never will fit.
    TooLarge,
    /// The size is not a finite, positive number of pixels.
    InvalidSize,
}

impl fmt::Display for GlyphUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoRoom => "no room in the glyph atlas this frame",
            Self::TooLarge => "glyph is larger than a glyph atlas page",
            Self::InvalidSize => "font size is not a finite, positive number of pixels",
        })
    }
}

impl std::error::Error for GlyphUnavailable {}

/// The device cannot hold the atlas: its texture limit is below the atlas already in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasLimitError {
    /// The device's largest texture side.
    pub max_side: u32,
    /// Texels the atlas needs at least: its page side, or its current height.
    pub needed: u32,
}

impl fmt::Display for AtlasLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "glyph atlas needs textures of {} texels, the device allows {}",
            self.needed, self.max_side
        )
    }
}

impl std::error::Error for AtlasLimitError {}

/// Pixels a renderer must copy into its atlas texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphUpload {
    /// Left edge in the atlas, in texels.
    pub x: u32,
    /// Top edge in the atlas, in texels.
    pub y: u32,
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Row-major coverage, `width` bytes per row; includes the glyph's transparent border.
    pub coverage: Vec<u8>,
}

/// A change the renderer must apply to its atlas texture, in queue order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AtlasUpdate {
    /// Make the texture at least `width` x `height`, keeping existing texels where they are.
    /// Always the first update, so the renderer learns the size from the cache itself.
    Resize {
        /// Width in texels.
        width: u32,
        /// Height in texels.
        height: u32,
    },
    /// Copy pixels into the texture.
    Write(GlyphUpload),
}

/// Cumulative cache counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct CacheStats {
    /// Lookups answered from the cache.
    pub hits: u64,
    /// Lookups of a glyph not cached yet.
    pub misses: u64,
    /// Glyphs evicted to make room.
    pub evictions: u64,
    /// Lookups that returned no glyph (see [`GlyphUnavailable`]).
    pub dropped: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: FontId,
    glyph: GlyphId,
    /// Size in 1/64 px, so nearly equal float sizes share entries (and rasterize alike).
    size_64: u32,
    subpixel: u8,
}

impl GlyphKey {
    /// The size the glyph is rasterized at: the key's, not the caller's unquantized one.
    fn px(self) -> f32 {
        self.size_64 as f32 / 64.0
    }
}

/// Rasterized coverage with its transparent border, waiting for (or written to) atlas space.
#[derive(Debug)]
struct Padded {
    width: u32,
    height: u32,
    coverage: Vec<u8>,
}

#[derive(Debug)]
enum Slot {
    /// In the atlas.
    Placed { page: usize, id: AllocId },
    /// Nothing to draw (whitespace).
    NoInk,
    /// Rasterized, no room yet. `failed_at` is the [`GlyphCache::room_stamp`] of the last
    /// attempt: retrying before it changes would fail again, so it is skipped.
    Waiting {
        padded: Padded,
        failed_at: (u64, u64),
    },
    /// Larger than a page: can never be placed.
    TooLarge,
}

#[derive(Debug)]
struct Entry {
    /// Metrics; `atlas_rect` is meaningful only when placed.
    glyph: CachedGlyph,
    slot: Slot,
    /// Frame of the last lookup.
    last_used: u64,
}

/// Rasterized glyphs packed into a paged coverage atlas with per-glyph LRU eviction.
///
/// Call [`begin_frame`](Self::begin_frame) once per frame, [`get`](Self::get) for every
/// glyph drawn, then apply [`drain_updates`](Self::drain_updates) to the GPU texture before
/// drawing. A glyph looked up in the current frame is never evicted, so every atlas rectangle
/// handed out during a frame stays valid until the next `begin_frame`. One cache serves one
/// frame loop: two render loops sharing it would evict each other's glyphs mid-frame.
///
/// When full, the atlas grows a page; at its page limit it evicts the glyphs used least
/// recently (never this frame's). A glyph is unavailable only if it does not fit even then.
///
/// Allocation: rasterizing a glyph for the first time allocates, and so does adding a page.
/// Hits, evictions and retries of waiting glyphs do not (the eviction order reuses its
/// buffer; it grows only past its largest size so far).
pub struct GlyphCache {
    config: AtlasConfig,
    pages: Vec<AtlasAllocator>,
    entries: FxHashMap<GlyphKey, Entry>,
    updates: Vec<AtlasUpdate>,
    /// Glyphs not used this frame, least recently used first; built on the frame's first
    /// eviction and consumed from `evict_next`.
    evict_order: Vec<(u64, GlyphKey)>,
    evict_next: usize,
    evict_frame: u64,
    /// Entries without atlas space, and the count that triggers forgetting cold ones.
    unplaced: usize,
    unplaced_limit: usize,
    frame: u64,
    /// Changes whenever atlas space is freed or added.
    room: u64,
    generation: u64,
    stats: CacheStats,
}

impl fmt::Debug for GlyphCache {
    // Hand-written: etagere's allocator does not implement Debug.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GlyphCache")
            .field("config", &self.config)
            .field("pages", &self.pages.len())
            .field("glyphs", &self.entries.len())
            .field("pending_updates", &self.updates.len())
            .field("frame", &self.frame)
            .field("generation", &self.generation)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl GlyphCache {
    /// An empty cache of one page; `config` is [normalized](AtlasConfig::normalized).
    pub fn new(config: AtlasConfig) -> Self {
        let config = config.normalized();
        let mut cache = Self {
            config,
            pages: Vec::with_capacity(config.max_pages as usize),
            entries: FxHashMap::default(),
            updates: Vec::new(),
            evict_order: Vec::new(),
            evict_next: 0,
            evict_frame: 0,
            unplaced: 0,
            unplaced_limit: MAX_UNPLACED,
            frame: 0,
            room: 0,
            generation: 0,
            stats: CacheStats::default(),
        };
        cache.add_page();
        cache
    }

    /// The atlas shape (normalized, and capped by [`limit_side`](Self::limit_side)).
    pub fn config(&self) -> AtlasConfig {
        self.config
    }

    /// Caps growth so the texture never exceeds `max_side` texels (the device's
    /// `max_texture_dimension_2d`).
    ///
    /// # Errors
    /// If a page, or the atlas already grown, is larger than `max_side`.
    pub fn limit_side(&mut self, max_side: u32) -> Result<(), AtlasLimitError> {
        let needed = self.config.page_size.max(self.atlas_extent().1);
        if needed > max_side {
            return Err(AtlasLimitError { max_side, needed });
        }
        let pages_fitting = max_side / self.config.page_size;
        self.config.max_pages = self.config.max_pages.min(pages_fitting);
        Ok(())
    }

    /// Current texture size in texels: `(page_size, page_size * pages)`.
    pub fn atlas_extent(&self) -> (u32, u32) {
        let page = self.config.page_size;
        // Lossless: pages are capped so the height stays within MAX_ATLAS_SIDE.
        (page, page * self.pages.len() as u32)
    }

    /// Glyphs cached, with or without atlas space.
    pub fn cached_glyphs(&self) -> usize {
        self.entries.len()
    }

    /// Starts a frame. Glyphs used in earlier frames become evictable.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// Increments whenever glyphs are evicted: atlas rectangles from earlier frames may now
    /// hold other glyphs.
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
    /// # Errors
    /// [`GlyphUnavailable`] when there is nothing to draw this frame; counted in
    /// [`CacheStats::dropped`].
    pub fn get(
        &mut self,
        font: &Font,
        glyph: GlyphId,
        px: f32,
        subpixel: u8,
    ) -> Result<CachedGlyph, GlyphUnavailable> {
        if !(px.is_finite() && px > 0.0) {
            self.stats.dropped += 1;
            return Err(GlyphUnavailable::InvalidSize);
        }
        let key = GlyphKey {
            font: font.id(),
            glyph,
            // Saturating cast: absurd sizes become a huge key, rejected as TooLarge below.
            size_64: (px * 64.0).round() as u32,
            subpixel: subpixel % SUBPIXEL_STEPS,
        };
        let room = self.room_stamp();
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = self.frame;
            match &entry.slot {
                Slot::Placed { .. } => {
                    self.stats.hits += 1;
                    return Ok(entry.glyph);
                }
                Slot::NoInk => {
                    self.stats.hits += 1;
                    return Ok(CachedGlyph::EMPTY);
                }
                Slot::TooLarge => {
                    self.stats.dropped += 1;
                    return Err(GlyphUnavailable::TooLarge);
                }
                Slot::Waiting { failed_at, .. } if *failed_at == room => {
                    self.stats.dropped += 1;
                    return Err(GlyphUnavailable::NoRoom);
                }
                Slot::Waiting { .. } => {}
            }
            let Some(Entry {
                glyph,
                slot: Slot::Waiting { padded, .. },
                ..
            }) = self.entries.remove(&key)
            else {
                unreachable!("the entry was just seen waiting");
            };
            self.unplaced -= 1;
            return self.place(key, glyph, padded);
        }
        self.stats.misses += 1;
        self.rasterize_new(key, font)
    }

    /// Takes the atlas changes queued since the last call, in the order they must be applied.
    /// Apply all of them: the queue is emptied even if the iterator is dropped early.
    pub fn drain_updates(&mut self) -> impl ExactSizeIterator<Item = AtlasUpdate> + '_ {
        self.updates.drain(..)
    }

    /// Identifies the free space: retrying a placement that failed under the same stamp
    /// cannot succeed (nothing was freed or added, and nothing became evictable).
    fn room_stamp(&self) -> (u64, u64) {
        (self.frame, self.room)
    }

    fn rasterize_new(
        &mut self,
        key: GlyphKey,
        font: &Font,
    ) -> Result<CachedGlyph, GlyphUnavailable> {
        let px = key.px();
        // Bound the work before rasterizing: a huge size must not allocate a huge bitmap.
        // Lenient by the raster slack (the exact check follows rasterization), so the bitmap
        // allocated is at most about a page.
        let limit = self.config.page_size.saturating_sub(2 * PADDING) as f32 + RASTER_SLACK;
        let may_fit = match font.glyph_extent(key.glyph, px) {
            Some([w, h]) => w <= limit && h <= limit,
            // Unknown extent: assume up to two ems.
            None => 2.0 * px <= limit,
        };
        if !may_fit {
            return self.too_large(key, CachedGlyph::EMPTY);
        }

        let x_offset = f32::from(key.subpixel) / f32::from(SUBPIXEL_STEPS);
        let bitmap = rasterize(font, key.glyph, px, x_offset);
        if bitmap.width == 0 {
            self.insert(key, CachedGlyph::EMPTY, Slot::NoInk);
            return Ok(CachedGlyph::EMPTY);
        }
        let glyph = CachedGlyph {
            atlas_rect: [0.0; 4],
            size: [bitmap.width as f32, bitmap.height as f32],
            offset: [bitmap.left as f32, bitmap.top as f32],
        };
        let max = self.config.page_size.saturating_sub(2 * PADDING);
        if bitmap.width > max || bitmap.height > max {
            return self.too_large(key, glyph); // The estimate was optimistic.
        }
        let padded = pad(bitmap.width, bitmap.height, &bitmap.coverage);
        self.place(key, glyph, padded)
    }

    fn too_large(
        &mut self,
        key: GlyphKey,
        glyph: CachedGlyph,
    ) -> Result<CachedGlyph, GlyphUnavailable> {
        self.stats.dropped += 1;
        self.insert(key, glyph, Slot::TooLarge);
        Err(GlyphUnavailable::TooLarge)
    }

    /// Finds atlas space for `padded` and queues its upload, or parks it as waiting.
    fn place(
        &mut self,
        key: GlyphKey,
        glyph: CachedGlyph,
        padded: Padded,
    ) -> Result<CachedGlyph, GlyphUnavailable> {
        let Some((page, allocation)) = self.allocate(padded.width, padded.height) else {
            self.stats.dropped += 1;
            let failed_at = self.room_stamp();
            self.insert(key, glyph, Slot::Waiting { padded, failed_at });
            return Err(GlyphUnavailable::NoRoom);
        };
        // Lossless: etagere rectangles lie inside the page, so they are non-negative, and
        // page offsets stay within MAX_ATLAS_SIDE.
        let x = allocation.rectangle.min.x as u32;
        let y = page as u32 * self.config.page_size + allocation.rectangle.min.y as u32;
        let (inner_x, inner_y) = ((x + PADDING) as f32, (y + PADDING) as f32);
        let placed = CachedGlyph {
            atlas_rect: [
                inner_x,
                inner_y,
                inner_x + glyph.size[0],
                inner_y + glyph.size[1],
            ],
            ..glyph
        };
        self.updates.push(AtlasUpdate::Write(GlyphUpload {
            x,
            y,
            width: padded.width,
            height: padded.height,
            coverage: padded.coverage,
        }));
        let slot = Slot::Placed {
            page,
            id: allocation.id,
        };
        self.insert(key, placed, slot);
        Ok(placed)
    }

    /// Space for a `width` x `height` rectangle (no larger than a page): first fit in an
    /// existing page, else a new page, else evicting least recently used glyphs not used
    /// this frame until it fits.
    fn allocate(&mut self, width: u32, height: u32) -> Option<(usize, Allocation)> {
        // Lossless: callers guarantee the size fits in a page (≤ 4096).
        let size = size2(width as i32, height as i32);
        let fit = self
            .pages
            .iter_mut()
            .enumerate()
            .find_map(|(index, page)| page.allocate(size).map(|a| (index, a)));
        if fit.is_some() {
            return fit;
        }
        if self.pages.len() < self.config.max_pages as usize {
            self.add_page();
            let index = self.pages.len() - 1;
            // An empty page always fits a rectangle no larger than itself.
            return self.pages[index].allocate(size).map(|a| (index, a));
        }

        if self.evict_frame != self.frame {
            self.build_evict_order();
        }
        while let Some(&(_, key)) = self.evict_order.get(self.evict_next) {
            self.evict_next += 1;
            let Some(page) = self.evict(key) else {
                continue; // Gone already, or used since the order was built.
            };
            // Freed space merges with its neighbours on that page only.
            if let Some(allocation) = self.pages[page].allocate(size) {
                return Some((page, allocation));
            }
        }
        None
    }

    /// Orders the glyphs placed before this frame, least recently used first. Reuses the
    /// buffer, so it allocates only past its largest size so far.
    fn build_evict_order(&mut self) {
        let frame = self.frame;
        self.evict_order.clear();
        self.evict_order.extend(
            self.entries
                .iter()
                .filter(|(_, e)| e.last_used < frame && matches!(e.slot, Slot::Placed { .. }))
                .map(|(key, e)| (e.last_used, *key)),
        );
        self.evict_order
            .sort_unstable_by_key(|(last_used, _)| *last_used);
        self.evict_next = 0;
        self.evict_frame = frame;
    }

    /// Evicts `key` if it is still placed and unused this frame; returns its page.
    fn evict(&mut self, key: GlyphKey) -> Option<usize> {
        let entry = self.entries.get(&key)?;
        let Slot::Placed { page, id } = entry.slot else {
            return None;
        };
        if entry.last_used >= self.frame {
            return None;
        }
        self.entries.remove(&key);
        self.pages[page].deallocate(id);
        self.stats.evictions += 1;
        self.generation += 1;
        self.room += 1;
        Some(page)
    }

    fn add_page(&mut self) {
        // Lossless enough: page sizes are clamped to 4096.
        let side = self.config.page_size as i32;
        self.pages.push(AtlasAllocator::new(size2(side, side)));
        self.room += 1;
        let (width, height) = self.atlas_extent();
        self.updates.push(AtlasUpdate::Resize { width, height });
    }

    fn insert(&mut self, key: GlyphKey, glyph: CachedGlyph, slot: Slot) {
        if !matches!(slot, Slot::Placed { .. }) {
            self.unplaced += 1;
            if self.unplaced > self.unplaced_limit {
                self.forget_cold_unplaced();
            }
        }
        let last_used = self.frame;
        self.entries.insert(
            key,
            Entry {
                glyph,
                slot,
                last_used,
            },
        );
    }

    /// Drops entries without atlas space that were not used this frame. If this frame alone
    /// uses more than the limit, the limit rises so this does not repeat on every insert.
    fn forget_cold_unplaced(&mut self) {
        let frame = self.frame;
        self.entries
            .retain(|_, e| matches!(e.slot, Slot::Placed { .. }) || e.last_used >= frame);
        self.unplaced = self
            .entries
            .values()
            .filter(|e| !matches!(e.slot, Slot::Placed { .. }))
            .count()
            + 1; // The entry being inserted.
        self.unplaced_limit = MAX_UNPLACED.max(2 * self.unplaced);
    }
}

/// `coverage` (`width` x `height`) surrounded by a transparent border of `PADDING`.
fn pad(width: u32, height: u32, coverage: &[u8]) -> Padded {
    let (padded_width, padded_height) = (width + 2 * PADDING, height + 2 * PADDING);
    let (w, p, pw) = (width as usize, PADDING as usize, padded_width as usize);
    let mut padded = vec![0u8; pw * padded_height as usize];
    for (row, src) in coverage.chunks_exact(w).enumerate() {
        let start = (row + p) * pw + p;
        padded[start..start + w].copy_from_slice(src);
    }
    Padded {
        width: padded_width,
        height: padded_height,
        coverage: padded,
    }
}
