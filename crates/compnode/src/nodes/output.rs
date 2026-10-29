//! Multi-layer viewer node.

use egui::Ui;
use egui_snarl::{NodeId, Snarl};

use crate::editor::GraphEditor;
use crate::model::{Kind, Node};
use crate::style::controls::muted;
use crate::style::font;
use crate::widgets::{node_body, preview_layers};

pub(crate) fn create(snarl: &mut Snarl<Node>, pos: egui::Pos2, _kind: Kind) {
    snarl.insert_node(pos, Node::viewer());
}

pub(crate) fn show_body(
    _editor: &mut GraphEditor,
    node: NodeId,
    ui: &mut Ui,
    snarl: &mut Snarl<Node>,
    _width: f32,
) {
    node_body(ui);
    let Some(output) = snarl[node].viewer_node() else {
        return;
    };
    if snarl[node].preview_enabled() {
        preview_layers(ui, &output.layers, output.canvas_size);
    }
    let text = match output.layers.len() {
        0 => "No layers".to_owned(),
        1 => "1 layer".to_owned(),
        count => format!("{count} layers"),
    };
    ui.add(egui::Label::new(muted(text, font::SMALL)).selectable(false));
}
