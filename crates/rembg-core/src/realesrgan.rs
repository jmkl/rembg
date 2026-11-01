//! Real-ESRGAN x2plus (RRDBNet, 23 blocks) super-resolution on `ort = "=2.0.0-rc.12"`.
//!
//! Model: skillsafe-ai/realesrgan-x2plus (fp32), derived from
//! xinntao/Real-ESRGAN RealESRGAN_x2plus.pth (v0.2.1).
//!
//! I/O contract (from the model card / manifest):
//!   input  `input`  : [1, 3, H, W] float32, RGB, values 0..1, H and W even.
//!   output `output` : [1, 3, H*2, W*2] float32, RGB, values ~0..1.
//!
//! Cargo.toml:
//!   anyhow = "1"
//!   image  = "0.25"
//!   ort    = "=2.0.0-rc.12"

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Result, bail};
use image::RgbImage;
use ort::{session::Session, value::Tensor};

use crate::config::{self, load_session, lock_err};

pub const SCALE: u32 = 2;

pub struct Realesrgan {
    session: Mutex<Session>,
}

impl Realesrgan {
    pub fn load() -> Result<Self> {
        Self::load_from(config::MODEL_REALESRGAN)
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self> {
        let session = load_session(path, config::VRAM_LIMIT_REALESRGAN_MB)?;
        Ok(Self {
            session: Mutex::new(session),
        })
    }

    /// Upscale an RGB image by 2x. Pads the input to even dimensions (edge
    /// replication) when needed and crops the result back to the exact 2x size.
    pub fn upscale(&self, img: &RgbImage) -> Result<RgbImage> {
        let (w, h) = (img.width(), img.height());
        if w == 0 || h == 0 {
            bail!("empty input image");
        }

        // Pad to even dimensions (required by the model).
        let pw = if w % 2 == 0 { w } else { w + 1 };
        let ph = if h % 2 == 0 { h } else { h + 1 };

        // Build CHW f32 input in [0, 1], replicating edge pixels into padding.
        let n = (pw * ph) as usize;
        let mut data = vec![0f32; 3 * n];
        for y in 0..ph {
            let sy = y.min(h - 1);
            for x in 0..pw {
                let sx = x.min(w - 1);
                let p = img.get_pixel(sx, sy);
                let idx = (y * pw + x) as usize;
                data[idx] = p[0] as f32 / 255.0;
                data[n + idx] = p[1] as f32 / 255.0;
                data[2 * n + idx] = p[2] as f32 / 255.0;
            }
        }

        let input = Tensor::from_array(([1usize, 3, ph as usize, pw as usize], data))?;

        let mut session = self.session.lock().map_err(lock_err)?;
        let outputs = session.run(ort::inputs![input])?;
        let (shape, out) = outputs[0].try_extract_tensor::<f32>()?;

        if shape.len() != 4 || shape[0] != 1 || shape[1] != 3 {
            bail!("Unexpected RealESRGAN output shape: {shape:?}");
        }
        let out_h = shape[2] as usize;
        let out_w = shape[3] as usize;
        if out.len() < 3 * out_h * out_w {
            bail!("RealESRGAN output tensor is too small");
        }
        if out_h < (h as usize * 2) || out_w < (w as usize * 2) {
            bail!(
                "RealESRGAN output {}x{} smaller than expected {}x{}",
                out_w,
                out_h,
                w * 2,
                h * 2
            );
        }

        // CHW f32 [0,1] -> RGB HWC u8, cropping to the exact 2x size.
        let mut result = RgbImage::new(w * 2, h * 2);
        let plane = out_h * out_w;
        for y in 0..(h * 2) {
            for x in 0..(w * 2) {
                let i = y as usize * out_w as usize + x as usize;
                let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                let px = image::Rgb([
                    to_u8(out[i]),
                    to_u8(out[plane + i]),
                    to_u8(out[2 * plane + i]),
                ]);
                result.put_pixel(x, y, px);
            }
        }

        Ok(result)
    }
}


