//! The accessibility tree and assistive technology's requests.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{log, ui, Probe};
use densa_core::Rect;
use densa_ui::{
    AccessAction, AccessId, AccessNode, AccessRequest, EventCx, Handled, Overflow, PaintCx, Role,
    Style, Widget,
};

/// The requests a widget got: action, part and text.
type Requests = Rc<RefCell<Vec<(AccessAction, Option<u32>, Option<String>)>>>;

/// A widget with two parts (one inside the other), recording the requests it gets.
struct Parted {
    requests: Requests,
}

impl Widget for Parted {
    fn accessibility(&self, cx: &PaintCx<'_>, node: &mut AccessNode) {
        node.set_role(Role::TabList);
        node.set_label("tabs");
        let mut outer = AccessNode::new(Role::Tab);
        outer.set_label("outer");
        outer.set_bounds(Rect::new(cx.rect().left(), cx.rect().top(), 10.0, 10.0));
        outer.add_action(AccessAction::Click);
        let mut inner = AccessNode::new(Role::Label);
        inner.set_value("inner");
        outer.push_part(7, inner);
        node.push_part(3, outer);
    }

    fn access_action(&mut self, _: &mut EventCx<'_>, request: &AccessRequest<'_>) -> Handled {
        let value = request.value.map(str::to_owned);
        self.requests
            .borrow_mut()
            .push((request.action, request.part, value));
        Handled::Yes
    }
}

#[test]
fn the_tree_holds_shown_widgets_and_their_parts_in_order() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let requests = Rc::default();
    let parted = ui
        .add(
            ui.root(),
            Parted { requests },
            Style::row().size(100.0, 40.0),
        )
        .unwrap();
    let plain = ui
        .add(
            ui.root(),
            Probe::new("plain", &log),
            Style::row().size(50.0, 20.0),
        )
        .unwrap();
    let hidden = Style::row().size(50.0, 20.0).hidden(true);
    let gone = ui.add(ui.root(), Probe::new("gone", &log), hidden).unwrap();
    let tree = ui.access_tree();

    let root = tree.node(tree.root()).unwrap();
    assert_eq!(root.role(), Role::Window);
    assert_eq!(tree.focus(), tree.root(), "nothing has focus");
    let (p, q) = (ui.access_id(parted).unwrap(), ui.access_id(plain).unwrap());
    assert_eq!(root.children(), [p, q], "hidden widgets are left out");
    assert!(tree.node(ui.access_id(gone).unwrap()).is_none());

    let node = tree.node(p).unwrap();
    assert_eq!((node.role(), node.label()), (Role::TabList, Some("tabs")));
    assert_eq!(node.bounds(), ui.rect(parted).unwrap());
    let outer = ui.access_id(parted).unwrap().part(3);
    assert_eq!(node.children(), [outer], "parts come first");
    let outer_node = tree.node(outer).unwrap();
    assert_eq!(outer_node.label(), Some("outer"));
    assert!(outer_node.has_action(AccessAction::Click));
    let inner = ui.access_id(parted).unwrap().part(7);
    assert_eq!(outer_node.children(), [inner]);
    assert_eq!(tree.node(inner).unwrap().value(), Some("inner"));

    let plain_node = tree.node(q).unwrap();
    assert_eq!(plain_node.role(), Role::Group, "the default");
    assert_eq!(plain_node.bounds(), ui.rect(plain).unwrap());
    // Ids survive a trip through a platform's numbers, and parents come before children.
    assert_eq!(AccessId::from_raw(inner.to_raw()), inner);
    let at = |id| tree.nodes().iter().position(|(i, _)| *i == id).unwrap();
    assert!(at(tree.root()) < at(p) && at(p) < at(outer) && at(outer) < at(inner));
}

