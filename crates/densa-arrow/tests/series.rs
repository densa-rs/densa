//! A live table's view as a chart series: numbers, times and dates read as numbers, in the
//! view's order, never from batches the series has not taken in.

use std::sync::Arc;

use arrow_array::{
    ArrayRef, Date32Array, Date64Array, Decimal128Array, Float32Array, Float64Array, Int16Array,
    Int32Array, Int64Array, Int8Array, RecordBatch, StringArray, TimestampMicrosecondArray,
    TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray, UInt16Array,
    UInt32Array, UInt64Array, UInt8Array,
};
use densa_arrow::{LiveSeries, SeriesError};
use densa_chart::Points;
use densa_data::{LiveTable, Order, ViewSpec};

/// A table keyed by `id` (1, 2, 3) with `value` holding `values`.
fn table_of(values: ArrayRef) -> LiveTable {
    let batch = RecordBatch::try_from_iter([
        ("id", Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef),
        ("value", values),
    ])
    .unwrap();
    let mut table = LiveTable::new(batch.schema(), "id").unwrap();
    table.upsert(batch).unwrap();
    table
}

/// The `value`s of `table` in id order, as a series reads them.
fn read(table: &mut LiveTable) -> Vec<f64> {
    let view = table
        .add_view(ViewSpec::new().sort_by("id", Order::Ascending))
        .unwrap();
    let series = LiveSeries::new(table, "id", "value").unwrap();
    let points = series.points(table, view).unwrap();
    (0..points.len()).map(|i| points.y(i)).collect()
}

/// Whether `got` is `want`, NaN matching NaN.
fn same(got: &[f64], want: &[f64]) -> bool {
    got.len() == want.len()
        && got
            .iter()
            .zip(want)
            .all(|(a, b)| a == b || (a.is_nan() && b.is_nan()))
}

#[test]
fn numbers_of_every_width_read_as_their_value_and_nulls_as_gaps() {
    let nan = f64::NAN;
    let cases: Vec<(ArrayRef, [f64; 3])> = vec![
        (
            Arc::new(Int8Array::from(vec![Some(-3), None, Some(7)])),
            [-3.0, nan, 7.0],
        ),
        (
            Arc::new(Int16Array::from(vec![Some(-300), None, Some(7)])),
            [-300.0, nan, 7.0],
        ),
        (
            Arc::new(Int32Array::from(vec![Some(-3), None, Some(7)])),
            [-3.0, nan, 7.0],
        ),
        (
            Arc::new(Int64Array::from(vec![Some(-3), None, Some(7)])),
            [-3.0, nan, 7.0],
        ),
        (
            Arc::new(UInt8Array::from(vec![Some(3), None, Some(255)])),
            [3.0, nan, 255.0],
        ),
        (
            Arc::new(UInt16Array::from(vec![Some(3), None, Some(7)])),
            [3.0, nan, 7.0],
        ),
        (
            Arc::new(UInt32Array::from(vec![Some(3), None, Some(7)])),
            [3.0, nan, 7.0],
        ),
        (
            Arc::new(UInt64Array::from(vec![Some(3), None, Some(7)])),
            [3.0, nan, 7.0],
        ),
        (
            Arc::new(Float32Array::from(vec![Some(1.5), None, Some(-2.25)])),
            [1.5, nan, -2.25],
        ),
        (
            Arc::new(Float64Array::from(vec![
                Some(1.5),
                None,
                Some(f64::INFINITY),
            ])),
            [1.5, nan, f64::INFINITY],
        ),
    ];
    for (values, want) in cases {
        let data_type = values.data_type().clone();
        let got = read(&mut table_of(values));
        assert!(same(&got, &want), "{data_type}: {got:?}");
    }
}

