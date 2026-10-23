use eframe::egui;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::Command,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

const DEFAULT_PORT: u16 = 7878;
const COMMANDS_JSON: &str = include_str!("../assets/commands.json");

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommandInfo {
    id: String,
    label: String,
    menu: Vec<String>,
    enabled: bool,
    disabled_reason: Option<String>,
    needs_document: bool,
    shortcut: Option<String>,
    params_schema: String,
    defaults: Value,
    #[serde(skip)]
    search_text: String,
}
impl CommandInfo {
    fn display(&self) -> String {
        let path = self.menu.join(" → ");
        if path.is_empty() {
            format!("{}  [{}]", self.label, self.id)
        } else {
            format!("{}  [{}]  — {}", self.label, self.id, path)
        }
    }
}

enum WorkerCommand {
    Send(String),
    Disconnect,
}
enum UiEvent {
    Status(String),
    Authenticated,
    Reply(String),
    Disconnected(String),
    Failed(String),
}

struct ControlApp {
    commands: Vec<CommandInfo>,
    // Indexes into `commands`, recomputed only when search/filter settings change.
    filtered_indices: Vec<usize>,
    selected_command: usize,
    search: String,
    show_disabled: bool,
    request_text: String,
    output: String,
    status: String,
    connected: bool,
    connecting: bool,
    next_request_id: u64,
    command_tx: Option<Sender<WorkerCommand>>,
    event_rx: Receiver<UiEvent>,
    event_tx: Sender<UiEvent>,
}
impl Default for ControlApp {
    fn default() -> Self {
        let mut commands: Vec<CommandInfo> = serde_json::from_str(COMMANDS_JSON)
            .expect("embedded command catalog must be valid JSON");

        // Precompute each command's searchable text once. Do not allocate and
        // lowercase 900+ strings on every egui frame.
        for cmd in &mut commands {
            cmd.search_text = format!(
                "{} {} {} {}",
                cmd.label,
                cmd.id,
                cmd.menu.join(" "),
                cmd.params_schema
            )
            .to_lowercase();
        }

        let (event_tx, event_rx) = mpsc::channel();
        let mut app = Self {
            commands,
            filtered_indices: Vec::new(),
            selected_command: 0,
            search: String::new(),
            show_disabled: false,
            request_text: String::new(),
            output: String::new(),
            status: "Disconnected".into(),
            connected: false,
            connecting: false,
            next_request_id: 1,
            command_tx: None,
            event_rx,
            event_tx,
        };
        app.refresh_filter();
        app.fill_selected_command();
        app
    }
}
impl ControlApp {
    fn refresh_filter(&mut self) {
        let needle = self.search.trim().to_lowercase();
        self.filtered_indices = self
            .commands
            .iter()
            .enumerate()
            .filter(|(_, cmd)| {
                (self.show_disabled || cmd.enabled)
                    && (needle.is_empty() || cmd.search_text.contains(&needle))
            })
            .map(|(index, _)| index)
            .collect();
    }

