use densa_data::{DataError, Filter, LiveTable, Order, RecordId, View, ViewId, ViewSpec};
use densa_grid::{CellStyle, CellText, Column, GridModel, Palette, SortOrder};

use crate::live::LiveGrid;
use crate::table::{CellState, TableError};

/// A `densa_data::LiveTable` as the model of a grid widget (`densa_widgets::DataGrid`): one
/// view of the table, sorted when the user clicks a column title and filtered by the app,
/// with rows keyed by their record so a selection follows its row as the data changes.
///
/// Change the table through [`table_mut`](Self::table_mut); the view follows each change
/// incrementally. Cells are formatted as in [`ArrowTable`](crate::ArrowTable), from
/// formatters brought up to date by [`GridModel::prepare`], which the grid calls before it
/// paints; until then, rows from batches added since show empty cells rather than stale
/// values. Columns are sized from the first rows to arrive, and the grid takes in new sizes
/// by itself ([`GridModel::columns_revision`]).
///
/// Costs: a frame without changes allocates nothing; one after upserts allocates a
/// formatter per new batch, and after a compaction rebuilds them all. Sorting or filtering
/// builds a new view, `O(n log n)` in the table's rows.
#[derive(Debug)]
pub struct LiveModel {
    table: LiveTable,
    grid: LiveGrid,
    columns: Vec<Column>,
    view: ViewId,
    filter: Option<Filter>,
    sort: Option<(usize, SortOrder)>,
    /// The table object (`LiveTable::instance`) shown.
    instance: u64,
    /// Bumped when the columns change, and when the table is replaced.
    columns_revision: u64,
    generation: u64,
}

impl LiveModel {
    /// A model showing every row of `table`, unsorted (in row-id order: the order keys
    /// arrived, until deleted keys' places are reused).
    ///
    /// # Errors
    /// Those of `LiveTable::add_view`; none for a view of every row, unsorted.
    pub fn new(mut table: LiveTable) -> Result<Self, DataError> {
        let view = table.add_view(ViewSpec::new())?;
        let grid = LiveGrid::new(&table);
        Ok(Self {
            columns: grid.columns(),
            grid,
            view,
            filter: None,
            sort: None,
            instance: table.instance(),
            table,
            columns_revision: 0,
            generation: 0,
        })
    }

    /// The table.
    pub fn table(&self) -> &LiveTable {
        &self.table
    }

    /// The table, to upsert, delete or compact; the view follows. Another table put in its
    /// place is shown from the next paint, filtered and sorted the same way where its columns
    /// allow, and the grid starts over (no selection, its columns read again).
    pub fn table_mut(&mut self) -> &mut LiveTable {
        &mut self.table
    }

