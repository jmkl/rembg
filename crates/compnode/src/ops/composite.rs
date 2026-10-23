//! Multi-input compositing: Group, Blend and Mask.

use image::{RgbaImage, imageops::FilterType};

use crate::model::ViewerLayer;
use crate::params::{self, Params};

type Rgb = [f32; 3];

fn resize_to(image: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    if image.dimensions() == (width, height) {
        image.clone()
    } else {
        image::imageops::resize(image, width, height, FilterType::Triangle)
    }
}

fn rgb_of(pixel: [u8; 4]) -> Rgb {
    [
        pixel[0] as f32 / 255.0,
        pixel[1] as f32 / 255.0,
        pixel[2] as f32 / 255.0,
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
            *target = image::Rgba(composite(target.0, pixel.0, 1.0, |_cb, cs| cs));
        }
    }
    Ok(out)
}

/// Composite `blend` over `base` with one of the 16 CSS blend modes.
pub(crate) fn blend(
    base: Option<&RgbaImage>,
    blend: Option<&RgbaImage>,
    params: &Params,
    canvas: (u32, u32),
) -> Result<RgbaImage, String> {
    if base.is_none() && blend.is_none() {
        return Err("No input connected".to_owned());
    }
    let (width, height) = base
        .map(RgbaImage::dimensions)
        .or_else(|| blend.map(RgbaImage::dimensions))
        .unwrap_or(canvas);

    let mut out = match base {
        Some(base) => resize_to(base, width, height),
        None => RgbaImage::new(width, height),
    };
    let Some(blend) = blend else {
        return Ok(out);
    };
    let blend = resize_to(blend, width, height);

    let mode = params::string(params, "mode", "source-over");
    let opacity = params::f32(params, "opacity", 100.0).clamp(0.0, 100.0) / 100.0;

    for (x, y, source) in blend.enumerate_pixels() {
        let target = out.get_pixel_mut(x, y);
        *target = image::Rgba(composite(target.0, source.0, opacity, |cb, cs| {
            blend_rgb(mode, cb, cs)
        }));
    }
    Ok(out)
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
        let brightness = (0.2126 * mask_pixel[0] as f32
            + 0.7152 * mask_pixel[1] as f32
            + 0.0722 * mask_pixel[2] as f32)
            / 255.0;
        let mask_alpha = if dark_is_opaque {
            1.0 - brightness
        } else {
            brightness
        };
        let factor = 1.0 - strength + mask_alpha * strength;
        source[3] = (source[3] as f32 * factor).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// W3C source-over compositing with an RGB blend function.
fn composite(
    base: [u8; 4],
    source: [u8; 4],
    opacity: f32,
    blend: impl Fn(Rgb, Rgb) -> Rgb,
) -> [u8; 4] {
    let alpha_s = (source[3] as f32 / 255.0) * opacity;
    let alpha_b = base[3] as f32 / 255.0;
    let alpha_o = alpha_s + alpha_b * (1.0 - alpha_s);
    if alpha_o <= 0.0 {
        return [0, 0, 0, 0];
    }

    let cb = rgb_of(base);
    let cs = rgb_of(source);
    let mixed = blend(cb, cs);

    let mut out = [0u8; 4];
    for channel in 0..3 {
        let premultiplied = alpha_s * (1.0 - alpha_b) * cs[channel]
            + alpha_s * alpha_b * mixed[channel]
            + (1.0 - alpha_s) * alpha_b * cb[channel];
        out[channel] = ((premultiplied / alpha_o) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    out[3] = (alpha_o * 255.0).round().clamp(0.0, 255.0) as u8;
    out
}

fn blend_rgb(mode: &str, cb: Rgb, cs: Rgb) -> Rgb {
    match mode {
        "hue" => set_luminosity(set_saturation(cs, saturation(cb)), luminosity(cb)),
        "saturation" => set_luminosity(set_saturation(cb, saturation(cs)), luminosity(cb)),
        "color" => set_luminosity(cs, luminosity(cb)),
        "luminosity" => set_luminosity(cb, luminosity(cs)),
        _ => [
            blend_channel(mode, cb[0], cs[0]),
            blend_channel(mode, cb[1], cs[1]),
            blend_channel(mode, cb[2], cs[2]),
        ],
    }
}

fn blend_channel(mode: &str, cb: f32, cs: f32) -> f32 {
    match mode {
        "multiply" => cb * cs,
        "screen" => cb + cs - cb * cs,
        "overlay" => hard_light(cs, cb),
        "darken" => cb.min(cs),
        "lighten" => cb.max(cs),
        "color-dodge" => {
            if cb <= 0.0 {
                0.0
            } else if cs >= 1.0 {
                1.0
            } else {
                (cb / (1.0 - cs)).min(1.0)
            }
        }
        "color-burn" => {
            if cb >= 1.0 {
                1.0
            } else if cs <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - cb) / cs).min(1.0)
            }
        }
        "hard-light" => hard_light(cb, cs),
        "soft-light" => soft_light(cb, cs),
        "difference" => (cb - cs).abs(),
        "exclusion" => cb + cs - 2.0 * cb * cs,
        // "source-over" and anything unknown behave like Normal.
        _ => cs,
    }
}

