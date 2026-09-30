// Core Image's motion blur kernels as the Mac's GPU runs them, one pass per dispatch (see
// `blur.rs`). Each pass works along rows: the image's own rows, or the rows of the image turned
// so the streak runs along them.
//
// Images between passes are float planes whose values are halfs: Core Image's default working
// format is RGBA half float, and storing a kernel's result there truncates it toward zero.
// Sampling a half plane between texels interpolates exactly and rounds to the nearest half, ties
// away from zero. Sampling the 8-bit source between texels is fixed point: the fraction goes to
// 1/256 and the result to 1/4080. Kernel arithmetic is float, with Apple's compiler fusing the
// multiply-adds written as `fma` and nothing else; `keep` stops DXC fusing more.

struct Params {
    guard: f32,
    op: u32,
    out_w: u32,
    out_h: u32,
    src_w: u32,
    src_h: u32,
    // 0: the 8-bit image, 1: a half plane.
    src_kind: u32,
    // 0: store a half (truncated), 1: write 8-bit pixels.
    out_kind: u32,
    level: u32,
    // The column array index 0 stands for, in the source's and the output's own pixels.
    src_origin: i32,
    out_origin: i32,
    count: u32,
    // A half plane holds columns [lo, hi) of its array; past them the GPU clamps to the edge.
    lo: i32,
    hi: i32,
    cos: f32,
    sin: f32,
    // The turned plane's first column and row, and the image's height (Core Image's y is up).
    x0: i32,
    y0: i32,
    image_h: u32,
    // The turns' translations: sin·h and cos·h.
    tx: f32,
    ty: f32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src8: array<u32>;
@group(0) @binding(2) var<storage, read> srcf: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> dstf: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> dst8: array<u32>;
@group(0) @binding(5) var<storage, read> weights: array<f32>;

// Truncated toward zero to a half.
fn half_trunc(x: f32) -> f32 {
    let a = abs(x);
    if (a < 6.1035156e-5) {
        return sign(x) * floor(a * 16777216.0) * 5.9604645e-8;
    }
    return bitcast<f32>(bitcast<u32>(x) & 0xffffe000u);
}

// Rounded to the nearest half, ties away from zero.
fn half_away(x: f32) -> f32 {
    let a = abs(x);
    if (a < 6.1035156e-5) {
        return sign(x) * floor(a * 16777216.0 + 0.5) * 5.9604645e-8;
    }
    return sign(x) * bitcast<f32>((bitcast<u32>(a) + 0x1000u) & 0xffffe000u);
}

// Steps of 1/4080 to floats, correctly rounded.
fn per4080(v: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(div(v.x, 4080.0), div(v.y, 4080.0), div(v.z, 4080.0), div(v.w, 4080.0));
}

fn texel8(x: i32, y: i32) -> vec4<f32> {
    if (x < 0 || y < 0 || x >= i32(params.src_w) || y >= i32(params.src_h)) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(unpack(src8[u32(y) * params.src_w + u32(x)]));
}

// The 8-bit image along memory row `row` at continuous x (texel centers at k + 0.5).
fn sample8(x: f32, row: i32) -> vec4<f32> {
    let t = x - 0.5;
    let i = floor(t);
    let q = floor((t - i) * 256.0 + 0.5);
    let a = texel8(i32(i), row);
    let b = texel8(i32(i) + 1, row);
    return per4080(floor((a * (256.0 - q) + b * q) / 16.0 + 0.5));
}

// The 8-bit image at continuous texture coordinates (u, v), v counted down the rows.
fn sample8_tex(u: f32, v: f32) -> vec4<f32> {
    let tx = u - 0.5;
    let ty = v - 0.5;
    let ix = floor(tx);
    let iy = floor(ty);
    let qx = floor((tx - ix) * 256.0 + 0.5);
    let qy = floor((ty - iy) * 256.0 + 0.5);
    let r = i32(iy);
    let s = texel8(i32(ix), r) * ((256.0 - qx) * (256.0 - qy)) + texel8(i32(ix) + 1, r) * (qx * (256.0 - qy))
        + texel8(i32(ix), r + 1) * ((256.0 - qx) * qy) + texel8(i32(ix) + 1, r + 1) * (qx * qy);
    return per4080(floor(s / 4096.0 + 0.5));
}

// The 8-bit image at continuous (x, y), y counted up from the bottom as Core Image does.
fn sample8_2d(x: f32, y: f32) -> vec4<f32> {
    return sample8_tex(x, keep(f32(params.src_h) - y));
}

fn texelf(x: i32, y: i32) -> vec4<f32> {
    let cx = clamp(x, params.lo, params.hi - 1);
    let cy = clamp(y, 0, i32(params.src_h) - 1);
    if (cx < 0 || cx >= i32(params.src_w)) {
        return vec4<f32>(0.0);
    }
    return srcf[u32(cy) * params.src_w + u32(cx)];
}

fn round_half(v: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(half_away(v.x), half_away(v.y), half_away(v.z), half_away(v.w));
}

// A half plane along row `row` at continuous x in array coordinates.
fn samplef(x: f32, row: i32) -> vec4<f32> {
    let t = x - 0.5;
    let i = floor(t);
    let f = floor((t - i) * 256.0 + 0.5) / 256.0;
    let a = texelf(i32(i), row);
    let b = texelf(i32(i) + 1, row);
    return round_half(a * (1.0 - f) + b * f);
}

fn sample(x: f32, row: i32) -> vec4<f32> {
    if (params.src_kind == 0u) {
        return sample8(x, row);
    }
    return samplef(x, row);
}

// A half plane at continuous (x, y) in array coordinates, clamped to its edges.
fn samplef_2d(x: f32, y: f32) -> vec4<f32> {
    let tx = x - 0.5;
    let ty = y - 0.5;
    let ix = floor(tx);
    let iy = floor(ty);
    let fx = floor((tx - ix) * 256.0 + 0.5) / 256.0;
    let fy = floor((ty - iy) * 256.0 + 0.5) / 256.0;
    let s = texelf(i32(ix), i32(iy)) * ((1.0 - fx) * (1.0 - fy)) + texelf(i32(ix) + 1, i32(iy)) * (fx * (1.0 - fy))
        + texelf(i32(ix), i32(iy) + 1) * ((1.0 - fx) * fy) + texelf(i32(ix) + 1, i32(iy) + 1) * (fx * fy);
    return round_half(s);
}

fn keep4(v: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(keep(v.x), keep(v.y), keep(v.z), keep(v.w));
}

fn pair(x: f32, o: f32, row: i32) -> vec4<f32> {
    return keep4(sample(keep(x + o), row) + sample(keep(x - o), row));
}

fn mad4(a: vec4<f32>, b: f32, c: vec4<f32>) -> vec4<f32> {
    return fma(a, vec4<f32>(b), c);
}

// `_gaussianReduce2` and `_gaussianReduce4` along the row. The weights buffer holds the center
// weight, the pairs' weights and then their offsets.
fn reduce(j: i32, row: i32) -> vec4<f32> {
    let l = f32(params.level);
    let p = keep(keep((f32(params.out_origin + j) + 0.5) * l) - f32(params.src_origin));
    let n = params.count;
    var acc = keep4(sample(p, row) * weights[0]);
    for (var k = 0u; k < n; k++) {
        acc = mad4(pair(p, weights[1u + n + k], row), weights[1u + k], acc);
    }
    return acc;
}

// `_gaussianBlurN` with taps on texel centers: `count` weights for steps 0…count-1.
fn blur_direct(x: f32, row: i32) -> vec4<f32> {
    let r = params.count - 1u;
    var acc = keep4(pair(x, f32(r), row) * weights[r]);
    for (var k = r - 1u; k >= 2u; k--) {
        acc = mad4(pair(x, f32(k), row), weights[k], acc);
    }
    acc = mad4(sample(x, row), weights[0], acc);
    return mad4(pair(x, 1.0, row), weights[1], acc);
}

// `_gaussianBlurN` from bilinear pairs: `count` (offset, weight) samples on each side.
fn blur_pairs(x: f32, row: i32) -> vec4<f32> {
    var acc = keep4(pair(x, weights[0], row) * weights[1]);
    for (var k = 1u; k < params.count; k++) {
        acc = mad4(pair(x, weights[2u * k], row), weights[2u * k + 1u], acc);
    }
    return acc;
}

fn quotient(a: f32, b: f32) -> f32 {
    if (a < 0.0) {
        return -div(-a, b);
    }
    return div(a, b);
}

// `_cubicUpsample10h`: a cubic B-spline through two bilinear samples.
fn upsample(xi: i32, row: i32) -> vec4<f32> {
    let x = f32(params.out_origin + xi) + 0.5;
    let sub = keep(keep(x * (1.0 / f32(params.level))) - 0.5);
    let i = floor(sub);
    let a = keep(keep(i - sub) + 1.0);
    let f = keep(sub - i);
    let a3 = keep(keep(a * a) * a);
    let m5 = keep(a3 * 0.3333333432674408);
    let m6 = keep(a * 0.5);
    let m7 = keep(m6 * a);
    let add10 = keep(keep(m7 - m5) + m6);
    let wa = keep(add10 + 0.1666666716337204);
    let wb = keep(0.8333333134651184 - add10);
    let m15 = keep(a3 * -0.5);
    let num = keep(keep(keep(m7 + m6) + 0.1666666716337204) + m15);
    let pa = keep(keep(i - 0.5) + quotient(num, wa));
    let d26 = keep(keep(keep(f * f) * 0.1666666716337204) * f);
    let pb = keep(keep(i + 1.5) + quotient(d26, wb));
    let o = f32(params.src_origin);
    let sa = samplef(keep(pa - o), row);
    let sb = samplef(keep(pb - o), row);
    return fma(sb, vec4<f32>(wb), keep4(sa * wa));
}

fn store(i: u32, v: vec4<f32>) {
    if (params.out_kind == 0u) {
        dstf[i] = vec4<f32>(half_trunc(v.x), half_trunc(v.y), half_trunc(v.z), half_trunc(v.w));
    } else {
        let b = clamp(floor(fma(v, vec4<f32>(255.0), vec4<f32>(0.5))), vec4<f32>(0.0), vec4<f32>(255.0));
        dst8[i] = pack(vec4<u32>(b));
    }
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.out_w || id.y >= params.out_h) {
        return;
    }
    let row = i32(id.y);
    var v: vec4<f32>;
    switch params.op {
        // Turn the 8-bit image so the streak runs along the rows.
        // The texture coordinates come straight from the inverse turn, as Core Image folds the
        // image's flip into it.
        case 0u: {
            let px = f32(params.x0 + i32(id.x)) + 0.5;
            let py = f32(params.y0 + i32(id.y)) + 0.5;
            let u = keep(keep(params.cos * px) + keep(-params.sin * py));
            let w = keep(keep(keep(-params.sin * px) + keep(-params.cos * py)) + f32(params.image_h));
            v = sample8_tex(u, w);
        }
        case 1u: {
            v = reduce(i32(id.x), row);
        }
        case 2u: {
            v = blur_direct(f32(params.out_origin - params.src_origin + i32(id.x)) + 0.5, row);
        }
        case 3u: {
            v = blur_pairs(f32(params.out_origin - params.src_origin + i32(id.x)) + 0.5, row);
        }
        case 4u: {
            v = upsample(i32(id.x), row);
        }
        // `_conv3x3sym` (Gaussian Blur with a small σ): four reads at (±p, ±p) around each pixel,
        // averaged; p comes in `cos`.
        case 6u: {
            let px = f32(id.x) + 0.5;
            let py = f32(i32(params.image_h) - 1 - i32(id.y)) + 0.5;
            let p = params.cos;
            let a = sample8_2d(keep(px + p), keep(py + p)) + sample8_2d(keep(px - p), keep(py + p));
            let b = keep4(a) + sample8_2d(keep(px - p), keep(py - p));
            v = keep4(keep4(b) + sample8_2d(keep(px + p), keep(py - p))) * 0.25;
        }
        // Turn back, reading the plane at each pixel's center.
        default: {
            let px = f32(id.x) + 0.5;
            let py = f32(id.y) + 0.5;
            let rx = keep(keep(keep(params.cos * px) + keep(-params.sin * py)) + params.tx);
            let ry = keep(keep(keep(-params.sin * px) + keep(-params.cos * py)) + params.ty);
            v = samplef_2d(keep(rx - f32(params.x0)), keep(ry - f32(params.y0)));
        }
    }
    store(id.y * params.out_w + id.x, v);
}
