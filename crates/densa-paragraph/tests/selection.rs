//! Hit-testing, caret and selection geometry, keyboard motions.

use densa_paragraph::{Motion, Paragraph, ParagraphStyle, Point, TextSelection, TextSystem};

const PX: f32 = 14.0;
/// JetBrains Mono advances 0.6 em.
const ADVANCE: f32 = 0.6 * PX;

fn layout(text: &str, max_width: Option<f32>) -> Paragraph {
    let style = ParagraphStyle::new(PX).with_max_width(max_width);
    TextSystem::bundled()
        .unwrap()
        .paragraph(text, &style, &[])
        .unwrap()
}

/// The middle of line `line`, `x` pixels from the left.
fn on_line(p: &Paragraph, line: usize, x: f32) -> Point {
    let l = &p.lines()[line];
    Point::new(x, (l.top + l.bottom) / 2.0)
}

mod hit_testing {
    use super::*;

    #[test]
    fn clicks_land_before_or_after_a_character_by_its_nearer_half() {
        let p = layout("hello world", None);
        let left_half = on_line(&p, 0, 4.0 * ADVANCE + 0.25 * ADVANCE);
        let right_half = on_line(&p, 0, 4.0 * ADVANCE + 0.75 * ADVANCE);
        assert_eq!(TextSelection::at_point(&p, left_half).focus(), 4);
        assert_eq!(TextSelection::at_point(&p, right_half).focus(), 5);
    }

    #[test]
    fn clicks_outside_the_text_clamp_to_its_ends() {
        let p = layout("hello", None);
        assert_eq!(
            TextSelection::at_point(&p, on_line(&p, 0, -50.0)).focus(),
            0
        );
        assert_eq!(
            TextSelection::at_point(&p, on_line(&p, 0, 500.0)).focus(),
            5
        );
        let below = Point::new(500.0, p.height() + 100.0);
        assert_eq!(TextSelection::at_point(&p, below).focus(), 5);
    }

    #[test]
    fn clicks_pick_the_line_under_them() {
        let p = layout("first line\nsecond line", None);
        let caret = TextSelection::at_point(&p, on_line(&p, 1, 0.0));
        assert_eq!(caret.focus(), "first line\n".len());
    }

    #[test]
    fn multi_byte_characters_are_never_split() {
        let text = "Ünïcödé façade";
        let p = layout(text, None);
        for step in 0..40 {
            let x = step as f32 * ADVANCE / 4.0;
            let index = TextSelection::at_point(&p, on_line(&p, 0, x)).focus();
            assert!(text.is_char_boundary(index), "x {x} -> byte {index}");
        }
    }

    #[test]
    fn a_double_click_selects_the_word() {
        let text = "hello world";
        let p = layout(text, None);
        let word = TextSelection::word_at_point(&p, on_line(&p, 0, 7.5 * ADVANCE));
        assert_eq!(&text[word.range()], "world");
    }

    #[test]
    fn dragging_extends_from_the_anchor() {
        let p = layout("hello world", None);
        let start = TextSelection::at_point(&p, on_line(&p, 0, 1.0 * ADVANCE));
        let dragged = start.extend_to_point(&p, on_line(&p, 0, 4.0 * ADVANCE));
        assert_eq!((dragged.anchor(), dragged.focus()), (1, 4));
        assert_eq!(dragged.range(), 1..4);
        assert!(!dragged.is_caret());
    }
}

mod geometry {
    use super::*;

    #[test]
    fn the_caret_sits_at_the_pen_position_and_spans_the_line() {
        let p = layout("hello", None);
        let line = &p.lines()[0];
        for index in 0..=5 {
            let rect = TextSelection::caret_at(&p, index).caret_rect(&p, 1.0);
            let x = index as f32 * ADVANCE;
            assert!((rect.left() - x).abs() < 0.5, "index {index}: {rect:?}");
            assert!((rect.width() - 1.0).abs() < 1e-3);
            assert!(
                rect.top() <= line.baseline && rect.bottom() >= line.baseline,
                "{rect:?}"
            );
        }
    }

    #[test]
    fn the_caret_of_a_selection_is_at_its_focus() {
        let p = layout("hello world", None);
        let s = TextSelection::caret_at(&p, 1).moved(&p, Motion::WordRight, true);
        let rect = s.caret_rect(&p, 1.0);
        assert!(
            (rect.left() - s.focus() as f32 * ADVANCE).abs() < 0.5,
            "{rect:?} vs {}",
            s.focus()
        );
    }

