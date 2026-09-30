use crate::gpu::Gpu;

/// Runs `body` (which sets `out` from `v`) once per input on the GPU.
fn eval(gpu: &Gpu, name: &'static str, body: &str, inputs: &[[f32; 4]]) -> Vec<f32> {
    let source = format!(
        "{}
struct Params {{ guard: f32, width: u32 }}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> input: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> output: array<f32>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    let i = id.y * params.width + id.x;
    if (i >= arrayLength(&input)) {{ return; }}
    let v = input[i];
    var out: f32;
    {body}
    output[i] = out;
}}",
        [include_str!("../adjust/float.wgsl"), include_str!("exact.wgsl")].join("\n")
    );
    let n = inputs.len() as u32;
    let width = 256;
    let input = gpu.bytes(bytemuck::cast_slice(inputs));
    let output = gpu.image(n, 1);
    let pipeline = gpu.pipeline(name, &source);
    let words = [f32::INFINITY.to_bits(), width];
    gpu.dispatch(&pipeline, bytemuck::cast_slice(&words), &[&input, &output.buffer], width, n.div_ceil(width));
    bytemuck::cast_slice(&gpu.download(&output).unwrap()).to_vec()
}

fn random(count: usize) -> Vec<[f32; 4]> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    (0..count)
        .map(|_| {
            let mut next = || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                f32::from_bits((state as u32 >> 9) | 0x3f80_0000) - 1.0
            };
            [next() * 64.0, next() * 64.0 - 32.0, next() * 9.0 + 0.001, next()]
        })
        .collect()
}

/// The hard tip's arithmetic rounds as the Mac GPU's does, on any adapter, including ones whose
/// own `fma` isn't fused.
#[test]
fn exact_helpers_round_to_nearest() {
    let gpu = Gpu::new().unwrap();
    let cases = random(1 << 14);
    let check = |name: &'static str, body: &str, want: &dyn Fn([f32; 4]) -> f32| {
        let got = eval(&gpu, name, body, &cases);
        let wrong: Vec<_> = cases.iter().zip(&got).filter(|(c, g)| want(**c).to_bits() != g.to_bits()).collect();
        assert!(wrong.is_empty(), "{name}: {} of {} wrong, e.g. {:?}", wrong.len(), cases.len(), &wrong[..wrong.len().min(3)]);
    };
    check("paint.test.fused", "out = fused(v.x, v.y, v.z);", &|v| v[0].mul_add(v[1], v[2]));
    check("paint.test.quotient", "out = quotient_rn(v.x, v.z);", &|v| v[0] / v[2]);
    check("paint.test.root", "out = root_rn(v.z);", &|v| v[2].sqrt());
    check("paint.test.rsqrt", "out = rsqrt_rn(v.z);", &|v| (1.0 / (v[2] as f64).sqrt()) as f32);
}
