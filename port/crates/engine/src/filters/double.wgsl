// Doubles as double-single numbers (`float.wgsl`), for the C kernels that work in `double`.
// About 48 bits against a double's 53: a byte can only come out differently where the exact
// result lies within about 1e-12 of a rounding boundary.

const DD_ONE = vec2<f32>(1.0, 0.0);
const DD_ZERO = vec2<f32>(0.0, 0.0);

fn dd(x: f32) -> vec2<f32> {
    return vec2<f32>(x, 0.0);
}

fn dd_sub(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return dd_add(a, -b);
}

fn dd_less(a: vec2<f32>, b: vec2<f32>) -> bool {
    return a.x < b.x || (a.x == b.x && a.y < b.y);
}

fn dd_min(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return select(b, a, dd_less(a, b));
}

fn dd_max(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return select(a, b, dd_less(a, b));
}

// `camera_clamp`: 0...1.
fn dd_clamp01(a: vec2<f32>) -> vec2<f32> {
    return dd_min(dd_max(a, DD_ZERO), DD_ONE);
}

// `round` and `lround` for a value >= -0.5: halves away from zero.
fn dd_round(a: vec2<f32>) -> f32 {
    let f = floor(a.x);
    let r = dd_add(a, dd(-f));
    if (r.x > 0.5 || (r.x == 0.5 && r.y >= 0.0)) {
        return f + 1.0;
    }
    return f;
}

// `rec709` (AdjustPixels.c), `0.2126 * r + 0.7152 * g + 0.0722 * b`, which clang contracts to
// `fma(0.0722, b, fma(0.2126, r, 0.7152 * g))`.
fn dd_rec709(r: vec2<f32>, g: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let kr = vec2<f32>(0.2125999927520752, 7.2479249269008506e-09);
    let kg = vec2<f32>(0.7152000069618225, -6.961822673900997e-09);
    let kb = vec2<f32>(0.0722000002861023, -2.861023085110048e-10);
    return dd_add(dd_mul(kb, b), dd_add(dd_mul(kr, r), dd_mul(kg, g)));
}

// e^x for |x| <= 40: x = k ln 2 + r with |r| <= ln 2 / 2, e^r by its Taylor series, then 2^k.
fn dd_exp(x: vec2<f32>) -> vec2<f32> {
    let ln2 = vec2<f32>(0.693147182464599609, -1.9046542121259336e-09);
    let k = floor(x.x * 1.44269504 + 0.5);
    let r = dd_sub(x, dd_mul(dd(k), ln2));
    var term = DD_ONE;
    var sum = DD_ONE;
    for (var n = 1; n <= 20; n++) {
        term = dd_div(dd_mul(term, r), dd(f32(n)));
        sum = dd_add(sum, term);
    }
    let scale = exp2(k);
    return vec2<f32>(sum.x * scale, sum.y * scale);
}

// tanh x as (e^{2|x|} − 1) / (e^{2|x|} + 1), with the sign put back.
fn dd_tanh(x: vec2<f32>) -> vec2<f32> {
    let negative = dd_less(x, DD_ZERO);
    let a = select(x, -x, negative);
    let e = dd_exp(dd_mul(a, dd(2.0)));
    let t = dd_div(dd_sub(e, DD_ONE), dd_add(e, DD_ONE));
    return select(t, -t, negative);
}
