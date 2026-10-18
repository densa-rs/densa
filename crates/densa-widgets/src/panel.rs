use densa_core::Color;
use densa_draw::Canvas;
use densa_ui::{PaintCx, Widget};

use crate::paint::outline;

/// A container that paints a background (the theme's surface color by default) and,
/// optionally, a border. Lay out its children with its style.
#[derive(Debug, Default)]
pub struct Panel {
    color: Option<Color>,
    transparent: bool,
    border: bool,
}

impl Panel {
    /// A panel in the theme's surface color.
    pub fn new() -> Self {
        Self::default()
    }

    /// A panel that paints nothing: only groups and lays out its children.
    pub fn transparent() -> Self {
        Self {
            transparent: true,
            ..Self::default()
        }
    }

    /// The panel in `color` instead of the theme's.
    #[must_use]
    pub fn with_color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self.transparent = false;
        self
    }

    /// The panel with a border in the theme's border color.
    #[must_use]
    pub fn with_border(mut self) -> Self {
        self.border = true;
        self
    }
}

impl Widget for Panel {
    fn paint(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        if !self.transparent {
            let color = self.color.unwrap_or(cx.theme().surface);
            canvas.fill_rect(cx.rect(), color);
        }
        if self.border {
            outline(canvas, cx.rect(), cx.px(1.0).round(), cx.theme().border);
        }
    }
}
