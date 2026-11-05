//! Keyboard-only editing: Tab moves a focus ring over the widgets, arrow keys,
//! Enter and Space drive the focused widget. Also the accessibility tree a
//! host can hand to a screen-reader API (for example AccessKit).
use super::*;
use crate::input::{InputEvent, Key, Modifiers};
use crate::widgets::value_text;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AccessRole {
    Node,
    Frame,
    Slider,
    SpinButton,
    CheckBox,
    Button,
    ComboBox,
    TextInput,
    ColorWell,
    Image,
    Label,
}

/// One entry of the accessibility tree. Rectangles are in graph space.
#[derive(Clone, Debug, PartialEq)]
pub struct AccessItem {
    pub uid: u64,
    /// `None` for the node itself, otherwise `(slot, is_output)` of a widget.
    pub widget: Option<(usize, bool)>,
    pub role: AccessRole,
    pub label: String,
    pub value: String,
    pub rect: Rect,
    pub focused: bool,
    pub selected: bool,
}

fn role_of(kind: WidgetKind, int: bool) -> Option<AccessRole> {
    Some(match kind {
        WidgetKind::None => return None,
        WidgetKind::Float | WidgetKind::Int => {
            let _ = int;
            AccessRole::SpinButton
        }
        WidgetKind::Slider => AccessRole::Slider,
        WidgetKind::Toggle => AccessRole::CheckBox,
        WidgetKind::Button => AccessRole::Button,
        WidgetKind::Dropdown => AccessRole::ComboBox,
        WidgetKind::TextArea => AccessRole::TextInput,
        WidgetKind::ColorPicker => AccessRole::ColorWell,
        WidgetKind::ImagePreview => AccessRole::Image,
        WidgetKind::Label => AccessRole::Label,
    })
}

fn interactive(kind: WidgetKind) -> bool {
    !matches!(
        kind,
        WidgetKind::None | WidgetKind::ImagePreview | WidgetKind::Label
    )
}

impl Editor {
    /// The widget that has keyboard focus: `(uid, slot, is_output)`.
    pub fn focus(&self) -> Option<(u64, usize, bool)> {
        self.focus
    }

    /// Focusable widgets in tab order: the selected nodes if any, otherwise
    /// every node, each in uid order and top to bottom.
    pub fn focus_order(&self) -> Vec<(u64, usize, bool)> {
        let mut uids = self.selection.to_vec();
        if uids.is_empty() {
            uids = self.doc.uids();
        }
        let mut out = Vec::new();
        for uid in uids {
            let Some(node) = self.doc.id_of(uid).and_then(|id| self.doc.graph.node(id)) else {
                continue;
            };
            if node.data.is_frame() || node.data.is_reroute() || node.data.collapsed {
                continue;
            }
            let Some(schema) = self.doc.registry.get(&node.data.schema) else {
                continue;
            };
            for i in 0..schema.pin_rows() {
                if schema.inputs.get(i).is_some_and(|d| interactive(d.widget))
                    && node.input_wire(i).is_none()
                {
                    out.push((uid, i, false));
                }
                if schema.outputs.get(i).is_some_and(|d| interactive(d.widget)) {
                    out.push((uid, i, true));
                }
            }
        }
        out
    }

    /// Move keyboard focus (or clear it with `None`).
    pub fn set_focus(&mut self, target: Option<(u64, usize, bool)>) {
        if let Some(key) = self.focus
            && let Some(w) = self.widgets.get_mut(&key)
        {
            w.set_focused(false);
        }
        self.focus = target;
        if let Some(key) = target {
            if let Some(w) = self.widgets.get_mut(&key) {
                w.set_focused(true);
            }
            if !self.selection.contains(key.0) {
                self.selection.set_only(key.0);
            }
        }
        self.ui_rev += 1;
    }

    fn cycle_focus(&mut self, backwards: bool) {
        let order = self.focus_order();
        if order.is_empty() {
            self.set_focus(None);
            return;
        }
        let pos = self.focus.and_then(|f| order.iter().position(|o| *o == f));
        let next = match (pos, backwards) {
            (None, false) => 0,
            (None, true) => order.len() - 1,
            (Some(p), false) => (p + 1) % order.len(),
            (Some(p), true) => (p + order.len() - 1) % order.len(),
        };
        self.set_focus(Some(order[next]));
    }

