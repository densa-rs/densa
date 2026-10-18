//! One column of one batch, with the function that formats its values chosen once, from its
//! data type, when the batch is added.

use std::fmt::{Display, Write as _};

use arrow_array::cast::AsArray;
use arrow_array::types::{
    ArrowDictionaryKeyType, ArrowPrimitiveType, Date32Type, Date64Type, Decimal128Type,
    DurationMicrosecondType, DurationMillisecondType, DurationNanosecondType, DurationSecondType,
    Float16Type, Float32Type, Float64Type, Int16Type, Int32Type, Int64Type, Int8Type,
    Time32MillisecondType, Time32SecondType, Time64MicrosecondType, Time64NanosecondType,
    TimestampMicrosecondType, TimestampMillisecondType, TimestampNanosecondType,
    TimestampSecondType, UInt16Type, UInt32Type, UInt64Type, UInt8Type,
};
use arrow_array::{Array, ArrayRef, OffsetSizeTrait};
use arrow_buffer::NullBuffer;
use arrow_schema::{DataType, TimeUnit};
use densa_fmt::write_fixed;
use densa_grid::CellText;

use crate::calendar::{write_date, write_time, write_timestamp, SECONDS_PER_DAY};

/// How a column's values are formatted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Format {
    /// Fixed decimal places for floating-point values; `None` prints the shortest exact form.
    pub(crate) decimals: Option<usize>,
    /// Text longer than this many characters is cut, ending in `…`.
    pub(crate) max_chars: usize,
    /// Whether `max_chars` was set by the application, so the first rows must not resize it.
    pub(crate) width_set: bool,
}

/// Formats the value at a row; `false` if the type has no formatter, the value is invalid or
/// the text is too long for a cell (what was written is then partial).
type WriteFn = fn(&Reader, usize, &mut CellText, &Format) -> bool;

/// Reads one dictionary key as an index into the dictionary's values.
type KeyFn = fn(&dyn Array, usize) -> Option<usize>;

/// A column of one batch, ready to format.
#[derive(Clone)]
pub(crate) struct Reader {
    array: ArrayRef,
    /// Logical nulls, computed once: they include null dictionary values and the nulls of
    /// types without a validity buffer (`Null`, run-end encoded, unions).
    nulls: Option<NullBuffer>,
    write: WriteFn,
    /// For a dictionary column: how to read a key, and the values' own reader.
    dictionary: Option<(KeyFn, Box<Reader>)>,
    supported: bool,
}

impl std::fmt::Debug for Reader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reader")
            .field("data_type", self.array.data_type())
            .finish_non_exhaustive()
    }
}

impl Reader {
    /// A reader for `array`, its formatter chosen by data type.
    pub(crate) fn new(array: ArrayRef) -> Self {
        let dictionary = match array.data_type() {
            DataType::Dictionary(key, values) => dictionary_key(key).map(|key| {
                let values = array_values(&array, values);
                (key, Box::new(Reader::new(values)))
            }),
            _ => None,
        };
        let write = if dictionary.is_some() {
            Some(write_dictionary as WriteFn)
        } else {
            writer(array.data_type())
        };
        Self {
            nulls: array.logical_nulls(),
            array,
            write: write.unwrap_or(write_unsupported),
            supported: write.is_some(),
            dictionary,
        }
    }

    /// Rows in the column.
    pub(crate) fn len(&self) -> usize {
        self.array.len()
    }

    /// Whether this column's type can be formatted.
    pub(crate) fn is_supported(&self) -> bool {
        self.supported
            && self
                .dictionary
                .as_ref()
                .is_none_or(|(_, v)| v.is_supported())
    }

    /// Whether the value at `row` is null (for a dictionary: its key or its value).
    pub(crate) fn is_null(&self, row: usize) -> bool {
        self.nulls.as_ref().is_some_and(|nulls| nulls.is_null(row))
    }

    /// Appends the value at `row` to `text`; `false` if it could not be formatted.
    pub(crate) fn write(&self, row: usize, text: &mut CellText, format: &Format) -> bool {
        (self.write)(self, row, text, format)
    }
}

