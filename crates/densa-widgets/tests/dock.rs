//! The dock driven headless: placing panels, switching tabs, dragging tabs and dividers.

use densa_core::{Point, Rect, Size};
use densa_draw::{Canvas, InstanceBuffer};
use densa_paragraph::TextSystem;
use densa_text::{AtlasConfig, GlyphCache};
use densa_ui::{
    Action, ActionKind, Cursor, PointerButton, PointerEvent, PointerKind, Ui, UiError, WidgetId,
};
use densa_ui::{Event, EventCx, Handled, LayoutCx, ScrollDelta, Style, Widget};
use densa_widgets::{DockArea, DockAt, DockPanel, DockSide, Label, Panel, ScrollView};
use std::cell::Cell;
use std::rc::Rc;

/// The tab bar's height and the gap between groups, as the dock draws them at scale 1.
const BAR: f32 = 28.0;
const GAP: f32 = 6.0;

fn dock() -> (Ui, WidgetId) {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(800.0, 600.0));
    let dock = ui
        .add(ui.root(), DockArea::new(), DockArea::style())
        .unwrap();
    (ui, dock)
}

fn panel(ui: &mut Ui, dock: WidgetId, title: &str, at: DockAt) -> WidgetId {
    DockArea::add_panel(ui, dock, Panel::new(), title, at).unwrap()
}

fn area(ui: &Ui, dock: WidgetId) -> &DockArea {
    ui.widget::<DockArea>(dock).unwrap()
}

fn pointer(ui: &mut Ui, kind: PointerKind, x: f32, y: f32) {
    ui.pointer(PointerEvent::new(kind, Point::new(x, y)));
}

fn drag(ui: &mut Ui, from: (f32, f32), to: (f32, f32)) {
    pointer(
        ui,
        PointerKind::Down(PointerButton::Primary),
        from.0,
        from.1,
    );
    pointer(ui, PointerKind::Move, from.0 + 20.0, from.1 + 20.0);
    pointer(ui, PointerKind::Move, to.0, to.1);
    pointer(ui, PointerKind::Up(PointerButton::Primary), to.0, to.1);
    ui.layout();
}

fn actions(ui: &mut Ui) -> Vec<ActionKind> {
    ui.actions().map(|Action { kind, .. }| kind).collect()
}

fn rect(ui: &Ui, id: WidgetId) -> Rect {
    ui.rect(id).unwrap()
}

fn frame(ui: &mut Ui) -> Vec<Rect> {
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(16_384);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    ui.paint(&mut canvas);
    out.as_slice()
        .iter()
        .map(|i| Rect::new(i.rect[0], i.rect[1], i.rect[2], i.rect[3]))
        .collect()
}

#[test]
fn a_lone_panel_fills_the_dock_below_its_tab_bar() {
    let (mut ui, dock) = dock();
    let only = panel(&mut ui, dock, "Watchlist", DockAt::First);
    ui.layout();
    let whole = rect(&ui, dock);
    assert_eq!(
        rect(&ui, only),
        Rect::from_edges(0.0, BAR, whole.right(), whole.bottom())
    );
    assert!(area(&ui, dock).is_shown(only));
    assert_eq!(area(&ui, dock).group_count(), 1);
}

#[test]
fn tabs_of_one_group_show_one_panel_and_a_click_switches_it() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::With(a));
    ui.layout();
    let dock_area = area(&ui, dock);
    assert!(dock_area.is_shown(b), "the newest tab is shown");
    assert!(!dock_area.is_shown(a));
    assert_eq!(dock_area.tabs_with(a).collect::<Vec<_>>(), [a, b]);
    assert!(
        ui.rect(a).is_none_or(|r| r.is_empty()),
        "a hidden tab takes no space"
    );

    pointer(
        &mut ui,
        PointerKind::Down(PointerButton::Primary),
        8.0,
        14.0,
    );
    pointer(&mut ui, PointerKind::Up(PointerButton::Primary), 8.0, 14.0);
    ui.layout();
    assert!(area(&ui, dock).is_shown(a));
    assert_eq!(actions(&mut ui), [ActionKind::Selected(0)]);
    // Clicking the shown tab again reports nothing.
    pointer(
        &mut ui,
        PointerKind::Down(PointerButton::Primary),
        8.0,
        14.0,
    );
    pointer(&mut ui, PointerKind::Up(PointerButton::Primary), 8.0, 14.0);
    assert!(actions(&mut ui).is_empty());
}

#[test]
fn a_panel_beside_another_halves_its_group_across_the_gap() {
    let (mut ui, dock) = dock();
    let left = panel(&mut ui, dock, "Left", DockAt::First);
    let right = panel(
        &mut ui,
        dock,
        "Right",
        DockAt::Beside(left, DockSide::Right),
    );
    let below = panel(
        &mut ui,
        dock,
        "Below",
        DockAt::Beside(right, DockSide::Bottom),
    );
    ui.layout();
    assert_eq!(area(&ui, dock).group_count(), 3);
    assert_eq!(
        rect(&ui, left),
        Rect::from_edges(0.0, BAR, 400.0 - GAP / 2.0, 600.0)
    );
    assert_eq!(
        rect(&ui, right),
        Rect::from_edges(403.0, BAR, 800.0, 300.0 - GAP / 2.0)
    );
    assert_eq!(
        rect(&ui, below),
        Rect::from_edges(403.0, 303.0 + BAR, 800.0, 600.0)
    );
}

