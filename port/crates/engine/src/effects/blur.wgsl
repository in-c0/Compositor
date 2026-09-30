// `effects_blur_rows` and `effects_blur_columns`: a Gaussian along a row or a column, with the
// edge pixels repeated past the edge, divided by the sum of its weights. The weights
// exp(-k² / 2σ²) are the same for every pixel, so the CPU works them out once (`weights` in
// mod.rs); the sums run here in the kernel's order, each product fused into its sum.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    radius: u32,
    // 0 along rows, 1 along columns.
    axis: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> result: array<f32>;
// Weights for offsets -radius…radius.
@group(0) @binding(3) var<storage, read> weights: array<f32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let radius = i32(params.radius);
    let rows = params.axis == 0u;
    let center = select(i32(id.y), i32(id.x), rows);
    let last = select(i32(params.height), i32(params.width), rows) - 1;
    var total = 0.0;
    var weight_sum = 0.0;
    for (var offset = -radius; offset <= radius; offset++) {
        let weight = weights[offset + radius];
        let sample = u32(clamp(center + offset, 0, last));
        var value: f32;
        if (rows) {
            value = source[id.y * params.width + sample];
        } else {
            value = source[sample * params.width + id.x];
        }
        total = keep(fma(weight, value, total));
        weight_sum = keep(weight_sum + weight);
    }
    // `div` needs a positive numerator; a sum of non-negative terms is zero or positive.
    var value = 0.0;
    if (total > 0.0) {
        value = div(total, weight_sum);
    }
    result[id.y * params.width + id.x] = value;
}
