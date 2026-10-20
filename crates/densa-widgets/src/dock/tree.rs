//! The arrangement: splits and tab groups in an arena, and the changes the user makes to it.

use super::{Axis, Divider, DockArea, DockSide, Frac, Kind, Node, Target};

impl DockArea {
    /// The tab indices of `group` in order, and the position of the shown one.
    pub(super) fn group_tabs(&self, group: usize) -> (&[usize], usize) {
        match self.nodes.get(group) {
            Some(Some(Node {
                kind: Kind::Group { tabs, active, .. },
                ..
            })) => (tabs, *active),
            _ => (&[], 0),
        }
    }

    /// A serial for a new tab or group, unused by any living one (they wrap only after
    /// millions of additions, far more than tabs alive at once).
    pub(super) fn next_serial(&mut self) -> u32 {
        self.last_serial = (self.last_serial + 1) % super::access::SERIALS;
        self.last_serial
    }

    /// `group`'s serial (0 if it is not a group).
    pub(super) fn group_serial(&self, group: usize) -> u32 {
        match self.nodes.get(group) {
            Some(Some(Node {
                kind: Kind::Group { serial, .. },
                ..
            })) => *serial,
            _ => 0,
        }
    }

    /// How far `group`'s tab bar is scrolled, in logical pixels.
    pub(super) fn group_scroll(&self, group: usize) -> f32 {
        match self.nodes.get(group) {
            Some(Some(Node {
                kind: Kind::Group { scroll, .. },
                ..
            })) => *scroll,
            _ => 0.0,
        }
    }

    pub(super) fn set_group_scroll(&mut self, group: usize, to: f32) {
        if let Some(Some(Node {
            kind: Kind::Group { scroll, .. },
            ..
        })) = self.nodes.get_mut(group)
        {
            *scroll = to;
        }
    }

    /// Where tab `index` is among its group's tabs (0 if it is not there).
    pub(super) fn position_in(&self, group: usize, index: usize) -> usize {
        let (tabs, _) = self.group_tabs(group);
        tabs.iter().position(|&t| t == index).unwrap_or(0)
    }

    pub(super) fn is_active(&self, index: usize) -> bool {
        let (tabs, active) = self.group_tabs(self.tabs[index].group);
        tabs.get(active) == Some(&index)
    }

    /// Puts tab `index` into `group` at `position` (the end if `None` or past it) as its
    /// shown tab, or into a new root group if there is none.
    pub(super) fn join(&mut self, index: usize, group: Option<usize>, position: Option<usize>) {
        let group = group.unwrap_or_else(|| {
            let serial = self.next_serial();
            let group = self.push(Kind::Group {
                tabs: Vec::new(),
                active: 0,
                scroll: 0.0,
                serial,
            });
            self.root = Some(group);
            group
        });
        if let Some(Node {
            kind: Kind::Group { tabs, active, .. },
            ..
        }) = &mut self.nodes[group]
        {
            let at = position.unwrap_or(tabs.len()).min(tabs.len());
            tabs.insert(at, index);
            *active = at;
        }
        self.tabs[index].group = group;
    }

    /// Puts tab `index` into a new group on `side` of `group`.
    pub(super) fn split(&mut self, index: usize, group: usize, side: DockSide) {
        let parent = self.nodes[group].as_ref().and_then(|n| n.parent);
        let serial = self.next_serial();
        let fresh = self.push(Kind::Group {
            tabs: vec![index],
            active: 0,
            scroll: 0.0,
            serial,
        });
        self.tabs[index].group = fresh;
        let axis = match side {
            DockSide::Left | DockSide::Right => Axis::Row,
            DockSide::Top | DockSide::Bottom => Axis::Column,
        };
        let (first, second) = match side {
            DockSide::Left | DockSide::Top => (fresh, group),
            DockSide::Right | DockSide::Bottom => (group, fresh),
        };
        let split = self.push(Kind::Split {
            axis,
            ratio: 0.5,
            first,
            second,
        });
        self.replace_child(parent, group, split);
        for child in [first, second] {
            if let Some(node) = &mut self.nodes[child] {
                node.parent = Some(split);
            }
        }
    }

    /// Makes `new` take `old`'s place under `parent` (as the root if there is none).
    fn replace_child(&mut self, parent: Option<usize>, old: usize, new: usize) {
        if let Some(node) = &mut self.nodes[new] {
            node.parent = parent;
        }
        let Some(parent) = parent else {
            self.root = Some(new);
            return;
        };
        if let Some(Node {
            kind: Kind::Split { first, second, .. },
            ..
        }) = &mut self.nodes[parent]
        {
            if *first == old {
                *first = new;
            } else if *second == old {
                *second = new;
            }
        }
    }

    /// Stores a node in a free slot, or a new one.
    pub(super) fn push(&mut self, kind: Kind) -> usize {
        let node = Some(Node { kind, parent: None });
        match self.nodes.iter().position(Option::is_none) {
            Some(slot) => {
                self.nodes[slot] = node;
                slot
            }
            None => {
                self.nodes.push(node);
                self.nodes.len() - 1
            }
        }
    }

