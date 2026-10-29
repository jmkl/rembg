//! Multi-input compositing: Group, Blend and Mask.

use image::{RgbaImage, imageops::FilterType};
use rayon::prelude::*;

use crate::model::ViewerLayer;
use crate::ops::math::{
    byte_to_unit, byte_value_to_u8_round, css_blend_luma, luma709_bytes, sample_bilinear_rgba8,
};
use crate::params::{self, Params};

type Rgb = [f32; 3];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlendOp {
    SourceOver,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendOp {
    pub(crate) fn from_str(s: &str) -> Self {
        match s {
            "multiply" => Self::Multiply,
            "screen" => Self::Screen,
            "overlay" => Self::Overlay,
            "darken" => Self::Darken,
            "lighten" => Self::Lighten,
            "color-dodge" => Self::ColorDodge,
            "color-burn" => Self::ColorBurn,
            "hard-light" => Self::HardLight,
            "soft-light" => Self::SoftLight,
            "difference" => Self::Difference,
            "exclusion" => Self::Exclusion,
            "hue" => Self::Hue,
            "saturation" => Self::Saturation,
            "color" => Self::Color,
            "luminosity" => Self::Luminosity,
            _ => Self::SourceOver,
        }
    }

    #[inline(always)]
    fn blend(&self, cb: Rgb, cs: Rgb) -> Rgb {
        match self {
            Self::SourceOver => cs,
            Self::Multiply => [cb[0] * cs[0], cb[1] * cs[1], cb[2] * cs[2]],
            Self::Screen => [
                cb[0] + cs[0] - cb[0] * cs[0],
                cb[1] + cs[1] - cb[1] * cs[1],
                cb[2] + cs[2] - cb[2] * cs[2],
            ],
            Self::Overlay => [
                hard_light(cs[0], cb[0]),
                hard_light(cs[1], cb[1]),
                hard_light(cs[2], cb[2]),
            ],
            Self::Darken => [cb[0].min(cs[0]), cb[1].min(cs[1]), cb[2].min(cs[2])],
            Self::Lighten => [cb[0].max(cs[0]), cb[1].max(cs[1]), cb[2].max(cs[2])],
            Self::Difference => [
                (cb[0] - cs[0]).abs(),
                (cb[1] - cs[1]).abs(),
                (cb[2] - cs[2]).abs(),
            ],
            Self::Exclusion => [
                cb[0] + cs[0] - 2.0 * cb[0] * cs[0],
                cb[1] + cs[1] - 2.0 * cb[1] * cs[1],
                cb[2] + cs[2] - 2.0 * cb[2] * cs[2],
            ],
            Self::HardLight => [
                hard_light(cb[0], cs[0]),
                hard_light(cb[1], cs[1]),
                hard_light(cb[2], cs[2]),
            ],
            Self::SoftLight => [
                soft_light(cb[0], cs[0]),
                soft_light(cb[1], cs[1]),
                soft_light(cb[2], cs[2]),
            ],
            Self::ColorDodge => [
                dodge(cb[0], cs[0]),
                dodge(cb[1], cs[1]),
                dodge(cb[2], cs[2]),
            ],
            Self::ColorBurn => [burn(cb[0], cs[0]), burn(cb[1], cs[1]), burn(cb[2], cs[2])],
            Self::Hue => set_luminosity(set_saturation(cs, saturation(cb)), luminosity(cb)),
            Self::Saturation => set_luminosity(set_saturation(cb, saturation(cs)), luminosity(cb)),
            Self::Color => set_luminosity(cs, luminosity(cb)),
            Self::Luminosity => set_luminosity(cb, luminosity(cs)),
        }
    }
}

fn resize_to(image: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    if image.dimensions() == (width, height) {
        image.clone()
    } else {
        image::imageops::resize(image, width, height, FilterType::Triangle)
    }
}

#[inline(always)]
fn rgb_of(pixel: [u8; 4]) -> Rgb {
    [
        byte_to_unit(pixel[0]),
        byte_to_unit(pixel[1]),
        byte_to_unit(pixel[2]),
    ]
}

/// Stack images bottom-to-top at their natural size (clipped to the union).
pub(crate) fn stack(layers: &[RgbaImage]) -> Result<RgbaImage, String> {
    let Some(first) = layers.first() else {
        return Err("No input connected".to_owned());
    };
    let width = layers
        .iter()
        .map(RgbaImage::width)
        .max()
        .unwrap_or(first.width());
    let height = layers
        .iter()
        .map(RgbaImage::height)
        .max()
        .unwrap_or(first.height());
    let mut out = RgbaImage::new(width.max(1), height.max(1));

    for layer in layers {
        for (x, y, pixel) in layer.enumerate_pixels() {
            if x >= width || y >= height {
                continue;
            }
            let target = out.get_pixel_mut(x, y);
            *target = image::Rgba(composite_pixel(target.0, pixel.0, 1.0, BlendOp::SourceOver));
        }
    }
    Ok(out)
}

/// Composite `top` over `bottom` on the base canvas (following settings canvas width and height).
/// Each input is fitted into the canvas preserving its aspect ratio, centred.
/// The blend mode and opacity affect the top layer.
pub(crate) fn blend(
    top: Option<&RgbaImage>,
    bottom: Option<&RgbaImage>,
    params: &Params,
    canvas: (u32, u32),
) -> Result<RgbaImage, String> {
    if top.is_none() && bottom.is_none() {
        return Err("No input connected".to_owned());
    }
    // The output is the document canvas when one is known; otherwise fall back
    // to the bottom (then top) image size.
    let (width, height) = if canvas.0 > 0 && canvas.1 > 0 {
        canvas
    } else {
        bottom
            .map(RgbaImage::dimensions)
            .or_else(|| top.map(RgbaImage::dimensions))
            .unwrap_or((1, 1))
    };

    let mut out = RgbaImage::new(width.max(1), height.max(1));

    let mode = params::string(params, "mode", "source-over");
    let opacity = params::f32(params, "opacity", 100.0).clamp(0.0, 100.0) / 100.0;

    // Bottom layer acts as the base canvas image (drawn normally, no mode).
    if let Some(bottom) = bottom {
        paste_fitted(&mut out, bottom);
    }

    if let Some(top) = top {
        let fitted = fit_canvas(top, width.max(1), height.max(1));
        if bottom.is_some() {
            let op = BlendOp::from_str(mode);
            out.as_mut()
                .par_chunks_exact_mut(4)
                .zip(fitted.as_raw().par_chunks_exact(4))
                .for_each(|(target, src)| {
                    let base = [target[0], target[1], target[2], target[3]];
                    let source = [src[0], src[1], src[2], src[3]];
                    let res = composite_pixel(base, source, opacity, op);
                    target.copy_from_slice(&res);
                });
        } else {
            // Only top layer connected: drawn with opacity.
            out.as_mut()
                .par_chunks_exact_mut(4)
                .zip(fitted.as_raw().par_chunks_exact(4))
                .for_each(|(target, src)| {
                    let base = [target[0], target[1], target[2], target[3]];
                    let source = [src[0], src[1], src[2], src[3]];
                    let res = composite_pixel(base, source, opacity, BlendOp::SourceOver);
                    target.copy_from_slice(&res);
                });
        }
    }

    Ok(out)
}

/// Fit `src` inside `canvas` keeping its aspect ratio, centred, on a
/// transparent `canvas`-sized image.
fn fit_canvas(src: &RgbaImage, canvas_w: u32, canvas_h: u32) -> RgbaImage {
    let mut out = RgbaImage::new(canvas_w, canvas_h);
    paste_fitted(&mut out, src);
    out
}

/// Draw `src` centred inside `dst`, scaled to fit while keeping its aspect.
fn paste_fitted(dst: &mut RgbaImage, src: &RgbaImage) {
    let (dw, dh) = (dst.width() as f64, dst.height() as f64);
    let (sw, sh) = (src.width() as f64, src.height() as f64);
    if sw <= 0.0 || sh <= 0.0 || dw <= 0.0 || dh <= 0.0 {
        return;
    }
    let factor = (dw / sw).min(dh / sh);
    let fit_w = ((sw * factor).round() as u32).max(1).min(dst.width());
    let fit_h = ((sh * factor).round() as u32).max(1).min(dst.height());
    let resized = resize_to(src, fit_w, fit_h);
    let x0 = (dst.width().saturating_sub(fit_w)) / 2;
    let y0 = (dst.height().saturating_sub(fit_h)) / 2;
    image::imageops::overlay(dst, &resized, x0 as i64, y0 as i64);
}

/// Multiply the image alpha by a mask's luminance.
pub(crate) fn apply_mask(
    image: &RgbaImage,
    mask: Option<&RgbaImage>,
    params: &Params,
) -> RgbaImage {
    let Some(mask) = mask else {
        return image.clone();
    };
    let (width, height) = image.dimensions();
    let mask = resize_to(mask, width, height);
    let strength = params::f32(params, "strength", 100.0).clamp(0.0, 100.0) / 100.0;
    let dark_is_opaque = params::string(params, "invert", "dark") != "light";

    let mut out = image.clone();
    for (source, mask_pixel) in out.pixels_mut().zip(mask.pixels()) {
        let brightness =
            luma709_bytes([mask_pixel[0], mask_pixel[1], mask_pixel[2]]) * (1.0 / 255.0);
        let mask_alpha = if dark_is_opaque {
            1.0 - brightness
        } else {
            brightness
        };
        let factor = 1.0 - strength + mask_alpha * strength;
        source[3] = byte_value_to_u8_round(source[3] as f32 * factor);
    }
    out
}

/// W3C source-over compositing with an RGB blend function.
#[inline(always)]
fn composite_pixel(base: [u8; 4], source: [u8; 4], opacity: f32, op: BlendOp) -> [u8; 4] {
    let alpha_s = (source[3] as f32 * (1.0 / 255.0)) * opacity;
    if alpha_s <= 0.0 {
        return base;
    }
    let alpha_b = base[3] as f32 * (1.0 / 255.0);
    let alpha_o = alpha_s + alpha_b * (1.0 - alpha_s);
    if alpha_o <= 0.0 {
        return [0, 0, 0, 0];
    }

    let cb = rgb_of(base);
    let cs = rgb_of(source);
    let mixed = op.blend(cb, cs);

    let inv_alpha_o = 255.0 / alpha_o;
    let s_term = alpha_s * (1.0 - alpha_b);
    let mix_term = alpha_s * alpha_b;
    let b_term = (1.0 - alpha_s) * alpha_b;

    [
        byte_value_to_u8_round(
            (s_term * cs[0] + mix_term * mixed[0] + b_term * cb[0]) * inv_alpha_o,
        ),
        byte_value_to_u8_round(
            (s_term * cs[1] + mix_term * mixed[1] + b_term * cb[1]) * inv_alpha_o,
        ),
        byte_value_to_u8_round(
            (s_term * cs[2] + mix_term * mixed[2] + b_term * cb[2]) * inv_alpha_o,
        ),
        byte_value_to_u8_round(alpha_o * 255.0),
    ]
}

#[inline(always)]
fn dodge(cb: f32, cs: f32) -> f32 {
    if cb <= 0.0 {
        0.0
    } else if cs >= 1.0 {
        1.0
    } else {
        (cb / (1.0 - cs)).min(1.0)
    }
}

#[inline(always)]
fn burn(cb: f32, cs: f32) -> f32 {
    if cb >= 1.0 {
        1.0
    } else if cs <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - cb) / cs).min(1.0)
    }
}

