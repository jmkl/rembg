//! GPU evaluation with wgpu (feature `gpu`). The device and queue come from
//! the host (the egui app shares its own), nothing here opens a window.
//!
//! Every node kind is one or a few full-screen fragment passes writing an
//! `Rgba16Float` texture (linear light, premultiplied alpha, the same
//! convention as the CPU reference in [`crate::cpu`]). The CPU operations are
//! the oracles: [`compare_with_cpu`] renders each stock node both ways and
//! reports the largest difference.
//!
//! Written against wgpu 25 (the version eframe 0.32 uses). Calls that changed
//! between wgpu releases are isolated in [`GpuContext::wait_idle`] and the
//! `copy_texture_to_buffer` / `write_texture` calls.
use crate::buffer::ImageBuf;
use crate::cpu::{self, BlendMode};
use crate::eval::{Data, EvalCtx, EvalError};
use crate::loader::{ImageLoader, PpmLoader, downscale_to_fit};
use crate::pool::{TexKey, TexturePool};
use compnode_core::hash::FxMap;
use compnode_ui::document::{Change, Document};
use compnode_ui::value::{SocketType, Value};
use std::borrow::Cow;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wgpu::util::DeviceExt;

pub const WORK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const DISPLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

// ───────────────────────── half-float conversion ─────────────────────────

/// `f32` to IEEE half bits (round to nearest even; overflow becomes infinity).
pub fn f32_to_f16(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xFF) as i32;
    let man = b & 0x007F_FFFF;
    if exp == 0xFF {
        return sign | 0x7C00 | if man != 0 { 0x0200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 31 {
        return sign | 0x7C00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = man | 0x0080_0000;
        let shift = (14 - e) as u32;
        let mut h = (m >> shift) as u16;
        let rem = m & ((1 << shift) - 1);
        let half = 1 << (shift - 1);
        if rem > half || (rem == half && (h & 1) == 1) {
            h += 1;
        }
        return sign | h;
    }
    let mut h = sign | ((e as u16) << 10) | (man >> 13) as u16;
    let rem = man & 0x1FFF;
    if rem > 0x1000 || (rem == 0x1000 && (h & 1) == 1) {
        h += 1;
    }
    h
}

/// Half bits to `f32`.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = u32::from(h & 0x8000) << 16;
    let exp = u32::from((h >> 10) & 0x1F);
    let man = u32::from(h & 0x03FF);
    let bits = if exp == 0 {
        if man == 0 {
            sign
        } else {
            let mut e = 127 - 15 + 1;
            let mut m = man;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | ((e as u32) << 23) | ((m & 0x3FF) << 13)
        }
    } else if exp == 31 {
        sign | 0x7F80_0000 | (man << 13)
    } else {
        sign | ((exp + 127 - 15) << 23) | (man << 13)
    };
    f32::from_bits(bits)
}

// ───────────────────────────── shaders ─────────────────────────────

const PRELUDE: &str = r#"
struct U { p: array<vec4<f32>, 4>, size: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var t0: texture_2d<f32>;
@group(0) @binding(3) var t1: texture_2d<f32>;
@group(0) @binding(4) var t2: texture_2d<f32>;

struct VOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VOut {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var o: VOut;
    o.pos = vec4<f32>(p[i], 0.0, 1.0);
    o.uv = vec2<f32>((p[i].x + 1.0) * 0.5, (1.0 - p[i].y) * 0.5);
    return o;
}

fn unp(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) { return c.rgb / c.a; }
    return vec3<f32>(0.0);
}
fn ld(t: texture_2d<f32>, x: i32, y: i32) -> vec4<f32> {
    let d = vec2<i32>(textureDimensions(t));
    return textureLoad(t, clamp(vec2<i32>(x, y), vec2<i32>(0), d - vec2<i32>(1)), 0);
}
fn ld0(t: texture_2d<f32>, x: i32, y: i32) -> vec4<f32> {
    let d = vec2<i32>(textureDimensions(t));
    if (x < 0 || y < 0 || x >= d.x || y >= d.y) { return vec4<f32>(0.0); }
    return textureLoad(t, vec2<i32>(x, y), 0);
}
fn sampc(t: texture_2d<f32>, x: f32, y: f32) -> vec4<f32> {
    let fx = x - 0.5; let fy = y - 0.5;
    let x0 = floor(fx); let y0 = floor(fy);
    let tx = fx - x0; let ty = fy - y0;
    let ix = i32(x0); let iy = i32(y0);
    let top = mix(ld(t, ix, iy), ld(t, ix + 1, iy), tx);
    let bot = mix(ld(t, ix, iy + 1), ld(t, ix + 1, iy + 1), tx);
    return mix(top, bot, ty);
}
fn samp0(t: texture_2d<f32>, x: f32, y: f32) -> vec4<f32> {
    let fx = x - 0.5; let fy = y - 0.5;
    let x0 = floor(fx); let y0 = floor(fy);
    let tx = fx - x0; let ty = fy - y0;
    let ix = i32(x0); let iy = i32(y0);
    let top = mix(ld0(t, ix, iy), ld0(t, ix + 1, iy), tx);
    let bot = mix(ld0(t, ix, iy + 1), ld0(t, ix + 1, iy + 1), tx);
    return mix(top, bot, ty);
}
fn luma(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)); }
"#;

