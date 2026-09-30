//! Core Graphics' fills and gradients, on the GPU.

use super::Result;
use super::gradient::Fill;
use crate::RenderError;
use crate::gpu::Gpu;
use comp_format::ShapeStyle;

/// The shape filling a `width` x `height` box, as `EditorSession.shapeImage` draws it, straight RGBA.
pub fn shape_image(_gpu: &Gpu, _style: &ShapeStyle, _width: u32, _height: u32) -> Result<image::RgbaImage> {
    Err(RenderError::Unsupported("drawing shapes".into()))
}

/// `fill` drawn over `base` (premultiplied, `width` x `height`) inside `region`, whose top-left
/// pixel sits at `offset` on the document.
pub fn gradient_over(_gpu: &Gpu, _base: &[u8], _width: u32, _height: u32, _region: [i64; 4], _offset: [f64; 2], _fill: &Fill) -> Result<Vec<u8>> {
    Err(RenderError::Unsupported("drawing gradients".into()))
}
