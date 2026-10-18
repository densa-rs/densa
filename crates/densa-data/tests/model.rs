//! Model-based test: thousands of random upserts, deletes and compactions, with every view
//! checked against a plain recomputation from a model of the table after each step.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, Float64Array, Int32Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use densa_data::{CompareOp, Filter, LiveTable, Order, RecordId, RowRef, Scalar, ViewId, ViewSpec};

const STEPS: usize = 3_000;
const KEYS: u64 = 300;
/// Names share their first 11 bytes, more than a sort prefix holds, so ordering by name
/// must fall back to comparing whole strings.
const NAMES: [&str; 6] = [
    "instrument-alpha",
    "instrument-beta",
    "instrument-gamma",
    "instrument-delta",
    "instrument-omega",
    "instrument-zeta",
];

/// One row of the model: price, name, quantity.
type Row = (Option<f64>, String, Option<i32>);

/// Deterministic xorshift, so a failure reproduces.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("price", DataType::Float64, true),
        Field::new("name", DataType::Utf8, false),
        Field::new("qty", DataType::Int32, true),
    ]))
}

fn random_row(rng: &mut Rng) -> Row {
    // Some negative zeros: they must sort and filter exactly like zero.
    let price = (rng.below(8) != 0).then(|| match rng.below(15) {
        0 => -0.0,
        _ => rng.below(1_000) as f64 / 4.0,
    });
    let name = NAMES[rng.below(NAMES.len() as u64) as usize].to_owned();
    let qty = (rng.below(6) != 0).then(|| rng.below(100) as i32);
    (price, name, qty)
}

