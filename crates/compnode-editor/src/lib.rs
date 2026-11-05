//! # compnode-editor
//!
//! The bridge between **egui** and **compnode-core**.
//!
//! The crate has three layers, and each one can be used on its own:
//!
//! | layer | crate | what it does |
//! |---|---|---|
//! | graph | [`compnode_core`] | the graph model: nodes, wires, geometry, spatial index |
//! | editor | [`compnode_ui`] | the headless editor: document, schemas, widgets, undo, draw list |
//! | bridge | `compnode-editor` | the egui front end: input, painting, node types, evaluation |
//!
//! ## Defining a node
//!
//! A node is a plain Rust struct. It owns its state, declares its sockets
//! and computes its outputs:
//!
//! ```
//! use compnode_editor::{CompnodeNode, NodeError, Payload, ProcessCtx};
//! use compnode_ui::schema::{Category, SocketDef};
//! use compnode_ui::value::{SocketType, Value};
//!
//! /// A node that scales a number.
//! #[derive(Default)]
//! struct Scale {
//!     factor: f32,
//! }
//!
//! impl CompnodeNode for Scale {
//!     fn id(&self) -> &'static str { "demo.scale" }
//!     fn title(&self) -> &str { "Scale" }
//!     fn category(&self) -> Category { Category::Utility }
//!
//!     fn inputs(&self) -> Vec<SocketDef> {
//!         vec![
//!             SocketDef::float("Value", 1.0),
//!             // the knob: its default comes from the node's own state
//!             SocketDef::float("Factor", self.factor).range(0.0, 10.0),
//!         ]
//!     }
//!
//!     fn outputs(&self) -> Vec<SocketDef> {
//!         vec![SocketDef::new("Value", SocketType::Float)]
//!     }
//!
//!     fn process(&mut self, ctx: &mut ProcessCtx) -> Result<Vec<Payload>, NodeError> {
//!         Ok(vec![Payload::value(Value::Float(ctx.float(0) * ctx.float(1)))])
//!     }
//!
//!     fn set_param(&mut self, key: &str, v: Value) -> bool {
//!         match key {
//!             "Factor" => { self.factor = v.as_f32().unwrap_or(self.factor); true }
//!             _ => false,
//!         }
//!     }
//! }
//! ```
//!
//! ## Using the editor
//!
//! ```
//! use compnode_editor::{CompnodeEditor, register_nodes};
//! # use compnode_editor::{CompnodeNode, Payload, ProcessCtx};
//! # use compnode_ui::schema::SocketDef;
//! # use compnode_ui::value::Value;
//! # #[derive(Default)]
//! # struct Scale { factor: f32 }
//! # impl CompnodeNode for Scale {
//! #     fn id(&self) -> &'static str { "demo.scale" }
//! #     fn title(&self) -> &str { "Scale" }
//! #     fn inputs(&self) -> Vec<SocketDef> { vec![SocketDef::float("Value", 1.0)] }
//! #     fn outputs(&self) -> Vec<SocketDef> { vec![] }
//! #     fn process(&mut self, ctx: &mut ProcessCtx) -> Result<Vec<Payload>, compnode_editor::NodeError> {
//! #         Ok(vec![Payload::value(Value::Float(ctx.float(0)))])
//! #     }
//! # }
//!
//! fn draw(ui: &mut egui::Ui) {
//!     let mut editor = register_nodes![Scale];
//!     editor.add_node("demo.scale", Default::default());
//!
//!     egui::CentralPanel::default().show(ui, |ui| {
//!         editor.draw(ui);
//!     });
//! }
//! ```
//!
//! ## How data flows
//!
//! The data flow follows the old `rembg/compnode` crate, with the image
//! type generalised into an opaque [`Payload`]:
//!
//! * a node **pulls** its inputs from the nodes wired into it,
//! * an input socket may carry several values (multi-input sockets),
//! * a node's result is cached and read by its consumers,
//! * an edit marks the node and everything downstream of it dirty, so only
//!   the branch that changed is recomputed.
//!
//! Parameter values (the numbers in the node body) live in the document, so
//! they survive undo, redo and saving. Everything else the node struct
//! holds is runtime state and is not part of the history.

#![forbid(unsafe_code)]

pub mod editor;
pub mod eval;
pub mod node;
pub mod paint;
pub mod types;

pub use editor::CompnodeEditor;
pub use eval::{Evaluator, Nodes};
pub use node::{CompnodeNode, NodeError, Payload, ProcessCtx};
pub use types::{NodeType, NodeTypes};

/// Build an editor with the given node types registered.
///
/// ```
/// # use compnode_editor::{CompnodeNode, Payload, ProcessCtx, register_nodes};
/// # use compnode_ui::schema::SocketDef;
/// # #[derive(Default)]
/// # struct A;
/// # impl CompnodeNode for A {
/// #     fn id(&self) -> &'static str { "a" }
/// #     fn title(&self) -> &str { "A" }
/// #     fn inputs(&self) -> Vec<SocketDef> { vec![] }
/// #     fn outputs(&self) -> Vec<SocketDef> { vec![] }
/// #     fn process(&mut self, _ctx: &mut ProcessCtx) -> Result<Vec<Payload>, compnode_editor::NodeError> { Ok(vec![]) }
/// # }
/// let editor = register_nodes![A];
/// assert_eq!(editor.types().len(), 1);
/// ```
#[macro_export]
macro_rules! register_nodes {
    ($($t:ty),+ $(,)?) => {{
        let mut editor = $crate::CompnodeEditor::new();
        $( editor.register::<$t>(); )+
        editor
    }};
}
