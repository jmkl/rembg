//! Evaluation and caching of generic filter nodes.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use egui::{TextureHandle, TextureOptions};
use egui_snarl::{InPinId, NodeId, Snarl};
use image::RgbaImage;

use crate::catalog;
use crate::model::{FilterKind, Kind, Node};
use crate::ops;
use crate::params::Params;

pub(crate) fn ensure_all(snarl: &mut Snarl<Node>, ctx: &egui::Context) -> bool {
    let ids: Vec<NodeId> = snarl
        .node_ids()
        .filter(|(_, node)| node.filter().is_some())
        .map(|(id, _)| id)
        .collect();
    let mut changed = false;
    for id in ids {
        if ensure_filter(snarl, id, ctx, &mut HashSet::new()) {
            changed = true;
        }
    }
    changed
}

fn ensure_filter(
    snarl: &mut Snarl<Node>,
    node: NodeId,
    ctx: &egui::Context,
    stack: &mut HashSet<NodeId>,
) -> bool {
    if !stack.insert(node) {
        return false;
    }

    let Some(filter) = snarl[node].filter() else {
        stack.remove(&node);
        return false;
    };
    let kind = filter.kind;
    let params = filter.params.clone();
    let cached_signature = filter.signature;
    let has_output = filter.preview.cpu.is_some();

    let signature = filter_signature(snarl, node, kind, &params, stack);

    let result = if has_output && cached_signature == Some(signature) {
        stack.remove(&node);
        return false;
    } else if kind == FilterKind::Group {
        // Composite the group's layers with their transforms.
        let canvas = {
            let c = snarl[node].canvas_size();
            if c.x > 0.0 && c.y > 0.0 {
                (c.x as u32, c.y as u32)
            } else {
                canvas_size(snarl)
            }
        };
        let layers = snarl[node].layers().map(|s| s.to_vec()).unwrap_or_default();
        let mut resolved: Vec<(crate::model::ViewerLayer, RgbaImage)> = Vec::new();
        for layer in &layers {
            if let Some(img) = resolve_image(snarl, layer.source, ctx, stack) {
                resolved.push((layer.clone(), img));
            }
        }
        ops::composite::composite_layers(&resolved, canvas)
    } else {
        let inputs = resolve_inputs(snarl, node, kind, ctx, stack);
        let canvas = canvas_size(snarl);
        ops::evaluate(kind, &params, &inputs, canvas)
    };

    let changed = match result {
        Ok(image) => {
            let texture = upload(ctx, node, &image);
            if let Some(filter) = snarl[node].filter_mut() {
                filter.preview.texture = Some(texture);
                filter.preview.cpu = Some(image);
                filter.preview.error = None;
                filter.signature = Some(signature);
            }
            true
        }
        Err(message) => {
            if let Some(filter) = snarl[node].filter_mut() {
                filter.preview.clear();
                filter.preview.error = Some(message);
                filter.signature = Some(signature);
            }
            true
        }
    };

    stack.remove(&node);
    changed
}

fn filter_signature(
    snarl: &Snarl<Node>,
    node: NodeId,
    kind: FilterKind,
    params: &Params,
    stack: &mut HashSet<NodeId>,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut hasher);
    if let Ok(serialized) = serde_json::to_string(params) {
        serialized.hash(&mut hasher);
    }
    canvas_size(snarl).hash(&mut hasher);

    if kind == FilterKind::Group {
        if let Some(layers) = snarl[node].layers() {
            for layer in layers {
                layer.source.0.hash(&mut hasher);
                node_signature(snarl, layer.source, stack).hash(&mut hasher);
                layer.center.x.to_bits().hash(&mut hasher);
                layer.center.y.to_bits().hash(&mut hasher);
                layer.size.x.to_bits().hash(&mut hasher);
                layer.size.y.to_bits().hash(&mut hasher);
                layer.angle.to_bits().hash(&mut hasher);
            }
        }
    }
    let spec = catalog::spec(Kind::Filter(kind));
    for input in 0..spec.sockets.len() {
        let remotes = snarl.in_pin(InPinId { node, input }).remotes.clone();
        for remote in remotes {
            node_signature(snarl, remote.node, stack).hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn node_signature(snarl: &Snarl<Node>, id: NodeId, stack: &mut HashSet<NodeId>) -> u64 {
    if !stack.insert(id) {
        return 0;
    }
    let signature = match &snarl[id] {
        Node::Filter(filter) => filter_signature(snarl, id, filter.kind, &filter.params, stack),
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
        Node::Output(_) => 0,
    };
    stack.remove(&id);
    signature
}

fn resolve_inputs(
    snarl: &mut Snarl<Node>,
    node: NodeId,
    kind: FilterKind,
    ctx: &egui::Context,
    stack: &mut HashSet<NodeId>,
) -> Vec<Vec<RgbaImage>> {
    let spec = catalog::spec(Kind::Filter(kind));
    let mut out = Vec::with_capacity(spec.sockets.len());
    for index in 0..spec.sockets.len() {
        let remotes = snarl.in_pin(InPinId { node, input: index }).remotes.clone();
        let mut images = Vec::new();
        for remote in remotes {
            if let Some(image) = resolve_image(snarl, remote.node, ctx, stack) {
                images.push(image);
            }
        }
        out.push(images);
    }
    out
}

fn resolve_image(
    snarl: &mut Snarl<Node>,
    id: NodeId,
    ctx: &egui::Context,
    stack: &mut HashSet<NodeId>,
) -> Option<RgbaImage> {
    if snarl[id].filter().is_some() {
        ensure_filter(snarl, id, ctx, stack);
        return snarl[id].cpu().cloned().map(|img| limit_image(&img));
    }

    if let Some(cpu) = snarl[id].cpu() {
        return Some(limit_image(cpu));
    }
    if let Some(path) = snarl[id].reload_path()
        && let Ok(image) = image::open(path)
    {
        return Some(limit_image(&image.to_rgba8()));
    }
    None
}

fn limit_image(image: &RgbaImage) -> RgbaImage {
    let (width, height) = image.dimensions();
    let longest = width.max(height);
    if longest <= ops::EVAL_MAX_EDGE {
        return image.clone();
    }
    let scale = ops::EVAL_MAX_EDGE as f64 / longest as f64;
    let target_width = ((width as f64 * scale).round() as u32).max(1);
    let target_height = ((height as f64 * scale).round() as u32).max(1);
    image::imageops::resize(
        image,
        target_width,
        target_height,
        image::imageops::FilterType::Triangle,
    )
}

fn upload(ctx: &egui::Context, node: NodeId, image: &RgbaImage) -> TextureHandle {
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
    }
    (1280, 720)
}
