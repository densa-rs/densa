//! Grouped aggregates: checked against a full recomputation after every random change, plus
//! the API's edges.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::{Float16Type, Float32Type, UInt8Type};
use arrow_array::{
    Array, ArrayRef, ArrowPrimitiveType, BooleanArray, Decimal128Array, DictionaryArray,
    Float16Array, Float32Array, Float64Array, Int32Array, Int64Array, ListArray, RecordBatch,
    StringArray, TimestampNanosecondArray, UInt64Array, UInt8Array,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use densa_data::{
    Aggregate, CompareOp, DataError, Filter, GroupSpec, GroupsId, LiveTable, Scalar, ViewSpec,
};

const STEPS: usize = 2_000;
const KEYS: u64 = 200;
const REGIONS: [&str; 4] = ["north", "south", "east", "west"];

/// One row of the model: region, flag, quantity, price, amount (in cents).
#[derive(Debug, Clone, PartialEq)]
struct Row {
    region: Option<String>,
    flag: bool,
    qty: Option<i32>,
    price: Option<f64>,
    cents: i128,
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("region", DataType::Utf8, true),
        Field::new("flag", DataType::Boolean, false),
        Field::new("qty", DataType::Int32, true),
        Field::new("price", DataType::Float64, true),
        Field::new("amount", DataType::Decimal128(12, 2), false),
    ]))
}

/// Mostly tenths (not exact in binary, so sums round), sometimes a value that would poison
/// a plain running sum.
fn random_price(rng: &mut Rng) -> Option<f64> {
    if rng.below(7) == 0 {
        return None;
    }
    Some(match rng.below(200) {
        0 => f64::NAN,
        1 => -f64::NAN,
        2 => f64::INFINITY,
        3 => f64::NEG_INFINITY,
        4 => f64::MAX,
        5 => -f64::MAX,
        6 => -0.0,
        _ => rng.below(4_000) as f64 / 10.0 - 100.0,
    })
}

fn random_row(rng: &mut Rng) -> Row {
    Row {
        region: (rng.below(10) != 0).then(|| REGIONS[rng.below(4) as usize].to_owned()),
        flag: rng.below(2) == 0,
        qty: (rng.below(6) != 0).then(|| rng.below(100) as i32),
        price: random_price(rng),
        cents: i128::from(rng.below(100_000)),
    }
}