#[test]
fn dragging_a_divider_resizes_both_sides_within_limits() {
    let (mut ui, dock) = dock();
    let left = panel(&mut ui, dock, "Left", DockAt::First);
    let right = panel(
        &mut ui,
        dock,
        "Right",
        DockAt::Beside(left, DockSide::Right),
    );
    ui.layout();
    pointer(
        &mut ui,
        PointerKind::Down(PointerButton::Primary),
        400.0,
        300.0,
    );
    pointer(&mut ui, PointerKind::Move, 200.0, 300.0);
    pointer(
        &mut ui,
        PointerKind::Up(PointerButton::Primary),
        200.0,
        300.0,
    );
    ui.layout();
    assert_eq!(rect(&ui, left).right(), 197.0);
    assert_eq!(rect(&ui, right).left(), 203.0);
    assert_eq!(actions(&mut ui), [ActionKind::Changed]);
    // Dragged past the end, it stops short of hiding the right side.
    pointer(
        &mut ui,
        PointerKind::Down(PointerButton::Primary),
        200.0,
        300.0,
    );
    pointer(&mut ui, PointerKind::Move, 900.0, 300.0);
    pointer(
        &mut ui,
        PointerKind::Up(PointerButton::Primary),
        900.0,
        300.0,
    );
    ui.layout();
    // 48 px either side of the divider's middle: the right group starts half a gap after.
    assert_eq!(rect(&ui, right).width(), 48.0 - GAP / 2.0);
    assert_eq!(actions(&mut ui), [ActionKind::Changed]);
}

#[test]
fn dropping_a_tab_near_an_edge_splits_and_into_a_group_joins() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::With(a));
    ui.layout();
    // B's tab (after A's) to the right edge of its own group: B gets the right half.
    drag(&mut ui, (40.0, 14.0), (780.0, 300.0));
    assert_eq!(area(&ui, dock).group_count(), 2);
    assert!(rect(&ui, b).left() > 400.0);
    assert!(
        area(&ui, dock).is_shown(a),
        "A is shown again in the left group"
    );
    assert_eq!(actions(&mut ui), [ActionKind::Changed]);

    // B's tab (now first in the right group) into the middle of A's group: one group again.
    let tab = rect(&ui, b).left() + 8.0;
    drag(&mut ui, (tab, 14.0), (200.0, 300.0));
    assert_eq!(area(&ui, dock).group_count(), 1);
    assert_eq!(area(&ui, dock).tabs_with(a).collect::<Vec<_>>(), [a, b]);
    assert_eq!(
        rect(&ui, b).right(),
        800.0,
        "the merged group takes the whole dock"
    );
}

#[test]
fn a_lone_tab_dropped_on_its_own_group_changes_nothing() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    ui.layout();
    let before = rect(&ui, a);
    drag(&mut ui, (8.0, 14.0), (790.0, 300.0));
    assert_eq!(rect(&ui, a), before);
    assert!(actions(&mut ui).is_empty());
}

#[test]
fn a_cancelled_tab_drag_leaves_the_tabs_alone() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::With(a));
    ui.layout();
    pointer(
        &mut ui,
        PointerKind::Down(PointerButton::Primary),
        40.0,
        14.0,
    );
    pointer(&mut ui, PointerKind::Move, 780.0, 300.0);
    ui.cancel_pointer();
    pointer(
        &mut ui,
        PointerKind::Up(PointerButton::Primary),
        780.0,
        300.0,
    );
    ui.layout();
    assert_eq!(area(&ui, dock).group_count(), 1);
    assert_eq!(area(&ui, dock).tabs_with(a).collect::<Vec<_>>(), [a, b]);
}

#[test]
fn a_dragged_tab_previews_where_it_would_land() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    panel(&mut ui, dock, "B", DockAt::With(a));
    ui.layout();
    // Where B's new group would be: the right half, beyond the gap that will divide them.
    let half = Rect::from_edges(400.0 + GAP / 2.0, 0.0, 800.0, 600.0);
    assert!(!frame(&mut ui).contains(&half));
    pointer(
        &mut ui,
        PointerKind::Down(PointerButton::Primary),
        40.0,
        14.0,
    );
    pointer(&mut ui, PointerKind::Move, 780.0, 300.0);
    assert!(
        frame(&mut ui).contains(&half),
        "the right half is highlighted"
    );
}

#[test]
fn removing_panels_shows_the_next_tab_and_gives_space_to_the_neighbor() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::With(a));
    let c = panel(&mut ui, dock, "C", DockAt::Beside(b, DockSide::Bottom));
    ui.layout();
    DockArea::remove_panel(&mut ui, dock, b).unwrap();
    ui.layout();
    assert!(ui.rect(b).is_none(), "the widget is gone");
    assert!(area(&ui, dock).is_shown(a));
    assert_eq!(area(&ui, dock).panels().collect::<Vec<_>>(), [a, c]);

    DockArea::remove_panel(&mut ui, dock, a).unwrap();
    ui.layout();
    assert_eq!(area(&ui, dock).group_count(), 1);
    assert_eq!(rect(&ui, c), Rect::from_edges(0.0, BAR, 800.0, 600.0));
    assert_eq!(
        DockArea::remove_panel(&mut ui, dock, a),
        Err(UiError::NoSuchWidget(a))
    );
}

