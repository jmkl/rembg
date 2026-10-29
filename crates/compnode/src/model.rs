//! Graph data types.
//!
//! Adding a node means:
//! 1. Add a [`Kind`] variant and a [`Node`] variant.
//! 2. Add its metadata/body/create functions to [`crate::catalog`].
//! 3. Add its body implementation under `src/nodes/`.
//! 4. Add its session mapping in [`crate::session`] if it has persistent data.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::TextureHandle;
use egui_snarl::NodeId;
use image::RgbaImage;
use serde_json::Value;

use crate::cancellation::CancellationToken;
use crate::catalog::{self, FieldKind, NodeSpec, SocketSpec};
use crate::params::{self, Params};

/// Stable identity for every node kind. Used by the catalog and session layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum Kind {
    Image,
    RemoveBg,
    RemoveBgV2,
    FaceRestore,
    Output,
    Route,
    Group,
    Blend,
    Texts,
    Filter(FilterKind),
}

impl Kind {
    /// Kinds that composite multiple upstream sources as transformable canvas layers.
    pub(crate) fn is_layer_kind(self) -> bool {
        matches!(self, Self::Group | Self::Blend)
    }
}

/// Generic CPU filter/generator node types (excluding structural pass-throughs,
/// layer containers, and specialized typography generators).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum FilterKind {
    Mask,
    Hsv,
    Curves,
    Exposure,
    ColorBalance,
    GradientMap,
    Invert,
    Gradient,
    Noise,
    RawFilter,
}

impl FilterKind {
    /// Stable string used in session files.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Mask => "mask",
            Self::Hsv => "hsv",
            Self::Curves => "curves",
            Self::Exposure => "exposure",
            Self::ColorBalance => "colorbalance",
            Self::GradientMap => "gradientmap",
            Self::Invert => "invert",
            Self::Gradient => "gradient",
            Self::Noise => "noise",
            Self::RawFilter => "rawfilter",
        }
    }

    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "mask" => Self::Mask,
            "hsv" => Self::Hsv,
            "curves" => Self::Curves,
            "exposure" => Self::Exposure,
            "colorbalance" => Self::ColorBalance,
            "gradientmap" => Self::GradientMap,
            "invert" => Self::Invert,
            "gradient" => Self::Gradient,
            "noise" => Self::Noise,
            "rawfilter" | "raw" => Self::RawFilter,
            _ => return None,
        })
    }

    /// Default parameters auto-derived from catalog specification.
    pub(crate) fn defaults(self) -> Params {
        let mut params = Params::new();
        let spec = crate::catalog::spec(Kind::Filter(self));
        spec.apply_defaults(&mut params);
        if self == Self::Curves {
            let mut curves = Params::new();
            for channel in ["rgb", "red", "green", "blue"] {
                curves.insert(channel.to_owned(), params::identity_curve_value());
            }
            params.insert("curves".to_owned(), Value::Object(curves));
        }
        params
    }
}

/// A generic CPU filter/generator node.
#[derive(Clone)]
pub(crate) struct FilterNode {
    pub kind: FilterKind,
    pub params: Params,
    /// Cached evaluated image and its preview texture (at most `eval::MAX_EDGE`
    /// on the longest side).
    pub preview: Preview,
    /// Signature the cached preview was produced from. `None` forces a
    /// re-evaluation on the next frame.
    pub signature: Option<u64>,
    /// Whether to render the preview image in the node body.
    pub preview_enabled: bool,
    /// Token of the in-flight full-resolution render, if any.
    pub job: Option<u64>,
    pub cancellation: Option<CancellationToken>,
    /// Absolute path to the persisted full-resolution composite on disk.
    pub output_path: Option<PathBuf>,
    /// Debounce state for scheduling the full-resolution render.
    pub render_state: Option<RenderState>,
    /// Eval signature whose full-resolution render has been claimed (in flight
    /// or completed). Prevents re-submitting identical renders every frame.
    pub rendered: Option<u64>,
}

/// Layer compositing container node (Group and Blend).
#[derive(Clone)]
pub(crate) struct GroupNode {
    pub is_blend: bool,
    pub params: Params,
    pub preview: Preview,
    pub signature: Option<u64>,
    pub preview_enabled: bool,
    pub layers: Vec<ViewerLayer>,
    pub canvas_size: egui::Vec2,
    pub job: Option<u64>,
    pub cancellation: Option<CancellationToken>,
    pub output_path: Option<PathBuf>,
    pub render_state: Option<RenderState>,
    pub rendered: Option<u64>,
}

impl GroupNode {
    pub(crate) fn new(is_blend: bool) -> Self {
        let mut params = Params::new();
        if is_blend {
            let spec = crate::catalog::spec(Kind::Blend);
            spec.apply_defaults(&mut params);
        }
        Self {
            is_blend,
            params,
            preview: Preview::default(),
            signature: None,
            preview_enabled: !is_blend,
            layers: Vec::new(),
            canvas_size: egui::Vec2::ZERO,
            job: None,
            cancellation: None,
            output_path: None,
            render_state: None,
            rendered: None,
        }
    }
}

