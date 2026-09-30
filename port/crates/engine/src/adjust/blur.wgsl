// A separable blur on premultiplied pixels, as Core Image's: taps outside the image read
// transparent black. `step` 0 runs along rows from the 8-bit input into floats, step 1 along
// columns from those floats into the 8-bit output.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    step: u32,
    radius: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
@group(0) @binding(3) var<storage, read_write> rows: array<vec4<f32>>;
// Tap weights for offsets -radius…radius.
@group(0) @binding(4) var<storage, read> weights: array<f32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let w = params.width;
    let h = params.height;
    if (id.x >= w || id.y >= h) {
        return;
    }
    let r = i32(params.radius);
    var sum = vec4<f32>(0.0);
    if (params.step == 0u) {
        for (var k = -r; k <= r; k++) {
            let x = i32(id.x) + k;
            if (x >= 0 && x < i32(w)) {
                sum += weights[k + r] * vec4<f32>(unpack(src[id.y * w + u32(x)]));
            }
        }
        rows[id.y * w + id.x] = sum / 255.0;
    } else {
        for (var k = -r; k <= r; k++) {
            let y = i32(id.y) + k;
            if (y >= 0 && y < i32(h)) {
                sum += weights[k + r] * rows[u32(y) * w + id.x];
            }
        }
        dst[id.y * w + id.x] = pack(to_bytes(sum));
    }
}
