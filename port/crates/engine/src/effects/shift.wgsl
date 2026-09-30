// `effects_shift`: the coverage moved by (dx, dy), read between pixels so a shadow moves smoothly
// rather than in whole steps. Metal's `mix(x, y, a)` is x + (y - x) * a, one multiply-add.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    dx: f32,
    dy: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> result: array<f32>;

fn lerp(x: f32, y: f32, a: f32) -> f32 {
    return mul_add(keep(y - x), a, x);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let w = params.width;
    let sx = keep(f32(id.x) - params.dx);
    let sy = keep(f32(id.y) - params.dy);
    var value = 0.0;
    if (sx >= 0.0 && sy >= 0.0 && sx <= f32(w - 1u) && sy <= f32(params.height - 1u)) {
        let x0 = u32(floor(sx));
        let y0 = u32(floor(sy));
        let x1 = min(x0 + 1u, w - 1u);
        let y1 = min(y0 + 1u, params.height - 1u);
        let fx = keep(sx - f32(x0));
        let fy = keep(sy - f32(y0));
        let top = lerp(source[y0 * w + x0], source[y0 * w + x1], fx);
        let bottom = lerp(source[y1 * w + x0], source[y1 * w + x1], fx);
        value = lerp(top, bottom, fy);
    }
    result[id.y * w + id.x] = value;
}
