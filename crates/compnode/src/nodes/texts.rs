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
use crate::style::controls::{Checkbox, Select, Slider};
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
const DEFAULT_COLORS: [Color32; 5] = [
    Color32::WHITE,
    Color32::BLACK,
    Color32::RED,
    Color32::GREEN,
    Color32::YELLOW,
];
const DEFAULT_TEXT_COLORS: [Color32; 3] = [Color32::WHITE, Color32::RED, Color32::YELLOW];

fn param_color(ui: &mut Ui, params: &mut Params, key: &str, label: &str, default: Color32) -> bool {
    let mut color = hex_to_color(params::string(params, key, ""), default);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label)
                .size(font::SMALL)
                .color(pal::TEXT),
        );
        for &preset in &DEFAULT_COLORS {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());

            if ui.is_rect_visible(rect) {
                ui.painter().rect_filled(rect, 3.0, preset);

                if color == preset {
                    ui.painter().rect_stroke(
                        rect.expand(1.0),
                        3.0,
                        egui::Stroke::new(2.0, ui.visuals().text_color()),
                        egui::StrokeKind::Outside,
                    );
                }
            }

            if response.clicked() {
                color = preset;
                changed = true;
            }

            response.on_hover_text(format!(
                "#{:02X}{:02X}{:02X}",
                preset.r(),
                preset.g(),
                preset.b()
            ));
        }
        changed |= egui::color_picker::color_edit_button_srgba(
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
        let mut font_id = params::string(params, "font", "unisans").to_owned();
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

        // Master switch for the decorative stack (outlines, 3D depth, shadow).
        let mut effects = params::bool(params, "effects", true);
        if ui
            .add(Checkbox::new(&mut effects, "Shadow, outline & 3D"))
            .changed()
        {
            params::set_bool(params, "effects", effects);
            changed = true;
        }

        // Horizontal alignment of the stacked rows: left or centred.
        let mut centered = params::string(params, "align", "left") == "center";
        if ui
            .add(Checkbox::new(&mut centered, "Center text"))
            .changed()
        {
            params::set_string(params, "align", if centered { "center" } else { "left" });
            changed = true;
        }

        // Case handling for every rendered row.
        let mut uppercase = params::bool(params, "uppercase", true);
        if ui.add(Checkbox::new(&mut uppercase, "Uppercase")).changed() {
            params::set_bool(params, "uppercase", uppercase);
            changed = true;
        }

        // Backdrop rectangle painted underneath the text block.
        egui::CollapsingHeader::new("Box")
            .default_open(false)
            .show(ui, |ui| {
                let mut box_on = params::bool(params, "box", false);
                if ui.add(Checkbox::new(&mut box_on, "Enable box")).changed() {
                    params::set_bool(params, "box", box_on);
                    changed = true;
                }
                ui.add_enabled_ui(box_on, |ui| {
                    changed |= param_color(ui, params, "box_color", "Color", Color32::BLACK);
                    changed |= param_slider(
                        ui,
                        params,
                        "box_padding_x",
                        "Padding X",
                        0.0..=400.0,
                        24.0,
                        " px",
                        1.0,
                        0,
                    );
                    changed |= param_slider(
                        ui,
                        params,
                        "box_padding_y",
                        "Padding Y",
                        0.0..=400.0,
                        12.0,
                        " px",
                        1.0,
                        0,
                    );
                });
            });

        egui::CollapsingHeader::new("Letter Gap & Spacing")
            .default_open(false)
            .show(ui, |ui| {
                changed |= param_slider(
                    ui,
                    params,
                    "line_gap",
                    "Line gap",
                    -100.0..=200.0,
                    -16.0,
                    " px",
                    1.0,
                    0,
                );
                changed |= param_slider(
                    ui,
                    params,
                    "letter_spacing",
                    "Letter spacing",
                    -50.0..=200.0,
                    -2.0,
                    " px",
                    1.0,
                    0,
                );
            });

        // Every decorative section below is gated by the master switch.
        ui.add_enabled_ui(effects, |ui| {
            // Outlines: the wider one is always drawn outermost.
            egui::CollapsingHeader::new("Outline")
                .default_open(false)
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
                        "Outline#1 color",
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
                        "Outline#2 color",
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
        });
        ui.add_space(6.0);

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
        let case_text = match uppercase {
            true => text.to_uppercase(),
            false => text,
        };
        let lines = texts::split_lines(&case_text);
        let mut styles = texts::read_styles(params);
        let mut rows_changed = false;
        egui::Grid::new("text_line_styles")
            .num_columns(3)
            .spacing(vec2(8.0, 4.0))
            .show(ui, |ui| {
                for (index, line) in lines.iter().enumerate() {
                    while styles.len() <= index {
                        styles.push(LineStyle::default());
                    }

                    ui.push_id(index, |ui| {
                        ui.add_sized(
                            vec2(ui.available_width(), 18.0),
                            egui::Label::new(
                                egui::RichText::new(*line)
                                    .size(font::SMALL)
                                    .color(pal::TEXT),
                            )
                            .truncate(),
                        );
                    });

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
                    for &preset in &DEFAULT_TEXT_COLORS {
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());

                        if ui.is_rect_visible(rect) {
                            ui.painter().rect_filled(rect, 3.0, preset);

                            if color == preset {
                                ui.painter().rect_stroke(
                                    rect.expand(1.0),
                                    3.0,
                                    egui::Stroke::new(2.0, ui.visuals().text_color()),
                                    egui::StrokeKind::Outside,
                                );
                            }
                        }

                        if response.clicked() {
                            color = preset;
                            style.color = [color.r(), color.g(), color.b()];
                            rows_changed = true;
                        }

                        response.on_hover_text(format!(
                            "#{:02X}{:02X}{:02X}",
                            preset.r(),
                            preset.g(),
                            preset.b()
                        ));
                    }
                    if egui::color_picker::color_edit_button_srgba(
                        ui,
                        &mut color,
                        egui::color_picker::Alpha::Opaque,
                    )
                    .changed()
                    {
                        style.color = [color.r(), color.g(), color.b()];
                        rows_changed = true;
                    }

                    ui.end_row();
                }
            });
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
