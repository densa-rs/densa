//! Input events, in densa's own types: a window layer (such as `densa-window`) translates its
//! platform's events into these, so widgets and their tests need no window.

use std::ops::BitOr;
use std::time::Duration;

use densa_core::Point;

/// What a widget receives.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Event<'a> {
    /// The pointer (mouse, pen, touch) pressed, released, moved, scrolled, entered or left.
    Pointer(PointerEvent),
    /// A key went down or up while the widget, or a widget inside it, has focus.
    Key(KeyEvent<'a>),
    /// An input method composed or committed text for the focused widget.
    Ime(Ime<'a>),
    /// The widget became the focused one.
    FocusGained,
    /// The widget stopped being the focused one.
    FocusLost,
}

/// A pointer event.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PointerEvent {
    /// What happened.
    pub kind: PointerKind,
    /// Where, in physical pixels from the window's top-left corner.
    pub position: Point,
    /// Modifier keys held.
    pub modifiers: Modifiers,
    /// When, from any fixed start: presses close in time and place count as one multi-click.
    pub time: Duration,
    /// For a press: 1, or 2 for a double click, 3 for a triple click... Set by the
    /// [`Ui`](crate::Ui).
    pub clicks: u8,
}

impl PointerEvent {
    /// `kind` at `position`, without modifiers, at time zero.
    pub fn new(kind: PointerKind, position: Point) -> Self {
        Self {
            kind,
            position,
            modifiers: Modifiers::NONE,
            time: Duration::ZERO,
            clicks: 1,
        }
    }

    /// The event with `modifiers` held.
    #[must_use]
    pub fn with_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    /// The event at `time`.
    #[must_use]
    pub fn at(mut self, time: Duration) -> Self {
        self.time = time;
        self
    }
}

/// What a pointer did.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum PointerKind {
    /// A button was pressed.
    Down(PointerButton),
    /// A button was released.
    Up(PointerButton),
    /// The pointer moved.
    Move,
    /// A wheel or touchpad scrolled.
    Scroll(ScrollDelta),
    /// The pointer came over the widget. Widgets receive it from the [`Ui`](crate::Ui), which
    /// ignores it as input.
    Enter,
    /// The pointer left the widget. Widgets receive it from the [`Ui`](crate::Ui), which
    /// ignores it as input.
    Leave,
    /// The press the widget was holding the pointer for was cancelled (the window lost
    /// focus): forget any drag. Widgets receive it from [`Ui::cancel_pointer`].
    ///
    /// [`Ui::cancel_pointer`]: crate::Ui::cancel_pointer
    Cancel,
}

/// A pointer button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PointerButton {
    /// The main button: left on a right-handed mouse, or a touch.
    Primary,
    /// The context button: right on a right-handed mouse.
    Secondary,
    /// The wheel button.
    Middle,
    /// Any other button, by the platform's number.
    Other(u16),
}

/// How far a scroll went.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum ScrollDelta {
    /// Lines (mouse wheel notches), positive down and to the right.
    Lines(f32, f32),
    /// Physical pixels (touchpads), positive down and to the right.
    Pixels(f32, f32),
}

/// A key event.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct KeyEvent<'a> {
    /// The key, after the keyboard layout.
    pub key: Key<'a>,
    /// The text the key types, if any: what an editor inserts.
    pub text: Option<&'a str>,
    /// Down (`true`) or up.
    pub pressed: bool,
    /// Down again because the key is held.
    pub repeat: bool,
    /// Modifier keys held.
    pub modifiers: Modifiers,
}

