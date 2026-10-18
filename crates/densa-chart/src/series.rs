use std::ops::Range;

use crate::Axis;

/// A series of points, in ascending `x` (a chart finds the visible ones by binary search):
/// a line, or the bars of a bar series. A `y` that is not finite is a gap.
///
/// Points out of order do not break anything, but they draw wrong and cost more: each step
/// back restarts the line, and the step forward after it is drawn across every column it
/// spans.
pub trait Points {
    /// How many points there are.
    fn len(&self) -> usize;

    /// Whether there are none.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The `x` of point `index`: never below the one before, never NaN.
    fn x(&self, index: usize) -> f64;

    /// The `y` of point `index`.
    fn y(&self, index: usize) -> f64;
}

impl Points for [(f64, f64)] {
    fn len(&self) -> usize {
        <[_]>::len(self)
    }

    fn x(&self, index: usize) -> f64 {
        self[index].0
    }

    fn y(&self, index: usize) -> f64 {
        self[index].1
    }
}

/// `x` values and `y` values side by side; as long as the shorter.
impl Points for (&[f64], &[f64]) {
    fn len(&self) -> usize {
        self.0.len().min(self.1.len())
    }

    fn x(&self, index: usize) -> f64 {
        self.0[index]
    }

    fn y(&self, index: usize) -> f64 {
        self.1[index]
    }
}

/// One candlestick: a period's first, highest, lowest and last values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candle {
    /// The first value.
    pub open: f64,
    /// The highest value.
    pub high: f64,
    /// The lowest value.
    pub low: f64,
    /// The last value.
    pub close: f64,
}

impl Candle {
    /// A candle from its four values.
    pub const fn new(open: f64, high: f64, low: f64, close: f64) -> Self {
        Self {
            open,
            high,
            low,
            close,
        }
    }

    /// Whether every value is finite (others are not drawn).
    pub fn is_finite(&self) -> bool {
        self.open.is_finite()
            && self.high.is_finite()
            && self.low.is_finite()
            && self.close.is_finite()
    }
}

/// A series of candlesticks in ascending `x` (each candle's period, by its start or its
/// middle: the chart centers a candle on its `x`).
pub trait Candles {
    /// How many candles there are.
    fn len(&self) -> usize;

    /// Whether there are none.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The `x` of candle `index`: never below the one before, never NaN.
    fn x(&self, index: usize) -> f64;

    /// Candle `index`.
    fn candle(&self, index: usize) -> Candle;
}

impl Candles for [(f64, Candle)] {
    fn len(&self) -> usize {
        <[_]>::len(self)
    }

    fn x(&self, index: usize) -> f64 {
        self[index].0
    }

    fn candle(&self, index: usize) -> Candle {
        self[index].1
    }
}

/// The indices whose `x` lies within `x` (ends included), found by binary search.
pub fn points_within<P: Points + ?Sized>(points: &P, x: &Axis) -> Range<usize> {
    index_range(points.len(), |index| points.x(index), x)
}

/// The indices of the candles whose `x` lies within `x` (ends included).
pub fn candles_within<C: Candles + ?Sized>(candles: &C, x: &Axis) -> Range<usize> {
    index_range(candles.len(), |index| candles.x(index), x)
}

/// The lowest and highest finite `y` among `range` of `points` (the part of it within
/// them), or `None` if none is finite.
pub fn y_extent<P: Points + ?Sized>(points: &P, range: Range<usize>) -> Option<(f64, f64)> {
    let range = range.start.min(points.len())..range.end.min(points.len());
    extent(range.map(|index| points.y(index)).map(|y| (y, y)))
}

/// The lowest low and the highest high among `range` of `candles` (the part of it within
/// them; finite candles only), or `None` if there are none.
pub fn candle_extent<C: Candles + ?Sized>(candles: &C, range: Range<usize>) -> Option<(f64, f64)> {
    let range = range.start.min(candles.len())..range.end.min(candles.len());
    extent(
        range
            .map(|index| candles.candle(index))
            .filter(Candle::is_finite)
            .map(|candle| (candle.low, candle.high)),
    )
}

fn extent(values: impl Iterator<Item = (f64, f64)>) -> Option<(f64, f64)> {
    values
        .filter(|(low, high)| low.is_finite() && high.is_finite())
        .fold(None, |seen, (low, high)| match seen {
            None => Some((low, high)),
            Some((min, max)) => Some((low.min(min), high.max(max))),
        })
}

/// The indices `0..len` whose `x` is within `axis`, for ascending `x`.
fn index_range(len: usize, x: impl Fn(usize) -> f64, axis: &Axis) -> Range<usize> {
    let start = partition_point(len, |index| x(index) < axis.start());
    let end = partition_point(len, |index| x(index) <= axis.end());
    start..end.max(start)
}

/// The first index of `0..len` for which `before` is false (`before` holds for a prefix).
fn partition_point(len: usize, before: impl Fn(usize) -> bool) -> usize {
    let (mut low, mut high) = (0, len);
    while low < high {
        let middle = low + (high - low) / 2;
        if before(middle) {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}

/// `range` grown by one index each way, within `0..len`: the neighbours just outside a
/// plot, so lines run on to its edges.
pub(crate) fn with_neighbours(range: Range<usize>, len: usize) -> Range<usize> {
    range.start.saturating_sub(1)..range.end.saturating_add(1).min(len)
}
