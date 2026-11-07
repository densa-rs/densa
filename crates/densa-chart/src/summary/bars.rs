use std::ops::Range;

use densa_core::Color;
use densa_draw::Canvas;

use super::{Levels, Run, Visit, NONE};
use crate::bars::BarDraw;
use crate::series::{points_within, with_neighbours};
use crate::{paint_bars, y_extent, BarStyle, Plot, Points};

/// What a run of bars holds.
#[derive(Debug, Clone, Copy)]
struct Node {
    /// The first and last of its bars with a finite `y` (drawn), or [`NONE`].
    first: usize,
    last: usize,
    /// The lowest and highest finite `y` among them.
    low: f64,
    high: f64,
    /// The first of its bars whose `y` is farthest from zero, or [`NONE`].
    largest: usize,
}

impl Run for Node {
    const EMPTY: Self = Self {
        first: NONE,
        last: NONE,
        low: f64::INFINITY,
        high: f64::NEG_INFINITY,
        largest: NONE,
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
            // The earlier on a tie, as one bar after another would keep it.
            largest: if next.size() > self.size() {
                next.largest
            } else {
                self.largest
            },
        }
    }
}

impl Node {
    fn of_bars<P: Points + ?Sized>(bars: &P, range: Range<usize>) -> Self {
        let mut node = Self::EMPTY;
        for index in range {
            let y = bars.y(index);
            if !y.is_finite() {
                continue;
            }
            if y.abs() > node.size() {
                node.largest = index;
            }
            node.low = node.low.min(y);
            node.high = node.high.max(y);
            if node.first == NONE {
                node.first = index;
            }
            node.last = index;
        }
        node
    }

    /// How far its largest bar's `y` is from zero: its lowest or its highest; below any size
    /// if it has no bars.
    fn size(self) -> f64 {
        if self.first == NONE {
            f64::NEG_INFINITY
        } else {
            self.high.abs().max(self.low.abs())
        }
    }

    /// Its lowest and highest finite `y`, if it has any.
    fn extent(self) -> Option<(f64, f64)> {
        (self.first != NONE).then_some((self.low, self.high))
    }
}

/// A bar series summarized, so that a frame of far more bars than the plot has columns costs
/// about the plot's width ([`paint_bars_summarized`]) and the vertical extent of any part of
/// them a few steps ([`y_extent`](Self::y_extent)), not a visit to each bar.
///
/// It holds, for runs of 32 bars and runs of runs eight at a time, each run's first and last
/// bar, its lowest and highest `y` and its largest bar: about 1.4 bytes a bar. As for a
/// [`LineSummary`](crate::LineSummary), build it once ([`of`](Self::of)) and keep it with the
/// bars, which must not change but by bars added at the end ([`update`](Self::update) takes
/// those in). A summary is checked against the bars it is used with by their count and the
/// `x` of the first and last of them; a value changed between the ends is not seen:
/// [`rebuild`](Self::rebuild) it after such a change.
///
/// ```
/// use densa_chart::{paint_bars_summarized, points_within, Axis, BarStyle, BarSummary, Plot, Rect};
/// use densa_draw::{Canvas, InstanceBuffer};
/// use densa_text::{AtlasConfig, GlyphCache};
///
/// // Requests counted each second for twelve days, kept with their summary.
/// let counts: Vec<(f64, f64)> = (0..1_000_000)
///     .map(|s| (f64::from(s), f64::from(s % 97 + s % 13)))
///     .collect();
/// let summary = BarSummary::of(&counts[..]);
///
/// // A frame of all of them: scaled from zero, drawn at about the cost of the plot's width.
/// let x = Axis::number(0.0, 1_000_000.0);
/// let (_, high) = summary.y_extent(&counts[..], points_within(&counts[..], &x)).unwrap();
/// let plot = Plot::new(Rect::new(0.0, 0.0, 1_000.0, 200.0), x, Axis::number(0.0, high));
/// let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
/// let mut out = InstanceBuffer::with_capacity(4_096);
/// let mut canvas = Canvas::begin(&mut cache, &mut out);
/// let color = |_| densa_chart::Color::WHITE;
/// paint_bars_summarized(&mut canvas, &plot, &counts[..], &summary, 1.0, BarStyle::new(), color);
/// assert!(out.len() <= 1_000, "a bar per pixel column at most");
/// ```
#[derive(Debug, Clone, Default)]
pub struct BarSummary {
    levels: Levels<Node>,
}

impl BarSummary {
    /// A summary of no bars.
    pub fn new() -> Self {
        Self::default()
    }

    /// A summary of `bars`.
    pub fn of<P: Points + ?Sized>(bars: &P) -> Self {
        let mut summary = Self::new();
        summary.update(bars);
        summary
    }

    /// How many bars it summarizes.
    pub fn len(&self) -> usize {
        self.levels.len
    }

    /// Whether it summarizes no bars.
    pub fn is_empty(&self) -> bool {
        self.levels.len == 0
    }