fn batch(rows: &[(i64, Row)]) -> RecordBatch {
    let pick = |f: fn(&Row) -> Option<&str>| rows.iter().map(|(_, r)| f(r)).collect::<Vec<_>>();
    let amounts = Decimal128Array::from(rows.iter().map(|(_, r)| r.cents).collect::<Vec<_>>())
        .with_precision_and_scale(12, 2)
        .unwrap();
    let columns: Vec<ArrayRef> = vec![
        Arc::new(Int64Array::from(
            rows.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(pick(|r| r.region.as_deref()))),
        Arc::new(BooleanArray::from(
            rows.iter().map(|(_, r)| r.flag).collect::<Vec<_>>(),
        )),
        Arc::new(Int32Array::from(
            rows.iter().map(|(_, r)| r.qty).collect::<Vec<_>>(),
        )),
        Arc::new(Float64Array::from(
            rows.iter().map(|(_, r)| r.price).collect::<Vec<_>>(),
        )),
        Arc::new(amounts),
    ];
    RecordBatch::try_new(schema(), columns).unwrap()
}

fn canonical(v: f64) -> f64 {
    if v.is_nan() {
        f64::NAN
    } else if v == 0.0 {
        0.0
    } else {
        v
    }
}

/// What a float column's values add up to, worked out by kind: NaN and infinities decide
/// alone; `f64::MAX` (the only huge value) counts in whole units; the rest sum plainly.
#[derive(Debug, Clone, Default)]
struct FloatTotal {
    nan: bool,
    infinity: bool,
    negative_infinity: bool,
    maxes: i32,
    rest: f64,
    values: u64,
}

impl FloatTotal {
    fn add(&mut self, v: f64) {
        self.values += 1;
        if v.is_nan() {
            self.nan = true;
        } else if v == f64::INFINITY {
            self.infinity = true;
        } else if v == f64::NEG_INFINITY {
            self.negative_infinity = true;
        } else if v.abs() == f64::MAX {
            self.maxes += if v > 0.0 { 1 } else { -1 };
        } else {
            self.rest += v;
        }
    }

    fn divided(&self, by: f64) -> f64 {
        if self.nan || (self.infinity && self.negative_infinity) {
            f64::NAN
        } else if self.infinity {
            f64::INFINITY
        } else if self.negative_infinity {
            f64::NEG_INFINITY
        } else {
            f64::from(self.maxes) * (f64::MAX / by) + self.rest / by
        }
    }

    fn sum(&self) -> Option<f64> {
        (self.values > 0).then(|| self.divided(1.0))
    }

    fn mean(&self) -> Option<f64> {
        (self.values > 0).then(|| self.divided(self.values as f64))
    }
}

/// What one group should show: count, sum(qty), min, max, sum and mean of price, sum(amount)
/// in cents.
#[derive(Debug, Clone, PartialEq, Default)]
struct Expected {
    count: u64,
    qty: Option<i128>,
    min: Option<f64>,
    max: Option<f64>,
    sum: Option<f64>,
    mean: Option<f64>,
    cents: i128,
}

type Key = (Option<String>, bool);
type Shown = Vec<(Key, Expected)>;

/// Groups ordered like the table's: region with null last, then flag.
type ModelKey = (bool, Option<String>, bool);

fn expected(rows: &BTreeMap<i64, Row>, keep: impl Fn(&Row) -> bool) -> Shown {
    let mut groups: BTreeMap<ModelKey, (Expected, FloatTotal)> = BTreeMap::new();
    for row in rows.values().filter(|r| keep(r)) {
        let key = (row.region.is_none(), row.region.clone(), row.flag);
        let (group, prices) = groups.entry(key).or_default();
        group.count += 1;
        if let Some(q) = row.qty {
            *group.qty.get_or_insert(0) += i128::from(q);
        }
        if let Some(p) = row.price.map(canonical) {
            let lowest = |m: f64| {
                if p.total_cmp(&m) == Ordering::Less {
                    p
                } else {
                    m
                }
            };
            let highest = |m: f64| {
                if p.total_cmp(&m) == Ordering::Greater {
                    p
                } else {
                    m
                }
            };
            group.min = Some(group.min.map_or(p, lowest));
            group.max = Some(group.max.map_or(p, highest));
            prices.add(p);
        }
        group.cents += row.cents;
    }
    groups
        .into_iter()
        .map(|((_, region, flag), (mut group, prices))| {
            group.sum = prices.sum();
            group.mean = prices.mean();
            ((region, flag), group)
        })
        .collect()
}

fn downcast<'a, T: 'static>(batch: &'a RecordBatch, name: &str) -> &'a T {
    batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("no column {name}"))
        .as_any()
        .downcast_ref::<T>()
        .unwrap_or_else(|| panic!("column {name} has another type"))
}

fn shown(batch: &RecordBatch) -> Shown {
    let region = downcast::<StringArray>(batch, "region");
    let flag = downcast::<BooleanArray>(batch, "flag");
    let count = downcast::<UInt64Array>(batch, "rows");
    let qty = downcast::<Decimal128Array>(batch, "sum(qty)");
    let min = downcast::<Float64Array>(batch, "cheapest");
    let max = downcast::<Float64Array>(batch, "dearest");
    let sum = downcast::<Float64Array>(batch, "sum(price)");
    let mean = downcast::<Float64Array>(batch, "mean_price");
    let cents = downcast::<Decimal128Array>(batch, "total");
    let float = |a: &Float64Array, i: usize| a.is_valid(i).then(|| a.value(i));
    (0..batch.num_rows())
        .map(|i| {
            let key = (
                region.is_valid(i).then(|| region.value(i).to_owned()),
                flag.value(i),
            );
            let group = Expected {
                count: count.value(i),
                qty: qty.is_valid(i).then(|| qty.value(i)),
                min: float(min, i),
                max: float(max, i),
                sum: float(sum, i),
                mean: float(mean, i),
                cents: cents.value(i),
            };
            (key, group)
        })
        .collect()
}

fn spec() -> GroupSpec {
    GroupSpec::new()
        .group_by("region")
        .group_by("flag")
        .aggregate(Aggregate::count().named("rows"))
        .aggregate(Aggregate::sum("qty"))
        .aggregate(Aggregate::min("price").named("cheapest"))
        .aggregate(Aggregate::max("price").named("dearest"))
        .aggregate(Aggregate::sum("price"))
        .aggregate(Aggregate::mean("price").named("mean_price"))
        .aggregate(Aggregate::sum("amount").named("total"))
}

/// Equal bits (canonical values: one NaN, one zero).
fn same_bits(a: Option<f64>, b: Option<f64>) -> bool {
    a.map(f64::to_bits) == b.map(f64::to_bits)
}

/// Equal NaN-ness and infinities; finite values within rounding.
fn close(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) if a.is_finite() && b.is_finite() => {
            (a - b).abs() <= 1e-9 * b.abs().max(1.0)
        }
        (Some(a), Some(b)) => (a.is_nan() && b.is_nan()) || a == b,
        (a, b) => a == b,
    }
}

