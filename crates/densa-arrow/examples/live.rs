//! A live table in a window: 100,000 instruments updated 100,000 times a second, and a "top
//! movers" view (sorted by change, descending) that densa-data keeps in order as prices move.
//! Scroll with the mouse wheel, arrows, PageUp/PageDown and Home/End.
//!
//! Run: `cargo run --release -p densa-arrow --example live [-- --rate N] [-- --seconds N]`

use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray, UInt64Array};
use densa_arrow::LiveGrid;
use densa_data::{LiveTable, Order, ViewId, ViewSpec};
use densa_draw::{Canvas, InstanceBuffer};
use densa_grid::{GridPainter, Palette};
use densa_render::GpuRenderer;
use densa_text::{AtlasConfig, Font, GlyphCache};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const INSTRUMENTS: u64 = 100_000;
const DEFAULT_RATE: u64 = 100_000;
const VISIBLE_ROWS: u32 = 40;
const FONT_PX: f32 = 13.0;
const MAX_INSTANCES: usize = 32_768;
const TITLE_EVERY: Duration = Duration::from_secs(1);

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let value_of = |flag: &str| -> Result<Option<u64>, Box<dyn Error>> {
        Ok(args
            .iter()
            .skip_while(|a| *a != flag)
            .nth(1)
            .map(|s| s.parse::<u64>())
            .transpose()?)
    };
    let seconds = value_of("--seconds")?;
    let rate = value_of("--rate")?.unwrap_or(DEFAULT_RATE);
    let event_loop = EventLoop::new()?;
    let mut app = App {
        deadline: seconds.map(|s| Instant::now() + Duration::from_secs(s)),
        rate,
        state: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

/// The synthetic market: each instrument's opening and last price.
struct Market {
    open: Vec<f64>,
    last: Vec<f64>,
    volume: Vec<u64>,
    state: u64,
}

impl Market {
    fn new() -> Self {
        let open: Vec<f64> = (0..INSTRUMENTS)
            .map(|i| 10.0 + ((i * 7_919) % 99_000) as f64 / 100.0)
            .collect();
        Self {
            last: open.clone(),
            open,
            volume: vec![0; INSTRUMENTS as usize],
            state: 0x9E37_79B9_7F4A_7C15,
        }
    }

    fn next(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    /// `ticks` random price moves, as one batch (the last move of an instrument wins).
    fn ticks(&mut self, ticks: usize) -> Result<RecordBatch, Box<dyn Error>> {
        let mut symbols = Vec::with_capacity(ticks);
        let (mut last, mut change, mut volume) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..ticks {
            let i = (self.next() % INSTRUMENTS) as usize;
            let step = (self.next() % 2_001) as f64 / 1_000.0 - 1.0; // -1% .. +1%
            self.last[i] = (self.last[i] * (1.0 + step / 100.0)).max(0.01);
            self.volume[i] += 1 + self.next() % 500;
            symbols.push(format!("SYM{i:05}"));
            last.push(self.last[i]);
            change.push((self.last[i] / self.open[i] - 1.0) * 100.0);
            volume.push(self.volume[i]);
        }
        Ok(quotes(symbols, last, change, volume)?)
    }

    /// Every instrument at its opening price.
    fn opening(&self) -> Result<RecordBatch, Box<dyn Error>> {
        let symbols = (0..INSTRUMENTS).map(|i| format!("SYM{i:05}")).collect();
        let zeros = vec![0.0; INSTRUMENTS as usize];
        Ok(quotes(
            symbols,
            self.open.clone(),
            zeros,
            self.volume.clone(),
        )?)
    }
}

fn quotes(
    symbols: Vec<String>,
    last: Vec<f64>,
    change: Vec<f64>,
    volume: Vec<u64>,
) -> Result<RecordBatch, arrow_schema::ArrowError> {
    RecordBatch::try_from_iter([
        ("symbol", Arc::new(StringArray::from(symbols)) as ArrayRef),
        ("last", Arc::new(Float64Array::from(last)) as ArrayRef),
        (
            "change_pct",
            Arc::new(Float64Array::from(change)) as ArrayRef,
        ),
        ("volume", Arc::new(UInt64Array::from(volume)) as ArrayRef),
    ])
}

struct App {
    deadline: Option<Instant>,
    rate: u64,
    state: Option<State>,
    error: Option<Box<dyn Error>>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match State::new(event_loop, self.rate) {
            Ok(state) => self.state = Some(state),
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.state else {
            return;
        };
        let result = match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                Ok(())
            }
            WindowEvent::Resized(size) => {
                state.gpu.resize(size);
                Ok(())
            }
            WindowEvent::RedrawRequested => state.frame(),
            WindowEvent::MouseWheel { delta, .. } => {
                let rows = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -(y * 3.0) as i64,
                    MouseScrollDelta::PixelDelta(p) => -(p.y / 16.0) as i64,
                };
                state.scroll_by(rows);
                Ok(())
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                state.key(&event.logical_key);
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.error = Some(e);
            event_loop.exit();
        }
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            event_loop.exit();
        } else {
            state.gpu.window.request_redraw();
        }
    }
}

/// The window, surface and renderer.
struct Gpu {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    view_format: wgpu::TextureFormat,
    renderer: GpuRenderer,
}

