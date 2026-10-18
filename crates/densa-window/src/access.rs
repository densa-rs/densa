//! densa's accessibility tree in AccessKit's terms, and AccessKit's requests in densa's.

use accesskit::{
    Action, ActionData, ActionRequest, Node, NodeId, Rect, Role as KitRole, TreeId, TreeInfo,
    TreeUpdate,
};
use densa_ui::{AccessAction, AccessId, AccessNode, AccessTree, Role, Ui};

/// The whole tree as an update: AccessKit replaces every node it is given, and a node left
/// out of its parent's children is removed with its descendants.
pub(crate) fn tree_update(tree: &AccessTree, title: &str) -> TreeUpdate {
    let root = tree.root();
    let nodes = tree
        .nodes()
        .iter()
        .map(|(id, node)| {
            let mut kit = node_of(node);
            if *id == root && node.label().is_none() {
                kit.set_label(title);
            }
            (NodeId(id.to_raw()), kit)
        })
        .collect();
    let mut info = TreeInfo::new(NodeId(root.to_raw()));
    info.toolkit_name = Some("densa".to_owned());
    info.toolkit_version = Some(env!("CARGO_PKG_VERSION").to_owned());
    TreeUpdate {
        nodes,
        tree: Some(info),
        tree_id: TreeId::ROOT,
        focus: NodeId(tree.focus().to_raw()),
    }
}

fn node_of(node: &AccessNode) -> Node {
    let mut kit = Node::new(role(node.role()));
    if let Some(label) = node.label() {
        kit.set_label(label);
    }
    if let Some(value) = node.value() {
        kit.set_value(value);
    }
    if let Some(placeholder) = node.placeholder() {
        kit.set_placeholder(placeholder);
    }
    let b = node.bounds();
    kit.set_bounds(Rect {
        x0: f64::from(b.left()),
        y0: f64::from(b.top()),
        x1: f64::from(b.right()),
        y1: f64::from(b.bottom()),
    });
    for action in node.actions() {
        if let Some(action) = kit_action(action) {
            kit.add_action(action);
        }
    }
    if let Some(selected) = node.selected() {
        kit.set_selected(selected);
    }
    if node.is_disabled() {
        kit.set_disabled();
    }
    if node.clips_children() {
        kit.set_clips_children();
    }
    kit.set_children(
        node.children()
            .iter()
            .map(|c| NodeId(c.to_raw()))
            .collect::<Vec<_>>(),
    );
    kit
}

fn role(role: Role) -> KitRole {
    match role {
        Role::Window => KitRole::Window,
        Role::Label => KitRole::Label,
        Role::Button => KitRole::Button,
        Role::TextInput => KitRole::TextInput,
        Role::ScrollView => KitRole::ScrollView,
        Role::Splitter => KitRole::Splitter,
        Role::TabList => KitRole::TabList,
        Role::Tab => KitRole::Tab,
        Role::TabPanel => KitRole::TabPanel,
        _ => KitRole::GenericContainer,
    }
}

fn kit_action(action: AccessAction) -> Option<Action> {
    Some(match action {
        AccessAction::Click => Action::Click,
        AccessAction::Focus => Action::Focus,
        AccessAction::SetValue => Action::SetValue,
        AccessAction::ScrollUp => Action::ScrollUp,
        AccessAction::ScrollDown => Action::ScrollDown,
        AccessAction::ScrollLeft => Action::ScrollLeft,
        AccessAction::ScrollRight => Action::ScrollRight,
        AccessAction::ScrollIntoView => Action::ScrollIntoView,
        _ => return None,
    })
}

fn densa_action(action: Action) -> Option<AccessAction> {
    Some(match action {
        Action::Click => AccessAction::Click,
        Action::Focus => AccessAction::Focus,
        Action::SetValue => AccessAction::SetValue,
        Action::ScrollUp => AccessAction::ScrollUp,
        Action::ScrollDown => AccessAction::ScrollDown,
        Action::ScrollLeft => AccessAction::ScrollLeft,
        Action::ScrollRight => AccessAction::ScrollRight,
        Action::ScrollIntoView => AccessAction::ScrollIntoView,
        _ => return None,
    })
}

