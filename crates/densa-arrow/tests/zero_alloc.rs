//! Proves painting frames of an Arrow table, scrolled anywhere, never touches the heap.

use std::sync::Arc;

use arrow_array::types::Int32Type;
use arrow_array::{
    ArrayRef, BooleanArray, Decimal128Array, DictionaryArray, Float64Array, Int64Array,
    RecordBatch, StringArray, TimestampMillisecondArray,
};
use densa_alloc::{is_active, measure, CountingAllocator};
use densa_arrow::ArrowTable;
use densa_draw::{Canvas, InstanceBuffer};
use densa_grid::{GridPainter, Palette};
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

#[test]
fn painting_a_live_view_between_changes_performs_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );
    let mut table = densa_data::LiveTable::new(batch(0, 1).schema(), "id").unwrap();
    let by_price = table
        .add_view(densa_data::ViewSpec::new().sort_by("price", densa_data::Order::Descending))
        .unwrap();
    for start in (0..ROWS).step_by(BATCH as usize) {
        table.upsert(batch(start, BATCH.min(ROWS - start))).unwrap();
    }
    let mut grid = densa_arrow::LiveGrid::new(&table);
    grid.set_decimals(3, Some(2)).unwrap();
    let palette = Palette::new();
    let font = Font::bundled_mono().unwrap();
    let mut painter = GridPainter::new(grid.columns(), 50, font, 13.0, palette);
    let mut buf = InstanceBuffer::with_capacity(painter.layout().max_instances());
    let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
    let printable: String = (' '..='~').chain(['…']).collect();
    painter.prewarm(&mut cache, &printable);

    let mut paint = |grid: &mut densa_arrow::LiveGrid, cache: &mut GlyphCache| {
        for frame in 0..1_000 {
            let first_row = (frame * 7_919) % ROWS as usize;
            let view = grid.view(&table, by_price, first_row, &palette).unwrap();
            let mut canvas = Canvas::begin(cache, &mut buf);
            painter.paint(&view, &mut canvas);
        }
    };
    paint(&mut grid, &mut cache); // warm-up: every glyph at every subpixel position
    cache.drain_updates().for_each(drop);

    let ((), stats) = measure(|| paint(&mut grid, &mut cache));
    assert!(stats.is_zero(), "painting a live view allocated: {stats:?}");
}
