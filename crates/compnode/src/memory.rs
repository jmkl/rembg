//! Bounded cache for decoded full-resolution images.
//!
//! Preview textures are small and stay on their nodes. Full-resolution CPU
//! images are only needed while a processing node consumes them, so the least
//! recently used ones are released when the graph exceeds the budget. A
//! released image can be decoded again from its source or result path.

use std::cmp::Reverse;
use std::collections::HashMap;

use egui_snarl::{NodeId, Snarl};

use crate::assets::AssetStore;
use crate::model::Node;

/// Default budget for full-resolution CPU images across the whole graph.
const DEFAULT_LIMIT: usize = 512 * 1024 * 1024;

pub(crate) struct ImageMemory {
    limit: usize,
    clock: u64,
    last_used: HashMap<NodeId, u64>,
}

impl Default for ImageMemory {
    fn default() -> Self {
        Self::new(DEFAULT_LIMIT)
    }
}

impl ImageMemory {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            clock: 0,
            last_used: HashMap::new(),
        }
    }

    /// Record that a node's full-resolution image was used. The most recently
    /// used images are the last to be released.
    pub(crate) fn touch(&mut self, node: NodeId) {
        self.clock = self.clock.wrapping_add(1);
        self.last_used.insert(node, self.clock);
    }

    pub(crate) fn clear(&mut self) {
        self.clock = 0;
        self.last_used.clear();
    }

    /// Release old CPU images until the budget is met. Nodes whose image
    /// cannot be recreated from disk keep their CPU copy.
    pub(crate) fn trim(&mut self, snarl: &mut Snarl<Node>, assets: &AssetStore) {
        self.last_used
            .retain(|node, _| snarl.get_node_info(*node).is_some());

        let mut total = 0usize;
        let mut candidates = Vec::new();
        for (node, data) in snarl.node_ids() {
            let Some(bytes) = data.cpu_bytes() else {
                continue;
            };
            total += bytes;
            candidates.push((node, self.last_used.get(&node).copied().unwrap_or(0), bytes));
        }
        if total <= self.limit {
            return;
        }

        candidates.sort_by_key(|(node, used, bytes)| (*used, Reverse(*bytes), node.0));
        for (node, _, bytes) in candidates {
            if total <= self.limit {
                break;
            }
            if !can_reload_image(snarl, node, assets) {
                continue;
            }
            if snarl[node].release_cpu() {
                total = total.saturating_sub(bytes);
                self.last_used.remove(&node);
            }
        }
    }
}

fn can_reload_image(snarl: &egui_snarl::Snarl<Node>, node: NodeId, assets: &AssetStore) -> bool {
    if let Some(image) = snarl[node].image_node() {
        if image
            .asset_id
            .as_ref()
            .is_some_and(|id| assets.get(id).is_some())
        {
            return true;
        }
        if !image.path.is_empty() && std::path::Path::new(&image.path).is_file() {
            return true;
        }
        return false;
    }
    snarl[node].can_reload_image()
}
