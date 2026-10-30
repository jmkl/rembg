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

use serde::Deserialize;
use serde_json::{Value, json};
pub enum WorkerCommand {
    Send(String),
    Disconnect,
}
pub enum UiEvent {
    Status(String),
    Authenticated,
    Reply(String),
    Disconnected(String),
    Failed(String),
}

pub fn connect_worker(
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
