//! The data grid's edge cases: column counts, huge tables, touchpads, keyed selections,
//! sorting on release, and what screen readers can ask of it.

use std::cell::Cell;
use std::fmt::Write as _;
use std::rc::Rc;

use densa_core::{Point, Rect, Size};
use densa_draw::{Canvas, InstanceBuffer};
use densa_grid::{Align, CellStyle, CellText, Color, Column, GridModel, Palette, SortOrder};
use densa_paragraph::TextSystem;
use densa_text::{AtlasConfig, GlyphCache};
use densa_ui::{
    AccessAction, AccessId, Action, ActionKind, Key, KeyEvent, Modifiers, NamedKey, PointerButton,
    PointerEvent, PointerKind, ScrollDelta, Theme, Ui, WidgetId,
};
use densa_widgets::DataGrid;

/// Numbered records, each with an id; keyed or not; counts the cells it is asked for.
struct Records {
    columns: Vec<Column>,
    /// Record ids, in the order shown.
    ids: Vec<u64>,
    /// Rows past `ids` (computed records, for huge tables).
    extra: usize,
    keyed: bool,
    /// The row whose cells the model fills itself (a flash).
    lit: Option<usize>,
    asked: Rc<Cell<usize>>,
    positive: Rc<Cell<Option<Color>>>,
    state: Option<(usize, SortOrder)>,
}

impl Records {
    fn new(rows: usize) -> Self {
        Self {
            columns: vec![
                Column::new("Id", 8, Align::Right),
                Column::new("Name", 10, Align::Left),
                Column::new("Value", 10, Align::Right),
            ],
            ids: (0..rows as u64).collect(),
            extra: 0,
            keyed: false,
            lit: None,
            asked: Rc::default(),
            positive: Rc::default(),
            state: None,
        }
    }

    fn id(&self, row: usize) -> u64 {
        self.ids.get(row).copied().unwrap_or(row as u64)
    }
}

impl GridModel for Records {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn rows(&self) -> usize {
        self.ids.len() + self.extra
    }

    fn cell(&self, row: usize, column: usize, text: &mut CellText, palette: &Palette) -> CellStyle {
        self.asked.set(self.asked.get() + 1);
        self.positive.set(Some(palette.positive));
        let id = self.id(row);
        let _ = match column {
            0 => write!(text, "{id}"),
            1 => write!(text, "rec{id}"),
            _ => write!(text, "{}", id * 3),
        };
        let style = CellStyle::new(palette.text);
        match self.lit {
            Some(lit) if lit == row => style.with_background(palette.positive),
            _ => style,
        }
    }

    fn sort(&mut self, column: usize, order: SortOrder) -> bool {
        // Every column's values grow with the id (names compare as strings, close enough).
        self.ids.sort_unstable();
        if order == SortOrder::Descending {
            self.ids.reverse();
        }
        // A model that reports its sort keeps the report up to date.
        self.state = self.state.map(|_| (column, order));
        true
    }

    fn sort_state(&self) -> Option<(usize, SortOrder)> {
        self.state
    }

    fn row_key(&self, row: usize) -> Option<u64> {
        self.keyed.then(|| self.id(row))
    }

    fn row_of_key(&self, key: u64) -> Option<usize> {
        self.ids.iter().position(|id| *id == key)
    }
}

fn grid_of(widget: DataGrid<Records>, size: Size) -> (Ui, WidgetId) {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(size);
    let id = ui
        .add(ui.root(), widget, DataGrid::<Records>::style())
        .unwrap();
    ui.layout();
    (ui, id)
}

fn grid(rows: usize) -> (Ui, WidgetId) {
    grid_of(DataGrid::new(Records::new(rows)), Size::new(400.0, 300.0))
}

fn view(ui: &Ui, id: WidgetId) -> &DataGrid<Records> {
    ui.widget::<DataGrid<Records>>(id).unwrap()
}

fn pointer(ui: &mut Ui, kind: PointerKind, at: Point) {
    ui.pointer(PointerEvent::new(kind, at));
}

fn click(ui: &mut Ui, at: Point) {
    pointer(ui, PointerKind::Down(PointerButton::Primary), at);
    pointer(ui, PointerKind::Up(PointerButton::Primary), at);
}

