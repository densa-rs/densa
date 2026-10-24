//! densa: a native Rust platform for data-dense, real-time applications.
//!
//! This crate gathers densa's libraries under one name, each as a module. Every module is
//! also its own crate (`densa-ui` is [`ui`], and so on): depend on those directly to take
//! only what you use.
//!
//! - Interface: [`ui`] (the widget tree, layout, events, focus, accessibility),
//!   [`widgets`] (labels, buttons, text inputs, scroll views, docking, the data grid and
//!   charts as widgets), `window` (a native window; feature `window`).
//! - Drawing: [`draw`] (the GPU-free display list), `render` (wgpu; feature `window`),
//!   [`grid`], [`chart`], [`text`] (fonts and the glyph atlas), [`paragraph`] (shaped,
//!   wrapped text, selection and editing).
//! - Data: `data` (live Arrow tables with incremental views and aggregates) and
//!   `arrow` (Arrow in the grid and charts), both behind the `data` feature.
//! - Runtime: [`sync`] (a wait-free triple buffer), [`platform`] (CPU placement and thread
//!   priority), [`fmt`] (allocation-free number formatting), [`alloc`] (a counting
//!   allocator for proving code allocates nothing), and [`core`] (the shared primitives).
//!
//! Data sources (CSV and Parquet files, WebSocket streams, Arrow Flight) live in
//! [densa-connectors](https://github.com/densa-rs/densa-connectors). See
//! [densa.dev](https://densa.dev) for guides and the roadmap, and
//! [densa.dev/llms.txt](https://densa.dev/llms.txt) for these docs in plain text.
//!
//! # Which module for which task
//!
//! | To | Use |
//! |---|---|
//! | open a window and run an app | `window::run` with your `window::App` |
//! | lay widgets out | [`ui::Ui::add`] with a [`ui::Style`] (flexbox, logical pixels) |
//! | show rows of live data | [`widgets::DataGrid`] over `arrow::LiveModel` (a live table) |
//! | show rows of your own data | [`widgets::DataGrid`] over your [`grid::GridModel`] |
//! | keep data sorted, filtered and aggregated as it changes | `data::LiveTable` with `data::ViewSpec` and `data::GroupSpec` |
//! | keep only recent rows, or count per minute | `data::Window`, `data::GroupSpec::group_by_time` |
//! | give each row the columns of the row it names in another table | `data::LookupJoin` |
//! | read a view's rows, or grouped results | `data::View::row` (a `data::RowRef` into `data::LiveTable::batches`), `data::Groups::to_batch` |
//! | chart data | [`widgets::Chart`] with your [`chart::ChartModel`]; `arrow::LiveSeries` reads a live table |
//! | arrange panels in tabs and splits | [`widgets::DockArea`] |
//! | add a menu bar | [`widgets::MenuBar`] with [`widgets::MenuItem`]s; choices arrive as [`ui::ActionKind::Command`] |
//! | update the screen from another thread | `window::Host::waker`, and [`sync::triple_buffer()`] for snapshots |
//! | format numbers in a frame without allocating | [`fmt`] |
//! | test widgets without a window | [`ui::Ui::new`] with [`paragraph::TextSystem::bundled`], then [`ui::Ui::pointer`] and [`ui::Ui::actions`] |
//!
//! Complete programs to start from are in
//! [densa-widgets' examples](https://github.com/densa-rs/densa/tree/main/crates/densa-widgets/examples):
//! `monitor` (a live table, a grid and a chart of the selected row), `linked`, `live_table`,
//! `dock`, `gallery` and `chart`.
//!
//! # A live table in a window
//!
//! ```no_run
//! use std::error::Error;
//! use std::sync::mpsc::{self, Receiver};
//! use std::sync::Arc;
//! use std::time::Duration;
//!
//! use densa::arrow::LiveModel;
//! use densa::data::arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray};
//! use densa::data::LiveTable;
//! use densa::ui::{Ui, WidgetId};
//! use densa::widgets::DataGrid;
//! use densa::window::{App, Host, WindowOptions};
//!
//! type Grid = DataGrid<LiveModel>;
//!
//! #[derive(Default)]
//! struct Monitor {
//!     grid: Option<WidgetId>,
//!     /// Rows from the thread that reads them (a socket, a file, a feed).
//!     arrivals: Option<Receiver<RecordBatch>>,
//! }
//!
//! /// Hosts and their load, keyed by host: a row with a known host replaces it.
//! fn loads(hosts: Vec<&str>, load: Vec<f64>) -> Result<RecordBatch, Box<dyn Error>> {
//!     Ok(RecordBatch::try_from_iter([
//!         ("host", Arc::new(StringArray::from(hosts)) as ArrayRef),
//!         ("load", Arc::new(Float64Array::from(load)) as ArrayRef),
//!     ])?)
//! }
//!
//! impl App for Monitor {
//!     fn build(&mut self, ui: &mut Ui, host: &mut Host<'_>) -> Result<(), Box<dyn Error>> {
//!         let rows = loads(vec!["web-1", "db-1"], vec![0.42, 3.5])?;
//!         let mut table = LiveTable::new(rows.schema(), "host")?;
//!         table.upsert(rows)?;
//!         let grid = Grid::new(LiveModel::new(table)?);
//!         self.grid = Some(ui.add(ui.root(), grid, Grid::style())?);
//!
//!         // Another thread reads the data and wakes the window when rows arrive.
//!         let (send, arrivals) = mpsc::channel();
//!         let waker = host.waker();
//!         std::thread::spawn(move || loop {
//!             std::thread::sleep(Duration::from_secs(1));
//!             let Ok(rows) = loads(vec!["web-1"], vec![0.5]) else { return };
//!             if send.send(rows).is_err() {
//!                 return; // the window closed
//!             }
//!             waker.wake();
//!         });
//!         self.arrivals = Some(arrivals);
//!         Ok(())
//!     }
//!
//!     /// Before each frame: take in what arrived. The grid repaints what changed.
//!     fn frame(&mut self, ui: &mut Ui, _: &mut Host<'_>) -> bool {
//!         let (Some(grid), Some(arrivals)) = (self.grid, &self.arrivals) else {
//!             return false;
//!         };
//!         for rows in arrivals.try_iter() {
//!             ui.update::<Grid, _>(grid, |grid, cx| {
//!                 if let Err(error) = grid.model_mut(cx).table_mut().upsert(rows) {
//!                     eprintln!("rows refused: {error}");
//!                 }
//!             });
//!         }
//!         false // nothing animates: the next frame comes when the thread wakes the window
//!     }
//! }
//!
//! densa::window::run(WindowOptions::new("Monitor"), Monitor::default())?;
//! # Ok::<(), Box<dyn Error>>(())
//! ```

pub use densa_alloc as alloc;
#[cfg(feature = "data")]
pub use densa_arrow as arrow;
pub use densa_chart as chart;
pub use densa_core as core;
#[cfg(feature = "data")]
pub use densa_data as data;
pub use densa_draw as draw;
pub use densa_fmt as fmt;
pub use densa_grid as grid;
pub use densa_paragraph as paragraph;
pub use densa_platform as platform;
#[cfg(feature = "window")]
pub use densa_render as render;
pub use densa_sync as sync;
pub use densa_text as text;
pub use densa_ui as ui;
pub use densa_widgets as widgets;
#[cfg(feature = "window")]
pub use densa_window as window;