/// Dedicated typographic generator node.
#[derive(Clone)]
pub(crate) struct TextsNode {
    pub params: Params,
    pub preview: Preview,
    pub signature: Option<u64>,
    pub preview_enabled: bool,
    pub job: Option<u64>,
    pub cancellation: Option<CancellationToken>,
    pub output_path: Option<PathBuf>,
    pub render_state: Option<RenderState>,
    pub rendered: Option<u64>,
}

impl TextsNode {
    pub(crate) fn new() -> Self {
        let mut params = Params::new();
        params::set_string(&mut params, "font", "unisans");
        params::set_string(&mut params, "align", "left");
        params::set_bool(&mut params, "uppercase", true);
        params::set_bool(&mut params, "effects", true);
        params::set_bool(&mut params, "box", false);
        params::set_string(&mut params, "box_color", "#000000");
        params::set_f32(&mut params, "box_padding_x", 24.0);
        params::set_f32(&mut params, "box_padding_y", 12.0);
        params::set_f32(&mut params, "line_gap", -16.0);
        params::set_f32(&mut params, "letter_spacing", -2.0);
        params::set_f32(&mut params, "outline_width", 0.0);
        params::set_string(&mut params, "outline_color", "#ffffff");
        params::set_f32(&mut params, "outline2_width", 12.0);
        params::set_string(&mut params, "outline2_color", "#000000");
        params::set_f32(&mut params, "depth", 7.0);
        params::set_f32(&mut params, "depth_color", 8.0);
        params::set_f32(&mut params, "depth_x", 1.0);
        params::set_f32(&mut params, "depth_y", 1.0);
        params::set_f32(&mut params, "shadow_opacity", 0.5);
        params::set_string(&mut params, "shadow_color", "#000000");
        params::set_f32(&mut params, "shadow_x", 10.0);
        params::set_f32(&mut params, "shadow_y", 10.0);
        params::set_f32(&mut params, "shadow_blur", 10.0);
        params::set_string(&mut params, "text", "Hello world,\nThis is the end!");
        Self {
            params,
            preview: Preview::default(),
            signature: None,
            preview_enabled: true,
            job: None,
            cancellation: None,
            output_path: None,
            render_state: None,
            rendered: None,
        }
    }
}

/// Debounce bookkeeping for the Group/Filter full-resolution render: the eval
/// signature that must stay stable and when it first became stable.
#[derive(Clone, Copy)]
pub(crate) struct RenderState {
    pub signature: u64,
    pub since: std::time::Instant,
}

impl FilterNode {
    pub(crate) fn new(kind: FilterKind) -> Self {
        Self {
            kind,
            params: kind.defaults(),
            preview: Preview::default(),
            signature: None,
            preview_enabled: true,
            job: None,
            cancellation: None,
            output_path: None,
            render_state: None,
            rendered: None,
        }
    }
}

/// Image preview shared by source and processed nodes.
#[derive(Clone, Default)]
pub(crate) struct Preview {
    pub texture: Option<TextureHandle>,
    pub cpu: Option<RgbaImage>,
    /// Evaluation-resolution copy (at most the normal preview tier on the longest
    /// side). Filter evaluation uses this instead of repeatedly downscaling
    /// `cpu`, which is expensive for full-resolution sources.
    pub eval: Option<RgbaImage>,
    pub error: Option<String>,
}

impl Preview {
    pub(crate) fn with_image(texture: TextureHandle, cpu: RgbaImage) -> Self {
        let eval = eval_copy(&cpu);
        Self {
            texture: Some(texture),
            cpu: Some(cpu),
            eval: Some(eval),
            error: None,
        }
    }

    pub(crate) fn set_cpu(&mut self, image: RgbaImage) {
        self.eval = Some(eval_copy(&image));
        self.cpu = Some(image);
    }

    pub(crate) fn clear(&mut self) {
        self.texture = None;
        self.cpu = None;
        self.eval = None;
        self.error = None;
    }

    pub(crate) fn eval_image(&self) -> Option<&RgbaImage> {
        self.eval.as_ref().or(self.cpu.as_ref())
    }
}

/// Downscale to the evaluation resolution used by the preview graph.
fn eval_copy(image: &RgbaImage) -> RgbaImage {
    let (width, height) = image.dimensions();
    let longest = width.max(height);
    let max_edge = crate::resolution::PreviewQuality::Normal.max_edge();
    if longest <= max_edge {
        return image.clone();
    }
    let scale = max_edge as f64 / longest as f64;
    let width = ((width as f64 * scale).round() as u32).max(1);
    let height = ((height as f64 * scale).round() as u32).max(1);
    image::imageops::resize(image, width, height, image::imageops::FilterType::Triangle)
}

/// Timing for the most recent successful inference run.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RunTiming {
    /// Worker wall time, including model loading, inference and result caching.
    pub total: Duration,
    /// Time spent loading models that were not already resident.
    pub load: Duration,
    /// Time spent running the model.
    pub infer: Duration,
    /// Whether the model cache setting was enabled for this run.
    pub cache_models: bool,
}

/// State common to inference nodes.
#[derive(Clone, Default)]
pub(crate) struct Process {
    pub preview: Preview,
    /// Token of the job currently running for this node, if any.
    pub job: Option<u64>,
    pub cancellation: Option<CancellationToken>,
    /// Absolute path to this node's result image on disk.
    pub output_path: Option<PathBuf>,
    /// Benchmark for the last successful run.
    pub last_timing: Option<RunTiming>,
    /// Whether to render the preview image in the node body.
    pub preview_enabled: bool,
}

