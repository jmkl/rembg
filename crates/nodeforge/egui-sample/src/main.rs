//! nodeforge + egui sample.
//!
//! Wheel: zoom at cursor | drag empty canvas or middle-drag: pan
//! Drag node: move | drag output pin -> input pin: connect | Del: delete node
//!
//! The engine does culling, wire flattening and hit-testing; this file only
//! turns the visible set into egui shapes and routes input.

use eframe::egui::{
    self, epaint::StrokeKind, Align2, Color32, CornerRadius, FontId, Frame, Key, PointerButton, Pos2, Rect, Sense,
    Shape, Stroke, Ui,
};
use nodeforge::geom::vec2;
use nodeforge::wire::{control_points, flatten};
use nodeforge::{Graph, InPin, Lod, NodeId, OutPin, PinRef, Vec2, Viewport, Visible, WireCache, WireId};
use std::time::Instant;

// ---- design tokens ---------------------------------------------------------
mod theme {
    use super::Color32;
    pub const BG: Color32 = Color32::from_rgb(24, 25, 28);
    pub const GRID: Color32 = Color32::from_rgb(44, 46, 52);
    pub const NODE_BODY: Color32 = Color32::from_rgb(42, 44, 50);
    pub const NODE_BORDER: Color32 = Color32::from_rgb(64, 67, 76);
    pub const TEXT: Color32 = Color32::from_rgb(222, 224, 229);
    pub const TEXT_DIM: Color32 = Color32::from_rgb(150, 154, 164);
    pub const WIRE: Color32 = Color32::from_rgb(120, 130, 150);
    pub const ACCENT: Color32 = Color32::from_rgb(92, 160, 255);
    pub const PIN_IDLE: Color32 = Color32::from_rgb(30, 31, 35);
    /// Header colors per node kind.
    pub const KINDS: [(&str, Color32); 6] = [
        ("Value", Color32::from_rgb(88, 110, 150)),
        ("Math", Color32::from_rgb(96, 128, 98)),
        ("Mix", Color32::from_rgb(140, 112, 76)),
        ("Noise", Color32::from_rgb(120, 92, 140)),
        ("Transform", Color32::from_rgb(76, 126, 134)),
        ("Output", Color32::from_rgb(150, 82, 86)),
    ];
    pub const PIN_HIT_PX: f32 = 10.0;
    pub const WIRE_HIT_PX: f32 = 6.0;
}

fn to_e(p: Vec2, origin: Pos2) -> Pos2 {
    Pos2::new(origin.x + p.x, origin.y + p.y)
}

// ---- demo graph ------------------------------------------------------------
type Data = u32; // node index; kind = index % 6

fn generate(n: usize) -> Graph<Data> {
    let cols = (n as f32).sqrt().ceil().max(1.0) as usize;
    let mut state = 42u64;
    let mut rnd = |m: usize| {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((state >> 33) as usize) % m
    };
    let mut g = Graph::new();
    let ids: Vec<NodeId> = (0..n)
        .map(|i| {
            let pos = vec2((i % cols) as f32 * 260.0 + rnd(40) as f32, (i / cols) as f32 * 160.0 + rnd(30) as f32);
            g.add_node(i as u32, pos, 180.0, 3, 2)
        })
        .collect();
    let mut edges = Vec::new();
    for i in 0..n {
        let (r, c) = (i / cols, i % cols);
        if c + 1 >= cols {
            continue;
        }
        for out in 0..2 {
            let tr = (r + rnd(5)).saturating_sub(2).min((n - 1) / cols);
            let j = tr * cols + c + 1;
            if j < n {
                edges.push((OutPin { node: ids[i], index: out }, InPin { node: ids[j], index: rnd(3) }));
            }
        }
    }
    g.load_edges(&edges).expect("generated graph is acyclic");
    g
}

fn graph_bounds(g: &Graph<Data>) -> Option<nodeforge::Rect> {
    let mut it = g.nodes().map(|(_, n)| n.rect());
    let first = it.next()?;
    Some(it.fold(first, |a, r| nodeforge::Rect::new(
        vec2(a.min.x.min(r.min.x), a.min.y.min(r.min.y)),
        vec2(a.max.x.max(r.max.x), a.max.y.max(r.max.y)),
    )))
}

