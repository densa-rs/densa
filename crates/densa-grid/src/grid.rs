use densa_core::{Color, Point, Rect};
use densa_draw::Canvas;
use densa_fmt::FmtBuf;
use densa_text::{Font, GlyphCache, GlyphMap, SUBPIXEL_STEPS};

use crate::{Align, Column, GridLayout, Palette};

/// Bytes a cell's text may take. [`FmtBuf`] appends are all-or-nothing, so a write that
/// does not fit leaves the cell as it was (blank if nothing was written yet), not truncated.
pub const CELL_BYTES: usize = 64;

/// The stack buffer a [`GridSource`] writes one cell's text into.
pub type CellText = FmtBuf<CELL_BYTES>;

/// How one cell is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellStyle {
    /// Text color.
    pub color: Color,
    /// Cell fill drawn under the text; `None` for none.
    pub background: Option<Color>,
}

/// The data a [`GridPainter`] draws: one call per visible cell, every frame.
///
/// Implementations format straight into `text` (see `densa_fmt`) so a frame allocates
/// nothing.
pub trait GridSource {
    /// Rows available; the painter draws at most as many as its layout holds.
    fn rows(&self) -> usize;

    /// Writes the text of the cell at `row`, `column` into `text` (empty on entry) and
    /// returns its style.
    ///
    /// Text over [`CELL_BYTES`] does not fit: write a placeholder instead (for example a
    /// run of `#`, which the painter shows as overflow) rather than ignoring the error.
    fn cell(&self, row: usize, column: usize, text: &mut CellText) -> CellStyle;
}

/// Draws a [`GridSource`] onto a [`Canvas`].
///
/// Owns the column layout and the font's character map; glyphs come from the canvas's
/// shared cache. Painting a frame whose characters have all been drawn (or
/// [prewarmed](Self::prewarm)) before allocates nothing; a character seen for the first time
/// is mapped and rasterized, allocating once.
#[derive(Debug)]
pub struct GridPainter {
    columns: Box<[Column]>,
    layout: GridLayout,
    palette: Palette,
    text: TextPen,
    cell: CellText,
}

impl GridPainter {
    /// A painter for `rows` data rows of `columns`, in `font` at `px` pixels per em.
    pub fn new(columns: Vec<Column>, rows: u32, font: Font, px: f32, palette: Palette) -> Self {
        let mut glyphs = GlyphMap::new(font, px);
        let layout = GridLayout::new(&columns, rows, &mut glyphs);
        Self {
            columns: columns.into_boxed_slice(),
            layout,
            palette,
            text: TextPen { glyphs },
            cell: CellText::new(),
        }
    }

    /// The geometry this painter draws with.
    pub fn layout(&self) -> &GridLayout {
        &self.layout
    }

    /// The colors this painter draws with.
    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// The columns, left to right.
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Rasterizes every character of `chars` into `cache` at every subpixel position now,
    /// so frames that only use them never rasterize (or allocate). Updates queue as usual.
    ///
    /// Returns how many glyphs could not be cached (the atlas is too small for the set, or a
    /// glyph is larger than a page); they would be retried, and allocate, while painting.
    pub fn prewarm(&mut self, cache: &mut GlyphCache, chars: &str) -> usize {
        let glyphs = &mut self.text.glyphs;
        let mut missing = 0;
        for ch in chars.chars() {
            let Some(mapped) = glyphs.get(ch) else {
                continue;
            };
            for bin in 0..SUBPIXEL_STEPS {
                let cached = cache.get(glyphs.font(), mapped.id, glyphs.px(), bin);
                missing += usize::from(cached.is_err());
            }
        }
        missing
    }

