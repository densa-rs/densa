//! Paragraph layout: lines, wrapping, reflow, alignment, spans, scripts, validation.

use densa_paragraph::{
    Color, Paragraph, ParagraphError, ParagraphStyle, Span, TextAlign, TextSystem,
};
use densa_text::{Font, GlyphId};

const PX: f32 = 14.0;
/// JetBrains Mono advances 0.6 em.
const ADVANCE: f32 = 0.6 * PX;

fn system() -> TextSystem {
    TextSystem::new().unwrap()
}

fn layout(text: &str, style: ParagraphStyle) -> Paragraph {
    system().paragraph(text, &style, &[]).unwrap()
}

fn style() -> ParagraphStyle {
    ParagraphStyle::new(PX)
}

/// Every glyph id, in drawing order.
fn glyph_ids(p: &Paragraph) -> Vec<GlyphId> {
    p.runs()
        .flat_map(|r| r.glyphs.iter().map(|g| g.id))
        .collect()
}

mod lines {
    use super::*;

    #[test]
    fn text_without_a_width_stays_on_one_line() {
        let p = layout("Hello world", style());
        assert_eq!(p.lines().len(), 1);
        assert_eq!(p.glyph_count(), 11);
        assert!((p.width() - 11.0 * ADVANCE).abs() < 0.01, "{}", p.width());
    }

    #[test]
    fn newlines_start_new_lines() {
        assert_eq!(layout("one\ntwo\nthree", style()).lines().len(), 3);
    }

    #[test]
    fn baselines_are_whole_pixels_and_lines_stack_downwards() {
        let p = layout("a\nb\nc", style());
        let lines = p.lines();
        for line in lines {
            assert_eq!(line.baseline, line.baseline.round(), "{line:?}");
            assert!(
                line.top < line.baseline && line.baseline < line.bottom,
                "{line:?}"
            );
        }
        assert!(lines.windows(2).all(|w| w[0].baseline < w[1].baseline));
        assert!((p.height() - lines[2].bottom).abs() < 0.01);
    }

    #[test]
    fn glyph_baselines_match_their_lines() {
        let p = layout("a\nb", style());
        let baselines: Vec<f32> = p
            .runs()
            .flat_map(|r| r.glyphs.iter().map(|g| g.y))
            .collect();
        assert_eq!(baselines, [p.lines()[0].baseline, p.lines()[1].baseline]);
    }
}

mod wrapping {
    use super::*;

    const TEXT: &str = "alpha beta gamma delta epsilon zeta eta theta";

    #[test]
    fn long_text_wraps_within_the_width() {
        let width = 15.0 * ADVANCE;
        let p = layout(TEXT, style().with_max_width(Some(width)));
        assert!(p.lines().len() >= 3, "{} lines", p.lines().len());
        for line in p.lines() {
            assert!(line.width <= width + 0.01, "{line:?} wider than {width}");
        }
        for run in p.runs() {
            for g in run.glyphs {
                assert!(
                    g.x + ADVANCE <= width + ADVANCE + 0.01,
                    "{g:?} past the width"
                );
            }
        }
    }

    #[test]
    fn lines_break_between_words() {
        let p = layout(TEXT, style().with_max_width(Some(15.0 * ADVANCE)));
        for line in &p.lines()[1..] {
            let first = &TEXT[line.text_range.clone()];
            assert!(!first.starts_with(' '), "line starts mid-gap: {first:?}");
            let before = TEXT[..line.text_range.start].chars().last();
            assert_eq!(before, Some(' '), "line starts mid-word: {first:?}");
        }
    }

    #[test]
    fn reflow_rebreaks_without_reshaping() {
        let mut p = layout(TEXT, style().with_max_width(Some(15.0 * ADVANCE)));
        let narrow = p.lines().len();
        let glyphs = glyph_ids(&p);

        p.reflow(None).unwrap();
        assert_eq!(p.lines().len(), 1);
        assert_eq!(p.max_width(), None);
        p.reflow(Some(8.0 * ADVANCE)).unwrap();
        assert!(p.lines().len() > narrow);
        assert_eq!(glyph_ids(&p), glyphs, "same shaped glyphs, only re-broken");
    }

    #[test]
    fn a_word_longer_than_the_width_still_lays_out() {
        let p = layout(
            "abcdefghijklmnopqrstuvwxyz",
            style().with_max_width(Some(5.0 * ADVANCE)),
        );
        assert_eq!(p.glyph_count(), 26);
    }
}

mod alignment {
    use super::*;

    const WIDTH: f32 = 30.0 * ADVANCE;

    fn line_of(align: TextAlign) -> (f32, f32) {
        let p = layout(
            "centre me",
            style().with_max_width(Some(WIDTH)).with_align(align),
        );
        let line = &p.lines()[0];
        (line.left, line.width)
    }

