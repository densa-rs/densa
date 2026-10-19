use std::any::Any;
use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

use densa_core::{Point, Rect, Size};
use densa_draw::Canvas;
use densa_paragraph::TextSystem;
use taffy::style::AvailableSpace;
use taffy::{TaffyResult, TaffyTree};

use crate::popup::{Incoming, Popup};
use crate::{
    Action, Available, Constraints, Cursor, EventCx, LayoutCx, Overflow, PaintCx, PointerButton,
    Style, Theme, Widget, WidgetId,
};

/// Actions held without allocating between reads.
const ACTION_CAPACITY: usize = 64;

/// Why a [`Ui`] refused a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UiError {
    /// The widget was removed, or belongs to another `Ui`.
    NoSuchWidget(WidgetId),
    /// More than 2^32 widgets.
    TooManyWidgets,
}

impl fmt::Display for UiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchWidget(id) => write!(f, "there is no widget {id:?}"),
            Self::TooManyWidgets => f.write_str("more than 2^32 widgets"),
        }
    }
}

impl std::error::Error for UiError {}

/// A widget's place in the tree and its layout.
#[derive(Debug)]
pub(crate) struct Node {
    pub(crate) generation: u32,
    /// Its number for assistive technology: never reused, unlike its slot.
    pub(crate) access: u64,
    pub(crate) live: bool,
    pub(crate) parent: Option<WidgetId>,
    pub(crate) children: Vec<WidgetId>,
    pub(crate) taffy: taffy::NodeId,
    pub(crate) style: Style,
    /// Where it is, in physical pixels from the window's top-left corner.
    pub(crate) rect: Rect,
    /// `rect` inside the padding.
    pub(crate) content: Rect,
    /// What its children cover.
    pub(crate) content_size: Size,
    /// How far its children are scrolled.
    pub(crate) scroll: Point,
    /// Whether the pointer is over it or one of its children.
    pub(crate) hovered: bool,
    pub(crate) cursor: Option<Cursor>,
    /// `rect` grown to cover children that overflow it: what painting it can touch.
    pub(crate) extent: Rect,
    /// Its style or content changed since `Widget::layout` was last called.
    pub(crate) dirty: bool,
    /// The size `Widget::layout` was last called with.
    pub(crate) notified: Option<Size>,
}

/// A press remembered to count multi-clicks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Press {
    pub(crate) time: Duration,
    pub(crate) position: Point,
    pub(crate) button: PointerButton,
    pub(crate) clicks: u8,
}

/// Everything but the widgets: what contexts borrow while a widget runs.
#[derive(Debug)]
pub(crate) struct State {
    pub(crate) nodes: Vec<Node>,
    free: Vec<u32>,
    /// The last widget serial given out (the root has 0).
    pub(crate) next_access: u64,
    taffy: TaffyTree<WidgetId>,
    pub(crate) root: WidgetId,
    /// The popup layer: a second root over the first, laid out against the viewport.
    pub(crate) layer: WidgetId,
    /// Open popups, oldest first.
    pub(crate) popups: Vec<Popup>,
    /// Widgets added during an event, given to the `Ui` after it.
    pub(crate) incoming: Incoming,
    /// Popups closed during an event, removed after it.
    pub(crate) closing: Vec<WidgetId>,
    pub(crate) theme: Theme,
    pub(crate) scale: f32,
    viewport: Size,
    pub(crate) text: TextSystem,
    pub(crate) focus: Option<WidgetId>,
    pub(crate) hover: Option<WidgetId>,
    pub(crate) capture: Option<WidgetId>,
    /// Focus asked for by a widget, given once its event is handled.
    pub(crate) requested_focus: Option<Option<WidgetId>>,
    pub(crate) actions: VecDeque<Action>,
    pub(crate) needs_layout: bool,
    pub(crate) needs_paint: bool,
    /// Scrolling moved widgets: their rectangles need computing again, not their layout.
    pub(crate) needs_place: bool,
    pub(crate) ime_area: Option<(WidgetId, Rect)>,
    pub(crate) last_press: Option<Press>,
    /// Where the pointer is, if over the window: hover follows layout changes under it.
    pub(crate) pointer: Option<Point>,
}

/// The layout tree mirrors the widget tree, so its operations only fail on ids this crate
/// never hands it.
fn sync<T>(result: TaffyResult<T>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => unreachable!("the layout tree is out of step with the widgets: {error}"),
    }
}

