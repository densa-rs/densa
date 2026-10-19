//! A grid and a chart over the same live data: 200 simulated instruments in a live table,
//! and every trade in another. Click an instrument in the grid and the chart shows its
//! trades, a view of the trades table filtered by symbol and sorted by time, which keeps up
//! as trades arrive (two hours are kept; older ones are deleted as new ones come in).
//!
//! `cargo run --release -p densa-widgets --example linked` (`--light` for the light theme,
//! `--seconds N` to close after N seconds).

use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arrow_array::{
    ArrayRef, Float64Array, Int64Array, RecordBatch, StringArray, TimestampMillisecondArray,
};
use densa_arrow::{LiveModel, LiveSeries};
use densa_chart::{
    paint_bars, paint_line, points_within, y_extent, Axis, BarStyle, ChartModel, ChartPalette,
    LineStyle, Plot,
};
use densa_data::{CompareOp, Filter, LiveTable, Order, Scalar, ViewId, ViewSpec};
use densa_draw::Canvas;
use densa_ui::{Action, ActionKind, Align, Length, Style, Theme, Ui, WidgetId};
use densa_widgets::{Chart, DataGrid, Label, Panel, Splitter};
use densa_window::{App, Host, WindowOptions};

/// Instruments in the grid.
const INSTRUMENTS: usize = 200;
/// Instruments that trade each frame.
const TRADES_PER_FRAME: usize = 20;
/// Simulated milliseconds that pass each frame.
const FRAME_MS: i64 = 1_000;
/// Trades kept: two simulated hours of them.
const KEPT: i64 = 2 * 3_600 * TRADES_PER_FRAME as i64;
/// 2026-10-19 08:00:00 UTC, in milliseconds: where the history starts.
const START_MS: i64 = 1_792_396_800_000;
/// What the chart shows at first: the last half hour.
const SHOWN: f64 = 1_800.0;
/// How often the status bar is written again.
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

    /// A number in `0..1`.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1_u64 << 53) as f64
    }
}

fn symbol(index: usize) -> String {
    format!("SYM{index:03}")
}

/// The market: each instrument's price and open, the clock, and the next trade's id.
struct Market {
    feed: Feed,
    prices: Vec<f64>,
    opens: Vec<f64>,
    now_ms: i64,
    next_id: i64,
}

/// One frame's trades, as rows of both tables.
struct Trades {
    quotes: RecordBatch,
    ticks: RecordBatch,
}

impl Market {
    fn new() -> Self {
        let opens: Vec<f64> = (0..INSTRUMENTS)
            .map(|i| 20.0 + (i * 7_919 % 4_000) as f64 / 10.0)
            .collect();
        Self {
            feed: Feed { seed: 0x5EED },
            prices: opens.clone(),
            opens,
            now_ms: START_MS,
            next_id: 0,
        }
    }

    /// `frames` frames of trades: each frame some instruments trade once, prices walking
    /// from where they were.
    fn trade(&mut self, frames: usize) -> Result<Trades, Box<dyn Error>> {
        let count = frames * TRADES_PER_FRAME;
        let (mut ids, mut symbols, mut times) = (Vec::new(), Vec::new(), Vec::new());
        let (mut prices, mut sizes) = (Vec::with_capacity(count), Vec::with_capacity(count));
        let mut latest = vec![None; INSTRUMENTS];
        for _ in 0..frames {
            for _ in 0..TRADES_PER_FRAME {
                // Lossless: below 200.
                let index = (self.feed.next() % INSTRUMENTS as u64) as usize;
                let price = &mut self.prices[index];
                *price = (*price * (1.0 + (self.feed.unit() - 0.5) * 0.004)).max(0.01);
                ids.push(self.next_id);
                symbols.push(symbol(index));
                // Lossless: below a second.
                times.push(self.now_ms + (self.feed.unit() * FRAME_MS as f64) as i64);
                prices.push(*price);
                sizes.push((self.feed.unit().powi(4) * 5_000.0).round());
                latest[index] = Some(*price);
                self.next_id += 1;
            }
            self.now_ms += FRAME_MS;
        }
        let changed: Vec<usize> = (0..INSTRUMENTS).filter(|&i| latest[i].is_some()).collect();
        let quotes = RecordBatch::try_from_iter([
            (
                "symbol",
                Arc::new(StringArray::from_iter_values(
                    changed.iter().map(|&i| symbol(i)),
                )) as ArrayRef,
            ),
            (
                "last",
                Arc::new(Float64Array::from_iter_values(
                    changed.iter().map(|&i| self.prices[i]),
                )),
            ),
            (
                "chg %",
                Arc::new(Float64Array::from_iter_values(
                    changed
                        .iter()
                        .map(|&i| (self.prices[i] / self.opens[i] - 1.0) * 100.0),
                )),
            ),
        ])?;
        let ticks = RecordBatch::try_from_iter([
            ("id", Arc::new(Int64Array::from(ids)) as ArrayRef),
            ("symbol", Arc::new(StringArray::from(symbols))),
            (
                "time",
                Arc::new(TimestampMillisecondArray::from(times).with_timezone("UTC")),
            ),
            ("price", Arc::new(Float64Array::from(prices))),
            ("size", Arc::new(Float64Array::from(sizes))),
        ])?;
        Ok(Trades { quotes, ticks })
    }
}

