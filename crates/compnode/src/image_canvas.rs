//! `ImageCanvas` – a self-contained egui widget that mirrors a compnode viewer's layers.
//!
//! Gizmos on the selected layer: drag body = move, corners = uniform scale,
//! edges = stretch, round handle = rotate (Shift = 15° snap).
//!
//! Multi-select: Shift/Ctrl-click toggles a layer (canvas or layer list),
//! drag on empty canvas = marquee. With 2+ layers selected a group gizmo
//! (move / uniform corner scale / rotate about the group center) is shown.
//! Hold Alt while dragging to disable snapping.

use crate::gpaste_app::GPasTeApp;
use crate::layer_cache::LayerCache;
use crate::model::{FilterKind, Kind};
use crate::style::controls::Button;
use crate::style::{font, icons, metrics, pal};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Margin, Mesh, Pos2, Rect, Sense,
    Shape, Stroke, StrokeKind, TextureHandle, TextureId, Vec2, epaint::Vertex, pos2, vec2,
};
use egui_snarl::NodeId;
use image::{DynamicImage, Rgba, RgbaImage};
use std::collections::{HashMap, HashSet};
use std::f32::consts::FRAC_PI_2;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const HANDLE_R: f32 = 8.0;
const ROT_DIST: f32 = 32.0;
const MIN_SIZE: f32 = 8.0;
const SIDEBAR_PADDING: f32 = 8.0;
const SIDEBAR_GAP: f32 = 8.0;
const ROW_H: f32 = 30.0;
#[allow(dead_code)]
const INPUT_RADIUS: u8 = 8;
#[allow(dead_code)]
const CANVAS_RADIUS: u8 = 12;
const LAYER_RADIUS: u8 = 8;
const DRAG_HANDLE_DOT: f32 = 4.0;
const DRAG_HANDLE_GAP: f32 = 3.0;
const DRAG_HANDLE_SIZE: f32 = 24.0;
const SNAP_PX: f32 = 6.0;
/// Alpha above which a pixel counts as visible ink for the Texts content
/// rect. Soft shadows fade below this, so the frame hugs what the eye sees.
const CONTENT_ALPHA: u8 = 8;

const SCALE_HANDLES: [(i8, i8); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
];

/// Corner handles of the group gizmo (no edge handles: stretching a group
/// would shear rotated members).
const GROUP_CORNERS: [(i8, i8); 4] = [(-1, -1), (1, -1), (1, 1), (-1, 1)];

#[derive(Clone, Copy)]
enum Guide {
    V(f32), // vertical line at canvas x
    H(f32), // horizontal line at canvas y
}

fn rot(v: Vec2, a: f32) -> Vec2 {
    let (s, c) = a.sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

#[derive(Clone, Copy)]
struct Snap {
    center: Vec2,
    size: Vec2,
    angle: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Handle {
    Scale(i8, i8),
    Rotate,
}

#[derive(Clone, Copy)]
enum Drag {
    None,
    Move {
        grab: Vec2,
    },
    Rotate,
    Scale {
        sx: i8,
        sy: i8,
        start: Snap,
    },
    /// `grab` = pointer position at press; `min`/`max` = group bbox at press.
    GroupMove {
        grab: Vec2,
        min: Vec2,
        max: Vec2,
    },
    GroupRotate {
        pivot: Vec2,
        a0: f32,
    },
    GroupScale {
        sx: i8,
        sy: i8,
        min: Vec2,
        max: Vec2,
    },
    Marquee {
        start: Vec2,
    },
}

pub struct Item {
    /// Source node id (the node whose output this layer shows).
    id: NodeId,
    /// Kind of the source node; drives kind-specific layer handling.
    kind: Kind,
    name: String,
    /// Full texture from the layer cache, or the node's low-res preview while
    /// the file is still being decoded.
    tex: Option<TextureHandle>,
    /// True once `tex` comes from the layer cache (not the placeholder).
    tex_is_full: bool,
    /// Whether the layer is visible in the canvas.
    visible: bool,
    /// Center in canvas coordinates
    center: Vec2,
    /// Size in canvas units (before rotation)
    size: Vec2,
    /// Radians, clockwise on screen
    angle: f32,
    /// Output filepath of the source node; key into the layer cache.
    source_path: Option<PathBuf>,
    /// Size is still the provisional default and must be fitted once the
    /// real image dimensions are known.
    fit_pending: bool,
    /// Pixel dimensions of the image currently backing `tex`. Used to refit
    /// the layer only when the real resolution changes (e.g. preview ->
    /// full-res), without resetting user transforms on same-size re-renders.
    tex_dims: (u32, u32),
    /// Transform written by the last automatic fit. A layer still sitting at
    /// exactly this transform has never been positioned/scaled by the user, so
    /// a texture resolution change may refit it. Any user edit (move, scale,
    /// rotate, inspector) makes the layer differ, and then it is never refit
    /// again: re-rendering an upstream node (preview <-> full-res) must not
    /// throw the canvas layout away.
    auto_fit: Option<Snap>,
    /// Normalized `[u0, v0, u1, v1]` of the non-transparent part of `tex`.
    ///
    /// Texts is rasterized on a full canvas-sized bitmap, with the ink (plus
    /// outlines/shadow/extrusion) occupying only a small region. `size` stays
    /// canvas-sized so the Viewer and Group export keep working, but the
    /// canvas gizmo, hit-testing and mesh UVs use this tighter rect.
    content_uv: Option<[f32; 4]>,
}

impl Item {
    fn snap(&self) -> Snap {
        Snap {
            center: self.center,
            size: self.size,
            angle: self.angle,
        }
    }

    fn is_text(&self) -> bool {
        self.kind == Kind::Filter(FilterKind::Texts)
    }

    /// True when the layer still has the transform produced by the last
    /// automatic fit (i.e. the user never moved/scaled/rotated it).
    fn is_pristine(&self) -> bool {
        self.auto_fit.is_some_and(|fit| {
            (self.center - fit.center).length() <= 0.01
                && (self.size - fit.size).length() <= 0.01
                && (self.angle - fit.angle).abs() <= 1e-4
        })
    }

    /// Visible content rect in normalized texture coordinates relative to
    /// the texture center: min/max within `[-0.5, 0.5]`.
    fn norm_bounds(&self) -> (Vec2, Vec2) {
        match self.content_uv {
            Some([u0, v0, u1, v1]) => (vec2(u0 - 0.5, v0 - 0.5), vec2(u1 - 0.5, v1 - 0.5)),
            None => (vec2(-0.5, -0.5), vec2(0.5, 0.5)),
        }
    }

    /// Visible content bounds in local (unrotated) coordinates, relative to
    /// `center`. Without a content rect this is the full texture.
    fn local_bounds(&self) -> (Vec2, Vec2) {
        let (n0, n1) = self.norm_bounds();
        (n0 * self.size, n1 * self.size)
    }

    /// Size of the visible content in canvas units.
    fn content_size(&self) -> Vec2 {
        let (min, max) = self.local_bounds();
        max - min
    }

    fn to_local(&self, p: Vec2) -> Vec2 {
        rot(p - self.center, -self.angle)
    }

    fn contains(&self, p: Vec2) -> bool {
        let l = self.to_local(p);
        let (min, max) = self.local_bounds();
        l.x >= min.x && l.x <= max.x && l.y >= min.y && l.y <= max.y
    }

    fn corners(&self) -> [Vec2; 4] {
        let (min, max) = self.local_bounds();
        [
            vec2(min.x, min.y),
            vec2(max.x, min.y),
            vec2(max.x, max.y),
            vec2(min.x, max.y),
        ]
        .map(|l| self.center + rot(l, self.angle))
    }

    fn handle_pos(&self, h: Handle, rot_dist: f32) -> Vec2 {
        let (min, max) = self.local_bounds();
        let (cx, cy) = ((min.x + max.x) * 0.5, (min.y + max.y) * 0.5);
        let local = match h {
            Handle::Scale(sx, sy) => vec2(
                if sx < 0 {
                    min.x
                } else if sx > 0 {
                    max.x
                } else {
                    cx
                },
                if sy < 0 {
                    min.y
                } else if sy > 0 {
                    max.y
                } else {
                    cy
                },
            ),
            Handle::Rotate => vec2(cx, min.y - rot_dist),
        };
        self.center + rot(local, self.angle)
    }

    fn hit_handle(&self, p: Vec2, tol: f32, rot_dist: f32) -> Option<Handle> {
        let scale = || {
            SCALE_HANDLES
                .iter()
                .map(|&(x, y)| Handle::Scale(x, y))
                .find(|&h| (self.handle_pos(h, rot_dist) - p).length() <= tol)
        };
        // Texts can be resized, but not individually rotated: rotation is
        // about the texture center, which makes the baked ink orbit.
        if self.is_text() {
            return scale();
        }
        std::iter::once(Handle::Rotate)
            .chain(SCALE_HANDLES.iter().map(|&(x, y)| Handle::Scale(x, y)))
            .find(|&h| (self.handle_pos(h, rot_dist) - p).length() <= tol)
    }

    /// Axis-aligned bounding box of the (possibly rotated) layer.
    fn aabb(&self) -> (Vec2, Vec2) {
        let c = self.corners();
        let (mut min, mut max) = (c[0], c[0]);
        for p in &c[1..] {
            min = vec2(min.x.min(p.x), min.y.min(p.y));
            max = vec2(max.x.max(p.x), max.y.max(p.y));
        }
        (min, max)
    }
}

fn cursor_for(h: Handle) -> CursorIcon {
    match h {
        Handle::Rotate => CursorIcon::Grab,
        Handle::Scale(x, y) => match (x, y) {
            (-1, -1) | (1, 1) => CursorIcon::ResizeNwSe,
            (1, -1) | (-1, 1) => CursorIcon::ResizeNeSw,
            (0, _) => CursorIcon::ResizeVertical,
            _ => CursorIcon::ResizeHorizontal,
        },
    }
}

/// Scale `dims` (pixels) down to fit within 80% of the canvas, never up.
fn fit_inside(dims: (u32, u32), canvas: Vec2) -> Vec2 {
    let (w, h) = (dims.0.max(1) as f32, dims.1.max(1) as f32);
    let k = (canvas.x * 0.8 / w).min(canvas.y * 0.8 / h).min(1.0);
    vec2(w * k, h * k)
}

/// Normalized `[u0, v0, u1, v1]` of the pixels whose alpha exceeds
/// `threshold`. `None` for a fully transparent image.
///
/// Scans row slices instead of `enumerate_pixels` so empty rows cost almost
/// nothing, which matters for the canvas-sized Texts bitmap.
fn alpha_content_uv(img: &RgbaImage, threshold: u8) -> Option<[f32; 4]> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let raw = img.as_raw();
    let stride = w as usize * 4;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (w, h, 0u32, 0u32);
    let mut found = false;
    for y in 0..h as usize {
        let row = &raw[y * stride..(y + 1) * stride];
        let Some(first) = row.chunks_exact(4).position(|p| p[3] > threshold) else {
            continue;
        };
        let last = row
            .chunks_exact(4)
            .rposition(|p| p[3] > threshold)
            .unwrap_or(first);
        found = true;
        min_x = min_x.min(first as u32);
        max_x = max_x.max(last as u32);
        min_y = min_y.min(y as u32);
        max_y = y as u32;
    }
    found.then(|| {
        [
            min_x as f32 / w as f32,
            min_y as f32 / h as f32,
            (max_x + 1) as f32 / w as f32,
            (max_y + 1) as f32 / h as f32,
        ]
    })
}

fn group_handle_pos(bb: (Vec2, Vec2), h: Handle, rot_dist: f32) -> Vec2 {
    let (min, max) = bb;
    match h {
        Handle::Scale(sx, sy) => vec2(
            if sx < 0 { min.x } else { max.x },
            if sy < 0 { min.y } else { max.y },
        ),
        Handle::Rotate => vec2((min.x + max.x) * 0.5, min.y - rot_dist),
    }
}

fn group_hit(bb: (Vec2, Vec2), p: Vec2, tol: f32, rot_dist: f32) -> Option<Handle> {
    std::iter::once(Handle::Rotate)
        .chain(GROUP_CORNERS.iter().map(|&(x, y)| Handle::Scale(x, y)))
        .find(|&h| (group_handle_pos(bb, h, rot_dist) - p).length() <= tol)
}

/// Read-only view of a layer (for exporting / inspecting from your app).
pub struct LayerInfo<'a> {
    pub id: NodeId,
    pub name: &'a str,
    pub texture: Option<&'a TextureHandle>,
    /// Center in canvas coordinates.
    pub center: Vec2,
    /// Size in canvas units, before rotation.
    pub size: Vec2,
    /// Radians, clockwise.
    pub angle: f32,
}