impl State {
    pub(crate) fn is_live(&self, id: WidgetId) -> bool {
        self.nodes
            .get(id.slot())
            .is_some_and(|node| node.live && node.generation == id.generation)
    }

    pub(crate) fn node(&self, id: WidgetId) -> Option<&Node> {
        self.is_live(id).then(|| &self.nodes[id.slot()])
    }

    pub(crate) fn viewport(&self) -> Size {
        self.viewport
    }

    /// `id`'s layout engine style: the root and the popup layer fill the viewport.
    fn taffy_style(&self, id: WidgetId) -> taffy::Style {
        let mut style = self.nodes[id.slot()].style.to_taffy(self.scale);
        if id == self.root || id == self.layer {
            style.size = taffy::Size {
                width: taffy::style::Dimension::length(self.viewport.width),
                height: taffy::style::Dimension::length(self.viewport.height),
            };
        }
        style
    }

    /// Gives `id` a new, childless layout node in its old one's place. taffy keeps the
    /// absolutely positioned descendants it lays out on their container ("hoisted children")
    /// and clears that list only when it lays the container out as one: a container whose
    /// last child is removed is laid out as a leaf from then on, and rounding the layout
    /// panicked on the removed child's id.
    fn renew_layout_node(&mut self, id: WidgetId) {
        let old = self.nodes[id.slot()].taffy;
        let fresh = sync(self.taffy.new_leaf_with_context(self.taffy_style(id), id));
        if let Some(parent) = self.taffy.parent(old) {
            let index = sync(self.taffy.children(parent))
                .iter()
                .position(|child| *child == old);
            if let Some(index) = index {
                sync(self.taffy.replace_child_at_index(parent, index, fresh));
            }
        }
        sync(self.taffy.remove(old));
        self.nodes[id.slot()].taffy = fresh;
        self.nodes[id.slot()].dirty = true;
    }

    /// Adds a node for a new widget as child number `position` of `parent` (past the end:
    /// the last); the caller puts the widget in the `Ui`'s slot for it.
    pub(crate) fn new_node(
        &mut self,
        parent: WidgetId,
        position: usize,
        style: Style,
    ) -> Result<WidgetId, UiError> {
        if !self.is_live(parent) {
            return Err(UiError::NoSuchWidget(parent));
        }
        let position = position.min(self.nodes[parent.slot()].children.len());
        let index = match self.free.pop() {
            Some(index) => index,
            None => u32::try_from(self.nodes.len()).map_err(|_| UiError::TooManyWidgets)?,
        };
        let slot = index as usize;
        let generation = self.nodes.get(slot).map_or(0, |node| node.generation);
        let id = WidgetId { index, generation };
        let taffy = sync(
            self.taffy
                .new_leaf_with_context(style.to_taffy(self.scale), id),
        );
        sync(
            self.taffy
                .insert_child_at_index(self.nodes[parent.slot()].taffy, position, taffy),
        );
        self.next_access += 1;
        let node = Node {
            generation,
            access: self.next_access,
            live: true,
            parent: Some(parent),
            children: Vec::new(),
            taffy,
            style,
            rect: Rect::default(),
            content: Rect::default(),
            content_size: Size::ZERO,
            scroll: Point::ORIGIN,
            hovered: false,
            cursor: None,
            extent: Rect::default(),
            dirty: true,
            notified: None,
        };
        if slot == self.nodes.len() {
            self.nodes.push(node);
        } else {
            self.nodes[slot] = node;
        }
        self.nodes[parent.slot()].children.insert(position, id);
        self.needs_layout = true;
        self.needs_paint = true;
        Ok(id)
    }

    pub(crate) fn set_style(&mut self, id: WidgetId, style: Style) -> bool {
        if !self.is_live(id) {
            return false;
        }
        self.nodes[id.slot()].style = style;
        self.nodes[id.slot()].dirty = true;
        let taffy_style = self.taffy_style(id);
        sync(
            self.taffy
                .set_style(self.nodes[id.slot()].taffy, taffy_style),
        );
        self.needs_layout = true;
        self.needs_paint = true;
        true
    }

    /// Whether `id` and every widget around it are shown (none hidden).
    pub(crate) fn is_shown(&self, id: WidgetId) -> bool {
        let mut at = Some(id);
        while let Some(current) = at.filter(|w| self.is_live(*w)) {
            let node = &self.nodes[current.slot()];
            if node.style.hidden {
                return false;
            }
            at = node.parent;
        }
        self.is_live(id)
    }

