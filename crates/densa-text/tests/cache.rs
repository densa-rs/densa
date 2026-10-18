//! Glyph cache: hits and misses, atlas updates, padding, growth, eviction, retries, limits.

use densa_text::{
    quantize_x, rasterize, AtlasConfig, AtlasUpdate, CachedGlyph, Font, GlyphCache, GlyphId,
    GlyphUnavailable, GlyphUpload, SUBPIXEL_STEPS,
};

fn mono() -> Font {
    Font::bundled_mono().unwrap()
}

fn cache(page_size: u32, max_pages: u32) -> GlyphCache {
    GlyphCache::new(AtlasConfig::new(page_size, max_pages))
}

fn glyph(font: &Font, ch: char) -> GlyphId {
    font.glyph_id(ch).unwrap()
}

fn writes(cache: &mut GlyphCache) -> Vec<GlyphUpload> {
    cache
        .drain_updates()
        .filter_map(|u| match u {
            AtlasUpdate::Write(w) => Some(w),
            _ => None,
        })
        .collect()
}

fn resizes(cache: &mut GlyphCache) -> Vec<(u32, u32)> {
    cache
        .drain_updates()
        .filter_map(|u| match u {
            AtlasUpdate::Resize { width, height } => Some((width, height)),
            _ => None,
        })
        .collect()
}

/// 64 x 64 pages hold only a few 40 px capitals each.
const BIG: f32 = 40.0;

mod basics {
    use super::*;

    #[test]
    fn the_first_update_announces_the_atlas_size() {
        let mut c = cache(256, 4);
        assert_eq!(resizes(&mut c), [(256, 256)]);
        assert_eq!(c.atlas_extent(), (256, 256));
    }

    #[test]
    fn first_request_rasterizes_and_queues_a_write_then_hits() {
        let font = mono();
        let mut c = cache(256, 1);
        let a = glyph(&font, 'A');
        c.begin_frame();
        let first = c.get(&font, a, 16.0, 0).unwrap();
        assert_eq!(c.get(&font, a, 16.0, 0).unwrap(), first);
        let stats = c.stats();
        assert_eq!((stats.hits, stats.misses, stats.dropped), (1, 1, 0));
        assert_eq!(writes(&mut c).len(), 1);
        assert_eq!(c.drain_updates().count(), 0, "drained");
    }

    #[test]
    fn atlas_rect_is_the_upload_minus_a_transparent_border() {
        let font = mono();
        let mut c = cache(256, 1);
        c.begin_frame();
        let g = c.get(&font, glyph(&font, 'W'), 20.0, 0).unwrap();
        let up = writes(&mut c).remove(0);

        let [w, h] = [g.size[0] as u32, g.size[1] as u32];
        assert_eq!((up.width, up.height), (w + 2, h + 2));
        assert_eq!(up.coverage.len(), (up.width * up.height) as usize);
        let (x, y) = (up.x as f32 + 1.0, up.y as f32 + 1.0);
        assert_eq!(g.atlas_rect, [x, y, x + g.size[0], y + g.size[1]]);

        let row = |r: u32| &up.coverage[(r * up.width) as usize..((r + 1) * up.width) as usize];
        assert!(row(0).iter().all(|&c| c == 0), "top border");
        assert!(row(up.height - 1).iter().all(|&c| c == 0), "bottom border");
        assert!((0..up.height).all(|r| row(r)[0] == 0 && row(r)[up.width as usize - 1] == 0));
        assert!(
            up.coverage.iter().any(|&c| c > 0),
            "the glyph itself has ink"
        );
    }

    #[test]
    fn whitespace_is_cached_without_atlas_space_or_uploads() {
        let font = mono();
        let mut c = cache(64, 1);
        c.drain_updates().for_each(drop);
        c.begin_frame();
        let space = c.get(&font, glyph(&font, ' '), 16.0, 0).unwrap();
        assert_eq!(space.size, [0.0, 0.0]);
        assert_eq!(c.drain_updates().count(), 0);
    }

    #[test]
    fn subpixel_variants_are_separate_entries() {
        let font = mono();
        let mut c = cache(256, 1);
        c.begin_frame();
        for bin in 0..SUBPIXEL_STEPS {
            c.get(&font, glyph(&font, '|'), 16.0, bin).unwrap();
        }
        assert_eq!(c.stats().misses, u64::from(SUBPIXEL_STEPS));
    }

