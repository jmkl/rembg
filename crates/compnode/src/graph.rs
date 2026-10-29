//! Operations on the Snarl graph: loading images, connecting wires,
//! invalidating stale outputs and refreshing Viewer layers.

use std::borrow::Cow;
use std::path::PathBuf;

use egui::TextureHandle;
use egui_snarl::{InPinId, NodeId, OutPinId, Snarl};

use crate::assets::AssetStore;
use crate::catalog;
use crate::memory::ImageMemory;
use crate::model::{FilterKind, Kind, Node, Preview, ViewerLayer};

/// Ensure the graph has exactly one Output node, creating one if missing and
/// removing any extras (dropping their wires).
pub(crate) fn ensure_output_node(snarl: &mut Snarl<Node>) {
    let output_ids: Vec<NodeId> = snarl
        .node_ids()
        .filter(|(_, node)| node.viewer_node().is_some())
        .map(|(id, _)| id)
        .collect();
    if output_ids.is_empty() {
        snarl.insert_node(egui::pos2(400.0, 200.0), Node::viewer());
        return;
    }
    for id in output_ids.iter().skip(1) {
        snarl.remove_node(*id);
    }
}

/// Longest edge of the GPU preview texture. Full-resolution pixels stay in
/// CPU memory for processing; uploading them all would only waste VRAM.
pub(crate) const PREVIEW_MAX_EDGE: u32 = 256;

pub(crate) fn preview_dimensions(width: u32, height: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= PREVIEW_MAX_EDGE {
        return (width, height);
    }

    let scale = PREVIEW_MAX_EDGE as f64 / longest as f64;
    (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    )
}

/// Upload a downscaled texture for display. The caller keeps the original
/// bitmap for inference.
pub(crate) fn rgba_to_texture(ctx: &egui::Context, img: &image::RgbaImage) -> egui::TextureHandle {
    let (width, height) = preview_dimensions(img.width(), img.height());
    let preview = if (width, height) == img.dimensions() {
        Cow::Borrowed(img)
    } else {
        Cow::Owned(image::imageops::thumbnail(img, width, height))
    };
    let size = [preview.width() as usize, preview.height() as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, preview.as_raw());
    ctx.load_texture("node-preview", color_image, egui::TextureOptions::LINEAR)
}

/// First node wired into `node`'s primary input, without any Route mapping.
pub(crate) fn first_input(snarl: &Snarl<Node>, node: NodeId) -> Option<NodeId> {
    let pin = snarl.in_pin(InPinId { node, input: 0 });
    pin.remotes.first().map(|remote| remote.node)
}

/// Follow a chain of Route nodes to the node that actually produces content.
///
/// Route is a pass-through: `image -> Route -> curve` must resolve exactly like
/// `image -> curve`. Non-Route nodes and unconnected Routes resolve to
/// themselves. Cycles are cut off at the first repeated Route.
pub(crate) fn resolve_route(snarl: &Snarl<Node>, node: NodeId) -> NodeId {
    let mut current = node;
    let mut seen = std::collections::HashSet::new();
    while snarl.get_node(current).is_some_and(|n| n.is_route()) {
        if !seen.insert(current) {
            return current;
        }
        match first_input(snarl, current) {
            Some(next) => current = next,
            None => return current,
        }
    }
    current
}

/// Source of `node`'s primary input, with any Route chain resolved away.
pub(crate) fn upstream(snarl: &Snarl<Node>, node: NodeId) -> Option<NodeId> {
    let remote = first_input(snarl, node)?;
    Some(resolve_route(snarl, remote))
}

/// Preview of the node feeding `node`'s first input, if that source has one.
/// Used as the fallback preview for processing nodes that have not run yet.
pub(crate) fn upstream_texture(snarl: &Snarl<Node>, node: NodeId) -> Option<egui::TextureHandle> {
    let upstream = upstream(snarl, node)?;
    effective_texture(snarl, upstream)
}

