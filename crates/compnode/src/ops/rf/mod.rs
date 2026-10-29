//! Raw-style development controls (white balance, texture/clarity, dehaze,
//! sharpening, noise reduction).
//!
//! Two interchangeable backends:
//!   * `cpu` - rayon, f32 working buffer, always available.
//!   * `gpu` - wgpu compute shaders, behind the `gpu` cargo feature.
//!
//! `raw_filter` tries the GPU first (when enabled and the image is big enough
//! to be worth the upload) and falls back to the CPU path.

mod cpu;
#[cfg(feature = "gpu")]
mod gpu;

use image::RgbaImage;

use crate::ops::math::luma709;
use crate::params::{self, Params};

/// Below this pixel count the upload/readback overhead beats the GPU win.
#[cfg(feature = "gpu")]
const GPU_MIN_PIXELS: u64 = 512 * 512;

#[cfg(feature = "gpu")]
#[allow(dead_code)]
fn min_pixels() -> u64 {
    std::env::var("RAW_FILTER_GPU_MIN_PIXELS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(128 * 128)
}

/// Clamped slider values plus every derived constant, so the CPU and GPU
/// backends cannot drift apart.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Settings {
    pub temp: f32,
    pub tint: f32,
    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub sharpen: f32,
    pub sharpen_radius: f32,
    pub sharpen_detail: f32,
    pub noise_reduction: f32,
    pub noise_reduction_detail: f32,
}

impl Settings {
    pub fn from_params(p: &Params) -> Self {
        Self {
            temp: params::f32(p, "temp", 0.0).clamp(-100.0, 100.0),
            tint: params::f32(p, "tint", 0.0).clamp(-100.0, 100.0),
            texture: params::f32(p, "texture", 0.0).clamp(-100.0, 100.0),
            clarity: params::f32(p, "clarity", 0.0).clamp(-100.0, 100.0),
            dehaze: params::f32(p, "dehaze", 0.0).clamp(-100.0, 100.0),
            sharpen: params::f32(p, "sharpen", 0.0).clamp(0.0, 150.0),
            sharpen_radius: params::f32(p, "sharpen_radius", 1.0),
            sharpen_detail: params::f32(p, "sharpen_detail", 25.0).clamp(0.0, 100.0),
            noise_reduction: params::f32(p, "noise_reduction", 0.0).clamp(0.0, 100.0),
            noise_reduction_detail: params::f32(p, "noise_reduction_detail", 50.0)
                .clamp(0.0, 100.0),
        }
    }

    pub fn is_identity(&self) -> bool {
        self.temp == 0.0
            && self.tint == 0.0
            && self.texture == 0.0
            && self.clarity == 0.0
            && self.dehaze == 0.0
            && self.sharpen == 0.0
            && self.noise_reduction == 0.0
    }

    pub fn wb_active(&self) -> bool {
        self.temp != 0.0 || self.tint != 0.0
    }

    /// Per-channel gains, already divided by their luma so brightness is stable.
    pub fn wb_gains(&self) -> [f32; 3] {
        let temp = (self.temp / 100.0).clamp(-1.0, 1.0);
        let tint = (self.tint / 100.0).clamp(-1.0, 1.0);
        let g = [
            (1.0 + 0.35 * temp + 0.10 * tint).max(0.01),
            (1.0 - 0.25 * tint).max(0.01),
            (1.0 - 0.35 * temp + 0.10 * tint).max(0.01),
        ];
        let norm = luma709(g).max(0.01);
        [g[0] / norm, g[1] / norm, g[2] / norm]
    }

    pub fn dehaze_amount(&self) -> f32 {
        (self.dehaze / 100.0).clamp(-1.0, 1.0)
    }
    pub fn texture_amount(&self) -> f32 {
        self.texture / 100.0 * 0.9
    }
    pub fn clarity_amount(&self) -> f32 {
        self.clarity / 100.0 * 0.7
    }
    pub fn sharpen_strength(&self) -> f32 {
        (self.sharpen / 100.0).clamp(0.0, 1.5) * 1.3
    }
    pub fn sharpen_threshold(&self) -> f32 {
        0.001 + (1.0 - (self.sharpen_detail / 100.0).clamp(0.0, 1.0)).powi(2) * 0.08
    }
    pub fn sharpen_sigma(&self) -> f32 {
        self.sharpen_radius.clamp(0.5, 3.0)
    }
    pub fn nr_amount(&self) -> f32 {
        (self.noise_reduction / 100.0).clamp(0.0, 1.0)
    }
    pub fn nr_threshold(&self) -> f32 {
        0.02 + (1.0 - (self.noise_reduction_detail / 100.0).clamp(0.0, 1.0)) * 0.14
    }
}

#[cfg(feature = "gpu")]
fn try_gpu(src: &RgbaImage, s: &Settings) -> Option<RgbaImage> {
    if (src.width() as u64) * (src.height() as u64) < GPU_MIN_PIXELS {
        // eprintln!("[raw_filter gpu] skipped: image below GPU_MIN_PIXELS");
        return None;
    }
    // println!("Processing raw filter with gpu");
    gpu::apply(src, s)
}

#[cfg(not(feature = "gpu"))]
fn try_gpu(_: &RgbaImage, _: &Settings) -> Option<RgbaImage> {
    None
}

pub(crate) fn raw_filter(src: &RgbaImage, params: &Params) -> RgbaImage {
    let settings = Settings::from_params(params);
    if settings.is_identity() || src.width() == 0 || src.height() == 0 {
        return src.clone();
    }

    let _t = std::time::Instant::now();
    let (result, _backend) = match try_gpu(src, &settings) {
        Some(out) => (out, "gpu"),
        None => (cpu::apply(src, &settings), "cpu"),
    };
    // eprintln!(
    //     "[raw_filter] {backend} {}x{} in {:?}",
    //     src.width(),
    //     src.height(),
    //     t.elapsed()
    // );
    result
}
