//! Cut, copy, paste and undo: text inputs and the grid with the Ui's clipboard.

use densa_core::Size;
use densa_paragraph::TextSystem;
use densa_ui::{
    ActionKind, Clipboard, ClipboardError, Handled, Key, KeyEvent, Modifiers, NamedKey, Style, Ui,
    WidgetId,
};
use densa_widgets::TextInput;

fn ui() -> Ui {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(800.0, 600.0));
    ui
}

fn command() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers::LOGO
    } else {
        Modifiers::CONTROL
    }
}

/// Ctrl (Cmd on macOS), with `more`, and `letter`.
fn shortcut(ui: &mut Ui, letter: &str, more: Modifiers) -> Handled {
    ui.key(&KeyEvent::down(Key::Character(letter)).with_modifiers(command() | more))
}

fn named(ui: &mut Ui, key: NamedKey, modifiers: Modifiers) -> Handled {
    ui.key(&KeyEvent::down(Key::Named(key)).with_modifiers(modifiers))
}

fn typing(ui: &mut Ui, text: &str) {
    for ch in text.chars() {
        let mut buffer = [0; 4];
        let s: &str = ch.encode_utf8(&mut buffer);
        ui.key(&KeyEvent::down(Key::Character(s)).with_text(s));
    }
}

/// An input holding `text`, focused, with everything selected.
fn input(ui: &mut Ui, text: &str) -> WidgetId {
    let id = ui
        .add(ui.root(), TextInput::new(text), Style::row())
        .unwrap();
    ui.layout();
    ui.set_focus(Some(id));
    shortcut(ui, "a", Modifiers::NONE);
    id
}

fn text_of(ui: &Ui, id: WidgetId) -> String {
    ui.widget::<TextInput>(id).unwrap().text().to_owned()
}

fn changes(ui: &mut Ui) -> usize {
    ui.actions()
        .filter(|a| a.kind == ActionKind::Changed)
        .count()
}

fn clipboard(ui: &mut Ui) -> Result<String, ClipboardError> {
    ui.clipboard().text()
}

/// A clipboard that takes nothing and gives nothing.
struct Locked;

impl Clipboard for Locked {
    fn text(&mut self) -> Result<String, ClipboardError> {
        Err(ClipboardError::Unavailable("locked".into()))
    }

    fn set_text(&mut self, _: &str) -> Result<(), ClipboardError> {
        Err(ClipboardError::Unavailable("locked".into()))
    }
}

mod text_input {
    use super::*;

    #[test]
    fn text_copied_from_one_input_pastes_into_another() {
        let mut ui = ui();
        let from = input(&mut ui, "hello world");
        assert_eq!(shortcut(&mut ui, "c", Modifiers::NONE), Handled::Yes);
        assert_eq!(clipboard(&mut ui).unwrap(), "hello world");
        assert_eq!(text_of(&ui, from), "hello world", "copying changes nothing");
        let to = input(&mut ui, "");
        changes(&mut ui);
        shortcut(&mut ui, "v", Modifiers::NONE);
        assert_eq!(text_of(&ui, to), "hello world");
        assert_eq!(changes(&mut ui), 1);
    }

    #[test]
    fn cut_copies_then_deletes_the_selection() {
        let mut ui = ui();
        let id = input(&mut ui, "abc");
        named(&mut ui, NamedKey::ArrowLeft, Modifiers::NONE);
        named(&mut ui, NamedKey::End, Modifiers::SHIFT);
        changes(&mut ui);
        shortcut(&mut ui, "x", Modifiers::NONE);
        assert_eq!(text_of(&ui, id), "");
        assert_eq!(clipboard(&mut ui).unwrap(), "abc");
        assert_eq!(changes(&mut ui), 1);
    }

