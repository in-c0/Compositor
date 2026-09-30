// Draws a transformed layer as Core Graphics draws an image into a rotated or scaled rectangle
// (`LayerRenderer.draw`), measured from references:
//
// - Sample positions step across the canvas in 32.32 fixed point from the center of canvas pixel
//   (0, 0), the start and the steps rounded down from exact values. So a position that is exactly
//   on a pixel boundary stays there when the steps are exact binary fractions, and lands just short
//   of it otherwise.
// - Interpolation .none takes the image pixel under the position, clamped to the image.
// - Interpolation .low blends at most two pixels per axis, vertically first, then horizontally.
//   The nearer pixel is the "heavy" one; the other adds in with a shift: heavy − (heavy >> k) +
//   (light >> k), k = 4, 3, 2 or 1 as the distance from the heavy pixel's center reaches 1/16,
//   3/16, 5/16 or 7/16 of a pixel (none below 1/16). The image is clamped at its edges.
// - With antialiasing, each edge of the image rectangle fades linearly across the canvas pixel's
//   width measured along the edge's normal (|cos| + |sin|); the four edges multiply, and the
//   coverage is floor(product × 256), at most 255. In a pixel that an edge only partly covers, the
//   image isn't interpolated across that edge: that axis takes its heavy pixel alone.
// - Without antialiasing (Nearest), a pixel is drawn when the rectangle overlaps it at all.
// - A mask clipped through its own rectangle is sampled the same way. Its coverage combines with the
//   edges' as floor(floor(mask × mask edges / 255) × image edges / 255).

struct Dda {
    // Position at the center of canvas pixel (0, 0), and its steps per canvas pixel right and down,
    // in source pixels: 32.32 two's complement, low word first.
    u0: vec2<u32>,
    u_dx: vec2<u32>,
    u_dy: vec2<u32>,
    v0: vec2<u32>,
    v_dx: vec2<u32>,
    v_dy: vec2<u32>,
    // Canvas pixels per source pixel along u and v, for edge distances; the source size.
    scale: vec2<f32>,
    size: vec2<u32>,
}

struct Params {
    canvas_width: u32,
    canvas_height: u32,
    mode: u32,
    // 1 when `mask` holds the layer's own mask (coverage per mask pixel), drawn through `mask_dda`.
    has_mask: u32,
    // 1 when `clip` holds coverage per canvas pixel.
    has_clip: u32,
    // 1 when `image` is already premultiplied, 0 for straight PNG pixels.
    premultiplied: u32,
    full_opacity: u32,
    // 0: interpolation .none; 1: .low.
    interpolation: u32,
    antialias: u32,
    // A canvas pixel's width across any edge of the layer, |cos| + |sin|.
    l1: f32,
    pad0: u32,
    pad1: u32,
    // The image rectangle's bounding box on the canvas: min x, min y, max x, max y.
    bounds: vec4<f32>,
    image_dda: Dda,
    mask_dda: Dda,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> canvas_in: array<u32>;
@group(0) @binding(2) var<storage, read> image: array<u32>;
@group(0) @binding(3) var<storage, read_write> canvas_out: array<u32>;
@group(0) @binding(4) var<storage, read> opacity_table: array<u32, 256>;
@group(0) @binding(5) var<storage, read> mask: array<u32>;
@group(0) @binding(6) var<storage, read> clip: array<u32>;

// 64-bit two's complement arithmetic on (low, high) word pairs.
fn add64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let lo = a.x + b.x;
    return vec2<u32>(lo, a.y + b.y + select(0u, 1u, lo < a.x));
}

// `a` × `n` for 0 ≤ n < 65536.
fn mul64(a: vec2<u32>, n: u32) -> vec2<u32> {
    let p0 = (a.x & 0xffffu) * n;
    let p1 = (a.x >> 16u) * n;
    let lo = p0 + (p1 << 16u);
    let carry = select(0u, 1u, lo < p0);
    return vec2<u32>(lo, a.y * n + (p1 >> 16u) + carry);
}

fn position(start: vec2<u32>, dx: vec2<u32>, dy: vec2<u32>, x: u32, y: u32) -> vec2<u32> {
    return add64(add64(start, mul64(dx, x)), mul64(dy, y));
}

fn to_float(p: vec2<u32>) -> f32 {
    return f32(bitcast<i32>(p.y)) + f32(p.x >> 8u) / 16777216.0;
}

// One axis of a sample: the heavy pixel, the light one and its shift (0: heavy alone).
struct Taps {
    heavy: i32,
    light: i32,
    shift: u32,
}

fn nearest(p: vec2<u32>) -> Taps {
    return Taps(bitcast<i32>(p.y), 0, 0u);
}

fn low(p: vec2<u32>) -> Taps {
    // The position less half a pixel: which pixel centers it lies between.
    let s = add64(p, vec2<u32>(0x80000000u, 0xffffffffu));
    let i = bitcast<i32>(s.y);
    var t = Taps(i, i + 1, 0u);
    var d = s.x;
    if s.x >= 0x80000000u {
        t = Taps(i + 1, i, 0u);
        d = 0u - s.x;
    }
    // 1/16, 3/16, 5/16 and 7/16 of a pixel.
    let reach = u32(d >= 0x10000000u) + u32(d >= 0x30000000u) + u32(d >= 0x50000000u) + u32(d >= 0x70000000u);
    if reach > 0u {
        t.shift = 5u - reach;
    }
    return t;
}

