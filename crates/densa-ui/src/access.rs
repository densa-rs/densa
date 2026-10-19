//! Accessibility: what each widget is to assistive technology such as screen readers, and the
//! requests it makes of them (press this, type that, scroll there).
//!
//! A window layer turns an [`AccessTree`] into the platform's accessibility tree and hands
//! the requests it receives back with [`Ui::access_action`](crate::Ui::access_action).
//! Nothing here depends on a platform API.

use densa_core::Rect;

/// What a widget is to assistive technology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Role {
    /// A container with no meaning of its own (the default).
    #[default]
    Group,
    /// The window: the tree's root.
    Window,
    /// Static text.
    Label,
    /// Something pressed to act.
    Button,
    /// Editable text.
    TextInput,
    /// A region whose content scrolls.
    ScrollView,
    /// A divider that resizes the panes either side of it.
    Splitter,
    /// A row of tabs.
    TabList,
    /// A tab, which shows its panel when chosen.
    Tab,
    /// The content a tab shows.
    TabPanel,
    /// Rows of cells under column headers.
    Table,
    /// A row of a table.
    Row,
    /// A column's title in a table.
    ColumnHeader,
    /// A cell of a table row.
    Cell,
    /// A picture of data, such as a chart: read by its label.
    Image,
    /// A row of menu titles.
    MenuBar,
    /// A list of commands that pops up.
    Menu,
    /// A command in a menu.
    MenuItem,
    /// A menu command that is on or off (see [`AccessNode::set_checked`]).
    MenuItemCheckBox,
}

/// Something assistive technology can ask a widget to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccessAction {
    /// The equivalent of a click.
    Click,
    /// Take keyboard focus.
    Focus,
    /// Replace the text (the request carries it).
    SetValue,
    /// Scroll the content up.
    ScrollUp,
    /// Scroll the content down.
    ScrollDown,
    /// Scroll the content left.
    ScrollLeft,
    /// Scroll the content right.
    ScrollRight,
    /// Scroll whatever contains the widget so that it is in view.
    ScrollIntoView,
}

impl AccessAction {
    const ALL: [Self; 8] = [
        Self::Click,
        Self::Focus,
        Self::SetValue,
        Self::ScrollUp,
        Self::ScrollDown,
        Self::ScrollLeft,
        Self::ScrollRight,
        Self::ScrollIntoView,
    ];

    fn bit(self) -> u16 {
        1 << (self as u16)
    }
}

/// A request from assistive technology, as a widget receives it in
/// [`Widget::access_action`](crate::Widget::access_action).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AccessRequest<'a> {
    /// What to do.
    pub action: AccessAction,
    /// The part of the widget it is for (see [`AccessNode::push_part`]), or the widget itself.
    pub part: Option<u32>,
    /// The text, for [`AccessAction::SetValue`].
    pub value: Option<&'a str>,
}

/// A node of the accessibility tree: a widget, or a part of one (such as a dock's tab).
///
/// Bits a node does not support are not asked of it: [`AccessNode::add_action`] says which
/// requests it handles.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AccessNode {
    role: Role,
    label: Option<String>,
    value: Option<String>,
    bounds: Rect,
    actions: u16,
    selected: Option<bool>,
    checked: Option<bool>,
    expanded: Option<bool>,
    disabled: bool,
    clips: bool,
    placeholder: Option<String>,
    focused_part: Option<u32>,
    parts: Vec<(u32, AccessNode)>,
    children: Vec<AccessId>,
}

impl AccessNode {
    /// A node with `role`.
    pub fn new(role: Role) -> Self {
        Self {
            role,
            ..Self::default()
        }
    }

    /// Sets what the node is.
    pub fn set_role(&mut self, role: Role) {
        self.role = role;
    }

