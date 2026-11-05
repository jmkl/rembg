//! Raw-style development controls: white balance, texture and clarity,
//! dehaze, sharpening and noise reduction (the Raw Filter node of the
//! original compnode).
//!
//! The pipeline is the original one: noise reduction, white balance and
//! dehaze, texture, clarity, sharpening. It runs on the sRGB-encoded straight
//! color (the constants were tuned there), clamping to `0..=1` after each
//! stage, and the result is decoded and premultiplied again. Sliders use
//! `-1..1` where the original used `-100..100`.
use crate::buffer::ImageBuf;
use crate::cpu::{dec, enc, luma};

/// Clamped slider values plus every derived constant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RawSettings {
    /// `-1..1`, negative is cooler (bluer).
    pub temp: f32,
    /// `-1..1`, negative is greener.
    pub tint: f32,
    /// `-1..1`, fine local contrast.
    pub texture: f32,
    /// `-1..1`, coarse local contrast.
    pub clarity: f32,
    /// `-1..1`, negative adds haze.
    pub dehaze: f32,
    /// `0..1.5`.
    pub sharpen: f32,
    /// Gaussian sigma in pixels, `0.5..3`.
    pub sharpen_radius: f32,
    /// `0..1`, higher sharpens finer detail.
    pub sharpen_detail: f32,
    /// `0..1`.
    pub noise_reduction: f32,
    /// `0..1`, higher keeps more detail.
    pub noise_reduction_detail: f32,
}

impl Default for RawSettings {
    fn default() -> Self {
        RawSettings {
            temp: 0.0,
            tint: 0.0,
            texture: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            sharpen: 0.0,
            sharpen_radius: 1.0,
            sharpen_detail: 0.25,
            noise_reduction: 0.0,
            noise_reduction_detail: 0.5,
        }
    }
}

impl RawSettings {
    /// Every value clamped to its range; non-finite values become the default.
    pub fn sanitized(self) -> RawSettings {
        let d = RawSettings::default();
        let f = |v: f32, lo: f32, hi: f32, default: f32| if v.is_finite() { v.clamp(lo, hi) } else { default };
        RawSettings {
            temp: f(self.temp, -1.0, 1.0, d.temp),
            tint: f(self.tint, -1.0, 1.0, d.tint),
            texture: f(self.texture, -1.0, 1.0, d.texture),
            clarity: f(self.clarity, -1.0, 1.0, d.clarity),
            dehaze: f(self.dehaze, -1.0, 1.0, d.dehaze),
            sharpen: f(self.sharpen, 0.0, 1.5, d.sharpen),
            sharpen_radius: f(self.sharpen_radius, 0.5, 3.0, d.sharpen_radius),
            sharpen_detail: f(self.sharpen_detail, 0.0, 1.0, d.sharpen_detail),
            noise_reduction: f(self.noise_reduction, 0.0, 1.0, d.noise_reduction),
            noise_reduction_detail: f(self.noise_reduction_detail, 0.0, 1.0, d.noise_reduction_detail),
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

    fn wb_active(&self) -> bool {
        self.temp != 0.0 || self.tint != 0.0
    }

    /// Per-channel gains, divided by their luminance so brightness is stable.
    pub fn wb_gains(&self) -> [f32; 3] {
        let g = [
            (1.0 + 0.35 * self.temp + 0.10 * self.tint).max(0.01),
            (1.0 - 0.25 * self.tint).max(0.01),
            (1.0 - 0.35 * self.temp + 0.10 * self.tint).max(0.01),
        ];
        let norm = luma(g).max(0.01);
        [g[0] / norm, g[1] / norm, g[2] / norm]
    }

    fn texture_amount(&self) -> f32 {
        self.texture * 0.9
    }
    fn clarity_amount(&self) -> f32 {
        self.clarity * 0.7
    }
    fn sharpen_strength(&self) -> f32 {
        self.sharpen * 1.3
    }
    fn sharpen_threshold(&self) -> f32 {
        0.001 + (1.0 - self.sharpen_detail).powi(2) * 0.08
    }
    fn nr_threshold(&self) -> f32 {
        0.02 + (1.0 - self.noise_reduction_detail) * 0.14
    }
}

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

/// Separable Gaussian on a single-channel plane, edges extended.
fn blur_plane(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let k = gaussian_kernel(sigma);
    let r = (k.len() / 2) as isize;
    let mut tmp = vec![0.0_f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (j, wt) in k.iter().enumerate() {
                let xx = (x as isize + j as isize - r).clamp(0, w as isize - 1) as usize;
                acc += wt * src[y * w + xx];
            }
            tmp[y * w + x] = acc;
        }
    }
    let mut out = vec![0.0_f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (j, wt) in k.iter().enumerate() {
                let yy = (y as isize + j as isize - r).clamp(0, h as isize - 1) as usize;
                acc += wt * tmp[yy * w + x];
            }
            out[y * w + x] = acc;
        }
    }
    out
}

