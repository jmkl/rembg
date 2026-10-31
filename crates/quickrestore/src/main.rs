use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use rembg_core::{FaceDetector, Gfpgan, RemoveBg};
use serde_json::Value;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

enum Cmd {
    SendText(String),
}

fn main() {
    // Locate quickrestore.json next to the executable
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let config_path = exe_dir.join("quickrestore.json");

    // Create default config if missing
    if !config_path.exists() {
        let default = serde_json::json!({ "model": "E:\\_CODE\\rust\\rembg\\models" });
        if let Ok(s) = serde_json::to_string_pretty(&default) {
            let _ = std::fs::write(&config_path, s);
        }
    }

    // Read model from config
    let mut model_root: Option<String> = None;
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(m) = v.get("model").and_then(|x| x.as_str()) {
                model_root = Some(m.to_string());
            }
        }
    }

    // Optional --model override
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
    println!("Using model root: {}", model_root);
    println!("Config: {}", config_path.display());

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async_main(model_root));
}

async fn async_main(model_root: String) {
    let (tx, rx) = mpsc::channel::<Cmd>(32);
    run_server(tx, rx, model_root).await;
}

async fn run_server(cmd_tx: mpsc::Sender<Cmd>, mut cmd_rx: mpsc::Receiver<Cmd>, model_root: String) {
    let listener = match TcpListener::bind("127.0.0.1:9001").await {
        Ok(l) => l,
        Err(e) => {
            println!("Bind failed: {}", e);
            return;
        }
    };
    println!("QuickRestore listening on ws://localhost:9001");

    type Sink = futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        Message,
    >;
    let sink_lock: Arc<tokio::sync::Mutex<Option<Sink>>> = Arc::new(tokio::sync::Mutex::new(None));

    // Command forwarder
    let sink_for_cmd = sink_lock.clone();
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            let text = match cmd {
                Cmd::SendText(t) => t,
            };
            if let Some(sink) = sink_for_cmd.lock().await.as_mut() {
                let _ = sink.send(Message::Text(text.into())).await;
            }
        }
    });

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                match accept_async(stream).await {
                    Ok(ws) => {
                        let (write, mut read) = ws.split();
                        *sink_lock.lock().await = Some(write);
                        println!("PS connected");
                        while let Some(Ok(msg)) = read.next().await {
                            if msg.is_text() {
                                handle_message(&cmd_tx, &model_root, msg.to_text().unwrap_or(""));
                            }
                        }
                        *sink_lock.lock().await = None;
                        println!("PS disconnected");
                    }
                    Err(e) => {
                        println!("WS accept error: {}", e);
                    }
                }
            }
            Err(e) => {
                println!("Accept error: {}", e);
            }
        }
    }
}

fn handle_message(cmd_tx: &mpsc::Sender<Cmd>, model_root: &str, text: &str) {
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            println!("raw: {}", text);
            return;
        }
    };
    match v.get("type").and_then(|x| x.as_str()) {
        Some("hello") => {
            println!("PS hello");
        }
        Some("selection") => {
            let w = v.get("width").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            let h = v.get("height").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            let components = v.get("components").and_then(|x| x.as_u64()).unwrap_or(3) as u8;
            let x = v.get("x").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
            let y = v.get("y").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
            let data_b64 = v.get("data").and_then(|x| x.as_str()).unwrap_or("");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data_b64)
                .unwrap_or_default();

            let path = save_png(w, h, components, &bytes);
            println!("Received selection {}x{} ({} bytes) saved {:?}", w, h, bytes.len(), path);

            let cmd_tx2 = cmd_tx.clone();
            let model_root2 = model_root.to_string();
            std::thread::spawn(move || {
                process_pipeline(cmd_tx2, &model_root2, x, y, w, h, components, bytes);
            });
        }
        Some("clipboard") => {
            let w = v.get("width").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            let h = v.get("height").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            let components = v.get("components").and_then(|x| x.as_u64()).unwrap_or(3) as u8;
            let data_b64 = v.get("data").and_then(|x| x.as_str()).unwrap_or("");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data_b64)
                .unwrap_or_default();
            // Convert to RGBA
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            if components >= 4 {
                rgba = bytes.clone();
            } else if components == 3 {
                for i in 0..(w * h) as usize {
                    let idx = i * 3;
                    rgba.push(bytes.get(idx).copied().unwrap_or(0));
                    rgba.push(bytes.get(idx + 1).copied().unwrap_or(0));
                    rgba.push(bytes.get(idx + 2).copied().unwrap_or(0));
                    rgba.push(255);
                }
            } else {
                for i in 0..(w * h) as usize {
                    let g = bytes.get(i).copied().unwrap_or(0);
                    rgba.push(g); rgba.push(g); rgba.push(g); rgba.push(255);
                }
            }
            // Set clipboard
            match arboard::Clipboard::new() {
                Ok(mut cb) => {
                    let img = arboard::ImageData {
                        width: w as usize,
                        height: h as usize,
                        bytes: std::borrow::Cow::from(rgba),
                    };
                    match cb.set_image(img) {
                        Ok(_) => println!("Copied {}x{} image to clipboard", w, h),
                        Err(e) => println!("Clipboard set_image failed: {}", e),
                    }
                }
                Err(e) => println!("Clipboard init failed: {}", e),
            }
        }
        Some("error") => {
            let msg = v.get("message").and_then(|x| x.as_str()).unwrap_or("unknown");
            println!("PS error: {}", msg);
        }
        _ => {
            println!("msg: {}", text);
        }
    }
}

