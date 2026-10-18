//! Painting the grid: the rows that fit, frozen columns over the scrolled ones, the sort
//! arrow, the scroll bar and the focus ring.

use std::ops::Range;

use densa_core::{Color, Rect};
use densa_draw::Canvas;
use densa_grid::{Align, GridModel, SortOrder};
use densa_ui::PaintCx;

use super::{DataGrid, Frame, Shown};
use crate::paint::outline;

/// Scroll bar thickness, in logical pixels.
const BAR: f32 = 6.0;
/// How wide the strip that grabs the scroll bar is, in logical pixels: wider than the bar.
pub(super) const BAR_GRAB: f32 = 12.0;
/// The shortest the scroll bar's thumb gets, in logical pixels.
const MIN_THUMB: f32 = 24.0;

impl<M: GridModel> DataGrid<M> {
    /// Where the header, the rows and the frozen columns are when the grid is at `rect`.
    pub(super) fn frame(&self, rect: Rect) -> Option<Frame> {
        let layout = self.layout_of()?;
        let header_bottom = rect.top() + (layout.padding() + layout.row_height()) as f32;
        let count = self.columns.len().min(layout.column_count());
        let frozen = match self.frozen.min(count) {
            0 => 0.0,
            // The right edge of the last frozen column, half a gap past its text.
            n => {
                let last = n - 1;
                let edge = layout.column_x(last) + layout.column_width(last);
                (edge as f32 + layout.gap() as f32 / 2.0).round()
            }
        };
        Some(Frame {
            rect,
            header: Rect::from_edges(rect.left(), rect.top(), rect.right(), header_bottom),
            body: Rect::from_edges(rect.left(), header_bottom, rect.right(), rect.bottom()),
            frozen,
        })
    }

    /// How far the columns scroll sideways at most, for a grid `width` wide.
    pub(super) fn max_scroll_x(&self, width: f32) -> f32 {
        self.layout_of()
            .map_or(0.0, |l| (l.width() as f32 - width).max(0.0))
    }

    /// The unfrozen columns any part of which shows in a grid `width` wide scrolled by
    /// `scroll_x`, right of the frozen edge `frozen`.
    fn scrolled_columns(&self, width: f32, scroll_x: f32, frozen: f32) -> Range<usize> {
        let Some(layout) = self.layout_of() else {
            return 0..0;
        };
        let count = self.columns.len().min(layout.column_count());
        let first = self.frozen.min(count);
        let half = layout.gap() as f32 / 2.0;
        let shows = |c: usize| {
            let left = layout.column_x(c) as f32 - half - scroll_x;
            let right = left + layout.column_width(c) as f32 + 2.0 * half;
            right > frozen && left < width
        };
        let start = (first..count).find(|&c| shows(c)).unwrap_or(count);
        let end = (start..count)
            .take_while(|&c| shows(c))
            .last()
            .map_or(start, |c| c + 1);
        start..end
    }

    /// The scroll bar's thumb, if the rows do not all fit.
    pub(super) fn thumb(&self, frame: &Frame, scale: f32) -> Option<Rect> {
        let rows = self.model.rows();
        let whole = self.whole_rows().max(1);
        if rows <= whole {
            return None;
        }
        let track = frame.body;
        let length = (track.height() * whole as f32 / rows as f32)
            .max(MIN_THUMB * scale)
            .min(track.height());
        let travel = track.height() - length;
        // f32 is enough: the ratio only places the thumb to a pixel (dragging it back to a
        // row needs f64, see `drag_thumb`).
        let at = self.first_row as f32 / self.last_first_row().max(1) as f32;
        let top = (track.top() + travel * at).round();
        let width = (BAR * scale).round();
        Some(Rect::from_edges(
            track.right() - width,
            top,
            track.right(),
            top + length.round(),
        ))
    }