    /// Whether `id` is `ancestor` or inside it.
    pub(crate) fn is_within(&self, id: WidgetId, ancestor: WidgetId) -> bool {
        let mut at = Some(id);
        while let Some(current) = at.filter(|w| self.is_live(*w)) {
            if current == ancestor {
                return true;
            }
            at = self.nodes[current.slot()].parent;
        }
        false
    }

    pub(crate) fn mark_dirty(&mut self, id: WidgetId) {
        if self.is_live(id) {
            sync(self.taffy.mark_dirty(self.nodes[id.slot()].taffy));
            self.nodes[id.slot()].dirty = true;
            self.needs_layout = true;
            self.needs_paint = true;
        }
    }

    pub(crate) fn scroll_to(&mut self, id: WidgetId, offset: Point) -> Point {
        let node = &mut self.nodes[id.slot()];
        let room = |content: f32, shown: f32| (content - shown).max(0.0);
        let max_x = room(node.content_size.width, node.rect.width());
        let max_y = room(node.content_size.height, node.rect.height());
        let clamped = Point::new(offset.x.clamp(0.0, max_x), offset.y.clamp(0.0, max_y));
        if clamped != node.scroll {
            node.scroll = clamped;
            self.needs_place = true;
            self.needs_paint = true;
        }
        clamped
    }
}

/// A tree of widgets: their layout, painting and events.
///
/// The `Ui` owns its widgets and lays them out with flexbox when their styles, contents or
/// the viewport change. Every frame, [`paint`](Self::paint) draws them onto a
/// [`Canvas`], and the window layer feeds input through [`pointer`](Self::pointer),
/// [`key`](Self::key) and [`ime`](Self::ime). Painting, hit-testing and delivering events
/// do not allocate. Coordinates are physical pixels; [`Style`] lengths are logical pixels,
/// multiplied by [`scale`](Self::scale).
pub struct Ui {
    pub(crate) widgets: Vec<Option<Box<dyn Widget>>>,
    pub(crate) state: State,
}

impl fmt::Debug for Ui {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ui")
            .field("widgets", &self.widgets.len())
            .field("viewport", &self.state.viewport)
            .field("focus", &self.state.focus)
            .finish_non_exhaustive()
    }
}

/// The widget at the root: the window's background.
struct Root;

/// The widget holding the popups: paints nothing of its own.
struct Layer;

impl Widget for Layer {}

impl Widget for Root {
    fn paint(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        canvas.fill_rect(cx.rect(), cx.theme().background);
    }
}

impl Ui {
    /// An empty interface with the dark theme at scale 1, its root a column filling the
    /// viewport. `text` shapes every widget's text.
    pub fn new(text: TextSystem) -> Self {
        let root = WidgetId {
            index: 0,
            generation: 0,
        };
        let layer = WidgetId {
            index: 1,
            generation: 0,
        };
        let style = Style::column();
        let mut taffy = TaffyTree::new();
        let node = sync(taffy.new_leaf_with_context(style.to_taffy(1.0), root));
        let layer_node = sync(taffy.new_leaf_with_context(style.to_taffy(1.0), layer));
        let top = |taffy: taffy::NodeId| Node {
            generation: 0,
            // The layer is never in the accessibility tree (its popups join the window's).
            access: 0,
            live: true,
            parent: None,
            children: Vec::new(),
            taffy,
            style: style.clone(),
            rect: Rect::default(),
            content: Rect::default(),
            content_size: Size::ZERO,
            scroll: Point::ORIGIN,
            hovered: false,
            cursor: None,
            extent: Rect::default(),
            dirty: true,
            notified: None,
        };
        let mut state = State {
            nodes: vec![top(node), top(layer_node)],
            free: Vec::new(),
            next_access: 0,
            taffy,
            root,
            layer,
            popups: Vec::new(),
            incoming: Incoming::default(),
            closing: Vec::new(),
            theme: Theme::default(),
            scale: 1.0,
            viewport: Size::ZERO,
            text,
            focus: None,
            hover: None,
            capture: None,
            requested_focus: None,
            actions: VecDeque::with_capacity(ACTION_CAPACITY),
            needs_layout: true,
            needs_paint: true,
            needs_place: false,
            ime_area: None,
            last_press: None,
            pointer: None,
        };
        for (id, taffy) in [(root, node), (layer, layer_node)] {
            let style = state.taffy_style(id);
            sync(state.taffy.set_style(taffy, style));
        }
        Self {
            widgets: vec![Some(Box::new(Root)), Some(Box::new(Layer))],
            state,
        }
    }