#[test]
fn panels_must_be_placed_next_to_panels_of_the_same_dock() {
    let (mut ui, dock) = dock();
    let stranger = ui.add(ui.root(), Panel::new(), Default::default()).unwrap();
    let placed = DockArea::add_panel(&mut ui, dock, Panel::new(), "X", DockAt::With(stranger));
    assert_eq!(placed, Err(UiError::NoSuchWidget(stranger)));
    let not_a_dock = DockArea::add_panel(&mut ui, stranger, Panel::new(), "X", DockAt::First);
    assert_eq!(not_a_dock, Err(UiError::NoSuchWidget(stranger)));
    assert!(ui.children(dock).is_empty(), "a refused panel is not added");
}

#[test]
fn removing_a_tab_before_the_shown_one_keeps_showing_it() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::With(a));
    let c = panel(&mut ui, dock, "C", DockAt::With(a));
    ui.update::<DockArea, _>(dock, |area, cx| area.show(cx, b));
    DockArea::remove_panel(&mut ui, dock, a).unwrap();
    ui.layout();
    assert!(area(&ui, dock).is_shown(b));
    assert!(!area(&ui, dock).is_shown(c));
}

#[test]
fn a_tab_dropped_on_another_groups_tab_bar_joins_that_group() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::Beside(a, DockSide::Right));
    ui.layout();
    // B's tab to the empty end of A's tab bar, which is near A's top edge.
    let tab = rect(&ui, b).left() + 8.0;
    drag(&mut ui, (tab, 14.0), (300.0, 10.0));
    assert_eq!(area(&ui, dock).group_count(), 1);
    assert_eq!(area(&ui, dock).tabs_with(a).collect::<Vec<_>>(), [a, b]);
}

#[test]
fn a_tab_dropped_into_its_own_group_keeps_its_place() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::With(a));
    ui.layout();
    drag(&mut ui, (8.0, 14.0), (400.0, 300.0));
    assert_eq!(area(&ui, dock).tabs_with(a).collect::<Vec<_>>(), [a, b]);
    assert_eq!(
        actions(&mut ui),
        [ActionKind::Selected(0)],
        "only the press showed A"
    );
}

#[test]
fn a_press_that_barely_moves_is_a_click_not_a_drag() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    panel(&mut ui, dock, "B", DockAt::With(a));
    ui.layout();
    // Dragged to the left half of A's tab, B would go before A: a marker shows where.
    let still = frame(&mut ui).len();
    pointer(
        &mut ui,
        PointerKind::Down(PointerButton::Primary),
        40.0,
        14.0,
    );
    pointer(&mut ui, PointerKind::Move, 37.0, 17.0);
    assert_eq!(frame(&mut ui).len(), still, "a small move shows nothing");
    assert_eq!(ui.cursor(), Cursor::Default);
    pointer(&mut ui, PointerKind::Move, 4.0, 14.0);
    assert_eq!(
        frame(&mut ui).len(),
        still + 1,
        "past the threshold it drags"
    );
    assert_eq!(ui.cursor(), Cursor::Grabbing);
}

#[test]
fn the_cursor_shows_which_way_a_divider_moves() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::Beside(a, DockSide::Right));
    panel(&mut ui, dock, "C", DockAt::Beside(b, DockSide::Bottom));
    ui.layout();
    pointer(&mut ui, PointerKind::Move, 400.0, 300.0);
    assert_eq!(ui.cursor(), Cursor::ResizeColumn);
    pointer(&mut ui, PointerKind::Move, 600.0, 300.0);
    assert_eq!(ui.cursor(), Cursor::ResizeRow);
    pointer(&mut ui, PointerKind::Move, 200.0, 300.0);
    assert_eq!(ui.cursor(), Cursor::Default);
}

#[test]
fn a_panel_added_with_another_joins_that_panels_group() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::Beside(a, DockSide::Right));
    let c = panel(&mut ui, dock, "C", DockAt::With(b));
    ui.layout();
    assert_eq!(area(&ui, dock).tabs_with(b).collect::<Vec<_>>(), [b, c]);
    assert_eq!(rect(&ui, c).left(), 403.0);
}

#[test]
fn a_hovered_tab_lights_up_until_the_pointer_leaves() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    panel(&mut ui, dock, "B", DockAt::With(a));
    ui.layout();
    let plain = frame(&mut ui).len();
    // A's tab is not the shown one: hovering it fills it.
    pointer(&mut ui, PointerKind::Move, 8.0, 14.0);
    assert_eq!(frame(&mut ui).len(), plain + 1);
    pointer(&mut ui, PointerKind::Move, 400.0, 300.0);
    assert_eq!(frame(&mut ui).len(), plain);
    pointer(&mut ui, PointerKind::Move, 8.0, 14.0);
    ui.pointer_left(Point::new(8.0, -1.0));
    assert_eq!(frame(&mut ui).len(), plain, "leaving the window clears it");
}

