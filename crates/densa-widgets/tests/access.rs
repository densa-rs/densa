//! The standard widgets to assistive technology: what they say they are, and what they do
//! when asked.

use densa_core::{Point, Size};
use densa_paragraph::TextSystem;
use densa_ui::{
    AccessAction, AccessId, Action, ActionKind, Key, KeyEvent, Modifiers, NamedKey, PointerButton,
    PointerEvent, PointerKind, Role, Style, Ui, WidgetId,
};
use densa_widgets::{
    Button, DockArea, DockAt, DockSide, Label, Panel, ScrollView, Splitter, TextInput,
};

fn ui() -> Ui {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(800.0, 600.0));
    ui
}

fn actions(ui: &mut Ui) -> Vec<Action> {
    ui.actions().collect()
}

#[test]
fn labels_buttons_and_inputs_say_what_they_are_and_what_they_hold() {
    let mut ui = ui();
    let row = ui
        .add(ui.root(), Panel::new(), Style::row().gap(4.0))
        .unwrap();
    let label = ui.add(row, Label::new("Symbol"), Style::row()).unwrap();
    let button = ui.add(row, Button::new("Buy"), Style::row()).unwrap();
    let input = TextInput::new("AAPL").with_placeholder("Ticker");
    let input = ui.add(row, input, Style::row()).unwrap();
    let tree = ui.access_tree();
    let node = |id: WidgetId| tree.node(ui.access_id(id).unwrap()).unwrap();

    assert_eq!(
        (node(label).role(), node(label).value()),
        (Role::Label, Some("Symbol"))
    );
    assert_eq!(
        (node(button).role(), node(button).label()),
        (Role::Button, Some("Buy"))
    );
    assert!(node(button).has_action(AccessAction::Click));
    assert!(node(button).has_action(AccessAction::Focus));
    let field = node(input);
    assert_eq!(
        (
            field.role(),
            field.value(),
            field.placeholder(),
            field.label()
        ),
        (Role::TextInput, Some("AAPL"), Some("Ticker"), None)
    );
    assert!(field.has_action(AccessAction::SetValue));
    assert_eq!(node(row).role(), Role::Group);
}

#[test]
fn a_requested_click_clicks_a_button_and_a_value_replaces_an_inputs_text() {
    let mut ui = ui();
    let button = ui.add(ui.root(), Button::new("Buy"), Style::row()).unwrap();
    let input = ui
        .add(ui.root(), TextInput::new("old"), Style::row())
        .unwrap();
    ui.layout();
    assert!(ui.access_action(ui.access_id(button).unwrap(), AccessAction::Click, None));
    assert!(ui.access_action(
        ui.access_id(input).unwrap(),
        AccessAction::SetValue,
        Some("MSFT")
    ));
    let kinds: Vec<_> = actions(&mut ui)
        .into_iter()
        .map(|a| (a.source, a.kind))
        .collect();
    assert_eq!(
        kinds,
        [(button, ActionKind::Clicked), (input, ActionKind::Changed)]
    );
    assert_eq!(ui.widget::<TextInput>(input).unwrap().text(), "MSFT");
    // A value is not a click, and a click carries no text.
    assert!(!ui.access_action(
        ui.access_id(button).unwrap(),
        AccessAction::SetValue,
        Some("x")
    ));
    assert!(!ui.access_action(ui.access_id(input).unwrap(), AccessAction::SetValue, None));
}

