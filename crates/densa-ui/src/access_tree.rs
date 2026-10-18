//! Building a [`Ui`]'s accessibility tree and carrying out assistive technology's requests.

use densa_core::Point;

use crate::access::{AccessAction, AccessId, AccessNode, AccessRequest, AccessTree, Role};
use crate::{EventCx, Handled, Overflow, PaintCx, Ui, WidgetId};

/// How far a scroll request moves content, in logical pixels.
const SCROLL_STEP: f32 = 48.0;

impl Ui {
    /// The accessibility node id of widget `id`, if it lives.
    pub fn access_id(&self, id: WidgetId) -> Option<AccessId> {
        self.state
            .node(id)
            .map(|node| AccessId::widget(node.access))
    }

    /// The accessibility tree of the shown widgets, laid out first if needed. It allocates:
    /// build it only while assistive technology is listening.
    pub fn access_tree(&mut self) -> AccessTree {
        self.layout();
        let mut nodes = Vec::new();
        let root = self.state.root;
        let root_id = self.access_node(root, &mut nodes);
        let focus = match self.state.focus.filter(|f| self.state.is_shown(*f)) {
            Some(widget) => {
                let id = AccessId::widget(self.state.nodes[widget.slot()].access);
                let part = nodes
                    .iter()
                    .find(|(i, _)| *i == id)
                    .and_then(|(_, node)| node.focused_part())
                    .map(|part| id.part(part));
                // A focused part the widget did not add has no node: the widget has focus.
                part.filter(|p| nodes.iter().any(|(i, _)| i == p))
                    .unwrap_or(id)
            }
            None => root_id,
        };
        AccessTree {
            nodes,
            root: root_id,
            focus,
        }
    }

    /// Adds widget `id`'s node, its parts and its shown children to `out`; returns its id.
    fn access_node(&self, id: WidgetId, out: &mut Vec<(AccessId, AccessNode)>) -> AccessId {
        let state = &self.state.nodes[id.slot()];
        let node_id = AccessId::widget(state.access);
        let role = if id == self.state.root {
            Role::Window
        } else {
            Role::Group
        };
        let mut node = AccessNode::new(role);
        node.set_bounds(state.rect);
        node.set_clips_children(state.style.overflow != Overflow::Visible);
        if let Some(widget) = self.widgets[id.slot()].as_ref() {
            widget.accessibility(
                &PaintCx {
                    state: &self.state,
                    id,
                },
                &mut node,
            );
        }
        if self.accepts_focus(id) {
            node.add_action(AccessAction::Focus);
        }
        node.add_action(AccessAction::ScrollIntoView);
        let at = out.len();
        out.push((node_id, AccessNode::default()));
        let mut seen = Vec::new();
        for (part, sub) in node.take_parts() {
            if let Some(child) = Self::access_part(node_id, part, sub, &mut seen, out) {
                node.push_child(child);
            }
        }
        for index in 0..self.state.nodes[id.slot()].children.len() {
            let child = self.state.nodes[id.slot()].children[index];
            if !self.state.nodes[child.slot()].style.hidden {
                let child = self.access_node(child, out);
                node.push_child(child);
            }
        }
        out[at].1 = node;
        node_id
    }

    /// Adds part `part` of the widget with node `widget`, and the parts inside it, to `out`;
    /// `None` (nothing added) for a number past [`MAX_PART`](crate::MAX_PART) or one the
    /// widget already used (`seen`), which would give two nodes one id.
    fn access_part(
        widget: AccessId,
        part: u32,
        mut node: AccessNode,
        seen: &mut Vec<u32>,
        out: &mut Vec<(AccessId, AccessNode)>,
    ) -> Option<AccessId> {
        if part > crate::MAX_PART || seen.contains(&part) {
            return None;
        }
        seen.push(part);
        let node_id = widget.part(part);
        let at = out.len();
        out.push((node_id, AccessNode::default()));
        for (inner, sub) in node.take_parts() {
            if let Some(child) = Self::access_part(widget, inner, sub, seen, out) {
                node.push_child(child);
            }
        }
        out[at].1 = node;
        Some(node_id)
    }

