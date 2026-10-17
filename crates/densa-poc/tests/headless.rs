//! End-to-end: worker thread + 120 Hz reader loop, with heap counting installed.

use densa_alloc::{is_active, CountingAllocator};
use densa_poc::{format_duration_ns, run_headless, Args};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

fn headless_args(extra: &[&str]) -> Args {
    let base = ["--headless", "--seconds", "1", "--symbols", "2000"];
    Args::parse(base.iter().chain(extra).map(|s| s.to_string())).unwrap()
}

#[test]
fn rate_limited_run_hits_targets_with_zero_hot_path_allocations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );
    let mut out = Vec::new();
    let summary = run_headless(&headless_args(&["--rate", "100000"]), &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();

    assert_eq!(summary.worker.heap_ops, 0, "worker allocated\n{text}");
    assert_eq!(
        summary.reader_heap_ops, 0,
        "reader frame loop allocated\n{text}"
    );
    assert_eq!(summary.worker.ticks_rejected, 0);

    let tick_rate = summary.worker.ticks_applied as f64 / summary.elapsed_s;
    assert!(
        (50_000.0..=110_000.0).contains(&tick_rate),
        "tick rate {tick_rate}\n{text}"
    );
    // ~120 frames in 1 s; generous lower bound for a busy machine.
    assert!(
        (60..=125).contains(&summary.frames),
        "frames {}\n{text}",
        summary.frames
    );
    assert!(summary.snapshot_age.count > 0);
    assert!(summary.tick_age.count > 0);

    assert!(text.contains("ticks/s"), "{text}");
    assert!(text.contains("summary"), "{text}");
}

#[test]
fn unbounded_run_is_much_faster_than_rate_limited() {
    let mut out = Vec::new();
    let summary = run_headless(&headless_args(&["--unbounded"]), &mut out).unwrap();
    let tick_rate = summary.worker.ticks_applied as f64 / summary.elapsed_s;
    assert!(
        tick_rate > 1_000_000.0,
        "unbounded only reached {tick_rate} ticks/s"
    );
    assert_eq!(summary.worker.heap_ops, 0);
    assert_eq!(summary.reader_heap_ops, 0);
}

#[test]
fn durations_are_human_readable_and_ascii() {
    assert_eq!(format_duration_ns(850), "850ns");
    assert_eq!(format_duration_ns(45_200), "45.2us");
    assert_eq!(format_duration_ns(1_250_000), "1.25ms");
    assert_eq!(format_duration_ns(3_500_000_000), "3.50s");
}
