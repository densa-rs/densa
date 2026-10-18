//! Which fonts a text system draws with: the bundled font alone, or with the operating
//! system's fonts filling in the characters it lacks.

use densa_paragraph::{Paragraph, ParagraphStyle, TextSystem};
use densa_text::Font;

/// Hangul, kanji and Hanzi: none are in the bundled font.
const CJK: &str = "주문 체결 注文 订单";

fn layout(system: &mut TextSystem, text: &str) -> Paragraph {
    system
        .paragraph(text, &ParagraphStyle::new(16.0), &[])
        .unwrap()
}

/// How many glyphs are the font's missing-glyph box (glyph 0).
fn missing(p: &Paragraph) -> usize {
    p.runs()
        .flat_map(|run| run.glyphs.iter())
        .filter(|g| g.id.0 == 0)
        .count()
}

fn is_bundled(font: &Font) -> bool {
    font.id() == Font::bundled_mono().unwrap().id()
}

#[test]
fn the_bundled_system_draws_only_the_bundled_font() {
    let mut system = TextSystem::bundled().unwrap();
    let p = layout(&mut system, CJK);
    assert!(p.runs().all(|run| is_bundled(run.font)));
    assert_eq!(
        missing(&p),
        CJK.chars().filter(|c| !c.is_whitespace()).count()
    );
}

#[test]
fn latin_text_keeps_the_bundled_font_with_system_fonts_on() {
    let mut system = TextSystem::new().unwrap();
    let p = layout(&mut system, "Order filled: 1,250 @ 71,300 (Tiếng Việt)");
    assert!(p.runs().all(|run| is_bundled(run.font)));
    assert_eq!(missing(&p), 0);
}

/// Windows and macOS ship fonts for these scripts; a bare Linux machine may not.
#[cfg(all(feature = "system-fonts", any(windows, target_os = "macos")))]
#[test]
fn system_fonts_draw_the_characters_the_bundled_font_lacks() {
    let mut system = TextSystem::new().unwrap();
    let p = layout(&mut system, CJK);
    assert_eq!(missing(&p), 0);
    assert!(p.runs().any(|run| !is_bundled(run.font)));
    assert!(
        p.runs().any(|run| is_bundled(run.font)),
        "spaces stay in the bundled font"
    );
}

/// Word breaks in Japanese and Chinese need the dictionary `complex-scripts` adds; without it
/// a double-click selects one character.
#[test]
fn a_japanese_word_is_selected_whole_with_complex_scripts() {
    let mut system = TextSystem::bundled().unwrap();
    let text = "東京都に住む人";
    let p = layout(&mut system, text);
    let word = densa_paragraph::TextSelection::word_at_point(&p, densa_core::Point::new(2.0, 8.0));
    let expected = if cfg!(feature = "complex-scripts") {
        "東京"
    } else {
        "東"
    };
    assert_eq!(&text[word.range()], expected);
}