const SOLID: &str = r#"
@fragment fn fs(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> { return u.p[0]; }
"#;

const INVERT: &str = r#"
@fragment fn fs(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
    let c = ld(t0, i32(fp.x), i32(fp.y));
    if (c.a <= 0.0) { return c; }
    let col = unp(c);
    let r = col + (vec3<f32>(1.0) - 2.0 * col) * u.p[0].x;
    return vec4<f32>(r * c.a, c.a);
}
"#;

const BRIGHTNESS_CONTRAST: &str = r#"
@fragment fn fs(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
    let c = ld(t0, i32(fp.x), i32(fp.y));
    if (c.a <= 0.0) { return c; }
    let col = unp(c);
    let r = max((col - vec3<f32>(0.5)) * u.p[0].x + vec3<f32>(0.5) + vec3<f32>(u.p[0].y), vec3<f32>(0.0));
    return vec4<f32>(r * c.a, c.a);
}
"#;

const LEVELS: &str = r#"
@fragment fn fs(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
    let c = ld(t0, i32(fp.x), i32(fp.y));
    if (c.a <= 0.0) { return c; }
    let col = unp(c);
    let ib = u.p[0].x; let iw = u.p[0].y; let gamma = u.p[0].z; let ob = u.p[0].w; let ow = u.p[1].x;
    let range = max(iw - ib, 0.000001);
    let t = clamp((col - vec3<f32>(ib)) / range, vec3<f32>(0.0), vec3<f32>(1.0));
    let r = vec3<f32>(ob) + pow(t, vec3<f32>(1.0 / max(gamma, 0.001))) * (ow - ob);
    return vec4<f32>(r * c.a, c.a);
}
"#;

const HUE_SAT: &str = r#"
fn rgb2hsv(c: vec3<f32>) -> vec3<f32> {
    let mx = max(c.r, max(c.g, c.b));
    let mn = min(c.r, min(c.g, c.b));
    let d = mx - mn;
    var h = 0.0;
    if (d > 0.0) {
        if (mx == c.r) { h = (c.g - c.b) / d; if (h < 0.0) { h = h + 6.0; } }
        else if (mx == c.g) { h = (c.b - c.r) / d + 2.0; }
        else { h = (c.r - c.g) / d + 4.0; }
    }
    var s = 0.0;
    if (mx > 0.0) { s = d / mx; }
    return vec3<f32>(h / 6.0, s, mx);
}
fn hsv2rgb(h0: f32, s: f32, v: f32) -> vec3<f32> {
    let h = (h0 - floor(h0)) * 6.0;
    let i = i32(floor(h));
    let f = h - floor(h);
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    if (i == 0) { return vec3<f32>(v, t, p); }
    if (i == 1) { return vec3<f32>(q, v, p); }
    if (i == 2) { return vec3<f32>(p, v, t); }
    if (i == 3) { return vec3<f32>(p, q, v); }
    if (i == 4) { return vec3<f32>(t, p, v); }
    return vec3<f32>(v, p, q);
}
@fragment fn fs(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
    let c = ld(t0, i32(fp.x), i32(fp.y));
    if (c.a <= 0.0) { return c; }
    let hsv = rgb2hsv(unp(c));
    let r = hsv2rgb(hsv.x + u.p[0].x / 360.0, clamp(hsv.y * u.p[0].y, 0.0, 1.0), hsv.z * u.p[0].z);
    return vec4<f32>(r * c.a, c.a);
}
"#;

const THRESHOLD: &str = r#"
@fragment fn fs(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
    let c = ld(t0, i32(fp.x), i32(fp.y));
    if (c.a <= 0.0) { return c; }
    var v = 0.0;
    if (luma(unp(c)) >= u.p[0].x) { v = 1.0; }
    return vec4<f32>(v * c.a, v * c.a, v * c.a, c.a);
}
"#;

const CURVES: &str = r#"
fn curve(x: f32) -> f32 {
    var ys = array<f32, 5>(u.p[0].x, u.p[0].y, u.p[0].z, u.p[0].w, u.p[1].x);
    var ms = array<f32, 5>(u.p[1].y, u.p[1].z, u.p[1].w, u.p[2].x, u.p[2].y);
    let xc = clamp(x, 0.0, 1.0);
    if (xc <= 0.0) { return clamp(ys[0], 0.0, 1.0); }
    if (xc >= 1.0) { return clamp(ys[4], 0.0, 1.0); }
    let s = xc * 4.0;
    let i = i32(floor(s));
    let t = s - f32(i);
    let h = 0.25;
    let t2 = t * t; let t3 = t2 * t;
    let y = (2.0 * t3 - 3.0 * t2 + 1.0) * ys[i] + (t3 - 2.0 * t2 + t) * h * ms[i]
          + (-2.0 * t3 + 3.0 * t2) * ys[i + 1] + (t3 - t2) * h * ms[i + 1];
    return clamp(y, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
    let c = ld(t0, i32(fp.x), i32(fp.y));
    if (c.a <= 0.0) { return c; }
    let col = unp(c);
    return vec4<f32>(vec3<f32>(curve(col.r), curve(col.g), curve(col.b)) * c.a, c.a);
}
"#;

const BLUR: &str = r#"
@fragment fn fs(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
    let x = i32(fp.x); let y = i32(fp.y);
    let dx = i32(u.p[0].x); let dy = i32(u.p[0].y);
    let sigma = u.p[0].z; let r = i32(u.p[0].w);
    var acc = vec4<f32>(0.0);
    var ws = 0.0;
    for (var i: i32 = -r; i <= r; i = i + 1) {
        let w = exp(-f32(i * i) / (2.0 * sigma * sigma));
        acc = acc + w * ld(t0, x + dx * i, y + dy * i);
        ws = ws + w;
    }
    return acc / ws;
}
"#;

const SHARPEN: &str = r#"
@fragment fn fs(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
    let x = i32(fp.x); let y = i32(fp.y);
    let c = ld(t0, x, y);
    let b = ld(t1, x, y);
    let amt = u.p[0].x;
    let a = clamp(c.a + amt * (c.a - b.a), 0.0, 1.0);
    let rgb = clamp(c.rgb + amt * (c.rgb - b.rgb), vec3<f32>(0.0), vec3<f32>(a));
    return vec4<f32>(rgb, a);
}
"#;

const BLEND: &str = r#"
fn bf(mode: i32, b: f32, s: f32) -> f32 {
    if (mode == 1) { return b * s; }
    if (mode == 2) { return b + s - b * s; }
    if (mode == 3) {
        if (b <= 0.5) { return 2.0 * b * s; }
        return 1.0 - 2.0 * (1.0 - b) * (1.0 - s);
    }
    if (mode == 4) { return b + s; }
    if (mode == 5) { return abs(b - s); }
    if (mode == 6) { return min(b, s); }
    if (mode == 7) { return max(b, s); }
    return s;
}
@fragment fn fs(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
    let x = i32(fp.x); let y = i32(fp.y);
    let cb = ld0(t0, x, y);
    let cs = ld0(t1, x, y);
    var m = 1.0;
    if (u.p[0].z > 0.5) { m = luma(ld0(t2, x, y).rgb); }
    let k = u.p[0].y * m;
    let a_s = cs.a * k;
    let a_b = cb.a;
    let sb = unp(cb);
    let ss = unp(cs);
    let mode = i32(u.p[0].x);
    let r = cs.r * k * (1.0 - a_b) + cb.r * (1.0 - a_s) + a_s * a_b * bf(mode, sb.r, ss.r);
    let g = cs.g * k * (1.0 - a_b) + cb.g * (1.0 - a_s) + a_s * a_b * bf(mode, sb.g, ss.g);
    let b2 = cs.b * k * (1.0 - a_b) + cb.b * (1.0 - a_s) + a_s * a_b * bf(mode, sb.b, ss.b);
    return vec4<f32>(r, g, b2, a_s + a_b * (1.0 - a_s));
}
"#;

const RESIZE: &str = r#"
@fragment fn fs(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
    let x = i32(fp.x); let y = i32(fp.y);
    let sx = u.p[0].x; let sy = u.p[0].y;
    if (u.p[0].z > 0.5) {
        return sampc(t0, (f32(x) + 0.5) * sx, (f32(y) + 0.5) * sy);
    }
    let d = vec2<i32>(textureDimensions(t0));
    let x0 = f32(x) * sx; let x1 = f32(x + 1) * sx;
    let y0 = f32(y) * sy; let y1 = f32(y + 1) * sy;
    let ix0 = i32(floor(x0)); let ix1 = min(i32(ceil(x1)), d.x);
    let iy0 = i32(floor(y0)); let iy1 = min(i32(ceil(y1)), d.y);
    var acc = vec4<f32>(0.0);
    var ws = 0.0;
    for (var iy: i32 = iy0; iy < iy1; iy = iy + 1) {
        let wy = max(min(y1, f32(iy + 1)) - max(y0, f32(iy)), 0.0);
        for (var ix: i32 = ix0; ix < ix1; ix = ix + 1) {
            let wx = max(min(x1, f32(ix + 1)) - max(x0, f32(ix)), 0.0);
            let w = wx * wy;
            acc = acc + textureLoad(t0, vec2<i32>(ix, iy), 0) * w;
            ws = ws + w;
        }
    }
    if (ws > 0.0) { acc = acc / ws; }
    return acc;
}
"#;

const CROP: &str = r#"
@fragment fn fs(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
    return ld0(t0, i32(fp.x) + i32(u.p[0].x), i32(fp.y) + i32(u.p[0].y));
}
"#;

const ROTATE: &str = r#"
@fragment fn fs(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
    let s = u.p[0].x; let c = u.p[0].y; let cx = u.p[0].z; let cy = u.p[0].w;
    let dx = floor(fp.x) + 0.5 - cx; let dy = floor(fp.y) + 0.5 - cy;
    return samp0(t0, cx + c * dx + s * dy, cy - s * dx + c * dy);
}
"#;

const DISPLAY: &str = r#"
fn enc(x: f32) -> f32 {
    let v = clamp(x, 0.0, 1.0);
    if (v > 0.0031308) { return 1.055 * pow(v, 1.0 / 2.4) - 0.055; }
    return 12.92 * v;
}
@fragment fn fs(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
    let c = ld(t0, i32(fp.x), i32(fp.y));
    let col = unp(c);
    return vec4<f32>(enc(col.r) * c.a, enc(col.g) * c.a, enc(col.b) * c.a, c.a);
}
"#;

// ───────────────────────────── textures ─────────────────────────────

pub struct GpuImage {
    pub tex: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
}

impl GpuImage {
    fn key(&self) -> TexKey {
        key_for(self.width, self.height, self.format)
    }
    pub fn bytes(&self) -> u64 {
        self.key().bytes()
    }
}

fn key_for(width: u32, height: u32, format: wgpu::TextureFormat) -> TexKey {
    if format == DISPLAY_FORMAT {
        TexKey {
            width,
            height,
            format: 1,
            bytes_per_pixel: 4,
        }
    } else {
        TexKey {
            width,
            height,
            format: 0,
            bytes_per_pixel: 8,
        }
    }
}

fn create_image(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> GpuImage {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("compnode image"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    GpuImage {
        tex,
        view,
        width,
        height,
        format,
    }
}

// ───────────────────────────── context ─────────────────────────────

pub struct GpuContext {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    bgl: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    sampler: wgpu::Sampler,
    dummy: GpuImage,
    pipelines: FxMap<(&'static str, wgpu::TextureFormat), wgpu::RenderPipeline>,
    pool: TexturePool<GpuImage>,
}

impl GpuContext {
    /// `budget_bytes` limits the pooled (idle) textures; textures in use are never taken away.
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        budget_bytes: u64,
    ) -> GpuContext {
        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compnode bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex_entry(2),
                tex_entry(3),
                tex_entry(4),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("compnode pipeline layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("compnode sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let dummy = create_image(&device, 1, 1, WORK_FORMAT); // never written: transparent after creation
        let ctx = GpuContext {
            device,
            queue,
            bgl,
            layout,
            sampler,
            dummy,
            pipelines: FxMap::default(),
            pool: TexturePool::new(budget_bytes),
        };
        ctx.clear(&ctx.dummy);
        ctx
    }

    /// Fill `img` with transparent black.
    fn clear(&self, img: &GpuImage) {
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("clear"),
            });
        {
            let _pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &img.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        self.queue.submit(Some(enc.finish()));
    }

    /// Block until the GPU is idle (profiling, readback). The one place that
    /// depends on the wgpu poll API.
    pub fn wait_idle(&self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }

    pub fn pool_allocated(&self) -> u64 {
        self.pool.allocated()
    }
    pub fn pool_stats(&self) -> (u64, u64, u64) {
        (self.pool.hits(), self.pool.misses(), self.pool.evictions())
    }
    pub fn set_pool_budget(&mut self, bytes: u64) {
        self.pool.set_budget(bytes);
    }

    /// A texture of the given size from the pool (contents undefined).
    pub fn alloc(&mut self, width: u32, height: u32, format: wgpu::TextureFormat) -> GpuImage {
        let device = Arc::clone(&self.device);
        self.pool.acquire(key_for(width, height, format), || {
            create_image(&device, width, height, format)
        })
    }

    /// Give a texture back to the pool.
    pub fn release(&mut self, img: GpuImage) {
        self.pool.release(img.key(), img);
    }

    fn pipeline(
        &mut self,
        name: &'static str,
        body: &'static str,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        if !self.pipelines.contains_key(&(name, format)) {
            let src = format!("{PRELUDE}{body}");
            let module = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(name),
                    source: wgpu::ShaderSource::Wgsl(Cow::Owned(src)),
                });
            let pipeline = self
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(name),
                    layout: Some(&self.layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                });
            self.pipelines.insert((name, format), pipeline);
        }
        &self.pipelines[&(name, format)]
    }

    /// Run one fragment pass: up to three input textures, 16 float
    /// parameters, a fresh output texture of `out` size.
    pub fn pass(
        &mut self,
        name: &'static str,
        body: &'static str,
        format: wgpu::TextureFormat,
        inputs: &[&GpuImage],
        params: [f32; 16],
        out: (u32, u32),
    ) -> GpuImage {
        let target = self.alloc(out.0.max(1), out.1.max(1), format);
        let mut bytes: Vec<u8> = Vec::with_capacity(80);
        for v in params {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        for v in [out.0 as f32, out.1 as f32, 0.0, 0.0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let ubuf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("compnode params"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let view = |i: usize| inputs.get(i).map_or(&self.dummy.view, |t| &t.view);
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(name),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: ubuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(view(0)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(view(1)),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(view(2)),
                },
            ],
        });
        // build the pipeline first (needs &mut self), then record the pass
        self.pipeline(name, body, format);
        let pipeline = &self.pipelines[&(name, format)];
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(name) });
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(name),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(pipeline);
            rp.set_bind_group(0, &bind, &[]);
            rp.draw(0..3, 0..1);
        }
        self.queue.submit(Some(enc.finish()));
        target
    }

    /// Upload a CPU image (converted to half floats).
    pub fn upload(&mut self, img: &ImageBuf) -> GpuImage {
        let tex = self.alloc(img.width, img.height, WORK_FORMAT);
        let mut bytes = Vec::with_capacity(img.data.len() * 8);
        for p in &img.data {
            for c in p {
                bytes.extend_from_slice(&f32_to_f16(*c).to_le_bytes());
            }
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex.tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(img.width * 8),
                rows_per_image: Some(img.height),
            },
            wgpu::Extent3d {
                width: img.width,
                height: img.height,
                depth_or_array_layers: 1,
            },
        );
        tex
    }

    /// Read a work-format texture back into a CPU image.
    pub fn download(&self, img: &GpuImage) -> Result<ImageBuf, String> {
        if img.format != WORK_FORMAT {
            return Err("download expects an Rgba16Float image".to_string());
        }
        let row = img.width * 8;
        let padded = row.div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded) * u64::from(img.height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("readback"),
            });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &img.tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(img.height),
                },
            },
            wgpu::Extent3d {
                width: img.width,
                height: img.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(enc.finish()));
        let (tx, rx) = std::sync::mpsc::channel();
        buf.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.wait_idle();
        rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("{e:?}"))?;
        let mut data = Vec::with_capacity(img.width as usize * img.height as usize);
        {
            let mapped = buf
                .slice(..)
                .get_mapped_range()
                .map_err(|e| format!("{e:?}"))?;
            for y in 0..img.height as usize {
                let line = &mapped[y * padded as usize..y * padded as usize + row as usize];
                for px in line.chunks(8) {
                    let h = |i: usize| f16_to_f32(u16::from_le_bytes([px[i], px[i + 1]]));
                    data.push([h(0), h(2), h(4), h(6)]);
                }
            }
        }
        buf.unmap();
        Ok(ImageBuf {
            width: img.width,
            height: img.height,
            data,
        })
    }

    /// Convert a work image to display form (sRGB, premultiplied, `Rgba8Unorm`)
    /// for showing it as a texture in the UI.
    pub fn to_display(&mut self, img: &GpuImage) -> GpuImage {
        self.pass(
            "display",
            DISPLAY,
            DISPLAY_FORMAT,
            &[img],
            [0.0; 16],
            (img.width, img.height),
        )
    }
}

