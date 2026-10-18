//! Editing: inserting, deleting, input-method composition, and the caret afterwards.

use densa_paragraph::{
    Affinity, Motion, ParagraphError, ParagraphStyle, Point, TextEditor, TextSelection, TextSystem,
};

const PX: f32 = 14.0;
/// JetBrains Mono advances 0.6 em.
const ADVANCE: f32 = 0.6 * PX;

fn editor(text: &str) -> (TextSystem, TextEditor) {
    let mut system = TextSystem::bundled().unwrap();
    let editor = TextEditor::new(&mut system, text, ParagraphStyle::new(PX)).unwrap();
    (system, editor)
}

/// `text` with the caret at byte `at`.
fn editor_at(text: &str, at: usize) -> (TextSystem, TextEditor) {
    let (system, mut editor) = editor(text);
    let caret = TextSelection::caret_at(editor.paragraph(), at);
    editor.set_selection(caret);
    (system, editor)
}

mod construction {
    use super::*;

    #[test]
    fn a_new_editor_holds_its_text_with_the_caret_at_the_end() {
        let (_, e) = editor("hello");
        assert_eq!(e.text(), "hello");
        assert!(e.selection().is_caret());
        assert_eq!(e.selection().focus(), 5);
        assert_eq!(e.composition(), None);
        assert!(e.shows_caret());
    }

    #[test]
    fn an_invalid_style_is_rejected() {
        let mut system = TextSystem::bundled().unwrap();
        let e = TextEditor::new(&mut system, "x", ParagraphStyle::new(0.0)).unwrap_err();
        assert_eq!(e, ParagraphError::InvalidSize(0.0));
    }

    #[test]
    fn an_empty_editor_has_one_line_to_place_the_caret_on() {
        let (_, e) = editor("");
        assert_eq!(e.paragraph().lines().len(), 1);
        assert!(e.paragraph().height() > 0.0);
        assert!(e.ime_area().height() > 0.0);
    }
}

mod inserting {
    use super::*;

    #[test]
    fn typing_inserts_at_the_caret_and_moves_it_past_the_text() {
        let (mut s, mut e) = editor_at("helo", 3);
        e.insert(&mut s, "l").unwrap();
        assert_eq!(e.text(), "hello");
        assert_eq!(e.selection().range(), 4..4);
    }

    #[test]
    fn typing_replaces_the_selection() {
        let (mut s, mut e) = editor("hello world");
        let word = TextSelection::from_range(e.paragraph(), 6, 11);
        e.set_selection(word);
        e.insert(&mut s, "there").unwrap();
        assert_eq!(e.text(), "hello there");
        assert_eq!(e.selection().range(), 11..11);
    }

    #[test]
    fn the_caret_stays_on_the_line_it_typed_on_but_moves_down_after_a_newline() {
        let (mut s, mut e) = editor_at("abcd", 1);
        e.insert(&mut s, "x").unwrap();
        assert_eq!(e.selection().focus_affinity(), Affinity::Upstream);
        e.insert(&mut s, "\n").unwrap();
        assert_eq!(e.text(), "ax\nbcd");
        assert_eq!(e.selection().focus_affinity(), Affinity::Downstream);
        let caret = e.selection().caret_rect(e.paragraph(), 1.0);
        assert_eq!(caret.top(), e.paragraph().lines()[1].top);
    }

    #[test]
    fn every_hard_line_break_moves_the_caret_to_the_next_line() {
        for line_break in ["\u{0B}", "\u{0C}", "\u{85}", "\u{2028}"] {
            let (mut s, mut e) = editor_at("abcd", 2);
            e.insert(&mut s, line_break).unwrap();
            let affinity = e.selection().focus_affinity();
            assert_eq!(affinity, Affinity::Downstream, "{line_break:?}");
        }
    }

    #[test]
    fn typing_a_letter_before_a_combining_accent_puts_the_caret_after_both() {
        let (mut s, mut e) = editor_at("\u{0301}", 0);
        e.insert(&mut s, "e").unwrap();
        assert_eq!(e.text(), "e\u{0301}");
        assert_eq!(e.selection().range(), 3..3);
    }

    #[test]
    fn a_newline_at_the_end_puts_the_caret_on_the_new_empty_line() {
        let (mut s, mut e) = editor("ab");
        e.insert(&mut s, "\n").unwrap();
        assert_eq!(e.paragraph().lines().len(), 2);
        let caret = e.selection().caret_rect(e.paragraph(), 1.0);
        assert_eq!(caret.top(), e.paragraph().lines()[1].top);
    }

