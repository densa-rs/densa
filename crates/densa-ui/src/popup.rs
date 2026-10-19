//! Popups: widgets shown above everything else (menus, completion lists, tooltips that take
//! input), until closed.
//!
//! Popups live on a layer of their own, a second root laid out against the viewport: painted
//! after the widget tree and hit-tested before it. Each is placed next to an [`Anchor`],
//! moved or flipped to stay inside the viewport.

use std::fmt;

use densa_core::{Point, Rect, Size};

use crate::ui::State;
use crate::{EventCx, PaintCx, Position, Style, Ui, UiError, Widget, WidgetId};

/// Where a popup goes: next to a rectangle, kept inside the viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    rect: Rect,
    side: Side,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Below,
    Beside,
}

impl Anchor {
    /// Below `rect`, its left edges lined up (a menu under its menu bar title); above `rect`
    /// if it does not fit below.
    pub fn below(rect: Rect) -> Self {
        Self {
            rect,
            side: Side::Below,
        }
    }

    /// To the right of `rect`, its top edges lined up (a submenu beside its item); to the
    /// left if it does not fit on the right.
    pub fn beside(rect: Rect) -> Self {
        Self {
            rect,
            side: Side::Beside,
        }
    }

    /// With its top-left corner at `point` (a context menu at the pointer), moved to fit.
    pub fn at(point: Point) -> Self {
        Self::below(Rect::new(point.x, point.y, 0.0, 0.0))
    }

    /// The top-left corner of a popup of `size` in `viewport`.
    pub(crate) fn place(&self, size: Size, viewport: Size) -> Point {
        let fit = |start: f32, length: f32, room: f32| start.min(room - length).max(0.0);
        let rect = self.rect;
        match self.side {
            Side::Below => {
                let below = rect.bottom();
                let y = if below + size.height <= viewport.height {
                    below
                } else if rect.top() - size.height >= 0.0 {
                    rect.top() - size.height
                } else {
                    fit(below, size.height, viewport.height)
                };
                Point::new(fit(rect.left(), size.width, viewport.width), y)
            }
            Side::Beside => {
                let right = rect.right();
                let x = if right + size.width <= viewport.width {
                    right
                } else if rect.left() - size.width >= 0.0 {
                    rect.left() - size.width
                } else {
                    fit(right, size.width, viewport.width)
                };
                Point::new(x, fit(rect.top(), size.height, viewport.height))
            }
        }
    }
}

/// An open popup.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Popup {
    pub(crate) id: WidgetId,
    pub(crate) anchor: Anchor,
    /// The widget that opened it: a press on it while the popup is open still reaches it
    /// (a menu bar switching menus).
    pub(crate) opener: Option<WidgetId>,
    /// What had focus before it opened, given back when it closes.
    pub(crate) focus_before: Option<WidgetId>,
}

/// Widgets added while another widget handles an event, given to the `Ui` once it has.
#[derive(Default)]
pub(crate) struct Incoming(pub(crate) Vec<(WidgetId, Box<dyn Widget>)>);

impl fmt::Debug for Incoming {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|(id, _)| id))
            .finish()
    }
}

impl State {
    /// Adds `widget` as a popup next to `anchor`, opened by `opener`; it takes focus if it
    /// accepts it. The widget joins the `Ui` when the current event is over.
    pub(crate) fn open_popup(
        &mut self,
        widget: Box<dyn Widget>,
        anchor: Anchor,
        style: Style,
        opener: Option<WidgetId>,
    ) -> Result<WidgetId, UiError> {
        let style = Style {
            position: Position::Absolute,
            ..style
        };
        let layer = self.layer;
        let end = self.nodes[layer.slot()].children.len();
        let id = self.new_node(layer, end, style)?;
        // Focus as it will be once this event is over: popups closed in the same event (a
        // menu bar switching menus) have already asked for theirs back.
        let focus_before = self.requested_focus.unwrap_or(self.focus);
        if widget.accepts_focus() {
            self.requested_focus = Some(Some(id));
        }
        self.popups.push(Popup {
            id,
            anchor,
            opener,
            focus_before,
        });
        self.incoming.0.push((id, widget));
        Ok(id)
    }

    /// Closes popup `id` and every popup opened after it (removed once the current event is
    /// over); focus inside them goes back to what had it before `id` opened. `false` if `id`
    /// is not an open popup.
    pub(crate) fn close_popup(&mut self, id: WidgetId) -> bool {
        match self.popups.iter().position(|popup| popup.id == id) {
            Some(index) => {
                self.close_from(index);
                true
            }
            None => false,
        }
    }

    /// Closes every popup from number `index` (oldest first) on.
    pub(crate) fn close_from(&mut self, index: usize) {
        let Some(first) = self.popups.get(index).copied() else {
            return;
        };
        let focus_inside = self.focus.is_some_and(|focus| {
            self.popups[index..]
                .iter()
                .any(|popup| self.is_within(focus, popup.id))
        });
        for popup in self.popups.drain(index..).rev() {
            self.closing.push(popup.id);
        }
        if focus_inside {
            self.requested_focus = Some(first.focus_before);
        }
        self.needs_paint = true;
    }

    /// Whether `id` is an open popup.
    pub(crate) fn is_popup_open(&self, id: WidgetId) -> bool {
        self.popups.iter().any(|popup| popup.id == id) && self.is_live(id)
    }

    /// Where popup `id`, of `size`, goes.
    pub(crate) fn popup_origin(&self, id: WidgetId, size: Size) -> Option<Point> {
        let popup = self.popups.iter().find(|popup| popup.id == id)?;
        Some(popup.anchor.place(size, self.viewport()))
    }
}

