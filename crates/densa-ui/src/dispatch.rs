//! Delivering input: hit-testing, hover, pointer capture, bubbling to parents, focus.

use std::time::Duration;

use densa_core::Point;

use crate::ui::Press;
use crate::{
    Event, EventCx, Handled, Ime, Key, KeyEvent, NamedKey, Overflow, PointerButton, PointerEvent,
    PointerKind, Ui, WidgetId,
};

/// Presses closer than this in time count as one multi-click.
const MULTI_CLICK_TIME: Duration = Duration::from_millis(500);
/// ... and closer than this in logical pixels.
const MULTI_CLICK_DISTANCE: f32 = 4.0;

impl Ui {
    /// Delivers a pointer event.
    ///
    /// A press goes to the topmost widget under the pointer, then up its parents until one
    /// handles it; that widget then holds the pointer: it alone gets the moves, any other
    /// press and the release, which ends the hold. A release no widget holds goes nowhere,
    /// so a press elsewhere never ends in another widget's click. Moves and scrolls without a
    /// holder go to the widget under the pointer, then up. A press first focuses the nearest
    /// widget (from the one under the pointer up) that accepts focus. Hover follows the
    /// pointer, except while a widget holds it: `Enter` and `Leave` go to the topmost widget
    /// the pointer comes over and leaves (its parents see [`PaintCx::is_hovered`]).
    ///
    /// Open popups are over everything. A press or a scroll outside all of them closes them
    /// and goes no further, unless it is on the widget that opened them: that widget gets it
    /// with the popups still open (a menu bar closing its menu, or switching to another),
    /// and they close only if it does not handle it. A secondary press also reports
    /// [`ActionKind::ContextMenu`], except inside a popup (outside them it closes them and
    /// reports it for the widget under the pointer).
    ///
    /// [`PaintCx::is_hovered`]: crate::PaintCx::is_hovered
    /// [`ActionKind::ContextMenu`]: crate::ActionKind::ContextMenu
    pub fn pointer(&mut self, mut event: PointerEvent) -> Handled {
        self.layout();
        match event.kind {
            PointerKind::Down(button) => event.clicks = self.count_clicks(&event, button),
            // The Ui sends these itself.
            PointerKind::Enter | PointerKind::Leave | PointerKind::Cancel => return Handled::No,
            _ => {}
        }
        self.state.pointer = Some(event.position);
        let (hit, in_popup) = self.hit_top(event.position);
        if self.state.capture.is_none() {
            self.set_hover(hit, event.position);
        }
        let outside = !in_popup
            && !self.state.popups.is_empty()
            && self.state.capture.is_none()
            && matches!(event.kind, PointerKind::Down(_) | PointerKind::Scroll(_));
        if outside {
            let on_opener = hit.is_some_and(|hit| {
                self.state.popups.iter().any(|popup| {
                    popup
                        .opener
                        .is_some_and(|opener| self.state.is_within(hit, opener))
                })
            });
            if !on_opener {
                self.state.close_from(0);
                // A press that only closed popups does not begin a double click.
                self.state.last_press = None;
                // A right-click elsewhere closes a context menu and asks for one there, as
                // native menus do.
                if event.kind == PointerKind::Down(PointerButton::Secondary) {
                    self.context_menu(hit.unwrap_or(self.state.root), event.position);
                }
                self.settle();
                return Handled::Yes;
            }
        }
        let delivered = Event::Pointer(event);
        let handled = match (event.kind, self.state.capture) {
            (PointerKind::Up(_), holder) => {
                // The holder still holds the pointer while it handles the release.
                let handled =
                    holder.is_some_and(|holder| self.deliver(holder, &delivered) == Handled::Yes);
                self.state.capture = None;
                self.set_hover(hit, event.position);
                handled
            }
            (_, Some(holder)) => self.deliver(holder, &delivered) == Handled::Yes,
            (PointerKind::Down(button), None) => {
                self.focus_from(hit);
                let handler = self.bubble(hit, &delivered);
                self.state.capture = handler;
                if button == PointerButton::Secondary && !in_popup {
                    let source = handler.or(hit).unwrap_or(self.state.root);
                    self.context_menu(source, event.position);
                }
                if outside && handler.is_none() {
                    // The opener let the press go: it closes the popups as any press would.
                    self.state.close_from(0);
                }
                handler.is_some()
            }
            (_, None) => {
                let handled = self.bubble(hit, &delivered).is_some();
                if outside && !handled {
                    self.state.close_from(0);
                }
                handled
            }
        };
        self.settle();
        handled.into()
    }