// ---- app -------------------------------------------------------------------
enum Drag {
    None,
    Pan,
    Node { id: NodeId, grab: Vec2 },
    Wire { from: OutPin },
}

#[derive(Default)]
struct Stats {
    ms: f32,
    nodes: usize,
    wires: usize,
}

struct App {
    graph: Graph<Data>,
    vp: Viewport,
    cache: WireCache,
    vis: Visible,
    drag: Drag,
    selected: Option<NodeId>,
    node_count: usize,
    need_fit: bool,
    stats: Stats,
    pts: Vec<Pos2>,
    poly: Vec<nodeforge::Vec2>,
}

impl App {
    fn from_graph(graph: Graph<Data>, node_count: usize) -> Self {
        App {
            graph,
            vp: Viewport::new(vec2(1200.0, 800.0)),
            cache: WireCache::new(),
            vis: Visible::default(),
            drag: Drag::None,
            selected: None,
            node_count,
            need_fit: true,
            stats: Stats::default(),
            pts: Vec::new(),
            poly: Vec::new(),
        }
    }

    fn new() -> Self {
        Self::from_graph(generate(1500), 1500)
    }

    fn fit(&mut self) {
        if let Some(b) = graph_bounds(&self.graph) {
            self.vp = Viewport::fit(self.vp.size, b, 40.0);
        }
    }

    /// Canvas: input handling first (using last frame's layout), then paint.
    fn canvas_ui(&mut self, ui: &mut Ui) {
        let t0 = Instant::now();
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        self.vp.size = vec2(rect.width(), rect.height());
        if self.need_fit {
            self.fit();
            self.need_fit = false;
        }
        let origin = rect.min;
        let local = |p: Pos2| vec2(p.x - origin.x, p.y - origin.y);
        let hit_r = theme::PIN_HIT_PX / self.vp.zoom;

        // -- zoom
        if resp.hovered() {
            let (scroll, zd) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = zd * (scroll * 0.0015).exp();
            if (factor - 1.0).abs() > 1e-4 {
                if let Some(p) = resp.hover_pos() {
                    self.vp.zoom_at(local(p), factor);
                }
            }
        }

        // -- drag start: decide what is being dragged
        if resp.drag_started_by(PointerButton::Primary) {
            let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(resp.interact_pointer_pos().unwrap_or(origin));
            let pg = self.vp.to_graph(local(press));
            self.drag = match self.graph.hit_pin(pg, hit_r) {
                Some(PinRef::Out(from)) => Drag::Wire { from },
                Some(PinRef::In(to)) => match self.graph.node(to.node).and_then(|n| n.input_wire(to.index)) {
                    // grabbing a connected input detaches the wire and keeps dragging its source
                    Some(w) => {
                        let from = self.graph.wire(w).map(|w| w.from);
                        self.graph.disconnect(w);
                        from.map_or(Drag::None, |from| Drag::Wire { from })
                    }
                    None => Drag::None,
                },
                None => match self.graph.hit_node(pg) {
                    Some(id) => {
                        self.selected = Some(id);
                        let pos = self.graph.node(id).map_or(Vec2::ZERO, |n| n.pos());
                        Drag::Node { id, grab: pg - pos }
                    }
                    None => Drag::Pan,
                },
            };
        }
        let pointer_g = resp.interact_pointer_pos().or(resp.hover_pos()).map(|p| self.vp.to_graph(local(p)));
        if resp.dragged_by(PointerButton::Middle) {
            self.vp.pan(vec2(resp.drag_delta().x, resp.drag_delta().y));
        } else if resp.dragged_by(PointerButton::Primary) {
            match (&self.drag, pointer_g) {
                (Drag::Pan, _) => self.vp.pan(vec2(resp.drag_delta().x, resp.drag_delta().y)),
                (Drag::Node { id, grab }, Some(pg)) => {
                    self.graph.move_node(*id, pg - *grab);
                }
                _ => {}
            }
        }
        if resp.drag_stopped() {
            if let (Drag::Wire { from }, Some(pg)) = (&self.drag, pointer_g) {
                if let Some(PinRef::In(to)) = self.graph.hit_pin(pg, hit_r) {
                    let _ = self.graph.connect(*from, to); // cycles/self-loops are rejected by the engine
                }
            }
            self.drag = Drag::None;
        }
        if resp.clicked() {
            self.selected = pointer_g.and_then(|p| self.graph.hit_node(p));
        }
        if let Some(sel) = self.selected {
            if ui.input(|i| i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace)) {
                self.graph.remove_node(sel);
                self.selected = None;
            }
        }

