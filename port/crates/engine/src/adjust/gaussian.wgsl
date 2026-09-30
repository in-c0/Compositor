// Metal Performance Shaders' Gaussian blur passes as the Mac's GPU runs them (see `gaussian.rs`),
// on float planes whose values are halfs. Every pass stores its result truncated to a half; a
// sample between texels is interpolated exactly, with the fraction at 1/256, and rounded to the
// nearest half, ties away from zero. `fma` marks the multiply-adds Apple's compiler fuses.

struct Params {
    guard: f32,
    op: u32,
    // 0: along columns (vertical), 1: along rows.
    axis: u32,
    out_w: u32,
    out_h: u32,
    src_w: u32,
    src_h: u32,
    count: u32,
    level: u32,
    // The pixel array index 0 stands for, along the axis, in the source's and the output's level.
    src_origin: i32,
    out_origin: i32,
    a: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> dst: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> weights: array<f32>;
@group(0) @binding(4) var<storage, read> src8: array<u32>;
@group(0) @binding(5) var<storage, read_write> dst8: array<u32>;

fn half_trunc(x: f32) -> f32 {
    let a = abs(x);
    if (a < 6.1035156e-5) {
        return sign(x) * floor(a * 16777216.0) * 5.9604645e-8;
    }
    return bitcast<f32>(bitcast<u32>(x) & 0xffffe000u);
}

fn half_away(x: f32) -> f32 {
    let a = abs(x);
    if (a < 6.1035156e-5) {
        return sign(x) * floor(a * 16777216.0 + 0.5) * 5.9604645e-8;
    }
    return sign(x) * bitcast<f32>((bitcast<u32>(a) + 0x1000u) & 0xffffe000u);
}

// `hi + lo` (lo tiny beside hi, both ≥ 0 in use) rounded to the nearest half, ties to even.
fn half_even(hi: f32, lo: f32) -> f32 {
    let a = abs(hi);
    if (a < 6.1035156e-5) {
        let t = hi * 16777216.0;
        let f = floor(t);
        let r = t - f;
        var up = r > 0.5 || (r == 0.5 && (lo > 0.0 || (lo == 0.0 && (u32(f) & 1u) == 1u)));
        return (f + select(0.0, 1.0, up)) * 5.9604645e-8;
    }
    let bits = bitcast<u32>(a);
    let rest = bits & 0x1fffu;
    var down = bits & 0xffffe000u;
    let even = (down & 0x2000u) == 0u;
    let tie_up = select(!even, lo * sign(hi) > 0.0, lo != 0.0);
    if (rest > 0x1000u || (rest == 0x1000u && tie_up)) {
        down += 0x2000u;
    } else if (rest == 0u && lo * sign(hi) < 0.0) {
        // Exactly a half with a negative remainder: the true value is just below.
        return sign(hi) * bitcast<f32>(down);
    }
    return sign(hi) * bitcast<f32>(down);
}

// Two halfs' product (exact in a float) plus a half, rounded once to a half.
fn half_sum(p: f32, acc: f32) -> f32 {
    let s = keep(p + acc);
    let v = keep(s - p);
    let e = keep(keep(p - keep(s - v)) + keep(acc - v));
    return half_even(s, e);
}

fn keep4(v: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(keep(v.x), keep(v.y), keep(v.z), keep(v.w));
}

// Texel `i` along the axis in the line through (x, y); transparent outside the plane.
fn texel(i: i32, x: u32, y: u32) -> vec4<f32> {
    var cx = i32(x);
    var cy = i32(y);
    if (params.axis == 0u) {
        cy = i;
    } else {
        cx = i;
    }
    if (cx < 0 || cy < 0 || cx >= i32(params.src_w) || cy >= i32(params.src_h)) {
        return vec4<f32>(0.0);
    }
    return src[u32(cy) * params.src_w + u32(cx)];
}

// A bilinear sample at continuous position p along the axis.
fn sample(p: f32, x: u32, y: u32) -> vec4<f32> {
    let t = p - 0.5;
    let i = floor(t);
    let f = floor((t - i) * 256.0 + 0.5) / 256.0;
    let v = texel(i32(i), x, y) * (1.0 - f) + texel(i32(i) + 1, x, y) * f;
    return vec4<f32>(half_away(v.x), half_away(v.y), half_away(v.z), half_away(v.w));
}

fn pair(p: f32, o: f32, x: u32, y: u32) -> vec4<f32> {
    return keep4(sample(keep(p + o), x, y) + sample(keep(p - o), x, y));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.out_w || id.y >= params.out_h) {
        return;
    }
    // The 8-bit image into the padded plane Core Image hands over (`count` is the padding), and
    // the result back to 8 bits.
    if (params.op == 3u) {
        let x = i32(id.x) - i32(params.count);
        let y = i32(id.y) - i32(params.count);
        var v = vec4<f32>(0.0);
        if (x >= 0 && y >= 0 && x < i32(params.src_w) && y < i32(params.src_h)) {
            let b = vec4<f32>(unpack(src8[u32(y) * params.src_w + u32(x)]));
            v = vec4<f32>(half_trunc(div(b.x, 255.0)), half_trunc(div(b.y, 255.0)), half_trunc(div(b.z, 255.0)), half_trunc(div(b.w, 255.0)));
        }
        dst[id.y * params.out_w + id.x] = v;
        return;
    }
    if (params.op == 4u) {
        let v = src[(id.y + params.count) * params.src_w + id.x + params.count];
        dst8[id.y * params.out_w + id.x] = pack(vec4<u32>(clamp(floor(fma(v, vec4<f32>(255.0), vec4<f32>(0.5))), vec4<f32>(0.0), vec4<f32>(255.0))));
        return;
    }
    // `kNx1`/`k1xN`, Metal Performance Shaders' separable convolution in half floats: taps on
    // texels (-r…r), each product added in one rounding.
    if (params.op == 5u) {
        let r = i32(params.count / 2u);
        var k = i32(id.x);
        if (params.axis == 0u) {
            k = i32(id.y);
        }
        let c = params.out_origin - params.src_origin + k;
        var acc = texel(c - r, id.x, id.y) * weights[0];
        acc = vec4<f32>(half_even(acc.x, 0.0), half_even(acc.y, 0.0), half_even(acc.z, 0.0), half_even(acc.w, 0.0));
        for (var t = 1u; t < params.count; t++) {
            let p = texel(c - r + i32(t), id.x, id.y) * weights[t];
            acc = vec4<f32>(half_sum(p.x, acc.x), half_sum(p.y, acc.y), half_sum(p.z, acc.z), half_sum(p.w, acc.w));
        }
        dst[id.y * params.out_w + id.x] = acc;
        return;
    }
    // The output's position along the axis, and the line it lies on in the source.
    var k = id.x;
    if (params.axis == 0u) {
        k = id.y;
    }
    let n = params.count;
    var acc: vec4<f32>;
    switch params.op {
        // `DlFn`: shrink by `level`, taps ±0.5, ±1.5… around the new pixel's center.
        case 0u: {
            let l = f32(params.level);
            let j = f32(params.out_origin + i32(k));
            let p = keep(keep(keep(j * l) + l * 0.5) - f32(params.src_origin));
            acc = keep4(sample(p, id.x, id.y) * keep(2.0 * weights[0]));
            for (var t = 1u; t < n; t += 2u) {
                let b = select(0.0, weights[t + 1u], t + 1u < n);
                let w = keep(weights[t] + b);
                let o = keep(div(b, w) + (f32(t) + 0.5));
                acc = fma(pair(p, o, id.x, id.y), vec4<f32>(w), acc);
            }
        }
        // `Fn`: taps on 0, ±1, ±2…, read in pairs.
        case 1u: {
            let p = f32(params.out_origin - params.src_origin + i32(k)) + 0.5;
            acc = keep4(sample(p, id.x, id.y) * weights[0]);
            for (var t = 1u; t < n; t += 2u) {
                var w = weights[t];
                var o = f32(t);
                if (t + 1u < n) {
                    w = keep(weights[t] + weights[t + 1u]);
                    o = keep(div(weights[t + 1u], w) + f32(t));
                }
                acc = fma(pair(p, o, id.x, id.y), vec4<f32>(w), acc);
            }
        }
        // `UlP3F3`: grow by `level` from three texels with Metal Performance Shaders' weights.
        default: {
            let s = i32(params.level);
            let x = params.out_origin + i32(k);
            let m = x - (((x % s) + s) % s);
            let t = f32(x - m);
            let mm = m / s;
            let a = params.a;
            var e: f32;
            var e1: f32;
            var e2m: f32;
            if (s == 2) {
                e = keep(t - 0.5);
                e1 = keep(t - 1.5);
                e2m = keep(-0.5 - t);
            } else {
                let ts = keep(t * (2.0 / f32(s)));
                e = keep(ts - (f32(s) - 1.0) / f32(s));
                e1 = keep(ts - (2.0 * f32(s) - 1.0) / f32(s));
                e2m = keep(-(1.0 / f32(s)) - ts);
            }
            let ee = keep(e * e);
            let eee = keep(ee * e);
            let s54 = keep(keep(e1 + ee) - eee);
            let m56 = keep(ee * -2.0);
            let m57 = keep(e * 5.0);
            let s61 = keep(keep(keep(keep(4.0 - m57) + m56) + keep(eee * 3.0)) * 0.125);
            let s64 = keep(keep(e2m + ee) + eee);
            let s69 = keep(keep(keep(keep(m57 + 4.0) + m56) + keep(eee * -3.0)) * 0.125);
            let ha = keep(a * 0.5);
            let wl = keep(keep(ha * s54) + s61);
            let wm = keep(keep(ee * keep(0.5 - a)) + a);
            let wr = keep(s69 + keep(ha * s64));
            let base = mm - params.src_origin;
            let l = texel(base - 1, id.x, id.y);
            let c = texel(base, id.x, id.y);
            let r = texel(base + 1, id.x, id.y);
            acc = fma(r, vec4<f32>(wr), fma(l, vec4<f32>(wl), keep4(c * wm)));
        }
    }
    dst[id.y * params.out_w + id.x] = vec4<f32>(half_trunc(acc.x), half_trunc(acc.y), half_trunc(acc.z), half_trunc(acc.w));
}
