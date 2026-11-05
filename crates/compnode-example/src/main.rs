//! Interactive node-graph viewer built on `compnode-core`.
//!
//! `compnode-core` knows nothing about rendering. It owns the graph, the
//! spatial index, the culling, the level of detail and the wire geometry.
//! This file is the *renderer*: every frame it asks the engine what is
//! visible and paints the answer with egui.
//!
//! Controls
//!
//! - drag empty space        pan
//! - scroll                  zoom about the cursor
//! - drag a node             move it
//! - drag output -> input    connect (the ghost turns green when allowed)
//! - Delete                  remove the selected node
//! - F                       fit the whole graph
//!
//! Run with `cargo run -p compnode-example`.

use compnode_core::geom::vec2;
use compnode_core::{
    ConnectError, Graph, InPin, Lod, NodeId, OutPin, PinRef, Rect, Vec2, Viewport, Visible,
    WireCache,
};
use eframe::egui;

/// Node body payload. The engine stores it opaquely; only the renderer
/// cares what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Source,
    Filter,
    Sink,
}

impl Kind {
    fn title(self) -> &'static str {
        match self {
            Kind::Source => "Source",
            Kind::Filter => "Filter",
            Kind::Sink => "Output",
        }
    }

    fn accent(self) -> egui::Color32 {
        match self {
            Kind::Source => egui::Color32::from_rgb(90, 170, 255),
            Kind::Filter => egui::Color32::from_rgb(150, 200, 130),
            Kind::Sink => egui::Color32::from_rgb(255, 170, 90),
        }
    }
}

/// What a node stores. The engine is generic over `N`, so this is where the
/// widget state lives: the renderer reads it, lets the user edit it, and
/// writes it back through [`Graph::node_data_mut`].
#[derive(Clone, Copy, Debug)]
struct NodeData {
    kind: Kind,
    /// Slider value, 0..=1.
    amount: f32,
    /// Checkbox value.
    enabled: bool,
    /// Combo-box selection (index into [`MODES`]).
    mode: usize,
}

const MODES: &[&str] = &["normal", "multiply", "screen", "overlay"];

const NODE_W: f32 = 180.0;
const PIN_R: f32 = 4.0;
/// Pin pick radius in screen pixels (converted to graph units per zoom level).
const PIN_HIT_PX: f32 = 11.0;
/// Height of the row that holds the node body widgets.
const BODY_H: f32 = 78.0;
/// Horizontal inset of the body widgets inside the node.
const BODY_PAD: f32 = 8.0;
/// Upper bound on how many node bodies get real egui widgets per frame.
const MAX_HOSTED_WIDGETS: usize = 64;

const BG: egui::Color32 = egui::Color32::from_rgb(24, 26, 31);
const NODE_BG: egui::Color32 = egui::Color32::from_rgb(38, 41, 48);
const NODE_SEL: egui::Color32 = egui::Color32::from_rgb(72, 96, 140);
const WIRE: egui::Color32 = egui::Color32::from_rgb(150, 158, 175);
const OK: egui::Color32 = egui::Color32::from_rgb(120, 220, 140);
const BAD: egui::Color32 = egui::Color32::from_rgb(235, 100, 110);

/// What a mouse gesture is currently doing.
enum Drag {
    /// Panning the viewport.
    Pan { last: egui::Pos2 },
    /// Moving a node; `grab` is the offset from the node origin.
    Node { id: NodeId, grab: Vec2 },
    /// Pulling a wire out of an output pin.
    Wire {
        from: OutPin,
        cursor: Vec2,
        ok: bool,
    },
}

struct Demo {
    graph: Graph<NodeData>,
    view: Viewport,
    visible: Visible,
    wires: WireCache,
    selected: Option<NodeId>,
    drag: Option<Drag>,
    status: String,
}