    /// Draws the grid for `source` onto `canvas`.
    pub fn paint<S: GridSource + ?Sized>(&mut self, source: &S, canvas: &mut Canvas<'_>) {
        let width = self.layout.width() as f32;
        let row_height = self.layout.row_height() as f32;

        let header_y = self.layout.row_y(0) as f32;
        canvas.fill_rect(
            Rect::new(0.0, header_y, width, row_height),
            self.palette.header_background,
        );
        let baseline = self.layout.baseline_y(0);
        for (index, column) in self.columns.iter().enumerate() {
            let cell = Cell {
                index,
                align: column.align,
                baseline,
                color: self.palette.header_text,
            };
            self.text.draw(&column.title, cell, &self.layout, canvas);
        }

        let visible = source.rows().min(self.layout.rows() as usize);
        for (row, grid_row) in (0..visible).zip(1u32..) {
            let y = self.layout.row_y(grid_row) as f32;
            if grid_row.is_multiple_of(2) {
                canvas.fill_rect(Rect::new(0.0, y, width, row_height), self.palette.stripe);
            }
            let baseline = self.layout.baseline_y(grid_row);
            for (index, column) in self.columns.iter().enumerate() {
                self.cell.clear();
                let style = source.cell(row, index, &mut self.cell);
                if let Some(background) = style.background {
                    canvas.fill_rect(self.cell_rect(index, y), background);
                }
                let cell = Cell {
                    index,
                    align: column.align,
                    baseline,
                    color: style.color,
                };
                self.text
                    .draw(self.cell.as_str(), cell, &self.layout, canvas);
            }
        }
    }

    /// Background rectangle of one cell: the column plus half the gap on each side, so
    /// neighbouring cell backgrounds meet without overlapping.
    fn cell_rect(&self, column: usize, y: f32) -> Rect {
        let margin = self.layout.gap() as f32 / 2.0;
        Rect::new(
            self.layout.column_x(column) as f32 - margin,
            y,
            self.layout.column_width(column) as f32 + 2.0 * margin,
            self.layout.row_height() as f32,
        )
    }
}

/// Where and how one cell's text goes.
#[derive(Debug, Clone, Copy)]
struct Cell {
    index: usize,
    align: Align,
    baseline: f32,
    color: Color,
}

/// The grid font's character map: lays out cell text and draws it glyph by glyph.
#[derive(Debug)]
struct TextPen {
    glyphs: GlyphMap,
}

/// Text that cannot be measured more precisely than this is treated as fitting.
const FIT_EPSILON: f32 = 0.01;

impl TextPen {
    /// Draws `text` aligned in its column; text wider than the column (in pixels or in
    /// characters) becomes `#` fill, like a spreadsheet, rather than a misleading truncated
    /// number.
    fn draw(&mut self, text: &str, cell: Cell, layout: &GridLayout, canvas: &mut Canvas<'_>) {
        let left = layout.column_x(cell.index) as f32;
        let room = layout.column_width(cell.index) as f32;
        let max_chars = layout.column_chars(cell.index) as usize;

        let width = self.glyphs.measure(text);
        if width <= room + FIT_EPSILON && text.chars().count() <= max_chars {
            self.run(text.chars(), width, left, room, cell, canvas);
            return;
        }
        let hash = self
            .glyphs
            .get('#')
            .map_or(layout.char_width(), |g| g.advance);
        // Lossless: column widths are a few hundred pixels at most.
        let count = ((room / hash) as usize).min(max_chars);
        let fill = std::iter::repeat_n('#', count);
        self.run(fill, count as f32 * hash, left, room, cell, canvas);
    }

    fn run(
        &mut self,
        chars: impl Iterator<Item = char>,
        width: f32,
        left: f32,
        room: f32,
        cell: Cell,
        canvas: &mut Canvas<'_>,
    ) {
        let mut pen = match cell.align {
            Align::Left => left,
            Align::Right => left + room - width,
        };
        for ch in chars {
            let Some(mapped) = self.glyphs.get(ch) else {
                continue;
            };
            let (font, px) = (self.glyphs.font(), self.glyphs.px());
            let at = Point::new(pen, cell.baseline);
            canvas.draw_glyph(font, mapped.id, px, at, cell.color);
            pen += mapped.advance;
        }
    }
}
