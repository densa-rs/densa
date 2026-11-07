use std::ops::Range;

use densa_draw::Canvas;

use super::{Levels, Run, Visit, NONE};
use crate::line::{begin, end, Emit, Spans};
use crate::series::{points_within, with_neighbours};
use crate::{paint_line, y_extent, LineStyle, Plot, Points};

/// What a run of a line's points holds.
#[derive(Debug, Clone, Copy)]
struct Node {
    /// The first and last of its points with finite `x` and `y` (drawn), or [`NONE`].
    first: usize,
    last: usize,
    /// The lowest and highest finite `y` among them (whatever their `x`).
    low: f64,
    high: f64,
    /// Whether every point with a finite `y` has a finite `x`: then the node can stand for
    /// its points in a line (a point with an `x` that is not finite is a gap there, though
    /// its `y` counts in [`LineSummary::y_extent`], as in `y_extent`).
    whole: bool,
}

impl Run for Node {
    const EMPTY: Self = Self {
        first: NONE,
        last: NONE,
        low: f64::INFINITY,
        high: f64::NEG_INFINITY,
        whole: true,
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
            whole: self.whole && next.whole,
        }
    }
}

impl Node {
    fn of_points<P: Points + ?Sized>(points: &P, range: Range<usize>) -> Self {
        let mut node = Self::EMPTY;
        for index in range {
            let y = points.y(index);
            if !y.is_finite() {
                continue;
            }
            node.low = node.low.min(y);
            node.high = node.high.max(y);
            if !points.x(index).is_finite() {
                node.whole = false;
                continue;
            }
            if node.first == NONE {
                node.first = index;
            }
            node.last = index;
        }
        node
    }

    /// Its lowest and highest finite `y`, if it has any.
    fn extent(self) -> Option<(f64, f64)> {
        (self.low <= self.high).then_some((self.low, self.high))
    }
}

/// A line's points summarized, so that a frame of far more of them than the plot has
/// columns costs about the plot's width ([`paint_line_summarized`]) and the vertical extent of
/// any part of them a few steps ([`y_extent`](Self::y_extent)), not a visit to each point.
///
/// It holds, for runs of 32 points and runs of runs eight at a time, each run's first and
/// last point and its lowest and highest `y`: about 1.4 bytes a point, a tenth of what two
/// `f64` columns take. Build it once ([`of`](Self::of)) and keep it with the points, which
/// must not change but by points added at the end: [`update`](Self::update) takes those in, at
/// the cost of the new points (allocating as it grows).
///
/// A summary is checked against the points it is used with by their count and the `x` of the
/// first and last of them: points dropped from the front or replaced (a window that slides) are
/// seen, and [`update`](Self::update) summarizes them anew while drawing and extents do without
/// the summary meanwhile. A value changed between the ends is not seen, and a summary of
/// other values draws them wrong: [`rebuild`](Self::rebuild) it after such a change.
///
/// ```
/// use densa_chart::{paint_line_summarized, points_within, Axis, LineStyle, LineSummary, Plot, Rect};
/// use densa_draw::{Canvas, InstanceBuffer};
/// use densa_text::{AtlasConfig, GlyphCache};
///
/// // A million readings, kept with their summary.
/// let mut readings: Vec<(f64, f64)> = (0..1_000_000)
///     .map(|i| (f64::from(i), (f64::from(i) / 5_000.0).sin()))
///     .collect();
/// let mut summary = LineSummary::of(&readings[..]);
///
/// // A frame: scaled to what shows, drawn, both at about the cost of the plot's width.
/// let x = Axis::number(200_000.0, 900_000.0);
/// let (low, high) = summary.y_extent(&readings[..], points_within(&readings[..], &x)).unwrap();
/// let plot = Plot::new(Rect::new(0.0, 0.0, 1_000.0, 300.0), x, Axis::number(low, high));
/// let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
/// let mut out = InstanceBuffer::with_capacity(4_096);
/// let mut canvas = Canvas::begin(&mut cache, &mut out);
/// let style = LineStyle::new(densa_chart::Color::WHITE);
/// paint_line_summarized(&mut canvas, &plot, &readings[..], &summary, style);
///
/// // More readings arrive: the summary takes in the new ones.
/// readings.push((1_000_000.0, 0.5));
/// summary.update(&readings[..]);
/// assert_eq!(summary.len(), readings.len());
/// ```
#[derive(Debug, Clone, Default)]
pub struct LineSummary {
    levels: Levels<Node>,
}

