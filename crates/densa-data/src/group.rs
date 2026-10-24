//! Grouped aggregates: count, sum, min, max and mean per group of key values, kept up to date
//! as rows change, the same way views are.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use arrow_array::cast::AsArray;
use arrow_array::types::{Float16Type, Float32Type, Float64Type};
use arrow_array::{
    new_null_array, Array, ArrayRef, ArrowPrimitiveType, Decimal128Array, Float64Array,
    RecordBatch, RecordBatchOptions, UInt64Array,
};
use arrow_schema::{DataType, Field, Schema};
use arrow_select::interleave::interleave;

use crate::bucket::{float_starts, whole_starts, Bucket};
use crate::dictionary::dictionary_index;
use crate::filter::{resolve, Filter, Predicate};
use crate::sorted::{Entry, SortedRows};
use crate::value::{canonical, prefix, unscale, Kind, Value, ValueFn};
use crate::view::{entry, entry_order, Rows, SortColumn};
use crate::window::Span;
use crate::{DataError, RowId, RowRef};

/// The largest magnitude a `Decimal128` of precision 38 holds, plus one.
const DECIMAL_LIMIT: u128 = 10u128.pow(38);

/// 2^`exponent`, for the exponent of a normal float.
const fn power_of_two(exponent: i32) -> f64 {
    f64::from_bits(((1023 + exponent) as u64) << 52)
}

/// Floats at least this large are summed apart, scaled by [`UNSCALE`], so no running sum can
/// overflow: 2^32 values (a table's most) below 2^960 sum below 2^992, and scaled values,
/// below 2^896, sum below 2^928.
const HUGE: f64 = power_of_two(960);
const UNSCALE: f64 = power_of_two(-128);
const SCALE: f64 = power_of_two(128);

/// What an aggregate computes, and over which column.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Function {
    Count,
    Sum(String),
    Min(String),
    Max(String),
    Mean(String),
}

/// One aggregate column of a [`GroupSpec`]: what it computes, over which column, and its name
/// in the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aggregate {
    function: Function,
    name: Option<String>,
}

impl Aggregate {
    /// Rows in the group. Named `count` unless [`named`](Self::named).
    #[must_use]
    pub fn count() -> Self {
        Self::of(Function::Count)
    }

    /// Sum of an integer, decimal or float column's non-null values; null when there are none.
    /// Integers and decimals sum exactly (see [`Groups::to_batch`] for the result types).
    /// Floats sum with compensation for rounding, so the error stays near that of one
    /// rounding of the largest partial sum the group has held; the sum is NaN when a value is
    /// NaN or both infinities occur, and infinite when one does. Named `sum(column)`.
    #[must_use]
    pub fn sum(column: impl Into<String>) -> Self {
        Self::of(Function::Sum(column.into()))
    }

    /// Smallest non-null value of any column views can sort, in their order: `-0.0` equals
    /// `0.0`, NaN sorts after infinity. Named `min(column)`.
    #[must_use]
    pub fn min(column: impl Into<String>) -> Self {
        Self::of(Function::Min(column.into()))
    }

    /// Largest non-null value of any column views can sort, in their order: NaN if the
    /// column holds one. Named `max(column)`.
    #[must_use]
    pub fn max(column: impl Into<String>) -> Self {
        Self::of(Function::Max(column.into()))
    }

    /// Mean of an integer, decimal or float column's non-null values, as a float (NaN and
    /// infinities as for [`sum`](Self::sum)). Named `mean(column)`.
    #[must_use]
    pub fn mean(column: impl Into<String>) -> Self {
        Self::of(Function::Mean(column.into()))
    }

    /// The aggregate, named `name` in the result.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    fn of(function: Function) -> Self {
        Self {
            function,
            name: None,
        }
    }

    fn name(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        let (function, column) = match &self.function {
            Function::Count => return "count".to_owned(),
            Function::Sum(column) => ("sum", column),
            Function::Min(column) => ("min", column),
            Function::Max(column) => ("max", column),
            Function::Mean(column) => ("mean", column),
        };
        format!("{function}({column})")
    }
}

/// Groups of rows with equal key values, and aggregates over each group.
///
/// ```
/// use std::sync::Arc;
/// use arrow_array::{ArrayRef, Decimal128Array, Int64Array, RecordBatch, StringArray};
/// use densa_data::{Aggregate, GroupSpec, LiveTable};
///
/// let trades = |ids: Vec<i64>, desks: Vec<&str>, qty: Vec<i64>| {
///     RecordBatch::try_from_iter([
///         ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
///         ("desk", Arc::new(StringArray::from(desks)) as ArrayRef),
///         ("qty", Arc::new(Int64Array::from(qty)) as ArrayRef),
///     ])
/// };
/// let mut table = LiveTable::new(trades(vec![], vec![], vec![])?.schema(), "id")?;
/// let by_desk = table.add_groups(
///     GroupSpec::new()
///         .group_by("desk")
///         .aggregate(Aggregate::count())
///         .aggregate(Aggregate::sum("qty")),
/// )?;
/// table.upsert(trades(vec![1, 2, 3], vec!["fx", "rates", "fx"], vec![5, 7, 10])?)?;
/// table.upsert(trades(vec![3], vec!["rates"], vec![10])?)?; // trade 3 moves desk
///
/// let totals = table.groups(by_desk).unwrap().to_batch()?; // fx: 1 trade, rates: 2
/// let qty = totals.column(2).as_any().downcast_ref::<Decimal128Array>().unwrap();
/// assert_eq!((qty.value(0), qty.value(1)), (5, 17));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GroupSpec {
    keys: Vec<KeySpec>,
    aggregates: Vec<Aggregate>,
    filter: Option<Filter>,
}

