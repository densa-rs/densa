//! A dense line: twenty million samples of a sensor read a thousand times a second, still
//! recording, kept with a `LineSummary` so each frame costs about the chart's width however
//! many samples show.
//!
//! The status bar tells how long scaling and drawing the line took a frame (the processor's
//! work, not the GPU's); `--plain` does both without the summary, visiting every sample that
//! shows, to compare. Scroll to zoom around the pointer, drag to pan, double-click (or press
//! End) to follow the newest samples again.
//!
//! `cargo run --release -p densa-widgets --example dense` (`--plain`, `--light` for the light
//! theme, `--seconds N` to close after N seconds).

use std::cell::Cell;
use std::error::Error;
use std::time::{Duration, Instant};

use densa_chart::{
    paint_line, paint_line_summarized, points_within, y_extent, Axis, ChartModel, ChartPalette,
    LineStyle, LineSummary, Plot,
};
use densa_draw::Canvas;
use densa_ui::{Action, Align, Style, Theme, Ui, WidgetId};
use densa_widgets::{Chart, Label, Panel};
use densa_window::{App, Host, WindowOptions};

/// Samples recorded before the window opens, and how many a second.
const SAMPLES: usize = 20_000_000;
const RATE: f64 = 1_000.0;
/// Samples that arrive each frame: about real time at 60 frames a second.
const PER_FRAME: usize = 16;
/// 2026-10-16 00:00:00 UTC, where the recording starts.
const START: f64 = 1_792_108_800.0;
/// How often the status bar is written again.
const STATUS_EVERY: Duration = Duration::from_millis(250);

/// Random numbers: wyrand, deterministic.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0xA076_1D64_78BD_642F);
        let t = u128::from(self.0) * u128::from(self.0 ^ 0xE703_7ED1_A0B4_28DB);
        // Lossless: the halves of a 128-bit product.
        (t >> 64) as u64 ^ t as u64
    }

    /// A number in `-0.5..0.5`.
    fn centred(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1_u64 << 53) as f64 - 0.5
    }
}

/// The sensor's samples (seconds since the epoch, reading; a dropout is a reading that is not
/// a number), and their summary, kept up to date as samples arrive.
struct Recording {
    noise: Noise,
    times: Vec<f64>,
    readings: Vec<f64>,
    level: f64,
    /// Samples left in a dropout.
    dropout: u32,
    summary: LineSummary,
    /// Whether to draw without the summary.
    plain: bool,
    /// The time spent scaling and drawing the line, and the frames it was drawn in, since the
    /// status bar last told it.
    spent: Cell<Duration>,
    frames: Cell<u32>,
}

impl Recording {
    fn new(plain: bool) -> Self {
        let mut recording = Self {
            noise: Noise(0x5EED),
            times: Vec::with_capacity(SAMPLES + SAMPLES / 4),
            readings: Vec::with_capacity(SAMPLES + SAMPLES / 4),
            level: 20.0,
            dropout: 0,
            summary: LineSummary::new(),
            plain,
            spent: Cell::new(Duration::ZERO),
            frames: Cell::new(0),
        };
        for _ in 0..SAMPLES {
            recording.sample();
        }
        recording.summary = LineSummary::of(&recording.points());
        recording
    }

    fn points(&self) -> (&[f64], &[f64]) {
        (&self.times, &self.readings)
    }

    /// The next sample: a drifting level with noise, and now and then a dropout.
    fn sample(&mut self) {
        // Lossless enough: sample counts far below 2^53.
        let time = START + self.times.len() as f64 / RATE;
        self.level += self.noise.centred() * 0.05;
        if self.dropout == 0 && self.noise.next().is_multiple_of(400_000) {
            self.dropout = 2_000;
        }
        let reading = if self.dropout > 0 {
            self.dropout -= 1;
            f64::NAN
        } else {
            self.level + self.noise.centred() * 2.0
        };
        self.times.push(time);
        self.readings.push(reading);
    }

    /// The time spent on the line a frame since this was last asked, and starts counting
    /// again.
    fn spent_a_frame(&self) -> Option<Duration> {
        let (spent, frames) = (self.spent.take(), self.frames.take());
        (frames > 0).then(|| spent / frames)
    }