    #[test]
    fn start_alignment_is_flush_left() {
        assert_eq!(line_of(TextAlign::Start).0, 0.0);
    }

    #[test]
    fn center_alignment_splits_the_free_space() {
        let (left, width) = line_of(TextAlign::Center);
        assert!((left - (WIDTH - width) / 2.0).abs() < 0.5, "{left} {width}");
    }

    #[test]
    fn right_alignment_ends_at_the_width() {
        let (left, width) = line_of(TextAlign::Right);
        assert!(
            (left + width - WIDTH).abs() < 0.5,
            "{left} + {width} vs {WIDTH}"
        );
    }
}

mod spans {
    use super::*;

    const RED: Color = Color::linear(1.0, 0.0, 0.0, 1.0);

    #[test]
    fn colored_spans_become_runs_of_their_color() {
        let text = "plain RED plain";
        let spans = [Span::new(6..9).with_color(RED)];
        let p = system().paragraph(text, &style(), &spans).unwrap();
        let red: usize = p
            .runs()
            .filter(|r| r.color == RED)
            .map(|r| r.glyphs.len())
            .sum();
        assert_eq!(red, 3);
        assert!(p.runs().any(|r| r.color == Color::WHITE));
    }

    #[test]
    fn sized_spans_are_shaped_at_their_size() {
        let spans = [Span::new(0..3).with_size(2.0 * PX)];
        let p = system().paragraph("BIGsmall", &style(), &spans).unwrap();
        let sizes: Vec<f32> = p.runs().map(|r| r.px).collect();
        assert!(
            sizes.contains(&(2.0 * PX)) && sizes.contains(&PX),
            "{sizes:?}"
        );
    }
}

mod scripts {
    use super::*;

    #[test]
    fn precomposed_and_decomposed_vietnamese_shape_without_missing_glyphs() {
        for text in ["Tiếng Việt", "Tie\u{0302}\u{0301}ng Vie\u{0323}\u{0302}t"] {
            let ids = glyph_ids(&layout(text, style()));
            assert!(
                !ids.contains(&GlyphId(0)),
                "{text:?} has missing glyphs: {ids:?}"
            );
        }
    }

    #[test]
    fn characters_no_font_covers_draw_as_the_missing_glyph() {
        // Only the bundled font is available: CJK has no glyphs (system fonts add fallback).
        assert!(glyph_ids(&layout("漢字", style())).contains(&GlyphId(0)));
    }

    #[test]
    fn the_bundled_font_keeps_its_identity() {
        let p = layout("abc", style());
        let bundled = Font::bundled_mono().unwrap().id();
        assert!(
            p.runs().all(|r| r.font.id() == bundled),
            "shares the grid's glyphs"
        );
    }

    #[test]
    fn layout_is_deterministic() {
        let a = layout("Xin chào thế giới", style().with_max_width(Some(80.0)));
        let b = layout("Xin chào thế giới", style().with_max_width(Some(80.0)));
        assert_eq!(a.lines(), b.lines());
        assert_eq!(glyph_ids(&a), glyph_ids(&b));
    }
}

mod validation {
    use super::*;

    fn error(text: &str, style: ParagraphStyle, spans: &[Span]) -> ParagraphError {
        system().paragraph(text, &style, spans).unwrap_err()
    }

    #[test]
    fn sizes_must_be_positive_and_finite() {
        for size in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let e = error("x", ParagraphStyle::new(size), &[]);
            assert!(matches!(e, ParagraphError::InvalidSize(_)), "{size}: {e}");
        }
    }

    #[test]
    fn widths_must_be_non_negative_and_finite() {
        let e = error("x", style().with_max_width(Some(-1.0)), &[]);
        assert_eq!(e, ParagraphError::InvalidWidth(-1.0));
        let mut p = layout("x", style());
        assert!(p.reflow(Some(f32::NAN)).is_err());
    }

    #[test]
    fn spans_must_lie_on_character_boundaries_within_the_text() {
        let text = "aệb"; // 'ệ' is 3 bytes: 1..4
                          // 3..1 is reversed on purpose.
        let reversed = std::ops::Range { start: 3, end: 1 };
        for range in [0..9, 2..3, reversed] {
            let e = error(text, style(), &[Span::new(range.clone())]);
            assert_eq!(e, ParagraphError::InvalidSpan(range));
        }
        assert!(system()
            .paragraph(text, &style(), &[Span::new(1..4)])
            .is_ok());
    }

    #[test]
    fn span_sizes_are_validated_too() {
        let e = error("ab", style(), &[Span::new(0..1).with_size(0.0)]);
        assert_eq!(e, ParagraphError::InvalidSize(0.0));
    }
}

mod fonts {
    use super::*;

