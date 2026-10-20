//! Drawing onto a canvas: lines by pixel column, candles, bars and the axes around them.

use densa_chart::{
    paint_bars, paint_candles, paint_line, Axis, BarStyle, Candle, CandleStyle, ChartPainter,
    ChartPalette, Color, LineStyle, Plot, Rect,
};
use densa_draw::{Canvas, Instance, InstanceBuffer};
use densa_text::{AtlasConfig, Font, GlyphCache};

const RED: Color = Color::linear(1.0, 0.0, 0.0, 1.0);
const GREEN: Color = Color::linear(0.0, 1.0, 0.0, 1.0);

/// What `draw` puts on a fresh canvas.
fn drawn(draw: impl FnOnce(&mut Canvas<'_>)) -> Vec<Instance> {
    let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
    let mut out = InstanceBuffer::with_capacity(100_000);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    draw(&mut canvas);
    assert_eq!(out.dropped(), 0);
    out.as_slice().to_vec()
}

/// A 10 x 10 pixel plot of 0..10 by 0..10: value `v` is at pixel `v` across and `10 - v`
/// down.
fn square() -> Plot {
    Plot::new(
        Rect::new(0.0, 0.0, 10.0, 10.0),
        Axis::number(0.0, 10.0),
        Axis::number(0.0, 10.0),
    )
}

/// Coverage (alpha times height) drawn in pixel column `x`.
fn coverage(instances: &[Instance], x: f32) -> f32 {
    instances
        .iter()
        .filter(|i| i.rect[0] == x)
        .map(|i| i.color[3] * i.rect[3])
        .sum()
}

#[test]
fn plots_map_values_to_pixels_with_y_upward() {
    let plot = Plot::new(
        Rect::new(100.0, 50.0, 200.0, 100.0),
        Axis::number(0.0, 10.0),
        Axis::number(-1.0, 1.0),
    );
    assert_eq!(plot.x_px(0.0), 100.0);
    assert_eq!(plot.x_px(5.0), 200.0);
    assert_eq!(plot.y_px(-1.0), 150.0, "the bottom");
    assert_eq!(plot.y_px(1.0), 50.0, "the top");
    assert_eq!(plot.x_value(300.0), 10.0);
    assert_eq!(plot.y_value(100.0), 0.0);
    let moved = plot
        .with_x(Axis::number(10.0, 20.0))
        .with_y(Axis::number(0.0, 4.0));
    assert_eq!(moved.x_px(15.0), 200.0);
    assert_eq!(moved.y_px(1.0), 125.0);
    assert_eq!(moved.area(), plot.area());
}

#[test]
fn a_line_on_a_pixel_centre_is_one_solid_pixel_per_column() {
    let points = [(0.0, 4.5), (10.0, 4.5)];
    let instances = drawn(|canvas| paint_line(canvas, &square(), &points[..], LineStyle::new(RED)));
    let want: Vec<[f32; 4]> = (0..10).map(|x| [x as f32, 5.0, 1.0, 1.0]).collect();
    let got: Vec<[f32; 4]> = instances.iter().map(|i| i.rect).collect();
    assert_eq!(got, want);
    assert!(instances.iter().all(|i| i.color == RED.to_array()));
}

#[test]
fn a_line_between_pixels_shades_both_by_their_coverage() {
    let points = [(0.0, 5.0), (10.0, 5.0)];
    let instances = drawn(|canvas| paint_line(canvas, &square(), &points[..], LineStyle::new(RED)));
    let column: Vec<_> = instances.iter().filter(|i| i.rect[0] == 3.0).collect();
    assert_eq!(column.len(), 2);
    assert_eq!(column[0].rect, [3.0, 4.0, 1.0, 1.0]);
    assert_eq!(column[1].rect, [3.0, 5.0, 1.0, 1.0]);
    assert!(column.iter().all(|i| (i.color[3] - 0.5).abs() < 1e-6));
}

#[test]
fn a_slope_covers_its_rise_plus_its_width_in_every_column() {
    // From the bottom left to the top right: each column rises one pixel.
    let points = [(0.0, 0.0), (10.0, 10.0)];
    let style = LineStyle::new(RED).with_width(2.0);
    let instances = drawn(|canvas| paint_line(canvas, &square(), &points[..], style));
    for column in 0..10 {
        // Rows 9 - column .. 10 - column, widened by one pixel each way, within the plot.
        let top = (9.0 - column as f32 - 1.0).max(0.0);
        let bottom = (10.0 - column as f32 + 1.0).min(10.0);
        let got = coverage(&instances, column as f32);
        assert!(
            (got - (bottom - top)).abs() < 1e-5,
            "column {column}: {got}"
        );
    }
}

#[test]
fn a_line_draws_only_inside_its_plot_and_runs_to_its_edges() {
    let plot = Plot::new(
        Rect::new(20.0, 10.0, 30.0, 10.0),
        Axis::number(2.5, 8.5),
        Axis::number(0.0, 10.0),
    );
    let points: Vec<(f64, f64)> = (0..=10).map(|x| (f64::from(x), f64::from(x))).collect();
    let instances = drawn(|canvas| {
        paint_line(canvas, &plot, &points[..], LineStyle::new(RED));
        assert_eq!(canvas.clip_rect(), Rect::EVERYTHING, "the clip comes back");
    });
    let area = plot.area();
    for i in &instances {
        let [x, y, w, h] = i.rect;
        assert!(x >= area.x0 && x + w <= area.x1 && y >= area.y0 && y + h <= area.y1);
    }
    for column in 20..50 {
        assert!(
            instances.iter().any(|i| i.rect[0] == column as f32),
            "column {column} is empty"
        );
    }
}

#[test]
fn a_gap_leaves_its_columns_empty() {
    let points = [
        (0.5, 4.5),
        (2.5, 4.5),
        (4.5, f64::NAN),
        (6.5, 4.5),
        (9.5, 4.5),
    ];
    let instances = drawn(|canvas| paint_line(canvas, &square(), &points[..], LineStyle::new(RED)));
    let columns: Vec<f32> = instances.iter().map(|i| i.rect[0]).collect();
    assert_eq!(columns, [0.0, 1.0, 2.0, 6.0, 7.0, 8.0, 9.0]);
}

#[test]
fn a_million_points_cost_no_more_than_the_columns_they_cross() {
    let xs: Vec<f64> = (0..1_000_000).map(|i| f64::from(i) / 100_000.0).collect();
    let ys: Vec<f64> = (0..1_000_000)
        .map(|i| 5.0 + 4.0 * (f64::from(i) * 0.37).sin())
        .collect();
    let instances = drawn(|canvas| {
        paint_line(canvas, &square(), &(&xs[..], &ys[..]), LineStyle::new(RED));
    });
    assert!(instances.len() <= 3 * 10, "{} instances", instances.len());
    // Every column still reaches the series' highs and lows (1 and 9, so pixels 1..9).
    for column in 0..10 {
        assert!(coverage(&instances, column as f32) > 7.9, "column {column}");
    }
}

#[test]
fn line_widths_are_kept_sensible() {
    assert_eq!(LineStyle::new(RED).width, 1.0);
    assert_eq!(LineStyle::new(RED).with_width(0.0).width, 0.1);
    assert_eq!(LineStyle::new(RED).with_width(f32::NAN).width, 1.0);
}

#[test]
fn an_empty_plot_draws_nothing() {
    let empty = Plot::new(
        Rect::new(0.0, 0.0, 0.0, 10.0),
        Axis::number(0.0, 1.0),
        Axis::number(0.0, 1.0),
    );
    let points = [(0.0, 0.5), (1.0, 0.5)];
    let candles = [(0.5, Candle::new(0.2, 0.9, 0.1, 0.8))];
    let instances = drawn(|canvas| {
        paint_line(canvas, &empty, &points[..], LineStyle::new(RED));
        paint_candles(
            canvas,
            &empty,
            &candles[..],
            1.0,
            CandleStyle::new(GREEN, RED),
        );
        paint_bars(canvas, &empty, &points[..], 1.0, BarStyle::new(), |_| GREEN);
    });
    assert!(instances.is_empty());
}

/// A 100 x 100 plot of 0..10 across and 0..100 up: value `v` is `100 - v` pixels down.
fn chart() -> Plot {
    Plot::new(
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Axis::number(0.0, 10.0),
        Axis::number(0.0, 100.0),
    )
}

#[test]
fn a_candle_is_a_wick_from_low_to_high_and_a_body_from_open_to_close() {
    let candles = [
        (5.0, Candle::new(40.0, 80.0, 20.0, 60.0)),
        (7.0, Candle::new(60.0, 70.0, 30.0, 40.0)),
    ];
    let style = CandleStyle::new(GREEN, RED);
    let instances = drawn(|canvas| paint_candles(canvas, &chart(), &candles[..], 1.0, style));
    let rects: Vec<([f32; 4], [f32; 4])> = instances.iter().map(|i| (i.rect, i.color)).collect();
    assert_eq!(
        rects,
        [
            // Ten pixels apart: a 7 pixel body from 46.5 (rounded), a wick in its middle.
            ([50.0, 20.0, 1.0, 60.0], GREEN.to_array()),
            ([47.0, 40.0, 7.0, 20.0], GREEN.to_array()),
            ([70.0, 30.0, 1.0, 40.0], RED.to_array()),
            ([67.0, 40.0, 7.0, 20.0], RED.to_array()),
        ]
    );
}

#[test]
fn narrow_candles_are_one_bar_and_flat_ones_still_show() {
    let candles = [
        (5.0, Candle::new(40.0, 80.0, 20.0, 60.0)),
        (5.2, Candle::new(50.0, 50.0, 50.0, 50.0)),
        (5.4, Candle::new(f64::NAN, 1.0, 1.0, 1.0)),
    ];
    let style = CandleStyle::new(GREEN, RED);
    // Candles two pixels apart: one pixel wide.
    let instances = drawn(|canvas| paint_candles(canvas, &chart(), &candles[..], 0.2, style));
    let rects: Vec<[f32; 4]> = instances.iter().map(|i| i.rect).collect();
    assert_eq!(rects, [[50.0, 20.0, 1.0, 60.0], [52.0, 50.0, 1.0, 1.0]]);
}

#[test]
fn candle_bodies_are_kept_to_a_sensible_share() {
    let style = CandleStyle::new(GREEN, RED);
    assert_eq!(style.body, 0.7);
    assert_eq!(style.with_body(2.0).body, 1.0);
    assert_eq!(style.with_body(0.0).body, 0.1);
    assert_eq!(style.with_body(f32::NAN).body, 0.7);
    let whole = style.with_body(1.0);
    let candles = [(5.0, Candle::new(40.0, 80.0, 20.0, 60.0))];
    let instances = drawn(|canvas| paint_candles(canvas, &chart(), &candles[..], 1.0, whole));
    assert_eq!(instances[1].rect, [45.0, 40.0, 10.0, 20.0]);
}

#[test]
fn values_far_off_the_plot_are_cut_at_its_edge() {
    let candles = [(5.0, Candle::new(40.0, 1e300, -1e300, 60.0))];
    let style = CandleStyle::new(GREEN, RED);
    let instances = drawn(|canvas| paint_candles(canvas, &chart(), &candles[..], 1.0, style));
    assert_eq!(instances[0].rect, [50.0, 0.0, 1.0, 100.0]);
}

#[test]
fn bars_rise_from_zero_in_their_own_colors() {
    let plot = Plot::new(
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Axis::number(0.0, 10.0),
        Axis::number(-50.0, 50.0),
    );
    let bars = [(2.0, 30.0), (4.0, -20.0), (6.0, f64::NAN), (8.0, 10.0)];
    let instances = drawn(|canvas| {
        paint_bars(canvas, &plot, &bars[..], 1.0, BarStyle::new(), |index| {
            if index % 2 == 0 {
                GREEN
            } else {
                RED
            }
        });
    });
    let rects: Vec<([f32; 4], [f32; 4])> = instances.iter().map(|i| (i.rect, i.color)).collect();
    assert_eq!(
        rects,
        [
            ([17.0, 20.0, 7.0, 30.0], GREEN.to_array()),
            ([37.0, 50.0, 7.0, 20.0], RED.to_array()),
            ([77.0, 40.0, 7.0, 10.0], RED.to_array()),
        ]
    );
}

fn painter() -> ChartPainter {
    ChartPainter::new(Font::bundled_mono().unwrap(), 13.0, ChartPalette::new())
}

#[test]
fn the_plot_leaves_room_for_labels_right_and_below() {
    let mut painter = painter();
    let bounds = Rect::new(0.5, 10.0, 400.0, 300.0);
    let area = painter.plot_area(bounds, true);
    assert_eq!((area.x0, area.y0), (1.0, 10.0), "edges on whole pixels");
    assert!(
        area.x1 < 330.0 && area.x1 > 300.0,
        "eight digits of room: {area:?}"
    );
    assert!(
        area.y1 < 300.0 && area.y1 > 280.0,
        "a line of room: {area:?}"
    );
    let stacked = painter.plot_area(bounds, false);
    assert_eq!(stacked.y1, 310.0);
    let wider = ChartPainter::new(Font::bundled_mono().unwrap(), 13.0, ChartPalette::new())
        .with_label_chars(12);
    let mut wider = wider;
    assert!(wider.plot_area(bounds, true).x1 < area.x1);
}

#[test]
fn axes_draw_gridlines_edges_and_labels_outside_the_plot() {
    let mut painter = painter();
    let palette = *painter.palette();
    let bounds = Rect::new(0.0, 0.0, 400.0, 300.0);
    let area = painter.plot_area(bounds, true);
    let plot = Plot::new(area, Axis::number(0.0, 10.0), Axis::number(0.0, 100.0));
    let instances = drawn(|canvas| painter.paint_axes(canvas, &plot, true));

    let grid = palette.grid.to_array();
    let horizontal = instances
        .iter()
        .filter(|i| i.color == grid && i.rect[3] == 1.0 && i.rect[2] == area.width());
    assert!(horizontal.count() >= 3, "gridlines at the y ticks");
    let vertical = instances
        .iter()
        .filter(|i| i.color == grid && i.rect[2] == 1.0 && i.rect[3] == area.height());
    assert!(vertical.count() >= 2, "gridlines at the x ticks");
    let axis = palette.axis.to_array();
    assert_eq!(instances.iter().filter(|i| i.color == axis).count(), 2);

    let glyphs: Vec<&Instance> = instances.iter().filter(|i| i.is_glyph()).collect();
    assert!(
        glyphs.iter().any(|g| g.rect[0] >= area.x1),
        "y labels at the right"
    );
    assert!(
        glyphs.iter().any(|g| g.rect[1] >= area.y1),
        "x labels below"
    );
    assert!(glyphs
        .iter()
        .all(|g| g.rect[0] >= area.x1 || g.rect[1] >= area.y1));

    let without = drawn(|canvas| painter.paint_axes(canvas, &plot, false));
    assert!(without
        .iter()
        .filter(|i| i.is_glyph())
        .all(|g| g.rect[1] < area.y1));
}

#[test]
fn a_title_is_written_in_the_top_left_corner_on_the_background() {
    let mut painter = painter();
    let palette = *painter.palette();
    let area = Rect::new(20.0, 10.0, 300.0, 200.0);
    let plot = Plot::new(area, Axis::number(0.0, 10.0), Axis::number(0.0, 100.0));
    let instances = drawn(|canvas| painter.paint_title(canvas, &plot, "Volume"));

    let backing = &instances[0];
    assert!(!backing.is_glyph());
    assert_eq!(backing.color, palette.background.to_array());
    assert_eq!(
        (backing.rect[0], backing.rect[1]),
        (21.0, 11.0),
        "inside the edges"
    );
    let glyphs: Vec<&Instance> = instances.iter().filter(|i| i.is_glyph()).collect();
    assert_eq!(glyphs.len(), 6);
    assert!(glyphs.iter().all(|g| g.color == palette.label.to_array()));
    for glyph in &glyphs {
        let (x, y) = (glyph.rect[0], glyph.rect[1]);
        assert!(x > backing.rect[0] && x + glyph.rect[2] <= backing.rect[0] + backing.rect[2]);
        assert!(y >= backing.rect[1] && y + glyph.rect[3] <= backing.rect[1] + backing.rect[3]);
    }
}

#[test]
fn a_title_is_cut_off_where_the_plot_ends() {
    let mut painter = painter();
    let area = Rect::new(0.0, 0.0, 40.0, 100.0);
    let plot = Plot::new(area, Axis::number(0.0, 10.0), Axis::number(0.0, 100.0));
    let instances = drawn(|canvas| painter.paint_title(canvas, &plot, "A rather long title"));
    assert!(
        instances.iter().all(|i| i.rect[0] + i.rect[2] <= 40.0),
        "{instances:?}"
    );
    assert!(instances.iter().filter(|i| i.is_glyph()).count() < 19);

    let after = drawn(|canvas| {
        painter.paint_title(canvas, &plot, "A rather long title");
        canvas.fill_rect(Rect::new(100.0, 0.0, 10.0, 10.0), RED);
    });
    assert_eq!(after.last().unwrap().rect[0], 100.0, "the clip is put back");
}

#[test]
fn an_empty_title_or_plot_draws_nothing() {
    let mut painter = painter();
    let plot = Plot::new(
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Axis::number(0.0, 10.0),
        Axis::number(0.0, 100.0),
    );
    assert!(drawn(|canvas| painter.paint_title(canvas, &plot, "")).is_empty());
    let empty = Plot::new(Rect::new(0.0, 0.0, 0.0, 0.0), plot.x(), plot.y());
    assert!(drawn(|canvas| painter.paint_title(canvas, &empty, "Volume")).is_empty());
}

#[test]
fn an_empty_plot_has_no_axes() {
    let mut painter = painter();
    let plot = Plot::new(
        Rect::new(0.0, 0.0, 0.0, 0.0),
        Axis::number(0.0, 10.0),
        Axis::number(0.0, 100.0),
    );
    assert!(drawn(|canvas| painter.paint_axes(canvas, &plot, true)).is_empty());
}

#[test]
fn the_palette_can_change() {
    let mut painter = painter();
    let mut palette = ChartPalette::new();
    palette.grid = RED;
    painter.set_palette(palette);
    assert_eq!(painter.palette().grid, RED);
    assert_eq!(ChartPalette::default(), ChartPalette::new());
}

#[test]
fn a_three_pixel_body_has_its_wick_and_a_flat_candle_counts_as_up() {
    let candles = [(5.0, Candle::new(50.0, 80.0, 20.0, 50.0))];
    let style = CandleStyle::new(GREEN, RED);
    // Five pixels apart: a 3 pixel body (3.5 rounded down) around pixel 50.
    let instances = drawn(|canvas| paint_candles(canvas, &chart(), &candles[..], 0.5, style));
    let rects: Vec<([f32; 4], [f32; 4])> = instances.iter().map(|i| (i.rect, i.color)).collect();
    assert_eq!(
        rects,
        [
            ([50.0, 20.0, 1.0, 60.0], GREEN.to_array()),
            ([49.0, 50.0, 3.0, 1.0], GREEN.to_array()),
        ]
    );
}

#[test]
fn a_bar_is_never_wider_than_the_plot() {
    let bars = [(2.0, 50.0)];
    let instances = drawn(|canvas| {
        paint_bars(
            canvas,
            &chart(),
            &bars[..],
            1_000.0,
            BarStyle::new(),
            |_| GREEN,
        )
    });
    // A plot-wide bar centred on 20 pixels, cut at the left edge.
    assert_eq!(instances[0].rect, [0.0, 50.0, 70.0, 50.0]);
}

#[test]
fn gridlines_stay_inside_the_plot() {
    let mut painter = painter();
    let palette = *painter.palette();
    let area = painter.plot_area(Rect::new(0.0, 0.0, 400.0, 300.0), true);
    // Ticks fall on both edges of both axes.
    let plot = Plot::new(area, Axis::number(0.0, 10.0), Axis::number(0.0, 100.0));
    let instances = drawn(|canvas| painter.paint_axes(canvas, &plot, true));
    let grid = palette.grid.to_array();
    for line in instances.iter().filter(|i| i.color == grid) {
        let [x, y, w, h] = line.rect;
        assert!(x >= area.x0 && x + w <= area.x1, "{:?}", line.rect);
        assert!(y >= area.y0 && y + h <= area.y1, "{:?}", line.rect);
    }
}

#[test]
fn a_line_with_gaps_still_costs_at_most_three_rects_per_column() {
    let xs: Vec<f64> = (0..100_000).map(|i| f64::from(i) / 10_000.0).collect();
    let ys: Vec<f64> = (0..100_000)
        .map(|i| {
            if i % 2 == 0 {
                f64::NAN
            } else {
                f64::from(i % 10)
            }
        })
        .collect();
    let instances = drawn(|canvas| {
        paint_line(canvas, &square(), &(&xs[..], &ys[..]), LineStyle::new(RED));
    });
    assert!(instances.len() <= 3 * 10, "{} instances", instances.len());
}

#[test]
fn candles_denser_than_pixels_merge_into_one_per_column() {
    // A hundred thousand candles across 100 pixels, each a step up from the last.
    let candles: Vec<(f64, Candle)> = (0..100_000)
        .map(|i| {
            let open = f64::from(i) / 1_000.0;
            (
                f64::from(i) / 10_000.0,
                Candle::new(open, open + 0.5, open, open + 0.001),
            )
        })
        .collect();
    let style = CandleStyle::new(GREEN, RED);
    let instances = drawn(|canvas| paint_candles(canvas, &chart(), &candles[..], 1e-4, style));
    assert!(instances.len() <= 100, "{} instances", instances.len());
    assert!(instances.iter().all(|i| i.color == GREEN.to_array()));

    // A column whose candles fall overall is drawn falling, whatever each one did.
    let falling: Vec<(f64, Candle)> = (0..1_000)
        .map(|i| {
            let open = 90.0 - f64::from(i) / 100.0;
            // One spike, halfway through, reaches 95.
            let high = if i == 500 { 95.0 } else { open };
            let x = 5.0 + f64::from(i) / 100_000.0;
            (x, Candle::new(open, high, open - 0.02, open + 0.001))
        })
        .collect();
    let instances = drawn(|canvas| paint_candles(canvas, &chart(), &falling[..], 1e-5, style));
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].color, RED.to_array());
    // From the spike (95) to the last low (80.01 - 0.02).
    assert_eq!(instances[0].rect, [50.0, 5.0, 1.0, 15.0]);
}

