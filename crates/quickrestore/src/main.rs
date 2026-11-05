// #![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod anim_helper;
mod config;
mod consts;
mod gpt;
mod model;
mod platform;
mod server;
mod tray;
mod widget;
use consts::*;
use eframe::egui::{
    self, Button, CentralPanel, Color32, CornerRadius, FontData, Frame, Id, Panel, Pos2, Sense,
    Stroke, StrokeKind, TextEdit, ViewportCommand, Visuals, vec2,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use wry::{
    Rect as WvRect, WebView, WebViewBuilder,
    dpi::{LogicalPosition, LogicalSize},
};

use crate::config::{BackgroundRemovalModel, Config, FaceRestoreModel};
use crate::gpt::ABORT_JS;
use crate::model::{Cmd, ProgressStatus, Status};
use crate::server::{CanvasImage, Server};
use crate::tray::{Tray, TrayAction};
use crate::widget::Btn;

/// eframe has no native frame-rate cap (`NativeOptions::max_fps` is web-only),
/// and every pointer move over the window triggers a paint.
const FRAME_BUDGET: Duration = Duration::from_micros(1_000_000 / MAX_FPS as u64);
/// Poll rate while hidden (menu clicks only become frames through polling).
const HIDDEN_POLL: Duration = Duration::from_millis(250);
/// Poll rate while a pipeline runs, so the dot/text stay live with an idle pointer.
const BUSY_POLL: Duration = Duration::from_millis(100);

const CONFIG_FILE: &str = "quickrestore.json";
const DEFAULT_MODEL_DIR: &str = "E:\\_CODE\\rust\\rembg\\models";
const START_URL: &str = "https://chatgpt.com";
const WEBVIEW_ZOOM: f64 = 0.50;
/// Vertical space reserved above the webview for the GPT buttons.
const WEBVIEW_TOP_GAP: f32 = 30.0;
const STATUS_DOT: f32 = 6.0;

enum ActionType {
    SelectionTool,
    RestoreSelection,
    CopyLayerToClipboard,
    GPTSend,
    GPTAbort,
}

struct Action {
    id: &'static str,
    icon: &'static str,
    tooltip: &'static str,
    status: &'static str,
    msg_type: &'static str,
    action_type: ActionType,
}

const ACTIONS: [Action; 3] = [
    // Action {
    //     id: "sel-tool",
    //     icon: "\u{f019f}",
    //     tooltip: "Get selection",
    //     status: "pic select tool",
    //     msg_type: "selection_tool",
    //     action_type: ActionType::SelectionTool,
    // },
    Action {
        id: "sel",
        icon: "\u{f1077}",
        tooltip: "Get selection",
        status: "Processing",
        msg_type: "get_selection",
        action_type: ActionType::RestoreSelection,
    },
    Action {
        id: "copy",
        icon: "\u{f0328}",
        tooltip: "Copy PS layer",
        status: "Requesting copy layer",
        msg_type: "copy",
        action_type: ActionType::CopyLayerToClipboard,
    },
    Action {
        id: "gpt-image",
        icon: "\u{f0b79}",
        tooltip: "Paste image to GPT",
        status: "Paste image to GPT",
        msg_type: "gpt_image",
        action_type: ActionType::GPTSend,
    },
    // Action {
    //     id: "gpt-abort",
    //     icon: "\u{f073a}",
    //     tooltip: "Abort GPT ops",
    //     status: "Abort gpt operations",
    //     msg_type: "gpt_abort",
    //     action_type: ActionType::GPTAbort,
    // },
];

fn status_color(status: &Status) -> Color32 {
    match status {
        Status::Processing => INFO,
        Status::Idle => SUCCESS,
        Status::Error => DANGER,
    }
}

const CARD_RADIUS: u8 = 8;
const CONTROL_RADIUS: u8 = 6;

/// A titled card used by the settings sheet.
fn settings_section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.label(egui::RichText::new(title).size(9.0).strong().color(MUTED));
    ui.add_space(5.0);
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        });
}

