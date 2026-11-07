use densa_core::{Color, Rect};
use densa_draw::{Canvas, CanvasState};

use crate::series::{candles_within, points_within, with_neighbours};
use crate::{Candle, Candles, Plot, Points};

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

/// Draws `candles` across `plot`, clipped to its area; `interval` is the distance between
/// two candles in `x` units (60 for one-minute candles on a time axis), which sets their
/// width.
///
/// Each candle is a body from open to close with a wick from low to high, in whole pixels;
/// candles narrower than three pixels are a single bar from low to high, and those that land
/// on the same pixel column are merged into one (first open, last close, highest high,
/// lowest low), so a frame costs at most two rectangles per candle and one per column
/// however many candles there are. Candles with a value that is not finite are skipped.
/// The work is proportional to the candles that show; for millions of them,
/// [`paint_candles_summarized`] draws the same at a cost of about the plot's width.
/// Allocation-free.
///
/// [`paint_candles_summarized`]: crate::paint_candles_summarized
pub fn paint_candles<C: Candles + ?Sized>(
    canvas: &mut Canvas<'_>,
    plot: &Plot,
    candles: &C,
    interval: f64,
    style: CandleStyle,
) {
    let Some(mut draw) = CandleDraw::begin(canvas, plot, interval, style) else {
        return;
    };
    for index in with_neighbours(candles_within(candles, &plot.x()), candles.len()) {
        draw.candle(candles.x(index), candles.candle(index));
    }
    draw.end();
}

/// Narrow candles that land on one left edge, merged into one: the first open, the last
/// close, and the highest high and lowest low (as pixels).
#[derive(Debug, Clone, Copy)]
struct Period {
    left: f64,
    top: f64,
    bottom: f64,
    open: f64,
    close: f64,
}

/// Candles drawn onto a plot, clipped to its area: give them in ascending `x`, then
/// [`end`](Self::end). Narrow ones are merged per left edge as they come.
pub(crate) struct CandleDraw<'c, 'a> {
    canvas: &'c mut Canvas<'a>,
    /// The canvas's clip before the candles.
    state: CanvasState,
    plot: Plot,
    width: f64,
    style: CandleStyle,
    /// The narrow candles at the last left edge, not drawn yet.
    merged: Option<Period>,
}

impl<'c, 'a> CandleDraw<'c, 'a> {
    /// Candles `interval` apart drawn onto `plot` in `style`, or `None` if its area is empty.
    pub(crate) fn begin(
        canvas: &'c mut Canvas<'a>,
        plot: &Plot,
        interval: f64,
        style: CandleStyle,
    ) -> Option<Self> {
        let area = plot.area();
        if area.is_empty() {
            return None;
        }
        let state = canvas.state();
        canvas.clip(area);
        Some(Self {
            canvas,
            state,
            plot: *plot,
            width: bar_width(plot, interval, style.body),
            style,
            merged: None,
        })
    }

    /// Whether candles are drawn as one bar each, merged per left edge.
    pub(crate) fn is_narrow(&self) -> bool {
        self.width < MIN_BODY_WITH_WICK
    }

    /// The left edge of a candle at `x`.
    pub(crate) fn left_of(&self, x: f64) -> f64 {
        left_of(&self.plot, x, self.width)
    }

    /// A candle at `x` (skipped unless finite).
    pub(crate) fn candle(&mut self, x: f64, candle: Candle) {
        if !candle.is_finite() {
            return;
        }
        let left = self.left_of(x);
        if self.is_narrow() {
            self.narrow(left, candle);
            return;
        }
        let color = self.style.color(candle.open, candle.close);
        let wick = left + (self.width / 2.0).floor();
        let (high, low) = (self.y(candle.high), self.y(candle.low));
        fill_bar(self.canvas, wick, 1.0, high, low, color);
        let (open, close) = (self.y(candle.open), self.y(candle.close));
        fill_bar(
            self.canvas,
            left,
            self.width,
            open.min(close),
            open.max(close),
            color,
        );
    }

    /// Narrow candles at `left`, as one: `period` opens as the first of them, closes as the
    /// last, and reaches the highest high and the lowest low among them. Merged with those
    /// at that edge before it.
    pub(crate) fn narrow(&mut self, left: f64, period: Candle) {
        let (top, bottom) = (self.y(period.high), self.y(period.low));
        match self.merged.as_mut() {
            // A left edge that is not a number (an `x` that is not) never merges, here and
            // in the summaries, which compare edges the same way.
            Some(merged) if merged.left == left => {
                merged.top = merged.top.min(top);
                merged.bottom = merged.bottom.max(bottom);
                merged.close = period.close;
            }
            _ => {
                self.flush();
                self.merged = Some(Period {
                    left,
                    top,
                    bottom,
                    open: period.open,
                    close: period.close,
                });
            }
        }
    }

    /// The pixel of `value`, held within a pixel of the plot.
    fn y(&self, value: f64) -> f64 {
        self.plot.clamp_y(self.plot.y_at(value))
    }

    /// Draws the merged narrow candles not drawn yet, as one bar colored by their direction.
    fn flush(&mut self) {
        if let Some(period) = self.merged.take() {
            let color = self.style.color(period.open, period.close);
            let (left, width) = (period.left, self.width);
            fill_bar(self.canvas, left, width, period.top, period.bottom, color);
        }
    }

