//! Filter > Dither (`DitherSettings.apply`, Document/Dither.swift, and `dither_apply`,
//! Rendering/DitherPixels.c).

use super::settings::DitherSettings;
use crate::RenderError;
use crate::gpu::{Gpu, GpuImage};

pub fn apply(_gpu: &Gpu, _image: &GpuImage, _s: &DitherSettings) -> Result<GpuImage, RenderError> {
    Err(RenderError::Unsupported("Dither".into()))
}
