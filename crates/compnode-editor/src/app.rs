//! The egui application: the node-graph canvas and the popups that belong to it.
//!
//! The window holds one thing: the graph. Its minimap, the add-node menu, the
//! right-click context menu and the inline text field are the only chrome.
//! Nothing is evaluated here — the graph is edited, not processed.
use crate::Args;
use compnode_core::Rect as GRect;
use compnode_core::geom::vec2;
use compnode_editor::paint::{self, Xf};
use compnode_ui::EditorAction;
use compnode_ui::draw::DrawList;
use compnode_ui::editor::focus::AccessRole;
use compnode_ui::editor::{DrawKey, Editor, Interaction, MenuConnect, ZOOM_STEP};
use compnode_ui::input::{InputEvent, Key, Modifiers, PointerButton};
use compnode_ui::minimap::Minimap;
use eframe::egui;
use egui::{Color32, pos2};
use std::collections::HashMap;
use std::sync::Arc;

/// Nerd Font (private-use) glyphs for the GUI icons. Codepoints are from the
/// Material Design Icons set (`nf-md-*`) that 3270 Nerd Font carries.
const ICON_ADD: &str = "\u{f0415}"; // nf-md-plus
const ICON_DUPLICATE: &str = "\u{f018f}"; // nf-md-content_copy
const ICON_FRAME: &str = "\u{f0f9c}"; // nf-md-arrow_expand
const ICON_MUTE: &str = "\u{f058f}"; // nf-md-eye_off
const ICON_DELETE: &str = "\u{f0a79}"; // nf-md-trash_can

/// Blender 5.x UI palette for the host chrome (popups and widgets).
mod bl {
    use eframe::egui::Color32;
    /// Gap between areas and the darkest region background.
    pub const GAP: Color32 = Color32::from_rgb(0x1d, 0x1d, 0x1d);
    /// Properties editor background.
    pub const PANEL: Color32 = Color32::from_rgb(0x30, 0x30, 0x30);
    pub const WIDGET: Color32 = Color32::from_rgb(0x54, 0x54, 0x54);
    pub const WIDGET_HOVER: Color32 = Color32::from_rgb(0x65, 0x65, 0x65);
    pub const POPUP: Color32 = Color32::from_rgb(0x1d, 0x1d, 0x1d);
    pub const ACCENT: Color32 = Color32::from_rgb(0x47, 0x72, 0xb3);
    pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe6, 0xe6);
}

/// The shared look for floating menus (add-node menu, context menu): a rounded
/// dark panel with a soft drop shadow, matching the graph's theme.
fn menu_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(bl::POPUP)
        .stroke(egui::Stroke::new(1.0, Color32::BLACK))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(6))
        .shadow(egui::epaint::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(140),
        })
}

/// Cached geometry of the graph: rebuilt only when the draw key changes.
struct ShapeCache {
    key: (DrawKey, u32, u32, u32),
    origin: compnode_core::Vec2,
    shapes: Vec<egui::Shape>,
}

struct TextEditUi {
    buf: String,
    first: bool,
}

struct MenuUi {
    screen: egui::Pos2,
    graph_pos: compnode_core::Vec2,
    connect: Option<MenuConnect>,
    query: String,
    first: bool,
}

struct ContextUi {
    screen: egui::Pos2,
    graph_pos: compnode_core::Vec2,
    context: Option<u64>,
    /// Currently highlighted menu row (keyboard navigation / hover).
    highlight: usize,
}

/// An action offered by the right-click context menu.
#[derive(Clone)]
enum ContextAction {
    Add {
        schema: String,
        pos: compnode_core::Vec2,
    },
    AddHere {
        pos: compnode_core::Vec2,
    },
    Duplicate,
    Delete,
    Mute,
    Frame,
}

/// Scripted pan/zoom benchmark over a generated graph.
struct Bench {
    target: usize,
    samples: Vec<f32>,
    nodes: usize,
}

