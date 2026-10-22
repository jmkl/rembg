//! `/api/workspaces/*` — templates and projects.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use http::Response;
use serde_json::{Value, json};

use crate::handlers::files;
use crate::transport::{bytes_response, error};
use crate::util;

const IMAGE_SUFFIXES: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".webp", ".bmp", ".gif", ".tif", ".tiff",
];

pub fn not_implemented() -> Result<Response<Vec<u8>>> {
    Ok(crate::transport::error(
        http::StatusCode::NOT_IMPLEMENTED,
        "Workspaces are not implemented yet.",
    ))
}

fn workspaces_dir() -> &'static str {
    files::WORKSPACES_DIR
}

fn is_kind(kind: &str) -> bool {
    kind == "templates" || kind == "projects"
}

fn workspace_root(kind: &str, sid: &str) -> PathBuf {
    PathBuf::from(workspaces_dir()).join(kind).join(sid)
}

fn read_manifest(root: &Path) -> Option<Value> {
    let path = root.join("workspace.json");
    let data = fs::read_to_string(&path).ok()?;
    let v: Value = serde_json::from_str(&data).ok()?;
    if v.is_object() { Some(v) } else { None }
}

fn write_manifest(root: &Path, manifest: &Value) -> Result<()> {
    let path = root.join("workspace.json");
    let tmp = root.join("workspace.json.tmp");
    let s = serde_json::to_string_pretty(manifest).map_err(|e| anyhow!("{e}"))?;
    fs::write(&tmp, s).map_err(|e| anyhow!("{e}"))?;
    fs::rename(&tmp, &path).map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

fn existing_output_file(value: &Value) -> Option<String> {
    let s = value.as_str()?;
    let rel = s.replace('\\', "/");
    let parts: Vec<&str> = rel
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.len() < 2 || parts[0] != "output" || parts.contains(&"..") {
        return None;
    }
    let full = Path::new(files::OUTPUT_DIR).join(parts[1..].join("/"));
    if full.is_file() {
        Some(parts.join("/"))
    } else {
        None
    }
}

fn workspace_meta(kind: &str, sid: &str, manifest: &Value) -> Value {
    let graph = manifest.get("graph");
    let nodes = graph
        .and_then(|g| g.get("nodes"))
        .and_then(|n| n.as_array());
    let node_count = nodes.map(|n| n.len()).unwrap_or(0);

    let mut thumb_url = existing_output_file(manifest.get("thumb").unwrap_or(&Value::Null));
    if let Some(u) = thumb_url.take() {
        thumb_url = Some(format!("/{}", u));
    } else {
        let legacy = workspace_root(kind, sid).join("thumb.png");
        if legacy.is_file() {
            thumb_url = Some(format!("/workspaces/{}/{}/thumb.png", kind, sid));
        }
    }

    json!({
        "id": sid,
        "name": manifest.get("name").and_then(|v| v.as_str()).unwrap_or(sid),
        "createdAt": manifest.get("createdAt"),
        "updatedAt": manifest.get("updatedAt"),
        "nodes": node_count,
        "thumb": thumb_url,
    })
}

fn sanitize_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", secs)
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    hex_encode(&result)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn save_shared_bytes(data: &[u8], dest_dir: &str, public_dir: &str, ext: &str) -> String {
    let hash = sha256_hex(data);
    let name = format!("{}{}", hash, ext);
    let dest = Path::new(dest_dir).join(&name);
    if !dest.exists() {
        let _ = fs::create_dir_all(dest_dir);
        let tmp = format!("{}.tmp", dest.display());
        if fs::write(&tmp, data).is_ok() {
            let _ = fs::rename(&tmp, &dest);
        }
    }
    format!("{}/{}", public_dir.trim_start_matches('/'), name)
}

fn asset_ext(filename: Option<&str>, mime: &str) -> String {
    if let Some(name) = filename {
        let lower = name.to_ascii_lowercase();
        for suf in IMAGE_SUFFIXES {
            if lower.ends_with(suf) {
                return suf.to_string();
            }
        }
    }
    let mime = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match mime.as_str() {
        "image/jpeg" | "image/jpg" => ".jpg".to_string(),
        "image/png" => ".png".to_string(),
        "image/webp" => ".webp".to_string(),
        "image/bmp" => ".bmp".to_string(),
        "image/gif" => ".gif".to_string(),
        "image/tiff" => ".tiff".to_string(),
        _ => ".png".to_string(),
    }
}

fn gen_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{:x}{:x}", ms, n)
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn extract_boundary_from_body(body: &[u8]) -> Option<String> {
    let end = body.iter().position(|&b| b == b'\n')?;
    let line = &body[..end];
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let line = std::str::from_utf8(line).ok()?;
    let b = line.strip_prefix("--")?;
    Some(b.to_string())
}

// ---------- handlers ----------

pub fn list(kind: &str) -> Result<Response<Vec<u8>>> {
    if !is_kind(kind) {
        return Ok(error(http::StatusCode::NOT_FOUND, "Unknown workspace type"));
    }
    let base = PathBuf::from(workspaces_dir()).join(kind);
    let mut items: Vec<Value> = Vec::new();
    if let Ok(entries) = fs::read_dir(&base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let sid = match path.file_name().and_then(|n| n.to_str()) {
                Some(s) if !s.starts_with('.') => s.to_string(),
                _ => continue,
            };
            if let Some(manifest) = read_manifest(&path) {
                items.push(workspace_meta(kind, &sid, &manifest));
            }
        }
    }
    items.sort_by(|a, b| {
        let a = a.get("updatedAt").and_then(|v| v.as_str()).unwrap_or("");
        let b = b.get("updatedAt").and_then(|v| v.as_str()).unwrap_or("");
        b.cmp(a)
    });
    let body = serde_json::to_vec(&items).unwrap_or_else(|_| b"[]".to_vec());
    Ok(bytes_response(body, "application/json"))
}