#[test]
fn decimals_read_at_their_scale() {
    let decimals = Decimal128Array::from(vec![Some(150), None, Some(-25)])
        .with_precision_and_scale(10, 2)
        .unwrap();
    let got = read(&mut table_of(Arc::new(decimals)));
    assert!(same(&got, &[1.5, f64::NAN, -0.25]), "{got:?}");
}

#[test]
fn times_read_as_seconds_since_the_epoch_and_dates_as_their_midnight() {
    let cases: Vec<(ArrayRef, [f64; 3])> = vec![
        (
            Arc::new(TimestampSecondArray::from(vec![Some(90), None, Some(-1)])),
            [90.0, f64::NAN, -1.0],
        ),
        (
            Arc::new(
                TimestampMillisecondArray::from(vec![Some(1_500), None, Some(-250)])
                    .with_timezone("UTC"),
            ),
            [1.5, f64::NAN, -0.25],
        ),
        (
            Arc::new(TimestampMicrosecondArray::from(vec![
                Some(2_500_000),
                None,
                Some(0),
            ])),
            [2.5, f64::NAN, 0.0],
        ),
        (
            Arc::new(TimestampNanosecondArray::from(vec![
                Some(3_000_000_000),
                None,
                Some(1),
            ])),
            [3.0, f64::NAN, 1e-9],
        ),
        (
            Arc::new(Date32Array::from(vec![Some(2), None, Some(-1)])),
            [172_800.0, f64::NAN, -86_400.0],
        ),
        (
            Arc::new(Date64Array::from(vec![Some(86_400_000), None, Some(500)])),
            [86_400.0, f64::NAN, 0.5],
        ),
    ];
    for (values, want) in cases {
        let data_type = values.data_type().clone();
        let got = read(&mut table_of(values));
        assert!(same(&got, &want), "{data_type}: {got:?}");
    }
}

#[test]
fn columns_must_exist_and_hold_numbers() {
    let table = table_of(Arc::new(StringArray::from(vec!["a", "b", "c"])));
    let missing = LiveSeries::new(&table, "id", "price").unwrap_err();
    assert_eq!(missing, SeriesError::NoSuchColumn("price".to_owned()));
    assert_eq!(missing.to_string(), "no column named \"price\"");
    let text = LiveSeries::new(&table, "value", "id").unwrap_err();
    assert_eq!(text, SeriesError::NotNumeric("value".to_owned()));
    assert_eq!(text.to_string(), "column \"value\" is not numeric");
}

/// A table of quotes: `id`, `time` (seconds) and `price`, one row per `(id, time, price)`.
fn quotes(rows: &[(i64, f64, f64)]) -> RecordBatch {
    RecordBatch::try_from_iter([
        (
            "id",
            Arc::new(Int64Array::from_iter_values(rows.iter().map(|r| r.0))) as ArrayRef,
        ),
        (
            "time",
            Arc::new(Float64Array::from_iter_values(rows.iter().map(|r| r.1))) as ArrayRef,
        ),
        (
            "price",
            Arc::new(Float64Array::from_iter_values(rows.iter().map(|r| r.2))) as ArrayRef,
        ),
    ])
    .unwrap()
}

fn live(rows: &[(i64, f64, f64)]) -> LiveTable {
    let batch = quotes(rows);
    let mut table = LiveTable::new(batch.schema(), "id").unwrap();
    table.upsert(batch).unwrap();
    table
}

#[test]
fn points_come_in_the_views_order() {
    let mut table = live(&[(1, 30.0, 3.0), (2, 10.0, 1.0), (3, 20.0, 2.0)]);
    let by_time = table
        .add_view(ViewSpec::new().sort_by("time", Order::Ascending))
        .unwrap();
    let series = LiveSeries::new(&table, "time", "price").unwrap();
    let points = series.points(&table, by_time).unwrap();
    let got: Vec<(f64, f64)> = (0..points.len())
        .map(|i| (points.x(i), points.y(i)))
        .collect();
    assert_eq!(got, [(10.0, 1.0), (20.0, 2.0), (30.0, 3.0)]);
    assert_eq!(points.x_extent(), Some((10.0, 30.0)));
    assert!(points.x(3).is_nan(), "past the end is a gap");
}

