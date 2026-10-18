//! Typed values read from Arrow columns, for sorting and filtering.

use std::cmp::Ordering;

use arrow_array::cast::AsArray;
use arrow_array::types::{
    ArrowDictionaryKeyType, Date32Type, Date64Type, Decimal128Type, DurationMicrosecondType,
    DurationMillisecondType, DurationNanosecondType, DurationSecondType, Float16Type, Float32Type,
    Float64Type, Int16Type, Int32Type, Int64Type, Int8Type, Time32MillisecondType,
    Time32SecondType, Time64MicrosecondType, Time64NanosecondType, TimestampMicrosecondType,
    TimestampMillisecondType, TimestampNanosecondType, TimestampSecondType, UInt16Type, UInt32Type,
    UInt64Type, UInt8Type,
};
use arrow_array::{Array, ArrowPrimitiveType, OffsetSizeTrait};
use arrow_schema::{DataType, TimeUnit};

use crate::Scalar;

/// One cell's value. Dates, times, timestamps and durations read as their integer count of
/// units; decimals as their unscaled integer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Value<'a> {
    Null,
    Int(i64),
    UInt(u64),
    Decimal(i128),
    Float(f64),
    Bool(bool),
    Str(&'a str),
}

/// What a column's values are, for checking filter literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Int,
    UInt,
    Decimal { scale: i8 },
    Float,
    Bool,
    Str,
}

impl Kind {
    pub(crate) fn is_numeric(self) -> bool {
        !matches!(self, Self::Bool | Self::Str)
    }
}

/// Reads the value at a row of a column.
pub(crate) type ValueFn = for<'a> fn(&'a dyn Array, usize) -> Value<'a>;

/// The kind and reader for a column of `data_type`; `None` if views cannot use it.
pub(crate) fn value_reader(data_type: &DataType) -> Option<(Kind, ValueFn)> {
    use DataType as D;
    use TimeUnit as U;
    Some(match data_type {
        D::Boolean => (Kind::Bool, read_bool),
        D::Int8 => (Kind::Int, read_int::<Int8Type>),
        D::Int16 => (Kind::Int, read_int::<Int16Type>),
        D::Int32 => (Kind::Int, read_int::<Int32Type>),
        D::Int64 => (Kind::Int, read_int::<Int64Type>),
        D::UInt8 => (Kind::UInt, read_uint::<UInt8Type>),
        D::UInt16 => (Kind::UInt, read_uint::<UInt16Type>),
        D::UInt32 => (Kind::UInt, read_uint::<UInt32Type>),
        D::UInt64 => (Kind::UInt, read_uint::<UInt64Type>),
        D::Float16 => (Kind::Float, read_float::<Float16Type>),
        D::Float32 => (Kind::Float, read_float::<Float32Type>),
        D::Float64 => (Kind::Float, read_float::<Float64Type>),
        D::Decimal128(_, scale) => (Kind::Decimal { scale: *scale }, read_decimal),
        D::Date32 => (Kind::Int, read_int::<Date32Type>),
        D::Date64 => (Kind::Int, read_int::<Date64Type>),
        D::Time32(U::Second) => (Kind::Int, read_int::<Time32SecondType>),
        D::Time32(U::Millisecond) => (Kind::Int, read_int::<Time32MillisecondType>),
        D::Time64(U::Microsecond) => (Kind::Int, read_int::<Time64MicrosecondType>),
        D::Time64(U::Nanosecond) => (Kind::Int, read_int::<Time64NanosecondType>),
        D::Timestamp(U::Second, _) => (Kind::Int, read_int::<TimestampSecondType>),
        D::Timestamp(U::Millisecond, _) => (Kind::Int, read_int::<TimestampMillisecondType>),
        D::Timestamp(U::Microsecond, _) => (Kind::Int, read_int::<TimestampMicrosecondType>),
        D::Timestamp(U::Nanosecond, _) => (Kind::Int, read_int::<TimestampNanosecondType>),
        D::Duration(U::Second) => (Kind::Int, read_int::<DurationSecondType>),
        D::Duration(U::Millisecond) => (Kind::Int, read_int::<DurationMillisecondType>),
        D::Duration(U::Microsecond) => (Kind::Int, read_int::<DurationMicrosecondType>),
        D::Duration(U::Nanosecond) => (Kind::Int, read_int::<DurationNanosecondType>),
        D::Utf8 => (Kind::Str, read_string::<i32>),
        D::LargeUtf8 => (Kind::Str, read_string::<i64>),
        D::Utf8View => (Kind::Str, read_string_view),
        D::Dictionary(key, values) if is_string(values) => (Kind::Str, dictionary_reader(key)?),
        _ => return None,
    })
}

