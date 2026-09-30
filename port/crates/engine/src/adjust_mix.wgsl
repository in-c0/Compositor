// How an adjustment layer's result goes back on the canvas (LiveMaskRenderer.adjust):
// step 0 mixes toward the original at the layer's opacity, as CIBlendWithMask does with a
// constant mask (orig + (adj - orig) * opacity in float on bytes / 255, rounded);
// step 1 copies through coverage, as a clipped CGBlendMode.copy does
// ((adj * m + orig * (255 - m) + 127) / 255).

struct Params {
    width: u32,
    height: u32,
    step: u32,
    opacity: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> original: array<u32>;
@group(0) @binding(2) var<storage, read> adjusted: array<u32>;
@group(0) @binding(3) var<storage, read> coverage: array<u32>;
@group(0) @binding(4) var<storage, read_write> result: array<u32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let i = id.y * params.width + id.x;
    let o = unpack(original[i]);
    let a = unpack(adjusted[i]);
    if params.step == 0u {
        let ov = vec4<f32>(o) / 255.0;
        let av = vec4<f32>(a) / 255.0;
        result[i] = pack(to_bytes(ov + (av - ov) * params.opacity));
    } else {
        let m = coverage[i];
        result[i] = pack((a * m + o * (255u - m) + vec4<u32>(127u)) / 255u);
    }
}