/// Numbers and times read best right-aligned.
pub(crate) fn is_numeric(data_type: &DataType) -> bool {
    use DataType as D;
    match data_type {
        D::Dictionary(_, values) => is_numeric(values),
        D::Int8 | D::Int16 | D::Int32 | D::Int64 => true,
        D::UInt8 | D::UInt16 | D::UInt32 | D::UInt64 => true,
        D::Float16 | D::Float32 | D::Float64 | D::Decimal128(..) => true,
        D::Date32 | D::Date64 | D::Timestamp(..) | D::Time32(_) | D::Time64(_) => true,
        D::Duration(_) => true,
        _ => false,
    }
}

/// The formatter for a (non-dictionary) data type.
fn writer(data_type: &DataType) -> Option<WriteFn> {
    use DataType as D;
    Some(match data_type {
        D::Boolean => write_bool,
        D::Int8 => write_display::<Int8Type>,
        D::Int16 => write_display::<Int16Type>,
        D::Int32 => write_display::<Int32Type>,
        D::Int64 => write_display::<Int64Type>,
        D::UInt8 => write_display::<UInt8Type>,
        D::UInt16 => write_display::<UInt16Type>,
        D::UInt32 => write_display::<UInt32Type>,
        D::UInt64 => write_display::<UInt64Type>,
        D::Float16 => write_float::<Float16Type>,
        D::Float32 => write_float::<Float32Type>,
        D::Float64 => write_float::<Float64Type>,
        D::Decimal128(_, scale) => decimal_writer(*scale),
        D::Utf8 => write_string::<i32>,
        D::LargeUtf8 => write_string::<i64>,
        D::Utf8View => write_string_view,
        D::Date32 => write_date32,
        D::Date64 => write_date64,
        D::Timestamp(unit, zone) => timestamp_writer(*unit, zone.is_some()),
        D::Time32(TimeUnit::Second) => write_time_of_day::<Time32SecondType, 1, 0>,
        D::Time32(TimeUnit::Millisecond) => write_time_of_day::<Time32MillisecondType, 1_000, 3>,
        D::Time64(TimeUnit::Microsecond) => {
            write_time_of_day::<Time64MicrosecondType, 1_000_000, 6>
        }
        D::Time64(TimeUnit::Nanosecond) => {
            write_time_of_day::<Time64NanosecondType, 1_000_000_000, 9>
        }
        D::Duration(TimeUnit::Second) => write_duration::<DurationSecondType, 0>,
        D::Duration(TimeUnit::Millisecond) => write_duration::<DurationMillisecondType, 1>,
        D::Duration(TimeUnit::Microsecond) => write_duration::<DurationMicrosecondType, 2>,
        D::Duration(TimeUnit::Nanosecond) => write_duration::<DurationNanosecondType, 3>,
        _ => return None,
    })
}

fn write_unsupported(_: &Reader, _: usize, text: &mut CellText, _: &Format) -> bool {
    // A cell always has room for one byte.
    let _ = text.push_str("?");
    false
}

fn write_bool(reader: &Reader, row: usize, text: &mut CellText, _: &Format) -> bool {
    let Some(array) = reader.array.as_boolean_opt() else {
        return false;
    };
    let value = if array.value(row) { "true" } else { "false" };
    text.push_str(value).is_ok()
}

fn write_display<T>(reader: &Reader, row: usize, text: &mut CellText, _: &Format) -> bool
where
    T: ArrowPrimitiveType,
    T::Native: Display,
{
    reader
        .array
        .as_primitive_opt::<T>()
        .is_some_and(|array| write!(text, "{}", array.value(row)).is_ok())
}

fn write_float<T>(reader: &Reader, row: usize, text: &mut CellText, format: &Format) -> bool
where
    T: ArrowPrimitiveType,
    T::Native: Display + Into<f64>,
{
    let Some(array) = reader.array.as_primitive_opt::<T>() else {
        return false;
    };
    let value = array.value(row);
    // Lossless: f16 and f32 widen exactly to f64.
    let wide: f64 = value.into();
    match format.decimals {
        Some(decimals) if wide.is_finite() => write_fixed(text, wide, decimals).is_ok(),
        // NaN and infinities print as `NaN`, `inf` and `-inf` whatever the decimals.
        _ => write!(text, "{value}").is_ok(),
    }
}

