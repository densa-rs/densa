use densa_core::Color;

/// Colors and sizes widgets share, so an application restyles every widget in one place.
/// Sizes are in logical pixels.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Theme {
    /// Behind everything: the window.
    pub background: Color,
    /// Panels and toolbars.
    pub surface: Color,
    /// Controls: buttons, inputs.
    pub raised: Color,
    /// A control under the pointer.
    pub hover: Color,
    /// A control being pressed.
    pub pressed: Color,
    /// Lines between and around things.
    pub border: Color,
    /// Text.
    pub text: Color,
    /// Secondary text: hints, labels of lesser weight.
    pub text_dim: Color,
    /// The accent: default buttons, selected items.
    pub accent: Color,
    /// Text on the accent color.
    pub on_accent: Color,
    /// Selected text's background.
    pub selection: Color,
    /// The ring around the focused control.
    pub focus: Color,
    /// Body text size.
    pub font_size: f32,
    /// The base unit of spacing.
    pub spacing: f32,
}

impl Theme {
    /// Light text on dark surfaces.
    pub fn dark() -> Self {
        Self {
            background: Color::srgb(0x0B1220),
            surface: Color::srgb(0x111A2B),
            raised: Color::srgb(0x1A2540),
            hover: Color::srgb(0x22304A),
            pressed: Color::srgb(0x2A3A5C),
            border: Color::srgb(0x22304A),
            text: Color::srgb(0xE6EDF7),
            text_dim: Color::srgb(0x9AA8BE),
            accent: Color::srgb(0x2F5BEA),
            on_accent: Color::WHITE,
            selection: Color::srgb(0x2F5BEA).with_alpha(0.45),
            focus: Color::srgb(0x6B9BFF),
            font_size: 13.0,
            spacing: 4.0,
        }
    }

    /// Dark text on light surfaces.
    pub fn light() -> Self {
        Self {
            background: Color::srgb(0xF4F6FA),
            surface: Color::srgb(0xFFFFFF),
            raised: Color::srgb(0xFFFFFF),
            hover: Color::srgb(0xEEF2F8),
            pressed: Color::srgb(0xE2E8F2),
            border: Color::srgb(0xD5DCE6),
            text: Color::srgb(0x0B1220),
            text_dim: Color::srgb(0x5B6B82),
            accent: Color::srgb(0x2F5BEA),
            on_accent: Color::WHITE,
            selection: Color::srgb(0x2F5BEA).with_alpha(0.25),
            focus: Color::srgb(0x2F5BEA),
            font_size: 13.0,
            spacing: 4.0,
        }
    }
}

impl Default for Theme {
    /// [`Theme::dark`]: data-dense screens are mostly read in dark rooms.
    fn default() -> Self {
        Self::dark()
    }
}