    /// The samples of the last frame, taken into the summary.
    fn record(&mut self) {
        for _ in 0..PER_FRAME {
            self.sample();
        }
        let (times, readings) = (&self.times[..], &self.readings[..]);
        self.summary.update(&(times, readings));
    }
}

impl ChartModel for Recording {
    fn x_extent(&self) -> Option<(f64, f64)> {
        Some((*self.times.first()?, *self.times.last()?))
    }

    fn x_step(&self) -> Option<f64> {
        Some(1.0 / RATE)
    }

    fn y_axis(&self, _pane: usize, x: &Axis) -> Option<Axis> {
        let started = Instant::now();
        let points = self.points();
        let shown = points_within(&points, x);
        let found = if self.plain {
            y_extent(&points, shown)
        } else {
            self.summary.y_extent(&points, shown)
        };
        self.spent.set(self.spent.get() + started.elapsed());
        let (low, high) = found?;
        Some(Axis::number(low, high).padded(0.08))
    }

    fn paint(&self, _pane: usize, plot: &Plot, canvas: &mut Canvas<'_>, palette: &ChartPalette) {
        let started = Instant::now();
        let style = LineStyle::new(palette.line);
        if self.plain {
            paint_line(canvas, plot, &self.points(), style);
        } else {
            paint_line_summarized(canvas, plot, &self.points(), &self.summary, style);
        }
        self.spent.set(self.spent.get() + started.elapsed());
        self.frames.set(self.frames.get() + 1);
    }

    fn pane_title(&self, _pane: usize) -> &str {
        "Sensor, 1,000 samples a second"
    }

    fn describe(&self) -> String {
        format!("Sensor readings, {} samples", self.times.len())
    }
}

type SensorChart = Chart<Recording>;

struct DenseApp {
    plain: bool,
    light: bool,
    close_at: Option<Instant>,
    ids: Option<Ids>,
    status_at: Instant,
}

struct Ids {
    chart: WidgetId,
    status: WidgetId,
}

/// `n` with a comma between groups of three digits.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut text = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

impl DenseApp {
    /// Tells how many samples there are and how long scaling and drawing the line took a
    /// frame since it was last told.
    fn show_status(ui: &mut Ui, ids: &Ids) {
        let Some(chart) = ui.widget::<SensorChart>(ids.chart) else {
            return;
        };
        let recording = chart.model();
        let Some(spent) = recording.spent_a_frame() else {
            return;
        };
        let how = if recording.plain {
            "every sample that shows visited (--plain)"
        } else {
            "through its summary"
        };
        let micros = spent.as_secs_f64() * 1e6;
        let samples = grouped(recording.times.len());
        let text =
            format!("{samples} samples · line scaled and drawn in {micros:.0} µs a frame, {how}");
        ui.update::<Label, _>(ids.status, |label, cx| label.set_text(cx, text));
    }
}

impl App for DenseApp {
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
        ui.add(
            toolbar,
            Label::new("Dense line").with_size(15.0),
            Style::row(),
        )?;
        let hint = "Scroll to zoom, drag to pan, double-click to follow";
        ui.add(toolbar, Label::new(hint).dim(), Style::row())?;

        // Lossless enough: the whole recording, in seconds.
        let shown = SAMPLES as f64 / RATE;
        let chart = SensorChart::new(Recording::new(self.plain), shown);
        let chart = ui.add(root, chart, SensorChart::style())?;

        let bar = Style::row().padding(6.0).shrink(0.0);
        let bar = ui.add(root, Panel::new(), bar)?;
        let status = ui.add(bar, Label::new("").dim(), Style::row())?;
        self.ids = Some(Ids { chart, status });
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
        ui.update::<SensorChart, _>(ids.chart, |chart, cx| chart.model_mut(cx).record());
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
    let flag = |name: &str| args.iter().any(|a| a == name);
    let seconds = args
        .iter()
        .skip_while(|a| *a != "--seconds")
        .nth(1)
        .map(|s| s.parse::<u64>())
        .transpose()?;
    let app = DenseApp {
        plain: flag("--plain"),
        light: flag("--light"),
        close_at: seconds.map(|s| Instant::now() + Duration::from_secs(s)),
        ids: None,
        status_at: Instant::now(),
    };
    densa_window::run(
        WindowOptions::new("densa dense line").with_size(1100.0, 600.0),
        app,
    )?;
    Ok(())
}
