//! Menus and the menu bar driven headless: choosing commands with the pointer and the
//! keyboard, submenus, switching a bar's menus, and what assistive technology reads.

use densa_core::{Point, Rect, Size};
use densa_draw::{Canvas, InstanceBuffer};
use densa_paragraph::TextSystem;
use densa_text::{AtlasConfig, GlyphCache};
use densa_ui::{
    ActionKind, Anchor, Key, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerKind, Role,
    Style, Ui, WidgetId,
};
use densa_widgets::{Menu, MenuBar, MenuItem, TextInput};

const NEW: u32 = 1;
const OPEN: u32 = 2;
const SAVE: u32 = 3;
const RECENT_A: u32 = 10;
const RECENT_B: u32 = 11;
const WRAP: u32 = 20;
const QUIT: u32 = 99;

fn file_items() -> Vec<MenuItem> {
    vec![
        MenuItem::new("New", NEW).with_shortcut("Ctrl+N"),
        MenuItem::new("Open", OPEN),
        MenuItem::submenu(
            "Recent",
            [
                MenuItem::new("a.parquet", RECENT_A),
                MenuItem::new("b.csv", RECENT_B),
            ],
        ),
        MenuItem::new("Save", SAVE).enabled(false),
        MenuItem::separator(),
        MenuItem::new("Word wrap", WRAP).checked(true),
        MenuItem::new("Quit", QUIT),
    ]
}

fn ui() -> Ui {
    let mut ui = Ui::new(TextSystem::bundled().unwrap());
    ui.resize(Size::new(800.0, 600.0));
    ui
}

fn paint(ui: &mut Ui) {
    let mut cache = GlyphCache::new(AtlasConfig::default());
    let mut out = InstanceBuffer::with_capacity(16_384);
    let mut canvas = Canvas::begin(&mut cache, &mut out);
    ui.paint(&mut canvas);
}

fn pointer(ui: &mut Ui, kind: PointerKind, at: Point) {
    ui.pointer(PointerEvent::new(kind, at));
}

fn click(ui: &mut Ui, at: Point) {
    pointer(ui, PointerKind::Down(PointerButton::Primary), at);
    pointer(ui, PointerKind::Up(PointerButton::Primary), at);
}

fn key(ui: &mut Ui, named: NamedKey) {
    ui.key(&KeyEvent::down(Key::Named(named)));
}

fn commands(ui: &mut Ui) -> Vec<u32> {
    ui.actions()
        .filter_map(|action| match action.kind {
            ActionKind::Command(command) => Some(command),
            _ => None,
        })
        .collect()
}

/// The bounds of part `part` of widget `id`, as assistive technology reads them.
fn part_rect(ui: &mut Ui, id: WidgetId, part: u32) -> Rect {
    let wanted = ui.access_id(id).unwrap().part(part);
    let tree = ui.access_tree();
    tree.nodes()
        .iter()
        .find(|(i, _)| *i == wanted)
        .map(|(_, node)| node.bounds())
        .unwrap_or_else(|| panic!("no part {part}"))
}

fn middle(rect: Rect) -> Point {
    Point::new(
        (rect.left() + rect.right()) / 2.0,
        (rect.top() + rect.bottom()) / 2.0,
    )
}

fn context_menu(ui: &mut Ui, at: Point) -> WidgetId {
    let id = ui
        .open_popup(Menu::new(file_items()), Anchor::at(at), Menu::style())
        .unwrap();
    ui.layout();
    id
}

#[test]
fn a_release_on_a_command_chooses_it_and_closes_the_menu() {
    let mut ui = ui();
    let menu = context_menu(&mut ui, Point::new(100.0, 100.0));
    let rect = ui.rect(menu).unwrap();
    assert_eq!((rect.left(), rect.top()), (100.0, 100.0));
    let open = middle(part_rect(&mut ui, menu, 1));
    pointer(&mut ui, PointerKind::Move, open);
    click(&mut ui, open);
    assert_eq!(commands(&mut ui), [OPEN]);
    assert!(!ui.is_popup_open(menu));
}

#[test]
fn unavailable_entries_and_separators_are_not_chosen() {
    let mut ui = ui();
    let menu = context_menu(&mut ui, Point::new(100.0, 100.0));
    let save = middle(part_rect(&mut ui, menu, 3));
    click(&mut ui, save);
    let rect = ui.rect(menu).unwrap();
    // The separator: between Save's bottom and Word wrap's top.
    let wrap = part_rect(&mut ui, menu, 5);
    let separator = Point::new(save.x, wrap.top() - 2.0);
    assert!(separator.y > part_rect(&mut ui, menu, 3).bottom());
    click(&mut ui, separator);
    assert!(rect.contains(separator));
    assert_eq!(commands(&mut ui), Vec::<u32>::new());
    assert!(ui.is_popup_open(menu), "still open");
}

