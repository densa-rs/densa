//! Widget behaviour pinned after review: typed spaces and AltGr, clicks that belong to the
//! button, dividers that start drags, caret and scroll in text inputs, scroll view details.

use densa_core::{Point, Rect, Size};
use densa_draw::{Canvas, InstanceBuffer};
use densa_paragraph::TextSystem;
use densa_text::{AtlasConfig, GlyphCache};
use densa_ui::{
    ActionKind, Align, Ime, Key, KeyEvent, Length, Modifiers, NamedKey, PointerButton,
    PointerEvent, PointerKind, ScrollDelta, Style, Ui, WidgetId,
};
use densa_widgets::{Button, Label, Panel, ScrollView, Splitter, TextInput};

fn ui() -> Ui {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(800.0, 600.0));
    ui
}

fn pointer(ui: &mut Ui, kind: PointerKind, x: f32, y: f32) {
    ui.pointer(PointerEvent::new(kind, Point::new(x, y)));
}

const DOWN: PointerKind = PointerKind::Down(PointerButton::Primary);
const UP: PointerKind = PointerKind::Up(PointerButton::Primary);

fn click(ui: &mut Ui, x: f32, y: f32) {
    pointer(ui, DOWN, x, y);
    pointer(ui, UP, x, y);
}

fn frame(ui: &mut Ui) -> Vec<Rect> {
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(16_384);
    ui.paint(&mut Canvas::begin(&mut cache, &mut out));
    out.as_slice()
        .iter()
        .map(|i| Rect::new(i.rect[0], i.rect[1], i.rect[2], i.rect[3]))
        .collect()
}

fn input(ui: &mut Ui, text: &str, width: f32) -> WidgetId {
    let bar = Style::row().padding(4.0).align_items(Align::Start);
    let bar = ui.add(ui.root(), Panel::new(), bar).unwrap();
    let style = Style::row().width(Length::Px(width));
    let id = ui.add(bar, TextInput::new(text), style).unwrap();
    ui.layout();
    ui.set_focus(Some(id));
    id
}

fn text_of(ui: &Ui, id: WidgetId) -> String {
    ui.widget::<TextInput>(id).unwrap().text().to_owned()
}

fn type_char(ui: &mut Ui, s: &str) {
    ui.key(&KeyEvent::down(Key::Character(s)).with_text(s));
}

#[test]
fn space_types_a_space_and_altgr_types_its_character() {
    let mut ui = ui();
    let id = input(&mut ui, "ab", 200.0);
    ui.key(&KeyEvent::down(Key::Named(NamedKey::Space)).with_text(" "));
    // AltGr+Q types @ on a German layout: Ctrl+Alt on Windows, not a shortcut.
    let altgr = Modifiers::CONTROL | Modifiers::ALT;
    ui.key(
        &KeyEvent::down(Key::Character("@"))
            .with_text("@")
            .with_modifiers(altgr),
    );
    // A plain shortcut types nothing.
    let shortcut = KeyEvent::down(Key::Character("s")).with_text("s");
    ui.key(&shortcut.with_modifiers(command()));
    assert_eq!(text_of(&ui, id), "ab @");
}

fn command() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers::LOGO
    } else {
        Modifiers::CONTROL
    }
}

#[test]
fn a_press_elsewhere_released_on_a_button_does_not_click_it() {
    let mut ui = ui();
    let bar = ui
        .add(ui.root(), Panel::new(), Style::row().padding(4.0))
        .unwrap();
    let ok = ui.add(bar, Button::new("OK"), Style::row()).unwrap();
    ui.layout();
    let r = ui.rect(ok).unwrap();
    pointer(&mut ui, DOWN, 700.0, 500.0);
    pointer(&mut ui, UP, r.left() + 2.0, r.top() + 2.0);
    assert_eq!(ui.actions().count(), 0);
}

#[test]
fn pressing_inside_a_pane_does_not_move_the_divider() {
    let mut ui = ui();
    let splitter = Splitter::row();
    let style = splitter.style();
    let split = ui.add(ui.root(), splitter, style).unwrap();
    let left = Style::column().width(Length::Px(200.0)).shrink(0.0);
    let left = ui.add(split, Panel::new(), left).unwrap();
    ui.add(left, Label::new("pane text"), Style::row()).unwrap();
    ui.add(split, Panel::new(), Style::column().grow(1.0))
        .unwrap();
    ui.layout();
    pointer(&mut ui, DOWN, 100.0, 300.0);
    pointer(&mut ui, PointerKind::Move, 160.0, 300.0);
    pointer(&mut ui, UP, 160.0, 300.0);
    ui.layout();
    assert_eq!(ui.rect(left).unwrap().width(), 200.0);
    // A drag cancelled half way stops following the pointer.
    let divider = ui.rect(left).unwrap().right() + 3.0;
    pointer(&mut ui, DOWN, divider, 300.0);
    ui.cancel_pointer();
    pointer(&mut ui, PointerKind::Move, divider + 80.0, 300.0);
    ui.layout();
    assert_eq!(ui.rect(left).unwrap().width(), 200.0);
}

#[test]
fn the_caret_stays_in_view_and_new_text_starts_unscrolled() {
    let mut ui = ui();
    let id = input(&mut ui, "", 100.0);
    for _ in 0..40 {
        type_char(&mut ui, "m");
    }
    let r = ui.rect(id).unwrap();
    let y = (r.top() + r.bottom()) / 2.0;
    // The end of the text is in view: a click at the right edge lands there.
    click(&mut ui, r.right() - 8.0, y);
    type_char(&mut ui, "X");
    assert!(text_of(&ui, id).ends_with("mX"), "{}", text_of(&ui, id));
    // Short text again: nothing to scroll, a click at the left edge lands at the start.
    ui.update::<TextInput, _>(id, |input, cx| input.set_text(cx, "a").unwrap());
    click(&mut ui, r.left() + 3.0, y);
    type_char(&mut ui, ">");
    assert_eq!(text_of(&ui, id), ">a");
}