    #[test]
    fn a_caret_has_no_selection_rectangles() {
        let p = layout("hello", None);
        let mut count = 0;
        TextSelection::caret_at(&p, 2).for_each_rect(&p, |_| count += 1);
        assert_eq!(count, 0);
    }

    #[test]
    fn a_selection_within_a_line_is_one_rectangle_over_its_characters() {
        let p = layout("hello world", None);
        let selection = TextSelection::caret_at(&p, 0).moved(&p, Motion::WordRight, true);
        let mut rects = Vec::new();
        selection.for_each_rect(&p, |r| rects.push(r));
        assert_eq!(rects.len(), 1);
        let chars = selection.range().len() as f32;
        assert!(
            (rects[0].width() - chars * ADVANCE).abs() < 0.5,
            "{rects:?}"
        );
    }

    #[test]
    fn a_selection_across_lines_has_a_rectangle_per_line() {
        let text = "alpha beta gamma delta epsilon zeta";
        let p = layout(text, Some(12.0 * ADVANCE));
        assert!(p.lines().len() >= 3);
        let all = TextSelection::caret_at(&p, 0).moved(&p, Motion::TextEnd, true);
        assert_eq!(all.range(), 0..text.len());
        let mut lines_hit = Vec::new();
        all.for_each_rect(&p, |r| lines_hit.push(r.top()));
        lines_hit.dedup();
        assert_eq!(lines_hit.len(), p.lines().len());
    }
}

mod motions {
    use super::*;

    fn moved(p: &Paragraph, from: usize, motion: Motion) -> usize {
        TextSelection::caret_at(p, from)
            .moved(p, motion, false)
            .focus()
    }

    #[test]
    fn left_and_right_step_one_character_and_stop_at_the_ends() {
        let p = layout("abc", None);
        assert_eq!(moved(&p, 0, Motion::Right), 1);
        assert_eq!(moved(&p, 2, Motion::Left), 1);
        assert_eq!(moved(&p, 0, Motion::Left), 0);
        assert_eq!(moved(&p, 3, Motion::Right), 3);
    }

    #[test]
    fn a_step_crosses_a_whole_character_however_it_is_encoded() {
        let precomposed = "ẃx"; // U+1E83: 3 bytes
        assert_eq!(moved(&layout(precomposed, None), 0, Motion::Right), 3);
        let decomposed = "e\u{0301}\u{0308}x"; // one grapheme: 1 + 2 + 2 bytes
        assert_eq!(moved(&layout(decomposed, None), 0, Motion::Right), 5);
    }

    #[test]
    fn word_motions_stop_at_word_boundaries() {
        let text = "hello brave world";
        let p = layout(text, None);
        let next = moved(&p, 0, Motion::WordRight);
        assert!([5, 6].contains(&next), "{next}");
        let back = moved(&p, text.len(), Motion::WordLeft);
        assert_eq!(back, "hello brave ".len());
    }

    #[test]
    fn up_and_down_keep_the_horizontal_position() {
        let p = layout("0123456789\nabcdefghij\nABCDEFGHIJ", None);
        let start = "0123456789\nabcd".len(); // column 4 of line 1
        assert_eq!(moved(&p, start, Motion::Up), 4);
        assert_eq!(
            moved(&p, start, Motion::Down),
            "0123456789\nabcdefghij\nABCD".len()
        );
    }

    #[test]
    fn line_start_and_end_follow_wrapped_lines() {
        let text = "alpha beta gamma delta";
        let p = layout(text, Some(12.0 * ADVANCE));
        let second = &p.lines()[1];
        let inside = second.text_range.start + 2;
        assert_eq!(
            moved(&p, inside, Motion::LineStart),
            second.text_range.start
        );
        let end = moved(&p, inside, Motion::LineEnd);
        assert!(
            end > inside && end <= second.text_range.end,
            "{end} in {:?}",
            second.text_range
        );
    }

    #[test]
    fn text_start_and_end_with_extend_keep_the_anchor() {
        let text = "some text";
        let p = layout(text, None);
        let caret = TextSelection::caret_at(&p, 4);
        let to_end = caret.moved(&p, Motion::TextEnd, true);
        assert_eq!((to_end.anchor(), to_end.focus()), (4, text.len()));
        let to_start = caret.moved(&p, Motion::TextStart, false);
        assert!(to_start.is_caret());
        assert_eq!(to_start.focus(), 0);
    }

