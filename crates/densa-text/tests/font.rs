//! Font loading, character mapping and metrics of the bundled JetBrains Mono.

use densa_text::{Font, FontError};

fn mono() -> Font {
    Font::bundled_mono().expect("bundled font parses")
}

#[test]
fn maps_ascii_characters_to_glyphs() {
    let font = mono();
    assert!(font.glyph_id('A').is_some());
    assert!(font.glyph_id('0').is_some());
}

#[test]
fn covers_precomposed_vietnamese() {
    let font = mono();
    for ch in "ệĐữẫơưằỹ".chars() {
        assert!(font.glyph_id(ch).is_some(), "missing glyph for {ch:?}");
    }
}

#[test]
fn unmapped_characters_have_no_glyph() {
    // A private-use code point no text font maps.
    assert_eq!(mono().glyph_id('\u{F8FF}'), None);
}

#[test]
fn every_glyph_has_the_same_advance_in_a_monospace_font() {
    let font = mono();
    let advance = |ch| font.advance(font.glyph_id(ch).unwrap(), 16.0);
    let zero = advance('0');
    assert!(
        (zero - 9.6).abs() < 0.01,
        "600/1000 em at 16 px, got {zero}"
    );
    assert_eq!(advance('W'), zero);
    assert_eq!(advance('ệ'), zero);
}

#[test]
fn line_metrics_scale_with_size() {
    let font = mono();
    let small = font.line_metrics(10.0);
    let large = font.line_metrics(20.0);
    assert!(small.ascent > 0.0 && small.descent < 0.0, "{small:?}");
    assert!((large.ascent - 2.0 * small.ascent).abs() < 0.01);
    assert!(
        large.line_height() > 20.0,
        "line height must exceed the em size"
    );
}

#[test]
fn fonts_get_distinct_ids() {
    assert_ne!(mono().id(), mono().id());
}

#[test]
fn rejects_data_that_is_not_a_font() {
    let err = Font::new(b"definitely not a font".to_vec()).unwrap_err();
    assert_eq!(err, FontError::Parse);
    assert_eq!(
        err.to_string(),
        "the data is not a readable OpenType/TrueType font"
    );
}
