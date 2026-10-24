//! Transport layer for the desktop app.
//!
//! The webview loads the UI from a custom protocol (`app://`), so every request
//! arrives here as an `http::Request` with the full method, path, headers and
//! body. `dispatch` mirrors the old FastAPI route table one-for-one.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use anyhow::{Result, anyhow};
use http::{Response, StatusCode, header::CONTENT_TYPE};
use serde::Serialize;

use crate::{handlers, util};

/// Models are loaded per request. This serialises inference so two requests can
/// never double VRAM at the same time.
pub static INFERENCE_LOCK: Mutex<()> = Mutex::new(());

/// A request split into the parts the handlers need.
pub struct ParsedRequest<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: HashMap<String, String>,
    pub body: &'a [u8],
}

/// Route a request to its handler. Handler errors become a 500 response so the
/// responder is always called exactly once.
pub fn dispatch(request: &http::Request<Vec<u8>>) -> Response<Vec<u8>> {
    try_dispatch(request).unwrap_or_else(|e| internal_error(&e))
}

fn try_dispatch(request: &http::Request<Vec<u8>>) -> Result<Response<Vec<u8>>> {
    let method = request.method().as_str();
    let uri = request.uri();
    let path = uri.path();
    let query = parse_query(uri.query().unwrap_or(""));

    let parsed = ParsedRequest {
        method,
        path,
        query,
        body: request.body(),
    };

    // Workspaces have their own routing (kind + optional id).
    if parsed.path.starts_with("/api/workspaces") {
        let rest = parsed.path.trim_start_matches("/api/workspaces/");
        let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        match (parsed.method, segments.as_slice()) {
            ("GET", [kind]) => return handlers::workspaces::list(kind),
            ("POST", [_kind]) => return handlers::workspaces::create(&parsed),
            ("GET", [kind, id]) => return handlers::workspaces::get_one(kind, id),
            ("PUT", [kind, id]) => return handlers::workspaces::update(&parsed, kind, id),
            ("PATCH", [kind, id]) => return handlers::workspaces::rename(&parsed, kind, id),
            ("DELETE", [kind, id]) => return handlers::workspaces::delete_one(kind, id),
            _ => return handlers::workspaces::not_implemented(),
        }
    }

    match (parsed.method, parsed.path) {
        ("GET", "/api/health") => handlers::health::get(),
        ("POST", "/api/unload") => handlers::health::unload(),
        ("POST", "/api/mask") => handlers::mask::post(&parsed),
        ("POST", "/api/restore") => handlers::restore::post(&parsed),
        ("GET", "/api/fetch") => handlers::fetch::get(&parsed),
        ("GET", "/api/files") => handlers::files::get(),
        ("GET", p)
            if p.starts_with("/input/")
                || p.starts_with("/output/")
                || p.starts_with("/workspaces/") =>
        {
            handlers::files::serve(parsed.method, p)
        }
        _ => serve_asset(request),
    }
}

fn parse_query(query: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        map.insert(util::percent_decode(key), util::percent_decode(value));
    }
    map
}

/// Serve UI assets. Prefer the built `frontend/dist` (static, no HMR/WebSocket)
/// when present; fall back to the Vite dev server for development.
fn serve_asset(request: &http::Request<Vec<u8>>) -> Result<Response<Vec<u8>>> {
    let path = request.uri().path();
    let dist = std::path::Path::new("frontend/dist");
    if dist.is_dir() {
        // Map URL path to a file under frontend/dist.
        let rel = path.trim_start_matches('/');
        let rel = if rel.is_empty() { "index.html" } else { rel };
        let file = dist.join(rel);
        // SPA fallback: unknown paths serve index.html.
        let file = if file.is_file() {
            file
        } else if !rel.contains('.') {
            dist.join("index.html")
        } else {
            file
        };
        if file.is_file() {
            let bytes = std::fs::read(&file).map_err(|e| anyhow!("read asset: {e}"))?;
            let ct = util::content_type_for(file.to_str().unwrap_or(""));
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, ct)
                .body(bytes)
                .expect("valid response"));
        }
    }

    // Dev fallback: proxy to Vite.
    let url = format!("http://localhost:5173{path}");
    let mut response = vite_agent()
        .get(&url)
        .call()
        .map_err(|e| anyhow!("dev proxy failed for {path}: {e}"))?;
    let status = response.status().as_u16();
    if status == 404 {
        eprintln!("dev proxy 404: {}", url);
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let body = response
        .body_mut()
        .read_to_vec()
        .map_err(|e| anyhow!("dev proxy body for {path}: {e}"))?;

    Ok(Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(body)
        .expect("valid response"))
}

/// Shared HTTP client for the vite dev proxy (plain localhost HTTP only).
fn vite_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build();
        config.new_agent()
    })
}

// ------------------------------------------------------------- responses ---

pub fn bytes_response(bytes: Vec<u8>, content_type: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, content_type)
        .body(bytes)
        .expect("valid response")
}

pub fn json_response<T: Serialize>(value: &T) -> Response<Vec<u8>> {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    bytes_response(body, "application/json")
}

pub fn error(status: StatusCode, message: &str) -> Response<Vec<u8>> {
    let body = serde_json::json!({ "detail": message }).to_string();
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(body.into_bytes())
        .expect("valid response")
}

pub fn not_found() -> Response<Vec<u8>> {
    error(StatusCode::NOT_FOUND, "Not found")
}

pub fn method_not_allowed() -> Response<Vec<u8>> {
    error(StatusCode::METHOD_NOT_ALLOWED, "Method not allowed")
}