impl Demo {
    fn new() -> Self {
        let mut graph = Graph::new();

        // `add_node` gives every node a uniform row per pin; `set_row_heights`
        // then appends a taller last row that becomes the widget body. The
        // pins keep their positions because the engine shifts them with the
        // rows.
        let mut add = |kind: Kind, pos: Vec2, inputs: usize, outputs: usize| {
            let id = graph.add_node(
                NodeData {
                    kind,
                    amount: 0.5,
                    enabled: true,
                    mode: 0,
                },
                pos,
                NODE_W,
                inputs,
                outputs,
            );
            let mut rows = vec![22.0; inputs.max(outputs).max(1)];
            rows.push(BODY_H);
            graph.set_row_heights(id, rows);
            id
        };

        let src_a = add(Kind::Source, vec2(0.0, 0.0), 0, 1);
        let src_b = add(Kind::Source, vec2(0.0, 260.0), 0, 1);
        let blur = add(Kind::Filter, vec2(340.0, 40.0), 1, 1);
        let merge = add(Kind::Filter, vec2(680.0, 40.0), 2, 1);
        let out = add(Kind::Sink, vec2(1020.0, 40.0), 1, 0);

        let out_pin = |id: NodeId, index: usize| OutPin { node: id, index };
        let in_pin = |id: NodeId, index: usize| InPin { node: id, index };

        // `connect` keeps the topological order up to date and refuses
        // cycles, so the order of these calls does not matter.
        let _ = graph.connect(out_pin(src_a, 0), in_pin(blur, 0));
        let _ = graph.connect(out_pin(src_b, 0), in_pin(merge, 1));
        let _ = graph.connect(out_pin(blur, 0), in_pin(merge, 0));
        let _ = graph.connect(out_pin(merge, 0), in_pin(out, 0));

        Self {
            graph,
            view: Viewport::new(vec2(1280.0, 720.0)),
            visible: Visible::default(),
            wires: WireCache::new(),
            selected: None,
            drag: None,
            status: "ready".to_owned(),
        }
    }
}

/// Bounding box of every node in `graph`.
fn graph_bounds(graph: &Graph<NodeData>) -> Rect {
    let mut bounds: Option<Rect> = None;
    for (_, node) in graph.nodes() {
        let r = node.rect();
        bounds = Some(match bounds {
            Some(b) => b.union(&r),
            None => r,
        });
    }
    bounds.unwrap_or_else(|| Rect::new(Vec2::ZERO, vec2(1.0, 1.0)))
}

/// The body area of a node (its last row), in graph space.
///
/// The engine stores per-row heights, so the renderer can ask where the body
/// starts without duplicating the layout maths.
fn node_body_rect<N>(node: &compnode_core::Node<N>) -> Option<Rect> {
    let rows = node.rows();
    let (&h, above) = rows.split_last()?;
    let top = node.pos().y + compnode_core::layout::HEADER_H + above.iter().sum::<f32>();
    let x = node.pos().x + BODY_PAD;
    Some(Rect::new(
        vec2(x, top),
        vec2(node.pos().x + node.width() - BODY_PAD, top + h),
    ))
}

/// Graph space -> screen space, as an egui position.
fn to_screen(view: &Viewport, p: Vec2) -> egui::Pos2 {
    let s = view.to_screen(p);
    egui::pos2(s.x, s.y)
}

/// egui position -> graph space.
fn to_graph(view: &Viewport, p: egui::Pos2) -> Vec2 {
    view.to_graph(vec2(p.x, p.y))
}

fn main() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "compnode-core example",
        options,
        Box::new(|_cc| Ok(Box::new(DemoApp(Demo::new())))),
    )?;
    Ok(())
}

struct DemoApp(Demo);

