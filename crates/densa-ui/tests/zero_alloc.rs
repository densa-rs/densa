//! Proves painting frames and delivering pointer and key events never touch the heap once
//! the interface is laid out.

use std::hint::black_box;

use densa_alloc::{is_active, measure, CountingAllocator};
use densa_core::{Color, Point, Size};
use densa_draw::{Canvas, InstanceBuffer};
use densa_paragraph::TextSystem;
use densa_text::{AtlasConfig, GlyphCache};
use densa_ui::{
    Cursor, Event, EventCx, Handled, Key, KeyEvent, NamedKey, Overflow, PaintCx, PointerButton,
    PointerEvent, PointerKind, ScrollDelta, Style, Ui, Widget,
};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

/// A cell that tracks hover, takes focus, handles presses and the wheel, and paints.
struct Cell {
    presses: u32,
}

impl Widget for Cell {
    fn paint(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        let color = if cx.is_hovered() {
            cx.theme().hover
        } else {
            Color::srgb(0x1A2540)
        };
        canvas.fill_rect(cx.rect(), color);
    }

    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        match event {
            Event::Pointer(p) => match p.kind {
                PointerKind::Enter => {
                    cx.set_cursor(Some(Cursor::Pointer));
                    cx.request_paint();
                    Handled::Yes
                }
                PointerKind::Down(_) => {
                    self.presses += 1;
                    Handled::Yes
                }
                PointerKind::Scroll(ScrollDelta::Pixels(_, dy)) => {
                    let at = cx.scroll();
                    cx.scroll_to(Point::new(at.x, at.y + dy));
                    Handled::No
                }
                _ => Handled::No,
            },
            Event::Key(_) => Handled::No,
            _ => Handled::No,
        }
    }

    fn accepts_focus(&self) -> bool {
        true
    }
}

/// A scrolling column of 200 rows of 5 cells.
fn interface() -> Ui {
    let mut ui = Ui::new(TextSystem::new().unwrap());
    ui.resize(Size::new(800.0, 600.0));
    let list = Style::column().grow(1.0).overflow(Overflow::Scroll);
    let list = ui.add(ui.root(), Cell { presses: 0 }, list).unwrap();
    for _ in 0..200 {
        let row = Style::row()
            .height(densa_ui::Length::Px(24.0))
            .shrink(0.0)
            .gap(2.0);
        let row = ui.add(list, Cell { presses: 0 }, row).unwrap();
        for _ in 0..5 {
            ui.add(row, Cell { presses: 0 }, Style::row().grow(1.0))
                .unwrap();
        }
    }
    ui
}

#[test]
fn frames_and_events_perform_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );
    let mut ui = interface();
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(8_192);
    // Startup: layout, and one frame and event of each kind.
    {
        let mut canvas = Canvas::begin(&mut cache, &mut out);
        ui.paint(&mut canvas);
    }
    let down = PointerKind::Down(PointerButton::Primary);
    let up = PointerKind::Up(PointerButton::Primary);
    let wheel = PointerKind::Scroll(ScrollDelta::Pixels(0.0, 3.0));
    let tab = KeyEvent::down(Key::Named(NamedKey::Tab));
    let typed = KeyEvent::down(Key::Character("a")).with_text("a");
    ui.pointer(PointerEvent::new(down, Point::new(10.0, 10.0)));
    ui.pointer(PointerEvent::new(up, Point::new(10.0, 10.0)));

    let ((), stats) = measure(|| {
        for frame in 0..500u16 {
            let x = f32::from(frame % 800);
            let y = f32::from(frame % 600);
            ui.pointer(PointerEvent::new(PointerKind::Move, Point::new(x, y)));
            if frame % 50 == 0 {
                ui.pointer(PointerEvent::new(down, Point::new(x, y)));
                ui.pointer(PointerEvent::new(up, Point::new(x, y)));
                ui.pointer(PointerEvent::new(wheel, Point::new(x, y)));
                ui.key(&tab);
                ui.key(&typed);
            }
            black_box(ui.cursor());
            let mut canvas = Canvas::begin(&mut cache, &mut out);
            ui.paint(&mut canvas);
            black_box(out.as_slice());
        }
    });
    assert!(stats.is_zero(), "frames or events allocated: {stats:?}");
    assert_eq!(out.dropped(), 0);
}