impl Process {
    /// Drop computed output and any pending job, preserving node parameters.
    pub(crate) fn clear_outputs(&mut self) {
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
        self.preview.clear();
        self.job = None;
        self.output_path = None;
        self.last_timing = None;
    }
}

/// Result of a successful worker job.
pub(crate) struct JobOutput {
    pub image: RgbaImage,
    pub path: Option<PathBuf>,
    pub timing: RunTiming,
}

/// One layer in a Viewer.
#[derive(Clone)]
pub struct ViewerLayer {
    pub source: NodeId,
    pub kind: Kind,
    pub texture: Option<TextureHandle>,
    /// Human-readable title of the source node (e.g. "Remove Background").
    pub source_name: String,
    /// Output filepath of the source node, if any.
    pub source_path: Option<PathBuf>,
    /// Center in canvas coordinates.
    pub center: egui::Vec2,
    /// Size in canvas units, before rotation.
    pub size: egui::Vec2,
    /// Radians, clockwise on screen.
    pub angle: f32,
}

impl ViewerLayer {
    #[allow(dead_code)]
    pub fn new(source: NodeId, kind: Kind, texture: Option<TextureHandle>) -> Self {
        Self {
            source,
            kind,
            texture,
            source_name: String::new(),
            source_path: None,
            center: egui::Vec2::ZERO,
            size: egui::Vec2::ZERO,
            angle: 0.0,
        }
    }
}

#[derive(Clone)]
pub(crate) struct ImageNode {
    /// Content-addressed source asset. `None` is only used by legacy sessions
    /// that still point at a file on disk.
    pub asset_id: Option<String>,
    /// Original source name, used for output filenames and the node label.
    pub name: String,
    /// Last known source path. The asset bytes are authoritative; this path is
    /// kept for "Reload" and for legacy sessions.
    pub path: String,
    pub preview: Preview,
    /// Whether to render the preview image in the node body.
    pub preview_enabled: bool,
}

impl ImageNode {
    pub(crate) fn new(path: impl Into<String>) -> Self {
        let path = path.into();
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            asset_id: None,
            name,
            path,
            preview: Preview::default(),
            preview_enabled: true,
        }
    }

    pub(crate) fn from_asset(
        asset_id: impl Into<String>,
        name: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            asset_id: Some(asset_id.into()),
            name: name.into(),
            path: path.into(),
            preview: Preview::default(),
            preview_enabled: true,
        }
    }
}

#[derive(Clone)]
pub(crate) struct FaceRestoreNode {
    pub strength: f32,
    pub only_center: bool,
    pub process: Process,
}

impl Default for FaceRestoreNode {
    fn default() -> Self {
        Self {
            strength: 1.0,
            only_center: false,
            process: Process::default(),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct ViewerNode {
    /// Bottom-to-top stack. The last layer is drawn last (on top).
    pub layers: Vec<ViewerLayer>,
    /// Logical canvas size, kept in sync with ImageCanvas so both previews match.
    pub canvas_size: egui::Vec2,
    /// Whether to render the preview image in the node body.
    pub preview_enabled: bool,
}

#[derive(Clone)]
pub(crate) enum Node {
    Image(ImageNode),
    RemoveBg(Process),
    RemoveBgV2(Process),
    FaceRestore(FaceRestoreNode),
    Output(ViewerNode),
    Route,
    Group(GroupNode),
    Blend(GroupNode),
    Texts(TextsNode),
    Filter(FilterNode),
}

/// Shared runtime contract for node instances.
///
/// The enum remains the storage representation, while this trait exposes the
/// definition metadata, ports, parameter schema, validation, and parameter
/// persistence hooks through one object-safe interface.
#[allow(dead_code)]
pub(crate) trait NodeTrait {
    fn kind(&self) -> Kind;

    fn parameters(&self) -> Option<&Params>;

    fn parameters_mut(&mut self) -> Option<&mut Params>;

    fn definition(&self) -> &'static NodeSpec {
        catalog::spec(self.kind())
    }

    fn input_ports(&self) -> &'static [SocketSpec] {
        self.definition().sockets
    }

    fn output_count(&self) -> usize {
        self.definition().outputs
    }

    fn parameter_schema(&self) -> &'static [catalog::FieldSpec] {
        self.definition().fields
    }

    fn serialize_parameters(&self) -> Option<Value> {
        self.parameters().cloned().map(Value::Object)
    }

    /// Merge saved values into constructor defaults, retaining unknown keys
    /// for forward compatibility with newer session files.
    fn restore_parameters(&mut self, stored: &Value) {
        let Some(stored) = stored.as_object() else {
            return;
        };
        let Some(parameters) = self.parameters_mut() else {
            return;
        };
        for (key, value) in stored {
            parameters.insert(key.clone(), value.clone());
        }
    }

