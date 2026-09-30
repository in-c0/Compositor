// `CIBloom`'s composite over its Gaussian blur, as measured against the references: each
// premultiplied channel moves toward the brighter of itself and the blur by the intensity,
// s + (max(s, blur) − s) × intensity, and Core Image stores the result clamped to a byte.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    intensity: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
@group(0) @binding(3) var<storage, read> blurred: array<u32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let i = id.y * params.width + id.x;
    let s = vec4<f32>(unpack(src[i])) / 255.0;
    let b = vec4<f32>(unpack(blurred[i])) / 255.0;
    dst[i] = pack(to_bytes(s + (max(s, b) - s) * params.intensity));
}
