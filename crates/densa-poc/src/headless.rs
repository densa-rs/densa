use crate::pinning::{describe_pin, worker_pin};
use crate::report::{
    format_cpu, format_duration_ns as dur, format_latency, format_priority, format_rate as rate,
};
use crate::Args;
use densa_alloc::measure;
use densa_data::ViewportSnapshot;
use densa_engine::{
    spawn_worker, Clock, FrameProbe, LatencySummary, ProbeReport, WorkerHandle, WorkerMetrics,
};
use densa_platform::logical_cpus;
use densa_render::{GlyphAtlas, GridLayout, GridPainter, InstanceBuffer};
use densa_sync::Reader;
use std::error::Error;
use std::hint::black_box;
use std::io::Write;
use std::time::Duration;

/// Rows in the visible viewport of the virtual grid.
pub const VIEWPORT_ROWS: usize = 50;
/// Headless run length when `--seconds` is not given.
pub const DEFAULT_HEADLESS_SECONDS: u64 = 10;

const NANOS_PER_SEC: u64 = 1_000_000_000;

/// Totals of a headless run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunSummary {
    /// Final worker counters.
    pub worker: WorkerMetrics,
    /// Frames rendered.
    pub frames: u64,
    /// Heap operations inside the measured per-frame work (excludes reporting).
    pub reader_heap_ops: u64,
    /// Wall-clock run time in seconds.
    pub elapsed_s: f64,
    /// Publish-to-display latency over the whole run.
    pub snapshot_age: LatencySummary,
    /// Tick-to-display latency of the newest visible tick, over the whole run.
    pub tick_age: LatencySummary,
}

/// Runs the worker plus a fixed-rate reader loop on the calling thread, printing one
/// metrics line per second to `out`, then a summary.
///
/// Each frame does everything a windowed frame does except touching the GPU: picks up
/// the newest snapshot, records latency, builds the grid's instances with the same
/// `GridPainter` the window uses, and scrolls.
pub fn run_headless(args: &Args, out: &mut impl Write) -> Result<RunSummary, Box<dyn Error>> {
    let seconds = args.seconds.unwrap_or(DEFAULT_HEADLESS_SECONDS);
    let mut config = args.worker.clone();
    config.pin = worker_pin(args.pin)?;

    let clock = Clock::new();
    let (worker, mut reader) = spawn_worker::<VIEWPORT_ROWS>(config, clock)?;
    let pin_note = describe_pin(
        worker.pinned_cpu(),
        args.pin,
        &logical_cpus().unwrap_or_default(),
    );
    let pin_note = format!(
        "{pin_note}, priority {}",
        format_priority(worker.priority())
    );
    writeln!(
        out,
        "densa headless: {} symbols, rate {}, batch {}, publish every {}, {} fps, {} s, {}",
        args.worker.symbols,
        args.worker.rate.map_or_else(
            || "unbounded".to_owned(),
            |r| format!("{} ticks/s", rate(r as f64))
        ),
        args.worker.batch_size,
        dur(args.worker.publish_interval_ns),
        args.fps,
        seconds,
        pin_note,
    )?;

    let mut frame = Frame::new(args.worker.symbols);
    let mut total = FrameProbe::new();
    let frame_ns = NANOS_PER_SEC / u64::from(args.fps);
    let start = clock.now_ns();
    let end = start + seconds * NANOS_PER_SEC;
    let mut next_frame = start;
    let mut next_report = start + NANOS_PER_SEC;
    let mut last = WorkerMetrics::default();
    let mut reader_heap_ops = 0;

    while clock.now_ns() < end {
        let ((), heap) = measure(|| frame.render(&mut reader, &worker, &mut total, clock));
        reader_heap_ops += heap.allocs + heap.deallocs + heap.reallocs;

        let now = clock.now_ns();
        if now >= next_report {
            let metrics = worker.metrics();
            let second = (now - start) / NANOS_PER_SEC;
            report_line(
                out,
                second,
                &last,
                &metrics,
                &frame.window.report(),
                reader_heap_ops,
            )?;
            frame.window.reset();
            last = metrics;
            next_report += NANOS_PER_SEC;
        }

        next_frame += frame_ns;
        if next_frame > now {
            std::thread::sleep(Duration::from_nanos(next_frame - now));
        } else if now - next_frame > frame_ns {
            next_frame = now; // Far behind: drop frames instead of bursting.
        }
    }

    let elapsed_s = (clock.now_ns() - start) as f64 / 1e9;
    let worker_metrics = worker.stop()?;
    let report = total.report();
    let summary = RunSummary {
        worker: worker_metrics,
        frames: report.frames,
        reader_heap_ops,
        elapsed_s,
        snapshot_age: report.snapshot_age,
        tick_age: report.tick_age,
    };
    summary_line(out, &summary)?;
    Ok(summary)
}

