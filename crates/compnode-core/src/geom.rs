//! Minimal 2D geometry: [`Vec2`], [`Rect`] and the affine transform [`Affine2`].
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

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
    pub const ONE: Vec2 = vec2(1.0, 1.0);

    #[inline]
    pub const fn splat(v: f32) -> Vec2 {
        vec2(v, v)
    }
    #[inline]
    pub fn length(self) -> f32 {
        self.x.hypot(self.y)
    }
    #[inline]
    pub fn length_sq(self) -> f32 {
        self.dot(self)
    }
    #[inline]
    pub fn dot(self, o: Vec2) -> f32 {
        self.x * o.x + self.y * o.y
    }
    #[inline]
    pub fn distance(self, o: Vec2) -> f32 {
        (self - o).length()
    }
    /// Component-wise minimum.
    #[inline]
    pub fn min(self, o: Vec2) -> Vec2 {
        vec2(self.x.min(o.x), self.y.min(o.y))
    }
    /// Component-wise maximum.
    #[inline]
    pub fn max(self, o: Vec2) -> Vec2 {
        vec2(self.x.max(o.x), self.y.max(o.y))
    }
    /// Linear interpolation: `self` at `t = 0`, `o` at `t = 1`.
    #[inline]
    pub fn lerp(self, o: Vec2, t: f32) -> Vec2 {
        self + (o - self) * t
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
impl Neg for Vec2 {
    type Output = Vec2;
    #[inline]
    fn neg(self) -> Vec2 {
        vec2(-self.x, -self.y)
    }
}
impl AddAssign for Vec2 {
    #[inline]
    fn add_assign(&mut self, o: Vec2) {
        *self = *self + o;
    }
}
impl SubAssign for Vec2 {
    #[inline]
    fn sub_assign(&mut self, o: Vec2) {
        *self = *self - o;
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
    #[inline]
    pub fn from_min_size(min: Vec2, size: Vec2) -> Self {
        Rect::new(min, min + size)
    }
    #[inline]
    pub fn from_center_size(center: Vec2, size: Vec2) -> Self {
        let h = size * 0.5;
        Rect::new(center - h, center + h)
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
    /// Inclusive overlap test: rectangles that merely touch intersect.
    #[inline]
    pub fn intersects(&self, o: &Rect) -> bool {
        self.min.x <= o.max.x
            && o.min.x <= self.max.x
            && self.min.y <= o.max.y
            && o.min.y <= self.max.y
    }
    #[inline]
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }
    /// `true` if `o` lies entirely inside `self` (edges may coincide).
    #[inline]
    pub fn contains_rect(&self, o: &Rect) -> bool {
        o.min.x >= self.min.x
            && o.max.x <= self.max.x
            && o.min.y >= self.min.y
            && o.max.y <= self.max.y
    }
    #[inline]
    pub fn expand(&self, m: f32) -> Rect {
        Rect::new(
            vec2(self.min.x - m, self.min.y - m),
            vec2(self.max.x + m, self.max.y + m),
        )
    }
    #[inline]
    pub fn translate(&self, d: Vec2) -> Rect {
        Rect::new(self.min + d, self.max + d)
    }
    #[inline]
    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }
    #[inline]
    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }
    #[inline]
    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }
    #[inline]
    pub fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }
    #[inline]
    pub fn area(&self) -> f32 {
        self.width() * self.height()
    }
    /// Smallest rectangle containing both.
    #[inline]
    pub fn union(&self, o: &Rect) -> Rect {
        Rect::new(self.min.min(o.min), self.max.max(o.max))
    }
    /// Overlapping region, or `None` if the rectangles are disjoint.
    /// Agrees with [`Rect::intersects`]: touching rectangles give a
    /// zero-area result.
    #[inline]
    pub fn intersection(&self, o: &Rect) -> Option<Rect> {
        let min = self.min.max(o.min);
        let max = self.max.min(o.max);
        if min.x <= max.x && min.y <= max.y {
            Some(Rect::new(min, max))
        } else {
            None
        }
    }
}

