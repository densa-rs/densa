use densa_core::{Color, Point, Rect, Size};
use densa_draw::Canvas;
use densa_paragraph::{Motion, ParagraphError, ParagraphStyle, TextEditor, TextSelection};
use densa_ui::{
    ActionKind, Constraints, Cursor, Event, EventCx, Handled, Ime, Key, KeyEvent, LayoutCx,
    NamedKey, PaintCx, PointerButton, PointerKind, Widget,
};

use crate::paint::outline;
use crate::text::Text;

/// Space between the frame and the text, in logical pixels.
const PADDING_X: f32 = 6.0;
const PADDING_Y: f32 = 5.0;
/// The width a text input asks for, in logical pixels.
const WIDTH: f32 = 180.0;

/// A single line of editable text. Reports [`ActionKind::Changed`] after every edit and
/// [`ActionKind::Submitted`] on Enter.
///
/// Clicks place the caret (two select a word, three everything), dragging selects, the arrow
/// keys and Home/End move (with Shift: select; with Ctrl, or Cmd on macOS: by word),
/// Backspace and Delete remove characters or words, and input methods compose in place.
#[derive(Debug)]
pub struct TextInput {
    /// The text until the editor exists (it is created at the first layout).
    initial: String,
    placeholder: Text,
    editor: Option<TextEditor>,
    /// The size and color the editor's text was shaped with.
    shaped: Option<(f32, Color)>,
    /// How far the text is scrolled left to keep the caret in view.
    scroll: f32,
}

