//! What a widget can see and do while it measures, paints or handles an event.

use densa_core::{Point, Rect, Size};
use densa_paragraph::TextSystem;

use crate::ui::State;
use crate::{Action, ActionKind, Clipboard, Cursor, Style, Theme, WidgetId};

/// What a widget gets while it measures itself or learns its size.
#[derive(Debug)]
pub struct LayoutCx<'a> {
    pub(crate) text: &'a mut TextSystem,
    pub(crate) theme: &'a Theme,
    pub(crate) scale: f32,
}

impl LayoutCx<'_> {
    /// The text system, to shape and wrap paragraphs.
    pub fn text_system(&mut self) -> &mut TextSystem {
        self.text
    }

    /// The theme.
    pub fn theme(&self) -> &Theme {
        self.theme
    }

    /// Physical pixels per logical pixel.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// `logical` pixels in physical pixels.
    pub fn px(&self, logical: f32) -> f32 {
        logical * self.scale
    }
}

/// What a widget gets while it paints.
#[derive(Debug, Clone, Copy)]
pub struct PaintCx<'a> {
    pub(crate) state: &'a State,
    pub(crate) id: WidgetId,
}

impl PaintCx<'_> {
    /// The widget being painted.
    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// Where the widget is, in physical pixels from the window's top-left corner.
    pub fn rect(&self) -> Rect {
        self.state.nodes[self.id.slot()].rect
    }

    /// The widget's rectangle inside its padding.
    pub fn content_rect(&self) -> Rect {
        self.state.nodes[self.id.slot()].content
    }

    /// How far the widget's children are scrolled.
    pub fn scroll(&self) -> Point {
        self.state.nodes[self.id.slot()].scroll
    }

    /// The size of what the widget's children cover, for scroll bars.
    pub fn content_size(&self) -> Size {
        self.state.nodes[self.id.slot()].content_size
    }

    /// Whether the pointer is over the widget or one of its children.
    pub fn is_hovered(&self) -> bool {
        self.state.nodes[self.id.slot()].hovered
    }

    /// Whether the widget has keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.state.focus == Some(self.id)
    }

    /// Whether the widget handled a pointer press that has not been released yet.
    pub fn is_pressed(&self) -> bool {
        self.state.capture == Some(self.id)
    }

    /// Whether the pointer is over the widget itself, not one of its children.
    pub fn is_hover_target(&self) -> bool {
        self.state.hover == Some(self.id)
    }

    /// The widget's children, first painted first.
    pub fn children(&self) -> &[WidgetId] {
        &self.state.nodes[self.id.slot()].children
    }

    /// Where `id` is, if it is alive.
    pub fn rect_of(&self, id: WidgetId) -> Option<Rect> {
        self.state.node(id).map(|node| node.rect)
    }

    /// The theme.
    pub fn theme(&self) -> &Theme {
        &self.state.theme
    }

    /// Physical pixels per logical pixel.
    pub fn scale(&self) -> f32 {
        self.state.scale
    }

    /// `logical` pixels in physical pixels.
    pub fn px(&self, logical: f32) -> f32 {
        logical * self.state.scale
    }
}

/// What a widget gets while it handles an event (or is updated by the application through
/// [`Ui::update`](crate::Ui::update)): its state, and requests to the [`Ui`](crate::Ui).
#[derive(Debug)]
pub struct EventCx<'a> {
    pub(crate) state: &'a mut State,
    pub(crate) id: WidgetId,
}

