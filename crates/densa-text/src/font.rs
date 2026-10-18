use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, MetadataProvider};

/// JetBrains Mono Regular, bundled under the SIL Open Font License 1.1 (`fonts/OFL.txt`).
/// Monospace (every glyph advances 0.6 em), with precomposed Vietnamese.
pub const BUNDLED_MONO: &[u8] = include_bytes!("../fonts/JetBrainsMono-Regular.ttf");

/// Identifies one loaded [`Font`] for the lifetime of the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontId(u32);

/// A glyph index within a font.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GlyphId(pub u32);

/// Vertical metrics of a font at a given size, in pixels (y up: `descent` is negative).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineMetrics {
    /// Distance from the baseline to the top of tall glyphs.
    pub ascent: f32,
    /// Distance from the baseline to the bottom of descenders; negative.
    pub descent: f32,
    /// Extra space the font recommends between lines.
    pub line_gap: f32,
}

impl LineMetrics {
    /// Baseline-to-baseline distance.
    pub fn line_height(&self) -> f32 {
        self.ascent - self.descent + self.line_gap
    }
}

/// Why font data could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontError {
    /// The bytes are not a readable OpenType/TrueType font.
    Parse,
}

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse => f.write_str("the data is not a readable OpenType/TrueType font"),
        }
    }
}

impl std::error::Error for FontError {}

static NEXT_FONT_ID: AtomicU32 = AtomicU32::new(1);

/// An OpenType/TrueType font. Cheap to clone (the bytes are shared).
#[derive(Debug, Clone)]
pub struct Font {
    data: Arc<[u8]>,
    id: FontId,
}

impl Font {
    /// Loads a font from its file bytes.
    ///
    /// # Errors
    ///
    /// [`FontError::Parse`] if the bytes are not a readable font.
    pub fn new(data: impl Into<Arc<[u8]>>) -> Result<Self, FontError> {
        let data = data.into();
        FontRef::new(&data).map_err(|_| FontError::Parse)?;
        // ORDERING: Relaxed. Only uniqueness matters; no data is published through it.
        let id = FontId(NEXT_FONT_ID.fetch_add(1, Ordering::Relaxed));
        Ok(Self { data, id })
    }

    /// The bundled JetBrains Mono ([`BUNDLED_MONO`]).
    ///
    /// # Errors
    ///
    /// Only if the bundled file were corrupt; tests guard against that.
    pub fn bundled_mono() -> Result<Self, FontError> {
        Self::new(BUNDLED_MONO)
    }

    /// This font's identity, used to key cached glyphs.
    pub fn id(&self) -> FontId {
        self.id
    }

    /// Parsed view of the font. Parsing reads table headers only: no allocation.
    /// Cannot fail after `new` succeeded on the same bytes; callers still degrade
    /// gracefully instead of panicking.
    pub(crate) fn font_ref(&self) -> Option<FontRef<'_>> {
        FontRef::new(&self.data).ok()
    }

    /// The glyph the font draws for `ch`, or `None` if it has no glyph for it.
    pub fn glyph_id(&self, ch: char) -> Option<GlyphId> {
        let glyph = self.font_ref()?.charmap().map(ch)?;
        // Glyph 0 is `.notdef`, the "missing glyph" box: treat it as unmapped.
        (glyph.to_u32() != 0).then_some(GlyphId(glyph.to_u32()))
    }

    /// Horizontal advance of `glyph` at `px` pixels per em; 0 if unknown.
    pub fn advance(&self, glyph: GlyphId, px: f32) -> f32 {
        self.font_ref()
            .and_then(|font| {
                font.glyph_metrics(Size::new(px), LocationRef::default())
                    .advance_width(skrifa::GlyphId::new(glyph.0))
            })
            .unwrap_or(0.0)
    }

    /// Ascent, descent and line gap at `px` pixels per em.
    pub fn line_metrics(&self, px: f32) -> LineMetrics {
        let Some(font) = self.font_ref() else {
            return LineMetrics {
                ascent: 0.0,
                descent: 0.0,
                line_gap: 0.0,
            };
        };
        let metrics = font.metrics(Size::new(px), LocationRef::default());
        LineMetrics {
            ascent: metrics.ascent,
            descent: metrics.descent,
            line_gap: metrics.leading,
        }
    }
}
