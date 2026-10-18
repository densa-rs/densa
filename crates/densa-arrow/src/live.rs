use densa_data::{LiveTable, RowRef, View, ViewId};
use densa_grid::{CellStyle, CellText, Color, Column, GridSource, Palette};

use crate::table::{ArrowTable, CellState, TableError};

/// Shows views of a `densa_data::LiveTable` in the grid, formatted like an [`ArrowTable`].
///
/// It keeps a formatter for each of the live table's batches; [`view`](Self::view) brings them
/// up to date first, which allocates only when batches were added or compacted since the
/// last call. Columns are sized from the first rows, as in [`ArrowTable`]. Given another
/// table (or a clone), it rebuilds its formatters; for a table with another schema it also
/// resets its column settings.
#[derive(Debug, Clone)]
pub struct LiveGrid {
    mirror: ArrowTable,
    /// The table object (`LiveTable::instance`) and generation the formatters were built for.
    table: (u64, u64),
}

impl LiveGrid {
    /// A grid for views of `table`, with formatters for its current batches.
    pub fn new(table: &LiveTable) -> Self {
        let mut grid = Self {
            mirror: ArrowTable::new(table.schema().clone()),
            table: (table.instance(), table.generation()),
        };
        grid.sync(table);
        grid
    }

    /// Grid columns, as [`ArrowTable::columns`].
    pub fn columns(&self) -> Vec<Column> {
        self.mirror.columns()
    }

    /// As [`ArrowTable::set_decimals`].
    ///
    /// # Errors
    /// [`TableError::NoSuchColumn`] if there is no such column.
    pub fn set_decimals(
        &mut self,
        column: usize,
        decimals: Option<usize>,
    ) -> Result<(), TableError> {
        self.mirror.set_decimals(column, decimals)
    }

    /// As [`ArrowTable::set_column_chars`].
    ///
    /// # Errors
    /// [`TableError::NoSuchColumn`] if there is no such column.
    pub fn set_column_chars(&mut self, column: usize, chars: usize) -> Result<(), TableError> {
        self.mirror.set_column_chars(column, chars)
    }

    /// A [`GridSource`] showing view `view` of `table` from `first_row` (scrolled), or
    /// `None` if the view does not exist. Call it each frame, after the table's changes.
    pub fn view<'a>(
        &'a mut self,
        table: &'a LiveTable,
        view: ViewId,
        first_row: usize,
        palette: &Palette,
    ) -> Option<LiveGridView<'a>> {
        self.sync(table);
        Some(LiveGridView {
            mirror: &self.mirror,
            rows: table.view(view)?,
            first_row,
            text: palette.text,
            dim: palette.dim_text,
        })
    }

    /// Whether rows have sized the columns (they are sized once, by the first rows).
    pub(crate) fn is_sized(&self) -> bool {
        self.mirror.is_sized()
    }

    /// Whether the formatters match `table`'s batches, so a [`RowRef`] of it reads the
    /// right values.
    pub(crate) fn is_synced(&self, table: &LiveTable) -> bool {
        self.table == (table.instance(), table.generation())
            && self.mirror.stored_batches() == table.batches().len()
    }

    /// Writes the value of `column` at `at` (a row of the synced table) into `text`.
    pub(crate) fn write(&self, at: RowRef, column: usize, text: &mut CellText) -> CellState {
        self.mirror.write_at(at.batch, at.row, column, text)
    }

    /// Adds formatters for new batches, or rebuilds them after a compaction or for another
    /// table.
    pub(crate) fn sync(&mut self, table: &LiveTable) {
        let current = (table.instance(), table.generation());
        if current != self.table {
            if self.mirror.schema() != table.schema() {
                self.mirror = ArrowTable::new(table.schema().clone());
            }
            self.mirror.clear_batches();
            self.table = current;
        }
        let new = table
            .batches()
            .get(self.mirror.stored_batches()..)
            .unwrap_or_default();
        for batch in new {
            // Same schema and never empty (a live table stores no empty batch), so a push
            // cannot fail; if it ever did, show nothing rather than misaligned rows.
            if self.mirror.push(batch.clone()).is_err() {
                self.mirror.clear_batches();
                return;
            }
        }
    }
}

/// A live view scrolled to a first row, as a grid's data. Cheap to create each frame.
#[derive(Debug, Clone, Copy)]
pub struct LiveGridView<'a> {
    mirror: &'a ArrowTable,
    rows: View<'a>,
    first_row: usize,
    text: Color,
    dim: Color,
}

impl GridSource for LiveGridView<'_> {
    fn rows(&self) -> usize {
        self.rows.len().saturating_sub(self.first_row)
    }

    fn cell(&self, row: usize, column: usize, text: &mut CellText) -> CellStyle {
        let state = match self.rows.row(self.first_row.saturating_add(row)) {
            Some(at) => self.mirror.write_at(at.batch, at.row, column, text),
            None => CellState::OutOfRange,
        };
        let color = if state == CellState::Value {
            self.text
        } else {
            self.dim
        };
        CellStyle::new(color)
    }
}
