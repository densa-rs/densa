//! Ingestion worker, pacing and latency instrumentation.
//!
//! Wires `densa-data` (what) to `densa-sync` (how it crosses threads):
//!
//! ```text
//! worker thread:  TickGenerator -> MarketTable -> snapshot_into -> Writer::publish
//! render thread:  Reader::update -> read snapshot -> FrameProbe::observe -> draw
//! ```
//!
//! The worker loop and [`FrameProbe::observe`] are hot-path safe: no allocation, no locks.

mod clock;
mod histogram;
mod pacer;
mod probe;
mod worker;

pub use clock::Clock;
pub use histogram::{LatencyHistogram, LatencySummary};
pub use pacer::{Pacer, MAX_PACER_LAG_NS};
pub use probe::{FrameProbe, ProbeReport};
pub use worker::{
    spawn_worker, ConfigError, Pin, SpawnError, WorkerConfig, WorkerHandle, WorkerMetrics,
    WorkerPanicked,
};