fn assert_matches(shown: &Shown, want: &Shown, step: usize) {
    assert_eq!(
        shown.len(),
        want.len(),
        "step {step}: groups\n{shown:?}\n{want:?}"
    );
    for ((key, got), (want_key, want)) in shown.iter().zip(want) {
        assert_eq!(key, want_key, "step {step}");
        let context = format!("step {step} {key:?}:\n{got:?}\n{want:?}");
        assert!(same_bits(got.min, want.min), "{context}");
        assert!(same_bits(got.max, want.max), "{context}");
        assert!(close(got.sum, want.sum), "{context}");
        assert!(close(got.mean, want.mean), "{context}");
        let exact = |e: &Expected| (e.count, e.qty, e.cents);
        assert_eq!(exact(got), exact(want), "{context}");
    }
}

fn check(table: &LiveTable, id: GroupsId, want: &Shown, step: usize) {
    let got = shown(&table.groups(id).unwrap().to_batch().unwrap());
    assert_matches(&got, want, step);
}

#[test]
fn grouped_aggregates_match_a_full_recomputation_after_every_change() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut table = LiveTable::new(schema(), "id").unwrap();
    let all = table.add_groups(spec()).unwrap();
    let busy_rows = |r: &Row| r.qty.is_some_and(|q| q > 30);
    let busy = table
        .add_groups(spec().filter(Filter::compare("qty", CompareOp::Gt, Scalar::Int(30))))
        .unwrap();
    let mut late = None;
    let mut model: BTreeMap<i64, Row> = BTreeMap::new();

    for step in 0..STEPS {
        match rng.below(20) {
            0..=12 => {
                let len = 1 + rng.below(10) as usize;
                let rows: Vec<(i64, Row)> = (0..len)
                    .map(|_| (rng.below(KEYS) as i64, random_row(&mut rng)))
                    .collect();
                table.upsert(batch(&rows)).unwrap();
                model.extend(rows);
            }
            13 => {
                let rows: Vec<(i64, Row)> = (0..KEYS as i64)
                    .map(|id| (id, random_row(&mut rng)))
                    .collect();
                table.upsert(batch(&rows)).unwrap();
                model.extend(rows);
            }
            14..=17 => {
                let keys: Vec<i64> = (0..3).map(|_| rng.below(KEYS) as i64).collect();
                table.delete(&Int64Array::from(keys.clone())).unwrap();
                for key in keys {
                    model.remove(&key);
                }
            }
            _ => table.compact().unwrap(),
        }
        if step == STEPS / 2 {
            // Added to a filled table: counted from its rows.
            late = Some(table.add_groups(spec()).unwrap());
        }
        let everything = expected(&model, |_| true);
        check(&table, all, &everything, step);
        check(&table, busy, &expected(&model, busy_rows), step);
        if let Some(late) = late {
            check(&table, late, &everything, step);
        }
    }
}

#[test]
fn the_revision_moves_with_every_change_to_the_groups() {
    let mut table = LiveTable::new(schema(), "id").unwrap();
    let id = table
        .add_groups(spec().filter(Filter::compare("qty", CompareOp::Gt, Scalar::Int(50))))
        .unwrap();
    let revision = |t: &LiveTable| t.groups(id).unwrap().revision();
    let before = revision(&table);
    let mut rng = Rng(7);
    let mut row = random_row(&mut rng);
    row.qty = Some(1); // filtered out: the groups do not change
                       // Enough rows that changing one patches the groups instead of rebuilding them.
    let filler: Vec<(i64, Row)> = (1..=20).map(|id| (id, row.clone())).collect();
    table.upsert(batch(&filler)).unwrap();
    assert_eq!(revision(&table), before);
    table.upsert(batch(&[(3, row.clone())])).unwrap();
    assert_eq!(
        revision(&table),
        before,
        "a changed row that is not counted"
    );
    table.delete(&Int64Array::from(vec![2])).unwrap();
    assert_eq!(revision(&table), before, "a row that was not counted left");
    row.qty = Some(99);
    table.upsert(batch(&[(1, row)])).unwrap();
    let counted = revision(&table);
    assert!(counted > before);
    assert_eq!(table.groups(id).unwrap().len(), 1);
    table.delete(&Int64Array::from(vec![1])).unwrap();
    assert!(revision(&table) > counted, "a counted row left");
}