#[allow(dead_code)]
struct Prompt {}

pub struct ImageCanvas {
    canvas_size: Vec2,
    background: Color32,

    /// Stored back → front (last = top-most). The layer list shows it reversed.
    items: Vec<Item>,
    /// Decoded images keyed by filepath; survives disconnect/reconnect.
    cache: LayerCache,
    /// Last transform of every path that left the canvas, restored when the
    /// same file comes back.
    remembered: HashMap<PathBuf, Snap>,
    /// Primary selection.
    selected: Option<NodeId>,
    /// Additional selected layers (multi-select). `selected` is the primary.
    extra: HashSet<NodeId>,
    /// Set when the user explicitly cleared the selection (right-click in the
    /// panel, or a click on empty canvas). Keeps the idle auto-select from
    /// re-selecting a layer and showing the gizmos again. Cleared as soon as a
    /// layer row or a canvas image is selected.
    selection_suppressed: bool,
    drag: Drag,
    /// Transforms of the group members when the current group drag started.
    group_start: Vec<(NodeId, Snap)>,
    /// Live marquee rectangle in canvas coordinates.
    marquee: Option<(Vec2, Vec2)>,
    list_drag: Option<NodeId>,
    /// Last time the user interacted with the canvas (drag/scale/rotate/reorder).
    last_activity: Instant,
    layer_count: usize,
    /// Signature of the layer set at the last sync (see `begin_sync`).
    last_sig: Option<u64>,
    /// Set when async work (a finished decode) needs another sync.
    dirty: bool,
    last_sync: Instant,
    /// Paths shown at the last sync; protects them from cache eviction.
    in_use: HashSet<PathBuf>,
    /// `last_activity` value at the last transform write-back.
    committed_activity: Option<Instant>,
    gpasteapp: GPasTeApp,
    canvas_win_w: f32,
    guides: Vec<Guide>,
    keep_aspect_ratio: bool,
}

/// How often a sync is forced even if nothing changed, so files modified on
/// disk are noticed.
const FORCE_SYNC_EVERY: Duration = Duration::from_millis(750);

impl ImageCanvas {
    pub fn new(gpaste_app: GPasTeApp) -> Self {
        let canvas_size = egui::vec2(1280.0, 720.0);
        Self {
            canvas_size,
            background: pal::CANVAS_BG,
            items: vec![],
            cache: LayerCache::default(),
            remembered: HashMap::new(),
            selected: None,
            extra: HashSet::new(),
            selection_suppressed: false,
            drag: Drag::None,
            group_start: Vec::new(),
            marquee: None,
            list_drag: None,
            last_activity: Instant::now(),
            layer_count: 0,
            last_sig: None,
            dirty: false,
            last_sync: Instant::now(),
            in_use: HashSet::new(),
            committed_activity: None,
            gpasteapp: gpaste_app,
            canvas_win_w: 1200.0,
            guides: Vec::new(),
            keep_aspect_ratio: true,
        }
    }

    pub(crate) fn gpaste_app(&mut self) -> &mut GPasTeApp {
        &mut self.gpasteapp
    }

    /// Logical canvas size (default 1280×720).
    pub fn with_canvas_size(mut self, w: f32, h: f32) -> Self {
        self.canvas_size = vec2(w, h);
        self
    }

    pub fn with_background(mut self, c: Color32) -> Self {
        self.background = c;
        self
    }

    // ---------------- public API ----------------

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn selected(&self) -> Option<NodeId> {
        self.selected
    }

    /// Clear the selection and hide the gizmos. The idle auto-select will not
    /// bring them back; selecting a layer row or clicking an image in the
    /// canvas does.
    pub fn clear_selection(&mut self) {
        self.selected = None;
        self.extra.clear();
        self.selection_suppressed = true;
        self.drag = Drag::None;
        self.last_activity = Instant::now();
    }

    /// Forget every item, selection and remembered transform because the graph
    /// was replaced wholesale (new document or opened session).
    ///
    /// Without this, items from the previous graph could survive when their
    /// source id and file path match, and `sync_layers` deliberately leaves the
    /// transform of existing items alone: a later save would then write those
    /// stale transforms over the ones restored from the session.
    pub fn reset_graph(&mut self) {
        self.items.clear();
        self.remembered.clear();
        self.selected = None;
        self.extra.clear();
        self.selection_suppressed = false;
        self.drag = Drag::None;
        self.group_start.clear();
        self.marquee = None;
        self.list_drag = None;
        self.layer_count = 0;
        // Force the next `begin_sync` to rebuild the items even when the layer
        // signature happens to match the previous document.
        self.last_sig = None;
        self.dirty = true;
        self.in_use.clear();
        self.committed_activity = None;
        self.guides.clear();
    }

    fn select(&mut self, id: NodeId) {
        self.selected = Some(id);
        self.extra.clear();
        self.selection_suppressed = false;
    }

    /// Add/remove a layer from the selection (Shift/Ctrl-click).
    fn toggle_select(&mut self, id: NodeId) {
        self.selection_suppressed = false;
        if self.selected == Some(id) {
            // promote another layer to primary
            self.selected = self.extra.iter().next().copied();
            if let Some(n) = self.selected {
                self.extra.remove(&n);
            }
        } else if !self.extra.remove(&id) {
            if self.selected.is_none() {
                self.selected = Some(id);
            } else {
                self.extra.insert(id);
            }
        }
    }

    /// Current logical canvas size.
    pub fn canvas_size(&self) -> Vec2 {
        self.canvas_size
    }

    /// Set the logical canvas size from the widget (or externally).
    pub fn set_canvas_size(&mut self, w: f32, h: f32) {
        self.canvas_size = vec2(w, h);
    }

    /// Whether a drag (move/scale/rotate/list-reorder) is currently in progress.
    pub fn is_dragging(&self) -> bool {
        !matches!(self.drag, Drag::None) || self.list_drag.is_some()
    }

    /// Whether the canvas has been idle (no drag/scale/rotate/reorder) for at
    /// least `secs` seconds.
    pub fn is_stable(&self, secs: f32) -> bool {
        !self.is_dragging() && self.last_activity.elapsed().as_secs_f32() >= secs
    }

