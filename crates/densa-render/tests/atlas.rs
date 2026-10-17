use densa_render::{GlyphAtlas, ATLAS_HEIGHT, ATLAS_WIDTH, GLYPH_PX};

/// Pixel of glyph `ch` at (`x`, `y`) inside its 8x8 cell.
fn glyph_pixel(atlas: &GlyphAtlas, ch: u8, x: u32, y: u32) -> u8 {
    let cell_x = u32::from(ch % 16) * GLYPH_PX;
    let cell_y = u32::from(ch / 16) * GLYPH_PX;
    atlas.pixel(cell_x + x, cell_y + y)
}

#[test]
fn atlas_is_a_16x8_grid_of_8px_glyphs() {
    let atlas = GlyphAtlas::font8x8();
    assert_eq!((ATLAS_WIDTH, ATLAS_HEIGHT), (128, 64));
    assert_eq!(atlas.pixels().len(), (ATLAS_WIDTH * ATLAS_HEIGHT) as usize);
    assert!(atlas.pixels().iter().all(|&p| p == 0 || p == 255));
}

#[test]
fn space_is_empty_and_letters_are_not() {
    let atlas = GlyphAtlas::font8x8();
    let lit = |ch: u8| {
        (0..8)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .filter(|&(x, y)| glyph_pixel(&atlas, ch, x, y) == 255)
            .count()
    };
    assert_eq!(lit(b' '), 0);
    for ch in [b'A', b'0', b'9', b'.', b',', b'#', b'-'] {
        assert!(lit(ch) > 0, "glyph {:?} is blank", ch as char);
    }
}

#[test]
fn bits_are_decoded_least_significant_first_so_glyphs_are_not_mirrored() {
    // font8x8 'A' top row is 0x0C: pixels 2 and 3 lit when bit 0 is the leftmost column.
    let atlas = GlyphAtlas::font8x8();
    let row0: Vec<u8> = (0..8).map(|x| glyph_pixel(&atlas, b'A', x, 0)).collect();
    assert_eq!(row0, [0, 0, 255, 255, 0, 0, 0, 0]);
    // '1' leans: its top-left area is lit and its bottom row is a wide base.
    let base: usize = (0..8)
        .filter(|&x| glyph_pixel(&atlas, b'1', x, 6) == 255)
        .count();
    assert!(base >= 4, "expected the wide base of '1'");
}

#[test]
fn uv_rectangles_cover_exactly_one_cell() {
    let atlas = GlyphAtlas::font8x8();
    let [u0, v0, u1, v1] = atlas.uv(b'A');
    let (w, h) = (ATLAS_WIDTH as f32, ATLAS_HEIGHT as f32);
    assert_eq!([u0 * w, v0 * h, u1 * w, v1 * h], [8.0, 32.0, 16.0, 40.0]);
}

#[test]
fn non_ascii_bytes_map_to_question_mark() {
    let atlas = GlyphAtlas::font8x8();
    assert_eq!(atlas.uv(0xE9), atlas.uv(b'?'));
}
