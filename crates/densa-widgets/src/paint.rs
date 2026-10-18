//! Drawing helpers shared by widgets.

use densa_core::{Color, Rect};
use densa_draw::Canvas;

/// A `width`-pixel line just inside `rect`'s edges.
pub(crate) fn outline(canvas: &mut Canvas<'_>, rect: Rect, width: f32, color: Color) {
    let (l, t, r, b) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    canvas.fill_rect(Rect::from_edges(l, t, r, t + width), color);
    canvas.fill_rect(Rect::from_edges(l, b - width, r, b), color);
    canvas.fill_rect(Rect::from_edges(l, t + width, l + width, b - width), color);
    canvas.fill_rect(Rect::from_edges(r - width, t + width, r, b - width), color);
}

/// The top-left corner that centers something of `width` x `height` in `rect`, on whole
/// pixels (text drawn between pixels blurs).
pub(crate) fn centered(rect: Rect, width: f32, height: f32) -> densa_core::Point {
    densa_core::Point::new(
        (rect.left() + (rect.width() - width) / 2.0).round(),
        (rect.top() + (rect.height() - height) / 2.0).round(),
    )
}
