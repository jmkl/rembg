//! CPU reference implementations of every image operation. They are the
//! oracles the GPU shaders are tested against, so they favor clarity over
//! speed. All work in linear light on premultiplied pixels; operations that
//! need straight color un-premultiply first and premultiply on the way out.
use crate::buffer::ImageBuf;
use compnode_core::color::{hsv_to_rgb, rgb_to_hsv};

fn map(img: &ImageBuf, f: impl Fn([f32; 4]) -> [f32; 4]) -> ImageBuf {
    ImageBuf {
        width: img.width,
        height: img.height,
        data: img.data.iter().map(|p| f(*p)).collect(),
    }
}

/// Apply `f` to the straight (un-premultiplied) color of every pixel.
fn map_straight(img: &ImageBuf, f: impl Fn([f32; 3]) -> [f32; 3]) -> ImageBuf {
    map(img, |p| {
        let a = p[3];
        if a <= 0.0 {
            return p;
        }
        let inv = 1.0 / a;
        let c = f([p[0] * inv, p[1] * inv, p[2] * inv]);
        [c[0] * a, c[1] * a, c[2] * a, a]
    })
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Invert colors; `amount` mixes between the original (0) and inverted (1).
pub fn invert(img: &ImageBuf, amount: f32) -> ImageBuf {
    map_straight(img, |c| {
        [
            c[0] + (1.0 - 2.0 * c[0]) * amount,
            c[1] + (1.0 - 2.0 * c[1]) * amount,
            c[2] + (1.0 - 2.0 * c[2]) * amount,
        ]
    })
}

/// `brightness` is added; `contrast` in `-1..1` scales around mid gray
/// (0.5 in linear light, a simplification).
pub fn brightness_contrast(img: &ImageBuf, brightness: f32, contrast: f32) -> ImageBuf {
    let factor = if contrast >= 0.0 {
        1.0 / (1.0 - contrast.min(0.99))
    } else {
        1.0 + contrast
    };
    map_straight(img, |c| {
        let f = |v: f32| ((v - 0.5) * factor + 0.5 + brightness).max(0.0);
        [f(c[0]), f(c[1]), f(c[2])]
    })
}

fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let r = (3.0 * sigma).ceil().max(1.0) as i64;
    let mut k: Vec<f32> = (-r..=r)
        .map(|i| (-((i * i) as f32) / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = k.iter().sum();
    for w in &mut k {
        *w /= sum;
    }
    k
}

fn blur_pass(src: &ImageBuf, horizontal: bool, k: &[f32]) -> ImageBuf {
    let r = (k.len() / 2) as i64;
    let mut out = ImageBuf::new(src.width, src.height);
    for y in 0..src.height {
        for x in 0..src.width {
            let mut acc = [0.0_f32; 4];
            for (j, w) in k.iter().enumerate() {
                let off = j as i64 - r;
                let p = if horizontal {
                    src.px_clamped(i64::from(x) + off, i64::from(y))
                } else {
                    src.px_clamped(i64::from(x), i64::from(y) + off)
                };
                for c in 0..4 {
                    acc[c] += w * p[c];
                }
            }
            out.set(x, y, acc);
        }
    }
    out
}

/// Gaussian blur with standard deviation `radius` pixels (edges extended).
pub fn blur(img: &ImageBuf, radius: f32) -> ImageBuf {
    if radius <= 0.0 || !radius.is_finite() {
        return img.clone();
    }
    let k = gaussian_kernel(radius);
    blur_pass(&blur_pass(img, true, &k), false, &k)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Add,
    Difference,
    Darken,
    Lighten,
}

impl BlendMode {
    pub const ALL: [BlendMode; 8] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Add,
        BlendMode::Difference,
        BlendMode::Darken,
        BlendMode::Lighten,
    ];
    pub fn name(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Multiply => "Multiply",
            BlendMode::Screen => "Screen",
            BlendMode::Overlay => "Overlay",
            BlendMode::Add => "Add",
            BlendMode::Difference => "Difference",
            BlendMode::Darken => "Darken",
            BlendMode::Lighten => "Lighten",
        }
    }
    pub fn from_index(i: usize) -> BlendMode {
        BlendMode::ALL.get(i).copied().unwrap_or(BlendMode::Normal)
    }
    /// Blend function on straight channel values: backdrop `b`, source `s`.
    fn f(self, b: f32, s: f32) -> f32 {
        match self {
            BlendMode::Normal => s,
            BlendMode::Multiply => b * s,
            BlendMode::Screen => b + s - b * s,
            BlendMode::Overlay => {
                if b <= 0.5 {
                    2.0 * b * s
                } else {
                    1.0 - 2.0 * (1.0 - b) * (1.0 - s)
                }
            }
            BlendMode::Add => b + s,
            BlendMode::Difference => (b - s).abs(),
            BlendMode::Darken => b.min(s),
            BlendMode::Lighten => b.max(s),
        }
    }
}

