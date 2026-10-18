//! Live views in the grid: order, updates, compaction.

use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use densa_arrow::LiveGrid;
use densa_data::{LiveTable, Order, ViewSpec};
use densa_grid::{CellText, GridSource, Palette};

fn quotes(symbols: Vec<&str>, prices: Vec<f64>) -> RecordBatch {
    RecordBatch::try_from_iter([
        ("symbol", Arc::new(StringArray::from(symbols)) as ArrayRef),
        ("price", Arc::new(Float64Array::from(prices)) as ArrayRef),
    ])
    .unwrap()
}

/// Every cell of the view's visible rows, row by row.
fn shown(source: &impl GridSource) -> Vec<String> {
    (0..source.rows())
        .map(|row| {
            (0..2)
                .map(|column| {
                    let mut text = CellText::new();
                    source.cell(row, column, &mut text);
                    text.as_str().to_owned()
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

#[test]
fn a_view_shows_its_rows_in_order_and_follows_updates() {
    let mut table = LiveTable::new(quotes(vec![], vec![]).schema(), "symbol").unwrap();
    let by_price = table
        .add_view(ViewSpec::new().sort_by("price", Order::Descending))
        .unwrap();
    table
        .upsert(quotes(vec!["A", "B", "C"], vec![1.5, 3.0, 2.25]))
        .unwrap();
    let mut grid = LiveGrid::new(&table);
    let palette = Palette::new();

    let view = grid.view(&table, by_price, 0, &palette).unwrap();
    assert_eq!(shown(&view), ["B 3", "C 2.25", "A 1.5"]);

    table.upsert(quotes(vec!["A"], vec![9.0])).unwrap();
    let view = grid.view(&table, by_price, 1, &palette).unwrap();
    assert_eq!(
        shown(&view),
        ["B 3", "C 2.25"],
        "scrolled one row, A now first"
    );
}

#[test]
fn compaction_rewrites_the_formatters_too() {
    let mut table = LiveTable::new(quotes(vec![], vec![]).schema(), "symbol").unwrap();
    let all = table
        .add_view(ViewSpec::new().sort_by("symbol", Order::Ascending))
        .unwrap();
    let mut grid = LiveGrid::new(&table);
    let palette = Palette::new();
    for round in 0..20 {
        table
            .upsert(quotes(vec!["X", "Y"], vec![f64::from(round); 2]))
            .unwrap();
        grid.view(&table, all, 0, &palette).unwrap();
    }
    table.compact().unwrap();
    let view = grid.view(&table, all, 0, &palette).unwrap();
    assert_eq!(shown(&view), ["X 19", "Y 19"]);
}

#[test]
fn a_missing_view_is_none() {
    let mut table = LiveTable::new(quotes(vec![], vec![]).schema(), "symbol").unwrap();
    let id = table.add_view(ViewSpec::new()).unwrap();
    table.remove_view(id);
    let mut grid = LiveGrid::new(&table);
    assert!(grid.view(&table, id, 0, &Palette::new()).is_none());
}

#[test]
fn integer_keys_and_rows_beyond_the_view_are_empty_cells() {
    let batch =
        RecordBatch::try_from_iter([("id", Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef)])
            .unwrap();
    let mut table = LiveTable::new(batch.schema(), "id").unwrap();
    let all = table.add_view(ViewSpec::new()).unwrap();
    table.upsert(batch).unwrap();
    let mut grid = LiveGrid::new(&table);
    let view = grid.view(&table, all, 0, &Palette::new()).unwrap();
    let mut text = CellText::new();
    view.cell(5, 0, &mut text);
    assert!(text.is_empty());
}

#[test]
fn a_grid_given_another_table_shows_that_table() {
    let mut big = LiveTable::new(quotes(vec![], vec![]).schema(), "symbol").unwrap();
    let big_view = big
        .add_view(ViewSpec::new().sort_by("symbol", Order::Ascending))
        .unwrap();
    for round in 0..5 {
        big.upsert(quotes(vec!["A"], vec![f64::from(round)]))
            .unwrap();
    }
    let mut grid = LiveGrid::new(&big);
    let palette = Palette::new();
    grid.view(&big, big_view, 0, &palette).unwrap();

    // A new table, at the same generation (0) but with fewer batches than the grid mirrors:
    // matching by generation alone would slice past its batches.
    let mut small = LiveTable::new(quotes(vec![], vec![]).schema(), "symbol").unwrap();
    let small_view = small.add_view(ViewSpec::new()).unwrap();
    small.upsert(quotes(vec!["B"], vec![7.0])).unwrap();
    assert_eq!(small.generation(), big.generation());
    let view = grid.view(&small, small_view, 0, &palette).unwrap();
    assert_eq!(shown(&view), ["B 7"]);

    // And a clone of the first table, which shares its views.
    let clone = big.clone();
    let view = grid.view(&clone, big_view, 0, &palette).unwrap();
    assert_eq!(shown(&view), ["A 4"]);
}