fn actions(ui: &mut Ui) -> Vec<ActionKind> {
    ui.actions().map(|Action { kind, .. }| kind).collect()
}

fn middle(r: Rect) -> Point {
    Point::new((r.left() + r.right()) / 2.0, (r.top() + r.bottom()) / 2.0)
}

/// The table's node id, its header cells' bounds, and the shown rows' (id, bounds).
fn nodes(ui: &mut Ui, id: WidgetId) -> (AccessId, Vec<Rect>, Vec<(AccessId, Rect)>) {
    let tree = ui.access_tree();
    let table_id = ui.access_id(id).unwrap();
    let table = tree.node(table_id).unwrap();
    let header = tree.node(table.children()[0]).unwrap();
    let headers = header
        .children()
        .iter()
        .map(|c| tree.node(*c).unwrap().bounds())
        .collect();
    let rows = table.children()[1..]
        .iter()
        .map(|r| (*r, tree.node(*r).unwrap().bounds()))
        .collect();
    (table_id, headers, rows)
}

fn frame(ui: &mut Ui) -> Vec<Rect> {
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(65_536);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    ui.paint(&mut canvas);
    out.as_slice()
        .iter()
        .map(|i| Rect::new(i.rect[0], i.rect[1], i.rect[2], i.rect[3]))
        .collect()
}

#[test]
fn every_column_or_more_can_be_frozen() {
    for frozen in [3, 9] {
        let widget = DataGrid::new(Records::new(20)).with_frozen_columns(frozen);
        let (mut ui, id) = grid_of(widget, Size::new(300.0, 200.0));
        frame(&mut ui);
        let (_, headers, rows) = nodes(&mut ui, id);
        click(&mut ui, middle(headers[0]));
        click(&mut ui, middle(rows[1].1));
        assert_eq!(view(&ui, id).selected(), Some(1), "frozen {frozen}");
    }
}

#[test]
fn the_wheel_scrolls_whole_rows_in_a_hundred_million() {
    let mut records = Records::new(0);
    records.extra = 100_000_000;
    let (mut ui, id) = grid_of(DataGrid::new(records), Size::new(400.0, 300.0));
    for (start, notches, expected) in [
        (80_000_001, 5.0, 80_000_016),
        (20_000_001, -5.0, 19_999_986),
    ] {
        ui.update::<DataGrid<Records>, _>(id, |g, cx| g.scroll_to_row(cx, start));
        let wheel = PointerKind::Scroll(ScrollDelta::Lines(0.0, notches));
        pointer(&mut ui, wheel, Point::new(100.0, 100.0));
        assert_eq!(view(&ui, id).first_row(), expected);
    }
}

#[test]
fn touchpad_pixels_add_up_to_rows_and_are_not_passed_on() {
    let (mut ui, id) = grid(200);
    let rows = nodes(&mut ui, id).2;
    let row_height = rows[1].1.top() - rows[0].1.top();
    let mut handled = 0;
    for _ in 0..20 {
        let small = PointerKind::Scroll(ScrollDelta::Pixels(0.0, 4.0));
        let event = PointerEvent::new(small, Point::new(100.0, 100.0));
        handled += usize::from(ui.pointer(event) == densa_ui::Handled::Yes);
    }
    assert_eq!(handled, 20, "the grid used every event");
    let expected = (80.0 / row_height).floor() as usize;
    assert_eq!(view(&ui, id).first_row(), expected);
}

#[test]
fn reading_the_columns_again_mid_resize_ends_the_resize() {
    let (mut ui, id) = grid(10);
    let headers = nodes(&mut ui, id).1;
    let edge = Point::new(headers[2].right(), middle(headers[2]).y);
    pointer(&mut ui, PointerKind::Down(PointerButton::Primary), edge);
    ui.update::<DataGrid<Records>, _>(id, |g, cx| {
        g.model_mut(cx).columns.truncate(1);
        g.reset_columns(cx);
    });
    pointer(
        &mut ui,
        PointerKind::Move,
        Point::new(edge.x + 50.0, edge.y),
    );
    pointer(&mut ui, PointerKind::Up(PointerButton::Primary), edge);
    frame(&mut ui);
    assert_eq!(view(&ui, id).columns().len(), 1);
}