    /// Layers from back to front.
    pub fn layers(&self) -> impl Iterator<Item = LayerInfo<'_>> {
        self.items.iter().map(|i| LayerInfo {
            id: i.id,
            name: &i.name,
            texture: i.tex.as_ref(),
            center: i.center,
            size: i.size,
            angle: i.angle,
        })
    }

    /// Cheap signature of a layer set: source ids, file paths and preview
    /// texture ids. No allocation; call it every frame instead of building the
    /// full layer list.
    pub fn layers_signature<'a>(
        layers: impl Iterator<Item = (NodeId, Option<&'a Path>, Option<TextureId>)>,
    ) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (id, path, tex) in layers {
            id.0.hash(&mut h);
            path.hash(&mut h);
            match tex {
                Some(TextureId::Managed(n)) => (1u8, n).hash(&mut h),
                Some(TextureId::User(n)) => (2u8, n).hash(&mut h),
                None => 0u8.hash(&mut h),
            }
        }
        h.finish()
    }

    /// Call once per frame with [`ImageCanvas::layers_signature`]. Returns
    /// `true` only when [`ImageCanvas::sync_layers`] actually needs to run.
    pub fn begin_sync(&mut self, ctx: &egui::Context, sig: u64) -> bool {
        if self.cache.poll(ctx, &self.in_use) {
            self.dirty = true;
        }
        let due = self.last_sig != Some(sig)
            || self.dirty
            || self.last_sync.elapsed() >= FORCE_SYNC_EVERY;
        if due {
            self.last_sig = Some(sig);
            self.dirty = false;
            self.last_sync = Instant::now();
        }
        due
    }

    /// True while a transform change has not been written back to the graph
    /// yet. Used to schedule the frames the idle commit needs.
    pub fn has_uncommitted_transform(&self) -> bool {
        self.committed_activity != Some(self.last_activity)
    }

    /// True exactly once after the user stops interacting for `secs`
    /// seconds; the caller then writes transforms back to the viewer.
    pub fn take_stable_commit(&mut self, secs: f32) -> bool {
        if !self.is_stable(secs) || self.committed_activity == Some(self.last_activity) {
            return false;
        }
        self.committed_activity = Some(self.last_activity);
        true
    }

    /// Sync canvas items with the layers an Output node currently exposes.
    ///
    /// Called every frame, so it must be cheap when nothing changed:
    ///
    /// * Items are matched by source node id; transforms of existing items are
    ///   never touched.
    /// * The image for a `filepath` is looked up in the canvas' own
    ///   [`LayerCache`]. On a miss it is decoded on a background thread.
    ///   While it loads, `preview_texture` is shown.
    /// * When a layer disappears its transform is remembered by filepath.
    pub(crate) fn sync_layers(
        &mut self,
        ctx: &egui::Context,
        layers: &[(
            NodeId,
            Kind,
            String,
            Option<egui::TextureHandle>,
            Option<PathBuf>,
            (egui::Vec2, egui::Vec2, f32),
            bool,
        )],
    ) {
        let in_use: HashSet<PathBuf> = layers
            .iter()
            .filter_map(|(_, _, _, _, p, _, _)| p.clone())
            .collect();

        self.cache.forget_unused_failures(&in_use);
        self.in_use = in_use;

        if layers.len() != self.layer_count {
            self.layer_count = layers.len();
        }

        // Drop items whose source is gone, remembering where they were.
        let present: HashSet<NodeId> = layers.iter().map(|(id, ..)| *id).collect();
        let mut removed = Vec::new();
        self.items.retain(|it| {
            let keep = present.contains(&it.id);
            if !keep {
                removed.push((it.source_path.clone(), it.snap()));
            }
            keep
        });
        for (path, snap) in removed {
            if let Some(path) = path {
                self.remembered.insert(path, snap);
            }
        }
        if let Some(sel) = self.selected {
            if !self.items.iter().any(|i| i.id == sel) {
                self.selected = None;
            }
        }
        let alive: HashSet<NodeId> = self.items.iter().map(|i| i.id).collect();
        self.extra.retain(|id| alive.contains(id));
        if self.selected.is_none() {
            // promote a remaining extra layer to primary
            self.selected = self.extra.iter().next().copied();
            if let Some(n) = self.selected {
                self.extra.remove(&n);
            }
        }

        let canvas_size = self.canvas_size;
        for (source_id, source_kind, source_name, preview, path, saved, use_preview) in layers {
            let use_preview = *use_preview;
            let is_text = *source_kind == Kind::Filter(FilterKind::Texts);
            // Ask the cache first: hit = already a CPU image + texture.
            let cached = path
                .as_deref()
                .and_then(|p| self.cache.get_or_load(ctx, p))
                .map(|c| (c.texture.clone(), c.dims, c.cpu.clone()));

            // Texture selection: the on-disk file is the highest-quality
            // correct content *when it is the node's own output*. For a
            // plain filter without its own output the preview texture is the
            // only correct content, so it wins even though it is low-res.
            // The CPU image is only available when the cached file is used.
            let (display_tex, display_dims, cached_cpu) = if use_preview {
                if let Some(p) = preview.clone() {
                    (p.clone(), (p.size()[0] as u32, p.size()[1] as u32), None)
                } else if let Some((tex, dims, cpu)) = cached.clone() {
                    (tex, dims, Some(cpu))
                } else {
                    continue;
                }
            } else if let Some((tex, dims, cpu)) = cached.clone() {
                (tex, dims, Some(cpu))
            } else if let Some(p) = preview.clone() {
                (p.clone(), (p.size()[0] as u32, p.size()[1] as u32), None)
            } else {
                continue;
            };

            let idx = match self.items.iter().position(|i| i.id == *source_id) {
                Some(i) => i,
                None => {
                    let remembered_snap = path.as_ref().and_then(|p| self.remembered.remove(p));
                    let saved_snap =
                        if remembered_snap.is_none() && saved.1.x > 0.0 && saved.1.y > 0.0 {
                            Some(Snap {
                                center: saved.0,
                                size: saved.1,
                                angle: saved.2,
                            })
                        } else {
                            None
                        };
                    let snap = remembered_snap.or(saved_snap);
                    self.items.push(Item {
                        id: *source_id,
                        kind: *source_kind,
                        name: format!("{} #{}", source_name, source_id.0),
                        tex: None,
                        tex_is_full: false,
                        visible: true,
                        center: snap.map_or(canvas_size / 2.0, |s| s.center),
                        // A new Texts layer maps its full canvas-sized
                        // raster; a restored one keeps the user's scale.
                        size: if is_text {
                            snap.map_or(canvas_size, |s| s.size)
                        } else {
                            snap.map_or(vec2(200.0, 200.0), |s| s.size)
                        },
                        angle: snap.map_or(0.0, |s| s.angle),
                        source_path: path.clone(),
                        fit_pending: snap.is_none(),
                        // A restored transform is authoritative: don't refit it
                        // just because the texture dims arrive now.
                        tex_dims: if snap.is_some() { display_dims } else { (0, 0) },
                        // A restored transform is user-owned, not an auto fit.
                        auto_fit: None,
                        content_uv: None,
                    });
                    self.items.len() - 1
                }
            };
            let it = &mut self.items[idx];

            // The node now produces a different file (e.g. a processing node
            // finished). Keep the transform, swap the image.
            if it.source_path != *path {
                it.source_path = path.clone();
                it.tex_is_full = false;
            }

            let tex_changed = it.tex.as_ref().map(|t| t.id()) != Some(display_tex.id());
            if tex_changed {
                it.tex = Some(display_tex.clone());
            }

            // Refit only a layer the user never touched. The texture can change
            // resolution when a filter re-renders (preview <-> full-res); that
            // must not reset the canvas scale/position. A pristine layer still
            // follows the new resolution, so the initial fit stays correct.
            let dims_changed = it.tex_dims != display_dims;
            if it.fit_pending || (dims_changed && it.is_pristine()) {
                // Texts is rasterized at the canvas resolution; anything else
                // is fitted into the canvas at its natural aspect.
                it.size = if is_text {
                    canvas_size
                } else {
                    fit_inside(display_dims, canvas_size)
                };
                it.fit_pending = false;
                it.auto_fit = Some(it.snap());
            }
            it.tex_dims = display_dims;

            // Tight content rect for canvas-sized layers that are mostly
            // transparent (Texts ink + outline/shadow/3D). Recomputed from the
            // full-res CPU pixels only when a new texture arrives; forced syncs
            // run every 750 ms and must not rescan the bitmap.
            if is_text {
                if tex_changed {
                    it.content_uv = cached_cpu
                        .as_deref()
                        .and_then(|img| alpha_content_uv(img, CONTENT_ALPHA));
                }
            } else {
                it.content_uv = None;
            }
            it.tex_is_full = true;
        }

        if self.selected.is_none()
            && !self.selection_suppressed
            && self.last_activity.elapsed().as_secs_f32() >= 2.0
        {
            self.selected = self.items.iter().rev().find(|i| i.visible).map(|i| i.id);
        }
    }

    /// Cached CPU image for a layer path, if it has been converted already.
    pub fn cpu_image(&self, path: &Path) -> Option<std::sync::Arc<image::RgbaImage>> {
        self.cache.get(path).map(|c| c.cpu.clone())
    }

    /// Apply transform changes from `LayerInfo` back to matching items by id.
    pub fn apply_transforms(&mut self, updates: &[(NodeId, egui::Vec2, egui::Vec2, f32)]) {
        for (id, center, size, angle) in updates {
            if let Some(it) = self.items.iter_mut().find(|i| i.id == *id) {
                it.center = *center;
                it.size = *size;
                it.angle = *angle;
            }
        }
    }

    // ---------------- the widget ----------------

    /// Draw the six-dot drag handle (2 columns × 3 rows)
    fn draw_drag_handle(&self, ui: &mut egui::Ui, rect: Rect) {
        let painter = ui.painter();
        let dot_r = DRAG_HANDLE_DOT * 0.5;
        let start_x =
            rect.left() + (rect.width() - (DRAG_HANDLE_DOT * 2.0 + DRAG_HANDLE_GAP)) * 0.5;
        let start_y =
            rect.top() + (rect.height() - (DRAG_HANDLE_DOT * 3.0 + DRAG_HANDLE_GAP * 2.0)) * 0.5;

        for row in 0..3 {
            for col in 0..2 {
                let cx = start_x + col as f32 * (DRAG_HANDLE_DOT + DRAG_HANDLE_GAP) + dot_r;
                let cy = start_y + row as f32 * (DRAG_HANDLE_DOT + DRAG_HANDLE_GAP) + dot_r;
                painter.circle_filled(egui::pos2(cx, cy), dot_r, pal::FAINT);
            }
        }
    }

    /// X and Y snap lines: canvas edges/center plus edges/center of other
    /// visible layers (those in `skip` are ignored).
    fn snap_targets(&self, skip: &[NodeId]) -> (Vec<f32>, Vec<f32>) {
        let mut xs = vec![0.0, self.canvas_size.x / 2.0, self.canvas_size.x];
        let mut ys = vec![0.0, self.canvas_size.y / 2.0, self.canvas_size.y];
        for it in self
            .items
            .iter()
            .filter(|i| !skip.contains(&i.id) && i.visible)
        {
            let (min, max) = it.aabb();
            xs.extend([min.x, (min.x + max.x) * 0.5, max.x]);
            ys.extend([min.y, (min.y + max.y) * 0.5, max.y]);
        }
        (xs, ys)
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui, settings: &crate::settings::Settings) {
        self.canvas_size = egui::vec2(settings.canvas_width, settings.canvas_height);

        let panel_rect = ui.max_rect(); // the sidebar
        if ui.input(|i| {
            i.pointer.secondary_clicked()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| panel_rect.contains(p))
        }) {
            self.clear_selection();
        }

        // Everything left of the sidebar is where the canvas may float.
        let content = ui.ctx().content_rect();
        let area = Rect::from_min_max(content.min, pos2(panel_rect.left(), content.max.y));
        self.floating_canvas(&ui.ctx().clone(), area);

        egui::Frame::new()
            .inner_margin(Margin::same(SIDEBAR_PADDING as i8))
            .show(ui, |ui| {
                let inner_width = ui.available_width();
                self.top_section_ui(ui, inner_width);
            });
    }
    fn top_section_ui(&mut self, ui: &mut egui::Ui, inner_width: f32) {
        ui.spacing_mut().item_spacing.y = SIDEBAR_GAP;

        ui.horizontal(|ui| {
            if ui
                .add(Button::new("Attach Image").trans().small())
                .clicked()
            {
                self.attach_canvas(ui.ctx());
            }
        });

        self.layer_list_ui(ui, inner_width);

        let mut transform_changed = false;
        let selected = self.selected;
        let keep_aspect_ratio = self.keep_aspect_ratio;

        if let Some(sel) = selected {
            let selected_indices = self.selection_indices();

            if selected_indices.len() >= 2 {
                ui.separator();

                let (min, max) = self.group_bbox(&selected_indices);

                // X/Y are the group's top-left bounding-box position.
                let mut x = min.x;
                let mut y = min.y;
                let mut width = max.x - min.x;
                let mut height = max.y - min.y;

                ui.horizontal(|ui| {
                    ui.label("X");
                    let x_changed = ui.add(egui::DragValue::new(&mut x).speed(1.0)).changed();

                    ui.label("Y");
                    let y_changed = ui.add(egui::DragValue::new(&mut y).speed(1.0)).changed();

                    ui.label("W");
                    let width_changed = ui
                        .add(
                            egui::DragValue::new(&mut width)
                                .range(MIN_SIZE..=f32::MAX)
                                .speed(1.0),
                        )
                        .changed();

                    ui.label("H");
                    let height_changed = ui
                        .add(
                            egui::DragValue::new(&mut height)
                                .range(MIN_SIZE..=f32::MAX)
                                .speed(1.0),
                        )
                        .changed();

                    ui.checkbox(&mut self.keep_aspect_ratio, "aspect ratio");

                    if x_changed || y_changed || width_changed || height_changed {
                        let old_width = (max.x - min.x).max(1.0);
                        let old_height = (max.y - min.y).max(1.0);

                        // First translate every selected layer by the same delta.
                        let delta = vec2(x - min.x, y - min.y);

                        if x_changed || y_changed {
                            for &i in &selected_indices {
                                self.items[i].center += delta;
                            }
                        }

                        // Then scale around the group's new center, if requested.
                        if width_changed || height_changed {
                            let sx = width.max(MIN_SIZE) / old_width;
                            let sy = height.max(MIN_SIZE) / old_height;

                            let (sx, sy) = if keep_aspect_ratio {
                                let s = if width_changed { sx } else { sy };
                                (s, s)
                            } else {
                                (sx, sy)
                            };

                            let pivot = vec2(x + (max.x - min.x) * 0.5, y + (max.y - min.y) * 0.5);

                            for &i in &selected_indices {
                                let item = &mut self.items[i];

                                item.center = pivot
                                    + vec2(
                                        (item.center.x - pivot.x) * sx,
                                        (item.center.y - pivot.y) * sy,
                                    );

                                item.size = vec2(
                                    (item.size.x * sx).max(MIN_SIZE),
                                    (item.size.y * sy).max(MIN_SIZE),
                                );
                            }
                        }

                        transform_changed = true;
                    }
                });
            } else if let Some(node) = self
                .items
                .iter_mut()
                .filter(|item| item.visible)
                .find(|item| item.id == sel)
            {
                ui.separator();

                // Single-layer X/Y are the unrotated content's top-left. W/H
                // are the visible content size; the texture is scaled to match
                // (identical to the old behaviour when the content fills it).
                let (n0, n1) = node.norm_bounds();
                let (wn, hn) = ((n1.x - n0.x).max(0.001), (n1.y - n0.y).max(0.001));
                let content = node.content_size();
                let orig_x = node.center.x + n0.x * node.size.x;
                let orig_y = node.center.y + n0.y * node.size.y;
                let mut x = orig_x;
                let mut y = orig_y;
                let aspect_ratio = content.x / content.y;
                let mut width = content.x;
                let mut height = content.y;

                ui.horizontal(|ui| {
                    ui.label("X");
                    let x_changed = ui.add(egui::DragValue::new(&mut x).speed(1.0)).changed();

                    ui.label("Y");
                    let y_changed = ui.add(egui::DragValue::new(&mut y).speed(1.0)).changed();

                    ui.label("W");
                    let width_changed = ui
                        .add(
                            egui::DragValue::new(&mut width)
                                .range(MIN_SIZE..=f32::MAX)
                                .speed(1.0),
                        )
                        .changed();

                    ui.label("H");
                    let height_changed = ui
                        .add(
                            egui::DragValue::new(&mut height)
                                .range(MIN_SIZE..=f32::MAX)
                                .speed(1.0),
                        )
                        .changed();

                    ui.checkbox(&mut self.keep_aspect_ratio, "aspect ratio");

                    if x_changed || y_changed || width_changed || height_changed {
                        if keep_aspect_ratio && aspect_ratio.is_finite() && aspect_ratio > 0.0 {
                            if width_changed && !height_changed {
                                height = width / aspect_ratio;
                            } else if height_changed && !width_changed {
                                width = height * aspect_ratio;
                            }
                        }

                        let content = vec2(width.max(MIN_SIZE), height.max(MIN_SIZE));
                        // Convert the content size back to texture size, then
                        // keep the content's top-left pinned at (x, y).
                        let size = vec2(content.x / wn, content.y / hn);
                        node.center = vec2(x, y) - vec2(n0.x * size.x, n0.y * size.y);
                        node.size = size;

                        transform_changed = true;
                    }
                });
            }
        }

        if transform_changed {
            self.last_activity = Instant::now();
        }

        ui.separator();
        self.gpasteapp.render_ui(ui);
    }

    fn old_top_section_ui(&mut self, ui: &mut egui::Ui, inner_width: f32) {
        ui.spacing_mut().item_spacing.y = SIDEBAR_GAP;
        ui.horizontal(|ui| {
            if ui
                .add(Button::new("Attach Image").trans().small())
                .clicked()
            {
                self.attach_canvas(ui.ctx());
            }
        });

        self.layer_list_ui(ui, inner_width);

        let mut size_changed = false;
        let selected = self.selected;
        let keep_aspect_ratio = self.keep_aspect_ratio;
        if let Some(sel) = selected {
            let selected_indices = self.selection_indices();

            if selected_indices.len() >= 2 {
                ui.separator();
                // Multi-selection: show the combined bounding-box dimensions.
                let (min, max) = self.group_bbox(&selected_indices);
                let mut width = max.x - min.x;
                let mut height = max.y - min.y;
                ui.horizontal(|ui| {
                    ui.horizontal(|ui| {
                        ui.label("W");
                        let width_changed = ui
                            .add(
                                egui::DragValue::new(&mut width)
                                    .range(MIN_SIZE..=f32::MAX)
                                    .speed(1.0),
                            )
                            .changed();

                        ui.label("H");
                        let height_changed = ui
                            .add(
                                egui::DragValue::new(&mut height)
                                    .range(MIN_SIZE..=f32::MAX)
                                    .speed(1.0),
                            )
                            .changed();
                        ui.checkbox(&mut self.keep_aspect_ratio, "aspect ratio");
                        if width_changed || height_changed {
                            let sx = width / (max.x - min.x).max(1.0);
                            let sy = height / (max.y - min.y).max(1.0);

                            // Uniform scaling when aspect ratio is enabled.
                            let (sx, sy) = if keep_aspect_ratio {
                                let s = if width_changed { sx } else { sy };
                                (s, s)
                            } else {
                                (sx, sy)
                            };

                            // Anchor at the center of the group for inspector resizing.
                            let pivot = (min + max) * 0.5;
                            let selected_ids: HashSet<NodeId> =
                                selected_indices.iter().map(|&i| self.items[i].id).collect();
                            for item in &mut self.items {
                                if selected_ids.contains(&item.id) {
                                    item.center = pivot
                                        + vec2(
                                            (item.center.x - pivot.x) * sx,
                                            (item.center.y - pivot.y) * sy,
                                        );
                                    item.size = vec2(
                                        (item.size.x * sx).max(MIN_SIZE),
                                        (item.size.y * sy).max(MIN_SIZE),
                                    );
                                }
                            }

                            size_changed = true;
                        }
                    });
                });
            } else if let Some(node) = self
                .items
                .iter_mut()
                .filter(|f| f.visible)
                .find(|item| item.id == sel)
            {
                ui.separator();

                // Single selection: edit the layer dimensions directly.
                let aspect_ratio = node.size.x / node.size.y;
                let mut width = node.size.x;
                let mut height = node.size.y;

                ui.horizontal(|ui| {
                    ui.label("W");
                    let width_changed = ui
                        .add(
                            egui::DragValue::new(&mut width)
                                .range(MIN_SIZE..=f32::MAX)
                                .speed(1.0),
                        )
                        .changed();

                    ui.label("H");
                    let height_changed = ui
                        .add(
                            egui::DragValue::new(&mut height)
                                .range(MIN_SIZE..=f32::MAX)
                                .speed(1.0),
                        )
                        .changed();
                    ui.checkbox(&mut self.keep_aspect_ratio, "aspect ratio");

                    if width_changed || height_changed {
                        if keep_aspect_ratio && aspect_ratio.is_finite() && aspect_ratio > 0.0 {
                            if width_changed && !height_changed {
                                height = width / aspect_ratio;
                            } else if height_changed && !width_changed {
                                width = height * aspect_ratio;
                            }
                        }

                        node.size = vec2(width.max(MIN_SIZE), height.max(MIN_SIZE));

                        size_changed = true;
                    }
                });
            }
        }
        if size_changed {
            self.last_activity = Instant::now();
        }

        ui.separator();
        self.gpasteapp.render_ui(ui);
    }

    // ---------------- selection helpers ----------------

    /// Indices (back → front) of all selected, visible layers.
    fn selection_indices(&self) -> Vec<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.visible && (self.selected == Some(it.id) || self.extra.contains(&it.id))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Union of the members' axis-aligned bounding boxes.
    fn group_bbox(&self, idxs: &[usize]) -> (Vec2, Vec2) {
        let mut min = vec2(f32::MAX, f32::MAX);
        let mut max = vec2(f32::MIN, f32::MIN);
        for &i in idxs {
            let (a, b) = self.items[i].aabb();
            min = vec2(min.x.min(a.x), min.y.min(a.y));
            max = vec2(max.x.max(b.x), max.y.max(b.y));
        }
        (min, max)
    }

    fn group_snaps(&self, idxs: &[usize]) -> Vec<(NodeId, Snap)> {
        idxs.iter()
            .map(|&i| (self.items[i].id, self.items[i].snap()))
            .collect()
    }

    /// Handle under `p` for either the single-layer or the group gizmo.
    fn gizmo_hit(&self, p: Vec2, tol: f32, rot_dist: f32) -> Option<Handle> {
        let sel = self.selection_indices();
        if sel.len() >= 2 {
            group_hit(self.group_bbox(&sel), p, tol, rot_dist)
        } else {
            sel.first()
                .and_then(|&i| self.items[i].hit_handle(p, tol, rot_dist))
        }
    }

    // ---------------- canvas ----------------

    fn on_press(&mut self, p: Vec2, tol: f32, rot_dist: f32, additive: bool) {
        self.last_activity = Instant::now();
        let sel = self.selection_indices();

        // 1) gizmo handles
        if sel.len() >= 2 {
            let bb = self.group_bbox(&sel);
            if let Some(h) = group_hit(bb, p, tol, rot_dist) {
                self.group_start = self.group_snaps(&sel);
                self.drag = match h {
                    Handle::Rotate => {
                        let c = (bb.0 + bb.1) * 0.5;
                        let v = p - c;
                        Drag::GroupRotate {
                            pivot: c,
                            a0: v.y.atan2(v.x),
                        }
                    }
                    Handle::Scale(sx, sy) => Drag::GroupScale {
                        sx,
                        sy,
                        min: bb.0,
                        max: bb.1,
                    },
                };
                return;
            }
        } else if let Some(&i) = sel.first() {
            let it = &self.items[i];
            if let Some(h) = it.hit_handle(p, tol, rot_dist) {
                self.drag = match h {
                    Handle::Rotate => Drag::Rotate,
                    Handle::Scale(sx, sy) => Drag::Scale {
                        sx,
                        sy,
                        start: it.snap(),
                    },
                };
                return;
            }
        }

        // 2) top-most visible layer under the cursor
        let hit = (0..self.items.len())
            .rev()
            .find(|&i| self.items[i].visible && self.items[i].contains(p));

        match hit {
            Some(i) => {
                let id = self.items[i].id;
                if additive {
                    self.toggle_select(id);
                    return;
                }
                let sel_now = self.selection_indices();
                if sel_now.len() >= 2 && sel_now.contains(&i) {
                    // drag the whole group without collapsing the selection
                    let bb = self.group_bbox(&sel_now);
                    self.group_start = self.group_snaps(&sel_now);
                    self.drag = Drag::GroupMove {
                        grab: p,
                        min: bb.0,
                        max: bb.1,
                    };
                } else {
                    self.select(id);
                    self.drag = Drag::Move {
                        grab: p - self.items[i].center,
                    };
                }
            }
            None => {
                if !additive {
                    self.clear_selection();
                    self.drag = Drag::Marquee { start: p };
                }
            }
        }
    }

    fn on_drag(&mut self, p: Vec2, shift: bool, alt: bool, tol: f32) {
        self.last_activity = Instant::now();

        match self.drag {
            Drag::GroupMove { .. } | Drag::GroupRotate { .. } | Drag::GroupScale { .. } => {
                self.on_group_drag(p, shift, alt, tol);
                return;
            }
            Drag::Marquee { start } => {
                self.on_marquee(start, p);
                return;
            }
            _ => {}
        }

        let idxs = self.selection_indices();
        let Some(&i) = idxs.first() else { return };

        // Alt = no snapping (empty targets never match).
        let (xs, ys) = if alt {
            (Vec::new(), Vec::new())
        } else {
            self.snap_targets(&[self.items[i].id])
        };
        let mut guides: Vec<Guide> = Vec::new();

        let drag = self.drag;
        let it = &mut self.items[i];
        match drag {
            Drag::Move { grab } => {
                it.center = p - grab;
                let (min, max) = it.aabb();
                let cx = (min.x + max.x) * 0.5;
                let cy = (min.y + max.y) * 0.5;

                // left / center / right edges → best match
                let sx = [min.x, cx, max.x]
                    .iter()
                    .filter_map(|&v| snap1(v, &xs, tol))
                    .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));
                let sy = [min.y, cy, max.y]
                    .iter()
                    .filter_map(|&v| snap1(v, &ys, tol))
                    .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));

                if let Some((d, t)) = sx {
                    it.center.x += d;
                    guides.push(Guide::V(t));
                }
                if let Some((d, t)) = sy {
                    it.center.y += d;
                    guides.push(Guide::H(t));
                }
            }
            Drag::Rotate => {
                let d = p - it.center;
                let mut a = d.y.atan2(d.x) + FRAC_PI_2;
                if shift {
                    let step = 15f32.to_radians();
                    a = (a / step).round() * step;
                }
                it.angle = a;
            }
            Drag::Scale { sx, sy, start } => {
                let (fx, fy) = (sx as f32, sy as f32);
                let (w0, h0) = (start.size.x.max(1.0), start.size.y.max(1.0));

                // Scale relative to the visible content, not the full texture.
                // Texts ink sits off-center in a canvas-sized bitmap, so
                // anchoring on the bitmap's opposite corner would drag the
                // text sideways while resizing. `an` is the normalized content
                // anchor (the opposite edge under the fixed handle).
                let (n0, n1) = it.norm_bounds();
                let (wn, hn) = ((n1.x - n0.x).max(0.001), (n1.y - n0.y).max(0.001));
                let min_w = MIN_SIZE / wn;
                let min_h = MIN_SIZE / hn;
                let an = vec2(
                    if fx > 0.0 {
                        n0.x
                    } else if fx < 0.0 {
                        n1.x
                    } else {
                        0.0
                    },
                    if fy > 0.0 {
                        n0.y
                    } else if fy < 0.0 {
                        n1.y
                    } else {
                        0.0
                    },
                );
                let anchor_world = start.center + rot(an * start.size, start.angle);
                let lp = rot(p - anchor_world, -start.angle);

                // Texture size implied by putting the moving content edge
                // under the pointer.
                let mut w = w0;
                let mut h = h0;
                if sx != 0 && sy != 0 {
                    let kx = (lp.x * fx / wn).max(min_w) / w0;
                    let ky = (lp.y * fy / hn).max(min_h) / h0;
                    let k = kx.max(ky);
                    w = w0 * k;
                    h = h0 * k;
                } else if sx != 0 {
                    w = (lp.x * fx / wn).max(min_w);
                } else {
                    h = (lp.y * fy / hn).max(min_h);
                }

                // Snap the moving edge(s). Only for unrotated layers, where
                // the edges are axis-aligned in canvas space.
                if start.angle.abs() < 1e-3 {
                    let ex = anchor_world.x + fx * w * wn;
                    let ey = anchor_world.y + fy * h * hn;
                    let snap_x = if sx != 0 { snap1(ex, &xs, tol) } else { None };
                    let snap_y = if sy != 0 { snap1(ey, &ys, tol) } else { None };
                    // Convert a target edge position back to a texture width.
                    let width_to = |edge: f32| ((edge - anchor_world.x) * fx / wn).max(min_w);
                    let height_to = |edge: f32| ((edge - anchor_world.y) * fy / hn).max(min_h);

                    if sx != 0 && sy != 0 {
                        // corner = uniform scale: follow whichever axis is closer
                        let use_x = match (snap_x, snap_y) {
                            (Some(a), Some(b)) => Some(a.0.abs() <= b.0.abs()),
                            (Some(_), None) => Some(true),
                            (None, Some(_)) => Some(false),
                            _ => None,
                        };
                        match use_x {
                            Some(true) => {
                                let (d, t) = snap_x.unwrap();
                                let k = width_to(ex + d) / w0;
                                w = w0 * k;
                                h = h0 * k;
                                guides.push(Guide::V(t));
                            }
                            Some(false) => {
                                let (d, t) = snap_y.unwrap();
                                let k = height_to(ey + d) / h0;
                                w = w0 * k;
                                h = h0 * k;
                                guides.push(Guide::H(t));
                            }
                            None => {}
                        }
                    } else if let Some((d, t)) = snap_x {
                        w = width_to(ex + d);
                        guides.push(Guide::V(t));
                    } else if let Some((d, t)) = snap_y {
                        h = height_to(ey + d);
                        guides.push(Guide::H(t));
                    }
                }

                it.size = vec2(w, h);
                // Move the texture center so the opposite content edge stays
                // exactly where it was when the drag started.
                it.center = anchor_world - rot(an * it.size, start.angle);
            }
            _ => {}
        }
        self.guides = guides;
    }

    fn on_marquee(&mut self, start: Vec2, p: Vec2) {
        let lo = vec2(start.x.min(p.x), start.y.min(p.y));
        let hi = vec2(start.x.max(p.x), start.y.max(p.y));
        self.marquee = Some((lo, hi));

        let hit: Vec<NodeId> = self
            .items
            .iter()
            .filter(|it| it.visible)
            .filter(|it| {
                let (mn, mx) = it.aabb();
                mn.x <= hi.x && mx.x >= lo.x && mn.y <= hi.y && mx.y >= lo.y
            })
            .map(|it| it.id)
            .collect();
        self.selected = hit.first().copied();
        self.extra = hit.into_iter().skip(1).collect();
        self.selection_suppressed = self.selected.is_none();
    }

    fn on_group_drag(&mut self, p: Vec2, shift: bool, alt: bool, tol: f32) {
        let start = std::mem::take(&mut self.group_start);
        let mut guides: Vec<Guide> = Vec::new();
        let drag = self.drag;

        match drag {
            Drag::GroupMove { grab, min, max } => {
                let mut d = p - grab;
                if !alt {
                    let ids: Vec<NodeId> = start.iter().map(|(id, _)| *id).collect();
                    let (xs, ys) = self.snap_targets(&ids);
                    let (mn, mx) = (min + d, max + d);
                    let best = |vals: [f32; 3], t: &[f32]| {
                        vals.iter()
                            .filter_map(|&v| snap1(v, t, tol))
                            .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()))
                    };
                    if let Some((dx, t)) = best([mn.x, (mn.x + mx.x) * 0.5, mx.x], &xs[..]) {
                        d.x += dx;
                        guides.push(Guide::V(t));
                    }
                    if let Some((dy, t)) = best([mn.y, (mn.y + mx.y) * 0.5, mx.y], &ys[..]) {
                        d.y += dy;
                        guides.push(Guide::H(t));
                    }
                }
                for (id, s) in &start {
                    if let Some(it) = self.items.iter_mut().find(|i| i.id == *id) {
                        it.center = s.center + d;
                    }
                }
            }
            Drag::GroupRotate { pivot, a0 } => {
                let v = p - pivot;
                let mut delta = v.y.atan2(v.x) - a0;
                if shift {
                    let step = 15f32.to_radians();
                    delta = (delta / step).round() * step;
                }
                for (id, s) in &start {
                    if let Some(it) = self.items.iter_mut().find(|i| i.id == *id) {
                        it.center = pivot + rot(s.center - pivot, delta);
                        it.angle = s.angle + delta;
                    }
                }
            }

            Drag::GroupScale { sx, sy, min, max } => {
                let (fx, fy) = (sx as f32, sy as f32);

                // Opposite corner of the group bounding box stays fixed.
                let anchor = vec2(
                    if sx > 0 { min.x } else { max.x },
                    if sy > 0 { min.y } else { max.y },
                );

                let (w0, h0) = ((max.x - min.x).max(1.0), (max.y - min.y).max(1.0));

                // Calculate one uniform scale factor for the entire group.
                let kx = ((p.x - anchor.x) * fx / w0).max(0.02);
                let ky = ((p.y - anchor.y) * fy / h0).max(0.02);
                let k = if sx != 0 && sy != 0 {
                    kx.max(ky)
                } else if sx != 0 {
                    kx
                } else {
                    ky
                };

                for (id, start_snap) in &start {
                    if let Some(item) = self.items.iter_mut().find(|i| i.id == *id) {
                        // Scale every layer's position relative to the group anchor.
                        item.center = anchor + (start_snap.center - anchor) * k;

                        // Scale each layer proportionally.
                        item.size = start_snap.size * k;
                        item.angle = start_snap.angle;
                    }
                }
            }

            _ => {}
        }

        self.group_start = start;
        self.guides = guides;
    }

    fn floating_canvas(&mut self, ctx: &egui::Context, area: Rect) {
        let w = self.canvas_win_w.clamp(160.0, area.width().max(160.0));
        let scale = w / self.canvas_size.x;
        let h = self.canvas_size.y * scale;

        egui::Window::new("Canvas")
            .id(egui::Id::new("floating_canvas"))
            .title_bar(false)
            .frame(
                egui::Frame::new()
                    .fill(pal::BG)
                    .inner_margin(Margin::ZERO)
                    .shadow(egui::Shadow::NONE),
            )
            .pivot(Align2::RIGHT_TOP)
            .fixed_pos(pos2(area.right(), area.top() + metrics::TITLEBAR_H))
            .fixed_size(vec2(w, h)) // we control the size; egui's resize is off
            .show(ctx, |ui| {
                self.canvas_ui(ui, scale);

                // Resize grip on the left edge (drawn after the canvas so it wins hit-testing).
                let r = ui.min_rect();
                let grip = Rect::from_min_max(r.left_top(), pos2(r.left() + 6.0, r.bottom()));
                let resp = ui.interact(grip, egui::Id::new("canvas_grip"), Sense::drag());
                if resp.hovered() || resp.dragged() {
                    ui.set_cursor_icon(CursorIcon::ResizeHorizontal);
                }
                if resp.dragged() {
                    // Right edge is pinned, so dragging left (negative dx) widens the canvas.
                    self.canvas_win_w -= resp.drag_delta().x;
                }
            });
    }

    fn canvas_ui(&mut self, ui: &mut egui::Ui, scale: f32) {
        let (resp, painter) =
            ui.allocate_painter(self.canvas_size * scale, Sense::click_and_drag());
        let rect = resp.rect;
        let origin = rect.min.to_vec2();
        let to_screen = move |v: Vec2| (origin + v * scale).to_pos2();
        let to_canvas = move |p: Pos2| (p.to_vec2() - origin) / scale;

        // handle sizes are in screen pixels, so convert them to canvas units for hit-testing
        let tol = (HANDLE_R + 2.0) / scale;
        let rot_dist = ROT_DIST / scale;

        // --- input (raw pointer: a plain click selects, no drag threshold) ---
        let (pressed, down, pos, shift, alt, additive) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.interact_pos(),
                i.modifiers.shift,
                i.modifiers.alt,
                i.modifiers.shift || i.modifiers.command,
            )
        });

        // allow grabbing gizmo handles that stick out of the canvas (e.g. the rotate handle)
        let on_outside_handle = pos.is_some_and(|p| {
            rect.expand(HANDLE_R + ROT_DIST + 4.0).contains(p)
                && self.gizmo_hit(to_canvas(p), tol, rot_dist).is_some()
        });

        if pressed && (resp.hovered() || on_outside_handle) {
            if let Some(p) = pos {
                self.on_press(to_canvas(p), tol, rot_dist, additive);
            }
        } else if down {
            if let Some(p) = pos {
                self.on_drag(to_canvas(p), shift, alt, SNAP_PX / scale);
            }
        }
        if !down {
            if !matches!(self.drag, Drag::None) {
                self.last_activity = Instant::now();
            }
            self.drag = Drag::None;
            self.guides.clear(); // guides disappear on release
            self.marquee = None;
            self.group_start.clear();
        }

        // --- cursor feedback ---
        if matches!(self.drag, Drag::None) && (resp.hovered() || on_outside_handle) {
            if let Some(p) = pos.map(to_canvas) {
                let mut icon = None;
                if let Some(h) = self.gizmo_hit(p, tol, rot_dist) {
                    icon = Some(cursor_for(h));
                }
                if icon.is_none() && self.items.iter().any(|it| it.visible && it.contains(p)) {
                    icon = Some(CursorIcon::Move);
                }
                if let Some(c) = icon {
                    ui.set_cursor_icon(c);
                }
            }
        }

        // --- draw canvas + layers (back → front) ---
        // Checkerboard background matching the node preview widgets.
        painter.rect_filled(rect, CornerRadius::ZERO, pal::BG);

        let cell = 8.0;
        let mut y = rect.top();
        let mut row = 0;
        while y < rect.bottom() {
            let mut x = rect.left() + if row % 2 == 0 { 0.0 } else { cell };
            while x < rect.right() {
                let tile = Rect::from_min_max(
                    pos2(x, y),
                    pos2((x + cell).min(rect.right()), (y + cell).min(rect.bottom())),
                );
                painter.rect_filled(tile, CornerRadius::ZERO, pal::NODE);
                x += cell * 2.0;
            }
            y += cell;
            row += 1;
        }

        for it in &self.items {
            if !it.visible {
                continue;
            }
            let Some(tex) = &it.tex else { continue };
            let c = it.corners();
            let uvs = match it.content_uv {
                Some([u0, v0, u1, v1]) => [pos2(u0, v0), pos2(u1, v0), pos2(u1, v1), pos2(u0, v1)],
                None => [
                    pos2(0.0, 0.0),
                    pos2(1.0, 0.0),
                    pos2(1.0, 1.0),
                    pos2(0.0, 1.0),
                ],
            };
            let mut mesh = Mesh::with_texture(tex.id());
            for k in 0..4 {
                mesh.vertices.push(Vertex {
                    pos: to_screen(c[k]),
                    uv: uvs[k],
                    color: pal::WHITE,
                });
            }
            mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
            painter.add(Shape::mesh(mesh));
        }
        painter.rect_stroke(
            rect,
            CornerRadius::ZERO,
            Stroke::new(1.0, pal::MUTED),
            StrokeKind::Inside,
        );

        // --- snap guides ---
        let guide_stroke = Stroke::new(1.0, Color32::from_rgb(255, 64, 160));
        for g in &self.guides {
            match *g {
                Guide::V(x) => {
                    let sx = to_screen(vec2(x, 0.0)).x;
                    painter.vline(sx, rect.y_range(), guide_stroke);
                }
                Guide::H(y) => {
                    let sy = to_screen(vec2(0.0, y)).y;
                    painter.hline(rect.x_range(), sy, guide_stroke);
                }
            }
        }

        // --- marquee ---
        if let Some((lo, hi)) = self.marquee {
            let r = Rect::from_min_max(to_screen(lo), to_screen(hi));
            painter.rect_filled(r, CornerRadius::ZERO, pal::ACCENT.gamma_multiply(0.15));
            painter.rect_stroke(
                r,
                CornerRadius::ZERO,
                Stroke::new(1.0, pal::ACCENT),
                StrokeKind::Inside,
            );
        }

        // --- gizmo (painted with a larger clip so handles outside the canvas stay visible) ---
        let sel = self.selection_indices();
        if sel.len() >= 2 {
            let gp = ui.painter().with_clip_rect(
                rect.expand(HANDLE_R + ROT_DIST + 4.0)
                    .intersect(ui.clip_rect()),
            );
            // faint outline per member
            for &i in &sel {
                let pts: Vec<Pos2> = self.items[i]
                    .corners()
                    .iter()
                    .map(|&c| to_screen(c))
                    .collect();
                gp.add(Shape::closed_line(
                    pts,
                    Stroke::new(1.0, pal::ACCENT.gamma_multiply(0.5)),
                ));
            }
            let bb = self.group_bbox(&sel);
            let r = Rect::from_min_max(to_screen(bb.0), to_screen(bb.1));
            gp.rect_stroke(
                r,
                CornerRadius::ZERO,
                Stroke::new(1.5, pal::ACCENT),
                StrokeKind::Middle,
            );

            let top_mid = pos2(r.center().x, r.top());
            let rot_pos = to_screen(group_handle_pos(bb, Handle::Rotate, rot_dist));
            gp.line_segment([top_mid, rot_pos], Stroke::new(1.5, pal::ACCENT));
            gp.circle(rot_pos, HANDLE_R, pal::WHITE, Stroke::new(2.0, pal::ACCENT));

            for &(sx, sy) in &GROUP_CORNERS {
                let c = to_screen(group_handle_pos(bb, Handle::Scale(sx, sy), rot_dist));
                let hr = Rect::from_center_size(c, Vec2::splat(HANDLE_R * 1.6));
                gp.rect_filled(hr, CornerRadius::same(1), pal::WHITE);
                gp.rect_stroke(
                    hr,
                    CornerRadius::same(1),
                    Stroke::new(2.0, pal::ACCENT),
                    StrokeKind::Inside,
                );
            }
        } else if let Some(&i) = sel.first() {
            let gp = ui.painter().with_clip_rect(
                rect.expand(HANDLE_R + ROT_DIST + 4.0)
                    .intersect(ui.clip_rect()),
            );
            let it = &self.items[i];
            let pts: Vec<Pos2> = it.corners().iter().map(|&c| to_screen(c)).collect();
            gp.add(Shape::closed_line(pts, Stroke::new(1.5, pal::ACCENT)));

            // Scale handles for every layer. Texts has no rotate handle:
            // rotating the full-canvas raster makes the ink orbit its center.
            if !it.is_text() {
                let top_mid = to_screen(it.handle_pos(Handle::Scale(0, -1), rot_dist));
                let rot_pos = to_screen(it.handle_pos(Handle::Rotate, rot_dist));
                gp.line_segment([top_mid, rot_pos], Stroke::new(1.5, pal::ACCENT));
                gp.circle(rot_pos, HANDLE_R, pal::WHITE, Stroke::new(2.0, pal::ACCENT));
            }

            for &(sx, sy) in &SCALE_HANDLES {
                let c = to_screen(it.handle_pos(Handle::Scale(sx, sy), rot_dist));
                let r = Rect::from_center_size(c, Vec2::splat(HANDLE_R * 1.6));
                gp.rect_filled(r, CornerRadius::same(1), pal::WHITE);
                gp.rect_stroke(
                    r,
                    CornerRadius::same(1),
                    Stroke::new(2.0, pal::ACCENT),
                    StrokeKind::Inside,
                );
            }
        }
    }

    // ---------------- layer list ----------------

    /// Move layer `id` so it is displayed right before display-row `slot` (0 = top, n = bottom).
    fn move_layer(&mut self, id: NodeId, slot: usize) {
        self.items.reverse(); // display order: top first
        if let Some(from) = self.items.iter().position(|i| i.id == id) {
            let it = self.items.remove(from);
            let to = if from < slot { slot - 1 } else { slot };
            self.items.insert(to.min(self.items.len()), it);
        }
        self.items.reverse();
    }

    fn layer_list_ui(&mut self, ui: &mut egui::Ui, inner_width: f32) {
        let n = self.items.len();
        if n == 0 {
            return;
        }

        // Layer list without scroll - just render the rows
        ui.spacing_mut().item_spacing.y = 4.0;
        self.layer_rows(ui, inner_width);
    }

    fn layer_rows(&mut self, ui: &mut egui::Ui, inner_width: f32) {
        let n = self.items.len();
        if n == 0 {
            return;
        }

        let additive = ui.input(|i| i.modifiers.shift || i.modifiers.command);

        let mut rows: Vec<Rect> = Vec::with_capacity(n);
        for d in 0..n {
            let idx = n - 1 - d; // top of the list = front-most layer
            let id = self.items[idx].id;
            let name = self.items[idx].name.clone();
            let mut visible = self.items[idx].visible;

            let (rect, resp) =
                ui.allocate_exact_size(vec2(inner_width, ROW_H), Sense::click_and_drag());
            rows.push(rect);

            if resp.clicked() || resp.drag_started() {
                if additive {
                    self.toggle_select(id);
                } else if !(self.selected == Some(id) || self.extra.contains(&id)) {
                    // keep a multi-selection intact when dragging one of its rows
                    self.select(id);
                } else if resp.clicked() {
                    self.select(id);
                }
            }
            if resp.dragged() {
                self.list_drag = Some(id);
                self.last_activity = Instant::now();
                ui.set_cursor_icon(CursorIcon::Grabbing);
            } else if resp.hovered() && self.list_drag.is_none() {
                ui.set_cursor_icon(CursorIcon::Grab);
            }

            let is_sel = self.selected == Some(id) || self.extra.contains(&id);
            let is_dragged = self.list_drag == Some(id);

            let bg = if is_sel {
                pal::ACTIVE_FILL
            } else if is_dragged {
                pal::hover(pal::BG)
            } else if resp.hovered() {
                pal::hover(pal::BG)
            } else {
                Color32::TRANSPARENT
            };

            ui.painter()
                .rect_filled(rect, CornerRadius::same(LAYER_RADIUS), bg);

            let border_color = if is_sel {
                pal::ACCENT
            } else {
                pal::NODE_STROKE
            };
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(LAYER_RADIUS),
                Stroke::new(1.0, border_color),
                StrokeKind::Inside,
            );

            // Six-dot drag handle on the left
            let handle_rect = Rect::from_min_size(
                egui::pos2(rect.left() + 12.0, rect.center().y - DRAG_HANDLE_SIZE * 0.5),
                vec2(DRAG_HANDLE_SIZE, DRAG_HANDLE_SIZE),
            );
            self.draw_drag_handle(ui, handle_rect);

            // Layer name (smaller text)
            ui.painter().text(
                egui::pos2(
                    rect.left() + 12.0 + DRAG_HANDLE_SIZE + 16.0,
                    rect.center().y,
                ),
                Align2::LEFT_CENTER,
                name,
                FontId::proportional(9.0),
                if visible { pal::TEXT } else { pal::MUTED },
            );

            // Eye toggle (right-aligned, 16×16)
            let eye_pos = rect.right() - 5.0;
            let eye_rect = Rect::from_min_size(
                egui::pos2(eye_pos - 16.0, rect.center().y - 6.0),
                vec2(16.0, 16.0),
            );
            let eye_id = egui::Id::new(format!("eye_{}", id.0));
            let eye_response = ui.interact(eye_rect, eye_id, Sense::click());
            if eye_response.clicked() {
                visible = !visible;
            }

            ui.painter().text(
                eye_rect.center(),
                Align2::CENTER_CENTER,
                if visible { icons::EYE } else { icons::EYE_OFF },
                icons::font(font::ICON),
                pal::TEXT,
            );

            // Update stored visibility
            if self.items[idx].visible != visible {
                self.items[idx].visible = visible;
                if !visible && (self.selected == Some(id) || self.extra.contains(&id)) {
                    // drop it from the selection (promotes another layer if any)
                    self.toggle_select(id);
                    if self.selected.is_none() {
                        self.selection_suppressed = true; // don't let idle auto-select bring it back
                    }
                }
            }
        }

        // --- drop indicator + apply the reorder on release ---
        if let Some(drag_id) = self.list_drag {
            if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                let slot = rows.iter().position(|r| p.y < r.center().y).unwrap_or(n);
                let y = if slot < n {
                    rows[slot].top() - 4.0
                } else {
                    rows[n - 1].bottom() + 4.0
                };
                ui.painter()
                    .hline(rows[0].x_range(), y, Stroke::new(2.0, pal::ACCENT));

                if ui.input(|i| i.pointer.primary_released()) {
                    self.move_layer(drag_id, slot);
                }
            }
        }
        if !ui.input(|i| i.pointer.primary_down()) && self.list_drag.take().is_some() {
            self.last_activity = Instant::now();
        }
    }

    // ---------------- export / attach ----------------

    /// Render all visible layers (back → front) at full canvas resolution.
    fn render_composite(&self) -> RgbaImage {
        let cw = self.canvas_size.x.round().max(1.0) as u32;
        let ch = self.canvas_size.y.round().max(1.0) as u32;
        let mut out = RgbaImage::from_pixel(cw, ch, Rgba([0, 0, 0, 0]));

        for it in &self.items {
            if !it.visible {
                continue;
            }
            // Only layers with a decoded CPU image can be rasterized.
            let Some(path) = it.source_path.as_deref() else {
                continue;
            };
            let Some(src) = self.cpu_image(path) else {
                continue;
            };
            let (sw, sh) = src.dimensions();

            // Bounding box of the rotated rect, clamped to the canvas.
            let cs = it.corners();
            let min_x = cs
                .iter()
                .map(|c| c.x)
                .fold(f32::MAX, f32::min)
                .floor()
                .max(0.0) as u32;
            let min_y = cs
                .iter()
                .map(|c| c.y)
                .fold(f32::MAX, f32::min)
                .floor()
                .max(0.0) as u32;
            let max_x = (cs
                .iter()
                .map(|c| c.x)
                .fold(f32::MIN, f32::max)
                .ceil()
                .max(0.0) as u32)
                .min(cw);
            let max_y = (cs
                .iter()
                .map(|c| c.y)
                .fold(f32::MIN, f32::max)
                .ceil()
                .max(0.0) as u32)
                .min(ch);

            for y in min_y..max_y {
                for x in min_x..max_x {
                    let l = it.to_local(vec2(x as f32 + 0.5, y as f32 + 0.5));
                    let u = l.x / it.size.x + 0.5;
                    let v = l.y / it.size.y + 0.5;
                    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                        continue;
                    }
                    let s = sample_bilinear(&src, u * sw as f32 - 0.5, v * sh as f32 - 0.5);
                    blend_over(out.get_pixel_mut(x, y), s);
                }
            }
        }
        out
    }

    fn attach_canvas(&mut self, ctx: &egui::Context) {
        let decoded = DynamicImage::ImageRgba8(self.render_composite());

        let mut bytes = Vec::new();
        if decoded
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .is_err()
        {
            return;
        }

        let attachment = GPasTeApp::build_attachment(
            ctx,
            "canvas.png".to_owned(),
            "image/png",
            &bytes,
            &decoded,
        );

        self.gpasteapp.set_selected_image(attachment);
    }
}

