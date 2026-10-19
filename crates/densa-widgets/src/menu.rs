//! Menus: a [`Menu`] pops up a list of commands; a [`MenuBar`] holds the titles of a
//! window's menus. Choosing a command reports [`ActionKind::Command`] with the number the
//! application gave it.

use densa_core::{Color, Point, Rect, Size};
use densa_draw::Canvas;
use densa_ui::{
    AccessAction, AccessNode, AccessRequest, ActionKind, Anchor, Constraints, Cursor, Event,
    EventCx, Handled, Key, LayoutCx, NamedKey, PaintCx, PointerButton, PointerKind, Role, Style,
    Widget, WidgetId,
};

use crate::paint::outline;
use crate::text::Text;

/// Space around a menu's rows, in logical pixels.
const MENU_PADDING: f32 = 4.0;
/// Space above and below a row's text.
const ROW_PADDING_Y: f32 = 4.0;
/// The column for check marks, left of the labels.
const GUTTER: f32 = 22.0;
/// Space between a label and its shortcut.
const SHORTCUT_GAP: f32 = 28.0;
/// The column for submenu arrows, right of the shortcuts.
const ARROW: f32 = 18.0;
/// A separator's height.
const SEPARATOR: f32 = 9.0;
/// Space either side of a menu bar title.
const TITLE_PADDING_X: f32 = 10.0;
const TITLE_PADDING_Y: f32 = 5.0;

/// An entry of a [`Menu`]: a command, a submenu or a separator.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    kind: ItemKind,
}

#[derive(Debug, Clone, PartialEq)]
enum ItemKind {
    Command {
        label: String,
        command: u32,
        shortcut: Option<String>,
        enabled: bool,
        checked: Option<bool>,
    },
    Submenu {
        label: String,
        items: Vec<MenuItem>,
        enabled: bool,
    },
    Separator,
}

impl MenuItem {
    /// A command labeled `label`: choosing it reports [`ActionKind::Command`]`(command)`.
    pub fn new(label: impl Into<String>, command: u32) -> Self {
        Self {
            kind: ItemKind::Command {
                label: label.into(),
                command,
                shortcut: None,
                enabled: true,
                checked: None,
            },
        }
    }

    /// An entry that opens `items` beside it.
    pub fn submenu(label: impl Into<String>, items: impl IntoIterator<Item = MenuItem>) -> Self {
        Self {
            kind: ItemKind::Submenu {
                label: label.into(),
                items: items.into_iter().collect(),
                enabled: true,
            },
        }
    }

    /// A line between groups of entries.
    pub fn separator() -> Self {
        Self {
            kind: ItemKind::Separator,
        }
    }

    /// Shows `shortcut` (such as "Ctrl+C") at the right of a command. It is only shown: the
    /// application handles the keys.
    #[must_use]
    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        if let ItemKind::Command { shortcut: s, .. } = &mut self.kind {
            *s = Some(shortcut.into());
        }
        self
    }

    /// The entry available (the default) or not: an unavailable one is shown dimmed and
    /// cannot be chosen.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        match &mut self.kind {
            ItemKind::Command { enabled: e, .. } | ItemKind::Submenu { enabled: e, .. } => {
                *e = enabled;
            }
            ItemKind::Separator => {}
        }
        self
    }

    /// Makes a command one that is on (checked) or off, shown with a check mark when on.
    /// Choosing it reports the command; the application turns it on or off.
    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        if let ItemKind::Command { checked: c, .. } = &mut self.kind {
            *c = Some(checked);
        }
        self
    }

    fn label(&self) -> &str {
        match &self.kind {
            ItemKind::Command { label, .. } | ItemKind::Submenu { label, .. } => label,
            ItemKind::Separator => "",
        }
    }

    fn is_enabled(&self) -> bool {
        match &self.kind {
            ItemKind::Command { enabled, .. } | ItemKind::Submenu { enabled, .. } => *enabled,
            ItemKind::Separator => false,
        }
    }
}

