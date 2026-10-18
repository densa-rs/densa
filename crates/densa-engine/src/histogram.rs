/// Linear sub-buckets per power of two: relative error is at most `1 / SUB_BUCKETS`.
const SUB_BITS: u32 = 4;
const SUB_BUCKETS: u64 = 1 << SUB_BITS;
/// Group 0 holds 0..16 exactly; groups 1..=60 cover each power of two up to `u64::MAX`.
const GROUPS: usize = 64 - SUB_BITS as usize + 1;
const BUCKETS: usize = GROUPS * SUB_BUCKETS as usize;

/// p50 / p99 / max digest of a histogram (all zero when empty).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LatencySummary {
    /// Samples recorded.
    pub count: u64,
    /// Median (bucket upper bound).
    pub p50: u64,
    /// 99th percentile (bucket upper bound).
    pub p99: u64,
    /// Exact maximum.
    pub max: u64,
}

/// Fixed-size log-linear histogram of `u64` samples (typically nanoseconds).
///
/// Covers the full `u64` range with no configuration and no allocation: values below 16
/// are exact, larger values land in buckets at most 1/16 wide relative to their value.
/// Quantiles report the bucket's upper bound (never an underestimate), capped at the
/// exact maximum.
#[derive(Debug, Clone)]
pub struct LatencyHistogram {
    counts: [u64; BUCKETS],
    total: u64,
    sum: u128,
    min: u64,
    max: u64,
}

impl LatencyHistogram {
    /// An empty histogram.
    pub const fn new() -> Self {
        Self {
            counts: [0; BUCKETS],
            total: 0,
            sum: 0,
            min: u64::MAX,
            max: 0,
        }
    }

    #[inline]
    /// Adds one sample. O(1) and allocation-free.
    pub fn record(&mut self, value: u64) {
        self.counts[bucket_index(value)] += 1;
        self.total += 1;
        self.sum += u128::from(value);
        self.min = self.min.min(value);
        self.max = self.max.max(value);
    }

    /// Samples recorded since creation or the last [`reset`](Self::reset).
    pub fn count(&self) -> u64 {
        self.total
    }

    /// Exact minimum, or `None` when empty.
    pub fn min(&self) -> Option<u64> {
        (self.total > 0).then_some(self.min)
    }

    /// Exact maximum, or `None` when empty.
    pub fn max(&self) -> Option<u64> {
        (self.total > 0).then_some(self.max)
    }

    /// Arithmetic mean, or `None` when empty.
    pub fn mean(&self) -> Option<f64> {
        (self.total > 0).then(|| self.sum as f64 / self.total as f64)
    }

    /// Value at quantile `q` (clamped to `[0, 1]`; `NaN` is treated as 0).
    pub fn quantile(&self, q: f64) -> Option<u64> {
        if self.total == 0 {
            return None;
        }
        let q = if q.is_nan() { 0.0 } else { q.clamp(0.0, 1.0) };
        // Lossless enough: ranks above 2^53 samples are far beyond any realistic window.
        let rank = ((q * self.total as f64).ceil() as u64).max(1);
        let mut seen = 0;
        for (index, &count) in self.counts.iter().enumerate() {
            seen += count;
            if seen >= rank {
                return Some(bucket_upper_bound(index).min(self.max));
            }
        }
        Some(self.max)
    }

    /// p50, p99 and max in one value; all zero when empty.
    pub fn summary(&self) -> LatencySummary {
        match (self.quantile(0.5), self.quantile(0.99), self.max()) {
            (Some(p50), Some(p99), Some(max)) => LatencySummary {
                count: self.total,
                p50,
                p99,
                max,
            },
            _ => LatencySummary::default(),
        }
    }

    /// Adds every sample of `other` to `self`.
    pub fn merge(&mut self, other: &Self) {
        for (mine, theirs) in self.counts.iter_mut().zip(other.counts.iter()) {
            *mine += theirs;
        }
        self.total += other.total;
        self.sum += other.sum;
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }

    /// Forgets every sample.
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn bucket_index(value: u64) -> usize {
    if value < SUB_BUCKETS {
        // Lossless: below 16.
        return value as usize;
    }
    let msb = 63 - value.leading_zeros(); // >= SUB_BITS
    let shift = msb - SUB_BITS;
    let group = (shift + 1) as usize;
    // The top bit is implicit in the group; the next SUB_BITS bits pick the sub-bucket.
    let sub = ((value >> shift) & (SUB_BUCKETS - 1)) as usize;
    group * SUB_BUCKETS as usize + sub
}

fn bucket_upper_bound(index: usize) -> u64 {
    let group = index / SUB_BUCKETS as usize;
    // Lossless: always below 16.
    let sub = (index % SUB_BUCKETS as usize) as u64;
    if group == 0 {
        return sub;
    }
    let shift = (group - 1) as u32;
    let lower = (SUB_BUCKETS + sub) << shift;
    // Cannot overflow: the last bucket's bound is exactly u64::MAX.
    lower + ((1u64 << shift) - 1)
}
