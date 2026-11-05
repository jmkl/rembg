//! Turning a `compnode_ui::DrawList` into egui shapes. Runs of solid
//! primitives are merged into one mesh, so a graph with thousands of
//! primitives costs a handful of egui shapes, while text and images keep
//! their place in the back-to-front order.
use compnode_core::Color;
use compnode_ui::draw::{Align, DrawList, Layer, Prim};
use eframe::egui;
use egui::epaint::Mesh;
use egui::{Color32, Pos2, Shape, TextureId, pos2};
use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI};

/// Graph space to local screen space: `(p - origin) * zoom`.
#[derive(Clone, Copy, Debug)]
pub struct Xf {
    pub origin: compnode_core::Vec2,
    pub zoom: f32,
}

impl Xf {
    pub const IDENTITY: Xf = Xf {
        origin: compnode_core::Vec2 { x: 0.0, y: 0.0 },
        zoom: 1.0,
    };
    fn p(&self, v: compnode_core::Vec2) -> Pos2 {
        pos2(
            (v.x - self.origin.x) * self.zoom,
            (v.y - self.origin.y) * self.zoom,
        )
    }
    fn rect(&self, r: compnode_core::Rect) -> egui::Rect {
        egui::Rect::from_min_max(self.p(r.min), self.p(r.max))
    }
}

pub fn c32(c: Color) -> Color32 {
    let [r, g, b, a] = c.to_srgb8();
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn quad(mesh: &mut Mesh, p: [Pos2; 4], col: Color32) {
    let i = mesh.vertices.len() as u32;
    for q in p {
        mesh.colored_vertex(q, col);
    }
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
}

/// Outline of a rounded rectangle, clockwise on screen, `4 * (seg + 1)` points
/// (corner arcs of radius zero collapse onto the corner).
pub fn rounded_outline(rect: egui::Rect, r: f32, seg: usize) -> Vec<Pos2> {
    let r = r.min(rect.width() * 0.5).min(rect.height() * 0.5).max(0.0);
    let corners = [
        (pos2(rect.min.x + r, rect.min.y + r), PI),
        (pos2(rect.max.x - r, rect.min.y + r), 1.5 * PI),
        (pos2(rect.max.x - r, rect.max.y - r), 0.0),
        (pos2(rect.min.x + r, rect.max.y - r), FRAC_PI_2),
    ];
    let mut pts = Vec::with_capacity(4 * (seg + 1));
    for (c, start) in corners {
        for i in 0..=seg {
            let a = start + FRAC_PI_2 * (i as f32 / seg.max(1) as f32);
            pts.push(pos2(c.x + r * a.cos(), c.y + r * a.sin()));
        }
    }
    pts
}

fn segments_for(radius_px: f32) -> usize {
    if radius_px <= 0.75 {
        0
    } else {
        ((radius_px / 2.0).ceil() as usize).clamp(2, 8)
    }
}

fn fill_rounded(mesh: &mut Mesh, rect: egui::Rect, radius_px: f32, col: Color32) {
    if col.a() == 0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let seg = segments_for(radius_px);
    if seg == 0 {
        mesh.add_colored_rect(rect, col);
        return;
    }
    let pts = rounded_outline(rect, radius_px, seg);
    let center = mesh.vertices.len() as u32;
    mesh.colored_vertex(rect.center(), col);
    for p in &pts {
        mesh.colored_vertex(*p, col);
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.add_triangle(center, center + 1 + i, center + 1 + (i + 1) % n);
    }
}

fn stroke_rounded(mesh: &mut Mesh, rect: egui::Rect, radius_px: f32, width_px: f32, col: Color32) {
    if col.a() == 0 || width_px <= 0.0 {
        return;
    }
    let w = width_px.min(rect.width().min(rect.height()) * 0.5);
    let seg = segments_for(radius_px);
    let outer = rounded_outline(rect, radius_px, seg);
    let inner = rounded_outline(rect.shrink(w), (radius_px - w).max(0.0), seg);
    let n = outer.len();
    for i in 0..n {
        let j = (i + 1) % n;
        quad(mesh, [outer[i], outer[j], inner[j], inner[i]], col);
    }
}

/// Thick polyline as a triangle strip with mitered joints (miter length limited).
pub fn polyline(mesh: &mut Mesh, pts: &[Pos2], width_px: f32, col: Color32) {
    let mut p: Vec<Pos2> = Vec::with_capacity(pts.len());
    for q in pts {
        if p.last()
            .is_none_or(|l| (l.x - q.x).abs() + (l.y - q.y).abs() > 1e-3)
        {
            p.push(*q);
        }
    }
    if p.len() < 2 || col.a() == 0 {
        return;
    }
    let normal = |a: Pos2, b: Pos2| {
        let d = b - a;
        let len = d.length().max(1e-6);
        egui::vec2(-d.y / len, d.x / len)
    };
    let half = (width_px * 0.5).max(0.4);
    let base = mesh.vertices.len() as u32;
    for i in 0..p.len() {
        let n = if i == 0 {
            normal(p[0], p[1])
        } else if i == p.len() - 1 {
            normal(p[i - 1], p[i])
        } else {
            let (n0, n1) = (normal(p[i - 1], p[i]), normal(p[i], p[i + 1]));
            let m = n0 + n1;
            let ml = m.length();
            if ml < 1e-4 {
                n0
            } else {
                let m = m / ml;
                m * (1.0 / m.dot(n0).max(0.4))
            }
        };
        mesh.colored_vertex(p[i] + n * half, col);
        mesh.colored_vertex(p[i] - n * half, col);
    }
    for i in 0..(p.len() as u32 - 1) {
        let k = base + 2 * i;
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k + 1, k + 3, k + 2);
    }
}