#[test]
fn scroll_views_and_splitters_say_what_they_are() {
    let mut ui = ui();
    let splitter = Splitter::row();
    let style = splitter.style();
    let split = ui.add(ui.root(), splitter, style).unwrap();
    let list = ui
        .add(split, ScrollView::new(), ScrollView::style())
        .unwrap();
    ui.add(split, Panel::new(), Style::column().grow(1.0))
        .unwrap();
    let empty = ui.access_tree();
    let node = empty.node(ui.access_id(list).unwrap()).unwrap();
    assert_eq!(
        node.actions().count(),
        1,
        "nothing to scroll: only scrolling into view"
    );
    for i in 0..200 {
        let row = Style::row().shrink(0.0);
        ui.add(list, Label::new(format!("row {i}")), row).unwrap();
    }
    let tree = ui.access_tree();
    assert_eq!(
        tree.node(ui.access_id(split).unwrap()).unwrap().role(),
        Role::Splitter
    );
    let list = tree.node(ui.access_id(list).unwrap()).unwrap();
    assert_eq!(list.role(), Role::ScrollView);
    assert!(list.has_action(AccessAction::ScrollDown));
    assert!(
        !list.has_action(AccessAction::ScrollUp),
        "already at the top"
    );
}

/// A dock with tabs A and B in one group and C beside them.
fn dock(ui: &mut Ui) -> (WidgetId, [WidgetId; 3]) {
    let dock = ui
        .add(ui.root(), DockArea::new(), DockArea::style())
        .unwrap();
    let add =
        |ui: &mut Ui, title, at| DockArea::add_panel(ui, dock, Panel::new(), title, at).unwrap();
    let a = add(ui, "Alpha", DockAt::First);
    let b = add(ui, "Bravo", DockAt::With(a));
    let c = add(ui, "Charlie", DockAt::Beside(a, DockSide::Right));
    ui.layout();
    (dock, [a, b, c])
}

#[test]
fn a_dock_is_tab_lists_of_tabs_named_by_their_titles() {
    let mut ui = ui();
    let (dock, panels) = dock(&mut ui);
    let tree = ui.access_tree();
    let lists: Vec<AccessId> =
        tree.node(ui.access_id(dock).unwrap()).unwrap().children()[..2].to_vec();
    let tabs = |list: AccessId| -> Vec<(Option<String>, Option<bool>)> {
        let list = tree.node(list).unwrap();
        assert_eq!(list.role(), Role::TabList);
        list.children()
            .iter()
            .map(|t| {
                let tab = tree.node(*t).unwrap();
                assert_eq!(tab.role(), Role::Tab);
                (tab.label().map(str::to_owned), tab.selected())
            })
            .collect()
    };
    let named = |name: &str, shown: bool| (Some(name.to_owned()), Some(shown));
    assert_eq!(
        tabs(lists[0]),
        [named("Alpha", false), named("Bravo", true)]
    );
    assert_eq!(tabs(lists[1]), [named("Charlie", true)]);
    // The panels follow the tab lists, hidden ones left out.
    let children = tree.node(ui.access_id(dock).unwrap()).unwrap().children();
    assert_eq!(
        children.len(),
        4,
        "two tab lists, then the two shown panels"
    );
    let shown = |p: WidgetId| children.contains(&ui.access_id(p).unwrap());
    assert!(!shown(panels[0]) && shown(panels[1]) && shown(panels[2]));
    // A tab's bounds are where it is drawn.
    let alpha = tree
        .node(tree.node(lists[0]).unwrap().children()[0])
        .unwrap();
    let dock_rect = ui.rect(dock).unwrap();
    let drawn = ui
        .widget::<DockArea>(dock)
        .unwrap()
        .tab_rect(panels[0], dock_rect, 1.0)
        .unwrap();
    assert_eq!(alpha.bounds(), drawn);
}

#[test]
fn a_requested_click_on_a_tab_shows_its_panel() {
    let mut ui = ui();
    let (dock, [a, _, _]) = dock(&mut ui);
    actions(&mut ui);
    let tree = ui.access_tree();
    let list = tree.node(ui.access_id(dock).unwrap()).unwrap().children()[0];
    let alpha = tree.node(list).unwrap().children()[0];
    assert!(ui.access_action(alpha, AccessAction::Click, None));
    assert!(ui.widget::<DockArea>(dock).unwrap().is_shown(a));
    let kinds: Vec<_> = actions(&mut ui).into_iter().map(|a| a.kind).collect();
    assert_eq!(kinds, [ActionKind::Selected(0)]);
    // A tab list is not a tab.
    assert!(!ui.access_action(list, AccessAction::Click, None));
}

