//! Source-image assets shared by Image nodes.
//!
//! Assets are content-addressed. The same bytes only get stored once even when
//! several Image nodes use them. Sessions embed the encoded bytes as base64 so
//! a project remains portable without a sidecar assets folder.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;

use egui_snarl::{NodeId, Snarl};
use image::ImageEncoder;
use sha2::{Digest, Sha256};

use crate::model::Node;
use crate::nodes::IMAGE_EXTENSIONS;
use crate::settings::Settings;

/// One source image. `encoded` keeps the original file bytes when possible so
/// sessions can restore the asset without the original path.
#[derive(Clone)]
pub(crate) struct Asset {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub path: Option<PathBuf>,
    pub encoded: Arc<Vec<u8>>,
}

impl Asset {
    pub(crate) fn decode(&self) -> Result<image::RgbaImage, String> {
        image::load_from_memory(&self.encoded)
            .map(|image| image.to_rgba8())
            .map_err(|err| format!("Could not decode {}: {err}", self.name))
    }
}

/// In-memory asset registry. Assets are only persisted when an Image node
/// references them.
#[derive(Clone, Default)]
pub(crate) struct AssetStore {
    assets: HashMap<String, Asset>,
    /// Insertion order for stable session output and source-name lookups.
    order: Vec<String>,
    /// Bumped whenever the set of stored assets changes.
    revision: u64,
}

impl AssetStore {
    pub(crate) fn get(&self, id: &str) -> Option<&Asset> {
        self.assets.get(id)
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn insert_path(&mut self, path: impl AsRef<Path>) -> Result<String, String> {
        let path = absolute(path.as_ref());
        let bytes =
            fs::read(&path).map_err(|err| format!("Could not read {}: {err}", path.display()))?;
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "image.png".to_owned());
        self.insert_bytes(name, bytes, Some(path))
    }

    pub(crate) fn insert_bytes(
        &mut self,
        name: impl Into<String>,
        encoded: Vec<u8>,
        path: Option<PathBuf>,
    ) -> Result<String, String> {
        let name = clean_name(&name.into());
        let id = content_id(&encoded);
        if let Some(asset) = self.assets.get_mut(&id) {
            // Keep the first useful name, but fill in a missing path.
            if asset.name.is_empty() {
                asset.name = name;
            }
            if asset.path.is_none() {
                asset.path = path;
            }
            return Ok(id);
        }

        let decoded = image::load_from_memory(&encoded)
            .map_err(|err| format!("Could not decode {name}: {err}"))?;
        let asset = Asset {
            name,
            width: decoded.width(),
            height: decoded.height(),
            path,
            encoded: Arc::new(encoded),
        };
        self.assets.insert(id.clone(), asset);
        self.order.push(id.clone());
        self.revision = self.revision.wrapping_add(1);
        Ok(id)
    }

    pub(crate) fn insert_rgba(
        &mut self,
        name: impl Into<String>,
        image: &image::RgbaImage,
    ) -> Result<String, String> {
        let mut encoded = Vec::new();
        image::codecs::png::PngEncoder::new(&mut encoded)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|err| format!("Could not encode pasted image: {err}"))?;
        self.insert_bytes(name, encoded, None)
    }

    /// Restore an asset from a session. The session already carries the
    /// dimensions, so this avoids decoding the image until a node needs it.
    pub(crate) fn insert_session(
        &mut self,
        id: String,
        name: String,
        width: u32,
        height: u32,
        encoded: Vec<u8>,
        path: Option<PathBuf>,
    ) {
        if self.assets.contains_key(&id) {
            return;
        }
        let asset = Asset {
            name: clean_name(&name),
            width,
            height,
            path,
            encoded: Arc::new(encoded),
        };
        self.assets.insert(id.clone(), asset);
        self.order.push(id);
    }

    /// Drop assets that no Image node references anymore.
    pub(crate) fn retain_used(&mut self, snarl: &Snarl<Node>) {
        let used: HashSet<String> = snarl
            .node_ids()
            .filter_map(|(_, node)| node.image_node())
            .filter_map(|image| image.asset_id.clone())
            .collect();
        let before = self.assets.len();
        self.assets.retain(|id, _| used.contains(id));
        self.order.retain(|id| used.contains(id));
        if self.assets.len() != before {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub(crate) fn name(&self, id: &str) -> Option<&str> {
        self.assets.get(id).map(|asset| asset.name.as_str())
    }

    pub(crate) fn path(&self, id: &str) -> Option<&Path> {
        self.assets.get(id).and_then(|asset| asset.path.as_deref())
    }

    pub(crate) fn dimensions(&self, id: &str) -> Option<(u32, u32)> {
        self.assets.get(id).map(|asset| (asset.width, asset.height))
    }
}

/// Asset IDs are stable content hashes. This also makes de-duplication
/// automatic when a file is dropped twice or pasted after being picked.
pub(crate) fn content_id(encoded: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"compnode-asset-v1");
    hasher.update(encoded);
    format!("{:x}", hasher.finalize())
}

