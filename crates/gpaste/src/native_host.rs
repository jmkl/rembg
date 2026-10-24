use serde_json::Value;
use std::{
    collections::VecDeque,
    io::{self, BufRead, BufReader, Read, Write},
    net::{Shutdown, TcpStream, ToSocketAddrs},
    sync::mpsc,
    thread,
    time::Duration,
};

const ADDRESS: &str = "127.0.0.1:38471";
const MAX_NATIVE_MESSAGE: usize = 64 * 1024 * 1024;
const NATIVE_CHUNK_SIZE: usize = 500_000;
const CHUNK_IMAGE_THRESHOLD: usize = 700_000;

enum Event {
    FromChrome(Value),
    FromGui(u64, String),
    GuiDisconnected(u64),
    ChromeDisconnected,
}

fn read_native<R: Read>(reader: &mut R) -> io::Result<Option<Value>> {
    let mut header = [0u8; 4];
    let mut got = 0;

    while got < 4 {
        match reader.read(&mut header[got..])? {
            0 if got == 0 => return Ok(None),
            0 => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated native message header",
                ));
            }
            n => got += n,
        }
    }

    let len = u32::from_le_bytes(header) as usize;
    if len == 0 || len > MAX_NATIVE_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid native message length: {len}"),
        ));
    }

    let mut payload = vec![0; len];
    reader.read_exact(&mut payload)?;
    serde_json::from_slice(&payload)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn write_native<W: Write>(writer: &mut W, value: &Value) -> io::Result<()> {
    let payload = serde_json::to_vec(value).map_err(io::Error::other)?;
    if payload.len() > 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "native host message exceeds 1 MiB",
        ));
    }

    writer.write_all(&(payload.len() as u32).to_le_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()
}

// Chrome limits each native-host -> extension message to 1 MiB.
// Large image data URLs are split into smaller messages and reassembled by background.js.
fn write_gui_value_to_chrome<W: Write>(writer: &mut W, value: Value) -> io::Result<()> {
    let is_send_prompt = value.get("type").and_then(Value::as_str) == Some("send_prompt");

    // Borrow the data URL instead of cloning it (it can be several MiB).
    let large_data_url = value
        .get("image_data_url")
        .and_then(Value::as_str)
        .filter(|data| is_send_prompt && data.len() > CHUNK_IMAGE_THRESHOLD);

    let Some(data_url) = large_data_url else {
        return write_native(writer, &value);
    };

    let (prefix, encoded) = data_url.split_once(',').ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "image_data_url is not a valid data URL",
        )
    })?;

    let request_id = value
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "send_prompt is missing request_id",
            )
        })?;

    let text = value.get("text").unwrap_or(&Value::Null);
    let image_name = value.get("image_name").unwrap_or(&Value::Null);

    write_native(
        writer,
        &serde_json::json!({
            "type": "send_prompt_start",
            "request_id": request_id,
            "text": if text.is_null() { Value::String(String::new()) } else { text.clone() },
            "image_name": image_name,
            "image_data_url_prefix": format!("{prefix},"),
        }),
    )?;

    for (index, chunk) in encoded.as_bytes().chunks(NATIVE_CHUNK_SIZE).enumerate() {
        // Base64 is pure ASCII, so every chunk boundary is a valid UTF-8 boundary.
        let chunk = std::str::from_utf8(chunk).map_err(io::Error::other)?;
        write_native(
            writer,
            &serde_json::json!({
                "type": "send_prompt_chunk",
                "request_id": request_id,
                "index": index,
                "data": chunk,
            }),
        )?;
    }

    write_native(
        writer,
        &serde_json::json!({
            "type": "send_prompt_end",
            "request_id": request_id,
            "chunk_count": encoded.len().div_ceil(NATIVE_CHUNK_SIZE),
        }),
    )
}

