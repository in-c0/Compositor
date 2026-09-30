// A stroke's paint drawn into the grid through its coverage, as `BrushStroke.publish` does with
// Core Graphics: the color filled through the coverage as a clip mask at the brush's opacity
// (mode 0), the same with destination-out to erase (1), gray on a mask (2), the Clone Stamp or
// Blur sample drawn through it (3), or copied through it for Smudge and Liquify (4).
//
// Core Graphics works in bytes here, and two ways, measured on the references. At full opacity
// the fill is a lerp through the clip mask's coverage m, rounded down:
//     (s·m + d·(255 − m)) / 255.
// Below it, the source is premultiplied by the opacity byte A = round(opacity × 255) and rounded,
// then source and alpha are each scaled by the coverage and rounded, and the backdrop is scaled
// by what's left and rounded:
//     round(s·m / 255) + round(d·(255 − round(A·m / 255)) / 255).

struct Params {
    width: u32,
    height: u32,
    mode: u32,
    is_mask: u32,
    // The paint, premultiplied by the opacity byte and rounded (the host computes it in
    // double precision, as Core Graphics does), and the opacity byte.
    color: vec4<u32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> base: array<u32>;
@group(0) @binding(2) var<storage, read> coverage: array<u32>;
@group(0) @binding(3) var<storage, read> sample: array<u32>;
@group(0) @binding(4) var<storage, read> reaches: array<u32>;
@group(0) @binding(5) var<storage, read_write> dest: array<u32>;

// x / 255, rounded down, for x up to 255 × 255 × 2.
fn floor255(x: vec4<u32>) -> vec4<u32> {
    return x / vec4<u32>(255u);
}

fn round255(x: vec4<u32>) -> vec4<u32> {
    return (x + vec4<u32>(127u)) / vec4<u32>(255u);
}

// Source-over of premultiplied `s` (already carrying the opacity) with alpha byte `alpha`,
// through coverage `m`, onto `d`.
fn over(s: vec4<u32>, alpha: u32, d: vec4<u32>, m: u32) -> vec4<u32> {
    if alpha == 255u {
        return floor255(s * m + d * (255u - m));
    }
    let a = (alpha * m + 127u) / 255u;
    return round255(s * m) + round255(d * (255u - a));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let i = id.y * params.width + id.x;
    let m = coverage[i];
    let d = base[i];
    if m == 0u {
        dest[i] = d;
        return;
    }
    let alpha = params.color.w;
    switch params.mode {
        case 0u: {
            dest[i] = pack(over(vec4<u32>(params.color.xyz, alpha), alpha, unpack(d), m));
        }
        case 1u: {
            // Destination-out: the backdrop scaled by what the eraser leaves, rounded, at any
            // opacity.
            dest[i] = pack(round255(unpack(d) * (255u - (alpha * m + 127u) / 255u)));
        }
        case 2u: {
            // A mask is one gray channel, with no alpha. Below full opacity the lerp is rounded
            // the other way, as if in 255 − gray (fitted on white over black; see the probes).
            let gray = select(0u, 255u, params.color.x != 0u);
            if alpha == 255u {
                dest[i] = (gray * m + d * (255u - m)) / 255u;
            } else {
                let am = alpha * m;
                dest[i] = 255u - ((255u - gray) * am + (255u - d) * (65025u - am)) / 65025u;
            }
        }
        case 3u: {
            if reaches[i] == 0u {
                dest[i] = d;
                return;
            }
            if params.is_mask == 1u {
                dest[i] = over(vec4<u32>(sample[i]), 255u, vec4<u32>(d), m).x;
            } else {
                // An image is drawn the translucent way even when it's opaque, after the
                // opacity scales it.
                let sp = round255(unpack(sample[i]) * alpha);
                dest[i] = pack(round255(sp * m) + round255(unpack(d) * (255u - (sp.w * m + 127u) / 255u)));
            }
        }
        default: {
            if reaches[i] == 0u {
                dest[i] = d;
                return;
            }
            dest[i] = pack(floor255(unpack(sample[i]) * m + unpack(d) * (255u - m)));
        }
    }
}
