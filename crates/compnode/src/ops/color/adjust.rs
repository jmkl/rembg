//! Per-pixel colour adjustments ported from the Svelte node registry.

use std::cmp::Ordering;

use image::RgbaImage;

use crate::ops::math::{byte_to_unit, clamp01, unit_to_u8_round};
use crate::params::{self, CurvePoint, Params};

fn percent(params: &Params, key: &str, default: f32) -> f32 {
    params::f32(params, key, default) / 100.0
}

// ---------------------------------------------------------------- curves ----

/// Normalize saved curve data into sorted, clamped points with fixed ends.
pub(crate) fn sanitize_curve(points: &[CurvePoint]) -> Vec<CurvePoint> {
    let mut sorted: Vec<CurvePoint> = points
        .iter()
        .filter(|point| point.x.is_finite() && point.y.is_finite())
        .map(|point| CurvePoint {
            x: clamp01(point.x),
            y: clamp01(point.y),
        })
        .collect();
    sorted.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(Ordering::Equal));

    let mut unique: Vec<CurvePoint> = Vec::with_capacity(sorted.len());
    for point in sorted {
        if let Some(last) = unique.last_mut()
            && (last.x - point.x).abs() < 1e-6
        {
            *last = point;
            continue;
        }
        unique.push(point);
    }
    if unique.len() < 2 {
        return params::identity_curve();
    }
    if unique[0].x > 1e-6 {
        unique.insert(
            0,
            CurvePoint {
                x: 0.0,
                y: unique[0].y,
            },
        );
    }
    if unique.last().is_some_and(|point| point.x < 1.0 - 1e-6) {
        let y = unique.last().map(|point| point.y).unwrap_or(1.0);
        unique.push(CurvePoint { x: 1.0, y });
    }
    if let Some(first) = unique.first_mut() {
        first.x = 0.0;
    }
    if let Some(last) = unique.last_mut() {
        last.x = 1.0;
    }
    unique
}

/// Monotone cubic interpolation, matching Photoshop's smooth curve feel.
fn sample_curve(points: &[CurvePoint], x: f32) -> f32 {
    let points = sanitize_curve(points);
    let n = points.len();
    if n < 2 {
        return clamp01(x);
    }
    if n == 2 {
        let a = points[0];
        let b = points[1];
        if x <= a.x {
            return a.y;
        }
        if x >= b.x {
            return b.y;
        }
        return clamp01(a.y + (b.y - a.y) * (x - a.x) / (b.x - a.x));
    }

    let mut dx = vec![0.0f32; n - 1];
    let mut slope = vec![0.0f32; n - 1];
    for i in 0..n - 1 {
        dx[i] = (points[i + 1].x - points[i].x).max(1e-6);
        slope[i] = (points[i + 1].y - points[i].y) / dx[i];
    }

    let mut tangent = vec![0.0f32; n];
    tangent[0] = slope[0];
    tangent[n - 1] = slope[n - 2];
    for i in 1..n - 1 {
        tangent[i] = (slope[i - 1] + slope[i]) / 2.0;
    }
    for i in 0..n - 1 {
        if slope[i].abs() < 1e-8 {
            tangent[i] = 0.0;
            tangent[i + 1] = 0.0;
            continue;
        }
        let a = tangent[i] / slope[i];
        let b = tangent[i + 1] / slope[i];
        let h = (a * a + b * b).sqrt();
        if h > 3.0 {
            let t = 3.0 / h;
            tangent[i] = t * a * slope[i];
            tangent[i + 1] = t * b * slope[i];
        }
    }

    let x = clamp01(x);
    if x <= points[0].x {
        return points[0].y;
    }
    if x >= points[n - 1].x {
        return points[n - 1].y;
    }
    let mut i = 0;
    while i < n - 2 && x > points[i + 1].x {
        i += 1;
    }
    let t = (x - points[i].x) / dx[i];
    let t2 = t * t;
    let t3 = t2 * t;
    clamp01(
        (2.0 * t3 - 3.0 * t2 + 1.0) * points[i].y
            + (t3 - 2.0 * t2 + t) * dx[i] * tangent[i]
            + (-2.0 * t3 + 3.0 * t2) * points[i + 1].y
            + (t3 - t2) * dx[i] * tangent[i + 1],
    )
}