pub fn create(req: &crate::transport::ParsedRequest) -> Result<Response<Vec<u8>>> {
    let rest = req.path.trim_start_matches("/api/workspaces/");
    let kind = rest.split('/').next().unwrap_or("");
    if !is_kind(kind) {
        return Ok(error(http::StatusCode::NOT_FOUND, "Unknown workspace type"));
    }
    let sid = gen_id();
    store_with(req, kind, &sid, true)
}

pub fn get_one(kind: &str, sid: &str) -> Result<Response<Vec<u8>>> {
    if !is_kind(kind) {
        return Ok(error(http::StatusCode::NOT_FOUND, "Unknown workspace type"));
    }
    let root = workspace_root(kind, sid);
    let manifest = match read_manifest(&root) {
        Some(m) => m,
        None => return Ok(error(http::StatusCode::NOT_FOUND, "Workspace not found")),
    };
    let mut m = manifest.clone();
    if let Some(obj) = m.as_object_mut() {
        obj.insert("id".to_string(), Value::String(sid.to_string()));
    }
    let meta = workspace_meta(kind, sid, &manifest);
    let thumb = meta.get("thumb").cloned().unwrap_or(Value::Null);
    if let Some(obj) = m.as_object_mut() {
        obj.insert("thumb".to_string(), thumb);
    }
    let body = serde_json::to_vec(&m).unwrap_or_else(|_| b"{}".to_vec());
    Ok(bytes_response(body, "application/json"))
}

pub fn update(
    req: &crate::transport::ParsedRequest,
    kind: &str,
    sid: &str,
) -> Result<Response<Vec<u8>>> {
    store_with(req, kind, sid, false)
}

pub fn rename(
    req: &crate::transport::ParsedRequest,
    kind: &str,
    sid: &str,
) -> Result<Response<Vec<u8>>> {
    if !is_kind(kind) {
        return Ok(error(http::StatusCode::NOT_FOUND, "Unknown workspace type"));
    }
    let root = workspace_root(kind, sid);
    let mut manifest = match read_manifest(&root) {
        Some(m) => m,
        None => return Ok(error(http::StatusCode::NOT_FOUND, "Workspace not found")),
    };
    let body: Value = serde_json::from_slice(req.body).unwrap_or(Value::Null);
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        return Ok(error(http::StatusCode::BAD_REQUEST, "Name is required"));
    }
    let name = sanitize_name(name);
    if let Some(obj) = manifest.as_object_mut() {
        obj.insert("name".to_string(), Value::String(name.clone()));
        obj.insert("updatedAt".to_string(), Value::String(now_iso()));
    }
    if let Err(e) = write_manifest(&root, &manifest) {
        return Ok(error(
            http::StatusCode::INTERNAL_SERVER_ERROR,
            &e.to_string(),
        ));
    }
    let meta = workspace_meta(kind, sid, &manifest);
    let body = serde_json::to_vec(&meta).unwrap_or_else(|_| b"{}".to_vec());
    Ok(bytes_response(body, "application/json"))
}

pub fn delete_one(kind: &str, sid: &str) -> Result<Response<Vec<u8>>> {
    if !is_kind(kind) {
        return Ok(error(http::StatusCode::NOT_FOUND, "Unknown workspace type"));
    }
    let root = workspace_root(kind, sid);
    if !root.is_dir() {
        return Ok(error(http::StatusCode::NOT_FOUND, "Workspace not found"));
    }
    let _ = fs::remove_dir_all(&root);
    let body = json!({"deleted": true, "id": sid});
    let b = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    Ok(bytes_response(b, "application/json"))
}

// ---------- core store ----------

