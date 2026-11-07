//! Pair joins: the output always equals a left outer join recomputed from both sides' rows,
//! whatever upserts and deletes come on either side; what cannot be joined is refused.

use std::collections::BTreeMap;
use std::sync::Arc;

use arrow_array::{Array, Int32Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use densa_data::{DataError, Filter, LiveTable, Order, PairJoin, ViewSpec};

mod outputs;
mod support;

use support::*;

#[test]
fn rows_pair_with_every_row_of_their_value_and_alone_with_nulls() {
    let (mut join, mut out) = join();
    assert_eq!(join.pair_column(), "pair");
    join.upsert_left(
        &mut out,
        lefts(&[(1, Some("ann"), 10), (2, Some("bob"), 20)]),
    )
    .unwrap();
    let addresses = rights(&[(10, Some("ann"), 1.0), (11, Some("ann"), 2.0)]);
    join.upsert_right(&mut out, addresses).unwrap();
    let ann = |address: i64, distance: f64| {
        (
            1,
            Some("ann".to_owned()),
            10,
            Some(address),
            Some(distance.to_bits()),
        )
    };
    let bob = (2, Some("bob".to_owned()), 20, None, None);
    assert_eq!(rows(&out), [ann(10, 1.0), ann(11, 2.0), bob.clone()]);
    // An address of Bob's: his row alone gives way to a pair.
    join.upsert_right(&mut out, rights(&[(12, Some("bob"), 3.0)]))
        .unwrap();
    let bob_paired = (
        2,
        Some("bob".to_owned()),
        20,
        Some(12),
        Some(3.0_f64.to_bits()),
    );
    assert_eq!(rows(&out), [ann(10, 1.0), ann(11, 2.0), bob_paired]);
    // His address goes: alone again.
    join.delete_right(&mut out, &Int64Array::from(vec![12]))
        .unwrap();
    assert_eq!(rows(&out), [ann(10, 1.0), ann(11, 2.0), bob]);
    // An order changes customer: its pairs follow.
    join.upsert_left(&mut out, lefts(&[(2, Some("ann"), 20)]))
        .unwrap();
    let both = |order: i64, amount: i64, address: i64, distance: f64| {
        (
            order,
            Some("ann".to_owned()),
            amount,
            Some(address),
            Some(distance.to_bits()),
        )
    };
    assert_eq!(
        rows(&out),
        [
            both(1, 10, 10, 1.0),
            both(1, 10, 11, 2.0),
            both(2, 20, 10, 1.0),
            both(2, 20, 11, 2.0)
        ]
    );
    // An order goes, with its pairs.
    join.delete_left(&mut out, &Int64Array::from(vec![1]))
        .unwrap();
    assert_eq!(rows(&out), [both(2, 20, 10, 1.0), both(2, 20, 11, 2.0)]);
}

#[test]
fn the_output_matches_a_join_recomputed_after_every_change() {
    const VALUES: [Option<&str>; 6] = [Some("a"), Some("b"), Some("c"), Some("d"), Some("e"), None];
    for seed in 1..=16_u64 {
        let mut random = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let (mut join, mut out) = join();
        let mut spare = join.table().unwrap();
        let mut left: BTreeMap<i64, (Option<String>, i64)> = BTreeMap::new();
        let mut right: BTreeMap<i64, (Option<String>, f64)> = BTreeMap::new();
        for step in 0..300 {
            match random.below(9) {
                0..=2 => {
                    let rows: Vec<(i64, Option<&str>, i64)> = (0..1 + random.below(6))
                        .map(|_| {
                            let key = random.below(30) as i64;
                            let value = VALUES[random.below(6) as usize];
                            (key, value, random.below(1_000) as i64)
                        })
                        .collect();
                    join.upsert_left(&mut out, lefts(&rows)).unwrap();
                    for (key, value, amount) in rows {
                        left.insert(key, (value.map(str::to_owned), amount));
                    }
                }
                3..=5 => {
                    let rows: Vec<(i64, Option<&str>, f64)> = (0..1 + random.below(6))
                        .map(|_| {
                            let key = random.below(20) as i64;
                            let value = VALUES[random.below(6) as usize];
                            (key, value, random.below(1_000) as f64 / 10.0)
                        })
                        .collect();
                    join.upsert_right(&mut out, rights(&rows)).unwrap();
                    for (key, value, distance) in rows {
                        right.insert(key, (value.map(str::to_owned), distance));
                    }
                }
                6 => {
                    let keys: Vec<i64> = (0..1 + random.below(4))
                        .map(|_| random.below(35) as i64)
                        .collect();
                    join.delete_left(&mut out, &Int64Array::from(keys.clone()))
                        .unwrap();
                    for key in keys {
                        left.remove(&key);
                    }
                }
                7 => {
                    let keys: Vec<i64> = (0..1 + random.below(4))
                        .map(|_| random.below(25) as i64)
                        .collect();
                    join.delete_right(&mut out, &Int64Array::from(keys.clone()))
                        .unwrap();
                    for key in keys {
                        right.remove(&key);
                    }
                }
                _ => {
                    // Another output table: a new one, or the one used before this, filled
                    // with every pair (and rid of pairs that ended) at the next call.
                    match random.below(3) {
                        0 => out = join.table().unwrap(),
                        1 => std::mem::swap(&mut out, &mut spare),
                        _ => {}
                    }
                    join.delete_left(&mut out, &Int64Array::from(Vec::<i64>::new()))
                        .unwrap();
                }
            }
            assert_eq!(
                rows(&out),
                expected(&left, &right),
                "seed {seed}, step {step}"
            );
            assert_eq!(join.left().len(), left.len());
            assert_eq!(join.right().len(), right.len());
        }
    }
}

#[test]
fn a_right_key_joined_on_and_integer_columns_of_two_widths_join() {
    // Trades name an instrument by id (Int32); instruments are keyed by that id (Int64).
    let left = Arc::new(Schema::new(vec![
        Field::new("trade", DataType::Int64, false),
        Field::new("instrument", DataType::Int32, true),
    ]));
    let right = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let mut join = PairJoin::new(
        left.clone(),
        "trade",
        "instrument",
        right.clone(),
        "id",
        "id",
    )
    .unwrap();
    let mut out = join.table().unwrap();
    // Without `id` (its value is in `instrument`).
    let names: Vec<&str> = join
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect();
    assert_eq!(names, ["trade", "instrument", "name", "pair"]);
    let trades = RecordBatch::try_new(
        left,
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(Int32Array::from(vec![Some(7), Some(8), None])),
        ],
    )
    .unwrap();
    join.upsert_left(&mut out, trades).unwrap();
    let instruments = RecordBatch::try_new(
        right,
        vec![
            Arc::new(Int64Array::from(vec![7])),
            Arc::new(StringArray::from(vec!["seven"])),
        ],
    )
    .unwrap();
    join.upsert_right(&mut out, instruments).unwrap();
    // Only trade 1 paired: a view keeping rows with a name holds it alone.
    let named = out
        .add_view(
            ViewSpec::new()
                .sort_by("trade", Order::Ascending)
                .filter(Filter::is_not_null("name")),
        )
        .unwrap();
    assert_eq!(out.len(), 3);
    assert_eq!(out.view(named).unwrap().len(), 1);
}

