//! # compnode-ui
//!
//! Headless editor layer on top of [`compnode`]. Nothing here draws or opens
//! a window: the host feeds [`input::InputEvent`]s into an [`editor::Editor`]
//! and renders the [`draw::DrawList`] it produces.
//!
//! * [`value`], [`schema`]: socket types, values, node schemas, registry
//! * [`widget`], [`widgets`]: the widget trait and the default widget set
//! * [`document`], [`command`], [`selection`]: the editable model with undo/redo
//! * [`editor`]: interaction state machine and rendering to a draw list
//! * [`theme`], [`draw`], [`text`]: the design system and the draw-list boundary
//! * [`project`]: save/load; [`canvas`]: output viewer state

#![forbid(unsafe_code)]

pub mod canvas;
pub mod command;
pub mod document;
pub mod draw;
pub mod editor;
pub mod input;
pub mod minimap;
pub mod project;
pub mod schema;
pub mod selection;
pub mod text;
pub mod theme;
pub mod value;
pub mod widget;
pub mod widgets;

/// The graph engine this crate builds on.
pub use compnode_core as engine;

pub use document::{Change, Document, Link, NodeData};
pub use draw::{DrawList, Layer, Prim};
pub use editor::focus::{AccessItem, AccessRole};
pub use editor::{Editor, EditorAction};
pub use input::{InputEvent, Key, Modifiers, PointerButton};
pub use schema::{Category, NodeSchema, Registry, SocketDef, WidgetKind};
pub use theme::Theme;
pub use value::{ImageHandle, SocketType, Value};
