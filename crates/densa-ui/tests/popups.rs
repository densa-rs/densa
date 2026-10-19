//! Popups: placed by their anchor over everything, hit first, closed by a press outside
//! (which goes no further unless it is on their opener), keeping Tab inside, giving focus
//! back; and context menu requests.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::{log, move_to, press, release, take, ui, Log, Probe};
use densa_core::{Point, Rect};
use densa_ui::{
    ActionKind, Anchor, Event, EventCx, Handled, Key, KeyEvent, Modifiers, NamedKey, PointerButton,
    PointerEvent, PointerKind, ScrollDelta, Style, Ui, Widget, WidgetId,
};

/// A popup of 100 x 50 that logs what it gets.
fn popup(name: &'static str, log: &Log) -> Probe {
    Probe::new(name, log).sized(100.0, 50.0).handling()
}

/// A widget that opens a popup when pressed, and closes it when pressed again.
struct Opener {
    log: Log,
    opened: Rc<Cell<Option<WidgetId>>>,
}

impl Widget for Opener {
    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        let Event::Pointer(p) = event else {
            return Handled::No;
        };
        if !matches!(p.kind, PointerKind::Down(_)) {
            return Handled::No;
        }
        self.log.borrow_mut().push("opener:down".to_owned());
        match self.opened.get().filter(|id| cx.is_popup_open(*id)) {
            Some(id) => {
                cx.close_popup(id);
                self.opened.set(None);
            }
            None => {
                let menu = popup("menu", &self.log);
                let id = cx
                    .open_popup(menu, Anchor::below(cx.rect()), Style::column())
                    .unwrap();
                self.opened.set(Some(id));
            }
        }
        Handled::Yes
    }
}

/// A focusable popup that closes itself on Escape.
struct Closing;

impl Widget for Closing {
    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        match event {
            Event::Key(key) if key.pressed && key.key == Key::Named(NamedKey::Escape) => {
                cx.close_popup(cx.id());
                Handled::Yes
            }
            _ => Handled::No,
        }
    }

    fn accepts_focus(&self) -> bool {
        true
    }
}

fn below() -> Anchor {
    Anchor::below(Rect::new(10.0, 10.0, 80.0, 20.0))
}

#[test]
fn a_popup_sits_by_its_anchor_over_everything_and_is_hit_first() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let under = ui
        .add(
            ui.root(),
            Probe::new("under", &log),
            Style::column().grow(1.0),
        )
        .unwrap();
    let id = ui
        .open_popup(popup("popup", &log), below(), Style::column())
        .unwrap();
    ui.layout();
    assert_eq!(ui.rect(id), Some(Rect::new(10.0, 30.0, 100.0, 50.0)));
    assert_eq!(ui.hit_test(Point::new(50.0, 50.0)), Some(id));
    assert_eq!(ui.hit_test(Point::new(300.0, 200.0)), Some(under));
    assert!(ui.is_popup_open(id));
    assert_eq!(ui.popups().collect::<Vec<_>>(), [id]);

    // At the bottom-right corner it flips above its anchor and moves left.
    let corner = Anchor::below(Rect::new(350.0, 280.0, 40.0, 20.0));
    let flipped = ui
        .open_popup(popup("low", &log), corner, Style::column())
        .unwrap();
    ui.layout();
    assert_eq!(ui.rect(flipped), Some(Rect::new(300.0, 230.0, 100.0, 50.0)));
}

#[test]
fn a_press_outside_closes_the_popups_and_goes_no_further() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    ui.add(
        ui.root(),
        Probe::new("under", &log).handling(),
        Style::column().grow(1.0),
    )
    .unwrap();
    let first = ui
        .open_popup(popup("one", &log), below(), Style::column())
        .unwrap();
    let second = ui
        .open_popup(
            popup("two", &log),
            Anchor::at(Point::new(200.0, 100.0)),
            Style::column(),
        )
        .unwrap();
    take(&log);
    assert_eq!(press(&mut ui, 300.0, 250.0), Handled::Yes);
    release(&mut ui, 300.0, 250.0);
    assert!(!ui.is_popup_open(first) && !ui.is_popup_open(second));
    assert_eq!(ui.popups().count(), 0);
    let seen = take(&log);
    assert!(
        !seen
            .iter()
            .any(|e| e.starts_with("under:down") || e.starts_with("under:up")),
        "{seen:?}"
    );

    // Now nothing is open: the next press reaches the widget.
    press(&mut ui, 300.0, 250.0);
    assert!(take(&log).contains(&"under:down1".to_owned()));
}

