// IEEE single-precision arithmetic as the Mac's C kernels do it, shared by the adjustment kernels.
//
// GPU compilers treat float math loosely: DXC and FXC fuse `a * b + c` into one multiply-add and
// may reassociate, `/` is only accurate to about 2 ulp, and `log`, `cos` and `sqrt` are hardware
// approximations. The Mac rounds every operation, and a 1-ulp difference is enough to move a byte
// across a rounding boundary. So every kernel's `Params` starts with `guard: f32`, which the host
// sets to +infinity: `min(x, params.guard)` returns `x` unchanged, but the compiler can't see
// through it, so the rounding of `x` is kept. Division, square roots, logarithms and cosines are
// rebuilt from exact pieces: `fma` (fused on Metal by definition, and measured fused on DX12 by
// the `float_helpers_are_correctly_rounded` test) and
// double-single arithmetic.

// `x`, rounded on its own: no fusing or reassociating it with what uses it.
fn keep(x: f32) -> f32 {
    return min(x, params.guard);
}

// Whether clang contracted the C kernels' `a * b + c` into fused multiply-adds when it built
// the Mac app (`-ffp-contract=on`, its default for C on Apple silicon).
const CONTRACT: bool = true;

// `a * b + c` as the C kernels compute it. Clang contracts a product that is a direct operand
// of a sum in the same expression, trying the left operand first: `x * y + z * w` becomes
// `fma(x, y, z * w)`.
fn mad(a: f32, b: f32, c: f32) -> f32 {
    if (CONTRACT) {
        return fma(a, b, c);
    }
    return keep(a * b) + c;
}

// The neighbors of a positive finite float.
fn up(x: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(x) + 1u);
}

fn down(x: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(x) - 1u);
}

// `a / b`, correctly rounded, for a >= 0 and b > 0. The hardware quotient is within 2 ulp; the
// exact remainder a - q·b (one fma) says which neighbor is nearest.
fn div(a: f32, b: f32) -> f32 {
    var q = keep(a / b);
    if (q <= 0.0) {
        return 0.0;
    }
    for (var i = 0; i < 3; i++) {
        let r = fma(-q, b, a);
        let next = select(down(q), up(q), r > 0.0);
        let s = fma(-next, b, a);
        if (abs(s) < abs(r) || (abs(s) == abs(r) && (bitcast<u32>(next) & 1u) == 0u)) {
            q = next;
        } else {
            break;
        }
    }
    return q;
}

// `sqrtf`, correctly rounded, for x >= 0.
fn root(x: f32) -> f32 {
    var s = keep(sqrt(x));
    if (s <= 0.0) {
        return 0.0;
    }
    for (var i = 0; i < 2; i++) {
        let r = fma(-s, s, x);
        let next = select(down(s), up(s), r > 0.0);
        let t = fma(-next, next, x);
        if (abs(t) < abs(r)) {
            s = next;
        } else {
            break;
        }
    }
    return s;
}

// `roundf` and `lroundf`: halves away from zero (WGSL's `round` goes to even).
fn round_away(x: f32) -> f32 {
    let a = abs(x);
    let t = trunc(a);
    return sign(x) * select(t, t + 1.0, a - t >= 0.5);
}

// Double-single numbers: `x + y` with |y| <= ulp(x) / 2, about 48 bits.

fn two_sum(a: f32, b: f32) -> vec2<f32> {
    let s = keep(a + b);
    let v = keep(s - a);
    let e = keep(a - keep(s - v)) + keep(b - v);
    return vec2<f32>(s, keep(e));
}

fn fast_two_sum(a: f32, b: f32) -> vec2<f32> {
    let s = keep(a + b);
    return vec2<f32>(s, keep(b - keep(s - a)));
}

fn two_prod(a: f32, b: f32) -> vec2<f32> {
    let p = keep(a * b);
    return vec2<f32>(p, keep(fma(a, b, -p)));
}

fn dd_add(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let s = two_sum(a.x, b.x);
    let t = two_sum(a.y, b.y);
    let u = fast_two_sum(s.x, keep(s.y + t.x));
    return fast_two_sum(u.x, keep(t.y + u.y));
}

fn dd_mul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let p = two_prod(a.x, b.x);
    let cross = keep(keep(a.x * b.y) + keep(a.y * b.x));
    return fast_two_sum(p.x, keep(p.y + cross));
}

fn dd_div(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    let q = keep(a.x / b.x);
    // a - q·b, exactly enough for the correction.
    let r = dd_add(a, -dd_mul(vec2<f32>(q, 0.0), b));
    let c = keep(r.x / b.x);
    return fast_two_sum(q, c);
}

// Horner steps: acc · x + k.
fn dd_horner(acc: vec2<f32>, x: vec2<f32>, k: vec2<f32>) -> vec2<f32> {
    return dd_add(dd_mul(acc, x), k);
}