fn luma_plane(buf: &[[f32; 3]]) -> Vec<f32> {
    buf.iter().map(|p| luma(*p)).collect()
}

fn noise_reduction(buf: &mut [[f32; 3]], w: usize, h: usize, s: &RawSettings) {
    let amount = s.noise_reduction;
    if amount <= 0.0 {
        return;
    }
    let threshold = s.nr_threshold();
    let blurred: [Vec<f32>; 3] = std::array::from_fn(|c| {
        let plane: Vec<f32> = buf.iter().map(|p| p[c]).collect();
        blur_plane(&plane, w, h, 1.2)
    });
    for (i, px) in buf.iter_mut().enumerate() {
        let b = [blurred[0][i], blurred[1][i], blurred[2][i]];
        let diff = luma(*px) - luma(b);
        let edge = (diff.abs() / threshold).powi(2);
        let blend = amount / (1.0 + 4.0 * edge);
        for c in 0..3 {
            px[c] = (px[c] + (b[c] - px[c]) * blend).clamp(0.0, 1.0);
        }
    }
}

fn dehaze_px(rgb: &mut [f32; 3], amount: f32) {
    if amount > 0.0 {
        let black = 0.12 * amount;
        let contrast = 1.0 + 0.30 * amount;
        let saturation = 1.0 + 0.30 * amount;
        for v in rgb.iter_mut() {
            *v = (*v - black) / (1.0 - black).max(1e-4);
            *v = (*v - 0.5) * contrast + 0.5;
        }
        let l = luma(*rgb);
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
        *v = v.clamp(0.0, 1.0);
    }
}

/// White balance and dehaze.
fn color_stage(buf: &mut [[f32; 3]], s: &RawSettings) {
    let wb = s.wb_active();
    let gains = s.wb_gains();
    if !wb && s.dehaze == 0.0 {
        return;
    }
    for px in buf.iter_mut() {
        if wb {
            for c in 0..3 {
                px[c] = (px[c] * gains[c]).clamp(0.0, 1.0);
            }
        }
        dehaze_px(px, s.dehaze);
    }
}

fn local_contrast(buf: &mut [[f32; 3]], w: usize, h: usize, sigma: f32, amount: f32, bias: f32) {
    if amount == 0.0 {
        return;
    }
    let l = luma_plane(buf);
    let b = blur_plane(&l, w, h, sigma);
    for (i, px) in buf.iter_mut().enumerate() {
        let detail = l[i] - b[i];
        let midtones = 4.0 * l[i] * (1.0 - l[i]);
        let weight = 1.0 - bias + bias * midtones;
        let delta = detail * amount * weight;
        for c in 0..3 {
            px[c] = (px[c] + delta).clamp(0.0, 1.0);
        }
    }
}

fn sharpen(buf: &mut [[f32; 3]], w: usize, h: usize, s: &RawSettings) {
    if s.sharpen <= 0.0 {
        return;
    }
    let strength = s.sharpen_strength();
    let threshold = s.sharpen_threshold();
    let l = luma_plane(buf);
    let b = blur_plane(&l, w, h, s.sharpen_radius);
    for (i, px) in buf.iter_mut().enumerate() {
        let detail = l[i] - b[i];
        let mask = (detail.abs() / threshold).min(1.0);
        let delta = detail * strength * mask;
        for c in 0..3 {
            px[c] = (px[c] + delta).clamp(0.0, 1.0);
        }
    }
}

