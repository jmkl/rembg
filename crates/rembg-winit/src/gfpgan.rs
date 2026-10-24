//! GFPGAN v1.4 face restoration on `ort = "=2.0.0-rc.12"`.
//!
//! Mirrors the Python path in main.py (facexlib FaceRestoreHelper):
//!   5 landmarks -> similarity transform onto the FFHQ-512 template
//!   -> warp 512x512 -> GFPGAN -> inverse warp -> feathered paste-back.
//!
//! GFPGAN only works on faces aligned like FFHQ (eyes/nose/mouth at fixed
//! positions). Resizing an arbitrary crop to 512x512 produces garbage.
//!
//! Cargo.toml:
//!   anyhow = "1"
//!   image  = "0.25"
//!   ort    = "=2.0.0-rc.12"   # keep your existing features (cuda, etc.)

use std::sync::Mutex;

use anyhow::{Result, anyhow, bail};
use image::{Rgb, RgbImage};
use ort::{session::Session, value::Tensor};

// Your existing items: adjust the path to wherever they live.
use crate::{MODEL_GFPGAN, SIZE_GFPGAN, VRAM_LIMIT_GFPGAN_MB, WARMUP, load_session, warmup};

/// 5 face landmarks in source-image pixels, in RetinaFace order:
/// left eye, right eye, nose tip, left mouth corner, right mouth corner
/// ("left" = left side of the image, not the subject's left).
pub type Landmarks = [[f32; 2]; 5];

/// 2x3 affine matrix, row-major: [x', y'] = [[a, b, tx], [c, d, ty]] * [x, y, 1].
type Affine = [[f64; 3]; 2];

/// facexlib's FFHQ template for a 512x512 face.
const FFHQ_512: [[f64; 2]; 5] = [
    [192.98138, 239.94708],
    [318.90277, 240.19360],
    [256.63416, 314.01935],
    [201.26117, 371.41043],
    [313.08905, 371.15118],
];

/// facexlib fills out-of-image areas of the aligned crop with BGR (135,133,132).
const BORDER_RGB: [f32; 3] = [132.0, 133.0, 135.0];

pub struct Gfpgan {
    // Only the GPU call holds the lock; CPU pre/post-processing runs unlocked.
    session: Mutex<Session>,
}

impl Gfpgan {
    pub fn load() -> Result<Self> {
        let mut session = load_session(MODEL_GFPGAN, VRAM_LIMIT_GFPGAN_MB)?;
        if WARMUP {
            warmup(&mut session, SIZE_GFPGAN)?;
        }
        Ok(Self {
            session: Mutex::new(session),
        })
    }

    /// Full pipeline: restore every face given by `faces` and paste it back.
    /// `strength` in 0..=1 blends the restored face over the original.
    pub fn restore_faces(
        &self,
        img: &RgbImage,
        faces: &[Landmarks],
        strength: f32,
    ) -> Result<RgbImage> {
        let mut out = img.clone();
        let strength = strength.clamp(0.0, 1.0);
        if strength <= 0.0 {
            return Ok(out);
        }
        let s = SIZE_GFPGAN;
        let scale = s as f64 / 512.0;

        let mut template = FFHQ_512;
        for p in template.iter_mut() {
            p[0] *= scale;
            p[1] *= scale;
        }

        for lm in faces {
            let src = lm.map(|p| [p[0] as f64, p[1] as f64]);
            let m = similarity(&src, &template)?;
            let aligned = warp_face(img, &m, s); // always from the ORIGINAL image
            let restored = self.restore_aligned(&aligned)?;
            paste_back(&mut out, &restored, &m, strength);
        }
        Ok(out)
    }