    /// The root: a column filling the viewport, painted with the theme's background.
    pub fn root(&self) -> WidgetId {
        self.state.root
    }

    /// Adds `widget` as the last child of `parent`, laid out by `style`.
    ///
    /// # Errors
    /// [`UiError::NoSuchWidget`] if `parent` was removed, [`UiError::TooManyWidgets`].
    pub fn add(
        &mut self,
        parent: WidgetId,
        widget: impl Widget,
        style: Style,
    ) -> Result<WidgetId, UiError> {
        let index = self.children(parent).len();
        self.insert(parent, index, widget, style)
    }

    /// Adds `widget` as child number `index` of `parent` (past the end: the last), laid out
    /// by `style`.
    ///
    /// # Errors
    /// [`UiError::NoSuchWidget`] if `parent` was removed, [`UiError::TooManyWidgets`].
    pub fn insert(
        &mut self,
        parent: WidgetId,
        index: usize,
        widget: impl Widget,
        style: Style,
    ) -> Result<WidgetId, UiError> {
        let id = self.state.new_node(parent, index, style)?;
        let slot = id.slot();
        if slot >= self.widgets.len() {
            self.widgets.resize_with(slot + 1, || None);
        }
        self.widgets[slot] = Some(Box::new(widget));
        Ok(id)
    }

    /// Removes `id` and everything inside it; `false` if it was already gone, or is the
    /// root. An open popup is closed as [`close_popup`](Self::close_popup) closes it: with
    /// the popups opened after it, focus going back.
    pub fn remove(&mut self, id: WidgetId) -> bool {
        if self.state.close_popup(id) {
            self.settle();
            return true;
        }
        self.remove_widget(id)
    }

    /// Removes `id` and everything inside it.
    pub(crate) fn remove_widget(&mut self, id: WidgetId) -> bool {
        if !self.state.is_live(id) || id == self.state.root || id == self.state.layer {
            return false;
        }
        if let Some(hover) = self.state.hover.filter(|h| self.state.is_within(*h, id)) {
            self.mark_hovered(Some(hover), false);
            self.state.hover = None;
        }
        let parent = self.state.nodes[id.slot()].parent;
        if let Some(parent) = parent {
            self.state.nodes[parent.slot()]
                .children
                .retain(|child| *child != id);
        }
        let mut doomed = vec![id];
        while let Some(gone) = doomed.pop() {
            let node = &mut self.state.nodes[gone.slot()];
            doomed.append(&mut node.children);
            node.live = false;
            node.generation = node.generation.wrapping_add(1);
            node.cursor = None;
            let taffy = node.taffy;
            sync(self.state.taffy.remove(taffy));
            self.widgets[gone.slot()] = None;
            self.state.free.push(gone.index);
        }
        if let Some(parent) = parent.filter(|p| self.state.nodes[p.slot()].children.is_empty()) {
            self.state.renew_layout_node(parent);
        }
        let state = &mut self.state;
        let live = |slot: &mut Option<WidgetId>, nodes: &[Node]| {
            if slot.is_some_and(|w| {
                !(nodes[w.slot()].live && nodes[w.slot()].generation == w.generation)
            }) {
                *slot = None;
            }
        };
        live(&mut state.focus, &state.nodes);
        live(&mut state.hover, &state.nodes);
        live(&mut state.capture, &state.nodes);
        let nodes = &state.nodes;
        state.popups.retain(|popup| {
            nodes[popup.id.slot()].live && nodes[popup.id.slot()].generation == popup.id.generation
        });
        if state.ime_area.is_some_and(|(w, _)| !state.is_live(w)) {
            state.ime_area = None;
        }
        state.needs_layout = true;
        state.needs_paint = true;
        true
    }

    /// Changes `id`'s style; `false` if it was removed.
    pub fn set_style(&mut self, id: WidgetId, style: Style) -> bool {
        self.state.set_style(id, style)
    }

    /// `id`'s style.
    pub fn style(&self, id: WidgetId) -> Option<&Style> {
        self.state.node(id).map(|node| &node.style)
    }

    /// `id`'s parent; `None` for the root or a removed widget.
    pub fn parent(&self, id: WidgetId) -> Option<WidgetId> {
        self.state.node(id).and_then(|node| node.parent)
    }