/// A row of a menu, as laid out.
#[derive(Debug)]
struct Row {
    item: MenuItem,
    label: Text,
    shortcut: Option<Text>,
    /// Top and bottom, from the menu's top edge, in physical pixels.
    top: f32,
    bottom: f32,
}

/// A list of commands that pops up: open it with [`Ui::open_popup`] (a context menu) or
/// from a [`MenuBar`].
///
/// The pointer highlights entries and a release on a command chooses it; a submenu opens
/// beside its entry when the pointer comes over it. Up and Down move the highlight, Enter or
/// Space chooses, Right opens a submenu and Left or Escape closes one; a key the menu does
/// not use goes on to what opened it (Left and Right switch a menu bar's menus). Choosing a
/// command reports [`ActionKind::Command`] and closes every menu.
///
/// [`Ui::open_popup`]: densa_ui::Ui::open_popup
#[derive(Debug)]
pub struct Menu {
    rows: Vec<Row>,
    check: Text,
    arrow: Text,
    highlighted: Option<usize>,
    /// The submenu open, and the row it belongs to.
    open: Option<(usize, WidgetId)>,
    /// Highlight the first entry when shown (opened from the keyboard).
    from_keyboard: bool,
    /// Opened by another menu: Left closes it.
    nested: bool,
    /// The pointer has come over it (a submenu opened by hovering takes the keyboard then).
    entered: bool,
    /// Where the pointer last was over it: moving toward an open submenu keeps it open.
    last_pointer: Option<Point>,
}

impl Menu {
    /// A menu of `items`.
    pub fn new(items: impl IntoIterator<Item = MenuItem>) -> Self {
        let rows = items
            .into_iter()
            .map(|item| Row {
                label: Text::new(item.label()),
                shortcut: match &item.kind {
                    ItemKind::Command {
                        shortcut: Some(s), ..
                    } => Some(Text::new(s.as_str())),
                    _ => None,
                },
                item,
                top: 0.0,
                bottom: 0.0,
            })
            .collect();
        Self {
            rows,
            check: Text::new("\u{2713}"),
            arrow: Text::new("\u{203A}"),
            highlighted: None,
            open: None,
            from_keyboard: false,
            nested: false,
            entered: false,
            last_pointer: None,
        }
    }

    /// The style to open a menu with: it sizes itself.
    pub fn style() -> Style {
        Style::column()
    }

    /// The menu with its first available entry highlighted, as when opened from the keyboard.
    #[must_use]
    pub fn highlighting_first(mut self) -> Self {
        self.from_keyboard = true;
        self.highlighted = self.next_enabled(None, true);
        self
    }

    /// The entry highlighted, by index.
    pub fn highlighted(&self) -> Option<usize> {
        self.highlighted
    }

    /// The next available entry after (`forward`) or before `from`, wrapping around; from
    /// the first (or last) when `from` is `None`.
    fn next_enabled(&self, from: Option<usize>, forward: bool) -> Option<usize> {
        let count = self.rows.len();
        if count == 0 {
            return None;
        }
        let start = match (from, forward) {
            (Some(at), true) => at + 1,
            (Some(at), false) => at + count - 1,
            (None, true) => 0,
            (None, false) => count - 1,
        };
        (0..count)
            .map(|step| {
                if forward {
                    (start + step) % count
                } else {
                    (start + count - step % count) % count
                }
            })
            .find(|&index| self.rows[index].item.is_enabled())
    }

    fn row_at(&self, rect: Rect, at: Point) -> Option<usize> {
        if !rect.contains(at) {
            return None;
        }
        let y = at.y - rect.top();
        self.rows
            .iter()
            .position(|row| y >= row.top && y < row.bottom)
    }

