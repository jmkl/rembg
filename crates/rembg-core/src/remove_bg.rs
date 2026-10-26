use anyhow::{Context, Result};
use image::{DynamicImage, GrayImage, Rgba, RgbaImage, imageops::FilterType};
use ort::{session::Session, value::Tensor};
use std::{path::Path, sync::Mutex, time::Instant};

use crate::config;

pub struct RemoveBg {
    session: Mutex<Session>,
}

impl RemoveBg {
    pub fn load() -> Result<Self> {
        Self::load_from(config::MODEL_RMBG)
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self> {
        let mut session = config::load_session(path, config::VRAM_LIMIT_BIREFNET_MB)?;
        if config::WARMUP {
            config::warmup(&mut session, config::SIZE_BIREFNET)?;
        }
        Ok(Self {
            session: Mutex::new(session),
        })
    }

    pub fn process(&self, path: &Path) -> Result<RgbaImage> {
        let img = image::open(path)?;
        self.process_image(&img)
    }

    pub fn process_image(&self, img: &DynamicImage) -> Result<RgbaImage> {
        let _total = Instant::now();

        let (orig_w, orig_h) = (img.width(), img.height());

        // 1. Resize to model input size (1024x1024) - no max-width clamp
        // (RMBG-1.4 processes full-resolution images at fixed 1024x1024 input)
        let resized = img
            .resize_exact(
                config::SIZE_BIREFNET,
                config::SIZE_BIREFNET,
                FilterType::Triangle,
            )
            .to_rgb8();

        // 2. Input tensor [1, 3, H, W], normalization: (x/255 - 0.5) / 1.0
        let s = config::SIZE_BIREFNET as usize;
        let plane = s * s;
        let mut input_data = vec![0.0f32; 3 * plane];

        for (x, y, p) in resized.enumerate_pixels() {
            let idx = y as usize * s + x as usize;
            input_data[idx] = p[0] as f32 / 255.0 - 0.5;
            input_data[plane + idx] = p[1] as f32 / 255.0 - 0.5;
            input_data[2 * plane + idx] = p[2] as f32 / 255.0 - 0.5;
        }

        // 3. Run inference
        let input = Tensor::from_array(([1usize, 3, s, s], input_data))?;

        let mask_small = {
            let mut session = self.session.lock().map_err(config::lock_err)?;
            let outputs = session.run(ort::inputs![input])?;
            let (_shape, mask_slice) = outputs[0].try_extract_tensor::<f32>()?;

            if mask_slice.len() < plane {
                anyhow::bail!("Unexpected mask size: got {} values, expected {}", mask_slice.len(), plane);
            }
            let mask_slice = &mask_slice[..plane];

            // 4. Min-max normalize (as in the reference RMBG-1.4 pipeline), then to u8
            let (min, max) = mask_slice
                .iter()
                .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
            let range = (max - min).max(1e-6);

            let mask_raw: Vec<u8> = mask_slice
                .iter()
                .map(|&v| (((v - min) / range).clamp(0.0, 1.0) * 255.0).round() as u8)
                .collect();

            GrayImage::from_raw(config::SIZE_BIREFNET, config::SIZE_BIREFNET, mask_raw)
                .context("Failed to create mask image")?
        };

        // 5. Resize mask back to original dimensions with CatmullRom
        let mask = image::imageops::resize(&mask_small, orig_w, orig_h, FilterType::CatmullRom);

        // Apply mask to alpha
        let mut result = img.to_rgba8();
        for (p, m) in result.pixels_mut().zip(mask.pixels()) {
            let m = m[0] as f32 / 255.0;
            *p = Rgba([p[0], p[1], p[2], (p[3] as f32 * m).round() as u8]);
        }

        Ok(result)
    }

    pub fn warmup(&self) -> Result<()> {
        let mut session = self.session.lock().map_err(config::lock_err)?;
        config::warmup(&mut session, config::SIZE_BIREFNET)
    }
}