fn sample_bilinear(img: &RgbaImage, x: f32, y: f32) -> [f32; 4] {
    let (w, h) = img.dimensions();
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);

    let px = |xx, yy| img.get_pixel(xx, yy).0.map(|c| c as f32);
    let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    let mut o = [0.0; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bot = c[i] + (d[i] - c[i]) * fx;
        o[i] = top + (bot - top) * fy;
    }
    o
}

/// Straight-alpha "source over" blend.
fn blend_over(dst: &mut Rgba<u8>, s: [f32; 4]) {
    let sa = s[3] / 255.0;
    let da = dst.0[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa <= 0.0 {
        return;
    }
    for i in 0..3 {
        let d = dst.0[i] as f32;
        dst.0[i] = ((s[i] * sa + d * da * (1.0 - sa)) / oa)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    dst.0[3] = (oa * 255.0).round() as u8;
}

/// Nearest target within `tol`: returns (delta to apply, target position).
fn snap1(val: f32, targets: &[f32], tol: f32) -> Option<(f32, f32)> {
    targets
        .iter()
        .map(|&t| (t - val, t))
        .filter(|(d, _)| d.abs() <= tol)
        .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()))
}

#[cfg(test)]
mod tests {
    use super::alpha_content_uv;
    use image::{Rgba, RgbaImage};