#[test]
fn group_specs_are_checked_against_the_schema() {
    let odd = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("region", DataType::Utf8, true),
        Field::new("flag", DataType::Boolean, true),
        Field::new(
            "at",
            DataType::Timestamp(TimeUnit::Nanosecond, Some("UTC".into())),
            true,
        ),
        Field::new("tags", DataType::new_list(DataType::Utf8, true), true),
    ]));
    let bad = |spec: GroupSpec| {
        LiveTable::new(Arc::clone(&odd), "id")
            .unwrap()
            .add_groups(spec)
            .unwrap_err()
    };
    let cases = [
        (
            GroupSpec::new().group_by("nope"),
            DataError::UnknownColumn("nope".into()),
        ),
        (
            GroupSpec::new().group_by("tags"),
            DataError::UnsupportedColumn("tags".into()),
        ),
        (
            GroupSpec::new().aggregate(Aggregate::sum("region")),
            DataError::NotNumeric("region".into()),
        ),
        (
            GroupSpec::new().aggregate(Aggregate::mean("flag")),
            DataError::NotNumeric("flag".into()),
        ),
        (
            GroupSpec::new().aggregate(Aggregate::sum("at")),
            DataError::NotNumeric("at".into()),
        ),
        (
            GroupSpec::new()
                .aggregate(Aggregate::count())
                .aggregate(Aggregate::count()),
            DataError::DuplicateName("count".into()),
        ),
        (
            GroupSpec::new()
                .group_by("region")
                .aggregate(Aggregate::min("at").named("region")),
            DataError::DuplicateName("region".into()),
        ),
        (
            GroupSpec::new().filter(Filter::compare("region", CompareOp::Gt, Scalar::Int(1))),
            DataError::LiteralMismatch("region".into()),
        ),
    ];
    for (spec, error) in cases {
        assert_eq!(bad(spec.clone()), error, "{spec:?}");
    }
    // Min and max read anything views sort: a timestamp is fine.
    let ok = GroupSpec::new()
        .aggregate(Aggregate::min("at"))
        .aggregate(Aggregate::max("at"));
    assert!(LiveTable::new(odd, "id").unwrap().add_groups(ok).is_ok());
}

#[test]
fn aggregates_are_named_after_their_function_and_column() {
    let mut table = LiveTable::new(schema(), "id").unwrap();
    let id = table
        .add_groups(
            GroupSpec::new()
                .group_by("region")
                .aggregate(Aggregate::count())
                .aggregate(Aggregate::sum("qty"))
                .aggregate(Aggregate::min("price"))
                .aggregate(Aggregate::max("price"))
                .aggregate(Aggregate::mean("price"))
                .aggregate(Aggregate::mean("qty").named("average")),
        )
        .unwrap();
    let out = table.groups(id).unwrap().to_batch().unwrap();
    let names: Vec<&str> = out
        .schema_ref()
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect();
    assert_eq!(
        names,
        [
            "region",
            "count",
            "sum(qty)",
            "min(price)",
            "max(price)",
            "mean(price)",
            "average"
        ]
    );
    let nullable: Vec<bool> = out
        .schema_ref()
        .fields()
        .iter()
        .map(|f| f.is_nullable())
        .collect();
    assert_eq!(
        nullable,
        [true, false, true, true, true, true, true],
        "counts never are"
    );
}

#[test]
fn a_group_whose_only_value_is_nan_sums_to_nan_not_null() {
    let (out, _, _) = sum_and_mean(Arc::new(Float64Array::from(vec![f64::NAN])));
    let sum = downcast::<Float64Array>(&out, "sum(v)");
    let mean = downcast::<Float64Array>(&out, "mean(v)");
    assert!(sum.is_valid(0) && sum.value(0).is_nan());
    assert!(mean.is_valid(0) && mean.value(0).is_nan());
}

