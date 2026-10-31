//! Fast Gaussian blur for interleaved 8-bit images.
//!
//! * Gaussian ~= 3 successive box blurs (very close for sigma >= ~1.5).
//! * Each box blur is a sliding-window sum: O(1) per pixel, any radius.
//! * Horizontal pass: parallel over rows, const-generic over channel count
//!   (1..=4) so the per-pixel channel loop is fully unrolled.
//! * Vertical pass: no transpose. A running sum is kept for every column and
//!   the image is swept top to bottom, so memory access is sequential and the
//!   inner loop auto-vectorizes (SSE2/AVX2/NEON). Parallel over row bands and
//!   tiled over columns so the sums stay in L1 cache.
//! * 100% safe Rust. Edges clamp to the border pixel.

use rayon::prelude::*;

/// In-place Gaussian blur approximation.
///
/// `img` is interleaved (R,G,B,R,G,B,...), `len == width * height * channels`,
/// `channels` in 1..=4. All channels (including alpha) are blurred equally;
/// if you need correct alpha blending, premultiply first.
pub fn gaussian_blur(img: &mut [u8], width: usize, height: usize, channels: usize, sigma: f32) {
    assert!((1..=4).contains(&channels), "channels must be 1..=4");
    assert_eq!(img.len(), width * height * channels, "buffer size mismatch");
    if sigma <= 0.0 || width == 0 || height == 0 {
        return;
    }
    let mut tmp = vec![0u8; img.len()];
    for r in box_radii(sigma, 3) {
        if r == 0 {
            continue;
        }
        horizontal(img, &mut tmp, width, height, channels, r);
        vertical(&tmp, img, width, height, channels, r);
    }
}

/// Radii of `n` box blurs whose composition approximates a Gaussian.
pub(crate) fn box_radii(sigma: f32, n: usize) -> Vec<usize> {
    let nf = n as f32;
    let w_ideal = (12.0 * sigma * sigma / nf + 1.0).sqrt();
    let mut wl = w_ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wu = wl + 2;
    let wlf = wl as f32;
    let m_ideal =
        (12.0 * sigma * sigma - nf * wlf * wlf - 4.0 * nf * wlf - 3.0 * nf) / (-4.0 * wlf - 4.0);
    let m = m_ideal.round().max(0.0) as usize;
    (0..n)
        .map(|i| {
            let width = if i < m { wl } else { wu };
            ((width.max(1) - 1) / 2) as usize
        })
        .collect()
}

#[inline(always)]
fn q(sum: i32, inv: f32) -> u8 {
    (sum as f32 * inv + 0.5) as u8
}

// ---------------------------------------------------------------- horizontal

fn horizontal(src: &[u8], dst: &mut [u8], w: usize, h: usize, c: usize, r: usize) {
    let _ = h;
    let inv = 1.0 / (2 * r + 1) as f32;
    let row_len = w * c;
    match c {
        1 => rows::<1>(src, dst, row_len, r, inv),
        2 => rows::<2>(src, dst, row_len, r, inv),
        3 => rows::<3>(src, dst, row_len, r, inv),
        _ => rows::<4>(src, dst, row_len, r, inv),
    }
}

fn rows<const CH: usize>(src: &[u8], dst: &mut [u8], row_len: usize, r: usize, inv: f32) {
    dst.par_chunks_mut(row_len)
        .zip(src.par_chunks(row_len))
        .for_each(|(d, s)| blur_row::<CH>(s, d, r, inv));
}