#[test]
fn what_cannot_be_joined_is_refused_with_the_reason() {
    let unknown = PairJoin::new(
        left_schema(),
        "order",
        "nope",
        right_schema(),
        "address",
        "owner",
    );
    assert!(matches!(unknown, Err(DataError::UnknownColumn(c)) if c == "nope"));
    let kinds = PairJoin::new(
        left_schema(),
        "order",
        "customer",
        right_schema(),
        "address",
        "address",
    );
    assert!(matches!(kinds, Err(DataError::PairColumns { .. })));
    let not_a_key = PairJoin::new(
        left_schema(),
        "order",
        "customer",
        right_schema(),
        "address",
        "distance",
    );
    assert!(matches!(not_a_key, Err(DataError::PairColumns { by, .. }) if by == "distance"));
    let twice = PairJoin::new(
        left_schema(),
        "order",
        "customer",
        left_schema(),
        "order",
        "customer",
    );
    assert!(matches!(twice, Err(DataError::DuplicateName(_))));
    // A column of the pair's name: the pair column takes another.
    let named = Arc::new(Schema::new(vec![
        Field::new("pair", DataType::Int64, false),
        Field::new("customer", DataType::Utf8, true),
    ]));
    let join = PairJoin::new(
        named,
        "pair",
        "customer",
        right_schema(),
        "address",
        "owner",
    )
    .unwrap();
    assert_eq!(join.pair_column(), "pair_");
    assert_eq!(join.table().unwrap().len(), 0);
}