#[test]
fn a_press_inside_a_popup_reaches_it_and_leaves_it_open() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let id = ui
        .open_popup(popup("popup", &log), below(), Style::column())
        .unwrap();
    take(&log);
    press(&mut ui, 50.0, 50.0);
    release(&mut ui, 50.0, 50.0);
    assert!(ui.is_popup_open(id));
    let seen = take(&log);
    assert!(
        seen.contains(&"popup:down1".to_owned()) && seen.contains(&"popup:up".to_owned()),
        "{seen:?}"
    );
}

#[test]
fn moves_outside_still_hover_the_widgets_beneath() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let under = ui
        .add(
            ui.root(),
            Probe::new("under", &log),
            Style::column().grow(1.0),
        )
        .unwrap();
    let id = ui
        .open_popup(popup("popup", &log), below(), Style::column())
        .unwrap();
    move_to(&mut ui, 50.0, 50.0);
    assert_eq!(ui.hovered(), Some(id));
    move_to(&mut ui, 300.0, 250.0);
    assert_eq!(ui.hovered(), Some(under));
    assert!(ui.is_popup_open(id), "a move does not close it");
}

#[test]
fn a_scroll_outside_closes_the_popups() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    ui.add(
        ui.root(),
        Probe::new("under", &log).handling(),
        Style::column().grow(1.0),
    )
    .unwrap();
    let id = ui
        .open_popup(popup("popup", &log), below(), Style::column())
        .unwrap();
    take(&log);
    let scroll = PointerKind::Scroll(ScrollDelta::Lines(0.0, 1.0));
    ui.pointer(PointerEvent::new(scroll, Point::new(300.0, 250.0)));
    assert!(!ui.is_popup_open(id));
    assert!(!take(&log).iter().any(|e| e.starts_with("under:scroll")));
}

#[test]
fn a_press_on_the_opener_closes_the_popup_and_still_reaches_it() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let opened = Rc::new(Cell::new(None));
    let opener = Opener {
        log: Rc::clone(&log),
        opened: Rc::clone(&opened),
    };
    ui.add(ui.root(), opener, Style::column().size(80.0, 20.0))
        .unwrap();
    press(&mut ui, 20.0, 10.0);
    release(&mut ui, 20.0, 10.0);
    let menu = opened.get().expect("opened");
    assert!(ui.is_popup_open(menu));
    ui.layout();
    assert_eq!(
        ui.rect(menu).map(|r| r.top()),
        Some(20.0),
        "below its opener"
    );

    take(&log);
    press(&mut ui, 20.0, 10.0);
    assert!(!ui.is_popup_open(menu));
    assert_eq!(take(&log), ["opener:down"], "the opener got the press");
}

#[test]
fn a_popup_takes_focus_and_gives_it_back_when_it_closes() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let field = ui
        .add(
            ui.root(),
            Probe::new("field", &log).focusable(),
            Style::column().size(50.0, 20.0),
        )
        .unwrap();
    ui.set_focus(Some(field));
    let id = ui
        .open_popup(Closing, below(), Style::column().size(60.0, 40.0))
        .unwrap();
    assert_eq!(ui.focus(), Some(id));
    ui.key(&KeyEvent::down(Key::Named(NamedKey::Escape)));
    assert!(!ui.is_popup_open(id));
    assert_eq!(ui.focus(), Some(field), "focus went back");

    // Closed from outside, with focus elsewhere meanwhile: focus stays where it is.
    let id = ui
        .open_popup(Closing, below(), Style::column().size(60.0, 40.0))
        .unwrap();
    ui.set_focus(Some(field));
    ui.close_popup(id);
    assert_eq!(ui.focus(), Some(field));
}

