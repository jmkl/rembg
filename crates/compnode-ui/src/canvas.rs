//! Output canvas viewer state: pan, zoom, fit and 1:1 for a fixed-size
//! canvas (2048 x 2048 by default). Headless, like the rest of this crate.
use compnode_core::geom::vec2;
use compnode_core::{Rect, Vec2, Viewport};

pub const CANVAS_SIZE: u32 = 2048;

#[derive(Clone, Copy, Debug)]
pub struct CanvasView {
    /// Canvas pixels are the "graph" space of this viewport.
    pub view: Viewport,
    pub canvas: (u32, u32),
}

impl CanvasView {
    pub fn new(screen: Vec2) -> CanvasView {
        let mut v = CanvasView {
            view: Viewport::new(screen),
            canvas: (CANVAS_SIZE, CANVAS_SIZE),
        };
        v.fit();
        v
    }
    pub fn set_screen_size(&mut self, size: Vec2) {
        self.view.size = size;
    }
    pub fn set_canvas(&mut self, w: u32, h: u32) {
        self.canvas = (w.max(1), h.max(1));
    }
    fn canvas_rect(&self) -> Rect {
        Rect::new(Vec2::ZERO, vec2(self.canvas.0 as f32, self.canvas.1 as f32))
    }
    /// Show the whole canvas with a small margin.
    pub fn fit(&mut self) {
        self.view = Viewport::fit(self.view.size, self.canvas_rect(), 12.0);
    }
    /// One canvas pixel per screen pixel, centered.
    pub fn one_to_one(&mut self) {
        let c = self.canvas_rect().center();
        self.view.zoom = 1.0;
        self.view.origin = c - self.view.size * 0.5;
    }
    pub fn zoom_at(&mut self, screen_pt: Vec2, factor: f32) {
        self.view.zoom_at(screen_pt, factor);
    }
    pub fn pan(&mut self, screen_delta: Vec2) {
        self.view.pan(screen_delta);
    }
    /// Where the canvas lands on screen (for the host to draw the texture).
    pub fn screen_rect(&self) -> Rect {
        let r = self.canvas_rect();
        Rect::new(self.view.to_screen(r.min), self.view.to_screen(r.max))
    }
    /// Canvas pixel under a screen point, if inside the canvas.
    pub fn pixel_at(&self, screen_pt: Vec2) -> Option<(u32, u32)> {
        let p = self.view.to_graph(screen_pt);
        if self.canvas_rect().contains(p)
            && p.x < self.canvas.0 as f32
            && p.y < self.canvas.1 as f32
        {
            Some((p.x as u32, p.y as u32))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_shows_the_whole_canvas_centered() {
        let v = CanvasView::new(vec2(1000.0, 800.0));
        let r = v.screen_rect();
        assert!(r.min.x >= 0.0 && r.min.y >= 0.0 && r.max.x <= 1000.0 && r.max.y <= 800.0);
        assert!((r.center() - vec2(500.0, 400.0)).length() < 1.0);
        assert!(v.view.zoom < 0.4);
    }

    #[test]
    fn one_to_one_maps_pixels_exactly() {
        let mut v = CanvasView::new(vec2(1000.0, 800.0));
        v.one_to_one();
        assert_eq!(v.view.zoom, 1.0);
        assert_eq!(v.pixel_at(vec2(500.0, 400.0)), Some((1024, 1024)));
        v.pan(vec2(-10.0, 0.0)); // drag left: image moves left, view moves right
        assert_eq!(v.pixel_at(vec2(500.0, 400.0)), Some((1034, 1024)));
    }

    #[test]
    fn pixel_at_outside_is_none_and_zoom_keeps_anchor() {
        let mut v = CanvasView::new(vec2(1000.0, 800.0));
        assert_eq!(v.pixel_at(vec2(1.0, 1.0)), None);
        let p = vec2(500.0, 400.0);
        let before = v.pixel_at(p);
        v.zoom_at(p, 2.0);
        let after = v.pixel_at(p).unwrap();
        let b = before.unwrap();
        assert!(
            (after.0 as i64 - b.0 as i64).abs() <= 1 && (after.1 as i64 - b.1 as i64).abs() <= 1
        );
        v.set_canvas(100, 50);
        v.fit();
        assert!(v.screen_rect().width() > v.screen_rect().height());
    }
}
