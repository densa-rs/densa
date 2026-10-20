//! Snapshots: the live rows of a table as plain record batches.

use std::collections::BTreeMap;
use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::{Float64Type, Int64Type, UInt8Type};
use arrow_array::{Array, ArrayRef, DictionaryArray, Float64Array, Int64Array, RecordBatch};
use densa_data::{LiveTable, TableOptions};

fn prices(ids: Vec<i64>, values: Vec<f64>) -> RecordBatch {
    RecordBatch::try_from_iter([
        ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
        ("price", Arc::new(Float64Array::from(values)) as ArrayRef),
    ])
    .unwrap()
}

/// A table that compacts only when asked, so tests decide what is stored.
fn table() -> LiveTable {
    let schema = prices(vec![], vec![]).schema();
    LiveTable::with_options(
        schema,
        "id",
        TableOptions::default().with_auto_compact(false),
    )
    .unwrap()
}

/// The snapshot's rows by id, checking each batch has the table's schema.
fn rows(table: &LiveTable) -> BTreeMap<i64, f64> {
    let mut out = BTreeMap::new();
    for batch in table.snapshot().unwrap() {
        assert_eq!(batch.schema(), *table.schema());
        assert!(batch.num_rows() > 0, "no empty batches");
        let ids = batch.column(0).as_primitive::<Int64Type>();
        let values = batch.column(1).as_primitive::<Float64Type>();
        for i in 0..batch.num_rows() {
            assert!(
                out.insert(ids.value(i), values.value(i)).is_none(),
                "one row per key"
            );
        }
    }
    out
}

#[test]
fn an_empty_table_has_no_batches() {
    assert!(table().snapshot().unwrap().is_empty());
}

#[test]
fn the_snapshot_holds_each_keys_latest_values() {
    let mut t = table();
    t.upsert(prices(vec![1, 2, 3], vec![10.0, 20.0, 30.0]))
        .unwrap();
    t.upsert(prices(vec![2, 4], vec![21.0, 40.0])).unwrap();
    t.delete(&Int64Array::from(vec![3])).unwrap();
    let expected = BTreeMap::from([(1, 10.0), (2, 21.0), (4, 40.0)]);
    assert_eq!(rows(&t), expected);
    t.compact().unwrap();
    assert_eq!(rows(&t), expected, "the same after compaction");
}

#[test]
fn wholly_live_batches_are_shared_and_dead_ones_left_out() {
    let mut t = table();
    t.upsert(prices(vec![1, 2], vec![1.0, 2.0])).unwrap();
    t.upsert(prices(vec![3, 4], vec![3.0, 4.0])).unwrap();
    // The first batch is wholly superseded, the second stays whole, the third is new.
    t.upsert(prices(vec![1, 2, 5], vec![1.5, 2.5, 5.0]))
        .unwrap();
    let snapshot = t.snapshot().unwrap();
    assert_eq!(snapshot.len(), 2);
    for (batch, stored) in snapshot.iter().zip(&t.batches()[1..]) {
        assert!(
            Arc::ptr_eq(batch.column(1), stored.column(1)),
            "shared, not copied"
        );
    }
}

#[test]
fn partly_live_batches_keep_their_live_rows_in_order() {
    let mut t = table();
    t.upsert(prices(vec![1, 2, 3, 4], vec![1.0, 2.0, 3.0, 4.0]))
        .unwrap();
    t.upsert(prices(vec![2], vec![2.5])).unwrap();
    t.delete(&Int64Array::from(vec![4])).unwrap();
    let snapshot = t.snapshot().unwrap();
    let ids: Vec<Vec<i64>> = snapshot
        .iter()
        .map(|b| b.column(0).as_primitive::<Int64Type>().values().to_vec())
        .collect();
    assert_eq!(ids, [vec![1, 3], vec![2]]);
}

#[test]
fn a_snapshot_stays_as_it_was_taken() {
    let mut t = table();
    t.upsert(prices(vec![1], vec![1.0])).unwrap();
    let before = t.snapshot().unwrap();
    t.upsert(prices(vec![1], vec![9.0])).unwrap();
    t.compact().unwrap();
    let price = before[0].column(1).as_primitive::<Float64Type>().value(0);
    assert_eq!(price, 1.0);
}

#[test]
fn dictionary_columns_stay_dictionaries() {
    let batch = |ids: Vec<i64>, names: Vec<&str>| {
        let names: DictionaryArray<UInt8Type> = names.into_iter().collect();
        RecordBatch::try_from_iter([
            ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
            ("name", Arc::new(names) as ArrayRef),
        ])
        .unwrap()
    };
    let mut t = LiveTable::new(batch(vec![], vec![]).schema(), "id").unwrap();
    t.upsert(batch(vec![1, 2, 3], vec!["a", "b", "a"])).unwrap();
    t.upsert(batch(vec![2], vec!["c"])).unwrap();
    let snapshot = t.snapshot().unwrap();
    let names: Vec<String> = snapshot
        .iter()
        .flat_map(|b| {
            let names = b.column(1).as_dictionary::<UInt8Type>();
            let values = names.values().as_string::<i32>().clone();
            names
                .keys()
                .iter()
                .map(move |k| values.value(k.unwrap() as usize).to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(names, ["a", "a", "c"]);
    assert!(snapshot
        .iter()
        .all(|b| b.column(1).data_type() == t.schema().field(1).data_type()));
}

#[test]
fn many_updates_snapshot_like_a_model() {
    let mut t = table();
    let mut model = BTreeMap::new();
    let mut seed = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = |below: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % below
    };
    for step in 0..300 {
        let n = 1 + next(20) as usize;
        let ids: Vec<i64> = (0..n).map(|_| next(200) as i64).collect();
        // One row per key in a batch: the last of duplicates wins, as in the model.
        let mut unique: BTreeMap<i64, f64> = BTreeMap::new();
        for (i, id) in ids.iter().enumerate() {
            unique.insert(*id, (step * 100 + i) as f64);
        }
        if next(4) == 0 {
            let gone: Vec<i64> = unique.keys().copied().collect();
            t.delete(&Int64Array::from(gone.clone())).unwrap();
            for id in gone {
                model.remove(&id);
            }
        } else {
            let (ids, values): (Vec<i64>, Vec<f64>) = unique.iter().map(|(k, v)| (*k, *v)).unzip();
            t.upsert(prices(ids, values)).unwrap();
            model.extend(unique);
        }
        if next(10) == 0 {
            t.compact().unwrap();
        }
        assert_eq!(rows(&t), model, "step {step}");
    }
}
