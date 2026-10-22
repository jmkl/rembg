//! `ImageCanvas` – a self-contained egui widget that mirrors a compnode viewer's layers.
//!
//! ```text
//! ┌──────────────────────┐
//! │  W: __x H: __         │  configurable canvas size
//! ├──────────────────────┤
//! │       CANVAS         │  scaled to fit the available width
//! └──────────────────────┘
//! ┌──────────────────────┐
//! │ ▣ Layer 1            │  layer list: top row = front-most layer
//! │ ▣ Layer 2            │  drag a row up/down to change stacking order
//! └──────────────────────┘
//! ```
//!
//! Gizmos on the selected layer: drag body = move, corners = uniform scale,
//! edges = stretch, round handle = rotate (Shift = 15° snap).

use crate::layer_cache::LayerCache;
use crate::style::pal;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, FontId, Margin, Mesh, Pos2, Rect, Sense,
    Shape, Stroke, StrokeKind, TextureHandle, TextureId, Vec2, epaint::Vertex, pos2, vec2,
};
use egui_snarl::NodeId;
use std::collections::{HashMap, HashSet};
use std::f32::consts::FRAC_PI_2;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const HANDLE_R: f32 = 8.0; // screen pixels
const ROT_DIST: f32 = 32.0; // screen pixels
const MIN_SIZE: f32 = 8.0; // canvas units
const ROW_H: f32 = 34.0;
const ACCENT: Color32 = Color32::from_rgb(0, 140, 255);

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
    Move { grab: Vec2 },
    Rotate,
    Scale { sx: i8, sy: i8, start: Snap },
}

pub struct Item {
    /// Source node id (the node whose output this layer shows).
    id: NodeId,
    name: String,
    /// Full texture from the layer cache, or the node's low-res preview while
    /// the file is still being decoded.
    tex: Option<TextureHandle>,
    /// True once `tex` comes from the layer cache (not the placeholder).
    tex_is_full: bool,
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
}

impl Item {
    fn snap(&self) -> Snap {
        Snap {
            center: self.center,
            size: self.size,
            angle: self.angle,
        }
    }

    fn to_local(&self, p: Vec2) -> Vec2 {
        rot(p - self.center, -self.angle)
    }

    fn contains(&self, p: Vec2) -> bool {
        let l = self.to_local(p);
        l.x.abs() <= self.size.x / 2.0 && l.y.abs() <= self.size.y / 2.0
    }

    fn corners(&self) -> [Vec2; 4] {
        let (hw, hh) = (self.size.x / 2.0, self.size.y / 2.0);
        [vec2(-hw, -hh), vec2(hw, -hh), vec2(hw, hh), vec2(-hw, hh)]
            .map(|l| self.center + rot(l, self.angle))
    }

    fn handle_pos(&self, h: Handle, rot_dist: f32) -> Vec2 {
        let local = match h {
            Handle::Scale(sx, sy) => {
                vec2(sx as f32 * self.size.x / 2.0, sy as f32 * self.size.y / 2.0)
            }
            Handle::Rotate => vec2(0.0, -self.size.y / 2.0 - rot_dist),
        };
        self.center + rot(local, self.angle)
    }

    fn hit_handle(&self, p: Vec2, tol: f32, rot_dist: f32) -> Option<Handle> {
        std::iter::once(Handle::Rotate)
            .chain(SCALE_HANDLES.iter().map(|&(x, y)| Handle::Scale(x, y)))
            .find(|&h| (self.handle_pos(h, rot_dist) - p).length() <= tol)
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
    selected: Option<NodeId>,
    drag: Drag,
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
}

/// How often a sync is forced even if nothing changed, so files modified on
/// disk are noticed.
const FORCE_SYNC_EVERY: Duration = Duration::from_millis(750);

impl Default for ImageCanvas {
    fn default() -> Self {
        Self {
            canvas_size: vec2(1280.0, 720.0),
            background: Color32::from_gray(245),
            items: vec![],
            cache: LayerCache::default(),
            remembered: HashMap::new(),
            selected: None,
            drag: Drag::None,
            list_drag: None,
            last_activity: Instant::now(),
            layer_count: 0,
            last_sig: None,
            dirty: false,
            last_sync: Instant::now(),
            in_use: HashSet::new(),
            committed_activity: None,
        }
    }
}

impl ImageCanvas {
    pub fn new() -> Self {
        Self::default()
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
        matches!(
            self.drag,
            Drag::Move { .. } | Drag::Rotate | Drag::Scale { .. }
        ) || self.list_drag.is_some()
    }

