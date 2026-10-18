use std::fmt;

use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use densa_grid::{Align, CellStyle, CellText, Color, Column, GridSource, Palette, CELL_BYTES};

use crate::reader::{is_numeric, Format, Reader};

/// Rows of the first batch measured to size the columns.
const SAMPLE_ROWS: usize = 1_000;
/// Widest a column is sized from its content; longer text is cut with `…`.
const MAX_SAMPLED_CHARS: usize = 40;
/// Width of a column with no rows yet, unless its title is wider.
const DEFAULT_CHARS: usize = 8;

/// What [`ArrowTable::write_cell`] wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CellState {
    /// The value.
    Value,
    /// `null`, for a missing value.
    Null,
    /// `?`, for a type this crate does not format (lists, structs, maps, ...).
    Unsupported,
    /// `#`, for a value that cannot be shown: longer than a cell holds (as `1e300` without
    /// fixed decimals), or invalid (a decimal scale beyond 38, a time of day outside a day).
    Invalid,
    /// Nothing: the row or column does not exist.
    OutOfRange,
}

/// Why a batch could not be added, or a column setting applied.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TableError {
    /// No batch was given, so there is no schema.
    NoBatches,
    /// The batch has a different number of columns.
    ColumnCount {
        /// Columns in the table.
        expected: usize,
        /// Columns in the batch.
        found: usize,
    },
    /// The batch's column at this index has another data type.
    ColumnMismatch {
        /// Index of the first differing column.
        column: usize,
    },
    /// There is no column at this index.
    NoSuchColumn(usize),
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBatches => f.write_str("no record batch was given, so there is no schema"),
            Self::ColumnCount { expected, found } => {
                write!(f, "the batch has {found} columns, the table {expected}")
            }
            Self::ColumnMismatch { column } => {
                write!(f, "column {column} of the batch has another data type")
            }
            Self::NoSuchColumn(column) => write!(f, "there is no column {column}"),
        }
    }
}

impl std::error::Error for TableError {}

/// Arrow record batches shown as one table: rows run through the batches in order.
///
/// Formatting is chosen per column from its data type when a batch is added, so writing a
/// cell is a lookup and a format into a stack buffer, without allocating. Batches can be
/// appended while the table is shown (a stream); their arrays are shared, not copied.
///
/// Formatted types: booleans, integers, floats, `Decimal128`, strings (`Utf8`, `LargeUtf8`,
/// `Utf8View`), dictionaries of those, dates, times, timestamps (with a time zone: in UTC,
/// marked `Z`) and durations. Other types (lists, structs, maps, binary, intervals) show
/// `?`.
///
/// A batch is accepted when its columns have the table's data types in order; field names,
/// nullability and metadata may differ. A timestamp's time zone is part of its data type.
#[derive(Debug, Clone)]
pub struct ArrowTable {
    schema: SchemaRef,
    /// Each non-empty batch's columns.
    batches: Vec<Vec<Reader>>,
    /// First row of each batch in `batches`.
    starts: Vec<usize>,
    rows: usize,
    formats: Vec<Format>,
    /// Whether the columns have been sized from a batch's content.
    sized: bool,
}

impl ArrowTable {
    /// An empty table for batches with `schema`; columns are sized by their titles until
    /// the first rows arrive.
    pub fn new(schema: SchemaRef) -> Self {
        let formats = schema
            .fields()
            .iter()
            .map(|field| Format {
                decimals: None,
                max_chars: field.name().chars().count().max(DEFAULT_CHARS),
                width_set: false,
            })
            .collect();
        Self {
            schema,
            batches: Vec::new(),
            starts: Vec::new(),
            rows: 0,
            formats,
            sized: false,
        }
    }

    /// A table of `batches`, with the first batch's schema.
    ///
    /// # Errors
    /// [`TableError::NoBatches`] for none; a column error if a batch's schema differs.
    pub fn from_batches(
        batches: impl IntoIterator<Item = RecordBatch>,
    ) -> Result<Self, TableError> {
        let mut batches = batches.into_iter();
        let first = batches.next().ok_or(TableError::NoBatches)?;
        let mut table = Self::new(first.schema());
        for batch in std::iter::once(first).chain(batches) {
            table.push(batch)?;
        }
        Ok(table)
    }

    /// Appends `batch`'s rows. The first rows to arrive size the columns (their titles and
    /// up to the first 1,000 values, at most 40 characters), except columns sized with
    /// [`set_column_chars`](Self::set_column_chars).
    ///
    /// # Errors
    /// A column error if `batch` has other columns or data types; the table is unchanged.
    pub fn push(&mut self, batch: RecordBatch) -> Result<(), TableError> {
        self.check(&batch)?;
        if batch.num_rows() == 0 {
            return Ok(());
        }
        let readers: Vec<Reader> = batch.columns().iter().cloned().map(Reader::new).collect();
        if !self.sized {
            self.size_columns(&readers, batch.num_rows());
        }
        self.starts.push(self.rows);
        self.rows += batch.num_rows();
        self.batches.push(readers);
        Ok(())
    }

    /// The table's schema.
    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    /// Rows in all batches.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Grid columns: titled by field name, sized by content, numbers and times aligned right.
    pub fn columns(&self) -> Vec<Column> {
        self.schema
            .fields()
            .iter()
            .zip(&self.formats)
            .map(|(field, format)| {
                let align = if is_numeric(field.data_type()) {
                    Align::Right
                } else {
                    Align::Left
                };
                let chars = u32::try_from(format.max_chars).unwrap_or(u32::MAX);
                Column::new(field.name().clone(), chars, align)
            })
            .collect()
    }

