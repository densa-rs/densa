//! Proves the full ingestion worker loop never touches the heap after startup.

use densa_alloc::{is_active, measure, CountingAllocator};
use std::time::{Duration, Instant};
use trading_demo::engine::{spawn_worker, Clock, FrameProbe, WorkerConfig};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

#[test]
fn worker_loop_and_reader_probe_perform_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );

    let clock = Clock::new();
    let config = WorkerConfig {
        symbols: 10_000,
        batch_size: 256,
        rate: None,
        publish_interval_ns: 0,
        ..WorkerConfig::default()
    };
    let (handle, mut reader) = spawn_worker::<50>(config, clock).unwrap();
    let mut probe = FrameProbe::new();

    let deadline = Instant::now() + Duration::from_millis(300);
    let ((), reader_stats) = measure(|| {
        let mut row = 0u32;
        while Instant::now() < deadline {
            let fresh = reader.update();
            reader.read(|snap| probe.observe(snap, fresh, clock.now_ns()));
            row = (row + 37) % 10_000;
            handle.set_viewport(row);
        }
    });

    let metrics = handle.stop().unwrap();
    assert!(metrics.batches > 100, "worker barely ran: {metrics:?}");
    assert_eq!(metrics.heap_ops, 0, "worker loop allocated: {metrics:?}");
    assert!(
        reader_stats.is_zero(),
        "reader loop allocated: {reader_stats:?}"
    );
    assert!(probe.report().fresh_frames > 0);
}
