use std::num::NonZeroU64;

const NANOS_PER_SEC: u128 = 1_000_000_000;

/// If the worker falls this far behind schedule, it restarts the schedule from "now"
/// instead of bursting through the backlog.
pub const MAX_PACER_LAG_NS: u64 = 10_000_000;

/// Paces fixed-size batches to a target ticks-per-second rate.
///
/// Deadlines are computed from the schedule origin (`origin + k * batch / rate`), not by
/// accumulating intervals, so fractional intervals never drift. The pacer itself never
/// waits: callers spin until [`Pacer::deadline_ns`] (the worker must not yield).
#[derive(Debug, Clone)]
pub struct Pacer {
    rate_per_sec: NonZeroU64,
    batch_size: u64,
    origin_ns: u64,
    batches: u64,
    lag_resets: u64,
}

impl Pacer {
    /// Paces batches of `batch_size` ticks at `rate_per_sec`, first batch at `start_ns`.
    ///
    /// The rate is `NonZeroU64` because a zero rate has no schedule; the type rules it out
    /// instead of a runtime panic.
    pub fn new(rate_per_sec: NonZeroU64, batch_size: usize, start_ns: u64) -> Self {
        Self {
            rate_per_sec,
            // Lossless on every supported target (usize <= 64 bits).
            batch_size: batch_size as u64,
            origin_ns: start_ns,
            batches: 0,
            lag_resets: 0,
        }
    }

    /// Earliest time the next batch may start.
    #[inline]
    pub fn deadline_ns(&self) -> u64 {
        let ticks = u128::from(self.batches) * u128::from(self.batch_size);
        let offset = ticks * NANOS_PER_SEC / u128::from(self.rate_per_sec.get());
        self.origin_ns
            .saturating_add(u64::try_from(offset).unwrap_or(u64::MAX))
    }

    /// Records that a batch started at `started_ns`.
    #[inline]
    pub fn complete_batch(&mut self, started_ns: u64) {
        self.batches += 1;
        if started_ns.saturating_sub(self.deadline_ns()) > MAX_PACER_LAG_NS {
            self.origin_ns = started_ns;
            self.batches = 0;
            self.lag_resets += 1;
        }
    }

    /// Number of times the schedule was restarted after falling too far behind.
    pub fn lag_resets(&self) -> u64 {
        self.lag_resets
    }
}