fn params(v: &[f32]) -> [f32; 16] {
    let mut p = [0.0_f32; 16];
    p[..v.len()].copy_from_slice(v);
    p
}

// ───────────────────────────── evaluator ─────────────────────────────

#[derive(Clone)]
enum GData {
    Image(Arc<GpuImage>),
    Val(Value),
}

/// Same contract as [`crate::Evaluator`], on the GPU: dirty propagation, an
/// LRU result cache with a memory budget, per-node errors and timings.
pub struct GpuEvaluator {
    pub ctx: GpuContext,
    cache: FxMap<u64, Vec<GData>>,
    dirty: FxMap<u64, ()>,
    errors: FxMap<u64, String>,
    files: FxMap<String, Arc<GpuImage>>,
    scratch_files: FxMap<String, Arc<ImageBuf>>,
    loader: Arc<dyn ImageLoader>,
    canvas: (u32, u32),
    budget: u64,
    tick: u64,
    used: FxMap<u64, u64>,
    /// Wall-clock time of each node's last evaluation (see `profile`).
    pub timings: FxMap<u64, Duration>,
    /// Wait for the GPU after every node so `timings` are real GPU times.
    pub profile: bool,
    pub last_evaluated: Vec<u64>,
}

impl GpuEvaluator {
    pub fn new(ctx: GpuContext, canvas: (u32, u32)) -> GpuEvaluator {
        GpuEvaluator {
            ctx,
            cache: FxMap::default(),
            dirty: FxMap::default(),
            errors: FxMap::default(),
            files: FxMap::default(),
            scratch_files: FxMap::default(),
            loader: Arc::new(PpmLoader),
            canvas: (canvas.0.max(1), canvas.1.max(1)),
            budget: u64::MAX,
            tick: 0,
            used: FxMap::default(),
            timings: FxMap::default(),
            profile: false,
            last_evaluated: Vec::new(),
        }
    }

