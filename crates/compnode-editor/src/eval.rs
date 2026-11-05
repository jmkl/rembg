//! Evaluation: walks the graph in topological order, recomputes only what
//! changed, and keeps the results of every node in a cache.
//!
//! The data flow is the one the old `rembg/compnode` crate used, with the
//! image type generalised:
//!
//! * a node **pulls** its inputs from the nodes wired into it,
//! * every input socket may carry several values (multi-input sockets),
//! * a node's result is stored on the node and read by its consumers,
//! * a change marks the node and everything downstream of it dirty, so an
//!   edit recomputes only the branch it touches.
use crate::node::{CompnodeNode, NodeError, Payload, ProcessCtx};
use compnode_core::hash::FxMap;
use compnode_ui::document::{Change, Document};

/// Per-node results and the dirty set.
pub struct Evaluator {
    /// The outputs of every evaluated node, by uid.
    cache: FxMap<u64, Vec<Payload>>,
    /// Nodes whose result must be recomputed.
    dirty: FxMap<u64, ()>,
    /// The message of the last failure of a node.
    errors: FxMap<u64, String>,
    canvas: (u32, u32),
    /// Nodes recomputed by the most recent evaluation.
    pub last_evaluated: Vec<u64>,
}

/// The live node instances, by uid.
pub type Nodes = FxMap<u64, Box<dyn CompnodeNode>>;

impl Evaluator {
    pub fn new(canvas: (u32, u32)) -> Evaluator {
        Evaluator {
            cache: FxMap::default(),
            dirty: FxMap::default(),
            errors: FxMap::default(),
            canvas: (canvas.0.max(1), canvas.1.max(1)),
            last_evaluated: Vec::new(),
        }
    }

    /// The size of the canvas the graph works on.
    pub fn canvas(&self) -> (u32, u32) {
        self.canvas
    }

    /// Change the canvas size; every result is dropped.
    pub fn set_canvas(&mut self, canvas: (u32, u32)) {
        self.canvas = (canvas.0.max(1), canvas.1.max(1));
        self.clear();
    }

    /// Forget every result.
    pub fn clear(&mut self) {
        self.cache.clear();
        self.dirty.clear();
        self.errors.clear();
    }

    /// The outputs of a node, if it has been evaluated.
    pub fn outputs(&self, uid: u64) -> Option<&[Payload]> {
        self.cache.get(&uid).map(Vec::as_slice)
    }

    /// The error message of the last evaluation of a node.
    pub fn error(&self, uid: u64) -> Option<&str> {
        self.errors.get(&uid).map(String::as_str)
    }

    /// Forget a node's result (it was deleted).
    pub fn forget(&mut self, uid: u64) {
        self.cache.remove(&uid);
        self.dirty.remove(&uid);
        self.errors.remove(&uid);
    }

    /// Mark a node and everything downstream of it for recomputation.
    pub fn mark_downstream(&mut self, doc: &Document, uid: u64) {
        let mut seen: FxMap<u64, ()> = FxMap::default();
        let mut stack = vec![uid];
        while let Some(u) = stack.pop() {
            if seen.insert(u, ()).is_some() {
                continue;
            }
            self.dirty.insert(u, ());
            let Some(node) = doc.id_of(u).and_then(|id| doc.graph.node(id)) else {
                continue;
            };
            for o in 0..node.outputs() {
                for &w in node.output_wires(o) {
                    if let Some(l) = doc.link_of_wire(w) {
                        stack.push(l.to.0);
                    }
                }
            }
        }
    }

    /// Feed the document's change log in; marks what must be recomputed.
    pub fn apply_changes(&mut self, doc: &Document, changes: &[Change]) {
        for c in changes {
            match c {
                Change::Value { uid, .. } | Change::Flag { uid } => self.mark_downstream(doc, *uid),
                Change::Structure { uids } => {
                    for &u in uids {
                        if doc.contains(u) {
                            self.mark_downstream(doc, u);
                        } else {
                            self.forget(u);
                        }
                    }
                }
            }
        }
    }

    /// Recompute every dirty node, in topological order.
    pub fn evaluate_all(&mut self, doc: &Document, nodes: &mut Nodes) {
        self.last_evaluated.clear();
        for id in doc.graph.topo_order() {
            let Some(u) = doc.uid_of(id) else { continue };
            if self.dirty.contains_key(&u) || !self.cache.contains_key(&u) {
                self.eval_node(doc, nodes, u);
            }
        }
    }

