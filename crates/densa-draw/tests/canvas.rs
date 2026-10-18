//! Canvas translation and clipping: rectangles and glyphs cut exactly, state brought back.

use densa_draw::{Canvas, Color, InstanceBuffer, Point, Rect};
use densa_text::{AtlasConfig, Font, GlyphCache};

const WHITE: Color = Color::WHITE;

fn rects(out: &InstanceBuffer) -> Vec<Rect> {
    out.as_slice()
        .iter()
        .map(|i| Rect::new(i.rect[0], i.rect[1], i.rect[2], i.rect[3]))
        .collect()
}

#[test]
fn rectangles_are_translated_and_clipped() {
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(16);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    let start = canvas.state();
    canvas.translate(100.0, 50.0);
    canvas.clip(Rect::new(0.0, 0.0, 20.0, 10.0));
    assert_eq!(canvas.clip_rect(), Rect::new(0.0, 0.0, 20.0, 10.0));
    canvas.fill_rect(Rect::new(10.0, 5.0, 30.0, 30.0), WHITE); // cut to its top-left part
    canvas.fill_rect(Rect::new(25.0, 0.0, 5.0, 5.0), WHITE); // entirely outside: skipped

    // Moving after clipping does not move the clip.
    canvas.translate(-100.0, -50.0);
    assert_eq!(canvas.clip_rect(), Rect::new(100.0, 50.0, 20.0, 10.0));
    canvas.restore(start);
    assert_eq!(canvas.clip_rect(), Rect::EVERYTHING);
    canvas.fill_rect(Rect::new(1.0, 2.0, 3.0, 4.0), WHITE); // as given, nothing applied
    assert_eq!(
        rects(&out),
        [
            Rect::from_edges(110.0, 55.0, 120.0, 60.0),
            Rect::new(1.0, 2.0, 3.0, 4.0)
        ]
    );
}

#[test]
fn nested_clips_intersect_and_an_empty_clip_hides_everything() {
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(16);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    canvas.clip(Rect::new(0.0, 0.0, 10.0, 10.0));
    canvas.clip(Rect::new(5.0, 5.0, 10.0, 10.0));
    assert_eq!(canvas.clip_rect(), Rect::from_edges(5.0, 5.0, 10.0, 10.0));
    canvas.clip(Rect::new(50.0, 50.0, 1.0, 1.0));
    assert!(canvas.clip_rect().is_empty());
    canvas.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), WHITE);
    assert!(out.is_empty());
}

#[test]
fn a_clipped_glyph_keeps_the_matching_part_of_its_atlas_cell() {
    let font = Font::bundled_mono().unwrap();
    let glyph = font.glyph_id('M').unwrap();
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(16);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    let pen = Point::new(10.0, 40.0);
    canvas.draw_glyph(&font, glyph, 32.0, pen, WHITE);
    let whole = canvas.instances().as_slice()[0];
    let (x, y, w, h) = (whole.rect[0], whole.rect[1], whole.rect[2], whole.rect[3]);
    // Keep the right half and the bottom quarter of the same glyph, drawn again.
    canvas.clip(Rect::from_edges(x + w / 2.0, y + h * 0.75, x + w, y + h));
    canvas.draw_glyph(&font, glyph, 32.0, pen, WHITE);
    let part = out.as_slice()[1];
    assert_eq!(
        part.rect,
        [x + w / 2.0, y + h * 0.75, w / 2.0, h / 4.0],
        "the visible part of the quad"
    );
    let [u0, v0, u1, v1] = whole.uv;
    let expected = [u0 + (u1 - u0) / 2.0, v0 + (v1 - v0) * 0.75, u1, v1];
    for (got, want) in part.uv.iter().zip(expected) {
        assert!((got - want).abs() < 1e-3, "{:?} vs {expected:?}", part.uv);
    }
}

#[test]
fn a_glyph_outside_the_clip_is_not_drawn() {
    let font = Font::bundled_mono().unwrap();
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(16);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    canvas.clip(Rect::new(0.0, 0.0, 5.0, 5.0));
    canvas.draw_glyph(
        &font,
        font.glyph_id('M').unwrap(),
        16.0,
        Point::new(100.0, 100.0),
        WHITE,
    );
    assert!(out.is_empty());
}

#[test]
fn glyphs_follow_the_translation() {
    let font = Font::bundled_mono().unwrap();
    let glyph = font.glyph_id('M').unwrap();
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(16);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    canvas.draw_glyph(&font, glyph, 16.0, Point::new(10.0, 40.0), WHITE);
    canvas.translate(100.0, 7.0);
    canvas.draw_glyph(&font, glyph, 16.0, Point::new(10.0, 40.0), WHITE);
    let [first, second] = [out.as_slice()[0], out.as_slice()[1]];
    assert_eq!(second.rect[0] - first.rect[0], 100.0);
    assert_eq!(second.rect[1] - first.rect[1], 7.0);
    assert_eq!(second.uv, first.uv);
}

#[test]
fn a_glyph_cut_on_its_right_and_bottom_keeps_the_matching_cell_part() {
    let font = Font::bundled_mono().unwrap();
    let glyph = font.glyph_id('W').unwrap();
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(16);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    let pen = Point::new(10.0, 40.0);
    canvas.draw_glyph(&font, glyph, 32.0, pen, WHITE);
    let whole = canvas.instances().as_slice()[0];
    let (x, y, w, h) = (whole.rect[0], whole.rect[1], whole.rect[2], whole.rect[3]);
    canvas.clip(Rect::from_edges(x, y, x + w / 4.0, y + h / 2.0));
    canvas.draw_glyph(&font, glyph, 32.0, pen, WHITE);
    let part = out.as_slice()[1];
    let [u0, v0, u1, v1] = whole.uv;
    let expected = [u0, v0, u0 + (u1 - u0) / 4.0, v0 + (v1 - v0) / 2.0];
    for (got, want) in part.uv.iter().zip(expected) {
        assert!((got - want).abs() < 1e-3, "{:?} vs {expected:?}", part.uv);
    }
}

#[test]
fn baselines_land_on_whole_pixels() {
    let font = Font::bundled_mono().unwrap();
    let glyph = font.glyph_id('x').unwrap();
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(16);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    canvas.draw_glyph(&font, glyph, 16.0, Point::new(10.0, 40.0), WHITE);
    canvas.draw_glyph(&font, glyph, 16.0, Point::new(10.0, 40.4), WHITE);
    canvas.draw_glyph(&font, glyph, 16.0, Point::new(10.0, 40.6), WHITE);
    let tops: Vec<f32> = out.as_slice().iter().map(|i| i.rect[1]).collect();
    assert_eq!(tops[0], tops[1]);
    assert_eq!(tops[2], tops[0] + 1.0);
}

#[test]
fn glyphs_far_outside_the_clip_never_reach_the_cache() {
    let font = Font::bundled_mono().unwrap();
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(16);
    {
        let mut canvas = Canvas::begin(&mut cache, &mut out);
        canvas.clip(Rect::new(0.0, 0.0, 100.0, 100.0));
        canvas.draw_glyph(
            &font,
            font.glyph_id('Q').unwrap(),
            16.0,
            Point::new(10.0, 1_000.0),
            WHITE,
        );
        // Just outside the clip, its ink could still show: it is looked up.
        canvas.draw_glyph(
            &font,
            font.glyph_id('R').unwrap(),
            16.0,
            Point::new(10.0, 110.0),
            WHITE,
        );
    }
    assert_eq!(cache.cached_glyphs(), 1);
}
