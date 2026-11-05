//! Background thumbnails for Image File nodes: request a path, poll for the
//! result. Only paths that are asked for are decoded, so the host asks for
//! the nodes that are on screen.
use crate::buffer::ImageBuf;
use crate::loader::{AsyncLoader, ImageLoader};
use compnode_core::hash::FxMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub enum ThumbState {
    Pending,
    Ready(Arc<ImageBuf>),
    Failed(String),
}

pub struct ThumbCache {
    loader: AsyncLoader,
    state: FxMap<String, ThumbState>,
    ids: FxMap<u64, String>,
    next_id: u64,
}

impl ThumbCache {
    /// `max_dim` is the longer side of the thumbnails; `threads` decode in parallel.
    pub fn new(loader: Arc<dyn ImageLoader>, threads: usize, max_dim: u32) -> ThumbCache {
        ThumbCache {
            loader: AsyncLoader::new(loader, threads, Some((max_dim, max_dim))),
            state: FxMap::default(),
            ids: FxMap::default(),
            next_id: 1,
        }
    }

    /// Ask for a thumbnail. Does nothing if it was requested before.
    pub fn request(&mut self, path: &str) {
        if path.trim().is_empty() || self.state.contains_key(path) {
            return;
        }
        self.state.insert(path.to_string(), ThumbState::Pending);
        let id = self.next_id;
        self.next_id += 1;
        self.ids.insert(id, path.to_string());
        self.loader.submit(id, path);
    }

    pub fn get(&self, path: &str) -> Option<&ThumbState> {
        self.state.get(path)
    }

    /// Number of requests still being decoded.
    pub fn pending(&self) -> usize {
        self.state
            .values()
            .filter(|s| matches!(s, ThumbState::Pending))
            .count()
    }

    /// Collect finished loads (never blocks). Returns the paths that changed.
    pub fn poll(&mut self) -> Vec<String> {
        let mut changed = Vec::new();
        for r in self.loader.poll() {
            let Some(path) = self.ids.remove(&r.id) else {
                continue;
            };
            if !matches!(self.state.get(&path), Some(ThumbState::Pending)) {
                continue; // forgotten while loading
            }
            let st = match r.result {
                Ok(img) => ThumbState::Ready(Arc::new(img)),
                Err(e) => ThumbState::Failed(e),
            };
            self.state.insert(path.clone(), st);
            changed.push(path);
        }
        changed
    }

    /// Poll until something arrives or `timeout` passes (tests, shutdown).
    pub fn wait(&mut self, timeout: Duration) -> Vec<String> {
        let end = Instant::now() + timeout;
        loop {
            let c = self.poll();
            if !c.is_empty() || Instant::now() >= end || self.pending() == 0 {
                return c;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Drop a path (the file changed); a later request decodes it again.
    pub fn forget(&mut self, path: &str) {
        self.state.remove(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::FnLoader;
    use compnode_core::Color;

    #[test]
    fn requests_are_deduplicated_decoded_in_the_background_and_downscaled() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let c2 = Arc::clone(&calls);
        let loader = FnLoader(move |p: &str| {
            c2.fetch_add(1, Ordering::SeqCst);
            if p == "bad" {
                Err("no such file".to_string())
            } else {
                Ok(ImageBuf::solid(800, 400, Color::WHITE))
            }
        });
        let mut t = ThumbCache::new(Arc::new(loader), 2, 64);
        t.request("a");
        t.request("a");
        t.request("bad");
        t.request("");
        assert_eq!(t.pending(), 2);
        let mut done = Vec::new();
        for _ in 0..200 {
            done.extend(t.wait(Duration::from_millis(50)));
            if done.len() == 2 {
                break;
            }
        }
        done.sort();
        assert_eq!(done, vec!["a".to_string(), "bad".to_string()]);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        match t.get("a") {
            Some(ThumbState::Ready(img)) => assert_eq!((img.width, img.height), (64, 32)),
            other => panic!("{other:?}"),
        }
        assert!(matches!(t.get("bad"), Some(ThumbState::Failed(m)) if m.contains("no such")));
        // asking again does not decode again; forgetting does
        t.request("a");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        t.forget("a");
        t.request("a");
        assert!(!t.wait(Duration::from_secs(5)).is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }
}
