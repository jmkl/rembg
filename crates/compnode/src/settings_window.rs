//! The Settings window.
//!
//! The window edits a draft `Settings`; the app applies and persists it only
//! when the user presses Apply.

use std::path::{Path, PathBuf};

use egui::{Align, Align2, Color32, CornerRadius, Frame, Layout, Margin, RichText, Stroke};
use rembg_core::{MODEL_BIREFNET_FILE, MODEL_GFPGAN_FILE, MODEL_SCRFD_FILE};

use crate::settings::{self, Settings};
use crate::style::pal;

pub(crate) enum SettingsAction {
    None,
    Apply,
    Cancel,
}

pub(crate) fn show(
    ctx: &egui::Context,
    open: &mut bool,
    settings: &mut Settings,
) -> SettingsAction {
    if !*open {
        return SettingsAction::None;
    }

    let mut window_open = *open;
    let mut action = SettingsAction::None;
    let frame = Frame::new()
        .fill(pal::PANEL)
        .stroke(Stroke::new(1.0, pal::NODE_STROKE))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(14));

    egui::Window::new("Settings")
        .id(egui::Id::new("compnode_settings"))
        .open(&mut window_open)
        .collapsible(false)
        .resizable(false)
        .default_width(540.0)
        .anchor(Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .frame(frame)
        .show(ctx, |ui| {
            section(ui, "Inference");
            ui.checkbox(
                &mut settings.cache_models,
                "Keep inference models in memory between runs",
            );
            ui.label(
                RichText::new(
                    "When enabled, models load on first use and stay resident. When disabled, each run loads them and releases them afterwards.",
                )
                .small()
                .color(pal::MUTED),
            );

            ui.add_space(10.0);
            model_dir_row(ui, settings);
            ui.add_space(10.0);

            section(ui, "Workspace");
            workspace_root_row(ui, settings);
            ui.label(
                RichText::new(
                    "Projects, exported images, masks and other workspace data are stored under this folder.",
                )
                .small()
                .color(pal::MUTED),
            );

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(6.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let apply = egui::Button::new(RichText::new("Apply").color(Color32::WHITE))
                    .fill(pal::ACCENT)
                    .min_size(egui::vec2(72.0, 24.0));
                if ui.add(apply).clicked() {
                    action = SettingsAction::Apply;
                }
                if ui
                    .add(
                        egui::Button::new("Cancel")
                            .fill(pal::NODE)
                            .min_size(egui::vec2(72.0, 24.0)),
                    )
                    .clicked()
                {
                    action = SettingsAction::Cancel;
                }
            });
        });

    *open = window_open;
    action
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).strong().size(13.0).color(pal::TEXT));
    ui.add_space(4.0);
}

fn model_dir_row(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.label("Model directory");
    let mut value = settings.model_dir.clone().unwrap_or_default();
    let changed = path_row(
        ui,
        "Default: ./models",
        &mut value,
        settings::default_model_dir(),
    );
    if changed {
        settings.model_dir = if value.as_os_str().is_empty() {
            None
        } else {
            Some(value)
        };
    }

    let dir = settings.model_dir();
    ui.label(
        RichText::new(format!(
            "Expected: {}, {}, {}",
            MODEL_BIREFNET_FILE, MODEL_GFPGAN_FILE, MODEL_SCRFD_FILE
        ))
        .small()
        .color(pal::MUTED),
    );
    if dir.is_dir() {
        ui.label(
            RichText::new(format!("Using {}", dir.display()))
                .small()
                .color(pal::MUTED),
        );
    } else {
        ui.label(
            RichText::new(format!(
                "Folder not found: {}. Inference will report the missing model.",
                dir.display()
            ))
            .small()
            .color(pal::DANGER),
        );
    }
}

fn workspace_root_row(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.label("Workspace root");
    let mut value = settings.workspace_root.clone();
    let changed = path_row(
        ui,
        "Default: ./workspaces",
        &mut value,
        settings::default_workspace_root(),
    );
    if changed {
        settings.workspace_root = value;
    }

    let root = settings.workspace_root();
    if root.exists() && !root.is_dir() {
        ui.label(
            RichText::new(format!("Not a folder: {}", root.display()))
                .small()
                .color(pal::DANGER),
        );
    }
}

fn path_row(ui: &mut egui::Ui, hint: &str, value: &mut PathBuf, default_dir: PathBuf) -> bool {
    let mut text = value.display().to_string();
    let mut changed = false;

    ui.horizontal(|ui| {
        let button_width = 82.0;
        let edit_width = (ui.available_width() - button_width - 6.0).max(160.0);
        let response = ui.add_sized(
            [edit_width, 22.0],
            egui::TextEdit::singleline(&mut text).hint_text(hint),
        );
        if response.changed() {
            changed = true;
        }
        if ui.button("Browse…").clicked() {
            let start = if text.trim().is_empty() {
                default_dir.clone()
            } else {
                PathBuf::from(text.trim())
            };
            if let Some(folder) = pick_folder(start) {
                text = folder.display().to_string();
                changed = true;
            }
        }
    });

    if changed {
        *value = PathBuf::from(text.trim());
    }
    changed
}

fn pick_folder(start: PathBuf) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new();
    let start = if start.is_dir() {
        start
    } else {
        start
            .parent()
            .filter(|parent| parent.is_dir())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    dialog = dialog.set_directory(start);
    dialog.pick_folder()
}