pub struct App {
    ed: Editor,
    cache: Option<ShapeCache>,
    graph_rect: egui::Rect,
    show_minimap: bool,
    minimap_drag: bool,
    text_edit: Option<TextEditUi>,
    menu: Option<MenuUi>,
    context_menu: Option<ContextUi>,
    bench: Option<Bench>,
}

fn map_key(k: egui::Key) -> Option<Key> {
    use egui::Key as E;
    Some(match k {
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

fn map_mods(m: &egui::Modifiers) -> Modifiers {
    Modifiers {
        shift: m.shift,
        ctrl: m.ctrl || m.command,
        alt: m.alt,
    }
}

fn map_button(b: egui::PointerButton) -> Option<PointerButton> {
    match b {
        egui::PointerButton::Primary => Some(PointerButton::Primary),
        egui::PointerButton::Secondary => Some(PointerButton::Secondary),
        egui::PointerButton::Middle => Some(PointerButton::Middle),
        _ => None,
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, args: &Args) -> App {
        let mut ed = Editor::new(compnode_image::registry());
        if let Some(p) = &args.project {
            match Self::load_project(p) {
                Ok(doc) => ed.replace_document(doc),
                Err(e) => eprintln!("could not open {}: {e}", p.display()),
            }
        }
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        setup_fonts(&cc.egui_ctx);
        setup_style(&cc.egui_ctx);
        let bench = args.bench.map(|n| Bench {
            target: 240,
            samples: Vec::new(),
            nodes: n,
        });
        let mut app = App {
            ed,
            cache: None,
            graph_rect: egui::Rect::NOTHING,
            show_minimap: true,
            minimap_drag: false,
            text_edit: None,
            menu: None,
            context_menu: None,
            bench,
        };
        if let Some(n) = app.bench.as_ref().map(|b| b.nodes) {
            app.build_bench_graph(n);
        }
        app
    }

    // ───────────── helpers ─────────────

    /// Graph position to window position.
    fn to_screen(&self, p: compnode_core::Vec2) -> egui::Pos2 {
        let s = self.ed.view.to_screen(p);
        pos2(self.graph_rect.min.x + s.x, self.graph_rect.min.y + s.y)
    }

    /// A generated graph of image nodes, used by the benchmark.
    fn build_bench_graph(&mut self, n: usize) {
        let kinds = [
            "image.invert",
            "image.blur",
            "image.hue_sat",
            "image.brightness_contrast",
        ];
        let mut prev: Option<u64> = None;
        for i in 0..n {
            let (col, row) = (i / 10, i % 10);
            let pos = vec2(col as f32 * 260.0, row as f32 * 140.0);
            let uid = if row == 0 {
                self.ed.add_node("image.solid", pos)
            } else {
                self.ed.add_node(kinds[i % kinds.len()], pos)
            };
            if let (Some(a), Some(b)) = (prev.filter(|_| row != 0), uid) {
                self.ed.connect((a, 0), (b, 0));
            }
            prev = uid;
        }
        self.ed.selection.clear();
        self.ed.view_all();
    }

    /// Read a project file into a document.
    fn load_project(path: &std::path::Path) -> Result<compnode_ui::Document, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        compnode_ui::project::load(&text, compnode_image::registry())
    }

    // ───────────── input ─────────────

    fn feed_graph_input(&mut self, ctx: &egui::Context, rect: egui::Rect, layer: egui::LayerId) {
        let events = ctx.input(|i| i.events.clone());
        let hover = ctx.input(|i| i.pointer.hover_pos());
        let typing = ctx.egui_wants_keyboard_input();
        let over_graph = |p: egui::Pos2| rect.contains(p) && ctx.layer_id_at(p) == Some(layer);
        let local = |p: egui::Pos2| vec2(p.x - rect.min.x, p.y - rect.min.y);
        let mm = self.minimap_rect();
        for ev in events {
            let busy = *self.ed.interaction() != Interaction::Idle;
            match ev {
                egui::Event::PointerMoved(p) => {
                    if self.minimap_drag {
                        let mut m = Minimap::new(mm);
                        m.update(&self.ed.doc, &self.ed.view);
                        self.ed.minimap_click(&m, local(p));
                    } else if busy || over_graph(p) {
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
                    if pressed {
                        if self.show_minimap && over_graph(pos) && mm.contains(local(pos)) && !busy
                        {
                            self.minimap_drag = true;
                            let mut m = Minimap::new(mm);
                            m.update(&self.ed.doc, &self.ed.view);
                            self.ed.minimap_click(&m, local(pos));
                        } else if over_graph(pos) {
                            self.ed.handle_event(&InputEvent::PointerDown {
                                pos: local(pos),
                                button,
                                mods,
                            });
                        }
                    } else if self.minimap_drag {
                        self.minimap_drag = false;
                    } else if busy {
                        self.ed.handle_event(&InputEvent::PointerUp {
                            pos: local(pos),
                            button,
                            mods,
                        });
                    }
                }
                egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } => {
                    if let Some(p) = hover.filter(|p| over_graph(*p)) {
                        let lines = match unit {
                            egui::MouseWheelUnit::Point => delta.y / 50.0,
                            egui::MouseWheelUnit::Line => delta.y,
                            egui::MouseWheelUnit::Page => delta.y * 10.0,
                        };
                        self.ed.handle_event(&InputEvent::Scroll {
                            pos: local(p),
                            delta: vec2(0.0, lines),
                            mods: map_mods(&modifiers),
                        });
                    }
                }
                egui::Event::Zoom(f) => {
                    if let Some(p) = hover.filter(|p| over_graph(*p)) {
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
                } if !typing && (busy || hover.is_some_and(over_graph)) => {
                    if let Some(k) = map_key(key) {
                        self.ed.handle_event(&InputEvent::Key {
                            key: k,
                            mods: map_mods(&modifiers),
                        });
                    }
                }
                // egui turns Ctrl+C/X/V into these events instead of key presses
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

    /// The minimap's rectangle in graph space (bottom-right of the canvas).
    fn minimap_rect(&self) -> GRect {
        let size = vec2(200.0, 140.0);
        let br = vec2(
            self.graph_rect.width() - 12.0,
            self.graph_rect.height() - 12.0,
        );
        GRect::from_min_size(br - size, size)
    }

    /// A dropped file becomes a node (images) or a project (`.cnp`).
    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped.is_empty() {
            return;
        }
        let at = ctx
            .input(|i| i.pointer.hover_pos())
            .filter(|p| self.graph_rect.contains(*p));
        let mut gp = match at {
            Some(p) => self.ed.view.to_graph(vec2(
                p.x - self.graph_rect.min.x,
                p.y - self.graph_rect.min.y,
            )),
            None => self.ed.view.to_graph(self.ed.view.size * 0.5),
        };
        for f in dropped {
            let path = f.path().to_path_buf();
            if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("cnp"))
            {
                match Self::load_project(&path) {
                    Ok(doc) => {
                        self.ed.replace_document(doc);
                        self.cache = None;
                    }
                    Err(e) => eprintln!("could not open {}: {e}", path.display()),
                }
                return;
            }
            if let Some(uid) = self.ed.add_node("image.file", gp) {
                self.ed.set_value(
                    uid,
                    0,
                    compnode_ui::Value::String(path.to_string_lossy().into_owned()),
                );
                gp += vec2(0.0, 80.0);
            }
        }
    }

    // ───────────── frame ─────────────

    fn pump_actions(&mut self, _ctx: &egui::Context) {
        for a in self.ed.drain_actions() {
            match a {
                EditorAction::OpenAddMenu { graph_pos, connect } => {
                    let s = self.to_screen(graph_pos);
                    self.menu = Some(MenuUi {
                        screen: s,
                        graph_pos,
                        connect,
                        query: String::new(),
                        first: true,
                    });
                }
                EditorAction::OpenContextMenu { graph_pos, context } => {
                    let s = self.to_screen(graph_pos);
                    self.context_menu = Some(ContextUi {
                        screen: s,
                        graph_pos,
                        context,
                        highlight: 0,
                    });
                }
                // Buttons that open a file dialog belong to the processing side
                // of the app and do nothing here.
                EditorAction::ButtonPressed { .. } => {}
                EditorAction::TextEdit { .. } => {}
            }
        }
    }

    fn text_overlay(&mut self, ctx: &egui::Context) {
        let Some(p) = self.ed.pending_text_edit().cloned() else {
            self.text_edit = None;
            return;
        };
        let mut st = self.text_edit.take().unwrap_or(TextEditUi {
            buf: p.request.text.clone(),
            first: true,
        });
        let rect = self.ed.widget_rect(p.uid, p.slot, p.output);
        let at = rect.map_or(self.graph_rect.center(), |r| self.to_screen(r.min));
        let width = rect.map_or(160.0, |r| (r.width() * self.ed.view.zoom).max(120.0));
        let mut commit = false;
        let mut cancel = false;
        egui::Area::new(egui::Id::new("compnode-text-edit"))
            .fixed_pos(at)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                menu_frame().show(ui, |ui| {
                    let resp = if p.request.multiline {
                        ui.add(
                            egui::TextEdit::multiline(&mut st.buf)
                                .desired_width(width)
                                .desired_rows(4),
                        )
                    } else {
                        ui.add(egui::TextEdit::singleline(&mut st.buf).desired_width(width))
                    };
                    if st.first {
                        resp.request_focus();
                        st.first = false;
                    } else if resp.lost_focus() {
                        commit = true;
                    }
                    let (enter, esc) = ui.input(|i| {
                        let multi_ok =
                            !p.request.multiline || i.modifiers.ctrl || i.modifiers.command;
                        (
                            i.key_pressed(egui::Key::Enter) && multi_ok,
                            i.key_pressed(egui::Key::Escape),
                        )
                    });
                    if esc {
                        cancel = true;
                        commit = false;
                    } else if enter {
                        commit = true;
                    }
                });
            });
        if cancel {
            self.ed.cancel_text();
            self.text_edit = None;
        } else if commit {
            self.ed.commit_text(&st.buf);
            self.text_edit = None;
        } else {
            self.text_edit = Some(st);
        }
        ctx.request_repaint();
    }

    fn menu_overlay(&mut self, ctx: &egui::Context) {
        let Some(mut m) = self.menu.take() else {
            return;
        };
        let mut pick: Option<String> = None;
        let mut close = false;
        let items: Vec<(String, String)> = self
            .ed
            .doc
            .registry
            .search(&m.query)
            .into_iter()
            .map(|s| {
                (
                    s.id.clone(),
                    format!("{}  ({})", s.title, s.category.name()),
                )
            })
            .collect();
        let area = egui::Area::new(egui::Id::new("compnode-add-menu"))
            .fixed_pos(m.screen)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                menu_frame().show(ui, |ui| {
                    ui.set_width(240.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut m.query)
                            .hint_text("Search nodes")
                            .desired_width(f32::INFINITY),
                    );
                    if m.first {
                        resp.request_focus();
                        m.first = false;
                    }
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for (id, label) in &items {
                                if ui.selectable_label(false, label).clicked() {
                                    pick = Some(id.clone());
                                }
                            }
                        });
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        pick = items.first().map(|(id, _)| id.clone());
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close = true;
                    }
                });
            });
        let clicked_away =
            ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer() && !m.first;
        if let Some(id) = pick {
            self.ed.add_from_menu(&id, m.graph_pos, m.connect);
        } else if !close && !clicked_away {
            self.menu = Some(m);
        }
        ctx.request_repaint();
    }

    fn context_overlay(&mut self, ctx: &egui::Context) {
        let Some(mut c) = self.context_menu.take() else {
            return;
        };
        let mut close = false;
        let uid = c.context;
        let graph_pos = c.graph_pos;
        let screen = c.screen;
        let response = egui::Area::new(egui::Id::new("compnode-context-menu"))
            .fixed_pos(screen)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                menu_frame().show(ui, |ui| {
                    ui.set_width(220.0);
                    let has_ctx = uid.is_some_and(|u| self.ed.doc.contains(u));
                    let mut items: Vec<(ContextAction, &'static str, &'static str, &'static str)> =
                        Vec::new();
                    if has_ctx {
                        let Some(uid) = uid else { return ui.response() };
                        let schema = self
                            .ed
                            .doc
                            .data(uid)
                            .map(|d| d.schema.clone())
                            .unwrap_or_default();
                        items.push((
                            ContextAction::Add {
                                schema,
                                pos: graph_pos + vec2(80.0, -70.0),
                            },
                            ICON_ADD,
                            "Add node",
                            "Add",
                        ));
                        items.push((
                            ContextAction::Duplicate,
                            ICON_DUPLICATE,
                            "Duplicate",
                            "Ctrl+D",
                        ));
                        items.push((ContextAction::Delete, ICON_DELETE, "Delete", "Del"));
                        items.push((ContextAction::Mute, ICON_MUTE, "Mute", "M"));
                        items.push((ContextAction::Frame, ICON_FRAME, "Frame", "Frame"));
                    } else {
                        items.push((
                            ContextAction::AddHere { pos: graph_pos },
                            ICON_ADD,
                            "Add node",
                            "Shift+A",
                        ));
                    }
                    let count = items.len();
                    let mut idx = c.highlight.min(count.saturating_sub(1));
                    for (i, (op, icon, label, shortcut)) in items.iter().enumerate() {
                        let selected = i == idx;
                        let text = egui::RichText::new(format!("{icon}  {label}      {shortcut}"));
                        let row = ui.selectable_label(selected, text);
                        if row.clicked() {
                            self.context_action(op.clone());
                            close = true;
                        }
                    }
                    // Blender-style keyboard navigation: Up/Down move the
                    // highlight, Enter activates, Escape dismisses.
                    let (down, up, enter, esc) = ctx.input(|i| {
                        (
                            i.key_pressed(egui::Key::ArrowDown),
                            i.key_pressed(egui::Key::ArrowUp),
                            i.key_pressed(egui::Key::Enter),
                            i.key_pressed(egui::Key::Escape),
                        )
                    });
                    if down && idx + 1 < count {
                        idx += 1;
                        c.highlight = idx;
                    }
                    if up && idx > 0 {
                        idx -= 1;
                        c.highlight = idx;
                    }
                    if enter && !items.is_empty() && idx < items.len() {
                        self.context_action(items[idx].0.clone());
                        close = true;
                    }
                    if esc {
                        close = true;
                    }
                    ui.response()
                })
            });
        let clicked_away = ctx.input(|i| i.pointer.any_pressed())
            && !response
                .response
                .rect
                .contains(ctx.input(|i| i.pointer.hover_pos()).unwrap_or(screen));
        if !close && !clicked_away {
            self.context_menu = Some(c);
        }
        ctx.request_repaint();
    }

    /// Run a context-menu action (Blender-style: Add / Duplicate / Delete / ...).
    fn context_action(&mut self, op: ContextAction) {
        match op {
            ContextAction::Add { schema, pos } => {
                self.ed.add_node(&schema, pos);
            }
            ContextAction::AddHere { pos } => {
                self.ed.open_add_menu_at(pos, None);
            }
            ContextAction::Duplicate => {
                self.ed.duplicate();
            }
            ContextAction::Delete => {
                self.ed.delete_selection();
            }
            ContextAction::Mute => {
                self.ed.toggle_mute();
            }
            ContextAction::Frame => {
                self.ed.frame_selected();
            }
        }
    }

    /// Screen-reader descriptions of the nodes and widgets on screen.
    fn accessibility(&self, ui: &egui::Ui) {
        for item in self.ed.accessibility() {
            let r = egui::Rect::from_min_max(
                self.to_screen(item.rect.min),
                self.to_screen(item.rect.max),
            );
            let id = egui::Id::new(("compnode-a11y", item.uid, item.widget));
            let resp = ui.interact(r, id, egui::Sense::focusable_noninteractive());
            let wt = match item.role {
                AccessRole::Slider => egui::WidgetType::Slider,
                AccessRole::SpinButton => egui::WidgetType::DragValue,
                AccessRole::CheckBox => egui::WidgetType::Checkbox,
                AccessRole::Button => egui::WidgetType::Button,
                AccessRole::ComboBox => egui::WidgetType::ComboBox,
                AccessRole::TextInput => egui::WidgetType::TextEdit,
                AccessRole::ColorWell => egui::WidgetType::ColorButton,
                AccessRole::Image => egui::WidgetType::Image,
                AccessRole::Node | AccessRole::Frame | AccessRole::Label => egui::WidgetType::Label,
            };
            let label = if item.value.is_empty() {
                item.label.clone()
            } else {
                format!("{}: {}", item.label, item.value)
            };
            resp.widget_info(move || egui::WidgetInfo::labeled(wt, true, label.clone()));
        }
    }

    fn graph_panel(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| {
                let rect = ui.available_rect_before_wrap();
                ui.allocate_rect(rect, egui::Sense::hover());
                self.graph_rect = rect;
                self.ed.set_viewport_size(vec2(rect.width(), rect.height()));
                self.feed_graph_input(&ctx, rect, ui.layer_id());
                self.ed.sync();
                self.paint_graph(ui, rect);
                self.accessibility(ui);
            });
    }

    fn paint_graph(&mut self, ui: &egui::Ui, rect: egui::Rect) {
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
        );
        if self.cache.as_ref().is_none_or(|c| c.key != key) {
            let mut dl = DrawList::new();
            self.ed.draw(&mut dl);
            let origin = self.ed.view.origin;
            let shapes = paint::build(&painter, &dl, Xf { origin, zoom }, &HashMap::new());
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
        if self.show_minimap {
            let mm_rect = self.minimap_rect();
            let mut mm = Minimap::new(mm_rect);
            mm.update(&self.ed.doc, &self.ed.view);
            let mut dl = DrawList::new();
            mm.draw(&self.ed.doc, &self.ed.view, &self.ed.theme, &mut dl);
            let shapes = paint::build(&painter, &dl, Xf::IDENTITY, &HashMap::new());
            for mut s in shapes {
                s.translate(egui::vec2(rect.min.x, rect.min.y));
                painter.add(s);
            }
        }
    }

    /// Scripted pan/zoom, printing frame times when the run is over.
    fn run_bench_step(&mut self, ctx: &egui::Context, frame_ms: f32) {
        let Some(b) = &mut self.bench else { return };
        b.samples.push(frame_ms);
        let n = b.samples.len();
        let t = n as f32 * 0.05;
        let center = self.ed.view.size * 0.5;
        self.ed.view.zoom_at(center, 1.0 + 0.012 * t.sin());
        self.ed.view.pan(vec2(3.0, 2.0));
        self.ed.handle_event(&InputEvent::PointerMove {
            pos: vec2(100.0 + t.cos() * 80.0, 100.0),
            mods: Modifiers::NONE,
        });
        if n >= b.target {
            let mut s: Vec<f32> = b.samples.iter().copied().skip(20).collect();
            s.sort_by(|a, b| a.total_cmp(b));
            let avg = s.iter().sum::<f32>() / s.len().max(1) as f32;
            let p95 = s.get(s.len() * 95 / 100).copied().unwrap_or(0.0);
            println!(
                "bench: {} nodes, {} frames: avg {avg:.2} ms, p95 {p95:.2} ms, max {:.2} ms",
                b.nodes,
                s.len(),
                s.last().copied().unwrap_or(0.0)
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            ctx.request_repaint();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let t0 = std::time::Instant::now();
        self.handle_drops(&ctx);
        self.graph_panel(root);
        self.pump_actions(&ctx);
        self.text_overlay(&ctx);
        self.menu_overlay(&ctx);
        self.context_overlay(&ctx);
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        self.run_bench_step(&ctx, ms);
    }
}

/// Blender 5.x look for the host chrome: flat grey widgets without outlines,
/// a blue accent for hover/selection, tight spacing and 4 px corners.
fn setup_style(ctx: &egui::Context) {
    use egui::{CornerRadius, FontFamily, FontId, Stroke, TextStyle};

    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        v.panel_fill = bl::GAP;
        v.window_fill = bl::POPUP;
        v.extreme_bg_color = bl::GAP;
        v.faint_bg_color = Color32::from_rgb(0x2b, 0x2b, 0x2b);
        v.window_stroke = Stroke::new(1.0, Color32::BLACK);
        v.menu_corner_radius = CornerRadius::same(4);
        v.window_corner_radius = CornerRadius::same(4);
        v.window_shadow = egui::epaint::Shadow {
            offset: [0, 4],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(110),
        };
        v.popup_shadow = v.window_shadow;
        v.selection.bg_fill = bl::ACCENT;
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);

        let r = CornerRadius::same(4);
        let w = &mut v.widgets;
        for s in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.active,
            &mut w.open,
        ] {
            s.corner_radius = r;
            s.expansion = 0.0;
        }
        w.noninteractive.bg_fill = bl::PANEL;
        w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x1a, 0x1a, 0x1a));
        w.noninteractive.fg_stroke = Stroke::new(1.0, bl::TEXT);
        for (s, fill, fg) in [
            (&mut w.inactive, bl::WIDGET, bl::TEXT),
            (&mut w.hovered, bl::WIDGET_HOVER, Color32::WHITE),
            (&mut w.active, bl::ACCENT, Color32::WHITE),
            (
                &mut w.open,
                Color32::from_rgb(0x4b, 0x4b, 0x4b),
                Color32::WHITE,
            ),
        ] {
            s.bg_fill = fill;
            s.weak_bg_fill = fill;
            s.bg_stroke = Stroke::NONE;
            s.fg_stroke = Stroke::new(1.0, fg);
        }

        style.spacing.item_spacing = egui::vec2(6.0, 3.0);
        style.spacing.button_padding = egui::vec2(8.0, 2.0);
        style.spacing.interact_size.y = 20.0;
        style.spacing.menu_margin = egui::Margin::same(4);
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(13.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(15.0, FontFamily::Proportional),
        );
    });
}