#[test]
fn the_keyboard_moves_past_what_cannot_be_chosen_and_chooses() {
    let mut ui = ui();
    let field = ui.add(ui.root(), TextInput::new(""), Style::row()).unwrap();
    ui.set_focus(Some(field));
    let menu = ui
        .open_popup(
            Menu::new(file_items()).highlighting_first(),
            Anchor::at(Point::new(50.0, 50.0)),
            Menu::style(),
        )
        .unwrap();
    assert_eq!(ui.focus(), Some(menu));
    let highlighted = |ui: &Ui| ui.widget::<Menu>(menu).unwrap().highlighted();
    assert_eq!(highlighted(&ui), Some(0));
    let mut seen = Vec::new();
    for _ in 0..6 {
        key(&mut ui, NamedKey::ArrowDown);
        seen.push(highlighted(&ui).unwrap());
    }
    assert_eq!(
        seen,
        [1, 2, 5, 6, 0, 1],
        "Save (off) and the separator are skipped"
    );
    key(&mut ui, NamedKey::ArrowUp);
    key(&mut ui, NamedKey::ArrowUp);
    assert_eq!(highlighted(&ui), Some(6), "wrapping backwards");
    key(&mut ui, NamedKey::Home);
    assert_eq!(highlighted(&ui), Some(0));
    key(&mut ui, NamedKey::End);
    assert_eq!(highlighted(&ui), Some(6));
    // A letter jumps to the next entry starting with it.
    ui.key(&KeyEvent::down(Key::Character("w")).with_text("w"));
    assert_eq!(highlighted(&ui), Some(5));
    key(&mut ui, NamedKey::Enter);
    assert_eq!(commands(&mut ui), [WRAP]);
    assert!(!ui.is_popup_open(menu));
    assert_eq!(ui.focus(), Some(field), "focus went back");
}

#[test]
fn escape_closes_a_menu_and_gives_focus_back() {
    let mut ui = ui();
    let field = ui.add(ui.root(), TextInput::new(""), Style::row()).unwrap();
    ui.set_focus(Some(field));
    let menu = context_menu(&mut ui, Point::new(100.0, 100.0));
    key(&mut ui, NamedKey::Escape);
    assert!(!ui.is_popup_open(menu));
    assert_eq!(ui.focus(), Some(field));
}

#[test]
fn a_submenu_opens_beside_its_entry_and_its_commands_close_every_menu() {
    let mut ui = ui();
    let menu = context_menu(&mut ui, Point::new(100.0, 100.0));
    let recent = part_rect(&mut ui, menu, 2);
    pointer(&mut ui, PointerKind::Move, middle(recent));
    let popups: Vec<_> = ui.popups().collect();
    assert_eq!(popups.len(), 2, "the submenu opened on hover");
    let submenu = popups[1];
    ui.layout();
    let beside = ui.rect(submenu).unwrap();
    assert_eq!(beside.left(), ui.rect(menu).unwrap().right());
    assert_eq!(beside.top(), recent.top());

    // Over another entry, the submenu closes.
    let new = middle(part_rect(&mut ui, menu, 0));
    pointer(&mut ui, PointerKind::Move, new);
    assert!(!ui.is_popup_open(submenu));
    pointer(&mut ui, PointerKind::Move, middle(recent));
    let submenu = ui.popups().nth(1).unwrap();
    ui.layout();
    let b = middle(part_rect(&mut ui, submenu, 1));
    pointer(&mut ui, PointerKind::Move, b);
    assert!(ui.is_popup_open(submenu) && ui.is_popup_open(menu));
    click(&mut ui, b);
    assert_eq!(commands(&mut ui), [RECENT_B]);
    assert_eq!(ui.popups().count(), 0);
}

#[test]
fn right_opens_a_submenu_and_left_closes_it() {
    let mut ui = ui();
    let menu = ui
        .open_popup(
            Menu::new(file_items()).highlighting_first(),
            Anchor::at(Point::new(50.0, 50.0)),
            Menu::style(),
        )
        .unwrap();
    key(&mut ui, NamedKey::ArrowDown);
    key(&mut ui, NamedKey::ArrowDown);
    key(&mut ui, NamedKey::ArrowRight);
    let submenu = ui.popups().nth(1).expect("submenu");
    assert_eq!(ui.focus(), Some(submenu));
    assert_eq!(ui.widget::<Menu>(submenu).unwrap().highlighted(), Some(0));
    key(&mut ui, NamedKey::ArrowLeft);
    assert!(!ui.is_popup_open(submenu));
    assert_eq!(ui.focus(), Some(menu), "back in the menu");
    key(&mut ui, NamedKey::Enter);
    assert!(ui.popups().nth(1).is_some(), "Enter opens a submenu too");
    key(&mut ui, NamedKey::Enter);
    assert_eq!(commands(&mut ui), [RECENT_A]);
}