    /// Whether the canvas has been idle (no drag/scale/rotate/reorder) for at
    /// least `secs` seconds. Returns true when nothing is being moved/rotated
    /// scaled/reordered and nothing has changed in the last `secs` seconds.
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
    /// `true` only when [`ImageCanvas::sync_layers`] actually needs to run:
    /// the layer set changed, a background decode finished, or the periodic
    /// file-modification check is due. Otherwise the frame costs one
    /// non-blocking channel poll and a few comparisons.
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
    /// Each entry is `(source_node_id, source_name, preview_texture, filepath)`.
    /// Called every frame, so it must be cheap when nothing changed:
    ///
    /// * Items are matched by source node id; transforms of existing items are
    ///   never touched.
    /// * The image for a `filepath` is looked up in the canvas' own
    ///   [`LayerCache`]. On a miss it is decoded on a background thread into a
    ///   CPU image and a texture, then stored, so the same file is never
    ///   decoded twice. While it loads, `preview_texture` is shown.
    /// * When a layer disappears its transform is remembered by filepath; if
    ///   the same file is connected again it gets its old center/size/angle
    ///   back, and its image comes straight from the cache.
    pub fn sync_layers(
        &mut self,
        ctx: &egui::Context,
        layers: &[(NodeId, String, Option<egui::TextureHandle>, Option<PathBuf>)],
    ) {
        let in_use: HashSet<PathBuf> = layers.iter().filter_map(|(_, _, _, p)| p.clone()).collect();

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

        let canvas_size = self.canvas_size;
        for (source_id, source_name, preview, path) in layers {
            // Ask the cache first: hit = already a CPU image + texture.
            let cached = path
                .as_deref()
                .and_then(|p| self.cache.get_or_load(ctx, p))
                .map(|c| (c.texture.clone(), c.dims));

            let idx = match self.items.iter().position(|i| i.id == *source_id) {
                Some(i) => i,
                None => {
                    let snap = path.as_ref().and_then(|p| self.remembered.remove(p));
                    self.items.push(Item {
                        id: *source_id,
                        name: format!("{} #{}", source_name, source_id.0),
                        tex: None,
                        tex_is_full: false,
                        center: snap.map_or(canvas_size / 2.0, |s| s.center),
                        size: snap.map_or(vec2(200.0, 200.0), |s| s.size),
                        angle: snap.map_or(0.0, |s| s.angle),
                        source_path: path.clone(),
                        fit_pending: snap.is_none(),
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

            match cached {
                Some((tex, dims)) => {
                    if it.fit_pending {
                        it.size = fit_inside(dims, canvas_size);
                        it.fit_pending = false;
                    } else if !it.tex_is_full {
                        // Placeholder -> real image: keep the user's width,
                        // correct the aspect only if it differs.
                        let aspect = dims.0 as f32 / dims.1.max(1) as f32;
                        if (it.size.x / it.size.y.max(1.0) - aspect).abs() > 0.01 {
                            it.size.y = it.size.x / aspect.max(0.01);
                        }
                    }
                    if it.tex.as_ref().map(|t| t.id()) != Some(tex.id()) {
                        it.tex = Some(tex);
                    }
                    it.tex_is_full = true;
                }
                None => {
                    // Still loading (or failed): show the node preview.
                    if !it.tex_is_full {
                        if let Some(prev) = preview {
                            if it.tex.as_ref().map(|t| t.id()) != Some(prev.id()) {
                                it.tex = Some(prev.clone());
                            }
                            if it.fit_pending {
                                let [w, h] = prev.size();
                                it.size = fit_inside((w as u32, h as u32), canvas_size);
                                // stays pending: refit with real dims later
                            }
                        }
                    }
                }
            }
        }

        if self.selected.is_none() && self.last_activity.elapsed().as_secs_f32() >= 2.0 {
            self.selected = self.items.last().map(|i| i.id);
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

    pub fn show(&mut self, ui: &mut egui::Ui) {
        // --- canvas size inputs ---
        ui.horizontal(|ui| {
            ui.label("Canvas W:");
            ui.add(
                egui::DragValue::new(&mut self.canvas_size.x)
                    .range(64.0..=8192.0)
                    .speed(1.0),
            );
            ui.label("H:");
            ui.add(
                egui::DragValue::new(&mut self.canvas_size.y)
                    .range(64.0..=8192.0)
                    .speed(1.0),
            );
        });
        ui.add_space(6.0);

        let scale = (ui.available_width() / self.canvas_size.x)
            .min(1.0)
            .max(0.05);
        self.canvas_ui(ui, scale);

        ui.add_space(10.0);
        self.layer_list_ui(ui);
    }

    // ---------------- canvas ----------------

    fn sel_index(&self) -> Option<usize> {
        let id = self.selected?;
        self.items.iter().position(|i| i.id == id)
    }

    fn on_press(&mut self, p: Vec2, tol: f32, rot_dist: f32) {
        self.last_activity = Instant::now();
        // 1) gizmo handles of the current selection
        if let Some(i) = self.sel_index() {
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
        // 2) top-most image under the cursor (selecting does not change the order)
        if let Some(i) = (0..self.items.len())
            .rev()
            .find(|&i| self.items[i].contains(p))
        {
            let it = &self.items[i];
            self.drag = Drag::Move {
                grab: p - it.center,
            };
            self.selected = Some(it.id);
        } else {
            self.selected = None;
            self.drag = Drag::None;
        }
    }

    fn on_drag(&mut self, p: Vec2, shift: bool) {
        self.last_activity = Instant::now();
        let Some(i) = self.sel_index() else { return };
        let drag = self.drag;
        let it = &mut self.items[i];
        match drag {
            Drag::None => {}
            Drag::Move { grab } => it.center = p - grab,
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
                let (w0, h0) = (start.size.x, start.size.y);
                let lp = rot(p - start.center, -start.angle);
                // the opposite handle (anchor) stays fixed in world space
                let (ax, ay) = (-fx * w0 / 2.0, -fy * h0 / 2.0);
                let (mut w, mut h) = (w0, h0);
                if sx != 0 && sy != 0 {
                    let dw = ((lp.x - ax) * fx).max(MIN_SIZE);
                    let dh = ((lp.y - ay) * fy).max(MIN_SIZE);
                    let s = (dw / w0).max(dh / h0);
                    w = w0 * s;
                    h = h0 * s;
                } else if sx != 0 {
                    w = ((lp.x - ax) * fx).max(MIN_SIZE);
                } else {
                    h = ((lp.y - ay) * fy).max(MIN_SIZE);
                }
                let anchor_world = start.center + rot(vec2(ax, ay), start.angle);
                it.size = vec2(w, h);
                it.center = anchor_world + rot(vec2(fx * w / 2.0, fy * h / 2.0), start.angle);
            }
        }
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
        let (pressed, down, pos, shift) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.interact_pos(),
                i.modifiers.shift,
            )
        });

        // allow grabbing gizmo handles that stick out of the canvas (e.g. the rotate handle)
        let on_outside_handle = pos.is_some_and(|p| {
            rect.expand(HANDLE_R + ROT_DIST + 4.0).contains(p)
                && self.sel_index().is_some_and(|i| {
                    self.items[i]
                        .hit_handle(to_canvas(p), tol, rot_dist)
                        .is_some()
                })
        });

        if pressed && (resp.hovered() || on_outside_handle) {
            if let Some(p) = pos {
                self.on_press(to_canvas(p), tol, rot_dist);
            }
        } else if down {
            if let Some(p) = pos {
                self.on_drag(to_canvas(p), shift);
            }
        }
        if !down {
            if !matches!(self.drag, Drag::None) {
                self.last_activity = Instant::now();
            }
            self.drag = Drag::None;
        }

        // --- cursor feedback ---
        if matches!(self.drag, Drag::None) && (resp.hovered() || on_outside_handle) {
            if let Some(p) = pos.map(to_canvas) {
                let mut icon = None;
                if let Some(i) = self.sel_index() {
                    if let Some(h) = self.items[i].hit_handle(p, tol, rot_dist) {
                        icon = Some(cursor_for(h));
                    }
                }
                if icon.is_none() && self.items.iter().any(|it| it.contains(p)) {
                    icon = Some(CursorIcon::Move);
                }
                if let Some(c) = icon {
                    ui.set_cursor_icon(c);
                }
            }
        }

        // --- draw canvas + layers (back → front) ---
        // Checkerboard background matching the node preview widgets.
        painter.rect_filled(rect, CornerRadius::ZERO, pal::CHECK_A);

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
                painter.rect_filled(tile, CornerRadius::ZERO, pal::CHECK_B);
                x += cell * 2.0;
            }
            y += cell;
            row += 1;
        }

        for it in &self.items {
            let Some(tex) = &it.tex else { continue };
            let c = it.corners();
            let uvs = [
                Pos2::new(0.0, 0.0),
                Pos2::new(1.0, 0.0),
                Pos2::new(1.0, 1.0),
                Pos2::new(0.0, 1.0),
            ];
            let mut mesh = Mesh::with_texture(tex.id());
            for k in 0..4 {
                mesh.vertices.push(Vertex {
                    pos: to_screen(c[k]),
                    uv: uvs[k],
                    color: Color32::WHITE,
                });
            }
            mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
            painter.add(Shape::mesh(mesh));
        }
        painter.rect_stroke(
            rect,
            CornerRadius::ZERO,
            Stroke::new(1.0, Color32::from_gray(120)),
            StrokeKind::Inside,
        );

        // --- gizmo (painted with a larger clip so handles outside the canvas stay visible) ---
        if let Some(i) = self.sel_index() {
            let gp = ui.painter().with_clip_rect(
                rect.expand(HANDLE_R + ROT_DIST + 4.0)
                    .intersect(ui.clip_rect()),
            );
            let it = &self.items[i];
            let pts: Vec<Pos2> = it.corners().iter().map(|&c| to_screen(c)).collect();
            gp.add(Shape::closed_line(pts, Stroke::new(1.5, ACCENT)));

            let top_mid = to_screen(it.handle_pos(Handle::Scale(0, -1), rot_dist));
            let rot_pos = to_screen(it.handle_pos(Handle::Rotate, rot_dist));
            gp.line_segment([top_mid, rot_pos], Stroke::new(1.5, ACCENT));
            gp.circle(rot_pos, HANDLE_R, Color32::WHITE, Stroke::new(2.0, ACCENT));

            for &(sx, sy) in &SCALE_HANDLES {
                let c = to_screen(it.handle_pos(Handle::Scale(sx, sy), rot_dist));
                let r = Rect::from_center_size(c, Vec2::splat(HANDLE_R * 1.6));
                gp.rect_filled(r, CornerRadius::same(1), Color32::WHITE);
                gp.rect_stroke(
                    r,
                    CornerRadius::same(1),
                    Stroke::new(2.0, ACCENT),
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

    fn layer_list_ui(&mut self, ui: &mut egui::Ui) {
        let h = ui.available_height().max(120.0);
        egui::Frame::new()
            .fill(Color32::from_gray(38))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height((h - 20.0).max(60.0))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        self.layer_rows(ui);
                    });
            });
    }

    fn layer_rows(&mut self, ui: &mut egui::Ui) {
        let n = self.items.len();
        if n == 0 {
            ui.colored_label(Color32::from_gray(150), "No layers yet.");
            return;
        }

        let mut rows: Vec<Rect> = Vec::with_capacity(n);
        for d in 0..n {
            let idx = n - 1 - d; // top of the list = front-most layer
            let (id, name, tex_id, size): (NodeId, String, Option<TextureId>, Vec2) = {
                let it = &self.items[idx];
                (
                    it.id,
                    it.name.clone(),
                    it.tex.as_ref().map(|t| t.id()),
                    it.size,
                )
            };

            let (rect, resp) =
                ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
            rows.push(rect);

            if resp.clicked() || resp.drag_started() {
                self.selected = Some(id);
            }
            if resp.dragged() {
                self.list_drag = Some(id);
                self.last_activity = Instant::now();
                ui.set_cursor_icon(CursorIcon::Grabbing);
            } else if resp.hovered() && self.list_drag.is_none() {
                ui.set_cursor_icon(CursorIcon::Grab);
            }

            let is_sel = self.selected == Some(id);
            let is_dragged = self.list_drag == Some(id);
            let mut bg = if is_sel {
                Color32::from_rgb(205, 228, 255)
            } else {
                Color32::from_gray(240)
            };
            if is_dragged {
                bg = bg.gamma_multiply(0.5);
            }

            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::same(3), bg);
            if is_sel {
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(3),
                    Stroke::new(1.5, ACCENT),
                    StrokeKind::Inside,
                );
            }

            // thumbnail (aspect-fit into a square)
            let box_sz = ROW_H - 8.0;
            let aspect = (size.x / size.y).max(0.01);
            let (tw, th) = if aspect >= 1.0 {
                (box_sz, box_sz / aspect)
            } else {
                (box_sz * aspect, box_sz)
            };
            let thumb = Rect::from_center_size(
                pos2(rect.left() + 4.0 + box_sz / 2.0, rect.center().y),
                vec2(tw, th),
            );
            if let Some(tex_id) = tex_id {
                painter.image(
                    tex_id,
                    thumb,
                    Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }

            painter.text(
                pos2(rect.left() + box_sz + 14.0, rect.center().y),
                Align2::LEFT_CENTER,
                name,
                FontId::proportional(14.0),
                Color32::BLACK,
            );
        }

        // --- drop indicator + apply the reorder on release ---
        if let Some(drag_id) = self.list_drag {
            if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                let slot = rows.iter().position(|r| p.y < r.center().y).unwrap_or(n);
                let y = if slot < n {
                    rows[slot].top() - 3.0
                } else {
                    rows[n - 1].bottom() + 3.0
                };
                ui.painter()
                    .hline(rows[0].x_range(), y, Stroke::new(2.0, ACCENT));

                if ui.input(|i| i.pointer.primary_released()) {
                    self.move_layer(drag_id, slot);
                }
            }
        }
        if !ui.input(|i| i.pointer.primary_down()) && self.list_drag.take().is_some() {
            self.last_activity = Instant::now();
        }
    }
}
