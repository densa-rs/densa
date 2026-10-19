use std::any::Any;

use densa_core::Size;
use densa_draw::Canvas;

use crate::{AccessNode, AccessRequest, Event, EventCx, LayoutCx, PaintCx, WidgetId};

/// A part of the interface: it can measure itself, paint, and handle events.
///
/// A [`Ui`](crate::Ui) owns its widgets in a tree; their places come from each widget's
/// [`Style`](crate::Style) and, for widgets without children, from
/// [`measure`](Self::measure). Painting and event handling run every frame, so they must
/// not allocate once a widget's content is set up; measuring and
/// [`layout`](Self::layout) run only when sizes change and may.
///
/// Every method has a default: a plain container needs to implement nothing.
pub trait Widget: Any {
    /// The size the widget wants within `constraints`, for a widget without children (one
    /// with children is sized by its style and its children). Physical pixels.
    fn measure(&mut self, cx: &mut LayoutCx<'_>, constraints: Constraints) -> Size {
        let _ = (cx, constraints);
        Size::ZERO
    }

    /// Layout decided the widget's size: re-wrap text or recompute anything that depends on
    /// it. Called after every layout pass that changed the size.
    fn layout(&mut self, cx: &mut LayoutCx<'_>, size: Size) {
        let _ = (cx, size);
    }

    /// Paints the widget below its children.
    fn paint(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        let _ = (cx, canvas);
    }

    /// Paints the widget over its children: scroll bars, focus rings, overlays.
    fn paint_over(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        let _ = (cx, canvas);
    }

    /// Handles `event`. An event the widget does not handle goes on to its parent.
    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        let _ = (cx, event);
        Handled::No
    }

    /// Whether the widget takes keyboard focus, by click or by Tab.
    fn accepts_focus(&self) -> bool {
        false
    }

    /// Whether a press on the widget, or on a widget inside it that takes no focus, focuses
    /// it (if it [accepts focus](Self::accepts_focus)). A container reachable by Tab, such
    /// as a dock's tab list, says no, so clicks on its contents do not focus it; it can still
    /// ask for focus itself.
    fn focus_on_press(&self) -> bool {
        self.accepts_focus()
    }

    /// Describes the widget to assistive technology: fills `node`, which starts as a
    /// [`Role::Group`](crate::Role::Group) with the widget's rectangle. Called only while
    /// assistive technology is listening; it may allocate.
    fn accessibility(&self, cx: &PaintCx<'_>, node: &mut AccessNode) {
        let _ = (cx, node);
    }

    /// Does what assistive technology asks (only actions the widget's node lists are asked
    /// of it). Focus and scrolling into view are done for every widget; a request the widget
    /// does not handle goes on to its parent if it is a scroll.
    fn access_action(&mut self, cx: &mut EventCx<'_>, request: &AccessRequest<'_>) -> Handled {
        let _ = (cx, request);
        Handled::No
    }
}

/// What layout asks a widget in [`Widget::measure`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Constraints {
    /// A width layout already fixed: the answer's width must be this.
    pub width: Option<f32>,
    /// A height layout already fixed: the answer's height must be this.
    pub height: Option<f32>,
    /// Room for the width.
    pub available_width: Available,
    /// Room for the height.
    pub available_height: Available,
}

/// Room along one axis.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Available {
    /// This many physical pixels.
    Definite(f32),
    /// As little as possible: the narrowest the content can be (text wrapped at every
    /// opportunity).
    MinContent,
    /// As much as the content wants (text on one line).
    MaxContent,
}

impl Available {
    /// The room as a limit, `None` for the content's own size.
    pub fn limit(self) -> Option<f32> {
        match self {
            Self::Definite(room) => Some(room),
            Self::MinContent | Self::MaxContent => None,
        }
    }
}

/// Whether a widget handled an event; one it did not goes on to the widget's parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handled {
    /// Handled: the event stops here.
    Yes,
    /// Not handled: the parent gets it.
    No,
}

impl From<bool> for Handled {
    fn from(handled: bool) -> Self {
        if handled {
            Self::Yes
        } else {
            Self::No
        }
    }
}

/// Something a widget reports to the application: a button was clicked, a text changed.
/// Read them with [`Ui::actions`](crate::Ui::actions) after delivering events.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Action {
    /// The widget that reported it.
    pub source: WidgetId,
    /// What happened.
    pub kind: ActionKind,
}

/// What a widget reports. The details are on the widget: read them through
/// [`Ui::widget`](crate::Ui::widget).
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum ActionKind {
    /// Clicked or activated with the keyboard.
    Clicked,
    /// Switched on (`true`) or off.
    Toggled(bool),
    /// Its value or text changed.
    Changed,
    /// Its value or text was confirmed (Enter in a text input).
    Submitted,
    /// An item was selected, by index.
    Selected(usize),
    /// A command was chosen (a menu item), by the number the application gave it.
    Command(u32),
    /// The user asked for a context menu on the widget: a secondary press at `at`, or the
    /// context menu key (or Shift+F10) while it has focus, `at` then its top-left corner.
    /// The widget is the one that handled the press, else the one under the pointer.
    ContextMenu {
        /// Where, in physical pixels from the window's top-left corner.
        at: densa_core::Point,
    },
}
