//! Generic grid painting: layout from font metrics, alignment, overflow, styles, capacity.

use densa_draw::{Canvas, Instance, InstanceBuffer};
use densa_grid::{Align, CellStyle, CellText, Color, Column, GridPainter, GridSource, Palette};
use densa_text::{AtlasConfig, Font, GlyphCache};

const PX: f32 = 14.0;

/// A grid of fixed strings with one style per column.
struct TestSource {
    rows: Vec<Vec<&'static str>>,
    styles: Vec<CellStyle>,
}

impl GridSource for TestSource {
    fn rows(&self) -> usize {
        self.rows.len()
    }

    fn cell(&self, row: usize, column: usize, text: &mut CellText) -> CellStyle {
        let _ = text.push_str(self.rows[row][column]);
        self.styles[column]
    }
}

fn columns() -> Vec<Column> {
    vec![
        Column::new("NAME", 6, Align::Left),
        Column::new("VALUE", 8, Align::Right),
    ]
}

fn white() -> CellStyle {
    CellStyle::new(Color::WHITE)
}

fn painter(rows: u32) -> GridPainter {
    GridPainter::new(
        columns(),
        rows,
        Font::bundled_mono().unwrap(),
        PX,
        Palette::new(),
    )
}

fn source(rows: Vec<Vec<&'static str>>) -> TestSource {
    TestSource {
        rows,
        styles: vec![white(), white()],
    }
}

fn paint(p: &mut GridPainter, s: &TestSource) -> InstanceBuffer {
    paint_with(p, s, &mut GlyphCache::new(AtlasConfig::DEFAULT))
}

/// One frame of `s` through `cache`.
fn paint_with(p: &mut GridPainter, s: &TestSource, cache: &mut GlyphCache) -> InstanceBuffer {
    let mut out = InstanceBuffer::with_capacity(p.layout().max_instances());
    let mut canvas = Canvas::begin(cache, &mut out);
    p.paint(s, &mut canvas);
    out
}

fn glyphs(buf: &InstanceBuffer) -> Vec<Instance> {
    buf.as_slice()
        .iter()
        .copied()
        .filter(Instance::is_glyph)
        .collect()
}

/// Glyphs whose vertical centre lies in grid row `row` (0 = header).
fn glyphs_in_row(p: &GridPainter, buf: &InstanceBuffer, row: u32) -> Vec<Instance> {
    let top = p.layout().row_y(row) as f32;
    let bottom = top + p.layout().row_height() as f32;
    glyphs(buf)
        .into_iter()
        .filter(|g| {
            let centre = g.rect[1] + g.rect[3] / 2.0;
            centre >= top && centre < bottom
        })
        .collect()
}

mod layout {
    use super::*;

    #[test]
    fn columns_are_sized_in_font_advances() {
        let p = painter(3);
        let l = p.layout();
        assert!(
            (l.char_width() - 0.6 * PX).abs() < 0.01,
            "JetBrains Mono is 0.6 em"
        );
        assert_eq!(l.column_x(0), l.padding());
        assert!(l.column_width(1) as f32 >= 8.0 * l.char_width() - 1.0);
        assert!(l.column_x(1) > l.column_x(0) + l.column_width(0));
    }

    #[test]
    fn rows_fit_the_fonts_line_height() {
        let p = painter(3);
        let line = Font::bundled_mono().unwrap().line_metrics(PX).line_height();
        assert!(p.layout().row_height() as f32 >= line);
        assert_eq!(
            p.layout().height(),
            2 * p.layout().padding() + 4 * p.layout().row_height()
        );
    }

    #[test]
    fn baseline_sits_inside_the_row() {
        let p = painter(3);
        let l = p.layout();
        let baseline = l.baseline_y(1);
        assert!(baseline > l.row_y(1) as f32);
        assert!(baseline < (l.row_y(1) + l.row_height()) as f32);
    }
}

mod text {
    use super::*;

