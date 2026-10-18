use densa_text::{GlyphMap, LineMetrics};
use std::borrow::Cow;

/// Horizontal alignment of text within a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// Flush with the column's left edge (labels).
    Left,
    /// Flush with the column's right edge (numbers).
    Right,
}

/// A grid column: header title, width in characters, alignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// Header text.
    pub title: Cow<'static, str>,
    /// Width in characters of the font's digit advance.
    pub chars: u32,
    /// Text alignment.
    pub align: Align,
}

impl Column {
    /// A column titled `title`, `chars` digits wide.
    pub fn new(title: impl Into<Cow<'static, str>>, chars: u32, align: Align) -> Self {
        Self {
            title: title.into(),
            chars,
            align,
        }
    }
}

/// Blank characters between columns.
const GAP_CHARS: f32 = 2.0;
/// Smallest margin around the grid in pixels.
const MIN_PADDING: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ColumnGeometry {
    x: u32,
    width: u32,
    chars: u32,
}

/// Pixel geometry of a grid, derived from its columns and the font's metrics.
///
/// Column edges and baselines are whole pixels, so text is positioned identically every
/// frame and glyphs stay crisp. All sizes are physical pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct GridLayout {
    rows: u32,
    row_height: u32,
    padding: u32,
    gap: u32,
    char_width: f32,
    /// Row top to baseline.
    baseline: u32,
    columns: Box<[ColumnGeometry]>,
    width: u32,
}

impl GridLayout {
    /// Layout for `rows` data rows (plus a header) of `columns`, drawn with `glyphs`.
    pub fn new(columns: &[Column], rows: u32, glyphs: &mut GlyphMap) -> Self {
        let char_width = glyphs.get('0').map_or(glyphs.px() * 0.5, |g| g.advance);
        let metrics: LineMetrics = glyphs.line_metrics();
        let (ascent, line_height) = (metrics.ascent, metrics.line_height());
        // Lossless: font sizes give row heights and gaps of a few dozen pixels.
        let row_height = line_height.ceil().max(1.0) as u32;
        let baseline = ((row_height as f32 - line_height) / 2.0 + ascent).round() as u32;
        let gap = (GAP_CHARS * char_width).ceil() as u32;
        // Cell backgrounds extend half a gap past their column; keep that on screen.
        let padding = MIN_PADDING.max(gap.div_ceil(2));

        let mut x = padding;
        let columns: Box<[ColumnGeometry]> = columns
            .iter()
            .map(|c| {
                let width = (c.chars as f32 * char_width).ceil() as u32;
                let geometry = ColumnGeometry {
                    x,
                    width,
                    chars: c.chars,
                };
                // Saturating: absurd column widths clamp instead of wrapping.
                x = x.saturating_add(width).saturating_add(gap);
                geometry
            })
            .collect();
        let content = columns
            .last()
            .map_or(0, |c| c.x.saturating_add(c.width) - padding);
        Self {
            rows,
            row_height,
            padding,
            gap,
            char_width,
            baseline,
            columns,
            width: content.saturating_add(2 * padding),
        }
    }

    /// Data rows (the header is extra).
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// Number of columns.
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Row pitch in pixels.
    pub fn row_height(&self) -> u32 {
        self.row_height
    }

    /// Margin around the grid in pixels.
    pub fn padding(&self) -> u32 {
        self.padding
    }

    /// Space between columns in pixels.
    pub fn gap(&self) -> u32 {
        self.gap
    }

    /// Advance of the font's `0`: the unit column widths are given in.
    pub fn char_width(&self) -> f32 {
        self.char_width
    }

    /// Total grid width in pixels, padding included.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Total grid height in pixels: header plus data rows, padding included.
    pub fn height(&self) -> u32 {
        self.padding * 2 + (self.rows + 1) * self.row_height
    }

    /// Left edge of column `index`.
    ///
    /// # Panics
    /// If `index` is not a column.
    pub fn column_x(&self, index: usize) -> u32 {
        self.columns[index].x
    }

    /// Width of column `index` in pixels.
    ///
    /// # Panics
    /// If `index` is not a column.
    pub fn column_width(&self, index: usize) -> u32 {
        self.columns[index].width
    }

    /// Width of column `index` in characters.
    ///
    /// # Panics
    /// If `index` is not a column.
    pub fn column_chars(&self, index: usize) -> u32 {
        self.columns[index].chars
    }

    /// Top edge of grid row `row` (0 is the header).
    pub fn row_y(&self, row: u32) -> u32 {
        self.padding + row * self.row_height
    }

    /// Baseline of grid row `row` (0 is the header), a whole pixel.
    pub fn baseline_y(&self, row: u32) -> f32 {
        (self.row_y(row) + self.baseline) as f32
    }

    /// Instances needed for a full frame in the worst case: the header bar, and per row a
    /// stripe, a background per cell and a glyph per character of every column.
    pub fn max_instances(&self) -> usize {
        let per_row: usize = self
            .columns
            .iter()
            .map(|c| c.chars as usize + 1)
            .sum::<usize>()
            + 1;
        (self.rows as usize + 1)
            .saturating_mul(per_row)
            .saturating_add(1)
    }
}
