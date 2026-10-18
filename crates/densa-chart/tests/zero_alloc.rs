//! Proves drawing warm chart frames never touches the heap.

use std::hint::black_box;

use densa_alloc::{is_active, measure, CountingAllocator};
use densa_chart::{
    paint_bars, paint_candles, paint_line, Axis, BarStyle, Candle, CandleStyle, ChartPainter,
    ChartPalette, LineStyle, Plot, Rect,
};
use densa_draw::{Canvas, InstanceBuffer};
use densa_text::{AtlasConfig, Font, GlyphCache};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

#[test]
fn frames_of_a_scrolling_chart_perform_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );

    // A day of seconds as a line, and of minutes as candles with volume.
    let start = 1_792_400_400.0;
    let line: Vec<(f64, f64)> = (0..86_400)
        .map(|s| (start + f64::from(s), 100.0 + (f64::from(s) * 0.001).sin()))
        .collect();
    let candles: Vec<(f64, Candle)> = (0..1_440)
        .map(|m| {
            let open = 100.0 + (f64::from(m) * 0.1).sin();
            let close = 100.0 + (f64::from(m + 1) * 0.1).sin();
            let candle = Candle::new(open, open.max(close) + 0.2, open.min(close) - 0.2, close);
            (start + 60.0 * f64::from(m), candle)
        })
        .collect();
    let volume: Vec<(f64, f64)> = candles
        .iter()
        .map(|(x, candle)| (*x, (candle.close - candle.open).abs() * 1e6))
        .collect();

    let palette = ChartPalette::new();
    let mut painter = ChartPainter::new(Font::bundled_mono().unwrap(), 13.0, palette);
    let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
    let mut out = InstanceBuffer::with_capacity(50_000);
    assert_eq!(painter.prewarm(&mut cache), 0);
    cache.drain_updates().for_each(drop);

    let top = Rect::new(0.0, 0.0, 1_200.0, 450.0);
    let bottom = Rect::new(0.0, 450.0, 1_200.0, 150.0);
    let style = CandleStyle::new(palette.up, palette.down);
    let ((), stats) = measure(|| {
        for frame in 0..300 {
            // Scrolls by a minute a frame, three hours on screen.
            let left = start + 60.0 * f64::from(frame);
            let x = Axis::time(left, left + 3.0 * 3_600.0);
            let prices = Plot::new(painter.plot_area(top, false), x, Axis::number(98.5, 101.5));
            let volumes = Plot::new(painter.plot_area(bottom, true), x, Axis::number(0.0, 3e5));
            let mut canvas = Canvas::begin(&mut cache, &mut out);
            painter.paint_axes(&mut canvas, &prices, false);
            painter.paint_axes(&mut canvas, &volumes, true);
            paint_candles(&mut canvas, &prices, &candles[..], 60.0, style);
            paint_line(
                &mut canvas,
                &prices,
                &line[..],
                LineStyle::new(palette.line),
            );
            paint_bars(
                &mut canvas,
                &volumes,
                &volume[..],
                60.0,
                BarStyle::new(),
                |_| palette.up,
            );
            cache.drain_updates().for_each(drop);
            black_box(out.as_slice());
        }
    });
    assert!(stats.is_zero(), "drawing allocated: {stats:?}");
    assert_eq!(out.dropped(), 0);
    assert_eq!(cache.stats().dropped, 0);
}