#[test]
fn bad_rows_and_tables_change_nothing() {
    let (mut join, mut out) = join();
    join.upsert_left(&mut out, lefts(&[(1, Some("ann"), 10)]))
        .unwrap();
    let before = rows(&out);
    // Rows of the other side's columns.
    let wrong = join.upsert_left(&mut out, rights(&[(5, Some("ann"), 1.0)]));
    assert!(wrong.is_err());
    // A table that is not of this join's output.
    let mut other = LiveTable::new(left_schema(), "order").unwrap();
    let refused = join.upsert_left(&mut other, lefts(&[(2, Some("bob"), 1)]));
    assert!(matches!(refused, Err(DataError::PairTable)));
    // Keys of the wrong kind.
    let strings = StringArray::from(vec!["1"]);
    assert!(matches!(
        join.delete_left(&mut out, &strings),
        Err(DataError::KeyType)
    ));
    assert_eq!(rows(&out), before);
    assert_eq!(join.left().len(), 1);
}

#[test]
fn a_table_used_again_after_another_holds_exactly_the_pairs() {
    let (mut join, mut first) = join();
    join.upsert_left(&mut first, lefts(&[(1, Some("ann"), 10)]))
        .unwrap();
    join.upsert_right(&mut first, rights(&[(10, Some("ann"), 1.0)]))
        .unwrap();
    // Another table: filled with every pair, then changed alone.
    let mut second = join.table().unwrap();
    join.delete_right(&mut second, &Int64Array::from(vec![10]))
        .unwrap();
    let alone = (1, Some("ann".to_owned()), 10, None, None);
    assert_eq!(rows(&second), std::slice::from_ref(&alone));
    // The first again: its pair that ended goes, the row alone comes.
    join.upsert_left(&mut first, lefts(&[(2, None, 5)]))
        .unwrap();
    assert_eq!(rows(&first), [alone, (2, None, 5, None, None)]);
}

#[test]
fn null_join_values_pair_with_nothing() {
    let (mut join, mut out) = join();
    join.upsert_left(&mut out, lefts(&[(1, None, 10)])).unwrap();
    join.upsert_right(&mut out, rights(&[(10, None, 1.0)]))
        .unwrap();
    assert_eq!(rows(&out), [(1, None, 10, None, None)]);
    let column = out.snapshot().unwrap()[0].column(3).clone();
    assert_eq!(column.null_count(), 1);
}

#[test]
fn unsigned_columns_join_and_signed_with_unsigned_is_refused() {
    use arrow_array::{UInt32Array, UInt64Array};
    let left = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("code", DataType::UInt64, true),
    ]));
    let right = Arc::new(Schema::new(vec![
        Field::new("rid", DataType::Int64, false),
        Field::new("rcode", DataType::UInt32, true),
        Field::new("label", DataType::Utf8, true),
    ]));
    let mut join = PairJoin::new(
        Arc::clone(&left),
        "id",
        "code",
        Arc::clone(&right),
        "rid",
        "rcode",
    )
    .unwrap();
    let mut out = join.table().unwrap();
    let lefts = RecordBatch::try_new(
        left,
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(UInt64Array::from(vec![7, 8])),
        ],
    )
    .unwrap();
    join.upsert_left(&mut out, lefts).unwrap();
    let rights = RecordBatch::try_new(
        right,
        vec![
            Arc::new(Int64Array::from(vec![5, 6])),
            Arc::new(UInt32Array::from(vec![7, 7])),
            Arc::new(StringArray::from(vec!["x", "y"])),
        ],
    )
    .unwrap();
    join.upsert_right(&mut out, rights).unwrap();
    // Row 1 pairs with both rights, row 2 with none.
    assert_eq!(out.len(), 3);
    let signed = Arc::new(Schema::new(vec![
        Field::new("rid", DataType::Int64, false),
        Field::new("rcode", DataType::Int32, true),
    ]));
    let joined = PairJoin::new(left_schema(), "order", "order", signed, "rid", "rcode");
    assert!(joined.is_ok(), "signed with signed joins");
    let unsigned = Arc::new(Schema::new(vec![
        Field::new("rid", DataType::Int64, false),
        Field::new("rcode", DataType::UInt32, true),
    ]));
    let refused = PairJoin::new(left_schema(), "order", "order", unsigned, "rid", "rcode");
    assert!(matches!(
        refused,
        Err(DataError::PairColumns { on, by }) if on == "order" && by == "rcode"
    ));
    let message = DataError::PairColumns {
        on: "a".into(),
        by: "b".into(),
    }
    .to_string();
    assert!(message.contains("both signed integers, both unsigned integers or both text"));
}

