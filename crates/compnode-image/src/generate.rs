//! Input-less generators ported from the original compnode: Gradient and
//! Noise. Both fill the whole canvas with opaque pixels.
//!
//! Colors are interpolated in sRGB space (like the original, which blended
//! 8-bit values) and stored linear. The noise functions and their hash are
//! the original ones, so a seed reproduces the same pattern.
use crate::buffer::ImageBuf;
use crate::cpu::{dec, enc};
use compnode_core::Color;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GradientMode {
    Linear,
    Radial,
    Solid,
}

impl GradientMode {
    pub const ALL: [GradientMode; 3] = [GradientMode::Linear, GradientMode::Radial, GradientMode::Solid];
    pub fn name(self) -> &'static str {
        match self {
            GradientMode::Linear => "Linear",
            GradientMode::Radial => "Radial",
            GradientMode::Solid => "Solid",
        }
    }
    pub fn from_index(i: usize) -> GradientMode {
        GradientMode::ALL.get(i).copied().unwrap_or(GradientMode::Linear)
    }
}

fn encoded(c: Color) -> [f32; 3] {
    [enc(c.r), enc(c.g), enc(c.b)]
}

fn sample_stops(stops: &[(f32, [f32; 3])], t: f32) -> [f32; 3] {
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
            let k = (t - start) / (end - start);
            return [from[0] + (to[0] - from[0]) * k, from[1] + (to[1] - from[1]) * k, from[2] + (to[2] - from[2]) * k];
        }
    }
    stops[stops.len() - 1].1
}

fn opaque(c: [f32; 3]) -> [f32; 4] {
    [dec(c[0]), dec(c[1]), dec(c[2]), 1.0]
}

/// A `width x height` gradient. `angle_deg` is the direction of a linear
/// gradient (90 runs top to bottom). `pos1` and `pos2` (`0..1`) are where
/// `color1` ends and `color2` starts; the defaults `0` and `1` blend across
/// the whole image. Solid fills with `color1`.
#[allow(clippy::too_many_arguments)]
pub fn gradient(
    width: u32,
    height: u32,
    mode: GradientMode,
    color1: Color,
    color2: Color,
    angle_deg: f32,
    pos1: f32,
    pos2: f32,
) -> ImageBuf {
    let (width, height) = (width.max(1), height.max(1));
    let (c1, c2) = (encoded(color1), encoded(color2));
    if mode == GradientMode::Solid {
        let p = opaque(c1);
        return ImageBuf::from_fn(width, height, |_, _| p);
    }
    let (p1, p2) = (pos1.clamp(0.0, 1.0), pos2.clamp(0.0, 1.0));
    let stops = [(0.0, c1), (p1.min(p2), c1), (p1.max(p2), c2), (1.0, c2)];
    let (wf, hf) = (width as f32, height as f32);
    match mode {
        GradientMode::Radial => {
            let radius = ((wf * wf + hf * hf).sqrt() / 2.0).max(1e-6);
            ImageBuf::from_fn(width, height, |x, y| {
                let dx = x as f32 + 0.5 - wf / 2.0;
                let dy = y as f32 + 0.5 - hf / 2.0;
                opaque(sample_stops(&stops, (dx * dx + dy * dy).sqrt() / radius))
            })
        }
        _ => {
            let a = angle_deg.to_radians();
            let (dx, dy) = (a.cos() * wf / 2.0, a.sin() * hf / 2.0);
            let (sx, sy) = (wf / 2.0 - dx, hf / 2.0 - dy);
            let (vx, vy) = (2.0 * dx, 2.0 * dy);
            let denom = (vx * vx + vy * vy).max(1e-6);
            ImageBuf::from_fn(width, height, |x, y| {
                let px = x as f32 + 0.5 - sx;
                let py = y as f32 + 0.5 - sy;
                opaque(sample_stops(&stops, ((px * vx + py * vy) / denom).clamp(0.0, 1.0)))
            })
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoiseKind {
    Fbm,
    Perlin,
    Simplex,
    Multifractal,
    Ridged,
}

impl NoiseKind {
    pub const ALL: [NoiseKind; 5] =
        [NoiseKind::Fbm, NoiseKind::Perlin, NoiseKind::Simplex, NoiseKind::Multifractal, NoiseKind::Ridged];
    pub fn name(self) -> &'static str {
        match self {
            NoiseKind::Fbm => "fBM",
            NoiseKind::Perlin => "Perlin",
            NoiseKind::Simplex => "Simplex",
            NoiseKind::Multifractal => "Multifractal",
            NoiseKind::Ridged => "Ridged",
        }
    }
    pub fn from_index(i: usize) -> NoiseKind {
        NoiseKind::ALL.get(i).copied().unwrap_or(NoiseKind::Fbm)
    }
}

/// Noise settings. See the Noise node for ranges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoiseParams {
    pub kind: NoiseKind,
    pub scale: f32,
    pub octaves: u32,
    pub roughness: f32,
    pub lacunarity: f32,
    pub distortion: f32,
    pub seed: i32,
}

impl Default for NoiseParams {
    fn default() -> Self {
        NoiseParams {
            kind: NoiseKind::Fbm,
            scale: 5.0,
            octaves: 4,
            roughness: 0.5,
            lacunarity: 2.0,
            distortion: 0.0,
            seed: 0,
        }
    }
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
    let (xi, yi) = (x.floor(), y.floor());
    let (xf, yf) = (x - xi, y - yi);
    let u = xf * xf * (3.0 - 2.0 * xf);
    let v = yf * yf * (3.0 - 2.0 * yf);
    let (xi, yi) = (xi as i32, yi as i32);
    let a = hash2(xi, yi, seed);
    let b = hash2(xi.wrapping_add(1), yi, seed);
    let c = hash2(xi, yi.wrapping_add(1), seed);
    let d = hash2(xi.wrapping_add(1), yi.wrapping_add(1), seed);
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
    let (u, v) = (fade(xf), fade(yf));
    let a = dot(ix, iy, xf, yf);
    let b = dot(ix.wrapping_add(1), iy, xf - 1.0, yf);
    let c = dot(ix, iy.wrapping_add(1), xf, yf - 1.0);
    let d = dot(ix.wrapping_add(1), iy.wrapping_add(1), xf - 1.0, yf - 1.0);
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
        attenuation.powi(4) * (angle.cos() * dx + angle.sin() * dy)
    };
    let value = 70.0
        * (contribution(i, j, x0, y0)
            + contribution(i + i1, j + j1, x1, y1)
            + contribution(i + 1, j + 1, x2, y2));
    (value * 0.5 + 0.5).clamp(0.0, 1.0)
}

