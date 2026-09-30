// `adjust_tonal_contrast` (AdjustPixels.c): each pixel's difference from its blurred base,
// through tanh, lifts or deepens it by an amount that depends on whether the base is a shadow, a
// midtone or a highlight. The C works in `double`; this works in double-single (`double.wgsl`).

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    pad: u32,
    // shadows, midtones, highlights, amount / 50, each a double-single.
    shadows: vec2<f32>,
    midtones: vec2<f32>,
    highlights: vec2<f32>,
    strength: vec2<f32>,
    // 0.5 − 0.15 and 0.85 − 0.5, as the C's doubles work them out.
    shadow_span: vec2<f32>,
    highlight_span: vec2<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
@group(0) @binding(3) var<storage, read> blurred: array<u32>;

const C015 = vec2<f32>(0.15000000596046448, -5.9604645663569045e-09);
const C05 = vec2<f32>(0.5, 0.0);
const C018 = vec2<f32>(0.18000000715255737, -7.1525572131747595e-09);

fn straight(p: vec4<u32>) -> array<vec2<f32>, 3> {
    let a = dd(f32(p.w));
    return array<vec2<f32>, 3>(
        dd_min(DD_ONE, dd_div(dd(f32(p.x)), a)),
        dd_min(DD_ONE, dd_div(dd(f32(p.y)), a)),
        dd_min(DD_ONE, dd_div(dd(f32(p.z)), a)));
}

// `tonal_smooth`: smoothstep from `low` over `span`.
fn tonal_smooth(low: vec2<f32>, span: vec2<f32>, value: vec2<f32>) -> vec2<f32> {
    let t = dd_clamp01(dd_div(dd_sub(value, low), span));
    // t × t × (3 − 2t), the last factor contracted to an fma.
    return dd_mul(dd_mul(t, t), dd_add(dd_mul(dd(-2.0), t), dd(3.0)));
}

fn channel(value: vec2<f32>, alpha: f32) -> u32 {
    let v = dd_round(dd_mul(value, dd(alpha)));
    return u32(clamp(v, 0.0, alpha));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let i = id.y * params.width + id.x;
    let p = unpack(src[i]);
    let base = unpack(blurred[i]);
    if (p.w == 0u || base.w == 0u) {
        dst[i] = src[i];
        return;
    }
    let c = straight(p);
    let b = straight(base);
    let lum = dd_rec709(c[0], c[1], c[2]);
    let base_lum = dd_rec709(b[0], b[1], b[2]);
    let shadow_weight = dd_sub(DD_ONE, tonal_smooth(C015, params.shadow_span, base_lum));
    let highlight_weight = tonal_smooth(C05, params.highlight_span, base_lum);
    let midtone_weight = dd_sub(dd_sub(DD_ONE, shadow_weight), highlight_weight);
    // (shadows × sw + midtones × mw + highlights × hw) / 100, contracted.
    let sum = dd_add(dd_mul(params.highlights, highlight_weight),
                     dd_add(dd_mul(params.shadows, shadow_weight), dd_mul(params.midtones, midtone_weight)));
    let weight = dd_div(sum, dd(100.0));
    let detail = dd_sub(lum, base_lum);
    // 0.18 × tanh(detail × 6) × weight × strength × (4 × lum × (1 − lum)), left to right.
    let curve = dd_mul(dd_mul(dd(4.0), lum), dd_sub(DD_ONE, lum));
    let delta = dd_mul(dd_mul(dd_mul(dd_mul(C018, dd_tanh(dd_mul(detail, dd(6.0)))), weight), params.strength), curve);
    let alpha = f32(p.w);
    dst[i] = pack(vec4<u32>(
        channel(dd_clamp01(dd_add(c[0], delta)), alpha),
        channel(dd_clamp01(dd_add(c[1], delta)), alpha),
        channel(dd_clamp01(dd_add(c[2], delta)), alpha),
        p.w));
}
