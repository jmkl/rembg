//! Depth-mask "lens blur" (Photoshop-style): the blur radius of every pixel is
//! driven by a grayscale depth mask, and the bokeh has the shape of the iris.
//!
//! Method: scatter-as-gather disc sampling.
//! * Each pixel gathers `N` samples from a Vogel (golden-angle) spiral that is
//!   scaled by the pixel's own blur radius. `N` grows with the disc area, up
//!   to `samples`, so small radii are cheap.
//! * Depth-aware weights prevent halos: a sample that is *nearer* than the
//!   pixel only contributes if its own blur disc actually reaches the pixel.
//!   A sharp foreground therefore never bleeds into a blurred background,
//!   while a blurry foreground still spreads over what is behind it.
//! * The iris can be a polygon (`blades >= 3`, with rotation). The sample
//!   pattern is jittered per pixel (16 pre-built phases) so a limited sample
//!   count turns into fine grain instead of visible ghost copies.
//! * Optional highlight boost makes bright pixels dominate, giving bokeh balls.
//! * Parallel over rows, const-generic over channel count, 100% safe Rust.
//!
//! Known limitation: a blurry foreground in front of a *sharp* background gets
//! soft edges only on its inner side (the pixels just outside it are sharp, so
//! they do not gather the foreground).

use rayon::prelude::*;
use std::f32::consts::{PI, TAU};

const PHASES: usize = 16;
const BIAS: f32 = 4096.0; // makes float -> int truncation act like round()
const BIAS_I: i32 = 4096;
const MAX_RADIUS: f32 = 2000.0;
const GOLDEN_ANGLE: f32 = 2.399_963_2;

#[derive(Clone, Debug)]
pub struct LensParams {
    /// Mask gray level (0..=255) that stays perfectly sharp.
    pub focus: f32,
    /// Half-width (in gray levels) around `focus` that is also fully sharp.
    pub focus_range: f32,
    /// Blur radius in pixels for the gray level farthest from `focus`.
    pub max_radius: f32,
    /// Maximum samples per pixel (quality vs speed). 48..128 is typical.
    pub samples: usize,
    /// Iris blade count; < 3 means a circular iris.
    pub blades: u32,
    /// Iris rotation in degrees.
    pub rotation_deg: f32,
    /// true: white (255) is nearest. false: black (0) is nearest.
    /// Only used to decide which pixel occludes which; the focus value is
    /// always a raw mask gray level.
    pub white_is_near: bool,
    /// 0 = off. Typical 2..10. Bright pixels get this much extra weight.
    pub highlight_gain: f32,
    /// Luma (0..=255) above which the highlight boost starts.
    pub highlight_threshold: f32,
    /// Depth difference (gray levels) over which occlusion fades in.
    pub edge_softness: f32,
}

impl Default for LensParams {
    fn default() -> Self {
        Self {
            focus: 255.0,
            focus_range: 0.0,
            max_radius: 16.0,
            samples: 64,
            blades: 0,
            rotation_deg: 0.0,
            white_is_near: true,
            highlight_gain: 0.0,
            highlight_threshold: 200.0,
            edge_softness: 8.0,
        }
    }
}

/// Position factor that turns the unit disc into a regular polygon whose
/// vertices touch the unit circle.
fn iris(angle: f32, blades: u32, rot: f32) -> f32 {
    if blades < 3 {
        return 1.0;
    }
    let b = blades as f32;
    let t = (angle - rot).rem_euclid(TAU / b);
    (PI / b).cos() / (t - PI / b).cos()
}

/// Sample tables: `tables[count_idx * PHASES + phase]`, entries are
/// `[dx, dy, dist, weight]` for a unit radius.
fn build_tables(max_samples: usize, blades: u32, rot: f32) -> (Vec<Vec<[f32; 4]>>, Vec<usize>) {
    let mut counts = Vec::new();
    let mut n = 8;
    while n < max_samples {
        counts.push(n);
        n *= 2;
    }
    counts.push(max_samples.max(1));
    counts.dedup();

    let mut tables = Vec::new();
    for &n in &counts {
        for ph in 0..PHASES {
            let mut v = Vec::with_capacity(n);
            for i in 0..n {
                let rho = ((i as f32 + 0.5) / n as f32).sqrt();
                let a = i as f32 * GOLDEN_ANGLE + ph as f32 * TAU / PHASES as f32;
                let f = iris(a, blades, rot);
                // Radial scaling by f squeezes density by 1/f^2; weight f^2
                // restores a uniform response over the polygon.
                v.push([a.cos() * rho * f, a.sin() * rho * f, rho * f, f * f]);
            }
            tables.push(v);
        }
    }
    (tables, counts)
}