/// Compact pill toggle used instead of a stock checkbox.
fn settings_toggle(ui: &mut egui::Ui, value: &mut bool) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(38.0, 22.0), Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(response.id, *value, 0.12);
    let track = if *value {
        FG
    } else if response.hovered() {
        HOVER
    } else {
        BORDER
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(11), track);
    let x = rect.left() + 11.0 + (rect.width() - 22.0) * t;
    ui.painter()
        .circle_filled(Pos2::new(x, rect.center().y), 8.0, TEXT);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One label/control row inside a settings card. The control is pushed to the
/// right edge, so every row lines up regardless of its widget.
fn settings_row(
    ui: &mut egui::Ui,
    label: &str,
    hint: Option<&str>,
    control: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(label).size(12.0).color(TEXT));
            if let Some(hint) = hint {
                ui.label(egui::RichText::new(hint).size(9.0).color(MUTED));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
}

/// Hairline between rows of a settings card.
fn settings_divider(ui: &mut egui::Ui) {
    ui.add_space(7.0);
    let rect = ui.available_rect_before_wrap();
    ui.painter()
        .hline(rect.x_range(), rect.top(), Stroke::new(1.0, BORDER));
    ui.add_space(7.0);
}

/// Small borderless icon button with a hover highlight.
fn settings_icon_button(ui: &mut egui::Ui, icon: &str, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let (fill, stroke, fg) = if response.hovered() {
        (
            DANGER.gamma_multiply(0.16),
            DANGER.gamma_multiply(0.55),
            DANGER,
        )
    } else {
        (Color32::TRANSPARENT, Color32::TRANSPARENT, MUTED)
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(CONTROL_RADIUS),
        fill,
        Stroke::new(1.0, stroke),
        StrokeKind::Inside,
    );
    let galley = ui
        .painter()
        .layout_no_wrap(icon.to_owned(), egui::FontId::proportional(11.0), fg);
    ui.painter().galley(
        rect.center() - galley.mesh_bounds.center().to_vec2(),
        galley,
        fg,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

struct QuickRestoreApp {
    webview: WebView,
    /// Last bounds pushed to the webview; avoids a native resize every frame.
    webview_rect: Option<egui::Rect>,
    compact_mode: bool,
    cmd_tx: mpsc::SyncSender<Cmd>,
    status: ProgressStatus,
    status_rx: mpsc::Receiver<ProgressStatus>,
    tray_rx: mpsc::Receiver<TrayAction>,
    last_frame: Instant,
    /// Set while the window lives only in the notification area.
    hidden: bool,
    /// Dropping the tray removes the notification-area icon.
    _tray: Option<Tray>,
    /// One-shot: window chrome (shadow / 1 px line) already stripped.
    chrome_stripped: bool,
    prompt: String,
    gpt_rx: mpsc::Receiver<gpt::GptEvent>,
    setting_mode: bool,
    config: Config,
    prompts: Vec<String>,
    prompt_input: String,
    prompt_selected: Option<usize>,
    add_prompt: bool,
}

// ---------------------------------------------------------------- helpers --

impl QuickRestoreApp {
    fn is_busy(&self) -> bool {
        matches!(self.status.is_processing, Status::Processing)
    }

    fn send(&self, msg: serde_json::Value) {
        let _ = self.cmd_tx.try_send(Cmd::SendText(msg.to_string()));
    }

    /// Runs a generated script in the webview, logging any failure.
    fn run_js(&self, script: anyhow::Result<String>) {
        match script {
            Ok(js) => {
                if let Err(e) = self.webview.evaluate_script(&js) {
                    eprintln!("script error: {e}");
                }
            }
            Err(e) => eprintln!("script build error: {e}"),
        }
    }

    fn start_action(&mut self, action: &Action) {
        match action.action_type {
            ActionType::SelectionTool => {
                self.status = ps!(Status::Idle, action.status);
                self.send(serde_json::json!({ "type": action.msg_type, "value": true }));
            }
            ActionType::RestoreSelection => {
                // A fresh request starts with a clean cancellation state; a
                // stale abort must not swallow the selection that follows.
                Server::clear_abort();
                self.status = ps!(Status::Processing, action.status);
                self.send(serde_json::json!({ "type": action.msg_type, "value": true }));
            }
            ActionType::CopyLayerToClipboard => {
                self.status = ps!(Status::Idle, action.status);
                self.send(serde_json::json!({ "type": action.msg_type, "value": true }));
            }
            ActionType::GPTSend => {
                self.add_prompt = true;
                // self.status = ps!(Status::Idle, action.status);
                // self.send_to_gpt();
            }
            ActionType::GPTAbort => self.abort_all(),
        }
    }

    /// Stops every in-flight operation: the GPT bridge, the Rust pipeline and
    /// whatever Photoshop is still doing for the current request.
    fn abort_all(&mut self) {
        self.status = ps!(Status::Error, "aborted");
        self.run_js(Ok(ABORT_JS.to_string()));
        Server::abort();
        self.send(serde_json::json!({ "type": "abort" }));
    }

    fn toggle_compact(&mut self, ctx: &egui::Context) {
        self.compact_mode = !self.compact_mode;
        let size = if self.compact_mode {
            vec2(WIN_W_COMPACT, WIN_H_COMPACT)
        } else {
            vec2(WIN_W, WIN_H)
        };
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
    }

    fn sync_webview(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        ui.painter().rect_filled(rect, 0, BG);
        let pill = rect.shrink(1.);
        let cy = WIN_H_COMPACT;
        let at = |x: f32| Pos2::new(x + BTN_W / 2.0, cy);

        let setting_pos = at(pill.right() - GAP - BTN_W);
        if Btn::click(
            ui,
            setting_pos,
            "setting_btn",
            "\u{f013}",
            "toltip",
            false,
            false,
        ) {
            self.setting_mode = !self.setting_mode;
            _ = self.webview.set_visible(!self.setting_mode);
        }
        if Btn::click(
            ui,
            at(pill.right() - GAP - 2. * (BTN_W + GAP)),
            "reload_gtp",
            "\u{f0450}",
            "reload_gpt",
            false,
            false,
        ) {
            _ = self.webview.evaluate_script("location.reload()");
        }
        if self.setting_mode {
            self.sync_settingview(ui);
        }
        //dont render again
        if self.webview_rect == Some(rect) {
            return;
        }
        self.webview_rect = Some(rect);
        let _ = self.webview.set_bounds(WvRect {
            position: LogicalPosition::new(rect.min.x, rect.min.y + WEBVIEW_TOP_GAP).into(),
            size: LogicalSize::new(rect.width(), rect.height() - WEBVIEW_TOP_GAP).into(),
        });
    }
    fn sync_settingview(&mut self, ui: &mut egui::Ui) {
        let mut rect = ui.available_rect_before_wrap();
        rect.min.y += BTN_H;
        let inner = rect.shrink2(egui::vec2(16.0, 12.0));

        let mut done = false;
        let mut to_remove: Option<usize> = None;

        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("settings_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());

                    // --- header -------------------------------------------------
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new("Settings")
                                    .size(15.0)
                                    .strong()
                                    .color(TEXT),
                            );
                            ui.label(
                                egui::RichText::new("Tune the restore pipeline")
                                    .size(9.0)
                                    .color(MUTED),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let button = ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("Done").size(11.0).color(BG),
                                    )
                                    .fill(FG)
                                    .stroke(Stroke::NONE)
                                    .corner_radius(CornerRadius::same(CONTROL_RADIUS))
                                    .min_size(vec2(58.0, 26.0)),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if button.clicked() {
                                done = true;
                            }
                        });
                    });
                    ui.add_space(14.0);

                    // --- pipeline -----------------------------------------------
                    settings_section(ui, "PIPELINE", |ui| {
                        settings_row(ui, "Face restore", Some("Restore detected faces"), |ui| {
                            let mut face = self.config.load().face_restore();
                            let before = face;
                            egui::ComboBox::from_id_salt("face_restore_model")
                                .width(140.0)
                                .selected_text(face.label())
                                .show_ui(ui, |ui| {
                                    for model in FaceRestoreModel::ALL {
                                        ui.selectable_value(&mut face, model, model.label());
                                    }
                                });
                            if face != before {
                                self.config.set_face_restore(face);
                            }
                        });

                        settings_divider(ui);

                        settings_row(ui, "Upscale", Some("RealESRGAN 2×"), |ui| {
                            let mut upscale = self.config.load().upscale();
                            if settings_toggle(ui, &mut upscale).changed() {
                                self.config.set_upscale(upscale);
                            }
                        });

                        settings_divider(ui);

                        settings_row(
                            ui,
                            "Background removal",
                            Some("Cut the subject out"),
                            |ui| {
                                let mut background = self.config.load().background_removal();
                                let before = background;
                                egui::ComboBox::from_id_salt("background_removal_model")
                                    .width(140.0)
                                    .selected_text(background.label())
                                    .show_ui(ui, |ui| {
                                        for model in BackgroundRemovalModel::ALL {
                                            ui.selectable_value(
                                                &mut background,
                                                model,
                                                model.label(),
                                            );
                                        }
                                    });
                                if background != before {
                                    self.config.set_background_removal(background);
                                }
                            },
                        );
                    });

                    // Push the current settings to the server for the next request.
                    Server::set_options(self.config.options());

                    ui.add_space(16.0);

                    // --- prompts ------------------------------------------------
                    settings_section(ui, "PROMPTS", |ui| {
                        Frame::new()
                            .fill(BG)
                            .stroke(Stroke::new(1.0, BORDER))
                            .corner_radius(CornerRadius::same(CONTROL_RADIUS))
                            .inner_margin(egui::Margin::symmetric(8, 6))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.add(
                                    egui::TextEdit::multiline(&mut self.prompt_input)
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(3)
                                        .frame(Frame::NONE)
                                        .hint_text("Write a prompt to reuse…"),
                                );
                            });

                        ui.add_space(8.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let can_add = !self.prompt_input.trim().is_empty();
                            let add = ui.add_enabled(
                                can_add,
                                egui::Button::new(
                                    egui::RichText::new("Add prompt").size(11.0).color(BG),
                                )
                                .fill(if can_add { FG } else { BORDER })
                                .stroke(Stroke::NONE)
                                .corner_radius(CornerRadius::same(CONTROL_RADIUS))
                                .min_size(vec2(92.0, 26.0)),
                            );
                            if add.clicked() {
                                let text = self.prompt_input.trim().to_owned();
                                if !text.is_empty() {
                                    self.config.add(text); // saves to disk
                                    self.prompt_input.clear();
                                }
                            }
                        });

                        let prompts = self.config.load().get().clone();
                        ui.add_space(10.0);
                        if prompts.is_empty() {
                            ui.label(
                                egui::RichText::new("No saved prompts yet.")
                                    .size(10.0)
                                    .italics()
                                    .color(MUTED),
                            );
                        } else {
                            ui.spacing_mut().item_spacing.y = 6.0;
                            for (i, item) in prompts.iter().enumerate() {
                                let selected = self.prompt_selected == Some(i);
                                Frame::new()
                                    .fill(if selected {
                                        FG.gamma_multiply(0.16)
                                    } else {
                                        BG
                                    })
                                    .stroke(Stroke::new(1.0, if selected { FG } else { BORDER }))
                                    .corner_radius(CornerRadius::same(CONTROL_RADIUS))
                                    .inner_margin(egui::Margin::symmetric(8, 4))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            let label = ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(item.as_str())
                                                        .size(11.0)
                                                        .color(TEXT),
                                                )
                                                .truncate()
                                                .sense(Sense::click()),
                                            );
                                            if label.clicked() {
                                                ui.ctx().copy_text(item.clone());
                                                self.prompt_selected = Some(i);
                                            }
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if settings_icon_button(ui, "✕", 20.0).clicked()
                                                    {
                                                        to_remove = Some(i);
                                                    }
                                                },
                                            );
                                        });
                                    });
                            }
                        }
                    });

                    ui.add_space(16.0);
                });
        });

        if done {
            self.setting_mode = false;
            _ = self.webview.set_visible(true);
        }

        if let Some(i) = to_remove {
            self.config.remove(i); // saves to disk
            self.prompt_selected = match self.prompt_selected {
                Some(s) if s == i => None,
                Some(s) if s > i => Some(s - 1),
                other => other,
            };
        }
    }
}