/// Down, a move and Up of the primary button.
fn press_drag(ui: &mut Ui, from: (f32, f32), to: (f32, f32)) {
    pointer(
        ui,
        PointerKind::Down(PointerButton::Primary),
        from.0,
        from.1,
    );
    pointer(ui, PointerKind::Move, to.0, to.1);
    pointer(ui, PointerKind::Up(PointerButton::Primary), to.0, to.1);
    ui.layout();
}

/// The middle of `panel`'s tab.
fn tab(ui: &Ui, dock: WidgetId, panel: WidgetId) -> (f32, f32) {
    let r = area(ui, dock)
        .tab_rect(panel, rect(ui, dock), ui.scale())
        .unwrap();
    ((r.left() + r.right()) / 2.0, (r.top() + r.bottom()) / 2.0)
}

#[test]
fn removing_the_last_panel_leaves_an_empty_dock_that_takes_panels_again() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    ui.layout();
    DockArea::remove_panel(&mut ui, dock, a).unwrap();
    ui.layout();
    assert_eq!(area(&ui, dock).group_count(), 0);
    let b = panel(&mut ui, dock, "B", DockAt::First);
    ui.layout();
    assert_eq!(rect(&ui, b), Rect::from_edges(0.0, BAR, 800.0, 600.0));
}

#[test]
fn a_panel_keeps_its_scroll_position_while_another_tab_is_shown() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let list = ui.add(a, ScrollView::new(), ScrollView::style()).unwrap();
    let rows: Vec<_> = (0..100)
        .map(|i| {
            let row = Style::row().padding(3.0).shrink(0.0);
            ui.add(list, Label::new(format!("row {i}")), row).unwrap()
        })
        .collect();
    let b = panel(&mut ui, dock, "B", DockAt::With(a));
    DockArea::show_panel(&mut ui, dock, a).unwrap();
    ui.layout();
    let wheel = PointerKind::Scroll(ScrollDelta::Lines(0.0, 3.0));
    pointer(&mut ui, wheel, 300.0, 300.0);
    ui.layout();
    let scrolled = rect(&ui, rows[0]);
    assert!(scrolled.top() < BAR, "the list scrolled");
    let (bx, by) = tab(&ui, dock, b);
    press_drag(&mut ui, (bx, by), (bx, by));
    let (ax, ay) = tab(&ui, dock, a);
    press_drag(&mut ui, (ax, ay), (ax, ay));
    assert_eq!(rect(&ui, rows[0]), scrolled);
}

#[test]
fn tabs_that_do_not_fit_shrink_and_the_shown_one_stays_reachable() {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(300.0, 400.0));
    let dock = ui
        .add(ui.root(), DockArea::new(), DockArea::style())
        .unwrap();
    let first = panel(&mut ui, dock, "Watchlist 0", DockAt::First);
    let mut panels = vec![first];
    for i in 1..9 {
        let title = format!("Watchlist {i}");
        panels.push(panel(&mut ui, dock, &title, DockAt::With(first)));
    }
    ui.layout();
    for &p in &panels {
        DockArea::show_panel(&mut ui, dock, p).unwrap();
        ui.layout();
        let r = area(&ui, dock).tab_rect(p, rect(&ui, dock), 1.0).unwrap();
        assert!(
            r.left() >= 0.0 && r.right() <= 300.0,
            "{r:?} is inside the bar"
        );
        assert!(r.width() >= 40.0, "{r:?} is still a tab");
    }
    // A click on the first tab, while it is in view, shows it.
    DockArea::show_panel(&mut ui, dock, first).unwrap();
    ui.layout();
    let (x, y) = tab(&ui, dock, first);
    DockArea::show_panel(&mut ui, dock, panels[1]).unwrap();
    ui.layout();
    press_drag(&mut ui, (x, y), (x, y));
    assert!(area(&ui, dock).is_shown(first));
}

#[test]
fn releasing_another_button_ends_a_divider_drag() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::Beside(a, DockSide::Right));
    ui.layout();
    pointer(
        &mut ui,
        PointerKind::Down(PointerButton::Primary),
        400.0,
        300.0,
    );
    pointer(
        &mut ui,
        PointerKind::Down(PointerButton::Secondary),
        400.0,
        300.0,
    );
    pointer(
        &mut ui,
        PointerKind::Up(PointerButton::Secondary),
        400.0,
        300.0,
    );
    pointer(&mut ui, PointerKind::Move, 150.0, 300.0);
    pointer(
        &mut ui,
        PointerKind::Up(PointerButton::Primary),
        150.0,
        300.0,
    );
    pointer(&mut ui, PointerKind::Move, 600.0, 300.0);
    ui.layout();
    assert_eq!(rect(&ui, b).left(), 403.0, "the divider did not move");
    pointer(&mut ui, PointerKind::Move, 200.0, 300.0);
    assert_eq!(ui.cursor(), Cursor::Default);
}

#[test]
fn a_panel_removed_from_the_ui_directly_is_forgotten_at_the_next_event() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::With(a));
    ui.layout();
    ui.remove(b);
    pointer(&mut ui, PointerKind::Move, 400.0, 300.0);
    ui.layout();
    assert_eq!(area(&ui, dock).panels().collect::<Vec<_>>(), [a]);
    assert!(area(&ui, dock).is_shown(a));
    assert_eq!(rect(&ui, a), Rect::from_edges(0.0, BAR, 800.0, 600.0));
}

