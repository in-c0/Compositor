// The byte operations a clipping stack runs on its surface, from the Mac's BrushPixels.c:
// layer_extract_alpha (step 0), layer_unpremultiply_opaque (1) and layer_restore_alpha (2).

struct Params {
    width: u32,
    height: u32,
    step: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> pixels: array<u32>;
@group(0) @binding(2) var<storage, read_write> alpha: array<u32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let index = id.y * params.width + id.x;
    let p = unpack(pixels[index]);
    switch params.step {
        case 0u: {
            alpha[index] = p.w;
        }
        case 1u: {
            // (c * 255 + a / 2) / a, capped at 255; alpha becomes opaque.
            var c = vec3<u32>(0u);
            if p.w > 0u {
                c = min((p.xyz * 255u + vec3<u32>(p.w / 2u)) / p.w, vec3<u32>(255u));
            }
            pixels[index] = pack(vec4<u32>(c, 255u));
        }
        default: {
            // (c * a + 127) / 255 with the saved alpha.
            let a = alpha[index];
            pixels[index] = pack(vec4<u32>((p.xyz * a + vec3<u32>(127u)) / 255u, a));
        }
    }
}
