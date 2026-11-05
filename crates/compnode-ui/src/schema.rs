//! Socket schema: what pins a node has, their types, defaults and widgets.
use crate::value::{SocketType, Value};

pub const REROUTE_ID: &str = "core.reroute";
pub const FRAME_ID: &str = "core.frame";

/// Node categories drive the header color.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Category {
    Input,
    Output,
    Filter,
    Color,
    Blend,
    Transform,
    Utility,
    Layout,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::Input,
        Category::Output,
        Category::Filter,
        Category::Color,
        Category::Blend,
        Category::Transform,
        Category::Utility,
        Category::Layout,
    ];
    pub fn index(self) -> usize {
        Category::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
    pub fn name(self) -> &'static str {
        match self {
            Category::Input => "input",
            Category::Output => "output",
            Category::Filter => "filter",
            Category::Color => "color",
            Category::Blend => "blend",
            Category::Transform => "transform",
            Category::Utility => "utility",
            Category::Layout => "layout",
        }
    }
}

/// Which widget edits an unconnected socket.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WidgetKind {
    None,
    Float,
    Int,
    Slider,
    Toggle,
    Button,
    Dropdown,
    TextArea,
    ColorPicker,
    ImagePreview,
    Label,
}

impl WidgetKind {
    /// The widget used when a socket does not ask for a specific one.
    pub fn default_for(ty: SocketType) -> WidgetKind {
        match ty {
            SocketType::Float => WidgetKind::Float,
            SocketType::Int => WidgetKind::Int,
            SocketType::Bool => WidgetKind::Toggle,
            SocketType::Color => WidgetKind::ColorPicker,
            SocketType::String => WidgetKind::TextArea,
            SocketType::Enum => WidgetKind::Dropdown,
            SocketType::Vec3 | SocketType::Image | SocketType::Any => WidgetKind::None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SocketDef {
    pub name: String,
    pub ty: SocketType,
    pub default: Value,
    pub widget: WidgetKind,
    /// Hard limits (values are clamped to them).
    pub min: f32,
    pub max: f32,
    pub step: f32,
    /// Range shown by sliders (may be narrower than the hard limits).
    pub soft_min: f32,
    pub soft_max: f32,
    pub options: Vec<String>,
    /// A multi-input socket accepts any number of incoming wires (e.g. the
    /// Output node's layer list). Single-input sockets replace on connect.
    pub multi: bool,
    /// Id of a host-provided widget. When set, the editor asks the host's
    /// widget factory for a widget with this id before falling back to the
    /// built-in [`WidgetKind`]. This is the extension point for widgets the
    /// library does not know about (curve editors, node graphs, ...).
    pub widget_id: Option<String>,
}

impl SocketDef {
    pub fn new(name: &str, ty: SocketType) -> SocketDef {
        SocketDef {
            name: name.to_string(),
            ty,
            default: ty.default_value(),
            widget: WidgetKind::default_for(ty),
            min: f32::NEG_INFINITY,
            max: f32::INFINITY,
            step: if ty == SocketType::Int { 1.0 } else { 0.1 },
            soft_min: f32::NEG_INFINITY,
            soft_max: f32::INFINITY,
            options: Vec::new(),
            multi: false,
            widget_id: None,
        }
    }
    pub fn float(name: &str, default: f32) -> SocketDef {
        SocketDef::new(name, SocketType::Float).with_default(Value::Float(default))
    }
    pub fn int(name: &str, default: i32) -> SocketDef {
        SocketDef::new(name, SocketType::Int).with_default(Value::Int(default))
    }
    pub fn color(name: &str, default: compnode_core::Color) -> SocketDef {
        SocketDef::new(name, SocketType::Color).with_default(Value::Color(default))
    }
    pub fn image(name: &str) -> SocketDef {
        SocketDef::new(name, SocketType::Image)
    }
    pub fn with_default(mut self, v: Value) -> SocketDef {
        self.default = v;
        self
    }
    pub fn widget(mut self, w: WidgetKind) -> SocketDef {
        self.widget = w;
        self
    }
    /// Hard limits; also the slider range unless [`SocketDef::soft`] is set.
    pub fn range(mut self, min: f32, max: f32) -> SocketDef {
        self.min = min;
        self.max = max;
        self
    }
    pub fn soft(mut self, min: f32, max: f32) -> SocketDef {
        self.soft_min = min;
        self.soft_max = max;
        self
    }
    pub fn step(mut self, step: f32) -> SocketDef {
        self.step = step;
        self
    }
    pub fn options(mut self, opts: &[&str]) -> SocketDef {
        self.options = opts.iter().map(|s| s.to_string()).collect();
        self
    }
    /// Mark the socket as a multi-input (accepts any number of wires).
    pub fn multi(mut self) -> SocketDef {
        self.multi = true;
        self
    }
    /// Use a host-provided widget, looked up by `id` in the editor's widget
    /// factory (see [`crate::editor::Editor::set_widget_factory`]).
    pub fn custom_widget(mut self, id: &str) -> SocketDef {
        self.widget_id = Some(id.to_string());
        self
    }
    /// Range for a slider: soft range, else hard range, else `0..1`.
    pub fn slider_range(&self) -> (f32, f32) {
        if self.soft_min.is_finite() && self.soft_max.is_finite() {
            (self.soft_min, self.soft_max)
        } else if self.min.is_finite() && self.max.is_finite() {
            (self.min, self.max)
        } else {
            (0.0, 1.0)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeSchema {
    pub id: String,
    pub title: String,
    pub category: Category,
    pub inputs: Vec<SocketDef>,
    pub outputs: Vec<SocketDef>,
    /// Widget-only rows below the pin rows (no pins). Their values live in
    /// `NodeData::out_values` after the outputs.
    pub footer: Vec<SocketDef>,
    pub width: f32,
}

impl NodeSchema {
    pub fn new(id: &str, title: &str, category: Category) -> NodeSchema {
        NodeSchema {
            id: id.to_string(),
            title: title.to_string(),
            category,
            inputs: Vec::new(),
            outputs: Vec::new(),
            footer: Vec::new(),
            width: compnode_core::layout::DEFAULT_WIDTH,
        }
    }
    pub fn input(mut self, d: SocketDef) -> NodeSchema {
        self.inputs.push(d);
        self
    }
    /// Add an output. Outputs show no widget unless one was asked for
    /// explicitly (the type's default widget is dropped).
    pub fn output(mut self, mut d: SocketDef) -> NodeSchema {
        if d.widget == WidgetKind::default_for(d.ty) {
            d.widget = WidgetKind::None;
        }
        self.outputs.push(d);
        self
    }
    pub fn width(mut self, w: f32) -> NodeSchema {
        self.width = w;
        self
    }
    pub fn default_values(&self) -> Vec<Value> {
        self.inputs.iter().map(|d| d.default.clone()).collect()
    }
    /// Add a widget-only footer row (for example an image preview).
    pub fn footer(mut self, d: SocketDef) -> NodeSchema {
        self.footer.push(d);
        self
    }
    /// Outputs first, then footer rows.
    pub fn default_out_values(&self) -> Vec<Value> {
        self.outputs
            .iter()
            .chain(self.footer.iter())
            .map(|d| d.default.clone())
            .collect()
    }
    /// Pin rows: the larger of the input and output counts.
    pub fn pin_rows(&self) -> usize {
        self.inputs.len().max(self.outputs.len())
    }
    /// Row index (from the top of the node body) of the widget stored at
    /// `out_values[slot]`.
    pub fn out_row(&self, slot: usize) -> usize {
        if slot < self.outputs.len() {
            slot
        } else {
            self.pin_rows() + (slot - self.outputs.len())
        }
    }
    /// Rows hold the input at the same index and the output at the same
    /// index; a row may not carry both an input widget and an output widget.
    pub fn validate(&self) -> Result<(), String> {
        for (i, d) in self.inputs.iter().enumerate() {
            if d.default.socket_type() != d.ty && d.ty != SocketType::Any {
                return Err(format!(
                    "{}: input {} default has the wrong type",
                    self.id, d.name
                ));
            }
            if d.widget == WidgetKind::Dropdown && d.options.is_empty() {
                return Err(format!("{}: dropdown {} has no options", self.id, d.name));
            }
            if let Some(o) = self.outputs.get(i)
                && d.widget != WidgetKind::None
                && o.widget != WidgetKind::None
            {
                return Err(format!(
                    "{}: row {i} has both an input and an output widget",
                    self.id
                ));
            }
        }
        for f in &self.footer {
            if f.widget == WidgetKind::None {
                return Err(format!("{}: footer row {} has no widget", self.id, f.name));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Registry {
    schemas: Vec<NodeSchema>,
}
impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}
impl Registry {
    /// Empty registry plus the built-in reroute and frame nodes.
    pub fn new() -> Registry {
        let mut r = Registry {
            schemas: Vec::new(),
        };
        r.register(
            NodeSchema::new(REROUTE_ID, "Reroute", Category::Layout)
                .input(SocketDef::new("", SocketType::Any).widget(WidgetKind::None))
                .output(SocketDef::new("", SocketType::Any).widget(WidgetKind::None))
                .width(40.0),
        );
        r.register(NodeSchema::new(FRAME_ID, "Frame", Category::Layout).width(300.0));
        r
    }
    /// Add a schema, replacing any with the same id.
    pub fn register(&mut self, s: NodeSchema) {
        if let Some(slot) = self.schemas.iter_mut().find(|x| x.id == s.id) {
            *slot = s;
        } else {
            self.schemas.push(s);
        }
    }
    pub fn get(&self, id: &str) -> Option<&NodeSchema> {
        self.schemas.iter().find(|s| s.id == id)
    }
    pub fn iter(&self) -> impl Iterator<Item = &NodeSchema> {
        self.schemas.iter()
    }
    pub fn len(&self) -> usize {
        self.schemas.len()
    }
    pub fn is_empty(&self) -> bool {
        self.schemas.is_empty()
    }
    /// Add-node menu search: case-insensitive; title prefix matches rank
    /// before substring matches, which rank before category matches. An
    /// empty query lists everything, grouped by category. Layout helpers
    /// (reroute, frame) are listed too.
    pub fn search(&self, query: &str) -> Vec<&NodeSchema> {
        let q = query.trim().to_lowercase();
        let mut hits: Vec<(u8, usize, &NodeSchema)> = Vec::new();
        for (i, s) in self.schemas.iter().enumerate() {
            let title = s.title.to_lowercase();
            let rank = if q.is_empty() {
                Some(s.category.index() as u8)
            } else if title.starts_with(&q) {
                Some(0)
            } else if title.contains(&q) || s.id.to_lowercase().contains(&q) {
                Some(1)
            } else if s.category.name().contains(&q) {
                Some(2)
            } else {
                None
            };
            if let Some(r) = rank {
                hits.push((r, i, s));
            }
        }
        hits.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.2.title.cmp(&b.2.title))
                .then(a.1.cmp(&b.1))
        });
        hits.into_iter().map(|h| h.2).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> Registry {
        let mut r = Registry::new();
        r.register(
            NodeSchema::new("t.blur", "Blur", Category::Filter)
                .input(SocketDef::image("Image"))
                .input(SocketDef::float("Radius", 4.0).range(0.0, 100.0))
                .output(SocketDef::image("Image")),
        );
        r.register(NodeSchema::new("t.blend", "Blend", Category::Blend));
        r.register(NodeSchema::new("t.bright", "Brightness", Category::Color));
        r
    }

    #[test]
    fn defaults_and_widget_selection() {
        let r = reg();
        let s = r.get("t.blur").unwrap();
        assert_eq!(
            s.default_values(),
            vec![Value::Image(None), Value::Float(4.0)]
        );
        assert_eq!(s.inputs[0].widget, WidgetKind::None);
        assert_eq!(s.inputs[1].widget, WidgetKind::Float);
        assert_eq!(
            SocketDef::new("x", SocketType::Color).widget,
            WidgetKind::ColorPicker
        );
        assert!(s.validate().is_ok());
    }

    #[test]
    fn validate_catches_mistakes() {
        let bad = NodeSchema::new("x", "X", Category::Utility)
            .input(SocketDef::new("m", SocketType::Enum));
        assert!(bad.validate().is_err()); // dropdown without options
        let bad = NodeSchema::new("x", "X", Category::Utility)
            .input(SocketDef::float("a", 0.0))
            .output(SocketDef::float("b", 0.0).widget(WidgetKind::Label));
        assert!(bad.validate().is_err()); // both widgets on one row
    }

    #[test]
    fn slider_range_prefers_soft_then_hard() {
        let d = SocketDef::float("a", 0.0);
        assert_eq!(d.slider_range(), (0.0, 1.0));
        let d = d.range(-5.0, 5.0);
        assert_eq!(d.slider_range(), (-5.0, 5.0));
        assert_eq!(d.soft(0.0, 2.0).slider_range(), (0.0, 2.0));
    }

    #[test]
    fn search_ranks_prefix_before_substring() {
        let r = reg();
        let hits: Vec<_> = r.search("bl").iter().map(|s| s.title.clone()).collect();
        assert_eq!(hits, vec!["Blend", "Blur"]);
        let hits: Vec<_> = r.search("ness").iter().map(|s| s.title.clone()).collect();
        assert_eq!(hits, vec!["Brightness"]);
        assert!(r.search("zzz").is_empty());
        assert_eq!(r.search("").len(), r.len());
        // registering again replaces
        let mut r = reg();
        let n = r.len();
        r.register(NodeSchema::new("t.blur", "Blur 2", Category::Filter));
        assert_eq!(r.len(), n);
        assert_eq!(r.get("t.blur").unwrap().title, "Blur 2");
    }
}