#[test]
fn a_tab_dragged_along_its_bar_changes_places() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::With(a));
    let c = panel(&mut ui, dock, "C", DockAt::With(a));
    ui.layout();
    actions(&mut ui);
    // C before A: onto the left half of A's tab.
    let (cx, cy) = tab(&ui, dock, c);
    let a_tab = area(&ui, dock).tab_rect(a, rect(&ui, dock), 1.0).unwrap();
    press_drag(&mut ui, (cx, cy), (a_tab.left() + 2.0, cy));
    assert_eq!(area(&ui, dock).tabs_with(a).collect::<Vec<_>>(), [c, a, b]);
    assert!(area(&ui, dock).is_shown(c), "the moved tab is shown");
    assert_eq!(actions(&mut ui), [ActionKind::Changed]);
    // C onto its own tab, or just after itself: nothing changes.
    let (cx, cy) = tab(&ui, dock, c);
    press_drag(&mut ui, (cx, cy), (cx + 8.0, cy));
    assert_eq!(area(&ui, dock).tabs_with(a).collect::<Vec<_>>(), [c, a, b]);
    assert!(actions(&mut ui).is_empty());
}

#[test]
fn a_tab_dropped_on_another_groups_tab_lands_beside_that_tab() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::With(a));
    let c = panel(&mut ui, dock, "C", DockAt::Beside(a, DockSide::Right));
    ui.layout();
    let a_tab = area(&ui, dock).tab_rect(a, rect(&ui, dock), 1.0).unwrap();
    let (cx, cy) = tab(&ui, dock, c);
    press_drag(&mut ui, (cx, cy), (a_tab.right() - 2.0, cy));
    assert_eq!(area(&ui, dock).tabs_with(a).collect::<Vec<_>>(), [a, c, b]);
    assert_eq!(area(&ui, dock).group_count(), 1);
}

#[test]
fn removing_another_panel_keeps_a_divider_drag_going() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::Beside(a, DockSide::Right));
    let extra = panel(&mut ui, dock, "Extra", DockAt::With(a));
    ui.layout();
    actions(&mut ui);
    pointer(
        &mut ui,
        PointerKind::Down(PointerButton::Primary),
        400.0,
        300.0,
    );
    DockArea::remove_panel(&mut ui, dock, extra).unwrap();
    pointer(&mut ui, PointerKind::Move, 200.0, 300.0);
    pointer(
        &mut ui,
        PointerKind::Up(PointerButton::Primary),
        200.0,
        300.0,
    );
    ui.layout();
    assert_eq!(rect(&ui, b).left(), 203.0);
    assert_eq!(actions(&mut ui), [ActionKind::Changed]);
}

#[test]
fn a_click_on_a_divider_without_moving_it_reports_no_change() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    panel(&mut ui, dock, "B", DockAt::Beside(a, DockSide::Right));
    ui.layout();
    actions(&mut ui);
    press_drag(&mut ui, (400.0, 300.0), (400.0, 300.0));
    // Grabbed off its middle, it keeps that offset rather than jumping to the pointer.
    press_drag(&mut ui, (402.0, 300.0), (402.0, 310.0));
    assert!(actions(&mut ui).is_empty());
}

#[test]
fn the_tab_bar_and_its_panel_meet_on_a_whole_pixel_at_any_scale() {
    for scale in [1.1, 1.15, 1.2, 1.25, 1.33, 1.5, 1.75, 2.0] {
        let (mut ui, dock) = dock();
        ui.set_scale(scale);
        let a = panel(&mut ui, dock, "A", DockAt::First);
        let b = panel(&mut ui, dock, "B", DockAt::Beside(a, DockSide::Bottom));
        ui.layout();
        for p in [a, b] {
            let top = rect(&ui, p).top();
            let r = area(&ui, dock).tab_rect(p, rect(&ui, dock), scale).unwrap();
            assert_eq!(top, r.bottom(), "scale {scale}: panel top and tab bottom");
            assert_eq!(top, top.round(), "scale {scale}");
        }
    }
}

#[test]
fn leaving_the_dock_puts_the_cursor_back() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    panel(&mut ui, dock, "B", DockAt::Beside(a, DockSide::Right));
    ui.layout();
    pointer(&mut ui, PointerKind::Move, 400.0, 300.0);
    assert_eq!(ui.cursor(), Cursor::ResizeColumn);
    ui.pointer_left(Point::new(400.0, -1.0));
    assert_eq!(ui.cursor(), Cursor::Default);
}

#[test]
fn panels_have_keys_and_titles_that_can_change() {
    let (mut ui, dock) = dock();
    let chart = DockPanel::new("AAPL").with_key("chart");
    let chart = DockArea::add_panel(&mut ui, dock, Panel::new(), chart, DockAt::First).unwrap();
    let notes = panel(&mut ui, dock, "Notes", DockAt::With(chart));
    ui.update::<DockArea, _>(dock, |area, cx| area.set_title(cx, chart, "MSFT"));
    let dock_area = area(&ui, dock);
    assert_eq!(dock_area.key_of(chart), Some("chart"));
    assert_eq!(dock_area.key_of(notes), None);
    assert_eq!(dock_area.panel_with_key("chart"), Some(chart));
    assert_eq!(dock_area.panel_with_key("orders"), None);
    assert_eq!(dock_area.title(chart), Some("MSFT"));
    assert_eq!(
        DockArea::show_panel(&mut ui, dock, dock),
        Err(UiError::NoSuchWidget(dock))
    );
}

