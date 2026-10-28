use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use image::RgbaImage;
pub struct Mask {
    pub w: usize,
    pub h: usize,
    pub a: Vec<f32>,
}
impl Mask {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            a: vec![0.0; w * h],
        }
    }
}
use crate::params::{self, Params};

/// Premultiplied RGBA canvas.
struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[f32; 4]>,
}
impl Canvas {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![[0.0; 4]; w * h],
        }
    }

    pub fn paint(&mut self, m: &Mask, rgb: [f32; 3], opacity: f32, (ox, oy): (i32, i32)) {
        for y in 0..self.h as i32 {
            let sy = y - oy;
            if sy < 0 || sy >= m.h as i32 {
                continue;
            }
            for x in 0..self.w as i32 {
                let sx = x - ox;
                if sx < 0 || sx >= m.w as i32 {
                    continue;
                }
                let a = m.a[sy as usize * m.w + sx as usize] * opacity;
                if a <= 0.0 {
                    continue;
                }
                let d = &mut self.px[y as usize * self.w + x as usize];
                let k = 1.0 - a;
                d[0] = rgb[0] * a + d[0] * k;
                d[1] = rgb[1] * a + d[1] * k;
                d[2] = rgb[2] * a + d[2] * k;
                d[3] = a + d[3] * k;
            }
        }
    }
    pub fn to_bytes(&self, premultiplied: bool) -> Vec<u8> {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        self.px
            .iter()
            .flat_map(|p| {
                let s = if premultiplied || p[3] <= 0.0 {
                    1.0
                } else {
                    1.0 / p[3]
                };
                [q(p[0] * s), q(p[1] * s), q(p[2] * s), q(p[3])]
            })
            .collect()
    }
    pub fn into_image(self) -> Option<image::RgbaImage> {
        let (w, h) = (self.w as u32, self.h as u32);
        image::RgbaImage::from_raw(w, h, self.to_bytes(false)) // straight alpha
    }
}
struct ShadowFx {
    color: [f32; 3],
    offset: (i32, i32),
    blur: usize,
    opacity: f32,
}

pub(crate) struct Fx {
    /// (reach in px from the glyph edge, color); sorted outer-first.
    strokes: Vec<(f32, [f32; 3])>,
    pub depth: u32,
    pub dir: (i32, i32),
    side: [f32; 3],
    shadow: Option<ShadowFx>,
}

impl Fx {
    pub fn is_plain(&self) -> bool {
        self.strokes.is_empty() && self.depth == 0 && self.shadow.is_none()
    }
    pub fn outer(&self) -> f32 {
        self.strokes.first().map_or(0.0, |s| s.0)
    }
    /// Farthest any effect can paint from the glyph, in px.
    pub fn reach(&self) -> f32 {
        let ext = self.depth as f32 * self.dir.0.abs().max(self.dir.1.abs()) as f32;
        let sh = self.shadow.as_ref().map_or(0.0, |s| {
            s.offset.0.abs().max(s.offset.1.abs()) as f32 + 2.0 * s.blur as f32
        });
        self.outer() + ext + sh + 2.0
    }
    /// Every size in the effect stack multiplied by `s`.
    pub fn scaled(&self, s: f32) -> Fx {
        let px = |v: i32| (v as f32 * s).round() as i32;
        Fx {
            strokes: self.strokes.iter().map(|&(w, c)| (w * s, c)).collect(),
            depth: (self.depth as f32 * s).round() as u32,
            dir: self.dir,
            side: self.side,
            shadow: self.shadow.as_ref().map(|sh| ShadowFx {
                color: sh.color,
                offset: (px(sh.offset.0), px(sh.offset.1)),
                blur: (sh.blur as f32 * s).round() as usize,
                opacity: sh.opacity,
            }),
        }
    }
}

fn color_param(params: &Params, key: &str, default: [f32; 3]) -> [f32; 3] {
    let s = params::string(params, key, "").trim_start_matches('#');
    if s.len() != 6 {
        return default;
    }
    let v = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).map(|b| f32::from(b) / 255.0);
    match (v(0), v(2), v(4)) {
        (Ok(r), Ok(g), Ok(b)) => [r, g, b],
        _ => default,
    }
}

