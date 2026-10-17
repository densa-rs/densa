use crate::{GlyphAtlas, Instance, InstanceBuffer, Palette, GLYPH_PX};
use densa_data::{Direction, RowSnapshot, ViewportSnapshot};
use densa_fmt::{write_fixed, write_grouped, FmtBuf};
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// A grid column: header title, width in characters, alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Column {
    pub title: &'static str,
    pub chars: u32,
    pub align: Align,
}

pub const COLUMNS: [Column; 5] = [
    Column {
        title: "SYMBOL",
        chars: 8,
        align: Align::Left,
    },
    Column {
        title: "BID",
        chars: 10,
        align: Align::Right,
    },
    Column {
        title: "ASK",
        chars: 10,
        align: Align::Right,
    },
    Column {
        title: "LAST",
        chars: 10,
        align: Align::Right,
    },
    Column {
        title: "VOLUME",
        chars: 15,
        align: Align::Right,
    },
];

const LAST_COLUMN: usize = 3;
const PRICE_DECIMALS: usize = 2;

/// How long the LAST cell flashes after the last-traded price changes.
///
/// Short on purpose: a busy symbol changes price many times per second, and a flash that
/// outlives the gap between changes never fades.
pub const FLASH_NS: u64 = 300_000_000;

/// Flash opacity `age_ns` after a price change: full at 0, linearly fading to 0.
pub fn flash_alpha(age_ns: u64) -> f32 {
    if age_ns >= FLASH_NS {
        return 0.0;
    }
    let remaining = 1.0 - age_ns as f32 / FLASH_NS as f32;
    Palette::FLASH_MAX_ALPHA * remaining
}

/// Pixel geometry of the grid. All sizes are physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridLayout {
    /// Data rows (the header is extra).
    pub rows: u32,
    /// Screen pixels per font pixel; integer so the bitmap font stays crisp.
    pub glyph_scale: u32,
    pub row_height: u32,
    pub padding: u32,
    /// Blank characters between columns.
    pub gap_chars: u32,
}

impl GridLayout {
    pub const fn new(rows: u32) -> Self {
        Self {
            rows,
            glyph_scale: 2,
            row_height: 18,
            padding: 8,
            gap_chars: 2,
        }
    }

    pub const fn char_width(&self) -> u32 {
        GLYPH_PX * self.glyph_scale
    }

    pub const fn glyph_height(&self) -> u32 {
        GLYPH_PX * self.glyph_scale
    }

    fn total_chars(&self) -> u32 {
        COLUMNS.iter().map(|c| c.chars).sum::<u32>() + self.gap_chars * (COLUMNS.len() as u32 - 1)
    }

    pub fn width(&self) -> u32 {
        self.padding * 2 + self.total_chars() * self.char_width()
    }

    pub fn height(&self) -> u32 {
        self.padding * 2 + (self.rows + 1) * self.row_height
    }

    /// Left edge of column `index`.
    pub fn column_x(&self, index: usize) -> u32 {
        let chars_before: u32 = COLUMNS[..index].iter().map(|c| c.chars).sum();
        self.padding + (chars_before + self.gap_chars * index as u32) * self.char_width()
    }

    /// Top edge of grid row `row` (0 is the header).
    pub fn row_y(&self, row: u32) -> u32 {
        self.padding + row * self.row_height
    }

    /// Instances needed for a full viewport in the worst case: every cell filled, plus a
    /// stripe and a flash per row and the header bar.
    pub fn max_instances(&self) -> usize {
        let per_row = COLUMNS.iter().map(|c| c.chars).sum::<u32>() + 2;
        ((self.rows + 1) * per_row + 1) as usize
    }
}

/// Builds the instances of one frame from a viewport snapshot. Allocation-free.
#[derive(Debug, Clone)]
pub struct GridPainter {
    layout: GridLayout,
    palette: Palette,
    uvs: [[f32; 4]; 128],
    cell: FmtBuf<32>,
}

impl GridPainter {
    pub fn new(layout: GridLayout, atlas: &GlyphAtlas) -> Self {
        let mut uvs = [[0.0; 4]; 128];
        for (code, uv) in (0u8..).zip(uvs.iter_mut()) {
            *uv = atlas.uv(code);
        }
        Self {
            layout,
            palette: Palette::new(),
            uvs,
            cell: FmtBuf::new(),
        }
    }