fn process_pipeline(
    cmd_tx: mpsc::Sender<Cmd>,
    model_root: &str,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    components: u8,
    bytes: Vec<u8>,
) {
    let mut rgb = image::RgbImage::new(w.max(1), h.max(1));
    if components >= 3 {
        for yy in 0..h {
            for xx in 0..w {
                let idx = ((yy * w + xx) as usize) * (components as usize);
                if idx + 2 < bytes.len() {
                    rgb.put_pixel(xx, yy, image::Rgb([bytes[idx], bytes[idx + 1], bytes[idx + 2]]));
                }
            }
        }
    } else {
        for yy in 0..h {
            for xx in 0..w {
                let idx = ((yy * w + xx) as usize) * (components.max(1) as usize);
                let g = bytes.get(idx).copied().unwrap_or(0);
                rgb.put_pixel(xx, yy, image::Rgb([g, g, g]));
            }
        }
    }

    println!("Processing face restore...");
    let restored_rgb = match (
        FaceDetector::load_from(rembg_core::config::model_path(Some(model_root), rembg_core::config::MODEL_SCRFD_FILE)),
        Gfpgan::load_from(rembg_core::config::model_path(Some(model_root), rembg_core::config::MODEL_GFPGAN_FILE)),
    ) {
        (Ok(det), Ok(gfpgan)) => {
            let faces = det.detect(&rgb, 0.5, 0.4).unwrap_or_default();
            let lms: Vec<rembg_core::Landmarks> = faces.iter().map(|f| f.landmarks).collect();
            if lms.is_empty() {
                println!("No faces detected");
                rgb.clone()
            } else {
                println!("Detected {} face(s)", lms.len());
                gfpgan.restore_faces(&rgb, &lms, 1.0).unwrap_or_else(|_| rgb.clone())
            }
        }
        (Err(e), _) => { println!("FaceDetector load failed: {}", e); rgb.clone() }
        (_, Err(e)) => { println!("Gfpgan load failed: {}", e); rgb.clone() }
    };

    let fr_avg: u64 = restored_rgb
        .pixels()
        .map(|p| (p[0] as u64 + p[1] as u64 + p[2] as u64) / 3)
        .sum();
    let fr_avg = fr_avg / (restored_rgb.width() as u64 * restored_rgb.height() as u64).max(1);
    let _ = restored_rgb.save(std::env::temp_dir().join("quickrestore_face_restore.png"));
    send_image(&cmd_tx, "face_restore", restored_rgb.width(), restored_rgb.height(), 3, x, y, restored_rgb.as_raw());
    println!("Face restore sent (avg {})", fr_avg);

    // Remove background
    println!("Removing background...");
    let rgba = match RemoveBg::load_from(rembg_core::config::model_path(Some(model_root), rembg_core::config::MODEL_RMBG_FILE)) {
        Ok(rb) => {
            let dyn_img = image::DynamicImage::ImageRgb8(restored_rgb.clone());
            rb.process_image(&dyn_img).unwrap_or_else(|e| {
                println!("RemoveBg process failed: {}", e);
                let mut r = image::RgbaImage::new(restored_rgb.width(), restored_rgb.height());
                for (p, src) in r.pixels_mut().zip(restored_rgb.pixels()) {
                    *p = image::Rgba([src[0], src[1], src[2], 255]);
                }
                r
            })
        }
        Err(e) => {
            println!("RemoveBg load failed: {}", e);
            let mut r = image::RgbaImage::new(restored_rgb.width(), restored_rgb.height());
            for (p, src) in r.pixels_mut().zip(restored_rgb.pixels()) {
                *p = image::Rgba([src[0], src[1], src[2], 255]);
            }
            r
        }
    };

    let rb_avg: u64 = rgba
        .pixels()
        .map(|p| (p[0] as u64 + p[1] as u64 + p[2] as u64) / 3)
        .sum();
    let rb_avg = rb_avg / (rgba.width() as u64 * rgba.height() as u64).max(1);
    let _ = rgba.save(std::env::temp_dir().join("quickrestore_remove_bg.png"));
    send_image(&cmd_tx, "remove_bg", rgba.width(), rgba.height(), 4, x, y, rgba.as_raw());
    println!("Remove bg sent (avg {})", rb_avg);
}

fn send_image(cmd_tx: &mpsc::Sender<Cmd>, name: &str, w: u32, h: u32, components: u8, x: i32, y: i32, bytes: &[u8]) {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    let msg = serde_json::json!({
        "type": "image",
        "name": name,
        "width": w,
        "height": h,
        "components": components,
        "colorSpace": "RGB",
        "x": x,
        "y": y,
        "data": b64
    })
    .to_string();
    let _ = cmd_tx.blocking_send(Cmd::SendText(msg));
}

fn save_png(w: u32, h: u32, components: u8, bytes: &[u8]) -> Option<String> {
    if w == 0 || h == 0 {
        return None;
    }
    let path = std::env::temp_dir().join("quickrestore_selection.png");
    let img = match components {
        3 => {
            let buf = image::RgbImage::from_raw(w, h, bytes.to_vec())?;
            image::DynamicImage::ImageRgb8(buf)
        }
        4 => {
            let buf = image::RgbaImage::from_raw(w, h, bytes.to_vec())?;
            image::DynamicImage::ImageRgba8(buf)
        }
        _ => return None,
    };
    img.save(&path).ok()?;
    Some(path.to_string_lossy().to_string())
}
