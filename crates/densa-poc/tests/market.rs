//! The market grid: columns, number formats, placeholders, direction colors, flashes.

use densa_data::{Direction, RowSnapshot, ViewportSnapshot};
use densa_poc::{
    flash_alpha, market_columns, market_painter, MarketGrid, FLASH_MAX_ALPHA, FLASH_NS,
    MARKET_GLYPHS,
};
use densa_render::{CellStyle, CellText, GridSource, InstanceBuffer, Palette};

const ROWS: usize = 4;
const LAST: usize = 3;

fn row(id: u32, bid: f64, ask: f64, last: f64, volume: u64) -> RowSnapshot {
    RowSnapshot {
        symbol_id: id,
        bid,
        ask,
        last,
        volume,
        updated_at_ns: 1,
        last_change_ns: 0,
        direction: Direction::Flat,
    }
}

fn snapshot(rows: &[RowSnapshot]) -> ViewportSnapshot<ROWS> {
    let mut snap = ViewportSnapshot::new();
    snap.rows[..rows.len()].copy_from_slice(rows);
    snap.len = rows.len() as u32;
    snap.seq = 1;
    snap
}

/// The text and style of every cell of the first row.
fn cells(row: RowSnapshot, now_ns: u64) -> Vec<(String, CellStyle)> {
    let snap = snapshot(&[row]);
    let grid = MarketGrid::new(&snap, now_ns, Palette::new());
    (0..market_columns().len())
        .map(|column| {
            let mut text = CellText::new();
            let style = grid.cell(0, column, &mut text);
            (text.as_str().to_owned(), style)
        })
        .collect()
}

fn texts(row: RowSnapshot) -> Vec<String> {
    cells(row, 0).into_iter().map(|(text, _)| text).collect()
}

#[test]
fn rows_are_formatted_per_column() {
    assert_eq!(
        texts(row(42, 9.5, 10.5, 10.0, 1_234)),
        ["SYM00042", "9.50", "10.50", "10.00", "1,234"]
    );
}

#[test]
fn missing_quotes_render_a_dimmed_dash() {
    let palette = Palette::new();
    let cells = cells(row(1, f64::NAN, 1.0, f64::NAN, 0), 0);
    assert_eq!(cells[1].0, "-");
    assert_eq!(cells[1].1.color, palette.dim_text);
    assert_eq!(cells[LAST].0, "-");
}

#[test]
fn source_has_one_row_per_visible_snapshot_row() {
    let snap = snapshot(&[row(1, 1.0, 1.0, 1.0, 1), row(2, 1.0, 1.0, 1.0, 1)]);
    assert_eq!(MarketGrid::new(&snap, 0, Palette::new()).rows(), 2);
}

#[test]
fn flash_alpha_fades_linearly_to_zero() {
    assert_eq!(flash_alpha(0), FLASH_MAX_ALPHA);
    assert!(flash_alpha(FLASH_NS / 2) < FLASH_MAX_ALPHA);
    assert!(flash_alpha(FLASH_NS / 2) > 0.0);
    assert_eq!(flash_alpha(FLASH_NS), 0.0);
    assert_eq!(flash_alpha(u64::MAX), 0.0);
}

// Design constraints, checked at compile time: a symbol may change price ~20x/s, so long
// flashes never fade; and text must stay readable over the flash.
const _: () = assert!(FLASH_NS <= 300_000_000, "flash too long to ever fade");
const _: () = assert!(
    FLASH_MAX_ALPHA <= 0.25,
    "flash too opaque for readable text"
);

#[test]
fn only_the_last_cell_of_a_recent_change_flashes() {
    let palette = Palette::new();
    let now = 10 * FLASH_NS;
    let up = RowSnapshot {
        direction: Direction::Up,
        last_change_ns: now - FLASH_NS / 4,
        ..row(0, 1.0, 1.1, 1.05, 1)
    };
    let cells = cells(up, now);

    // Only the LAST cell, not the whole row: at ~20 price changes per second per symbol a
    // whole-row flash turns the grid into a wall of color (seen in a screenshot).
    let flashing: Vec<usize> = (0..cells.len())
        .filter(|&c| cells[c].1.background.is_some())
        .collect();
    assert_eq!(flashing, [LAST]);
    let [r, g, b, a] = cells[LAST].1.background.unwrap();
    assert_eq!([r, g, b], palette.positive[..3]);
    assert!((a - flash_alpha(FLASH_NS / 4)).abs() < 1e-6);
}

#[test]
fn old_changes_no_longer_flash() {
    let now = 10 * FLASH_NS;
    let down_but_old = RowSnapshot {
        direction: Direction::Down,
        last_change_ns: now - 2 * FLASH_NS,
        ..row(1, 1.0, 1.1, 1.05, 1)
    };
    assert!(cells(down_but_old, now)
        .iter()
        .all(|(_, s)| s.background.is_none()));
}

#[test]
fn last_price_text_is_colored_by_direction() {
    let palette = Palette::new();
    let down = RowSnapshot {
        direction: Direction::Down,
        ..row(0, 1.0, 1.1, 1.05, 1)
    };
    let cells = cells(down, 0);
    assert_eq!(cells[LAST].1.color, palette.negative);
    assert_eq!(cells[1].1.color, palette.text, "BID keeps the plain color");
}

#[test]
fn prices_too_long_for_the_buffer_overflow_to_hash_fill() {
    let painter = market_painter(1).unwrap();
    let snap = snapshot(&[row(1, 1e300, 1.0, 1.0, 0)]);
    let grid = MarketGrid::new(&snap, 0, *painter.palette());
    let mut text = CellText::new();
    grid.cell(0, 1, &mut text);
    assert!(text.as_str().chars().all(|c| c == '#'), "{}", text.as_str());
}

#[test]
fn worst_case_viewport_fits_capacity_and_needs_no_new_glyphs() {
    let mut painter = market_painter(50).unwrap();
    painter.drain_uploads().for_each(drop);
    let mut buf = InstanceBuffer::with_capacity(painter.layout().max_instances());
    let mut snap = ViewportSnapshot::<50>::new();
    for (i, r) in snap.rows.iter_mut().enumerate() {
        *r = RowSnapshot {
            direction: Direction::Up,
            last_change_ns: 100,
            ..row(
                4_000_000_000 + i as u32,
                9_999_999.99,
                f64::NAN,
                1e15,
                u64::MAX,
            )
        };
    }
    snap.len = 50;
    painter.paint(&MarketGrid::new(&snap, 100, *painter.palette()), &mut buf);
    assert_eq!(
        buf.dropped(),
        0,
        "capacity {}",
        painter.layout().max_instances()
    );
    assert_eq!(
        painter.drain_uploads().count(),
        0,
        "a glyph is missing from MARKET_GLYPHS ({MARKET_GLYPHS})"
    );
}
