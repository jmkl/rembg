//! Bounded graph command history for structural edits and node controls.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use egui_snarl::{NodeId, Snarl};

use crate::assets::AssetStore;
use crate::graph::invalidate_node;
use crate::model::Node;

const HISTORY_LIMIT: usize = 100;
const SCRUB_MERGE_WINDOW: Duration = Duration::from_millis(500);

/// Transform of one viewer layer: `(source node, center, size, angle)`.
pub(crate) type LayerPose = (NodeId, egui::Vec2, egui::Vec2, f32);

/// Every layer of a viewer node (Output/Group/Blend), bottom to top.
pub(crate) type LayerState = Vec<LayerPose>;

/// Read the layer transforms and order of a node, if it is a viewer.
pub(crate) fn layer_state(node: &Node) -> Option<LayerState> {
    node.layers().map(|layers| {
        layers
            .iter()
            .map(|l| (l.source, l.center, l.size, l.angle))
            .collect()
    })
}

/// Write `state` onto the layers of `node`: transforms by source id, then
/// order. Layers the state does not mention keep their values.
pub(crate) fn apply_layer_state(node: &mut Node, state: &[LayerPose]) -> bool {
    let Some(layers) = node.layers_mut() else {
        return false;
    };
    for layer in layers.iter_mut() {
        if let Some((_, center, size, angle)) = state.iter().find(|(id, ..)| *id == layer.source) {
            layer.center = *center;
            layer.size = *size;
            layer.angle = *angle;
        }
    }
    layers.sort_by_key(|l| {
        state
            .iter()
            .position(|(id, ..)| *id == l.source)
            .unwrap_or(usize::MAX)
    });
    true
}

/// Layer transforms of every viewer node in the graph.
fn capture_poses(snarl: &Snarl<Node>) -> HashMap<NodeId, LayerState> {
    snarl
        .node_ids()
        .filter_map(|(id, node)| layer_state(node).map(|state| (id, state)))
        .collect()
}

/// Put previously captured layer transforms back after a graph or node
/// restore. Canvas transforms live in their own history, so undoing a graph
/// edit must never move or reorder layers.
fn reapply_poses(snarl: &mut Snarl<Node>, poses: &HashMap<NodeId, LayerState>) {
    for (id, state) in poses {
        if let Some(node) = snarl.get_node_mut(*id) {
            apply_layer_state(node, state);
        }
    }
}

pub(crate) struct GraphSnapshot {
    pub(crate) snarl: Snarl<Node>,
    pub(crate) assets: AssetStore,
}

impl GraphSnapshot {
    pub(crate) fn capture(snarl: &Snarl<Node>, assets: &AssetStore) -> Self {
        let mut snarl = snarl.clone();
        for (_, node) in snarl.nodes_ids_mut() {
            node.release_cpu_if_reloadable();
        }
        Self {
            snarl,
            assets: assets.clone(),
        }
    }

    fn restore(mut self, snarl: &mut Snarl<Node>, assets: &mut AssetStore) {
        let poses = capture_poses(snarl);
        for (_, node) in snarl.nodes_ids_mut() {
            node.reset_transient_state();
        }
        for (_, node) in self.snarl.nodes_ids_mut() {
            node.reset_transient_state();
        }
        *snarl = self.snarl;
        *assets = self.assets;
        reapply_poses(snarl, &poses);
        assets.retain_used(snarl);
    }
}

enum Change {
    Graph {
        before: GraphSnapshot,
        after: GraphSnapshot,
    },
    Node {
        id: NodeId,
        before: Node,
        after: Node,
        changed_at: Instant,
    },
}

#[derive(Default)]
pub(crate) struct GraphHistory {
    undo: Vec<Change>,
    redo: Vec<Change>,
}

impl GraphHistory {
    pub(crate) fn record_graph(&mut self, before: GraphSnapshot, after: GraphSnapshot) {
        self.undo.push(Change::Graph { before, after });
        cap_oldest(&mut self.undo);
        self.redo.clear();
    }

