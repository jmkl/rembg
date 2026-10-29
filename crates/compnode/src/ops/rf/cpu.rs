//! CPU backend: rayon, one f32 RGBA working buffer for the whole pipeline.
//!
//! Compared with the original:
//!   * no per-stage `RgbaImage` clones or u8 round-trips (values are clamped to
//!     0..1 after each stage, so the stage-to-stage behaviour is unchanged);
//!   * white balance + dehaze are fused into one pass;
//!   * texture / clarity / sharpen blur a single luma plane instead of RGBA;
//!   * every loop, including the separable Gaussian blur, runs on rayon.

use image::RgbaImage;
use rayon::prelude::*;

use super::Settings;
use crate::ops::math::{clamp01, luma709};

pub(super) fn apply(src: &RgbaImage, s: &Settings) -> RgbaImage {
    let (w, h) = (src.width() as usize, src.height() as usize);

    let mut buf: Vec<f32> = src
        .as_raw()
        .par_iter()
        .map(|&v| v as f32 * (1.0 / 255.0))
        .collect();

    noise_reduction(&mut buf, w, h, s);
    color_stage(&mut buf, s);
    local_contrast(&mut buf, w, h, 1.2, s.texture_amount(), 0.0);
    local_contrast(&mut buf, w, h, 6.0, s.clarity_amount(), 0.6);
    sharpen(&mut buf, w, h, s);

    let bytes: Vec<u8> = buf
        .par_iter()
        .map(|&v| crate::ops::math::unit_to_u8_round(v))
        .collect();
    RgbaImage::from_raw(w as u32, h as u32, bytes).expect("buffer matches dimensions")
}

// ---------------------------------------------------------------------------
// Blur
// ---------------------------------------------------------------------------

fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let sigma = sigma.max(0.1);
    let radius = (sigma * 3.0).ceil() as usize;
    let mut k: Vec<f32> = (0..=2 * radius)
        .map(|i| {
            let x = i as f32 - radius as f32;
            (-(x * x) / (2.0 * sigma * sigma)).exp()
        })
        .collect();
    let sum: f32 = k.iter().sum();
    k.iter_mut().for_each(|v| *v /= sum);
    k
}

/// Separable Gaussian on a single-channel plane, clamp-to-edge.
fn blur_plane(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let kernel = gaussian_kernel(sigma);
    let r = kernel.len() / 2;

    // Horizontal: pad each row once so the inner loop has no bounds logic.
    let mut tmp = vec![0.0f32; w * h];
    tmp.par_chunks_mut(w).zip(src.par_chunks(w)).for_each_init(
        || vec![0.0f32; w + 2 * r],
        |pad, (out, inp)| {
            pad[..r].fill(inp[0]);
            pad[r..r + w].copy_from_slice(inp);
            pad[r + w..].fill(inp[w - 1]);
            for (x, o) in out.iter_mut().enumerate() {
                *o = pad[x..x + 2 * r + 1]
                    .iter()
                    .zip(&kernel)
                    .map(|(a, b)| a * b)
                    .sum::<f32>();
            }
        },
    );

    // Vertical: accumulate whole rows (contiguous, auto-vectorises).
    let mut out = vec![0.0f32; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (k, &wt) in kernel.iter().enumerate() {
            let yy = (y as isize + k as isize - r as isize).clamp(0, h as isize - 1) as usize;
            let src_row = &tmp[yy * w..(yy + 1) * w];
            for (o, &v) in row.iter_mut().zip(src_row) {
                *o += wt * v;
            }
        }
    });
    out
}

fn luma_plane(buf: &[f32]) -> Vec<f32> {
    buf.par_chunks_exact(4)
        .map(|p| luma709([p[0], p[1], p[2]]))
        .collect()
}

// ---------------------------------------------------------------------------
// Stages
// ---------------------------------------------------------------------------