#[test]
fn shift_click_ctrl_arrows_and_cancelled_compositions_edit_as_expected() {
    let mut ui = ui();
    let id = input(&mut ui, "alpha beta", 300.0);
    ui.key(&KeyEvent::down(Key::Named(NamedKey::ArrowLeft)).with_modifiers(command()));
    type_char(&mut ui, "X");
    assert_eq!(
        text_of(&ui, id),
        "alpha Xbeta",
        "Ctrl/Cmd+Left: a word back"
    );
    let preedit = Ime::Preedit {
        text: "ni",
        cursor: Some((2, 2)),
    };
    ui.ime(&preedit);
    ui.ime(&Ime::Disabled);
    assert_eq!(
        text_of(&ui, id),
        "alpha Xbeta",
        "a cancelled composition leaves nothing"
    );
    let r = ui.rect(id).unwrap();
    let y = (r.top() + r.bottom()) / 2.0;
    click(&mut ui, r.left() + 3.0, y);
    let shift =
        PointerEvent::new(DOWN, Point::new(r.right() - 3.0, y)).with_modifiers(Modifiers::SHIFT);
    ui.pointer(shift);
    pointer(&mut ui, UP, r.right() - 3.0, y);
    type_char(&mut ui, "Z");
    assert_eq!(text_of(&ui, id), "Z", "shift-click selected everything");
    let changed = ui
        .actions()
        .filter(|a| a.kind == ActionKind::Changed)
        .count();
    assert!(changed >= 2);
}

fn scroll_view(ui: &mut Ui, rows: usize) -> (WidgetId, Vec<WidgetId>) {
    let holder = ui
        .add(ui.root(), Panel::new(), Style::column().size(200.0, 100.0))
        .unwrap();
    let view = ui
        .add(holder, ScrollView::new(), ScrollView::style())
        .unwrap();
    let rows = (0..rows)
        .map(|i| {
            let style = Style::row().height(Length::Px(20.0)).shrink(0.0);
            ui.add(view, Label::new(format!("row {i}")), style).unwrap()
        })
        .collect();
    (view, rows)
}

#[test]
fn a_wheel_notch_scales_and_a_long_list_keeps_a_grabbable_thumb() {
    let mut ui = ui();
    ui.set_scale(2.0);
    let (view, rows) = scroll_view(&mut ui, 2_000);
    let painted = frame(&mut ui);
    let r = ui.rect(view).unwrap();
    // 40,000 px of content in 200: the thumb would be a pixel; it stays 24 logical px.
    let thumb = Rect::new(r.right() - 12.0, r.top(), 12.0, 48.0);
    assert!(painted.contains(&thumb), "no {thumb:?}");
    pointer(
        &mut ui,
        PointerKind::Scroll(ScrollDelta::Lines(0.0, 1.0)),
        r.left() + 5.0,
        r.top() + 5.0,
    );
    ui.layout();
    assert_eq!(
        ui.rect(rows[0]).unwrap().top(),
        r.top() - 96.0,
        "48 logical px at scale 2"
    );
}

#[test]
fn a_released_thumb_stops_scrolling() {
    let mut ui = ui();
    let (view, rows) = scroll_view(&mut ui, 20);
    ui.layout();
    let r = ui.rect(view).unwrap();
    pointer(&mut ui, DOWN, r.right() - 3.0, r.top() + 2.0);
    pointer(&mut ui, PointerKind::Move, r.right() - 3.0, r.top() + 20.0);
    pointer(&mut ui, UP, r.right() - 3.0, r.top() + 20.0);
    ui.layout();
    let after_drag = ui.rect(rows[0]).unwrap().top();
    assert!(after_drag < r.top());
    pointer(&mut ui, PointerKind::Move, r.right() - 3.0, r.bottom());
    ui.layout();
    assert_eq!(ui.rect(rows[0]).unwrap().top(), after_drag);
}

#[test]
fn a_centered_label_sits_in_the_middle() {
    let mut ui = ui();
    let row = ui
        .add(ui.root(), Panel::transparent(), Style::row())
        .unwrap();
    let style = Style::row().width(Length::Px(400.0));
    let left = ui.add(row, Label::new("ok"), style.clone()).unwrap();
    let painted = frame(&mut ui);
    let first_left = painted
        .iter()
        .skip(1)
        .map(|r| r.left())
        .fold(f32::MAX, f32::min);
    ui.remove(left);
    ui.add(row, Label::new("ok").centered(), style).unwrap();
    let painted = frame(&mut ui);
    let first_centered = painted
        .iter()
        .skip(1)
        .map(|r| r.left())
        .fold(f32::MAX, f32::min);
    assert!(
        first_left < 10.0 && first_centered > 180.0,
        "{first_left} {first_centered}"
    );
}

#[test]
fn a_widened_input_scrolls_back_to_show_its_start() {
    let mut ui = ui();
    let id = input(&mut ui, "", 100.0);
    for _ in 0..40 {
        type_char(&mut ui, "m");
    }
    ui.set_style(id, Style::row().width(Length::Px(700.0)));
    ui.layout();
    let r = ui.rect(id).unwrap();
    click(&mut ui, r.left() + 3.0, (r.top() + r.bottom()) / 2.0);
    type_char(&mut ui, ">");
    assert!(text_of(&ui, id).starts_with(">m"), "{}", text_of(&ui, id));
}
