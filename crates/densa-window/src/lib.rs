//! Runs a densa interface in a native window.
//!
//! [`run`] opens a window, creates a [`Ui`] for it and hands it to an [`App`]
//! to fill with widgets. From then on it translates the window system's input (winit) into
//! `densa_ui` events, passes the actions widgets report to the app, and paints the interface
//! with `densa_render` (wgpu) in one draw call per frame, presented with V-Sync.
//!
//! Frames are painted when something changed, when another thread asks for one through a
//! [`Waker`] (new data arrived), or continuously while the app asks for it (animation).
//!
//! The [`Ui`]'s text uses `densa_paragraph::TextSystem::new`: the bundled font, and the
//! system's fonts for characters it lacks (Korean, Japanese, Chinese, ...). Turning off the
//! default `system-fonts` feature leaves the bundled font alone.
//!
//! ```no_run
//! use std::error::Error;
//! use densa_ui::{Ui, Style};
//! use densa_window::{App, Host, WindowOptions};
//!
//! struct Hello;
//!
//! impl App for Hello {
//!     fn build(&mut self, ui: &mut Ui, host: &mut Host<'_>) -> Result<(), Box<dyn Error>> {
//!         // Add widgets under ui.root() here; hand host.waker() to threads that feed them.
//!         let _ = (ui.root(), host.waker());
//!         Ok(())
//!     }
//! }
//!
//! densa_window::run(WindowOptions::new("Hello"), Hello)?;
//! # Ok::<(), Box<dyn Error>>(())
//! ```

mod gpu;
mod translate;

use std::error::Error;
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use densa_core::{Point, Size};
use densa_draw::{Canvas, InstanceBuffer};
use densa_paragraph::TextSystem;
use densa_text::{AtlasConfig, GlyphCache};
use densa_ui::{Action, Cursor, KeyEvent, PointerEvent, PointerKind, Theme, Ui};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::gpu::Gpu;

/// An application: it builds its interface once, then reacts to what widgets report and
/// updates live content before each frame. Every call gets the [`Host`]: the window it runs
/// in.
pub trait App: 'static {
    /// Fills the interface: called once, when the window and its [`Ui`] exist.
    ///
    /// # Errors
    /// Any error stops [`run`], which returns it.
    fn build(&mut self, ui: &mut Ui, host: &mut Host<'_>) -> Result<(), Box<dyn Error>>;

    /// Handles an action a widget reported (a click, an edit), right after the input that
    /// caused it.
    fn action(&mut self, ui: &mut Ui, host: &mut Host<'_>, action: Action) {
        let _ = (ui, host, action);
    }

    /// Updates live content before a frame is painted. Return `true` to keep painting frames
    /// continuously (animation), `false` to paint only when something changes or a
    /// [`Waker`] asks for a frame.
    fn frame(&mut self, ui: &mut Ui, host: &mut Host<'_>) -> bool {
        let _ = (ui, host);
        false
    }
}

/// The window an [`App`] runs in.
#[derive(Debug)]
pub struct Host<'a> {
    window: &'a Window,
    proxy: &'a EventLoopProxy<Wake>,
    close: &'a mut bool,
}

impl Host<'_> {
    /// A handle any thread can use to ask for a frame: a feed thread calls
    /// [`Waker::wake`] after publishing new data.
    pub fn waker(&self) -> Waker {
        Waker(self.proxy.clone())
    }

    /// Sets the window's title.
    pub fn set_title(&self, title: &str) {
        self.window.set_title(title);
    }

    /// Closes the window once the current call returns, which ends [`run`].
    pub fn close(&mut self) {
        *self.close = true;
    }
}

/// Asks the window's thread for a frame, from any thread.
#[derive(Debug, Clone)]
pub struct Waker(EventLoopProxy<Wake>);

impl Waker {
    /// Asks for a frame: [`App::frame`] runs and the interface is painted soon. `false` if
    /// the window is gone.
    pub fn wake(&self) -> bool {
        self.0.send_event(Wake).is_ok()
    }
}

/// The event a [`Waker`] sends.
#[derive(Debug, Clone, Copy)]
struct Wake;

/// The window [`run`] opens.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WindowOptions {
    /// The window's title.
    pub title: String,
    /// The window's inner size, in logical pixels.
    pub size: Size,
    /// Colors and sizes of the widgets.
    pub theme: Theme,
    /// Rectangles and glyphs a frame can hold; more are dropped (and counted).
    pub instances: usize,
}