    #[test]
    fn nothing_is_cut_that_the_clipboard_did_not_take() {
        let mut ui = ui();
        ui.set_clipboard(Locked);
        let id = input(&mut ui, "keep");
        changes(&mut ui);
        assert_eq!(shortcut(&mut ui, "x", Modifiers::NONE), Handled::Yes);
        assert_eq!(text_of(&ui, id), "keep");
        assert_eq!(changes(&mut ui), 0);
        assert!(!ui.update(id, |w: &mut TextInput, cx| w.paste(cx)).unwrap());
    }

    #[test]
    fn copying_nothing_leaves_the_clipboard_as_it_was() {
        let mut ui = ui();
        ui.clipboard().set_text("before").unwrap();
        input(&mut ui, "abc");
        named(&mut ui, NamedKey::End, Modifiers::NONE);
        shortcut(&mut ui, "c", Modifiers::NONE);
        shortcut(&mut ui, "x", Modifiers::NONE);
        assert_eq!(clipboard(&mut ui).unwrap(), "before");
    }

    #[test]
    fn pasted_line_breaks_become_spaces_and_trailing_ones_go() {
        let mut ui = ui();
        ui.clipboard()
            .set_text("one\r\ntwo\nthree\rfour\r\n\n")
            .unwrap();
        let id = input(&mut ui, "");
        shortcut(&mut ui, "v", Modifiers::NONE);
        assert_eq!(text_of(&ui, id), "one two three four");
    }

    #[test]
    fn an_empty_clipboard_pastes_nothing_and_keeps_the_key() {
        let mut ui = ui();
        let id = input(&mut ui, "abc");
        changes(&mut ui);
        assert_eq!(shortcut(&mut ui, "v", Modifiers::NONE), Handled::Yes);
        ui.clipboard().set_text("\n").unwrap();
        shortcut(&mut ui, "v", Modifiers::NONE);
        assert_eq!(text_of(&ui, id), "abc");
        assert_eq!(changes(&mut ui), 0);
    }

    #[test]
    fn the_windows_keys_cut_copy_and_paste_too() {
        let mut ui = ui();
        let id = input(&mut ui, "abc");
        named(&mut ui, NamedKey::Insert, Modifiers::CONTROL);
        assert_eq!(clipboard(&mut ui).unwrap(), "abc");
        ui.clipboard().set_text("xyz").unwrap();
        named(&mut ui, NamedKey::Delete, Modifiers::SHIFT);
        assert_eq!(text_of(&ui, id), "");
        assert_eq!(clipboard(&mut ui).unwrap(), "abc");
        named(&mut ui, NamedKey::Insert, Modifiers::SHIFT);
        assert_eq!(text_of(&ui, id), "abc");
        // Ctrl+Shift+Insert and Ctrl+Shift+Delete are not clipboard keys.
        let both = Modifiers::CONTROL | Modifiers::SHIFT;
        ui.clipboard().set_text("other").unwrap();
        named(&mut ui, NamedKey::Insert, both);
        named(&mut ui, NamedKey::Home, Modifiers::NONE);
        named(&mut ui, NamedKey::Delete, both);
        assert_eq!(clipboard(&mut ui).unwrap(), "other");
    }

    #[test]
    fn undo_and_redo_follow_the_keys() {
        let mut ui = ui();
        let id = input(&mut ui, "");
        typing(&mut ui, "ab cd");
        changes(&mut ui);
        shortcut(&mut ui, "z", Modifiers::NONE);
        assert_eq!(text_of(&ui, id), "ab ");
        shortcut(&mut ui, "Z", Modifiers::SHIFT);
        assert_eq!(text_of(&ui, id), "ab cd");
        shortcut(&mut ui, "z", Modifiers::NONE);
        shortcut(&mut ui, "z", Modifiers::NONE);
        assert_eq!(text_of(&ui, id), "");
        shortcut(&mut ui, "y", Modifiers::NONE);
        assert_eq!(text_of(&ui, id), "ab ");
        assert_eq!(changes(&mut ui), 5);
        // Nothing left to redo: the key is still the input's, and nothing changes.
        shortcut(&mut ui, "y", Modifiers::NONE);
        assert_eq!(shortcut(&mut ui, "y", Modifiers::NONE), Handled::Yes);
        assert_eq!(changes(&mut ui), 1);
    }

