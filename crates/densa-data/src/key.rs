//! Row keys: integers or strings, read from any integer or string Arrow layout.

use arrow_array::cast::AsArray;
use arrow_array::types::{
    Int16Type, Int32Type, Int64Type, Int8Type, UInt16Type, UInt32Type, UInt64Type, UInt8Type,
};
use arrow_array::{Array, ArrowPrimitiveType, OffsetSizeTrait};
use arrow_schema::DataType;
use rustc_hash::FxHashMap;

use crate::RowId;

/// A key borrowed from a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum KeyRef<'a> {
    Int(i64),
    UInt(u64),
    Str(&'a str),
}

/// Reads the key at a row; `None` for a null key.
pub(crate) type KeyFn = for<'a> fn(&'a dyn Array, usize) -> Option<KeyRef<'a>>;

/// The reader for keys of `data_type`; `None` if it cannot be a key.
pub(crate) fn key_reader(data_type: &DataType) -> Option<KeyFn> {
    use DataType as D;
    Some(match data_type {
        D::Int8 => signed::<Int8Type>,
        D::Int16 => signed::<Int16Type>,
        D::Int32 => signed::<Int32Type>,
        D::Int64 => signed::<Int64Type>,
        D::UInt8 => unsigned::<UInt8Type>,
        D::UInt16 => unsigned::<UInt16Type>,
        D::UInt32 => unsigned::<UInt32Type>,
        D::UInt64 => unsigned::<UInt64Type>,
        D::Utf8 => string::<i32>,
        D::LargeUtf8 => string::<i64>,
        D::Utf8View => string_view,
        _ => return None,
    })
}

fn signed<T>(array: &dyn Array, row: usize) -> Option<KeyRef<'_>>
where
    T: ArrowPrimitiveType,
    T::Native: Into<i64>,
{
    let array = array.as_primitive_opt::<T>()?;
    array
        .is_valid(row)
        .then(|| KeyRef::Int(array.value(row).into()))
}

fn unsigned<T>(array: &dyn Array, row: usize) -> Option<KeyRef<'_>>
where
    T: ArrowPrimitiveType,
    T::Native: Into<u64>,
{
    let array = array.as_primitive_opt::<T>()?;
    array
        .is_valid(row)
        .then(|| KeyRef::UInt(array.value(row).into()))
}

fn string<O: OffsetSizeTrait>(array: &dyn Array, row: usize) -> Option<KeyRef<'_>> {
    let array = array.as_string_opt::<O>()?;
    array.is_valid(row).then(|| KeyRef::Str(array.value(row)))
}

fn string_view(array: &dyn Array, row: usize) -> Option<KeyRef<'_>> {
    let array = array.as_string_view_opt()?;
    array.is_valid(row).then(|| KeyRef::Str(array.value(row)))
}

/// Every live key and the row id holding it. Owned string keys are allocated once per new
/// key; lookups borrow.
#[derive(Debug, Clone)]
pub(crate) enum KeyIndex {
    Int(FxHashMap<i64, RowId>),
    UInt(FxHashMap<u64, RowId>),
    Str(FxHashMap<Box<str>, RowId>),
}

impl KeyIndex {
    /// An empty index for keys of `data_type` (as accepted by [`key_reader`]).
    pub(crate) fn for_type(data_type: &DataType) -> Option<Self> {
        use DataType as D;
        Some(match data_type {
            D::Int8 | D::Int16 | D::Int32 | D::Int64 => Self::Int(FxHashMap::default()),
            D::UInt8 | D::UInt16 | D::UInt32 | D::UInt64 => Self::UInt(FxHashMap::default()),
            D::Utf8 | D::LargeUtf8 | D::Utf8View => Self::Str(FxHashMap::default()),
            _ => return None,
        })
    }

    /// Whether keys of `data_type` are of this index's kind.
    pub(crate) fn same_kind(&self, data_type: &DataType) -> bool {
        matches!(
            (self, Self::for_type(data_type)),
            (Self::Int(_), Some(Self::Int(_)))
                | (Self::UInt(_), Some(Self::UInt(_)))
                | (Self::Str(_), Some(Self::Str(_)))
        )
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Int(map) => map.len(),
            Self::UInt(map) => map.len(),
            Self::Str(map) => map.len(),
        }
    }

    pub(crate) fn get(&self, key: KeyRef<'_>) -> Option<RowId> {
        match (self, key) {
            (Self::Int(map), KeyRef::Int(k)) => map.get(&k).copied(),
            (Self::UInt(map), KeyRef::UInt(k)) => map.get(&k).copied(),
            (Self::Str(map), KeyRef::Str(k)) => map.get(k).copied(),
            _ => None,
        }
    }

    /// Records `key` at `id`. Keys come from the key column, so they are of this index's
    /// kind.
    pub(crate) fn insert(&mut self, key: KeyRef<'_>, id: RowId) {
        match (self, key) {
            (Self::Int(map), KeyRef::Int(k)) => {
                map.insert(k, id);
            }
            (Self::UInt(map), KeyRef::UInt(k)) => {
                map.insert(k, id);
            }
            (Self::Str(map), KeyRef::Str(k)) => {
                map.insert(k.into(), id);
            }
            _ => debug_assert!(false, "key kind checked before inserting"),
        }
    }

    pub(crate) fn remove(&mut self, key: KeyRef<'_>) -> Option<RowId> {
        match (self, key) {
            (Self::Int(map), KeyRef::Int(k)) => map.remove(&k),
            (Self::UInt(map), KeyRef::UInt(k)) => map.remove(&k),
            (Self::Str(map), KeyRef::Str(k)) => map.remove(k),
            _ => None,
        }
    }
}
