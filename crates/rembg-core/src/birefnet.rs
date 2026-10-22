use anyhow::Result;
use image::{DynamicImage, RgbaImage, imageops::FilterType};
use ort::{session::Session, value::Tensor};
use std::{path::Path, sync::Mutex, time::Instant};

use crate::config;

pub struct BirefNet {
    session: Mutex<Session>,
}

impl BirefNet {
    pub fn load() -> Result<Self> {
        Self::load_from(config::MODEL_BIREFNET)
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self> {
        let mut session =
            config::load_session(path, config::VRAM_LIMIT_BIREFNET_MB)?;
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
        let total = Instant::now();

        let mut img = img.clone();
        if img.width() > config::MAX_WIDTH {
            let h = (img.height() as f32 * config::MAX_WIDTH as f32 / img.width() as f32).round()
                as u32;
            img = img.resize_exact(config::MAX_WIDTH, h, FilterType::Lanczos3);
        }
        let (w, h) = (img.width(), img.height());

        let small = img
            .resize_exact(
                config::SIZE_BIREFNET,
                config::SIZE_BIREFNET,
                FilterType::Triangle,
            )
            .to_rgb8();
        let data = config::to_chw(&small, config::MEAN_IMAGENET, config::STD_IMAGENET);
        drop(small);
        let input = Tensor::from_array((
            [
                1usize,
                3,
                config::SIZE_BIREFNET as usize,
                config::SIZE_BIREFNET as usize,
            ],
            data,
        ))?;

        let mask_small = {
            let mut session = self.session.lock().map_err(config::lock_err)?;
            let outputs = session.run(ort::inputs![input])?;
            let (_shape, out) = outputs[0].try_extract_tensor::<f32>()?;
            config::mask_from_logits(out)?
        };

        let mask = image::imageops::resize(&mask_small, w, h, FilterType::Triangle);
        drop(mask_small);

        let mut result = img.into_rgba8();
        for (p, m) in result.pixels_mut().zip(mask.pixels()) {
            p[3] = m[0];
        }
        let _ = total;
        Ok(result)
    }
}
