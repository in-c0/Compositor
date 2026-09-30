use super::*;
use std::path::PathBuf;

/// Runs `body` (WGSL statements reading `v: vec4<f32>` and writing `out: f32`) over `inputs`.
fn eval(gpu: &Gpu, name: &'static str, body: &str, inputs: &[[f32; 4]]) -> Vec<f32> {
    let source = format!(
        "{FLOAT}
struct Params {{ guard: f32, width: u32, height: u32 }}
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
}}"
    );
    let n = inputs.len() as u32;
    let width = 256;
    let input = storage(gpu, bytemuck::cast_slice(inputs));
    let output = gpu.image(n, 1);
    let pipeline = gpu.pipeline(name, &source);
    let words = [f32::INFINITY.to_bits(), width, 0];
    gpu.dispatch(&pipeline, bytemuck::cast_slice(&words), &[&input, &output.buffer], width, n.div_ceil(width));
    bytemuck::cast_slice(&gpu.download(&output).unwrap()).to_vec()
}

fn random(count: usize, mut f: impl FnMut(&mut dyn FnMut() -> u64) -> [f32; 4]) -> Vec<[f32; 4]> {
    let mut state = 0x9E3779B97F4A7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..count).map(|_| f(&mut next)).collect()
}

/// The float helpers round exactly as the Mac's libm and IEEE arithmetic do.
#[test]
fn float_helpers_are_correctly_rounded() {
    let gpu = Gpu::new().unwrap();
    let cases = random(1 << 16, |next| {
        let a = (next() % 65026) as f32;
        let b = (next() % 255 + 1) as f32;
        let u = (next() >> 40) as f32 * (1.0 / 16777216.0);
        let x = f32::from_bits((next() as u32 >> 9) | 0x3f00_0000) * 300.0;
        [a, b, u, x]
    });
    let check = |name: &'static str, body: &str, want: &dyn Fn([f32; 4]) -> f32| {
        let got = eval(&gpu, name, body, &cases);
        let wrong: Vec<_> = cases.iter().zip(&got).filter(|(c, g)| want(**c).to_bits() != g.to_bits()).collect();
        assert!(wrong.is_empty(), "{name}: {} of {} wrong, e.g. {:?}", wrong.len(), cases.len(), &wrong[..wrong.len().min(3)]);
    };
    check("test.div", "out = div(v.x, v.y);", &|v| v[0] / v[1]);
    check("test.div2", "out = div(v.w, v.y);", &|v| v[3] / v[1]);
    check("test.root", "out = root(v.w);", &|v| v[3].sqrt());
    check("test.log", "out = log_rn(1.0 - v.z);", &|v| ((1.0 - v[2]) as f64).ln() as f32);
    check("test.cos", "out = cos_rn(keep(6.2831853 * v.z));", &|v| ((6.2831853f32 * v[2]) as f64).cos() as f32);
    check("test.round", "out = round_away(floor(v.w) + 0.5);", &|v| (v[3].floor() + 0.5).round());
    check("test.round2", "out = round_away(v.w);", &|v| v[3].round());
}

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../parity/corpus")
}

/// Straight alpha, as the export's PNG stores it.
fn unpremultiply(p: &[u8]) -> [u8; 4] {
    let a = p[3] as u32;
    if a == 0 {
        return [0; 4];
    }
    let c = |v: u8| ((v as u32 * 255 + a / 2) / a).min(255) as u8;
    [c(p[0]), c(p[1]), c(p[2]), p[3]]
}

fn premultiply(p: &[u8]) -> [u8; 4] {
    let a = p[3] as u32;
    let c = |v: u8| ((v as u32 * a + 127) / 255) as u8;
    [c(p[0]), c(p[1]), c(p[2]), p[3]]
}

/// The blur cases still outside 1/255, with the largest channel difference measured on an RTX 2080
/// (DX12). The test fails if any of them gets worse; see `blur.rs` for what is and isn't matched.
/// Streaks off the axes turn the image with bilinear reads whose 8-bit fractions depend on how
/// the Mac's GPU rounds texture coordinates, which isn't modeled, so a few pixels near rounding
/// boundaries land one level off (two or three once unpremultiplied at low alpha).
const KNOWN_GAPS: &[(&str, u8)] = &[("motion-45-20", 2), ("probe-motion-45-20", 2), ("probe-motion-45-40", 3)];

/// Every plain adjust case: an opaque photo drawn onto a transparent canvas, the adjustment
/// applied to the whole canvas. Set `PARITY_REFS` to the downloaded references (the folder
/// holding `adjust/<case>.png`). Prints each case's largest channel difference and fails when a
/// plain case is off by more than 1/255, apart from the known blur gaps. The modifier cases
/// (opacity, blend mode, mask, translucent layers) also need the compositing around the
/// adjustment, so they are printed for information only.
#[test]
fn adjust_cases_match_references() {
    let Some(refs) = std::env::var_os("PARITY_REFS").map(PathBuf::from) else {
        eprintln!("PARITY_REFS not set; skipping");
        return;
    };
    let filter = std::env::var("ADJUST_CASES").unwrap_or_default();
    let gpu = Gpu::new().unwrap();
    let mut failures = Vec::new();
    let mut dirs: Vec<_> = std::fs::read_dir(corpus().join("adjust")).unwrap().map(|e| e.unwrap().path()).collect();
    dirs.sort();
    for dir in dirs {
        let id = dir.file_name().unwrap().to_string_lossy().to_string();
        if !id.contains(&filter) {
            continue;
        }
        let project = comp_format::load(&dir.join("input.comp")).unwrap();
        let layers = &project.manifest.layers;
        let plain = layers.len() == 2
            && layers[1].opacity() == 1.0
            && layers[1].blend_mode() == comp_format::BlendMode::Normal
            && layers[1].mask_file.is_none();
        let photo = &project.images[&layers[0].id].pixels;
        let opaque = photo.pixels().all(|p| p[3] == 255);
        let Ok(reference) = image::open(refs.join(format!("adjust/{id}.png"))) else { continue };
        let reference = reference.to_rgba8();
        let (w, h) = (project.manifest.width as u32, project.manifest.height as u32);
        assert_eq!((photo.width(), photo.height()), (w, h));
        let canvas: Vec<u8> = photo.pixels().flat_map(|p| premultiply(&p.0)).collect();
        let image = gpu.upload(w, h, &canvas);
        let adjustment = layers[1].adjustment.as_ref().unwrap();
        if let Some(what) = unsupported(adjustment, Region::whole(&image)) {
            println!("{id:<24} pending: {what}");
            continue;
        }
        let out = apply(&gpu, &image, adjustment, Region::whole(&image)).unwrap();
        let out = gpu.download(&out).unwrap();
        let (mut max, mut count) = (0u8, 0usize);
        for (got, want) in out.chunks(4).zip(reference.pixels()) {
            let got = unpremultiply(got);
            if got[3] == 0 && want[3] == 0 {
                continue;
            }
            let d = (0..4).map(|c| got[c].abs_diff(want[c])).max().unwrap();
            max = max.max(d);
            count += (d != 0) as usize;
        }
        let kind = if plain && opaque { "plain" } else { "modifier" };
        println!("{id:<24} {kind:<8} max {max:>3}  differing pixels {count:>5} of {}", w * h);
        let allowed = KNOWN_GAPS.iter().find(|(case, _)| *case == id).map_or(1, |(_, max)| *max);
        if plain && opaque && max > allowed {
            failures.push(id);
        }
    }
    assert!(failures.is_empty(), "worse than allowed: {failures:?}");
}
