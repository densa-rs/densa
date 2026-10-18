//! Behaviour pinned after review: who gets a release, cancelled drags, scroll and hover that
//! follow layout, focus on hidden widgets, layout notifications, insertion order.

mod common;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use common::{log, move_to, press, release, take, ui, Probe};
use densa_core::{Point, Rect, Size};
use densa_ui::{
    Event, EventCx, Handled, Length, Overflow, PaintCx, PointerButton, PointerEvent, PointerKind,
    Style, Widget,
};

#[test]
fn a_release_reaches_only_the_widget_that_took_the_press() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let style = Style::row().size(100.0, 100.0);
    let a = ui
        .add(ui.root(), Probe::new("a", &log).handling(), style.clone())
        .unwrap();
    let b = ui
        .add(ui.root(), Probe::new("b", &log).handling(), style)
        .unwrap();
    // Pressed on the background, released over a widget: nobody gets the release.
    press(&mut ui, 300.0, 250.0);
    release(&mut ui, 10.0, 150.0);
    assert!(!take(&log).iter().any(|e| e.ends_with(":up")));
    // Pressed on `a`, which goes away, released over `b`: still nobody.
    press(&mut ui, 10.0, 10.0);
    ui.remove(a);
    take(&log);
    release(&mut ui, 10.0, 50.0);
    assert!(
        !take(&log).iter().any(|e| e.ends_with(":up")),
        "b never got the press"
    );
    let _ = b;
}

#[test]
fn the_holder_still_holds_the_pointer_while_it_handles_the_release() {
    struct Holder(Rc<Cell<Option<bool>>>);
    impl Widget for Holder {
        fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
            match event {
                Event::Pointer(p) if matches!(p.kind, PointerKind::Down(_)) => Handled::Yes,
                Event::Pointer(p) if matches!(p.kind, PointerKind::Up(_)) => {
                    self.0.set(Some(cx.is_pressed()));
                    Handled::Yes
                }
                _ => Handled::No,
            }
        }
    }
    let seen = Rc::new(Cell::new(None));
    let mut ui = ui(400.0, 300.0);
    let holder = Holder(Rc::clone(&seen));
    ui.add(ui.root(), holder, Style::row().size(100.0, 100.0))
        .unwrap();
    press(&mut ui, 10.0, 10.0);
    release(&mut ui, 10.0, 10.0);
    assert_eq!(seen.get(), Some(true));
}

#[test]
fn a_second_button_goes_to_the_holder_and_does_not_end_the_hold() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let style = Style::row().size(100.0, 100.0);
    ui.add(ui.root(), Probe::new("a", &log).handling(), style.clone())
        .unwrap();
    ui.add(ui.root(), Probe::new("b", &log).handling(), style)
        .unwrap();
    press(&mut ui, 10.0, 10.0);
    let secondary = PointerKind::Down(PointerButton::Secondary);
    ui.pointer(PointerEvent::new(secondary, Point::new(10.0, 150.0)));
    move_to(&mut ui, 10.0, 160.0);
    let seen = take(&log);
    assert_eq!(seen[seen.len() - 2..], ["a:down1", "a:move"]);
}

#[test]
fn a_cancelled_press_tells_the_holder_and_lets_go() {
    struct Dragger(Rc<Cell<bool>>);
    impl Widget for Dragger {
        fn event(&mut self, _: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
            match event {
                Event::Pointer(p) if matches!(p.kind, PointerKind::Cancel) => {
                    self.0.set(true);
                    Handled::Yes
                }
                Event::Pointer(p) if matches!(p.kind, PointerKind::Down(_)) => Handled::Yes,
                _ => Handled::No,
            }
        }
    }
    let cancelled = Rc::new(Cell::new(false));
    let mut ui = ui(400.0, 300.0);
    let dragger = Dragger(Rc::clone(&cancelled));
    ui.add(ui.root(), dragger, Style::row().size(100.0, 100.0))
        .unwrap();
    press(&mut ui, 10.0, 10.0);
    ui.cancel_pointer();
    assert!(cancelled.get());
    assert_eq!(ui.hovered(), None);
    let cancel = PointerEvent::new(PointerKind::Cancel, Point::new(1.0, 1.0));
    assert_eq!(ui.pointer(cancel), Handled::No, "only the Ui sends it");
}

