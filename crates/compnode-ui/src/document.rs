//! The editable document: a node graph plus the data the UI layer attaches
//! to it. Nodes have a stable `uid` that survives delete/undo/redo (arena
//! ids do not), so commands and the clipboard refer to nodes by uid.
use crate::schema::{FRAME_ID, NodeSchema, REROUTE_ID, Registry};
use crate::value::Value;
use compnode_core::geom::vec2;
use compnode_core::hash::FxMap;
use compnode_core::{ConnectError, Graph, InPin, NodeId, OutPin, Vec2, WireId};

/// One layer of a multi-input canvas node (the Output node and friends): a
/// source node plus how it is placed on the canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    /// uid of the node feeding this layer.
    pub source: u64,
    /// Center on the canvas, in canvas pixels (0,0 = top-left corner).
    pub center: [f32; 2],
    /// Size on the canvas in canvas pixels; `0,0` means "fit the source".
    pub size: [f32; 2],
    /// Rotation in degrees.
    pub angle: f32,
}

impl Layer {
    pub fn new(source: u64) -> Layer {
        Layer {
            source,
            center: [0.0, 0.0],
            size: [0.0, 0.0],
            angle: 0.0,
        }
    }
}

/// Data stored on every graph node.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeData {
    pub uid: u64,
    pub schema: String,
    /// One value per input socket (used while the socket is unconnected).
    pub values: Vec<Value>,
    /// One value per output socket (previews and labels live here).
    pub out_values: Vec<Value>,
    pub muted: bool,
    pub collapsed: bool,
    pub title: Option<String>,
    /// Content height of a frame node (ignored by other nodes).
    pub frame_h: f32,
    /// Per-layer transforms for a multi-input canvas node, keyed by source
    /// node uid. The stacking order is the order of this list (bottom first).
    /// Wires that are connected but have no entry use identity transforms.
    pub layers: Vec<Layer>,
}

impl NodeData {
    pub fn is_reroute(&self) -> bool {
        self.schema == REROUTE_ID
    }
    pub fn is_frame(&self) -> bool {
        self.schema == FRAME_ID
    }
}

