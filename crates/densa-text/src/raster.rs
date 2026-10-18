use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::MetadataProvider;
use zeno::{Command, Format, Mask, Point};

use crate::{Font, GlyphId};

/// An 8-bit coverage bitmap of one glyph, positioned relative to the pen.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GlyphBitmap {
    /// Width in pixels; 0 for glyphs without ink (spaces).
    pub width: u32,
    /// Height in pixels; 0 for glyphs without ink.
    pub height: u32,
    /// Offset from the pen position to the bitmap's left edge, in pixels.
    pub left: i32,
    /// Offset from the baseline to the bitmap's top edge, in pixels (negative = above).
    pub top: i32,
    /// Row-major coverage, `width` bytes per row, 0 (none) to 255 (full).
    pub coverage: Vec<u8>,
}

/// Rasterizes `glyph` at `px` pixels per em, shifted right by `x_offset` pixels (use a
/// fraction in `0..1` for subpixel positioning).
///
/// Glyphs without an outline, or outlines that fail to draw, yield an empty bitmap: a
/// broken glyph renders as nothing rather than taking the frame down.
pub fn rasterize(font: &Font, glyph: GlyphId, px: f32, x_offset: f32) -> GlyphBitmap {
    let Some(font_ref) = font.font_ref() else {
        return GlyphBitmap::default();
    };
    let outlines = font_ref.outline_glyphs();
    let Some(outline) = outlines.get(skrifa::GlyphId::new(glyph.0)) else {
        return GlyphBitmap::default();
    };

    let mut pen = PathPen {
        commands: Vec::new(),
        dx: x_offset,
    };
    let settings = DrawSettings::unhinted(Size::new(px), LocationRef::default());
    if outline.draw(settings, &mut pen).is_err() || pen.commands.is_empty() {
        return GlyphBitmap::default();
    }

    let (coverage, placement) = Mask::new(&pen.commands[..]).format(Format::Alpha).render();
    if placement.width == 0 || placement.height == 0 {
        return GlyphBitmap::default();
    }
    GlyphBitmap {
        width: placement.width,
        height: placement.height,
        left: placement.left,
        top: placement.top,
        coverage,
    }
}

/// Collects outline segments as zeno path commands, flipping from font space (y up) to
/// raster space (y down) and applying the subpixel shift.
struct PathPen {
    commands: Vec<Command>,
    dx: f32,
}

impl PathPen {
    fn point(&self, x: f32, y: f32) -> Point {
        Point::new(x + self.dx, -y)
    }
}

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.commands.push(Command::MoveTo(p));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.commands.push(Command::LineTo(p));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        let (c, p) = (self.point(cx0, cy0), self.point(x, y));
        self.commands.push(Command::QuadTo(c, p));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (c0, c1, p) = (self.point(cx0, cy0), self.point(cx1, cy1), self.point(x, y));
        self.commands.push(Command::CurveTo(c0, c1, p));
    }

    fn close(&mut self) {
        self.commands.push(Command::Close);
    }
}
