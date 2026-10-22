//! `POST /api/mask` — BiRefNet background mask.
//!
//! Request: `{ image: "<base64 or data-url>", name, max_side, max_width }`.
//! Response: white RGB + mask alpha PNG (the browser applies it with
//! `destination-in`).

use anyhow::{Result, anyhow};
use image::{DynamicImage, RgbaImage, imageops::FilterType};
use serde::Deserialize;

use crate::{
    transport::{INFERENCE_LOCK, ParsedRequest, bytes_response},
    util,
};
use rembg_core::BirefNet;

#[derive(Deserialize)]
struct MaskRequest {
    image: String,
    // Kept for API parity; output-file naming is not implemented yet.
    #[serde(default)]
    #[allow(dead_code)]
    name: String,
    #[serde(default)]
    max_side: u32,
    #[serde(default)]
    max_width: u32,
}

pub fn post(req: &ParsedRequest) -> Result<http::Response<Vec<u8>>> {
    let payload: MaskRequest =
        serde_json::from_slice(req.body).map_err(|e| anyhow!("invalid request body: {e}"))?;

    let bytes = util::b64_decode(&payload.image)?;
    let img =
        image::load_from_memory(&bytes).map_err(|e| anyhow!("could not read that image: {e}"))?;
    let img = limit_image(img, payload.max_width, payload.max_side);

    // Load per request; the lock keeps a second inference from overlapping.
    let _guard = INFERENCE_LOCK
        .lock()
        .map_err(|_| anyhow!("inference lock poisoned"))?;
    let model = BirefNet::load()?;
    let cutout = model.process_image(&img)?;
    drop(model);
    drop(_guard);

    // `/api/mask` returns the mask itself, not the cutout: RGB white, alpha = mask.
    let mask = white_mask(cutout);
    let mut buf = Vec::new();
    DynamicImage::ImageRgba8(mask)
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)?;

    Ok(bytes_response(buf, "image/png"))
}

/// Keep the mask alpha, force RGB to white.
fn white_mask(img: RgbaImage) -> RgbaImage {
    let mut img = img;
    for p in img.pixels_mut() {
        p[0] = 255;
        p[1] = 255;
        p[2] = 255;
    }
    img
}

/// Same resize rules as the Python `limit_image` (fit within max_side, then max_width).
fn limit_image(mut img: DynamicImage, max_width: u32, max_side: u32) -> DynamicImage {
    if max_side > 0 && img.width().max(img.height()) > max_side {
        img = img.resize(max_side, max_side, FilterType::Lanczos3);
    }
    if max_width > 0 && img.width() > max_width {
        img = img.resize(max_width, u32::MAX, FilterType::Lanczos3);
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ParsedRequest;
    use base64::Engine;
    use std::collections::HashMap;

    /// Runs the real BiRefNet model. Ignored by default because it needs the
    /// weights and a GPU.
    #[test]
    #[ignore = "loads the BiRefNet model; run manually"]
    fn mask_handler_end_to_end() {
        let mut img = image::RgbImage::new(64, 64);
        for p in img.pixels_mut() {
            *p = image::Rgb([200, 40, 40]);
        }
        let mut png = Vec::new();
        DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();

        let body = serde_json::json!({
            "image": base64::engine::general_purpose::STANDARD.encode(&png),
            "name": "test",
            "max_side": 0,
            "max_width": 0,
        })
        .to_string();

        let req = ParsedRequest {
            method: "POST",
            path: "/api/mask",
            query: HashMap::new(),
            body: body.as_bytes(),
        };
        let resp = post(&req).expect("mask handler failed");
        assert_eq!(resp.status(), http::StatusCode::OK);

        let out = image::load_from_memory(resp.body()).expect("response is not a PNG");
        assert_eq!(out.width(), 64);
        assert_eq!(out.height(), 64);
        assert!(out.color().has_alpha());
    }
}