    /// Validate fields described by the catalog and preserve unknown values.
    /// Returns false for nodes whose state is stored in typed fields instead
    /// of a parameter map.
    fn validate_parameters(&mut self) -> bool {
        let definition = self.definition();
        let Some(parameters) = self.parameters_mut() else {
            return false;
        };
        for field in definition.fields {
            match field.kind {
                FieldKind::Range {
                    min, max, default, ..
                }
                | FieldKind::Number {
                    min, max, default, ..
                } => {
                    let value = parameters
                        .get(field.key)
                        .and_then(Value::as_f64)
                        .map(|value| value as f32)
                        .filter(|value| value.is_finite())
                        .unwrap_or(default)
                        .clamp(min, max);
                    params::set_f32(parameters, field.key, value);
                }
                FieldKind::Color { default } => {
                    let valid = parameters
                        .get(field.key)
                        .and_then(Value::as_str)
                        .is_some_and(is_rgb_hex_color);
                    if !valid {
                        params::set_string(parameters, field.key, default);
                    }
                }
                FieldKind::Select { options, default } => {
                    let valid = parameters
                        .get(field.key)
                        .and_then(Value::as_str)
                        .is_some_and(|value| options.iter().any(|option| option.value == value));
                    if !valid {
                        params::set_string(parameters, field.key, default);
                    }
                }
            }
        }
        true
    }
}

fn is_rgb_hex_color(value: &str) -> bool {
    let value = value.strip_prefix('#').unwrap_or(value);
    value.len() == 6 && value.chars().all(|character| character.is_ascii_hexdigit())
}

impl NodeTrait for Node {
    fn kind(&self) -> Kind {
        Node::kind(self)
    }

    fn parameters(&self) -> Option<&Params> {
        match self {
            Self::Group(node) | Self::Blend(node) => Some(&node.params),
            Self::Texts(node) => Some(&node.params),
            Self::Filter(node) => Some(&node.params),
            _ => None,
        }
    }

    fn parameters_mut(&mut self) -> Option<&mut Params> {
        match self {
            Self::Group(node) | Self::Blend(node) => Some(&mut node.params),
            Self::Texts(node) => Some(&mut node.params),
            Self::Filter(node) => Some(&mut node.params),
            _ => None,
        }
    }
}

#[cfg(test)]
mod node_trait_tests {
    use super::{FilterKind, Kind, Node, NodeTrait};
    use image::RgbaImage;
    use serde_json::{Value, json};

    #[test]
    fn node_trait_exposes_definition_and_validates_catalog_fields() {
        let mut node = Node::new_filter(FilterKind::Hsv);
        let definition = (&node as &dyn NodeTrait).definition();
        assert_eq!(definition.kind, Kind::Filter(FilterKind::Hsv));
        assert_eq!(definition.title, "Hue / Saturation");
        assert_eq!(node.input_ports().len(), 1);
        assert_eq!(node.output_count(), 1);
        assert_eq!(node.parameter_schema().len(), 3);

        let parameters = node.parameters_mut().unwrap();
        parameters.insert("hue".into(), json!(900.0));
        parameters.insert("sat".into(), json!(1e300));
        parameters.insert("val".into(), json!("invalid"));
        parameters.insert("future_field".into(), json!("keep me"));

        assert!(node.validate_parameters());
        let parameters = node.parameters().unwrap();
        assert_eq!(parameters.get("hue"), Some(&json!(360.0)));
        assert_eq!(parameters.get("sat"), Some(&json!(100.0)));
        assert_eq!(parameters.get("val"), Some(&json!(100.0)));
        assert_eq!(parameters.get("future_field"), Some(&json!("keep me")));

        let encoded = node.serialize_parameters().unwrap();
        let mut restored = Node::new_filter(FilterKind::Hsv);
        restored.restore_parameters(&encoded);
        assert_eq!(restored.serialize_parameters(), Some(encoded));
    }

    #[test]
    fn parameter_restore_keeps_defaults_and_non_parameter_nodes_are_explicit() {
        let mut node = Node::new_filter(FilterKind::Hsv);
        node.restore_parameters(&json!({ "hue": 42.0 }));
        let parameters = node.parameters().unwrap();
        assert_eq!(parameters.get("hue"), Some(&json!(42.0)));
        assert_eq!(parameters.get("sat"), Some(&json!(100.0)));
        assert_eq!(
            node.serialize_parameters().unwrap()["hue"],
            Value::from(42.0)
        );

        let mut route = Node::route();
        assert!(route.parameters().is_none());
        assert!(route.serialize_parameters().is_none());
        assert!(!route.validate_parameters());
    }

    #[test]
    fn parameter_validation_covers_colors_and_select_options() {
        let mut node = Node::new_filter(FilterKind::GradientMap);
        let parameters = node.parameters_mut().unwrap();
        parameters.insert("color1".into(), json!("not-a-color"));
        parameters.insert("reverse".into(), json!("maybe"));

        assert!(node.validate_parameters());
        let parameters = node.parameters().unwrap();
        assert_eq!(parameters.get("color1"), Some(&json!("#000000")));
        assert_eq!(parameters.get("reverse"), Some(&json!("no")));
    }

    #[test]
    fn cpu_preview_updates_one_bounded_evaluation_cache() {
        let mut node = Node::image("input.png");
        node.set_cpu(RgbaImage::new(1024, 512));

        assert_eq!(node.cpu().unwrap().dimensions(), (1024, 512));
        assert_eq!(node.eval_image().unwrap().dimensions(), (256, 128));
    }
}

impl Node {
    pub(crate) fn image(path: impl Into<String>) -> Self {
        Self::Image(ImageNode::new(path))
    }

    pub(crate) fn remove_bg() -> Self {
        Self::RemoveBg(Process::default())
    }

