/// A color in linear RGB with straight (not premultiplied) alpha: the space densa blends in.
///
/// Write colors the way designers specify them, as sRGB hex ([`Color::srgb`]); they are
/// converted to linear once, here. Render targets are sRGB, so the GPU converts back on
/// output and the hex value is what appears on screen.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Color {
    /// Red, linear, 0 to 1.
    pub r: f32,
    /// Green, linear, 0 to 1.
    pub g: f32,
    /// Blue, linear, 0 to 1.
    pub b: f32,
    /// Opacity, 0 (transparent) to 1 (opaque).
    pub a: f32,
}

impl Color {
    /// Fully transparent black (the default).
    pub const TRANSPARENT: Self = Self::linear(0.0, 0.0, 0.0, 0.0);
    /// Opaque black.
    pub const BLACK: Self = Self::linear(0.0, 0.0, 0.0, 1.0);
    /// Opaque white.
    pub const WHITE: Self = Self::linear(1.0, 1.0, 1.0, 1.0);

    /// A color from linear components.
    pub const fn linear(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// The opaque color with sRGB value `0xRRGGBB`.
    pub fn srgb(hex: u32) -> Self {
        // Lossless: each masked channel fits in a byte.
        let channel = |shift: u32| ((hex >> shift) & 0xFF) as u8;
        Self::srgb8(channel(16), channel(8), channel(0))
    }

    /// The opaque color with 8-bit sRGB channels `r`, `g`, `b`.
    pub fn srgb8(r: u8, g: u8, b: u8) -> Self {
        Self::linear(to_linear(r), to_linear(g), to_linear(b), 1.0)
    }

    /// The same color with opacity `alpha`, clamped to 0..=1 (NaN becomes 0).
    #[must_use]
    pub fn with_alpha(self, alpha: f32) -> Self {
        Self {
            a: if alpha.is_nan() {
                0.0
            } else {
                alpha.clamp(0.0, 1.0)
            },
            ..self
        }
    }

    /// The nearest sRGB value `0xRRGGBB`, ignoring alpha.
    pub fn to_srgb_hex(self) -> u32 {
        (u32::from(to_srgb8(self.r)) << 16)
            | (u32::from(to_srgb8(self.g)) << 8)
            | u32::from(to_srgb8(self.b))
    }

    /// `[r, g, b, a]`, linear.
    pub const fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

impl From<[f32; 4]> for Color {
    fn from([r, g, b, a]: [f32; 4]) -> Self {
        Self::linear(r, g, b, a)
    }
}

impl From<Color> for [f32; 4] {
    fn from(color: Color) -> Self {
        color.to_array()
    }
}

/// The sRGB transfer function, decoding an 8-bit channel.
fn to_linear(channel: u8) -> f32 {
    let c = f32::from(channel) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The sRGB transfer function, encoding a linear channel to the nearest 8-bit value.
fn to_srgb8(linear: f32) -> u8 {
    let l = linear.clamp(0.0, 1.0);
    let c = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    // Lossless: `c` is in 0..=1, so the rounded value is in 0..=255.
    (c * 255.0).round() as u8
}