    fn row_rect(&self, rect: Rect, index: usize) -> Rect {
        let row = &self.rows[index];
        Rect::from_edges(
            rect.left(),
            rect.top() + row.top,
            rect.right(),
            rect.top() + row.bottom,
        )
    }

    /// Highlights row `index` (`None`: nothing), closing a submenu of another row and opening
    /// its own if it has one and `open_submenu` holds.
    fn highlight(&mut self, cx: &mut EventCx<'_>, index: Option<usize>, open_submenu: bool) {
        if self.highlighted != index {
            self.highlighted = index;
            cx.request_paint();
        }
        if let Some((row, id)) = self.open {
            if Some(row) != index || !cx.is_popup_open(id) {
                cx.close_popup(id);
                self.open = None;
            }
        }
        if open_submenu {
            if let Some(index) = index {
                self.open_submenu(cx, index, false);
            }
        }
    }

    /// Opens row `index`'s submenu, if it has one and it is not open.
    fn open_submenu(&mut self, cx: &mut EventCx<'_>, index: usize, from_keyboard: bool) -> bool {
        if self
            .open
            .is_some_and(|(row, id)| row == index && cx.is_popup_open(id))
        {
            return true;
        }
        let ItemKind::Submenu { items, enabled, .. } = &self.rows[index].item.kind else {
            return false;
        };
        if !*enabled {
            return false;
        }
        let mut menu = Menu::new(items.iter().cloned());
        menu.nested = true;
        if from_keyboard {
            menu = menu.highlighting_first();
        }
        let anchor = Anchor::beside(self.row_rect(cx.rect(), index));
        match cx.open_popup(menu, anchor, Menu::style()) {
            Ok(id) => {
                self.open = Some((index, id));
                true
            }
            Err(_) => false,
        }
    }

    /// Chooses row `index`: reports its command and closes every menu, or opens its submenu.
    fn choose(&mut self, cx: &mut EventCx<'_>, index: usize, from_keyboard: bool) {
        match &self.rows[index].item.kind {
            ItemKind::Command {
                command, enabled, ..
            } if *enabled => {
                cx.emit(ActionKind::Command(*command));
                cx.close_popups();
            }
            ItemKind::Submenu { .. } => {
                self.open_submenu(cx, index, from_keyboard);
            }
            _ => {}
        }
    }

    fn key(&mut self, cx: &mut EventCx<'_>, key: NamedKey) -> Handled {
        match key {
            NamedKey::ArrowDown | NamedKey::ArrowUp => {
                let next = self.next_enabled(self.highlighted, key == NamedKey::ArrowDown);
                self.highlight(cx, next, false);
                Handled::Yes
            }
            NamedKey::Home | NamedKey::End => {
                let next = self.next_enabled(None, key == NamedKey::Home);
                self.highlight(cx, next, false);
                Handled::Yes
            }
            NamedKey::Enter | NamedKey::Space => {
                if let Some(index) = self.highlighted {
                    self.choose(cx, index, true);
                }
                Handled::Yes
            }
            NamedKey::ArrowRight => {
                // A submenu passes Right on while one of its own is open: to the menu bar.
                let open = self.open.is_some_and(|(_, id)| cx.is_popup_open(id));
                match self.highlighted {
                    Some(index) if !open => self.open_submenu(cx, index, true).into(),
                    _ => Handled::No,
                }
            }
            NamedKey::ArrowLeft if self.nested => {
                cx.close_popup(cx.id());
                Handled::Yes
            }
            NamedKey::Escape => {
                cx.close_popup(cx.id());
                Handled::Yes
            }
            // Left from a menu that is not a submenu goes on to its menu bar.
            NamedKey::ArrowLeft => Handled::No,
            // Nothing else gets past an open menu (Tab included).
            _ => Handled::Yes,
        }
    }