// ----------------------------------------------------------- frame phases --

impl QuickRestoreApp {
    /// Notification-area menu: bring the window back or quit for real.
    fn handle_tray(&mut self, ctx: &egui::Context) {
        for action in self.tray_rx.try_iter() {
            match action {
                TrayAction::Show => {
                    self.hidden = false;
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                TrayAction::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
            }
        }
    }

    /// Stretches each frame to `FRAME_BUDGET` so mouse-move bursts cannot paint
    /// faster than MAX_FPS. Measured from the previous frame's *start*, so when
    /// painting is already slower (vsync, slow present) this sleeps for zero.
    fn throttle(&mut self) {
        if let Some(remaining) =
            (self.last_frame + FRAME_BUDGET).checked_duration_since(Instant::now())
        {
            std::thread::sleep(remaining);
        }
        self.last_frame = Instant::now();
    }

    /// The server pushes one-way status updates; keep the newest one.
    fn poll_status(&mut self, ctx: &egui::Context) {
        if let Some(latest) = self.status_rx.try_iter().last() {
            self.status = latest;
        }
        if self.is_busy() {
            ctx.request_repaint_after(BUSY_POLL);
        }
    }

    /// Drag area, background pill, action buttons and the compact toggle.
    /// In compact mode the strip is as tall as the whole window.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let height = if self.compact_mode {
            WIN_H_COMPACT
        } else {
            BTN_H
        };

