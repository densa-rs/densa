use crate::report::{format_duration_ns as dur, format_rate as rate};
use crate::Args;
use densa_alloc::measure;
use densa_data::{RowSnapshot, ViewportSnapshot};
use densa_engine::{
    spawn_worker, Clock, FrameProbe, LatencySummary, ProbeReport, WorkerHandle, WorkerMetrics,
};
use densa_fmt::{write_fixed, write_grouped, FmtBuf};
use densa_sync::Reader;
use std::error::Error;
use std::hint::black_box;
use std::io::Write;
use std::time::Duration;

/// Rows in the visible viewport of the virtual grid.
pub const VIEWPORT_ROWS: usize = 50;

const NANOS_PER_SEC: u64 = 1_000_000_000;
const PRICE_DECIMALS: usize = 2;

/// Totals of a headless run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunSummary {
    pub worker: WorkerMetrics,
    pub frames: u64,
    /// Heap operations inside the measured per-frame work (excludes reporting).
    pub reader_heap_ops: u64,
    pub elapsed_s: f64,
    pub snapshot_age: LatencySummary,
    pub tick_age: LatencySummary,
}

/// Runs the worker plus a fixed-rate reader loop on the calling thread, printing one
/// metrics line per second to `out`, then a summary.
///
/// The reader does everything a real frame does except touching the GPU: picks up the
/// newest snapshot, records latency, formats every visible cell as text, and scrolls.
pub fn run_headless(args: &Args, out: &mut impl Write) -> Result<RunSummary, Box<dyn Error>> {
    let clock = Clock::new();
    let (worker, mut reader) = spawn_worker::<VIEWPORT_ROWS>(args.worker.clone(), clock)?;
    writeln!(
        out,
        "densa headless: {} symbols, rate {}, batch {}, publish every {}, {} fps, {} s",
        args.worker.symbols,
        args.worker.rate.map_or_else(
            || "unbounded".to_owned(),
            |r| format!("{} ticks/s", rate(r as f64))
        ),
        args.worker.batch_size,
        dur(args.worker.publish_interval_ns),
        args.fps,
        args.seconds,
    )?;

    let mut frame = Frame::new(args.worker.symbols);
    let mut total = FrameProbe::new();
    let frame_ns = NANOS_PER_SEC / u64::from(args.fps);
    let start = clock.now_ns();
    let end = start + args.seconds * NANOS_PER_SEC;
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
    cell: FmtBuf<32>,
    first_row: u32,
    max_first_row: u32,
    scrolling_down: bool,
}

impl Frame {
    fn new(symbols: u32) -> Self {
        Self {
            window: FrameProbe::new(),
            cell: FmtBuf::new(),
            first_row: 0,
            // Lossless: VIEWPORT_ROWS is a small constant.
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
        let (window, cell) = (&mut self.window, &mut self.cell);
        reader.read(|snapshot| {
            window.observe(snapshot, fresh, now);
            total.observe(snapshot, fresh, now);
            black_box(format_rows(snapshot.visible(), cell));
        });
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

/// Formats every visible cell the way the grid will draw it; returns total text bytes.
fn format_rows(rows: &[RowSnapshot], cell: &mut FmtBuf<32>) -> usize {
    let mut bytes = 0;
    for row in rows {
        for price in [row.bid, row.ask, row.last] {
            cell.clear();
            if write_fixed(cell, price, PRICE_DECIMALS).is_err() {
                // No quote yet (NaN). A one-byte placeholder always fits.
                cell.clear();
                let _ = cell.push_str("-");
            }
            bytes += cell.len();
        }
        cell.clear();
        // u64::MAX grouped is 26 bytes: always fits in 32.
        let _ = write_grouped(cell, row.volume, ',');
        bytes += cell.len();
    }
    bytes
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
         snapshot age {} | tick age {} | heap ops: worker {}, frame loop {}",
        rate((now.ticks_applied - last.ticks_applied) as f64),
        rate((now.publishes - last.publishes) as f64),
        frames.frames,
        frames.fresh_frames,
        frames.skipped_snapshots,
        latency(&frames.snapshot_age),
        latency(&frames.tick_age),
        now.heap_ops,
        reader_heap_ops,
    )
}

fn summary_line(out: &mut impl Write, s: &RunSummary) -> std::io::Result<()> {
    writeln!(
        out,
        "summary: {:.2}s | {} ticks ({} ticks/s, {} rejected, {} pacer resets) | {} publishes | \
         {} frames ({:.1} fps) | snapshot age {} | tick age {} | heap ops after startup: \
         worker {}, frame loop {}",
        s.elapsed_s,
        s.worker.ticks_applied,
        rate(s.worker.ticks_applied as f64 / s.elapsed_s),
        s.worker.ticks_rejected,
        s.worker.lag_resets,
        s.worker.publishes,
        s.frames,
        s.frames as f64 / s.elapsed_s,
        latency(&s.snapshot_age),
        latency(&s.tick_age),
        s.worker.heap_ops,
        s.reader_heap_ops,
    )
}

fn latency(s: &LatencySummary) -> String {
    format!("p50 {} p99 {} max {}", dur(s.p50), dur(s.p99), dur(s.max))
}
