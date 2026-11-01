use base64::Engine;
use rembg_core::{FaceDetector, Gfpgan, Realesrgan, RemoveBg};
use serde_json::Value;
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tungstenite::{Message, accept};

use crate::model::{Cmd, ProgressStatus, Status};
use crate::ps;

/// A connection is served by a single thread: it blocks in `read` for at most this
/// long, then drains whatever the UI queued for Photoshop.
const READ_TIMEOUT: Duration = Duration::from_millis(50);

/// Set while a request is being processed, so a second request does not start a
/// parallel pipeline that would duplicate the work and race on the output.
static BUSY: AtomicBool = AtomicBool::new(false);

/// The ONNX sessions take seconds to create, so they are loaded once and then
/// reused for every request.
struct Models {
    face: Option<FaceDetector>,
    gfpgan: Option<Gfpgan>,
    realesrgan: Option<Realesrgan>,
    remove_bg: Option<RemoveBg>,
}

static MODELS: LazyLock<Mutex<Models>> = LazyLock::new(|| {
    Mutex::new(Models {
        face: None,
        gfpgan: None,
        realesrgan: None,
        remove_bg: None,
    })
});

impl Models {
    /// Load every model that is not loaded yet.
    fn ensure_loaded(&mut self, model_root: &str) {
        if self.face.is_none() {
            match FaceDetector::load_from(rembg_core::config::model_path(
                Some(model_root),
                rembg_core::config::MODEL_SCRFD_FILE,
            )) {
                Ok(m) => self.face = Some(m),
                Err(e) => eprintln!("FaceDetector load failed: {}", e),
            }
        }
        if self.gfpgan.is_none() {
            match Gfpgan::load_from(rembg_core::config::model_path(
                Some(model_root),
                rembg_core::config::MODEL_GFPGAN_FILE,
            )) {
                Ok(m) => self.gfpgan = Some(m),
                Err(e) => eprintln!("Gfpgan load failed: {}", e),
            }
        }
        if self.remove_bg.is_none() {
            match RemoveBg::load_from(rembg_core::config::model_path(
                Some(model_root),
                rembg_core::config::MODEL_RMBG_FILE,
            )) {
                Ok(m) => self.remove_bg = Some(m),
                Err(e) => eprintln!("RemoveBg load failed: {}", e),
            }
        }
        if self.realesrgan.is_none() {
            match Realesrgan::load_from(rembg_core::config::model_path(
                Some(model_root),
                rembg_core::config::MODEL_REALESRGAN_FILE,
            )) {
                Ok(m) => self.realesrgan = Some(m),
                Err(e) => eprintln!("Realesrgan load failed: {}", e),
            }
        }
    }
}

/// Clears the busy flag when the pipeline ends, also when it panics.
struct BusyGuard;

impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::SeqCst);
    }
}

pub struct Server {}
impl Server {
    pub fn run(
        cmd_tx: SyncSender<Cmd>,
        cmd_rx: Receiver<Cmd>,
        model_root: String,
        status: mpsc::Sender<ProgressStatus>,
    ) {
        let listener = match TcpListener::bind("127.0.0.1:9001") {
            Ok(l) => l,
            Err(e) => {
                set_status(&status, ps!(format!("Bind failed: {}", e)));
                return;
            }
        };
        set_status(&status, ps!("Listening on ws://localhost:9001"));

        loop {
            match listener.accept() {
                Ok((stream, _)) => Server::serve(&cmd_tx, &cmd_rx, &model_root, &status, stream),
                Err(e) => set_status(&status, ps!(format!("Accept error: {}", e))),
            }
        }
    }

