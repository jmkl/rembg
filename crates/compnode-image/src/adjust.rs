//! Color adjustments ported from the original compnode: Exposure, Color
//! Balance, Gradient Map and Mask.
//!
//! Pixel convention (see `docs/GUIDE.md`, "Ported nodes"): buffers are
//! linear-light and premultiplied like everywhere else. Color Balance,
//! Gradient Map and Mask were tuned on sRGB-encoded 8-bit values, so they
//! encode to sRGB, apply the original formula and decode again
//! ([`crate::cpu::map_srgb`]); their results match the old look. Exposure is
//! a photographic gain and works directly in linear light.
//!
//! Parameters use the new `0..1` conventions: where the original editor used
//! percent (`-100..100`), these functions take `-1..1`.
use crate::buffer::ImageBuf;
use crate::cpu::{enc, luma, map_srgb, map_straight, resize};
use compnode_core::Color;

/// Exposure in linear light: `out = max(in * 2^ev + offset, 0) ^ (1 / gamma)`.
/// `ev` is clamped to `-5..=5`, `offset` to `-0.5..=0.5` and `gamma` to
/// `0.1..=3`. Values above white are kept (no upper clamp).
pub fn exposure(img: &ImageBuf, ev: f32, offset: f32, gamma: f32) -> ImageBuf {
    let gain = 2.0_f32.powf(ev.clamp(-5.0, 5.0));
    let offset = offset.clamp(-0.5, 0.5);
    let inv_gamma = 1.0 / gamma.clamp(0.1, 3.0);
    let f = |v: f32| (v * gain + offset).max(0.0).powf(inv_gamma);
    map_straight(img, |c| [f(c[0]), f(c[1]), f(c[2])])
}

/// Which tones a color balance shift affects.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToneRange {
    Shadows,
    Midtones,
    Highlights,
}

impl ToneRange {
    pub const ALL: [ToneRange; 3] = [ToneRange::Shadows, ToneRange::Midtones, ToneRange::Highlights];
    pub fn name(self) -> &'static str {
        match self {
            ToneRange::Shadows => "Shadows",
            ToneRange::Midtones => "Midtones",
            ToneRange::Highlights => "Highlights",
        }
    }
    pub fn from_index(i: usize) -> ToneRange {
        ToneRange::ALL.get(i).copied().unwrap_or(ToneRange::Midtones)
    }
    fn weight(self, lum: f32) -> f32 {
        match self {
            ToneRange::Shadows => (1.0 - lum) * (1.0 - lum),
            ToneRange::Highlights => lum * lum,
            ToneRange::Midtones => 4.0 * lum * (1.0 - lum),
        }
    }
}

/// Shift cyan/red, magenta/green and yellow/blue (each `-1..1`) within a
/// tone range. With `preserve_luma` the result is rescaled to the original
/// luminance.
pub fn color_balance(
    img: &ImageBuf,
    cyan_red: f32,
    magenta_green: f32,
    yellow_blue: f32,
    range: ToneRange,
    preserve_luma: bool,
) -> ImageBuf {
    let (cr, mg, yb) = (cyan_red.clamp(-1.0, 1.0), magenta_green.clamp(-1.0, 1.0), yellow_blue.clamp(-1.0, 1.0));
    map_srgb(img, |c| {
        let lum = luma(c);
        let w = range.weight(lum);
        let mut n = [c[0] + cr * 0.5 * w, c[1] + mg * 0.5 * w, c[2] + yb * 0.5 * w];
        if preserve_luma {
            let next = luma(n);
            if next > 0.0001 {
                let k = lum / next;
                n = [n[0] * k, n[1] * k, n[2] * k];
            }
        }
        n
    })
}

fn encoded(c: Color) -> [f32; 3] {
    [enc(c.r), enc(c.g), enc(c.b)]
}

/// Map luminance to a three-color gradient (`shadow` at 0, `mid` at
/// `midpoint`, `highlight` at 1), interpolated in sRGB space. `reverse`
/// flips the mapping. Alpha is kept.
pub fn gradient_map(img: &ImageBuf, shadow: Color, mid: Color, highlight: Color, midpoint: f32, reverse: bool) -> ImageBuf {
    let midpoint = midpoint.clamp(0.0, 1.0);
    let (s, m, h) = (encoded(shadow), encoded(mid), encoded(highlight));
    map_srgb(img, |c| {
        let mut t = luma(c).clamp(0.0, 1.0);
        if reverse {
            t = 1.0 - t;
        }
        let (from, to, local) = if t <= midpoint {
            (s, m, if midpoint <= 0.0 { 0.0 } else { t / midpoint })
        } else {
            (m, h, if midpoint >= 1.0 { 0.0 } else { (t - midpoint) / (1.0 - midpoint) })
        };
        [
            from[0] + (to[0] - from[0]) * local,
            from[1] + (to[1] - from[1]) * local,
            from[2] + (to[2] - from[2]) * local,
        ]
    })
}

