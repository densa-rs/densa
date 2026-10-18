//! Proves a warm glyph cache serves frames without touching the heap.

use densa_alloc::{is_active, measure, CountingAllocator};
use densa_text::{quantize_x, Font, GlyphCache};
use std::hint::black_box;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

#[test]
fn warm_cache_lookups_perform_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );

    let font = Font::bundled_mono().unwrap();
    let mut cache = GlyphCache::new(1024);
    let text = "0123456789.,- SYM ệĐữ";
    let glyphs: Vec<_> = text.chars().filter_map(|c| font.glyph_id(c)).collect();
    let advance = font.advance(glyphs[0], 14.0);

    // Warm-up frame: every glyph at every subpixel bin it will be drawn at.
    let draw_frame = |cache: &mut GlyphCache| {
        cache.begin_frame();
        let mut x = 0.0;
        for &g in &glyphs {
            let (_pixel, bin) = quantize_x(x);
            black_box(cache.get(&font, g, 14.0, bin));
            x += advance;
        }
        cache.drain_uploads().for_each(|u| {
            black_box(u);
        });
    };
    draw_frame(&mut cache);

    let ((), stats) = measure(|| {
        for _ in 0..10_000 {
            draw_frame(&mut cache);
        }
    });
    assert!(stats.is_zero(), "warm cache allocated: {stats:?}");
    assert_eq!(cache.stats().dropped, 0);
}
