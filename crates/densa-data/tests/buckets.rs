//! Groups by bucket (per minute, per step): checked against a recount of the table's live
//! rows after every random change, windowed or not, plus the edges of each column type.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use arrow_array::cast::AsArray;
use arrow_array::types::{
    Date32Type, Decimal128Type, Float16Type, Float32Type, Float64Type, Int8Type,
    TimestampMillisecondType, UInt64Type,
};
use arrow_array::{
    Array, ArrayRef, BooleanArray, Date32Array, Decimal128Array, Float16Array, Float32Array,
    Float64Array, Int32Array, Int64Array, Int8Array, RecordBatch, StringArray,
    TimestampMillisecondArray, UInt64Array,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use densa_data::{Aggregate, DataError, GroupSpec, LiveTable, Window};

const MINUTE: i64 = 60_000;

#[allow(non_camel_case_types)]
type f16 = <Float16Type as arrow_array::ArrowPrimitiveType>::Native;

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
    let time = DataType::Timestamp(TimeUnit::Millisecond, Some("+00:00".into()));
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("time", time, true),
        Field::new("region", DataType::Utf8, true),
        Field::new("qty", DataType::Int32, true),
    ]))
}

type Row = (i64, Option<i64>, Option<&'static str>, Option<i32>);

fn batch(rows: &[Row]) -> RecordBatch {
    let times = TimestampMillisecondArray::from(rows.iter().map(|r| r.1).collect::<Vec<_>>())
        .with_timezone("+00:00");
    let columns: Vec<ArrayRef> = vec![
        Arc::new(Int64Array::from(
            rows.iter().map(|r| r.0).collect::<Vec<_>>(),
        )),
        Arc::new(times),
        Arc::new(StringArray::from(
            rows.iter().map(|r| r.2).collect::<Vec<_>>(),
        )),
        Arc::new(Int32Array::from(
            rows.iter().map(|r| r.3).collect::<Vec<_>>(),
        )),
    ];
    RecordBatch::try_new(schema(), columns).unwrap()
}

/// A group's key, ordered as groups are: values ascending, nulls (`None`) last.
type ModelKey = ((bool, String), (bool, i64));

/// Count, sum of qty, min and max time.
type Totals = (u64, Option<i128>, Option<i64>, Option<i64>);

/// The groups worked out afresh from the table's live rows.
fn recount(table: &LiveTable) -> BTreeMap<ModelKey, Totals> {
    let mut groups = BTreeMap::new();
    for batch in table.snapshot().unwrap() {
        let times = batch.column(1).as_primitive::<TimestampMillisecondType>();
        let regions = batch.column(2).as_string::<i32>();
        let qty = batch
            .column(3)
            .as_primitive::<arrow_array::types::Int32Type>();
        for row in 0..batch.num_rows() {
            let time = times.is_valid(row).then(|| times.value(row));
            let region = regions.is_valid(row).then(|| regions.value(row).to_owned());
            let start = time.map(|t| t.div_euclid(MINUTE) * MINUTE);
            let key = (
                (region.is_none(), region.unwrap_or_default()),
                (start.is_none(), start.unwrap_or_default()),
            );
            let totals: &mut Totals = groups.entry(key).or_default();
            totals.0 += 1;
            if qty.is_valid(row) {
                totals.1 = Some(totals.1.unwrap_or(0) + i128::from(qty.value(row)));
            }
            if let Some(time) = time {
                totals.2 = Some(totals.2.map_or(time, |m| m.min(time)));
                totals.3 = Some(totals.3.map_or(time, |m| m.max(time)));
            }
        }
    }
    groups
}

/// What the table's groups say, in the model's terms, after checking they come in key order.
fn read(batch: &RecordBatch) -> BTreeMap<ModelKey, Totals> {
    let groups = groups_of(batch);
    assert!(
        groups.windows(2).all(|w| w[0].0 < w[1].0),
        "groups in key order"
    );
    groups.into_iter().collect()
}

fn groups_of(batch: &RecordBatch) -> Vec<(ModelKey, Totals)> {
    let regions = batch.column(0).as_string::<i32>();
    let starts = batch.column(1).as_primitive::<TimestampMillisecondType>();
    let counts = batch.column(2).as_primitive::<UInt64Type>();
    let sums = batch.column(3).as_primitive::<Decimal128Type>();
    let mins = batch.column(4).as_primitive::<TimestampMillisecondType>();
    let maxes = batch.column(5).as_primitive::<TimestampMillisecondType>();
    let value = |a: &TimestampMillisecondArray, i| a.is_valid(i).then(|| a.value(i));
    (0..batch.num_rows())
        .map(|i| {
            let region = regions.is_valid(i).then(|| regions.value(i).to_owned());
            let start = value(starts, i);
            let key = (
                (region.is_none(), region.unwrap_or_default()),
                (start.is_none(), start.unwrap_or_default()),
            );
            let sum = sums.is_valid(i).then(|| sums.value(i));
            (key, (counts.value(i), sum, value(mins, i), value(maxes, i)))
        })
        .collect()
}

fn per_minute() -> GroupSpec {
    GroupSpec::new()
        .group_by("region")
        .group_by_time("time", Duration::from_secs(60))
        .aggregate(Aggregate::count())
        .aggregate(Aggregate::sum("qty"))
        .aggregate(Aggregate::min("time"))
        .aggregate(Aggregate::max("time"))
}

#[test]
fn buckets_follow_every_change() {
    const REGIONS: [&str; 3] = ["north", "south", "east"];
    for seed in 1..=12u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut table = LiveTable::new(schema(), "id").unwrap();
        if seed % 2 == 0 {
            let window = Window::duration("time", Duration::from_secs(600));
            table.set_window(Some(window)).unwrap();
        }
        let groups = table.add_groups(per_minute()).unwrap();
        let mut clock: i64 = -30 * MINUTE;
        for step in 0..400 {
            clock += i64::try_from(rng.below(20_000)).unwrap();
            if rng.below(10) == 0 {
                let ids: Vec<i64> = (0..rng.below(5))
                    .map(|_| i64::try_from(rng.below(300)).unwrap())
                    .collect();
                table.delete(&Int64Array::from(ids)).unwrap();
            } else {
                let rows: Vec<Row> = (0..1 + rng.below(20))
                    .map(|_| {
                        let id = i64::try_from(rng.below(300)).unwrap();
                        let age = i64::try_from(rng.below(15 * 60_000)).unwrap();
                        let time = (rng.below(15) != 0).then_some(clock - age);
                        let region = (rng.below(6) != 0).then(|| REGIONS[rng.below(3) as usize]);
                        let qty = (rng.below(5) != 0).then(|| rng.below(100) as i32);
                        (id, time, region, qty)
                    })
                    .collect();
                table.upsert(batch(&rows)).unwrap();
            }
            let found = read(&table.groups(groups).unwrap().to_batch().unwrap());
            assert_eq!(found, recount(&table), "seed {seed}, step {step}");
        }
    }
}