    #[test]
    fn nearly_equal_sizes_share_one_rasterization() {
        let font = mono();
        let mut c = cache(256, 1);
        c.begin_frame();
        let a = c.get(&font, glyph(&font, 'A'), 16.0, 0).unwrap();
        let b = c.get(&font, glyph(&font, 'A'), 16.001, 0).unwrap();
        assert_eq!(a, b);
        assert_eq!(c.stats().misses, 1);
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
    fn config_is_normalized_to_supported_sizes() {
        let c = AtlasConfig::new(0, 0).normalized();
        assert_eq!((c.page_size, c.max_pages), (1, 1));
        let c = AtlasConfig::new(100_000, 100).normalized();
        assert_eq!((c.page_size, c.max_pages), (4096, 2));
    }
}

mod sizes {
    use super::*;

    #[test]
    fn invalid_sizes_are_rejected_without_caching_anything() {
        let font = mono();
        let mut c = cache(256, 1);
        c.begin_frame();
        for px in [0.0, -3.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                c.get(&font, glyph(&font, 'A'), px, 0),
                Err(GlyphUnavailable::InvalidSize),
                "{px}"
            );
        }
        assert_eq!(c.cached_glyphs(), 0);
        assert_eq!(c.stats().misses, 0);
    }

    #[test]
    fn huge_sizes_are_too_large_without_rasterizing() {
        let font = mono();
        let mut c = cache(1024, 4);
        c.begin_frame();
        let start = std::time::Instant::now();
        for px in [30_000.0, 1e9, f32::MAX] {
            assert_eq!(
                c.get(&font, glyph(&font, 'W'), px, 0),
                Err(GlyphUnavailable::TooLarge),
                "{px}"
            );
        }
        // A rasterized 30,000 px glyph would take seconds and gigabytes.
        assert!(start.elapsed().as_millis() < 500, "{:?}", start.elapsed());
        assert_eq!(c.drain_updates().count(), 1, "only the initial resize");
    }

    #[test]
    fn glyphs_larger_than_a_page_are_dropped_every_time() {
        let font = mono();
        let mut c = cache(16, 4);
        c.begin_frame();
        let w = glyph(&font, 'W');
        assert_eq!(c.get(&font, w, BIG, 0), Err(GlyphUnavailable::TooLarge));
        c.begin_frame();
        assert_eq!(c.get(&font, w, BIG, 0), Err(GlyphUnavailable::TooLarge));
        assert_eq!(c.stats().misses, 1, "looked into once");
        assert_eq!(c.stats().dropped, 2);
        assert_eq!(c.atlas_extent(), (16, 16), "no pointless growth");
    }

    #[test]
    fn a_page_sized_glyph_is_placed_evicted_and_placed_again() {
        let font = mono();
        // Size the page to exactly the padded 'W'.
        let mut probe = cache(256, 1);
        probe.begin_frame();
        probe.get(&font, glyph(&font, 'W'), BIG, 0).unwrap();
        let up = writes(&mut probe).remove(0);
        let page = up.width.max(up.height);

        let mut c = cache(page, 1);
        for ch in ['W', 'M', 'W'] {
            c.begin_frame();
            assert!(c.get(&font, glyph(&font, ch), BIG, 0).is_ok(), "{ch}");
        }
        assert_eq!(c.stats().evictions, 2);
        assert_eq!(c.stats().misses, 3);
    }
}

mod pressure {
    use super::*;

    #[test]
    fn a_full_atlas_grows_a_page_without_moving_placed_glyphs() {
        let font = mono();
        let mut c = cache(64, 3);
        c.drain_updates().for_each(drop);
        c.begin_frame();
        let a = c.get(&font, glyph(&font, 'A'), BIG, 0).unwrap();
        for ch in "BCDEF".chars() {
            let _ = c.get(&font, glyph(&font, ch), BIG, 0);
        }
        assert!(c.atlas_extent().1 > 64, "grew: {:?}", c.atlas_extent());
        assert_eq!(resizes(&mut c).first(), Some(&(64, 128)));
        assert_eq!(c.get(&font, glyph(&font, 'A'), BIG, 0).unwrap(), a);
        assert_eq!(c.stats().evictions, 0);
    }

