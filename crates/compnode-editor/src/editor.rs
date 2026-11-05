//! The bridge between egui and the headless editor.
//!
//! [`CompnodeEditor`] owns a [`compnode_ui::Editor`] (the document, the
//! interaction state machine and the undo history), the node instances and
//! the evaluator. [`CompnodeEditor::draw`] renders one frame and feeds the
//! pointer and keyboard back into the editor, so a host only has to place it
//! in a panel:
//!
//! ```
//! # use compnode_editor::CompnodeEditor;
//! fn draw(ui: &mut egui::Ui) {
//!     let mut editor = CompnodeEditor::new();
//!     egui::CentralPanel::default().show(ui, |ui| {
//!         editor.draw(ui);
//!     });
//! }
//! ```
use crate::eval::{Evaluator, Nodes};
use crate::node::CompnodeNode;
use crate::paint::{self, Xf};
use crate::types::NodeTypes;
use compnode_core::Vec2;
use compnode_core::geom::vec2;
use compnode_core::hash::FxMap;
use compnode_ui::document::{Change, Document};
use compnode_ui::draw::DrawList;
use compnode_ui::editor::{DrawKey, Editor, EditorAction, MenuConnect, ZOOM_STEP};
use compnode_ui::input::{InputEvent, Key, Modifiers, PointerButton};
use compnode_ui::theme::Theme;
use eframe::egui;

/// The add-node menu.
struct Menu {
    /// Where the menu is drawn (screen space).
    at: egui::Pos2,
    /// Where the new node goes (graph space).
    graph_pos: Vec2,
    /// The pin the new node should be wired to, if the menu was opened from
    /// one.
    connect: Option<MenuConnect>,
    query: String,
}

/// The inline text field a text widget asks the host for.
struct TextEdit {
    buf: String,
    uid: u64,
    slot: usize,
    output: bool,
}

/// Cached geometry of the graph: rebuilt only when the draw key changes.
struct ShapeCache {
    key: (DrawKey, u32, u32, u32, u64),
    origin: Vec2,
    shapes: Vec<egui::Shape>,
}

/// The editor: a graph of user-defined nodes, drawn with egui.
pub struct CompnodeEditor {
    ed: Editor,
    types: NodeTypes,
    nodes: Nodes,
    ev: Evaluator,
    cache: Option<ShapeCache>,
    menu: Option<Menu>,
    text_edit: Option<TextEdit>,
    /// The rectangle the graph was drawn in last frame.
    rect: egui::Rect,
    /// Textures for the image previews, by image handle.
    textures: std::collections::HashMap<u64, egui::TextureId>,
    /// Bumped whenever the texture map changes, so the shape cache is
    /// rebuilt.
    texture_rev: u64,
}

impl Default for CompnodeEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl CompnodeEditor {
    /// An editor with no node types registered yet.
    pub fn new() -> CompnodeEditor {
        CompnodeEditor {
            ed: Editor::new(compnode_ui::schema::Registry::new()),
            types: NodeTypes::new(),
            nodes: FxMap::default(),
            ev: Evaluator::new((512, 512)),
            cache: None,
            menu: None,
            text_edit: None,
            rect: egui::Rect::NOTHING,
            textures: std::collections::HashMap::new(),
            texture_rev: 0,
        }
    }

    // ───────────── registration ─────────────

    /// Register a node type. The node's schema is taken from a default
    /// instance, so the node shows up in the add-node menu right away.
    pub fn register<T: CompnodeNode + Default>(&mut self) -> &mut Self {
        self.types.register::<T>();
        self.publish_schemas();
        self
    }

    /// Register a node type with a custom constructor.
    pub fn register_with<T: CompnodeNode>(
        &mut self,
        make: impl Fn() -> T + Send + Sync + 'static,
    ) -> &mut Self {
        self.types.register_with::<T>(make);
        self.publish_schemas();
        self
    }