fn start_chrome_reader(tx: mpsc::Sender<Event>) {
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut input = stdin.lock();

        loop {
            match read_native(&mut input) {
                Ok(Some(value)) => {
                    if tx.send(Event::FromChrome(value)).is_err() {
                        return;
                    }
                }
                Ok(None) => {
                    // Chrome closes native messaging stdin when the extension
                    // disconnects/reloads. Tell the main thread to terminate.
                    let _ = tx.send(Event::ChromeDisconnected);
                    return;
                }
                Err(error) => {
                    let _ = tx.send(Event::FromChrome(serde_json::json!({
                        "type": "bridge_error",
                        "error": error.to_string()
                    })));
                    let _ = tx.send(Event::ChromeDisconnected);
                    return;
                }
            }
        }
    });
}
fn start_gui_reader(stream: TcpStream, tx: mpsc::Sender<Event>, connection_id: u64) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();

        loop {
            line.clear();

            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx
                        .send(Event::FromGui(connection_id, line.trim().to_owned()))
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }

        let _ = tx.send(Event::GuiDisconnected(connection_id));
    });
}
pub fn run() -> io::Result<()> {
    let (tx, rx) = mpsc::channel::<Event>();
    start_chrome_reader(tx.clone());

    let mut queued_from_chrome = VecDeque::<Value>::new();
    let mut connection_id = 0_u64;

    let stdout = io::stdout();
    let mut output = stdout.lock();

    loop {
        // Keep trying until the GUI becomes available.
        let mut stream = loop {
            // Chrome shutdown must be detected even while the GUI is offline.
            loop {
                match rx.try_recv() {
                    Ok(Event::ChromeDisconnected) => {
                        eprintln!("[GPasTe] Chrome disconnected; stopping host.");
                        return Ok(());
                    }
                    Ok(Event::FromChrome(value)) => {
                        queued_from_chrome.push_back(value);
                    }
                    Ok(Event::FromGui(_, _)) | Ok(Event::GuiDisconnected(_)) => {}
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                }
            }

            let address = match ADDRESS.to_socket_addrs() {
                Ok(mut addresses) => match addresses.next() {
                    Some(address) => address,
                    None => {
                        thread::sleep(Duration::from_millis(250));
                        continue;
                    }
                },
                Err(error) => {
                    eprintln!("[GPasTe] Address resolution failed: {error}");
                    thread::sleep(Duration::from_millis(250));
                    continue;
                }
            };

            match TcpStream::connect_timeout(&address, Duration::from_millis(500)) {
                Ok(stream) => {
                    eprintln!("[GPasTe] Connected to GUI at {ADDRESS}");
                    break stream;
                }
                Err(_) => {
                    // Avoid flooding the log while the GUI is closed.
                    thread::sleep(Duration::from_millis(250));
                }
            }
        };

        connection_id = connection_id.wrapping_add(1);
        let current_id = connection_id;

        if let Err(error) = stream.set_nodelay(true) {
            eprintln!("[GPasTe] TCP setup failed: {error}");
            let _ = stream.shutdown(Shutdown::Both);
            continue;
        }

        let reader_stream = match stream.try_clone() {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("[GPasTe] Cannot clone TCP stream: {error}");
                let _ = stream.shutdown(Shutdown::Both);
                continue;
            }
        };

        start_gui_reader(reader_stream, tx.clone(), current_id);

        // Process messages until either side disconnects.
        loop {
            let event = match queued_from_chrome.pop_front() {
                Some(value) => Event::FromChrome(value),
                None => match rx.recv() {
                    Ok(event) => event,
                    Err(_) => {
                        let _ = stream.shutdown(Shutdown::Both);
                        return Ok(());
                    }
                },
            };

            match event {
                Event::ChromeDisconnected => {
                    eprintln!("[GPasTe] Chrome disconnected; stopping host.");
                    let _ = stream.shutdown(Shutdown::Both);
                    return Ok(());
                }

                Event::FromChrome(value) => {
                    let mut bytes = serde_json::to_vec(&value).map_err(io::Error::other)?;
                    bytes.push(b'\n');

                    if let Err(error) = stream.write_all(&bytes) {
                        eprintln!("[GPasTe] GUI connection lost while sending: {error}");

                        // Retain the request for the next connection.
                        queued_from_chrome.push_front(value);
                        break;
                    }
                }

                Event::FromGui(id, line) if id == current_id => {
                    match serde_json::from_str::<Value>(&line) {
                        Ok(value) => {
                            if let Err(error) = write_gui_value_to_chrome(&mut output, value) {
                                eprintln!("[GPasTe] Failed to send to Chrome: {error}");
                                let _ = stream.shutdown(Shutdown::Both);
                                return Err(error);
                            }
                        }
                        Err(error) => {
                            eprintln!("[GPasTe] Invalid GUI message: {error}");
                        }
                    }
                }

                Event::GuiDisconnected(id) if id == current_id => {
                    eprintln!("[GPasTe] GUI disconnected.");
                    break;
                }

                // Ignore delayed events from previous TCP connections.
                Event::FromGui(_, _) | Event::GuiDisconnected(_) => {}
            }
        }

        let _ = stream.shutdown(Shutdown::Both);
        eprintln!("[GPasTe] Retrying GUI connection...");
        thread::sleep(Duration::from_millis(250));
    }
}