/// One instrument's trades from the trades table: its price as a line, sizes below.
struct Trail {
    ticks: LiveTable,
    price: LiveSeries,
    size: LiveSeries,
    /// The selected instrument's trades, by time.
    view: Option<ViewId>,
    symbol: String,
}

impl Trail {
    fn new(ticks: LiveTable) -> Result<Self, Box<dyn Error>> {
        let price = LiveSeries::new(&ticks, "time", "price")?;
        let size = LiveSeries::new(&ticks, "time", "size")?;
        Ok(Self {
            ticks,
            price,
            size,
            view: None,
            symbol: String::new(),
        })
    }

    /// Shows `symbol`'s trades.
    fn show(&mut self, symbol: &str) -> Result<(), Box<dyn Error>> {
        let only = Filter::compare("symbol", CompareOp::Eq, Scalar::Str(symbol.to_owned()));
        let spec = ViewSpec::new()
            .filter(only)
            .sort_by("time", Order::Ascending);
        let view = self.ticks.add_view(spec)?;
        if let Some(old) = self.view.replace(view) {
            self.ticks.remove_view(old);
        }
        symbol.clone_into(&mut self.symbol);
        self.sync();
        Ok(())
    }

    /// Adds `trades` and deletes those older than the two hours kept.
    fn add(&mut self, trades: RecordBatch, next_id: i64) -> Result<(), Box<dyn Error>> {
        self.ticks.upsert(trades)?;
        let first_kept = next_id - KEPT;
        let gone: Vec<i64> = (first_kept - TRADES_PER_FRAME as i64..first_kept)
            .filter(|&id| id >= 0)
            .collect();
        if !gone.is_empty() {
            self.ticks.delete(&Int64Array::from(gone))?;
        }
        self.sync();
        Ok(())
    }

    /// Brings both series up to date with the table, right after it changed.
    fn sync(&mut self) {
        self.price.sync(&self.ticks);
        self.size.sync(&self.ticks);
    }
}

impl ChartModel for Trail {
    fn x_extent(&self) -> Option<(f64, f64)> {
        self.price.points(&self.ticks, self.view?)?.x_extent()
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
        let series = if pane == 0 { &self.price } else { &self.size };
        let points = series.points(&self.ticks, self.view?)?;
        let (low, high) = y_extent(&points, points_within(&points, x))?;
        if pane == 0 {
            Some(Axis::number(low, high).padded(0.08))
        } else {
            Some(Axis::number(0.0, high * 1.15))
        }
    }

    fn paint(&self, pane: usize, plot: &Plot, canvas: &mut Canvas<'_>, palette: &ChartPalette) {
        let Some(view) = self.view else {
            return;
        };
        if pane == 0 {
            if let Some(points) = self.price.points(&self.ticks, view) {
                let line = LineStyle::new(palette.line).with_width(1.5);
                paint_line(canvas, plot, &points, line);
            }
        } else if let Some(points) = self.size.points(&self.ticks, view) {
            let color = palette.label.with_alpha(0.6);
            paint_bars(canvas, plot, &points, 4.0, BarStyle::new(), |_| color);
        }
    }

    fn prepare(&mut self) {
        self.sync();
    }

    fn describe(&self) -> String {
        format!("Trades of {}", self.symbol)
    }
}

type Grid = DataGrid<LiveModel>;
type TrailChart = Chart<Trail>;

struct LinkedApp {
    light: bool,
    close_at: Option<Instant>,
    market: Market,
    ids: Option<Ids>,
    status_at: Instant,
}

struct Ids {
    grid: WidgetId,
    chart: WidgetId,
    title: WidgetId,
    status: WidgetId,
}