    #[test]
    fn at_its_limit_the_least_recently_used_glyphs_are_evicted() {
        let font = mono();
        let mut c = cache(64, 2);
        let generation = c.generation();
        for ch in "ABCDEFGHIJKL".chars() {
            c.begin_frame(); // one glyph per frame: earlier ones become evictable
            assert!(c.get(&font, glyph(&font, ch), BIG, 0).is_ok(), "{ch}");
        }
        assert_eq!(c.atlas_extent(), (64, 128), "never beyond max_pages");
        assert!(c.stats().evictions > 0, "{:?}", c.stats());
        assert!(
            c.generation() > generation,
            "eviction invalidates old rects"
        );

        let misses = c.stats().misses;
        c.begin_frame();
        c.get(&font, glyph(&font, 'L'), BIG, 0).unwrap();
        assert_eq!(c.stats().misses, misses, "the newest letter was kept");
        c.get(&font, glyph(&font, 'A'), BIG, 0).unwrap();
        assert_eq!(
            c.stats().misses,
            misses + 1,
            "the oldest letter was evicted"
        );
    }

    #[test]
    fn glyphs_used_this_frame_are_never_evicted() {
        let font = mono();
        let mut c = cache(64, 1);
        c.begin_frame();
        let placed = "ABCDEFGH"
            .chars()
            .filter(|&ch| c.get(&font, glyph(&font, ch), BIG, 0).is_ok())
            .count();
        let stats = c.stats();
        assert_eq!(stats.evictions, 0);
        assert!(
            stats.dropped > 0,
            "the overflow is dropped, not evicting live glyphs"
        );
        assert_eq!(placed as u64 + stats.dropped, 8);
    }

    #[test]
    fn dropped_glyphs_are_placed_later_without_rasterizing_again() {
        let font = mono();
        let mut c = cache(64, 1);
        c.begin_frame();
        let dropped: Vec<char> = "ABCDEFGH"
            .chars()
            .filter(|&ch| c.get(&font, glyph(&font, ch), BIG, 0) == Err(GlyphUnavailable::NoRoom))
            .collect();
        assert!(!dropped.is_empty());
        let misses = c.stats().misses;

        c.begin_frame();
        assert!(c.get(&font, glyph(&font, dropped[0]), BIG, 0).is_ok());
        assert_eq!(c.stats().misses, misses, "placed from the kept bitmap");
    }

    /// One recently used ("hot") glyph on every page must not stop a new glyph from being
    /// placed, nor be evicted for it, whichever is looked up first.
    #[test]
    fn hot_glyphs_on_every_page_neither_block_nor_lose_to_new_glyphs() {
        let font = mono();
        for new_first in [false, true] {
            let mut c = cache(64, 2);
            c.begin_frame();
            let mut placed: Vec<(char, CachedGlyph)> = Vec::new();
            for ch in "ABCDEFGHIJKLMNOP".chars() {
                if let Ok(g) = c.get(&font, glyph(&font, ch), BIG, 0) {
                    placed.push((ch, g));
                }
            }
            let page_of = |g: &CachedGlyph| (g.atlas_rect[1] / 64.0) as u32;
            let hot: Vec<char> = (0..2)
                .filter_map(|p| {
                    placed
                        .iter()
                        .find(|(_, g)| page_of(g) == p)
                        .map(|(ch, _)| *ch)
                })
                .collect();
            assert_eq!(hot.len(), 2, "both pages hold glyphs");
            c.begin_frame();
            for ch in &hot {
                c.get(&font, glyph(&font, *ch), BIG, 0).unwrap(); // hot: used last frame
            }

            c.begin_frame();
            let new = glyph(&font, 'Z');
            let misses = c.stats().misses;
            if new_first {
                assert!(c.get(&font, new, BIG, 0).is_ok(), "new glyph first");
            }
            for ch in &hot {
                c.get(&font, glyph(&font, *ch), BIG, 0).unwrap();
            }
            if !new_first {
                assert!(c.get(&font, new, BIG, 0).is_ok(), "new glyph last");
            }
            assert_eq!(
                c.stats().misses,
                misses + 1,
                "hot glyphs stayed (new first: {new_first})"
            );
        }
    }

    #[test]
    fn whitespace_at_ever_changing_sizes_stays_bounded() {
        let font = mono();
        let mut c = cache(256, 1);
        let space = glyph(&font, ' ');
        for i in 0..20_000u32 {
            c.begin_frame();
            c.get(&font, space, 8.0 + i as f32 / 64.0, 0).unwrap();
        }
        assert!(c.cached_glyphs() <= 8_192, "{} entries", c.cached_glyphs());
    }