/// Composite `layer` over `base` (W3C compositing model, source-over with a
/// separable blend function). The result has the size of `base`; `layer` and
/// `mask` are read at the same pixel coordinates and are transparent where
/// they do not reach. The mask's luminance scales the layer's opacity.
pub fn blend(
    base: &ImageBuf,
    layer: &ImageBuf,
    mode: BlendMode,
    opacity: f32,
    mask: Option<&ImageBuf>,
) -> ImageBuf {
    let mut out = ImageBuf::new(base.width, base.height);
    for y in 0..base.height {
        for x in 0..base.width {
            let (xi, yi) = (i64::from(x), i64::from(y));
            let cb = base.get(x, y);
            let cs = layer.px_or_transparent(xi, yi);
            let m = mask.map_or(1.0, |m| {
                let p = m.px_or_transparent(xi, yi);
                luma([p[0], p[1], p[2]])
            });
            let k = opacity * m;
            let a_s = cs[3] * k;
            let a_b = cb[3];
            let straight = |p: [f32; 4]| {
                if p[3] > 0.0 {
                    [p[0] / p[3], p[1] / p[3], p[2] / p[3]]
                } else {
                    [0.0; 3]
                }
            };
            let (sb, ss) = (straight(cb), straight(cs));
            let mut o = [0.0; 4];
            for c in 0..3 {
                let blended = mode.f(sb[c], ss[c]);
                o[c] = cs[c] * k * (1.0 - a_b) + cb[c] * (1.0 - a_s) + a_s * a_b * blended;
            }
            o[3] = a_s + a_b * (1.0 - a_s);
            out.set(x, y, o);
        }
    }
    out
}

/// One layer to composite: its source image and how it sits on the canvas.
pub struct LayerPaint<'a> {
    pub image: &'a ImageBuf,
    /// Center on the canvas in pixels; `None` centers the fitted image.
    pub center: Option<[f32; 2]>,
    /// Size on the canvas in pixels; `None`/zero fits the image to the canvas.
    pub size: Option<[f32; 2]>,
    /// Rotation in degrees.
    pub angle: f32,
}

/// Composite `layers` bottom-to-top (source-over) onto a `canvas_w x canvas_h`
/// transparent image. Each layer is bilinearly sampled with its transform;
/// layers with no explicit transform are aspect-fitted and centered. This is
/// what the Output node uses to build its canvas from its input layers.
pub fn composite_layers(layers: &[LayerPaint], canvas_w: u32, canvas_h: u32) -> ImageBuf {
    let (cw, ch) = (canvas_w.max(1), canvas_h.max(1));
    let mut out = ImageBuf::new(cw, ch);
    for layer in layers {
        let src = layer.image;
        if src.width == 0 || src.height == 0 {
            continue;
        }
        let (sw, sh) = (src.width as f32, src.height as f32);
        // Fit into the canvas when no explicit size is given.
        let (dw, dh) = match layer.size {
            Some([w, h]) if w > 0.0 && h > 0.0 => (w, h),
            _ => {
                let factor = (cw as f32 / sw).min(ch as f32 / sh);
                ((sw * factor).max(1.0), (sh * factor).max(1.0))
            }
        };
        let [cx, cy] = layer.center.unwrap_or([cw as f32 * 0.5, ch as f32 * 0.5]);
        let (cos, sin) = layer.angle.to_radians().sin_cos();
        let inv_sw = sw / dw;
        let inv_sh = sh / dh;
        for y in 0..ch {
            for x in 0..cw {
                let px = x as f32 + 0.5 - cx;
                let py = y as f32 + 0.5 - cy;
                // Inverse rotation maps the destination pixel into layer space.
                let lx = px * cos + py * sin;
                let ly = -px * sin + py * cos;
                let sx = (lx + dw * 0.5) * inv_sw;
                let sy = (ly + dh * 0.5) * inv_sh;
                if sx < 0.0 || sy < 0.0 || sx >= sw || sy >= sh {
                    continue;
                }
                let cs = src.sample(sx, sy, false);
                if cs[3] <= 0.0 {
                    continue;
                }
                let cb = out.get(x, y);
                let a_s = cs[3];
                let a_b = cb[3];
                let a_o = a_s + a_b * (1.0 - a_s);
                if a_o <= 0.0 {
                    continue;
                }
                // Straight-alpha source-over.
                let s = if a_s > 0.0 {
                    [cs[0] / a_s, cs[1] / a_s, cs[2] / a_s]
                } else {
                    [0.0; 3]
                };
                let b = if a_b > 0.0 {
                    [cb[0] / a_b, cb[1] / a_b, cb[2] / a_b]
                } else {
                    [0.0; 3]
                };
                let inv = 1.0 / a_o;
                let o = [
                    (s[0] * a_s + b[0] * a_b * (1.0 - a_s)) * inv,
                    (s[1] * a_s + b[1] * a_b * (1.0 - a_s)) * inv,
                    (s[2] * a_s + b[2] * a_b * (1.0 - a_s)) * inv,
                    a_o,
                ];
                out.set(x, y, o);
            }
        }
    }
    out
}

