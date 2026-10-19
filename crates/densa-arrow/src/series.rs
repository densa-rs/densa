//! Two numeric columns of a live table's view as a chart series.

use std::fmt;

use arrow_array::cast::AsArray;
use arrow_array::types::{
    ArrowPrimitiveType, Date32Type, Date64Type, Decimal128Type, Float32Type, Float64Type,
    Int16Type, Int32Type, Int64Type, Int8Type, TimestampMicrosecondType, TimestampMillisecondType,
    TimestampNanosecondType, TimestampSecondType, UInt16Type, UInt32Type, UInt64Type, UInt8Type,
};
use arrow_array::{Array, ArrayRef};
use arrow_schema::{DataType, TimeUnit};
use densa_chart::Points;
use densa_data::{LiveTable, View, ViewId};

/// Why a series could not be made.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SeriesError {
    /// The table has no column of this name.
    NoSuchColumn(String),
    /// The column of this name holds values that are not numbers, times or dates.
    NotNumeric(String),
}

impl fmt::Display for SeriesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchColumn(name) => write!(f, "no column named {name:?}"),
            Self::NotNumeric(name) => write!(f, "column {name:?} is not numeric"),
        }
    }
}

impl std::error::Error for SeriesError {}

/// Reads one value of a column as a number: NaN for a null, or for an array of another type.
type NumberFn = fn(&dyn Array, usize) -> f64;

/// A column found by name, with its reader.
#[derive(Debug, Clone, Copy)]
struct Column {
    index: usize,
    read: NumberFn,
}

/// Two columns of a `densa_data::LiveTable` read as numbers, `x` and `y`, so a view of the
/// table is a `densa_chart` series ([`points`](Self::points)).
///
/// Integers, `Float32`, `Float64` and `Decimal128` read as their value; timestamps as
/// seconds since the epoch (whatever their unit and time zone: instants are UTC), and dates
/// as the seconds of their midnight, so time columns go straight onto a time axis. Other
/// types (`Float16`, `Decimal256`, times of day, durations, dictionaries) are not numeric
/// here. Nulls are gaps.
///
/// A chart needs `x` ascending: give a view sorted by the `x` column, ascending. Its nulls
/// sort last, past every point a chart looks for, so they cost nothing; a descending view
/// draws wrong.
///
/// Like [`LiveGrid`](crate::LiveGrid), it keeps a handle to each batch's two arrays, brought up
/// to date by [`sync`](Self::sync), which allocates only when batches were added or
/// compacted; reading points allocates nothing. Until it is synced after a change,
/// [`points`](Self::points) gives nothing (never values from the wrong batch): sync it right
/// after changing the table, and before painting. Given another table, it finds its columns
/// again by name.
#[derive(Debug, Clone)]
pub struct LiveSeries {
    names: (String, String),
    /// `None` while the table has no numeric columns of those names.
    columns: Option<(Column, Column)>,
    /// The `x` and `y` arrays of each batch of the table.
    arrays: Vec<(ArrayRef, ArrayRef)>,
    /// The table object (`LiveTable::instance`) and generation the arrays are from.
    table: (u64, u64),
}

impl LiveSeries {
    /// A series of `table`'s columns named `x` and `y`.
    ///
    /// # Errors
    /// [`SeriesError::NoSuchColumn`] or [`SeriesError::NotNumeric`] for either column.
    pub fn new(table: &LiveTable, x: &str, y: &str) -> Result<Self, SeriesError> {
        let columns = (column(table, x)?, column(table, y)?);
        let mut series = Self {
            names: (x.to_owned(), y.to_owned()),
            columns: Some(columns),
            arrays: Vec::new(),
            table: (table.instance(), table.generation()),
        };
        series.sync(table);
        Ok(series)
    }

    /// Takes in `table`'s batches added or compacted since the last call (or another
    /// table: its columns are found again by name). Call it after the table changed, before
    /// reading points.
    pub fn sync(&mut self, table: &LiveTable) {
        let current = (table.instance(), table.generation());
        if current != self.table {
            if self.table.0 != current.0 {
                self.columns = column(table, &self.names.0)
                    .and_then(|x| Ok((x, column(table, &self.names.1)?)))
                    .ok();
            }
            self.arrays.clear();
            self.table = current;
        }
        let Some((x, y)) = self.columns else {
            return;
        };
        let new = table.batches().get(self.arrays.len()..).unwrap_or_default();
        for batch in new {
            self.arrays
                .push((batch.column(x.index).clone(), batch.column(y.index).clone()));
        }
    }

    /// The name of the `x` column.
    pub fn x_name(&self) -> &str {
        &self.names.0
    }

    /// The name of the `y` column.
    pub fn y_name(&self) -> &str {
        &self.names.1
    }

    /// Whether the table last synced has numeric columns of these names (after another
    /// table, it may not).
    pub fn is_resolved(&self) -> bool {
        self.columns.is_some()
    }

    /// The rows of `view` as points, in the view's order, or `None` if the view does not
    /// exist, the table changed since [`sync`](Self::sync) (never values from the wrong
    /// batch), or the table has no numeric columns of the names given.
    pub fn points<'a>(&'a self, table: &'a LiveTable, view: ViewId) -> Option<SeriesPoints<'a>> {
        let (x, y) = self.columns?;
        let synced = self.table == (table.instance(), table.generation())
            && self.arrays.len() == table.batches().len();
        if !synced {
            return None;
        }
        Some(SeriesPoints {
            arrays: &self.arrays,
            x: x.read,
            y: y.read,
            rows: table.view(view)?,
        })
    }
}