    pub(crate) fn remove_bg_v2() -> Self {
        Self::RemoveBgV2(Process::default())
    }

    pub(crate) fn face_restore() -> Self {
        Self::FaceRestore(FaceRestoreNode::default())
    }

    pub(crate) fn viewer() -> Self {
        Self::Output(ViewerNode::default())
    }

    #[allow(dead_code)]
    pub(crate) fn route() -> Self {
        Self::Route
    }

    pub(crate) fn group() -> Self {
        Self::Group(GroupNode::new(false))
    }

    pub(crate) fn blend() -> Self {
        Self::Blend(GroupNode::new(true))
    }

    #[allow(dead_code)]
    pub(crate) fn texts() -> Self {
        Self::Texts(TextsNode::new())
    }

    pub(crate) fn new_filter(kind: FilterKind) -> Self {
        Self::Filter(FilterNode::new(kind))
    }

    pub(crate) fn kind(&self) -> Kind {
        match self {
            Self::Image(_) => Kind::Image,
            Self::RemoveBg(_) => Kind::RemoveBg,
            Self::RemoveBgV2(_) => Kind::RemoveBgV2,
            Self::FaceRestore(_) => Kind::FaceRestore,
            Self::Output(_) => Kind::Output,
            Self::Route => Kind::Route,
            Self::Group(_) => Kind::Group,
            Self::Blend(_) => Kind::Blend,
            Self::Texts(_) => Kind::Texts,
            Self::Filter(node) => Kind::Filter(node.kind),
        }
    }

    /// A Route node is a pure pass-through: downstream nodes resolve its
    /// upstream wire as if the Route were not in the graph.
    pub(crate) fn is_route(&self) -> bool {
        matches!(self, Self::Route)
    }

    pub(crate) fn is_blend(&self) -> bool {
        matches!(self, Self::Blend(_))
    }

    pub(crate) fn is_evaluable(&self) -> bool {
        matches!(
            self,
            Self::Filter(_) | Self::Group(_) | Self::Blend(_) | Self::Texts(_)
        )
    }

    pub(crate) fn can_render_full_res(&self) -> bool {
        matches!(
            self,
            Self::Filter(_) | Self::Group(_) | Self::Blend(_) | Self::Texts(_)
        )
    }

    #[allow(dead_code)]
    pub(crate) fn filter(&self) -> Option<&FilterNode> {
        self.filter_node()
    }

    #[allow(dead_code)]
    pub(crate) fn filter_mut(&mut self) -> Option<&mut FilterNode> {
        self.filter_node_mut()
    }

    pub(crate) fn filter_node(&self) -> Option<&FilterNode> {
        match self {
            Self::Filter(node) => Some(node),
            _ => None,
        }
    }

    pub(crate) fn filter_node_mut(&mut self) -> Option<&mut FilterNode> {
        match self {
            Self::Filter(node) => Some(node),
            _ => None,
        }
    }

    pub(crate) fn group_node(&self) -> Option<&GroupNode> {
        match self {
            Self::Group(node) | Self::Blend(node) => Some(node),
            _ => None,
        }
    }

    pub(crate) fn group_node_mut(&mut self) -> Option<&mut GroupNode> {
        match self {
            Self::Group(node) | Self::Blend(node) => Some(node),
            _ => None,
        }
    }

    pub(crate) fn texts_node(&self) -> Option<&TextsNode> {
        match self {
            Self::Texts(node) => Some(node),
            _ => None,
        }
    }

    pub(crate) fn texts_node_mut(&mut self) -> Option<&mut TextsNode> {
        match self {
            Self::Texts(node) => Some(node),
            _ => None,
        }
    }

    /// Layers for nodes that act as a viewer (Output, Group, Blend).
    pub(crate) fn layers(&self) -> Option<&[ViewerLayer]> {
        match self {
            Self::Output(v) => Some(&v.layers),
            Self::Group(g) | Self::Blend(g) => Some(&g.layers),
            _ => None,
        }
    }

    pub(crate) fn layers_mut(&mut self) -> Option<&mut Vec<ViewerLayer>> {
        match self {
            Self::Output(v) => Some(&mut v.layers),
            Self::Group(g) | Self::Blend(g) => Some(&mut g.layers),
            _ => None,
        }
    }

    pub(crate) fn canvas_size(&self) -> egui::Vec2 {
        match self {
            Self::Output(v) => v.canvas_size,
            Self::Group(g) | Self::Blend(g) => g.canvas_size,
            _ => egui::Vec2::ZERO,
        }
    }

    pub(crate) fn set_canvas_size(&mut self, size: egui::Vec2) {
        match self {
            Self::Output(v) => v.canvas_size = size,
            Self::Group(g) | Self::Blend(g) => g.canvas_size = size,
            _ => {}
        }
    }

    pub(crate) fn image_node(&self) -> Option<&ImageNode> {
        match self {
            Self::Image(node) => Some(node),
            _ => None,
        }
    }

    pub(crate) fn image_node_mut(&mut self) -> Option<&mut ImageNode> {
        match self {
            Self::Image(node) => Some(node),
            _ => None,
        }
    }

    /// Shared processing-state accessor for Remove Background and Face Restore.
    pub(crate) fn process(&self) -> Option<&Process> {
        match self {
            Self::RemoveBg(process) => Some(process),
            Self::FaceRestore(node) => Some(&node.process),
            _ => None,
        }
    }

