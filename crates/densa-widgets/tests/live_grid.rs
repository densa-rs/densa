//! A live table in the grid widget: rows sorted by a click, a selection that follows its
//! record as prices move, and filtering from the app.

use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use densa_arrow::LiveModel;
use densa_core::{Point, Rect, Size};
use densa_data::{Filter, LiveTable, Scalar};
use densa_draw::{Canvas, InstanceBuffer};
use densa_grid::SortOrder;
use densa_paragraph::TextSystem;
use densa_text::{AtlasConfig, GlyphCache};
use densa_ui::{PointerButton, PointerEvent, PointerKind, Ui, WidgetId};
use densa_widgets::DataGrid;

type Grid = DataGrid<LiveModel>;

fn quotes(symbols: Vec<&str>, prices: Vec<f64>) -> RecordBatch {
    RecordBatch::try_from_iter([
        ("symbol", Arc::new(StringArray::from(symbols)) as ArrayRef),
        ("price", Arc::new(Float64Array::from(prices)) as ArrayRef),
    ])
    .unwrap()
}

/// A grid of four quotes: A 4, B 3, C 2, D 1.
fn grid() -> (Ui, WidgetId) {
    let mut table = LiveTable::new(quotes(vec![], vec![]).schema(), "symbol").unwrap();
    table
        .upsert(quotes(vec!["A", "B", "C", "D"], vec![4.0, 3.0, 2.0, 1.0]))
        .unwrap();
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(400.0, 300.0));
    let model = LiveModel::new(table).unwrap();
    let id = ui.add(ui.root(), Grid::new(model), Grid::style()).unwrap();
    ui.layout();
    paint(&mut ui);
    (ui, id)
}

fn paint(ui: &mut Ui) {
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(65_536);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    ui.paint(&mut canvas);
}

fn grid_of(ui: &Ui, id: WidgetId) -> &Grid {
    ui.widget::<Grid>(id).unwrap()
}

/// The symbol in the selected row.
fn selected_symbol(ui: &Ui, id: WidgetId) -> Option<String> {
    let grid = grid_of(ui, id);
    let record = grid.model().record(grid.selected()?)?;
    let at = grid.model().table().row_of(record)?;
    let batch = &grid.model().table().batches()[at.batch];
    let symbols = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    Some(symbols.value(at.row).to_owned())
}

/// Where the title of column `column` is.
fn title(ui: &mut Ui, id: WidgetId, column: usize) -> Rect {
    let tree = ui.access_tree();
    let table = tree.node(ui.access_id(id).unwrap()).unwrap();
    let header = tree.node(table.children()[0]).unwrap();
    tree.node(header.children()[column]).unwrap().bounds()
}

/// Clicks the title of column `column`.
fn click_title(ui: &mut Ui, id: WidgetId, column: usize) {
    let title = title(ui, id, column);
    let at = Point::new(
        (title.left() + title.right()) / 2.0,
        (title.top() + title.bottom()) / 2.0,
    );
    for kind in [
        PointerKind::Down(PointerButton::Primary),
        PointerKind::Up(PointerButton::Primary),
    ] {
        ui.pointer(PointerEvent::new(kind, at));
    }
}

fn upsert(ui: &mut Ui, id: WidgetId, symbols: Vec<&str>, prices: Vec<f64>) {
    ui.update::<Grid, _>(id, |grid, cx| {
        grid.model_mut(cx)
            .table_mut()
            .upsert(quotes(symbols, prices))
            .unwrap();
    });
    paint(ui);
}

#[test]
fn a_click_on_a_title_sorts_the_live_rows() {
    let (mut ui, id) = grid();
    click_title(&mut ui, id, 1);
    assert_eq!(
        grid_of(&ui, id).sort_order(),
        Some((1, SortOrder::Ascending))
    );
    ui.update::<Grid, _>(id, |grid, cx| grid.select(cx, Some(0)));
    assert_eq!(
        selected_symbol(&ui, id).as_deref(),
        Some("D"),
        "cheapest first"
    );
}

