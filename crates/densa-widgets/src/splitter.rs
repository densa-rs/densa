use densa_core::Rect;
use densa_draw::Canvas;
use densa_ui::{
    Cursor, Event, EventCx, Handled, Length, PaintCx, PointerButton, PointerKind, Style, Widget,
};

/// The divider's thickness, in logical pixels: the gap between the panes.
const HANDLE: f32 = 6.0;
/// The smallest a pane gets by dragging, in logical pixels.
const MIN_PANE: f32 = 48.0;

/// Which way the panes sit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Split {
    Row,
    Column,
}

/// Two panes side by side (or one above the other) with a draggable divider between them.
///
/// Add two children: the first keeps the size the divider gives it, the second takes the
/// rest. Give the first a starting size in its style (`width` for a row, `height` for a
/// column) and the second `grow(1.0)`; [`Splitter::style`] spaces them for the divider.
#[derive(Debug)]
pub struct Splitter {
    direction: Split,
    /// Where on the divider the drag started, from the first pane's far edge.
    grab: Option<f32>,
}

impl Splitter {
    /// Panes left and right of a vertical divider.
    pub fn row() -> Self {
        Self {
            direction: Split::Row,
            grab: None,
        }
    }

    /// Panes above and below a horizontal divider.
    pub fn column() -> Self {
        Self {
            direction: Split::Column,
            ..Self::row()
        }
    }

    /// The splitter's style: its direction, the divider's gap, filling its parent's free
    /// space and allowed to be smaller than its content (whose panes then scroll or clip).
    pub fn style(&self) -> Style {
        let base = match self.direction {
            Split::Row => Style::row(),
            Split::Column => Style::column(),
        };
        base.gap(HANDLE)
            .grow(1.0)
            .min_size(Length::Px(0.0), Length::Px(0.0))
    }

    fn cursor(&self) -> Cursor {
        match self.direction {
            Split::Row => Cursor::ResizeColumn,
            Split::Column => Cursor::ResizeRow,
        }
    }

    /// Along the splitter's axis: where `rect` starts and ends.
    fn span(&self, rect: Rect) -> (f32, f32) {
        match self.direction {
            Split::Row => (rect.left(), rect.right()),
            Split::Column => (rect.top(), rect.bottom()),
        }
    }
}

impl Widget for Splitter {
    fn paint(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        let Some(pane) = cx.children().first().and_then(|&first| cx.rect_of(first)) else {
            return;
        };
        // A line in the middle of the gap, in the accent color while pointed at or dragged.
        let content = cx.content_rect();
        let line = cx.px(1.0).round().max(1.0);
        let middle = (self.span(pane).1 + cx.px(HANDLE) / 2.0 - line / 2.0).round();
        let divider = match self.direction {
            Split::Row => Rect::new(middle, content.top(), line, content.height()),
            Split::Column => Rect::new(content.left(), middle, content.width(), line),
        };
        let theme = cx.theme();
        let color = if cx.is_pressed() || cx.is_hover_target() {
            theme.accent
        } else {
            theme.border
        };
        canvas.fill_rect(divider, color);
    }

    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        let Event::Pointer(pointer) = event else {
            return Handled::No;
        };
        let Some(&first) = cx.children().first() else {
            return Handled::No;
        };
        let Some(pane) = cx.rect_of(first) else {
            return Handled::No;
        };
        let at = match self.direction {
            Split::Row => pointer.position.x,
            Split::Column => pointer.position.y,
        };
        match pointer.kind {
            // Over the splitter itself, not a pane: the pointer is on the divider.
            PointerKind::Enter => {
                cx.set_cursor(Some(self.cursor()));
                cx.request_paint();
                Handled::Yes
            }
            PointerKind::Leave => {
                cx.set_cursor(None);
                cx.request_paint();
                Handled::Yes
            }
            PointerKind::Down(PointerButton::Primary) => {
                // Presses in a pane that its widgets leave unhandled come up here too: only
                // one on the divider, between the panes, starts a drag.
                let position = pointer.position;
                let on_pane = cx
                    .children()
                    .iter()
                    .any(|&child| cx.rect_of(child).is_some_and(|r| r.contains(position)));
                if on_pane {
                    return Handled::No;
                }
                self.grab = Some(at - self.span(pane).1);
                cx.request_paint();
                Handled::Yes
            }
            PointerKind::Cancel => {
                self.grab = None;
                cx.request_paint();
                Handled::Yes
            }
            PointerKind::Move => {
                let Some(grab) = self.grab else {
                    return Handled::No;
                };
                let (start, end) = self.span(cx.content_rect());
                let (pane_start, _) = self.span(pane);
                let most = (end - start - cx.px(HANDLE) - cx.px(MIN_PANE)).max(cx.px(MIN_PANE));
                let size = (at - grab - pane_start).clamp(cx.px(MIN_PANE), most);
                let logical = Length::Px(size / cx.scale());
                let style = cx.style_of(first).cloned().unwrap_or_default().shrink(0.0);
                let style = match self.direction {
                    Split::Row => style.width(logical),
                    Split::Column => style.height(logical),
                };
                cx.set_style(first, style);
                Handled::Yes
            }
            PointerKind::Up(PointerButton::Primary) => {
                self.grab = None;
                cx.request_paint();
                Handled::Yes
            }
            _ => Handled::No,
        }
    }
}