    pub(crate) fn process_mut(&mut self) -> Option<&mut Process> {
        match self {
            Self::RemoveBg(process) => Some(process),
            Self::RemoveBgV2(process) => Some(process),
            Self::FaceRestore(node) => Some(&mut node.process),
            _ => None,
        }
    }

    pub(crate) fn face_restore_node(&self) -> Option<&FaceRestoreNode> {
        match self {
            Self::FaceRestore(node) => Some(node),
            _ => None,
        }
    }

    pub(crate) fn face_restore_node_mut(&mut self) -> Option<&mut FaceRestoreNode> {
        match self {
            Self::FaceRestore(node) => Some(node),
            _ => None,
        }
    }

    pub(crate) fn viewer_node(&self) -> Option<&ViewerNode> {
        match self {
            Self::Output(node) => Some(node),
            _ => None,
        }
    }

    pub(crate) fn viewer_node_mut(&mut self) -> Option<&mut ViewerNode> {
        match self {
            Self::Output(node) => Some(node),
            _ => None,
        }
    }

    pub(crate) fn texture(&self) -> Option<&TextureHandle> {
        match self {
            Self::Image(node) => node.preview.texture.as_ref(),
            Self::RemoveBg(process)
            | Self::RemoveBgV2(process)
            | Self::FaceRestore(FaceRestoreNode { process, .. }) => {
                process.preview.texture.as_ref()
            }
            Self::Output(viewer) => viewer
                .layers
                .last()
                .and_then(|layer| layer.texture.as_ref()),
            Self::Route => None,
            Self::Group(group) | Self::Blend(group) => group.preview.texture.as_ref(),
            Self::Texts(texts) => texts.preview.texture.as_ref(),
            Self::Filter(node) => node.preview.texture.as_ref(),
        }
    }

    /// Whether to render the preview image in the node body.
    pub(crate) fn preview_enabled(&self) -> bool {
        match self {
            Self::Image(node) => node.preview_enabled,
            Self::RemoveBg(process)
            | Self::RemoveBgV2(process)
            | Self::FaceRestore(FaceRestoreNode { process, .. }) => process.preview_enabled,
            Self::Output(viewer) => viewer.preview_enabled,
            Self::Route => false,
            Self::Group(group) | Self::Blend(group) => group.preview_enabled,
            Self::Texts(texts) => texts.preview_enabled,
            Self::Filter(node) => node.preview_enabled,
        }
    }

    pub(crate) fn set_preview_enabled(&mut self, enabled: bool) {
        match self {
            Self::Image(node) => node.preview_enabled = enabled,
            Self::RemoveBg(process)
            | Self::RemoveBgV2(process)
            | Self::FaceRestore(FaceRestoreNode { process, .. }) => {
                process.preview_enabled = enabled
            }
            Self::Output(viewer) => viewer.preview_enabled = enabled,
            Self::Route => {}
            Self::Group(group) | Self::Blend(group) => group.preview_enabled = enabled,
            Self::Texts(texts) => texts.preview_enabled = enabled,
            Self::Filter(node) => node.preview_enabled = enabled,
        }
    }

    pub(crate) fn cpu(&self) -> Option<&RgbaImage> {
        match self {
            Self::Image(node) => node.preview.cpu.as_ref(),
            Self::RemoveBg(process)
            | Self::RemoveBgV2(process)
            | Self::FaceRestore(FaceRestoreNode { process, .. }) => process.preview.cpu.as_ref(),
            Self::Output(_) | Self::Route => None,
            Self::Group(group) | Self::Blend(group) => group.preview.cpu.as_ref(),
            Self::Texts(texts) => texts.preview.cpu.as_ref(),
            Self::Filter(node) => node.preview.cpu.as_ref(),
        }
    }

    /// Bytes held by this node's full-resolution CPU image, if any.
    pub(crate) fn cpu_bytes(&self) -> Option<usize> {
        self.cpu().map(|image| image.as_raw().len())
    }

    /// Path that can recreate this node's full-resolution image. Preview
    /// textures are kept when CPU data is released.
    pub(crate) fn reload_path(&self) -> Option<&Path> {
        match self {
            Self::Image(node) if !node.path.is_empty() => Some(Path::new(&node.path)),
            Self::RemoveBg(process) | Self::RemoveBgV2(process) => process.output_path.as_deref(),
            Self::FaceRestore(node) => node.process.output_path.as_deref(),
            Self::Filter(node) => node.output_path.as_deref(),
            Self::Group(node) | Self::Blend(node) => node.output_path.as_deref(),
            Self::Image(_) | Self::Output(_) | Self::Route | Self::Texts(_) => None,
        }
    }

    /// True when releasing the CPU image would not destroy the only copy.
    pub(crate) fn can_reload_image(&self) -> bool {
        self.reload_path().is_some_and(|path| path.is_file())
    }

    fn preview(&self) -> Option<&Preview> {
        match self {
            Self::Image(node) => Some(&node.preview),
            Self::RemoveBg(process) | Self::RemoveBgV2(process) => Some(&process.preview),
            Self::FaceRestore(node) => Some(&node.process.preview),
            Self::Output(_) | Self::Route => None,
            Self::Group(group) | Self::Blend(group) => Some(&group.preview),
            Self::Texts(texts) => Some(&texts.preview),
            Self::Filter(node) => Some(&node.preview),
        }
    }

