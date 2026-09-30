// An outline filled as Core Graphics fills a path: each pixel's coverage is the exact area of the
// outline's (stepped) edges inside it, `min(255, floor(area * 256))`, and the color is
// premultiplied by that coverage with truncation, `(color * coverage) / 255`.

struct Params {
    width: u32,
    height: u32,
    count: u32,
    color: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
// Directed edges [x0, y0, x1, y1] in pixels, y down, together closing the outline.
@group(0) @binding(1) var<storage, read> edges: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> out: array<u32>;

// The integral of clamp(u, 0, 1).
fn ramp(u: f32) -> f32 {
    if (u <= 0.0) {
        return 0.0;
    }
    if (u >= 1.0) {
        return u - 0.5;
    }
    return 0.5 * u * u;
}

// The edge p -> q's share of the area inside the pixel whose corner is `corner`: the integral of
// clamp(x - corner.x, 0, 1) dy along the edge, over the pixel's rows (Green's theorem).
fn edge_area(p: vec2<f32>, q: vec2<f32>, corner: vec2<f32>) -> f32 {
    if (p.y == q.y) {
        return 0.0;
    }
    let ya = clamp(p.y, corner.y, corner.y + 1.0);
    let yb = clamp(q.y, corner.y, corner.y + 1.0);
    if (ya == yb) {
        return 0.0;
    }
    let slope = (q.x - p.x) / (q.y - p.y);
    let ua = p.x + (ya - p.y) * slope - corner.x;
    let ub = p.x + (yb - p.y) * slope - corner.x;
    var mean: f32;
    if (abs(ub - ua) < 1e-6) {
        mean = clamp(0.5 * (ua + ub), 0.0, 1.0);
    } else {
        mean = (ramp(ub) - ramp(ua)) / (ub - ua);
    }
    return (yb - ya) * mean;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let corner = vec2<f32>(f32(id.x), f32(id.y));
    var area = 0.0;
    for (var i = 0u; i < params.count; i = i + 1u) {
        let e = edges[i];
        area = area + edge_area(e.xy, e.zw, corner);
    }
    let coverage = u32(clamp(floor(min(abs(area), 1.0) * 256.0), 0.0, 255.0));
    let color = unpack(params.color);
    let rgb = (color.xyz * coverage) / vec3<u32>(255u);
    out[id.y * params.width + id.x] = pack(vec4<u32>(rgb, coverage));
}
