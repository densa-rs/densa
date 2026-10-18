//! Grid painter: layout, alignment, placeholders, overflow, flashes, capacity.

use densa_data::{Direction, RowSnapshot, ViewportSnapshot};
use densa_render::{
    flash_alpha, GlyphAtlas, GridLayout, GridPainter, Instance, InstanceBuffer, Palette, COLUMNS,
    FLASH_NS,
};

const ROWS: usize = 4;

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

fn painter() -> GridPainter {
    GridPainter::new(GridLayout::new(ROWS as u32), &GlyphAtlas::font8x8())
}

fn glyphs(buf: &InstanceBuffer) -> Vec<Instance> {
    buf.as_slice()
        .iter()
        .copied()
        .filter(Instance::is_glyph)
        .collect()
}

fn solids(buf: &InstanceBuffer) -> Vec<Instance> {
    buf.as_slice()
        .iter()
        .copied()
        .filter(|i| !i.is_glyph())
        .collect()
}

/// Glyphs whose top edge lies within data row `r` (0-based, below the header).
fn glyphs_in_row(buf: &InstanceBuffer, layout: &GridLayout, r: u32) -> Vec<Instance> {
    let top = layout.row_y(r + 1) as f32;
    let bottom = top + layout.row_height as f32;
    glyphs(buf)
        .into_iter()
        .filter(|g| g.rect[1] >= top && g.rect[1] < bottom)
        .collect()
}

#[test]
fn layout_dimensions_follow_from_columns() {
    let layout = GridLayout::new(50);
    assert_eq!(layout.char_width(), 16);
    let chars: u32 = COLUMNS.iter().map(|c| c.chars).sum::<u32>() + layout.gap_chars * 4;
    assert_eq!(layout.width(), layout.padding * 2 + chars * 16);
    assert_eq!(layout.height(), layout.padding * 2 + 51 * layout.row_height);
    assert!(layout.height() <= 1000, "must fit a 1080p work area");
    assert_eq!(layout.column_x(0), layout.padding);
    assert_eq!(
        layout.column_x(1),
        layout.padding + (COLUMNS[0].chars + layout.gap_chars) * 16
    );
}

#[test]
fn draws_header_and_every_cell_as_glyphs() {
    let mut p = painter();
    let mut buf = InstanceBuffer::with_capacity(p.layout().max_instances());
    p.paint(&snapshot(&[row(42, 9.5, 10.5, 10.0, 1_234)]), 0, &mut buf);

    // "SYMBOL" "BID" "ASK" "LAST" "VOLUME" = 22 glyphs; one row:
    // "SYM00042"(8) "9.50"(4) "10.50"(5) "10.00"(5) "1,234"(5) = 27.
    assert_eq!(glyphs(&buf).len(), 22 + 27);
    assert_eq!(buf.dropped(), 0);
}

#[test]
fn numbers_are_right_aligned_to_their_column_edge() {
    let mut p = painter();
    let layout = *p.layout();
    let mut buf = InstanceBuffer::with_capacity(layout.max_instances());
    p.paint(&snapshot(&[row(1, 9.5, 10.5, 10.0, 7)]), 0, &mut buf);

    let bid_column_right = layout.column_x(1) + COLUMNS[1].chars * layout.char_width();
    let row_glyphs = glyphs_in_row(&buf, &layout, 0);
    let bid_glyphs: Vec<&Instance> = row_glyphs
        .iter()
        .filter(|g| g.rect[0] >= layout.column_x(1) as f32 && g.rect[0] < bid_column_right as f32)
        .collect();
    assert_eq!(bid_glyphs.len(), 4, "9.50");
    let last = bid_glyphs
        .iter()
        .map(|g| g.rect[0] + g.rect[2])
        .fold(0.0, f32::max);
    assert_eq!(last, bid_column_right as f32);
}

#[test]
fn missing_quotes_render_a_dash_and_overflow_renders_hashes() {
    let mut p = painter();
    let layout = *p.layout();
    let mut buf = InstanceBuffer::with_capacity(layout.max_instances());
    p.paint(
        &snapshot(&[row(1, f64::NAN, 1e15, f64::NAN, 0)]),
        0,
        &mut buf,
    );

    // "SYM00001"(8) "-"(1) "##########"(10: 1e15 with 2 decimals overflows) "-"(1) "0"(1)
    assert_eq!(glyphs_in_row(&buf, &layout, 0).len(), 8 + 1 + 10 + 1 + 1);
}