    #[test]
    fn draws_one_glyph_per_inked_character() {
        let mut p = painter(1);
        let buf = paint(&mut p, &source(vec![vec!["ab c", "1.5"]]));
        // Header "NAME" + "VALUE" = 9; row: "ab c" has 3 inked glyphs, "1.5" 3.
        assert_eq!(glyphs(&buf).len(), 9 + 6);
        assert_eq!(buf.dropped(), 0);
    }

    #[test]
    fn right_aligned_text_ends_at_the_column_edge() {
        let mut p = painter(1);
        let buf = paint(&mut p, &source(vec![vec!["x", "42"]]));
        let l = p.layout();
        let right = (l.column_x(1) + l.column_width(1)) as f32;
        let ink_right = glyphs_in_row(&p, &buf, 1)
            .iter()
            .map(|g| g.rect[0] + g.rect[2])
            .fold(0.0, f32::max);
        assert!(
            ink_right <= right + 0.5,
            "{ink_right} past the edge {right}"
        );
        assert!(
            ink_right > right - l.char_width(),
            "{ink_right} far from {right}"
        );
    }

    #[test]
    fn left_aligned_text_starts_at_the_column_edge() {
        let mut p = painter(1);
        let buf = paint(&mut p, &source(vec![vec!["H", ""]]));
        let left = p.layout().column_x(0) as f32;
        let ink = glyphs_in_row(&p, &buf, 1)[0].rect[0];
        assert!(
            (ink - left).abs() < p.layout().char_width() / 2.0,
            "{ink} vs {left}"
        );
    }

    #[test]
    fn text_wider_than_its_column_becomes_hash_fill() {
        let mut p = painter(1);
        let buf = paint(&mut p, &source(vec![vec!["", "123456789012"]]));
        // VALUE is 8 characters wide: 12 digits overflow into 8 '#'.
        assert_eq!(glyphs_in_row(&p, &buf, 1).len(), 8);
    }

    #[test]
    fn text_over_cell_bytes_leaves_the_cell_blank() {
        let mut p = painter(1);
        // 65 bytes: FmtBuf rejects the whole write, so nothing is drawn (documented on
        // CELL_BYTES); sources are expected to write a placeholder instead.
        let long: &'static str = Box::leak("x".repeat(65).into_boxed_str());
        let buf = paint(&mut p, &source(vec![vec![long, ""]]));
        assert!(glyphs_in_row(&p, &buf, 1).is_empty());
    }

    #[test]
    fn non_ascii_text_is_drawn() {
        let mut p = painter(1);
        let buf = paint(&mut p, &source(vec![vec!["Café", ""]]));
        assert_eq!(glyphs_in_row(&p, &buf, 1).len(), 4);
    }

    #[test]
    fn glyphs_use_the_cell_style_colour() {
        let red = Color::linear(1.0, 0.0, 0.0, 1.0);
        let mut p = painter(1);
        let s = TestSource {
            rows: vec![vec!["a", "b"]],
            styles: vec![white(), CellStyle::new(red)],
        };
        let buf = paint(&mut p, &s);
        let row = glyphs_in_row(&p, &buf, 1);
        assert!(row.iter().any(|g| g.color == red.to_array()));
        assert!(row.iter().any(|g| g.color == Color::WHITE.to_array()));
    }
}

mod styles {
    use super::*;

    #[test]
    fn cell_backgrounds_cover_only_their_cell() {
        let bg = Color::linear(0.0, 0.5, 0.0, 0.25);
        let mut p = painter(1);
        let s = TestSource {
            rows: vec![vec!["a", "b"]],
            styles: vec![white(), CellStyle::new(Color::WHITE).with_background(bg)],
        };
        let buf = paint(&mut p, &s);
        let fills: Vec<_> = buf
            .as_slice()
            .iter()
            .filter(|i| i.color == bg.to_array())
            .collect();
        assert_eq!(fills.len(), 1);
        let l = p.layout();
        let [x, _, w, _] = fills[0].rect;
        assert!(
            x > (l.column_x(0) + l.column_width(0)) as f32,
            "clear of column 0"
        );
        assert!(x + w >= (l.column_x(1) + l.column_width(1)) as f32);
    }