    /// Prints floating-point values of `column` with `decimals` fixed decimal places
    /// (`None`: the shortest exact form).
    ///
    /// # Errors
    /// [`TableError::NoSuchColumn`] if there is no such column.
    pub fn set_decimals(
        &mut self,
        column: usize,
        decimals: Option<usize>,
    ) -> Result<(), TableError> {
        self.format_mut(column)?.decimals = decimals;
        Ok(())
    }

    /// Sets `column` to `chars` characters wide (1 to [`CELL_BYTES`], the most a cell can
    /// hold): [`columns`](Self::columns) reports it, longer text is cut to fit ending in
    /// `…`, and the first rows to arrive no longer size it.
    ///
    /// # Errors
    /// [`TableError::NoSuchColumn`] if there is no such column.
    pub fn set_column_chars(&mut self, column: usize, chars: usize) -> Result<(), TableError> {
        let format = self.format_mut(column)?;
        format.max_chars = chars.clamp(1, CELL_BYTES);
        format.width_set = true;
        Ok(())
    }

    /// Appends the value at `row` of `column` to `text`, or a placeholder (see
    /// [`CellState`]), never a partial value. Never allocates.
    pub fn write_cell(&self, row: usize, column: usize, text: &mut CellText) -> CellState {
        if row >= self.rows {
            return CellState::OutOfRange;
        }
        // At least one start is <= row, since row < rows and starts[0] == 0.
        let batch = self.starts.partition_point(|&start| start <= row) - 1;
        self.write_at(batch, row - self.starts[batch], column, text)
    }

    /// [`write_cell`](Self::write_cell) for row `local` of stored batch `batch` (the batches
    /// with rows, in the order pushed).
    pub(crate) fn write_at(
        &self,
        batch: usize,
        local: usize,
        column: usize,
        text: &mut CellText,
    ) -> CellState {
        let reader = self.batches.get(batch).and_then(|b| b.get(column));
        let (Some(reader), Some(format)) = (reader, self.formats.get(column)) else {
            return CellState::OutOfRange;
        };
        if local >= reader.len() {
            return CellState::OutOfRange;
        }
        if reader.is_null(local) {
            // A cell always has room for `null`.
            let _ = text.push_str("null");
            return CellState::Null;
        }
        if !reader.is_supported() {
            reader.write(local, text, format);
            return CellState::Unsupported;
        }
        let mark = text.len();
        if reader.write(local, text, format) {
            return CellState::Value;
        }
        text.truncate(mark);
        // A cell always has room for one byte.
        let _ = text.push_str("#");
        CellState::Invalid
    }

    /// A [`GridSource`] showing the table from `first_row` (scrolled), in `palette`'s
    /// text color, with nulls and unsupported values dimmed.
    pub fn view<'a>(&'a self, first_row: usize, palette: &Palette) -> ArrowGrid<'a> {
        ArrowGrid {
            table: self,
            first_row,
            text: palette.text,
            dim: palette.dim_text,
        }
    }

    fn check(&self, batch: &RecordBatch) -> Result<(), TableError> {
        let ours = self.schema.fields();
        let theirs = batch.schema_ref().fields();
        if ours.len() != theirs.len() {
            return Err(TableError::ColumnCount {
                expected: ours.len(),
                found: theirs.len(),
            });
        }
        match ours
            .iter()
            .zip(theirs.iter())
            .position(|(a, b)| a.data_type() != b.data_type())
        {
            Some(column) => Err(TableError::ColumnMismatch { column }),
            None => Ok(()),
        }
    }

    /// Widens each column to its longest value among the first rows (and its title).
    fn size_columns(&mut self, readers: &[Reader], rows: usize) {
        let mut text = CellText::new();
        for ((reader, format), field) in readers
            .iter()
            .zip(&mut self.formats)
            .zip(self.schema.fields().iter())
        {
            if format.width_set {
                continue;
            }
            let sampling = Format {
                max_chars: MAX_SAMPLED_CHARS,
                ..*format
            };
            let mut widest = field.name().chars().count();
            for row in 0..rows.min(SAMPLE_ROWS) {
                text.clear();
                if !reader.is_null(row) {
                    reader.write(row, &mut text, &sampling);
                }
                widest = widest.max(text.as_str().chars().count());
            }
            format.max_chars = widest.max(1);
        }
        self.sized = true;
    }

    /// Batches with rows held, in the order pushed.
    pub(crate) fn stored_batches(&self) -> usize {
        self.batches.len()
    }

    /// Drops every batch, keeping the schema and the column formats and widths.
    pub(crate) fn clear_batches(&mut self) {
        self.batches.clear();
        self.starts.clear();
        self.rows = 0;
    }

    fn format_mut(&mut self, column: usize) -> Result<&mut Format, TableError> {
        self.formats
            .get_mut(column)
            .ok_or(TableError::NoSuchColumn(column))
    }
}

/// An [`ArrowTable`] as a grid's data, scrolled to a first row. Cheap to create each frame.
#[derive(Debug, Clone, Copy)]
pub struct ArrowGrid<'a> {
    table: &'a ArrowTable,
    first_row: usize,
    text: Color,
    dim: Color,
}

impl GridSource for ArrowGrid<'_> {
    fn rows(&self) -> usize {
        self.table.rows().saturating_sub(self.first_row)
    }

    fn cell(&self, row: usize, column: usize, text: &mut CellText) -> CellStyle {
        let state = self
            .table
            .write_cell(self.first_row.saturating_add(row), column, text);
        let color = if state == CellState::Value {
            self.text
        } else {
            self.dim
        };
        CellStyle {
            color,
            background: None,
        }
    }
}