#[test]
fn recent_price_changes_flash_and_fade() {
    assert_eq!(flash_alpha(0), Palette::FLASH_MAX_ALPHA);
    assert!(flash_alpha(FLASH_NS / 2) < Palette::FLASH_MAX_ALPHA);
    assert!(flash_alpha(FLASH_NS / 2) > 0.0);
    assert_eq!(flash_alpha(FLASH_NS), 0.0);
    assert_eq!(flash_alpha(u64::MAX), 0.0);

    let palette = Palette::new();
    let mut p = painter();
    let mut buf = InstanceBuffer::with_capacity(p.layout().max_instances());
    let now = 10 * FLASH_NS;
    let up = RowSnapshot {
        direction: Direction::Up,
        last_change_ns: now - FLASH_NS / 4,
        ..row(0, 1.0, 1.1, 1.05, 1)
    };
    let down_but_old = RowSnapshot {
        direction: Direction::Down,
        last_change_ns: now - 2 * FLASH_NS,
        ..row(1, 1.0, 1.1, 1.05, 1)
    };
    p.paint(&snapshot(&[up, down_but_old]), now, &mut buf);

    let flashes: Vec<Instance> = solids(&buf)
        .into_iter()
        .filter(|s| s.color[..3] == palette.up[..3] || s.color[..3] == palette.down[..3])
        .collect();
    assert_eq!(flashes.len(), 1, "only the recent change flashes");
    assert_eq!(flashes[0].color[..3], palette.up[..3]);
    assert!((flashes[0].color[3] - flash_alpha(FLASH_NS / 4)).abs() < 1e-6);

    // Only the LAST cell flashes, not the whole row: at ~20 price changes per second per
    // symbol a whole-row flash turns the grid into a wall of color (seen in a screenshot).
    let layout = *p.layout();
    let [x, _, w, _] = flashes[0].rect;
    let last_x0 = layout.column_x(3) as f32;
    let last_x1 = last_x0 + (COLUMNS[3].chars * layout.char_width()) as f32;
    assert!(
        x <= last_x0 && x + w >= last_x1,
        "flash covers the LAST cell"
    );
    assert!(x > layout.column_x(2) as f32, "flash stays clear of ASK");
    assert!(
        x + w < layout.column_x(4) as f32,
        "flash stays clear of VOLUME"
    );
}

// Design constraints, checked at compile time: a symbol may change price ~20x/s, so long
// flashes never fade; and text must stay readable over the flash.
const _: () = assert!(FLASH_NS <= 300_000_000, "flash too long to ever fade");
const _: () = assert!(
    Palette::FLASH_MAX_ALPHA <= 0.25,
    "flash too opaque for readable text"
);

#[test]
fn last_price_text_is_colored_by_direction() {
    let palette = Palette::new();
    let mut p = painter();
    let layout = *p.layout();
    let mut buf = InstanceBuffer::with_capacity(layout.max_instances());
    let down = RowSnapshot {
        direction: Direction::Down,
        ..row(0, 1.0, 1.1, 1.05, 1)
    };
    p.paint(&snapshot(&[down]), 0, &mut buf);

    let last_x0 = layout.column_x(3) as f32;
    let last_x1 = last_x0 + (COLUMNS[3].chars * layout.char_width()) as f32;
    let last_glyphs: Vec<Instance> = glyphs_in_row(&buf, &layout, 0)
        .into_iter()
        .filter(|g| g.rect[0] >= last_x0 && g.rect[0] < last_x1)
        .collect();
    assert!(!last_glyphs.is_empty());
    assert!(last_glyphs.iter().all(|g| g.color == palette.down));
}

#[test]
fn worst_case_viewport_fits_the_advertised_capacity() {
    let layout = GridLayout::new(50);
    let mut p = GridPainter::new(layout, &GlyphAtlas::font8x8());
    let mut buf = InstanceBuffer::with_capacity(layout.max_instances());
    let mut snap = ViewportSnapshot::<50>::new();
    for (i, r) in snap.rows.iter_mut().enumerate() {
        *r = RowSnapshot {
            direction: Direction::Up,
            last_change_ns: 100,
            ..row(
                4_000_000_000 + i as u32,
                9_999_999.99,
                9_999_999.99,
                9_999_999.99,
                u64::MAX,
            )
        };
    }
    snap.len = 50;
    p.paint(&snap, 100, &mut buf);
    assert_eq!(
        buf.dropped(),
        0,
        "capacity {} too small",
        layout.max_instances()
    );
}

#[test]
fn painting_replaces_the_previous_frame() {
    let mut p = painter();
    let mut buf = InstanceBuffer::with_capacity(p.layout().max_instances());
    p.paint(
        &snapshot(&[row(1, 1.0, 1.1, 1.05, 1), row(2, 1.0, 1.1, 1.05, 1)]),
        0,
        &mut buf,
    );
    let two_rows = buf.len();
    p.paint(&snapshot(&[row(1, 1.0, 1.1, 1.05, 1)]), 0, &mut buf);
    assert!(buf.len() < two_rows);
}