#[inline(always)]
fn luminosity(color: Rgb) -> f32 {
    css_blend_luma(color)
}

#[inline(always)]
fn saturation(color: Rgb) -> f32 {
    color[0].max(color[1]).max(color[2]) - color[0].min(color[1]).min(color[2])
}

#[inline(always)]
fn hard_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        2.0 * cb * cs
    } else {
        1.0 - 2.0 * (1.0 - cb) * (1.0 - cs)
    }
}

#[inline(always)]
fn soft_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
    } else {
        let d = if cb <= 0.25 {
            ((16.0 * cb - 12.0) * cb + 4.0) * cb
        } else {
            cb.sqrt()
        };
        cb + (2.0 * cs - 1.0) * (d - cb)
    }
}

fn set_luminosity(color: Rgb, target: f32) -> Rgb {
    let delta = target - luminosity(color);
    clip_color([color[0] + delta, color[1] + delta, color[2] + delta])
}

fn clip_color(color: Rgb) -> Rgb {
    let lum = luminosity(color);
    let min = color[0].min(color[1]).min(color[2]);
    let max = color[0].max(color[1]).max(color[2]);
    let mut out = color;
    if min < 0.0 {
        for channel in &mut out {
            *channel = lum + (*channel - lum) * lum / (lum - min);
        }
    }
    if max > 1.0 {
        for channel in &mut out {
            *channel = lum + (*channel - lum) * (1.0 - lum) / (max - lum);
        }
    }
    out
}