impl LineSummary {
    /// A summary of no points.
    pub fn new() -> Self {
        Self::default()
    }

    /// A summary of `points`.
    pub fn of<P: Points + ?Sized>(points: &P) -> Self {
        let mut summary = Self::new();
        summary.update(points);
        summary
    }

    /// How many points it summarizes.
    pub fn len(&self) -> usize {
        self.levels.len
    }

    /// Whether it summarizes no points.
    pub fn is_empty(&self) -> bool {
        self.levels.len == 0
    }

    /// Takes in the points of `points` past those it summarizes, the earlier ones taken as
    /// they were. Fewer points than it summarizes, or a first point or a last summarized point
    /// at another `x` (a window that slid), are summarized anew.
    pub fn update<P: Points + ?Sized>(&mut self, points: &P) {
        self.levels.update(
            points.len(),
            |index| points.x(index),
            |run| Node::of_points(points, run),
        );
    }

    /// Summarizes `points` anew, keeping the memory it holds.
    pub fn rebuild<P: Points + ?Sized>(&mut self, points: &P) {
        self.levels.rebuild(
            points.len(),
            |index| points.x(index),
            |run| Node::of_points(points, run),
        );
    }

    /// Whether it summarizes `points`, as far as their count and the `x` of their ends tell.
    fn covers<P: Points + ?Sized>(&self, points: &P) -> bool {
        self.levels.covers(points.len(), |index| points.x(index))
    }

    /// The lowest and highest finite `y` among `range` of `points` (the part of it within
    /// them), or `None` if none is finite: what [`y_extent`](crate::y_extent) gives, in a few
    /// steps. A summary not of `points` (another count of them, or other ends) visits each one,
    /// as `y_extent` does; a value changed between the ends is not seen (see [`LineSummary`]).
    pub fn y_extent<P: Points + ?Sized>(
        &self,
        points: &P,
        range: Range<usize>,
    ) -> Option<(f64, f64)> {
        if !self.covers(points) {
            return y_extent(points, range);
        }
        self.levels
            .fold(range, |run| Node::of_points(points, run))
            .extent()
    }
}

/// A line pushed onto spans: the points of a run of nodes within one pixel column joined into
/// one run before they are pushed, so a column costs a few points however many nodes it holds.
struct Walk<'w, P: Points + ?Sized, E: Emit> {
    points: &'w P,
    plot: &'w Plot,
    spans: &'w mut Spans<E>,
    /// The run of nodes not pushed yet: its pixel column, what it holds and its points.
    pending: Option<(f64, Node, Range<usize>)>,
}

impl<P: Points + ?Sized, E: Emit> Visit<Node> for Walk<'_, P, E> {
    fn node(&mut self, node: Node, run: Range<usize>) -> bool {
        let Some(column) = self.column_of(node, run.start) else {
            return false;
        };
        match &mut self.pending {
            Some((at, held, held_run)) if *at == column && held_run.end == run.start => {
                *held = held.then(node);
                held_run.end = run.end;
            }
            _ => {
                self.flush();
                self.pending = Some((column, node, run));
            }
        }
        true
    }

    fn item(&mut self, at: usize) {
        self.flush();
        let (x, y) = (self.points.x(at), self.points.y(at));
        self.spans.push(self.plot.x_at(x), self.plot.y_at(y));
    }
}

