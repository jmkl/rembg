//! Node selection by uid.
use crate::document::Document;
use compnode_core::hash::FxMap;

#[derive(Default, Clone, Debug)]
pub struct Selection {
    set: FxMap<u64, ()>,
    /// The "active" node (last clicked); drawn with the active outline.
    pub active: Option<u64>,
}

impl Selection {
    pub fn contains(&self, uid: u64) -> bool {
        self.set.contains_key(&uid)
    }
    pub fn len(&self) -> usize {
        self.set.len()
    }
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }
    pub fn clear(&mut self) {
        self.set.clear();
        self.active = None;
    }
    pub fn add(&mut self, uid: u64) {
        self.set.insert(uid, ());
        self.active = Some(uid);
    }
    pub fn remove(&mut self, uid: u64) {
        self.set.remove(&uid);
        if self.active == Some(uid) {
            self.active = None;
        }
    }
    pub fn toggle(&mut self, uid: u64) {
        if self.contains(uid) {
            self.remove(uid);
        } else {
            self.add(uid);
        }
    }
    pub fn set_only(&mut self, uid: u64) {
        self.set.clear();
        self.add(uid);
    }
    pub fn set_all(&mut self, uids: &[u64]) {
        self.set.clear();
        for &u in uids {
            self.set.insert(u, ());
        }
        self.active = uids.last().copied();
    }
    /// Sorted (deterministic).
    pub fn to_vec(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.set.keys().copied().collect();
        v.sort_unstable();
        v
    }
    pub fn select_all(&mut self, doc: &Document) {
        let all = doc.uids();
        self.set_all(&all);
    }
    pub fn invert(&mut self, doc: &Document) {
        let inv: Vec<u64> = doc
            .uids()
            .into_iter()
            .filter(|u| !self.contains(*u))
            .collect();
        self.set_all(&inv);
    }
    /// Forget nodes that no longer exist.
    pub fn retain_existing(&mut self, doc: &Document) {
        self.set.retain(|u, _| doc.contains(*u));
        if let Some(a) = self.active
            && !doc.contains(a)
        {
            self.active = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::test_support::registry;
    use compnode_core::geom::vec2;

    #[test]
    fn selection_operations() {
        let mut d = Document::new(registry());
        let ids: Vec<u64> = (0..4)
            .map(|i| d.add_node("t.src", vec2(i as f32 * 300.0, 0.0)).unwrap())
            .collect();
        let mut s = Selection::default();
        s.add(ids[0]);
        s.toggle(ids[1]);
        assert_eq!(s.to_vec(), vec![ids[0], ids[1]]);
        assert_eq!(s.active, Some(ids[1]));
        s.toggle(ids[1]);
        assert_eq!(s.active, None);
        s.invert(&d);
        assert_eq!(s.to_vec(), vec![ids[1], ids[2], ids[3]]);
        s.select_all(&d);
        assert_eq!(s.len(), 4);
        s.set_only(ids[2]);
        assert_eq!(s.to_vec(), vec![ids[2]]);
        d.remove_node(ids[2]);
        s.retain_existing(&d);
        assert!(s.is_empty() && s.active.is_none());
    }
}
