// `adjust_grain` (AdjustPixels.c): two octaves of seeded value noise fixed in document space,
// added to the brightness, strongest in the midtones.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    seed: u32,
    fine_seed: u32,
    // `amount / 100 · 0.35 · 255` and `roughness / 100`, as the C rounds them.
    strength: f32,
    rough: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
// Per column, then per row: the main grain's cell and smoothed fraction, then the fine detail's.
@group(0) @binding(3) var<storage, read> columns: array<vec4<u32>>;
@group(0) @binding(4) var<storage, read> rows: array<vec4<u32>>;

fn mix32(v: u32) -> u32 {
    var x = v;
    x ^= x >> 16u;
    x *= 0x7feb352du;
    x ^= x >> 15u;
    x *= 0x846ca68bu;
    x ^= x >> 16u;
    return x;
}

fn lattice(ix: u32, iy: u32, seed: u32) -> f32 {
    let h = mix32(ix * 0x9E3779B1u ^ mix32(iy * 0x85EBCA77u ^ seed));
    return keep(keep(div(f32(h & 0xFFFFu), 65535.0) + div(f32(h >> 16u), 65535.0)) - 1.0);
}

fn grain_field(ix: u32, iy: u32, tx: f32, ty: f32, seed: u32) -> f32 {
    let n00 = lattice(ix, iy, seed);
    let n10 = lattice(ix + 1u, iy, seed);
    let n01 = lattice(ix, iy + 1u, seed);
    let n11 = lattice(ix + 1u, iy + 1u, seed);
    let top = mad(keep(n10 - n00), tx, n00);
    let bottom = mad(keep(n11 - n01), tx, n01);
    return keep(mad(keep(bottom - top), ty, top) * 1.6);
}

fn clamp255(v: f32) -> f32 {
    return select(select(v, 255.0, v > 255.0), 0.0, v < 0.0);
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
    let column = columns[id.x];
    let row = rows[id.y];
    let coarse = grain_field(column.x, row.x, bitcast<f32>(column.y), bitcast<f32>(row.y), params.seed);
    let fine = grain_field(column.z, row.z, bitcast<f32>(column.w), bitcast<f32>(row.w), params.fine_seed);
    let noise = mad(keep(fine - coarse), params.rough, coarse);
    let a = f32(p.w);
    let unpremultiply = select(div(255.0, a), 1.0, p.w == 255u);
    let r = keep(f32(p.x) * unpremultiply);
    let g = keep(f32(p.y) * unpremultiply);
    let b = keep(f32(p.z) * unpremultiply);
    let level = min(1.0, div(mad(0.0722, b, mad(0.2126, r, keep(0.7152 * g))), 255.0));
    let delta = keep(keep(noise * params.strength) * mad(keep(2.4 * level), keep(1.0 - level), 0.4));
    let coverage = div(a, 255.0);
    let out = vec3<u32>(
        u32(mad(clamp255(keep(r + delta)), coverage, 0.5)),
        u32(mad(clamp255(keep(g + delta)), coverage, 0.5)),
        u32(mad(clamp255(keep(b + delta)), coverage, 0.5)));
    dst[i] = pack(vec4<u32>(out, p.w));
}
