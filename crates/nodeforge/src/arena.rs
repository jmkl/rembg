//! Generational arena: O(1) insert/remove/lookup, slots are reused, and stale
//! keys are detected (no ABA bugs).

/// Stable id into an [`Arena`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Key {
    pub index: u32,
    pub generation: u32,
}

struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

pub struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    len: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Arena { slots: Vec::new(), free: Vec::new(), len: 0 }
    }
}

impl<T> Arena<T> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_capacity(n: usize) -> Self {
        Arena { slots: Vec::with_capacity(n), free: Vec::new(), len: 0 }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn insert(&mut self, value: T) -> Key {
        self.len += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.value = Some(value);
            Key { index, generation: slot.generation }
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(Slot { generation: 0, value: Some(value) });
            Key { index, generation: 0 }
        }
    }

    pub fn remove(&mut self, key: Key) -> Option<T> {
        let slot = self.slots.get_mut(key.index as usize)?;
        if slot.generation != key.generation {
            return None;
        }
        let value = slot.value.take()?;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(key.index);
        self.len -= 1;
        Some(value)
    }

    #[inline]
    pub fn get(&self, key: Key) -> Option<&T> {
        let slot = self.slots.get(key.index as usize)?;
        if slot.generation == key.generation { slot.value.as_ref() } else { None }
    }

    #[inline]
    pub fn get_mut(&mut self, key: Key) -> Option<&mut T> {
        let slot = self.slots.get_mut(key.index as usize)?;
        if slot.generation == key.generation { slot.value.as_mut() } else { None }
    }

    pub fn contains(&self, key: Key) -> bool {
        self.get(key).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = (Key, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.value.as_ref().map(|v| (Key { index: i as u32, generation: s.generation }, v))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_keys_are_rejected_after_slot_reuse() {
        let mut a = Arena::new();
        let k1 = a.insert("a");
        assert_eq!(a.remove(k1), Some("a"));
        let k2 = a.insert("b");
        assert_eq!(k1.index, k2.index);
        assert_ne!(k1, k2);
        assert!(a.get(k1).is_none());
        assert_eq!(a.get(k2), Some(&"b"));
        assert_eq!(a.remove(k1), None);
        assert_eq!(a.len(), 1);
    }
}
