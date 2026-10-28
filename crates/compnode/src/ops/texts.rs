//! Multi-line text rasterizer behind the Texts generator node.
//!
//! The node body holds one multiline editor; every `\n`-separated, non-empty
//! line becomes a row in the output image, stacked top-to-bottom. Each row
//! carries its own size and colour; the font family is shared. Glyphs are
//! drawn with straight-alpha blending (the preview upload treats buffers as
//! unmultiplied).

use crate::{
    ops::effects::{self, Fx},
    params::{self, Params},
};
use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use image::Rgba;
use image::RgbaImage;
use serde_json::Value;

/// Hardcoded font families until custom font loading lands: `(params id,
/// display name, embedded TTF bytes)`.
const FAMILIES: &[(&str, &str, &[u8])] = &[
    (
        "unisans-italic",
        "Unisans-italic",
        include_bytes!("../../assets/fonts/Uni Sans Heavy Italic.otf"),
    ),
    (
        "unisans",
        "Unisans",
        include_bytes!("../../assets/fonts/Uni Sans Heavy.otf"),
    ),
    (
        "anton",
        "Anton",
        include_bytes!("../../assets/fonts/Anton-Regular.ttf"),
    ),
];

/// `(params id, display name)` pairs for the font-family dropdown.
pub(crate) fn families() -> impl Iterator<Item = (&'static str, &'static str)> {
    FAMILIES.iter().map(|(id, name, _)| (*id, *name))
}

fn font_bytes(id: &str) -> &'static [u8] {
    FAMILIES
        .iter()
        .find(|(key, _, _)| *key == id)
        .map(|(_, _, bytes)| *bytes)
        .unwrap_or(FAMILIES[0].2)
}

/// The rendered rows: `\n`-separated, empty lines skipped.
pub(crate) fn split_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .collect()
}

/// Per-row customization, aligned by index with [`split_lines`]. Rows beyond
/// the stored list fall back to these defaults.
#[derive(Clone)]
pub(crate) struct LineStyle {
    pub size: f32,
    pub color: [u8; 3],
}

impl Default for LineStyle {
    fn default() -> Self {
        Self {
            size: 64.0,
            color: [255, 255, 255],
        }
    }
}

