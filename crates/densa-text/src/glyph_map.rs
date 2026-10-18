use rustc_hash::FxHashMap;

use crate::{Font, GlyphId, LineMetrics};

/// Non-ASCII characters remembered at most; past it the memory is forgotten and rebuilt.
pub const MAX_CACHED_CHARS: usize = 4096;

/// A character's glyph and advance at the map's size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MappedGlyph {
    /// The glyph the font draws for the character.
    pub id: GlyphId,
    /// Horizontal advance in pixels.
    pub advance: f32,
}

/// Cached character-to-glyph mapping for one font at one size.
///
/// [`Font::glyph_id`] re-reads the font's tables on every call; per-frame text needs the
/// answer for thousands of characters, so this caches it: ASCII is resolved up front into
/// a table, any other character on first use (that first use allocates, later ones don't).
/// Characters the font lacks map to its `?` glyph.
///
/// Memory is bounded: after [`MAX_CACHED_CHARS`] distinct non-ASCII characters (streaming
/// text, CJK), the non-ASCII memory is cleared, keeping its capacity, and refilled on use.
#[derive(Debug, Clone)]
pub struct GlyphMap {
    font: Font,
    px: f32,
    ascii: [Option<MappedGlyph>; 128],
    other: FxHashMap<char, Option<MappedGlyph>>,
    fallback: Option<MappedGlyph>,
}

impl GlyphMap {
    /// A map for `font` at `px` pixels per em.
    pub fn new(font: Font, px: f32) -> Self {
        let resolve = |ch: char| {
            font.glyph_id(ch).map(|id| MappedGlyph {
                id,
                advance: font.advance(id, px),
            })
        };
        let mut ascii = [None; 128];
        for (code, slot) in (0u8..).zip(ascii.iter_mut()) {
            *slot = resolve(char::from(code));
        }
        let fallback = resolve('?');
        Self {
            font,
            px,
            ascii,
            other: FxHashMap::default(),
            fallback,
        }
    }

    /// The font this map resolves against.
    pub fn font(&self) -> &Font {
        &self.font
    }

    /// The size in pixels per em.
    pub fn px(&self) -> f32 {
        self.px
    }

    /// Non-ASCII characters currently remembered (at most [`MAX_CACHED_CHARS`]).
    pub fn cached_chars(&self) -> usize {
        self.other.len()
    }

    /// The font's line metrics at this size.
    pub fn line_metrics(&self) -> LineMetrics {
        self.font.line_metrics(self.px)
    }

    /// The glyph for `ch`, or the `?` glyph if the font has none; `None` only if the font
    /// lacks `?` too.
    #[inline]
    pub fn get(&mut self, ch: char) -> Option<MappedGlyph> {
        let mapped = match self.ascii.get(ch as usize) {
            Some(slot) => *slot,
            None => {
                if self.other.len() >= MAX_CACHED_CHARS && !self.other.contains_key(&ch) {
                    self.other.clear();
                }
                let (font, px) = (&self.font, self.px);
                *self.other.entry(ch).or_insert_with(|| {
                    font.glyph_id(ch).map(|id| MappedGlyph {
                        id,
                        advance: font.advance(id, px),
                    })
                })
            }
        };
        mapped.or(self.fallback)
    }

    /// Total advance of `text` in pixels.
    pub fn measure(&mut self, text: &str) -> f32 {
        text.chars()
            .filter_map(|ch| self.get(ch))
            .map(|g| g.advance)
            .sum()
    }
}
