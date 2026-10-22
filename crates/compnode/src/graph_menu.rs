//! Filterable "add node" menu used by right-clicking the canvas.
//!
//! The menu is rendered by `app` with `PopupCloseBehavior::CloseOnClickOutside`
//! so clicking the filter input does not dismiss it.

use egui::{RichText, Ui};
use egui_snarl::Snarl;
use elegance::{MenuItem, TextInput};

use crate::catalog::NODE_SPECS;
use crate::model::Node;
use crate::style::pal;

pub(crate) fn show(ui: &mut Ui, snarl: &mut Snarl<Node>, filter: &mut String, pos: egui::Pos2) {
    ui.set_min_width(220.0);
    ui.add(
        TextInput::new(filter)
            .hint("Filter nodes…")
            .compact(true)
            .desired_width(220.0)
            .id_salt("graph_node_filter"),
    );
    ui.add_space(3.0);

    let mut shown = 0;
    for spec in NODE_SPECS {
        if !spec.matches(filter) {
            continue;
        }
        shown += 1;
        if ui
            .add(MenuItem::new(spec.title).icon(spec.icon.to_string()))
            .clicked()
        {
            (spec.create)(snarl, pos);
            ui.close();
        }
    }

    if shown == 0 {
        ui.add(
            egui::Label::new(
                RichText::new("No matching nodes")
                    .size(10.0)
                    .color(pal::MUTED),
            )
            .selectable(false),
        );
    }
}