struct Ctx<'a> {
    src: &'a [u8],
    depth: &'a [u8],
    w: usize,
    h: usize,
    r_lut: [f32; 256],   // mask level -> blur radius (px)
    ord_lut: [f32; 256], // mask level -> "nearness" (bigger = nearer)
    idx_lut: [u8; 256],  // mask level -> sample table index
    hl_lut: [f32; 256],  // luma -> highlight weight
    use_hl: bool,
    inv_soft: f32,
    tables: Vec<Vec<[f32; 4]>>,
}

/// Apply the lens blur. `img` is interleaved with `c` channels (1..=4),
/// `depth` is a w*h 8-bit mask. Returns a new buffer.
pub fn lens_blur(
    img: &[u8],
    depth: &[u8],
    w: usize,
    h: usize,
    c: usize,
    p: &LensParams,
) -> Vec<u8> {
    assert!((1..=4).contains(&c), "channels must be 1..=4");
    assert_eq!(img.len(), w * h * c, "image buffer size mismatch");
    assert_eq!(depth.len(), w * h, "depth mask must be width*height bytes");
    let mut out = vec![0u8; img.len()];
    if w == 0 || h == 0 {
        return out;
    }

    let max_r = p.max_radius.clamp(0.0, MAX_RADIUS);
    let (tables, counts) = build_tables(p.samples, p.blades, p.rotation_deg.to_radians());

    let norm = (p.focus.max(255.0 - p.focus) - p.focus_range).max(1.0);
    let mut r_lut = [0f32; 256];
    let mut ord_lut = [0f32; 256];
    let mut idx_lut = [0u8; 256];
    let mut hl_lut = [1f32; 256];
    for d in 0..256usize {
        let e = ((d as f32 - p.focus).abs() - p.focus_range).max(0.0) / norm;
        let r = max_r * e.min(1.0);
        r_lut[d] = r;
        ord_lut[d] = if p.white_is_near {
            d as f32
        } else {
            255.0 - d as f32
        };
        let area = PI * r * r;
        idx_lut[d] = counts
            .iter()
            .position(|&n| n as f32 >= area)
            .unwrap_or(counts.len() - 1) as u8;
        let span = (255.0 - p.highlight_threshold).max(1.0);
        let t = ((d as f32 - p.highlight_threshold) / span).clamp(0.0, 1.0);
        hl_lut[d] = 1.0 + p.highlight_gain.max(0.0) * t;
    }

    let ctx = Ctx {
        src: img,
        depth,
        w,
        h,
        r_lut,
        ord_lut,
        idx_lut,
        hl_lut,
        use_hl: p.highlight_gain > 0.0,
        inv_soft: 1.0 / p.edge_softness.max(1e-3),
        tables,
    };

    match c {
        1 => run::<1>(&ctx, &mut out),
        2 => run::<2>(&ctx, &mut out),
        3 => run::<3>(&ctx, &mut out),
        _ => run::<4>(&ctx, &mut out),
    }
    out
}

fn run<const CH: usize>(ctx: &Ctx, out: &mut [u8]) {
    out.par_chunks_mut(ctx.w * CH)
        .enumerate()
        .for_each(|(y, row)| row_pass::<CH>(ctx, y, row));
}

#[inline(always)]
fn pixel_phase(x: usize, y: usize) -> usize {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h >> 28) as usize & (PHASES - 1)
}