/// Carries out an AccessKit request on `ui`; whether anything did it.
pub(crate) fn perform(ui: &mut Ui, request: &ActionRequest) -> bool {
    let Some(action) = densa_action(request.action) else {
        return false;
    };
    let value = match &request.data {
        Some(ActionData::Value(value)) => Some(&**value),
        _ => None,
    };
    let target = AccessId::from_raw(request.target_node.0);
    ui.access_action(target, action, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use densa_core::Size;
    use densa_paragraph::TextSystem;
    use densa_ui::{AccessRequest, EventCx, Handled, PaintCx, Style, Widget};

    /// A button-like widget counting clicks and recording set values.
    #[derive(Default)]
    struct Pressable {
        clicks: u32,
        value: String,
    }

    impl Widget for Pressable {
        fn accessibility(&self, _: &PaintCx<'_>, node: &mut AccessNode) {
            node.set_role(Role::Button);
            node.set_label("Buy");
            node.add_action(AccessAction::Click);
            node.set_selected(true);
            node.set_disabled(true);
            node.set_placeholder("Order size");
            let mut part = AccessNode::new(Role::Tab);
            part.set_value("tab");
            node.push_part(0, part);
        }

        fn access_action(&mut self, _: &mut EventCx<'_>, request: &AccessRequest<'_>) -> Handled {
            match (request.action, request.value) {
                (AccessAction::Click, _) => self.clicks += 1,
                (AccessAction::SetValue, Some(value)) => self.value = value.to_owned(),
                _ => return Handled::No,
            }
            Handled::Yes
        }
    }

    fn ui() -> (Ui, densa_ui::WidgetId) {
        let mut ui = Ui::new(TextSystem::bundled().unwrap());
        ui.resize(Size::new(200.0, 100.0));
        let clip = Style::row()
            .size(50.0, 20.0)
            .overflow(densa_ui::Overflow::Clip);
        let id = ui.add(ui.root(), Pressable::default(), clip).unwrap();
        (ui, id)
    }

    #[test]
    fn the_update_carries_every_node_with_roles_names_and_children() {
        let (mut ui, id) = ui();
        let update = tree_update(&ui.access_tree(), "Orders");
        let tree = update.tree.as_ref().unwrap();
        let node = |raw: u64| &update.nodes.iter().find(|(n, _)| n.0 == raw).unwrap().1;
        let root = node(tree.root.0);
        assert_eq!(root.role(), KitRole::Window);
        assert_eq!(
            root.label(),
            Some("Orders"),
            "the window is named by its title"
        );
        let button_id = ui.access_id(id).unwrap().to_raw();
        assert_eq!(root.children(), [NodeId(button_id)]);
        let button = node(button_id);
        assert_eq!(
            (button.role(), button.label()),
            (KitRole::Button, Some("Buy"))
        );
        assert!(button.supports_action(Action::Click));
        assert!(button.supports_action(Action::ScrollIntoView));
        assert_eq!(button.is_selected(), Some(true));
        assert!(button.is_disabled());
        let b = button.bounds().unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (0.0, 0.0, 50.0, 20.0));
        let part = NodeId(ui.access_id(id).unwrap().part(0).to_raw());
        assert_eq!(button.children(), [part]);
        assert_eq!(node(part.0).value(), Some("tab"));
        assert_eq!(update.focus, tree.root, "no widget has focus");
        assert!(button.clips_children(), "it clips what it holds");
        assert_eq!(button.placeholder(), Some("Order size"));
        assert!(!root.clips_children());
        assert_eq!(update.tree_id, TreeId::ROOT);
    }

    #[test]
    fn requests_reach_the_widget_with_their_text() {
        let (mut ui, id) = ui();
        let target = NodeId(ui.access_id(id).unwrap().to_raw());
        let request = |action, data| ActionRequest {
            action,
            target_tree: TreeId::ROOT,
            target_node: target,
            data,
        };
        assert!(perform(&mut ui, &request(Action::Click, None)));
        let value = Some(ActionData::Value("MSFT".into()));
        assert!(perform(&mut ui, &request(Action::SetValue, value)));
        // An action densa does not know is refused.
        assert!(!perform(&mut ui, &request(Action::Expand, None)));
        let widget = ui.widget::<Pressable>(id).unwrap();
        assert_eq!((widget.clicks, widget.value.as_str()), (1, "MSFT"));
    }

    #[test]
    fn every_action_maps_both_ways() {
        let all = [
            AccessAction::Click,
            AccessAction::Focus,
            AccessAction::SetValue,
            AccessAction::ScrollUp,
            AccessAction::ScrollDown,
            AccessAction::ScrollLeft,
            AccessAction::ScrollRight,
            AccessAction::ScrollIntoView,
        ];
        for action in all {
            assert_eq!(densa_action(kit_action(action).unwrap()), Some(action));
        }
    }

    #[test]
    fn every_role_has_its_own_accesskit_role() {
        let roles = [
            (Role::Group, KitRole::GenericContainer),
            (Role::Window, KitRole::Window),
            (Role::Label, KitRole::Label),
            (Role::Button, KitRole::Button),
            (Role::TextInput, KitRole::TextInput),
            (Role::ScrollView, KitRole::ScrollView),
            (Role::Splitter, KitRole::Splitter),
            (Role::TabList, KitRole::TabList),
            (Role::Tab, KitRole::Tab),
            (Role::TabPanel, KitRole::TabPanel),
        ];
        for (densa, kit) in roles {
            assert_eq!(role(densa), kit);
        }
    }
}