#[test]
fn bars_denser_than_pixels_merge_in_the_color_of_the_tallest() {
    let bars: Vec<(f64, f64)> = (0..100_000)
        .map(|i| (f64::from(i) / 10_000.0, f64::from(i % 50)))
        .collect();
    let instances = drawn(|canvas| {
        paint_bars(
            canvas,
            &chart(),
            &bars[..],
            1e-4,
            BarStyle::new(),
            |index| {
                if index % 50 == 49 {
                    RED
                } else {
                    GREEN
                }
            },
        );
    });
    assert!(instances.len() <= 100, "{} instances", instances.len());
    assert!(instances
        .iter()
        .all(|i| i.color == RED.to_array() && i.rect[3] == 49.0));
}

#[test]
fn bar_bodies_take_their_share() {
    let bars = [(5.0, 50.0)];
    let wide = BarStyle::new().with_body(1.0);
    let instances = drawn(|canvas| paint_bars(canvas, &chart(), &bars[..], 1.0, wide, |_| GREEN));
    assert_eq!(instances[0].rect, [45.0, 50.0, 10.0, 50.0]);
    assert_eq!(BarStyle::default(), BarStyle::new());
    assert_eq!(BarStyle::new().with_body(5.0).body, 1.0);
    assert_eq!(BarStyle::new().with_body(f32::NAN).body, 0.7);
    let mut raw = BarStyle::new();
    raw.body = 50.0;
    let held = drawn(|canvas| paint_bars(canvas, &chart(), &bars[..], 1.0, raw, |_| GREEN));
    assert_eq!(
        held[0].rect,
        [45.0, 50.0, 10.0, 50.0],
        "held to one interval"
    );
}

