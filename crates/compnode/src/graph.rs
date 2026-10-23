//! Operations on the Snarl graph: loading images, connecting wires,
//! invalidating stale outputs and refreshing Viewer layers.

use std::borrow::Cow;
use std::path::PathBuf;

use egui::TextureHandle;
use egui_snarl::{InPinId, NodeId, OutPinId, Snarl};

use crate::assets::AssetStore;
use crate::catalog;
use crate::memory::ImageMemory;
use crate::model::{Node, Preview, ViewerLayer};

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
pub(crate) const PREVIEW_MAX_EDGE: u32 = 200;

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

pub(crate) fn upstream(snarl: &Snarl<Node>, node: NodeId) -> Option<NodeId> {
    let pin = snarl.in_pin(InPinId { node, input: 0 });
    pin.remotes.first().map(|remote| remote.node)
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
        if let Some(path) = snarl[current].output_path() {
            return Some(path.to_path_buf());
        }
        if let Some(upstream) = upstream(snarl, current) {
            current = upstream;
        } else {
            return None;
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
        if let Some(tex) = snarl[current].texture() {
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
    if !catalog::spec(snarl[to.node].kind()).multi_input {
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
    let Some(image) = snarl[node].image_node() else {
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
            if let Some(image) = snarl[node].image_node_mut() {
                image.preview = Preview::with_image(texture, rgba);
            }
            // Adopt file-backed assets on first load so they become portable.
            if asset_id.is_none()
                && !path.is_empty()
                && let Ok(id) = assets.insert_path(&path)
                && let Some(image) = snarl[node].image_node_mut()
            {
                image.asset_id = Some(id);
            }
        }
        Some(Err(err)) => snarl[node].set_error(format!("Could not open image: {err}")),
        None => {}
    }
}

/// Image that a processing node should consume, or a message explaining why
/// there is none yet.
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
    while snarl[up].output_path().is_none() {
        if !seen.insert(up) {
            return Err("Image input cycle detected".to_owned());
        }
        if let Some(next) = upstream(snarl, up) {
            up = next;
        } else {
            break;
        }
    }

    let path = snarl[up]
        .output_path()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "Run the upstream node first".to_owned())?;

    ensure_image_loaded(snarl, up, ctx, assets);
    memory.touch(up);

    if let Some(image) = snarl[up].cpu() {
        return Ok(image.clone());
    }
    if let Some(error) = snarl[up].error() {
        return Err(error.to_owned());
    }
    if snarl[up].job().is_some() {
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
    snarl[node].invalidate_output();
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
            let can_propagate =
                snarl[down].invalidate_output() && catalog::spec(snarl[down].kind()).outputs > 0;
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
    let remotes = snarl.in_pin(InPinId { node, input: 0 }).remotes.clone();

    // Build the set of source node IDs that should have layers.
    let mut sources: Vec<NodeId> = remotes.iter().map(|remote| remote.node).collect();
    sources.sort_by_key(|source| source.0);
    sources.dedup();

    // Phase 1: Load any missing images (mutable borrow of snarl).
    for source in &sources {
        ensure_image_loaded(snarl, *source, ctx, assets);
    }

    // Phase 2: Read all texture data immutably (before borrowing viewer mutably).
    let mut layer_data: Vec<(NodeId, String, Option<PathBuf>, Option<TextureHandle>)> =
        Vec::with_capacity(sources.len());
    for source in &sources {
        let spec = catalog::spec(snarl[*source].kind());
        let source_name = spec.title.to_string();
        let source_path = snarl[*source].output_path().map(|p| p.to_path_buf());
        let existing_tex = effective_texture(snarl, *source);
        layer_data.push((*source, source_name, source_path, existing_tex));
    }

    // Phase 3: Get the viewer node mutably and update layers.
    let Some(viewer) = snarl[node].viewer_node_mut() else {
        return;
    };

    // If the set of sources hasn't changed, just update textures for existing layers.
    let existing_sources: Vec<NodeId> = viewer.layers.iter().map(|l| l.source).collect();
    if sources.len() == existing_sources.len()
        && sources.iter().all(|s| existing_sources.contains(s))
    {
        for (source, _, _, new_tex) in &layer_data {
            if let Some(layer) = viewer.layers.iter_mut().find(|l| l.source == *source) {
                if layer.texture != *new_tex {
                    layer.texture = new_tex.clone();
                }
            }
        }
        return;
    }

    // Build new layers list, reusing existing layer data where possible.
    let mut new_layers = Vec::with_capacity(sources.len());
    for (source, source_name, source_path, _) in &layer_data {
        let source = *source;
        // Reuse existing layer data if we already had this source.
        let mut layer = viewer
            .layers
            .iter()
            .find(|l| l.source == source)
            .cloned()
            .unwrap_or_else(|| ViewerLayer {
                source,
                texture: None,
                source_name: source_name.clone(),
                source_path: source_path.clone(),
                center: egui::Vec2::ZERO,
                size: egui::Vec2::ZERO,
                angle: 0.0,
            });

        // Update texture if changed (texture was pre-fetched in phase 2).
        let new_tex = layer_data
            .iter()
            .find(|(s, _, _, _)| *s == source)
            .and_then(|(_, _, _, tex)| tex.clone());
        if layer.texture != new_tex {
            layer.texture = new_tex;
        }

        new_layers.push(layer);
    }

    viewer.layers = new_layers;
}

/// Dimensions of the image a node effectively outputs.
pub(crate) fn effective_dimensions(
    snarl: &Snarl<Node>,
    node: NodeId,
    assets: &AssetStore,
) -> Option<(u32, u32)> {
    let mut current = node;
    let mut seen = std::collections::HashSet::new();
    while let Some(image) = snarl[current].image_node() {
        if !seen.insert(current) {
            return None;
        }
        if let Some(upstream) = upstream(snarl, current) {
            current = upstream;
        } else if let Some(cpu) = image.preview.cpu.as_ref() {
            return Some(cpu.dimensions());
        } else if let Some(asset_id) = image.asset_id.as_deref() {
            return assets.dimensions(asset_id);
        } else if !image.path.is_empty() {
            return image::image_dimensions(&image.path).ok();
        } else {
            return None;
        }
    }
    if let Some(cpu) = snarl[current].cpu() {
        return Some(cpu.dimensions());
    }
    if let Some(path) = snarl[current].reload_path() {
        return image::image_dimensions(path).ok();
    }
    None
}

/// Source name used for output filenames. It follows Image pass-through chains
/// and prefers an asset's original name.
pub(crate) fn source_name(snarl: &Snarl<Node>, node: NodeId) -> String {
    let mut current = node;
    let mut seen = std::collections::HashSet::new();
    while let Some(image) = snarl[current].image_node() {
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