    #[test]
    fn typing_rewraps_the_text() {
        let mut system = TextSystem::bundled().unwrap();
        let style = ParagraphStyle::new(PX).with_max_width(Some(10.5 * ADVANCE));
        let mut e = TextEditor::new(&mut system, "hello", style).unwrap();
        assert_eq!(e.paragraph().lines().len(), 1);
        e.insert(&mut system, " brave world").unwrap();
        assert!(e.paragraph().lines().len() > 1);
    }
}

mod deleting {
    use super::*;

    #[test]
    fn backspace_removes_one_character_before_the_caret() {
        let (mut s, mut e) = editor_at("hello", 5);
        e.delete(&mut s, Motion::Previous).unwrap();
        assert_eq!(e.text(), "hell");
        assert_eq!(e.selection().range(), 4..4);
    }

    #[test]
    fn backspace_removes_an_accent_before_its_letter() {
        // A decomposed "é": backspace takes the accent back off, as text editors do.
        let (mut s, mut e) = editor("e\u{0301}");
        e.delete(&mut s, Motion::Previous).unwrap();
        assert_eq!(e.text(), "e");
    }

    #[test]
    fn backspace_removes_a_whole_emoji_and_a_whole_line_break() {
        let (mut s, mut e) = editor("a\u{1F44D}\u{1F3FD}"); // thumbs up, skin tone modifier
        e.delete(&mut s, Motion::Previous).unwrap();
        assert_eq!(e.text(), "a");

        let (mut s, mut e) = editor("a\r\n");
        e.delete(&mut s, Motion::Previous).unwrap();
        assert_eq!(e.text(), "a");
    }

    #[test]
    fn forward_delete_removes_the_whole_character_after_the_caret() {
        let (mut s, mut e) = editor_at("e\u{0301}x", 0);
        e.delete(&mut s, Motion::Next).unwrap();
        assert_eq!(e.text(), "x");
        assert_eq!(e.selection().range(), 0..0);
    }

    #[test]
    fn word_deletion_stops_at_word_boundaries() {
        let (mut s, mut e) = editor("hello brave world");
        e.delete(&mut s, Motion::PreviousWord).unwrap();
        assert_eq!(e.text(), "hello brave ");

        let (mut s, mut e) = editor_at("hello brave world", 0);
        e.delete(&mut s, Motion::NextWord).unwrap();
        assert!(
            e.text() == " brave world" || e.text() == "brave world",
            "{}",
            e.text()
        );
    }

    #[test]
    fn deleting_to_the_line_start_removes_the_line_before_the_caret() {
        let (mut s, mut e) = editor("first\nsecond");
        e.delete(&mut s, Motion::LineStart).unwrap();
        assert_eq!(e.text(), "first\n");
    }

    #[test]
    fn deleting_with_a_selection_removes_only_the_selection() {
        for motion in [Motion::Previous, Motion::Next, Motion::NextWord] {
            let (mut s, mut e) = editor("hello world");
            e.set_selection(TextSelection::from_range(e.paragraph(), 2, 7));
            e.delete(&mut s, motion).unwrap();
            assert_eq!(e.text(), "heorld", "{motion:?}");
            assert_eq!(e.selection().range(), 2..2);
        }
    }

    #[test]
    fn deleting_past_either_end_changes_nothing() {
        let (mut s, mut e) = editor_at("abc", 0);
        e.delete(&mut s, Motion::Previous).unwrap();
        e.delete(&mut s, Motion::PreviousWord).unwrap();
        assert_eq!(e.text(), "abc");

        let (mut s, mut e) = editor("abc");
        e.delete(&mut s, Motion::Next).unwrap();
        assert_eq!(e.text(), "abc");
        assert_eq!(e.selection().range(), 3..3);
    }
}

mod navigation {
    use super::*;

    #[test]
    fn the_caret_moves_and_extends_like_a_selection() {
        let (_, mut e) = editor_at("hello", 0);
        e.move_caret(Motion::Right, false);
        assert_eq!(e.selection().range(), 1..1);
        e.move_caret(Motion::TextEnd, true);
        assert_eq!(e.selection().range(), 1..5);
        assert_eq!(e.selected_text(), "ello");
    }

    #[test]
    fn a_click_places_the_caret() {
        let (_, mut e) = editor("hello world");
        let line = &e.paragraph().lines()[0];
        let point = Point::new(6.0 * ADVANCE, (line.top + line.bottom) / 2.0);
        let caret = TextSelection::at_point(e.paragraph(), point);
        e.set_selection(caret);
        assert_eq!(e.selection().focus(), 6);
    }

