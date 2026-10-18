//! Paragraphs in a window: shaped, wrapped and aligned text that re-wraps on resize, with
//! selection: click to place the caret, drag, shift+click or shift+arrows to select,
//! double-click for a word, triple-click for a line, ctrl+arrows by word, Home/End by line
//! (ctrl: whole text).
//!
//! Run: `cargo run --release -p densa-paragraph --example paragraphs [-- --seconds N]`
//! (`--select` starts with a word selected, for screenshots).

use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use densa_paragraph::{
    Motion, Paragraph, ParagraphStyle, Span, TextAlign, TextSelection, TextSystem,
};
use densa_render::{Canvas, Color, GpuGrid, InstanceBuffer, Palette, Point};
use densa_text::{AtlasConfig, GlyphCache};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

const MARGIN: f32 = 24.0;
const GAP: f32 = 16.0;
const MAX_INSTANCES: usize = 8_192;
const CARET_WIDTH: f32 = 2.0;
/// Clicks closer than this (in time and space) count as a double or triple click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const DOUBLE_CLICK_SLOP: f32 = 4.0;

const TITLE: &str = "densa paragraphs";
const INTRO: &str = "Text is shaped by parley, wrapped to the width of the window and aligned; \
                     every glyph comes from the same atlas as the data grid. Resize the window \
                     and the lines re-wrap without shaping again.";
const CENTERED: &str = "Centered lines suit captions and headings.";
const JUSTIFIED: &str = "Justified text stretches every line but the last to the full width. \
                         Shaping happens once; resizing only re-breaks and re-aligns the lines, \
                         and drawing a laid-out paragraph each frame allocates nothing.";
const RIGHT: &str = "Right-aligned, in a calmer color.";