/// W3C SetSat: keep the hue/lightness ordering and force the spread to `sat`.
fn set_saturation(color: Rgb, sat: f32) -> Rgb {
    let mut order = [0usize, 1, 2];
    order.sort_by(|a, b| {
        color[*a]
            .partial_cmp(&color[*b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let (min, mid, max) = (order[0], order[1], order[2]);
    let mut out = [0.0f32; 3];
    if color[max] > color[min] {
        out[mid] = (color[mid] - color[min]) * sat / (color[max] - color[min]);
        out[max] = sat;
    }
    out[min] = 0.0;
    out
}

/// Composite a set of viewer-style layers (with transforms) into a single
/// bitmap, downscaled so the longest edge stays within the eval cap. Used for
/// the interactive Group preview.
pub(crate) fn composite_layers(
    layers: &[(ViewerLayer, image::RgbaImage)],
    canvas: (u32, u32),
) -> Result<RgbaImage, String> {
    composite_layers_with(
        layers,
        canvas,
        Some(super::EVAL_MAX_EDGE as u64),
        "source-over",
        1.0,
    )
}

/// Composite layers at (up to) the given resolution. `max_edge = None` renders
/// at full canvas resolution (used by the background disk render).
///
/// The bottom layer is drawn normally. Every layer above it is blended with
/// `mode` at `opacity`, so a Blend node keeps its base/overlay semantics even
/// when layers are moved or rotated in the canvas.
pub(crate) fn composite_layers_with(
    layers: &[(ViewerLayer, image::RgbaImage)],
    canvas: (u32, u32),
    max_edge: Option<u64>,
    mode: &str,
    opacity: f32,
) -> Result<RgbaImage, String> {
    if layers.is_empty() {
        return Err("No layers connected".to_owned());
    }
    let (cw, ch) = canvas;
    let longest = cw.max(ch).max(1);
    let scale = match max_edge {
        Some(cap) if longest as u64 > cap => cap as f32 / longest as f32,
        _ => 1.0,
    };
    let out_w = ((cw as f32 * scale).round() as u32).max(1);
    let out_h = ((ch as f32 * scale).round() as u32).max(1);
    let mut out = RgbaImage::new(out_w, out_h);

    let op = BlendOp::from_str(mode);

    for (index, (layer, src)) in layers.iter().enumerate() {
        let (src_w, src_h) = (src.width() as f32, src.height() as f32);
        if src_w <= 0.0 || src_h <= 0.0 {
            continue;
        }
        let (cx, cy, sw, sh) = if layer.size.x > 0.0 && layer.size.y > 0.0 {
            (
                layer.center.x * scale,
                layer.center.y * scale,
                (layer.size.x * scale).max(1.0),
                (layer.size.y * scale).max(1.0),
            )
        } else {
            let fit = fit_size(
                src_w as f64,
                src_h as f64,
                cw.max(1) as f64,
                ch.max(1) as f64,
            );
            (
                cw.max(1) as f32 * scale / 2.0,
                ch.max(1) as f32 * scale / 2.0,
                (fit.0 as f32 * scale).max(1.0),
                (fit.1 as f32 * scale).max(1.0),
            )
        };

        let layer_op = if index == 0 { BlendOp::SourceOver } else { op };
        let layer_opacity = if index == 0 { 1.0 } else { opacity };
        if layer_opacity <= 0.0 {
            continue;
        }

        let inv_sw = src_w / sw;
        let inv_sh = src_h / sh;
        let cos = layer.angle.cos();
        let sin = layer.angle.sin();
        let is_unrotated = layer.angle.abs() < 1e-5;

        let src_raw = src.as_raw();
        let row_stride = out_w as usize * 4;
        let src_w_u32 = src.width();
        let src_h_u32 = src.height();

        out.as_mut()
            .par_chunks_exact_mut(row_stride)
            .enumerate()
            .for_each(|(oy, row)| {
                let py = (oy as f32 + 0.5) - cy;

                if is_unrotated {
                    let sy = (py + sh * 0.5) * inv_sh;
                    if sy < 0.0 || sy >= src_h {
                        return;
                    }
                    let ox_min_f = (cx - sw * 0.5) - 0.5;
                    let ox_max_f = (cx + sw * 0.5) + 0.5;
                    let ox_start = (ox_min_f.floor().max(0.0) as usize).min(out_w as usize);
                    let ox_end = (ox_max_f.ceil().max(0.0) as usize).min(out_w as usize);

                    for ox in ox_start..ox_end {
                        let px = (ox as f32 + 0.5) - cx;
                        let sx = (px + sw * 0.5) * inv_sw;
                        if sx < 0.0 || sx >= src_w {
                            continue;
                        }
                        let rgba = sample_bilinear_rgba8(src_raw, src_w_u32, src_h_u32, sx, sy);
                        let offset = ox * 4;
                        let base = [
                            row[offset],
                            row[offset + 1],
                            row[offset + 2],
                            row[offset + 3],
                        ];
                        let mixed = composite_pixel(base, rgba, layer_opacity, layer_op);
                        row[offset..offset + 4].copy_from_slice(&mixed);
                    }
                } else {
                    for ox in 0..out_w as usize {
                        let px = (ox as f32 + 0.5) - cx;
                        let lx = px * cos + py * sin;
                        let ly = -px * sin + py * cos;
                        let sx = (lx + sw * 0.5) * inv_sw;
                        let sy = (ly + sh * 0.5) * inv_sh;
                        if sx < 0.0 || sy < 0.0 || sx >= src_w || sy >= src_h {
                            continue;
                        }
                        let rgba = sample_bilinear_rgba8(src_raw, src_w_u32, src_h_u32, sx, sy);
                        let offset = ox * 4;
                        let base = [
                            row[offset],
                            row[offset + 1],
                            row[offset + 2],
                            row[offset + 3],
                        ];
                        let mixed = composite_pixel(base, rgba, layer_opacity, layer_op);
                        row[offset..offset + 4].copy_from_slice(&mixed);
                    }
                }
            });
    }
    Ok(out)
}

/// Size of `src` fitted inside `canvas`, keeping its aspect ratio.
fn fit_size(src_w: f64, src_h: f64, canvas_w: f64, canvas_h: f64) -> (f64, f64) {
    if src_w <= 0.0 || src_h <= 0.0 {
        return (canvas_w.max(1.0), canvas_h.max(1.0));
    }
    let factor = (canvas_w / src_w).min(canvas_h / src_h);
    ((src_w * factor).max(1.0), (src_h * factor).max(1.0))
}