    #[test]
    fn a_selection_from_another_text_is_clamped() {
        let mut system = TextSystem::bundled().unwrap();
        let longer = system
            .paragraph("a much longer text", &ParagraphStyle::new(PX), &[])
            .unwrap();
        let (_, mut e) = editor("short");
        e.set_selection(TextSelection::select_all(&longer));
        assert_eq!(e.selection().range(), 0..5);
    }

    #[test]
    fn a_new_width_rewraps_and_keeps_the_caret_on_its_character() {
        let (_, mut e) = editor_at("hello brave new world", 12);
        assert_eq!(e.paragraph().lines().len(), 1);
        e.set_width(Some(6.5 * ADVANCE)).unwrap();
        assert!(e.paragraph().lines().len() >= 3);
        assert_eq!(e.selection().range(), 12..12);
        assert_eq!(e.style().max_width, Some(6.5 * ADVANCE));
    }

    #[test]
    fn an_invalid_width_is_rejected_and_changes_nothing() {
        let (_, mut e) = editor("hello");
        assert_eq!(
            e.set_width(Some(-1.0)),
            Err(ParagraphError::InvalidWidth(-1.0))
        );
        assert_eq!(e.style().max_width, None);
    }
}

mod restyling {
    use super::*;

    #[test]
    fn a_new_style_reshapes_and_keeps_the_selection() {
        let (mut s, mut e) = editor("hello");
        e.set_selection(TextSelection::from_range(e.paragraph(), 1, 3));
        let before = e.paragraph().width();
        e.set_style(&mut s, ParagraphStyle::new(2.0 * PX)).unwrap();
        assert!(e.paragraph().width() > 1.9 * before);
        assert_eq!(e.selection().range(), 1..3);
    }

    #[test]
    fn an_invalid_style_is_rejected_and_changes_nothing() {
        let (mut s, mut e) = editor("hello");
        let err = e
            .set_style(&mut s, ParagraphStyle::new(f32::NAN))
            .unwrap_err();
        assert!(matches!(err, ParagraphError::InvalidSize(_)));
        assert_eq!(e.style().size_px, PX);
    }

    #[test]
    fn replacing_the_text_puts_the_caret_at_its_end() {
        let (mut s, mut e) = editor_at("hello", 1);
        e.set_text(&mut s, "goodbye").unwrap();
        assert_eq!(e.text(), "goodbye");
        assert_eq!(e.selection().range(), 7..7);
    }
}

mod composition {
    use super::*;

    #[test]
    fn preedit_text_is_inserted_and_tracked_until_committed() {
        let (mut s, mut e) = editor_at("ab", 1);
        e.set_composition(&mut s, "k", Some(1..1)).unwrap();
        assert_eq!(e.text(), "akb");
        assert_eq!(e.composition(), Some(1..2));
        assert_eq!(e.selection().range(), 2..2);

        // The input method refines its preedit: it replaces the previous one.
        e.set_composition(&mut s, "\u{304B}", Some(3..3)).unwrap(); // か, 3 bytes
        assert_eq!(e.text(), "a\u{304B}b");
        assert_eq!(e.composition(), Some(1..4));
        assert_eq!(e.selection().range(), 4..4);

        // Commit: the preedit is cleared, then the final text is inserted.
        e.set_composition(&mut s, "", None).unwrap();
        assert_eq!(e.text(), "ab");
        assert_eq!(e.selection().range(), 1..1);
        e.insert(&mut s, "\u{304B}").unwrap();
        assert_eq!(e.text(), "a\u{304B}b");
        assert_eq!(e.composition(), None);
        assert_eq!(e.selection().range(), 4..4);
    }

    #[test]
    fn preedit_replaces_the_selection_it_starts_on() {
        let (mut s, mut e) = editor("hello world");
        e.set_selection(TextSelection::from_range(e.paragraph(), 0, 5));
        e.set_composition(&mut s, "hi", Some(2..2)).unwrap();
        assert_eq!(e.text(), "hi world");
        assert_eq!(e.composition(), Some(0..2));
    }

    #[test]
    fn cancelling_a_preedit_restores_the_selection_it_replaced() {
        let (mut s, mut e) = editor("hello world");
        e.set_selection(TextSelection::from_range(e.paragraph(), 0, 5));
        e.set_composition(&mut s, "hi", Some(2..2)).unwrap();
        e.set_composition(&mut s, "h", Some(1..1)).unwrap();
        e.set_composition(&mut s, "", None).unwrap();
        assert_eq!(e.text(), "hello world");
        assert_eq!(e.selection().range(), 0..5);
    }