/// 2D affine transform (a 2x3 matrix):
///
/// ```text
/// x' = a*x + c*y + e
/// y' = b*x + d*y + f
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine2 {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Default for Affine2 {
    fn default() -> Self {
        Affine2::IDENTITY
    }
}

impl Affine2 {
    pub const IDENTITY: Affine2 = Affine2 {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    #[inline]
    pub const fn translation(t: Vec2) -> Affine2 {
        Affine2 {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: t.x,
            f: t.y,
        }
    }
    #[inline]
    pub const fn scale(s: f32) -> Affine2 {
        Affine2::scale_xy(vec2(s, s))
    }
    #[inline]
    pub const fn scale_xy(s: Vec2) -> Affine2 {
        Affine2 {
            a: s.x,
            b: 0.0,
            c: 0.0,
            d: s.y,
            e: 0.0,
            f: 0.0,
        }
    }
    /// Counter-clockwise rotation (in a y-up frame) by `rad` radians about the origin.
    #[inline]
    pub fn rotation(rad: f32) -> Affine2 {
        let (sin, cos) = rad.sin_cos();
        Affine2 {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Compose: apply `self` first, then `next`.
    #[inline]
    pub fn then(self, next: Affine2) -> Affine2 {
        let (s, n) = (self, next);
        Affine2 {
            a: n.a * s.a + n.c * s.b,
            b: n.b * s.a + n.d * s.b,
            c: n.a * s.c + n.c * s.d,
            d: n.b * s.c + n.d * s.d,
            e: n.a * s.e + n.c * s.f + n.e,
            f: n.b * s.e + n.d * s.f + n.f,
        }
    }

    #[inline]
    pub fn transform_point(&self, p: Vec2) -> Vec2 {
        vec2(
            self.a * p.x + self.c * p.y + self.e,
            self.b * p.x + self.d * p.y + self.f,
        )
    }
    /// Transform a direction: like [`Affine2::transform_point`] without the translation.
    #[inline]
    pub fn transform_vector(&self, v: Vec2) -> Vec2 {
        vec2(self.a * v.x + self.c * v.y, self.b * v.x + self.d * v.y)
    }
    /// Axis-aligned bounds of the transformed rectangle.
    pub fn transform_rect(&self, r: &Rect) -> Rect {
        let corners = [
            self.transform_point(r.min),
            self.transform_point(vec2(r.max.x, r.min.y)),
            self.transform_point(r.max),
            self.transform_point(vec2(r.min.x, r.max.y)),
        ];
        Rect::from_points(&corners)
    }

    #[inline]
    pub fn determinant(&self) -> f32 {
        self.a * self.d - self.b * self.c
    }

    /// Inverse transform, or `None` if the matrix is (numerically) singular.
    pub fn inverse(&self) -> Option<Affine2> {
        let det = self.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        let (a, b, c, d) = (self.d * inv, -self.b * inv, -self.c * inv, self.a * inv);
        Some(Affine2 {
            a,
            b,
            c,
            d,
            e: -(a * self.e + c * self.f),
            f: -(b * self.e + d * self.f),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn vec2_helpers() {
        let a = vec2(1.0, 5.0);
        let b = vec2(3.0, 2.0);
        assert_eq!(a.min(b), vec2(1.0, 2.0));
        assert_eq!(a.max(b), vec2(3.0, 5.0));
        assert_eq!(a.lerp(b, 0.5), vec2(2.0, 3.5));
        assert_eq!(-a, vec2(-1.0, -5.0));
        assert_eq!(Vec2::splat(2.0), vec2(2.0, 2.0));
        assert_eq!(vec2(3.0, 4.0).length_sq(), 25.0);
        assert_eq!(vec2(0.0, 0.0).distance(vec2(3.0, 4.0)), 5.0);
        let mut c = a;
        c += b;
        c -= vec2(1.0, 1.0);
        assert_eq!(c, vec2(3.0, 6.0));
    }

    #[test]
    fn rect_union_intersection_center() {
        let a = Rect::new(vec2(0.0, 0.0), vec2(10.0, 10.0));
        let b = Rect::new(vec2(5.0, 5.0), vec2(20.0, 8.0));
        assert_eq!(a.union(&b), Rect::new(vec2(0.0, 0.0), vec2(20.0, 10.0)));
        assert_eq!(
            a.intersection(&b),
            Some(Rect::new(vec2(5.0, 5.0), vec2(10.0, 8.0)))
        );
        assert_eq!(a.center(), vec2(5.0, 5.0));
        assert_eq!((a.width(), a.height(), a.area()), (10.0, 10.0, 100.0));

        let far = Rect::new(vec2(50.0, 50.0), vec2(60.0, 60.0));
        assert_eq!(a.intersection(&far), None);
        assert_eq!(a.intersects(&far), a.intersection(&far).is_some());
        assert_eq!(a.intersects(&b), a.intersection(&b).is_some());

        // touching edges: both agree they intersect (zero-width result)
        let touch = Rect::new(vec2(10.0, 0.0), vec2(15.0, 10.0));
        assert!(a.intersects(&touch));
        assert_eq!(a.intersection(&touch).map(|r| r.width()), Some(0.0));

        assert!(a.contains_rect(&Rect::new(vec2(1.0, 1.0), vec2(9.0, 9.0))));
        assert!(a.contains_rect(&a));
        assert!(!a.contains_rect(&b));
    }

    #[test]
    fn rect_constructors_and_translate() {
        let r = Rect::from_min_size(vec2(2.0, 3.0), vec2(4.0, 6.0));
        assert_eq!(r, Rect::new(vec2(2.0, 3.0), vec2(6.0, 9.0)));
        assert_eq!(
            Rect::from_center_size(vec2(4.0, 6.0), vec2(4.0, 6.0)),
            Rect::new(vec2(2.0, 3.0), vec2(6.0, 9.0))
        );
        assert_eq!(
            r.translate(vec2(1.0, -1.0)),
            Rect::new(vec2(3.0, 2.0), vec2(7.0, 8.0))
        );
    }

    #[test]
    fn affine_compose_and_inverse() {
        let t = Affine2::translation(vec2(10.0, -5.0));
        let s = Affine2::scale(2.0);
        // scale first, then translate
        let m = s.then(t);
        assert!(close(m.transform_point(vec2(3.0, 4.0)), vec2(16.0, 3.0)));
        assert!(close(m.transform_vector(vec2(3.0, 4.0)), vec2(6.0, 8.0)));

        let inv = m.inverse().expect("invertible");
        assert!(close(inv.transform_point(vec2(16.0, 3.0)), vec2(3.0, 4.0)));
        let id = m.then(inv);
        for p in [vec2(0.0, 0.0), vec2(7.0, -3.0), vec2(-100.0, 42.0)] {
            assert!(close(id.transform_point(p), p));
        }

        let r = Affine2::rotation(std::f32::consts::FRAC_PI_2);
        assert!(close(r.transform_point(vec2(1.0, 0.0)), vec2(0.0, 1.0)));
        assert!(Affine2::scale(0.0).inverse().is_none());
        assert_eq!(Affine2::default(), Affine2::IDENTITY);
    }

    #[test]
    fn affine_transform_rect_gives_bounds() {
        let r = Rect::new(vec2(0.0, 0.0), vec2(2.0, 1.0));
        let rot = Affine2::rotation(std::f32::consts::FRAC_PI_2).transform_rect(&r);
        assert!(close(rot.min, vec2(-1.0, 0.0)));
        assert!(close(rot.max, vec2(0.0, 2.0)));
    }
}
