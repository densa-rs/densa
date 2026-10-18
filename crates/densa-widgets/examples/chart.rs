//! A live chart: one simulated instrument as one-minute candles with a moving average,
//! volume below, a new trade every frame and a new candle every simulated minute. Scroll to
//! zoom around the pointer, drag to pan, double-click (or press End) to follow the newest
//! candle again.
//!
//! `cargo run --release -p densa-widgets --example chart` (`--light` for the light theme,
//! `--seconds N` to close after N seconds).

use std::error::Error;
use std::time::{Duration, Instant};

use densa_chart::{
    candle_extent, candles_within, paint_bars, paint_candles, paint_line, points_within, y_extent,
    Axis, BarStyle, Candle, CandleStyle, ChartModel, ChartPalette, LineStyle, Plot,
};
use densa_draw::Canvas;
use densa_ui::{Action, Align, Style, Theme, Ui, WidgetId};
use densa_widgets::{Chart, Label, Panel};
use densa_window::{App, Host, WindowOptions};

/// One candle's period, in seconds.
const MINUTE: f64 = 60.0;
/// Candles made before the window opens: three days.
const HISTORY: usize = 3 * 24 * 60;
/// Simulated seconds that pass each frame.
const SECONDS_PER_FRAME: f64 = 1.0;
/// Closes the moving average is taken over.
const AVERAGE: usize = 20;
/// 2026-10-16 00:00:00 UTC, where the history starts.
const START: f64 = 1_792_108_800.0;
/// What shows at first: the last three hours.
const SHOWN: f64 = 3.0 * 3_600.0;
/// How often the status bar shows the last price again.
const STATUS_EVERY: Duration = Duration::from_millis(250);

/// Random numbers: wyrand, deterministic.
struct Feed {
    seed: u64,
}

impl Feed {
    fn next(&mut self) -> u64 {
        self.seed = self.seed.wrapping_add(0xA076_1D64_78BD_642F);
        let t = u128::from(self.seed) * u128::from(self.seed ^ 0xE703_7ED1_A0B4_28DB);
        // Lossless: the halves of a 128-bit product.
        (t >> 64) as u64 ^ t as u64
    }

    /// A number in `-0.5..0.5`.
    fn centred(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1_u64 << 53) as f64 - 0.5
    }
}

/// A simulated instrument trading once a second: candles, their volume and the average of
/// their closes, each in ascending time.
struct Market {
    feed: Feed,
    now: f64,
    price: f64,
    candles: Vec<(f64, Candle)>,
    volume: Vec<(f64, f64)>,
    average: Vec<(f64, f64)>,
}

impl Market {
    fn new() -> Self {
        let mut market = Self {
            feed: Feed { seed: 0xC0FFEE },
            now: START,
            price: 182.0,
            candles: Vec::with_capacity(HISTORY * 2),
            volume: Vec::with_capacity(HISTORY * 2),
            average: Vec::with_capacity(HISTORY * 2),
        };
        // Lossless: three days of minutes.
        let seconds = (HISTORY as f64 * MINUTE) as usize;
        for _ in 0..seconds {
            market.trade();
        }
        market
    }

    /// One second's trade: the price moves, the current candle takes it (a new candle every
    /// minute) and the average follows.
    fn trade(&mut self) {
        // A random walk with a slow swing, so there are trends to see.
        let swing = (self.now / 7_200.0).sin() * 0.0006;
        self.price = (self.price + self.feed.centred() * 0.12 + swing).max(1.0);
        // Mostly small trades, now and then a large one.
        let size = (self.feed.centred() + 0.5).powi(6) * 6_000.0;
        let period = (self.now / MINUTE).floor() * MINUTE;
        match self.candles.last_mut() {
            Some((at, candle)) if *at == period => {
                candle.high = candle.high.max(self.price);
                candle.low = candle.low.min(self.price);
                candle.close = self.price;
                if let Some((_, volume)) = self.volume.last_mut() {
                    *volume += size;
                }
            }
            _ => {
                let open = self.candles.last().map_or(self.price, |(_, c)| c.close);
                let high = open.max(self.price);
                let low = open.min(self.price);
                self.candles
                    .push((period, Candle::new(open, high, low, self.price)));
                self.volume.push((period, size));
                self.average.push((period, self.price));
            }
        }
        let count = self.candles.len().min(AVERAGE);
        let closes = self.candles[self.candles.len() - count..]
            .iter()
            .map(|(_, candle)| candle.close);
        // Lossless: at most twenty closes.
        let mean = closes.sum::<f64>() / count as f64;
        if let Some((_, average)) = self.average.last_mut() {
            *average = mean;
        }
        self.now += SECONDS_PER_FRAME;
    }
}

impl ChartModel for Market {
    fn x_extent(&self) -> Option<(f64, f64)> {
        Some((self.candles.first()?.0, self.candles.last()?.0))
    }

    fn x_step(&self) -> Option<f64> {
        Some(MINUTE)
    }

    fn panes(&self) -> usize {
        2
    }