/// Output filepath that `node` (or its nearest upstream ancestor) produces.
/// Walk the wire upstream until we find a node that has written an output path.
pub(crate) fn effective_output_path(snarl: &Snarl<Node>, node: NodeId) -> Option<PathBuf> {
    let mut current = node;
    let mut seen = std::collections::HashSet::new();
    loop {
        if !seen.insert(current) {
            return None;
        }
        let Some(node_ref) = snarl.get_node(current) else {
            return None;
        };
        // Route produces no output of its own; keep walking through it.
        if !node_ref.is_route()
            && let Some(path) = node_ref.output_path()
        {
            return Some(path.to_path_buf());
        }
        if let Some(upstream) = upstream(snarl, current) {
            current = upstream;
        } else {
            return None;
        }
    }
}

/// Full-resolution file for a node used as a render input.
///
/// A filter must have rendered its own output: walking past it would silently
/// use the unfiltered upstream image instead of the filter result. Image nodes
/// are pass-through while an upstream wire exists, so the wire is followed
/// before falling back to their own file.
pub(crate) fn full_res_source_path(snarl: &Snarl<Node>, node: NodeId) -> Option<PathBuf> {
    let mut current = node;
    let mut seen = std::collections::HashSet::new();
    loop {
        if !seen.insert(current) {
            return None;
        }
        let Some(node_ref) = snarl.get_node(current) else {
            return None;
        };
        // A Route has no output file of its own; downstream consumers must
        // read the pass-through input exactly as if Route were not there.
        if node_ref.is_route() {
            match upstream(snarl, current) {
                Some(next) => {
                    current = next;
                    continue;
                }
                None => return None,
            }
        }
        match node_ref {
            Node::Filter(_) => {
                return node_ref.output_path().map(|path| path.to_path_buf());
            }
            Node::Image(_) => match upstream(snarl, current) {
                Some(upstream) => current = upstream,
                None => return node_ref.output_path().map(|path| path.to_path_buf()),
            },
            _ => return effective_output_path(snarl, current),
        }
    }
}

/// Texture handle that `node` (or its nearest upstream ancestor) produces.
/// Walk the wire upstream until we find a node that has rendered a preview.
pub(crate) fn effective_texture(snarl: &Snarl<Node>, node: NodeId) -> Option<egui::TextureHandle> {
    let mut current = node;
    let mut seen = std::collections::HashSet::new();
    loop {
        if !seen.insert(current) {
            return None;
        }
        let Some(node_ref) = snarl.get_node(current) else {
            return None;
        };
        // Never show a Route's own (possibly stale) texture: its content is
        // whatever its upstream produces.
        if !node_ref.is_route()
            && let Some(tex) = node_ref.texture()
        {
            return Some(tex.clone());
        }
        if let Some(upstream) = upstream(snarl, current) {
            current = upstream;
        } else {
            return None;
        }
    }
}

/// Connect an output to an input.
///
/// Single-input nodes replace an existing wire. Multi-input nodes (currently
/// the Viewer's `Layers` socket) keep every connection.
pub(crate) fn connect_input(snarl: &mut Snarl<Node>, from: OutPinId, to: InPinId) -> bool {
    let Some(to_node) = snarl.get_node(to.node) else {
        return false;
    };
    if !catalog::spec(to_node.kind()).multi_input {
        let existing = snarl.in_pin(to).remotes.clone();
        for remote in existing {
            snarl.disconnect(remote, to);
        }
    }
    snarl.connect(from, to)
}

/// Read an image node from disk or its embedded asset once; the result is
/// cached on the node. Upstream-connected Image nodes are pass-through nodes
/// and do not need their own asset decoded.
pub(crate) fn ensure_image_loaded(
    snarl: &mut Snarl<Node>,
    node: NodeId,
    ctx: &egui::Context,
    assets: &mut AssetStore,
) {
    let Some(image) = snarl.get_node(node).and_then(|n| n.image_node()) else {
        return;
    };
    if image.preview.texture.is_some() || image.preview.error.is_some() {
        return;
    }
    if upstream(snarl, node).is_some() {
        return;
    }

    let asset_id = image.asset_id.clone();
    let path = image.path.clone();

    let decoded = if let Some(asset_id) = asset_id.as_deref() {
        assets.get(asset_id).map(|asset| asset.decode())
    } else if !path.is_empty() {
        Some(
            image::open(&path)
                .map(|img| img.to_rgba8())
                .map_err(|err| err.to_string()),
        )
    } else {
        None
    };

    match decoded {
        Some(Ok(rgba)) => {
            let texture = rgba_to_texture(ctx, &rgba);
            if let Some(image) = snarl.get_node_mut(node).and_then(|n| n.image_node_mut()) {
                image.preview = Preview::with_image(texture, rgba);
            }
            // Adopt file-backed assets on first load so they become portable.
            if asset_id.is_none()
                && !path.is_empty()
                && let Ok(id) = assets.insert_path(&path)
                && let Some(image) = snarl.get_node_mut(node).and_then(|n| n.image_node_mut())
            {
                image.asset_id = Some(id);
            }
        }
        Some(Err(err)) => {
            if let Some(node_mut) = snarl.get_node_mut(node) {
                node_mut.set_error(format!("Could not open image: {err}"));
            }
        }
        None => {}
    }
}

