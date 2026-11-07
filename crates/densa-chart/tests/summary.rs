//! A line drawn through its summary is drawn as without one, pixel for pixel, whatever the
//! points (dense, sparse, repeated x, gaps, runs of gaps, values that are not finite), the plot
//! and the view; a summary grown point by point equals one made at once; its extents are
//! `y_extent`'s.

use densa_chart::{
    paint_line, paint_line_summarized, y_extent, Axis, Color, LineStyle, LineSummary, Plot, Rect,
};
use densa_draw::{Canvas, Instance, InstanceBuffer};
use densa_text::{AtlasConfig, GlyphCache};

/// A small random generator (xorshift), so every run sees the same cases.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A number in `0..n`.
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// A number in `0.0..1.0`.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1_u64 << 53) as f64
    }
}

/// `count` points of a random kind: ascending `x` (some repeated, some far apart), a walking
/// `y` with gaps and runs of gaps, now and then a value that is not finite.
fn points(random: &mut Random, count: usize) -> Vec<(f64, f64)> {
    let dense = random.below(2) == 0;
    let mut x = random.unit() * 100.0 - 50.0;
    let mut y = 0.0;
    let mut gap = 0;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        x += match random.below(20) {
            0 => 0.0,
            1 => random.unit() * 500.0,
            _ if dense => random.unit() * 0.05,
            _ => random.unit() * 3.0,
        };
        y += random.unit() * 2.0 - 1.0;
        if gap == 0 && random.below(200) == 0 {
            gap = 1 + random.below(80);
        }
        let value = if gap > 0 {
            gap -= 1;
            f64::NAN
        } else {
            match random.below(2_000) {
                0 => f64::INFINITY,
                1 => f64::NEG_INFINITY,
                _ => y,
            }
        };
        out.push((x, value));
    }
    out
}