impl Ui {
    /// Opens `widget` as a popup next to `anchor`, above every other widget, laid out by
    /// `style` (its position is made absolute: the anchor places it). It takes keyboard focus
    /// if it accepts it, and gives it back when it closes.
    ///
    /// While popups are open, a press or a scroll outside all of them closes them all and
    /// goes no further; moves still reach the widgets under the pointer.
    ///
    /// # Errors
    /// [`UiError::TooManyWidgets`].
    pub fn open_popup(
        &mut self,
        widget: impl Widget,
        anchor: Anchor,
        style: Style,
    ) -> Result<WidgetId, UiError> {
        let id = self
            .state
            .open_popup(Box::new(widget), anchor, style, None)?;
        self.settle();
        Ok(id)
    }

    /// Closes popup `id` and every popup opened after it. `false` if `id` is not open.
    pub fn close_popup(&mut self, id: WidgetId) -> bool {
        let closed = self.state.close_popup(id);
        self.settle();
        closed
    }

    /// Closes every popup (the window lost focus, or was resized).
    pub fn close_popups(&mut self) {
        self.state.close_from(0);
        self.settle();
    }

    /// The open popups, oldest first.
    pub fn popups(&self) -> impl Iterator<Item = WidgetId> + '_ {
        self.state.popups.iter().map(|popup| popup.id)
    }

    /// Whether `id` is an open popup.
    pub fn is_popup_open(&self, id: WidgetId) -> bool {
        self.state.is_popup_open(id)
    }

    /// Takes in the widgets added during the last event and removes the popups closed in it.
    pub(crate) fn adopt(&mut self) {
        let mut incoming = std::mem::take(&mut self.state.incoming.0);
        for (id, widget) in incoming.drain(..) {
            if !self.state.is_live(id) {
                continue;
            }
            let slot = id.slot();
            if slot >= self.widgets.len() {
                self.widgets.resize_with(slot + 1, || None);
            }
            self.widgets[slot] = Some(widget);
        }
        // Kept for the next popup, unless one was opened meanwhile.
        if self.state.incoming.0.is_empty() {
            self.state.incoming.0 = incoming;
        }
        let mut closing = std::mem::take(&mut self.state.closing);
        for id in closing.drain(..) {
            self.remove_widget(id);
        }
        if self.state.closing.is_empty() {
            self.state.closing = closing;
        }
    }
}

impl EventCx<'_> {
    /// Opens `widget` as a popup next to `anchor`, as [`Ui::open_popup`] does, with this
    /// widget as its opener: a press on this widget while the popup is open closes the
    /// popup and still reaches this widget. The popup joins the `Ui` once the event is
    /// handled.
    ///
    /// # Errors
    /// [`UiError::TooManyWidgets`].
    pub fn open_popup(
        &mut self,
        widget: impl Widget,
        anchor: Anchor,
        style: Style,
    ) -> Result<WidgetId, UiError> {
        let opener = Some(self.id);
        self.state
            .open_popup(Box::new(widget), anchor, style, opener)
    }

    /// Closes popup `id` (this widget, say) and every popup opened after it, once the event
    /// is handled. `false` if `id` is not open.
    pub fn close_popup(&mut self, id: WidgetId) -> bool {
        self.state.close_popup(id)
    }

    /// Closes every popup once the event is handled (a menu item was chosen).
    pub fn close_popups(&mut self) {
        self.state.close_from(0);
    }

    /// Whether `id` is an open popup.
    pub fn is_popup_open(&self, id: WidgetId) -> bool {
        self.state.is_popup_open(id)
    }
}

impl PaintCx<'_> {
    /// Whether `id` is an open popup.
    pub fn is_popup_open(&self, id: WidgetId) -> bool {
        self.state.is_popup_open(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: Size = Size {
        width: 800.0,
        height: 600.0,
    };

    #[test]
    fn below_fits_flips_and_clamps() {
        let size = Size::new(200.0, 100.0);
        let under = Anchor::below(Rect::new(10.0, 20.0, 80.0, 30.0));
        assert_eq!(under.place(size, VIEW), Point::new(10.0, 50.0));
        let low = Anchor::below(Rect::new(700.0, 550.0, 80.0, 30.0));
        assert_eq!(
            low.place(size, VIEW),
            Point::new(600.0, 450.0),
            "above, moved left"
        );
        let tall = Anchor::below(Rect::new(0.0, 50.0, 10.0, 10.0));
        assert_eq!(
            tall.place(Size::new(100.0, 580.0), VIEW),
            Point::new(0.0, 20.0),
            "neither fits: as low as it can go"
        );
    }

    #[test]
    fn beside_fits_flips_and_clamps() {
        let size = Size::new(200.0, 100.0);
        let item = Anchor::beside(Rect::new(100.0, 40.0, 150.0, 24.0));
        assert_eq!(item.place(size, VIEW), Point::new(250.0, 40.0));
        let edge = Anchor::beside(Rect::new(500.0, 560.0, 150.0, 24.0));
        assert_eq!(
            edge.place(size, VIEW),
            Point::new(300.0, 500.0),
            "left, moved up"
        );
        let wide = Anchor::beside(Rect::new(100.0, 0.0, 600.0, 24.0));
        assert_eq!(
            wide.place(size, VIEW),
            Point::new(600.0, 0.0),
            "neither side: pushed in"
        );
    }

    #[test]
    fn at_a_point_is_below_an_empty_rect() {
        let size = Size::new(200.0, 100.0);
        assert_eq!(
            Anchor::at(Point::new(790.0, 590.0)).place(size, VIEW),
            Point::new(600.0, 490.0)
        );
    }
}