    /// `id`'s children, first painted first; empty for a removed widget.
    pub fn children(&self, id: WidgetId) -> &[WidgetId] {
        self.state.node(id).map_or(&[], |node| &node.children)
    }

    /// Where `id` is, in physical pixels from the window's top-left corner, as of the last
    /// layout.
    pub fn rect(&self, id: WidgetId) -> Option<Rect> {
        self.state.node(id).map(|node| node.rect)
    }

    /// The widget `id`, if it is a `T`.
    pub fn widget<T: Widget>(&self, id: WidgetId) -> Option<&T> {
        if !self.state.is_live(id) {
            return None;
        }
        let widget: &dyn Widget = self.widgets.get(id.slot())?.as_deref()?;
        (widget as &dyn Any).downcast_ref::<T>()
    }

    /// Runs `update` on the widget `id`, if it is a `T`, with a context to request layout,
    /// paint or focus as an event handler would.
    pub fn update<T: Widget, R>(
        &mut self,
        id: WidgetId,
        update: impl FnOnce(&mut T, &mut EventCx<'_>) -> R,
    ) -> Option<R> {
        if !self.state.is_live(id) {
            return None;
        }
        let widget: &mut dyn Widget = self.widgets.get_mut(id.slot())?.as_deref_mut()?;
        let widget = (widget as &mut dyn Any).downcast_mut::<T>()?;
        let mut cx = EventCx {
            state: &mut self.state,
            id,
        };
        let result = update(widget, &mut cx);
        self.settle();
        Some(result)
    }

    /// The theme.
    pub fn theme(&self) -> &Theme {
        &self.state.theme
    }

    /// Changes the theme; widgets are measured again (sizes may differ).
    pub fn set_theme(&mut self, theme: Theme) {
        self.state.theme = theme;
        self.restyle();
    }

    /// Physical pixels per logical pixel.
    pub fn scale(&self) -> f32 {
        self.state.scale
    }

    /// Changes the scale factor (the window moved to a screen of another density).
    pub fn set_scale(&mut self, scale: f32) {
        if scale.is_finite() && scale > 0.0 && scale != self.state.scale {
            self.state.scale = scale;
            self.restyle();
        }
    }

    /// The area the root fills, in physical pixels.
    pub fn viewport(&self) -> Size {
        self.state.viewport
    }

    /// Changes the area the root fills (the window was resized).
    pub fn resize(&mut self, viewport: Size) {
        if viewport != self.state.viewport {
            self.state.viewport = viewport;
            let root = self.state.root;
            let style = self.state.taffy_style(root);
            sync(
                self.state
                    .taffy
                    .set_style(self.state.nodes[root.slot()].taffy, style),
            );
            self.state.needs_layout = true;
            self.state.needs_paint = true;
        }
    }

    /// The text system widgets shape their text with.
    pub fn text_system(&mut self) -> &mut TextSystem {
        &mut self.state.text
    }

    /// Whether something changed since the last [`paint`](Self::paint).
    pub fn needs_paint(&self) -> bool {
        self.state.needs_paint || self.state.needs_layout || self.state.needs_place
    }

    /// The cursor to show: the one set by the widget under the pointer (or holding it), or
    /// its nearest parent that set one.
    pub fn cursor(&self) -> Cursor {
        let mut at = self.state.capture.or(self.state.hover);
        while let Some(id) = at.filter(|id| self.state.is_live(*id)) {
            let node = &self.state.nodes[id.slot()];
            if let Some(cursor) = node.cursor {
                return cursor;
            }
            at = node.parent;
        }
        Cursor::Default
    }

    /// Where the focused widget wants input method candidates, if it does.
    pub fn ime_area(&self) -> Option<Rect> {
        self.state
            .ime_area
            .filter(|(id, _)| self.state.focus == Some(*id))
            .map(|(_, area)| area)
    }

    /// The actions widgets reported and not yet read, oldest first, taken out.
    pub fn actions(&mut self) -> impl Iterator<Item = Action> + '_ {
        self.state.actions.drain(..)
    }

    /// The oldest action not yet read, taken out: lets the application act on each with the
    /// `Ui` at hand.
    pub fn next_action(&mut self) -> Option<Action> {
        self.state.actions.pop_front()
    }

