//! Docking: panels in tab groups, side by side or stacked, rearranged by dragging tabs.

mod access;
mod geometry;
mod input;
mod layout;
mod tree;

use densa_core::{Color, Point, Rect, Size};
use densa_draw::Canvas;
use densa_ui::{
    AccessNode, AccessRequest, Event, EventCx, Handled, LayoutCx, Length, Overflow, PaintCx,
    PointerButton, PointerKind, Style, Ui, UiError, Widget, WidgetId,
};

pub use self::layout::{
    DockAxis, DockLayout, DockLayoutError, DockLayoutErrorKind, MAX_LAYOUT_DEPTH,
};

use self::geometry::{bar, beside, content_style, divider_rect, group_rect, hairline};
use crate::paint::{centered, outline};
use crate::text::Text;

/// Space before a title cut off by its shrunk tab, in logical pixels.
const TITLE_INSET: f32 = 6.0;

/// Where [`DockArea::add_panel`] puts a panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DockAt {
    /// As another tab of the first group (a new group if the dock is empty).
    First,
    /// As another tab of `panel`'s group.
    With(WidgetId),
    /// In a new group on `side` of `panel`'s group, taking half its space.
    Beside(WidgetId, DockSide),
}

/// A side of a dock group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DockSide {
    /// Left.
    Left,
    /// Right.
    Right,
    /// Above.
    Top,
    /// Below.
    Bottom,
}

/// What a dock shows for a panel: its tab's title and, for saving and restoring workspaces,
/// a key that stays the same across runs (widget ids do not, and titles need not be unique).
///
/// `&str` and `String` convert into a `DockPanel` with that title and no key.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DockPanel {
    /// The tab's title.
    pub title: String,
    /// The application's name for the panel, such as `"orders"`.
    pub key: Option<String>,
}

impl DockPanel {
    /// A panel titled `title`, without a key.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            key: None,
        }
    }

    /// The panel with `key`. Give each panel its own key: panels sharing one take each
    /// other's places in turn when a layout is restored.
    #[must_use]
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl From<&str> for DockPanel {
    fn from(title: &str) -> Self {
        Self::new(title)
    }
}

impl From<String> for DockPanel {
    fn from(title: String) -> Self {
        Self::new(title)
    }
}

/// Which way a split's parts sit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    /// Side by side.
    Row,
    /// One above the other.
    Column,
}

/// Part of the dock, as fractions of its rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Frac {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Frac {
    const WHOLE: Self = Self {
        x0: 0.0,
        y0: 0.0,
        x1: 1.0,
        y1: 1.0,
    };
}

#[derive(Debug, Clone)]
enum Kind {
    /// Two parts sharing a rectangle; `ratio` of it is the first's.
    Split {
        axis: Axis,
        ratio: f32,
        first: usize,
        second: usize,
    },
    /// Tabs (indices into `DockArea::tabs`), the position of the shown one among them, and
    /// how far the tab bar is scrolled, in logical pixels (when its tabs do not fit).
    Group {
        tabs: Vec<usize>,
        active: usize,
        scroll: f32,
        /// The group's number for assistive technology: kept while it lives.
        serial: u32,
    },
}

#[derive(Debug, Clone)]
struct Node {
    kind: Kind,
    parent: Option<usize>,
}

/// A panel's tab: its widget, title, key and group.
#[derive(Debug)]
struct Tab {
    widget: WidgetId,
    title: Text,
    key: Option<String>,
    group: usize,
    /// The tab's number for assistive technology: kept while it lives, whatever its index.
    serial: u32,
}

/// The divider between a split's two parts.
#[derive(Debug, Clone, Copy)]
struct Divider {
    split: usize,
    axis: Axis,
    ratio: f32,
    /// The split's whole area.
    area: Frac,
}

/// Where a dragged tab would land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// Into the group: before the tab at the position (dropped on its tab bar), or at the end
    /// (`None`: dropped in its middle, where its own tab is already in it).
    Into(usize, Option<usize>),
    /// Beside the group, in a new one.
    Beside(usize, DockSide),
}

