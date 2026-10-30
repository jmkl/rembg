//! Multiline text -> rows (text, font size, color) -> PhotoCraft text layers `layer{index}`.
//!
//! One sync pass (runs 300 ms after the last edit):
//!   1. engine.execute document.inspect        -> which layer names exist
//!   2. for each row i, strictly one request at a time:
//!        exists:  select.findLayers {name}  -> type.edit {text}
//!        missing: <create text layer> -> layer.setProps {name} -> type.edit {text}
//!
//! Protocol (from PhotoCraft docs/control-protocol.md):
//!   request : {"id":1,"method":"engine.execute","params":{"command":"...","params":{...}}}
//!   reply   : {"id":1,"ok":true,"result":{...}}  or  {"id":1,"ok":false,"error":"..."}
//!   auth    : {"id":"auth","method":"auth","params":{"token":"<hex>"}}  (first request)

use std::{
    collections::HashSet,
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32};
use serde_json::{Value, json};

// ---- commands -------------------------------------------------------------
const C_INSPECT: &str = "document.inspect";
const C_FIND: &str = "select.findLayers";
const C_SET_STYLE: &str = "type.setStyle";
const C_EDIT_TEXT: &str = "type.edit";
const C_SET_PROPS: &str = "layer.setProps";
/// Creates a NEW text layer. Params: {orientation, size, text, x, y}.
const C_NEW_TEXT_LAYER: &str = "type.create";
/// Vertical position of row i's new layer: y = i * LINE_SPACING (document pixels).
const LINE_SPACING: f32 = 80.0;
/// Command that sets font size / color on the active text layer, if there is one.
/// `None` = skip styling (only the text is synced). Params sent: {"fontSize","color"}.
// const C_STYLE: Option<&str> = None;

const ADDR: &str = "127.0.0.1:7878";
const DEBOUNCE: Duration = Duration::from_millis(300);

// ---------------------------------------------------------------------------
// TCP worker thread
// ---------------------------------------------------------------------------

enum ToWorker {
    Connect(String),
    Send(Value),
}

enum FromWorker {
    Connected,
    Disconnected(String),
    Response(Value),
}