/// Levels: map `[in_black, in_white]` to `[0, 1]`, apply `gamma`, then map
/// to `[out_black, out_white]`.
pub fn levels(
    img: &ImageBuf,
    in_black: f32,
    in_white: f32,
    gamma: f32,
    out_black: f32,
    out_white: f32,
) -> ImageBuf {
    let range = (in_white - in_black).max(1e-6);
    let inv_gamma = 1.0 / gamma.max(1e-3);
    map_straight(img, |c| {
        let f = |v: f32| {
            out_black
                + (((v - in_black) / range).clamp(0.0, 1.0)).powf(inv_gamma)
                    * (out_white - out_black)
        };
        [f(c[0]), f(c[1]), f(c[2])]
    })
}

/// Hue shift (degrees), saturation and value multipliers. Works on the
/// linear channel values directly (a simplification of perceptual HSV).
pub fn hue_saturation(img: &ImageBuf, hue_deg: f32, saturation: f32, value: f32) -> ImageBuf {
    map_straight(img, |c| {
        let [h, s, v] = rgb_to_hsv(c[0], c[1], c[2]);
        hsv_to_rgb(
            h + hue_deg / 360.0,
            (s * saturation).clamp(0.0, 1.0),
            v * value,
        )
    })
}

/// Unsharp mask: `img + amount * (img - blur(img, radius))`.
pub fn sharpen(img: &ImageBuf, amount: f32, radius: f32) -> ImageBuf {
    let b = blur(img, radius);
    let mut out = ImageBuf::new(img.width, img.height);
    for (i, (p, q)) in img.data.iter().zip(&b.data).enumerate() {
        let a = (p[3] + amount * (p[3] - q[3])).clamp(0.0, 1.0);
        let mut o = [0.0; 4];
        for c in 0..3 {
            o[c] = (p[c] + amount * (p[c] - q[c])).clamp(0.0, a);
        }
        o[3] = a;
        out.data[i] = o;
    }
    out
}

/// Black/white by luminance against `t`; alpha is kept.
pub fn threshold(img: &ImageBuf, t: f32) -> ImageBuf {
    map_straight(img, |c| {
        let v = if luma(c) >= t { 1.0 } else { 0.0 };
        [v, v, v]
    })
}

fn downscale_area(src: &ImageBuf, w: u32, h: u32) -> ImageBuf {
    let sx = src.width as f32 / w as f32;
    let sy = src.height as f32 / h as f32;
    let mut out = ImageBuf::new(w, h);
    for dy in 0..h {
        let (y0, y1) = (dy as f32 * sy, (dy + 1) as f32 * sy);
        for dx in 0..w {
            let (x0, x1) = (dx as f32 * sx, (dx + 1) as f32 * sx);
            let mut acc = [0.0_f32; 4];
            let mut wsum = 0.0_f32;
            let (iy0, iy1) = (y0.floor() as u32, (y1.ceil() as u32).min(src.height));
            let (ix0, ix1) = (x0.floor() as u32, (x1.ceil() as u32).min(src.width));
            for iy in iy0..iy1 {
                let wy = (y1.min((iy + 1) as f32) - y0.max(iy as f32)).max(0.0);
                for ix in ix0..ix1 {
                    let wx = (x1.min((ix + 1) as f32) - x0.max(ix as f32)).max(0.0);
                    let wgt = wx * wy;
                    let p = src.get(ix, iy);
                    for c in 0..4 {
                        acc[c] += p[c] * wgt;
                    }
                    wsum += wgt;
                }
            }
            if wsum > 0.0 {
                for v in &mut acc {
                    *v /= wsum;
                }
            }
            out.set(dx, dy, acc);
        }
    }
    out
}

