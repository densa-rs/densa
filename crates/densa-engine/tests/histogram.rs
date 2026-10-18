//! Latency histogram: exact small values, bounded relative error, merge and reset.

use densa_engine::{LatencyHistogram, LatencySummary};

fn assert_within_bucket_error(actual: u64, expected: u64) {
    // Upper bucket bound: never below the true value, at most 1/16 above it.
    assert!(actual >= expected, "{actual} < {expected}");
    let slack = expected / 16 + 1;
    assert!(
        actual - expected <= slack,
        "{actual} too far above {expected}"
    );
}

#[test]
fn empty_histogram_reports_nothing() {
    let h = LatencyHistogram::new();
    assert_eq!(h.count(), 0);
    assert_eq!(h.min(), None);
    assert_eq!(h.max(), None);
    assert_eq!(h.mean(), None);
    assert_eq!(h.quantile(0.5), None);
    assert_eq!(h.summary(), LatencySummary::default());
}

#[test]
fn small_values_are_exact() {
    let mut h = LatencyHistogram::new();
    for v in 0..16 {
        h.record(v);
    }
    assert_eq!(h.count(), 16);
    assert_eq!(h.min(), Some(0));
    assert_eq!(h.max(), Some(15));
    assert_eq!(h.quantile(0.5), Some(7));
    assert_eq!(h.quantile(1.0), Some(15));
}

#[test]
fn quantiles_are_within_one_sixteenth_over_the_full_range() {
    let mut h = LatencyHistogram::new();
    let values: Vec<u64> = (1..=10_000u64).map(|i| i * 1_237).collect();
    for &v in &values {
        h.record(v);
    }
    for q in [0.01, 0.25, 0.5, 0.9, 0.99, 0.999] {
        let rank = ((q * values.len() as f64).ceil() as usize).max(1);
        let expected = values[rank - 1];
        assert_within_bucket_error(h.quantile(q).unwrap(), expected);
    }
    assert_eq!(
        h.quantile(1.0),
        Some(*values.last().unwrap()),
        "p100 is the exact max"
    );
}

#[test]
fn extreme_values_do_not_panic() {
    let mut h = LatencyHistogram::new();
    h.record(u64::MAX);
    h.record(0);
    assert_eq!(h.max(), Some(u64::MAX));
    assert_eq!(h.quantile(1.0), Some(u64::MAX));
    assert_eq!(h.quantile(0.0), Some(0));
}

#[test]
fn quantile_argument_is_clamped() {
    let mut h = LatencyHistogram::new();
    h.record(100);
    assert_eq!(h.quantile(-1.0), h.quantile(0.0));
    assert_eq!(h.quantile(2.0), h.quantile(1.0));
    assert_eq!(h.quantile(f64::NAN), h.quantile(0.0));
}

#[test]
fn mean_min_max_and_summary() {
    let mut h = LatencyHistogram::new();
    for v in [10, 20, 30, 1_000] {
        h.record(v);
    }
    assert_eq!(h.mean(), Some(265.0));
    let s = h.summary();
    assert_eq!(s.count, 4);
    assert_eq!(s.max, 1_000);
    assert_within_bucket_error(s.p50, 20);
    assert_eq!(s.p99, 1_000);
}

#[test]
fn merge_combines_and_reset_clears() {
    let mut a = LatencyHistogram::new();
    let mut b = LatencyHistogram::new();
    a.record(5);
    b.record(500);
    b.record(7);
    a.merge(&b);
    assert_eq!(a.count(), 3);
    assert_eq!(a.min(), Some(5));
    assert_eq!(a.max(), Some(500));

    a.reset();
    assert_eq!(a.count(), 0);
    assert_eq!(a.max(), None);
}

#[test]
fn merging_an_empty_histogram_changes_nothing() {
    let mut a = LatencyHistogram::default();
    a.record(42);
    a.merge(&LatencyHistogram::new());
    assert_eq!((a.count(), a.min(), a.max()), (1, Some(42), Some(42)));
}
