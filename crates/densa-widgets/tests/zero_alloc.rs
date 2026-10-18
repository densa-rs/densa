//! Proves the standard widgets paint and follow the pointer and caret keys without touching
//! the heap once laid out (typing re-lays text out, which may allocate).

use std::hint::black_box;

use densa_alloc::{is_active, measure, CountingAllocator};
use densa_core::{Point, Size};
use densa_draw::{Canvas, InstanceBuffer};
use densa_paragraph::TextSystem;
use densa_text::{AtlasConfig, GlyphCache};
use densa_ui::{
    Align, Key, KeyEvent, Length, NamedKey, PointerButton, PointerEvent, PointerKind, ScrollDelta,
    Style, Ui,
};
use densa_widgets::{Button, Label, Panel, ScrollView, Splitter, TextInput};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator::system();

fn interface() -> (Ui, densa_ui::WidgetId) {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(1000.0, 700.0));
    let root = ui.root();
    let bar = Style::row()
        .gap(6.0)
        .padding(8.0)
        .align_items(Align::Center);
    let bar = ui.add(root, Panel::new(), bar).unwrap();
    let input = ui
        .add(bar, TextInput::new("filter text"), Style::row())
        .unwrap();
    for label in ["Add", "Clear", "Run"] {
        ui.add(bar, Button::new(label), Style::row()).unwrap();
    }
    let splitter = Splitter::row();
    let style = splitter.style();
    let body = ui.add(root, splitter, style).unwrap();
    let left = Style::column().width(Length::Px(300.0)).shrink(0.0);
    let left = ui.add(body, Panel::new().with_border(), left).unwrap();
    let list = ui
        .add(left, ScrollView::new(), ScrollView::style())
        .unwrap();
    for i in 0..200 {
        let row = Style::row().padding(4.0).shrink(0.0);
        ui.add(
            list,
            Label::new(format!("row {i:>4}  {:>9.2}", i as f64 * 1.5)),
            row,
        )
        .unwrap();
    }
    let right = Style::column().grow(1.0).padding(12.0);
    let right = ui.add(body, Panel::new(), right).unwrap();
    let text = "A paragraph that wraps at the pane's width, painted every frame.";
    ui.add(right, Label::new(text).wrapping(), Style::row())
        .unwrap();
    (ui, input)
}

#[test]
fn widget_frames_pointer_and_caret_moves_perform_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );
    let (mut ui, input) = interface();
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(65_536);
    {
        let mut canvas = Canvas::begin(&mut cache, &mut out);
        ui.paint(&mut canvas);
    }
    ui.set_focus(Some(input));
    let left = KeyEvent::down(Key::Named(NamedKey::ArrowLeft));
    let right = KeyEvent::down(Key::Named(NamedKey::ArrowRight));
    let wheel = PointerKind::Scroll(ScrollDelta::Lines(0.0, 1.0));
    let down = PointerKind::Down(PointerButton::Primary);
    let up = PointerKind::Up(PointerButton::Primary);
    // One round of everything first: glyphs into the cache, scroll state settled. Every row
    // is scrolled into view once, so its glyphs are rasterized before measuring.
    for kind in [PointerKind::Move, wheel, down, up] {
        ui.pointer(PointerEvent::new(kind, Point::new(100.0, 200.0)));
    }
    let back = PointerKind::Scroll(ScrollDelta::Lines(0.0, -1_000.0));
    for scroll in std::iter::repeat_n(wheel, 200).chain([back]) {
        ui.pointer(PointerEvent::new(scroll, Point::new(100.0, 300.0)));
        let mut canvas = Canvas::begin(&mut cache, &mut out);
        ui.paint(&mut canvas);
    }
    ui.set_focus(Some(input));

    let ((), stats) = measure(|| {
        for frame in 0..300u16 {
            let x = f32::from(frame * 3 % 1000);
            let y = f32::from(frame * 7 % 700);
            ui.pointer(PointerEvent::new(PointerKind::Move, Point::new(x, y)));
            if frame % 30 == 0 {
                ui.pointer(PointerEvent::new(wheel, Point::new(100.0, 300.0)));
                ui.key(&left);
                ui.key(&right);
            }
            black_box(ui.cursor());
            let mut canvas = Canvas::begin(&mut cache, &mut out);
            ui.paint(&mut canvas);
            black_box(out.as_slice());
        }
    });
    assert!(stats.is_zero(), "widgets allocated: {stats:?}");
    assert_eq!(out.dropped(), 0);
}
