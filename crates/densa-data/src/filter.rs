//! Row filters for views, with SQL's three-valued logic: a comparison with a null is
//! unknown, and only rows whose filter is true are shown.

use std::cmp::Ordering;

use arrow_array::{Array, RecordBatch};
use arrow_schema::{DataType, Schema};

use crate::value::{compare_literal, literal_fits, value_reader, Kind, Value, ValueFn};
use crate::DataError;

/// A literal value a column is compared with.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Scalar {
    /// A signed integer; compares with any numeric column (decimals at their scale).
    Int(i64),
    /// An unsigned integer; compares with any numeric column.
    UInt(u64),
    /// A float; compares with any numeric column.
    Float(f64),
    /// A boolean.
    Bool(bool),
    /// Text; compares with string and dictionary-of-string columns.
    Str(String),
}

/// How a column compares with a literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CompareOp {
    /// Equal.
    Eq,
    /// Not equal. Like every comparison, unknown for null and for NaN.
    Ne,
    /// Less than.
    Lt,
    /// Less than or equal.
    Le,
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Ge,
}

impl CompareOp {
    fn holds(self, order: Ordering) -> bool {
        match self {
            Self::Eq => order.is_eq(),
            Self::Ne => order.is_ne(),
            Self::Lt => order.is_lt(),
            Self::Le => order.is_le(),
            Self::Gt => order.is_gt(),
            Self::Ge => order.is_ge(),
        }
    }
}

/// Which rows a view shows. Dates, times, timestamps and durations compare as their integer
/// count of units (milliseconds since the epoch for `Timestamp(Millisecond)`).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Filter {
    /// `column op value`; unknown when the column is null.
    Compare {
        /// Column name.
        column: String,
        /// Comparison.
        op: CompareOp,
        /// Literal.
        value: Scalar,
    },
    /// The column is null. Works on columns of any type.
    IsNull(String),
    /// The string column contains `text` (case-sensitive); unknown when null.
    Contains {
        /// Column name.
        column: String,
        /// Text searched for.
        text: String,
    },
    /// Every filter holds (true for none).
    All(Vec<Filter>),
    /// Some filter holds (false for none).
    Any(Vec<Filter>),
    /// The filter does not hold (unknown stays unknown).
    Not(Box<Filter>),
}

impl Filter {
    /// `column op value`.
    pub fn compare(column: impl Into<String>, op: CompareOp, value: Scalar) -> Self {
        Self::Compare {
            column: column.into(),
            op,
            value,
        }
    }

    /// The column is null.
    pub fn is_null(column: impl Into<String>) -> Self {
        Self::IsNull(column.into())
    }

    /// The column is not null.
    pub fn is_not_null(column: impl Into<String>) -> Self {
        Self::not(Self::is_null(column))
    }

    /// The string column contains `text`, case included: `"AAPL"` contains `"AA"` but not
    /// `"aa"`. To ignore case, keep the column in one case and change the text to match.
    pub fn contains(column: impl Into<String>, text: impl Into<String>) -> Self {
        Self::Contains {
            column: column.into(),
            text: text.into(),
        }
    }

    /// Every one of `filters` holds.
    pub fn all(filters: impl IntoIterator<Item = Filter>) -> Self {
        Self::All(filters.into_iter().collect())
    }

    /// At least one of `filters` holds.
    pub fn any(filters: impl IntoIterator<Item = Filter>) -> Self {
        Self::Any(filters.into_iter().collect())
    }

    /// `filter` does not hold.
    #[expect(
        clippy::should_implement_trait,
        reason = "a constructor, not negation of a value"
    )]
    pub fn not(filter: Filter) -> Self {
        Self::Not(Box::new(filter))
    }
}

/// A filter bound to column indices and readers.
#[derive(Debug, Clone)]
pub(crate) enum Predicate {
    Compare {
        column: usize,
        kind: Kind,
        read: ValueFn,
        op: CompareOp,
        value: Scalar,
    },
    IsNull {
        column: usize,
        /// The value reader, for types views read (dictionaries: a null value counts);
        /// other types use the array's own nulls.
        read: Option<ValueFn>,
    },
    Contains {
        column: usize,
        read: ValueFn,
        text: String,
    },
    All(Vec<Predicate>),
    Any(Vec<Predicate>),
    Not(Box<Predicate>),
}