/// A view's rows as points of a [`LiveSeries`]. Cheap to make each frame; reading allocates
/// nothing.
#[derive(Debug, Clone, Copy)]
pub struct SeriesPoints<'a> {
    arrays: &'a [(ArrayRef, ArrayRef)],
    x: NumberFn,
    y: NumberFn,
    rows: View<'a>,
}

impl SeriesPoints<'_> {
    /// The lowest and highest `x`, for a view sorted by `x` (nulls sort last, so the
    /// finite values come first), or `None` if there is none: found by binary search.
    pub fn x_extent(&self) -> Option<(f64, f64)> {
        let first = self.x(0);
        if !first.is_finite() {
            return None;
        }
        // The rows with a finite `x` come first: find where they end.
        let (mut low, mut high) = (0, self.rows.len());
        while low < high {
            let middle = low + (high - low) / 2;
            if self.x(middle).is_finite() {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let last = self.x(low - 1);
        Some((first.min(last), first.max(last)))
    }

    /// Reads row `index` of the view with `read` from the array `pick` chooses.
    fn read(
        &self,
        index: usize,
        pick: fn(&(ArrayRef, ArrayRef)) -> &ArrayRef,
        read: NumberFn,
    ) -> f64 {
        self.rows
            .row(index)
            .and_then(|at| Some((self.arrays.get(at.batch)?, at.row)))
            .map_or(f64::NAN, |(arrays, row)| read(pick(arrays).as_ref(), row))
    }
}

impl Points for SeriesPoints<'_> {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn x(&self, index: usize) -> f64 {
        self.read(index, |arrays| &arrays.0, self.x)
    }

    fn y(&self, index: usize) -> f64 {
        self.read(index, |arrays| &arrays.1, self.y)
    }
}

/// The column of `table` named `name`, with a reader for its type.
fn column(table: &LiveTable, name: &str) -> Result<Column, SeriesError> {
    let schema = table.schema();
    let (index, field) = schema
        .column_with_name(name)
        .ok_or_else(|| SeriesError::NoSuchColumn(name.to_owned()))?;
    let read =
        number_reader(field.data_type()).ok_or_else(|| SeriesError::NotNumeric(name.to_owned()))?;
    Ok(Column { index, read })
}

/// The reader for values of `data_type`, or `None` if they are not numbers, times or dates.
fn number_reader(data_type: &DataType) -> Option<NumberFn> {
    Some(match data_type {
        DataType::Int8 => read::<Int8Type>,
        DataType::Int16 => read::<Int16Type>,
        DataType::Int32 => read::<Int32Type>,
        DataType::Int64 => read::<Int64Type>,
        DataType::UInt8 => read::<UInt8Type>,
        DataType::UInt16 => read::<UInt16Type>,
        DataType::UInt32 => read::<UInt32Type>,
        DataType::UInt64 => read::<UInt64Type>,
        DataType::Float32 => read::<Float32Type>,
        DataType::Float64 => read::<Float64Type>,
        DataType::Decimal128(_, _) => read_decimal,
        DataType::Timestamp(TimeUnit::Second, _) => read::<TimestampSecondType>,
        DataType::Timestamp(TimeUnit::Millisecond, _) => read_scaled::<TimestampMillisecondType, 3>,
        DataType::Timestamp(TimeUnit::Microsecond, _) => read_scaled::<TimestampMicrosecondType, 6>,
        DataType::Timestamp(TimeUnit::Nanosecond, _) => read_scaled::<TimestampNanosecondType, 9>,
        DataType::Date32 => read_days,
        DataType::Date64 => read_scaled::<Date64Type, 3>,
        _ => return None,
    })
}

/// A value that converts to a number for a chart.
trait ToNumber {
    fn to_number(self) -> f64;
}

macro_rules! to_number {
    ($($t:ty),*) => {$(
        impl ToNumber for $t {
            fn to_number(self) -> f64 {
                // Lossless enough: a chart draws to a pixel, far coarser than 2^-53.
                self as f64
            }
        }
    )*};
}

to_number!(i8, i16, i32, i64, u8, u16, u32, u64, f32, f64);

/// Reads a primitive value as a number.
fn read<T>(array: &dyn Array, row: usize) -> f64
where
    T: ArrowPrimitiveType,
    T::Native: ToNumber,
{
    match array.as_primitive_opt::<T>() {
        Some(values) if row < values.len() && values.is_valid(row) => values.value(row).to_number(),
        _ => f64::NAN,
    }
}

/// Reads a primitive value counted in `10^-DIGITS` of a unit as a number of units.
fn read_scaled<T, const DIGITS: i32>(array: &dyn Array, row: usize) -> f64
where
    T: ArrowPrimitiveType,
    T::Native: ToNumber,
{
    read::<T>(array, row) / 10f64.powi(DIGITS)
}

/// Reads days since the epoch as the seconds of that midnight.
fn read_days(array: &dyn Array, row: usize) -> f64 {
    read::<Date32Type>(array, row) * 86_400.0
}

/// Reads a decimal as its value: the integer over ten to its scale.
fn read_decimal(array: &dyn Array, row: usize) -> f64 {
    match array.as_primitive_opt::<Decimal128Type>() {
        Some(values) if row < values.len() && values.is_valid(row) => {
            // Lossless enough: a chart draws to a pixel.
            let value = values.value(row) as f64;
            let scale = i32::from(values.scale());
            // Dividing by an exact power of ten, rather than multiplying by an inexact
            // tenth, keeps 3 at scale -2 and 1.5 at scale 2 exact.
            if scale >= 0 {
                value / 10f64.powi(scale)
            } else {
                value * 10f64.powi(-scale)
            }
        }
        _ => f64::NAN,
    }
}