/// Gray noise filling a `width x height` canvas. The gray level is an sRGB
/// value (as in the original), stored linear.
pub fn noise(width: u32, height: u32, p: NoiseParams) -> ImageBuf {
    let (width, height) = (width.max(1), height.max(1));
    let scale = if p.scale.is_finite() { p.scale.max(0.01) } else { 5.0 };
    let octaves = p.octaves.clamp(1, 8);
    let roughness = p.roughness.clamp(0.0, 1.0);
    let lacunarity = p.lacunarity.clamp(1.0, 10.0);
    let distortion = if p.distortion.is_finite() { p.distortion.max(0.0) } else { 0.0 };
    let step = scale / width.min(height) as f32;

    ImageBuf::from_fn(width, height, |x, y| {
        let mut nx = x as f32 * step;
        let mut ny = y as f32 * step;
        if distortion > 0.0 {
            let amount = distortion * 0.15;
            // both offsets are taken from the undistorted position
            let dx = (value_noise(nx * 0.7 + 11.3, ny * 0.7 - 7.9, p.seed.wrapping_add(101)) - 0.5) * amount;
            let dy = (value_noise(nx * 0.7 - 3.1, ny * 0.7 + 5.7, p.seed.wrapping_add(211)) - 0.5) * amount;
            nx += dx;
            ny += dy;
        }
        let (mut sum, mut amplitude, mut frequency, mut norm) = (0.0, 1.0, 1.0, 0.0);
        for octave in 0..octaves {
            let seed = p.seed.wrapping_add(octave as i32 * 173);
            let (fx, fy) = (nx * frequency, ny * frequency);
            let mut n = match p.kind {
                NoiseKind::Perlin => gradient_noise(fx, fy, seed),
                NoiseKind::Simplex => simplex_noise(fx, fy, seed),
                _ => value_noise(fx, fy, seed),
            };
            match p.kind {
                NoiseKind::Ridged => n = 1.0 - (n * 2.0 - 1.0).abs(),
                NoiseKind::Multifractal => n *= 0.5 + 0.5 * n,
                _ => {}
            }
            sum += n * amplitude;
            norm += amplitude;
            amplitude *= roughness;
            frequency *= lacunarity;
        }
        let mut v = if norm > 0.0 { sum / norm } else { 0.0 };
        v = v * v * (3.0 - 2.0 * v);
        let g = dec(v);
        [g, g, g, 1.0]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 2e-3
    }

    #[test]
    fn linear_gradient_runs_from_the_first_to_the_second_color() {
        // 90 degrees = top to bottom, black to white
        let g = gradient(8, 64, GradientMode::Linear, Color::BLACK, Color::WHITE, 90.0, 0.0, 1.0);
        assert!(g.get(4, 0)[0] < 0.01);
        assert!(g.get(4, 63)[0] > 0.97);
        // every row is the same, and values rise monotonically
        assert!(near(g.get(0, 30)[0], g.get(7, 30)[0]));
        assert!((1..64).all(|y| g.get(2, y)[0] >= g.get(2, y - 1)[0] - 1e-6));
        assert!(g.data.iter().all(|p| p[3] == 1.0));
        // the midpoint of the ramp is the sRGB midpoint (about 0.2140 linear)
        let mid = gradient(1, 64, GradientMode::Linear, Color::BLACK, Color::WHITE, 90.0, 0.0, 1.0).get(0, 31)[0];
        assert!((mid - dec(0.5)).abs() < 0.02, "{mid}");
    }

    #[test]
    fn gradient_positions_hold_the_end_colors() {
        let g = gradient(1, 100, GradientMode::Linear, Color::BLACK, Color::WHITE, 90.0, 0.4, 0.6);
        assert!(g.get(0, 10)[0] < 1e-4); // before the first position: color 1
        assert!(g.get(0, 90)[0] > 0.9999); // after the second: color 2
        // swapped positions behave the same as ordered ones
        let s = gradient(1, 100, GradientMode::Linear, Color::BLACK, Color::WHITE, 90.0, 0.6, 0.4);
        assert_eq!(g, s);
    }

    #[test]
    fn radial_and_solid_gradients() {
        let r = gradient(33, 33, GradientMode::Radial, Color::WHITE, Color::BLACK, 0.0, 0.0, 1.0);
        assert!(r.get(16, 16)[0] > 0.98); // center is color 1
        assert!(r.get(0, 0)[0] < r.get(16, 16)[0]);
        let red = Color::rgb(1.0, 0.0, 0.0);
        let s = gradient(4, 4, GradientMode::Solid, red, Color::WHITE, 0.0, 0.0, 1.0);
        assert!(s.data.iter().all(|p| near(p[0], 1.0) && p[1].abs() < 1e-5 && p[3] == 1.0));
        // degenerate sizes produce one pixel
        assert_eq!(gradient(0, 0, GradientMode::Linear, Color::BLACK, Color::WHITE, 0.0, 0.0, 1.0).width, 1);
    }

    #[test]
    fn noise_is_repeatable_opaque_gray_in_range() {
        for kind in NoiseKind::ALL {
            let p = NoiseParams { kind, seed: 19, ..NoiseParams::default() };
            let a = noise(32, 24, p);
            let b = noise(32, 24, p);
            assert_eq!(a, b, "{kind:?}");
            assert!(a.data.iter().all(|px| px[3] == 1.0 && px[0] == px[1] && px[1] == px[2]), "{kind:?}");
            assert!(a.data.iter().all(|px| (0.0..=1.0).contains(&px[0])), "{kind:?}");
            // not a flat image
            let (lo, hi) = a.data.iter().fold((1.0_f32, 0.0_f32), |(lo, hi), px| (lo.min(px[0]), hi.max(px[0])));
            assert!(hi - lo > 0.05, "{kind:?} is flat: {lo}..{hi}");
        }
    }

    #[test]
    fn noise_depends_on_the_seed_and_survives_odd_settings() {
        let a = noise(16, 16, NoiseParams { seed: 1, ..NoiseParams::default() });
        let b = noise(16, 16, NoiseParams { seed: 2, ..NoiseParams::default() });
        assert_ne!(a, b);
        let odd = NoiseParams {
            scale: f32::NAN,
            octaves: 99,
            roughness: 5.0,
            lacunarity: 0.0,
            distortion: f32::INFINITY,
            ..NoiseParams::default()
        };
        assert!(noise(8, 8, odd).data.iter().all(|p| p[0].is_finite()));
        assert_eq!(noise(0, 0, NoiseParams::default()).width, 1);
        // distortion changes the pattern
        let d = noise(16, 16, NoiseParams { distortion: 5.0, ..NoiseParams::default() });
        assert_ne!(d, noise(16, 16, NoiseParams::default()));
    }
}