/// A connection between two nodes, by uid.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Link {
    pub from: (u64, usize),
    pub to: (u64, usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeSnapshot {
    pub uid: u64,
    pub schema: String,
    pub pos: Vec2,
    pub width: f32,
    pub frame_h: f32,
    pub values: Vec<Value>,
    pub out_values: Vec<Value>,
    pub muted: bool,
    pub collapsed: bool,
    pub title: Option<String>,
    pub layers: Vec<Layer>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Snapshot {
    pub nodes: Vec<NodeSnapshot>,
    pub links: Vec<Link>,
}

/// What changed, for dirty tracking in an evaluator.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// An unconnected input value changed.
    Value { uid: u64, input: usize },
    /// Nodes were added or removed, or links changed. Lists the nodes whose
    /// inputs may differ now (and removed uids, which no longer exist).
    Structure { uids: Vec<u64> },
    /// Mute or collapse flag toggled.
    Flag { uid: u64 },
}

pub const FRAME_DEFAULT_H: f32 = 200.0;

#[derive(Debug)]
pub struct Document {
    pub graph: Graph<NodeData>,
    pub registry: Registry,
    uid_map: FxMap<u64, NodeId>,
    next_uid: u64,
    /// Bumped by every mutation.
    pub revision: u64,
    changes: Vec<Change>,
    layout_dirty: Vec<u64>,
}

impl Document {
    pub fn new(registry: Registry) -> Document {
        Document {
            graph: Graph::new(),
            registry,
            uid_map: FxMap::default(),
            next_uid: 1,
            revision: 0,
            changes: Vec::new(),
            layout_dirty: Vec::new(),
        }
    }

    pub fn alloc_uid(&mut self) -> u64 {
        let u = self.next_uid;
        self.next_uid += 1;
        u
    }
    /// Make sure future uids are above `uid` (used when loading files).
    pub fn reserve_uid(&mut self, uid: u64) {
        self.next_uid = self.next_uid.max(uid + 1);
    }
    pub fn next_uid(&self) -> u64 {
        self.next_uid
    }

    pub fn id_of(&self, uid: u64) -> Option<NodeId> {
        self.uid_map.get(&uid).copied()
    }
    pub fn uid_of(&self, id: NodeId) -> Option<u64> {
        self.graph.node(id).map(|n| n.data.uid)
    }
    pub fn contains(&self, uid: u64) -> bool {
        self.uid_map.contains_key(&uid)
    }
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }
    pub fn data(&self, uid: u64) -> Option<&NodeData> {
        self.graph.node(self.id_of(uid)?).map(|n| &n.data)
    }
    pub fn schema_of(&self, uid: u64) -> Option<&NodeSchema> {
        self.registry.get(&self.data(uid)?.schema)
    }
    /// All uids, sorted (deterministic).
    pub fn uids(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.uid_map.keys().copied().collect();
        v.sort_unstable();
        v
    }

    pub fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.changes)
    }
    pub fn take_layout_dirty(&mut self) -> Vec<u64> {
        let mut v = std::mem::take(&mut self.layout_dirty);
        v.sort_unstable();
        v.dedup();
        v
    }
    fn touch(&mut self) {
        self.revision += 1;
    }
    fn mark_layout(&mut self, uid: u64) {
        self.layout_dirty.push(uid);
    }

    /// Describe a new node of `schema_id` (default values, fresh uid)
    /// without inserting it.
    pub fn new_snapshot(&mut self, schema_id: &str, pos: Vec2) -> Option<NodeSnapshot> {
        let schema = self.registry.get(schema_id)?;
        let snap = NodeSnapshot {
            uid: 0,
            schema: schema.id.clone(),
            pos,
            width: schema.width,
            frame_h: FRAME_DEFAULT_H,
            values: schema.default_values(),
            out_values: schema.default_out_values(),
            muted: false,
            collapsed: false,
            title: None,
            layers: Vec::new(),
        };
        let uid = self.alloc_uid();
        Some(NodeSnapshot { uid, ..snap })
    }

    /// Create a node of `schema_id` with default values. Returns its uid.
    pub fn add_node(&mut self, schema_id: &str, pos: Vec2) -> Option<u64> {
        let snap = self.new_snapshot(schema_id, pos)?;
        self.insert_snapshot(&snap)?;
        Some(snap.uid)
    }

    /// Insert a node exactly as described (uid included).
    pub fn insert_snapshot(&mut self, s: &NodeSnapshot) -> Option<NodeId> {
        if self.uid_map.contains_key(&s.uid) {
            return None;
        }
        let schema = self.registry.get(&s.schema)?;
        let (ni, no) = (schema.inputs.len(), schema.outputs.len());
        let mut values = s.values.clone();
        let defaults = schema.default_values();
        values.resize(ni, Value::Float(0.0));
        for (i, v) in values.iter_mut().enumerate() {
            if v.socket_type() != defaults[i].socket_type() {
                *v = defaults[i].clone();
            }
        }
        let mut out_values = s.out_values.clone();
        let out_defaults = schema.default_out_values();
        out_values.resize(out_defaults.len(), Value::Float(0.0));
        for (i, v) in out_values.iter_mut().enumerate() {
            if v.socket_type() != out_defaults[i].socket_type() {
                *v = out_defaults[i].clone();
            }
        }
        let data = NodeData {
            uid: s.uid,
            schema: s.schema.clone(),
            values,
            out_values,
            muted: s.muted,
            collapsed: s.collapsed,
            title: s.title.clone(),
            frame_h: s.frame_h,
            layers: s.layers.clone(),
        };
        let id = self.graph.add_node(data, s.pos, s.width, ni, no);
        self.uid_map.insert(s.uid, id);
        self.reserve_uid(s.uid);
        self.mark_layout(s.uid);
        self.changes.push(Change::Structure { uids: vec![s.uid] });
        self.touch();
        Some(id)
    }

    pub fn snapshot_node(&self, uid: u64) -> Option<NodeSnapshot> {
        let id = self.id_of(uid)?;
        let n = self.graph.node(id)?;
        Some(NodeSnapshot {
            uid,
            schema: n.data.schema.clone(),
            pos: n.pos(),
            width: n.width(),
            frame_h: n.data.frame_h,
            values: n.data.values.clone(),
            out_values: n.data.out_values.clone(),
            muted: n.data.muted,
            collapsed: n.data.collapsed,
            title: n.data.title.clone(),
            layers: n.data.layers.clone(),
        })
    }

    pub fn link_of_wire(&self, w: WireId) -> Option<Link> {
        let wire = self.graph.wire(w)?;
        Some(Link {
            from: (self.uid_of(wire.from.node)?, wire.from.index),
            to: (self.uid_of(wire.to.node)?, wire.to.index),
        })
    }

    /// Every link in the document, sorted (deterministic).
    pub fn all_links(&self) -> Vec<Link> {
        let mut v: Vec<Link> = self
            .graph
            .wires()
            .filter_map(|(id, _)| self.link_of_wire(id))
            .collect();
        v.sort_by_key(|l| (l.from, l.to));
        v
    }

    /// Links attached to `uid` (either end), sorted.
    pub fn links_of(&self, uid: u64) -> Vec<Link> {
        let Some(id) = self.id_of(uid) else {
            return Vec::new();
        };
        let Some(n) = self.graph.node(id) else {
            return Vec::new();
        };
        let mut v = Vec::new();
        for i in 0..n.inputs() {
            if let Some(l) = n.input_wire(i).and_then(|w| self.link_of_wire(w)) {
                v.push(l);
            }
        }
        for o in 0..n.outputs() {
            for &w in n.output_wires(o) {
                if let Some(l) = self.link_of_wire(w) {
                    v.push(l);
                }
            }
        }
        v.sort_by_key(|l| (l.from, l.to));
        v
    }

    /// The link feeding input `input` of `uid`, if any.
    pub fn input_link(&self, uid: u64, input: usize) -> Option<Link> {
        let n = self.graph.node(self.id_of(uid)?)?;
        self.link_of_wire(n.input_wire(input)?)
    }

    /// Every link feeding input `input` of `uid`, in connection order.
    pub fn input_links(&self, uid: u64, input: usize) -> Vec<Link> {
        let Some(n) = self.id_of(uid).and_then(|id| self.graph.node(id)) else {
            return Vec::new();
        };
        n.input_wires(input)
            .iter()
            .filter_map(|&w| self.link_of_wire(w))
            .collect()
    }

    /// Snapshot of nodes plus their links. With `external` the links that
    /// leave the set are included too (needed to undo a deletion); without
    /// it only links between the nodes are (copy and paste).
    pub fn snapshot_nodes(&self, uids: &[u64], external: bool) -> Snapshot {
        let mut sorted: Vec<u64> = uids.iter().copied().filter(|u| self.contains(*u)).collect();
        sorted.sort_unstable();
        sorted.dedup();
        let set: FxMap<u64, ()> = sorted.iter().map(|u| (*u, ())).collect();
        let nodes: Vec<NodeSnapshot> = sorted
            .iter()
            .filter_map(|u| self.snapshot_node(*u))
            .collect();
        let mut links: Vec<Link> = Vec::new();
        for &u in &sorted {
            for l in self.links_of(u) {
                let inside = set.contains_key(&l.from.0) && set.contains_key(&l.to.0);
                if (inside || external) && !links.contains(&l) {
                    links.push(l);
                }
            }
        }
        links.sort_by_key(|l| (l.from, l.to));
        Snapshot { nodes, links }
    }

    pub fn remove_node(&mut self, uid: u64) -> Option<NodeSnapshot> {
        let snap = self.snapshot_node(uid)?;
        let id = self.id_of(uid)?;
        let mut affected: Vec<u64> = vec![uid];
        for l in self.links_of(uid) {
            if l.to.0 != uid {
                affected.push(l.to.0);
                self.mark_layout(l.to.0);
            }
        }
        self.graph.remove_node(id);
        self.uid_map.remove(&uid);
        self.changes.push(Change::Structure { uids: affected });
        self.touch();
        Some(snap)
    }

    fn pins(&self, link: Link) -> Option<(OutPin, InPin)> {
        let f = self.id_of(link.from.0)?;
        let t = self.id_of(link.to.0)?;
        Some((
            OutPin {
                node: f,
                index: link.from.1,
            },
            InPin {
                node: t,
                index: link.to.1,
            },
        ))
    }

    /// Connect (replacing whatever feeds that input, unless it is a
    /// multi-input socket, in which case the wire is appended).
    pub fn connect(&mut self, link: Link) -> Result<(), ConnectError> {
        let (o, i) = self.pins(link).ok_or(ConnectError::NoSuchNode)?;
        if self.is_multi_input(link.to.0, link.to.1) {
            self.graph.connect(o, i)?;
        } else {
            self.graph.connect_replacing(o, i)?;
        }
        self.mark_layout(link.to.0);
        self.changes.push(Change::Structure {
            uids: vec![link.to.0],
        });
        self.touch();
        Ok(())
    }

    /// Whether input `input` of `uid` accepts multiple wires.
    pub fn is_multi_input(&self, uid: u64, input: usize) -> bool {
        self.schema_of(uid)
            .and_then(|s| s.inputs.get(input))
            .is_some_and(|d| d.multi)
    }

    /// Dry run of [`Document::connect`].
    pub fn can_connect(&self, link: Link) -> Result<(), ConnectError> {
        let (o, i) = self.pins(link).ok_or(ConnectError::NoSuchNode)?;
        self.graph.can_connect(o, i)
    }

    pub fn disconnect(&mut self, link: Link) -> bool {
        let Some(n) = self.id_of(link.to.0).and_then(|id| self.graph.node(id)) else {
            return false;
        };
        let Some(w) = n.input_wire(link.to.1) else {
            return false;
        };
        if self.link_of_wire(w) != Some(link) {
            return false;
        }
        self.graph.disconnect(w);
        self.mark_layout(link.to.0);
        self.changes.push(Change::Structure {
            uids: vec![link.to.0],
        });
        self.touch();
        true
    }

    /// Bulk-connect with one topological sort (file loading). On a cycle
    /// nothing is connected.
    pub fn load_links(&mut self, links: &[Link]) -> Result<(), ConnectError> {
        let mut pairs = Vec::with_capacity(links.len());
        for l in links {
            pairs.push(self.pins(*l).ok_or(ConnectError::NoSuchNode)?);
        }
        self.graph.load_edges(&pairs)?;
        let uids: Vec<u64> = links.iter().map(|l| l.to.0).collect();
        for &u in &uids {
            self.mark_layout(u);
        }
        self.changes.push(Change::Structure { uids });
        self.touch();
        Ok(())
    }

    pub fn set_value(&mut self, uid: u64, input: usize, v: Value) -> bool {
        let Some(id) = self.id_of(uid) else {
            return false;
        };
        let Some(d) = self.graph.node_data_mut(id) else {
            return false;
        };
        match d.values.get_mut(input) {
            Some(slot) => *slot = v,
            None => return false,
        }
        self.note_value_changed(uid, input);
        true
    }

    /// Record that a value was modified in place (by a widget).
    pub fn note_value_changed(&mut self, uid: u64, input: usize) {
        self.changes.push(Change::Value { uid, input });
        self.mark_layout(uid);
        self.touch();
    }

    /// Record that an output value (preview, label) was modified in place.
    pub fn note_out_value_changed(&mut self, uid: u64) {
        self.mark_layout(uid);
        self.touch();
    }

    pub fn set_out_value(&mut self, uid: u64, output: usize, v: Value) -> bool {
        let Some(id) = self.id_of(uid) else {
            return false;
        };
        let Some(d) = self.graph.node_data_mut(id) else {
            return false;
        };
        match d.out_values.get_mut(output) {
            Some(slot) => *slot = v,
            None => return false,
        }
        self.note_out_value_changed(uid);
        true
    }

    pub fn set_muted(&mut self, uid: u64, v: bool) -> bool {
        let Some(id) = self.id_of(uid) else {
            return false;
        };
        let Some(d) = self.graph.node_data_mut(id) else {
            return false;
        };
        d.muted = v;
        self.changes.push(Change::Flag { uid });
        self.touch();
        true
    }

    pub fn set_collapsed(&mut self, uid: u64, v: bool) -> bool {
        let Some(id) = self.id_of(uid) else {
            return false;
        };
        let Some(d) = self.graph.node_data_mut(id) else {
            return false;
        };
        d.collapsed = v;
        self.changes.push(Change::Flag { uid });
        self.mark_layout(uid);
        self.touch();
        true
    }

    pub fn set_pos(&mut self, uid: u64, pos: Vec2) -> bool {
        let Some(id) = self.id_of(uid) else {
            return false;
        };
        self.touch();
        self.graph.move_node(id, pos)
    }

    /// Resize (width; frames also their content height).
    pub fn set_size(&mut self, uid: u64, width: f32, frame_h: f32) -> bool {
        let Some(id) = self.id_of(uid) else {
            return false;
        };
        if let Some(d) = self.graph.node_data_mut(id) {
            d.frame_h = frame_h;
        }
        self.mark_layout(uid);
        self.touch();
        self.graph.set_width(id, width)
    }

    pub fn pos(&self, uid: u64) -> Option<Vec2> {
        self.graph.node(self.id_of(uid)?).map(|n| n.pos())
    }
    pub fn size(&self, uid: u64) -> Option<(f32, f32)> {
        let n = self.graph.node(self.id_of(uid)?)?;
        Some((n.width(), n.data.frame_h))
    }

    /// The layers of a multi-input canvas node (bottom-to-top).
    pub fn layers(&self, uid: u64) -> &[Layer] {
        self.data(uid).map_or(&[], |d| d.layers.as_slice())
    }

    /// The layers of `uid`, reconciled with its multi-input wires: one layer
    /// per connected source, preserving the existing order and transforms for
    /// sources that are still connected, appending new ones on top. Removed
    /// sources are dropped. `input` is the multi-input socket index.
    ///
    /// Returns `true` if the layer set changed (a caller can then invalidate).
    pub fn sync_layers(&mut self, uid: u64, input: usize) -> bool {
        let sources: Vec<u64> = self
            .input_links(uid, input)
            .into_iter()
            .map(|l| l.from.0)
            .collect();
        let Some(d) = self.id_of(uid).and_then(|id| self.graph.node_data_mut(id)) else {
            return false;
        };
        // Keep existing layers (in order) whose source is still connected, and
        // drop duplicates so a source appears once.
        let mut seen: Vec<u64> = Vec::new();
        let mut kept: Vec<Layer> = Vec::new();
        for l in &d.layers {
            if sources.contains(&l.source) && !seen.contains(&l.source) {
                seen.push(l.source);
                kept.push(*l);
            }
        }
        // Append any newly connected source on top.
        let mut added = false;
        for s in &sources {
            if !seen.contains(s) {
                seen.push(*s);
                kept.push(Layer::new(*s));
                added = true;
            }
        }
        let changed = added || kept.len() != d.layers.len();
        if changed {
            d.layers = kept;
        }
        changed
    }

    /// Replace the layer list of `uid` (used by reorder / transform edits).
    pub fn set_layers(&mut self, uid: u64, layers: Vec<Layer>) -> bool {
        let Some(d) = self.id_of(uid).and_then(|id| self.graph.node_data_mut(id)) else {
            return false;
        };
        if d.layers == layers {
            return false;
        }
        d.layers = layers;
        self.changes.push(Change::Flag { uid });
        self.touch();
        true
    }

    /// Update one layer's transform (by source uid); returns `false` if the
    /// layer does not exist.
    pub fn set_layer_transform(
        &mut self,
        uid: u64,
        source: u64,
        f: impl FnOnce(&mut Layer),
    ) -> bool {
        let Some(d) = self.id_of(uid).and_then(|id| self.graph.node_data_mut(id)) else {
            return false;
        };
        let Some(l) = d.layers.iter_mut().find(|l| l.source == source) else {
            return false;
        };
        f(l);
        self.changes.push(Change::Flag { uid });
        self.touch();
        true
    }

    /// Reconcile the layers of every multi-input node with its wires. Call
    /// after structural changes (connect / disconnect / load).
    pub fn sync_all_layers(&mut self) {
        let targets: Vec<(u64, usize)> = self
            .uids()
            .into_iter()
            .filter_map(|u| {
                let s = self.schema_of(u)?;
                s.inputs.iter().position(|d| d.multi).map(|i| (u, i))
            })
            .collect();
        for (uid, input) in targets {
            if self.sync_layers(uid, input) {
                self.changes.push(Change::Structure { uids: vec![uid] });
            }
        }
    }

    /// Bounding box of nodes, if any exist.
    pub fn bounds(&self, uids: &[u64]) -> Option<compnode_core::Rect> {
        let mut out: Option<compnode_core::Rect> = None;
        for &u in uids {
            if let Some(n) = self.id_of(u).and_then(|id| self.graph.node(id)) {
                let r = n.rect();
                out = Some(out.map_or(r, |o| o.union(&r)));
            }
        }
        out
    }
}

