//! Paragraphs in a window: shaped, wrapped and aligned text that re-wraps on resize.
//!
//! Run: `cargo run --release -p densa-paragraph --example paragraphs [-- --seconds N]`

use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use densa_paragraph::{Paragraph, ParagraphStyle, Span, TextAlign, TextSystem};
use densa_render::{srgb, Canvas, GpuGrid, InstanceBuffer, Palette};
use densa_text::{AtlasConfig, GlyphCache};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

const MARGIN: f32 = 24.0;
const GAP: f32 = 16.0;
const MAX_INSTANCES: usize = 8_192;

const TITLE: &str = "densa paragraphs";
const INTRO: &str = "Văn bản được shape bằng parley, xuống dòng theo chiều rộng cửa sổ và căn lề; \
                     mỗi glyph đi qua cùng một atlas với lưới dữ liệu. Kéo cửa sổ để xem các \
                     đoạn văn tự xuống dòng lại mà không phải shape lại.";
const JUSTIFIED: &str = "Justified text stretches every line but the last to the full width. \
                         Shaping happens once; resizing only re-breaks and re-aligns the lines, \
                         and drawing a laid-out paragraph each frame allocates nothing.";
const RIGHT: &str = "Right-aligned, in a calmer colour.";

fn main() -> Result<(), Box<dyn Error>> {
    let seconds = std::env::args()
        .skip_while(|a| a != "--seconds")
        .nth(1)
        .map(|s| s.parse::<u64>())
        .transpose()?;
    let event_loop = EventLoop::new()?;
    let mut app = App {
        deadline: seconds.map(|s| Instant::now() + Duration::from_secs(s)),
        state: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

struct App {
    deadline: Option<Instant>,
    state: Option<State>,
    error: Option<Box<dyn Error>>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match State::new(event_loop) {
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
            WindowEvent::Resized(size) => state.resize(size),
            WindowEvent::RedrawRequested => state.draw(),
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
    paragraphs: Vec<Paragraph>,
    background: wgpu::Color,
}

impl State {
    fn new(event_loop: &ActiveEventLoop) -> Result<Self, Box<dyn Error>> {
        let attributes = Window::default_attributes()
            .with_title(TITLE)
            .with_inner_size(PhysicalSize::new(720, 520));
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
        let gpu = GpuGrid::new(&device, view_format, MAX_INSTANCES);
        let paragraphs = build_paragraphs(text_width(size.width))?;
        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            view_format,
            gpu,
            cache,
            instances: InstanceBuffer::with_capacity(MAX_INSTANCES),
            paragraphs,
            background: Palette::new().clear_color(),
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) -> Result<(), Box<dyn Error>> {
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        let width = text_width(size.width);
        for paragraph in &mut self.paragraphs {
            paragraph.reflow(width)?; // re-break only: no shaping
        }
        Ok(())
    }

    fn draw(&mut self) -> Result<(), Box<dyn Error>> {
        let mut canvas = Canvas::begin(&mut self.cache, &mut self.instances);
        let mut y = MARGIN;
        for paragraph in &self.paragraphs {
            for run in paragraph.runs() {
                canvas.draw_run(run, [MARGIN, y]);
            }
            y += paragraph.height() + GAP;
        }
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
        self.gpu.render(&mut encoder, &view, self.background, count);
        self.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.queue.present(frame);
        Ok(())
    }
}

/// The width paragraphs wrap at inside a window `window_width` pixels wide.
fn text_width(window_width: u32) -> Option<f32> {
    Some((window_width as f32 - 2.0 * MARGIN).max(1.0))
}

fn build_paragraphs(width: Option<f32>) -> Result<Vec<Paragraph>, Box<dyn Error>> {
    let mut system = TextSystem::new()?;
    let text = srgb(0xC9D1D9, 1.0);
    let base = ParagraphStyle::new(15.0)
        .with_color(text)
        .with_max_width(width);

    let title_style = base.clone().with_align(TextAlign::Center);
    let title_spans = [Span::new(0.."densa".len())
        .with_size(30.0)
        .with_color(srgb(0x58A6FF, 1.0))];
    let accent = srgb(0x3FB950, 1.0);
    let intro_spans = [Span::new(0.."Văn bản".len()).with_color(accent)];
    Ok(vec![
        system.paragraph(TITLE, &title_style, &title_spans)?,
        system.paragraph(INTRO, &base, &intro_spans)?,
        system.paragraph(JUSTIFIED, &base.clone().with_align(TextAlign::Justify), &[])?,
        system.paragraph(
            RIGHT,
            &base
                .with_align(TextAlign::Right)
                .with_color(srgb(0x8B949E, 1.0)),
            &[],
        )?,
    ])
}
