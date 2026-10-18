//! Proves painting warm frames never touches the heap.

use densa_alloc::{is_active, measure, CountingAllocator};
use densa_fmt::write_fixed;
use densa_render::{
    Align, Canvas, CellStyle, CellText, Column, GridPainter, GridSource, InstanceBuffer, Palette,
};
use densa_text::{AtlasConfig, Font, GlyphCache};
use std::hint::black_box;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

/// Numbers that change every frame, like a live data grid.
struct Ticking {
    frame: u64,
}

impl GridSource for Ticking {
    fn rows(&self) -> usize {
        50
    }

    fn cell(&self, row: usize, column: usize, text: &mut CellText) -> CellStyle {
        let value = (self.frame as f64 * 0.37 + row as f64 * 11.0) % 10_000.0;
        let _ = match column {
            0 => text.push_str("Ngân hàng"),
            _ => write_fixed(text, value, 2),
        };
        CellStyle {
            color: [1.0; 4],
            background: row.is_multiple_of(3).then_some([0.0, 1.0, 0.0, 0.2]),
        }
    }
}

#[test]
fn painting_warm_frames_performs_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );

    let columns = vec![
        Column::new("NAME", 10, Align::Left),
        Column::new("VALUE", 10, Align::Right),
    ];
    let font = Font::bundled_mono().unwrap();
    let mut painter = GridPainter::new(columns, 50, font, 14.0, Palette::new());
    let mut buf = InstanceBuffer::with_capacity(painter.layout().max_instances());
    let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);

    // Startup: every glyph the frames use, at every subpixel position.
    assert_eq!(painter.prewarm(&mut cache, "NAMEVLU0123456789.Ngân hà"), 0);
    cache.drain_updates().for_each(drop);

    let ((), stats) = measure(|| {
        for frame in 0..1_000 {
            let mut canvas = Canvas::begin(&mut cache, &mut buf);
            painter.paint(&Ticking { frame }, &mut canvas);
            cache.drain_updates().for_each(drop);
            black_box(buf.as_slice());
        }
    });
    assert!(stats.is_zero(), "painting allocated: {stats:?}");
    assert_eq!(buf.dropped(), 0);
    assert_eq!(cache.stats().dropped, 0);
}
