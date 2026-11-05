//! The editor: document + viewport + selection + widgets + undo history +
//! the interaction state machine. Feed it [`InputEvent`]s (screen space),
//! read back [`EditorAction`]s, and ask it to draw into a [`DrawList`].
//! No window, GPU or toolkit is involved, so whole editing sessions can be
//! scripted and verified in tests.
use crate::command::{Command, Flag, History};
use crate::document::{Document, Link, NodeData, Snapshot};
use crate::schema::{Registry, SocketDef, WidgetKind};
use crate::selection::Selection;
use crate::theme::{Num, Theme};
use crate::value::{ImageHandle, SocketType, Value};
use crate::widget::{TextEditRequest, Widget, WidgetFactory, make_widget};
use compnode_core::geom::vec2;
use compnode_core::hash::FxMap;
use compnode_core::layout::{HEADER_H, ROW_H};
use compnode_core::{Node, NodeId, PinRef, Rect, Vec2, Viewport, Visible, WireCache};

pub mod focus;
mod interact;
mod render;

/// How far (screen pixels) beyond the viewport the draw list reaches. Equal to
/// the cell size of [`DrawKey`], so a cached draw list can be translated while
/// the pan stays inside one cell.
pub const DRAW_MARGIN_PX: f32 = 256.0;

/// Zoom multiplier per scroll line; a mouse-wheel notch is one line. Higher
/// is faster. Hosts convert a pinch-zoom factor `f` into scroll lines with
/// `f.ln() / ZOOM_STEP.ln()`, so a pinch always scales the view by exactly
/// `f` while wheel scrolling steps by this constant.
pub const ZOOM_STEP: f32 = 1.2;

pub(crate) type WidgetMap = FxMap<(u64, usize, bool), Box<dyn Widget>>;

/// Result of checking whether a link could be made.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Feedback {
    Ok,
    SameNode,
    BadPin,
    TypeMismatch,
    Cycle,
}