#[test]
fn tab_stays_inside_the_topmost_popup() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let outside = ui
        .add(
            ui.root(),
            Probe::new("outside", &log).focusable(),
            Style::column().size(50.0, 20.0),
        )
        .unwrap();
    let id = ui
        .open_popup(Probe::new("box", &log), below(), Style::column())
        .unwrap();
    let a = ui
        .add(
            id,
            Probe::new("a", &log).focusable(),
            Style::column().size(40.0, 20.0),
        )
        .unwrap();
    let b = ui
        .add(
            id,
            Probe::new("b", &log).focusable(),
            Style::column().size(40.0, 20.0),
        )
        .unwrap();
    let tab = KeyEvent::down(Key::Named(NamedKey::Tab));
    let mut seen = Vec::new();
    for _ in 0..4 {
        ui.key(&tab);
        seen.push(ui.focus());
    }
    assert_eq!(seen, [Some(a), Some(b), Some(a), Some(b)]);
    ui.close_popups();
    ui.key(&tab);
    assert_eq!(
        ui.focus(),
        Some(outside),
        "back in the window once it closed"
    );
}

#[test]
fn closing_a_popup_closes_those_opened_after_it() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let first = ui
        .open_popup(popup("one", &log), below(), Style::column())
        .unwrap();
    let second = ui
        .open_popup(
            popup("two", &log),
            Anchor::beside(Rect::new(110.0, 30.0, 0.0, 20.0)),
            Style::column(),
        )
        .unwrap();
    let third = ui
        .open_popup(
            popup("three", &log),
            Anchor::at(Point::new(5.0, 5.0)),
            Style::column(),
        )
        .unwrap();
    assert!(ui.close_popup(second));
    assert!(ui.is_popup_open(first));
    assert!(!ui.is_popup_open(second) && !ui.is_popup_open(third));
    assert!(!ui.close_popup(second), "already closed");
    assert!(ui.rect(second).is_none(), "removed");
}

#[test]
fn a_secondary_press_asks_for_a_context_menu() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let plain = ui
        .add(
            ui.root(),
            Probe::new("plain", &log),
            Style::column().size(100.0, 100.0),
        )
        .unwrap();
    let handler = ui
        .add(
            ui.root(),
            Probe::new("handler", &log).handling(),
            Style::column().size(100.0, 100.0),
        )
        .unwrap();
    let secondary = PointerKind::Down(PointerButton::Secondary);
    ui.pointer(PointerEvent::new(secondary, Point::new(20.0, 30.0)));
    ui.pointer(PointerEvent::new(secondary, Point::new(20.0, 130.0)));
    let primary = PointerKind::Down(PointerButton::Primary);
    ui.pointer(PointerEvent::new(primary, Point::new(20.0, 130.0)));
    let asked: Vec<_> = ui
        .actions()
        .filter_map(|action| match action.kind {
            ActionKind::ContextMenu { at } => Some((action.source, at)),
            _ => None,
        })
        .collect();
    assert_eq!(
        asked,
        [
            (plain, Point::new(20.0, 30.0)),
            (handler, Point::new(20.0, 130.0))
        ],
        "the widget under the pointer, or the one that handled the press"
    );
}

