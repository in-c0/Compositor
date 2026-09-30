// Copies an image into rows `stride` pixels apart, the layout texture uploads need.

struct Params {
    width: u32,
    height: u32,
    stride: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> source: array<u32>;
@group(0) @binding(2) var<storage, read_write> padded: array<u32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    padded[id.y * params.stride + id.x] = source[id.y * params.width + id.x];
}
