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
use densa_widgets::{
    Button, DockArea, DockAt, DockSide, Label, Panel, ScrollView, Splitter, TextInput,
};

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

#[test]
fn dock_frames_and_tab_drags_perform_zero_heap_operations() {
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );
    let (mut ui, dock) = workspace();
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(65_536);
    let mut frame = |ui: &mut Ui| {
        black_box(ui.cursor());
        let mut canvas = Canvas::begin(&mut cache, &mut out);
        ui.paint(&mut canvas);
    };
    warm_up(&mut ui, &mut frame);
    let quotes = shown_tab(&ui, dock);

    let ((), stats) = measure(|| {
        for step in 0..100u16 {
            send(
                &mut ui,
                PointerKind::Move,
                f32::from(step * 9 % 1000),
                f32::from(step * 7 % 700),
            );
            frame(&mut ui);
        }
        // The shown tab dragged around: the drop preview follows, nothing moves.
        send(&mut ui, DOWN, quotes.0, quotes.1);
        for step in 0..100u16 {
            send(
                &mut ui,
                PointerKind::Move,
                f32::from(step * 9 % 1000),
                f32::from(step * 5 % 700),
            );
            frame(&mut ui);
        }
        ui.cancel_pointer();
        frame(&mut ui);
    });
    assert!(stats.is_zero(), "the dock allocated: {stats:?}");
    black_box(dock);
}

/// Dragging a divider resizes panels, so every move lays them out again, and layout (taffy's
/// flexbox) allocates. The dock itself must add nothing: a divider move costs what laying
/// out the same panels again costs.
#[test]
fn a_divider_move_costs_no_more_than_laying_its_panels_out() {
    assert!(is_active(), "counting allocator not installed");
    let (mut ui, dock) = workspace();
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(65_536);
    let mut frame = |ui: &mut Ui| {
        let mut canvas = Canvas::begin(&mut cache, &mut out);
        ui.paint(&mut canvas);
    };
    warm_up(&mut ui, &mut frame);
    let styles: Vec<_> = ui
        .children(dock)
        .iter()
        .map(|&id| (id, ui.style(id).unwrap().clone()))
        .collect();

    let ((), relayout) = measure(|| {
        for _ in 0..100 {
            for (id, style) in &styles {
                ui.set_style(*id, style.clone());
            }
            frame(&mut ui);
        }
    });
    let ((), dragged) = measure(|| {
        send(&mut ui, DOWN, 500.0, 350.0);
        for step in 0..100u16 {
            send(
                &mut ui,
                PointerKind::Move,
                300.0 + f32::from(step * 4),
                350.0,
            );
            frame(&mut ui);
        }
    });
    assert!(
        relayout.allocs > 0,
        "layout no longer allocates: make the dock test strict"
    );
    assert!(
        dragged.allocs <= relayout.allocs,
        "dragging a divider allocated {dragged:?}, laying out the panels {relayout:?}"
    );
}

/// A divider held against its limit does not move, so its moves change no style and lay
/// nothing out: they allocate nothing at all.
#[test]
fn a_divider_held_at_its_limit_performs_zero_heap_operations() {
    assert!(is_active(), "counting allocator not installed");
    let (mut ui, _) = workspace();
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(65_536);
    let mut frame = |ui: &mut Ui| {
        let mut canvas = Canvas::begin(&mut cache, &mut out);
        ui.paint(&mut canvas);
    };
    warm_up(&mut ui, &mut frame);
    send(&mut ui, DOWN, 500.0, 350.0);
    send(&mut ui, PointerKind::Move, 0.0, 350.0);
    frame(&mut ui);

    let ((), stats) = measure(|| {
        for step in 0..100u16 {
            send(&mut ui, PointerKind::Move, f32::from(step % 20), 350.0);
            frame(&mut ui);
        }
    });
    assert!(
        stats.is_zero(),
        "a divider at its limit allocated: {stats:?}"
    );
}

const DOWN: PointerKind = PointerKind::Down(PointerButton::Primary);

fn send(ui: &mut Ui, kind: PointerKind, x: f32, y: f32) {
    ui.pointer(PointerEvent::new(kind, Point::new(x, y)));
}

/// Four panels in three groups, laid out.
fn workspace() -> (Ui, densa_ui::WidgetId) {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(1000.0, 700.0));
    let dock = ui
        .add(ui.root(), DockArea::new(), DockArea::style())
        .unwrap();
    let add = |ui: &mut Ui, title: &str, at| {
        DockArea::add_panel(ui, dock, Panel::new(), title, at).unwrap()
    };
    let quotes = add(&mut ui, "Quotes", DockAt::First);
    add(&mut ui, "Trades", DockAt::With(quotes));
    let chart = add(&mut ui, "Chart", DockAt::Beside(quotes, DockSide::Right));
    add(&mut ui, "Orders", DockAt::Beside(chart, DockSide::Bottom));
    DockArea::show_panel(&mut ui, dock, quotes).unwrap();
    ui.layout();
    (ui, dock)
}

/// The middle of the Quotes tab, the one its group shows.
fn shown_tab(ui: &Ui, dock: densa_ui::WidgetId) -> (f32, f32) {
    let area = ui.widget::<DockArea>(dock).unwrap();
    let quotes = area.panels().next().unwrap();
    assert!(area.is_shown(quotes));
    let r = area
        .tab_rect(quotes, ui.rect(dock).unwrap(), ui.scale())
        .unwrap();
    ((r.left() + r.right()) / 2.0, (r.top() + r.bottom()) / 2.0)
}

/// One round of hovering, a divider drag and a tab drag, so glyphs are cached and buffers
/// (the action queue among them) have their size.
fn warm_up(ui: &mut Ui, frame: &mut impl FnMut(&mut Ui)) {
    let up = PointerKind::Up(PointerButton::Primary);
    for (kind, x) in [
        (PointerKind::Move, 300.0),
        (DOWN, 500.0),
        (PointerKind::Move, 450.0),
        (up, 450.0),
    ] {
        send(ui, kind, x, 350.0);
        frame(ui);
    }
    let dock = ui.children(ui.root())[0];
    let (x, y) = shown_tab(ui, dock);
    send(ui, DOWN, x, y);
    send(ui, PointerKind::Move, 900.0, 300.0);
    frame(ui);
    ui.cancel_pointer();
    frame(ui);
    ui.actions().for_each(drop);
}