    /// Carries out assistive technology's request to do `action` to node `target`, with
    /// `value` for [`AccessAction::SetValue`]; `false` if nothing did it: the widget is gone
    /// or not shown (a hidden widget is never acted on), or neither it nor, for scrolling, a
    /// parent handles it.
    pub fn access_action(
        &mut self,
        target: AccessId,
        action: AccessAction,
        value: Option<&str>,
    ) -> bool {
        let (serial, part) = target.split();
        let Some(id) = self.widget_numbered(serial) else {
            return false;
        };
        self.layout();
        let request = AccessRequest {
            action,
            part,
            value,
        };
        let done = match action {
            AccessAction::Focus if part.is_none() && self.accepts_focus(id) => {
                self.set_focus(Some(id));
                true
            }
            AccessAction::ScrollIntoView if part.is_none() => {
                self.scroll_into_view(id);
                true
            }
            AccessAction::ScrollUp
            | AccessAction::ScrollDown
            | AccessAction::ScrollLeft
            | AccessAction::ScrollRight => self.access_scroll(id, &request),
            _ => self.deliver_access(id, &request) == Handled::Yes,
        };
        self.settle();
        done
    }

    /// The shown widget numbered `serial`.
    fn widget_numbered(&self, serial: u64) -> Option<WidgetId> {
        let slot = self
            .state
            .nodes
            .iter()
            .position(|node| node.access == serial)?;
        let id = WidgetId {
            index: u32::try_from(slot).ok()?,
            generation: self.state.nodes[slot].generation,
        };
        self.state.is_shown(id).then_some(id)
    }

    fn deliver_access(&mut self, id: WidgetId, request: &AccessRequest<'_>) -> Handled {
        let Some(widget) = self.widgets.get_mut(id.slot()).and_then(Option::as_mut) else {
            return Handled::No;
        };
        let mut cx = EventCx {
            state: &mut self.state,
            id,
        };
        widget.access_action(&mut cx, request)
    }

    /// A scroll request: the widget, else the nearest scrolling widget around it.
    fn access_scroll(&mut self, id: WidgetId, request: &AccessRequest<'_>) -> bool {
        let mut at = Some(id);
        while let Some(current) = at {
            if self.deliver_access(current, request) == Handled::Yes {
                return true;
            }
            let node = &self.state.nodes[current.slot()];
            if node.style.overflow == Overflow::Scroll {
                let step = SCROLL_STEP * self.state.scale;
                let (dx, dy) = match request.action {
                    AccessAction::ScrollUp => (0.0, -step),
                    AccessAction::ScrollDown => (0.0, step),
                    AccessAction::ScrollLeft => (-step, 0.0),
                    _ => (step, 0.0),
                };
                let to = Point::new(node.scroll.x + dx, node.scroll.y + dy);
                let before = node.scroll;
                return self.state.scroll_to(current, to) != before;
            }
            at = node.parent;
        }
        false
    }

    /// Scrolls every scrolling widget around `id` so that `id` is in view.
    fn scroll_into_view(&mut self, id: WidgetId) {
        let mut at = self.state.nodes[id.slot()].parent;
        while let Some(container) = at {
            self.layout();
            let target = self.state.nodes[id.slot()].rect;
            let node = &self.state.nodes[container.slot()];
            if node.style.overflow == Overflow::Scroll {
                let view = node.content;
                let shift = |start: f32, end: f32, view_start: f32, view_end: f32| {
                    if start < view_start {
                        start - view_start
                    } else if end > view_end {
                        (end - view_end).min(start - view_start)
                    } else {
                        0.0
                    }
                };
                let dx = shift(target.left(), target.right(), view.left(), view.right());
                let dy = shift(target.top(), target.bottom(), view.top(), view.bottom());
                if dx != 0.0 || dy != 0.0 {
                    let to = Point::new(node.scroll.x + dx, node.scroll.y + dy);
                    self.state.scroll_to(container, to);
                }
            }
            at = self.state.nodes[container.slot()].parent;
        }
    }
}