pub(crate) fn is_string(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
    )
}

fn read_bool(array: &dyn Array, row: usize) -> Value<'_> {
    match array.as_boolean_opt() {
        Some(a) if a.is_valid(row) => Value::Bool(a.value(row)),
        _ => Value::Null,
    }
}

fn read_int<T>(array: &dyn Array, row: usize) -> Value<'_>
where
    T: ArrowPrimitiveType,
    T::Native: Into<i64>,
{
    match array.as_primitive_opt::<T>() {
        Some(a) if a.is_valid(row) => Value::Int(a.value(row).into()),
        _ => Value::Null,
    }
}

fn read_uint<T>(array: &dyn Array, row: usize) -> Value<'_>
where
    T: ArrowPrimitiveType,
    T::Native: Into<u64>,
{
    match array.as_primitive_opt::<T>() {
        Some(a) if a.is_valid(row) => Value::UInt(a.value(row).into()),
        _ => Value::Null,
    }
}

fn read_float<T>(array: &dyn Array, row: usize) -> Value<'_>
where
    T: ArrowPrimitiveType,
    T::Native: Into<f64>,
{
    match array.as_primitive_opt::<T>() {
        Some(a) if a.is_valid(row) => Value::Float(canonical(a.value(row).into())),
        _ => Value::Null,
    }
}

/// One NaN (positive) and one zero (positive): sorting and filtering then agree that `-0.0`
/// equals `0.0`, and every NaN sorts after infinity (`0.0 / 0.0` is a negative NaN on x86).
pub(crate) fn canonical(v: f64) -> f64 {
    if v.is_nan() {
        f64::NAN
    } else if v == 0.0 {
        0.0
    } else {
        v
    }
}

fn read_decimal(array: &dyn Array, row: usize) -> Value<'_> {
    match array.as_primitive_opt::<Decimal128Type>() {
        Some(a) if a.is_valid(row) => Value::Decimal(a.value(row)),
        _ => Value::Null,
    }
}

fn read_string<O: OffsetSizeTrait>(array: &dyn Array, row: usize) -> Value<'_> {
    match array.as_string_opt::<O>() {
        Some(a) if a.is_valid(row) => Value::Str(a.value(row)),
        _ => Value::Null,
    }
}

fn read_string_view(array: &dyn Array, row: usize) -> Value<'_> {
    match array.as_string_view_opt() {
        Some(a) if a.is_valid(row) => Value::Str(a.value(row)),
        _ => Value::Null,
    }
}

fn dictionary_reader(key: &DataType) -> Option<ValueFn> {
    use DataType as D;
    Some(match key {
        D::Int8 => read_dictionary::<Int8Type>,
        D::Int16 => read_dictionary::<Int16Type>,
        D::Int32 => read_dictionary::<Int32Type>,
        D::Int64 => read_dictionary::<Int64Type>,
        D::UInt8 => read_dictionary::<UInt8Type>,
        D::UInt16 => read_dictionary::<UInt16Type>,
        D::UInt32 => read_dictionary::<UInt32Type>,
        D::UInt64 => read_dictionary::<UInt64Type>,
        _ => return None,
    })
}

/// A dictionary of strings: the string its key points at.
fn read_dictionary<K>(array: &dyn Array, row: usize) -> Value<'_>
where
    K: ArrowDictionaryKeyType,
    K::Native: TryInto<usize>,
{
    let Some(dictionary) = array.as_dictionary_opt::<K>() else {
        return Value::Null;
    };
    if dictionary.is_null(row) {
        return Value::Null;
    }
    // A negative or out-of-range key is invalid data: read as null rather than panicking.
    let Ok(key) = dictionary.keys().value(row).try_into() else {
        return Value::Null;
    };
    let values = dictionary.values().as_ref();
    if key >= values.len() {
        return Value::Null;
    }
    match values.data_type() {
        DataType::Utf8 => read_string::<i32>(values, key),
        DataType::LargeUtf8 => read_string::<i64>(values, key),
        _ => read_string_view(values, key),
    }
}

