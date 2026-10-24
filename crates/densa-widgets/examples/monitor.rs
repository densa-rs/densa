//! A fleet monitor: 1,000 simulated hosts in a live table, each reporting its CPU, memory,
//! latency and traffic every five seconds, sorted with the slowest first. Click a host and
//! the chart shows its latency and traffic over the last half hour: a view of a live
//! samples table, filtered by host and sorted by time (an hour is kept; older samples are
//! deleted as new ones arrive).
//!
//! The samples come the way an application's data does: a worker thread simulates the fleet,
//! sends each second's samples over a channel and wakes the window; each frame takes in what
//! arrived, and nothing is repainted while nothing comes.
//!
//! `cargo run --release -p densa-widgets --example monitor` (`--light` for the light theme,
//! `--seconds N` to close after N seconds).

use std::error::Error;
use std::f64::consts::TAU;
use std::sync::mpsc::{self, Receiver};
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
use densa_grid::{GridModel, SortOrder};
use densa_ui::{Action, ActionKind, Align, Length, Style, Theme, Ui, WidgetId};
use densa_widgets::{Chart, DataGrid, Label, Panel, Splitter};
use densa_window::{App, Host, WindowOptions};

/// Hosts in the fleet.
const HOSTS: usize = 1_000;
/// Seconds between two samples of a host.
const EVERY: usize = 5;
/// Simulated milliseconds in each of the worker's steps: a second of the fleet.
const FRAME_MS: i64 = 1_000;
/// Real time between two of the worker's steps: a simulated minute passes each real second.
const STEP: Duration = Duration::from_millis(16);
/// Samples kept: an hour of them.
const KEPT: i64 = 3_600 * (HOSTS / EVERY) as i64;
/// 2026-10-19 08:00:00 UTC, in milliseconds: where the history starts.
const START_MS: i64 = 1_792_396_800_000;
/// What the chart shows at first: the last half hour.
const SHOWN: f64 = 1_800.0;
/// How often the status bar is written again.
const STATUS_EVERY: Duration = Duration::from_millis(250);
/// Services, with the latency they answer in when all is well (milliseconds).
const SERVICES: [(&str, f64); 8] = [
    ("api", 42.0),
    ("web", 28.0),
    ("db", 9.0),
    ("cache", 2.5),
    ("queue", 6.0),
    ("search", 65.0),
    ("auth", 18.0),
    ("billing", 35.0),
];
const REGIONS: [&str; 3] = ["eu", "us", "ap"];
/// The latency column, which the grid is sorted by.
const P99: usize = 5;

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

/// One host: what it runs, where, and how it is doing.
struct Machine {
    name: String,
    service: usize,
    region: usize,
    /// Requests a second at the busiest time of the cycle.
    peak: f64,
    phase: f64,
    memory: f64,
    /// Until when (seconds) an incident slows it down, and by how much.
    incident: Option<(i64, f64)>,
}

/// The fleet: every host and the clock.
struct Fleet {
    feed: Feed,
    machines: Vec<Machine>,
    second: i64,
    next_id: i64,
}

/// One step's samples, as rows of both tables.
struct Samples {
    hosts: RecordBatch,
    samples: RecordBatch,
    /// The id the next sample will have.
    next_id: i64,
}

impl Fleet {
    fn new() -> Self {
        let mut feed = Feed { seed: 0xF1EE7 };
        let machines = (0..HOSTS)
            .map(|i| {
                let (service, region) = (i % SERVICES.len(), i / SERVICES.len() % REGIONS.len());
                let n = i / (SERVICES.len() * REGIONS.len()) + 1;
                Machine {
                    name: format!("{}-{}-{n:02}", SERVICES[service].0, REGIONS[region]),
                    service,
                    region,
                    peak: 200.0 + feed.unit() * 1_800.0,
                    phase: feed.unit() * TAU,
                    memory: 35.0 + feed.unit() * 40.0,
                    incident: None,
                }
            })
            .collect();
        Self {
            feed,
            machines,
            second: 0,
            next_id: 0,
        }
    }