pub(crate) fn read_fx(params: &Params) -> Fx {
    // One switch turns the whole decorative stack off without losing the
    // individual widths/opacities, so toggling back on restores them.
    let enabled = params::bool(params, "effects", true);
    let mut strokes = Vec::new();
    if enabled {
        for (wk, ck, dc) in [
            ("outline_width", "outline_color", [0.0; 3]),
            ("outline2_width", "outline2_color", [1.0; 3]),
        ] {
            let w = params::f32(params, wk, 0.0).clamp(0.0, 128.0);
            if w > 0.0 {
                strokes.push((w, color_param(params, ck, dc)));
            }
        }
        strokes.sort_by(|a, b| b.0.total_cmp(&a.0));
    }

    let shadow_opacity = if enabled {
        params::f32(params, "shadow_opacity", 0.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let shadow = (shadow_opacity > 0.0).then(|| ShadowFx {
        color: color_param(params, "shadow_color", [0.0; 3]),
        offset: (
            params::f32(params, "shadow_x", 0.0)
                .clamp(-256.0, 256.0)
                .round() as i32,
            params::f32(params, "shadow_y", 8.0)
                .clamp(-256.0, 256.0)
                .round() as i32,
        ),
        blur: params::f32(params, "shadow_blur", 6.0)
            .clamp(0.0, 64.0)
            .round() as usize,
        opacity: shadow_opacity,
    });

    Fx {
        strokes,
        depth: if enabled {
            params::f32(params, "depth", 0.0).clamp(0.0, 64.0).round() as u32
        } else {
            0
        },
        dir: (
            params::f32(params, "depth_x", 1.0).clamp(-4.0, 4.0).round() as i32,
            params::f32(params, "depth_y", 1.0).clamp(-4.0, 4.0).round() as i32,
        ),
        side: color_param(params, "depth_color", [0.0; 3]),
        shadow,
    }
}

fn line_mask(
    font: &FontRef,
    size: f32,
    line: &str,
    spacing: f32,
    x0: f32,
    baseline: f32,
    w: usize,
    h: usize,
) -> Mask {
    let scale = PxScale::from(size);
    let scaled = font.as_scaled(scale);
    let mut m = Mask::new(w, h);
    let mut x = x0;
    for ch in line.chars() {
        let id = scaled.glyph_id(ch);
        let glyph = id.with_scale_and_position(scale, point(x, baseline));
        x += scaled.h_advance(id) + spacing;
        if let Some(og) = font.outline_glyph(glyph) {
            let b = og.px_bounds();
            og.draw(|gx, gy, c| {
                let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                    let i = py as usize * w + px as usize;
                    m.a[i] = m.a[i].max(c);
                }
            });
        }
    }
    m
}

pub fn render_styled(
    font: &FontRef,
    rows: &[(&str, f32, [u8; 3], f32)], // text, size, color, baseline
    xs: &[f32],                         // pen start x per row
    fx: &Fx,
    letter_spacing: f32,
    y_shift: f32,
    w: usize,
    h: usize,
) -> Option<RgbaImage> {
    struct Prepared {
        glyph: Mask,
        dist: Vec<f32>,
        silhouette: Mask,
        fill: [f32; 3],
    }

    let outer = fx.outer();
    debug_assert_eq!(xs.len(), rows.len());
    let prepared: Vec<Prepared> = rows
        .iter()
        .zip(xs)
        .map(|((line, size, color, baseline), &x)| {
            let glyph = line_mask(
                font,
                *size,
                line,
                letter_spacing,
                x,
                baseline + y_shift,
                w,
                h,
            );
            let (dist, silhouette) = if outer > 0.0 {
                let d = distance_outside(&glyph);
                let sil = grow(&glyph, &d, outer);
                (d, sil)
            } else {
                (
                    Vec::new(),
                    Mask {
                        w,
                        h,
                        a: glyph.a.clone(),
                    },
                )
            };
            Prepared {
                glyph,
                dist,
                silhouette,
                fill: color.map(|c| f32::from(c) / 255.0),
            }
        })
        .collect();

    let mut canvas = Canvas::new(w, h);

    // 1. Shadows of all lines.
    if let Some(sh) = &fx.shadow {
        let off = (
            fx.dir.0 * fx.depth as i32 + sh.offset.0,
            fx.dir.1 * fx.depth as i32 + sh.offset.1,
        );
        for p in &prepared {
            let mut m = Mask {
                w,
                h,
                a: p.silhouette.a.clone(),
            };
            for _ in 0..2 {
                box_blur(&mut m.a, w, h, sh.blur);
            }
            canvas.paint(&m, sh.color, sh.opacity, off);
        }
    }
    // 2. Extrusion of all lines, far to near.
    for step in (1..=fx.depth as i32).rev() {
        for p in &prepared {
            canvas.paint(
                &p.silhouette,
                fx.side,
                1.0,
                (fx.dir.0 * step, fx.dir.1 * step),
            );
        }
    }
    // 3. Outlines of all lines, outer to inner.
    for (width, color) in &fx.strokes {
        for p in &prepared {
            canvas.paint(&grow(&p.glyph, &p.dist, *width), *color, 1.0, (0, 0));
        }
    }
    // 4. Fills last, so every face sits on top.
    for p in &prepared {
        canvas.paint(&p.glyph, p.fill, 1.0, (0, 0));
    }
    canvas.into_image()
}

/// Distance (px) from each pixel to the nearest glyph pixel. Two-pass chamfer.
pub fn distance_outside(m: &Mask) -> Vec<f32> {
    let (w, h) = (m.w, m.h);
    let s2 = std::f32::consts::SQRT_2;
    let mut d: Vec<f32> =
        m.a.iter()
            .map(|&a| if a >= 0.5 { 0.0 } else { 1e9 })
            .collect();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut v = d[i];
            if x > 0 {
                v = v.min(d[i - 1] + 1.0);
            }
            if y > 0 {
                v = v.min(d[i - w] + 1.0);
                if x > 0 {
                    v = v.min(d[i - w - 1] + s2);
                }
                if x + 1 < w {
                    v = v.min(d[i - w + 1] + s2);
                }
            }
            d[i] = v;
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            let mut v = d[i];
            if x + 1 < w {
                v = v.min(d[i + 1] + 1.0);
            }
            if y + 1 < h {
                v = v.min(d[i + w] + 1.0);
                if x > 0 {
                    v = v.min(d[i + w - 1] + s2);
                }
                if x + 1 < w {
                    v = v.min(d[i + w + 1] + s2);
                }
            }
            d[i] = v;
        }
    }
    d
}