impl GroupSpec {
    /// No keys and no aggregates yet. Without keys there is exactly one group, of every
    /// counted row, even when there are none.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Groups by `column` too, after the key columns given before it. Groups are listed by
    /// key in the order views sort: ascending, nulls last. `-0.0` and `0.0` are one key, and
    /// so are all NaNs.
    #[must_use]
    pub fn group_by(self, column: impl Into<String>) -> Self {
        self.key(column, None)
    }

    /// Groups by buckets of `every` (a minute, an hour) of `column`, a column of timestamps,
    /// dates, times of day or durations, as a key after the ones added before it.
    ///
    /// The buckets are back to back from the column's zero (midnight UTC for timestamps,
    /// whatever their time zone), so a bucket of a minute starts on a whole minute; each value
    /// is in the bucket that starts at or before it, less than `every` earlier. Buckets have
    /// one length: a week of dates starts on a Thursday, like 1970-01-01, and there are no
    /// months. The key column holds each bucket's start, in the column's type and under its
    /// name, so a column is a key once (a second key of it is [`DataError::DuplicateName`]).
    /// The lowest bucket of a type's range, which may start before it, starts at the type's
    /// smallest value. `every` is taken in whole units of the column, rounding down, and must
    /// be at least one ([`DataError::BucketWidth`] otherwise). Null times make a group of
    /// their own, last.
    ///
    /// With a [`Window`](crate::Window) on the same column, the oldest bucket holds only the
    /// rows the window still keeps, and goes once they expire.
    ///
    /// ```
    /// use std::sync::Arc;
    /// use std::time::Duration;
    /// use arrow_array::{ArrayRef, Int64Array, RecordBatch, TimestampSecondArray, UInt64Array};
    /// use densa_data::{Aggregate, GroupSpec, LiveTable};
    ///
    /// let requests = |ids: Vec<i64>, times: Vec<i64>| {
    ///     RecordBatch::try_from_iter([
    ///         ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
    ///         ("time", Arc::new(TimestampSecondArray::from(times)) as ArrayRef),
    ///     ])
    /// };
    /// let mut table = LiveTable::new(requests(vec![], vec![])?.schema(), "id")?;
    /// let per_minute = table.add_groups(
    ///     GroupSpec::new()
    ///         .group_by_time("time", Duration::from_secs(60))
    ///         .aggregate(Aggregate::count()),
    /// )?;
    /// table.upsert(requests(vec![1, 2, 3], vec![5, 59, 61])?)?;
    ///
    /// let counts = table.groups(per_minute).unwrap().to_batch()?;
    /// let starts = counts.column(0).as_any().downcast_ref::<TimestampSecondArray>().unwrap();
    /// let count = counts.column(1).as_any().downcast_ref::<UInt64Array>().unwrap();
    /// assert_eq!(starts.values(), &[0, 60]); // 00:00 and 00:01
    /// assert_eq!(count.values(), &[2, 1]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn group_by_time(self, column: impl Into<String>, every: Duration) -> Self {
        self.key(column, Some(Span::Duration(every)))
    }

    /// Groups by buckets of `step` of `column`, a column of integers, decimals or floats, as a
    /// key after the ones added before it. `step` is in the column's own units: `0.5` is
    /// fifty cents of a decimal of dollars.
    ///
    /// As with [`group_by_time`](Self::group_by_time), the buckets are back to back from
    /// zero, each value is in the one that starts at or before it, and the key column holds
    /// each bucket's start (the type's smallest or largest value for a bucket that starts
    /// past the type's range). For integers and decimals `step` is taken in whole units,
    /// rounding down, but a step within a billionth of its size of a whole number counts as
    /// it (`0.29` is 29 cents, though `0.29 * 100.0` is just below 29); it must be at least one
    /// unit, or above zero and finite for floats ([`DataError::BucketWidth`] otherwise).
    ///
    /// Float buckets forgive the rounding of the column's type: a value at most two roundings
    /// below a bucket's start, and never more than a sixteenth of a step, is in that bucket.
    /// `0.3` is in the bucket of `0.1` steps that starts at `0.3` (`3.0 * 0.1`, itself just
    /// above `0.3`), though `0.3 / 0.1` is just below 3. NaN, the infinities and nulls are
    /// groups of their own.
    #[must_use]
    pub fn group_by_step(self, column: impl Into<String>, step: f64) -> Self {
        self.key(column, Some(Span::Value(step)))
    }

    fn key(mut self, column: impl Into<String>, bucket: Option<Span>) -> Self {
        self.keys.push(KeySpec {
            column: column.into(),
            bucket,
        });
        self
    }

    /// Adds an aggregate column. Every key and aggregate needs its own name.
    #[must_use]
    pub fn aggregate(mut self, aggregate: Aggregate) -> Self {
        self.aggregates.push(aggregate);
        self
    }

    /// Counts only rows for which `filter` is true.
    #[must_use]
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filter = Some(filter);
        self
    }
}

