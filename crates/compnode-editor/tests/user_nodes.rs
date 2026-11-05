//! The node-authoring API, end to end: a host defines nodes, registers
//! them, and the editor does the rest.

use compnode_editor::{CompnodeNode, NodeError, Payload, ProcessCtx, register_nodes};
use compnode_ui::schema::{Category, SocketDef, WidgetKind};
use compnode_ui::value::{SocketType, Value};

/// The payload that travels between the nodes.
#[derive(Clone, Debug, Default, PartialEq)]
struct Image {
    tint: [f32; 4],
}

/// Makes an image.
struct Source {
    size: f32,
}

impl Default for Source {
    fn default() -> Self {
        Source { size: 1.0 }
    }
}

impl CompnodeNode for Source {
    fn id(&self) -> &'static str {
        "it.source"
    }
    fn title(&self) -> &str {
        "Source"
    }
    fn category(&self) -> Category {
        Category::Input
    }
    fn inputs(&self) -> Vec<SocketDef> {
        vec![SocketDef::float("Size", self.size).range(1.0, 64.0)]
    }
    fn outputs(&self) -> Vec<SocketDef> {
        vec![SocketDef::image("Image")]
    }
    fn process(&mut self, ctx: &mut ProcessCtx) -> Result<Vec<Payload>, NodeError> {
        let size = ctx.float(0);
        Ok(vec![Payload::new(Image {
            tint: [size, 0.0, 0.0, 1.0],
        })])
    }
    fn set_param(&mut self, key: &str, v: Value) -> bool {
        match key {
            "Size" => {
                self.size = v.as_f32().unwrap_or(self.size);
                true
            }
            _ => false,
        }
    }
}

/// The node from the sketch.
#[derive(Default)]
struct MyNode {
    x: f32,
    y: f32,
    image: Image,
    clicks: u32,
}

impl MyNode {
    fn btn_click(&mut self) {
        self.clicks += 1;
        self.x = (self.x + 0.25).fract();
    }
}

impl CompnodeNode for MyNode {
    fn id(&self) -> &'static str {
        "it.mynode"
    }
    fn title(&self) -> &str {
        "Something"
    }
    fn category(&self) -> Category {
        Category::Filter
    }
    fn inputs(&self) -> Vec<SocketDef> {
        vec![
            SocketDef::image("Image"),
            SocketDef::float("X", self.x).range(0.0, 1.0),
            SocketDef::float("Y", self.y).range(0.0, 1.0),
        ]
    }
    fn outputs(&self) -> Vec<SocketDef> {
        vec![
            SocketDef::image("Image"),
            SocketDef::new("Clicks", SocketType::Float),
        ]
    }
    fn footer(&self) -> Vec<SocketDef> {
        vec![
            SocketDef::image("Preview").widget(WidgetKind::ImagePreview),
            SocketDef::new("Open", SocketType::Bool).widget(WidgetKind::Button),
        ]
    }
    fn process(&mut self, ctx: &mut ProcessCtx) -> Result<Vec<Payload>, NodeError> {
        let image = ctx.cloned::<Image>(0).ok_or(NodeError::MissingInput(0))?;
        self.image = image.clone();
        Ok(vec![
            Payload::new(self.image.clone()),
            Payload::value(Value::Float(self.clicks as f32)),
        ])
    }
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
    fn on_button(&mut self, key: &str) {
        if key == "Open" {
            self.btn_click();
        }
    }
}

/// How many times the node's button was pressed, read back through its
/// second output.
fn clicks(ed: &compnode_editor::CompnodeEditor, uid: u64) -> f32 {
    ed.evaluator()
        .outputs(uid)
        .and_then(|o| o.get(1))
        .and_then(|p| p.cloned::<Value>())
        .and_then(|v| v.as_f32())
        .unwrap_or(0.0)
}

