//! Shared pixel math for CPU image operations.

pub(crate) const REC709_LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];
pub(crate) const REC601_LUMA: [f32; 3] = [0.299, 0.587, 0.114];
const CSS_BLEND_LUMA: [f32; 3] = [0.3, 0.59, 0.11];

#[inline]
pub(crate) fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

#[inline]
pub(crate) fn luma709(rgb: [f32; 3]) -> f32 {
    REC709_LUMA[0] * rgb[0] + REC709_LUMA[1] * rgb[1] + REC709_LUMA[2] * rgb[2]
}

#[inline]
#[allow(dead_code)]
pub(crate) fn luma601(rgb: [f32; 3]) -> f32 {
    REC601_LUMA[0] * rgb[0] + REC601_LUMA[1] * rgb[1] + REC601_LUMA[2] * rgb[2]
}

#[inline]
pub(crate) fn css_blend_luma(rgb: [f32; 3]) -> f32 {
    CSS_BLEND_LUMA[0] * rgb[0] + CSS_BLEND_LUMA[1] * rgb[1] + CSS_BLEND_LUMA[2] * rgb[2]
}

/// Convert normalized channel data to a byte, with round-to-nearest semantics.
#[inline]
pub(crate) fn unit_to_u8_round(value: f32) -> u8 {
    (clamp01(value) * 255.0).round() as u8
}

#[inline]
pub(crate) fn byte_to_unit(value: u8) -> f32 {
    value as f32 * (1.0 / 255.0)
}

/// Bilinearly sample a tightly packed RGBA8 image at source coordinates.
#[inline(always)]
pub(crate) fn sample_bilinear_rgba8(
    raw: &[u8],
    src_w: u32,
    src_h: u32,
    sx: f32,
    sy: f32,
) -> [u8; 4] {
    let x0 = (sx.floor() as u32).min(src_w.saturating_sub(1));
    let y0 = (sy.floor() as u32).min(src_h.saturating_sub(1));
    let x1 = (x0 + 1).min(src_w.saturating_sub(1));
    let y1 = (y0 + 1).min(src_h.saturating_sub(1));
    let fx = sx - x0 as f32;
    let fy = sy - y0 as f32;
    let stride = src_w as usize * 4;
    let (r0, r1) = (y0 as usize * stride, y1 as usize * stride);
    let (i00, i10) = (r0 + x0 as usize * 4, r0 + x1 as usize * 4);
    let (i01, i11) = (r1 + x0 as usize * 4, r1 + x1 as usize * 4);
    let weights = [
        (1.0 - fx) * (1.0 - fy),
        fx * (1.0 - fy),
        (1.0 - fx) * fy,
        fx * fy,
    ];
    let indices = [i00, i10, i01, i11];
    let mut out = [0; 4];
    for channel in 0..4 {
        out[channel] = (indices
            .iter()
            .zip(weights)
            .map(|(&index, weight)| raw[index + channel] as f32 * weight)
            .sum::<f32>()
            + 0.5) as u8;
    }
    out
}

/// Bicubic Catmull-Rom sample of tightly packed RGBA8 data, clamped at edges.
#[allow(dead_code)]
pub(crate) fn sample_bicubic_rgba8(
    raw: &[u8],
    src_w: u32,
    src_h: u32,
    sx: f32,
    sy: f32,
) -> [u8; 4] {
    if src_w == 0 || src_h == 0 {
        return [0; 4];
    }
    let (x, y) = (sx.floor() as i32, sy.floor() as i32);
    let (tx, ty) = (sx - x as f32, sy - y as f32);
    let stride = src_w as usize * 4;
    let mut out = [0; 4];
    for channel in 0..4 {
        let mut rows = [0.0; 4];
        for (j, row) in rows.iter_mut().enumerate() {
            let py = (y + j as i32 - 1).clamp(0, src_h.saturating_sub(1) as i32) as usize;
            let mut samples = [0.0; 4];
            for (i, sample) in samples.iter_mut().enumerate() {
                let px = (x + i as i32 - 1).clamp(0, src_w.saturating_sub(1) as i32) as usize;
                *sample = raw[py * stride + px * 4 + channel] as f32;
            }
            *row = catmull_rom(samples, tx);
        }
        out[channel] = catmull_rom(rows, ty).round().clamp(0.0, 255.0) as u8;
    }
    out
}

#[inline]
fn catmull_rom(p: [f32; 4], t: f32) -> f32 {
    let [p0, p1, p2, p3] = p;
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t)
}
