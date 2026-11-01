mod consts;
mod model;

mod server;
mod tray;
mod widget;
use consts::*;
use eframe::egui::{
    self, CentralPanel, CornerRadius, Frame, Panel, Pos2, Sense, Stroke, StrokeKind,
    ViewportCommand, Visuals, vec2,
};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::model::{Cmd, ProgressStatus, Status};
use crate::tray::{Tray, TrayAction};
use crate::widget::Btn;

/// eframe has no native frame-rate cap (`NativeOptions::max_fps` is web-only),
/// and every pointer move over the window triggers a paint.
const FRAME_BUDGET: Duration = Duration::from_micros(1_000_000 / MAX_FPS as u64);

struct QuickRestoreApp {
    compact_mode: bool,
    cmd_tx: mpsc::SyncSender<Cmd>,
    status: ProgressStatus,
    status_rx: mpsc::Receiver<ProgressStatus>,
    config_path: String,
    last_frame: Instant,
    tray_rx: mpsc::Receiver<TrayAction>,
    /// Set while the window lives only in the notification area.
    hidden: bool,
    /// Dropping the tray removes the notification-area icon.
    _tray: Option<Tray>,
    /// One-shot: drop the Windows drop shadow / 1 px top line.
    shadow_removed: bool,
}

impl std::ops::Deref for QuickRestoreApp {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.config_path
    }
}

impl QuickRestoreApp {
    fn send(&self, v: serde_json::Value) {
        let _ = self.cmd_tx.try_send(Cmd::SendText(v.to_string()));
    }
}

