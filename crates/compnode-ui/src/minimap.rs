//! Minimap: a small overview of the whole graph with the current view drawn
//! on top. Click or drag it to move the view. The draw list it produces is in
//! **screen** space (it is an overlay, not part of the graph).
use crate::document::Document;
use crate::draw::{DrawList, Layer};
use crate::theme::{Col, Theme};
use compnode_core::geom::vec2;
use compnode_core::{Rect, Vec2, Viewport};

/// Above this many nodes only a sample is drawn (the minimap stays cheap).
pub const MAX_DRAWN_NODES: usize = 2000;

#[derive(Clone, Copy, Debug)]
pub struct Minimap {
    /// Where the minimap sits on screen.
    pub screen: Rect,
    world: Rect,
    scale: f32,
    offset: Vec2,
}

impl Minimap {
    pub fn new(screen: Rect) -> Minimap {
        Minimap {
            screen,
            world: Rect::new(Vec2::ZERO, vec2(1.0, 1.0)),
            scale: 1.0,
            offset: screen.min,
        }
    }

    /// Recompute the world rectangle (all nodes plus the current view) and
    /// the mapping into the minimap rectangle.
    pub fn update(&mut self, doc: &Document, view: &Viewport) {
        let mut world = view.visible_rect(0.0);
        for uid in doc.uids() {
            if let Some(n) = doc.id_of(uid).and_then(|id| doc.graph.node(id)) {
                world = world.union(&n.rect());
            }
        }
        self.set_world(world.expand(20.0));
    }

    /// Use an explicit world rectangle (also used by tests).
    pub fn set_world(&mut self, world: Rect) {
        self.world = world;
        let sw = self.screen.width().max(1.0);
        let sh = self.screen.height().max(1.0);
        self.scale = (sw / world.width().max(1.0)).min(sh / world.height().max(1.0));
        let used = vec2(world.width() * self.scale, world.height() * self.scale);
        self.offset = self.screen.min + vec2((sw - used.x) * 0.5, (sh - used.y) * 0.5);
    }

    pub fn to_map(&self, g: Vec2) -> Vec2 {
        self.offset + (g - self.world.min) * self.scale
    }

    pub fn to_world(&self, p: Vec2) -> Vec2 {
        self.world.min + (p - self.offset) * (1.0 / self.scale)
    }

    /// The view rectangle in minimap coordinates.
    pub fn view_rect(&self, view: &Viewport) -> Rect {
        let r = view.visible_rect(0.0);
        Rect::new(self.to_map(r.min), self.to_map(r.max))
    }

    /// The `Viewport::origin` that centers the world point under `click`.
    pub fn origin_for_click(&self, click: Vec2, view: &Viewport) -> Vec2 {
        let c = self.to_world(click);
        c - view.size * (0.5 / view.zoom)
    }

    pub fn contains(&self, p: Vec2) -> bool {
        self.screen.contains(p)
    }

