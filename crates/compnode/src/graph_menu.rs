//! Filterable "add node" menu used by right-clicking the canvas.
//!
//! The menu is rendered by `app` with `PopupCloseBehavior::CloseOnClickOutside`
//! so clicking the filter input does not dismiss it.

use egui::Ui;
use egui_snarl::Snarl;

use crate::catalog::NODE_SPECS;
use crate::model::Node;
use crate::style::controls::{MenuItem, muted};
use crate::style::font;

pub(crate) fn show(ui: &mut Ui, snarl: &mut Snarl<Node>, filter: &mut String, pos: egui::Pos2) {
    ui.set_min_width(220.0);
    ui.add(
        egui::TextEdit::singleline(filter)
            .hint_text("Filter nodes…")
            .desired_width(220.0)
            .id_salt("graph_node_filter"),
    );
    ui.add_space(3.0);

    let mut shown = 0;
    for spec in NODE_SPECS {
        if spec.hidden {
            continue;
        }
        if !spec.matches(filter) {
            continue;
        }
        shown += 1;
        if ui.add(MenuItem::new(spec.title).icon(spec.icon)).clicked() {
            (spec.create)(snarl, pos, spec.kind);
            ui.close();
        }
    }

    if shown == 0 {
        ui.add(egui::Label::new(muted("No matching nodes", font::SMALL)).selectable(false));
    }
}
