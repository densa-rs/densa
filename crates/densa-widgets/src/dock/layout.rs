//! Saving and restoring how a dock's panels are arranged, by the panels' keys.

use std::fmt;
use std::str::FromStr;

use densa_ui::{EventCx, Ui, UiError, WidgetId};

use super::{Axis, DockArea, Kind, Node};

/// How deeply splits may nest in a layout: far more than any workspace has, few enough that
/// reading a damaged or hostile layout cannot exhaust the stack. [`DockArea::layout`] keeps
/// within it (deeper groups are saved as one), and reading refuses deeper text.
pub const MAX_LAYOUT_DEPTH: usize = 64;

/// The format's name, before its version.
const FORMAT: &str = "densa-dock";
/// The version written, and the only one read.
const VERSION: &str = "1";
/// The smallest share of a split either part keeps when restored, so no group vanishes.
const MIN_RATIO: f32 = 0.01;

/// Which way a split's two parts sit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockAxis {
    /// Side by side: the first part on the left.
    Row,
    /// One above the other: the first part on top.
    Column,
}

/// How a dock's panels are arranged, naming panels by their keys
/// ([`DockPanel::with_key`](super::DockPanel::with_key)): what
/// [`DockArea::layout`] saves and [`DockArea::restore_layout`] puts back, for example when an
/// application starts again.
///
/// It is written as text (`to_string()`) and read back with [`str::parse`]:
/// `densa-dock 1 (row 0.6 (tabs 0 "orders" "trades") (tabs 0 "chart"))`, a version then the
/// tree, each split with its axis and the fraction its first part takes, each group with the
/// position of its shown tab and its panels' keys (quoted, `\"` and `\\` escaped). Layouts
/// built in code are written however deep they are, but only those within
/// [`MAX_LAYOUT_DEPTH`] read back.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DockLayout {
    /// Two parts sharing a rectangle along `axis`; `ratio` (0 to 1) of it is the first's.
    Split {
        /// Which way the parts sit.
        axis: DockAxis,
        /// The fraction of the rectangle the first part takes.
        ratio: f32,
        /// The left or upper part.
        first: Box<DockLayout>,
        /// The right or lower part.
        second: Box<DockLayout>,
    },
    /// A group of tabs: its panels' keys in tab order, and the position of the shown one.
    Tabs {
        /// The panels' keys, in tab order.
        panels: Vec<String>,
        /// Which of them is shown.
        shown: usize,
    },
}

/// Why text is not a [`DockLayout`], and where reading stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DockLayoutError {
    /// The byte offset in the text where reading stopped.
    pub at: usize,
    /// What was wrong there.
    pub kind: DockLayoutErrorKind,
}

/// What was wrong with a layout's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DockLayoutErrorKind {
    /// It does not start as a dock layout.
    NotALayout,
    /// A dock layout of another version, written by a later densa.
    UnsupportedVersion,
    /// `(` was expected.
    ExpectedOpen,
    /// `)` was expected.
    ExpectedClose,
    /// `row`, `column` or `tabs` was expected.
    ExpectedNode,
    /// A split's ratio is not a number from 0 to 1.
    BadRatio,
    /// A group's shown tab is not a position.
    BadShown,
    /// A group has no panels, or shows one it does not have.
    ShownMissing,
    /// A key has no closing quote.
    UnclosedKey,
    /// A key has an escape other than `\"` and `\\`.
    BadEscape,
    /// Splits nest deeper than [`MAX_LAYOUT_DEPTH`].
    TooDeep,
    /// Text follows the layout.
    TrailingText,
}

impl fmt::Display for DockLayoutErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotALayout => "not a densa dock layout",
            Self::UnsupportedVersion => "a dock layout of a later version",
            Self::ExpectedOpen => "expected `(`",
            Self::ExpectedClose => "expected `)`",
            Self::ExpectedNode => "expected `row`, `column` or `tabs`",
            Self::BadRatio => "expected a ratio from 0 to 1",
            Self::BadShown => "expected the shown tab's position",
            Self::ShownMissing => "a group must show one of its panels",
            Self::UnclosedKey => "a key without its closing quote",
            Self::BadEscape => "only `\\\"` and `\\\\` are escapes",
            Self::TooDeep => "splits nested too deeply",
            Self::TrailingText => "text after the layout",
        })
    }
}

impl fmt::Display for DockLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid dock layout at byte {}: {}", self.at, self.kind)
    }
}

impl std::error::Error for DockLayoutError {}

