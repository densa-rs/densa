//! Text costs: warm glyph-cache lookups for a grid frame, and cold rasterization.
//!
//! Run: `cargo bench -p densa-bench --bench text`

use criterion::{criterion_group, Criterion, Throughput};
use densa_text::{quantize_x, rasterize, Font, GlyphCache};
use std::hint::black_box;

/// About one 50-row grid frame: 50 rows x 50 characters.
const LOOKUPS_PER_FRAME: usize = 2_500;
const PX: f32 = 14.0;

fn warm_lookups(c: &mut Criterion) {
    let font = Font::bundled_mono().expect("bundled font");
    let glyphs: Vec<_> = "0123456789.,-SYMBOLASKVUE"
        .chars()
        .filter_map(|ch| font.glyph_id(ch))
        .collect();
    let advance = font.advance(glyphs[0], PX);
    let mut cache = GlyphCache::new(1024);
    let frame = |cache: &mut GlyphCache| {
        cache.begin_frame();
        let mut x = 0.0;
        for i in 0..LOOKUPS_PER_FRAME {
            let (_, bin) = quantize_x(x);
            black_box(cache.get(&font, glyphs[i % glyphs.len()], PX, bin));
            x += advance;
        }
    };
    frame(&mut cache); // warm every glyph at every bin it will hit
    for _ in 0..4 {
        frame(&mut cache);
    }

    let mut group = c.benchmark_group("glyph_cache");
    group.throughput(Throughput::Elements(LOOKUPS_PER_FRAME as u64));
    group.bench_function("warm_frame_2500_lookups", |b| b.iter(|| frame(&mut cache)));
    group.finish();
}

fn cold_raster(c: &mut Criterion) {
    let font = Font::bundled_mono().expect("bundled font");
    let glyph = font.glyph_id('8').expect("digit");
    c.bench_function("rasterize_digit_14px", |b| {
        b.iter(|| black_box(rasterize(&font, glyph, PX, 0.25)))
    });
}

criterion_group!(benches, warm_lookups, cold_raster);

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