/// A key column of a [`GroupSpec`], and its bucket width if it has one.
#[derive(Debug, Clone, PartialEq)]
struct KeySpec {
    column: String,
    bucket: Option<Span>,
}

/// One value of a key, ordered like [`crate::value::compare`] ascending, nulls last (the
/// variant order).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    Bool(bool),
    Int(i64),
    UInt(u64),
    Decimal(i128),
    /// A float's IEEE total-order bits (canonical NaN and zero).
    Float(u64),
    Str(Box<str>),
    Null,
}

impl Part {
    fn of(value: Value<'_>) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(v) => Self::Bool(v),
            Value::Int(v) => Self::Int(v),
            Value::UInt(v) => Self::UInt(v),
            Value::Decimal(v) => Self::Decimal(v),
            Value::Float(_) => Self::Float(prefix(value, false)),
            Value::Str(v) => Self::Str(v.into()),
        }
    }
}

/// The float whose [`Part::Float`] bits are `bits`.
fn from_prefix(bits: u64) -> f64 {
    const SIGN: u64 = 1 << 63;
    f64::from_bits(if bits & SIGN != 0 {
        bits & !SIGN
    } else {
        !bits
    })
}

/// A column the groups read: its index, reader, kind, data type and name.
#[derive(Debug, Clone)]
struct Source {
    column: usize,
    read: ValueFn,
    kind: Kind,
    data_type: DataType,
    name: String,
}

impl Source {
    fn new(schema: &Schema, name: &str) -> Result<Self, DataError> {
        let (column, kind, read) = resolve(schema, name)?;
        Ok(Self {
            column,
            read,
            kind,
            data_type: schema.field(column).data_type().clone(),
            name: name.to_owned(),
        })
    }

    fn read<'a>(&self, batches: &'a [RecordBatch], at: RowRef) -> Value<'a> {
        (self.read)(batches[at.batch].column(self.column).as_ref(), at.row)
    }

    /// The type its values come out as: a dictionary's value type (a dictionary's key type
    /// may not have room for every group's value), otherwise its own.
    fn output_type(&self) -> &DataType {
        match &self.data_type {
            DataType::Dictionary(_, values) => values,
            other => other,
        }
    }
}

/// A key column the groups read, and the buckets its values go in, if any.
#[derive(Debug, Clone)]
struct Key {
    source: Source,
    bucket: Option<Bucket>,
}

impl Key {
    fn new(schema: &Schema, spec: &KeySpec) -> Result<Self, DataError> {
        let source = Source::new(schema, &spec.column)?;
        let bucket = spec
            .bucket
            .map(|width| {
                Bucket::new(width, &source.data_type, source.kind)
                    .ok_or_else(|| DataError::BucketWidth(spec.column.clone()))
            })
            .transpose()?;
        Ok(Self { source, bucket })
    }

    fn part(&self, batches: &[RecordBatch], at: RowRef) -> Part {
        let value = self.source.read(batches, at);
        Part::of(match &self.bucket {
            Some(bucket) => bucket.start(value),
            None => value,
        })
    }
}

/// The column types sums and means read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Numeric {
    Integer,
    Decimal { scale: i8 },
    Float,
}

impl Numeric {
    fn of(data_type: &DataType) -> Option<Self> {
        Some(match data_type {
            DataType::Decimal128(_, scale) => Self::Decimal { scale: *scale },
            t if t.is_integer() => Self::Integer,
            t if t.is_floating() => Self::Float,
            _ => return None,
        })
    }
}

/// What the groups keep about a column, shared by the aggregates that need it: a running sum
/// (for sum and mean) or its values in order (for min and max).
#[derive(Debug, Clone)]
enum Track {
    Sum(Numeric),
    Order(SortColumn),
}

#[derive(Debug, Clone)]
struct Tracker {
    source: Source,
    track: Track,
}

impl Tracker {
    /// Adds (`sign` 1) or removes (`sign` -1) the value of row `id`, at `at`, to `state`.
    fn apply(&self, state: &mut State, rows: Rows<'_>, id: RowId, at: RowRef, sign: i8) {
        match (&self.track, state, self.source.read(rows.batches, at)) {
            (_, _, Value::Null) => {}
            (Track::Order(column), State::Ordered(sorted), _) => {
                let sort = std::slice::from_ref(column);
                let probe = entry(sort, rows.batches, id, at);
                let order = |other: Entry| entry_order(sort, rows, other, probe, at);
                if sign > 0 {
                    sorted.insert(probe, order);
                } else {
                    let found = sorted.remove(id, order);
                    debug_assert!(found, "a counted value is among the sorted values");
                }
            }
            (_, State::Exact(sum), Value::Int(v)) => sum.apply(i128::from(v), sign),
            (_, State::Exact(sum), Value::UInt(v)) => sum.apply(i128::from(v), sign),
            (_, State::Exact(sum), Value::Decimal(v)) => sum.apply(v, sign),
            (_, State::Float(sum), Value::Float(v)) => sum.apply(v, sign),
            _ => debug_assert!(false, "a tracker's state does not fit its column"),
        }
    }
}

/// `count` plus or minus one.
fn step(count: u64, sign: i8) -> u64 {
    if sign > 0 {
        count + 1
    } else {
        count - 1
    }
}

