//! Without `#[global_allocator]`, counters stay at zero; `is_active` must expose that so a
//! "zero allocations" assertion can never pass vacuously.

use densa_alloc::{is_active, measure, CountingAllocator};
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;

#[test]
fn direct_calls_through_a_non_global_instance_are_counted() {
    let allocator = CountingAllocator::new(System);
    let layout = Layout::from_size_align(64, 16).unwrap();
    let ((), stats) = measure(|| {
        // SAFETY: `layout` has non-zero size; every pointer is checked for null, used
        // only with the layout it was allocated with, and freed exactly once.
        unsafe {
            let p = allocator.alloc(layout);
            assert!(!p.is_null());
            let p = allocator.realloc(p, layout, 128);
            assert!(!p.is_null());
            allocator.dealloc(p, Layout::from_size_align(128, 16).unwrap());

            let z = allocator.alloc_zeroed(layout);
            assert!(!z.is_null());
            assert!(std::slice::from_raw_parts(z, 64).iter().all(|&b| b == 0));
            allocator.dealloc(z, layout);
        }
    });
    assert_eq!(stats.allocs, 2);
    assert_eq!(stats.reallocs, 1);
    assert_eq!(stats.deallocs, 2);
    assert_eq!(stats.bytes_allocated, 64 + 128 + 64);
}

#[test]
fn reports_inactive_when_not_installed() {
    assert!(!is_active());
}

#[test]
fn measure_sees_nothing_when_not_installed() {
    let ((), stats) = measure(|| drop(black_box(Box::new(1u32))));
    assert!(stats.is_zero());
}