fn noise_reduction(buf: &mut [f32], w: usize, h: usize, s: &Settings) {
    let amount = s.nr_amount();
    if amount <= 0.0 {
        return;
    }
    let threshold = s.nr_threshold();

    // Colour is blended towards the blurred RGB, so this stage needs 3 planes.
    let blurred: [Vec<f32>; 3] = std::array::from_fn(|c| {
        let plane: Vec<f32> = buf.par_chunks_exact(4).map(|p| p[c]).collect();
        blur_plane(&plane, w, h, 1.2)
    });

    buf.par_chunks_exact_mut(4).enumerate().for_each(|(i, px)| {
        let b = [blurred[0][i], blurred[1][i], blurred[2][i]];
        let diff = luma709([px[0], px[1], px[2]]) - luma709([b[0], b[1], b[2]]);
        let edge = (diff.abs() / threshold).powi(2);
        let blend = amount / (1.0 + 4.0 * edge);
        for c in 0..3 {
            px[c] = clamp01(px[c] + (b[c] - px[c]) * blend);
        }
    });
}

/// White balance + dehaze in a single pass.
fn color_stage(buf: &mut [f32], s: &Settings) {
    let wb_on = s.wb_active();
    let gains = s.wb_gains();
    let dehaze = s.dehaze_amount();
    if !wb_on && dehaze == 0.0 {
        return;
    }
    buf.par_chunks_exact_mut(4).for_each(|px| {
        let mut rgb = [px[0], px[1], px[2]];
        if wb_on {
            for c in 0..3 {
                rgb[c] = clamp01(rgb[c] * gains[c]);
            }
        }
        dehaze_px(&mut rgb, dehaze);
        px[..3].copy_from_slice(&rgb);
    });
}

#[inline]
fn dehaze_px(rgb: &mut [f32; 3], amount: f32) {
    if amount > 0.0 {
        let black = 0.12 * amount;
        let contrast = 1.0 + 0.30 * amount;
        let saturation = 1.0 + 0.30 * amount;
        for v in rgb.iter_mut() {
            *v = (*v - black) / (1.0 - black).max(1e-4);
            *v = (*v - 0.5) * contrast + 0.5;
        }
        let l = luma709(*rgb);
        for v in rgb.iter_mut() {
            *v = l + (*v - l) * saturation;
        }
    } else if amount < 0.0 {
        let haze = -amount;
        let haze_color = [0.75, 0.80, 0.88];
        for (c, v) in rgb.iter_mut().enumerate() {
            *v = *v * (1.0 - 0.5 * haze) + haze_color[c] * 0.5 * haze;
            *v = (*v - 0.5) * (1.0 - 0.20 * haze) + 0.5;
        }
    }
    for v in rgb.iter_mut() {
        *v = clamp01(*v);
    }
}

fn local_contrast(buf: &mut [f32], w: usize, h: usize, sigma: f32, amount: f32, bias: f32) {
    if amount == 0.0 {
        return;
    }
    let luma_p = luma_plane(buf);
    let blur = blur_plane(&luma_p, w, h, sigma);

    buf.par_chunks_exact_mut(4)
        .zip(luma_p.par_iter().zip(blur.par_iter()))
        .for_each(|(px, (&l, &b))| {
            let detail = l - b;
            let midtones = 4.0 * l * (1.0 - l);
            let weight = 1.0 - bias + bias * midtones;
            let delta = detail * amount * weight;
            for c in 0..3 {
                px[c] = clamp01(px[c] + delta);
            }
        });
}

fn sharpen(buf: &mut [f32], w: usize, h: usize, s: &Settings) {
    if s.sharpen <= 0.0 {
        return;
    }
    let strength = s.sharpen_strength();
    let threshold = s.sharpen_threshold();

    let luma_p = luma_plane(buf);
    let blur = blur_plane(&luma_p, w, h, s.sharpen_sigma());

    buf.par_chunks_exact_mut(4)
        .zip(luma_p.par_iter().zip(blur.par_iter()))
        .for_each(|(px, (&l, &b))| {
            let detail = l - b;
            let mask = (detail.abs() / threshold).min(1.0);
            let delta = detail * strength * mask;
            for c in 0..3 {
                px[c] = clamp01(px[c] + delta);
            }
        });
}
