//! Backend-neutral node-graph model.
//!
//! Everything outside [`crate::view`] talks to the graph through this module:
//! ids, wires, node values and positions. No other module names the node-graph
//! widget crate, and `view::tests::widget_crate_is_confined_to_backend_modules`
//! keeps it that way.
//!
//! # Storage
//!
//! [`NodeGraph`] currently stores its data in egui-snarl's `Snarl`, because the
//! Snarl widget draws and edits exactly that type. The storage is private. To
//! replace the widget, implement a new `GraphView` backend, then swap the
//! `inner` field here for your own storage; the API below is what the rest of
//! the crate depends on, so no other file changes.
//!
//! The `from_snarl` / `into_snarl` / `snarl_mut` accessors and the `From`
//! conversions exist for the Snarl backend only.

use std::ops::{Index, IndexMut};

use egui::Pos2;
use egui_snarl::Snarl;

/// Identifies a node. Ids are never reused while the node exists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct NodeId(pub(crate) usize);

/// Identifies an input pin: a node and the index of the input on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct InPinId {
    pub(crate) node: NodeId,
    pub(crate) input: usize,
}

/// Identifies an output pin: a node and the index of the output on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct OutPinId {
    pub(crate) node: NodeId,
    pub(crate) output: usize,
}

/// An input pin and every output wired to it, in connection order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InPin {
    pub(crate) id: InPinId,
    pub(crate) remotes: Vec<OutPinId>,
}

/// An output pin and every input wired to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OutPin {
    pub(crate) id: OutPinId,
    pub(crate) remotes: Vec<InPinId>,
}

/// Position and collapsed state of a node (a copy; use the graph to change it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NodeInfo {
    pub(crate) pos: Pos2,
    /// `false` while the node is collapsed.
    pub(crate) open: bool,
}

/// The node graph: nodes of type `T`, their positions and the wires between pins.
#[derive(Clone)]
pub(crate) struct NodeGraph<T> {
    inner: Snarl<T>,
}

impl<T> Default for NodeGraph<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> NodeGraph<T> {
    pub(crate) fn new() -> Self {
        Self {
            inner: Snarl::new(),
        }
    }

    pub(crate) fn insert_node(&mut self, pos: Pos2, value: T) -> NodeId {
        self.inner.insert_node(pos, value).into()
    }

    /// Remove a node and every wire attached to it.
    pub(crate) fn remove_node(&mut self, id: NodeId) {
        self.inner.remove_node(id.into());
    }

    pub(crate) fn get_node(&self, id: NodeId) -> Option<&T> {
        self.inner.get_node(id.into())
    }

    pub(crate) fn get_node_mut(&mut self, id: NodeId) -> Option<&mut T> {
        self.inner.get_node_mut(id.into())
    }

    /// Position and collapsed state, or `None` when the node does not exist.
    pub(crate) fn get_node_info(&self, id: NodeId) -> Option<NodeInfo> {
        self.inner.get_node_info(id.into()).map(|info| NodeInfo {
            pos: info.pos,
            open: info.open,
        })
    }

    /// Collapse (`false`) or expand (`true`) a node.
    pub(crate) fn open_node(&mut self, id: NodeId, open: bool) {
        self.inner.open_node(id.into(), open);
    }

    pub(crate) fn node_ids(&self) -> impl Iterator<Item = (NodeId, &T)> {
        self.inner.node_ids().map(|(id, node)| (id.into(), node))
    }

    pub(crate) fn nodes_ids_mut(&mut self) -> impl Iterator<Item = (NodeId, &mut T)> {
        self.inner
            .nodes_ids_mut()
            .map(|(id, node)| (id.into(), node))
    }

    pub(crate) fn in_pin(&self, id: InPinId) -> InPin {
        let pin = self.inner.in_pin(id.into());
        InPin {
            id,
            remotes: pin.remotes.iter().map(|remote| (*remote).into()).collect(),
        }
    }

    pub(crate) fn out_pin(&self, id: OutPinId) -> OutPin {
        let pin = self.inner.out_pin(id.into());
        OutPin {
            id,
            remotes: pin.remotes.iter().map(|remote| (*remote).into()).collect(),
        }
    }

    /// Wire `from` to `to`. Returns whether a new wire was added. This is plain
    /// storage: replacing an existing wire on a single-input pin is the
    /// caller's job (see `graph::connect_input`).
    pub(crate) fn connect(&mut self, from: OutPinId, to: InPinId) -> bool {
        self.inner.connect(from.into(), to.into())
    }

    /// Remove the wire `from` -> `to`. Returns whether there was one.
    pub(crate) fn disconnect(&mut self, from: OutPinId, to: InPinId) -> bool {
        let existed = self.in_pin(to).remotes.contains(&from);
        self.inner.disconnect(from.into(), to.into());
        existed
    }

