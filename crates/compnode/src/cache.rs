//! Content-addressed result cache shared by every session on this machine.
//!
//! The worker writes results here as soon as inference finishes. A saved
//! session copies referenced files beside the session JSON so it can move to
//! another machine without re-running the models.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use crate::assets::output_stem;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CacheKey(pub(crate) String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CacheError(pub(crate) String);

impl std::fmt::Display for CacheError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Storage boundary for processed RGBA outputs. Tests can use the in-memory
/// implementation without creating cache files on disk.
pub(crate) trait OutputCacheInterface: Send + Sync {
    fn get(&self, key: &CacheKey) -> Option<image::RgbaImage>;
    #[allow(dead_code)]
    fn put(&self, key: &CacheKey, image: &image::RgbaImage) -> Result<PathBuf, CacheError>;
    fn path_for(&self, key: &str) -> PathBuf;
    fn store_named(
        &self,
        key: &str,
        source_name: &str,
        suffix: &str,
        image: &image::RgbaImage,
    ) -> Result<PathBuf, CacheError>;
}

#[derive(Clone)]
pub(crate) struct OutputCache {
    dir: PathBuf,
    index: Arc<Mutex<HashMap<CacheKey, PathBuf>>>,
}

impl OutputCache {
    pub(crate) fn new(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&dir);
        // Session files store result paths, so keep the cache root absolute.
        let dir = std::path::absolute(&dir).unwrap_or(dir);
        Self {
            dir,
            index: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn default_dir() -> PathBuf {
        if let Some(base) = std::env::var_os("LOCALAPPDATA") {
            PathBuf::from(base).join("rembg").join("studio-cache")
        } else {
            std::env::temp_dir().join("compnode-cache")
        }
    }

    pub(crate) fn path_for(&self, key: &str) -> PathBuf {
        let cache_key = CacheKey(key.to_owned());
        if let Some(path) = self
            .index
            .lock()
            .ok()
            .and_then(|index| index.get(&cache_key).cloned())
        {
            return path;
        }
        let direct = self.dir.join(format!("{key}.png"));
        if direct.is_file() {
            return direct;
        }
        let suffix = &key[..key.len().min(12)];
        let found = fs::read_dir(&self.dir).ok().and_then(|entries| {
            entries.flatten().map(|entry| entry.path()).find(|path| {
                path.extension().is_some_and(|extension| extension == "png")
                    && path
                        .file_stem()
                        .is_some_and(|stem| stem.to_string_lossy().ends_with(suffix))
            })
        });
        if let Some(path) = found {
            if let Ok(mut index) = self.index.lock() {
                index.insert(cache_key, path.clone());
            }
            path
        } else {
            direct
        }
    }

    pub(crate) fn store_named(
        &self,
        key: &str,
        source_name: &str,
        suffix: &str,
        img: &image::RgbaImage,
    ) -> Result<PathBuf, String> {
        let path = self
            .dir
            .join(format!("{}.png", output_stem(source_name, suffix, key)));
        fs::create_dir_all(&self.dir)
            .map_err(|e| format!("create cache dir {}: {e}", self.dir.display()))?;
        image::save_buffer(
            &path,
            img.as_raw(),
            img.width(),
            img.height(),
            image::ColorType::Rgba8,
        )
        .map_err(|e| format!("write cached result: {e}"))?;
        if let Ok(mut index) = self.index.lock() {
            index.insert(CacheKey(key.to_owned()), path.clone());
        }
        Ok(path)
    }
}

impl OutputCacheInterface for OutputCache {
    fn get(&self, key: &CacheKey) -> Option<image::RgbaImage> {
        image::open(self.path_for(&key.0))
            .ok()
            .map(|image| image.to_rgba8())
    }

    fn put(&self, key: &CacheKey, image: &image::RgbaImage) -> Result<PathBuf, CacheError> {
        let path = self.path_for(&key.0);
        fs::create_dir_all(&self.dir)
            .map_err(|err| CacheError(format!("create cache directory: {err}")))?;
        image::save_buffer(
            &path,
            image.as_raw(),
            image.width(),
            image.height(),
            image::ColorType::Rgba8,
        )
        .map_err(|err| CacheError(format!("write cache image: {err}")))?;
        Ok(path)
    }

    fn path_for(&self, key: &str) -> PathBuf {
        OutputCache::path_for(self, key)
    }

    fn store_named(
        &self,
        key: &str,
        source_name: &str,
        suffix: &str,
        image: &image::RgbaImage,
    ) -> Result<PathBuf, CacheError> {
        OutputCache::store_named(self, key, source_name, suffix, image).map_err(CacheError)
    }
}

#[derive(Default)]
pub(crate) struct MemoryOutputCache {
    entries: Mutex<HashMap<CacheKey, image::RgbaImage>>,
}

impl OutputCacheInterface for MemoryOutputCache {
    fn get(&self, key: &CacheKey) -> Option<image::RgbaImage> {
        self.entries.lock().ok()?.get(key).cloned()
    }

