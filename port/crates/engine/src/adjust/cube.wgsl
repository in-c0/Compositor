// `cube_apply` (LevelsPixels.c): Hue/Saturation's color cube, trilinearly interpolated on the
// unpremultiplied color.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    dimension: u32,
    // `(dimension - 1) / 255.0f`.
    scale: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
@group(0) @binding(3) var<storage, read> cube: array<f32>;

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
    let n = params.dimension;
    var lo: vec3<u32>;
    var fraction: vec3<f32>;
    for (var c = 0u; c < 3u; c++) {
        let position = keep(min(255.0, div(keep(f32(p[c]) * 255.0), alpha)) * params.scale);
        lo[c] = min(u32(position), n - 2u);
        fraction[c] = keep(position - f32(lo[c]));
    }
    let base = (lo.x + lo.y * n + lo.z * n * n) * 4u;
    let sx = 4u;
    let sy = n * 4u;
    let sz = n * n * 4u;
    var out = p;
    for (var c = 0u; c < 3u; c++) {
        let b = base + c;
        let c000 = cube[b];
        let c010 = cube[b + sy];
        let c001 = cube[b + sz];
        let c011 = cube[b + sz + sy];
        let x00 = mad(keep(cube[b + sx] - c000), fraction.x, c000);
        let x10 = mad(keep(cube[b + sy + sx] - c010), fraction.x, c010);
        let x01 = mad(keep(cube[b + sz + sx] - c001), fraction.x, c001);
        let x11 = mad(keep(cube[b + sz + sy + sx] - c011), fraction.x, c011);
        let y0 = mad(keep(x10 - x00), fraction.y, x00);
        let y1 = mad(keep(x11 - x01), fraction.y, x01);
        let result = mad(keep(y1 - y0), fraction.z, y0);
        out[c] = u32(min(alpha, max(0.0, round_away(keep(result * alpha)))));
    }
    dst[i] = pack(out);
}