fn blur_row<const CH: usize>(src: &[u8], dst: &mut [u8], r: usize, inv: f32) {
    let w = src.len() / CH;
    let first: [i32; CH] = std::array::from_fn(|k| src[k] as i32);
    let last: [i32; CH] = std::array::from_fn(|k| src[(w - 1) * CH + k] as i32);
    let px = |i: usize, k: usize| src[i.min(w - 1) * CH + k] as i32;

    // Window for x = 0 covers [-r, r]: r copies of the first pixel + [0..=r].
    let mut sum = [0i32; CH];
    for k in 0..CH {
        sum[k] = first[k] * r as i32;
        for x in 0..=r {
            sum[k] += px(x, k);
        }
    }

    // Row shorter than the window: fully clamped slow path.
    if w <= 2 * r + 1 {
        for x in 0..w {
            for k in 0..CH {
                dst[x * CH + k] = q(sum[k], inv);
                let sub = if x >= r { px(x - r, k) } else { first[k] };
                sum[k] += px(x + r + 1, k) - sub;
            }
        }
        return;
    }

    // Left edge: the pixel leaving the window is the clamped first pixel.
    for x in 0..=r {
        for k in 0..CH {
            dst[x * CH + k] = q(sum[k], inv);
            sum[k] += src[(x + r + 1) * CH + k] as i32 - first[k];
        }
    }

    // Interior: no clamping, no bounds checks (iterator zip).
    let n_in = w - 2 * r - 2;
    let out = dst[(r + 1) * CH..(r + 1 + n_in) * CH].chunks_exact_mut(CH);
    let add = src[(2 * r + 2) * CH..].chunks_exact(CH);
    let sub = src[CH..].chunks_exact(CH);
    for ((o, a), s) in out.zip(add).zip(sub) {
        for k in 0..CH {
            o[k] = q(sum[k], inv);
            sum[k] += a[k] as i32 - s[k] as i32;
        }
    }

    // Right edge: the pixel entering the window is the clamped last pixel.
    for x in w - r - 1..w {
        for k in 0..CH {
            dst[x * CH + k] = q(sum[k], inv);
            sum[k] += last[k] - src[(x - r) * CH + k] as i32;
        }
    }
}

// ------------------------------------------------------------------ vertical

/// Samples per column tile; the i32 running sums (8 KiB) live in L1.
const COL_TILE: usize = 2048;

fn vertical(src: &[u8], dst: &mut [u8], w: usize, h: usize, c: usize, r: usize) {
    let row_len = w * c;
    let inv = 1.0 / (2 * r + 1) as f32;
    // Each band re-computes its initial window (2r+1 rows), so keep bands
    // tall relative to r to bound that overhead.
    let band_h = (8 * r).max(64).min(h);
    dst.par_chunks_mut(band_h * row_len)
        .enumerate()
        .for_each(|(bi, band)| vblur_band(src, band, row_len, h, bi * band_h, r, inv));
}

fn vblur_band(
    src: &[u8],
    band: &mut [u8],
    row_len: usize,
    h: usize,
    y0: usize,
    r: usize,
    inv: f32,
) {
    let mut sums_buf = [0i32; COL_TILE];
    let last_row = h as isize - 1;

    for cs in (0..row_len).step_by(COL_TILE) {
        let ce = (cs + COL_TILE).min(row_len);
        let sums = &mut sums_buf[..ce - cs];
        sums.fill(0);

        // Initial window for row y0: rows [y0 - r, y0 + r], clamped.
        for k in 0..=2 * r {
            let yy = (y0 as isize + k as isize - r as isize).clamp(0, last_row) as usize;
            let row = &src[yy * row_len + cs..yy * row_len + ce];
            for (s, &v) in sums.iter_mut().zip(row) {
                *s += v as i32;
            }
        }

        // Sweep down the band.
        for (j, out_row) in band.chunks_exact_mut(row_len).enumerate() {
            let y = y0 + j;
            let add_y = (y + r + 1).min(h - 1);
            let sub_y = y.saturating_sub(r);
            let add = &src[add_y * row_len + cs..add_y * row_len + ce];
            let sub = &src[sub_y * row_len + cs..sub_y * row_len + ce];
            let out = &mut out_row[cs..ce];
            for (((o, s), &a), &b) in out.iter_mut().zip(sums.iter_mut()).zip(add).zip(sub) {
                *o = q(*s, inv);
                *s += a as i32 - b as i32;
            }
        }
    }
}