impl<'a> KeyEvent<'a> {
    /// `key` going down, typing nothing, without modifiers.
    pub fn down(key: Key<'a>) -> Self {
        Self {
            key,
            text: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    /// `key` going up.
    pub fn up(key: Key<'a>) -> Self {
        Self {
            pressed: false,
            ..Self::down(key)
        }
    }

    /// The event typing `text`.
    #[must_use]
    pub fn with_text(mut self, text: &'a str) -> Self {
        self.text = Some(text);
        self
    }

    /// The event with `modifiers` held.
    #[must_use]
    pub fn with_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    /// The event repeated because the key is held.
    #[must_use]
    pub fn repeated(mut self) -> Self {
        self.repeat = true;
        self
    }
}

/// A key, after the keyboard layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Key<'a> {
    /// A key with a name.
    Named(NamedKey),
    /// A key producing characters, as the layout gives them (`"a"`, `"A"` with shift).
    Character(&'a str),
}

/// Keys with names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NamedKey {
    /// Enter or Return.
    Enter,
    /// Tab.
    Tab,
    /// The space bar.
    Space,
    /// Backspace.
    Backspace,
    /// Delete (forward).
    Delete,
    /// Escape.
    Escape,
    /// Left arrow.
    ArrowLeft,
    /// Right arrow.
    ArrowRight,
    /// Up arrow.
    ArrowUp,
    /// Down arrow.
    ArrowDown,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Insert.
    Insert,
    /// Shift.
    Shift,
    /// Control.
    Control,
    /// Alt (Option on macOS).
    Alt,
    /// The logo key: Windows, Command on macOS.
    Super,
    /// The context menu key.
    ContextMenu,
    /// A function key: `F(1)` is F1.
    F(u8),
}

/// Modifier keys held during an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    bits: u8,
}

impl Modifiers {
    /// None held.
    pub const NONE: Self = Self { bits: 0 };
    /// Shift.
    pub const SHIFT: Self = Self { bits: 1 };
    /// Control.
    pub const CONTROL: Self = Self { bits: 2 };
    /// Alt (Option on macOS).
    pub const ALT: Self = Self { bits: 4 };
    /// The logo key: Windows, Command on macOS.
    pub const LOGO: Self = Self { bits: 8 };

    /// Whether every modifier of `other` is held.
    pub fn contains(self, other: Self) -> bool {
        self.bits & other.bits == other.bits
    }

    /// Shift held.
    pub fn shift(self) -> bool {
        self.contains(Self::SHIFT)
    }

    /// Control held.
    pub fn control(self) -> bool {
        self.contains(Self::CONTROL)
    }

    /// Alt held.
    pub fn alt(self) -> bool {
        self.contains(Self::ALT)
    }

    /// The logo key held.
    pub fn logo(self) -> bool {
        self.contains(Self::LOGO)
    }

    /// The platform's command modifier held: Command on macOS, Control elsewhere.
    pub fn command(self) -> bool {
        if cfg!(target_os = "macos") {
            self.logo()
        } else {
            self.control()
        }
    }
}

impl BitOr for Modifiers {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self {
            bits: self.bits | other.bits,
        }
    }
}

/// Input method events: text being composed (preedit) and text committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Ime<'a> {
    /// The input method became active.
    Enabled,
    /// Text being composed, not yet part of the document; `cursor` is a byte range within it.
    /// Empty text ends the composition.
    Preedit {
        /// The composition.
        text: &'a str,
        /// The input method's cursor or selection within `text`, in bytes.
        cursor: Option<(usize, usize)>,
    },
    /// Text to insert, replacing any composition.
    Commit(&'a str),
    /// The input method became inactive.
    Disabled,
}

/// The mouse cursor a widget asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Cursor {
    /// The platform's arrow.
    #[default]
    Default,
    /// A hand, over something clickable.
    Pointer,
    /// An I-beam, over editable text.
    Text,
    /// Resizing columns: a left-right arrow.
    ResizeColumn,
    /// Resizing rows: an up-down arrow.
    ResizeRow,
    /// Moving something.
    Move,
    /// Something can be grabbed.
    Grab,
    /// Something is being dragged.
    Grabbing,
    /// The action is not allowed.
    NotAllowed,
}
