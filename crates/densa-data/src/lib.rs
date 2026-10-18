//! Live Arrow tables: rows identified by a key, updated by record batches, and views that
//! stay sorted and filtered as the rows change.
//!
//! [`LiveTable`] takes Arrow [`RecordBatch`](arrow_array::RecordBatch)es from a feed, a
//! query engine or a file, and upserts them by a key column; [`ViewSpec`] describes a view
//! (a [`Filter`] and sort columns), which the table keeps up to date incrementally instead of
//! recomputing it. A view's rows are [`RowRef`]s into [`LiveTable::batches`], so a display
//! layer (such as `densa-arrow`'s grid) reads values straight from the Arrow arrays.
//!
//! This crate has no user-interface dependency.
//!
//! ```
//! use std::sync::Arc;
//! use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray};
//! use densa_data::{CompareOp, Filter, LiveTable, Order, Scalar, ViewSpec};
//!
//! let quotes = |symbols: Vec<&str>, prices: Vec<f64>| {
//!     RecordBatch::try_from_iter([
//!         ("symbol", Arc::new(StringArray::from(symbols)) as ArrayRef),
//!         ("price", Arc::new(Float64Array::from(prices)) as ArrayRef),
//!     ])
//! };
//! let mut table = LiveTable::new(quotes(vec![], vec![])?.schema(), "symbol")?;
//! let expensive = table.add_view(
//!     ViewSpec::new()
//!         .filter(Filter::compare("price", CompareOp::Gt, Scalar::Float(100.0)))
//!         .sort_by("price", Order::Descending),
//! )?;
//!
//! table.upsert(quotes(vec!["AAA", "BBB", "CCC"], vec![150.0, 90.0, 120.0])?)?;
//! table.upsert(quotes(vec!["BBB"], vec![200.0])?)?; // BBB rises: it enters the view
//!
//! let view = table.view(expensive).unwrap();
//! assert_eq!(view.len(), 3); // BBB 200, AAA 150, CCC 120
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod error;
mod filter;
mod key;
mod sorted;
mod table;
mod value;
mod view;

pub use arrow_array;
pub use arrow_schema;
pub use error::DataError;
pub use filter::{CompareOp, Filter, Scalar};
pub use table::{Changes, LiveTable, RowRef, TableOptions, View, ViewId};
pub use view::{Order, ViewSpec};

/// A row's internal identity: stable for as long as its key is live.
type RowId = u32;
