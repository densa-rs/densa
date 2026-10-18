//! The widget tree: adding, removing, stale ids, typed access, scrolling.

mod common;

use common::{log, press, take, ui, Probe};
use densa_core::{Point, Rect};
use densa_ui::{
    Event, EventCx, Handled, Overflow, PointerKind, ScrollDelta, Style, UiError, Widget,
};

#[test]
fn removing_a_widget_removes_everything_inside_and_its_ids_go_stale() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let panel = ui
        .add(
            ui.root(),
            Probe::new("panel", &log).focusable(),
            Style::column(),
        )
        .unwrap();
    let child = ui
        .add(panel, Probe::new("child", &log), Style::row())
        .unwrap();
    let kept = ui
        .add(ui.root(), Probe::new("kept", &log), Style::row())
        .unwrap();
    ui.set_focus(Some(panel));
    assert!(ui.remove(panel));
    assert_eq!(ui.children(ui.root()), [kept]);
    assert_eq!(ui.focus(), None, "the focused widget went away");
    for gone in [panel, child] {
        assert!(ui.widget::<Probe>(gone).is_none());
        assert!(ui.rect(gone).is_none());
        assert_eq!(
            ui.add(gone, Probe::new("x", &log), Style::row()),
            Err(UiError::NoSuchWidget(gone))
        );
    }
    assert!(!ui.remove(panel), "already gone");
    assert!(!ui.remove(ui.root()), "the root stays");
    // A new widget may take a freed slot, but never answers to an old id.
    let fresh = ui
        .add(ui.root(), Probe::new("fresh", &log), Style::row())
        .unwrap();
    assert_ne!(fresh, panel);
    assert_ne!(fresh, child);
    assert!(ui.widget::<Probe>(fresh).is_some());
    assert!(ui.widget::<Probe>(panel).is_none());
}

#[test]
fn widgets_are_reached_by_type() {
    struct Other;
    impl Widget for Other {}
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let probe = ui
        .add(ui.root(), Probe::new("probe", &log), Style::row())
        .unwrap();
    let other = ui.add(ui.root(), Other, Style::row()).unwrap();
    assert_eq!(ui.widget::<Probe>(probe).unwrap().name, "probe");
    assert!(ui.widget::<Probe>(other).is_none());
    assert!(ui.widget::<Other>(other).is_some());
    assert_eq!(ui.update::<Probe, _>(other, |_, _| ()), None);
    assert_eq!(ui.parent(probe), Some(ui.root()));
    assert_eq!(ui.style(probe), Some(&Style::row()));
}

#[test]
fn a_removed_widget_under_the_pointer_or_holding_it_lets_go() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let style = Style::row().size(100.0, 100.0);
    let target = ui
        .add(ui.root(), Probe::new("target", &log).handling(), style)
        .unwrap();
    press(&mut ui, 10.0, 10.0);
    assert_eq!(ui.hovered(), Some(target));
    ui.remove(target);
    assert_eq!(ui.hovered(), None);
    take(&log);
    common::release(&mut ui, 10.0, 10.0); // nobody holds the pointer any more
    assert!(take(&log).is_empty());
}

/// Scrolls its content by the wheel.
struct Scroller;

impl Widget for Scroller {
    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        match event {
            Event::Pointer(p) => match p.kind {
                PointerKind::Scroll(ScrollDelta::Pixels(dx, dy)) => {
                    let at = cx.scroll();
                    cx.scroll_to(Point::new(at.x + dx, at.y + dy));
                    Handled::Yes
                }
                _ => Handled::No,
            },
            _ => Handled::No,
        }
    }
}

#[test]
fn scrolling_moves_children_and_stays_within_the_content() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let frame = Style::column()
        .size(100.0, 100.0)
        .overflow(Overflow::Scroll);
    let frame = ui.add(ui.root(), Scroller, frame).unwrap();
    let rows: Vec<_> = (0..10)
        .map(|_| {
            let row = Style::row().size(100.0, 30.0).shrink(0.0);
            ui.add(frame, Probe::new("row", &log), row).unwrap()
        })
        .collect();
    ui.layout();
    assert_eq!(ui.rect(rows[1]), Some(Rect::new(0.0, 30.0, 100.0, 30.0)));
    let wheel = |dy: f32| {
        let kind = PointerKind::Scroll(ScrollDelta::Pixels(0.0, dy));
        densa_ui::PointerEvent::new(kind, Point::new(50.0, 50.0))
    };
    ui.pointer(wheel(45.0));
    ui.layout();
    assert_eq!(ui.rect(rows[1]), Some(Rect::new(0.0, -15.0, 100.0, 30.0)));
    assert_eq!(ui.hit_test(Point::new(50.0, 50.0)), Some(rows[3]));
    // 300 of content in 100 of frame: at most 200 down, and never above the top.
    ui.pointer(wheel(10_000.0));
    ui.layout();
    assert_eq!(ui.rect(rows[9]).unwrap().bottom(), 100.0);
    ui.pointer(wheel(-10_000.0));
    ui.layout();
    assert_eq!(ui.rect(rows[0]).unwrap().top(), 0.0);
}