    #[test]
    fn registering_a_font_reports_its_families() {
        let bytes: densa_text::FontBytes = std::sync::Arc::new(densa_text::BUNDLED_MONO);
        let names = system().register_font(bytes).unwrap();
        assert_eq!(names, ["JetBrains Mono"]);
    }

    #[test]
    fn registering_garbage_is_an_error() {
        let bytes: densa_text::FontBytes = std::sync::Arc::new(b"not a font".to_vec());
        assert!(system().register_font(bytes).is_err());
    }
}

#[test]
fn an_empty_paragraph_is_one_empty_line() {
    // One line of height, as a caret in an empty text field needs.
    let p = layout("", style());
    assert_eq!(p.glyph_count(), 0);
    assert_eq!(p.runs().len(), 0);
    assert_eq!(p.lines().len(), 1);
    assert_eq!(p.lines()[0].text_range, 0..0, "within the (empty) text");
    assert_eq!(p.height(), p.lines()[0].bottom);
    assert!(p.height() > 0.0);
}

mod more_alignment {
    use super::*;

    #[test]
    fn justified_lines_span_the_width_except_the_last() {
        let width = 20.0 * ADVANCE;
        let text = "one two three four five six seven eight nine ten";
        let p = layout(
            text,
            style()
                .with_max_width(Some(width))
                .with_align(TextAlign::Justify),
        );
        assert!(p.lines().len() >= 3);
        let ends = line_ends(&p);
        let last = ends.len() - 1;
        for (line, end) in ends.iter().enumerate() {
            if line < last {
                assert!(
                    (end - width).abs() < 1.0,
                    "line {line} ends at {end}, not {width}"
                );
            } else {
                assert!(
                    *end < width - ADVANCE,
                    "the last line is not stretched: {end}"
                );
            }
        }
    }

    #[test]
    fn right_to_left_text_starts_at_the_right() {
        // Hebrew has no glyphs in the bundled font, but its direction still governs layout:
        // `Start` is the right edge of a right-to-left paragraph.
        let width = 30.0 * ADVANCE;
        let p = layout("שלום עולם", style().with_max_width(Some(width)));
        let line = &p.lines()[0];
        assert!((line.left + line.width - width).abs() < 0.5, "{line:?}");
    }

    /// Pen position after the last visible glyph of each line. Trailing spaces are left
    /// out: they hang past the edge (as in CSS) instead of being justified against it.
    fn line_ends(p: &Paragraph) -> Vec<f32> {
        let space = Font::bundled_mono().unwrap().glyph_id(' ').unwrap();
        p.lines()
            .iter()
            .map(|line| {
                p.runs()
                    .flat_map(|r| r.glyphs.iter())
                    .filter(|g| g.y == line.baseline && g.id != space)
                    .map(|g| g.x + ADVANCE)
                    .fold(0.0, f32::max)
            })
            .collect()
    }
}

mod limits {
    use super::*;
    use densa_paragraph::{MAX_FONT_PX, MAX_LINE_HEIGHT};

    #[test]
    fn sizes_beyond_the_supported_range_are_rejected() {
        for size in [MAX_FONT_PX * 1.01, 1e30, f32::MAX] {
            let e = system()
                .paragraph("x", &ParagraphStyle::new(size), &[])
                .unwrap_err();
            assert!(matches!(e, ParagraphError::InvalidSize(_)), "{size}: {e}");
        }
        let tall = style().with_line_height(MAX_LINE_HEIGHT * 2.0);
        assert!(system().paragraph("x", &tall, &[]).is_err());
    }

    #[test]
    fn the_largest_supported_sizes_give_finite_geometry() {
        let style = ParagraphStyle::new(MAX_FONT_PX).with_line_height(MAX_LINE_HEIGHT);
        let p = system()
            .paragraph(
                "Wg
Wg",
                &style,
                &[],
            )
            .unwrap();
        assert!(p.width().is_finite() && p.height().is_finite());
        assert!(p.lines().iter().all(|l| l.baseline.is_finite()));
    }
}

mod identity {
    use super::*;

    #[test]
    fn the_same_font_bytes_registered_again_are_still_one_face() {
        let mut system = system();
        let bytes: densa_text::FontBytes = std::sync::Arc::new(densa_text::BUNDLED_MONO);
        system.register_font(bytes).unwrap();
        let p = system
            .paragraph("abc", &style().with_family("JetBrains Mono"), &[])
            .unwrap();
        let bundled = Font::bundled_mono().unwrap().id();
        assert!(
            p.runs().all(|r| r.font.id() == bundled),
            "no duplicate glyphs"
        );
    }

    #[test]
    fn text_systems_and_paragraphs_can_move_between_threads() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<TextSystem>();
        send_sync::<Paragraph>();
    }
}
