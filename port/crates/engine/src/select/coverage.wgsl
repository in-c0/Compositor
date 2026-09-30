// A selection outline filled with the nonzero winding rule, one coverage byte per pixel (stored
// in the low byte of each word). Anti-aliased, a pixel's coverage is the area of the pixel inside
// the outline; aliased, a pixel is filled when the outline takes in any point of a fine grid.

struct Params {
    width: u32,
    height: u32,
    antialias: u32,
    pad: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
// Directed edges [x0, y0, x1, y1] in pixels, y down.
@group(0) @binding(1) var<storage, read> edges: array<vec4<f32>>;
// For each row, the range of `index` listing the edges that reach into it.
@group(0) @binding(2) var<storage, read> offsets: array<u32>;
@group(0) @binding(3) var<storage, read> index: array<u32>;
@group(0) @binding(4) var<storage, read_write> out: array<u32>;

// ∫ over the part of the edge between rows y0 and y0 + 1 of (clamp(x, x0, x0 + 1) - x0) dy,
// signed by the edge's direction: summed over every edge, the winding number integrated over the
// pixel [x0, x0 + 1] × [y0, y0 + 1].
fn edge_area(e: vec4<f32>, x0: f32, y0: f32) -> f32 {
    // Pixel-local coordinates.
    var a = vec2<f32>(e.x - x0, e.y - y0);
    var b = vec2<f32>(e.z - x0, e.w - y0);
    var dir = 1.0;
    if (a.y > b.y) {
        let t = a;
        a = b;
        b = t;
        dir = -1.0;
    }
    let top = max(a.y, 0.0);
    let bottom = min(b.y, 1.0);
    if (bottom <= top) {
        return 0.0;
    }
    let slope = (b.x - a.x) / (b.y - a.y);
    var xt = a.x + (top - a.y) * slope;
    var xb = a.x + (bottom - a.y) * slope;
    if (top == a.y) { xt = a.x; }
    if (bottom == b.y) { xb = b.x; }
    // Order the ends by x; the integrand only depends on x along the edge.
    let lo = min(xt, xb);
    let hi = max(xt, xb);
    let h = bottom - top;
    if (hi <= 0.0) {
        return 0.0;
    }
    if (lo >= 1.0) {
        return dir * h;
    }
    if (hi - lo < 1e-7) {
        return dir * h * clamp(lo, 0.0, 1.0);
    }
    // Split at x = 0 and x = 1: left of 0 adds nothing, right of 1 adds its height, and in between
    // the height times the mean x.
    let dy_dx = h / (hi - lo);
    let l = max(lo, 0.0);
    let r = min(hi, 1.0);
    var sum = (r - l) * dy_dx * (l + r) * 0.5;
    if (hi > 1.0) {
        sum += (hi - 1.0) * dy_dx;
    }
    return dir * sum;
}

// Without anti-aliasing, Core Graphics fills a pixel when the outline takes in any point of a
// 256 × 256 grid over it, the grid's first row and column on the pixel's top and left edges. On
// each grid row, a point is inside from a crossing that starts the winding up to (not including)
// the crossing that ends it.
fn touches_grid(first: u32, last: u32, px: f32, py: f32) -> bool {
    var xs: array<f32, 32>;
    var ds: array<i32, 32>;
    for (var j = 0u; j < 256u; j++) {
        let y = py + f32(j) / 256.0;
        var winding = 0;
        var n = 0u;
        for (var i = first; i < last; i++) {
            let e = edges[index[i]];
            let top = min(e.y, e.w);
            let bottom = max(e.y, e.w);
            if (!(y >= top && y < bottom)) {
                continue;
            }
            let x = e.x + (y - e.y) * (e.z - e.x) / (e.w - e.y);
            let d = select(-1, 1, e.w > e.y);
            if (x <= px) {
                winding += d;
            } else if (x < px + 1.0 && n < 32u) {
                // Kept sorted by x.
                var k = n;
                while (k > 0u && xs[k - 1u] > x) {
                    xs[k] = xs[k - 1u];
                    ds[k] = ds[k - 1u];
                    k--;
                }
                xs[k] = x;
                ds[k] = d;
                n++;
            }
        }
        if (winding != 0) {
            return true;
        }
        for (var k = 0u; k < n; k++) {
            winding += ds[k];
            if (winding == 0) {
                continue;
            }
            // The first grid point at or after this crossing, before the next one.
            let g = ceil((xs[k] - px) * 256.0);
            let end = select(px + 1.0, xs[k + 1u], k + 1u < n);
            if (g <= 255.0 && px + g / 256.0 < end) {
                return true;
            }
        }
    }
    return false;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let first = offsets[id.y];
    let last = offsets[id.y + 1u];
    var area = 0.0;
    for (var i = first; i < last; i++) {
        area += edge_area(edges[index[i]], f32(id.x), f32(id.y));
    }
    // Core Graphics keeps coverage in 256ths, truncated, with a whole pixel held at 255. The
    // allowance absorbs float error on coverages that are exact in 256ths.
    let level = min(u32(floor(min(abs(area), 1.0) * 256.0 + 1e-4)), 255u);
    var value = level;
    if (params.antialias == 0u) {
        // A pixel covering 4/256 or more always takes in a grid point; the grid only needs
        // checking for slivers.
        var hit = level >= 4u;
        if (!hit && abs(area) > 0.0) {
            hit = touches_grid(first, last, f32(id.x), f32(id.y));
        }
        value = select(0u, 255u, hit);
    }
    out[id.y * params.width + id.x] = value;
}