#[test]
fn the_context_menu_key_asks_for_the_focused_widget() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    ui.add(
        ui.root(),
        Probe::new("top", &log),
        Style::column().size(100.0, 40.0),
    )
    .unwrap();
    let field = ui
        .add(
            ui.root(),
            Probe::new("field", &log).focusable(),
            Style::column().size(100.0, 40.0),
        )
        .unwrap();
    // Nothing focused: nothing to ask about.
    ui.key(&KeyEvent::down(Key::Named(NamedKey::ContextMenu)));
    assert_eq!(ui.actions().count(), 0);
    ui.set_focus(Some(field));
    ui.key(&KeyEvent::down(Key::Named(NamedKey::ContextMenu)));
    ui.key(&KeyEvent::down(Key::Named(NamedKey::F(10))).with_modifiers(Modifiers::SHIFT));
    ui.key(&KeyEvent::down(Key::Named(NamedKey::F(10))));
    let asked: Vec<_> = ui
        .actions()
        .filter(|a| a.source == field)
        .map(|a| a.kind)
        .collect();
    let at = Point::new(0.0, 40.0);
    assert_eq!(
        asked,
        [
            ActionKind::ContextMenu { at },
            ActionKind::ContextMenu { at }
        ],
        "F10 alone is not the menu key"
    );
}

#[test]
fn popups_are_in_the_accessibility_tree_after_the_window_contents() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let content = ui
        .add(
            ui.root(),
            Probe::new("content", &log),
            Style::column().size(50.0, 50.0),
        )
        .unwrap();
    let id = ui
        .open_popup(popup("popup", &log), below(), Style::column())
        .unwrap();
    let tree = ui.access_tree();
    let window = tree
        .nodes()
        .iter()
        .find(|(i, _)| *i == tree.root())
        .map(|(_, node)| node.children().to_vec())
        .unwrap();
    assert_eq!(
        window,
        [ui.access_id(content).unwrap(), ui.access_id(id).unwrap()]
    );
}

#[test]
fn a_popup_removed_directly_is_no_longer_open() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let id = ui
        .open_popup(popup("popup", &log), below(), Style::column())
        .unwrap();
    assert!(ui.remove(id));
    assert!(!ui.is_popup_open(id));
    assert_eq!(ui.popups().count(), 0);
    // A widget added in its slot is not a popup.
    let reused = ui
        .add(ui.root(), Probe::new("next", &log), Style::column())
        .unwrap();
    assert!(!ui.is_popup_open(reused));
}

/// A popup opened from a widget's event joins the Ui once the event is over.
#[test]
fn a_popup_opened_during_an_event_paints_and_takes_events() {
    let log = log();
    let mut ui: Ui = ui(400.0, 300.0);
    let opened = Rc::new(Cell::new(None));
    ui.add(
        ui.root(),
        Opener {
            log: Rc::clone(&log),
            opened: Rc::clone(&opened),
        },
        Style::column().size(80.0, 20.0),
    )
    .unwrap();
    press(&mut ui, 10.0, 10.0);
    release(&mut ui, 10.0, 10.0);
    let menu = opened.get().unwrap();
    take(&log);
    press(&mut ui, 30.0, 40.0);
    assert_eq!(take(&log), ["menu:enter", "menu:down1"]);
    assert!(ui.is_popup_open(menu));
}

#[test]
fn a_right_click_outside_closes_the_popups_and_asks_for_a_menu_there() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let under = ui
        .add(
            ui.root(),
            Probe::new("under", &log),
            Style::column().grow(1.0),
        )
        .unwrap();
    let id = ui
        .open_popup(popup("popup", &log), below(), Style::column())
        .unwrap();
    let secondary = PointerKind::Down(PointerButton::Secondary);
    // Inside the popup: no context menu over a menu.
    ui.pointer(PointerEvent::new(secondary, Point::new(50.0, 50.0)));
    ui.pointer(PointerEvent::new(
        PointerKind::Up(PointerButton::Secondary),
        Point::new(50.0, 50.0),
    ));
    assert!(ui.is_popup_open(id));
    assert!(!ui
        .actions()
        .any(|a| matches!(a.kind, ActionKind::ContextMenu { .. })));
    // Outside: closed, and a context menu asked for where the press landed.
    ui.pointer(PointerEvent::new(secondary, Point::new(300.0, 250.0)));
    assert!(!ui.is_popup_open(id));
    let asked: Vec<_> = ui.actions().map(|a| (a.source, a.kind)).collect();
    let at = Point::new(300.0, 250.0);
    assert_eq!(asked, [(under, ActionKind::ContextMenu { at })]);
}