impl TextInput {
    /// An input holding `text`.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            initial: text.into(),
            placeholder: Text::new(""),
            editor: None,
            shaped: None,
            scroll: 0.0,
        }
    }

    /// The input showing `placeholder`, dimmed, while it is empty.
    #[must_use]
    pub fn with_placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Text::new(placeholder);
        self
    }

    /// The text.
    pub fn text(&self) -> &str {
        self.editor
            .as_ref()
            .map_or(self.initial.as_str(), TextEditor::text)
    }

    /// Replaces the text, the caret at its end.
    ///
    /// # Errors
    /// [`ParagraphError::UnreadableFont`] if a font for `text` cannot be read; nothing
    /// changes.
    pub fn set_text(
        &mut self,
        cx: &mut EventCx<'_>,
        text: impl Into<String>,
    ) -> Result<(), ParagraphError> {
        let text = text.into();
        match &mut self.editor {
            Some(editor) => editor.set_text(cx.text_system(), &text)?,
            None => self.initial = text,
        }
        self.follow_caret(cx);
        Ok(())
    }

    /// The editor, created or restyled for the theme's text size and color.
    fn prepare(&mut self, cx: &mut LayoutCx<'_>) {
        let px = cx.px(cx.theme().font_size);
        let color = cx.theme().text;
        if self.shaped == Some((px, color)) && self.editor.is_some() {
            return;
        }
        let style = ParagraphStyle::new(px).with_color(color);
        match &mut self.editor {
            Some(editor) => {
                if editor.set_style(cx.text_system(), style).is_err() {
                    return;
                }
            }
            None => {
                let text = std::mem::take(&mut self.initial);
                match TextEditor::new(cx.text_system(), text.clone(), style) {
                    Ok(editor) => self.editor = Some(editor),
                    Err(_) => {
                        self.initial = text;
                        return;
                    }
                }
            }
        }
        self.shaped = Some((px, color));
    }

    /// The rectangle the text is shown in: inside the frame's padding.
    fn inner(content: Rect, scale: f32) -> Rect {
        let pad = PADDING_X * scale;
        Rect::from_edges(
            content.left() + pad,
            content.top(),
            content.right() - pad,
            content.bottom(),
        )
    }

    /// Where the paragraph's top-left corner is drawn.
    fn origin(&self, inner: Rect) -> Point {
        let height = self.editor.as_ref().map_or(0.0, |e| e.paragraph().height());
        Point::new(
            inner.left() - self.scroll,
            (inner.top() + (inner.height() - height) / 2.0).round(),
        )
    }

    /// Scrolls so the caret is in view, and tells the input method where it is.
    fn follow_caret(&mut self, cx: &mut EventCx<'_>) {
        let Some(editor) = &self.editor else {
            return;
        };
        let inner = Self::inner(cx.content_rect(), cx.scale());
        let paragraph = editor.paragraph();
        let caret = editor.selection().caret_rect(paragraph, cx.px(1.0));
        let room = inner.width().max(0.0);
        if caret.left() < self.scroll {
            self.scroll = caret.left();
        } else if caret.right() > self.scroll + room {
            self.scroll = caret.right() - room;
        }
        let longest = (paragraph.width() + caret.width() - room).max(0.0);
        self.scroll = self.scroll.clamp(0.0, longest);
        let origin = self.origin(inner);
        cx.set_ime_area(Some(editor.ime_area().translate(origin)));
        cx.request_paint();
    }

    fn pointer(
        &mut self,
        cx: &mut EventCx<'_>,
        kind: PointerKind,
        at: Point,
        clicks: u8,
        shift: bool,
    ) -> Handled {
        let inner = Self::inner(cx.content_rect(), cx.scale());
        let origin = self.origin(inner);
        let Some(editor) = &mut self.editor else {
            return Handled::No;
        };
        let local = Point::new(at.x - origin.x, at.y - origin.y);
        let paragraph = editor.paragraph();
        let selection = match kind {
            PointerKind::Down(PointerButton::Primary) => match clicks {
                1 if shift => editor.selection().shift_click(paragraph, local),
                1 => TextSelection::at_point(paragraph, local),
                2 => TextSelection::word_at_point(paragraph, local),
                _ => TextSelection::select_all(paragraph),
            },
            PointerKind::Move if cx.is_pressed() => {
                editor.selection().extend_to_point(paragraph, local)
            }
            _ => return Handled::No,
        };
        editor.set_selection(selection);
        self.follow_caret(cx);
        Handled::Yes
    }

    fn key(&mut self, cx: &mut EventCx<'_>, key: &KeyEvent<'_>) -> Handled {
        let Some(editor) = &mut self.editor else {
            return Handled::No;
        };
        let (command, shift) = (key.modifiers.command(), key.modifiers.shift());
        let by_word = |word: Motion, single: Motion| if command { word } else { single };
        // Ctrl+Alt is AltGr on Windows: it types (@, €) rather than commands.
        let types = !command || key.modifiers.alt();
        let edited = match key.key {
            Key::Named(NamedKey::Backspace) => editor.delete(
                cx.text_system(),
                by_word(Motion::PreviousWord, Motion::Previous),
            ),
            Key::Named(NamedKey::Delete) => {
                editor.delete(cx.text_system(), by_word(Motion::NextWord, Motion::Next))
            }
            Key::Named(named) => {
                let motion = match named {
                    NamedKey::ArrowLeft => by_word(Motion::WordLeft, Motion::Left),
                    NamedKey::ArrowRight => by_word(Motion::WordRight, Motion::Right),
                    NamedKey::Home => Motion::LineStart,
                    NamedKey::End => Motion::LineEnd,
                    NamedKey::Enter => {
                        cx.emit(ActionKind::Submitted);
                        return Handled::Yes;
                    }
                    // Space and other named keys that type something.
                    _ => match key.text.filter(|_| types) {
                        Some(text) => {
                            let edited = editor.insert(cx.text_system(), text);
                            if edited.is_ok() {
                                cx.emit(ActionKind::Changed);
                            }
                            self.follow_caret(cx);
                            return Handled::Yes;
                        }
                        None => return Handled::No,
                    },
                };
                editor.move_caret(motion, shift);
                self.follow_caret(cx);
                return Handled::Yes;
            }
            Key::Character(c) if command && c.eq_ignore_ascii_case("a") => {
                let all = TextSelection::select_all(editor.paragraph());
                editor.set_selection(all);
                self.follow_caret(cx);
                return Handled::Yes;
            }
            _ => match key.text.filter(|_| types) {
                Some(text) => editor.insert(cx.text_system(), text),
                None => return Handled::No,
            },
        };
        if edited.is_ok() {
            cx.emit(ActionKind::Changed);
        }
        self.follow_caret(cx);
        Handled::Yes
    }

    fn ime(&mut self, cx: &mut EventCx<'_>, ime: &Ime<'_>) -> Handled {
        let Some(editor) = &mut self.editor else {
            return Handled::No;
        };
        // A composition is not part of the text yet: only a commit changes it. A composition
        // the editor refuses (a cursor inside a character) leaves it as it was.
        let changed = match *ime {
            Ime::Preedit { text, cursor } => {
                let cursor = cursor.map(|(start, end)| start..end);
                let _ = editor.set_composition(cx.text_system(), text, cursor);
                false
            }
            Ime::Commit(text) => editor.insert(cx.text_system(), text).is_ok(),
            Ime::Disabled => {
                let _ = editor.set_composition(cx.text_system(), "", None);
                false
            }
            _ => false,
        };
        if changed {
            cx.emit(ActionKind::Changed);
        }
        self.follow_caret(cx);
        Handled::Yes
    }
}