impl Gpu {
    fn new(event_loop: &ActiveEventLoop, size: PhysicalSize<u32>) -> Result<Self, Box<dyn Error>> {
        let attributes = Window::default_attributes()
            .with_title("densa live")
            .with_inner_size(size);
        let window = Arc::new(event_loop.create_window(attributes)?);
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(event_loop.owned_display_handle()),
        ));
        let surface = instance.create_surface(Arc::clone(&window))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("the GPU adapter cannot present to this window")?;
        config.present_mode = wgpu::PresentMode::Fifo;
        let view_format = config.format.add_srgb_suffix();
        if view_format != config.format {
            config.view_formats.push(view_format);
        }
        surface.configure(&device, &config);
        let renderer = GpuRenderer::new(&device, view_format, MAX_INSTANCES);
        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            view_format,
            renderer,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
    }

    fn present(
        &mut self,
        cache: &mut GlyphCache,
        instances: &InstanceBuffer,
        palette: &Palette,
    ) -> Result<(), Box<dyn Error>> {
        self.renderer
            .apply_atlas_updates(&self.device, &self.queue, cache.drain_updates())?;
        let count = self.renderer.prepare(
            &self.queue,
            (self.config.width, self.config.height),
            instances.as_slice(),
        );
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("surface texture acquisition failed validation".into());
            }
            _ => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.view_format),
            ..Default::default()
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.renderer
            .render(&mut encoder, &view, palette.background, count);
        self.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.queue.present(frame);
        Ok(())
    }
}

struct State {
    gpu: Gpu,
    cache: GlyphCache,
    instances: InstanceBuffer,
    painter: GridPainter,
    palette: Palette,
    market: Market,
    table: LiveTable,
    grid: LiveGrid,
    movers: ViewId,
    first_row: usize,
    rate: u64,
    last_frame: Instant,
    /// Ticks applied and time spent applying them since the title was last updated.
    window_ticks: u64,
    window_upsert: Duration,
    window_start: Instant,
}

impl State {
    fn new(event_loop: &ActiveEventLoop, rate: u64) -> Result<Self, Box<dyn Error>> {
        let market = Market::new();
        let opening = market.opening()?;
        let mut table = LiveTable::new(opening.schema(), "symbol")?;
        let movers = table.add_view(ViewSpec::new().sort_by("change_pct", Order::Descending))?;
        table.upsert(opening)?;
        let mut grid = LiveGrid::new(&table);
        grid.set_decimals(1, Some(2))?;
        grid.set_decimals(2, Some(3))?;
        grid.set_column_chars(1, 9)?;
        grid.set_column_chars(2, 10)?;
        grid.set_column_chars(3, 12)?;
        let palette = Palette::new();
        let painter = GridPainter::new(
            grid.columns(),
            VISIBLE_ROWS,
            Font::bundled_mono()?,
            FONT_PX,
            palette,
        );
        let layout = painter.layout();
        let size = PhysicalSize::new(layout.width(), layout.height());
        let gpu = Gpu::new(event_loop, size)?;
        let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
        cache.limit_side(GpuRenderer::max_atlas_side(&gpu.device))?;
        let now = Instant::now();
        Ok(Self {
            gpu,
            cache,
            instances: InstanceBuffer::with_capacity(MAX_INSTANCES),
            painter,
            palette,
            market,
            table,
            grid,
            movers,
            first_row: 0,
            rate,
            last_frame: now,
            window_ticks: 0,
            window_upsert: Duration::ZERO,
            window_start: now,
        })
    }

    fn scroll_by(&mut self, rows: i64) {
        let last = self.table.len().saturating_sub(VISIBLE_ROWS as usize) as i64;
        // Lossless: row counts are far below i64::MAX.
        self.first_row = (self.first_row as i64 + rows).clamp(0, last) as usize;
    }

    fn key(&mut self, key: &Key) {
        let page = i64::from(VISIBLE_ROWS);
        match key {
            Key::Named(NamedKey::ArrowDown) => self.scroll_by(1),
            Key::Named(NamedKey::ArrowUp) => self.scroll_by(-1),
            Key::Named(NamedKey::PageDown) => self.scroll_by(page),
            Key::Named(NamedKey::PageUp) => self.scroll_by(-page),
            Key::Named(NamedKey::Home) => self.first_row = 0,
            Key::Named(NamedKey::End) => self.scroll_by(i64::MAX / 2),
            _ => {}
        }
    }

    /// Applies the ticks due since the last frame and reports the rate in the title.
    fn tick(&mut self) -> Result<(), Box<dyn Error>> {
        let now = Instant::now();
        let elapsed = now - self.last_frame;
        self.last_frame = now;
        // Lossless enough: at most a few seconds' worth of ticks.
        let due = (self.rate as f64 * elapsed.as_secs_f64()).min(self.rate as f64) as usize;
        if due > 0 {
            let batch = self.market.ticks(due)?;
            let start = Instant::now();
            self.table.upsert(batch)?;
            self.window_upsert += start.elapsed();
            self.window_ticks += due as u64;
        }
        let window = now - self.window_start;
        if window >= TITLE_EVERY {
            let per_second = self.window_ticks as f64 / window.as_secs_f64();
            let per_tick = self.window_upsert.as_secs_f64() * 1e9 / self.window_ticks.max(1) as f64;
            let status = format!(
                "densa live | {} instruments | {per_second:.0} ticks/s | view upkeep {per_tick:.0} ns/tick",
                self.table.len()
            );
            println!("{status}");
            self.gpu.window.set_title(&status);
            self.window_ticks = 0;
            self.window_upsert = Duration::ZERO;
            self.window_start = now;
        }
        Ok(())
    }

    fn frame(&mut self) -> Result<(), Box<dyn Error>> {
        self.tick()?;
        let view = self
            .grid
            .view(&self.table, self.movers, self.first_row, &self.palette)
            .ok_or("the movers view exists")?;
        let mut canvas = Canvas::begin(&mut self.cache, &mut self.instances);
        self.painter.paint(&view, &mut canvas);
        self.gpu
            .present(&mut self.cache, &self.instances, &self.palette)
    }
}
