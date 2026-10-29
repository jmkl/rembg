//! Session JSON format and autosave paths.
//!
//! Session files are self-contained JSON. Processed images produced by
//! Remove Background and Face Restore are referenced by their absolute path,
//! not copied into a sidecar assets folder.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use egui_snarl::{InPinId, NodeId, OutPinId, Snarl};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::assets::AssetStore;
use crate::cache::{OutputCache, load_cached};
use crate::catalog;
use crate::graph::{connect_input, ensure_output_node, rgba_to_texture};
use crate::model::{
    FaceRestoreNode, FilterKind, GroupNode, ImageNode, Node, NodeTrait, Preview, Process,
    TextsNode, ViewerNode,
};

pub(crate) const SESSION_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, PartialEq)]
pub(crate) struct SessionFile {
    pub version: u32,
    pub nodes: Vec<SessionNode>,
    pub links: Vec<SessionLink>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<SessionAsset>,
}

#[derive(Serialize, Deserialize, PartialEq)]
pub(crate) struct SessionNode {
    pub id: usize,
    pub pos: [f32; 2],
    pub collapsed: bool,
    pub data: SessionNodeData,
}

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SessionNodeData {
    Image {
        #[serde(default)]
        asset_id: Option<String>,
        #[serde(default)]
        name: String,
        #[serde(default)]
        path: String,
    },
    RemoveBg {
        /// Absolute path to the processed result.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Legacy pre-path sessions: key into the shared result cache.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_key: Option<String>,
    },
    FaceRestore {
        strength: f32,
        only_center: bool,
        /// Absolute path to the processed result.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Legacy pre-path sessions: key into the shared result cache.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_key: Option<String>,
    },
    /// Remove background v2 (RMBG-1.4 logic)
    RemoveBgV2 {
        /// Absolute path to the processed result.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Legacy pre-path sessions: key into the shared result cache.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_key: Option<String>,
    },
    Viewer {
        /// Layer source node ids, bottom-to-top.
        #[serde(default)]
        layers: Vec<usize>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        transforms: Vec<SessionLayerTransform>,
    },
    Filter {
        filter_kind: String,
        #[serde(default)]
        params: Value,
        #[serde(default)]
        layers: Vec<usize>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        transforms: Vec<SessionLayerTransform>,
    },
}

/// Saved per-layer transform (canvas position, size and rotation), matched to
/// layers by source node id. Optional so older session files still parse.
#[derive(Serialize, Deserialize, PartialEq, Clone, Copy)]
pub(crate) struct SessionLayerTransform {
    pub source: usize,
    pub center: [f32; 2],
    pub size: [f32; 2],
    pub angle: f32,
}

#[derive(Serialize, Deserialize, PartialEq)]
pub(crate) struct SessionLink {
    pub from: usize,
    pub to: usize,
    #[serde(default)]
    pub from_output: usize,
    #[serde(default)]
    pub to_input: usize,
}

#[derive(Serialize, Deserialize, PartialEq)]
pub(crate) struct SessionAsset {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub data: String,
}

/// Old sessions kept processed images beside the JSON in a `*.assets` folder.
fn legacy_assets_dir(session: &Path) -> PathBuf {
    session.with_extension("assets")
}

