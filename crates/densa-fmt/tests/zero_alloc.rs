//! Proves formatting never touches the heap.

use densa_alloc::{is_active, measure, CountingAllocator};
use densa_fmt::{write_fixed, write_grouped, FmtBuf};
use std::hint::black_box;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

#[test]
fn formatting_performs_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );
    let mut buf = FmtBuf::<64>::new();
    let ((), stats) = measure(|| {
        for i in 0..10_000u32 {
            let value = black_box(f64::from(i) * 1.37 - 5_000.0);
            buf.clear();
            write_fixed(&mut buf, value, 2).unwrap();
            buf.clear();
            write_fixed(&mut buf, value * 1e9, 6).unwrap();
            buf.clear();
            write_grouped(&mut buf, black_box(u64::from(i) * 987_654_321), ',').unwrap();
            black_box(buf.as_bytes());
        }
    });
    assert!(stats.is_zero(), "formatting allocated: {stats:?}");
}