    /// Run GFPGAN on a face that is ALREADY FFHQ-aligned (size x size).
    /// Input and output are RGB.
    pub fn restore_aligned(&self, face: &RgbImage) -> Result<RgbImage> {
        let s = SIZE_GFPGAN;
        if face.dimensions() != (s, s) {
            bail!("aligned face must be {s}x{s}, got {:?}", face.dimensions());
        }
        let n = (s * s) as usize;

        // RGB u8 -> CHW f32 in [-1, 1]  ((v/255 - 0.5) / 0.5)
        let mut data = vec![0f32; 3 * n];
        for (i, p) in face.pixels().enumerate() {
            data[i] = p[0] as f32 / 127.5 - 1.0;
            data[n + i] = p[1] as f32 / 127.5 - 1.0;
            data[2 * n + i] = p[2] as f32 / 127.5 - 1.0;
        }
        let input = Tensor::from_array(([1usize, 3, s as usize, s as usize], data))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| anyhow!("GFPGAN session mutex poisoned"))?;
        let outputs = session.run(ort::inputs![input])?;
        let (shape, out) = outputs[0].try_extract_tensor::<f32>()?;

        if shape.len() != 4
            || shape[0] != 1
            || shape[1] != 3
            || shape[2] != s as i64
            || shape[3] != s as i64
        {
            bail!("Unexpected GFPGAN output shape: {shape:?}");
        }
        if out.len() < 3 * n {
            bail!("GFPGAN output tensor is too small");
        }