    fn pane_weight(&self, pane: usize) -> f32 {
        if pane == 0 {
            3.0
        } else {
            1.0
        }
    }

    fn y_axis(&self, pane: usize, x: &Axis) -> Option<Axis> {
        if pane == 0 {
            let candles = candle_extent(&self.candles[..], candles_within(&self.candles[..], x));
            let average = y_extent(&self.average[..], points_within(&self.average[..], x));
            let (low, high) = match (candles, average) {
                (Some((a, b)), Some((c, d))) => (a.min(c), b.max(d)),
                (one, other) => one.or(other)?,
            };
            return Some(Axis::number(low, high).padded(0.08));
        }
        let (_, most) = y_extent(&self.volume[..], points_within(&self.volume[..], x))?;
        Some(Axis::number(0.0, most * 1.15))
    }

    fn paint(&self, pane: usize, plot: &Plot, canvas: &mut Canvas<'_>, palette: &ChartPalette) {
        if pane == 0 {
            let style = CandleStyle::new(palette.up, palette.down);
            paint_candles(canvas, plot, &self.candles[..], MINUTE, style);
            let line = LineStyle::new(palette.line).with_width(1.5);
            paint_line(canvas, plot, &self.average[..], line);
            return;
        }
        let candles = &self.candles;
        paint_bars(
            canvas,
            plot,
            &self.volume[..],
            MINUTE,
            BarStyle::new(),
            |index| {
                let (_, candle) = candles[index];
                let color = if candle.close >= candle.open {
                    palette.up
                } else {
                    palette.down
                };
                color.with_alpha(0.5)
            },
        );
    }

    fn describe(&self) -> String {
        let last = self.candles.last().map_or(0.0, |(_, candle)| candle.close);
        format!("Simulated instrument, one-minute candles, last {last:.2}")
    }
}

type MarketChart = Chart<Market>;

struct ChartApp {
    light: bool,
    close_at: Option<Instant>,
    ids: Option<Ids>,
    status_at: Instant,
}

struct Ids {
    chart: WidgetId,
    status: WidgetId,
}

impl ChartApp {
    /// Shows the last price and whether the chart follows it.
    fn show_status(ui: &mut Ui, ids: &Ids) {
        let Some(chart) = ui.widget::<MarketChart>(ids.chart) else {
            return;
        };
        let last = chart
            .model()
            .candles
            .last()
            .map_or(0.0, |(_, candle)| candle.close);
        let mode = if chart.is_following() {
            "following the newest candle"
        } else {
            "paused: double-click or press End to follow"
        };
        let text = format!("Last {last:.2}, {mode}");
        ui.update::<Label, _>(ids.status, |label, cx| label.set_text(cx, text));
    }
}

impl App for ChartApp {
    fn build(&mut self, ui: &mut Ui, _: &mut Host<'_>) -> Result<(), Box<dyn Error>> {
        if self.light {
            ui.set_theme(Theme::light());
        }
        let root = ui.root();
        let bar = Style::row()
            .padding(8.0)
            .gap(12.0)
            .shrink(0.0)
            .align_items(Align::Center);
        let toolbar = ui.add(root, Panel::new(), bar)?;
        ui.add(toolbar, Label::new("Chart").with_size(15.0), Style::row())?;
        let hint = "Scroll to zoom, drag to pan, double-click to follow";
        ui.add(toolbar, Label::new(hint).dim(), Style::row())?;

        let chart = MarketChart::new(Market::new(), SHOWN);
        let chart = ui.add(root, chart, MarketChart::style())?;

        let bar = Style::row().padding(6.0).shrink(0.0);
        let bar = ui.add(root, Panel::new(), bar)?;
        let status = ui.add(bar, Label::new("").dim(), Style::row())?;
        let ids = Ids { chart, status };
        Self::show_status(ui, &ids);
        self.ids = Some(ids);
        Ok(())
    }

    fn action(&mut self, ui: &mut Ui, _: &mut Host<'_>, _: Action) {
        if let Some(ids) = &self.ids {
            Self::show_status(ui, ids);
        }
    }

    fn frame(&mut self, ui: &mut Ui, host: &mut Host<'_>) -> bool {
        let Some(ids) = &self.ids else {
            return false;
        };
        ui.update::<MarketChart, _>(ids.chart, |chart, cx| chart.model_mut(cx).trade());
        if self.status_at.elapsed() >= STATUS_EVERY {
            self.status_at = Instant::now();
            Self::show_status(ui, ids);
        }
        if self.close_at.is_some_and(|at| Instant::now() >= at) {
            host.close();
            return false;
        }
        true
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let light = args.iter().any(|a| a == "--light");
    let seconds = args
        .iter()
        .skip_while(|a| *a != "--seconds")
        .nth(1)
        .map(|s| s.parse::<u64>())
        .transpose()?;
    let app = ChartApp {
        light,
        close_at: seconds.map(|s| Instant::now() + Duration::from_secs(s)),
        ids: None,
        status_at: Instant::now(),
    };
    densa_window::run(
        WindowOptions::new("densa chart").with_size(1100.0, 700.0),
        app,
    )?;
    Ok(())
}
