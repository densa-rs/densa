//! Pacer: evenly spaced, drift-free deadlines and lag recovery.

use std::num::NonZeroU64;

use densa_engine::{Pacer, MAX_PACER_LAG_NS};

fn rate(n: u64) -> NonZeroU64 {
    NonZeroU64::new(n).expect("test rates are non-zero")
}

const SECOND: u64 = 1_000_000_000;

#[test]
fn first_batch_may_start_immediately() {
    let pacer = Pacer::new(rate(1_000), 10, 500);
    assert_eq!(pacer.deadline_ns(), 500);
}

#[test]
fn deadlines_are_evenly_spaced() {
    // 1000 ticks/s in batches of 10 => one batch every 10 ms.
    let mut pacer = Pacer::new(rate(1_000), 10, 0);
    for k in 1..=5u64 {
        let start = pacer.deadline_ns();
        pacer.complete_batch(start);
        assert_eq!(pacer.deadline_ns(), k * 10_000_000);
    }
}

#[test]
fn fractional_intervals_do_not_drift() {
    // 3 ticks/s, batch 1 => 333_333_333.3.. ns per batch; after 3 batches exactly 1 s.
    let mut pacer = Pacer::new(rate(3), 1, 0);
    for _ in 0..3 {
        let start = pacer.deadline_ns();
        pacer.complete_batch(start);
    }
    assert_eq!(pacer.deadline_ns(), SECOND);

    // Long run: 200k ticks/s in batches of 64 over 1 simulated hour stays exact.
    let mut pacer = Pacer::new(rate(200_000), 64, 0);
    let batches = 200_000 * 3_600 / 64;
    for _ in 0..batches {
        let start = pacer.deadline_ns();
        pacer.complete_batch(start);
    }
    assert_eq!(pacer.deadline_ns(), 3_600 * SECOND);
}

#[test]
fn small_lag_is_caught_up_without_reset() {
    let mut pacer = Pacer::new(rate(1_000), 10, 0);
    pacer.complete_batch(MAX_PACER_LAG_NS / 2);
    assert_eq!(pacer.lag_resets(), 0);
    assert_eq!(pacer.deadline_ns(), 10_000_000, "schedule unchanged");
}

#[test]
fn large_lag_rebases_instead_of_bursting() {
    let mut pacer = Pacer::new(rate(1_000), 10, 0);
    let stalled_until = 10_000_000 + MAX_PACER_LAG_NS + 1;
    pacer.complete_batch(stalled_until);
    assert_eq!(pacer.lag_resets(), 1);
    assert_eq!(
        pacer.deadline_ns(),
        stalled_until,
        "next batch starts now, not a backlog"
    );
    pacer.complete_batch(stalled_until);
    assert_eq!(pacer.deadline_ns(), stalled_until + 10_000_000);
}