    /// Whether the pointer, now at `at`, is moving toward the open submenu while level with
    /// it: crossing other rows on the way should not close it.
    fn heading_to_submenu(&self, cx: &EventCx<'_>, at: Point) -> bool {
        let (Some((_, id)), Some(last)) = (self.open, self.last_pointer) else {
            return false;
        };
        let Some(submenu) = cx.rect_of(id).filter(|_| cx.is_popup_open(id)) else {
            return false;
        };
        let toward = if submenu.left() >= cx.rect().right() - 1.0 {
            at.x > last.x
        } else {
            at.x < last.x
        };
        toward && at.y >= submenu.top() && at.y < submenu.bottom()
    }
}

/// Whether `label` starts with `typed`'s first character, ignoring case.
fn starts_with_key(label: &str, typed: &str) -> bool {
    match (label.chars().next(), typed.chars().next()) {
        (Some(first), Some(key)) => first.to_lowercase().eq(key.to_lowercase()),
        _ => false,
    }
}

/// Shapes `text` in `color` on one line; its size.
fn shaped(text: &mut Text, cx: &mut LayoutCx<'_>, color: Color) -> Size {
    text.fit(cx, f32::INFINITY, color);
    text.size()
}

impl Widget for Menu {
    fn measure(&mut self, cx: &mut LayoutCx<'_>, _: Constraints) -> Size {
        let theme = cx.theme().clone();
        let check = shaped(&mut self.check, cx, theme.text);
        let arrow = shaped(&mut self.arrow, cx, theme.text_dim);
        let (mut label_width, mut shortcut_width, mut submenus) = (0.0_f32, 0.0_f32, false);
        let mut y = cx.px(MENU_PADDING);
        let line = check.height.max(arrow.height);
        for row in &mut self.rows {
            let color = if row.item.is_enabled() {
                theme.text
            } else {
                theme.text_dim
            };
            row.top = y;
            if matches!(row.item.kind, ItemKind::Separator) {
                y += cx.px(SEPARATOR).round();
                row.bottom = y;
                continue;
            }
            let label = shaped(&mut row.label, cx, color);
            label_width = label_width.max(label.width);
            if let Some(shortcut) = &mut row.shortcut {
                shortcut_width = shortcut_width.max(shaped(shortcut, cx, theme.text_dim).width);
            }
            submenus |= matches!(row.item.kind, ItemKind::Submenu { .. });
            y += (label.height.max(line) + 2.0 * cx.px(ROW_PADDING_Y)).round();
            row.bottom = y;
        }
        let shortcuts = if shortcut_width > 0.0 {
            cx.px(SHORTCUT_GAP) + shortcut_width
        } else {
            0.0
        };
        let arrows = if submenus { cx.px(ARROW) } else { 0.0 };
        let width = 2.0 * cx.px(MENU_PADDING)
            + cx.px(GUTTER)
            + label_width
            + shortcuts
            + arrows
            + cx.px(GUTTER) / 2.0;
        Size::new(width.ceil(), (y + cx.px(MENU_PADDING)).ceil())
    }

