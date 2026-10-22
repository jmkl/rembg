//! `POST /api/restore` — GFPGAN face restoration.
//!
//! Request: `{ image, strength (0..1), only_center, name, max_side }`.
//! Response: PNG, input resolution, alpha channel preserved.

use anyhow::{Result, anyhow};
use image::DynamicImage;
use serde::Deserialize;

use crate::{
    transport::{INFERENCE_LOCK, ParsedRequest, bytes_response},
    util,
};
use rembg_core::{FaceDetector, Gfpgan, Landmarks, pick_center};

#[derive(Deserialize)]
struct RestoreRequest {
    image: String,
    #[serde(default = "default_strength")]
    strength: f32,
    #[serde(default)]
    only_center: bool,
    #[serde(default)]
    #[allow(dead_code)]
    name: String,
    #[serde(default)]
    max_side: u32,
}

fn default_strength() -> f32 {
    1.0
}

pub fn post(req: &ParsedRequest) -> Result<http::Response<Vec<u8>>> {
    let payload: RestoreRequest =
        serde_json::from_slice(req.body).map_err(|e| anyhow!("invalid request body: {e}"))?;

    let bytes = util::b64_decode(&payload.image)?;
    let img =
        image::load_from_memory(&bytes).map_err(|e| anyhow!("could not read that image: {e}"))?;
    let img = if payload.max_side > 0 && img.width().max(img.height()) > payload.max_side {
        img.resize(
            payload.max_side,
            payload.max_side,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };

    let (w, h) = (img.width(), img.height());
    let has_alpha = img.color().has_alpha();
    let alpha = img.to_rgba8();
    let rgb = img.to_rgb8();

    // Load per request; the lock keeps a second inference from overlapping.
    let _guard = INFERENCE_LOCK
        .lock()
        .map_err(|_| anyhow!("inference lock poisoned"))?;
    let model = Gfpgan::load()?;
    let detector = FaceDetector::load()?;
    let faces = detector.detect(&rgb, 0.5, 0.4)?;
    let faces = if payload.only_center {
        pick_center(faces, w, h)
    } else {
        faces
    };
    let landmarks: Vec<Landmarks> = faces.iter().map(|f| f.landmarks).collect();
    let restored = model.restore_faces(&rgb, &landmarks, payload.strength)?;
    drop(detector);
    drop(model);
    drop(_guard);

    let mut out = DynamicImage::ImageRgb8(restored).to_rgba8();
    if has_alpha {
        for (p, a) in out.pixels_mut().zip(alpha.pixels()) {
            p[3] = a[3];
        }
    }

    let mut buf = Vec::new();
    DynamicImage::ImageRgba8(out)
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)?;

    Ok(bytes_response(buf, "image/png"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ParsedRequest;
    use base64::Engine;
    use std::collections::HashMap;

    /// Runs the real GFPGAN + SCRFD models. Ignored by default because it needs
    /// the weights and a GPU.
    #[test]
    #[ignore = "loads GFPGAN + SCRFD; run manually"]
    fn restore_handler_end_to_end() {
        let mut img = image::RgbaImage::new(256, 256);
        for (i, p) in img.pixels_mut().enumerate() {
            *p = image::Rgba([120, 90, 70, if i % 2 == 0 { 255 } else { 0 }]);
        }
        let mut png = Vec::new();
        DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();

        let body = serde_json::json!({
            "image": base64::engine::general_purpose::STANDARD.encode(&png),
            "strength": 1.0,
            "only_center": false,
            "name": "test",
            "max_side": 0,
        })
        .to_string();

        let req = ParsedRequest {
            method: "POST",
            path: "/api/restore",
            query: HashMap::new(),
            body: body.as_bytes(),
        };
        let resp = post(&req).expect("restore handler failed");
        assert_eq!(resp.status(), http::StatusCode::OK);

        let out = image::load_from_memory(resp.body()).expect("response is not a PNG");
        assert_eq!(out.width(), 256);
        assert_eq!(out.height(), 256);
        assert!(out.color().has_alpha());
    }
}