    /// Takes in the bars of `bars` past those it summarizes, the earlier ones taken as they
    /// were. Fewer bars than it summarizes, or a first bar or a last summarized bar at another
    /// `x` (a window that slid), are summarized anew.
    pub fn update<P: Points + ?Sized>(&mut self, bars: &P) {
        self.levels.update(
            bars.len(),
            |index| bars.x(index),
            |run| Node::of_bars(bars, run),
        );
    }

    /// Summarizes `bars` anew, keeping the memory it holds.
    pub fn rebuild<P: Points + ?Sized>(&mut self, bars: &P) {
        self.levels.rebuild(
            bars.len(),
            |index| bars.x(index),
            |run| Node::of_bars(bars, run),
        );
    }

    /// Whether it summarizes `bars`, as far as their count and the `x` of their ends tell.
    fn covers<P: Points + ?Sized>(&self, bars: &P) -> bool {
        self.levels.covers(bars.len(), |index| bars.x(index))
    }

    /// The lowest and highest finite `y` among `range` of `bars` (the part of it within them),
    /// or `None` if none is finite: what [`y_extent`](crate::y_extent) gives, in a few steps
    /// (put zero in yourself, where bars start). A summary not of `bars` visits each one, as
    /// `y_extent` does.
    pub fn y_extent<P: Points + ?Sized>(
        &self,
        bars: &P,
        range: Range<usize>,
    ) -> Option<(f64, f64)> {
        if !self.covers(bars) {
            return y_extent(bars, range);
        }
        self.levels
            .fold(range, |run| Node::of_bars(bars, run))
            .extent()
    }
}

/// Bars given to a [`BarDraw`]: a node whose bars share one left edge as one stack.
struct Walk<'w, 'c, 'a, P: ?Sized, F> {
    bars: &'w P,
    draw: BarDraw<'c, 'a, F>,
}

impl<P: Points + ?Sized, F: Fn(usize) -> Color> Visit<Node> for Walk<'_, '_, '_, P, F> {
    fn node(&mut self, node: Node, _: Range<usize>) -> bool {
        if node.first == NONE {
            // Nothing to draw.
            return true;
        }
        let left = self.draw.left_of(self.bars.x(node.first));
        if self.draw.left_of(self.bars.x(node.last)) != left {
            return false;
        }
        self.draw
            .stack(left, node.low, node.high, node.largest, node.size());
        true
    }

    fn item(&mut self, index: usize) {
        self.draw.bar(index, self.bars.x(index), self.bars.y(index));
    }
}

/// Draws `bars` across `plot` as [`paint_bars`] does, pixel for pixel, at a cost of about the
/// plot's width however many bars show: `summary` (of these `bars`) stands for each run of them
/// that lands on one left edge. As for `paint_bars`, the bars come in ascending `x` (out of
/// order, the two may draw them differently). A summary not of `bars` (another count of them,
/// or other ends: [`BarSummary::update`] it after they change) draws as `paint_bars` does,
/// visiting each bar; a value changed between the ends is not seen (see [`BarSummary`]).
/// Nothing is allocated.
pub fn paint_bars_summarized<P: Points + ?Sized>(
    canvas: &mut Canvas<'_>,
    plot: &Plot,
    bars: &P,
    summary: &BarSummary,
    interval: f64,
    style: BarStyle,
    color: impl Fn(usize) -> Color,
) {
    if !summary.covers(bars) {
        paint_bars(canvas, plot, bars, interval, style, color);
        return;
    }
    let Some(draw) = BarDraw::begin(canvas, plot, interval, style, color) else {
        return;
    };
    let range = with_neighbours(points_within(bars, &plot.x()), bars.len());
    let mut walk = Walk { bars, draw };
    summary.levels.walk(&mut walk, &range);
    walk.draw.end();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_keeps_the_first_of_its_largest_bars() {
        let bars: Vec<(f64, f64)> = [f64::NAN, 3.0, -7.0, 7.0, -7.0, f64::INFINITY, 8.0]
            .iter()
            .enumerate()
            .map(|(i, &y)| (i as f64, y))
            .collect();
        let node = Node::of_bars(&bars[..], 0..6);
        assert_eq!((node.first, node.last, node.largest), (1, 4, 2));
        assert_eq!((node.size(), node.extent()), (7.0, Some((-7.0, 7.0))));
        // Joined, the earlier keeps a tie and the later wins only by being larger.
        let (left, right) = (
            Node::of_bars(&bars[..], 0..3),
            Node::of_bars(&bars[..], 3..6),
        );
        assert_eq!(left.then(right).largest, 2);
        let bigger = Node::of_bars(&bars[..], 5..7);
        assert_eq!(left.then(bigger).largest, 6);
        let none = Node::of_bars(&bars[..], 5..6);
        assert_eq!(
            (none.first, none.largest, none.extent()),
            (NONE, NONE, None)
        );
        assert_eq!(none.then(left).largest, 2);
        assert_eq!(left.then(none).largest, 2);
    }
}