#[test]
fn the_last_row_ends_flush_with_the_grid_when_rows_fit_exactly() {
    let (mut ui, id) = grid(100);
    let (_, _, rows) = nodes(&mut ui, id);
    let row_height = rows[1].1.top() - rows[0].1.top();
    let top = rows[0].1.top() - ui.rect(id).unwrap().top();
    // A grid holding exactly ten rows under its header.
    let height = top + 10.0 * row_height;
    let (mut ui, id) = grid_of(DataGrid::new(Records::new(100)), Size::new(400.0, height));
    assert_eq!(view(&ui, id).whole_rows(), 10);
    ui.update::<DataGrid<Records>, _>(id, |g, cx| g.scroll_to_row(cx, 1_000));
    assert_eq!(view(&ui, id).first_row(), 90);
    let last = nodes(&mut ui, id).2.last().unwrap().1;
    assert_eq!(last.bottom(), height, "no blank row under the last");
}

#[test]
fn the_sort_arrow_stays_inside_the_grid() {
    let mut records = Records::new(20);
    records.columns.push(Column::new("Far", 60, Align::Left));
    records.state = Some((3, SortOrder::Ascending));
    let (mut ui, id) = grid_of(DataGrid::new(records), Size::new(300.0, 200.0));
    let grid_rect = ui.rect(id).unwrap();
    let outside = frame(&mut ui)
        .into_iter()
        .filter(|r| !r.is_empty() && (r.left() < grid_rect.left() || r.right() > grid_rect.right()))
        .count();
    assert_eq!(outside, 0);
}

#[test]
fn screen_readers_scroll_the_table_and_choose_rows_by_their_cells() {
    let (mut ui, id) = grid(200);
    let (table, _, _) = nodes(&mut ui, id);
    assert!(ui.access_action(table, AccessAction::ScrollDown, None));
    let page = view(&ui, id).whole_rows();
    assert_eq!(view(&ui, id).first_row(), page);
    assert!(ui.access_action(table, AccessAction::ScrollUp, None));
    assert!(
        !ui.access_action(table, AccessAction::ScrollUp, None),
        "at the top"
    );
    // A click on a cell chooses its row.
    let tree = ui.access_tree();
    let row = tree.node(table).unwrap().children()[3];
    let cell = tree.node(row).unwrap().children()[1];
    assert!(ui.access_action(cell, AccessAction::Click, None));
    assert_eq!(view(&ui, id).selected(), Some(2));
    // With focus, the selected row is where screen readers are.
    ui.set_focus(Some(id));
    assert_eq!(ui.access_tree().focus(), row);
}

#[test]
fn a_cell_keeps_its_id_while_its_row_scrolls() {
    let (mut ui, id) = grid(200);
    let cell_of = |ui: &mut Ui, row_index: usize| {
        let tree = ui.access_tree();
        let table = tree.node(ui.access_id(id).unwrap()).unwrap();
        let row = tree.node(table.children()[1 + row_index]).unwrap();
        row.children()[0]
    };
    let before = cell_of(&mut ui, 3);
    let wheel = PointerKind::Scroll(ScrollDelta::Lines(0.0, 1.0));
    pointer(&mut ui, wheel, Point::new(100.0, 100.0));
    // Three rows on: the record that was fourth is now first.
    assert_eq!(cell_of(&mut ui, 0), before);
}

#[test]
fn a_keyed_selection_follows_its_record_through_sorts_and_inserts() {
    let mut records = Records::new(50);
    records.keyed = true;
    records.ids.reverse();
    let (mut ui, id) = grid_of(DataGrid::new(records), Size::new(400.0, 300.0));
    let rows = nodes(&mut ui, id).2;
    click(&mut ui, middle(rows[2].1));
    assert_eq!(view(&ui, id).selected(), Some(2), "record 47");
    let headers = nodes(&mut ui, id).1;
    click(&mut ui, middle(headers[0]));
    assert_eq!(
        view(&ui, id).selected(),
        Some(47),
        "record 47, sorted ascending"
    );
    ui.update::<DataGrid<Records>, _>(id, |g, cx| {
        let ids = &mut g.model_mut(cx).ids;
        ids.splice(0..0, 100..105);
    });
    frame(&mut ui);
    assert_eq!(
        view(&ui, id).selected(),
        Some(52),
        "five records inserted before it"
    );
}

