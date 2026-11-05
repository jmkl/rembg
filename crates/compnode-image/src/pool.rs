//! Backend-independent texture pool: reuses textures by size/format, tracks
//! memory against a soft budget and evicts the least recently released free
//! texture first. The GPU backend instantiates it with its own texture type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TexKey {
    pub width: u32,
    pub height: u32,
    /// Backend-defined format id.
    pub format: u32,
    pub bytes_per_pixel: u32,
}

impl TexKey {
    /// RGBA 16-bit float texture, the working format of the node graph.
    pub fn rgba16f(width: u32, height: u32) -> TexKey {
        TexKey {
            width,
            height,
            format: 0,
            bytes_per_pixel: 8,
        }
    }
    pub fn bytes(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * u64::from(self.bytes_per_pixel)
    }
}

struct Free<T> {
    key: TexKey,
    tex: T,
    released: u64,
}

pub struct TexturePool<T> {
    free: Vec<Free<T>>,
    allocated: u64,
    budget: u64,
    clock: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl<T> TexturePool<T> {
    pub fn new(budget_bytes: u64) -> TexturePool<T> {
        TexturePool {
            free: Vec::new(),
            allocated: 0,
            budget: budget_bytes,
            clock: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    pub fn set_budget(&mut self, bytes: u64) {
        self.budget = bytes;
        self.evict_to(self.budget);
    }

    /// Bytes of every live texture: in use plus pooled.
    pub fn allocated(&self) -> u64 {
        self.allocated
    }
    pub fn free_count(&self) -> usize {
        self.free.len()
    }
    pub fn hits(&self) -> u64 {
        self.hits
    }
    pub fn misses(&self) -> u64 {
        self.misses
    }
    pub fn evictions(&self) -> u64 {
        self.evictions
    }

    fn evict_one(&mut self) -> bool {
        let Some(i) = self
            .free
            .iter()
            .enumerate()
            .min_by_key(|(_, f)| f.released)
            .map(|(i, _)| i)
        else {
            return false;
        };
        let f = self.free.swap_remove(i);
        self.allocated -= f.key.bytes();
        self.evictions += 1;
        true
    }

    fn evict_to(&mut self, target: u64) {
        while self.allocated > target && self.evict_one() {}
    }

    /// A texture for `key`: a pooled one if available, otherwise `create()`.
    /// Pooled textures are evicted first if the new one would exceed the
    /// budget (textures in use are never taken away, so the budget is soft).
    pub fn acquire(&mut self, key: TexKey, create: impl FnOnce() -> T) -> T {
        if let Some(i) = self.free.iter().position(|f| f.key == key) {
            self.hits += 1;
            return self.free.swap_remove(i).tex;
        }
        self.misses += 1;
        self.evict_to(self.budget.saturating_sub(key.bytes()));
        self.allocated += key.bytes();
        create()
    }

    /// Return a texture to the pool.
    pub fn release(&mut self, key: TexKey, tex: T) {
        self.clock += 1;
        self.free.push(Free {
            key,
            tex,
            released: self.clock,
        });
        self.evict_to(self.budget);
    }

    /// A texture was destroyed outside the pool.
    pub fn forget(&mut self, key: TexKey) {
        self.allocated = self.allocated.saturating_sub(key.bytes());
    }

    /// Drop every pooled texture.
    pub fn trim(&mut self) {
        while self.evict_one() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_match_the_plan_numbers() {
        // 2048 x 2048 RGBA16F is 32 MiB; fifty cached results are 1.6 GiB
        let k = TexKey::rgba16f(2048, 2048);
        assert_eq!(k.bytes(), 33_554_432);
        assert_eq!(50 * k.bytes(), 1_677_721_600);
    }

    #[test]
    fn reuses_by_key_and_counts() {
        let mut p: TexturePool<u32> = TexturePool::new(u64::MAX);
        let k = TexKey::rgba16f(64, 64);
        let a = p.acquire(k, || 1);
        assert_eq!((p.hits(), p.misses(), p.allocated()), (0, 1, k.bytes()));
        p.release(k, a);
        assert_eq!(p.free_count(), 1);
        assert_eq!(p.acquire(k, || 2), 1); // the pooled texture, not a new one
        assert_eq!((p.hits(), p.misses()), (1, 1));
        // different size is a different texture
        let k2 = TexKey::rgba16f(64, 32);
        p.release(k, 1);
        assert_eq!(p.acquire(k2, || 3), 3);
        assert_eq!(p.allocated(), k.bytes() + k2.bytes());
    }

    #[test]
    fn evicts_least_recently_released_when_over_budget() {
        let k = TexKey::rgba16f(16, 16); // 2048 bytes
        let mut p: TexturePool<&str> = TexturePool::new(2 * k.bytes());
        let (a, b, c) = (
            p.acquire(k, || "a"),
            p.acquire(k, || "b"),
            p.acquire(k, || "c"),
        );
        assert_eq!(p.allocated(), 3 * k.bytes()); // everything in use: soft budget
        p.release(k, a);
        p.release(k, b);
        p.release(k, c);
        assert_eq!(p.free_count(), 2);
        assert_eq!(p.evictions(), 1);
        assert_eq!(p.allocated(), 2 * k.bytes());
        // "a" went first; "b" and "c" remain
        let first = p.acquire(k, || "new");
        let second = p.acquire(k, || "new");
        let mut got = [first, second];
        got.sort_unstable();
        assert_eq!(got, ["b", "c"]);
    }

    #[test]
    fn new_allocations_make_room_by_evicting_free_textures() {
        let small = TexKey::rgba16f(16, 16);
        let big = TexKey::rgba16f(32, 32); // 4x the bytes
        let mut p: TexturePool<u8> = TexturePool::new(big.bytes());
        let t = p.acquire(small, || 0);
        p.release(small, t);
        assert_eq!(p.free_count(), 1);
        let _b = p.acquire(big, || 1); // needs the whole budget
        assert_eq!(p.free_count(), 0);
        assert_eq!(p.allocated(), big.bytes());
        p.forget(big);
        assert_eq!(p.allocated(), 0);
    }

    #[test]
    fn trim_and_budget_changes() {
        let k = TexKey::rgba16f(16, 16);
        let mut p: TexturePool<u8> = TexturePool::new(u64::MAX);
        for i in 0..4 {
            let t = p.acquire(k, || i);
            p.release(k, t);
        }
        // the same texture was reused every time
        assert_eq!(p.free_count(), 1);
        let extra: Vec<u8> = (0..3)
            .map(|i| p.acquire(TexKey::rgba16f(8, 8), || i))
            .collect();
        for e in extra {
            p.release(TexKey::rgba16f(8, 8), e);
        }
        assert_eq!(p.free_count(), 4);
        p.set_budget(k.bytes());
        assert!(p.allocated() <= k.bytes() + 1);
        p.trim();
        assert_eq!((p.free_count(), p.allocated()), (0, 0));
    }
}