fn luminosity(color: Rgb) -> f32 {
    0.3 * color[0] + 0.59 * color[1] + 0.11 * color[2]
}

fn saturation(color: Rgb) -> f32 {
    color[0].max(color[1]).max(color[2]) - color[0].min(color[1]).min(color[2])
}

fn hard_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        2.0 * cb * cs
    } else {
        1.0 - 2.0 * (1.0 - cb) * (1.0 - cs)
    }
}

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
    composite_layers_at(layers, canvas, Some(super::EVAL_MAX_EDGE as u64))
}

/// Composite layers at (up to) the given resolution. `max_edge = None` renders
/// at full canvas resolution (used by the background disk render).
pub(crate) fn composite_layers_at(
    layers: &[(ViewerLayer, image::RgbaImage)],
    canvas: (u32, u32),
    max_edge: Option<u64>,
) -> Result<RgbaImage, String> {
    if layers.is_empty() {
        return Err("No layers connected".to_owned());
    }
    let (cw, ch) = canvas;
    let longest = cw.max(ch).max(1);
    let scale = match max_edge {
        Some(cap) if longest as u64 > cap => cap as f64 / longest as f64,
        _ => 1.0,
    };
    let out_w = ((cw as f64 * scale).round() as u32).max(1);
    let out_h = ((ch as f64 * scale).round() as u32).max(1);
    let mut out = RgbaImage::new(out_w, out_h);

    for (layer, src) in layers {
        let cx = layer.center.x as f64 * scale;
        let cy = layer.center.y as f64 * scale;
        let sw = (layer.size.x as f64 * scale).max(1.0);
        let sh = (layer.size.y as f64 * scale).max(1.0);
        let (src_w, src_h) = (src.width() as f64, src.height() as f64);
        let cos = layer.angle.cos() as f64;
        let sin = layer.angle.sin() as f64;

        for oy in 0..out_h {
            for ox in 0..out_w {
                let px = ox as f64 + 0.5;
                let py = oy as f64 + 0.5;
                let rx = px - cx;
                let ry = py - cy;
                // inverse rotate by -angle
                let lx = rx * cos + ry * sin;
                let ly = -rx * sin + ry * cos;
                let sx = (lx + sw / 2.0) / sw * src_w;
                let sy = (ly + sh / 2.0) / sh * src_h;
                if sx < 0.0 || sy < 0.0 || sx >= src_w || sy >= src_h {
                    continue;
                }
                let x0 = sx.floor() as u32;
                let y0 = sy.floor() as u32;
                let x1 = (x0 + 1).min(src.width().saturating_sub(1));
                let y1 = (y0 + 1).min(src.height().saturating_sub(1));
                let fx = sx - x0 as f64;
                let fy = sy - y0 as f64;
                let c00 = src.get_pixel(x0, y0);
                let c10 = src.get_pixel(x1, y0);
                let c01 = src.get_pixel(x0, y1);
                let c11 = src.get_pixel(x1, y1);
                let mut rgba = [0u8; 4];
                for c in 0..4 {
                    let v = c00[c] as f64 * (1.0 - fx) * (1.0 - fy)
                        + c10[c] as f64 * fx * (1.0 - fy)
                        + c01[c] as f64 * (1.0 - fx) * fy
                        + c11[c] as f64 * fx * fy;
                    rgba[c] = v.round().clamp(0.0, 255.0) as u8;
                }
                let sa = rgba[3] as f64 / 255.0;
                let dst = out.get_pixel(ox, oy);
                let da = dst[3] as f64 / 255.0;
                let oa = sa + da * (1.0 - sa);
                if oa <= 0.0 {
                    continue;
                }
                let mut res = [0u8; 4];
                for c in 0..3 {
                    let sc = rgba[c] as f64 / 255.0;
                    let dc = dst[c] as f64 / 255.0;
                    let oc = (sa * sc + da * dc * (1.0 - sa)) / oa;
                    res[c] = (oc * 255.0).round().clamp(0.0, 255.0) as u8;
                }
                res[3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
                *out.get_pixel_mut(ox, oy) = image::Rgba(res);
            }
        }
    }
    Ok(out)
}