    #[test]
    fn every_other_row_is_striped() {
        let palette = Palette::new();
        let mut p = painter(4);
        let buf = paint(&mut p, &source(vec![vec!["a", "b"]; 4]));
        let stripes = buf
            .as_slice()
            .iter()
            .filter(|i| i.color == palette.stripe.to_array())
            .count();
        assert_eq!(stripes, 2);
    }

    #[test]
    fn source_rows_beyond_the_layout_are_ignored() {
        let mut p = painter(2);
        let buf = paint(&mut p, &source(vec![vec!["a", "b"]; 10]));
        assert_eq!(glyphs(&buf).len(), 9 + 2 * 2);
    }
}

mod capacity {
    use super::*;

    #[test]
    fn worst_case_frame_fits_the_advertised_capacity() {
        let mut p = painter(20);
        let s = TestSource {
            rows: vec![vec!["WWWWWW", "88888888"]; 20],
            styles: vec![
                CellStyle::new(Color::WHITE).with_background(Color::WHITE),
                CellStyle::new(Color::WHITE).with_background(Color::WHITE),
            ],
        };
        let buf = paint(&mut p, &s);
        assert_eq!(buf.dropped(), 0, "capacity {}", p.layout().max_instances());
    }

    #[test]
    fn painting_replaces_the_previous_frame() {
        let mut p = painter(2);
        let two = paint(&mut p, &source(vec![vec!["a", "b"]; 2])).len();
        let one = paint(&mut p, &source(vec![vec!["a", "b"]; 1])).len();
        assert!(one < two);
    }

    #[test]
    fn new_glyphs_are_queued_for_upload_once() {
        let mut p = painter(1);
        let s = source(vec![vec!["abc", "1"]]);
        let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
        paint_with(&mut p, &s, &mut cache);
        let first = cache.drain_updates().count();
        assert!(first >= 4, "header and cell glyphs uploaded, got {first}");
        paint_with(&mut p, &s, &mut cache);
        assert_eq!(
            cache.drain_updates().count(),
            0,
            "warm frame uploads nothing"
        );
    }

    #[test]
    fn painters_append_to_one_frame_through_one_cache() {
        let (mut a, mut b) = (painter(1), painter(1));
        let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
        let mut out = InstanceBuffer::with_capacity(2 * a.layout().max_instances());
        let s = source(vec![vec!["ab", "1"]]);
        let one = paint(&mut a, &s).len();

        let mut canvas = Canvas::begin(&mut cache, &mut out);
        a.paint(&s, &mut canvas);
        let misses = canvas.cache().stats().misses;
        b.paint(&s, &mut canvas);
        assert_eq!(
            canvas.instances().len(),
            2 * one,
            "the second painter appends"
        );
        assert_eq!(
            canvas.cache().stats().misses,
            misses,
            "and reuses every glyph"
        );
    }
}

#[test]
fn painting_some_columns_draws_only_their_titles_and_cells() {
    let mut p = painter(2);
    let s = source(vec![vec!["AB", "1"], vec!["CD", "2"]]);
    let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
    let mut out = InstanceBuffer::with_capacity(p.layout().max_instances());
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    p.paint_columns(&s, &mut canvas, 0..1);
    let end = (p.layout().column_x(0) + p.layout().column_width(0)) as f32;
    for row in 0..3 {
        let row_glyphs = glyphs_in_row(&p, &out, row);
        assert!(!row_glyphs.is_empty(), "row {row} shows its first column");
        assert!(
            row_glyphs.iter().all(|g| g.rect[0] < end),
            "row {row} drew past the first column"
        );
    }
}