    /// Handle a key for the focus ring. `Some(next)` if it was used.
    pub(crate) fn focus_key(&mut self, key: Key, mods: Modifiers) -> Option<Interaction> {
        match key {
            Key::Tab => {
                self.cycle_focus(mods.shift);
                Some(Interaction::Idle)
            }
            Key::Escape if self.focus.is_some() => {
                self.set_focus(None);
                Some(Interaction::Idle)
            }
            Key::Left | Key::Right | Key::Up | Key::Down | Key::Enter | Key::Space => {
                let (uid, slot, output) = self.focus?;
                let resp = self.widget_event(uid, slot, output, &InputEvent::Key { key, mods })?;
                if !resp.consumed {
                    return None;
                }
                if self.widget_captures(uid, slot, output) || self.pending_text.is_some() {
                    Some(Interaction::EditWidget { uid, slot, output })
                } else {
                    Some(Interaction::Idle)
                }
            }
            _ => None,
        }
    }

    /// Accessibility tree for the nodes currently in view (widgets only at
    /// the full level of detail, as on screen).
    pub fn accessibility(&self) -> Vec<AccessItem> {
        let mut items = Vec::new();
        let full = self.view.lod() == compnode_core::Lod::Full;
        for uid in self.visible_uids() {
            let Some(node) = self.doc.id_of(uid).and_then(|id| self.doc.graph.node(id)) else {
                continue;
            };
            let Some(schema) = self.doc.registry.get(&node.data.schema) else {
                continue;
            };
            let title = node
                .data
                .title
                .clone()
                .unwrap_or_else(|| schema.title.clone());
            let selected = self.selection.contains(uid);
            let role = if node.data.is_frame() {
                AccessRole::Frame
            } else {
                AccessRole::Node
            };
            let state = if node.data.muted { " (muted)" } else { "" };
            items.push(AccessItem {
                uid,
                widget: None,
                role,
                label: format!("{title}{state}"),
                value: String::new(),
                rect: node.rect(),
                focused: false,
                selected,
            });
            if !full || node.data.is_frame() || node.data.is_reroute() || node.data.collapsed {
                continue;
            }
            let mut push = |slot: usize, output: bool, def: &SocketDef, value: &Value| {
                let Some(role) = role_of(def.widget, def.ty == SocketType::Int) else {
                    return;
                };
                let Some(rect) = self.widget_rect(uid, slot, output) else {
                    return;
                };
                let text = match (def.widget, value) {
                    (WidgetKind::Dropdown, Value::Enum(i)) => {
                        def.options.get(*i).cloned().unwrap_or_default()
                    }
                    (_, v) => value_text(v),
                };
                items.push(AccessItem {
                    uid,
                    widget: Some((slot, output)),
                    role,
                    label: format!("{title}: {}", def.name),
                    value: text,
                    rect,
                    focused: self.focus == Some((uid, slot, output)),
                    selected,
                });
            };
            for (i, def) in schema.inputs.iter().enumerate() {
                if node.input_wire(i).is_none() {
                    push(i, false, def, &node.data.values[i]);
                }
            }
            for (i, def) in schema
                .outputs
                .iter()
                .chain(schema.footer.iter())
                .enumerate()
            {
                push(i, true, def, &node.data.out_values[i]);
            }
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::editor;
    use super::*;
    use crate::input::InputEvent;
    use crate::schema::{Category, NodeSchema};

    fn two_widget_editor() -> (Editor, u64) {
        let mut reg = crate::document::test_support::registry();
        reg.register(
            NodeSchema::new("t.mix", "Mix", Category::Filter)
                .input(SocketDef::image("Image"))
                .input(SocketDef::new("Mode", SocketType::Enum).options(&["A", "B", "C"]))
                .input(SocketDef::new("On", SocketType::Bool))
                .output(SocketDef::image("Image")),
        );
        let mut e = Editor::new(reg);
        let n = e.add_node("t.mix", vec2(40.0, 40.0)).unwrap();
        (e, n)
    }

    #[test]
    fn tab_walks_the_widgets_and_wraps() {
        let (mut e, n) = two_widget_editor();
        assert_eq!(e.focus_order(), vec![(n, 1, false), (n, 2, false)]);
        e.handle_event(&InputEvent::key(Key::Tab));
        assert_eq!(e.focus(), Some((n, 1, false)));
        e.handle_event(&InputEvent::key(Key::Tab));
        assert_eq!(e.focus(), Some((n, 2, false)));
        e.handle_event(&InputEvent::key(Key::Tab));
        assert_eq!(e.focus(), Some((n, 1, false)));
        e.handle_event(&InputEvent::key(Key::Tab).with_mods(Modifiers::SHIFT));
        assert_eq!(e.focus(), Some((n, 2, false)));
        e.handle_event(&InputEvent::key(Key::Escape));
        assert_eq!(e.focus(), None);
        e.handle_event(&InputEvent::key(Key::Tab).with_mods(Modifiers::SHIFT));
        assert_eq!(e.focus(), Some((n, 2, false)));
    }

    #[test]
    fn focused_widgets_respond_to_keys_with_undo() {
        let (mut e, n) = two_widget_editor();
        e.set_focus(Some((n, 2, false)));
        e.handle_event(&InputEvent::key(Key::Space)); // toggle
        assert_eq!(e.doc.data(n).unwrap().values[2], Value::Bool(true));
        e.handle_event(&InputEvent::key(Key::Enter));
        assert_eq!(e.doc.data(n).unwrap().values[2], Value::Bool(false));
        assert!(e.undo());
        assert_eq!(e.doc.data(n).unwrap().values[2], Value::Bool(true));

        // dropdown: Enter opens it (captures), arrows move, Enter picks
        e.set_focus(Some((n, 1, false)));
        e.handle_event(&InputEvent::key(Key::Enter));
        assert!(matches!(e.interaction(), Interaction::EditWidget { .. }));
        e.handle_event(&InputEvent::key(Key::Down));
        e.handle_event(&InputEvent::key(Key::Enter));
        assert_eq!(*e.interaction(), Interaction::Idle);
        assert_eq!(e.doc.data(n).unwrap().values[1], Value::Enum(1));
        // without a popup, arrows change the choice directly
        e.handle_event(&InputEvent::key(Key::Down));
        assert_eq!(e.doc.data(n).unwrap().values[1], Value::Enum(2));
    }

    #[test]
    fn number_fields_step_and_type_from_the_keyboard() {
        let mut e = editor();
        let f = e.add_node("t.filter", vec2(0.0, 0.0)).unwrap();
        e.handle_event(&InputEvent::key(Key::Tab));
        assert_eq!(e.focus(), Some((f, 1, false)));
        e.handle_event(&InputEvent::key(Key::Right));
        let v = e.doc.data(f).unwrap().values[1].as_f32().unwrap();
        assert!((v - 2.1).abs() < 1e-4, "{v}");
        e.handle_event(&InputEvent::key(Key::Enter));
        assert!(e.pending_text_edit().is_some());
        e.commit_text("7");
        assert_eq!(e.doc.data(f).unwrap().values[1], Value::Float(7.0));
        assert_eq!(*e.interaction(), Interaction::Idle);
    }

    #[test]
    fn connected_inputs_and_collapsed_nodes_are_skipped() {
        let (mut e, n) = two_widget_editor();
        let t = e.add_node("t.num", vec2(0.0, 300.0)).unwrap();
        e.selection.set_all(&[n, t]);
        assert_eq!(e.focus_order().len(), 3);
        e.connect((t, 0), (n, 2)); // Float -> Bool input: now wired
        assert_eq!(e.focus_order(), vec![(n, 1, false), (t, 0, false)]);
        e.selection.set_only(n);
        e.toggle_collapse();
        assert!(e.focus_order().is_empty());
        e.handle_event(&InputEvent::key(Key::Tab));
        assert_eq!(e.focus(), None);
    }

    #[test]
    fn deleting_the_focused_node_clears_focus() {
        let (mut e, n) = two_widget_editor();
        e.set_focus(Some((n, 1, false)));
        e.selection.set_only(n);
        e.delete_selection();
        assert_eq!(e.focus(), None);
    }

    #[test]
    fn accessibility_lists_nodes_and_widgets_with_roles() {
        let (mut e, n) = two_widget_editor();
        e.set_focus(Some((n, 2, false)));
        let items = e.accessibility();
        assert_eq!(items[0].role, AccessRole::Node);
        assert_eq!(items[0].label, "Mix");
        let mode = items.iter().find(|i| i.widget == Some((1, false))).unwrap();
        assert_eq!(
            (mode.role, mode.value.as_str(), mode.label.as_str()),
            (AccessRole::ComboBox, "A", "Mix: Mode")
        );
        let on = items.iter().find(|i| i.widget == Some((2, false))).unwrap();
        assert_eq!(
            (on.role, on.focused, on.value.as_str()),
            (AccessRole::CheckBox, true, "Off")
        );
        assert!(items.iter().all(|i| i.rect.width() > 0.0));
        // no widgets below Full level of detail
        e.view.zoom = 0.4;
        assert_eq!(e.accessibility().len(), 1);
    }
}