fn store_with(
    req: &crate::transport::ParsedRequest,
    kind: &str,
    sid: &str,
    create: bool,
) -> Result<Response<Vec<u8>>> {
    if !is_kind(kind) {
        return Ok(error(http::StatusCode::NOT_FOUND, "Unknown workspace type"));
    }
    let base = PathBuf::from(workspaces_dir()).join(kind);
    let root = base.join(sid);

    if create && root.exists() {
        return Ok(error(
            http::StatusCode::CONFLICT,
            "Workspace already exists",
        ));
    }
    if !create && !root.is_dir() {
        return Ok(error(http::StatusCode::NOT_FOUND, "Workspace not found"));
    }

    let old_manifest = if create { None } else { read_manifest(&root) };

    let _ = fs::create_dir_all(&base);
    let tmp = base.join(format!(".{}.{}.tmp", sid, gen_id()));
    let _ = fs::create_dir_all(&tmp);

    match do_store(req, kind, sid, create, &root, &tmp, old_manifest.as_ref()) {
        Ok(meta) => {
            if root.exists() {
                let _ = fs::remove_dir_all(&root);
            }
            let mut promoted = false;
            for _attempt in 0..4 {
                if fs::rename(&tmp, &root).is_ok() {
                    promoted = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            if !promoted {
                let _ = copy_dir_all(&tmp, &root);
                let _ = fs::remove_dir_all(&tmp);
            }
            let body = serde_json::to_vec(&meta).unwrap_or_else(|_| b"{}".to_vec());
            Ok(bytes_response(body, "application/json"))
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&tmp);
            Ok(error(
                http::StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    }
}

fn do_store(
    req: &crate::transport::ParsedRequest,
    kind: &str,
    sid: &str,
    _create: bool,
    _root: &Path,
    tmp: &Path,
    old: Option<&Value>,
) -> Result<Value> {
    let boundary = extract_boundary_from_body(req.body)
        .ok_or_else(|| anyhow!("could not determine multipart boundary"))?;
    let ct = format!("multipart/form-data; boundary={}", boundary);
    let parts = util::parse_multipart(&ct, req.body)?;

    let manifest_part = parts
        .iter()
        .find(|p| p.name == "manifest")
        .ok_or_else(|| anyhow!("missing manifest"))?;
    let manifest_str = String::from_utf8(manifest_part.data.clone())
        .map_err(|e| anyhow!("invalid manifest utf8: {e}"))?;
    let mut manifest: Value =
        serde_json::from_str(&manifest_str).map_err(|e| anyhow!("invalid manifest json: {e}"))?;
    if !manifest.is_object() {
        return Err(anyhow!("manifest must be an object"));
    }
    if !manifest
        .get("graph")
        .map(|v| v.is_object())
        .unwrap_or(false)
    {
        return Err(anyhow!("manifest has no graph"));
    }

    // assets
    let mut assets_out: Vec<Value> = Vec::new();
    if let Some(arr) = manifest.get("assets").and_then(|v| v.as_array()).cloned() {
        for (i, item) in arr.into_iter().enumerate() {
            if !item.is_object() {
                continue;
            }
            let mut item = item;
            let aid = item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let aid = if aid.is_empty() {
                format!("asset{}", i)
            } else {
                aid
            };
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(&aid)
                .to_string();
            let mime = item
                .get("mime")
                .and_then(|v| v.as_str())
                .unwrap_or("image/png")
                .to_string();
            let field = item
                .get("field")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("asset_{}", i));

            let mut saved = false;
            if let Some(up) = parts.iter().find(|p| p.name == field)
                && up.filename.is_some()
            {
                let ext = asset_ext(up.filename.as_deref(), &mime);
                let rel = save_shared_bytes(
                    &up.data,
                    files::SHARED_ASSET_DIR,
                    "output/shared/assets",
                    &ext,
                );
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("id".to_string(), Value::String(aid.clone()));
                    obj.insert("name".to_string(), Value::String(name.clone()));
                    obj.insert(
                        "mime".to_string(),
                        Value::String(up.content_type.clone().unwrap_or(mime.clone())),
                    );
                    obj.insert("file".to_string(), Value::String(rel));
                }
                assets_out.push(item.clone());
                saved = true;
            }
            if saved {
                continue;
            }
            if let Some(rel) = existing_output_file(item.get("file").unwrap_or(&Value::Null)) {
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("id".to_string(), Value::String(aid.clone()));
                    obj.insert("name".to_string(), Value::String(name.clone()));
                    obj.insert("mime".to_string(), Value::String(mime.clone()));
                    obj.insert("file".to_string(), Value::String(rel));
                }
                assets_out.push(item.clone());
            }
        }
    }

    // masks
    let mut masks_out: Vec<Value> = Vec::new();
    if let Some(arr) = manifest.get("masks").and_then(|v| v.as_array()).cloned() {
        for (i, item) in arr.into_iter().enumerate() {
            if !item.is_object() {
                continue;
            }
            let mut item = item;
            let node_id = item
                .get("nodeId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let node_id = if node_id.is_empty() {
                format!("node{}", i)
            } else {
                node_id
            };
            let field = item
                .get("field")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("mask_{}", i));

            let mut saved = false;
            if let Some(up) = parts.iter().find(|p| p.name == field)
                && up.filename.is_some()
            {
                let rel = save_shared_bytes(
                    &up.data,
                    files::SHARED_MASK_DIR,
                    "output/shared/masks",
                    ".png",
                );
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("nodeId".to_string(), Value::String(node_id.clone()));
                    obj.insert("file".to_string(), Value::String(rel));
                }
                masks_out.push(item.clone());
                saved = true;
            }
            if saved {
                continue;
            }
            if let Some(rel) = existing_output_file(item.get("file").unwrap_or(&Value::Null)) {
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("nodeId".to_string(), Value::String(node_id.clone()));
                    obj.insert("file".to_string(), Value::String(rel));
                }
                masks_out.push(item.clone());
            }
        }
    }

    // restores
    let mut restores_out: Vec<Value> = Vec::new();
    if let Some(arr) = manifest.get("restores").and_then(|v| v.as_array()).cloned() {
        for (i, item) in arr.into_iter().enumerate() {
            if !item.is_object() {
                continue;
            }
            let mut item = item;
            let node_id = item
                .get("nodeId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let node_id = if node_id.is_empty() {
                format!("node{}", i)
            } else {
                node_id
            };
            let strength = item.get("strength").cloned().unwrap_or(Value::Null);
            let only_center = item
                .get("onlyCenter")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let field = item
                .get("field")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("restore_{}", i));

            let mut saved = false;
            if let Some(up) = parts.iter().find(|p| p.name == field)
                && up.filename.is_some()
            {
                let rel = save_shared_bytes(
                    &up.data,
                    files::SHARED_RESTORE_DIR,
                    "output/shared/restores",
                    ".png",
                );
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("nodeId".to_string(), Value::String(node_id.clone()));
                    obj.insert("strength".to_string(), strength.clone());
                    obj.insert("onlyCenter".to_string(), Value::Bool(only_center));
                    obj.insert("file".to_string(), Value::String(rel));
                }
                restores_out.push(item.clone());
                saved = true;
            }
            if saved {
                continue;
            }
            if let Some(rel) = existing_output_file(item.get("file").unwrap_or(&Value::Null)) {
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("nodeId".to_string(), Value::String(node_id.clone()));
                    obj.insert("strength".to_string(), strength.clone());
                    obj.insert("onlyCenter".to_string(), Value::Bool(only_center));
                    obj.insert("file".to_string(), Value::String(rel));
                }
                restores_out.push(item.clone());
            }
        }
    }

    // thumb
    let mut thumb_value = manifest.get("thumb").cloned().unwrap_or(Value::Null);
    if let Some(up) = parts.iter().find(|p| p.name == "thumb")
        && up.filename.is_some()
    {
        let rel = save_shared_bytes(
            &up.data,
            files::SHARED_THUMB_DIR,
            "output/shared/thumbs",
            ".png",
        );
        thumb_value = Value::String(rel);
    } else {
        if let Some(rel) = existing_output_file(manifest.get("thumb").unwrap_or(&Value::Null)) {
            thumb_value = Value::String(rel);
        } else {
            let legacy = _root.join("thumb.png");
            if legacy.is_file()
                && let Ok(data) = fs::read(&legacy)
            {
                let rel = save_shared_bytes(
                    &data,
                    files::SHARED_THUMB_DIR,
                    "output/shared/thumbs",
                    ".png",
                );
                thumb_value = Value::String(rel);
            }
        }
    }

    let name = manifest
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let name = sanitize_name(&name);

    let now = now_iso();
    let created = old
        .and_then(|o| o.get("createdAt").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| now.clone());

    if let Some(obj) = manifest.as_object_mut() {
        obj.insert("id".to_string(), Value::String(sid.to_string()));
        obj.insert("name".to_string(), Value::String(name));
        obj.insert("assets".to_string(), Value::Array(assets_out));
        obj.insert("masks".to_string(), Value::Array(masks_out));
        obj.insert("restores".to_string(), Value::Array(restores_out));
        obj.insert("thumb".to_string(), thumb_value);
        obj.insert("createdAt".to_string(), Value::String(created));
        obj.insert("updatedAt".to_string(), Value::String(now));
        obj.entry("version".to_string())
            .or_insert(Value::Number(1.into()));
    }

    // write to tmp
    fs::create_dir_all(tmp).ok();
    write_manifest(tmp, &manifest)?;

    Ok(workspace_meta(kind, sid, &manifest))
}
