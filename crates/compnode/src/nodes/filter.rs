//! Generic body UI for every filter node.

use egui::{Color32, RichText, Ui, Vec2};
use egui_snarl::{NodeId, Snarl};

use crate::catalog::{FieldKind, FieldSpec, NodeSpec};
use crate::editor::GraphEditor;
use crate::graph::invalidate_node;
use crate::model::{FilterKind, Kind, Node};
use crate::ops;
use crate::params::{self, CurvePoint, Params};
use crate::style::pal;
use crate::widgets::{error_line, preview};

pub(crate) fn create(snarl: &mut Snarl<Node>, pos: egui::Pos2, kind: Kind) {
    let filter_kind = match kind {
        Kind::Filter(value) => value,
        _ => return,
    };
    snarl.insert_node(pos, Node::new_filter(filter_kind));
}

pub(crate) fn show_body(
    _editor: &mut GraphEditor,
    node: NodeId,
    ui: &mut Ui,
    snarl: &mut Snarl<Node>,
    width: f32,
) {
    let (kind, texture, preview_enabled, error) = {
        let Some(filter) = snarl[node].filter() else {
            return;
        };
        (
            filter.kind,
            filter.preview.texture.clone(),
            filter.preview_enabled,
            filter.preview.error.clone(),
        )
    };

    if preview_enabled {
        preview(ui, texture.as_ref(), Vec2::new(width, 140.0));
    }
    error_line(ui, error.as_deref());

    let fields = spec_for(kind).fields;
    let mut changed = false;
    {
        let Some(filter) = snarl[node].filter_mut() else {
            return;
        };
        for field in fields {
            changed |= show_field(ui, &mut filter.params, field);
        }
        if kind == FilterKind::Curves {
            changed |= curve_editor(ui, &mut filter.params);
        }
    }
    if changed {
        invalidate_node(snarl, node);
    }
}

fn spec_for(kind: FilterKind) -> &'static NodeSpec {
    crate::catalog::spec(Kind::Filter(kind))
}

fn show_field(ui: &mut Ui, params: &mut Params, field: &FieldSpec) -> bool {
    match &field.kind {
        FieldKind::Range { min, max, step, suffix } => {
            ui.label(RichText::new(field.label).size(9.0).color(pal::MUTED));
            let mut value = params::f32(params, field.key, *min);
            let response = ui.add(
                egui::Slider::new(&mut value, *min..=*max)
                    .step_by(*step)
                    .suffix(*suffix),
            );
            if response.changed() {
                params::set_f32(params, field.key, value);
                true
            } else {
                false
            }
        }
        FieldKind::Number { min, max, step } => {
            ui.label(RichText::new(field.label).size(9.0).color(pal::MUTED));
            let mut value = params::f32(params, field.key, *min);
            let response = ui.add(
                egui::DragValue::new(&mut value)
                    .range(*min..=*max)
                    .speed(*step),
            );
            if response.changed() {
                params::set_f32(params, field.key, value);
                true
            } else {
                false
            }
        }
        FieldKind::Color => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(field.label).size(9.0).color(pal::MUTED));
                let mut rgb = params::color(params, field.key, "#000000");
                let response = ui.color_edit_button_srgb(&mut rgb);
                if response.changed() {
                    params::set_string(params, field.key, params::rgb_hex(rgb));
                }
                response.changed()
            })
            .inner
        }
        FieldKind::Select(options) => {
            ui.label(RichText::new(field.label).size(9.0).color(pal::MUTED));
            let current = params::string(params, field.key, options.first().map(|o| o.value).unwrap_or("")).to_owned();
            let mut selected = current.clone();
            let mut changed = false;
            egui::ComboBox::from_id_salt((field.key, field.label))
                .selected_text(
                    options
                        .iter()
                        .find(|option| option.value == current)
                        .map(|option| option.label)
                        .unwrap_or(current.as_str()),
                )
                .show_ui(ui, |ui| {
                    for option in *options {
                        if ui
                            .selectable_value(&mut selected, option.value.to_owned(), option.label)
                            .clicked()
                        {
                            changed = true;
                        }
                    }
                });
            if changed || selected != current {
                params::set_string(params, field.key, selected);
                true
            } else {
                false
            }
        }
    }
}

// ---------------------------------------------------------------- curves ----