/// 256-entry lookup table used by the evaluator and the curve editor.
pub(crate) fn curve_lut(points: &[CurvePoint]) -> [u8; 256] {
    let normalized = sanitize_curve(points);
    let mut lut = [0u8; 256];
    for (index, slot) in lut.iter_mut().enumerate() {
        *slot = (sample_curve(&normalized, index as f32 / 255.0) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    lut
}

pub(crate) fn curves(src: &RgbaImage, params: &Params) -> RgbaImage {
    let master = curve_lut(&params::curve(params, "rgb"));
    let red = curve_lut(&params::curve(params, "red"));
    let green = curve_lut(&params::curve(params, "green"));
    let blue = curve_lut(&params::curve(params, "blue"));

    let mut out = src.clone();
    for pixel in out.pixels_mut() {
        pixel[0] = red[master[pixel[0] as usize] as usize];
        pixel[1] = green[master[pixel[1] as usize] as usize];
        pixel[2] = blue[master[pixel[2] as usize] as usize];
    }
    out
}

// ------------------------------------------------------------- basic ops ----

pub(crate) fn invert(src: &RgbaImage, params: &Params) -> RgbaImage {
    let amount = clamp01(percent(params, "amount", 100.0));
    let mut out = src.clone();
    for pixel in out.pixels_mut() {
        for channel in 0..3 {
            let value = pixel[channel] as f32;
            pixel[channel] = (value + (255.0 - 2.0 * value) * amount)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
    out
}

pub(crate) fn exposure(src: &RgbaImage, params: &Params) -> RgbaImage {
    let ev = params::f32(params, "ev", 0.0).clamp(-5.0, 5.0);
    let offset = params::f32(params, "offset", 0.0).clamp(-50.0, 50.0) / 100.0;
    let gamma = params::f32(params, "gamma", 1.0).clamp(0.1, 3.0);
    let gain = 2.0f32.powf(ev);

    let mut lut = [0u8; 256];
    for (value, slot) in lut.iter_mut().enumerate() {
        let normalized = clamp01((value as f32 / 255.0) * gain + offset);
        *slot = (normalized.powf(1.0 / gamma) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
    }

    let mut out = src.clone();
    for pixel in out.pixels_mut() {
        pixel[0] = lut[pixel[0] as usize];
        pixel[1] = lut[pixel[1] as usize];
        pixel[2] = lut[pixel[2] as usize];
    }
    out
}

/// CSS `hue-rotate` + `saturate` + `brightness`, matching the reference's
/// canvas filter chain.
pub(crate) fn hsv(src: &RgbaImage, params: &Params) -> RgbaImage {
    let hue = params::f32(params, "hue", 0.0).to_radians();
    let saturation = params::f32(params, "sat", 100.0) / 100.0;
    let brightness = params::f32(params, "val", 100.0) / 100.0;

    let (sin, cos) = hue.sin_cos();
    let hue_matrix = [
        [
            0.213 + cos * 0.787 - sin * 0.213,
            0.715 - cos * 0.715 - sin * 0.715,
            0.072 - cos * 0.072 + sin * 0.928,
        ],
        [
            0.213 - cos * 0.213 + sin * 0.143,
            0.715 + cos * 0.285 + sin * 0.140,
            0.072 - cos * 0.072 - sin * 0.283,
        ],
        [
            0.213 - cos * 0.213 - sin * 0.787,
            0.715 - cos * 0.715 + sin * 0.715,
            0.072 + cos * 0.928 + sin * 0.072,
        ],
    ];
    let sat_matrix = [
        [
            0.213 + 0.787 * saturation,
            0.715 - 0.715 * saturation,
            0.072 - 0.072 * saturation,
        ],
        [
            0.213 - 0.213 * saturation,
            0.715 + 0.285 * saturation,
            0.072 - 0.072 * saturation,
        ],
        [
            0.213 - 0.213 * saturation,
            0.715 - 0.715 * saturation,
            0.072 + 0.928 * saturation,
        ],
    ];

    let mut out = src.clone();
    for pixel in out.pixels_mut() {
        let mut rgb = [
            byte_to_unit(pixel[0]),
            byte_to_unit(pixel[1]),
            byte_to_unit(pixel[2]),
        ];

        rgb = multiply_matrix(&hue_matrix, rgb);
        rgb = multiply_matrix(&sat_matrix, rgb);
        for value in &mut rgb {
            *value = (*value * brightness).clamp(0.0, 1.0);
        }

        for channel in 0..3 {
            pixel[channel] = unit_to_u8_round(rgb[channel]);
        }
    }
    out
}

fn multiply_matrix(matrix: &[[f32; 3]; 3], rgb: [f32; 3]) -> [f32; 3] {
    [
        matrix[0][0] * rgb[0] + matrix[0][1] * rgb[1] + matrix[0][2] * rgb[2],
        matrix[1][0] * rgb[0] + matrix[1][1] * rgb[1] + matrix[1][2] * rgb[2],
        matrix[2][0] * rgb[0] + matrix[2][1] * rgb[1] + matrix[2][2] * rgb[2],
    ]
}

pub(crate) fn color_balance(src: &RgbaImage, params: &Params) -> RgbaImage {
    let range = params::string(params, "range", "midtones");
    let cr = params::f32(params, "cyanRed", 0.0).clamp(-100.0, 100.0) / 100.0;
    let mg = params::f32(params, "magentaGreen", 0.0).clamp(-100.0, 100.0) / 100.0;
    let yb = params::f32(params, "yellowBlue", 0.0).clamp(-100.0, 100.0) / 100.0;
    let preserve = params::string(params, "preserve", "yes") != "no";

    let mut out = src.clone();
    for pixel in out.pixels_mut() {
        let r = pixel[0] as f32 / 255.0;
        let g = pixel[1] as f32 / 255.0;
        let b = pixel[2] as f32 / 255.0;
        let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let weight = match range {
            "shadows" => (1.0 - lum) * (1.0 - lum),
            "highlights" => lum * lum,
            _ => 4.0 * lum * (1.0 - lum),
        };

        let mut nr = r + cr * 0.5 * weight;
        let mut ng = g + mg * 0.5 * weight;
        let mut nb = b + yb * 0.5 * weight;

        if preserve {
            let next_lum = 0.2126 * nr + 0.7152 * ng + 0.0722 * nb;
            if next_lum > 0.0001 {
                let scale = lum / next_lum;
                nr *= scale;
                ng *= scale;
                nb *= scale;
            }
        }

        pixel[0] = unit_to_u8_round(nr);
        pixel[1] = unit_to_u8_round(ng);
        pixel[2] = unit_to_u8_round(nb);
    }
    out
}

pub(crate) fn gradient_map(src: &RgbaImage, params: &Params) -> RgbaImage {
    let midpoint = clamp01(params::f32(params, "midpoint", 50.0) / 100.0);
    let reverse = params::string(params, "reverse", "no") == "yes";
    let shadow = params::color(params, "color1", "#000000");
    let mid = params::color(params, "color2", "#808080");
    let highlight = params::color(params, "color3", "#ffffff");

    let mut lut = [[0u8; 3]; 256];
    for (value, slot) in lut.iter_mut().enumerate() {
        let mut t = value as f32 / 255.0;
        if reverse {
            t = 1.0 - t;
        }
        let (from, to, local) = if t <= midpoint {
            (
                shadow,
                mid,
                if midpoint <= 0.0 { 0.0 } else { t / midpoint },
            )
        } else {
            (
                mid,
                highlight,
                if midpoint >= 1.0 {
                    0.0
                } else {
                    (t - midpoint) / (1.0 - midpoint)
                },
            )
        };
        for channel in 0..3 {
            slot[channel] = (from[channel] as f32
                + (to[channel] as f32 - from[channel] as f32) * local)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }

    let mut out = src.clone();
    for pixel in out.pixels_mut() {
        let lum = 0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32;
        let entry = &lut[lum.round().clamp(0.0, 255.0) as usize];
        pixel[0] = entry[0];
        pixel[1] = entry[1];
        pixel[2] = entry[2];
    }
    out
}
