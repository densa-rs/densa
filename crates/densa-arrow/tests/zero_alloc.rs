//! Proves painting frames of an Arrow table, scrolled anywhere, never touches the heap.

use std::sync::Arc;

use arrow_array::types::Int32Type;
use arrow_array::{
    ArrayRef, BooleanArray, Decimal128Array, DictionaryArray, Float64Array, Int64Array,
    RecordBatch, StringArray, TimestampMillisecondArray,
};
use densa_alloc::{is_active, measure, CountingAllocator};
use densa_arrow::ArrowTable;
use densa_render::{Canvas, GridPainter, InstanceBuffer, Palette};
use densa_text::{AtlasConfig, Font, GlyphCache};
use std::hint::black_box;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

const ROWS: i64 = 100_000;
const BATCH: i64 = 8_192;

fn batch(start: i64, len: i64) -> RecordBatch {
    let ids: Vec<i64> = (start..start + len).collect();
    let names: Vec<String> = ids.iter().map(|i| format!("item {i}")).collect();
    let regions: DictionaryArray<Int32Type> = ids
        .iter()
        .map(|i| ["north", "south", "east", "west"][(i % 4) as usize])
        .collect();
    let prices: Vec<Option<f64>> = ids
        .iter()
        .map(|&i| (i % 7 != 0).then_some(i as f64 * 0.37))
        .collect();
    let amounts =
        Decimal128Array::from(ids.iter().map(|&i| i128::from(i) * 101).collect::<Vec<_>>())
            .with_precision_and_scale(20, 2)
            .unwrap();
    let times: Vec<i64> = ids.iter().map(|&i| 1_700_000_000_000 + i * 250).collect();
    RecordBatch::try_from_iter([
        ("id", Arc::new(Int64Array::from(ids.clone())) as ArrayRef),
        ("name", Arc::new(StringArray::from(names)) as ArrayRef),
        ("region", Arc::new(regions) as ArrayRef),
        ("price", Arc::new(Float64Array::from(prices)) as ArrayRef),
        ("amount", Arc::new(amounts) as ArrayRef),
        (
            "time",
            Arc::new(TimestampMillisecondArray::from(times)) as ArrayRef,
        ),
        (
            "active",
            Arc::new(BooleanArray::from(
                ids.iter().map(|i| i % 3 == 0).collect::<Vec<_>>(),
            )) as ArrayRef,
        ),
    ])
    .unwrap()
}

#[test]
fn painting_scrolled_frames_of_an_arrow_table_performs_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );
    let batches = (0..ROWS)
        .step_by(BATCH as usize)
        .map(|s| batch(s, BATCH.min(ROWS - s)));
    let mut table = ArrowTable::from_batches(batches).unwrap();
    table.set_decimals(3, Some(2)).unwrap();
    let palette = Palette::new();
    let font = Font::bundled_mono().unwrap();
    let mut painter = GridPainter::new(table.columns(), 50, font, 13.0, palette);
    let mut buf = InstanceBuffer::with_capacity(painter.layout().max_instances());
    let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
    let printable: String = (' '..='~').chain(['…']).collect();
    painter.prewarm(&mut cache, &printable);
    cache.drain_updates().for_each(drop);

    let first_rows = (0..1_000).map(|frame| (frame * 7_919) % ROWS as usize);
    let paint = |painter: &mut GridPainter, cache: &mut GlyphCache, buf: &mut InstanceBuffer| {
        for first_row in first_rows.clone() {
            let mut canvas = Canvas::begin(cache, buf);
            painter.paint(&table.view(first_row, &palette), &mut canvas);
            black_box(buf.as_slice());
        }
    };
    paint(&mut painter, &mut cache, &mut buf); // warm-up: every subpixel position drawn once
    cache.drain_updates().for_each(drop);

    let ((), stats) = measure(|| paint(&mut painter, &mut cache, &mut buf));
    assert!(stats.is_zero(), "painting allocated: {stats:?}");
    assert_eq!(buf.dropped(), 0);
}