    /// The pointer left the window: nothing is hovered any more (unless a widget holds the
    /// pointer, which keeps it until the release).
    pub fn pointer_left(&mut self, position: Point) {
        self.state.pointer = None;
        if self.state.capture.is_none() {
            self.set_hover(None, position);
        }
    }

    /// The press in progress was cancelled, say because the window lost focus: the widget
    /// holding the pointer gets [`PointerKind::Cancel`] and lets it go, and nothing is
    /// hovered.
    pub fn cancel_pointer(&mut self) {
        let at = self.state.pointer.unwrap_or(Point::ORIGIN);
        if let Some(holder) = self.state.capture {
            let cancel = PointerEvent::new(PointerKind::Cancel, at);
            self.deliver(holder, &Event::Pointer(cancel));
            self.state.capture = None;
        }
        self.state.pointer = None;
        self.set_hover(None, at);
        self.settle();
    }

    /// Delivers a key event to the focused widget (the root if none), then up its parents
    /// until one handles it. An unhandled Tab moves focus to the next widget that accepts it
    /// (Shift+Tab to the previous), in tree order (within the topmost popup while popups are
    /// open). An unhandled context menu key (or Shift+F10) reports
    /// [`ActionKind::ContextMenu`](crate::ActionKind::ContextMenu) for the focused widget.
    pub fn key(&mut self, event: &KeyEvent<'_>) -> Handled {
        self.layout();
        let target = self.state.focus.or(Some(self.state.root));
        let mut handled = self.bubble_key(target, &Event::Key(*event)).is_some();
        let menu_key = event.key == Key::Named(NamedKey::ContextMenu)
            || (event.key == Key::Named(NamedKey::F(10)) && event.modifiers.shift());
        if !handled && event.pressed && menu_key {
            let in_popup = self.state.focus.is_some_and(|focus| {
                self.state
                    .popups
                    .iter()
                    .any(|p| self.state.is_within(focus, p.id))
            });
            if let Some(focus) = self.state.focus.filter(|_| !in_popup) {
                let rect = self.state.nodes[focus.slot()].rect;
                self.context_menu(focus, Point::new(rect.left(), rect.top()));
                handled = true;
            }
        }
        if !handled && event.pressed && event.key == Key::Named(NamedKey::Tab) {
            let next = self.next_focus(!event.modifiers.shift());
            if next.is_some() {
                self.give_focus(next);
            }
            handled = true;
        }
        self.settle();
        handled.into()
    }

    /// Delivers an input method event to the focused widget, then up its parents.
    pub fn ime(&mut self, ime: &Ime<'_>) -> Handled {
        self.layout();
        let Some(focus) = self.state.focus else {
            return Handled::No;
        };
        let handled = self.bubble(Some(focus), &Event::Ime(*ime)).is_some();
        self.settle();
        handled.into()
    }

    /// The widget with keyboard focus.
    pub fn focus(&self) -> Option<WidgetId> {
        self.state.focus
    }

    /// Gives keyboard focus to `id` (`None`: to nobody), sending `FocusLost` and
    /// `FocusGained`. A removed or hidden widget gets nothing.
    pub fn set_focus(&mut self, id: Option<WidgetId>) {
        self.give_focus(id);
        self.settle();
    }

