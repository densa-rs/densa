use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

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

/// Font file bytes shared without copying: a `Vec`, a `&'static [u8]`, or another library's
/// shared buffer (such as a parley/fontique `Blob`).
pub type FontBytes = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// One face of an OpenType/TrueType font file or collection. Cheap to clone (the bytes are
/// shared); clones keep the same [`FontId`], so they share cached glyphs.
#[derive(Clone)]
pub struct Font {
    data: FontBytes,
    index: u32,
    id: FontId,
}

impl fmt::Debug for Font {
    // Hand-written: the shared bytes are opaque (and large).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Font")
            .field("id", &self.id)
            .field("index", &self.index)
            .field("bytes", &self.bytes().len())
            .finish()
    }
}

impl Font {
    /// Loads a font from its file bytes (face 0 of a collection).
    ///
    /// # Errors
    ///
    /// [`FontError::Parse`] if the bytes are not a readable font.
    pub fn new(data: impl Into<Arc<[u8]>>) -> Result<Self, FontError> {
        let data: Arc<[u8]> = data.into();
        Self::from_shared(Arc::new(data), 0)
    }

    /// Face `index` of the font file or collection in `data`, sharing the bytes.
    ///
    /// Each call creates a new [`FontId`]: load a face once and clone the `Font`.
    ///
    /// # Errors
    ///
    /// [`FontError::Parse`] if the bytes are not a readable font or have no face `index`.
    pub fn from_shared(data: FontBytes, index: u32) -> Result<Self, FontError> {
        FontRef::from_index((*data).as_ref(), index).map_err(|_| FontError::Parse)?;
        // ORDERING: Relaxed. Only uniqueness matters; no data is published through it.
        let id = FontId(NEXT_FONT_ID.fetch_add(1, Ordering::Relaxed));
        Ok(Self { data, index, id })
    }

    /// The bundled JetBrains Mono ([`BUNDLED_MONO`]). Every call returns the same font
    /// (same [`FontId`]), so all users share its cached glyphs.
    ///
    /// # Errors
    ///
    /// Only if the bundled file were corrupt; tests guard against that.
    pub fn bundled_mono() -> Result<Self, FontError> {
        static BUNDLED: OnceLock<Result<Font, FontError>> = OnceLock::new();
        BUNDLED
            .get_or_init(|| Self::from_shared(Arc::new(BUNDLED_MONO), 0))
            .clone()
    }

    /// This font's identity, used to key cached glyphs.
    pub fn id(&self) -> FontId {
        self.id
    }

    /// The face's index within its file (0 unless the file is a collection).
    pub fn index(&self) -> u32 {
        self.index
    }

    /// The whole font file.
    pub fn bytes(&self) -> &[u8] {
        (*self.data).as_ref()
    }

    /// The shared file bytes, for handing to another library without copying.
    pub fn shared_bytes(&self) -> FontBytes {
        Arc::clone(&self.data)
    }

    /// Parsed view of the font. Parsing reads table headers only: no allocation.
    /// Cannot fail after construction succeeded on the same bytes; callers still degrade
    /// gracefully instead of panicking.
    pub(crate) fn font_ref(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(self.bytes(), self.index).ok()
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

    /// Width and height in pixels of `glyph`'s outline bounding box at `px` pixels per em,
    /// without rasterizing it; `None` if the font cannot say (then the caller must assume the
    /// worst). `Some([0.0, 0.0])` for glyphs without an outline.
    pub fn glyph_extent(&self, glyph: GlyphId, px: f32) -> Option<[f32; 2]> {
        let font = self.font_ref()?;
        let metrics = font.glyph_metrics(Size::new(px), LocationRef::default());
        let bounds = metrics.bounds(skrifa::GlyphId::new(glyph.0))?;
        Some([
            (bounds.x_max - bounds.x_min).max(0.0),
            (bounds.y_max - bounds.y_min).max(0.0),
        ])
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
