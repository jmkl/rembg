//! Filterable "add node" menu used by right-clicking the canvas.
//!
//! The menu is rendered by `app` with `PopupCloseBehavior::CloseOnClickOutside`
//! so clicking the filter input does not dismiss it.

use egui::Ui;
use egui_snarl::Snarl;

use crate::catalog::{self, NodeCategory};
use crate::model::{Kind, Node};
use crate::style::controls::{MenuItem, muted};
use crate::style::font;

pub(crate) fn show(
    ui: &mut Ui,
    snarl: &mut Snarl<Node>,
    filter: &mut String,
    recent: &mut Vec<Kind>,
    pos: egui::Pos2,
) {
    ui.set_min_width(220.0);
    ui.add(
        egui::TextEdit::singleline(filter)
            .hint_text("Filter nodes…")
            .desired_width(220.0)
            .id_salt("graph_node_filter"),
    );
    ui.add_space(3.0);

    let query = filter.trim();
    let mut shown = 0;
    if query.is_empty() && !recent.is_empty() {
        ui.label(muted("Recently used", font::SMALL));
        let recent_snapshot = recent.clone();
        for kind in recent_snapshot {
            let spec = catalog::spec(kind);
            if !spec.hidden {
                shown += 1;
                if add_row(ui, spec, snarl, pos, recent) {
                    return;
                }
            }
        }
        ui.separator();
    }

    let specs = catalog::all_specs();
    let mut matches: Vec<_> = specs
        .into_iter()
        .filter(|spec| !spec.hidden)
        .filter_map(|spec| spec.match_score(query).map(|score| (score, spec)))
        .collect();
    matches.sort_by_key(|(score, spec)| (*score, spec.title));
    for category in NodeCategory::ALL {
        let rows: Vec<_> = matches
            .iter()
            .filter(|(_, spec)| spec.category() == category)
            .map(|(_, spec)| *spec)
            .collect();
        if rows.is_empty() {
            continue;
        }
        shown += rows.len();
        egui::CollapsingHeader::new(category.label())
            .default_open(!query.is_empty())
            .show(ui, |ui| {
                for spec in rows {
                    if add_row(ui, spec, snarl, pos, recent) {
                        ui.close();
                    }
                }
            });
    }

    if shown == 0 {
        ui.add(egui::Label::new(muted("No matching nodes", font::SMALL)).selectable(false));
    }
}

fn add_row(
    ui: &mut Ui,
    spec: &'static crate::catalog::NodeSpec,
    snarl: &mut Snarl<Node>,
    pos: egui::Pos2,
    recent: &mut Vec<Kind>,
) -> bool {
    if !ui.add(MenuItem::new(spec.title).icon(spec.icon)).clicked() {
        return false;
    }
    (spec.create)(snarl, pos, spec.kind);
    recent.retain(|kind| *kind != spec.kind);
    recent.insert(0, spec.kind);
    recent.truncate(5);
    ui.close();
    true
}