    /// `seconds` seconds of samples: each second, a fifth of the hosts report.
    fn sample(&mut self, seconds: usize) -> Result<Samples, Box<dyn Error>> {
        let mut rows = Rows::default();
        let mut latest = vec![None; HOSTS];
        for _ in 0..seconds {
            // Lossless: a second's position in the reporting cycle.
            let turn = self.second.rem_euclid(EVERY as i64) as usize;
            for index in (turn..HOSTS).step_by(EVERY) {
                let sample = self.measure(index);
                latest[index] = Some(sample);
                rows.push(
                    self.next_id,
                    &self.machines[index].name,
                    self.time_ms(),
                    sample,
                );
                self.next_id += 1;
            }
            self.second += 1;
        }
        let changed: Vec<(usize, Sample)> = latest
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.map(|s| (i, s)))
            .collect();
        let machines = &self.machines;
        let column = |f: fn(&Sample) -> f64| {
            Arc::new(Float64Array::from_iter_values(
                changed.iter().map(|(_, s)| f(s)),
            )) as ArrayRef
        };
        let hosts = RecordBatch::try_from_iter([
            (
                "host",
                Arc::new(StringArray::from_iter_values(
                    changed.iter().map(|(i, _)| machines[*i].name.as_str()),
                )) as ArrayRef,
            ),
            (
                "service",
                Arc::new(StringArray::from_iter_values(
                    changed
                        .iter()
                        .map(|(i, _)| SERVICES[machines[*i].service].0),
                )),
            ),
            (
                "region",
                Arc::new(StringArray::from_iter_values(
                    changed.iter().map(|(i, _)| REGIONS[machines[*i].region]),
                )),
            ),
            ("cpu %", column(|s| s.cpu)),
            ("mem %", column(|s| s.memory)),
            ("p99 ms", column(|s| s.p99)),
            ("req/s", column(|s| s.requests)),
        ])?;
        Ok(Samples {
            hosts,
            samples: rows.batch()?,
            next_id: self.next_id,
        })
    }

    /// Host `index` now: traffic follows a slow cycle, CPU the traffic, latency the CPU, and
    /// now and then an incident slows a host down for a few minutes.
    fn measure(&mut self, index: usize) -> Sample {
        let now = self.second;
        let noise = self.feed.unit() - 0.5;
        let spark = self.feed.unit();
        let length = self.feed.unit();
        let factor = self.feed.unit();
        let machine = &mut self.machines[index];
        if machine.incident.is_some_and(|(until, _)| until <= now) {
            machine.incident = None;
        }
        if machine.incident.is_none() && spark < 0.0004 {
            // Lossless: two to ten minutes.
            let minutes = (2.0 + length * 8.0) as i64;
            machine.incident = Some((now + minutes * 60, 2.5 + factor * 5.0));
        }
        // Lossless enough: seconds of a simulation.
        let cycle = 0.6 + 0.4 * (now as f64 / 1_200.0 * TAU + machine.phase).sin();
        let requests = (machine.peak * cycle * (1.0 + noise * 0.15)).max(1.0);
        let cpu = (8.0 + requests / 2_000.0 * 70.0 + noise * 6.0).clamp(1.0, 99.0);
        machine.memory = (machine.memory + noise * 0.4).clamp(20.0, 92.0);
        let load = 1.0 + 3.0 * ((cpu - 70.0) / 30.0).max(0.0);
        let slowed = machine.incident.map_or(1.0, |(_, factor)| factor);
        let p99 = SERVICES[machine.service].1 * (1.0 + noise * 0.3) * load * slowed;
        Sample {
            cpu,
            memory: machine.memory,
            p99,
            requests,
        }
    }

    fn time_ms(&self) -> i64 {
        START_MS + self.second * FRAME_MS
    }
}