    fn preview_mut(&mut self) -> Option<&mut Preview> {
        match self {
            Self::Image(node) => Some(&mut node.preview),
            Self::RemoveBg(process) | Self::RemoveBgV2(process) => Some(&mut process.preview),
            Self::FaceRestore(node) => Some(&mut node.process.preview),
            Self::Output(_) | Self::Route => None,
            Self::Group(group) | Self::Blend(group) => Some(&mut group.preview),
            Self::Texts(texts) => Some(&mut texts.preview),
            Self::Filter(node) => Some(&mut node.preview),
        }
    }

    /// Store a decoded full-resolution image without touching its preview
    /// texture.
    #[allow(dead_code)]
    pub(crate) fn set_cpu(&mut self, image: RgbaImage) {
        if let Some(preview) = self.preview_mut() {
            preview.set_cpu(image);
        }
    }

    /// Cached evaluation copy bounded by the normal preview resolution tier.
    pub(crate) fn eval_image(&self) -> Option<&RgbaImage> {
        self.preview().and_then(Preview::eval_image)
    }

    /// Replace both the CPU image and its downscaled preview texture.
    #[allow(dead_code)]
    pub(crate) fn set_image_data(&mut self, texture: TextureHandle, image: RgbaImage) {
        if let Some(preview) = self.preview_mut() {
            *preview = Preview::with_image(texture, image);
        }
    }

    /// Release the full-resolution CPU image while keeping the preview
    /// texture. Returns `true` when something was released.
    pub(crate) fn release_cpu(&mut self) -> bool {
        self.preview_mut()
            .and_then(|preview| preview.cpu.take())
            .is_some()
    }

