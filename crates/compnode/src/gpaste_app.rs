use arboard::Clipboard;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use egui::{Sense, vec2};
use crate::style::controls::Button;
use crate::style::{font, icons, pal};
use gpaste::protocol::Message;
use image::{DynamicImage, ImageFormat, RgbaImage};
use std::{
    borrow::Cow,
    fs,
    io::Cursor,
    path::Path,
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

const PROMPTS_FILE: &str = "saved_prompts.json";
#[allow(dead_code)]
const MAX_PREVIEW: f32 = 300.0;
const SLOT_H: f32 = 160.0;
const SLOT_MIN_W: f32 = 110.0; // below this, the two holders stack vertically
struct AttachedImage {
    name: String,
    data_url: String,
    preview: egui::TextureHandle,
}

/// Result of decoding a generated image on a worker thread.
struct DecodedImage {
    width: usize,
    height: usize,
    raw: Vec<u8>,
    thumb: egui::ColorImage,
    clipboard_error: Option<String>,
    mime: String,
}

pub struct GPasTeApp {
    prompt: String,
    selected_image: Option<AttachedImage>,
    status: String,
    /// Id of the request we are currently waiting on. `Some` == pending.
    /// Messages for any other id (e.g. an aborted request) are ignored.
    active_request: Option<String>,
    /// Makes request ids unique across GUI restarts so a stale abort can't hit a new request.
    session: u128,
    tx: Sender<Message>,
    rx: Receiver<Message>,
    decoded_tx: Sender<(u64, Result<DecodedImage, String>)>,
    decoded_rx: Receiver<(u64, Result<DecodedImage, String>)>,
    decode_gen: u64,
    next_id: u64,
    generated_texture: Option<egui::TextureHandle>,
    generated_size: Option<[usize; 2]>,
    // Full-resolution RGBA pixels, kept so we can re-copy to the clipboard.
    generated_rgba: Option<(usize, usize, Vec<u8>)>,
    image_status: String,
    sidebar_open: bool,
    saved_prompts: Vec<String>,
}

fn tight_frame() -> egui::Frame {
    egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 2))
}

/// Scale `size` down (never up) so it fits inside max_w x max_h, keeping aspect ratio.
#[allow(dead_code)]
fn fit_size(size: egui::Vec2, max_w: f32, max_h: f32) -> egui::Vec2 {
    if size.x <= 0.0 || size.y <= 0.0 {
        return size;
    }
    let scale = (max_w / size.x).min(max_h / size.y).min(1.0);
    size * scale
}

enum CardAction {
    None,
    Open,
    Delete,
}

/// One-line, ellipsized text laid out inside `max_w`.
fn one_line(
    ui: &egui::Ui,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
    max_w: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, max_w);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    ui.painter().layout_job(job)
}

/// A compact card: title, subtitle, hover highlight, delete button on hover.
fn prompt_card(ui: &mut egui::Ui, index: usize, prompt: &str) -> CardAction {
    const H: f32 = 46.0;
    const PAD_X: f32 = 10.0;
    const DEL: f32 = 22.0;

    let mut action = CardAction::None;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), H), Sense::click());

    // Delete button zone (registered after the row, so it wins clicks on overlap).
    let del_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - PAD_X - DEL * 0.5, rect.center().y),
        vec2(DEL, DEL),
    );
    let del_resp = ui
        .interact(
            del_rect,
            ui.id().with(("del_prompt", index)),
            Sense::click(),
        )
        .on_hover_text("Delete");

    let hovered = resp.hovered() || del_resp.hovered();
    let v = ui.visuals();
    let bg = if hovered {
        v.widgets.hovered.bg_fill
    } else {
        v.widgets.inactive.weak_bg_fill
    };
    let border = if hovered {
        v.selection.bg_fill
    } else {
        v.widgets.noninteractive.bg_stroke.color
    };
    let text_col = v.text_color();
    let weak_col = v.weak_text_color();

    let p = ui.painter();
    p.rect_filled(rect, egui::CornerRadius::same(8), bg);
    p.rect_stroke(
        rect,
        egui::CornerRadius::same(8),
        egui::Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );

    // Title = first non-empty line, subtitle = next one (or a size hint).
    let mut lines = prompt.lines().map(str::trim).filter(|l| !l.is_empty());
    let title = lines.next().unwrap_or("(empty)");
    let subtitle = lines
        .next()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{} chars", prompt.chars().count()));

    let text_w = rect.width() - PAD_X * 2.0 - if hovered { DEL + 6.0 } else { 0.0 };
    let title_g = one_line(
        ui,
        title,
        egui::FontId::proportional(13.0),
        text_col,
        text_w,
    );
    let sub_g = one_line(
        ui,
        &subtitle,
        egui::FontId::proportional(11.0),
        weak_col,
        text_w,
    );

    let total_h = title_g.size().y + 2.0 + sub_g.size().y;
    let top = rect.center().y - total_h * 0.5;
    ui.painter().galley(
        egui::pos2(rect.left() + PAD_X, top),
        title_g.clone(),
        text_col,
    );
    ui.painter().galley(
        egui::pos2(rect.left() + PAD_X, top + title_g.size().y + 2.0),
        sub_g,
        weak_col,
    );

    // Delete "✕", only while the card is hovered.
    if hovered {
        if del_resp.hovered() {
            ui.painter()
                .circle_filled(del_rect.center(), DEL * 0.5, pal::DANGER_TINT);
        }
        ui.painter().text(
            del_rect.center(),
            egui::Align2::CENTER_CENTER,
            icons::X,
            icons::font(font::BUTTON),
            if del_resp.hovered() { pal::DANGER } else { weak_col },
        );
    }

    if resp.on_hover_text(prompt).clicked() && !del_resp.hovered() {
        action = CardAction::Open;
    }
    if del_resp.clicked() {
        action = CardAction::Delete;
    }
    action
}

