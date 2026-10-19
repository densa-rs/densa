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
//! [densa.dev](https://densa.dev) for guides and the roadmap.

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
