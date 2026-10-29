struct Uniforms {
    size: vec4<u32>,
    f0: vec4<f32>,
    f1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read_write> pixels: array<u32>;

fn byte_at(word: u32, shift: u32) -> u32 {
    return (word >> shift) & 255u;
}

fn pack_rgba(r: u32, g: u32, b: u32, a: u32) -> u32 {
    return r | (g << 8u) | (b << 16u) | (a << 24u);
}

@group(0) @binding(2) var<storage, read> auxiliary: array<u32>;

@compute @workgroup_size(8, 8)
fn curves(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.size.x || id.y >= u.size.y {
        return;
    }
    let index = id.y * u.size.x + id.x;
    let pixel = pixels[index];
    let r = byte_at(pixel, 0u);
    let g = byte_at(pixel, 8u);
    let b = byte_at(pixel, 16u);
    let alpha = byte_at(pixel, 24u);
    let mapped_r = auxiliary[256u + auxiliary[r]];
    let mapped_g = auxiliary[512u + auxiliary[g]];
    let mapped_b = auxiliary[768u + auxiliary[b]];
    pixels[index] = pack_rgba(mapped_r, mapped_g, mapped_b, alpha);
}

fn luma709(rgb: vec3<f32>) -> f32 {
    return dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@compute @workgroup_size(8, 8)
fn color_balance(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.size.x || id.y >= u.size.y {
        return;
    }
    let index = id.y * u.size.x + id.x;
    let pixel = pixels[index];
    let original = vec3<f32>(
        f32(byte_at(pixel, 0u)),
        f32(byte_at(pixel, 8u)),
        f32(byte_at(pixel, 16u)),
    ) / 255.0;
    let lum = luma709(original);
    var weight = 4.0 * lum * (1.0 - lum);
    if u.f0.w < -0.5 {
        weight = (1.0 - lum) * (1.0 - lum);
    } else if u.f0.w > 0.5 {
        weight = lum * lum;
    }
    var adjusted = original + vec3<f32>(u.f0.x, u.f0.y, u.f0.z) * (0.5 * weight);
    if u.f1.x > 0.5 {
        let next_lum = luma709(adjusted);
        if next_lum > 0.0001 {
            adjusted *= lum / next_lum;
        }
    }
    let rgb = vec3<u32>(round(clamp(adjusted, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0));
    pixels[index] = pack_rgba(rgb.x, rgb.y, rgb.z, byte_at(pixel, 24u));
}

@compute @workgroup_size(8, 8)
fn composite_source_over(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.size.x || id.y >= u.size.y {
        return;
    }
    let px = (f32(id.x) + 0.5) - u.f0.x;
    let py = (f32(id.y) + 0.5) - u.f0.y;
    let local_x = px * u.f1.x + py * u.f1.y;
    let local_y = -px * u.f1.y + py * u.f1.x;
    let sx = (local_x + u.f0.z * 0.5) * (f32(u.size.z) / u.f0.z);
    let sy = (local_y + u.f0.w * 0.5) * (f32(u.size.w) / u.f0.w);
    if sx < 0.0 || sy < 0.0 || sx >= f32(u.size.z) || sy >= f32(u.size.w) {
        return;
    }

    let sample_x = clamp(sx, 0.0, f32(u.size.z - 1u));
    let sample_y = clamp(sy, 0.0, f32(u.size.w - 1u));
    let x0 = u32(floor(sample_x));
    let y0 = u32(floor(sample_y));
    let x1 = min(x0 + 1u, u.size.z - 1u);
    let y1 = min(y0 + 1u, u.size.w - 1u);
    let fx = sample_x - f32(x0);
    let fy = sample_y - f32(y0);
    let i00 = y0 * u.size.z + x0;
    let i10 = y0 * u.size.z + x1;
    let i01 = y1 * u.size.z + x0;
    let i11 = y1 * u.size.z + x1;
    let p00 = auxiliary[i00];
    let p10 = auxiliary[i10];
    let p01 = auxiliary[i01];
    let p11 = auxiliary[i11];
    var sampled: vec4<f32>;
    for (var channel = 0u; channel < 4u; channel += 1u) {
        let shift = channel * 8u;
        let top = mix(f32(byte_at(p00, shift)), f32(byte_at(p10, shift)), fx);
        let bottom = mix(f32(byte_at(p01, shift)), f32(byte_at(p11, shift)), fx);
        sampled[channel] = mix(top, bottom, fy);
    }
    let source = vec4<f32>(vec4<u32>(round(sampled))) / 255.0;

    let index = id.y * u.size.x + id.x;
    let base_word = pixels[index];
    let base = vec4<f32>(
        f32(byte_at(base_word, 0u)),
        f32(byte_at(base_word, 8u)),
        f32(byte_at(base_word, 16u)),
        f32(byte_at(base_word, 24u)),
    ) / 255.0;
    let alpha_s = source.a * u.f1.z;
    if alpha_s <= 0.0 {
        return;
    }
    let alpha_o = alpha_s + base.a * (1.0 - alpha_s);
    if alpha_o <= 0.0 {
        pixels[index] = 0u;
        return;
    }
    let rgb = (source.rgb * alpha_s + base.rgb * base.a * (1.0 - alpha_s)) / alpha_o;
    let out = vec4<u32>(round(clamp(vec4<f32>(rgb, alpha_o), vec4<f32>(0.0), vec4<f32>(1.0)) * 255.0));
    pixels[index] = pack_rgba(out.r, out.g, out.b, out.a);
}