/// What a host reports.
#[derive(Debug, Clone, Copy)]
struct Sample {
    cpu: f64,
    memory: f64,
    p99: f64,
    requests: f64,
}

/// Samples being gathered into a batch of the samples table.
#[derive(Default)]
struct Rows {
    ids: Vec<i64>,
    hosts: Vec<String>,
    times: Vec<i64>,
    p99: Vec<f64>,
    requests: Vec<f64>,
}

impl Rows {
    fn push(&mut self, id: i64, host: &str, time: i64, sample: Sample) {
        self.ids.push(id);
        self.hosts.push(host.to_owned());
        self.times.push(time);
        self.p99.push(sample.p99);
        self.requests.push(sample.requests);
    }

    fn batch(self) -> Result<RecordBatch, Box<dyn Error>> {
        Ok(RecordBatch::try_from_iter([
            ("id", Arc::new(Int64Array::from(self.ids)) as ArrayRef),
            ("host", Arc::new(StringArray::from(self.hosts))),
            (
                "time",
                Arc::new(TimestampMillisecondArray::from(self.times).with_timezone("UTC")),
            ),
            ("p99 ms", Arc::new(Float64Array::from(self.p99))),
            ("req/s", Arc::new(Float64Array::from(self.requests))),
        ])?)
    }
}

/// One host's samples from the samples table: latency as a line, traffic below.
struct Trail {
    samples: LiveTable,
    latency: LiveSeries,
    traffic: LiveSeries,
    view: Option<ViewId>,
    host: String,
}

impl Trail {
    fn new(samples: LiveTable) -> Result<Self, Box<dyn Error>> {
        let latency = LiveSeries::new(&samples, "time", "p99 ms")?;
        let traffic = LiveSeries::new(&samples, "time", "req/s")?;
        Ok(Self {
            samples,
            latency,
            traffic,
            view: None,
            host: String::new(),
        })
    }

    /// Shows `host`'s samples.
    fn show(&mut self, host: &str) -> Result<(), Box<dyn Error>> {
        let only = Filter::compare("host", CompareOp::Eq, Scalar::Str(host.to_owned()));
        let spec = ViewSpec::new()
            .filter(only)
            .sort_by("time", Order::Ascending);
        let view = self.samples.add_view(spec)?;
        if let Some(old) = self.view.replace(view) {
            self.samples.remove_view(old);
        }
        host.clone_into(&mut self.host);
        self.sync();
        Ok(())
    }

    /// Adds `samples` and deletes those older than the hour kept.
    fn add(&mut self, samples: RecordBatch, next_id: i64) -> Result<(), Box<dyn Error>> {
        // Lossless: a second of samples.
        let added = samples.num_rows() as i64;
        self.samples.upsert(samples)?;
        let first_kept = next_id - KEPT;
        let gone: Vec<i64> = (first_kept - added..first_kept)
            .filter(|&id| id >= 0)
            .collect();
        if !gone.is_empty() {
            self.samples.delete(&Int64Array::from(gone))?;
        }
        self.sync();
        Ok(())
    }

    /// Brings both series up to date with the table, right after it changed.
    fn sync(&mut self) {
        self.latency.sync(&self.samples);
        self.traffic.sync(&self.samples);
    }
}

impl ChartModel for Trail {
    fn x_extent(&self) -> Option<(f64, f64)> {
        self.latency.points(&self.samples, self.view?)?.x_extent()
    }

    fn x_step(&self) -> Option<f64> {
        // Lossless: five seconds.
        Some(EVERY as f64)
    }

    fn panes(&self) -> usize {
        2
    }

    fn pane_weight(&self, pane: usize) -> f32 {
        if pane == 0 {
            2.5
        } else {
            1.0
        }
    }

    fn y_axis(&self, pane: usize, x: &Axis) -> Option<Axis> {
        let series = if pane == 0 {
            &self.latency
        } else {
            &self.traffic
        };
        let points = series.points(&self.samples, self.view?)?;
        let (_, high) = y_extent(&points, points_within(&points, x))?;
        // Latency and traffic both read best from zero.
        Some(Axis::number(0.0, high * 1.12))
    }

