//! The data grid widget driven headless: scrolling, selecting, sorting, resizing, frozen
//! columns, and what screen readers see.

use std::cell::Cell;
use std::fmt::Write as _;
use std::rc::Rc;

use densa_core::{Point, Rect, Size};
use densa_draw::{Canvas, InstanceBuffer};
use densa_grid::{Align, CellStyle, CellText, Color, Column, GridModel, SortOrder};
use densa_paragraph::TextSystem;
use densa_text::{AtlasConfig, GlyphCache};
use densa_ui::{
    AccessAction, AccessId, Action, ActionKind, Key, KeyEvent, NamedKey, PointerButton,
    PointerEvent, PointerKind, Role, ScrollDelta, Ui, WidgetId,
};
use densa_widgets::DataGrid;

/// Instruments and prices; sorts by either column; counts the cells it was asked for.
struct Quotes {
    columns: Vec<Column>,
    rows: Vec<(String, f64)>,
    asked: Rc<Cell<usize>>,
    sorts: bool,
}

impl Quotes {
    fn new(rows: usize) -> Self {
        let rows = (0..rows)
            .map(|i| (format!("S{i:04}"), (i * 37 % 1000) as f64 / 10.0))
            .collect();
        Self {
            columns: vec![
                Column::new("Symbol", 8, Align::Left),
                Column::new("Price", 10, Align::Right),
                Column::new("Venue", 12, Align::Left),
                Column::new("Note", 30, Align::Left),
            ],
            rows,
            asked: Rc::default(),
            sorts: true,
        }
    }
}

impl GridModel for Quotes {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn rows(&self) -> usize {
        self.rows.len()
    }

    fn cell(
        &self,
        row: usize,
        column: usize,
        text: &mut CellText,
        _: &densa_grid::Palette,
    ) -> CellStyle {
        self.asked.set(self.asked.get() + 1);
        let (symbol, price) = &self.rows[row];
        let _ = match column {
            0 => text.write_str(symbol),
            1 => write!(text, "{price:.2}"),
            2 => text.write_str("XNAS"),
            _ => text.write_str("-"),
        };
        CellStyle::new(Color::WHITE)
    }

    fn sort(&mut self, column: usize, order: SortOrder) -> bool {
        if !self.sorts {
            return false;
        }
        match column {
            0 => self.rows.sort_by(|a, b| a.0.cmp(&b.0)),
            1 => self.rows.sort_by(|a, b| a.1.total_cmp(&b.1)),
            _ => return false,
        }
        if order == SortOrder::Descending {
            self.rows.reverse();
        }
        true
    }
}

fn grid(rows: usize) -> (Ui, WidgetId) {
    grid_of(DataGrid::new(Quotes::new(rows)), Size::new(400.0, 300.0))
}

fn grid_of(widget: DataGrid<Quotes>, size: Size) -> (Ui, WidgetId) {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(size);
    let id = ui
        .add(ui.root(), widget, DataGrid::<Quotes>::style())
        .unwrap();
    ui.layout();
    (ui, id)
}

fn view(ui: &Ui, id: WidgetId) -> &DataGrid<Quotes> {
    ui.widget::<DataGrid<Quotes>>(id).unwrap()
}

fn pointer(ui: &mut Ui, kind: PointerKind, at: Point) {
    ui.pointer(PointerEvent::new(kind, at));
}

fn click(ui: &mut Ui, at: Point) {
    pointer(ui, PointerKind::Down(PointerButton::Primary), at);
    pointer(ui, PointerKind::Up(PointerButton::Primary), at);
}

fn key(ui: &mut Ui, key: NamedKey) {
    ui.key(&KeyEvent::down(Key::Named(key)));
}

fn actions(ui: &mut Ui) -> Vec<ActionKind> {
    ui.actions().map(|Action { kind, .. }| kind).collect()
}

fn middle(r: Rect) -> Point {
    Point::new((r.left() + r.right()) / 2.0, (r.top() + r.bottom()) / 2.0)
}

/// What screen readers see: the header cells' bounds and the shown rows (label, selected,
/// bounds).
struct Seen {
    headers: Vec<(String, Rect, Option<bool>)>,
    rows: Vec<(String, bool, Rect, AccessId)>,
}