#[test]
fn a_selection_without_keys_is_cleared_by_sorting() {
    let (mut ui, id) = grid(50);
    let rows = nodes(&mut ui, id).2;
    click(&mut ui, middle(rows[2].1));
    let headers = nodes(&mut ui, id).1;
    click(&mut ui, middle(headers[0]));
    assert_eq!(view(&ui, id).selected(), None);
}

#[test]
fn a_title_sorts_on_release_not_when_the_press_moves_away() {
    let (mut ui, id) = grid(20);
    let headers = nodes(&mut ui, id).1;
    let title = middle(headers[0]);
    pointer(&mut ui, PointerKind::Down(PointerButton::Primary), title);
    assert_eq!(view(&ui, id).sort_order(), None, "not on press");
    pointer(
        &mut ui,
        PointerKind::Move,
        Point::new(title.x, title.y + 60.0),
    );
    pointer(
        &mut ui,
        PointerKind::Up(PointerButton::Primary),
        Point::new(title.x, title.y + 60.0),
    );
    assert_eq!(view(&ui, id).sort_order(), None, "moved away");
    click(&mut ui, title);
    assert_eq!(view(&ui, id).sort_order(), Some((0, SortOrder::Ascending)));
}

#[test]
fn the_models_own_sort_shows_and_a_click_reverses_it() {
    let mut records = Records::new(20);
    records.state = Some((0, SortOrder::Ascending));
    let (mut ui, id) = grid_of(DataGrid::new(records), Size::new(400.0, 300.0));
    assert_eq!(view(&ui, id).sort_order(), Some((0, SortOrder::Ascending)));
    let headers = nodes(&mut ui, id).1;
    click(&mut ui, middle(headers[0]));
    // The model's ascending sort was the last: the click reversed it.
    let first: u64 = view(&ui, id).model().ids[0];
    assert_eq!(first, 19);
    assert_eq!(view(&ui, id).sort_order(), Some((0, SortOrder::Descending)));
}

#[test]
fn enter_and_double_clicks_activate_and_modified_keys_are_left_alone() {
    let (mut ui, id) = grid(20);
    let rows = nodes(&mut ui, id).2;
    let at = middle(rows[1].1);
    pointer(&mut ui, PointerKind::Down(PointerButton::Primary), at);
    pointer(&mut ui, PointerKind::Up(PointerButton::Primary), at);
    pointer(&mut ui, PointerKind::Down(PointerButton::Primary), at);
    pointer(&mut ui, PointerKind::Up(PointerButton::Primary), at);
    ui.key(&KeyEvent::down(Key::Named(NamedKey::Enter)));
    let shift_down =
        KeyEvent::down(Key::Named(NamedKey::ArrowDown)).with_modifiers(Modifiers::SHIFT);
    ui.key(&shift_down);
    assert_eq!(
        actions(&mut ui),
        [
            ActionKind::Selected(1),
            ActionKind::Submitted,
            ActionKind::Submitted
        ]
    );
    assert_eq!(view(&ui, id).selected(), Some(1));
}

#[test]
fn the_selected_row_keeps_a_fill_its_model_draws() {
    let mut records = Records::new(20);
    records.lit = Some(1);
    let (mut ui, id) = grid_of(DataGrid::new(records), Size::new(400.0, 300.0));
    let rows = nodes(&mut ui, id).2;
    click(&mut ui, middle(rows[1].1));
    let positive = Palette::new().positive;
    let fills = {
        let mut cache = GlyphCache::new(AtlasConfig::default());
        let mut out = InstanceBuffer::with_capacity(65_536);
        let mut canvas = Canvas::begin(&mut cache, &mut out);
        ui.paint(&mut canvas);
        out.as_slice()
            .iter()
            .filter(|i| i.color == positive.to_array())
            .count()
    };
    assert!(
        fills >= 3,
        "the row's own fill is drawn under every cell ({fills})"
    );
    let _ = id;
}

