use densa_core::{Color, Size};
use densa_draw::Canvas;
use densa_ui::{
    ActionKind, Constraints, Cursor, Event, EventCx, Handled, Key, LayoutCx, NamedKey, PaintCx,
    PointerButton, PointerKind, Theme, Widget,
};

use crate::paint::{centered, outline};
use crate::text::Text;

/// Space around the label, in logical pixels.
const PADDING_X: f32 = 12.0;
const PADDING_Y: f32 = 5.0;

/// A push button: reports [`ActionKind::Clicked`] when released over itself, or when Enter
/// or Space is pressed while it has focus.
#[derive(Debug)]
pub struct Button {
    text: Text,
    primary: bool,
}

impl Button {
    /// A button labeled `label`.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            text: Text::new(label),
            primary: false,
        }
    }

    /// The button in the accent color: the action a form or toolbar is for.
    #[must_use]
    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    /// The label.
    pub fn label(&self) -> &str {
        self.text.as_str()
    }

    /// Shows `label` instead, measured again at the next layout.
    pub fn set_label(&mut self, cx: &mut EventCx<'_>, label: impl Into<String>) {
        self.text.set(label);
        cx.request_layout();
    }

    fn text_color(&self, theme: &Theme) -> Color {
        if self.primary {
            theme.on_accent
        } else {
            theme.text
        }
    }

    fn fill(&self, cx: &PaintCx<'_>) -> Color {
        let theme = cx.theme();
        match (self.primary, cx.is_pressed(), cx.is_hovered()) {
            (true, true, _) => theme.accent.with_alpha(0.75),
            (true, false, true) => theme.accent.with_alpha(0.88),
            (true, false, false) => theme.accent,
            (false, true, _) => theme.pressed,
            (false, false, true) => theme.hover,
            (false, false, false) => theme.raised,
        }
    }
}

impl Widget for Button {
    fn measure(&mut self, cx: &mut LayoutCx<'_>, constraints: Constraints) -> Size {
        let color = self.text_color(cx.theme());
        let text = self.text.measure(cx, constraints, color);
        Size::new(
            text.width + 2.0 * cx.px(PADDING_X),
            text.height + 2.0 * cx.px(PADDING_Y),
        )
    }

    fn layout(&mut self, cx: &mut LayoutCx<'_>, size: Size) {
        let color = self.text_color(cx.theme());
        self.text.fit(cx, size.width, color);
    }

    fn paint(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        let rect = cx.rect();
        canvas.fill_rect(rect, self.fill(cx));
        if !self.primary {
            outline(canvas, rect, cx.px(1.0).round(), cx.theme().border);
        }
        if cx.is_focused() {
            outline(canvas, rect, cx.px(1.5).round(), cx.theme().focus);
        }
        let size = self.text.size();
        self.text
            .paint(canvas, centered(cx.content_rect(), size.width, size.height));
    }

    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        match event {
            Event::Pointer(pointer) => match pointer.kind {
                PointerKind::Enter => {
                    cx.set_cursor(Some(Cursor::Pointer));
                    cx.request_paint();
                    Handled::Yes
                }
                PointerKind::Leave | PointerKind::Down(PointerButton::Primary) => {
                    cx.request_paint();
                    Handled::Yes
                }
                PointerKind::Up(PointerButton::Primary) => {
                    // Only a release ending this button's own press clicks it.
                    if cx.is_pressed() && cx.rect().contains(pointer.position) {
                        cx.emit(ActionKind::Clicked);
                    }
                    cx.request_paint();
                    Handled::Yes
                }
                _ => Handled::No,
            },
            Event::Key(key)
                if key.pressed
                    && !key.repeat
                    && matches!(
                        key.key,
                        Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space)
                    ) =>
            {
                cx.emit(ActionKind::Clicked);
                Handled::Yes
            }
            Event::FocusGained | Event::FocusLost => {
                cx.request_paint();
                Handled::Yes
            }
            _ => Handled::No,
        }
    }

    fn accepts_focus(&self) -> bool {
        true
    }
}
