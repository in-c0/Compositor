// `PixelInvert`'s vImage matrix on premultiplied RGBA: each color becomes alpha − color.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let i = id.y * params.width + id.x;
    let p = unpack(src[i]);
    // (256·alpha − 256·color) / 256, saturated at 0.
    let c = vec3<u32>(p.w) - min(p.xyz, vec3<u32>(p.w));
    dst[i] = pack(vec4<u32>(c, p.w));
}
