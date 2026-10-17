//! Windowed mode: winit event loop + wgpu surface, presenting with V-Sync (FIFO).
//!
//! Glue only: frame content comes from `GridPainter` (unit-tested, allocation-free) and
//! drawing from `GpuGrid` (GPU-tested). This module wires events, timing and reporting.

use crate::pinning::resolve_worker_pin;
use crate::report::{format_cpu, format_duration_ns as dur, format_latency, format_rate as rate};
use crate::scroll::{wheel_rows, Scroll, ScrollAction};
use crate::{Args, VIEWPORT_ROWS};
use densa_alloc::{measure, AllocStats};
use densa_data::ViewportSnapshot;
use densa_engine::{
    spawn_worker, Clock, FrameProbe, LatencyHistogram, WorkerHandle, WorkerMetrics,
};
use densa_render::{GlyphAtlas, GpuGrid, GridLayout, GridPainter, InstanceBuffer};
use densa_sync::Reader;
use std::error::Error;
use std::io::Write;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const NANOS_PER_SEC: u64 = 1_000_000_000;

/// Opens the grid window and runs until it is closed (or `--seconds` elapse), then prints
/// a summary to stdout.
pub fn run_window(args: &Args) -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::new()?;
    let mut app = App {
        args: args.clone(),
        session: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    if let Some(e) = app.error {
        return Err(e);
    }
    match app.session {
        Some(session) => session.finish(&mut std::io::stdout().lock()),
        None => Err("the window was never created".into()),
    }
}

struct App {
    args: Args,
    session: Option<Session>,
    error: Option<Box<dyn Error>>,
}

impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: Box<dyn Error>) {
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.session.is_some() {
            return;
        }
        match Session::new(event_loop, &self.args, &mut std::io::stdout().lock()) {
            Ok(session) => {
                session.window.request_redraw();
                self.session = Some(session);
            }
            Err(e) => self.fail(event_loop, e),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(session) = &mut self.session else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => session.resize(size),
            WindowEvent::MouseWheel { delta, .. } => {
                let rows = match delta {
                    MouseScrollDelta::LineDelta(_, y) => wheel_rows(f64::from(y), false, 0),
                    MouseScrollDelta::PixelDelta(p) => {
                        wheel_rows(p.y, true, session.painter.layout().row_height)
                    }
                };
                session.scroll(ScrollAction::Rows(rows));
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let action = match event.logical_key {
                    Key::Named(NamedKey::Escape) => {
                        event_loop.exit();
                        return;
                    }
                    Key::Named(NamedKey::ArrowDown) => ScrollAction::Rows(1),
                    Key::Named(NamedKey::ArrowUp) => ScrollAction::Rows(-1),
                    Key::Named(NamedKey::PageDown) => ScrollAction::Pages(1),
                    Key::Named(NamedKey::PageUp) => ScrollAction::Pages(-1),
                    Key::Named(NamedKey::Home) => ScrollAction::Top,
                    Key::Named(NamedKey::End) => ScrollAction::Bottom,
                    _ => return,
                };
                session.scroll(action);
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = session.frame() {
                    self.fail(event_loop, e);
                    return;
                }
                if session.expired() {
                    event_loop.exit();
                } else {
                    // Continuous rendering; FIFO presentation paces it to the display.
                    session.window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

/// Per-second and whole-run frame statistics.
#[derive(Default)]
struct FrameStats {
    window_probe: FrameProbe,
    total_probe: FrameProbe,
    /// Time to build the frame's instances (`GridPainter::paint`).
    paint: LatencyHistogram,
    paint_total: LatencyHistogram,
    /// Start-to-start time between frames: the presentation cadence.
    interval: LatencyHistogram,
    interval_total: LatencyHistogram,
    last_frame_start: Option<u64>,
    paint_heap_ops: u64,
    gpu_heap_ops: u64,
    skipped_frames: u64,
    reconfigures: u64,
}

fn heap_ops(stats: AllocStats) -> u64 {
    stats.allocs + stats.deallocs + stats.reallocs
}

struct Session {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    view_format: wgpu::TextureFormat,
    gpu: GpuGrid,
    painter: GridPainter,
    instances: InstanceBuffer,
    reader: Reader<ViewportSnapshot<VIEWPORT_ROWS>>,
    worker: WorkerHandle,
    clock: Clock,
    scroll: Scroll,
    stats: FrameStats,
    start_ns: u64,
    next_report_ns: u64,
    deadline_ns: Option<u64>,
    last_worker: WorkerMetrics,
}

impl Session {
    fn new(
        event_loop: &ActiveEventLoop,
        args: &Args,
        out: &mut impl Write,
    ) -> Result<Self, Box<dyn Error>> {
        // Lossless: VIEWPORT_ROWS is a small constant.
        let layout = GridLayout::new(VIEWPORT_ROWS as u32);
        let attributes = Window::default_attributes()
            .with_title("densa")
            .with_inner_size(PhysicalSize::new(layout.width(), layout.height()));
        let window = Arc::new(event_loop.create_window(attributes)?);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(event_loop.owned_display_handle()),
        ));
        let surface = instance.create_surface(Arc::clone(&window))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("densa"),
                ..Default::default()
            }))?;

        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("the GPU adapter cannot present to this window")?;
        // FIFO is V-Sync: presentation waits for the display's refresh, never tears.
        config.present_mode = wgpu::PresentMode::Fifo;
        let view_format = config.format.add_srgb_suffix();
        if view_format != config.format {
            config.view_formats.push(view_format);
        }
        surface.configure(&device, &config);
        let gpu = GpuGrid::new(&device, &queue, view_format, layout.max_instances());

        let mut worker_config = args.worker.clone();
        let (pin, pin_note) = resolve_worker_pin(args.pin, out)?;
        worker_config.pin = pin;
        let clock = Clock::new();
        let symbols = worker_config.symbols;
        let (worker, reader) = spawn_worker::<VIEWPORT_ROWS>(worker_config, clock)?;

        let info = adapter.get_info();
        writeln!(
            out,
            "densa window: {} ({:?}), {}x{} px, {:?}, {} symbols, rate {}, {}",
            info.name,
            info.backend,
            config.width,
            config.height,
            config.present_mode,
            symbols,
            args.worker.rate.map_or_else(
                || "unbounded".to_owned(),
                |r| format!("{} ticks/s", rate(r as f64))
            ),
            pin_note,
        )?;

        let start_ns = clock.now_ns();
        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            view_format,
            gpu,
            painter: GridPainter::new(layout, &GlyphAtlas::font8x8()),
            instances: InstanceBuffer::with_capacity(layout.max_instances()),
            reader,
            worker,
            clock,
            scroll: Scroll::new(symbols, layout.rows),
            stats: FrameStats::default(),
            start_ns,
            next_report_ns: start_ns + NANOS_PER_SEC,
            deadline_ns: args.seconds.map(|s| start_ns + s * NANOS_PER_SEC),
            last_worker: WorkerMetrics::default(),
        })
    }

    fn expired(&self) -> bool {
        self.deadline_ns.is_some_and(|d| self.clock.now_ns() >= d)
    }

    fn scroll(&mut self, action: ScrollAction) {
        if self.scroll.apply(action) {
            self.worker.set_viewport(self.scroll.first_row());
        }
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return; // Minimized: keep the old configuration until restored.
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
    }

    fn frame(&mut self) -> Result<(), Box<dyn Error>> {
        let start = self.clock.now_ns();
        if let Some(previous) = self.stats.last_frame_start {
            self.stats.interval.record(start - previous);
            self.stats.interval_total.record(start - previous);
        }
        self.stats.last_frame_start = Some(start);

        let fresh = self.reader.update();
        let now = self.clock.now_ns();
        let (reader, painter, instances, stats) = (
            &self.reader,
            &mut self.painter,
            &mut self.instances,
            &mut self.stats,
        );
        let ((), paint_heap) = measure(|| {
            reader.read(|snapshot| {
                stats.window_probe.observe(snapshot, fresh, now);
                stats.total_probe.observe(snapshot, fresh, now);
                painter.paint(snapshot, now, instances);
            });
        });
        let paint_ns = self.clock.now_ns() - now;
        self.stats.paint.record(paint_ns);
        self.stats.paint_total.record(paint_ns);
        self.stats.paint_heap_ops += heap_ops(paint_heap);

        // wgpu allocates internally per frame; measured separately and reported, not hidden.
        let (presented, gpu_heap) = measure(|| self.submit());
        self.stats.gpu_heap_ops += heap_ops(gpu_heap);
        if !presented? {
            self.stats.skipped_frames += 1;
        }

        if self.clock.now_ns() >= self.next_report_ns {
            self.report_second()?;
        }
        Ok(())
    }

    /// Uploads, draws and presents. `Ok(false)` means the frame was skipped (surface busy,
    /// occluded or just reconfigured).
    fn submit(&mut self) -> Result<bool, Box<dyn Error>> {
        let count = self.gpu.prepare(
            &self.queue,
            (self.config.width, self.config.height),
            self.instances.as_slice(),
        );
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                // Present this one, then reconfigure for the next.
                self.stats.reconfigures += 1;
                self.surface.configure(&self.device, &self.config);
                frame
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.stats.reconfigures += 1;
                self.surface.configure(&self.device, &self.config);
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("surface texture acquisition failed validation".into());
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.view_format),
            ..Default::default()
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.gpu.render(
            &mut encoder,
            &view,
            self.painter.palette().clear_color(),
            count,
        );
        self.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.queue.present(frame);
        Ok(true)
    }

    fn report_second(&mut self) -> Result<(), Box<dyn Error>> {
        let now = self.clock.now_ns();
        let worker = self.worker.metrics();
        let probe = self.stats.window_probe.report();
        let ticks = worker.ticks_applied - self.last_worker.ticks_applied;
        let second = (now - self.start_ns) / NANOS_PER_SEC;
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "[{second:>3}s] {} fps | paint {} | frame interval {} | snapshot age {} | \
             tick age {} | {} ticks/s | worker cpu {} | heap ops: worker {}, paint {}, gpu submit {}",
            probe.frames,
            format_latency(&self.stats.paint.summary()),
            format_latency(&self.stats.interval.summary()),
            format_latency(&probe.snapshot_age),
            format_latency(&probe.tick_age),
            rate(ticks as f64),
            format_cpu(worker.last_cpu),
            worker.heap_ops,
            self.stats.paint_heap_ops,
            self.stats.gpu_heap_ops,
        )?;
        self.window.set_title(&format!(
            "densa | {} fps | {} ticks/s | snapshot age p99 {} | rows {}-{}",
            probe.frames,
            rate(ticks as f64),
            dur(probe.snapshot_age.p99),
            self.scroll.first_row(),
            self.scroll.first_row() + self.painter.layout().rows - 1,
        ));

        self.stats.window_probe.reset();
        self.stats.paint.reset();
        self.stats.interval.reset();
        self.last_worker = worker;
        self.next_report_ns += NANOS_PER_SEC;
        Ok(())
    }

    fn finish(self, out: &mut impl Write) -> Result<(), Box<dyn Error>> {
        let elapsed_s = (self.clock.now_ns() - self.start_ns) as f64 / 1e9;
        let worker = self.worker.stop()?;
        let probe = self.stats.total_probe.report();
        writeln!(
            out,
            "summary: {elapsed_s:.2}s | {} frames ({:.1} fps, {} skipped, {} reconfigures) | \
             paint {} | frame interval {} | snapshot age {} | tick age {} | {} ticks/s | \
             worker cpu {}, migrations {} | heap ops after startup: worker {}, paint {}, gpu submit {}",
            probe.frames,
            probe.frames as f64 / elapsed_s,
            self.stats.skipped_frames,
            self.stats.reconfigures,
            format_latency(&self.stats.paint_total.summary()),
            format_latency(&self.stats.interval_total.summary()),
            format_latency(&probe.snapshot_age),
            format_latency(&probe.tick_age),
            rate(worker.ticks_applied as f64 / elapsed_s),
            format_cpu(worker.last_cpu),
            worker.cpu_migrations,
            worker.heap_ops,
            self.stats.paint_heap_ops,
            self.stats.gpu_heap_ops,
        )?;
        Ok(())
    }
}
