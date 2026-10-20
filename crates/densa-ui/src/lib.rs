//! The widget layer of densa: a retained tree of widgets, laid out with flexbox, painted
//! onto a `densa_draw::Canvas`, and driven by pointer, keyboard and input method events.
//!
//! A [`Ui`] owns the widgets. The application adds them under the [root](Ui::root) with a
//! [`Style`], then a window layer (such as `densa-window`) calls [`Ui::paint`] each frame and
//! hands it input with [`Ui::pointer`], [`Ui::key`] and [`Ui::ime`]. Widgets report what
//! the user did as [`Action`]s, read with [`Ui::actions`]. Nothing here depends on a window
//! system or a GPU, so an interface runs, and is tested, headless.
//!
//! Layout runs only when a style, a widget's content or the viewport changes; painting,
//! hit-testing and delivering events run every frame and do not allocate.
//!
//! Popups (menus, completion lists) open above every other widget with [`Ui::open_popup`],
//! placed next to an [`Anchor`] and kept inside the viewport; a press outside closes them. A
//! secondary press or the context menu key reports [`ActionKind::ContextMenu`], where an
//! application opens its context menu.
//!
//! Widgets copy and paste through the Ui's [`Clipboard`] ([`EventCx::clipboard`]): a
//! [`MemoryClipboard`] until the window gives it the system's.
//!
//! Widgets also describe themselves to assistive technology ([`Widget::accessibility`]):
//! [`Ui::access_tree`] is what a screen reader reads, and [`Ui::access_action`] does what it
//! asks. Both are platform-neutral; `densa-window` connects them to the operating system.
//!
//! ```
//! use densa_core::{Point, Size};
//! use densa_paragraph::TextSystem;
//! use densa_ui::{
//!     ActionKind, Event, EventCx, Handled, PointerButton, PointerEvent, PointerKind, Style, Ui,
//!     Widget,
//! };
//!
//! /// Reports a click when pressed.
//! struct Clickable;
//!
//! impl Widget for Clickable {
//!     fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
//!         match event {
//!             Event::Pointer(p) if matches!(p.kind, PointerKind::Down(_)) => {
//!                 cx.emit(ActionKind::Clicked);
//!                 Handled::Yes
//!             }
//!             _ => Handled::No,
//!         }
//!     }
//! }
//!
//! let mut ui = Ui::new(TextSystem::new()?);
//! ui.resize(Size::new(400.0, 300.0));
//! let button = ui.add(ui.root(), Clickable, Style::row().size(120.0, 32.0))?;
//!
//! let press = PointerKind::Down(PointerButton::Primary);
//! ui.pointer(PointerEvent::new(press, Point::new(10.0, 10.0)));
//! let clicked: Vec<_> = ui.actions().map(|action| action.source).collect();
//! assert_eq!(clicked, [button]);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod access;
mod access_tree;
mod clipboard;
mod context;
mod dispatch;
mod event;
mod id;
mod popup;
mod style;
mod theme;
mod ui;
mod widget;

pub use access::{AccessAction, AccessId, AccessNode, AccessRequest, AccessTree, Role, MAX_PART};
pub use clipboard::{Clipboard, ClipboardError, MemoryClipboard};
pub use context::{EventCx, LayoutCx, PaintCx};
pub use event::{
    Cursor, Event, Ime, Key, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent,
    PointerKind, ScrollDelta,
};
pub use id::WidgetId;
pub use popup::Anchor;
pub use style::{Align, Direction, Edges, Justify, Length, Overflow, Position, Style};
pub use theme::Theme;
pub use ui::{Ui, UiError};
pub use widget::{Action, ActionKind, Available, Constraints, Handled, Widget};
