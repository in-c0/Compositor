// `adjust_color_balance` (AdjustPixels.c): each channel shifted by overlapping shadow, midtone
// and highlight weights of its own value, then optionally scaled back to its old luma.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    preserve_luminosity: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
// Shadows, midtones, highlights: red, green, blue each.
@group(0) @binding(3) var<storage, read> shifts: array<f32, 9>;

// `tonal_weights`; dividing by ±0.25 is exact, so it's written as a product.
fn tonal_weights(v: f32) -> vec3<f32> {
    let b = 0.333;
    let below = keep(v - b);
    let above = keep(keep(v + b) - 1.0);
    let s = clamp(keep(below * -4.0) + 0.5, 0.0, 1.0);
    let h = clamp(keep(above * 4.0) + 0.5, 0.0, 1.0);
    let m1 = clamp(keep(below * 4.0) + 0.5, 0.0, 1.0);
    let m2 = clamp(keep(above * -4.0) + 0.5, 0.0, 1.0);
    return vec3<f32>(keep(s * 0.7), keep(keep(m1 * m2) * 0.7), keep(h * 0.7));
}

// `0.299f * c[0] + 0.587f * c[1] + 0.114f * c[2]`, contracted from the left.
fn luma(c: vec3<f32>) -> f32 {
    return mad(0.114, c.z, mad(0.299, c.x, keep(0.587 * c.y)));
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
    var c: vec3<f32>;
    for (var k = 0u; k < 3u; k++) {
        c[k] = div(min(255.0, div(keep(f32(p[k]) * 255.0), alpha)), 255.0);
    }
    let before = luma(c);
    for (var k = 0u; k < 3u; k++) {
        let w = tonal_weights(c[k]);
        let shift = mad(shifts[6u + k], w.z, mad(shifts[k], w.x, keep(shifts[3u + k] * w.y)));
        c[k] = min(1.0, max(0.0, keep(c[k] + shift)));
    }
    if (params.preserve_luminosity != 0u) {
        let after = luma(c);
        if (after > 0.0001) {
            let ratio = div(before, after);
            for (var k = 0u; k < 3u; k++) {
                c[k] = min(1.0, max(0.0, keep(c[k] * ratio)));
            }
        }
    }
    var out = p;
    for (var k = 0u; k < 3u; k++) {
        out[k] = u32(min(alpha, max(0.0, round_away(keep(c[k] * alpha)))));
    }
    dst[i] = pack(out);
}