        Panel::top("top-panel")
            .frame(Frame::NONE)
            .exact_size(BTN_H)
            .show(ui, |ui| {
                let full = ui.max_rect();

                // Drag area first, so buttons sit on top of it.
                let drag = ui.interact(full, ui.id().with("drag"), Sense::click_and_drag());
                if drag.drag_started_by(egui::PointerButton::Primary) {
                    ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
                }

                let pill = full.shrink(1.0);
                ui.painter().rect(
                    pill,
                    CornerRadius::same(0),
                    BG,
                    Stroke::new(1.0, BORDER),
                    StrokeKind::Inside,
                );

                let cy = pill.center().y;
                let at = |x: f32| Pos2::new(x + BTN_W / 2.0, cy);

                // Actions hug the left edge; the compact toggle hugs the right.
                let busy = self.is_busy();
                if self.add_prompt {
                    ui.horizontal_centered(|ui| {
                        let response = ui.put(
                            full.shrink(2.0),
                            TextEdit::singleline(&mut self.prompt).id(Id::new("quick-prompt")),
                        );
                        // Focus the input
                        response.request_focus();

                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));

                        if response.has_focus() && enter {
                            self.send_to_gpt();
                            self.prompt.clear();
                            self.add_prompt = false;
                        }

                        if response.has_focus() && escape {
                            eprintln!("escape: {}", self.prompt);

                            self.prompt.clear();
                            self.add_prompt = false;
                        }
                    });
                } else {
                    for (i, action) in ACTIONS.iter().enumerate() {
                        let x = pill.left() + GAP + i as f32 * (BTN_W + GAP);

                        let clicked = Btn::click(
                            ui,
                            at(x),
                            action.id,
                            action.icon,
                            action.tooltip,
                            false,
                            busy,
                        );

                        if clicked {
                            self.start_action(action);
                        }
                    }
                }

