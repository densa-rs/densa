//! Event delivery: hit-testing, bubbling, pointer capture, hover, focus, Tab, multi-clicks.

mod common;

use std::time::Duration;

use common::{log, move_to, press, release, take, ui, Probe};
use densa_core::{Point, Rect};
use densa_ui::{
    Cursor, Event, EventCx, Handled, Ime, Key, KeyEvent, Modifiers, NamedKey, Overflow,
    PointerButton, PointerEvent, PointerKind, Style, Ui, Widget, WidgetId,
};

/// Root column; a 100x100 `outer` at the top-left holding a 50x50 `inner` at its corner, and
/// a 100x100 `other` below.
fn nested(outer_handles: bool) -> (Ui, common::Log, WidgetId, WidgetId, WidgetId) {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let mut outer = Probe::new("outer", &log);
    outer.handles_pointer = outer_handles;
    let outer = ui
        .add(ui.root(), outer, Style::column().size(100.0, 100.0))
        .unwrap();
    let inner = Style::row().size(50.0, 50.0);
    let inner = ui.add(outer, Probe::new("inner", &log), inner).unwrap();
    let other = Style::row().size(100.0, 100.0);
    let other = ui
        .add(ui.root(), Probe::new("other", &log).handling(), other)
        .unwrap();
    (ui, log, outer, inner, other)
}

#[test]
fn an_unhandled_event_bubbles_to_the_parent() {
    let (mut ui, log, outer, inner, _) = nested(true);
    assert_eq!(ui.hit_test(Point::new(10.0, 10.0)), Some(inner));
    assert_eq!(press(&mut ui, 10.0, 10.0), Handled::Yes);
    let seen = take(&log);
    assert_eq!(seen, ["inner:enter", "inner:down1", "outer:down1"]);
    assert_eq!(ui.hit_test(Point::new(75.0, 75.0)), Some(outer));
    assert_eq!(ui.hit_test(Point::new(399.0, 299.0)), Some(ui.root()));
}

#[test]
fn the_widget_that_handled_a_press_gets_the_moves_and_the_release() {
    let (mut ui, log, outer, _, _) = nested(true);
    press(&mut ui, 10.0, 10.0);
    take(&log);
    // Dragging over `other`: still `outer`'s, and hover does not follow until release.
    move_to(&mut ui, 50.0, 150.0);
    release(&mut ui, 50.0, 150.0);
    let seen = take(&log);
    assert_eq!(
        seen,
        ["outer:move", "outer:up", "inner:leave", "other:enter"]
    );
    assert_eq!(ui.hovered(), ui.hit_test(Point::new(50.0, 150.0)));
    let _ = outer;
}

#[test]
fn without_a_handler_moves_go_to_whatever_is_under_the_pointer() {
    let (mut ui, log, _, _, other) = nested(false);
    press(&mut ui, 10.0, 10.0); // nobody handles it: no capture
    take(&log);
    move_to(&mut ui, 50.0, 150.0);
    let seen = take(&log);
    assert_eq!(seen, ["inner:leave", "other:enter", "other:move"]);
    assert_eq!(ui.hovered(), Some(other));
}

#[test]
fn hover_marks_the_widget_under_the_pointer_and_its_parents() {
    struct Hovering(std::rc::Rc<std::cell::Cell<bool>>);
    impl Widget for Hovering {
        fn event(&mut self, cx: &mut EventCx<'_>, _: &Event<'_>) -> Handled {
            self.0.set(cx.is_hovered());
            Handled::No
        }
    }
    let flag = std::rc::Rc::new(std::cell::Cell::new(false));
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let parent = Hovering(std::rc::Rc::clone(&flag));
    let parent = ui
        .add(ui.root(), parent, Style::column().size(100.0, 100.0))
        .unwrap();
    let child = ui
        .add(
            parent,
            Probe::new("child", &log),
            Style::row().size(20.0, 20.0),
        )
        .unwrap();
    move_to(&mut ui, 5.0, 5.0);
    assert_eq!(ui.hovered(), Some(child));
    move_to(&mut ui, 50.0, 50.0); // over the parent only: it gets Enter and sees its flag
    assert!(flag.get());
    ui.pointer_left(Point::new(-1.0, -1.0));
    assert_eq!(ui.hovered(), None);
    assert!(!flag.get(), "Leave arrives after the flag is cleared");
}

#[test]
fn a_press_focuses_the_nearest_widget_that_accepts_focus() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let field = Probe::new("field", &log).focusable();
    let field = ui
        .add(ui.root(), field, Style::column().size(100.0, 40.0))
        .unwrap();
    let icon = ui
        .add(
            field,
            Probe::new("icon", &log),
            Style::row().size(10.0, 10.0),
        )
        .unwrap();
    press(&mut ui, 2.0, 2.0); // on the icon, inside the field
    assert_eq!(ui.focus(), Some(field));
    assert_eq!(take(&log)[..2], ["icon:enter", "field:focus+"]);
    // A press on nothing focusable keeps the focus where it is.
    release(&mut ui, 2.0, 2.0);
    press(&mut ui, 300.0, 200.0);
    assert_eq!(ui.focus(), Some(field));
    let _ = icon;
}

