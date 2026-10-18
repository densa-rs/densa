use crate::{Instance, InstanceBuffer};
use densa_core::{Color, Point, Rect};
use densa_text::{quantize_x, Font, GlyphCache, GlyphId, GlyphRun};

/// How far from its pen a glyph's ink can reach, in ems: generous, so only glyphs certainly
/// outside the clip are skipped unseen.
const GLYPH_REACH_EMS: f32 = 4.0;

/// One frame being drawn: everything painters emit goes through it.
///
/// [`Canvas::begin`] starts the frame on both the instance list and the glyph cache, so the
/// cache's "used this frame" protection spans every painter of the frame: a glyph drawn by
/// one painter cannot be evicted by the next. One glyph cache serves every painter of a
/// frame loop and its updates feed one GPU atlas (`densa_render::GpuRenderer`).
///
/// Positions go through a translation and are clipped to a rectangle (see
/// [`translate`](Self::translate) and [`clip`](Self::clip)); [`state`](Self::state) and
/// [`restore`](Self::restore) save and bring back both, as a widget does around its children.
/// Clipping happens here, on each rectangle and glyph, so a frame stays one draw call.
///
/// Allocation-free once the frame's glyphs are cached.
#[derive(Debug)]
pub struct Canvas<'a> {
    cache: &'a mut GlyphCache,
    out: &'a mut InstanceBuffer,
    state: CanvasState,
}

/// A canvas's translation and clip, saved with [`Canvas::state`] and brought back with
/// [`Canvas::restore`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasState {
    /// Added to every position drawn.
    offset: Point,
    /// Where drawing shows, in frame coordinates (after the offset).
    clip: Rect,
}

impl<'a> Canvas<'a> {
    /// Starts a frame: clears `out` and begins a frame on `cache`.
    pub fn begin(cache: &'a mut GlyphCache, out: &'a mut InstanceBuffer) -> Self {
        cache.begin_frame();
        out.clear();
        Self {
            cache,
            out,
            state: CanvasState {
                offset: Point::ORIGIN,
                clip: Rect::EVERYTHING,
            },
        }
    }

    /// The current translation and clip, to [`restore`](Self::restore) later.
    pub fn state(&self) -> CanvasState {
        self.state
    }

    /// Brings back a translation and clip saved with [`state`](Self::state).
    pub fn restore(&mut self, state: CanvasState) {
        self.state = state;
    }

    /// Moves everything drawn from now on by `dx`, `dy`.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.state.offset = self.state.offset.offset(dx, dy);
    }

    /// Draws from now on only inside `rect` (in current coordinates), within the current
    /// clip.
    pub fn clip(&mut self, rect: Rect) {
        let rect = rect.translate(self.state.offset);
        self.state.clip = self.state.clip.intersect(rect);
    }

    /// The area drawing still shows in, in current coordinates; empty when everything is
    /// clipped away.
    pub fn clip_rect(&self) -> Rect {
        let offset = self.state.offset;
        let back = Point::new(-offset.x, -offset.y);
        self.state.clip.translate(back)
    }

    /// A filled rectangle.
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let rect = rect.translate(self.state.offset).intersect(self.state.clip);
        if !rect.is_empty() {
            self.out.push(Instance::solid(rect.to_xywh(), color));
        }
    }

    /// `glyph` of `font` at `px` pixels per em, its pen at `at`: `x` is the pen position
    /// (fractional: drawn at the nearest subpixel bin), `y` the baseline (rounded to a whole
    /// pixel).
    ///
    /// Glyphs the cache cannot provide this frame are skipped (counted in its stats).
    pub fn draw_glyph(&mut self, font: &Font, glyph: GlyphId, px: f32, at: Point, color: Color) {
        let at = at.offset(self.state.offset.x, self.state.offset.y);
        // A glyph lies within a few ems of its pen: one far outside the clip is skipped before
        // the cache would rasterize and store it.
        let reach = GLYPH_REACH_EMS * px;
        let near = Rect::from_edges(at.x - reach, at.y - reach, at.x + reach, at.y + reach);
        if near.intersect(self.state.clip).is_empty() {
            return;
        }
        let (pixel, bin) = quantize_x(at.x);
        let Ok(cached) = self.cache.get(font, glyph, px, bin) else {
            return;
        };
        if cached.size[0] == 0.0 {
            return; // No ink, e.g. a space.
        }
        let quad = Rect::new(
            pixel as f32 + cached.offset[0],
            at.y.round() + cached.offset[1],
            cached.size[0],
            cached.size[1],
        );
        let shown = quad.intersect(self.state.clip);
        if shown.is_empty() {
            return;
        }
        let uv = clipped_uv(quad, shown, cached.atlas_rect);
        self.out.push(Instance::glyph(shown.to_xywh(), uv, color));
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

/// The part of atlas rectangle `uv` (`[x0, y0, x1, y1]`, mapped linearly onto `quad`) that
/// `shown`, a part of `quad`, covers.
fn clipped_uv(quad: Rect, shown: Rect, uv: [f32; 4]) -> [f32; 4] {
    if shown == quad {
        return uv;
    }
    let along = |edge: f32, from: f32, length: f32, start: f32, span: f32| {
        start + (edge - from) / length * span
    };
    let (du, dv) = (uv[2] - uv[0], uv[3] - uv[1]);
    [
        along(shown.left(), quad.left(), quad.width(), uv[0], du),
        along(shown.top(), quad.top(), quad.height(), uv[1], dv),
        along(shown.right(), quad.left(), quad.width(), uv[0], du),
        along(shown.bottom(), quad.top(), quad.height(), uv[1], dv),
    ]
}
