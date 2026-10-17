//! densa proof of concept: zero-lock ingestion feeding a fixed-rate render loop.
//!
//! The library half holds everything testable (argument parsing, the headless loop,
//! report formatting); `main.rs` only installs the counting allocator and dispatches.

mod args;
mod headless;
mod pinning;
mod report;

pub use args::{Args, ArgsError, Mode, USAGE};
pub use headless::{run_headless, RunSummary, VIEWPORT_ROWS};
pub use pinning::{resolve_pin, write_cpu_table, PinError, PinRequest};
pub use report::{format_duration_ns, format_rate};
