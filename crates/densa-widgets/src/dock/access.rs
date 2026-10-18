//! The dock to assistive technology: a tab list per group, a tab per panel.

use densa_ui::{
    AccessAction, AccessNode, AccessRequest, ActionKind, EventCx, Handled, PaintCx, Role,
};

use super::DockArea;

/// Serials of tabs and groups run below this; tab lists' part numbers start here (a group's
/// serial is added), tabs' are their serials. Both fit under `densa_ui::MAX_PART`.
pub(super) const SERIALS: u32 = 1 << 23;

impl DockArea {
    /// A tab list per group, holding a tab per panel: named by its title, selected when shown,
    /// chosen by a click.
    pub(super) fn describe(&self, cx: &PaintCx<'_>, node: &mut AccessNode) {
        let (rect, scale) = (cx.rect(), cx.scale());
        for &(group, _) in &self.frames {
            let mut list = AccessNode::new(Role::TabList);
            if let Some(strip) = self.strip_of(group, rect, scale) {
                list.set_bounds(strip.tabs);
            }
            // Tabs scrolled out of the bar are off screen.
            list.set_clips_children(true);
            let (tabs, _) = self.group_tabs(group);
            for &index in tabs {
                let tab = &self.tabs[index];
                let mut item = AccessNode::new(Role::Tab);
                item.set_label(tab.title.as_str());
                item.set_selected(self.is_active(index));
                if let Some(bounds) = self.tab_rect_of(index, rect, scale) {
                    item.set_bounds(bounds);
                }
                item.add_action(AccessAction::Click);
                item.add_action(AccessAction::ScrollIntoView);
                list.push_part(tab.serial, item);
            }
            node.push_part(SERIALS + self.group_serial(group), list);
        }
        // While the dock has focus, it is on the focused group's shown tab.
        if let Some(group) = self.focused_group() {
            let (tabs, active) = self.group_tabs(group);
            if let Some(&index) = tabs.get(active) {
                node.set_focused_part(self.tabs[index].serial);
            }
        }
    }

    /// Shows the tab a click is for, or scrolls its bar to it.
    pub(super) fn act(&mut self, cx: &mut EventCx<'_>, request: &AccessRequest<'_>) -> Handled {
        let Some(index) = request.part.and_then(|part| self.tab_with_serial(part)) else {
            return Handled::No;
        };
        match request.action {
            AccessAction::Click => {
                if !self.is_active(index) {
                    self.activate(index);
                    self.restyle(cx);
                    cx.emit(ActionKind::Selected(index));
                }
                self.reveal(self.tabs[index].group, cx.rect(), cx.scale());
                Handled::Yes
            }
            AccessAction::ScrollIntoView => {
                let group = self.tabs[index].group;
                let position = self.position_in(group, index);
                self.reveal_at(group, position, cx.rect(), cx.scale());
                cx.request_paint();
                Handled::Yes
            }
            _ => Handled::No,
        }
    }
}

impl DockArea {
    /// The tab numbered `serial`; tab lists' numbers are no tab's.
    fn tab_with_serial(&self, serial: u32) -> Option<usize> {
        if serial >= SERIALS {
            return None;
        }
        self.tabs.iter().position(|tab| tab.serial == serial)
    }
}