/// An exact sum of integers or decimals, `sum + carry * 2^128`: adding and removing a value
/// are exact inverses, even while the sum is beyond `i128`.
#[derive(Debug, Clone, Copy, Default)]
struct ExactSum {
    sum: i128,
    carry: i64,
    values: u64,
}

impl ExactSum {
    fn apply(&mut self, value: i128, sign: i8) {
        let (sum, wrapped) = if sign > 0 {
            self.sum.overflowing_add(value)
        } else {
            self.sum.overflowing_sub(value)
        };
        if wrapped {
            // Past `i128::MAX` if the change was positive, past `i128::MIN` if negative.
            self.carry += if (sign > 0) == (value > 0) { 1 } else { -1 };
        }
        self.sum = sum;
        self.values = step(self.values, sign);
    }

    /// The sum, if there are values and it fits `i128`.
    fn total(&self) -> Option<i128> {
        (self.values > 0 && self.carry == 0).then_some(self.sum)
    }

    fn mean(&self) -> Option<f64> {
        // Lossy by design: a mean is a float. 2^128 is exact in f64.
        let total = self.sum as f64 + self.carry as f64 * SCALE;
        (self.values > 0).then(|| total / self.values as f64)
    }
}

/// A compensated (Neumaier) float sum and how many values are in it.
#[derive(Debug, Clone, Copy, Default)]
struct Compensated {
    sum: f64,
    compensation: f64,
    values: u64,
}

impl Compensated {
    fn apply(&mut self, value: f64, sign: i8) {
        self.values = step(self.values, sign);
        if self.values == 0 {
            // Nothing left: drop the rounding the removed values leave behind.
            *self = Self::default();
            return;
        }
        let value = if sign > 0 { value } else { -value };
        let total = self.sum + value;
        if self.sum.abs() >= value.abs() {
            self.compensation += (self.sum - total) + value;
        } else {
            self.compensation += (value - total) + self.sum;
        }
        self.sum = total;
    }

    fn total(&self) -> f64 {
        self.sum + self.compensation
    }
}

/// A float sum that NaN and infinities cannot poison: they are counted, not added, so the sum
/// recovers when their rows change. Huge values are summed apart (see [`HUGE`]).
#[derive(Debug, Clone, Copy, Default)]
struct FloatSum {
    normal: Compensated,
    /// Values of at least [`HUGE`], times [`UNSCALE`].
    huge: Compensated,
    nan: u64,
    infinity: u64,
    negative_infinity: u64,
}

impl FloatSum {
    fn apply(&mut self, value: f64, sign: i8) {
        if value.is_nan() {
            self.nan = step(self.nan, sign);
        } else if value == f64::INFINITY {
            self.infinity = step(self.infinity, sign);
        } else if value == f64::NEG_INFINITY {
            self.negative_infinity = step(self.negative_infinity, sign);
        } else if value.abs() >= HUGE {
            self.huge.apply(value * UNSCALE, sign);
        } else {
            self.normal.apply(value, sign);
        }
    }

    fn values(&self) -> u64 {
        self.normal.values + self.huge.values + self.nan + self.infinity + self.negative_infinity
    }

    fn sum(&self) -> Option<f64> {
        (self.values() > 0).then(|| self.divided(1.0))
    }

    fn mean(&self) -> Option<f64> {
        let values = self.values();
        (values > 0).then(|| self.divided(values as f64))
    }

    /// The sum divided by `divisor`, overflowing only if the result itself is beyond `f64`.
    fn divided(&self, divisor: f64) -> f64 {
        if self.nan > 0 || (self.infinity > 0 && self.negative_infinity > 0) {
            return f64::NAN;
        }
        if self.infinity > 0 {
            return f64::INFINITY;
        }
        if self.negative_infinity > 0 {
            return f64::NEG_INFINITY;
        }
        let normal = self.normal.total() / divisor;
        let huge = self.huge.total() / divisor;
        if huge == 0.0 {
            normal
        } else {
            (huge + normal * UNSCALE) * SCALE
        }
    }
}

/// Running state of one tracker in one group.
#[derive(Debug, Clone)]
enum State {
    Exact(ExactSum),
    Float(FloatSum),
    /// Non-null values in order: min first, max last.
    Ordered(SortedRows),
}

impl State {
    fn new(track: &Track) -> Self {
        match track {
            Track::Sum(Numeric::Float) => Self::Float(FloatSum::default()),
            Track::Sum(_) => Self::Exact(ExactSum::default()),
            Track::Order(_) => Self::Ordered(SortedRows::default()),
        }
    }

    fn exact(&self) -> Option<&ExactSum> {
        match self {
            Self::Exact(sum) => Some(sum),
            _ => None,
        }
    }

    fn float(&self) -> Option<&FloatSum> {
        match self {
            Self::Float(sum) => Some(sum),
            _ => None,
        }
    }

    /// The row of the smallest value, or of the largest if `max`.
    fn extreme(&self, max: bool) -> Option<RowId> {
        match self {
            Self::Ordered(sorted) if max => sorted.last(),
            Self::Ordered(sorted) => sorted.first(),
            _ => None,
        }
    }
}

/// What an aggregate column shows; the numbers index `GroupState::trackers`.
#[derive(Debug, Clone, Copy)]
enum Output {
    Count,
    Sum(usize, Numeric),
    Mean(usize, Numeric),
    Min(usize),
    Max(usize),
}

