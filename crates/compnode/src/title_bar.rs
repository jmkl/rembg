//! Custom undecorated window title bar and app menus.

use egui::{Color32, Frame};
use elegance::{BrandLogo, Button, ButtonSize, MenuBar, MenuItem, glyphs};

use crate::style::pal;

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

pub(crate) fn show(
    ui: &mut egui::Ui,
    status: &str,
    status_color: Color32,
    selected: usize,
    maximized: bool,
    fullscreen: bool,
) -> (Actions, f32) {
    let mut actions = Actions::default();

    let panel_response = egui::Panel::top("titlebar")
        .frame(Frame::new().fill(pal::PANEL))
        .show(ui, |ui| {
            MenuBar::new("compnode_menubar")
                .brand("CompNode")
                .brand_logo(BrandLogo::Glyph(glyphs::NETWORK.to_string()))
                .status_with_dot(status, status_color)
                .title_bar(true)
                .show(ui, |bar| {
                    bar.menu("File", |ui| {
                        if ui
                            .add(
                                MenuItem::new("New graph")
                                    .shortcut("Ctrl+N")
                                    .icon(glyphs::PLUS.to_string()),
                            )
                            .clicked()
                        {
                            actions.new_graph = true;
                            ui.close();
                        }
                        if ui
                            .add(
                                MenuItem::new("Open…")
                                    .shortcut("Ctrl+O")
                                    .icon(glyphs::FOLDER_OPEN.to_string()),
                            )
                            .clicked()
                        {
                            actions.open = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .add(
                                MenuItem::new("Save")
                                    .shortcut("Ctrl+S")
                                    .icon(glyphs::SAVE.to_string()),
                            )
                            .clicked()
                        {
                            actions.save = true;
                            ui.close();
                        }
                        if ui
                            .add(
                                MenuItem::new("Save As…")
                                    .shortcut("Ctrl+Shift+S")
                                    .icon(glyphs::COPY.to_string()),
                            )
                            .clicked()
                        {
                            actions.save_as = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .add(MenuItem::new("Add images…").icon(glyphs::PLUS.to_string()))
                            .clicked()
                        {
                            actions.add_images = true;
                            ui.close();
                        }
                        if ui
                            .add(
                                MenuItem::new("Add image from URL…")
                                    .icon(glyphs::NETWORK.to_string()),
                            )
                            .clicked()
                        {
                            actions.add_url = true;
                            ui.close();
                        }
                        if ui
                            .add(
                                MenuItem::new("Paste clipboard image")
                                    .shortcut("Ctrl+V")
                                    .icon(glyphs::COPY.to_string()),
                            )
                            .clicked()
                        {
                            actions.paste_clipboard = true;
                            ui.close();
                        }
                        if ui
                            .add(
                                MenuItem::new("Insert from input folder…")
                                    .icon(glyphs::FOLDER_OPEN.to_string()),
                            )
                            .clicked()
                        {
                            actions.insert_input = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .add(
                                MenuItem::new("Quit")
                                    .shortcut("Alt+F4")
                                    .icon(glyphs::X.to_string())
                                    .danger(),
                            )
                            .clicked()
                        {
                            actions.quit = true;
                            ui.close();
                        }
                    });

                    bar.menu("Edit", |ui| {
                        if ui
                            .add(
                                MenuItem::new("Delete selected")
                                    .shortcut("Del")
                                    .icon(glyphs::TRASH.to_string())
                                    .enabled(selected > 0),
                            )
                            .clicked()
                        {
                            actions.delete_selected = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .add(
                                MenuItem::new("Settings…")
                                    .shortcut("Ctrl+,")
                                    .icon(glyphs::SETTINGS.to_string()),
                            )
                            .clicked()
                        {
                            actions.settings = true;
                            ui.close();
                        }
                    });

                    bar.menu("View", |ui| {
                        if ui
                            .add(
                                MenuItem::new("Fullscreen")
                                    .shortcut("F11")
                                    .checked(fullscreen),
                            )
                            .clicked()
                        {
                            actions.toggle_fullscreen = true;
                            ui.close();
                        }
                    });

                    bar.menu("Help", |ui| {
                        if ui
                            .add(MenuItem::new("About CompNode").icon(glyphs::INFO.to_string()))
                            .clicked()
                        {
                            actions.about = true;
                            ui.close();
                        }
                    });

                    bar.trailing(|ui| {
                        if ui
                            .add(
                                Button::icon(glyphs::X, "Close")
                                    .size(ButtonSize::Small)
                                    .outline(),
                            )
                            .clicked()
                        {
                            actions.quit = true;
                        }
                        let maximize_glyph = if maximized { '\u{2750}' } else { '\u{25A1}' };
                        if ui
                            .add(
                                Button::icon(maximize_glyph, "Maximize")
                                    .size(ButtonSize::Small)
                                    .outline(),
                            )
                            .clicked()
                        {
                            actions.toggle_maximize = true;
                        }
                        if ui
                            .add(
                                Button::icon('\u{2013}', "Minimize")
                                    .size(ButtonSize::Small)
                                    .outline(),
                            )
                            .clicked()
                        {
                            actions.minimize = true;
                        }
                    });
                });
        });

    let title_bar_height = panel_response.response.rect.height();
    (actions, title_bar_height)
}
