// `levels_apply` (LevelsPixels.c): one 256-entry table per color channel, looked up on the
// unpremultiplied color with linear interpolation between entries. Levels, Curves and Exposure
// all end here.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
@group(0) @binding(3) var<storage, read> tables: array<f32>;

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
    var out = p;
    for (var c = 0u; c < 3u; c++) {
        let x = min(255.0, div(keep(f32(p[c]) * 255.0), alpha));
        let lo = u32(x);
        let hi = min(lo + 1u, 255u);
        let low = tables[c * 256u + lo];
        let result = mad(keep(tables[c * 256u + hi] - low), keep(x - f32(lo)), low);
        out[c] = u32(min(alpha, max(0.0, round_away(keep(result * alpha)))));
    }
    dst[i] = pack(out);
}