fn curve_editor(ui: &mut Ui, params: &mut Params) -> bool {
    let channel = params::string(params, "channel", "rgb").to_owned();
    let mut points = params::curve(params, &channel);
    let mut changed = false;

    let size = Vec2::new(ui.available_width().max(120.0), 150.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 4.0, Color32::from_gray(24));
    // Grid.
    for step in 1..4 {
        let t = step as f32 / 4.0;
        let x = rect.left() + rect.width() * t;
        let y = rect.top() + rect.height() * (1.0 - t);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, Color32::from_gray(40)),
        );
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(1.0, Color32::from_gray(40)),
        );
    }
    // Diagonal reference.
    painter.line_segment(
        [rect.left_bottom(), rect.right_top()],
        egui::Stroke::new(1.0, Color32::from_gray(60)),
    );

    // Map normalized point to screen.
    let to_screen = |point: CurvePoint| -> egui::Pos2 {
        egui::pos2(
            rect.left() + rect.width() * point.x.clamp(0.0, 1.0),
            rect.bottom() - rect.height() * point.y.clamp(0.0, 1.0),
        )
    };

    // Draw the curve using the LUT.
    let lut = ops::curve_lut(&points);
    let mut prev: Option<egui::Pos2> = None;
    for (index, value) in lut.iter().enumerate() {
        let t = index as f32 / 255.0;
        let y = *value as f32 / 255.0;
        let pos = egui::pos2(rect.left() + rect.width() * t, rect.bottom() - rect.height() * y);
        if let Some(previous) = prev {
            painter.line_segment([previous, pos], egui::Stroke::new(1.5, Color32::from_rgb(0x7a, 0xb8, 0xff)));
        }
        prev = Some(pos);
    }

    // Handles.
    for (index, point) in points.iter().enumerate() {
        let pos = to_screen(*point);
        painter.circle_filled(pos, 5.0, Color32::from_rgb(0xff, 0xff, 0xff));
        painter.circle_stroke(pos, 5.0, egui::Stroke::new(1.0, Color32::BLACK));
        let _ = index;
    }

    // Interaction.
    let pointer = response.interact_pointer_pos();
    let id = ui.id().with("curve");
    let dragging: Option<usize> = ui.data(|data| data.get_temp(id));

    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(pos) = pointer
    {
        let mut nearest = None;
        let mut best = 12.0f32;
        for (index, point) in points.iter().enumerate() {
            let handle = to_screen(*point);
            let distance = handle.distance(pos);
            if distance < best {
                best = distance;
                nearest = Some(index);
            }
        }
        if let Some(index) = nearest {
            ui.data_mut(|data| data.insert_temp(id, index));
        } else {
            let x = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let y = (1.0 - (pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            points.push(CurvePoint { x, y });
            points.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
            if let Some(index) = points.iter().position(|p| (p.x - x).abs() < 1e-4 && (p.y - y).abs() < 1e-4) {
                ui.data_mut(|data| data.insert_temp(id, index));
            }
            changed = true;
        }
    }

    if response.dragged_by(egui::PointerButton::Primary)
        && let Some(index) = dragging
        && let Some(pos) = pointer
    {
        let x = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let y = (1.0 - (pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        let count = points.len();
        let mut new_x = x;
        if index == 0 {
            new_x = 0.0;
        } else if index + 1 == count {
            new_x = 1.0;
        } else {
            let prev_x = points[index - 1].x;
            let next_x = points[index + 1].x;
            new_x = new_x.clamp(prev_x + 0.001, next_x - 0.001);
        }
        if (points[index].x - new_x).abs() > 1e-5 || (points[index].y - y).abs() > 1e-5 {
            points[index].x = new_x;
            points[index].y = y;
            changed = true;
        }
    }

    if response.drag_stopped_by(egui::PointerButton::Primary) {
        ui.data_mut(|data| data.remove_temp::<usize>(id));
    }

    if response.clicked_by(egui::PointerButton::Secondary)
        && let Some(pos) = pointer
    {
        let mut nearest = None;
        let mut best = 12.0f32;
        for (index, point) in points.iter().enumerate() {
            if index == 0 || index + 1 == points.len() {
                continue;
            }
            let handle = to_screen(*point);
            let distance = handle.distance(pos);
            if distance < best {
                best = distance;
                nearest = Some(index);
            }
        }
        if let Some(index) = nearest {
            points.remove(index);
            changed = true;
        }
    }

    // Reset button.
    ui.horizontal(|ui| {
        if ui.small_button("Reset").clicked() {
            points = params::identity_curve();
            changed = true;
        }
    });

    if changed {
        params::set_curve(params, &channel, &points);
    }
    changed
}

