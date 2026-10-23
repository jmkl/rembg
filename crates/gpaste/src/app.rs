use crate::protocol::Message;
use arboard::Clipboard;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use image::{DynamicImage, ImageFormat, RgbaImage};
use std::{fs, io::Cursor, path::Path, time::Duration};

const PROMPTS_FILE: &str = "saved_prompts.json";
const MAX_PREVIEW: f32 = 300.0;

struct AttachedImage {
    name: String,
    data_url: String,
    preview: egui::TextureHandle,
}

pub struct ChatGptApp {
    prompt: String,
    selected_image: Option<AttachedImage>,
    status: String,
    pending: bool,
    tx: Sender<Message>,
    rx: Receiver<Message>,
    next_id: u64,
    generated_texture: Option<egui::TextureHandle>,
    generated_size: Option<[usize; 2]>,
    // Full-resolution RGBA pixels, kept so we can re-copy to the clipboard.
    generated_rgba: Option<(usize, usize, Vec<u8>)>,
    image_status: String,
    sidebar_open: bool,
    saved_prompts: Vec<String>,
}

/// Scale `size` down (never up) so it fits inside max_w x max_h, keeping aspect ratio.
fn fit_size(size: egui::Vec2, max_w: f32, max_h: f32) -> egui::Vec2 {
    if size.x <= 0.0 || size.y <= 0.0 {
        return size;
    }
    let scale = (max_w / size.x).min(max_h / size.y).min(1.0);
    size * scale
}

/// First line of a prompt, shortened for the sidebar list.
fn prompt_title(prompt: &str) -> String {
    let first = prompt.lines().next().unwrap_or("").trim();
    let mut title: String = first.chars().take(36).collect();
    if first.chars().count() > 36 || prompt.lines().count() > 1 {
        title.push('…');
    }
    title
}

