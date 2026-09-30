// Blend modes, in the order of comp_format::BlendMode::ALL, fitted to the Mac app's output.
//
// `composite` takes the canvas pixel (premultiplied bytes) and the layer's source pixel after
// opacity (premultiplied bytes, see `scale_source`) and returns the new canvas pixel.
//
// The Mac draws in three ways, and each is reproduced here exactly:
// - Most modes (Core Graphics' own, and the ones SeparableBlend hands to Core Image) follow the
//   W3C compositing formula on straight colors in float, rounded once at the end.
// - Hue, Saturation, Color and Luminosity, and Multiply at full opacity, are Core Graphics'
//   integer path: the blend runs on premultiplied bytes, each side scaled by the other's alpha,
//   with luminance (77 R + 151 G + 28 B) / 256 and 16.16 fixed-point clipping; the two
//   "uncovered" terms are then added as floats and the sum rounded once.
// - Hard Light takes its multiply branch while 2 × source ≤ source alpha + 1 (premultiplied).

const NORMAL: u32 = 0u;
const DARKEN: u32 = 1u;
const MULTIPLY: u32 = 2u;
const COLOR_BURN: u32 = 3u;
const LINEAR_BURN: u32 = 4u;
const LIGHTEN: u32 = 5u;
const SCREEN: u32 = 6u;
const COLOR_DODGE: u32 = 7u;
const LINEAR_DODGE: u32 = 8u;
const OVERLAY: u32 = 9u;
const SOFT_LIGHT: u32 = 10u;
const HARD_LIGHT: u32 = 11u;
const VIVID_LIGHT: u32 = 12u;
const LINEAR_LIGHT: u32 = 13u;
const PIN_LIGHT: u32 = 14u;
const HARD_MIX: u32 = 15u;
const DIFFERENCE: u32 = 16u;
const EXCLUSION: u32 = 17u;
const SUBTRACT: u32 = 18u;
const DIVIDE: u32 = 19u;
const HUE: u32 = 20u;
const SATURATION: u32 = 21u;
const COLOR: u32 = 22u;
const LUMINOSITY: u32 = 23u;

// Layer opacity as Core Graphics applies it: the opacity quantized to a byte, every premultiplied
// byte, alpha included, scaled by it and rounded. The CPU builds the 256 answers (`opacity_table`).
fn scale_premultiplied(p: vec4<u32>) -> vec4<u32> {
    return vec4<u32>(opacity_table[p.x], opacity_table[p.y], opacity_table[p.z], opacity_table[p.w]);
}

fn color_dodge(b: f32, s: f32) -> f32 {
    if b == 0.0 { return 0.0; }
    if s >= 1.0 { return 1.0; }
    return min(1.0, b / (1.0 - s));
}

fn color_burn(b: f32, s: f32) -> f32 {
    if b >= 1.0 { return 1.0; }
    if s <= 0.0 { return 0.0; }
    return 1.0 - min(1.0, (1.0 - b) / s);
}

// `s_multiply` is Hard Light's (and Overlay's, with the roles swapped) branch choice.
fn separable(mode: u32, b: f32, s: f32, s_multiply: bool, b_multiply: bool) -> f32 {
    switch mode {
        case 1u: { return min(b, s); }
        case 2u: { return b * s; }
        case 3u: { return color_burn(b, s); }
        case 4u: { return max(b + s - 1.0, 0.0); }
        case 5u: { return max(b, s); }
        case 6u: { return b + s - b * s; }
        case 7u: { return color_dodge(b, s); }
        case 8u: { return min(b + s, 1.0); }
        case 9u: {
            if b_multiply { return s * 2.0 * b; }
            let t = 2.0 * b - 1.0;
            return s + t - s * t;
        }
        case 10u: {
            if s <= 0.5 { return b - (1.0 - 2.0 * s) * b * (1.0 - b); }
            var d = sqrt(b);
            if b <= 0.25 { d = ((16.0 * b - 12.0) * b + 4.0) * b; }
            return b + (2.0 * s - 1.0) * (d - b);
        }
        case 11u: {
            if s_multiply { return b * 2.0 * s; }
            let t = 2.0 * s - 1.0;
            return b + t - b * t;
        }
        case 12u: {
            if s <= 0.5 { return color_burn(b, 2.0 * s); }
            return color_dodge(b, 2.0 * (s - 0.5));
        }
        case 13u: { return clamp(b + 2.0 * s - 1.0, 0.0, 1.0); }
        case 14u: {
            if s <= 0.5 { return min(b, 2.0 * s); }
            return max(b, 2.0 * s - 1.0);
        }
        case 15u: { return select(0.0, 1.0, b + s > 1.0 + 1e-6); }
        case 16u: { return abs(b - s); }
        case 17u: { return b + s - 2.0 * b * s; }
        case 18u: { return max(b - s, 0.0); }
        case 19u: {
            if s <= 0.0 { return select(0.0, 1.0, b > 0.0); }
            return min(b / s, 1.0);
        }
        default: { return s; }
    }
}

