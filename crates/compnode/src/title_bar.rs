//! Custom undecorated window title bar and app menus.
//!
//! Built from vanilla egui: a top panel holding the brand, an `egui::MenuBar`
//! and the window controls. The empty part of the bar drags / maximises the
//! window.

use egui::{
    Align, Color32, Layout, PointerButton, Rect, RichText, Sense, UiBuilder, ViewportCommand, vec2,
};

use crate::style::controls::{MenuItem, WindowGlyph, muted, titlebar_frame, window_button};
use crate::style::{font, icons, metrics};

/// Width reserved for the File / Edit / View / Help strip.
const MENUS_WIDTH: f32 = 190.0;

/// Actions requested by the title bar in one frame.
#[derive(Default)]
pub(crate) struct Actions {
    pub new_graph: bool,
    pub open: bool,
    pub save: bool,
    pub save_as: bool,
    pub add_images: bool,
    pub add_url: bool,
    pub insert_input: bool,
    pub paste_clipboard: bool,
    pub delete_selected: bool,
    pub settings: bool,
    pub toggle_fullscreen: bool,
    pub about: bool,
    pub quit: bool,
    pub minimize: bool,
    pub toggle_maximize: bool,
}

/// One clickable menu row; closes the menu and returns whether it was clicked.
fn item(ui: &mut egui::Ui, item: MenuItem) -> bool {
    let clicked = ui.add(item).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    status: &str,
    status_color: Color32,
    selected: usize,
    maximized: bool,
    fullscreen: bool,
) -> (Actions, f32) {
    let mut actions = Actions::default();

    let panel = egui::Panel::top("titlebar")
        .frame(titlebar_frame())
        .exact_size(metrics::TITLEBAR_H)
        .resizable(false)
        .show(ui, |ui| {
            let bar = ui.max_rect();
            // Registered first so every widget below sits on top of it: only
            // the empty strip moves the window.
            let drag = ui.interact(bar, ui.id().with("drag"), Sense::click_and_drag());

            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(bar)
                    .layout(Layout::left_to_right(Align::Center)),
                |ui| {
                    brand(ui);
                    ui.add_space(6.0);
                    let strip =
                        Rect::from_min_size(ui.cursor().min, vec2(MENUS_WIDTH, bar.height()));
                    ui.scope_builder(UiBuilder::new().max_rect(strip), |ui| {
                        egui::MenuBar::new()
                            .ui(ui, |ui| menus(ui, &mut actions, selected, fullscreen));
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if window_button(ui, WindowGlyph::Close, "Close").clicked() {
                            actions.quit = true;
                        }
                        let (glyph, tip) = if maximized {
                            (WindowGlyph::Restore, "Restore")
                        } else {
                            (WindowGlyph::Maximize, "Maximize")
                        };
                        if window_button(ui, glyph, tip).clicked() {
                            actions.toggle_maximize = true;
                        }
                        if window_button(ui, WindowGlyph::Minimize, "Minimize").clicked() {
                            actions.minimize = true;
                        }
                        ui.add_space(10.0);
                        ui.label(muted(status, font::LABEL));
                        status_dot(ui, status_color);
                    });
                },
            );

            if drag.drag_started_by(PointerButton::Primary) {
                ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
            }
            if drag.double_clicked_by(PointerButton::Primary) {
                actions.toggle_maximize = true;
            }
        });

    (actions, panel.response.rect.height())
}

fn brand(ui: &mut egui::Ui) {
    ui.label(icons::rich(icons::NETWORK, 15.0));
    ui.label(RichText::new("CompNode").strong().size(font::BODY));
}

fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

fn menus(ui: &mut egui::Ui, actions: &mut Actions, selected: usize, fullscreen: bool) {
    ui.menu_button("File", |ui| {
        if item(
            ui,
            MenuItem::new("New graph")
                .shortcut("Ctrl+N")
                .icon(icons::PLUS),
        ) {
            actions.new_graph = true;
        }
        if item(
            ui,
            MenuItem::new("Open…")
                .shortcut("Ctrl+O")
                .icon(icons::FOLDER_OPEN),
        ) {
            actions.open = true;
        }
        ui.separator();
        if item(
            ui,
            MenuItem::new("Save").shortcut("Ctrl+S").icon(icons::SAVE),
        ) {
            actions.save = true;
        }
        if item(
            ui,
            MenuItem::new("Save As…")
                .shortcut("Ctrl+Shift+S")
                .icon(icons::COPY),
        ) {
            actions.save_as = true;
        }
        ui.separator();
        if item(ui, MenuItem::new("Add images…").icon(icons::PLUS)) {
            actions.add_images = true;
        }
        if item(
            ui,
            MenuItem::new("Add image from URL…").icon(icons::NETWORK),
        ) {
            actions.add_url = true;
        }
        if item(
            ui,
            MenuItem::new("Paste clipboard image")
                .shortcut("Ctrl+V")
                .icon(icons::COPY),
        ) {
            actions.paste_clipboard = true;
        }
        if item(
            ui,
            MenuItem::new("Insert from input folder…").icon(icons::FOLDER_OPEN),
        ) {
            actions.insert_input = true;
        }
        ui.separator();
        if item(
            ui,
            MenuItem::new("Quit")
                .shortcut("Alt+F4")
                .icon(icons::X)
                .danger(),
        ) {
            actions.quit = true;
        }
    });

    ui.menu_button("Edit", |ui| {
        if item(
            ui,
            MenuItem::new("Delete selected")
                .shortcut("Del")
                .icon(icons::TRASH)
                .enabled(selected > 0),
        ) {
            actions.delete_selected = true;
        }
        ui.separator();
        if item(
            ui,
            MenuItem::new("Settings…")
                .shortcut("Ctrl+,")
                .icon(icons::SETTINGS),
        ) {
            actions.settings = true;
        }
    });

    ui.menu_button("View", |ui| {
        if item(
            ui,
            MenuItem::new("Fullscreen")
                .shortcut("F11")
                .checked(fullscreen),
        ) {
            actions.toggle_fullscreen = true;
        }
    });

    ui.menu_button("Help", |ui| {
        if item(ui, MenuItem::new("About CompNode").icon(icons::INFO)) {
            actions.about = true;
        }
    });
}
