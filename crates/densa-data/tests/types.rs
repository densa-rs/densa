//! Every column type views can sort and filter, every key layout, and error messages.

use std::sync::Arc;

use arrow_array::types::UInt8Type;
use arrow_array::{
    Array, ArrayRef, BooleanArray, Date64Array, Decimal128Array, DictionaryArray,
    DurationNanosecondArray, Float32Array, Int64Array, LargeStringArray, RecordBatch,
    StringViewArray, Time32SecondArray, UInt32Array, UInt64Array, UInt8Array,
};
use densa_data::{CompareOp, DataError, Filter, LiveTable, Order, Scalar, ViewSpec};

/// Five rows of every supported type, keyed by `id` 10..=14.
fn table() -> LiveTable {
    let regions: DictionaryArray<UInt8Type> = vec!["west", "east", "north", "south", "east"]
        .into_iter()
        .collect();
    let amounts = Decimal128Array::from(vec![3, 1, 4, 1, 5])
        .with_precision_and_scale(10, -2)
        .unwrap();
    let batch = RecordBatch::try_from_iter([
        (
            "id",
            Arc::new(UInt32Array::from(vec![10, 11, 12, 13, 14])) as ArrayRef,
        ),
        (
            "flag",
            Arc::new(BooleanArray::from(vec![true, false, true, false, true])) as ArrayRef,
        ),
        (
            "count",
            Arc::new(UInt64Array::from(vec![5, 3, 9, 1, 7])) as ArrayRef,
        ),
        (
            "ratio",
            Arc::new(Float32Array::from(vec![0.5, -1.5, 2.5, 0.0, f32::NAN])) as ArrayRef,
        ),
        (
            "clock",
            Arc::new(Time32SecondArray::from(vec![30, 10, 50, 20, 40])) as ArrayRef,
        ),
        (
            "day",
            Arc::new(Date64Array::from(vec![300, 100, 500, 200, 400])) as ArrayRef,
        ),
        (
            "wait",
            Arc::new(DurationNanosecondArray::from(vec![3, 1, 5, 2, 4])) as ArrayRef,
        ),
        (
            "large",
            Arc::new(LargeStringArray::from(vec!["c", "a", "e", "b", "d"])) as ArrayRef,
        ),
        (
            "view",
            Arc::new(StringViewArray::from(vec!["cc", "aa", "ee", "bb", "dd"])) as ArrayRef,
        ),
        ("region", Arc::new(regions) as ArrayRef),
        ("amount", Arc::new(amounts) as ArrayRef),
    ])
    .unwrap();
    let mut table = LiveTable::new(batch.schema(), "id").unwrap();
    table.upsert(batch).unwrap();
    table
}

/// The ids of a new view of `table`, in its order.
fn ids(table: &mut LiveTable, spec: ViewSpec) -> Vec<u32> {
    let view = table.add_view(spec).unwrap();
    let rows = table.view(view).unwrap();
    (0..rows.len())
        .map(|i| {
            let at = rows.row(i).unwrap();
            let column = table.batches()[at.batch].column(0);
            column
                .as_any()
                .downcast_ref::<UInt32Array>()
                .unwrap()
                .value(at.row)
        })
        .collect()
}

fn compare(column: &str, op: CompareOp, value: Scalar) -> ViewSpec {
    ViewSpec::new().filter(Filter::compare(column, op, value))
}

#[test]
fn every_supported_type_sorts() {
    let mut t = table();
    let sort = |column: &str| ViewSpec::new().sort_by(column, Order::Ascending);
    assert_eq!(ids(&mut t, sort("flag")), [11, 13, 10, 12, 14]);
    assert_eq!(ids(&mut t, sort("count")), [13, 11, 10, 14, 12]);
    // NaN sorts after every number (IEEE total order).
    assert_eq!(ids(&mut t, sort("ratio")), [11, 13, 10, 12, 14]);
    for column in ["clock", "day", "wait", "large", "view"] {
        assert_eq!(ids(&mut t, sort(column)), [11, 13, 10, 14, 12], "{column}");
    }
    // Ties (east twice; 1 twice) fall back to row order.
    assert_eq!(ids(&mut t, sort("region")), [11, 14, 12, 13, 10]);
    assert_eq!(ids(&mut t, sort("amount")), [11, 13, 10, 12, 14]);
    assert_eq!(
        ids(&mut t, ViewSpec::new().sort_by("count", Order::Descending)),
        [12, 14, 10, 11, 13]
    );
}

