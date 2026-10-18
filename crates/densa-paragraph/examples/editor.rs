//! An editable text field: type, select with the mouse or shift + arrows, delete with
//! Backspace/Delete (ctrl: by word), Enter for a new line, ctrl+A to select all, and compose
//! with the operating system's input methods (IME). Resize the window to re-wrap.
//!
//! Run: `cargo run --release -p densa-paragraph --example editor [-- --seconds N]`
//! (`--demo` starts with typed text and an input-method preedit, for screenshots).

use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use densa_paragraph::{Motion, ParagraphStyle, TextEditor, TextSelection, TextSystem};
use densa_render::{Canvas, Color, GpuGrid, InstanceBuffer, Palette, Point, Rect};
use densa_text::{AtlasConfig, GlyphCache};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

const MARGIN: f32 = 24.0;
const MAX_INSTANCES: usize = 16_384;
const CARET_WIDTH: f32 = 2.0;
const UNDERLINE: f32 = 2.0;
/// The caret is shown and hidden for this long each; typing restarts the cycle.
const BLINK: Duration = Duration::from_millis(530);
/// Clicks closer than this (in time and space) count as a double or triple click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const DOUBLE_CLICK_SLOP: f32 = 4.0;

const TEXT: &str = "Type here. Select with the mouse or shift + arrows; Backspace and Delete \
                    remove a character, with ctrl a word. Input methods compose in place.\n\n\
                    Resize the window: the text re-wraps without shaping again.";

