//! Buckets: a group key that puts values in back-to-back ranges of one width (a minute of
//! timestamps, ten units of a number), each named by where it starts.

use std::sync::Arc;

use arrow_array::types::{
    Date32Type, Date64Type, Decimal128Type, DurationMicrosecondType, DurationMillisecondType,
    DurationNanosecondType, DurationSecondType, Float16Type, Float32Type, Float64Type, Int16Type,
    Int32Type, Int64Type, Int8Type, Time32MillisecondType, Time32SecondType, Time64MicrosecondType,
    Time64NanosecondType, TimestampMicrosecondType, TimestampMillisecondType,
    TimestampNanosecondType, TimestampSecondType, UInt16Type, UInt32Type, UInt64Type, UInt8Type,
};
use arrow_array::{ArrayRef, ArrowPrimitiveType, PrimitiveArray};
use arrow_schema::{DataType, TimeUnit};

use crate::value::{canonical, Kind, Value};
use crate::window::{reach, Reach, Span};

type F16 = <Float16Type as ArrowPrimitiveType>::Native;

/// A bucket width bound to its column.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bucket {
    width: Reach,
    /// Rounds a float start to the column's precision, so the start a group is keyed by is
    /// the one written in the column's type.
    narrow: fn(f64) -> f64,
    /// How close, relative to it, a float quotient must be to a whole number to be taken as
    /// it: two roundings of the column's type.
    near: f64,
}

impl Bucket {
    /// The bucket of `width` over a column of `data_type`, of `kind`: `None` unless the width
    /// fits the column and is at least one of its units (above zero and finite for floats).
    pub(crate) fn new(width: Span, data_type: &DataType, kind: Kind) -> Option<Self> {
        let width = reach(width, data_type, kind).filter(|w| match *w {
            Reach::Whole(units) => units >= 1,
            Reach::Float(width) => width > 0.0 && width.is_finite(),
        })?;
        let (narrow, epsilon): (fn(f64) -> f64, f64) = match data_type {
            // Lossless: an f32 or f16 converts to f64 exactly.
            DataType::Float32 => (|v| f64::from(v as f32), f64::from(f32::EPSILON)),
            DataType::Float16 => (|v| F16::from_f64(v).to_f64(), F16::EPSILON.to_f64()),
            _ => (|v| v, f64::EPSILON),
        };
        Some(Self {
            width,
            narrow,
            near: 2.0 * epsilon,
        })
    }