#[test]
fn rows_added_since_the_last_sync_are_not_read_until_synced() {
    let mut table = live(&[(1, 10.0, 1.0)]);
    let view = table.add_view(ViewSpec::new()).unwrap();
    let mut series = LiveSeries::new(&table, "time", "price").unwrap();
    table.upsert(quotes(&[(2, 20.0, 2.0)])).unwrap();
    assert!(series.points(&table, view).is_none());
    series.sync(&table);
    let points = series.points(&table, view).unwrap();
    assert_eq!((points.len(), points.y(1)), (2, 2.0));
}

#[test]
fn a_compaction_to_as_many_batches_is_not_read_until_synced() {
    let mut table = live(&[(1, 10.0, 1.0), (2, 20.0, 2.0)]);
    let view = table.add_view(ViewSpec::new()).unwrap();
    let mut series = LiveSeries::new(&table, "time", "price").unwrap();
    table.delete(&Int64Array::from(vec![1])).unwrap();
    series.sync(&table);
    table.compact().unwrap();
    assert_eq!(table.batches().len(), 1);
    assert!(
        series.points(&table, view).is_none(),
        "never the old batch's rows"
    );
    series.sync(&table);
    let points = series.points(&table, view).unwrap();
    assert_eq!((points.len(), points.x(0), points.y(0)), (1, 20.0, 2.0));
}

#[test]
fn another_table_has_its_columns_found_again_by_name() {
    let mut table = live(&[(1, 10.0, 1.0)]);
    let mut series = LiveSeries::new(&table, "time", "price").unwrap();

    // Another table with the columns in another order.
    let reordered = RecordBatch::try_from_iter([
        ("price", Arc::new(Float64Array::from(vec![5.0])) as ArrayRef),
        ("id", Arc::new(Int64Array::from(vec![9])) as ArrayRef),
        ("time", Arc::new(Float64Array::from(vec![50.0])) as ArrayRef),
    ])
    .unwrap();
    let mut other = LiveTable::new(reordered.schema(), "id").unwrap();
    other.upsert(reordered).unwrap();
    let view = other.add_view(ViewSpec::new()).unwrap();
    series.sync(&other);
    let points = series.points(&other, view).unwrap();
    assert_eq!((points.x(0), points.y(0)), (50.0, 5.0));

    // One without them: no points, until a table with them comes back.
    let mut bare = table_of(Arc::new(Int64Array::from(vec![1, 2, 3])));
    let bare_view = bare.add_view(ViewSpec::new()).unwrap();
    series.sync(&bare);
    assert!(series.points(&bare, bare_view).is_none());
    let view = table.add_view(ViewSpec::new()).unwrap();
    series.sync(&table);
    assert_eq!(series.points(&table, view).unwrap().y(0), 1.0);
}

#[test]
fn a_missing_view_has_no_points() {
    let mut table = live(&[(1, 10.0, 1.0)]);
    let view = table.add_view(ViewSpec::new()).unwrap();
    let series = LiveSeries::new(&table, "time", "price").unwrap();
    table.remove_view(view);
    assert!(series.points(&table, view).is_none());
}

