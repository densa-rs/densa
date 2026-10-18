use densa_core::Color;

use crate::{Font, GlyphId};

/// A glyph placed by a text layout, relative to the layout's origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedGlyph {
    /// The glyph to draw.
    pub id: GlyphId,
    /// Pen position in pixels (may be fractional: drawn at the nearest subpixel bin).
    pub x: f32,
    /// Baseline in pixels, y down (rounded to a whole pixel when drawn).
    pub y: f32,
}

/// Consecutive glyphs sharing one font, size and color: what a layout hands a renderer.
///
/// The common currency between text layout (which produces runs) and rendering (which draws
/// them), so neither depends on the other.
#[derive(Debug, Clone, Copy)]
pub struct GlyphRun<'a> {
    /// The face the glyph ids belong to.
    pub font: &'a Font,
    /// Size in pixels per em.
    pub px: f32,
    /// Text color.
    pub color: Color,
    /// The glyphs, in drawing order.
    pub glyphs: &'a [PositionedGlyph],
}