/// Per-frame state of the simulated render loop. Everything is preallocated.
struct Frame {
    window: FrameProbe,
    painter: GridPainter,
    instances: InstanceBuffer,
    first_row: u32,
    max_first_row: u32,
    scrolling_down: bool,
}

impl Frame {
    fn new(symbols: u32) -> Self {
        // Lossless: VIEWPORT_ROWS is a small constant.
        let layout = GridLayout::new(VIEWPORT_ROWS as u32);
        Self {
            window: FrameProbe::new(),
            painter: GridPainter::new(layout, &GlyphAtlas::font8x8()),
            instances: InstanceBuffer::with_capacity(layout.max_instances()),
            first_row: 0,
            max_first_row: symbols.saturating_sub(VIEWPORT_ROWS as u32),
            scrolling_down: true,
        }
    }

    fn render(
        &mut self,
        reader: &mut Reader<ViewportSnapshot<VIEWPORT_ROWS>>,
        worker: &WorkerHandle,
        total: &mut FrameProbe,
        clock: Clock,
    ) {
        let fresh = reader.update();
        let now = clock.now_ns();
        let (window, painter, instances) =
            (&mut self.window, &mut self.painter, &mut self.instances);
        reader.read(|snapshot| {
            window.observe(snapshot, fresh, now);
            total.observe(snapshot, fresh, now);
            painter.paint(snapshot, now, instances);
        });
        black_box(self.instances.as_slice());
        self.scroll();
        worker.set_viewport(self.first_row);
    }

    /// Scrolls one row per frame, bouncing between the top and the last full page.
    fn scroll(&mut self) {
        if self.scrolling_down && self.first_row >= self.max_first_row {
            self.scrolling_down = false;
        } else if !self.scrolling_down && self.first_row == 0 {
            self.scrolling_down = true;
        }
        if self.scrolling_down {
            self.first_row = (self.first_row + 1).min(self.max_first_row);
        } else {
            self.first_row = self.first_row.saturating_sub(1);
        }
    }
}

fn report_line(
    out: &mut impl Write,
    second: u64,
    last: &WorkerMetrics,
    now: &WorkerMetrics,
    frames: &ProbeReport,
    reader_heap_ops: u64,
) -> std::io::Result<()> {
    writeln!(
        out,
        "[{second:>3}s] {} ticks/s | {} publishes/s | {} frames ({} fresh, {} skipped) | \
         snapshot age {} | tick age {} | worker cpu {} (+{} migrations) | \
         heap ops: worker {}, frame loop {}",
        rate((now.ticks_applied - last.ticks_applied) as f64),
        rate((now.publishes - last.publishes) as f64),
        frames.frames,
        frames.fresh_frames,
        frames.skipped_snapshots,
        format_latency(&frames.snapshot_age),
        format_latency(&frames.tick_age),
        format_cpu(now.last_cpu),
        now.cpu_migrations - last.cpu_migrations,
        now.heap_ops,
        reader_heap_ops,
    )
}

fn summary_line(out: &mut impl Write, s: &RunSummary) -> std::io::Result<()> {
    writeln!(
        out,
        "summary: {:.2}s | {} ticks ({} ticks/s, {} rejected, {} pacer resets) | {} publishes | \
         {} frames ({:.1} fps) | snapshot age {} | tick age {} | worker cpu {}, migrations {} | \
         heap ops after startup: worker {}, frame loop {}",
        s.elapsed_s,
        s.worker.ticks_applied,
        rate(s.worker.ticks_applied as f64 / s.elapsed_s),
        s.worker.ticks_rejected,
        s.worker.lag_resets,
        s.worker.publishes,
        s.frames,
        s.frames as f64 / s.elapsed_s,
        format_latency(&s.snapshot_age),
        format_latency(&s.tick_age),
        format_cpu(s.worker.last_cpu),
        s.worker.cpu_migrations,
        s.worker.heap_ops,
        s.reader_heap_ops,
    )
}