    #[test]
    fn collapsing_keeps_the_focus() {
        let p = layout("hello", None);
        let s = TextSelection::caret_at(&p, 1).moved(&p, Motion::Right, true);
        assert_eq!(s.collapsed().focus(), 2);
        assert!(s.collapsed().is_caret());
    }
}

mod lifecycle {
    use super::*;

    #[test]
    fn carets_clamp_to_the_text() {
        let p = layout("abc", None);
        assert_eq!(TextSelection::caret_at(&p, 999).focus(), 3);
    }

    #[test]
    fn selections_survive_a_reflow() {
        let text = "alpha beta gamma delta epsilon";
        let mut p = layout(text, None);
        let selection = TextSelection::caret_at(&p, 6).moved(&p, Motion::WordRight, true);
        let range = selection.range();
        p.reflow(Some(10.0 * ADVANCE)).unwrap();
        let refreshed = selection.refreshed(&p);
        assert_eq!(refreshed.range(), range);
        let mut rects = 0;
        refreshed.for_each_rect(&p, |_| rects += 1);
        assert!(rects >= 1);
    }

    #[test]
    fn selections_compare_by_position() {
        let p = layout("hello", None);
        assert_eq!(
            TextSelection::caret_at(&p, 2),
            TextSelection::caret_at(&p, 2)
        );
        assert_ne!(
            TextSelection::caret_at(&p, 2),
            TextSelection::caret_at(&p, 3)
        );
    }
}

mod edges {
    use super::*;
    use densa_paragraph::Affinity;

    const ALL: [Motion; 14] = [
        Motion::Left,
        Motion::Right,
        Motion::WordLeft,
        Motion::WordRight,
        Motion::Previous,
        Motion::Next,
        Motion::PreviousWord,
        Motion::NextWord,
        Motion::Up,
        Motion::Down,
        Motion::LineStart,
        Motion::LineEnd,
        Motion::TextStart,
        Motion::TextEnd,
    ];

    #[test]
    fn an_empty_paragraph_supports_every_operation() {
        let p = layout("", None);
        let caret = TextSelection::caret_at(&p, 0);
        assert_eq!(
            TextSelection::at_point(&p, Point::new(30.0, 5.0)).focus(),
            0
        );
        for motion in ALL {
            for extend in [false, true] {
                assert_eq!(caret.moved(&p, motion, extend).focus(), 0, "{motion:?}");
            }
        }
        let rect = caret.caret_rect(&p, 1.0);
        assert!(rect.height() > 0.0, "{rect:?}");
        let mut rects = 0;
        TextSelection::select_all(&p).for_each_rect(&p, |_| rects += 1);
        assert_eq!(rects, 0);
    }

    #[test]
    fn after_a_trailing_newline_the_caret_sits_on_the_empty_last_line() {
        let p = layout("abc\n", None);
        let end = TextSelection::caret_at(&p, 0).moved(&p, Motion::TextEnd, false);
        assert_eq!(end.focus(), 4);
        let rect = end.caret_rect(&p, 1.0);
        let first = &p.lines()[0];
        assert!(
            rect.top() >= first.bottom - 0.5,
            "{rect:?} not below {first:?}"
        );
        assert!(rect.left() < 1.0, "at the line start: {rect:?}");
    }

    #[test]
    fn up_from_the_first_line_and_down_from_the_last_go_to_the_ends() {
        let text = "one\ntwo";
        let p = layout(text, None);
        let first = TextSelection::caret_at(&p, 2);
        assert_eq!(first.moved(&p, Motion::Up, false).focus(), 0);
        let last = TextSelection::caret_at(&p, 5);
        assert_eq!(last.moved(&p, Motion::Down, false).focus(), text.len());
    }

    #[test]
    fn moving_without_extending_collapses_a_range() {
        let p = layout("hello world", None);
        let range = TextSelection::from_range(&p, 2, 7);
        assert!(range.moved(&p, Motion::Left, false).is_caret());
        assert!(range.moved(&p, Motion::Right, false).is_caret());
    }

    #[test]
    fn a_triple_click_selects_the_visual_line() {
        let text = "alpha beta gamma delta";
        let p = layout(text, Some(12.0 * ADVANCE));
        let line = &p.lines()[1];
        let selected = TextSelection::line_at_point(&p, on_line(&p, 1, ADVANCE));
        assert_eq!(selected.range().start, line.text_range.start);
        assert!(
            selected.range().end >= line.text_range.end - 1,
            "{:?}",
            selected.range()
        );
    }

