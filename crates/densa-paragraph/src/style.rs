use densa_core::Color;
use std::borrow::Cow;
use std::fmt;
use std::ops::Range;

/// Largest font size accepted, in pixels per em. Far beyond any UI text; it keeps every
/// line metric finite (sizes near `f32::MAX` turn layout geometry into NaN).
pub const MAX_FONT_PX: f32 = 4096.0;

/// Largest line height accepted, as a multiple of the font size.
pub const MAX_LINE_HEIGHT: f32 = 16.0;

/// Horizontal alignment of each line within the paragraph's width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TextAlign {
    /// Left for left-to-right paragraphs, right for right-to-left ones.
    #[default]
    Start,
    /// Right for left-to-right paragraphs, left for right-to-left ones.
    End,
    /// Flush left.
    Left,
    /// Centered.
    Center,
    /// Flush right.
    Right,
    /// Stretched to the full width, except a paragraph's last line.
    Justify,
}

/// How a whole paragraph is set; [`Span`]s override parts of it.
///
/// Sizes and widths are in physical pixels: multiply logical sizes by the display's scale
/// factor, and lay out again when it changes.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ParagraphStyle {
    /// CSS-style family list, for example `"Inter, sans-serif"`.
    pub family: Cow<'static, str>,
    /// Font size in pixels per em.
    pub size_px: f32,
    /// Line height as a multiple of the font size.
    pub line_height: f32,
    /// Text color.
    pub color: Color,
    /// Line alignment.
    pub align: TextAlign,
    /// Width lines wrap at, in pixels; `None` breaks only at explicit newlines.
    pub max_width: Option<f32>,
}

impl ParagraphStyle {
    /// White `monospace` text of `size_px` pixels, 1.3 line height, start-aligned, unwrapped.
    pub fn new(size_px: f32) -> Self {
        Self {
            family: Cow::Borrowed("monospace"),
            size_px,
            line_height: 1.3,
            color: Color::WHITE,
            align: TextAlign::Start,
            max_width: None,
        }
    }

    /// The style with the family list `family`.
    #[must_use]
    pub fn with_family(mut self, family: impl Into<Cow<'static, str>>) -> Self {
        self.family = family.into();
        self
    }

    /// The style with text color `color`.
    #[must_use]
    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// The style with line alignment `align`.
    #[must_use]
    pub fn with_align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    /// The style with a line height of `line_height` times the font size.
    #[must_use]
    pub fn with_line_height(mut self, line_height: f32) -> Self {
        self.line_height = line_height;
        self
    }

    /// The style wrapping lines at `max_width` pixels (`None`: no wrapping).
    #[must_use]
    pub fn with_max_width(mut self, max_width: Option<f32>) -> Self {
        self.max_width = max_width;
        self
    }
}

/// A styled part of a paragraph's text.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Span {
    /// Byte range of the text it styles; must lie on character boundaries.
    pub range: Range<usize>,
    /// Text color, if different from the paragraph's.
    pub color: Option<Color>,
    /// Font size in pixels per em, if different from the paragraph's.
    pub size_px: Option<f32>,
}

impl Span {
    /// A span over `range` that changes nothing yet.
    pub fn new(range: Range<usize>) -> Self {
        Self {
            range,
            color: None,
            size_px: None,
        }
    }

    /// The span coloring its text `color`.
    #[must_use]
    pub fn with_color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// The span setting its text at `size_px` pixels per em.
    #[must_use]
    pub fn with_size(mut self, size_px: f32) -> Self {
        self.size_px = Some(size_px);
        self
    }
}

/// Why a paragraph could not be laid out.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ParagraphError {
    /// A font size outside `(0, MAX_FONT_PX]`, or a line height outside
    /// `(0, MAX_LINE_HEIGHT]`.
    InvalidSize(f32),
    /// The maximum width is not a finite, non-negative number.
    InvalidWidth(f32),
    /// A span's range is reversed, outside the text, or splits a character.
    InvalidSpan(Range<usize>),
    /// A byte range (such as an input method's cursor in its preedit text) is reversed,
    /// outside its text, or splits a character.
    InvalidRange(Range<usize>),
    /// The shaper chose a font whose data cannot be read for drawing.
    UnreadableFont,
}

impl fmt::Display for ParagraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSize(size) => write!(
                f,
                "size {size} is out of range: font sizes go above 0 up to {MAX_FONT_PX} px, \
                 line heights above 0 up to {MAX_LINE_HEIGHT} times the font size"
            ),
            Self::InvalidWidth(width) => {
                write!(f, "width {width} is not a finite, non-negative number")
            }
            Self::InvalidSpan(range) => write!(
                f,
                "span {range:?} is reversed, outside the text, or splits a character"
            ),
            Self::InvalidRange(range) => write!(
                f,
                "range {range:?} is reversed, outside its text, or splits a character"
            ),
            Self::UnreadableFont => f.write_str("a font chosen for the text cannot be read"),
        }
    }
}

impl std::error::Error for ParagraphError {}

pub(crate) fn check_font_size(size: f32) -> Result<f32, ParagraphError> {
    in_range(size, MAX_FONT_PX)
}

pub(crate) fn check_line_height(line_height: f32) -> Result<f32, ParagraphError> {
    in_range(line_height, MAX_LINE_HEIGHT)
}

/// `value` if it lies in `(0, max]` (NaN does not).
fn in_range(value: f32, max: f32) -> Result<f32, ParagraphError> {
    if value > 0.0 && value <= max {
        Ok(value)
    } else {
        Err(ParagraphError::InvalidSize(value))
    }
}

pub(crate) fn check_width(width: Option<f32>) -> Result<Option<f32>, ParagraphError> {
    match width {
        Some(w) if !(w.is_finite() && w >= 0.0) => Err(ParagraphError::InvalidWidth(w)),
        _ => Ok(width),
    }
}

pub(crate) fn check_span(text: &str, span: &Span) -> Result<(), ParagraphError> {
    if !is_char_range(text, &span.range) {
        return Err(ParagraphError::InvalidSpan(span.range.clone()));
    }
    span.size_px.map(check_font_size).transpose()?;
    Ok(())
}

/// Whether `range` is in order, within `text` and on character boundaries.
pub(crate) fn is_char_range(text: &str, range: &Range<usize>) -> bool {
    range.start <= range.end
        && range.end <= text.len()
        && text.is_char_boundary(range.start)
        && text.is_char_boundary(range.end)
}
