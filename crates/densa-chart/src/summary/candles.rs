use std::ops::Range;

use densa_draw::Canvas;

use super::{Levels, Run, Visit, NONE};
use crate::bars::CandleDraw;
use crate::series::{candles_within, with_neighbours};
use crate::{candle_extent, paint_candles, Candle, CandleStyle, Candles, Plot};

/// What a run of candles holds.
#[derive(Debug, Clone, Copy)]
struct Node {
    /// The first and last of its candles whose values are all finite (drawn), or [`NONE`].
    first: usize,
    last: usize,
    /// The lowest low and the highest high among them.
    low: f64,
    high: f64,
}

impl Run for Node {
    const EMPTY: Self = Self {
        first: NONE,
        last: NONE,
        low: f64::INFINITY,
        high: f64::NEG_INFINITY,
    };

    fn then(self, next: Self) -> Self {
        Self {
            first: if self.first == NONE {
                next.first
            } else {
                self.first
            },
            last: if next.last == NONE {
                self.last
            } else {
                next.last
            },
            low: self.low.min(next.low),
            high: self.high.max(next.high),
        }
    }
}

impl Node {
    fn of_candles<C: Candles + ?Sized>(candles: &C, range: Range<usize>) -> Self {
        let mut node = Self::EMPTY;
        for index in range {
            let candle = candles.candle(index);
            if !candle.is_finite() {
                continue;
            }
            node.low = node.low.min(candle.low);
            node.high = node.high.max(candle.high);
            if node.first == NONE {
                node.first = index;
            }
            node.last = index;
        }
        node
    }

    /// Its lowest low and highest high, if it has candles.
    fn extent(self) -> Option<(f64, f64)> {
        (self.first != NONE).then_some((self.low, self.high))
    }
}

/// Candlesticks summarized, so that a frame of far more of them than the plot has columns
/// costs about the plot's width ([`paint_candles_summarized`]) and the vertical extent of any
/// part of them a few steps ([`y_extent`](Self::y_extent)), not a visit to each candle.
///
/// It holds, for runs of 32 candles and runs of runs eight at a time, each run's first and last
/// candle and its lowest low and highest high: about 1.1 bytes a candle. As for a
/// [`LineSummary`](crate::LineSummary), build it once ([`of`](Self::of)) and keep it with the
/// candles, which must not change but by candles added at the end ([`update`](Self::update)
/// takes those in; a candle still forming changes the last one: update it after the next one
/// starts, or [`rebuild`](Self::rebuild)). A summary is checked against the candles it is used
/// with by their count and the `x` of the first and last of them; a value changed between the
/// ends is not seen.
///
/// ```
/// use densa_chart::{candles_within, paint_candles_summarized, Axis, Candle, CandleStyle};
/// use densa_chart::{CandleSummary, Color, Plot, Rect};
/// use densa_draw::{Canvas, InstanceBuffer};
/// use densa_text::{AtlasConfig, GlyphCache};
///
/// // Two years of one-minute candles, kept with their summary.
/// let candles: Vec<(f64, Candle)> = (0..1_000_000)
///     .map(|m| {
///         let open = 100.0 + (f64::from(m) / 9_000.0).sin() * 20.0;
///         (f64::from(m) * 60.0, Candle::new(open, open + 0.4, open - 0.3, open + 0.1))
///     })
///     .collect();
/// let summary = CandleSummary::of(&candles[..]);
///
/// // A frame of all of them, scaled to what shows: about the cost of the plot's width.
/// let x = Axis::time(0.0, 60_000_000.0);
/// let (low, high) = summary.y_extent(&candles[..], candles_within(&candles[..], &x)).unwrap();
/// let plot = Plot::new(Rect::new(0.0, 0.0, 1_000.0, 300.0), x, Axis::number(low, high));
/// let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
/// let mut out = InstanceBuffer::with_capacity(4_096);
/// let mut canvas = Canvas::begin(&mut cache, &mut out);
/// let style = CandleStyle::new(Color::srgb(0x2e7d32), Color::srgb(0xc62828));
/// paint_candles_summarized(&mut canvas, &plot, &candles[..], &summary, 60.0, style);
/// assert!(out.len() <= 1_000, "a bar per pixel column at most");
/// ```
#[derive(Debug, Clone, Default)]
pub struct CandleSummary {
    levels: Levels<Node>,
}

impl CandleSummary {
    /// A summary of no candles.
    pub fn new() -> Self {
        Self::default()
    }

    /// A summary of `candles`.
    pub fn of<C: Candles + ?Sized>(candles: &C) -> Self {
        let mut summary = Self::new();
        summary.update(candles);
        summary
    }

    /// How many candles it summarizes.
    pub fn len(&self) -> usize {
        self.levels.len
    }

    /// Whether it summarizes no candles.
    pub fn is_empty(&self) -> bool {
        self.levels.len == 0
    }