#[test]
fn a_window_drops_the_buckets_it_no_longer_keeps() {
    let mut table = LiveTable::new(schema(), "id").unwrap();
    table
        .set_window(Some(Window::duration("time", Duration::from_secs(120))))
        .unwrap();
    let spec = GroupSpec::new()
        .group_by_time("time", Duration::from_secs(60))
        .aggregate(Aggregate::count());
    let groups = table.add_groups(spec).unwrap();
    let starts = |table: &LiveTable| {
        let batch = table.groups(groups).unwrap().to_batch().unwrap();
        let starts = batch.column(0).as_primitive::<TimestampMillisecondType>();
        let counts = batch.column(1).as_primitive::<UInt64Type>();
        starts
            .values()
            .iter()
            .copied()
            .zip(counts.values().iter().copied())
            .collect::<Vec<_>>()
    };
    let rows = [(1, Some(1_000), None, None), (2, Some(61_000), None, None)];
    table.upsert(batch(&rows)).unwrap();
    assert_eq!(starts(&table), [(0, 1), (MINUTE, 1)]);
    table
        .upsert(batch(&[(3, Some(150_000), None, None)]))
        .unwrap();
    // 150 s keeps 30 s on: row 1 (at 1 s) is gone, and its minute with it.
    assert_eq!(starts(&table), [(MINUTE, 1), (2 * MINUTE, 1)]);
}