impl eframe::App for DemoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("hint").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("drag: pan / node / wire");
                ui.label("| scroll: zoom");
                ui.label("| Del: remove node");
                ui.label("| F: fit");
            });
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let d = &self.0;
                ui.label(format!(
                    "{} nodes · {} wires · {} visible · zoom {:.2} · lod {}",
                    d.graph.node_count(),
                    d.graph.wire_count(),
                    d.visible.nodes.len(),
                    d.view.zoom,
                    match d.view.lod() {
                        Lod::Full => "full",
                        Lod::Compact => "compact",
                        Lod::Dot => "dot",
                    },
                ));
                ui.separator();
                ui.label(d.status.as_str());
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            let (resp, painter) =
                ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
            let rect = resp.rect;
            painter.rect_filled(rect, egui::CornerRadius::ZERO, BG);

            // Split the borrow so the engine's fields can be used mutably
            // and immutably at the same time.
            let Demo {
                graph,
                view,
                visible,
                wires,
                selected,
                drag,
                status,
            } = &mut self.0;

            view.size = vec2(rect.width(), rect.height());

            // ---- input -------------------------------------------------
            let (pressed, down, released, pointer, scroll) = ui.input(|i| {
                (
                    i.pointer.primary_pressed(),
                    i.pointer.primary_down(),
                    i.pointer.primary_released(),
                    i.pointer.interact_pos(),
                    i.smooth_scroll_delta.y,
                )
            });

            if scroll != 0.0
                && let Some(p) = pointer.filter(|p| rect.contains(*p))
            {
                view.zoom_at(to_graph(view, p), 1.0 + scroll * 0.01);
            }

            let gp = pointer.map(|p| to_graph(view, p));

            if pressed && let Some(gp) = gp {
                let hit_radius = PIN_HIT_PX / view.zoom;
                if let Some(PinRef::Out(from)) = graph.hit_pin(gp, hit_radius) {
                    *drag = Some(Drag::Wire {
                        from,
                        cursor: gp,
                        ok: false,
                    });
                    *status = "dragging a wire...".to_owned();
                } else if let Some(id) = graph.hit_node(gp) {
                    *selected = Some(id);
                    let node = graph.node(id);
                    let in_body = node
                        .and_then(node_body_rect)
                        .is_some_and(|body| body.contains(gp));
                    // A press inside the body belongs to the widgets, not to
                    // the node: do not start a drag, or the slider would move
                    // the node instead of the value.
                    if !in_body {
                        let grab = gp - node.map(|n| n.pos()).unwrap_or(Vec2::ZERO);
                        *drag = Some(Drag::Node { id, grab });
                    }
                } else if let Some(p) = pointer {
                    *drag = Some(Drag::Pan { last: p });
                }
            }

            if down
                && !pressed
                && let (Some(p), Some(d)) = (pointer, drag.as_mut())
            {
                let gp = to_graph(view, p);
                match d {
                    Drag::Pan { last } => {
                        let delta = vec2(p.x - last.x, p.y - last.y);
                        *last = p;
                        view.pan(delta);
                    }
                    Drag::Node { id, grab } => {
                        graph.move_node(*id, gp - *grab);
                    }
                    Drag::Wire { from, cursor, ok } => {
                        *cursor = gp;
                        *ok = match graph.hit_pin(gp, PIN_HIT_PX / view.zoom) {
                            Some(PinRef::In(to)) => graph.can_connect(*from, to).is_ok(),
                            _ => false,
                        };
                    }
                }
            }

            if released {
                if let Some(Drag::Wire { from, .. }) = drag.take()
                    && let Some(gp) = gp
                    && let Some(PinRef::In(to)) = graph.hit_pin(gp, PIN_HIT_PX / view.zoom)
                {
                    match graph.connect(from, to) {
                        Ok(_) => *status = "connected".to_owned(),
                        Err(ConnectError::Cycle) => {
                            *status = "refused: that would create a cycle".to_owned()
                        }
                        Err(e) => *status = format!("refused: {e:?}"),
                    }
                }
                *drag = None;
            }

            if !down {
                *drag = None;
            }

            let ctx = ui.ctx();
            if ctx.input(|i| i.key_pressed(egui::Key::Delete))
                && let Some(id) = *selected
            {
                graph.remove_node(id);
                *selected = None;
                *status = "node removed".to_owned();
            }
            if ctx.input(|i| i.key_pressed(egui::Key::F)) {
                *view = Viewport::fit(view.size, graph_bounds(graph), 48.0);
            }

            // ---- draw --------------------------------------------------
            wires.begin_frame();
            graph.collect_visible(&view.visible_rect(64.0), visible);

            let lod = view.lod();
            let tol = view.wire_tolerance();

            for &wid in &visible.wires {
                let Some((a, b)) = graph.wire_endpoints(wid) else {
                    continue;
                };
                // The engine returns the flattened polyline in graph space.
                let pts = wires.get(wid, a, b, tol);
                let screen: Vec<egui::Pos2> = pts.iter().map(|p| to_screen(view, *p)).collect();
                if screen.len() >= 2 {
                    painter.add(egui::Shape::line(screen, egui::Stroke::new(2.0, WIRE)));
                }
            }

            for &nid in &visible.nodes {
                let Some(node) = graph.node(nid) else {
                    continue;
                };
                let kind = node.data.kind;
                let r = node.rect();
                let screen_rect = egui::Rect::from_min_size(
                    to_screen(view, r.min),
                    egui::vec2(r.width() * view.zoom, r.height() * view.zoom),
                );

                let is_selected = *selected == Some(nid);
                painter.rect_filled(
                    screen_rect,
                    egui::CornerRadius::same(6),
                    if is_selected { NODE_SEL } else { NODE_BG },
                );
                painter.rect_stroke(
                    screen_rect,
                    egui::CornerRadius::same(6),
                    egui::Stroke::new(1.0, kind.accent()),
                    egui::StrokeKind::Inside,
                );

                if lod != Lod::Dot {
                    for i in 0..node.inputs() {
                        painter.circle_filled(
                            to_screen(view, node.in_pos(i)),
                            PIN_R,
                            kind.accent(),
                        );
                    }
                    for i in 0..node.outputs() {
                        painter.circle_filled(
                            to_screen(view, node.out_pos(i)),
                            PIN_R,
                            kind.accent(),
                        );
                    }
                }

                if lod == Lod::Full {
                    painter.text(
                        egui::pos2(screen_rect.left() + 10.0, screen_rect.top() + 14.0),
                        egui::Align2::LEFT_CENTER,
                        kind.title(),
                        egui::FontId::proportional(13.0),
                        egui::Color32::WHITE,
                    );
                }
            }

            // ---- real egui widgets inside the node bodies ---------------
            //
            // `Ui::new_child` with an explicit `max_rect` gives us a `Ui`
            // positioned exactly on the node body, so any egui widget can be
            // dropped in and behaves like a normal widget (focus, keyboard,
            // accessibility). `new_child` does not allocate space in the
            // parent, so the canvas layout is untouched.
            //
            // Only at `Lod::Full`: at lower zoom the body is not drawn, and
            // hosting widgets for hundreds of nodes would be wasteful.
            if lod == Lod::Full {
                let mut hosted = 0usize;
                for &nid in &visible.nodes {
                    if hosted >= MAX_HOSTED_WIDGETS {
                        break;
                    }
                    let Some(node) = graph.node(nid) else {
                        continue;
                    };
                    let Some(body) = node_body_rect(node) else {
                        continue;
                    };
                    // Skip nodes whose body is too small on screen to be usable.
                    if body.width() * view.zoom < 60.0 {
                        continue;
                    }
                    let Some(mut data) = graph.node_data_mut(nid).copied() else {
                        continue;
                    };

                    let screen_body = egui::Rect::from_min_max(
                        to_screen(view, body.min),
                        to_screen(view, body.max),
                    );
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .id(egui::Id::new(("node_body", nid)))
                            .max_rect(screen_body)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    child.spacing_mut().item_spacing.y = 4.0;

                    child.horizontal(|ui| {
                        ui.checkbox(&mut data.enabled, "on");
                        ui.label(format!("{:.0}%", data.amount * 100.0));
                    });
                    child.add(egui::Slider::new(&mut data.amount, 0.0..=1.0).show_value(false));
                    egui::ComboBox::from_id_salt(("mode", nid))
                        .selected_text(MODES[data.mode.min(MODES.len() - 1)])
                        .show_ui(&mut child, |ui| {
                            for (i, m) in MODES.iter().enumerate() {
                                ui.selectable_value(&mut data.mode, i, *m);
                            }
                        });

                    // Write the edited values back into the graph.
                    if let Some(slot) = graph.node_data_mut(nid) {
                        *slot = data;
                    }
                    hosted += 1;
                }
            }

            // The wire currently being dragged.
            if let Some(Drag::Wire { from, cursor, ok }) = drag.as_ref()
                && let Some(node) = graph.node(from.node)
            {
                let a = to_screen(view, node.out_pos(from.index));
                let b = to_screen(view, *cursor);
                painter.add(egui::Shape::line(
                    vec![a, b],
                    egui::Stroke::new(2.0, if *ok { OK } else { BAD }),
                ));
            }

            wires.end_frame();
        });
    }
}