    /// Takes tab `index` out of its group, removing the group if that empties it.
    pub(super) fn detach(&mut self, index: usize) {
        let group = self.tabs[index].group;
        let empty = match &mut self.nodes[group] {
            Some(Node {
                kind: Kind::Group { tabs, active, .. },
                ..
            }) => {
                if let Some(at) = tabs.iter().position(|&t| t == index) {
                    tabs.remove(at);
                    // Keep showing the same tab, or the next one if this was it.
                    if at < *active {
                        *active -= 1;
                    }
                }
                *active = (*active).min(tabs.len().saturating_sub(1));
                tabs.is_empty()
            }
            _ => false,
        };
        if empty {
            self.remove_group(group);
        }
    }

    /// Removes an empty group: its sibling takes its parent split's place.
    fn remove_group(&mut self, group: usize) {
        let Some(node) = self.nodes[group].take() else {
            return;
        };
        // Its slot may be reused by a new group, which must not inherit the keyboard.
        if self.focus_group == Some(group) {
            self.focus_group = None;
        }
        let Some(split) = node.parent else {
            self.root = None;
            return;
        };
        let Some(Node {
            kind: Kind::Split { first, second, .. },
            parent,
        }) = self.nodes[split].take()
        else {
            return;
        };
        let sibling = if first == group { second } else { first };
        self.replace_child(parent, split, sibling);
    }

    pub(super) fn activate(&mut self, index: usize) {
        let group = self.tabs[index].group;
        if let Some(Node {
            kind: Kind::Group { tabs, active, .. },
            ..
        }) = &mut self.nodes[group]
        {
            if let Some(at) = tabs.iter().position(|&t| t == index) {
                *active = at;
            }
        }
    }

    /// Whether moving tab `index` to `target` would change anything.
    pub(super) fn changes(&self, index: usize, target: Target) -> bool {
        let own = self.tabs[index].group;
        let (tabs, _) = self.group_tabs(own);
        match target {
            // Into its own group's middle: it is already there.
            Target::Into(group, None) if group == own => false,
            Target::Into(group, Some(to)) if group == own => {
                // Back into the slot it left, before or after itself, changes nothing.
                let from = self.position_in(own, index);
                let to = to.min(tabs.len());
                to != from && to != from + 1
            }
            Target::Beside(group, _) if group == own => tabs.len() > 1,
            Target::Into(..) | Target::Beside(..) => true,
        }
    }

    /// Moves tab `index` to `target`; `false` if that changes nothing.
    pub(super) fn move_tab(&mut self, index: usize, target: Target) -> bool {
        if !self.changes(index, target) {
            return false;
        }
        let own = self.tabs[index].group;
        match target {
            Target::Into(group, position) => {
                // Within its own group, the slots after the tab move down one.
                let from = self.position_in(own, index);
                let position = position.map(|p| if group == own && p > from { p - 1 } else { p });
                self.detach(index);
                // Its own group survives: it had other tabs, or the move changed nothing.
                self.join(index, Some(group), position);
            }
            Target::Beside(group, side) => {
                self.detach(index);
                self.split(index, group, side);
            }
        }
        true
    }

    /// Removes tab `index` from the arrangement, renumbering the tabs after it.
    pub(super) fn forget(&mut self, index: usize) {
        self.detach(index);
        self.tabs.remove(index);
        for node in self.nodes.iter_mut().flatten() {
            if let Kind::Group { tabs, .. } = &mut node.kind {
                for t in tabs.iter_mut().filter(|t| **t > index) {
                    *t -= 1;
                }
            }
        }
    }

    /// Whether node `split` is (still) a split.
    pub(super) fn is_split(&self, split: usize) -> bool {
        matches!(
            self.nodes.get(split),
            Some(Some(Node {
                kind: Kind::Split { .. },
                ..
            }))
        )
    }

    /// Sets split `split`'s ratio; `false` if it was already that.
    pub(super) fn set_ratio(&mut self, split: usize, to: f32) -> bool {
        match &mut self.nodes[split] {
            Some(Node {
                kind: Kind::Split { ratio, .. },
                ..
            }) if *ratio != to => {
                *ratio = to;
                true
            }
            _ => false,
        }
    }

    /// Recomputes where each group and divider is, as fractions of the dock.
    pub(super) fn arrange(&mut self) {
        self.frames.clear();
        self.dividers.clear();
        self.stack.clear();
        self.stack.extend(self.root.map(|root| (root, Frac::WHOLE)));
        while let Some((node, area)) = self.stack.pop() {
            match self.nodes[node].as_ref().map(|n| &n.kind) {
                Some(Kind::Group { .. }) => self.frames.push((node, area)),
                Some(&Kind::Split {
                    axis,
                    ratio,
                    first,
                    second,
                }) => {
                    self.dividers.push(Divider {
                        split: node,
                        axis,
                        ratio,
                        area,
                    });
                    let (a, b) = match axis {
                        Axis::Row => {
                            let x = area.x0 + ratio * (area.x1 - area.x0);
                            (Frac { x1: x, ..area }, Frac { x0: x, ..area })
                        }
                        Axis::Column => {
                            let y = area.y0 + ratio * (area.y1 - area.y0);
                            (Frac { y1: y, ..area }, Frac { y0: y, ..area })
                        }
                    };
                    self.stack.push((second, b));
                    self.stack.push((first, a));
                }
                None => {}
            }
        }
    }
}
