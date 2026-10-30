mod util;
mod widget;
mod worker;
use anyhow::Result;
use eframe::egui::{
    self, Align, ColorImage, Context, CursorIcon, Frame, IntoAtoms, Layout, Panel, Rect as ERect,
    Sense, TextEdit, TextureHandle, TextureOptions, UiBuilder, Vec2, ViewportCommand, Visuals,
    pos2, vec2,
};
use egui_nerdfonts::regular as nf;
use serde_json::{Value, json};
use std::{
    borrow::Cow,
    env,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};
use widget::Btn;
use wry::{
    Rect, WebView, WebViewBuilder,
    dpi::{LogicalPosition, LogicalSize},
};

use crate::worker::{UiEvent, WorkerCommand};
const DEFAULT_PORT: u16 = 7878;

const TEMPLATE: [&str; 3] = ["\u{f292}", "\u{e81b}", "\u{f0065}"];
struct CraftApp {
    webview: WebView,
    texts: String,
    side_w: f32,     // init: 300.0
    web_shown: bool, // init: true
    last_rect: Option<egui::Rect>,
    is_image_shown: bool,

    con_status: String,
    con_connected: bool,
    con_connecting: bool,
    con_output: String,
    con_next_request_id: u64,
    con_cmd_tx: Option<Sender<worker::WorkerCommand>>,
    con_event_rx: Receiver<worker::UiEvent>,
    con_event_tx: Sender<worker::UiEvent>,
    is_editing: bool,
}

impl CraftApp {
    fn side_chat(&mut self, ui: &mut egui::Ui) {
        let (web_rect, dragging) = Panel::left("chatgpt")
            .resizable(false)
            .exact_size(self.side_w)
            .frame(Frame::NONE)
            .show(ui, |ui| {
                let full = ui.max_rect();
                let handle = ERect::from_min_max(pos2(full.max.x - 8.0, full.min.y), full.max);

                let resp = ui.interact(handle, ui.id().with("splitter"), Sense::drag());
                if resp.hovered() || resp.dragged() {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                }
                if resp.dragged() {
                    self.side_w = (self.side_w + resp.drag_delta().x).clamp(80.0, 600.0);
                }
                // visible divider line
                ui.painter().rect_filled(
                    handle.shrink2(vec2(3.0, 0.0)),
                    0.0,
                    ui.visuals().widgets.noninteractive.bg_stroke.color,
                );

                // webview gets everything left of the strip
                let web = ERect::from_min_max(full.min, pos2(full.max.x - 8.0, full.max.y));
                (web, resp.dragged())
            })
            .inner;

        if self.web_shown == dragging {
            self.web_shown = !dragging;
            let _ = self.webview.set_visible(self.web_shown);
        }
        if self.last_rect != Some(web_rect) {
            if let Err(e) = self.webview.set_bounds(Rect {
                position: LogicalPosition::new(web_rect.min.x as f64, web_rect.min.y as f64).into(),
                size: LogicalSize::new(web_rect.width() as f64, web_rect.height() as f64).into(),
            }) {
                eprintln!("set_bounds failed: {e}");
            }
            self.last_rect = Some(web_rect);
        }
    }
    fn resize_window(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(900.0, 500.0)));
    }

    fn web_floating(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut web_rect: Option<egui::Rect> = None;

        egui::Window::new("Chat")
            .open(&mut true) // adds the ✕ button
            .default_size([420.0, 320.0])
            .resizable(true)
            .show(&ctx, |ui| {
                ui.set_min_size(vec2(200.0, 150.0));
                let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
                web_rect = Some(rect.shrink(2.0)); // 2px margin so window edges stay grabbable
            });

        match web_rect {
            Some(r) => {
                if self.last_rect != Some(r) {
                    let _ = self.webview.set_bounds(Rect {
                        position: LogicalPosition::new(r.min.x as f64, r.min.y as f64).into(),
                        size: LogicalSize::new(r.width() as f64, r.height() as f64).into(),
                    });
                    self.last_rect = Some(r);
                }
            }
            None => {
                let _ = self.webview.set_visible(false);
            }
        }
    }
    fn con_or_discon(&mut self) {
        if self.con_connected {
            if let Some(tx) = self.con_cmd_tx.take() {
                let _ = tx.send(WorkerCommand::Disconnect);
            }
            self.con_connected = false;
            self.con_connecting = false;
            self.con_status = "Disconnect requested".into();
            return;
        }
        if self.con_connecting {
            return;
        }
        let exe_path = env::var_os("PHOTOCRAFT_EXE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                env::current_dir()
                    .unwrap_or_default()
                    .join("photocraft.exe")
            });
        eprintln!("pc : {:?}", exe_path);
        let token_path = env::var_os("PHOTOCRAFT_CONTROL_TOKEN_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                env::current_dir()
                    .unwrap_or_default()
                    .join("photocraft-control.token")
            });
        let port = env::var("PHOTOCRAFT_CONTROL_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(DEFAULT_PORT);
        let (command_tx, command_rx) = mpsc::channel();
        self.con_cmd_tx = Some(command_tx);
        self.con_connecting = true;
        self.con_status = format!("Connecting to 127.0.0.1:{port}…");
        let event_tx = self.con_event_tx.clone();
        thread::spawn(move || {
            if let Err(error) =
                worker::connect_worker(exe_path, token_path, port, command_rx, event_tx.clone())
            {
                let _ = event_tx.send(worker::UiEvent::Failed(error));
            }
        });
    }
    fn poll_events(&mut self) {
        while let Ok(event) = self.con_event_rx.try_recv() {
            match event {
                UiEvent::Status(s) => self.con_status = s,
                UiEvent::Authenticated => {
                    self.con_connected = true;
                    self.con_connecting = false;
                    self.con_status = "Authenticated — ready to send commands".into();
                    self.con_output.push_str("\n[PhotoCraft authenticated]\n");
                }
                UiEvent::Reply(reply) => {
                    self.con_output.push_str("\n< ");
                    self.con_output.push_str(&reply);
                    self.con_output.push('\n');
                }
                UiEvent::Disconnected(reason) => {
                    self.con_connected = false;
                    self.con_connecting = false;
                    self.con_cmd_tx = None;
                    self.con_status = format!("Disconnected: {reason}");
                }
                UiEvent::Failed(error) => {
                    self.con_connected = false;
                    self.con_connecting = false;
                    self.con_cmd_tx = None;
                    self.con_status = format!("Connection failed: {error}");
                    self.con_output.push_str("\n[ERROR] ");
                    self.con_output.push_str(&error);
                    self.con_output.push('\n');
                }
            }
        }
    }

    fn send_request(&mut self, parsed: Value) {
        if !self.con_connected {
            self.con_output = "Connect to PhotoCraft before sending a request.".into();
            return;
        }
        if !parsed.is_object() {
            self.con_output = "A request must be a JSON object.".into();
            return;
        }
        let compact = parsed.to_string();
        match self
            .con_cmd_tx
            .as_ref()
            .map(|tx| tx.send(WorkerCommand::Send(compact)))
        {
            Some(Ok(())) => {
                self.con_output.push_str("\n> ");
                self.con_output.push_str("\nWaiting for response…\n");
                self.con_next_request_id = self.con_next_request_id.saturating_add(1);
            }
            _ => {
                self.con_output = "Connection worker is unavailable. Reconnect.".into();
                self.con_connected = false;
                self.con_status = "Disconnected".into();
            }
        }
    }

    fn edit_lines(&mut self, texts: &str) {
        let lines: Vec<&str> = {
            texts
                .trim()
                .split('\n')
                .filter(|line| !line.trim().is_empty())
                .collect()
        };

        self.is_editing = true;

        for (idx, line) in lines.iter().enumerate() {
            let select = json!({
              "method": "engine.execute",
              "params": {
                "command": "select.findLayers",
                "params": {
                  "name": format!("text{}",idx+1)
                }
              }
            });
            let text = json!({
              "method": "engine.execute",
              "params": {
                "command": "type.edit",
                "params": {
                    "text":line,
                }
              }
            });
            self.send_request(select);
            self.send_request(text);
        }
        self.is_editing = false;
    }
}