fn decimal_writer(scale: i8) -> WriteFn {
    match scale {
        s if s > 0 => write_decimal_scaled,
        _ => write_decimal_whole,
    }
}

/// A decimal with digits after the point: its integer value divided by 10^scale.
fn write_decimal_scaled(reader: &Reader, row: usize, text: &mut CellText, _: &Format) -> bool {
    let Some(array) = reader.array.as_primitive_opt::<Decimal128Type>() else {
        return false;
    };
    let value = array.value(row);
    // Lossless: a positive scale is at most 38 for Decimal128.
    let scale = array.scale().unsigned_abs() as usize;
    let Some(divisor) = 10u128.checked_pow(scale as u32) else {
        return false;
    };
    let magnitude = value.unsigned_abs();
    let sign = if value < 0 { "-" } else { "" };
    let (whole, fraction) = (magnitude / divisor, magnitude % divisor);
    write!(text, "{sign}{whole}.{fraction:0scale$}").is_ok()
}

/// A decimal with a zero or negative scale: its integer value followed by -scale zeros.
fn write_decimal_whole(reader: &Reader, row: usize, text: &mut CellText, _: &Format) -> bool {
    let Some(array) = reader.array.as_primitive_opt::<Decimal128Type>() else {
        return false;
    };
    let value = array.value(row);
    let zeros = if value == 0 {
        0
    } else {
        usize::from(array.scale().unsigned_abs())
    };
    write!(text, "{value}{:0>zeros$}", "").is_ok()
}

fn write_string<O: OffsetSizeTrait>(
    reader: &Reader,
    row: usize,
    text: &mut CellText,
    format: &Format,
) -> bool {
    reader
        .array
        .as_string_opt::<O>()
        .is_some_and(|array| write_cut(text, array.value(row), format.max_chars))
}

fn write_string_view(reader: &Reader, row: usize, text: &mut CellText, format: &Format) -> bool {
    reader
        .array
        .as_string_view_opt()
        .is_some_and(|array| write_cut(text, array.value(row), format.max_chars))
}

/// Appends `value`, or as much of it as fits in `max_chars` characters (and in the cell)
/// followed by `…`.
fn write_cut(text: &mut CellText, value: &str, max_chars: usize) -> bool {
    const ELLIPSIS: &str = "…";
    let room = densa_grid::CELL_BYTES - text.len();
    if value.len() <= room && value.chars().count() <= max_chars {
        return text.push_str(value).is_ok();
    }
    let keep = max_chars.saturating_sub(1);
    let mut end = 0;
    for (count, (index, ch)) in value.char_indices().enumerate() {
        if count == keep || index + ch.len_utf8() + ELLIPSIS.len() > room {
            break;
        }
        end = index + ch.len_utf8();
    }
    text.push_str(&value[..end]).is_ok() && text.push_str(ELLIPSIS).is_ok()
}

fn write_date32(reader: &Reader, row: usize, text: &mut CellText, _: &Format) -> bool {
    reader
        .array
        .as_primitive_opt::<Date32Type>()
        .is_some_and(|array| write_date(text, i64::from(array.value(row))).is_ok())
}

fn write_date64(reader: &Reader, row: usize, text: &mut CellText, _: &Format) -> bool {
    const MS_PER_DAY: i64 = SECONDS_PER_DAY * 1_000;
    reader
        .array
        .as_primitive_opt::<Date64Type>()
        .is_some_and(|array| write_date(text, array.value(row).div_euclid(MS_PER_DAY)).is_ok())
}

fn timestamp_writer(unit: TimeUnit, utc: bool) -> WriteFn {
    match (unit, utc) {
        (TimeUnit::Second, false) => write_timestamp_in::<TimestampSecondType, 1, 0, false>,
        (TimeUnit::Second, true) => write_timestamp_in::<TimestampSecondType, 1, 0, true>,
        (TimeUnit::Millisecond, false) => {
            write_timestamp_in::<TimestampMillisecondType, 1_000, 3, false>
        }
        (TimeUnit::Millisecond, true) => {
            write_timestamp_in::<TimestampMillisecondType, 1_000, 3, true>
        }
        (TimeUnit::Microsecond, false) => {
            write_timestamp_in::<TimestampMicrosecondType, 1_000_000, 6, false>
        }
        (TimeUnit::Microsecond, true) => {
            write_timestamp_in::<TimestampMicrosecondType, 1_000_000, 6, true>
        }
        (TimeUnit::Nanosecond, false) => {
            write_timestamp_in::<TimestampNanosecondType, 1_000_000_000, 9, false>
        }
        (TimeUnit::Nanosecond, true) => {
            write_timestamp_in::<TimestampNanosecondType, 1_000_000_000, 9, true>
        }
    }
}