    fn paint(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        let theme = cx.theme();
        let rect = cx.rect();
        // A soft edge below and to the right: the menu floats over what it covers.
        for (offset, alpha) in [(1.0, 0.22), (2.0, 0.14), (3.0, 0.07)] {
            let d = cx.px(offset).round();
            let shadow = Rect::from_edges(
                rect.left() + d,
                rect.top() + d,
                rect.right() + d,
                rect.bottom() + d,
            );
            canvas.fill_rect(shadow, Color::BLACK.with_alpha(alpha));
        }
        canvas.fill_rect(rect, theme.raised);
        outline(canvas, rect, cx.px(1.0).round(), theme.border);
        let padding = cx.px(MENU_PADDING);
        let (left, right) = (rect.left() + padding, rect.right() - padding);
        for (index, row) in self.rows.iter().enumerate() {
            let (top, bottom) = (rect.top() + row.top, rect.top() + row.bottom);
            if matches!(row.item.kind, ItemKind::Separator) {
                let middle = ((top + bottom) / 2.0).round();
                let line = cx.px(1.0).round();
                canvas.fill_rect(
                    Rect::from_edges(left, middle, right, middle + line),
                    theme.border,
                );
                continue;
            }
            let open = self
                .open
                .is_some_and(|(row, id)| row == index && cx.is_popup_open(id));
            if self.highlighted == Some(index) || open {
                canvas.fill_rect(Rect::from_edges(left, top, right, bottom), theme.hover);
            }
            let text_top = |height: f32| (top + (bottom - top - height) / 2.0).round();
            if let ItemKind::Command {
                checked: Some(true),
                ..
            } = row.item.kind
            {
                let size = self.check.size();
                let x = (left + (cx.px(GUTTER) - size.width) / 2.0).round();
                self.check
                    .paint(canvas, Point::new(x, text_top(size.height)));
            }
            let label = row.label.size();
            let x = (left + cx.px(GUTTER)).round();
            row.label
                .paint(canvas, Point::new(x, text_top(label.height)));
            let mut end = right - cx.px(GUTTER) / 2.0;
            if matches!(row.item.kind, ItemKind::Submenu { .. }) {
                let size = self.arrow.size();
                let at = Point::new((end - size.width).round(), text_top(size.height));
                self.arrow.paint(canvas, at);
                end -= cx.px(ARROW);
            }
            if let Some(shortcut) = &row.shortcut {
                let size = shortcut.size();
                let at = Point::new((end - size.width).round(), text_top(size.height));
                shortcut.paint(canvas, at);
            }
        }
    }

    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        if let Some((_, id)) = self.open {
            if !cx.is_popup_open(id) {
                self.open = None;
            }
        }
        match event {
            Event::Pointer(pointer) => match pointer.kind {
                PointerKind::Enter | PointerKind::Move => {
                    cx.set_cursor(Some(Cursor::Default));
                    if self.nested && !self.entered {
                        // A submenu opened by hovering takes the keyboard once the pointer
                        // comes over it.
                        self.entered = true;
                        cx.request_focus();
                    }
                    let at = pointer.position;
                    let heading = self.heading_to_submenu(cx, at);
                    self.last_pointer = Some(at);
                    let row = self
                        .row_at(cx.rect(), at)
                        .filter(|&index| self.rows[index].item.is_enabled());
                    // Between rows, over a separator, or on the way to it diagonally across
                    // other rows: keep the submenu open.
                    if !heading && (row.is_some() || self.open.is_none()) {
                        self.highlight(cx, row, true);
                    }
                    Handled::Yes
                }
                PointerKind::Leave => {
                    self.last_pointer = None;
                    if self.open.is_none() {
                        self.highlight(cx, None, false);
                    }
                    Handled::Yes
                }
                PointerKind::Up(PointerButton::Primary) => {
                    if let Some(index) = self.row_at(cx.rect(), pointer.position) {
                        self.choose(cx, index, false);
                    }
                    Handled::Yes
                }
                PointerKind::Down(_) | PointerKind::Up(_) | PointerKind::Scroll(_) => Handled::Yes,
                _ => Handled::No,
            },
            Event::Key(key) if key.pressed => match key.key {
                Key::Named(named) => self.key(cx, named),
                // Shortcuts do not reach the window behind an open menu.
                Key::Character(_) if key.modifiers.control() || key.modifiers.alt() => Handled::Yes,
                Key::Character(typed) => {
                    // Typing a letter jumps to the next entry starting with it.
                    let count = self.rows.len();
                    let start = self.highlighted.map_or(0, |at| at + 1);
                    let found = (0..count)
                        .map(|step| (start + step) % count)
                        .find(|&index| {
                            let row = &self.rows[index];
                            row.item.is_enabled() && starts_with_key(row.item.label(), typed)
                        });
                    if found.is_some() {
                        self.highlight(cx, found, false);
                    }
                    Handled::Yes
                }
                _ => Handled::Yes,
            },
            Event::FocusGained | Event::FocusLost => {
                cx.request_paint();
                Handled::Yes
            }
            _ => Handled::No,
        }
    }

    /// A menu takes the keyboard when it opens, except a submenu opened by hovering, which
    /// takes it when the pointer comes over it (the parent keeps it meanwhile).
    fn accepts_focus(&self) -> bool {
        !self.nested || self.from_keyboard || self.entered
    }

    fn accessibility(&self, cx: &PaintCx<'_>, node: &mut AccessNode) {
        node.set_role(Role::Menu);
        let rect = cx.rect();
        for (index, row) in self.rows.iter().enumerate() {
            let role = match row.item.kind {
                ItemKind::Separator => continue,
                ItemKind::Command {
                    checked: Some(_), ..
                } => Role::MenuItemCheckBox,
                _ => Role::MenuItem,
            };
            let mut item = AccessNode::new(role);
            item.set_label(row.item.label());
            item.set_bounds(self.row_rect(rect, index));
            item.set_disabled(!row.item.is_enabled());
            match &row.item.kind {
                ItemKind::Command {
                    checked: Some(checked),
                    ..
                } => item.set_checked(*checked),
                ItemKind::Submenu { .. } => {
                    let open = self
                        .open
                        .is_some_and(|(at, id)| at == index && cx.is_popup_open(id));
                    item.set_expanded(open);
                }
                _ => {}
            }
            if row.item.is_enabled() {
                item.add_action(AccessAction::Click);
            }
            // Lossless: a menu has far fewer than 2^32 entries.
            node.push_part(index as u32, item);
        }
        if let Some(index) = self.highlighted {
            // Lossless: as above.
            node.set_focused_part(index as u32);
        }
    }

    fn access_action(&mut self, cx: &mut EventCx<'_>, request: &AccessRequest<'_>) -> Handled {
        let index = request.part.map(|part| part as usize);
        match (request.action, index) {
            (AccessAction::Click, Some(index)) if index < self.rows.len() => {
                self.highlight(cx, Some(index), false);
                self.choose(cx, index, true);
                Handled::Yes
            }
            _ => Handled::No,
        }
    }
}

