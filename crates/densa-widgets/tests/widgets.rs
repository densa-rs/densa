//! The standard widgets driven headless: what they report, how they lay out and paint.

use densa_core::{Point, Rect, Size};
use densa_draw::{Canvas, InstanceBuffer};
use densa_paragraph::TextSystem;
use densa_text::{AtlasConfig, GlyphCache};
use densa_ui::{
    Action, ActionKind, Align, Ime, Key, KeyEvent, Length, Modifiers, NamedKey, PointerButton,
    PointerEvent, PointerKind, ScrollDelta, Style, Ui, WidgetId,
};
use densa_widgets::{Button, Label, Panel, ScrollView, Splitter, TextInput};

fn ui() -> Ui {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(800.0, 600.0));
    ui
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

fn pointer(ui: &mut Ui, kind: PointerKind, x: f32, y: f32) {
    ui.pointer(PointerEvent::new(kind, Point::new(x, y)));
}

fn click(ui: &mut Ui, x: f32, y: f32) {
    pointer(ui, PointerKind::Down(PointerButton::Primary), x, y);
    pointer(ui, PointerKind::Up(PointerButton::Primary), x, y);
}

fn actions(ui: &mut Ui) -> Vec<Action> {
    ui.actions().collect()
}

fn center(ui: &Ui, id: WidgetId) -> (f32, f32) {
    let r = ui.rect(id).unwrap();
    ((r.left() + r.right()) / 2.0, (r.top() + r.bottom()) / 2.0)
}

fn bar(ui: &mut Ui) -> WidgetId {
    let style = Style::row().gap(4.0).padding(4.0).align_items(Align::Start);
    ui.add(ui.root(), Panel::new(), style).unwrap()
}

#[test]
fn a_button_clicks_when_released_over_itself_or_by_enter_and_space() {
    let mut ui = ui();
    let row = bar(&mut ui);
    let ok = ui.add(row, Button::new("OK"), Style::row()).unwrap();
    ui.layout();
    let (x, y) = center(&ui, ok);
    click(&mut ui, x, y);
    assert_eq!(
        actions(&mut ui),
        [Action {
            source: ok,
            kind: ActionKind::Clicked
        }]
    );
    // Pressed, then released elsewhere: no click.
    pointer(&mut ui, PointerKind::Down(PointerButton::Primary), x, y);
    pointer(
        &mut ui,
        PointerKind::Up(PointerButton::Primary),
        700.0,
        500.0,
    );
    assert!(actions(&mut ui).is_empty());
    assert_eq!(ui.focus(), Some(ok), "a press focuses the button");
    for key in [NamedKey::Enter, NamedKey::Space] {
        ui.key(&KeyEvent::down(Key::Named(key)));
    }
    ui.key(&KeyEvent::down(Key::Named(NamedKey::Enter)).repeated());
    assert_eq!(actions(&mut ui).len(), 2, "held keys do not repeat clicks");
}

#[test]
fn a_button_is_its_label_plus_padding_and_shows_focus() {
    let mut ui = ui();
    let row = bar(&mut ui);
    let label = ui.add(row, Label::new("Refresh"), Style::row()).unwrap();
    let button = ui.add(row, Button::new("Refresh"), Style::row()).unwrap();
    let before = frame(&mut ui).len();
    let (label, button) = (ui.rect(label).unwrap(), ui.rect(button).unwrap());
    assert_eq!(button.width(), label.width() + 24.0);
    assert_eq!(button.height(), label.height() + 10.0);
    ui.set_focus(ui.children(row).get(1).copied());
    assert_eq!(
        frame(&mut ui).len(),
        before + 4,
        "a focus ring of four edges"
    );
}

