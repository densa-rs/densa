//! Pair joins and their output table: failures mended, windows and deletes on it, the changes
//! counted, tables refused.

use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::{Int64Type, UInt64Type};
use arrow_array::{Array, ArrayRef, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use densa_data::{DataError, LiveTable, PairJoin, Scalar, Window};

use crate::support::*;

#[test]
fn a_failure_to_build_the_output_is_mended_by_the_next_call() {
    use arrow_array::types::Int8Type;
    use arrow_array::DictionaryArray;
    let tags = DataType::Dictionary(Box::new(DataType::Int8), Box::new(DataType::Utf8));
    let left = Arc::new(Schema::new(vec![
        Field::new("order", DataType::Int64, false),
        Field::new("customer", DataType::Utf8, true),
        Field::new("tag", tags, true),
    ]));
    let mut join = PairJoin::new(
        Arc::clone(&left),
        "order",
        "customer",
        right_schema(),
        "address",
        "owner",
    )
    .unwrap();
    let mut out = join.table().unwrap();
    // 200 orders of one customer, each in a batch of its own with a tag of its own: an Int8
    // dictionary holds at most 128 of their tags at once.
    for order in 0..200_i64 {
        let name = format!("tag {order}");
        let tag: DictionaryArray<Int8Type> = [name.as_str()].into_iter().collect();
        let columns: Vec<ArrayRef> = vec![
            Arc::new(Int64Array::from(vec![order])),
            Arc::new(StringArray::from(vec!["ann"])),
            Arc::new(tag),
        ];
        let rows = RecordBatch::try_new(Arc::clone(&left), columns).unwrap();
        join.upsert_left(&mut out, rows).unwrap();
    }
    assert_eq!(out.len(), 200);
    // An address of the customer: 200 pairs to build at once, more tags than the dictionary
    // holds.
    let failed = join.upsert_right(&mut out, rights(&[(10, Some("ann"), 1.0)]));
    assert!(matches!(failed, Err(DataError::Arrow(_))), "{failed:?}");
    assert_eq!(join.right().len(), 1, "the join took the row");
    // Fewer orders: the next call fills the output whole, to exactly the pairs.
    join.delete_left(&mut out, &Int64Array::from_iter_values(0..150))
        .unwrap();
    assert_eq!(out.len(), 50);
    let alone: usize = out
        .snapshot()
        .unwrap()
        .iter()
        .map(|batch| batch.column(3).null_count())
        .sum();
    assert_eq!(alone, 0, "every order paired with the address");
}

#[test]
fn a_window_on_the_output_drops_pairs_that_come_back_when_their_rows_change() {
    let (mut join, mut out) = join();
    out.set_window(Some(Window::span("amount", 10.0))).unwrap();
    join.upsert_left(
        &mut out,
        lefts(&[(1, Some("ann"), 0), (2, Some("ann"), 100)]),
    )
    .unwrap();
    join.upsert_right(&mut out, rights(&[(10, Some("ann"), 1.0)]))
        .unwrap();
    // Order 1 is far below the newest amount: outside the window.
    let orders: Vec<i64> = rows(&out).iter().map(|row| row.0).collect();
    assert_eq!(orders, [2]);
    // The address changes: both pairs are written again, and the window drops order 1 again.
    join.upsert_right(&mut out, rights(&[(10, Some("ann"), 2.0)]))
        .unwrap();
    let kept = (
        2,
        Some("ann".to_owned()),
        100,
        Some(10),
        Some(2.0_f64.to_bits()),
    );
    assert_eq!(rows(&out), [kept]);
}

#[test]
fn a_pair_deleted_from_the_output_directly_comes_back_when_its_row_changes() {
    let (mut join, mut out) = join();
    join.upsert_left(&mut out, lefts(&[(1, Some("ann"), 10)]))
        .unwrap();
    join.upsert_right(&mut out, rights(&[(10, Some("ann"), 1.0)]))
        .unwrap();
    let pair = out.snapshot().unwrap()[0]
        .column(5)
        .as_primitive::<UInt64Type>()
        .value(0);
    out.delete(&arrow_array::UInt64Array::from(vec![pair]))
        .unwrap();
    assert!(rows(&out).is_empty());
    join.upsert_left(&mut out, lefts(&[(1, Some("ann"), 11)]))
        .unwrap();
    assert_eq!(rows(&out).len(), 1);
}

#[test]
fn a_row_the_left_side_refuses_leaves_a_new_output_table_empty() {
    let (mut join, mut out) = join();
    join.upsert_left(&mut out, lefts(&[(1, Some("ann"), 10)]))
        .unwrap();
    let mut fresh = join.table().unwrap();
    let nullable = Arc::new(Schema::new(vec![
        Field::new("order", DataType::Int64, true),
        Field::new("customer", DataType::Utf8, true),
        Field::new("amount", DataType::Int64, true),
    ]));
    let rows = RecordBatch::try_new(
        nullable,
        vec![
            Arc::new(Int64Array::from(vec![None])),
            Arc::new(StringArray::from(vec![Some("bob")])),
            Arc::new(Int64Array::from(vec![Some(1)])),
        ],
    )
    .unwrap();
    let refused = join.upsert_left(&mut fresh, rows);
    assert!(
        matches!(refused, Err(DataError::NullKey { .. })),
        "{refused:?}"
    );
    assert!(fresh.is_empty(), "nothing changed");
    assert_eq!(join.left().len(), 1);
}

#[test]
fn changes_count_pairs_and_a_pair_rewritten_is_updated_in_place() {
    let counts =
        |changes: densa_data::Changes| (changes.inserted, changes.updated, changes.deleted);
    let (mut join, mut out) = join();
    let both = lefts(&[(1, Some("ann"), 10), (2, Some("bob"), 20)]);
    assert_eq!(counts(join.upsert_left(&mut out, both).unwrap()), (2, 0, 0));
    // Ann's row with nulls makes way for her pair.
    let address = rights(&[(10, Some("ann"), 1.0)]);
    assert_eq!(
        counts(join.upsert_right(&mut out, address).unwrap()),
        (1, 0, 1)
    );
    // Either row of the pair changing rewrites it, keeping its row in the output.
    let record = |out: &LiveTable| {
        let batches = out.snapshot().unwrap();
        let pair = batches.iter().find_map(|batch| {
            let orders = batch.column(0).as_primitive::<Int64Type>();
            let pairs = batch.column(5).as_primitive::<UInt64Type>();
            (0..batch.num_rows())
                .find(|&row| orders.value(row) == 1)
                .map(|row| pairs.value(row))
        })?;
        out.record_of_key(&Scalar::UInt(pair))
    };
    let before = record(&out);
    assert!(before.is_some());
    let moved = rights(&[(10, Some("ann"), 2.0)]);
    assert_eq!(
        counts(join.upsert_right(&mut out, moved).unwrap()),
        (0, 1, 0)
    );
    let bigger = lefts(&[(1, Some("ann"), 11)]);
    assert_eq!(
        counts(join.upsert_left(&mut out, bigger).unwrap()),
        (0, 1, 0)
    );
    assert_eq!(record(&out), before);
    // The address goes: Ann's row with nulls comes back.
    let gone = Int64Array::from(vec![10]);
    assert_eq!(
        counts(join.delete_right(&mut out, &gone).unwrap()),
        (1, 0, 1)
    );
    let bob = Int64Array::from(vec![2]);
    assert_eq!(counts(join.delete_left(&mut out, &bob).unwrap()), (0, 0, 1));
}

#[test]
fn a_table_unlike_the_output_in_one_way_is_refused() {
    let (mut join, _) = join();
    let fields: Vec<Field> = join
        .schema()
        .fields()
        .iter()
        .map(|field| field.as_ref().clone())
        .collect();
    let pair = join.pair_column().to_owned();
    let table =
        |fields: Vec<Field>, key: &str| LiveTable::new(Arc::new(Schema::new(fields)), key).unwrap();
    let mut renamed = fields.clone();
    renamed[2] = Field::new("total", DataType::Int64, true);
    let mut retyped = fields.clone();
    retyped[2] = Field::new("amount", DataType::Int32, true);
    let mut longer = fields.clone();
    longer.push(Field::new("extra", DataType::Int64, true));
    let unlike = [
        ("keyed by another column", table(fields.clone(), "order")),
        ("a column renamed", table(renamed, &pair)),
        ("a column of another type", table(retyped, &pair)),
        ("a column more", table(longer, &pair)),
    ];
    for (how, mut out) in unlike {
        let refused = join.upsert_left(&mut out, lefts(&[(1, Some("ann"), 10)]));
        assert!(
            matches!(refused, Err(DataError::PairTable)),
            "{how}: {refused:?}"
        );
        assert!(out.is_empty(), "{how}");
    }
    assert!(join.left().is_empty(), "nothing was taken");
    let mut same = table(fields, &pair);
    join.upsert_left(&mut same, lefts(&[(1, Some("ann"), 10)]))
        .unwrap();
    assert_eq!(same.len(), 1);
}