/// A menu bar title, as laid out.
#[derive(Debug)]
struct Title {
    text: Text,
    items: Vec<MenuItem>,
    /// Left and right, from the bar's left edge, in physical pixels.
    left: f32,
    right: f32,
}

/// A row of menu titles: a press on one opens its [`Menu`] below it, and while one is open
/// the pointer coming over another title switches to it. Left and Right switch menus from
/// the keyboard. The bar is not in the Tab order: give it the keyboard with
/// [`MenuBar::activate`] (on F10, say).
#[derive(Debug, Default)]
pub struct MenuBar {
    titles: Vec<Title>,
    highlighted: Option<usize>,
    /// The menu open, and the title it belongs to.
    open: Option<(usize, WidgetId)>,
}

impl MenuBar {
    /// A bar without menus.
    pub fn new() -> Self {
        Self::default()
    }

    /// The bar with a menu titled `title` of `items` after the others.
    #[must_use]
    pub fn with_menu(
        mut self,
        title: impl Into<String>,
        items: impl IntoIterator<Item = MenuItem>,
    ) -> Self {
        self.titles.push(Title {
            text: Text::new(title),
            items: items.into_iter().collect(),
            left: 0.0,
            right: 0.0,
        });
        self
    }

    /// The style for a bar across the top of a window.
    pub fn style() -> Style {
        Style::row().shrink(0.0)
    }

    /// Gives the bar the keyboard, its first title highlighted: Left and Right move, Down,
    /// Enter or Space open, Escape lets go.
    pub fn activate(&mut self, cx: &mut EventCx<'_>) {
        if !self.titles.is_empty() {
            self.highlighted = Some(0);
        }
        cx.request_focus();
        cx.request_paint();
    }

