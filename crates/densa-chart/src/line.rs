use std::ops::Range;

use densa_core::{Color, Rect};
use densa_draw::{Canvas, CanvasState};

use crate::series::{points_within, with_neighbours};
use crate::{Plot, Points};

/// How a line is drawn: `LineStyle::new(color)`, [`with_width`](Self::with_width) for a
/// thicker one.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct LineStyle {
    /// The line's color.
    pub color: Color,
    /// Its thickness in pixels, measured vertically.
    pub width: f32,
}

impl LineStyle {
    /// A line one pixel thick in `color`.
    pub const fn new(color: Color) -> Self {
        Self { color, width: 1.0 }
    }

    /// The style `width` pixels thick (at least a tenth of a pixel).
    #[must_use]
    pub fn with_width(mut self, width: f32) -> Self {
        self.width = usable_width(width);
        self
    }
}

/// `width` held to at least a tenth of a pixel; one pixel if it is not a number.
fn usable_width(width: f32) -> f32 {
    if width.is_finite() {
        width.max(0.1)
    } else {
        1.0
    }
}

/// Draws `points` as a line across `plot`, clipped to its area.
///
/// The line is drawn one pixel column at a time: each column gets one vertical span from
/// the lowest to the highest point the line reaches in it, the ends shaded by how much of
/// their pixel they cover. Far more points than columns cost one span per column (the
/// columns still show every high and low, as M4 aggregation does), and far fewer still
/// draw a joined line; each column gets at most one span, gaps or not, so a frame costs at
/// most three rectangles per column. A gap (a `y` that is not finite) breaks the line. The
/// work is proportional to the points within the plot plus the width that shows; nothing is
/// allocated. For lines of many millions of points, [`paint_line_summarized`] draws the same
/// at a cost of about the width alone.
///
/// [`paint_line_summarized`]: crate::paint_line_summarized
pub fn paint_line<P: Points + ?Sized>(
    canvas: &mut Canvas<'_>,
    plot: &Plot,
    points: &P,
    style: LineStyle,
) {
    let Some(mut spans) = begin(canvas, plot, style) else {
        return;
    };
    let range = with_neighbours(points_within(points, &plot.x()), points.len());
    for index in range {
        spans.push(plot.x_at(points.x(index)), plot.y_at(points.y(index)));
    }
    end(spans);
}

/// The spans of a line drawn onto `plot`, clipped to its area, or `None` if it has none: push
/// its points, then [`end`] it.
pub(crate) fn begin<'c, 'a>(
    canvas: &'c mut Canvas<'a>,
    plot: &Plot,
    style: LineStyle,
) -> Option<Spans<Painter<'c, 'a>>> {
    let area = plot.area();
    if area.is_empty() {
        return None;
    }
    let state = canvas.state();
    canvas.clip(area);
    // Only the columns that show: a plot wider than its view costs the view's width.
    let shown = canvas.clip_rect();
    // Lossless enough: columns are whole pixels of a canvas.
    let columns = shown.x0.floor() as i64..shown.x1.ceil() as i64;
    let painter = Painter {
        canvas,
        state,
        plot: *plot,
        half: f64::from(usable_width(style.width)) / 2.0,
        color: style.color,
    };
    Some(Spans::new(columns, painter))
}

/// Draws the last span of a line [`begin`] began, and gives the canvas its clip back.
pub(crate) fn end(spans: Spans<Painter<'_, '_>>) {
    let painter = spans.finish();
    painter.canvas.restore(painter.state);
}

/// Draws the spans of a line onto a canvas.
pub(crate) struct Painter<'c, 'a> {
    canvas: &'c mut Canvas<'a>,
    /// The canvas's clip before the line.
    state: CanvasState,
    plot: Plot,
    /// Half the line's thickness, in pixels.
    half: f64,
    color: Color,
}

impl Emit for Painter<'_, '_> {
    fn emit(&mut self, column: i64, low: f64, high: f64) {
        let top = self.plot.clamp_y(low - self.half);
        let bottom = self.plot.clamp_y(high + self.half);
        // Lossless enough: a pixel column of a canvas.
        fill_span(self.canvas, column as f32, top, bottom, self.color);
    }
}

