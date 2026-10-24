//! Route node - simple pass-through with centered title, no preview, no body.

use egui::{RichText, Ui};
use egui_snarl::{NodeId, Snarl};

use crate::editor::GraphEditor;
use crate::model::Node;
use crate::style::pal;

pub(crate) fn create(snarl: &mut Snarl<Node>, pos: egui::Pos2, _kind: crate::model::Kind) {
    snarl.insert_node(pos, Node::new_filter(crate::model::FilterKind::Route));
}

pub(crate) fn show_body(
    _editor: &mut GraphEditor,
    node: NodeId,
    ui: &mut egui::Ui,
    snarl: &mut Snarl<Node>,
    _width: f32,
) {
    // No preview for Route node
    // Just show centered title that fills the width
    let title = "Route";
    ui.vertical_centered(|ui| {
        ui.add_space(8.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new("Route")
                    .size(14.0)
                    .strong()
                    .color(crate::style::pal::HEADER_TEXT),
            )
            .selectable(false),
        );
        ui.add_space(8.0);
    });
}