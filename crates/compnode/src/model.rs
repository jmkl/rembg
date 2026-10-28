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

use crate::params::{self, Params};

/// Stable identity for every node kind. Used by the catalog and session layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum Kind {
    Image,
    RemoveBg,
    RemoveBgV2,
    FaceRestore,
    Output,
    Filter(FilterKind),
}

/// Every CPU-only filter/generator node type.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum FilterKind {
    Route,
    Group,
    Blend,
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
    Texts,
}

impl FilterKind {
    /// Stable string used in session files.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Route => "route",
            Self::Group => "mix",
            Self::Blend => "blend",
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
            Self::Texts => "texts",
        }
    }

    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "route" => Self::Route,
            // "mix" is the stored name for Group, kept for saved-graph parity.
            "mix" | "group" => Self::Group,
            "blend" => Self::Blend,
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
            "texts" | "text" => Self::Texts,
            _ => return None,
        })
    }

    /// Default parameters, mirroring `frontend/src/core/nodes.ts`.
    pub(crate) fn defaults(self) -> Params {
        let mut params = Params::new();
        match self {
            Self::Route | Self::Group => {}
            Self::Blend => {
                params::set_string(&mut params, "mode", "source-over");
                params::set_f32(&mut params, "opacity", 100.0);
            }
            Self::Mask => {
                params::set_string(&mut params, "invert", "dark");
                params::set_f32(&mut params, "strength", 100.0);
            }
            Self::Hsv => {
                params::set_f32(&mut params, "hue", 0.0);
                params::set_f32(&mut params, "sat", 100.0);
                params::set_f32(&mut params, "val", 100.0);
            }
            Self::Curves => {
                params::set_string(&mut params, "channel", "rgb");
                let mut curves = Params::new();
                for channel in ["rgb", "red", "green", "blue"] {
                    curves.insert(channel.to_owned(), params::identity_curve_value());
                }
                params.insert("curves".to_owned(), Value::Object(curves));
            }
            Self::Exposure => {
                params::set_f32(&mut params, "ev", 0.0);
                params::set_f32(&mut params, "offset", 0.0);
                params::set_f32(&mut params, "gamma", 1.0);
            }
            Self::ColorBalance => {
                params::set_string(&mut params, "range", "midtones");
                params::set_f32(&mut params, "cyanRed", 0.0);
                params::set_f32(&mut params, "magentaGreen", 0.0);
                params::set_f32(&mut params, "yellowBlue", 0.0);
                params::set_string(&mut params, "preserve", "yes");
            }
            Self::GradientMap => {
                params::set_string(&mut params, "color1", "#000000");
                params::set_string(&mut params, "color2", "#808080");
                params::set_string(&mut params, "color3", "#ffffff");
                params::set_f32(&mut params, "midpoint", 50.0);
                params::set_string(&mut params, "reverse", "no");
            }
            Self::Invert => params::set_f32(&mut params, "amount", 100.0),
            Self::Gradient => {
                params::set_string(&mut params, "mode", "linear");
                params::set_string(&mut params, "color1", "#000000");
                params::set_string(&mut params, "color2", "#ffffff");
                params::set_f32(&mut params, "angle", 90.0);
                params::set_f32(&mut params, "color1Position", -200.0);
                params::set_f32(&mut params, "color2Position", 200.0);
                params::set_u32(&mut params, "width", 1280);
                params::set_u32(&mut params, "height", 720);
            }
            Self::Noise => {
                params::set_string(&mut params, "kind", "fbm");
                params::set_f32(&mut params, "scale", 5.0);
                params::set_f32(&mut params, "detail", 4.0);
                params::set_f32(&mut params, "roughness", 0.5);
                params::set_f32(&mut params, "lacunarity", 2.0);
                params::set_f32(&mut params, "distortion", 0.0);
                params::set_f32(&mut params, "seed", 0.0);
            }
            Self::RawFilter => {
                params::set_f32(&mut params, "temp", 0.0);
                params::set_f32(&mut params, "tint", 0.0);
                params::set_f32(&mut params, "texture", 0.0);
                params::set_f32(&mut params, "clarity", 0.0);
                params::set_f32(&mut params, "dehaze", 0.0);
                params::set_f32(&mut params, "sharpen", 0.0);
                params::set_f32(&mut params, "sharpen_radius", 1.0);
                params::set_f32(&mut params, "sharpen_detail", 25.0);
                params::set_f32(&mut params, "noise_reduction", 0.0);
                params::set_f32(&mut params, "noise_reduction_detail", 50.0);
            }
            Self::Texts => {
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
            }
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
    /// Viewer-style layers used by the Group node. Empty for other filters.
    pub layers: Vec<ViewerLayer>,
    pub canvas_size: egui::Vec2,
    /// Token of the in-flight full-resolution render, if any.
    pub job: Option<u64>,
    /// Absolute path to the persisted full-resolution composite on disk.
    pub output_path: Option<PathBuf>,
    /// Debounce state for scheduling the full-resolution render.
    pub render_state: Option<RenderState>,
    /// Eval signature whose full-resolution render has been claimed (in flight
    /// or completed). Prevents re-submitting identical renders every frame.
    pub rendered: Option<u64>,
}

/// Debounce bookkeeping for the Group full-resolution render: the eval
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
            layers: Vec::new(),
            canvas_size: egui::Vec2::ZERO,
            job: None,
            output_path: None,
            render_state: None,
            rendered: None,
        }
    }

    /// Restore a saved node, filling any newly added parameter with its default.
    pub(crate) fn restore(kind: FilterKind, stored: Option<Params>) -> Self {
        let mut params = kind.defaults();
        if let Some(stored) = stored {
            for (key, value) in stored {
                params.insert(key, value);
            }
        }
        Self {
            kind,
            params,
            preview: Preview::default(),
            signature: None,
            preview_enabled: true,
            layers: Vec::new(),
            canvas_size: egui::Vec2::ZERO,
            job: None,
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
    /// Evaluation-resolution copy (at most `ops::EVAL_MAX_EDGE` on the longest
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
    if longest <= crate::ops::EVAL_MAX_EDGE {
        return image.clone();
    }
    let scale = crate::ops::EVAL_MAX_EDGE as f64 / longest as f64;
    let width = ((width as f64 * scale).round() as u32).max(1);
    let height = ((height as f64 * scale).round() as u32).max(1);
    image::imageops::thumbnail(image, width, height)
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
    Filter(FilterNode),
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
            Self::Filter(node) => Kind::Filter(node.kind),
        }
    }

    /// A Route node is a pure pass-through: downstream nodes resolve its
    /// upstream wire as if the Route were not in the graph.
    pub(crate) fn is_route(&self) -> bool {
        matches!(self, Self::Filter(node) if node.kind == FilterKind::Route)
    }

    pub(crate) fn filter(&self) -> Option<&FilterNode> {
        self.filter_node()
    }

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

    /// Layers for nodes that act as a viewer (Output, or Group).
    pub(crate) fn layers(&self) -> Option<&[ViewerLayer]> {
        match self {
            Self::Output(v) => Some(&v.layers),
            Self::Filter(f) if f.kind == FilterKind::Group => Some(&f.layers),
            _ => None,
        }
    }

    pub(crate) fn layers_mut(&mut self) -> Option<&mut Vec<ViewerLayer>> {
        match self {
            Self::Output(v) => Some(&mut v.layers),
            Self::Filter(f) if f.kind == FilterKind::Group => Some(&mut f.layers),
            _ => None,
        }
    }

    pub(crate) fn canvas_size(&self) -> egui::Vec2 {
        match self {
            Self::Output(v) => v.canvas_size,
            Self::Filter(f) if f.kind == FilterKind::Group => f.canvas_size,
            _ => egui::Vec2::ZERO,
        }
    }

    pub(crate) fn set_canvas_size(&mut self, size: egui::Vec2) {
        match self {
            Self::Output(v) => v.canvas_size = size,
            Self::Filter(f) if f.kind == FilterKind::Group => f.canvas_size = size,
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
            Self::Filter(node) => node.preview_enabled = enabled,
        }
    }

    pub(crate) fn cpu(&self) -> Option<&RgbaImage> {
        match self {
            Self::Image(node) => node.preview.cpu.as_ref(),
            Self::RemoveBg(process)
            | Self::RemoveBgV2(process)
            | Self::FaceRestore(FaceRestoreNode { process, .. }) => process.preview.cpu.as_ref(),
            Self::Output(_) => None,
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
            Self::Image(_) | Self::Output(_) => None,
        }
    }

    /// True when releasing the CPU image would not destroy the only copy.
    pub(crate) fn can_reload_image(&self) -> bool {
        self.reload_path().is_some_and(|path| path.is_file())
    }

    fn preview_mut(&mut self) -> Option<&mut Preview> {
        match self {
            Self::Image(node) => Some(&mut node.preview),
            Self::RemoveBg(process) | Self::RemoveBgV2(process) => Some(&mut process.preview),
            Self::FaceRestore(node) => Some(&mut node.process.preview),
            Self::Output(_) => None,
            Self::Filter(node) => Some(&mut node.preview),
        }
    }

    /// Store a decoded full-resolution image without touching its preview
    /// texture.
    #[allow(dead_code)]
    pub(crate) fn set_cpu(&mut self, image: RgbaImage) {
        if let Some(preview) = self.preview_mut() {
            preview.cpu = Some(image);
        }
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
            _ => self.process().and_then(|process| process.job),
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
            _ => None,
        }
    }

    pub(crate) fn set_output_path(&mut self, path: Option<PathBuf>) {
        match self {
            Self::Filter(node) => node.output_path = path,
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
            Self::Output(_) => None,
            Self::Filter(node) => node.preview.error.as_deref(),
        }
    }

    pub(crate) fn set_error(&mut self, message: impl Into<String>) {
        match self {
            Self::Image(node) => node.preview.error = Some(message.into()),
            Self::RemoveBg(process)
            | Self::RemoveBgV2(process)
            | Self::FaceRestore(FaceRestoreNode { process, .. }) => {
                process.preview.error = Some(message.into());
            }
            Self::Output(_) => {}
            Self::Filter(node) => node.preview.error = Some(message.into()),
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
        match outcome {
            Ok((texture, output)) => {
                process.preview.texture = Some(texture);
                process.preview.cpu = Some(output.image);
                process.preview.error = None;
                process.output_path = output.path;
                process.last_timing = Some(output.timing);
            }
            Err(message) => process.preview.error = Some(message),
        }
    }

    /// Drop transient worker/eval state so a duplicated node runs and
    /// evaluates independently of its source. Preview data is kept.
    pub(crate) fn reset_transient_state(&mut self) {
        if let Self::Filter(node) = self {
            node.job = None;
            node.render_state = None;
            node.rendered = None;
            node.signature = None;
            return;
        }
        if let Some(process) = self.process_mut() {
            process.job = None;
        }
    }

    /// Drop computed preview/data after an upstream change. Returns `true` if
    /// the node can have downstream consumers that should also be invalidated.
    pub(crate) fn invalidate_output(&mut self) -> bool {
        match self {
            // An Image is a pass-through: downstream consumers must refresh
            // when either its selected asset or its upstream input changes.
            Self::Image(_) => true,
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
            // Filters keep their previous preview until `eval` recomputes the
            // new signature, avoiding a one-frame flash while dragging. A
            // persisted Group render is dropped together with its pending job.
            Self::Filter(node) => {
                node.signature = None;
                node.job = None;
                node.render_state = None;
                // The full-resolution render must be re-claimed even when the
                // signature later returns to an earlier value.
                node.rendered = None;
                if node.output_path.take().is_some() {
                    node.preview.clear();
                }
                true
            }
        }
    }
}