#[test]
fn requests_reach_the_widget_with_their_part_and_text() {
    let mut ui = ui(400.0, 300.0);
    let requests: Requests = Rc::default();
    let parted = Parted {
        requests: Rc::clone(&requests),
    };
    let parted = ui
        .add(ui.root(), parted, Style::row().size(100.0, 40.0))
        .unwrap();
    ui.layout();
    assert!(ui.access_action(
        ui.access_id(parted).unwrap().part(3),
        AccessAction::Click,
        None
    ));
    let id = ui.access_id(parted).unwrap();
    assert!(ui.access_action(id, AccessAction::SetValue, Some("typed")));
    assert_eq!(
        *requests.borrow(),
        [
            (AccessAction::Click, Some(3), None),
            (AccessAction::SetValue, None, Some("typed".to_owned()))
        ]
    );
}

#[test]
fn focus_is_given_to_widgets_that_take_it_and_shows_in_the_tree() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let field = Probe::new("field", &log).focusable();
    let field = ui
        .add(ui.root(), field, Style::row().size(80.0, 20.0))
        .unwrap();
    let inert = ui
        .add(
            ui.root(),
            Probe::new("inert", &log),
            Style::row().size(80.0, 20.0),
        )
        .unwrap();
    let tree = ui.access_tree();
    assert!(tree
        .node(ui.access_id(field).unwrap())
        .unwrap()
        .has_action(AccessAction::Focus));
    assert!(!tree
        .node(ui.access_id(inert).unwrap())
        .unwrap()
        .has_action(AccessAction::Focus));
    assert!(!ui.access_action(ui.access_id(inert).unwrap(), AccessAction::Focus, None));
    assert!(ui.access_action(ui.access_id(field).unwrap(), AccessAction::Focus, None));
    assert_eq!(ui.focus(), Some(field));
    assert_eq!(ui.access_tree().focus(), ui.access_id(field).unwrap());
}

/// A scrolling column of `rows` 30 px rows in a 100 px frame.
fn scroller(ui: &mut densa_ui::Ui, rows: usize) -> (densa_ui::WidgetId, Vec<densa_ui::WidgetId>) {
    let log = log();
    let frame = Style::column()
        .size(100.0, 100.0)
        .overflow(Overflow::Scroll);
    let frame = ui.add(ui.root(), Probe::new("frame", &log), frame).unwrap();
    let rows = (0..rows)
        .map(|_| {
            let row = Style::row().size(100.0, 30.0).shrink(0.0);
            ui.add(frame, Probe::new("row", &log), row).unwrap()
        })
        .collect();
    ui.layout();
    (frame, rows)
}

#[test]
fn scroll_requests_scroll_the_nearest_scrolling_widget() {
    let mut ui = ui(400.0, 300.0);
    let (frame, rows) = scroller(&mut ui, 10);
    // Asked of a row, the frame around it scrolls.
    assert!(ui.access_action(
        ui.access_id(rows[0]).unwrap(),
        AccessAction::ScrollDown,
        None
    ));
    ui.layout();
    assert_eq!(ui.rect(rows[0]).unwrap().top(), -48.0);
    assert!(ui.access_action(ui.access_id(frame).unwrap(), AccessAction::ScrollUp, None));
    ui.layout();
    assert_eq!(ui.rect(rows[0]).unwrap().top(), 0.0);
    let tree = ui.access_tree();
    assert!(tree
        .node(ui.access_id(frame).unwrap())
        .unwrap()
        .clips_children());
    assert!(!tree
        .node(ui.access_id(rows[0]).unwrap())
        .unwrap()
        .clips_children());
    // At the top already: nothing moves.
    assert!(!ui.access_action(ui.access_id(frame).unwrap(), AccessAction::ScrollUp, None));
}

#[test]
fn scrolling_into_view_brings_a_widget_inside_its_scrolling_parent() {
    let mut ui = ui(400.0, 300.0);
    let (frame, rows) = scroller(&mut ui, 10);
    assert!(ui.access_action(
        ui.access_id(rows[6]).unwrap(),
        AccessAction::ScrollIntoView,
        None
    ));
    ui.layout();
    let view = ui.rect(frame).unwrap();
    let row = ui.rect(rows[6]).unwrap();
    assert_eq!(row.bottom(), view.bottom(), "just far enough to show it");
    assert!(ui.access_action(
        ui.access_id(rows[0]).unwrap(),
        AccessAction::ScrollIntoView,
        None
    ));
    ui.layout();
    assert_eq!(ui.rect(rows[0]).unwrap().top(), view.top());
}

