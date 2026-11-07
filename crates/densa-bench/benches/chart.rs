//! Chart costs: a frame of a line far denser than the plot is wide, drawn and scaled.
//!
//! Run: `cargo bench -p densa-bench --bench chart`

use criterion::{criterion_group, BenchmarkId, Criterion, Throughput};
use densa_chart::{
    paint_line, paint_line_summarized, points_within, y_extent, Axis, Color, LineStyle,
    LineSummary, Plot, Rect,
};
use densa_draw::{Canvas, InstanceBuffer};
use densa_text::{AtlasConfig, GlyphCache};
use std::hint::black_box;

/// The plot's width in pixels: a frame of a wide chart.
const WIDTH: f32 = 1_000.0;

/// `count` points of a random walk, one per unit of `x`.
fn walk(count: usize) -> (Vec<f64>, Vec<f64>) {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut y = 0.0;
    let ys = (0..count)
        .map(|_| {
            // xorshift: enough noise for a jagged line.
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            y += (state % 2_001) as f64 / 1_000.0 - 1.0;
            y
        })
        .collect();
    let xs = (0..count).map(|i| i as f64).collect();
    (xs, ys)
}

fn dense_line(c: &mut Criterion) {
    let mut group = c.benchmark_group("dense_line");
    for count in [1_000_000_usize, 10_000_000] {
        let (xs, ys) = walk(count);
        let points = (&xs[..], &ys[..]);
        let x = Axis::number(0.0, count as f64);
        let (low, high) = y_extent(&points, 0..count).expect("finite");
        let plot = Plot::new(
            Rect::new(0.0, 0.0, WIDTH, 400.0),
            x,
            Axis::number(low, high),
        );
        let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
        let mut out = InstanceBuffer::with_capacity(8 * WIDTH as usize);
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::new("paint_line", count), &count, |b, _| {
            b.iter(|| {
                let mut canvas = Canvas::begin(&mut cache, &mut out);
                paint_line(&mut canvas, &plot, &points, LineStyle::new(Color::WHITE));
            })
        });
        group.bench_with_input(BenchmarkId::new("y_extent", count), &count, |b, _| {
            b.iter(|| black_box(y_extent(&points, points_within(&points, &x))))
        });
        let summary = LineSummary::of(&points);
        group.bench_with_input(BenchmarkId::new("summarized", count), &count, |b, _| {
            b.iter(|| {
                let mut canvas = Canvas::begin(&mut cache, &mut out);
                let style = LineStyle::new(Color::WHITE);
                paint_line_summarized(&mut canvas, &plot, &points, &summary, style);
            })
        });
        // Zoomed in to a tenth of the points, still far more than the plot is wide.
        let near = plot.with_x(Axis::number(count as f64 * 0.45, count as f64 * 0.55));
        group.bench_with_input(
            BenchmarkId::new("summarized_tenth", count),
            &count,
            |b, _| {
                b.iter(|| {
                    let mut canvas = Canvas::begin(&mut cache, &mut out);
                    let style = LineStyle::new(Color::WHITE);
                    paint_line_summarized(&mut canvas, &near, &points, &summary, style);
                })
            },
        );
        group.bench_with_input(
            BenchmarkId::new("summary_y_extent", count),
            &count,
            |b, _| b.iter(|| black_box(summary.y_extent(&points, points_within(&points, &x)))),
        );
        group.bench_with_input(BenchmarkId::new("summary_build", count), &count, |b, _| {
            b.iter(|| black_box(LineSummary::of(&points)))
        });
    }
    group.finish();
}

criterion_group!(benches, dense_line);

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