    /// Copy the schemas of every registered node into the document's
    /// registry, so the add-node menu knows about them.
    fn publish_schemas(&mut self) {
        self.types.fill_registry(&mut self.ed.doc.registry);
    }

    /// The registered node types.
    pub fn types(&self) -> &NodeTypes {
        &self.types
    }

    // ───────────── state ─────────────

    /// The document: the graph, the node parameters and the undo history.
    pub fn document(&self) -> &Document {
        &self.ed.doc
    }

    /// The document, for hosts that need to load a project.
    pub fn document_mut(&mut self) -> &mut Document {
        &mut self.ed.doc
    }

    /// The headless editor underneath.
    pub fn editor(&self) -> &Editor {
        &self.ed
    }

    /// The headless editor underneath, for hosts that need to drive it.
    pub fn editor_mut(&mut self) -> &mut Editor {
        &mut self.ed
    }

    /// Open the add-node menu in the middle of the view.
    pub fn open_add_menu(&mut self) {
        let center = self.ed.view.size * 0.5;
        let pos = self.ed.view.to_graph(center);
        self.ed.open_add_menu_at(pos, None);
    }

    /// Frame the whole graph in the view.
    pub fn view_all(&mut self) {
        self.ed.view_all();
        self.cache = None;
    }

    /// Frame the selection in the view.
    pub fn view_selected(&mut self) {
        self.ed.view_selected();
        self.cache = None;
    }

    /// The live node instances, by uid.
    pub fn nodes(&self) -> &Nodes {
        &self.nodes
    }

    /// One node instance.
    pub fn node(&self, uid: u64) -> Option<&dyn CompnodeNode> {
        self.nodes.get(&uid).map(|n| n.as_ref())
    }

    /// One node instance, mutably.
    pub fn node_mut(&mut self, uid: u64) -> Option<&mut Box<dyn CompnodeNode>> {
        self.nodes.get_mut(&uid)
    }

    /// The evaluator (results, errors, dirty set).
    pub fn evaluator(&self) -> &Evaluator {
        &self.ev
    }

    /// The size of the canvas the graph works on.
    pub fn canvas(&self) -> (u32, u32) {
        self.ev.canvas()
    }

    /// Change the canvas size; every result is dropped.
    pub fn set_canvas(&mut self, canvas: (u32, u32)) {
        self.ev.set_canvas(canvas);
    }

    /// Replace the theme.
    pub fn set_theme(&mut self, theme: Theme) {
        self.ed.set_theme(theme);
        self.cache = None;
    }

    /// Undo the last edit.
    pub fn undo(&mut self) -> bool {
        let r = self.ed.undo();
        self.reconcile();
        r
    }

    /// Redo the last undone edit.
    pub fn redo(&mut self) -> bool {
        let r = self.ed.redo();
        self.reconcile();
        r
    }

    /// Add a node of the registered type `id` at `pos`.
    pub fn add_node(&mut self, id: &str, pos: Vec2) -> Option<u64> {
        let uid = self.ed.add_node(id, pos);
        self.reconcile();
        uid
    }

    /// Evaluate the graph: recompute every node whose inputs changed and
    /// leave the rest alone.
    ///
    /// [`CompnodeEditor::draw`] calls this once per frame; a host that
    /// drives the editor itself can call it whenever it wants the results
    /// to be up to date.
    pub fn evaluate(&mut self) {
        self.reconcile();
        self.ev.evaluate_all(&self.ed.doc, &mut self.nodes);
    }

    // ───────────── drawing ─────────────

    /// Draw one frame of the editor into `ui`.
    ///
    /// The whole available area is used for the graph. Pointer and keyboard
    /// events are taken from the egui context, so the host does not have to
    /// forward them.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, egui::Sense::hover());
        self.rect = rect;
        self.ed.set_viewport_size(vec2(rect.width(), rect.height()));