#[test]
fn a_tab_dragged_to_a_later_place_lands_there() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::With(a));
    let c = panel(&mut ui, dock, "C", DockAt::With(a));
    ui.layout();
    // A after B: onto the right half of B's tab.
    let (ax, ay) = tab(&ui, dock, a);
    let b_tab = area(&ui, dock).tab_rect(b, rect(&ui, dock), 1.0).unwrap();
    press_drag(&mut ui, (ax, ay), (b_tab.right() - 2.0, ay));
    assert_eq!(area(&ui, dock).tabs_with(a).collect::<Vec<_>>(), [b, a, c]);
    // B to the bar's empty end: last.
    let (bx, by) = tab(&ui, dock, b);
    press_drag(&mut ui, (bx, by), (700.0, by));
    assert_eq!(area(&ui, dock).tabs_with(a).collect::<Vec<_>>(), [a, c, b]);
}

#[test]
fn tabs_that_fit_once_shrunk_are_all_shown() {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(300.0, 400.0));
    let dock = ui
        .add(ui.root(), DockArea::new(), DockArea::style())
        .unwrap();
    let first = panel(&mut ui, dock, "Watchlist 0", DockAt::First);
    let mut panels = vec![first];
    for i in 1..4 {
        let title = format!("Watchlist {i}");
        panels.push(panel(&mut ui, dock, &title, DockAt::With(first)));
    }
    ui.layout();
    let dock_rect = rect(&ui, dock);
    let tabs: Vec<Rect> = panels
        .iter()
        .map(|&p| area(&ui, dock).tab_rect(p, dock_rect, 1.0).unwrap())
        .collect();
    let widths: f32 = tabs.iter().map(Rect::width).sum();
    assert!(widths <= 300.0, "{tabs:?} fit the bar");
    assert!(
        tabs.iter().all(|t| t.width() > 48.0),
        "{tabs:?} shrank evenly"
    );
}

#[test]
fn a_pressed_tab_keeps_being_the_one_dragged_when_another_panel_goes() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::With(a));
    let c = panel(&mut ui, dock, "C", DockAt::With(a));
    ui.layout();
    let (cx, cy) = tab(&ui, dock, c);
    pointer(&mut ui, PointerKind::Down(PointerButton::Primary), cx, cy);
    DockArea::remove_panel(&mut ui, dock, a).unwrap();
    ui.layout();
    // C to the right edge of its group: it, not B, gets a group of its own.
    pointer(&mut ui, PointerKind::Move, 700.0, 300.0);
    pointer(&mut ui, PointerKind::Move, 790.0, 300.0);
    pointer(
        &mut ui,
        PointerKind::Up(PointerButton::Primary),
        790.0,
        300.0,
    );
    ui.layout();
    assert_eq!(area(&ui, dock).tabs_with(c).collect::<Vec<_>>(), [c]);
    assert_eq!(area(&ui, dock).tabs_with(b).collect::<Vec<_>>(), [b]);
}

#[test]
fn removing_a_panel_that_takes_the_held_divider_away_ends_the_drag() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::Beside(a, DockSide::Right));
    ui.layout();
    pointer(
        &mut ui,
        PointerKind::Down(PointerButton::Primary),
        400.0,
        300.0,
    );
    assert_eq!(ui.cursor(), Cursor::ResizeColumn);
    DockArea::remove_panel(&mut ui, dock, b).unwrap();
    assert_eq!(ui.cursor(), Cursor::Default);
    pointer(&mut ui, PointerKind::Move, 200.0, 300.0);
    pointer(
        &mut ui,
        PointerKind::Up(PointerButton::Primary),
        200.0,
        300.0,
    );
    ui.layout();
    assert_eq!(rect(&ui, a), Rect::from_edges(0.0, BAR, 800.0, 600.0));
}

/// Counts how often layout tells it its size.
struct Counted(Rc<Cell<u32>>);

impl Widget for Counted {
    fn layout(&mut self, _: &mut LayoutCx<'_>, _: Size) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn switching_tabs_lays_out_only_the_panels_that_change() {
    let (mut ui, dock) = dock();
    let other = Rc::new(Cell::new(0));
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::With(a));
    let side = DockAt::Beside(a, DockSide::Right);
    DockArea::add_panel(&mut ui, dock, Counted(Rc::clone(&other)), "Other", side).unwrap();
    ui.layout();
    let before = other.get();
    let (ax, ay) = tab(&ui, dock, a);
    press_drag(&mut ui, (ax, ay), (ax, ay));
    let (bx, by) = tab(&ui, dock, b);
    press_drag(&mut ui, (bx, by), (bx, by));
    assert!(area(&ui, dock).is_shown(b));
    assert_eq!(
        other.get(),
        before,
        "the other group's panel was not laid out again"
    );
}

