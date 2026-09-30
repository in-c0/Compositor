// Exact float pieces that don't lean on the GPU's `fma`, which some adapters (Microsoft's WARP,
// the software renderer CI's Windows runners use) compute unfused. Products are split
// Veltkamp–Dekker style instead. Needs `keep`, `two_sum`, `fast_two_sum`, `up` and `down` from
// adjust/float.wgsl.

fn split(a: f32) -> vec2<f32> {
    let c = keep(4097.0 * a);
    let hi = keep(c - keep(c - a));
    return vec2<f32>(hi, keep(a - hi));
}

// a · b exactly, as a rounded product and its error.
fn exact_product(a: f32, b: f32) -> vec2<f32> {
    let p = keep(a * b);
    let sa = split(a);
    let sb = split(b);
    let e = keep(keep(keep(keep(sa.x * sb.x) - p) + keep(sa.x * sb.y)) + keep(sa.y * sb.x)) + keep(sa.y * sb.y);
    return vec2<f32>(p, keep(e));
}

// a · b + c with one rounding, as Metal's fused multiply-add.
fn fused(a: f32, b: f32, c: f32) -> f32 {
    let p = exact_product(a, b);
    let s = two_sum(p.x, c);
    return keep(s.x + keep(s.y + p.y));
}

// a - q · b, exactly enough to tell which neighbor of q is nearest a / b.
fn remainder(a: f32, q: f32, b: f32) -> f32 {
    let p = exact_product(q, b);
    return keep(keep(a - p.x) - p.y);
}

// `a / b`, correctly rounded, for a >= 0 and b > 0.
fn quotient_rn(a: f32, b: f32) -> f32 {
    var q = keep(a / b);
    if q <= 0.0 {
        return 0.0;
    }
    for (var i = 0; i < 3; i++) {
        let r = remainder(a, q, b);
        let next = select(down(q), up(q), r > 0.0);
        let s = remainder(a, next, b);
        if abs(s) < abs(r) || (abs(s) == abs(r) && (bitcast<u32>(next) & 1u) == 0u) {
            q = next;
        } else {
            break;
        }
    }
    return q;
}

// `sqrtf`, correctly rounded, for x >= 0.
fn root_rn(x: f32) -> f32 {
    var s = keep(sqrt(x));
    if s <= 0.0 {
        return 0.0;
    }
    for (var i = 0; i < 2; i++) {
        let r = remainder(x, s, s);
        let next = select(down(s), up(s), r > 0.0);
        let t = remainder(x, next, next);
        if abs(t) < abs(r) {
            s = next;
        } else {
            break;
        }
    }
    return s;
}

// 1 / sqrt(x), rounded to nearest, from a double-single square root.
fn rsqrt_rn(x: f32) -> f32 {
    let s0 = root_rn(x);
    let s = fast_two_sum(s0, keep(remainder(x, s0, s0) / keep(2.0 * s0)));
    let q = keep(1.0 / s.x);
    let r = keep(remainder(1.0, q, s.x) - keep(q * s.y));
    return keep(q + keep(r / s.x));
}
