//! The graph model. All mutations keep the spatial indices in sync, and every
//! operation is O(degree) (plus a DFS for cycle checks), never O(graph).
use crate::arena::{Arena, Key};
use crate::geom::{vec2, Rect, Vec2};
use crate::grid::Grid;
use crate::hash::FxMap;
use crate::layout;
use crate::wire::{self, WireCache};

pub type NodeId = Key;
pub type WireId = Key;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct OutPin {
    pub node: NodeId,
    pub index: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct InPin {
    pub node: NodeId,
    pub index: usize,
}

/// Either kind of pin, as returned by [`Graph::hit_pin`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PinRef {
    Out(OutPin),
    In(InPin),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConnectError {
    NoSuchNode,
    BadPin,
    SameNode,
    /// Connecting would create a cycle (geometry-node style DAG).
    Cycle,
}

pub struct Node<N> {
    pub data: N,
    pos: Vec2,
    width: f32,
    in_wires: Vec<Option<WireId>>,
    out_wires: Vec<Vec<WireId>>,
    /// Topological position: every wire goes from a lower to a higher `ord`.
    ord: u64,
}

impl<N> Node<N> {
    pub fn pos(&self) -> Vec2 {
        self.pos
    }
    pub fn width(&self) -> f32 {
        self.width
    }
    pub fn inputs(&self) -> usize {
        self.in_wires.len()
    }
    pub fn outputs(&self) -> usize {
        self.out_wires.len()
    }
    pub fn rect(&self) -> Rect {
        let h = layout::node_height(self.inputs(), self.outputs());
        Rect::new(self.pos, self.pos + vec2(self.width, h))
    }
    pub fn in_pos(&self, i: usize) -> Vec2 {
        vec2(self.pos.x, self.pos.y + layout::pin_y(i))
    }
    pub fn out_pos(&self, i: usize) -> Vec2 {
        vec2(self.pos.x + self.width, self.pos.y + layout::pin_y(i))
    }
    /// The (single) wire feeding input `i`.
    pub fn input_wire(&self, i: usize) -> Option<WireId> {
        self.in_wires.get(i).copied().flatten()
    }
    /// Wires leaving output `i`.
    pub fn output_wires(&self, i: usize) -> &[WireId] {
        self.out_wires.get(i).map_or(&[], |v| v.as_slice())
    }
}

pub struct Wire {
    pub from: OutPin,
    pub to: InPin,
    bbox: Rect,
}

impl Wire {
    pub fn bbox(&self) -> Rect {
        self.bbox
    }
}

/// Reusable output buffer for [`Graph::collect_visible`].
#[derive(Default)]
pub struct Visible {
    pub nodes: Vec<NodeId>,
    pub wires: Vec<WireId>,
}

const CELL: f32 = 256.0;

pub struct Graph<N> {
    nodes: Arena<Node<N>>,
    wires: Arena<Wire>,
    node_grid: Grid<NodeId>,
    wire_grid: Grid<WireId>,
    scratch: Vec<WireId>,
    next_ord: u64,
}

impl<N> Default for Graph<N> {
    fn default() -> Self {
        Self::new()
    }
}

fn wire_bbox(a: Vec2, b: Vec2) -> Rect {
    Rect::from_points(&wire::control_points(a, b)).expand(2.0)
}

impl<N> Graph<N> {
    pub fn new() -> Self {
        Graph {
            nodes: Arena::new(),
            wires: Arena::new(),
            node_grid: Grid::new(CELL),
            wire_grid: Grid::new(CELL),
            scratch: Vec::new(),
            next_ord: 0,
        }
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
    pub fn wire_count(&self) -> usize {
        self.wires.len()
    }
    pub fn node(&self, id: NodeId) -> Option<&Node<N>> {
        self.nodes.get(id)
    }
    pub fn node_data_mut(&mut self, id: NodeId) -> Option<&mut N> {
        self.nodes.get_mut(id).map(|n| &mut n.data)
    }
    pub fn wire(&self, id: WireId) -> Option<&Wire> {
        self.wires.get(id)
    }
    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node<N>)> {
        self.nodes.iter()
    }
    pub fn wires(&self) -> impl Iterator<Item = (WireId, &Wire)> {
        self.wires.iter()
    }

    pub fn add_node(&mut self, data: N, pos: Vec2, width: f32, inputs: usize, outputs: usize) -> NodeId {
        let node = Node {
            data,
            pos,
            width,
            in_wires: vec![None; inputs],
            out_wires: vec![Vec::new(); outputs],
            ord: self.next_ord,
        };
        self.next_ord += 1;
        let rect = node.rect();
        let id = self.nodes.insert(node);
        self.node_grid.update(id, &rect);
        id
    }

    /// Endpoints (output pin pos, input pin pos) of a wire.
    pub fn wire_endpoints(&self, id: WireId) -> Option<(Vec2, Vec2)> {
        let w = self.wires.get(id)?;
        self.endpoints(w.from, w.to)
    }

    fn endpoints(&self, from: OutPin, to: InPin) -> Option<(Vec2, Vec2)> {
        Some((self.nodes.get(from.node)?.out_pos(from.index), self.nodes.get(to.node)?.in_pos(to.index)))
    }

    /// Maintain the topological order for a new edge `x -> y` (Pearce-Kelly).
    /// Returns `false` (leaving the order untouched) if the edge would close a
    /// cycle. Edges that already agree with the order cost O(1); back-edges
    /// only search the nodes whose order lies between `y` and `x`.
    fn order_edge(&mut self, x: NodeId, y: NodeId) -> bool {
        let ord = |g: &Self, n: NodeId| g.nodes.get(n).map_or(0, |n| n.ord);
        let (ox, oy) = (ord(self, x), ord(self, y));
        if ox < oy {
            return true;
        }
        // Forward from y, restricted to ord < ox. Reaching x means a cycle.
        let mut fwd = vec![y];
        let mut seen: FxMap<NodeId, ()> = FxMap::default();
        seen.insert(y, ());
        let mut i = 0;
        while i < fwd.len() {
            let n = fwd[i];
            i += 1;
            for &w in self.nodes.get(n).into_iter().flat_map(|n| n.out_wires.iter().flatten()) {
                let t = self.wires.get(w).expect("wire exists").to.node;
                if t == x {
                    return false;
                }
                if ord(self, t) < ox && seen.insert(t, ()).is_none() {
                    fwd.push(t);
                }
            }
        }
        // Backward from x, restricted to ord > oy.
        let mut bwd = vec![x];
        let mut seen: FxMap<NodeId, ()> = FxMap::default();
        seen.insert(x, ());
        let mut i = 0;
        while i < bwd.len() {
            let n = bwd[i];
            i += 1;
            for &w in self.nodes.get(n).into_iter().flat_map(|n| n.in_wires.iter().flatten()) {
                let f = self.wires.get(w).expect("wire exists").from.node;
                if ord(self, f) > oy && seen.insert(f, ()).is_none() {
                    bwd.push(f);
                }
            }
        }
        // Reassign the pooled order values: ancestors of x first, then
        // descendants of y, each keeping its relative order.
        bwd.sort_unstable_by_key(|n| ord(self, *n));
        fwd.sort_unstable_by_key(|n| ord(self, *n));
        let mut pool: Vec<u64> = bwd.iter().chain(fwd.iter()).map(|n| ord(self, *n)).collect();
        pool.sort_unstable();
        for (n, o) in bwd.iter().chain(fwd.iter()).zip(pool) {
            self.nodes.get_mut(*n).expect("exists").ord = o;
        }
        true
    }

    fn check_pins(&self, from: OutPin, to: InPin) -> Result<(), ConnectError> {
        if from.node == to.node {
            return Err(ConnectError::SameNode);
        }
        let f = self.nodes.get(from.node).ok_or(ConnectError::NoSuchNode)?;
        let t = self.nodes.get(to.node).ok_or(ConnectError::NoSuchNode)?;
        if from.index >= f.outputs() || to.index >= t.inputs() {
            return Err(ConnectError::BadPin);
        }
        Ok(())
    }

    /// Insert the wire (replacing any wire already on the input) and index it.
    /// Pins must already be validated.
    fn insert_wire(&mut self, from: OutPin, to: InPin) -> WireId {
        let a = self.nodes.get(from.node).expect("validated").out_pos(from.index);
        let b = self.nodes.get(to.node).expect("validated").in_pos(to.index);
        if let Some(old) = self.nodes.get(to.node).expect("validated").in_wires[to.index] {
            self.disconnect(old);
        }
        let bbox = wire_bbox(a, b);
        let id = self.wires.insert(Wire { from, to, bbox });
        self.nodes.get_mut(from.node).expect("validated").out_wires[from.index].push(id);
        self.nodes.get_mut(to.node).expect("validated").in_wires[to.index] = Some(id);
        self.wire_grid.update(id, &bbox);
        id
    }

    /// Connect an output to an input. An input accepts a single wire, so an
    /// existing one is replaced. Rejects self-loops, bad pins and cycles.
    pub fn connect(&mut self, from: OutPin, to: InPin) -> Result<WireId, ConnectError> {
        self.check_pins(from, to)?;
        if !self.order_edge(from.node, to.node) {
            return Err(ConnectError::Cycle);
        }
        Ok(self.insert_wire(from, to))
    }

    /// Recompute the topological order from scratch (Kahn, O(V+E)).
    /// Returns `false` if the graph contains a cycle.
    fn rebuild_order(&mut self) -> bool {
        let mut indeg: FxMap<NodeId, usize> = FxMap::default();
        let mut queue: Vec<NodeId> = Vec::new();
        for (id, n) in self.nodes.iter() {
            let d = n.in_wires.iter().flatten().count();
            if d == 0 {
                queue.push(id);
            } else {
                indeg.insert(id, d);
            }
        }
        let mut next = 0u64;
        let mut i = 0;
        while i < queue.len() {
            let n = queue[i];
            i += 1;
            self.nodes.get_mut(n).expect("exists").ord = next;
            next += 1;
            let targets: Vec<NodeId> = self.nodes.get(n).expect("exists").out_wires.iter().flatten()
                .map(|w| self.wires.get(*w).expect("wire exists").to.node).collect();
            for t in targets {
                let d = indeg.get_mut(&t).expect("has input");
                *d -= 1;
                if *d == 0 {
                    queue.push(t);
                }
            }
        }
        if queue.len() != self.nodes.len() {
            return false;
        }
        self.next_ord = next;
        true
    }

    /// Bulk-load wires in O(V+E): validates pins, inserts everything, then
    /// computes the topological order once. Use this (not repeated
    /// [`Graph::connect`]) when loading a file or pasting a large subgraph
    /// whose nodes are not stored in dependency order.
    ///
    /// On error every wire added by this call is removed again. Inputs that
    /// were replaced by it are *not* restored, so call it on a fresh graph or
    /// on inputs known to be free.
    pub fn load_edges(&mut self, edges: &[(OutPin, InPin)]) -> Result<Vec<WireId>, ConnectError> {
        for &(f, t) in edges {
            self.check_pins(f, t)?;
        }
        let ids: Vec<WireId> = edges.iter().map(|&(f, t)| self.insert_wire(f, t)).collect();
        if !self.rebuild_order() {
            for &w in &ids {
                self.disconnect(w);
            }
            self.rebuild_order();
            return Err(ConnectError::Cycle);
        }
        Ok(ids)
    }

    pub fn disconnect(&mut self, id: WireId) -> bool {
        let Some(w) = self.wires.remove(id) else { return false };
        if let Some(n) = self.nodes.get_mut(w.from.node) {
            if let Some(v) = n.out_wires.get_mut(w.from.index) {
                v.retain(|x| *x != id);
            }
        }
        if let Some(n) = self.nodes.get_mut(w.to.node) {
            if let Some(slot) = n.in_wires.get_mut(w.to.index) {
                if *slot == Some(id) {
                    *slot = None;
                }
            }
        }
        self.wire_grid.remove(id);
        true
    }

    fn attached_wires(&mut self, id: NodeId) -> Option<Vec<WireId>> {
        let node = self.nodes.get(id)?;
        let mut ids = std::mem::take(&mut self.scratch);
        ids.clear();
        ids.extend(node.in_wires.iter().flatten().copied());
        ids.extend(node.out_wires.iter().flatten().copied());
        Some(ids)
    }

    pub fn remove_node(&mut self, id: NodeId) -> Option<N> {
        let ids = self.attached_wires(id)?;
        for &w in &ids {
            self.disconnect(w);
        }
        self.scratch = ids;
        self.node_grid.remove(id);
        self.nodes.remove(id).map(|n| n.data)
    }

    /// Move a node, updating its wires' bounds and both indices. O(degree).
    pub fn move_node(&mut self, id: NodeId, pos: Vec2) -> bool {
        let Some(node) = self.nodes.get_mut(id) else { return false };
        node.pos = pos;
        let rect = node.rect();
        self.node_grid.update(id, &rect);
        let ids = self.attached_wires(id).expect("exists");
        for &wid in &ids {
            let (from, to) = {
                let w = self.wires.get(wid).expect("attached wire exists");
                (w.from, w.to)
            };
            if let Some((a, b)) = self.endpoints(from, to) {
                let bbox = wire_bbox(a, b);
                self.wires.get_mut(wid).expect("exists").bbox = bbox;
                self.wire_grid.update(wid, &bbox);
            }
        }
        self.scratch = ids;
        true
    }

    /// Gather everything intersecting `rect` (graph space) into `out`.
    /// Cost is proportional to the visible set. `out` is reused across frames.
    pub fn collect_visible(&self, rect: &Rect, out: &mut Visible) {
        out.nodes.clear();
        out.wires.clear();
        self.node_grid.query(rect, &mut out.nodes);
        out.nodes.retain(|id| self.nodes.get(*id).map_or(false, |n| n.rect().intersects(rect)));
        self.wire_grid.query(rect, &mut out.wires);
        out.wires.retain(|id| self.wires.get(*id).map_or(false, |w| w.bbox.intersects(rect)));
    }

    /// Node under `p` (graph space), if any.
    pub fn hit_node(&self, p: Vec2) -> Option<NodeId> {
        let mut c = Vec::new();
        self.node_grid.query(&Rect::new(p, p), &mut c);
        c.into_iter().filter(|id| self.nodes.get(*id).map_or(false, |n| n.rect().contains(p))).max()
    }

    /// Closest pin within `radius` of `p` (graph space).
    pub fn hit_pin(&self, p: Vec2, radius: f32) -> Option<PinRef> {
        let mut c = Vec::new();
        self.node_grid.query(&Rect::new(p, p).expand(radius), &mut c);
        let mut best: Option<(f32, PinRef)> = None;
        let mut consider = |d: f32, pin: PinRef| {
            if d <= radius && best.map_or(true, |(bd, _)| d < bd) {
                best = Some((d, pin));
            }
        };
        for id in c {
            let Some(n) = self.nodes.get(id) else { continue };
            for i in 0..n.inputs() {
                consider((n.in_pos(i) - p).length(), PinRef::In(InPin { node: id, index: i }));
            }
            for i in 0..n.outputs() {
                consider((n.out_pos(i) - p).length(), PinRef::Out(OutPin { node: id, index: i }));
            }
        }
        best.map(|(_, pin)| pin)
    }

    /// Closest wire within `radius` of `p` (graph space).
    pub fn hit_wire(&self, p: Vec2, radius: f32, tol: f32, cache: &mut WireCache) -> Option<WireId> {
        let probe = Rect::new(p, p).expand(radius);
        let mut c = Vec::new();
        self.wire_grid.query(&probe, &mut c);
        let mut best: Option<(f32, WireId)> = None;
        for id in c {
            let Some(w) = self.wires.get(id) else { continue };
            if !w.bbox.intersects(&probe) {
                continue;
            }
            let Some((a, b)) = self.endpoints(w.from, w.to) else { continue };
            let d = wire::dist_to_polyline(p, cache.get(id, a, b, tol));
            if d <= radius && best.map_or(true, |(bd, _)| d < bd) {
                best = Some((d, id));
            }
        }
        best.map(|(_, id)| id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(n: NodeId, i: usize) -> OutPin {
        OutPin { node: n, index: i }
    }
    fn inp(n: NodeId, i: usize) -> InPin {
        InPin { node: n, index: i }
    }

    fn three() -> (Graph<&'static str>, NodeId, NodeId, NodeId) {
        let mut g = Graph::new();
        let a = g.add_node("a", vec2(0.0, 0.0), 180.0, 1, 2);
        let b = g.add_node("b", vec2(400.0, 0.0), 180.0, 2, 1);
        let c = g.add_node("c", vec2(800.0, 0.0), 180.0, 1, 1);
        (g, a, b, c)
    }

    #[test]
    fn connect_replaces_existing_input() {
        let (mut g, a, b, _) = three();
        let w1 = g.connect(out(a, 0), inp(b, 0)).unwrap();
        let w2 = g.connect(out(a, 1), inp(b, 0)).unwrap();
        assert_eq!(g.wire_count(), 1);
        assert!(g.wire(w1).is_none());
        assert_eq!(g.node(b).unwrap().input_wire(0), Some(w2));
        assert!(g.node(a).unwrap().output_wires(0).is_empty());
        assert_eq!(g.node(a).unwrap().output_wires(1), &[w2]);
    }

    #[test]
    fn rejects_cycles_self_loops_and_bad_pins() {
        let (mut g, a, b, c) = three();
        g.connect(out(a, 0), inp(b, 0)).unwrap();
        g.connect(out(b, 0), inp(c, 0)).unwrap();
        assert_eq!(g.connect(out(c, 0), inp(a, 0)), Err(ConnectError::Cycle));
        assert_eq!(g.connect(out(a, 0), inp(a, 0)), Err(ConnectError::SameNode));
        assert_eq!(g.connect(out(a, 9), inp(b, 1)), Err(ConnectError::BadPin));
        assert_eq!(g.wire_count(), 2);
    }

    #[test]
    fn remove_node_cleans_wires_and_indices() {
        let (mut g, a, b, c) = three();
        g.connect(out(a, 0), inp(b, 0)).unwrap();
        g.connect(out(b, 0), inp(c, 0)).unwrap();
        assert_eq!(g.remove_node(b), Some("b"));
        assert_eq!(g.wire_count(), 0);
        assert!(g.node(c).unwrap().input_wire(0).is_none());
        assert!(g.node(a).unwrap().output_wires(0).is_empty());
        let mut v = Visible::default();
        g.collect_visible(&Rect::new(vec2(-1e4, -1e4), vec2(1e4, 1e4)), &mut v);
        assert_eq!(v.nodes.len(), 2);
        assert!(v.wires.is_empty());
    }

    #[test]
    fn culling_includes_wires_passing_through_view() {
        let mut g = Graph::new();
        let a = g.add_node((), vec2(0.0, 0.0), 180.0, 1, 1);
        let b = g.add_node((), vec2(5000.0, 0.0), 180.0, 1, 1);
        g.connect(out(a, 0), inp(b, 0)).unwrap();
        let mut v = Visible::default();
        g.collect_visible(&Rect::new(vec2(2000.0, -100.0), vec2(2500.0, 200.0)), &mut v);
        assert!(v.nodes.is_empty());
        assert_eq!(v.wires.len(), 1);
        g.collect_visible(&Rect::new(vec2(2000.0, 2000.0), vec2(2500.0, 2500.0)), &mut v);
        assert!(v.wires.is_empty());
    }

    #[test]
    fn move_node_updates_wire_bounds_and_hit_tests() {
        let (mut g, a, b, _) = three();
        let w = g.connect(out(a, 0), inp(b, 0)).unwrap();
        let mut cache = WireCache::new();
        let (pa, pb) = g.wire_endpoints(w).unwrap();
        let mid = wire::eval(&wire::control_points(pa, pb), 0.5);
        assert_eq!(g.hit_wire(mid, 3.0, 0.3, &mut cache), Some(w));
        assert_eq!(g.hit_wire(mid + vec2(0.0, 80.0), 3.0, 0.3, &mut cache), None);

        assert!(g.move_node(b, vec2(400.0, 3000.0)));
        let mut v = Visible::default();
        g.collect_visible(&Rect::new(vec2(300.0, 2800.0), vec2(500.0, 3200.0)), &mut v);
        assert_eq!(v.wires, vec![w]); // wire follows the moved node
        assert_eq!(g.hit_node(vec2(450.0, 3010.0)), Some(b));
        assert_eq!(g.hit_node(vec2(450.0, 10.0)), None);
    }

    #[test]
    fn reordering_handles_reverse_creation_order() {
        let mut g = Graph::new();
        let c = g.add_node((), vec2(0.0, 0.0), 180.0, 1, 1);
        let b = g.add_node((), vec2(0.0, 0.0), 180.0, 1, 1);
        let a = g.add_node((), vec2(0.0, 0.0), 180.0, 1, 1);
        g.connect(out(a, 0), inp(b, 0)).unwrap(); // back-edge vs creation order
        g.connect(out(b, 0), inp(c, 0)).unwrap();
        assert_eq!(g.connect(out(c, 0), inp(a, 0)), Err(ConnectError::Cycle));
    }

    /// Random edges, many rejected: the accepted graph must be acyclic and
    /// `ord` must be a valid topological order (checked independently).
    #[test]
    fn random_stress_stays_acyclic() {
        let mut state = 12345u64;
        let mut rnd = |n: usize| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) as usize) % n
        };
        let mut g = Graph::new();
        let ids: Vec<_> = (0..200).map(|_| g.add_node((), vec2(0.0, 0.0), 180.0, 3, 3)).collect();
        let (mut ok, mut cyc) = (0, 0);
        for _ in 0..4000 {
            let (a, b) = (ids[rnd(200)], ids[rnd(200)]);
            match g.connect(out(a, rnd(3)), inp(b, rnd(3))) {
                Ok(_) => ok += 1,
                Err(ConnectError::Cycle) => cyc += 1,
                Err(_) => {}
            }
        }
        assert!(ok > 100 && cyc > 100, "ok {ok} cyc {cyc}");
        for (_, w) in g.wires() {
            let (fo, to) = (g.node(w.from.node).unwrap().ord, g.node(w.to.node).unwrap().ord);
            assert!(fo < to, "order violated");
        }
        // Kahn's algorithm: all nodes must be consumed.
        let mut indeg: std::collections::HashMap<NodeId, usize> = ids.iter().map(|&n| (n, 0)).collect();
        for (_, w) in g.wires() {
            *indeg.get_mut(&w.to.node).unwrap() += 1;
        }
        let mut q: Vec<_> = indeg.iter().filter(|(_, d)| **d == 0).map(|(n, _)| *n).collect();
        let mut seen = 0;
        while let Some(n) = q.pop() {
            seen += 1;
            for &wid in g.node(n).unwrap().out_wires.iter().flatten() {
                let t = g.wire(wid).unwrap().to.node;
                let d = indeg.get_mut(&t).unwrap();
                *d -= 1;
                if *d == 0 {
                    q.push(t);
                }
            }
        }
        assert_eq!(seen, ids.len(), "graph has a cycle");
    }

    #[test]
    fn load_edges_orders_once_and_rolls_back_on_cycle() {
        let mut g = Graph::new();
        let c = g.add_node((), vec2(0.0, 0.0), 180.0, 1, 1);
        let b = g.add_node((), vec2(0.0, 0.0), 180.0, 1, 1);
        let a = g.add_node((), vec2(0.0, 0.0), 180.0, 1, 1);
        let ids = g.load_edges(&[(out(a, 0), inp(b, 0)), (out(b, 0), inp(c, 0))]).unwrap();
        assert_eq!(ids.len(), 2);
        // order is valid for later interactive edits
        assert_eq!(g.connect(out(c, 0), inp(a, 0)), Err(ConnectError::Cycle));

        let mut h = Graph::new();
        let x = h.add_node((), vec2(0.0, 0.0), 180.0, 1, 1);
        let y = h.add_node((), vec2(0.0, 0.0), 180.0, 1, 1);
        let r = h.load_edges(&[(out(x, 0), inp(y, 0)), (out(y, 0), inp(x, 0))]);
        assert_eq!(r, Err(ConnectError::Cycle));
        assert_eq!(h.wire_count(), 0);
        assert!(h.connect(out(x, 0), inp(y, 0)).is_ok());
    }

    #[test]
    fn hit_pin_finds_nearest_pin_on_node_edge() {
        let mut g = Graph::new();
        let a = g.add_node((), vec2(100.0, 100.0), 180.0, 2, 1);
        // input 1 sits on the left edge, output 0 on the right edge
        let i1 = g.node(a).unwrap().in_pos(1);
        let o0 = g.node(a).unwrap().out_pos(0);
        assert_eq!(g.hit_pin(i1 + vec2(-3.0, 2.0), 8.0), Some(PinRef::In(inp(a, 1))));
        assert_eq!(g.hit_pin(o0 + vec2(4.0, 0.0), 8.0), Some(PinRef::Out(out(a, 0))));
        assert_eq!(g.hit_pin(vec2(190.0, 190.0), 8.0), None);
    }
}