/// What `draw` puts on a fresh canvas.
fn drawn(draw: impl FnOnce(&mut Canvas<'_>)) -> Vec<Instance> {
    let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
    let mut out = InstanceBuffer::with_capacity(200_000);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    draw(&mut canvas);
    assert_eq!(out.dropped(), 0);
    out.as_slice().to_vec()
}

/// A plot of `points` in a random area, over a random view of them.
fn plot(random: &mut Random, points: &[(f64, f64)]) -> Plot {
    let (first, last) = match (points.first(), points.last()) {
        (Some(first), Some(last)) => (first.0, last.0.max(first.0 + 1.0)),
        _ => (0.0, 1.0),
    };
    let span = last - first;
    let x = match random.below(4) {
        0 => Axis::number(first, last),
        1 => {
            let start = first + random.unit() * span;
            Axis::number(start, start + random.unit() * span * 0.1 + 1e-3)
        }
        2 => Axis::number(first - span * random.unit(), last + span * random.unit()),
        _ => {
            let start = first + random.unit() * span;
            Axis::number(start, start + random.unit() * span)
        }
    };
    let width = 1.0 + random.below(1_500) as f32 + random.unit() as f32;
    let left = random.unit() as f32 * 20.0;
    let area = Rect::new(left, 3.5, width, 120.0 + random.below(300) as f32);
    Plot::new(area, x, Axis::number(-60.0, 60.0))
}

fn style(random: &mut Random) -> LineStyle {
    LineStyle::new(Color::WHITE).with_width(0.5 + random.unit() as f32 * 3.0)
}

#[test]
fn a_line_drawn_through_its_summary_is_the_line_drawn_without_one() {
    let mut random = Random(0x2545_f491_4f6c_dd1d);
    for case in 0..400 {
        let count = match case % 5 {
            0 => random.below(70) as usize,
            1 => random.below(2_000) as usize,
            _ => 1_000 + random.below(60_000) as usize,
        };
        let points = points(&mut random, count);
        let summary = LineSummary::of(&points[..]);
        assert_eq!(summary.len(), count);
        for view in 0..4 {
            let plot = plot(&mut random, &points);
            let style = style(&mut random);
            let plain = drawn(|canvas| paint_line(canvas, &plot, &points[..], style));
            let summarized = drawn(|canvas| {
                paint_line_summarized(canvas, &plot, &points[..], &summary, style);
            });
            assert_eq!(summarized, plain, "case {case}, view {view}");
        }
    }
}

#[test]
fn infinite_x_at_the_ends_draws_as_without_a_summary() {
    let mut points: Vec<(f64, f64)> = (0..5_000)
        .map(|i| (f64::from(i) / 7.0, f64::from(i % 13)))
        .collect();
    points[0] = (f64::NEG_INFINITY, 3.0);
    points[4_999] = (f64::INFINITY, 5.0);
    let summary = LineSummary::of(&points[..]);
    let plot = Plot::new(
        Rect::new(0.0, 0.0, 300.0, 100.0),
        Axis::number(-10.0, 800.0),
        Axis::number(0.0, 13.0),
    );
    let style = LineStyle::new(Color::WHITE);
    let plain = drawn(|canvas| paint_line(canvas, &plot, &points[..], style));
    let summarized =
        drawn(|canvas| paint_line_summarized(canvas, &plot, &points[..], &summary, style));
    assert_eq!(summarized, plain);
    // Their y counts in the extent, as in y_extent.
    assert_eq!(
        summary.y_extent(&points[..], 0..5_000),
        y_extent(&points[..], 0..5_000)
    );
    // Dense, the ends' runs fit in a column: their values, far from the rest, are not drawn.
    let mut dense: Vec<(f64, f64)> = (0..5_000)
        .map(|i| (f64::from(i) * 1e-3, f64::from(i % 13)))
        .collect();
    dense[0] = (f64::NEG_INFINITY, 1e3);
    dense[4_999] = (f64::INFINITY, -1e3);
    let summary = LineSummary::of(&dense[..]);
    // The first run's drawn points (x 0.001 to 0.031) all in column 2 of 300.
    let plot = plot.with_x(Axis::number(-0.5, 60.5));
    let plain = drawn(|canvas| paint_line(canvas, &plot, &dense[..], style));
    let summarized =
        drawn(|canvas| paint_line_summarized(canvas, &plot, &dense[..], &summary, style));
    assert_eq!(summarized, plain);
}

#[test]
fn extents_are_those_of_y_extent_for_any_range() {
    let mut random = Random(0x9e37_79b9_7f4a_7c15);
    for case in 0..200 {
        let count = random.below(40_000) as usize;
        let points = points(&mut random, count);
        let summary = LineSummary::of(&points[..]);
        for _ in 0..20 {
            let start = random.below(count as u64 + 10) as usize;
            let end = start + random.below(count as u64 + 10) as usize;
            assert_eq!(
                summary.y_extent(&points[..], start..end),
                y_extent(&points[..], start..end),
                "case {case}, {start}..{end}"
            );
        }
    }
}

#[test]
fn a_summary_grown_as_points_arrive_is_one_made_at_once() {
    let mut random = Random(0xdead_beef_cafe_f00d);
    let all = points(&mut random, 70_000);
    let whole = LineSummary::of(&all[..]);
    let mut grown = LineSummary::new();
    let mut seen = 0;
    while seen < all.len() {
        seen = (seen + 1 + random.below(3_000) as usize).min(all.len());
        grown.update(&all[..seen]);
        assert_eq!(grown.len(), seen);
        assert_eq!(
            grown.y_extent(&all[..seen], 0..seen),
            y_extent(&all[..seen], 0..seen)
        );
    }
    for _ in 0..20 {
        let plot = plot(&mut random, &all);
        let style = style(&mut random);
        let from_whole =
            drawn(|canvas| paint_line_summarized(canvas, &plot, &all[..], &whole, style));
        let from_grown =
            drawn(|canvas| paint_line_summarized(canvas, &plot, &all[..], &grown, style));
        assert_eq!(from_grown, from_whole);
    }
    // Fewer points: summarized anew.
    grown.update(&all[..100]);
    assert_eq!(grown.len(), 100);
    assert_eq!(
        grown.y_extent(&all[..100], 0..100),
        y_extent(&all[..100], 0..100)
    );
    grown.rebuild(&all[..]);
    assert_eq!(grown.len(), all.len());
}

#[test]
fn a_summary_of_other_points_draws_and_measures_without_its_help() {
    let points: Vec<(f64, f64)> = (0..1_000)
        .map(|i| (f64::from(i), f64::from(i % 10)))
        .collect();
    let stale = LineSummary::of(&points[..500]);
    let plot = Plot::new(
        Rect::new(0.0, 0.0, 200.0, 100.0),
        Axis::number(0.0, 1_000.0),
        Axis::number(0.0, 10.0),
    );
    let style = LineStyle::new(Color::WHITE);
    let plain = drawn(|canvas| paint_line(canvas, &plot, &points[..], style));
    let summarized =
        drawn(|canvas| paint_line_summarized(canvas, &plot, &points[..], &stale, style));
    assert_eq!(summarized, plain);
    assert_eq!(stale.y_extent(&points[..], 0..1_000), Some((0.0, 9.0)));
}

#[test]
fn a_window_that_slides_is_seen_drawn_without_the_summary_and_summarized_anew() {
    let all: Vec<(f64, f64)> = (0..20_000)
        .map(|i| (f64::from(i), f64::from(i % 97)))
        .collect();
    let mut summary = LineSummary::of(&all[..10_000]);
    // The same count of points, slid on by 500: older ones dropped, newer ones added.
    let slid = &all[500..10_500];
    let plot = Plot::new(
        Rect::new(0.0, 0.0, 20.0, 100.0),
        Axis::number(500.0, 10_500.0),
        Axis::number(0.0, 97.0),
    );
    let style = LineStyle::new(Color::WHITE);
    let plain = drawn(|canvas| paint_line(canvas, &plot, slid, style));
    let stale = drawn(|canvas| paint_line_summarized(canvas, &plot, slid, &summary, style));
    assert_eq!(stale, plain, "drawn without the summary of other points");
    assert_eq!(summary.y_extent(slid, 0..100), y_extent(slid, 0..100));
    summary.update(slid);
    assert_eq!(summary.len(), 10_000);
    let counted = Counted {
        points: slid,
        reads: std::cell::Cell::new(0),
    };
    let fresh = drawn(|canvas| paint_line_summarized(canvas, &plot, &counted, &summary, style));
    assert_eq!(fresh, plain);
    let reads = counted.reads.take();
    assert!(
        reads < 10_000,
        "{reads} reads: the summary is of these points again"
    );
    // A value changed between the ends is not seen: rebuild after such a change.
    let mut changed = slid.to_vec();
    changed[5_000].1 = 1e6;
    summary.rebuild(&changed[..]);
    assert_eq!(summary.y_extent(&changed[..], 0..10_000), Some((0.0, 1e6)));
}

#[test]
fn column_pairs_time_axes_clipped_canvases_and_run_edges_draw_as_without_a_summary() {
    let mut random = Random(0x0123_4567_89ab_cdef);
    // Counts at the edges of runs of 32, 256 and 2,048 points, and just past them.
    for count in [
        31, 32, 33, 255, 256, 257, 2_047, 2_048, 2_049, 16_384, 16_385,
    ] {
        let points = points(&mut random, count);
        let (xs, ys): (Vec<f64>, Vec<f64>) = points.iter().copied().unzip();
        let pairs = (&xs[..], &ys[..]);
        let summary = LineSummary::of(&pairs);
        for _ in 0..6 {
            let plot = plot(&mut random, &points);
            let timed = plot.with_x(Axis::time(plot.x().start(), plot.x().end()));
            let style = style(&mut random);
            for plot in [plot, timed] {
                let plain = drawn(|canvas| paint_line(canvas, &plot, &pairs, style));
                let summarized =
                    drawn(|canvas| paint_line_summarized(canvas, &plot, &pairs, &summary, style));
                assert_eq!(summarized, plain, "{count} points");
                // On a canvas clipped to part of the plot.
                let area = plot.area();
                let part = Rect::new(area.x0 + area.width() / 3.0, area.y0, 40.0, area.height());
                let clipped = |summarized: bool| {
                    drawn(|canvas| {
                        canvas.clip(part);
                        if summarized {
                            paint_line_summarized(canvas, &plot, &pairs, &summary, style);
                        } else {
                            paint_line(canvas, &plot, &pairs, style);
                        }
                    })
                };
                assert_eq!(clipped(true), clipped(false), "{count} points, clipped");
            }
        }
    }
}

/// Gap rows whose `x` is not a number (padding), and values far beyond a tiny vertical axis
/// (whose pixels are not finite): drawn as without a summary.
#[test]
fn padding_rows_and_values_off_any_pixel_draw_as_without_a_summary() {
    let mut random = Random(0x7777_1234_abcd_0001);
    for _ in 0..40 {
        let mut points = points(&mut random, 20_000);
        for _ in 0..200 {
            let at = random.below(20_000) as usize;
            points[at] = (f64::NAN, f64::NAN);
        }
        let summary = LineSummary::of(&points[..]);
        let plot = plot(&mut random, &points);
        let style = style(&mut random);
        let plain = drawn(|canvas| paint_line(canvas, &plot, &points[..], style));
        let summarized =
            drawn(|canvas| paint_line_summarized(canvas, &plot, &points[..], &summary, style));
        assert_eq!(summarized, plain);
    }
    for far in [1e300, -1e300] {
        let huge: Vec<(f64, f64)> = (0..10_000)
            .map(|i| {
                let near = 1e-302 * f64::from(i % 7 + 1);
                (f64::from(i) / 40.0, if i % 3 == 0 { far } else { near })
            })
            .collect();
        let summary = LineSummary::of(&huge[..]);
        let plot = Plot::new(
            Rect::new(0.0, 0.0, 200.0, 100.0),
            Axis::number(0.0, 250.0),
            Axis::number(0.0, 1e-300),
        );
        let style = LineStyle::new(Color::WHITE);
        let plain = drawn(|canvas| paint_line(canvas, &plot, &huge[..], style));
        let summarized =
            drawn(|canvas| paint_line_summarized(canvas, &plot, &huge[..], &summary, style));
        assert_eq!(summarized, plain, "values at {far}");
    }
}

/// Points that count how often they are read.
struct Counted<'a> {
    points: &'a [(f64, f64)],
    reads: std::cell::Cell<usize>,
}

