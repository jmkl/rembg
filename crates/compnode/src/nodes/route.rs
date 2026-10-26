//! Route node - simple pass-through with centered title, no preview, no body.

use egui_snarl::{NodeId, Snarl};

use crate::editor::GraphEditor;
use crate::model::Node;

#[allow(dead_code)]
pub(crate) fn create(snarl: &mut Snarl<Node>, pos: egui::Pos2, _kind: crate::model::Kind) {
    snarl.insert_node(pos, Node::new_filter(crate::model::FilterKind::Route));
}

pub(crate) fn show_body(
    _editor: &mut GraphEditor,
    _node: NodeId,
    _ui: &mut egui::Ui,
    _snarl: &mut Snarl<Node>,
    _width: f32,
) {
    // Route node has no body; always collapsed.
}