        // [-1, 1] CHW -> [0, 255] RGB HWC
        let to_u8 = |v: f32| ((v.clamp(-1.0, 1.0) + 1.0) * 127.5).round() as u8;
        let mut result = RgbImage::new(s, s);
        for (i, p) in result.pixels_mut().enumerate() {
            *p = Rgb([to_u8(out[i]), to_u8(out[n + i]), to_u8(out[2 * n + i])]);
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// Least-squares similarity transform (rotation + uniform scale + translation)
/// mapping `src` onto `dst`. Closed form, equivalent to Umeyama without
/// reflection, and to cv2.estimateAffinePartial2D for clean landmarks.
fn similarity(src: &[[f64; 2]; 5], dst: &[[f64; 2]; 5]) -> Result<Affine> {
    let n = 5.0;
    let (mut msx, mut msy, mut mdx, mut mdy) = (0.0, 0.0, 0.0, 0.0);
    for i in 0..5 {
        msx += src[i][0];
        msy += src[i][1];
        mdx += dst[i][0];
        mdy += dst[i][1];
    }
    msx /= n;
    msy /= n;
    mdx /= n;
    mdy /= n;

    let (mut dot, mut cross, mut var) = (0.0, 0.0, 0.0);
    for i in 0..5 {
        let (sx, sy) = (src[i][0] - msx, src[i][1] - msy);
        let (dx, dy) = (dst[i][0] - mdx, dst[i][1] - mdy);
        dot += sx * dx + sy * dy;
        cross += sx * dy - sy * dx;
        var += sx * sx + sy * sy;
    }
    if var < 1e-9 {
        bail!("degenerate landmarks");
    }
    let (a, b) = (dot / var, cross / var); // a = s*cos(t), b = s*sin(t)
    Ok([
        [a, -b, mdx - (a * msx - b * msy)],
        [b, a, mdy - (b * msx + a * msy)],
    ])
}

fn invert(m: &Affine) -> Affine {
    let det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
    let a = m[1][1] / det;
    let b = -m[0][1] / det;
    let c = -m[1][0] / det;
    let d = m[0][0] / det;
    [
        [a, b, -(a * m[0][2] + b * m[1][2])],
        [c, d, -(c * m[0][2] + d * m[1][2])],
    ]
}

#[inline]
fn apply(m: &Affine, x: f64, y: f64) -> (f64, f64) {
    (
        m[0][0] * x + m[0][1] * y + m[0][2],
        m[1][0] * x + m[1][1] * y + m[1][2],
    )
}

/// Equivalent of cv2.warpAffine(img, m, (size, size), BORDER_CONSTANT, border):
/// `m` maps source -> destination; sampling is bilinear.
fn warp_face(img: &RgbImage, m: &Affine, size: u32) -> RgbImage {
    let inv = invert(m);
    let (w, h) = (img.width() as i64, img.height() as i64);
    let tap = |x: i64, y: i64| -> [f32; 3] {
        if x < 0 || y < 0 || x >= w || y >= h {
            BORDER_RGB
        } else {
            let p = img.get_pixel(x as u32, y as u32);
            [p[0] as f32, p[1] as f32, p[2] as f32]
        }
    };

    let mut out = RgbImage::new(size, size);
    for y in 0..size {
        for x in 0..size {
            let (sx, sy) = apply(&inv, x as f64, y as f64);
            let (fx0, fy0) = (sx.floor(), sy.floor());
            let (tx, ty) = ((sx - fx0) as f32, (sy - fy0) as f32);
            let (x0, y0) = (fx0 as i64, fy0 as i64);
            let (p00, p10) = (tap(x0, y0), tap(x0 + 1, y0));
            let (p01, p11) = (tap(x0, y0 + 1), tap(x0 + 1, y0 + 1));
            let mut px = [0u8; 3];
            for c in 0..3 {
                let v = p00[c] * (1.0 - tx) * (1.0 - ty)
                    + p10[c] * tx * (1.0 - ty)
                    + p01[c] * (1.0 - tx) * ty
                    + p11[c] * tx * ty;
                px[c] = v.round().clamp(0.0, 255.0) as u8;
            }
            out.put_pixel(x, y, Rgb(px));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Paste-back (facexlib's non-parse path: square mask, eroded and feathered)
// ---------------------------------------------------------------------------

/// Warp `face` (the restored, aligned crop) back into `dst` using the inverse
/// of `m` and blend with a feathered mask. Only the face's bounding box is
/// touched, so cost does not scale with image size.
fn paste_back(dst: &mut RgbImage, face: &RgbImage, m: &Affine, strength: f32) {
    let (iw, ih) = (dst.width() as i64, dst.height() as i64);
    let fs = face.width() as i64;
    let inv = invert(m);

    // Bounding box of the face square in image space.
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (cx, cy) in [
        (0.0, 0.0),
        (fs as f64, 0.0),
        (0.0, fs as f64),
        (fs as f64, fs as f64),
    ] {
        let (x, y) = apply(&inv, cx, cy);
        minx = minx.min(x);
        miny = miny.min(y);
        maxx = maxx.max(x);
        maxy = maxy.max(y);
    }
    let x0 = (minx.floor() as i64 - 3).max(0);
    let y0 = (miny.floor() as i64 - 3).max(0);
    let x1 = (maxx.ceil() as i64 + 3).min(iw);
    let y1 = (maxy.ceil() as i64 + 3).min(ih);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (bw, bh) = ((x1 - x0) as usize, (y1 - y0) as usize);

    // Inverse-warp the restored face and the "ones" coverage mask together.
    let mut mask = vec![0f32; bw * bh];
    let mut col = vec![[0f32; 3]; bw * bh];
    for by in 0..bh {
        for bx in 0..bw {
            let (u, v) = apply(m, (x0 + bx as i64) as f64, (y0 + by as i64) as f64);
            let (fu, fv) = (u.floor(), v.floor());
            let (tx, ty) = ((u - fu) as f32, (v - fv) as f32);
            let (iu, iv) = (fu as i64, fv as i64);
            let taps = [
                (iu, iv, (1.0 - tx) * (1.0 - ty)),
                (iu + 1, iv, tx * (1.0 - ty)),
                (iu, iv + 1, (1.0 - tx) * ty),
                (iu + 1, iv + 1, tx * ty),
            ];
            let (mut cov, mut rgb) = (0f32, [0f32; 3]);
            for (xx, yy, w) in taps {
                if xx >= 0 && yy >= 0 && xx < fs && yy < fs {
                    let p = face.get_pixel(xx as u32, yy as u32);
                    cov += w;
                    for k in 0..3 {
                        rgb[k] += w * p[k] as f32;
                    }
                }
            }
            let i = by * bw + bx;
            mask[i] = cov;
            if cov > 1e-6 {
                for k in 0..3 {
                    col[i][k] = rgb[k] / cov; // un-darken partially covered edge pixels
                }
            }
        }
    }

    // facexlib: erode 2x2 to drop the black border, then size the feather
    // from the face area, erode again and Gaussian-blur.
    let mask = erode(&mask, bw, bh, 2);
    let area: f32 = mask.iter().sum();
    let w_edge = ((area.sqrt() as usize) / 20).max(1);
    let mask = erode(&mask, bw, bh, w_edge * 2);
    let mask = gaussian_blur(&mask, bw, bh, w_edge);

    for by in 0..bh {
        for bx in 0..bw {
            let i = by * bw + bx;
            let a = (mask[i] * strength).clamp(0.0, 1.0);
            if a <= 0.0 {
                continue;
            }
            let p = dst.get_pixel_mut((x0 + bx as i64) as u32, (y0 + by as i64) as u32);
            for k in 0..3 {
                p[k] = (a * col[i][k] + (1.0 - a) * p[k] as f32)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Square-kernel erosion (min filter), separable. Out-of-range taps ignored
/// (cv2.erode's default border behaviour). Anchor at k/2, like OpenCV.
fn erode(src: &[f32], w: usize, h: usize, k: usize) -> Vec<f32> {
    let lo = (k / 2) as i64;
    let hi = k as i64 - 1 - lo;
    let mut tmp = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut v = f32::MAX;
            for o in -lo..=hi {
                let xx = x as i64 + o;
                if xx >= 0 && (xx as usize) < w {
                    v = v.min(src[y * w + xx as usize]);
                }
            }
            tmp[y * w + x] = v;
        }
    }
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut v = f32::MAX;
            for o in -lo..=hi {
                let yy = y as i64 + o;
                if yy >= 0 && (yy as usize) < h {
                    v = v.min(tmp[yy as usize * w + x]);
                }
            }
            out[y * w + x] = v;
        }
    }
    out
}

/// Separable Gaussian blur, kernel size 2*radius+1, with OpenCV's sigma rule
/// for sigma=0: 0.3 * ((ksize - 1) / 2 - 1) + 0.8. Edge pixels replicate.
fn gaussian_blur(src: &[f32], w: usize, h: usize, radius: usize) -> Vec<f32> {
    let r = radius as i64;
    let sigma = (0.3 * (radius as f32 - 1.0) + 0.8).max(0.1);
    let mut kern: Vec<f32> = (-r..=r)
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = kern.iter().sum();
    kern.iter_mut().for_each(|v| *v /= sum);

    let mut tmp = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (j, kv) in kern.iter().enumerate() {
                let xx = (x as i64 + j as i64 - r).clamp(0, w as i64 - 1) as usize;
                acc += kv * src[y * w + xx];
            }
            tmp[y * w + x] = acc;
        }
    }
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (j, kv) in kern.iter().enumerate() {
                let yy = (y as i64 + j as i64 - r).clamp(0, h as i64 - 1) as usize;
                acc += kv * tmp[yy * w + x];
            }
            out[y * w + x] = acc;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similarity_recovers_template() {
        // Landmarks = template under a known rotation/scale/translation.
        let (th, sc) = (0.3f64, 0.4f64);
        let (c, s) = (th.cos() * sc, th.sin() * sc);
        let mut lm = [[0.0; 2]; 5];
        for i in 0..5 {
            let [x, y] = FFHQ_512[i];
            lm[i] = [c * x - s * y + 300.0, s * x + c * y + 200.0];
        }
        let m = similarity(&lm, &FFHQ_512).unwrap();
        for i in 0..5 {
            let (x, y) = apply(&m, lm[i][0], lm[i][1]);
            assert!((x - FFHQ_512[i][0]).abs() < 1e-6 && (y - FFHQ_512[i][1]).abs() < 1e-6);
        }
        let inv = invert(&m);
        let (x, y) = apply(&inv, FFHQ_512[2][0], FFHQ_512[2][1]);
        assert!((x - lm[2][0]).abs() < 1e-6 && (y - lm[2][1]).abs() < 1e-6);
    }
}