fn batch(rows: &[(i64, Row)]) -> RecordBatch {
    let columns: Vec<ArrayRef> = vec![
        Arc::new(Int64Array::from(
            rows.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        )),
        Arc::new(Float64Array::from(
            rows.iter().map(|(_, r)| r.0).collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(
            rows.iter().map(|(_, r)| r.1.as_str()).collect::<Vec<_>>(),
        )),
        Arc::new(Int32Array::from(
            rows.iter().map(|(_, r)| r.2).collect::<Vec<_>>(),
        )),
    ];
    RecordBatch::try_new(schema(), columns).unwrap()
}

/// The model's row for a view row, read back from the table's batches.
fn read(table: &LiveTable, at: RowRef) -> (i64, Row) {
    let batch = &table.batches()[at.batch];
    let column = |i: usize| batch.column(i).as_ref();
    let ids = column(0).as_any().downcast_ref::<Int64Array>().unwrap();
    let prices = column(1).as_any().downcast_ref::<Float64Array>().unwrap();
    let names = column(2).as_any().downcast_ref::<StringArray>().unwrap();
    let qtys = column(3).as_any().downcast_ref::<Int32Array>().unwrap();
    let price = (!prices.is_null(at.row)).then(|| prices.value(at.row));
    let qty = (!qtys.is_null(at.row)).then(|| qtys.value(at.row));
    let name = names.value(at.row).to_owned();
    (ids.value(at.row), (price, name, qty))
}

/// A view under test: its spec's filter and ordering, written as plain Rust for the model.
struct Checked {
    id: ViewId,
    name: &'static str,
    keep: fn(&Row) -> bool,
    order: fn(&Row, &Row) -> Ordering,
}

fn nulls_last<T: PartialOrd>(a: &Option<T>, b: &Option<T>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn add_views(table: &mut LiveTable) -> Vec<Checked> {
    let by_price = table
        .add_view(ViewSpec::new().sort_by("price", Order::Ascending))
        .unwrap();
    let by_name_then_qty = table
        .add_view(
            ViewSpec::new()
                .sort_by("name", Order::Descending)
                .sort_by("qty", Order::Ascending)
                .filter(Filter::compare("qty", CompareOp::Gt, Scalar::Int(10))),
        )
        .unwrap();
    let unsorted = table
        .add_view(ViewSpec::new().filter(Filter::any([
            Filter::contains("name", "ta"),
            Filter::is_null("price"),
        ])))
        .unwrap();
    let not_small = table
        .add_view(ViewSpec::new().filter(Filter::not(Filter::compare(
            "qty",
            CompareOp::Lt,
            Scalar::Int(50),
        ))))
        .unwrap();
    let mid_range = table
        .add_view(
            ViewSpec::new()
                .sort_by("qty", Order::Descending)
                .filter(Filter::all([
                    Filter::compare("qty", CompareOp::Ge, Scalar::Int(20)),
                    Filter::compare("price", CompareOp::Lt, Scalar::Float(200.0)),
                ])),
        )
        .unwrap();
    vec![
        Checked {
            id: mid_range,
            name: "qty >= 20 and price < 200 (nulls excluded), qty descending",
            keep: |r| r.2.is_some_and(|q| q >= 20) && r.0.is_some_and(|p| p < 200.0),
            order: |a, b| nulls_last(&b.2, &a.2),
        },
        Checked {
            id: by_price,
            name: "by price, nulls last",
            keep: |_| true,
            order: |a, b| nulls_last(&a.0, &b.0),
        },
        Checked {
            id: by_name_then_qty,
            name: "qty > 10 by name descending, then qty",
            keep: |r| r.2.is_some_and(|q| q > 10),
            order: |a, b| b.1.cmp(&a.1).then_with(|| nulls_last(&a.2, &b.2)),
        },
        Checked {
            id: unsorted,
            name: "name contains 'ta' or price is null",
            keep: |r| r.1.contains("ta") || r.0.is_none(),
            order: |_, _| Ordering::Equal,
        },
        Checked {
            id: not_small,
            name: "not qty < 50 (null qty excluded)",
            keep: |r| r.2.is_some_and(|q| q >= 50),
            order: |_, _| Ordering::Equal,
        },
    ]
}

/// Checks every view against the model, and record ids: each live key keeps the record it
/// got, every record is found where the view shows it, and a deleted key's record is gone.
fn check(
    table: &LiveTable,
    model: &BTreeMap<i64, Row>,
    views: &[Checked],
    records: &mut BTreeMap<i64, RecordId>,
    step: usize,
) {
    assert_eq!(table.len(), model.len(), "step {step}: live rows");
    for (key, record) in records.iter() {
        let now = table.record_of_key(&Scalar::Int(*key));
        if model.contains_key(key) {
            assert_eq!(now, Some(*record), "step {step}: key {key} changed record");
        } else {
            assert_eq!(now, None, "step {step}: deleted key {key} has a record");
        }
    }
    records.retain(|key, _| model.contains_key(key));
    for key in model.keys() {
        let record = table.record_of_key(&Scalar::Int(*key));
        assert!(
            record.is_some(),
            "step {step}: live key {key} has no record"
        );
        records.entry(*key).or_insert(record.unwrap());
    }
    for view in views {
        let rows = table.view(view.id).unwrap();
        for i in 0..rows.len() {
            let record = rows.record(i).unwrap();
            assert_eq!(
                rows.position(record),
                Some(i),
                "step {step}, {}: position",
                view.name
            );
            assert_eq!(
                table.row_of(record),
                rows.row(i),
                "step {step}, {}: row",
                view.name
            );
        }
        assert_eq!(rows.record(rows.len()), None);
        let shown: Vec<(i64, Row)> = (0..rows.len())
            .map(|i| read(table, rows.row(i).unwrap()))
            .collect();
        let mut ids: Vec<i64> = shown.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        let expected: Vec<i64> = model
            .iter()
            .filter(|(_, row)| (view.keep)(row))
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(ids, expected, "step {step}, {}: rows", view.name);
        for (id, row) in &shown {
            assert_eq!(
                Some(row),
                model.get(id),
                "step {step}, {}: stale row",
                view.name
            );
        }
        for pair in shown.windows(2) {
            assert_ne!(
                (view.order)(&pair[0].1, &pair[1].1),
                Ordering::Greater,
                "step {step}, {}: out of order {pair:?}",
                view.name
            );
        }
    }
}

#[test]
fn views_match_a_full_recomputation_after_every_change() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut table = LiveTable::new(schema(), "id").unwrap();
    let views = add_views(&mut table);
    let mut model: BTreeMap<i64, Row> = BTreeMap::new();
    let mut records = BTreeMap::new();
    let mut gone: Vec<RecordId> = Vec::new();

    for step in 0..STEPS {
        match rng.below(20) {
            0..=12 => {
                // Small upserts, keys may repeat within a batch: the last one wins.
                let len = 1 + rng.below(12) as usize;
                let rows: Vec<(i64, Row)> = (0..len)
                    .map(|_| (rng.below(KEYS) as i64, random_row(&mut rng)))
                    .collect();
                table.upsert(batch(&rows)).unwrap();
                model.extend(rows);
            }
            13 => {
                // A large upsert: touches most rows, so views rebuild instead of patching.
                let rows: Vec<(i64, Row)> = (0..KEYS as i64)
                    .map(|id| (id, random_row(&mut rng)))
                    .collect();
                table.upsert(batch(&rows)).unwrap();
                model.extend(rows);
            }
            14..=17 => {
                let keys: Vec<i64> = (0..1 + rng.below(4))
                    .map(|_| rng.below(KEYS + 20) as i64)
                    .collect();
                let deleted = table
                    .delete(&Int64Array::from(keys.clone()))
                    .unwrap()
                    .deleted;
                let mut expected = 0;
                for key in &keys {
                    expected += usize::from(model.remove(key).is_some());
                    gone.extend(records.get(key));
                }
                // The latest few are enough: storage is reused soon after a delete.
                let excess = gone.len().saturating_sub(64);
                gone.drain(..excess);
                assert_eq!(deleted, expected, "step {step}: deleted count");
            }
            _ => table.compact().unwrap(),
        }
        check(&table, &model, &views, &mut records, step);
        // A deleted key's record is never found again, even once its storage is reused.
        for record in &gone {
            assert_eq!(table.row_of(*record), None, "step {step}: deleted record");
            for view in &views {
                assert_eq!(table.view(view.id).unwrap().position(*record), None);
            }
        }
    }
}