/// Install the editor's bundled fonts: Inter for UI text and 3270 Nerd Font
/// for GUI icons (its private-use glyphs carry the menu symbols).
fn setup_fonts(ctx: &egui::Context) {
    use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
    use egui::{FontData, FontDefinitions, FontFamily};

    const INTER: &[u8] = include_bytes!("../assets/fonts/Inter.ttf");
    const NERD: &[u8] = include_bytes!("../assets/fonts/3270NerdFont-Regular.ttf");

    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("inter".to_owned(), Arc::new(FontData::from_static(INTER)));
    fonts
        .font_data
        .insert("nerd".to_owned(), Arc::new(FontData::from_static(NERD)));

    // Inter is the primary UI font; the Nerd Font is the last fallback so its
    // private-use icon glyphs resolve without changing the text look.
    let proportional = fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .expect("proportional family");
    proportional.insert(0, "inter".to_owned());
    proportional.push("nerd".to_owned());
    let monospace = fonts
        .families
        .get_mut(&FontFamily::Monospace)
        .expect("monospace family");
    monospace.push("nerd".to_owned());
    ctx.set_fonts(fonts);

    // A named "icons" family backed by the Nerd Font, so icon labels can opt in
    // explicitly with `FontId::new(size, FontFamily::Name("icons".into()))`.
    ctx.add_font(FontInsert::new(
        "icons",
        FontData::from_static(NERD),
        vec![InsertFontFamily {
            family: FontFamily::Name("icons".into()),
            priority: FontPriority::Highest,
        }],
    ));
}