#[test]
fn scrolling_sideways_moves_children_left_and_stops_at_the_content_end() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let frame = Style::row().size(100.0, 50.0).overflow(Overflow::Scroll);
    let frame = ui.add(ui.root(), Scroller, frame).unwrap();
    let wide = Style::row().size(300.0, 50.0).shrink(0.0);
    let wide = ui.add(frame, Probe::new("wide", &log), wide).unwrap();
    let wheel = |dx: f32| {
        let kind = PointerKind::Scroll(ScrollDelta::Pixels(dx, 0.0));
        densa_ui::PointerEvent::new(kind, Point::new(50.0, 25.0))
    };
    ui.pointer(wheel(50.0));
    ui.layout();
    assert_eq!(ui.rect(wide), Some(Rect::new(-50.0, 0.0, 300.0, 50.0)));
    ui.pointer(wheel(10_000.0));
    ui.layout();
    assert_eq!(
        ui.rect(wide).unwrap().right(),
        100.0,
        "200 at most: 300 in 100"
    );
    ui.pointer(wheel(-10_000.0));
    ui.layout();
    assert_eq!(ui.rect(wide).unwrap().left(), 0.0);
}

#[test]
fn a_widget_restyles_itself_and_what_it_holds_but_nothing_else() {
    struct Shrinker {
        stranger: Option<densa_ui::WidgetId>,
        results: Vec<bool>,
    }
    impl Widget for Shrinker {
        fn event(&mut self, cx: &mut EventCx<'_>, _: &Event<'_>) -> Handled {
            let child = cx.children()[0];
            let smaller = Style::row().size(10.0, 10.0);
            self.results = vec![
                cx.set_style(cx.id(), Style::column().size(80.0, 80.0)),
                cx.set_style(child, smaller.clone()),
                cx.set_style(self.stranger.unwrap(), smaller),
            ];
            Handled::Yes
        }
    }
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let stranger = ui
        .add(
            ui.root(),
            Probe::new("stranger", &log),
            Style::row().size(50.0, 50.0),
        )
        .unwrap();
    let shrinker = Shrinker {
        stranger: Some(stranger),
        results: Vec::new(),
    };
    let parent = ui
        .add(ui.root(), shrinker, Style::column().size(100.0, 100.0))
        .unwrap();
    let child = ui
        .add(
            parent,
            Probe::new("child", &log),
            Style::row().size(50.0, 50.0),
        )
        .unwrap();
    press(&mut ui, 90.0, 140.0); // on the parent, beside the child
    ui.layout();
    assert_eq!(
        ui.widget::<Shrinker>(parent).unwrap().results,
        [true, true, false]
    );
    assert_eq!(ui.rect(parent).unwrap().width(), 80.0);
    assert_eq!(ui.rect(child).unwrap().width(), 10.0);
    assert_eq!(ui.rect(stranger).unwrap().width(), 50.0);
}

#[test]
fn removing_the_only_absolute_child_leaves_its_parent_laid_out() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let parent = ui
        .add(
            ui.root(),
            Probe::new("parent", &log),
            Style::column().grow(1.0),
        )
        .unwrap();
    let inset = densa_ui::Edges::all(densa_ui::Length::Px(10.0));
    let child = ui
        .add(
            parent,
            Probe::new("child", &log),
            Style::column().absolute(inset),
        )
        .unwrap();
    ui.layout();
    assert!(ui.remove(child));
    ui.layout();
    let again = ui
        .add(
            parent,
            Probe::new("again", &log),
            Style::column().absolute(inset),
        )
        .unwrap();
    ui.layout();
    assert_eq!(
        ui.rect(again),
        ui.rect(parent).map(|r| Rect::from_edges(
            r.left() + 10.0,
            r.top() + 10.0,
            r.right() - 10.0,
            r.bottom() - 10.0
        ))
    );
}

#[test]
fn a_hidden_widget_keeps_the_scroll_inside_it_until_shown_again() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let page = ui
        .add(ui.root(), Probe::new("page", &log), Style::column())
        .unwrap();
    let frame = Style::column()
        .size(100.0, 100.0)
        .overflow(Overflow::Scroll);
    let frame = ui.add(page, Scroller, frame).unwrap();
    let rows: Vec<_> = (0..10)
        .map(|_| {
            let row = Style::row().size(100.0, 30.0).shrink(0.0);
            ui.add(frame, Probe::new("row", &log), row).unwrap()
        })
        .collect();
    ui.layout();
    let kind = PointerKind::Scroll(ScrollDelta::Pixels(0.0, 45.0));
    ui.pointer(densa_ui::PointerEvent::new(kind, Point::new(50.0, 50.0)));
    ui.layout();
    let scrolled = ui.rect(rows[1]);
    ui.set_style(page, Style::column().hidden(true));
    ui.layout();
    ui.set_style(page, Style::column());
    ui.layout();
    assert_eq!(ui.rect(rows[1]), scrolled);
}