#[test]
fn frozen_columns_are_not_asked_for_twice() {
    let widget = DataGrid::new(Records::new(100)).with_frozen_columns(1);
    let (mut ui, id) = grid_of(widget, Size::new(400.0, 300.0));
    let asked = Rc::clone(&view(&ui, id).model().asked);
    asked.set(0);
    frame(&mut ui);
    let shown = view(&ui, id).visible_rows();
    assert!(
        asked.get() <= shown * 3,
        "asked {} cells of {shown} rows",
        asked.get()
    );
}

#[test]
fn the_light_theme_gets_darker_rise_and_fall_colors() {
    let (mut ui, id) = grid(5);
    frame(&mut ui);
    let dark = view(&ui, id).model().positive.get();
    ui.set_theme(Theme::light());
    frame(&mut ui);
    let light = view(&ui, id).model().positive.get();
    assert_ne!(dark, light);
}

#[test]
fn reading_the_columns_again_drops_a_resize_in_progress() {
    let (mut ui, id) = grid(10);
    let headers = nodes(&mut ui, id).1;
    let edge = Point::new(headers[0].right(), middle(headers[0]).y);
    pointer(&mut ui, PointerKind::Down(PointerButton::Primary), edge);
    ui.update::<DataGrid<Records>, _>(id, |g, cx| g.reset_columns(cx));
    pointer(
        &mut ui,
        PointerKind::Move,
        Point::new(edge.x + 60.0, edge.y),
    );
    ui.layout();
    assert_eq!(
        view(&ui, id).columns()[0].chars,
        8,
        "the column kept its width"
    );
}

#[test]
fn a_grid_shorter_than_a_row_still_shows_the_last_row() {
    let (mut ui, id) = grid_of(DataGrid::new(Records::new(30)), Size::new(300.0, 30.0));
    ui.update::<DataGrid<Records>, _>(id, |g, cx| g.scroll_to_row(cx, 1_000));
    assert_eq!(view(&ui, id).first_row(), 29);
}

#[test]
fn moving_past_the_last_whole_row_scrolls_by_one() {
    let (mut ui, id) = grid(100);
    let whole = view(&ui, id).whole_rows();
    ui.update::<DataGrid<Records>, _>(id, |g, cx| g.select(cx, Some(whole - 1)));
    ui.set_focus(Some(id));
    assert_eq!(
        view(&ui, id).first_row(),
        0,
        "the last whole row is in view already"
    );
    ui.key(&KeyEvent::down(Key::Named(NamedKey::ArrowDown)));
    assert_eq!(view(&ui, id).first_row(), 1);
    ui.key(&KeyEvent::down(Key::Named(NamedKey::PageDown)));
    assert_eq!(view(&ui, id).selected(), Some(2 * whole));
}

#[test]
fn only_the_columns_that_show_are_asked_for() {
    let mut records = Records::new(100);
    records.columns = (0..10)
        .map(|_| Column::new("C", 20, Align::Right))
        .collect();
    let widget = DataGrid::new(records).with_frozen_columns(1);
    let (mut ui, id) = grid_of(widget, Size::new(300.0, 300.0));
    let asked = Rc::clone(&view(&ui, id).model().asked);
    asked.set(0);
    frame(&mut ui);
    let shown = view(&ui, id).visible_rows();
    // 300 px hold the frozen column and about two of 20 characters more.
    assert!(
        asked.get() <= shown * 4,
        "asked {} cells of {shown} rows",
        asked.get()
    );
}