    #[test]
    fn a_paste_is_undone_apart_from_the_typing_around_it() {
        let mut ui = ui();
        let id = input(&mut ui, "");
        ui.clipboard().set_text("PASTE").unwrap();
        typing(&mut ui, "ab");
        shortcut(&mut ui, "v", Modifiers::NONE);
        typing(&mut ui, "cd");
        shortcut(&mut ui, "z", Modifiers::NONE);
        assert_eq!(text_of(&ui, id), "abPASTE");
        shortcut(&mut ui, "z", Modifiers::NONE);
        assert_eq!(text_of(&ui, id), "ab");
    }

    #[test]
    fn an_edit_menu_asks_the_input_what_it_can_do() {
        let mut ui = ui();
        let id = input(&mut ui, "abc");
        let state = |ui: &Ui| {
            let w = ui.widget::<TextInput>(id).unwrap();
            (w.has_selection(), w.can_undo(), w.can_redo())
        };
        assert_eq!(state(&ui), (true, false, false));
        assert!(ui.update(id, |w: &mut TextInput, cx| w.cut(cx)).unwrap());
        assert_eq!(state(&ui), (false, true, false));
        assert!(ui.update(id, |w: &mut TextInput, cx| w.undo(cx)).unwrap());
        assert_eq!(
            state(&ui),
            (true, false, true),
            "the cut text selected again"
        );
        assert!(ui.update(id, |w: &mut TextInput, cx| w.redo(cx)).unwrap());
        assert!(ui.update(id, |w: &mut TextInput, cx| w.paste(cx)).unwrap());
        assert!(!ui.update(id, |w: &mut TextInput, cx| w.copy(cx)).unwrap());
        assert_eq!(text_of(&ui, id), "abc");
    }

    #[test]
    fn altgr_still_types_the_letters_of_shortcuts() {
        let mut ui = ui();
        ui.clipboard().set_text("pasted").unwrap();
        let id = input(&mut ui, "");
        // Ctrl+Alt is AltGr on Windows: it types.
        let altgr = KeyEvent::down(Key::Character("v"))
            .with_text("v")
            .with_modifiers(Modifiers::CONTROL | Modifiers::ALT);
        ui.key(&altgr);
        assert_eq!(text_of(&ui, id), "v");
    }

    #[test]
    fn other_shortcuts_are_left_to_the_application() {
        let mut ui = ui();
        input(&mut ui, "abc");
        assert_eq!(shortcut(&mut ui, "s", Modifiers::NONE), Handled::No);
    }
}

mod grid {
    use std::fmt::Write as _;

    use densa_core::{Point, Rect};
    use densa_draw::{Canvas, InstanceBuffer};
    use densa_grid::{Align, CellStyle, CellText, Column, GridModel, Palette, SortOrder};
    use densa_text::{AtlasConfig, GlyphCache};
    use densa_ui::{Action, PointerButton, PointerEvent, PointerKind};
    use densa_widgets::DataGrid;

    use super::*;

    /// Records with ids and names, keyed by id.
    struct Fruits {
        columns: Vec<Column>,
        ids: Vec<u64>,
    }

    impl Fruits {
        fn new(rows: u64) -> Self {
            Self {
                columns: vec![
                    Column::new("Id", 4, Align::Right),
                    Column::new("Name", 10, Align::Left),
                ],
                ids: (0..rows).collect(),
            }
        }
    }

    impl GridModel for Fruits {
        fn columns(&self) -> &[Column] {
            &self.columns
        }

        fn rows(&self) -> usize {
            self.ids.len()
        }

