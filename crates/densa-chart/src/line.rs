use std::ops::Range;

use densa_core::{Color, Rect};
use densa_draw::Canvas;

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
/// allocated.
pub fn paint_line<P: Points + ?Sized>(
    canvas: &mut Canvas<'_>,
    plot: &Plot,
    points: &P,
    style: LineStyle,
) {
    let area = plot.area();
    if area.is_empty() {
        return;
    }
    let state = canvas.state();
    canvas.clip(area);
    let range = with_neighbours(points_within(points, &plot.x()), points.len());
    let at = range.map(|index| (plot.x_at(points.x(index)), plot.y_at(points.y(index))));
    // Only the columns that show: a plot wider than its view costs the view's width.
    let shown = canvas.clip_rect();
    // Lossless enough: columns are whole pixels of a canvas.
    let columns = shown.x0.floor() as i64..shown.x1.ceil() as i64;
    let half = f64::from(usable_width(style.width)) / 2.0;
    for_each_span(at, columns, |column, low, high| {
        let top = plot.clamp_y(low - half);
        let bottom = plot.clamp_y(high + half);
        // Lossless enough: a pixel column of a canvas.
        fill_span(canvas, column as f32, top, bottom, style.color);
    });
    canvas.restore(state);
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

/// Calls `emit(column, low, high)` for each pixel column in `columns` the polyline through
/// `points` (canvas pixels, ascending `x`) passes, left to right: the `y` range the line
/// covers within that column; for ascending `x`, at most one span per column. A point that
/// is not finite breaks the line; a lone point is a span of no height.
pub(crate) fn for_each_span(
    points: impl Iterator<Item = (f64, f64)>,
    columns: Range<i64>,
    mut emit: impl FnMut(i64, f64, f64),
) {
    let mut flush = |span: Option<Span>| {
        if let Some(span) = span.filter(|span| columns.contains(&span.column)) {
            emit(span.column, span.low, span.high);
        }
    };
    let mut open: Option<Span> = None;
    let mut last: Option<(f64, f64)> = None;
    for (x, y) in points {
        if !(x.is_finite() && y.is_finite()) {
            // A gap: nothing joins across it, but its column's span stays open, so a column
            // still gets one span however many gaps fall in it.
            last = None;
            continue;
        }
        // Lossless enough: pixel columns, saturating far off the canvas.
        let column = x.floor() as i64;
        match (last, open.as_mut()) {
            (_, Some(span)) if column == span.column => span.reach(y),
            (Some((x0, y0)), Some(span)) if column > span.column => {
                let slope = (y - y0) / (x - x0);
                let y_at = |at: f64| y0 + (at - x0) * slope;
                // Lossless enough: column edges.
                let edge = |column: i64| y_at(column as f64);
                span.reach(edge(span.column + 1));
                let first = span.column + 1;
                flush(open.take());
                for between in first.max(columns.start)..column.min(columns.end) {
                    flush(Some(Span::new(between, edge(between), edge(between + 1))));
                }
                open = Some(Span::new(column, edge(column), y));
            }
            _ => {
                // The first point, one after a gap, or one left of the last (x out of
                // order): start afresh.
                flush(open.take());
                open = Some(Span::new(column, y, y));
            }
        }
        last = Some((x, y));
    }
    flush(open);
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
