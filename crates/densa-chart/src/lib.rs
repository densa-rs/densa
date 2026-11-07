//! Charts over your own data, drawn onto a `densa_draw::Canvas`: lines, candlesticks and
//! bars, with gridlines and tick labels.
//!
//! Domain-agnostic: an application implements [`Points`] (a line or bars) or [`Candles`]
//! over its data in ascending `x`, picks the visible part with an [`Axis`] for each
//! direction, and draws onto a [`Plot`] (an area of the canvas and its two axes). Time axes
//! count seconds since the epoch and label ticks as times of day or dates (UTC); category
//! axes place named things (a bar chart's bars) at `0, 1, 2, ...` and label them by name.
//!
//! A chart widget asks its data through [`ChartModel`]: panes on one time axis, each giving
//! its vertical axis for what shows and drawing its series.
//!
//! A line is drawn as one vertical span per pixel column, shaded at its ends, so a frame
//! holds a few rectangles per column however many points share one, and every high and low
//! still shows (M4 aggregation). Finding them visits each point that shows; for lines of
//! millions of points, a [`LineSummary`] built once lets a frame
//! ([`paint_line_summarized`]) and the vertical extent of what shows cost about the plot's
//! width instead, drawing the same pixels. Bars and candles denser than the pixels merge per
//! column; a [`BarSummary`] ([`paint_bars_summarized`]) or a [`CandleSummary`]
//! ([`paint_candles_summarized`]) does the same for millions of them. No GPU dependency: hand
//! the canvas's instances to `densa-render` (or any other backend) to show them.
//!
//! Painting allocates nothing once the label characters are cached.
//!
//! ```
//! use densa_chart::{paint_line, points_within, y_extent, Axis, LineStyle, Plot, Rect};
//! use densa_draw::{Canvas, InstanceBuffer};
//! use densa_text::{AtlasConfig, GlyphCache};
//!
//! // A day of readings, one a second: (seconds since the epoch, value).
//! let readings: Vec<(f64, f64)> = (0..86_400)
//!     .map(|s| (1_792_368_000.0 + f64::from(s), (f64::from(s) / 3_600.0).sin()))
//!     .collect();
//!
//! // The last hour, scaled to what it holds.
//! let x = Axis::time(1_792_450_800.0, 1_792_454_400.0);
//! let (low, high) = y_extent(&readings[..], points_within(&readings[..], &x)).unwrap();
//! let plot = Plot::new(Rect::new(0.0, 0.0, 800.0, 300.0), x, Axis::number(low, high).padded(0.05));
//!
//! let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
//! let mut out = InstanceBuffer::with_capacity(4_096);
//! let mut canvas = Canvas::begin(&mut cache, &mut out);
//! paint_line(&mut canvas, &plot, &readings[..], LineStyle::new(densa_chart::Color::WHITE));
//! assert!(out.len() <= 3 * 800, "at most a few rectangles per pixel column");
//! ```

mod axis;
mod bars;
mod line;
mod model;
mod painter;
mod palette;
mod plot;
mod series;
mod summary;

pub use axis::{Axis, AxisKind, Ticks};
pub use bars::{paint_bars, paint_candles, BarStyle, CandleStyle};
pub use densa_core::{Color, Point, Rect};
pub use line::{paint_line, LineStyle};
pub use model::ChartModel;
pub use painter::ChartPainter;
pub use palette::ChartPalette;
pub use plot::Plot;
pub use series::{candle_extent, candles_within, points_within, y_extent, Candle, Candles, Points};
pub use summary::{
    paint_bars_summarized, paint_candles_summarized, paint_line_summarized, BarSummary,
    CandleSummary, LineSummary,
};
