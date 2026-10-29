//! Generic body UI for every filter node.

use egui::{Color32, Ui, Vec2};
use egui_snarl::{NodeId, Snarl};

use crate::catalog::{FieldKind, FieldSpec};
use crate::editor::GraphEditor;
use crate::graph::invalidate_node;
use crate::model::{FilterKind, Kind, Node};
use crate::ops;
use crate::params::{self, CurvePoint, Params};
use crate::style::controls::{Button, ColorField, Select, Slider};
use crate::style::pal;
use crate::widgets::{error_line, node_body, node_preview};

pub(crate) fn create(snarl: &mut Snarl<Node>, pos: egui::Pos2, kind: Kind) {
    match kind {
        Kind::Filter(filter_kind) => {
            snarl.insert_node(pos, Node::new_filter(filter_kind));
        }
        Kind::Group => {
            snarl.insert_node(pos, Node::group());
        }
        Kind::Blend => {
            snarl.insert_node(pos, Node::blend());
        }
        _ => {}
    }
}

pub(crate) fn show_body(
    _editor: &mut GraphEditor,
    node: NodeId,
    ui: &mut Ui,
    snarl: &mut Snarl<Node>,
    width: f32,
) {
    node_body(ui);
    let (node_kind, texture, preview_enabled, error, fields, defaults) = match &snarl[node] {
        Node::Filter(f) => (
            Kind::Filter(f.kind),
            f.preview.texture.clone(),
            f.preview_enabled,
            f.preview.error.clone(),
            crate::catalog::spec(Kind::Filter(f.kind)).fields,
            f.kind.defaults(),
        ),
        Node::Blend(g) => {
            let spec = crate::catalog::spec(Kind::Blend);
            let mut defs = Params::new();
            spec.apply_defaults(&mut defs);
            (
                Kind::Blend,
                g.preview.texture.clone(),
                g.preview_enabled,
                g.preview.error.clone(),
                spec.fields,
                defs,
            )
        }
        Node::Group(g) => {
            let spec = crate::catalog::spec(Kind::Group);
            (
                Kind::Group,
                g.preview.texture.clone(),
                g.preview_enabled,
                g.preview.error.clone(),
                spec.fields,
                Params::new(),
            )
        }
        _ => return,
    };

    if preview_enabled {
        node_preview(ui, texture.as_ref(), Vec2::new(width, 140.0));
    }
    error_line(ui, error.as_deref());

    let mut changed = false;
    {
        let params = match &mut snarl[node] {
            Node::Filter(f) => &mut f.params,
            Node::Blend(g) | Node::Group(g) => &mut g.params,
            _ => return,
        };
        for field in fields {
            changed |= show_field(ui, params, field, &defaults);
        }
        if node_kind == Kind::Filter(FilterKind::Curves) {
            changed |= curve_editor(ui, params);
        }
    }
    if changed {
        invalidate_node(snarl, node);
    }
}

fn show_field(ui: &mut Ui, params: &mut Params, field: &FieldSpec, defaults: &Params) -> bool {
    match &field.kind {
        FieldKind::Range {
            min,
            max,
            default,
            step,
            suffix,
        } => {
            let mut value = params::f32(params, field.key, *default);
            let response = ui.add(
                Slider::new(&mut value, *min..=*max)
                    .label(field.label)
                    .suffix(*suffix)
                    .step(*step)
                    .decimals(decimals_for(*step))
                    .default(params::f32(defaults, field.key, *default)),
            );
            if response.changed() {
                params::set_f32(params, field.key, value);
                true
            } else {
                false
            }
        }
        FieldKind::Number {
            min,
            max,
            default,
            step,
        } => {
            let mut value = params::f32(params, field.key, *default);
            let response = ui.add(
                Slider::new(&mut value, *min..=*max)
                    .label(field.label)
                    .step(*step)
                    .decimals(decimals_for(*step))
                    .default(params::f32(defaults, field.key, *default)),
            );
            if response.changed() {
                params::set_f32(params, field.key, value);
                true
            } else {
                false
            }
        }
        FieldKind::Color { default } => {
            let rgb = params::color(params, field.key, default);
            let mut color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            let response =
                ui.add(ColorField::new((field.key, field.label), &mut color).label(field.label));
            if response.changed() {
                params::set_string(
                    params,
                    field.key,
                    params::rgb_hex([color.r(), color.g(), color.b()]),
                );
                true
            } else {
                false
            }
        }
        FieldKind::Select { options, default } => {
            let current = params::string(params, field.key, default).to_owned();
            let mut selected = current.clone();
            let response = ui.add(
                Select::new((field.key, field.label), &mut selected)
                    .label(field.label)
                    .options(options.iter().map(|o| (o.value.to_owned(), o.label)))
                    .width(ui.available_width()),
            );
            if response.changed() {
                params::set_string(params, field.key, selected);
                true
            } else {
                false
            }
        }
    }
}

/// Fraction digits for a slider driven by `step` (1.0 → 0 digits, 0.1 → 1, ...).
fn decimals_for(step: f64) -> usize {
    if step >= 1.0 {
        0
    } else if step >= 0.1 {
        1
    } else if step >= 0.01 {
        2
    } else {
        3
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

    painter.rect_filled(rect, 4.0, pal::INPUT_BG);
    // Grid.
    for step in 1..4 {
        let t = step as f32 / 4.0;
        let x = rect.left() + rect.width() * t;
        let y = rect.top() + rect.height() * (1.0 - t);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, pal::NODE_STROKE),
        );
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(1.0, pal::NODE_STROKE),
        );
    }
    // Diagonal reference.
    painter.line_segment(
        [rect.left_bottom(), rect.right_top()],
        egui::Stroke::new(1.0, pal::NODE_STROKE),
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
        let pos = egui::pos2(
            rect.left() + rect.width() * t,
            rect.bottom() - rect.height() * y,
        );
        if let Some(previous) = prev {
            painter.line_segment([previous, pos], egui::Stroke::new(1.5, pal::ACCENT));
        }
        prev = Some(pos);
    }

    // Handles.
    for (index, point) in points.iter().enumerate() {
        let pos = to_screen(*point);
        painter.circle_filled(pos, 5.0, pal::WHITE);
        painter.circle_stroke(pos, 5.0, egui::Stroke::new(1.0, pal::BLACK));
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
            if let Some(index) = points
                .iter()
                .position(|p| (p.x - x).abs() < 1e-4 && (p.y - y).abs() < 1e-4)
            {
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
    if ui.add(Button::new("Reset").small()).clicked() {
        points = params::identity_curve();
        changed = true;
    }

    if changed {
        params::set_curve(params, &channel, &points);
    }
    changed
}