#[derive(Debug, Clone, Copy)]
enum Drag {
    /// A tab pressed at `from` that has not moved far: a click so far.
    Pressed { tab: usize, from: Point },
    /// A tab being dragged, and where it would land.
    Tab { tab: usize, target: Option<Target> },
    /// A split's divider being dragged: grabbed `grab` from its middle; whether it moved.
    Divider {
        split: usize,
        grab: f32,
        moved: bool,
    },
}

/// What a point of the dock is on.
#[derive(Debug, Clone, Copy)]
enum Found {
    Tab(usize, Rect),
    Divider(usize),
    Group(usize, Rect),
    /// A scroll button of a group's tab bar: forward (`true`) or back.
    ScrollTabs(usize, bool),
    Nothing,
}

/// Panels in tab groups, side by side or stacked, that the user rearranges: click a tab to
/// show its panel, drag it along its bar to reorder, onto another group to join that group,
/// or near a group's edge to split the group, and drag the gaps between groups to resize
/// them. Tabs that do not fit their bar shrink, and the shown tab always stays in view.
///
/// The panels are the dock's children, added with [`DockArea::add_panel`] and removed with
/// [`DockArea::remove_panel`] (a panel removed from the [`Ui`] directly is forgotten at the
/// dock's next event). The dock owns their styles, which place and hide them, so style what
/// goes inside a panel rather than the panel itself, and leave the dock's own padding at
/// zero. A hidden panel keeps its state, scroll positions included. Give the dock
/// [`DockArea::style`].
///
/// The arrangement of panels added with keys ([`DockPanel::with_key`]) is saved with
/// [`DockArea::layout`] and put back, in this run or the next, with
/// [`DockArea::restore_layout`]; a [`DockLayout`] reads and writes itself as a line of text.
///
/// ```
/// use densa_core::Size;
/// use densa_paragraph::TextSystem;
/// use densa_ui::Ui;
/// use densa_widgets::{DockArea, DockAt, DockPanel, DockSide, Label, Panel};
///
/// let mut ui = Ui::new(TextSystem::new()?);
/// ui.resize(Size::new(1200.0, 800.0));
/// let dock = ui.add(ui.root(), DockArea::new(), DockArea::style())?;
/// let quotes = DockArea::add_panel(&mut ui, dock, Panel::new(), "Quotes", DockAt::First)?;
/// let right = DockAt::Beside(quotes, DockSide::Right);
/// let chart = DockPanel::new("Chart").with_key("chart");
/// let chart = DockArea::add_panel(&mut ui, dock, Panel::new(), chart, right)?;
/// DockArea::add_panel(&mut ui, dock, Panel::new(), "Trades", DockAt::With(quotes))?;
/// ui.add(chart, Label::new("Panel content goes inside the panel"), Default::default())?;
/// ui.layout();
/// let area = ui.widget::<DockArea>(dock).unwrap();
/// assert_eq!(area.group_count(), 2);
/// assert!(!area.is_shown(quotes), "Trades, added last, is the tab shown");
/// assert_eq!(area.panel_with_key("chart"), Some(chart));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// It reports [`ActionKind::Selected`](densa_ui::ActionKind::Selected) with the panel's
/// position in [`DockArea::panels`] (valid until a panel is removed) when a click shows a
/// panel, and [`ActionKind::Changed`](densa_ui::ActionKind::Changed) when a drag moved a tab
/// or a divider.
#[derive(Debug, Default)]
pub struct DockArea {
    /// Splits and groups; `None` slots are free.
    nodes: Vec<Option<Node>>,
    root: Option<usize>,
    tabs: Vec<Tab>,
    /// The groups and where they are, recomputed when the arrangement changes.
    frames: Vec<(usize, Frac)>,
    dividers: Vec<Divider>,
    /// Reused by `arrange`, which runs on every move of a divider drag.
    stack: Vec<(usize, Frac)>,
    drag: Option<Drag>,
    /// The tab under the pointer.
    hovered: Option<usize>,
    /// The scale the panels were last styled at.
    styled_scale: f32,
    /// The group whose tabs the keyboard moves between (the last one clicked or chosen).
    focus_group: Option<usize>,
    /// The last serial given to a tab or a group.
    last_serial: u32,
}

