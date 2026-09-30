// `dither_apply` and `dither_dots` (DitherPixels.c), one stage per dispatch. The C works in
// `float` throughout; every step here rounds where the C rounds (`keep`, `mad` and the correctly
// rounded helpers in `float.wgsl`), since a threshold compare turns one ulp into a whole level.
//
// `tone` holds one plane per channel dithered (one for the two-color looks, three for Original):
// each pixel's straight color or luminance after density and contrast.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    stage: u32,
    planes: u32,
    levels: u32,
    style: u32,
    light_on_dark: u32,
    original: u32,
    // Halftone cell, or the distance between scanlines; the block size for round pixels.
    cell: u32,
    // Error diffusion works through rows `row0 <= y < row1` per dispatch.
    row0: u32,
    row1: u32,
    diffusion: f32,
    contrast: f32,
    cos_a: f32,
    sin_a: f32,
    dots: f32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
    dark: vec4<f32>,
    light: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
@group(0) @binding(3) var<storage, read_write> tone: array<f32>;
// Scanlines: each line's sideways wobble, in pixels.
@group(0) @binding(4) var<storage, read> shifts: array<i32>;

const TONE = 0u;
const DIFFUSE = 1u;
const ORDERED = 2u;
const WRITE_LEVELS = 3u;
const MARKS = 4u;
const SCANLINES = 5u;
const ENLARGE = 6u;
const ROUND_PIXELS = 7u;

const ATKINSON = 0u;
const BAYER_2 = 2u;
const BAYER_4 = 3u;
const BAYER_8 = 4u;
const DOTS = 5u;
const LINES = 6u;
const PATTERNS = 8u;

fn clamp01(v: f32) -> f32 {
    return select(select(v, 1.0, v > 1.0), 0.0, v < 0.0);
}

// `a / b` for b > 0 and any sign of a, correctly rounded.
fn sdiv(a: f32, b: f32) -> f32 {
    return select(div(a, b), -div(-a, b), a < 0.0);
}

// `0.2126f * r + 0.7152f * g + 0.0722f * b`, as clang contracts it.
fn luma(r: f32, g: f32, b: f32) -> f32 {
    return mad(0.0722, b, mad(0.2126, r, keep(0.7152 * g)));
}

// `adjust_tone` with gamma 1 (Density 0): powf(v, 1) is v.
fn adjust_tone(v: f32) -> f32 {
    return clamp01(mad(keep(clamp01(v) - 0.5), params.contrast, 0.5));
}

fn alpha_at(i: u32) -> u32 {
    return src[i] >> 24u;
}

// The straight color of a premultiplied pixel, `px[c] * (1.0f / px[3])`.
fn straight(p: vec4<u32>) -> vec3<f32> {
    if (p.w == 0u) {
        return vec3<f32>(0.0);
    }
    let scale = div(1.0, f32(p.w));
    return vec3<f32>(keep(f32(p.x) * scale), keep(f32(p.y) * scale), keep(f32(p.z) * scale));
}

// `write_pixel`: straight color back to premultiplied bytes at the pixel's own alpha.
fn write_pixel(alpha: u32, c: vec3<f32>) -> u32 {
    let a = div(f32(alpha), 255.0);
    let r = round_away(keep(keep(clamp01(c.x) * a) * 255.0));
    let g = round_away(keep(keep(clamp01(c.y) * a) * 255.0));
    let b = round_away(keep(keep(clamp01(c.z) * a) * 255.0));
    return pack(vec4<u32>(u32(r), u32(g), u32(b), alpha));
}

fn quantize(v: f32, levels: u32) -> f32 {
    let steps = f32(levels - 1u);
    return div(round_away(keep(clamp01(v) * steps)), steps);
}