fn clean_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        "image.png".to_owned()
    } else {
        name.to_owned()
    }
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Name for an image fetched from a URL. Query strings and fragments are
/// ignored, and an image-less URL gets a generic name.
pub(crate) fn url_name(url: &str) -> String {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    without_query
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty() && name.contains('.'))
        .map(|name| name.to_owned())
        .unwrap_or_else(|| "downloaded-image.png".to_owned())
}

pub(crate) fn looks_like_url(text: &str) -> bool {
    let text = text.trim();
    text.starts_with("http://") || text.starts_with("https://")
}

pub(crate) struct FetchedImage {
    pub url: String,
    pub target: Option<NodeId>,
    pub pos: egui::Pos2,
    pub result: Result<Vec<u8>, String>,
}

/// Fetch an image off the UI thread. The result is delivered through the
/// app's channel and decoded on the next frame.
pub(crate) fn fetch_url(
    ctx: egui::Context,
    sender: Sender<FetchedImage>,
    url: String,
    target: Option<NodeId>,
    pos: egui::Pos2,
) {
    thread::spawn(move || {
        let result = ureq::get(&url)
            .call()
            .and_then(|mut response| response.body_mut().read_to_vec())
            .map_err(|err| format!("Could not fetch {url}: {err}"));
        let _ = sender.send(FetchedImage {
            url,
            target,
            pos,
            result,
        });
        ctx.request_repaint();
    });
}

/// Read an image from the system clipboard. Text-only clipboards return
/// `Ok(None)` so the caller can fall back to a URL.
pub(crate) fn clipboard_image() -> Result<Option<(String, image::RgbaImage)>, String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|err| format!("Clipboard unavailable: {err}"))?;
    match clipboard.get_image() {
        Ok(data) => {
            let image = image::RgbaImage::from_raw(
                data.width as u32,
                data.height as u32,
                data.bytes.into_owned(),
            )
            .ok_or_else(|| "Clipboard image has invalid dimensions".to_owned())?;
            Ok(Some(("pasted-image.png".to_owned(), image)))
        }
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(err) => Err(format!("Could not read clipboard image: {err}")),
    }
}

pub(crate) fn clipboard_text() -> Option<String> {
    arboard::Clipboard::new()
        .ok()
        .and_then(|mut clipboard| clipboard.get_text().ok())
}

/// The backend drop folder used by the studio. `INPUT_DIR` overrides the
/// repository-relative default.
/// Save dropped/pasted/downloaded bytes into the backend `input/` folder so
/// they can be reused later. A unique filename is chosen when the requested
/// name already exists.
pub(crate) fn save_to_input(name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let dir = input_directory();
    fs::create_dir_all(&dir).map_err(|err| format!("create input dir {}: {err}", dir.display()))?;
    let base = Path::new(name)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_owned());
    let extension = Path::new(name)
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned())
        .unwrap_or_else(|| "png".to_owned());
    let mut candidate = dir.join(format!("{base}.{extension}"));
    let mut number = 2u32;
    while candidate.exists() {
        candidate = dir.join(format!("{base} ({number}).{extension}"));
        number += 1;
    }
    fs::write(&candidate, bytes).map_err(|err| format!("write {}: {err}", candidate.display()))?;
    Ok(candidate)
}

pub(crate) fn input_directory() -> PathBuf {
    Settings::load().input_dir()
}

pub(crate) fn list_input_images() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(input_directory())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_image_path(path))
        .collect();
    files.sort_by_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    });
    files
}

pub(crate) fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            IMAGE_EXTENSIONS
                .iter()
                .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        })
}

/// Name a processed result after its source image, with a short content hash
/// so two runs never silently overwrite each other.
pub(crate) fn output_stem(source_name: &str, suffix: &str, key: &str) -> String {
    let stem = Path::new(source_name)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_owned());
    let mut clean = String::with_capacity(stem.len());
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ' ' | '.') {
            clean.push(ch);
        } else {
            clean.push('_');
        }
    }
    let clean = clean.trim_matches([' ', '.', '_']);
    let clean = if clean.is_empty() { "image" } else { clean };
    let clean: String = clean.chars().take(80).collect();
    format!("{clean}-{suffix}-{}", &key[..key.len().min(12)])
}