/// Fills column `x` from `top` to `bottom` (pixels): whole pixels solid, the partly covered
/// pixel at each end with its coverage as alpha.
fn fill_span(canvas: &mut Canvas<'_>, x: f32, top: f64, bottom: f64, color: Color) {
    let pixel = |canvas: &mut Canvas<'_>, y: f64, coverage: f64| {
        // Lossless enough: pixels and a coverage within 0..=1.
        let alpha = color.a * coverage as f32;
        canvas.fill_rect(Rect::new(x, y as f32, 1.0, 1.0), color.with_alpha(alpha));
    };
    let (inner_top, inner_bottom) = (top.ceil(), bottom.floor());
    if inner_top > inner_bottom {
        // Within one pixel row.
        pixel(canvas, top.floor(), bottom - top);
        return;
    }
    if inner_top > top {
        pixel(canvas, top.floor(), inner_top - top);
    }
    if inner_bottom > inner_top {
        // Lossless enough: pixels.
        let rect = Rect::from_edges(x, inner_top as f32, x + 1.0, inner_bottom as f32);
        canvas.fill_rect(rect, color);
    }
    if bottom > inner_bottom {
        pixel(canvas, inner_bottom, bottom - inner_bottom);
    }
}

/// The part of a line in one pixel column: the lowest and highest `y` it reaches there.
#[derive(Debug, Clone, Copy)]
struct Span {
    column: i64,
    low: f64,
    high: f64,
}

impl Span {
    fn new(column: i64, a: f64, b: f64) -> Self {
        Self {
            column,
            low: a.min(b),
            high: a.max(b),
        }
    }

    fn reach(&mut self, y: f64) {
        self.low = self.low.min(y);
        self.high = self.high.max(y);
    }
}

/// Where the spans of a line go: `emit(column, low, high)` for each pixel column it passes.
pub(crate) trait Emit {
    fn emit(&mut self, column: i64, low: f64, high: f64);
}

impl<F: FnMut(i64, f64, f64)> Emit for F {
    fn emit(&mut self, column: i64, low: f64, high: f64) {
        self(column, low, high);
    }
}

/// The spans of the polyline through the points pushed (canvas pixels, ascending `x`), each
/// pixel column in `columns` it passes given to `emit` left to right: the `y` range the line
/// covers within that column; for ascending `x`, at most one span per column. A point that is
/// not finite breaks the line; a lone point is a span of no height.
pub(crate) struct Spans<E: Emit> {
    columns: Range<i64>,
    emit: E,
    /// The span of the column the line is in, not given yet.
    open: Option<Span>,
    /// The last point, unless a gap came after it.
    last: Option<(f64, f64)>,
}

impl<E: Emit> Spans<E> {
    pub(crate) fn new(columns: Range<i64>, emit: E) -> Self {
        Self {
            columns,
            emit,
            open: None,
            last: None,
        }
    }

    /// The line on to point (`x`, `y`).
    pub(crate) fn push(&mut self, x: f64, y: f64) {
        if !(x.is_finite() && y.is_finite()) {
            // A gap: nothing joins across it, but its column's span stays open, so a column
            // still gets one span however many gaps fall in it.
            self.last = None;
            return;
        }
        // Lossless enough: pixel columns, saturating far off the canvas.
        let column = x.floor() as i64;
        match (self.last, self.open.as_mut()) {
            (_, Some(span)) if column == span.column => span.reach(y),
            (Some((x0, y0)), Some(span)) if column > span.column => {
                let slope = (y - y0) / (x - x0);
                let y_at = |at: f64| y0 + (at - x0) * slope;
                // Lossless enough: column edges.
                let edge = |column: i64| y_at(column as f64);
                span.reach(edge(span.column + 1));
                let first = span.column + 1;
                let done = self.open.take();
                self.flush(done);
                for between in first.max(self.columns.start)..column.min(self.columns.end) {
                    self.flush(Some(Span::new(between, edge(between), edge(between + 1))));
                }
                self.open = Some(Span::new(column, edge(column), y));
            }
            _ => {
                // The first point, one after a gap, or one left of the last (x out of
                // order): start afresh.
                let done = self.open.take();
                self.flush(done);
                self.open = Some(Span::new(column, y, y));
            }
        }
        self.last = Some((x, y));
    }

    /// Gives the last span; what it gave the spans to.
    pub(crate) fn finish(mut self) -> E {
        let done = self.open.take();
        self.flush(done);
        self.emit
    }