pub(crate) fn read_styles(params: &Params) -> Vec<LineStyle> {
    params
        .get("items")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| LineStyle {
                    size: row
                        .get("size")
                        .and_then(Value::as_f64)
                        .map(|size| size as f32)
                        .unwrap_or(64.0),
                    color: row
                        .get("color")
                        .and_then(Value::as_str)
                        .map(params::hex_rgb)
                        .unwrap_or([255, 255, 255]),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn write_styles(params: &mut Params, styles: &[LineStyle]) {
    let rows: Vec<Value> = styles
        .iter()
        .map(|style| {
            serde_json::json!({
                "size": style.size,
                "color": params::rgb_hex(style.color),
            })
        })
        .collect();
    params.insert("items".to_owned(), Value::Array(rows));
}

/// Blend one glyph-coverage pixel over the buffer with straight alpha.
fn blend(pixel: &mut Rgba<u8>, color: [u8; 3], coverage: f32) {
    let coverage = coverage.clamp(0.0, 1.0);
    if coverage <= 0.0 {
        return;
    }
    let src_a = coverage;
    let dst_a = pixel.0[3] as f32 / 255.0;
    let out_a = src_a + dst_a * (1.0 - src_a);
    if out_a <= 0.0 {
        return;
    }
    for channel in 0..3 {
        let src = color[channel] as f32 / 255.0 * src_a;
        let dst = pixel.0[channel] as f32 / 255.0 * dst_a;
        let value = (src + dst * (1.0 - src_a)) / out_a;
        pixel.0[channel] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    pixel.0[3] = (out_a.clamp(0.0, 1.0) * 255.0).round() as u8;
}

/// Draw one line of text with the pen starting at `pen_x`, `baseline` down.
/// `letter_spacing` is added after every character and may be negative.
fn draw_line(
    out: &mut RgbaImage,
    font: &FontRef<'_>,
    size: f32,
    text: &str,
    color: [u8; 3],
    letter_spacing: f32,
    pen_x: f32,
    baseline: f32,
) {
    let scaled = font.as_scaled(size);
    let mut x = pen_x;
    for character in text.chars() {
        let mut glyph = scaled.scaled_glyph(character);
        glyph.position = ab_glyph::point(x, baseline);
        let advance = scaled.h_advance(glyph.id);
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            let (width, height) = out.dimensions();
            outlined.draw(|gx, gy, alpha| {
                // Draw coordinates are relative to `bounds.min`.
                let px = bounds.min.x as i64 + gx as i64;
                let py = bounds.min.y as i64 + gy as i64;
                if px < 0 || py < 0 || px >= width as i64 || py >= height as i64 {
                    return;
                }
                blend(out.get_pixel_mut(px as u32, py as u32), color, alpha);
            });
        }
        x += advance + letter_spacing;
    }
}

struct Layout<'a> {
    rows: Vec<(&'a str, f32, [u8; 3], f32)>, // text, size, color, baseline (block-local, top = 0)
    widths: Vec<f32>,                        // pen width of each row
    width: f32,                              // widest pen width
    spacing: f32,                            // letter spacing at this scale
    fx: Fx,                                  // effects at this scale
}

fn cap_height(font: &FontRef<'_>, size: f32) -> f32 {
    let glyph = font
        .glyph_id('H')
        .with_scale_and_position(PxScale::from(size), point(0.0, 0.0));
    font.outline_glyph(glyph)
        .map(|o| o.px_bounds().height())
        .unwrap_or(size * 0.7)
}

/// Stacks the rows top to bottom. Pure arithmetic: nothing is drawn here.
/// Every size-like value is multiplied by `s`.
fn layout<'a>(
    font: &FontRef<'_>,
    lines: &[(&'a str, f32, [u8; 3])],
    line_gap: f32,
    letter_spacing: f32,
    fx: &Fx,
    s: f32,
) -> Layout<'a> {
    let fx = fx.scaled(s);
    let gap = line_gap * s;
    let spacing = letter_spacing * s;

    let mut rows = Vec::with_capacity(lines.len());
    let mut widths = Vec::with_capacity(lines.len());
    let mut cursor_y = 0.0f32;
    let mut width = 0.0f32;
    for &(line, size, color) in lines {
        let size = (size * s).max(1.0);
        let sc = font.as_scaled(size);
        let baseline = cursor_y + cap_height(font, size);
        let pen: f32 = line
            .chars()
            .map(|c| sc.h_advance(sc.glyph_id(c)))
            .sum::<f32>()
            + spacing * line.chars().count().saturating_sub(1) as f32;
        width = width.max(pen);
        widths.push(pen);
        rows.push((line, size, color, baseline));
        // Space between rows = your gap, plus what the outline and extrusion occupy.
        cursor_y = baseline + gap + fx.outer() * 2.0 + fx.depth as f32 * fx.dir.1.abs() as f32;
    }
    Layout {
        rows,
        widths,
        width,
        spacing,
        fx,
    }
}

pub(crate) fn texts(params: &Params, canvas: (u32, u32)) -> Result<RgbaImage, String> {
    let source = params::string(params, "text", "");
    let text = if params::bool(params, "uppercase", true) {
        source.to_uppercase()
    } else {
        source.to_owned()
    };
    let font_id = params::string(params, "font", "inter");
    let font = FontRef::try_from_slice(font_bytes(font_id))
        .map_err(|err| format!("Could not load font: {err}"))?;
    let styles = read_styles(params);
    let fx = effects::read_fx(params);
    let line_gap = params::f32(params, "line_gap", 0.0).clamp(-512.0, 512.0);
    let letter_spacing = params::f32(params, "letter_spacing", 0.0).clamp(-256.0, 512.0);
    let margin = params::f32(params, "margin", 0.0).clamp(0.0, 512.0);

    let lines: Vec<(&str, f32, [u8; 3])> = split_lines(&text)
        .iter()
        .enumerate()
        .map(|(i, &line)| {
            let st = styles.get(i).cloned().unwrap_or_default();
            (line, st.size.clamp(1.0, 512.0), st.color)
        })
        .collect();

    let (cw, ch) = (canvas.0.max(1), canvas.1.max(1));
    if lines.is_empty() {
        return Ok(RgbaImage::new(cw, ch));
    }

    // 1. Measure at scale 1: widest line plus the effect reach on both sides.
    let natural = layout(&font, &lines, line_gap, letter_spacing, &fx, 1.0);
    let natural_w = natural.width + 2.0 * natural.fx.reach().ceil();
    let avail_w = (cw as f32 - 2.0 * margin).max(1.0);

    // 2. One scale for everything. Never above 1.
    let s = (avail_w / natural_w.max(1.0)).min(1.0);
    let lay = if s < 1.0 {
        layout(&font, &lines, line_gap, letter_spacing, &fx, s * 0.995)
    } else {
        natural
    };

    // 3. Draw straight into the canvas, anchored to its bottom-left corner.
    let reach = lay.fx.reach().ceil();
    let x0 = margin + reach;
    let inner_w = (cw as f32 - 2.0 * x0).max(1.0);
    let centered = params::string(params, "align", "left") == "center";
    let xs: Vec<f32> = lay
        .widths
        .iter()
        .map(|&pen| {
            if centered {
                x0 + (inner_w - pen) * 0.5
            } else {
                x0
            }
        })
        .collect();

    // Bottom of the text block = last baseline plus its descender. `reach`
    // keeps room below the glyphs for shadows/outlines/extrusion.
    let descent = lay.rows.last().map_or(0.0, |&(_, size, _, _)| {
        (-font.as_scaled(size).descent()).max(0.0)
    });
    let block_h = lay.rows.last().map_or(0.0, |&(_, _, _, baseline)| baseline) + descent;
    let y_shift = (ch as f32 - margin - reach - block_h).max(margin + reach);

    let mut img = if lay.fx.is_plain() {
        let mut img = RgbaImage::new(cw, ch);
        for ((line, size, color, baseline), &x) in lay.rows.iter().zip(&xs) {
            draw_line(
                &mut img,
                &font,
                *size,
                line,
                *color,
                lay.spacing,
                x,
                baseline + y_shift,
            );
        }
        img
    } else {
        effects::render_styled(
            &font,
            &lay.rows,
            &xs,
            &lay.fx,
            lay.spacing,
            y_shift,
            cw as usize,
            ch as usize,
        )
        .ok_or_else(|| "Could not build text image".to_owned())?
    };

    // 4. Optional backdrop box, composited underneath the text. Its bounds are
    // the text block (widest rows) grown by the paddings.
    if params::bool(params, "box", false) {
        let pad_x = params::f32(params, "box_padding_x", 24.0).clamp(0.0, 1024.0);
        let pad_y = params::f32(params, "box_padding_y", 12.0).clamp(0.0, 1024.0);
        let left = xs.iter().copied().fold(f32::INFINITY, f32::min) - pad_x;
        let right = xs
            .iter()
            .zip(&lay.widths)
            .map(|(&x, &w)| x + w)
            .fold(f32::NEG_INFINITY, f32::max)
            + pad_x;
        let rect = (left, y_shift - pad_y, right, y_shift + block_h + pad_y);
        paint_box(
            &mut img,
            rect,
            params::color(params, "box_color", "#000000"),
        );
    }

    Ok(img)
}

/// Fill an axis-aligned rectangle with an opaque colour, compositing any text
/// already drawn on top of it.
fn paint_box(
    img: &mut RgbaImage,
    (left, top, right, bottom): (f32, f32, f32, f32),
    color: [u8; 3],
) {
    let (cw, ch) = img.dimensions();
    let x0 = left.floor().clamp(0.0, cw as f32) as u32;
    let y0 = top.floor().clamp(0.0, ch as f32) as u32;
    let x1 = right.ceil().clamp(0.0, cw as f32) as u32;
    let y1 = bottom.ceil().clamp(0.0, ch as f32) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let pixel = img.get_pixel_mut(x, y);
            let alpha = pixel.0[3] as f32 / 255.0;
            for channel in 0..3 {
                let src = pixel.0[channel] as f32 * alpha;
                let dst = color[channel] as f32 * (1.0 - alpha);
                pixel.0[channel] = (src + dst).round() as u8;
            }
            pixel.0[3] = 255;
        }
    }
}
