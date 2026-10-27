//! Per-canvas cache of decoded layer images.
//!
//! The Output node sends `layers [filepath, ...]` to the canvas every frame.
//! The canvas asks this cache whether a path has already been converted to a
//! CPU image. If not, the file is decoded on a background thread, uploaded as
//! a texture and stored here, so connecting, disconnecting and reconnecting
//! the same node never decodes the file again (until the file changes on disk
//! or the entry is evicted by the memory budget).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, TextureHandle, TextureOptions};
use image::RgbaImage;

/// Budget for decoded CPU images held by one canvas.
const CPU_BUDGET: usize = 512 * 1024 * 1024;
/// Longest edge of the GPU texture. Layers are drawn scaled to the canvas, so
/// uploading the full bitmap of a huge photo is wasteful.
/// Increased to 4096 to accommodate RemoveBgV2 which removes the max-width clamp.
const TEXTURE_MAX_EDGE: u32 = 4096;
/// How often an entry's file is checked for modification.
const STALE_CHECK_EVERY: Duration = Duration::from_millis(750);

/// One converted layer image.
pub struct CachedImage {
    /// Full-resolution pixels, shared so export/inspection never copies them.
    pub cpu: Arc<RgbaImage>,
    pub texture: TextureHandle,
    /// Full-resolution size in pixels (texture may be smaller).
    pub dims: (u32, u32),
    mtime: Option<SystemTime>,
    last_check: Instant,
    last_used: u64,
}

struct Decoded {
    path: PathBuf,
    mtime: Option<SystemTime>,
    result: Result<RgbaImage, String>,
}

pub struct LayerCache {
    entries: HashMap<PathBuf, CachedImage>,
    pending: HashSet<PathBuf>,
    failed: HashMap<PathBuf, String>,
    tx: Sender<Decoded>,
    rx: Receiver<Decoded>,
    clock: u64,
}

impl Default for LayerCache {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            entries: HashMap::new(),
            pending: HashSet::new(),
            failed: HashMap::new(),
            tx,
            rx,
            clock: 0,
        }
    }
}

fn mtime_of(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn upload(ctx: &egui::Context, path: &Path, img: &RgbaImage) -> TextureHandle {
    let (w, h) = img.dimensions();
    let longest = w.max(h);
    let name = format!("layer:{}", path.display());
    if longest <= TEXTURE_MAX_EDGE {
        let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
        return ctx.load_texture(name, ci, TextureOptions::LINEAR);
    }
    let s = TEXTURE_MAX_EDGE as f64 / longest as f64;
    let (tw, th) = (
        ((w as f64 * s).round() as u32).max(1),
        ((h as f64 * s).round() as u32).max(1),
    );
    let small = image::imageops::thumbnail(img, tw, th);
    let ci = egui::ColorImage::from_rgba_unmultiplied([tw as usize, th as usize], small.as_raw());
    ctx.load_texture(name, ci, TextureOptions::LINEAR)
}

impl LayerCache {
    /// Look up a path. On a miss, starts a background decode and returns
    /// `None`; the caller keeps showing its placeholder until a later frame.
    /// Call [`LayerCache::poll`] once per frame to receive finished decodes.
    pub fn get_or_load(&mut self, ctx: &egui::Context, path: &Path) -> Option<&CachedImage> {
        self.clock += 1;
        let clock = self.clock;

        // Detect a file that changed on disk (throttled: one stat per entry
        // every STALE_CHECK_EVERY, not every frame).
        if let Some(entry) = self.entries.get_mut(path) {
            if entry.last_check.elapsed() >= STALE_CHECK_EVERY {
                entry.last_check = Instant::now();
                if mtime_of(path) != entry.mtime {
                    self.entries.remove(path);
                }
            }
        }

        if self.entries.contains_key(path) {
            let entry = self.entries.get_mut(path)?;
            entry.last_used = clock;
            return Some(&*entry);
        }

        if !self.pending.contains(path) && !self.failed.contains_key(path) {
            self.pending.insert(path.to_path_buf());
            let tx = self.tx.clone();
            let path = path.to_path_buf();
            let repaint = ctx.clone();
            std::thread::spawn(move || {
                let mtime = mtime_of(&path);
                let result = image::open(&path)
                    .map(|i| i.to_rgba8())
                    .map_err(|e| e.to_string());
                let _ = tx.send(Decoded {
                    path,
                    mtime,
                    result,
                });
                repaint.request_repaint();
            });
        }
        None
    }

    /// Receive finished decodes, upload their textures and enforce the budget.
    /// Returns `true` if anything changed (so the caller can repaint).
    pub fn poll(&mut self, ctx: &egui::Context, in_use: &HashSet<PathBuf>) -> bool {
        let mut changed = false;
        while let Ok(done) = self.rx.try_recv() {
            self.pending.remove(&done.path);
            changed = true;
            match done.result {
                Ok(img) => {
                    self.failed.remove(&done.path);
                    self.clock += 1;
                    let texture = upload(ctx, &done.path, &img);
                    let dims = img.dimensions();
                    self.entries.insert(
                        done.path,
                        CachedImage {
                            cpu: Arc::new(img),
                            texture,
                            dims,
                            mtime: done.mtime,
                            last_check: Instant::now(),
                            last_used: self.clock,
                        },
                    );
                }
                Err(e) => {
                    self.failed.insert(done.path, e);
                }
            }
        }
        if changed {
            self.trim(in_use);
        }
        // A failed path is retried once the file's mtime changes or the
        // layer is disconnected (handled by `forget_unused_failures`).
        changed
    }

    /// Forget failures for paths nothing references any more, so reconnecting
    /// the node later retries the load.
    pub fn forget_unused_failures(&mut self, in_use: &HashSet<PathBuf>) {
        self.failed.retain(|p, _| in_use.contains(p));
    }

    #[allow(dead_code)]
    pub fn is_pending(&self, path: &Path) -> bool {
        self.pending.contains(path)
    }

    #[allow(dead_code)]
    pub fn error(&self, path: &Path) -> Option<&str> {
        self.failed.get(path).map(String::as_str)
    }

    #[allow(dead_code)]
    pub fn contains(&self, path: &Path) -> bool {
        self.entries.contains_key(path)
    }

    pub fn get(&self, path: &Path) -> Option<&CachedImage> {
        self.entries.get(path)
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Evict least-recently-used entries over budget. Paths currently shown
    /// on the canvas are never evicted; disconnected ones are kept until the
    /// budget forces them out, which is what makes reconnecting instant.
    fn trim(&mut self, in_use: &HashSet<PathBuf>) {
        let mut total: usize = self.entries.values().map(|e| e.cpu.as_raw().len()).sum();
        if total <= CPU_BUDGET {
            return;
        }
        let mut order: Vec<(PathBuf, u64, usize)> = self
            .entries
            .iter()
            .filter(|(p, _)| !in_use.contains(*p))
            .map(|(p, e)| (p.clone(), e.last_used, e.cpu.as_raw().len()))
            .collect();
        order.sort_by_key(|(_, used, _)| *used);
        for (path, _, bytes) in order {
            if total <= CPU_BUDGET {
                break;
            }
            self.entries.remove(&path);
            total = total.saturating_sub(bytes);
        }
    }
}