    pub fn layout(&self) -> &GridLayout {
        &self.layout
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// Replaces `out` with the frame for `snapshot` as of `now_ns` (same clock as the
    /// snapshot's timestamps; drives the flash fade).
    pub fn paint<const N: usize>(
        &mut self,
        snapshot: &ViewportSnapshot<N>,
        now_ns: u64,
        out: &mut InstanceBuffer,
    ) {
        out.clear();
        let header_y = self.layout.row_y(0) as f32;
        out.push(Instance::solid(
            [
                0.0,
                header_y,
                self.layout.width() as f32,
                self.layout.row_height as f32,
            ],
            self.palette.header_background,
        ));
        for (index, column) in COLUMNS.iter().enumerate() {
            self.draw(
                column.title.as_bytes(),
                index,
                header_y,
                self.palette.header_text,
                out,
            );
        }
        for (r, row) in (1u32..).zip(snapshot.visible()) {
            self.paint_row(row, r, now_ns, out);
        }
    }

    fn paint_row(
        &mut self,
        row: &RowSnapshot,
        grid_row: u32,
        now_ns: u64,
        out: &mut InstanceBuffer,
    ) {
        let y = self.layout.row_y(grid_row) as f32;
        let full_row = [
            0.0,
            y,
            self.layout.width() as f32,
            self.layout.row_height as f32,
        ];
        if grid_row % 2 == 0 {
            out.push(Instance::solid(full_row, self.palette.stripe));
        }

        let direction_color = match row.direction {
            Direction::Up => Some(self.palette.up),
            Direction::Down => Some(self.palette.down),
            Direction::Flat => None,
        };
        if let Some([r, g, b, _]) = direction_color {
            let alpha = flash_alpha(now_ns.saturating_sub(row.last_change_ns));
            if alpha > 0.0 {
                out.push(Instance::solid(
                    self.cell_rect(LAST_COLUMN, y),
                    [r, g, b, alpha],
                ));
            }
        }

        self.cell.clear();
        // Fits: "SYM" + at most 10 digits of a u32 is 13 bytes of a 32-byte buffer.
        let _ = write!(self.cell, "SYM{:05}", row.symbol_id);
        self.draw_cell(0, y, self.palette.text, out);

        for (column, price) in [(1, row.bid), (2, row.ask)] {
            let color = self.price_text(price, self.palette.text);
            self.draw_cell(column, y, color, out);
        }
        let last_color = self.price_text(row.last, direction_color.unwrap_or(self.palette.text));
        self.draw_cell(LAST_COLUMN, y, last_color, out);

        self.cell.clear();
        // Fits: u64::MAX grouped is 26 bytes; wider than the column, it renders as '#'.
        let _ = write_grouped(&mut self.cell, row.volume, ',');
        self.draw_cell(4, y, self.palette.text, out);
    }

    /// Formats `price` into the cell buffer; returns the color to draw it with.
    fn price_text(&mut self, price: f64, color: [f32; 4]) -> [f32; 4] {
        self.cell.clear();
        if write_fixed(&mut self.cell, price, PRICE_DECIMALS).is_ok() {
            return color;
        }
        // NaN (no quote yet) or too long for any column: `draw` turns the latter into '#'.
        self.cell.clear();
        if price.is_finite() {
            // Longer than every column, so it renders as '#' fill.
            let _ = self.cell.push_str("################################");
            color
        } else {
            let _ = self.cell.push_str("-");
            self.palette.dim_text
        }
    }

    /// Background rectangle of one cell: the column plus half a character of margin on
    /// each side, so it never touches neighbouring columns.
    fn cell_rect(&self, column: usize, y: f32) -> [f32; 4] {
        let char_width = self.layout.char_width() as f32;
        let margin = char_width / 2.0;
        [
            self.layout.column_x(column) as f32 - margin,
            y,
            (COLUMNS[column].chars as f32) * char_width + 2.0 * margin,
            self.layout.row_height as f32,
        ]
    }

    fn draw_cell(&self, column: usize, y: f32, color: [f32; 4], out: &mut InstanceBuffer) {
        self.draw(self.cell.as_bytes(), column, y, color, out);
    }

    /// Draws `text` in `column` at row top `y`; text wider than the column becomes `#`s,
    /// like a spreadsheet, rather than a misleading truncated number.
    fn draw(&self, text: &[u8], column: usize, y: f32, color: [f32; 4], out: &mut InstanceBuffer) {
        let spec = COLUMNS[column];
        let width = spec.chars as usize;
        let overflow = text.len() > width;
        let len = if overflow { width } else { text.len() };
        let pad = match spec.align {
            Align::Left => 0,
            Align::Right => width - len,
        };

        let char_width = self.layout.char_width() as f32;
        let glyph_height = self.layout.glyph_height() as f32;
        let x0 = self.layout.column_x(column) as f32 + pad as f32 * char_width;
        let top = y + (self.layout.row_height - self.layout.glyph_height()) as f32 / 2.0;
        for (k, &byte) in text.iter().take(len).enumerate() {
            let byte = if overflow { b'#' } else { byte };
            if byte == b' ' {
                continue;
            }
            let uv = self.uvs[usize::from(byte & 0x7F)];
            let x = x0 + k as f32 * char_width;
            out.push(Instance::glyph(
                [x, top, char_width, glyph_height],
                uv,
                color,
            ));
        }
    }
}