#[test]
fn scrolling_stays_within_content_that_shrinks_or_a_view_that_grows() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let view = Style::column()
        .grow(1.0)
        .min_size(Length::Px(0.0), Length::Px(0.0))
        .overflow(Overflow::Scroll);
    let view = ui.add(ui.root(), Probe::new("view", &log), view).unwrap();
    let rows: Vec<_> = (0..20)
        .map(|_| {
            let row = Style::row().height(Length::Px(30.0)).shrink(0.0);
            ui.add(view, Probe::new("row", &log), row).unwrap()
        })
        .collect();
    ui.layout();
    ui.update::<Probe, _>(view, |_, cx| cx.scroll_to(Point::new(0.0, 10_000.0)));
    ui.layout();
    assert_eq!(ui.rect(rows[19]).unwrap().bottom(), 300.0);
    for &row in &rows[5..] {
        ui.remove(row);
    }
    ui.layout();
    assert_eq!(
        ui.rect(rows[0]).unwrap().top(),
        0.0,
        "150 of content fits: no scroll"
    );
    ui.resize(Size::new(400.0, 2_000.0));
    ui.layout();
    assert_eq!(ui.rect(rows[0]).unwrap().top(), 0.0);
}

#[test]
fn hover_follows_widgets_that_move_under_a_still_pointer_and_leaves_with_them() {
    struct Hovered(Rc<Cell<bool>>);
    impl Widget for Hovered {
        fn paint(&mut self, cx: &PaintCx<'_>, _: &mut densa_draw::Canvas<'_>) {
            self.0.set(cx.is_hovered());
        }
    }
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let flag = Rc::new(Cell::new(false));
    let panel = Hovered(Rc::clone(&flag));
    let panel = ui
        .add(ui.root(), panel, Style::column().size(200.0, 200.0))
        .unwrap();
    let child = ui
        .add(
            panel,
            Probe::new("child", &log),
            Style::row().size(50.0, 50.0),
        )
        .unwrap();
    move_to(&mut ui, 10.0, 10.0);
    assert_eq!(ui.hovered(), Some(child));
    // Removed, and the pointer leaves before any layout: nothing keeps a hovered flag.
    ui.remove(child);
    ui.pointer_left(Point::new(-1.0, -1.0));
    let mut cache = densa_text::GlyphCache::new(densa_text::AtlasConfig::default());
    let mut out = densa_draw::InstanceBuffer::with_capacity(64);
    ui.paint(&mut densa_draw::Canvas::begin(&mut cache, &mut out));
    assert!(!flag.get(), "the removed child's chain is unmarked");
    // The pointer comes back: the panel is the one under it, then nothing once it leaves.
    move_to(&mut ui, 10.0, 10.0);
    assert_eq!(ui.hovered(), Some(panel));
    ui.pointer_left(Point::new(-1.0, -1.0));
    ui.paint(&mut densa_draw::Canvas::begin(&mut cache, &mut out));
    assert!(!flag.get());
    // A widget moving in under the pointer becomes the hovered one.
    move_to(&mut ui, 300.0, 20.0); // beside the panel, over the root
    let late = ui
        .insert(
            ui.root(),
            0,
            Probe::new("late", &log),
            Style::row().size(400.0, 300.0),
        )
        .unwrap();
    ui.layout();
    assert_eq!(ui.hovered(), Some(late));
}

#[test]
fn a_hidden_widget_loses_focus() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let group = ui
        .add(ui.root(), Probe::new("group", &log), Style::column())
        .unwrap();
    let field = Probe::new("field", &log).focusable();
    let field = ui.add(group, field, Style::row().size(50.0, 20.0)).unwrap();
    ui.set_focus(Some(field));
    ui.set_style(group, Style::column().hidden(true));
    ui.layout();
    assert_eq!(ui.focus(), None);
    ui.set_focus(Some(field));
    assert_eq!(ui.focus(), None, "a hidden widget cannot take focus");
}

#[test]
fn widgets_are_told_their_size_only_when_it_or_their_content_changed() {
    struct Counted(Rc<Cell<u32>>);
    impl Widget for Counted {
        fn layout(&mut self, _: &mut densa_ui::LayoutCx<'_>, _: Size) {
            self.0.set(self.0.get() + 1);
        }
    }
    let calls = Rc::new(Cell::new(0));
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let counted = Counted(Rc::clone(&calls));
    let counted = ui
        .add(ui.root(), counted, Style::row().size(50.0, 20.0))
        .unwrap();
    let other = ui
        .add(
            ui.root(),
            Probe::new("other", &log),
            Style::row().size(10.0, 10.0),
        )
        .unwrap();
    ui.layout();
    assert_eq!(calls.get(), 1);
    ui.set_style(other, Style::row().size(20.0, 10.0)); // someone else changed
    ui.layout();
    assert_eq!(calls.get(), 1);
    ui.update::<Counted, _>(counted, |_, cx| cx.request_layout()); // its content changed
    ui.layout();
    assert_eq!(calls.get(), 2);
    ui.set_style(counted, Style::row().size(60.0, 20.0)); // its size changed
    ui.layout();
    assert_eq!(calls.get(), 3);
}