/// Pick a free session filename, appending ` (2)`, ` (3)`, ... before the
/// extension when the requested name already exists.
pub(crate) fn unique_session_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }

    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, extension) = if let Some(stem) = name.strip_suffix(".rembg.json") {
        (stem.to_owned(), ".rembg.json".to_owned())
    } else {
        (
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path.extension()
                .map(|ext| format!(".{}", ext.to_string_lossy()))
                .unwrap_or_default(),
        )
    };
    let parent = path.parent().unwrap_or_else(|| Path::new(""));

    for number in 2u32.. {
        let candidate = parent.join(format!("{stem} ({number}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("ran out of session filename numbers")
}

/// Recovery snapshot written while you work. It lives outside the manual
/// session file so autosaving never overwrites a document the user explicitly
/// saved.
#[allow(dead_code)]
pub(crate) fn default_autosave_path() -> PathBuf {
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        PathBuf::from(base)
            .join("rembg")
            .join("studio-autosave.rembg.json")
    } else {
        std::env::temp_dir().join("compnode-autosave.rembg.json")
    }
}

pub(crate) fn snapshot(snarl: &Snarl<Node>, assets: &AssetStore) -> SessionFile {
    let mut used_assets = HashSet::new();
    let mut nodes = Vec::new();
    for (id, node) in snarl.node_ids() {
        let Some(info) = snarl.get_node_info(id) else {
            continue;
        };
        let node_definition: &dyn NodeTrait = node;
        let data = match node {
            Node::Image(image) => {
                if let Some(asset_id) = &image.asset_id {
                    used_assets.insert(asset_id.clone());
                }
                SessionNodeData::Image {
                    asset_id: image.asset_id.clone(),
                    name: image.name.clone(),
                    path: image.path.clone(),
                }
            }
            Node::RemoveBg(process) => SessionNodeData::RemoveBg {
                path: process.output_path.as_deref().map(path_string),
                cache_key: None,
            },
            Node::RemoveBgV2(process) => SessionNodeData::RemoveBgV2 {
                path: process.output_path.as_deref().map(path_string),
                cache_key: None,
            },
            Node::FaceRestore(data) => SessionNodeData::FaceRestore {
                strength: data.strength,
                only_center: data.only_center,
                path: data.process.output_path.as_deref().map(path_string),
                cache_key: None,
            },
            Node::Output(viewer) => SessionNodeData::Viewer {
                layers: viewer.layers.iter().map(|layer| layer.source.0).collect(),
                transforms: layer_transforms(&viewer.layers),
            },
            Node::Route => SessionNodeData::Filter {
                filter_kind: "route".to_owned(),
                params: node_definition
                    .serialize_parameters()
                    .unwrap_or_else(|| Value::Object(Default::default())),
                layers: Vec::new(),
                transforms: Vec::new(),
            },
            Node::Group(group) => SessionNodeData::Filter {
                filter_kind: "mix".to_owned(),
                params: node_definition
                    .serialize_parameters()
                    .unwrap_or_else(|| Value::Object(Default::default())),
                layers: group.layers.iter().map(|l| l.source.0).collect(),
                transforms: layer_transforms(&group.layers),
            },
            Node::Blend(blend) => SessionNodeData::Filter {
                filter_kind: "blend".to_owned(),
                params: node_definition
                    .serialize_parameters()
                    .unwrap_or_else(|| Value::Object(Default::default())),
                layers: blend.layers.iter().map(|l| l.source.0).collect(),
                transforms: layer_transforms(&blend.layers),
            },
            Node::Texts(_) => SessionNodeData::Filter {
                filter_kind: "texts".to_owned(),
                params: node_definition
                    .serialize_parameters()
                    .unwrap_or_else(|| Value::Object(Default::default())),
                layers: Vec::new(),
                transforms: Vec::new(),
            },
            Node::Filter(filter) => SessionNodeData::Filter {
                filter_kind: filter.kind.id().to_owned(),
                params: node_definition
                    .serialize_parameters()
                    .unwrap_or_else(|| Value::Object(Default::default())),
                layers: Vec::new(),
                transforms: Vec::new(),
            },
        };
        nodes.push(SessionNode {
            id: id.0,
            pos: [info.pos.x, info.pos.y],
            collapsed: !info.open,
            data,
        });
    }

    let mut links = Vec::new();
    for (id, node) in snarl.node_ids() {
        for output in 0..catalog::spec(node.kind()).outputs {
            for remote in snarl.out_pin(OutPinId { node: id, output }).remotes {
                links.push(SessionLink {
                    from: id.0,
                    to: remote.node.0,
                    from_output: output,
                    to_input: remote.input,
                });
            }
        }
    }

    let mut used_assets: Vec<String> = used_assets.into_iter().collect();
    used_assets.sort();
    let assets = used_assets
        .iter()
        .filter_map(|id| {
            assets.get(id).map(|asset| SessionAsset {
                id: id.clone(),
                name: asset.name.clone(),
                width: asset.width,
                height: asset.height,
                path: asset.path.as_deref().map(path_string),
                data: base64::engine::general_purpose::STANDARD.encode(&*asset.encoded),
            })
        })
        .collect();

    SessionFile {
        version: SESSION_VERSION,
        nodes,
        links,
        assets,
    }
}

/// Write a graph snapshot as a single JSON file. Processed results stay at
/// their existing absolute paths and are not copied beside the session.
pub(crate) fn write(path: &Path, session: &SessionFile) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }

    let json = serde_json::to_vec_pretty(session).map_err(|e| format!("serialize session: {e}"))?;
    fs::write(path, json).map_err(|e| format!("write {}: {e}", path.display()))
}