    /// The uid of every node that feeds `uid`, directly or indirectly.
    fn ancestors(doc: &Document, uid: u64) -> Vec<u64> {
        let mut seen: FxMap<u64, ()> = FxMap::default();
        let mut stack = vec![uid];
        let mut out = Vec::new();
        while let Some(u) = stack.pop() {
            if seen.insert(u, ()).is_some() {
                continue;
            }
            out.push(u);
            let Some(node) = doc.id_of(u).and_then(|id| doc.graph.node(id)) else {
                continue;
            };
            for i in 0..node.inputs() {
                for &w in node.input_wires(i) {
                    if let Some(l) = doc.link_of_wire(w) {
                        stack.push(l.from.0);
                    }
                }
            }
        }
        out
    }

    /// Evaluate one node and cache its outputs.
    fn eval_node(&mut self, doc: &Document, nodes: &mut Nodes, uid: u64) {
        self.dirty.remove(&uid);
        self.last_evaluated.push(uid);
        let Some(node) = doc.id_of(uid).and_then(|id| doc.graph.node(id)) else {
            return;
        };
        let Some(schema) = doc.registry.get(&node.data.schema) else {
            return;
        };

        // Gather the values arriving on every input.
        let mut inputs: Vec<Vec<Payload>> = Vec::with_capacity(node.inputs());
        for i in 0..node.inputs() {
            let mut values: Vec<Payload> = Vec::new();
            for &w in node.input_wires(i) {
                let Some(link) = doc.link_of_wire(w) else {
                    continue;
                };
                if let Some(msg) = self.errors.get(&link.from.0) {
                    let name = schema.inputs.get(i).map_or("?", |d| d.name.as_str());
                    self.cache.remove(&uid);
                    self.errors
                        .insert(uid, format!("input {name} failed upstream: {msg}"));
                    return;
                }
                if let Some(p) = self
                    .cache
                    .get(&link.from.0)
                    .and_then(|v| v.get(link.from.1))
                {
                    values.push(p.clone());
                }
            }
            inputs.push(values);
        }

        // A muted node passes its first input through.
        if node.data.muted {
            let out = inputs
                .iter()
                .flatten()
                .next()
                .cloned()
                .map(|p| vec![p])
                .unwrap_or_default();
            self.errors.remove(&uid);
            self.cache.insert(uid, out);
            return;
        }

        let Some(impl_) = nodes.get_mut(&uid) else {
            self.cache.remove(&uid);
            self.errors
                .insert(uid, format!("no implementation for {}", node.data.schema));
            return;
        };

        let mut ctx = ProcessCtx {
            inputs: &inputs,
            values: &node.data.values,
            canvas: self.canvas,
            node: uid,
        };
        match impl_.process(&mut ctx) {
            Ok(out) => {
                self.errors.remove(&uid);
                self.cache.insert(uid, out);
            }
            Err(e) => {
                self.cache.remove(&uid);
                self.errors.insert(uid, e.to_string());
            }
        }
    }

    /// Every node that has no consumer, by uid. These are the results a host
    /// usually wants to show.
    pub fn sinks(&self, doc: &Document) -> Vec<u64> {
        let mut v: Vec<u64> = doc
            .uids()
            .into_iter()
            .filter(|&u| {
                doc.id_of(u)
                    .and_then(|id| doc.graph.node(id))
                    .is_some_and(|n| n.outputs() == 0 || n.output_wires(0).is_empty())
            })
            .collect();
        v.sort_unstable();
        v
    }

    /// The ancestors of a node, in no particular order.
    pub fn dependencies(&self, doc: &Document, uid: u64) -> Vec<u64> {
        let mut v = Self::ancestors(doc, uid);
        v.sort_unstable();
        v
    }
}

