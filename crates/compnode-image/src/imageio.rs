//! Decoding and encoding through the `image` crate (feature `image-io`).
use crate::buffer::ImageBuf;
use crate::loader::{ExportFormat, ImageLoader};
use compnode_core::color::linear_to_srgb;
use image::ExtendedColorType;
use std::io::BufWriter;

/// Decodes every format the `image` crate was built with (PNG, JPEG, WebP).
pub struct ImageCrateLoader;

impl ImageLoader for ImageCrateLoader {
    fn load(&self, path: &str) -> Result<ImageBuf, String> {
        let img = image::open(path).map_err(|e| format!("{path}: {e}"))?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        ImageBuf::from_srgb8_rgba(w, h, rgba.as_raw())
            .ok_or_else(|| format!("{path}: bad pixel data"))
    }
}

/// Opaque sRGB8 RGB bytes, with transparency composited over black.
fn rgb_over_black(img: &ImageBuf) -> Vec<u8> {
    let enc = |l: f32| (linear_to_srgb(l.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
    let mut out = Vec::with_capacity(img.data.len() * 3);
    for p in &img.data {
        out.extend_from_slice(&[enc(p[0]), enc(p[1]), enc(p[2])]);
    }
    out
}

/// Write JPEG (`quality` 1..=100; no alpha, composited over black) or
/// lossless WebP (keeps alpha; `quality` is ignored).
pub fn export_with_image_crate(
    img: &ImageBuf,
    path: &str,
    format: ExportFormat,
    quality: u8,
) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let writer = BufWriter::new(file);
    let err = |e: image::ImageError| format!("{path}: {e}");
    match format {
        ExportFormat::Jpeg => {
            let mut enc =
                image::codecs::jpeg::JpegEncoder::new_with_quality(writer, quality.clamp(1, 100));
            enc.encode(
                &rgb_over_black(img),
                img.width,
                img.height,
                ExtendedColorType::Rgb8,
            )
            .map_err(err)
        }
        ExportFormat::WebP => {
            let enc = image::codecs::webp::WebPEncoder::new_lossless(writer);
            enc.encode(
                &img.to_srgb8_rgba(),
                img.width,
                img.height,
                ExtendedColorType::Rgba8,
            )
            .map_err(err)
        }
        ExportFormat::Png | ExportFormat::Ppm => {
            Err("use loader::export for PNG and PPM".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::export;
    use compnode_core::Color;

    fn sample() -> ImageBuf {
        ImageBuf::from_fn(16, 8, |x, y| {
            let c = Color::from_srgb8((x * 16) as u8, (y * 32) as u8, 128, 255);
            [c.r, c.g, c.b, 1.0]
        })
    }

    #[test]
    fn our_png_encoder_is_readable_by_the_image_crate() {
        let img = sample();
        let dir = std::env::temp_dir().join(format!("compnode-imageio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.png");
        export(&img, p.to_str().unwrap(), ExportFormat::Png).unwrap();
        let back = ImageCrateLoader.load(p.to_str().unwrap()).unwrap();
        assert!(img.max_abs_diff(&back).unwrap() < 2e-3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn webp_is_lossless_and_jpeg_is_close() {
        let img = sample();
        let dir = std::env::temp_dir().join(format!("compnode-imageio2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let w = dir.join("a.webp");
        export(&img, w.to_str().unwrap(), ExportFormat::WebP).unwrap();
        let back = ImageCrateLoader.load(w.to_str().unwrap()).unwrap();
        assert!(img.max_abs_diff(&back).unwrap() < 2e-3);
        let j = dir.join("a.jpg");
        export(&img, j.to_str().unwrap(), ExportFormat::Jpeg).unwrap();
        let back = ImageCrateLoader.load(j.to_str().unwrap()).unwrap();
        assert_eq!((back.width, back.height), (16, 8));
        let mean: f32 = img
            .data
            .iter()
            .zip(&back.data)
            .map(|(a, b)| (a[0] - b[0]).abs())
            .sum::<f32>()
            / img.data.len() as f32;
        assert!(mean < 0.05, "mean error {mean}");
        assert!(export_with_image_crate(&img, j.to_str().unwrap(), ExportFormat::Png, 90).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