    /// Push the minimap into `dl` (screen space, `Layer::Overlay`).
    pub fn draw(&self, doc: &Document, view: &Viewport, theme: &Theme, dl: &mut DrawList) {
        dl.rect_stroked(
            Layer::Overlay,
            self.screen,
            4.0,
            theme.c(Col::Popup),
            theme.c(Col::PopupBorder),
            1.0,
        );
        let uids = doc.uids();
        let stride = uids.len().div_ceil(MAX_DRAWN_NODES).max(1);
        for uid in uids.into_iter().step_by(stride) {
            let Some(n) = doc.id_of(uid).and_then(|id| doc.graph.node(id)) else {
                continue;
            };
            let r = n.rect();
            let (a, b) = (self.to_map(r.min), self.to_map(r.max));
            let rect = Rect::new(a, vec2(b.x.max(a.x + 1.5), b.y.max(a.y + 1.5)));
            let color = if n.data.is_frame() {
                theme.c(Col::FrameFill)
            } else {
                theme.header(
                    doc.registry
                        .get(&n.data.schema)
                        .map_or(crate::Category::Utility, |s| s.category),
                )
            };
            dl.rect(Layer::Overlay, rect, 0.0, color);
        }
        let vr = self.view_rect(view);
        dl.rect_stroked(
            Layer::Overlay,
            vr,
            0.0,
            theme.c(Col::BoxFill),
            theme.c(Col::Selected),
            1.5,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::test_support::registry;
    use crate::draw::Prim;

    fn mm() -> Minimap {
        let mut m = Minimap::new(Rect::from_min_size(vec2(10.0, 10.0), vec2(200.0, 100.0)));
        m.set_world(Rect::new(vec2(0.0, 0.0), vec2(2000.0, 500.0)));
        m
    }

    #[test]
    fn mapping_roundtrips_and_letterboxes() {
        let m = mm();
        // 2000x500 into 200x100: scale 0.1, 50 px of height left over
        assert!((m.to_map(vec2(0.0, 0.0)) - vec2(10.0, 35.0)).length() < 1e-4);
        assert!((m.to_map(vec2(2000.0, 500.0)) - vec2(210.0, 85.0)).length() < 1e-3);
        for p in [vec2(0.0, 0.0), vec2(1234.0, 77.0), vec2(2000.0, 500.0)] {
            assert!((m.to_world(m.to_map(p)) - p).length() < 1e-3);
        }
    }

    #[test]
    fn click_centers_the_view_on_the_clicked_world_point() {
        let m = mm();
        let mut view = Viewport::new(vec2(800.0, 600.0));
        view.zoom = 0.5;
        let click = m.to_map(vec2(1000.0, 250.0));
        view.origin = m.origin_for_click(click, &view);
        let center = view.to_graph(view.size * 0.5);
        assert!((center - vec2(1000.0, 250.0)).length() < 1e-2);
        // and the view rectangle is centered on the click
        assert!((m.view_rect(&view).center() - click).length() < 1e-2);
        assert!(m.contains(click) && !m.contains(vec2(500.0, 500.0)));
    }

    #[test]
    fn update_covers_nodes_and_view_and_draw_emits_overlay() {
        let mut d = Document::new(registry());
        d.add_node("t.src", vec2(0.0, 0.0)).unwrap();
        d.add_node("t.sink", vec2(3000.0, 1500.0)).unwrap();
        let view = Viewport::new(vec2(800.0, 600.0));
        let mut m = Minimap::new(Rect::from_min_size(vec2(0.0, 0.0), vec2(200.0, 150.0)));
        m.update(&d, &view);
        for p in [vec2(0.0, 0.0), vec2(3180.0, 1554.0)] {
            assert!(m.screen.contains(m.to_map(p)), "{p:?}");
        }
        let mut dl = DrawList::new();
        m.draw(&d, &view, &Theme::blender_dark(), &mut dl);
        // background, two nodes, view outline
        assert_eq!(dl.layer(Layer::Overlay).len(), 4);
        assert!(
            dl.layer(Layer::Overlay)
                .iter()
                .all(|p| matches!(p, Prim::RoundedRect { .. }))
        );
    }

    #[test]
    fn large_graphs_are_sampled() {
        let mut d = Document::new(registry());
        for i in 0..(MAX_DRAWN_NODES + 500) {
            d.add_node(
                "t.src",
                vec2((i % 100) as f32 * 200.0, (i / 100) as f32 * 100.0),
            )
            .unwrap();
        }
        let view = Viewport::new(vec2(800.0, 600.0));
        let mut m = Minimap::new(Rect::from_min_size(vec2(0.0, 0.0), vec2(200.0, 150.0)));
        m.update(&d, &view);
        let mut dl = DrawList::new();
        m.draw(&d, &view, &Theme::blender_dark(), &mut dl);
        assert!(dl.len() <= MAX_DRAWN_NODES + 2);
    }
}