impl densa_chart::Points for Counted<'_> {
    fn len(&self) -> usize {
        self.points.len()
    }

    fn x(&self, index: usize) -> f64 {
        self.reads.set(self.reads.get() + 1);
        self.points[index].0
    }

    fn y(&self, index: usize) -> f64 {
        self.reads.set(self.reads.get() + 1);
        self.points[index].1
    }
}

#[test]
fn a_summarized_frame_and_extent_read_a_few_points_a_column_however_many_show() {
    // A million points of a gap in the middle, summarized as well.
    let all: Vec<(f64, f64)> = (0..2_000_000)
        .map(|i| {
            let gap = (500_000..1_500_000).contains(&i);
            (
                f64::from(i),
                if gap { f64::NAN } else { f64::from(i % 1_009) },
            )
        })
        .collect();
    let points = Counted {
        points: &all,
        reads: std::cell::Cell::new(0),
    };
    let summary = LineSummary::of(&points);
    let plot = Plot::new(
        Rect::new(0.0, 0.0, 1_000.0, 300.0),
        Axis::number(0.0, 2_000_000.0),
        Axis::number(0.0, 1_009.0),
    );
    points.reads.set(0);
    drawn(|canvas| {
        paint_line_summarized(
            canvas,
            &plot,
            &points,
            &summary,
            LineStyle::new(Color::WHITE),
        );
    });
    // A binary search, then about a run of 32 points at each column's edge and a few more.
    let reads = points.reads.take();
    assert!(
        reads < 200 * 1_000,
        "{reads} reads for a frame 1,000 columns wide"
    );
    assert_eq!(
        summary.y_extent(&points, 0..2_000_000),
        Some((0.0, 1_008.0))
    );
    let reads = points.reads.take();
    assert!(reads < 1_000, "{reads} reads for an extent");
}