    #[test]
    fn alpha_content_uv_finds_ink_rect() {
        let mut img = RgbaImage::from_pixel(10, 8, Rgba([0, 0, 0, 0]));
        for y in 2..5 {
            for x in 3..7 {
                img.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        assert_eq!(alpha_content_uv(&img, 8), Some([0.3, 0.25, 0.7, 0.625]));
    }

    #[test]
    fn alpha_content_uv_uses_threshold() {
        let mut img = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 0, 0]));
        img.put_pixel(1, 1, Rgba([255, 255, 255, 4]));
        assert_eq!(alpha_content_uv(&img, 8), None);
        img.put_pixel(2, 2, Rgba([255, 255, 255, 9]));
        assert_eq!(alpha_content_uv(&img, 8), Some([0.5, 0.5, 0.75, 0.75]));
    }

    #[test]
    fn text_content_rect_drives_hit_testing() {
        use crate::model::{FilterKind, Kind};
        use eframe::egui::vec2;
        use egui_snarl::NodeId;

        let item = super::Item {
            id: NodeId(0),
            kind: Kind::Filter(FilterKind::Texts),
            name: String::new(),
            tex: None,
            tex_is_full: false,
            visible: true,
            center: vec2(640.0, 360.0),
            size: vec2(1280.0, 720.0),
            angle: 0.0,
            source_path: None,
            fit_pending: false,
            tex_dims: (1280, 720),
            auto_fit: None,
            content_uv: Some([0.25, 0.5, 0.75, 0.9]),
        };

        let (min, max) = item.local_bounds();
        assert!((min - vec2(-320.0, 0.0)).length() < 0.01);
        assert!((max - vec2(320.0, 288.0)).length() < 0.01);
        assert!((item.content_size() - vec2(640.0, 288.0)).length() < 0.01);

        // Inside the ink rect, not the full canvas bitmap.
        assert!(item.contains(vec2(640.0, 500.0)));
        assert!(!item.contains(vec2(640.0, 100.0)));
        assert!(!item.contains(vec2(100.0, 500.0)));

        // Texts can be resized, but has no individual rotate handle.
        let corner = item.handle_pos(super::Handle::Scale(1, -1), 10.0);
        assert!(matches!(
            item.hit_handle(corner, 4.0, 10.0),
            Some(super::Handle::Scale(1, -1))
        ));
        assert!(
            item.hit_handle(item.handle_pos(super::Handle::Rotate, 10.0), 4.0, 10.0)
                .is_none()
        );
    }
}