#[test]
fn labels_below_a_second_get_room_for_their_digits() {
    let mut painter = painter();
    // A plot about 897 pixels wide: room for ten 11-character labels, which would put
    // "09:00:00.020" (12 characters) about 90 pixels from the next.
    let area = painter.plot_area(Rect::new(0.0, 0.0, 973.0, 300.0), true);
    let start = 1_792_400_400.0;
    let plot = Plot::new(
        area,
        Axis::time(start, start + 0.199),
        Axis::number(0.0, 1.0),
    );
    let grid = painter.palette().grid.to_array();
    let instances = drawn(|canvas| painter.paint_axes(canvas, &plot, true));
    let mut xs: Vec<f32> = instances
        .iter()
        .filter(|i| i.color == grid && i.rect[2] == 1.0)
        .map(|i| i.rect[0])
        .collect();
    xs.sort_by(f32::total_cmp);
    assert!(xs.len() >= 2);
    // A 12-character label and a gap: more than thirteen digits apart.
    assert!(xs.windows(2).all(|w| w[1] - w[0] > 13.0 * 7.8), "{xs:?}");
}

#[test]
fn a_width_set_directly_is_still_held_to_something_drawable() {
    let mut style = LineStyle::new(RED);
    style.width = f32::NAN;
    let points = [(0.0, 4.5), (10.0, 4.5)];
    let instances = drawn(|canvas| paint_line(canvas, &square(), &points[..], style));
    assert_eq!(instances.len(), 10, "one pixel thick");
}