// `logf`, correctly rounded but for vanishingly rare ties, for positive normal x.
fn log_rn(x: f32) -> f32 {
    let bits = bitcast<u32>(x);
    var e = i32(bits >> 23u) - 127;
    var m = bitcast<f32>((bits & 0x7fffffu) | 0x3f800000u);
    if (m > 1.41421356) {
        m = m * 0.5;
        e += 1;
    }
    // log m = 2 atanh f, f = (m - 1) / (m + 1); m - 1 is exact.
    let f = dd_div(vec2<f32>(keep(m - 1.0), 0.0), two_sum(m, 1.0));
    let s = dd_mul(f, f);
    var acc = vec2<f32>(0.0344827584922313690, 1.284582856753147e-10); // 1/29
    acc = dd_horner(acc, s, vec2<f32>(0.0370370373129844666, -2.759474315716659e-10)); // 1/27
    acc = dd_horner(acc, s, vec2<f32>(0.0399999991059303284, 8.940696516468449e-10)); // 1/25
    acc = dd_horner(acc, s, vec2<f32>(0.0434782616794109344, -8.098457460192776e-10)); // 1/23
    acc = dd_horner(acc, s, vec2<f32>(0.0476190485060214996, -8.869738832295582e-10)); // 1/21
    acc = dd_horner(acc, s, vec2<f32>(0.0526315793395042419, -3.9213582381236733e-10)); // 1/19
    acc = dd_horner(acc, s, vec2<f32>(0.0588235296308994293, -2.1913472425527658e-10)); // 1/17
    acc = dd_horner(acc, s, vec2<f32>(0.0666666701436042786, -3.47693762670076e-09)); // 1/15
    acc = dd_horner(acc, s, vec2<f32>(0.0769230797886848450, -2.8656079731348427e-09)); // 1/13
    acc = dd_horner(acc, s, vec2<f32>(0.0909090936183929443, -2.709302115988521e-09)); // 1/11
    acc = dd_horner(acc, s, vec2<f32>(0.1111111119389533997, -8.278422947149977e-10)); // 1/9
    acc = dd_horner(acc, s, vec2<f32>(0.1428571492433547974, -6.38621200366174e-09)); // 1/7
    acc = dd_horner(acc, s, vec2<f32>(0.2000000029802322388, -2.9802322831784522e-09)); // 1/5
    acc = dd_horner(acc, s, vec2<f32>(0.3333333432674407959, -9.934107758624577e-09)); // 1/3
    acc = dd_horner(acc, s, vec2<f32>(1.0, 0.0));
    let log_m = dd_mul(dd_mul(acc, f), vec2<f32>(2.0, 0.0));
    let ln2 = vec2<f32>(0.693147182464599609, -1.9046542121259336e-09);
    let total = dd_add(dd_mul(vec2<f32>(f32(e), 0.0), ln2), log_m);
    return total.x;
}

// `cosf`, correctly rounded but for vanishingly rare ties, for 0 <= x < 8.
fn cos_rn(x: f32) -> f32 {
    // x - k·π/2 in double-single, with π/2 in three parts; k·p1 is exact.
    let k = floor(keep(x * 0.636619772) + 0.5);
    let p1 = 1.57079601287841797;
    let p2 = 3.1391647326017846e-07;
    let p3 = 5.329070518200751e-15;
    var r = two_sum(x, -keep(k * p1));
    r = dd_add(r, -two_prod(k, p2));
    r = dd_add(r, vec2<f32>(-keep(k * p3), 0.0));
    let r2 = dd_mul(r, r);
    let quadrant = u32(k) & 3u;
    var v: vec2<f32>;
    if ((quadrant & 1u) == 0u) {
        // cos r = 1 - r²/2! + r⁴/4! - …
        var acc = vec2<f32>(4.7794772561329454e-14, 7.62544404448643e-22); // 1/16!
        acc = dd_horner(acc, r2, vec2<f32>(-1.1470745360508960e-11, -2.372207689231238e-19)); // -1/14!
        acc = dd_horner(acc, r2, vec2<f32>(2.0876755879584152e-09, 1.1082839809204342e-16)); // 1/12!
        acc = dd_horner(acc, r2, vec2<f32>(-2.7557319981497130e-07, 7.575112209051195e-15)); // -1/10!
        acc = dd_horner(acc, r2, vec2<f32>(2.4801587642286904e-05, -3.40699609366682e-13)); // 1/8!
        acc = dd_horner(acc, r2, vec2<f32>(-0.0013888889225199819, 3.3631094437103215e-11)); // -1/6!
        acc = dd_horner(acc, r2, vec2<f32>(0.0416666679084300995, -1.2417634698280722e-09)); // 1/4!
        acc = dd_horner(acc, r2, vec2<f32>(-0.5, 0.0));
        v = dd_horner(acc, r2, vec2<f32>(1.0, 0.0));
    } else {
        // sin r = r (1 - r²/3! + r⁴/5! - …)
        var acc = vec2<f32>(-2.8114573589663704e-15, 1.0462084739763658e-22); // -1/17!
        acc = dd_horner(acc, r2, vec2<f32>(7.6471636098127130e-13, 1.2200710471178288e-20)); // 1/15!
        acc = dd_horner(acc, r2, vec2<f32>(-1.6059044372074283e-10, 5.352526511562726e-18)); // -1/13!
        acc = dd_horner(acc, r2, vec2<f32>(2.5052107943679403e-08, 4.4176230446483665e-16)); // 1/11!
        acc = dd_horner(acc, r2, vec2<f32>(-2.7557318844628753e-06, -3.793571224297229e-14)); // -1/9!
        acc = dd_horner(acc, r2, vec2<f32>(0.00019841270113829523, -2.725596874933456e-12)); // 1/7!
        acc = dd_horner(acc, r2, vec2<f32>(-0.0083333337679505348, 4.34617203337595e-10)); // -1/5!
        acc = dd_horner(acc, r2, vec2<f32>(0.1666666716337203979, -4.967053879312289e-09)); // 1/3!
        acc = dd_horner(acc, r2, vec2<f32>(-1.0, 0.0));
        v = dd_mul(acc, r); // -sin r
    }
    // Quadrants 0..3: cos r, -sin r, -cos r, sin r.
    let negate = quadrant == 2u || quadrant == 3u;
    return select(v.x, -v.x, negate);
}