    /// Takes in the candles of `candles` past those it summarizes, the earlier ones taken as
    /// they were. Fewer candles than it summarizes, or a first candle or a last summarized
    /// candle at another `x` (a window that slid), are summarized anew.
    pub fn update<C: Candles + ?Sized>(&mut self, candles: &C) {
        self.levels.update(
            candles.len(),
            |index| candles.x(index),
            |run| Node::of_candles(candles, run),
        );
    }

    /// Summarizes `candles` anew, keeping the memory it holds.
    pub fn rebuild<C: Candles + ?Sized>(&mut self, candles: &C) {
        self.levels.rebuild(
            candles.len(),
            |index| candles.x(index),
            |run| Node::of_candles(candles, run),
        );
    }

    /// Whether it summarizes `candles`, as far as their count and the `x` of their ends tell.
    fn covers<C: Candles + ?Sized>(&self, candles: &C) -> bool {
        self.levels.covers(candles.len(), |index| candles.x(index))
    }

    /// The lowest low and the highest high among `range` of `candles` (the part of it within
    /// them; finite candles only), or `None` if there are none: what
    /// [`candle_extent`](crate::candle_extent) gives, in a few steps. A summary not of
    /// `candles` visits each one, as `candle_extent` does.
    pub fn y_extent<C: Candles + ?Sized>(
        &self,
        candles: &C,
        range: Range<usize>,
    ) -> Option<(f64, f64)> {
        if !self.covers(candles) {
            return candle_extent(candles, range);
        }
        self.levels
            .fold(range, |run| Node::of_candles(candles, run))
            .extent()
    }
}

/// Candles given to a [`CandleDraw`]: when they are narrow, a node whose candles share one
/// left edge as one candle.
struct Walk<'w, 'c, 'a, C: ?Sized> {
    candles: &'w C,
    draw: CandleDraw<'c, 'a>,
}

impl<C: Candles + ?Sized> Visit<Node> for Walk<'_, '_, '_, C> {
    fn node(&mut self, node: Node, _: Range<usize>) -> bool {
        if node.first == NONE {
            // Nothing to draw.
            return true;
        }
        // Candles drawn whole are few enough to draw one by one.
        if !self.draw.is_narrow() {
            return false;
        }
        let left = self.draw.left_of(self.candles.x(node.first));
        if self.draw.left_of(self.candles.x(node.last)) != left {
            return false;
        }
        let (first, last) = (
            self.candles.candle(node.first),
            self.candles.candle(node.last),
        );
        let period = Candle::new(first.open, node.high, node.low, last.close);
        self.draw.narrow(left, period);
        true
    }

    fn item(&mut self, index: usize) {
        self.draw
            .candle(self.candles.x(index), self.candles.candle(index));
    }
}

/// Draws `candles` across `plot` as [`paint_candles`] does, pixel for pixel, at a cost of about
/// the plot's width however many candles show: `summary` (of these `candles`) stands for each
/// run of narrow candles that lands on one left edge. Candles wide enough to draw whole (three
/// pixels or more) are drawn one by one, as by `paint_candles`: few of them fit when `interval`
/// is their true distance apart, but an `interval` far wider than that draws every candle that
/// shows, each whole. As for `paint_candles`, the candles come in ascending
/// `x` (out of order, the two may draw them differently). A summary not of `candles` (another
/// count of them, or other ends: [`CandleSummary::update`] it after they change) draws as
/// `paint_candles` does, visiting each candle; a value changed between the ends is not seen
/// (see [`CandleSummary`]). Nothing is allocated.
pub fn paint_candles_summarized<C: Candles + ?Sized>(
    canvas: &mut Canvas<'_>,
    plot: &Plot,
    candles: &C,
    summary: &CandleSummary,
    interval: f64,
    style: CandleStyle,
) {
    if !summary.covers(candles) {
        paint_candles(canvas, plot, candles, interval, style);
        return;
    }
    let Some(draw) = CandleDraw::begin(canvas, plot, interval, style) else {
        return;
    };
    let range = with_neighbours(candles_within(candles, &plot.x()), candles.len());
    let mut walk = Walk { candles, draw };
    summary.levels.walk(&mut walk, &range);
    walk.draw.end();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_keeps_its_first_and_last_finite_candles_and_their_extremes() {
        let candles = [
            (0.0, Candle::new(1.0, f64::NAN, 0.0, 1.0)),
            (1.0, Candle::new(5.0, 6.0, 4.0, 5.5)),
            (2.0, Candle::new(5.5, 9.0, 5.0, 8.0)),
            // A candle whose low is above its high counts as given.
            (3.0, Candle::new(8.0, 2.0, 3.0, 2.5)),
            (4.0, Candle::new(f64::INFINITY, 1.0, 1.0, 1.0)),
        ];
        let node = Node::of_candles(&candles[..], 0..5);
        assert_eq!((node.first, node.last), (1, 3));
        assert_eq!(node.extent(), Some((3.0, 9.0)));
        assert_eq!(
            Node::of_candles(&candles[..], 3..4).extent(),
            candle_extent(&candles[..], 3..4)
        );
        let none = Node::of_candles(&candles[..], 4..5);
        assert_eq!((none.first, none.extent()), (NONE, None));
        assert!(CandleSummary::new().is_empty());
    }
}
