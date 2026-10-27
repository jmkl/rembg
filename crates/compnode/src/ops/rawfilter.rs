//! Raw-style development controls.
//!
//! The Raw Filter combines the adjustments usually found in a raw developer:
//! white balance, local contrast (texture/clarity), dehaze, sharpening and
//! noise reduction. Every stage works on the RGB channels and leaves alpha
//! untouched.

use image::RgbaImage;

use crate::params::{self, Params};

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn luma(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

/// Gaussian-ish blur that never bleeds the alpha channel into the colour
/// channels. Transparent pixels are treated as opaque for the blur itself and
/// get their original alpha restored afterwards.
fn blurred(src: &RgbaImage, sigma: f32) -> RgbaImage {
    if src.width() == 0 || src.height() == 0 {
        return src.clone();
    }
    let mut opaque = src.clone();
    for pixel in opaque.pixels_mut() {
        pixel[3] = 255;
    }
    let mut out = image::imageops::fast_blur(&opaque, sigma.max(0.1));
    for (dst, source) in out.pixels_mut().zip(src.pixels()) {
        dst[3] = source[3];
    }
    out
}

fn pixel_rgb(pixel: &image::Rgba<u8>) -> [f32; 3] {
    [
        pixel[0] as f32 / 255.0,
        pixel[1] as f32 / 255.0,
        pixel[2] as f32 / 255.0,
    ]
}

fn store_rgb(pixel: &mut image::Rgba<u8>, rgb: [f32; 3]) {
    for channel in 0..3 {
        pixel[channel] = (clamp01(rgb[channel]) * 255.0).round() as u8;
    }
}

/// Temperature and tint gains. Positive temperature warms the image, positive
/// tint pushes it towards magenta.
fn white_balance(src: &RgbaImage, temp: f32, tint: f32) -> RgbaImage {
    let temp = (temp / 100.0).clamp(-1.0, 1.0);
    let tint = (tint / 100.0).clamp(-1.0, 1.0);

    let gains = [
        (1.0 + 0.35 * temp + 0.10 * tint).max(0.01),
        (1.0 - 0.25 * tint).max(0.01),
        (1.0 - 0.35 * temp + 0.10 * tint).max(0.01),
    ];
    // Keep the overall brightness stable so the sliders only shift colour.
    let norm = luma(gains).max(0.01);

    let mut out = src.clone();
    for pixel in out.pixels_mut() {
        let rgb = pixel_rgb(pixel);
        store_rgb(
            pixel,
            [
                rgb[0] * gains[0] / norm,
                rgb[1] * gains[1] / norm,
                rgb[2] * gains[2] / norm,
            ],
        );
    }
    out
}

/// Positive values remove atmospheric haze, negative values add it back.
fn dehaze(src: &RgbaImage, amount: f32) -> RgbaImage {
    let amount = (amount / 100.0).clamp(-1.0, 1.0);
    let mut out = src.clone();
    for pixel in out.pixels_mut() {
        let mut rgb = pixel_rgb(pixel);

        if amount >= 0.0 {
            // Lift the black point, then add contrast and saturation.
            let black = 0.12 * amount;
            let contrast = 1.0 + 0.30 * amount;
            let saturation = 1.0 + 0.30 * amount;
            for value in &mut rgb {
                *value = (*value - black) / (1.0 - black).max(1e-4);
                *value = (*value - 0.5) * contrast + 0.5;
            }
            let l = luma(rgb);
            for value in &mut rgb {
                *value = l + (*value - l) * saturation;
            }
        } else {
            // Blend towards a cool, bright haze and flatten the contrast.
            let haze = -amount;
            let haze_color = [0.75, 0.80, 0.88];
            for (channel, value) in rgb.iter_mut().enumerate() {
                *value = *value * (1.0 - 0.5 * haze) + haze_color[channel] * 0.5 * haze;
                *value = (*value - 0.5) * (1.0 - 0.20 * haze) + 0.5;
            }
        }

        store_rgb(pixel, rgb);
    }
    out
}

/// Frequency-based local contrast. `midtone_bias` keeps the effect strongest
/// in the midtones (clarity) or spreads it evenly (texture).
fn local_contrast(src: &RgbaImage, sigma: f32, amount: f32, midtone_bias: f32) -> RgbaImage {
    if amount == 0.0 {
        return src.clone();
    }

    let blur = blurred(src, sigma);
    let mut out = src.clone();
    for (pixel, blurred_pixel) in out.pixels_mut().zip(blur.pixels()) {
        let rgb = pixel_rgb(pixel);
        let blurred_rgb = pixel_rgb(blurred_pixel);
        let l = luma(rgb);
        let detail = l - luma(blurred_rgb);
        let midtones = 4.0 * l * (1.0 - l);
        let weight = 1.0 - midtone_bias + midtone_bias * midtones;
        let delta = detail * amount * weight;
        store_rgb(pixel, [rgb[0] + delta, rgb[1] + delta, rgb[2] + delta]);
    }
    out
}

/// Luma unsharp mask. `detail` raises or lowers the edge threshold so fine
/// texture can be excluded when the slider is low.
fn sharpen(src: &RgbaImage, amount: f32, radius: f32, detail: f32) -> RgbaImage {
    if amount <= 0.0 {
        return src.clone();
    }

    let blur = blurred(src, radius.clamp(0.5, 3.0));
    let strength = (amount / 100.0).clamp(0.0, 1.5) * 1.3;
    let threshold = 0.001 + (1.0 - (detail / 100.0).clamp(0.0, 1.0)).powi(2) * 0.08;

    let mut out = src.clone();
    for (pixel, blurred_pixel) in out.pixels_mut().zip(blur.pixels()) {
        let rgb = pixel_rgb(pixel);
        let blurred_rgb = pixel_rgb(blurred_pixel);
        let detail_signal = luma(rgb) - luma(blurred_rgb);
        let mask = (detail_signal.abs() / threshold).min(1.0);
        let delta = detail_signal * strength * mask;
        store_rgb(pixel, [rgb[0] + delta, rgb[1] + delta, rgb[2] + delta]);
    }
    out
}

/// Edge-aware smoothing. `detail` controls how strongly edges are protected:
/// lower values smooth more, higher values keep more fine detail.
fn noise_reduction(src: &RgbaImage, amount: f32, detail: f32) -> RgbaImage {
    if amount <= 0.0 {
        return src.clone();
    }

    let amount = (amount / 100.0).clamp(0.0, 1.0);
    let blur = blurred(src, 1.2);
    let threshold = 0.02 + (1.0 - (detail / 100.0).clamp(0.0, 1.0)) * 0.14;

    let mut out = src.clone();
    for (pixel, blurred_pixel) in out.pixels_mut().zip(blur.pixels()) {
        let rgb = pixel_rgb(pixel);
        let blurred_rgb = pixel_rgb(blurred_pixel);
        let difference = luma(rgb) - luma(blurred_rgb);
        let edge = (difference.abs() / threshold).powi(2);
        let weight = 1.0 / (1.0 + 4.0 * edge);
        let blend = amount * weight;
        store_rgb(
            pixel,
            [
                rgb[0] + (blurred_rgb[0] - rgb[0]) * blend,
                rgb[1] + (blurred_rgb[1] - rgb[1]) * blend,
                rgb[2] + (blurred_rgb[2] - rgb[2]) * blend,
            ],
        );
    }
    out
}

pub(crate) fn raw_filter(src: &RgbaImage, params: &Params) -> RgbaImage {
    let out = crate::ops::rf::raw_filter(src, params);
    return out;
    // let temp = params::f32(params, "temp", 0.0).clamp(-100.0, 100.0);
    // let tint = params::f32(params, "tint", 0.0).clamp(-100.0, 100.0);
    // let texture = params::f32(params, "texture", 0.0).clamp(-100.0, 100.0);
    // let clarity = params::f32(params, "clarity", 0.0).clamp(-100.0, 100.0);
    // let dehaze_amount = params::f32(params, "dehaze", 0.0).clamp(-100.0, 100.0);
    // let sharpen_amount = params::f32(params, "sharpen", 0.0).clamp(0.0, 150.0);
    // let sharpen_radius = params::f32(params, "sharpen_radius", 1.0);
    // let sharpen_detail = params::f32(params, "sharpen_detail", 25.0).clamp(0.0, 100.0);
    // let noise_reduction_amount = params::f32(params, "noise_reduction", 0.0).clamp(0.0, 100.0);
    // let noise_reduction_detail =
    // params::f32(params, "noise_reduction_detail", 50.0).clamp(0.0, 100.0);
    //
    // if temp == 0.0
    // && tint == 0.0
    // && texture == 0.0
    // && clarity == 0.0
    // && dehaze_amount == 0.0
    // && sharpen_amount == 0.0
    // && noise_reduction_amount == 0.0
    // {
    // return src.clone();
    // }
    //
    // let mut out = noise_reduction(src, noise_reduction_amount, noise_reduction_detail);
    // out = white_balance(&out, temp, tint);
    // out = dehaze(&out, dehaze_amount);
    // out = local_contrast(&out, 1.2, texture / 100.0 * 0.9, 0.0);
    // out = local_contrast(&out, 6.0, clarity / 100.0 * 0.7, 0.6);
    // out = sharpen(&out, sharpen_amount, sharpen_radius, sharpen_detail);
    // out
}