fn main() -> Result<(), Box<dyn Error>> {
    let seconds = std::env::args()
        .skip_while(|a| a != "--seconds")
        .nth(1)
        .map(|s| s.parse::<u64>())
        .transpose()?;
    let demo = std::env::args().any(|a| a == "--demo");
    let event_loop = EventLoop::new()?;
    let mut app = App {
        deadline: seconds.map(|s| Instant::now() + Duration::from_secs(s)),
        demo,
        state: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

struct App {
    deadline: Option<Instant>,
    demo: bool,
    state: Option<State>,
    error: Option<Box<dyn Error>>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match State::new(event_loop, self.demo) {
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
            WindowEvent::KeyboardInput { event, .. } => state.key(&event),
            WindowEvent::Ime(ime) => state.ime(ime),
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
    system: TextSystem,
    editor: TextEditor,
    background: Color,
    /// When the caret blink cycle last restarted (at the last edit or caret move).
    blink_from: Instant,
    dragging: bool,
    pointer: Point,
    modifiers: ModifiersState,
    /// When and where the last click was, and how many clicks in a row it made.
    last_click: Option<(Instant, Point, u8)>,
}

impl State {
    fn new(event_loop: &ActiveEventLoop, demo: bool) -> Result<Self, Box<dyn Error>> {
        let attributes = Window::default_attributes()
            .with_title("densa editor")
            .with_inner_size(PhysicalSize::new(720, 360));
        let window = Arc::new(event_loop.create_window(attributes)?);
        window.set_ime_allowed(true);
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
        let mut system = TextSystem::new()?;
        let style = ParagraphStyle::new(16.0)
            .with_color(Color::srgb(0xC9D1D9))
            .with_max_width(text_width(size.width));
        let mut editor = TextEditor::new(&mut system, TEXT, style)?;
        if demo {
            play_demo(&mut system, &mut editor)?;
        }
        let state = Self {
            window,
            surface,
            device,
            queue,
            config,
            view_format,
            gpu,
            cache,
            instances: InstanceBuffer::with_capacity(MAX_INSTANCES),
            system,
            editor,
            background: Palette::new().background,
            blink_from: Instant::now(),
            dragging: false,
            pointer: Point::ORIGIN,
            modifiers: ModifiersState::empty(),
            last_click: None,
        };
        state.report_ime_area();
        Ok(state)
    }

    fn resize(&mut self, size: PhysicalSize<u32>) -> Result<(), Box<dyn Error>> {
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.editor.set_width(text_width(size.width))?; // re-break only: no shaping
        self.report_ime_area();
        Ok(())
    }

    fn draw(&mut self) -> Result<(), Box<dyn Error>> {
        let origin = Point::new(MARGIN, MARGIN);
        let editor = &self.editor;
        let mut canvas = Canvas::begin(&mut self.cache, &mut self.instances);
        let selection = editor.selection();
        selection.for_each_rect(editor.paragraph(), |rect| {
            canvas.fill_rect(
                rect.translate(origin),
                Color::srgb(0x1F6FEB).with_alpha(0.45),
            );
        });
        for run in editor.paragraph().runs() {
            canvas.draw_run(run, origin);
        }
        editor.for_each_composition_rect(|rect| {
            let underline = Rect::from_edges(
                rect.left(),
                rect.bottom() - UNDERLINE,
                rect.right(),
                rect.bottom(),
            );
            canvas.fill_rect(underline.translate(origin), Color::srgb(0xC9D1D9));
        });
        let blink_on =
            (self.blink_from.elapsed().as_millis() / BLINK.as_millis()).is_multiple_of(2);
        if editor.shows_caret() && blink_on {
            let caret = selection.caret_rect(editor.paragraph(), CARET_WIDTH);
            canvas.fill_rect(caret.translate(origin), Color::srgb(0x58A6FF));
        }
        self.present()
    }

    fn present(&mut self) -> Result<(), Box<dyn Error>> {
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

    /// After an edit or a caret move: show the caret at once and tell the input method where
    /// the text is, so its candidate window opens next to it.
    fn changed(&mut self) {
        self.blink_from = Instant::now();
        self.report_ime_area();
    }

    fn report_ime_area(&self) {
        let area = self.editor.ime_area().translate(Point::new(MARGIN, MARGIN));
        self.window.set_ime_cursor_area(
            PhysicalPosition::new(area.left(), area.top()),
            PhysicalSize::new(area.width().max(1.0), area.height()),
        );
    }
}

impl State {
    /// `point` (window coordinates) relative to the text's top-left corner.
    fn local(point: Point) -> Point {
        Point::new(point.x - MARGIN, point.y - MARGIN)
    }

    fn pointer_moved(&mut self, point: Point) {
        self.pointer = point;
        if !self.dragging {
            return;
        }
        let paragraph = self.editor.paragraph();
        let extended = self
            .editor
            .selection()
            .extend_to_point(paragraph, Self::local(point));
        self.editor.set_selection(extended);
        self.changed();
    }

    fn left_button(&mut self, pressed: bool) {
        self.dragging = pressed;
        if !pressed {
            return;
        }
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
        let paragraph = self.editor.paragraph();
        let local = Self::local(self.pointer);
        let selection = match clicks {
            2 => TextSelection::word_at_point(paragraph, local),
            3 => TextSelection::line_at_point(paragraph, local),
            _ if self.modifiers.shift_key() => {
                self.editor.selection().shift_click(paragraph, local)
            }
            _ => TextSelection::at_point(paragraph, local),
        };
        self.editor.set_selection(selection);
        self.changed();
    }

    fn key(&mut self, event: &KeyEvent) -> Result<(), Box<dyn Error>> {
        if event.state != ElementState::Pressed {
            return Ok(());
        }
        let ctrl = self.modifiers.control_key() || self.modifiers.super_key();
        let shift = self.modifiers.shift_key();
        let system = &mut self.system;
        match &event.logical_key {
            Key::Named(NamedKey::Backspace) => {
                let motion = if ctrl {
                    Motion::PreviousWord
                } else {
                    Motion::Previous
                };
                self.editor.delete(system, motion)?;
            }
            Key::Named(NamedKey::Delete) => {
                let motion = if ctrl { Motion::NextWord } else { Motion::Next };
                self.editor.delete(system, motion)?;
            }
            Key::Named(NamedKey::Enter) => self.editor.insert(system, "\n")?,
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("a") => {
                let all = TextSelection::select_all(self.editor.paragraph());
                self.editor.set_selection(all);
            }
            key => {
                if let Some(motion) = navigation(key, ctrl) {
                    self.editor.move_caret(motion, shift);
                } else if let Some(text) = typed_text(event, ctrl) {
                    self.editor.insert(system, text)?;
                } else {
                    return Ok(());
                }
            }
        }
        self.changed();
        Ok(())
    }

    fn ime(&mut self, ime: Ime) -> Result<(), Box<dyn Error>> {
        let system = &mut self.system;
        match ime {
            Ime::Preedit(text, cursor) => {
                let cursor = cursor.map(|(start, end)| start..end);
                self.editor.set_composition(system, &text, cursor)?;
            }
            Ime::Commit(text) => self.editor.insert(system, &text)?,
            Ime::Disabled => self.editor.set_composition(system, "", None)?,
            Ime::Enabled => {}
        }
        self.changed();
        Ok(())
    }
}

/// What `--demo` shows: text typed at the end, and an input method composing after it.
fn play_demo(system: &mut TextSystem, editor: &mut TextEditor) -> Result<(), Box<dyn Error>> {
    editor.insert(system, " Typed and ")?;
    editor.set_composition(system, "composing", Some(9..9))?;
    Ok(())
}

/// The caret motion a navigation key makes (`ctrl`: by word, or to the text's ends).
fn navigation(key: &Key, ctrl: bool) -> Option<Motion> {
    Some(match key {
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
        _ => return None,
    })
}

/// The text a key press types: none for shortcuts and control characters.
fn typed_text(event: &KeyEvent, ctrl: bool) -> Option<&str> {
    let text = event.text.as_deref()?;
    (!ctrl && !text.chars().any(char::is_control)).then_some(text)
}

/// The width text wraps at inside a window `window_width` pixels wide.
fn text_width(window_width: u32) -> Option<f32> {
    Some((window_width as f32 - 2.0 * MARGIN).max(1.0))
}
