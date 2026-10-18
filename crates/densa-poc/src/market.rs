//! The PoC's market grid: what its columns are and how a quote row is drawn.
//!
//! App code on top of the domain-agnostic `densa_render` grid: columns, number formats,
//! direction colors and the price-change flash all live here.

use densa_data::{Direction, RowSnapshot, ViewportSnapshot};
use densa_fmt::{write_fixed, write_grouped};
use densa_render::{Align, CellStyle, CellText, Column, GridPainter, GridSource, Palette};
use densa_text::{Font, GlyphCache};
use std::error::Error;
use std::fmt::Write as _;

/// Pixels per em of the grid font.
pub const FONT_PX: f32 = 13.0;

/// How long the LAST cell flashes after the last-traded price changes.
///
/// Short on purpose: a busy symbol changes price many times per second, and a flash that
/// outlives the gap between changes never fades.
pub const FLASH_NS: u64 = 300_000_000;

/// Opacity of the LAST cell flash at the moment the price changes; low enough that the
/// price stays readable over it.
pub const FLASH_MAX_ALPHA: f32 = 0.25;

/// Every character the market grid draws: header titles, symbols, numbers, placeholders.
/// Rasterized at startup so frames never rasterize (or allocate).
pub const MARKET_GLYPHS: &str = "SYMBOLIDASKTVUE0123456789.,-#";

/// Column indices, left to right.
const SYMBOL: usize = 0;
const BID: usize = 1;
const ASK: usize = 2;
const LAST: usize = 3;
const VOLUME: usize = 4;

const PRICE_DECIMALS: usize = 2;

/// The grid's columns, left to right: SYMBOL, BID, ASK, LAST, VOLUME.
pub fn market_columns() -> Vec<Column> {
    vec![
        Column::new("SYMBOL", 8, Align::Left),
        Column::new("BID", 10, Align::Right),
        Column::new("ASK", 10, Align::Right),
        Column::new("LAST", 10, Align::Right),
        Column::new("VOLUME", 15, Align::Right),
    ]
}

/// Flash opacity `age_ns` after a price change: full at 0, linearly fading to 0.
pub fn flash_alpha(age_ns: u64) -> f32 {
    if age_ns >= FLASH_NS {
        return 0.0;
    }
    let remaining = 1.0 - age_ns as f32 / FLASH_NS as f32;
    FLASH_MAX_ALPHA * remaining
}

/// One frame's view of a viewport snapshot as grid cells.
#[derive(Debug, Clone, Copy)]
pub struct MarketGrid<'a, const N: usize> {
    snapshot: &'a ViewportSnapshot<N>,
    now_ns: u64,
    palette: Palette,
}

impl<'a, const N: usize> MarketGrid<'a, N> {
    /// Cells of `snapshot` as of `now_ns` (the snapshot's clock; drives the flash fade).
    pub fn new(snapshot: &'a ViewportSnapshot<N>, now_ns: u64, palette: Palette) -> Self {
        Self {
            snapshot,
            now_ns,
            palette,
        }
    }

    fn direction_color(&self, row: &RowSnapshot) -> Option<[f32; 4]> {
        match row.direction {
            Direction::Up => Some(self.palette.positive),
            Direction::Down => Some(self.palette.negative),
            Direction::Flat => None,
        }
    }

    /// Formats `price`; returns the color to draw it with (dimmed for "no quote yet").
    fn price(&self, price: f64, color: [f32; 4], text: &mut CellText) -> [f32; 4] {
        if write_fixed(text, price, PRICE_DECIMALS).is_ok() {
            return color;
        }
        text.clear();
        if price.is_finite() {
            // Too long for the buffer: wider than any column, so it renders as '#' fill.
            let _ = text.push_str("################################");
            color
        } else {
            let _ = text.push_str("-");
            self.palette.dim_text
        }
    }

    fn last(&self, row: &RowSnapshot, text: &mut CellText) -> CellStyle {
        let direction = self.direction_color(row);
        let color = self.price(row.last, direction.unwrap_or(self.palette.text), text);
        let background = direction.and_then(|[r, g, b, _]| {
            let alpha = flash_alpha(self.now_ns.saturating_sub(row.last_change_ns));
            (alpha > 0.0).then_some([r, g, b, alpha])
        });
        CellStyle { color, background }
    }
}

impl<const N: usize> GridSource for MarketGrid<'_, N> {
    fn rows(&self) -> usize {
        self.snapshot.visible().len()
    }

    fn cell(&self, row: usize, column: usize, text: &mut CellText) -> CellStyle {
        let row = &self.snapshot.visible()[row];
        let plain = |color| CellStyle {
            color,
            background: None,
        };
        match column {
            SYMBOL => {
                // Fits: "SYM" + at most 10 digits of a u32.
                let _ = write!(text, "SYM{:05}", row.symbol_id);
                plain(self.palette.text)
            }
            BID => plain(self.price(row.bid, self.palette.text, text)),
            ASK => plain(self.price(row.ask, self.palette.text, text)),
            LAST => self.last(row, text),
            VOLUME => {
                // Fits: u64::MAX grouped is 26 bytes; wider than the column, it renders as '#'.
                let _ = write_grouped(text, row.volume, ',');
                plain(self.palette.text)
            }
            _ => plain(self.palette.text),
        }
    }
}

/// The market grid painter for `rows` data rows, with every market glyph already
/// rasterized into `cache` (their uploads are queued for the first frame).
///
/// # Errors
/// If the bundled font fails to load, or the glyph atlas cannot hold `MARKET_GLYPHS` (the
/// frame loop would then rasterize, and allocate, every frame).
pub fn market_painter(rows: u32, cache: &mut GlyphCache) -> Result<GridPainter, Box<dyn Error>> {
    let font = Font::bundled_mono()?;
    let mut painter = GridPainter::new(market_columns(), rows, font, FONT_PX, Palette::new());
    let missing = painter.prewarm(cache, MARKET_GLYPHS);
    if missing > 0 {
        return Err(
            format!("the glyph atlas cannot hold the market glyphs ({missing} missing)").into(),
        );
    }
    Ok(painter)
}
