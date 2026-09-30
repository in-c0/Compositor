//! The flattened document, drawn as the Mac app's export draws it (`ImageExporter.render` and
//! `LiveMaskRenderer`): layers in folder order, each clipped by its folders' masks; clipping
//! stacks drawn together on a surface; lone clipped layers clipped to their base's coverage.

use crate::blend::{self, Draw};
use crate::gpu::{Gpu, GpuImage};
use crate::order::{self, Stacks};
use crate::{RenderError, mask};
use comp_format::{LayerRecord, Project};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub struct Compositor<'a> {
    gpu: &'a Gpu,
    project: &'a Project,
    by_id: HashMap<&'a str, &'a LayerRecord>,
    width: u32,
    height: u32,
    stacks: Stacks,
    /// Each clipping base's alpha coverage, drawn once per render.
    coverage: RefCell<HashMap<String, Rc<wgpu::Buffer>>>,
}

type Result<T> = std::result::Result<T, RenderError>;

fn unsupported<T>(what: &str) -> Result<T> {
    Err(RenderError::Unsupported(what.to_string()))
}

impl<'a> Compositor<'a> {
    pub fn new(gpu: &'a Gpu, project: &'a Project) -> Self {
        let m = &project.manifest;
        Self {
            gpu,
            project,
            by_id: m.layers.iter().map(|l| (l.id.as_str(), l)).collect(),
            width: m.width as u32,
            height: m.height as u32,
            stacks: order::prepare_stacks(&order::visible_layers(&m.layers)),
            coverage: RefCell::new(HashMap::new()),
        }
    }

    /// The flattened canvas, premultiplied.
    pub fn render(&self) -> Result<GpuImage> {
        let mut canvas = self.gpu.image(self.width, self.height);
        for layer in order::visible_layers(&self.project.manifest.layers) {
            let clip = self.folder_clip(layer)?;
            canvas = self.draw_composite(layer, canvas, clip.as_ref())?;
        }
        Ok(canvas)
    }

    /// `LiveMaskRenderer.drawComposite`.
    fn draw_composite(&self, layer: &LayerRecord, target: GpuImage, clip: Option<&wgpu::Buffer>) -> Result<GpuImage> {
        if self.stacks.stacked.contains(&layer.id) {
            return Ok(target);
        }
        if layer.adjustment.is_some() {
            // A clipped adjustment only ever draws inside its base's stack.
            if layer.mask_source_id.is_some() {
                return Ok(target);
            }
            return self.adjust(layer, target, clip);
        }
        let Some(children) = self.stacks.stacks.get(&layer.id) else {
            return self.draw(layer, target, clip);
        };
        // The base on a surface of its own; its alpha is set aside and the surface made opaque, the
        // clipped layers drawn over it, then the alpha put back.
        let mut surface = self.draw_own(layer, self.gpu.image(self.width, self.height), None)?;
        let alpha = blend::extract_alpha(self.gpu, &surface);
        blend::unpremultiply_opaque(self.gpu, &surface);
        for child in children {
            let child = self.by_id[child.as_str()];
            surface = if child.adjustment.is_some() { self.adjust(child, surface, None)? } else { self.draw_own(child, surface, None)? };
        }
        blend::restore_alpha(self.gpu, &surface, &alpha);
        // The stack blends in its base's mode.
        let draw = Draw {
            pixels: &surface,
            premultiplied: true,
            offset: (0, 0),
            mode: self.stacks.modes[&layer.id],
            opacity: 1.0,
            mask: None,
            clip,
        };
        Ok(blend::draw_upright(self.gpu, &target, &draw))
    }

    /// `LiveMaskRenderer.draw`: the layer, clipped to its base's coverage when it has one.
    fn draw(&self, layer: &LayerRecord, target: GpuImage, clip: Option<&wgpu::Buffer>) -> Result<GpuImage> {
        let Some(source) = layer.mask_source_id.as_deref() else {
            return self.draw_own(layer, target, clip);
        };
        let coverage = self.coverage_of(source)?;
        let combined = match clip {
            Some(clip) => Some(mask::multiply(self.gpu, clip, &coverage, self.width, self.height)),
            None => None,
        };
        self.draw_own(layer, target, Some(combined.as_ref().unwrap_or(&coverage)))
    }

    /// A clipping base's alpha: the base drawn alone, with its opacity, mask and own clipping,
    /// whether or not it is visible.
    fn coverage_of(&self, id: &str) -> Result<Rc<wgpu::Buffer>> {
        if let Some(c) = self.coverage.borrow().get(id) {
            return Ok(c.clone());
        }
        let Some(layer) = self.by_id.get(id) else {
            return unsupported("a missing clipping base");
        };
        let surface = self.draw(layer, self.gpu.image(self.width, self.height), None)?;
        let alpha = Rc::new(blend::extract_alpha(self.gpu, &surface));
        self.coverage.borrow_mut().insert(id.to_string(), alpha.clone());
        Ok(alpha)
    }

