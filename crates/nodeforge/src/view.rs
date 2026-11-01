//! Pan/zoom viewport and level-of-detail selection.
use crate::geom::{vec2, Rect, Vec2};

/// How much detail a renderer should draw at the current zoom.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lod {
    /// Everything: header, pin labels, body widgets.
    Full,
    /// Header and pins only; no body widgets or labels.
    Compact,
    /// Flat rounded rect; no text.
    Dot,
}

#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    /// Graph-space point shown at the top-left of the screen area.
    pub origin: Vec2,
    pub zoom: f32,
    /// Screen-area size in pixels.
    pub size: Vec2,
}

impl Viewport {
    pub const MIN_ZOOM: f32 = 0.02;
    pub const MAX_ZOOM: f32 = 4.0;

    pub fn new(size: Vec2) -> Self {
        Viewport { origin: Vec2::ZERO, zoom: 1.0, size }
    }

    #[inline]
    pub fn to_screen(&self, g: Vec2) -> Vec2 {
        (g - self.origin) * self.zoom
    }
    #[inline]
    pub fn to_graph(&self, s: Vec2) -> Vec2 {
        s * (1.0 / self.zoom) + self.origin
    }

    /// Visible graph-space rectangle, expanded by `margin_px` screen pixels so
    /// that things just outside the screen are ready when panning.
    pub fn visible_rect(&self, margin_px: f32) -> Rect {
        let m = margin_px / self.zoom;
        Rect::new(self.origin, self.origin + self.size * (1.0 / self.zoom)).expand(m)
    }

    /// Pan by a screen-space delta (dragging the canvas).
    pub fn pan(&mut self, screen_delta: Vec2) {
        self.origin = self.origin - screen_delta * (1.0 / self.zoom);
    }

    /// Zoom keeping the graph point under `screen_pt` fixed.
    pub fn zoom_at(&mut self, screen_pt: Vec2, factor: f32) {
        let anchor = self.to_graph(screen_pt);
        self.zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        self.origin = anchor - screen_pt * (1.0 / self.zoom);
    }

    pub fn lod(&self) -> Lod {
        if self.zoom >= 0.6 {
            Lod::Full
        } else if self.zoom >= 0.25 {
            Lod::Compact
        } else {
            Lod::Dot
        }
    }

    /// Flatten tolerance in graph units. Sub-pixel (0.35 px) when zoomed in;
    /// relaxed when zoomed out, where wires are hairlines and fine curvature
    /// is invisible.
    #[inline]
    pub fn wire_tolerance(&self) -> f32 {
        let px = match self.lod() {
            Lod::Full => 0.35,
            Lod::Compact => 0.75,
            Lod::Dot => 1.5,
        };
        px / self.zoom
    }

    /// Viewport that fits `rect` (with padding) into the screen area.
    pub fn fit(size: Vec2, rect: Rect, pad_px: f32) -> Self {
        let r = rect.size();
        let zx = (size.x - 2.0 * pad_px) / r.x.max(1.0);
        let zy = (size.y - 2.0 * pad_px) / r.y.max(1.0);
        let zoom = zx.min(zy).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        let center = vec2((rect.min.x + rect.max.x) * 0.5, (rect.min.y + rect.max.y) * 0.5);
        Viewport { origin: center - size * (0.5 / zoom), zoom, size }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_anchor_fixed() {
        let mut v = Viewport::new(vec2(800.0, 600.0));
        v.pan(vec2(-120.0, 40.0));
        let p = vec2(300.0, 200.0);
        let before = v.to_graph(p);
        v.zoom_at(p, 1.7);
        let after = v.to_graph(p);
        assert!((before - after).length() < 1e-3);
    }

    #[test]
    fn lod_steps_down_when_zooming_out() {
        let mut v = Viewport::new(vec2(800.0, 600.0));
        assert_eq!(v.lod(), Lod::Full);
        v.zoom = 0.4;
        assert_eq!(v.lod(), Lod::Compact);
        v.zoom = 0.1;
        assert_eq!(v.lod(), Lod::Dot);
    }
}
