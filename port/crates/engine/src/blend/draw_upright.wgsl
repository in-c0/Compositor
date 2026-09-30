// Draws a layer placed 1:1 and upright at a whole-pixel offset: what Core Graphics does when a
// layer's size equals its image and it isn't rotated (interpolation .none, a straight pixel copy),
// composited over the canvas in the layer's blend mode.

struct Params {
    canvas_width: u32,
    canvas_height: u32,
    layer_width: u32,
    layer_height: u32,
    offset_x: i32,
    offset_y: i32,
    mode: u32,
    // 1 when `mask` holds coverage per layer pixel (the layer's own mask).
    has_mask: u32,
    // 1 when `clip` holds coverage per canvas pixel (clipping-mask coverage, folder masks).
    has_clip: u32,
    // 1 when `layer` is already premultiplied (a surface), 0 for straight PNG pixels.
    premultiplied: u32,
    // 1 when the layer is drawn at full opacity, which picks Multiply's integer path.
    full_opacity: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> canvas_in: array<u32>;
@group(0) @binding(2) var<storage, read> layer: array<u32>;
@group(0) @binding(3) var<storage, read_write> canvas_out: array<u32>;
@group(0) @binding(4) var<storage, read> opacity_table: array<u32, 256>;
@group(0) @binding(5) var<storage, read> mask: array<u32>;
@group(0) @binding(6) var<storage, read> clip: array<u32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.canvas_width || id.y >= params.canvas_height {
        return;
    }
    let index = id.y * params.canvas_width + id.x;
    let backdrop = canvas_in[index];
    let lx = i32(id.x) - params.offset_x;
    let ly = i32(id.y) - params.offset_y;
    if lx < 0 || ly < 0 || lx >= i32(params.layer_width) || ly >= i32(params.layer_height) {
        canvas_out[index] = backdrop;
        return;
    }
    let layer_index = u32(ly) * params.layer_width + u32(lx);
    var pixel = unpack(layer[layer_index]);
    if params.premultiplied == 0u {
        pixel = vec4<u32>(div255v(vec4<u32>(pixel.xyz * pixel.w, 0u)).xyz, pixel.w);
    }
    var source = scale_premultiplied(pixel);
    // Clips come after opacity: the layer's mask, then any clip on the context, each scaling the
    // premultiplied bytes by coverage / 255 and rounding.
    var coverage = 255u;
    if params.has_mask != 0u {
        coverage = mask[layer_index];
    }
    if params.has_clip != 0u {
        coverage = div255(coverage * clip[index]);
    }
    if coverage != 255u {
        source = div255v(source * coverage);
    }
    canvas_out[index] = pack(composite(params.mode, unpack(backdrop), source, params.full_opacity != 0u));
}
