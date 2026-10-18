//! Proves building a frame's instances never touches the heap.

use densa_alloc::{is_active, measure, CountingAllocator};
use densa_data::{Direction, RowSnapshot, ViewportSnapshot};
use densa_render::{GlyphAtlas, GridLayout, GridPainter, InstanceBuffer, FLASH_NS};
use std::hint::black_box;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

#[test]
fn painting_frames_performs_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );

    // Startup allocations: painter, atlas, buffer, snapshot.
    let layout = GridLayout::new(50);
    let mut painter = GridPainter::new(layout, &GlyphAtlas::font8x8());
    let mut buf = InstanceBuffer::with_capacity(layout.max_instances());
    let mut snap = Box::new(ViewportSnapshot::<50>::new());
    for (i, row) in snap.rows.iter_mut().enumerate() {
        *row = RowSnapshot {
            symbol_id: i as u32,
            bid: 100.0 + i as f64,
            ask: 100.5 + i as f64,
            last: if i.is_multiple_of(7) {
                f64::NAN
            } else {
                100.25 + i as f64
            },
            volume: (i as u64) * 1_000_003,
            updated_at_ns: 1,
            last_change_ns: (i as u64) * FLASH_NS / 50,
            direction: [Direction::Up, Direction::Down, Direction::Flat][i % 3],
        };
    }
    snap.len = 50;
    snap.seq = 1;

    let ((), stats) = measure(|| {
        for frame in 0..1_000u64 {
            painter.paint(&snap, black_box(frame * 1_000_000), &mut buf);
            black_box(buf.as_slice());
        }
    });
    assert!(stats.is_zero(), "painting allocated: {stats:?}");
    assert_eq!(buf.dropped(), 0);
}