    fn fill_selected_command(&mut self) {
        let Some(cmd) = self.commands.get(self.selected_command) else {
            return;
        };
        let request = json!({
            "id": self.next_request_id,
            "method": "engine.execute",
            "params": { "command": cmd.id, "params": cmd.defaults }
        });
        self.request_text =
            serde_json::to_string_pretty(&request).unwrap_or_else(|_| request.to_string());
    }
    fn selected_command_changed(&mut self, index: usize) {
        if index < self.commands.len() {
            self.selected_command = index;
            self.fill_selected_command();
        }
    }
    fn connect_or_disconnect(&mut self) {
        if self.connected {
            if let Some(tx) = self.command_tx.take() {
                let _ = tx.send(WorkerCommand::Disconnect);
            }
            self.connected = false;
            self.connecting = false;
            self.status = "Disconnect requested".into();
            return;
        }
        if self.connecting {
            return;
        }
        let exe_path = env::var_os("PHOTOCRAFT_EXE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                env::current_dir()
                    .unwrap_or_default()
                    .join("photocraft.exe")
            });
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
        self.command_tx = Some(command_tx);
        self.connecting = true;
        self.status = format!("Connecting to 127.0.0.1:{port}…");
        let event_tx = self.event_tx.clone();
        thread::spawn(move || {
            if let Err(error) =
                connect_worker(exe_path, token_path, port, command_rx, event_tx.clone())
            {
                let _ = event_tx.send(UiEvent::Failed(error));
            }
        });
    }
    fn send_request(&mut self) {
        if !self.connected {
            self.output = "Connect to PhotoCraft before sending a request.".into();
            return;
        }
        let parsed: Value = match serde_json::from_str(&self.request_text) {
            Ok(v) => v,
            Err(e) => {
                self.output = format!("Invalid JSON: {e}");
                return;
            }
        };
        if !parsed.is_object() {
            self.output = "A request must be a JSON object.".into();
            return;
        }
        let compact = parsed.to_string();
        match self
            .command_tx
            .as_ref()
            .map(|tx| tx.send(WorkerCommand::Send(compact)))
        {
            Some(Ok(())) => {
                self.output.push_str("\n> ");
                self.output.push_str(&self.request_text);
                self.output.push_str("\nWaiting for response…\n");
                self.next_request_id = self.next_request_id.saturating_add(1);
            }
            _ => {
                self.output = "Connection worker is unavailable. Reconnect.".into();
                self.connected = false;
                self.status = "Disconnected".into();
            }
        }
    }
    fn poll_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                UiEvent::Status(s) => self.status = s,
                UiEvent::Authenticated => {
                    self.connected = true;
                    self.connecting = false;
                    self.status = "Authenticated — ready to send commands".into();
                    self.output.push_str("\n[PhotoCraft authenticated]\n");
                }
                UiEvent::Reply(reply) => {
                    self.output.push_str("\n< ");
                    self.output.push_str(&reply);
                    self.output.push('\n');
                }
                UiEvent::Disconnected(reason) => {
                    self.connected = false;
                    self.connecting = false;
                    self.command_tx = None;
                    self.status = format!("Disconnected: {reason}");
                }
                UiEvent::Failed(error) => {
                    self.connected = false;
                    self.connecting = false;
                    self.command_tx = None;
                    self.status = format!("Connection failed: {error}");
                    self.output.push_str("\n[ERROR] ");
                    self.output.push_str(&error);
                    self.output.push('\n');
                }
            }
        }
    }
}
impl eframe::App for ControlApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_events();
        egui::Panel::top("top_panel")
            .min_size(32.0)
            .show_collapsible(ui, &mut true, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let label = if self.connected {
                            "Disconnect"
                        } else if self.connecting {
                            "Connecting…"
                        } else {
                            "Launch + Connect"
                        };
                        if ui
                            .add_enabled(!self.connecting, egui::Button::new(label))
                            .clicked()
                        {
                            self.connect_or_disconnect();
                        }
                        ui.label(&self.status);
                    });
                });
            });

        egui::Panel::left("left_panel")
            .resizable(true)
            .default_size(280.0)
            .size_range(180.0..=420.0)
            .show_collapsible(ui, &mut true, |ui| {
                ui.heading(format!(
                    "Commands ({}/{})",
                    self.filtered_indices.len(),
                    self.commands.len()
                ));

                let search_response = ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search command / id / menu…")
                        .desired_width(f32::INFINITY),
                );

                let disabled_response =
                    ui.checkbox(&mut self.show_disabled, "Show disabled commands");

                // Refresh the cached indexes only when the filter inputs change.
                if search_response.changed() || disabled_response.changed() {
                    self.refresh_filter();
                }

                ui.separator();

                // Virtualize the list: egui lays out only the rows visible in
                // the scroll viewport, rather than all 900+ commands.
                const COMMAND_ROW_HEIGHT: f32 = 24.0;
                let mut clicked = None;
                let selected_command = self.selected_command;
                let commands = &self.commands;
                let filtered_indices = &self.filtered_indices;

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(
                        ui,
                        COMMAND_ROW_HEIGHT,
                        filtered_indices.len(),
                        |ui, visible_rows| {
                            for row in visible_rows {
                                let command_index = filtered_indices[row];
                                let cmd = &commands[command_index];
                                let selected = command_index == selected_command;
                                let label = if cmd.enabled {
                                    let path = cmd.menu.join("\\");
                                    if path.is_empty() {
                                        format!("{}", cmd.label)
                                    } else {
                                        format!("{}\t{}", cmd.label, path)
                                    }
                                } else {
                                    format!("{} [DISABLE]", cmd.label)
                                };

                                let row_width = ui.available_width();

                                let response = ui.add_sized(
                                    [row_width, COMMAND_ROW_HEIGHT],
                                    egui::Button::selectable(selected, label),
                                );

                                if response.clicked() {
                                    clicked = Some(command_index);
                                }
                            }
                        },
                    );

                if let Some(index) = clicked {
                    self.selected_command_changed(index);
                }
            });

        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(cmd) = self.commands.get(self.selected_command).cloned() {
                ui.heading(&cmd.label);
                ui.monospace(&cmd.id);
                ui.horizontal_wrapped(|ui| {
                    if !cmd.menu.is_empty() { ui.label(format!("Menu: {}", cmd.menu.join(" → "))); }
                    if let Some(shortcut) = &cmd.shortcut { ui.label(format!("Shortcut: {shortcut}")); }
                    ui.label(if cmd.needs_document { "Requires document" } else { "No document requirement" });
                    if !cmd.enabled { ui.colored_label(egui::Color32::YELLOW, "Currently disabled"); }
                });
                if let Some(reason) = &cmd.disabled_reason { ui.label(format!("Disabled reason: {reason}")); }
                ui.collapsing("Parameter schema", |ui| { ui.label(&cmd.params_schema); });
                ui.horizontal(|ui| {
                    if ui.button("Reset to command defaults").clicked() { self.fill_selected_command(); }
                    if ui.button("Copy command ID").clicked() { ui.ctx().copy_text(cmd.id.clone()); }
                });
            }
            ui.separator();
            ui.label("Request JSON (editable). Defaults are generated from the command catalog; required fields without documented defaults use safe placeholders.");
            ui.add(egui::TextEdit::multiline(&mut self.request_text).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(12));
            ui.horizontal(|ui| {
                if ui.add_enabled(self.connected, egui::Button::new("Send request")).clicked() { self.send_request(); }
                if ui.button("Clear response log").clicked() { self.output.clear(); }
            });
            ui.separator();
            ui.label("Response log");
            egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut self.output).desired_width(f32::INFINITY).desired_rows(10).font(egui::TextStyle::Monospace).interactive(false));
            });
        });
    }
}

