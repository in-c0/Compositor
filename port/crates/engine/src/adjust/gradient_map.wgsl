// `adjust_gradient_map` (AdjustPixels.c), all in integers: Rec. 709 luma of the unpremultiplied
// color picks a color from the 256-entry table.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
// RGB bytes packed with red in the low byte.
@group(0) @binding(3) var<storage, read> table: array<u32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let i = id.y * params.width + id.x;
    let p = unpack(src[i]);
    let a = p.w;
    if (a == 0u) {
        dst[i] = src[i];
        return;
    }
    var c = p.xyz;
    if (a < 255u) {
        c = min((c * 255u + vec3<u32>(a / 2u)) / a, vec3<u32>(255u));
    }
    let level = (2126u * c.x + 7152u * c.y + 722u * c.z + 5000u) / 10000u;
    let color = unpack(table[min(level, 255u)]).xyz;
    dst[i] = pack(vec4<u32>((color * a + vec3<u32>(127u)) / 255u, a));
}