    /// Lays the widgets out if anything changed. [`paint`](Self::paint) and the event
    /// methods call it first; call it to read [`rect`](Self::rect)s before painting.
    pub fn layout(&mut self) {
        if self.state.needs_layout {
            self.compute(self.state.root);
            if !self.state.nodes[self.state.layer.slot()]
                .children
                .is_empty()
            {
                self.compute(self.state.layer);
            }
            self.state.needs_layout = false;
            self.place(self.state.root, Point::ORIGIN, true, true);
            self.place(self.state.layer, Point::ORIGIN, true, true);
            self.state.needs_place = false;
            self.after_moves();
        }
        if self.state.needs_place {
            self.place(self.state.root, Point::ORIGIN, false, true);
            self.place(self.state.layer, Point::ORIGIN, false, true);
            self.state.needs_place = false;
            self.after_moves();
        }
    }

    /// Lays out the tree under `top` (the root, or the popup layer) in the viewport.
    fn compute(&mut self, top: WidgetId) {
        {
            let Self { widgets, state } = self;
            let root = state.nodes[top.slot()].taffy;
            let available = taffy::Size {
                width: AvailableSpace::Definite(state.viewport.width),
                height: AvailableSpace::Definite(state.viewport.height),
            };
            let scale = state.scale;
            let (text, theme) = (&mut state.text, &state.theme);
            let computed = state.taffy.compute_layout_with_measure(
                root,
                available,
                |inputs, _, context, style| {
                    taffy::compute_leaf_layout(
                        inputs,
                        style,
                        |_, _| 0.0,
                        |known, room| {
                            let widget =
                                context.and_then(|id| widgets.get_mut(id.slot())?.as_mut());
                            let Some(widget) = widget else {
                                return taffy::Size::ZERO;
                            };
                            let mut cx = LayoutCx {
                                text: &mut *text,
                                theme,
                                scale,
                            };
                            let size = widget.measure(&mut cx, constraints(known, room));
                            taffy::Size {
                                width: known.width.unwrap_or(size.width),
                                height: known.height.unwrap_or(size.height),
                            }
                        },
                    )
                },
            );
            sync(computed);
        }
    }

    /// After widgets moved, resized or were hidden: a hidden widget loses focus, and hover
    /// follows the widgets that are now under a pointer that did not move.
    fn after_moves(&mut self) {
        if self.state.focus.is_some_and(|f| !self.state.is_shown(f)) {
            self.set_focus(None);
        }
        if let (Some(at), None) = (self.state.pointer, self.state.capture) {
            let (hit, _) = self.hit_top(at);
            self.set_hover(hit, at);
        }
    }

    /// Computes `id`'s rectangle (and its children's) from the last layout, its parent's
    /// content starting at `origin`; `notify` tells widgets their sizes.
    fn place(&mut self, id: WidgetId, origin: Point, notify: bool, shown: bool) {
        let state = &mut self.state;
        let (taffy, parent) = (state.nodes[id.slot()].taffy, state.nodes[id.slot()].parent);
        let layout = *sync(state.taffy.layout(taffy));
        let size = Size::new(layout.size.width, layout.size.height);
        let corner = match parent.filter(|parent| *parent == state.layer) {
            // On whole pixels, as their text and borders are.
            Some(_) => state
                .popup_origin(id, size)
                .map(|at| Point::new(at.x.round(), at.y.round())),
            None => None,
        }
        .unwrap_or(Point::new(
            origin.x + layout.location.x,
            origin.y + layout.location.y,
        ));
        let node = &mut state.nodes[id.slot()];
        let rect = Rect::new(corner.x, corner.y, size.width, size.height);
        node.rect = rect;
        node.content = Rect::from_edges(
            rect.left() + layout.border.left + layout.padding.left,
            rect.top() + layout.border.top + layout.padding.top,
            rect.right() - layout.border.right - layout.padding.right,
            rect.bottom() - layout.border.bottom - layout.padding.bottom,
        );
        // Measured from the padding box's corner: how far the content reaches.
        let overflow = layout.scrollable_overflow_rect;
        node.content_size = Size::new(overflow.right, overflow.bottom);
        // Content that shrank, or a view that grew, leaves less to scroll. A hidden widget
        // has no size, so it keeps its scroll for when it is shown again (a tab switched
        // away and back).
        let shown = shown && !node.style.hidden;
        let room = |content: f32, view: f32| (content - view).max(0.0);
        let clamped = if !shown {
            node.scroll
        } else {
            Point::new(
                node.scroll.x.clamp(0.0, room(overflow.right, rect.width())),
                node.scroll
                    .y
                    .clamp(0.0, room(overflow.bottom, rect.height())),
            )
        };
        if clamped != node.scroll {
            node.scroll = clamped;
            state.needs_paint = true;
        }
        node.extent = if node.style.overflow == Overflow::Visible {
            Rect::from_edges(
                rect.left() + overflow.left.min(0.0),
                rect.top() + overflow.top.min(0.0),
                rect.right().max(rect.left() + overflow.right),
                rect.bottom().max(rect.top() + overflow.bottom),
            )
        } else {
            rect
        };
        let inner = Point::new(rect.left() - node.scroll.x, rect.top() - node.scroll.y);
        let changed = node.dirty || node.notified != Some(rect.size());
        if notify && changed {
            node.dirty = false;
            node.notified = Some(rect.size());
            if let Some(widget) = self.widgets.get_mut(id.slot()).and_then(Option::as_mut) {
                let mut cx = LayoutCx {
                    text: &mut state.text,
                    theme: &state.theme,
                    scale: state.scale,
                };
                widget.layout(&mut cx, rect.size());
            }
        }
        for index in 0..self.state.nodes[id.slot()].children.len() {
            let child = self.state.nodes[id.slot()].children[index];
            self.place(child, inner, notify, shown);
        }
    }

