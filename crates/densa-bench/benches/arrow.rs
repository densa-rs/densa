//! Painting a 50-row frame of a million-row Arrow table, scrolled to a new place each frame.
//!
//! Run: `cargo bench -p densa-bench --bench arrow`

use std::sync::Arc;

use arrow_array::types::Int32Type;
use arrow_array::{
    ArrayRef, Decimal128Array, DictionaryArray, Float64Array, Int64Array, RecordBatch, StringArray,
    TimestampMillisecondArray,
};
use criterion::{criterion_group, Criterion};
use densa_arrow::ArrowTable;
use densa_draw::{Canvas, InstanceBuffer};
use densa_grid::{GridPainter, Palette};
use densa_text::{AtlasConfig, Font, GlyphCache};
use std::hint::black_box;

const ROWS: usize = 1_000_000;
const BATCH_ROWS: usize = 65_536;
const VISIBLE_ROWS: u32 = 50;

fn batch(start: usize, len: usize) -> RecordBatch {
    let ids: Vec<i64> = (start..start + len).map(|i| i as i64).collect();
    let names: Vec<String> = ids
        .iter()
        .map(|i| format!("customer-{:05}", i % 50_000))
        .collect();
    let regions: DictionaryArray<Int32Type> = ids
        .iter()
        .map(|i| ["north", "south", "east", "west"][(i % 4) as usize])
        .collect();
    let prices: Vec<f64> = ids.iter().map(|&i| (i % 100_000) as f64 / 100.0).collect();
    let totals =
        Decimal128Array::from(ids.iter().map(|&i| i128::from(i) * 101).collect::<Vec<_>>())
            .with_precision_and_scale(18, 2)
            .expect("valid decimal");
    let times: Vec<i64> = ids.iter().map(|&i| 1_760_000_000_000 + i * 1_250).collect();
    RecordBatch::try_from_iter([
        ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
        ("customer", Arc::new(StringArray::from(names)) as ArrayRef),
        ("region", Arc::new(regions) as ArrayRef),
        ("price", Arc::new(Float64Array::from(prices)) as ArrayRef),
        ("total", Arc::new(totals) as ArrayRef),
        (
            "time",
            Arc::new(TimestampMillisecondArray::from(times)) as ArrayRef,
        ),
    ])
    .expect("valid batch")
}

fn paint_frames(c: &mut Criterion) {
    let batches = (0..ROWS)
        .step_by(BATCH_ROWS)
        .map(|start| batch(start, BATCH_ROWS.min(ROWS - start)));
    let mut table = ArrowTable::from_batches(batches).expect("one schema");
    table.set_decimals(3, Some(2)).expect("price column");
    let palette = Palette::new();
    let font = Font::bundled_mono().expect("bundled font");
    let mut painter = GridPainter::new(table.columns(), VISIBLE_ROWS, font, 13.0, palette);
    let mut instances = InstanceBuffer::with_capacity(painter.layout().max_instances());
    let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
    let printable: String = (' '..='~').chain(['…']).collect();
    painter.prewarm(&mut cache, &printable);

    let mut first_row = 0;
    let mut frame = || {
        // A large prime stride: every frame lands in another batch and region of the table.
        first_row = (first_row + 7_919) % (ROWS - VISIBLE_ROWS as usize);
        let mut canvas = Canvas::begin(&mut cache, &mut instances);
        painter.paint(&table.view(first_row, &palette), &mut canvas);
        black_box(instances.as_slice());
    };
    for _ in 0..1_000 {
        frame(); // warm every glyph at every subpixel position
    }
    c.bench_function("arrow_paint_50_rows_of_1m", |b| b.iter(&mut frame));
}

criterion_group!(benches, paint_frames);

fn main() {
    use densa_platform::{logical_cpus, pick_performance_cpu, pin_current_thread};
    match logical_cpus().map(|cpus| pick_performance_cpu(&cpus)) {
        Ok(Some(cpu)) => match pin_current_thread(cpu) {
            Ok(()) => eprintln!("bench thread pinned to cpu {}", cpu.id),
            Err(e) => eprintln!("warning: bench thread not pinned: {e}"),
        },
        Ok(None) => eprintln!("warning: bench thread not pinned: no CPUs reported"),
        Err(e) => eprintln!("warning: bench thread not pinned: {e}"),
    }
    benches();
    Criterion::default().configure_from_args().final_summary();
}