fn menu_bar(ui: &mut Ui) -> WidgetId {
    let bar = MenuBar::new()
        .with_menu("File", file_items())
        .with_menu(
            "Edit",
            [MenuItem::new("Copy", 30), MenuItem::new("Paste", 31)],
        )
        .with_menu("View", [MenuItem::new("Zoom in", 40)]);
    let id = ui.add(ui.root(), bar, MenuBar::style()).unwrap();
    ui.layout();
    id
}

#[test]
fn a_bar_opens_a_menu_below_a_title_and_closes_it_on_a_second_press() {
    let mut ui = ui();
    let bar = menu_bar(&mut ui);
    let file = part_rect(&mut ui, bar, 0);
    click(&mut ui, middle(file));
    let menu = ui.popups().next().expect("open");
    ui.layout();
    let rect = ui.rect(menu).unwrap();
    assert_eq!((rect.left(), rect.top()), (file.left(), file.bottom()));
    assert_eq!(ui.widget::<MenuBar>(bar).unwrap().open_menu(), Some(0));
    click(&mut ui, middle(file));
    assert_eq!(ui.popups().count(), 0, "closed by its title");
    click(&mut ui, middle(file));
    assert_eq!(ui.popups().count(), 1, "and opened again");
}

#[test]
fn hovering_another_title_switches_menus() {
    let mut ui = ui();
    let bar = menu_bar(&mut ui);
    let (file, edit) = (part_rect(&mut ui, bar, 0), part_rect(&mut ui, bar, 1));
    // Nothing open: hovering only highlights.
    pointer(&mut ui, PointerKind::Move, middle(edit));
    assert_eq!(ui.popups().count(), 0);
    click(&mut ui, middle(file));
    pointer(&mut ui, PointerKind::Move, middle(edit));
    assert_eq!(ui.widget::<MenuBar>(bar).unwrap().open_menu(), Some(1));
    let menu = ui.popups().next().unwrap();
    ui.layout();
    assert_eq!(ui.rect(menu).unwrap().left(), edit.left());
    let copy = middle(part_rect(&mut ui, menu, 0));
    pointer(&mut ui, PointerKind::Move, copy);
    click(&mut ui, copy);
    assert_eq!(commands(&mut ui), [30]);
}

#[test]
fn left_and_right_in_an_open_menu_switch_the_bar_menus() {
    let mut ui = ui();
    let bar = menu_bar(&mut ui);
    ui.update::<MenuBar, _>(bar, |bar, cx| bar.activate(cx));
    assert_eq!(ui.focus(), Some(bar));
    key(&mut ui, NamedKey::ArrowRight);
    key(&mut ui, NamedKey::ArrowDown);
    let open = |ui: &Ui| ui.widget::<MenuBar>(bar).unwrap().open_menu();
    assert_eq!(open(&ui), Some(1), "Edit, opened from the keyboard");
    let menu = ui.popups().next().unwrap();
    assert_eq!(ui.focus(), Some(menu));
    key(&mut ui, NamedKey::ArrowRight);
    assert_eq!(open(&ui), Some(2));
    key(&mut ui, NamedKey::ArrowRight);
    assert_eq!(open(&ui), Some(0), "wrapping");
    key(&mut ui, NamedKey::ArrowLeft);
    assert_eq!(open(&ui), Some(2));
    key(&mut ui, NamedKey::Enter);
    assert_eq!(commands(&mut ui), [40]);
    assert_eq!(ui.popups().count(), 0);
}

#[test]
fn right_on_a_submenu_entry_opens_it_instead_of_switching() {
    let mut ui = ui();
    let bar = menu_bar(&mut ui);
    ui.update::<MenuBar, _>(bar, |bar, cx| bar.activate(cx));
    key(&mut ui, NamedKey::Enter);
    key(&mut ui, NamedKey::ArrowDown);
    key(&mut ui, NamedKey::ArrowDown);
    key(&mut ui, NamedKey::ArrowRight);
    assert_eq!(ui.popups().count(), 2, "Recent opened");
    assert_eq!(ui.widget::<MenuBar>(bar).unwrap().open_menu(), Some(0));
    // Right again from inside the submenu: nothing to open there, so the bar switches.
    key(&mut ui, NamedKey::ArrowRight);
    assert_eq!(ui.widget::<MenuBar>(bar).unwrap().open_menu(), Some(1));
    assert_eq!(ui.popups().count(), 1);
}