#[test]
fn widgets_insert_at_a_position_and_content_rects_leave_out_the_padding() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let row = Style::row().padding(10.0);
    let row = ui.add(ui.root(), Probe::new("row", &log), row).unwrap();
    let b = ui.add(row, Probe::new("b", &log), Style::row()).unwrap();
    let a = ui
        .insert(row, 0, Probe::new("a", &log), Style::row())
        .unwrap();
    let c = ui
        .insert(row, 99, Probe::new("c", &log), Style::row())
        .unwrap();
    assert_eq!(ui.children(row), [a, b, c]);
    struct Content(Rc<Cell<Rect>>);
    impl Widget for Content {
        fn paint(&mut self, cx: &PaintCx<'_>, _: &mut densa_draw::Canvas<'_>) {
            self.0.set(cx.content_rect());
        }
    }
    let seen = Rc::new(Cell::new(Rect::default()));
    let style = Style::row().size(100.0, 50.0).padding(8.0);
    ui.add(ui.root(), Content(Rc::clone(&seen)), style).unwrap();
    let mut cache = densa_text::GlyphCache::new(densa_text::AtlasConfig::default());
    let mut out = densa_draw::InstanceBuffer::with_capacity(64);
    ui.paint(&mut densa_draw::Canvas::begin(&mut cache, &mut out));
    let content = seen.get();
    assert_eq!((content.width(), content.height()), (84.0, 34.0));
}

#[test]
fn a_widget_can_give_focus_up_and_bad_scales_are_ignored() {
    struct Quitter;
    impl Widget for Quitter {
        fn event(&mut self, cx: &mut EventCx<'_>, _: &Event<'_>) -> Handled {
            cx.release_focus();
            Handled::Yes
        }
        fn accepts_focus(&self) -> bool {
            true
        }
    }
    let mut ui = ui(400.0, 300.0);
    let quitter = ui
        .add(ui.root(), Quitter, Style::row().size(50.0, 50.0))
        .unwrap();
    ui.set_focus(Some(quitter));
    assert_eq!(ui.focus(), None, "it gave focus up when told it had it");
    for scale in [0.0, -1.0, f32::INFINITY, f32::NAN] {
        ui.set_scale(scale);
        assert_eq!(ui.scale(), 1.0);
    }
}

#[test]
fn multi_clicks_need_the_same_place_on_both_axes_scaled() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    ui.add(
        ui.root(),
        Probe::new("t", &log).handling(),
        Style::row().size(400.0, 300.0),
    )
    .unwrap();
    ui.set_scale(2.0);
    let down = |ms: u64, x: f32, y: f32| {
        let kind = PointerKind::Down(PointerButton::Primary);
        PointerEvent::new(kind, Point::new(x, y)).at(Duration::from_millis(ms))
    };
    let up = PointerEvent::new(
        PointerKind::Up(PointerButton::Primary),
        Point::new(0.0, 0.0),
    );
    // 7 px apart at scale 2 is within 4 logical px; 20 px down is not.
    for (ms, x, y) in [(0, 100.0, 100.0), (100, 107.0, 100.0), (200, 107.0, 120.0)] {
        ui.pointer(down(ms, x, y));
        ui.pointer(up);
    }
    let presses: Vec<String> = take(&log)
        .into_iter()
        .filter(|e| e.starts_with("t:down"))
        .collect();
    assert_eq!(presses, ["t:down1", "t:down2", "t:down1"]);
}

#[test]
fn a_child_overflowing_a_parent_outside_the_clip_is_still_painted() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let frame = Style::column().size(100.0, 100.0).overflow(Overflow::Clip);
    let frame = ui.add(ui.root(), Probe::new("frame", &log), frame).unwrap();
    // The parent sits left of the frame; its child reaches back into it.
    let at = |left: f32| densa_ui::Edges {
        left: Length::Px(left),
        top: Length::Px(10.0),
        right: Length::Auto,
        bottom: Length::Auto,
    };
    let parent = Style::row().absolute(at(-50.0)).size(10.0, 10.0);
    let parent = ui.add(frame, Probe::new("parent", &log), parent).unwrap();
    let child = Style::row().absolute(at(60.0)).size(10.0, 10.0);
    let child = ui.add(parent, Probe::new("child", &log), child).unwrap();
    let mut cache = densa_text::GlyphCache::new(densa_text::AtlasConfig::default());
    let mut out = densa_draw::InstanceBuffer::with_capacity(64);
    ui.paint(&mut densa_draw::Canvas::begin(&mut cache, &mut out));
    assert!(ui.rect(parent).unwrap().right() < 0.0);
    assert_eq!(ui.widget::<Probe>(child).unwrap().painted, 1);
}