    #[test]
    fn shift_click_after_a_word_keeps_the_far_side_of_the_word() {
        let text = "one two three";
        let p = layout(text, None);
        let word = TextSelection::word_at_point(&p, on_line(&p, 0, 5.0 * ADVANCE));
        assert_eq!(&text[word.range()], "two");
        let back = word.shift_click(&p, on_line(&p, 0, 0.5 * ADVANCE));
        assert_eq!(
            (back.anchor(), back.focus()),
            (7, 0),
            "anchor at the end of 'two'"
        );
        let forward = word.shift_click(&p, on_line(&p, 0, 12.5 * ADVANCE));
        assert_eq!(forward.anchor(), 4, "anchor at the start of 'two'");
    }

    #[test]
    fn programmatic_selections_clamp_to_the_text() {
        let p = layout("hello", None);
        assert_eq!(TextSelection::from_range(&p, 4, 1).range(), 1..4);
        assert_eq!(TextSelection::from_range(&p, 2, 99).range(), 2..5);
        assert_eq!(TextSelection::select_all(&p).range(), 0..5);
    }

    #[test]
    fn upstream_affinity_keeps_a_caret_at_the_end_of_a_wrapped_line() {
        let text = "alpha beta gamma delta";
        let p = layout(text, Some(12.0 * ADVANCE));
        let wrap = p.lines()[1].text_range.start;
        let down = TextSelection::caret_with(&p, wrap, Affinity::Downstream);
        let up = TextSelection::caret_with(&p, wrap, Affinity::Upstream);
        assert_eq!(up.focus_affinity(), Affinity::Upstream);
        let (down_rect, up_rect) = (down.caret_rect(&p, 1.0), up.caret_rect(&p, 1.0));
        assert!(
            up_rect.top() < down_rect.top(),
            "{up_rect:?} vs {down_rect:?}"
        );
    }

    #[test]
    fn a_selection_from_another_paragraph_is_clamped_by_refreshing_and_never_panics() {
        let long = layout("a much longer paragraph of text", None);
        let short = layout("tiny", None);
        let foreign = TextSelection::from_range(&long, 3, 25);
        for motion in ALL {
            let _ = foreign.moved(&short, motion, true);
        }
        let _ = foreign.caret_rect(&short, 1.0);
        foreign.for_each_rect(&short, |_| {});
        assert_eq!(foreign.refreshed(&short).range(), 3..4);
    }

    #[test]
    fn equal_positions_are_equal_selections() {
        let p = layout("hello", None);
        let a = TextSelection::caret_at(&p, 2);
        let b = TextSelection::at_point(&p, on_line(&p, 0, 2.0 * ADVANCE));
        assert_eq!(a.focus(), b.focus());
        assert_eq!(a.collapsed(), a);
    }
}

mod right_to_left {
    use super::*;

    // Hebrew: no glyphs in the bundled font, but bidi still orders it right to left.
    const TEXT: &str = "שלום עולם"; // each letter is 2 bytes

    #[test]
    fn left_moves_forward_through_right_to_left_text() {
        let p = layout(TEXT, None);
        let start = TextSelection::caret_at(&p, 0);
        assert_eq!(
            start.moved(&p, Motion::Left, false).focus(),
            2,
            "visual left = next"
        );
        assert_eq!(
            start.moved(&p, Motion::Right, false).focus(),
            0,
            "already at the right end"
        );
    }

    #[test]
    fn logical_motions_ignore_direction() {
        let p = layout(TEXT, None);
        let start = TextSelection::caret_at(&p, 0);
        assert_eq!(start.moved(&p, Motion::Next, false).focus(), 2);
        let end = TextSelection::caret_at(&p, TEXT.len());
        assert_eq!(
            end.moved(&p, Motion::Previous, false).focus(),
            TEXT.len() - 2
        );
    }

    #[test]
    fn logical_motions_step_whole_characters_in_left_to_right_text_too() {
        let p = layout("ẃa", None);
        let start = TextSelection::caret_at(&p, 0);
        assert_eq!(start.moved(&p, Motion::Next, false).focus(), 3);
        let end = TextSelection::caret_at(&p, 4);
        assert_eq!(end.moved(&p, Motion::Previous, true).range(), 3..4);
        let word = TextSelection::caret_at(&p, 0).moved(&p, Motion::NextWord, false);
        assert_eq!(word.focus(), 4);
    }
}
