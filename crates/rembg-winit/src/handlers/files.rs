//! `GET /api/files` and the read-only static mounts `/input`, `/output`,
//! `/workspaces`.

use std::path::PathBuf;

use anyhow::Result;
use http::Response;
use serde::Serialize;

use crate::{
    transport::{bytes_response, method_not_allowed, not_found},
    util,
};

pub const INPUT_DIR: &str = "input";
pub const OUTPUT_DIR: &str = "output";
pub const WORKSPACES_DIR: &str = "workspaces";

pub const SHARED_DIR: &str = "output/shared";
pub const SHARED_ASSET_DIR: &str = "output/shared/assets";
pub const SHARED_MASK_DIR: &str = "output/shared/masks";
pub const SHARED_RESTORE_DIR: &str = "output/shared/restores";
pub const SHARED_THUMB_DIR: &str = "output/shared/thumbs";
pub const CACHE_DIR: &str = "output/cache";

/// Create the working folders on startup.
pub fn ensure_dirs() {
    for dir in [
        INPUT_DIR,
        OUTPUT_DIR,
        WORKSPACES_DIR,
        SHARED_DIR,
        SHARED_ASSET_DIR,
        SHARED_MASK_DIR,
        SHARED_RESTORE_DIR,
        SHARED_THUMB_DIR,
        CACHE_DIR,
        "output/cache/mask",
        "output/cache/restore",
    ] {
        let _ = std::fs::create_dir_all(dir);
    }
    for kind in ["templates", "projects"] {
        let _ = std::fs::create_dir_all(format!("{}/{}", WORKSPACES_DIR, kind));
    }
}

#[derive(Serialize)]
struct FileItem {
    name: String,
    kind: &'static str,
    url: String,
    size: u64,
    mtime: u64,
}

/// Images available in the local `./input` and `./output` folders.
pub fn get() -> Result<Response<Vec<u8>>> {
    let listing = |folder: &str, kind: &'static str| -> Vec<FileItem> {
        let mut items = Vec::new();
        if let Ok(entries) = std::fs::read_dir(folder) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if !util::is_image(name) || !path.is_file() {
                    continue;
                }
                let Ok(meta) = std::fs::metadata(&path) else {
                    continue;
                };
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                items.push(FileItem {
                    name: name.to_string(),
                    kind,
                    url: format!("/{folder}/{name}"),
                    size: meta.len(),
                    mtime,
                });
            }
        }
        items.sort_by_key(|a| std::cmp::Reverse(a.mtime));
        items.truncate(200);
        items
    };

    let body = serde_json::json!({
        "input": listing(INPUT_DIR, "input"),
        "output": listing(OUTPUT_DIR, "output"),
    });
    Ok(crate::transport::json_response(&body))
}

/// Serve a file from `/input/<rel>`, `/output/<rel>` or `/workspaces/<rel>`.
pub fn serve(method: &str, path: &str) -> Result<Response<Vec<u8>>> {
    if method != "GET" && method != "HEAD" {
        return Ok(method_not_allowed());
    }

    let (base, rel) = if let Some(rest) = path.strip_prefix("/input/") {
        (INPUT_DIR, rest)
    } else if let Some(rest) = path.strip_prefix("/output/") {
        (OUTPUT_DIR, rest)
    } else if let Some(rest) = path.strip_prefix("/workspaces/") {
        (WORKSPACES_DIR, rest)
    } else {
        return Ok(not_found());
    };

    let base = resolve_base(base);
    let target = match resolve_within(&base, &util::percent_decode(rel)) {
        Some(target) => target,
        None => return Ok(not_found()),
    };
    if !target.is_file() {
        return Ok(not_found());
    }

    let bytes = match std::fs::read(&target) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(not_found()),
    };
    let content_type = util::content_type_for(&target.to_string_lossy());
    Ok(bytes_response(bytes, content_type))
}

fn resolve_base(dir: &str) -> PathBuf {
    std::fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir))
}

/// Join `rel` onto `base` and make sure the result stays inside `base`.
fn resolve_within(base: &std::path::Path, rel: &str) -> Option<PathBuf> {
    let target = base.join(rel);
    let target = std::fs::canonicalize(&target).ok()?;
    if !target.starts_with(base) {
        return None;
    }
    Some(target)
}
