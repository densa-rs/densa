//! Text a widget shows, shaped when needed and re-wrapped when its width changes.

use densa_core::{Color, Point, Size};
use densa_draw::Canvas;
use densa_paragraph::{Paragraph, ParagraphStyle};
use densa_ui::{Available, Constraints, LayoutCx};

#[derive(Debug)]
pub(crate) struct Text {
    content: String,
    /// Logical pixels; `None` for the theme's size.
    size: Option<f32>,
    wrap: bool,
    paragraph: Option<Paragraph>,
    /// The physical size and color it was shaped with.
    shaped: Option<(f32, Color)>,
}

impl Text {
    pub(crate) fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            size: None,
            wrap: false,
            paragraph: None,
            shaped: None,
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.content
    }

    /// Replaces the text; it is shaped again at the next layout.
    pub(crate) fn set(&mut self, content: impl Into<String>) {
        self.content = content.into();
        self.paragraph = None;
    }

    pub(crate) fn set_size(&mut self, logical: f32) {
        self.size = Some(logical);
        self.paragraph = None;
    }

    pub(crate) fn set_wrap(&mut self, wrap: bool) {
        self.wrap = wrap;
    }

    /// The paragraph in `color`, shaped again if the text, size or color changed; `None`
    /// if it cannot be shaped (a font that cannot be read).
    pub(crate) fn shape(&mut self, cx: &mut LayoutCx<'_>, color: Color) -> Option<&mut Paragraph> {
        let px = cx.px(self.size.unwrap_or(cx.theme().font_size));
        if self.paragraph.is_none() || self.shaped != Some((px, color)) {
            let style = ParagraphStyle::new(px).with_color(color);
            self.paragraph = cx.text_system().paragraph(&self.content, &style, &[]).ok();
            self.shaped = Some((px, color));
        }
        self.paragraph.as_mut()
    }

    /// The text's size within `constraints`: on one line, or wrapped at the width available
    /// if it wraps.
    pub(crate) fn measure(
        &mut self,
        cx: &mut LayoutCx<'_>,
        constraints: Constraints,
        color: Color,
    ) -> Size {
        let wrap = self.wrap;
        let Some(paragraph) = self.shape(cx, color) else {
            return Size::ZERO;
        };
        // Wrapping text is as narrow as its longest word at its smallest (every break taken),
        // and on one line at its widest.
        let width = constraints.width.or(match constraints.available_width {
            Available::Definite(room) => Some(room),
            Available::MinContent => Some(0.0),
            _ => None,
        });
        // A width layout gives is finite and not negative: reflowing cannot fail.
        let _ = paragraph.reflow(width.filter(|_| wrap));
        Size::new(paragraph.width().ceil(), paragraph.height().ceil())
    }

    /// Shapes the text if needed and wraps it at `width`.
    pub(crate) fn fit(&mut self, cx: &mut LayoutCx<'_>, width: f32, color: Color) {
        let wrap = self.wrap;
        if let Some(paragraph) = self.shape(cx, color) {
            let _ = paragraph.reflow(Some(width.max(0.0)).filter(|_| wrap));
        }
    }

    /// The text's size as last laid out.
    pub(crate) fn size(&self) -> Size {
        self.paragraph
            .as_ref()
            .map_or(Size::ZERO, |p| Size::new(p.width(), p.height()))
    }

    /// Draws the text with its top-left corner at `origin`.
    pub(crate) fn paint(&self, canvas: &mut Canvas<'_>, origin: Point) {
        if let Some(paragraph) = &self.paragraph {
            for run in paragraph.runs() {
                canvas.draw_run(run, origin);
            }
        }
    }
}
