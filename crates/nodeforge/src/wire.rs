//! Wire geometry: Blender-style horizontal-tangent cubic beziers, adaptive
//! flattening by screen tolerance, hit testing and a per-wire polyline cache.
use crate::arena::Key;
use crate::geom::{vec2, Vec2};
use crate::hash::FxMap;

/// Control points of the wire from output pin `a` to input pin `b`.
#[inline]
pub fn control_points(a: Vec2, b: Vec2) -> [Vec2; 4] {
    let h = ((b.x - a.x).abs() * 0.5).max(30.0) + 10.0;
    [a, vec2(a.x + h, a.y), vec2(b.x - h, b.y), b]
}

#[inline]
pub fn eval(p: &[Vec2; 4], t: f32) -> Vec2 {
    let mt = 1.0 - t;
    let (a, b, c, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
    vec2(
        a * p[0].x + b * p[1].x + c * p[2].x + d * p[3].x,
        a * p[0].y + b * p[1].y + c * p[2].y + d * p[3].y,
    )
}

/// Flatten into a polyline whose deviation from the true curve is <= `tol`
/// (graph units). Uses the second-difference error bound
/// `err <= 0.75 * dd / n^2`, so the segment count is computed up-front with
/// no recursion.
pub fn flatten(p: &[Vec2; 4], tol: f32, out: &mut Vec<Vec2>) {
    // Convex-hull property: if both inner control points lie within `tol` of
    // the chord, the whole curve does. Very common for same-height pins.
    if dist_to_segment(p[1], p[0], p[3]) <= tol && dist_to_segment(p[2], p[0], p[3]) <= tol {
        out.push(p[0]);
        out.push(p[3]);
        return;
    }
    let d1 = p[0] - p[1] * 2.0 + p[2];
    let d2 = p[1] - p[2] * 2.0 + p[3];
    let dd = d1.length().max(d2.length());
    let n = ((0.75 * dd / tol.max(1e-4)).sqrt().ceil() as usize).clamp(1, 256);
    out.push(p[0]);
    for i in 1..n {
        out.push(eval(p, i as f32 / n as f32));
    }
    out.push(p[3]);
}

pub fn dist_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len2 = ab.dot(ab);
    let t = if len2 <= f32::EPSILON { 0.0 } else { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) };
    (p - (a + ab * t)).length()
}

/// Minimum distance from `p` to a polyline.
pub fn dist_to_polyline(p: Vec2, pts: &[Vec2]) -> f32 {
    pts.windows(2).map(|w| dist_to_segment(p, w[0], w[1])).fold(f32::INFINITY, f32::min)
}

struct Entry {
    a: Vec2,
    b: Vec2,
    tol: f32,
    frame: u32,
    valid: bool,
    pts: Vec<Vec2>,
}

/// Caches flattened wires. An entry is reused while its endpoints and
/// (quantized) tolerance are unchanged; entries not touched during a frame are
/// dropped by [`WireCache::end_frame`], so the cache never outgrows the
/// visible set. Polyline buffers are reused, avoiding per-frame allocation.
#[derive(Default)]
pub struct WireCache {
    map: FxMap<Key, Entry>,
    frame: u32,
}

impl WireCache {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    pub fn end_frame(&mut self) {
        let f = self.frame;
        self.map.retain(|_, e| e.frame == f);
    }

    /// Flattened polyline for wire `id` between `a` and `b` at tolerance `tol`.
    /// Tolerance is quantized down to a power of two so small zoom changes
    /// do not invalidate the cache.
    pub fn get(&mut self, id: Key, a: Vec2, b: Vec2, tol: f32) -> &[Vec2] {
        let tol_q = tol.max(1e-3).log2().floor().exp2();
        let frame = self.frame;
        let e = self.map.entry(id).or_insert_with(|| Entry {
            a,
            b,
            tol: 0.0,
            frame,
            valid: false,
            pts: Vec::new(),
        });
        e.frame = frame;
        if !e.valid || e.a != a || e.b != b || e.tol != tol_q {
            e.a = a;
            e.b = b;
            e.tol = tol_q;
            e.valid = true;
            e.pts.clear();
            flatten(&control_points(a, b), tol_q, &mut e.pts);
        }
        &e.pts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatten_stays_within_tolerance() {
        let p = control_points(vec2(0.0, 0.0), vec2(300.0, 200.0));
        for tol in [2.0_f32, 0.5, 0.1] {
            let mut pts = Vec::new();
            flatten(&p, tol, &mut pts);
            let worst = (0..=2000)
                .map(|i| dist_to_polyline(eval(&p, i as f32 / 2000.0), &pts))
                .fold(0.0_f32, f32::max);
            assert!(worst <= tol * 1.05, "tol {tol}: worst {worst}");
        }
    }

    #[test]
    fn straight_wire_uses_few_segments() {
        let p = control_points(vec2(0.0, 0.0), vec2(300.0, 0.0));
        let mut pts = Vec::new();
        flatten(&p, 0.5, &mut pts);
        assert!(pts.len() <= 3, "{}", pts.len());
    }

    #[test]
    fn cache_reuses_and_invalidates() {
        let mut c = WireCache::new();
        let id = Key { index: 0, generation: 0 };
        c.begin_frame();
        let n1 = c.get(id, vec2(0.0, 0.0), vec2(100.0, 50.0), 0.5).len();
        let ptr1 = c.get(id, vec2(0.0, 0.0), vec2(100.0, 50.0), 0.5).as_ptr();
        let ptr2 = c.get(id, vec2(0.0, 0.0), vec2(100.0, 50.0), 0.5).as_ptr();
        assert_eq!(ptr1, ptr2);
        let n2 = c.get(id, vec2(0.0, 0.0), vec2(100.0, 50.0), 0.001).len();
        assert!(n2 > n1);
        c.end_frame();
        assert_eq!(c.len(), 1);
        c.begin_frame();
        c.end_frame(); // untouched -> evicted
        assert!(c.is_empty());
    }
}
