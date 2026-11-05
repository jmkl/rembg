//! Event handling: the interaction state machine.
use super::*;
use crate::command::Command;
use crate::input::{InputEvent, Key, Modifiers, PointerButton};
use compnode_core::Visible;

const OFFSCREEN: Vec2 = Vec2 {
    x: -1.0e9,
    y: -1.0e9,
};
/// Pointer travel (screen px) before a press becomes a drag.
const DRAG_START_PX: f32 = 3.0;

fn snap_to(v: f32, grid: f32) -> f32 {
    (v / grid).round() * grid
}

fn ccw(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

/// Do segments `ab` and `cd` cross (proper crossings only)?
pub fn segments_cross(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    ccw(a, b, c) * ccw(a, b, d) < 0.0 && ccw(c, d, a) * ccw(c, d, b) < 0.0
}

impl Editor {
    #[allow(dead_code)]
    fn pin_radius_graph(&self) -> f32 {
        self.snap.pin_radius / self.view.zoom
    }

    fn to_graph_event(&self, ev: &InputEvent) -> InputEvent {
        match ev.pos() {
            Some(p) => ev.with_pos(self.view.to_graph(p)),
            None => ev.clone(),
        }
    }

    /// Feed one input event (pointer positions in screen space).
    pub fn handle_event(&mut self, ev: &InputEvent) {
        if !matches!(ev, InputEvent::Text(_)) {
            self.mods = ev.mods();
        }
        if let Some(p) = ev.pos() {
            self.pointer = p;
        }
        if self.pending_text.is_some() {
            return; // the host's text field owns the keyboard
        }
        let was_idle_move =
            matches!(ev, InputEvent::PointerMove { .. }) && self.interaction == Interaction::Idle;
        let hover_before = (self.hover.clone(), self.hover_widget);
        let st = std::mem::replace(&mut self.interaction, Interaction::Idle);
        let next = self.step(st, ev);
        self.interaction = next;
        self.sync();
        if !was_idle_move || hover_before != (self.hover.clone(), self.hover_widget) {
            self.ui_rev += 1;
        }
    }

    fn step(&mut self, st: Interaction, ev: &InputEvent) -> Interaction {
        match st {
            Interaction::Idle => self.idle_event(ev),
            Interaction::Pan { last } => match ev {
                InputEvent::PointerMove { pos, .. } => {
                    self.view.pan(*pos - last);
                    Interaction::Pan { last: *pos }
                }
                InputEvent::PointerUp { .. } => Interaction::Idle,
                _ => Interaction::Pan { last },
            },
            Interaction::DragNodes(d) => self.drag_nodes_event(d, ev),
            Interaction::BoxSelect { start, cur, mode } => {
                self.box_select_event(start, cur, mode, ev)
            }
            Interaction::DragWire(d) => self.drag_wire_event(d, ev),
            Interaction::CutLinks { points, hits } => self.cut_event(points, hits, ev),
            Interaction::ContextClick { start, graph_pos } => match ev {
                InputEvent::PointerUp {
                    pos,
                    button: PointerButton::Secondary,
                    ..
                } => {
                    if pos.distance(start) < 4.0 {
                        let gp = self.view.to_graph(*pos);
                        let ctx = self.node_at(gp).or(self.hover.node);
                        self.actions.push(EditorAction::OpenContextMenu {
                            graph_pos,
                            context: ctx,
                        });
                    }
                    Interaction::Idle
                }
                _ => Interaction::ContextClick { start, graph_pos },
            },
            Interaction::EditWidget { uid, slot, output } => {
                self.edit_widget_event(uid, slot, output, ev)
            }
            Interaction::Resize {
                uid,
                kind,
                anchor,
                old,
            } => self.resize_event(uid, kind, anchor, old, ev),
        }
    }

    // ───────────── idle ─────────────

    fn idle_event(&mut self, ev: &InputEvent) -> Interaction {
        match ev {
            InputEvent::PointerMove { pos, .. } => {
                self.update_hover(*pos);
                Interaction::Idle
            }
            InputEvent::PointerDown { pos, button, mods } => {
                self.pointer_down(ev, *pos, *button, *mods)
            }
            InputEvent::Scroll { pos, delta, .. } => {
                let f = ZOOM_STEP.powf(delta.y.clamp(-10.0, 10.0));
                self.view.zoom_at(*pos, f);
                Interaction::Idle
            }
            InputEvent::Key { key, mods } => {
                if let Some(next) = self.focus_key(*key, *mods) {
                    return next;
                }
                self.handle_key(*key, *mods);
                Interaction::Idle
            }
            _ => Interaction::Idle,
        }
    }

    fn update_hover(&mut self, pos: Vec2) {
        let gp = self.view.to_graph(pos);
        let widget = self.widget_at(gp);
        if widget != self.hover_widget {
            if let Some((u, s, o)) = self.hover_widget {
                let away = InputEvent::mv(OFFSCREEN);
                self.widget_event(u, s, o, &away);
            }
            self.hover_widget = widget;
        }
        if let Some((u, s, o)) = widget {
            let ev = InputEvent::PointerMove {
                pos: gp,
                mods: self.mods,
            };
            self.widget_event(u, s, o, &ev);
        }
        let node = self.node_at(gp);
        let pin = self.pin_at(gp);
        let wire = if node.is_none() && pin.is_none() {
            let tol = self.view.wire_tolerance();
            let r = 8.0 / self.view.zoom;
            self.doc
                .graph
                .hit_wire(gp, r, tol, &mut self.wire_cache)
                .and_then(|w| self.doc.link_of_wire(w))
        } else {
            None
        };
        self.hover = Hover { node, pin, wire };
    }

    fn pointer_down(
        &mut self,
        ev: &InputEvent,
        pos: Vec2,
        button: PointerButton,
        mods: Modifiers,
    ) -> Interaction {
        let gp = self.view.to_graph(pos);
        match button {
            PointerButton::Middle => return Interaction::Pan { last: pos },
            PointerButton::Secondary => {
                return if mods.ctrl {
                    Interaction::CutLinks {
                        points: vec![gp],
                        hits: Vec::new(),
                    }
                } else {
                    Interaction::ContextClick {
                        start: pos,
                        graph_pos: gp,
                    }
                };
            }
            PointerButton::Primary => {}
        }
        if mods.alt {
            return Interaction::Pan { last: pos };
        }
        if let Some((uid, idx, is_out)) = self.pin_at(gp) {
            return self.start_wire_drag(uid, idx, is_out, gp);
        }
        if let Some((uid, slot, output)) = self.widget_at(gp) {
            let gev = self.to_graph_event(ev);
            if let Some(resp) = self.widget_event(uid, slot, output, &gev)
                && resp.consumed
            {
                return if self.widget_captures(uid, slot, output) || self.pending_text.is_some() {
                    Interaction::EditWidget { uid, slot, output }
                } else {
                    Interaction::Idle
                };
            }
        }
        if let Some((uid, kind)) = self.resize_at(gp) {
            let old = self.doc.size(uid).unwrap_or((180.0, 200.0));
            return Interaction::Resize {
                uid,
                kind,
                anchor: gp,
                old,
            };
        }
        if let Some(uid) = self.node_at(gp) {
            return self.start_node_drag(uid, gp, mods);
        }
        let mode = if mods.shift {
            SelectMode::Add
        } else if mods.ctrl {
            SelectMode::Subtract
        } else {
            SelectMode::Replace
        };
        Interaction::BoxSelect {
            start: gp,
            cur: gp,
            mode,
        }
    }

    fn start_wire_drag(&mut self, uid: u64, idx: usize, is_out: bool, gp: Vec2) -> Interaction {
        if is_out {
            return Interaction::DragWire(WireDrag {
                start: WireStart::Out(uid, idx),
                cursor: gp,
                target: None,
                replacing: None,
            });
        }
        // On a multi-input socket, pick up the most recently connected wire.
        match self.doc.input_links(uid, idx).last().copied() {
            Some(link) => Interaction::DragWire(WireDrag {
                start: WireStart::Out(link.from.0, link.from.1),
                cursor: gp,
                target: None,
                replacing: Some(link),
            }),
            None => Interaction::DragWire(WireDrag {
                start: WireStart::In(uid, idx),
                cursor: gp,
                target: None,
                replacing: None,
            }),
        }
    }

    fn start_node_drag(&mut self, uid: u64, gp: Vec2, mods: Modifiers) -> Interaction {
        let was_selected = self.selection.contains(uid);
        if mods.shift {
            self.selection.toggle(uid);
            if was_selected {
                return Interaction::Idle;
            }
        } else if !was_selected {
            self.selection.set_only(uid);
        }
        self.selection.active = Some(uid);
        let mut uids = self.selection.to_vec();
        // a frame carries the nodes that lie fully inside it
        let mut extra: Vec<u64> = Vec::new();
        for &f in &uids {
            let Some(n) = self.doc.id_of(f).and_then(|id| self.doc.graph.node(id)) else {
                continue;
            };
            if !n.data.is_frame() {
                continue;
            }
            let fr = n.rect();
            let mut vis = Visible::default();
            self.doc.graph.collect_visible(&fr, &mut vis);
            for id in vis.nodes {
                let Some(m) = self.doc.graph.node(id) else {
                    continue;
                };
                let u = m.data.uid;
                if u != f
                    && !self.selection.contains(u)
                    && fr.contains_rect(&m.rect())
                    && !extra.contains(&u)
                {
                    extra.push(u);
                }
            }
        }
        uids.extend(extra);
        let orig: Vec<(u64, Vec2)> = uids
            .iter()
            .filter_map(|u| self.doc.pos(*u).map(|p| (*u, p)))
            .collect();
        Interaction::DragNodes(NodeDrag {
            anchor: gp,
            primary: uid,
            orig,
            moved: false,
            click_uid: (was_selected && !mods.shift).then_some(uid),
            insert: None,
        })
    }

    // ───────────── dragging nodes ─────────────

    fn drag_nodes_event(&mut self, mut d: NodeDrag, ev: &InputEvent) -> Interaction {
        match ev {
            InputEvent::PointerMove { pos, .. } => {
                let gp = self.view.to_graph(*pos);
                let mut delta = gp - d.anchor;
                if !d.moved {
                    if delta.length() * self.view.zoom < DRAG_START_PX {
                        return Interaction::DragNodes(d);
                    }
                    d.moved = true;
                }
                let prim0 = d
                    .orig
                    .iter()
                    .find(|(u, _)| *u == d.primary)
                    .map_or(Vec2::ZERO, |(_, p)| *p);
                let grid_on = self.snap.grid.is_some() != self.mods.ctrl;
                self.guides.clear();
                if grid_on {
                    let g = self
                        .snap
                        .grid
                        .unwrap_or(self.theme.n(Num::GridStep))
                        .max(1.0);
                    let t = prim0 + delta;
                    delta = vec2(snap_to(t.x, g), snap_to(t.y, g)) - prim0;
                } else if self.snap.align {
                    delta = self.align_snap(&d.orig, delta);
                }
                for (u, o) in &d.orig {
                    self.doc.set_pos(*u, *o + delta);
                }
                d.insert = self.find_insert_target(&d, gp);
                Interaction::DragNodes(d)
            }
            InputEvent::PointerUp {
                button: PointerButton::Primary,
                ..
            } => {
                self.guides.clear();
                if !d.moved {
                    if let Some(u) = d.click_uid {
                        self.selection.set_only(u);
                    }
                    return Interaction::Idle;
                }
                let moves: Vec<(u64, Vec2, Vec2)> = d
                    .orig
                    .iter()
                    .map(|(u, o)| (*u, *o, self.doc.pos(*u).unwrap_or(*o)))
                    .collect();
                self.history.begin();
                self.history.record(Command::MoveNodes(moves));
                if let Some(link) = d.insert {
                    self.insert_node_on_link(d.primary, link);
                }
                self.history.end();
                Interaction::Idle
            }
            InputEvent::Key {
                key: Key::Escape, ..
            } => {
                for (u, o) in &d.orig {
                    self.doc.set_pos(*u, *o);
                }
                self.guides.clear();
                Interaction::Idle
            }
            _ => Interaction::DragNodes(d),
        }
    }

    /// Snap the dragged group's edges/centers to nearby nodes; records guides.
    fn align_snap(&mut self, orig: &[(u64, Vec2)], delta: Vec2) -> Vec2 {
        let moving: Vec<u64> = orig.iter().map(|(u, _)| *u).collect();
        let Some(base) = self.bounds_at(orig) else {
            return delta;
        };
        let bb = base.translate(delta);
        let thr = self.snap.align_px / self.view.zoom;
        let mut vis = Visible::default();
        self.doc
            .graph
            .collect_visible(&bb.expand(thr * 4.0 + 200.0), &mut vis);
        // (distance to move, guide position, other rect)
        let mut best_x: Option<(f32, f32, Rect)> = None;
        let mut best_y: Option<(f32, f32, Rect)> = None;
        let mine_x = [bb.min.x, bb.center().x, bb.max.x];
        let mine_y = [bb.min.y, bb.center().y, bb.max.y];
        for id in vis.nodes {
            let Some(n) = self.doc.graph.node(id) else {
                continue;
            };
            if moving.contains(&n.data.uid) || n.data.is_frame() {
                continue;
            }
            let r = n.rect();
            for c in [r.min.x, r.center().x, r.max.x] {
                for m in mine_x {
                    let dx = c - m;
                    if dx.abs() <= thr && best_x.is_none_or(|(b, _, _)| dx.abs() < b.abs()) {
                        best_x = Some((dx, c, r));
                    }
                }
            }
            for c in [r.min.y, r.center().y, r.max.y] {
                for m in mine_y {
                    let dy = c - m;
                    if dy.abs() <= thr && best_y.is_none_or(|(b, _, _)| dy.abs() < b.abs()) {
                        best_y = Some((dy, c, r));
                    }
                }
            }
        }
        let mut out = delta;
        if let Some((dx, pos, r)) = best_x {
            out.x += dx;
            self.guides.push(Guide {
                vertical: true,
                pos,
                from: bb.min.y.min(r.min.y),
                to: bb.max.y.max(r.max.y),
            });
        }
        if let Some((dy, pos, r)) = best_y {
            out.y += dy;
            self.guides.push(Guide {
                vertical: false,
                pos,
                from: bb.min.x.min(r.min.x),
                to: bb.max.x.max(r.max.x),
            });
        }
        out
    }

    /// Bounding box of nodes placed at the given positions.
    fn bounds_at(&self, orig: &[(u64, Vec2)]) -> Option<Rect> {
        let mut out: Option<Rect> = None;
        for (u, p) in orig {
            let n = self.doc.graph.node(self.doc.id_of(*u)?)?;
            let r = Rect::from_min_size(*p, n.rect().size());
            out = Some(out.map_or(r, |o| o.union(&r)));
        }
        out
    }

    /// A lone, unconnected node dragged over a wire can be inserted into it.
    fn find_insert_target(&mut self, d: &NodeDrag, gp: Vec2) -> Option<Link> {
        if d.orig.len() != 1 || !self.doc.links_of(d.primary).is_empty() {
            return None;
        }
        let schema = self.doc.schema_of(d.primary)?;
        if schema.inputs.is_empty() || schema.outputs.is_empty() {
            return None;
        }
        let tol = self.view.wire_tolerance();
        let r = 10.0 / self.view.zoom;
        let w = self.doc.graph.hit_wire(gp, r, tol, &mut self.wire_cache)?;
        let link = self.doc.link_of_wire(w)?;
        self.insert_pair(d.primary, link).map(|_| link)
    }

    /// First input/output of `uid` that can sit between the ends of `link`.
    fn insert_pair(&self, uid: u64, link: Link) -> Option<(usize, usize)> {
        let schema = self.doc.schema_of(uid)?;
        let src = self.out_type(link.from.0, link.from.1)?;
        let dst = self.in_type(link.to.0, link.to.1)?;
        let i = schema
            .inputs
            .iter()
            .position(|d| SocketType::can_connect(src, d.ty))?;
        let o = schema
            .outputs
            .iter()
            .position(|d| SocketType::can_connect(d.ty, dst))?;
        Some((i, o))
    }

    fn insert_node_on_link(&mut self, uid: u64, link: Link) {
        let Some((i, o)) = self.insert_pair(uid, link) else {
            return;
        };
        let cmd = Command::SetLinks {
            remove: vec![link],
            add: vec![
                Link {
                    from: link.from,
                    to: (uid, i),
                },
                Link {
                    from: (uid, o),
                    to: link.to,
                },
            ],
        };
        self.history.execute(&mut self.doc, cmd);
    }

    // ───────────── box select ─────────────

    fn box_select_event(
        &mut self,
        start: Vec2,
        cur: Vec2,
        mode: SelectMode,
        ev: &InputEvent,
    ) -> Interaction {
        match ev {
            InputEvent::PointerMove { pos, .. } => Interaction::BoxSelect {
                start,
                cur: self.view.to_graph(*pos),
                mode,
            },
            InputEvent::PointerUp {
                pos,
                button: PointerButton::Primary,
                ..
            } => {
                let end = self.view.to_graph(*pos);
                let rect = Rect::new(start.min(end), start.max(end));
                if rect.size().length() * self.view.zoom < DRAG_START_PX {
                    if mode == SelectMode::Replace {
                        self.selection.clear();
                    }
                    return Interaction::Idle;
                }
                let mut vis = Visible::default();
                self.doc.graph.collect_visible(&rect, &mut vis);
                let mut hit: Vec<u64> = Vec::new();
                for id in vis.nodes {
                    let Some(n) = self.doc.graph.node(id) else {
                        continue;
                    };
                    if n.data.is_frame() && !rect.contains_rect(&n.rect()) {
                        continue;
                    }
                    hit.push(n.data.uid);
                }
                hit.sort_unstable();
                match mode {
                    SelectMode::Replace => self.selection.set_all(&hit),
                    SelectMode::Add => {
                        for u in hit {
                            self.selection.add(u);
                        }
                    }
                    SelectMode::Subtract => {
                        for u in hit {
                            self.selection.remove(u);
                        }
                    }
                }
                Interaction::Idle
            }
            InputEvent::Key {
                key: Key::Escape, ..
            } => Interaction::Idle,
            _ => Interaction::BoxSelect { start, cur, mode },
        }
    }

    // ───────────── dragging wires ─────────────

    fn drag_wire_event(&mut self, mut d: WireDrag, ev: &InputEvent) -> Interaction {
        match ev {
            InputEvent::PointerMove { pos, .. } => {
                let gp = self.view.to_graph(*pos);
                d.cursor = gp;
                d.target = self.wire_target(d.start, gp);
                Interaction::DragWire(d)
            }
            InputEvent::PointerUp {
                pos,
                button: PointerButton::Primary,
                ..
            } => {
                let gp = self.view.to_graph(*pos);
                d.cursor = gp;
                d.target = self.wire_target(d.start, gp);
                match d.target {
                    Some((tu, ti, Feedback::Ok)) => {
                        let link = match d.start {
                            WireStart::Out(u, i) => Link {
                                from: (u, i),
                                to: (tu, ti),
                            },
                            WireStart::In(u, i) => Link {
                                from: (tu, ti),
                                to: (u, i),
                            },
                        };
                        self.apply_link(link, d.replacing);
                    }
                    Some(_) => {} // not allowed: leave everything as it was
                    None => match (d.replacing, d.start) {
                        (Some(l), _) => self.execute(Command::SetLinks {
                            remove: vec![l],
                            add: vec![],
                        }),
                        (None, WireStart::Out(u, i)) => {
                            self.actions.push(EditorAction::OpenAddMenu {
                                graph_pos: gp,
                                connect: Some(MenuConnect::FromOut(u, i)),
                            })
                        }
                        (None, WireStart::In(u, i)) => {
                            self.actions.push(EditorAction::OpenAddMenu {
                                graph_pos: gp,
                                connect: Some(MenuConnect::ToIn(u, i)),
                            })
                        }
                    },
                }
                Interaction::Idle
            }
            InputEvent::Key {
                key: Key::Escape, ..
            } => Interaction::Idle,
            _ => Interaction::DragWire(d),
        }
    }

    /// Pin of the opposite kind under `gp`, with whether linking would work.
    fn wire_target(&self, start: WireStart, gp: Vec2) -> Option<(u64, usize, Feedback)> {
        let (uid, idx, is_out) = self.pin_at(gp)?;
        match (start, is_out) {
            (WireStart::Out(u, i), false) => Some((
                uid,
                idx,
                self.check_link(Link {
                    from: (u, i),
                    to: (uid, idx),
                }),
            )),
            (WireStart::In(u, i), true) => Some((
                uid,
                idx,
                self.check_link(Link {
                    from: (uid, idx),
                    to: (u, i),
                }),
            )),
            _ => None,
        }
    }

    // ───────────── cutting links ─────────────

    fn cut_event(
        &mut self,
        mut points: Vec<Vec2>,
        hits: Vec<Link>,
        ev: &InputEvent,
    ) -> Interaction {
        match ev {
            InputEvent::PointerMove { pos, .. } => {
                let gp = self.view.to_graph(*pos);
                if points
                    .last()
                    .is_none_or(|l| l.distance(gp) * self.view.zoom >= 2.0)
                {
                    points.push(gp);
                }
                let hits = self.links_crossing(&points);
                Interaction::CutLinks { points, hits }
            }
            InputEvent::PointerUp {
                button: PointerButton::Secondary,
                ..
            } => {
                if !hits.is_empty() {
                    self.execute(Command::SetLinks {
                        remove: hits,
                        add: vec![],
                    });
                }
                Interaction::Idle
            }
            InputEvent::Key {
                key: Key::Escape, ..
            } => Interaction::Idle,
            _ => Interaction::CutLinks { points, hits },
        }
    }

    /// Links whose wire crosses the polyline `pts`.
    pub(crate) fn links_crossing(&mut self, pts: &[Vec2]) -> Vec<Link> {
        if pts.len() < 2 {
            return Vec::new();
        }
        let bb = Rect::from_points(pts).expand(2.0);
        let mut vis = Visible::default();
        self.doc.graph.collect_visible(&bb, &mut vis);
        let tol = self.view.wire_tolerance();
        let mut out = Vec::new();
        for w in vis.wires {
            let Some((a, b)) = self.doc.graph.wire_endpoints(w) else {
                continue;
            };
            let poly = self.wire_cache.get(w, a, b, tol).to_vec();
            let crossed = poly.windows(2).any(|s| {
                pts.windows(2)
                    .any(|q| segments_cross(s[0], s[1], q[0], q[1]))
            });
            if crossed && let Some(l) = self.doc.link_of_wire(w) {
                out.push(l);
            }
        }
        out.sort_by_key(|l| (l.from, l.to));
        out
    }

    // ───────────── widgets and resizing ─────────────

    fn edit_widget_event(
        &mut self,
        uid: u64,
        slot: usize,
        output: bool,
        ev: &InputEvent,
    ) -> Interaction {
        let keep = Interaction::EditWidget { uid, slot, output };
        if !matches!(
            ev,
            InputEvent::PointerMove { .. }
                | InputEvent::PointerDown { .. }
                | InputEvent::PointerUp { .. }
                | InputEvent::Key { .. }
        ) {
            return keep;
        }
        let gev = self.to_graph_event(ev);
        match self.widget_event(uid, slot, output, &gev) {
            Some(_) if self.widget_captures(uid, slot, output) || self.pending_text.is_some() => {
                keep
            }
            _ => Interaction::Idle,
        }
    }

    fn resize_event(
        &mut self,
        uid: u64,
        kind: ResizeKind,
        anchor: Vec2,
        old: (f32, f32),
        ev: &InputEvent,
    ) -> Interaction {
        let back = Interaction::Resize {
            uid,
            kind,
            anchor,
            old,
        };
        match ev {
            InputEvent::PointerMove { pos, .. } => {
                let d = self.view.to_graph(*pos) - anchor;
                let min_w = 60.0;
                let w = (old.0 + d.x).max(min_w);
                let h = if kind == ResizeKind::Corner {
                    (old.1 + d.y).max(60.0)
                } else {
                    old.1
                };
                self.doc.set_size(uid, w, h);
                back
            }
            InputEvent::PointerUp {
                button: PointerButton::Primary,
                ..
            } => {
                if let Some(new) = self.doc.size(uid) {
                    self.history.record(Command::Resize { uid, old, new });
                }
                Interaction::Idle
            }
            InputEvent::Key {
                key: Key::Escape, ..
            } => {
                self.doc.set_size(uid, old.0, old.1);
                Interaction::Idle
            }
            _ => back,
        }
    }

    // ───────────── keyboard ─────────────

    fn handle_key(&mut self, key: Key, mods: Modifiers) {
        let gp = self.view.to_graph(self.pointer);
        match (key, mods.ctrl, mods.shift) {
            (Key::Z, true, false) => {
                self.undo();
            }
            (Key::Z, true, true) | (Key::Y, true, _) => {
                self.redo();
            }
            (Key::Delete, false, _) | (Key::X, false, _) => self.delete_selection(),
            (Key::C, true, _) => self.copy(),
            (Key::V, true, _) => {
                self.paste(gp);
            }
            (Key::D, true, _) => {
                self.duplicate();
            }
            // Blender-style: Shift+A opens the add-node menu at the cursor.
            (Key::A, false, true) => {
                self.actions.push(EditorAction::OpenAddMenu {
                    graph_pos: gp,
                    connect: None,
                });
            }
            (Key::A, false, false) => self.select_all(),
            (Key::I, true, _) => self.invert_selection(),
            (Key::M, false, _) => self.toggle_mute(),
            (Key::H, false, _) => self.toggle_collapse(),
            (Key::Home, _, _) => self.view_all(),
            (Key::Period, _, _) => self.view_selected(),
            (Key::F, false, _) => {
                self.frame_selected();
            }
            (Key::R, false, _) => {
                if let Some(link) = self.hover.wire {
                    self.insert_reroute(link, gp);
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
    use crate::input::Modifiers;
    use crate::value::Value;

    fn drag(e: &mut Editor, from: Vec2, to: Vec2) {
        e.handle_event(&InputEvent::mv(from));
        e.handle_event(&InputEvent::down(from));
        let mid = from.lerp(to, 0.5);
        e.handle_event(&InputEvent::mv(mid));
        e.handle_event(&InputEvent::mv(to));
        e.handle_event(&InputEvent::up(to));
    }

    fn out_pin(e: &Editor, uid: u64, i: usize) -> Vec2 {
        e.doc
            .graph
            .node(e.doc.id_of(uid).unwrap())
            .unwrap()
            .out_pos(i)
    }
    fn in_pin(e: &Editor, uid: u64, i: usize) -> Vec2 {
        e.doc
            .graph
            .node(e.doc.id_of(uid).unwrap())
            .unwrap()
            .in_pos(i)
    }
    fn header_of(e: &Editor, uid: u64) -> Vec2 {
        e.doc.pos(uid).unwrap() + vec2(60.0, 10.0)
    }

    #[test]
    fn segment_crossing() {
        assert!(segments_cross(
            vec2(0.0, 0.0),
            vec2(10.0, 10.0),
            vec2(0.0, 10.0),
            vec2(10.0, 0.0)
        ));
        assert!(!segments_cross(
            vec2(0.0, 0.0),
            vec2(1.0, 1.0),
            vec2(5.0, 0.0),
            vec2(6.0, 1.0)
        ));
    }

    #[test]
    fn click_selects_shift_toggles_and_empty_click_clears() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let b = e.add_node("t.sink", vec2(400.0, 0.0)).unwrap();
        e.selection.clear();
        e.handle_event(&InputEvent::down(header_of(&e, a)));
        e.handle_event(&InputEvent::up(header_of(&e, a)));
        assert_eq!(e.selection.to_vec(), vec![a]);
        let sh = |ev: InputEvent| ev.with_mods(Modifiers::SHIFT);
        e.handle_event(&sh(InputEvent::down(header_of(&e, b))));
        e.handle_event(&sh(InputEvent::up(header_of(&e, b))));
        assert_eq!(e.selection.to_vec(), vec![a, b]);
        e.handle_event(&sh(InputEvent::down(header_of(&e, b))));
        e.handle_event(&sh(InputEvent::up(header_of(&e, b))));
        assert_eq!(e.selection.to_vec(), vec![a]);
        e.handle_event(&InputEvent::down(vec2(700.0, 700.0)));
        e.handle_event(&InputEvent::up(vec2(700.0, 700.0)));
        assert!(e.selection.is_empty());
    }

    #[test]
    fn dragging_nodes_moves_the_whole_selection_as_one_undo_step() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let b = e.add_node("t.sink", vec2(400.0, 0.0)).unwrap();
        e.selection.set_all(&[a, b]);
        let from = header_of(&e, a);
        drag(&mut e, from, from + vec2(0.0, 300.0));
        assert_eq!(e.doc.pos(a), Some(vec2(0.0, 300.0)));
        assert_eq!(e.doc.pos(b), Some(vec2(400.0, 300.0)));
        assert_eq!(e.selection.len(), 2);
        assert_eq!(*e.interaction(), Interaction::Idle);
        assert!(e.undo());
        assert_eq!(e.doc.pos(a), Some(vec2(0.0, 0.0)));
        assert_eq!(e.doc.pos(b), Some(vec2(400.0, 0.0)));
        assert!(e.redo());
        assert_eq!(e.doc.pos(b), Some(vec2(400.0, 300.0)));
    }

    #[test]
    fn click_on_selected_node_narrows_selection_and_tiny_move_is_a_click() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let b = e.add_node("t.sink", vec2(400.0, 0.0)).unwrap();
        e.selection.set_all(&[a, b]);
        let p = header_of(&e, a);
        e.handle_event(&InputEvent::down(p));
        e.handle_event(&InputEvent::mv(p + vec2(1.0, 1.0)));
        e.handle_event(&InputEvent::up(p + vec2(1.0, 1.0)));
        assert_eq!(e.selection.to_vec(), vec![a]);
        assert_eq!(e.doc.pos(a), Some(vec2(0.0, 0.0)));
        assert!(!e.can_undo() || e.doc.pos(a) == Some(vec2(0.0, 0.0)));
    }

    #[test]
    fn escape_cancels_a_node_drag() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let p = header_of(&e, a);
        e.handle_event(&InputEvent::down(p));
        e.handle_event(&InputEvent::mv(p + vec2(80.0, 80.0)));
        assert_ne!(e.doc.pos(a), Some(vec2(0.0, 0.0)));
        e.handle_event(&InputEvent::key(Key::Escape));
        assert_eq!(e.doc.pos(a), Some(vec2(0.0, 0.0)));
        e.handle_event(&InputEvent::up(p));
        assert_eq!(*e.interaction(), Interaction::Idle);
    }

    #[test]
    fn grid_snap_and_ctrl_inversion() {
        let mut e = editor();
        e.snap.grid = Some(20.0);
        e.snap.align = false;
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let p = header_of(&e, a);
        drag(&mut e, p, p + vec2(33.0, 47.0));
        assert_eq!(e.doc.pos(a), Some(vec2(40.0, 40.0)));
        // Ctrl turns snapping off
        let p = header_of(&e, a);
        e.handle_event(&InputEvent::down(p));
        e.handle_event(&InputEvent::mv(p + vec2(7.0, 7.0)).with_mods(Modifiers::CTRL));
        e.handle_event(&InputEvent::up(p + vec2(7.0, 7.0)).with_mods(Modifiers::CTRL));
        assert_eq!(e.doc.pos(a), Some(vec2(47.0, 47.0)));
    }

    #[test]
    fn alignment_snaps_to_neighbor_and_leaves_guides_while_dragging() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let b = e.add_node("t.sink", vec2(500.0, 300.0)).unwrap();
        let p = header_of(&e, b);
        e.handle_event(&InputEvent::down(p));
        // 3 px above a's top edge row: should snap onto y = 0 and x aligned with a's left edge
        e.handle_event(&InputEvent::mv(p + vec2(-497.0, -297.0)));
        let pb = e.doc.pos(b).unwrap();
        assert!(pb.x.abs() < 0.01 && pb.y.abs() < 0.01, "{pb:?}");
        assert!(!e.guides.is_empty());
        e.handle_event(&InputEvent::up(p + vec2(-497.0, -297.0)));
        assert!(e.guides.is_empty());
        let _ = a;
    }

    #[test]
    fn box_select_modes() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let b = e.add_node("t.sink", vec2(400.0, 0.0)).unwrap();
        let c = e.add_node("t.sink", vec2(800.0, 0.0)).unwrap();
        e.selection.clear();
        drag(&mut e, vec2(-20.0, -20.0), vec2(450.0, 100.0));
        assert_eq!(e.selection.to_vec(), vec![a, b]);
        // Shift adds
        let sh = Modifiers::SHIFT;
        e.handle_event(&InputEvent::down(vec2(750.0, -20.0)).with_mods(sh));
        e.handle_event(&InputEvent::mv(vec2(900.0, 100.0)).with_mods(sh));
        e.handle_event(&InputEvent::up(vec2(900.0, 100.0)).with_mods(sh));
        assert_eq!(e.selection.to_vec(), vec![a, b, c]);
        // Ctrl subtracts
        let ct = Modifiers::CTRL;
        e.handle_event(&InputEvent::down(vec2(-20.0, -20.0)).with_mods(ct));
        e.handle_event(&InputEvent::mv(vec2(450.0, 100.0)).with_mods(ct));
        e.handle_event(&InputEvent::up(vec2(450.0, 100.0)).with_mods(ct));
        assert_eq!(e.selection.to_vec(), vec![c]);
    }

    #[test]
    fn wire_drag_connects_and_undoes() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let f = e.add_node("t.filter", vec2(400.0, 0.0)).unwrap();
        let (from, to) = (out_pin(&e, a, 0), in_pin(&e, f, 0));
        drag(&mut e, from, to);
        assert_eq!(
            e.doc.all_links(),
            vec![Link {
                from: (a, 0),
                to: (f, 0)
            }]
        );
        e.undo();
        assert!(e.doc.all_links().is_empty());
    }

    #[test]
    fn wire_drag_reports_feedback_and_refuses_bad_drops() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let n = e.add_node("t.num", vec2(0.0, 300.0)).unwrap();
        let f = e.add_node("t.filter", vec2(400.0, 0.0)).unwrap();
        // float output onto an image input: mismatch
        let (from, to) = (out_pin(&e, n, 0), in_pin(&e, f, 0));
        e.handle_event(&InputEvent::mv(from));
        e.handle_event(&InputEvent::down(from));
        e.handle_event(&InputEvent::mv(to));
        match e.interaction() {
            Interaction::DragWire(d) => assert_eq!(d.target, Some((f, 0, Feedback::TypeMismatch))),
            other => panic!("{other:?}"),
        }
        e.handle_event(&InputEvent::up(to));
        assert!(e.doc.all_links().is_empty());
        assert!(e.drain_actions().is_empty()); // not an empty-space drop
        // same node
        let (from, to) = (out_pin(&e, f, 0), in_pin(&e, f, 0));
        e.handle_event(&InputEvent::down(from));
        e.handle_event(&InputEvent::mv(to));
        if let Interaction::DragWire(d) = e.interaction() {
            assert_eq!(d.target.map(|t| t.2), Some(Feedback::SameNode));
        }
        e.handle_event(&InputEvent::up(to));
        let _ = a;
    }

    #[test]
    fn wire_drag_cycle_feedback() {
        let mut e = editor();
        let f1 = e.add_node("t.filter", vec2(0.0, 0.0)).unwrap();
        let f2 = e.add_node("t.filter", vec2(400.0, 0.0)).unwrap();
        e.connect((f1, 0), (f2, 0));
        let (from, to) = (out_pin(&e, f2, 0), in_pin(&e, f1, 0));
        e.handle_event(&InputEvent::down(from));
        e.handle_event(&InputEvent::mv(to));
        match e.interaction() {
            Interaction::DragWire(d) => assert_eq!(d.target.map(|t| t.2), Some(Feedback::Cycle)),
            other => panic!("{other:?}"),
        }
        e.handle_event(&InputEvent::up(to));
        assert_eq!(e.doc.all_links().len(), 1);
    }

    #[test]
    fn dropping_a_wire_on_empty_space_opens_the_add_menu() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let from = out_pin(&e, a, 0);
        drag(&mut e, from, vec2(600.0, 500.0));
        assert_eq!(
            e.drain_actions(),
            vec![EditorAction::OpenAddMenu {
                graph_pos: vec2(600.0, 500.0),
                connect: Some(MenuConnect::FromOut(a, 0))
            }]
        );
        // dragging from an unconnected input asks for a node to feed it
        let f = e.add_node("t.filter", vec2(400.0, 0.0)).unwrap();
        let ip = in_pin(&e, f, 0);
        drag(&mut e, ip, vec2(300.0, 700.0));
        assert_eq!(
            e.drain_actions(),
            vec![EditorAction::OpenAddMenu {
                graph_pos: vec2(300.0, 700.0),
                connect: Some(MenuConnect::ToIn(f, 0))
            }]
        );
    }

    #[test]
    fn dragging_off_a_connected_input_reroutes_or_removes() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let f1 = e.add_node("t.filter", vec2(400.0, 0.0)).unwrap();
        let f2 = e.add_node("t.filter", vec2(400.0, 300.0)).unwrap();
        e.connect((a, 0), (f1, 0));
        // re-route to f2
        let (from, to) = (in_pin(&e, f1, 0), in_pin(&e, f2, 0));
        drag(&mut e, from, to);
        assert_eq!(
            e.doc.all_links(),
            vec![Link {
                from: (a, 0),
                to: (f2, 0)
            }]
        );
        // drop in empty space: the link is removed (no menu)
        let from = in_pin(&e, f2, 0);
        drag(&mut e, from, vec2(200.0, 900.0));
        assert!(e.doc.all_links().is_empty());
        assert!(e.drain_actions().is_empty());
        e.undo();
        assert_eq!(e.doc.all_links().len(), 1);
    }

    #[test]
    fn link_cut_removes_crossed_wires_only() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let f = e.add_node("t.filter", vec2(500.0, 0.0)).unwrap();
        let a2 = e.add_node("t.src", vec2(0.0, 600.0)).unwrap();
        let f2 = e.add_node("t.filter", vec2(500.0, 600.0)).unwrap();
        e.connect((a, 0), (f, 0));
        e.connect((a2, 0), (f2, 0));
        let (p, q) = (out_pin(&e, a, 0), in_pin(&e, f, 0));
        let mid = p.lerp(q, 0.5);
        let rmb = |ev: InputEvent| {
            ev.with_button(PointerButton::Secondary)
                .with_mods(Modifiers::CTRL)
        };
        e.handle_event(&rmb(InputEvent::down(mid + vec2(0.0, -80.0))));
        e.handle_event(&rmb(InputEvent::mv(mid + vec2(0.0, -20.0))));
        e.handle_event(&rmb(InputEvent::mv(mid + vec2(0.0, 40.0))));
        e.handle_event(&rmb(InputEvent::mv(mid + vec2(0.0, 80.0))));
        match e.interaction() {
            Interaction::CutLinks { hits, .. } => assert_eq!(hits.len(), 1),
            other => panic!("{other:?}"),
        }
        e.handle_event(&rmb(InputEvent::up(mid + vec2(0.0, 80.0))));
        assert_eq!(
            e.doc.all_links(),
            vec![Link {
                from: (a2, 0),
                to: (f2, 0)
            }]
        );
        e.undo();
        assert_eq!(e.doc.all_links().len(), 2);
    }

    #[test]
    fn right_click_opens_context_menu_and_scroll_zooms_about_cursor() {
        let mut e = editor();
        let rmb = |ev: InputEvent| ev.with_button(PointerButton::Secondary);
        e.handle_event(&rmb(InputEvent::down(vec2(100.0, 100.0))));
        e.handle_event(&rmb(InputEvent::up(vec2(100.0, 100.0))));
        assert_eq!(
            e.drain_actions(),
            vec![EditorAction::OpenContextMenu {
                graph_pos: vec2(100.0, 100.0),
                context: None
            }]
        );
        let p = vec2(300.0, 200.0);
        let before = e.view.to_graph(p);
        e.handle_event(&InputEvent::scroll(p, 3.0));
        assert!(e.view.zoom > 1.0);
        assert!((e.view.to_graph(p) - before).length() < 1e-2);
        // middle-drag pans
        let o = e.view.origin;
        let mmb = |ev: InputEvent| ev.with_button(PointerButton::Middle);
        e.handle_event(&mmb(InputEvent::down(vec2(10.0, 10.0))));
        e.handle_event(&InputEvent::mv(vec2(60.0, 10.0)));
        e.handle_event(&mmb(InputEvent::up(vec2(60.0, 10.0))));
        assert!(e.view.origin.x < o.x);
    }

    #[test]
    fn one_wheel_notch_zooms_by_exactly_the_zoom_step() {
        let mut e = editor();
        let p = vec2(300.0, 200.0);
        let z0 = e.view.zoom;
        e.handle_event(&InputEvent::scroll(p, 1.0));
        assert!((e.view.zoom / z0 - ZOOM_STEP).abs() < 1e-4);
        // three lines out after one line in leaves two lines out
        e.handle_event(&InputEvent::scroll(p, -3.0));
        assert!((e.view.zoom / z0 - ZOOM_STEP.powi(-2)).abs() < 1e-4);
    }

    #[test]
    fn dropping_a_node_on_a_wire_inserts_it() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let s = e.add_node("t.sink", vec2(800.0, 0.0)).unwrap();
        e.connect((a, 0), (s, 0));
        let f = e.add_node("t.filter", vec2(300.0, 300.0)).unwrap();
        let (p, q) = (out_pin(&e, a, 0), in_pin(&e, s, 0));
        let on_wire = p.lerp(q, 0.5);
        let start = header_of(&e, f);
        e.handle_event(&InputEvent::mv(start));
        e.handle_event(&InputEvent::down(start));
        e.handle_event(&InputEvent::mv(on_wire + vec2(0.0, 60.0)));
        e.handle_event(&InputEvent::mv(on_wire));
        match e.interaction() {
            Interaction::DragNodes(d) => assert!(d.insert.is_some()),
            other => panic!("{other:?}"),
        }
        e.handle_event(&InputEvent::up(on_wire));
        assert_eq!(
            e.doc.all_links(),
            vec![
                Link {
                    from: (a, 0),
                    to: (f, 0)
                },
                Link {
                    from: (f, 0),
                    to: (s, 0)
                }
            ]
        );
        // move + insertion is a single undo step
        e.undo();
        assert_eq!(
            e.doc.all_links(),
            vec![Link {
                from: (a, 0),
                to: (s, 0)
            }]
        );
        assert_eq!(e.doc.pos(f), Some(vec2(300.0, 300.0)));
    }

    #[test]
    fn frames_carry_the_nodes_inside_them() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(100.0, 100.0)).unwrap();
        let out = e.add_node("t.sink", vec2(900.0, 900.0)).unwrap();
        e.selection.set_only(a);
        let fr = e.frame_selected().unwrap();
        let p = e.doc.pos(fr).unwrap() + vec2(30.0, 8.0);
        drag(&mut e, p, p + vec2(50.0, 0.0));
        assert_eq!(e.doc.pos(a), Some(vec2(150.0, 100.0)));
        assert_eq!(e.doc.pos(out), Some(vec2(900.0, 900.0)));
    }

    #[test]
    fn resize_node_width_with_undo() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let (w0, _) = e.doc.size(a).unwrap();
        // grab the right edge below the pin rows
        let r = e.doc.graph.node(e.doc.id_of(a).unwrap()).unwrap().rect();
        let grab = vec2(r.max.x, r.max.y - 3.0);
        drag(&mut e, grab, grab + vec2(60.0, 0.0));
        assert_eq!(e.doc.size(a).unwrap().0, w0 + 60.0);
        e.undo();
        assert_eq!(e.doc.size(a).unwrap().0, w0);
    }

    #[test]
    fn keyboard_shortcuts() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        e.handle_event(&InputEvent::key(Key::A));
        assert_eq!(e.selection.to_vec(), vec![a]);
        e.handle_event(&InputEvent::key(Key::M));
        assert!(e.doc.data(a).unwrap().muted);
        let ctrl = |k| InputEvent::key(k).with_mods(Modifiers::CTRL);
        e.handle_event(&ctrl(Key::Z));
        assert!(!e.doc.data(a).unwrap().muted);
        e.handle_event(&ctrl(Key::Y));
        assert!(e.doc.data(a).unwrap().muted);
        e.handle_event(&ctrl(Key::C));
        e.handle_event(&ctrl(Key::V));
        assert_eq!(e.doc.node_count(), 2);
        e.handle_event(&InputEvent::key(Key::Delete));
        assert_eq!(e.doc.node_count(), 1);
        e.handle_event(&InputEvent::key(Key::Home));
        assert!(e.view.zoom > 0.0);
    }

    #[test]
    fn scrubbing_a_field_is_one_undo_step_and_typing_works() {
        let mut e = editor();
        let f = e.add_node("t.filter", vec2(0.0, 0.0)).unwrap();
        // row 1 (Radius) holds the float field
        let r = e.slot_rect(f, 1).unwrap();
        let c = r.center();
        e.handle_event(&InputEvent::mv(c));
        e.handle_event(&InputEvent::down(c));
        for dx in [10.0, 20.0, 30.0, 40.0] {
            e.handle_event(&InputEvent::mv(c + vec2(dx, 0.0)));
        }
        e.handle_event(&InputEvent::up(c + vec2(40.0, 0.0)));
        let v = e.doc.data(f).unwrap().values[1].as_f32().unwrap();
        assert!(v > 2.5, "{v}");
        assert!(e.undo());
        assert_eq!(e.doc.data(f).unwrap().values[1], Value::Float(2.0));
        assert!(!e.can_undo() || e.doc.node_count() == 1);
        e.redo();
        // click without moving -> text edit overlay
        e.handle_event(&InputEvent::down(c));
        e.handle_event(&InputEvent::up(c));
        assert!(matches!(
            e.drain_actions().last(),
            Some(EditorAction::TextEdit { .. })
        ));
        assert!(e.pending_text_edit().is_some());
        // events are ignored while the host's text field is open
        e.handle_event(&InputEvent::key(Key::Delete));
        e.commit_text("12.5");
        assert_eq!(e.doc.data(f).unwrap().values[1], Value::Float(12.5));
        assert_eq!(*e.interaction(), Interaction::Idle);
        e.undo();
        assert!(e.doc.data(f).unwrap().values[1].as_f32().unwrap() < 12.0);
    }

    #[test]
    fn cancelled_text_edit_changes_nothing() {
        let mut e = editor();
        let f = e.add_node("t.filter", vec2(0.0, 0.0)).unwrap();
        let c = e.slot_rect(f, 1).unwrap().center();
        e.handle_event(&InputEvent::down(c));
        e.handle_event(&InputEvent::up(c));
        assert!(e.pending_text_edit().is_some());
        e.cancel_text();
        assert!(e.pending_text_edit().is_none());
        assert_eq!(*e.interaction(), Interaction::Idle);
        assert_eq!(e.doc.data(f).unwrap().values[1], Value::Float(2.0));
    }

    #[test]
    fn connected_inputs_hide_their_widget() {
        let mut e = editor();
        let n = e.add_node("t.num", vec2(0.0, 0.0)).unwrap();
        let f = e.add_node("t.filter", vec2(400.0, 0.0)).unwrap();
        let c = e.slot_rect(f, 1).unwrap().center();
        assert_eq!(e.widget_at(c), Some((f, 1, false)));
        assert_eq!(e.connect((n, 0), (f, 1)), Feedback::Ok);
        assert_eq!(e.widget_at(c), None);
        e.undo();
        assert_eq!(e.widget_at(c), Some((f, 1, false)));
        // and below Full level of detail
        e.view.zoom = 0.4;
        assert_eq!(e.widget_at(c), None);
    }

    #[test]
    fn scripted_session_end_to_end() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let f = e.add_node("t.filter", vec2(400.0, 0.0)).unwrap();
        let s = e.add_node("t.sink", vec2(800.0, 0.0)).unwrap();
        let (p1, q1) = (out_pin(&e, a, 0), in_pin(&e, f, 0));
        drag(&mut e, p1, q1);
        let (p2, q2) = (out_pin(&e, f, 0), in_pin(&e, s, 0));
        drag(&mut e, p2, q2);
        assert_eq!(e.doc.all_links().len(), 2);
        // delete the middle node: ends up disconnected; undo reconnects it
        e.selection.set_only(f);
        e.handle_event(&InputEvent::key(Key::Delete));
        assert!(e.doc.all_links().is_empty());
        e.handle_event(&InputEvent::key(Key::Z).with_mods(Modifiers::CTRL));
        assert_eq!(e.doc.all_links().len(), 2);
    }
}
