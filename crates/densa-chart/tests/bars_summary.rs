//! Bars and candles drawn through their summaries are drawn as without one, pixel for pixel,
//! whatever the series (dense, sparse, repeated x, gaps, runs of gaps, values that are not
//! finite, values far past the plot, ties for the largest bar), the plot, the view and the
//! width of a bar; a summary grown as items arrive equals one made at once; its extents are
//! `y_extent`'s and `candle_extent`'s.

use densa_chart::{
    candle_extent, paint_bars, paint_bars_summarized, paint_candles, paint_candles_summarized,
    y_extent, Axis, BarStyle, BarSummary, Candle, CandleStyle, CandleSummary, Color, Plot, Rect,
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

/// `count` ascending `x`: some repeated, some far apart, dense or sparse; now and then an
/// infinite one at either end.
fn xs(random: &mut Random, count: usize) -> Vec<f64> {
    let dense = random.below(2) == 0;
    let mut x = random.unit() * 100.0 - 50.0;
    let mut out: Vec<f64> = (0..count)
        .map(|_| {
            x += match random.below(20) {
                0 => 0.0,
                1 => random.unit() * 500.0,
                _ if dense => random.unit() * 0.05,
                _ => random.unit() * 3.0,
            };
            x
        })
        .collect();
    if count > 2 && random.below(8) == 0 {
        out[0] = f64::NEG_INFINITY;
        out[count - 1] = f64::INFINITY;
    }
    out
}

/// A value that is not finite now and then, in runs too; `value` otherwise.
struct Gaps {
    left: u64,
}

impl Gaps {
    fn or(&mut self, random: &mut Random, value: f64) -> f64 {
        if self.left == 0 && random.below(200) == 0 {
            self.left = 1 + random.below(80);
        }
        if self.left > 0 {
            self.left -= 1;
            return f64::NAN;
        }
        match random.below(2_000) {
            0 => f64::INFINITY,
            1 => f64::NEG_INFINITY,
            _ => value,
        }
    }
}

/// `count` bars of a random kind: whole numbers around zero (so many tie for the largest),
/// now and then one far past any plot, gaps and runs of gaps.
fn bars(random: &mut Random, count: usize) -> Vec<(f64, f64)> {
    let xs = xs(random, count);
    let mut gaps = Gaps { left: 0 };
    let mut level = 0.0_f64;
    xs.into_iter()
        .map(|x| {
            level = (level + (random.unit() * 4.0 - 2.0).round()).clamp(-40.0, 40.0);
            let y = if random.below(500) == 0 {
                level.signum() * 1e9 * (1.0 + random.unit())
            } else {
                level
            };
            (x, gaps.or(random, y))
        })
        .collect()
}

/// `count` candles of a random kind: a walk with highs and lows around open and close, now and
/// then one far past any plot, one whose low is above its high, and gaps.
fn candles(random: &mut Random, count: usize) -> Vec<(f64, Candle)> {
    let xs = xs(random, count);
    let mut gaps = Gaps { left: 0 };
    let mut price = 0.0_f64;
    xs.into_iter()
        .map(|x| {
            let open = price;
            price += random.unit() * 4.0 - 2.0;
            let close = price;
            let mut high = open.max(close) + random.unit() * 3.0;
            let mut low = open.min(close) - random.unit() * 3.0;
            match random.below(400) {
                0 => high = 1e9,
                1 => low = -1e9,
                2 => std::mem::swap(&mut high, &mut low),
                _ => {}
            }
            let candle = match random.below(4) {
                0 => Candle::new(gaps.or(random, open), high, low, close),
                1 => Candle::new(open, gaps.or(random, high), low, close),
                2 => Candle::new(open, high, gaps.or(random, low), close),
                _ => Candle::new(open, high, low, gaps.or(random, close)),
            };
            (x, candle)
        })
        .collect()
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

/// A plot over a random view of `x` (finite ones) in a random area, its vertical axis around
/// zero and narrower than the values now and then.
fn plot(random: &mut Random, x: &[f64]) -> Plot {
    let finite: Vec<f64> = x.iter().copied().filter(|x| x.is_finite()).collect();
    let (first, last) = match (finite.first(), finite.last()) {
        (Some(&first), Some(&last)) => (first, last.max(first + 1.0)),
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
    let area = Rect::new(left, 3.5, width, 60.0 + random.below(300) as f32);
    let reach = 1.0 + random.unit() * 60.0;
    // Around zero, or now and then away from it, so bars start at the plot's edge.
    let away = [0.0, 0.0, 30.0, -30.0][random.below(4) as usize];
    let y = Axis::number(away - reach * random.unit(), away + reach);
    Plot::new(area, x, y)
}

/// A distance between items to give the painters: from far less than a pixel to many.
fn interval(random: &mut Random, plot: &Plot) -> f64 {
    let pixel = plot.x().span() / f64::from(plot.area().width());
    pixel * 10_f64.powf(random.unit() * 4.0 - 2.5)
}

/// A color of its own for each index (of the first thousand or so).
fn color_of(index: usize) -> Color {
    // Lossless: small whole numbers.
    let part = |n: usize| (index % n) as f32 / n as f32;
    Color::linear(part(7), part(11), part(13), 1.0)
}

#[test]
fn bars_drawn_through_their_summary_are_the_bars_drawn_without_one() {
    let mut random = Random(0x2545_f491_4f6c_dd1d);
    for case in 0..300 {
        let count = match case % 5 {
            0 => random.below(70) as usize,
            1 => random.below(2_000) as usize,
            _ => 1_000 + random.below(40_000) as usize,
        };
        let bars = bars(&mut random, count);
        let summary = BarSummary::of(&bars[..]);
        assert_eq!(summary.len(), count);
        let x: Vec<f64> = bars.iter().map(|bar| bar.0).collect();
        for view in 0..4 {
            let plot = plot(&mut random, &x);
            let interval = interval(&mut random, &plot);
            let style = BarStyle::new().with_body(0.1 + random.unit() as f32);
            let plain = drawn(|canvas| {
                paint_bars(canvas, &plot, &bars[..], interval, style, color_of);
            });
            let summarized = drawn(|canvas| {
                paint_bars_summarized(
                    canvas,
                    &plot,
                    &bars[..],
                    &summary,
                    interval,
                    style,
                    color_of,
                );
            });
            assert_eq!(summarized, plain, "case {case}, view {view}");
        }
    }
}

#[test]
fn candles_drawn_through_their_summary_are_the_candles_drawn_without_one() {
    let mut random = Random(0x9e37_79b9_7f4a_7c15);
    let style = CandleStyle::new(Color::WHITE, Color::BLACK);
    for case in 0..300 {
        let count = match case % 5 {
            0 => random.below(70) as usize,
            1 => random.below(2_000) as usize,
            _ => 1_000 + random.below(40_000) as usize,
        };
        let candles = candles(&mut random, count);
        let summary = CandleSummary::of(&candles[..]);
        assert_eq!(summary.len(), count);
        let x: Vec<f64> = candles.iter().map(|candle| candle.0).collect();
        for view in 0..4 {
            let plot = plot(&mut random, &x);
            let interval = interval(&mut random, &plot);
            let style = style.with_body(0.1 + random.unit() as f32);
            let plain = drawn(|canvas| paint_candles(canvas, &plot, &candles[..], interval, style));
            let summarized = drawn(|canvas| {
                paint_candles_summarized(canvas, &plot, &candles[..], &summary, interval, style);
            });
            assert_eq!(summarized, plain, "case {case}, view {view}");
        }
    }
}

#[test]
fn extents_are_those_of_y_extent_and_candle_extent_for_any_range() {
    let mut random = Random(0xdead_beef_cafe_f00d);
    for case in 0..200 {
        let count = random.below(30_000) as usize;
        let bars = bars(&mut random, count);
        let candles = candles(&mut random, count);
        let (bar_summary, candle_summary) =
            (BarSummary::of(&bars[..]), CandleSummary::of(&candles[..]));
        for _ in 0..20 {
            let start = random.below(count as u64 + 10) as usize;
            let end = start + random.below(count as u64 + 10) as usize;
            assert_eq!(
                bar_summary.y_extent(&bars[..], start..end),
                y_extent(&bars[..], start..end),
                "case {case}, bars {start}..{end}"
            );
            assert_eq!(
                candle_summary.y_extent(&candles[..], start..end),
                candle_extent(&candles[..], start..end),
                "case {case}, candles {start}..{end}"
            );
        }
    }
}

#[test]
fn summaries_grown_as_items_arrive_are_those_made_at_once() {
    let mut random = Random(0x0123_4567_89ab_cdef);
    let all_bars = bars(&mut random, 50_000);
    let all_candles = candles(&mut random, 50_000);
    let x: Vec<f64> = all_bars.iter().map(|bar| bar.0).collect();
    let (mut bars, mut candles) = (BarSummary::new(), CandleSummary::new());
    let mut seen = 0;
    while seen < all_bars.len() {
        seen = (seen + 1 + random.below(3_000) as usize).min(all_bars.len());
        bars.update(&all_bars[..seen]);
        candles.update(&all_candles[..seen]);
        assert_eq!((bars.len(), candles.len()), (seen, seen));
    }
    let style = CandleStyle::new(Color::WHITE, Color::BLACK);
    for view in 0..20 {
        let plot = plot(&mut random, &x);
        let interval = interval(&mut random, &plot);
        let grown = drawn(|canvas| {
            paint_bars_summarized(
                canvas,
                &plot,
                &all_bars[..],
                &bars,
                interval,
                BarStyle::new(),
                color_of,
            );
            paint_candles_summarized(canvas, &plot, &all_candles[..], &candles, interval, style);
        });
        let plain = drawn(|canvas| {
            paint_bars(
                canvas,
                &plot,
                &all_bars[..],
                interval,
                BarStyle::new(),
                color_of,
            );
            paint_candles(canvas, &plot, &all_candles[..], interval, style);
        });
        assert_eq!(grown, plain, "view {view}");
    }
    for _ in 0..50 {
        let start = random.below(50_000) as usize;
        let end = start + random.below(50_000) as usize;
        assert_eq!(
            bars.y_extent(&all_bars[..], start..end),
            y_extent(&all_bars[..], start..end)
        );
        assert_eq!(
            candles.y_extent(&all_candles[..], start..end),
            candle_extent(&all_candles[..], start..end)
        );
    }
}

#[test]
fn a_summary_of_other_items_is_not_used_and_one_of_a_slid_window_is_made_anew() {
    let bars: Vec<(f64, f64)> = (0..20_000)
        .map(|i| (f64::from(i), f64::from(i % 7)))
        .collect();
    let summary = BarSummary::of(&bars[..]);
    // The same count and ends, other values between: not seen (documented); another count or
    // other ends: drawn as without a summary.
    let fewer = &bars[..19_000];
    let slid: Vec<(f64, f64)> = (500..20_500)
        .map(|i| (f64::from(i), f64::from(i % 5)))
        .collect();
    let plot = Plot::new(
        Rect::new(0.0, 0.0, 400.0, 100.0),
        Axis::number(0.0, 21_000.0),
        Axis::number(0.0, 7.0),
    );
    for others in [fewer, &slid[..]] {
        let plain =
            drawn(|canvas| paint_bars(canvas, &plot, others, 1.0, BarStyle::new(), color_of));
        let stale = drawn(|canvas| {
            paint_bars_summarized(
                canvas,
                &plot,
                others,
                &summary,
                1.0,
                BarStyle::new(),
                color_of,
            );
        });
        assert_eq!(stale, plain);
        assert_eq!(
            summary.y_extent(others, 0..others.len()),
            y_extent(others, 0..others.len())
        );
    }
    // A window that slid: updated, it summarizes the window anew.
    let mut moving = summary.clone();
    moving.update(&slid[..]);
    assert_eq!(moving.len(), slid.len());
    assert_eq!(moving.y_extent(&slid[..], 0..100), Some((0.0, 4.0)));
    let candles: Vec<(f64, Candle)> = (0..20_000)
        .map(|i| (f64::from(i), Candle::new(1.0, 2.0, 0.0, 1.5)))
        .collect();
    let mut summary = CandleSummary::of(&candles[..]);
    // Fewer candles, or a window of them that slid: drawn as without a summary.
    let style = CandleStyle::new(Color::WHITE, Color::BLACK);
    let slid: Vec<(f64, Candle)> = (500..20_500)
        .map(|i| (f64::from(i), Candle::new(1.0, 3.0, 0.5, 2.0)))
        .collect();
    for others in [&candles[..19_000], &slid[..]] {
        let plot = plot.with_y(Axis::number(0.0, 4.0));
        let plain = drawn(|canvas| paint_candles(canvas, &plot, others, 1.0, style));
        let stale = drawn(|canvas| {
            paint_candles_summarized(canvas, &plot, others, &summary, 1.0, style);
        });
        assert_eq!(stale, plain);
        let all = 0..others.len();
        assert_eq!(
            summary.y_extent(others, all.clone()),
            candle_extent(others, all)
        );
    }
    summary.rebuild(&candles[..10]);
    assert_eq!(summary.len(), 10);
    assert_eq!(summary.y_extent(&candles[..10], 0..10), Some((0.0, 2.0)));
}

/// Bars that count how often they are read.
struct CountedBars<'a> {
    bars: &'a [(f64, f64)],
    reads: std::cell::Cell<usize>,
}

impl densa_chart::Points for CountedBars<'_> {
    fn len(&self) -> usize {
        self.bars.len()
    }

    fn x(&self, index: usize) -> f64 {
        self.reads.set(self.reads.get() + 1);
        self.bars[index].0
    }

    fn y(&self, index: usize) -> f64 {
        self.reads.set(self.reads.get() + 1);
        self.bars[index].1
    }
}

/// Candles that count how often they are read.
struct CountedCandles<'a> {
    candles: &'a [(f64, Candle)],
    reads: std::cell::Cell<usize>,
}

