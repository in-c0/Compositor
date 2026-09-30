// MetalBrushCoverage's `continuousBrush`, ported: the tip swept along every settled segment of a
// stroke, as coverage bytes over the stroke's pixel grid. Only the final coverage is drawn, so the
// provisional tails the Mac draws between pointer events (always replaced, never accumulated) are
// left out; each kernel call's settled segments are applied in order, with the permanent density
// clamped after every call as the Mac's buffer is.

struct Params {
    // a, b, c, d of the grid-to-document mapping.
    mapping: vec4<f32>,
    // Document position of the grid's origin, radius, hardness.
    geometry: vec4<f32>,
    // Canvas width, height, antialias width, deposition spacing.
    canvas: vec4<f32>,
    // Grid width, height, number of calls, unused.
    counts: vec4<u32>,
}

@group(0) @binding(0) var<uniform> u: Params;
@group(0) @binding(1) var<storage, read> segments: array<vec4<f32>>;
// The index one past each call's last segment.
@group(0) @binding(2) var<storage, read> ends: array<u32>;
@group(0) @binding(3) var<storage, read_write> coverage: array<u32>;

fn segment_distance_squared(p: vec2<f32>, s: vec4<f32>) -> f32 {
    let v = s.zw - s.xy;
    let t = clamp(dot(p - s.xy, v) / max(dot(v, v), 1e-12), 0.0, 1.0);
    let delta = p - (s.xy + t * v);
    return dot(delta, delta);
}

fn brush_coverage(distance_squared: f32) -> f32 {
    let distance = sqrt(distance_squared);
    let radius = u.geometry.z;
    if u.geometry.w >= 1.0 {
        return clamp((radius - distance) / u.canvas.z + 0.5, 0.0, 1.0);
    }
    let t = clamp((distance / radius - u.geometry.w) / (1.0 - u.geometry.w), 0.0, 1.0);
    return max(0.0, (exp(-2.5 * t * t) - exp(-2.5)) / (1.0 - exp(-2.5)));
}

fn tip_density(distance_squared: f32) -> f32 {
    return -log(max(1.0 - brush_coverage(distance_squared), 0.001));
}

fn segment_density(p: vec2<f32>, s: vec4<f32>) -> f32 {
    let v = s.zw - s.xy;
    let len = length(v);
    if len < 1e-6 {
        return tip_density(dot(p - s.xy, p - s.xy));
    }
    let direction = v / len;
    let projection = dot(p - s.xy, direction);
    let perpendicular = p - s.xy - projection * direction;
    let perpendicular_squared = dot(perpendicular, perpendicular);
    let radius_squared = u.geometry.z * u.geometry.z;
    if perpendicular_squared >= radius_squared {
        return 0.0;
    }
    let reach = sqrt(radius_squared - perpendicular_squared);
    let lo = max(0.0, projection - reach);
    let hi = min(len, projection + reach);
    if hi <= lo {
        return 0.0;
    }
    let midpoint = (lo + hi) * 0.5;
    let half_length = (hi - lo) * 0.5;
    // Eight-point Gauss-Legendre quadrature, clipped to the tip's support.
    let nodes = array<f32, 4>(0.1834346425, 0.5255324099, 0.7966664774, 0.9602898565);
    let weights = array<f32, 4>(0.3626837834, 0.3137066459, 0.2223810345, 0.1012285363);
    var integral = 0.0;
    for (var i = 0u; i < 4u; i++) {
        let a = midpoint - half_length * nodes[i] - projection;
        let b = midpoint + half_length * nodes[i] - projection;
        integral += weights[i] * (tip_density(perpendicular_squared + a * a) + tip_density(perpendicular_squared + b * b));
    }
    return integral * half_length / u.canvas.w;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= u.counts.x || id.y >= u.counts.y {
        return;
    }
    let index = id.y * u.counts.x + id.x;
    let local = vec2<f32>(id.xy) + 0.5;
    let p = u.geometry.xy + local.x * u.mapping.xy + local.y * u.mapping.zw;
    if any(p < vec2<f32>(0.0)) || any(p >= u.canvas.xy) {
        coverage[index] = 0u;
        return;
    }
    var value = 0.0;
    var first = 0u;
    if u.geometry.w >= 1.0 {
        // Hard tips keep the largest coverage any segment gives, which is the nearest segment's.
        var nearest = 3.4e38;
        for (var call = 0u; call < u.counts.z; call++) {
            for (var i = first; i < ends[call]; i++) {
                nearest = min(nearest, segment_distance_squared(p, segments[i]));
            }
            first = ends[call];
        }
        value = brush_coverage(nearest);
    } else {
        for (var call = 0u; call < u.counts.z; call++) {
            for (var i = first; i < ends[call]; i++) {
                value += segment_density(p, segments[i]);
            }
            value = min(value, 20.0);
            first = ends[call];
        }
        value = 1.0 - exp(-value);
    }
    // Metal's round: half away from zero (the value is never negative), on the product as
    // rounded, which a fused 255 × value + 0.5 wouldn't be.
    let x = 255.0 * value;
    let f = floor(x);
    coverage[index] = u32(select(f, f + 1.0, x - f >= 0.5));
}