    /// Paints every visible widget onto `canvas`, laying them out first if needed.
    pub fn paint(&mut self, canvas: &mut Canvas<'_>) {
        self.layout();
        self.paint_node(self.state.root, canvas);
        self.paint_node(self.state.layer, canvas);
        self.state.needs_paint = false;
    }

    fn paint_node(&mut self, id: WidgetId, canvas: &mut Canvas<'_>) {
        let node = &self.state.nodes[id.slot()];
        if node.style.hidden {
            return;
        }
        let rect = node.rect;
        let clips = node.style.overflow != Overflow::Visible;
        // Nothing it or its children paint would show.
        if canvas.clip_rect().intersect(node.extent).is_empty() {
            return;
        }
        let saved = canvas.state();
        if let Some(widget) = self.widgets.get_mut(id.slot()).and_then(Option::as_mut) {
            widget.paint(
                &PaintCx {
                    state: &self.state,
                    id,
                },
                canvas,
            );
        }
        if clips {
            canvas.clip(rect);
        }
        for index in 0..self.state.nodes[id.slot()].children.len() {
            let child = self.state.nodes[id.slot()].children[index];
            self.paint_node(child, canvas);
        }
        canvas.restore(saved);
        if let Some(widget) = self.widgets.get_mut(id.slot()).and_then(Option::as_mut) {
            widget.paint_over(
                &PaintCx {
                    state: &self.state,
                    id,
                },
                canvas,
            );
        }
    }

    /// Re-styles every widget (scale or theme changed), so all are measured again.
    fn restyle(&mut self) {
        for slot in 0..self.state.nodes.len() {
            let node = &self.state.nodes[slot];
            if node.live {
                let id = WidgetId {
                    index: slot as u32,
                    generation: node.generation,
                };
                let style = self.state.taffy_style(id);
                sync(self.state.taffy.set_style(node.taffy, style));
                self.state.nodes[slot].dirty = true;
            }
        }
        self.state.needs_layout = true;
        self.state.needs_paint = true;
    }

    /// Gives focus a widget asked for while handling an event, and re-places scrolled widgets.
    pub(crate) fn settle(&mut self) {
        // Focus events can ask for focus again, or open and close popups: give up after a
        // few rounds.
        for _ in 0..4 {
            self.adopt();
            match self.state.requested_focus.take() {
                Some(focus) => self.give_focus(focus),
                None => break,
            }
        }
        // What the last focus events opened or closed.
        self.adopt();
        self.state.requested_focus = None;
    }
}

/// Layout's question in densa's terms.
fn constraints(known: taffy::Size<Option<f32>>, room: taffy::Size<AvailableSpace>) -> Constraints {
    let available = |space: AvailableSpace| match space {
        AvailableSpace::Definite(room) => Available::Definite(room),
        AvailableSpace::MinContent => Available::MinContent,
        AvailableSpace::MaxContent => Available::MaxContent,
    };
    Constraints {
        width: known.width,
        height: known.height,
        available_width: available(room.width),
        available_height: available(room.height),
    }
}
