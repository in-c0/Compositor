// The blurs Core Image runs for Gaussian Blur and Motion Blur, on premultiplied pixels: taps are
// spaced one step apart along a direction, weighted, and summed in floats, and a tap outside the
// image reads transparent black. Steps along a row or column land on pixel centers; others are
// read bilinearly. `stage` 0 reads the 8-bit input and writes floats, 1 reads floats and writes
// the 8-bit output, 2 does both in one pass.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    stage: u32,
    radius: u32,
    // One step between taps, in pixels (y down).
    dx: f32,
    dy: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
@group(0) @binding(3) var<storage, read_write> floats: array<vec4<f32>>;
// Tap weights for steps -radius…radius.
@group(0) @binding(4) var<storage, read> weights: array<f32>;

fn texel(x: i32, y: i32) -> vec4<f32> {
    if (x < 0 || y < 0 || x >= i32(params.width) || y >= i32(params.height)) {
        return vec4<f32>(0.0);
    }
    let i = u32(y) * params.width + u32(x);
    if (params.stage == 1u) {
        return floats[i];
    }
    return vec4<f32>(unpack(src[i])) / 255.0;
}

// Bilinear, with transparent black outside the image.
fn sample(x: f32, y: f32) -> vec4<f32> {
    let x0 = floor(x);
    let y0 = floor(y);
    let tx = x - x0;
    let ty = y - y0;
    let ix = i32(x0);
    let iy = i32(y0);
    var v = texel(ix, iy) * (1.0 - tx) * (1.0 - ty);
    if (tx > 0.0) {
        v += texel(ix + 1, iy) * tx * (1.0 - ty);
    }
    if (ty > 0.0) {
        v += texel(ix, iy + 1) * (1.0 - tx) * ty;
        if (tx > 0.0) {
            v += texel(ix + 1, iy + 1) * tx * ty;
        }
    }
    return v;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let r = i32(params.radius);
    var sum = vec4<f32>(0.0);
    for (var k = -r; k <= r; k++) {
        let x = f32(id.x) + f32(k) * params.dx;
        let y = f32(id.y) + f32(k) * params.dy;
        sum += weights[k + r] * sample(x, y);
    }
    let i = id.y * params.width + id.x;
    if (params.stage == 0u) {
        floats[i] = sum;
    } else {
        dst[i] = pack(to_bytes(sum));
    }
}