fn connect_worker(
    exe_path: PathBuf,
    token_path: PathBuf,
    port: u16,
    command_rx: Receiver<WorkerCommand>,
    event_tx: Sender<UiEvent>,
) -> Result<(), String> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let _ = event_tx.send(UiEvent::Status(format!(
        "Checking existing PhotoCraft at {address}…"
    )));
    // If a server is already listening, reuse it. Never launch a duplicate merely
    // because its token file is missing or invalid.
    match connect_socket(address) {
        Ok(stream) => {
            let token = read_token(&token_path).map_err(|e| format!("PhotoCraft is already listening at {address}, but authentication credentials are unavailable: {e}"))?;
            return authenticated_session(stream, &token, command_rx, event_tx);
        }
        Err(_) => {} // no listener; launch below
    }
    if !exe_path.is_file() {
        return Err(format!(
            "PhotoCraft executable not found at '{}'. Set PHOTOCRAFT_EXE or put photocraft.exe beside this app.",
            exe_path.display()
        ));
    }
    let _ = event_tx.send(UiEvent::Status(format!(
        "Launching {}…",
        exe_path.display()
    )));
    Command::new(&exe_path)
        .arg("--control")
        .arg(port.to_string())
        .arg("--control-token-file")
        .arg(&token_path)
        .spawn()
        .map_err(|e| format!("could not launch '{}': {e}", exe_path.display()))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut last_error = "waiting for control server".to_owned();
    while Instant::now() < deadline {
        if let Ok(token) = read_token(&token_path) {
            match connect_socket(address) {
                Ok(stream) => return authenticated_session(stream, &token, command_rx, event_tx),
                Err(e) => last_error = e,
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err(format!(
        "timed out waiting for PhotoCraft at {address}: {last_error}"
    ))
}
fn read_token(path: &PathBuf) -> Result<String, String> {
    let token = fs::read_to_string(path)
        .map_err(|e| format!("cannot read token file '{}': {e}", path.display()))?;
    let token = token.trim().to_owned();
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!(
            "token file '{}' must contain a 64-character hexadecimal token",
            path.display()
        ));
    }
    Ok(token)
}
fn connect_socket(address: SocketAddr) -> Result<TcpStream, String> {
    let stream = TcpStream::connect_timeout(&address, Duration::from_millis(400))
        .map_err(|e| format!("cannot connect to {address}: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(65)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;
    Ok(stream)
}
fn authenticated_session(
    mut stream: TcpStream,
    token: &str,
    command_rx: Receiver<WorkerCommand>,
    event_tx: Sender<UiEvent>,
) -> Result<(), String> {
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let auth = json!({"id":"auth","method":"auth","params":{"token":token}});
    write_json_line(&mut stream, &auth)?;
    let reply = read_json_line(&mut reader)?;
    let value: Value =
        serde_json::from_str(&reply).map_err(|e| format!("invalid auth response: {e}; {reply}"))?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(format!("PhotoCraft rejected authentication: {reply}"));
    }
    event_tx
        .send(UiEvent::Authenticated)
        .map_err(|_| "UI closed".to_owned())?;
    loop {
        match command_rx.recv() {
            Ok(WorkerCommand::Send(line)) => {
                let value: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
                write_json_line(&mut stream, &value)?;
                let reply = read_json_line(&mut reader)?;
                let pretty = serde_json::from_str::<Value>(&reply)
                    .ok()
                    .and_then(|v| serde_json::to_string_pretty(&v).ok())
                    .unwrap_or(reply);
                event_tx
                    .send(UiEvent::Reply(pretty))
                    .map_err(|_| "UI closed".to_owned())?;
            }
            Ok(WorkerCommand::Disconnect) => {
                let _ = event_tx.send(UiEvent::Disconnected("closed by user".into()));
                return Ok(());
            }
            Err(_) => {
                let _ = event_tx.send(UiEvent::Disconnected("client closed".into()));
                return Ok(());
            }
        }
    }
}
fn write_json_line(stream: &mut TcpStream, value: &Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    stream
        .write_all(&bytes)
        .map_err(|e| format!("TCP write failed: {e}"))?;
    stream.flush().map_err(|e| format!("TCP flush failed: {e}"))
}
fn read_json_line(reader: &mut BufReader<TcpStream>) -> Result<String, String> {
    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .map_err(|e| format!("TCP read failed: {e}"))?;
    if n == 0 {
        return Err("PhotoCraft closed the connection".into());
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([780.0, 520.0])
            .with_title("PhotoCraft Command Explorer"),
        ..Default::default()
    };
    eframe::run_native(
        "PhotoCraft Command Explorer",
        options,
        Box::new(|_cc| Ok(Box::new(ControlApp::default()))),
    )
}