    pub fn with_loader(mut self, loader: Arc<dyn ImageLoader>) -> GpuEvaluator {
        self.loader = loader;
        self
    }
    pub fn error(&self, uid: u64) -> Option<&str> {
        self.errors.get(&uid).map(String::as_str)
    }
    pub fn is_cached(&self, uid: u64) -> bool {
        self.cache.contains_key(&uid)
    }
    pub fn set_cache_budget(&mut self, bytes: u64) {
        self.budget = bytes;
    }
    pub fn cached_bytes(&self) -> u64 {
        self.cache
            .values()
            .flatten()
            .map(|d| {
                if let GData::Image(i) = d {
                    i.bytes()
                } else {
                    0
                }
            })
            .sum()
    }
    pub fn set_canvas(&mut self, canvas: (u32, u32)) {
        self.canvas = (canvas.0.max(1), canvas.1.max(1));
        let uids: Vec<u64> = self.cache.keys().copied().collect();
        for u in uids {
            self.drop_outputs(u);
        }
        self.files.clear();
    }

    fn drop_outputs(&mut self, uid: u64) {
        if let Some(outs) = self.cache.remove(&uid) {
            for d in outs {
                if let GData::Image(arc) = d
                    && let Ok(img) = Arc::try_unwrap(arc)
                {
                    self.ctx.release(img);
                }
            }
        }
    }

    /// Forget a loaded file (it changed, or finished loading) and recompute
    /// the Image File nodes that use it.
    pub fn invalidate_file(&mut self, doc: &Document, path: &str) {
        self.files.remove(path);
        let hit: Vec<u64> = doc
            .uids()
            .into_iter()
            .filter(|u| {
                doc.data(*u).is_some_and(|d| {
                    d.schema == "image.file"
                        && matches!(d.values.first(), Some(Value::String(s)) if s == path)
                })
            })
            .collect();
        for u in hit {
            self.mark_downstream(doc, u);
        }
    }

    pub fn mark_downstream(&mut self, doc: &Document, uid: u64) {
        let mut seen: FxMap<u64, ()> = FxMap::default();
        let mut stack = vec![uid];
        while let Some(u) = stack.pop() {
            if seen.insert(u, ()).is_some() {
                continue;
            }
            self.dirty.insert(u, ());
            let Some(node) = doc.id_of(u).and_then(|id| doc.graph.node(id)) else {
                continue;
            };
            for o in 0..node.outputs() {
                for &w in node.output_wires(o) {
                    if let Some(l) = doc.link_of_wire(w) {
                        stack.push(l.to.0);
                    }
                }
            }
        }
    }

    pub fn apply_changes(&mut self, doc: &Document, changes: &[Change]) {
        for c in changes {
            match c {
                Change::Value { uid, .. } | Change::Flag { uid } => self.mark_downstream(doc, *uid),
                Change::Structure { uids } => {
                    for &u in uids {
                        if doc.contains(u) {
                            self.mark_downstream(doc, u);
                        } else {
                            self.drop_outputs(u);
                            self.errors.remove(&u);
                            self.dirty.remove(&u);
                            self.used.remove(&u);
                        }
                    }
                }
            }
        }
    }