fn seen(ui: &mut Ui, id: WidgetId) -> Seen {
    let tree = ui.access_tree();
    let table = tree.node(ui.access_id(id).unwrap()).unwrap();
    assert_eq!(table.role(), Role::Table);
    let header = tree.node(table.children()[0]).unwrap();
    let headers = header
        .children()
        .iter()
        .map(|c| {
            let cell = tree.node(*c).unwrap();
            assert_eq!(cell.role(), Role::ColumnHeader);
            (
                cell.label().unwrap().to_owned(),
                cell.bounds(),
                cell.selected(),
            )
        })
        .collect();
    let rows = table.children()[1..]
        .iter()
        .map(|r| {
            let row = tree.node(*r).unwrap();
            assert_eq!(row.role(), Role::Row);
            assert_eq!(row.label(), None, "a row reads as its cells");
            let cells: Vec<&str> = row
                .children()
                .iter()
                .map(|c| tree.node(*c).unwrap().value().unwrap())
                .collect();
            (
                cells.join(", "),
                row.selected() == Some(true),
                row.bounds(),
                *r,
            )
        })
        .collect();
    Seen { headers, rows }
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
fn only_the_rows_on_screen_are_asked_for() {
    let (mut ui, id) = grid(100_000);
    let asked = Rc::clone(&view(&ui, id).model().asked);
    frame(&mut ui);
    let shown = view(&ui, id).visible_rows();
    assert!(shown > 5 && shown < 40, "{shown} rows fit 300 px");
    // Four columns of each shown row, painted twice at most (frozen columns are painted
    // over the scrolled ones), whatever the row count.
    assert!(asked.get() <= shown * 4 * 2, "asked {} cells", asked.get());
}

#[test]
fn the_wheel_scrolls_rows_and_stops_at_the_ends() {
    let (mut ui, id) = grid(100);
    let wheel = |dy| PointerKind::Scroll(ScrollDelta::Lines(0.0, dy));
    pointer(&mut ui, wheel(1.0), Point::new(100.0, 100.0));
    assert_eq!(view(&ui, id).first_row(), 3);
    pointer(&mut ui, wheel(-10.0), Point::new(100.0, 100.0));
    assert_eq!(view(&ui, id).first_row(), 0);
    pointer(&mut ui, wheel(1000.0), Point::new(100.0, 100.0));
    let last = seen(&mut ui, id).rows.last().unwrap().0.clone();
    assert!(
        last.starts_with("S0099"),
        "the last row is on screen: {last}"
    );
}

#[test]
fn clicks_and_keys_select_rows_and_keep_them_in_view() {
    let (mut ui, id) = grid(100);
    let rows = seen(&mut ui, id).rows;
    click(&mut ui, middle(rows[2].2));
    assert_eq!(view(&ui, id).selected(), Some(2));
    assert_eq!(ui.focus(), Some(id), "a click focuses the grid");
    key(&mut ui, NamedKey::ArrowDown);
    key(&mut ui, NamedKey::ArrowUp);
    key(&mut ui, NamedKey::ArrowUp);
    assert_eq!(view(&ui, id).selected(), Some(1));
    key(&mut ui, NamedKey::End);
    assert_eq!(view(&ui, id).selected(), Some(99));
    let shown = seen(&mut ui, id).rows;
    assert!(shown
        .iter()
        .any(|(label, selected, _, _)| *selected && label.starts_with("S0099")));
    key(&mut ui, NamedKey::PageUp);
    let page = view(&ui, id).visible_rows() - 1;
    assert_eq!(view(&ui, id).selected(), Some(99 - page));
    key(&mut ui, NamedKey::Home);
    assert_eq!(view(&ui, id).first_row(), 0);
    let selected: Vec<_> = actions(&mut ui)
        .into_iter()
        .filter_map(|a| match a {
            ActionKind::Selected(row) => Some(row),
            _ => None,
        })
        .collect();
    assert_eq!(selected, [2, 3, 2, 1, 99, 99 - page, 0]);
}

#[test]
fn a_title_click_sorts_and_another_reverses() {
    let (mut ui, id) = grid(50);
    let price = seen(&mut ui, id).headers[1].1;
    let rows = seen(&mut ui, id).rows;
    click(&mut ui, middle(rows[0].2));
    actions(&mut ui);
    click(&mut ui, middle(price));
    assert_eq!(view(&ui, id).sort_order(), Some((1, SortOrder::Ascending)));
    assert_eq!(
        view(&ui, id).selected(),
        None,
        "a position means another row now"
    );
    let prices: Vec<f64> = view(&ui, id).model().rows.iter().map(|r| r.1).collect();
    assert!(prices.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(actions(&mut ui), [ActionKind::Changed]);
    let after = seen(&mut ui, id);
    assert_eq!(
        after.headers[1].0, "Price, sorted ascending",
        "screen readers hear it"
    );
    assert_eq!(
        after.headers[1].2, None,
        "a sorted column is not a selection"
    );
    click(&mut ui, middle(price));
    assert_eq!(view(&ui, id).sort_order(), Some((1, SortOrder::Descending)));
    let prices: Vec<f64> = view(&ui, id).model().rows.iter().map(|r| r.1).collect();
    assert!(prices.windows(2).all(|w| w[0] >= w[1]));
}

#[test]
fn a_model_that_does_not_sort_keeps_its_order() {
    let mut quotes = Quotes::new(20);
    quotes.sorts = false;
    let (mut ui, id) = grid_of(DataGrid::new(quotes), Size::new(400.0, 300.0));
    let price = seen(&mut ui, id).headers[1].1;
    click(&mut ui, middle(price));
    assert_eq!(view(&ui, id).sort_order(), None);
    assert!(actions(&mut ui).is_empty());
}

#[test]
fn dragging_a_title_edge_resizes_its_column() {
    let (mut ui, id) = grid(10);
    let before = seen(&mut ui, id).headers[0].1;
    let edge = Point::new(before.right(), middle(before).y);
    pointer(&mut ui, PointerKind::Move, edge);
    assert_eq!(ui.cursor(), densa_ui::Cursor::ResizeColumn);
    pointer(&mut ui, PointerKind::Down(PointerButton::Primary), edge);
    pointer(
        &mut ui,
        PointerKind::Move,
        Point::new(edge.x + 40.0, edge.y),
    );
    pointer(&mut ui, PointerKind::Up(PointerButton::Primary), edge);
    ui.layout();
    let after = seen(&mut ui, id).headers[0].1;
    assert!(
        after.width() > before.width() + 30.0,
        "{before:?} -> {after:?}"
    );
    assert!(view(&ui, id).columns()[0].chars > 8);
    assert_eq!(view(&ui, id).sort_order(), None, "resizing does not sort");
}

#[test]
fn frozen_columns_stay_while_the_others_scroll_sideways() {
    let widget = DataGrid::new(Quotes::new(10)).with_frozen_columns(1);
    let (mut ui, id) = grid_of(widget, Size::new(300.0, 200.0));
    let before = seen(&mut ui, id).headers;
    let sideways = PointerKind::Scroll(ScrollDelta::Lines(2.0, 0.0));
    pointer(&mut ui, sideways, Point::new(150.0, 100.0));
    let after = seen(&mut ui, id).headers;
    assert_eq!(after[0].1, before[0].1, "the frozen column stays");
    assert!(after[1].1.left() < before[1].1.left(), "the others scroll");
    // A click where a scrolled column now lies under the frozen one is the frozen one's.
    let under = Point::new(before[0].1.right() - 10.0, middle(before[0].1).y);
    click(&mut ui, under);
    assert_eq!(view(&ui, id).sort_order(), Some((0, SortOrder::Ascending)));
}

#[test]
fn dragging_the_scroll_bar_scrolls() {
    let (mut ui, id) = grid(1_000);
    let rect = ui.rect(id).unwrap();
    let bar = Point::new(rect.right() - 2.0, rect.bottom() - 3.0);
    pointer(&mut ui, PointerKind::Down(PointerButton::Primary), bar);
    pointer(&mut ui, PointerKind::Up(PointerButton::Primary), bar);
    assert!(
        view(&ui, id).first_row() > 900,
        "a press on the track jumps there"
    );
    let top = Point::new(rect.right() - 2.0, rect.top() + 40.0);
    pointer(&mut ui, PointerKind::Down(PointerButton::Primary), top);
    pointer(&mut ui, PointerKind::Move, Point::new(top.x, rect.top()));
    pointer(&mut ui, PointerKind::Up(PointerButton::Primary), top);
    assert_eq!(view(&ui, id).first_row(), 0);
    assert_eq!(
        view(&ui, id).selected(),
        None,
        "the scroll bar does not select"
    );
}

#[test]
fn screen_readers_read_rows_and_choose_them() {
    let (mut ui, id) = grid(30);
    let seen_now = seen(&mut ui, id);
    let titles: Vec<_> = seen_now.headers.iter().map(|h| h.0.as_str()).collect();
    assert_eq!(titles, ["Symbol", "Price", "Venue", "Note"]);
    let (label, _, _, row) = &seen_now.rows[1];
    assert_eq!(label, "S0001, 3.70, XNAS, -");
    assert!(ui.access_action(*row, AccessAction::Click, None));
    assert_eq!(view(&ui, id).selected(), Some(1));
    assert!(seen(&mut ui, id).rows[1].1, "selected");
    assert_eq!(actions(&mut ui), [ActionKind::Selected(1)]);
}

#[test]
fn rows_that_go_away_take_the_selection_and_scroll_with_them() {
    let (mut ui, id) = grid(100);
    ui.update::<DataGrid<Quotes>, _>(id, |g, cx| {
        g.select(cx, Some(90));
        g.model_mut(cx).rows.truncate(5);
    });
    frame(&mut ui);
    assert_eq!(view(&ui, id).selected(), None);
    assert_eq!(view(&ui, id).first_row(), 0);
}

#[test]
fn a_secondary_press_selects_its_row_then_asks_for_a_context_menu() {
    let (mut ui, id) = grid(100);
    let rows = seen(&mut ui, id).rows;
    let at = middle(rows[4].2);
    ui.actions().count();
    pointer(
        &mut ui,
        PointerKind::Down(densa_ui::PointerButton::Secondary),
        at,
    );
    pointer(
        &mut ui,
        PointerKind::Up(densa_ui::PointerButton::Secondary),
        at,
    );
    assert_eq!(view(&ui, id).selected(), Some(4));
    let asked: Vec<_> = ui
        .actions()
        .filter(|a| a.source == id)
        .map(|a| a.kind)
        .filter(|k| matches!(k, ActionKind::ContextMenu { .. }))
        .collect();
    assert_eq!(asked, [ActionKind::ContextMenu { at }]);
}