fn ordered_threshold(style: u32, x: u32, y: u32) -> f32 {
    if (style == BAYER_2) {
        let m = array<u32, 4>(0u, 2u, 3u, 1u);
        return (f32(m[(y & 1u) * 2u + (x & 1u)]) + 0.5) / 4.0;
    }
    if (style == BAYER_4) {
        let m = array<u32, 16>(0u, 8u, 2u, 10u, 12u, 4u, 14u, 6u, 3u, 11u, 1u, 9u, 15u, 7u, 13u, 5u);
        return (f32(m[(y & 3u) * 4u + (x & 3u)]) + 0.5) / 16.0;
    }
    let m = array<u32, 64>(
        0u, 32u, 8u, 40u, 2u, 34u, 10u, 42u, 48u, 16u, 56u, 24u, 50u, 18u, 58u, 26u,
        12u, 44u, 4u, 36u, 14u, 46u, 6u, 38u, 60u, 28u, 52u, 20u, 62u, 30u, 54u, 22u,
        3u, 35u, 11u, 43u, 1u, 33u, 9u, 41u, 51u, 19u, 59u, 27u, 49u, 17u, 57u, 25u,
        15u, 47u, 7u, 39u, 13u, 45u, 5u, 37u, 63u, 31u, 55u, 23u, 61u, 29u, 53u, 21u);
    return (f32(m[(y & 7u) * 8u + (x & 7u)]) + 0.5) / 64.0;
}

// Old Mac fill patterns, one byte per row with the leftmost pixel in the top bit, sparsest first.
fn pattern_row(index: u32, row: u32) -> u32 {
    let patterns = array<array<u32, 8>, 17>(
        array<u32, 8>(0x00u, 0x00u, 0x00u, 0x00u, 0x00u, 0x00u, 0x00u, 0x00u),
        array<u32, 8>(0x80u, 0x00u, 0x00u, 0x00u, 0x08u, 0x00u, 0x00u, 0x00u),
        array<u32, 8>(0x88u, 0x00u, 0x22u, 0x00u, 0x88u, 0x00u, 0x22u, 0x00u),
        array<u32, 8>(0x80u, 0x40u, 0x20u, 0x10u, 0x08u, 0x04u, 0x02u, 0x01u),
        array<u32, 8>(0x88u, 0x22u, 0x88u, 0x22u, 0x88u, 0x22u, 0x88u, 0x22u),
        array<u32, 8>(0x00u, 0xFFu, 0x00u, 0x00u, 0x00u, 0xFFu, 0x00u, 0x00u),
        array<u32, 8>(0x11u, 0x22u, 0x44u, 0x88u, 0x11u, 0x22u, 0x44u, 0x88u),
        array<u32, 8>(0xAAu, 0x00u, 0xAAu, 0x00u, 0xAAu, 0x00u, 0xAAu, 0x00u),
        array<u32, 8>(0x88u, 0x55u, 0x22u, 0x55u, 0x88u, 0x55u, 0x22u, 0x55u),
        array<u32, 8>(0xFFu, 0x80u, 0x80u, 0x80u, 0xFFu, 0x08u, 0x08u, 0x08u),
        array<u32, 8>(0xAAu, 0x55u, 0xAAu, 0x55u, 0xAAu, 0x55u, 0xAAu, 0x55u),
        array<u32, 8>(0x81u, 0x42u, 0x24u, 0x18u, 0x18u, 0x24u, 0x42u, 0x81u),
        array<u32, 8>(0x77u, 0xAAu, 0xDDu, 0xAAu, 0x77u, 0xAAu, 0xDDu, 0xAAu),
        array<u32, 8>(0xEEu, 0xDDu, 0xBBu, 0x77u, 0xEEu, 0xDDu, 0xBBu, 0x77u),
        array<u32, 8>(0x77u, 0xFFu, 0xDDu, 0xFFu, 0x77u, 0xFFu, 0xDDu, 0xFFu),
        array<u32, 8>(0x7Fu, 0xFFu, 0xFFu, 0xFFu, 0xF7u, 0xFFu, 0xFFu, 0xFFu),
        array<u32, 8>(0xFFu, 0xFFu, 0xFFu, 0xFFu, 0xFFu, 0xFFu, 0xFFu, 0xFFu));
    return patterns[index][row];
}