pub(crate) fn load(
    path: &Path,
    cache: &OutputCache,
    ctx: &egui::Context,
    assets: &mut AssetStore,
) -> Result<Snarl<Node>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let file: SessionFile =
        serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
    if file.version != SESSION_VERSION {
        return Err(format!(
            "unsupported session version {} (expected {SESSION_VERSION})",
            file.version
        ));
    }

    let legacy_assets = legacy_assets_dir(path);
    let mut next = Snarl::new();
    let mut id_map = HashMap::with_capacity(file.nodes.len());
    let mut pending_viewers: Vec<(NodeId, Vec<usize>, Vec<SessionLayerTransform>)> = Vec::new();

    for stored_asset in file.assets {
        if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&stored_asset.data) {
            assets.insert_session(
                stored_asset.id,
                stored_asset.name,
                stored_asset.width,
                stored_asset.height,
                decoded,
                stored_asset.path.map(PathBuf::from),
            );
        }
    }

    for stored in file.nodes {
        let mut pending_layers: Option<(Vec<usize>, Vec<SessionLayerTransform>)> = None;
        let node = match stored.data {
            SessionNodeData::Image {
                asset_id,
                name,
                path,
            } => {
                let asset_id = asset_id.or_else(|| {
                    if path.is_empty() {
                        None
                    } else {
                        assets.insert_path(&path).ok()
                    }
                });
                let name = if name.is_empty() {
                    asset_id
                        .as_deref()
                        .and_then(|id| assets.name(id))
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            std::path::Path::new(&path)
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        })
                } else {
                    name
                };
                Node::Image(ImageNode {
                    asset_id,
                    name,
                    path,
                    preview: Preview::default(),
                    preview_enabled: true,
                })
            }
            SessionNodeData::RemoveBg { path, cache_key } => {
                Node::RemoveBg(restore_process(cache, &legacy_assets, path, cache_key, ctx))
            }
            SessionNodeData::RemoveBgV2 { path, cache_key } => {
                Node::RemoveBgV2(restore_process(cache, &legacy_assets, path, cache_key, ctx))
            }
            SessionNodeData::FaceRestore {
                strength,
                only_center,
                path,
                cache_key,
            } => Node::FaceRestore(FaceRestoreNode {
                strength,
                only_center,
                process: restore_process(cache, &legacy_assets, path, cache_key, ctx),
            }),
            SessionNodeData::Viewer { layers, transforms } => {
                pending_layers = Some((layers, transforms));
                Node::Output(ViewerNode::default())
            }
            SessionNodeData::Filter {
                filter_kind,
                params,
                layers,
                transforms,
            } => match filter_kind.as_str() {
                "route" => Node::Route,
                "mix" | "group" => {
                    pending_layers = Some((layers, transforms));
                    let mut node = Node::Group(GroupNode::new(false));
                    let node_definition: &mut dyn NodeTrait = &mut node;
                    node_definition.restore_parameters(&params);
                    node
                }
                "blend" => {
                    pending_layers = Some((layers, transforms));
                    let mut node = Node::Blend(GroupNode::new(true));
                    let node_definition: &mut dyn NodeTrait = &mut node;
                    node_definition.restore_parameters(&params);
                    node
                }
                "texts" | "text" => {
                    let mut node = Node::Texts(TextsNode::new());
                    let node_definition: &mut dyn NodeTrait = &mut node;
                    node_definition.restore_parameters(&params);
                    node
                }
                other => {
                    let Some(filter_kind) = FilterKind::from_id(other) else {
                        continue;
                    };
                    let mut node = Node::new_filter(filter_kind);
                    let node_definition: &mut dyn NodeTrait = &mut node;
                    node_definition.restore_parameters(&params);
                    node
                }
            },
        };

        let pos = egui::pos2(stored.pos[0], stored.pos[1]);
        let id = if stored.collapsed {
            next.insert_node_collapsed(pos, node)
        } else {
            next.insert_node(pos, node)
        };
        id_map.insert(stored.id, id);
        if let Some((layers, transforms)) = pending_layers {
            pending_viewers.push((id, layers, transforms));
        }
    }

    // Restore Viewer/Group/Blend layer order and transforms after every node
    // has its new id. `refresh_viewer` reuses these layers when the restored
    // wires match, keeping the saved transforms.
    for (owner, old_layers, transforms) in pending_viewers {
        let layers = old_layers
            .into_iter()
            .filter_map(|old| id_map.get(&old).copied())
            .map(|source| {
                let saved = transforms
                    .iter()
                    .find(|t| id_map.get(&t.source).copied() == Some(source));
                crate::model::ViewerLayer {
                    source,
                    kind: next[source].kind(),
                    texture: None,
                    source_name: String::new(),
                    source_path: None,
                    center: saved
                        .map(|t| egui::vec2(t.center[0], t.center[1]))
                        .unwrap_or(egui::Vec2::ZERO),
                    size: saved
                        .map(|t| egui::vec2(t.size[0], t.size[1]))
                        .unwrap_or(egui::Vec2::ZERO),
                    angle: saved.map(|t| t.angle).unwrap_or(0.0),
                }
            })
            .collect();
        if let Some(owner_layers) = next[owner].layers_mut() {
            *owner_layers = layers;
        }
    }

    for link in file.links {
        let (Some(&from), Some(&to)) = (id_map.get(&link.from), id_map.get(&link.to)) else {
            continue;
        };
        let _ = connect_input(
            &mut next,
            OutPinId {
                node: from,
                output: link.from_output,
            },
            InPinId {
                node: to,
                input: link.to_input,
            },
        );
    }

    ensure_output_node(&mut next);
    Ok(next)
}