/// First line of a prompt, shortened for the sidebar list.
#[allow(dead_code)]
fn prompt_title(prompt: &str) -> String {
    let first = prompt.lines().next().unwrap_or("").trim();
    let mut title: String = first.chars().take(36).collect();
    if first.chars().count() > 36 || prompt.lines().count() > 1 {
        title.push('…');
    }
    title
}

impl GPasTeApp {
    pub fn new(tx: Sender<Message>, rx: Receiver<Message>) -> Self {
        let (decoded_tx, decoded_rx) = crossbeam_channel::unbounded();
        Self {
            prompt: String::new(),
            selected_image: None,
            status: "Waiting for Chrome extension…".into(),
            active_request: None,
            session: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            tx,
            rx,
            decoded_tx,
            decoded_rx,
            decode_gen: 0,
            next_id: 1,
            generated_texture: None,
            generated_size: None,
            generated_rgba: None,
            image_status: String::new(),
            sidebar_open: true,
            saved_prompts: Self::load_prompts(),
        }
    }

    // ---------- saved prompts ----------

    fn load_prompts() -> Vec<String> {
        fs::read_to_string(PROMPTS_FILE)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
            .unwrap_or_default()
    }

    fn persist_prompts(&self) {
        match serde_json::to_string_pretty(&self.saved_prompts) {
            Ok(json) => {
                if let Err(error) = fs::write(PROMPTS_FILE, json) {
                    eprintln!("[GPasTe] Could not save prompts: {error}");
                }
            }
            Err(error) => eprintln!("[GPasTe] Could not serialize prompts: {error}"),
        }
    }

    fn save_current_prompt(&mut self) {
        let text = self.prompt.trim();
        if text.is_empty() {
            self.status = "Nothing to save: the prompt is empty".into();
            return;
        }
        if self.saved_prompts.iter().any(|p| p == text) {
            self.status = "This prompt is already saved".into();
            return;
        }
        self.saved_prompts.insert(0, text.to_owned());
        self.persist_prompts();
        self.status = "Prompt saved to sidebar".into();
    }

    // ---------- bridge messages ----------

    fn pending(&self) -> bool {
        self.active_request.is_some()
    }

    fn is_current(&self, request_id: &str) -> bool {
        self.active_request.as_deref() == Some(request_id)
    }