impl DockArea {
    /// An empty dock.
    pub fn new() -> Self {
        Self::default()
    }

    /// A dock filling its parent's free space, allowed to be smaller than its panels' content
    /// and clipping them.
    pub fn style() -> Style {
        Style::column()
            .grow(1.0)
            .min_size(Length::Px(0.0), Length::Px(0.0))
            .overflow(Overflow::Clip)
    }

    /// Adds `widget` to `dock` as a panel (a title, or a [`DockPanel`]), placed `at`, and
    /// shows it.
    ///
    /// # Errors
    /// [`UiError::NoSuchWidget`] if `dock` is gone or is not a [`DockArea`], or if `at`
    /// names a widget that is not one of its panels; [`UiError::TooManyWidgets`] as for
    /// [`Ui::add`].
    pub fn add_panel(
        ui: &mut Ui,
        dock: WidgetId,
        widget: impl Widget,
        panel: impl Into<DockPanel>,
        at: DockAt,
    ) -> Result<WidgetId, UiError> {
        let area = ui
            .widget::<DockArea>(dock)
            .ok_or(UiError::NoSuchWidget(dock))?;
        if let DockAt::With(other) | DockAt::Beside(other, _) = at {
            if area.index_of(other).is_none() {
                return Err(UiError::NoSuchWidget(other));
            }
        }
        let id = ui.add(dock, widget, Style::default().hidden(true))?;
        let panel = panel.into();
        ui.update::<DockArea, _>(dock, |area, cx| area.attach(cx, id, panel, at));
        Ok(id)
    }

    /// Removes `panel` from `dock` and from the `Ui`; its group shows its next tab, and an
    /// emptied group's space goes to its neighbor.
    ///
    /// # Errors
    /// [`UiError::NoSuchWidget`] if `dock` is gone or is not a [`DockArea`], or `panel` is
    /// not one of its panels.
    pub fn remove_panel(ui: &mut Ui, dock: WidgetId, panel: WidgetId) -> Result<(), UiError> {
        let removed = ui
            .update::<DockArea, _>(dock, |area, cx| {
                let index = area.index_of(panel)?;
                area.drop_tab(cx, index);
                Some(())
            })
            .ok_or(UiError::NoSuchWidget(dock))?;
        removed.ok_or(UiError::NoSuchWidget(panel))?;
        ui.remove(panel);
        Ok(())
    }

    /// Shows `panel` in its group.
    ///
    /// # Errors
    /// [`UiError::NoSuchWidget`] if `dock` is gone or is not a [`DockArea`], or `panel` is
    /// not one of its panels.
    pub fn show_panel(ui: &mut Ui, dock: WidgetId, panel: WidgetId) -> Result<(), UiError> {
        let shown = ui
            .update::<DockArea, _>(dock, |area, cx| area.show(cx, panel))
            .ok_or(UiError::NoSuchWidget(dock))?;
        shown.then_some(()).ok_or(UiError::NoSuchWidget(panel))
    }