impl LinkedApp {
    /// The grid's model and the chart's, from two hours of trades.
    fn models(&mut self) -> Result<(LiveModel, Trail), Box<dyn Error>> {
        // Lossless: two hours of frames.
        let history = self
            .market
            .trade((KEPT / TRADES_PER_FRAME as i64) as usize)?;
        let mut quotes = LiveTable::new(history.quotes.schema(), "symbol")?;
        quotes.upsert(history.quotes)?;
        let mut grid = LiveModel::new(quotes)?;
        grid.set_decimals(1, Some(2))?;
        grid.set_decimals(2, Some(2))?;
        let mut ticks = LiveTable::new(history.ticks.schema(), "id")?;
        ticks.upsert(history.ticks)?;
        let mut trail = Trail::new(ticks)?;
        trail.show(&symbol(0))?;
        Ok((grid, trail))
    }

    /// The symbol of the grid's selected row.
    fn selected(ui: &Ui, ids: &Ids) -> Option<String> {
        let grid = ui.widget::<Grid>(ids.grid)?;
        let model = grid.model();
        let at = model.table().row_of(model.record(grid.selected()?)?)?;
        let symbols = model.table().batches()[at.batch].column(0);
        let symbols = symbols.as_any().downcast_ref::<StringArray>()?;
        Some(symbols.value(at.row).to_owned())
    }

    fn show_status(ui: &mut Ui, ids: &Ids) {
        let Some(chart) = ui.widget::<TrailChart>(ids.chart) else {
            return;
        };
        let trail = chart.model();
        let shown = trail
            .view
            .and_then(|view| trail.ticks.view(view))
            .map_or(0, |v| v.len());
        let mode = if chart.is_following() {
            "following"
        } else {
            "paused: double-click to follow"
        };
        let text = format!(
            "{} trades kept, {shown} of {}, {mode}",
            trail.ticks.len(),
            trail.symbol
        );
        let title = format!("{} trades", trail.symbol);
        ui.update::<Label, _>(ids.status, |label, cx| label.set_text(cx, text));
        ui.update::<Label, _>(ids.title, |label, cx| label.set_text(cx, title));
    }
}

impl App for LinkedApp {
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
        ui.add(toolbar, Label::new("Linked").with_size(15.0), Style::row())?;
        let hint = "Click an instrument to chart its trades";
        ui.add(toolbar, Label::new(hint).dim(), Style::row())?;

        let (grid, trail) = self.models()?;
        let splitter = Splitter::row();
        let style = splitter.style();
        let body = ui.add(root, splitter, style)?;
        let left = Grid::style().grow(0.0).width(Length::Px(240.0));
        let grid = ui.add(body, Grid::new(grid).with_frozen_columns(1), left)?;
        let right = ui.add(
            body,
            Panel::new(),
            Style::column().grow(1.0).padding(6.0).gap(6.0),
        )?;
        let title = ui.add(right, Label::new(""), Style::row().shrink(0.0))?;
        let chart = ui.add(right, TrailChart::new(trail, SHOWN), TrailChart::style())?;

        let bar = Style::row().padding(6.0).shrink(0.0);
        let bar = ui.add(root, Panel::new(), bar)?;
        let status = ui.add(bar, Label::new("").dim(), Style::row())?;
        let ids = Ids {
            grid,
            chart,
            title,
            status,
        };
        Self::show_status(ui, &ids);
        self.ids = Some(ids);
        Ok(())
    }

    fn action(&mut self, ui: &mut Ui, _: &mut Host<'_>, action: Action) {
        let Some(ids) = &self.ids else {
            return;
        };
        if action.source == ids.grid && matches!(action.kind, ActionKind::Selected(_)) {
            if let Some(symbol) = Self::selected(ui, ids) {
                ui.update::<TrailChart, _>(ids.chart, |chart, cx| {
                    // A filter on a string column always compiles.
                    let _ = chart.model_mut(cx).show(&symbol);
                    chart.follow(cx);
                });
            }
        }
        Self::show_status(ui, ids);
    }

    fn frame(&mut self, ui: &mut Ui, host: &mut Host<'_>) -> bool {
        let Some(ids) = &self.ids else {
            return false;
        };
        if let Ok(trades) = self.market.trade(1) {
            let next_id = self.market.next_id;
            ui.update::<Grid, _>(ids.grid, |grid, cx| {
                // The batch has the table's schema: an upsert cannot fail.
                let _ = grid.model_mut(cx).table_mut().upsert(trades.quotes);
            });
            ui.update::<TrailChart, _>(ids.chart, |chart, cx| {
                let _ = chart.model_mut(cx).add(trades.ticks, next_id);
            });
        }
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
    let app = LinkedApp {
        light,
        close_at: seconds.map(|s| Instant::now() + Duration::from_secs(s)),
        market: Market::new(),
        ids: None,
        status_at: Instant::now(),
    };
    densa_window::run(
        WindowOptions::new("densa linked grid and chart").with_size(1200.0, 720.0),
        app,
    )?;
    Ok(())
}