    fn needs_compute(&self, uid: u64) -> bool {
        self.dirty.contains_key(&uid) || !self.cache.contains_key(&uid)
    }

    fn sources(doc: &Document, uid: u64) -> Vec<u64> {
        let Some(node) = doc.id_of(uid).and_then(|id| doc.graph.node(id)) else {
            return Vec::new();
        };
        (0..node.inputs())
            .filter_map(|i| {
                node.input_wire(i)
                    .and_then(|w| doc.link_of_wire(w))
                    .map(|l| l.from.0)
            })
            .collect()
    }

    /// Evaluate `uid` and return its first output texture.
    pub fn evaluate(&mut self, doc: &Document, uid: u64) -> Result<Arc<GpuImage>, EvalError> {
        self.last_evaluated.clear();
        if !doc.contains(uid) {
            return Err(EvalError::NoSuchNode);
        }
        let mut ancestors: FxMap<u64, ()> = FxMap::default();
        let mut stack = vec![uid];
        while let Some(u) = stack.pop() {
            if ancestors.insert(u, ()).is_some() {
                continue;
            }
            stack.extend(Self::sources(doc, u));
        }
        let mut must: FxMap<u64, ()> = FxMap::default();
        let mut stack = vec![uid];
        while let Some(u) = stack.pop() {
            if must.contains_key(&u) || !self.needs_compute(u) {
                continue;
            }
            must.insert(u, ());
            stack.extend(Self::sources(doc, u));
        }
        for id in doc.graph.topo_order() {
            let Some(u) = doc.uid_of(id) else { continue };
            if must.contains_key(&u) {
                self.eval_node(doc, u);
            }
        }
        self.tick += 1;
        for &u in ancestors.keys() {
            self.used.insert(u, self.tick);
        }
        self.enforce_budget(&ancestors);
        if let Some(msg) = self.errors.get(&uid) {
            return Err(EvalError::Failed {
                uid,
                message: msg.clone(),
            });
        }
        match self.cache.get(&uid).and_then(|v| v.first()) {
            Some(GData::Image(img)) => Ok(Arc::clone(img)),
            _ => Err(EvalError::NoImage),
        }
    }

    fn enforce_budget(&mut self, keep: &FxMap<u64, ()>) {
        let mut total = self.cached_bytes();
        if total <= self.budget {
            return;
        }
        let mut victims: Vec<(u64, u64)> = self
            .cache
            .keys()
            .filter(|u| !keep.contains_key(u))
            .map(|u| (self.used.get(u).copied().unwrap_or(0), *u))
            .collect();
        victims.sort_unstable();
        for (_, u) in victims {
            if total <= self.budget {
                break;
            }
            let freed: u64 = self.cache.get(&u).map_or(0, |v| {
                v.iter()
                    .map(|d| {
                        if let GData::Image(i) = d {
                            i.bytes()
                        } else {
                            0
                        }
                    })
                    .sum()
            });
            self.drop_outputs(u);
            total = total.saturating_sub(freed);
        }
    }

    fn eval_node(&mut self, doc: &Document, uid: u64) {
        self.dirty.remove(&uid);
        self.last_evaluated.push(uid);
        let started = Instant::now();
        let Some(node) = doc.id_of(uid).and_then(|id| doc.graph.node(id)) else {
            return;
        };
        let Some(schema) = doc.registry.get(&node.data.schema) else {
            return;
        };
        let mut imgs: Vec<Option<Arc<GpuImage>>> = Vec::with_capacity(node.inputs());
        let mut vals: Vec<Option<Data>> = Vec::with_capacity(node.inputs());
        // Every image on each input, in connection order (multi-input sockets).
        let mut multi_imgs: Vec<Vec<Arc<GpuImage>>> = Vec::with_capacity(node.inputs());
        for i in 0..node.inputs() {
            let links: Vec<compnode_ui::document::Link> = node
                .input_wires(i)
                .iter()
                .filter_map(|&w| doc.link_of_wire(w))
                .collect();
            if links.is_empty() {
                imgs.push(None);
                vals.push(None);
                multi_imgs.push(Vec::new());
                continue;
            }
            let mut all: Vec<Arc<GpuImage>> = Vec::with_capacity(links.len());
            let mut first: Option<Arc<GpuImage>> = None;
            let mut first_val: Option<Data> = None;
            for link in &links {
                if self.errors.contains_key(&link.from.0) {
                    self.drop_outputs(uid);
                    self.errors.insert(uid, "input failed upstream".to_string());
                    return;
                }
                match self
                    .cache
                    .get(&link.from.0)
                    .and_then(|v| v.get(link.from.1))
                    .cloned()
                {
                    Some(GData::Image(img)) => {
                        if first.is_none() {
                            first = Some(Arc::clone(&img));
                        }
                        all.push(img);
                    }
                    Some(GData::Val(v)) => {
                        if first_val.is_none() {
                            first_val = Some(Data::Val(v));
                        }
                    }
                    None => {}
                }
            }
            imgs.push(first);
            vals.push(first_val);
            multi_imgs.push(all);
        }
        let kind = node.data.schema.clone();
        let muted = node.data.muted;
        let first_is_image = schema
            .outputs
            .first()
            .is_some_and(|o| o.ty == SocketType::Image);
        let image_input_ix = schema.inputs.iter().position(|d| d.ty == SocketType::Image);
        let values = node.data.values.clone();
        let result: Result<Vec<GData>, String> = if muted {
            match image_input_ix.and_then(|i| imgs[i].clone()) {
                Some(img) => Ok(vec![GData::Image(img)]),
                None if first_is_image => {
                    let t = self.transparent();
                    Ok(vec![GData::Image(Arc::new(t))])
                }
                None => Ok(Vec::new()),
            }
        } else {
            self.run_node(&kind, &imgs, &multi_imgs, &node.data.layers, vals, &values)
        };
        self.drop_outputs(uid);
        match result {
            Ok(out) => {
                self.errors.remove(&uid);
                self.cache.insert(uid, out);
            }
            Err(msg) => {
                self.errors.insert(uid, msg);
            }
        }
        if self.profile {
            self.ctx.wait_idle();
        }
        self.timings.insert(uid, started.elapsed());
    }

    fn transparent(&mut self) -> GpuImage {
        let (w, h) = self.canvas;
        self.ctx
            .pass("solid", SOLID, WORK_FORMAT, &[], [0.0; 16], (w, h))
    }

    fn img_or_transparent(&mut self, i: Option<&Arc<GpuImage>>) -> Arc<GpuImage> {
        match i {
            Some(a) => Arc::clone(a),
            None => Arc::new(self.transparent()),
        }
    }

