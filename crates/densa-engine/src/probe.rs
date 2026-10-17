use crate::{LatencyHistogram, LatencySummary};
use densa_data::ViewportSnapshot;

/// Digest of one reporting window of [`FrameProbe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProbeReport {
    pub frames: u64,
    /// Frames that picked up a snapshot newer than the previous frame's.
    pub fresh_frames: u64,
    /// Snapshots published but overwritten before any frame read them (expected when the
    /// worker publishes faster than the display refreshes).
    pub skipped_snapshots: u64,
    /// `frame time - publish time` of the displayed snapshot.
    pub snapshot_age: LatencySummary,
    /// `frame time - newest tick time` among the displayed rows (end-to-end latency).
    pub tick_age: LatencySummary,
}

/// Render-side latency instrumentation. Allocation-free; call once per frame.
#[derive(Debug, Clone, Default)]
pub struct FrameProbe {
    snapshot_age: LatencyHistogram,
    tick_age: LatencyHistogram,
    frames: u64,
    fresh_frames: u64,
    skipped_snapshots: u64,
    last_seq: u64,
}

impl FrameProbe {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the snapshot displayed at `now_ns`; `is_new` is `Reader::update`'s result.
    pub fn observe<const N: usize>(
        &mut self,
        snapshot: &ViewportSnapshot<N>,
        is_new: bool,
        now_ns: u64,
    ) {
        self.frames += 1;
        if snapshot.seq == 0 {
            return; // Initial placeholder: nothing published yet.
        }
        if is_new {
            self.fresh_frames += 1;
            if self.last_seq != 0 && snapshot.seq > self.last_seq + 1 {
                self.skipped_snapshots += snapshot.seq - self.last_seq - 1;
            }
            self.last_seq = snapshot.seq;
        }

        self.snapshot_age
            .record(now_ns.saturating_sub(snapshot.published_at_ns));
        let newest_tick = snapshot
            .visible()
            .iter()
            .map(|row| row.updated_at_ns)
            .filter(|&ts| ts > 0)
            .max();
        if let Some(ts) = newest_tick {
            self.tick_age.record(now_ns.saturating_sub(ts));
        }
    }

    pub fn report(&self) -> ProbeReport {
        ProbeReport {
            frames: self.frames,
            fresh_frames: self.fresh_frames,
            skipped_snapshots: self.skipped_snapshots,
            snapshot_age: self.snapshot_age.summary(),
            tick_age: self.tick_age.summary(),
        }
    }

    /// Starts a new reporting window. Sequence tracking carries over.
    pub fn reset(&mut self) {
        self.snapshot_age.reset();
        self.tick_age.reset();
        self.frames = 0;
        self.fresh_frames = 0;
        self.skipped_snapshots = 0;
    }
}