#[test]
fn float32_and_float16_keys_come_out_canonical_too() {
    type F16 = <Float16Type as ArrowPrimitiveType>::Native;
    // -0.0 and 0.0 form one group whose first row holds -0.0; -NaN is the NaN group's.
    let values = [-0.0, 0.0, -f64::NAN, 1.5];
    let groups = |keys: ArrayRef| {
        let batch = one_column(keys);
        let mut table = LiveTable::new(batch.schema(), "id").unwrap();
        let id = table
            .add_groups(
                GroupSpec::new()
                    .group_by("v")
                    .aggregate(Aggregate::min("v")),
            )
            .unwrap();
        table.upsert(batch).unwrap();
        table.groups(id).unwrap().to_batch().unwrap()
    };
    let out = groups(Arc::new(Float32Array::from(
        values.map(|v| v as f32).to_vec(),
    )));
    for column in out.columns() {
        let bits: Vec<u32> = column
            .as_primitive::<Float32Type>()
            .values()
            .iter()
            .map(|v| v.to_bits())
            .collect();
        assert_eq!(bits, [0.0f32, 1.5, f32::NAN].map(f32::to_bits));
    }
    let out = groups(Arc::new(Float16Array::from_iter_values(
        values.map(F16::from_f64),
    )));
    for column in out.columns() {
        let bits: Vec<u16> = column
            .as_primitive::<Float16Type>()
            .values()
            .iter()
            .map(|v| v.to_bits())
            .collect();
        let expected = [0.0, 1.5, f64::NAN].map(|v| F16::from_f64(v).to_bits());
        assert_eq!(bits, expected);
    }
}

#[test]
fn without_keys_there_is_one_group_even_over_no_rows() {
    let mut table = LiveTable::new(schema(), "id").unwrap();
    let id = table
        .add_groups(
            GroupSpec::new()
                .aggregate(Aggregate::count())
                .aggregate(Aggregate::sum("qty"))
                .aggregate(Aggregate::max("price")),
        )
        .unwrap();
    let read = |t: &LiveTable| {
        let groups = t.groups(id).unwrap();
        assert_eq!(groups.len(), 1);
        assert!(!groups.is_empty());
        let out = groups.to_batch().unwrap();
        assert_eq!(out.num_rows(), 1);
        let count = downcast::<UInt64Array>(&out, "count").value(0);
        (count, out.column(1).is_null(0), out.column(2).is_null(0))
    };
    assert_eq!(read(&table), (0, true, true));
    let mut rng = Rng(3);
    let rows: Vec<(i64, Row)> = (0..5).map(|id| (id, random_row(&mut rng))).collect();
    table.upsert(batch(&rows)).unwrap();
    assert_eq!(read(&table).0, 5);
    table
        .delete(&Int64Array::from((0..5).collect::<Vec<_>>()))
        .unwrap();
    assert_eq!(read(&table), (0, true, true), "every row left, one by one");
}

#[test]
fn groups_added_to_a_filled_table_count_its_rows() {
    let mut table = LiveTable::new(schema(), "id").unwrap();
    let mut rng = Rng(5);
    let rows: Vec<(i64, Row)> = (0..50).map(|id| (id, random_row(&mut rng))).collect();
    table.upsert(batch(&rows)).unwrap();
    let id = table
        .add_groups(GroupSpec::new().aggregate(Aggregate::count()))
        .unwrap();
    let out = table.groups(id).unwrap().to_batch().unwrap();
    assert_eq!(downcast::<UInt64Array>(&out, "count").value(0), 50);
}

#[test]
fn ids_belong_to_their_table_and_to_clones_made_after_them() {
    let mut a = LiveTable::new(schema(), "id").unwrap();
    let id = a.add_groups(spec()).unwrap();
    let view = a.add_view(ViewSpec::new()).unwrap();
    assert!(LiveTable::new(schema(), "id").unwrap().groups(id).is_none());
    let mut b = a.clone();
    assert!(b.groups(id).is_some() && b.view(view).is_some());
    // Added after the clone: each id resolves on its own table only.
    let on_a = a
        .add_groups(GroupSpec::new().aggregate(Aggregate::count()))
        .unwrap();
    let on_b = b
        .add_groups(GroupSpec::new().aggregate(Aggregate::sum("qty")))
        .unwrap();
    assert!(a.groups(on_b).is_none() && b.groups(on_a).is_none());
    let view_a = a.add_view(ViewSpec::new()).unwrap();
    let view_b = b.add_view(ViewSpec::new()).unwrap();
    assert!(a.view(view_b).is_none() && b.view(view_a).is_none());
    assert!(!a.remove_groups(on_b) && !a.remove_view(view_b));
    assert!(a.remove_groups(id));
    assert!(a.groups(id).is_none() && b.groups(id).is_some());
    assert!(!a.remove_groups(id));
}

/// A table of one column `v` of `values`'s type, keyed by `id`.
fn one_column(values: ArrayRef) -> RecordBatch {
    let ids = (0..values.len() as i64).collect::<Vec<_>>();
    RecordBatch::try_from_iter([
        ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
        ("v", values),
    ])
    .unwrap()
}