    pub(crate) fn give_focus(&mut self, id: Option<WidgetId>) {
        let id = id.filter(|id| self.state.is_shown(*id));
        if id == self.state.focus {
            return;
        }
        let old = std::mem::replace(&mut self.state.focus, id);
        if let Some(old) = old {
            self.deliver(old, &Event::FocusLost);
        }
        if let Some(new) = id {
            self.deliver(new, &Event::FocusGained);
        }
        self.state.needs_paint = true;
    }

    /// The widget under the pointer, deepest first.
    pub fn hovered(&self) -> Option<WidgetId> {
        self.state.hover
    }

    /// The topmost visible widget at `at`: later children are over earlier ones, and a
    /// widget that clips hides its children outside it.
    pub fn hit_test(&mut self, at: Point) -> Option<WidgetId> {
        self.layout();
        self.hit_top(at).0
    }

    /// The topmost widget at `at`, popups first; and whether it is in a popup.
    pub(crate) fn hit_top(&self, at: Point) -> (Option<WidgetId>, bool) {
        let layer = &self.state.nodes[self.state.layer.slot()];
        for &popup in layer.children.iter().rev() {
            if let Some(hit) = self.hit(popup, at) {
                return (Some(hit), true);
            }
        }
        (self.hit(self.state.root, at), false)
    }

    /// Reports a context menu request for `source` at `at`.
    fn context_menu(&mut self, source: WidgetId, at: Point) {
        self.state.actions.push_back(crate::Action {
            source,
            kind: crate::ActionKind::ContextMenu { at },
        });
    }

    pub(crate) fn hit(&self, id: WidgetId, at: Point) -> Option<WidgetId> {
        let node = &self.state.nodes[id.slot()];
        if node.style.hidden {
            return None;
        }
        let clips = node.style.overflow != Overflow::Visible;
        if clips && !node.rect.contains(at) {
            return None;
        }
        for &child in node.children.iter().rev() {
            if let Some(hit) = self.hit(child, at) {
                return Some(hit);
            }
        }
        node.rect.contains(at).then_some(id)
    }

    fn deliver(&mut self, id: WidgetId, event: &Event<'_>) -> Handled {
        if !self.state.is_live(id) {
            return Handled::No;
        }
        let Some(widget) = self.widgets.get_mut(id.slot()).and_then(Option::as_mut) else {
            return Handled::No;
        };
        let mut cx = EventCx {
            state: &mut self.state,
            id,
        };
        widget.event(&mut cx, event)
    }

    /// Delivers a key `event` to `target`, then its parents, until one handles it; from a
    /// popup it goes on to the widget that opened the popup (a menu passing Left and Right on
    /// to its menu bar, a submenu passing them to its menu).
    fn bubble_key(&mut self, mut target: Option<WidgetId>, event: &Event<'_>) -> Option<WidgetId> {
        while let Some(id) = target.filter(|id| self.state.is_live(*id)) {
            if self.deliver(id, event) == Handled::Yes {
                return Some(id);
            }
            let parent = self.state.nodes[id.slot()].parent;
            target = match parent {
                Some(layer) if layer == self.state.layer => self
                    .state
                    .popups
                    .iter()
                    .find(|popup| popup.id == id)
                    .and_then(|popup| popup.opener),
                _ => parent,
            };
        }
        None
    }

    /// Delivers `event` to `target`, then its parents, until one handles it.
    fn bubble(&mut self, mut target: Option<WidgetId>, event: &Event<'_>) -> Option<WidgetId> {
        while let Some(id) = target.filter(|id| self.state.is_live(*id)) {
            if self.deliver(id, event) == Handled::Yes {
                return Some(id);
            }
            target = self.state.nodes[id.slot()].parent;
        }
        None
    }

