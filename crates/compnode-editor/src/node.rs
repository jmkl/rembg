//! The node-authoring API.
//!
//! A node is a plain Rust struct. The struct owns the node's state, the
//! editor asks it for its sockets, edits its parameters and runs its
//! `process` function. Values that travel along a wire are [`Payload`]s.
//!
//! ```
//! use compnode_editor::{CompnodeNode, Payload, ProcessCtx};
//! use compnode_ui::schema::{Category, SocketDef};
//! use compnode_ui::value::Value;
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
//!     fn inputs(&self) -> Vec<SocketDef> {
//!         vec![SocketDef::float("Value", 1.0)]
//!     }
//!     fn outputs(&self) -> Vec<SocketDef> {
//!         vec![SocketDef::new("Value", compnode_ui::value::SocketType::Float)]
//!     }
//!     fn process(&mut self, ctx: &mut ProcessCtx) -> Result<Vec<Payload>, compnode_editor::NodeError> {
//!         let v = ctx.float(0) * self.factor;
//!         Ok(vec![Payload::value(Value::Float(v))])
//!     }
//!     fn set_param(&mut self, key: &str, v: Value) -> bool {
//!         match key {
//!             "Value" => { self.factor = v.as_f32().unwrap_or(self.factor); true }
//!             _ => false,
//!         }
//!     }
//! }
//! ```
use compnode_core::Color;
use compnode_ui::schema::{Category, SocketDef};
use compnode_ui::value::{SocketType, Value};
use std::any::Any;
use std::sync::Arc;

/// A value travelling along a wire.
///
/// The payload is type-erased, so one editor can carry any kind of data
/// between nodes. The node that produced it knows the concrete type and
/// reads it back with [`Payload::get`].
///
/// Cloning a payload is cheap: the value is shared, not copied.
#[derive(Clone)]
pub struct Payload {
    inner: Arc<dyn Any + Send + Sync>,
    name: &'static str,
}

impl Payload {
    /// Wrap a value so it can travel on a wire.
    pub fn new<T: Any + Send + Sync>(value: T) -> Payload {
        Payload {
            inner: Arc::new(value),
            name: std::any::type_name::<T>(),
        }
    }

    /// A payload carrying a plain socket value.
    pub fn value(v: Value) -> Payload {
        Payload::new(v)
    }

    /// The value, if it has type `T`.
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.inner.as_ref().downcast_ref::<T>()
    }

    /// A clone of the value, if it has type `T`.
    pub fn cloned<T: Any + Clone>(&self) -> Option<T> {
        self.get::<T>().cloned()
    }

    /// The plain socket value, if the payload carries one.
    pub fn as_value(&self) -> Option<&Value> {
        self.get::<Value>()
    }

    /// The type name of the payload, for error messages.
    pub fn type_name(&self) -> &'static str {
        self.name
    }

    /// Does the payload hold a `T`?
    pub fn is<T: Any>(&self) -> bool {
        self.inner.as_ref().is::<T>()
    }
}

impl std::fmt::Debug for Payload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Payload({})", self.name)
    }
}

/// Why a node failed to produce its outputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeError {
    /// Nothing arrived on input `i`.
    MissingInput(usize),
    /// The value on input `i` has a different type than the node expected.
    TypeMismatch {
        input: usize,
        expected: &'static str,
        got: &'static str,
    },
    /// Anything else. The message is shown on the node.
    Failed(String),
}

impl std::fmt::Display for NodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeError::MissingInput(i) => write!(f, "input {i} is not connected"),
            NodeError::TypeMismatch {
                input,
                expected,
                got,
            } => write!(f, "input {input} is a {got}, expected {expected}"),
            NodeError::Failed(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for NodeError {}

/// What a node sees while it computes its outputs.
///
/// Inputs hold whatever arrived on the wires; a socket that is not connected
/// falls back to the value the user typed into it.
pub struct ProcessCtx<'a> {
    pub(crate) inputs: &'a [Vec<Payload>],
    pub(crate) values: &'a [Value],
    pub(crate) canvas: (u32, u32),
    pub(crate) node: u64,
}