    fn paint(&self, pane: usize, plot: &Plot, canvas: &mut Canvas<'_>, palette: &ChartPalette) {
        let Some(view) = self.view else {
            return;
        };
        if pane == 0 {
            if let Some(points) = self.latency.points(&self.samples, view) {
                let line = LineStyle::new(palette.line).with_width(1.5);
                paint_line(canvas, plot, &points, line);
            }
        } else if let Some(points) = self.traffic.points(&self.samples, view) {
            let color = palette.up.with_alpha(0.55);
            // Lossless: five seconds.
            paint_bars(canvas, plot, &points, EVERY as f64, BarStyle::new(), |_| {
                color
            });
        }
    }

    fn prepare(&mut self) {
        self.sync();
    }

    fn describe(&self) -> String {
        format!("Latency and traffic of {}", self.host)
    }
}

type Grid = DataGrid<LiveModel>;
type TrailChart = Chart<Trail>;

struct MonitorApp {
    light: bool,
    close_at: Option<Instant>,
    /// The fleet, until `build` hands it to the worker thread.
    fleet: Option<Fleet>,
    /// What the worker sends.
    arrivals: Option<Receiver<Samples>>,
    ids: Option<Ids>,
    status_at: Instant,
}

struct Ids {
    grid: WidgetId,
    chart: WidgetId,
    title: WidgetId,
    status: WidgetId,
}

impl MonitorApp {
    /// The grid's model and the chart's, from an hour of samples; the chart shows the
    /// slowest host.
    fn models(&mut self) -> Result<(LiveModel, Trail), Box<dyn Error>> {
        // Lossless: an hour of seconds.
        let fleet = self.fleet.as_mut().ok_or("the fleet is with the worker")?;
        let history = fleet.sample((KEPT / (HOSTS / EVERY) as i64) as usize)?;
        let slowest = slowest(&history.hosts);
        let mut hosts = LiveTable::new(history.hosts.schema(), "host")?;
        hosts.upsert(history.hosts)?;
        let mut grid = LiveModel::new(hosts)?;
        for column in 3..=P99 {
            grid.set_decimals(column, Some(1))?;
        }
        grid.set_decimals(P99 + 1, Some(0))?;
        grid.sort(P99, SortOrder::Descending);
        let mut samples = LiveTable::new(history.samples.schema(), "id")?;
        samples.upsert(history.samples)?;
        let mut trail = Trail::new(samples)?;
        trail.show(&slowest)?;
        Ok((grid, trail))
    }

    /// The host of the grid's selected row.
    fn selected(ui: &Ui, ids: &Ids) -> Option<String> {
        let grid = ui.widget::<Grid>(ids.grid)?;
        let model = grid.model();
        let at = model.table().row_of(model.record(grid.selected()?)?)?;
        let hosts = model.table().batches()[at.batch].column(0);
        let hosts = hosts.as_any().downcast_ref::<StringArray>()?;
        Some(hosts.value(at.row).to_owned())
    }

    fn show_status(ui: &mut Ui, ids: &Ids) {
        let Some(chart) = ui.widget::<TrailChart>(ids.chart) else {
            return;
        };
        let trail = chart.model();
        let mode = if chart.is_following() {
            "following"
        } else {
            "paused: double-click to follow"
        };
        let text = format!(
            "{HOSTS} hosts, {} samples kept, {mode}",
            trail.samples.len()
        );
        let title = format!("{}: p99 latency (ms) and requests per second", trail.host);
        ui.update::<Label, _>(ids.status, |label, cx| label.set_text(cx, text));
        ui.update::<Label, _>(ids.title, |label, cx| label.set_text(cx, title));
    }
}

