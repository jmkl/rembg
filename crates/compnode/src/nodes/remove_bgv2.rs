//! Background removal node v2 (uses new RemoveBg library with RMBG-1.4 logic).

use egui::Ui;
use egui_snarl::{NodeId, Snarl};
use crate::style::controls::Button;

use crate::editor::GraphEditor;
use crate::graph::upstream_texture;
use crate::model::{Kind, Node};
use crate::widgets::{error_line, preview, timing_line};

pub(crate) fn create(snarl: &mut Snarl<Node>, pos: egui::Pos2, _kind: Kind) {
    snarl.insert_node(pos, Node::remove_bg_v2());
}

pub(crate) fn show_body(
    editor: &mut GraphEditor,
    node: NodeId,
    ui: &mut Ui,
    snarl: &mut Snarl<Node>,
    width: f32,
) {
    let texture = snarl[node]
        .texture()
        .cloned()
        .or_else(|| upstream_texture(snarl, node));
    if snarl[node].preview_enabled() {
        preview(ui, texture.as_ref(), egui::vec2(width, 140.0));
    }

    let busy = snarl[node].job().is_some();
    let has_output = snarl[node].texture().is_some();
    let mut run = false;
    if busy {
        ui.add(Button::new("Processing...").full_width().loading(true));
    } else {
        let label = if has_output {
            "Re-run"
        } else {
            "Remove background (v2)"
        };
        let button = if has_output {
            Button::new(label).outline()
        } else {
            Button::new(label)
        };
        run = ui.add(button.full_width()).clicked();
    }
    if run {
        editor.start_remove_bg_v2(snarl, node, ui.ctx());
    }
    error_line(ui, snarl[node].error());
    timing_line(ui, snarl[node].timing());
}