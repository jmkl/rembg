//! Incremental uniform spatial grid. Items are rectangles; queries return
//! candidate ids (callers do exact tests). Updates that stay within the same
//! cell range are free, which makes dragging cheap.
use crate::geom::Rect;
use crate::hash::FxMap;
use std::hash::Hash;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct CellRange {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl CellRange {
    fn cell_count(&self) -> u64 {
        ((self.x1 - self.x0 + 1) as u64) * ((self.y1 - self.y0 + 1) as u64)
    }
    fn intersects(&self, cx: i32, cy: i32) -> bool {
        cx >= self.x0 && cx <= self.x1 && cy >= self.y0 && cy <= self.y1
    }
}

pub struct Grid<K> {
    inv: f32,
    cells: FxMap<(i32, i32), Vec<K>>,
    ranges: FxMap<K, CellRange>,
}

impl<K: Copy + Eq + Hash + Ord> Grid<K> {
    pub fn new(cell_size: f32) -> Self {
        Grid { inv: 1.0 / cell_size, cells: FxMap::default(), ranges: FxMap::default() }
    }

    pub fn len(&self) -> usize {
        self.ranges.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    #[inline]
    fn range_of(&self, r: &Rect) -> CellRange {
        CellRange {
            x0: (r.min.x * self.inv).floor() as i32,
            y0: (r.min.y * self.inv).floor() as i32,
            x1: (r.max.x * self.inv).floor() as i32,
            y1: (r.max.y * self.inv).floor() as i32,
        }
    }

    /// Insert or move an item.
    pub fn update(&mut self, key: K, rect: &Rect) {
        let new = self.range_of(rect);
        match self.ranges.get(&key) {
            Some(old) if *old == new => return,
            Some(_) => self.remove(key),
            None => {}
        }
        for cy in new.y0..=new.y1 {
            for cx in new.x0..=new.x1 {
                self.cells.entry((cx, cy)).or_default().push(key);
            }
        }
        self.ranges.insert(key, new);
    }

    pub fn remove(&mut self, key: K) {
        let Some(old) = self.ranges.remove(&key) else { return };
        for cy in old.y0..=old.y1 {
            for cx in old.x0..=old.x1 {
                if let Some(v) = self.cells.get_mut(&(cx, cy)) {
                    if let Some(p) = v.iter().position(|k| *k == key) {
                        v.swap_remove(p);
                    }
                    if v.is_empty() {
                        self.cells.remove(&(cx, cy));
                    }
                }
            }
        }
    }

    /// Append unique candidate ids intersecting `rect` to `out`.
    pub fn query(&self, rect: &Rect, out: &mut Vec<K>) {
        let q = self.range_of(rect);
        let start = out.len();
        // When the query covers more cells than exist (zoomed far out), walk
        // the occupied cells instead of the whole rectangle.
        if q.cell_count() > self.cells.len() as u64 {
            for (&(cx, cy), v) in &self.cells {
                if q.intersects(cx, cy) {
                    out.extend_from_slice(v);
                }
            }
        } else {
            for cy in q.y0..=q.y1 {
                for cx in q.x0..=q.x1 {
                    if let Some(v) = self.cells.get(&(cx, cy)) {
                        out.extend_from_slice(v);
                    }
                }
            }
        }
        let tail = &mut out[start..];
        tail.sort_unstable();
        // dedup tail in place
        let mut w = 0;
        for r in 0..tail.len() {
            if r == 0 || tail[r] != tail[w - 1] {
                tail[w] = tail[r];
                w += 1;
            }
        }
        out.truncate(start + w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::vec2;

    fn r(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::new(vec2(x0, y0), vec2(x1, y1))
    }

    #[test]
    fn insert_query_move_remove() {
        let mut g: Grid<u32> = Grid::new(100.0);
        g.update(1, &r(10.0, 10.0, 50.0, 50.0));
        g.update(2, &r(500.0, 500.0, 650.0, 560.0)); // spans several cells
        let mut out = Vec::new();
        g.query(&r(0.0, 0.0, 200.0, 200.0), &mut out);
        assert_eq!(out, vec![1]);

        out.clear();
        g.query(&r(0.0, 0.0, 2000.0, 2000.0), &mut out);
        assert_eq!(out, vec![1, 2]); // multi-cell item deduped

        g.update(1, &r(900.0, 900.0, 920.0, 920.0));
        out.clear();
        g.query(&r(0.0, 0.0, 200.0, 200.0), &mut out);
        assert!(out.is_empty());

        g.remove(2);
        assert_eq!(g.len(), 1);
        out.clear();
        g.query(&r(400.0, 400.0, 700.0, 700.0), &mut out);
        assert!(out.is_empty());
    }
}