/// Resize to `w x h` (at least 1x1): bilinear when enlarging in both
/// directions, area averaging otherwise (no aliasing when shrinking).
pub fn resize(img: &ImageBuf, w: u32, h: u32) -> ImageBuf {
    let (w, h) = (w.max(1), h.max(1));
    if w == img.width && h == img.height {
        return img.clone();
    }
    if w >= img.width && h >= img.height {
        let sx = img.width as f32 / w as f32;
        let sy = img.height as f32 / h as f32;
        return ImageBuf::from_fn(w, h, |x, y| {
            img.sample((x as f32 + 0.5) * sx, (y as f32 + 0.5) * sy, true)
        });
    }
    downscale_area(img, w, h)
}

/// Crop to the rectangle at `(x, y)` of size `w x h`; areas outside the
/// source are transparent.
pub fn crop(img: &ImageBuf, x: i64, y: i64, w: u32, h: u32) -> ImageBuf {
    let (w, h) = (w.max(1), h.max(1));
    ImageBuf::from_fn(w, h, |dx, dy| {
        img.px_or_transparent(x + i64::from(dx), y + i64::from(dy))
    })
}

/// Rotate clockwise by `degrees` about the center, keeping the size.
/// Uncovered areas are transparent.
pub fn rotate(img: &ImageBuf, degrees: f32) -> ImageBuf {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (cx, cy) = (img.width as f32 * 0.5, img.height as f32 * 0.5);
    ImageBuf::from_fn(img.width, img.height, |x, y| {
        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
        // inverse rotation maps the destination pixel back to the source
        let (sx, sy) = (cx + cos * dx + sin * dy, cy - sin * dx + cos * dy);
        img.sample(sx, sy, false)
    })
}

/// Samples of a monotone cubic curve (Fritsch-Carlson) through `points`
/// `(x, y)`, which must lie in `0..=1`. Outside the first and last point the
/// curve is flat. Monotone data never overshoots, so a "lift the shadows"
/// curve cannot produce negative colors.
pub fn curve_lut(points: &[(f32, f32)], samples: usize) -> Vec<f32> {
    let mut pts: Vec<(f32, f32)> = points.to_vec();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    pts.dedup_by(|b, a| a.0 == b.0);
    let n = samples.max(2);
    if pts.is_empty() {
        return (0..n).map(|i| i as f32 / (n - 1) as f32).collect();
    }
    if pts.len() == 1 {
        return vec![pts[0].1.clamp(0.0, 1.0); n];
    }
    let k = pts.len();
    let d: Vec<f32> = (0..k - 1)
        .map(|i| (pts[i + 1].1 - pts[i].1) / (pts[i + 1].0 - pts[i].0))
        .collect();
    let mut m = vec![0.0_f32; k];
    m[0] = d[0];
    m[k - 1] = d[k - 2];
    for i in 1..k - 1 {
        m[i] = if d[i - 1] * d[i] <= 0.0 {
            0.0
        } else {
            (d[i - 1] + d[i]) * 0.5
        };
    }
    for i in 0..k - 1 {
        if d[i] == 0.0 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
            continue;
        }
        let (a, b) = (m[i] / d[i], m[i + 1] / d[i]);
        let s = a * a + b * b;
        if s > 9.0 {
            let t = 3.0 / s.sqrt();
            m[i] = t * a * d[i];
            m[i + 1] = t * b * d[i];
        }
    }
    (0..n)
        .map(|j| {
            let x = j as f32 / (n - 1) as f32;
            if x <= pts[0].0 {
                return pts[0].1.clamp(0.0, 1.0);
            }
            if x >= pts[k - 1].0 {
                return pts[k - 1].1.clamp(0.0, 1.0);
            }
            let i = pts.iter().rposition(|p| p.0 <= x).unwrap_or(0).min(k - 2);
            let (x0, y0, x1, y1) = (pts[i].0, pts[i].1, pts[i + 1].0, pts[i + 1].1);
            let h = x1 - x0;
            let t = (x - x0) / h;
            let (t2, t3) = (t * t, t * t * t);
            let y = (2.0 * t3 - 3.0 * t2 + 1.0) * y0
                + (t3 - 2.0 * t2 + t) * h * m[i]
                + (-2.0 * t3 + 3.0 * t2) * y1
                + (t3 - t2) * h * m[i + 1];
            y.clamp(0.0, 1.0)
        })
        .collect()
}