/// Image that a processing node should consume, or a message explaining why
/// there is none yet.
#[allow(dead_code)]
pub(crate) fn upstream_image(
    snarl: &mut Snarl<Node>,
    node: NodeId,
    ctx: &egui::Context,
    memory: &mut ImageMemory,
    assets: &mut AssetStore,
) -> Result<image::RgbaImage, String> {
    // Walk upstream through ALL node types to find a filepath.
    // Any node (Image, RemoveBg, FaceRestore) can produce an output path.
    let mut up = upstream(snarl, node).ok_or_else(|| "No input connected".to_owned())?;
    let mut seen = std::collections::HashSet::new();
    while snarl.get_node(up).and_then(|n| n.output_path()).is_none() {
        if !seen.insert(up) {
            return Err("Image input cycle detected".to_owned());
        }
        if let Some(next) = upstream(snarl, up) {
            up = next;
        } else {
            break;
        }
    }

    let up_node = snarl
        .get_node(up)
        .ok_or_else(|| "Upstream node not found".to_owned())?;
    let path = up_node
        .output_path()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "Run the upstream node first".to_owned())?;

    ensure_image_loaded(snarl, up, ctx, assets);
    memory.touch(up);

    let up_node = snarl
        .get_node(up)
        .ok_or_else(|| "Upstream node not found".to_owned())?;
    if let Some(image) = up_node.cpu() {
        return Ok(image.clone());
    }
    if let Some(error) = up_node.error() {
        return Err(error.to_owned());
    }
    if up_node.job().is_some() {
        return Err("The upstream node is still processing".to_owned());
    }

    // Load the image from the upstream filepath.
    let image = image::open(&path)
        .map_err(|err| format!("Could not load {}: {err}", path.display()))?
        .to_rgba8();
    Ok(image)
}

/// Drop a node's own output and everything downstream of it. Used when an
/// input wire changes, including connect and disconnect.
pub(crate) fn invalidate_node(snarl: &mut Snarl<Node>, node: NodeId) {
    if let Some(n) = snarl.get_node_mut(node) {
        n.invalidate_output();
    }
    invalidate_downstream(snarl, node);
}

/// Drop outputs of nodes downstream of `node`, so a changed input never shows
/// stale results. Any in-flight job for those nodes is discarded too.
pub(crate) fn invalidate_downstream(snarl: &mut Snarl<Node>, node: NodeId) {
    let mut stack = vec![node];
    let mut seen = std::collections::HashSet::new();
    while let Some(current) = stack.pop() {
        if !seen.insert(current) {
            continue;
        }
        let remotes = snarl
            .out_pin(OutPinId {
                node: current,
                output: 0,
            })
            .remotes;
        for remote in remotes {
            let down = remote.node;
            let can_propagate = if let Some(down_node) = snarl.get_node_mut(down) {
                down_node.invalidate_output() && catalog::spec(down_node.kind()).outputs > 0
            } else {
                false
            };
            if can_propagate {
                stack.push(down);
            }
        }
    }
}