impl ProcessCtx<'_> {
    /// Every payload connected to input `i`, in connection order. Multi-input
    /// sockets have more than one.
    pub fn payloads(&self, i: usize) -> &[Payload] {
        self.inputs.get(i).map_or(&[], Vec::as_slice)
    }

    /// The first payload on input `i`.
    pub fn payload(&self, i: usize) -> Option<&Payload> {
        self.payloads(i).first()
    }

    /// The value on input `i`, if it has type `T`.
    pub fn get<T: Any>(&self, i: usize) -> Option<&T> {
        self.payload(i).and_then(Payload::get)
    }

    /// A clone of the value on input `i`, if it has type `T`.
    pub fn cloned<T: Any + Clone>(&self, i: usize) -> Option<T> {
        self.payload(i).and_then(Payload::cloned)
    }

    /// Every value on input `i` that has type `T` (multi-input sockets).
    pub fn all<T: Any + Clone>(&self, i: usize) -> Vec<T> {
        self.payloads(i)
            .iter()
            .filter_map(|p| p.cloned::<T>())
            .collect()
    }

    /// The value on input `i`: the one on the wire when it is a plain socket
    /// value, otherwise the value the user typed into the socket.
    pub fn value(&self, i: usize) -> Value {
        if let Some(v) = self.payload(i).and_then(Payload::as_value) {
            return v.clone();
        }
        self.values.get(i).cloned().unwrap_or(Value::Float(0.0))
    }

    pub fn float(&self, i: usize) -> f32 {
        self.value(i).as_f32().unwrap_or(0.0)
    }

    pub fn int(&self, i: usize) -> i32 {
        match self.value(i).convert(SocketType::Int) {
            Some(Value::Int(v)) => v,
            _ => 0,
        }
    }

    pub fn bool(&self, i: usize) -> bool {
        matches!(
            self.value(i).convert(SocketType::Bool),
            Some(Value::Bool(true))
        )
    }

    pub fn color(&self, i: usize) -> Color {
        match self.value(i).convert(SocketType::Color) {
            Some(Value::Color(c)) => c,
            _ => Color::BLACK,
        }
    }

    pub fn string(&self, i: usize) -> String {
        match self.value(i) {
            Value::String(s) => s,
            _ => String::new(),
        }
    }

    pub fn enum_index(&self, i: usize) -> usize {
        match self.value(i) {
            Value::Enum(e) => e,
            _ => 0,
        }
    }

    /// The size of the canvas the graph works on.
    pub fn canvas(&self) -> (u32, u32) {
        self.canvas
    }

    /// The uid of the node being processed.
    pub fn node_id(&self) -> u64 {
        self.node
    }
}

/// A node the user defines.
///
/// One instance of the implementing type exists per node in the graph, so
/// the struct can hold whatever state the node needs (cached images, the
/// last result, settings the user typed in).
///
/// The trait is object-safe: the editor stores nodes as
/// `Box<dyn CompnodeNode>`. Register the type with
/// [`crate::CompnodeEditor::register`], which needs it to be `Default`;
/// use [`crate::CompnodeEditor::register_with`] for a custom constructor.
pub trait CompnodeNode: Send + Sync + 'static {
    /// Stable id, unique within an editor. It is written to project files,
    /// so it must not change between releases.
    fn id(&self) -> &'static str;

    /// The title shown on the node header.
    fn title(&self) -> &str;

    /// The header colour category.
    fn category(&self) -> Category {
        Category::Utility
    }

    /// The input sockets, in order. Socket `i` is pin `i`.
    ///
    /// A socket with a widget shows that widget while it is not connected;
    /// the value the user enters is stored in the document, so it survives
    /// undo, redo and saving.
    fn inputs(&self) -> Vec<SocketDef> {
        Vec::new()
    }

    /// The output sockets, in order. Socket `i` is pin `i`.
    fn outputs(&self) -> Vec<SocketDef> {
        Vec::new()
    }

    /// Body rows without pins: image previews, buttons, and anything else
    /// that is not a socket. They are drawn below the pins.
    fn footer(&self) -> Vec<SocketDef> {
        Vec::new()
    }

    /// Compute the node's outputs from the values on its inputs.
    ///
    /// The returned vector is indexed by output socket.
    fn process(&mut self, ctx: &mut ProcessCtx) -> Result<Vec<Payload>, NodeError>;

    /// A socket value changed. Mirror it into the node's own state so
    /// `process` can use it.
    ///
    /// `key` is the socket's name. Return `true` when the value was stored.
    fn set_param(&mut self, _key: &str, _value: Value) -> bool {
        false
    }

    /// A button in the body was pressed. `key` is the button's name.
    fn on_button(&mut self, _key: &str) {}
}

/// Build the schema of a node from the node itself.
pub(crate) fn schema_of(node: &dyn CompnodeNode) -> compnode_ui::schema::NodeSchema {
    use compnode_ui::schema::NodeSchema;
    let mut s = NodeSchema::new(node.id(), node.title(), node.category());
    for i in node.inputs() {
        s = s.input(i);
    }
    for o in node.outputs() {
        s = s.output(o);
    }
    for f in node.footer() {
        s = s.footer(f);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_round_trips_values() {
        let p = Payload::new(42u32);
        assert_eq!(p.cloned::<u32>(), Some(42));
        assert!(p.is::<u32>());
        assert!(!p.is::<f32>());
        assert_eq!(p.type_name(), "u32");
    }

    #[test]
    fn payload_carries_plain_values() {
        let p = Payload::value(Value::Float(1.5));
        assert_eq!(p.as_value(), Some(&Value::Float(1.5)));
        assert_eq!(p.cloned::<Value>(), Some(Value::Float(1.5)));
    }

    #[test]
    fn payload_clone_shares_the_value() {
        let p = Payload::new(vec![1u8, 2, 3]);
        let q = p.clone();
        assert_eq!(q.cloned::<Vec<u8>>(), Some(vec![1, 2, 3]));
    }
}
