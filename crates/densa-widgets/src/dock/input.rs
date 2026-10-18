//! What the pointer does to the dock: clicking and dragging tabs, dragging dividers.

use densa_core::Point;
use densa_ui::{ActionKind, Cursor, EventCx, Handled, ScrollDelta};

use super::geometry::divider_rect;
use super::{Axis, DockArea, Drag, Found};

/// How far a pressed tab moves before it is being dragged, in logical pixels.
const DRAG_START: f32 = 6.0;
/// How far a wheel notch scrolls a tab bar, in logical pixels.
const WHEEL_LINE: f32 = 40.0;
/// The smallest a divider leaves either side of it, in logical pixels.
const MIN_PANE: f32 = 48.0;

/// The resize cursor for a divider between parts laid out along `axis`.
pub(super) fn resize_cursor(axis: Axis) -> Cursor {
    match axis {
        Axis::Row => Cursor::ResizeColumn,
        Axis::Column => Cursor::ResizeRow,
    }
}

impl DockArea {
    pub(super) fn press(&mut self, cx: &mut EventCx<'_>, at: Point) -> Handled {
        let (rect, scale) = (cx.rect(), cx.scale());
        match self.find(at, rect, scale) {
            Found::Tab(index, _) => {
                if !self.is_active(index) {
                    self.activate(index);
                    self.restyle(cx);
                    cx.emit(ActionKind::Selected(index));
                }
                // A tab partly scrolled out of its bar comes wholly into view.
                self.reveal(self.tabs[index].group, rect, scale);
                self.drag = Some(Drag::Pressed {
                    tab: index,
                    from: at,
                });
                Handled::Yes
            }
            Found::Divider(i) => {
                let divider = self.dividers[i];
                let strip = divider_rect(&divider, rect, scale);
                // Where on the divider it was grabbed, so it does not jump to the pointer.
                let grab = match divider.axis {
                    Axis::Row => at.x - (strip.left() + strip.right()) / 2.0,
                    Axis::Column => at.y - (strip.top() + strip.bottom()) / 2.0,
                };
                self.drag = Some(Drag::Divider {
                    split: divider.split,
                    grab,
                    moved: false,
                });
                cx.set_cursor(Some(resize_cursor(divider.axis)));
                cx.request_paint();
                Handled::Yes
            }
            Found::ScrollTabs(group, forward) => {
                // Half the visible tabs at a time.
                let strip = self.strip_of(group, rect, scale);
                let by = strip.map_or(0.0, |strip| strip.tabs.width() / 2.0);
                if self.scroll_tabs(group, if forward { by } else { -by }, rect, scale) {
                    cx.request_paint();
                }
                Handled::Yes
            }
            Found::Group(..) | Found::Nothing => Handled::No,
        }
    }

    pub(super) fn drag_to(&mut self, cx: &mut EventCx<'_>, at: Point) -> Handled {
        let (rect, scale) = (cx.rect(), cx.scale());
        match self.drag {
            Some(Drag::Pressed { tab, from }) => {
                let far = cx.px(DRAG_START);
                if (at.x - from.x).abs() > far || (at.y - from.y).abs() > far {
                    let target = self.target_at(at, rect, scale);
                    self.drag = Some(Drag::Tab { tab, target });
                    cx.set_cursor(Some(Cursor::Grabbing));
                    cx.request_paint();
                }
                Handled::Yes
            }
            Some(Drag::Tab { tab, target }) => {
                let now = self.target_at(at, rect, scale);
                if now != target {
                    self.drag = Some(Drag::Tab { tab, target: now });
                    cx.request_paint();
                }
                Handled::Yes
            }
            Some(Drag::Divider { split, grab, moved }) => {
                let changed = self.resize(cx, split, at, grab);
                self.drag = Some(Drag::Divider {
                    split,
                    grab,
                    moved: moved || changed,
                });
                Handled::Yes
            }
            None => {
                let found = self.find(at, rect, scale);
                let cursor = match found {
                    Found::Divider(i) => Some(resize_cursor(self.dividers[i].axis)),
                    _ => None,
                };
                cx.set_cursor(cursor);
                let tab = match found {
                    Found::Tab(index, _) => Some(index),
                    _ => None,
                };
                self.hover(cx, tab);
                Handled::No
            }
        }
    }