#[test]
fn wrapping_labels_wrap_to_their_width_and_grow_taller() {
    let mut ui = ui();
    let text = "Widgets lay out with flexbox and paint into one draw call per frame.";
    let column = Style::column().width(Length::Px(150.0));
    let column = ui.add(ui.root(), Panel::new(), column).unwrap();
    let one_line = ui.add(column, Label::new("x"), Style::row()).unwrap();
    let wrapped = ui
        .add(column, Label::new(text).wrapping(), Style::row())
        .unwrap();
    // In a row with free space, the panel shrinks to fit and the text wraps rather than
    // overflowing: its smallest width is its longest word.
    let row = ui
        .add(
            ui.root(),
            Panel::new(),
            Style::row().width(Length::Px(200.0)),
        )
        .unwrap();
    let squeezed = Style::column().grow(1.0);
    let squeezed = ui.add(row, Panel::new(), squeezed).unwrap();
    let inner = ui
        .add(squeezed, Label::new(text).wrapping(), Style::row())
        .unwrap();
    ui.layout();
    let (one, many) = (ui.rect(one_line).unwrap(), ui.rect(wrapped).unwrap());
    assert!(many.width() <= 150.0);
    assert!(many.height() >= 3.0 * one.height(), "{many:?} vs {one:?}");
    assert!(ui.rect(inner).unwrap().width() <= 200.0);
    assert!(ui.rect(squeezed).unwrap().width() <= 200.0);
}

fn typing(ui: &mut Ui, text: &str) {
    for ch in text.chars() {
        let mut buffer = [0; 4];
        let s: &str = ch.encode_utf8(&mut buffer);
        ui.key(&KeyEvent::down(Key::Character(s)).with_text(s));
    }
}

fn text_of(ui: &Ui, id: WidgetId) -> String {
    ui.widget::<TextInput>(id).unwrap().text().to_owned()
}

#[test]
fn a_text_input_edits_and_reports_changes_and_submits() {
    let mut ui = ui();
    let row = bar(&mut ui);
    let input = ui.add(row, TextInput::new("ab"), Style::row()).unwrap();
    ui.layout();
    ui.set_focus(Some(input));
    typing(&mut ui, "cd");
    assert_eq!(text_of(&ui, input), "abcd");
    ui.key(&KeyEvent::down(Key::Named(NamedKey::Backspace)));
    ui.key(&KeyEvent::down(Key::Named(NamedKey::ArrowLeft)));
    ui.key(&KeyEvent::down(Key::Named(NamedKey::Delete)));
    assert_eq!(text_of(&ui, input), "ab");
    let changes = actions(&mut ui)
        .iter()
        .filter(|a| a.kind == ActionKind::Changed)
        .count();
    assert_eq!(
        changes, 4,
        "two typed, two deleted; the arrow changes nothing"
    );
    let select_all = KeyEvent::down(Key::Character("a")).with_modifiers(command());
    ui.key(&select_all);
    typing(&mut ui, "z");
    assert_eq!(text_of(&ui, input), "z", "typing replaces the selection");
    ui.key(&KeyEvent::down(Key::Named(NamedKey::Enter)));
    let last = actions(&mut ui).pop().unwrap();
    assert_eq!(last.kind, ActionKind::Submitted);
    // Tab is not the input's: it moves focus on.
    ui.key(&KeyEvent::down(Key::Named(NamedKey::Tab)));
    assert_eq!(text_of(&ui, input), "z");
}

fn command() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers::LOGO
    } else {
        Modifiers::CONTROL
    }
}

#[test]
fn input_methods_compose_in_place_and_commit_once() {
    let mut ui = ui();
    let row = bar(&mut ui);
    let input = ui.add(row, TextInput::new("x"), Style::row()).unwrap();
    ui.layout();
    ui.set_focus(Some(input));
    actions(&mut ui);
    let preedit = Ime::Preedit {
        text: "ni",
        cursor: Some((2, 2)),
    };
    ui.ime(&preedit);
    assert_eq!(text_of(&ui, input), "xni", "shown while composing");
    assert!(actions(&mut ui).is_empty(), "not a change yet");
    ui.ime(&Ime::Commit("你"));
    assert_eq!(text_of(&ui, input), "x你");
    assert_eq!(actions(&mut ui).len(), 1);
}

