//! Turning the editor state into a [`DrawList`]. Everything is emitted in
//! graph space; the host applies [`Viewport::graph_to_screen`].
use super::*;
use crate::draw::{Align, DrawList, Layer, Prim};
use crate::schema::NodeSchema;
use crate::text::ellipsize;
use crate::theme::{Col, NodeState};
use crate::widget::widget_visible;
use compnode_core::wire::{control_points, flatten};
use compnode_core::{Color, Lod};

/// Move `c` toward white by `t` (used for header highlights and category dots).
fn lighten(c: Color, t: f32) -> Color {
    c.lerp(Color::WHITE, t)
}

impl Editor {
    /// Rebuild `dl` for the current state. Cost follows what is on screen.
    pub fn draw(&mut self, dl: &mut DrawList) {
        dl.clear();
        self.sync();
        let lod = self.view.lod();
        let mut vis = Visible::default();
        self.doc
            .graph
            .collect_visible(&self.view.visible_rect(DRAW_MARGIN_PX), &mut vis);
        self.draw_grid(dl);
        self.draw_wires(dl, &vis);
        self.draw_nodes(dl, &vis, lod);
        self.draw_overlays(dl);
    }

    pub fn node_state(&self, uid: u64) -> NodeState {
        if self.errors.contains_key(&uid) {
            NodeState::Error
        } else if self.selection.contains(uid) && self.selection.active == Some(uid) {
            NodeState::Active
        } else if self.selection.contains(uid) {
            NodeState::Selected
        } else if self.hover.node == Some(uid) {
            NodeState::Hovered
        } else if self.doc.data(uid).is_some_and(|d| d.muted) {
            NodeState::Muted
        } else {
            NodeState::Normal
        }
    }

    fn draw_grid(&self, dl: &mut DrawList) {
        let th = &self.theme;
        let zoom = self.view.zoom;
        let mut step = th.n(Num::GridStep).max(1.0);
        let mut guard = 0;
        while step * zoom < 8.0 && guard < 32 {
            step *= 2.0;
            guard += 1;
        }
        let fade = ((step * zoom - 8.0) / 16.0).clamp(0.0, 1.0);
        let r = self.view.visible_rect(DRAW_MARGIN_PX);
        let major_every = th.n(Num::GridMajorEvery).max(1.0) as i64;
        let lw = 1.0 / zoom;
        let (x0, x1) = (
            (r.min.x / step).floor() as i64,
            (r.max.x / step).ceil() as i64,
        );
        let (y0, y1) = (
            (r.min.y / step).floor() as i64,
            (r.max.y / step).ceil() as i64,
        );
        if x1 - x0 > 600 || y1 - y0 > 600 {
            return;
        }
        let color = |i: i64| {
            if i % major_every == 0 {
                th.c(Col::GridMajor)
            } else {
                let c = th.c(Col::GridMinor);
                c.with_alpha(c.a * fade)
            }
        };
        for i in x0..=x1 {
            let x = i as f32 * step;
            dl.line(
                Layer::Grid,
                vec2(x, r.min.y),
                vec2(x, r.max.y),
                color(i),
                lw,
            );
        }
        for i in y0..=y1 {
            let y = i as f32 * step;
            dl.line(
                Layer::Grid,
                vec2(r.min.x, y),
                vec2(r.max.x, y),
                color(i),
                lw,
            );
        }
    }