                let toggle_pos = at(pill.right() - GAP - BTN_W);
                if self.compact_mode {
                    ui.painter().circle_filled(
                        toggle_pos,
                        STATUS_DOT / 2.0,
                        status_color(&self.status.is_processing),
                    );

                    egui::Area::new(Id::new("source"))
                        .fixed_pos(egui::pos2(2., 40.))
                        .show(ui, |ui| {
                            ui.painter().rect_filled(ui.max_rect(), 0, BORDER);
                            ui.set_width(ui.available_width());
                            let text = egui::RichText::new(&self.status.status)
                                .color(TEXT)
                                .size(8.0);

                            ui.add(egui::Label::new(text).truncate());
                        });
                }
                // Keeps the app (and the pipeline) alive; quit lives in the tray menu.
                if Btn::click(
                    ui,
                    at(pill.right() - GAP - 2. * BTN_W),
                    "gpt-abort",
                    "\u{f073a}",
                    "abort all operations",
                    true,
                    false,
                ) {
                    self.abort_all();
                }
                if Btn::click(
                    ui,
                    toggle_pos,
                    "compact",
                    "\u{f4c3}",
                    "2.*Compact",
                    true,
                    false,
                ) {
                    self.toggle_compact(ui.ctx());
                }
            });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        Panel::bottom("status-panel")
            .frame(Frame::NONE.inner_margin(egui::Margin::symmetric(6, 2)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;

                    let (dot, _) =
                        ui.allocate_exact_size(vec2(STATUS_DOT, STATUS_DOT), Sense::hover());
                    ui.painter().circle_filled(
                        dot.center(),
                        STATUS_DOT / 2.0,
                        status_color(&self.status.is_processing),
                    );

                    // One line tall: clamp with an ellipsis.
                    let text = egui::RichText::new(&self.status.status)
                        .color(TEXT)
                        .size(10.0);
                    ui.add(egui::Label::new(text).truncate());
                });
            });
    }

    /// Hosts the native webview plus the GPT helper buttons.
    fn web_panel(&mut self, ui: &mut egui::Ui) {
        CentralPanel::default().frame(Frame::NONE).show(ui, |ui| {
            let rect = ui.available_rect_before_wrap();
            self.sync_webview(ui, rect);
        });
    }
    fn send_to_gpt(&mut self) {
        let image = gpt::clipboard_image_base64().ok();
        self.run_js(gpt::process_prompt_js(image.as_deref(), &self.prompt).map_err(Into::into));
        self.status = ps!(Status::Idle, "Sending to GPT");
    }

    fn poll_gpt(&mut self) {
        use gpt::GptEvent::*;
        let events: Vec<_> = self.gpt_rx.try_iter().collect();
        for ev in events {
            self.status = match ev {
                Ready { .. } => continue,
                PromptSent { .. } => ps!(Status::Idle, "Waiting for image"),
                ImageCopied {
                    base64,
                    width,
                    height,
                    ..
                } => {
                    if let Err(e) = Server::save_gpt_image(
                        &self.cmd_tx,
                        CanvasImage {
                            base64: base64,
                            mime_type: "image/png".into(),
                            width,
                            height,
                        },
                    ) {
                        ps!(Status::Error, format!("{}", e))
                    } else {
                        ps!(Status::Idle, format!("Got image {width}x{height}"))
                    }
                }
                BridgeError { error, .. } => ps!(Status::Error, error),
            };
        }
    }
}

