//! Keeping a sorted, filtered view of a million-row live table up to date.
//!
//! Run: `cargo bench -p densa-bench --bench live`

use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, Int64Array, RecordBatch};
use criterion::{criterion_group, BatchSize, Criterion, Throughput};
use densa_data::{Aggregate, CompareOp, Filter, GroupSpec, LiveTable, Order, Scalar, ViewSpec};

const ROWS: i64 = 1_000_000;
const UPDATE_ROWS: usize = 64;

fn batch(ids: Vec<i64>, prices: Vec<f64>) -> RecordBatch {
    RecordBatch::try_from_iter([
        ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
        ("price", Arc::new(Float64Array::from(prices)) as ArrayRef),
    ])
    .expect("valid batch")
}

/// A deterministic stream of `UPDATE_ROWS`-row batches updating random keys.
fn updates(count: usize) -> Vec<RecordBatch> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..count)
        .map(|_| {
            let ids = (0..UPDATE_ROWS)
                .map(|_| (next() % ROWS as u64) as i64)
                .collect();
            let prices = (0..UPDATE_ROWS)
                .map(|_| (next() % 100_000) as f64 / 100.0)
                .collect();
            batch(ids, prices)
        })
        .collect()
}

fn table_with_view() -> LiveTable {
    let ids: Vec<i64> = (0..ROWS).collect();
    let prices: Vec<f64> = ids
        .iter()
        .map(|&i| ((i * 7_919) % 100_000) as f64 / 100.0)
        .collect();
    let initial = batch(ids, prices);
    let mut table = LiveTable::new(initial.schema(), "id").expect("int key");
    table
        .add_view(
            ViewSpec::new()
                .sort_by("price", Order::Descending)
                .filter(Filter::compare("price", CompareOp::Gt, Scalar::Float(10.0))),
        )
        .expect("valid view");
    table.upsert(initial).expect("same schema");
    table
}

fn update_sorted_view(c: &mut Criterion) {
    let mut table = table_with_view();
    let stream = updates(4_096);
    let mut next = stream.iter().cycle();
    let mut group = c.benchmark_group("live_table_1m");
    group.throughput(Throughput::Elements(UPDATE_ROWS as u64));
    group.bench_function("upsert_64_rows_sorted_filtered_view", |b| {
        b.iter_batched(
            || next.next().cloned().expect("cycle"),
            |update| table.upsert(update).expect("same schema"),
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn build_view(c: &mut Criterion) {
    let table = table_with_view();
    c.bench_function("add_sorted_view_over_1m_rows", |b| {
        b.iter_batched(
            || table.clone(),
            |mut t| {
                t.add_view(ViewSpec::new().sort_by("price", Order::Ascending))
                    .expect("valid view")
            },
            BatchSize::LargeInput,
        )
    });
}

/// The same updates with aggregates over the whole table too: count, sum, min and max.
fn update_view_and_groups(c: &mut Criterion) {
    let mut table = table_with_view();
    table
        .add_groups(
            GroupSpec::new()
                .aggregate(Aggregate::count())
                .aggregate(Aggregate::sum("price"))
                .aggregate(Aggregate::min("price"))
                .aggregate(Aggregate::max("price")),
        )
        .expect("valid groups");
    let stream = updates(4_096);
    let mut next = stream.iter().cycle();
    let mut group = c.benchmark_group("live_table_1m");
    group.throughput(Throughput::Elements(UPDATE_ROWS as u64));
    group.bench_function("upsert_64_rows_view_and_aggregates", |b| {
        b.iter_batched(
            || next.next().cloned().expect("cycle"),
            |update| table.upsert(update).expect("same schema"),
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

criterion_group!(
    benches,
    update_sorted_view,
    update_view_and_groups,
    build_view
);

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
