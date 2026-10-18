use std::ops::Range;

use densa_core::Rect;

use crate::selection::backspace_start;
use crate::style::{is_char_range, ParagraphError, ParagraphStyle};
use crate::{Affinity, Motion, Paragraph, TextSelection, TextSystem};

/// Editable text: a [`Paragraph`] with a [`TextSelection`], typing, deletion and input-method
/// composition.
///
/// Every change to the text (typing, deleting, composing, a new style) lays the text out
/// again, which allocates; moving the caret, drawing [`paragraph`](Self::paragraph) and the
/// geometry queries do not. A change that fails leaves the editor as it was.
///
/// Input methods (IME) compose text before committing it: forward their preedit to
/// [`set_composition`](Self::set_composition) (an empty preedit removes it) and the committed
/// text to [`insert`](Self::insert); underline [`for_each_composition_rect`] and report
/// [`ime_area`](Self::ime_area) to the window so the candidate list appears next to the text.
/// Typing replaces an active preedit, deleting cancels it (restoring any selection it
/// replaced), and moving the caret keeps it as ordinary text.
///
/// Not yet supported: undo and redo, styled spans, a maximum length.
///
/// [`for_each_composition_rect`]: Self::for_each_composition_rect
#[derive(Debug)]
pub struct TextEditor {
    text: String,
    style: ParagraphStyle,
    paragraph: Paragraph,
    selection: TextSelection,
    composition: Option<Composition>,
    shows_caret: bool,
}

/// Preedit text an input method is composing.
#[derive(Debug, Clone)]
struct Composition {
    /// Byte range of the preedit text.
    range: Range<usize>,
    /// The selected text the preedit replaced, put back if the input method cancels.
    replaced: String,
}

/// Characters after which the caret belongs to the next line: the hard line breaks of
/// Unicode's line breaking algorithm (UAX #14 classes BK, CR, LF and NL).
const LINE_BREAKS: [char; 7] = [
    '\n', '\r', '\u{0B}', '\u{0C}', '\u{85}', '\u{2028}', '\u{2029}',
];

impl TextEditor {
    /// An editor holding `text` set in `style`, with the caret at the end.
    ///
    /// # Errors
    /// [`ParagraphError`] if `style` is out of range or a font cannot be read.
    pub fn new(
        system: &mut TextSystem,
        text: impl Into<String>,
        style: ParagraphStyle,
    ) -> Result<Self, ParagraphError> {
        let text = text.into();
        let paragraph = system.paragraph(&text, &style, &[])?;
        let selection = TextSelection::caret_at(&paragraph, text.len());
        Ok(Self {
            text,
            style,
            paragraph,
            selection,
            composition: None,
            shows_caret: true,
        })
    }

    /// The text, including any preedit text being composed.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The style the text is set in.
    pub fn style(&self) -> &ParagraphStyle {
        &self.style
    }

    /// The laid-out text, to draw and to hit-test.
    pub fn paragraph(&self) -> &Paragraph {
        &self.paragraph
    }

    /// The caret or selected range.
    pub fn selection(&self) -> TextSelection {
        self.selection
    }

    /// The selected text (empty for a caret), for copying.
    pub fn selected_text(&self) -> &str {
        &self.text[self.selection.range()]
    }

    /// The byte range of the preedit text an input method is composing, if any.
    pub fn composition(&self) -> Option<Range<usize>> {
        self.composition.as_ref().map(|c| c.range.clone())
    }

    /// Whether to draw the caret: an input method can hide it while it composes.
    pub fn shows_caret(&self) -> bool {
        self.shows_caret
    }

    /// Selects `selection` (clamped to this text), for example from a click or a drag. A
    /// preedit being composed stays as ordinary text.
    pub fn set_selection(&mut self, selection: TextSelection) {
        self.end_composition();
        self.selection = selection.refreshed(&self.paragraph);
    }

    /// Moves the caret by `motion`, or with `extend` moves the selection's focus (shift +
    /// navigation keys). A preedit being composed stays as ordinary text.
    pub fn move_caret(&mut self, motion: Motion, extend: bool) {
        self.end_composition();
        self.selection = self.selection.moved(&self.paragraph, motion, extend);
    }