fn spawn_worker(ctx: egui::Context) -> (mpsc::Sender<ToWorker>, mpsc::Receiver<FromWorker>) {
    let (tx_cmd, rx_cmd) = mpsc::channel::<ToWorker>();
    let (tx_evt, rx_evt) = mpsc::channel::<FromWorker>();

    thread::spawn(move || {
        let emit = |e: FromWorker| {
            let _ = tx_evt.send(e);
            ctx.request_repaint(); // wake egui, otherwise it may not redraw
        };
        let mut conn: Option<(TcpStream, BufReader<TcpStream>)> = None;

        for cmd in rx_cmd {
            match cmd {
                ToWorker::Connect(addr) => match TcpStream::connect(&addr) {
                    Ok(stream) => {
                        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
                        let reader = BufReader::new(stream.try_clone().expect("clone socket"));
                        conn = Some((stream, reader));
                        emit(FromWorker::Connected);
                    }
                    Err(e) => emit(FromWorker::Disconnected(e.to_string())),
                },
                ToWorker::Send(req) => {
                    let Some((w, r)) = conn.as_mut() else {
                        emit(FromWorker::Disconnected("not connected".into()));
                        continue;
                    };
                    let res = (|| -> std::io::Result<Value> {
                        let want = req["id"].clone();
                        writeln!(w, "{req}")?; // one JSON object per line
                        loop {
                            let mut line = String::new();
                            if r.read_line(&mut line)? == 0 {
                                return Err(std::io::ErrorKind::UnexpectedEof.into());
                            }
                            let v: Value = serde_json::from_str(&line)?;
                            if v["id"] == want {
                                return Ok(v); // skip stale replies with another id
                            }
                        }
                    })();
                    match res {
                        Ok(v) => emit(FromWorker::Response(v)),
                        Err(e) => {
                            conn = None;
                            emit(FromWorker::Disconnected(e.to_string()));
                        }
                    }
                }
            }
        }
    });

    (tx_cmd, rx_evt)
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Row {
    text: String,
    size: f32,
    color: Color32,
}

impl Default for Row {
    fn default() -> Self {
        Self {
            text: String::new(),
            size: 48.0,
            color: Color32::WHITE,
        }
    }
}

/// What we are currently waiting a reply for. `usize` = row index.
enum Phase {
    Idle,
    AwaitAuth,
    AwaitInspect,
    AwaitFind(usize),   // select.findLayers
    AwaitCreate(usize), // create text layer
    AwaitRename(usize), // layer.setProps {name}
    AwaitEdit(usize),   // type.edit
    AwaitStyle(usize),  // optional style command
}

struct App {
    tx: mpsc::Sender<ToWorker>,
    rx: mpsc::Receiver<FromWorker>,

    status: String,
    log: Vec<String>,
    connected: bool,

    input: String,
    rows: Vec<Row>,

    dirty_at: Option<Instant>, // debounce timer
    pending: bool,             // edited again while a sync was running
    phase: Phase,
    job: Vec<Row>,             // snapshot of rows for the running sync
    existing: HashSet<String>, // layer names found by document.inspect
    sync_failed: bool,
    next_id: u64,
}

impl App {
    fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = spawn_worker(ctx);
        let mut app = Self {
            tx,
            rx,
            status: "Disconnected".into(),
            log: Vec::new(),
            connected: false,
            input: "Hello\nWorld".into(),
            rows: Vec::new(),
            dirty_at: None,
            pending: false,
            phase: Phase::Idle,
            job: Vec::new(),
            existing: HashSet::new(),
            sync_failed: false,
            next_id: 0,
        };
        app.rebuild_rows();
        app
    }

    /// Line N of the text box <-> row N <-> layer "layerN".
    /// Blank lines are kept so indices stay stable. Size/color are kept by index.
    fn rebuild_rows(&mut self) {
        let lines: Vec<&str> = self.input.lines().collect();
        self.rows.resize_with(lines.len(), Row::default);
        for (row, line) in self.rows.iter_mut().zip(lines) {
            row.text = line.to_owned();
        }
    }

    fn push_log(&mut self, mut s: String) {
        if s.chars().count() > 300 {
            s = s.chars().take(300).collect::<String>() + "…";
        }
        self.log.push(s);
        if self.log.len() > 200 {
            self.log.drain(0..self.log.len() - 200);
        }
    }

    fn send_raw(&mut self, req: Value) {
        self.push_log(format!("-> {req}"));
        let _ = self.tx.send(ToWorker::Send(req));
    }

    /// engine.execute {command, params}
    fn exec(&mut self, command: &str, params: Value) {
        self.next_id += 1;
        self.send_raw(json!({
            "id": self.next_id,
            "method": "engine.execute",
            "params": { "command": command, "params": params }
        }));
    }

    // ---- connection ----

    fn connect(&mut self) {
        self.status = "Connecting...".into();
        let _ = self.tx.send(ToWorker::Connect(ADDR.into()));
    }

    fn read_token() -> String {
        let token_path = std::env::var_os("PHOTOCRAFT_CONTROL_TOKEN_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join("photocraft-control.token")
            });
        if let Ok(s) = std::fs::read_to_string(token_path) {
            return s.trim().to_owned();
        }
        std::env::var("PHOTOCRAFT_CONTROL_TOKEN").unwrap_or_default()
    }

    fn on_connected(&mut self) {
        self.status = "Authenticating...".into();
        self.phase = Phase::AwaitAuth;
        let token = Self::read_token();
        self.send_raw(json!({ "id": "auth", "method": "auth", "params": { "token": token } }));
    }

    // ---- sync state machine ----

    fn start_sync(&mut self) {
        self.dirty_at = None;
        self.pending = false;
        self.sync_failed = false;
        self.status = "Syncing...".into();
        self.job = self.rows.clone();
        self.existing.clear();
        self.phase = Phase::AwaitInspect;
        self.exec(C_INSPECT, json!({}));
    }

    /// Decide what to do for row `i`: layer exists -> select it, else create it.
    fn begin_row(&mut self, i: usize) {
        if i >= self.job.len() {
            return self.finish();
        }
        let name = format!("layer{i}");
        if self.existing.contains(&name) {
            self.exec(C_FIND, json!({ "name": name })); // selects the layer
            self.phase = Phase::AwaitFind(i);
        } else {
            // the text and size go in at creation, so no type.edit is needed afterwards
            let row = self.job[i].clone();
            self.exec(
                C_NEW_TEXT_LAYER,
                json!({
                    "orientation": "horizontal",
                    "size": row.size,
                    "text": row.text,
                    "font":"Uni Sans",
                    "fontStyle":"Heavy CAPS",
                    "weight": 900,
                    "postscript_name":"UniSansHeavyCAPS",
                    "caps":"Normal",
                    "x": 100,
                    "y": 100 + (i as f32 * LINE_SPACING) as i64,
                }),
            );
            self.phase = Phase::AwaitCreate(i);
        }
    }

    fn edit_row(&mut self, i: usize) {
        let text = self.job[i].text.clone();
        self.exec(C_EDIT_TEXT, json!({ "text": text })); // edits the active text layer
        self.phase = Phase::AwaitEdit(i);
    }

    /// Text is in place: apply the optional style command, then move to the next row.
    fn after_text(&mut self, i: usize) {
        let row = self.job[i].clone();
        self.exec(
            C_SET_STYLE,
            json!({ "size": row.size, "color": hex(row.color) }),
        );
        self.phase = Phase::AwaitStyle(i);
    }

    fn fail_row(&mut self, i: usize) {
        self.sync_failed = true;
        self.begin_row(i + 1);
    }

    fn finish(&mut self) {
        self.phase = Phase::Idle;
        self.status = if self.sync_failed {
            "Synced with errors (see log)".into()
        } else {
            "Synced".into()
        };
        if self.pending {
            self.start_sync(); // edited while busy: go again with the latest rows
        }
    }

    fn on_response(&mut self, v: Value) {
        self.push_log(format!("<- {v}"));
        let ok = v["ok"].as_bool().unwrap_or(false);

        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::AwaitAuth => {
                if ok {
                    self.connected = true;
                    self.status = "Connected".into();
                    self.dirty_at = Some(Instant::now()); // push current rows
                } else {
                    self.status = "Authentication failed".into();
                }
            }
            Phase::AwaitInspect => {
                if !ok {
                    self.status = "document.inspect failed (see log)".into();
                    return;
                }
                collect_names(&v["result"], &mut self.existing);
                self.begin_row(0);
            }
            Phase::AwaitFind(i) => {
                if ok {
                    self.edit_row(i)
                } else {
                    self.fail_row(i)
                }
            }
            Phase::AwaitCreate(i) => {
                if ok {
                    // name the freshly created (active) layer
                    let name = format!("layer{i}");
                    self.exec(C_SET_PROPS, json!({ "name": name }));
                    self.phase = Phase::AwaitRename(i);
                } else {
                    self.fail_row(i);
                }
            }
            Phase::AwaitRename(i) => {
                if ok {
                    self.existing.insert(format!("layer{i}"));
                    self.after_text(i); // text was already set by type.create
                } else {
                    self.fail_row(i);
                }
            }
            Phase::AwaitEdit(i) => {
                if ok {
                    self.after_text(i)
                } else {
                    self.fail_row(i)
                }
            }
            Phase::AwaitStyle(i) => {
                if !ok {
                    self.sync_failed = true;
                }
                self.begin_row(i + 1);
            }
            Phase::Idle => {}
        }
    }
}

