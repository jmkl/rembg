#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use crossbeam_channel::unbounded;
use gpaste::{app, protocol};
use std::{env, net::TcpListener, thread, time::Duration};

const ADDRESS: &str = "127.0.0.1:38471";

fn main() -> eframe::Result<()> {
    let (to_gui_tx, to_gui_rx) = unbounded::<protocol::Message>();
    let (to_host_tx, to_host_rx) = unbounded::<protocol::Message>();
    start_bridge_server(to_gui_tx, to_host_rx);

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([760.0, 640.0])
            .with_min_inner_size([520.0, 420.0]),
        ..Default::default()
    };

    eframe::run_native(
        "ChatGPT Bridge",
        options,
        Box::new(move |_cc| Ok(Box::new(app::ChatGptApp::new(to_host_tx, to_gui_rx)))),
    )
}

fn start_bridge_server(
    incoming_tx: crossbeam_channel::Sender<protocol::Message>,
    outgoing_rx: crossbeam_channel::Receiver<protocol::Message>,
) {
    thread::spawn(move || {
        let listener = loop {
            match TcpListener::bind(ADDRESS) {
                Ok(listener) => break listener,
                Err(error) => {
                    eprintln!("Cannot bind {ADDRESS}: {error}; retrying…");
                    thread::sleep(Duration::from_secs(2));
                }
            }
        };
        eprintln!("Bridge listening on {ADDRESS}");

        loop {
            let (stream, peer) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) => {
                    eprintln!("TCP accept failed: {error}");
                    continue;
                }
            };
            eprintln!("Native host connected: {peer}");

            let mut writer = match stream.try_clone() {
                Ok(stream) => stream,
                Err(error) => {
                    eprintln!("Could not clone TCP stream: {error}");
                    continue;
                }
            };
            let read_stream = match stream.try_clone() {
                Ok(stream) => stream,
                Err(error) => {
                    eprintln!("Could not clone TCP stream: {error}");
                    continue;
                }
            };
            let read_tx = incoming_tx.clone();
            let reader_thread = thread::spawn(move || {
                use std::io::{BufRead, BufReader};
                let mut reader = BufReader::new(read_stream);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => match serde_json::from_str::<protocol::Message>(line.trim()) {
                            Ok(message) => {
                                if read_tx.send(message).is_err() {
                                    break;
                                }
                            }
                            Err(error) => eprintln!("Invalid message from host: {error}"),
                        },
                        Err(error) => {
                            eprintln!("TCP read failed: {error}");
                            break;
                        }
                    }
                }
            });

            let _ = incoming_tx.send(protocol::Message::BridgeStatus {
                connected: true,
                detail: "native host".into(),
            });

            loop {
                match outgoing_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(message) => {
                        use std::io::Write;
                        let mut bytes = match serde_json::to_vec(&message) {
                            Ok(bytes) => bytes,
                            Err(error) => {
                                eprintln!("Could not serialize message: {error}");
                                continue;
                            }
                        };
                        bytes.push(b'\n');
                        if let Err(error) = writer.write_all(&bytes) {
                            eprintln!("TCP write failed: {error}");
                            break;
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        if reader_thread.is_finished() {
                            break;
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                }
            }
            let _ = incoming_tx.send(protocol::Message::BridgeStatus {
                connected: false,
                detail: "native host disconnected".into(),
            });
        }
    });
}
