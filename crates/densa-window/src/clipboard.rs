//! The operating system's clipboard, given to the window's `Ui`.

use densa_ui::{Clipboard, ClipboardError};

/// The system clipboard, opened when first used: a Linux session can lack one, and then
/// only copying and pasting fail.
#[derive(Default)]
pub(crate) struct SystemClipboard {
    opened: Option<arboard::Clipboard>,
}

impl SystemClipboard {
    fn open(&mut self) -> Result<&mut arboard::Clipboard, ClipboardError> {
        match &mut self.opened {
            Some(clipboard) => Ok(clipboard),
            slot @ None => Ok(slot.insert(arboard::Clipboard::new().map_err(error)?)),
        }
    }
}

impl Clipboard for SystemClipboard {
    fn text(&mut self) -> Result<String, ClipboardError> {
        self.open()?.get_text().map_err(error)
    }

    fn set_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        self.open()?.set_text(text).map_err(error)
    }
}

fn error(error: arboard::Error) -> ClipboardError {
    match error {
        arboard::Error::ContentNotAvailable => ClipboardError::Empty,
        other => ClipboardError::Unavailable(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uses the real clipboard, so it runs only when asked for (`--ignored`), and puts back
    /// the text that was there.
    #[test]
    #[ignore = "writes the system clipboard"]
    fn text_goes_through_the_system_clipboard() {
        let mut clipboard = SystemClipboard::default();
        let before = clipboard.text();
        clipboard.set_text("densa clipboard check").unwrap();
        assert_eq!(clipboard.text().unwrap(), "densa clipboard check");
        if let Ok(text) = before {
            clipboard.set_text(&text).unwrap();
        }
    }
}
