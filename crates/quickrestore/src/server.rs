use anyhow::{Context, Result};
use base64::Engine;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{ImageBuffer, Rgba};
use rembg_core::{BirefNet, FaceDetector, Gfpgan, Gpen, Realesrgan, RemoveBg};
use serde::Deserialize;
use serde_json::Value;
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tungstenite::{Message, accept};

use crate::config::{BackgroundRemovalModel, Config, FaceRestoreModel, OperationOptions};
use crate::model::{Cmd, ProgressStatus, Status};
use crate::ps;

/// A connection is served by a single thread: it blocks in `read` for at most this
/// long, then drains whatever the UI queued for Photoshop.
const READ_TIMEOUT: Duration = Duration::from_millis(50);

/// Set while a request is being processed, so a second request does not start a
/// parallel pipeline that would duplicate the work and race on the output.
static BUSY: AtomicBool = AtomicBool::new(false);

/// Set by the abort button. It cancels the running pipeline at the next stage
/// boundary and drops a selection that is still on its way from Photoshop.
static ABORTED: AtomicBool = AtomicBool::new(false);

/// True once the user asked to abort the current operation.
fn aborted() -> bool {
    ABORTED.load(Ordering::SeqCst)
}

/// Ends the current request early if the abort button was pressed.
fn stop_if_aborted(status: &mpsc::Sender<ProgressStatus>, cmd_tx: &SyncSender<Cmd>) -> bool {
    if !aborted() {
        return false;
    }
    set_status(status, ps!(Status::Error, "aborted"));
    // Let Photoshop drop its own "processing" state/lock.
    let _ = cmd_tx.try_send(Cmd::SendText(
        serde_json::json!({ "type": "abort" }).to_string(),
    ));
    true
}

/// The ONNX sessions take seconds to create, so they are loaded once and then
/// reused for every request.
struct Models {
    face: Option<FaceDetector>,
    gfpgan: Option<Gfpgan>,
    gpen: Option<Gpen>,
    realesrgan: Option<Realesrgan>,
    remove_bg: Option<RemoveBg>,
    birefnet: Option<BirefNet>,
}

#[derive(Deserialize)]
pub struct CanvasImage {
    pub base64: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
}

static MODELS: LazyLock<Mutex<Models>> = LazyLock::new(|| {
    Mutex::new(Models {
        face: None,
        gfpgan: None,
        gpen: None,
        realesrgan: None,
        remove_bg: None,
        birefnet: None,
    })
});

/// Pipeline settings pushed by the UI. Read once per request.
static OPTIONS: LazyLock<Mutex<OperationOptions>> =
    LazyLock::new(|| Mutex::new(OperationOptions::default()));

impl Models {
    /// Load the models needed by `options`, dropping the alternatives so only
    /// one face-restore and one background-removal session stays in VRAM.
    fn ensure_loaded(&mut self, model_root: &str, options: &OperationOptions) {
        match options.face_restore {
            FaceRestoreModel::Gfpgan => {
                self.gpen = None;
                if self.gfpgan.is_none() {
                    match Gfpgan::load_from(rembg_core::config::model_path(
                        Some(model_root),
                        rembg_core::config::MODEL_GFPGAN_FILE,
                    )) {
                        Ok(m) => self.gfpgan = Some(m),
                        Err(e) => eprintln!("Gfpgan load failed: {}", e),
                    }
                }
            }
            FaceRestoreModel::Gpen => {
                self.gfpgan = None;
                if self.gpen.is_none() {
                    match Gpen::load_from(rembg_core::config::model_path(
                        Some(model_root),
                        rembg_core::config::MODEL_GPEN_FILE,
                    )) {
                        Ok(m) => self.gpen = Some(m),
                        Err(e) => eprintln!("Gpen load failed: {}", e),
                    }
                }
            }
        }
        if self.face.is_none() {
            match FaceDetector::load_from(rembg_core::config::model_path(
                Some(model_root),
                rembg_core::config::MODEL_SCRFD_FILE,
            )) {
                Ok(m) => self.face = Some(m),
                Err(e) => eprintln!("FaceDetector load failed: {}", e),
            }
        }
        if options.upscale {
            if self.realesrgan.is_none() {
                match Realesrgan::load_from(rembg_core::config::model_path(
                    Some(model_root),
                    rembg_core::config::MODEL_REALESRGAN_FILE,
                )) {
                    Ok(m) => self.realesrgan = Some(m),
                    Err(e) => eprintln!("Realesrgan load failed: {}", e),
                }
            }
        } else {
            self.realesrgan = None;
        }
        match options.background_removal {
            BackgroundRemovalModel::Rmbg => {
                self.birefnet = None;
                if self.remove_bg.is_none() {
                    match RemoveBg::load_from(rembg_core::config::model_path(
                        Some(model_root),
                        rembg_core::config::MODEL_RMBG_FILE,
                    )) {
                        Ok(m) => self.remove_bg = Some(m),
                        Err(e) => eprintln!("RemoveBg load failed: {}", e),
                    }
                }
            }
            BackgroundRemovalModel::Birefnet => {
                self.remove_bg = None;
                if self.birefnet.is_none() {
                    match BirefNet::load_from(rembg_core::config::model_path(
                        Some(model_root),
                        rembg_core::config::MODEL_BIREFNET_FILE,
                    )) {
                        Ok(m) => self.birefnet = Some(m),
                        Err(e) => eprintln!("BirefNet load failed: {}", e),
                    }
                }
            }
        }
    }
}

