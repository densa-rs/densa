//! Where the dock's parts are: groups, tab bars, tabs, dividers and drop previews.

use densa_core::{Point, Rect};
use densa_ui::{Edges, Length, Overflow, Style};

use super::{Divider, DockArea, DockSide, Found, Frac, Target};

/// Tab bar height, in logical pixels.
const TAB_BAR: f32 = 28.0;
/// Space either side of a tab's title, in logical pixels.
const TAB_PADDING: f32 = 12.0;
/// The narrowest a tab shrinks to when the bar is full, in logical pixels.
const TAB_MIN: f32 = 48.0;
/// The width of each of the buttons that scroll tabs that do not fit, in logical pixels.
const SCROLL_BUTTON: f32 = 20.0;
/// The gap between groups, which holds the divider, in logical pixels.
pub(super) const GAP: f32 = 6.0;
/// The share of a group's width or height, from an edge, that drops a tab beside the group
/// rather than into it.
const EDGE: f32 = 0.25;
/// Fractions this close to the dock's edge are on it.
const EPSILON: f32 = 1e-4;

/// A one-pixel line at `scale`, at least one physical pixel wide.
pub(super) fn hairline(scale: f32) -> f32 {
    scale.round().max(1.0)
}

/// The tab bar's height in whole physical pixels, so the bar and the panel below meet on a
/// pixel edge at any scale.
fn bar_height(scale: f32) -> f32 {
    (TAB_BAR * scale).round()
}

/// Half the gap, unless the edge is the dock's own.
fn inner(on_edge: bool, half: f32) -> f32 {
    if on_edge {
        0.0
    } else {
        half
    }
}

/// Where group `frac` is within `rect`, inside the gaps, on whole pixels.
pub(super) fn group_rect(frac: Frac, rect: Rect, scale: f32) -> Rect {
    let half = GAP * scale / 2.0;
    let (w, h) = (rect.width(), rect.height());
    Rect::from_edges(
        (rect.left() + frac.x0 * w + inner(frac.x0 < EPSILON, half)).round(),
        (rect.top() + frac.y0 * h + inner(frac.y0 < EPSILON, half)).round(),
        (rect.left() + frac.x1 * w - inner(frac.x1 > 1.0 - EPSILON, half)).round(),
        (rect.top() + frac.y1 * h - inner(frac.y1 > 1.0 - EPSILON, half)).round(),
    )
}

/// Where tabs go along the top of a group's area, inside its outline.
pub(super) fn bar(area: Rect, scale: f32) -> Rect {
    let line = hairline(scale);
    let bottom = (area.top() + bar_height(scale)).min(area.bottom());
    Rect::from_edges(area.left() + line, area.top(), area.right() - line, bottom)
}

/// The style placing a group's shown panel: at the group's fractions of the dock, below its
/// tab bar, inside the gaps. Must agree with [`group_rect`] and [`bar`].
pub(super) fn content_style(frac: Frac, scale: f32) -> Style {
    let half = GAP / 2.0;
    let inset = Edges {
        left: Length::Fraction(frac.x0),
        top: Length::Fraction(frac.y0),
        right: Length::Fraction(1.0 - frac.x1),
        bottom: Length::Fraction(1.0 - frac.y1),
    };
    let margin = Edges {
        left: Length::Px(inner(frac.x0 < EPSILON, half)),
        top: Length::Px(bar_height(scale) / scale + inner(frac.y0 < EPSILON, half)),
        right: Length::Px(inner(frac.x1 > 1.0 - EPSILON, half)),
        bottom: Length::Px(inner(frac.y1 > 1.0 - EPSILON, half)),
    };
    Style::column()
        .absolute(inset)
        .margin(margin)
        .min_size(Length::Px(0.0), Length::Px(0.0))
        .overflow(Overflow::Clip)
}

/// The divider's strip, the gap between the split's parts.
pub(super) fn divider_rect(divider: &Divider, rect: Rect, scale: f32) -> Rect {
    let (area, ratio) = (divider.area, divider.ratio);
    let half = GAP * scale / 2.0;
    let (w, h) = (rect.width(), rect.height());
    if matches!(divider.axis, super::Axis::Row) {
        let x = rect.left() + (area.x0 + ratio * (area.x1 - area.x0)) * w;
        Rect::from_edges(
            x - half,
            rect.top() + area.y0 * h,
            x + half,
            rect.top() + area.y1 * h,
        )
    } else {
        let y = rect.top() + (area.y0 + ratio * (area.y1 - area.y0)) * h;
        Rect::from_edges(
            rect.left() + area.x0 * w,
            y - half,
            rect.left() + area.x1 * w,
            y + half,
        )
    }
}

