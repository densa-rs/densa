use crate::{Instance, InstanceBuffer};
use densa_core::{Color, Point, Rect};
use densa_text::{quantize_x, Font, GlyphCache, GlyphId, GlyphRun};

/// One frame being drawn: everything painters emit goes through it.
///
/// [`Canvas::begin`] starts the frame on both the instance list and the glyph cache, so the
/// cache's "used this frame" protection spans every painter of the frame: a glyph drawn by
/// one painter cannot be evicted by the next. One glyph cache serves every painter of a
/// frame loop and its updates feed one GPU atlas (`densa_render::GpuRenderer`).
///
/// Allocation-free once the frame's glyphs are cached.
#[derive(Debug)]
pub struct Canvas<'a> {
    cache: &'a mut GlyphCache,
    out: &'a mut InstanceBuffer,
}

impl<'a> Canvas<'a> {
    /// Starts a frame: clears `out` and begins a frame on `cache`.
    pub fn begin(cache: &'a mut GlyphCache, out: &'a mut InstanceBuffer) -> Self {
        cache.begin_frame();
        out.clear();
        Self { cache, out }
    }

    /// A filled rectangle.
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.out.push(Instance::solid(rect.to_xywh(), color));
    }

    /// `glyph` of `font` at `px` pixels per em, its pen at `at`: `x` is the pen position
    /// (fractional: drawn at the nearest subpixel bin), `y` the baseline (rounded to a whole
    /// pixel).
    ///
    /// Glyphs the cache cannot provide this frame are skipped (counted in its stats).
    pub fn draw_glyph(&mut self, font: &Font, glyph: GlyphId, px: f32, at: Point, color: Color) {
        let (pixel, bin) = quantize_x(at.x);
        let Ok(cached) = self.cache.get(font, glyph, px, bin) else {
            return;
        };
        if cached.size[0] == 0.0 {
            return; // No ink, e.g. a space.
        }
        self.out.push(Instance::glyph(
            [
                pixel as f32 + cached.offset[0],
                at.y.round() + cached.offset[1],
                cached.size[0],
                cached.size[1],
            ],
            cached.atlas_rect,
            color,
        ));
    }

    /// Every glyph of `run`, offset by `origin`.
    pub fn draw_run(&mut self, run: GlyphRun<'_>, origin: Point) {
        for glyph in run.glyphs {
            let at = origin.offset(glyph.x, glyph.y);
            self.draw_glyph(run.font, glyph.id, run.px, at, run.color);
        }
    }

    /// The glyph cache this frame draws through.
    pub fn cache(&self) -> &GlyphCache {
        self.cache
    }

    /// The instances emitted so far this frame.
    pub fn instances(&self) -> &InstanceBuffer {
        self.out
    }
}
