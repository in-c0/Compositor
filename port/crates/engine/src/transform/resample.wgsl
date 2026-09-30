// Draws a transformed layer: every canvas pixel center is mapped back into the image, sampled with
// Core Graphics' filter for the draw, faded by the image rectangle's edge coverage, and composited
// over the canvas in the layer's blend mode, as `LayerRenderer.draw` has Core Graphics do it.

struct Params {
    canvas_width: u32,
    canvas_height: u32,
    image_width: u32,
    image_height: u32,
    mask_width: u32,
    mask_height: u32,
    mode: u32,
    // 1 when `mask` holds the layer's own mask (coverage per mask pixel), drawn through `mask_rect`.
    has_mask: u32,
    // 1 when `clip` holds coverage per canvas pixel.
    has_clip: u32,
    // 1 when `image` is already premultiplied, 0 for straight PNG pixels.
    premultiplied: u32,
    full_opacity: u32,
    // 0: interpolation .none; 1: .low.
    interpolation: u32,
    antialias: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
    // Canvas point to drawing space: q.x = a·x + b·y + c, q.y = d·x + e·y + f; `l1` is a canvas
    // pixel's width across any edge of the layer.
    inverse_x: vec4<f32>, // a, b, c, l1
    inverse_y: vec4<f32>, // d, e, f, unused
    rect: vec4<f32>,      // the image's rectangle in drawing space: min x, min y, max x, max y
    mask_rect: vec4<f32>,
    bounds: vec4<f32>,    // the image rectangle's bounding box on the canvas: min x, min y, max x, max y
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> canvas_in: array<u32>;
@group(0) @binding(2) var<storage, read> image: array<u32>;
@group(0) @binding(3) var<storage, read_write> canvas_out: array<u32>;
@group(0) @binding(4) var<storage, read> opacity_table: array<u32, 256>;
@group(0) @binding(5) var<storage, read> mask: array<u32>;
@group(0) @binding(6) var<storage, read> clip: array<u32>;

// Low's weights, in sixteenths, for a phase rounded to eighths.
const LOW_WEIGHTS = array<u32, 9>(0u, 1u, 2u, 4u, 8u, 12u, 14u, 15u, 16u);

fn premultiplied_pixel(i: u32) -> vec4<u32> {
    let p = unpack(image[i]);
    if params.premultiplied != 0u {
        return p;
    }
    return vec4<u32>(div255v(vec4<u32>(p.xyz * p.w, 0u)).xyz, p.w);
}

fn image_at(x: i32, y: i32) -> vec4<u32> {
    let cx = u32(clamp(x, 0, i32(params.image_width) - 1));
    let cy = u32(clamp(y, 0, i32(params.image_height) - 1));
    return premultiplied_pixel(cy * params.image_width + cx);
}

fn mask_at(x: i32, y: i32) -> u32 {
    let cx = u32(clamp(x, 0, i32(params.mask_width) - 1));
    let cy = u32(clamp(y, 0, i32(params.mask_height) - 1));
    return mask[cy * params.mask_width + cx];
}

// The point `q` (drawing space) in the pixels of a `size` image filling `rect`, y down.
fn to_pixels(q: vec2<f32>, rect: vec4<f32>, size: vec2<f32>) -> vec2<f32> {
    return vec2<f32>((q.x - rect.x) * size.x / (rect.z - rect.x), (rect.w - q.y) * size.y / (rect.w - rect.y));
}

// Low's filter: the phase past the pixel center to the upper left, rounded to eighths, and the
// weights for it.
fn low_taps(p: vec2<f32>) -> vec4<i32> {
    let s = p - vec2<f32>(0.5);
    let base = floor(s);
    let phase = vec2<u32>(floor((s - base) * 8.0 + vec2<f32>(0.5)));
    return vec4<i32>(vec2<i32>(base), i32(LOW_WEIGHTS[phase.x]), i32(LOW_WEIGHTS[phase.y]));
}

fn sample_image(p: vec2<f32>) -> vec4<u32> {
    if params.interpolation == 0u {
        let f = vec2<i32>(floor(p));
        return image_at(f.x, f.y);
    }
    let t = low_taps(p);
    let a = u32(t.z);
    let b = u32(t.w);
    let sum = image_at(t.x, t.y) * ((16u - a) * (16u - b)) + image_at(t.x + 1, t.y) * (a * (16u - b))
        + image_at(t.x, t.y + 1) * ((16u - a) * b) + image_at(t.x + 1, t.y + 1) * (a * b);
    return (sum + vec4<u32>(128u)) >> vec4<u32>(8u);
}

fn sample_mask(p: vec2<f32>) -> u32 {
    if params.interpolation == 0u {
        let f = vec2<i32>(floor(p));
        return mask_at(f.x, f.y);
    }
    let t = low_taps(p);
    let a = u32(t.z);
    let b = u32(t.w);
    let sum = mask_at(t.x, t.y) * ((16u - a) * (16u - b)) + mask_at(t.x + 1, t.y) * (a * (16u - b))
        + mask_at(t.x, t.y + 1) * ((16u - a) * b) + mask_at(t.x + 1, t.y + 1) * (a * b);
    return (sum + 128u) >> 8u;
}

// How much of the canvas pixel centered at `q` the rectangle covers, 0...1: with antialiasing,
// each edge fades over the pixel's width across it; without, any overlap counts in full.
fn edge_coverage(q: vec2<f32>, pixel: vec2<f32>, rect: vec4<f32>) -> f32 {
    let l1 = params.inverse_x.w;
    let inside = vec4<f32>(q.x - rect.x, q.y - rect.y, rect.z - q.x, rect.w - q.y);
    if params.antialias == 0u {
        let b = params.bounds;
        let overlaps = all(inside > vec4<f32>(-0.5 * l1)) && pixel.x + 1.0 > b.x && pixel.x < b.z && pixel.y + 1.0 > b.y && pixel.y < b.w;
        return select(0.0, 1.0, overlaps);
    }
    let c = clamp(vec4<f32>(0.5) + inside / l1, vec4<f32>(0.0), vec4<f32>(1.0));
    return c.x * c.y * c.z * c.w;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.canvas_width || id.y >= params.canvas_height {
        return;
    }
    let index = id.y * params.canvas_width + id.x;
    let backdrop = canvas_in[index];
    let pixel = vec2<f32>(f32(id.x), f32(id.y));
    let center = pixel + vec2<f32>(0.5);
    let q = vec2<f32>(
        params.inverse_x.x * center.x + params.inverse_x.y * center.y + params.inverse_x.z,
        params.inverse_y.x * center.x + params.inverse_y.y * center.y + params.inverse_y.z,
    );
    let covered = edge_coverage(q, pixel, params.rect);
    if covered <= 0.0 {
        canvas_out[index] = backdrop;
        return;
    }
    let p = to_pixels(q, params.rect, vec2<f32>(f32(params.image_width), f32(params.image_height)));
    var source = scale_premultiplied(sample_image(p));
    var coverage = u32(floor(covered * 255.0 + 0.5));
    if params.has_mask != 0u {
        let m = to_pixels(q, params.mask_rect, vec2<f32>(f32(params.mask_width), f32(params.mask_height)));
        coverage = div255(coverage * sample_mask(m));
    }
    if params.has_clip != 0u {
        coverage = div255(coverage * clip[index]);
    }
    if coverage != 255u {
        source = div255v(source * coverage);
    }
    canvas_out[index] = pack(composite(params.mode, unpack(backdrop), source, params.full_opacity != 0u));
}
