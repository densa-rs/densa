//! Cached character-to-glyph mapping for one font at one size.

use densa_text::{Font, GlyphMap};

fn map() -> GlyphMap {
    GlyphMap::new(Font::bundled_mono().unwrap(), 16.0)
}

#[test]
fn ascii_maps_to_the_fonts_glyph_and_advance() {
    let mut glyphs = map();
    let font = Font::bundled_mono().unwrap();
    let a = glyphs.get('A').unwrap();
    assert_eq!(a.id, font.glyph_id('A').unwrap());
    assert!((a.advance - 9.6).abs() < 0.01);
}

#[test]
fn non_ascii_characters_map_too() {
    let mut glyphs = map();
    let font = Font::bundled_mono().unwrap();
    assert_eq!(glyphs.get('ệ').unwrap().id, font.glyph_id('ệ').unwrap());
}

#[test]
fn missing_characters_fall_back_to_a_question_mark() {
    let mut glyphs = map();
    let question = glyphs.get('?').unwrap();
    assert_eq!(glyphs.get('\u{F8FF}'), Some(question));
}

#[test]
fn measure_sums_advances() {
    let mut glyphs = map();
    assert!((glyphs.measure("12.50") - 5.0 * 9.6).abs() < 0.01);
    assert_eq!(glyphs.measure(""), 0.0);
}

#[test]
fn exposes_its_font_size_and_line_metrics() {
    let glyphs = map();
    assert_eq!(glyphs.px(), 16.0);
    assert_eq!(glyphs.line_metrics(), glyphs.font().line_metrics(16.0));
}