        fn cell(
            &self,
            row: usize,
            column: usize,
            text: &mut CellText,
            palette: &Palette,
        ) -> CellStyle {
            let id = self.ids[row];
            let _ = match (column, id) {
                (0, _) => write!(text, "{id}"),
                (_, 7) => write!(text, "tab\there\nline"),
                _ => write!(text, "n{id}"),
            };
            CellStyle::new(palette.text)
        }

        fn sort(&mut self, _: usize, order: SortOrder) -> bool {
            self.ids.sort_unstable();
            if order == SortOrder::Descending {
                self.ids.reverse();
            }
            true
        }

        fn row_key(&self, row: usize) -> Option<u64> {
            self.ids.get(row).copied()
        }

        fn row_of_key(&self, key: u64) -> Option<usize> {
            self.ids.iter().position(|id| *id == key)
        }
    }

    type Grid = DataGrid<Fruits>;

    /// A focused grid of `rows` records.
    fn grid(rows: u64) -> (Ui, WidgetId) {
        let mut ui = super::ui();
        ui.resize(Size::new(300.0, 400.0));
        let id = ui
            .add(ui.root(), Grid::new(Fruits::new(rows)), Grid::style())
            .unwrap();
        ui.layout();
        ui.set_focus(Some(id));
        (ui, id)
    }

    fn rows(ui: &Ui, id: WidgetId) -> std::ops::Range<usize> {
        ui.widget::<Grid>(id).unwrap().selected_rows()
    }

    fn key(ui: &mut Ui, key: NamedKey, modifiers: Modifiers) -> Handled {
        named(ui, key, modifiers)
    }

    fn selections(ui: &mut Ui) -> Vec<ActionKind> {
        ui.actions()
            .map(|Action { kind, .. }| kind)
            .filter(|k| matches!(k, ActionKind::Selected(_)))
            .collect()
    }

    /// The shown rows' bounds, from the accessibility tree, with whether each is selected.
    fn shown(ui: &mut Ui, id: WidgetId) -> Vec<(Rect, bool)> {
        let tree = ui.access_tree();
        let table = tree.node(ui.access_id(id).unwrap()).unwrap();
        table.children()[1..]
            .iter()
            .map(|r| {
                let row = tree.node(*r).unwrap();
                (row.bounds(), row.selected() == Some(true))
            })
            .collect()
    }

    fn press(ui: &mut Ui, id: WidgetId, row: usize, button: PointerButton, shift: bool) {
        let r = shown(ui, id)[row].0;
        let at = Point::new(r.left() + 10.0, (r.top() + r.bottom()) / 2.0);
        let modifiers = if shift {
            Modifiers::SHIFT
        } else {
            Modifiers::NONE
        };
        let down = PointerEvent::new(PointerKind::Down(button), at).with_modifiers(modifiers);
        ui.pointer(down);
        ui.pointer(PointerEvent::new(PointerKind::Up(button), at));
    }