    /// The panels' widgets, in the order they were added (removals close the gaps).
    pub fn panels(&self) -> impl ExactSizeIterator<Item = WidgetId> + '_ {
        self.tabs.iter().map(|t| t.widget)
    }

    /// Whether `panel` is the one its group shows.
    pub fn is_shown(&self, panel: WidgetId) -> bool {
        self.index_of(panel).is_some_and(|i| self.is_active(i))
    }

    /// Shows `panel` in its group (from an [`Ui::update`] on the dock); `false` if it is not
    /// one of the dock's panels.
    pub fn show(&mut self, cx: &mut EventCx<'_>, panel: WidgetId) -> bool {
        let Some(index) = self.index_of(panel) else {
            return false;
        };
        self.activate(index);
        self.reveal(self.tabs[index].group, cx.rect(), cx.scale());
        self.restyle(cx);
        true
    }

    /// Changes `panel`'s tab title (from an [`Ui::update`] on the dock); `false` if it is not
    /// one of the dock's panels.
    pub fn set_title(
        &mut self,
        cx: &mut EventCx<'_>,
        panel: WidgetId,
        title: impl Into<String>,
    ) -> bool {
        let Some(index) = self.index_of(panel) else {
            return false;
        };
        self.tabs[index].title.set(title);
        // Shaped in `layout`, which keeps the shown tab in view.
        cx.request_layout();
        cx.request_paint();
        true
    }

    /// `panel`'s tab title.
    pub fn title(&self, panel: WidgetId) -> Option<&str> {
        self.index_of(panel).map(|i| self.tabs[i].title.as_str())
    }

    /// The key `panel` was added with.
    pub fn key_of(&self, panel: WidgetId) -> Option<&str> {
        self.index_of(panel)
            .and_then(|i| self.tabs[i].key.as_deref())
    }

    /// The first panel added with `key`.
    pub fn panel_with_key(&self, key: &str) -> Option<WidgetId> {
        self.tabs
            .iter()
            .find(|t| t.key.as_deref() == Some(key))
            .map(|t| t.widget)
    }

    /// How many groups the panels are in.
    pub fn group_count(&self) -> usize {
        self.frames.len()
    }

    /// The panels sharing `panel`'s group, in tab order; none if it is not a panel.
    pub fn tabs_with(&self, panel: WidgetId) -> impl Iterator<Item = WidgetId> + '_ {
        let tabs = self
            .index_of(panel)
            .map_or(&[][..], |i| self.group_tabs(self.tabs[i].group).0);
        tabs.iter().map(|&i| self.tabs[i].widget)
    }

    /// Where `panel`'s tab is drawn when the dock is at `dock` (its rectangle) and `scale`;
    /// `None` if it is not a panel or its tab is scrolled out of its bar.
    pub fn tab_rect(&self, panel: WidgetId, dock: Rect, scale: f32) -> Option<Rect> {
        self.tab_rect_of(self.index_of(panel)?, dock, scale)
    }

    fn index_of(&self, panel: WidgetId) -> Option<usize> {
        self.tabs.iter().position(|t| t.widget == panel)
    }

    fn attach(&mut self, cx: &mut EventCx<'_>, widget: WidgetId, panel: DockPanel, at: DockAt) {
        let index = self.tabs.len();
        let serial = self.next_serial();
        self.tabs.push(Tab {
            widget,
            title: Text::new(panel.title),
            key: panel.key,
            group: usize::MAX,
            serial,
        });
        let first = self.frames.first().map(|&(group, _)| group);
        let other = match at {
            DockAt::With(other) | DockAt::Beside(other, _) => self.index_of(other),
            DockAt::First => None,
        };
        match (at, other) {
            (DockAt::Beside(_, side), Some(other)) => {
                let group = self.tabs[other].group;
                self.split(index, group, side);
            }
            (DockAt::With(_), Some(other)) => {
                let group = self.tabs[other].group;
                self.join(index, Some(group), None);
            }
            _ => self.join(index, first, None),
        }
        self.arrange();
        self.restyle(cx);
        // The new title is shaped in `layout`, which then brings it into view.
        cx.request_layout();
    }

    /// Shows each group's active panel where its group is, and hides the others; panels whose
    /// style is already right are left alone (setting a style lays the panel out again).
    fn restyle(&mut self, cx: &mut EventCx<'_>) {
        let scale = cx.scale();
        self.styled_scale = scale;
        for (index, tab) in self.tabs.iter().enumerate() {
            let frame = self.frames.iter().find(|(g, _)| *g == tab.group);
            let style = match frame {
                Some(&(_, frac)) if self.is_active(index) => content_style(frac, scale),
                _ => Style::default().hidden(true),
            };
            if cx.style_of(tab.widget) != Some(&style) {
                cx.set_style(tab.widget, style);
            }
        }
        cx.request_paint();
    }

    /// Paints a group's frame and tab bar; the shown tab takes its panel's color, so it reads
    /// as joined to the panel below.
    fn paint_group(&self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>, group: usize, area: Rect) {
        let (theme, scale) = (cx.theme(), cx.scale());
        let line = hairline(scale);
        let bar = bar(area, scale);
        canvas.fill_rect(area, theme.surface);
        let strip = Rect::from_edges(area.left(), area.top(), area.right(), bar.bottom());
        canvas.fill_rect(strip, theme.background);
        outline(canvas, area, line, theme.border);
        let under = Rect::from_edges(area.left(), bar.bottom() - line, area.right(), bar.bottom());
        canvas.fill_rect(under, theme.border);
        let strip = self.strip(group, bar, scale);
        if let Some((back, forward)) = strip.buttons {
            let can = |yes: bool| if yes { theme.text } else { theme.text_dim };
            chevron(canvas, back, line, false, can(strip.offset > 0.0));
            chevron(
                canvas,
                forward,
                line,
                true,
                can(strip.offset < strip.max_offset),
            );
        }
        let saved = canvas.state();
        canvas.clip(strip.tabs);
        let (_, active) = self.group_tabs(group);
        for (position, (index, tab)) in self.tab_rects(group, bar, scale).enumerate() {
            // A tab whose panel was removed from the Ui directly, before the dock forgets it.
            if tab.is_empty() || cx.rect_of(self.tabs[index].widget).is_none() {
                continue;
            }
            if position == active {
                canvas.fill_rect(tab, theme.surface);
                if cx.is_focused() && self.focused_group() == Some(group) {
                    outline(canvas, tab, 2.0 * line, theme.focus);
                }
                let accent =
                    Rect::from_edges(tab.left(), tab.top(), tab.right(), tab.top() + 2.0 * line);
                canvas.fill_rect(accent, theme.accent);
            } else if self.hovered == Some(index) {
                let above = Rect::from_edges(tab.left(), tab.top(), tab.right(), under.top());
                canvas.fill_rect(above, theme.hover);
            }
            let title = &self.tabs[index].title;
            let size = title.size();
            let saved = canvas.state();
            canvas.clip(tab);
            let mut at = centered(tab, size.width, size.height);
            // A title wider than its shrunk tab shows its start, not its middle.
            let inset = (TITLE_INSET * scale).round();
            if size.width > tab.width() - 2.0 * inset {
                at.x = tab.left() + inset;
            }
            title.paint(canvas, at);
            canvas.restore(saved);
        }
        canvas.restore(saved);
    }

    /// Paints where a dragged tab would land: the new group's place beside a group, or a
    /// marker between tabs (and the group, when it is another one).
    fn paint_target(&self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>, tab: usize, target: Target) {
        if !self.changes(tab, target) {
            return;
        }
        let (rect, scale) = (cx.rect(), cx.scale());
        let (Target::Into(group, _) | Target::Beside(group, _)) = target;
        let Some(&(_, frac)) = self.frames.iter().find(|(g, _)| *g == group) else {
            return;
        };
        let area = group_rect(frac, rect, scale);
        let accent = cx.theme().accent;
        let line = hairline(scale);
        match target {
            Target::Beside(_, side) => {
                let shown = beside(frac, side, rect, scale);
                canvas.fill_rect(shown, accent.with_alpha(0.22));
                outline(canvas, shown, 2.0 * line, accent);
            }
            Target::Into(_, position) => {
                if group != self.tabs[tab].group {
                    canvas.fill_rect(area, accent.with_alpha(0.12));
                }
                let bar = bar(area, scale);
                let mut x = bar.right();
                if let Some(position) = position {
                    let slot = self.tab_rects(group, bar, scale).nth(position);
                    if let Some((_, slot)) = slot {
                        x = slot.left();
                    } else if let Some((_, last)) = self.tab_rects(group, bar, scale).last() {
                        x = last.right();
                    }
                } else if let Some((_, last)) = self.tab_rects(group, bar, scale).last() {
                    x = last.right();
                }
                let x = x.clamp(bar.left(), bar.right() - 2.0 * line);
                let marker = Rect::from_edges(x, bar.top(), x + 2.0 * line, bar.bottom());
                canvas.fill_rect(marker, accent);
            }
        }
    }
}

