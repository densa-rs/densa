//! A live table as a grid widget's model: rows, cells, sorting, filtering and records.

use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use densa_arrow::LiveModel;
use densa_data::{CompareOp, Filter, LiveTable, Scalar};
use densa_grid::{CellText, GridModel, Palette, SortOrder};

fn quotes(symbols: Vec<&str>, prices: Vec<f64>) -> RecordBatch {
    RecordBatch::try_from_iter([
        ("symbol", Arc::new(StringArray::from(symbols)) as ArrayRef),
        ("price", Arc::new(Float64Array::from(prices)) as ArrayRef),
    ])
    .unwrap()
}

fn model(symbols: Vec<&str>, prices: Vec<f64>) -> LiveModel {
    let mut table = LiveTable::new(quotes(vec![], vec![]).schema(), "symbol").unwrap();
    table.upsert(quotes(symbols, prices)).unwrap();
    LiveModel::new(table).unwrap()
}

/// Every row as the grid would paint it: cells joined by spaces.
fn shown(model: &impl GridModel) -> Vec<String> {
    let palette = Palette::new();
    (0..model.rows())
        .map(|row| {
            (0..model.columns().len())
                .map(|column| {
                    let mut text = CellText::new();
                    model.cell(row, column, &mut text, &palette);
                    text.as_str().to_owned()
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

#[test]
fn it_shows_every_row_in_row_id_order_with_the_tables_columns() {
    let mut model = model(vec!["B", "A", "C"], vec![3.0, 1.5, 2.25]);
    model.prepare();
    let titles: Vec<&str> = model.columns().iter().map(|c| &*c.title).collect();
    assert_eq!(titles, ["symbol", "price"]);
    assert_eq!(shown(&model), ["B 3", "A 1.5", "C 2.25"]);
    assert_eq!(model.sort_state(), None);
}

#[test]
fn sorting_orders_the_rows_and_reports_the_sort() {
    let mut model = model(vec!["B", "A", "C"], vec![3.0, 1.5, 2.25]);
    assert!(model.sort(1, SortOrder::Descending));
    model.prepare();
    assert_eq!(shown(&model), ["B 3", "C 2.25", "A 1.5"]);
    assert_eq!(model.sort_state(), Some((1, SortOrder::Descending)));
    assert!(model.sort(0, SortOrder::Ascending));
    assert_eq!(shown(&model), ["A 1.5", "B 3", "C 2.25"]);
}

#[test]
fn sorting_by_a_column_that_does_not_exist_changes_nothing() {
    let mut model = model(vec!["B", "A"], vec![3.0, 1.5]);
    assert!(model.sort(1, SortOrder::Ascending));
    assert!(!model.sort(2, SortOrder::Descending));
    model.prepare();
    assert_eq!(model.sort_state(), Some((1, SortOrder::Ascending)));
    assert_eq!(shown(&model), ["A 1.5", "B 3"]);
}

#[test]
fn a_row_key_follows_its_record_as_rows_move_and_ends_when_deleted() {
    let mut model = model(vec!["A", "B", "C"], vec![1.0, 2.0, 3.0]);
    assert!(model.sort(1, SortOrder::Ascending));
    let b = model.row_key(1).unwrap();
    model
        .table_mut()
        .upsert(quotes(vec!["B"], vec![9.0]))
        .unwrap();
    assert_eq!(model.row_of_key(b), Some(2), "B moved to the end");
    assert_eq!(model.row_key(2), Some(b));
    model
        .table_mut()
        .delete(&StringArray::from(vec!["B"]))
        .unwrap();
    assert_eq!(model.row_of_key(b), None);
    model
        .table_mut()
        .upsert(quotes(vec!["B"], vec![2.0]))
        .unwrap();
    assert_eq!(model.row_of_key(b), None, "a new B is another record");
    assert_eq!(model.row_key(3), None);
}

#[test]
fn a_filter_keeps_the_sort_and_a_bad_one_changes_nothing() {
    let mut model = model(vec!["A", "B", "C"], vec![1.0, 2.0, 3.0]);
    assert!(model.sort(1, SortOrder::Descending));
    let cheap = Filter::compare("price", CompareOp::Lt, Scalar::Float(2.5));
    model.set_filter(Some(cheap.clone())).unwrap();
    model.prepare();
    assert_eq!(shown(&model), ["B 2", "A 1"]);
    assert_eq!(model.filter(), Some(&cheap));

    let unknown = Filter::compare("volume", CompareOp::Lt, Scalar::Float(1.0));
    assert!(model.set_filter(Some(unknown)).is_err());
    assert_eq!(shown(&model), ["B 2", "A 1"]);
    model.set_filter(None).unwrap();
    assert_eq!(model.rows(), 3);
    assert_eq!(model.sort_state(), Some((1, SortOrder::Descending)));
}

#[test]
fn cells_stay_empty_until_prepared_after_the_table_grew() {
    let mut model = model(vec!["A"], vec![1.0]);
    model.prepare();
    model
        .table_mut()
        .upsert(quotes(vec!["B"], vec![2.0]))
        .unwrap();
    assert_eq!(model.rows(), 2);
    assert_eq!(
        shown(&model),
        [" ", " "],
        "never values from the wrong batch"
    );
    model.prepare();
    assert_eq!(shown(&model), ["A 1", "B 2"]);
}

#[test]
fn compaction_is_followed_after_prepare() {
    let mut model = model(vec!["A", "B"], vec![1.0, 2.0]);
    assert!(model.sort(1, SortOrder::Ascending));
    for price in 3..10 {
        model
            .table_mut()
            .upsert(quotes(vec!["A"], vec![f64::from(price)]))
            .unwrap();
    }
    model.prepare();
    model.table_mut().compact().unwrap();
    model.prepare();
    assert_eq!(shown(&model), ["B 2", "A 9"]);
}

#[test]
fn cells_stay_empty_until_prepared_after_a_compaction_to_as_many_batches() {
    let mut model = model(vec!["A", "B"], vec![1.0, 2.0]);
    model
        .table_mut()
        .delete(&StringArray::from(vec!["A"]))
        .unwrap();
    model.prepare();
    // One batch before and after: B moves from its second row to the first.
    model.table_mut().compact().unwrap();
    assert_eq!(model.table().batches().len(), 1);
    assert_eq!(shown(&model), [" "], "never the old batch's first row");
    model.prepare();
    assert_eq!(shown(&model), ["B 2"]);
}

#[test]
fn a_replaced_table_gets_the_view_back_on_prepare() {
    let mut model = model(vec!["A", "B"], vec![2.0, 1.0]);
    assert!(model.sort(1, SortOrder::Ascending));
    let mut other = LiveTable::new(quotes(vec![], vec![]).schema(), "symbol").unwrap();
    other
        .upsert(quotes(vec!["X", "Y"], vec![5.0, 4.0]))
        .unwrap();
    *model.table_mut() = other;
    assert_eq!(model.rows(), 0, "the old view is gone");
    model.prepare();
    assert_eq!(shown(&model), ["Y 4", "X 5"]);
}

#[test]
fn records_and_rows_of_the_view_are_exposed() {
    let mut model = model(vec!["A", "B"], vec![2.0, 1.0]);
    assert!(model.sort(1, SortOrder::Ascending));
    let record = model.record(0).unwrap();
    let at = model.table().row_of(record).unwrap();
    assert_eq!(model.table().find(&Scalar::Str("B".into())), Some(at));
    assert_eq!(model.view().unwrap().len(), 2);
}

#[test]
fn the_first_rows_size_the_columns_of_a_table_that_started_empty() {
    let table = LiveTable::new(quotes(vec![], vec![]).schema(), "symbol").unwrap();
    let mut model = LiveModel::new(table).unwrap();
    let before = (model.columns_revision(), model.columns()[0].chars);
    model
        .table_mut()
        .upsert(quotes(vec!["A-VERY-LONG-SYMBOL"], vec![1.0]))
        .unwrap();
    model.prepare();
    assert_ne!(model.columns_revision(), before.0, "the grid is told");
    assert_eq!(model.columns()[0].chars, 18, "was {}", before.1);
    let revision = model.columns_revision();
    model
        .table_mut()
        .upsert(quotes(vec!["B"], vec![2.0]))
        .unwrap();
    model.prepare();
    assert_eq!(model.columns_revision(), revision, "sized once");
}

#[test]
fn a_later_number_too_wide_widens_its_column_and_never_narrows_it() {
    let mut model = model(vec!["A"], vec![1.0]);
    assert_eq!(model.columns()[1].chars, 5, "\"price\"");
    let revision = model.columns_revision();
    model
        .table_mut()
        .upsert(quotes(vec!["A-LONGER-SYMBOL"], vec![123_456_789.5]))
        .unwrap();
    model.prepare();
    assert_ne!(model.columns_revision(), revision, "the grid is told");
    assert_eq!(model.columns()[1].chars, 11);
    assert_eq!(model.columns()[0].chars, 6, "text is cut, not widened");

    let revision = model.columns_revision();
    model
        .table_mut()
        .upsert(quotes(vec!["B"], vec![2.0]))
        .unwrap();
    model.prepare();
    assert_eq!(
        model.columns_revision(),
        revision,
        "nothing wider: nothing to tell"
    );
    assert_eq!(model.columns()[1].chars, 11, "never narrower");

    model.set_column_chars(1, 4).unwrap();
    model
        .table_mut()
        .upsert(quotes(vec!["C"], vec![1e15]))
        .unwrap();
    model.prepare();
    assert_eq!(model.columns()[1].chars, 4, "a width the app set stays");
}

#[test]
fn changing_decimals_or_widths_tells_the_grid() {
    let mut model = model(vec!["A"], vec![1.0 / 3.0]);
    let revision = model.columns_revision();
    model.set_decimals(1, Some(2)).unwrap();
    assert_ne!(model.columns_revision(), revision);
    assert_eq!(model.columns()[1].chars, 5, "\"price\" is longer than 0.33");
    let revision = model.columns_revision();
    model.set_column_chars(0, 12).unwrap();
    assert_ne!(model.columns_revision(), revision);
    assert_eq!(model.columns()[0].chars, 12);
    assert!(model.set_decimals(5, None).is_err());
}

#[test]
fn another_table_starts_a_new_generation_and_drops_a_sort_it_cannot_do() {
    let mut model = model(vec!["A", "B"], vec![2.0, 1.0]);
    assert!(model.sort(1, SortOrder::Ascending));
    let (generation, revision) = (model.generation(), model.columns_revision());
    let names = Arc::new(StringArray::from(vec!["x", "y"])) as ArrayRef;
    let only_names = RecordBatch::try_from_iter([("name", names)]).unwrap();
    let mut other = LiveTable::new(only_names.schema(), "name").unwrap();
    other.upsert(only_names).unwrap();
    *model.table_mut() = other;
    model.prepare();
    assert_ne!(model.generation(), generation);
    assert_ne!(model.columns_revision(), revision);
    assert_eq!(model.sort_state(), None, "there is no column 1 now");
    assert_eq!(shown(&model), ["x", "y"]);
}

#[test]
fn a_column_named_like_an_earlier_one_does_not_sort() {
    let prices = |v: Vec<f64>| Arc::new(Float64Array::from(v)) as ArrayRef;
    let batch = RecordBatch::try_from_iter([
        (
            "symbol",
            Arc::new(StringArray::from(vec!["A", "B"])) as ArrayRef,
        ),
        ("price", prices(vec![2.0, 1.0])),
        ("price", prices(vec![1.0, 2.0])),
    ])
    .unwrap();
    let mut table = LiveTable::new(batch.schema(), "symbol").unwrap();
    table.upsert(batch).unwrap();
    let mut model = LiveModel::new(table).unwrap();
    assert!(
        !model.sort(2, SortOrder::Ascending),
        "it would sort column 1"
    );
    assert!(model.sort(1, SortOrder::Ascending));
}