/// The sorted, de-duplicated control points of a curve and the tangent at
/// each of them (Fritsch-Carlson), for evaluators that interpolate the
/// Hermite segments themselves (the GPU shader does).
pub fn curve_tangents(points: &[(f32, f32)]) -> (Vec<(f32, f32)>, Vec<f32>) {
    let mut pts: Vec<(f32, f32)> = points.to_vec();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    pts.dedup_by(|b, a| a.0 == b.0);
    let k = pts.len();
    if k < 2 {
        return (pts, vec![0.0; k]);
    }
    let d: Vec<f32> = (0..k - 1)
        .map(|i| (pts[i + 1].1 - pts[i].1) / (pts[i + 1].0 - pts[i].0))
        .collect();
    let mut m = vec![0.0_f32; k];
    m[0] = d[0];
    m[k - 1] = d[k - 2];
    for i in 1..k - 1 {
        m[i] = if d[i - 1] * d[i] <= 0.0 {
            0.0
        } else {
            (d[i - 1] + d[i]) * 0.5
        };
    }
    for i in 0..k - 1 {
        if d[i] == 0.0 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
            continue;
        }
        let (a, b) = (m[i] / d[i], m[i + 1] / d[i]);
        let s = a * a + b * b;
        if s > 9.0 {
            let t = 3.0 / s.sqrt();
            m[i] = t * a * d[i];
            m[i + 1] = t * b * d[i];
        }
    }
    (pts, m)
}

/// Apply one curve to every color channel (straight color, clamped to `0..=1`).
pub fn curves(img: &ImageBuf, points: &[(f32, f32)]) -> ImageBuf {
    const N: usize = 1024;
    let lut = curve_lut(points, N + 1);
    let look = |v: f32| {
        let x = v.clamp(0.0, 1.0) * N as f32;
        let i = (x.floor() as usize).min(N - 1);
        lut[i] + (lut[i + 1] - lut[i]) * (x - i as f32)
    };
    map_straight(img, |c| [look(c[0]), look(c[1]), look(c[2])])
}

#[cfg(test)]
mod tests {
    use super::*;
    use compnode_core::Color;

    const TOL: f32 = 1e-4;

    fn solid(r: f32, g: f32, b: f32) -> ImageBuf {
        ImageBuf::solid(4, 4, Color::rgb(r, g, b))
    }
    fn close(a: &ImageBuf, b: &ImageBuf, tol: f32) {
        let d = a.max_abs_diff(b).expect("same size");
        assert!(d <= tol, "max diff {d} > {tol}");
    }

    #[test]
    fn invert_golden() {
        close(
            &invert(&solid(0.2, 0.4, 0.6), 1.0),
            &solid(0.8, 0.6, 0.4),
            TOL,
        );
        close(
            &invert(&solid(0.2, 0.4, 0.6), 0.0),
            &solid(0.2, 0.4, 0.6),
            TOL,
        );
        close(
            &invert(&solid(0.2, 0.4, 0.6), 0.5),
            &solid(0.5, 0.5, 0.5),
            TOL,
        );
        // alpha is preserved and color stays premultiplied
        let half = ImageBuf::solid(2, 2, Color::new(1.0, 0.0, 0.0, 0.5));
        let inv = invert(&half, 1.0);
        let p = inv.get(0, 0);
        assert!((p[3] - 0.5).abs() < TOL && p[0].abs() < TOL && (p[1] - 0.5).abs() < TOL);
        // transparent pixels stay transparent
        assert_eq!(invert(&ImageBuf::new(2, 2), 1.0), ImageBuf::new(2, 2));
    }

    #[test]
    fn brightness_contrast_golden() {
        close(
            &brightness_contrast(&solid(0.2, 0.2, 0.2), 0.1, 0.0),
            &solid(0.3, 0.3, 0.3),
            TOL,
        );
        // contrast 0.5 doubles the distance from mid gray
        close(
            &brightness_contrast(&solid(0.25, 0.75, 0.5), 0.0, 0.5),
            &solid(0.0, 1.0, 0.5),
            TOL,
        );
        close(
            &brightness_contrast(&solid(0.0, 1.0, 0.5), 0.0, -0.5),
            &solid(0.25, 0.75, 0.5),
            TOL,
        );
        close(
            &brightness_contrast(&solid(0.5, 0.5, 0.5), 0.0, 0.3),
            &solid(0.5, 0.5, 0.5),
            TOL,
        );
    }

