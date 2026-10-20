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

/// A large model formatting numbers into the cell buffer.
struct Ticks(Vec<densa_grid::Column>);

impl densa_grid::GridModel for Ticks {
    fn columns(&self) -> &[densa_grid::Column] {
        &self.0
    }

    fn rows(&self) -> usize {
        100_000
    }

    fn cell(
        &self,
        row: usize,
        column: usize,
        text: &mut densa_grid::CellText,
        _: &densa_grid::Palette,
    ) -> densa_grid::CellStyle {
        use std::fmt::Write as _;
        let _ = match column {
            0 => write!(text, "S{row:05}"),
            _ => write!(
                text,
                "{:.2}",
                (row * 37 % 1000) as f64 / 10.0 + column as f64
            ),
        };
        densa_grid::CellStyle::new(densa_core::Color::WHITE)
    }
}

#[test]
fn grid_frames_scrolling_and_selection_perform_zero_heap_operations() {
    assert!(is_active(), "counting allocator not installed");
    use densa_grid::{Align, Column};
    let columns = (0..8)
        .map(|i| Column::new(if i == 0 { "Symbol" } else { "Value" }, 10, Align::Right))
        .collect();
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(600.0, 500.0));
    let grid = densa_widgets::DataGrid::new(Ticks(columns)).with_frozen_columns(1);
    let grid = ui
        .add(ui.root(), grid, densa_widgets::DataGrid::<Ticks>::style())
        .unwrap();
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(65_536);
    let mut frame = |ui: &mut Ui| {
        let mut canvas = Canvas::begin(&mut cache, &mut out);
        ui.paint(&mut canvas);
    };
    let wheel = |dx: f32, dy: f32| PointerKind::Scroll(ScrollDelta::Lines(dx, dy));
    let down = KeyEvent::down(Key::Named(NamedKey::ArrowDown));
    // Warm up: every digit and letter drawn, a click to focus and select, both scrolls.
    send(&mut ui, DOWN, 300.0, 200.0);
    send(
        &mut ui,
        PointerKind::Up(PointerButton::Primary),
        300.0,
        200.0,
    );
    for step in 0..20 {
        send(&mut ui, wheel(0.0, 7.0), 300.0, 200.0);
        send(
            &mut ui,
            wheel(if step % 2 == 0 { 3.0 } else { -3.0 }, 0.0),
            300.0,
            200.0,
        );
        ui.key(&down);
        frame(&mut ui);
    }
    let _ = actions_of(&mut ui);

    let ((), stats) = measure(|| {
        for step in 0..200u16 {
            send(&mut ui, wheel(0.0, 1.0), 300.0, 200.0);
            send(
                &mut ui,
                wheel(if step % 2 == 0 { 1.0 } else { -1.0 }, 0.0),
                300.0,
                200.0,
            );
            ui.key(&down);
            let x = f32::from(step * 3 % 600);
            send(&mut ui, PointerKind::Move, x, 10.0);
            frame(&mut ui);
            // As an application does after each event.
            black_box(actions_of(&mut ui));
        }
    });
    assert!(stats.is_zero(), "the grid allocated: {stats:?}");
    black_box(grid);
}

/// Drains the actions without keeping them (their queue keeps its capacity).
fn actions_of(ui: &mut Ui) -> usize {
    ui.actions().count()
}

/// A day of one-second prices as a line over minute candles, with volume below.
struct Day {
    line: Vec<(f64, f64)>,
    candles: Vec<(f64, densa_chart::Candle)>,
    volume: Vec<(f64, f64)>,
}

impl Day {
    fn new() -> Self {
        let line: Vec<(f64, f64)> = (0..86_400)
            .map(|s| (f64::from(s), 100.0 + (f64::from(s) * 0.001).sin()))
            .collect();
        let candles: Vec<_> = (0..1_440)
            .map(|m| {
                let open = 100.0 + (f64::from(m) * 0.06).sin();
                let close = 100.0 + (f64::from(m + 1) * 0.06).sin();
                let candle =
                    densa_chart::Candle::new(open, open.max(close), open.min(close), close);
                (60.0 * f64::from(m), candle)
            })
            .collect();
        let volume = candles
            .iter()
            .map(|(x, c)| (*x, (c.close - c.open).abs()))
            .collect();
        Self {
            line,
            candles,
            volume,
        }
    }
}

impl densa_chart::ChartModel for Day {
    fn x_extent(&self) -> Option<(f64, f64)> {
        Some((0.0, 86_399.0))
    }

    fn x_step(&self) -> Option<f64> {
        Some(1.0)
    }

    fn panes(&self) -> usize {
        2
    }

    fn y_axis(&self, pane: usize, x: &densa_chart::Axis) -> Option<densa_chart::Axis> {
        let range = densa_chart::points_within(&self.line[..], x);
        let (low, high) = if pane == 0 {
            densa_chart::y_extent(&self.line[..], range)?
        } else {
            (0.0, 0.1)
        };
        Some(densa_chart::Axis::number(low, high).padded(0.05))
    }

    fn pane_title(&self, pane: usize) -> &str {
        ["Price, 1 min", "Volume"][pane]
    }