        self.feed_input(ui, rect);
        self.ed.sync();
        self.pump_actions();
        self.evaluate();
        self.paint(ui, rect);
        self.text_overlay(ui);
        self.menu_overlay(ui);
    }

    /// The rectangle the graph was drawn in last frame.
    pub fn rect(&self) -> egui::Rect {
        self.rect
    }

    // ───────────── image previews ─────────────

    /// Show an image in a preview row of a node.
    ///
    /// `slot` is the index of the row: output sockets come first, then the
    /// footer rows. `handle` identifies the image; the host owns the pixels
    /// and puts the matching texture into [`CompnodeEditor::set_textures`].
    pub fn set_preview(
        &mut self,
        uid: u64,
        slot: usize,
        handle: Option<compnode_ui::value::ImageHandle>,
        aspect: f32,
    ) {
        self.ed.set_preview(uid, slot, handle, aspect);
        self.cache = None;
    }

    /// The textures the graph draws, by image handle.
    pub fn textures(&self) -> &std::collections::HashMap<u64, egui::TextureId> {
        &self.textures
    }

    /// Replace the textures the graph draws. The host uploads them and hands
    /// the ids over; the editor only needs them to fill the image prims.
    pub fn set_textures(&mut self, textures: std::collections::HashMap<u64, egui::TextureId>) {
        self.textures = textures;
        self.texture_rev += 1;
        self.cache = None;
    }

    // ───────────── input ─────────────

    fn feed_input(&mut self, ui: &egui::Ui, rect: egui::Rect) {
        let ctx = ui.ctx().clone();
        let events = ctx.input(|i| i.events.clone());
        let hover = ctx.input(|i| i.pointer.hover_pos());
        let typing = ctx.egui_wants_keyboard_input();
        let over = |p: egui::Pos2| rect.contains(p);
        let local = |p: egui::Pos2| vec2(p.x - rect.min.x, p.y - rect.min.y);
        for ev in events {
            match ev {
                egui::Event::PointerMoved(p) => {
                    if over(p) {
                        self.ed.handle_event(&InputEvent::PointerMove {
                            pos: local(p),
                            mods: Modifiers::NONE,
                        });
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } => {
                    let Some(button) = map_button(button) else {
                        continue;
                    };
                    let mods = map_mods(&modifiers);
                    if pressed && over(pos) {
                        self.ed.handle_event(&InputEvent::PointerDown {
                            pos: local(pos),
                            button,
                            mods,
                        });
                    } else if !pressed {
                        self.ed.handle_event(&InputEvent::PointerUp {
                            pos: local(pos),
                            button,
                            mods,
                        });
                    }
                }
                egui::Event::MouseWheel { unit, delta, .. } => {
                    if let Some(p) = hover.filter(|p| over(*p)) {
                        // A wheel notch arrives as one "line" (winit sends
                        // LineDelta ±1 on Windows); pixels are about 50 per
                        // line, pages about 10 lines.
                        let lines = match unit {
                            egui::MouseWheelUnit::Point => delta.y / 50.0,
                            egui::MouseWheelUnit::Line => delta.y,
                            egui::MouseWheelUnit::Page => delta.y * 10.0,
                        };
                        self.ed.handle_event(&InputEvent::Scroll {
                            pos: local(p),
                            delta: vec2(0.0, lines),
                            mods: Modifiers::NONE,
                        });
                    }
                }
                egui::Event::Zoom(f) => {
                    if let Some(p) = hover.filter(|p| over(*p)) {
                        let lines = f.ln() / ZOOM_STEP.ln();
                        self.ed.handle_event(&InputEvent::Scroll {
                            pos: local(p),
                            delta: vec2(0.0, lines),
                            mods: Modifiers::NONE,
                        });
                    }
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if !typing && hover.is_some_and(over) => {
                    if let Some(k) = map_key(key) {
                        self.ed.handle_event(&InputEvent::Key {
                            key: k,
                            mods: map_mods(&modifiers),
                        });
                    }
                }
                egui::Event::Copy if !typing => self.ed.handle_event(&InputEvent::Key {
                    key: Key::C,
                    mods: Modifiers::CTRL,
                }),
                egui::Event::Cut if !typing => {
                    self.ed.handle_event(&InputEvent::Key {
                        key: Key::C,
                        mods: Modifiers::CTRL,
                    });
                    self.ed.handle_event(&InputEvent::Key {
                        key: Key::Delete,
                        mods: Modifiers::NONE,
                    });
                }
                egui::Event::Paste(_) if !typing => self.ed.handle_event(&InputEvent::Key {
                    key: Key::V,
                    mods: Modifiers::CTRL,
                }),
                _ => {}
            }
        }
    }

    // ───────────── actions ─────────────

    fn pump_actions(&mut self) {
        for a in self.ed.drain_actions() {
            match a {
                EditorAction::OpenAddMenu { graph_pos, connect } => {
                    let at = self.to_screen(graph_pos);
                    self.menu = Some(Menu {
                        at,
                        graph_pos,
                        connect,
                        query: String::new(),
                    });
                }
                EditorAction::OpenContextMenu { .. } => {}
                EditorAction::ButtonPressed { uid, input } => self.press_button(uid, input),
                EditorAction::TextEdit { uid, slot, request } => {
                    self.text_edit = Some(TextEdit {
                        buf: request.text,
                        uid,
                        slot,
                        output: false,
                    });
                }
            }
        }
    }

    /// A button in a node body was pressed: hand it to the node.
    ///
    /// The headless editor reports presses; a host that drives the editor
    /// itself can call this directly. `slot` is the index of the row: output
    /// sockets come first, then the footer rows.
    pub fn press_button(&mut self, uid: u64, slot: usize) {
        let Some(schema) = self.ed.doc.schema_of(uid) else {
            return;
        };
        // Footer rows live after the output sockets.
        let Some(index) = slot.checked_sub(schema.outputs.len()) else {
            return;
        };
        let Some(def) = schema.footer.get(index) else {
            return;
        };
        let key = def.name.clone();
        if let Some(node) = self.nodes.get_mut(&uid) {
            node.on_button(&key);
        }
        // the node changed: recompute it and everything downstream
        self.mark_dirty(uid);
    }

    /// Mark a node and everything downstream of it for recomputation.
    ///
    /// A host that changes a node's state by hand (through
    /// [`CompnodeEditor::node_mut`]) calls this so the change is picked up.
    pub fn mark_dirty(&mut self, uid: u64) {
        self.ev.mark_downstream(&self.ed.doc, uid);
    }

    // ───────────── node instances ─────────────

    /// Bring the node instances in line with the document: create the ones
    /// that were added, drop the ones that were removed and mirror the
    /// parameter values into the nodes.
    fn reconcile(&mut self) {
        let changes = self.ed.doc.take_changes();
        for c in &changes {
            match c {
                Change::Value { uid, input } => self.sync_param(*uid, *input),
                Change::Structure { uids } => {
                    for &u in uids {
                        if self.ed.doc.contains(u) {
                            if !self.nodes.contains_key(&u) {
                                self.create_node(u);
                            }
                        } else {
                            self.nodes.remove(&u);
                            self.ev.forget(u);
                        }
                    }
                }
                Change::Flag { .. } => {}
            }
        }
        self.ev.apply_changes(&self.ed.doc, &changes);
    }

    /// Make a fresh instance of a node and mirror the document's values into
    /// it, so the node starts in the state the document describes.
    fn create_node(&mut self, uid: u64) {
        let Some(schema_id) = self.ed.doc.data(uid).map(|d| d.schema.clone()) else {
            return;
        };
        let Some(ty) = self.types.get(&schema_id) else {
            return;
        };
        let mut node = ty.create();
        let values = self
            .ed
            .doc
            .data(uid)
            .map(|d| d.values.clone())
            .unwrap_or_default();
        if let Some(schema) = self.ed.doc.schema_of(uid) {
            for (i, def) in schema.inputs.iter().enumerate() {
                if let Some(v) = values.get(i) {
                    node.set_param(&def.name, v.clone());
                }
            }
        }
        self.nodes.insert(uid, node);
    }

    /// Copy one parameter value into the node instance.
    fn sync_param(&mut self, uid: u64, input: usize) {
        let Some(value) = self
            .ed
            .doc
            .data(uid)
            .and_then(|d| d.values.get(input))
            .cloned()
        else {
            return;
        };
        let Some(key) = self
            .ed
            .doc
            .schema_of(uid)
            .and_then(|s| s.inputs.get(input))
            .map(|d| d.name.clone())
        else {
            return;
        };
        if let Some(node) = self.nodes.get_mut(&uid) {
            node.set_param(&key, value);
        }
    }

    // ───────────── painting ─────────────

    fn to_screen(&self, p: Vec2) -> egui::Pos2 {
        let s = self.ed.view.to_screen(p);
        egui::pos2(self.rect.min.x + s.x, self.rect.min.y + s.y)
    }

    fn paint(&mut self, ui: &egui::Ui, rect: egui::Rect) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(
            rect,
            0.0,
            paint::c32(self.ed.theme.c(compnode_ui::theme::Col::Background)),
        );
        let zoom = self.ed.view.zoom;
        let key = (
            self.ed.draw_key(),
            zoom.to_bits(),
            rect.width().to_bits(),
            rect.height().to_bits(),
            self.texture_rev,
        );
        if self.cache.as_ref().is_none_or(|c| c.key != key) {
            let mut dl = DrawList::new();
            self.ed.draw(&mut dl);
            let origin = self.ed.view.origin;
            let shapes = paint::build(&painter, &dl, Xf { origin, zoom }, &self.textures);
            self.cache = Some(ShapeCache {
                key,
                origin,
                shapes,
            });
        }
        if let Some(c) = &self.cache {
            let d = (c.origin - self.ed.view.origin) * zoom;
            let delta = egui::vec2(rect.min.x + d.x, rect.min.y + d.y);
            for s in &c.shapes {
                let mut s = s.clone();
                s.translate(delta);
                painter.add(s);
            }
        }
    }

    // ───────────── overlays ─────────────

    fn text_overlay(&mut self, ui: &egui::Ui) {
        let Some(te) = self.text_edit.take() else {
            return;
        };
        let mut te = te;
        let rect = self.ed.widget_rect(te.uid, te.slot, te.output);
        let at = rect.map_or(self.rect.center(), |r| self.to_screen(r.min));
        let mut done = None;
        egui::Area::new(egui::Id::new("compnode-text-edit"))
            .fixed_pos(at)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                let response = ui.text_edit_singleline(&mut te.buf);
                if response.lost_focus() && !response.has_focus() {
                    done = Some(false);
                }
            });
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if enter {
            done = Some(true);
        } else if escape {
            done = Some(false);
        }
        match done {
            Some(true) => self.ed.commit_text(&te.buf),
            Some(false) => self.ed.cancel_text(),
            None => self.text_edit = Some(te),
        }
    }

    fn menu_overlay(&mut self, ui: &egui::Ui) {
        let Some(menu) = self.menu.take() else {
            return;
        };
        let mut menu = menu;
        let mut picked: Option<String> = None;
        egui::Area::new(egui::Id::new("compnode-add-menu"))
            .fixed_pos(menu.at)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.text_edit_singleline(&mut menu.query);
                    let hits: Vec<String> = self
                        .ed
                        .doc
                        .registry
                        .search(&menu.query)
                        .iter()
                        .map(|s| s.id.clone())
                        .collect();
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| {
                            for id in hits {
                                let title = self
                                    .ed
                                    .doc
                                    .registry
                                    .get(&id)
                                    .map(|s| s.title.clone())
                                    .unwrap_or_else(|| id.clone());
                                if ui.button(title).clicked() {
                                    picked = Some(id);
                                }
                            }
                        });
                });
            });
        if let Some(id) = picked {
            self.ed.add_from_menu(&id, menu.graph_pos, menu.connect);
            self.reconcile();
        } else {
            self.menu = Some(menu);
        }
    }
}