    fn run_node(
        &mut self,
        kind: &str,
        imgs: &[Option<Arc<GpuImage>>],
        multi_imgs: &[Vec<Arc<GpuImage>>],
        layers: &[compnode_ui::document::Layer],
        vals: Vec<Option<Data>>,
        values: &[Value],
    ) -> Result<Vec<GData>, String> {
        let canvas = self.canvas;
        // parameter access shares the CPU evaluator's rules (connected values win)
        let loader = Arc::clone(&self.loader);
        let mut scratch = std::mem::take(&mut self.scratch_files);
        // A reroute forwards a value input verbatim (image inputs are handled
        // separately); grab it before `vals` is moved into the context.
        let passthrough_val = vals.iter().find_map(|v| match v {
            Some(Data::Val(val)) => Some(val.clone()),
            _ => None,
        });
        let p = EvalCtx::new(canvas, vals, values, &mut scratch, loader.as_ref());
        let f = |i: usize| p.float(i);
        let one = |this: &mut GpuEvaluator,
                   name: &'static str,
                   body: &'static str,
                   prm: &[f32],
                   idx: usize| {
            let src = this.img_or_transparent(imgs.get(idx).and_then(|o| o.as_ref()));
            let (w, h) = (src.width, src.height);
            GData::Image(Arc::new(this.ctx.pass(
                name,
                body,
                WORK_FORMAT,
                &[&*src],
                params(prm),
                (w, h),
            )))
        };
        let out: Result<Vec<GData>, String> = match kind {
            "image.file" => {
                let path = p.string(0);
                if path.trim().is_empty() {
                    Err("no file chosen".to_string())
                } else if let Some(img) = self.files.get(path.trim()) {
                    Ok(vec![GData::Image(Arc::clone(img))])
                } else {
                    match loader.load(path.trim()) {
                        Ok(cpu_img) => {
                            let cpu_img = downscale_to_fit(cpu_img, canvas.0, canvas.1);
                            let g = Arc::new(self.ctx.upload(&cpu_img));
                            self.files.insert(path.trim().to_string(), Arc::clone(&g));
                            Ok(vec![GData::Image(g)])
                        }
                        Err(e) => Err(e),
                    }
                }
            }
            "image.solid" => {
                let c = p.color(0).premultiplied();
                Ok(vec![GData::Image(Arc::new(self.ctx.pass(
                    "solid",
                    SOLID,
                    WORK_FORMAT,
                    &[],
                    params(&[c.r, c.g, c.b, c.a]),
                    canvas,
                )))])
            }
            "image.invert" => Ok(vec![one(
                self,
                "invert",
                INVERT,
                &[f(1).clamp(0.0, 1.0)],
                0,
            )]),
            "image.brightness_contrast" => {
                let contrast = f(2).clamp(-1.0, 1.0);
                let factor = if contrast >= 0.0 {
                    1.0 / (1.0 - contrast.min(0.99))
                } else {
                    1.0 + contrast
                };
                Ok(vec![one(
                    self,
                    "bc",
                    BRIGHTNESS_CONTRAST,
                    &[factor, f(1)],
                    0,
                )])
            }
            "image.levels" => Ok(vec![one(
                self,
                "levels",
                LEVELS,
                &[f(1), f(2), f(3), f(4), f(5)],
                0,
            )]),
            "image.hue_sat" => Ok(vec![one(self, "hue_sat", HUE_SAT, &[f(1), f(2), f(3)], 0)]),
            "image.threshold" => Ok(vec![one(self, "threshold", THRESHOLD, &[f(1)], 0)]),
            "image.curves" => {
                let pts: Vec<(f32, f32)> = (0..5).map(|i| (i as f32 / 4.0, f(i + 1))).collect();
                let (_, m) = cpu::curve_tangents(&pts);
                let ys: Vec<f32> = pts.iter().map(|p| p.1).collect();
                let mut prm = ys;
                prm.extend(m);
                Ok(vec![one(self, "curves", CURVES, &prm, 0)])
            }
            "image.blur" => {
                let sigma = f(1);
                let src = self.img_or_transparent(imgs.first().and_then(|o| o.as_ref()));
                if sigma <= 0.0 || !sigma.is_finite() {
                    Ok(vec![GData::Image(src)])
                } else {
                    Ok(vec![GData::Image(Arc::new(self.blur(&src, sigma)))])
                }
            }
            "image.sharpen" => {
                let src = self.img_or_transparent(imgs.first().and_then(|o| o.as_ref()));
                let radius = f(2);
                let blurred = if radius > 0.0 && radius.is_finite() {
                    Arc::new(self.blur(&src, radius))
                } else {
                    Arc::clone(&src)
                };
                let out = self.ctx.pass(
                    "sharpen",
                    SHARPEN,
                    WORK_FORMAT,
                    &[&*src, &*blurred],
                    params(&[f(1)]),
                    (src.width, src.height),
                );
                Ok(vec![GData::Image(Arc::new(out))])
            }
            "image.blend" => {
                let base = self.img_or_transparent(imgs.first().and_then(|o| o.as_ref()));
                let layer = self.img_or_transparent(imgs.get(1).and_then(|o| o.as_ref()));
                let mask = imgs.get(2).and_then(|o| o.clone());
                let mode = p.enum_index(3).min(BlendMode::ALL.len() - 1) as f32;
                let has_mask = if mask.is_some() { 1.0 } else { 0.0 };
                let mut inputs: Vec<&GpuImage> = vec![&*base, &*layer];
                if let Some(m) = &mask {
                    inputs.push(&**m);
                }
                let out = self.ctx.pass(
                    "blend",
                    BLEND,
                    WORK_FORMAT,
                    &inputs,
                    params(&[mode, f(4), has_mask]),
                    (base.width, base.height),
                );
                Ok(vec![GData::Image(Arc::new(out))])
            }
            "image.resize" => {
                let src = self.img_or_transparent(imgs.first().and_then(|o| o.as_ref()));
                let w = p.int(1).clamp(1, 8192) as u32;
                let h = p.int(2).clamp(1, 8192) as u32;
                if w == src.width && h == src.height {
                    Ok(vec![GData::Image(src)])
                } else {
                    let up = if w >= src.width && h >= src.height {
                        1.0
                    } else {
                        0.0
                    };
                    let (sx, sy) = (src.width as f32 / w as f32, src.height as f32 / h as f32);
                    let out = self.ctx.pass(
                        "resize",
                        RESIZE,
                        WORK_FORMAT,
                        &[&*src],
                        params(&[sx, sy, up]),
                        (w, h),
                    );
                    Ok(vec![GData::Image(Arc::new(out))])
                }
            }
            "image.crop" => {
                let src = self.img_or_transparent(imgs.first().and_then(|o| o.as_ref()));
                let (w, h) = (
                    p.int(3).clamp(1, 8192) as u32,
                    p.int(4).clamp(1, 8192) as u32,
                );
                let out = self.ctx.pass(
                    "crop",
                    CROP,
                    WORK_FORMAT,
                    &[&*src],
                    params(&[p.int(1) as f32, p.int(2) as f32]),
                    (w, h),
                );
                Ok(vec![GData::Image(Arc::new(out))])
            }
            "image.rotate" => {
                let src = self.img_or_transparent(imgs.first().and_then(|o| o.as_ref()));
                let (s, c) = f(1).to_radians().sin_cos();
                let prm = [s, c, src.width as f32 * 0.5, src.height as f32 * 0.5];
                let out = self.ctx.pass(
                    "rotate",
                    ROTATE,
                    WORK_FORMAT,
                    &[&*src],
                    params(&prm),
                    (src.width, src.height),
                );
                Ok(vec![GData::Image(Arc::new(out))])
            }
            "image.output" => {
                // The Output node is a layer canvas: composite every connected
                // input layer (bottom-to-top) with its transform. The GPU
                // compositor only handles axis-aligned source-over at full
                // resolution, so download the layers and composite on the CPU,
                // then upload the result. This node is terminal, so the cost is
                // acceptable and keeps the transform math identical to the CPU
                // reference.
                let input_layers = multi_imgs.first().map(Vec::as_slice).unwrap_or(&[]);
                if input_layers.is_empty() {
                    return Ok(vec![GData::Image(self.img_or_transparent(None))]);
                }
                let mut cpu_layers: Vec<(compnode_ui::document::Layer, ImageBuf)> = Vec::new();
                for (i, img) in input_layers.iter().enumerate() {
                    let buf = self.ctx.download(img)?;
                    let layer = layers
                        .get(i)
                        .copied()
                        .unwrap_or(compnode_ui::document::Layer::new(0));
                    cpu_layers.push((layer, buf));
                }
                let paints: Vec<crate::cpu::LayerPaint> = cpu_layers
                    .iter()
                    .map(|(l, buf)| crate::cpu::LayerPaint {
                        image: buf,
                        center: (l.center != [0.0, 0.0]).then_some(l.center),
                        size: (l.size[0] > 0.0 && l.size[1] > 0.0).then_some(l.size),
                        angle: l.angle,
                    })
                    .collect();
                let (cw, ch) = canvas;
                let composited = crate::cpu::composite_layers(&paints, cw, ch);
                Ok(vec![GData::Image(Arc::new(self.ctx.upload(&composited)))])
            }
            "util.float" => Ok(vec![GData::Val(Value::Float(f(0)))]),
            "util.color" => Ok(vec![GData::Val(Value::Color(p.color(0)))]),
            // Reroute forwards whatever flows into it, image or value, so a
            // parameter can reach a downstream socket through it.
            "core.reroute" => {
                if let Some(v) = passthrough_val {
                    Ok(vec![GData::Val(v)])
                } else {
                    Ok(vec![GData::Image(self.img_or_transparent(
                        imgs.first().and_then(|o| o.as_ref()),
                    ))])
                }
            }
            other => Err(format!("no GPU implementation for {other}")),
        };
        drop(p);
        self.scratch_files = scratch;
        out
    }