/// Default node position helper for tests and demos.
pub fn pos(x: f32, y: f32) -> Vec2 {
    vec2(x, y)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::schema::{Category, SocketDef};
    use crate::value::SocketType;

    /// Registry with a few small schemas used across the tests.
    pub fn registry() -> Registry {
        let mut r = Registry::new();
        r.register(
            NodeSchema::new("t.src", "Source", Category::Input)
                .input(SocketDef::float("Amount", 1.0).range(0.0, 10.0))
                .output(SocketDef::image("Image")),
        );
        r.register(
            NodeSchema::new("t.filter", "Filter", Category::Filter)
                .input(SocketDef::image("Image"))
                .input(SocketDef::float("Radius", 2.0).range(0.0, 100.0))
                .output(SocketDef::image("Image")),
        );
        r.register(
            NodeSchema::new("t.sink", "Sink", Category::Output).input(SocketDef::image("Image")),
        );
        r.register(
            NodeSchema::new("t.multi", "Multi", Category::Output)
                .input(SocketDef::image("Image").multi()),
        );
        r.register(
            NodeSchema::new("t.num", "Number", Category::Utility)
                .input(SocketDef::float("Value", 0.0))
                .output(SocketDef::new("Value", SocketType::Float)),
        );
        r
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::registry;
    use super::*;

    fn doc() -> Document {
        Document::new(registry())
    }

    #[test]
    fn add_node_uses_schema_defaults_and_unique_uids() {
        let mut d = doc();
        let a = d.add_node("t.filter", pos(0.0, 0.0)).unwrap();
        let b = d.add_node("t.filter", pos(300.0, 0.0)).unwrap();
        assert_ne!(a, b);
        assert_eq!(
            d.data(a).unwrap().values,
            vec![Value::Image(None), Value::Float(2.0)]
        );
        assert!(d.add_node("nope", pos(0.0, 0.0)).is_none());
        assert_eq!(d.node_count(), 2);
        assert_eq!(d.schema_of(a).unwrap().title, "Filter");
    }

    #[test]
    fn uid_survives_remove_and_reinsert() {
        let mut d = doc();
        let a = d.add_node("t.src", pos(0.0, 0.0)).unwrap();
        let b = d.add_node("t.filter", pos(300.0, 0.0)).unwrap();
        d.connect(Link {
            from: (a, 0),
            to: (b, 0),
        })
        .unwrap();
        let before = d.snapshot_nodes(&[a], true);
        let old_id = d.id_of(a).unwrap();
        d.remove_node(a).unwrap();
        assert!(!d.contains(a));
        assert!(d.links_of(b).is_empty());
        // reinserting gives a new arena id but the same uid
        for n in &before.nodes {
            d.insert_snapshot(n).unwrap();
        }
        for l in &before.links {
            d.connect(*l).unwrap();
        }
        assert!(d.contains(a));
        assert_ne!(d.id_of(a), Some(old_id));
        assert_eq!(d.links_of(b).len(), 1);
        // allocating after loading never reuses a uid
        let c = d.add_node("t.sink", pos(0.0, 0.0)).unwrap();
        assert!(c > b);
    }

    #[test]
    fn snapshot_links_internal_vs_external() {
        let mut d = doc();
        let a = d.add_node("t.src", pos(0.0, 0.0)).unwrap();
        let b = d.add_node("t.filter", pos(300.0, 0.0)).unwrap();
        let c = d.add_node("t.sink", pos(600.0, 0.0)).unwrap();
        d.connect(Link {
            from: (a, 0),
            to: (b, 0),
        })
        .unwrap();
        d.connect(Link {
            from: (b, 0),
            to: (c, 0),
        })
        .unwrap();
        assert_eq!(d.snapshot_nodes(&[a, b], false).links.len(), 1);
        assert_eq!(d.snapshot_nodes(&[a, b], true).links.len(), 2);
        assert_eq!(d.snapshot_nodes(&[b], true).links.len(), 2);
        assert_eq!(d.snapshot_nodes(&[b], false).links.len(), 0);
        assert_eq!(d.all_links().len(), 2);
    }

    #[test]
    fn changes_and_layout_dirty_are_recorded() {
        let mut d = doc();
        let a = d.add_node("t.src", pos(0.0, 0.0)).unwrap();
        let b = d.add_node("t.filter", pos(300.0, 0.0)).unwrap();
        d.take_changes();
        d.take_layout_dirty();
        d.connect(Link {
            from: (a, 0),
            to: (b, 0),
        })
        .unwrap();
        assert_eq!(d.take_changes(), vec![Change::Structure { uids: vec![b] }]);
        assert_eq!(d.take_layout_dirty(), vec![b]);
        assert!(d.set_value(b, 1, Value::Float(9.0)));
        assert_eq!(d.take_changes(), vec![Change::Value { uid: b, input: 1 }]);
        assert!(!d.set_value(b, 9, Value::Float(0.0)));
        d.set_muted(b, true);
        assert_eq!(d.take_changes(), vec![Change::Flag { uid: b }]);
        let rev = d.revision;
        d.set_pos(a, pos(5.0, 5.0));
        assert!(d.revision > rev);
    }

    #[test]
    fn disconnect_only_matching_link_and_cycles_rejected() {
        let mut d = doc();
        let a = d.add_node("t.filter", pos(0.0, 0.0)).unwrap();
        let b = d.add_node("t.filter", pos(300.0, 0.0)).unwrap();
        let l = Link {
            from: (a, 0),
            to: (b, 0),
        };
        d.connect(l).unwrap();
        assert_eq!(
            d.can_connect(Link {
                from: (b, 0),
                to: (a, 0)
            }),
            Err(ConnectError::Cycle)
        );
        assert!(!d.disconnect(Link {
            from: (a, 0),
            to: (b, 1)
        }));
        assert!(d.disconnect(l));
        assert!(!d.disconnect(l));
        assert_eq!(d.input_link(b, 0), None);
    }

    #[test]
    fn load_links_bulk_and_rollback() {
        let mut d = doc();
        let c = d.add_node("t.sink", pos(0.0, 0.0)).unwrap();
        let b = d.add_node("t.filter", pos(0.0, 0.0)).unwrap();
        let a = d.add_node("t.src", pos(0.0, 0.0)).unwrap();
        d.load_links(&[
            Link {
                from: (a, 0),
                to: (b, 0),
            },
            Link {
                from: (b, 0),
                to: (c, 0),
            },
        ])
        .unwrap();
        assert_eq!(d.all_links().len(), 2);
        let mut e = doc();
        let x = e.add_node("t.filter", pos(0.0, 0.0)).unwrap();
        let y = e.add_node("t.filter", pos(0.0, 0.0)).unwrap();
        let r = e.load_links(&[
            Link {
                from: (x, 0),
                to: (y, 0),
            },
            Link {
                from: (y, 0),
                to: (x, 0),
            },
        ]);
        assert_eq!(r, Err(ConnectError::Cycle));
        assert!(e.all_links().is_empty());
    }

    #[test]
    fn insert_snapshot_repairs_mismatched_values() {
        let mut d = doc();
        let a = d.add_node("t.filter", pos(0.0, 0.0)).unwrap();
        let mut s = d.snapshot_node(a).unwrap();
        d.remove_node(a);
        s.values = vec![Value::String("bad".into())]; // wrong type and too short
        d.insert_snapshot(&s).unwrap();
        assert_eq!(
            d.data(a).unwrap().values,
            vec![Value::Image(None), Value::Float(0.0)]
        );
        // duplicate uid refused
        assert!(d.insert_snapshot(&s).is_none());
    }
}