impl Widget for DockArea {
    fn accessibility(&self, cx: &PaintCx<'_>, node: &mut AccessNode) {
        self.describe(cx, node);
    }

    fn access_action(&mut self, cx: &mut EventCx<'_>, request: &AccessRequest<'_>) -> Handled {
        self.act(cx, request)
    }

    /// Reachable by Tab, as a tab list; clicks on panels do not focus it (a tab press does).
    fn accepts_focus(&self) -> bool {
        !self.tabs.is_empty()
    }

    fn focus_on_press(&self) -> bool {
        false
    }

    fn layout(&mut self, cx: &mut LayoutCx<'_>, size: Size) {
        let color = cx.theme().text;
        for tab in &mut self.tabs {
            tab.title.fit(cx, f32::INFINITY, color);
        }
        // New titles, or a new size: keep every group's shown tab in view. Widths do not
        // depend on where the dock is, only on its size.
        let rect = Rect::new(0.0, 0.0, size.width, size.height);
        for i in 0..self.frames.len() {
            let group = self.frames[i].0;
            self.reveal(group, rect, cx.scale());
        }
    }

    fn paint(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        let (rect, scale) = (cx.rect(), cx.scale());
        canvas.fill_rect(rect, cx.theme().background);
        for &(group, frac) in &self.frames {
            self.paint_group(cx, canvas, group, group_rect(frac, rect, scale));
        }
        if let Some(Drag::Divider { split, .. }) = self.drag {
            if let Some(divider) = self.dividers.iter().find(|d| d.split == split) {
                let strip = divider_rect(divider, rect, scale);
                canvas.fill_rect(strip, cx.theme().accent.with_alpha(0.6));
            }
        }
    }

