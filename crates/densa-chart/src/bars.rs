use densa_core::{Color, Rect};
use densa_draw::Canvas;

use crate::series::{candles_within, points_within, with_neighbours};
use crate::{Candles, Plot, Points};

/// How candlesticks are drawn: `CandleStyle::new(up, down)`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct CandleStyle {
    /// Candles that closed at or above their open.
    pub up: Color,
    /// Candles that closed below their open.
    pub down: Color,
    /// How much of the distance between two candles a body takes (0.1 to 1).
    pub body: f32,
}

impl CandleStyle {
    /// Candles in `up` and `down`, bodies taking 70% of the distance between candles.
    pub const fn new(up: Color, down: Color) -> Self {
        Self {
            up,
            down,
            body: DEFAULT_BODY,
        }
    }

    /// The style with bodies taking `fraction` of the distance between candles.
    #[must_use]
    pub fn with_body(mut self, fraction: f32) -> Self {
        self.body = usable_body(fraction);
        self
    }

    /// The color of a period that opened at `open` and closed at `close`.
    fn color(&self, open: f64, close: f64) -> Color {
        if close >= open {
            self.up
        } else {
            self.down
        }
    }
}

/// How bars are drawn: `BarStyle::new()`, [`with_body`](Self::with_body) for wider or
/// narrower ones.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct BarStyle {
    /// How much of the distance between two bars a bar takes (0.1 to 1).
    pub body: f32,
}

impl BarStyle {
    /// Bars taking 70% of the distance between bars.
    pub const fn new() -> Self {
        Self { body: DEFAULT_BODY }
    }

    /// The style with bars taking `fraction` of the distance between bars.
    #[must_use]
    pub fn with_body(mut self, fraction: f32) -> Self {
        self.body = usable_body(fraction);
        self
    }
}

impl Default for BarStyle {
    fn default() -> Self {
        Self::new()
    }
}

/// The share of the distance between bars or candles a body takes unless told otherwise.
const DEFAULT_BODY: f32 = 0.7;

/// `fraction` held to 0.1..=1; the default if it is not a number.
fn usable_body(fraction: f32) -> f32 {
    if fraction.is_finite() {
        fraction.clamp(0.1, 1.0)
    } else {
        DEFAULT_BODY
    }
}

/// Bodies narrower than this many pixels are drawn as one bar from low to high, without a
/// separate wick.
const MIN_BODY_WITH_WICK: f64 = 3.0;

/// Narrow candles that land on one pixel column, merged into one: the first open, the last
/// close, and the highest high and lowest low (as pixels).
#[derive(Debug, Clone, Copy)]
struct Period {
    left: f64,
    top: f64,
    bottom: f64,
    open: f64,
    close: f64,
}

/// Draws `candles` across `plot`, clipped to its area; `interval` is the distance between
/// two candles in `x` units (60 for one-minute candles on a time axis), which sets their
/// width.
///
/// Each candle is a body from open to close with a wick from low to high, in whole pixels;
/// candles narrower than three pixels are a single bar from low to high, and those that land
/// on the same pixel column are merged into one (first open, last close, highest high,
/// lowest low), so a frame costs at most two rectangles per candle and one per column
/// however many candles there are. Candles with a value that is not finite are skipped.
/// Allocation-free.
pub fn paint_candles<C: Candles + ?Sized>(
    canvas: &mut Canvas<'_>,
    plot: &Plot,
    candles: &C,
    interval: f64,
    style: CandleStyle,
) {
    let area = plot.area();
    if area.is_empty() {
        return;
    }
    let state = canvas.state();
    canvas.clip(area);
    let width = bar_width(plot, interval, style.body);
    let mut merged: Option<Period> = None;
    for index in with_neighbours(candles_within(candles, &plot.x()), candles.len()) {
        let candle = candles.candle(index);
        if !candle.is_finite() {
            continue;
        }
        let left = (plot.x_at(candles.x(index)) - width / 2.0).round();
        let high = plot.clamp_y(plot.y_at(candle.high));
        let low = plot.clamp_y(plot.y_at(candle.low));
        if width < MIN_BODY_WITH_WICK {
            match merged.as_mut() {
                Some(period) if period.left == left => {
                    period.top = period.top.min(high);
                    period.bottom = period.bottom.max(low);
                    period.close = candle.close;
                }
                _ => {
                    if let Some(period) = merged.take() {
                        fill_period(canvas, period, width, style);
                    }
                    merged = Some(Period {
                        left,
                        top: high,
                        bottom: low,
                        open: candle.open,
                        close: candle.close,
                    });
                }
            }
            continue;
        }
        let color = style.color(candle.open, candle.close);
        let wick = left + (width / 2.0).floor();
        fill_bar(canvas, wick, 1.0, high, low, color);
        let open = plot.clamp_y(plot.y_at(candle.open));
        let close = plot.clamp_y(plot.y_at(candle.close));
        fill_bar(canvas, left, width, open.min(close), open.max(close), color);
    }
    if let Some(period) = merged {
        fill_period(canvas, period, width, style);
    }
    canvas.restore(state);
}

