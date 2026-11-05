//! Linear-light RGBA color with sRGB and hex conversion.
//!
//! The image engine works in linear light, so [`Color`] stores **linear**
//! `f32` channels. Use [`Color::from_srgb8`] / [`Color::to_srgb8`] (or the hex
//! helpers) at the edges, where UI colors and files are sRGB. Channels are
//! *straight* (not premultiplied) unless you call [`Color::premultiplied`].

/// sRGB-encoded value in `0..=1` to linear light.
#[inline]
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light in `0..=1` to an sRGB-encoded value.
#[inline]
pub fn linear_to_srgb(l: f32) -> f32 {
    if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

/// RGB (each `0..=1`) to HSV, with hue in `0..1` (0 = red, 1/3 = green).
/// The transfer curve is not touched: pass sRGB-encoded values for a
/// perceptual picker, or linear values for linear-light operations.
pub fn rgb_to_hsv(r: f32, g: f32, b: f32) -> [f32; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let s = if max <= 0.0 { 0.0 } else { d / max };
    let h6 = if d <= 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [h6 / 6.0, s, max]
}

/// Inverse of [`rgb_to_hsv`]. Hue wraps around (`1.0` is the same as `0.0`).
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor();
    let f = h6 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match (i as i32) % 6 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// Linear-light RGBA (straight alpha unless stated otherwise).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color::new(0.0, 0.0, 0.0, 0.0);
    pub const BLACK: Color = Color::new(0.0, 0.0, 0.0, 1.0);
    pub const WHITE: Color = Color::new(1.0, 1.0, 1.0, 1.0);

    /// From linear-light channels.
    #[inline]
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Color {
        Color { r, g, b, a }
    }

    /// Opaque color from linear-light channels.
    #[inline]
    pub const fn rgb(r: f32, g: f32, b: f32) -> Color {
        Color::new(r, g, b, 1.0)
    }

    /// From 8-bit sRGB channels. Alpha is linear (`a / 255`).
    pub fn from_srgb8(r: u8, g: u8, b: u8, a: u8) -> Color {
        let ch = |v: u8| srgb_to_linear(f32::from(v) / 255.0);
        Color::new(ch(r), ch(g), ch(b), f32::from(a) / 255.0)
    }

    /// To 8-bit sRGB channels (clamped and rounded). Alpha is linear.
    pub fn to_srgb8(self) -> [u8; 4] {
        let ch = |l: f32| (linear_to_srgb(l.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
        [
            ch(self.r),
            ch(self.g),
            ch(self.b),
            (self.a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        ]
    }

    /// Parse `#RRGGBB`, `#RRGGBBAA` (the `#` is optional) as sRGB.
    pub fn from_hex(s: &str) -> Option<Color> {
        let s = s.strip_prefix('#').unwrap_or(s);
        if !s.is_ascii() || (s.len() != 6 && s.len() != 8) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
        let a = if s.len() == 8 { byte(6)? } else { 255 };
        Some(Color::from_srgb8(byte(0)?, byte(2)?, byte(4)?, a))
    }

    /// `#rrggbb`, or `#rrggbbaa` when alpha is not fully opaque.
    pub fn to_hex(self) -> String {
        let [r, g, b, a] = self.to_srgb8();
        if a == 255 {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        }
    }

    #[inline]
    pub fn with_alpha(self, a: f32) -> Color {
        Color { a, ..self }
    }

    /// Per-channel linear interpolation (in linear light).
    pub fn lerp(self, o: Color, t: f32) -> Color {
        let m = |a: f32, b: f32| a + (b - a) * t;
        Color::new(
            m(self.r, o.r),
            m(self.g, o.g),
            m(self.b, o.b),
            m(self.a, o.a),
        )
    }

    /// Multiply RGB by alpha.
    #[inline]
    pub fn premultiplied(self) -> Color {
        Color::new(self.r * self.a, self.g * self.a, self.b * self.a, self.a)
    }

    /// Inverse of [`Color::premultiplied`]. Fully transparent pixels become
    /// [`Color::TRANSPARENT`].
    pub fn unpremultiplied(self) -> Color {
        if self.a <= 0.0 {
            return Color::TRANSPARENT;
        }
        let inv = 1.0 / self.a;
        Color::new(self.r * inv, self.g * inv, self.b * inv, self.a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb8_roundtrips_every_value() {
        for v in 0..=255u8 {
            let c = Color::from_srgb8(v, v, v, v);
            assert_eq!(c.to_srgb8(), [v, v, v, v], "value {v}");
        }
    }

    #[test]
    fn srgb_transfer_endpoints_and_midpoint() {
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-6);
        // sRGB 0.5 is about 0.214 in linear light
        assert!((srgb_to_linear(0.5) - 0.2140).abs() < 1e-3);
        assert!((linear_to_srgb(srgb_to_linear(0.37)) - 0.37).abs() < 1e-5);
    }

    #[test]
    fn hex_parse_and_format() {
        let c = Color::from_hex("#ff8000").expect("valid");
        assert_eq!(c.to_srgb8(), [255, 128, 0, 255]);
        assert_eq!(c.to_hex(), "#ff8000");

        let c = Color::from_hex("ff800080").expect("valid, no #");
        assert_eq!(c.to_srgb8(), [255, 128, 0, 128]);
        assert_eq!(c.to_hex(), "#ff800080");

        for bad in ["", "#fff", "#12345", "#gg0000", "#ff80000", "é12345"] {
            assert!(Color::from_hex(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn premultiply_roundtrip_and_transparent() {
        let c = Color::new(0.8, 0.4, 0.2, 0.5);
        let back = c.premultiplied().unpremultiplied();
        for (x, y) in [(c.r, back.r), (c.g, back.g), (c.b, back.b), (c.a, back.a)] {
            assert!((x - y).abs() < 1e-6);
        }
        assert_eq!(
            Color::new(0.9, 0.9, 0.9, 0.0).unpremultiplied(),
            Color::TRANSPARENT
        );
    }

    #[test]
    fn hsv_roundtrip_and_primaries() {
        assert_eq!(rgb_to_hsv(1.0, 0.0, 0.0), [0.0, 1.0, 1.0]);
        let g = rgb_to_hsv(0.0, 1.0, 0.0);
        assert!((g[0] - 1.0 / 3.0).abs() < 1e-6 && g[1] == 1.0 && g[2] == 1.0);
        assert_eq!(rgb_to_hsv(0.5, 0.5, 0.5), [0.0, 0.0, 0.5]);
        for (r, g, b) in [
            (0.2, 0.7, 0.4),
            (0.9, 0.1, 0.8),
            (0.3, 0.3, 0.9),
            (1.0, 0.5, 0.0),
        ] {
            let [h, s, v] = rgb_to_hsv(r, g, b);
            let [r2, g2, b2] = hsv_to_rgb(h, s, v);
            assert!((r - r2).abs() < 1e-5 && (g - g2).abs() < 1e-5 && (b - b2).abs() < 1e-5);
        }
        assert_eq!(hsv_to_rgb(1.0, 1.0, 1.0), hsv_to_rgb(0.0, 1.0, 1.0));
    }

    #[test]
    fn lerp_and_alpha() {
        let m = Color::BLACK.lerp(Color::WHITE, 0.25);
        assert_eq!((m.r, m.g, m.b, m.a), (0.25, 0.25, 0.25, 1.0));
        assert_eq!(Color::WHITE.with_alpha(0.5).a, 0.5);
    }
}
