// `lens_distort` (LensPixels.c): each pixel is a bilinear sample of the source at a position the
// host works out in `f64`, with the four weights; the sums are kept in double-single, as the C
// keeps them in `double`, and rounded half away from zero.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
}

struct Sample {
    x0: i32,
    y0: i32,
    // Top left, top right, bottom left, bottom right.
    weights: array<vec2<f32>, 4>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
@group(0) @binding(3) var<storage, read> samples: array<Sample>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let i = id.y * params.width + id.x;
    let s = samples[i];
    var sums = array<vec2<f32>, 4>(DD_ZERO, DD_ZERO, DD_ZERO, DD_ZERO);
    for (var j = 0; j < 2; j++) {
        let row = s.y0 + j;
        if (row < 0 || row >= i32(params.height)) {
            continue;
        }
        for (var k = 0; k < 2; k++) {
            let column = s.x0 + k;
            if (column < 0 || column >= i32(params.width)) {
                continue;
            }
            let weight = s.weights[j * 2 + k];
            if (weight.x == 0.0) {
                continue;
            }
            let p = unpack(src[u32(row) * params.width + u32(column)]);
            // sums[c] += weight × p[c], contracted to an fma.
            for (var c = 0; c < 4; c++) {
                sums[c] = dd_add(dd_mul(weight, dd(f32(p[c]))), sums[c]);
            }
        }
    }
    var out: vec4<u32>;
    for (var c = 0; c < 4; c++) {
        out[c] = u32(dd_round(sums[c]));
    }
    dst[i] = pack(out);
}