/// Viewer nodes mirror every upstream output connected to their `Layers`
/// socket. Existing layer order is kept; newly discovered connections are
/// appended (and sorted by node id for deterministic session loading).
pub(crate) fn refresh_viewer(
    snarl: &mut Snarl<Node>,
    node: NodeId,
    ctx: &egui::Context,
    assets: &mut AssetStore,
) {
    let Some(viewer_ref) = snarl.get_node(node) else {
        return;
    };
    let sockets = catalog::spec(viewer_ref.kind()).sockets.len();
    let is_blend = viewer_ref.is_blend();

    // Build the set of source node IDs that should have layers.
    // For Blend: Socket 1 is Bottom Layer (layer 0/base), Socket 0 is Top Layer (layer 1/overlay).
    // For Viewer/Group: sockets are checked in socket order.
    let mut sources: Vec<NodeId> = Vec::new();
    let socket_order: Vec<usize> = if is_blend && sockets == 2 {
        vec![1, 0]
    } else {
        (0..sockets).collect()
    };
    for input in socket_order {
        let remotes = snarl.in_pin(InPinId { node, input }).remotes.clone();
        for remote in remotes {
            let source = resolve_route(snarl, remote.node);
            if snarl.get_node(source).is_some() && !sources.contains(&source) {
                sources.push(source);
            }
        }
    }

    // Existing layers are matched by their resolved source, so inserting or
    // removing a Route keeps the layer identity (and its user transform).
    let existing_sources: Vec<NodeId> = snarl
        .get_node(node)
        .and_then(|n| n.layers())
        .map(|layers| {
            layers
                .iter()
                .map(|layer| resolve_route(snarl, layer.source))
                .collect()
        })
        .unwrap_or_default();

    // Phase 1: Load any missing images (mutable borrow of snarl).
    for source in &sources {
        ensure_image_loaded(snarl, *source, ctx, assets);
    }

    // Phase 2: Read all texture data immutably (before borrowing viewer mutably).
    let mut layer_data: Vec<(NodeId, Kind, String, Option<PathBuf>, Option<TextureHandle>)> =
        Vec::with_capacity(sources.len());
    for source in &sources {
        let Some(source_node) = snarl.get_node(*source) else {
            continue;
        };
        let spec = catalog::spec(source_node.kind());
        let source_name = spec.title.to_string();
        let source_kind = spec.kind;
        let source_path = source_node.output_path().map(|p| p.to_path_buf());
        let existing_tex = effective_texture(snarl, *source);
        layer_data.push((*source, source_kind, source_name, source_path, existing_tex));
    }

    // Phase 3: Get the layers mutably and update.
    let Some(viewer_layers) = snarl[node].layers_mut() else {
        return;
    };

    // If the set of sources hasn't changed, just update textures for existing
    // layers. A layer created before this Route-aware matching may still store
    // the Route id, so repair `source` while we are here.
    if sources.len() == existing_sources.len()
        && sources.iter().all(|s| existing_sources.contains(s))
    {
        for (source, kind, source_name, source_path, new_tex) in &layer_data {
            let Some(index) = existing_sources.iter().position(|s| s == source) else {
                continue;
            };
            let layer = &mut viewer_layers[index];
            if layer.source != *source {
                layer.source = *source;
            }
            if layer.kind != *kind {
                layer.kind = *kind;
            }
            if layer.texture != *new_tex {
                layer.texture = new_tex.clone();
            }
            // A processing node that just finished has a new output file;
            // without this the sidebar canvas keeps loading the stale one.
            if layer.source_path != *source_path {
                layer.source_path = source_path.clone();
            }
            if layer.source_name != *source_name {
                layer.source_name = source_name.clone();
            }
        }
        return;
    }

    // Build new layers list, reusing existing layer data where possible.
    let mut new_layers = Vec::with_capacity(sources.len());
    for (source, kind, source_name, source_path, new_tex) in &layer_data {
        let source = *source;
        // Reuse existing layer data if we already had this source (possibly
        // through a Route).
        let mut layer = existing_sources
            .iter()
            .position(|s| *s == source)
            .map(|index| viewer_layers[index].clone())
            .unwrap_or_else(|| ViewerLayer {
                source,
                kind: *kind,
                texture: None,
                source_name: source_name.clone(),
                source_path: source_path.clone(),
                center: egui::Vec2::ZERO,
                size: egui::Vec2::ZERO,
                angle: 0.0,
            });
        layer.source = source;

        // Update texture if changed (texture was pre-fetched in phase 2).
        if layer.texture != *new_tex {
            layer.texture = new_tex.clone();
        }

        new_layers.push(layer);
    }

    *viewer_layers = new_layers;
}