    /// Types `text` (a key press, a paste or committed input-method text): it replaces the
    /// preedit being composed if there is one, otherwise the selection, and the caret
    /// follows it.
    ///
    /// # Errors
    /// [`ParagraphError::UnreadableFont`] if a font chosen for the new text cannot be read.
    pub fn insert(&mut self, system: &mut TextSystem, text: &str) -> Result<(), ParagraphError> {
        let range = self.composition().unwrap_or_else(|| self.selection.range());
        let caret = range.start + text.len();
        // After a line break the caret belongs to the new line; otherwise it stays at the
        // end of the line it typed on, even where that line wraps.
        let affinity = if text.ends_with(LINE_BREAKS) {
            Affinity::Downstream
        } else {
            Affinity::Upstream
        };
        self.replace(system, range, text, |p| {
            let at = TextSelection::caret_with(p, caret, affinity);
            // A caret inside a character snaps to its start: typing a letter before a
            // combining accent forms one character, and the caret belongs after it.
            if at.focus() < caret {
                at.moved(p, Motion::Next, false)
            } else {
                at
            }
        })?;
        self.composition = None;
        self.shows_caret = true;
        Ok(())
    }

    /// Deletes the selection, or with a caret the text between it and where `motion` would
    /// move it: `Previous` for Backspace (one character, so a combining accent comes off its
    /// letter; a line break or emoji goes whole), `Next` for Delete, `PreviousWord` and
    /// `NextWord` with Ctrl, `LineStart` to delete to the start of the line. A preedit being
    /// composed is removed instead.
    ///
    /// # Errors
    /// [`ParagraphError::UnreadableFont`] if a font chosen for the remaining text cannot be
    /// read.
    pub fn delete(
        &mut self,
        system: &mut TextSystem,
        motion: Motion,
    ) -> Result<(), ParagraphError> {
        if self.composition.is_some() {
            return self.set_composition(system, "", None);
        }
        let range = self.deletion_range(motion);
        if range.is_empty() {
            return Ok(());
        }
        let caret = range.start;
        self.replace(system, range, "", |p| TextSelection::caret_at(p, caret))
    }

    /// Replaces the whole text with `text`, the caret at its end.
    ///
    /// # Errors
    /// [`ParagraphError::UnreadableFont`] if a font chosen for `text` cannot be read.
    pub fn set_text(&mut self, system: &mut TextSystem, text: &str) -> Result<(), ParagraphError> {
        let all = 0..self.text.len();
        let caret = text.len();
        self.replace(system, all, text, |p| TextSelection::caret_at(p, caret))?;
        self.composition = None;
        self.shows_caret = true;
        Ok(())
    }

    /// Sets the text in `style`, keeping the text and the selection.
    ///
    /// # Errors
    /// [`ParagraphError`] if `style` is out of range or a font cannot be read; the editor
    /// keeps its previous style.
    pub fn set_style(
        &mut self,
        system: &mut TextSystem,
        style: ParagraphStyle,
    ) -> Result<(), ParagraphError> {
        let paragraph = system.paragraph(&self.text, &style, &[])?;
        self.selection = self.selection.refreshed(&paragraph);
        self.paragraph = paragraph;
        self.style = style;
        Ok(())
    }

    /// Wraps the text at `max_width` pixels (`None`: only at line breaks) without shaping
    /// it again: call when the editor is resized.
    ///
    /// # Errors
    /// [`ParagraphError::InvalidWidth`] if `max_width` is negative or not finite; nothing
    /// changes.
    pub fn set_width(&mut self, max_width: Option<f32>) -> Result<(), ParagraphError> {
        self.paragraph.reflow(max_width)?;
        self.style.max_width = max_width;
        self.selection = self.selection.refreshed(&self.paragraph);
        Ok(())
    }

