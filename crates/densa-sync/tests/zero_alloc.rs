//! Proves the triple buffer hot path never touches the heap, on both threads.
#![cfg(not(loom))]

use densa_alloc::{is_active, measure, CountingAllocator};
use densa_sync::triple_buffer;
use std::hint::black_box;
use std::thread;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

const PUBLISHES: u64 = if cfg!(miri) { 200 } else { 100_000 };

#[test]
fn write_publish_update_read_perform_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );

    // Construction allocates (Arc + slots); that is startup, not hot path.
    let (mut writer, mut reader) = triple_buffer([0u64; 64]);

    let producer = thread::spawn(move || {
        let ((), stats) = measure(|| {
            for seq in 1..=PUBLISHES {
                writer.write(|buf| buf.fill(black_box(seq)));
                writer.publish();
            }
        });
        stats
    });

    let (last_seen, reader_stats) = measure(|| {
        let mut last = 0;
        while last < PUBLISHES {
            if reader.update() {
                last = reader.read(|buf| buf[0]);
            } else {
                std::hint::spin_loop();
            }
        }
        last
    });

    let writer_stats = producer.join().expect("producer panicked");
    assert_eq!(last_seen, PUBLISHES);
    assert!(writer_stats.is_zero(), "writer allocated: {writer_stats:?}");
    assert!(reader_stats.is_zero(), "reader allocated: {reader_stats:?}");
}