fn main() -> Result<(), Box<dyn Error>> {
    let seconds = std::env::args()
        .skip_while(|a| a != "--seconds")
        .nth(1)
        .map(|s| s.parse::<u64>())
        .transpose()?;
    let preselect = std::env::args().any(|a| a == "--select");
    let event_loop = EventLoop::new()?;
    let mut app = App {
        deadline: seconds.map(|s| Instant::now() + Duration::from_secs(s)),
        preselect,
        state: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

struct App {
    deadline: Option<Instant>,
    preselect: bool,
    state: Option<State>,
    error: Option<Box<dyn Error>>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match State::new(event_loop, self.preselect) {
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
            WindowEvent::ModifiersChanged(modifiers) => {
                state.modifiers = modifiers.state();
                Ok(())
            }
            WindowEvent::CursorMoved { position, .. } => {
                // Lossless enough: window coordinates are small.
                state.pointer_moved(Point::new(position.x as f32, position.y as f32));
                Ok(())
            }
            WindowEvent::MouseInput {
                state: pressed,
                button: MouseButton::Left,
                ..
            } => {
                state.left_button(pressed == ElementState::Pressed);
                Ok(())
            }
            WindowEvent::KeyboardInput { event, .. } => {
                state.key(&event);
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
    paragraphs: Vec<Paragraph>,
    background: Color,
    /// Top of each paragraph, as last drawn.
    tops: Vec<f32>,
    /// The paragraph holding the caret, and its selection.
    selection: Option<(usize, TextSelection)>,
    dragging: bool,
    pointer: Point,
    modifiers: ModifiersState,
    /// When and where the last click was, and how many clicks in a row it made.
    last_click: Option<(Instant, Point, u8)>,
}

impl State {
    fn new(event_loop: &ActiveEventLoop, preselect: bool) -> Result<Self, Box<dyn Error>> {
        let attributes = Window::default_attributes()
            .with_title(TITLE)
            .with_inner_size(PhysicalSize::new(720, 320));
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
        let selection = preselect.then(|| {
            let intro = &paragraphs[1];
            let start = INTRO.find("parley").unwrap_or(0);
            let word = TextSelection::caret_at(intro, start).moved(intro, Motion::WordRight, true);
            (1, word)
        });
        let tops = stacked_tops(&paragraphs);
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
            background: Palette::new().background,
            tops,
            selection,
            dragging: false,
            pointer: Point::ORIGIN,
            modifiers: ModifiersState::empty(),
            last_click: None,
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
        if let Some((index, selection)) = &mut self.selection {
            *selection = selection.refreshed(&self.paragraphs[*index]);
        }
        self.tops = stacked_tops(&self.paragraphs);
        Ok(())
    }

    fn draw(&mut self) -> Result<(), Box<dyn Error>> {
        let mut canvas = Canvas::begin(&mut self.cache, &mut self.instances);
        let selected = Color::srgb(0x1F6FEB).with_alpha(0.45);
        let caret = Color::srgb(0x58A6FF);
        for (index, (paragraph, &top)) in self.paragraphs.iter().zip(&self.tops).enumerate() {
            let origin = Point::new(MARGIN, top);
            let active = self.selection.filter(|(i, _)| *i == index);
            if let Some((_, selection)) = active {
                selection.for_each_rect(paragraph, |rect| {
                    canvas.fill_rect(rect.translate(origin), selected);
                });
            }
            for run in paragraph.runs() {
                canvas.draw_run(run, origin);
            }
            if let Some((_, selection)) = active {
                let rect = selection.caret_rect(paragraph, CARET_WIDTH);
                canvas.fill_rect(rect.translate(origin), caret);
            }
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

impl State {
    /// The paragraph under `point` (or nearest above it) and `point` relative to it.
    fn hit(&self, point: Point) -> Option<(usize, Point)> {
        let index = self
            .tops
            .iter()
            .rposition(|&top| top <= point.y)
            .unwrap_or(0);
        let top = *self.tops.get(index)?;
        Some((index, Point::new(point.x - MARGIN, point.y - top)))
    }

    fn pointer_moved(&mut self, point: Point) {
        self.pointer = point;
        if !self.dragging {
            return;
        }
        let Some((index, selection)) = self.selection else {
            return;
        };
        let local = Point::new(point.x - MARGIN, point.y - self.tops[index]);
        let extended = selection.extend_to_point(&self.paragraphs[index], local);
        self.selection = Some((index, extended));
    }

    fn left_button(&mut self, pressed: bool) {
        self.dragging = pressed;
        if !pressed {
            return;
        }
        let Some((index, local)) = self.hit(self.pointer) else {
            return;
        };
        let now = Instant::now();
        let repeated = self.last_click.is_some_and(|(at, p, _)| {
            now - at < DOUBLE_CLICK
                && (p.x - self.pointer.x).abs() < DOUBLE_CLICK_SLOP
                && (p.y - self.pointer.y).abs() < DOUBLE_CLICK_SLOP
        });
        let clicks = match self.last_click {
            Some((_, _, count)) if repeated => count % 3 + 1,
            _ => 1,
        };
        self.last_click = Some((now, self.pointer, clicks));
        let paragraph = &self.paragraphs[index];
        let current = self.selection.filter(|(i, _)| *i == index);
        let selection = match (clicks, current) {
            (2, _) => TextSelection::word_at_point(paragraph, local),
            (3, _) => TextSelection::line_at_point(paragraph, local),
            (_, Some((_, current))) if self.modifiers.shift_key() => {
                current.shift_click(paragraph, local)
            }
            _ => TextSelection::at_point(paragraph, local),
        };
        self.selection = Some((index, selection));
    }

    fn key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let Some((index, selection)) = self.selection else {
            return;
        };
        let ctrl = self.modifiers.control_key();
        let motion = match &event.logical_key {
            Key::Named(NamedKey::ArrowLeft) if ctrl => Motion::WordLeft,
            Key::Named(NamedKey::ArrowRight) if ctrl => Motion::WordRight,
            Key::Named(NamedKey::ArrowLeft) => Motion::Left,
            Key::Named(NamedKey::ArrowRight) => Motion::Right,
            Key::Named(NamedKey::ArrowUp) => Motion::Up,
            Key::Named(NamedKey::ArrowDown) => Motion::Down,
            Key::Named(NamedKey::Home) if ctrl => Motion::TextStart,
            Key::Named(NamedKey::End) if ctrl => Motion::TextEnd,
            Key::Named(NamedKey::Home) => Motion::LineStart,
            Key::Named(NamedKey::End) => Motion::LineEnd,
            _ => return,
        };
        let extend = self.modifiers.shift_key();
        let moved = selection.moved(&self.paragraphs[index], motion, extend);
        self.selection = Some((index, moved));
    }
}

/// Top of each paragraph, stacked from the margin down.
fn stacked_tops(paragraphs: &[Paragraph]) -> Vec<f32> {
    let mut y = MARGIN;
    paragraphs
        .iter()
        .map(|p| {
            let top = y;
            y += p.height() + GAP;
            top
        })
        .collect()
}

/// The width paragraphs wrap at inside a window `window_width` pixels wide.
fn text_width(window_width: u32) -> Option<f32> {
    Some((window_width as f32 - 2.0 * MARGIN).max(1.0))
}

fn build_paragraphs(width: Option<f32>) -> Result<Vec<Paragraph>, Box<dyn Error>> {
    let mut system = TextSystem::new()?;
    let text = Color::srgb(0xC9D1D9);
    let base = ParagraphStyle::new(15.0)
        .with_color(text)
        .with_max_width(width);

    let title_style = base.clone().with_align(TextAlign::Center);
    let title_spans = [Span::new(0.."densa".len())
        .with_size(30.0)
        .with_color(Color::srgb(0x58A6FF))];
    let accent = Color::srgb(0x3FB950);
    let intro_spans = [Span::new(0.."Text".len()).with_color(accent)];
    Ok(vec![
        system.paragraph(TITLE, &title_style, &title_spans)?,
        system.paragraph(INTRO, &base, &intro_spans)?,
        system.paragraph(JUSTIFIED, &base.clone().with_align(TextAlign::Justify), &[])?,
        system.paragraph(CENTERED, &base.clone().with_align(TextAlign::Center), &[])?,
        system.paragraph(
            RIGHT,
            &base
                .with_align(TextAlign::Right)
                .with_color(Color::srgb(0x8B949E)),
            &[],
        )?,
    ])
}