    #[test]
    fn blur_preserves_flat_images_and_energy_and_symmetry() {
        let flat = solid(0.3, 0.6, 0.9);
        close(&blur(&flat, 2.0), &flat, TOL);
        close(&blur(&flat, 0.0), &flat, 0.0);
        let mut imp = ImageBuf::new(21, 21);
        imp.set(10, 10, [1.0, 1.0, 1.0, 1.0]);
        let b = blur(&imp, 1.5);
        let total: f32 = b.data.iter().map(|p| p[0]).sum();
        assert!((total - 1.0).abs() < 1e-3, "{total}");
        // symmetric around the impulse, peak at the center
        for d in 1..6 {
            assert!((b.get(10 - d, 10)[0] - b.get(10 + d, 10)[0]).abs() < 1e-6);
            assert!((b.get(10, 10 - d)[0] - b.get(10, 10 + d)[0]).abs() < 1e-6);
        }
        assert!(b.get(10, 10)[0] > b.get(11, 10)[0]);
        // known value: 1D gaussian sigma 1.5, center weight squared
        let k = gaussian_kernel(1.5);
        let center = k[k.len() / 2];
        assert!((b.get(10, 10)[0] - center * center).abs() < 1e-5);
    }

    #[test]
    fn blend_golden() {
        let (a, b) = (solid(0.5, 0.5, 0.5), solid(0.5, 0.25, 1.0));
        close(
            &blend(&a, &b, BlendMode::Multiply, 1.0, None),
            &solid(0.25, 0.125, 0.5),
            TOL,
        );
        close(
            &blend(&a, &b, BlendMode::Screen, 1.0, None),
            &solid(0.75, 0.625, 1.0),
            TOL,
        );
        close(&blend(&a, &b, BlendMode::Normal, 1.0, None), &b, TOL);
        close(&blend(&a, &b, BlendMode::Normal, 0.0, None), &a, TOL);
        close(
            &blend(&a, &b, BlendMode::Normal, 0.5, None),
            &solid(0.5, 0.375, 0.75),
            TOL,
        );
        close(
            &blend(&a, &b, BlendMode::Add, 1.0, None),
            &solid(1.0, 0.75, 1.5),
            TOL,
        );
        close(
            &blend(&a, &b, BlendMode::Difference, 1.0, None),
            &solid(0.0, 0.25, 0.5),
            TOL,
        );
        close(
            &blend(&a, &b, BlendMode::Darken, 1.0, None),
            &solid(0.5, 0.25, 0.5),
            TOL,
        );
        close(
            &blend(&a, &b, BlendMode::Lighten, 1.0, None),
            &solid(0.5, 0.5, 1.0),
            TOL,
        );
        // overlay: backdrop 0.5 uses the multiply branch (2*b*s)
        close(
            &blend(&a, &b, BlendMode::Overlay, 1.0, None),
            &solid(0.5, 0.25, 1.0),
            TOL,
        );
        let bright = solid(0.8, 0.8, 0.8);
        let o = blend(
            &bright,
            &solid(0.5, 0.5, 0.5),
            BlendMode::Overlay,
            1.0,
            None,
        );
        assert!((o.get(0, 0)[0] - (1.0 - 2.0 * 0.2 * 0.5)).abs() < TOL);
    }

