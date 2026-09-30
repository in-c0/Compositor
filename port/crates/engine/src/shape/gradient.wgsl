// A gradient drawn over a layer's pixels as Core Graphics' axial and radial shadings draw it.
//
// Fitted to references: the ramp is a table of `slots` colors, sampled at the slots' centers,
// where `slots` is 16 * (floor(ceil(length) / 16) + 1) - 2 for the line's length (a radial
// gradient's radius). A pixel takes slot ceil(t * slots) - 1, the start color where t <= 0 and
// the end color where t > 1. Each premultiplied channel is then dithered to a byte,
// floor(value + (d + 0.5) / 256), with `d` from a 16 x 16 table: row by document row, column by
// document column for a slanted or level line, and by the counts the Mac's upright-line and
// radial loops step through (see `column`). The tool's opacity scales the bytes, rounded, and the
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
    // t * slots at the grid's top-left pixel center (axial), or the center (radial).
    base: vec2<f32>,
    // How t * slots changes per pixel right and down (axial); slots / radius (radial).
    step: vec2<f32>,
    slots: f32,
    _pad: f32,
    // Premultiplied colors at the start and the end, 0-255.
    from_color: vec4<f32>,
    to_color: vec4<f32>,
}

const AXIAL: u32 = 0u;
const UPRIGHT: u32 = 1u;
const RADIAL: u32 = 2u;

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> dither: array<u32, 256>;
@group(0) @binding(2) var<storage, read> layer: array<u32>;
@group(0) @binding(3) var<storage, read_write> out: array<u32>;

// The dither table's column for document column x.
fn column(x: i32) -> u32 {
    let c = u32(max(x, 0));
    switch params.kind {
        case 1u: { return ((c >> 2u) + ((c & 1u) + ((c >> 1u) & 1u))) & 15u; }
        case 2u: { return ((c + 1u) >> 1u) & 15u; }
        default: { return c & 15u; }
    }
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
    var u: f32;
    if (params.kind == RADIAL) {
        let d = vec2<f32>(p) + vec2<f32>(0.5) - params.base;
        u = sqrt(dot(d, d)) * params.step.x;
    } else {
        u = params.base.x + f32(p.x) * params.step.x + f32(p.y) * params.step.y;
    }
    var value: vec4<f32>;
    if (u <= 0.0) {
        value = params.from_color;
    } else if (u > params.slots) {
        value = params.to_color;
    } else {
        let slot = ceil(u) - 1.0;
        value = params.from_color + (params.to_color - params.from_color) * ((slot + 0.5) / params.slots);
    }
    let doc = params.origin + p;
    let d = (f32(dither[(u32(doc.y) & 15u) * 16u + column(doc.x)]) + 0.5) / 256.0;
    var source = vec4<u32>(clamp(floor(value + vec4<f32>(d)), vec4<f32>(0.0), vec4<f32>(255.0)));
    if (params.alpha < 255u) {
        source = div255v(source * params.alpha);
    }
    let backdrop = unpack(layer[index]);
    out[index] = pack(source + div255v(backdrop * (255u - source.w)));
}