    pub(crate) fn set_hover(&mut self, hover: Option<WidgetId>, position: Point) {
        let old = self.state.hover;
        if old == hover {
            return;
        }
        self.mark_hovered(old, false);
        self.mark_hovered(hover, true);
        self.state.hover = hover;
        if let Some(old) = old {
            let leave = PointerEvent::new(PointerKind::Leave, position);
            self.deliver(old, &Event::Pointer(leave));
        }
        if let Some(new) = hover {
            let enter = PointerEvent::new(PointerKind::Enter, position);
            self.deliver(new, &Event::Pointer(enter));
        }
        self.state.needs_paint = true;
    }

    /// Sets the hovered flag of `from` and its parents.
    pub(crate) fn mark_hovered(&mut self, from: Option<WidgetId>, hovered: bool) {
        let mut at = from;
        while let Some(id) = at.filter(|id| self.state.is_live(*id)) {
            let node = &mut self.state.nodes[id.slot()];
            node.hovered = hovered;
            at = node.parent;
        }
    }

    /// Focuses the nearest widget from `from` up that accepts focus, if any.
    fn focus_from(&mut self, from: Option<WidgetId>) {
        let mut at = from;
        while let Some(id) = at.filter(|id| self.state.is_live(*id)) {
            let widget = self.widgets.get(id.slot()).and_then(Option::as_deref);
            if widget.is_some_and(|w| w.accepts_focus() && w.focus_on_press()) {
                self.give_focus(Some(id));
                return;
            }
            at = self.state.nodes[id.slot()].parent;
        }
    }

    pub(crate) fn accepts_focus(&self, id: WidgetId) -> bool {
        self.widgets
            .get(id.slot())
            .and_then(Option::as_deref)
            .is_some_and(|w| w.accepts_focus())
    }

    /// 1 for a press, or one more than the last press of the same button if it was close in
    /// time and place.
    fn count_clicks(&mut self, event: &PointerEvent, button: crate::PointerButton) -> u8 {
        let near = MULTI_CLICK_DISTANCE * self.state.scale;
        let clicks = match self.state.last_press {
            Some(last)
                if last.button == button
                    && event.time >= last.time
                    && event.time - last.time <= MULTI_CLICK_TIME
                    && (event.position.x - last.position.x).abs() <= near
                    && (event.position.y - last.position.y).abs() <= near =>
            {
                last.clicks.saturating_add(1)
            }
            _ => 1,
        };
        self.state.last_press = Some(Press {
            time: event.time,
            position: event.position,
            button,
            clicks,
        });
        clicks
    }

    /// The widget Tab (`forward`) or Shift+Tab moves focus to: the next (or previous) visible
    /// widget accepting focus in tree order, wrapping around.
    fn next_focus(&self, forward: bool) -> Option<WidgetId> {
        let current = self.state.focus;
        let mut order = FocusOrder {
            current,
            seen: false,
            first: None,
            last: None,
            before: None,
            after: None,
        };
        // While popups are open, Tab stays in the topmost.
        let within = self
            .state
            .popups
            .last()
            .map_or(self.state.root, |popup| popup.id);
        self.visit_focusable(within, &mut order);
        if forward {
            order.after.or(order.first)
        } else {
            order.before.or(order.last)
        }
    }

    fn visit_focusable(&self, id: WidgetId, order: &mut FocusOrder) {
        let node = &self.state.nodes[id.slot()];
        if node.style.hidden {
            return;
        }
        if self.accepts_focus(id) {
            order.visit(id);
        }
        for &child in &node.children {
            self.visit_focusable(child, order);
        }
    }
}

/// Focusable widgets seen around the focused one, in tree order.
struct FocusOrder {
    current: Option<WidgetId>,
    seen: bool,
    first: Option<WidgetId>,
    last: Option<WidgetId>,
    before: Option<WidgetId>,
    after: Option<WidgetId>,
}

impl FocusOrder {
    fn visit(&mut self, id: WidgetId) {
        self.first = self.first.or(Some(id));
        self.last = Some(id);
        if Some(id) == self.current {
            self.seen = true;
        } else if self.seen {
            self.after = self.after.or(Some(id));
        } else {
            self.before = Some(id);
        }
    }
}
