//! Bounded graph command history for structural edits and node controls.

use std::time::{Duration, Instant};

use egui_snarl::{NodeId, Snarl};

use crate::assets::AssetStore;
use crate::graph::invalidate_node;
use crate::model::Node;

const HISTORY_LIMIT: usize = 100;
const SCRUB_MERGE_WINDOW: Duration = Duration::from_millis(500);

pub(crate) struct GraphSnapshot {
    pub(crate) snarl: Snarl<Node>,
    pub(crate) assets: AssetStore,
}

impl GraphSnapshot {
    pub(crate) fn capture(snarl: &Snarl<Node>, assets: &AssetStore) -> Self {
        Self {
            snarl: snarl.clone(),
            assets: assets.clone(),
        }
    }

    fn restore(mut self, snarl: &mut Snarl<Node>, assets: &mut AssetStore) {
        for (_, node) in snarl.nodes_ids_mut() {
            node.reset_transient_state();
        }
        for (_, node) in self.snarl.nodes_ids_mut() {
            node.reset_transient_state();
        }
        *snarl = self.snarl;
        *assets = self.assets;
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
        self.undo.truncate(HISTORY_LIMIT);
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
            self.undo.truncate(HISTORY_LIMIT);
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

fn restore_node(snarl: &mut Snarl<Node>, id: NodeId, mut node: Node, assets: &mut AssetStore) {
    node.reset_transient_state();
    if let Some(current) = snarl.get_node_mut(id) {
        *current = node;
        invalidate_node(snarl, id);
    }
    assets.retain_used(snarl);
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