/// A compiled aggregate.
#[derive(Debug, Clone)]
struct Compiled {
    output: Output,
    name: String,
}

/// One group: its rows (any of them gives the key values) and a state per tracker.
#[derive(Debug, Clone)]
struct Group {
    rows: BTreeSet<RowId>,
    states: Vec<State>,
}

impl Group {
    fn new(trackers: &[Tracker]) -> Self {
        Self {
            rows: BTreeSet::new(),
            states: trackers.iter().map(|t| State::new(&t.track)).collect(),
        }
    }
}

/// A group spec's compiled form and its groups.
#[derive(Debug, Clone)]
pub(crate) struct GroupState {
    keys: Vec<Key>,
    trackers: Vec<Tracker>,
    aggregates: Vec<Compiled>,
    filter: Option<Predicate>,
    groups: BTreeMap<Vec<Part>, Group>,
    /// Whether each row id is counted.
    member: Vec<bool>,
    revision: u64,
}

impl GroupState {
    pub(crate) fn compile(spec: &GroupSpec, schema: &Schema) -> Result<Self, DataError> {
        let keys = spec
            .keys
            .iter()
            .map(|k| Key::new(schema, k))
            .collect::<Result<Vec<_>, _>>()?;
        let mut trackers = Vec::new();
        let aggregates = spec
            .aggregates
            .iter()
            .map(|aggregate| {
                let output = match &aggregate.function {
                    Function::Count => Output::Count,
                    Function::Sum(column) => {
                        let (index, numeric) = sum_tracker(&mut trackers, schema, column)?;
                        Output::Sum(index, numeric)
                    }
                    Function::Mean(column) => {
                        let (index, numeric) = sum_tracker(&mut trackers, schema, column)?;
                        Output::Mean(index, numeric)
                    }
                    Function::Min(column) => {
                        Output::Min(order_tracker(&mut trackers, schema, column)?)
                    }
                    Function::Max(column) => {
                        Output::Max(order_tracker(&mut trackers, schema, column)?)
                    }
                };
                Ok(Compiled {
                    output,
                    name: aggregate.name(),
                })
            })
            .collect::<Result<Vec<_>, DataError>>()?;
        let mut names = BTreeSet::new();
        let all_names = keys
            .iter()
            .map(|k| &k.source.name)
            .chain(aggregates.iter().map(|a| &a.name));
        for name in all_names {
            if !names.insert(name) {
                return Err(DataError::DuplicateName(name.clone()));
            }
        }
        let filter = spec
            .filter
            .as_ref()
            .map(|f| Predicate::compile(f, schema))
            .transpose()?;
        let mut state = Self {
            keys,
            trackers,
            aggregates,
            filter,
            groups: BTreeMap::new(),
            member: Vec::new(),
            revision: 0,
        };
        state.clear();
        Ok(state)
    }

    /// No counted rows: no groups, or the one group of a spec without keys, empty.
    fn clear(&mut self) {
        self.groups.clear();
        self.member.clear();
        if self.keys.is_empty() {
            self.groups.insert(Vec::new(), Group::new(&self.trackers));
        }
    }

    fn passes(&self, batches: &[RecordBatch], at: RowRef) -> bool {
        self.filter
            .as_ref()
            .is_none_or(|f| f.eval(&batches[at.batch], at.row) == Some(true))
    }

    fn key(&self, batches: &[RecordBatch], at: RowRef) -> Vec<Part> {
        self.keys.iter().map(|k| k.part(batches, at)).collect()
    }

    /// Counts row `id` (values at `at`) if it passes the filter, and says whether it did.
    /// `sorted` false leaves min and max values out, for [`sort_values`](Self::sort_values).
    fn count(&mut self, rows: Rows<'_>, id: RowId, at: RowRef, sorted: bool) -> bool {
        if !self.passes(rows.batches, at) {
            return false;
        }
        let key = self.key(rows.batches, at);
        let trackers = &self.trackers;
        let group = self
            .groups
            .entry(key)
            .or_insert_with(|| Group::new(trackers));
        group.rows.insert(id);
        for (tracker, state) in trackers.iter().zip(&mut group.states) {
            if sorted || !matches!(tracker.track, Track::Order(_)) {
                tracker.apply(state, rows, id, at, 1);
            }
        }
        if self.member.len() <= id as usize {
            self.member.resize(id as usize + 1, false);
        }
        self.member[id as usize] = true;
        true
    }

    /// Counts row `id` (values at `at`) if it passes the filter.
    pub(crate) fn insert(&mut self, rows: Rows<'_>, id: RowId, at: RowRef) {
        if self.count(rows, id, at, true) {
            self.revision += 1;
        }
    }

    /// Uncounts row `id`, whose values are at `at` (where they were when it was counted).
    pub(crate) fn remove(&mut self, rows: Rows<'_>, id: RowId, at: RowRef) {
        if !self.member.get(id as usize).copied().unwrap_or(false) {
            return;
        }
        let key = self.key(rows.batches, at);
        let Some(group) = self.groups.get_mut(&key) else {
            debug_assert!(false, "a counted row's group exists");
            return;
        };
        group.rows.remove(&id);
        for (tracker, state) in self.trackers.iter().zip(&mut group.states) {
            tracker.apply(state, rows, id, at, -1);
        }
        // Without keys, the one group stays, empty.
        if group.rows.is_empty() && !self.keys.is_empty() {
            self.groups.remove(&key);
        }
        self.member[id as usize] = false;
        self.revision += 1;
    }