impl fmt::Display for DockLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{FORMAT} {VERSION} ")?;
        write_node(f, self)
    }
}

fn write_node(f: &mut fmt::Formatter<'_>, layout: &DockLayout) -> fmt::Result {
    match layout {
        DockLayout::Split {
            axis,
            ratio,
            first,
            second,
        } => {
            let axis = match axis {
                DockAxis::Row => "row",
                DockAxis::Column => "column",
            };
            // `{}` writes the shortest text that reads back as the same f32; adding 0 turns
            // -0 into 0.
            write!(f, "({axis} {} ", ratio + 0.0)?;
            write_node(f, first)?;
            f.write_str(" ")?;
            write_node(f, second)?;
            f.write_str(")")
        }
        DockLayout::Tabs { panels, shown } => {
            write!(f, "(tabs {shown}")?;
            for key in panels {
                f.write_str(" \"")?;
                for c in key.chars() {
                    if c == '"' || c == '\\' {
                        f.write_str("\\")?;
                    }
                    write!(f, "{c}")?;
                }
                f.write_str("\"")?;
            }
            f.write_str(")")
        }
    }
}

impl FromStr for DockLayout {
    type Err = DockLayoutError;

    /// Reads a layout written by `to_string()`.
    ///
    /// # Errors
    /// [`DockLayoutError`] for another format or version, a ratio that is not a number from
    /// 0 to 1, a group without panels or showing a tab it lacks, splits nested deeper than
    /// [`MAX_LAYOUT_DEPTH`], or anything after the layout; see [`DockLayoutErrorKind`].
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        use DockLayoutErrorKind::{NotALayout, TrailingText, UnsupportedVersion};
        let mut reader = Reader { text, at: 0 };
        reader.skip_space();
        if !reader.rest().starts_with(FORMAT) {
            return reader.fail(NotALayout);
        }
        reader.at += FORMAT.len();
        if !reader.rest().starts_with(char::is_whitespace) {
            return reader.fail(NotALayout);
        }
        reader.skip_space();
        let start = reader.at;
        let version = reader.word();
        let known = version == VERSION;
        let numbered = !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit());
        if !known || !reader.rest().starts_with(char::is_whitespace) {
            reader.at = start;
            return reader.fail(if numbered && !known {
                UnsupportedVersion
            } else {
                NotALayout
            });
        }
        let layout = reader.node(0)?;
        reader.skip_space();
        if !reader.rest().is_empty() {
            return reader.fail(TrailingText);
        }
        Ok(layout)
    }
}

/// Reads a layout from its text, a token at a time.
struct Reader<'a> {
    text: &'a str,
    at: usize,
}

impl Reader<'_> {
    fn rest(&self) -> &str {
        &self.text[self.at..]
    }

    fn fail<T>(&self, kind: DockLayoutErrorKind) -> Result<T, DockLayoutError> {
        Err(DockLayoutError { at: self.at, kind })
    }

    fn skip_space(&mut self) {
        let rest = self.rest();
        self.at += rest.len() - rest.trim_start().len();
    }

    /// Takes `c` after any space.
    fn eat(&mut self, c: char) -> bool {
        self.skip_space();
        let found = self.rest().starts_with(c);
        if found {
            self.at += c.len_utf8();
        }
        found
    }

    /// A run of letters, digits and number signs after any space.
    fn word(&mut self) -> &str {
        self.skip_space();
        let rest = self.rest();
        let len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')))
            .unwrap_or(rest.len());
        let start = self.at;
        self.at += len;
        &self.text[start..self.at]
    }

    fn node(&mut self, depth: usize) -> Result<DockLayout, DockLayoutError> {
        if depth > MAX_LAYOUT_DEPTH {
            return self.fail(DockLayoutErrorKind::TooDeep);
        }
        if !self.eat('(') {
            return self.fail(DockLayoutErrorKind::ExpectedOpen);
        }
        let start = self.at;
        let layout = match self.word() {
            "row" => self.split(DockAxis::Row, depth)?,
            "column" => self.split(DockAxis::Column, depth)?,
            "tabs" => self.tabs()?,
            _ => {
                self.at = start;
                return self.fail(DockLayoutErrorKind::ExpectedNode);
            }
        };
        if !self.eat(')') {
            return self.fail(DockLayoutErrorKind::ExpectedClose);
        }
        Ok(layout)
    }

    fn split(&mut self, axis: DockAxis, depth: usize) -> Result<DockLayout, DockLayoutError> {
        self.skip_space();
        let start = self.at;
        let ratio = match self.word().parse::<f32>() {
            Ok(ratio) if (0.0..=1.0).contains(&ratio) => ratio,
            _ => {
                self.at = start;
                return self.fail(DockLayoutErrorKind::BadRatio);
            }
        };
        let first = Box::new(self.node(depth + 1)?);
        let second = Box::new(self.node(depth + 1)?);
        Ok(DockLayout::Split {
            axis,
            ratio,
            first,
            second,
        })
    }

    fn tabs(&mut self) -> Result<DockLayout, DockLayoutError> {
        self.skip_space();
        let start = self.at;
        let Ok(shown) = self.word().parse::<usize>() else {
            self.at = start;
            return self.fail(DockLayoutErrorKind::BadShown);
        };
        let mut panels = Vec::new();
        while self.eat('"') {
            panels.push(self.key()?);
        }
        if shown >= panels.len() {
            self.at = start;
            return self.fail(DockLayoutErrorKind::ShownMissing);
        }
        Ok(DockLayout::Tabs { panels, shown })
    }

    /// A key's text after its opening quote, up to and past its closing one.
    fn key(&mut self) -> Result<String, DockLayoutError> {
        let mut key = String::new();
        let mut chars = self.rest().char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => {
                    self.at += i + 1;
                    return Ok(key);
                }
                '\\' => match chars.next() {
                    Some((_, escaped @ ('"' | '\\'))) => key.push(escaped),
                    _ => {
                        self.at += i;
                        return self.fail(DockLayoutErrorKind::BadEscape);
                    }
                },
                c => key.push(c),
            }
        }
        self.at = self.text.len();
        self.fail(DockLayoutErrorKind::UnclosedKey)
    }
}