#[test]
fn literals_compare_across_numeric_kinds() {
    let mut t = table();
    assert_eq!(
        ids(&mut t, compare("count", CompareOp::Gt, Scalar::Int(-1))).len(),
        5
    );
    assert_eq!(
        ids(&mut t, compare("count", CompareOp::Ge, Scalar::Float(6.5))),
        [12, 14]
    );
    assert_eq!(
        ids(&mut t, compare("count", CompareOp::Eq, Scalar::UInt(3))),
        [11]
    );
    // NaN compares with nothing: neither above nor below.
    assert_eq!(
        ids(&mut t, compare("ratio", CompareOp::Lt, Scalar::Int(1))),
        [10, 11, 13]
    );
    assert_eq!(
        ids(&mut t, compare("ratio", CompareOp::Ne, Scalar::Float(0.5))),
        [11, 12, 13]
    );
    // Scale -2: the stored 4 means 400.
    assert_eq!(
        ids(&mut t, compare("amount", CompareOp::Eq, Scalar::Int(400))),
        [12]
    );
    assert_eq!(
        ids(
            &mut t,
            compare("amount", CompareOp::Le, Scalar::Float(150.0))
        ),
        [11, 13]
    );
    assert_eq!(
        ids(&mut t, compare("flag", CompareOp::Eq, Scalar::Bool(false))),
        [11, 13]
    );
    assert_eq!(
        ids(&mut t, compare("clock", CompareOp::Ge, Scalar::Int(40))),
        [12, 14]
    );
}

#[test]
fn text_filters_work_on_every_string_layout() {
    let mut t = table();
    assert_eq!(
        ids(
            &mut t,
            compare("view", CompareOp::Le, Scalar::Str("bb".into()))
        ),
        [11, 13]
    );
    let contains =
        |column: &str, text: &str| ViewSpec::new().filter(Filter::contains(column, text));
    assert_eq!(ids(&mut t, contains("large", "e")), [12]);
    assert_eq!(ids(&mut t, contains("region", "st")), [10, 11, 14]);
    let missing = ViewSpec::new().filter(Filter::all([
        Filter::is_not_null("large"),
        Filter::any(Vec::<Filter>::new()),
    ]));
    assert!(ids(&mut t, missing).is_empty(), "an empty any() is false");
}

#[test]
fn unsigned_keys_are_found_and_deleted_by_any_unsigned_layout() {
    let mut t = table();
    assert!(t.find(&Scalar::Int(12)).is_some());
    assert!(t.find(&Scalar::UInt(12)).is_some());
    assert_eq!(t.find(&Scalar::Int(-1)), None);
    assert_eq!(t.find(&Scalar::Str("12".into())), None);
    assert_eq!(
        t.delete(&UInt8Array::from(vec![12, 99])).unwrap().deleted,
        1
    );
    assert_eq!(
        t.delete(&Int64Array::from(vec![10])),
        Err(DataError::KeyType)
    );
    assert_eq!(t.len(), 4);
}

#[test]
fn string_view_and_large_string_keys() {
    for keys in [
        Arc::new(StringViewArray::from(vec!["a", "b"])) as ArrayRef,
        Arc::new(LargeStringArray::from(vec!["a", "b"])) as ArrayRef,
    ] {
        let batch = RecordBatch::try_from_iter([("key", keys)]).unwrap();
        let mut t = LiveTable::new(batch.schema(), "key").unwrap();
        t.upsert(batch).unwrap();
        assert!(t.find(&Scalar::Str("b".into())).is_some());
        assert_eq!(
            t.delete(&LargeStringArray::from(vec!["a"]))
                .unwrap()
                .deleted,
            1
        );
        assert_eq!(t.len(), 1);
    }
}

#[test]
fn errors_say_what_is_wrong() {
    let cases = [
        (DataError::UnknownColumn("x".into()), "x"),
        (DataError::UnsupportedKey("price".into()), "price"),
        (DataError::UnsupportedColumn("tags".into()), "tags"),
        (DataError::LiteralMismatch("qty".into()), "qty"),
        (DataError::NotNumeric("region".into()), "region"),
        (DataError::DuplicateName("count".into()), "count"),
        (DataError::WindowSpan("time".into()), "window"),
        (DataError::BucketWidth("time".into()), "bucket"),
        (
            DataError::ColumnCount {
                expected: 2,
                found: 3,
            },
            "3 columns",
        ),
        (DataError::ColumnMismatch { column: 4 }, "column 4"),
        (DataError::NullKey { row: 7 }, "row 7"),
        (DataError::KeyType, "key"),
        (DataError::TooManyRows, "2^32"),
        (DataError::Arrow("overflow".into()), "overflow"),
    ];
    for (error, needle) in cases {
        let message = error.to_string();
        assert!(message.contains(needle), "{message:?} lacks {needle:?}");
    }
}

#[test]
fn a_dictionary_entry_pointing_at_a_null_value_reads_as_null() {
    let values = arrow_array::StringArray::from(vec![Some("x"), None]);
    let keys = UInt8Array::from(vec![0, 1]);
    let region = DictionaryArray::<UInt8Type>::try_new(keys, Arc::new(values)).unwrap();
    let batch = RecordBatch::try_from_iter([
        ("id", Arc::new(UInt32Array::from(vec![1, 2])) as ArrayRef),
        ("region", Arc::new(region) as ArrayRef),
    ])
    .unwrap();
    let mut t = LiveTable::new(batch.schema(), "id").unwrap();
    t.upsert(batch).unwrap();
    let view = t
        .add_view(ViewSpec::new().filter(Filter::is_null("region")))
        .unwrap();
    assert_eq!(t.view(view).unwrap().len(), 1);
}