    /// Remove every wire attached to an input pin.
    pub(crate) fn drop_inputs(&mut self, pin: InPinId) {
        self.inner.drop_inputs(pin.into());
    }

    /// Remove every wire attached to an output pin.
    pub(crate) fn drop_outputs(&mut self, pin: OutPinId) {
        self.inner.drop_outputs(pin.into());
    }

    /// Every wire as `(output, input)`.
    pub(crate) fn wires(&self) -> impl Iterator<Item = (OutPinId, InPinId)> + '_ {
        self.inner
            .wires()
            .map(|(from, to)| (from.into(), to.into()))
    }
}

impl<T> Index<NodeId> for NodeGraph<T> {
    type Output = T;

    fn index(&self, id: NodeId) -> &T {
        &self.inner[egui_snarl::NodeId::from(id)]
    }
}

impl<T> IndexMut<NodeId> for NodeGraph<T> {
    fn index_mut(&mut self, id: NodeId) -> &mut T {
        &mut self.inner[egui_snarl::NodeId::from(id)]
    }
}

// --- Snarl backend support ------------------------------------------------

impl<T> NodeGraph<T> {
    /// Wrap a `Snarl`. Used by the Snarl backend inside its callbacks.
    pub(crate) fn from_snarl(inner: Snarl<T>) -> Self {
        Self { inner }
    }

    pub(crate) fn into_snarl(self) -> Snarl<T> {
        self.inner
    }

    /// The storage the Snarl widget draws and edits.
    pub(crate) fn snarl_mut(&mut self) -> &mut Snarl<T> {
        &mut self.inner
    }
}

impl From<egui_snarl::NodeId> for NodeId {
    fn from(id: egui_snarl::NodeId) -> Self {
        Self(id.0)
    }
}

impl From<NodeId> for egui_snarl::NodeId {
    fn from(id: NodeId) -> Self {
        egui_snarl::NodeId(id.0)
    }
}

impl From<egui_snarl::InPinId> for InPinId {
    fn from(id: egui_snarl::InPinId) -> Self {
        Self {
            node: id.node.into(),
            input: id.input,
        }
    }
}

impl From<InPinId> for egui_snarl::InPinId {
    fn from(id: InPinId) -> Self {
        egui_snarl::InPinId {
            node: id.node.into(),
            input: id.input,
        }
    }
}

impl From<egui_snarl::OutPinId> for OutPinId {
    fn from(id: egui_snarl::OutPinId) -> Self {
        Self {
            node: id.node.into(),
            output: id.output,
        }
    }
}

impl From<OutPinId> for egui_snarl::OutPinId {
    fn from(id: OutPinId) -> Self {
        egui_snarl::OutPinId {
            node: id.node.into(),
            output: id.output,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin_pair(from: NodeId, to: NodeId) -> (OutPinId, InPinId) {
        (
            OutPinId {
                node: from,
                output: 0,
            },
            InPinId { node: to, input: 0 },
        )
    }

    #[test]
    fn nodes_wires_and_pins_round_trip() {
        let mut graph: NodeGraph<&str> = NodeGraph::new();
        let a = graph.insert_node(Pos2::new(1.0, 2.0), "a");
        let b = graph.insert_node(Pos2::new(3.0, 4.0), "b");
        let (out, input) = pin_pair(a, b);

        assert!(graph.connect(out, input));
        assert_eq!(graph.in_pin(input).remotes, vec![out]);
        assert_eq!(graph.out_pin(out).remotes, vec![input]);
        assert_eq!(graph.wires().collect::<Vec<_>>(), vec![(out, input)]);
        assert_eq!(graph[a], "a");
        assert_eq!(
            graph.get_node_info(b).map(|info| info.pos),
            Some(Pos2::new(3.0, 4.0))
        );

        assert!(graph.disconnect(out, input));
        assert!(!graph.disconnect(out, input));
        assert!(graph.wires().next().is_none());
    }

    #[test]
    fn open_state_and_removal() {
        let mut graph: NodeGraph<u8> = NodeGraph::new();
        let a = graph.insert_node(Pos2::ZERO, 1);
        assert_eq!(graph.get_node_info(a).map(|info| info.open), Some(true));
        graph.open_node(a, false);
        assert_eq!(graph.get_node_info(a).map(|info| info.open), Some(false));

        graph.remove_node(a);
        assert!(graph.get_node(a).is_none());
        assert!(graph.get_node_info(a).is_none());
    }

    #[test]
    fn clone_is_independent() {
        let mut graph: NodeGraph<u8> = NodeGraph::new();
        let a = graph.insert_node(Pos2::ZERO, 1);
        let copy = graph.clone();
        graph[a] = 9;
        assert_eq!(copy[a], 1);
    }
}