impl Widget for TextInput {
    fn measure(&mut self, cx: &mut LayoutCx<'_>, _: Constraints) -> Size {
        self.prepare(cx);
        let line = self
            .editor
            .as_ref()
            .map_or(0.0, |e| e.paragraph().height())
            .max(cx.px(cx.theme().font_size) * 1.2);
        Size::new(cx.px(WIDTH), (line + 2.0 * cx.px(PADDING_Y)).ceil())
    }

    fn layout(&mut self, cx: &mut LayoutCx<'_>, size: Size) {
        self.prepare(cx);
        // A wider input, or shorter text, leaves less to scroll.
        if let Some(editor) = &self.editor {
            let room = (size.width - 2.0 * cx.px(PADDING_X)).max(0.0);
            let longest = (editor.paragraph().width() + cx.px(1.0) - room).max(0.0);
            self.scroll = self.scroll.clamp(0.0, longest);
        }
        let dim = cx.theme().text_dim;
        self.placeholder.fit(cx, f32::INFINITY, dim);
    }

    fn paint(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        let theme = cx.theme();
        let rect = cx.rect();
        canvas.fill_rect(rect, theme.raised);
        let border = if cx.is_focused() {
            theme.focus
        } else {
            theme.border
        };
        outline(canvas, rect, cx.px(1.0).round(), border);
        let inner = Self::inner(cx.content_rect(), cx.scale());
        let origin = self.origin(inner);
        let Some(editor) = &self.editor else {
            return;
        };
        let saved = canvas.state();
        canvas.clip(inner);
        if editor.text().is_empty() && editor.composition().is_none() {
            let size = self.placeholder.size();
            let y = (inner.top() + (inner.height() - size.height) / 2.0).round();
            self.placeholder.paint(canvas, Point::new(inner.left(), y));
        }
        let paragraph = editor.paragraph();
        let selection = editor.selection();
        if cx.is_focused() && !selection.is_caret() {
            selection.for_each_rect(paragraph, |r| {
                canvas.fill_rect(r.translate(origin), theme.selection);
            });
        }
        let underline = cx.px(1.0).round();
        editor.for_each_composition_rect(|r| {
            let r = r.translate(origin);
            canvas.fill_rect(
                Rect::from_edges(r.left(), r.bottom() - underline, r.right(), r.bottom()),
                theme.text,
            );
        });
        for run in paragraph.runs() {
            canvas.draw_run(run, origin);
        }
        if cx.is_focused() && editor.shows_caret() {
            let caret = selection.caret_rect(paragraph, cx.px(1.0).round());
            canvas.fill_rect(caret.translate(origin), theme.text);
        }
        canvas.restore(saved);
    }

    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        match event {
            Event::Pointer(pointer) => match pointer.kind {
                PointerKind::Enter => {
                    cx.set_cursor(Some(Cursor::Text));
                    Handled::Yes
                }
                PointerKind::Up(PointerButton::Primary) => Handled::Yes,
                kind => {
                    let shift = pointer.modifiers.shift();
                    self.pointer(cx, kind, pointer.position, pointer.clicks, shift)
                }
            },
            Event::Key(key) if key.pressed => self.key(cx, key),
            Event::Ime(ime) => self.ime(cx, ime),
            Event::FocusGained | Event::FocusLost => {
                self.follow_caret(cx);
                Handled::Yes
            }
            _ => Handled::No,
        }
    }

    fn accepts_focus(&self) -> bool {
        true
    }
}