    pub(crate) fn record_node(&mut self, id: NodeId, before: Node, after: Node) {
        let now = Instant::now();
        if let Some(Change::Node {
            id: previous_id,
            after: previous_after,
            changed_at,
            ..
        }) = self.undo.last_mut()
            && *previous_id == id
            && now.duration_since(*changed_at) <= SCRUB_MERGE_WINDOW
        {
            *previous_after = after;
            *changed_at = now;
        } else {
            self.undo.push(Change::Node {
                id,
                before,
                after,
                changed_at: now,
            });
            cap_oldest(&mut self.undo);
        }
        self.redo.clear();
    }

    pub(crate) fn undo(&mut self, snarl: &mut Snarl<Node>, assets: &mut AssetStore) -> bool {
        let Some(change) = self.undo.pop() else {
            return false;
        };
        match &change {
            Change::Graph { before, .. } => {
                let snapshot = GraphSnapshot::capture(&before.snarl, &before.assets);
                snapshot.restore(snarl, assets);
            }
            Change::Node { id, before, .. } => restore_node(snarl, *id, before.clone(), assets),
        }
        self.redo.push(change);
        true
    }

    pub(crate) fn redo(&mut self, snarl: &mut Snarl<Node>, assets: &mut AssetStore) -> bool {
        let Some(change) = self.redo.pop() else {
            return false;
        };
        match &change {
            Change::Graph { after, .. } => {
                let snapshot = GraphSnapshot::capture(&after.snarl, &after.assets);
                snapshot.restore(snarl, assets);
            }
            Change::Node { id, after, .. } => restore_node(snarl, *id, after.clone(), assets),
        }
        self.undo.push(change);
        true
    }

    pub(crate) fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

/// Keep the newest `HISTORY_LIMIT` entries. (`Vec::truncate` would keep the
/// oldest ones and drop the newest edit instead.)
fn cap_oldest(stack: &mut Vec<Change>) {
    if stack.len() > HISTORY_LIMIT {
        let excess = stack.len() - HISTORY_LIMIT;
        stack.drain(..excess);
    }
}

fn restore_node(snarl: &mut Snarl<Node>, id: NodeId, mut node: Node, assets: &mut AssetStore) {
    node.reset_transient_state();
    if let Some(current) = snarl.get_node_mut(id) {
        // Layer transforms belong to the canvas history, not to this one.
        if let Some(state) = layer_state(current) {
            apply_layer_state(&mut node, &state);
        }
        *current = node;
        invalidate_node(snarl, id);
    }
    assets.retain_used(snarl);
}

/// One committed canvas edit of a viewer node's layers.
struct CanvasChange {
    before: LayerState,
    after: LayerState,
}

#[derive(Default)]
struct CanvasStack {
    undo: Vec<CanvasChange>,
    redo: Vec<CanvasChange>,
}

/// Undo history of the layer canvas (move / rotate / scale / reorder).
///
/// It is kept apart from [`GraphHistory`] and split per viewer node, so the
/// canvas of the active node only ever undoes its own transforms, and the
/// node graph never undoes canvas edits.
#[derive(Default)]
pub(crate) struct CanvasHistory {
    stacks: HashMap<NodeId, CanvasStack>,
}

impl CanvasHistory {
    pub(crate) fn record(&mut self, node: NodeId, before: LayerState, after: LayerState) {
        if before == after {
            return;
        }
        let stack = self.stacks.entry(node).or_default();
        stack.undo.push(CanvasChange { before, after });
        if stack.undo.len() > HISTORY_LIMIT {
            let excess = stack.undo.len() - HISTORY_LIMIT;
            stack.undo.drain(..excess);
        }
        stack.redo.clear();
    }

    /// Undo the newest canvas edit of `node`; returns the state that was
    /// applied to the node's layers.
    pub(crate) fn undo(&mut self, snarl: &mut Snarl<Node>, node: NodeId) -> Option<LayerState> {
        let stack = self.stacks.get_mut(&node)?;
        let change = stack.undo.pop()?;
        let applied = snarl
            .get_node_mut(node)
            .is_some_and(|n| apply_layer_state(n, &change.before));
        let state = applied.then(|| change.before.clone());
        stack.redo.push(change);
        state
    }

    pub(crate) fn redo(&mut self, snarl: &mut Snarl<Node>, node: NodeId) -> Option<LayerState> {
        let stack = self.stacks.get_mut(&node)?;
        let change = stack.redo.pop()?;
        let applied = snarl
            .get_node_mut(node)
            .is_some_and(|n| apply_layer_state(n, &change.after));
        let state = applied.then(|| change.after.clone());
        stack.undo.push(change);
        state
    }