    pub(crate) fn job(&self) -> Option<u64> {
        match self {
            Self::Filter(node) => node.job,
            Self::Group(node) | Self::Blend(node) => node.job,
            Self::Texts(node) => node.job,
            _ => self.process().and_then(|process| process.job),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn set_job(&mut self, job: Option<u64>) {
        match self {
            Self::Filter(node) => node.job = job,
            Self::Group(node) | Self::Blend(node) => node.job = job,
            Self::Texts(node) => node.job = job,
            _ => {
                if let Some(process) = self.process_mut() {
                    process.job = job;
                }
            }
        }
    }

    fn cancel_pending_job(&mut self) {
        let cancellation = match self {
            Self::Filter(node) => node.cancellation.take(),
            Self::Group(node) | Self::Blend(node) => node.cancellation.take(),
            Self::Texts(node) => node.cancellation.take(),
            _ => self
                .process_mut()
                .and_then(|process| process.cancellation.take()),
        };
        if let Some(cancellation) = cancellation {
            cancellation.cancel();
        }
    }

    pub(crate) fn render_state(&self) -> Option<RenderState> {
        match self {
            Self::Filter(node) => node.render_state,
            Self::Group(node) | Self::Blend(node) => node.render_state,
            Self::Texts(node) => node.render_state,
            _ => None,
        }
    }

    pub(crate) fn set_render_state(&mut self, state: Option<RenderState>) {
        match self {
            Self::Filter(node) => node.render_state = state,
            Self::Group(node) | Self::Blend(node) => node.render_state = state,
            Self::Texts(node) => node.render_state = state,
            _ => {}
        }
    }

    pub(crate) fn rendered(&self) -> Option<u64> {
        match self {
            Self::Filter(node) => node.rendered,
            Self::Group(node) | Self::Blend(node) => node.rendered,
            Self::Texts(node) => node.rendered,
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn set_rendered(&mut self, rendered: Option<u64>) {
        match self {
            Self::Filter(node) => node.rendered = rendered,
            Self::Group(node) | Self::Blend(node) => node.rendered = rendered,
            Self::Texts(node) => node.rendered = rendered,
            _ => {}
        }
    }

    pub(crate) fn timing(&self) -> Option<RunTiming> {
        self.process().and_then(|process| process.last_timing)
    }

    /// Output filepath that this node produces. Downstream nodes read this
    /// to load the full-resolution image for processing.
    pub(crate) fn output_path(&self) -> Option<&Path> {
        match self {
            Self::Image(node) if !node.path.is_empty() => Some(Path::new(&node.path)),
            Self::RemoveBg(process)
            | Self::RemoveBgV2(process)
            | Self::FaceRestore(FaceRestoreNode { process, .. }) => process.output_path.as_deref(),
            Self::Filter(node) => node.output_path.as_deref(),
            Self::Group(node) | Self::Blend(node) => node.output_path.as_deref(),
            Self::Texts(node) => node.output_path.as_deref(),
            _ => None,
        }
    }

    pub(crate) fn set_output_path(&mut self, path: Option<PathBuf>) {
        match self {
            Self::Filter(node) => node.output_path = path,
            Self::Group(node) | Self::Blend(node) => node.output_path = path,
            Self::Texts(node) => node.output_path = path,
            _ => {
                if let Some(process) = self.process_mut() {
                    process.output_path = path;
                }
            }
        }
    }

    pub(crate) fn error(&self) -> Option<&str> {
        match self {
            Self::Image(node) => node.preview.error.as_deref(),
            Self::RemoveBg(process)
            | Self::RemoveBgV2(process)
            | Self::FaceRestore(FaceRestoreNode { process, .. }) => {
                process.preview.error.as_deref()
            }
            Self::Output(_) | Self::Route => None,
            Self::Group(group) | Self::Blend(group) => group.preview.error.as_deref(),
            Self::Texts(texts) => texts.preview.error.as_deref(),
            Self::Filter(node) => node.preview.error.as_deref(),
        }
    }

    pub(crate) fn set_error(&mut self, message: impl Into<String>) {
        let msg = message.into();
        match self {
            Self::Image(node) => node.preview.error = Some(msg),
            Self::RemoveBg(process)
            | Self::RemoveBgV2(process)
            | Self::FaceRestore(FaceRestoreNode { process, .. }) => {
                process.preview.error = Some(msg);
            }
            Self::Output(_) | Self::Route => {}
            Self::Group(group) | Self::Blend(group) => group.preview.error = Some(msg),
            Self::Texts(texts) => texts.preview.error = Some(msg),
            Self::Filter(node) => node.preview.error = Some(msg),
        }
    }

    /// Apply a worker result, ignoring it if the node has been re-run since.
    pub(crate) fn finish_job(
        &mut self,
        token: u64,
        outcome: Result<(TextureHandle, JobOutput), String>,
    ) {
        if let Self::Filter(node) = self {
            if node.job != Some(token) {
                return;
            }
            node.job = None;
            node.cancellation = None;
            node.render_state = None;
            match outcome {
                Ok((texture, output)) => {
                    node.preview = Preview::with_image(texture, output.image);
                    node.output_path = output.path;
                }
                Err(message) => node.preview.error = Some(message),
            }
            return;
        }
        if let Self::Group(node) | Self::Blend(node) = self {
            if node.job != Some(token) {
                return;
            }
            node.job = None;
            node.cancellation = None;
            node.render_state = None;
            match outcome {
                Ok((texture, output)) => {
                    node.preview = Preview::with_image(texture, output.image);
                    node.output_path = output.path;
                }
                Err(message) => node.preview.error = Some(message),
            }
            return;
        }
        if let Self::Texts(node) = self {
            if node.job != Some(token) {
                return;
            }
            node.job = None;
            node.cancellation = None;
            node.render_state = None;
            match outcome {
                Ok((texture, output)) => {
                    node.preview = Preview::with_image(texture, output.image);
                    node.output_path = output.path;
                }
                Err(message) => node.preview.error = Some(message),
            }
            return;
        }
        let Some(process) = self.process_mut() else {
            return;
        };
        if process.job != Some(token) {
            return;
        }
        process.job = None;
        process.cancellation = None;
        match outcome {
            Ok((texture, output)) => {
                process.preview = Preview::with_image(texture, output.image);
                process.output_path = output.path;
                process.last_timing = Some(output.timing);
            }
            Err(message) => process.preview.error = Some(message),
        }
    }

    /// Drop transient worker/eval state so a duplicated node runs and
    /// evaluates independently of its source. Preview data is kept.
    pub(crate) fn reset_transient_state(&mut self) {
        self.cancel_pending_job();
        match self {
            Self::Filter(node) => {
                node.job = None;
                node.cancellation = None;
                node.render_state = None;
                node.rendered = None;
                node.signature = None;
            }
            Self::Group(node) | Self::Blend(node) => {
                node.job = None;
                node.cancellation = None;
                node.render_state = None;
                node.rendered = None;
                node.signature = None;
            }
            Self::Texts(node) => {
                node.job = None;
                node.cancellation = None;
                node.render_state = None;
                node.rendered = None;
                node.signature = None;
            }
            _ => {
                if let Some(process) = self.process_mut() {
                    process.job = None;
                    process.cancellation = None;
                }
            }
        }
    }

    /// Drop computed preview/data after an upstream change. Returns `true` if
    /// the node can have downstream consumers that should also be invalidated.
    pub(crate) fn invalidate_output(&mut self) -> bool {
        self.cancel_pending_job();
        match self {
            // An Image is a pass-through: downstream consumers must refresh
            // when either its selected asset or its upstream input changes.
            Self::Image(_) => true,
            Self::Route => true,
            Self::RemoveBg(process) => {
                process.clear_outputs();
                true
            }
            Self::RemoveBgV2(process) => {
                process.clear_outputs();
                true
            }
            Self::FaceRestore(node) => {
                node.process.clear_outputs();
                true
            }
            Self::Output(viewer) => {
                for layer in &mut viewer.layers {
                    layer.texture = None;
                }
                true
            }
            Self::Group(node) | Self::Blend(node) => {
                node.signature = None;
                node.job = None;
                node.cancellation = None;
                node.render_state = None;
                node.rendered = None;
                if node.output_path.take().is_some() {
                    node.preview.clear();
                }
                true
            }
            Self::Texts(node) => {
                node.signature = None;
                node.job = None;
                node.cancellation = None;
                node.render_state = None;
                node.rendered = None;
                if node.output_path.take().is_some() {
                    node.preview.clear();
                }
                true
            }
            Self::Filter(node) => {
                node.signature = None;
                node.job = None;
                node.cancellation = None;
                node.render_state = None;
                node.rendered = None;
                if node.output_path.take().is_some() {
                    node.preview.clear();
                }
                true
            }
        }
    }
}