// Error diffusion of one plane over rows row0..row1, in serpentine order.
fn diffuse(plane: u32) {
    let w = params.width;
    let h = params.height;
    let base = plane * w * h;
    let atkinson = params.style == ATKINSON;
    // Atkinson: six neighbors at 1/8 each. Floyd–Steinberg: four at 7, 3, 5, 1 sixteenths.
    var dxs = array<i32, 6>(1, -1, 0, 1, 0, 0);
    var dys = array<i32, 6>(0, 1, 1, 1, 0, 0);
    var weights = array<f32, 6>(7.0, 3.0, 5.0, 1.0, 0.0, 0.0);
    var count = 4;
    var divisor = 16.0;
    if (atkinson) {
        dxs = array<i32, 6>(1, 2, -1, 0, 1, 0);
        dys = array<i32, 6>(0, 0, 1, 1, 1, 2);
        weights = array<f32, 6>(1.0, 1.0, 1.0, 1.0, 1.0, 1.0);
        count = 6;
        divisor = 8.0;
    }
    for (var y = params.row0; y < params.row1; y++) {
        let reverse = (y & 1u) != 0u;
        for (var i = 0u; i < w; i++) {
            let x = select(i, w - 1u - i, reverse);
            let at = y * w + x;
            if (alpha_at(at) == 0u) {
                continue;
            }
            let old = tone[base + at];
            let q = quantize(old, params.levels);
            tone[base + at] = q;
            let error = sdiv(keep(keep(old - q) * params.diffusion), divisor);
            for (var t = 0; t < count; t++) {
                let nx = i32(x) + select(dxs[t], -dxs[t], reverse);
                let ny = i32(y) + dys[t];
                if (nx < 0 || nx >= i32(w) || ny >= i32(h)) {
                    continue;
                }
                let n = base + u32(ny) * w + u32(nx);
                tone[n] = mad(error, weights[t], tone[n]);
            }
        }
    }
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = params.width;
    let h = params.height;
    let count = w * h;
    if (params.stage == DIFFUSE) {
        if (id.x < params.planes && id.y == 0u) {
            diffuse(id.x);
        }
        return;
    }
    if (id.x >= w || id.y >= h) {
        return;
    }
    let x = id.x;
    let y = id.y;
    let i = y * w + x;
    let p = unpack(src[i]);

    if (params.stage == ENLARGE) {
        // The dithered copy blown back up without smoothing; `src` is the small image.
        let small_width = (w + params.cell - 1u) / params.cell;
        dst[i] = src[(y / params.cell) * small_width + x / params.cell];
        return;
    }
    if (params.stage == ROUND_PIXELS) {
        // `dither_dots`: each block becomes a round dot in its own color on the gap color.
        let block = params.cell;
        let radius = keep(f32(block) * 0.42);
        let middle = f32(block) / 2.0;
        var out = p;
        if (p.w != 0u) {
            let dy = keep(keep(f32(y % block) + 0.5) - middle);
            let dx = keep(keep(f32(x % block) + 0.5) - middle);
            let cover = clamp01(keep(keep(radius - root(mad(dx, dx, keep(dy * dy)))) + 0.5));
            if (cover < 1.0) {
                // The gap color's bytes.
                let gap = params.dark.xyz;
                for (var c = 0; c < 3; c++) {
                    let back = keep(div(gap[c] * f32(p.w), 255.0) * keep(1.0 - cover));
                    out[c] = u32(round_away(mad(f32(p[c]), cover, back)));
                }
            }
        }
        dst[i] = pack(out);
        return;
    }
    if (params.stage == TONE) {
        let s = straight(p);
        if (params.original != 0u) {
            tone[i] = adjust_tone(s.x);
            tone[count + i] = adjust_tone(s.y);
            tone[2u * count + i] = adjust_tone(s.z);
        } else {
            tone[i] = adjust_tone(luma(s.x, s.y, s.z));
        }
        return;
    }
    if (params.stage == ORDERED) {
        if (p.w != 0u) {
            let steps = f32(params.levels - 1u);
            let threshold = ordered_threshold(params.style, x, y);
            for (var c = 0u; c < params.planes; c++) {
                let v = tone[c * count + i];
                let q = floor(mad(clamp01(v), steps, threshold));
                tone[c * count + i] = div(min(q, steps), steps);
            }
        }
        return;
    }
    if (p.w == 0u) {
        dst[i] = src[i];
        return;
    }
    let dark = params.dark.xyz;
    let light = params.light.xyz;
    if (params.stage == WRITE_LEVELS) {
        if (params.original != 0u) {
            dst[i] = write_pixel(p.w, vec3<f32>(tone[i], tone[count + i], tone[2u * count + i]));
        } else {
            let t = tone[i];
            dst[i] = write_pixel(p.w, vec3<f32>(
                mad(keep(light.x - dark.x), t, dark.x),
                mad(keep(light.y - dark.y), t, dark.y),
                mad(keep(light.z - dark.z), t, dark.z)));
        }
        return;
    }
    if (params.stage == MARKS) {
        var t = tone[i];
        if (params.original != 0u) {
            t = luma(tone[i], tone[count + i], tone[2u * count + i]);
        }
        let lod = params.light_on_dark != 0u;
        var amount: f32;
        if (params.style == PATTERNS) {
            let coverage = select(keep(1.0 - t), t, lod);
            let index = u32(round_away(keep(coverage * 16.0)));
            amount = f32((pattern_row(index, y & 7u) >> (7u - (x & 7u))) & 1u);
        } else {
            let cell = f32(params.cell);
            let fx = f32(x) + 0.5;
            let fy = f32(y) + 0.5;
            var u = sdiv(mad(fx, params.cos_a, keep(fy * params.sin_a)), cell);
            var v = sdiv(mad(-fx, params.sin_a, keep(fy * params.cos_a)), cell);
            u = keep(u - keep(floor(u) + 0.5));
            v = keep(v - keep(floor(v) + 0.5));
            var spot: f32;
            if (params.style == DOTS) {
                spot = keep(3.14159265 * mad(u, u, keep(v * v)));
            } else if (params.style == LINES) {
                spot = keep(abs(v) * 2.0);
            } else {
                spot = keep(abs(u) + abs(v));
            }
            amount = select(0.0, 1.0, select(keep(1.0 - t), t, lod) > spot);
        }
        if (params.original != 0u) {
            let paper = select(1.0, 0.0, lod);
            let s = straight(p);
            dst[i] = write_pixel(p.w, vec3<f32>(
                mad(keep(s.x - paper), amount, paper),
                mad(keep(s.y - paper), amount, paper),
                mad(keep(s.z - paper), amount, paper)));
        } else {
            let ink = select(dark, light, lod);
            let paper = select(light, dark, lod);
            dst[i] = write_pixel(p.w, vec3<f32>(
                mad(keep(ink.x - paper.x), amount, paper.x),
                mad(keep(ink.y - paper.y), amount, paper.y),
                mad(keep(ink.z - paper.z), amount, paper.z)));
        }
        return;
    }
    // SCANLINES: a CRT's lines, each the average of the rows it covers.
    let spacing = max(params.cell, 2u);
    let middle = f32(spacing) / 2.0;
    let dots = clamp01(params.dots);
    let line = y / spacing;
    let top = line * spacing;
    let bottom = min(top + spacing, h);
    let shift = shifts[line];
    let offset = abs(keep(keep(f32(y - top) + 0.5) - middle));
    let along = keep(keep(f32(x % spacing) + 0.5) - middle);
    let centered = i32(round_away(mad(-along, dots, f32(x))));
    let at = u32(clamp(centered, 0, i32(w) - 1));
    // The line's tone at column `at`: the rows it covers, `shift` pixels to the left.
    var sum = vec3<f32>(0.0);
    var n = 0u;
    let sx = i32(at) - shift;
    if (sx >= 0 && sx < i32(w)) {
        for (var yy = top; yy < bottom; yy++) {
            let k = yy * w + u32(sx);
            if (alpha_at(k) == 0u) {
                continue;
            }
            for (var c = 0u; c < params.planes; c++) {
                sum[c] = keep(sum[c] + tone[c * count + k]);
            }
            n++;
        }
    }
    var scan = vec3<f32>(0.0);
    if (n != 0u) {
        for (var c = 0u; c < params.planes; c++) {
            scan[c] = div(sum[c], f32(n));
        }
    }
    var color: vec3<f32>;
    var t: f32;
    if (params.original != 0u) {
        color = scan;
        t = luma(scan.x, scan.y, scan.z);
    } else {
        t = scan.x;
        color = vec3<f32>(
            mad(keep(light.x - dark.x), t, dark.x),
            mad(keep(light.y - dark.y), t, dark.y),
            mad(keep(light.z - dark.z), t, dark.z));
    }
    color = vec3<f32>(keep(color.x * 1.35), keep(color.y * 1.35), keep(color.z * 1.35));
    let beam = keep(middle * mad(0.5, root(clamp01(t)), 0.2));
    let across = keep(along * dots);
    let distance = root(mad(offset, offset, keep(across * across)));
    let cover = clamp01(keep(keep(beam - distance) + 0.5));
    let screen = select(dark, vec3<f32>(0.0), params.original != 0u);
    dst[i] = write_pixel(p.w, vec3<f32>(
        mad(keep(color.x - screen.x), cover, screen.x),
        mad(keep(color.y - screen.y), cover, screen.y),
        mad(keep(color.z - screen.z), cover, screen.z)));
}