        // -- hover highlight (wires only when not over a node)
        let tol = self.vp.wire_tolerance();
        let hover_wire: Option<WireId> = match (&self.drag, resp.hover_pos()) {
            (Drag::None, Some(hp)) => {
                let pg = self.vp.to_graph(local(hp));
                if self.graph.hit_node(pg).is_none() {
                    self.graph.hit_wire(pg, theme::WIRE_HIT_PX / self.vp.zoom, tol, &mut self.cache)
                } else {
                    None
                }
            }
            _ => None,
        };

        // -- paint
        painter.rect_filled(rect, 0.0, theme::BG);
        self.paint_grid(&painter, rect);

        self.cache.begin_frame();
        self.graph.collect_visible(&self.vp.visible_rect(32.0), &mut self.vis);

        let wire_w = (2.0 * self.vp.zoom.sqrt()).clamp(1.0, 3.0);
        for &w in &self.vis.wires {
            let Some((a, b)) = self.graph.wire_endpoints(w) else { continue };
            let poly = self.cache.get(w, a, b, tol);
            self.pts.clear();
            self.pts.extend(poly.iter().map(|p| to_e(self.vp.to_screen(*p), origin)));
            let hot = hover_wire == Some(w);
            let stroke = if hot { Stroke::new(wire_w + 1.0, theme::ACCENT) } else { Stroke::new(wire_w, theme::WIRE) };
            painter.add(Shape::line(self.pts.clone(), stroke));
        }
        self.cache.end_frame();

        // wire being dragged
        if let (Drag::Wire { from }, Some(pg)) = (&self.drag, pointer_g) {
            if let Some(n) = self.graph.node(from.node) {
                self.poly.clear();
                flatten(&control_points(n.out_pos(from.index), pg), tol, &mut self.poly);
                let pts = self.poly.iter().map(|p| to_e(self.vp.to_screen(*p), origin)).collect();
                painter.add(Shape::line(pts, Stroke::new(wire_w + 1.0, theme::ACCENT)));
            }
        }

        // nodes: selected last so it is drawn on top
        let lod = self.vp.lod();
        let mut order: Vec<NodeId> = self.vis.nodes.clone();
        order.sort_by_key(|id| Some(*id) == self.selected);
        for id in order {
            self.paint_node(&painter, id, origin, lod);
        }