impl EventCx<'_> {
    /// The widget handling the event.
    pub fn id(&self) -> WidgetId {
        self.id
    }

    /// Where the widget is, in physical pixels from the window's top-left corner.
    pub fn rect(&self) -> Rect {
        self.state.nodes[self.id.slot()].rect
    }

    /// The widget's rectangle inside its padding.
    pub fn content_rect(&self) -> Rect {
        self.state.nodes[self.id.slot()].content
    }

    /// How far the widget's children are scrolled.
    pub fn scroll(&self) -> Point {
        self.state.nodes[self.id.slot()].scroll
    }

    /// The size of what the widget's children cover.
    pub fn content_size(&self) -> Size {
        self.state.nodes[self.id.slot()].content_size
    }

    /// Whether the pointer is over the widget or one of its children.
    pub fn is_hovered(&self) -> bool {
        self.state.nodes[self.id.slot()].hovered
    }

    /// Whether the widget has keyboard focus.
    pub fn is_focused(&self) -> bool {
        self.state.focus == Some(self.id)
    }

    /// Whether the widget handled a pointer press that has not been released yet.
    pub fn is_pressed(&self) -> bool {
        self.state.capture == Some(self.id)
    }

    /// The theme.
    pub fn theme(&self) -> &Theme {
        &self.state.theme
    }

    /// Physical pixels per logical pixel.
    pub fn scale(&self) -> f32 {
        self.state.scale
    }

    /// `logical` pixels in physical pixels.
    pub fn px(&self, logical: f32) -> f32 {
        logical * self.state.scale
    }

    /// The text system, to edit and re-lay out text.
    pub fn text_system(&mut self) -> &mut TextSystem {
        &mut self.state.text
    }

    /// Asks for the next frame to be painted.
    pub fn request_paint(&mut self) {
        self.state.needs_paint = true;
    }

    /// The clipboard, to copy to and paste from.
    pub fn clipboard(&mut self) -> &mut dyn Clipboard {
        self.state.clipboard.0.as_mut()
    }

    /// Asks for the widget to be measured again: its content changed size.
    pub fn request_layout(&mut self) {
        self.state.mark_dirty(self.id);
    }

    /// Asks for keyboard focus, given once the event has been handled.
    pub fn request_focus(&mut self) {
        self.state.requested_focus = Some(Some(self.id));
    }

    /// Gives keyboard focus up, if the widget has it.
    pub fn release_focus(&mut self) {
        if self.state.focus == Some(self.id) {
            self.state.requested_focus = Some(None);
        }
    }

    /// Stops receiving the pointer's moves and release after a press.
    pub fn release_pointer(&mut self) {
        if self.state.capture == Some(self.id) {
            self.state.capture = None;
        }
    }

    /// The cursor shown over the widget (and its children that set none); `None` for its
    /// parent's.
    pub fn set_cursor(&mut self, cursor: Option<Cursor>) {
        self.state.nodes[self.id.slot()].cursor = cursor;
    }

    /// Where an input method should show its candidates while the widget has focus: the
    /// caret's rectangle.
    pub fn set_ime_area(&mut self, area: Option<Rect>) {
        self.state.ime_area = area.map(|area| (self.id, area));
    }

    /// Reports `kind` to the application.
    pub fn emit(&mut self, kind: ActionKind) {
        let source = self.id;
        self.state.actions.push_back(Action { source, kind });
    }

    /// Scrolls the widget's children by `offset` from their laid-out place, clamped so the
    /// content stays in view. Returns the offset applied.
    pub fn scroll_to(&mut self, offset: Point) -> Point {
        self.state.scroll_to(self.id, offset)
    }

    /// The widget's children, first painted first.
    pub fn children(&self) -> &[WidgetId] {
        &self.state.nodes[self.id.slot()].children
    }

    /// Where `id` is, if it is alive.
    pub fn rect_of(&self, id: WidgetId) -> Option<Rect> {
        self.state.node(id).map(|node| node.rect)
    }

    /// The style of `id`, if it is alive.
    pub fn style_of(&self, id: WidgetId) -> Option<&Style> {
        self.state.node(id).map(|node| &node.style)
    }

    /// Whether keyboard focus is on `id` or a widget inside it.
    pub fn focus_within(&self, id: WidgetId) -> bool {
        self.state
            .focus
            .is_some_and(|focus| self.state.is_within(focus, id))
    }

    /// Changes the style of the widget itself or of a widget inside it (a splitter resizing
    /// its panes, tabs showing one page); `false` for any other widget.
    pub fn set_style(&mut self, id: WidgetId, style: Style) -> bool {
        self.state.is_within(id, self.id) && self.state.set_style(id, style)
    }
}
