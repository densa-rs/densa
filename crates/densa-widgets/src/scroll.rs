use densa_core::{Point, Rect, Size};
use densa_draw::Canvas;
use densa_ui::{
    AccessAction, AccessNode, Event, EventCx, Handled, Length, Overflow, PaintCx, PointerButton,
    PointerKind, Role, ScrollDelta, Style, Widget,
};

/// Logical pixels one wheel notch scrolls.
const LINE: f32 = 48.0;
/// Scroll bar thickness, in logical pixels.
const BAR: f32 = 6.0;
/// The shortest a scroll bar's thumb gets, in logical pixels.
const MIN_THUMB: f32 = 24.0;

/// Scrolls its children with the wheel, a touchpad or by dragging its scroll bars.
///
/// Give it a style with [`Overflow::Scroll`], as [`ScrollView::style`] does: its children
/// are then laid out at their own size and cut at its edges. A wheel scroll it cannot use
/// (already at the end) goes on to the views around it.
#[derive(Debug, Default)]
pub struct ScrollView {
    /// The axis being dragged by its thumb, and where on the thumb it was grabbed.
    drag: Option<(Axis, f32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

impl ScrollView {
    /// A scroll view.
    pub fn new() -> Self {
        Self::default()
    }

    /// A column filling its parent's free space, allowed to be smaller than its content, and
    /// scrolling the overflow.
    pub fn style() -> Style {
        Style::column()
            .grow(1.0)
            .min_size(Length::Px(0.0), Length::Px(0.0))
            .overflow(Overflow::Scroll)
    }

    fn scroll_by(cx: &mut EventCx<'_>, dx: f32, dy: f32) -> Handled {
        let at = cx.scroll();
        let moved = cx.scroll_to(Point::new(at.x + dx, at.y + dy));
        (moved != at).into()
    }
}

/// Where the thumb along `axis` is, if the content overflows on it.
fn thumb(axis: Axis, rect: Rect, content: Size, scroll: Point, scale: f32) -> Option<Rect> {
    let (shown, total, offset) = match axis {
        Axis::Vertical => (rect.height(), content.height, scroll.y),
        Axis::Horizontal => (rect.width(), content.width, scroll.x),
    };
    if total <= shown || shown <= 0.0 {
        return None;
    }
    let length = (shown * shown / total).max(MIN_THUMB * scale).min(shown);
    let start = offset / (total - shown) * (shown - length);
    let bar = BAR * scale;
    Some(match axis {
        Axis::Vertical => Rect::new(rect.right() - bar, rect.top() + start, bar, length),
        Axis::Horizontal => Rect::new(rect.left() + start, rect.bottom() - bar, length, bar),
    })
}

impl Widget for ScrollView {
    fn accessibility(&self, cx: &PaintCx<'_>, node: &mut AccessNode) {
        node.set_role(Role::ScrollView);
        let (at, content, view) = (cx.scroll(), cx.content_size(), cx.rect());
        let (room_x, room_y) = (content.width - view.width(), content.height - view.height());
        for (action, can) in [
            (AccessAction::ScrollUp, at.y > 0.0),
            (AccessAction::ScrollDown, at.y < room_y),
            (AccessAction::ScrollLeft, at.x > 0.0),
            (AccessAction::ScrollRight, at.x < room_x),
        ] {
            if can {
                node.add_action(action);
            }
        }
    }

    fn paint_over(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        let theme = cx.theme();
        for axis in [Axis::Vertical, Axis::Horizontal] {
            let at = thumb(axis, cx.rect(), cx.content_size(), cx.scroll(), cx.scale());
            if let Some(at) = at {
                let alpha = if self.drag.is_some_and(|(a, _)| a == axis) {
                    0.65
                } else {
                    0.45
                };
                let color = theme.text_dim.with_alpha(alpha);
                canvas.fill_rect(at, color);
            }
        }
    }

    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        let Event::Pointer(pointer) = event else {
            return Handled::No;
        };
        let at = pointer.position;
        match pointer.kind {
            PointerKind::Scroll(ScrollDelta::Lines(dx, dy)) => {
                let line = cx.px(LINE);
                Self::scroll_by(cx, dx * line, dy * line)
            }
            PointerKind::Scroll(ScrollDelta::Pixels(dx, dy)) => Self::scroll_by(cx, dx, dy),
            PointerKind::Down(PointerButton::Primary) => {
                let (rect, content, scroll) = (cx.rect(), cx.content_size(), cx.scroll());
                for axis in [Axis::Vertical, Axis::Horizontal] {
                    if let Some(bar) = thumb(axis, rect, content, scroll, cx.scale()) {
                        if bar.contains(at) {
                            let grab = match axis {
                                Axis::Vertical => at.y - bar.top(),
                                Axis::Horizontal => at.x - bar.left(),
                            };
                            self.drag = Some((axis, grab));
                            cx.request_paint();
                            return Handled::Yes;
                        }
                    }
                }
                Handled::No
            }
            PointerKind::Move => {
                let Some((axis, grab)) = self.drag else {
                    return Handled::No;
                };
                let (rect, content, scroll) = (cx.rect(), cx.content_size(), cx.scroll());
                let Some(bar) = thumb(axis, rect, content, scroll, cx.scale()) else {
                    return Handled::Yes;
                };
                // Thumb travel maps onto scroll travel.
                let (track, length, total, shown, pointer, start) = match axis {
                    Axis::Vertical => (
                        rect.height(),
                        bar.height(),
                        content.height,
                        rect.height(),
                        at.y,
                        rect.top(),
                    ),
                    Axis::Horizontal => (
                        rect.width(),
                        bar.width(),
                        content.width,
                        rect.width(),
                        at.x,
                        rect.left(),
                    ),
                };
                let travel = (track - length).max(1.0);
                let offset = (pointer - grab - start) / travel * (total - shown);
                let target = match axis {
                    Axis::Vertical => Point::new(scroll.x, offset),
                    Axis::Horizontal => Point::new(offset, scroll.y),
                };
                cx.scroll_to(target);
                Handled::Yes
            }
            PointerKind::Up(PointerButton::Primary) | PointerKind::Cancel
                if self.drag.take().is_some() =>
            {
                cx.request_paint();
                Handled::Yes
            }
            _ => Handled::No,
        }
    }
}