    /// Recounts every live row.
    pub(crate) fn rebuild(&mut self, rows: Rows<'_>) {
        let had_rows = self.groups.values().any(|g| !g.rows.is_empty());
        self.clear();
        let mut counted = false;
        for (id, slot) in rows.slots.iter().enumerate() {
            if let (Ok(id), Some(slot)) = (RowId::try_from(id), slot) {
                counted |= self.count(rows, id, slot.row_ref(), false);
            }
        }
        self.sort_values(rows);
        if had_rows || counted {
            self.revision += 1;
        }
    }

    /// Fills each group's min and max values from its rows, sorting them at once rather than
    /// inserting them one by one.
    fn sort_values(&mut self, rows: Rows<'_>) {
        for (index, tracker) in self.trackers.iter().enumerate() {
            let Track::Order(column) = &tracker.track else {
                continue;
            };
            let sort = std::slice::from_ref(column);
            for group in self.groups.values_mut() {
                let mut entries: Vec<Entry> = group
                    .rows
                    .iter()
                    .map(|&id| (id, rows.at(id)))
                    .filter(|&(_, at)| tracker.source.read(rows.batches, at) != Value::Null)
                    .map(|(id, at)| entry(sort, rows.batches, id, at))
                    .collect();
                entries.sort_unstable_by(|&a, &b| entry_order(sort, rows, a, b, rows.at(b.id)));
                if let State::Ordered(sorted) = &mut group.states[index] {
                    sorted.rebuild(&entries);
                }
            }
        }
    }
}

/// The tracker index for `column`'s sum, and the column's numeric type.
fn sum_tracker(
    trackers: &mut Vec<Tracker>,
    schema: &Schema,
    column: &str,
) -> Result<(usize, Numeric), DataError> {
    let source = Source::new(schema, column)?;
    let numeric =
        Numeric::of(&source.data_type).ok_or_else(|| DataError::NotNumeric(column.to_owned()))?;
    Ok((tracker(trackers, source, Track::Sum(numeric)), numeric))
}

/// The tracker index for `column`'s values in order.
fn order_tracker(
    trackers: &mut Vec<Tracker>,
    schema: &Schema,
    column: &str,
) -> Result<usize, DataError> {
    let source = Source::new(schema, column)?;
    let sort = SortColumn::ascending(source.column, source.read);
    Ok(tracker(trackers, source, Track::Order(sort)))
}

/// The index of the tracker of `source` that keeps `track`, added if no aggregate needed it
/// yet.
fn tracker(trackers: &mut Vec<Tracker>, source: Source, track: Track) -> usize {
    let same = |t: &Tracker| {
        t.source.column == source.column
            && std::mem::discriminant(&t.track) == std::mem::discriminant(&track)
    };
    if let Some(index) = trackers.iter().position(same) {
        return index;
    }
    trackers.push(Tracker { source, track });
    trackers.len() - 1
}

/// The groups of a [`GroupSpec`], ordered by key. Cheap to get.
#[derive(Debug, Clone, Copy)]
pub struct Groups<'a> {
    pub(crate) state: &'a GroupState,
    pub(crate) rows: Rows<'a>,
}

