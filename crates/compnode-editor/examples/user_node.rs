//! A node defined by the host — the whole point of the bridge.
//!
//! ```text
//! cargo run -p compnode-editor --example user_node
//! ```
//!
//! The example defines two nodes as plain Rust structs:
//!
//! * `Source` produces an [`Image`],
//! * `MyNode` takes an image, owns two knobs and a button, and gives the
//!   image back.
//!
//! Everything else — the graph, the wires, the undo history, the add-node
//! menu, the evaluation — is handled by `compnode-editor`.

use compnode_editor::{CompnodeNode, NodeError, Payload, ProcessCtx, register_nodes};
use compnode_ui::schema::{Category, SocketDef, WidgetKind};
use compnode_ui::value::{SocketType, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

// ───────────────────────────── the payload ─────────────────────────────

/// Every evaluated image gets a fresh id, so the host can tell whether the
/// texture it uploaded is still current.
static NEXT_IMAGE_ID: AtomicU64 = AtomicU64::new(1);

fn next_image_id() -> u64 {
    NEXT_IMAGE_ID.fetch_add(1, Ordering::Relaxed)
}

/// The data that travels along the wires of this example.
#[derive(Clone, Debug, Default, PartialEq)]
struct Image {
    /// Changes whenever a node produces a new image.
    id: u64,
    width: u32,
    height: u32,
    /// A colour, so the generated preview shows something.
    tint: [f32; 4],
    /// Straight-alpha sRGB RGBA8, when the image came from a file. Shared so
    /// cloning a payload stays cheap even for a large image.
    pixels: Option<Arc<[u8]>>,
}

impl Image {
    /// Decode an image file into RGBA8.
    fn from_path(path: &Path) -> Result<Image, String> {
        let rgba = image::open(path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .to_rgba8();
        let (width, height) = rgba.dimensions();
        Ok(Image {
            id: next_image_id(),
            width,
            height,
            tint: [1.0; 4],
            pixels: Some(Arc::from(rgba.into_raw())),
        })
    }
}

// ───────────────────────────── the nodes ─────────────────────────────

/// A node that makes an image.
#[derive(Default)]
struct Source {
    size: u32,
}

impl CompnodeNode for Source {
    fn id(&self) -> &'static str {
        "demo.source"
    }

    fn title(&self) -> &str {
        "Source"
    }

    fn category(&self) -> Category {
        Category::Input
    }

    fn inputs(&self) -> Vec<SocketDef> {
        vec![
            SocketDef::int("Size", self.size as i32).range(16.0, 512.0),
            SocketDef::color("Tint", compnode_core::Color::rgb(0.3, 0.6, 0.9)),
        ]
    }

    fn outputs(&self) -> Vec<SocketDef> {
        vec![SocketDef::image("Image")]
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> Result<Vec<Payload>, NodeError> {
        let size = ctx.int(0).max(1) as u32;
        let c = ctx.color(1);
        Ok(vec![Payload::new(Image {
            id: next_image_id(),
            width: size,
            height: size,
            tint: [c.r, c.g, c.b, c.a],
            pixels: None,
        })])
    }

    fn set_param(&mut self, key: &str, v: Value) -> bool {
        match key {
            "Size" => {
                self.size = v.as_f32().unwrap_or(self.size as f32) as u32;
                true
            }
            _ => false,
        }
    }
}

/// The node from the sketch: a struct that owns its state, a button and two
/// knobs.
#[derive(Default)]
struct MyNode {
    x: f32,
    y: f32,
    image: Image,
    /// Set once a file was opened through the button; from then on the node
    /// shows that image instead of the one arriving on its input.
    loaded: bool,
    /// The last file-dialog/decoder error, reported by `process`.
    error: Option<String>,
}

impl MyNode {
    /// The button: pick an image file and load it into the node.
    fn open_image(&mut self) {
        let Some(path) = pick_image_path() else {
            return; // the user cancelled
        };
        match Image::from_path(&path) {
            Ok(image) => {
                self.image = image;
                self.loaded = true;
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
    }
}

fn pick_image_path() -> Option<std::path::PathBuf> {
    // Tests cannot drive the native dialog; they supply a path instead.
    #[cfg(test)]
    if let Some(path) = std::env::var_os("COMPNODE_USER_NODE_IMAGE") {
        return Some(path.into());
    }
    rfd::FileDialog::new()
        .add_filter(
            "images",
            &[
                "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "ico", "tga", "dds",
                "hdr", "exr", "pnm", "ff", "qoi",
            ],
        )
        .pick_file()
}

impl CompnodeNode for MyNode {
    fn id(&self) -> &'static str {
        "demo.mynode"
    }

    fn title(&self) -> &str {
        "Something"
    }

    fn category(&self) -> Category {
        Category::Filter
    }

    fn inputs(&self) -> Vec<SocketDef> {
        vec![
            // the data that comes in
            SocketDef::image("Image"),
            // the knobs: their default comes from the node's own state
            SocketDef::float("X", self.x).range(0.0, 1.0),
            SocketDef::float("Y", self.y).range(0.0, 1.0),
        ]
    }

    fn outputs(&self) -> Vec<SocketDef> {
        vec![SocketDef::image("Image")]
    }

    /// Rows without pins: the image viewer and the button.
    fn footer(&self) -> Vec<SocketDef> {
        vec![
            SocketDef::image("Preview").widget(WidgetKind::ImagePreview),
            SocketDef::new("Open", SocketType::Bool).widget(WidgetKind::Button),
        ]
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> Result<Vec<Payload>, NodeError> {
        if let Some(message) = &self.error {
            return Err(NodeError::Failed(message.clone()));
        }
        if !self.loaded {
            // Pull the image that arrived on the first input.
            self.image = ctx.cloned::<Image>(0).ok_or(NodeError::MissingInput(0))?;
        }
        Ok(vec![Payload::new(self.image.clone())])
    }

    /// The user moved a knob: mirror it into the node's own state.
    fn set_param(&mut self, key: &str, v: Value) -> bool {
        match key {
            "X" => {
                self.x = v.as_f32().unwrap_or(self.x);
                true
            }
            "Y" => {
                self.y = v.as_f32().unwrap_or(self.y);
                true
            }
            _ => false,
        }
    }

    /// The user pressed the button.
    fn on_button(&mut self, key: &str) {
        if key == "Open" {
            self.open_image();
        }
    }
}

// ───────────────────────────── the app ─────────────────────────────

/// A preview texture uploaded for one node.
struct Preview {
    /// The `Image::id` that was uploaded.
    image_id: u64,
    /// The handle the editor refers to.
    handle: u64,
    /// Keeping the handle alive is what keeps the GPU texture allocated: a
    /// `TextureId` alone is just an index into the texture manager.
    texture: egui::TextureHandle,
}

struct App {
    editor: compnode_editor::CompnodeEditor,
    /// Preview textures, by node uid.
    previews: HashMap<u64, Preview>,
    /// The next image handle.
    next_handle: u64,
}

impl App {
    fn new() -> App {
        let mut editor = register_nodes![Source, MyNode];
        // A small graph to start with: source -> something.
        let a = editor.add_node("demo.source", Default::default());
        let b = editor.add_node("demo.mynode", compnode_core::geom::vec2(320.0, 0.0));
        if let (Some(a), Some(b)) = (a, b) {
            editor.editor_mut().connect((a, 0), (b, 0));
        }
        editor.view_all();
        App {
            editor,
            previews: HashMap::new(),
            next_handle: 1,
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        egui::Panel::top("bar").show(root, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Add node").clicked() {
                    self.editor.open_add_menu();
                }
                if ui.button("Undo").clicked() {
                    self.editor.undo();
                }
                if ui.button("Redo").clicked() {
                    self.editor.redo();
                }
            });
        });
        egui::CentralPanel::default().show(root, |ui| {
            self.editor.draw(ui);
        });
        self.upload_previews(&ctx);
    }
}

impl App {
    /// Give every node that produced an image a texture, so the preview rows
    /// show it. The editor owns the graph; the host owns the pixels.
    fn upload_previews(&mut self, ctx: &egui::Context) {
        let uids: Vec<u64> = self.editor.document().uids();
        let mut previews: HashMap<u64, Preview> = HashMap::with_capacity(uids.len());

        for uid in uids {
            // Copy the cheap, shared parts of the payload so the editor can be
            // borrowed mutably below.
            let image = self
                .editor
                .evaluator()
                .outputs(uid)
                .and_then(|outputs| outputs.first())
                .and_then(|p| p.get::<Image>())
                .map(|image| {
                    (
                        image.id,
                        image.width,
                        image.height,
                        image.tint,
                        image.pixels.clone(),
                    )
                });
            let Some((image_id, width, height, tint, pixels)) = image else {
                continue;
            };

            let preview = match self.previews.remove(&uid) {
                // The texture is already up to date.
                Some(preview) if preview.image_id == image_id => preview,
                // The node produced a new image: replace the pixels in place.
                Some(mut preview) => {
                    let color = color_image(width, height, tint, pixels);
                    preview.texture.set(color, egui::TextureOptions::LINEAR);
                    preview.image_id = image_id;
                    preview
                }
                None => {
                    let color = color_image(width, height, tint, pixels);
                    let texture = ctx.load_texture(
                        format!("node-{uid}"),
                        color,
                        egui::TextureOptions::LINEAR,
                    );
                    let handle = self.next_handle;
                    self.next_handle += 1;
                    Preview {
                        image_id,
                        handle,
                        texture,
                    }
                }
            };

            // The preview row is the first footer row, which sits after the
            // output sockets.
            let slot = self
                .editor
                .document()
                .schema_of(uid)
                .map(|s| s.outputs.len())
                .unwrap_or(0);
            // The preview widget expects height / width.
            let aspect = height as f32 / width.max(1) as f32;
            self.editor.set_preview(
                uid,
                slot,
                Some(compnode_ui::value::ImageHandle(preview.handle)),
                aspect,
            );
            previews.insert(uid, preview);
        }

        let ids: HashMap<u64, egui::TextureId> = previews
            .values()
            .map(|preview| (preview.handle, preview.texture.id()))
            .collect();
        self.editor.set_textures(ids);
        self.previews = previews;
    }
}

/// Preview textures are capped to this width. The preview row is small, so
/// uploading a full-resolution image wastes GPU memory (and can exhaust it)
/// without making the preview look any better.
const MAX_PREVIEW_WIDTH: u32 = 256;

/// The texture size for an image of `width` x `height`: scaled down to
/// [`MAX_PREVIEW_WIDTH`] when it is wider, keeping the aspect ratio.
fn preview_size(width: u32, height: u32) -> (u32, u32) {
    let (width, height) = (width.max(1), height.max(1));
    if width <= MAX_PREVIEW_WIDTH {
        return (width, height);
    }
    let scale = MAX_PREVIEW_WIDTH as f32 / width as f32;
    (
        MAX_PREVIEW_WIDTH,
        ((height as f32 * scale).round() as u32).max(1),
    )
}

/// Build the pixels for a preview: decoded RGBA8 when there are pixels,
/// otherwise the tinted checkerboard. Both are scaled down to
/// [`MAX_PREVIEW_WIDTH`]; the payload keeps the full-resolution image.
fn color_image(
    width: u32,
    height: u32,
    tint: [f32; 4],
    pixels: Option<Arc<[u8]>>,
) -> egui::ColorImage {
    let (preview_w, preview_h) = preview_size(width, height);
    let generated;
    let scaled;
    let bytes: &[u8] = match &pixels {
        // Pixels decoded from a file are already straight-alpha RGBA8.
        Some(pixels) if (preview_w, preview_h) == (width, height) => pixels,
        Some(pixels) => {
            let source = image::RgbaImage::from_raw(width, height, pixels.to_vec())
                .expect("decoded pixels match the image size");
            scaled = image::imageops::thumbnail(&source, preview_w, preview_h).into_raw();
            &scaled
        }
        None => {
            generated = (0..(preview_w * preview_h))
                .flat_map(|i| {
                    let checker = (i / 8 + i % 8) % 2 == 0;
                    let m = if checker { 1.0 } else { 0.75 };
                    [
                        (tint[0] * m * 255.0) as u8,
                        (tint[1] * m * 255.0) as u8,
                        (tint[2] * m * 255.0) as u8,
                        (tint[3] * 255.0) as u8,
                    ]
                })
                .collect::<Vec<u8>>();
            &generated
        }
    };
    egui::ColorImage::from_rgba_unmultiplied([preview_w as usize, preview_h as usize], bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_are_capped_at_256px_wide() {
        assert_eq!(preview_size(1024, 1280), (256, 320));
        assert_eq!(preview_size(1024, 512), (256, 128));
        assert_eq!(preview_size(256, 64), (256, 64));
        assert_eq!(preview_size(100, 50), (100, 50));

        // A decoded image is thumbnailed to the capped size.
        let pixels = vec![0u8; 1024 * 1280 * 4];
        let color = color_image(1024, 1280, [1.0; 4], Some(Arc::from(pixels)));
        assert_eq!(color.size, [256, 320]);

        // The generated checkerboard is built at the capped size directly.
        let color = color_image(1024, 1280, [1.0; 4], None);
        assert_eq!(color.size, [256, 320]);
    }

    #[test]
    fn open_button_loads_the_image_and_sets_the_preview() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test_images/test.jpg")
            .canonicalize()
            .unwrap();
        unsafe { std::env::set_var("COMPNODE_USER_NODE_IMAGE", &path) };
        let mut app = App::new();
        let b = app
            .editor
            .document()
            .uids()
            .into_iter()
            .find(|&u| {
                app.editor
                    .document()
                    .schema_of(u)
                    .is_some_and(|s| s.id == "demo.mynode")
            })
            .unwrap();
        // footer index 1 is the Open button
        app.editor.press_button(b, 2);
        app.editor.evaluate();
        let out = app.editor.evaluator().outputs(b).unwrap();
        let image = out[0].get::<Image>().unwrap();
        assert!(image.pixels.is_some(), "the file was not decoded");

        let ctx = egui::Context::default();
        app.upload_previews(&ctx);
        let data = app.editor.document().data(b).unwrap();
        let handle = match &data.out_values[1] {
            Value::Image(Some(handle)) => handle.0,
            other => panic!("preview was not set: {other:?}"),
        };
        assert!(app.editor.textures().contains_key(&handle));
        let preview = app.previews.get(&b).unwrap();
        assert_eq!(preview.handle, handle);
        assert_eq!(app.editor.textures()[&handle], preview.texture.id());

        // Uploading again with the same image reuses the texture.
        let texture_id = preview.texture.id();
        app.upload_previews(&ctx);
        let preview = app.previews.get(&b).unwrap();
        assert_eq!(preview.handle, handle);
        assert_eq!(preview.texture.id(), texture_id);
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        multisampling: 4,
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        "compnode — user nodes",
        options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}
