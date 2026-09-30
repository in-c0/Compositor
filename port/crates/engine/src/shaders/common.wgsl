// Shared by every kernel. Pixels are RGBA8 packed into a u32 with R in the low byte.

fn unpack(p: u32) -> vec4<u32> {
    return vec4<u32>(p & 255u, (p >> 8u) & 255u, (p >> 16u) & 255u, p >> 24u);
}

fn pack(c: vec4<u32>) -> u32 {
    let m = min(c, vec4<u32>(255u));
    return m.x | (m.y << 8u) | (m.z << 16u) | (m.w << 24u);
}

// x / 255, rounded to nearest; exact for every product of two bytes.
fn div255(x: u32) -> u32 {
    let t = x + 128u;
    return (t + (t >> 8u)) >> 8u;
}

fn div255v(x: vec4<u32>) -> vec4<u32> {
    let t = x + vec4<u32>(128u);
    return (t + (t >> vec4<u32>(8u))) >> vec4<u32>(8u);
}

// Float in 0...1 to a byte, rounded to nearest, as Core Graphics and Core Image store results.
fn to_byte(v: f32) -> u32 {
    return u32(clamp(floor(v * 255.0 + 0.5), 0.0, 255.0));
}

fn to_bytes(v: vec4<f32>) -> vec4<u32> {
    return vec4<u32>(clamp(floor(v * 255.0 + vec4<f32>(0.5)), vec4<f32>(0.0), vec4<f32>(255.0)));
}