/// Where the new group would be if a tab were dropped on `side` of group `frac`: the half
/// a split would give it, placed the way every group is.
pub(super) fn beside(frac: Frac, side: DockSide, rect: Rect, scale: f32) -> Rect {
    let mx = (frac.x0 + frac.x1) / 2.0;
    let my = (frac.y0 + frac.y1) / 2.0;
    let half = match side {
        DockSide::Left => Frac { x1: mx, ..frac },
        DockSide::Right => Frac { x0: mx, ..frac },
        DockSide::Top => Frac { y1: my, ..frac },
        DockSide::Bottom => Frac { y0: my, ..frac },
    };
    group_rect(half, rect, scale)
}

/// How a group's tabs sit along its bar.
#[derive(Debug, Clone, Copy)]
pub(super) struct Strip {
    /// Where the tabs go: the bar, less the scroll buttons when they do not fit.
    pub(super) tabs: Rect,
    /// The buttons scrolling the tabs back and forward, when they do not fit.
    pub(super) buttons: Option<(Rect, Rect)>,
    /// How far the tabs are scrolled, in physical pixels.
    pub(super) offset: f32,
    /// The furthest they scroll.
    pub(super) max_offset: f32,
    /// Each tab's natural width is multiplied by this (1 when they fit).
    shrink: f32,
    pad: f32,
    min: f32,
}

impl Strip {
    /// Tab `index`'s width in this strip.
    fn width(&self, dock: &DockArea, index: usize) -> f32 {
        let natural = dock.natural_width(index, self.pad);
        if self.shrink < 1.0 {
            // Down, so shrunk tabs never add up to more than the bar.
            (natural * self.shrink).floor().max(self.min)
        } else {
            natural
        }
    }
}

impl DockArea {
    /// How `group`'s tabs sit along `bar`. Tabs that do not fit shrink (down to [`TAB_MIN`]);
    /// if they still do not, the bar gets scroll buttons and scrolls by the group's offset.
    pub(super) fn strip(&self, group: usize, bar: Rect, scale: f32) -> Strip {
        let (members, _) = self.group_tabs(group);
        let pad = TAB_PADDING * scale;
        let min = (TAB_MIN * scale).round();
        let natural: f32 = members.iter().map(|&i| self.natural_width(i, pad)).sum();
        let room = bar.width().max(0.0);
        let shrink = if natural > room { room / natural } else { 1.0 };
        let mut strip = Strip {
            tabs: bar,
            buttons: None,
            offset: 0.0,
            max_offset: 0.0,
            shrink,
            pad,
            min,
        };
        let total: f32 = members.iter().map(|&i| strip.width(self, i)).sum();
        if total > room {
            let button = (SCROLL_BUTTON * scale).round();
            let tabs_right = (bar.right() - 2.0 * button).max(bar.left());
            strip.tabs = Rect::from_edges(bar.left(), bar.top(), tabs_right, bar.bottom());
            let back = Rect::from_edges(tabs_right, bar.top(), tabs_right + button, bar.bottom());
            let forward = Rect::from_edges(back.right(), bar.top(), bar.right(), bar.bottom());
            strip.buttons = Some((back, forward));
            strip.max_offset = (total - strip.tabs.width()).max(0.0);
            strip.offset = (self.group_scroll(group) * scale).clamp(0.0, strip.max_offset);
        }
        strip
    }

    /// Tab `index`'s width with no shrinking: its title and padding.
    fn natural_width(&self, index: usize, pad: f32) -> f32 {
        (self.tabs[index].title.size().width + 2.0 * pad).round()
    }