#[test]
fn assistive_technology_reads_menus_and_their_entries() {
    let mut ui = ui();
    let bar = menu_bar(&mut ui);
    let file = middle(part_rect(&mut ui, bar, 0));
    click(&mut ui, file);
    let menu = ui.popups().next().unwrap();
    let tree = ui.access_tree();
    let node = |id: densa_ui::AccessId| {
        tree.nodes()
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, node)| node.clone())
            .unwrap()
    };
    let bar_id = ui.access_id(bar).unwrap();
    assert_eq!(node(bar_id).role(), Role::MenuBar);
    assert_eq!(node(bar_id.part(0)).expanded(), Some(true));
    assert_eq!(node(bar_id.part(1)).expanded(), Some(false));
    let menu_id = ui.access_id(menu).unwrap();
    assert_eq!(node(menu_id).role(), Role::Menu);
    assert_eq!(node(menu_id.part(0)).label(), Some("New"));
    assert!(node(menu_id.part(3)).is_disabled());
    let wrap = node(menu_id.part(5));
    assert_eq!(
        (wrap.role(), wrap.checked()),
        (Role::MenuItemCheckBox, Some(true))
    );
    assert!(
        tree.nodes().iter().all(|(i, _)| *i != menu_id.part(4)),
        "no node for the separator"
    );

    // Clicking an entry for assistive technology chooses it.
    let done = ui.access_action(menu_id.part(1), densa_ui::AccessAction::Click, None);
    assert!(done);
    assert_eq!(commands(&mut ui), [OPEN]);
}

#[test]
fn a_menu_paints_with_its_bar() {
    let mut ui = ui();
    let bar = menu_bar(&mut ui);
    let file = middle(part_rect(&mut ui, bar, 0));
    click(&mut ui, file);
    paint(&mut ui);
    let menu = ui.popups().next().unwrap();
    let rect = ui.rect(menu).unwrap();
    assert!(rect.width() > 100.0 && rect.height() > 100.0, "{rect:?}");
}

#[test]
fn switching_menus_then_escape_gives_focus_back_to_where_it_was() {
    let mut ui = ui();
    let field = ui.add(ui.root(), TextInput::new(""), Style::row()).unwrap();
    let bar = menu_bar(&mut ui);
    ui.set_focus(Some(field));
    let (file, edit) = (part_rect(&mut ui, bar, 0), part_rect(&mut ui, bar, 1));
    click(&mut ui, middle(file));
    pointer(&mut ui, PointerKind::Move, middle(edit));
    assert_eq!(ui.widget::<MenuBar>(bar).unwrap().open_menu(), Some(1));
    key(&mut ui, NamedKey::Escape);
    assert_eq!(ui.popups().count(), 0);
    assert_eq!(
        ui.focus(),
        Some(field),
        "not lost with the menu switched away from"
    );
}

fn two_submenus() -> Vec<MenuItem> {
    vec![
        MenuItem::submenu("Alpha", [MenuItem::new("a1", 1), MenuItem::new("a2", 2)]),
        MenuItem::submenu("Beta", [MenuItem::new("b1", 3), MenuItem::new("b2", 4)]),
        MenuItem::new("Gamma", 5),
    ]
}

#[test]
fn a_submenu_opened_by_hovering_takes_the_keyboard_only_once_entered() {
    let mut ui = ui();
    let menu = ui
        .open_popup(
            Menu::new(two_submenus()),
            Anchor::at(Point::new(50.0, 50.0)),
            Menu::style(),
        )
        .unwrap();
    ui.layout();
    let (alpha, beta) = (part_rect(&mut ui, menu, 0), part_rect(&mut ui, menu, 1));
    pointer(&mut ui, PointerKind::Move, middle(alpha));
    let first = ui.popups().nth(1).unwrap();
    assert_eq!(ui.focus(), Some(menu), "the menu keeps the keyboard");

    // The sibling's submenu replaces it; then entering that one gives it the keyboard.
    pointer(
        &mut ui,
        PointerKind::Move,
        Point::new(alpha.left() + 5.0, middle(beta).y),
    );
    assert!(!ui.is_popup_open(first));
    let second = ui.popups().nth(1).unwrap();
    ui.layout();
    let b1 = middle(part_rect(&mut ui, second, 0));
    pointer(&mut ui, PointerKind::Move, b1);
    assert_eq!(ui.focus(), Some(second));
    key(&mut ui, NamedKey::ArrowLeft);
    assert!(!ui.is_popup_open(second));
    assert_eq!(ui.focus(), Some(menu), "Left goes back to the menu");
}