#[test]
fn requests_for_removed_widgets_or_unhandled_actions_do_nothing() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let gone = ui
        .add(
            ui.root(),
            Probe::new("gone", &log),
            Style::row().size(10.0, 10.0),
        )
        .unwrap();
    let gone_id = ui.access_id(gone).unwrap();
    ui.remove(gone);
    assert!(ui.access_id(gone).is_none());
    assert!(!ui.access_action(gone_id, AccessAction::Click, None));
    let inert = ui
        .add(
            ui.root(),
            Probe::new("inert", &log),
            Style::row().size(10.0, 10.0),
        )
        .unwrap();
    assert!(!ui.access_action(ui.access_id(inert).unwrap(), AccessAction::Click, None));
    let nowhere = AccessId::from_raw(u64::MAX);
    assert!(!ui.access_action(nowhere, AccessAction::Click, None));
}

/// Takes focus by Tab but not from presses inside it.
struct TabOnly;

impl Widget for TabOnly {
    fn accepts_focus(&self) -> bool {
        true
    }

    fn focus_on_press(&self) -> bool {
        false
    }
}

#[test]
fn a_widget_can_take_focus_by_tab_but_not_from_presses_inside_it() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let outer = ui
        .add(ui.root(), TabOnly, Style::column().size(200.0, 200.0))
        .unwrap();
    ui.add(
        outer,
        Probe::new("inner", &log),
        Style::row().size(50.0, 50.0),
    )
    .unwrap();
    common::press(&mut ui, 10.0, 10.0);
    assert_eq!(ui.focus(), None, "a press inside does not focus it");
    common::press(&mut ui, 150.0, 150.0);
    assert_eq!(ui.focus(), None, "nor a press on it");
    let tab = densa_ui::KeyEvent::down(densa_ui::Key::Named(densa_ui::NamedKey::Tab));
    ui.key(&tab);
    assert_eq!(ui.focus(), Some(outer), "Tab reaches it");
}

#[test]
fn a_widget_taller_than_the_view_scrolls_in_at_its_top() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let frame = Style::column()
        .size(100.0, 100.0)
        .overflow(Overflow::Scroll);
    let frame = ui.add(ui.root(), Probe::new("frame", &log), frame).unwrap();
    let rows: Vec<_> = [30.0, 30.0, 30.0, 150.0, 30.0]
        .into_iter()
        .map(|height| {
            let row = Style::row().size(100.0, height).shrink(0.0);
            ui.add(frame, Probe::new("row", &log), row).unwrap()
        })
        .collect();
    ui.layout();
    assert!(ui.access_action(
        ui.access_id(rows[3]).unwrap(),
        AccessAction::ScrollIntoView,
        None
    ));
    ui.layout();
    assert_eq!(
        ui.rect(rows[3]).unwrap().top(),
        ui.rect(frame).unwrap().top()
    );
}

#[test]
fn a_removed_widget_is_not_scrolled_into_view() {
    let mut ui = ui(400.0, 300.0);
    let (_, rows) = scroller(&mut ui, 10);
    let id = ui.access_id(rows[6]).unwrap();
    ui.remove(rows[6]);
    assert!(!ui.access_action(id, AccessAction::ScrollIntoView, None));
}

/// Counts the requests it handles.
#[derive(Default)]
struct Counter(Rc<RefCell<u32>>);

impl Widget for Counter {
    fn accessibility(&self, _: &PaintCx<'_>, node: &mut AccessNode) {
        node.set_role(Role::Button);
        node.add_action(AccessAction::Click);
    }

    fn access_action(&mut self, _: &mut EventCx<'_>, _: &AccessRequest<'_>) -> Handled {
        *self.0.borrow_mut() += 1;
        Handled::Yes
    }
}

