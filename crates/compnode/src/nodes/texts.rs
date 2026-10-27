//! Texts generator node body: font family dropdown, a multiline editor whose
//! `\n`-separated lines become stacked rows, each row with its own size and
//! colour.

use egui::{Color32, Ui, Vec2, vec2};
use egui_snarl::{NodeId, Snarl};

use crate::editor::GraphEditor;
use crate::graph::invalidate_node;
use crate::model::Node;
use crate::ops::texts::{self, LineStyle};
use crate::params;
use crate::style::controls::{Select, Slider};
use crate::style::{font, pal};
use crate::widgets::{error_line, preview};

use crate::params::Params; // use whatever path your `Params` type actually lives at

fn hex_to_color(s: &str, default: Color32) -> Color32 {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return default;
    }
    let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16);
    match (byte(0), byte(2), byte(4)) {
        (Ok(r), Ok(g), Ok(b)) => Color32::from_rgb(r, g, b),
        _ => default,
    }
}

fn color_to_hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

/// One slider bound to an f32 param. Returns true when the value changed.
fn param_slider(
    ui: &mut Ui,
    params: &mut Params,
    key: &str,
    label: &str,
    range: std::ops::RangeInclusive<f32>,
    default: f32,
    suffix: &str,
    step: f64,
    decimals: usize,
) -> bool {
    let mut value = params::f32(params, key, default);
    let changed = ui
        .add(
            Slider::new(&mut value, range)
                .label(label)
                .suffix(suffix)
                .step(step)
                .decimals(decimals)
                .default(default),
        )
        .changed();
    if changed {
        params::set_f32(params, key, value);
    }
    changed
}

/// One color swatch bound to a "#rrggbb" string param.
fn param_color(ui: &mut Ui, params: &mut Params, key: &str, label: &str, default: Color32) -> bool {
    let mut color = hex_to_color(params::string(params, key, ""), default);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label)
                .size(font::SMALL)
                .color(pal::TEXT),
        );
        changed = egui::color_picker::color_edit_button_srgba(
            ui,
            &mut color,
            egui::color_picker::Alpha::Opaque,
        )
        .changed();
    });
    if changed {
        params::set_string(params, key, color_to_hex(color));
    }
    changed
}

