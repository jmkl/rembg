//! Evaluation and caching of generic filter nodes.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use egui::{TextureHandle, TextureOptions};
use egui_snarl::{InPinId, NodeId, Snarl};
use image::RgbaImage;

use crate::catalog;
use crate::model::{Kind, Node};
use crate::ops;
use crate::params;
use crate::params::Params;
pub(crate) use crate::resolution::PreviewQuality;

#[path = "eval/signature.rs"]
mod signature;

/// Evaluation reads graph topology through this interface so the evaluator is
/// not tied to Snarl's pin and node storage APIs.
trait EvalContext {
    fn node_ids(&self) -> Vec<NodeId>;
    fn node(&self, id: NodeId) -> Option<&Node>;
    fn node_mut(&mut self, id: NodeId) -> Option<&mut Node>;
    fn input_sources(&self, node: NodeId, input: usize) -> Vec<NodeId>;
    fn resolve_route(&self, id: NodeId) -> NodeId;
    fn canvas_size(&self) -> (u32, u32);
}

struct SnarlEvalContext<'a>(&'a mut Snarl<Node>);

impl EvalContext for SnarlEvalContext<'_> {
    fn node_ids(&self) -> Vec<NodeId> {
        self.0.node_ids().map(|(id, _)| id).collect()
    }
    fn node(&self, id: NodeId) -> Option<&Node> {
        self.0.get_node(id)
    }
    fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.0.get_node_mut(id)
    }
    fn input_sources(&self, node: NodeId, input: usize) -> Vec<NodeId> {
        self.0
            .in_pin(InPinId { node, input })
            .remotes
            .iter()
            .map(|r| r.node)
            .collect()
    }
    fn resolve_route(&self, id: NodeId) -> NodeId {
        crate::graph::resolve_route(self.0, id)
    }
    fn canvas_size(&self) -> (u32, u32) {
        canvas_size(self.0)
    }
}

pub(crate) fn ensure_all(snarl: &mut Snarl<Node>, ctx: &egui::Context) -> bool {
    ensure_all_with_quality(snarl, ctx, PreviewQuality::Normal)
}

#[allow(dead_code)] // UI callers currently select Normal; tests exercise all tiers.
pub(crate) fn ensure_all_with_quality(
    snarl: &mut Snarl<Node>,
    ctx: &egui::Context,
    quality: PreviewQuality,
) -> bool {
    let mut graph = SnarlEvalContext(snarl);
    let ids: Vec<NodeId> = graph
        .node_ids()
        .into_iter()
        .filter(|id| graph.node(*id).is_some_and(Node::is_evaluable))
        .collect();
    let mut changed = false;
    for id in ids {
        if ensure_filter(&mut graph, id, ctx, &mut HashSet::new(), quality) {
            changed = true;
        }
    }
    changed
}

