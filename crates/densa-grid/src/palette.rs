use densa_core::Color;

/// Grid colors (dark theme). Built once at startup.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// Window background (what `GpuRenderer::render` clears to).
    pub background: Color,
    /// Header row bar.
    pub header_background: Color,
    /// Column titles.
    pub header_text: Color,
    /// Every other data row.
    pub stripe: Color,
    /// Cell text.
    pub text: Color,
    /// Placeholders and secondary text.
    pub dim_text: Color,
    /// Good news: a value rose, a check passed.
    pub positive: Color,
    /// Bad news: a value fell, a check failed.
    pub negative: Color,
}

impl Palette {
    /// The default dark palette.
    pub fn new() -> Self {
        Self {
            background: Color::srgb(0x0D1117),
            header_background: Color::srgb(0x161B22),
            header_text: Color::srgb(0x8B949E),
            stripe: Color::srgb(0x131922),
            text: Color::srgb(0xC9D1D9),
            dim_text: Color::srgb(0x6E7681),
            positive: Color::srgb(0x3FB950),
            negative: Color::srgb(0xF85149),
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::new()
    }
}
