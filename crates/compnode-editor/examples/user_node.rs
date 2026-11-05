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

// ───────────────────────────── the payload ─────────────────────────────

/// The data that travels along the wires of this example.
#[derive(Clone, Debug, Default, PartialEq)]
struct Image {
    width: u32,
    height: u32,
    /// A colour, so the preview shows something.
    tint: [f32; 4],
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
            width: size,
            height: size,
            tint: [c.r, c.g, c.b, c.a],
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
}

impl MyNode {
    /// What the button does.
    fn btn_click(&mut self) {
        self.x = (self.x + 0.25).fract();
    }
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
        // pull the image that arrived on the first input
        let image = ctx.cloned::<Image>(0).ok_or(NodeError::MissingInput(0))?;
        self.image = image.clone();
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
            self.btn_click();
        }
    }
}

// ───────────────────────────── the app ─────────────────────────────

struct App {
    editor: compnode_editor::CompnodeEditor,
    /// The textures uploaded for the image handles.
    textures: HashMap<u64, egui::TextureId>,
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
            textures: HashMap::new(),
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
        let mut textures = self.textures.clone();
        let uids: Vec<u64> = self.editor.document().uids();
        for uid in uids {
            let Some(outputs) = self.editor.evaluator().outputs(uid) else {
                continue;
            };
            let Some(image) = outputs.first().and_then(|p| p.get::<Image>()) else {
                continue;
            };
            let (width, height, tint) = (image.width, image.height, image.tint);
            let pixels: Vec<u8> = (0..(width * height))
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
                .collect();
            let color = egui::ColorImage::from_rgba_unmultiplied(
                [width as usize, height as usize],
                &pixels,
            );
            let id = ctx
                .load_texture(format!("node-{uid}"), color, egui::TextureOptions::LINEAR)
                .id();
            let handle = self.next_handle;
            self.next_handle += 1;
            textures.insert(handle, id);

            // The preview row of `MyNode` is the first footer row, which sits
            // after the output sockets.
            let slot = self
                .editor
                .document()
                .schema_of(uid)
                .map(|s| s.outputs.len())
                .unwrap_or(0);
            let aspect = width as f32 / height.max(1) as f32;
            self.editor.set_preview(
                uid,
                slot,
                Some(compnode_ui::value::ImageHandle(handle)),
                aspect,
            );
        }
        self.editor.set_textures(textures);
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
