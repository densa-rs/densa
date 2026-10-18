use crate::{Align, Column, GridLayout, Instance, InstanceBuffer, Palette};
use densa_fmt::FmtBuf;
use densa_text::{quantize_x, CacheStats, Font, GlyphCache, GlyphMap, GlyphUpload, SUBPIXEL_STEPS};

/// Bytes a cell's text may take. [`FmtBuf`] appends are all-or-nothing, so a write that
/// does not fit leaves the cell as it was (blank if nothing was written yet), not truncated.
pub const CELL_BYTES: usize = 64;

/// The stack buffer a [`GridSource`] writes one cell's text into.
pub type CellText = FmtBuf<CELL_BYTES>;

/// Side of the square glyph atlas in pixels: room for thousands of glyphs at grid sizes.
pub const ATLAS_SIZE: u32 = 1024;

/// How one cell is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellStyle {
    /// Text color, linear RGBA.
    pub color: [f32; 4],
    /// Cell fill drawn under the text, linear RGBA; `None` for none.
    pub background: Option<[f32; 4]>,
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

/// Turns a [`GridSource`] into the [`Instance`]s of one frame.
///
/// Owns the font, its character map and the glyph atlas cache. Painting a frame whose
/// characters have all been drawn (or [prewarmed](Self::prewarm)) before allocates nothing;
/// a character seen for the first time is mapped and rasterized (allocating, once) and its
/// pixels queued in [`drain_uploads`](Self::drain_uploads) for the GPU atlas, whose side must
/// be [`atlas_size`](Self::atlas_size).
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
            text: TextPen {
                glyphs,
                cache: GlyphCache::new(ATLAS_SIZE),
            },
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

    /// Side of the square atlas texture the uploads target, in pixels.
    pub fn atlas_size(&self) -> u32 {
        self.text.cache.atlas_size()
    }

    /// Glyph cache counters.
    pub fn cache_stats(&self) -> CacheStats {
        self.text.cache.stats()
    }

    /// Rasterizes every character of `chars` at every subpixel position now, so frames that
    /// only use them never rasterize (or allocate). Uploads queue as usual.
    pub fn prewarm(&mut self, chars: &str) {
        let pen = &mut self.text;
        pen.cache.begin_frame();
        for ch in chars.chars() {
            let Some(mapped) = pen.glyphs.get(ch) else {
                continue;
            };
            for bin in 0..SUBPIXEL_STEPS {
                let _ = pen
                    .cache
                    .get(pen.glyphs.font(), mapped.id, pen.glyphs.px(), bin);
            }
        }
    }

    /// Takes the glyph pixels queued since the last call; copy them into the GPU atlas
    /// before drawing the frame just painted.
    pub fn drain_uploads(&mut self) -> std::vec::Drain<'_, GlyphUpload> {
        self.text.cache.drain_uploads()
    }

    /// Replaces `out` with the frame for `source`.
    pub fn paint<S: GridSource + ?Sized>(&mut self, source: &S, out: &mut InstanceBuffer) {
        out.clear();
        self.text.cache.begin_frame();
        let width = self.layout.width() as f32;
        let row_height = self.layout.row_height() as f32;

        let header_y = self.layout.row_y(0) as f32;
        out.push(Instance::solid(
            [0.0, header_y, width, row_height],
            self.palette.header_background,
        ));
        let baseline = self.layout.baseline_y(0);
        for (index, column) in self.columns.iter().enumerate() {
            let cell = Cell {
                index,
                align: column.align,
                baseline,
                color: self.palette.header_text,
            };
            self.text.draw(&column.title, cell, &self.layout, out);
        }

        let visible = source.rows().min(self.layout.rows() as usize);
        for (row, grid_row) in (0..visible).zip(1u32..) {
            let y = self.layout.row_y(grid_row) as f32;
            if grid_row.is_multiple_of(2) {
                out.push(Instance::solid(
                    [0.0, y, width, row_height],
                    self.palette.stripe,
                ));
            }
            let baseline = self.layout.baseline_y(grid_row);
            for (index, column) in self.columns.iter().enumerate() {
                self.cell.clear();
                let style = source.cell(row, index, &mut self.cell);
                if let Some(background) = style.background {
                    out.push(Instance::solid(self.cell_rect(index, y), background));
                }
                let cell = Cell {
                    index,
                    align: column.align,
                    baseline,
                    color: style.color,
                };
                self.text.draw(self.cell.as_str(), cell, &self.layout, out);
            }
        }
    }

    /// Background rectangle of one cell: the column plus half the gap on each side, so
    /// neighbouring cell backgrounds meet without overlapping.
    fn cell_rect(&self, column: usize, y: f32) -> [f32; 4] {
        let margin = self.layout.gap() as f32 / 2.0;
        [
            self.layout.column_x(column) as f32 - margin,
            y,
            self.layout.column_width(column) as f32 + 2.0 * margin,
            self.layout.row_height() as f32,
        ]
    }
}

/// Where and how one cell's text goes.
#[derive(Debug, Clone, Copy)]
struct Cell {
    index: usize,
    align: Align,
    baseline: f32,
    color: [f32; 4],
}

/// Character mapping plus glyph cache: lays out and emits glyph quads.
#[derive(Debug)]
struct TextPen {
    glyphs: GlyphMap,
    cache: GlyphCache,
}

/// Text that cannot be measured more precisely than this is treated as fitting.
const FIT_EPSILON: f32 = 0.01;

impl TextPen {
    /// Draws `text` aligned in its column; text wider than the column (in pixels or in
    /// characters) becomes `#` fill, like a spreadsheet, rather than a misleading truncated
    /// number.
    fn draw(&mut self, text: &str, cell: Cell, layout: &GridLayout, out: &mut InstanceBuffer) {
        let left = layout.column_x(cell.index) as f32;
        let room = layout.column_width(cell.index) as f32;
        let max_chars = layout.column_chars(cell.index) as usize;

        let width = self.glyphs.measure(text);
        if width <= room + FIT_EPSILON && text.chars().count() <= max_chars {
            self.run(text.chars(), width, left, room, cell, out);
            return;
        }
        let hash = self
            .glyphs
            .get('#')
            .map_or(layout.char_width(), |g| g.advance);
        // Lossless: column widths are a few hundred pixels at most.
        let count = ((room / hash) as usize).min(max_chars);
        let fill = std::iter::repeat_n('#', count);
        self.run(fill, count as f32 * hash, left, room, cell, out);
    }

    fn run(
        &mut self,
        chars: impl Iterator<Item = char>,
        width: f32,
        left: f32,
        room: f32,
        cell: Cell,
        out: &mut InstanceBuffer,
    ) {
        let mut pen = match cell.align {
            Align::Left => left,
            Align::Right => left + room - width,
        };
        for ch in chars {
            let Some(mapped) = self.glyphs.get(ch) else {
                continue;
            };
            let (x, bin) = quantize_x(pen);
            pen += mapped.advance;
            let (font, px) = (self.glyphs.font(), self.glyphs.px());
            let Some(glyph) = self.cache.get(font, mapped.id, px, bin) else {
                continue; // Atlas full of this frame's glyphs: counted in the cache stats.
            };
            if glyph.size[0] == 0.0 {
                continue; // No ink, e.g. a space.
            }
            out.push(Instance::glyph(
                [
                    x as f32 + glyph.offset[0],
                    cell.baseline + glyph.offset[1],
                    glyph.size[0],
                    glyph.size[1],
                ],
                glyph.uv,
                cell.color,
            ));
        }
    }
}