    #[test]
    fn committing_over_a_selection_replaces_it() {
        // winit sends an empty preedit right before every commit.
        let (mut s, mut e) = editor("hello world");
        e.set_selection(TextSelection::from_range(e.paragraph(), 0, 5));
        e.set_composition(&mut s, "hi", Some(2..2)).unwrap();
        e.set_composition(&mut s, "", None).unwrap();
        e.insert(&mut s, "hey").unwrap();
        assert_eq!(e.text(), "hey world");
        assert_eq!(e.selection().range(), 3..3);
    }

    #[test]
    fn the_input_method_can_select_inside_the_preedit_or_hide_the_caret() {
        let (mut s, mut e) = editor("");
        e.set_composition(&mut s, "abc", Some(1..3)).unwrap();
        assert_eq!(e.selection().range(), 1..3);
        assert!(e.shows_caret());
        e.set_composition(&mut s, "abc", None).unwrap();
        assert!(!e.shows_caret());
        e.set_composition(&mut s, "", None).unwrap();
        assert!(e.shows_caret());
    }

    #[test]
    fn a_preedit_cursor_outside_the_preedit_or_inside_a_character_is_rejected() {
        let (mut s, mut e) = editor("ab");
        // 2..1 is reversed on purpose.
        let reversed = std::ops::Range { start: 2, end: 1 };
        for cursor in [0..4, reversed, 0..1] {
            let err = e
                .set_composition(&mut s, "\u{304B}", Some(cursor.clone()))
                .unwrap_err();
            assert_eq!(err, ParagraphError::InvalidRange(cursor));
        }
        assert_eq!(e.text(), "ab");
        assert_eq!(e.composition(), None);
    }

    #[test]
    fn typing_during_composition_replaces_the_preedit() {
        let (mut s, mut e) = editor("x");
        e.set_composition(&mut s, "abc", Some(3..3)).unwrap();
        e.insert(&mut s, "Z").unwrap();
        assert_eq!(e.text(), "xZ");
        assert_eq!(e.composition(), None);
    }

    #[test]
    fn deleting_during_composition_cancels_the_preedit() {
        let (mut s, mut e) = editor("x");
        e.set_composition(&mut s, "abc", Some(3..3)).unwrap();
        e.delete(&mut s, Motion::Previous).unwrap();
        assert_eq!(e.text(), "x");
        assert_eq!(e.composition(), None);
        assert_eq!(e.selection().range(), 1..1);
    }

    #[test]
    fn moving_the_caret_keeps_the_preedit_as_text() {
        let (mut s, mut e) = editor("x");
        e.set_composition(&mut s, "abc", None).unwrap();
        e.move_caret(Motion::TextStart, false);
        assert_eq!(e.text(), "xabc");
        assert_eq!(e.composition(), None);
        assert!(e.shows_caret());
    }

    #[test]
    fn a_click_during_composition_keeps_the_preedit_as_text() {
        let (mut s, mut e) = editor("x");
        e.set_composition(&mut s, "abc", None).unwrap();
        e.set_selection(TextSelection::caret_at(e.paragraph(), 0));
        assert_eq!(e.text(), "xabc");
        assert_eq!(e.composition(), None);
        assert!(e.shows_caret());
    }

    #[test]
    fn the_input_method_area_spans_every_line_of_a_wrapped_preedit() {
        let mut s = TextSystem::bundled().unwrap();
        let style = ParagraphStyle::new(PX).with_max_width(Some(8.5 * ADVANCE));
        let mut e = TextEditor::new(&mut s, "abcd ", style).unwrap();
        e.set_composition(&mut s, "efgh ijkl", Some(9..9)).unwrap();
        let mut rects = Vec::new();
        e.for_each_composition_rect(|rect| rects.push(rect));
        assert!(rects.len() >= 2, "{rects:?}");
        let area = e.ime_area();
        assert_eq!(area.top(), rects[0].top());
        assert_eq!(area.bottom(), rects[rects.len() - 1].bottom());
    }

    #[test]
    fn the_input_method_area_covers_the_preedit_or_else_the_caret() {
        let (mut s, mut e) = editor("hello");
        let caret = e.selection().caret_rect(e.paragraph(), 0.0);
        assert_eq!(e.ime_area().left(), caret.left());

        e.set_composition(&mut s, "abc", Some(3..3)).unwrap();
        let area = e.ime_area();
        assert!((area.left() - 5.0 * ADVANCE).abs() < 0.5, "{area:?}");
        assert!((area.width() - 3.0 * ADVANCE).abs() < 0.5, "{area:?}");

        let mut rects = 0;
        e.for_each_composition_rect(|rect| {
            rects += 1;
            assert_eq!(rect.left(), area.left());
        });
        assert_eq!(rects, 1);
    }
}