/// Handles every pointer move itself, so its parent never sees them.
struct Sink;

impl Widget for Sink {
    fn event(&mut self, _: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        match event {
            Event::Pointer(p) if p.kind == PointerKind::Move => Handled::Yes,
            _ => Handled::No,
        }
    }
}

#[test]
fn moving_from_a_divider_into_a_panel_puts_the_cursor_back() {
    let (mut ui, dock) = dock();
    let a = DockArea::add_panel(&mut ui, dock, Sink, "A", DockAt::First).unwrap();
    panel(&mut ui, dock, "B", DockAt::Beside(a, DockSide::Right));
    ui.layout();
    pointer(&mut ui, PointerKind::Move, 400.0, 300.0);
    assert_eq!(ui.cursor(), Cursor::ResizeColumn);
    pointer(&mut ui, PointerKind::Move, 200.0, 300.0);
    assert_eq!(ui.cursor(), Cursor::Default);
}

#[test]
fn a_drop_that_changes_nothing_shows_nothing() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    panel(&mut ui, dock, "B", DockAt::With(a));
    ui.layout();
    let still = frame(&mut ui).len();
    let (ax, ay) = tab(&ui, dock, a);
    pointer(&mut ui, PointerKind::Down(PointerButton::Primary), ax, ay);
    // Into the middle of its own group: it is already there.
    pointer(&mut ui, PointerKind::Move, 400.0, 300.0);
    assert_eq!(frame(&mut ui).len(), still);
}

#[test]
fn a_tab_held_over_another_group_lights_that_group_up() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::Beside(a, DockSide::Right));
    ui.layout();
    let (bx, by) = tab(&ui, dock, b);
    pointer(&mut ui, PointerKind::Down(PointerButton::Primary), bx, by);
    pointer(&mut ui, PointerKind::Move, 200.0, 300.0);
    let group = Rect::from_edges(0.0, 0.0, 400.0 - GAP / 2.0, 600.0);
    let fills = frame(&mut ui).iter().filter(|r| **r == group).count();
    // The group's own fill, and the highlight over it.
    assert_eq!(fills, 2);
}

/// A 300 px dock with nine tabs in one group, far more than fit.
fn crowded() -> (Ui, WidgetId, Vec<WidgetId>) {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(300.0, 400.0));
    let dock = ui
        .add(ui.root(), DockArea::new(), DockArea::style())
        .unwrap();
    let first = panel(&mut ui, dock, "Watchlist 0", DockAt::First);
    let mut panels = vec![first];
    for i in 1..9 {
        let title = format!("Watchlist {i}");
        panels.push(panel(&mut ui, dock, &title, DockAt::With(first)));
        ui.layout();
    }
    (ui, dock, panels)
}

#[test]
fn every_tab_can_be_reached_with_clicks_alone() {
    let (mut ui, _, panels) = crowded();
    actions(&mut ui);
    let mut reached = vec![false; panels.len()];
    reached[8] = true; // shown already: added last
                       // Back to the start, then along the bar: click every few pixels, then the forward button.
    for _ in 0..10 {
        press_drag(&mut ui, (269.0, 14.0), (269.0, 14.0));
    }
    for _ in 0..10 {
        for x in (2..258).step_by(6) {
            let x = x as f32;
            press_drag(&mut ui, (x, 14.0), (x, 14.0));
        }
        for kind in actions(&mut ui) {
            if let ActionKind::Selected(i) = kind {
                reached[i] = true;
            }
        }
        press_drag(&mut ui, (289.0, 14.0), (289.0, 14.0));
    }
    assert!(reached.iter().all(|r| *r), "reached {reached:?}");
}

#[test]
fn the_wheel_scrolls_a_crowded_tab_bar() {
    let (mut ui, dock, panels) = crowded();
    let dock_rect = rect(&ui, dock);
    // The newest tab was brought into view, which scrolled the first one out.
    assert!(area(&ui, dock)
        .tab_rect(panels[8], dock_rect, 1.0)
        .is_some());
    assert!(area(&ui, dock)
        .tab_rect(panels[0], dock_rect, 1.0)
        .is_none());
    let back = PointerKind::Scroll(ScrollDelta::Lines(0.0, -20.0));
    pointer(&mut ui, back, 100.0, 14.0);
    ui.layout();
    assert!(area(&ui, dock)
        .tab_rect(panels[0], dock_rect, 1.0)
        .is_some());
    assert!(area(&ui, dock)
        .tab_rect(panels[8], dock_rect, 1.0)
        .is_none());
    // Over a panel, the wheel is not the tab bar's.
    let forward = PointerKind::Scroll(ScrollDelta::Lines(0.0, 20.0));
    pointer(&mut ui, forward, 100.0, 200.0);
    assert!(area(&ui, dock)
        .tab_rect(panels[0], dock_rect, 1.0)
        .is_some());
}