    fn serve(
        cmd_tx: &SyncSender<Cmd>,
        cmd_rx: &Receiver<Cmd>,
        model_root: &str,
        status: &mpsc::Sender<ProgressStatus>,
        stream: TcpStream,
    ) {
        if let Err(e) = stream.set_read_timeout(Some(READ_TIMEOUT)) {
            set_status(status, ps!(format!("Socket setup failed: {}", e)));
            return;
        }
        let mut ws = match accept(stream) {
            Ok(ws) => ws,
            Err(e) => {
                set_status(status, ps!(format!("WS accept error: {}", e)));
                return;
            }
        };
        set_status(status, ps!(Status::Idle, "PS connected"));

        loop {
            // Forward anything the UI queued for Photoshop.
            loop {
                match cmd_rx.try_recv() {
                    Ok(Cmd::SendText(text)) => {
                        if let Err(e) = ws.send(Message::text(text)) {
                            set_status(status, ps!(format!("Send error: {}", e)));
                            return;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }

            match ws.read() {
                Ok(Message::Text(text)) => {
                    Server::handle_message(cmd_tx, model_root, &text, status);
                }
                Ok(Message::Close(_)) => break,
                Ok(_) => {}
                // Idle: the read timeout fired, go drain the outgoing queue.
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(_) => break,
            }
        }
        set_status(status, ps!("PS disconnected"));
    }

    fn handle_message(
        cmd_tx: &SyncSender<Cmd>,
        model_root: &str,
        text: &str,
        status: &mpsc::Sender<ProgressStatus>,
    ) {
        let v: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(_) => {
                set_status(status, ps!(format!("raw: {}", text)));
                return;
            }
        };
        match v.get("type").and_then(|x| x.as_str()) {
            Some("selection") => {
                if BUSY.swap(true, Ordering::SeqCst) {
                    // A pipeline is still running. Tell Photoshop instead of
                    // starting a second one.
                    set_status(status, ps!(Status::Processing, "already processing"));
                    let _ = cmd_tx.try_send(Cmd::SendText(
                        serde_json::json!({ "type": "error", "message": "already processing" })
                            .to_string(),
                    ));
                    return;
                }
                set_status(status, ps!(Status::Processing, "selection"));
                let w = v.get("width").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                let h = v.get("height").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                let components = v.get("components").and_then(|x| x.as_u64()).unwrap_or(3) as u8;
                let x = v.get("x").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
                let y = v.get("y").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
                let data_b64 = v.get("data").and_then(|x| x.as_str()).unwrap_or("");
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data_b64)
                    .unwrap_or_default();
                let _path = Server::save_png(w, h, components, &bytes);

                let cmd_tx2 = cmd_tx.clone();
                let status = status.clone();
                let model_root2 = model_root.to_string();
                std::thread::spawn(move || {
                    // Releases the busy flag when the pipeline ends.
                    let _guard = BusyGuard;
                    Server::process_pipeline(
                        cmd_tx2,
                        status,
                        &model_root2,
                        x,
                        y,
                        w,
                        h,
                        components,
                        bytes,
                    );
                });
            }
            Some("status") => {
                let msg = v
                    .get("message")
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                set_status(status, ps!(Status::Processing, msg.to_string()));
            }
            Some("error") => {
                let msg = v
                    .get("message")
                    .and_then(|x| x.as_str())
                    .unwrap_or("unknown");
                set_status(status, ps!(format!("PS error: {}", msg)));
            }
            Some("done") => {
                set_status(status, ps!(Status::Idle, "DONE"));
            }
            _ => {
                set_status(status, ps!(format!("msg: {}", text)));
            }
        }
    }

    fn process_pipeline(
        cmd_tx: SyncSender<Cmd>,
        status: mpsc::Sender<ProgressStatus>,
        model_root: &str,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        components: u8,
        bytes: Vec<u8>,
    ) {
        set_status(&status, ps!(Status::Processing, "to rust"));
        let mut rgb = image::RgbImage::new(w.max(1), h.max(1));
        if components >= 3 {
            for yy in 0..h {
                for xx in 0..w {
                    let idx = ((yy * w + xx) as usize) * (components as usize);
                    if idx + 2 < bytes.len() {
                        rgb.put_pixel(
                            xx,
                            yy,
                            image::Rgb([bytes[idx], bytes[idx + 1], bytes[idx + 2]]),
                        );
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

        // The models are shared between requests: loading them takes seconds.
        let mut models = MODELS.lock().unwrap();
        models.ensure_loaded(model_root);

        let restored_rgb = match (&models.face, &models.gfpgan, &models.realesrgan) {
            (Some(det), Some(gfpgan), Some(realesrgan)) => {
                set_status(&status, ps!(Status::Processing, "upscale"));
                let upscaled = realesrgan.upscale(&rgb).unwrap_or_else(|_| rgb.clone());
                rgb = upscaled;
                set_status(&status, ps!(Status::Processing, "facerestore"));
                let faces = det.detect(&rgb, 0.5, 0.4).unwrap_or_default();
                let lms: Vec<rembg_core::Landmarks> = faces.iter().map(|f| f.landmarks).collect();
                if lms.is_empty() {
                    rgb.clone()
                } else {
                    gfpgan
                        .restore_faces(&rgb, &lms, 1.0)
                        .unwrap_or_else(|_| rgb.clone())
                }
            }
            _ => rgb.clone(),
        };

        set_status(&status, ps!(Status::Processing, "rembg"));

        let output_dir = std::env::var("LOCALAPPDATA")
            .map(|p| std::path::PathBuf::from(p).join("QuickRestore").join("output"))
            .unwrap_or_else(|_| std::env::temp_dir().join("quickrestore"));
        let _ = std::fs::create_dir_all(&output_dir);

        let _counter = {
            use std::sync::atomic::{AtomicU64, Ordering};
            static C: AtomicU64 = AtomicU64::new(0);
            C.fetch_add(1, Ordering::Relaxed)
        };
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let face_path = output_dir.join(format!("quickrestore_face_restore_{}_{}.png", ts, _counter));
        let face_path_str = if restored_rgb.save(&face_path).is_ok() {
            Some(face_path.to_string_lossy().to_string())
        } else {
            eprintln!("failed to save face_restore png");
            None
        };

        Server::send_image(
            &cmd_tx,
            "face_restore",
            restored_rgb.width(),
            restored_rgb.height(),
            3,
            x,
            y,
            Some(w as u32),
            Some(h as u32),
            face_path_str.as_deref(),
            restored_rgb.as_raw(),
        );

        let rgba = match &models.remove_bg {
            Some(rb) => {
                let dyn_img = image::DynamicImage::ImageRgb8(restored_rgb.clone());
                rb.process_image(&dyn_img)
                    .unwrap_or_else(|_| Server::opaque(&restored_rgb))
            }
            None => Server::opaque(&restored_rgb),
        };

        let _ = rgba.save(output_dir.join("quickrestore_remove_bg.png"));
        Server::send_image(
            &cmd_tx,
            "remove_bg",
            rgba.width(),
            rgba.height(),
            4,
            x,
            y,
            Some(w as u32),
            Some(h as u32),
            None,
            rgba.as_raw(),
        );

        set_status(&status, ps!(Status::Processing, "mask"));
    }

    /// The same image with every pixel fully opaque.
    fn opaque(rgb: &image::RgbImage) -> image::RgbaImage {
        let mut out = image::RgbaImage::new(rgb.width(), rgb.height());
        for (p, src) in out.pixels_mut().zip(rgb.pixels()) {
            *p = image::Rgba([src[0], src[1], src[2], 255]);
        }
        out
    }

    fn send_image(
        cmd_tx: &SyncSender<Cmd>,
        name: &str,
        w: u32,
        h: u32,
        components: u8,
        x: i32,
        y: i32,
        orig_w: Option<u32>,
        orig_h: Option<u32>,
        path: Option<&str>,
        bytes: &[u8],
    ) {
        let mut msg = serde_json::json!({
            "type": "image",
            "name": name,
            "width": w,
            "height": h,
            "components": components,
            "colorSpace": "RGB",
            "x": x,
            "y": y,
        });
        if let Some(ow) = orig_w {
            msg["origWidth"] = serde_json::Value::from(ow);
        }
        if let Some(oh) = orig_h {
            msg["origHeight"] = serde_json::Value::from(oh);
        }
        if let Some(p) = path {
            msg["path"] = serde_json::Value::String(p.to_string());
        } else {
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            msg["data"] = serde_json::Value::String(b64);
        }
        let msg = msg.to_string();
        // Never block here: the only drainer is the connection thread, so a pipeline
        // that finishes after a disconnect would otherwise park forever.
        let _ = cmd_tx.try_send(Cmd::SendText(msg));
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
}

fn set_status(status: &mpsc::Sender<ProgressStatus>, msg: ProgressStatus) {
    let _ = status.send(msg);
}
