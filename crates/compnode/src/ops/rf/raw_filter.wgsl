// Compute passes for the raw filter. One module, many entry points.
// All images are array<vec4<f32>> (RGBA, 0..1); `io` holds packed RGBA8.
// Binding 0 is a per-pass uniform; unused bindings are simply not referenced
// by an entry point (the host builds bind groups from auto-derived layouts).

struct Uni {
    size: vec4<u32>,   // x = width, y = height
    f0: vec4<f32>,
    f1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uni;
@group(0) @binding(1) var<storage, read_write> io: array<u32>;
@group(0) @binding(2) var<storage, read_write> img: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> t0: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> t1: array<vec4<f32>>;

// Keep these Rec. 709 coefficients aligned with ops::math::luma709.
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}
fn in_bounds(id: vec3<u32>) -> bool {
    return id.x < u.size.x && id.y < u.size.y;
}
fn idx(id: vec3<u32>) -> u32 {
    return id.y * u.size.x + id.x;
}
fn gauss(x: f32, sigma: f32) -> f32 {
    return exp(-(x * x) / (2.0 * sigma * sigma));
}
fn clamp01(c: vec3<f32>) -> vec3<f32> {
    return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
}

// ---- packing -------------------------------------------------------------

@compute @workgroup_size(8, 8, 1)
fn unpack(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id)) { return; }
    let i = idx(id);
    img[i] = unpack4x8unorm(io[i]);
}

@compute @workgroup_size(8, 8, 1)
fn pack(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id)) { return; }
    let i = idx(id);
    io[i] = pack4x8unorm(clamp(img[i], vec4<f32>(0.0), vec4<f32>(1.0)));
}

// ---- separable gaussian: img -> t0 -> t1 (sigma in f0.x) ------------------

@compute @workgroup_size(8, 8, 1)
fn blur_h(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id)) { return; }
    let sigma = max(u.f0.x, 0.1);
    let r = i32(ceil(sigma * 3.0));
    let w = i32(u.size.x);
    let row = i32(id.y) * w;
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var k = -r; k <= r; k++) {
        let x = clamp(i32(id.x) + k, 0, w - 1);
        let wt = gauss(f32(k), sigma);
        acc += img[u32(row + x)] * wt;
        wsum += wt;
    }
    t0[idx(id)] = acc / wsum;
}

@compute @workgroup_size(8, 8, 1)
fn blur_v(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id)) { return; }
    let sigma = max(u.f0.x, 0.1);
    let r = i32(ceil(sigma * 3.0));
    let w = i32(u.size.x);
    let h = i32(u.size.y);
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var k = -r; k <= r; k++) {
        let y = clamp(i32(id.y) + k, 0, h - 1);
        let wt = gauss(f32(k), sigma);
        acc += t0[u32(y * w + i32(id.x))] * wt;
        wsum += wt;
    }
    t1[idx(id)] = acc / wsum;
}

// ---- noise reduction: f0 = (amount, threshold) ----------------------------

@compute @workgroup_size(8, 8, 1)
fn nr_apply(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id)) { return; }
    let i = idx(id);
    let c = img[i];
    let b = t1[i];
    let diff = luma(c.rgb) - luma(b.rgb);
    let e = abs(diff) / u.f0.y;
    let blend = u.f0.x / (1.0 + 4.0 * e * e);
    img[i] = vec4<f32>(clamp01(c.rgb + (b.rgb - c.rgb) * blend), c.a);
}

// ---- white balance + dehaze: f0 = (gain.rgb, wb_on), f1.x = dehaze --------

fn dehaze_px(c: vec3<f32>, amount: f32) -> vec3<f32> {
    var rgb = c;
    if (amount > 0.0) {
        let black = 0.12 * amount;
        let contrast = 1.0 + 0.30 * amount;
        let saturation = 1.0 + 0.30 * amount;
        rgb = (rgb - vec3<f32>(black)) / max(1.0 - black, 1e-4);
        rgb = (rgb - vec3<f32>(0.5)) * contrast + vec3<f32>(0.5);
        let l = luma(rgb);
        rgb = vec3<f32>(l) + (rgb - vec3<f32>(l)) * saturation;
    } else if (amount < 0.0) {
        let haze = -amount;
        let haze_color = vec3<f32>(0.75, 0.80, 0.88);
        rgb = rgb * (1.0 - 0.5 * haze) + haze_color * 0.5 * haze;
        rgb = (rgb - vec3<f32>(0.5)) * (1.0 - 0.20 * haze) + vec3<f32>(0.5);
    }
    return clamp01(rgb);
}

@compute @workgroup_size(8, 8, 1)
fn color(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id)) { return; }
    let i = idx(id);
    let c = img[i];
    var rgb = c.rgb;
    if (u.f0.w > 0.5) {
        rgb = clamp01(rgb * u.f0.xyz);
    }
    img[i] = vec4<f32>(dehaze_px(rgb, u.f1.x), c.a);
}

// ---- local contrast: f0 = (amount, midtone_bias) --------------------------

@compute @workgroup_size(8, 8, 1)
fn local_contrast(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id)) { return; }
    let i = idx(id);
    let c = img[i];
    let l = luma(c.rgb);
    let detail = l - luma(t1[i].rgb);
    let midtones = 4.0 * l * (1.0 - l);
    let weight = 1.0 - u.f0.y + u.f0.y * midtones;
    let delta = detail * u.f0.x * weight;
    img[i] = vec4<f32>(clamp01(c.rgb + vec3<f32>(delta)), c.a);
}

// ---- sharpen: f0 = (strength, threshold) ----------------------------------

@compute @workgroup_size(8, 8, 1)
fn sharpen_apply(@builtin(global_invocation_id) id: vec3<u32>) {
    if (!in_bounds(id)) { return; }
    let i = idx(id);
    let c = img[i];
    let detail = luma(c.rgb) - luma(t1[i].rgb);
    let mask = min(abs(detail) / u.f0.y, 1.0);
    let delta = detail * u.f0.x * mask;
    img[i] = vec4<f32>(clamp01(c.rgb + vec3<f32>(delta)), c.a);
}
