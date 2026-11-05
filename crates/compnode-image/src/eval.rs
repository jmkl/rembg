//! Evaluation: walks the graph in topological order, recomputes only what
//! changed, keeps results in a cache with an optional memory budget, and
//! reports per-node errors.
use crate::buffer::ImageBuf;
use crate::loader::{ImageLoader, PpmLoader};
use crate::nodes::NodeOp;
use compnode_core::Color;
use compnode_core::hash::FxMap;
use compnode_ui::document::{Change, Document, Layer};
use compnode_ui::schema::Registry;
use compnode_ui::value::{SocketType, Value};
use std::sync::Arc;

/// A value flowing along a wire.
#[derive(Clone, Debug)]
pub enum Data {
    Image(Arc<ImageBuf>),
    Val(Value),
}

impl Data {
    pub fn bytes(&self) -> usize {
        match self {
            Data::Image(i) => i.bytes(),
            Data::Val(_) => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EvalError {
    NoSuchNode,
    /// The node produced no image on its first output.
    NoImage,
    Failed {
        uid: u64,
        message: String,
    },
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::NoSuchNode => write!(f, "no such node"),
            EvalError::NoImage => write!(f, "node has no image output"),
            EvalError::Failed { uid, message } => write!(f, "node {uid}: {message}"),
        }
    }
}

impl std::error::Error for EvalError {}

/// What a node operation sees: connected values, its own parameters and the
/// canvas size.
pub struct EvalCtx<'a> {
    pub canvas: (u32, u32),
    inputs: Vec<Option<Data>>,
    /// Every value on each input (in connection order); used by multi-input
    /// sockets such as the Output node's layer list. `multi[i]` is ordered
    /// bottom-to-top.
    multi: Vec<Vec<Data>>,
    /// Layer transforms of the node being evaluated (may be empty).
    layers: &'a [Layer],
    values: &'a [Value],
    files: &'a mut FxMap<String, Arc<ImageBuf>>,
    loader: &'a dyn ImageLoader,
}

impl<'a> EvalCtx<'a> {
    /// Build a context (used by other evaluators that reuse the parameter
    /// accessors, such as the GPU one).
    #[cfg(feature = "gpu")]
    pub(crate) fn new(
        canvas: (u32, u32),
        inputs: Vec<Option<Data>>,
        values: &'a [Value],
        files: &'a mut FxMap<String, Arc<ImageBuf>>,
        loader: &'a dyn ImageLoader,
    ) -> EvalCtx<'a> {
        let multi = inputs.iter().map(|i| i.iter().cloned().collect()).collect();
        EvalCtx {
            canvas,
            inputs,
            multi,
            layers: &[],
            values,
            files,
            loader,
        }
    }

    /// Build a context that also carries the multi-input values and layer
    /// transforms (used by the CPU evaluator).
    pub(crate) fn with_multi(
        canvas: (u32, u32),
        inputs: Vec<Option<Data>>,
        multi: Vec<Vec<Data>>,
        layers: &'a [Layer],
        values: &'a [Value],
        files: &'a mut FxMap<String, Arc<ImageBuf>>,
        loader: &'a dyn ImageLoader,
    ) -> EvalCtx<'a> {
        EvalCtx {
            canvas,
            inputs,
            multi,
            layers,
            values,
            files,
            loader,
        }
    }
}