impl WindowOptions {
    /// A 1280x800 window titled `title`, with the default theme.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            size: Size::new(1280.0, 800.0),
            theme: Theme::default(),
            instances: 65_536,
        }
    }

    /// The options with an inner size in logical pixels.
    #[must_use]
    pub fn with_size(mut self, width: f32, height: f32) -> Self {
        self.size = Size::new(width, height);
        self
    }

    /// The options with `theme`.
    #[must_use]
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// The options holding up to `instances` rectangles and glyphs a frame.
    #[must_use]
    pub fn with_instances(mut self, instances: usize) -> Self {
        self.instances = instances;
        self
    }
}

/// Why [`run`] stopped with an error.
#[derive(Debug)]
#[non_exhaustive]
pub enum WindowError {
    /// The window system's event loop or window failed.
    Window(String),
    /// The GPU could not be set up or drawn to.
    Gpu(String),
    /// The fonts could not be loaded.
    Text(String),
    /// The application's [`App::build`] failed.
    App(Box<dyn Error>),
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Window(message) => write!(f, "window system: {message}"),
            Self::Gpu(message) => write!(f, "GPU: {message}"),
            Self::Text(message) => write!(f, "fonts: {message}"),
            Self::App(error) => write!(f, "application: {error}"),
        }
    }
}

impl Error for WindowError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::App(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

/// Opens a window for `app` and runs until it is closed.
///
/// # Errors
/// [`WindowError`] if the window, the GPU or the fonts cannot be set up, a frame cannot be
/// drawn, or [`App::build`] fails.
pub fn run(options: WindowOptions, app: impl App) -> Result<(), WindowError> {
    let event_loop = EventLoop::<Wake>::with_user_event()
        .build()
        .map_err(|e| WindowError::Window(e.to_string()))?;
    let mut shell = Shell {
        app,
        options,
        proxy: event_loop.create_proxy(),
        session: None,
        error: None,
        close: false,
    };
    event_loop
        .run_app(&mut shell)
        .map_err(|e| WindowError::Window(e.to_string()))?;
    shell.error.map_or(Ok(()), Err)
}

struct Shell<A: App> {
    app: A,
    options: WindowOptions,
    proxy: EventLoopProxy<Wake>,
    session: Option<Session>,
    error: Option<WindowError>,
    /// The app asked to close the window while building its interface.
    close: bool,
}

/// The window, its GPU surface and its interface.
struct Session {
    window: Arc<Window>,
    gpu: Gpu,
    ui: Ui,
    cache: GlyphCache,
    instances: InstanceBuffer,
    start: Instant,
    pointer: Point,
    modifiers: ModifiersState,
    cursor: Cursor,
    continuous: bool,
}

impl<A: App> Shell<A> {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: WindowError) {
        self.error = Some(error);
        event_loop.exit();
    }

    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<Session, WindowError> {
        let size = self.options.size;
        let attributes = Window::default_attributes()
            .with_title(self.options.title.clone())
            .with_inner_size(LogicalSize::new(size.width, size.height));
        let window = event_loop
            .create_window(attributes)
            .map_err(|e| WindowError::Window(e.to_string()))?;
        let window = Arc::new(window);
        window.set_ime_allowed(true);
        let mut cache = GlyphCache::new(AtlasConfig::DEFAULT);
        let gpu = Gpu::new(event_loop, &window, self.options.instances, &mut cache)?;
        let text = TextSystem::new().map_err(|e| WindowError::Text(e.to_string()))?;
        let mut ui = Ui::new(text);
        ui.set_theme(self.options.theme.clone());
        ui.set_scale(window.scale_factor() as f32);
        let inner = window.inner_size();
        ui.resize(Size::new(inner.width as f32, inner.height as f32));
        let mut host = Host {
            window: &window,
            proxy: &self.proxy,
            close: &mut self.close,
        };
        self.app
            .build(&mut ui, &mut host)
            .map_err(WindowError::App)?;
        Ok(Session {
            window,
            gpu,
            ui,
            cache,
            instances: InstanceBuffer::with_capacity(self.options.instances),
            start: Instant::now(),
            pointer: Point::ORIGIN,
            modifiers: ModifiersState::empty(),
            cursor: Cursor::Default,
            continuous: false,
        })
    }
}