impl eframe::App for QuickRestoreApp {
    /// Opaque window: clear to the app background so no black frame shows
    /// around the pill.
    fn clear_color(&self, _visuals: &Visuals) -> [f32; 4] {
        egui::Rgba::from(BG).to_array()
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        self.handle_tray(&ctx);

        // A hidden window receives no input, so poll slowly instead.
        if self.hidden {
            ctx.request_repaint_after(HIDDEN_POLL);
            return;
        }

        if !self.chrome_stripped {
            if let Some(window) = frame.winit_window() {
                platform::strip_window_chrome(window);
                self.chrome_stripped = true;
            }
        }

        self.throttle();
        self.poll_status(&ctx);
        self.poll_gpt();

        self.top_bar(ui);
        if !self.compact_mode {
            self.status_bar(ui);
            self.web_panel(ui);
        }
    }
}

// ------------------------------------------------------------------ setup --

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Reads `model` from the config file, creating it with defaults if missing.
fn model_from_config(path: &Path) -> Option<String> {
    if !path.exists() {
        let default = serde_json::json!({ "model": DEFAULT_MODEL_DIR });
        if let Ok(s) = serde_json::to_string_pretty(&default) {
            let _ = std::fs::write(path, s);
        }
    }
    let content = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    json.get("model")?.as_str().map(str::to_owned)
}

/// `--model <dir>` or `--model=<dir>`; the last occurrence wins.
fn model_from_cli() -> Option<String> {
    let mut found = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--model" {
            found = args.next().or(found);
        } else if let Some(val) = arg.strip_prefix("--model=") {
            found = Some(val.to_owned());
        }
    }
    found
}

fn spawn_server(
    model_root: String,
    status_tx: mpsc::Sender<ProgressStatus>,
) -> mpsc::SyncSender<Cmd> {
    let (tx, rx) = mpsc::sync_channel::<Cmd>(32);
    let cmd_tx = tx.clone();
    std::thread::spawn(move || server::Server::run(tx, rx, model_root, status_tx));
    cmd_tx
}
/// Registers `fd` under `name` and places it at `priority` inside `family`.
/// Lower indexes win when two fonts contain the same glyph, so the text face
/// goes first and the icon face follows as a fallback.
pub fn add_to_fonts(
    fonts: &mut egui::FontDefinitions,
    family: egui::FontFamily,
    name: &str,
    fd: FontData,
    priority: usize,
) {
    fonts.font_data.insert(name.to_owned(), Arc::new(fd));
    let list = fonts.families.entry(family).or_default();
    list.retain(|font| font != name);
    list.insert(priority.min(list.len()), name.to_owned());
}

