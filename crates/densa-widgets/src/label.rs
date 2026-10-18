use densa_core::{Color, Point, Size};
use densa_draw::Canvas;
use densa_ui::{AccessNode, Constraints, EventCx, LayoutCx, PaintCx, Role, Theme, Widget};

use crate::text::Text;

/// Text, on one line or wrapped to its width, vertically centered in its rectangle.
#[derive(Debug)]
pub struct Label {
    text: Text,
    dim: bool,
    centered: bool,
}

impl Label {
    /// `text` on one line, in the theme's text color and size.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: Text::new(text),
            dim: false,
            centered: false,
        }
    }

    /// The label in the theme's secondary text color.
    #[must_use]
    pub fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    /// The label at `size` logical pixels instead of the theme's size.
    #[must_use]
    pub fn with_size(mut self, size: f32) -> Self {
        self.text.set_size(size);
        self
    }

    /// The label wrapped onto more lines at the width layout gives it.
    #[must_use]
    pub fn wrapping(mut self) -> Self {
        self.text.set_wrap(true);
        self
    }

    /// The label centered horizontally too.
    #[must_use]
    pub fn centered(mut self) -> Self {
        self.centered = true;
        self
    }

    /// The text shown.
    pub fn text(&self) -> &str {
        self.text.as_str()
    }

    /// Shows `text` instead, measured again at the next layout.
    pub fn set_text(&mut self, cx: &mut EventCx<'_>, text: impl Into<String>) {
        self.text.set(text);
        cx.request_layout();
    }

    fn color(&self, theme: &Theme) -> Color {
        if self.dim {
            theme.text_dim
        } else {
            theme.text
        }
    }
}

impl Widget for Label {
    fn accessibility(&self, _: &PaintCx<'_>, node: &mut AccessNode) {
        node.set_role(Role::Label);
        node.set_value(self.text.as_str());
    }

    fn measure(&mut self, cx: &mut LayoutCx<'_>, constraints: Constraints) -> Size {
        let color = self.color(cx.theme());
        self.text.measure(cx, constraints, color)
    }

    fn layout(&mut self, cx: &mut LayoutCx<'_>, size: Size) {
        let color = self.color(cx.theme());
        self.text.fit(cx, size.width, color);
    }

    fn paint(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        let content = cx.content_rect();
        let size = self.text.size();
        let mut origin = crate::paint::centered(content, size.width, size.height);
        if !self.centered {
            origin = Point::new(content.left(), origin.y);
        }
        self.text.paint(canvas, origin);
    }
}