#[test]
fn the_preview_beside_a_group_at_the_docks_edge_is_where_the_group_goes() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "B", DockAt::With(a));
    panel(&mut ui, dock, "C", DockAt::Beside(a, DockSide::Right));
    ui.layout();
    // B to the right edge of the left group, which touches the dock's left edge.
    let (bx, by) = tab(&ui, dock, b);
    pointer(&mut ui, PointerKind::Down(PointerButton::Primary), bx, by);
    pointer(&mut ui, PointerKind::Move, 390.0, 300.0);
    let preview = frame(&mut ui);
    pointer(
        &mut ui,
        PointerKind::Up(PointerButton::Primary),
        390.0,
        300.0,
    );
    ui.layout();
    let landed = rect(&ui, b);
    let group = Rect::from_edges(
        landed.left(),
        landed.top() - BAR,
        landed.right(),
        landed.bottom(),
    );
    assert!(preview.contains(&group), "{group:?} was previewed");
}

#[test]
fn releasing_a_divider_under_the_pointer_keeps_the_resize_cursor() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    panel(&mut ui, dock, "B", DockAt::Beside(a, DockSide::Right));
    ui.layout();
    press_drag(&mut ui, (400.0, 300.0), (300.0, 300.0));
    assert_eq!(ui.cursor(), Cursor::ResizeColumn);
}

#[test]
fn a_tab_whose_panel_was_removed_directly_is_not_drawn() {
    let (mut ui, dock) = dock();
    let a = panel(&mut ui, dock, "A", DockAt::First);
    let b = panel(&mut ui, dock, "Bravo", DockAt::With(a));
    DockArea::show_panel(&mut ui, dock, a).unwrap();
    ui.layout();
    ui.remove(b);
    let before_the_dock_knows = frame(&mut ui).len();
    pointer(&mut ui, PointerKind::Move, 400.0, 300.0);
    assert_eq!(frame(&mut ui).len(), before_the_dock_knows);
}

/// Where tabs end in the crowded dock's bar: its right edge (299, inside the 1 px outline)
/// less the two 20 px scroll buttons.
const CROWDED_TABS_END: f32 = 259.0;

#[test]
fn clicking_a_tab_partly_scrolled_out_brings_it_wholly_into_view() {
    let (mut ui, dock, panels) = crowded();
    let dock_rect = rect(&ui, dock);
    let whole = area(&ui, dock).tab_rect(panels[8], dock_rect, 1.0).unwrap();
    // One notch back cuts the shown tab off at the end of the bar.
    let back = PointerKind::Scroll(ScrollDelta::Lines(0.0, -1.0));
    pointer(&mut ui, back, 100.0, 14.0);
    let cut = area(&ui, dock).tab_rect(panels[8], dock_rect, 1.0).unwrap();
    assert!(cut.width() < whole.width());
    let x = (cut.left() + cut.right()) / 2.0;
    press_drag(&mut ui, (x, 14.0), (x, 14.0));
    let after = area(&ui, dock).tab_rect(panels[8], dock_rect, 1.0).unwrap();
    assert_eq!(after.width(), whole.width());
}

#[test]
fn a_tab_dropped_at_the_end_of_a_crowded_bar_is_in_view() {
    let (mut ui, dock, panels) = crowded();
    let extra = panel(
        &mut ui,
        dock,
        "Extra",
        DockAt::Beside(panels[0], DockSide::Bottom),
    );
    ui.layout();
    let (x, y) = tab(&ui, dock, extra);
    // Onto the crowded group's scroll buttons: after its last tab.
    press_drag(&mut ui, (x, y), (280.0, 14.0));
    assert_eq!(area(&ui, dock).tabs_with(extra).last(), Some(extra));
    assert!(area(&ui, dock)
        .tab_rect(extra, rect(&ui, dock), 1.0)
        .is_some());
}

#[test]
fn a_bar_scrolled_to_its_end_stays_at_its_end_when_tabs_go() {
    let (mut ui, dock, panels) = crowded();
    let dock_rect = rect(&ui, dock);
    let last = area(&ui, dock).tab_rect(panels[8], dock_rect, 1.0).unwrap();
    assert_eq!(last.right(), CROWDED_TABS_END);
    for &gone in &panels[..2] {
        DockArea::remove_panel(&mut ui, dock, gone).unwrap();
    }
    ui.layout();
    let last = area(&ui, dock).tab_rect(panels[8], dock_rect, 1.0).unwrap();
    assert_eq!(last.right(), CROWDED_TABS_END, "no gap after the last tab");
}

#[test]
fn the_shown_tab_stays_in_view_when_the_dock_narrows() {
    // Built before the window has a size, as an app's `build` is.
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    let dock = ui
        .add(ui.root(), DockArea::new(), DockArea::style())
        .unwrap();
    let watch = panel(&mut ui, dock, "Watchlist", DockAt::First);
    let screener = panel(&mut ui, dock, "Screener", DockAt::With(watch));
    DockArea::show_panel(&mut ui, dock, watch).unwrap();
    for width in [1200.0, 80.0] {
        ui.resize(Size::new(width, 400.0));
        ui.layout();
        let dock_rect = rect(&ui, dock);
        assert!(
            area(&ui, dock).tab_rect(watch, dock_rect, 1.0).is_some(),
            "at {width}"
        );
    }
    DockArea::show_panel(&mut ui, dock, screener).unwrap();
    ui.layout();
    assert!(area(&ui, dock)
        .tab_rect(screener, rect(&ui, dock), 1.0)
        .is_some());
}
