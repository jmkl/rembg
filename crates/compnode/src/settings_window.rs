//! The Settings window.
//!
//! The window edits a draft `Settings`; the app applies and persists it only
//! when the user presses Apply.

use std::path::{Path, PathBuf};

use egui::{Align, Align2, Layout, RichText};
use rembg_core::{MODEL_BIREFNET_FILE, MODEL_GFPGAN_FILE, MODEL_SCRFD_FILE};

use crate::settings::{self, Settings};
use crate::style::controls::{Button, Checkbox, danger_text, muted, section_title, window_frame};
use crate::style::font;

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

    egui::Window::new("Settings")
        .id(egui::Id::new("compnode_settings"))
        .open(&mut window_open)
        .collapsible(false)
        .resizable(false)
        .default_width(540.0)
        .anchor(Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .frame(window_frame())
        .show(ctx, |ui| {
            section(ui, "Inference");
            ui.add(Checkbox::new(
                &mut settings.cache_models,
                "Keep inference models in memory between runs",
            ));
            ui.label(hint(
                "When enabled, models load on first use and stay resident. When disabled, each run loads them and releases them afterwards.",
            ));

            ui.add_space(10.0);
            model_dir_row(ui, settings);
            ui.add_space(10.0);

            section(ui, "Canvas");
            canvas_size_row(ui, settings);
            ui.label(hint(
                "Canvas dimensions for the image viewer. Width and height in pixels.",
            ));

            section(ui, "Workspace");
            workspace_root_row(ui, settings);
            ui.label(hint(
                "Projects, exported images, masks and other workspace data are stored under this folder.",
            ));

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(6.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(Button::new("Apply").min_width(72.0)).clicked()
                {
                    action = SettingsAction::Apply;
                }
                if ui
                    .add(Button::new("Cancel").outline().min_width(72.0))
                    .clicked()
                {
                    action = SettingsAction::Cancel;
                }
            });
        });

    *open = window_open;
    action
}

fn hint(text: impl Into<String>) -> RichText {
    muted(text, font::SMALL)
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(section_title(title));
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
    ui.label(hint(format!(
        "Expected: {}, {}, {}",
        MODEL_BIREFNET_FILE, MODEL_GFPGAN_FILE, MODEL_SCRFD_FILE
    )));
    if dir.is_dir() {
        ui.label(hint(format!("Using {}", dir.display())));
    } else {
        ui.label(danger_text(format!(
            "Folder not found: {}. Inference will report the missing model.",
            dir.display()
        )));
    }
}

fn canvas_size_row(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.label("Canvas size");
    let mut w = settings.canvas_width;
    let mut h = settings.canvas_height;
    ui.horizontal(|ui| {
        ui.add(egui::Label::new("W"));
        let w_resp = ui.add(egui::DragValue::new(&mut w).range(64.0..=8192.0).fixed_decimals(0));
        ui.add(egui::Label::new("H"));
        let h_resp = ui.add(egui::DragValue::new(&mut h).range(64.0..=8192.0).fixed_decimals(0));
        if w_resp.changed() || h_resp.changed() {
            settings.canvas_width = w;
            settings.canvas_height = h;
        }
    });
    ui.add_space(10.0);
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
        ui.label(danger_text(format!("Not a folder: {}", root.display())));
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