    #[test]
    fn limit_side_caps_growth_to_the_device() {
        let font = mono();
        let mut c = cache(64, 8);
        c.limit_side(128).unwrap();
        assert_eq!(c.config().max_pages, 2);
        c.begin_frame();
        for ch in "ABCDEFGHIJKLMNOPQRSTUVWXYZ".chars() {
            let _ = c.get(&font, glyph(&font, ch), BIG, 0);
        }
        assert_eq!(c.atlas_extent(), (64, 128));

        let err = c.limit_side(100).unwrap_err();
        assert_eq!((err.max_side, err.needed), (100, 128));
        assert!(
            cache(256, 1).limit_side(128).is_err(),
            "a page larger than the device"
        );
    }
}

/// Replays the update stream into a CPU copy of the texture, as a GPU would, and checks
/// every glyph handed out in a frame shows exactly its own pixels, with a clear border.
mod replay {
    use super::*;

    struct Shadow {
        width: u32,
        height: u32,
        texels: Vec<u8>,
    }

    impl Shadow {
        fn apply(&mut self, update: AtlasUpdate) {
            match update {
                AtlasUpdate::Resize { width, height } => {
                    let (width, height) = (width.max(self.width), height.max(self.height));
                    let mut grown = vec![0u8; (width * height) as usize];
                    for row in 0..self.height {
                        let src = (row * self.width) as usize;
                        let dst = (row * width) as usize;
                        grown[dst..dst + self.width as usize]
                            .copy_from_slice(&self.texels[src..src + self.width as usize]);
                    }
                    *self = Self {
                        width,
                        height,
                        texels: grown,
                    };
                }
                AtlasUpdate::Write(w) => {
                    assert!(w.x + w.width <= self.width && w.y + w.height <= self.height);
                    for row in 0..w.height {
                        let dst = ((w.y + row) * self.width + w.x) as usize;
                        let src = (row * w.width) as usize;
                        self.texels[dst..dst + w.width as usize]
                            .copy_from_slice(&w.coverage[src..src + w.width as usize]);
                    }
                }
                _ => unreachable!("unknown update"),
            }
        }

        fn at(&self, x: u32, y: u32) -> u8 {
            self.texels[(y * self.width + x) as usize]
        }
    }

    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n
        }
    }

    #[test]
    fn every_glyph_in_a_frame_shows_its_own_pixels_under_churn() {
        let font = mono();
        let mut c = cache(96, 2);
        let mut shadow = Shadow {
            width: 0,
            height: 0,
            texels: Vec::new(),
        };
        let chars: Vec<char> = ('A'..='Z').chain('a'..='z').chain('0'..='9').collect();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut checked = 0;
        for _ in 0..300 {
            c.begin_frame();
            let mut frame: Vec<(GlyphId, f32, u8, CachedGlyph)> = Vec::new();
            for _ in 0..30 {
                let id = glyph(&font, chars[rng.below(chars.len() as u64) as usize]);
                let px = (10 + rng.below(36)) as f32;
                let bin = rng.below(u64::from(SUBPIXEL_STEPS)) as u8;
                if let Ok(g) = c.get(&font, id, px, bin) {
                    frame.push((id, px, bin, g));
                }
            }
            c.drain_updates().for_each(|u| shadow.apply(u));

            for &(id, px, bin, g) in &frame {
                if g.size[0] == 0.0 {
                    continue;
                }
                let expected = rasterize(&font, id, px, f32::from(bin) / 4.0);
                let [x0, y0, x1, y1] = g.atlas_rect.map(|v| v as u32);
                assert_eq!((x1 - x0, y1 - y0), (expected.width, expected.height));
                for y in y0..y1 {
                    for x in x0..x1 {
                        let i = ((y - y0) * expected.width + x - x0) as usize;
                        let want = expected.coverage[i];
                        assert_eq!(shadow.at(x, y), want, "{id:?} {px}px texel ({x},{y})");
                    }
                }
                for x in x0 - 1..=x1 {
                    assert_eq!(shadow.at(x, y0 - 1), 0, "top border");
                    assert_eq!(shadow.at(x, y1), 0, "bottom border");
                }
                for y in y0..y1 {
                    assert_eq!(shadow.at(x0 - 1, y), 0, "left border");
                    assert_eq!(shadow.at(x1, y), 0, "right border");
                }
                checked += 1;
            }
        }
        let stats = c.stats();
        assert!(stats.evictions > 100, "must exercise eviction: {stats:?}");
        assert!(stats.dropped > 0, "must exercise waiting glyphs: {stats:?}");
        assert!(checked > 4_000, "checked {checked}");
    }
}
