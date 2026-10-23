//! Time windows: a table that keeps only its rows within a span of the newest time.

use std::time::Duration;

use arrow_schema::{DataType, Schema, TimeUnit};

use crate::filter::resolve;
use crate::value::{Kind, Value, ValueFn};
use crate::view::{Rows, SortColumn, ViewState};
use crate::DataError;

/// Keeps a [`LiveTable`](crate::LiveTable) to its recent rows: those whose value in a time
/// column is within a span of the newest. Set it with
/// [`LiveTable::set_window`](crate::LiveTable::set_window).
///
/// The window ends at the largest finite value of the column among the live rows, so it
/// moves with the data, not with the clock: replayed or delayed data is windowed by its own
/// times, and nothing expires while no rows arrive. A row expires when the end is more than
/// the span past its value; rows whose value is null or NaN never do.
///
/// ```
/// use std::sync::Arc;
/// use std::time::Duration;
/// use arrow_array::{ArrayRef, Int64Array, RecordBatch, TimestampMillisecondArray};
/// use densa_data::{LiveTable, Window};
///
/// let ticks = |ids: Vec<i64>, times: Vec<i64>| {
///     RecordBatch::try_from_iter([
///         ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
///         ("time", Arc::new(TimestampMillisecondArray::from(times)) as ArrayRef),
///     ])
/// };
/// let mut table = LiveTable::new(ticks(vec![], vec![])?.schema(), "id")?;
/// table.set_window(Some(Window::duration("time", Duration::from_secs(60))))?;
///
/// table.upsert(ticks(vec![1, 2], vec![0, 30_000])?)?;
/// let changes = table.upsert(ticks(vec![3], vec![75_000])?)?;
/// assert_eq!(changes.deleted, 1); // row 1 is 75 s older than the newest row
/// assert_eq!(table.len(), 2);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    column: String,
    span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Span {
    Duration(Duration),
    Value(f64),
}

impl Window {
    /// A window of `span` over a column of timestamps, dates, times of day or durations.
    #[must_use]
    pub fn duration(column: impl Into<String>, span: Duration) -> Self {
        Self {
            column: column.into(),
            span: Span::Duration(span),
        }
    }

    /// A window of `span` over a column of numbers (integers, decimals or floats), in the
    /// column's own units: `60_000.0` keeps a minute of milliseconds. `span` must be zero or
    /// more. A span within a billionth of a whole number of an integer or decimal column's
    /// units counts as that number, so float error (`0.29` hundredths) loses no unit.
    #[must_use]
    pub fn span(column: impl Into<String>, span: f64) -> Self {
        Self {
            column: column.into(),
            span: Span::Value(span),
        }
    }

    /// The time column's name.
    pub fn column(&self) -> &str {
        &self.column
    }
}

/// How far below the newest value a row may be, in the column's units: whole units for
/// integer columns (differences are whole, so the span rounds down without changing which
/// rows stay), the span itself for floats.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Reach {
    Whole(i128),
    Float(f64),
}

/// A window bound to its column, with the table's rows in the column's order.
#[derive(Debug, Clone)]
pub(crate) struct WindowState {
    spec: Window,
    reach: Reach,
    /// Every live row, oldest first (nulls, then NaN, last).
    pub(crate) rows: ViewState,
    column: usize,
    read: ValueFn,
}

impl WindowState {
    pub(crate) fn compile(spec: &Window, schema: &Schema) -> Result<Self, DataError> {
        let (column, kind, read) = resolve(schema, &spec.column)?;
        let mismatch = || DataError::WindowSpan(spec.column.clone());
        let reach = match (spec.span, time_unit(schema.field(column).data_type())) {
            (Span::Duration(span), Some(unit)) => {
                // Whole units, rounding down. Lossless: at most 2^64 seconds of nanoseconds.
                Reach::Whole((span.as_nanos() / unit).cast_signed())
            }
            (Span::Value(span), None) if span >= 0.0 => match kind {
                Kind::Int | Kind::UInt => Reach::Whole(whole(span)),
                Kind::Decimal { scale } => Reach::Whole(whole(span * 10f64.powi(scale.into()))),
                Kind::Float => Reach::Float(span),
                Kind::Bool | Kind::Str => return Err(mismatch()),
            },
            _ => return Err(mismatch()),
        };
        Ok(Self {
            spec: spec.clone(),
            reach,
            rows: ViewState::sorted(SortColumn::ascending(column, read)),
            column,
            read,
        })
    }