impl eframe::App for CraftApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_events();
        let ctx = ui.ctx().clone();
        Panel::top("title")
            .show(ui, |ui| {
                let bar = ui
                    .scope_builder(UiBuilder::new().sense(Sense::drag()), |ui| {
                        ui.horizontal(|ui| {
                            ui.label(nf::I_SETI_RUST);
                            ui.label("Craftnailizer 1.0-rc");
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                Btn::new(nf::CLOSE)
                                    .clicked(ui, || ctx.send_viewport_cmd(ViewportCommand::Close));
                            });
                        });
                    })
                    .response;
                if bar.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
            })
            .response;
        // self.web_floating(ui);
        // self.side_chat(ui);
        let _ = self.webview.set_visible(self.web_shown);
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                for t in TEMPLATE {
                    Btn::new(t).size(16.0).clicked(ui, || {
                        eprintln!("{}", t);
                        eprintln!("-------");
                    });
                }
            });
            if ui
                .add(
                    TextEdit::multiline(&mut self.texts)
                        .desired_width(f32::INFINITY)
                        .desired_rows(6),
                )
                .changed()
            {
                self.edit_lines(&self.texts.clone());
            };
            let label = if self.con_connected {
                "Disconnect"
            } else if self.con_connecting {
                "Connecting…"
            } else {
                "Launch + Connect"
            };
            if ui.button(label).clicked() {
                self.con_or_discon();
            }
            ui.separator();
            ui.label(&self.con_status);
        });
    }
}
fn main() -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_decorations(false)
            .with_always_on_top()
            .with_inner_size(Vec2::new(290.0, 480.0)),
        ..Default::default()
    };
    let app = eframe::run_native(
        "Craftnailizer",
        options,
        Box::new(|cc| {
            let webview = WebViewBuilder::new()
                .with_url("https://chatgpt.com")
                .with_ipc_handler(|req| println!("from page: {}", req.body()))
                .build_as_child(cc)?;
            if let Err(e) = webview.zoom(0.65) {
                eprintln!("failed to zoom {e}");
            }

            let ctx = &cc.egui_ctx;

            ctx.set_visuals(Visuals::dark());
            util::install_fonts(ctx);
            let mut fonts = egui::FontDefinitions::default();
            egui_nerdfonts::add_to_fonts(&mut fonts, egui_nerdfonts::Variant::Regular);
            cc.egui_ctx.set_fonts(fonts);

            let (event_tx, event_rx) = mpsc::channel();
            Ok(Box::new(CraftApp {
                webview,
                texts: String::new(),
                con_output: String::new(),
                last_rect: None,
                side_w: 0.0,
                web_shown: false,
                is_image_shown: false,
                con_status: "Disconnected".into(),
                con_connected: false,
                con_connecting: false,
                con_next_request_id: 5,
                con_cmd_tx: None,
                con_event_rx: event_rx,
                con_event_tx: event_tx,
                is_editing: false,
            }))
        }),
    )
    .expect("Error runnung app");
    Ok(app)
}