    /// Draws what is left and gives the canvas its clip back.
    pub(crate) fn end(mut self) {
        self.flush();
        self.canvas.restore(self.state);
    }
}

/// Draws `bars` across `plot` as columns from zero to each `y`, clipped to its area;
/// `interval` is the distance between two bars in `x` units, and `color` gives each bar's
/// color by index (to color volume by its candle's direction, say).
///
/// Bars that land on the same left edge (more bars than pixels) are merged into one, in the
/// color of the largest (the `y` farthest from zero; the first of those as large), so a frame
/// costs at most one rectangle per bar and per pixel column. Bars whose `y` is not finite are
/// skipped. Bars start at zero: give `plot` a vertical axis that holds it (bars on the far side
/// of a zero out of view are drawn with no height, though the largest of a column still gives
/// it its color). The work is proportional to the bars that show; for millions of them,
/// [`paint_bars_summarized`] draws the same at a cost of about the plot's width.
/// Allocation-free.
///
/// [`paint_bars_summarized`]: crate::paint_bars_summarized
pub fn paint_bars<P: Points + ?Sized>(
    canvas: &mut Canvas<'_>,
    plot: &Plot,
    bars: &P,
    interval: f64,
    style: BarStyle,
    color: impl Fn(usize) -> Color,
) {
    let Some(mut draw) = BarDraw::begin(canvas, plot, interval, style, color) else {
        return;
    };
    for index in with_neighbours(points_within(bars, &plot.x()), bars.len()) {
        draw.bar(index, bars.x(index), bars.y(index));
    }
    draw.end();
}

/// Bars that land on one left edge, merged into one: their whole extent (as pixels), in the
/// color of the largest.
#[derive(Debug, Clone, Copy)]
struct Stack {
    left: f64,
    top: f64,
    bottom: f64,
    /// The largest bar's index, and how far its `y` is from zero.
    largest: usize,
    size: f64,
}

/// Bars drawn onto a plot, clipped to its area: give them in ascending `x`, then
/// [`end`](Self::end). Those at one left edge are merged as they come.
pub(crate) struct BarDraw<'c, 'a, F> {
    canvas: &'c mut Canvas<'a>,
    /// The canvas's clip before the bars.
    state: CanvasState,
    plot: Plot,
    width: f64,
    /// The pixel of zero, where bars start.
    base: f64,
    color: F,
    /// The bars at the last left edge, not drawn yet.
    merged: Option<Stack>,
}

impl<'c, 'a, F: Fn(usize) -> Color> BarDraw<'c, 'a, F> {
    /// Bars `interval` apart drawn onto `plot` in `style`, each in its `color`, or `None` if
    /// its area is empty.
    pub(crate) fn begin(
        canvas: &'c mut Canvas<'a>,
        plot: &Plot,
        interval: f64,
        style: BarStyle,
        color: F,
    ) -> Option<Self> {
        let area = plot.area();
        if area.is_empty() {
            return None;
        }
        let state = canvas.state();
        canvas.clip(area);
        Some(Self {
            canvas,
            state,
            plot: *plot,
            width: bar_width(plot, interval, style.body),
            base: plot.clamp_y(plot.y_at(0.0)),
            color,
            merged: None,
        })
    }

    /// The left edge of a bar at `x`.
    pub(crate) fn left_of(&self, x: f64) -> f64 {
        left_of(&self.plot, x, self.width)
    }

    /// Bar `index`, at `x` up (or down) to `y` (skipped unless `y` is finite).
    pub(crate) fn bar(&mut self, index: usize, x: f64, y: f64) {
        if y.is_finite() {
            self.stack(self.left_of(x), y, y, index, y.abs());
        }
    }

    /// Bars at `left`, as one: from zero to their lowest `y`, `low`, and their highest, `high`,
    /// the largest of them bar `largest`, `size` from zero. Merged with those at that edge
    /// before them.
    pub(crate) fn stack(&mut self, left: f64, low: f64, high: f64, largest: usize, size: f64) {
        let top = self.y(high).min(self.base);
        let bottom = self.y(low).max(self.base);
        match self.merged.as_mut() {
            Some(stack) if stack.left == left => {
                stack.top = stack.top.min(top);
                stack.bottom = stack.bottom.max(bottom);
                if size > stack.size {
                    stack.largest = largest;
                    stack.size = size;
                }
            }
            _ => {
                self.flush();
                self.merged = Some(Stack {
                    left,
                    top,
                    bottom,
                    largest,
                    size,
                });
            }
        }
    }

    /// The pixel of `value`, held within a pixel of the plot.
    fn y(&self, value: f64) -> f64 {
        self.plot.clamp_y(self.plot.y_at(value))
    }

    /// Draws the merged bars not drawn yet, as one.
    fn flush(&mut self) {
        if let Some(stack) = self.merged.take() {
            let color = (self.color)(stack.largest);
            let (left, width) = (stack.left, self.width);
            fill_bar(self.canvas, left, width, stack.top, stack.bottom, color);
        }
    }

    /// Draws what is left and gives the canvas its clip back.
    pub(crate) fn end(mut self) {
        self.flush();
        self.canvas.restore(self.state);
    }
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

/// The left edge of a bar `width` pixels wide centred on `x`, in whole pixels.
fn left_of(plot: &Plot, x: f64, width: f64) -> f64 {
    (plot.x_at(x) - width / 2.0).round()
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