pub(crate) fn show_body(
    _editor: &mut GraphEditor,
    node: NodeId,
    ui: &mut Ui,
    snarl: &mut Snarl<Node>,
    width: f32,
) {
    let (texture, preview_enabled, error) = {
        let Some(filter) = snarl[node].filter() else {
            return;
        };
        (
            filter.preview.texture.clone(),
            filter.preview_enabled,
            filter.preview.error.clone(),
        )
    };

    if preview_enabled {
        preview(ui, texture.as_ref(), Vec2::new(width, 140.0));
    }
    error_line(ui, error.as_deref());

    let mut changed = false;
    {
        let Some(filter) = snarl[node].filter_mut() else {
            return;
        };
        let params = &mut filter.params;

        // Font family (hardcoded list until custom font loading lands).
        let mut font_id = params::string(params, "font", "inter").to_owned();
        if ui
            .add(
                Select::new("texts-font", &mut font_id)
                    .label("Font family")
                    .options(texts::families().map(|(id, name)| (id.to_owned(), name))),
            )
            .changed()
        {
            params::set_string(params, "font", font_id);
            changed = true;
        }

        // Vertical gap between stacked rows (may be negative: rows overlap).
        let mut line_gap = params::f32(params, "line_gap", 0.0);
        if ui
            .add(
                Slider::new(&mut line_gap, -100.0..=200.0)
                    .label("Line gap")
                    .suffix(" px")
                    .step(1.0)
                    .decimals(0)
                    .default(0.0),
            )
            .changed()
        {
            params::set_f32(params, "line_gap", line_gap);
            changed = true;
        }

        // Extra space between characters (negative pulls them together).
        let mut letter_spacing = params::f32(params, "letter_spacing", 0.0);
        if ui
            .add(
                Slider::new(&mut letter_spacing, -50.0..=200.0)
                    .label("Letter spacing")
                    .suffix(" px")
                    .step(1.0)
                    .decimals(0)
                    .default(0.0),
            )
            .changed()
        {
            params::set_f32(params, "letter_spacing", letter_spacing);
            changed = true;
        }

        // Outlines: the wider one is always drawn outermost.
        egui::CollapsingHeader::new("Outline")
            .default_open(true)
            .show(ui, |ui| {
                changed |= param_slider(
                    ui,
                    params,
                    "outline_width",
                    "Outline 1 width",
                    0.0..=60.0,
                    0.0,
                    " px",
                    1.0,
                    0,
                );
                changed |= param_color(
                    ui,
                    params,
                    "outline_color",
                    "Outline 1 color",
                    Color32::BLACK,
                );
                changed |= param_slider(
                    ui,
                    params,
                    "outline2_width",
                    "Outline 2 width",
                    0.0..=60.0,
                    0.0,
                    " px",
                    1.0,
                    0,
                );
                changed |= param_color(
                    ui,
                    params,
                    "outline2_color",
                    "Outline 2 color",
                    Color32::WHITE,
                );
            });

        // Fake 3D: the outlined shape stamped `depth` times down-right.
        egui::CollapsingHeader::new("3D depth")
            .default_open(false)
            .show(ui, |ui| {
                changed |=
                    param_slider(ui, params, "depth", "Depth", 0.0..=30.0, 0.0, " px", 1.0, 0);
                changed |= param_color(ui, params, "depth_color", "Side color", Color32::BLACK);
                changed |= param_slider(
                    ui,
                    params,
                    "depth_x",
                    "Direction X",
                    -4.0..=4.0,
                    1.0,
                    "",
                    1.0,
                    0,
                );
                changed |= param_slider(
                    ui,
                    params,
                    "depth_y",
                    "Direction Y",
                    -4.0..=4.0,
                    1.0,
                    "",
                    1.0,
                    0,
                );
            });

        // Soft drop shadow under the whole stack. Opacity 0 turns it off.
        egui::CollapsingHeader::new("Shadow")
            .default_open(false)
            .show(ui, |ui| {
                changed |= param_slider(
                    ui,
                    params,
                    "shadow_opacity",
                    "Opacity",
                    0.0..=1.0,
                    0.0,
                    "",
                    0.05,
                    2,
                );
                changed |= param_color(ui, params, "shadow_color", "Color", Color32::BLACK);
                changed |= param_slider(
                    ui,
                    params,
                    "shadow_x",
                    "Offset X",
                    -100.0..=100.0,
                    0.0,
                    " px",
                    1.0,
                    0,
                );
                changed |= param_slider(
                    ui,
                    params,
                    "shadow_y",
                    "Offset Y",
                    -100.0..=100.0,
                    8.0,
                    " px",
                    1.0,
                    0,
                );
                changed |= param_slider(
                    ui,
                    params,
                    "shadow_blur",
                    "Blur",
                    0.0..=40.0,
                    6.0,
                    " px",
                    1.0,
                    0,
                );
            });
        ui.add_space(6.0);

        //outline_width
        //outline_color
        //outline2_width
        //outline2_color
        //shadow_opacity
        //shadow_color
        //shadow_x
        //shadow_y
        //shadow_blur
        //depth

        // Multiline content; every non-empty line becomes one stacked row.
        let mut text = params::string(params, "text", "").to_owned();
        if ui
            .add(
                egui::TextEdit::multiline(&mut text)
                    .desired_width(ui.available_width())
                    .desired_rows(4),
            )
            .changed()
        {
            params::set_string(params, "text", text.clone());
            changed = true;
        }

        // One row per parsed line, each with its own size and colour.
        ui.add_space(6.0);
        let lines = texts::split_lines(&text);
        let mut styles = texts::read_styles(params);
        let mut rows_changed = false;
        for (index, line) in lines.iter().enumerate() {
            while styles.len() <= index {
                styles.push(LineStyle::default());
            }
            let row_width = ui.available_width();
            let label_width = (row_width - 96.0).max(24.0);
            ui.push_id(index, |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        vec2(label_width, 18.0),
                        egui::Label::new(
                            egui::RichText::new(*line)
                                .size(font::SMALL)
                                .color(pal::TEXT),
                        )
                        .truncate(),
                    );
                    let style = &mut styles[index];
                    let mut size = style.size;
                    if ui
                        .add(
                            egui::DragValue::new(&mut size)
                                .range(8.0..=512.0)
                                .fixed_decimals(0)
                                .suffix(" px"),
                        )
                        .changed()
                    {
                        style.size = size;
                        rows_changed = true;
                    }
                    let mut color =
                        Color32::from_rgb(style.color[0], style.color[1], style.color[2]);
                    let swatch = egui::color_picker::color_edit_button_srgba(
                        ui,
                        &mut color,
                        egui::color_picker::Alpha::Opaque,
                    );
                    if swatch.changed() {
                        style.color = [color.r(), color.g(), color.b()];
                        rows_changed = true;
                    }
                });
            });
        }
        if rows_changed {
            styles.truncate(lines.len());
            texts::write_styles(params, &styles);
            changed = true;
        }
    }
    if changed {
        invalidate_node(snarl, node);
    }
}