    fn flush(&mut self, span: Option<Span>) {
        if let Some(span) = span.filter(|span| self.columns.contains(&span.column)) {
            self.emit.emit(span.column, span.low, span.high);
        }
    }
}

/// Calls `emit(column, low, high)` for each pixel column in `columns` the polyline through
/// `points` passes, as [`Spans`] gives them.
#[cfg(test)]
pub(crate) fn for_each_span(
    points: impl Iterator<Item = (f64, f64)>,
    columns: Range<i64>,
    emit: impl FnMut(i64, f64, f64),
) {
    let mut spans = Spans::new(columns, emit);
    for (x, y) in points {
        spans.push(x, y);
    }
    spans.finish();
}

#[cfg(test)]
mod tests {
    use super::for_each_span;

    fn spans(points: &[(f64, f64)], columns: std::ops::Range<i64>) -> Vec<(i64, f64, f64)> {
        let mut out = Vec::new();
        for_each_span(points.iter().copied(), columns, |column, low, high| {
            out.push((column, low, high));
        });
        out
    }

    #[test]
    fn a_flat_line_is_one_span_per_column_it_crosses() {
        let got = spans(&[(0.5, 5.0), (9.5, 5.0)], 0..100);
        let want: Vec<_> = (0..10).map(|column| (column, 5.0, 5.0)).collect();
        assert_eq!(got, want);
    }

    #[test]
    fn a_slope_covers_its_rise_in_each_column_and_neighbours_meet() {
        let got = spans(&[(0.0, 0.0), (4.0, 8.0)], 0..100);
        assert_eq!(
            got,
            [
                (0, 0.0, 2.0),
                (1, 2.0, 4.0),
                (2, 4.0, 6.0),
                (3, 6.0, 8.0),
                (4, 8.0, 8.0)
            ]
        );
    }

    #[test]
    fn points_within_one_column_become_its_lowest_and_highest() {
        let got = spans(&[(3.1, 4.0), (3.4, 1.0), (3.6, 7.0), (3.9, 2.0)], 0..100);
        assert_eq!(got, [(3, 1.0, 7.0)]);
    }

    #[test]
    fn a_gap_breaks_the_line() {
        let got = spans(
            &[(0.0, 0.0), (1.5, f64::NAN), (3.0, 6.0), (4.0, 6.0)],
            0..100,
        );
        assert_eq!(got, [(0, 0.0, 0.0), (3, 6.0, 6.0), (4, 6.0, 6.0)]);
        let infinite = spans(&[(0.0, 0.0), (1.0, f64::INFINITY), (2.0, 0.0)], 0..100);
        assert_eq!(infinite, [(0, 0.0, 0.0), (2, 0.0, 0.0)]);
    }

    #[test]
    fn only_the_columns_asked_for_are_given() {
        let got = spans(&[(-1_000.0, 0.0), (1_000.0, 2_000.0)], 0..3);
        assert_eq!(
            got,
            [
                (0, 1_000.0, 1_001.0),
                (1, 1_001.0, 1_002.0),
                (2, 1_002.0, 1_003.0)
            ]
        );
        let far = spans(&[(-1e30, 1.0), (1e30, 1.0)], 0..2);
        assert_eq!(far, [(0, 1.0, 1.0), (1, 1.0, 1.0)]);
    }

    #[test]
    fn a_point_left_of_the_last_starts_the_line_afresh() {
        let got = spans(&[(5.0, 0.0), (2.0, 1.0), (3.0, 3.0)], 0..100);
        assert_eq!(got, [(5, 0.0, 0.0), (2, 1.0, 3.0), (3, 3.0, 3.0)]);
    }

    #[test]
    fn gaps_within_a_column_leave_it_one_span() {
        let points = [
            (0.1, 1.0),
            (0.2, f64::NAN),
            (0.3, 5.0),
            (0.4, f64::NAN),
            (1.5, 2.0),
        ];
        assert_eq!(spans(&points, 0..9), [(0, 1.0, 5.0), (1, 2.0, 2.0)]);
    }

    #[test]
    fn a_lone_point_is_a_span_of_no_height() {
        assert_eq!(spans(&[(7.2, 3.0)], 0..100), [(7, 3.0, 3.0)]);
        assert_eq!(spans(&[], 0..100), []);
    }
}
