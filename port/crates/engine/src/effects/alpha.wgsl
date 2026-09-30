// `effects_alpha`, with the step before it: the layer's pixels as they are shown (premultiplied,
// through the layer's own mask) placed `inset` pixels in from every edge of the grown image, and
// the shape's coverage, alpha / 255, as a float per pixel.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    layer_width: u32,
    layer_height: u32,
    inset: u32,
    has_mask: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
// Straight PNG pixels, one per layer pixel.
@group(0) @binding(1) var<storage, read> layer: array<u32>;
@group(0) @binding(2) var<storage, read> mask: array<u32>;
@group(0) @binding(3) var<storage, read_write> padded: array<u32>;
@group(0) @binding(4) var<storage, read_write> coverage: array<f32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let index = id.y * params.width + id.x;
    let lx = i32(id.x) - i32(params.inset);
    let ly = i32(id.y) - i32(params.inset);
    var pixel = vec4<u32>(0u);
    if (lx >= 0 && ly >= 0 && lx < i32(params.layer_width) && ly < i32(params.layer_height)) {
        let layer_index = u32(ly) * params.layer_width + u32(lx);
        let straight = unpack(layer[layer_index]);
        pixel = vec4<u32>(div255v(vec4<u32>(straight.xyz * straight.w, 0u)).xyz, straight.w);
        if (params.has_mask != 0u) {
            pixel = div255v(pixel * mask[layer_index]);
        }
    }
    padded[index] = pack(pixel);
    coverage[index] = div(f32(pixel.w), 255.0);
}