#[test]
fn a_tab_scrolled_out_of_its_bar_can_be_scrolled_back_into_view() {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(300.0, 400.0));
    let dock = ui
        .add(ui.root(), DockArea::new(), DockArea::style())
        .unwrap();
    let first =
        DockArea::add_panel(&mut ui, dock, Panel::new(), "Watchlist 0", DockAt::First).unwrap();
    for i in 1..9 {
        let title = format!("Watchlist {i}");
        DockArea::add_panel(&mut ui, dock, Panel::new(), title, DockAt::With(first)).unwrap();
    }
    ui.layout();
    let dock_rect = ui.rect(dock).unwrap();
    assert!(
        area(&ui, dock).tab_rect(first, dock_rect, 1.0).is_none(),
        "scrolled out"
    );
    let tree = ui.access_tree();
    let list = tree.node(ui.access_id(dock).unwrap()).unwrap().children()[0];
    let tab = tree.node(list).unwrap().children()[0];
    assert!(ui.access_action(tab, AccessAction::ScrollIntoView, None));
    assert!(area(&ui, dock).tab_rect(first, dock_rect, 1.0).is_some());
    assert!(
        !area(&ui, dock).is_shown(first),
        "scrolling to a tab does not show it"
    );
}

fn area(ui: &Ui, dock: WidgetId) -> &DockArea {
    ui.widget::<DockArea>(dock).unwrap()
}

fn click(ui: &mut Ui, x: f32, y: f32) {
    for kind in [
        PointerKind::Down(PointerButton::Primary),
        PointerKind::Up(PointerButton::Primary),
    ] {
        ui.pointer(PointerEvent::new(kind, Point::new(x, y)));
    }
}

fn press(ui: &mut Ui, key: NamedKey, modifiers: Modifiers) {
    ui.key(&KeyEvent::down(Key::Named(key)).with_modifiers(modifiers));
}

/// The middle of `panel`'s tab.
fn tab_middle(ui: &Ui, dock: WidgetId, panel: WidgetId) -> (f32, f32) {
    let r = area(ui, dock)
        .tab_rect(panel, ui.rect(dock).unwrap(), 1.0)
        .unwrap();
    ((r.left() + r.right()) / 2.0, (r.top() + r.bottom()) / 2.0)
}

fn shown(ui: &Ui, dock: WidgetId) -> Vec<WidgetId> {
    let area = area(ui, dock);
    area.panels().filter(|p| area.is_shown(*p)).collect()
}

#[test]
fn after_a_tab_click_the_arrow_keys_home_and_end_choose_tabs() {
    let mut ui = ui();
    let (dock, [a, b, c]) = dock(&mut ui);
    let (x, y) = tab_middle(&ui, dock, a);
    click(&mut ui, x, y);
    assert_eq!(ui.focus(), Some(dock), "a tab press focuses the tab list");
    assert_eq!(shown(&ui, dock), [a, c]);
    let none = Modifiers::default();
    press(&mut ui, NamedKey::ArrowRight, none);
    assert_eq!(
        shown(&ui, dock),
        [b, c],
        "the clicked group's tabs, not the other's"
    );
    press(&mut ui, NamedKey::ArrowRight, none);
    assert_eq!(shown(&ui, dock), [a, c], "wrapping round");
    press(&mut ui, NamedKey::End, none);
    assert_eq!(shown(&ui, dock), [b, c]);
    press(&mut ui, NamedKey::Home, none);
    assert_eq!(shown(&ui, dock), [a, c]);
    press(&mut ui, NamedKey::ArrowLeft, none);
    assert_eq!(shown(&ui, dock), [b, c]);
    let selected: Vec<_> = actions(&mut ui)
        .into_iter()
        .filter_map(|a| match a.kind {
            ActionKind::Selected(i) => Some(i),
            _ => None,
        })
        .collect();
    assert_eq!(selected, [0, 1, 0, 1, 0, 1]);
}