#[test]
fn tab_walks_focusable_widgets_in_tree_order_and_skips_hidden_ones() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let row = ui
        .add(ui.root(), Probe::new("row", &log), Style::row())
        .unwrap();
    let a = ui
        .add(row, Probe::new("a", &log).focusable(), Style::row())
        .unwrap();
    let group = ui
        .add(row, Probe::new("group", &log), Style::row())
        .unwrap();
    let b = ui
        .add(group, Probe::new("b", &log).focusable(), Style::row())
        .unwrap();
    let hidden = Style::row().hidden(true);
    let gone = ui
        .add(row, Probe::new("gone", &log).focusable(), hidden)
        .unwrap();
    let c = ui
        .add(row, Probe::new("c", &log).focusable(), Style::row())
        .unwrap();
    let tab = KeyEvent::down(Key::Named(NamedKey::Tab));
    let back = tab.with_modifiers(Modifiers::SHIFT);
    let mut order = Vec::new();
    for _ in 0..4 {
        ui.key(&tab);
        order.push(ui.focus().unwrap());
    }
    assert_eq!(order, [a, b, c, a], "forward, wrapping around");
    ui.key(&back);
    assert_eq!(ui.focus(), Some(c), "backward wraps too");
    assert_ne!(ui.focus(), Some(gone));
    let seen = take(&log);
    assert!(seen.contains(&"a:focus-".to_owned()) && seen.contains(&"b:focus+".to_owned()));
}

#[test]
fn keys_go_to_the_focused_widget_and_bubble_and_a_handled_tab_stays() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let form = Probe::new("form", &log).handling();
    let form = ui.add(ui.root(), form, Style::column()).unwrap();
    let field = Probe::new("field", &log).focusable();
    let field = ui.add(form, field, Style::row()).unwrap();
    let other = ui
        .add(form, Probe::new("other", &log).focusable(), Style::row())
        .unwrap();
    ui.set_focus(Some(field));
    take(&log);
    let x = KeyEvent::down(Key::Character("x")).with_text("x");
    assert_eq!(ui.key(&x), Handled::Yes);
    assert_eq!(take(&log), ["field:key x", "form:key x"]);
    // The form handles every key, Tab included: focus stays.
    ui.key(&KeyEvent::down(Key::Named(NamedKey::Tab)));
    assert_eq!(ui.focus(), Some(field));
    let ime = Ime::Commit("é");
    assert_eq!(ui.ime(&ime), Handled::Yes);
    let _ = other;
}

#[test]
fn quick_presses_in_one_place_count_as_multi_clicks() {
    let (mut ui, log, _, _, _) = nested(true);
    let at = |ms: u64, x: f32, button: PointerButton| {
        PointerEvent::new(PointerKind::Down(button), Point::new(x, 10.0))
            .at(Duration::from_millis(ms))
    };
    let up = PointerEvent::new(
        PointerKind::Up(PointerButton::Primary),
        Point::new(5.0, 10.0),
    );
    for event in [
        at(0, 5.0, PointerButton::Primary),
        at(200, 6.0, PointerButton::Primary),
        at(390, 5.0, PointerButton::Primary),
        at(1_500, 5.0, PointerButton::Primary),
        at(1_600, 40.0, PointerButton::Primary),
        at(1_700, 40.0, PointerButton::Secondary),
    ] {
        ui.pointer(event);
        ui.pointer(up);
    }
    let presses: Vec<String> = take(&log)
        .into_iter()
        .filter(|e| e.starts_with("outer:down"))
        .collect();
    assert_eq!(
        presses,
        [
            "outer:down1",
            "outer:down2",
            "outer:down3",
            "outer:down1", // too late
            "outer:down1", // too far
            "outer:down1", // another button
        ]
    );
}

#[test]
fn a_widget_can_ask_for_focus_and_the_cursor_and_ime_area_follow() {
    struct Field;
    impl Widget for Field {
        fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
            match event {
                Event::Pointer(p) if matches!(p.kind, PointerKind::Enter) => {
                    cx.set_cursor(Some(Cursor::Text));
                    Handled::Yes
                }
                Event::Pointer(p) if matches!(p.kind, PointerKind::Down(_)) => {
                    cx.request_focus();
                    Handled::Yes
                }
                Event::FocusGained => {
                    cx.set_ime_area(Some(Rect::new(1.0, 2.0, 3.0, 4.0)));
                    Handled::Yes
                }
                _ => Handled::No,
            }
        }
    }
    let mut ui = ui(400.0, 300.0);
    let field = ui
        .add(ui.root(), Field, Style::row().size(100.0, 20.0))
        .unwrap();
    move_to(&mut ui, 5.0, 5.0);
    assert_eq!(ui.cursor(), Cursor::Text);
    press(&mut ui, 5.0, 5.0);
    assert_eq!(ui.focus(), Some(field));
    assert_eq!(ui.ime_area(), Some(Rect::new(1.0, 2.0, 3.0, 4.0)));
    ui.set_focus(None);
    assert_eq!(ui.ime_area(), None, "only the focused widget's area counts");
    move_to(&mut ui, 300.0, 200.0);
    assert_eq!(
        ui.cursor(),
        Cursor::Text,
        "the pressed widget keeps the pointer"
    );
    release(&mut ui, 300.0, 200.0);
    assert_eq!(ui.cursor(), Cursor::Default);
}