        // overlay
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        self.stats.ms = if self.stats.ms == 0.0 { ms } else { self.stats.ms * 0.9 + ms * 0.1 };
        self.stats.nodes = self.vis.nodes.len();
        self.stats.wires = self.vis.wires.len();
        painter.text(
            rect.left_top() + egui::vec2(12.0, 10.0),
            Align2::LEFT_TOP,
            format!(
                "{}/{} nodes  {}/{} wires  zoom {:.2}  lod {:?}  cpu {:.2} ms",
                self.stats.nodes,
                self.graph.node_count(),
                self.stats.wires,
                self.graph.wire_count(),
                self.vp.zoom,
                lod,
                self.stats.ms
            ),
            FontId::monospace(11.0),
            theme::TEXT_DIM,
        );
        painter.text(
            rect.left_bottom() + egui::vec2(12.0, -10.0),
            Align2::LEFT_BOTTOM,
            "wheel zoom · drag canvas pan · drag node move · output→input connect · Del delete",
            FontId::proportional(11.0),
            theme::TEXT_DIM,
        );
    }

    fn paint_grid(&self, painter: &egui::Painter, rect: Rect) {
        // dot spacing doubles until it is >= 14 px on screen
        let mut step = 24.0_f32;
        while step * self.vp.zoom < 14.0 {
            step *= 2.0;
        }
        let vr = self.vp.visible_rect(0.0);
        let (x0, y0) = ((vr.min.x / step).floor() as i32, (vr.min.y / step).floor() as i32);
        let (x1, y1) = ((vr.max.x / step).ceil() as i32, (vr.max.y / step).ceil() as i32);
        let r = (1.1 * self.vp.zoom.sqrt()).clamp(0.6, 1.6);
        for iy in y0..=y1 {
            for ix in x0..=x1 {
                let s = self.vp.to_screen(vec2(ix as f32 * step, iy as f32 * step));
                painter.circle_filled(to_e(s, rect.min), r, theme::GRID);
            }
        }
    }

    fn paint_node(&self, painter: &egui::Painter, id: NodeId, origin: Pos2, lod: Lod) {
        let Some(node) = self.graph.node(id) else { return };
        let z = self.vp.zoom;
        let r = node.rect();
        let rect = Rect::from_min_max(
            to_e(self.vp.to_screen(r.min), origin),
            to_e(self.vp.to_screen(r.max), origin),
        );
        let (name, kind_color) = theme::KINDS[node.data as usize % theme::KINDS.len()];
        let selected = self.selected == Some(id);
        let radius = CornerRadius::same((6.0 * z).clamp(1.0, 12.0) as u8);

        if lod == Lod::Dot {
            painter.rect_filled(rect, radius, kind_color);
            if selected {
                painter.rect_stroke(rect, radius, Stroke::new(1.5, theme::ACCENT), StrokeKind::Outside);
            }
            return;
        }

        painter.rect_filled(rect, radius, theme::NODE_BODY);
        let header = Rect::from_min_size(rect.min, egui::vec2(rect.width(), nodeforge::layout::HEADER_H * z));
        painter.rect_filled(header, CornerRadius { nw: radius.nw, ne: radius.ne, sw: 0, se: 0 }, kind_color);
        let border = if selected { Stroke::new(1.5, theme::ACCENT) } else { Stroke::new(1.0, theme::NODE_BORDER) };
        painter.rect_stroke(rect, radius, border, StrokeKind::Inside);

        let pin_r = 4.5 * z;
        let full = lod == Lod::Full;
        if full {
            painter.text(
                header.left_center() + egui::vec2(10.0 * z, 0.0),
                Align2::LEFT_CENTER,
                format!("{name} {}", node.data),
                FontId::proportional(12.5 * z),
                theme::TEXT,
            );
        }
        for i in 0..node.inputs() {
            let p = to_e(self.vp.to_screen(node.in_pos(i)), origin);
            let connected = node.input_wire(i).is_some();
            self.paint_pin(painter, p, pin_r, connected);
            if full {
                painter.text(p + egui::vec2(10.0 * z, 0.0), Align2::LEFT_CENTER, ["A", "B", "C"][i % 3],
                    FontId::proportional(11.0 * z), theme::TEXT_DIM);
            }
        }
        for i in 0..node.outputs() {
            let p = to_e(self.vp.to_screen(node.out_pos(i)), origin);
            let connected = !node.output_wires(i).is_empty();
            self.paint_pin(painter, p, pin_r, connected);
            if full {
                painter.text(p - egui::vec2(10.0 * z, 0.0), Align2::RIGHT_CENTER, ["X", "Y"][i % 2],
                    FontId::proportional(11.0 * z), theme::TEXT_DIM);
            }
        }
    }

    fn paint_pin(&self, painter: &egui::Painter, p: Pos2, r: f32, connected: bool) {
        let fill = if connected { theme::ACCENT } else { theme::PIN_IDLE };
        painter.circle(p, r, fill, Stroke::new((1.2 * self.vp.zoom).max(0.8), theme::WIRE));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong("nodeforge · egui sample");
                ui.separator();
                ui.label("nodes");
                ui.add(egui::Slider::new(&mut self.node_count, 100..=40_000).logarithmic(true));
                if ui.button("Regenerate").clicked() {
                    self.graph = generate(self.node_count);
                    self.cache = WireCache::new();
                    self.selected = None;
                    self.need_fit = true;
                }
                if ui.button("Fit").clicked() {
                    self.need_fit = true;
                }
            });
        });
        egui::CentralPanel::default().frame(Frame::NONE).show(ctx, |ui| self.canvas_ui(ui));
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]).with_title("nodeforge"),
        ..Default::default()
    };
    eframe::run_native(
        "nodeforge",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(App::new()))
        }),
    )
}