    fn paint(
        &self,
        pane: usize,
        plot: &densa_chart::Plot,
        canvas: &mut Canvas<'_>,
        palette: &densa_chart::ChartPalette,
    ) {
        if pane == 0 {
            let style = densa_chart::CandleStyle::new(palette.up, palette.down);
            densa_chart::paint_candles(canvas, plot, &self.candles[..], 60.0, style);
            let line = densa_chart::LineStyle::new(palette.line);
            densa_chart::paint_line(canvas, plot, &self.line[..], line);
        } else {
            densa_chart::paint_bars(
                canvas,
                plot,
                &self.volume[..],
                60.0,
                densa_chart::BarStyle::new(),
                |_| palette.up,
            );
        }
    }
}

#[test]
fn chart_frames_zooming_panning_and_the_crosshair_perform_zero_heap_operations() {
    assert!(is_active(), "counting allocator not installed");
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(1_000.0, 600.0));
    let chart = densa_widgets::Chart::new(Day::new(), 6.0 * 3_600.0);
    let chart = ui
        .add(ui.root(), chart, densa_widgets::Chart::<Day>::style())
        .unwrap();
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(65_536);
    let mut frame = |ui: &mut Ui| {
        let mut canvas = Canvas::begin(&mut cache, &mut out);
        ui.paint(&mut canvas);
    };
    let wheel = |dy: f32| PointerKind::Scroll(ScrollDelta::Lines(0.0, dy));
    let left = KeyEvent::down(Key::Named(NamedKey::ArrowLeft));
    let step = |ui: &mut Ui, i: u16, frame: &mut dyn FnMut(&mut Ui)| {
        let x = f32::from(100 + i * 7 % 700);
        send(ui, PointerKind::Move, x, 200.0);
        send(
            ui,
            wheel(if i.is_multiple_of(2) { 1.0 } else { -1.0 }),
            x,
            200.0,
        );
        send(ui, DOWN, x, 300.0);
        send(ui, PointerKind::Move, x + 40.0, 300.0);
        send(ui, PointerKind::Up(PointerButton::Primary), x + 40.0, 300.0);
        ui.key(&left);
        frame(ui);
        black_box(actions_of(ui));
    };
    // Warm up: every label character drawn, the crosshair's and the titles' too.
    for i in 0..60 {
        step(&mut ui, i, &mut frame);
    }
    let ((), stats) = measure(|| {
        for i in 0..200 {
            step(&mut ui, i, &mut frame);
        }
    });
    assert!(stats.is_zero(), "the chart allocated: {stats:?}");
    black_box(chart);
}

#[test]
fn an_open_menu_follows_the_pointer_and_keys_with_zero_heap_operations() {
    use densa_widgets::{MenuBar, MenuItem};
    assert!(
        is_active(),
        "counting allocator not installed: test would be vacuous"
    );
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(1000.0, 700.0));
    let items = || {
        let mut items: Vec<MenuItem> = (0..12)
            .map(|i| MenuItem::new(format!("Command {i}"), i).with_shortcut("Ctrl+K"))
            .collect();
        items.push(MenuItem::separator());
        items.push(MenuItem::new("Wrap", 50).checked(true));
        items.push(MenuItem::submenu(
            "More",
            [MenuItem::new("One", 60), MenuItem::new("Two", 61)],
        ));
        items
    };
    let bar = MenuBar::new()
        .with_menu("File", items())
        .with_menu("Edit", items());
    let bar = ui.add(ui.root(), bar, MenuBar::style()).unwrap();
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(16_384);
    let mut frame = |ui: &mut Ui| {
        let mut canvas = Canvas::begin(&mut cache, &mut out);
        ui.paint(&mut canvas);
        black_box(out.as_slice());
    };
    frame(&mut ui);
    let title = ui.rect(bar).unwrap();
    let file = Point::new(title.left() + 10.0, title.top() + 5.0);
    let down = PointerKind::Down(PointerButton::Primary);
    ui.pointer(PointerEvent::new(down, file));
    frame(&mut ui);
    let menu = ui.popups().next().unwrap();
    let rect = ui.rect(menu).unwrap();
    // Warm up: open the submenu once (opening allocates), then leave it.
    let more = Point::new(rect.left() + 20.0, rect.bottom() - 10.0);
    ui.pointer(PointerEvent::new(PointerKind::Move, more));
    frame(&mut ui);
    let up = KeyEvent::down(Key::Named(NamedKey::ArrowUp));
    let down_key = KeyEvent::down(Key::Named(NamedKey::ArrowDown));
    ui.key(&up);
    ui.key(&down_key);
    frame(&mut ui);

    let ((), stats) = measure(|| {
        for step in 0..400u16 {
            // Down and up the first twelve commands: highlighting moves, nothing opens.
            let row = f32::from(step % 200) * 0.5;
            let at = Point::new(rect.left() + 30.0, rect.top() + 8.0 + row);
            ui.pointer(PointerEvent::new(PointerKind::Move, at));
            if step % 40 == 0 {
                ui.key(&down_key);
                ui.key(&up);
            }
            frame(&mut ui);
        }
    });
    assert!(ui.is_popup_open(menu));
    assert!(stats.is_zero(), "menu frames allocated: {stats:?}");
}