fn ensure_filter(
    snarl: &mut dyn EvalContext,
    node: NodeId,
    ctx: &egui::Context,
    stack: &mut HashSet<NodeId>,
    quality: PreviewQuality,
) -> bool {
    if !stack.insert(node) {
        return false;
    }

    let Some(node_ref) = snarl.node(node) else {
        stack.remove(&node);
        return false;
    };
    let (kind, params, cached_signature, has_output) = match node_ref {
        Node::Filter(f) => (
            Kind::Filter(f.kind),
            f.params.clone(),
            f.signature,
            f.preview.cpu.is_some(),
        ),
        Node::Group(g) => (
            Kind::Group,
            g.params.clone(),
            g.signature,
            g.preview.cpu.is_some(),
        ),
        Node::Blend(g) => (
            Kind::Blend,
            g.params.clone(),
            g.signature,
            g.preview.cpu.is_some(),
        ),
        Node::Texts(t) => (
            Kind::Texts,
            t.params.clone(),
            t.signature,
            t.preview.cpu.is_some(),
        ),
        _ => {
            stack.remove(&node);
            return false;
        }
    };

    let bypassed = snarl.node(node).is_some_and(Node::bypassed);
    let signature = filter_signature(snarl, node, kind, &params, stack, quality)
        ^ if bypassed { 0x9e37_79b9_7f4a_7c15 } else { 0 };

    let result = if has_output && cached_signature == Some(signature) {
        stack.remove(&node);
        return false;
    } else if bypassed {
        let inputs = resolve_inputs(snarl, node, kind, ctx, stack, quality);
        inputs
            .into_iter()
            .flatten()
            .next()
            .ok_or_else(|| "Bypassed node has no input image".to_owned())
    } else if kind.is_layer_kind()
        && snarl
            .node(node)
            .and_then(Node::layers)
            .is_some_and(|l| !l.is_empty())
    {
        // Group and Blend composite their layers with their canvas transforms.
        let canvas = {
            let c = snarl.node(node).map(Node::canvas_size).unwrap_or_default();
            if c.x > 0.0 && c.y > 0.0 {
                (c.x as u32, c.y as u32)
            } else {
                snarl.canvas_size()
            }
        };
        let layers = snarl
            .node(node)
            .and_then(Node::layers)
            .map(|s| s.to_vec())
            .unwrap_or_default();
        let mut resolved: Vec<(crate::model::ViewerLayer, RgbaImage)> = Vec::new();
        for layer in &layers {
            let img = resolve_image(snarl, layer.source, ctx, stack, quality);
            if let Some(img) = img {
                resolved.push((layer.clone(), img));
            }
        }
        // Group stacks source-over; Blend applies its mode/opacity to every
        // layer above the bottom one.
        if kind == Kind::Blend {
            let mode = params::string(&params, "mode", "source-over");
            let opacity = params::f32(&params, "opacity", 100.0).clamp(0.0, 100.0) / 100.0;
            ops::composite::composite_layers_with(
                &resolved,
                canvas,
                Some(quality.max_edge() as u64),
                mode,
                opacity,
            )
        } else {
            if quality == PreviewQuality::Normal {
                ops::composite::composite_layers(&resolved, canvas)
            } else {
                ops::composite::composite_layers_with(
                    &resolved,
                    canvas,
                    Some(quality.max_edge() as u64),
                    "source-over",
                    1.0,
                )
            }
        }
    } else {
        let inputs = resolve_inputs(snarl, node, kind, ctx, stack, quality);
        // Text rasterization is presented across the document canvas and the
        // canvas applies its alpha-content crop in that same coordinate
        // space. Keep this preview at canvas resolution; resizing it here
        // makes the output layer visibly blurry until its worker render lands.
        let canvas = if kind == Kind::Texts {
            snarl.canvas_size()
        } else {
            quality.dimensions(snarl.canvas_size())
        };
        let spec = catalog::spec(kind);
        let eval_fn = match spec.eval {
            Some(f) => f,
            None => {
                stack.remove(&node);
                return false;
            }
        };
        eval_fn(&inputs, &params, canvas)
    };

    let changed = match result {
        Ok(image) => {
            let texture = upload(ctx, node, &image);
            if let Some(node) = snarl.node_mut(node) {
                match node {
                    Node::Filter(f) => {
                        f.preview.texture = Some(texture);
                        f.preview.set_cpu(image);
                        f.preview.error = None;
                        f.signature = Some(signature);
                    }
                    Node::Group(g) | Node::Blend(g) => {
                        g.preview.texture = Some(texture);
                        g.preview.set_cpu(image);
                        g.preview.error = None;
                        g.signature = Some(signature);
                    }
                    Node::Texts(t) => {
                        t.preview.texture = Some(texture);
                        t.preview.set_cpu(image);
                        t.preview.error = None;
                        t.signature = Some(signature);
                    }
                    _ => {}
                }
            }
            true
        }
        Err(message) => {
            if let Some(node) = snarl.node_mut(node) {
                match node {
                    Node::Filter(f) => {
                        f.preview.clear();
                        f.preview.error = Some(message);
                        f.signature = Some(signature);
                    }
                    Node::Group(g) | Node::Blend(g) => {
                        g.preview.clear();
                        g.preview.error = Some(message);
                        g.signature = Some(signature);
                    }
                    Node::Texts(t) => {
                        t.preview.clear();
                        t.preview.error = Some(message);
                        t.signature = Some(signature);
                    }
                    _ => {}
                }
            }
            true
        }
    };

    stack.remove(&node);
    changed
}