    /// Each tab of `group` along `bar`: the tab's index and rectangle, cut to the part of the
    /// bar tabs show in (empty when scrolled out of it).
    pub(super) fn tab_rects(
        &self,
        group: usize,
        bar: Rect,
        scale: f32,
    ) -> impl Iterator<Item = (usize, Rect)> + '_ {
        let (members, _) = self.group_tabs(group);
        let strip = self.strip(group, bar, scale);
        let area = strip.tabs;
        let mut x = area.left() - strip.offset;
        members.iter().map(move |&i| {
            let w = strip.width(self, i);
            let left = x.clamp(area.left(), area.right());
            x += w;
            let right = x.clamp(area.left(), area.right());
            (i, Rect::from_edges(left, area.top(), right, area.bottom()))
        })
    }

    /// How `group`'s tabs sit when the dock is at `rect`; `None` if it is not a group.
    pub(super) fn strip_of(&self, group: usize, rect: Rect, scale: f32) -> Option<Strip> {
        let &(_, frac) = self.frames.iter().find(|(g, _)| *g == group)?;
        Some(self.strip(group, bar(group_rect(frac, rect, scale), scale), scale))
    }

    /// Scrolls `group`'s tab bar so its shown tab is wholly in view, when the dock is at
    /// `rect`.
    pub(super) fn reveal(&mut self, group: usize, rect: Rect, scale: f32) {
        let Some(strip) = self.strip_of(group, rect, scale) else {
            return;
        };
        if strip.buttons.is_none() {
            return;
        }
        let (members, active) = self.group_tabs(group);
        let start: f32 = members[..active]
            .iter()
            .map(|&i| strip.width(self, i))
            .sum();
        let end = start + members.get(active).map_or(0.0, |&i| strip.width(self, i));
        let room = strip.tabs.width();
        let offset = if start < strip.offset {
            start
        } else if end > strip.offset + room {
            end - room
        } else {
            strip.offset
        };
        self.set_group_scroll(group, offset.clamp(0.0, strip.max_offset) / scale);
    }

    /// Scrolls `group`'s tab bar by `by` physical pixels, within its tabs; whether it moved.
    pub(super) fn scroll_tabs(&mut self, group: usize, by: f32, rect: Rect, scale: f32) -> bool {
        let Some(strip) = self.strip_of(group, rect, scale) else {
            return false;
        };
        let offset = (strip.offset + by).clamp(0.0, strip.max_offset);
        if offset == strip.offset {
            return false;
        }
        self.set_group_scroll(group, offset / scale);
        true
    }

    /// What is at `at`: a tab, a divider, or a group's area.
    pub(super) fn find(&self, at: Point, rect: Rect, scale: f32) -> Found {
        if let Some(i) = self
            .dividers
            .iter()
            .position(|d| divider_rect(d, rect, scale).contains(at))
        {
            return Found::Divider(i);
        }
        for &(group, frac) in &self.frames {
            let area = group_rect(frac, rect, scale);
            if !area.contains(at) {
                continue;
            }
            let bar = bar(area, scale);
            if let Some((back, forward)) = self.strip(group, bar, scale).buttons {
                if back.contains(at) || forward.contains(at) {
                    return Found::ScrollTabs(group, forward.contains(at));
                }
            }
            let tab = self
                .tab_rects(group, bar, scale)
                .find(|(_, tab)| tab.contains(at));
            return match tab {
                Some((index, tab)) => Found::Tab(index, tab),
                None => Found::Group(group, area),
            };
        }
        Found::Nothing
    }

    /// Where a tab dropped at `at` would land.
    pub(super) fn target_at(&self, at: Point, rect: Rect, scale: f32) -> Option<Target> {
        let (group, area) = match self.find(at, rect, scale) {
            Found::Group(group, area) => (group, area),
            Found::ScrollTabs(group, _) => {
                let (tabs, _) = self.group_tabs(group);
                return Some(Target::Into(group, Some(tabs.len())));
            }
            Found::Tab(index, tab) => {
                // Before the tab under the pointer, or after it past its middle.
                let group = self.tabs[index].group;
                let position = self.position_in(group, index);
                let after = at.x >= (tab.left() + tab.right()) / 2.0;
                let index = position + usize::from(after);
                return Some(Target::Into(group, Some(index)));
            }
            Found::Divider(_) | Found::Nothing => return None,
        };
        // On the tab bar's empty end: after the group's last tab.
        if bar(area, scale).contains(at) {
            let (tabs, _) = self.group_tabs(group);
            return Some(Target::Into(group, Some(tabs.len())));
        }
        let fx = (at.x - area.left()) / area.width().max(1.0);
        let fy = (at.y - area.top()) / area.height().max(1.0);
        let nearest = [
            (fx, DockSide::Left),
            (1.0 - fx, DockSide::Right),
            (fy, DockSide::Top),
            (1.0 - fy, DockSide::Bottom),
        ]
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .filter(|(distance, _)| *distance < EDGE);
        Some(match nearest {
            Some((_, side)) => Target::Beside(group, side),
            None => Target::Into(group, None),
        })
    }

    /// Where tab `index` of the dock at `rect` is drawn, if it is shown in its bar.
    pub(super) fn tab_rect_of(&self, index: usize, rect: Rect, scale: f32) -> Option<Rect> {
        let group = self.tabs.get(index)?.group;
        let &(_, frac) = self.frames.iter().find(|(g, _)| *g == group)?;
        let area = group_rect(frac, rect, scale);
        self.tab_rects(group, bar(area, scale), scale)
            .find(|(i, _)| *i == index)
            .map(|(_, tab)| tab)
            .filter(|tab| !tab.is_empty())
    }
}
