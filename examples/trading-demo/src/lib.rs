//! A real-time trading grid built with densa: a worker thread streams a synthetic feed into a
//! market table and hands the visible rows to the UI thread through a wait-free triple
//! buffer; the grid renders at the display's refresh rate.
//!
//! - [`data`]: the market table, tick feed and viewport snapshots.
//! - [`engine`]: the ingestion worker, pacing and latency instrumentation.
//! - The rest: market columns, argument parsing, the headless loop and the window.
//!
//! The library half holds everything testable; `main.rs` only installs the counting
//! allocator and dispatches.

pub mod data;
pub mod engine;

mod args;
mod headless;
mod market;
mod pinning;
mod report;
mod scroll;
mod window;

pub use args::{Args, ArgsError, Mode, DEFAULT_PRIORITY, USAGE};
pub use headless::{run_headless, RunSummary, DEFAULT_HEADLESS_SECONDS, VIEWPORT_ROWS};
pub use market::{
    flash_alpha, market_columns, market_painter, MarketGrid, FLASH_MAX_ALPHA, FLASH_NS, FONT_PX,
    MARKET_GLYPHS,
};
pub use pinning::{describe_pin, resolve_pin, write_cpu_table, PinError, PinRequest};
pub use report::{format_cpu, format_duration_ns, format_latency, format_priority, format_rate};
pub use scroll::{wheel_rows, Scroll, ScrollAction, WHEEL_ROWS_PER_LINE};
pub use window::run_window;
