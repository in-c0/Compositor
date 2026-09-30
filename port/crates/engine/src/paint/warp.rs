//! Smudge and Liquify (`WarpStroke`, `MetalWarp`), then `finishWarp`.

use super::{Result, Tip};
use crate::RenderError;
use crate::gpu::Gpu;
use comp_format::Project;

pub fn apply(_gpu: &Gpu, _project: &mut Project, _layer: &str, _smudge: bool, _tip: Tip, _points: &[[f64; 2]]) -> Result<()> {
    Err(RenderError::Unsupported("Smudge and Liquify".into()))
}