/// Sum and mean of `v` over `values`, as a table with these groups computes them.
fn sum_and_mean(values: ArrayRef) -> (RecordBatch, LiveTable, GroupsId) {
    let batch = one_column(values);
    let mut table = LiveTable::new(batch.schema(), "id").unwrap();
    let id = table
        .add_groups(
            GroupSpec::new()
                .aggregate(Aggregate::sum("v"))
                .aggregate(Aggregate::mean("v")),
        )
        .unwrap();
    table.upsert(batch.clone()).unwrap();
    let out = table.groups(id).unwrap().to_batch().unwrap();
    (out, table, id)
}

fn decimals(values: Vec<i128>, precision: u8, scale: i8) -> ArrayRef {
    Arc::new(
        Decimal128Array::from(values)
            .with_precision_and_scale(precision, scale)
            .unwrap(),
    )
}

fn decimal_sum(table: &LiveTable, id: GroupsId) -> Option<i128> {
    let out = table.groups(id).unwrap().to_batch().unwrap();
    let sum = downcast::<Decimal128Array>(&out, "sum(v)");
    sum.is_valid(0).then(|| sum.value(0))
}

#[test]
fn decimal_sums_keep_the_scale_and_stop_at_38_digits() {
    let (out, _, _) = sum_and_mean(decimals(vec![150, 250], 12, 2));
    assert_eq!(
        out.schema().field(0).data_type(),
        &DataType::Decimal128(38, 2)
    );
    assert_eq!(downcast::<Decimal128Array>(&out, "sum(v)").value(0), 400);
    assert_eq!(downcast::<Float64Array>(&out, "mean(v)").value(0), 2.0);

    // Enough rows that changing one patches the groups instead of rebuilding them.
    let largest = 10i128.pow(38) - 1;
    let mut values = vec![0; 100];
    values[0] = largest;
    let (_, mut table, id) = sum_and_mean(decimals(values, 38, 0));
    assert_eq!(decimal_sum(&table, id), Some(largest), "38 nines fit");
    let set = |table: &mut LiveTable, row: i64, value: i128| {
        let batch = RecordBatch::try_from_iter([
            ("id", Arc::new(Int64Array::from(vec![row])) as ArrayRef),
            ("v", decimals(vec![value], 38, 0)),
        ])
        .unwrap();
        table.upsert(batch).unwrap();
    };
    set(&mut table, 1, 1);
    assert_eq!(decimal_sum(&table, id), None, "10^38 needs 39 digits");
    set(&mut table, 1, largest);
    assert_eq!(decimal_sum(&table, id), None, "beyond i128");
    set(&mut table, 1, -largest);
    assert_eq!(
        decimal_sum(&table, id),
        Some(0),
        "back once the values change"
    );
}

#[test]
fn integer_sums_are_exact_decimals_beyond_64_bits() {
    let sum = |out: &RecordBatch| {
        assert_eq!(
            out.schema().field(0).data_type(),
            &DataType::Decimal128(38, 0)
        );
        downcast::<Decimal128Array>(out, "sum(v)").value(0)
    };
    let (out, _, _) = sum_and_mean(Arc::new(UInt64Array::from(vec![u64::MAX, 3])));
    assert_eq!(sum(&out), i128::from(u64::MAX) + 3);
    let mean = downcast::<Float64Array>(&out, "mean(v)").value(0);
    assert_eq!(mean, u64::MAX as f64 / 2.0);
    let (out, _, _) = sum_and_mean(Arc::new(Int64Array::from(vec![i64::MAX, 1])));
    assert_eq!(sum(&out), i128::from(i64::MAX) + 1);
    let (out, _, _) = sum_and_mean(Arc::new(Int64Array::from(vec![i64::MIN, -1])));
    assert_eq!(sum(&out), i128::from(i64::MIN) - 1);
    let (out, _, _) = sum_and_mean(Arc::new(Int32Array::from(vec![7, -2])));
    assert_eq!(sum(&out), 5);
}

#[test]
fn decimal_means_use_the_scale_whatever_its_sign() {
    for (scale, mean) in [(2, 0.04), (-2, 400.0), (-128, 4e128)] {
        let (out, _, _) = sum_and_mean(decimals(vec![3, 5], 10, scale));
        let got = downcast::<Float64Array>(&out, "mean(v)").value(0);
        assert!((got - mean).abs() <= mean * 1e-12, "scale {scale}: {got}");
    }
}

