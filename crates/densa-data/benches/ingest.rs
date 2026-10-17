//! Ingestion hot path costs, including the table layout comparison that chose AoS.
//!
//! Run: `cargo bench -p densa-data --bench ingest`
//!
//! Ticks are generated inside the timed loop so symbol accesses cover the whole table
//! uniformly, as a real feed does. (Cycling through a few pre-generated batches touches only
//! ~64k distinct symbols and flatters any layout on a 1M-row table.) Every variant pays the
//! same generator cost; `generator_fill_1024` measures it alone for subtraction.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use densa_data::{Direction, MarketTable, Tick, TickGenerator, ViewportSnapshot};
use std::hint::black_box;

const BATCH: usize = 1024;
/// 10k symbols fit in L2; 1M symbols (~56-64 MB) exceed this machine's 33 MB L3.
const SYMBOL_COUNTS: [u32; 2] = [10_000, 1_000_000];

/// Shared update rule of the comparison twins; identical to `MarketTable::apply`.
#[inline(always)]
fn valid(tick: &Tick) -> bool {
    tick.bid.is_finite() && tick.ask.is_finite() && tick.last.is_finite()
}

/// Structure-of-arrays twin: one cache miss per column per tick on large tables.
struct SoaTable {
    bid: Box<[f64]>,
    ask: Box<[f64]>,
    last: Box<[f64]>,
    volume: Box<[u64]>,
    updated_at_ns: Box<[u64]>,
    last_change_ns: Box<[u64]>,
    direction: Box<[Direction]>,
}

impl SoaTable {
    fn new(symbols: u32) -> Self {
        let n = symbols as usize;
        Self {
            bid: vec![f64::NAN; n].into(),
            ask: vec![f64::NAN; n].into(),
            last: vec![f64::NAN; n].into(),
            volume: vec![0; n].into(),
            updated_at_ns: vec![0; n].into(),
            last_change_ns: vec![0; n].into(),
            direction: vec![Direction::Flat; n].into(),
        }
    }

    fn apply_batch(&mut self, ticks: &[Tick]) -> u64 {
        let mut applied = 0;
        for tick in ticks {
            let i = tick.symbol_id as usize;
            if i >= self.last.len() || !valid(tick) {
                continue;
            }
            if tick.last > self.last[i] {
                self.direction[i] = Direction::Up;
                self.last_change_ns[i] = tick.ts_ns;
            } else if tick.last < self.last[i] {
                self.direction[i] = Direction::Down;
                self.last_change_ns[i] = tick.ts_ns;
            }
            self.bid[i] = tick.bid;
            self.ask[i] = tick.ask;
            self.last[i] = tick.last;
            self.volume[i] = self.volume[i].saturating_add(tick.qty);
            self.updated_at_ns[i] = tick.ts_ns;
            applied += 1;
        }
        applied
    }
}

/// Packed array-of-structs row: 56 bytes, so about half the rows straddle two cache lines.
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

/// The same row padded to exactly one 64-byte cache line.
#[derive(Clone, Copy)]
#[repr(align(64))]
struct LineRow(Row);

trait RowSlot: Copy {
    fn wrap(row: Row) -> Self;
    fn row(&mut self) -> &mut Row;
}

impl RowSlot for Row {
    fn wrap(row: Row) -> Self {
        row
    }
    fn row(&mut self) -> &mut Row {
        self
    }
}

impl RowSlot for LineRow {
    fn wrap(row: Row) -> Self {
        Self(row)
    }
    fn row(&mut self) -> &mut Row {
        &mut self.0
    }
}

struct AosTable<R> {
    rows: Box<[R]>,
}

impl<R: RowSlot> AosTable<R> {
    fn new(symbols: u32) -> Self {
        let empty = R::wrap(Row {
            bid: f64::NAN,
            ask: f64::NAN,
            last: f64::NAN,
            volume: 0,
            updated_at_ns: 0,
            last_change_ns: 0,
            direction: Direction::Flat,
        });
        Self {
            rows: vec![empty; symbols as usize].into(),
        }
    }

    fn apply_batch(&mut self, ticks: &[Tick]) -> u64 {
        let mut applied = 0;
        for tick in ticks {
            let Some(slot) = self.rows.get_mut(tick.symbol_id as usize) else {
                continue;
            };
            if !valid(tick) {
                continue;
            }
            let row = slot.row();
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

/// Benches one layout: generate a batch, apply it.
fn bench_layout(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &str,
    symbols: u32,
    mut apply: impl FnMut(&[Tick]) -> u64,
) {
    let mut generator = TickGenerator::new(symbols, 42);
    let mut batch = vec![Tick::default(); BATCH];
    group.bench_with_input(BenchmarkId::new(name, symbols), &symbols, |b, _| {
        b.iter(|| {
            generator.fill(&mut batch, 1);
            black_box(apply(&batch))
        })
    });
}

fn layouts(c: &mut Criterion) {
    let mut group = c.benchmark_group("generate_and_apply_1024");
    group.throughput(Throughput::Elements(BATCH as u64));
    for symbols in SYMBOL_COUNTS {
        let mut table = MarketTable::new(symbols);
        bench_layout(&mut group, "market_table", symbols, |t| {
            table.apply_batch(t).applied
        });
        let mut soa = SoaTable::new(symbols);
        bench_layout(&mut group, "soa", symbols, |t| soa.apply_batch(t));
        let mut aos = AosTable::<Row>::new(symbols);
        bench_layout(&mut group, "aos_56b", symbols, |t| aos.apply_batch(t));
        let mut aos_line = AosTable::<LineRow>::new(symbols);
        bench_layout(&mut group, "aos_64b", symbols, |t| aos_line.apply_batch(t));
    }
    group.finish();
}

fn generate(c: &mut Criterion) {
    let mut group = c.benchmark_group("generator_fill_1024");
    group.throughput(Throughput::Elements(BATCH as u64));
    for symbols in SYMBOL_COUNTS {
        let mut generator = TickGenerator::new(symbols, 7);
        let mut batch = vec![Tick::default(); BATCH];
        group.bench_with_input(BenchmarkId::from_parameter(symbols), &symbols, |b, _| {
            b.iter(|| generator.fill(black_box(&mut batch), 1))
        });
    }
    group.finish();
}

fn snapshot(c: &mut Criterion) {
    let symbols = 10_000;
    let mut table = MarketTable::new(symbols);
    let mut generator = TickGenerator::new(symbols, 3);
    let mut batch = vec![Tick::default(); BATCH];
    for round in 1..=64 {
        generator.fill(&mut batch, round);
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

criterion_group!(benches, layouts, generate, snapshot);
criterion_main!(benches);