/// Orders two values of one column. Nulls sort after every value whatever the direction;
/// floats (read canonical: one NaN, one zero) sort with NaN after infinity.
pub(crate) fn compare(a: Value<'_>, b: Value<'_>, descending: bool) -> Ordering {
    let order = match (a, b) {
        (Value::Null, Value::Null) => return Ordering::Equal,
        (Value::Null, _) => return Ordering::Greater,
        (_, Value::Null) => return Ordering::Less,
        (Value::Int(a), Value::Int(b)) => a.cmp(&b),
        (Value::UInt(a), Value::UInt(b)) => a.cmp(&b),
        (Value::Decimal(a), Value::Decimal(b)) => a.cmp(&b),
        (Value::Float(a), Value::Float(b)) => a.total_cmp(&b),
        (Value::Bool(a), Value::Bool(b)) => a.cmp(&b),
        (Value::Str(a), Value::Str(b)) => a.cmp(b),
        // One column holds one kind of value.
        _ => Ordering::Equal,
    };
    if descending {
        order.reverse()
    } else {
        order
    }
}

/// A 64-bit key whose order agrees with [`compare`]: when `prefix(a) < prefix(b)`, `a`
/// sorts before `b`. Equal prefixes decide nothing; the caller then compares the values.
/// Floats use the usual sign-flip encoding of IEEE total order, strings their first 8 bytes,
/// decimals their top 64 bits; nulls take the largest key in either direction.
pub(crate) fn prefix(value: Value<'_>, descending: bool) -> u64 {
    const SIGN: u64 = 1 << 63;
    let key = match value {
        Value::Null => return u64::MAX,
        // Two's complement with the sign bit flipped orders like the signed value.
        Value::Int(v) => (v as u64) ^ SIGN,
        Value::UInt(v) => v,
        Value::Decimal(v) => (((v as u128) ^ (1 << 127)) >> 64) as u64,
        Value::Float(v) => {
            let bits = v.to_bits();
            if bits & SIGN != 0 {
                !bits
            } else {
                bits | SIGN
            }
        }
        Value::Bool(v) => u64::from(v),
        Value::Str(s) => {
            let mut bytes = [0; 8];
            let len = s.len().min(8);
            bytes[..len].copy_from_slice(&s.as_bytes()[..len]);
            u64::from_be_bytes(bytes)
        }
    };
    if descending {
        !key
    } else {
        key
    }
}

/// Whether a literal can be compared with values of `kind`.
pub(crate) fn literal_fits(kind: Kind, literal: &Scalar) -> bool {
    match literal {
        Scalar::Int(_) | Scalar::UInt(_) | Scalar::Float(_) => kind.is_numeric(),
        Scalar::Bool(_) => kind == Kind::Bool,
        Scalar::Str(_) => kind == Kind::Str,
    }
}

/// `value` compared with `literal`; `None` for a null value or an unordered (NaN) pair.
/// Decimals compare at their scale: 5.00 equals the literal `Int(5)`.
pub(crate) fn compare_literal(kind: Kind, value: Value<'_>, literal: &Scalar) -> Option<Ordering> {
    match (value, literal) {
        (Value::Null, _) => None,
        (Value::Bool(v), Scalar::Bool(l)) => Some(v.cmp(l)),
        (Value::Str(v), Scalar::Str(l)) => Some(v.cmp(l.as_str())),
        (Value::Float(v), _) => v.partial_cmp(&literal_as_f64(literal)?),
        (_, Scalar::Float(l)) => as_f64(kind, value)?.partial_cmp(l),
        (_, Scalar::Int(_) | Scalar::UInt(_)) => {
            let literal = literal_as_i128(literal)?;
            compare_exact(kind, value, literal)
        }
        _ => None,
    }
}

fn literal_as_f64(literal: &Scalar) -> Option<f64> {
    // Lossy by design for huge integers: compared with a float, precision is the float's.
    match literal {
        Scalar::Int(l) => Some(*l as f64),
        Scalar::UInt(l) => Some(*l as f64),
        Scalar::Float(l) => Some(*l),
        _ => None,
    }
}

fn literal_as_i128(literal: &Scalar) -> Option<i128> {
    match literal {
        Scalar::Int(l) => Some(i128::from(*l)),
        Scalar::UInt(l) => Some(i128::from(*l)),
        _ => None,
    }
}