    /// `LayerRenderer.draw` for one layer, in its own mode and opacity, through its own mask.
    fn draw_own(&self, layer: &LayerRecord, target: GpuImage, clip: Option<&wgpu::Buffer>) -> Result<GpuImage> {
        if layer.effects.is_some() {
            return unsupported("layer effects");
        }
        let Some(asset) = self.project.images.get(&layer.id) else {
            return Ok(target);
        };
        let t = &layer.transform;
        let (w, h) = asset.pixels.dimensions();
        let upright = t.rotation == 0.0
            && !t.flip_x
            && !t.flip_y
            && t.size == [w as f64, h as f64]
            && t.origin[0].fract() == 0.0
            && t.origin[1].fract() == 0.0;
        if !upright {
            return unsupported("transformed layers");
        }
        let own_mask = mask::layer_coverage(self.project, layer, (w, h)).map_err(RenderError::Unsupported)?;
        let own_mask = own_mask.map(|c| self.gpu.bytes(bytemuck::cast_slice(&c)));
        let pixels = self.gpu.upload(w, h, asset.pixels.as_raw());
        let draw = Draw {
            pixels: &pixels,
            premultiplied: false,
            offset: (t.origin[0] as i32, t.origin[1] as i32),
            mode: layer.blend_mode(),
            opacity: order::effective_opacity(layer, &self.by_id),
            mask: own_mask.as_ref(),
            clip,
        };
        Ok(blend::draw_upright(self.gpu, &target, &draw))
    }

    /// `LiveMaskRenderer.adjust`: the adjustment runs on everything drawn so far, is blended in the
    /// layer's mode and mixed at its opacity, then copied back through its mask and `clip`.
    fn adjust(&self, layer: &LayerRecord, target: GpuImage, clip: Option<&wgpu::Buffer>) -> Result<GpuImage> {
        let gpu = self.gpu;
        let settings = layer.adjustment.as_ref().expect("an adjustment layer");
        // Core Image's blurs aren't reproduced exactly yet (see feature/ci-blur).
        if matches!(settings.kind, comp_format::AdjustmentKind::GaussianBlur | comp_format::AdjustmentKind::MotionBlur) {
            return unsupported("Gaussian and Motion Blur adjustment layers");
        }
        let region = crate::adjust::Region::whole(&target);
        let mut adjusted = crate::adjust::apply(gpu, &target, settings, region)?;
        let mode = layer.blend_mode();
        if mode != comp_format::BlendMode::Normal {
            // Colors blend at full coverage, then the original alpha comes back, so soft edges
            // don't thicken.
            let base = gpu.copy(&target);
            let alpha = blend::extract_alpha(gpu, &base);
            blend::unpremultiply_opaque(gpu, &base);
            blend::unpremultiply_opaque(gpu, &adjusted);
            let draw = Draw { pixels: &adjusted, premultiplied: true, offset: (0, 0), mode, opacity: 1.0, mask: None, clip: None };
            let blended = blend::draw_upright(gpu, &base, &draw);
            blend::restore_alpha(gpu, &blended, &alpha);
            adjusted = blended;
        }
        let opacity = order::effective_opacity(layer, &self.by_id);
        if opacity < 1.0 {
            adjusted = self.mix(&target, &adjusted, None, 0, opacity);
        }
        let own = if layer.mask_enabled() {
            Some(mask::folder_coverage(self.project, layer, (self.width, self.height)).map_err(RenderError::Unsupported)?)
        } else {
            None
        };
        let own = own.map(|c| gpu.bytes(bytemuck::cast_slice(&c)));
        let coverage = match (own.as_ref(), clip) {
            (Some(a), Some(b)) => Some(mask::multiply(gpu, b, a, self.width, self.height)),
            (Some(a), None) => Some(gpu.copy_buffer(a)),
            (None, Some(b)) => Some(gpu.copy_buffer(b)),
            (None, None) => None,
        };
        Ok(match coverage {
            Some(c) => self.mix(&target, &adjusted, Some(&c), 1, 1.0),
            None => adjusted,
        })
    }

    fn mix(&self, original: &GpuImage, adjusted: &GpuImage, coverage: Option<&wgpu::Buffer>, step: u32, opacity: f64) -> GpuImage {
        let gpu = self.gpu;
        let pipeline = gpu.pipeline("adjust_mix", include_str!("adjust_mix.wgsl"));
        let out = gpu.image(self.width, self.height);
        let params = [self.width.to_le_bytes(), self.height.to_le_bytes(), step.to_le_bytes(), (opacity as f32).to_le_bytes()].concat();
        let placeholder = gpu.bytes(&[0u8; 4]);
        gpu.dispatch(&pipeline, &params, &[&original.buffer, &adjusted.buffer, coverage.unwrap_or(&placeholder), &out.buffer], self.width, self.height);
        out
    }

    /// Every enclosing folder's enabled mask, multiplied together, as coverage per canvas pixel.
    fn folder_clip(&self, layer: &LayerRecord) -> Result<Option<wgpu::Buffer>> {
        let mut combined: Option<Vec<u32>> = None;
        for folder in order::folders(layer, &self.by_id) {
            if !folder.mask_enabled() {
                continue;
            }
            let coverage = mask::folder_coverage(self.project, folder, (self.width, self.height)).map_err(RenderError::Unsupported)?;
            combined = Some(match combined {
                None => coverage,
                Some(prev) => prev.iter().zip(&coverage).map(|(a, b)| (a * b + 127) / 255).collect(),
            });
        }
        Ok(combined.map(|c| self.gpu.bytes(bytemuck::cast_slice(&c))))
    }
}
