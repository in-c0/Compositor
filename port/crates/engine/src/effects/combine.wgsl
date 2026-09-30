// `effects_ring` (step 0): what a stroke covers, the difference between the shape and the
// reached-out (or pulled-in) shape. `effects_inside` (step 1): an inner shadow's or inner glow's
// coverage, what is outside the layer, softened, kept to the layer's own shape.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    step: u32,
    smallest: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> shape: array<f32>;
@group(0) @binding(2) var<storage, read> moved: array<f32>;
@group(0) @binding(3) var<storage, read_write> result: array<f32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let index = id.y * params.width + id.x;
    var value: f32;
    if (params.step == 0u) {
        value = select(moved[index] - shape[index], shape[index] - moved[index], params.smallest == 1u);
    } else {
        value = shape[index] * keep(1.0 - moved[index]);
    }
    result[index] = clamp(value, 0.0, 1.0);
}