// --------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use image::DynamicImage;

    use super::*;

    /// Naive float reference: same 3-box algorithm, O(r) per pixel, no
    /// intermediate rounding.
    fn reference(img: &[u8], w: usize, h: usize, c: usize, sigma: f32) -> Vec<f32> {
        let mut cur: Vec<f32> = img.iter().map(|&v| v as f32).collect();
        for r in box_radii(sigma, 3) {
            let n = (2 * r + 1) as f32;
            let mut a = vec![0f32; cur.len()];
            for y in 0..h {
                for x in 0..w {
                    for k in 0..c {
                        let mut s = 0.0;
                        for d in -(r as isize)..=r as isize {
                            let xx = (x as isize + d).clamp(0, w as isize - 1) as usize;
                            s += cur[(y * w + xx) * c + k];
                        }
                        a[(y * w + x) * c + k] = s / n;
                    }
                }
            }
            let mut b = vec![0f32; cur.len()];
            for y in 0..h {
                for x in 0..w {
                    for k in 0..c {
                        let mut s = 0.0;
                        for d in -(r as isize)..=r as isize {
                            let yy = (y as isize + d).clamp(0, h as isize - 1) as usize;
                            s += a[(yy * w + x) * c + k];
                        }
                        b[(y * w + x) * c + k] = s / n;
                    }
                }
            }
            cur = b;
        }
        cur
    }

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
    fn matches_reference_on_many_shapes() {
        let shapes = [
            (1, 1, 1),
            (5, 7, 3),
            (17, 17, 2),
            (64, 50, 4),
            (200, 130, 3),
            (33, 300, 1),
            (2100, 70, 3), // wider than COL_TILE samples
        ];
        for &(w, h, c) in &shapes {
            for &sigma in &[1.5f32, 4.0, 12.0] {
                let img = noise(w * h * c, (w * 31 + h) as u32);
                let want = reference(&img, w, h, c, sigma);
                let mut got = img.clone();
                gaussian_blur(&mut got, w, h, c, sigma);
                let max_err = got
                    .iter()
                    .zip(&want)
                    .map(|(&g, &t)| (g as f32 - t).abs())
                    .fold(0.0f32, f32::max);
                assert!(max_err <= 3.0, "{w}x{h}x{c} sigma={sigma}: err {max_err}");
            }
        }
    }

    #[test]
    fn constant_image_is_unchanged() {
        let mut img = vec![123u8; 37 * 29 * 3];
        gaussian_blur(&mut img, 37, 29, 3, 5.0);
        assert!(img.iter().all(|&v| v == 123));
    }

    fn blur_dynamic(img: DynamicImage, sigma: f32) -> DynamicImage {
        match img {
            DynamicImage::ImageLuma8(mut b) => {
                let (w, h) = b.dimensions();
                gaussian_blur(&mut *b, w as usize, h as usize, 1, sigma);
                DynamicImage::ImageLuma8(b)
            }
            DynamicImage::ImageRgb8(mut b) => {
                let (w, h) = b.dimensions();
                gaussian_blur(&mut *b, w as usize, h as usize, 3, sigma);
                DynamicImage::ImageRgb8(b)
            }
            DynamicImage::ImageRgba8(mut b) => {
                let (w, h) = b.dimensions();
                gaussian_blur(&mut *b, w as usize, h as usize, 4, sigma);
                DynamicImage::ImageRgba8(b)
            }
            other => {
                let converted = if other.color().has_alpha() {
                    DynamicImage::ImageRgba8(other.to_rgba8())
                } else {
                    DynamicImage::ImageRgb8(other.to_rgb8())
                };
                blur_dynamic(converted, sigma)
            }
        }
    }
    // #[test]
    fn gaussian_blur_image() {
        let img = image::open("../../test_images/test.jpg").unwrap();
        let sigma = 2.0;
        let output = "../../test_images/test_gaussian_blur_image.jpg";
        eprintln!("{} {}", img.width(), img.height());
        let t = Instant::now();

        let out = blur_dynamic(img, sigma);
        eprintln!("blurred (sigma={sigma}) in {:?}", t.elapsed());

        // 3. Save (format is chosen from the output extension). JPEG has no
        //    alpha channel, so drop it there.
        let is_jpeg = std::path::Path::new(&output)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
        if is_jpeg {
            DynamicImage::ImageRgb8(out.to_rgb8())
                .save(&output)
                .unwrap();
        } else {
            out.save(&output).unwrap();
        }
        eprintln!("saved {output}");
    }

    #[test]
    fn zero_sigma_is_noop() {
        let img = noise(10 * 10 * 3, 7);
        let mut out = img.clone();
        gaussian_blur(&mut out, 10, 10, 3, 0.0);
        assert_eq!(img, out);
    }
}
