//! The market: flat, preallocated data storage, a synthetic tick feed, and fixed-size viewport
//! snapshots.
//!
//! Everything here allocates only at construction. `apply_batch`, `snapshot_into` and
//! `TickGenerator::fill` are hot-path safe: no allocation, no locks, no syscalls.

mod generator;
mod rng;
mod snapshot;
mod table;
mod tick;

pub use generator::TickGenerator;
pub use rng::WyRand;
pub use snapshot::{RowSnapshot, ViewportSnapshot};
pub use table::{ApplyStats, MarketTable};
pub use tick::{Direction, Tick};