impl<A: App> ApplicationHandler<Wake> for Shell<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.session.is_some() {
            return;
        }
        match self.open(event_loop) {
            Ok(session) => {
                session.window.request_redraw();
                self.session = Some(session);
                if self.close {
                    event_loop.exit();
                }
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let window = Arc::clone(&session.window);
        let mut close = false;
        let mut failed = None;
        {
            let mut host = Host {
                window: &window,
                proxy: &self.proxy,
                close: &mut close,
            };
            match event {
                WindowEvent::CloseRequested => host.close(),
                WindowEvent::RedrawRequested => {
                    session.continuous = self.app.frame(&mut session.ui, &mut host);
                    failed = session.paint().err();
                }
                other => session.input(&other),
            }
            if failed.is_none() {
                while let Some(action) = session.ui.next_action() {
                    self.app.action(&mut session.ui, &mut host, action);
                }
                session.reflect();
            }
        }
        if let Some(error) = failed {
            self.fail(event_loop, error);
        } else if close {
            event_loop.exit();
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, _: Wake) {
        if let Some(session) = &self.session {
            session.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(session) = &self.session {
            if session.continuous {
                session.window.request_redraw();
            }
        }
    }
}

impl Session {
    /// Hands a window event to the interface.
    fn input(&mut self, event: &WindowEvent) {
        let modifiers = translate::modifiers(self.modifiers);
        let at = |kind: PointerKind, position: Point, start: Instant| {
            PointerEvent::new(kind, position)
                .with_modifiers(modifiers)
                .at(start.elapsed())
        };
        match event {
            WindowEvent::Resized(size) => {
                self.gpu.resize(size.width, size.height);
                self.ui
                    .resize(Size::new(size.width as f32, size.height as f32));
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.ui.set_scale(*scale_factor as f32);
            }
            WindowEvent::ModifiersChanged(state) => self.modifiers = state.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = Point::new(position.x as f32, position.y as f32);
                self.ui
                    .pointer(at(PointerKind::Move, self.pointer, self.start));
            }
            WindowEvent::CursorLeft { .. } => self.ui.pointer_left(self.pointer),
            // Alt+Tab mid-drag: no release will come, and modifiers may be released elsewhere.
            WindowEvent::Focused(false) => {
                self.ui.cancel_pointer();
                self.modifiers = ModifiersState::empty();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let button = translate::button(*button);
                let kind = match state {
                    ElementState::Pressed => PointerKind::Down(button),
                    ElementState::Released => PointerKind::Up(button),
                };
                self.ui.pointer(at(kind, self.pointer, self.start));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let kind = PointerKind::Scroll(translate::scroll(*delta));
                self.ui.pointer(at(kind, self.pointer, self.start));
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                let Some(key) = translate::key(&event.logical_key) else {
                    return;
                };
                let mut key = if event.state == ElementState::Pressed {
                    KeyEvent::down(key)
                } else {
                    KeyEvent::up(key)
                }
                .with_modifiers(modifiers);
                if let Some(text) = translate::text(event.text.as_deref()) {
                    key = key.with_text(text);
                }
                if event.repeat {
                    key = key.repeated();
                }
                self.ui.key(&key);
            }
            WindowEvent::Ime(ime) => {
                self.ui.ime(&translate::ime(ime));
            }
            _ => {}
        }
    }

    /// Paints the interface and presents it.
    fn paint(&mut self) -> Result<(), WindowError> {
        {
            let mut canvas = Canvas::begin(&mut self.cache, &mut self.instances);
            self.ui.paint(&mut canvas);
        }
        let background = self.ui.theme().background;
        self.gpu.draw(
            &self.window,
            &mut self.cache,
            self.instances.as_slice(),
            background,
        )?;
        Ok(())
    }

    /// Shows the interface's state in the window: cursor, input method area, repaint.
    fn reflect(&mut self) {
        let cursor = self.ui.cursor();
        if cursor != self.cursor {
            self.cursor = cursor;
            self.window.set_cursor(translate::cursor(cursor));
        }
        if let Some(area) = self.ui.ime_area() {
            self.window.set_ime_cursor_area(
                PhysicalPosition::new(area.left(), area.top()),
                PhysicalSize::new(area.width().max(1.0), area.height().max(1.0)),
            );
        }
        if self.ui.needs_paint() {
            self.window.request_redraw();
        }
    }
}