/// The host with the highest latency in `hosts`.
fn slowest(hosts: &RecordBatch) -> String {
    let names = hosts.column(0).as_any().downcast_ref::<StringArray>();
    let p99 = hosts.column(P99).as_any().downcast_ref::<Float64Array>();
    let (Some(names), Some(p99)) = (names, p99) else {
        return String::new();
    };
    (0..hosts.num_rows())
        .max_by(|&a, &b| p99.value(a).total_cmp(&p99.value(b)))
        .map_or_else(String::new, |row| names.value(row).to_owned())
}

impl App for MonitorApp {
    fn build(&mut self, ui: &mut Ui, host: &mut Host<'_>) -> Result<(), Box<dyn Error>> {
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
        ui.add(toolbar, Label::new("Fleet").with_size(15.0), Style::row())?;
        let hint = "Slowest first. Click a host to chart its latency and traffic";
        ui.add(toolbar, Label::new(hint).dim(), Style::row())?;

        let (grid, trail) = self.models()?;
        let splitter = Splitter::row();
        let style = splitter.style();
        let body = ui.add(root, splitter, style)?;
        let left = Grid::style().grow(0.0).width(Length::Px(560.0));
        let grid = ui.add(body, Grid::new(grid).with_frozen_columns(1), left)?;
        let right = Style::column().grow(1.0).padding(6.0).gap(6.0);
        let right = ui.add(body, Panel::new(), right)?;
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

        // From here on the fleet lives on a thread of its own, as a data source would: it
        // never touches the interface, only sends samples and wakes the window.
        let mut fleet = self.fleet.take().ok_or("built twice")?;
        let (send, arrivals) = mpsc::channel();
        let waker = host.waker();
        std::thread::spawn(move || loop {
            std::thread::sleep(STEP);
            let Ok(samples) = fleet.sample(1) else {
                return;
            };
            if send.send(samples).is_err() {
                return; // the window closed
            }
            waker.wake();
        });
        self.arrivals = Some(arrivals);
        Ok(())
    }

    fn action(&mut self, ui: &mut Ui, _: &mut Host<'_>, action: Action) {
        let Some(ids) = &self.ids else {
            return;
        };
        if action.source == ids.grid && matches!(action.kind, ActionKind::Selected(_)) {
            if let Some(host) = Self::selected(ui, ids) {
                ui.update::<TrailChart, _>(ids.chart, |chart, cx| {
                    // A filter on a string column always compiles.
                    let _ = chart.model_mut(cx).show(&host);
                    chart.follow(cx);
                });
            }
        }
        Self::show_status(ui, ids);
    }

    /// Takes in the samples that arrived since the last frame. Returns `false`: the next frame
    /// comes when the worker wakes the window again (or the user does something).
    fn frame(&mut self, ui: &mut Ui, host: &mut Host<'_>) -> bool {
        let (Some(ids), Some(arrivals)) = (&self.ids, &self.arrivals) else {
            return false;
        };
        for samples in arrivals.try_iter() {
            ui.update::<Grid, _>(ids.grid, |grid, cx| {
                // The batch has the table's schema: an upsert cannot fail.
                let _ = grid.model_mut(cx).table_mut().upsert(samples.hosts);
            });
            ui.update::<TrailChart, _>(ids.chart, |chart, cx| {
                let _ = chart.model_mut(cx).add(samples.samples, samples.next_id);
            });
        }
        if self.status_at.elapsed() >= STATUS_EVERY {
            self.status_at = Instant::now();
            Self::show_status(ui, ids);
        }
        if self.close_at.is_some_and(|at| Instant::now() >= at) {
            host.close();
        }
        false
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
    let app = MonitorApp {
        light,
        close_at: seconds.map(|s| Instant::now() + Duration::from_secs(s)),
        fleet: Some(Fleet::new()),
        arrivals: None,
        ids: None,
        status_at: Instant::now(),
    };
    densa_window::run(
        WindowOptions::new("densa fleet monitor").with_size(1280.0, 760.0),
        app,
    )?;
    Ok(())
}