    /// Sets its name, such as a button's text. Static text goes in
    /// [`set_value`](Self::set_value).
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = Some(label.into());
    }

    /// Sets its text content: a label's text, a text input's text.
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = Some(value.into());
    }

    /// Sets where it is, in physical pixels from the window's top-left corner (a widget's
    /// node starts with the widget's rectangle).
    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
    }

    /// Says it handles `action`.
    pub fn add_action(&mut self, action: AccessAction) {
        self.actions |= action.bit();
    }

    /// Sets whether it is the chosen one of its kind, such as the shown tab.
    pub fn set_selected(&mut self, selected: bool) {
        self.selected = Some(selected);
    }

    /// Sets whether it is on, for something that is on or off (a check menu item).
    pub fn set_checked(&mut self, checked: bool) {
        self.checked = Some(checked);
    }

    /// Sets whether what it opens is open, for something that opens more (a menu title, an
    /// item with a submenu).
    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = Some(expanded);
    }

    /// Whether it is on, if it is something that is on or off.
    pub fn checked(&self) -> Option<bool> {
        self.checked
    }

    /// Whether what it opens is open, if it opens something.
    pub fn expanded(&self) -> Option<bool> {
        self.expanded
    }

    /// Sets whether it is unavailable.
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
    }

    /// Says which of its parts has keyboard focus while the widget does (such as a dock's
    /// chosen tab), so assistive technology reads that part rather than the whole widget.
    pub fn set_focused_part(&mut self, part: u32) {
        self.focused_part = Some(part);
    }

    /// The part with keyboard focus while the widget has it.
    pub fn focused_part(&self) -> Option<u32> {
        self.focused_part
    }

    /// Adds `node` as a part of this one, numbered `part`: a piece that is not a widget of its
    /// own, such as a tab of a dock. Requests for it come to the widget with that number.
    /// Parts come before the widget's children. Numbers must be unique within the widget, at
    /// any depth, and at most [`MAX_PART`]; a part breaking either rule is left out. Keep a
    /// part's number while it lasts (number by identity, not by position), or a screen reader
    /// tracking it follows another one.
    pub fn push_part(&mut self, part: u32, node: AccessNode) {
        self.parts.push((part, node));
    }

    /// What the node is.
    pub fn role(&self) -> Role {
        self.role
    }

    /// Its name.
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Its text content.
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Where it is, in physical pixels.
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Whether it handles `action`.
    pub fn has_action(&self, action: AccessAction) -> bool {
        self.actions & action.bit() != 0
    }

    /// The actions it handles.
    pub fn actions(&self) -> impl Iterator<Item = AccessAction> + '_ {
        AccessAction::ALL
            .into_iter()
            .filter(|a| self.has_action(*a))
    }

    /// Whether it is the chosen one of its kind, if that applies.
    pub fn selected(&self) -> Option<bool> {
        self.selected
    }

    /// Whether it is unavailable.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Whether it cuts off its children where they leave its bounds (it scrolls or clips),
    /// so children outside it are off screen.
    pub fn clips_children(&self) -> bool {
        self.clips
    }

    /// Sets whether it cuts off its children where they leave its bounds (a widget's node
    /// starts with this set when its style clips or scrolls).
    pub fn set_clips_children(&mut self, clips: bool) {
        self.clips = clips;
    }

    /// Sets the hint shown while it is empty, such as a text input's placeholder.
    pub fn set_placeholder(&mut self, placeholder: impl Into<String>) {
        self.placeholder = Some(placeholder.into());
    }

    /// The hint shown while it is empty.
    pub fn placeholder(&self) -> Option<&str> {
        self.placeholder.as_deref()
    }

    /// Its children in the tree, in order (filled in by the [`Ui`](crate::Ui)).
    pub fn children(&self) -> &[AccessId] {
        &self.children
    }

    pub(crate) fn take_parts(&mut self) -> Vec<(u32, AccessNode)> {
        std::mem::take(&mut self.parts)
    }

    pub(crate) fn push_child(&mut self, child: AccessId) {
        self.children.push(child);
    }
}

/// The highest part number a widget can use (see [`AccessNode::push_part`]).
pub const MAX_PART: u32 = (1 << PART_BITS) - 2;

/// Bits of an [`AccessId`] numbering a widget's parts; the rest number widgets.
const PART_BITS: u32 = 24;

/// Identifies a node of the accessibility tree: a widget, or a part of one. A widget's id is
/// never given to another widget, even one in its slot after it is removed, so a request for
/// a removed widget reaches nothing. Get one from [`Ui::access_id`](crate::Ui::access_id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccessId(u64);

impl AccessId {
    /// The node of the widget numbered `serial`.
    pub(crate) fn widget(serial: u64) -> Self {
        Self(serial << PART_BITS)
    }

    /// The node of part `part` of this widget's node (see [`AccessNode::push_part`]); parts
    /// past [`MAX_PART`] have no node, and get the widget's.
    #[must_use]
    pub fn part(self, part: u32) -> Self {
        if part > MAX_PART {
            return self;
        }
        // Offset by one: part 0 differs from the widget itself.
        Self(self.0 & !Self::PART_MASK | u64::from(part + 1))
    }

    const PART_MASK: u64 = (1 << PART_BITS) - 1;

    /// The id as a number, for platform APIs.
    pub fn to_raw(self) -> u64 {
        self.0
    }

    /// The id a [`to_raw`](Self::to_raw) number came from.
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The widget's serial and the part (`None` for the widget itself).
    pub(crate) fn split(self) -> (u64, Option<u32>) {
        let part = (self.0 & Self::PART_MASK) as u32;
        (self.0 >> PART_BITS, part.checked_sub(1))
    }
}

/// The accessibility tree of a [`Ui`](crate::Ui): every shown widget (and the parts widgets
/// add), parents before children.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessTree {
    pub(crate) nodes: Vec<(AccessId, AccessNode)>,
    pub(crate) root: AccessId,
    pub(crate) focus: AccessId,
}

impl AccessTree {
    /// Every node and its id.
    pub fn nodes(&self) -> &[(AccessId, AccessNode)] {
        &self.nodes
    }

    /// The root's id: the [`Ui`](crate::Ui)'s root widget.
    pub fn root(&self) -> AccessId {
        self.root
    }

    /// The node with keyboard focus (the root if no widget has it).
    pub fn focus(&self) -> AccessId {
        self.focus
    }

    /// The node `id`.
    pub fn node(&self, id: AccessId) -> Option<&AccessNode> {
        self.nodes.iter().find(|(i, _)| *i == id).map(|(_, n)| n)
    }
}
