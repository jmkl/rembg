//! Minimal 2D geometry.
use std::ops::{Add, Mul, Sub};

/// 2D vector / point (f32).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

#[inline]
pub const fn vec2(x: f32, y: f32) -> Vec2 {
    Vec2 { x, y }
}

impl Vec2 {
    pub const ZERO: Vec2 = vec2(0.0, 0.0);
    #[inline]
    pub fn length(self) -> f32 {
        self.x.hypot(self.y)
    }
    #[inline]
    pub fn dot(self, o: Vec2) -> f32 {
        self.x * o.x + self.y * o.y
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    #[inline]
    fn add(self, o: Vec2) -> Vec2 {
        vec2(self.x + o.x, self.y + o.y)
    }
}
impl Sub for Vec2 {
    type Output = Vec2;
    #[inline]
    fn sub(self, o: Vec2) -> Vec2 {
        vec2(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f32> for Vec2 {
    type Output = Vec2;
    #[inline]
    fn mul(self, s: f32) -> Vec2 {
        vec2(self.x * s, self.y * s)
    }
}

/// Axis-aligned rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    #[inline]
    pub fn new(min: Vec2, max: Vec2) -> Self {
        Rect { min, max }
    }
    pub fn from_points(pts: &[Vec2]) -> Self {
        let mut r = Rect::new(pts[0], pts[0]);
        for p in &pts[1..] {
            r.min.x = r.min.x.min(p.x);
            r.min.y = r.min.y.min(p.y);
            r.max.x = r.max.x.max(p.x);
            r.max.y = r.max.y.max(p.y);
        }
        r
    }
    #[inline]
    pub fn intersects(&self, o: &Rect) -> bool {
        self.min.x <= o.max.x && o.min.x <= self.max.x && self.min.y <= o.max.y && o.min.y <= self.max.y
    }
    #[inline]
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }
    #[inline]
    pub fn expand(&self, m: f32) -> Rect {
        Rect::new(vec2(self.min.x - m, self.min.y - m), vec2(self.max.x + m, self.max.y + m))
    }
    #[inline]
    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }
}