impl eframe::App for QuickRestoreApp {
    fn clear_color(&self, _visuals: &Visuals) -> [f32; 4] {
        // Opaque window: clear to the app background so no black frame /
        // compositor-default pixels show through around the pill.
        egui::Rgba::from(BG).to_array()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Notification-area menu: bring the window back or quit for real.
        for action in self.tray_rx.try_iter() {
            match action {
                TrayAction::Show => {
                    self.hidden = false;
                    ui.ctx().send_viewport_cmd(ViewportCommand::Visible(true));
                    ui.ctx().send_viewport_cmd(ViewportCommand::Focus);
                }
                TrayAction::Quit => {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }

        // A hidden window receives no input, so nothing would run this handler.
        // Poll slowly while hidden: that is what turns menu clicks into frames.
        if self.hidden {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
            return;
        }

        // Drop the Windows drop shadow / 1 px top line that winit draws for
        // undecorated windows (one-shot; the call is idempotent but touches
        // window styles, so don't repeat it every frame).
        if !self.shadow_removed {
            if let Some(w) = _frame.winit_window() {
                #[cfg(windows)]
                {
                    use winit::platform::windows::WindowExtWindows;
                    w.set_undecorated_shadow(false);
                    // Match the border to the app background so the 1 px DWM
                    // frame line blends in instead of showing as a dark edge.
                    w.set_border_color(Some(winit::platform::windows::Color::from_rgb(18, 20, 26)));

                    // Disable DWM non-client rendering entirely: this removes the
                    // remaining 1 px frame line that Win11 draws for windows that
                    // still carry WS_CAPTION (winit keeps it for snap support).
                    #[link(name = "dwmapi")]
                    unsafe extern "system" {
                        fn DwmSetWindowAttribute(
                            hwnd: isize,
                            attr: u32,
                            value: *const core::ffi::c_void,
                            size: u32,
                        ) -> i32;
                    }
                    // DWMWA_NCRENDERING_POLICY = 2, DWMNCRP_DISABLED = 2
                    let hwnd = {
                        use winit::raw_window_handle::HasWindowHandle;
                        match w.window_handle() {
                            Ok(h) => match h.as_raw() {
                                winit::raw_window_handle::RawWindowHandle::Win32(w32) => {
                                    w32.hwnd.get()
                                }
                                _ => 0,
                            },
                            Err(_) => 0,
                        }
                    };
                    let policy: i32 = 2;
                    if hwnd != 0 {
                        unsafe {
                            DwmSetWindowAttribute(
                                hwnd,
                                2,
                                &policy as *const _ as *const core::ffi::c_void,
                                4,
                            );
                        }
                    }
                }
                self.shadow_removed = true;
            }
        }

        // Stretch each frame to the budget so mouse-move bursts cannot paint faster
        // than MAX_FPS. The budget is measured from the previous frame's *start*, so
        // when painting is already slower than the budget (vsync on a 60 Hz panel, a
        // slow present) this sleeps for zero instead of halving the rate.
        if let Some(remaining) =
            (self.last_frame + FRAME_BUDGET).checked_duration_since(Instant::now())
        {
            std::thread::sleep(remaining);
        }
        self.last_frame = Instant::now();

        // The server pushes one-way status updates; keep the newest one.
        if let Some(latest) = self.status_rx.try_iter().last() {
            self.status = latest;
        }

        // While a pipeline runs, poll at 10 Hz so the dot and text stay live
        // even when the pointer is idle (frames otherwise only come from input).
        match self.status.is_processing {
            Status::Processing => ui.ctx().request_repaint_after(Duration::from_millis(100)),
            _ => {}
        }
        // One panel type for both modes: in compact mode the strip is as tall as
        // the whole window, so it covers it the way a `CentralPanel` would.
        let panel = Panel::top("top-panel")
            .frame(Frame::NONE)
            .exact_size(if self.compact_mode {
                WIN_H_COMPACT
            } else {
                BTN_H
            });

        panel.show(ui, |ui| {
            let full = ui.max_rect();

            // drag area (added first so buttons sit on top of it)
            let drag = ui.interact(full, ui.id().with("drag"), Sense::click_and_drag());
            if drag.drag_started_by(egui::PointerButton::Primary) {
                ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
            }

            // pill background
            let pill = full.shrink(1.0);
            ui.painter().rect(
                pill,
                CornerRadius::same(0),
                // CornerRadius::same((pill.height() / 2.0) as u8),
                BG,
                Stroke::new(1.0, BORDER),
                StrokeKind::Inside,
            );

            // three actions hug the left edge, close hugs the right edge
            let cy = pill.center().y;
            let at = |x: f32| Pos2::new(x + BTN_W / 2.0, cy);

            let left = pill.left() + GAP;
            let right = pill.right() - GAP; // right edge of the close button

            let x1 = left;
            let x2 = left + BTN_W + GAP;
            let x3 = left + 2.0 * (BTN_W + GAP);
            let x4 = right - BTN_W;

            // Action buttons are muted while a pipeline is running.
            let busy = matches!(self.status.is_processing, Status::Processing);
            if Btn::click(ui, at(x1), "sel-tool", "\u{f019f}", "Get selection", false, busy) {
                self.status = ps!(Status::Processing, "Processing");
                self.send(serde_json::json!({ "type": "selection_tool", "value": true }));
            }
            if Btn::click(ui, at(x2), "sel", "\u{f1077}", "Get selection", false, busy) {
                self.status = ps!(Status::Processing, "Processing");
                self.send(serde_json::json!({ "type": "get_selection", "value": true }));
            }
            if Btn::click(ui, at(x3), "copy", "\u{f0100}", "Copy", false, busy) {
                self.status = ps!(Status::Processing, "Requesting copy layer");
                self.send(serde_json::json!({ "type": "copy", "value": true }));
            }
            if self.compact_mode {
                let color = match self.status.is_processing {
                    Status::Processing => INFO,
                    Status::Idle => SUCCESS,
                    Status::Error => DANGER,
                };
                ui.painter().circle_filled(at(x4), 6.0 / 2.0, color);
            }
            if Btn::click(ui, at(x4), "compact", "\u{f4c3}", "Hide to tray", true, false) {
                // Keep the app (and the pipeline) alive; quit lives in the tray menu.
                //self.hidden = true;
                self.compact_mode = !self.compact_mode;
                let size = match self.compact_mode {
                    true => vec2(WIN_W_COMPACT, WIN_H_COMPACT),
                    false => vec2(WIN_W, WIN_H),
                };
                ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(size));
            }
        });
        if !self.compact_mode {
            Panel::bottom("status-panel")
                .frame(Frame::NONE.inner_margin(egui::Margin::symmetric(6, 2)))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;

                        // tiny status pill: green while processing, red when idle
                        let d = 6.0;
                        let (dot, _) = ui.allocate_exact_size(egui::vec2(d, d), Sense::hover());
                        let color = match self.status.is_processing {
                            Status::Processing => INFO,
                            Status::Idle => SUCCESS,
                            Status::Error => DANGER,
                        };

                        ui.painter().circle_filled(dot.center(), d / 2.0, color);

                        // keep the bar one line tall: clamp with an ellipsis
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&self.status.status)
                                    .color(TEXT)
                                    .size(10.),
                            )
                            .truncate(),
                        );
                    });
                });
            CentralPanel::default()
                .frame(Frame::NONE)
                .show(ui, |ui| ui.label("Content"));
        }
    }
}

