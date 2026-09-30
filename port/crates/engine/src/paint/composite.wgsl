// A stroke's paint drawn into the grid through its coverage, as `BrushStroke.publish` does with
// Core Graphics: the color filled through the coverage as a clip mask at the brush's opacity
// (mode 0), the same with destination-out to erase (1), gray on a mask (2), or the Clone Stamp or
// Blur sample drawn through it (3), or copied through it for Smudge and Liquify (4).

struct Params {
    width: u32,
    height: u32,
    mode: u32,
    is_mask: u32,
    color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> base: array<u32>;
@group(0) @binding(2) var<storage, read> coverage: array<u32>;
@group(0) @binding(3) var<storage, read> sample: array<u32>;
@group(0) @binding(4) var<storage, read> reaches: array<u32>;
@group(0) @binding(5) var<storage, read_write> dest: array<u32>;

// Core Graphics' source-over of `s` (premultiplied bytes) at `alpha` over `d`.
fn over(s: vec4<u32>, d: vec4<u32>, alpha: f32) -> vec4<u32> {
    let a = vec4<f32>(alpha);
    let r = vec4<f32>(s) * a + vec4<f32>(d) * (vec4<f32>(1.0) - a * f32(s.w) / 255.0);
    return vec4<u32>(clamp(floor(r + vec4<f32>(0.5)), vec4<f32>(0.0), vec4<f32>(255.0)));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let i = id.y * params.width + id.x;
    let c = coverage[i];
    let d = base[i];
    if c == 0u {
        dest[i] = d;
        return;
    }
    let alpha = f32(c) / 255.0 * params.color.w;
    switch params.mode {
        case 0u: {
            let s = vec4<u32>(to_bytes(vec4<f32>(params.color.xyz, 1.0)));
            dest[i] = pack(over(s, unpack(d), alpha));
        }
        case 1u: {
            let r = vec4<f32>(unpack(d)) * (1.0 - alpha);
            dest[i] = pack(vec4<u32>(floor(r + vec4<f32>(0.5))));
        }
        case 2u: {
            let s = to_byte(params.color.x);
            let r = f32(s) * alpha + f32(d) * (1.0 - alpha);
            dest[i] = u32(floor(r + 0.5));
        }
        case 3u: {
            if reaches[i] == 0u {
                dest[i] = d;
                return;
            }
            if params.is_mask == 1u {
                let r = f32(sample[i]) * alpha + f32(d) * (1.0 - alpha);
                dest[i] = u32(floor(r + 0.5));
            } else {
                dest[i] = pack(over(unpack(sample[i]), unpack(d), alpha));
            }
        }
        default: {
            if reaches[i] == 0u {
                dest[i] = d;
                return;
            }
            let s = vec4<f32>(unpack(sample[i])) * alpha + vec4<f32>(unpack(d)) * (1.0 - f32(c) / 255.0);
            dest[i] = pack(vec4<u32>(floor(s + vec4<f32>(0.5))));
        }
    }
}