    fn draw_wires(&mut self, dl: &mut DrawList, vis: &Visible) {
        let tol = self.view.wire_tolerance();
        let width = self.theme.wire_width_graph(self.view.zoom);
        let px = 1.0 / self.view.zoom;
        let need_link = self.interaction != Interaction::Idle || self.hover.wire.is_some();
        self.wire_cache.begin_frame();
        for &w in &vis.wires {
            let Some((a, b)) = self.doc.graph.wire_endpoints(w) else {
                continue;
            };
            let Some(wire) = self.doc.graph.wire(w) else {
                continue;
            };
            let (from, to) = (wire.from, wire.to);
            let (Some(fnode), Some(tnode)) =
                (self.doc.graph.node(from.node), self.doc.graph.node(to.node))
            else {
                continue;
            };
            let ty = self
                .doc
                .registry
                .get(&fnode.data.schema)
                .and_then(|s| s.outputs.get(from.index))
                .map_or(SocketType::Any, |d| d.ty);
            let (fu, tu) = (fnode.data.uid, tnode.data.uid);
            let muted = fnode.data.muted || tnode.data.muted;
            let selected = self.selection.contains(fu) || self.selection.contains(tu);
            let mut color = self.theme.socket(ty);
            let mut wd = width;
            let link = if need_link {
                self.doc.link_of_wire(w)
            } else {
                None
            };
            let highlighted = selected || (link.is_some() && link == self.hover.wire);
            if highlighted {
                color = color.lerp(self.theme.c(Col::WireHighlight), 0.5);
                wd *= 1.25;
            }
            if muted {
                color = color.with_alpha(color.a * self.theme.n(Num::WireDimAlpha));
            }
            match &self.interaction {
                Interaction::DragWire(d) if d.replacing.is_some() && d.replacing == link => {
                    color = color.with_alpha(color.a * 0.3);
                }
                Interaction::CutLinks { hits, .. } if link.is_some_and(|l| hits.contains(&l)) => {
                    color = self.theme.c(Col::WireCut);
                }
                _ => {}
            }
            let points = self.wire_cache.get(w, a, b, tol).to_vec();
            // A soft dark underlay separates the noodle from the node bodies.
            // It stays under the colored wire; highlighted wires get a bright
            // halo instead, so the selected/muted assertions read the main
            // (last) LineStrip of each wire.
            let shadow = self.theme.c(Col::WireShadow);
            if shadow.a > 0.0 && !highlighted {
                dl.push(
                    Layer::Wires,
                    Prim::LineStrip {
                        points: points.clone(),
                        color: shadow.with_alpha(shadow.a * 0.35),
                        width: wd + 2.2 * px,
                    },
                );
            }
            if highlighted {
                dl.push(
                    Layer::Wires,
                    Prim::LineStrip {
                        points: points.clone(),
                        color,
                        width: wd * 2.0,
                    },
                );
            }
            dl.push(
                Layer::Wires,
                Prim::LineStrip {
                    points,
                    color,
                    width: wd,
                },
            );
        }
        self.wire_cache.end_frame();
    }

    fn draw_nodes(&self, dl: &mut DrawList, vis: &Visible, lod: Lod) {
        let mut order: Vec<(bool, bool, NodeId)> = vis
            .nodes
            .iter()
            .filter_map(|&id| {
                let n = self.doc.graph.node(id)?;
                Some((!n.data.is_frame(), self.selection.contains(n.data.uid), id))
            })
            .collect();
        order.sort();
        // Shadows first, so a node never casts one on top of another node.
        if lod != Lod::Dot {
            for &(_, _, id) in &order {
                self.draw_node_shadow(dl, id);
            }
        }
        for (_, _, id) in order {
            self.draw_node(dl, id, lod);
        }
    }

    fn draw_node_shadow(&self, dl: &mut DrawList, id: NodeId) {
        let Some(node) = self.doc.graph.node(id) else {
            return;
        };
        if node.data.is_frame() || node.data.is_reroute() {
            return;
        }
        let th = &self.theme;
        let shadow = th.c(Col::NodeShadow);
        if shadow.a <= 0.0 {
            return;
        }
        let px = 1.0 / self.view.zoom;
        let off = th.n(Num::ShadowOffset) * px;
        let spread = th.n(Num::ShadowSpread) * px;
        let radius = th.n(Num::Radius);
        let rect = node.rect();
        dl.rect(
            Layer::Nodes,
            rect.translate(vec2(0.0, off)).expand(spread * 2.0),
            radius + spread * 2.0,
            shadow.with_alpha(shadow.a * 0.13),
        );
        dl.rect(
            Layer::Nodes,
            rect.translate(vec2(0.0, off * 0.55)).expand(spread),
            radius + spread,
            shadow.with_alpha(shadow.a * 0.22),
        );
    }

    fn draw_state_outline(
        &self,
        dl: &mut DrawList,
        rect: Rect,
        radius: f32,
        state: NodeState,
        px: f32,
    ) {
        let Some((color, width)) = self.theme.state_outline(state) else {
            return;
        };
        let glow = self.theme.n(Num::GlowWidth) * px;
        if matches!(
            state,
            NodeState::Selected | NodeState::Active | NodeState::Error
        ) && glow > 0.0
        {
            let glow_c = if state == NodeState::Error {
                self.theme.c(Col::Error)
            } else {
                self.theme.c(Col::SelectionGlow)
            };
            dl.rect_stroked(
                Layer::Nodes,
                rect.expand(glow * 0.45),
                radius + glow * 0.45,
                Color::TRANSPARENT,
                glow_c.with_alpha(0.24),
                glow,
            );
        }
        dl.rect_stroked(
            Layer::Nodes,
            rect,
            radius,
            Color::TRANSPARENT,
            color,
            width * px,
        );
    }

