// `effects_compose`: shadow behind, outer glow over it, an outside stroke over that, the layer's
// pixels over that, then a color overlay, an inner glow, an inner shadow and an inside stroke on
// top, each drawn source-over in float and the result rounded to premultiplied bytes once. The
// multiply-adds are fused, as Metal's compiler fuses them.

struct Params {
    guard: f32,
    width: u32,
    height: u32,
    // Bit 0 stroke, 1 inside stroke, 2 shadow, 3 inner shadow, 4 color overlay, 5 outer glow,
    // 6 inner glow.
    flags: u32,
    // rgb, opacity.
    stroke: vec4<f32>,
    shadow: vec4<f32>,
    overlay: vec4<f32>,
    inner: vec4<f32>,
    glow: vec4<f32>,
    inner_glow: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> pixels: array<u32>;
@group(0) @binding(2) var<storage, read> ring: array<f32>;
@group(0) @binding(3) var<storage, read> shadow: array<f32>;
@group(0) @binding(4) var<storage, read_write> result: array<u32>;
@group(0) @binding(5) var<storage, read> inner: array<f32>;
@group(0) @binding(6) var<storage, read> shape: array<f32>;
@group(0) @binding(7) var<storage, read> glow: array<f32>;
@group(0) @binding(8) var<storage, read> inner_glow: array<f32>;

fn has(bit: u32) -> bool {
    return (params.flags & (1u << bit)) != 0u;
}

var<private> color: vec3<f32>;
var<private> alpha: f32;

// color = c * coverage + color * (1 - coverage), alpha = coverage + alpha * (1 - coverage).
// Metal fuses the second product of the color sum, not the first: at an exact tie between two
// bytes (effects/fractional, where the sum is 59.5 / 255) only that order rounds as the Mac does.
fn over(c: vec3<f32>, coverage: f32) {
    let rest = keep(1.0 - coverage);
    let painted = vec3<f32>(keep(c.x * coverage), keep(c.y * coverage), keep(c.z * coverage));
    color = fma(color, vec3<f32>(rest), painted);
    alpha = keep(fma(alpha, rest, coverage));
}

// `uchar(clamp(v, 0, 1) * 255 + 0.5)`.
fn byte(v: f32) -> u32 {
    return u32(floor(fma(clamp(v, 0.0, 1.0), 255.0, 0.5)));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let index = id.y * params.width + id.x;
    color = vec3<f32>(0.0);
    alpha = 0.0;
    if (has(2u)) {
        let coverage = clamp(shadow[index] * params.shadow.w, 0.0, 1.0);
        color = params.shadow.xyz * coverage;
        alpha = coverage;
    }
    if (has(5u)) {
        over(params.glow.xyz, clamp(keep(glow[index] * keep(1.0 - shape[index])) * params.glow.w, 0.0, 1.0));
    }
    let stroke_coverage = select(0.0, clamp(ring[index] * params.stroke.w, 0.0, 1.0), has(0u));
    if (has(0u) && !has(1u)) {
        over(params.stroke.xyz, stroke_coverage);
    }
    let p = unpack(pixels[index]);
    let source = vec4<f32>(div(f32(p.x), 255.0), div(f32(p.y), 255.0), div(f32(p.z), 255.0), div(f32(p.w), 255.0));
    let rest = keep(1.0 - source.w);
    color = fma(color, vec3<f32>(rest), source.xyz);
    alpha = keep(fma(alpha, rest, source.w));
    if (has(4u)) {
        over(params.overlay.xyz, clamp(shape[index] * params.overlay.w, 0.0, 1.0));
    }
    if (has(6u)) {
        over(params.inner_glow.xyz, clamp(inner_glow[index] * params.inner_glow.w, 0.0, 1.0));
    }
    if (has(3u)) {
        over(params.inner.xyz, clamp(inner[index] * params.inner.w, 0.0, 1.0));
    }
    if (has(0u) && has(1u)) {
        over(params.stroke.xyz, stroke_coverage);
    }
    result[index] = pack(vec4<u32>(byte(color.x), byte(color.y), byte(color.z), byte(alpha)));
}
