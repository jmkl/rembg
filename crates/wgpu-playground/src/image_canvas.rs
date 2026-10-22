//! `ImageCanvas` – a self-contained egui widget:
//!
//! ```text
//! ┌──────────────────────┐
//! │       CANVAS         │  1280×720 logical canvas (scaled to fit the available width)
//! └──────────────────────┘
//! [ Add image ] [ Delete ]
//! ┌──────────────────────┐
//! │ ▣ Layer 1            │  layer list: top row = front-most layer
//! │ ▣ Layer 2            │  drag a row up/down to change the stacking order
//! └──────────────────────┘
//! ```
//!
//! Usage:
//! ```ignore
//! struct MyApp { canvas: ImageCanvas }
//! // in `App::ui`:
//! egui::CentralPanel::default().show(ui, |ui| { self.canvas.show(ui); });
//! ```
//!
//! Gizmos on the selected layer: drag body = move, corners = uniform scale,
//! edges = stretch, round handle = rotate (Shift = 15° snap).

use eframe::egui::{
    self, Align2, Color32, ColorImage, CornerRadius, CursorIcon, FontId, Key, Margin, Mesh, Pos2,
    Rect, Sense, Shape, Stroke, StrokeKind, TextureHandle, TextureId, TextureOptions, Vec2,
    epaint::Vertex, pos2, vec2,
};
use std::f32::consts::FRAC_PI_2;

const HANDLE_R: f32 = 8.0; // screen pixels
const ROT_DIST: f32 = 32.0; // screen pixels
const MIN_SIZE: f32 = 8.0; // canvas units
const MAX_TEX_SIDE: u32 = 4096;
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

struct Item {
    id: u64,
    name: String,
    tex: TextureHandle,
    /// Center in canvas coordinates
    center: Vec2,
    /// Size in canvas units (before rotation)
    size: Vec2,
    /// Radians, clockwise on screen
    angle: f32,
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

/// Read-only view of a layer (for exporting / inspecting from your app).
pub struct LayerInfo<'a> {
    pub id: u64,
    pub name: &'a str,
    pub texture: &'a TextureHandle,
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
    accept_drops: bool,

    /// Stored back → front (last = top-most). The layer list shows it reversed.
    items: Vec<Item>,
    next_id: u64,
    selected: Option<u64>,
    drag: Drag,
    list_drag: Option<u64>,
    error: Option<String>,
}

impl Default for ImageCanvas {
    fn default() -> Self {
        Self {
            canvas_size: vec2(1280.0, 720.0),
            background: Color32::from_gray(245),
            accept_drops: true,
            items: vec![],
            next_id: 1,
            selected: None,
            drag: Drag::None,
            list_drag: None,
            error: None,
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

    /// Whether image files dropped onto the window are added as layers (default true).
    pub fn accept_drops(mut self, yes: bool) -> Self {
        self.accept_drops = yes;
        self
    }

    // ---------------- public API ----------------

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn selected(&self) -> Option<u64> {
        self.selected
    }

    pub fn remove(&mut self, id: u64) {
        self.items.retain(|i| i.id != id);
        if self.selected == Some(id) {
            self.selected = None;
        }
        self.drag = Drag::None;
    }

    /// Layers from back to front.
    pub fn layers(&self) -> impl Iterator<Item = LayerInfo<'_>> {
        self.items.iter().map(|i| LayerInfo {
            id: i.id,
            name: &i.name,
            texture: &i.tex,
            center: i.center,
            size: i.size,
            angle: i.angle,
        })
    }

    /// Decode PNG/JPEG/WebP/BMP/GIF bytes and add them as a new top layer.
    pub fn add_image_bytes(
        &mut self,
        ctx: &egui::Context,
        name: String,
        bytes: &[u8],
    ) -> Result<u64, String> {
        let mut img =
            image::load_from_memory(bytes).map_err(|e| format!("Could not load {name}: {e}"))?;
        if img.width().max(img.height()) > MAX_TEX_SIDE {
            img = img.resize(
                MAX_TEX_SIDE,
                MAX_TEX_SIDE,
                image::imageops::FilterType::Triangle,
            );
        }
        let rgba = img.to_rgba8();
        let ci = ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_raw(),
        );
        Ok(self.add_color_image(ctx, name, ci))
    }

