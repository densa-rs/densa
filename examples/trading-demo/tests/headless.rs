//! End-to-end: worker thread + 120 Hz reader loop, with heap counting installed.

use densa_alloc::{is_active, CountingAllocator};
use trading_demo::{format_duration_ns, run_headless, Args};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

/// One session at a time. Each runs a busy-spinning worker: on a 2-CPU machine two of them
/// plus their frame loops leave no CPU for anything (a 1 s run took 5.7 s at 0.4 fps).
/// Separate cores for concurrent workers are tested in `tests/worker.rs`.
static SESSION: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn one_session_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    SESSION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Throughput floors are performance measurements: meaningful on real hardware, not on shared
/// CI virtual machines, where sleeps overshoot (macOS runners once paced a 120 fps loop at
/// 41 fps). There (`CI` is set) only correctness is checked: limits never exceeded, data
/// flowing, nothing allocated.
fn on_ci() -> bool {
    std::env::var_os("CI").is_some()
}

fn headless_args(extra: &[&str]) -> Args {
    // Default `--pin auto`, as users run it.
    let base = ["--headless", "--seconds", "1", "--symbols", "2000"];
    Args::parse(base.iter().chain(extra).map(|s| s.to_string())).unwrap()
}

#[test]
fn rate_limited_run_hits_targets_with_zero_hot_path_allocations() {
    let _session = one_session_at_a_time();
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

    // The pacers never overshoot their targets (100k ticks/s, 120 fps), anywhere.
    let tick_rate = summary.worker.ticks_applied as f64 / summary.elapsed_s;
    assert!(tick_rate <= 110_000.0, "tick rate {tick_rate}\n{text}");
    assert!(summary.frames <= 125, "frames {}\n{text}", summary.frames);
    // And reach them on real hardware (~120 frames in 1 s; generous for a busy machine).
    let (min_rate, min_frames) = if on_ci() { (1.0, 1) } else { (50_000.0, 60) };
    assert!(tick_rate >= min_rate, "tick rate {tick_rate}\n{text}");
    assert!(
        summary.frames >= min_frames,
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
    let _session = one_session_at_a_time();
    let mut out = Vec::new();
    let summary = run_headless(&headless_args(&["--unbounded"]), &mut out).unwrap();
    let tick_rate = summary.worker.ticks_applied as f64 / summary.elapsed_s;
    // At least twice the rate-limited target anywhere; over a million on real hardware.
    let floor = if on_ci() { 200_000.0 } else { 1_000_000.0 };
    assert!(
        tick_rate > floor,
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