#[test]
fn the_selection_follows_its_record_as_prices_move() {
    let (mut ui, id) = grid();
    click_title(&mut ui, id, 1);
    ui.update::<Grid, _>(id, |grid, cx| grid.select(cx, Some(1)));
    assert_eq!(selected_symbol(&ui, id).as_deref(), Some("C"));

    upsert(&mut ui, id, vec!["C"], vec![9.0]);
    assert_eq!(grid_of(&ui, id).selected(), Some(3), "C is now the dearest");
    assert_eq!(selected_symbol(&ui, id).as_deref(), Some("C"));

    click_title(&mut ui, id, 1);
    assert_eq!(
        grid_of(&ui, id).selected(),
        Some(0),
        "and first when reversed"
    );
}

#[test]
fn a_deleted_record_loses_the_selection() {
    let (mut ui, id) = grid();
    ui.update::<Grid, _>(id, |grid, cx| grid.select(cx, Some(2)));
    ui.update::<Grid, _>(id, |grid, cx| {
        grid.model_mut(cx)
            .table_mut()
            .delete(&StringArray::from(vec!["C"]))
            .unwrap();
    });
    paint(&mut ui);
    assert_eq!(grid_of(&ui, id).selected(), None);
}

#[test]
fn a_filter_from_the_app_keeps_the_selection_if_its_row_stays() {
    let (mut ui, id) = grid();
    ui.update::<Grid, _>(id, |grid, cx| grid.select(cx, Some(3)));
    ui.update::<Grid, _>(id, |grid, cx| {
        let filter = Filter::compare("price", densa_data::CompareOp::Lt, Scalar::Float(2.5));
        grid.model_mut(cx).set_filter(Some(filter)).unwrap();
    });
    paint(&mut ui);
    assert_eq!(grid_of(&ui, id).model().view().unwrap().len(), 2);
    assert_eq!(selected_symbol(&ui, id).as_deref(), Some("D"));
}

#[test]
fn a_grid_started_on_an_empty_table_takes_the_widths_of_the_first_rows() {
    let table = LiveTable::new(quotes(vec![], vec![]).schema(), "symbol").unwrap();
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(400.0, 300.0));
    let model = LiveModel::new(table).unwrap();
    let id = ui.add(ui.root(), Grid::new(model), Grid::style()).unwrap();
    ui.layout();
    paint(&mut ui);
    upsert(&mut ui, id, vec!["A-VERY-LONG-SYMBOL"], vec![1.0]);
    assert_eq!(grid_of(&ui, id).columns()[0].chars, 18);
}

#[test]
fn a_number_too_wide_widens_its_column_and_a_dragged_width_stays() {
    let (mut ui, id) = grid();
    let symbol = title(&mut ui, id, 0);
    let edge = Point::new(symbol.right(), (symbol.top() + symbol.bottom()) / 2.0);
    let wider = Point::new(edge.x + 60.0, edge.y);
    for (kind, at) in [
        (PointerKind::Down(PointerButton::Primary), edge),
        (PointerKind::Move, wider),
        (PointerKind::Up(PointerButton::Primary), wider),
    ] {
        ui.pointer(PointerEvent::new(kind, at));
    }
    ui.layout();
    let dragged = grid_of(&ui, id).columns()[0].chars;
    assert!(dragged > 6, "{dragged}");
    assert_eq!(grid_of(&ui, id).columns()[1].chars, 5, "\"price\" and 4");

    upsert(&mut ui, id, vec!["E"], vec![123_456_789.5]);
    let columns = grid_of(&ui, id).columns();
    assert_eq!(columns[1].chars, 11, "the price column widened");
    assert_eq!(
        columns[0].chars, dragged,
        "the width the user dragged stays"
    );
}

