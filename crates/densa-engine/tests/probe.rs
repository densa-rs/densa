use densa_data::{MarketTable, Tick, ViewportSnapshot};
use densa_engine::FrameProbe;

fn snapshot(seq: u64, published_at_ns: u64, tick_ts: &[u64]) -> ViewportSnapshot<4> {
    let mut table = MarketTable::new(4);
    let ticks: Vec<Tick> = tick_ts
        .iter()
        .enumerate()
        .map(|(i, &ts_ns)| Tick {
            symbol_id: i as u32,
            bid: 1.0,
            ask: 1.1,
            last: 1.05,
            qty: 1,
            ts_ns,
        })
        .collect();
    table.apply_batch(&ticks);
    let mut snap = ViewportSnapshot::new();
    table.snapshot_into(0, &mut snap);
    snap.seq = seq;
    snap.published_at_ns = published_at_ns;
    snap
}

#[test]
fn records_snapshot_age_and_freshest_tick_age() {
    let mut probe = FrameProbe::new();
    probe.observe(&snapshot(1, 900, &[100, 850, 400]), true, 1_000);
    let report = probe.report();
    assert_eq!(report.frames, 1);
    assert_eq!(report.fresh_frames, 1);
    assert_eq!(report.snapshot_age.max, 100);
    assert_eq!(
        report.tick_age.max, 150,
        "age of the newest visible tick (850)"
    );
}

#[test]
fn reused_snapshot_counts_frame_but_not_fresh_frame() {
    let mut probe = FrameProbe::new();
    let snap = snapshot(1, 900, &[850]);
    probe.observe(&snap, true, 1_000);
    probe.observe(&snap, false, 2_000);
    let report = probe.report();
    assert_eq!(report.frames, 2);
    assert_eq!(report.fresh_frames, 1);
    assert_eq!(
        report.snapshot_age.count, 2,
        "displayed data ages every frame"
    );
    assert_eq!(report.snapshot_age.max, 1_100);
}

#[test]
fn counts_snapshots_overwritten_between_frames() {
    let mut probe = FrameProbe::new();
    probe.observe(&snapshot(1, 10, &[5]), true, 20);
    probe.observe(&snapshot(5, 30, &[25]), true, 40);
    assert_eq!(probe.report().skipped_snapshots, 3);
}

#[test]
fn ignores_snapshot_before_first_publish() {
    let mut probe = FrameProbe::new();
    probe.observe(&ViewportSnapshot::<4>::new(), false, 1_000);
    let report = probe.report();
    assert_eq!(report.frames, 1);
    assert_eq!(report.snapshot_age.count, 0);
    assert_eq!(report.tick_age.count, 0);
}

#[test]
fn rows_without_ticks_do_not_define_freshness() {
    let mut probe = FrameProbe::new();
    // Rows 1..3 never traded (updated_at_ns == 0): only row 0 counts.
    probe.observe(&snapshot(1, 90, &[70]), true, 100);
    assert_eq!(probe.report().tick_age.max, 30);
}

#[test]
fn clock_skew_never_underflows() {
    let mut probe = FrameProbe::new();
    probe.observe(&snapshot(1, 2_000, &[1_500]), true, 1_000);
    let report = probe.report();
    assert_eq!(report.snapshot_age.max, 0);
    assert_eq!(report.tick_age.max, 0);
}

#[test]
fn reset_starts_a_new_reporting_window_but_keeps_sequence_tracking() {
    let mut probe = FrameProbe::new();
    probe.observe(&snapshot(1, 10, &[5]), true, 20);
    probe.reset();
    assert_eq!(probe.report().frames, 0);
    probe.observe(&snapshot(2, 30, &[25]), true, 40);
    assert_eq!(
        probe.report().skipped_snapshots,
        0,
        "seq 1 -> 2 skips nothing"
    );
}