    pub(crate) fn spec(&self) -> &Window {
        &self.spec
    }

    /// How many of the oldest rows are past the window.
    pub(crate) fn expired(&self, rows: Rows<'_>) -> usize {
        let value = |id| {
            let at = rows.at(id);
            (self.read)(rows.batches[at.batch].column(self.column).as_ref(), at.row)
        };
        // Values below +inf come first (-inf, then numbers); +inf, NaN and nulls after them.
        // The newest is the last of those. If it is -inf, every row below +inf is -inf, and
        // `-inf - -inf` is NaN: none is older.
        let below = self.rows.partition_point(|id| below_infinity(value(id)));
        let Some(newest) = below.checked_sub(1).and_then(|i| self.rows.get(i)) else {
            return 0;
        };
        let newest = value(newest);
        self.rows
            .partition_point(|id| older(value(id), newest, self.reach))
    }
}

const NANOS_PER_DAY: u128 = 86_400 * 1_000_000_000;

/// The nanoseconds in one unit of a column of dates, timestamps, times or durations; `None`
/// for other types.
fn time_unit(data_type: &DataType) -> Option<u128> {
    let unit = match data_type {
        DataType::Timestamp(unit, _)
        | DataType::Time32(unit)
        | DataType::Time64(unit)
        | DataType::Duration(unit) => unit,
        DataType::Date32 => return Some(NANOS_PER_DAY),
        DataType::Date64 => &TimeUnit::Millisecond,
        _ => return None,
    };
    Some(match unit {
        TimeUnit::Second => 1_000_000_000,
        TimeUnit::Millisecond => 1_000_000,
        TimeUnit::Microsecond => 1_000,
        TimeUnit::Nanosecond => 1,
    })
}

/// `span` rounded down to a whole number (saturating: a span past `i128` keeps every row).
/// A whole number a float error away is taken as it: `0.29 * 100.0` is 28.999999999999996.
fn whole(span: f64) -> i128 {
    let nearest = span.round();
    let span = match (span - nearest).abs() <= NEAR * nearest.abs() {
        true => nearest,
        false => span.floor(),
    };
    // Saturating cast; `span` is not NaN (checked by the caller's `>= 0.0`).
    span as i128
}

/// How close, relative to it, a span must be to a whole number to be taken as it.
const NEAR: f64 = 1e-9;

fn below_infinity(value: Value<'_>) -> bool {
    match value {
        Value::Int(_) | Value::UInt(_) | Value::Decimal(_) => true,
        Value::Float(v) => v < f64::INFINITY,
        Value::Null | Value::Bool(_) | Value::Str(_) => false,
    }
}

/// Whether `value` is more than `reach` below `newest`, the window's end (a value of the same
/// column below +inf).
fn older(value: Value<'_>, newest: Value<'_>, reach: Reach) -> bool {
    let whole = |v: Value<'_>| match v {
        Value::Int(v) => Some(i128::from(v)),
        Value::UInt(v) => Some(i128::from(v)),
        Value::Decimal(v) => Some(v),
        _ => None,
    };
    match (reach, value, newest) {
        (Reach::Float(reach), Value::Float(value), Value::Float(newest)) => newest - value > reach,
        (Reach::Whole(reach), value, newest) => match (whole(value), whole(newest)) {
            // Saturating: two decimals far apart are as far apart as can be.
            (Some(value), Some(newest)) => newest.saturating_sub(value) > reach,
            _ => false,
        },
        _ => false,
    }
}