#[test]
fn the_sort_arrow_sits_inside_its_column_away_from_the_title() {
    let mut records = Records::new(10);
    records.state = Some((0, SortOrder::Ascending));
    let (mut ui, id) = grid_of(DataGrid::new(records), Size::new(400.0, 300.0));
    let header = nodes(&mut ui, id).1[0];
    let accent = Theme::dark().accent.to_array();
    let arrows: Vec<Rect> = {
        let mut cache = GlyphCache::new(AtlasConfig::default());
        let mut out = InstanceBuffer::with_capacity(65_536);
        let mut canvas = Canvas::begin(&mut cache, &mut out);
        ui.paint(&mut canvas);
        out.as_slice()
            .iter()
            .filter(|i| i.color == accent)
            .map(|i| Rect::new(i.rect[0], i.rect[1], i.rect[2], i.rect[3]))
            .collect()
    };
    assert!(!arrows.is_empty());
    // The Id column aligns right: the arrow is in its left half.
    let mid = (header.left() + header.right()) / 2.0;
    assert!(
        arrows
            .iter()
            .all(|r| r.left() >= header.left() && r.right() <= mid),
        "{arrows:?} in {header:?}"
    );
}

#[test]
fn turning_a_touchpad_back_drops_what_was_left_the_other_way() {
    let (mut ui, id) = grid(200);
    ui.update::<DataGrid<Records>, _>(id, |g, cx| g.scroll_to_row(cx, 10));
    let rows = nodes(&mut ui, id).2;
    let row_height = rows[1].1.top() - rows[0].1.top();
    let by = |ui: &mut Ui, rows: f32| {
        let kind = PointerKind::Scroll(ScrollDelta::Pixels(0.0, rows * row_height));
        ui.pointer(PointerEvent::new(kind, Point::new(100.0, 100.0)));
    };
    by(&mut ui, 0.6);
    by(&mut ui, -0.6);
    assert_eq!(view(&ui, id).first_row(), 10);
    by(&mut ui, -0.6);
    assert_eq!(view(&ui, id).first_row(), 9, "1.2 rows up, not 0.6");
}

#[test]
fn dragging_the_thumb_to_the_bottom_of_a_hundred_million_reaches_the_end() {
    let mut records = Records::new(0);
    records.extra = 100_000_000;
    let (mut ui, id) = grid_of(DataGrid::new(records), Size::new(400.0, 300.0));
    let rect = ui.rect(id).unwrap();
    let top = Point::new(rect.right() - 3.0, rect.top() + 40.0);
    pointer(&mut ui, PointerKind::Down(PointerButton::Primary), top);
    pointer(
        &mut ui,
        PointerKind::Move,
        Point::new(top.x, rect.bottom() + 100.0),
    );
    pointer(&mut ui, PointerKind::Up(PointerButton::Primary), top);
    let last = 100_000_000 - view(&ui, id).whole_rows();
    assert_eq!(view(&ui, id).first_row(), last);
}

#[test]
fn a_title_press_left_of_the_frozen_columns_sorts_nothing() {
    let widget = DataGrid::new(Records::new(10)).with_frozen_columns(1);
    let (mut ui, id) = grid_of(widget, Size::new(100.0, 200.0));
    // Small text: the padding left of the first column is wider than half a gap, so a strip
    // there belongs to no column, frozen or not.
    let mut theme = Theme::dark();
    theme.font_size = 8.0;
    ui.set_theme(theme);
    ui.layout();
    let sideways = PointerKind::Scroll(ScrollDelta::Lines(3.0, 0.0));
    pointer(&mut ui, sideways, Point::new(50.0, 100.0));
    let header = nodes(&mut ui, id).1[0];
    let left = ui.rect(id).unwrap().left();
    // A scrolled column lies under that strip.
    click(&mut ui, Point::new(left + 1.0, middle(header).y));
    assert_eq!(view(&ui, id).sort_order(), None);
    click(&mut ui, middle(header));
    assert_eq!(view(&ui, id).sort_order(), Some((0, SortOrder::Ascending)));
}

#[test]
fn enter_without_a_selection_activates_nothing() {
    let (mut ui, id) = grid(10);
    ui.set_focus(Some(id));
    ui.key(&KeyEvent::down(Key::Named(NamedKey::Enter)));
    assert!(actions(&mut ui).is_empty());
}

#[test]
fn the_table_offers_only_the_scrolls_it_can_make() {
    let (mut ui, id) = grid(200);
    let tree = ui.access_tree();
    let table = tree.node(ui.access_id(id).unwrap()).unwrap();
    assert!(!table.has_action(AccessAction::ScrollUp), "at the top");
    assert!(table.has_action(AccessAction::ScrollDown));
}
