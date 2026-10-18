//! Glyph cache: hits and misses, GPU uploads, subpixel variants, LRU eviction.

use densa_text::{quantize_x, CacheStats, Font, GlyphCache, SUBPIXEL_STEPS};

fn mono() -> Font {
    Font::bundled_mono().unwrap()
}

#[test]
fn first_request_rasterizes_and_queues_an_upload_then_hits() {
    let font = mono();
    let mut cache = GlyphCache::new(256);
    let a = font.glyph_id('A').unwrap();

    cache.begin_frame();
    let first = cache.get(&font, a, 16.0, 0).unwrap();
    let second = cache.get(&font, a, 16.0, 0).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        cache.stats(),
        CacheStats {
            hits: 1,
            misses: 1,
            evictions: 0,
            dropped: 0
        }
    );

    let uploads: Vec<_> = cache.drain_uploads().collect();
    assert_eq!(uploads.len(), 1);
    assert_eq!(
        uploads[0].coverage.len(),
        (uploads[0].width * uploads[0].height) as usize
    );
    assert_eq!(cache.drain_uploads().count(), 0, "drained");
}

#[test]
fn uv_rectangle_matches_the_uploaded_pixels() {
    let font = mono();
    let mut cache = GlyphCache::new(256);
    cache.begin_frame();
    let g = cache
        .get(&font, font.glyph_id('W').unwrap(), 20.0, 0)
        .unwrap();
    let up = cache.drain_uploads().next().unwrap();
    let size = cache.atlas_size() as f32;
    assert_eq!(
        g.uv,
        [
            up.x as f32 / size,
            up.y as f32 / size,
            (up.x + up.width) as f32 / size,
            (up.y + up.height) as f32 / size,
        ]
    );
    assert_eq!(g.size, [up.width as f32, up.height as f32]);
}

#[test]
fn whitespace_is_cached_without_atlas_space_or_uploads() {
    let font = mono();
    let mut cache = GlyphCache::new(64);
    cache.begin_frame();
    let space = cache
        .get(&font, font.glyph_id(' ').unwrap(), 16.0, 0)
        .unwrap();
    assert_eq!(space.size, [0.0, 0.0]);
    assert_eq!(cache.drain_uploads().count(), 0);
}

#[test]
fn subpixel_variants_are_separate_entries() {
    let font = mono();
    let mut cache = GlyphCache::new(256);
    cache.begin_frame();
    let g = font.glyph_id('|').unwrap();
    for bin in 0..SUBPIXEL_STEPS {
        cache.get(&font, g, 16.0, bin).unwrap();
    }
    assert_eq!(cache.stats().misses, u64::from(SUBPIXEL_STEPS));
}

#[test]
fn quantize_x_splits_a_position_into_pixel_and_subpixel_bin() {
    assert_eq!(quantize_x(10.0), (10, 0));
    assert_eq!(quantize_x(10.24), (10, 1));
    assert_eq!(quantize_x(10.5), (10, 2));
    assert_eq!(quantize_x(10.9), (11, 0), "rounds up into the next pixel");
    assert_eq!(quantize_x(-0.3), (-1, 3));
}

#[test]
fn full_atlas_evicts_glyphs_not_used_this_frame() {
    let font = mono();
    // 64x64 holds only a few 40 px glyphs.
    let mut cache = GlyphCache::new(64);
    let letters: Vec<_> = "ABCDEFGH"
        .chars()
        .map(|c| font.glyph_id(c).unwrap())
        .collect();
    let generation = cache.generation();

    for &g in &letters {
        cache.begin_frame(); // each glyph in its own frame: older ones are evictable
        assert!(cache.get(&font, g, 40.0, 0).is_some());
    }
    assert!(cache.stats().evictions > 0, "{:?}", cache.stats());
    assert!(
        cache.generation() > generation,
        "eviction invalidates earlier UVs"
    );

    // The first letter was evicted: asking again is a miss.
    let misses = cache.stats().misses;
    cache.begin_frame();
    cache.get(&font, letters[0], 40.0, 0).unwrap();
    assert_eq!(cache.stats().misses, misses + 1);
}

#[test]
fn glyphs_used_in_the_current_frame_are_never_evicted() {
    let font = mono();
    let mut cache = GlyphCache::new(64);
    cache.begin_frame();
    let mut placed = Vec::new();
    for ch in "ABCDEFGH".chars() {
        if let Some(g) = cache.get(&font, font.glyph_id(ch).unwrap(), 40.0, 0) {
            placed.push(g);
        }
    }
    let stats = cache.stats();
    assert_eq!(stats.evictions, 0, "nothing from an earlier frame to evict");
    assert!(
        stats.dropped > 0,
        "the overflow is dropped, not evicting live glyphs"
    );
    assert_eq!(placed.len() as u64 + stats.dropped, 8);
}
