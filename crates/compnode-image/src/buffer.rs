//! CPU image buffer: linear-light RGBA `f32`, **premultiplied** alpha. This
//! is the reference representation the GPU path will be checked against.
use compnode_core::Color;
use compnode_core::color::{linear_to_srgb, srgb_to_linear};

#[derive(Clone, Debug, PartialEq)]
pub struct ImageBuf {
    pub width: u32,
    pub height: u32,
    pub data: Vec<[f32; 4]>,
}

impl ImageBuf {
    /// Fully transparent image.
    pub fn new(width: u32, height: u32) -> ImageBuf {
        ImageBuf {
            width,
            height,
            data: vec![[0.0; 4]; width as usize * height as usize],
        }
    }

    /// Uniform color (given as straight alpha, stored premultiplied).
    pub fn solid(width: u32, height: u32, c: Color) -> ImageBuf {
        let p = c.premultiplied();
        ImageBuf {
            width,
            height,
            data: vec![[p.r, p.g, p.b, p.a]; width as usize * height as usize],
        }
    }

    pub fn from_fn(width: u32, height: u32, f: impl Fn(u32, u32) -> [f32; 4]) -> ImageBuf {
        let mut data = Vec::with_capacity(width as usize * height as usize);
        for y in 0..height {
            for x in 0..width {
                data.push(f(x, y));
            }
        }
        ImageBuf {
            width,
            height,
            data,
        }
    }

    #[inline]
    fn idx(&self, x: u32, y: u32) -> usize {
        y as usize * self.width as usize + x as usize
    }
    pub fn get(&self, x: u32, y: u32) -> [f32; 4] {
        self.data[self.idx(x, y)]
    }
    pub fn set(&mut self, x: u32, y: u32, p: [f32; 4]) {
        let i = self.idx(x, y);
        self.data[i] = p;
    }
    /// Pixel with coordinates clamped to the image (edge extension).
    pub fn px_clamped(&self, x: i64, y: i64) -> [f32; 4] {
        let cx = x.clamp(0, i64::from(self.width) - 1) as u32;
        let cy = y.clamp(0, i64::from(self.height) - 1) as u32;
        self.get(cx, cy)
    }
    /// Pixel, or transparent outside the image.
    pub fn px_or_transparent(&self, x: i64, y: i64) -> [f32; 4] {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            [0.0; 4]
        } else {
            self.get(x as u32, y as u32)
        }
    }

    /// Bilinear sample at continuous coordinates where pixel `(i, j)` has its
    /// center at `(i + 0.5, j + 0.5)`. With `clamp` the edge is extended,
    /// otherwise outside taps are transparent.
    pub fn sample(&self, x: f32, y: f32, clamp: bool) -> [f32; 4] {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (ix, iy) = (x0 as i64, y0 as i64);
        let tap = |dx: i64, dy: i64| {
            if clamp {
                self.px_clamped(ix + dx, iy + dy)
            } else {
                self.px_or_transparent(ix + dx, iy + dy)
            }
        };
        let (p00, p10, p01, p11) = (tap(0, 0), tap(1, 0), tap(0, 1), tap(1, 1));
        let mut o = [0.0; 4];
        for c in 0..4 {
            let top = p00[c] + (p10[c] - p00[c]) * tx;
            let bot = p01[c] + (p11[c] - p01[c]) * tx;
            o[c] = top + (bot - top) * ty;
        }
        o
    }

    /// Memory used by the pixels.
    pub fn bytes(&self) -> usize {
        self.data.len() * std::mem::size_of::<[f32; 4]>()
    }

    /// Largest per-channel difference, or `None` if the sizes differ.
    pub fn max_abs_diff(&self, o: &ImageBuf) -> Option<f32> {
        if self.width != o.width || self.height != o.height {
            return None;
        }
        let mut m = 0.0_f32;
        for (a, b) in self.data.iter().zip(&o.data) {
            for c in 0..4 {
                m = m.max((a[c] - b[c]).abs());
            }
        }
        Some(m)
    }

    /// From straight-alpha sRGB8 RGBA bytes (`width * height * 4` of them).
    pub fn from_srgb8_rgba(width: u32, height: u32, bytes: &[u8]) -> Option<ImageBuf> {
        if bytes.len() != width as usize * height as usize * 4 {
            return None;
        }
        let data = bytes
            .chunks(4)
            .map(|p| {
                let a = f32::from(p[3]) / 255.0;
                let ch = |v: u8| srgb_to_linear(f32::from(v) / 255.0) * a;
                [ch(p[0]), ch(p[1]), ch(p[2]), a]
            })
            .collect();
        Some(ImageBuf {
            width,
            height,
            data,
        })
    }

    /// To straight-alpha sRGB8 RGBA bytes (clamped and rounded).
    pub fn to_srgb8_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len() * 4);
        for p in &self.data {
            let c = Color::new(p[0], p[1], p[2], p[3]).unpremultiplied();
            let enc = |l: f32| (linear_to_srgb(l.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
            out.extend_from_slice(&[
                enc(c.r),
                enc(c.g),
                enc(c.b),
                (p[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            ]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_is_premultiplied() {
        let b = ImageBuf::solid(2, 2, Color::new(1.0, 0.5, 0.0, 0.5));
        assert_eq!(b.get(1, 1), [0.5, 0.25, 0.0, 0.5]);
        assert_eq!(b.bytes(), 4 * 16);
    }

    #[test]
    fn srgb8_roundtrip_including_alpha() {
        let bytes: Vec<u8> = (0..16u8).map(|i| i * 16 + 7).collect();
        let img = ImageBuf::from_srgb8_rgba(2, 2, &bytes).unwrap();
        let back = img.to_srgb8_rgba();
        for (a, b) in bytes.iter().zip(&back) {
            assert!((i32::from(*a) - i32::from(*b)).abs() <= 1, "{a} vs {b}");
        }
        assert!(ImageBuf::from_srgb8_rgba(2, 2, &bytes[..5]).is_none());
    }

    #[test]
    fn bilinear_edges_and_centers() {
        let img = ImageBuf::from_fn(2, 1, |x, _| {
            if x == 0 {
                [0.0, 0.0, 0.0, 1.0]
            } else {
                [1.0, 1.0, 1.0, 1.0]
            }
        });
        assert_eq!(img.sample(0.5, 0.5, true)[0], 0.0);
        assert_eq!(img.sample(1.5, 0.5, true)[0], 1.0);
        assert!((img.sample(1.0, 0.5, true)[0] - 0.5).abs() < 1e-6);
        // far outside: clamped edge vs transparent
        assert_eq!(img.sample(-5.0, 0.5, true)[0], 0.0);
        assert_eq!(img.sample(-5.0, 0.5, false)[3], 0.0);
    }

    #[test]
    fn diff_requires_equal_sizes() {
        let a = ImageBuf::solid(2, 2, Color::WHITE);
        let mut b = a.clone();
        b.set(0, 0, [0.9, 1.0, 1.0, 1.0]);
        assert!((a.max_abs_diff(&b).unwrap() - 0.1).abs() < 1e-6);
        assert!(a.max_abs_diff(&ImageBuf::new(3, 2)).is_none());
    }
}
