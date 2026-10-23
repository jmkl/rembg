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
}

impl FilterNode {
    pub(crate) fn new(kind: FilterKind) -> Self {
        Self {
            kind,
            params: kind.defaults(),
            preview: Preview::default(),
            signature: None,
            preview_enabled: true,
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
        }
    }
}

/// Image preview shared by source and processed nodes.
#[derive(Clone, Default)]
pub(crate) struct Preview {
    pub texture: Option<TextureHandle>,
    pub cpu: Option<RgbaImage>,
    pub error: Option<String>,
}

impl Preview {
    pub(crate) fn with_image(texture: TextureHandle, cpu: RgbaImage) -> Self {
        Self {
            texture: Some(texture),
            cpu: Some(cpu),
            error: None,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.texture = None;
        self.cpu = None;
        self.error = None;
    }
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
    pub fn new(source: NodeId, texture: Option<TextureHandle>) -> Self {
        Self {
            source,
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
            Self::FaceRestore(_) => Kind::FaceRestore,
            Self::Output(_) => Kind::Output,
            Self::Filter(node) => Kind::Filter(node.kind),
        }
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
            Self::RemoveBg(process) | Self::FaceRestore(FaceRestoreNode { process, .. }) => {
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
            Self::RemoveBg(process) | Self::FaceRestore(FaceRestoreNode { process, .. }) => {
                process.preview_enabled
            }
            Self::Output(viewer) => viewer.preview_enabled,
            Self::Filter(node) => node.preview_enabled,
        }
    }

    pub(crate) fn set_preview_enabled(&mut self, enabled: bool) {
        match self {
            Self::Image(node) => node.preview_enabled = enabled,
            Self::RemoveBg(process) | Self::FaceRestore(FaceRestoreNode { process, .. }) => {
                process.preview_enabled = enabled
            }
            Self::Output(viewer) => viewer.preview_enabled = enabled,
            Self::Filter(node) => node.preview_enabled = enabled,
        }
    }

    pub(crate) fn cpu(&self) -> Option<&RgbaImage> {
        match self {
            Self::Image(node) => node.preview.cpu.as_ref(),
            Self::RemoveBg(process) | Self::FaceRestore(FaceRestoreNode { process, .. }) => {
                process.preview.cpu.as_ref()
            }
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
            Self::RemoveBg(process) => process.output_path.as_deref(),
            Self::FaceRestore(node) => node.process.output_path.as_deref(),
            Self::Image(_) | Self::Output(_) | Self::Filter(_) => None,
        }
    }

    /// True when releasing the CPU image would not destroy the only copy.
    pub(crate) fn can_reload_image(&self) -> bool {
        self.reload_path().is_some_and(|path| path.is_file())
    }

    fn preview_mut(&mut self) -> Option<&mut Preview> {
        match self {
            Self::Image(node) => Some(&mut node.preview),
            Self::RemoveBg(process) => Some(&mut process.preview),
            Self::FaceRestore(node) => Some(&mut node.process.preview),
            Self::Output(_) => None,
            Self::Filter(node) => Some(&mut node.preview),
        }
    }

    /// Store a decoded full-resolution image without touching its preview
    /// texture.
    pub(crate) fn set_cpu(&mut self, image: RgbaImage) {
        if let Some(preview) = self.preview_mut() {
            preview.cpu = Some(image);
        }
    }

    /// Replace both the CPU image and its downscaled preview texture.
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
        self.process().and_then(|process| process.job)
    }

    pub(crate) fn timing(&self) -> Option<RunTiming> {
        self.process().and_then(|process| process.last_timing)
    }

    /// Output filepath that this node produces. Downstream nodes read this
    /// to load the full-resolution image for processing.
    pub(crate) fn output_path(&self) -> Option<&Path> {
        match self {
            Self::Image(node) if !node.path.is_empty() => Some(Path::new(&node.path)),
            Self::RemoveBg(process) | Self::FaceRestore(FaceRestoreNode { process, .. }) => {
                process.output_path.as_deref()
            }
            _ => None,
        }
    }

    pub(crate) fn set_output_path(&mut self, path: Option<PathBuf>) {
        if let Some(process) = self.process_mut() {
            process.output_path = path;
        }
    }

    pub(crate) fn error(&self) -> Option<&str> {
        match self {
            Self::Image(node) => node.preview.error.as_deref(),
            Self::RemoveBg(process) | Self::FaceRestore(FaceRestoreNode { process, .. }) => {
                process.preview.error.as_deref()
            }
            Self::Output(_) => None,
            Self::Filter(node) => node.preview.error.as_deref(),
        }
    }

    pub(crate) fn set_error(&mut self, message: impl Into<String>) {
        match self {
            Self::Image(node) => node.preview.error = Some(message.into()),
            Self::RemoveBg(process) | Self::FaceRestore(FaceRestoreNode { process, .. }) => {
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
            // new signature, avoiding a one-frame flash while dragging.
            Self::Filter(node) => {
                node.signature = None;
                true
            }
        }
    }
}