    #[test]
    fn blend_alpha_and_mask() {
        let base = ImageBuf::solid(2, 1, Color::new(0.0, 0.0, 1.0, 1.0));
        let half_red = ImageBuf::solid(2, 1, Color::new(1.0, 0.0, 0.0, 0.5));
        let o = blend(&base, &half_red, BlendMode::Normal, 1.0, None).get(0, 0);
        assert!((o[0] - 0.5).abs() < TOL && (o[2] - 0.5).abs() < TOL && (o[3] - 1.0).abs() < TOL);
        // transparent base: result is the layer itself
        let o = blend(
            &ImageBuf::new(2, 1),
            &half_red,
            BlendMode::Multiply,
            1.0,
            None,
        )
        .get(0, 0);
        assert!((o[0] - 0.5).abs() < TOL && (o[3] - 0.5).abs() < TOL);
        // mask: left pixel masked out (black), right pixel fully on (white)
        let mask = ImageBuf::from_fn(2, 1, |x, _| {
            if x == 0 {
                [0.0, 0.0, 0.0, 1.0]
            } else {
                [1.0, 1.0, 1.0, 1.0]
            }
        });
        let red = ImageBuf::solid(2, 1, Color::rgb(1.0, 0.0, 0.0));
        let o = blend(&base, &red, BlendMode::Normal, 1.0, Some(&mask));
        assert!((o.get(0, 0)[2] - 1.0).abs() < TOL && o.get(0, 0)[0].abs() < TOL);
        assert!((o.get(1, 0)[0] - 1.0).abs() < TOL && o.get(1, 0)[2].abs() < TOL);
        // a smaller layer only covers its own area
        let tiny = ImageBuf::solid(1, 1, Color::rgb(1.0, 0.0, 0.0));
        let o = blend(&base, &tiny, BlendMode::Normal, 1.0, None);
        assert!((o.get(0, 0)[0] - 1.0).abs() < TOL && o.get(1, 0)[0].abs() < TOL);
    }

    #[test]
    fn levels_golden() {
        close(
            &levels(&solid(0.5, 0.5, 0.5), 0.0, 1.0, 1.0, 0.0, 1.0),
            &solid(0.5, 0.5, 0.5),
            TOL,
        );
        close(
            &levels(&solid(0.25, 0.5, 0.75), 0.25, 0.75, 1.0, 0.0, 1.0),
            &solid(0.0, 0.5, 1.0),
            TOL,
        );
        close(
            &levels(&solid(0.25, 0.25, 0.25), 0.0, 1.0, 2.0, 0.0, 1.0),
            &solid(0.5, 0.5, 0.5),
            TOL,
        );
        close(
            &levels(&solid(0.0, 1.0, 0.5), 0.0, 1.0, 1.0, 0.2, 0.6),
            &solid(0.2, 0.6, 0.4),
            TOL,
        );
    }

    #[test]
    fn hue_saturation_golden() {
        close(
            &hue_saturation(&solid(1.0, 0.0, 0.0), 120.0, 1.0, 1.0),
            &solid(0.0, 1.0, 0.0),
            TOL,
        );
        close(
            &hue_saturation(&solid(1.0, 0.0, 0.0), 360.0, 1.0, 1.0),
            &solid(1.0, 0.0, 0.0),
            TOL,
        );
        close(
            &hue_saturation(&solid(1.0, 0.0, 0.0), 0.0, 0.0, 1.0),
            &solid(1.0, 1.0, 1.0),
            TOL,
        );
        close(
            &hue_saturation(&solid(0.4, 0.2, 0.2), 0.0, 1.0, 0.5),
            &solid(0.2, 0.1, 0.1),
            TOL,
        );
    }