#[test]
fn bucket_starts_keep_the_column_type_and_name() {
    let mut table = LiveTable::new(schema(), "id").unwrap();
    let spec = GroupSpec::new()
        .group_by_time("time", Duration::from_secs(3_600))
        .aggregate(Aggregate::count());
    let groups = table.add_groups(spec).unwrap();
    let rows = [
        (1, Some(-1), None, None),
        (2, None, None, None),
        (3, Some(3_600_000 + 5), None, None),
    ];
    table.upsert(batch(&rows)).unwrap();
    let batch = table.groups(groups).unwrap().to_batch().unwrap();
    assert_eq!(
        batch.schema().field(0),
        &Field::new("time", schema().field(1).data_type().clone(), true)
    );
    let starts = batch.column(0).as_primitive::<TimestampMillisecondType>();
    assert_eq!(
        starts.iter().collect::<Vec<_>>(),
        [Some(-3_600_000), Some(3_600_000), None],
        "the hour before the epoch, the hour after it, then null times"
    );
}

/// One table of `id` and `value`, and the starts of `value`'s buckets of `spec` as a batch.
fn starts_of(values: ArrayRef, spec: GroupSpec) -> Result<ArrayRef, DataError> {
    let ids: Vec<i64> = (0..values.len() as i64).collect();
    let batch = RecordBatch::try_from_iter([
        ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
        ("value", values),
    ])
    .unwrap();
    let mut table = LiveTable::new(batch.schema(), "id")?;
    let groups = table.add_groups(spec.aggregate(Aggregate::count()))?;
    table.upsert(batch)?;
    let batch = table.groups(groups).unwrap().to_batch()?;
    Ok(Arc::clone(batch.column(0)))
}

fn steps(step: f64) -> GroupSpec {
    GroupSpec::new().group_by_step("value", step)
}

#[test]
fn dates_bucket_from_the_epoch() {
    let week = GroupSpec::new().group_by_time("value", Duration::from_secs(7 * 86_400));
    // Day 0 (1970-01-01), day 6, day 7 and the day before the epoch.
    let starts = starts_of(Arc::new(Date32Array::from(vec![0, 6, 7, -1])), week).unwrap();
    let starts = starts.as_primitive::<Date32Type>();
    assert_eq!(starts.values(), &[-7, 0, 7]);
}

#[test]
fn decimal_steps_are_in_the_columns_units() {
    let cents = Decimal128Array::from(vec![49, 50, 149, -1])
        .with_precision_and_scale(10, 2)
        .unwrap();
    // Half a unit: 50 cents.
    let starts = starts_of(Arc::new(cents), steps(0.5)).unwrap();
    assert_eq!(starts.data_type(), &DataType::Decimal128(10, 2));
    assert_eq!(
        starts.as_primitive::<Decimal128Type>().values(),
        &[-50, 0, 50, 100]
    );
    let cents = Decimal128Array::from(vec![29, 30])
        .with_precision_and_scale(10, 2)
        .unwrap();
    // 0.29 * 100 is 28.999999999999996: still 29 cents.
    let starts = starts_of(Arc::new(cents), steps(0.29)).unwrap();
    assert_eq!(starts.as_primitive::<Decimal128Type>().values(), &[29]);
}

#[test]
fn narrow_integers_keep_their_range() {
    let values = Int8Array::from(vec![-128, -1, 0, 127]);
    let starts = starts_of(Arc::new(values), steps(100.0)).unwrap();
    // The lowest bucket starts at -200, which an Int8 cannot hold.
    assert_eq!(
        starts.as_primitive::<Int8Type>().values(),
        &[-128, -100, 0, 100]
    );
    let values = UInt64Array::from(vec![u64::MAX, 3]);
    let starts = starts_of(Arc::new(values), steps(10.0)).unwrap();
    assert_eq!(
        starts.as_primitive::<UInt64Type>().values(),
        &[0, u64::MAX - 5]
    );
}

#[test]
fn float_buckets_put_nan_and_infinities_apart() {
    let values = Float64Array::from(vec![
        Some(0.3),
        Some(0.25),
        Some(f64::NAN),
        Some(f64::INFINITY),
        None,
        Some(-0.0),
        Some(f64::NEG_INFINITY),
    ]);
    let starts = starts_of(Arc::new(values), steps(0.1)).unwrap();
    let starts = starts.as_primitive::<Float64Type>();
    let values: Vec<Option<f64>> = starts.iter().collect();
    assert_eq!(
        values[..5],
        [
            Some(f64::NEG_INFINITY),
            Some(0.0),
            Some(0.2),
            Some(3.0 * 0.1),
            Some(f64::INFINITY)
        ]
    );
    assert!(values[5].is_some_and(f64::is_nan));
    assert_eq!(values[6], None);
    let values = Float32Array::from(vec![0.3f32, 0.7]);
    let starts = starts_of(Arc::new(values), steps(0.5)).unwrap();
    assert_eq!(starts.as_primitive::<Float32Type>().values(), &[0.0, 0.5]);
}