#[test]
fn moving_diagonally_toward_a_submenu_keeps_it_open() {
    let mut ui = ui();
    let menu = ui
        .open_popup(
            Menu::new(two_submenus()),
            Anchor::at(Point::new(50.0, 50.0)),
            Menu::style(),
        )
        .unwrap();
    ui.layout();
    let (alpha, beta) = (part_rect(&mut ui, menu, 0), part_rect(&mut ui, menu, 1));
    let start = Point::new(alpha.right() - 40.0, middle(alpha).y);
    pointer(&mut ui, PointerKind::Move, start);
    let submenu = ui.popups().nth(1).unwrap();
    ui.layout();
    // Down and right across Beta's row, level with the submenu.
    let crossing = Point::new(start.x + 20.0, middle(beta).y);
    assert!(ui.rect(submenu).unwrap().bottom() > crossing.y);
    pointer(&mut ui, PointerKind::Move, crossing);
    assert!(ui.is_popup_open(submenu), "on the way to it");
    assert_eq!(ui.widget::<Menu>(menu).unwrap().highlighted(), Some(0));
    // Going back left over Beta: its own submenu opens instead.
    pointer(
        &mut ui,
        PointerKind::Move,
        Point::new(crossing.x - 30.0, crossing.y),
    );
    assert!(!ui.is_popup_open(submenu));
    assert_eq!(ui.widget::<Menu>(menu).unwrap().highlighted(), Some(1));
}

#[test]
fn shortcuts_do_not_get_past_an_open_menu() {
    let mut ui = ui();
    let _menu = context_menu(&mut ui, Point::new(100.0, 100.0));
    let save = KeyEvent::down(Key::Character("s"))
        .with_text("s")
        .with_modifiers(densa_ui::Modifiers::CONTROL);
    assert_eq!(ui.key(&save), densa_ui::Handled::Yes);
    assert_eq!(
        ui.key(&KeyEvent::down(Key::Named(NamedKey::F(5)))),
        densa_ui::Handled::Yes
    );
    assert_eq!(commands(&mut ui), Vec::<u32>::new());
}

#[test]
fn moving_right_below_a_submenu_switches_rows() {
    let mut ui = ui();
    let items = vec![
        MenuItem::submenu("Short", [MenuItem::new("s1", 1)]),
        MenuItem::new("Two", 2),
        MenuItem::new("Three", 3),
        MenuItem::new("Four", 4),
        MenuItem::new("Five", 5),
    ];
    let menu = ui
        .open_popup(
            Menu::new(items),
            Anchor::at(Point::new(50.0, 50.0)),
            Menu::style(),
        )
        .unwrap();
    ui.layout();
    let short = part_rect(&mut ui, menu, 0);
    let start = Point::new(short.right() - 40.0, middle(short).y);
    pointer(&mut ui, PointerKind::Move, start);
    let submenu = ui.popups().nth(1).unwrap();
    ui.layout();
    let five = part_rect(&mut ui, menu, 4);
    assert!(
        ui.rect(submenu).unwrap().bottom() < five.top(),
        "the submenu is short"
    );
    // Toward it, but below it: not on the way, so the row under the pointer takes over.
    pointer(
        &mut ui,
        PointerKind::Move,
        Point::new(start.x + 10.0, middle(five).y),
    );
    assert!(!ui.is_popup_open(submenu));
    assert_eq!(ui.widget::<Menu>(menu).unwrap().highlighted(), Some(4));
}

#[test]
fn a_letter_with_ctrl_does_not_move_the_highlight() {
    let mut ui = ui();
    let menu = ui
        .open_popup(
            Menu::new(file_items()).highlighting_first(),
            Anchor::at(Point::new(50.0, 50.0)),
            Menu::style(),
        )
        .unwrap();
    let ctrl_w = KeyEvent::down(Key::Character("w"))
        .with_text("w")
        .with_modifiers(densa_ui::Modifiers::CONTROL);
    ui.key(&ctrl_w);
    assert_eq!(ui.widget::<Menu>(menu).unwrap().highlighted(), Some(0));
    ui.key(&KeyEvent::down(Key::Character("w")).with_text("w"));
    assert_eq!(ui.widget::<Menu>(menu).unwrap().highlighted(), Some(5));
}