impl Groups<'_> {
    /// Number of groups: the rows of [`to_batch`](Self::to_batch). A spec without keys always
    /// has one.
    pub fn len(&self) -> usize {
        self.state.groups.len()
    }

    /// Whether there are no groups: the spec has keys and no row is counted.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Increases whenever a group changes (and sometimes when a bulk change recounts the
    /// same groups): rebuild a display of [`to_batch`](Self::to_batch) only when it moved.
    pub fn revision(&self) -> u64 {
        self.state.revision
    }

    /// The groups as one record batch, one row per group in key order: the key columns,
    /// then each aggregate. A bucketed key holds each bucket's start. Keys, min and max have
    /// the column's type, except that dictionary
    /// columns come out decoded (as their value type) and floats canonical (`0.0` for `-0.0`,
    /// one NaN). Counts are `UInt64`, never null. Sums of integers are `Decimal128(38, 0)`,
    /// which holds any sum a table can produce; sums of decimals `Decimal128(38, scale)`,
    /// null if the exact sum needs more than 38 digits; sums of floats `Float64`. Means are
    /// `Float64`. Any other aggregate over no values is null. Allocates.
    ///
    /// # Errors
    /// [`DataError::Arrow`] if Arrow cannot assemble a column (over 2 GiB of strings).
    pub fn to_batch(&self) -> Result<RecordBatch, DataError> {
        let groups: Vec<&Group> = self.state.groups.values().collect();
        let mut fields = Vec::new();
        let mut columns = Vec::new();
        for (index, key) in self.state.keys.iter().enumerate() {
            let source = &key.source;
            columns.push(match key.bucket {
                Some(_) => self.starts(index, source),
                None => {
                    let rows = groups.iter().map(|g| g.rows.first().copied());
                    self.gather(source, rows)?
                }
            });
            fields.push(Field::new(&source.name, source.output_type().clone(), true));
        }
        for aggregate in &self.state.aggregates {
            let (column, data_type) = self.column(aggregate.output, &groups)?;
            let nullable = !matches!(aggregate.output, Output::Count);
            fields.push(Field::new(&aggregate.name, data_type, nullable));
            columns.push(column);
        }
        let schema = Arc::new(Schema::new(fields));
        let options = RecordBatchOptions::new().with_row_count(Some(groups.len()));
        RecordBatch::try_new_with_options(schema, columns, &options).map_err(DataError::arrow)
    }

    fn column(&self, output: Output, groups: &[&Group]) -> Result<(ArrayRef, DataType), DataError> {
        Ok(match output {
            Output::Count => {
                // Lossless: a group has at most 2^32 rows.
                let counts: UInt64Array =
                    groups.iter().map(|g| Some(g.rows.len() as u64)).collect();
                (Arc::new(counts), DataType::UInt64)
            }
            Output::Sum(index, numeric) => sums(numeric, groups.iter().map(|g| &g.states[index]))?,
            Output::Mean(index, numeric) => {
                let means: Float64Array = groups
                    .iter()
                    .map(|g| mean(&g.states[index], numeric))
                    .collect();
                (Arc::new(means), DataType::Float64)
            }
            Output::Min(index) | Output::Max(index) => {
                let max = matches!(output, Output::Max(_));
                let rows = groups.iter().map(|g| g.states[index].extreme(max));
                let source = &self.state.trackers[index].source;
                (self.gather(source, rows)?, source.output_type().clone())
            }
        })
    }

    /// The bucket starts of key `index`, over `source`, one per group.
    fn starts(&self, index: usize, source: &Source) -> ArrayRef {
        let parts = self.state.groups.keys().map(|key| &key[index]);
        if source.kind == Kind::Float {
            let starts: Vec<Option<f64>> = parts
                .map(|part| match part {
                    Part::Float(bits) => Some(from_prefix(*bits)),
                    _ => None,
                })
                .collect();
            return float_starts(&source.data_type, &starts);
        }
        let starts: Vec<Option<i128>> = parts
            .map(|part| match part {
                Part::Decimal(start) => Some(*start),
                _ => None,
            })
            .collect();
        whole_starts(&source.data_type, &starts)
    }

    /// `source`'s values at rows `ids` (`None`: null), dictionaries decoded and floats
    /// canonical.
    fn gather(
        &self,
        source: &Source,
        ids: impl Iterator<Item = Option<RowId>>,
    ) -> Result<ArrayRef, DataError> {
        let batches = self.rows.batches;
        let column = |batch: usize| batches[batch].column(source.column).as_ref();
        let decode = matches!(source.data_type, DataType::Dictionary(..));
        let null = new_null_array(source.output_type(), 1);
        let mut arrays: Vec<&dyn Array> = (0..batches.len())
            .map(|batch| {
                let array = column(batch);
                match array.as_any_dictionary_opt() {
                    Some(dictionary) if decode => dictionary.values().as_ref(),
                    _ => array,
                }
            })
            .collect();
        arrays.push(null.as_ref());
        let null_at = (arrays.len() - 1, 0);
        let indices: Vec<(usize, usize)> = ids
            .map(|id| {
                let place = id.and_then(|id| {
                    let at = self.rows.at(id);
                    let row = if decode {
                        dictionary_index(column(at.batch), at.row)?
                    } else {
                        at.row
                    };
                    Some((at.batch, row))
                });
                place.unwrap_or(null_at)
            })
            .collect();
        let array = interleave(&arrays, &indices).map_err(DataError::arrow)?;
        Ok(if source.kind == Kind::Float {
            canonical_floats(&array)
        } else {
            array
        })
    }
}

/// `array` with canonical floats: `0.0` for `-0.0`, one NaN.
fn canonical_floats(array: &ArrayRef) -> ArrayRef {
    type F16 = <Float16Type as ArrowPrimitiveType>::Native;
    if let Some(a) = array.as_primitive_opt::<Float64Type>() {
        Arc::new(a.unary::<_, Float64Type>(canonical))
    } else if let Some(a) = array.as_primitive_opt::<Float32Type>() {
        // Lossless: an f32 converts to f64 and back exactly.
        Arc::new(a.unary::<_, Float32Type>(|v| canonical(f64::from(v)) as f32))
    } else if let Some(a) = array.as_primitive_opt::<Float16Type>() {
        Arc::new(a.unary::<_, Float16Type>(|v| F16::from_f64(canonical(v.into()))))
    } else {
        Arc::clone(array)
    }
}

fn mean(state: &State, numeric: Numeric) -> Option<f64> {
    match (state, numeric) {
        (State::Float(sum), _) => sum.mean(),
        (State::Exact(sum), Numeric::Decimal { scale }) => sum.mean().map(|m| unscale(m, scale)),
        (State::Exact(sum), _) => sum.mean(),
        (State::Ordered(_), _) => None,
    }
}