#[test]
fn the_context_menu_key_inside_a_popup_asks_for_nothing() {
    let mut ui = ui(400.0, 300.0);
    let id = ui
        .open_popup(Closing, below(), Style::column().size(60.0, 40.0))
        .unwrap();
    assert_eq!(ui.focus(), Some(id));
    ui.key(&KeyEvent::down(Key::Named(NamedKey::ContextMenu)));
    assert_eq!(ui.actions().count(), 0);
}

#[test]
fn removing_a_popup_closes_it_as_closing_does() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let field = ui
        .add(
            ui.root(),
            Probe::new("field", &log).focusable(),
            Style::column().size(50.0, 20.0),
        )
        .unwrap();
    ui.set_focus(Some(field));
    let first = ui
        .open_popup(Closing, below(), Style::column().size(60.0, 40.0))
        .unwrap();
    let second = ui
        .open_popup(
            Closing,
            Anchor::at(Point::new(200.0, 100.0)),
            Style::column().size(60.0, 40.0),
        )
        .unwrap();
    assert_eq!(ui.focus(), Some(second));
    assert!(ui.remove(first));
    assert!(
        !ui.is_popup_open(first) && !ui.is_popup_open(second),
        "and those after it"
    );
    assert!(ui.rect(second).is_none());
    assert_eq!(ui.focus(), Some(field), "focus went back");
}

/// Opens a popup when it gains focus, and closes it when it loses focus.
struct Combo {
    log: Log,
    opened: Rc<Cell<Option<WidgetId>>>,
}

impl Widget for Combo {
    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        match event {
            Event::FocusGained => {
                let list = popup("list", &self.log);
                let anchor = Anchor::below(cx.rect());
                self.opened
                    .set(cx.open_popup(list, anchor, Style::column()).ok());
                Handled::Yes
            }
            Event::FocusLost => {
                if let Some(id) = self.opened.take() {
                    cx.close_popup(id);
                }
                Handled::Yes
            }
            _ => Handled::No,
        }
    }

    fn accepts_focus(&self) -> bool {
        true
    }
}

#[test]
fn popups_opened_and_closed_by_focus_changes_are_taken_in_at_once() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let opened = Rc::new(Cell::new(None));
    let combo = ui
        .add(
            ui.root(),
            Combo {
                log: Rc::clone(&log),
                opened: Rc::clone(&opened),
            },
            Style::column().size(80.0, 20.0),
        )
        .unwrap();
    let other = ui
        .add(
            ui.root(),
            Probe::new("other", &log).focusable(),
            Style::column().size(80.0, 20.0),
        )
        .unwrap();
    // Laid out first: the popup is placed below the combo's rectangle.
    ui.layout();
    ui.set_focus(Some(combo));
    let list = opened.get().expect("opened on focus");
    assert!(ui.widget::<Probe>(list).is_some(), "its widget joined");
    ui.layout();
    assert_eq!(ui.rect(list).map(|r| r.top()), Some(20.0));
    ui.set_focus(Some(other));
    assert!(!ui.is_popup_open(list));
    assert!(ui.rect(list).is_none(), "removed in the same call");
}

/// Swaps its popup for a new one on each press (a menu bar switching menus), and logs the
/// keys its popups pass on.
struct Switcher {
    log: Log,
    opened: Rc<Cell<Option<WidgetId>>>,
    handles_presses: bool,
}

impl Widget for Switcher {
    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        match event {
            Event::Pointer(p) if matches!(p.kind, PointerKind::Down(_)) => {
                if !self.handles_presses && self.opened.get().is_some() {
                    return Handled::No;
                }
                if let Some(id) = self.opened.take() {
                    cx.close_popup(id);
                }
                let anchor = Anchor::below(cx.rect());
                let style = Style::column().size(60.0, 40.0);
                self.opened.set(cx.open_popup(Closing, anchor, style).ok());
                Handled::Yes
            }
            Event::Key(key) => {
                self.log
                    .borrow_mut()
                    .push(format!("switcher:{:?}", key.key));
                Handled::Yes
            }
            _ => Handled::No,
        }
    }
}