    /// Separable gaussian blur (same kernel as the CPU reference).
    fn blur(&mut self, src: &Arc<GpuImage>, sigma: f32) -> GpuImage {
        let radius = (3.0 * sigma).ceil().max(1.0);
        let size = (src.width, src.height);
        let h = self.ctx.pass(
            "blur",
            BLUR,
            WORK_FORMAT,
            &[&**src],
            params(&[1.0, 0.0, sigma, radius]),
            size,
        );
        let v = self.ctx.pass(
            "blur",
            BLUR,
            WORK_FORMAT,
            &[&h],
            params(&[0.0, 1.0, sigma, radius]),
            size,
        );
        self.ctx.release(h);
        v
    }
}

// ───────────────────────────── CPU comparison ─────────────────────────────

/// Result of comparing one node configuration.
#[derive(Clone, Debug)]
pub struct CompareReport {
    pub name: String,
    pub max_diff: f32,
    pub ok: bool,
    pub message: String,
}

fn test_image(w: u32, h: u32, phase: f32) -> ImageBuf {
    ImageBuf::from_fn(w, h, |x, y| {
        let (fx, fy) = (x as f32 / (w - 1) as f32, y as f32 / (h - 1) as f32);
        let a = 0.35 + 0.65 * ((fx * 3.1 + phase).sin() * 0.5 + 0.5);
        let c = [fx, fy, (fx * fy + phase).fract(), 1.0];
        [c[0] * a, c[1] * a, c[2] * a, a]
    })
}