/// The sums of `numeric` values, one per state, and their type.
fn sums<'g>(
    numeric: Numeric,
    states: impl Iterator<Item = &'g State>,
) -> Result<(ArrayRef, DataType), DataError> {
    let scale = match numeric {
        Numeric::Float => {
            let sums: Float64Array = states.map(|s| s.float().and_then(FloatSum::sum)).collect();
            return Ok((Arc::new(sums), DataType::Float64));
        }
        // Integer sums always fit: 2^32 values of at most 2^64 stay below 2^96 < 10^38.
        Numeric::Integer => 0,
        Numeric::Decimal { scale } => scale,
    };
    let sums: Decimal128Array = states
        .map(|s| {
            let sum = s.exact().and_then(ExactSum::total);
            sum.filter(|v| v.unsigned_abs() < DECIMAL_LIMIT)
        })
        .collect();
    let sums = sums
        .with_precision_and_scale(38, scale)
        .map_err(DataError::arrow)?;
    Ok((Arc::new(sums), DataType::Decimal128(38, scale)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compensated_sum_keeps_what_a_plain_sum_loses() {
        // A plain sum gives 0 either way: 1e16 + 1 rounds to 1e16.
        for (first, second) in [(1e16, 1.0), (1.0, 1e16)] {
            let mut sum = FloatSum::default();
            sum.apply(first, 1);
            sum.apply(second, 1);
            sum.apply(1e16, -1);
            assert_eq!(sum.sum(), Some(1.0), "{first} then {second}");
        }
    }

    #[test]
    fn a_float_sum_forgets_its_rounding_once_its_values_are_gone() {
        let huge = [1.7e308, 1.3e308, 0.9e308, 1.1e308];
        let mut sum = FloatSum::default();
        sum.apply(1.0, 1);
        for v in huge {
            sum.apply(v, 1);
        }
        for v in huge.iter().rev() {
            sum.apply(*v, -1);
        }
        assert_eq!(sum.sum(), Some(1.0));
        let mut sum = FloatSum::default();
        for v in [0.1, 0.2, 0.3] {
            sum.apply(v, 1);
        }
        for v in [0.2, 0.1, 0.3] {
            sum.apply(v, -1);
        }
        sum.apply(1e-30, 1);
        assert_eq!(sum.sum(), Some(1e-30));
    }

    #[test]
    fn nan_and_infinities_leave_a_float_sum_with_their_values() {
        let mut sum = FloatSum::default();
        for _ in 0..100 {
            sum.apply(1.0, 1);
        }
        sum.apply(f64::NAN, 1);
        assert!(sum.sum().is_some_and(f64::is_nan));
        sum.apply(f64::NAN, -1);
        assert_eq!(sum.sum(), Some(100.0));
        sum.apply(f64::INFINITY, 1);
        assert_eq!(sum.sum(), Some(f64::INFINITY));
        sum.apply(f64::NEG_INFINITY, 1);
        assert!(sum.sum().is_some_and(f64::is_nan), "inf + -inf");
        sum.apply(f64::INFINITY, -1);
        assert_eq!(sum.sum(), Some(f64::NEG_INFINITY));
        sum.apply(f64::NEG_INFINITY, -1);
        assert_eq!(sum.sum(), Some(100.0));
        assert_eq!(sum.mean(), Some(1.0));
    }

    #[test]
    fn huge_floats_sum_past_f64_and_back() {
        let mut sum = FloatSum::default();
        sum.apply(1.0, 1);
        sum.apply(f64::MAX, 1);
        sum.apply(f64::MAX, 1);
        assert_eq!(sum.sum(), Some(f64::INFINITY));
        let mean = sum.mean().unwrap();
        let expected = f64::MAX / 3.0 * 2.0;
        assert!(
            (mean - expected).abs() <= expected * 1e-15,
            "{mean} vs {expected}"
        );
        sum.apply(f64::MAX, -1);
        assert_eq!(sum.sum(), Some(f64::MAX), "f64::MAX + 1");
        sum.apply(f64::MAX, -1);
        assert_eq!(sum.sum(), Some(1.0));
    }

    #[test]
    fn a_sum_just_below_overflow_does_not_round_to_infinity() {
        let mut sum = FloatSum::default();
        sum.apply(power_of_two(1023), 1);
        sum.apply(power_of_two(1023), 1);
        for _ in 0..4_096 {
            sum.apply(-power_of_two(959), 1);
        }
        // 2^1024 - 4096 * 2^959 = 2^1024 - 2^971: exactly f64::MAX.
        assert_eq!(sum.sum(), Some(f64::MAX));
    }

    #[test]
    fn huge_values_keep_their_rounding_too() {
        // Their scaled running sum rounds: only the compensation gets the last bit right.
        let (a, b, c) = (f64::MAX, 1e308, -1.5e308);
        let mut sum = FloatSum::default();
        for v in [a, b, c] {
            sum.apply(v, 1);
        }
        // a + c is exact (Sterbenz), so adding b to it rounds once: the correctly rounded sum.
        assert_eq!(sum.sum(), Some((a + c) + b));
    }

    #[test]
    fn an_exact_sum_past_i128_comes_back_when_the_values_leave() {
        for big in [i128::MAX, i128::MIN] {
            let mut sum = ExactSum::default();
            sum.apply(7, 1);
            sum.apply(big, 1);
            sum.apply(big, 1);
            assert_eq!(sum.total(), None, "7 + 2 * {big}");
            let mean = sum.mean().unwrap();
            assert!((mean - big as f64 * 2.0 / 3.0).abs() <= 1e25, "{mean}");
            sum.apply(big, -1);
            assert_eq!(sum.total(), big.checked_add(7));
            sum.apply(big, -1);
            assert_eq!(sum.total(), Some(7));
        }
    }
}
