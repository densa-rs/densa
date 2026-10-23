//! winit's input in densa's terms.

use densa_ui::{Cursor, Ime, Key, Modifiers, NamedKey, PointerButton, ScrollDelta};
use winit::event::{Ime as WinitIme, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey as WinitNamed};
use winit::window::CursorIcon;

use crate::FileDrop;

/// The file drop `event` is, if it is one.
pub(crate) fn file_drop(event: &WindowEvent) -> Option<FileDrop> {
    Some(match event {
        WindowEvent::HoveredFile(path) => FileDrop::Hovering(path.clone()),
        WindowEvent::HoveredFileCancelled => FileDrop::Left,
        WindowEvent::DroppedFile(path) => FileDrop::Dropped(path.clone()),
        _ => return None,
    })
}

pub(crate) fn modifiers(state: ModifiersState) -> Modifiers {
    let mut modifiers = Modifiers::NONE;
    for (held, modifier) in [
        (state.shift_key(), Modifiers::SHIFT),
        (state.control_key(), Modifiers::CONTROL),
        (state.alt_key(), Modifiers::ALT),
        (state.super_key(), Modifiers::LOGO),
    ] {
        if held {
            modifiers = modifiers | modifier;
        }
    }
    modifiers
}

pub(crate) fn button(button: MouseButton) -> PointerButton {
    match button {
        MouseButton::Left => PointerButton::Primary,
        MouseButton::Right => PointerButton::Secondary,
        MouseButton::Middle => PointerButton::Middle,
        MouseButton::Back => PointerButton::Other(3),
        MouseButton::Forward => PointerButton::Other(4),
        MouseButton::Other(number) => PointerButton::Other(number),
    }
}

/// winit reports a wheel turned away from the user as positive; densa scrolls the view by the
/// delta, so that is negative (towards the top).
pub(crate) fn scroll(delta: MouseScrollDelta) -> ScrollDelta {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => ScrollDelta::Lines(-x, -y),
        MouseScrollDelta::PixelDelta(at) => ScrollDelta::Pixels(-at.x as f32, -at.y as f32),
    }
}

/// The key, or `None` for keys densa has no name for (dead keys, unidentified ones).
pub(crate) fn key(key: &WinitKey) -> Option<Key<'_>> {
    let named = match key {
        WinitKey::Character(text) => return Some(Key::Character(text.as_str())),
        WinitKey::Named(named) => named,
        _ => return None,
    };
    let named = match named {
        WinitNamed::Enter => NamedKey::Enter,
        WinitNamed::Tab => NamedKey::Tab,
        WinitNamed::Space => NamedKey::Space,
        WinitNamed::Backspace => NamedKey::Backspace,
        WinitNamed::Delete => NamedKey::Delete,
        WinitNamed::Escape => NamedKey::Escape,
        WinitNamed::ArrowLeft => NamedKey::ArrowLeft,
        WinitNamed::ArrowRight => NamedKey::ArrowRight,
        WinitNamed::ArrowUp => NamedKey::ArrowUp,
        WinitNamed::ArrowDown => NamedKey::ArrowDown,
        WinitNamed::Home => NamedKey::Home,
        WinitNamed::End => NamedKey::End,
        WinitNamed::PageUp => NamedKey::PageUp,
        WinitNamed::PageDown => NamedKey::PageDown,
        WinitNamed::Insert => NamedKey::Insert,
        WinitNamed::Shift => NamedKey::Shift,
        WinitNamed::Control => NamedKey::Control,
        WinitNamed::Alt => NamedKey::Alt,
        WinitNamed::Super => NamedKey::Super,
        WinitNamed::ContextMenu => NamedKey::ContextMenu,
        WinitNamed::F1 => NamedKey::F(1),
        WinitNamed::F2 => NamedKey::F(2),
        WinitNamed::F3 => NamedKey::F(3),
        WinitNamed::F4 => NamedKey::F(4),
        WinitNamed::F5 => NamedKey::F(5),
        WinitNamed::F6 => NamedKey::F(6),
        WinitNamed::F7 => NamedKey::F(7),
        WinitNamed::F8 => NamedKey::F(8),
        WinitNamed::F9 => NamedKey::F(9),
        WinitNamed::F10 => NamedKey::F(10),
        WinitNamed::F11 => NamedKey::F(11),
        WinitNamed::F12 => NamedKey::F(12),
        _ => return None,
    };
    Some(Key::Named(named))
}

/// The text a key types: none for control characters (Backspace's, Enter's).
pub(crate) fn text(text: Option<&str>) -> Option<&str> {
    text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
}

pub(crate) fn ime(ime: &WinitIme) -> Ime<'_> {
    match ime {
        WinitIme::Enabled => Ime::Enabled,
        WinitIme::Preedit(text, cursor) => Ime::Preedit {
            text,
            cursor: *cursor,
        },
        WinitIme::Commit(text) => Ime::Commit(text),
        WinitIme::Disabled => Ime::Disabled,
    }
}

pub(crate) fn cursor(cursor: Cursor) -> CursorIcon {
    match cursor {
        Cursor::Pointer => CursorIcon::Pointer,
        Cursor::Text => CursorIcon::Text,
        Cursor::ResizeColumn => CursorIcon::ColResize,
        Cursor::ResizeRow => CursorIcon::RowResize,
        Cursor::Move => CursorIcon::Move,
        Cursor::Grab => CursorIcon::Grab,
        Cursor::Grabbing => CursorIcon::Grabbing,
        Cursor::NotAllowed => CursorIcon::NotAllowed,
        _ => CursorIcon::Default,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn files_dragged_over_the_window_become_file_drops() {
        let path = PathBuf::from("data/trips.parquet");
        assert_eq!(
            file_drop(&WindowEvent::HoveredFile(path.clone())),
            Some(FileDrop::Hovering(path.clone()))
        );
        assert_eq!(
            file_drop(&WindowEvent::DroppedFile(path.clone())),
            Some(FileDrop::Dropped(path))
        );
        assert_eq!(
            file_drop(&WindowEvent::HoveredFileCancelled),
            Some(FileDrop::Left)
        );
        assert_eq!(file_drop(&WindowEvent::CloseRequested), None);
    }
}