/// Apply the Raw Filter pipeline. Settings are sanitized first.
pub fn raw_filter(img: &ImageBuf, settings: RawSettings) -> ImageBuf {
    let s = settings.sanitized();
    if s.is_identity() || img.width == 0 || img.height == 0 {
        return img.clone();
    }
    let (w, h) = (img.width as usize, img.height as usize);
    let alpha: Vec<f32> = img.data.iter().map(|p| p[3]).collect();
    let mut buf: Vec<[f32; 3]> = img
        .data
        .iter()
        .map(|p| {
            if p[3] > 0.0 {
                [enc(p[0] / p[3]), enc(p[1] / p[3]), enc(p[2] / p[3])]
            } else {
                [0.0; 3]
            }
        })
        .collect();

    noise_reduction(&mut buf, w, h, &s);
    color_stage(&mut buf, &s);
    local_contrast(&mut buf, w, h, 1.2, s.texture_amount(), 0.0);
    local_contrast(&mut buf, w, h, 6.0, s.clarity_amount(), 0.6);
    sharpen(&mut buf, w, h, &s);

    let mut out = ImageBuf::new(img.width, img.height);
    for (i, p) in buf.iter().enumerate() {
        let a = alpha[i];
        if a > 0.0 {
            out.data[i] = [dec(p[0]) * a, dec(p[1]) * a, dec(p[2]) * a, a];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use compnode_core::Color;

    fn gray(v: f32) -> ImageBuf {
        ImageBuf::solid(8, 8, Color::rgb(v, v, v))
    }

    /// Left half dark, right half bright (a vertical edge).
    fn edge() -> ImageBuf {
        ImageBuf::from_fn(16, 8, |x, _| if x < 8 { [0.1, 0.1, 0.1, 1.0] } else { [0.6, 0.6, 0.6, 1.0] })
    }

    #[test]
    fn identity_settings_return_the_input() {
        let img = edge();
        assert_eq!(raw_filter(&img, RawSettings::default()), img);
        // out-of-range and non-finite values are sanitized, not propagated
        let s = RawSettings { temp: f32::NAN, sharpen: -4.0, ..RawSettings::default() }.sanitized();
        assert!(s.is_identity());
    }

    #[test]
    fn warm_temperature_raises_red_and_lowers_blue() {
        let out = raw_filter(&gray(0.2), RawSettings { temp: 0.8, ..RawSettings::default() }).get(0, 0);
        assert!(out[0] > 0.2 && out[2] < 0.2, "{out:?}");
        let cool = raw_filter(&gray(0.2), RawSettings { temp: -0.8, ..RawSettings::default() }).get(0, 0);
        assert!(cool[2] > 0.2 && cool[0] < 0.2, "{cool:?}");
    }

    #[test]
    fn white_balance_gains_keep_luminance() {
        for (t, n) in [(0.5, 0.0), (-0.7, 0.3), (1.0, -1.0)] {
            let g = RawSettings { temp: t, tint: n, ..RawSettings::default() }.wb_gains();
            assert!((luma(g) - 1.0).abs() < 1e-4, "{g:?}");
        }
    }

    #[test]
    fn sharpen_increases_the_contrast_across_an_edge() {
        let img = edge();
        let out = raw_filter(&img, RawSettings { sharpen: 1.5, sharpen_detail: 1.0, ..RawSettings::default() });
        // pixels next to the edge move apart; flat areas far from it do not
        assert!(out.get(7, 4)[0] < img.get(7, 4)[0]);
        assert!(out.get(8, 4)[0] > img.get(8, 4)[0]);
        assert!((out.get(0, 4)[0] - img.get(0, 4)[0]).abs() < 1e-4);
    }

    #[test]
    fn noise_reduction_smooths_noise_but_keeps_a_strong_edge() {
        let noisy = ImageBuf::from_fn(16, 16, |x, y| {
            let v = if (x + y) % 2 == 0 { 0.30 } else { 0.34 };
            [v, v, v, 1.0]
        });
        let out = raw_filter(&noisy, RawSettings { noise_reduction: 1.0, noise_reduction_detail: 0.0, ..RawSettings::default() });
        let spread = |i: &ImageBuf| {
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for y in 4..12 {
                for x in 4..12 {
                    let v = i.get(x, y)[0];
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            hi - lo
        };
        assert!(spread(&out) < spread(&noisy) * 0.5);
        let e = edge();
        let kept = raw_filter(&e, RawSettings { noise_reduction: 1.0, noise_reduction_detail: 1.0, ..RawSettings::default() });
        assert!(kept.get(8, 4)[0] - kept.get(7, 4)[0] > 0.3);
    }

    #[test]
    fn dehaze_changes_contrast_in_both_directions_and_stays_in_range() {
        let img = edge();
        let spread = |i: &ImageBuf| i.get(12, 4)[0] - i.get(2, 4)[0];
        let more = raw_filter(&img, RawSettings { dehaze: 1.0, ..RawSettings::default() });
        let less = raw_filter(&img, RawSettings { dehaze: -1.0, ..RawSettings::default() });
        assert!(spread(&more) > spread(&img));
        assert!(spread(&less) < spread(&img));
        for out in [&more, &less] {
            assert!(out.data.iter().flatten().all(|v| v.is_finite() && (0.0..=1.0001).contains(v)));
        }
    }

    #[test]
    fn alpha_is_kept_and_transparent_pixels_stay_transparent() {
        let mut img = ImageBuf::from_fn(8, 8, |_, _| [0.1, 0.1, 0.1, 0.5]);
        img.set(0, 0, [0.0; 4]);
        let out = raw_filter(&img, RawSettings { temp: 0.5, clarity: 0.5, sharpen: 1.0, ..RawSettings::default() });
        assert_eq!(out.get(0, 0), [0.0; 4]);
        assert!((out.get(3, 3)[3] - 0.5).abs() < 1e-6);
    }
}