    pub(crate) fn clear(&mut self) {
        self.stacks.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{GraphHistory, GraphSnapshot};
    use crate::assets::AssetStore;
    use crate::model::{FilterKind, Node, NodeTrait};
    use egui_snarl::Snarl;
    use serde_json::json;

    #[test]
    fn graph_snapshot_undoes_and_redoes_node_addition() {
        let mut snarl = Snarl::new();
        let mut assets = AssetStore::default();
        let mut history = GraphHistory::default();
        let before = GraphSnapshot::capture(&snarl, &assets);
        snarl.insert_node(egui::Pos2::ZERO, Node::new_filter(FilterKind::Hsv));
        history.record_graph(before, GraphSnapshot::capture(&snarl, &assets));

        assert!(history.undo(&mut snarl, &mut assets));
        assert_eq!(snarl.node_ids().count(), 0);
        assert!(history.redo(&mut snarl, &mut assets));
        assert_eq!(snarl.node_ids().count(), 1);
    }

    #[test]
    fn history_drops_oldest_entries_when_full() {
        let mut snarl = Snarl::new();
        let mut assets = AssetStore::default();
        let mut history = GraphHistory::default();
        for _ in 0..(super::HISTORY_LIMIT + 5) {
            let before = GraphSnapshot::capture(&snarl, &assets);
            snarl.insert_node(egui::Pos2::ZERO, Node::new_filter(FilterKind::Hsv));
            history.record_graph(before, GraphSnapshot::capture(&snarl, &assets));
        }
        assert_eq!(history.undo.len(), super::HISTORY_LIMIT);
        // The newest edit must still be undoable.
        assert!(history.undo(&mut snarl, &mut assets));
        assert_eq!(snarl.node_ids().count(), super::HISTORY_LIMIT + 4);
    }

    #[test]
    fn canvas_history_is_isolated_per_node_and_from_the_graph() {
        use super::{CanvasHistory, layer_state};
        let mut snarl = Snarl::new();
        let mut assets = AssetStore::default();
        let mut graph = GraphHistory::default();
        let mut canvas = CanvasHistory::default();
        let source = snarl.insert_node(egui::Pos2::ZERO, Node::new_filter(FilterKind::Hsv));
        let a = snarl.insert_node(egui::Pos2::ZERO, Node::viewer());
        let b = snarl.insert_node(egui::Pos2::ZERO, Node::viewer());
        for id in [a, b] {
            snarl[id]
                .layers_mut()
                .unwrap()
                .push(crate::model::ViewerLayer::new(
                    source,
                    crate::model::Kind::Output,
                    None,
                ));
        }

        let before = layer_state(&snarl[a]).unwrap();
        snarl[a].layers_mut().unwrap()[0].center = egui::vec2(50.0, 20.0);
        let after = layer_state(&snarl[a]).unwrap();
        canvas.record(a, before, after);

        // The other node and the graph history have nothing to undo.
        assert!(canvas.undo(&mut snarl, b).is_none());
        assert!(!graph.undo(&mut snarl, &mut assets));

        assert!(canvas.undo(&mut snarl, a).is_some());
        assert_eq!(snarl[a].layers().unwrap()[0].center, egui::Vec2::ZERO);
        assert!(canvas.redo(&mut snarl, a).is_some());
        assert_eq!(
            snarl[a].layers().unwrap()[0].center,
            egui::vec2(50.0, 20.0)
        );
    }

    #[test]
    fn node_history_restores_parameter_edits() {
        let mut snarl = Snarl::new();
        let mut assets = AssetStore::default();
        let mut history = GraphHistory::default();
        let id = snarl.insert_node(egui::Pos2::ZERO, Node::new_filter(FilterKind::Hsv));
        let before = snarl[id].clone();
        snarl[id]
            .parameters_mut()
            .unwrap()
            .insert("hue".into(), json!(90.0));
        let after = snarl[id].clone();
        history.record_node(id, before, after);

        assert!(history.undo(&mut snarl, &mut assets));
        assert_eq!(snarl[id].parameters().unwrap()["hue"], json!(0.0));
        assert!(history.redo(&mut snarl, &mut assets));
        assert_eq!(snarl[id].parameters().unwrap()["hue"], json!(90.0));
    }
}