    /// The rows shown, in order (`None` only between replacing the table and the next
    /// paint).
    pub fn view(&self) -> Option<View<'_>> {
        self.table.view(self.view)
    }

    /// The record of the row at `index`, as shown.
    pub fn record(&self, index: usize) -> Option<RecordId> {
        self.view()?.record(index)
    }

    /// The filter rows are shown by.
    pub fn filter(&self) -> Option<&Filter> {
        self.filter.as_ref()
    }

    /// Shows only the rows `filter` keeps (all of them for `None`), in the same sort. A
    /// selected row the filter hides is no longer selected.
    ///
    /// # Errors
    /// Those of `LiveTable::add_view` for the filter (an unknown column, a comparison
    /// with a value of another type); the model is unchanged then.
    pub fn set_filter(&mut self, filter: Option<Filter>) -> Result<(), DataError> {
        self.replace_view(filter, self.sort)
    }

    /// As [`ArrowTable::set_decimals`](crate::ArrowTable::set_decimals); the grid takes in
    /// the new width.
    ///
    /// # Errors
    /// [`TableError::NoSuchColumn`] if there is no such column.
    pub fn set_decimals(
        &mut self,
        column: usize,
        decimals: Option<usize>,
    ) -> Result<(), TableError> {
        self.grid.set_decimals(column, decimals)?;
        self.read_columns();
        Ok(())
    }

    /// As [`ArrowTable::set_column_chars`](crate::ArrowTable::set_column_chars); the grid
    /// takes in the new width.
    ///
    /// # Errors
    /// [`TableError::NoSuchColumn`] if there is no such column.
    pub fn set_column_chars(&mut self, column: usize, chars: usize) -> Result<(), TableError> {
        self.grid.set_column_chars(column, chars)?;
        self.read_columns();
        Ok(())
    }

    /// Reads the columns again and tells the grid.
    fn read_columns(&mut self) {
        self.columns = self.grid.columns();
        self.columns_revision = self.columns_revision.wrapping_add(1);
    }

    /// Builds the view for `filter` and `sort`, then drops the old one; on error nothing
    /// changes.
    fn replace_view(
        &mut self,
        filter: Option<Filter>,
        sort: Option<(usize, SortOrder)>,
    ) -> Result<(), DataError> {
        let view = self.table.add_view(self.spec(filter.clone(), sort))?;
        self.table.remove_view(self.view);
        self.view = view;
        self.filter = filter;
        self.sort = sort;
        Ok(())
    }

    /// The view spec for `filter` and `sort` (a sort the table cannot do is left out: see
    /// [`sort_name`](Self::sort_name)).
    fn spec(&self, filter: Option<Filter>, sort: Option<(usize, SortOrder)>) -> ViewSpec {
        let mut spec = ViewSpec::new();
        if let Some(filter) = filter {
            spec = spec.filter(filter);
        }
        if let Some((column, order)) = sort {
            if let Some(name) = self.sort_name(column) {
                let order = match order {
                    SortOrder::Descending => Order::Descending,
                    _ => Order::Ascending,
                };
                spec = spec.sort_by(name, order);
            }
        }
        spec
    }

    /// The name views sort `column` by: none if the table has no such column, or if an
    /// earlier column has the same name (views find columns by name).
    fn sort_name(&self, column: usize) -> Option<String> {
        let schema = self.table.schema();
        let name = schema.fields().get(column)?.name();
        let (first, _) = schema.column_with_name(name)?;
        (first == column).then(|| name.clone())
    }

    /// Shows the table put in place of the last one: the same filter and sort where its
    /// columns allow (else none), and a fresh start for the grid.
    fn adopt_table(&mut self) {
        self.instance = self.table.instance();
        self.generation = self.generation.wrapping_add(1);
        if self
            .sort
            .is_some_and(|(column, _)| self.sort_name(column).is_none())
        {
            self.sort = None;
        }
        if self.view().is_none() {
            let spec = self.spec(self.filter.clone(), self.sort);
            let view = self.table.add_view(spec).or_else(|_| {
                self.filter = None;
                self.sort = None;
                self.table.add_view(ViewSpec::new())
            });
            if let Ok(view) = view {
                self.view = view;
            }
        }
    }
}

impl GridModel for LiveModel {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn rows(&self) -> usize {
        self.view().map_or(0, |view| view.len())
    }

    fn cell(&self, row: usize, column: usize, text: &mut CellText, palette: &Palette) -> CellStyle {
        let at = self.view().and_then(|view| view.row(row));
        let state = match at {
            Some(at) if self.grid.is_synced(&self.table) => self.grid.write(at, column, text),
            _ => CellState::OutOfRange,
        };
        let color = if state == CellState::Value {
            palette.text
        } else {
            palette.dim_text
        };
        CellStyle::new(color)
    }

    fn prepare(&mut self) {
        let replaced = self.instance != self.table.instance();
        if replaced {
            self.adopt_table();
        }
        let sized = self.grid.is_sized();
        self.grid.sync(&self.table);
        // Another table (maybe of another schema), or the first rows sized the columns:
        // read them again (allocates, once).
        if replaced || sized != self.grid.is_sized() {
            self.read_columns();
        }
    }

    fn sort(&mut self, column: usize, order: SortOrder) -> bool {
        self.sort_name(column).is_some()
            && self
                .replace_view(self.filter.clone(), Some((column, order)))
                .is_ok()
    }

    fn sort_state(&self) -> Option<(usize, SortOrder)> {
        self.sort
    }

    fn row_key(&self, row: usize) -> Option<u64> {
        self.record(row).map(RecordId::to_bits)
    }

    fn row_of_key(&self, key: u64) -> Option<usize> {
        self.view()?.position(RecordId::from_bits(key))
    }

    fn columns_revision(&self) -> u64 {
        self.columns_revision
    }

    fn generation(&self) -> u64 {
        self.generation
    }
}