// ---- headless tests: real egui frames with injected pointer input ----------
#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, RawInput};

    fn frame(ctx: &egui::Context, app: &mut App, t: f64, events: Vec<Event>) -> egui::FullOutput {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 700.0))),
            time: Some(t),
            events,
            ..Default::default()
        };
        ctx.run(input, |ctx| {
            egui::CentralPanel::default().frame(Frame::NONE).show(ctx, |ui| app.canvas_ui(ui));
        })
    }

    fn press(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
    }

    /// Drive a primary-button drag through several frames.
    fn drag(ctx: &egui::Context, app: &mut App, t: f64, from: Pos2, to: Pos2) {
        let mid = Pos2::new((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
        frame(ctx, app, t, vec![Event::PointerMoved(from)]);
        frame(ctx, app, t + 0.05, vec![press(from, true)]);
        frame(ctx, app, t + 0.10, vec![Event::PointerMoved(mid)]);
        frame(ctx, app, t + 0.15, vec![Event::PointerMoved(to)]);
        frame(ctx, app, t + 0.20, vec![press(to, false)]);
        frame(ctx, app, t + 0.25, vec![]);
    }

    fn fixed_app(g: Graph<Data>) -> App {
        let mut app = App::from_graph(g, 2);
        app.need_fit = false;
        app.vp = Viewport::new(vec2(1000.0, 700.0)); // zoom 1, origin 0 => graph == screen
        app
    }

    #[test]
    fn renders_visible_nodes_and_wires() {
        let mut app = App::from_graph(generate(400), 400);
        let ctx = egui::Context::default();
        let out = frame(&ctx, &mut app, 0.0, vec![]);
        assert!(!out.shapes.is_empty());
        assert!(app.stats.nodes > 0 && app.stats.wires > 0);
        assert!(app.stats.nodes <= 400);
    }

    #[test]
    fn dragging_a_node_moves_it() {
        let mut g = Graph::new();
        let id = g.add_node(0, vec2(100.0, 100.0), 180.0, 1, 1);
        let mut app = fixed_app(g);
        let ctx = egui::Context::default();
        frame(&ctx, &mut app, -0.1, vec![]); // establish layout
        drag(&ctx, &mut app, 0.0, Pos2::new(150.0, 110.0), Pos2::new(250.0, 210.0)); // header grab
        let pos = app.graph.node(id).unwrap().pos();
        assert!((pos.x - 200.0).abs() < 1.0 && (pos.y - 200.0).abs() < 1.0, "moved to {pos:?}");
        assert_eq!(app.selected, Some(id));
    }

    #[test]
    fn dragging_output_to_input_connects_and_rejects_cycles() {
        let mut g = Graph::new();
        let a = g.add_node(0, vec2(100.0, 100.0), 180.0, 1, 1);
        let b = g.add_node(1, vec2(500.0, 100.0), 180.0, 1, 1);
        let a_out = g.node(a).unwrap().out_pos(0);
        let b_in = g.node(b).unwrap().in_pos(0);
        let b_out = g.node(b).unwrap().out_pos(0);
        let a_in = g.node(a).unwrap().in_pos(0);
        let mut app = fixed_app(g);
        let ctx = egui::Context::default();
        frame(&ctx, &mut app, -0.1, vec![]);

        drag(&ctx, &mut app, 0.0, Pos2::new(a_out.x, a_out.y), Pos2::new(b_in.x, b_in.y));
        assert_eq!(app.graph.wire_count(), 1);

        // b -> a would close a cycle: engine must refuse
        drag(&ctx, &mut app, 10.0, Pos2::new(b_out.x, b_out.y), Pos2::new(a_in.x, a_in.y));
        assert_eq!(app.graph.wire_count(), 1);
    }

    #[test]
    fn wheel_zoom_keeps_point_under_cursor() {
        let mut app = App::from_graph(generate(50), 50);
        let ctx = egui::Context::default();
        frame(&ctx, &mut app, 0.0, vec![]);
        let cursor = Pos2::new(400.0, 300.0);
        let before = app.vp.to_graph(vec2(cursor.x, cursor.y));
        let z0 = app.vp.zoom;
        frame(&ctx, &mut app, 0.1, vec![Event::PointerMoved(cursor)]);
        frame(&ctx, &mut app, 0.2, vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 120.0),
            modifiers: Modifiers::NONE,
        }]);
        let after = app.vp.to_graph(vec2(cursor.x, cursor.y));
        assert!(app.vp.zoom != z0, "zoom unchanged");
        assert!((before - after).length() < 0.5, "anchor drifted {before:?} -> {after:?}");
    }
}