    pub fn poll_messages(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::Hello { .. } => {
                    self.status = "Chrome native host connected".into();
                }
                Message::PromptSent { request_id } => {
                    if !self.is_current(&request_id) {
                        continue; // aborted / stale
                    }
                    // Stay pending: we are now waiting for the image.
                    self.status = "Prompt sent, waiting for ChatGPT…".into();
                }
                Message::AssistantResponse { request_id, .. } => {
                    if !self.is_current(&request_id) {
                        continue;
                    }
                    self.active_request = None;
                    self.status = "ChatGPT response received".into();
                }
                Message::BridgeStatus { connected, detail } => {
                    self.status = if connected {
                        format!("Connected: {detail}")
                    } else {
                        format!("Disconnected: {detail}")
                    };
                }
                Message::BridgeError { request_id, error } => {
                    // Errors for an aborted/other request must not clobber the current one.
                    if let Some(id) = &request_id {
                        if !self.is_current(id) {
                            continue;
                        }
                    }
                    self.status = format!("Error: {error}");
                    self.active_request = None;
                }
                Message::ImageCopied {
                    request_id,
                    base64,
                    mime_type,
                    ..
                } => {
                    if !self.is_current(&request_id) {
                        println!("[GPasTe] Ignoring image for aborted request {request_id}");
                        continue;
                    }
                    println!("[GPasTe] Received image for {request_id}");
                    self.active_request = None;
                    self.status = "Image received, processing…".into();
                    self.spawn_decode(ctx, base64, mime_type);
                }
                Message::SendPrompt { .. } | Message::AbortPrompt { .. } => {}
            }
        }

        // Finished background decodes (texture upload must happen on the UI thread).
        while let Ok((generation, result)) = self.decoded_rx.try_recv() {
            if generation != self.decode_gen {
                continue; // superseded by a newer image
            }
            match result {
                Ok(decoded) => self.apply_decoded(ctx, decoded),
                Err(error) => {
                    self.image_status = format!("Image processing failed: {error}");
                    eprintln!("[GPasTe] {}", self.image_status);
                }
            }
        }
        // No polling repaint needed: the bridge thread wakes the UI on every incoming
        // message, and the spinner repaints itself while a request is pending.
    }

    // ---------- images ----------

    /// Downscale (never upscale) to fit `max` and convert to an egui image.
    fn thumb_color_image(img: &DynamicImage, max: u32) -> egui::ColorImage {
        let rgba = if img.width().max(img.height()) > max {
            img.thumbnail(max, max).to_rgba8()
        } else {
            img.to_rgba8()
        };
        let size = [rgba.width() as usize, rgba.height() as usize];
        egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())
    }

    /// Build an attachment from already-decoded pixels (avoids decoding twice).
    fn build_attachment(
        ctx: &egui::Context,
        name: String,
        mime: &str,
        bytes: &[u8],
        decoded: &DynamicImage,
    ) -> AttachedImage {
        // Larger than the 300px display size so it stays sharp on HiDPI.
        let preview = ctx.load_texture(
            format!("gpaste-preview-{name}"),
            Self::thumb_color_image(decoded, 600),
            egui::TextureOptions::LINEAR,
        );
        AttachedImage {
            name,
            data_url: format!("data:{mime};base64,{}", STANDARD.encode(bytes)),
            preview,
        }
    }

    fn load_image_file(ctx: &egui::Context, path: &Path) -> Result<AttachedImage, String> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("gpaste-image.png")
            .to_owned();
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mime = match extension.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "bmp" => "image/bmp",
            "avif" => "image/avif",
            _ => {
                return Err(
                    "Unsupported image type. Choose PNG, JPG, WebP, GIF, BMP, or AVIF.".into(),
                );
            }
        };
        let bytes = fs::read(path).map_err(|error| format!("Could not read image: {error}"))?;
        let decoded = image::load_from_memory(&bytes)
            .map_err(|error| format!("Could not decode image: {error}"))?;
        Ok(Self::build_attachment(ctx, name, mime, &bytes, &decoded))
    }

    /// Decode the generated image, make a preview and fill the clipboard, all off the UI thread.
    fn spawn_decode(&mut self, ctx: &egui::Context, base64_data: String, mime: String) {
        self.decode_gen += 1;
        let generation = self.decode_gen;
        let tx = self.decoded_tx.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = Self::decode_generated(&base64_data, mime);
            let _ = tx.send((generation, result));
            ctx.request_repaint();
        });
    }

    fn decode_generated(base64_data: &str, mime: String) -> Result<DecodedImage, String> {
        let bytes = STANDARD
            .decode(base64_data)
            .map_err(|error| error.to_string())?;
        let image = image::load_from_memory(&bytes).map_err(|error| error.to_string())?;
        drop(bytes);

        // Preview is capped; the full-resolution pixels are kept separately.
        let thumb = Self::thumb_color_image(&image, 1024);
        let rgba = image.into_rgba8(); // consumes, no extra copy
        let (width, height) = (rgba.width() as usize, rgba.height() as usize);
        let raw = rgba.into_raw();

        let clipboard_error = Clipboard::new()
            .and_then(|mut cb| {
                cb.set_image(arboard::ImageData {
                    width,
                    height,
                    bytes: Cow::Borrowed(&raw),
                })
            })
            .err()
            .map(|error| error.to_string());

        Ok(DecodedImage {
            width,
            height,
            raw,
            thumb,
            clipboard_error,
            mime,
        })
    }

    fn apply_decoded(&mut self, ctx: &egui::Context, decoded: DecodedImage) {
        let DecodedImage {
            width,
            height,
            raw,
            thumb,
            clipboard_error,
            mime,
        } = decoded;
        self.generated_texture =
            Some(ctx.load_texture("generated-image", thumb, egui::TextureOptions::LINEAR));
        self.generated_size = Some([width, height]);
        self.generated_rgba = Some((width, height, raw));
        self.image_status = match clipboard_error {
            None => {
                format!("Image received and copied to clipboard ({width} × {height}, {mime}).")
            }
            Some(error) => format!("Image received, but clipboard copy failed: {error}"),
        };
        self.status = "Image ready".into();
    }

    /// Failsafe: copy the full-resolution generated image again.
    fn copy_generated_to_clipboard(&mut self) {
        let Some((width, height, raw)) = &self.generated_rgba else {
            self.image_status = "No generated image to copy".into();
            return;
        };

        let data = arboard::ImageData {
            width: *width,
            height: *height,
            bytes: std::borrow::Cow::Borrowed(raw),
        };

        self.image_status = match Clipboard::new().and_then(|mut cb| cb.set_image(data)) {
            Ok(()) => format!("Full image copied to clipboard ({width} × {height})."),
            Err(error) => format!("Copy failed: {error}"),
        };
    }

    fn paste_image_from_clipboard(ctx: &egui::Context) -> Result<AttachedImage, String> {
        let mut clipboard =
            Clipboard::new().map_err(|error| format!("Could not open clipboard: {error}"))?;
        let image = clipboard.get_image().map_err(|error| {
            format!(
                "No image found in clipboard. Copy an image in your browser first. Details: {error}"
            )
        })?;

        let rgba = RgbaImage::from_raw(
            image.width as u32,
            image.height as u32,
            image.bytes.into_owned(),
        )
        .ok_or_else(|| "Clipboard image had invalid pixel data.".to_owned())?;

        // Keep the pixels we already have for the preview; only encode PNG for the upload.
        let decoded = DynamicImage::ImageRgba8(rgba);
        let mut cursor = Cursor::new(Vec::new());
        decoded
            .write_to(&mut cursor, ImageFormat::Png)
            .map_err(|error| format!("Could not encode clipboard image: {error}"))?;

        Ok(Self::build_attachment(
            ctx,
            "clipboard-image.png".to_owned(),
            "image/png",
            &cursor.into_inner(),
            &decoded,
        ))
    }

    fn send_to_chatgpt(&mut self) {
        if self.pending() {
            return;
        }
        let request_id = format!("req-{}-{}", self.session, self.next_id);
        self.next_id += 1;

        let message = Message::SendPrompt {
            request_id: request_id.clone(),
            text: self.prompt.trim().to_owned(),
            image_data_url: self
                .selected_image
                .as_ref()
                .map(|image| image.data_url.clone()),
            image_name: self.selected_image.as_ref().map(|image| image.name.clone()),
        };

        match self.tx.send(message) {
            Ok(()) => {
                self.active_request = Some(request_id);
                self.status = "Request sent to bridge…".into();
            }
            Err(error) => {
                self.status = format!("Bridge worker unavailable: {error}");
            }
        }
    }

    /// Abort the in-flight request. The UI is released immediately (optimistic);
    /// anything the extension still reports for this id is ignored.
    fn abort_request(&mut self) {
        let Some(request_id) = self.active_request.take() else {
            return;
        };
        self.status = match self.tx.send(Message::AbortPrompt { request_id }) {
            Ok(()) => "Aborted. You can send a new prompt.".into(),
            Err(error) => format!("Aborted locally, but bridge worker unavailable: {error}"),
        };
    }

    // ---------- UI pieces ----------

    fn ui_top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top_bar")
            .frame(tight_frame())
            .show(ui, |ui| {
                // ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let (icon, tip) = if self.sidebar_open {
                        (icons::ARROW_LEFT, "Collapse sidebar")
                    } else {
                        (icons::MENU, "Expand sidebar")
                    };
                    if ui
                        .add(Button::icon(icon).outline())
                        .on_hover_text(tip)
                        .clicked()
                    {
                        self.sidebar_open = !self.sidebar_open;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&self.status).weak()).truncate(),
                        );
                    });
                });
                ui.add_space(3.0);
            });
    }

    fn ui_sidebar(&mut self, ui: &mut egui::Ui) {
        let mut to_fill: Option<usize> = None;
        let mut to_delete: Option<usize> = None;

        egui::CentralPanel::default()
            .frame(tight_frame())
            .show(ui, |ui| {
                // ── Header ──
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Saved prompts").strong().size(14.0));
                    ui.label(
                        egui::RichText::new(self.saved_prompts.len().to_string())
                            .small()
                            .weak(),
                    );
                });
                ui.add_space(2.0);

                // ── Empty state ──
                if self.saved_prompts.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("No saved prompts yet").weak());
                        ui.label(
                            egui::RichText::new("Write a prompt and press save")
                                .small()
                                .weak(),
                        );
                    });
                    return;
                }

                // ── Cards ──
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for (index, prompt) in self.saved_prompts.iter().enumerate() {
                            match prompt_card(ui, index, prompt) {
                                CardAction::Open => to_fill = Some(index),
                                CardAction::Delete => to_delete = Some(index),
                                CardAction::None => {}
                            }
                        }
                    });
            });

        if let Some(index) = to_fill {
            self.prompt = self.saved_prompts[index].clone();
            self.sidebar_open = false; // jump back to the editor
            self.status = "Prompt loaded".into();
        }
        if let Some(index) = to_delete {
            self.saved_prompts.remove(index);
            self.persist_prompts();
            self.status = "Prompt deleted".into();
        }
    }
    fn ui_main(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();

        egui::CentralPanel::default()
            .frame(tight_frame())
            .show(ui, |ui| {
                // Height we have to fill, captured BEFORE entering the scroll area
                // (inside it the available height is unbounded).
                let _avail_h = ui.available_height();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // ----- Prompt (label removed, the hint text says it) -----
                        egui::ScrollArea::vertical()
                            .id_salt("prompt_scroll")
                            .max_height(100.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut self.prompt)
                                        .desired_rows(3)
                                        .hint_text(
                                            "Type a prompt (optional if you attach an image)…",
                                        )
                                        .desired_width(f32::INFINITY),
                                );
                            });

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add(Button::icon(icons::SAVE).outline())
                                .on_hover_text("Save current prompt")
                                .clicked()
                            {
                                self.save_current_prompt();
                            }
                            if ui
                                .add(Button::icon(icons::REFRESH).outline())
                                .on_hover_text("Clear texts")
                                .clicked()
                            {
                                self.prompt.clear();
                                self.selected_image = None;
                            }
                            if ui
                                .add(Button::icon(icons::FOLDER_OPEN).outline())
                                .on_hover_text("Pick image from folder")
                                .clicked()
                            {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter(
                                        "Images",
                                        &["png", "jpg", "jpeg", "webp", "gif", "bmp", "avif"],
                                    )
                                    .pick_file()
                                {
                                    match Self::load_image_file(&ctx, &path) {
                                        Ok(image) => {
                                            self.selected_image = Some(image);
                                            self.status = "Image selected".into();
                                        }
                                        Err(error) => self.status = error,
                                    }
                                }
                            }

                            if ui
                                .add(Button::icon(icons::DOWNLOAD).outline())
                                .on_hover_text("Paste image from clipboard")
                                .clicked()
                            {
                                match Self::paste_image_from_clipboard(&ctx) {
                                    Ok(image) => {
                                        self.selected_image = Some(image);
                                        self.status = "Image pasted from clipboard".into();
                                    }
                                    Err(error) => self.status = error,
                                }
                            }

                            if self.selected_image.is_some()
                                && ui
                                    .add(Button::icon(icons::TRASH).danger())
                                    .clicked()
                            {
                                self.selected_image = None;
                                self.status = "Image removed".into();
                            }

                            if self.generated_texture.is_some()
                                && ui
                                    .button("To Clipboard")
                                    .on_hover_text(
                                        "Copy the full-resolution image to the clipboard",
                                    )
                                    .clicked()
                            {
                                self.copy_generated_to_clipboard();
                            }

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if self.pending() {
                                        let abort = Button::new(
                                            egui::RichText::new("⏹ Abort").strong(),
                                        )
                                        .small()
                                        .danger();
                                        if ui
                                            .add(abort)
                                            .on_hover_text(
                                                "Stop waiting and stop ChatGPT's generation (Esc)",
                                            )
                                            .clicked()
                                        {
                                            self.abort_request();
                                        }
                                        ui.add(egui::Spinner::new().color(pal::ACCENT));
                                    } else {
                                        let has_content = !self.prompt.trim().is_empty()
                                            || self.selected_image.is_some();
                                        let send = Button::new(
                                            egui::RichText::new("Send to ChatGPT").strong(),
                                        );
                                        if ui.add_enabled(has_content, send).clicked() {
                                            self.send_to_chatgpt();
                                        }
                                    }
                                },
                            );
                        });

                        ui.separator();
                        ui.vertical(|ui| {
                            // Two equal slots side by side, each allocated in the layout flow.
                            let gap = 8.0;
                            let total_w = ui.available_width();
                            let stacked = total_w < SLOT_MIN_W * 2.0 + gap;
                            let w = if stacked {
                                total_w
                            } else {
                                ((total_w - gap) / 2.0).floor()
                            };

                            let area_h = if stacked { SLOT_H * 2.0 + gap } else { SLOT_H };
                            let (area, _) =
                                ui.allocate_exact_size(vec2(total_w, area_h), Sense::hover());

                            let slot1 = egui::Rect::from_min_size(area.min, vec2(w, SLOT_H));
                            let slot2 = if stacked {
                                egui::Rect::from_min_size(
                                    area.min + vec2(0.0, SLOT_H + gap),
                                    vec2(w, SLOT_H),
                                )
                            } else {
                                egui::Rect::from_min_size(
                                    area.min + vec2(w + gap, 0.0),
                                    vec2(w, SLOT_H),
                                )
                            };

                            image_slot(
                                ui,
                                slot1,
                                "attached",
                                self.selected_image.as_ref().map(|i| &i.preview),
                                "No image attached.",
                                self.selected_image.as_ref().map(|i| i.name.as_str()),
                            );
                            image_slot(
                                ui,
                                slot2,
                                "generated",
                                self.generated_texture.as_ref(),
                                "Nothing generated yet.",
                                None,
                            );

                            if !self.image_status.is_empty() {
                                ui.label(egui::RichText::new(&self.image_status).small().weak());
                            }
                        });

                        if !self.image_status.is_empty() {
                            ui.add_space(4.0);
                            ui.label(&self.image_status);
                        }
                    });
            });
    }
    pub fn render_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        let paste_shortcut =
            ctx.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::F));
        if paste_shortcut {
            match Self::paste_image_from_clipboard(&ctx) {
                Ok(image) => {
                    self.selected_image = Some(image);
                    self.status = "Image pasted from clipboard".into();
                }
                Err(error) => self.status = error,
            }
        }

        // Esc aborts the in-flight request.
        if self.pending() && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.abort_request();
        }

        // Panel order matters: top, then side, then central.
        self.ui_top_bar(ui);
        if self.sidebar_open {
            self.ui_sidebar(ui);
        } else {
            self.ui_main(ui);
        }
    }
}
/// Paints one preview holder inside `rect`. Everything is clipped to `rect`.
fn image_slot(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: &str,
    tex: Option<&egui::TextureHandle>,
    empty_text: &str,
    name: Option<&str>,
) {
    let painter = ui.painter_at(rect); // clips to the holder
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(6),
        ui.visuals().widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );

    let inner = rect.shrink(4.0);
    match tex {
        Some(tex) if tex.size_vec2().x > 0.0 && tex.size_vec2().y > 0.0 => {
            let t = tex.size_vec2();
            let k = (inner.width() / t.x).min(inner.height() / t.y);
            let img_rect = egui::Rect::from_center_size(inner.center(), t * k);
            painter.image(
                tex.id(),
                img_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                pal::WHITE,
            );
        }
        _ => {
            let color = ui.visuals().weak_text_color();
            // Wraps to the holder width instead of overflowing it.
            let galley = painter.layout(
                empty_text.to_owned(),
                egui::FontId::proportional(12.0),
                color,
                inner.width().max(1.0),
            );
            let pos = inner.center() - galley.size() / 2.0;
            painter.galley(pos, galley, color);
        }
    }

    let resp = ui.interact(rect, ui.id().with(("preview_slot", id)), Sense::hover());
    if let Some(name) = name {
        resp.on_hover_text(name);
    }
}