/// An integer-like value as a float, decimals divided by their scale.
fn as_f64(kind: Kind, value: Value<'_>) -> Option<f64> {
    match (kind, value) {
        (_, Value::Int(v)) => Some(v as f64),
        (_, Value::UInt(v)) => Some(v as f64),
        (Kind::Decimal { scale }, Value::Decimal(v)) => Some(unscale(v as f64, scale)),
        _ => None,
    }
}

/// A decimal's unscaled value `v` as the number it stands for.
pub(crate) fn unscale(v: f64, scale: i8) -> f64 {
    // Multiply or divide by an exact power of ten (10^n is exact in f64 for n <= 22), so a
    // decimal equal to a float literal compares equal.
    let power = 10f64.powi(i32::from(scale.unsigned_abs()));
    if scale >= 0 {
        v / power
    } else {
        v * power
    }
}

/// An integer-like value compared exactly with an integer literal.
fn compare_exact(kind: Kind, value: Value<'_>, literal: i128) -> Option<Ordering> {
    match (kind, value) {
        (_, Value::Int(v)) => Some(i128::from(v).cmp(&literal)),
        (_, Value::UInt(v)) => Some(i128::from(v).cmp(&literal)),
        (Kind::Decimal { scale }, Value::Decimal(v)) => {
            let factor = 10i128.checked_pow(u32::from(scale.unsigned_abs()));
            let exact = match (scale >= 0, factor) {
                (true, Some(f)) => literal.checked_mul(f).map(|l| v.cmp(&l)),
                (false, Some(f)) => v.checked_mul(f).map(|v| v.cmp(&literal)),
                (_, None) => None,
            };
            // Beyond i128 the float comparison is as exact as the data allows.
            exact.or_else(|| as_f64(kind, value)?.partial_cmp(&(literal as f64)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values of every kind, chosen to collide in their prefixes: shared 8-byte string
    /// starts, NUL bytes, equal top halves of decimals, extreme integers, signed zeros, NaN.
    fn pools() -> Vec<Vec<Value<'static>>> {
        let floats = [
            f64::NEG_INFINITY,
            -1.5,
            -0.0,
            0.0,
            1e-300,
            2.5,
            f64::MAX,
            f64::INFINITY,
            f64::NAN,
        ];
        vec![
            [i64::MIN, -1, 0, 1, i64::MAX].map(Value::Int).to_vec(),
            [0, 1, u64::MAX - 1, u64::MAX].map(Value::UInt).to_vec(),
            [i128::MIN, -1, 0, 1, 1 << 64, (1 << 64) + 1, i128::MAX]
                .map(Value::Decimal)
                .to_vec(),
            floats.map(|f| Value::Float(canonical(f))).to_vec(),
            [false, true].map(Value::Bool).to_vec(),
            [
                "",
                "a",
                "a ",
                "a b",
                "instrument",
                "instrument-1",
                "instrument-2",
                "zz",
            ]
            .map(Value::Str)
            .to_vec(),
        ]
        .into_iter()
        .map(|mut pool| {
            pool.push(Value::Null);
            pool
        })
        .collect()
    }

    #[test]
    fn a_smaller_prefix_never_contradicts_the_full_order() {
        for pool in pools() {
            for descending in [false, true] {
                for &a in &pool {
                    for &b in &pool {
                        if prefix(a, descending) < prefix(b, descending) {
                            assert_eq!(
                                compare(a, b, descending),
                                Ordering::Less,
                                "{a:?} vs {b:?}, descending {descending}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn negative_nan_and_negative_zero_read_canonical() {
        assert!(canonical(-f64::NAN).is_sign_positive());
        assert!(canonical(-0.0).is_sign_positive());
        assert_eq!(
            compare(
                Value::Float(canonical(-f64::NAN)),
                Value::Float(f64::INFINITY),
                false
            ),
            Ordering::Greater
        );
    }

    #[test]
    fn decimals_with_a_negative_scale_convert_exactly() {
        let kind = Kind::Decimal { scale: -2 };
        assert_eq!(as_f64(kind, Value::Decimal(3)), Some(300.0));
        let literal = Scalar::Float(300.0);
        assert_eq!(
            compare_literal(kind, Value::Decimal(3), &literal),
            Some(Ordering::Equal)
        );
    }
}