/// Fills merged narrow candles as one bar, colored by the period's direction.
fn fill_period(canvas: &mut Canvas<'_>, period: Period, width: f64, style: CandleStyle) {
    let color = style.color(period.open, period.close);
    fill_bar(canvas, period.left, width, period.top, period.bottom, color);
}

/// Bars that land on one left edge, merged into one: their whole extent, in the color of
/// the tallest.
#[derive(Debug, Clone, Copy)]
struct Stack {
    left: f64,
    top: f64,
    bottom: f64,
    color: Color,
    height: f64,
}

/// Draws `bars` across `plot` as columns from zero to each `y`, clipped to its area;
/// `interval` is the distance between two bars in `x` units, and `color` gives each bar's
/// color by index (to color volume by its candle's direction, say).
///
/// Bars that land on the same left edge (more bars than pixels) are merged into one, in the
/// color of the tallest, so a frame costs at most one rectangle per bar and per pixel
/// column. Bars whose `y` is not finite are skipped. Allocation-free.
pub fn paint_bars<P: Points + ?Sized>(
    canvas: &mut Canvas<'_>,
    plot: &Plot,
    bars: &P,
    interval: f64,
    style: BarStyle,
    color: impl Fn(usize) -> Color,
) {
    let area = plot.area();
    if area.is_empty() {
        return;
    }
    let state = canvas.state();
    canvas.clip(area);
    let width = bar_width(plot, interval, style.body);
    let base = plot.clamp_y(plot.y_at(0.0));
    let mut merged: Option<Stack> = None;
    for index in with_neighbours(points_within(bars, &plot.x()), bars.len()) {
        let y = bars.y(index);
        if !y.is_finite() {
            continue;
        }
        let left = (plot.x_at(bars.x(index)) - width / 2.0).round();
        let end = plot.clamp_y(plot.y_at(y));
        let (top, bottom) = (end.min(base), end.max(base));
        match merged.as_mut() {
            Some(stack) if stack.left == left => {
                stack.top = stack.top.min(top);
                stack.bottom = stack.bottom.max(bottom);
                if bottom - top > stack.height {
                    stack.height = bottom - top;
                    stack.color = color(index);
                }
            }
            _ => {
                if let Some(stack) = merged.take() {
                    fill_stack(canvas, stack, width);
                }
                merged = Some(Stack {
                    left,
                    top,
                    bottom,
                    color: color(index),
                    height: bottom - top,
                });
            }
        }
    }
    if let Some(stack) = merged {
        fill_stack(canvas, stack, width);
    }
    canvas.restore(state);
}

/// Fills merged bars as one.
fn fill_stack(canvas: &mut Canvas<'_>, stack: Stack, width: f64) {
    fill_bar(
        canvas,
        stack.left,
        width,
        stack.top,
        stack.bottom,
        stack.color,
    );
}

/// Kept off a width before rounding it down, so 70% of 10 pixels (0.7 as an `f32` is a
/// little less) is 7.
const WIDTH_EPSILON: f64 = 1e-4;

/// Whole pixels a bar `fraction` of `interval` wide takes, at least one.
fn bar_width(plot: &Plot, interval: f64, fraction: f32) -> f64 {
    let spacing = plot.x_length(interval).abs();
    let width = (spacing * f64::from(usable_body(fraction)) + WIDTH_EPSILON).floor();
    if width.is_finite() {
        // Never wider than the plot: a huge interval would only make huge rects.
        width.clamp(1.0, f64::from(plot.area().width()).max(1.0))
    } else {
        1.0
    }
}

/// Fills `width` pixels from `left`, `top` to `bottom` rounded to whole pixels, at least
/// one pixel tall.
fn fill_bar(canvas: &mut Canvas<'_>, left: f64, width: f64, top: f64, bottom: f64, color: Color) {
    let top = top.round();
    let bottom = bottom.round().max(top + 1.0);
    // Lossless enough: pixels within a pixel of the plot.
    let rect = Rect::from_edges(
        left as f32,
        top as f32,
        (left + width) as f32,
        bottom as f32,
    );
    canvas.fill_rect(rect, color);
}
