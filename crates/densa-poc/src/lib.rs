//! densa proof of concept: zero-lock ingestion feeding a V-Synced GPU grid.
//!
//! The library half holds everything testable (argument parsing, the headless loop,
//! scrolling, report formatting) plus the window glue; `main.rs` only installs the
//! counting allocator and dispatches.

mod args;
mod headless;
mod pinning;
mod report;
mod scroll;
mod window;

pub use args::{Args, ArgsError, Mode, USAGE};
pub use headless::{run_headless, RunSummary, DEFAULT_HEADLESS_SECONDS, VIEWPORT_ROWS};
pub use pinning::{describe_pin, resolve_pin, write_cpu_table, PinError, PinRequest};
pub use report::{format_cpu, format_duration_ns, format_latency, format_priority, format_rate};
pub use scroll::{wheel_rows, Scroll, ScrollAction, WHEEL_ROWS_PER_LINE};
pub use window::run_window;
