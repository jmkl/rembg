//! Content-addressed result cache shared by every session on this machine.
//!
//! The worker writes results here as soon as inference finishes. A saved
//! session copies referenced files beside the session JSON so it can move to
//! another machine without re-running the models.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::assets::output_stem;

#[derive(Clone)]
pub(crate) struct OutputCache {
    dir: PathBuf,
}

impl OutputCache {
    pub(crate) fn new(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&dir);
        // Session files store result paths, so keep the cache root absolute.
        let dir = std::path::absolute(&dir).unwrap_or(dir);
        Self { dir }
    }

    pub(crate) fn default_dir() -> PathBuf {
        if let Some(base) = std::env::var_os("LOCALAPPDATA") {
            PathBuf::from(base).join("rembg").join("studio-cache")
        } else {
            std::env::temp_dir().join("compnode-cache")
        }
    }

    pub(crate) fn path_for(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.png"))
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
        Ok(path)
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
}
