//! # nodeforge
//!
//! A headless, renderer-agnostic node-graph engine designed so that per-frame
//! cost scales with **what is on screen**, not with graph size.
//!
//! * [`arena`]  – generational arena (stable, reusable, ABA-safe ids)
//! * [`geom`]   – tiny `Vec2` / `Rect` types
//! * [`hash`]   – fast non-cryptographic hasher for integer keys
//! * [`grid`]   – incremental uniform spatial grid
//! * [`wire`]   – bezier wire geometry, adaptive flattening, hit testing, cache
//! * [`layout`] – node/pin geometry constants
//! * [`graph`]  – the graph model (nodes, wires, adjacency, cycle checks, indices)
//! * [`view`]   – pan/zoom viewport and level-of-detail selection

pub mod arena;
pub mod geom;
pub mod graph;
pub mod grid;
pub mod hash;
pub mod layout;
pub mod view;
pub mod wire;

pub use arena::Key;
pub use geom::{Rect, Vec2};
pub use graph::{ConnectError, Graph, InPin, Node, NodeId, OutPin, PinRef, Visible, Wire, WireId};
pub use view::{Lod, Viewport};
pub use wire::WireCache;