/// Where a wire drag started (node uid, pin index).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WireStart {
    Out(u64, usize),
    In(u64, usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectMode {
    Replace,
    Add,
    Subtract,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResizeKind {
    Width,
    Corner,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeDrag {
    pub anchor: Vec2,
    pub primary: u64,
    pub orig: Vec<(u64, Vec2)>,
    pub moved: bool,
    /// Set when a selected node was pressed: a click without a drag then
    /// narrows the selection to it.
    pub click_uid: Option<u64>,
    /// Wire the dragged node would be inserted into on drop.
    pub insert: Option<Link>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WireDrag {
    pub start: WireStart,
    pub cursor: Vec2,
    /// Pin under the cursor (node uid, pin index) and whether linking works.
    pub target: Option<(u64, usize, Feedback)>,
    /// The existing link being re-routed (dragged off an input).
    pub replacing: Option<Link>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Interaction {
    Idle,
    Pan {
        last: Vec2,
    },
    DragNodes(NodeDrag),
    BoxSelect {
        start: Vec2,
        cur: Vec2,
        mode: SelectMode,
    },
    DragWire(WireDrag),
    CutLinks {
        points: Vec<Vec2>,
        hits: Vec<Link>,
    },
    ContextClick {
        start: Vec2,
        graph_pos: Vec2,
    },
    EditWidget {
        uid: u64,
        slot: usize,
        output: bool,
    },
    Resize {
        uid: u64,
        kind: ResizeKind,
        anchor: Vec2,
        old: (f32, f32),
    },
}

/// An alignment guide shown while dragging nodes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guide {
    pub vertical: bool,
    pub pos: f32,
    pub from: f32,
    pub to: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapSettings {
    /// Grid step in graph units, or `None`. Holding Ctrl inverts this.
    pub grid: Option<f32>,
    /// Pin pick radius in screen pixels.
    pub pin_radius: f32,
    /// Align dragged nodes to other nodes' edges and centers.
    pub align: bool,
    /// Alignment distance in screen pixels.
    pub align_px: f32,
}

impl Default for SnapSettings {
    fn default() -> Self {
        SnapSettings {
            grid: None,
            pin_radius: 12.0,
            align: true,
            align_px: 6.0,
        }
    }
}

/// What the add-node menu should connect the new node to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuConnect {
    /// Wire from this output into the new node.
    FromOut(u64, usize),
    /// Wire from the new node into this input.
    ToIn(u64, usize),
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditorAction {
    /// Show the add-node menu at `graph_pos` (search with [`Registry::search`],
    /// then call [`Editor::add_from_menu`]).
    OpenAddMenu {
        graph_pos: Vec2,
        connect: Option<MenuConnect>,
    },
    /// Show the right-click context menu at `graph_pos`; `context` is the
    /// hovered node (if any) it was opened on.
    OpenContextMenu {
        graph_pos: Vec2,
        context: Option<u64>,
    },
    ButtonPressed {
        uid: u64,
        input: usize,
    },
    /// Show a text field; answer with `commit_text` / `cancel_text`.
    TextEdit {
        uid: u64,
        slot: usize,
        request: TextEditRequest,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingText {
    pub uid: u64,
    pub slot: usize,
    pub output: bool,
    pub request: TextEditRequest,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Hover {
    pub node: Option<u64>,
    /// `(uid, pin index, is_output)`
    pub pin: Option<(u64, usize, bool)>,
    pub wire: Option<Link>,
}

/// Compare two keys to know whether cached draw output is still valid.
/// Panning inside a 256 px cell does not change the key: translate the cached
/// geometry instead.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DrawKey {
    pub doc: u64,
    pub ui: u64,
    pub theme: u64,
    pub zoom_bucket: i32,
    pub cell: (i32, i32),
}

pub struct Editor {
    pub doc: Document,
    pub view: Viewport,
    pub theme: Theme,
    pub selection: Selection,
    pub snap: SnapSettings,
    pub guides: Vec<Guide>,
    pub errors: FxMap<u64, String>,
    interaction: Interaction,
    hover: Hover,
    hover_widget: Option<(u64, usize, bool)>,
    focus: Option<(u64, usize, bool)>,
    history: History,
    widgets: WidgetMap,
    /// Host-supplied widget builder; consulted before the built-in set.
    widget_factory: Option<WidgetFactory>,
    mods: crate::input::Modifiers,
    pointer: Vec2,
    clipboard: Option<Snapshot>,
    actions: Vec<EditorAction>,
    pending_text: Option<PendingText>,
    wire_cache: WireCache,
    open_gestures: u32,
    ui_rev: u64,
    last_count: usize,
}

fn ensure_widget<'a>(
    map: &'a mut WidgetMap,
    factory: Option<&WidgetFactory>,
    uid: u64,
    slot: usize,
    output: bool,
    def: &SocketDef,
    label_frac: f32,
) -> Option<&'a mut Box<dyn Widget>> {
    let key = (uid, slot, output);
    if map.get(&key).is_none() {
        let custom = factory.and_then(|f| f(def));
        let w: Box<dyn Widget> = match custom {
            Some(w) => w,
            None => make_widget(def)?,
        };
        map.insert(key, w);
    }
    let w = map.get_mut(&key)?;
    w.set_label_frac(label_frac);
    Some(w)
}

/// Top of row `i` (graph space) and its height.
fn row_top(node: &Node<NodeData>, i: usize) -> (f32, f32) {
    let rows = node.rows();
    if rows.is_empty() {
        (node.pos().y + HEADER_H + i as f32 * ROW_H, ROW_H)
    } else {
        let above: f32 = rows.iter().take(i).sum();
        let extra = i.saturating_sub(rows.len()) as f32 * ROW_H;
        (
            node.pos().y + HEADER_H + above + extra,
            rows.get(i).copied().unwrap_or(ROW_H),
        )
    }
}

impl Editor {
    pub fn new(registry: Registry) -> Editor {
        Editor {
            doc: Document::new(registry),
            view: Viewport::new(vec2(1280.0, 720.0)),
            theme: Theme::default(),
            selection: Selection::default(),
            snap: SnapSettings::default(),
            guides: Vec::new(),
            errors: FxMap::default(),
            interaction: Interaction::Idle,
            hover: Hover::default(),
            hover_widget: None,
            focus: None,
            history: History::new(),
            widgets: FxMap::default(),
            widget_factory: None,
            mods: crate::input::Modifiers::NONE,
            pointer: Vec2::ZERO,
            clipboard: None,
            actions: Vec::new(),
            pending_text: None,
            wire_cache: WireCache::new(),
            open_gestures: 0,
            ui_rev: 0,
            last_count: 0,
        }
    }

    // ───────────── state access ─────────────

    pub fn interaction(&self) -> &Interaction {
        &self.interaction
    }
    pub fn hover(&self) -> &Hover {
        &self.hover
    }
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }
    pub fn pending_text_edit(&self) -> Option<&PendingText> {
        self.pending_text.as_ref()
    }
    pub fn drain_actions(&mut self) -> Vec<EditorAction> {
        std::mem::take(&mut self.actions)
    }
    pub fn set_viewport_size(&mut self, size: Vec2) {
        self.view.size = size;
        self.ui_rev += 1;
    }
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.ui_rev += 1;
        for u in self.doc.uids() {
            self.relayout(u);
        }
    }

    /// Install a widget factory. It is asked for every socket before the
    /// built-in widget set, so a host can add widgets the library does not
    /// know about (`SocketDef::custom_widget`) and replace built-in ones.
    ///
    /// Widgets already built for a socket are dropped, so the new factory
    /// takes effect immediately.
    pub fn set_widget_factory(&mut self, factory: WidgetFactory) {
        self.widget_factory = Some(factory);
        self.widgets.clear();
        self.ui_rev += 1;
        for u in self.doc.uids() {
            self.relayout(u);
        }
    }
    pub fn set_node_error(&mut self, uid: u64, msg: Option<String>) {
        match msg {
            Some(m) => {
                self.errors.insert(uid, m);
            }
            None => {
                self.errors.remove(&uid);
            }
        }
        self.ui_rev += 1;
    }
    pub fn draw_key(&self) -> DrawKey {
        let cell_px = DRAW_MARGIN_PX;
        DrawKey {
            doc: self.doc.revision,
            ui: self.ui_rev,
            theme: self.theme.revision,
            zoom_bucket: (self.view.zoom.log2() * 4.0).round() as i32,
            cell: (
                (self.view.origin.x * self.view.zoom / cell_px).floor() as i32,
                (self.view.origin.y * self.view.zoom / cell_px).floor() as i32,
            ),
        }
    }
    /// Uids of nodes in the current view (for lazily generated thumbnails).
    pub fn visible_uids(&self) -> Vec<u64> {
        let mut vis = Visible::default();
        self.doc
            .graph
            .collect_visible(&self.view.visible_rect(0.0), &mut vis);
        let mut v: Vec<u64> = vis
            .nodes
            .iter()
            .filter_map(|id| self.doc.uid_of(*id))
            .collect();
        v.sort_unstable();
        v
    }

    // ───────────── socket types and link checks ─────────────

    pub fn out_type(&self, uid: u64, idx: usize) -> Option<SocketType> {
        self.doc.schema_of(uid)?.outputs.get(idx).map(|d| d.ty)
    }
    pub fn in_type(&self, uid: u64, idx: usize) -> Option<SocketType> {
        self.doc.schema_of(uid)?.inputs.get(idx).map(|d| d.ty)
    }

    pub fn check_link(&self, link: Link) -> Feedback {
        let (Some(ft), Some(tt)) = (
            self.out_type(link.from.0, link.from.1),
            self.in_type(link.to.0, link.to.1),
        ) else {
            return Feedback::BadPin;
        };
        if link.from.0 == link.to.0 {
            return Feedback::SameNode;
        }
        if !SocketType::can_connect(ft, tt) {
            return Feedback::TypeMismatch;
        }
        match self.doc.can_connect(link) {
            Ok(()) => Feedback::Ok,
            Err(compnode_core::ConnectError::SameNode) => Feedback::SameNode,
            Err(compnode_core::ConnectError::Cycle) => Feedback::Cycle,
            Err(_) => Feedback::BadPin,
        }
    }

    // ───────────── layout ─────────────

    /// Recompute row heights for a node (widgets are taller than plain rows).
    pub(crate) fn relayout(&mut self, uid: u64) {
        let Some(id) = self.doc.id_of(uid) else {
            return;
        };
        let rows: Vec<f32> = {
            let Some(node) = self.doc.graph.node(id) else {
                return;
            };
            let Some(schema) = self.doc.registry.get(&node.data.schema) else {
                return;
            };
            let n_rows = schema.inputs.len().max(schema.outputs.len());
            if node.data.is_frame() {
                vec![node.data.frame_h.max(20.0)]
            } else if node.data.is_reroute() {
                vec![0.0]
            } else if node.data.collapsed {
                vec![0.0; n_rows + schema.footer.len()]
            } else {
                let w = (node.width() - 2.0 * self.theme.n(Num::FieldInset)).max(10.0);
                let label_frac = self.theme.n(Num::WidgetLabelFrac);
                let mut rows = Vec::with_capacity(n_rows);
                for i in 0..n_rows {
                    let mut h = ROW_H;
                    if let Some(def) = schema.inputs.get(i)
                        && def.widget != WidgetKind::None
                        && let Some(wd) = ensure_widget(
                            &mut self.widgets,
                            self.widget_factory.as_ref(),
                            uid,
                            i,
                            false,
                            def,
                            label_frac,
                        )
                        && node.input_wire(i).is_none()
                    {
                        h = h.max(wd.measure(w, &node.data.values[i]));
                    }
                    if let Some(def) = schema.outputs.get(i)
                        && def.widget != WidgetKind::None
                        && let Some(wd) = ensure_widget(
                            &mut self.widgets,
                            self.widget_factory.as_ref(),
                            uid,
                            i,
                            true,
                            def,
                            label_frac,
                        )
                    {
                        h = h.max(wd.measure(w, &node.data.out_values[i]));
                    }
                    rows.push(h);
                }
                for (k, def) in schema.footer.iter().enumerate() {
                    let slot = schema.outputs.len() + k;
                    let mut h = ROW_H;
                    if let Some(wd) = ensure_widget(
                        &mut self.widgets,
                        self.widget_factory.as_ref(),
                        uid,
                        slot,
                        true,
                        def,
                        label_frac,
                    ) {
                        h = h.max(wd.measure(w, &node.data.out_values[slot]));
                    }
                    rows.push(h);
                }
                // An empty row list means "one uniform row per pin", so it can
                // only stand in for uniform *pin* rows. Footer rows are extra
                // rows the pin count cannot express: dropping them would leave
                // the footer outside the node body, where it is neither drawn
                // nor hit-tested.
                if schema.footer.is_empty() && rows.iter().all(|h| *h == ROW_H) {
                    rows.clear();
                }
                rows
            }
        };
        self.doc.graph.set_row_heights(id, rows);
    }

    /// Apply pending layout work and forget state of removed nodes. Called
    /// after every public mutation and every event.
    pub fn sync(&mut self) {
        for u in self.doc.take_layout_dirty() {
            self.relayout(u);
        }
        let n = self.doc.node_count();
        if n != self.last_count {
            self.last_count = n;
            let doc = &self.doc;
            self.widgets.retain(|k, _| doc.contains(k.0));
            self.selection.retain_existing(&self.doc);
        }
        if self.focus.is_some_and(|(u, _, _)| !self.doc.contains(u)) {
            self.focus = None;
        }
    }

    /// Rectangle (graph space) where the widget of row `i` is placed.
    pub fn slot_rect(&self, uid: u64, i: usize) -> Option<Rect> {
        let node = self.doc.graph.node(self.doc.id_of(uid)?)?;
        let (top, h) = row_top(node, i);
        let inset = self.theme.n(Num::FieldInset);
        Some(Rect::new(
            vec2(node.pos().x + inset, top + 2.0),
            vec2(
                node.pos().x + node.width() - inset,
                (top + h - 2.0).max(top + 2.0),
            ),
        ))
    }

    /// Where the widget stored at `slot` is placed (maps footer slots to their rows).
    pub fn widget_rect(&self, uid: u64, slot: usize, output: bool) -> Option<Rect> {
        let row = if output {
            self.doc.schema_of(uid)?.out_row(slot)
        } else {
            slot
        };
        self.slot_rect(uid, row)
    }

    // ───────────── hit testing ─────────────

    fn node_at(&self, gp: Vec2) -> Option<u64> {
        let mut vis = Visible::default();
        self.doc.graph.collect_visible(&Rect::new(gp, gp), &mut vis);
        let mut best: Option<(NodeId, u64)> = None;
        let mut frame: Option<(NodeId, u64)> = None;
        for id in vis.nodes {
            let Some(n) = self.doc.graph.node(id) else {
                continue;
            };
            if !n.rect().contains(gp) {
                continue;
            }
            if n.data.is_frame() {
                if gp.y <= n.pos().y + HEADER_H && frame.is_none_or(|(b, _)| id > b) {
                    frame = Some((id, n.data.uid));
                }
            } else if best.is_none_or(|(b, _)| id > b) {
                best = Some((id, n.data.uid));
            }
        }
        best.or(frame).map(|b| b.1)
    }

    /// Pin near `gp`: `(uid, index, is_output)`. Collapsed nodes have no pins.
    pub(crate) fn pin_at(&self, gp: Vec2) -> Option<(u64, usize, bool)> {
        let r = self.snap.pin_radius / self.view.zoom;
        let pin = self.doc.graph.hit_pin(gp, r)?;
        let (id, index, out) = match pin {
            PinRef::Out(o) => (o.node, o.index, true),
            PinRef::In(i) => (i.node, i.index, false),
        };
        let n = self.doc.graph.node(id)?;
        if n.data.collapsed {
            return None;
        }
        Some((n.data.uid, index, out))
    }

    pub(crate) fn widget_at(&self, gp: Vec2) -> Option<(u64, usize, bool)> {
        if self.view.lod() != compnode_core::Lod::Full {
            return None;
        }
        let uid = self.node_at(gp)?;
        let node = self.doc.graph.node(self.doc.id_of(uid)?)?;
        if node.data.is_frame() || node.data.is_reroute() || node.data.collapsed {
            return None;
        }
        let schema = self.doc.registry.get(&node.data.schema)?;
        for i in 0..schema.inputs.len().max(schema.outputs.len()) {
            let Some(rect) = self.slot_rect(uid, i) else {
                continue;
            };
            if !rect.contains(gp) {
                continue;
            }
            if schema
                .inputs
                .get(i)
                .is_some_and(|d| d.widget != WidgetKind::None)
                && node.input_wire(i).is_none()
            {
                return Some((uid, i, false));
            }
            if schema
                .outputs
                .get(i)
                .is_some_and(|d| d.widget != WidgetKind::None)
            {
                return Some((uid, i, true));
            }
        }
        for k in 0..schema.footer.len() {
            let slot = schema.outputs.len() + k;
            if self
                .widget_rect(uid, slot, true)
                .is_some_and(|r| r.contains(gp))
            {
                return Some((uid, slot, true));
            }
        }
        None
    }

    pub(crate) fn resize_at(&self, gp: Vec2) -> Option<(u64, ResizeKind)> {
        let mut vis = Visible::default();
        self.doc
            .graph
            .collect_visible(&Rect::new(gp, gp).expand(10.0), &mut vis);
        let mut best: Option<(NodeId, u64, ResizeKind)> = None;
        for id in vis.nodes {
            let Some(n) = self.doc.graph.node(id) else {
                continue;
            };
            if n.data.is_reroute() {
                continue;
            }
            let r = n.rect();
            let kind = if n.data.is_frame()
                && (gp.x - r.max.x).abs() <= 10.0
                && (gp.y - r.max.y).abs() <= 10.0
            {
                Some(ResizeKind::Corner)
            } else if (gp.x - r.max.x).abs() <= 5.0 && gp.y >= r.min.y + HEADER_H && gp.y <= r.max.y
            {
                Some(ResizeKind::Width)
            } else {
                None
            };
            if let Some(k) = kind
                && best.is_none_or(|(b, _, _)| id > b)
            {
                best = Some((id, n.data.uid, k));
            }
        }
        best.map(|(_, u, k)| (u, k))
    }

    // ───────────── widget plumbing ─────────────

    pub(crate) fn widget_captures(&self, uid: u64, slot: usize, output: bool) -> bool {
        self.widgets
            .get(&(uid, slot, output))
            .is_some_and(|w| w.captures())
    }

    /// Hand a graph-space event to a widget and process its response.
    pub(crate) fn widget_event(
        &mut self,
        uid: u64,
        slot: usize,
        output: bool,
        ev: &crate::input::InputEvent,
    ) -> Option<crate::widget::Response> {
        let rect = self.widget_rect(uid, slot, output)?;
        let id = self.doc.id_of(uid)?;
        let data = self.doc.graph.node_data_mut(id)?;
        let value = if output {
            data.out_values.get_mut(slot)?
        } else {
            data.values.get_mut(slot)?
        };
        let w = self.widgets.get_mut(&(uid, slot, output))?;
        let before = value.clone();
        let resp = w.event(ev, rect, value);
        self.apply_response(uid, slot, output, before, &resp);
        Some(resp)
    }

    /// Finish a host text edit with `text` (or cancel with `None`).
    fn widget_commit(&mut self, p: &PendingText, text: Option<&str>) {
        let Some(id) = self.doc.id_of(p.uid) else {
            return;
        };
        let Some(data) = self.doc.graph.node_data_mut(id) else {
            return;
        };
        let value = if p.output {
            data.out_values.get_mut(p.slot)
        } else {
            data.values.get_mut(p.slot)
        };
        let Some(value) = value else { return };
        let Some(w) = self.widgets.get_mut(&(p.uid, p.slot, p.output)) else {
            return;
        };
        match text {
            Some(t) => {
                let before = value.clone();
                let resp = w.commit_text(t, value);
                self.apply_response(p.uid, p.slot, p.output, before, &resp);
            }
            None => w.cancel_edit(),
        }
    }

    pub(crate) fn apply_response(
        &mut self,
        uid: u64,
        slot: usize,
        output: bool,
        before: Value,
        resp: &crate::widget::Response,
    ) {
        if resp.begin_edit {
            self.history.begin();
            self.open_gestures += 1;
        }
        if resp.changed {
            if output {
                self.doc.note_out_value_changed(uid);
            } else {
                self.doc.note_value_changed(uid, slot);
                let after = self.doc.data(uid).and_then(|d| d.values.get(slot)).cloned();
                if let Some(new) = after {
                    self.history.record(Command::SetValue {
                        uid,
                        input: slot,
                        old: before,
                        new,
                    });
                }
            }
        }
        if resp.end_edit && self.open_gestures > 0 {
            self.history.end();
            self.open_gestures -= 1;
        }
        if resp.clicked {
            self.actions
                .push(EditorAction::ButtonPressed { uid, input: slot });
        }
        if let Some(req) = &resp.text_edit {
            self.pending_text = Some(PendingText {
                uid,
                slot,
                output,
                request: req.clone(),
            });
            self.actions.push(EditorAction::TextEdit {
                uid,
                slot,
                request: req.clone(),
            });
        }
        self.ui_rev += 1;
    }

    /// The host's text field finished with `text`.
    pub fn commit_text(&mut self, text: &str) {
        if let Some(p) = self.pending_text.take() {
            self.widget_commit(&p, Some(text));
            self.after_text_edit(&p);
        }
        self.sync();
    }

    /// The host's text field was dismissed.
    pub fn cancel_text(&mut self) {
        if let Some(p) = self.pending_text.take() {
            self.widget_commit(&p, None);
            self.after_text_edit(&p);
        }
    }

    fn after_text_edit(&mut self, p: &PendingText) {
        if let Interaction::EditWidget { .. } = self.interaction
            && !self.widget_captures(p.uid, p.slot, p.output)
        {
            self.interaction = Interaction::Idle;
        }
    }

    /// Host-provided preview image for an output widget.
    pub fn set_preview(
        &mut self,
        uid: u64,
        output: usize,
        handle: Option<ImageHandle>,
        aspect: f32,
    ) {
        self.doc.set_out_value(uid, output, Value::Image(handle));
        if let Some(w) = self.widgets.get_mut(&(uid, output, true)) {
            w.set_aspect(aspect);
        }
        self.sync();
        self.ui_rev += 1;
    }

    // ───────────── commands ─────────────

    fn execute(&mut self, cmd: Command) {
        self.history.execute(&mut self.doc, cmd);
        self.sync();
        self.ui_rev += 1;
    }

    /// Run several commands as one undo step.
    fn execute_group(&mut self, cmds: Vec<Command>) {
        self.history.begin();
        for c in cmds {
            self.history.execute(&mut self.doc, c);
        }
        self.history.end();
        self.sync();
        self.ui_rev += 1;
    }

    pub fn undo(&mut self) -> bool {
        let r = self.history.undo(&mut self.doc);
        self.sync();
        self.ui_rev += 1;
        r
    }

    pub fn redo(&mut self) -> bool {
        let r = self.history.redo(&mut self.doc);
        self.sync();
        self.ui_rev += 1;
        r
    }

    /// Add a node (one undo step) and select it.
    pub fn add_node(&mut self, schema_id: &str, pos: Vec2) -> Option<u64> {
        self.add_from_menu(schema_id, pos, None)
    }

    /// Queue the add-node menu at `graph_pos` (Shift+A, context menu, toolbar).
    pub fn open_add_menu_at(&mut self, graph_pos: Vec2, connect: Option<MenuConnect>) {
        self.actions
            .push(EditorAction::OpenAddMenu { graph_pos, connect });
    }

    /// Add a node chosen in the add-node menu, optionally wired to the pin
    /// the menu was opened from (the first compatible socket is used).
    pub fn add_from_menu(
        &mut self,
        schema_id: &str,
        pos: Vec2,
        connect: Option<MenuConnect>,
    ) -> Option<u64> {
        let snap = self.doc.new_snapshot(schema_id, pos)?;
        let uid = snap.uid;
        let schema = self.doc.registry.get(schema_id)?.clone();
        let mut cmds = vec![Command::AddNodes(Snapshot {
            nodes: vec![snap],
            links: vec![],
        })];
        match connect {
            Some(MenuConnect::FromOut(u, i)) => {
                if let Some(ot) = self.out_type(u, i)
                    && let Some(k) = schema
                        .inputs
                        .iter()
                        .position(|d| SocketType::can_connect(ot, d.ty))
                {
                    cmds.push(Command::SetLinks {
                        remove: vec![],
                        add: vec![Link {
                            from: (u, i),
                            to: (uid, k),
                        }],
                    });
                }
            }
            Some(MenuConnect::ToIn(u, i)) => {
                if let Some(it) = self.in_type(u, i)
                    && let Some(k) = schema
                        .outputs
                        .iter()
                        .position(|d| SocketType::can_connect(d.ty, it))
                {
                    let remove: Vec<Link> = self.doc.input_link(u, i).into_iter().collect();
                    cmds.push(Command::SetLinks {
                        remove,
                        add: vec![Link {
                            from: (uid, k),
                            to: (u, i),
                        }],
                    });
                }
            }
            None => {}
        }
        self.execute_group(cmds);
        self.selection.set_only(uid);
        Some(uid)
    }

    /// Connect two pins (one undo step). Replaces whatever feeds the input.
    pub fn connect(&mut self, from: (u64, usize), to: (u64, usize)) -> Feedback {
        let link = Link { from, to };
        let fb = self.check_link(link);
        if fb == Feedback::Ok {
            self.apply_link(link, None);
        }
        fb
    }

    /// Link `link`, removing what it replaces (and `replacing`, a link that
    /// was dragged off its input).
    pub(crate) fn apply_link(&mut self, link: Link, replacing: Option<Link>) {
        let multi = self.doc.is_multi_input(link.to.0, link.to.1);
        if replacing == Some(link) {
            return;
        }
        // A multi-input socket keeps its other wires (it only refuses an exact
        // duplicate); a single-input socket drops whatever fed it.
        if multi {
            if self.doc.input_links(link.to.0, link.to.1).contains(&link) {
                return;
            }
        } else if self.doc.input_link(link.to.0, link.to.1) == Some(link) {
            return;
        }
        let mut remove: Vec<Link> = Vec::new();
        if let Some(r) = replacing {
            remove.push(r);
        }
        if !multi
            && let Some(e) = self.doc.input_link(link.to.0, link.to.1)
            && !remove.contains(&e)
        {
            remove.push(e);
        }
        self.execute(Command::SetLinks {
            remove,
            add: vec![link],
        });
    }

    pub fn disconnect(&mut self, link: Link) {
        self.execute(Command::SetLinks {
            remove: vec![link],
            add: vec![],
        });
    }

    /// Set an unconnected input value (one undo step).
    pub fn set_value(&mut self, uid: u64, input: usize, new: Value) -> bool {
        let Some(old) = self
            .doc
            .data(uid)
            .and_then(|d| d.values.get(input))
            .cloned()
        else {
            return false;
        };
        self.execute(Command::SetValue {
            uid,
            input,
            old,
            new,
        });
        true
    }

    pub fn move_nodes(&mut self, moves: &[(u64, Vec2)]) {
        let m: Vec<(u64, Vec2, Vec2)> = moves
            .iter()
            .filter_map(|(u, to)| self.doc.pos(*u).map(|from| (*u, from, *to)))
            .collect();
        self.execute(Command::MoveNodes(m));
    }

    pub fn delete_selection(&mut self) {
        let uids = self.selection.to_vec();
        if uids.is_empty() {
            return;
        }
        let snap = self.doc.snapshot_nodes(&uids, true);
        self.execute(Command::RemoveNodes(snap));
        self.selection.clear();
    }

    pub fn copy(&mut self) {
        let uids = self.selection.to_vec();
        if !uids.is_empty() {
            self.clipboard = Some(self.doc.snapshot_nodes(&uids, false));
        }
    }

    pub fn has_clipboard(&self) -> bool {
        self.clipboard.is_some()
    }

    /// Paste so that the top-left of the copied nodes lands at `at`.
    pub fn paste(&mut self, at: Vec2) -> Vec<u64> {
        let Some(clip) = self.clipboard.clone() else {
            return Vec::new();
        };
        let min = clip
            .nodes
            .iter()
            .fold(None::<Vec2>, |m, n| Some(m.map_or(n.pos, |m| m.min(n.pos))));
        let shift = min.map_or(Vec2::ZERO, |m| at - m);
        self.insert_copy(&clip, shift)
    }

    /// Copy the selection and paste it offset by 20 units.
    pub fn duplicate(&mut self) -> Vec<u64> {
        let uids = self.selection.to_vec();
        if uids.is_empty() {
            return Vec::new();
        }
        let snap = self.doc.snapshot_nodes(&uids, false);
        self.insert_copy(&snap, vec2(20.0, 20.0))
    }

    fn insert_copy(&mut self, clip: &Snapshot, shift: Vec2) -> Vec<u64> {
        if clip.nodes.is_empty() {
            return Vec::new();
        }
        let mut map: FxMap<u64, u64> = FxMap::default();
        let mut nodes = Vec::with_capacity(clip.nodes.len());
        for n in &clip.nodes {
            let uid = self.doc.alloc_uid();
            map.insert(n.uid, uid);
            let mut c = n.clone();
            c.uid = uid;
            c.pos = n.pos + shift;
            nodes.push(c);
        }
        let links: Vec<Link> = clip
            .links
            .iter()
            .filter_map(|l| {
                Some(Link {
                    from: (*map.get(&l.from.0)?, l.from.1),
                    to: (*map.get(&l.to.0)?, l.to.1),
                })
            })
            .collect();
        let new: Vec<u64> = nodes.iter().map(|n| n.uid).collect();
        self.execute(Command::AddNodes(Snapshot { nodes, links }));
        self.selection.set_all(&new);
        new
    }

    pub fn select_all(&mut self) {
        self.selection.select_all(&self.doc);
        self.ui_rev += 1;
    }

    pub fn invert_selection(&mut self) {
        self.selection.invert(&self.doc);
        self.ui_rev += 1;
    }

    fn flag_cmds(&self, flag: Flag) -> Vec<Command> {
        let mut out = Vec::new();
        for uid in self.selection.to_vec() {
            let Some(d) = self.doc.data(uid) else {
                continue;
            };
            if d.is_frame() || (flag == Flag::Collapsed && d.is_reroute()) {
                continue;
            }
            let old = if flag == Flag::Muted {
                d.muted
            } else {
                d.collapsed
            };
            out.push(Command::SetFlag {
                uid,
                flag,
                old,
                new: !old,
            });
        }
        out
    }

    /// Toggle mute on every selected node.
    pub fn toggle_mute(&mut self) {
        let c = self.flag_cmds(Flag::Muted);
        if !c.is_empty() {
            self.execute_group(c);
        }
    }

    /// Toggle collapse on every selected node.
    pub fn toggle_collapse(&mut self) {
        let c = self.flag_cmds(Flag::Collapsed);
        if !c.is_empty() {
            self.execute_group(c);
        }
    }

    /// Split `link` with a reroute node centered at `at`.
    pub fn insert_reroute(&mut self, link: Link, at: Vec2) -> Option<u64> {
        let snap = self
            .doc
            .new_snapshot(crate::schema::REROUTE_ID, at - vec2(20.0, HEADER_H))?;
        let uid = snap.uid;
        let cmds = vec![
            Command::AddNodes(Snapshot {
                nodes: vec![snap],
                links: vec![],
            }),
            Command::SetLinks {
                remove: vec![link],
                add: vec![
                    Link {
                        from: link.from,
                        to: (uid, 0),
                    },
                    Link {
                        from: (uid, 0),
                        to: link.to,
                    },
                ],
            },
        ];
        self.execute_group(cmds);
        self.selection.set_only(uid);
        Some(uid)
    }

    /// Put a frame around the selected nodes.
    pub fn frame_selected(&mut self) -> Option<u64> {
        let uids = self.selection.to_vec();
        let b = self.doc.bounds(&uids)?;
        let pad = 20.0;
        let mut snap = self.doc.new_snapshot(
            crate::schema::FRAME_ID,
            vec2(b.min.x - pad, b.min.y - pad - HEADER_H),
        )?;
        snap.width = b.width() + 2.0 * pad;
        snap.frame_h = b.height() + 2.0 * pad;
        let uid = snap.uid;
        self.execute(Command::AddNodes(Snapshot {
            nodes: vec![snap],
            links: vec![],
        }));
        self.selection.set_only(uid);
        Some(uid)
    }

    /// Move the view so the clicked minimap point is centered.
    pub fn minimap_click(&mut self, minimap: &crate::minimap::Minimap, p: Vec2) {
        self.view.origin = minimap.origin_for_click(p, &self.view);
        self.ui_rev += 1;
    }

    /// Replace the whole document (opening a project). Selection, undo
    /// history, widgets, focus and errors are reset.
    pub fn replace_document(&mut self, doc: Document) {
        self.doc = doc;
        self.selection = Selection::default();
        self.history = History::new();
        self.widgets.clear();
        self.focus = None;
        self.hover = Hover::default();
        self.hover_widget = None;
        self.interaction = Interaction::Idle;
        self.pending_text = None;
        self.open_gestures = 0;
        self.errors.clear();
        self.guides.clear();
        self.last_count = 0;
        self.wire_cache = WireCache::new();
        for u in self.doc.uids() {
            self.relayout(u);
        }
        self.doc.take_layout_dirty();
        self.view_all();
        self.ui_rev += 1;
    }

    /// Fit every node into the viewport.
    pub fn view_all(&mut self) {
        let all = self.doc.uids();
        if let Some(b) = self.doc.bounds(&all) {
            self.view = Viewport::fit(self.view.size, b, 40.0);
            self.ui_rev += 1;
        }
    }

    /// Fit the selection into the viewport.
    pub fn view_selected(&mut self) {
        let sel = self.selection.to_vec();
        if let Some(b) = self.doc.bounds(&sel) {
            self.view = Viewport::fit(self.view.size, b, 80.0);
            self.ui_rev += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::test_support::registry;

    pub(crate) fn editor() -> Editor {
        Editor::new(registry())
    }

    #[test]
    fn layout_follows_widgets_and_connections() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let b = e.add_node("t.filter", vec2(400.0, 0.0)).unwrap();
        // fields are one standard row, so nodes keep uniform rows
        assert!(
            e.doc
                .graph
                .node(e.doc.id_of(b).unwrap())
                .unwrap()
                .rows()
                .is_empty()
        );
        assert_eq!(e.connect((a, 0), (b, 0)), Feedback::Ok);
        assert_eq!(e.doc.all_links().len(), 1);
    }

    #[test]
    fn check_link_reports_why() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let n = e.add_node("t.num", vec2(0.0, 200.0)).unwrap();
        let f = e.add_node("t.filter", vec2(400.0, 0.0)).unwrap();
        assert_eq!(
            e.check_link(Link {
                from: (a, 0),
                to: (f, 0)
            }),
            Feedback::Ok
        );
        assert_eq!(
            e.check_link(Link {
                from: (a, 0),
                to: (a, 0)
            }),
            Feedback::SameNode
        );
        // float output into an image input
        assert_eq!(
            e.check_link(Link {
                from: (n, 0),
                to: (f, 0)
            }),
            Feedback::TypeMismatch
        );
        assert_eq!(
            e.check_link(Link {
                from: (a, 5),
                to: (f, 0)
            }),
            Feedback::BadPin
        );
        e.connect((a, 0), (f, 0));
        let f2 = e.add_node("t.filter", vec2(800.0, 0.0)).unwrap();
        e.connect((f, 0), (f2, 0));
        assert_eq!(
            e.check_link(Link {
                from: (f2, 0),
                to: (f, 0)
            }),
            Feedback::Cycle
        );
    }

    #[test]
    fn copy_paste_remaps_uids_and_internal_links() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(100.0, 100.0)).unwrap();
        let b = e.add_node("t.filter", vec2(400.0, 100.0)).unwrap();
        let c = e.add_node("t.sink", vec2(800.0, 100.0)).unwrap();
        e.connect((a, 0), (b, 0));
        e.connect((b, 0), (c, 0));
        e.selection.set_all(&[a, b]);
        e.copy();
        let new = e.paste(vec2(100.0, 500.0));
        assert_eq!(new.len(), 2);
        assert!(new.iter().all(|u| ![a, b, c].contains(u)));
        assert_eq!(e.doc.node_count(), 5);
        assert_eq!(e.doc.all_links().len(), 3); // 2 old + the internal copy; b->c is not copied
        assert_eq!(e.doc.pos(new[0]), Some(vec2(100.0, 500.0)));
        assert_eq!(e.selection.to_vec(), new);
        assert!(e.undo());
        assert_eq!(e.doc.node_count(), 3);
        assert!(e.redo());
        assert_eq!(e.doc.node_count(), 5);
    }

    #[test]
    fn duplicate_delete_and_undo() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        e.selection.set_only(a);
        let d = e.duplicate();
        assert_eq!(e.doc.pos(d[0]), Some(vec2(20.0, 20.0)));
        e.delete_selection();
        assert_eq!(e.doc.node_count(), 1);
        e.undo();
        assert_eq!(e.doc.node_count(), 2);
        assert!(e.doc.contains(d[0]));
    }

    #[test]
    fn multi_input_accepts_several_wires() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let b = e.add_node("t.src", vec2(0.0, 200.0)).unwrap();
        let m = e.add_node("t.multi", vec2(600.0, 0.0)).unwrap();
        assert_eq!(e.connect((a, 0), (m, 0)), Feedback::Ok);
        assert_eq!(e.connect((b, 0), (m, 0)), Feedback::Ok);
        assert_eq!(e.doc.input_links(m, 0).len(), 2);
    }

    #[test]
    fn menu_add_connects_first_compatible_socket() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let f = e
            .add_from_menu(
                "t.filter",
                vec2(400.0, 0.0),
                Some(MenuConnect::FromOut(a, 0)),
            )
            .unwrap();
        assert_eq!(
            e.doc.all_links(),
            vec![Link {
                from: (a, 0),
                to: (f, 0)
            }]
        );
        let s = e.add_node("t.sink", vec2(900.0, 0.0)).unwrap();
        let g = e
            .add_from_menu("t.filter", vec2(600.0, 0.0), Some(MenuConnect::ToIn(s, 0)))
            .unwrap();
        assert_eq!(
            e.doc.input_link(s, 0),
            Some(Link {
                from: (g, 0),
                to: (s, 0)
            })
        );
        // one undo step per menu add
        e.undo();
        assert!(!e.doc.contains(g));
        assert_eq!(e.doc.input_link(s, 0), None);
    }

    #[test]
    fn mute_collapse_reroute_and_frame() {
        let mut e = editor();
        let a = e.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        let f = e.add_node("t.filter", vec2(400.0, 0.0)).unwrap();
        e.connect((a, 0), (f, 0));
        e.selection.set_only(f);
        e.toggle_mute();
        assert!(e.doc.data(f).unwrap().muted);
        e.toggle_collapse();
        let n = e.doc.graph.node(e.doc.id_of(f).unwrap()).unwrap();
        assert!(n.data.collapsed);
        assert!(n.height() < 60.0);
        e.undo();
        e.undo();
        assert!(!e.doc.data(f).unwrap().muted);

        let link = e.doc.all_links()[0];
        let r = e.insert_reroute(link, vec2(250.0, 20.0)).unwrap();
        assert_eq!(e.doc.all_links().len(), 2);
        assert!(e.doc.data(r).unwrap().is_reroute());
        e.undo();
        assert_eq!(e.doc.all_links(), vec![link]);

        e.selection.set_all(&[a, f]);
        let fr = e.frame_selected().unwrap();
        let b = e.doc.bounds(&[a, f]).unwrap();
        let fb = e.doc.bounds(&[fr]).unwrap();
        assert!(fb.contains_rect(&b));
    }

    #[test]
    fn view_all_fits_and_draw_key_changes() {
        let mut e = editor();
        e.add_node("t.src", vec2(0.0, 0.0));
        e.add_node("t.sink", vec2(3000.0, 2000.0));
        let k0 = e.draw_key();
        e.view_all();
        let r = e.view.visible_rect(0.0);
        let all = e.doc.bounds(&e.doc.uids()).unwrap();
        assert!(r.contains_rect(&all));
        assert_ne!(e.draw_key(), k0);
        assert_eq!(e.visible_uids().len(), 2);
    }
}