#[test]
fn a_hidden_widget_is_never_acted_on() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let page = ui
        .add(ui.root(), Probe::new("page", &log), Style::column())
        .unwrap();
    let count = Rc::new(RefCell::new(0));
    let sell = Counter(Rc::clone(&count));
    let sell = ui.add(page, sell, Style::row().size(40.0, 20.0)).unwrap();
    let field = Probe::new("field", &log).focusable();
    let field = ui
        .add(ui.root(), field, Style::row().size(40.0, 20.0))
        .unwrap();
    ui.set_focus(Some(field));
    let (sell_id, page_id) = (ui.access_id(sell).unwrap(), ui.access_id(page).unwrap());
    ui.set_style(page, Style::column().hidden(true));
    assert!(!ui.access_action(sell_id, AccessAction::Click, None));
    assert!(!ui.access_action(page_id, AccessAction::Focus, None));
    assert_eq!(*count.borrow(), 0, "the hidden button was not pressed");
    assert_eq!(ui.focus(), Some(field), "focus stayed where it was");
}

#[test]
fn an_id_of_a_removed_widget_does_not_reach_the_widget_in_its_slot() {
    let mut ui = ui(400.0, 300.0);
    let first = Rc::new(RefCell::new(0));
    let old = ui
        .add(
            ui.root(),
            Counter(Rc::clone(&first)),
            Style::row().size(40.0, 20.0),
        )
        .unwrap();
    let old_id = ui.access_id(old).unwrap();
    ui.remove(old);
    let second = Rc::new(RefCell::new(0));
    let new = ui
        .add(
            ui.root(),
            Counter(Rc::clone(&second)),
            Style::row().size(40.0, 20.0),
        )
        .unwrap();
    // The freed slot is reused for the new widget; only its id tells them apart.
    assert_ne!(ui.access_id(new).unwrap(), old_id);
    assert!(!ui.access_action(old_id, AccessAction::Click, None));
    assert_eq!(*second.borrow(), 0);
}

/// Adds parts that break the rules: a repeated number, and one past the last.
struct Unruly {
    focus: u32,
}

impl Widget for Unruly {
    fn accessibility(&self, _: &PaintCx<'_>, node: &mut AccessNode) {
        let mut first = AccessNode::new(Role::Tab);
        first.set_label("first");
        let mut again = AccessNode::new(Role::Tab);
        again.set_label("again");
        first.push_part(1, again);
        node.push_part(1, first);
        node.push_part(densa_ui::MAX_PART + 1, AccessNode::new(Role::Tab));
        node.push_part(densa_ui::MAX_PART, AccessNode::new(Role::Tab));
        node.set_focused_part(self.focus);
    }

    fn accepts_focus(&self) -> bool {
        true
    }
}

#[test]
fn parts_that_would_share_an_id_are_left_out_and_focus_stays_on_a_real_node() {
    let mut ui = ui(400.0, 300.0);
    let widget = ui
        .add(
            ui.root(),
            Unruly { focus: 99 },
            Style::row().size(40.0, 20.0),
        )
        .unwrap();
    ui.set_focus(Some(widget));
    let tree = ui.access_tree();
    let id = ui.access_id(widget).unwrap();
    let mut ids: Vec<_> = tree.nodes().iter().map(|(i, _)| *i).collect();
    let count = ids.len();
    ids.sort_by_key(|i| i.to_raw());
    ids.dedup();
    assert_eq!(ids.len(), count, "every id is unique");
    let node = tree.node(id).unwrap();
    assert_eq!(node.children(), [id.part(1), id.part(densa_ui::MAX_PART)]);
    assert!(
        tree.node(id.part(1)).unwrap().children().is_empty(),
        "the repeat is left out"
    );
    assert_eq!(id.part(densa_ui::MAX_PART + 1), id, "no part past the last");
    assert_eq!(id.part(u32::MAX), id);
    assert_eq!(
        tree.focus(),
        id,
        "part 99 does not exist: the widget has focus"
    );
}
