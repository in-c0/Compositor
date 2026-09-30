// Premultiplied canvas to straight alpha, as ImageIO writes a premultiplied CGImage to PNG.

struct Params {
    width: u32,
    height: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> source: array<u32>;
@group(0) @binding(2) var<storage, read_write> dest: array<u32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let index = id.y * params.width + id.x;
    let p = unpack(source[index]);
    if p.w == 0u {
        dest[index] = 0u;
        return;
    }
    let c = min((p.xyz * 255u + vec3<u32>(p.w / 2u)) / p.w, vec3<u32>(255u));
    dest[index] = pack(vec4<u32>(c, p.w));
}
