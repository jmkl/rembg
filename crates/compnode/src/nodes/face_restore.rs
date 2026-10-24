//! Face restoration node.

use egui::Ui;
use egui_snarl::{NodeId, Snarl};
use crate::style::controls::{Button, Checkbox, Slider};

use crate::editor::GraphEditor;
use crate::graph::upstream_texture;
use crate::model::{Kind, Node};
use crate::widgets::{error_line, preview, timing_line};

pub(crate) fn create(snarl: &mut Snarl<Node>, pos: egui::Pos2, _kind: Kind) {
    snarl.insert_node(pos, Node::face_restore());
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

    let (mut strength, mut only_center) = snarl[node]
        .face_restore_node()
        .map(|data| (data.strength, data.only_center))
        .unwrap_or((1.0, false));

    let mut percent = strength * 100.0;
    if ui
        .add(
            Slider::new(&mut percent, 0.0..=100.0)
                .label("Strength")
                .suffix("%")
                .decimals(0),
        )
        .changed()
    {
        strength = percent / 100.0;
        if let Some(data) = snarl[node].face_restore_node_mut() {
            data.strength = strength;
        }
    }
    if ui
        .add(Checkbox::new(&mut only_center, "Center face only"))
        .changed()
        && let Some(data) = snarl[node].face_restore_node_mut()
    {
        data.only_center = only_center;
    }

    let busy = snarl[node].job().is_some();
    let has_output = snarl[node].texture().is_some();
    let mut run = false;
    if busy {
        ui.add(Button::new("Processing…").full_width().loading(true));
    } else {
        let label = if has_output {
            "Re-run"
        } else {
            "Restore faces"
        };
        let button = if has_output {
            Button::new(label).outline()
        } else {
            Button::new(label)
        };
        run = ui.add(button.full_width()).clicked();
    }
    if run {
        editor.start_face_restore(snarl, node, ui.ctx());
    }
    error_line(ui, snarl[node].error());
    timing_line(ui, snarl[node].timing());
}