/// Collect every `"name": "<string>"` found anywhere in the JSON (shape-agnostic,
/// so nested layer groups work without knowing the exact tree layout).
fn collect_names(v: &Value, out: &mut HashSet<String>) {
    match v {
        Value::Object(m) => {
            if let Some(Value::String(s)) = m.get("name") {
                out.insert(s.clone());
            }
            for x in m.values() {
                collect_names(x, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                collect_names(x, out);
            }
        }
        _ => {}
    }
}

fn hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

// ---------------------------------------------------------------------------
// UI
// ---------------------------------------------------------------------------

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        while let Ok(evt) = self.rx.try_recv() {
            match evt {
                FromWorker::Connected => self.on_connected(),
                FromWorker::Disconnected(e) => {
                    self.connected = false;
                    self.phase = Phase::Idle;
                    self.status = format!("Disconnected: {e}");
                }
                FromWorker::Response(v) => self.on_response(v),
            }
        }

        // 2. draw
        let mut changed = false;

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let dot = if self.connected {
                    Color32::GREEN
                } else {
                    Color32::RED
                };
                ui.colored_label(dot, "●");
                ui.label(&self.status);
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            if ui.button("Connect").clicked() {
                self.connect();
            }

            ui.add_space(6.0);
            let resp = ui.add(
                egui::TextEdit::multiline(&mut self.input)
                    .desired_rows(6)
                    .desired_width(f32::INFINITY)
                    .hint_text("One line = one text layer"),
            );
            if resp.changed() {
                self.rebuild_rows();
                changed = true;
            }

            ui.add_space(6.0);
            ui.separator();

            for (i, row) in self.rows.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("layer{i}"));
                    ui.add_sized(
                        [ui.available_width() - 130.0, 20.0],
                        egui::Label::new(&row.text).truncate(),
                    );
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut row.size)
                                .range(1.0..=500.0)
                                .speed(1.0),
                        )
                        .changed();
                    changed |= ui.color_edit_button_srgba(&mut row.color).changed();
                });
            }

            ui.add_space(6.0);
            ui.separator();
            ui.label("Log");
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for l in &self.log {
                        ui.monospace(l);
                    }
                });
        });

        // 3. debounce: fire DEBOUNCE after the last change
        if changed {
            self.dirty_at = Some(Instant::now());
        }
        if let Some(t) = self.dirty_at {
            if t.elapsed() >= DEBOUNCE {
                if !self.connected {
                    self.dirty_at = None; // will resync right after connecting
                } else if matches!(self.phase, Phase::Idle) {
                    self.start_sync();
                } else {
                    self.pending = true; // run again when the current sync ends
                    self.dirty_at = None;
                }
            } else {
                ui.ctx().request_repaint_after(DEBOUNCE - t.elapsed());
            }
        }
    }
}

fn main() -> eframe::Result {
    eframe::run_native(
        "PhotoCraft Layer Sync",
        eframe::NativeOptions::default(),
        Box::new(|cc| Ok(Box::new(App::new(cc.egui_ctx.clone())))),
    )
}