impl densa_chart::Candles for CountedCandles<'_> {
    fn len(&self) -> usize {
        self.candles.len()
    }

    fn x(&self, index: usize) -> f64 {
        self.reads.set(self.reads.get() + 1);
        self.candles[index].0
    }

    fn candle(&self, index: usize) -> Candle {
        self.reads.set(self.reads.get() + 1);
        self.candles[index].1
    }
}

#[test]
fn a_summarized_frame_and_extent_read_a_few_items_a_column_however_many_show() {
    // Two million bars and candles, a gap of a million in the middle.
    let gap = |i: u32| (500_000..1_500_000).contains(&i);
    let bars: Vec<(f64, f64)> = (0..2_000_000)
        .map(|i| {
            (
                f64::from(i),
                if gap(i) {
                    f64::NAN
                } else {
                    f64::from(i % 1_009)
                },
            )
        })
        .collect();
    let candles: Vec<(f64, Candle)> = (0..2_000_000)
        .map(|i| {
            let v = if gap(i) {
                f64::NAN
            } else {
                f64::from(i % 1_009)
            };
            (f64::from(i), Candle::new(v, v + 1.0, v - 1.0, v))
        })
        .collect();
    let bars = CountedBars {
        bars: &bars,
        reads: std::cell::Cell::new(0),
    };
    let candles = CountedCandles {
        candles: &candles,
        reads: std::cell::Cell::new(0),
    };
    let (bar_summary, candle_summary) = (BarSummary::of(&bars), CandleSummary::of(&candles));
    let plot = Plot::new(
        Rect::new(0.0, 0.0, 1_000.0, 300.0),
        Axis::number(0.0, 2_000_000.0),
        Axis::number(-1.0, 1_010.0),
    );
    let style = CandleStyle::new(Color::WHITE, Color::BLACK);
    bars.reads.set(0);
    candles.reads.set(0);
    let drawn_bars = drawn(|canvas| {
        paint_bars_summarized(
            canvas,
            &plot,
            &bars,
            &bar_summary,
            1.0,
            BarStyle::new(),
            color_of,
        );
    });
    let drawn_candles = drawn(|canvas| {
        paint_candles_summarized(canvas, &plot, &candles, &candle_summary, 1.0, style);
    });
    // A binary search, then about a run of 32 items at each column's edge and a few more.
    let reads = (bars.reads.take(), candles.reads.take());
    assert!(
        reads.0 < 200 * 1_000,
        "{} reads of bars for a frame 1,000 columns wide",
        reads.0
    );
    assert!(
        reads.1 < 200 * 1_000,
        "{} reads of candles for a frame",
        reads.1
    );
    // Something was drawn: a bar per column on either side of the gap.
    assert!(drawn_bars.len() > 400 && drawn_candles.len() > 400);
    assert_eq!(
        bar_summary.y_extent(&bars, 0..2_000_000),
        Some((0.0, 1_008.0))
    );
    assert_eq!(
        candle_summary.y_extent(&candles, 0..2_000_000),
        Some((-1.0, 1_009.0))
    );
    let reads = (bars.reads.take(), candles.reads.take());
    assert!(
        reads.0 < 1_000 && reads.1 < 1_000,
        "{reads:?} reads for extents"
    );
}
