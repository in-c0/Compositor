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
//   (light >> k): the phase past the first pixel's center, rounded half up to eighths, is 1 to 3
//   eighths from the heavy pixel for k = 4 to 2, and a half for k = 1 (none at 0). The heavy pixel
//   is the first below a phase of one half. The image is clamped at its edges.
// - With antialiasing, each edge of the image rectangle fades linearly across the canvas pixel's
//   width measured along the edge's normal (|cos| + |sin|); the four edges multiply, and the
//   coverage is ceil(product × 256) − 1, at most 255. In a pixel that an edge only partly covers, the
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
    // 1 when the layer isn't rotated, so `tables` holds its edges.
    upright: u32,
    pad0: u32,
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
@group(0) @binding(7) var<storage, read> tables: array<vec4<u32>>;

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
    // The position less half a pixel: which pixel centers it lies between, and the phase past the
    // first, rounded half up to eighths.
    let s = add64(p, vec2<u32>(0x80000000u, 0xffffffffu));
    let i = bitcast<i32>(s.y);
    let eighths = ((s.x >> 28u) + 1u) >> 1u;
    var t = Taps(i, i + 1, 0u);
    var reach = eighths;
    if s.x >= 0x80000000u {
        t = Taps(i + 1, i, 0u);
        reach = 8u - eighths;
    }
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

// The coverage byte for a product of fades: ceil(p × 256) − 1, so an exact multiple of 1/256
// counts one less.
fn coverage_byte(p: f32) -> u32 {
    return u32(clamp(ceil(p * 256.0) - 1.0, 0.0, 255.0));
}

// How a rectangle covers a canvas pixel: each edge's fade (left, top, right, bottom in source
// terms) and the coverage byte, 0 when the pixel isn't drawn.
struct Edges {
    fade: vec4<f32>,
    covered: u32,
}

// `tables` holds, for an upright layer, each canvas column's left and right fades and each row's top
// and bottom fades with their coverage bytes, worked out in double precision: image columns, image
// rows, then mask columns and mask rows.
fn edges(u: vec2<u32>, v: vec2<u32>, dda: Dda, table: u32, x: u32, y: u32) -> Edges {
    var e = Edges(vec4<f32>(1.0), 255u);
    if params.upright != 0u {
        let col = tables[table + x];
        let row = tables[table + params.canvas_width + y];
        e.fade = vec4<f32>(bitcast<f32>(col.x), bitcast<f32>(row.x), bitcast<f32>(col.y), bitcast<f32>(row.y));
        if params.antialias == 0u {
            e.covered = select(0u, 255u, all(e.fade > vec4<f32>(0.0)));
            return e;
        }
        let across = e.fade.x * e.fade.z;
        let down = e.fade.y * e.fade.w;
        if down >= 1.0 {
            e.covered = col.z;
        } else if across >= 1.0 {
            e.covered = row.z;
        } else {
            e.covered = coverage_byte(across * down);
        }
        return e;
    }
    let distances = edge_distances(u, v, dda);
    if params.antialias == 0u {
        let b = params.bounds;
        let pixel = vec2<f32>(f32(x), f32(y));
        let overlaps = all(distances > vec4<f32>(-0.5 * params.l1)) && pixel.x + 1.0 > b.x && pixel.x < b.z && pixel.y + 1.0 > b.y && pixel.y < b.w;
        e.covered = select(0u, 255u, overlaps);
        return e;
    }
    e.fade = clamp(vec4<f32>(0.5) + distances / params.l1, vec4<f32>(0.0), vec4<f32>(1.0));
    e.covered = coverage_byte(e.fade.x * e.fade.y * e.fade.z * e.fade.w);
    return e;
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
    let e = edges(u, v, d, 0u, id.x, id.y);
    if e.covered == 0u {
        canvas_out[index] = backdrop;
        return;
    }
    let f = e.fade;
    let source = sample_image(taps(u, f.x < 1.0 || f.z < 1.0), taps(v, f.y < 1.0 || f.w < 1.0));
    let covered = e.covered;
    var coverage = covered;
    if params.has_mask != 0u {
        let m = params.mask_dda;
        let mu = position(m.u0, m.u_dx, m.u_dy, id.x, id.y);
        let mv = position(m.v0, m.v_dx, m.v_dy, id.x, id.y);
        let me = edges(mu, mv, m, params.canvas_width + params.canvas_height, id.x, id.y);
        let mf = me.fade;
        let mask_covered = me.covered;
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
