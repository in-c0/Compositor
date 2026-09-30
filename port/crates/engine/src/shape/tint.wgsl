// A shape's coverage painted in its color: the color premultiplied by the coverage with
// truncation, `(color * coverage) / 255`, as Core Graphics fills a path in one color.

struct Params {
    width: u32,
    height: u32,
    color: u32,
    pad: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
// One coverage byte per pixel, in the low byte of each word.
@group(0) @binding(1) var<storage, read> coverage: array<u32>;
@group(0) @binding(2) var<storage, read_write> out: array<u32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let index = id.y * params.width + id.x;
    let a = coverage[index] & 255u;
    let color = unpack(params.color);
    out[index] = pack(vec4<u32>((color.xyz * a) / vec3<u32>(255u), a));
}
