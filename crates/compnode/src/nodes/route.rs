//! Route node - pass-through routing with toggle switch, no preview.

use egui_snarl::{NodeId, Snarl};

use crate::editor::GraphEditor;
use crate::model::Node;
use crate::style::controls::Checkbox;

#[allow(dead_code)]
pub(crate) fn create(snarl: &mut Snarl<Node>, pos: egui::Pos2, _kind: crate::model::Kind) {
    snarl.insert_node(pos, Node::route());
}

pub(crate) fn show_body(
    editor: &mut GraphEditor,
    node: NodeId,
    ui: &mut egui::Ui,
    snarl: &mut Snarl<Node>,
    _width: f32,
) {
    let mut enabled = snarl[node].route_enabled();
    let before = snarl[node].clone();
    if ui.add(Checkbox::new(&mut enabled, "Pass-through")).changed() {
        snarl[node].set_route_enabled(enabled);
        crate::graph::invalidate_node(snarl, node);
        editor.record_node_change(node, before, snarl);
    }
}