    /// Replaces the entries of menu number `index` (a check mark that follows a setting);
    /// shown from the next time the menu opens. `false` if there is no such menu.
    pub fn set_items(&mut self, index: usize, items: impl IntoIterator<Item = MenuItem>) -> bool {
        match self.titles.get_mut(index) {
            Some(title) => {
                title.items = items.into_iter().collect();
                true
            }
            None => false,
        }
    }

    /// The title of the menu open, by index.
    pub fn open_menu(&self) -> Option<usize> {
        self.open.map(|(index, _)| index)
    }

    fn title_at(&self, rect: Rect, at: Point) -> Option<usize> {
        if !rect.contains(at) {
            return None;
        }
        let x = at.x - rect.left();
        self.titles.iter().position(|t| x >= t.left && x < t.right)
    }

    fn title_rect(&self, rect: Rect, index: usize) -> Rect {
        let title = &self.titles[index];
        Rect::from_edges(
            rect.left() + title.left,
            rect.top(),
            rect.left() + title.right,
            rect.bottom(),
        )
    }

    /// Opens title `index`'s menu below it, closing another open one.
    fn open(&mut self, cx: &mut EventCx<'_>, index: usize, from_keyboard: bool) {
        if let Some((_, id)) = self.open.take() {
            cx.close_popup(id);
        }
        let mut menu = Menu::new(self.titles[index].items.iter().cloned());
        if from_keyboard {
            menu = menu.highlighting_first();
        }
        let anchor = Anchor::below(self.title_rect(cx.rect(), index));
        if let Ok(id) = cx.open_popup(menu, anchor, Menu::style()) {
            self.open = Some((index, id));
        }
        self.highlighted = Some(index);
        cx.request_paint();
    }

    fn close(&mut self, cx: &mut EventCx<'_>) {
        if let Some((_, id)) = self.open.take() {
            cx.close_popup(id);
        }
        cx.request_paint();
    }

    /// The title `step` away from `from`, wrapping around.
    fn neighbour(&self, from: usize, step: isize) -> usize {
        // Lossless: a bar has few titles, and the result is in 0..count.
        let count = self.titles.len() as isize;
        (from as isize + step).rem_euclid(count) as usize
    }
}

impl Widget for MenuBar {
    fn measure(&mut self, cx: &mut LayoutCx<'_>, _: Constraints) -> Size {
        let color = cx.theme().text;
        let padding = cx.px(TITLE_PADDING_X).round();
        let (mut x, mut height) = (0.0_f32, 0.0_f32);
        for title in &mut self.titles {
            let size = shaped(&mut title.text, cx, color);
            title.left = x;
            x += (size.width + 2.0 * padding).round();
            title.right = x;
            height = height.max(size.height);
        }
        Size::new(x.ceil(), (height + 2.0 * cx.px(TITLE_PADDING_Y)).ceil())
    }