    /// Where the bucket of `value` starts: a decimal's unscaled integer for whole-unit
    /// columns (integers, decimals, times), a float for floats; null stays null, and so do
    /// NaN and the infinities, each a bucket of its own.
    pub(crate) fn start(&self, value: Value<'_>) -> Value<'static> {
        match (self.width, value) {
            (Reach::Whole(width), Value::Int(v)) => Value::Decimal(floor(i128::from(v), width)),
            (Reach::Whole(width), Value::UInt(v)) => Value::Decimal(floor(i128::from(v), width)),
            (Reach::Whole(width), Value::Decimal(v)) => Value::Decimal(floor(v, width)),
            (Reach::Float(width), Value::Float(v)) => {
                let quotient = v / width;
                let start = if quotient.is_finite() {
                    // Past f64 only for the outermost buckets of a width near f64::MAX: they
                    // start at the end of f64's range, as integers start at their type's.
                    let start = near_floor(quotient, self.near) * width;
                    start.clamp(-f64::MAX, f64::MAX)
                } else {
                    // NaN or an infinity, each a bucket of its own; or a width below the
                    // value's precision, where no two values share a bucket.
                    v
                };
                // `canonical`: a negative value too small for its quotient (it underflows to
                // -0.0) is a rounding below 0.0, whose bucket it joins.
                Value::Float(canonical((self.narrow)(start)))
            }
            _ => Value::Null,
        }
    }
}

/// The multiple of `width` (at least 1) at or below `value`.
fn floor(value: i128, width: i128) -> i128 {
    // Saturating: only the lowest bucket of a decimal near its limit can fall past i128.
    value.div_euclid(width).saturating_mul(width)
}

/// `quotient` rounded down, unless it is a rounding error below a whole number (within
/// `near` of it, relative to it, and [`MOST_SNAP`] at most): `0.3 / 0.1` is
/// 2.9999999999999996, and 0.3 belongs in the bucket that starts at 0.3.
fn near_floor(quotient: f64, near: f64) -> f64 {
    let nearest = quotient.round();
    if (quotient - nearest).abs() <= (near * nearest.abs()).min(MOST_SNAP) {
        nearest
    } else {
        quotient.floor()
    }
}

/// The most of a bucket a value below it may be and still count as in it: a relative
/// tolerance grows with the quotient, and past 2^47 (2^18 for f32, 2^5 for f16) it
/// would take in values that are no rounding error away.
const MOST_SNAP: f64 = 1.0 / 16.0;

/// A column of bucket starts of `data_type` (one that [`Bucket::new`] took), from
/// [`Bucket::start`]'s unscaled integers. A start below the type's range (the lowest
/// bucket of a narrow type, such as -200 for `Int8` buckets of 100) is the type's smallest
/// value.
pub(crate) fn whole_starts(data_type: &DataType, starts: &[Option<i128>]) -> ArrayRef {
    use DataType as D;
    use TimeUnit as U;
    let column = |f: fn(&DataType, &[Option<i128>]) -> ArrayRef| f(data_type, starts);
    match data_type {
        D::Int8 => column(integer::<Int8Type>),
        D::Int16 => column(integer::<Int16Type>),
        D::Int32 => column(integer::<Int32Type>),
        D::Int64 => column(integer::<Int64Type>),
        D::UInt8 => column(integer::<UInt8Type>),
        D::UInt16 => column(integer::<UInt16Type>),
        D::UInt32 => column(integer::<UInt32Type>),
        D::UInt64 => column(integer::<UInt64Type>),
        D::Decimal128(precision, _) => {
            let low = 1 - 10i128.pow(u32::from(*precision));
            primitive::<Decimal128Type>(data_type, starts, low)
        }
        D::Date32 => column(integer::<Date32Type>),
        D::Date64 => column(integer::<Date64Type>),
        D::Time32(U::Second) => column(integer::<Time32SecondType>),
        D::Time32(U::Millisecond) => column(integer::<Time32MillisecondType>),
        D::Time64(U::Microsecond) => column(integer::<Time64MicrosecondType>),
        D::Time64(U::Nanosecond) => column(integer::<Time64NanosecondType>),
        D::Timestamp(U::Second, _) => column(integer::<TimestampSecondType>),
        D::Timestamp(U::Millisecond, _) => column(integer::<TimestampMillisecondType>),
        D::Timestamp(U::Microsecond, _) => column(integer::<TimestampMicrosecondType>),
        D::Timestamp(U::Nanosecond, _) => column(integer::<TimestampNanosecondType>),
        D::Duration(U::Second) => column(integer::<DurationSecondType>),
        D::Duration(U::Millisecond) => column(integer::<DurationMillisecondType>),
        D::Duration(U::Microsecond) => column(integer::<DurationMicrosecondType>),
        D::Duration(U::Nanosecond) => column(integer::<DurationNanosecondType>),
        other => {
            debug_assert!(false, "{other} has no whole-unit buckets");
            arrow_array::new_null_array(other, starts.len())
        }
    }
}

/// [`primitive`], with the smallest value of a signed integer of `T`'s width as the low end
/// (unsigned starts are never below zero, so it never comes into play for them).
fn integer<T>(data_type: &DataType, starts: &[Option<i128>]) -> ArrayRef
where
    T: ArrowPrimitiveType,
    T::Native: TryFrom<i128>,
{
    let bits = 8 * std::mem::size_of::<T::Native>() - 1;
    primitive::<T>(data_type, starts, -(1i128 << bits))
}

fn primitive<T>(data_type: &DataType, starts: &[Option<i128>], low: i128) -> ArrayRef
where
    T: ArrowPrimitiveType,
    T::Native: TryFrom<i128>,
{
    // A start is at most a value of the column, so only the low end can be out of range.
    let values = starts
        .iter()
        .map(|s| s.and_then(|s| T::Native::try_from(s.max(low)).ok()));
    let array: PrimitiveArray<T> = values.collect();
    Arc::new(array.with_data_type(data_type.clone()))
}

/// A column of bucket starts of the float type `data_type`, from [`Bucket::start`]'s floats
/// (already of the column's precision).
pub(crate) fn float_starts(data_type: &DataType, starts: &[Option<f64>]) -> ArrayRef {
    let values = starts.iter().copied();
    match data_type {
        DataType::Float16 => {
            let array: PrimitiveArray<Float16Type> = values.map(|s| s.map(F16::from_f64)).collect();
            Arc::new(array)
        }
        DataType::Float32 => {
            // Exact: the start has an f32's precision.
            let array: PrimitiveArray<Float32Type> = values.map(|s| s.map(|v| v as f32)).collect();
            Arc::new(array)
        }
        _ => Arc::new(values.collect::<PrimitiveArray<Float64Type>>()),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn bucket(width: Span, data_type: &DataType, kind: Kind) -> Bucket {
        Bucket::new(width, data_type, kind).unwrap()
    }

    #[test]
    fn whole_buckets_start_at_or_below_their_values() {
        let minute = bucket(
            Span::Duration(Duration::from_secs(60)),
            &DataType::Timestamp(TimeUnit::Millisecond, None),
            Kind::Int,
        );
        let start = |v| minute.start(Value::Int(v));
        assert_eq!(start(0), Value::Decimal(0));
        assert_eq!(start(59_999), Value::Decimal(0));
        assert_eq!(start(60_000), Value::Decimal(60_000));
        assert_eq!(start(-1), Value::Decimal(-60_000));
        assert_eq!(start(i64::MIN), Value::Decimal(-9_223_372_036_854_780_000));
        assert_eq!(minute.start(Value::Null), Value::Null);
    }

    #[test]
    fn a_width_must_be_a_unit_at_least() {
        let millis = DataType::Timestamp(TimeUnit::Millisecond, None);
        let half = Span::Duration(Duration::from_micros(500));
        assert!(Bucket::new(half, &millis, Kind::Int).is_none());
        assert!(Bucket::new(Span::Value(0.5), &DataType::Int64, Kind::Int).is_none());
        assert!(Bucket::new(Span::Value(1.0), &DataType::Int64, Kind::Int).is_some());
        let cents = DataType::Decimal128(10, 2);
        let decimal = Kind::Decimal { scale: 2 };
        assert!(Bucket::new(Span::Value(0.01), &cents, decimal).is_some());
        assert!(Bucket::new(Span::Value(0.001), &cents, decimal).is_none());
        for width in [0.0, -1.0, f64::INFINITY, f64::NAN] {
            let none = Bucket::new(Span::Value(width), &DataType::Float64, Kind::Float);
            assert!(none.is_none(), "{width}");
        }
    }

    #[test]
    fn float_buckets_forgive_rounding_but_not_more() {
        let tenth = bucket(Span::Value(0.1), &DataType::Float64, Kind::Float);
        let start = |v| match tenth.start(Value::Float(v)) {
            Value::Float(start) => start,
            other => panic!("{other:?}"),
        };
        assert_eq!(start(0.3), 3.0 * 0.1);
        assert_eq!(start(0.25), 2.0 * 0.1);
        assert_eq!(start(-0.05), -0.1);
        assert_eq!(start(-0.0), 0.0);
        let one = bucket(Span::Value(1.0), &DataType::Float64, Kind::Float);
        let start = |v| one.start(Value::Float(v));
        assert_eq!(
            start(3.0 - 1e-15),
            Value::Float(3.0),
            "within two roundings"
        );
        assert_eq!(
            start(3.0 - 1e-14),
            Value::Float(2.0),
            "more than a few roundings"
        );
        let wide = bucket(Span::Value(1e300), &DataType::Float64, Kind::Float);
        let Value::Float(start) = wide.start(Value::Float(-1e-300)) else {
            panic!("a float start");
        };
        assert_eq!(start.to_bits(), 0.0f64.to_bits(), "0.0, not -0.0");
        let one = bucket(Span::Value(1.0), &DataType::Float64, Kind::Float);
        let start = |v| one.start(Value::Float(v));
        assert_eq!(start(1e15 + 0.5), Value::Float(1e15));
        assert_eq!(
            start(1e15 + 0.875),
            Value::Float(1e15),
            "an eighth below 1e15 + 1"
        );
        assert_eq!(
            start(1e15 + 0.9375),
            Value::Float(1e15 + 1.0),
            "a sixteenth below"
        );
        assert_eq!(start(f64::MAX), Value::Float(f64::MAX));
        assert!(matches!(start(f64::NAN), Value::Float(v) if v.is_nan()));
        assert_eq!(start(f64::NEG_INFINITY), Value::Float(f64::NEG_INFINITY));
        let tiny = bucket(Span::Value(1e-300), &DataType::Float64, Kind::Float);
        assert_eq!(tiny.start(Value::Float(1e300)), Value::Float(1e300));
        let huge = bucket(Span::Value(0.7 * f64::MAX), &DataType::Float64, Kind::Float);
        for v in [-f64::MAX, -0.8 * f64::MAX] {
            assert_eq!(huge.start(Value::Float(v)), Value::Float(-f64::MAX), "{v}");
        }
        assert_eq!(
            huge.start(Value::Float(f64::MAX)),
            Value::Float(0.7 * f64::MAX)
        );
    }

    #[test]
    fn narrow_floats_forgive_their_own_rounding() {
        let tenth = |data_type| bucket(Span::Value(0.1), &data_type, Kind::Float);
        let f32_tenth = tenth(DataType::Float32);
        // 0.7f32 is 0.699999988..., a tenth of an f32's rounding below 0.7.
        let start = f32_tenth.start(Value::Float(f64::from(0.7f32)));
        assert_eq!(start, Value::Float(f64::from(0.7f32)));
        let f16_tenth = tenth(DataType::Float16);
        let nine = F16::from_f64(0.9).to_f64(); // 0.89990234375
        assert_eq!(f16_tenth.start(Value::Float(nine)), Value::Float(nine));
        let below = F16::from_f64(0.69).to_f64();
        assert_eq!(
            f16_tenth.start(Value::Float(below)),
            Value::Float(F16::from_f64(0.6).to_f64())
        );
        // f64 data is held to an f64's rounding.
        let start = tenth(DataType::Float64).start(Value::Float(f64::from(0.7f32)));
        assert_eq!(start, Value::Float(6.0 * 0.1));
    }

    #[test]
    fn narrow_floats_start_at_a_value_of_their_type() {
        let third = bucket(Span::Value(1.0 / 3.0), &DataType::Float32, Kind::Float);
        let Value::Float(start) = third.start(Value::Float(1.0)) else {
            panic!("a float start");
        };
        assert_eq!(f64::from(start as f32), start);
    }

    #[test]
    fn starts_below_a_narrow_type_are_its_smallest_value() {
        let starts = whole_starts(&DataType::Int8, &[Some(-200), Some(100), None]);
        let starts = starts
            .as_any()
            .downcast_ref::<arrow_array::Int8Array>()
            .unwrap();
        assert_eq!(
            starts.iter().collect::<Vec<_>>(),
            [Some(-128), Some(100), None]
        );
        let decimals = whole_starts(&DataType::Decimal128(3, 1), &[Some(-2_000)]);
        let decimals = decimals
            .as_any()
            .downcast_ref::<arrow_array::Decimal128Array>();
        assert_eq!(decimals.unwrap().value(0), -999);
    }
}