/// Dimensions of the image a node effectively outputs.
pub(crate) fn effective_dimensions(
    snarl: &Snarl<Node>,
    node: NodeId,
    assets: &AssetStore,
) -> Option<(u32, u32)> {
    let mut current = resolve_route(snarl, node);
    let mut seen = std::collections::HashSet::new();
    while seen.insert(current) {
        let Some(n) = snarl.get_node(current) else {
            return None;
        };
        // 1. If the node has an existing on-disk output file that exists, use its dimensions.
        if let Some(path) = n.output_path().or_else(|| n.reload_path()) {
            if let Ok(dims) = image::image_dimensions(path) {
                return Some(dims);
            }
        }
        // 2. Node-specific resolution inspection.
        match n {
            Node::Image(image) => {
                if let Some(up) = upstream(snarl, current) {
                    current = resolve_route(snarl, up);
                    continue;
                }
                if let Some(asset_id) = image.asset_id.as_deref() {
                    if let Some(dims) = assets.dimensions(asset_id) {
                        return Some(dims);
                    }
                }
                if !image.path.is_empty() {
                    if let Ok(dims) = image::image_dimensions(&image.path) {
                        return Some(dims);
                    }
                }
                if let Some(cpu) = image.preview.cpu.as_ref() {
                    return Some(cpu.dimensions());
                }
                return None;
            }
            Node::Filter(filter) => {
                if filter.kind == FilterKind::Gradient {
                    let w = crate::params::u32(&filter.params, "width", 1280).clamp(100, 2000);
                    let h = crate::params::u32(&filter.params, "height", 720).clamp(100, 2000);
                    return Some((w, h));
                }
                if let Some(up) = upstream(snarl, current) {
                    current = resolve_route(snarl, up);
                    continue;
                }
                if let Some(cpu) = filter.preview.cpu.as_ref() {
                    return Some(cpu.dimensions());
                }
                return None;
            }
            Node::RemoveBg(_) | Node::RemoveBgV2(_) | Node::FaceRestore(_) => {
                if let Some(up) = upstream(snarl, current) {
                    current = resolve_route(snarl, up);
                    continue;
                }
                if let Some(cpu) = n.cpu() {
                    return Some(cpu.dimensions());
                }
                return None;
            }
            Node::Group(group) | Node::Blend(group) => {
                if group.canvas_size.x > 0.0 && group.canvas_size.y > 0.0 {
                    return Some((
                        group.canvas_size.x.round() as u32,
                        group.canvas_size.y.round() as u32,
                    ));
                }
                if let Some(up) = upstream(snarl, current) {
                    current = resolve_route(snarl, up);
                    continue;
                }
                return None;
            }
            Node::Texts(texts) => {
                if let Some(cpu) = texts.preview.cpu.as_ref() {
                    return Some(cpu.dimensions());
                }
                return None;
            }
            Node::Route => {
                if let Some(up) = upstream(snarl, current) {
                    current = resolve_route(snarl, up);
                    continue;
                }
                return None;
            }
            Node::Output(_) => return None,
        }
    }
    None
}

/// Source name used for output filenames. It follows Image pass-through chains
/// and prefers an asset's original name.
pub(crate) fn source_name(snarl: &Snarl<Node>, node: NodeId) -> String {
    let node = resolve_route(snarl, node);
    let mut current = node;
    let mut seen = std::collections::HashSet::new();
    while let Some(image) = snarl.get_node(current).and_then(|n| n.image_node()) {
        if !seen.insert(current) {
            return "image".to_owned();
        }
        if let Some(upstream) = upstream(snarl, current) {
            current = upstream;
        } else {
            if !image.name.is_empty() {
                return image.name.clone();
            }
            if !image.path.is_empty() {
                return std::path::Path::new(&image.path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "image".to_owned());
            }
            return "image".to_owned();
        }
    }
    "image".to_owned()
}