    fn draw_node(&self, dl: &mut DrawList, id: NodeId, lod: Lod) {
        let Some(node) = self.doc.graph.node(id) else {
            return;
        };
        let Some(schema) = self.doc.registry.get(&node.data.schema) else {
            return;
        };
        let th = &self.theme;
        let uid = node.data.uid;
        let rect = node.rect();
        let px = 1.0 / self.view.zoom;
        let radius = th.n(Num::Radius);
        let state = self.node_state(uid);
        let fade = if node.data.muted {
            th.n(Num::MutedAlpha)
        } else {
            1.0
        };
        let fade_c = |c: Color| c.with_alpha(c.a * fade);
        let (stroke_c, stroke_w) = match th.state_outline(state) {
            Some((c, w)) => (c, w * px),
            None => (th.c(Col::NodeBorder), th.n(Num::BorderWidth) * px),
        };
        let title = node
            .data
            .title
            .clone()
            .unwrap_or_else(|| schema.title.clone());
        let size = th.n(Num::FontSize);

        if node.data.is_frame() {
            self.draw_frame(
                dl, schema, rect, radius, state, stroke_c, stroke_w, &title, size, lod,
            );
            return;
        }
        if node.data.is_reroute() {
            self.draw_reroute(dl, node, rect, state, px);
            return;
        }
        let header = fade_c(th.header(schema.category));
        if lod == Lod::Dot {
            dl.rect_stroked(Layer::Nodes, rect, radius, header, stroke_c, stroke_w);
            return;
        }
        // Body: rounded base, then a subtle vertical gradient inset so the
        // rounded corners stay clean.
        dl.rect_stroked(
            Layer::Nodes,
            rect,
            radius,
            fade_c(th.c(Col::NodeBody)),
            stroke_c,
            stroke_w,
        );
        let body = Rect::new(
            vec2(rect.min.x + radius, rect.min.y + HEADER_H),
            vec2(rect.max.x - radius, rect.max.y - radius),
        );
        if body.width() > 1.0 && body.height() > 1.0 {
            let top = fade_c(th.c(Col::NodeBodyTop));
            let bottom = fade_c(th.c(Col::NodeBodyBottom));
            dl.push(
                Layer::Nodes,
                Prim::GradientQuad {
                    rect: body,
                    colors: [top, top, bottom, bottom],
                },
            );
        }
        // Header: rounded top, square bottom, with a gentle top-down gradient.
        let head = Rect::new(rect.min, vec2(rect.max.x, rect.min.y + HEADER_H));
        dl.rect(Layer::Nodes, head, radius, header);
        let square = Rect::new(vec2(head.min.x, head.max.y - radius), head.max);
        dl.rect(Layer::Nodes, square, 0.0, header);
        let hg = Rect::new(
            vec2(head.min.x + radius, head.min.y + 1.0),
            vec2(head.max.x - radius, head.max.y - 1.0),
        );
        if hg.width() > 1.0 && hg.height() > 1.0 {
            let hi = fade_c(lighten(th.header(schema.category), 0.09));
            dl.push(
                Layer::Nodes,
                Prim::GradientQuad {
                    rect: hg,
                    colors: [hi, hi, header, header],
                },
            );
        }
        let accent_h = th.n(Num::HeaderAccentH).max(0.0);
        if accent_h > 0.0 {
            let accent = Rect::new(
                vec2(head.min.x + radius, head.max.y - accent_h),
                vec2(head.max.x - radius, head.max.y),
            );
            dl.rect(Layer::Nodes, accent, 0.0, fade_c(th.c(Col::HeaderAccent)));
        }
        if lod == Lod::Full {
            // A small category dot makes node types readable at a glance.
            let dot = Rect::from_center_size(
                vec2(rect.min.x + 12.0, rect.min.y + HEADER_H * 0.5),
                vec2(7.0, 7.0),
            );
            dl.rect(
                Layer::Nodes,
                dot,
                3.5,
                fade_c(lighten(th.header(schema.category), 0.38)),
            );
            let t = ellipsize(&title, node.width() - 32.0, size);
            let tc = if node.data.muted {
                th.c(Col::TextDim)
            } else {
                th.c(Col::HeaderText)
            };
            dl.text(
                Layer::Nodes,
                vec2(rect.min.x + 22.0, rect.min.y + HEADER_H * 0.5),
                &t,
                size,
                tc,
                Align::Left,
            );
            if self.errors.contains_key(&uid) {
                let c = vec2(rect.max.x - 13.0, rect.min.y + HEADER_H * 0.5);
                dl.rect(
                    Layer::Nodes,
                    Rect::from_center_size(c, vec2(14.0, 14.0)),
                    7.0,
                    th.c(Col::Error),
                );
                dl.text(Layer::Nodes, c, "!", 11.0, Color::WHITE, Align::Center);
            } else if node.data.muted {
                let c = vec2(rect.max.x - 13.0, rect.min.y + HEADER_H * 0.5);
                dl.line(
                    Layer::Nodes,
                    c + vec2(-4.0, -4.0),
                    c + vec2(4.0, 4.0),
                    th.c(Col::TextDim),
                    1.5 * px,
                );
            }
        }
        if !node.data.collapsed {
            if lod == Lod::Full {
                self.draw_row_hover(dl, node, schema, uid);
                self.draw_rows(dl, node, schema, uid);
            }
            self.draw_pins(dl, node, schema, uid);
        }
        self.draw_state_outline(dl, rect, radius, state, px);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_frame(
        &self,
        dl: &mut DrawList,
        schema: &NodeSchema,
        rect: Rect,
        radius: f32,
        state: NodeState,
        stroke_c: Color,
        stroke_w: f32,
        title: &str,
        size: f32,
        lod: Lod,
    ) {
        let th = &self.theme;
        let fill = th.c(Col::FrameFill);
        let border = if state == NodeState::Normal {
            th.c(Col::FrameBorder)
        } else {
            stroke_c
        };
        dl.rect_stroked(Layer::Nodes, rect, radius, fill, border, stroke_w);
        // A colored bar along the top identifies the frame at a glance.
        let bar = Rect::new(rect.min, vec2(rect.max.x, rect.min.y + 3.0));
        dl.rect(
            Layer::Nodes,
            bar,
            radius,
            th.header(schema.category).with_alpha(0.85),
        );
        if lod != Lod::Dot {
            dl.text(
                Layer::Nodes,
                vec2(rect.min.x + 10.0, rect.min.y + HEADER_H * 0.5),
                title,
                size,
                th.c(Col::TextDim),
                Align::Left,
            );
        }
        // Resize grip in the bottom-right corner when the frame is interactive.
        if lod != Lod::Dot
            && matches!(
                state,
                NodeState::Hovered | NodeState::Selected | NodeState::Active
            )
        {
            let c = th.c(Col::TextDim);
            let px = 1.0 / self.view.zoom;
            for k in 0..3 {
                let o = 4.0 + k as f32 * 4.0;
                dl.line(
                    Layer::Nodes,
                    vec2(rect.max.x - o, rect.max.y - 4.0),
                    vec2(rect.max.x - 4.0, rect.max.y - o),
                    c,
                    1.0 * px,
                );
            }
        }
    }

    fn draw_reroute(
        &self,
        dl: &mut DrawList,
        node: &Node<NodeData>,
        rect: Rect,
        state: NodeState,
        px: f32,
    ) {
        let th = &self.theme;
        let c = vec2(rect.min.x + node.width() * 0.5, node.in_pos(0).y);
        let mut r = 6.0;
        let mut ring = th.c(Col::PinRing);
        let mut rw = th.n(Num::PinRingWidth) * px;
        if matches!(
            state,
            NodeState::Hovered | NodeState::Selected | NodeState::Active
        ) {
            r = 7.5;
            ring = if state == NodeState::Hovered {
                th.c(Col::Hover)
            } else {
                th.c(Col::Active)
            };
            rw = 2.0 * px;
        }
        dl.rect_stroked(
            Layer::Nodes,
            Rect::from_center_size(c, vec2(2.0 * r, 2.0 * r)),
            r,
            th.socket(SocketType::Any),
            ring,
            rw,
        );
    }

    fn draw_row_hover(
        &self,
        dl: &mut DrawList,
        node: &Node<NodeData>,
        schema: &NodeSchema,
        uid: u64,
    ) {
        if self.hover.node != Some(uid) {
            return;
        }
        let gp = self.view.to_graph(self.pointer);
        let rows = schema.inputs.len().max(schema.outputs.len());
        for i in 0..rows {
            let Some(r) = self.slot_rect(uid, i) else {
                continue;
            };
            if gp.y >= r.min.y - 2.0 && gp.y <= r.max.y + 2.0 {
                let hl = Rect::new(
                    vec2(node.pos().x + 3.0, r.min.y - 1.0),
                    vec2(node.pos().x + node.width() - 3.0, r.max.y + 1.0),
                );
                dl.rect(Layer::Nodes, hl, 3.0, self.theme.c(Col::RowHover));
                break;
            }
        }
    }

    fn draw_pins(&self, dl: &mut DrawList, node: &Node<NodeData>, schema: &NodeSchema, uid: u64) {
        let th = &self.theme;
        let base_r = th.n(Num::PinRadius);
        let px = 1.0 / self.view.zoom;
        let drag_target = match &self.interaction {
            Interaction::DragWire(d) => d.target,
            _ => None,
        };
        let pin = |dl: &mut DrawList,
                   pos: Vec2,
                   ty: SocketType,
                   idx: usize,
                   is_out: bool,
                   connected: bool| {
            let mut r = base_r;
            let mut ring = th.c(Col::PinRing);
            let mut rw = th.n(Num::PinRingWidth) * px;
            let mut hot = false;
            if self.hover.pin == Some((uid, idx, is_out)) {
                r *= 1.35;
                ring = th.c(Col::WireHighlight);
                rw = 1.5 * px;
                hot = true;
            }
            if let Some((tu, ti, fb)) = drag_target {
                // the target pin of the dragged wire is on the opposite side of its start
                let start_is_out = matches!(&self.interaction, Interaction::DragWire(d) if matches!(d.start, WireStart::Out(..)));
                if tu == uid && ti == idx && start_is_out != is_out {
                    r *= 1.5;
                    ring = if fb == Feedback::Ok {
                        th.c(Col::Active)
                    } else {
                        th.c(Col::WireInvalid)
                    };
                    rw = 2.0 * px;
                    hot = true;
                }
            }
            if hot {
                dl.rect_stroked(
                    Layer::Nodes,
                    Rect::from_center_size(pos, vec2(2.0 * r + 5.0 * px, 2.0 * r + 5.0 * px)),
                    r + 2.5 * px,
                    Color::TRANSPARENT,
                    th.c(Col::WireHighlight).with_alpha(0.18),
                    3.0 * px,
                );
            }
            let fill = th.socket(ty).with_alpha(if connected { 1.0 } else { 0.82 });
            dl.rect_stroked(
                Layer::Nodes,
                Rect::from_center_size(pos, vec2(2.0 * r, 2.0 * r)),
                r,
                fill,
                ring,
                rw,
            );
        };
        for (i, d) in schema.inputs.iter().enumerate() {
            pin(
                dl,
                node.in_pos(i),
                d.ty,
                i,
                false,
                node.input_wire(i).is_some(),
            );
        }
        for (i, d) in schema.outputs.iter().enumerate() {
            pin(
                dl,
                node.out_pos(i),
                d.ty,
                i,
                true,
                !node.output_wires(i).is_empty(),
            );
        }
    }

    fn draw_rows(&self, dl: &mut DrawList, node: &Node<NodeData>, schema: &NodeSchema, uid: u64) {
        let th = &self.theme;
        let size = th.n(Num::FontSize);
        let w = node.width();
        let label_w = (w - 20.0) * 0.5;
        for i in 0..schema.inputs.len().max(schema.outputs.len()) {
            let Some(rect) = self.slot_rect(uid, i) else {
                continue;
            };
            let cy = node.in_pos(i).y;
            if let Some(def) = schema.inputs.get(i) {
                let shown = def.widget != WidgetKind::None
                    && widget_visible(node.input_wire(i).is_some(), Lod::Full);
                match self.widgets.get(&(uid, i, false)).filter(|_| shown) {
                    Some(wd) => wd.draw(dl, rect, &node.data.values[i], &def.name, th),
                    None => {
                        let t = ellipsize(&def.name, label_w, size);
                        dl.text(
                            Layer::Nodes,
                            vec2(node.pos().x + 10.0, cy),
                            &t,
                            size,
                            th.c(Col::Text),
                            Align::Left,
                        );
                    }
                }
            }
            if let Some(def) = schema.outputs.get(i) {
                match self
                    .widgets
                    .get(&(uid, i, true))
                    .filter(|_| def.widget != WidgetKind::None)
                {
                    Some(wd) => wd.draw(dl, rect, &node.data.out_values[i], &def.name, th),
                    None => {
                        let t = ellipsize(&def.name, label_w, size);
                        dl.text(
                            Layer::Nodes,
                            vec2(node.pos().x + w - 10.0, cy),
                            &t,
                            size,
                            th.c(Col::Text),
                            Align::Right,
                        );
                    }
                }
            }
        }
        for (k, def) in schema.footer.iter().enumerate() {
            let slot = schema.outputs.len() + k;
            let Some(rect) = self.widget_rect(uid, slot, true) else {
                continue;
            };
            if let Some(wd) = self.widgets.get(&(uid, slot, true)) {
                wd.draw(dl, rect, &node.data.out_values[slot], &def.name, th);
            }
        }
    }

    fn link_endpoints(&self, link: Link) -> Option<(Vec2, Vec2)> {
        let f = self.doc.graph.node(self.doc.id_of(link.from.0)?)?;
        let t = self.doc.graph.node(self.doc.id_of(link.to.0)?)?;
        Some((f.out_pos(link.from.1), t.in_pos(link.to.1)))
    }

    fn polyline(&self, a: Vec2, b: Vec2) -> Vec<Vec2> {
        let mut pts = Vec::new();
        flatten(&control_points(a, b), self.view.wire_tolerance(), &mut pts);
        pts
    }

    fn draw_overlays(&self, dl: &mut DrawList) {
        let th = &self.theme;
        let px = 1.0 / self.view.zoom;
        for g in &self.guides {
            let (a, b) = if g.vertical {
                (vec2(g.pos, g.from), vec2(g.pos, g.to))
            } else {
                (vec2(g.from, g.pos), vec2(g.to, g.pos))
            };
            dl.line(Layer::Overlay, a, b, th.c(Col::Guide), px);
        }
        match &self.interaction {
            Interaction::BoxSelect { start, cur, .. } => {
                let r = Rect::new(start.min(*cur), start.max(*cur));
                dl.rect_stroked(
                    Layer::Overlay,
                    r,
                    0.0,
                    th.c(Col::BoxFill),
                    th.c(Col::BoxBorder),
                    px,
                );
            }
            Interaction::CutLinks { points, .. } if points.len() >= 2 => {
                dl.push(
                    Layer::Overlay,
                    Prim::LineStrip {
                        points: points.clone(),
                        color: th.c(Col::WireCut),
                        width: 1.5 * px,
                    },
                );
            }
            Interaction::DragWire(d) => {
                let node_pin = |uid: u64, idx: usize, out: bool| -> Option<Vec2> {
                    let n = self.doc.graph.node(self.doc.id_of(uid)?)?;
                    Some(if out { n.out_pos(idx) } else { n.in_pos(idx) })
                };
                let (a, b, ty) = match d.start {
                    WireStart::Out(u, i) => {
                        let a = node_pin(u, i, true).unwrap_or(d.cursor);
                        let b = d
                            .target
                            .and_then(|t| node_pin(t.0, t.1, false))
                            .unwrap_or(d.cursor);
                        (a, b, self.out_type(u, i).unwrap_or(SocketType::Any))
                    }
                    WireStart::In(u, i) => {
                        let b = node_pin(u, i, false).unwrap_or(d.cursor);
                        let a = d
                            .target
                            .and_then(|t| node_pin(t.0, t.1, true))
                            .unwrap_or(d.cursor);
                        (a, b, self.in_type(u, i).unwrap_or(SocketType::Any))
                    }
                };
                let color = match d.target {
                    Some((_, _, fb)) if fb != Feedback::Ok => th.c(Col::WireInvalid),
                    _ => th.socket(ty),
                };
                dl.push(
                    Layer::Overlay,
                    Prim::LineStrip {
                        points: self.polyline(a, b),
                        color,
                        width: th.wire_width_graph(self.view.zoom),
                    },
                );
            }
            Interaction::DragNodes(d) => {
                if let Some((a, b)) = d.insert.and_then(|l| self.link_endpoints(l)) {
                    dl.push(
                        Layer::Overlay,
                        Prim::LineStrip {
                            points: self.polyline(a, b),
                            color: th.c(Col::WireInsert),
                            width: th.wire_width_graph(self.view.zoom) * 2.0,
                        },
                    );
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::editor;
    use super::*;
    use crate::input::InputEvent;
    use crate::theme::Theme;

    fn count(dl: &DrawList, f: impl Fn(&Prim) -> bool) -> usize {
        dl.iter().filter(|(_, p)| f(p)).count()
    }
    fn texts(dl: &DrawList) -> Vec<String> {
        dl.iter()
            .filter_map(|(_, p)| {
                if let Prim::Text { text, .. } = p {
                    Some(text.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn draws_grid_wires_nodes_in_layer_order() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(40.0, 40.0)).unwrap();
        let f = e.add_node("t.filter", vec2(400.0, 40.0)).unwrap();
        e.connect((a, 0), (f, 0));
        e.selection.clear();
        let mut dl = DrawList::new();
        e.draw(&mut dl);
        assert!(!dl.layer(Layer::Grid).is_empty());
        // A plain wire now has a shadow underlay plus the noodle itself.
        let wires: Vec<_> = dl
            .layer(Layer::Wires)
            .iter()
            .filter(|p| matches!(p, Prim::LineStrip { .. }))
            .collect();
        assert_eq!(wires.len(), 2);
        assert!(wires.windows(2).all(|w| {
            let (a, b) = (w[0], w[1]);
            matches!((a, b), (Prim::LineStrip { width: wa, .. }, Prim::LineStrip { width: wb, .. }) if wa > wb)
        }));
        assert!(!dl.layer(Layer::Nodes).is_empty());
        let layers: Vec<Layer> = dl.iter().map(|(l, _)| l).collect();
        assert!(layers.windows(2).all(|w| w[0] <= w[1]));
        let t = texts(&dl);
        assert!(t.contains(&"Source".to_string()) && t.contains(&"Filter".to_string()));
    }

    #[test]
    fn lod_drops_text_then_widgets() {
        let mut e = editor();
        e.add_node("t.filter", vec2(40.0, 40.0));
        let mut full = DrawList::new();
        e.draw(&mut full);
        e.view.zoom = 0.4; // Compact
        let mut compact = DrawList::new();
        e.draw(&mut compact);
        e.view.zoom = 0.1; // Dot
        let mut dot = DrawList::new();
        e.draw(&mut dot);
        assert!(!texts(&full).is_empty());
        assert!(texts(&compact).is_empty());
        assert!(texts(&dot).is_empty());
        let nodes = |d: &DrawList| d.layer(Layer::Nodes).len();
        assert!(nodes(&full) > nodes(&compact) && nodes(&compact) > nodes(&dot));
        assert_eq!(nodes(&dot), 1);
    }

    #[test]
    fn selection_changes_outline_and_theme_changes_colors() {
        let mut e = editor();
        let a = e.add_node("t.sink", vec2(40.0, 40.0)).unwrap();
        let outline = |e: &mut Editor| {
            let mut dl = DrawList::new();
            e.draw(&mut dl);
            dl.layer(Layer::Nodes).iter().find_map(|p| {
                if let Prim::RoundedRect {
                    stroke: Some((c, _)),
                    radius,
                    ..
                } = p
                {
                    (*radius > 5.9).then_some(*c)
                } else {
                    None
                }
            })
        };
        let selected = outline(&mut e).unwrap();
        assert_eq!(selected, e.theme.c(Col::Active));
        e.selection.clear();
        let normal = outline(&mut e).unwrap();
        assert_eq!(normal, e.theme.c(Col::NodeBorder));
        assert_ne!(selected, normal);
        e.set_node_error(a, Some("boom".into()));
        assert_eq!(outline(&mut e).unwrap(), e.theme.c(Col::Error));
        let mut t = Theme::light();
        t.set_c(Col::NodeBorder, Color::from_hex("#00ff00").unwrap());
        e.set_node_error(a, None);
        e.set_theme(t);
        assert_eq!(
            outline(&mut e).unwrap(),
            Color::from_hex("#00ff00").unwrap()
        );
    }

    #[test]
    fn overlays_appear_during_interactions() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(40.0, 40.0)).unwrap();
        e.selection.clear();
        // box select rectangle
        e.handle_event(&InputEvent::down(vec2(600.0, 400.0)));
        e.handle_event(&InputEvent::mv(vec2(700.0, 500.0)));
        let mut dl = DrawList::new();
        e.draw(&mut dl);
        assert_eq!(dl.layer(Layer::Overlay).len(), 1);
        e.handle_event(&InputEvent::up(vec2(700.0, 500.0)));
        // wire being dragged
        let pin = e
            .doc
            .graph
            .node(e.doc.id_of(a).unwrap())
            .unwrap()
            .out_pos(0);
        e.handle_event(&InputEvent::down(pin));
        e.handle_event(&InputEvent::mv(pin + vec2(150.0, 90.0)));
        e.draw(&mut dl);
        assert!(count(&dl, |p| matches!(p, Prim::LineStrip { .. })) >= 1);
        assert_eq!(dl.layer(Layer::Overlay).len(), 1);
        e.handle_event(&InputEvent::up(pin + vec2(150.0, 90.0)));
        e.draw(&mut dl);
        assert!(dl.layer(Layer::Overlay).is_empty());
    }

    #[test]
    fn muted_and_selected_wires_are_styled() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(40.0, 40.0)).unwrap();
        let f = e.add_node("t.filter", vec2(400.0, 40.0)).unwrap();
        e.connect((a, 0), (f, 0));
        let color = |e: &mut Editor| {
            let mut dl = DrawList::new();
            e.draw(&mut dl);
            // The last LineStrip of the (single) wire is the noodle itself;
            // earlier ones are the shadow underlay or the highlight halo.
            dl.layer(Layer::Wires)
                .iter()
                .rev()
                .find_map(|p| match p {
                    Prim::LineStrip { color, width, .. } => Some((*color, *width)),
                    _ => None,
                })
                .expect("wire primitive")
        };
        e.selection.clear();
        let (plain_c, plain_w) = color(&mut e);
        e.selection.set_only(a);
        let (sel_c, sel_w) = color(&mut e);
        assert!(sel_w > plain_w);
        assert_ne!(sel_c, plain_c);
        e.selection.clear();
        e.selection.set_only(f);
        e.toggle_mute();
        e.selection.clear();
        let (muted_c, _) = color(&mut e);
        assert!(muted_c.a < plain_c.a);
    }

    #[test]
    fn output_widgets_draw_host_images_and_grow_the_row() {
        use crate::schema::{Category, NodeSchema, SocketDef};
        let mut reg = crate::document::test_support::registry();
        reg.register(
            NodeSchema::new("t.mode", "Mode", Category::Filter)
                .input(SocketDef::new("Mode", SocketType::Enum).options(&["A", "B"]))
                .output(SocketDef::image("Image").widget(WidgetKind::ImagePreview)),
        );
        let mut e = Editor::new(reg);
        let m = e.add_node("t.mode", vec2(40.0, 40.0)).unwrap();
        e.set_preview(m, 0, Some(ImageHandle(9)), 0.5);
        let mut dl = DrawList::new();
        e.draw(&mut dl);
        assert_eq!(
            count(&dl, |p| matches!(p, Prim::Image { texture: 9, .. })),
            1
        );
        // preview row is taller than a plain row
        let n = e.doc.graph.node(e.doc.id_of(m).unwrap()).unwrap();
        assert!(n.height() > 100.0);
        // sharing a row between an input widget and an output widget is flagged
        assert!(e.doc.schema_of(m).unwrap().validate().is_err());
    }

    #[test]
    fn draw_key_changes_with_state_not_with_idle_redraws() {
        let mut e = editor();
        e.add_node("t.src", vec2(0.0, 0.0));
        let k = e.draw_key();
        let mut dl = DrawList::new();
        e.draw(&mut dl);
        assert_eq!(e.draw_key(), k);
        e.handle_event(&InputEvent::mv(vec2(900.0, 600.0)));
        assert_eq!(e.draw_key(), k); // hovering nothing changes nothing
        e.handle_event(&InputEvent::mv(vec2(60.0, 10.0)));
        assert_ne!(e.draw_key(), k); // hovering a node does
    }

    #[test]
    fn footer_rows_host_previews_without_pins() {
        use crate::schema::{Category, NodeSchema, SocketDef};
        let mut reg = crate::document::test_support::registry();
        reg.register(
            NodeSchema::new("t.viewer", "Viewer", Category::Output)
                .input(SocketDef::image("Image"))
                .footer(SocketDef::image("Preview").widget(WidgetKind::ImagePreview)),
        );
        assert!(reg.get("t.viewer").unwrap().validate().is_ok());
        let mut e = Editor::new(reg);
        let v = e.add_node("t.viewer", vec2(40.0, 40.0)).unwrap();
        let before = e.doc.graph.node(e.doc.id_of(v).unwrap()).unwrap().height();
        // a footer preview is taller than a plain row (aspect 0.5)
        e.set_preview(v, 0, Some(ImageHandle(5)), 0.5);
        let node = e.doc.graph.node(e.doc.id_of(v).unwrap()).unwrap();
        // the preview area reserves space for the image (aspect-fit)
        let h = node.height();
        assert!(h > 100.0, "height {h} before {before}");
        assert_eq!(node.outputs(), 0);
        let mut dl = DrawList::new();
        e.draw(&mut dl);
        assert_eq!(
            count(&dl, |p| matches!(p, Prim::Image { texture: 5, .. })),
            1
        );
        let rect = e.widget_rect(v, 0, true).unwrap();
        assert_eq!(e.widget_at(rect.center()), Some((v, 0, true)));
    }
}