#[test]
fn a_click_inside_a_panel_does_not_focus_the_dock_but_tab_reaches_it() {
    let mut ui = ui();
    let (dock, [_, b, _]) = dock(&mut ui);
    let panel = ui.rect(b).unwrap();
    click(&mut ui, panel.left() + 20.0, panel.top() + 20.0);
    assert_eq!(ui.focus(), None);
    press(&mut ui, NamedKey::Tab, Modifiers::default());
    assert_eq!(ui.focus(), Some(dock));
}

#[test]
fn ctrl_tab_cycles_the_tabs_of_the_group_holding_focus() {
    let mut ui = ui();
    let (dock, [a, b, c]) = dock(&mut ui);
    // Focus on an input inside B's panel.
    let input = ui.add(b, TextInput::new(""), Style::row()).unwrap();
    ui.set_focus(Some(input));
    press(&mut ui, NamedKey::Tab, Modifiers::CONTROL);
    assert_eq!(shown(&ui, dock), [a, c]);
    assert_eq!(
        ui.focus(),
        Some(dock),
        "focus left the hidden panel for the tab list"
    );
    press(&mut ui, NamedKey::Tab, Modifiers::CONTROL);
    assert_eq!(shown(&ui, dock), [b, c], "still the same group");
    let back = Modifiers::CONTROL | Modifiers::SHIFT;
    press(&mut ui, NamedKey::Tab, back);
    assert_eq!(shown(&ui, dock), [a, c]);
}

#[test]
fn screen_readers_hear_the_focused_tab_and_see_a_focus_ring() {
    let mut ui = ui();
    let (dock, [_, b, _]) = dock(&mut ui);
    let (x, y) = tab_middle(&ui, dock, b);
    click(&mut ui, x, y);
    let tree = ui.access_tree();
    let focus = tree.node(tree.focus()).unwrap();
    assert_eq!(
        (focus.role(), focus.label(), focus.selected()),
        (Role::Tab, Some("Bravo"), Some(true))
    );
    // Without focus, no ring: one rectangle fewer is drawn.
    let frame = |ui: &mut Ui| {
        let mut cache = densa_text::GlyphCache::new(densa_text::AtlasConfig::default());
        let mut out = densa_draw::InstanceBuffer::with_capacity(16_384);
        let mut canvas = densa_draw::Canvas::begin(&mut cache, &mut out);
        ui.paint(&mut canvas);
        out.as_slice().len()
    };
    let focused = frame(&mut ui);
    ui.set_focus(None);
    assert!(frame(&mut ui) < focused);
}

/// A dock with tabs A, B and C in one group and D, E beside them, and an input in each panel.
fn two_groups(ui: &mut Ui) -> (WidgetId, [WidgetId; 5], [WidgetId; 5]) {
    let dock = ui
        .add(ui.root(), DockArea::new(), DockArea::style())
        .unwrap();
    let add =
        |ui: &mut Ui, title, at| DockArea::add_panel(ui, dock, Panel::new(), title, at).unwrap();
    let a = add(ui, "Alpha", DockAt::First);
    let b = add(ui, "Bravo", DockAt::With(a));
    let c = add(ui, "Charlie", DockAt::With(a));
    let d = add(ui, "Delta", DockAt::Beside(a, DockSide::Right));
    let e = add(ui, "Echo", DockAt::With(d));
    let panels = [a, b, c, d, e];
    let inputs = panels.map(|p| ui.add(p, TextInput::new(""), Style::row()).unwrap());
    ui.layout();
    (dock, panels, inputs)
}

#[test]
fn home_goes_to_the_first_tab_of_three() {
    let mut ui = ui();
    let (dock, [a, _, c, _, e], _) = two_groups(&mut ui);
    let (x, y) = tab_middle(&ui, dock, c);
    click(&mut ui, x, y);
    press(&mut ui, NamedKey::Home, Modifiers::default());
    assert_eq!(shown(&ui, dock), [a, e]);
}