    pub(super) fn paint_grid(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        let rect = cx.rect();
        let theme = cx.theme();
        canvas.fill_rect(rect, theme.surface);
        let Some(frame) = self.frame(rect) else {
            return;
        };
        let scroll_x = self.scroll_x.min(self.max_scroll_x(rect.width())).round();
        let scrolled = self.scrolled_columns(rect.width(), scroll_x, frame.frozen);
        let frozen = 0..self.frozen.min(self.columns.len());
        let shown = Shown {
            model: &self.model,
            first: self.first_row,
            selected: self.selected.map(|s| s.row),
            highlight: theme.selection,
            palette: super::palette(theme),
        };
        let Some(painter) = self.painter.as_mut() else {
            return;
        };
        let saved = canvas.state();
        canvas.clip(rect);
        canvas.translate(rect.left(), rect.top());
        // The scrolled columns, right of the frozen ones; only those that show are asked for.
        let inner = canvas.state();
        canvas.clip(Rect::from_edges(
            frame.frozen,
            0.0,
            rect.width(),
            rect.height(),
        ));
        canvas.translate(-scroll_x, 0.0);
        painter.paint_columns(&shown, canvas, scrolled);
        canvas.restore(inner);
        // The frozen columns, in place.
        if frame.frozen > 0.0 {
            canvas.clip(Rect::from_edges(0.0, 0.0, frame.frozen, rect.height()));
            painter.paint_columns(&shown, canvas, frozen);
        }
        canvas.restore(saved);

        let line = cx.px(1.0).round().max(1.0);
        let inside = canvas.state();
        canvas.clip(rect);
        if frame.frozen > 0.0 && scroll_x > 0.0 {
            let x = rect.left() + frame.frozen;
            let edge = Rect::from_edges(x, rect.top(), x + line, rect.bottom());
            canvas.fill_rect(edge, theme.border);
        }
        self.paint_sort(cx, canvas, &frame, scroll_x);
        if let Some(thumb) = self.thumb(&frame, cx.scale()) {
            let color = if matches!(self.drag, Some(super::Drag::Thumb { .. })) {
                theme.accent
            } else {
                theme.text_dim.with_alpha(0.6)
            };
            canvas.fill_rect(thumb, color);
        }
        if cx.is_focused() {
            outline(canvas, rect, line, theme.focus);
        }
        canvas.restore(inside);
    }

    /// The arrow in the title of the column the rows are sorted by: inside the column, on
    /// the side away from its title, clear of the edges that resize columns.
    fn paint_sort(&self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>, frame: &Frame, scroll_x: f32) {
        let (Some((column, order)), Some(layout)) = (self.sort_order(), self.layout_of()) else {
            return;
        };
        if column >= self.columns.len().min(layout.column_count()) {
            return;
        }
        let line = cx.px(1.0).round().max(1.0);
        let shift = if column < self.frozen { 0.0 } else { scroll_x };
        let left = frame.rect.left() + layout.column_x(column) as f32 - shift;
        let right = left + layout.column_width(column) as f32;
        let x = match self.columns[column].align {
            Align::Right => left + 3.0 * line,
            _ => right - 3.0 * line,
        }
        .round();
        let middle = ((frame.header.top() + frame.header.bottom()) / 2.0).round();
        // Under the frozen columns when scrolled there: hidden.
        if column >= self.frozen && x < frame.rect.left() + frame.frozen {
            return;
        }
        arrow(canvas, x, middle, line, order, cx.theme().accent);
    }
}

/// A small arrow pointing up (ascending) or down, centred on `x`, `middle`, drawn as rows of
/// `line`-high rectangles widening away from its point.
fn arrow(canvas: &mut Canvas<'_>, x: f32, middle: f32, line: f32, order: SortOrder, color: Color) {
    let rows = 4_u8;
    for row in 0..rows {
        let width = (2.0 * f32::from(row) + 1.0) * line;
        let step = f32::from(row) * line;
        let y = match order {
            SortOrder::Ascending => middle - 2.0 * line + step,
            _ => middle + line - step,
        };
        let left = (x - width / 2.0).round();
        canvas.fill_rect(Rect::from_edges(left, y, left + width, y + line), color);
    }
}
