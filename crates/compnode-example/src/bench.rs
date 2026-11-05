//! Frame-cost benchmark.
//!
//! Run: `cargo run --release -p compnode-example --bin bench`
//! Smoke run with custom sizes: `... --bin bench -- 1000 2000`
use compnode_core::geom::vec2;
use compnode_core::{Graph, InPin, OutPin, Viewport, Visible, WireCache};
use std::time::Instant;

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() as usize) % n
    }
}

fn build(n: usize) -> (Graph<()>, Vec<compnode_core::NodeId>) {
    let cols = (n as f32).sqrt().ceil() as usize;
    let mut rng = Lcg(42);
    let mut g = Graph::new();
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let (c, r) = (i % cols, i / cols);
        let jitter = vec2(rng.below(40) as f32, rng.below(30) as f32);
        ids.push(g.add_node(
            (),
            vec2(c as f32 * 260.0, r as f32 * 160.0) + jitter,
            180.0,
            3,
            2,
        ));
    }
    // forward-only edges (column c -> nearby node in column c+1): acyclic by construction.
    // Nodes are created row-major but edges run column-wise, i.e. NOT in dependency
    // order -- exactly the case `load_edges` exists for.
    let mut edges = Vec::new();
    for i in 0..n {
        let c = i % cols;
        if c + 1 >= cols {
            continue;
        }
        for out in 0..2 {
            let r = i / cols;
            let tr = (r + rng.below(5)).saturating_sub(2).min((n - 1) / cols);
            let j = (tr * cols + c + 1).min(n - 1);
            if j % cols == c + 1 {
                edges.push((
                    OutPin {
                        node: ids[i],
                        index: out,
                    },
                    InPin {
                        node: ids[j],
                        index: rng.below(3),
                    },
                ));
            }
        }
    }
    g.load_edges(&edges).expect("acyclic");
    (g, ids)
}

/// One frame of "everything except the GPU": cull + flatten visible wires.
fn frame(
    g: &Graph<()>,
    vp: &Viewport,
    vis: &mut Visible,
    cache: &mut WireCache,
) -> (usize, usize, usize) {
    cache.begin_frame();
    g.collect_visible(&vp.visible_rect(64.0), vis);
    let tol = vp.wire_tolerance();
    let mut pts = 0;
    for &w in &vis.wires {
        if let Some((a, b)) = g.wire_endpoints(w) {
            pts += cache.get(w, a, b, tol).len();
        }
    }
    cache.end_frame();
    (vis.nodes.len(), vis.wires.len(), pts)
}

fn time<F: FnMut()>(iters: usize, mut f: F) -> f64 {
    let t = Instant::now();
    for _ in 0..iters {
        f();
    }
    t.elapsed().as_secs_f64() * 1e6 / iters as f64
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let sizes = if args.is_empty() {
        vec![1_000, 5_000, 20_000]
    } else {
        args
    };
    for &n in &sizes {
        let t = Instant::now();
        let (mut g, ids) = build(n);
        let build_ms = t.elapsed().as_secs_f64() * 1e3;
        println!(
            "\n== {n} nodes, {} wires (build {build_ms:.1} ms) ==",
            g.wire_count()
        );

        let mut vis = Visible::default();
        let mut cache = WireCache::new();
        let mut vp = Viewport::new(vec2(1920.0, 1080.0));
        vp.origin = vec2(2000.0, 1500.0);

        let (nn, nw, np) = frame(&g, &vp, &mut vis, &mut cache);
        let cold = time(1, || {
            cache = WireCache::new();
            frame(&g, &vp, &mut vis, &mut cache);
        });
        let warm = time(2000, || {
            frame(&g, &vp, &mut vis, &mut cache);
        });
        println!(
            "zoom 1.0  : {nn:5} nodes {nw:5} wires {np:6} pts | cold {cold:8.1} us | warm {warm:7.1} us"
        );

        // continuous panning (cache churns as new wires scroll in)
        let mut x = 0.0;
        let pan = time(2000, || {
            x += 7.0;
            vp.origin.x = 2000.0 + (x % 3000.0);
            frame(&g, &vp, &mut vis, &mut cache);
        });
        println!("panning   :                                   | avg  {pan:8.1} us/frame");

        // zoomed way out showing (nearly) the whole graph
        let mut far = Viewport::new(vec2(1920.0, 1080.0));
        far.zoom = 0.05;
        let (nn, nw, np) = frame(&g, &far, &mut vis, &mut cache);
        let t_far = time(200, || {
            frame(&g, &far, &mut vis, &mut cache);
        });
        println!("zoom 0.05 : {nn:5} nodes {nw:5} wires {np:6} pts | avg  {t_far:8.1} us/frame");

        // drag a node back and forth
        let victim = ids[ids.len() / 2];
        let base = g.node(victim).unwrap().pos();
        let mut k = 0.0f32;
        let drag = time(20_000, || {
            k += 1.0;
            g.move_node(victim, base + vec2((k * 3.0) % 400.0, (k * 2.0) % 300.0));
        });
        println!("drag node : {drag:8.2} us/move");

        // hover hit-testing
        let mut cache2 = WireCache::new();
        let mut m = 0.0f32;
        let hit = time(20_000, || {
            m += 13.0;
            let p = vp.origin + vec2(m % 1800.0, (m * 0.7) % 1000.0);
            let _ = g.hit_node(p);
            let _ = g.hit_wire(p, 4.0, vp.wire_tolerance(), &mut cache2);
        });
        println!("hover test: {hit:8.2} us/probe (node+wire)");
    }
}
