//! `GET /api/fetch?url=` — download an image server-side and re-encode it for
//! a reliable browser preview.

use anyhow::{Result, anyhow, bail};
use image::{DynamicImage, imageops::FilterType};

use crate::transport::{ParsedRequest, bytes_response};

const MAX_DOWNLOAD_BYTES: u64 = 30 * 1024 * 1024;
const MAX_PREVIEW_SIDE: u32 = 4096;

pub fn get(req: &ParsedRequest) -> Result<http::Response<Vec<u8>>> {
    let url = req.query.get("url").cloned().unwrap_or_default();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        bail!("URL must start with http:// or https://");
    }

    let mut response = ureq::get(&url)
        .header("User-Agent", "Mozilla/5.0 bgremover")
        .call()
        .map_err(|e| anyhow!("could not download image: {e}"))?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD_BYTES)
        .read_to_vec()
        .map_err(|e| anyhow!("could not download image: {e}"))?;

    let mut img = image::load_from_memory(&bytes)
        .map_err(|e| anyhow!("could not decode that URL as an image: {e}"))?;
    if img.width().max(img.height()) > MAX_PREVIEW_SIDE {
        img = img.resize(MAX_PREVIEW_SIDE, MAX_PREVIEW_SIDE, FilterType::Lanczos3);
    }

    let mut buf = Vec::new();
    let has_alpha = img.color().has_alpha();
    if has_alpha {
        DynamicImage::ImageRgba8(img.to_rgba8())
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)?;
        Ok(bytes_response(buf, "image/png"))
    } else {
        DynamicImage::ImageRgb8(img.to_rgb8()).write_to(
            &mut std::io::Cursor::new(&mut buf),
            image::ImageFormat::Jpeg,
        )?;
        Ok(bytes_response(buf, "image/jpeg"))
    }
}