fn setup_egui(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_visuals_of(egui::Theme::Dark, consts::visuals());

    let mut fonts = egui::FontDefinitions::default();

    // Nerd Fonts are only needed for their private-use icon glyphs, so keep
    // the vertical tweak that lines those glyphs up with the text baseline.
    let nerd = || {
        let mut fd = FontData::from_static(include_bytes!("assets/font/3270NerdFont-Regular.ttf"));
        fd.tweak.y_offset_factor = 0.0;
        fd
    };
    let inter = FontData::from_static(include_bytes!("assets/font/Inter_18pt-Medium.ttf"));

    // Proportional: Inter is the default text face, Nerd Font is the fallback
    // for icon glyphs. egui's emoji fonts stay after both.
    add_to_fonts(
        &mut fonts,
        egui::FontFamily::Proportional,
        "inter",
        inter,
        0,
    );
    add_to_fonts(
        &mut fonts,
        egui::FontFamily::Proportional,
        "nerdfonts",
        nerd(),
        1,
    );
    // Monospace: keep Hack for code, but still resolve icon glyphs.
    add_to_fonts(
        &mut fonts,
        egui::FontFamily::Monospace,
        "nerdfonts",
        nerd(),
        1,
    );

    ctx.set_fonts(fonts);
}

fn build_webview(
    cc: &eframe::CreationContext<'_>,
    events: mpsc::Sender<gpt::GptEvent>,
) -> anyhow::Result<WebView> {
    let ctx = cc.egui_ctx.clone();
    let webview = WebViewBuilder::new()
        .with_url(START_URL)
        .with_devtools(true)
        .with_initialization_script_for_main_only(gpt::INIT_JS, true)
        .with_ipc_handler(move |req| match serde_json::from_str(req.body()) {
            Ok(ev) => {
                let _ = events.send(ev);
                ctx.request_repaint(); // wake the UI so poll_gpt runs
            }
            Err(e) => eprintln!("bad ipc message: {e}"),
        })
        .build_as_child(cc)?;
    _ = webview.zoom(WEBVIEW_ZOOM);
    Ok(webview)
}

fn main() -> eframe::Result {
    let model_root = model_from_cli().or_else(|| model_from_config(&exe_dir().join(CONFIG_FILE)));
    let model_root = rembg_core::config::model_root(model_root.as_deref());

    let (status_tx, status_rx) = mpsc::channel::<ProgressStatus>();
    let _ = status_tx.send(ps!(format!("Using model root: {model_root}")));

    let config = Config::new();
    Server::set_options(config.options());
    let cmd_tx = spawn_server(model_root, status_tx);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(vec2(WIN_W, WIN_H))
            .with_decorations(false)
            .with_resizable(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_transparent(false),
        ..Default::default()
    };

    eframe::run_native(
        "QuickRestore",
        native_options,
        Box::new(move |cc| {
            setup_egui(&cc.egui_ctx);

            let (tray_tx, tray_rx) = mpsc::channel();
            let tray = Tray::new(&cc.egui_ctx, tray_tx)
                .map_err(|e| eprintln!("notification-area icon unavailable: {e}"))
                .ok();
            let (gpt_tx, gpt_rx) = mpsc::channel();
            let prompts = config.load().get().clone();
            Ok(Box::new(QuickRestoreApp {
                webview: build_webview(cc, gpt_tx)?,
                webview_rect: None,
                compact_mode: false,
                cmd_tx,
                status: ps!("Initiating..."),
                status_rx,
                tray_rx,
                last_frame: Instant::now(),
                hidden: false,
                _tray: tray,
                chrome_stripped: false,
                prompt: String::new(),
                gpt_rx,
                setting_mode: false,
                config,
                prompts,
                prompt_input: String::new(),
                prompt_selected: None,
                add_prompt: false,
            }))
        }),
    )
}
