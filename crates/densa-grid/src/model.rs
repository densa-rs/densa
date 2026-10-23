use crate::{CellStyle, CellText, Column, Palette};

/// Which way a column sorts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SortOrder {
    /// Smallest first.
    Ascending,
    /// Largest first.
    Descending,
}

impl SortOrder {
    /// The other order.
    #[must_use]
    pub fn flipped(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// The data of a grid widget: its columns, its rows, and each cell's text and style, plus
/// what the widget asks of it as the user works with it.
///
/// Rows are numbered from 0 in the order they are shown; the widget asks only for the rows
/// on screen. [`cell`](Self::cell) runs for every visible cell every frame and formats into
/// a stack buffer, so it must not allocate.
pub trait GridModel {
    /// The columns, left to right.
    fn columns(&self) -> &[Column];

    /// How many rows there are.
    fn rows(&self) -> usize;

    /// Writes the text of the cell at `row`, `column` into `text` (empty on entry) and
    /// returns its style, in the grid's colors (`palette`, which follows the theme).
    fn cell(&self, row: usize, column: usize, text: &mut CellText, palette: &Palette) -> CellStyle;

    /// Brings the model up to date before the grid paints (taking in data that arrived, for
    /// example). It may allocate when the data changed; it should not otherwise. It runs only
    /// when the grid paints, not while it is hidden (an inactive tab): feed a model that
    /// must keep up while hidden from the application, through
    /// `DataGrid::model_mut`.
    fn prepare(&mut self) {}

    /// Sorts the rows by `column` (an index into [`columns`](Self::columns)) in `order`, when
    /// the user asks for it (a click on its title); `false` if the model does not sort by
    /// that column, which leaves the order alone.
    fn sort(&mut self, column: usize, order: SortOrder) -> bool {
        let _ = (column, order);
        false
    }

    /// The column and order the rows are sorted by now, if the model knows (a model sorted
    /// before the user clicked, or one that sorts itself); the grid shows it and reverses it
    /// on the next click.
    fn sort_state(&self) -> Option<(usize, SortOrder)> {
        None
    }

    /// A value identifying the record at `row` while rows are added, removed or sorted (a
    /// database key, an instrument id). With it, a selection follows its record; without
    /// it (`None`, the default), a selection is a position and is cleared by sorting.
    fn row_key(&self, row: usize) -> Option<u64> {
        let _ = row;
        None
    }

    /// Where the record [`row_key`](Self::row_key) gave `key` for is now, if it is still
    /// shown. Asked once a frame while a row is selected: make it fast for large models.
    /// `None` (the record was removed, or is filtered out) clears the selection.
    fn row_of_key(&self, key: u64) -> Option<usize> {
        let _ = key;
        None
    }

    /// Changes whenever [`columns`](Self::columns) changes (another schema, new widths): the
    /// grid then reads the columns again, keeping the widths the user dragged if the column
    /// titles are the same as before. Checked each frame after [`prepare`](Self::prepare).
    fn columns_revision(&self) -> u64 {
        0
    }

    /// Changes when the rows were replaced wholesale (another table), so that keys from
    /// before mean nothing: the grid then clears its selection and scrolls to the top.
    /// Checked each frame after [`prepare`](Self::prepare).
    fn generation(&self) -> u64 {
        0
    }
}