#[test]
fn widths_that_do_not_fit_are_refused() {
    let millis = || Arc::new(TimestampMillisecondArray::from(vec![0])) as ArrayRef;
    let ints = || Arc::new(Int64Array::from(vec![0])) as ArrayRef;
    let floats = || Arc::new(Float64Array::from(vec![0.0])) as ArrayRef;
    let time = |every| GroupSpec::new().group_by_time("value", every);
    let refused = [
        (millis(), time(Duration::ZERO)),
        (millis(), time(Duration::from_micros(999))),
        (millis(), steps(60_000.0)),
        (ints(), time(Duration::from_secs(1))),
        (ints(), steps(0.5)),
        (ints(), steps(-1.0)),
        (floats(), steps(0.0)),
        (floats(), steps(f64::NAN)),
        (floats(), steps(f64::INFINITY)),
        (Arc::new(StringArray::from(vec!["a"])), steps(1.0)),
        (Arc::new(BooleanArray::from(vec![true])), steps(1.0)),
    ];
    for (values, spec) in refused {
        let data_type = values.data_type().clone();
        let error = starts_of(values, spec.clone()).unwrap_err();
        assert_eq!(
            error,
            DataError::BucketWidth("value".into()),
            "{data_type} {spec:?}"
        );
    }
    let spec = GroupSpec::new().group_by_time("missing", Duration::from_secs(1));
    assert_eq!(
        starts_of(millis(), spec).unwrap_err(),
        DataError::UnknownColumn("missing".into())
    );
    let twice = GroupSpec::new()
        .group_by("value")
        .group_by_time("value", Duration::from_secs(1));
    assert_eq!(
        starts_of(millis(), twice).unwrap_err(),
        DataError::DuplicateName("value".into())
    );
}

#[test]
fn the_lowest_buckets_start_at_the_types_smallest_value() {
    let times = TimestampMillisecondArray::from(vec![i64::MIN, i64::MIN + MINUTE, 0]);
    let minute = GroupSpec::new().group_by_time("value", Duration::from_secs(60));
    let starts = starts_of(Arc::new(times), minute).unwrap();
    let starts = starts.as_primitive::<TimestampMillisecondType>();
    // i64::MIN's minute starts before i64::MIN; the next one starts within range.
    let next = (i64::MIN + MINUTE).div_euclid(MINUTE) * MINUTE;
    assert_eq!(starts.values(), &[i64::MIN, next, 0]);
    let decimals = Decimal128Array::from(vec![-999, 999])
        .with_precision_and_scale(3, 1)
        .unwrap();
    // Buckets of 10.0: -99.9 is in the one from -100.0, which a Decimal128(3, 1) cannot hold.
    let starts = starts_of(Arc::new(decimals), steps(10.0)).unwrap();
    assert_eq!(starts.data_type(), &DataType::Decimal128(3, 1));
    assert_eq!(
        starts.as_primitive::<Decimal128Type>().values(),
        &[-999, 900]
    );
}

#[test]
fn narrow_float_columns_forgive_their_own_rounding() {
    let values = Float32Array::from(vec![0.7f32, 0.69]);
    let starts = starts_of(Arc::new(values), steps(0.1)).unwrap();
    // 0.7f32 is just below 0.7: still in the bucket from 0.7 (as an f32).
    assert_eq!(
        starts.as_primitive::<Float32Type>().values(),
        &[0.6f32, 0.7]
    );
    let values = Float16Array::from(vec![f16::from_f32(0.9), f16::from_f32(-0.25)]);
    let starts = starts_of(Arc::new(values), steps(0.1)).unwrap();
    let starts = starts.as_primitive::<Float16Type>();
    assert_eq!(starts.values(), &[f16::from_f64(-0.3), f16::from_f64(0.9)]);
}
