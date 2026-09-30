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
    opacity: f32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> canvas_in: array<u32>;
@group(0) @binding(2) var<storage, read> layer: array<u32>;
@group(0) @binding(3) var<storage, read_write> canvas_out: array<u32>;

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
    let source = scale_source(unpack(layer[u32(ly) * params.layer_width + u32(lx)]), params.opacity);
    canvas_out[index] = pack(composite(params.mode, unpack(backdrop), source, params.opacity >= 1.0));
}
