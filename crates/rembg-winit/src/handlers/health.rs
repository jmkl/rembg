//! `GET /api/health` and `POST /api/unload`.

use anyhow::Result;
use http::Response;
use std::path::Path;

use crate::transport::json_response;

pub fn get() -> Result<Response<Vec<u8>>> {
    // Models are loaded per request, so nothing stays resident between calls.
    let body = serde_json::json!({
        "ok": true,
        "model": "BiRefNet",
        "device": "cuda",
        "loaded": false,
        "status": "unloaded",
        "face_model": "GFPGANv1.4.onnx",
        "face_device": "cuda",
        "face_loaded": false,
        "face_status": "unloaded",
        "face_available": Path::new("models/GFPGANv1.4.onnx").is_file(),
    });
    Ok(json_response(&body))
}

pub fn unload() -> Result<Response<Vec<u8>>> {
    let body = serde_json::json!({
        "unloaded": true,
        "status": "unloaded",
        "face_unloaded": true,
        "face_status": "unloaded",
    });
    Ok(json_response(&body))
}
