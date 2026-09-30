// `noise_add_at` (NoisePixels.c): uniform or Box–Muller Gaussian noise per pixel and channel,
// hashed from the pixel's document position so partial redraws agree.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    seed: u32,
    // The low 32 bits of the region's origin in the image's pixels.
    origin_x: u32,
    origin_y: u32,
    gaussian: u32,
    monochromatic: u32,
    // `amount / 100.0f * 127.5f`.
    spread: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;

fn noise_hash(v: u32) -> u32 {
    var x = v;
    x ^= x >> 16u;
    x *= 0x7feb352du;
    x ^= x >> 15u;
    x *= 0x846ca68bu;
    x ^= x >> 16u;
    return x;
}

// Uniform in [0, 1).
fn noise_unit(key: u32) -> f32 {
    return f32(noise_hash(key) >> 8u) * (1.0 / 16777216.0);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let i = id.y * params.width + id.x;
    let p = unpack(src[i]);
    if (p.w == 0u) {
        dst[i] = src[i];
        return;
    }
    let alpha = f32(p.w);
    let px = params.origin_x + id.x;
    let py = params.origin_y + id.y;
    let base = noise_hash(params.seed ^ noise_hash(px * 0x9e3779b9u ^ noise_hash(py * 0x85ebca6bu)));
    var out = p;
    for (var c = 0u; c < 3u; c++) {
        let key = select(base + c * 0x9e3779b9u, base, params.monochromatic != 0u);
        var n: f32;
        if (params.gaussian != 0u) {
            let u1 = noise_unit(key);
            let u2 = noise_unit(key ^ 0x68e31da4u);
            let radius = root(keep(-2.0 * log_rn(keep(1.0 - u1))));
            let angle = cos_rn(keep(6.2831853 * u2));
            n = keep(keep(keep(radius * angle) * params.spread) * (2.0 / 3.0));
        } else {
            n = keep(mul_add(noise_unit(key), 2.0, -1.0) * params.spread);
        }
        var value = keep(div(keep(f32(p[c]) * 255.0), alpha) + n);
        value = clamp(value, 0.0, 255.0);
        out[c] = u32(round_away(div(keep(value * alpha), 255.0)));
    }
    dst[i] = pack(out);
}
