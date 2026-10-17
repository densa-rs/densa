//! Ingestion hot path costs, including the SoA vs AoS layout comparison.
//!
//! Run: `cargo bench -p densa-data --bench ingest`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use densa_data::{Direction, MarketTable, Tick, TickGenerator, ViewportSnapshot};
use std::hint::black_box;

const BATCH: usize = 1024;
/// Pre-generated batches cycled through so the bench never measures the generator.
const BATCHES: usize = 64;
/// 10k symbols fit in L2; 1M symbols (~56 MB) exceed this machine's 33 MB L3.
const SYMBOL_COUNTS: [u32; 2] = [10_000, 1_000_000];

/// Array-of-structs twin of `MarketTable`, with identical update logic, for comparison.
#[derive(Clone, Copy)]
struct Row {
    bid: f64,
    ask: f64,
    last: f64,
    volume: u64,
    updated_at_ns: u64,
    last_change_ns: u64,
    direction: Direction,
}

struct AosTable {
    rows: Box<[Row]>,
}

impl AosTable {
    fn new(symbols: u32) -> Self {
        let empty = Row {
            bid: f64::NAN,
            ask: f64::NAN,
            last: f64::NAN,
            volume: 0,
            updated_at_ns: 0,
            last_change_ns: 0,
            direction: Direction::Flat,
        };
        Self {
            rows: vec![empty; symbols as usize].into_boxed_slice(),
        }
    }

    fn apply_batch(&mut self, ticks: &[Tick]) -> u64 {
        let mut applied = 0;
        for tick in ticks {
            let Some(row) = self.rows.get_mut(tick.symbol_id as usize) else {
                continue;
            };
            if !(tick.bid.is_finite() && tick.ask.is_finite() && tick.last.is_finite()) {
                continue;
            }
            if tick.last > row.last {
                row.direction = Direction::Up;
                row.last_change_ns = tick.ts_ns;
            } else if tick.last < row.last {
                row.direction = Direction::Down;
                row.last_change_ns = tick.ts_ns;
            }
            row.bid = tick.bid;
            row.ask = tick.ask;
            row.last = tick.last;
            row.volume = row.volume.saturating_add(tick.qty);
            row.updated_at_ns = tick.ts_ns;
            applied += 1;
        }
        applied
    }
}

fn pregenerated(symbols: u32) -> Vec<Vec<Tick>> {
    let mut generator = TickGenerator::new(symbols, 42);
    (0..BATCHES)
        .map(|i| {
            let mut batch = vec![Tick::default(); BATCH];
            generator.fill(&mut batch, i as u64 + 1);
            batch
        })
        .collect()
}

fn apply_layouts(c: &mut Criterion) {
    let mut group = c.benchmark_group("apply_batch_1024");
    group.throughput(Throughput::Elements(BATCH as u64));
    for symbols in SYMBOL_COUNTS {
        let batches = pregenerated(symbols);

        let mut soa = MarketTable::new(symbols);
        let mut i = 0;
        group.bench_with_input(BenchmarkId::new("soa", symbols), &symbols, |b, _| {
            b.iter(|| {
                i = (i + 1) % BATCHES;
                black_box(soa.apply_batch(black_box(&batches[i])))
            })
        });

        let mut aos = AosTable::new(symbols);
        let mut j = 0;
        group.bench_with_input(BenchmarkId::new("aos", symbols), &symbols, |b, _| {
            b.iter(|| {
                j = (j + 1) % BATCHES;
                black_box(aos.apply_batch(black_box(&batches[j])))
            })
        });
    }
    group.finish();
}

/// Generates in the loop so accesses cover the whole table uniformly (the pre-generated
/// batches above touch only ~64k distinct symbols, which flatters a 1M-row table).
/// Both layouts pay the identical generator cost.
fn generate_and_apply_layouts(c: &mut Criterion) {
    let mut group = c.benchmark_group("generate_and_apply_1024");
    group.throughput(Throughput::Elements(BATCH as u64));
    for symbols in SYMBOL_COUNTS {
        let mut batch = vec![Tick::default(); BATCH];

        let mut generator = TickGenerator::new(symbols, 42);
        let mut soa = MarketTable::new(symbols);
        group.bench_with_input(BenchmarkId::new("soa", symbols), &symbols, |b, _| {
            b.iter(|| {
                generator.fill(&mut batch, 1);
                black_box(soa.apply_batch(&batch))
            })
        });

        let mut generator = TickGenerator::new(symbols, 42);
        let mut aos = AosTable::new(symbols);
        group.bench_with_input(BenchmarkId::new("aos", symbols), &symbols, |b, _| {
            b.iter(|| {
                generator.fill(&mut batch, 1);
                black_box(aos.apply_batch(&batch))
            })
        });
    }
    group.finish();
}

fn generate(c: &mut Criterion) {
    let mut group = c.benchmark_group("generator_fill_1024");
    group.throughput(Throughput::Elements(BATCH as u64));
    let mut generator = TickGenerator::new(10_000, 7);
    let mut batch = vec![Tick::default(); BATCH];
    group.bench_function("10000", |b| {
        b.iter(|| generator.fill(black_box(&mut batch), 1))
    });
    group.finish();
}

fn snapshot(c: &mut Criterion) {
    let symbols = 10_000;
    let mut table = MarketTable::new(symbols);
    for batch in pregenerated(symbols) {
        table.apply_batch(&batch);
    }
    let mut snap = Box::new(ViewportSnapshot::<50>::new());
    let mut first_row = 0u32;
    c.bench_function("snapshot_into_50_rows", |b| {
        b.iter(|| {
            first_row = (first_row + 997) % symbols;
            table.snapshot_into(black_box(first_row), &mut snap);
            black_box(&snap);
        })
    });
}

criterion_group!(
    benches,
    apply_layouts,
    generate_and_apply_layouts,
    generate,
    snapshot
);
criterion_main!(benches);
