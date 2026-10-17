//! Behaviour with the counting allocator installed as the global allocator.

use densa_alloc::{is_active, measure, thread_stats, AllocStats, CountingAllocator};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

#[test]
fn reports_active_when_installed() {
    assert!(is_active());
}

#[test]
fn counts_a_box_allocation_and_its_free() {
    let ((), stats) = measure(|| drop(black_box(Box::new(42u64))));
    assert_eq!(stats.allocs, 1);
    assert_eq!(stats.deallocs, 1);
    assert_eq!(stats.reallocs, 0);
    assert_eq!(stats.bytes_allocated, 8);
    assert!(!stats.is_zero());
}

#[test]
fn counts_zeroed_allocations() {
    let ((), stats) = measure(|| drop(black_box(vec![0u8; 4096])));
    assert_eq!(stats.allocs, 1, "vec![0; n] goes through alloc_zeroed");
    assert_eq!(stats.bytes_allocated, 4096);
}

#[test]
fn counts_reallocations_from_vec_growth() {
    let ((), stats) = measure(|| {
        let mut v: Vec<u64> = Vec::with_capacity(1);
        for i in 0..64 {
            v.push(black_box(i));
        }
        drop(black_box(v));
    });
    assert_eq!(stats.allocs, 1);
    assert!(stats.reallocs >= 1, "{stats:?}");
    assert_eq!(stats.deallocs, 1);
}

#[test]
fn allocation_free_code_measures_zero() {
    let (sum, stats) = measure(|| {
        let buf = [1u64; 64];
        black_box(&buf).iter().sum::<u64>()
    });
    assert_eq!(sum, 64);
    assert!(stats.is_zero(), "{stats:?}");
}

#[test]
fn returns_the_closure_result() {
    let (value, _) = measure(|| 7);
    assert_eq!(value, 7);
}

#[test]
fn other_threads_allocations_are_not_attributed_to_this_thread() {
    let go = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let (go2, done2) = (Arc::clone(&go), Arc::clone(&done));
    let worker = thread::spawn(move || {
        while !go2.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }
        for i in 0..1_000u32 {
            drop(black_box(Box::new(i)));
        }
        done2.store(true, Ordering::Release);
    });

    let ((), stats) = measure(|| {
        go.store(true, Ordering::Release);
        while !done.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }
    });
    worker.join().unwrap();
    assert!(
        stats.is_zero(),
        "worker allocations leaked into this thread: {stats:?}"
    );
}

#[test]
fn thread_stats_are_monotonic_and_since_gives_the_delta() {
    let before = thread_stats();
    drop(black_box(Box::new([0u8; 100])));
    let after = thread_stats();
    let delta = after.since(before);
    assert_eq!(delta.allocs, 1);
    assert_eq!(delta.bytes_allocated, 100);
    assert_eq!(
        AllocStats::default().since(AllocStats::default()),
        AllocStats::ZERO
    );
}