fn filter_signature(
    snarl: &mut dyn EvalContext,
    node: NodeId,
    kind: Kind,
    params: &Params,
    stack: &mut HashSet<NodeId>,
    quality: PreviewQuality,
) -> u64 {
    let mut layer_entries = Vec::new();
    if kind.is_layer_kind() {
        if let Some(layers) = snarl
            .node(node)
            .and_then(Node::layers)
            .map(|layers| layers.to_vec())
        {
            for layer in &layers {
                layer_entries.push(signature::LayerSignature {
                    source_id: layer.source.0,
                    source_signature: node_signature(snarl, layer.source, stack, quality),
                    center_x: layer.center.x.to_bits(),
                    center_y: layer.center.y.to_bits(),
                    size_x: layer.size.x.to_bits(),
                    size_y: layer.size.y.to_bits(),
                    angle: layer.angle.to_bits(),
                });
            }
        }
    }
    let spec = catalog::spec(kind);
    let mut inputs = Vec::new();
    for input in 0..spec.sockets.len() {
        let remotes = snarl.input_sources(node, input);
        for remote in remotes {
            inputs.push(node_signature(snarl, remote, stack, quality));
        }
    }
    let canvas = if kind.is_layer_kind() {
        snarl
            .node(node)
            .map(Node::canvas_size)
            .filter(|size| size.x > 0.0 && size.y > 0.0)
            .map(|size| (size.x.round() as u32, size.y.round() as u32))
            .unwrap_or_else(|| snarl.canvas_size())
    } else {
        snarl.canvas_size()
    };
    signature::compute(kind, params, canvas, quality, &layer_entries, &inputs)
}

fn node_signature(
    snarl: &mut dyn EvalContext,
    id: NodeId,
    stack: &mut HashSet<NodeId>,
    quality: PreviewQuality,
) -> u64 {
    // Route contributes no effect of its own: hash the node it forwards.
    let id = snarl.resolve_route(id);
    if !stack.insert(id) {
        return 0;
    }
    // Never clone the whole `Node` here: it owns pixel buffers and this runs
    // for every upstream node on every frame. Copy out only the parameters.
    let layered = match snarl.node(id) {
        None => {
            stack.remove(&id);
            return 0;
        }
        Some(Node::Filter(filter)) => Some((Kind::Filter(filter.kind), filter.params.clone())),
        Some(Node::Group(group)) => Some((Kind::Group, group.params.clone())),
        Some(Node::Blend(blend)) => Some((Kind::Blend, blend.params.clone())),
        Some(Node::Texts(texts)) => Some((Kind::Texts, texts.params.clone())),
        Some(_) => None,
    };
    if let Some((kind, params)) = layered {
        let signature = filter_signature(snarl, id, kind, &params, stack, quality);
        stack.remove(&id);
        return signature;
    }
    let Some(node) = snarl.node(id) else {
        stack.remove(&id);
        return 0;
    };
    let signature = match node {
        Node::Filter(_) | Node::Group(_) | Node::Blend(_) | Node::Texts(_) => 0,
        Node::Image(image) => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            image.asset_id.hash(&mut hasher);
            image.path.hash(&mut hasher);
            if let Some(texture) = &image.preview.texture {
                texture.id().hash(&mut hasher);
            }
            hasher.finish()
        }
        Node::RemoveBg(process) => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            process.output_path.hash(&mut hasher);
            if let Some(texture) = &process.preview.texture {
                texture.id().hash(&mut hasher);
            }
            process.job.hash(&mut hasher);
            hasher.finish()
        }
        Node::RemoveBgV2(process) => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            process.output_path.hash(&mut hasher);
            if let Some(texture) = &process.preview.texture {
                texture.id().hash(&mut hasher);
            }
            process.job.hash(&mut hasher);
            hasher.finish()
        }
        Node::FaceRestore(node) => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            node.strength.to_bits().hash(&mut hasher);
            node.only_center.hash(&mut hasher);
            node.process.output_path.hash(&mut hasher);
            if let Some(texture) = &node.process.preview.texture {
                texture.id().hash(&mut hasher);
            }
            hasher.finish()
        }
        Node::Output(_) | Node::Route(_) => 0,
    };
    stack.remove(&id);
    signature
}

