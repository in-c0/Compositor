// `adjust_colored_vignette` (AdjustPixels.c) on a layer's own pixels: each pixel that is there
// moves toward the vignette color by strength × mask, eased off bright pixels by Highlights.
// The C works in `double`; this works in double-single (`double.wgsl`). The strength × mask per
// pixel comes from the host, which works it out in `f64`.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    pad: u32,
    // -(highlights / 100), then the color, each a double-single.
    highlights: vec2<f32>,
    red: vec2<f32>,
    green: vec2<f32>,
    blue: vec2<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
@group(0) @binding(3) var<storage, read> scaled_mask: array<vec2<f32>>;

fn channel(value: vec2<f32>, alpha: f32) -> u32 {
    // `write_premultiplied`: fmin(alpha, fmax(0, round(value × alpha))).
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
    let strength = scaled_mask[i];
    if (p.w == 0u || strength.x <= 0.0) {
        dst[i] = src[i];
        return;
    }
    let alpha = f32(p.w);
    let r = dd_min(DD_ONE, dd_div(dd(f32(p.x)), dd(alpha)));
    let g = dd_min(DD_ONE, dd_div(dd(f32(p.y)), dd(alpha)));
    let b = dd_min(DD_ONE, dd_div(dd(f32(p.z)), dd(alpha)));
    let c045 = vec2<f32>(0.44999998807907104, 1.1920929132713809e-08);
    let c055 = vec2<f32>(0.550000011920929, -1.1920929132713809e-08);
    let bright = dd_clamp01(dd_div(dd_sub(dd_rec709(r, g, b), c045), c055));
    // strength × mask × (1 − (highlights / 100) × bright), the last factor contracted to an fma.
    let effect = dd_mul(strength, dd_add(dd_mul(params.highlights, bright), DD_ONE));
    // r + (red − r) × effect, contracted.
    let nr = dd_add(dd_mul(dd_sub(params.red, r), effect), r);
    let ng = dd_add(dd_mul(dd_sub(params.green, g), effect), g);
    let nb = dd_add(dd_mul(dd_sub(params.blue, b), effect), b);
    dst[i] = pack(vec4<u32>(channel(nr, alpha), channel(ng, alpha), channel(nb, alpha), p.w));
}
