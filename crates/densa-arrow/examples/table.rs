//! A million-row Arrow table in a window: scroll with the mouse wheel, arrows, PageUp/PageDown
//! and Home/End. `--live` appends a batch every 100 ms, like a stream, and follows the end
//! while scrolled to the bottom.
//!
//! Run: `cargo run --release -p densa-arrow --example table [-- --live] [-- --seconds N]`

use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arrow_array::types::Int32Type;
use arrow_array::{
    ArrayRef, BooleanArray, Decimal128Array, DictionaryArray, Float64Array, Int64Array,
    RecordBatch, StringArray, TimestampMillisecondArray,
};
use densa_arrow::ArrowTable;
use densa_render::{Canvas, GpuGrid, GridPainter, InstanceBuffer, Palette};
use densa_text::{AtlasConfig, Font, GlyphCache};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

const ROWS: usize = 1_000_000;
const BATCH_ROWS: usize = 65_536;
const LIVE_BATCH_ROWS: usize = 500;
const LIVE_EVERY: Duration = Duration::from_millis(100);
const VISIBLE_ROWS: u32 = 40;
const FONT_PX: f32 = 13.0;
const MAX_INSTANCES: usize = 32_768;
const REGIONS: [&str; 5] = ["north", "south", "east", "west", "central"];

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    let seconds = args
        .iter()
        .skip_while(|a| *a != "--seconds")
        .nth(1)
        .map(|s| s.parse::<u64>())
        .transpose()?;
    let event_loop = EventLoop::new()?;
    let mut app = App {
        deadline: seconds.map(|s| Instant::now() + Duration::from_secs(s)),
        live: args.iter().any(|a| a == "--live"),
        state: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

/// `len` synthetic orders starting at id `start`.
fn orders(start: usize, len: usize) -> Result<RecordBatch, Box<dyn Error>> {
    let ids: Vec<i64> = (start..start + len).map(|i| i as i64).collect();
    let customers: Vec<String> = ids
        .iter()
        .map(|i| format!("customer-{:05}", i % 50_000))
        .collect();
    let regions: DictionaryArray<Int32Type> = ids
        .iter()
        .map(|i| REGIONS[(*i as usize) % REGIONS.len()])
        .collect();
    let prices: Vec<Option<f64>> = ids
        .iter()
        .map(|&i| (i % 97 != 0).then(|| 5.0 + ((i * 7_919) % 100_000) as f64 / 100.0))
        .collect();
    let totals: Vec<i128> = ids
        .iter()
        .map(|&i| i128::from((i * 12_345) % 9_999_999))
        .collect();
    let totals = Decimal128Array::from(totals).with_precision_and_scale(18, 2)?;
    let times: Vec<i64> = ids.iter().map(|&i| 1_760_000_000_000 + i * 1_250).collect();
    let paid: Vec<bool> = ids.iter().map(|i| i % 5 != 0).collect();
    Ok(RecordBatch::try_from_iter([
        ("order_id", Arc::new(Int64Array::from(ids)) as ArrayRef),
        (
            "customer",
            Arc::new(StringArray::from(customers)) as ArrayRef,
        ),
        ("region", Arc::new(regions) as ArrayRef),
        (
            "unit_price",
            Arc::new(Float64Array::from(prices)) as ArrayRef,
        ),
        ("total", Arc::new(totals) as ArrayRef),
        (
            "placed_at",
            Arc::new(TimestampMillisecondArray::from(times)) as ArrayRef,
        ),
        ("paid", Arc::new(BooleanArray::from(paid)) as ArrayRef),
    ])?)
}

fn build_table() -> Result<ArrowTable, Box<dyn Error>> {
    let batches = (0..ROWS)
        .step_by(BATCH_ROWS)
        .map(|start| orders(start, BATCH_ROWS.min(ROWS - start)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut table = ArrowTable::from_batches(batches)?;
    table.set_decimals(3, Some(2))?;
    Ok(table)
}

struct App {
    deadline: Option<Instant>,
    live: bool,
    state: Option<State>,
    error: Option<Box<dyn Error>>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match State::new(event_loop, self.live) {
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
                state.resize(size);
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
            state.window.request_redraw();
        }
    }
}

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    view_format: wgpu::TextureFormat,
    gpu: GpuGrid,
    cache: GlyphCache,
    instances: InstanceBuffer,
    painter: GridPainter,
    palette: Palette,
    table: ArrowTable,
    first_row: usize,
    /// When to append the next live batch (`None`: not live).
    next_batch: Option<Instant>,
}

impl State {
    fn new(event_loop: &ActiveEventLoop, live: bool) -> Result<Self, Box<dyn Error>> {
        let table = build_table()?;
        let palette = Palette::new();
        let painter = GridPainter::new(
            table.columns(),
            VISIBLE_ROWS,
            Font::bundled_mono()?,
            FONT_PX,
            palette,
        );
        let layout = painter.layout();
        let attributes = Window::default_attributes()
            .with_title(title(table.rows()))
            .with_inner_size(PhysicalSize::new(layout.width(), layout.height()));
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
        let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
        cache.limit_side(GpuGrid::max_atlas_side(&device))?;
        Ok(Self {
            window,
            surface,
            gpu: GpuGrid::new(&device, view_format, MAX_INSTANCES),
            device,
            queue,
            config,
            view_format,
            cache,
            instances: InstanceBuffer::with_capacity(MAX_INSTANCES),
            painter,
            palette,
            table,
            first_row: 0,
            next_batch: live.then(Instant::now),
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

    fn last_first_row(&self) -> usize {
        self.table.rows().saturating_sub(VISIBLE_ROWS as usize)
    }

    fn scroll_by(&mut self, rows: i64) {
        let target = self.first_row as i64 + rows;
        // Lossless: row counts are far below i64::MAX.
        self.first_row = target.clamp(0, self.last_first_row() as i64) as usize;
    }

    fn key(&mut self, key: &Key) {
        let page = i64::from(VISIBLE_ROWS);
        match key {
            Key::Named(NamedKey::ArrowDown) => self.scroll_by(1),
            Key::Named(NamedKey::ArrowUp) => self.scroll_by(-1),
            Key::Named(NamedKey::PageDown) => self.scroll_by(page),
            Key::Named(NamedKey::PageUp) => self.scroll_by(-page),
            Key::Named(NamedKey::Home) => self.first_row = 0,
            Key::Named(NamedKey::End) => self.first_row = self.last_first_row(),
            _ => {}
        }
    }

    /// Appends a live batch when one is due, following the end if scrolled to it.
    fn stream(&mut self) -> Result<(), Box<dyn Error>> {
        let Some(due) = self.next_batch.filter(|due| Instant::now() >= *due) else {
            return Ok(());
        };
        let at_end = self.first_row == self.last_first_row();
        self.table
            .push(orders(self.table.rows(), LIVE_BATCH_ROWS)?)?;
        if at_end {
            self.first_row = self.last_first_row();
        }
        self.window.set_title(&title(self.table.rows()));
        self.next_batch = Some(due + LIVE_EVERY);
        Ok(())
    }

    fn frame(&mut self) -> Result<(), Box<dyn Error>> {
        self.stream()?;
        let mut canvas = Canvas::begin(&mut self.cache, &mut self.instances);
        self.painter
            .paint(&self.table.view(self.first_row, &self.palette), &mut canvas);
        self.gpu
            .apply_atlas_updates(&self.device, &self.queue, self.cache.drain_updates())?;
        let count = self.gpu.prepare(
            &self.queue,
            (self.config.width, self.config.height),
            self.instances.as_slice(),
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
        self.gpu
            .render(&mut encoder, &view, self.palette.background, count);
        self.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.queue.present(frame);
        Ok(())
    }
}

fn title(rows: usize) -> String {
    format!("densa arrow table: {rows} rows")
}
