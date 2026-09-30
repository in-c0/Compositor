// `adjust_black_white` (AdjustPixels.c): gray from the brightest channel's primary and the
// secondary between the two brightest, then optionally a tint whose C arithmetic is in double,
// reproduced here in double-single.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    // 1 when tinting with a positive saturation.
    tint: u32,
    // Which of the six hue sectors `fmod(tintHue, 360) / 60` falls in.
    sector: u32,
    // The tint's saturation (0…1) and `1 - |fmod(hp, 2) - 1|`, as doubles split in two floats.
    saturation_hi: f32,
    saturation_lo: f32,
    second_hi: f32,
    second_lo: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
// Red, yellow, green, cyan, blue, magenta.
@group(0) @binding(3) var<storage, read> weights: array<f32, 6>;

fn tinted(gray: f32) -> vec3<f32> {
    // c = (1 - |2·gray - 1|) · saturation; the sum is exact in double.
    var d = two_sum(keep(2.0 * gray), -1.0);
    if (d.x < 0.0) {
        d = -d;
    }
    let c = dd_mul(dd_add(vec2<f32>(1.0, 0.0), -d), vec2<f32>(params.saturation_hi, params.saturation_lo));
    let x = dd_mul(c, vec2<f32>(params.second_hi, params.second_lo));
    let m = dd_add(vec2<f32>(gray, 0.0), -c * 0.5);
    let zero = vec2<f32>(0.0);
    var r1 = zero;
    var g1 = zero;
    var b1 = zero;
    switch (params.sector) {
        case 0u: { r1 = c; g1 = x; }
        case 1u: { r1 = x; g1 = c; }
        case 2u: { g1 = c; b1 = x; }
        case 3u: { g1 = x; b1 = c; }
        case 4u: { r1 = x; b1 = c; }
        default: { r1 = c; b1 = x; }
    }
    let out = vec3<f32>(dd_add(r1, m).x, dd_add(g1, m).x, dd_add(b1, m).x);
    return clamp(out, vec3<f32>(0.0), vec3<f32>(1.0));
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
    let r = div(min(255.0, div(keep(f32(p.x) * 255.0), alpha)), 255.0);
    let g = div(min(255.0, div(keep(f32(p.y) * 255.0), alpha)), 255.0);
    let b = div(min(255.0, div(keep(f32(p.z) * 255.0), alpha)), 255.0);
    let mx = max(r, max(g, b));
    let mn = min(r, min(g, b));
    let md = keep(keep(keep(keep(r + g) + b) - mx) - mn);
    var primary: u32;
    var secondary: u32;
    if (mx == r) {
        primary = 0u;
        secondary = select(5u, 1u, g >= b);
    } else if (mx == g) {
        primary = 2u;
        secondary = select(3u, 1u, r >= b);
    } else {
        primary = 4u;
        secondary = select(5u, 3u, g >= r);
    }
    var gray = mad(keep(mx - md), weights[primary], mad(keep(md - mn), weights[secondary], mn));
    gray = min(1.0, max(0.0, gray));
    var out = vec3<f32>(gray);
    if (params.tint != 0u) {
        out = tinted(gray);
    }
    let bytes = vec3<u32>(min(vec3<f32>(alpha), max(vec3<f32>(0.0), vec3<f32>(
        round_away(keep(out.x * alpha)), round_away(keep(out.y * alpha)), round_away(keep(out.z * alpha))))));
    dst[i] = pack(vec4<u32>(bytes, p.w));
}
