//! Per-operation cost of the triple buffer, uncontended and with a live reader.
//!
//! Run: `cargo bench -p densa-sync --bench triple_buffer`

use criterion::{criterion_group, criterion_main, Criterion};
use densa_sync::triple_buffer;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

/// Roughly a 50-row viewport snapshot (50 rows x 56 bytes).
type Payload = [u64; 350];

fn uncontended(c: &mut Criterion) {
    let (mut writer, mut reader) = triple_buffer::<u64>(0);
    c.bench_function("publish_u64", |b| {
        b.iter(|| {
            writer.write(|v| *v = black_box(1));
            writer.publish();
        })
    });
    c.bench_function("update_and_read_u64", |b| {
        b.iter(|| {
            writer.publish();
            black_box(reader.update());
            black_box(reader.read(|v| *v))
        })
    });
    c.bench_function("update_no_news", |b| b.iter(|| black_box(reader.update())));
}

fn publish_with_live_reader(c: &mut Criterion) {
    let (mut writer, mut reader) = triple_buffer::<Payload>([0; 350]);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_reader = Arc::clone(&stop);
    // A reader spinning on another core keeps the shared cache line bouncing: this is the
    // worst case the writer sees in production.
    let spinner = thread::spawn(move || {
        while !stop_reader.load(Ordering::Relaxed) {
            if reader.update() {
                black_box(reader.read(|p| p[0]));
            }
        }
    });

    let mut seq = 0u64;
    c.bench_function("fill_and_publish_2_8kb_contended", |b| {
        b.iter(|| {
            seq += 1;
            writer.write(|p| p.fill(seq));
            writer.publish();
        })
    });

    stop.store(true, Ordering::Relaxed);
    spinner.join().unwrap();
}

criterion_group!(benches, uncontended, publish_with_live_reader);
criterion_main!(benches);