/// A timestamp counted in `1 / PER_SECOND` seconds. Values with a time zone are UTC instants
/// and print in UTC with a `Z` (converting to the zone would need a time-zone database).
fn write_timestamp_in<T, const PER_SECOND: i64, const DIGITS: usize, const UTC: bool>(
    reader: &Reader,
    row: usize,
    text: &mut CellText,
    _: &Format,
) -> bool
where
    T: ArrowPrimitiveType<Native = i64>,
{
    let Some(array) = reader.array.as_primitive_opt::<T>() else {
        return false;
    };
    write_timestamp(text, array.value(row), PER_SECOND, DIGITS).is_ok()
        && (!UTC || text.push_str("Z").is_ok())
}

/// A time of day counted in `1 / PER_SECOND` seconds since midnight.
fn write_time_of_day<T, const PER_SECOND: i64, const DIGITS: usize>(
    reader: &Reader,
    row: usize,
    text: &mut CellText,
    _: &Format,
) -> bool
where
    T: ArrowPrimitiveType,
    T::Native: Into<i64>,
{
    let Some(array) = reader.array.as_primitive_opt::<T>() else {
        return false;
    };
    let value: i64 = array.value(row).into();
    let (seconds, fraction) = (value.div_euclid(PER_SECOND), value.rem_euclid(PER_SECOND));
    // A time of day outside one day is invalid data: no clock time to show.
    (0..SECONDS_PER_DAY).contains(&seconds) && write_time(text, seconds, fraction, DIGITS).is_ok()
}

/// A duration with its unit: `s`, `ms`, `us` or `ns` for `UNIT` 0 to 3.
fn write_duration<T, const UNIT: usize>(
    reader: &Reader,
    row: usize,
    text: &mut CellText,
    _: &Format,
) -> bool
where
    T: ArrowPrimitiveType<Native = i64>,
{
    const SUFFIXES: [&str; 4] = ["s", "ms", "us", "ns"];
    reader
        .array
        .as_primitive_opt::<T>()
        .is_some_and(|array| write!(text, "{}{}", array.value(row), SUFFIXES[UNIT]).is_ok())
}

fn write_dictionary(reader: &Reader, row: usize, text: &mut CellText, format: &Format) -> bool {
    let Some((key, values)) = &reader.dictionary else {
        return false;
    };
    key(&*reader.array, row).is_some_and(|index| values.write(index, text, format))
}

/// How to read keys of type `key`; `None` for a type that is not a dictionary key.
fn dictionary_key(key: &DataType) -> Option<KeyFn> {
    use DataType as D;
    Some(match key {
        D::Int8 => key_at::<Int8Type>,
        D::Int16 => key_at::<Int16Type>,
        D::Int32 => key_at::<Int32Type>,
        D::Int64 => key_at::<Int64Type>,
        D::UInt8 => key_at::<UInt8Type>,
        D::UInt16 => key_at::<UInt16Type>,
        D::UInt32 => key_at::<UInt32Type>,
        D::UInt64 => key_at::<UInt64Type>,
        _ => return None,
    })
}

fn key_at<K>(array: &dyn Array, row: usize) -> Option<usize>
where
    K: ArrowDictionaryKeyType,
    K::Native: TryInto<usize>,
{
    let dictionary = array.as_dictionary_opt::<K>()?;
    // A negative key is invalid data: treated as null rather than as a huge index.
    let key: usize = dictionary.keys().value(row).try_into().ok()?;
    (key < dictionary.values().len()).then_some(key)
}

/// The values array of a dictionary `array` whose values have type `values`.
fn array_values(array: &ArrayRef, values: &DataType) -> ArrayRef {
    array
        .as_any_dictionary_opt()
        .map(|dictionary| dictionary.values().clone())
        .unwrap_or_else(|| arrow_array::new_null_array(values, 0))
}