    fn paint(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        let theme = cx.theme();
        let rect = cx.rect();
        canvas.fill_rect(rect, theme.surface);
        let line = cx.px(1.0).round();
        canvas.fill_rect(
            Rect::from_edges(
                rect.left(),
                rect.bottom() - line,
                rect.right(),
                rect.bottom(),
            ),
            theme.border,
        );
        let open = self
            .open
            .filter(|(_, id)| cx.is_popup_open(*id))
            .map(|(index, _)| index);
        for (index, title) in self.titles.iter().enumerate() {
            let area = self.title_rect(rect, index);
            if open == Some(index) {
                canvas.fill_rect(area, theme.pressed);
            } else if self.highlighted == Some(index) && (cx.is_hovered() || cx.is_focused()) {
                canvas.fill_rect(area, theme.hover);
            }
            let size = title.text.size();
            let at = crate::paint::centered(area, size.width, size.height);
            title.text.paint(canvas, at);
        }
    }

    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        if let Some((_, id)) = self.open {
            if !cx.is_popup_open(id) {
                self.open = None;
            }
        }
        match event {
            Event::Pointer(pointer) => match pointer.kind {
                PointerKind::Enter | PointerKind::Move => {
                    let at = self.title_at(cx.rect(), pointer.position);
                    match (at, self.open) {
                        (Some(index), Some((open, _))) if index != open => {
                            self.open(cx, index, false);
                        }
                        (None, Some(_)) => {}
                        _ => {
                            if self.highlighted != at {
                                self.highlighted = at;
                                cx.request_paint();
                            }
                        }
                    }
                    Handled::Yes
                }
                PointerKind::Leave => {
                    if self.open.is_none() && !cx.is_focused() {
                        self.highlighted = None;
                        cx.request_paint();
                    }
                    Handled::Yes
                }
                PointerKind::Down(PointerButton::Primary) => {
                    match (self.title_at(cx.rect(), pointer.position), self.open) {
                        (Some(index), Some((open, _))) if index == open => self.close(cx),
                        (Some(index), _) => self.open(cx, index, false),
                        (None, _) => self.close(cx),
                    }
                    Handled::Yes
                }
                PointerKind::Down(_) | PointerKind::Up(_) => Handled::Yes,
                _ => Handled::No,
            },
            Event::Key(key) if key.pressed => {
                let Key::Named(named) = key.key else {
                    return Handled::No;
                };
                let Some(at) = self.open.map(|(index, _)| index).or(self.highlighted) else {
                    return Handled::No;
                };
                match named {
                    NamedKey::ArrowLeft | NamedKey::ArrowRight => {
                        let step = if named == NamedKey::ArrowLeft { -1 } else { 1 };
                        let next = self.neighbour(at, step);
                        if self.open.is_some() {
                            self.open(cx, next, true);
                        } else {
                            self.highlighted = Some(next);
                            cx.request_paint();
                        }
                        Handled::Yes
                    }
                    NamedKey::ArrowDown | NamedKey::Enter | NamedKey::Space => {
                        self.open(cx, at, true);
                        Handled::Yes
                    }
                    NamedKey::Escape => {
                        self.highlighted = None;
                        cx.release_focus();
                        cx.request_paint();
                        Handled::Yes
                    }
                    _ => Handled::No,
                }
            }
            Event::FocusLost => {
                if self.open.is_none() {
                    self.highlighted = None;
                }
                cx.request_paint();
                Handled::Yes
            }
            _ => Handled::No,
        }
    }

    fn accessibility(&self, cx: &PaintCx<'_>, node: &mut AccessNode) {
        node.set_role(Role::MenuBar);
        let rect = cx.rect();
        let open = self
            .open
            .filter(|(_, id)| cx.is_popup_open(*id))
            .map(|(index, _)| index);
        for (index, title) in self.titles.iter().enumerate() {
            let mut item = AccessNode::new(Role::MenuItem);
            item.set_label(title.text.as_str());
            item.set_bounds(self.title_rect(rect, index));
            item.set_expanded(open == Some(index));
            item.add_action(AccessAction::Click);
            // Lossless: a bar has few titles.
            node.push_part(index as u32, item);
        }
        if let Some(index) = self.highlighted.filter(|_| cx.is_focused()) {
            // Lossless: as above.
            node.set_focused_part(index as u32);
        }
    }

    fn access_action(&mut self, cx: &mut EventCx<'_>, request: &AccessRequest<'_>) -> Handled {
        let index = request.part.map(|part| part as usize);
        match (request.action, index) {
            (AccessAction::Click, Some(index)) if index < self.titles.len() => {
                let open = self
                    .open
                    .is_some_and(|(at, id)| at == index && cx.is_popup_open(id));
                if open {
                    self.close(cx);
                } else {
                    self.open(cx, index, true);
                }
                Handled::Yes
            }
            _ => Handled::No,
        }
    }
}
