//! Small reusable drawing/widget helpers shared by node bodies.

use egui::{
    Align2, CornerRadius, FontId, Mesh, Pos2, Rect, Shape, Stroke, Vec2,
    epaint::Vertex, pos2, vec2,
};
use egui_snarl::ui::PinInfo;

use crate::model::{RunTiming, ViewerLayer};
use crate::style::controls::muted;
use crate::style::{font, pal};

/// Viewer preview: draw every layer bottom-to-top into the same frame,
/// applying each layer's center/size/angle transform so it matches ImageCanvas.
pub(crate) fn preview_layers(ui: &mut egui::Ui, layers: &[ViewerLayer], canvas_size: Vec2) {
    // Fit the logical canvas into the available width, same as ImageCanvas.
    let available_width = ui.available_width().max(64.0);
    let scale = (available_width / canvas_size.x).min(1.0).max(0.05);
    let screen_size = canvas_size * scale;

    let (rect, _resp) = ui.allocate_exact_size(screen_size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let origin = rect.min.to_vec2();

    let to_screen = move |v: Vec2| (origin + v * scale).to_pos2();

    // Match ImageCanvas background: checkerboard.
    painter.rect_filled(rect, CornerRadius::ZERO, pal::CHECK_A);

    let cell = 8.0;
    let mut y = rect.top();
    let mut row = 0;
    while y < rect.bottom() {
        let mut x = rect.left() + if row % 2 == 0 { 0.0 } else { cell };
        while x < rect.right() {
            let tile = Rect::from_min_max(
                pos2(x, y),
                pos2((x + cell).min(rect.right()), (y + cell).min(rect.bottom())),
            );
            painter.rect_filled(tile, CornerRadius::ZERO, pal::CHECK_B);
            x += cell * 2.0;
        }
        y += cell;
        row += 1;
    }

    let mut painted = false;
    for layer in layers {
        if let Some(texture) = &layer.texture {
            let (hw, hh) = (layer.size.x / 2.0, layer.size.y / 2.0);
            let s = layer.angle.sin();
            let c = layer.angle.cos();
            let corners = [vec2(-hw, -hh), vec2(hw, -hh), vec2(hw, hh), vec2(-hw, hh)].map(|l| {
                let rotated = vec2(l.x * c - l.y * s, l.x * s + l.y * c);
                layer.center + rotated
            });

            let uvs = [
                Pos2::new(0.0, 0.0),
                Pos2::new(1.0, 0.0),
                Pos2::new(1.0, 1.0),
                Pos2::new(0.0, 1.0),
            ];
            let mut mesh = Mesh::with_texture(texture.id());
            for k in 0..4 {
                mesh.vertices.push(Vertex {
                    pos: to_screen(corners[k]),
                    uv: uvs[k],
                    color: pal::NO_TINT,
                });
            }
            mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
            painter.add(Shape::mesh(mesh));
            painted = true;
        }
    }

    if !painted {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "connect layers",
            FontId::proportional(font::LABEL),
            pal::MUTED,
        );
    }

    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.0, pal::NODE_STROKE),
        egui::StrokeKind::Inside,
    );
}

/// Small checkered image preview.
pub(crate) fn preview(ui: &mut egui::Ui, texture: Option<&egui::TextureHandle>, size: Vec2) {
    let (rect, painter) = preview_background(ui, size);

    match texture {
        Some(texture) => paint_preview_texture(&painter, rect, texture),
        None => {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "no preview",
                FontId::proportional(font::LABEL),
                pal::MUTED,
            );
        }
    }

    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0, pal::NODE_STROKE),
        egui::StrokeKind::Inside,
    );
}

pub(crate) fn preview_background(ui: &mut egui::Ui, size: Vec2) -> (egui::Rect, egui::Painter) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(4), pal::CHECK_A);

    let cell = 8.0;
    let mut y = rect.top();
    let mut row = 0;
    while y < rect.bottom() {
        let mut x = rect.left() + if row % 2 == 0 { 0.0 } else { cell };
        while x < rect.right() {
            let tile = egui::Rect::from_min_max(
                pos2(x, y),
                pos2((x + cell).min(rect.right()), (y + cell).min(rect.bottom())),
            );
            painter.rect_filled(tile, CornerRadius::ZERO, pal::CHECK_B);
            x += cell * 2.0;
        }
        y += cell;
        row += 1;
    }
    (rect, painter)
}

fn paint_preview_texture(painter: &egui::Painter, rect: egui::Rect, texture: &egui::TextureHandle) {
    let source = texture.size_vec2();
    let scale = (rect.width() / source.x)
        .min(rect.height() / source.y)
        .min(1.0);
    let dst = egui::Rect::from_center_size(rect.center(), source * scale);
    painter.image(
        texture.id(),
        dst,
        egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        pal::NO_TINT,
    );
}

pub(crate) use crate::style::controls::error_line;

fn duration_text(duration: std::time::Duration) -> String {
    if duration.as_secs_f64() < 1.0 {
        format!("{:.0} ms", duration.as_secs_f64() * 1000.0)
    } else {
        format!("{:.2} s", duration.as_secs_f64())
    }
}

/// Compact benchmark for the node's most recent successful run.
pub(crate) fn timing_line(ui: &mut egui::Ui, timing: Option<RunTiming>) {
    let Some(timing) = timing else {
        return;
    };
    let cache = if timing.cache_models {
        "model cache on"
    } else {
        "model cache off"
    };
    ui.add(
        egui::Label::new(muted(
            format!(
                "Last run {} · load {}\nInference {} · {}",
                duration_text(timing.total),
                duration_text(timing.load),
                duration_text(timing.infer),
                cache,
            ),
            font::TINY,
        ))
        .selectable(false),
    );
}

pub(crate) fn socket(ui: &mut egui::Ui, label: &str) -> PinInfo {
    if !label.is_empty() {
        ui.add(egui::Label::new(muted(label, font::LABEL)).selectable(false));
    }
    PinInfo::circle()
        .with_fill(pal::SOCKET)
        .with_stroke(Stroke::new(1.0, pal::SOCKET_STROKE))
        .with_wire_color(pal::SOCKET)
}
