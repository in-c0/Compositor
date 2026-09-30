// MetalWarp's kernels, ported: Smudge's pick-up and dab, and Liquify's forward warp. The Mac's
// textures are 8-bit unorm (read as k / 255, written rounded) and 32-bit float; here they are
// storage buffers holding the same values. One dispatch is one Mac dispatch, `kind` says which.

struct Dab {
    guard: f32,
    kind: u32,
    radius: i32,
    inverse_radius: f32,
    center: vec2<i32>,
    size: vec2<i32>,
    origin: vec2<i32>,
    area: vec2<i32>,
    hardness: f32,
    keep_amount: f32,
    move_by: vec2<f32>,
}

@group(0) @binding(0) var<uniform> params: Dab;
@group(0) @binding(1) var<storage, read_write> canvas: array<u32>;
@group(0) @binding(2) var<storage, read_write> carried: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> original: array<u32>;
@group(0) @binding(4) var<storage, read_write> offsets: array<vec2<f32>>;
@group(0) @binding(5) var<storage, read_write> scratch: array<vec2<f32>>;

// An 8-bit unorm texel as a float read gives it.
fn texel_value(p: u32) -> vec4<f32> {
    let k = vec4<f32>(unpack(p));
    return vec4<f32>(div(k.x, 255.0), div(k.y, 255.0), div(k.z, 255.0), div(k.w, 255.0));
}

fn texel(i: i32) -> u32 {
    return canvas[i];
}

// A float in 0...1 written to an 8-bit unorm texture.
fn store(v: vec4<f32>) -> u32 {
    return pack(vec4<u32>(clamp(floor(v * 255.0 + vec4<f32>(0.5)), vec4<f32>(0.0), vec4<f32>(255.0))));
}

fn round_all(v: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(round_away(v.x), round_away(v.y), round_away(v.z), round_away(v.w));
}

// How much a dab moves pixels at a distance u (0 center, 1 rim) from its center.
fn weight(u: f32, hardness: f32) -> f32 {
    if u >= 1.0 {
        return 0.0;
    }
    if u <= hardness {
        return 1.0;
    }
    let t = (1.0 - u) / (1.0 - hardness);
    return t * t * (3.0 - 2.0 * t);
}

fn dab_weight(offset: vec2<i32>) -> f32 {
    return weight(root(f32(offset.x * offset.x + offset.y * offset.y)) * params.inverse_radius, params.hardness);
}

fn inside(p: vec2<i32>) -> bool {
    return p.x >= 0 && p.y >= 0 && p.x < params.size.x && p.y < params.size.y;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let gid = vec2<i32>(id.xy);
    let side = 2 * params.radius + 1;
    let width = params.size.x;
    switch params.kind {
        // warp_pick_up
        case 0u: {
            if gid.x >= side || gid.y >= side {
                return;
            }
            let p = params.center + gid - params.radius;
            var value = vec4<f32>(0.0);
            if inside(p) {
                value = texel_value(texel(p.y * width + p.x)) * 255.0;
            }
            carried[gid.y * side + gid.x] = value;
        }
        // warp_smudge
        case 1u: {
            if gid.x >= side || gid.y >= side {
                return;
            }
            let offset = gid - params.radius;
            let p = params.center + offset;
            if !inside(p) {
                return;
            }
            let w = dab_weight(offset);
            if w <= 0.0 {
                return;
            }
            let i = p.y * width + p.x;
            let c = gid.y * side + gid.x;
            let under = texel_value(texel(i)) * 255.0;
            let held = carried[c];
            let painted = under + (held - under) * w * params.keep_amount;
            canvas[i] = store(clamp(round_all(painted), vec4<f32>(0.0), vec4<f32>(255.0)) / 255.0);
            carried[c] = painted;
        }
        // warp_copy, the offsets under the dab into scratch
        case 2u: {
            if gid.x >= params.area.x || gid.y >= params.area.y {
                return;
            }
            let p = params.origin + gid;
            scratch[gid.y * params.area.x + gid.x] = offsets[p.y * width + p.x];
        }
        // warp_copy, the layer as the stroke found it
        case 3u: {
            if gid.x >= params.size.x || gid.y >= params.size.y {
                return;
            }
            original[gid.y * width + gid.x] = canvas[gid.y * width + gid.x];
        }
        // warp_push
        default: {
            if gid.x >= side || gid.y >= side {
                return;
            }
            let offset = gid - params.radius;
            let p = params.center + offset;
            let last = params.origin + params.area - 1;
            if p.x < params.origin.x || p.y < params.origin.y || p.x > last.x || p.y > last.y {
                return;
            }
            let w = dab_weight(offset);
            if w <= 0.0 {
                return;
            }
            // Bilinear sample of the offsets as they were, from behind the brush's travel.
            let sx = min(f32(params.area.x - 1), max(0.0, f32(p.x - params.origin.x) - params.move_by.x * w));
            let sy = min(f32(params.area.y - 1), max(0.0, f32(p.y - params.origin.y) - params.move_by.y * w));
            let ix = min(params.area.x - 2, i32(sx));
            let iy = min(params.area.y - 2, i32(sy));
            if ix < 0 || iy < 0 {
                return;
            }
            let fx = sx - f32(ix);
            let fy = sy - f32(iy);
            let aw = params.area.x;
            let o00 = scratch[iy * aw + ix];
            let o10 = scratch[iy * aw + ix + 1];
            let o01 = scratch[(iy + 1) * aw + ix];
            let o11 = scratch[(iy + 1) * aw + ix + 1];
            let moved = mix(mix(o00, o10, vec2<f32>(fx)), mix(o01, o11, vec2<f32>(fx)), vec2<f32>(fy)) - params.move_by * w;
            offsets[p.y * width + p.x] = moved;
            // The untouched layer where that offset points, held to its edges.
            let source = clamp(vec2<f32>(p) + moved, vec2<f32>(0.0), vec2<f32>(params.size - 1));
            let si = min(vec2<i32>(source), params.size - 2);
            let f = source - vec2<f32>(si);
            let c00 = texel_value(original[si.y * width + si.x]);
            let c10 = texel_value(original[si.y * width + si.x + 1]);
            let c01 = texel_value(original[(si.y + 1) * width + si.x]);
            let c11 = texel_value(original[(si.y + 1) * width + si.x + 1]);
            let color = mix(mix(c00, c10, vec4<f32>(f.x)), mix(c01, c11, vec4<f32>(f.x)), vec4<f32>(f.y));
            canvas[p.y * width + p.x] = store(clamp(round_all(color * 255.0), vec4<f32>(0.0), vec4<f32>(255.0)) / 255.0);
        }
    }
}