    /// Add an already-decoded image as a new top layer.
    pub fn add_color_image(&mut self, ctx: &egui::Context, name: String, ci: ColorImage) -> u64 {
        let (w, h) = (ci.width(), ci.height());
        let tex = ctx.load_texture(name.clone(), ci, TextureOptions::LINEAR);
        // fit the initial size into ~45% of the canvas height
        let s = ((self.canvas_size.y * 0.45) / w.max(h) as f32).min(1.0);
        let off = (self.items.len() % 6) as f32 * 30.0;
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(Item {
            id,
            name,
            tex,
            center: self.canvas_size / 2.0 + vec2(off - 75.0, off - 75.0),
            size: vec2(w as f32 * s, h as f32 * s),
            angle: 0.0,
        });
        self.selected = Some(id);
        id
    }

    // ---------------- the widget ----------------

    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.handle_dropped_files(ui);

        // Delete key (not while typing in a field)
        if ui.input(|i| i.key_pressed(Key::Delete)) && ui.memory(|m| m.focused().is_none()) {
            if let Some(id) = self.selected {
                self.remove(id);
            }
        }

        let scale = (ui.available_width() / self.canvas_size.x)
            .min(1.0)
            .max(0.05);
        self.canvas_ui(ui, scale);

        ui.add_space(10.0);
        self.buttons_ui(ui);
        if let Some(e) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, e);
        }
        ui.add_space(10.0);

        self.layer_list_ui(ui);
    }

    fn handle_dropped_files(&mut self, ui: &egui::Ui) {
        if !self.accept_drops {
            return;
        }
        let ctx = ui.ctx().clone();
        let dropped = ui.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            // egui 0.36: `DroppedFile` is a trait
            let name = f
                .path()
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "dropped image".to_owned());
            match f.bytes() {
                Ok(bytes) => {
                    let r = self.add_image_bytes(&ctx, name, &bytes);
                    self.report(r);
                }
                Err(e) => self.error = Some(format!("{name}: {e}")),
            }
        }
    }

    fn report(&mut self, r: Result<u64, String>) {
        self.error = r.err();
    }

    // ---------------- buttons ----------------

    fn buttons_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .add_sized([120.0, 32.0], egui::Button::new("Add image…"))
                .clicked()
            {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("images", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
                    .pick_files()
                {
                    let ctx = ui.ctx().clone();
                    for p in paths {
                        let name = p
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        match std::fs::read(&p) {
                            Ok(bytes) => {
                                let r = self.add_image_bytes(&ctx, name, &bytes);
                                self.report(r);
                            }
                            Err(e) => self.error = Some(format!("{name}: {e}")),
                        }
                    }
                }
            }
            let has_sel = self.selected.is_some();
            if ui
                .add_enabled(
                    has_sel,
                    egui::Button::new("Delete").min_size(vec2(120.0, 32.0)),
                )
                .clicked()
            {
                if let Some(id) = self.selected {
                    self.remove(id);
                }
            }
        });
    }

    // ---------------- canvas ----------------

    fn sel_index(&self) -> Option<usize> {
        let id = self.selected?;
        self.items.iter().position(|i| i.id == id)
    }

    fn on_press(&mut self, p: Vec2, tol: f32, rot_dist: f32) {
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
        painter.rect_filled(rect, CornerRadius::ZERO, self.background);
        for it in &self.items {
            let c = it.corners();
            let uvs = [
                Pos2::new(0.0, 0.0),
                Pos2::new(1.0, 0.0),
                Pos2::new(1.0, 1.0),
                Pos2::new(0.0, 1.0),
            ];
            let mut mesh = Mesh::with_texture(it.tex.id());
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
    fn move_layer(&mut self, id: u64, slot: usize) {
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
            ui.colored_label(
                Color32::from_gray(150),
                "No layers yet – add an image or drop files here.",
            );
            return;
        }

        let mut rows: Vec<Rect> = Vec::with_capacity(n);
        for d in 0..n {
            let idx = n - 1 - d; // top of the list = front-most layer
            let (id, name, tex_id, size): (u64, String, TextureId, Vec2) = {
                let it = &self.items[idx];
                (it.id, it.name.clone(), it.tex.id(), it.size)
            };

            let (rect, resp) =
                ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
            rows.push(rect);

            if resp.clicked() || resp.drag_started() {
                self.selected = Some(id);
            }
            if resp.dragged() {
                self.list_drag = Some(id);
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
            painter.image(
                tex_id,
                thumb,
                Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                Color32::WHITE,
            );

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
        if !ui.input(|i| i.pointer.primary_down()) {
            self.list_drag = None;
        }
    }
}