fn map_button(b: egui::PointerButton) -> Option<PointerButton> {
    Some(match b {
        egui::PointerButton::Primary => PointerButton::Primary,
        egui::PointerButton::Secondary => PointerButton::Secondary,
        egui::PointerButton::Middle => PointerButton::Middle,
        _ => return None,
    })
}

fn map_mods(m: &egui::Modifiers) -> Modifiers {
    Modifiers {
        shift: m.shift,
        ctrl: m.ctrl || m.command,
        alt: m.alt,
    }
}

fn map_key(key: egui::Key) -> Option<Key> {
    use egui::Key as E;
    Some(match key {
        E::A => Key::A,
        E::C => Key::C,
        E::D => Key::D,
        E::F => Key::F,
        E::H => Key::H,
        E::I => Key::I,
        E::M => Key::M,
        E::R => Key::R,
        E::V => Key::V,
        E::X => Key::X,
        E::Y => Key::Y,
        E::Z => Key::Z,
        E::Delete => Key::Delete,
        E::Backspace => Key::Backspace,
        E::Escape => Key::Escape,
        E::Enter => Key::Enter,
        E::Space => Key::Space,
        E::Tab => Key::Tab,
        E::ArrowLeft => Key::Left,
        E::ArrowRight => Key::Right,
        E::ArrowUp => Key::Up,
        E::ArrowDown => Key::Down,
        E::Home => Key::Home,
        E::End => Key::End,
        E::Period => Key::Period,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use compnode_ui::schema::{Category, SocketDef};
    use compnode_ui::value::{SocketType, Value};

    #[derive(Default)]
    struct Counter {
        count: f32,
    }

    impl CompnodeNode for Counter {
        fn id(&self) -> &'static str {
            "t.counter"
        }
        fn title(&self) -> &str {
            "Counter"
        }
        fn category(&self) -> Category {
            Category::Utility
        }
        fn inputs(&self) -> Vec<SocketDef> {
            vec![SocketDef::float("Step", self.count).range(0.0, 10.0)]
        }
        fn outputs(&self) -> Vec<SocketDef> {
            vec![SocketDef::new("Value", SocketType::Float)]
        }
        fn process(
            &mut self,
            ctx: &mut crate::node::ProcessCtx,
        ) -> Result<Vec<crate::node::Payload>, crate::node::NodeError> {
            self.count = ctx.float(0);
            Ok(vec![crate::node::Payload::value(Value::Float(self.count))])
        }
        fn set_param(&mut self, key: &str, v: Value) -> bool {
            match key {
                "Step" => {
                    self.count = v.as_f32().unwrap_or(self.count);
                    true
                }
                _ => false,
            }
        }
    }

    #[test]
    fn registering_makes_the_node_known() {
        let mut ed = CompnodeEditor::new();
        ed.register::<Counter>();
        assert_eq!(ed.types().len(), 1);
        assert!(ed.document().registry.get("t.counter").is_some());
    }

    #[test]
    fn adding_a_node_makes_an_instance_and_evaluates_it() {
        let mut ed = CompnodeEditor::new();
        ed.register::<Counter>();
        let uid = ed.add_node("t.counter", vec2(0.0, 0.0)).unwrap();
        assert!(ed.node(uid).is_some());
        ed.evaluate();
        let out = ed.evaluator().outputs(uid).unwrap();
        assert_eq!(out[0].cloned::<Value>(), Some(Value::Float(0.0)));
    }

    #[test]
    fn a_parameter_edit_reaches_the_node_instance() {
        let mut ed = CompnodeEditor::new();
        ed.register::<Counter>();
        let uid = ed.add_node("t.counter", vec2(0.0, 0.0)).unwrap();
        ed.ed.set_value(uid, 0, Value::Float(3.5));
        ed.evaluate();
        // the node mirrors the value the user typed in
        let out = ed.evaluator().outputs(uid).unwrap();
        assert_eq!(out[0].cloned::<Value>(), Some(Value::Float(3.5)));
    }
}
