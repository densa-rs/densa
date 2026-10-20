//! Text copied and pasted between widgets and other applications.

use std::fmt;

/// Where copied text goes and pasted text comes from. A [`Ui`](crate::Ui) starts with a
/// [`MemoryClipboard`], shared by its widgets only; a window gives it the system's
/// ([`Ui::set_clipboard`](crate::Ui::set_clipboard)).
pub trait Clipboard {
    /// The text on the clipboard.
    ///
    /// # Errors
    /// [`ClipboardError::Empty`] if it holds no text; [`ClipboardError::Unavailable`] if it
    /// cannot be read.
    fn text(&mut self) -> Result<String, ClipboardError>;

    /// Puts `text` on the clipboard, replacing what it held.
    ///
    /// # Errors
    /// [`ClipboardError::Unavailable`] if it cannot be written: what it held stays.
    fn set_text(&mut self, text: &str) -> Result<(), ClipboardError>;
}

/// Why the clipboard gave no text, or took none.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClipboardError {
    /// It holds no text (nothing, or only an image or files).
    Empty,
    /// The system would not let it be used, with its reason.
    Unavailable(String),
}

impl fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("the clipboard holds no text"),
            Self::Unavailable(reason) => write!(f, "the clipboard is unavailable: {reason}"),
        }
    }
}

impl std::error::Error for ClipboardError {}

/// A clipboard in memory, seen by this process only: for tests and windowless interfaces.
#[derive(Debug, Clone, Default)]
pub struct MemoryClipboard {
    text: Option<String>,
}

impl Clipboard for MemoryClipboard {
    fn text(&mut self) -> Result<String, ClipboardError> {
        self.text.clone().ok_or(ClipboardError::Empty)
    }

    fn set_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        self.text = Some(text.to_owned());
        Ok(())
    }
}

/// The clipboard a [`Ui`](crate::Ui) holds.
pub(crate) struct Held(pub(crate) Box<dyn Clipboard>);

impl Default for Held {
    fn default() -> Self {
        Self(Box::new(MemoryClipboard::default()))
    }
}

impl fmt::Debug for Held {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Clipboard")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memory_clipboard_gives_back_the_last_text_put_on_it() {
        let mut clipboard = MemoryClipboard::default();
        assert_eq!(clipboard.text(), Err(ClipboardError::Empty));
        clipboard.set_text("one").unwrap();
        clipboard.set_text("two").unwrap();
        assert_eq!(clipboard.text().unwrap(), "two");
        assert_eq!(clipboard.text().unwrap(), "two", "reading leaves it there");
    }

    #[test]
    fn errors_say_what_went_wrong() {
        assert_eq!(
            ClipboardError::Empty.to_string(),
            "the clipboard holds no text"
        );
        let busy = ClipboardError::Unavailable("busy".into());
        assert_eq!(busy.to_string(), "the clipboard is unavailable: busy");
    }
}
