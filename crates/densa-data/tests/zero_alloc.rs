//! Proves the ingestion hot path (generate -> apply -> snapshot) never touches the heap.

use densa_alloc::{is_active, measure, CountingAllocator};
use densa_data::{MarketTable, Tick, TickGenerator, ViewportSnapshot};
use std::hint::black_box;
use std::num::NonZeroU32;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

const SYMBOLS: u32 = 10_000;
const BATCH: usize = 1024;
const ROUNDS: u64 = if cfg!(miri) { 4 } else { 500 };

#[test]
fn generate_apply_snapshot_perform_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );

    // Startup: every buffer is allocated here, outside the measurement.
    let mut generator = TickGenerator::new(NonZeroU32::new(SYMBOLS).expect("non-zero"), 42);
    let mut table = MarketTable::new(SYMBOLS);
    let mut batch = vec![Tick::default(); BATCH];
    let mut snapshot = Box::new(ViewportSnapshot::<50>::new());

    let (applied, stats) = measure(|| {
        let mut applied = 0;
        for round in 0..ROUNDS {
            generator.fill(&mut batch, round);
            applied += table.apply_batch(&batch).applied;
            // Scroll the viewport around, including past the end.
            let first_row = black_box((round as u32).wrapping_mul(997) % (SYMBOLS + 100));
            table.snapshot_into(first_row, &mut snapshot);
            black_box(snapshot.visible());
        }
        applied
    });

    assert_eq!(applied, ROUNDS * BATCH as u64);
    assert!(stats.is_zero(), "hot path allocated: {stats:?}");
}
