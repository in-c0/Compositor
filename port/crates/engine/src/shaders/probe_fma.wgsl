// One fused multiply-add of the first three values into the fourth (see `Gpu::probe_fma`).

struct Probe {
    unused: u32,
}

@group(0) @binding(0) var<uniform> probe: Probe;
@group(0) @binding(1) var<storage, read_write> values: array<f32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x == probe.unused && id.y == 0u) {
        values[3] = fma(values[0], values[1], values[2]);
    }
}