#[test]
fn clicks_place_the_caret_and_double_clicks_select_a_word() {
    let mut ui = ui();
    let row = bar(&mut ui);
    let input = TextInput::new("alpha beta");
    let input = ui
        .add(row, input, Style::row().width(Length::Px(300.0)))
        .unwrap();
    ui.layout();
    let r = ui.rect(input).unwrap();
    let y = (r.top() + r.bottom()) / 2.0;
    // Far left of the text: the caret goes to its start, so typing prepends.
    click(&mut ui, r.left() + 2.0, y);
    typing(&mut ui, ">");
    assert_eq!(text_of(&ui, input), ">alpha beta");
    let down = PointerKind::Down(PointerButton::Primary);
    let up = PointerKind::Up(PointerButton::Primary);
    let near_end = Point::new(r.right() - 120.0, y);
    for _ in 0..2 {
        ui.pointer(PointerEvent::new(down, near_end));
        ui.pointer(PointerEvent::new(up, near_end));
    }
    typing(&mut ui, "X");
    let text = text_of(&ui, input);
    assert!(
        text == ">alpha X" || text == ">X beta",
        "a word replaced: {text}"
    );
}

#[test]
fn a_scroll_view_shows_a_thumb_and_scrolls_by_wheel_and_drag() {
    let mut ui = ui();
    let frame_style = Style::column().size(200.0, 100.0);
    let holder = ui.add(ui.root(), Panel::new(), frame_style).unwrap();
    let view = ui
        .add(holder, ScrollView::new(), ScrollView::style())
        .unwrap();
    let rows: Vec<WidgetId> = (0..20)
        .map(|i| {
            let style = Style::row().height(Length::Px(20.0)).shrink(0.0);
            ui.add(view, Label::new(format!("row {i}")), style).unwrap()
        })
        .collect();
    let painted = frame(&mut ui);
    let r = ui.rect(view).unwrap();
    // 400 of content in 100 of view: a quarter-height thumb at the right edge.
    let thumb = Rect::new(r.right() - 6.0, r.top(), 6.0, 25.0);
    assert!(
        painted.contains(&thumb),
        "no thumb at {thumb:?} in {painted:?}"
    );
    pointer(
        &mut ui,
        PointerKind::Scroll(ScrollDelta::Lines(0.0, 1.0)),
        50.0,
        50.0,
    );
    ui.layout();
    assert_eq!(
        ui.rect(rows[0]).unwrap().top(),
        r.top() - 48.0,
        "a notch is 48 px"
    );
    // Dragging the thumb to the bottom of the track scrolls to the end.
    let thumb_top = r.top() + 48.0 / 300.0 * 75.0;
    pointer(
        &mut ui,
        PointerKind::Down(PointerButton::Primary),
        r.right() - 3.0,
        thumb_top + 1.0,
    );
    pointer(
        &mut ui,
        PointerKind::Move,
        r.right() - 3.0,
        r.bottom() + 50.0,
    );
    pointer(
        &mut ui,
        PointerKind::Up(PointerButton::Primary),
        r.right() - 3.0,
        r.bottom() + 50.0,
    );
    ui.layout();
    assert_eq!(ui.rect(rows[19]).unwrap().bottom(), r.bottom());
}

#[test]
fn a_splitter_divider_resizes_the_first_pane_within_limits() {
    let mut ui = ui();
    let splitter = Splitter::row();
    let style = splitter.style();
    let split = ui.add(ui.root(), splitter, style).unwrap();
    let left = Style::column().width(Length::Px(200.0)).shrink(0.0);
    let left = ui.add(split, Panel::new(), left).unwrap();
    let right = ui
        .add(split, Panel::new(), Style::column().grow(1.0))
        .unwrap();
    ui.layout();
    let divider = ui.rect(left).unwrap().right() + 3.0;
    assert_eq!(ui.hit_test(Point::new(divider, 100.0)), Some(split));
    pointer(&mut ui, PointerKind::Move, divider, 100.0);
    assert_eq!(ui.cursor(), densa_ui::Cursor::ResizeColumn);
    pointer(
        &mut ui,
        PointerKind::Down(PointerButton::Primary),
        divider,
        100.0,
    );
    pointer(&mut ui, PointerKind::Move, divider + 100.0, 100.0);
    ui.layout();
    assert_eq!(ui.rect(left).unwrap().width(), 300.0);
    pointer(&mut ui, PointerKind::Move, -500.0, 100.0);
    ui.layout();
    assert_eq!(
        ui.rect(left).unwrap().width(),
        48.0,
        "never below the smallest pane"
    );
    pointer(
        &mut ui,
        PointerKind::Up(PointerButton::Primary),
        -500.0,
        100.0,
    );
    assert!(ui.rect(right).unwrap().left() > ui.rect(left).unwrap().right());
}