    #[test]
    fn sharpen_and_threshold() {
        close(
            &sharpen(&solid(0.4, 0.4, 0.4), 2.0, 1.0),
            &solid(0.4, 0.4, 0.4),
            TOL,
        );
        let step = ImageBuf::from_fn(16, 1, |x, _| {
            if x < 8 {
                [0.2, 0.2, 0.2, 1.0]
            } else {
                [0.6, 0.6, 0.6, 1.0]
            }
        });
        let s = sharpen(&step, 1.0, 1.0);
        assert!(s.get(7, 0)[0] < 0.2 && s.get(8, 0)[0] > 0.6); // overshoot at the edge
        assert!((s.get(0, 0)[0] - 0.2).abs() < 1e-3);
        let t = threshold(&step, 0.4);
        assert_eq!(t.get(0, 0), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(t.get(15, 0), [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn resize_crop_rotate_golden() {
        // constant images stay constant under any resize
        close(
            &resize(&solid(0.3, 0.6, 0.9), 2, 2),
            &ImageBuf::solid(2, 2, Color::rgb(0.3, 0.6, 0.9)),
            TOL,
        );
        close(
            &resize(&solid(0.3, 0.6, 0.9), 9, 7),
            &ImageBuf::solid(9, 7, Color::rgb(0.3, 0.6, 0.9)),
            TOL,
        );
        // area average of a checker is the mean
        let checker = ImageBuf::from_fn(4, 4, |x, y| {
            if (x + y) % 2 == 0 {
                [1.0; 4]
            } else {
                [0.0, 0.0, 0.0, 1.0]
            }
        });
        let small = resize(&checker, 1, 1);
        assert!((small.get(0, 0)[0] - 0.5).abs() < TOL);
        assert_eq!(resize(&checker, 4, 4), checker);

        let img = ImageBuf::from_fn(4, 4, |x, y| [x as f32 / 10.0, y as f32 / 10.0, 0.0, 1.0]);
        let c = crop(&img, 1, 2, 2, 2);
        assert_eq!(c.get(0, 0), [0.1, 0.2, 0.0, 1.0]);
        assert_eq!(c.get(1, 1), [0.2, 0.3, 0.0, 1.0]);
        let out = crop(&img, -1, 0, 2, 1);
        assert_eq!(out.get(0, 0)[3], 0.0);

        let mut dot = ImageBuf::new(4, 4);
        dot.set(0, 0, [1.0, 1.0, 1.0, 1.0]);
        let r = rotate(&dot, 90.0);
        let mut want = ImageBuf::new(4, 4);
        want.set(3, 0, [1.0, 1.0, 1.0, 1.0]); // top-left goes to top-right (clockwise)
        close(&r, &want, 1e-3);
        close(&rotate(&dot, 0.0), &dot, 1e-4);
        close(&rotate(&rotate(&dot, 90.0), -90.0), &dot, 1e-3);
    }

    #[test]
    fn blend_mode_names_and_indices() {
        for (i, m) in BlendMode::ALL.iter().enumerate() {
            assert_eq!(BlendMode::from_index(i), *m);
            assert!(!m.name().is_empty());
        }
        assert_eq!(BlendMode::from_index(99), BlendMode::Normal);
    }

    #[test]
    fn curve_passes_through_its_points_and_is_monotone() {
        let pts = [
            (0.0, 0.0),
            (0.25, 0.4),
            (0.5, 0.55),
            (0.75, 0.8),
            (1.0, 1.0),
        ];
        let n = 1025;
        let lut = curve_lut(&pts, n);
        for (x, y) in pts {
            let v = lut[(x * (n - 1) as f32).round() as usize];
            assert!((v - y).abs() < 1e-4, "at {x}: {v} vs {y}");
        }
        assert!(
            lut.windows(2).all(|w| w[1] >= w[0] - 1e-6),
            "monotone data must give a monotone curve"
        );
        // a steep step cannot overshoot
        let steep = curve_lut(&[(0.0, 0.0), (0.5, 0.1), (0.55, 0.9), (1.0, 1.0)], 1025);
        assert!(steep.iter().all(|v| (0.0..=1.0).contains(v)));
        assert!(steep.windows(2).all(|w| w[1] >= w[0] - 1e-6));
    }

    #[test]
    fn curve_lut_edge_cases() {
        let id = curve_lut(&[], 5);
        assert_eq!(id, vec![0.0, 0.25, 0.5, 0.75, 1.0]);
        assert_eq!(curve_lut(&[(0.3, 0.7)], 3), vec![0.7, 0.7, 0.7]);
        // unsorted and duplicate x are tolerated; flat outside the end points
        let l = curve_lut(&[(0.8, 0.8), (0.2, 0.2), (0.2, 0.9)], 11);
        assert!((l[0] - l[1]).abs() < 1e-6 && (l[9] - l[10]).abs() < 1e-6);
    }

    #[test]
    fn curves_golden() {
        let ident = [
            (0.0, 0.0),
            (0.25, 0.25),
            (0.5, 0.5),
            (0.75, 0.75),
            (1.0, 1.0),
        ];
        close(
            &curves(&solid(0.1, 0.5, 0.9), &ident),
            &solid(0.1, 0.5, 0.9),
            2e-3,
        );
        let lift = [(0.0, 0.0), (0.5, 0.75), (1.0, 1.0)];
        let out = curves(&solid(0.5, 0.0, 1.0), &lift);
        let p = out.get(0, 0);
        assert!((p[0] - 0.75).abs() < 2e-3 && p[1].abs() < 1e-4 && (p[2] - 1.0).abs() < 1e-4);
        // inverted curve
        close(
            &curves(&solid(0.2, 0.6, 1.0), &[(0.0, 1.0), (1.0, 0.0)]),
            &solid(0.8, 0.4, 0.0),
            2e-3,
        );
        // alpha is preserved
        let half = ImageBuf::solid(2, 2, Color::new(0.5, 0.5, 0.5, 0.5));
        assert!((curves(&half, &lift).get(0, 0)[3] - 0.5).abs() < 1e-6);
    }
}