fn checker(mesh: &mut Mesh, rect: egui::Rect, cell_px: f32, a: Color32, b: Color32) {
    mesh.add_colored_rect(rect, a);
    let cell = cell_px.max(3.0);
    let (nx, ny) = (
        (rect.width() / cell).ceil() as usize,
        (rect.height() / cell).ceil() as usize,
    );
    if nx * ny > 20_000 {
        return; // too fine to see: plain base color
    }
    for j in 0..ny {
        for i in 0..nx {
            if (i + j) % 2 == 1 {
                let min = pos2(rect.min.x + i as f32 * cell, rect.min.y + j as f32 * cell);
                let max = pos2(
                    (min.x + cell).min(rect.max.x),
                    (min.y + cell).min(rect.max.y),
                );
                mesh.add_colored_rect(egui::Rect::from_min_max(min, max), b);
            }
        }
    }
}

/// Push the merged solids, if there are any.
fn flush(mesh: &mut Mesh, out: &mut Vec<Shape>) {
    if !mesh.is_empty() {
        out.push(Shape::mesh(std::mem::take(mesh)));
    }
}

/// Build the shapes for a draw list. `painter` is only used to lay out text.
///
/// The primitives keep their order: a text or an image is emitted between the
/// solids that come before it and the ones that come after it. Without that,
/// the text of a node would float on top of the node drawn after it, and two
/// overlapping nodes would show their labels and widgets through each other.
pub fn build(
    painter: &egui::Painter,
    dl: &DrawList,
    xf: Xf,
    textures: &HashMap<u64, TextureId>,
) -> Vec<Shape> {
    let mut out: Vec<Shape> = Vec::new();
    for layer in Layer::ALL {
        // Solids are merged into as few meshes as possible. A text or an
        // image ends the current run, so it lands between the solids around
        // it instead of on top of all of them.
        let mut mesh = Mesh::default();
        for prim in dl.layer(layer) {
            match prim {
                Prim::RoundedRect {
                    rect,
                    radius,
                    fill,
                    stroke,
                } => {
                    let r = xf.rect(*rect);
                    let rp = radius * xf.zoom;
                    fill_rounded(&mut mesh, r, rp, c32(*fill));
                    if let Some((c, w)) = stroke {
                        stroke_rounded(&mut mesh, r, rp, (w * xf.zoom).max(0.5), c32(*c));
                    }
                }
                Prim::LineStrip {
                    points,
                    color,
                    width,
                } => {
                    let pts: Vec<Pos2> = points.iter().map(|p| xf.p(*p)).collect();
                    polyline(&mut mesh, &pts, (width * xf.zoom).max(0.5), c32(*color));
                }
                Prim::Triangle { p, color } => {
                    let i = mesh.vertices.len() as u32;
                    for v in p {
                        mesh.colored_vertex(xf.p(*v), c32(*color));
                    }
                    mesh.add_triangle(i, i + 1, i + 2);
                }
                Prim::GradientQuad { rect, colors } => {
                    let r = xf.rect(*rect);
                    let i = mesh.vertices.len() as u32;
                    mesh.colored_vertex(r.left_top(), c32(colors[0]));
                    mesh.colored_vertex(r.right_top(), c32(colors[1]));
                    mesh.colored_vertex(r.right_bottom(), c32(colors[3]));
                    mesh.colored_vertex(r.left_bottom(), c32(colors[2]));
                    mesh.add_triangle(i, i + 1, i + 2);
                    mesh.add_triangle(i, i + 2, i + 3);
                }
                Prim::Checker { rect, cell, a, b } => {
                    checker(&mut mesh, xf.rect(*rect), cell * xf.zoom, c32(*a), c32(*b));
                }
                Prim::Image {
                    rect,
                    texture,
                    tint,
                } => {
                    flush(&mut mesh, &mut out);
                    if let Some(id) = textures.get(texture) {
                        let mut m = Mesh::with_texture(*id);
                        m.add_rect_with_uv(
                            xf.rect(*rect),
                            egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            c32(*tint),
                        );
                        out.push(Shape::mesh(m));
                    }
                }
                Prim::Text {
                    pos,
                    text,
                    size,
                    color,
                    align,
                } => {
                    flush(&mut mesh, &mut out);
                    let font = egui::FontId::proportional((size * xf.zoom).max(1.0));
                    let galley = painter.layout_no_wrap(text.clone(), font, c32(*color));
                    let sz = galley.size();
                    let at = xf.p(*pos);
                    let x = match align {
                        Align::Left => at.x,
                        Align::Center => at.x - sz.x * 0.5,
                        Align::Right => at.x - sz.x,
                    };
                    out.push(Shape::galley(
                        pos2(x, at.y - sz.y * 0.5),
                        galley,
                        c32(*color),
                    ));
                }
            }
        }
        flush(&mut mesh, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use compnode_core::geom::vec2;

    fn rect() -> egui::Rect {
        egui::Rect::from_min_max(pos2(10.0, 20.0), pos2(110.0, 70.0))
    }

    /// A painter with fonts loaded.
    fn painter() -> egui::Painter {
        let ctx = egui::Context::default();
        let mut p = None;
        let out = ctx.run_ui(egui::RawInput::default(), |ui| {
            p = Some(ui.painter().clone());
        });
        // Nobody uploads the font atlas in a test.
        out.drop_without_applying_deltas();
        p.expect("painter")
    }

    #[test]
    fn text_keeps_its_place_between_solids() {
        let mut dl = DrawList::new();
        let r = compnode_core::Rect::new(vec2(0.0, 0.0), vec2(10.0, 10.0));
        dl.rect(Layer::Nodes, r, 0.0, Color::BLACK);
        dl.text(
            Layer::Nodes,
            vec2(0.0, 0.0),
            "hello",
            12.0,
            Color::WHITE,
            Align::Left,
        );
        dl.rect(Layer::Nodes, r, 0.0, Color::BLACK);

        let shapes = build(&painter(), &dl, Xf::IDENTITY, &HashMap::new());
        assert_eq!(shapes.len(), 3);
        assert!(matches!(shapes[0], egui::Shape::Mesh(_)));
        assert!(matches!(shapes[1], egui::Shape::Text(_)));
        assert!(matches!(shapes[2], egui::Shape::Mesh(_)));
    }

    #[test]
    fn an_overlapping_node_is_drawn_over_the_text_of_the_one_below() {
        use compnode_ui::schema::{Category, NodeSchema, Registry, SocketDef};
        use compnode_ui::value::SocketType;

        let mut reg = Registry::new();
        reg.register(
            NodeSchema::new("t.a", "A", Category::Utility)
                .input(SocketDef::new("Value", SocketType::Float)),
        );
        let mut e = compnode_ui::Editor::new(reg);
        e.add_node("t.a", vec2(0.0, 0.0));
        // the second node sits on top of the first one
        e.add_node("t.a", vec2(20.0, 0.0));

        let mut dl = DrawList::new();
        e.draw(&mut dl);
        let shapes = build(&painter(), &dl, Xf::IDENTITY, &HashMap::new());

        let first_text = shapes
            .iter()
            .position(|s| matches!(s, egui::Shape::Text(_)))
            .expect("a label");
        let last_mesh = shapes
            .iter()
            .rposition(|s| matches!(s, egui::Shape::Mesh(_)))
            .expect("a body");
        assert!(
            first_text < last_mesh,
            "the label of the first node is drawn on top of the second node"
        );
    }

    #[test]
    fn outline_has_fixed_point_count_and_stays_inside() {
        for (r, seg) in [(0.0, 0), (6.0, 3), (100.0, 8)] {
            let pts = rounded_outline(rect(), r, seg);
            assert_eq!(pts.len(), 4 * (seg + 1));
            let big = rect().expand(0.01);
            assert!(pts.iter().all(|p| big.contains(*p)), "{r}");
        }
        // zero radius is the rectangle's corners
        let pts = rounded_outline(rect(), 0.0, 0);
        assert_eq!(pts[0], rect().left_top());
        assert_eq!(pts[2], rect().right_bottom());
    }

    #[test]
    fn filled_rects_use_one_mesh_and_valid_indices() {
        let mut m = Mesh::default();
        fill_rounded(&mut m, rect(), 6.0, Color32::RED);
        fill_rounded(&mut m, rect(), 0.0, Color32::BLUE);
        stroke_rounded(&mut m, rect(), 6.0, 2.0, Color32::WHITE);
        assert!(!m.is_empty());
        let n = m.vertices.len() as u32;
        assert!(m.indices.iter().all(|i| *i < n));
        assert_eq!(m.indices.len() % 3, 0);
    }

    #[test]
    fn polyline_makes_a_strip_and_skips_degenerate_input() {
        let mut m = Mesh::default();
        polyline(
            &mut m,
            &[pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(20.0, 10.0)],
            2.0,
            Color32::WHITE,
        );
        assert_eq!(m.vertices.len(), 6);
        assert_eq!(m.indices.len(), 12);
        // width is respected on a straight segment
        assert!((m.vertices[0].pos.y - m.vertices[1].pos.y).abs() - 2.0 < 1e-4);
        let mut e = Mesh::default();
        polyline(
            &mut e,
            &[pos2(1.0, 1.0), pos2(1.0, 1.0)],
            2.0,
            Color32::WHITE,
        );
        polyline(&mut e, &[pos2(1.0, 1.0)], 2.0, Color32::WHITE);
        assert!(e.is_empty());
    }

    #[test]
    fn colors_convert_through_srgb() {
        assert_eq!(c32(Color::WHITE), Color32::WHITE);
        assert_eq!(c32(Color::BLACK), Color32::BLACK);
        let half = c32(Color::new(0.0, 0.0, 0.0, 0.5));
        assert!(half.a() >= 127 && half.a() <= 128);
    }

    #[test]
    fn checker_alternates_cells() {
        let mut m = Mesh::default();
        checker(
            &mut m,
            egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(40.0, 40.0)),
            10.0,
            Color32::GRAY,
            Color32::WHITE,
        );
        // 1 base rect + 8 white cells of a 4x4 board
        assert_eq!(m.vertices.len(), 4 + 8 * 4);
    }
}