/// Multiply the image (all four premultiplied channels, so its alpha) by a
/// factor derived from the luminance of `mask`. The mask is resized to the
/// image. Its luminance is taken on the sRGB-encoded straight color, so a
/// mask painted 50 % gray masks by 50 %, and its own alpha is ignored where
/// it is zero. Without a mask the image passes through.
///
/// `light_is_opaque`: a bright mask keeps the image (otherwise a dark one
/// does). `strength` blends between no effect (0) and the full mask (1).
pub fn mask(img: &ImageBuf, mask: Option<&ImageBuf>, light_is_opaque: bool, strength: f32) -> ImageBuf {
    let Some(m) = mask else {
        return img.clone();
    };
    let resized;
    let m = if (m.width, m.height) == (img.width, img.height) {
        m
    } else {
        resized = resize(m, img.width, img.height);
        &resized
    };
    let strength = strength.clamp(0.0, 1.0);
    let mut out = ImageBuf::new(img.width, img.height);
    for (i, (p, q)) in img.data.iter().zip(&m.data).enumerate() {
        let brightness = if q[3] > 0.0 {
            let l = luma([q[0] / q[3], q[1] / q[3], q[2] / q[3]]);
            enc(l)
        } else {
            0.0
        };
        let opaque = if light_is_opaque { brightness } else { 1.0 - brightness };
        let k = 1.0 - strength + opaque * strength;
        out.data[i] = [p[0] * k, p[1] * k, p[2] * k, p[3] * k];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::dec;

    fn solid(r: f32, g: f32, b: f32) -> ImageBuf {
        ImageBuf::solid(4, 4, Color::rgb(r, g, b))
    }
    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 2e-3
    }

    #[test]
    fn exposure_doubles_per_stop_and_is_identity_at_defaults() {
        let img = solid(0.1, 0.2, 0.3);
        let one = exposure(&img, 1.0, 0.0, 1.0).get(0, 0);
        assert!(near(one[0], 0.2) && near(one[1], 0.4) && near(one[2], 0.6), "{one:?}");
        let id = exposure(&img, 0.0, 0.0, 1.0);
        assert!(id.max_abs_diff(&img).unwrap() < 1e-6);
        // offset adds, gamma 2 takes the square root, values above white survive
        let off = exposure(&img, 0.0, 0.25, 1.0).get(0, 0);
        assert!(near(off[0], 0.35));
        let g = exposure(&solid(0.25, 0.25, 0.25), 0.0, 0.0, 2.0).get(0, 0);
        assert!(near(g[0], 0.5));
        let bright = exposure(&solid(0.8, 0.8, 0.8), 2.0, 0.0, 1.0).get(0, 0);
        assert!(bright[0] > 1.0);
        // ev beyond the limit is clamped to 5 stops
        let capped = exposure(&solid(0.01, 0.01, 0.01), 50.0, 0.0, 1.0).get(0, 0);
        assert!(near(capped[0], 0.32));
    }

    #[test]
    fn exposure_keeps_alpha_and_transparent_pixels() {
        let mut img = ImageBuf::new(2, 1);
        img.set(0, 0, [0.1, 0.1, 0.1, 0.5]); // straight 0.2 at alpha 0.5
        let out = exposure(&img, 1.0, 0.0, 1.0);
        assert!(near(out.get(0, 0)[3], 0.5) && near(out.get(0, 0)[0], 0.2));
        assert_eq!(out.get(1, 0), [0.0; 4]);
    }

    #[test]
    fn color_balance_is_identity_at_zero_and_pushes_the_chosen_axis() {
        let img = solid(0.2, 0.2, 0.2);
        let id = color_balance(&img, 0.0, 0.0, 0.0, ToneRange::Midtones, true);
        assert!(id.max_abs_diff(&img).unwrap() < 2e-3);
        let red = color_balance(&img, 1.0, 0.0, 0.0, ToneRange::Midtones, false).get(0, 0);
        assert!(red[0] > 0.2 && near(red[1], 0.2) && near(red[2], 0.2), "{red:?}");
        let blue = color_balance(&img, 0.0, 0.0, 1.0, ToneRange::Midtones, false).get(0, 0);
        assert!(blue[2] > 0.2 && near(blue[0], 0.2));
    }

    #[test]
    fn color_balance_ranges_weight_the_tones() {
        let dark = solid(0.01, 0.01, 0.01);
        let bright = solid(0.9, 0.9, 0.9);
        let shift = |img: &ImageBuf, r| color_balance(img, 1.0, 0.0, 0.0, r, false).get(0, 0)[0] - img.get(0, 0)[0];
        assert!(shift(&dark, ToneRange::Shadows) > shift(&dark, ToneRange::Highlights));
        assert!(shift(&bright, ToneRange::Highlights) > shift(&bright, ToneRange::Shadows));
        // preserving luma keeps the luminance of a gray close to what it was
        let img = solid(0.3, 0.3, 0.3);
        let out = color_balance(&img, 0.4, -0.2, 0.0, ToneRange::Midtones, true).get(0, 0);
        let l = crate::cpu::luma([enc(out[0]), enc(out[1]), enc(out[2])]);
        assert!((l - enc(0.3)).abs() < 0.01, "{l}");
    }

    #[test]
    fn gradient_map_maps_black_gray_and_white() {
        let (s, m, h) = (Color::rgb(0.0, 0.0, 1.0), Color::rgb(0.0, 1.0, 0.0), Color::rgb(1.0, 0.0, 0.0));
        let at = |v: f32, mid: f32, rev: bool| gradient_map(&solid(v, v, v), s, m, h, mid, rev).get(0, 0);
        let black = at(0.0, 0.5, false);
        assert!(near(black[2], 1.0) && near(black[0], 0.0));
        let white = at(1.0, 0.5, false);
        assert!(near(white[0], 1.0) && near(white[2], 0.0));
        // the midpoint color sits at the midpoint (encoded luma 0.5 is linear 0.2140)
        let mid = at(lin(0.5), 0.5, false);
        assert!(near(mid[1], 1.0), "{mid:?}");
        // reverse swaps the ends
        let rev = at(0.0, 0.5, true);
        assert!(near(rev[0], 1.0));
        // alpha is kept
        let mut half = ImageBuf::new(1, 1);
        half.set(0, 0, [0.0, 0.0, 0.0, 0.5]);
        assert!(near(gradient_map(&half, s, m, h, 0.5, false).get(0, 0)[3], 0.5));
    }

    fn lin(encoded: f32) -> f32 {
        dec(encoded)
    }

    #[test]
    fn mask_scales_alpha_by_the_mask_and_passes_through_without_one() {
        let img = solid(0.5, 0.5, 0.5);
        assert_eq!(mask(&img, None, false, 1.0), img);
        let white = solid(1.0, 1.0, 1.0);
        let black = solid(0.0, 0.0, 0.0);
        // dark is opaque: black keeps the image, white removes it
        assert!(near(mask(&img, Some(&black), false, 1.0).get(0, 0)[3], 1.0));
        assert!(near(mask(&img, Some(&white), false, 1.0).get(0, 0)[3], 0.0));
        // light is opaque: the other way around
        assert!(near(mask(&img, Some(&white), true, 1.0).get(0, 0)[3], 1.0));
        assert!(near(mask(&img, Some(&black), true, 1.0).get(0, 0)[3], 0.0));
        // strength 0 means no effect, 0.5 means halfway
        assert!(near(mask(&img, Some(&white), false, 0.0).get(0, 0)[3], 1.0));
        assert!(near(mask(&img, Some(&white), false, 0.5).get(0, 0)[3], 0.5));
        // a sRGB 50 % gray masks by 50 %
        let gray = solid(lin(0.5), lin(0.5), lin(0.5));
        assert!(near(mask(&img, Some(&gray), true, 1.0).get(0, 0)[3], 0.5));
        // color channels are scaled with alpha, so the result stays premultiplied
        let out = mask(&img, Some(&gray), true, 1.0).get(0, 0);
        assert!(near(out[0], 0.25));
    }

    #[test]
    fn mask_is_resized_to_the_image() {
        let img = solid(1.0, 1.0, 1.0);
        let small = ImageBuf::solid(1, 1, Color::rgb(0.0, 0.0, 0.0));
        let out = mask(&img, Some(&small), false, 1.0);
        assert_eq!((out.width, out.height), (4, 4));
        assert!(near(out.get(3, 3)[3], 1.0));
    }
}
