use std::time::Instant;

/// Monotonic nanosecond clock with a shared epoch.
///
/// Copy one `Clock` to every thread so all timestamps (tick, publish, frame) share the
/// same origin and can be subtracted directly.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    epoch: Instant,
}

impl Clock {
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }

    /// Nanoseconds since the epoch. Saturates after ~584 years.
    #[inline]
    pub fn now_ns(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}