impl DockArea {
    /// How the panels with keys are arranged now; `None` if none has a key. Panels without
    /// a key are left out, and a group with none of them with them.
    pub fn layout(&self) -> Option<DockLayout> {
        self.layout_of(self.root?, 0)
    }

    fn layout_of(&self, node: usize, depth: usize) -> Option<DockLayout> {
        let kind = &self.nodes.get(node)?.as_ref()?.kind;
        if depth == MAX_LAYOUT_DEPTH && matches!(kind, Kind::Split { .. }) {
            return self.merged(node);
        }
        match kind {
            Kind::Group { tabs, active, .. } => {
                let mut panels = Vec::new();
                let mut shown = 0;
                for (position, &tab) in tabs.iter().enumerate() {
                    if let Some(key) = &self.tabs[tab].key {
                        if position == *active {
                            shown = panels.len();
                        }
                        panels.push(key.clone());
                    }
                }
                (!panels.is_empty()).then_some(DockLayout::Tabs { panels, shown })
            }
            &Kind::Split {
                axis,
                ratio,
                first,
                second,
            } => match (
                self.layout_of(first, depth + 1),
                self.layout_of(second, depth + 1),
            ) {
                (Some(first), Some(second)) => Some(DockLayout::Split {
                    axis: match axis {
                        Axis::Row => DockAxis::Row,
                        Axis::Column => DockAxis::Column,
                    },
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (first, second) => first.or(second),
            },
        }
    }

    /// The keyed panels of `node`'s groups, in order, as one group showing the first group's
    /// shown panel: what is saved of splits nested too deeply to read back.
    fn merged(&self, node: usize) -> Option<DockLayout> {
        let mut panels = Vec::new();
        let mut shown = None;
        let mut stack = vec![node];
        while let Some(node) = stack.pop() {
            match self
                .nodes
                .get(node)
                .and_then(Option::as_ref)
                .map(|n| &n.kind)
            {
                Some(&Kind::Split { first, second, .. }) => stack.extend([second, first]),
                Some(Kind::Group { tabs, active, .. }) => {
                    for (position, &tab) in tabs.iter().enumerate() {
                        if let Some(key) = &self.tabs[tab].key {
                            if position == *active && shown.is_none() {
                                shown = Some(panels.len());
                            }
                            panels.push(key.clone());
                        }
                    }
                }
                None => {}
            }
        }
        let shown = shown.unwrap_or(0);
        (!panels.is_empty()).then_some(DockLayout::Tabs { panels, shown })
    }

    /// Arranges `dock`'s panels as `layout` says, matching them by key; see
    /// [`set_layout`](Self::set_layout).
    ///
    /// # Errors
    /// [`UiError::NoSuchWidget`] if `dock` is gone or is not a [`DockArea`].
    pub fn restore_layout(ui: &mut Ui, dock: WidgetId, layout: &DockLayout) -> Result<(), UiError> {
        ui.update::<DockArea, _>(dock, |area, cx| area.set_layout(cx, layout))
            .ok_or(UiError::NoSuchWidget(dock))
    }

    /// Arranges the panels as `layout` says (from an [`Ui::update`] on the dock): each key
    /// takes the next panel with that key. Keys without a panel are passed over (a group
    /// left empty goes, its space to its neighbor), and panels the layout does not name
    /// (without a key, or added since it was saved) join the first group, so none is lost.
    /// Ratios are kept from 0 to 1, and a group shows its first tab if its shown one is
    /// missing. Either part of a split keeps at least 1% of it, so no group vanishes. A drag
    /// in progress ends, and the keyboard stays with the group of the panel it was on.
    pub fn set_layout(&mut self, cx: &mut EventCx<'_>, layout: &DockLayout) {
        self.prune(cx);
        if self.drag.take().is_some() {
            cx.set_cursor(None);
        }
        self.hovered = None;
        // The tab the keyboard was on, to find its group again.
        let keyboard = self.focus_group.and_then(|group| {
            let (tabs, active) = self.group_tabs(group);
            tabs.get(active).copied()
        });
        self.focus_group = None;
        self.nodes.clear();
        self.root = None;
        let mut placed = vec![false; self.tabs.len()];
        self.root = self.build(layout, &mut placed, 0);
        if let Some(root) = self.root {
            if let Some(node) = &mut self.nodes[root] {
                node.parent = None;
            }
        }
        // The rest join the first group, which keeps showing what the layout shows.
        let mut first = self.first_group();
        let mut shown = first.and_then(|group| {
            let (tabs, active) = self.group_tabs(group);
            tabs.get(active).copied()
        });
        for index in (0..self.tabs.len()).filter(|&i| !placed[i]) {
            self.join(index, first, None);
            first = Some(self.tabs[index].group);
            // Nothing was placed: the first of the rest is shown.
            shown = shown.or(Some(index));
        }
        if let Some(tab) = shown {
            self.activate(tab);
        }
        self.focus_group = keyboard.map(|tab| self.tabs[tab].group);
        self.arrange();
        self.restyle(cx);
        // Titles may sit in groups of other widths: `layout` brings each shown tab into view.
        cx.request_layout();
    }

    /// The group in the top-left corner.
    fn first_group(&self) -> Option<usize> {
        let mut node = self.root?;
        loop {
            match self.nodes.get(node)?.as_ref()?.kind {
                Kind::Split { first, .. } => node = first,
                Kind::Group { .. } => return Some(node),
            }
        }
    }

    /// Makes the nodes `layout` describes, with the tabs it names that are not `placed` yet;
    /// the node made, or `None` if no tab was left for it.
    fn build(&mut self, layout: &DockLayout, placed: &mut [bool], depth: usize) -> Option<usize> {
        if depth > MAX_LAYOUT_DEPTH {
            return None;
        }
        match layout {
            DockLayout::Tabs { panels, shown } => {
                let mut tabs = Vec::new();
                let mut active = 0;
                for (position, key) in panels.iter().enumerate() {
                    let free = |i: &usize| !placed[*i] && self.tabs[*i].key.as_ref() == Some(key);
                    let Some(index) = (0..self.tabs.len()).find(free) else {
                        continue;
                    };
                    placed[index] = true;
                    if position == *shown {
                        active = tabs.len();
                    }
                    tabs.push(index);
                }
                if tabs.is_empty() {
                    return None;
                }
                let serial = self.next_serial();
                let group = self.push(Kind::Group {
                    tabs: tabs.clone(),
                    active,
                    scroll: 0.0,
                    serial,
                });
                for tab in tabs {
                    self.tabs[tab].group = group;
                }
                Some(group)
            }
            DockLayout::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let first = self.build(first, placed, depth + 1);
                let second = self.build(second, placed, depth + 1);
                let (Some(first), Some(second)) = (first, second) else {
                    return first.or(second);
                };
                let axis = match axis {
                    DockAxis::Row => Axis::Row,
                    DockAxis::Column => Axis::Column,
                };
                let ratio = if ratio.is_nan() {
                    0.5
                } else {
                    ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO)
                };
                let split = self.push(Kind::Split {
                    axis,
                    ratio,
                    first,
                    second,
                });
                for child in [first, second] {
                    if let Some(Node { parent, .. }) = &mut self.nodes[child] {
                        *parent = Some(split);
                    }
                }
                Some(split)
            }
        }
    }
}
