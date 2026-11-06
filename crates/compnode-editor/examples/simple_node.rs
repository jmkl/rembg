//! The smallest useful host: define one node and show it in an editor.
//!
//! ```text
//! cargo run -p compnode-editor --example simple_node
//! ```
//!
//! The node multiplies two numbers. Its inputs, the widget in the node body
//! and the evaluation are all driven by the `CompnodeNode` implementation;
//! the host only creates the editor and draws it.

use compnode_editor::{CompnodeNode, NodeError, Payload, ProcessCtx, register_nodes};
use compnode_ui::schema::{Category, SocketDef, WidgetKind};
use compnode_ui::value::{SocketType, Value};

// ───────────────────────────── the node ─────────────────────────────

/// Multiplies the value on the first input by the factor on the second.
struct Scale {
    factor: f32,
}

impl Default for Scale {
    fn default() -> Scale {
        Scale { factor: 2.0 }
    }
}

impl CompnodeNode for Scale {
    fn id(&self) -> &'static str {
        // The id is written to project files: keep it stable.
        "demo.scale"
    }

    fn title(&self) -> &str {
        "Scale"
    }

    fn category(&self) -> Category {
        Category::Utility
    }

    fn inputs(&self) -> Vec<SocketDef> {
        vec![
            // A plain input pin with no widget: it expects a wire.
            SocketDef::new("Value", SocketType::Float)
                .with_default(Value::Float(1.0))
                .widget(WidgetKind::None),
            // A pin with a slider widget while it is not connected.
            SocketDef::float("Factor", self.factor).range(0.0, 10.0),
        ]
    }

    fn outputs(&self) -> Vec<SocketDef> {
        vec![SocketDef::new("Result", SocketType::Float)]
    }

    fn process(&mut self, ctx: &mut ProcessCtx) -> Result<Vec<Payload>, NodeError> {
        // A connected socket yields the value on the wire; otherwise the
        // value the user typed into the widget is used.
        let value = ctx.float(0);
        let factor = ctx.float(1);
        Ok(vec![Payload::value(Value::Float(value * factor))])
    }

    fn set_param(&mut self, key: &str, value: Value) -> bool {
        // Mirror document values into the instance so `process` sees them.
        if key == "Factor" {
            self.factor = value.as_f32().unwrap_or(self.factor);
            true
        } else {
            false
        }
    }
}

// ───────────────────────────── the host ─────────────────────────────

struct App {
    editor: compnode_editor::CompnodeEditor,
}

impl App {
    fn new() -> App {
        // Registering a node makes it appear in the add-node menu.
        let mut editor = register_nodes![Scale];
        editor.add_node("demo.scale", Default::default());
        editor.view_all();
        App { editor }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            self.editor.draw(ui);
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        multisampling: 4,
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_inner_size([1000.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "compnode — simple node",
        options,
        Box::new(|_cc| Ok(Box::new(App::new()))),
    )
}