/// Glyph grown outward by `width` px, antialiased. Reuses one distance field for any width.
pub fn grow(glyph: &Mask, dist: &[f32], width: f32) -> Mask {
    let a = glyph
        .a
        .iter()
        .zip(dist)
        .map(|(&g, &d)| g.max((width + 0.5 - d).clamp(0.0, 1.0)))
        .collect();
    Mask {
        w: glyph.w,
        h: glyph.h,
        a,
    }
}

fn box_blur(a: &mut [f32], w: usize, h: usize, r: usize) {
    let k = (2 * r + 1) as f32;
    let mut tmp = vec![0.0; a.len()];
    for y in 0..h {
        let row = &a[y * w..(y + 1) * w];
        let mut sum: f32 = row[..=r.min(w - 1)].iter().sum();
        for x in 0..w {
            tmp[y * w + x] = sum / k;
            if x + r + 1 < w {
                sum += row[x + r + 1];
            }
            if x >= r {
                sum -= row[x - r];
            }
        }
    }
    for x in 0..w {
        let mut sum: f32 = (0..=r.min(h - 1)).map(|y| tmp[y * w + x]).sum();
        for y in 0..h {
            a[y * w + x] = sum / k;
            if y + r + 1 < h {
                sum += tmp[(y + r + 1) * w + x];
            }
            if y >= r {
                sum -= tmp[(y - r) * w + x];
            }
        }
    }
}