#[test]
fn a_host_registers_its_nodes_and_the_editor_runs_them() {
    let mut ed = register_nodes![Source, MyNode];
    assert_eq!(ed.types().len(), 2);
    assert!(ed.document().registry.get("it.source").is_some());
    assert!(ed.document().registry.get("it.mynode").is_some());

    let a = ed.add_node("it.source", Default::default()).unwrap();
    let b = ed.add_node("it.mynode", Default::default()).unwrap();
    assert!(ed.node(a).is_some());
    assert!(ed.node(b).is_some());

    // the source produces an image even without a connection
    ed.evaluate();
    let out = ed.evaluator().outputs(a).unwrap();
    assert_eq!(
        out[0].cloned::<Image>(),
        Some(Image {
            tint: [1.0, 0.0, 0.0, 1.0]
        })
    );

    // the consumer needs its input
    assert!(ed.evaluator().error(b).is_some());

    // wire them together
    ed.editor_mut().connect((a, 0), (b, 0));
    ed.evaluate();
    assert!(ed.evaluator().error(b).is_none());
    let out = ed.evaluator().outputs(b).unwrap();
    assert_eq!(
        out[0].cloned::<Image>(),
        Some(Image {
            tint: [1.0, 0.0, 0.0, 1.0]
        })
    );
}

#[test]
fn editing_a_parameter_only_recomputes_the_branch_it_touches() {
    let mut ed = register_nodes![Source, MyNode];
    let a = ed.add_node("it.source", Default::default()).unwrap();
    let b = ed.add_node("it.mynode", Default::default()).unwrap();
    ed.editor_mut().connect((a, 0), (b, 0));
    ed.evaluate();

    // change the source: both nodes are recomputed
    ed.editor_mut().set_value(a, 0, Value::Float(4.0));
    ed.evaluate();
    let out = ed.evaluator().outputs(b).unwrap();
    assert_eq!(
        out[0].cloned::<Image>(),
        Some(Image {
            tint: [4.0, 0.0, 0.0, 1.0]
        })
    );

    // change the consumer's own knob: only the consumer is recomputed
    ed.editor_mut().set_value(b, 1, Value::Float(0.5));
    ed.evaluate();
    let out = ed.evaluator().outputs(b).unwrap();
    assert_eq!(
        out[0].cloned::<Image>(),
        Some(Image {
            tint: [4.0, 0.0, 0.0, 1.0]
        })
    );
}

#[test]
fn a_button_press_reaches_the_node() {
    let mut ed = register_nodes![Source, MyNode];
    let b = ed.add_node("it.mynode", Default::default()).unwrap();
    // the node needs an input before it produces anything
    let a = ed.add_node("it.source", Default::default()).unwrap();
    ed.editor_mut().connect((a, 0), (b, 0));
    ed.evaluate();
    assert_eq!(clicks(&ed, b), 0.0);

    // the "Open" button is the second footer row, after the output socket
    let slot = ed.document().schema_of(b).unwrap().outputs.len() + 1;
    ed.press_button(b, slot);
    ed.evaluate();
    assert_eq!(clicks(&ed, b), 1.0);
}

#[test]
fn deleting_a_node_removes_it_from_the_graph_and_the_results() {
    let mut ed = register_nodes![Source, MyNode];
    let a = ed.add_node("it.source", Default::default()).unwrap();
    ed.evaluate();
    assert!(ed.evaluator().outputs(a).is_some());
    ed.editor_mut().delete_selection();
    ed.evaluate();
    assert!(ed.evaluator().outputs(a).is_none());
}

#[test]
fn drawing_frames_does_not_panic() {
    let mut ed = register_nodes![Source, MyNode];
    let a = ed.add_node("it.source", Default::default()).unwrap();
    let b = ed.add_node("it.mynode", Default::default()).unwrap();
    ed.editor_mut().connect((a, 0), (b, 0));

    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    for _ in 0..3 {
        let raw = egui::RawInput {
            screen_rect: Some(rect),
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ctx| {
            let mut ui = egui::Ui::new(
                ctx.clone(),
                egui::Id::new("root"),
                egui::UiBuilder::new().max_rect(rect),
            );
            ed.draw(&mut ui);
        });
        // We have no painter to upload the font atlas to, so drop the
        // texture deltas on purpose.
        out.drop_without_applying_deltas();
    }
    // the graph still works after being drawn
    assert!(ed.evaluator().outputs(b).is_some());
}