#[test]
fn widths_dragged_on_other_columns_are_forgotten() {
    let (mut ui, id) = grid();
    let symbol = title(&mut ui, id, 0);
    let edge = Point::new(symbol.right(), (symbol.top() + symbol.bottom()) / 2.0);
    let wider = Point::new(edge.x + 60.0, edge.y);
    for (kind, at) in [
        (PointerKind::Down(PointerButton::Primary), edge),
        (PointerKind::Move, wider),
        (PointerKind::Up(PointerButton::Primary), wider),
    ] {
        ui.pointer(PointerEvent::new(kind, at));
    }
    // Another table, of other columns: then its first column grows.
    let counts = |n: Vec<i64>| {
        RecordBatch::try_from_iter([("n", Arc::new(Int64Array::from(n)) as ArrayRef)]).unwrap()
    };
    ui.update::<Grid, _>(id, |grid, cx| {
        let mut other = LiveTable::new(counts(vec![]).schema(), "n").unwrap();
        other.upsert(counts(vec![1])).unwrap();
        *grid.model_mut(cx).table_mut() = other;
    });
    paint(&mut ui);
    ui.update::<Grid, _>(id, |grid, cx| {
        let table = grid.model_mut(cx).table_mut();
        table.upsert(counts(vec![123_456_789])).unwrap();
    });
    paint(&mut ui);
    assert_eq!(
        grid_of(&ui, id).columns()[0].chars,
        9,
        "the new column's own width"
    );
}

#[test]
fn a_column_widening_mid_drag_does_not_end_the_drag() {
    let (mut ui, id) = grid();
    let symbol = title(&mut ui, id, 0);
    let edge = Point::new(symbol.right(), (symbol.top() + symbol.bottom()) / 2.0);
    ui.pointer(PointerEvent::new(
        PointerKind::Down(PointerButton::Primary),
        edge,
    ));
    upsert(&mut ui, id, vec!["E"], vec![123_456_789.5]);
    assert_eq!(grid_of(&ui, id).columns()[1].chars, 11);
    let wider = Point::new(edge.x + 60.0, edge.y);
    ui.pointer(PointerEvent::new(PointerKind::Move, wider));
    ui.pointer(PointerEvent::new(
        PointerKind::Up(PointerButton::Primary),
        wider,
    ));
    ui.layout();
    assert!(grid_of(&ui, id).columns()[0].chars > 6, "the drag went on");
}

#[test]
fn another_table_clears_the_selection_instead_of_moving_it_to_a_stranger() {
    let (mut ui, id) = grid();
    ui.update::<Grid, _>(id, |grid, cx| grid.select(cx, Some(1)));
    ui.update::<Grid, _>(id, |grid, cx| {
        let mut other = LiveTable::new(quotes(vec![], vec![]).schema(), "symbol").unwrap();
        other
            .upsert(quotes(vec!["W", "X", "Y"], vec![1.0, 2.0, 3.0]))
            .unwrap();
        *grid.model_mut(cx).table_mut() = other;
    });
    ui.layout();
    paint(&mut ui);
    assert_eq!(grid_of(&ui, id).selected(), None);
    assert_eq!(grid_of(&ui, id).model().view().unwrap().len(), 3);
}

#[test]
fn a_row_the_filter_hides_stays_unselected_when_it_shows_again() {
    let (mut ui, id) = grid();
    ui.update::<Grid, _>(id, |grid, cx| grid.select(cx, Some(0)));
    let filter = |price: f64| {
        Some(Filter::compare(
            "price",
            densa_data::CompareOp::Lt,
            Scalar::Float(price),
        ))
    };
    ui.update::<Grid, _>(id, |grid, cx| {
        grid.model_mut(cx).set_filter(filter(2.5)).unwrap();
    });
    paint(&mut ui);
    assert_eq!(grid_of(&ui, id).selected(), None, "A is hidden");
    ui.update::<Grid, _>(id, |grid, cx| grid.model_mut(cx).set_filter(None).unwrap());
    paint(&mut ui);
    assert_eq!(grid_of(&ui, id).selected(), None, "as in Qt's proxy models");
}
