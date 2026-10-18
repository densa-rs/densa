//! Glyph rasterization: shape, orientation, placement and subpixel offsets.

use densa_text::{rasterize, Font, GlyphBitmap};

const PX: f32 = 32.0;

fn glyph(ch: char, x_offset: f32) -> GlyphBitmap {
    let font = Font::bundled_mono().unwrap();
    rasterize(&font, font.glyph_id(ch).unwrap(), PX, x_offset)
}

fn ink(bitmap: &GlyphBitmap, x: u32, y: u32) -> u8 {
    bitmap.coverage[(y * bitmap.width + x) as usize]
}

fn row_ink(bitmap: &GlyphBitmap, y: u32) -> u32 {
    (0..bitmap.width)
        .map(|x| u32::from(ink(bitmap, x, y)))
        .sum()
}

/// Ink-weighted mean column.
fn centroid_x(bitmap: &GlyphBitmap) -> f32 {
    let (mut sum, mut weight) = (0.0, 0.0);
    for y in 0..bitmap.height {
        for x in 0..bitmap.width {
            let w = f32::from(ink(bitmap, x, y));
            sum += (bitmap.left as f32 + x as f32 + 0.5) * w;
            weight += w;
        }
    }
    sum / weight
}

#[test]
fn visible_glyphs_have_antialiased_ink() {
    let a = glyph('A', 0.0);
    assert!(a.width > 0 && a.height > 0);
    assert_eq!(a.coverage.len(), (a.width * a.height) as usize);
    assert!(a.coverage.contains(&255), "solid interior");
    assert!(
        a.coverage.iter().any(|&c| c > 0 && c < 255),
        "antialiased edges"
    );
    assert!(a.width <= PX as u32 + 2 && a.height <= PX as u32 + 2);
}

#[test]
fn whitespace_has_an_empty_bitmap() {
    let space = glyph(' ', 0.0);
    assert_eq!((space.width, space.height), (0, 0));
    assert!(space.coverage.is_empty());
}

#[test]
fn glyphs_are_upright_not_flipped() {
    // 'T': the crossbar is at the top, the stem alone at the bottom. Compare the inkiest
    // row of each third: the very first row is only a partial (antialiased) crossbar edge.
    let t = glyph('T', 0.0);
    let third = t.height / 3;
    let top = (0..third).map(|y| row_ink(&t, y)).max().unwrap();
    let bottom = (t.height - third..t.height)
        .map(|y| row_ink(&t, y))
        .max()
        .unwrap();
    assert!(top > 3 * bottom, "top third {top} vs bottom third {bottom}");
}

#[test]
fn capitals_sit_on_the_baseline() {
    // `top` is the offset from the baseline to the bitmap's first row (negative = above).
    let h = glyph('H', 0.0);
    assert!(h.top < 0, "cap top above the baseline, got {}", h.top);
    let bottom = h.top + h.height as i32;
    assert!(
        bottom.abs() <= 1,
        "cap bottom on the baseline, got {bottom}"
    );
}

#[test]
fn subpixel_offset_moves_the_ink_right() {
    let at_0 = centroid_x(&glyph('|', 0.0));
    let at_half = centroid_x(&glyph('|', 0.5));
    let shift = at_half - at_0;
    assert!((shift - 0.5).abs() < 0.15, "expected ~0.5 px, got {shift}");
}