fn switcher(
    ui: &mut Ui,
    log: &Log,
    handles_presses: bool,
) -> (WidgetId, Rc<Cell<Option<WidgetId>>>) {
    let opened = Rc::new(Cell::new(None));
    let widget = Switcher {
        log: Rc::clone(log),
        opened: Rc::clone(&opened),
        handles_presses,
    };
    let id = ui
        .add(ui.root(), widget, Style::column().size(80.0, 20.0))
        .unwrap();
    ui.layout();
    (id, opened)
}

#[test]
fn a_popup_swapped_for_another_in_one_event_still_gives_focus_back() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let field = ui
        .add(
            ui.root(),
            Probe::new("field", &log).focusable(),
            Style::column().size(50.0, 20.0),
        )
        .unwrap();
    let (_, opened) = switcher(&mut ui, &log, true);
    ui.set_focus(Some(field));
    press(&mut ui, 10.0, 30.0);
    release(&mut ui, 10.0, 30.0);
    let first = opened.get().unwrap();
    press(&mut ui, 10.0, 30.0);
    release(&mut ui, 10.0, 30.0);
    let second = opened.get().unwrap();
    assert_ne!(first, second);
    assert_eq!(ui.focus(), Some(second));
    ui.key(&KeyEvent::down(Key::Named(NamedKey::Escape)));
    assert_eq!(
        ui.focus(),
        Some(field),
        "not the popup that was swapped away"
    );
}

#[test]
fn an_opener_that_lets_a_press_go_has_its_popups_closed() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let (opener, opened) = switcher(&mut ui, &log, false);
    let y = at(&ui, opener);
    press(&mut ui, 10.0, y);
    release(&mut ui, 10.0, y);
    let popup = opened.get().unwrap();
    assert!(ui.is_popup_open(popup));
    press(&mut ui, 10.0, y);
    assert!(
        !ui.is_popup_open(popup),
        "the press went to the opener, which ignored it"
    );
}

#[test]
fn keys_a_popup_lets_go_reach_its_opener() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    let (opener, opened) = switcher(&mut ui, &log, true);
    let y = at(&ui, opener);
    press(&mut ui, 10.0, y);
    release(&mut ui, 10.0, y);
    assert_eq!(ui.focus(), opened.get());
    take(&log);
    ui.key(&KeyEvent::down(Key::Named(NamedKey::ArrowRight)));
    assert_eq!(take(&log), ["switcher:Named(ArrowRight)"]);
}

#[test]
fn popups_are_painted_over_everything_on_whole_pixels() {
    let log = log();
    let mut ui = ui(400.0, 300.0);
    ui.add(
        ui.root(),
        Probe::new("under", &log),
        Style::column().grow(1.0),
    )
    .unwrap();
    let anchor = Anchor::at(Point::new(10.4, 20.6));
    let id = ui
        .open_popup(popup("popup", &log), anchor, Style::column())
        .unwrap();
    let mut cache = densa_text::GlyphCache::new(densa_text::AtlasConfig::default());
    let mut out = densa_draw::InstanceBuffer::with_capacity(1_024);
    {
        let mut canvas = densa_draw::Canvas::begin(&mut cache, &mut out);
        ui.paint(&mut canvas);
    }
    assert_eq!(ui.widget::<Probe>(id).unwrap().painted, 1);
    let rect = ui.rect(id).unwrap();
    assert_eq!((rect.left(), rect.top()), (10.0, 21.0));
    // The popup's own rectangle is the last thing drawn: over the widget beneath.
    let last = out.as_slice().last().unwrap();
    assert_eq!((last.rect[0], last.rect[1]), (10.0, 21.0));
}

/// The middle of `id`'s rectangle, top to bottom.
fn at(ui: &Ui, id: WidgetId) -> f32 {
    let rect = ui.rect(id).unwrap();
    (rect.top() + rect.bottom()) / 2.0
}