    fn put(&self, key: &CacheKey, image: &image::RgbaImage) -> Result<PathBuf, CacheError> {
        self.entries
            .lock()
            .map_err(|_| CacheError("memory cache lock was poisoned".to_owned()))?
            .insert(key.clone(), image.clone());
        Ok(PathBuf::from("memory-cache").join(format!("{}.png", key.0)))
    }

    fn path_for(&self, key: &str) -> PathBuf {
        PathBuf::from("memory-cache").join(format!("{key}.png"))
    }

    fn store_named(
        &self,
        key: &str,
        _source_name: &str,
        _suffix: &str,
        image: &image::RgbaImage,
    ) -> Result<PathBuf, CacheError> {
        self.put(&CacheKey(key.to_owned()), image)
    }
}

#[cfg(test)]
mod interface_tests {
    use super::{CacheKey, MemoryOutputCache, OutputCache, OutputCacheInterface};

    #[test]
    fn memory_cache_roundtrips_without_disk_storage() {
        let cache = MemoryOutputCache::default();
        let key = CacheKey("headless-test".to_owned());
        let image = image::RgbaImage::from_pixel(2, 1, image::Rgba([4, 8, 12, 255]));

        let path = cache.put(&key, &image).unwrap();

        assert_eq!(
            path,
            std::path::PathBuf::from("memory-cache/headless-test.png")
        );
        assert_eq!(cache.get(&key), Some(image));
    }

    #[test]
    fn disk_cache_interface_finds_named_results_by_content_key() {
        let root = std::env::temp_dir().join(format!(
            "compnode-cache-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cache = OutputCache::new(root.clone());
        let key = CacheKey("0123456789abcdef".to_owned());
        let image = image::RgbaImage::from_pixel(2, 1, image::Rgba([9, 8, 7, 255]));

        let stored = cache
            .store_named(&key.0, "source.png", "filter", &image)
            .unwrap();

        assert_eq!(cache.path_for(&key.0), stored);
        assert_eq!(cache.get(&key), Some(image));
        let _ = std::fs::remove_dir_all(root);
    }
}

pub(crate) fn cache_key(domain: &str, input: &image::RgbaImage, params: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(params);
    hasher.update(input.width().to_le_bytes());
    hasher.update(input.height().to_le_bytes());
    hasher.update(input.as_raw());
    format!("{:x}", hasher.finalize())
}

/// Content-addressed key for a multi-image composite (Group node): hashes every
/// source image plus the serialized transform parameters.
pub(crate) fn cache_key_layers(domain: &str, inputs: &[image::RgbaImage], params: &[u8]) -> String {
    cache_key_refs(domain, inputs.iter(), params)
}

/// Same as [`cache_key_layers`], but borrows the inputs so a filter render can
/// hash images it still needs for evaluation without cloning them.
pub(crate) fn cache_key_refs<'a>(
    domain: &str,
    inputs: impl IntoIterator<Item = &'a image::RgbaImage>,
    params: &[u8],
) -> String {
    let inputs: Vec<&image::RgbaImage> = inputs.into_iter().collect();
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(params);
    hasher.update((inputs.len() as u32).to_le_bytes());
    for input in inputs {
        hasher.update(input.width().to_le_bytes());
        hasher.update(input.height().to_le_bytes());
        hasher.update(input.as_raw());
    }
    format!("{:x}", hasher.finalize())
}

/// Fallback key when the worker could not persist its result: hash the output
/// itself so a session save can still keep the image.
pub(crate) fn output_cache_key(img: &image::RgbaImage) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"compnode-output-v1");
    hasher.update(img.width().to_le_bytes());
    hasher.update(img.height().to_le_bytes());
    hasher.update(img.as_raw());
    format!("{:x}", hasher.finalize())
}

/// Find a legacy cache-keyed result. Old sessions may keep a copy beside the
/// JSON; new sessions store absolute paths and do not use this.
pub(crate) fn load_cached(
    cache: &OutputCache,
    assets: Option<&Path>,
    key: &str,
) -> Option<(PathBuf, image::RgbaImage)> {
    let mut candidates = Vec::with_capacity(2);
    if let Some(assets) = assets {
        candidates.push(assets.join(format!("{key}.png")));
    }
    candidates.push(cache.path_for(key));

    candidates
        .into_iter()
        .find_map(|path| image::open(&path).ok().map(|img| (path, img.to_rgba8())))
        .or_else(|| {
            let key = CacheKey(key.to_owned());
            cache.get(&key).map(|image| (cache.path_for(&key.0), image))
        })
}
