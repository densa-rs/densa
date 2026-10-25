//! Apache Arrow tables in densa's data grid.
//!
//! [`ArrowTable`] holds Arrow [`RecordBatch`](arrow_array::RecordBatch)es (from Parquet,
//! DataFusion, DuckDB, a Flight stream or your own code) and formats their values into
//! grid cells; [`ArrowTable::view`] turns it into a `densa_grid::GridSource` scrolled to
//! a row, which `GridPainter` draws. Arrays are shared with the batches, never copied, so a
//! table of millions of rows costs no more to show than its visible rows, and drawing a
//! frame allocates nothing.
//!
//! For live data, [`LiveModel`] makes a `densa_data::LiveTable` the model of the grid widget
//! (`densa_widgets::DataGrid`): sorted by a click, filtered by the app, its selection kept on
//! a row as the row moves. [`LiveGrid`] paints a view of one with `GridPainter` directly,
//! and [`LiveSeries`] reads two of its numeric columns as a `densa_chart` series (a price
//! over time), so a chart follows the same live data.
//!
//! For tables too large to hold (a query engine's result over billions of rows), a
//! [`PagedModel`] is the grid widget's model: your [`PageSource`] gives rows a page at a
//! time, on a thread of the model's own, and the grid fetches only the pages it shows, in
//! the order a click on a column title asks for.
//!
//! ```
//! use std::sync::Arc;
//! use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray};
//! use densa_arrow::ArrowTable;
//! use densa_grid::{CellText, GridSource, Palette};
//!
//! let batch = RecordBatch::try_from_iter([
//!     ("sensor", Arc::new(StringArray::from(vec!["a-1", "b-2"])) as ArrayRef),
//!     ("celsius", Arc::new(Float64Array::from(vec![21.5, 80.25])) as ArrayRef),
//! ])?;
//! let mut table = ArrowTable::from_batches([batch])?;
//! table.set_decimals(1, Some(1))?;
//!
//! let columns = table.columns(); // for `GridPainter::new`
//! let view = table.view(0, &Palette::new()); // each frame: `painter.paint(&view, &mut canvas)`
//! let mut text = CellText::new();
//! view.cell(1, 1, &mut text);
//! assert_eq!(text.as_str(), "80.2");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

//! The API takes and returns [`arrow_array`] and [`arrow_schema`] types, re-exported here so
//! applications use the same Arrow version: a major Arrow upgrade is a breaking release of
//! this crate.

pub use arrow_array;
pub use arrow_schema;

mod calendar;
mod live;
mod model;
mod paged;
mod reader;
mod series;
mod table;

pub use live::{LiveGrid, LiveGridView};
pub use model::LiveModel;
pub use paged::{FetchError, PageError, PageOptions, PageQuery, PageSource, PagedModel, SortKey};
pub use series::{LiveSeries, SeriesError, SeriesPoints};
pub use table::{ArrowGrid, ArrowTable, CellState, TableError};