/// Convert a node error into the message shown on the node.
pub fn error_message(e: &NodeError) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::CompnodeNode;
    use compnode_core::geom::vec2;
    use compnode_ui::schema::{Category, SocketDef};
    use compnode_ui::value::Value;

    /// Doubles the number on its input.
    #[derive(Default)]
    struct Double;

    impl CompnodeNode for Double {
        fn id(&self) -> &'static str {
            "test.double"
        }
        fn title(&self) -> &str {
            "Double"
        }
        fn category(&self) -> Category {
            Category::Utility
        }
        fn inputs(&self) -> Vec<SocketDef> {
            vec![SocketDef::float("Value", 1.0)]
        }
        fn outputs(&self) -> Vec<SocketDef> {
            vec![SocketDef::new(
                "Value",
                compnode_ui::value::SocketType::Float,
            )]
        }
        fn process(&mut self, ctx: &mut ProcessCtx) -> Result<Vec<Payload>, NodeError> {
            Ok(vec![Payload::value(Value::Float(ctx.float(0) * 2.0))])
        }
    }

    /// Fails on purpose.
    #[derive(Default)]
    struct Boom;

    impl CompnodeNode for Boom {
        fn id(&self) -> &'static str {
            "test.boom"
        }
        fn title(&self) -> &str {
            "Boom"
        }
        fn inputs(&self) -> Vec<SocketDef> {
            vec![SocketDef::float("Value", 1.0)]
        }
        fn outputs(&self) -> Vec<SocketDef> {
            vec![SocketDef::new(
                "Value",
                compnode_ui::value::SocketType::Float,
            )]
        }
        fn process(&mut self, _ctx: &mut ProcessCtx) -> Result<Vec<Payload>, NodeError> {
            Err(NodeError::Failed("boom".into()))
        }
    }

    fn registry() -> compnode_ui::schema::Registry {
        let mut r = compnode_ui::schema::Registry::new();
        r.register(crate::node::schema_of(&Double));
        r.register(crate::node::schema_of(&Boom));
        r
    }

    fn setup() -> (Document, Nodes) {
        let mut doc = Document::new(registry());
        let mut nodes: Nodes = FxMap::default();
        let a = doc.add_node("test.double", vec2(0.0, 0.0)).unwrap();
        let b = doc.add_node("test.double", vec2(200.0, 0.0)).unwrap();
        nodes.insert(a, Box::<Double>::default());
        nodes.insert(b, Box::<Double>::default());
        doc.connect(compnode_ui::document::Link {
            from: (a, 0),
            to: (b, 0),
        })
        .unwrap();
        (doc, nodes)
    }

    #[test]
    fn evaluates_a_chain_and_caches_it() {
        let (doc, mut nodes) = setup();
        let mut ev = Evaluator::new((8, 8));
        let a = doc.uids()[0];
        let b = doc.uids()[1];
        ev.evaluate_all(&doc, &mut nodes);
        assert_eq!(ev.last_evaluated.len(), 2);
        // 1.0 doubles to 2.0, and doubles again to 4.0
        assert_eq!(
            ev.outputs(a).unwrap()[0].cloned::<Value>(),
            Some(Value::Float(2.0))
        );
        assert_eq!(
            ev.outputs(b).unwrap()[0].cloned::<Value>(),
            Some(Value::Float(4.0))
        );

        // nothing changed: nothing is recomputed
        ev.evaluate_all(&doc, &mut nodes);
        assert!(ev.last_evaluated.is_empty());
    }

    #[test]
    fn an_edit_recomputes_only_the_branch_it_touches() {
        let (mut doc, mut nodes) = setup();
        let mut ev = Evaluator::new((8, 8));
        let a = doc.uids()[0];
        let b = doc.uids()[1];
        ev.evaluate_all(&doc, &mut nodes);
        assert_eq!(ev.last_evaluated.len(), 2);

        doc.set_value(a, 0, Value::Float(5.0));
        let changes = doc.take_changes();
        ev.apply_changes(&doc, &changes);
        ev.evaluate_all(&doc, &mut nodes);
        let mut got = ev.last_evaluated.clone();
        got.sort_unstable();
        assert_eq!(got, vec![a, b]);
        assert_eq!(
            ev.outputs(b).unwrap()[0].cloned::<Value>(),
            Some(Value::Float(20.0))
        );
    }

    #[test]
    fn a_failing_node_reports_its_error_downstream() {
        let mut doc = Document::new(registry());
        let mut nodes: Nodes = FxMap::default();
        let bad = doc.add_node("test.boom", vec2(0.0, 0.0)).unwrap();
        let good = doc.add_node("test.double", vec2(200.0, 0.0)).unwrap();
        nodes.insert(bad, Box::<Boom>::default());
        nodes.insert(good, Box::<Double>::default());
        doc.connect(compnode_ui::document::Link {
            from: (bad, 0),
            to: (good, 0),
        })
        .unwrap();
        let mut ev = Evaluator::new((8, 8));
        ev.evaluate_all(&doc, &mut nodes);
        assert_eq!(ev.error(bad), Some("boom"));
        assert!(ev.error(good).unwrap().contains("upstream"));
    }

    #[test]
    fn deleting_a_node_purges_its_result() {
        let (mut doc, mut nodes) = setup();
        let mut ev = Evaluator::new((8, 8));
        ev.evaluate_all(&doc, &mut nodes);
        let a = doc.uids()[0];
        doc.remove_node(a);
        let changes = doc.take_changes();
        ev.apply_changes(&doc, &changes);
        assert!(ev.outputs(a).is_none());
    }
}
