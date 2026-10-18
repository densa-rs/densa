//! Proves reading a view, every frame, never touches the heap.

use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, Int64Array, RecordBatch};
use densa_alloc::{is_active, measure, CountingAllocator};
use densa_data::{CompareOp, Filter, LiveTable, Order, Scalar, ViewSpec};
use std::hint::black_box;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

#[test]
fn reading_a_sorted_filtered_view_performs_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );
    let ids: Vec<i64> = (0..10_000).collect();
    let prices: Vec<f64> = ids.iter().map(|&i| ((i * 7_919) % 1_000) as f64).collect();
    let batch = RecordBatch::try_from_iter([
        ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
        ("price", Arc::new(Float64Array::from(prices)) as ArrayRef),
    ])
    .unwrap();
    let mut table = LiveTable::new(batch.schema(), "id").unwrap();
    let view = table
        .add_view(
            ViewSpec::new()
                .sort_by("price", Order::Descending)
                .filter(Filter::compare(
                    "price",
                    CompareOp::Gt,
                    Scalar::Float(100.0),
                )),
        )
        .unwrap();
    table.upsert(batch).unwrap();

    let ((), stats) = measure(|| {
        for frame in 0..1_000 {
            let rows = table.view(view).unwrap();
            let first = (frame * 97) % rows.len();
            for index in first..(first + 50).min(rows.len()) {
                black_box(rows.row(index));
                // A grid finding its selected record again.
                let record = rows.record(index).unwrap();
                black_box(rows.position(record));
                black_box(table.row_of(record));
            }
        }
    });
    assert!(stats.is_zero(), "reading a view allocated: {stats:?}");
}