/// Resolves column `name` in `schema` to its index, kind and reader.
pub(crate) fn resolve(schema: &Schema, name: &str) -> Result<(usize, Kind, ValueFn), DataError> {
    let (index, field) = schema
        .column_with_name(name)
        .ok_or_else(|| DataError::UnknownColumn(name.to_owned()))?;
    let (kind, read) = value_reader(field.data_type())
        .ok_or_else(|| DataError::UnsupportedColumn(name.to_owned()))?;
    Ok((index, kind, read))
}

impl Predicate {
    pub(crate) fn compile(filter: &Filter, schema: &Schema) -> Result<Self, DataError> {
        Ok(match filter {
            Filter::Compare { column, op, value } => {
                let (index, kind, read) = resolve(schema, column)?;
                if !literal_fits(kind, value) {
                    return Err(DataError::LiteralMismatch(column.clone()));
                }
                Self::Compare {
                    column: index,
                    kind,
                    read,
                    op: *op,
                    value: value.clone(),
                }
            }
            Filter::IsNull(column) => {
                let (index, field) = schema
                    .column_with_name(column)
                    .ok_or_else(|| DataError::UnknownColumn(column.clone()))?;
                Self::IsNull {
                    column: index,
                    read: value_reader(field.data_type()).map(|(_, read)| read),
                }
            }
            Filter::Contains { column, text } => {
                let (index, kind, read) = resolve(schema, column)?;
                if kind != Kind::Str {
                    return Err(DataError::LiteralMismatch(column.clone()));
                }
                Self::Contains {
                    column: index,
                    read,
                    text: text.clone(),
                }
            }
            Filter::All(filters) => Self::All(compile_all(filters, schema)?),
            Filter::Any(filters) => Self::Any(compile_all(filters, schema)?),
            Filter::Not(filter) => Self::Not(Box::new(Self::compile(filter, schema)?)),
        })
    }

    /// The filter's value at `row` of `batch`: `None` for unknown.
    pub(crate) fn eval(&self, batch: &RecordBatch, row: usize) -> Option<bool> {
        let value = |column: usize, read: ValueFn| read(batch.column(column).as_ref(), row);
        match self {
            Self::Compare {
                column,
                kind,
                read,
                op,
                value: literal,
            } => compare_literal(*kind, value(*column, *read), literal).map(|o| op.holds(o)),
            Self::IsNull { column, read } => Some(match read {
                Some(read) => value(*column, *read) == Value::Null,
                None => {
                    let array = batch.column(*column);
                    // A `Null` column has no validity buffer: every value is null.
                    *array.data_type() == DataType::Null || array.is_null(row)
                }
            }),
            Self::Contains { column, read, text } => match value(*column, *read) {
                Value::Str(s) => Some(s.contains(text.as_str())),
                _ => None,
            },
            Self::All(predicates) => kleene_all(predicates, batch, row),
            Self::Any(predicates) => kleene_any(predicates, batch, row),
            Self::Not(predicate) => predicate.eval(batch, row).map(|b| !b),
        }
    }
}

fn compile_all(filters: &[Filter], schema: &Schema) -> Result<Vec<Predicate>, DataError> {
    filters
        .iter()
        .map(|f| Predicate::compile(f, schema))
        .collect()
}

/// False if any is false, else unknown if any is unknown, else true.
fn kleene_all(predicates: &[Predicate], batch: &RecordBatch, row: usize) -> Option<bool> {
    let mut unknown = false;
    for predicate in predicates {
        match predicate.eval(batch, row) {
            Some(false) => return Some(false),
            None => unknown = true,
            Some(true) => {}
        }
    }
    (!unknown).then_some(true)
}

/// True if any is true, else unknown if any is unknown, else false.
fn kleene_any(predicates: &[Predicate], batch: &RecordBatch, row: usize) -> Option<bool> {
    let mut unknown = false;
    for predicate in predicates {
        match predicate.eval(batch, row) {
            Some(true) => return Some(true),
            None => unknown = true,
            Some(false) => {}
        }
    }
    (!unknown).then_some(false)
}
