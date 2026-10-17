/// `0xRRGGBB` sRGB plus alpha, converted to the linear RGBA the shader blends in.
///
/// Render targets are sRGB-format views, so the GPU re-encodes on write: colors written
/// here as their familiar hex values come out on screen exactly as specified.
pub fn srgb(hex: u32, alpha: f32) -> [f32; 4] {
    let channel = |shift: u32| {
        let c = ((hex >> shift) & 0xFF) as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    [channel(16), channel(8), channel(0), alpha]
}

/// Grid colors (dark theme), in linear space. Built once at startup.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub background: [f32; 4],
    pub header_background: [f32; 4],
    pub header_text: [f32; 4],
    pub stripe: [f32; 4],
    pub text: [f32; 4],
    /// Placeholders such as `-` for a symbol without quotes yet.
    pub dim_text: [f32; 4],
    pub up: [f32; 4],
    pub down: [f32; 4],
}

impl Palette {
    /// Opacity of a row flash at the moment the price changes.
    pub const FLASH_MAX_ALPHA: f32 = 0.25;

    pub fn new() -> Self {
        Self {
            background: srgb(0x0D1117, 1.0),
            header_background: srgb(0x161B22, 1.0),
            header_text: srgb(0x8B949E, 1.0),
            stripe: srgb(0x131922, 1.0),
            text: srgb(0xC9D1D9, 1.0),
            dim_text: srgb(0x6E7681, 1.0),
            up: srgb(0x3FB950, 1.0),
            down: srgb(0xF85149, 1.0),
        }
    }

    /// The background as a render-pass clear color.
    pub fn clear_color(&self) -> wgpu::Color {
        let [r, g, b, a] = self.background;
        wgpu::Color {
            r: f64::from(r),
            g: f64::from(g),
            b: f64::from(b),
            a: f64::from(a),
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::new()
    }
}
