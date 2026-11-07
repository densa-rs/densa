//! A window on a pair join's left rows: the rows it lets go of leave with every pair they are
//! in, and the join holds only the rows within it.

use std::collections::BTreeMap;

use arrow_array::Int64Array;
use densa_data::Window;

use crate::support::*;

type Left = BTreeMap<i64, (Option<String>, i64)>;

/// Lets go of the rows of `left` more than `span` below the largest amount, as a window on
/// `amount` does after an upsert.
fn expire(left: &mut Left, span: i64) {
    let Some(end) = left.values().map(|(_, amount)| *amount).max() else {
        return;
    };
    left.retain(|_, (_, amount)| end - *amount <= span);
}

#[test]
fn a_left_window_keeps_the_pairs_of_the_recent_left_rows_only() {
    const VALUES: [Option<&str>; 4] = [Some("a"), Some("b"), Some("c"), None];
    const SPAN: i64 = 20;
    for seed in 1..=12_u64 {
        let (mut join, mut out) = join();
        let window = Window::span("amount", SPAN as f64);
        join.set_left_window(&mut out, Some(window)).unwrap();
        let mut left = Left::new();
        let mut right = BTreeMap::new();
        let mut random = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        for step in 0..300_i64 {
            match random.below(6) {
                0..=2 => {
                    // Amounts drift up with the steps, some behind the newest by more than
                    // the span.
                    let count = 1 + random.below(4);
                    let rows: Vec<(i64, Option<&str>, i64)> = (0..count)
                        .map(|_| {
                            let key = random.below(40) as i64;
                            let value = VALUES[random.below(4) as usize];
                            (key, value, step / 2 + random.below(30) as i64 - 15)
                        })
                        .collect();
                    join.upsert_left(&mut out, lefts(&rows)).unwrap();
                    for &(key, value, amount) in &rows {
                        left.insert(key, (value.map(str::to_owned), amount));
                    }
                    expire(&mut left, SPAN);
                }
                3 => {
                    let keys: Vec<i64> = (0..2).map(|_| random.below(40) as i64).collect();
                    join.delete_left(&mut out, &Int64Array::from(keys.clone()))
                        .unwrap();
                    for key in keys {
                        left.remove(&key);
                    }
                }
                4 => {
                    let count = 1 + random.below(3);
                    let rows: Vec<(i64, Option<&str>, f64)> = (0..count)
                        .map(|_| {
                            let key = random.below(10) as i64;
                            (key, VALUES[random.below(4) as usize], step as f64)
                        })
                        .collect();
                    join.upsert_right(&mut out, rights(&rows)).unwrap();
                    for &(key, value, distance) in &rows {
                        right.insert(key, (value.map(str::to_owned), distance));
                    }
                }
                _ => {
                    let key = random.below(10) as i64;
                    join.delete_right(&mut out, &Int64Array::from(vec![key]))
                        .unwrap();
                    right.remove(&key);
                }
            }
            assert_eq!(
                rows(&out),
                expected(&left, &right),
                "seed {seed} step {step}"
            );
            assert_eq!(join.left().len(), left.len(), "seed {seed} step {step}");
        }
    }
}

#[test]
fn a_window_set_on_a_join_with_rows_lets_go_of_those_past_it_and_their_pairs() {
    let (mut join, mut out) = join();
    let orders = [
        (1, Some("ann"), 0),
        (2, Some("ann"), 5),
        (3, Some("bob"), 100),
    ];
    join.upsert_left(&mut out, lefts(&orders)).unwrap();
    join.upsert_right(
        &mut out,
        rights(&[(10, Some("ann"), 1.0), (11, Some("ann"), 2.0)]),
    )
    .unwrap();
    assert_eq!(out.len(), 5);
    let changes = join
        .set_left_window(&mut out, Some(Window::span("amount", 10.0)))
        .unwrap();
    assert_eq!(changes.deleted, 4, "two orders, two pairs each");
    assert_eq!(rows(&out), [(3, Some("bob".to_owned()), 100, None, None)]);
    assert_eq!(join.left().len(), 1);
    // No window: old orders are kept again.
    join.set_left_window(&mut out, None).unwrap();
    join.upsert_left(&mut out, lefts(&[(4, Some("ann"), 0)]))
        .unwrap();
    assert_eq!(out.len(), 3);
}

#[test]
fn a_row_moved_past_the_window_leaves_its_old_pairs() {
    let (mut join, mut out) = join();
    join.set_left_window(&mut out, Some(Window::span("amount", 10.0)))
        .unwrap();
    join.upsert_right(&mut out, rights(&[(10, Some("ann"), 1.0)]))
        .unwrap();
    join.upsert_left(&mut out, lefts(&[(1, Some("ann"), 100)]))
        .unwrap();
    assert_eq!(out.len(), 1);
    // Order 1 moves to another customer and an old amount, in the batch that brings the
    // newest: gone at once, with the pair of its old customer.
    let batch = lefts(&[(1, Some("cat"), 50), (2, Some("bob"), 100)]);
    join.upsert_left(&mut out, batch).unwrap();
    assert_eq!(rows(&out), [(2, Some("bob".to_owned()), 100, None, None)]);
    assert_eq!(join.left().len(), 1);
}

#[test]
fn a_left_window_the_rows_cannot_take_changes_nothing() {
    let (mut join, mut out) = join();
    join.upsert_left(&mut out, lefts(&[(1, Some("ann"), 0)]))
        .unwrap();
    let unknown = join.set_left_window(&mut out, Some(Window::span("when", 10.0)));
    assert!(unknown.is_err());
    let mut other = densa_data::LiveTable::new(left_schema(), "order").unwrap();
    let wrong = join.set_left_window(&mut other, Some(Window::span("amount", 10.0)));
    assert!(matches!(wrong, Err(densa_data::DataError::PairTable)));
    assert!(join.left().window().is_none());
    assert_eq!(out.len(), 1);
}

#[test]
fn rows_let_go_of_by_the_thousand_compact_the_left_rows_and_still_pair_right() {
    const SPAN: i64 = 100;
    let (mut join, mut out) = join();
    join.set_left_window(&mut out, Some(Window::span("amount", SPAN as f64)))
        .unwrap();
    let mut right = BTreeMap::new();
    let addresses = [(10, Some("a"), 1.0), (11, Some("b"), 2.0)];
    join.upsert_right(&mut out, rights(&addresses)).unwrap();
    for (key, value, distance) in addresses {
        right.insert(key, (value.map(str::to_owned), distance));
    }
    let mut left = Left::new();
    for round in 0..20_i64 {
        let rows: Vec<(i64, Option<&str>, i64)> = (0..1_000)
            .map(|i| {
                let at = round * 1_000 + i;
                (at, Some(["a", "b"][(i % 2) as usize]), at)
            })
            .collect();
        join.upsert_left(&mut out, lefts(&rows)).unwrap();
        for (key, value, amount) in rows {
            left.insert(key, (value.map(str::to_owned), amount));
        }
        expire(&mut left, SPAN);
    }
    assert!(join.left().generation() > 0, "the left rows were compacted");
    assert_eq!(rows(&out), expected(&left, &right));
    // The addresses change after the compaction: the rows still held pair with them.
    let moved = [(10, Some("b"), 3.0)];
    join.upsert_right(&mut out, rights(&moved)).unwrap();
    right.insert(10, (Some("b".to_owned()), 3.0));
    assert_eq!(rows(&out), expected(&left, &right));
    assert_eq!(join.left().len(), left.len());
}