    /// Shows an input method's preedit `text` in place of the selection (or of the previous
    /// preedit), with the input method's `cursor` (a byte range within `text`) selected, or
    /// the caret hidden for `None`. An empty `text` removes the preedit and restores what it
    /// replaced: the selected text, selected again, or the caret where it was.
    ///
    /// # Errors
    /// [`ParagraphError::InvalidRange`] if `cursor` is reversed, outside `text` or splits a
    /// character; [`ParagraphError::UnreadableFont`] if a font cannot be read. Nothing
    /// changes on error.
    pub fn set_composition(
        &mut self,
        system: &mut TextSystem,
        text: &str,
        cursor: Option<Range<usize>>,
    ) -> Result<(), ParagraphError> {
        if let Some(cursor) = cursor.as_ref().filter(|c| !is_char_range(text, c)) {
            return Err(ParagraphError::InvalidRange(cursor.clone()));
        }
        if text.is_empty() {
            return self.cancel_composition(system);
        }
        let (range, replaced) = match &self.composition {
            Some(composition) => (composition.range.clone(), composition.replaced.clone()),
            None => (self.selection.range(), self.selected_text().to_owned()),
        };
        let start = range.start;
        let selected = cursor.clone().unwrap_or(0..0);
        self.replace(system, range, text, |p| {
            TextSelection::from_range(p, start + selected.start, start + selected.end)
        })?;
        self.composition = Some(Composition {
            range: start..start + text.len(),
            replaced,
        });
        self.shows_caret = cursor.is_some();
        Ok(())
    }

    /// Calls `f` with each rectangle covering the preedit text (one per line it spans),
    /// relative to the paragraph's top-left corner: draw an underline along their bottoms.
    pub fn for_each_composition_rect(&self, f: impl FnMut(Rect)) {
        if let Some(range) = self.composition() {
            TextSelection::from_range(&self.paragraph, range.start, range.end)
                .for_each_rect(&self.paragraph, f);
        }
    }

    /// Where an input method should show its candidate window, relative to the paragraph's
    /// top-left corner: around the preedit text while composing, otherwise the caret.
    pub fn ime_area(&self) -> Rect {
        let caret = self.selection.caret_rect(&self.paragraph, 0.0);
        let mut area: Option<Rect> = None;
        self.for_each_composition_rect(|rect| {
            area = Some(area.map_or(rect, |a| union(a, rect)));
        });
        area.unwrap_or(caret)
    }

    /// The byte range `delete(motion)` removes.
    fn deletion_range(&self, motion: Motion) -> Range<usize> {
        if !self.selection.is_caret() {
            return self.selection.range();
        }
        let caret = self.selection.focus();
        if motion == Motion::Previous {
            return backspace_start(&self.paragraph, &self.text, caret)..caret;
        }
        self.selection.moved(&self.paragraph, motion, true).range()
    }

    /// Removes the preedit text and puts back what it replaced, selected.
    fn cancel_composition(&mut self, system: &mut TextSystem) -> Result<(), ParagraphError> {
        if let Some(Composition { range, replaced }) = self.composition.clone() {
            let (start, end) = (range.start, range.start + replaced.len());
            self.replace(system, range, &replaced, |p| {
                TextSelection::from_range(p, start, end)
            })?;
        }
        self.end_composition();
        Ok(())
    }

    /// Keeps the preedit text as ordinary text.
    fn end_composition(&mut self) {
        self.composition = None;
        self.shows_caret = true;
    }

    /// Replaces `range` of the text with `with`, lays it out again and selects what
    /// `select` returns for the new layout; on error nothing changes.
    fn replace(
        &mut self,
        system: &mut TextSystem,
        range: Range<usize>,
        with: &str,
        select: impl FnOnce(&Paragraph) -> TextSelection,
    ) -> Result<(), ParagraphError> {
        let mut text = String::with_capacity(self.text.len() - range.len() + with.len());
        text.push_str(&self.text[..range.start]);
        text.push_str(with);
        text.push_str(&self.text[range.end..]);
        let paragraph = system.paragraph(&text, &self.style, &[])?;
        self.selection = select(&paragraph);
        self.paragraph = paragraph;
        self.text = text;
        Ok(())
    }
}

/// The smallest rectangle containing `a` and `b`.
fn union(a: Rect, b: Rect) -> Rect {
    Rect::from_edges(
        a.left().min(b.left()),
        a.top().min(b.top()),
        a.right().max(b.right()),
        a.bottom().max(b.bottom()),
    )
}
