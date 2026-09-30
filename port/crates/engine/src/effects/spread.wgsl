// `effects_spread_rows` and `effects_spread_columns`: the largest (or smallest) value within
// reach along a row or a column; past the edge there is nothing.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    reach: u32,
    smallest: u32,
    // 0 along rows, 1 along columns.
    axis: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> source: array<f32>;
@group(0) @binding(2) var<storage, read_write> result: array<f32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let reach = i32(params.reach);
    let smallest = params.smallest == 1u;
    let rows = params.axis == 0u;
    let center = select(i32(id.y), i32(id.x), rows);
    let count = select(i32(params.height), i32(params.width), rows);
    var best = select(0.0, 1.0, smallest);
    for (var offset = -reach; offset <= reach; offset++) {
        let sample = center + offset;
        var value = 0.0;
        if (sample >= 0 && sample < count) {
            if (rows) {
                value = source[id.y * params.width + u32(sample)];
            } else {
                value = source[u32(sample) * params.width + id.x];
            }
        }
        best = select(max(best, value), min(best, value), smallest);
    }
    result[id.y * params.width + id.x] = best;
}