impl ChatGptApp {
    pub fn new(tx: Sender<Message>, rx: Receiver<Message>) -> Self {
        Self {
            prompt: String::new(),
            selected_image: None,
            status: "Waiting for Chrome extension…".into(),
            pending: false,
            tx,
            rx,
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

    fn poll_messages(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::Hello { .. } => {
                    self.status = "Chrome native host connected".into();
                }
                Message::PromptSent { .. } => {
                    // Keep `pending` true: we are now waiting for the image.
                    self.status = "Prompt sent, waiting for ChatGPT…".into();
                }
                Message::AssistantResponse { .. } => {
                    self.pending = false;
                    self.status = "ChatGPT response received".into();
                }
                Message::BridgeStatus { connected, detail } => {
                    self.status = if connected {
                        format!("Connected: {detail}")
                    } else {
                        format!("Disconnected: {detail}")
                    };
                }
                Message::BridgeError { error, .. } => {
                    self.status = format!("Error: {error}");
                    self.pending = false;
                }
                Message::ImageCopied {
                    request_id,
                    base64,
                    mime_type,
                    width: _,
                    height: _,
                } => {
                    println!("[GPasTe] Received image for {request_id}");
                    self.pending = false;
                    self.handle_generated_image(ctx, &base64, &mime_type);
                }
                Message::SendPrompt { .. } => {}
            }
        }

        if self.pending {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    // ---------- images ----------

    fn image_from_bytes(
        ctx: &egui::Context,
        name: String,
        mime: &str,
        bytes: Vec<u8>,
    ) -> Result<AttachedImage, String> {
        let decoded = image::load_from_memory(&bytes)
            .map_err(|error| format!("Could not decode image: {error}"))?;
        // Larger thumbnail than the 300px display size so it stays sharp on HiDPI.
        let thumbnail = decoded.thumbnail(600, 600).to_rgba8();
        let dimensions = [thumbnail.width() as usize, thumbnail.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(dimensions, thumbnail.as_raw());
        let preview = ctx.load_texture(
            format!("gpaste-preview-{name}"),
            color_image,
            egui::TextureOptions::LINEAR,
        );

        Ok(AttachedImage {
            name,
            data_url: format!("data:{mime};base64,{}", STANDARD.encode(bytes)),
            preview,
        })
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
        Self::image_from_bytes(ctx, name, mime, bytes)
    }

    fn handle_generated_image(&mut self, ctx: &egui::Context, base64_data: &str, mime_type: &str) {
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let bytes = STANDARD.decode(base64_data)?;
            let image = image::load_from_memory(&bytes)?;
            let rgba = image.to_rgba8();

            let width = rgba.width() as usize;
            let height = rgba.height() as usize;
            let raw = rgba.into_raw();

            let color_image = egui::ColorImage::from_rgba_unmultiplied([width, height], &raw);
            self.generated_texture = Some(ctx.load_texture(
                "generated-image",
                color_image,
                egui::TextureOptions::LINEAR,
            ));
            self.generated_size = Some([width, height]);
            self.generated_rgba = Some((width, height, raw.clone()));

            let clipboard_image = arboard::ImageData {
                width,
                height,
                bytes: std::borrow::Cow::Owned(raw),
            };
            let mut clipboard = arboard::Clipboard::new()?;
            clipboard.set_image(clipboard_image)?;

            self.image_status = format!(
                "Image received and copied to clipboard ({width} × {height}, {mime_type})."
            );
            Ok(())
        })();

        if let Err(error) = result {
            self.image_status = format!("Image processing failed: {error}");
            eprintln!("[GPasTe] {}", self.image_status);
        }
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

        let mut cursor = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(rgba)
            .write_to(&mut cursor, ImageFormat::Png)
            .map_err(|error| format!("Could not encode clipboard image: {error}"))?;

        Self::image_from_bytes(
            ctx,
            "clipboard-image.png".to_owned(),
            "image/png",
            cursor.into_inner(),
        )
    }

    fn send_to_chatgpt(&mut self) {
        let message = Message::SendPrompt {
            request_id: format!("req-{}", self.next_id),
            text: self.prompt.trim().to_owned(),
            image_data_url: self
                .selected_image
                .as_ref()
                .map(|image| image.data_url.clone()),
            image_name: self.selected_image.as_ref().map(|image| image.name.clone()),
        };
        self.next_id += 1;

        match self.tx.send(message) {
            Ok(()) => {
                self.pending = true;
                self.status = "Request sent to bridge…".into();
            }
            Err(error) => {
                self.status = format!("Bridge worker unavailable: {error}");
            }
        }
    }

    // ---------- UI pieces ----------

    fn ui_top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top_bar").show_inside(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let (icon, tip) = if self.sidebar_open {
                    ("◀", "Collapse sidebar")
                } else {
                    ("☰", "Expand sidebar")
                };
                if ui.button(icon).on_hover_text(tip).clicked() {
                    self.sidebar_open = !self.sidebar_open;
                }
                ui.heading("ChatGPT Bridge");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(&self.status).weak()).truncate());
                });
            });
            ui.add_space(6.0);
        });
    }

    fn ui_sidebar(&mut self, ui: &mut egui::Ui) {
        let mut to_fill: Option<usize> = None;
        let mut to_delete: Option<usize> = None;

        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(250.0)
            .min_size(180.0)
            .show_inside(ui, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.heading("Saved prompts");
                    ui.label(egui::RichText::new(format!("({})", self.saved_prompts.len())).weak());
                });
                ui.separator();

                if self.saved_prompts.is_empty() {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(
                            "No saved prompts yet.\nWrite one and press “Save prompt”.",
                        )
                        .weak(),
                    );
                    return;
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (index, prompt) in self.saved_prompts.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.small_button("✕").on_hover_text("Delete").clicked() {
                                    to_delete = Some(index);
                                }
                                let response = ui
                                    .selectable_label(false, prompt_title(prompt))
                                    .on_hover_text(prompt);
                                if response.clicked() {
                                    to_fill = Some(index);
                                }
                            });
                        }
                    });
            });

        if let Some(index) = to_fill {
            self.prompt = self.saved_prompts[index].clone();
            self.status = "Prompt loaded from sidebar".into();
        }
        if let Some(index) = to_delete {
            self.saved_prompts.remove(index);
            self.persist_prompts();
            self.status = "Prompt deleted".into();
        }
    }

    fn ui_main(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // ----- Prompt -----
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Prompt").strong());
                    egui::ScrollArea::vertical()
                        .id_salt("prompt_scroll")
                        .max_height(160.0) // the height at which it starts scrolling
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.prompt)
                                    .desired_rows(4) // minimum height
                                    .hint_text("Type a prompt (optional if you attach an image)…")
                                    .desired_width(f32::INFINITY),
                            );
                        });

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button("＋ Save prompt")
                            .on_hover_text("Save this prompt to the sidebar")
                            .clicked()
                        {
                            self.save_current_prompt();
                        }
                        if ui.button("Clear").clicked() {
                            self.prompt.clear();
                            self.selected_image = None;
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let has_content =
                                !self.prompt.trim().is_empty() || self.selected_image.is_some();
                            let label = if self.pending {
                                "Sending…"
                            } else {
                                "Send to ChatGPT"
                            };
                            let button = egui::Button::new(egui::RichText::new(label).strong())
                                .min_size(egui::vec2(140.0, 28.0));
                            if ui
                                .add_enabled(!self.pending && has_content, button)
                                .clicked()
                            {
                                self.send_to_chatgpt();
                            }
                            if self.pending {
                                ui.spinner();
                            }
                        });
                    });

                    // ----- Attached image -----
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Attachment").strong());

                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Choose image…").clicked() {
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

                        if ui.button("Paste image (Ctrl+F)").clicked() {
                            match Self::paste_image_from_clipboard(&ctx) {
                                Ok(image) => {
                                    self.selected_image = Some(image);
                                    self.status = "Image pasted from clipboard".into();
                                }
                                Err(error) => self.status = error,
                            }
                        }

                        if self.selected_image.is_some() && ui.button("Remove image").clicked() {
                            self.selected_image = None;
                            self.status = "Image removed".into();
                        }
                    });

                    if let Some(image) = &self.selected_image {
                        ui.label(egui::RichText::new(&image.name).weak());
                        let size = fit_size(
                            image.preview.size_vec2(),
                            ui.available_width().min(MAX_PREVIEW),
                            MAX_PREVIEW,
                        );
                        ui.add(egui::Image::new((image.preview.id(), size)).corner_radius(6.0));
                    } else {
                        ui.label(egui::RichText::new("No image attached.").weak());
                    }

                    // ----- Generated image -----
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Generated image").strong());

                    if let Some(texture) = &self.generated_texture {
                        let size = fit_size(
                            texture.size_vec2(),
                            ui.available_width().min(MAX_PREVIEW),
                            MAX_PREVIEW,
                        );
                        ui.add(egui::Image::new((texture.id(), size)).corner_radius(6.0));

                        if let Some([w, h]) = self.generated_size {
                            ui.label(
                                egui::RichText::new(format!("{w} × {h} px (full size)")).weak(),
                            );
                        }

                        if ui
                            .button("📋 Copy full image")
                            .on_hover_text("Copy the full-resolution image to the clipboard")
                            .clicked()
                        {
                            self.copy_generated_to_clipboard();
                        }
                    } else {
                        ui.label(egui::RichText::new("Nothing generated yet.").weak());
                    }

                    if !self.image_status.is_empty() {
                        ui.add_space(4.0);
                        ui.label(&self.image_status);
                    }
                });
        });
    }
}

impl eframe::App for ChatGptApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_messages(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Ctrl+F pastes an image from the clipboard.
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

        // Panel order matters: top, then side, then central.
        self.ui_top_bar(ui);
        if self.sidebar_open {
            self.ui_sidebar(ui);
        }
        self.ui_main(ui);
    }
}