    /// Moves split `split`'s divider so its middle is `grab` before `at`, leaving each side
    /// at least [`MIN_PANE`]; whether it moved.
    fn resize(&mut self, cx: &mut EventCx<'_>, split: usize, at: Point, grab: f32) -> bool {
        let Some(divider) = self.dividers.iter().find(|d| d.split == split).copied() else {
            return false;
        };
        let (rect, area) = (cx.rect(), divider.area);
        let (start, extent, to) = match divider.axis {
            Axis::Row => (
                rect.left() + area.x0 * rect.width(),
                (area.x1 - area.x0) * rect.width(),
                at.x - grab,
            ),
            Axis::Column => (
                rect.top() + area.y0 * rect.height(),
                (area.y1 - area.y0) * rect.height(),
                at.y - grab,
            ),
        };
        let extent = extent.max(1.0);
        let least = (cx.px(MIN_PANE) / extent).min(0.5);
        let share = ((to - start) / extent).clamp(least, 1.0 - least);
        if !share.is_finite() || !self.set_ratio(split, share) {
            return false;
        }
        self.arrange();
        self.restyle(cx);
        true
    }

    /// Scrolls the tab bar under `at` by a wheel's `delta`; handled if it scrolled.
    pub(super) fn wheel(&mut self, cx: &mut EventCx<'_>, at: Point, delta: ScrollDelta) -> Handled {
        let (rect, scale) = (cx.rect(), cx.scale());
        let group = match self.find(at, rect, scale) {
            Found::Tab(index, _) => self.tabs[index].group,
            Found::ScrollTabs(group, _) => group,
            Found::Group(..) | Found::Divider(_) | Found::Nothing => return Handled::No,
        };
        // Down or right scrolls forward: wheels without a sideways axis can scroll too.
        let by = match delta {
            ScrollDelta::Lines(dx, dy) => (dx + dy) * cx.px(WHEEL_LINE),
            ScrollDelta::Pixels(dx, dy) => dx + dy,
            _ => 0.0,
        };
        if !self.scroll_tabs(group, by, rect, scale) {
            return Handled::No;
        }
        cx.request_paint();
        Handled::Yes
    }

    /// Marks the tab `index` as the one under the pointer.
    pub(super) fn hover(&mut self, cx: &mut EventCx<'_>, index: Option<usize>) {
        if self.hovered != index {
            self.hovered = index;
            cx.request_paint();
        }
    }

    /// The pointer was released (`dropped`) or the press was cancelled.
    pub(super) fn finish(&mut self, cx: &mut EventCx<'_>, dropped: bool) -> Handled {
        let Some(drag) = self.drag.take() else {
            return Handled::No;
        };
        cx.set_cursor(None);
        cx.request_paint();
        match drag {
            Drag::Tab {
                tab,
                target: Some(target),
            } if dropped => {
                if self.move_tab(tab, target) {
                    self.hovered = None;
                    self.arrange();
                    self.reveal(self.tabs[tab].group, cx.rect(), cx.scale());
                    self.restyle(cx);
                    cx.emit(ActionKind::Changed);
                }
            }
            Drag::Divider { moved: true, .. } => cx.emit(ActionKind::Changed),
            _ => {}
        }
        Handled::Yes
    }

    /// Forgets tabs whose widgets were removed from the `Ui` directly rather than through
    /// [`DockArea::remove_panel`].
    pub(super) fn prune(&mut self, cx: &mut EventCx<'_>) {
        while let Some(dead) = self
            .tabs
            .iter()
            .position(|t| cx.style_of(t.widget).is_none())
        {
            self.drop_tab(cx, dead);
        }
    }

    /// Takes tab `index` out of the dock, keeping a drag of another tab or a divider going.
    pub(super) fn drop_tab(&mut self, cx: &mut EventCx<'_>, index: usize) {
        let renumber = |t: usize| if t > index { t - 1 } else { t };
        self.drag = match self.drag {
            Some(Drag::Pressed { tab, .. } | Drag::Tab { tab, .. }) if tab == index => {
                cx.set_cursor(None);
                None
            }
            Some(Drag::Pressed { tab, from }) => Some(Drag::Pressed {
                tab: renumber(tab),
                from,
            }),
            // Groups may go: the next move finds where it would land again.
            Some(Drag::Tab { tab, .. }) => Some(Drag::Tab {
                tab: renumber(tab),
                target: None,
            }),
            divider => divider,
        };
        self.hovered = None;
        self.forget(index);
        self.arrange();
        if let Some(Drag::Divider { split, .. }) = self.drag {
            // Removing the tab took the split away.
            if !self.is_split(split) {
                self.drag = None;
                cx.set_cursor(None);
            }
        }
        self.restyle(cx);
    }
}