    fn paint_over(&mut self, cx: &PaintCx<'_>, canvas: &mut Canvas<'_>) {
        if let Some(Drag::Tab {
            tab,
            target: Some(target),
        }) = self.drag
        {
            self.paint_target(cx, canvas, tab, target);
        }
    }

    fn event(&mut self, cx: &mut EventCx<'_>, event: &Event<'_>) -> Handled {
        self.prune(cx);
        // Panel margins are in whole physical pixels: restyle after a scale change.
        if cx.scale() != self.styled_scale {
            self.restyle(cx);
        }
        let pointer = match event {
            Event::Pointer(pointer) => pointer,
            Event::Key(key) => return self.key(cx, key),
            Event::FocusGained | Event::FocusLost => {
                cx.request_paint();
                return Handled::No;
            }
            _ => return Handled::No,
        };
        match pointer.kind {
            PointerKind::Down(PointerButton::Primary) => self.press(cx, pointer.position),
            PointerKind::Move => self.drag_to(cx, pointer.position),
            // Any release ends the press (the Ui stops sending the dock the pointer); only
            // the primary button's drops a dragged tab.
            PointerKind::Up(button) => {
                let handled = self.finish(cx, button == PointerButton::Primary);
                // Released over a divider: it can be grabbed again right there.
                if let Found::Divider(i) = self.find(pointer.position, cx.rect(), cx.scale()) {
                    cx.set_cursor(Some(input::resize_cursor(self.dividers[i].axis)));
                }
                handled
            }
            PointerKind::Cancel => self.finish(cx, false),
            PointerKind::Scroll(delta) => self.wheel(cx, pointer.position, delta),
            PointerKind::Leave => {
                if self.drag.is_none() {
                    cx.set_cursor(None);
                }
                self.hover(cx, None);
                Handled::No
            }
            _ => Handled::No,
        }
    }
}

/// A chevron pointing back (`<`) or forward (`>`) in the middle of `button`, drawn as a stack
/// of `line`-high rectangles.
fn chevron(canvas: &mut Canvas<'_>, button: Rect, line: f32, forward: bool, color: Color) {
    let rows = 7_u8;
    let reach = f32::from(rows / 2) * line;
    let cx = ((button.left() + button.right()) / 2.0).round();
    let top = ((button.top() + button.bottom()) / 2.0 - reach - line / 2.0).round();
    for row in 0..rows {
        let step = f32::from(row.abs_diff(rows / 2)) * line;
        // `>`: the middle row furthest right; `<`: furthest left.
        let x = if forward {
            cx - reach / 2.0 + (reach - step)
        } else {
            cx - reach / 2.0 + step
        };
        let y = top + f32::from(row) * line;
        canvas.fill_rect(Rect::from_edges(x, y, x + 2.0 * line, y + line), color);
    }
}