#[test]
fn float_sums_keep_low_order_bits_and_recover_from_nan() {
    let (out, _, _) = sum_and_mean(Arc::new(Float64Array::from(vec![1e16, 1.0, -1e16])));
    // A plain running sum gives 0: 1e16 + 1 rounds to 1e16.
    assert_eq!(downcast::<Float64Array>(&out, "sum(v)").value(0), 1.0);
    assert_eq!(
        downcast::<Float64Array>(&out, "mean(v)").value(0),
        1.0 / 3.0
    );

    let (_, mut table, id) = sum_and_mean(Arc::new(Float64Array::from(vec![1.0; 100])));
    let sum = |t: &LiveTable| {
        let out = t.groups(id).unwrap().to_batch().unwrap();
        downcast::<Float64Array>(&out, "sum(v)").value(0)
    };
    for poison in [f64::NAN, f64::INFINITY, f64::MAX] {
        table
            .upsert(one_column(Arc::new(Float64Array::from(vec![
                poison, poison,
            ]))))
            .unwrap();
        assert!(!sum(&table).is_finite(), "{poison} twice");
        table
            .upsert(one_column(Arc::new(Float64Array::from(vec![1.0, 1.0]))))
            .unwrap();
        assert_eq!(sum(&table), 100.0, "after {poison}");
    }
}

#[test]
fn float_keys_merge_signed_zeros_and_nans_and_come_out_canonical() {
    let keys = Float64Array::from(vec![-0.0, 0.0, -f64::NAN, f64::NAN, 1.5]);
    let batch = one_column(Arc::new(keys));
    let mut table = LiveTable::new(batch.schema(), "id").unwrap();
    let id = table
        .add_groups(
            GroupSpec::new()
                .group_by("v")
                .aggregate(Aggregate::count())
                .aggregate(Aggregate::min("v")),
        )
        .unwrap();
    table.upsert(batch).unwrap();
    let out = table.groups(id).unwrap().to_batch().unwrap();
    let key = downcast::<Float64Array>(&out, "v");
    let bits: Vec<u64> = key.values().iter().map(|v| v.to_bits()).collect();
    assert_eq!(bits, [0.0f64, 1.5, f64::NAN].map(f64::to_bits));
    let min = downcast::<Float64Array>(&out, "min(v)");
    assert_eq!(
        min.value(0).to_bits(),
        0.0f64.to_bits(),
        "-0.0 comes out as 0.0"
    );
    let count = downcast::<UInt64Array>(&out, "count");
    assert_eq!(count.values().as_ref(), [2, 1, 2]);
}

#[test]
fn dictionary_columns_come_out_decoded_even_past_their_key_type() {
    // Two batches of 200 distinct strings each: 400 groups, more than a u8 key can index.
    let dictionary = |first: usize| {
        let values: Vec<String> = (first..first + 200).map(|i| format!("v{i:03}")).collect();
        let keys = UInt8Array::from((0..200u8).collect::<Vec<_>>());
        let values = StringArray::from(values);
        DictionaryArray::<UInt8Type>::try_new(keys, Arc::new(values)).unwrap()
    };
    let rows = |first: usize| {
        let ids: Vec<i64> = (first as i64..first as i64 + 200).collect();
        RecordBatch::try_from_iter([
            ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
            ("v", Arc::new(dictionary(first)) as ArrayRef),
        ])
        .unwrap()
    };
    let mut table = LiveTable::new(rows(0).schema(), "id").unwrap();
    let id = table
        .add_groups(
            GroupSpec::new()
                .group_by("v")
                .aggregate(Aggregate::max("v")),
        )
        .unwrap();
    table.upsert(rows(0)).unwrap();
    table.upsert(rows(200)).unwrap();
    let out = table.groups(id).unwrap().to_batch().unwrap();
    assert_eq!(out.num_rows(), 400);
    assert_eq!(out.schema().field(0).data_type(), &DataType::Utf8);
    assert_eq!(out.schema().field(1).data_type(), &DataType::Utf8);
    let keys = downcast::<StringArray>(&out, "v");
    let max = downcast::<StringArray>(&out, "max(v)");
    assert_eq!((keys.value(0), keys.value(399)), ("v000", "v399"));
    assert_eq!(max.value(250), "v250");
}