fn restore_process(
    cache: &OutputCache,
    legacy_assets: &Path,
    path: Option<String>,
    legacy_key: Option<String>,
    ctx: &egui::Context,
) -> Process {
    if let Some(path) = path {
        let path = absolute(path);
        return match image::open(&path) {
            Ok(image) => process_with_image(path, image.to_rgba8(), ctx),
            Err(err) => {
                let mut process =
                    missing_process(format!("Result missing at {}: {err}", path.display()));
                process.output_path = Some(path);
                process
            }
        };
    }

    let Some(key) = legacy_key else {
        return Process::default();
    };
    match load_cached(cache, Some(legacy_assets), &key) {
        Some((path, image)) => process_with_image(absolute(path), image, ctx),
        None => missing_process("Cached result missing — press Run to regenerate".to_owned()),
    }
}

fn process_with_image(path: PathBuf, image: image::RgbaImage, ctx: &egui::Context) -> Process {
    Process {
        preview: Preview::with_image(rgba_to_texture(ctx, &image), image),
        job: None,
        output_path: Some(path),
        last_timing: None,
        preview_enabled: false,
    }
}

fn missing_process(error: String) -> Process {
    Process {
        preview: Preview {
            error: Some(error),
            ..Default::default()
        },
        job: None,
        output_path: None,
        last_timing: None,
        preview_enabled: false,
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn absolute(path: impl Into<PathBuf>) -> PathBuf {
    let path = path.into();
    std::path::absolute(&path).unwrap_or(path)
}

/// Serialize layer transforms for the session file.
fn layer_transforms(layers: &[crate::model::ViewerLayer]) -> Vec<SessionLayerTransform> {
    layers
        .iter()
        .map(|layer| SessionLayerTransform {
            source: layer.source.0,
            center: [layer.center.x, layer.center.y],
            size: [layer.size.x, layer.size.y],
            angle: layer.angle,
        })
        .collect()
}