#[test]
fn ctrl_tab_acts_on_the_group_whose_panel_has_focus_and_the_arrows_follow() {
    let mut ui = ui();
    let (dock, [_, _, c, d, e], inputs) = two_groups(&mut ui);
    // Focus in Echo's panel, the second group.
    ui.set_focus(Some(inputs[4]));
    press(&mut ui, NamedKey::Tab, Modifiers::CONTROL);
    assert_eq!(
        shown(&ui, dock),
        [c, d],
        "the second group cycled, the first did not"
    );
    assert_eq!(ui.focus(), Some(dock));
    // The arrow keys now move within that group.
    press(&mut ui, NamedKey::ArrowRight, Modifiers::default());
    assert_eq!(shown(&ui, dock), [c, e]);
}

#[test]
fn a_plain_tab_in_a_panel_moves_focus_rather_than_switching_tabs() {
    let mut ui = ui();
    let (dock, [_, _, c, _, e], inputs) = two_groups(&mut ui);
    ui.set_focus(Some(inputs[2]));
    press(&mut ui, NamedKey::Tab, Modifiers::default());
    assert_eq!(shown(&ui, dock), [c, e]);
    assert_ne!(ui.focus(), Some(inputs[2]), "focus moved on");
}

/// The tab nodes of the dock's first tab list: (id, title).
fn tab_nodes(ui: &mut Ui, dock: WidgetId) -> Vec<(AccessId, String)> {
    let tree = ui.access_tree();
    let dock_node = tree.node(ui.access_id(dock).unwrap()).unwrap();
    let list = tree.node(dock_node.children()[0]).unwrap();
    assert!(
        list.clips_children(),
        "tabs scrolled out of the bar are off screen"
    );
    list.children()
        .iter()
        .map(|id| (*id, tree.node(*id).unwrap().label().unwrap().to_owned()))
        .collect()
}

#[test]
fn a_tabs_id_survives_the_removal_of_another_tab() {
    let mut ui = ui();
    let (dock, [a, _, _]) = dock(&mut ui);
    let before = tab_nodes(&mut ui, dock);
    let bravo = before.iter().find(|(_, t)| t == "Bravo").unwrap().0;
    DockArea::remove_panel(&mut ui, dock, a).unwrap();
    let after = tab_nodes(&mut ui, dock);
    assert_eq!(after, [(bravo, "Bravo".to_owned())]);
    // The old Alpha id reaches nothing.
    let alpha = before.iter().find(|(_, t)| t == "Alpha").unwrap().0;
    assert!(!ui.access_action(alpha, AccessAction::Click, None));
    assert!(ui.access_action(bravo, AccessAction::Click, None));
}

#[test]
fn a_new_group_in_a_removed_groups_slot_does_not_take_the_keyboard() {
    let mut ui = ui();
    let (dock, [a, _, c]) = dock(&mut ui);
    let (x, y) = tab_middle(&ui, dock, c);
    click(&mut ui, x, y);
    DockArea::remove_panel(&mut ui, dock, c).unwrap();
    let d = DockArea::add_panel(
        &mut ui,
        dock,
        Panel::new(),
        "Delta",
        DockAt::Beside(a, DockSide::Right),
    )
    .unwrap();
    let e = DockArea::add_panel(&mut ui, dock, Panel::new(), "Echo", DockAt::With(d)).unwrap();
    ui.layout();
    ui.set_focus(Some(dock));
    // The keyboard moves within the first group, not the new one in the old group's slot.
    press(&mut ui, NamedKey::ArrowRight, Modifiers::default());
    assert_eq!(shown(&ui, dock), [a, e]);
}

#[test]
fn altgr_tab_types_rather_than_switching_tabs() {
    let mut ui = ui();
    let (dock, [_, b, c]) = dock(&mut ui);
    let input = ui.add(b, TextInput::new(""), Style::row()).unwrap();
    ui.set_focus(Some(input));
    // AltGr is Ctrl+Alt on Windows.
    press(&mut ui, NamedKey::Tab, Modifiers::CONTROL | Modifiers::ALT);
    assert_eq!(shown(&ui, dock), [b, c]);
}
