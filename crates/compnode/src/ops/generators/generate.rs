//! Input-less generators: Gradient and Noise Texture.

use image::{Rgba, RgbaImage};

use crate::ops::math::unit_to_u8_round;
use crate::params::{self, Params};

fn sample_stops(stops: &[(f32, [u8; 3])], t: f32) -> [u8; 3] {
    if t <= stops[0].0 {
        return stops[0].1;
    }
    for pair in stops.windows(2) {
        let (start, from) = pair[0];
        let (end, to) = pair[1];
        if t >= start && t <= end {
            if end - start <= 1e-6 {
                return to;
            }
            let local = (t - start) / (end - start);
            let mut out = [0u8; 3];
            for channel in 0..3 {
                out[channel] = (from[channel] as f32
                    + (to[channel] as f32 - from[channel] as f32) * local)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
            return out;
        }
    }
    stops[stops.len() - 1].1
}

pub(crate) fn gradient(params: &Params) -> RgbaImage {
    let width = params::u32(params, "width", 1280).clamp(100, 2000);
    let height = params::u32(params, "height", 720).clamp(100, 2000);
    let mode = params::string(params, "mode", "linear");
    let color1 = params::color(params, "color1", "#000000");
    let color2 = params::color(params, "color2", "#ffffff");
    let mut out = RgbaImage::new(width, height);

    if mode == "solid" {
        for pixel in out.pixels_mut() {
            *pixel = Rgba([color1[0], color1[1], color1[2], 255]);
        }
        return out;
    }

    // Positions are stored in -200..200 %, normalized to 0..1 like the
    // reference. Defaults span the full range.
    let normalize = |value: f32| ((value + 200.0) / 400.0).clamp(0.0, 1.0);
    let pos1 = normalize(params::f32(params, "color1Position", -200.0));
    let pos2 = normalize(params::f32(params, "color2Position", 200.0));
    let stops = [
        (0.0, color1),
        (pos1.min(pos2), color1),
        (pos1.max(pos2), color2),
        (1.0, color2),
    ];

    let (width_f, height_f) = (width as f32, height as f32);
    match mode {
        "radial" => {
            let center = (width_f / 2.0, height_f / 2.0);
            let radius = (width_f * width_f + height_f * height_f).sqrt() / 2.0;
            for (x, y, pixel) in out.enumerate_pixels_mut() {
                let dx = x as f32 + 0.5 - center.0;
                let dy = y as f32 + 0.5 - center.1;
                let t = (dx * dx + dy * dy).sqrt() / radius.max(1e-6);
                let color = sample_stops(&stops, t);
                *pixel = Rgba([color[0], color[1], color[2], 255]);
            }
        }
        _ => {
            let angle = params::f32(params, "angle", 90.0).to_radians();
            let dx = angle.cos() * width_f / 2.0;
            let dy = angle.sin() * height_f / 2.0;
            let (start_x, start_y) = (width_f / 2.0 - dx, height_f / 2.0 - dy);
            let (dir_x, dir_y) = (2.0 * dx, 2.0 * dy);
            let denom = (dir_x * dir_x + dir_y * dir_y).max(1e-6);
            for (x, y, pixel) in out.enumerate_pixels_mut() {
                let px = x as f32 + 0.5 - start_x;
                let py = y as f32 + 0.5 - start_y;
                let t = ((px * dir_x + py * dir_y) / denom).clamp(0.0, 1.0);
                let color = sample_stops(&stops, t);
                *pixel = Rgba([color[0], color[1], color[2], 255]);
            }
        }
    }
    out
}

fn hash2(ix: i32, iy: i32, seed: i32) -> f32 {
    let mut h = ix
        .wrapping_mul(374_761_393)
        .wrapping_add(iy.wrapping_mul(668_265_263))
        .wrapping_add(seed.wrapping_mul(1_442_695_041));
    h = (h ^ (((h as u32) >> 13) as i32)).wrapping_mul(1_274_126_177);
    (((h ^ (((h as u32) >> 16) as i32)) as u32) as f32) / 4_294_967_296.0
}

fn value_noise(x: f32, y: f32, seed: i32) -> f32 {
    let xi = x.floor();
    let yi = y.floor();
    let xf = x - xi;
    let yf = y - yi;
    let u = xf * xf * (3.0 - 2.0 * xf);
    let v = yf * yf * (3.0 - 2.0 * yf);
    let (xi, yi) = (xi as i32, yi as i32);
    let a = hash2(xi, yi, seed);
    let b = hash2(xi + 1, yi, seed);
    let c = hash2(xi, yi + 1, seed);
    let d = hash2(xi + 1, yi + 1, seed);
    a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v
}

fn gradient_noise(x: f32, y: f32, seed: i32) -> f32 {
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (xf, yf) = (x - x.floor(), y - y.floor());
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let dot = |gx: i32, gy: i32, dx: f32, dy: f32| {
        let angle = hash2(gx, gy, seed) * std::f32::consts::TAU;
        angle.cos() * dx + angle.sin() * dy
    };
    let u = fade(xf);
    let v = fade(yf);
    let a = dot(ix, iy, xf, yf);
    let b = dot(ix + 1, iy, xf - 1.0, yf);
    let c = dot(ix, iy + 1, xf, yf - 1.0);
    let d = dot(ix + 1, iy + 1, xf - 1.0, yf - 1.0);
    let top = a + (b - a) * u;
    let bottom = c + (d - c) * u;
    ((top + (bottom - top) * v) * 0.7071 + 0.5).clamp(0.0, 1.0)
}

fn simplex_noise(x: f32, y: f32, seed: i32) -> f32 {
    const F2: f32 = 0.366_025_4;
    const G2: f32 = 0.211_324_87;
    let skew = (x + y) * F2;
    let (i, j) = ((x + skew).floor() as i32, (y + skew).floor() as i32);
    let unskew = (i + j) as f32 * G2;
    let (x0, y0) = (x - (i as f32 - unskew), y - (j as f32 - unskew));
    let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
    let (x1, y1) = (x0 - i1 as f32 + G2, y0 - j1 as f32 + G2);
    let (x2, y2) = (x0 - 1.0 + 2.0 * G2, y0 - 1.0 + 2.0 * G2);
    let contribution = |gx: i32, gy: i32, dx: f32, dy: f32| {
        let attenuation = 0.5 - dx * dx - dy * dy;
        if attenuation <= 0.0 {
            return 0.0;
        }
        let angle = hash2(gx, gy, seed) * std::f32::consts::TAU;
        let gradient = angle.cos() * dx + angle.sin() * dy;
        attenuation.powi(4) * gradient
    };
    let value = 70.0
        * (contribution(i, j, x0, y0)
            + contribution(i + i1, j + j1, x1, y1)
            + contribution(i + 1, j + 1, x2, y2));
    (value * 0.5 + 0.5).clamp(0.0, 1.0)
}

pub(crate) fn noise(params: &Params, canvas: (u32, u32)) -> RgbaImage {
    let width = canvas.0.max(1);
    let height = canvas.1.max(1);
    let scale = params::f32(params, "scale", 5.0).max(0.01);
    let octaves = params::f32(params, "detail", 4.0).round().clamp(1.0, 8.0) as u32;
    let roughness = params::f32(params, "roughness", 0.5).clamp(0.0, 1.0);
    let lacunarity = params::f32(params, "lacunarity", 2.0).clamp(1.0, 10.0);
    let distortion = params::f32(params, "distortion", 0.0).max(0.0);
    let seed = params::f32(params, "seed", 0.0).round() as i32;
    let kind = params::string(params, "kind", "fbm");

    let step = scale / width.min(height) as f32;
    let mut out = RgbaImage::new(width, height);

    for (x, y, pixel) in out.enumerate_pixels_mut() {
        let mut nx = x as f32 * step;
        let mut ny = y as f32 * step;

        if distortion > 0.0 {
            let amount = distortion * 0.15;
            let dx = (value_noise(nx * 0.7 + 11.3, ny * 0.7 - 7.9, seed + 101) - 0.5) * amount;
            let dy = (value_noise(nx * 0.7 - 3.1, ny * 0.7 + 5.7, seed + 211) - 0.5) * amount;
            nx += dx;
            ny += dy;
        }

        let mut sum = 0.0;
        let mut amplitude = 1.0;
        let mut frequency = 1.0;
        let mut norm = 0.0;
        for octave in 0..octaves {
            let sample = match kind {
                "perlin" => {
                    gradient_noise(nx * frequency, ny * frequency, seed + octave as i32 * 173)
                }
                "simplex" => {
                    simplex_noise(nx * frequency, ny * frequency, seed + octave as i32 * 173)
                }
                _ => value_noise(nx * frequency, ny * frequency, seed + octave as i32 * 173),
            };
            let mut n = sample;
            if kind == "ridged" {
                n = 1.0 - (n * 2.0 - 1.0).abs();
            } else if kind == "multifractal" {
                n *= 0.5 + 0.5 * n;
            }
            sum += n * amplitude;
            norm += amplitude;
            amplitude *= roughness;
            frequency *= lacunarity;
        }

        let mut value = if norm > 0.0 { sum / norm } else { 0.0 };
        value = value * value * (3.0 - 2.0 * value);
        let gray = unit_to_u8_round(value);
        *pixel = Rgba([gray, gray, gray, 255]);
    }
    out
}