fn row_pass<const CH: usize>(ctx: &Ctx, y: usize, row: &mut [u8]) {
    let (src, depth) = (ctx.src, ctx.depth);
    let wi = ctx.w as i32 - 1;
    let hi = ctx.h as i32 - 1;

    for x in 0..ctx.w {
        let p = y * ctx.w + x;
        let dp = depth[p] as usize;
        let rp = ctx.r_lut[dp];
        let center = &src[p * CH..p * CH + CH];

        // Essentially in focus: copy.
        if rp < 0.5 {
            row[x * CH..x * CH + CH].copy_from_slice(center);
            continue;
        }

        let table = &ctx.tables[ctx.idx_lut[dp] as usize * PHASES + pixel_phase(x, y)];
        let ord_p = ctx.ord_lut[dp];
        let fx = x as f32 + BIAS + 0.5;
        let fy = y as f32 + BIAS + 0.5;

        // The pixel itself is always a sample (guarantees a non-zero weight).
        let mut acc = [0f32; CH];
        for k in 0..CH {
            acc[k] = center[k] as f32;
        }
        let mut wsum = 1.0f32;

        for o in table {
            let sx = ((fx + o[0] * rp) as i32 - BIAS_I).clamp(0, wi);
            let sy = ((fy + o[1] * rp) as i32 - BIAS_I).clamp(0, hi);
            let sp = sy as usize * ctx.w + sx as usize;
            let ds = depth[sp] as usize;

            let mut wgt = o[3];
            let ord_s = ctx.ord_lut[ds];
            if ord_s > ord_p {
                // Sample is nearer: counts only if its own disc reaches us.
                let dx = (sx - x as i32) as f32;
                let dy = (sy - y as i32) as f32;
                let dist = (dx * dx + dy * dy).sqrt();
                let cover = (ctx.r_lut[ds] - dist + 0.5).clamp(0.0, 1.0);
                let t = ((ord_s - ord_p) * ctx.inv_soft).min(1.0);
                wgt *= 1.0 + t * (cover - 1.0);
            }

            let px = &src[sp * CH..sp * CH + CH];
            if ctx.use_hl {
                let luma = if CH >= 3 {
                    (px[0] as u32 * 54 + px[1] as u32 * 183 + px[2] as u32 * 19) >> 8
                } else {
                    px[0] as u32
                };
                wgt *= ctx.hl_lut[luma as usize];
            }
            for k in 0..CH {
                acc[k] += wgt * px[k] as f32;
            }
            wsum += wgt;
        }

        let inv = 1.0 / wsum;
        for k in 0..CH {
            row[x * CH + k] = (acc[k] * inv + 0.5) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, path::Path, time::Instant};

    use image::{DynamicImage, imageops::FilterType};

    use super::*;

    fn noise(len: usize, seed: u32) -> Vec<u8> {
        let mut s = seed;
        (0..len)
            .map(|_| {
                s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                (s >> 24) as u8
            })
            .collect()
    }

    #[test]
    fn constant_image_is_unchanged() {
        for c in 1..=4 {
            let img = vec![77u8; 31 * 23 * c];
            let depth = noise(31 * 23, 5);
            let p = LensParams {
                focus: 128.0,
                blades: 6,
                highlight_gain: 4.0,
                ..Default::default()
            };
            assert_eq!(lens_blur(&img, &depth, 31, 23, c, &p), img);
        }
    }

    #[test]
    fn everything_in_focus_is_a_copy() {
        let img = noise(40 * 30 * 3, 1);
        let depth = vec![200u8; 40 * 30];
        let p = LensParams {
            focus: 200.0,
            ..Default::default()
        };
        assert_eq!(lens_blur(&img, &depth, 40, 30, 3, &p), img);
    }

    #[test]
    fn uniform_blur_keeps_mean_and_reduces_noise() {
        let (w, h) = (120, 90);
        let img = noise(w * h, 3);
        let depth = vec![0u8; w * h]; // focus 255 -> full radius everywhere
        let p = LensParams {
            max_radius: 6.0,
            samples: 64,
            ..Default::default()
        };
        let out = lens_blur(&img, &depth, w, h, 1, &p);
        let mean = |v: &[u8]| v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64;
        let var = |v: &[u8]| {
            let m = mean(v);
            v.iter().map(|&x| (x as f64 - m).powi(2)).sum::<f64>() / v.len() as f64
        };
        assert!((mean(&img) - mean(&out)).abs() < 3.0);
        assert!(var(&out) < var(&img) * 0.2);
    }

    #[test]
    fn sharp_foreground_does_not_halo_into_blurred_background() {
        let (w, h) = (64, 32);
        let mut img = vec![0u8; w * h];
        let mut depth = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w / 2 {
                img[y * w + x] = 255; // white, near, in focus
                depth[y * w + x] = 255;
            }
        }
        let p = LensParams {
            focus: 255.0,
            max_radius: 8.0,
            ..Default::default()
        };
        let out = lens_blur(&img, &depth, w, h, 1, &p);
        for y in 0..h {
            for x in 0..w {
                let v = out[y * w + x];
                if x < w / 2 {
                    assert_eq!(v, 255, "foreground changed at {x},{y}");
                } else {
                    assert_eq!(v, 0, "halo at {x},{y}: {v}");
                }
            }
        }
    }
    fn save(img: DynamicImage, path: &str) -> Result<(), Box<dyn Error>> {
        let is_jpeg = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
        if is_jpeg {
            DynamicImage::ImageRgb8(img.to_rgb8()).save(path)?;
        } else {
            img.save(path)?;
        }
        eprintln!("saved {path}");
        Ok(())
    }
    /// Run `f` on the raw interleaved 8-bit pixels of any DynamicImage. 8-bit
    /// gray / RGB / RGBA are used as-is; everything else is converted to RGB(A).
    fn map_pixels(
        img: DynamicImage,
        f: &mut dyn FnMut(&mut [u8], usize, usize, usize),
    ) -> DynamicImage {
        match img {
            DynamicImage::ImageLuma8(mut b) => {
                let (w, h) = b.dimensions();
                f(&mut *b, w as usize, h as usize, 1);
                DynamicImage::ImageLuma8(b)
            }
            DynamicImage::ImageRgb8(mut b) => {
                let (w, h) = b.dimensions();
                f(&mut *b, w as usize, h as usize, 3);
                DynamicImage::ImageRgb8(b)
            }
            DynamicImage::ImageRgba8(mut b) => {
                let (w, h) = b.dimensions();
                f(&mut *b, w as usize, h as usize, 4);
                DynamicImage::ImageRgba8(b)
            }
            other => {
                let converted = if other.color().has_alpha() {
                    DynamicImage::ImageRgba8(other.to_rgba8())
                } else {
                    DynamicImage::ImageRgb8(other.to_rgb8())
                };
                map_pixels(converted, f)
            }
        }
    }

    // #[test]
    fn lens_blur_image() {
        let img = image::open("../../test_images/ariel.webp").unwrap();
        let depth_path = "../../test_images/depth.png";
        let output = "../../test_images/test_lens_image.jpg";
        let (w, h) = (img.width(), img.height());
        let p = LensParams {
            focus: 155.0,
            focus_range: 22.0, // keeps 133..177 fully sharp: head, torso, shoulder
            max_radius: 24.0,  // about 3.5% of this image's width
            samples: 64,
            blades: 6,
            white_is_near: true,
            edge_softness: 10.0,
            ..Default::default()
        };
        eprintln!("{} {}", img.width(), img.height());

        // Depth mask -> 8-bit gray, resized to the photo if the sizes differ.
        let mut mask = image::open(depth_path).unwrap().to_luma8();
        if mask.dimensions() != (w, h) {
            eprintln!("resizing mask {:?} -> {w}x{h}", mask.dimensions());
            mask = image::imageops::resize(&mask, w, h, FilterType::Triangle);
        }
        let depth = mask.into_raw();

        let t = Instant::now();
        let out = map_pixels(img, &mut |buf, w, h, c| {
            let blurred = lens_blur(buf, &depth, w, h, c, &p);
            buf.copy_from_slice(&blurred);
        });
        eprintln!("lens blur in {:?}", t.elapsed());
        _ = save(out, output);

        eprintln!("saved {output}");
    }

    #[test]
    fn odd_sizes_and_iris_shapes_do_not_panic() {
        for &(w, h) in &[(1usize, 1usize), (1, 9), (9, 1), (7, 5)] {
            for c in 1..=4 {
                let img = noise(w * h * c, 11);
                let depth = noise(w * h, 12);
                for blades in [0u32, 3, 5, 8] {
                    let p = LensParams {
                        max_radius: 50.0,
                        blades,
                        rotation_deg: 17.0,
                        highlight_gain: 3.0,
                        samples: 24,
                        ..Default::default()
                    };
                    let out = lens_blur(&img, &depth, w, h, c, &p);
                    assert_eq!(out.len(), img.len());
                }
            }
        }
    }
}