/// Clears the busy flag when the pipeline ends, also when it panics.
struct BusyGuard;

impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::SeqCst);
        // The pipeline consumed any abort request; the next one starts clean.
        ABORTED.store(false, Ordering::SeqCst);
    }
}

pub struct Server {}
impl Server {
    /// Cancels the running pipeline and any selection still in flight.
    pub fn abort() {
        ABORTED.store(true, Ordering::SeqCst);
    }

    /// Clears a stale abort before starting a new request.
    pub fn clear_abort() {
        ABORTED.store(false, Ordering::SeqCst);
    }

    /// Replaces the pipeline settings used by the next request.
    pub fn set_options(options: OperationOptions) {
        if let Ok(mut current) = OPTIONS.lock() {
            *current = options;
        }
    }

    fn options() -> OperationOptions {
        OPTIONS.lock().map(|current| *current).unwrap_or_default()
    }

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
                    // starting a second one. Don't touch ABORTED here: the
                    // running pipeline has to see it at its next checkpoint.
                    set_status(status, ps!(Status::Processing, "already processing"));
                    let _ = cmd_tx.try_send(Cmd::SendText(
                        serde_json::json!({ "type": "error", "message": "already processing" })
                            .to_string(),
                    ));
                    return;
                }
                // The selection was requested before the user hit abort: drop
                // it and tell Photoshop to drop its pending state as well.
                if ABORTED.swap(false, Ordering::SeqCst) {
                    BUSY.store(false, Ordering::SeqCst);
                    set_status(status, ps!(Status::Error, "aborted"));
                    let _ = cmd_tx.try_send(Cmd::SendText(
                        serde_json::json!({ "type": "abort" }).to_string(),
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
                let options = Server::options();
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
                        options,
                    );
                });
            }
            Some("status") => {
                let msg = v.get("message").and_then(|x| x.as_str()).unwrap_or("");
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
        options: OperationOptions,
    ) {
        set_status(&status, ps!(Status::Processing, "to rust"));
        if stop_if_aborted(&status, &cmd_tx) {
            return;
        }
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
        if stop_if_aborted(&status, &cmd_tx) {
            return;
        }
        let mut models = MODELS.lock().unwrap();
        models.ensure_loaded(model_root, &options);
        if stop_if_aborted(&status, &cmd_tx) {
            return;
        }

        if options.upscale {
            if let Some(realesrgan) = &models.realesrgan {
                set_status(&status, ps!(Status::Processing, "upscale"));
                rgb = realesrgan.upscale(&rgb).unwrap_or_else(|_| rgb.clone());
                if stop_if_aborted(&status, &cmd_tx) {
                    return;
                }
            }
        }

        set_status(&status, ps!(Status::Processing, "facerestore"));
        let restored_rgb = {
            let faces = match &models.face {
                Some(det) => det.detect(&rgb, 0.5, 0.4).unwrap_or_default(),
                None => Vec::new(),
            };
            if stop_if_aborted(&status, &cmd_tx) {
                return;
            }
            let lms: Vec<rembg_core::Landmarks> = faces.iter().map(|f| f.landmarks).collect();
            if lms.is_empty() {
                rgb.clone()
            } else {
                let restored = match options.face_restore {
                    FaceRestoreModel::Gfpgan => models
                        .gfpgan
                        .as_ref()
                        .and_then(|m| m.restore_faces(&rgb, &lms, 1.0).ok()),
                    FaceRestoreModel::Gpen => models
                        .gpen
                        .as_ref()
                        .and_then(|m| m.restore_faces(&rgb, &lms, 1.0).ok()),
                };
                restored.unwrap_or_else(|| rgb.clone())
            }
        };
        if stop_if_aborted(&status, &cmd_tx) {
            return;
        }

        set_status(&status, ps!(Status::Processing, "rembg"));
        let output_dir = Config::root_dir();

        let _counter = {
            use std::sync::atomic::{AtomicU64, Ordering};
            static C: AtomicU64 = AtomicU64::new(0);
            C.fetch_add(1, Ordering::Relaxed)
        };
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let face_path =
            output_dir.join(format!("quickrestore_face_restore_{}_{}.png", ts, _counter));
        if stop_if_aborted(&status, &cmd_tx) {
            return;
        }
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
        if stop_if_aborted(&status, &cmd_tx) {
            return;
        }

        let dyn_img = image::DynamicImage::ImageRgb8(restored_rgb.clone());
        let rgba = match options.background_removal {
            BackgroundRemovalModel::Rmbg => models
                .remove_bg
                .as_ref()
                .map(|rb| rb.process_image(&dyn_img)),
            BackgroundRemovalModel::Birefnet => models
                .birefnet
                .as_ref()
                .map(|bn| bn.process_image(&dyn_img)),
        }
        .and_then(Result::ok)
        .unwrap_or_else(|| Server::opaque(&restored_rgb));
        // BiRefNet clamps very wide inputs internally; keep the mask aligned
        // with the restored image that Photoshop is about to receive.
        let rgba = if rgba.dimensions() != restored_rgb.dimensions() {
            image::imageops::resize(
                &rgba,
                restored_rgb.width(),
                restored_rgb.height(),
                image::imageops::FilterType::Triangle,
            )
        } else {
            rgba
        };
        if stop_if_aborted(&status, &cmd_tx) {
            return;
        }

        let _ = rgba.save(output_dir.join("quickrestore_remove_bg.png"));
        if stop_if_aborted(&status, &cmd_tx) {
            return;
        }
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

    pub(crate) fn send_image(
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
    pub fn save_gpt_image(cmd_tx: &SyncSender<Cmd>, canvas_img: CanvasImage) -> Result<()> {
        let output_dir = Config::root_dir();
        let _counter = {
            use std::sync::atomic::{AtomicU64, Ordering};
            static C: AtomicU64 = AtomicU64::new(0);
            C.fetch_add(1, Ordering::Relaxed)
        };

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let out_path = output_dir.join(format!("gpt_{}_{}.png", ts, _counter));
        let buf = Server::decode_canvas_image(&canvas_img)?;
        let gpt_img_path = if buf.save(&out_path).is_ok() {
            Some(out_path.to_string_lossy().to_string())
        } else {
            eprintln!("failed to save face_restore png");
            None
        };
        Server::send_image(
            &cmd_tx,
            "gpt_image",
            buf.width(),
            buf.height(),
            3,
            0,
            0,
            Some(buf.width() as u32),
            Some(buf.height() as u32),
            gpt_img_path.as_deref(),
            buf.as_raw(),
        );
        Ok(())
    }
    fn decode_canvas_image(img: &CanvasImage) -> Result<ImageBuffer<Rgba<u8>, Vec<u8>>> {
        // 1. base64 -> PNG bytes
        let bytes = STANDARD
            .decode(img.base64.trim())
            .context("invalid base64")?;

        // 2. PNG bytes -> decoded image -> RGBA8 buffer
        let dynamic = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
            .context("failed to decode PNG")?;
        let buffer = dynamic.to_rgba8();

        // 3. optional sanity check against the reported size
        if buffer.width() != img.width || buffer.height() != img.height {
            anyhow::bail!(
                "size mismatch: got {}x{}, expected {}x{}",
                buffer.width(),
                buffer.height(),
                img.width,
                img.height
            );
        }

        Ok(buffer)
    }
}

fn set_status(status: &mpsc::Sender<ProgressStatus>, msg: ProgressStatus) {
    let _ = status.send(msg);
}