#[test]
fn a_key_twice_in_a_batch_takes_its_last_row() {
    let (mut join, mut out) = join();
    join.upsert_right(
        &mut out,
        rights(&[(10, Some("a"), 1.0), (11, Some("b"), 2.0)]),
    )
    .unwrap();
    join.upsert_left(&mut out, lefts(&[(1, Some("a"), 5), (1, Some("b"), 6)]))
        .unwrap();
    let paired = (
        1,
        Some("b".to_owned()),
        6,
        Some(11),
        Some(2.0_f64.to_bits()),
    );
    assert_eq!(rows(&out), [paired]);
    // On the right too: address 10 moves to b, then back to a, in one batch.
    join.upsert_right(
        &mut out,
        rights(&[(10, Some("b"), 3.0), (10, Some("a"), 4.0)]),
    )
    .unwrap();
    assert_eq!(rows(&out).len(), 1);
}

#[test]
fn a_value_held_by_many_rows_on_both_sides_pairs_them_all() {
    let (mut join, mut out) = join();
    let mut left = BTreeMap::new();
    let mut right = BTreeMap::new();
    let orders: Vec<(i64, Option<&str>, i64)> = (0..60).map(|k| (k, Some("hot"), k)).collect();
    join.upsert_left(&mut out, lefts(&orders)).unwrap();
    for (key, value, amount) in &orders {
        left.insert(*key, (value.map(str::to_owned), *amount));
    }
    let addresses: Vec<(i64, Option<&str>, f64)> =
        (0..60).map(|k| (k, Some("hot"), k as f64)).collect();
    join.upsert_right(&mut out, rights(&addresses)).unwrap();
    for (key, value, distance) in &addresses {
        right.insert(*key, (value.map(str::to_owned), *distance));
    }
    assert_eq!(out.len(), 3_600);
    assert_eq!(rows(&out), expected(&left, &right));
    join.delete_right(&mut out, &Int64Array::from_iter_values(0..30))
        .unwrap();
    for key in 0..30 {
        right.remove(&key);
    }
    assert_eq!(rows(&out), expected(&left, &right));
    let cold: Vec<(i64, Option<&str>, f64)> = (30..60).map(|k| (k, Some("cold"), 0.0)).collect();
    join.upsert_right(&mut out, rights(&cold)).unwrap();
    for (key, value, distance) in &cold {
        right.insert(*key, (value.map(str::to_owned), *distance));
    }
    assert_eq!(rows(&out), expected(&left, &right));
    assert_eq!(out.len(), 60, "every order alone");
}

#[test]
fn rows_rewritten_until_the_sides_compact_still_pair_right() {
    let (mut join, mut out) = join();
    let mut left = BTreeMap::new();
    let mut right = BTreeMap::new();
    join.upsert_right(
        &mut out,
        rights(&[(10, Some("a"), 1.0), (11, Some("b"), 2.0)]),
    )
    .unwrap();
    right.insert(10, (Some("a".to_owned()), 1.0));
    right.insert(11, (Some("b".to_owned()), 2.0));
    for round in 0..600_i64 {
        let orders: Vec<(i64, Option<&str>, i64)> = (0..10)
            .map(|k| (k, Some(["a", "b"][((round + k) % 2) as usize]), round))
            .collect();
        join.upsert_left(&mut out, lefts(&orders)).unwrap();
        for (key, value, amount) in orders {
            left.insert(key, (value.map(str::to_owned), amount));
        }
    }
    assert!(join.left().generation() > 0, "the left rows were compacted");
    assert_eq!(rows(&out), expected(&left, &right));
}