fn resolve_inputs(
    snarl: &mut dyn EvalContext,
    node: NodeId,
    kind: Kind,
    ctx: &egui::Context,
    stack: &mut HashSet<NodeId>,
    quality: PreviewQuality,
) -> Vec<Vec<RgbaImage>> {
    let spec = catalog::spec(kind);
    let mut out = Vec::with_capacity(spec.sockets.len());
    for index in 0..spec.sockets.len() {
        let remotes = snarl.input_sources(node, index);
        let mut images = Vec::new();
        for remote in remotes {
            if let Some(image) = resolve_image(snarl, remote, ctx, stack, quality) {
                images.push(image);
            }
        }
        out.push(images);
    }
    out
}

fn resolve_image(
    snarl: &mut dyn EvalContext,
    id: NodeId,
    ctx: &egui::Context,
    stack: &mut HashSet<NodeId>,
    quality: PreviewQuality,
) -> Option<RgbaImage> {
    // A Route passes its input straight through, with no image of its own.
    let id = snarl.resolve_route(id);
    // Borrow instead of cloning the node (a clone would copy its pixel data).
    let evaluable = snarl.node(id)?.is_evaluable();
    if evaluable {
        ensure_filter(snarl, id, ctx, stack, quality);
        return snarl
            .node(id)
            .and_then(Node::cpu)
            .map(|image| bound_image(image, quality));
    }

    let node = snarl.node(id)?;
    if let Some(cpu) = node.cpu() {
        return Some(bound_image(cpu, quality));
    }
    if let Some(eval) = node.eval_image() {
        return Some(bound_image(eval, quality));
    }
    let reload_path = node.reload_path().map(|path| path.to_path_buf());
    if let Some(path) = reload_path
        && let Ok(image) = image::open(path)
    {
        let rgba = image.to_rgba8();
        if let Some(node_mut) = snarl.node_mut(id) {
            node_mut.set_cpu(rgba.clone());
        }
        return snarl
            .node(id)
            .and_then(Node::cpu)
            .map(|image| bound_image(image, quality));
    }
    None
}

fn bound_image(image: &RgbaImage, quality: PreviewQuality) -> RgbaImage {
    let (width, height) = quality.dimensions(image.dimensions());
    if (width, height) == image.dimensions() {
        image.clone()
    } else {
        image::imageops::resize(image, width, height, image::imageops::FilterType::Triangle)
    }
}

pub(crate) fn upload(ctx: &egui::Context, node: NodeId, image: &RgbaImage) -> TextureHandle {
    let (width, height) = image.dimensions();
    let color_image =
        egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], image.as_raw());
    ctx.load_texture(
        format!("filter-{}", node.0),
        color_image,
        TextureOptions::LINEAR,
    )
}

pub(crate) fn canvas_size(snarl: &Snarl<Node>) -> (u32, u32) {
    for (_, node) in snarl.node_ids() {
        if let Some(viewer) = node.viewer_node() {
            let width = viewer.canvas_size.x as u32;
            let height = viewer.canvas_size.y as u32;
            if width > 0 && height > 0 {
                return (width, height);
            }
        }
        if node.kind().is_layer_kind() {
            let cs = node.canvas_size();
            let width = cs.x as u32;
            let height = cs.y as u32;
            if width > 0 && height > 0 {
                return (width, height);
            }
        }
    }
    let settings = crate::settings::Settings::load();
    let w = settings.canvas_width.round().max(1.0) as u32;
    let h = settings.canvas_height.round().max(1.0) as u32;
    (w, h)
}
