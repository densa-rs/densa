//! A test widget that logs what it receives.

#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use densa_core::{Color, Point, Size};
use densa_draw::Canvas;
use densa_paragraph::TextSystem;
use densa_ui::{
    Constraints, Event, EventCx, Handled, Key, LayoutCx, PaintCx, PointerButton, PointerEvent,
    PointerKind, Ui, Widget,
};

/// What every probe saw, in order, as `"name:what"`.
pub type Log = Rc<RefCell<Vec<String>>>;

pub fn log() -> Log {
    Rc::new(RefCell::new(Vec::new()))
}

pub fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

/// A widget that logs events, optionally handles them, takes focus, has an intrinsic size
/// and paints its rectangle in its own color.
pub struct Probe {
    pub name: &'static str,
    pub log: Log,
    pub handles_pointer: bool,
    pub handles_keys: bool,
    pub focusable: bool,
    pub intrinsic: Size,
    pub laid_out: Option<Size>,
    pub color: Color,
    pub measured: u32,
    pub painted: u32,
}

impl Probe {
    pub fn new(name: &'static str, log: &Log) -> Self {
        Self {
            name,
            log: Rc::clone(log),
            handles_pointer: false,
            handles_keys: false,
            focusable: false,
            intrinsic: Size::ZERO,
            laid_out: None,
            color: Color::WHITE,
            measured: 0,
            painted: 0,
        }
    }

    pub fn handling(mut self) -> Self {
        self.handles_pointer = true;
        self.handles_keys = true;
        self
    }

    pub fn focusable(mut self) -> Self {
        self.focusable = true;
        self
    }

    pub fn sized(mut self, width: f32, height: f32) -> Self {
        self.intrinsic = Size::new(width, height);
        self
    }

    pub fn colored(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    fn note(&self, what: String) {
        self.log.borrow_mut().push(format!("{}:{what}", self.name));
    }
}

impl Widget for Probe {
    fn measure(&mut self, _: &mut LayoutCx<'_>, _: Constraints) -> Size {
        self.measured += 1;
        self.intrinsic
    }

    fn layout(&mut self, _: &mut LayoutCx<'_>, size: Size) {
        self.laid_out = Some(size);
    }

    fn paint(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        self.painted += 1;
        canvas.fill_rect(cx.rect(), self.color);
    }

    fn event(&mut self, _: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        match event {
            Event::Pointer(p) => {
                let what = match p.kind {
                    PointerKind::Down(_) => format!("down{}", p.clicks),
                    PointerKind::Up(_) => "up".to_owned(),
                    PointerKind::Move => "move".to_owned(),
                    PointerKind::Scroll(_) => "scroll".to_owned(),
                    PointerKind::Enter => "enter".to_owned(),
                    PointerKind::Leave => "leave".to_owned(),
                    _ => "other".to_owned(),
                };
                self.note(what);
                self.handles_pointer.into()
            }
            Event::Key(key) => {
                let name = match key.key {
                    Key::Character(c) => c.to_owned(),
                    Key::Named(named) => format!("{named:?}"),
                    _ => "other".to_owned(),
                };
                self.note(format!("key {name}"));
                self.handles_keys.into()
            }
            Event::Ime(_) => {
                self.note("ime".to_owned());
                self.handles_keys.into()
            }
            Event::FocusGained => {
                self.note("focus+".to_owned());
                Handled::Yes
            }
            Event::FocusLost => {
                self.note("focus-".to_owned());
                Handled::Yes
            }
            _ => Handled::No,
        }
    }

    fn accepts_focus(&self) -> bool {
        self.focusable
    }
}

pub fn ui(width: f32, height: f32) -> Ui {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(width, height));
    ui
}

pub fn press(ui: &mut Ui, x: f32, y: f32) -> Handled {
    let down = PointerKind::Down(PointerButton::Primary);
    ui.pointer(PointerEvent::new(down, Point::new(x, y)))
}

pub fn release(ui: &mut Ui, x: f32, y: f32) -> Handled {
    let up = PointerKind::Up(PointerButton::Primary);
    ui.pointer(PointerEvent::new(up, Point::new(x, y)))
}

pub fn move_to(ui: &mut Ui, x: f32, y: f32) -> Handled {
    ui.pointer(PointerEvent::new(PointerKind::Move, Point::new(x, y)))
}