fn main() -> eframe::Result {
    // Locate quickrestore.json next to the executable
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let config_path = exe_dir.join("quickrestore.json");

    if !config_path.exists() {
        let default = serde_json::json!({ "model": "E:\\_CODE\\rust\\rembg\\models" });
        if let Ok(s) = serde_json::to_string_pretty(&default) {
            let _ = std::fs::write(&config_path, s);
        }
    }

    let mut model_root: Option<String> = None;
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(m) = v.get("model").and_then(|x| x.as_str()) {
                model_root = Some(m.to_string());
            }
        }
    }

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--model" {
            if let Some(val) = args.next() {
                model_root = Some(val);
            }
        } else if let Some(val) = arg.strip_prefix("--model=") {
            model_root = Some(val.to_string());
        }
    }

    let model_root = rembg_core::config::model_root(model_root.as_deref());
    let (status_tx, status_rx) = mpsc::channel::<ProgressStatus>();
    let _ = status_tx.send(ps!(format!("Using model root: {}", model_root)));

    let (tx, rx) = mpsc::sync_channel::<Cmd>(32);
    let cmd_tx = tx.clone();
    let mr = model_root.clone();
    std::thread::spawn(move || {
        server::Server::run(tx, rx, mr, status_tx);
    });
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size(egui::vec2(WIN_W, WIN_H))
            .with_decorations(false)
            .with_resizable(false)
            .with_always_on_top()
            // Tool window: no taskbar button, no Alt-Tab entry. The tray is the
            // only place where the app can be found (and restored) from.
            .with_taskbar(false)
            .with_transparent(false),
        ..Default::default()
    };
    eframe::run_native(
        "QuickRestore",
        native_options,
        Box::new(|cc| {
            cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
            cc.egui_ctx
                .set_visuals_of(egui::Theme::Dark, consts::visuals());

            let mut fonts = egui::FontDefinitions::default();
            egui_nerdfonts::add_to_fonts(&mut fonts, egui_nerdfonts::Variant::Regular);
            cc.egui_ctx.set_fonts(fonts);

            let (tray_tx, tray_rx) = mpsc::channel();
            let tray = match Tray::new(&cc.egui_ctx, tray_tx) {
                Ok(tray) => Some(tray),
                Err(err) => {
                    eprintln!("notification-area icon unavailable: {err}");
                    None
                }
            };

            Ok(Box::new(QuickRestoreApp {
                compact_mode: false,
                status: ps!("Initiating..."),
                status_rx,
                cmd_tx,
                config_path: config_path.to_string_lossy().to_string(),
                last_frame: Instant::now(),
                tray_rx,
                hidden: false,
                _tray: tray,
                shadow_removed: false,
            }))
        }),
    )
}
