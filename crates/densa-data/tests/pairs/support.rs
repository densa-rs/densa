//! What the pair tests share: two sides' schemas, rows of them, the output read back, and
//! the join recomputed from scratch.

use std::collections::BTreeMap;
use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::{Float64Type, Int64Type, UInt64Type};
use arrow_array::{Array, ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use densa_data::{LiveTable, PairJoin};

/// Orders: `order` (key), `customer` (joined on), `amount`.
pub fn left_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("order", DataType::Int64, false),
        Field::new("customer", DataType::Utf8, true),
        Field::new("amount", DataType::Int64, true),
    ]))
}

/// Addresses: `address` (key), `owner` (joined on), `distance`.
pub fn right_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("address", DataType::Int64, false),
        Field::new("owner", DataType::Utf8, true),
        Field::new("distance", DataType::Float64, true),
    ]))
}

pub fn join() -> (PairJoin, LiveTable) {
    let join = PairJoin::new(
        left_schema(),
        "order",
        "customer",
        right_schema(),
        "address",
        "owner",
    )
    .unwrap();
    let out = join.table().unwrap();
    (join, out)
}

pub fn lefts(rows: &[(i64, Option<&str>, i64)]) -> RecordBatch {
    let keys: ArrayRef = Arc::new(Int64Array::from_iter_values(rows.iter().map(|r| r.0)));
    let values: ArrayRef = Arc::new(rows.iter().map(|r| r.1).collect::<StringArray>());
    let amounts: ArrayRef = Arc::new(Int64Array::from_iter_values(rows.iter().map(|r| r.2)));
    RecordBatch::try_new(left_schema(), vec![keys, values, amounts]).unwrap()
}

pub fn rights(rows: &[(i64, Option<&str>, f64)]) -> RecordBatch {
    let keys: ArrayRef = Arc::new(Int64Array::from_iter_values(rows.iter().map(|r| r.0)));
    let values: ArrayRef = Arc::new(rows.iter().map(|r| r.1).collect::<StringArray>());
    let distances: ArrayRef = Arc::new(Float64Array::from_iter_values(rows.iter().map(|r| r.2)));
    RecordBatch::try_new(right_schema(), vec![keys, values, distances]).unwrap()
}

/// An output row: order, customer, amount, then the address and its distance (bits) if paired.
pub type Row = (i64, Option<String>, i64, Option<i64>, Option<u64>);

/// The output's rows, sorted, after checking that each one's pair id is unique.
pub fn rows(out: &LiveTable) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut pairs = Vec::new();
    for batch in out.snapshot().unwrap() {
        let order = batch.column(0).as_primitive::<Int64Type>();
        let customer = batch.column(1).as_string::<i32>();
        let amount = batch.column(2).as_primitive::<Int64Type>();
        let address = batch.column(3).as_primitive::<Int64Type>();
        let distance = batch.column(4).as_primitive::<Float64Type>();
        let pair = batch.column(5).as_primitive::<UInt64Type>();
        for row in 0..batch.num_rows() {
            rows.push((
                order.value(row),
                customer
                    .is_valid(row)
                    .then(|| customer.value(row).to_owned()),
                amount.value(row),
                address.is_valid(row).then(|| address.value(row)),
                distance
                    .is_valid(row)
                    .then(|| distance.value(row).to_bits()),
            ));
            pairs.push(pair.value(row));
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    assert_eq!(pairs.len(), rows.len(), "one pair id a row");
    rows.sort();
    rows
}

/// The left outer join of `left` and `right` (keyed rows: join value, payload).
pub fn expected(
    left: &BTreeMap<i64, (Option<String>, i64)>,
    right: &BTreeMap<i64, (Option<String>, f64)>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    for (&order, (customer, amount)) in left {
        let paired: Vec<Row> = right
            .iter()
            .filter(|(_, (owner, _))| customer.is_some() && owner == customer)
            .map(|(&address, (_, distance))| {
                let distance = Some(distance.to_bits());
                (order, customer.clone(), *amount, Some(address), distance)
            })
            .collect();
        if paired.is_empty() {
            rows.push((order, customer.clone(), *amount, None, None));
        }
        rows.extend(paired);
    }
    rows.sort();
    rows
}

/// A small random generator (xorshift), so every run sees the same cases.
pub struct Random(pub u64);

impl Random {
    pub fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}