fn taps(p: vec2<u32>, across_edge: bool) -> Taps {
    if params.interpolation == 0u {
        return nearest(p);
    }
    var t = low(p);
    if across_edge {
        t.shift = 0u;
    }
    return t;
}

fn blend_taps(heavy: vec4<u32>, light: vec4<u32>, k: u32) -> vec4<u32> {
    if k == 0u {
        return heavy;
    }
    let shift = vec4<u32>(k);
    return heavy - (heavy >> shift) + (light >> shift);
}

fn image_at(x: i32, y: i32) -> vec4<u32> {
    let size = params.image_dda.size;
    let cx = u32(clamp(x, 0, i32(size.x) - 1));
    let cy = u32(clamp(y, 0, i32(size.y) - 1));
    let p = unpack(image[cy * size.x + cx]);
    if params.premultiplied != 0u {
        return p;
    }
    return vec4<u32>(div255v(vec4<u32>(p.xyz * p.w, 0u)).xyz, p.w);
}

fn mask_at(x: i32, y: i32) -> vec4<u32> {
    let size = params.mask_dda.size;
    let cx = u32(clamp(x, 0, i32(size.x) - 1));
    let cy = u32(clamp(y, 0, i32(size.y) - 1));
    return vec4<u32>(mask[cy * size.x + cx]);
}

fn sample_image(tu: Taps, tv: Taps) -> vec4<u32> {
    let heavy = blend_taps(image_at(tu.heavy, tv.heavy), image_at(tu.heavy, tv.light), tv.shift);
    if tu.shift == 0u {
        return heavy;
    }
    let light = blend_taps(image_at(tu.light, tv.heavy), image_at(tu.light, tv.light), tv.shift);
    return blend_taps(heavy, light, tu.shift);
}

fn sample_mask(tu: Taps, tv: Taps) -> u32 {
    let heavy = blend_taps(mask_at(tu.heavy, tv.heavy), mask_at(tu.heavy, tv.light), tv.shift);
    if tu.shift == 0u {
        return heavy.x;
    }
    let light = blend_taps(mask_at(tu.light, tv.heavy), mask_at(tu.light, tv.light), tv.shift);
    return blend_taps(heavy, light, tu.shift).x;
}

// Distances in canvas pixels from the sample position to the rectangle's four edges (left, top,
// right, bottom in source terms), positive inside.
fn edge_distances(u: vec2<u32>, v: vec2<u32>, dda: Dda) -> vec4<f32> {
    // ~p is −p less 2^−32, which doesn't matter at this precision.
    let right = add64(vec2<u32>(0u, dda.size.x), ~u);
    let bottom = add64(vec2<u32>(0u, dda.size.y), ~v);
    return vec4<f32>(to_float(u) * dda.scale.x, to_float(v) * dda.scale.y, to_float(right) * dda.scale.x, to_float(bottom) * dda.scale.y);
}

// Each edge's fade, 0...1.
fn fades(distances: vec4<f32>) -> vec4<f32> {
    return clamp(vec4<f32>(0.5) + distances / params.l1, vec4<f32>(0.0), vec4<f32>(1.0));
}

fn coverage_byte(f: vec4<f32>) -> u32 {
    return min(255u, u32(floor(f.x * f.y * f.z * f.w * 256.0)));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.canvas_width || id.y >= params.canvas_height {
        return;
    }
    let index = id.y * params.canvas_width + id.x;
    let backdrop = canvas_in[index];
    let d = params.image_dda;
    let u = position(d.u0, d.u_dx, d.u_dy, id.x, id.y);
    let v = position(d.v0, d.v_dx, d.v_dy, id.x, id.y);
    let distances = edge_distances(u, v, d);
    var covered = 255u;
    var f = vec4<f32>(1.0);
    if params.antialias != 0u {
        f = fades(distances);
        covered = coverage_byte(f);
    } else {
        let b = params.bounds;
        let pixel = vec2<f32>(f32(id.x), f32(id.y));
        let overlaps = all(distances > vec4<f32>(-0.5 * params.l1)) && pixel.x + 1.0 > b.x && pixel.x < b.z && pixel.y + 1.0 > b.y && pixel.y < b.w;
        covered = select(0u, 255u, overlaps);
    }
    if covered == 0u {
        canvas_out[index] = backdrop;
        return;
    }
    let source = sample_image(taps(u, f.x < 1.0 || f.z < 1.0), taps(v, f.y < 1.0 || f.w < 1.0));
    var coverage = covered;
    if params.has_mask != 0u {
        let m = params.mask_dda;
        let mu = position(m.u0, m.u_dx, m.u_dy, id.x, id.y);
        let mv = position(m.v0, m.v_dx, m.v_dy, id.x, id.y);
        var mf = vec4<f32>(1.0);
        var mask_covered = 255u;
        if params.antialias != 0u {
            mf = fades(edge_distances(mu, mv, m));
            mask_covered = coverage_byte(mf);
        }
        let value = sample_mask(taps(mu, mf.x < 1.0 || mf.z < 1.0), taps(mv, mf.y < 1.0 || mf.w < 1.0));
        coverage = (value * mask_covered / 255u) * covered / 255u;
    }
    if params.has_clip != 0u {
        coverage = div255(coverage * clip[index]);
    }
    var pixel = scale_premultiplied(source);
    if coverage != 255u {
        pixel = div255v(pixel * coverage);
    }
    canvas_out[index] = pack(composite(params.mode, unpack(backdrop), pixel, params.full_opacity != 0u));
}
