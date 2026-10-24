use std::fmt;

/// Why a table or view operation was refused. Nothing changes when one is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataError {
    /// No column has this name.
    UnknownColumn(String),
    /// The key column must hold integers or strings.
    UnsupportedKey(String),
    /// Views and groups cannot read this column's data type (lists, structs, binary, ...).
    UnsupportedColumn(String),
    /// A filter's value or operation does not fit this column's type (text compared with a
    /// number, `contains` on a number).
    LiteralMismatch(String),
    /// Sums and means need an integer, decimal or float column; this one holds something
    /// else (text, booleans, dates, times).
    NotNumeric(String),
    /// A window's span does not fit its column: a `Duration` goes with dates, times and
    /// timestamps, a number (zero or more) with integers, decimals and floats.
    WindowSpan(String),
    /// A group's bucket width does not fit its column: a `Duration` of at least one of its
    /// units goes with dates, times and timestamps, a number of at least one unit with
    /// integers and decimals, a finite number above zero with floats.
    BucketWidth(String),
    /// Two columns of a group spec's result would have this name.
    DuplicateName(String),
    /// The batch has a different number of columns.
    ColumnCount {
        /// Columns in the table.
        expected: usize,
        /// Columns in the batch.
        found: usize,
    },
    /// The batch's column at this index has another name or data type.
    ColumnMismatch {
        /// Index of the first differing column.
        column: usize,
    },
    /// A row of the batch has no key.
    NullKey {
        /// The row, within the batch.
        row: usize,
    },
    /// Keys to delete or find are not of the key column's kind (text for a number key).
    KeyType,
    /// More than 2^32 live keys, or rows in one batch.
    TooManyRows,
    /// Arrow could not build a batch: compacting a table, or assembling groups.
    Arrow(String),
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownColumn(name) => write!(
                f,
                "there is no column named {name:?}: names match exactly, case included \
                 (the table's schema lists them)"
            ),
            Self::UnsupportedKey(name) => {
                write!(f, "key column {name:?} must hold integers or strings")
            }
            Self::UnsupportedColumn(name) => {
                write!(
                    f,
                    "views and groups cannot read column {name:?} of this type: they read \
                     booleans, integers, floats, decimals, text (or dictionaries of text), dates, \
                     times, timestamps and durations"
                )
            }
            Self::LiteralMismatch(name) => {
                write!(f, "the filter does not fit the type of column {name:?}")
            }
            Self::NotNumeric(name) => {
                write!(
                    f,
                    "column {name:?} is not numeric, so it has no sum or mean: those take \
                     integers, decimals and floats (count takes no column; min and max take any column \
                     views can sort)"
                )
            }
            Self::WindowSpan(name) => {
                write!(
                    f,
                    "the window's span does not fit the type of column {name:?}: \
                     Window::duration goes with dates, times, timestamps and durations, \
                     Window::span (zero or more) with integers, decimals and floats"
                )
            }
            Self::BucketWidth(name) => {
                write!(
                    f,
                    "the bucket width does not fit the type of column {name:?}: \
                     group_by_time takes at least one of the column's units for dates, times, \
                     timestamps and durations, group_by_step at least one unit for integers \
                     and decimals, or a finite step above zero for floats"
                )
            }
            Self::DuplicateName(name) => write!(
                f,
                "two result columns are named {name:?}: give an aggregate another name with \
                 Aggregate::named"
            ),
            Self::ColumnCount { expected, found } => {
                write!(f, "the batch has {found} columns, the table {expected}")
            }
            Self::ColumnMismatch { column } => {
                write!(
                    f,
                    "column {column} of the batch has another name or data type than the \
                     table's: every batch must have the table's schema (LiveTable::schema)"
                )
            }
            Self::NullKey { row } => write!(f, "row {row} of the batch has a null key"),
            Self::KeyType => f.write_str(
                "the keys are not of the key column's kind: give integers for a key column of \
                 integers, strings for one of strings",
            ),
            Self::TooManyRows => f.write_str("more than 2^32 live keys, or rows in one batch"),
            Self::Arrow(message) => write!(f, "Arrow could not build a batch: {message}"),
        }
    }
}

impl std::error::Error for DataError {}

impl DataError {
    pub(crate) fn arrow(error: arrow_schema::ArrowError) -> Self {
        Self::Arrow(error.to_string())
    }
}