#[test]
fn a_clipping_widget_hides_its_children_outside_it_from_the_pointer() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let frame = Style::column().size(50.0, 50.0).overflow(Overflow::Clip);
    let frame = ui.add(ui.root(), Probe::new("frame", &log), frame).unwrap();
    let wide = Style::row().size(200.0, 20.0).shrink(0.0);
    let wide = ui.add(frame, Probe::new("wide", &log), wide).unwrap();
    assert_eq!(ui.hit_test(Point::new(10.0, 10.0)), Some(wide));
    assert_eq!(ui.hit_test(Point::new(150.0, 10.0)), Some(ui.root()));
    // Without clipping, the overflowing child is still there to hit.
    ui.set_style(frame, Style::column().size(50.0, 50.0));
    assert_eq!(ui.hit_test(Point::new(150.0, 10.0)), Some(wide));
}

#[test]
fn enter_and_leave_from_outside_are_ignored() {
    let (mut ui, log, _, _, _) = nested(true);
    let enter = PointerEvent::new(PointerKind::Enter, Point::new(10.0, 10.0));
    assert_eq!(ui.pointer(enter), Handled::No);
    assert!(take(&log).is_empty());
}

#[test]
fn a_later_sibling_is_over_an_earlier_one() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let flow = ui
        .add(
            ui.root(),
            Probe::new("flow", &log),
            Style::row().size(100.0, 100.0),
        )
        .unwrap();
    let inset = densa_ui::Edges::all(densa_ui::Length::Px(0.0));
    let over = Style::row().absolute(inset).size(50.0, 50.0);
    let over = ui.add(ui.root(), Probe::new("over", &log), over).unwrap();
    assert_eq!(ui.hit_test(Point::new(10.0, 10.0)), Some(over));
    assert_eq!(ui.hit_test(Point::new(75.0, 75.0)), Some(flow));
}

#[test]
fn presses_exactly_the_multi_click_time_apart_still_count() {
    let (mut ui, log, _, _, _) = nested(true);
    let down = |ms: u64| {
        let kind = PointerKind::Down(PointerButton::Primary);
        PointerEvent::new(kind, Point::new(5.0, 5.0)).at(Duration::from_millis(ms))
    };
    let up = PointerEvent::new(
        PointerKind::Up(PointerButton::Primary),
        Point::new(5.0, 5.0),
    );
    for ms in [0, 500, 1_001] {
        ui.pointer(down(ms));
        ui.pointer(up);
    }
    let presses: Vec<String> = take(&log)
        .into_iter()
        .filter(|e| e.starts_with("outer:down"))
        .collect();
    assert_eq!(presses, ["outer:down1", "outer:down2", "outer:down1"]);
}

#[test]
fn a_widget_shows_its_parents_cursor_unless_it_sets_one() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let parent = ui
        .add(
            ui.root(),
            Probe::new("parent", &log),
            Style::column().size(100.0, 100.0),
        )
        .unwrap();
    let child = ui
        .add(
            parent,
            Probe::new("child", &log),
            Style::row().size(20.0, 20.0),
        )
        .unwrap();
    ui.update::<Probe, _>(parent, |_, cx| cx.set_cursor(Some(Cursor::Grab)));
    move_to(&mut ui, 5.0, 5.0);
    assert_eq!(ui.hovered(), Some(child));
    assert_eq!(ui.cursor(), Cursor::Grab);
    ui.update::<Probe, _>(child, |_, cx| cx.set_cursor(Some(Cursor::Text)));
    assert_eq!(ui.cursor(), Cursor::Text);
}

#[test]
fn the_application_can_focus_through_a_widget_and_a_widget_can_let_the_pointer_go() {
    struct Dropper;
    impl Widget for Dropper {
        fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
            match event {
                Event::Pointer(p) if matches!(p.kind, PointerKind::Down(_)) => Handled::Yes,
                Event::Pointer(p) if matches!(p.kind, PointerKind::Move) => {
                    cx.release_pointer();
                    Handled::Yes
                }
                _ => Handled::No,
            }
        }
    }
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let dropper = ui
        .add(ui.root(), Dropper, Style::row().size(100.0, 100.0))
        .unwrap();
    let other = Probe::new("other", &log).focusable();
    let other = ui
        .add(ui.root(), other, Style::row().size(100.0, 100.0))
        .unwrap();
    ui.update::<Probe, _>(other, |_, cx| cx.request_focus());
    assert_eq!(ui.focus(), Some(other));
    press(&mut ui, 10.0, 10.0);
    move_to(&mut ui, 10.0, 150.0); // the dropper lets go on its first move
    take(&log);
    move_to(&mut ui, 10.0, 160.0);
    assert_eq!(take(&log), ["other:enter", "other:move"]);
    let _ = dropper;
}