/// Render every stock node on the GPU and on the CPU and compare. `tol` is
/// the allowed per-channel difference (half floats need about 5e-3).
pub fn compare_with_cpu(
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    tol: f32,
) -> Vec<CompareReport> {
    use crate::eval::Evaluator;
    use crate::loader::FnLoader;
    use compnode_core::geom::vec2;
    use compnode_ui::document::Link;

    let (w, h) = (64u32, 48u32);
    let loader: Arc<dyn ImageLoader> = Arc::new(FnLoader(move |p: &str| {
        Ok(test_image(w, h, if p.ends_with('b') { 1.7 } else { 0.3 }))
    }));
    let registry = crate::nodes::registry();
    let mut reports = Vec::new();

    type Case = (&'static str, &'static str, Vec<(usize, Value)>);
    let mut cases: Vec<Case> = vec![
        ("invert", "image.invert", vec![(1, Value::Float(0.7))]),
        (
            "brightness/contrast",
            "image.brightness_contrast",
            vec![(1, Value::Float(0.1)), (2, Value::Float(0.4))],
        ),
        (
            "levels",
            "image.levels",
            vec![
                (1, Value::Float(0.1)),
                (2, Value::Float(0.9)),
                (3, Value::Float(1.5)),
                (4, Value::Float(0.05)),
                (5, Value::Float(0.95)),
            ],
        ),
        (
            "hue/saturation",
            "image.hue_sat",
            vec![
                (1, Value::Float(40.0)),
                (2, Value::Float(1.2)),
                (3, Value::Float(0.9)),
            ],
        ),
        (
            "threshold",
            "image.threshold",
            vec![(1, Value::Float(0.45))],
        ),
        (
            "curves",
            "image.curves",
            vec![
                (1, Value::Float(0.05)),
                (2, Value::Float(0.35)),
                (3, Value::Float(0.6)),
                (4, Value::Float(0.8)),
                (5, Value::Float(0.95)),
            ],
        ),
        ("blur", "image.blur", vec![(1, Value::Float(2.5))]),
        (
            "sharpen",
            "image.sharpen",
            vec![(1, Value::Float(1.5)), (2, Value::Float(1.5))],
        ),
        (
            "resize down",
            "image.resize",
            vec![(1, Value::Int(23)), (2, Value::Int(17))],
        ),
        (
            "resize up",
            "image.resize",
            vec![(1, Value::Int(101)), (2, Value::Int(77))],
        ),
        (
            "crop",
            "image.crop",
            vec![
                (1, Value::Int(-5)),
                (2, Value::Int(7)),
                (3, Value::Int(50)),
                (4, Value::Int(30)),
            ],
        ),
        ("rotate", "image.rotate", vec![(1, Value::Float(30.0))]),
    ];
    for (i, m) in BlendMode::ALL.iter().enumerate() {
        let name: &'static str = Box::leak(format!("blend {}", m.name()).into_boxed_str());
        cases.push((
            name,
            "image.blend",
            vec![(3, Value::Enum(i)), (4, Value::Float(0.8))],
        ));
    }

    let ctx = GpuContext::new(device, queue, u64::MAX);
    let mut gpu = GpuEvaluator::new(ctx, (w, h)).with_loader(Arc::clone(&loader));
    let mut cpu_eval = Evaluator::new((w, h)).with_loader(Arc::clone(&loader));

    for (name, kind, set) in cases {
        let mut doc = Document::new(registry.clone());
        let pos = vec2(0.0, 0.0);
        let src = doc.add_node("image.file", pos).expect("file node");
        doc.set_value(src, 0, Value::String("mem://a".into()));
        let node = doc.add_node(kind, pos).expect("node");
        let out = doc.add_node("image.output", pos).expect("output");
        let link = |doc: &mut Document, a: u64, b: u64, bi: usize| {
            doc.connect(Link {
                from: (a, 0),
                to: (b, bi),
            })
        };
        let mut ok = link(&mut doc, src, node, 0).is_ok();
        if kind == "image.blend" {
            let layer = doc.add_node("image.file", pos).expect("layer node");
            doc.set_value(layer, 0, Value::String("mem://b".into()));
            ok &= link(&mut doc, layer, node, 1).is_ok();
            ok &= link(&mut doc, src, node, 2).is_ok(); // mask
        }
        for (i, v) in set {
            doc.set_value(node, i, v);
        }
        ok &= link(&mut doc, node, out, 0).is_ok();
        let changes = doc.take_changes();
        gpu.apply_changes(&doc, &changes);
        cpu_eval.apply_changes(&doc, &changes);
        let report = (|| -> Result<f32, String> {
            if !ok {
                return Err("could not build the test graph".to_string());
            }
            let cpu_img = cpu_eval
                .evaluate(&doc, out)
                .map_err(|e| format!("cpu: {e}"))?;
            let gpu_img = gpu.evaluate(&doc, out).map_err(|e| format!("gpu: {e}"))?;
            let back = gpu.ctx.download(&gpu_img)?;
            if kind == "image.threshold" {
                // a hard threshold flips pixels that sit within half-float rounding
                // of the limit: compare the fraction of flipped pixels instead
                let flipped = cpu_img
                    .data
                    .iter()
                    .zip(&back.data)
                    .filter(|(a, b)| (a[0] - b[0]).abs() > 0.5)
                    .count();
                let fraction = flipped as f32 / cpu_img.data.len().max(1) as f32;
                return Ok(if fraction <= 0.01 { 0.0 } else { 1.0 });
            }
            cpu_img.max_abs_diff(&back).ok_or_else(|| {
                format!(
                    "size differs: cpu {}x{}, gpu {}x{}",
                    cpu_img.width, cpu_img.height, back.width, back.height
                )
            })
        })();
        reports.push(match report {
            Ok(d) => CompareReport {
                name: name.to_string(),
                max_diff: d,
                ok: d <= tol,
                message: String::new(),
            },
            Err(m) => CompareReport {
                name: name.to_string(),
                max_diff: f32::NAN,
                ok: false,
                message: m,
            },
        });
    }
    reports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_conversion_exact_values_and_specials() {
        for v in [0.0_f32, 1.0, -1.0, 0.5, 2.0, 1024.0, -0.25, 65504.0] {
            assert_eq!(f16_to_f32(f32_to_f16(v)), v, "{v}");
        }
        assert_eq!(f32_to_f16(1.0), 0x3C00);
        assert_eq!(f32_to_f16(-2.0), 0xC000);
        assert_eq!(f32_to_f16(65504.0), 0x7BFF);
        assert_eq!(f32_to_f16(1.0e6), 0x7C00); // overflow -> infinity
        assert!(f16_to_f32(f32_to_f16(f32::NAN)).is_nan());
        assert_eq!(f16_to_f32(0x7C00), f32::INFINITY);
        assert_eq!(f32_to_f16(1.0e-9), 0); // underflow -> zero
        // the smallest subnormal and the smallest normal
        assert_eq!(f16_to_f32(1), 2.0_f32.powi(-24));
        assert_eq!(f16_to_f32(0x0400), 2.0_f32.powi(-14));
        assert_eq!(f32_to_f16(2.0_f32.powi(-24)), 1);
    }

    #[test]
    fn half_conversion_rounds_to_nearest_even_and_roundtrips_all_halves() {
        // halfway between 1.0 and the next half (1 + 2^-10) rounds to even (1.0)
        assert_eq!(f32_to_f16(1.0 + 2.0_f32.powi(-11)), 0x3C00);
        // just above halfway rounds up
        assert_eq!(
            f32_to_f16(1.0 + 2.0_f32.powi(-11) + 2.0_f32.powi(-20)),
            0x3C01
        );
        for h in 0..=u16::MAX {
            let f = f16_to_f32(h);
            if f.is_nan() {
                continue;
            }
            assert_eq!(f32_to_f16(f), h, "bits {h:#06x} -> {f}");
        }
    }

    #[test]
    fn working_precision_is_good_enough_for_the_tolerance() {
        // relative error of a half float is below 2^-11, far under the 5e-3 test tolerance
        for i in 0..1000 {
            let v = i as f32 / 999.0;
            let back = f16_to_f32(f32_to_f16(v));
            assert!((back - v).abs() <= v * 2.0_f32.powi(-10) + 1e-7);
        }
    }

    #[test]
    fn param_packing() {
        let p = params(&[1.0, 2.0]);
        assert_eq!(&p[..3], &[1.0, 2.0, 0.0]);
        assert_eq!(p.len(), 16);
    }

    #[test]
    fn every_shader_declares_the_entry_point_the_pipeline_expects() {
        for (name, body) in [
            ("solid", SOLID),
            ("invert", INVERT),
            ("bc", BRIGHTNESS_CONTRAST),
            ("levels", LEVELS),
            ("hue", HUE_SAT),
            ("threshold", THRESHOLD),
            ("curves", CURVES),
            ("blur", BLUR),
            ("sharpen", SHARPEN),
            ("blend", BLEND),
            ("resize", RESIZE),
            ("crop", CROP),
            ("rotate", ROTATE),
            ("display", DISPLAY),
        ] {
            assert!(body.contains("fn fs("), "{name}");
            assert!(
                !body.contains("fn vs("),
                "{name}: the vertex stage lives in the prelude"
            );
        }
        assert!(PRELUDE.contains("fn vs("));
    }
}