impl EvalCtx<'_> {
    /// The connected image on input `i`, if any.
    pub fn image(&self, i: usize) -> Option<Arc<ImageBuf>> {
        match self.inputs.get(i)? {
            Some(Data::Image(img)) => Some(Arc::clone(img)),
            _ => None,
        }
    }

    /// Every image connected to input `i`, in connection order (bottom-to-top).
    pub fn images(&self, i: usize) -> Vec<Arc<ImageBuf>> {
        self.multi
            .get(i)
            .map(|v| {
                v.iter()
                    .filter_map(|d| {
                        if let Data::Image(img) = d {
                            Some(Arc::clone(img))
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Layer transforms of the node being evaluated.
    pub fn layers(&self) -> &[Layer] {
        self.layers
    }

    /// The transform stored for the layer fed by node uid `source` (identity
    /// when none is stored).
    pub fn layer_of(&self, source: u64) -> Layer {
        self.layers
            .iter()
            .copied()
            .find(|l| l.source == source)
            .unwrap_or(Layer::new(source))
    }

    /// Build a [`crate::cpu::LayerPaint`] for layer `i` (the i-th connected
    /// image on the multi-input socket), using the stored transform if present.
    pub fn layer_paint<'b>(&self, i: usize, image: &'b ImageBuf) -> crate::cpu::LayerPaint<'b> {
        let l = self.layers.get(i);
        let (center, size, angle) = match l {
            Some(l) => (
                (l.center != [0.0, 0.0]).then_some(l.center),
                (l.size[0] > 0.0 && l.size[1] > 0.0).then_some(l.size),
                l.angle,
            ),
            None => (None, None, 0.0),
        };
        crate::cpu::LayerPaint {
            image,
            center,
            size,
            angle,
        }
    }

    /// The connected image, or a transparent canvas-sized one.
    pub fn image_or_transparent(&self, i: usize) -> Arc<ImageBuf> {
        self.image(i)
            .unwrap_or_else(|| Arc::new(ImageBuf::new(self.canvas.0, self.canvas.1)))
    }

    /// Parameter value: from the wire when connected, else the node's own.
    pub fn val(&self, i: usize) -> Value {
        match self.inputs.get(i) {
            Some(Some(Data::Val(v))) => v.clone(),
            _ => self.values.get(i).cloned().unwrap_or(Value::Float(0.0)),
        }
    }

    pub fn float(&self, i: usize) -> f32 {
        self.val(i).as_f32().unwrap_or(0.0)
    }
    pub fn int(&self, i: usize) -> i32 {
        match self.val(i).convert(SocketType::Int) {
            Some(Value::Int(v)) => v,
            _ => 0,
        }
    }
    pub fn bool(&self, i: usize) -> bool {
        matches!(
            self.val(i).convert(SocketType::Bool),
            Some(Value::Bool(true))
        )
    }
    pub fn color(&self, i: usize) -> Color {
        match self.val(i).convert(SocketType::Color) {
            Some(Value::Color(c)) => c,
            _ => Color::BLACK,
        }
    }
    pub fn enum_index(&self, i: usize) -> usize {
        match self.val(i) {
            Value::Enum(e) => e,
            _ => 0,
        }
    }
    pub fn string(&self, i: usize) -> String {
        match self.val(i) {
            Value::String(s) => s,
            _ => String::new(),
        }
    }

    /// Load (and remember) an image file, downscaled to the canvas if larger.
    pub fn load_file(&mut self, path: &str) -> Result<Arc<ImageBuf>, String> {
        if let Some(img) = self.files.get(path) {
            return Ok(Arc::clone(img));
        }
        let img = self.loader.load(path)?;
        let img = crate::loader::downscale_to_fit(img, self.canvas.0, self.canvas.1);
        let img = Arc::new(img);
        self.files.insert(path.to_string(), Arc::clone(&img));
        Ok(img)
    }
}

pub struct Evaluator {
    ops: FxMap<String, Box<dyn NodeOp>>,
    cache: FxMap<u64, Vec<Data>>,
    dirty: FxMap<u64, ()>,
    errors: FxMap<u64, String>,
    files: FxMap<String, Arc<ImageBuf>>,
    loader: Arc<dyn ImageLoader>,
    canvas: (u32, u32),
    budget: usize,
    tick: u64,
    used: FxMap<u64, u64>,
    /// Nodes recomputed by the most recent [`Evaluator::evaluate`] call.
    pub last_evaluated: Vec<u64>,
}

impl Evaluator {
    /// Evaluator with the built-in image nodes and a PPM file loader.
    pub fn new(canvas: (u32, u32)) -> Evaluator {
        let mut e = Evaluator::empty(canvas);
        for op in crate::nodes::builtin_ops() {
            e.register(op);
        }
        e
    }

    /// Evaluator without any node implementations.
    pub fn empty(canvas: (u32, u32)) -> Evaluator {
        Evaluator {
            ops: FxMap::default(),
            cache: FxMap::default(),
            dirty: FxMap::default(),
            errors: FxMap::default(),
            files: FxMap::default(),
            loader: Arc::new(PpmLoader),
            canvas: (canvas.0.max(1), canvas.1.max(1)),
            budget: usize::MAX,
            tick: 0,
            used: FxMap::default(),
            last_evaluated: Vec::new(),
        }
    }

    pub fn with_loader(mut self, loader: Arc<dyn ImageLoader>) -> Evaluator {
        self.loader = loader;
        self
    }

    pub fn register(&mut self, op: Box<dyn NodeOp>) {
        self.ops.insert(op.schema().id, op);
    }

    /// Add the schemas of every registered operation to `reg`.
    pub fn register_schemas(&self, reg: &mut Registry) {
        let mut all: Vec<_> = self.ops.values().map(|o| o.schema()).collect();
        all.sort_by(|a, b| a.id.cmp(&b.id));
        for s in all {
            reg.register(s);
        }
    }

    pub fn canvas(&self) -> (u32, u32) {
        self.canvas
    }

    /// Change the canvas size; everything is recomputed.
    pub fn set_canvas(&mut self, canvas: (u32, u32)) {
        self.canvas = (canvas.0.max(1), canvas.1.max(1));
        self.cache.clear();
        self.files.clear();
    }

    /// Forget every result and loaded file (after opening another project).
    pub fn clear_cache(&mut self) {
        self.cache.clear();
        self.dirty.clear();
        self.errors.clear();
        self.files.clear();
        self.used.clear();
    }

    /// Limit the memory held by cached images (least recently used results
    /// that the current evaluation does not need are dropped first).
    pub fn set_cache_budget(&mut self, bytes: usize) {
        self.budget = bytes;
    }

    pub fn cached_bytes(&self) -> usize {
        self.cache.values().flatten().map(Data::bytes).sum()
    }

    pub fn is_cached(&self, uid: u64) -> bool {
        self.cache.contains_key(&uid)
    }

    /// Error message of the last evaluation of `uid`, if it failed.
    pub fn error(&self, uid: u64) -> Option<&str> {
        self.errors.get(&uid).map(String::as_str)
    }

    /// Forget a loaded file (after it changed on disk).
    pub fn invalidate_file(&mut self, doc: &Document, path: &str) {
        self.files.remove(path);
        let hit: Vec<u64> = doc
            .uids()
            .into_iter()
            .filter(|u| {
                doc.data(*u).is_some_and(|d| {
                    d.schema == "image.file"
                        && matches!(d.values.first(), Some(Value::String(s)) if s == path)
                })
            })
            .collect();
        for u in hit {
            self.mark_downstream(doc, u);
        }
    }

    /// Mark `uid` and everything downstream of it for recomputation.
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
                            self.cache.remove(&u);
                            self.errors.remove(&u);
                            self.dirty.remove(&u);
                            self.used.remove(&u);
                        }
                    }
                }
            }
        }
    }

    fn needs_compute(&self, uid: u64) -> bool {
        self.dirty.contains_key(&uid) || !self.cache.contains_key(&uid)
    }

    fn sources(doc: &Document, uid: u64) -> Vec<u64> {
        let Some(node) = doc.id_of(uid).and_then(|id| doc.graph.node(id)) else {
            return Vec::new();
        };
        // Use input_wires to support multi-input sockets (multiple wires per input).
        let mut sources = Vec::new();
        for i in 0..node.inputs() {
            for &w in node.input_wires(i) {
                if let Some(l) = doc.link_of_wire(w) {
                    sources.push(l.from.0);
                }
            }
        }
        sources
    }

    /// Evaluate `uid` (recomputing only dirty or missing ancestors) and
    /// return its first output image.
    pub fn evaluate(&mut self, doc: &Document, uid: u64) -> Result<Arc<ImageBuf>, EvalError> {
        self.last_evaluated.clear();
        if !doc.contains(uid) {
            return Err(EvalError::NoSuchNode);
        }
        // everything the target depends on
        let mut ancestors: FxMap<u64, ()> = FxMap::default();
        let mut stack = vec![uid];
        while let Some(u) = stack.pop() {
            if ancestors.insert(u, ()).is_some() {
                continue;
            }
            stack.extend(Self::sources(doc, u));
        }
        // the part of that which must be recomputed: stop descending at
        // nodes whose cached result is still valid
        let mut must: FxMap<u64, ()> = FxMap::default();
        let mut stack = vec![uid];
        while let Some(u) = stack.pop() {
            if must.contains_key(&u) || !self.needs_compute(u) {
                continue;
            }
            must.insert(u, ());
            stack.extend(Self::sources(doc, u));
        }
        for id in doc.graph.topo_order() {
            let Some(u) = doc.uid_of(id) else { continue };
            if must.contains_key(&u) {
                self.eval_node(doc, u);
            }
        }
        self.tick += 1;
        for &u in ancestors.keys() {
            self.used.insert(u, self.tick);
        }
        self.enforce_budget(&ancestors);
        if let Some(msg) = self.errors.get(&uid) {
            return Err(EvalError::Failed {
                uid,
                message: msg.clone(),
            });
        }
        match self.cache.get(&uid).and_then(|v| v.first()) {
            Some(Data::Image(img)) => Ok(Arc::clone(img)),
            _ => Err(EvalError::NoImage),
        }
    }

    fn eval_node(&mut self, doc: &Document, uid: u64) {
        self.dirty.remove(&uid);
        self.last_evaluated.push(uid);
        let Some(node) = doc.id_of(uid).and_then(|id| doc.graph.node(id)) else {
            return;
        };
        let Some(schema) = doc.registry.get(&node.data.schema) else {
            return;
        };
        let mut inputs: Vec<Option<Data>> = Vec::with_capacity(node.inputs());
        let mut multi: Vec<Vec<Data>> = Vec::with_capacity(node.inputs());
        for i in 0..node.inputs() {
            let links: Vec<compnode_ui::document::Link> = node
                .input_wires(i)
                .iter()
                .filter_map(|&w| doc.link_of_wire(w))
                .collect();
            if links.is_empty() {
                inputs.push(None);
                multi.push(Vec::new());
                continue;
            }
            let mut values: Vec<Data> = Vec::with_capacity(links.len());
            for link in &links {
                eprintln!(
                    "DBG eval link.from=({}, {}) cache_has={} cache_val={:?}",
                    link.from.0,
                    link.from.1,
                    self.cache.contains_key(&link.from.0),
                    self.cache
                        .get(&link.from.0)
                        .and_then(|v| v.get(link.from.1))
                );
                if self.errors.contains_key(&link.from.0) {
                    self.cache.remove(&uid);
                    self.errors.insert(
                        uid,
                        format!(
                            "input {} failed upstream",
                            schema.inputs.get(i).map_or("?", |d| d.name.as_str())
                        ),
                    );
                    return;
                }
                if let Some(d) = self
                    .cache
                    .get(&link.from.0)
                    .and_then(|v| v.get(link.from.1))
                    .cloned()
                {
                    values.push(d);
                }
            }
            inputs.push(values.first().cloned());
            multi.push(values);
        }
        if node.data.muted {
            let pass = schema.inputs.iter().enumerate().find_map(|(i, d)| {
                if d.ty == SocketType::Image {
                    inputs.get(i).cloned().flatten()
                } else {
                    None
                }
            });
            let first = schema.outputs.first();
            let out = match (pass, first) {
                (Some(d), _) => vec![d],
                (None, Some(o)) if o.ty == SocketType::Image => {
                    vec![Data::Image(Arc::new(ImageBuf::new(
                        self.canvas.0,
                        self.canvas.1,
                    )))]
                }
                (None, Some(o)) => vec![Data::Val(o.default.clone())],
                (None, None) => Vec::new(),
            };
            self.errors.remove(&uid);
            self.cache.insert(uid, out);
            return;
        }
        let Some(op) = self.ops.get(&node.data.schema) else {
            // A reroute has no implementation: it just forwards whatever
            // flows into it. Pair the inputs and outputs positionally, so a
            // rerouted image (or value) reaches the same downstream socket.
            if node.data.is_reroute() {
                // Forward everything on input 0 to output 0 (a reroute can
                // carry a multi-input's whole list downstream), and copy any
                // other sockets positionally.
                let mut out: Vec<Data> = Vec::with_capacity(schema.outputs.len());
                for i in 0..schema.outputs.len() {
                    match multi.get(i).filter(|v| !v.is_empty()) {
                        Some(vals) => out.extend(vals.iter().cloned()),
                        None => {
                            let def = &schema.outputs[i].default;
                            out.push(if def.socket_type() == SocketType::Image {
                                Data::Image(Arc::new(ImageBuf::new(self.canvas.0, self.canvas.1)))
                            } else {
                                Data::Val(def.clone())
                            });
                        }
                    }
                }
                self.errors.remove(&uid);
                self.cache.insert(uid, out);
                return;
            }
            self.cache.remove(&uid);
            self.errors
                .insert(uid, format!("no implementation for {}", node.data.schema));
            return;
        };
        let mut ctx = EvalCtx::with_multi(
            self.canvas,
            inputs,
            multi,
            &node.data.layers,
            &node.data.values,
            &mut self.files,
            &*self.loader,
        );
        let result = op.eval(&mut ctx);
        match result {
            Ok(out) => {
                self.errors.remove(&uid);
                self.cache.insert(uid, out);
            }
            Err(msg) => {
                self.cache.remove(&uid);
                self.errors.insert(uid, msg);
            }
        }
    }

    fn enforce_budget(&mut self, keep: &FxMap<u64, ()>) {
        let mut total = self.cached_bytes();
        if total <= self.budget {
            return;
        }
        let mut victims: Vec<(u64, u64)> = self
            .cache
            .keys()
            .filter(|u| !keep.contains_key(u))
            .map(|u| (self.used.get(u).copied().unwrap_or(0), *u))
            .collect();
        victims.sort_unstable();
        for (_, u) in victims {
            if total <= self.budget {
                break;
            }
            if let Some(v) = self.cache.remove(&u) {
                total -= v.iter().map(Data::bytes).sum::<usize>();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::FnLoader;
    use compnode_core::geom::vec2;
    use compnode_ui::document::Link;

    const N: u32 = 8;

    fn setup() -> (Document, Evaluator) {
        let ev = Evaluator::new((N, N));
        let mut reg = Registry::new();
        ev.register_schemas(&mut reg);
        (Document::new(reg), ev)
    }

    fn add(d: &mut Document, kind: &str) -> u64 {
        d.add_node(kind, vec2(0.0, 0.0))
            .unwrap_or_else(|| panic!("unknown node {kind}"))
    }
    fn link(d: &mut Document, a: u64, ai: usize, b: u64, bi: usize) {
        d.connect(Link {
            from: (a, ai),
            to: (b, bi),
        })
        .unwrap();
    }
    fn sync(d: &mut Document, e: &mut Evaluator) {
        d.sync_all_layers();
        let ch = d.take_changes();
        e.apply_changes(d, &ch);
    }
    fn center(img: &ImageBuf) -> [f32; 4] {
        img.get(img.width / 2, img.height / 2)
    }
    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// solid(0.2,0.4,0.6) -> invert -> blur -> output
    fn chain(d: &mut Document) -> (u64, u64, u64, u64) {
        let s = add(d, "image.solid");
        d.set_value(s, 0, Value::Color(Color::rgb(0.2, 0.4, 0.6)));
        let inv = add(d, "image.invert");
        let blur = add(d, "image.blur");
        let out = add(d, "image.output");
        link(d, s, 0, inv, 0);
        link(d, inv, 0, blur, 0);
        link(d, blur, 0, out, 0);
        (s, inv, blur, out)
    }

    #[test]
    fn every_stock_schema_is_valid() {
        let (d, _) = setup();
        for s in d.registry.iter() {
            s.validate().unwrap_or_else(|e| panic!("{e}"));
        }
    }

    #[test]
    fn evaluates_a_chain() {
        let (mut d, mut e) = setup();
        let (_, _, _, out) = chain(&mut d);
        sync(&mut d, &mut e);
        let img = e.evaluate(&d, out).unwrap();
        assert_eq!((img.width, img.height), (N, N));
        let c = center(&img);
        assert!(
            near(c[0], 0.8) && near(c[1], 0.6) && near(c[2], 0.4) && near(c[3], 1.0),
            "{c:?}"
        );
        assert_eq!(e.last_evaluated.len(), 4);
    }

    #[test]
    fn unchanged_graph_evaluates_nothing_and_edits_recompute_only_downstream() {
        let (mut d, mut e) = setup();
        let (s, inv, blur, out) = chain(&mut d);
        // a second branch that must not be touched
        let other = add(&mut d, "image.solid");
        let other_out = add(&mut d, "image.output");
        link(&mut d, other, 0, other_out, 0);
        sync(&mut d, &mut e);
        e.evaluate(&d, out).unwrap();
        e.evaluate(&d, other_out).unwrap();
        e.evaluate(&d, out).unwrap();
        assert!(e.last_evaluated.is_empty());

        // change the blur radius: blur and output only
        d.set_value(blur, 1, Value::Float(2.0));
        sync(&mut d, &mut e);
        e.evaluate(&d, out).unwrap();
        let mut got = e.last_evaluated.clone();
        got.sort_unstable();
        assert_eq!(got, vec![blur, out]);

        // change the source color: everything on that branch, not the other
        d.set_value(s, 0, Value::Color(Color::rgb(1.0, 1.0, 1.0)));
        sync(&mut d, &mut e);
        let img = e.evaluate(&d, out).unwrap();
        assert!(near(center(&img)[0], 0.0));
        let mut got = e.last_evaluated.clone();
        got.sort_unstable();
        assert_eq!(got, vec![s, inv, blur, out]);
        e.evaluate(&d, other_out).unwrap();
        assert!(e.last_evaluated.is_empty());
    }

    #[test]
    fn rewiring_recomputes_and_deleting_purges_the_cache() {
        let (mut d, mut e) = setup();
        let (s, inv, _, out) = chain(&mut d);
        sync(&mut d, &mut e);
        e.evaluate(&d, out).unwrap();
        // bypass the invert node
        let blur = d.input_link(out, 0).unwrap().from.0;
        d.connect(Link {
            from: (s, 0),
            to: (blur, 0),
        })
        .unwrap();
        sync(&mut d, &mut e);
        let img = e.evaluate(&d, out).unwrap();
        assert!(near(center(&img)[0], 0.2));
        d.remove_node(inv);
        sync(&mut d, &mut e);
        assert!(!e.is_cached(inv));
    }

    #[test]
    fn muted_nodes_pass_their_input_through() {
        let (mut d, mut e) = setup();
        let (_, inv, _, out) = chain(&mut d);
        d.set_muted(inv, true);
        sync(&mut d, &mut e);
        let img = e.evaluate(&d, out).unwrap();
        assert!(near(center(&img)[0], 0.2)); // not inverted
        d.set_muted(inv, false);
        sync(&mut d, &mut e);
        assert!(near(center(&e.evaluate(&d, out).unwrap())[0], 0.8));
    }

    #[test]
    fn reroute_passes_an_image_downstream() {
        let (mut d, mut e) = setup();
        let s = add(&mut d, "image.solid");
        d.set_value(s, 0, Value::Color(Color::rgb(0.2, 0.4, 0.6)));
        let r = add(&mut d, "core.reroute");
        let out = add(&mut d, "image.output");
        link(&mut d, s, 0, r, 0);
        link(&mut d, r, 0, out, 0);
        sync(&mut d, &mut e);
        let img = e.evaluate(&d, out).unwrap();
        let c = center(&img);
        assert!(
            near(c[0], 0.2) && near(c[1], 0.4) && near(c[2], 0.6) && near(c[3], 1.0),
            "{c:?}"
        );
        assert!(e.error(r).is_none());
    }

    #[test]
    fn reroute_passes_a_parameter_value() {
        let (mut d, mut e) = setup();
        let s = add(&mut d, "image.solid");
        d.set_value(s, 0, Value::Color(Color::rgb(0.2, 0.2, 0.2)));
        let num = add(&mut d, "util.float");
        d.set_value(num, 0, Value::Float(0.3));
        let r = add(&mut d, "core.reroute");
        let bc = add(&mut d, "image.brightness_contrast");
        let out = add(&mut d, "image.output");
        link(&mut d, s, 0, bc, 0);
        link(&mut d, bc, 0, out, 0);
        link(&mut d, num, 0, r, 0);
        link(&mut d, r, 0, bc, 1); // Float -> Brightness through the reroute
        sync(&mut d, &mut e);
        assert!(near(center(&e.evaluate(&d, out).unwrap())[0], 0.5));
    }

    #[test]
    fn output_stacks_multiple_layers_top_to_bottom() {
        let (mut d, mut e) = setup();
        // Two full-canvas solids wired into the Output node. The last wire is
        // the top layer and wins where they overlap.
        let bottom = add(&mut d, "image.solid");
        d.set_value(bottom, 0, Value::Color(Color::rgb(1.0, 0.0, 0.0)));
        let top = add(&mut d, "image.solid");
        d.set_value(top, 0, Value::Color(Color::rgb(0.0, 0.0, 1.0)));
        let out = add(&mut d, "image.output");
        link(&mut d, bottom, 0, out, 0);
        link(&mut d, top, 0, out, 0);
        sync(&mut d, &mut e);
        // Both wires must land on the same (multi) input and stack in order.
        let img = e.evaluate(&d, out).unwrap();
        let c = center(&img);
        assert!(near(c[2], 1.0) && near(c[0], 0.0), "top layer wins: {c:?}");
        // The document keeps one layer per source, bottom-to-top.
        let layers = d.layers(out);
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].source, bottom);
        assert_eq!(layers[1].source, top);
    }

    #[test]
    fn output_layer_transform_moves_the_layer() {
        let (mut d, mut e) = setup();
        let n = N as f32;
        let a = add(&mut d, "image.solid");
        d.set_value(a, 0, Value::Color(Color::rgb(1.0, 1.0, 1.0)));
        let out = add(&mut d, "image.output");
        link(&mut d, a, 0, out, 0);
        sync(&mut d, &mut e);
        // No transform: the fitted layer fills the canvas.
        assert!(near(center(&e.evaluate(&d, out).unwrap())[3], 1.0));
        // Shrink and push the layer to the top-left corner.
        d.set_layer_transform(out, a, |l| {
            l.center = [n * 0.25, n * 0.25];
            l.size = [n * 0.4, n * 0.4];
        });
        sync(&mut d, &mut e);
        let img = e.evaluate(&d, out).unwrap();
        assert!(img.get(1, 1)[3] > 0.5, "layer covers the top-left corner");
        assert!(
            img.get((n - 1.0) as u32, (n - 1.0) as u32)[3] < 0.5,
            "far corner is empty"
        );
    }

    #[test]
    fn missing_inputs_are_transparent_and_errors_propagate() {
        let (mut d, mut e) = setup();
        let blur = add(&mut d, "image.blur");
        let out = add(&mut d, "image.output");
        link(&mut d, blur, 0, out, 0);
        sync(&mut d, &mut e);
        let img = e.evaluate(&d, out).unwrap();
        assert_eq!(center(&img)[3], 0.0);

        let file = add(&mut d, "image.file");
        let inv = add(&mut d, "image.invert");
        let out2 = add(&mut d, "image.output");
        link(&mut d, file, 0, inv, 0);
        link(&mut d, inv, 0, out2, 0);
        sync(&mut d, &mut e);
        let err = e.evaluate(&d, out2).unwrap_err();
        assert!(
            matches!(err, EvalError::Failed { uid, .. } if uid == out2),
            "{err:?}"
        );
        assert!(e.error(file).unwrap().contains("no file"));
        assert!(e.error(inv).unwrap().contains("upstream"));
        assert!(e.error(blur).is_none());
        // fixing the path clears the errors
        d.set_value(file, 0, Value::String("mem://red".into()));
        sync(&mut d, &mut e);
        assert!(e.error(file).is_some()); // not recomputed yet
    }

    #[test]
    fn image_file_uses_the_loader_downscales_and_reloads() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let c2 = Arc::clone(&calls);
        let loader = FnLoader(move |path: &str| {
            c2.fetch_add(1, Ordering::SeqCst);
            match path {
                "mem://big" => Ok(ImageBuf::solid(32, 16, Color::rgb(1.0, 0.0, 0.0))),
                other => Err(format!("cannot open {other}")),
            }
        });
        let mut ev = Evaluator::new((N, N)).with_loader(Arc::new(loader));
        let mut reg = Registry::new();
        ev.register_schemas(&mut reg);
        let mut d = Document::new(reg);
        let f = add(&mut d, "image.file");
        let out = add(&mut d, "image.output");
        link(&mut d, f, 0, out, 0);
        d.set_value(f, 0, Value::String("mem://big".into()));
        sync(&mut d, &mut ev);
        let img = ev.evaluate(&d, out).unwrap();
        // The Output node is a canvas: the source is aspect-fit onto the 8x8
        // canvas and centered, so the result is canvas-sized, not source-sized.
        assert_eq!((img.width, img.height), (8, 8));
        assert!(
            img.get(3, 2)[3] > 0.5,
            "fitted layer covers the canvas center"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // touching an unrelated parameter does not reload
        let other = add(&mut d, "image.blur");
        link(&mut d, f, 0, other, 0);
        sync(&mut d, &mut ev);
        let bo = add(&mut d, "image.output");
        link(&mut d, other, 0, bo, 0);
        sync(&mut d, &mut ev);
        ev.evaluate(&d, bo).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // a missing file is an error on that node
        d.set_value(f, 0, Value::String("mem://gone".into()));
        sync(&mut d, &mut ev);
        assert!(ev.evaluate(&d, out).is_err());
        assert!(ev.error(f).unwrap().contains("cannot open"));
        // invalidating forces a reload of the same path
        d.set_value(f, 0, Value::String("mem://big".into()));
        sync(&mut d, &mut ev);
        ev.evaluate(&d, out).unwrap();
        let n = calls.load(Ordering::SeqCst);
        ev.invalidate_file(&d, "mem://big");
        ev.evaluate(&d, out).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), n + 1);
    }

    #[test]
    fn parameter_nodes_drive_other_nodes() {
        let (mut d, mut e) = setup();
        let s = add(&mut d, "image.solid");
        d.set_value(s, 0, Value::Color(Color::rgb(0.2, 0.2, 0.2)));
        let bc = add(&mut d, "image.brightness_contrast");
        let num = add(&mut d, "util.float");
        let out = add(&mut d, "image.output");
        link(&mut d, s, 0, bc, 0);
        link(&mut d, bc, 0, out, 0);
        d.set_value(num, 0, Value::Float(0.3));
        link(&mut d, num, 0, bc, 1); // Float -> Brightness
        sync(&mut d, &mut e);
        assert!(near(center(&e.evaluate(&d, out).unwrap())[0], 0.5));
        d.set_value(num, 0, Value::Float(-0.1));
        sync(&mut d, &mut e);
        assert!(near(center(&e.evaluate(&d, out).unwrap())[0], 0.1));
        let mut got = e.last_evaluated.clone();
        got.sort_unstable();
        let mut want = vec![num, bc, out];
        want.sort_unstable();
        assert_eq!(got, want);
        // a Color connected to a Float socket converts (luminance)
        let cnt = add(&mut d, "util.color");
        d.set_value(cnt, 0, Value::Color(Color::rgb(0.5, 0.5, 0.5)));
        link(&mut d, cnt, 0, bc, 1); // Color -> Float (luminance)
        sync(&mut d, &mut e);
        assert!(near(center(&e.evaluate(&d, out).unwrap())[0], 0.7));
    }

    #[test]
    fn blend_node_uses_mode_opacity_and_mask_inputs() {
        let (mut d, mut e) = setup();
        let a = add(&mut d, "image.solid");
        d.set_value(a, 0, Value::Color(Color::rgb(0.5, 0.5, 0.5)));
        let b = add(&mut d, "image.solid");
        d.set_value(b, 0, Value::Color(Color::rgb(0.5, 0.25, 1.0)));
        let bl = add(&mut d, "image.blend");
        let out = add(&mut d, "image.output");
        link(&mut d, a, 0, bl, 0);
        link(&mut d, b, 0, bl, 1);
        link(&mut d, bl, 0, out, 0);
        let mode_index = d
            .schema_of(bl)
            .unwrap()
            .inputs
            .iter()
            .position(|i| i.name == "Mode")
            .unwrap();
        d.set_value(bl, mode_index, Value::Enum(1)); // Multiply
        sync(&mut d, &mut e);
        let c = center(&e.evaluate(&d, out).unwrap());
        assert!(
            near(c[0], 0.25) && near(c[1], 0.125) && near(c[2], 0.5),
            "{c:?}"
        );
    }

    #[test]
    fn cache_budget_evicts_least_recently_used_unneeded_results() {
        let (mut d, mut e) = setup();
        let one = (N * N) as usize * 16;
        // two independent chains of 2 images each
        let mut outs = Vec::new();
        for _ in 0..2 {
            let s = add(&mut d, "image.solid");
            let o = add(&mut d, "image.output");
            link(&mut d, s, 0, o, 0);
            outs.push((s, o));
        }
        sync(&mut d, &mut e);
        e.evaluate(&d, outs[0].1).unwrap();
        assert_eq!(e.cached_bytes(), 2 * one);
        e.set_cache_budget(2 * one);
        e.evaluate(&d, outs[1].1).unwrap(); // needs 2 more; the first chain is evicted
        assert!(e.cached_bytes() <= 2 * one);
        assert!(!e.is_cached(outs[0].1) && e.is_cached(outs[1].1));
        // the evicted chain still evaluates correctly (and evicts the other)
        e.evaluate(&d, outs[0].1).unwrap();
        assert!(e.is_cached(outs[0].1) && !e.is_cached(outs[1].1));
        // the current target is never evicted, even with a tiny budget
        e.set_cache_budget(1);
        e.evaluate(&d, outs[1].1).unwrap();
        assert!(e.is_cached(outs[1].1));
    }

    #[test]
    fn unknown_nodes_and_canvas_changes() {
        let (mut d, mut e) = setup();
        assert_eq!(e.evaluate(&d, 999).unwrap_err(), EvalError::NoSuchNode);
        let s = add(&mut d, "image.solid");
        let o = add(&mut d, "image.output");
        link(&mut d, s, 0, o, 0);
        sync(&mut d, &mut e);
        assert_eq!(e.evaluate(&d, o).unwrap().width, N);
        e.set_canvas((4, 4));
        assert_eq!(e.evaluate(&d, o).unwrap().width, 4);
        // a node whose first output is not an image
        let f = add(&mut d, "util.float");
        assert_eq!(e.evaluate(&d, f).unwrap_err(), EvalError::NoImage);
        // the evaluator can describe its nodes to a registry
        let mut reg = Registry::new();
        e.register_schemas(&mut reg);
        assert!(reg.get("image.blur").is_some());
    }

    #[test]
    fn every_stock_node_evaluates_with_defaults() {
        let (mut d, mut e) = setup();
        let ids: Vec<String> = d
            .registry
            .iter()
            .filter(|s| s.id.starts_with("image.") && s.id != "image.file")
            .map(|s| s.id.clone())
            .collect();
        assert!(ids.len() >= 12, "{ids:?}");
        for id in ids {
            let n = add(&mut d, &id);
            sync(&mut d, &mut e);
            let img = e
                .evaluate(&d, n)
                .unwrap_or_else(|err| panic!("{id}: {err}"));
            assert!(img.width >= 1 && img.height >= 1, "{id}");
        }
    }
}