impl<P: Points + ?Sized, E: Emit> Walk<'_, P, E> {
    /// The pixel column node `node` (its points from `start`) draws in, if it can stand for
    /// its points: its drawn points in one column, their extremes drawn at finite pixels, and
    /// no point with a finite `y` that is not drawn (an `x` that is not finite). A node of gaps
    /// only stands for them anywhere.
    fn column_of(&self, node: Node, start: usize) -> Option<f64> {
        let column = |at: usize| self.plot.x_at(self.points.x(at)).floor();
        if node.first == NONE {
            return node.whole.then(|| column(start));
        }
        let drawn = |y: f64| self.plot.y_at(y).is_finite();
        let fits = node.whole
            && drawn(node.low)
            && drawn(node.high)
            && column(node.first) == column(node.last);
        fits.then(|| column(node.first))
    }

    /// Pushes the run of nodes not pushed yet.
    fn flush(&mut self) {
        if let Some((_, node, run)) = self.pending.take() {
            push_node(self.points, self.plot, node, run, self.spans);
        }
    }
}

/// Pushes onto `spans` what the points `run` of `points` (in one pixel column, summarized by
/// `node`) would: its first drawn point, its lowest and highest `y` in that column, its last
/// drawn point, and the gaps before and after them, so the spans come out as they would of
/// every point.
fn push_node<P: Points + ?Sized, E: Emit>(
    points: &P,
    plot: &Plot,
    node: Node,
    run: Range<usize>,
    spans: &mut Spans<E>,
) {
    let x_at = |at: usize| plot.x_at(points.x(at));
    if node.first == NONE {
        spans.push(x_at(run.start), f64::NAN);
        return;
    }
    if node.first > run.start {
        spans.push(x_at(run.start), f64::NAN);
    }
    let x = x_at(node.first);
    spans.push(x, plot.y_at(points.y(node.first)));
    spans.push(x, plot.y_at(node.low));
    spans.push(x, plot.y_at(node.high));
    spans.push(x_at(node.last), plot.y_at(points.y(node.last)));
    if node.last + 1 < run.end {
        spans.push(x_at(run.end - 1), f64::NAN);
    }
}

/// Draws `points` as a line across `plot`, as [`paint_line`] does, pixel for pixel, at a cost
/// of about the plot's width however many points show: `summary` (of these `points`) stands
/// for each run of them within one pixel column. As for `paint_line`, the points come in
/// ascending `x` (out of order, the two may draw them differently). A summary not of `points`
/// (another count of them, or other ends: [`LineSummary::update`] it after they change) draws
/// as `paint_line` does, visiting each point; a value changed between the ends is not seen
/// (see [`LineSummary`]). Nothing is allocated.
pub fn paint_line_summarized<P: Points + ?Sized>(
    canvas: &mut Canvas<'_>,
    plot: &Plot,
    points: &P,
    summary: &LineSummary,
    style: LineStyle,
) {
    if !summary.covers(points) {
        paint_line(canvas, plot, points, style);
        return;
    }
    let Some(mut spans) = begin(canvas, plot, style) else {
        return;
    };
    let range = with_neighbours(points_within(points, &plot.x()), points.len());
    let mut walk = Walk {
        points,
        plot,
        spans: &mut spans,
        pending: None,
    };
    summary.levels.walk(&mut walk, &range);
    walk.flush();
    end(spans);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_keeps_its_first_and_last_drawn_points_and_its_extent() {
        let points: Vec<(f64, f64)> = [f64::NAN, 3.0, -1.0, 7.0, f64::NAN]
            .iter()
            .enumerate()
            .map(|(i, &y)| (i as f64, y))
            .collect();
        let node = Node::of_points(&points[..], 0..5);
        assert_eq!((node.first, node.last), (1, 3));
        assert_eq!(node.extent(), Some((-1.0, 7.0)));
        assert!(node.whole);
        let gaps = Node::of_points(&points[..], 4..5);
        assert_eq!((gaps.first, gaps.extent()), (NONE, None));
        // A finite y at an x that is not: counted in the extent, not drawn.
        let odd = [(f64::NEG_INFINITY, 2.0), (1.0, 4.0)];
        let node = Node::of_points(&odd[..], 0..2);
        assert_eq!(
            (node.first, node.extent(), node.whole),
            (1, Some((2.0, 4.0)), false)
        );
        assert!(LineSummary::new().is_empty());
    }
}