#[test]
fn keys_and_extremes_keep_their_column_types() {
    let at = TimestampNanosecondArray::from(vec![5, 5, 9]).with_timezone("UTC");
    let batch = RecordBatch::try_from_iter([
        ("id", Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef),
        ("at", Arc::new(at) as ArrayRef),
        (
            "name",
            Arc::new(StringArray::from(vec!["b", "a", "c"])) as ArrayRef,
        ),
        (
            "ok",
            Arc::new(BooleanArray::from(vec![true, false, true])) as ArrayRef,
        ),
        ("n", Arc::new(Int32Array::from(vec![7, -2, 4])) as ArrayRef),
    ])
    .unwrap();
    let mut table = LiveTable::new(batch.schema(), "id").unwrap();
    let id = table
        .add_groups(
            GroupSpec::new()
                .group_by("at")
                .aggregate(Aggregate::min("name"))
                .aggregate(Aggregate::max("name"))
                .aggregate(Aggregate::min("ok"))
                .aggregate(Aggregate::max("n")),
        )
        .unwrap();
    table.upsert(batch.clone()).unwrap();
    let out = table.groups(id).unwrap().to_batch().unwrap();
    let sources = ["at", "name", "name", "ok", "n"];
    for (field, source) in out.schema_ref().fields().iter().zip(sources) {
        let source = batch.column_by_name(source).unwrap().data_type();
        assert_eq!(field.data_type(), source, "{field:?}");
    }
    let at = downcast::<TimestampNanosecondArray>(&out, "at");
    assert_eq!(at.values().as_ref(), [5, 9]);
    let min = downcast::<StringArray>(&out, "min(name)");
    let max = downcast::<StringArray>(&out, "max(name)");
    assert_eq!((min.value(0), max.value(0)), ("a", "b"));
    assert!(!downcast::<BooleanArray>(&out, "min(ok)").value(0));
    assert_eq!(downcast::<Int32Array>(&out, "max(n)").value(0), 7);
}

#[test]
fn list_columns_are_refused_as_aggregates_too() {
    let tags =
        ListArray::from_iter_primitive::<arrow_array::types::Int32Type, _, _>(vec![Some(vec![
            Some(1),
        ])]);
    let batch = one_column(Arc::new(tags));
    let mut table = LiveTable::new(batch.schema(), "id").unwrap();
    assert_eq!(
        table.add_groups(GroupSpec::new().aggregate(Aggregate::max("v"))),
        Err(DataError::UnsupportedColumn("v".into()))
    );
}

#[test]
fn integer_sums_and_means_follow_values_turning_null_and_rows_leaving() {
    let rows = |ids: Vec<i64>, qty: Vec<Option<i64>>| {
        RecordBatch::try_from_iter([
            ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
            ("qty", Arc::new(Int64Array::from(qty)) as ArrayRef),
        ])
        .unwrap()
    };
    let mut table = LiveTable::new(rows(vec![], vec![]).schema(), "id").unwrap();
    let id = table
        .add_groups(
            GroupSpec::new()
                .aggregate(Aggregate::sum("qty"))
                .aggregate(Aggregate::mean("qty")),
        )
        .unwrap();
    // Several rows at once (a rebuild), then one at a time (patches).
    table
        .upsert(rows(vec![1, 2], vec![Some(10), Some(20)]))
        .unwrap();
    for _ in 0..10 {
        table.upsert(rows(vec![100], vec![None])).unwrap(); // a row without a value
    }
    let read = |t: &LiveTable| {
        let out = t.groups(id).unwrap().to_batch().unwrap();
        let sum = downcast::<Decimal128Array>(&out, "sum(qty)");
        let mean = downcast::<Float64Array>(&out, "mean(qty)");
        (
            sum.is_valid(0).then(|| sum.value(0)),
            mean.is_valid(0).then(|| mean.value(0)),
        )
    };
    assert_eq!(read(&table), (Some(30), Some(15.0)));
    table.upsert(rows(vec![2], vec![None])).unwrap();
    assert_eq!(read(&table), (Some(10), Some(10.0)));
    table.delete(&Int64Array::from(vec![1])).unwrap();
    assert_eq!(
        read(&table),
        (None, None),
        "rows remain, but none has a value"
    );
}

#[test]
fn a_bulk_change_that_fills_or_empties_the_groups_moves_the_revision() {
    let mut table = LiveTable::new(schema(), "id").unwrap();
    let id = table
        .add_groups(spec().filter(Filter::compare("qty", CompareOp::Gt, Scalar::Int(50))))
        .unwrap();
    let empty = table.groups(id).unwrap().revision();
    let mut rng = Rng(11);
    let mut rows: Vec<(i64, Row)> = (0..20)
        .map(|i| {
            let mut row = random_row(&mut rng);
            row.qty = Some(99);
            (i, row)
        })
        .collect();
    table.upsert(batch(&rows)).unwrap(); // into an empty table: the groups are rebuilt
    assert!(!table.groups(id).unwrap().is_empty());
    let before = table.groups(id).unwrap().revision();
    assert!(before > empty);
    for (_, row) in &mut rows {
        row.qty = Some(1);
    }
    table.upsert(batch(&rows)).unwrap(); // every row changes: the groups are rebuilt, empty
    assert!(table.groups(id).unwrap().is_empty());
    assert!(table.groups(id).unwrap().revision() > before);
}