#[test]
fn the_extent_is_found_past_nulls_in_either_order() {
    let batch = RecordBatch::try_from_iter([
        (
            "id",
            Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5])) as ArrayRef,
        ),
        (
            "time",
            Arc::new(Float64Array::from(vec![
                None,
                Some(5.0),
                Some(9.0),
                None,
                Some(7.0),
            ])) as ArrayRef,
        ),
    ])
    .unwrap();
    let mut table = LiveTable::new(batch.schema(), "id").unwrap();
    table.upsert(batch).unwrap();
    let series = LiveSeries::new(&table, "time", "time").unwrap();
    for order in [Order::Ascending, Order::Descending] {
        let view = table
            .add_view(ViewSpec::new().sort_by("time", order))
            .unwrap();
        let points = series.points(&table, view).unwrap();
        assert_eq!(points.x_extent(), Some((5.0, 9.0)), "{order:?}");
    }
    let empty = table
        .add_view(ViewSpec::new().filter(densa_data::Filter::is_null("id")))
        .unwrap();
    assert_eq!(series.points(&table, empty).unwrap().x_extent(), None);
    let nulls = table
        .add_view(ViewSpec::new().filter(densa_data::Filter::is_null("time")))
        .unwrap();
    assert_eq!(series.points(&table, nulls).unwrap().x_extent(), None);
}

#[test]
fn a_clone_of_the_table_is_read_after_a_sync() {
    let mut table = live(&[(1, 10.0, 1.0)]);
    let mut series = LiveSeries::new(&table, "time", "price").unwrap();
    let mut copy = table.clone();
    copy.upsert(quotes(&[(2, 20.0, 2.0)])).unwrap();
    let view = copy
        .add_view(ViewSpec::new().sort_by("time", Order::Ascending))
        .unwrap();
    assert!(series.points(&copy, view).is_none(), "another table object");
    series.sync(&copy);
    let points = series.points(&copy, view).unwrap();
    assert_eq!((points.len(), points.y(1)), (2, 2.0));
    let _ = table.add_view(ViewSpec::new()).unwrap();
}

#[test]
fn a_column_of_another_type_in_another_table_is_read_as_its_type() {
    let mut table = live(&[(1, 10.0, 1.5)]);
    let mut series = LiveSeries::new(&table, "time", "price").unwrap();
    assert!(series.is_resolved());
    assert_eq!((series.x_name(), series.y_name()), ("time", "price"));
    let ints = RecordBatch::try_from_iter([
        ("id", Arc::new(Int64Array::from(vec![1])) as ArrayRef),
        ("time", Arc::new(Int64Array::from(vec![30])) as ArrayRef),
        ("price", Arc::new(Int32Array::from(vec![7])) as ArrayRef),
    ])
    .unwrap();
    let mut other = LiveTable::new(ints.schema(), "id").unwrap();
    other.upsert(ints).unwrap();
    let view = other.add_view(ViewSpec::new()).unwrap();
    series.sync(&other);
    let points = series.points(&other, view).unwrap();
    assert_eq!((points.x(0), points.y(0)), (30.0, 7.0));

    let text = table_of(Arc::new(StringArray::from(vec!["a", "b", "c"])));
    series.sync(&text);
    assert!(
        !series.is_resolved(),
        "value is text here, and there is no time or price"
    );
    let view = table.add_view(ViewSpec::new()).unwrap();
    series.sync(&table);
    assert!(series.is_resolved());
    assert_eq!(series.points(&table, view).unwrap().y(0), 1.5);
}

#[test]
fn decimals_with_a_negative_scale_count_in_tens() {
    let decimals = Decimal128Array::from(vec![Some(3), None, Some(-123)])
        .with_precision_and_scale(10, -5)
        .unwrap();
    let got = read(&mut table_of(Arc::new(decimals)));
    // Exact: 123 / 1e-5 would be 12299999.999999998.
    assert!(same(&got, &[300_000.0, f64::NAN, -12_300_000.0]), "{got:?}");
}

#[test]
fn nanosecond_times_keep_their_order() {
    let base = 1_792_400_400_000_000_000_i64;
    let times = TimestampNanosecondArray::from(vec![base, base + 1, base + 1_000_000_000]);
    let got = read(&mut table_of(Arc::new(times)));
    assert!(got.windows(2).all(|w| w[0] <= w[1]), "{got:?}");
    assert!((got[2] - got[0] - 1.0).abs() < 1e-6, "{got:?}");
}