pub fn internal_error(err: &impl std::fmt::Display) -> Response<Vec<u8>> {
    error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(path: &str) -> Response<Vec<u8>> {
        dispatch(
            &http::Request::builder()
                .method("GET")
                .uri(format!("http://app.localhost{path}"))
                .body(Vec::new())
                .unwrap(),
        )
    }

    #[test]
    fn health_is_json() {
        let resp = get("/api/health");
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let value: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
        assert_eq!(value["ok"], true);
    }

    #[test]
    fn files_is_json() {
        let resp = get("/api/files");
        assert_eq!(resp.status(), StatusCode::OK);
        let value: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
        assert!(value["input"].is_array());
        assert!(value["output"].is_array());
    }

    #[test]
    fn workspaces_list_is_json() {
        let resp = get("/api/workspaces/templates");
        assert_eq!(resp.status(), StatusCode::OK);
        let value: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
        assert!(value.is_array());
    }

    #[test]
    fn workspaces_create_roundtrip() {
        let boundary = "----testboundary";
        let manifest = r#"{"name":"Smoke","graph":{"nodes":[],"edges":[],"params":{}}}"#;
        let body = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"manifest\"\r\n\r\n{m}\r\n--{b}--\r\n",
            b = boundary,
            m = manifest
        );
        let req = http::Request::builder()
            .method("POST")
            .uri("http://app.localhost/api/workspaces/templates")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(body.into_bytes())
            .unwrap();
        let resp = dispatch(&req);
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(resp.body())
        );
        let meta: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
        let id = meta["id"].as_str().unwrap().to_string();
        assert!(!id.is_empty());

        // GET
        let resp = get(&format!("/api/workspaces/templates/{}", id));
        assert_eq!(resp.status(), StatusCode::OK);
        let m: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
        assert_eq!(m["name"], "Smoke");

        // DELETE
        let req = http::Request::builder()
            .method("DELETE")
            .uri(format!(
                "http://app.localhost/api/workspaces/templates/{}",
                id
            ))
            .body(Vec::new())
            .unwrap();
        let resp = dispatch(&req);
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn workspaces_with_asset_roundtrip() {
        // Build a tiny valid PNG.
        let img = image::RgbImage::new(4, 4);
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();

        let boundary = "----assetb";
        // manifest references asset_0
        let manifest = r#"{"name":"WithAsset","graph":{"nodes":[],"edges":[],"params":{}},"assets":[{"id":"a0","name":"t.png","mime":"image/png","field":"asset_0"}]}"#;
        // build multipart with file part
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{b}\r\nContent-Disposition: form-data; name=\"manifest\"\r\n\r\n{m}\r\n",
                b = boundary,
                m = manifest
            )
            .as_bytes(),
        );
        body.extend_from_slice(format!("--{b}\r\nContent-Disposition: form-data; name=\"asset_0\"; filename=\"t.png\"\r\nContent-Type: image/png\r\n\r\n", b=boundary).as_bytes());
        body.extend_from_slice(&png);
        body.extend_from_slice(format!("\r\n--{b}--\r\n", b = boundary).as_bytes());

        let req = http::Request::builder()
            .method("POST")
            .uri("http://app.localhost/api/workspaces/templates")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(body)
            .unwrap();
        let resp = dispatch(&req);
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(resp.body())
        );
        let meta: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
        let id = meta["id"].as_str().unwrap().to_string();

        // GET manifest and verify asset file path
        let resp = get(&format!("/api/workspaces/templates/{}", id));
        assert_eq!(resp.status(), StatusCode::OK);
        let m: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
        let file = m["assets"][0]["file"].as_str().unwrap();
        assert!(file.starts_with("output/shared/assets/"));

        // serve the file
        let resp = get(&format!("/{}", file));
        assert_eq!(resp.status(), StatusCode::OK);

        // rename
        let req = http::Request::builder()
            .method("PATCH")
            .uri(format!(
                "http://app.localhost/api/workspaces/templates/{}",
                id
            ))
            .header("content-type", "application/json")
            .body(br#"{"name":"Renamed"}"#.to_vec())
            .unwrap();
        let resp = dispatch(&req);
        assert_eq!(resp.status(), StatusCode::OK);
        let meta: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
        assert_eq!(meta["name"], "Renamed");

        // cleanup
        let req = http::Request::builder()
            .method("DELETE")
            .uri(format!(
                "http://app.localhost/api/workspaces/templates/{}",
                id
            ))
            .body(Vec::new())
            .unwrap();
        let _ = dispatch(&req);
    }

    #[test]
    fn mask_rejects_a_bad_body() {
        let req = http::Request::builder()
            .method("POST")
            .uri("http://app.localhost/api/mask")
            .header("content-type", "application/json")
            .body(br#"{"image":"!!!not base64!!!"}"#.to_vec())
            .unwrap();
        let resp = dispatch(&req);
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn static_file_is_served() {
        std::fs::create_dir_all("output").unwrap();
        let path = "output/__transport_test.png";
        std::fs::write(path, b"\x89PNG").unwrap();
        let resp = get("/output/__transport_test.png");
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(CONTENT_TYPE).unwrap(), "image/png");
        assert_eq!(resp.body(), b"\x89PNG");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn static_path_traversal_is_blocked() {
        assert_eq!(get("/output/../Cargo.toml").status(), StatusCode::NOT_FOUND);
        assert_eq!(
            get("/output/%2e%2e/Cargo.toml").status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn query_is_decoded() {
        let query = parse_query("url=https%3A%2F%2Fx%2Fy.png&a=1");
        assert_eq!(query.get("url").unwrap(), "https://x/y.png");
        assert_eq!(query.get("a").unwrap(), "1");
    }
}
