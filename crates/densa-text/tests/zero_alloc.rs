//! Proves a warm glyph cache serves frames without touching the heap.

use densa_alloc::{is_active, measure, CountingAllocator};
use densa_text::{quantize_x, AtlasConfig, Font, GlyphCache, GlyphMap};
use std::hint::black_box;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

#[test]
fn glyph_map_lookups_after_first_use_perform_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );
    let mut glyphs = GlyphMap::new(Font::bundled_mono().unwrap(), 14.0);
    let text = "SYM00042 1,234.50 €ẃß";
    black_box(glyphs.measure(text)); // first use of the non-ASCII characters caches them
    let ((), stats) = measure(|| {
        for _ in 0..10_000 {
            black_box(glyphs.measure(black_box(text)));
        }
    });
    assert!(stats.is_zero(), "glyph map allocated: {stats:?}");
}

#[test]
fn warm_cache_lookups_perform_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );

    let font = Font::bundled_mono().unwrap();
    let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
    let text = "0123456789.,- SYM €ẃß";
    let glyphs: Vec<_> = text.chars().filter_map(|c| font.glyph_id(c)).collect();
    let advance = font.advance(glyphs[0], 14.0);

    // Warm-up frame: every glyph at every subpixel bin it will be drawn at.
    let draw_frame = |cache: &mut GlyphCache| {
        cache.begin_frame();
        let mut x = 0.0;
        for &g in &glyphs {
            let (_pixel, bin) = quantize_x(x);
            black_box(
                cache
                    .get(&font, g, 14.0, bin)
                    .expect("fits the default atlas"),
            );
            x += advance;
        }
        cache.drain_updates().for_each(|u| {
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

#[test]
fn evicting_and_retrying_dropped_glyphs_perform_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );
    let font = Font::bundled_mono().unwrap();
    // One 64 x 64 page: a frame of 40 px capitals overflows it.
    let mut cache = GlyphCache::new(AtlasConfig::new(64, 1));
    let glyphs: Vec<_> = "ABCDEFGHIJKLMNOP"
        .chars()
        .filter_map(|c| font.glyph_id(c))
        .collect();
    cache.begin_frame();
    let waiting: Vec<_> = glyphs
        .iter()
        .copied()
        .filter(|&g| cache.get(&font, g, 40.0, 0).is_err())
        .collect();
    assert!(waiting.len() >= 2, "{} waiting", waiting.len());

    // One eviction episode sizes the eviction order buffer; its largest size is now known.
    cache.begin_frame();
    cache.get(&font, waiting[0], 40.0, 0).unwrap();
    cache.drain_updates().for_each(drop);
    let evictions = cache.stats().evictions;

    // Updates are drained after measuring: dropping their pixels deallocates by design.
    let ((), stats) = measure(|| {
        cache.begin_frame();
        for &g in &waiting[1..] {
            black_box(cache.get(&font, g, 40.0, 0).ok());
        }
    });
    assert!(stats.is_zero(), "eviction or retry allocated: {stats:?}");
    assert!(cache.stats().evictions > evictions, "the frame evicted");
    cache.drain_updates().for_each(drop);
}