    fn update(ui: &mut Ui, id: WidgetId, f: impl FnOnce(&mut Grid, &mut densa_ui::EventCx<'_>)) {
        ui.update(id, f).unwrap();
    }

    /// Paints a frame (the grid follows its model as it paints): the colors drawn.
    fn paint(ui: &mut Ui) -> Vec<[f32; 4]> {
        let mut cache = GlyphCache::new(AtlasConfig::default());
        let mut out = InstanceBuffer::with_capacity(65_536);
        let mut canvas = Canvas::begin(&mut cache, &mut out);
        ui.paint(&mut canvas);
        out.as_slice().iter().map(|i| i.color).collect()
    }

    #[test]
    fn shift_with_the_selection_keys_extends_it_and_a_plain_key_ends_it() {
        let (mut ui, id) = grid(30);
        key(&mut ui, NamedKey::ArrowDown, Modifiers::NONE);
        key(&mut ui, NamedKey::ArrowDown, Modifiers::SHIFT);
        key(&mut ui, NamedKey::ArrowDown, Modifiers::SHIFT);
        assert_eq!(rows(&ui, id), 0..3);
        key(&mut ui, NamedKey::ArrowUp, Modifiers::SHIFT);
        assert_eq!(rows(&ui, id), 0..2, "back toward where it started");
        key(&mut ui, NamedKey::End, Modifiers::SHIFT);
        assert_eq!(rows(&ui, id), 0..30);
        key(&mut ui, NamedKey::PageUp, Modifiers::SHIFT);
        assert!(rows(&ui, id).end < 30 && rows(&ui, id).start == 0);
        assert_eq!(
            selections(&mut ui),
            [0, 1, 2, 1, 29, rows(&ui, id).end - 1].map(ActionKind::Selected)
        );
        key(&mut ui, NamedKey::ArrowDown, Modifiers::NONE);
        let row = ui.widget::<Grid>(id).unwrap().selected().unwrap();
        assert_eq!(rows(&ui, id), row..row + 1);
        // Shift with Left, Right and Enter does nothing.
        for k in [NamedKey::ArrowRight, NamedKey::Enter] {
            assert_eq!(key(&mut ui, k, Modifiers::SHIFT), Handled::No);
        }
    }

    #[test]
    fn a_shift_click_extends_from_where_the_selection_started() {
        let (mut ui, id) = grid(30);
        press(&mut ui, id, 3, PointerButton::Primary, true);
        assert_eq!(rows(&ui, id), 3..4, "nothing selected: just the row");
        press(&mut ui, id, 1, PointerButton::Primary, false);
        press(&mut ui, id, 4, PointerButton::Primary, true);
        assert_eq!(rows(&ui, id), 1..5);
        press(&mut ui, id, 0, PointerButton::Primary, true);
        assert_eq!(rows(&ui, id), 0..2);
        let selected: Vec<bool> = shown(&mut ui, id).iter().map(|r| r.1).take(4).collect();
        assert_eq!(
            selected,
            [true, true, false, false],
            "as screen readers see it"
        );
        press(&mut ui, id, 2, PointerButton::Primary, false);
        assert_eq!(rows(&ui, id), 2..3);
    }

    #[test]
    fn a_right_click_keeps_the_range_it_lands_in() {
        let (mut ui, id) = grid(30);
        update(&mut ui, id, |g, cx| g.select_rows(cx, 1, 3));
        press(&mut ui, id, 2, PointerButton::Secondary, false);
        assert_eq!(rows(&ui, id), 1..4);
        press(&mut ui, id, 5, PointerButton::Secondary, false);
        assert_eq!(rows(&ui, id), 5..6);
    }

    #[test]
    fn ctrl_c_copies_the_selected_rows_as_tab_separated_lines() {
        let (mut ui, id) = grid(30);
        update(&mut ui, id, |g, cx| g.select_rows(cx, 3, 1));
        assert_eq!(super::shortcut(&mut ui, "c", Modifiers::NONE), Handled::Yes);
        assert_eq!(clipboard(&mut ui).unwrap(), "1\tn1\n2\tn2\n3\tn3");
        update(&mut ui, id, |g, cx| g.select(cx, Some(7)));
        ui.clipboard().set_text("").unwrap();
        key(&mut ui, NamedKey::Insert, Modifiers::CONTROL);
        assert_eq!(clipboard(&mut ui).unwrap(), "7\ttab here line");
    }

    #[test]
    fn with_nothing_selected_or_no_clipboard_nothing_is_copied() {
        let (mut ui, id) = grid(30);
        ui.clipboard().set_text("before").unwrap();
        assert_eq!(super::shortcut(&mut ui, "c", Modifiers::NONE), Handled::No);
        assert_eq!(clipboard(&mut ui).unwrap(), "before");
        ui.set_clipboard(Locked);
        update(&mut ui, id, |g, cx| g.select(cx, Some(0)));
        assert_eq!(super::shortcut(&mut ui, "c", Modifiers::NONE), Handled::No);
        assert!(!ui.update(id, |g: &mut Grid, cx| g.copy(cx)).unwrap());
        // Ctrl+Shift+C is not copying.
        assert_eq!(super::shortcut(&mut ui, "c", Modifiers::SHIFT), Handled::No);
    }

    #[test]
    fn select_all_takes_every_row_without_scrolling() {
        let (mut ui, id) = grid(100);
        update(&mut ui, id, |g, cx| g.scroll_to_row(cx, 40));
        selections(&mut ui);
        assert_eq!(super::shortcut(&mut ui, "a", Modifiers::NONE), Handled::Yes);
        assert_eq!(rows(&ui, id), 0..100);
        assert_eq!(ui.widget::<Grid>(id).unwrap().first_row(), 40);
        assert_eq!(selections(&mut ui), [ActionKind::Selected(0)]);
        super::shortcut(&mut ui, "a", Modifiers::NONE);
        assert!(selections(&mut ui).is_empty(), "already selected");
        let (mut empty, grid) = grid(0);
        assert_eq!(
            super::shortcut(&mut empty, "a", Modifiers::NONE),
            Handled::No
        );
        assert_eq!(rows(&empty, grid), 0..0);
    }

    #[test]
    fn a_range_follows_its_records_and_ends_with_its_start() {
        let (mut ui, id) = grid(30);
        update(&mut ui, id, |g, cx| g.select_rows(cx, 2, 4));
        // Record 0 goes: the range's records move up a row.
        update(&mut ui, id, |g, cx| {
            g.model_mut(cx).ids.remove(0);
        });
        paint(&mut ui);
        assert_eq!(rows(&ui, id), 1..4);
        // The record the range started from goes: the selected row is left.
        update(&mut ui, id, |g, cx| {
            g.model_mut(cx).ids.retain(|id| *id != 2);
        });
        paint(&mut ui);
        assert_eq!(rows(&ui, id), 2..3);
        assert_eq!(ui.widget::<Grid>(id).unwrap().selected(), Some(2));
    }

    #[test]
    fn sorting_leaves_the_selected_record_alone_selected() {
        let (mut ui, id) = grid(30);
        update(&mut ui, id, |g, cx| g.select_rows(cx, 1, 3));
        let title = shown(&mut ui, id)[0].0;
        let header = Point::new(title.left() + 10.0, title.top() - 5.0);
        for kind in [
            PointerKind::Down(PointerButton::Primary),
            PointerKind::Up(PointerButton::Primary),
        ] {
            ui.pointer(PointerEvent::new(kind, header));
        }
        ui.layout();
        let grid = ui.widget::<Grid>(id).unwrap();
        assert_eq!(grid.sort_order(), Some((0, SortOrder::Ascending)));
        assert_eq!(rows(&ui, id), 3..4, "record 3, alone");
    }

    #[test]
    fn rows_given_past_the_end_are_the_last() {
        let (mut ui, id) = grid(10);
        update(&mut ui, id, |g, cx| g.select_rows(cx, 50, 8));
        assert_eq!(rows(&ui, id), 8..10);
        update(&mut ui, id, |g, cx| g.select_rows(cx, 2, 50));
        assert_eq!(rows(&ui, id), 2..10);
        assert_eq!(ui.widget::<Grid>(id).unwrap().selected(), Some(9));
        let (mut empty, grid) = grid(0);
        update(&mut empty, grid, |g, cx| g.select_rows(cx, 0, 0));
        assert_eq!(rows(&empty, grid), 0..0);
    }

    #[test]
    fn every_selected_row_is_highlighted() {
        let (mut ui, id) = grid(30);
        let h = ui.theme().selection;
        let color = [h.r, h.g, h.b, h.a];
        let painted = |ui: &mut Ui| paint(ui).iter().filter(|c| **c == color).count();
        update(&mut ui, id, |g, cx| g.select(cx, Some(2)));
        let one = painted(&mut ui);
        assert!(one > 0);
        update(&mut ui, id, |g, cx| g.select_rows(cx, 2, 4));
        assert_eq!(painted(&mut ui), 3 * one);
    }
}
