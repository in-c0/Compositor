// A gradient drawn over a layer's pixels as Core Graphics' axial and radial shadings draw it.
//
// Fitted to references: the ramp is a table of `slots` colors, sampled at the slots' centers,
// where `slots` is 16 * (floor(ceil(length) / 16) + 1) - 2 for the line's length (a radial
// gradient's radius). A pixel takes slot ceil(t * slots) - 1, the start color where t <= 0 and
// the end color where t > 1. A linear gradient's t runs along a unit direction kept to 14
// significant bits; a radial one's t × slots is kept to 14 significant bits. Each premultiplied channel is then dithered to a byte,
// floor(value + (d + 0.5) / 256), with `d` from a 16 x 16 table: row by document row; column by
// document column along a row that changes color, halved for a radial gradient, and along a row
// that is one slot (or one end) all the way across by the count the Mac's span fill steps
// through (see `column`). The tool's opacity scales the bytes, rounded, and the
// result goes over the layer's pixels: s + (b * (255 - s.a) + 127) / 255.

struct Params {
    width: u32,
    height: u32,
    kind: u32,
    alpha: u32,
    // The canvas, the only part the gradient touches: x, y, width, height in the grid.
    region: vec4<i32>,
    // Document coordinates of the grid's top-left pixel.
    origin: vec2<i32>,
    slots: f32,
    // +infinity: `keep` rounds through it (see adjust/float.wgsl).
    guard: f32,
    // Each value below as a pair of floats, high and low, summing to it: t * slots at the grid's
    // top-left pixel center (x; linear), or the center in the grid (radial).
    base_hi: vec2<f32>,
    base_lo: vec2<f32>,
    // How t * slots changes per pixel right and down (linear); slots / radius first (radial).
    step_hi: vec2<f32>,
    step_lo: vec2<f32>,
    // Premultiplied colors at the start and the end, 0-255.
    from_color: vec4<f32>,
    to_color: vec4<f32>,
}

// `kind`: 0 for a linear gradient, 1 for a radial one.
const RADIAL: u32 = 1u;

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> dither: array<u32, 256>;
@group(0) @binding(2) var<storage, read> layer: array<u32>;
@group(0) @binding(3) var<storage, read_write> out: array<u32>;

// `v` (positive) truncated to 14 significant bits, as the Mac keeps a radial gradient's position.
fn coarse14(v: f32) -> f32 {
    if (v <= 0.0) {
        return v;
    }
    let scale = exp2(13.0 - floor(log2(v)));
    return trunc(v * scale) / scale;
}

fn keep(x: f32) -> f32 {
    return min(x, params.guard);
}

// a + b as a rounded sum and its exact error.
fn two_sum(a: f32, b: f32) -> vec2<f32> {
    let s = keep(a + b);
    let bb = keep(s - a);
    return vec2<f32>(s, keep(a - keep(s - bb)) + keep(b - bb));
}

// A value held as high + low floats, so a position just past a slot boundary stays past it on
// every GPU.
// The rounding error of a · b, whose rounded product is p, exactly (Dekker's split). `fma` would
// do it in one step, but WARP computes `fma` unfused.
fn product_error(a: f32, b: f32, p: f32) -> f32 {
    let ca = keep(4097.0 * a);
    let ah = keep(ca - keep(ca - a));
    let al = keep(a - ah);
    let cb = keep(4097.0 * b);
    let bh = keep(cb - keep(cb - b));
    let bl = keep(b - bh);
    return keep(keep(keep(keep(keep(ah * bh) - p) + keep(ah * bl)) + keep(al * bh)) + keep(al * bl));
}

fn position(p: vec2<i32>) -> vec2<f32> {
    if (params.kind == RADIAL) {
        let d = vec2<f32>(p) + vec2<f32>(0.5) - params.base_hi;
        return vec2<f32>(coarse14(sqrt(dot(d, d)) * params.step_hi.x), 0.0);
    }
    let x = f32(p.x);
    let y = f32(p.y);
    let px = keep(x * params.step_hi.x);
    let py = keep(y * params.step_hi.y);
    let ex = product_error(x, params.step_hi.x, px) + x * params.step_lo.x;
    let ey = product_error(y, params.step_hi.y, py) + y * params.step_lo.y;
    let a = two_sum(params.base_hi.x, px);
    let b = two_sum(a.x, py);
    return two_sum(b.x, keep(keep(keep(a.y + b.y) + keep(ex + ey)) + params.base_lo.x));
}

// Which color a position takes: a slot, or -1 and slots + 1 for the two ends.
fn key(u: vec2<f32>) -> i32 {
    if (u.x < 0.0 || (u.x == 0.0 && u.y <= 0.0)) {
        return -1;
    }
    if (u.x > params.slots || (u.x == params.slots && u.y > 0.0)) {
        return i32(params.slots) + 1;
    }
    // ceil(u) - 1, deciding a whole high part by the low one.
    var c = ceil(u.x);
    if (c == u.x && u.y > 0.0) {
        c += 1.0;
    }
    return i32(c) - 1;
}

// The dither table's column for document column x. `flat`: the whole row is one color.
fn column(x: i32, flat: bool) -> u32 {
    let c = u32(max(x, 0));
    if (flat) {
        return ((c >> 2u) + (c & 1u) + ((c >> 1u) & 1u)) & 15u;
    }
    if (params.kind == RADIAL) {
        return ((c + 1u) >> 1u) & 15u;
    }
    return c & 15u;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let index = id.y * params.width + id.x;
    let p = vec2<i32>(id.xy);
    let r = params.region;
    if (p.x < r.x || p.y < r.y || p.x >= r.x + r.z || p.y >= r.y + r.w) {
        out[index] = layer[index];
        return;
    }
    let u = position(p);
    // The row's two ends, and for a radial gradient the pixel nearest its center, bound the
    // colors along it.
    let first = key(position(vec2<i32>(r.x, p.y)));
    var flat = first == key(position(vec2<i32>(r.x + r.z - 1, p.y)));
    if (params.kind == RADIAL) {
        let nearest = clamp(i32(floor(params.base_hi.x)), r.x, r.x + r.z - 1);
        flat = flat && first == key(position(vec2<i32>(nearest, p.y)));
    }
    let slot = key(u);
    var value: vec4<f32>;
    if (slot < 0) {
        value = params.from_color;
    } else if (slot > i32(params.slots)) {
        value = params.to_color;
    } else {
        value = params.from_color + (params.to_color - params.from_color) * ((f32(slot) + 0.5) / params.slots);
    }
    let doc = params.origin + p;
    let d = (f32(dither[(u32(doc.y) & 15u) * 16u + column(doc.x, flat)]) + 0.5) / 256.0;
    var source = vec4<u32>(clamp(floor(value + vec4<f32>(d)), vec4<f32>(0.0), vec4<f32>(255.0)));
    if (params.alpha < 255u) {
        source = div255v(source * params.alpha);
    }
    let backdrop = unpack(layer[index]);
    out[index] = pack(source + div255v(backdrop * (255u - source.w)));
}