// The W3C general formula, one rounding at the end. The terms that don't depend on the blend
// are exact integers (premultiplied bytes times the other side's uncovered alpha); only the blend
// itself is float, which keeps f32 close enough to the Mac's arithmetic to round the same way.
fn composite_float(mode: u32, backdrop: vec4<u32>, source: vec4<u32>, full_opacity: bool) -> vec4<u32> {
    let ba = backdrop.w;
    let sa = source.w;
    var b = vec3<f32>(0.0);
    if ba > 0u { b = vec3<f32>(backdrop.xyz) / f32(ba); }
    var s = vec3<f32>(0.0);
    if sa > 0u { s = vec3<f32>(source.xyz) / f32(sa); }
    var mixed: vec3<f32>;
    for (var i = 0; i < 3; i++) {
        let s_multiply = 2u * source[i] <= sa + 1u;
        let b_multiply = 2u * backdrop[i] <= ba;
        if mode == HARD_MIX {
            // b + s > 1, compared exactly: bp·sa + sp·ba > ba·sa.
            mixed[i] = select(0.0, 1.0, backdrop[i] * sa + source[i] * ba > ba * sa);
        } else {
            mixed[i] = separable(mode, b[i], s[i], s_multiply, b_multiply);
        }
    }
    let uncovered = source.xyz * (255u - ba) + backdrop.xyz * (255u - sa);
    let color = floor((vec3<f32>(uncovered) + f32(sa * ba) * mixed) / 255.0 + vec3<f32>(0.5));
    var alpha = (sa * 255u + ba * (255u - sa) + 127u) / 255u;
    // At full opacity, Core Graphics' Hard Light keeps the whole backdrop alpha under a source
    // alpha of 1/255.
    if mode == HARD_LIGHT && sa == 1u && full_opacity {
        alpha = ba + 1u;
    }
    return vec4<u32>(vec3<u32>(clamp(color, vec3<f32>(0.0), vec3<f32>(255.0))), min(alpha, 255u));
}

fn lum256(c: vec3<i32>) -> i32 {
    return 77 * c.x + 151 * c.y + 28 * c.z;
}

// Floor division for the fixed-point ratios (numerators and denominators here are >= 0).
fn ratio16(num: i32, den: i32) -> i32 {
    if den <= 0 { return 0; }
    return (num << 16u) / den;
}

fn clip_color_int(c: vec3<i32>, top: i32) -> vec3<i32> {
    let l = (lum256(c) + 128) >> 8u;
    let n = min(c.x, min(c.y, c.z));
    let x = max(c.x, max(c.y, c.z));
    var out = c;
    if n < 0 {
        let k = ratio16(l, l - n);
        out = vec3<i32>(l) + (((out - vec3<i32>(l)) * k + vec3<i32>(0x8000)) >> vec3<u32>(16u));
    }
    if x > top {
        let k = ratio16(top - l, x - l);
        out = vec3<i32>(l) + (((out - vec3<i32>(l)) * k + vec3<i32>(0x8000)) >> vec3<u32>(16u));
    }
    return out;
}

fn set_lum_int(c: vec3<i32>, target_color: vec3<i32>, top: i32) -> vec3<i32> {
    let d = (lum256(target_color) - lum256(c) + 128) >> 8u;
    return clip_color_int(c + vec3<i32>(d), top);
}

fn sat_int(c: vec3<i32>) -> i32 {
    return max(c.x, max(c.y, c.z)) - min(c.x, min(c.y, c.z));
}

fn set_sat_int(c: vec3<i32>, s: i32) -> vec3<i32> {
    // Stable order: min, mid, max, ties keeping channel order, as the Mac sorts.
    var mn = 0; var md = 1; var mx = 2;
    if c[md] < c[mn] { let t = mn; mn = md; md = t; }
    if c[mx] < c[md] { let t = md; md = mx; mx = t; }
    if c[md] < c[mn] { let t = mn; mn = md; md = t; }
    var out = vec3<i32>(0);
    let den = c[mx] - c[mn];
    if den > 0 {
        out[md] = ((c[md] - c[mn]) * ((s << 16u) / den) + 0x8000) >> 16u;
        out[mx] = s;
    }
    return out;
}

fn scale_round(v: vec3<u32>, by: u32) -> vec3<i32> {
    return vec3<i32>(div255v(vec4<u32>(v * by, 0u)).xyz);
}

// Core Graphics' integer path (see the header).
fn composite_int(mode: u32, backdrop: vec4<u32>, source: vec4<u32>) -> vec4<u32> {
    let ba = backdrop.w;
    let sa = source.w;
    let top = i32(div255(sa * ba));
    let tb = scale_round(backdrop.xyz, sa);
    let ts = scale_round(source.xyz, ba);
    var mixed: vec3<i32>;
    switch mode {
        case 20u: { mixed = set_lum_int(set_sat_int(ts, sat_int(tb)), tb, top); }
        case 21u: { mixed = set_lum_int(set_sat_int(tb, sat_int(ts)), tb, top); }
        case 22u: { mixed = set_lum_int(ts, tb, top); }
        case 23u: { mixed = set_lum_int(tb, ts, top); }
        default: { mixed = vec3<i32>(div255v(vec4<u32>(source.xyz * backdrop.xyz, 0u)).xyz); }
    }
    // round(mixed + uncovered / 255), exactly: 255 is odd, so the sum never lands on a half.
    let uncovered = vec3<i32>(source.xyz * (255u - ba) + backdrop.xyz * (255u - sa));
    let color = (mixed * 255 + uncovered + vec3<i32>(127)) / 255;
    // Multiply's alpha goes through the same sum; the non-separable modes keep the usual alpha.
    var alpha = i32(sa + ba) - top;
    if mode == MULTIPLY {
        alpha = (top * 255 + i32(sa * (255u - ba) + ba * (255u - sa)) + 127) / 255;
    }
    return vec4<u32>(vec3<u32>(clamp(color, vec3<i32>(0), vec3<i32>(255))), u32(clamp(alpha, 0, 255)));
}

fn composite(mode: u32, backdrop: vec4<u32>, source: vec4<u32>, full_opacity: bool) -> vec4<u32> {
    if mode >= HUE || (mode == MULTIPLY && full_opacity) {
        return composite_int(mode, backdrop, source);
    }
    return composite_float(mode, backdrop, source, full_opacity);
}
