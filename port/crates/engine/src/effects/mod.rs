//! Layer effects (stroke, drop shadow, color overlay, inner shadow, outer glow, inner glow), as the
//! Mac's `LayerEffectsRenderer` draws them: the layer's pixels through its mask, on an image
//! grown by `inset` pixels on every side, with its effects composited around and over them. The
//! caller draws that image in place of the layer, grown by the same inset (`placed`), which for a
//! layer drawn 1:1 and upright is the same 1:1 draw moved up and left by the inset.
//!
//! The Mac makes the image with Metal compute kernels (`MetalLayerEffects`) whenever it has a
//! Metal device, which it always has; each kernel here is one of those, with the same float
//! operations in the same order. Where Metal's fast math leaves the rounding open (it fuses
//! multiply-adds and may approximate `/` and `exp`), the port fuses and rounds correctly. Which
//! product of `compose`'s color sums is fused decides one exact tie in effects/fractional (see
//! `over` in compose.wgsl); otherwise, modeled on the CPU, the cases drawn over transparency come
//! out the same byte for byte fused or not, with `/` as a reciprocal multiply, and with `exp` through `exp2`.

use crate::gpu::{Gpu, GpuImage};
use comp_format::{ColorOverlayEffect, Effects, GlowEffect, ShadowEffect, StrokeEffect};

const FLOAT: &str = include_str!("../adjust/float.wgsl");

/// The grown image, premultiplied, and how far it reaches past the layer on every side.
pub struct Built {
    pub image: GpuImage,
    pub inset: u32,
}

fn enabled(flag: Option<bool>) -> bool {
    flag.unwrap_or(true)
}

/// The effects the Mac draws for a layer (`LayerEffectsRenderer.cached`): the enabled ones, or
/// `None` when there are none or any is out of range, and the layer is drawn as it is.
pub fn shown(effects: &Effects) -> Option<Effects> {
    let visible = Effects {
        stroke: effects.stroke.filter(|e| enabled(e.enabled)),
        shadow: effects.shadow.filter(|e| enabled(e.enabled)),
        color_overlay: effects.color_overlay.filter(|e| enabled(e.enabled)),
        inner_shadow: effects.inner_shadow.filter(|e| enabled(e.enabled)),
        outer_glow: effects.outer_glow.filter(|e| enabled(e.enabled)),
        inner_glow: effects.inner_glow.filter(|e| enabled(e.enabled)),
    };
    let empty = visible.stroke.is_none()
        && visible.shadow.is_none()
        && visible.color_overlay.is_none()
        && visible.inner_shadow.is_none()
        && visible.outer_glow.is_none()
        && visible.inner_glow.is_none();
    (!empty && is_valid(&visible)).then_some(visible)
}

fn unit(v: f64) -> bool {
    v.is_finite() && (0.0..=1.0).contains(&v)
}

fn colors(rgb: [f64; 3], opacity: f64) -> bool {
    unit(opacity) && rgb.iter().all(|&c| unit(c))
}

fn stroke_valid(e: &StrokeEffect) -> bool {
    e.size.is_finite() && (0.0..=500.0).contains(&e.size) && colors([e.red, e.green, e.blue], e.opacity)
}

fn shadow_valid(e: &ShadowEffect) -> bool {
    [e.angle, e.distance, e.blur].iter().all(|v| v.is_finite())
        && (-360.0..=360.0).contains(&e.angle)
        && (0.0..=5000.0).contains(&e.distance)
        && (0.0..=500.0).contains(&e.blur)
        && colors([e.red, e.green, e.blue], e.opacity)
}

fn overlay_valid(e: &ColorOverlayEffect) -> bool {
    colors([e.red, e.green, e.blue], e.opacity)
}

fn glow_valid(e: &GlowEffect) -> bool {
    e.size.is_finite() && (0.0..=500.0).contains(&e.size) && colors([e.red, e.green, e.blue], e.opacity)
}

/// `LayerEffects.isValid`.
fn is_valid(e: &Effects) -> bool {
    e.stroke.as_ref().is_none_or(stroke_valid)
        && e.shadow.as_ref().is_none_or(shadow_valid)
        && e.color_overlay.as_ref().is_none_or(overlay_valid)
        && e.inner_shadow.as_ref().is_none_or(shadow_valid)
        && e.outer_glow.as_ref().is_none_or(glow_valid)
        && e.inner_glow.as_ref().is_none_or(glow_valid)
}

/// `LayerEffectsRenderer.margin`: room for an outside stroke, the shadow's reach and three sigma
/// of its blur, and the outer glow, plus two pixels. Effects at zero opacity still count.
pub fn margin(e: &Effects) -> u32 {
    let mut margin = 0.0f64;
    if let Some(stroke) = e.stroke.filter(|s| !s.inside) {
        margin = margin.max(stroke.size);
    }
    if let Some(shadow) = e.shadow {
        margin = margin.max(shadow.distance + shadow.blur * 3.0);
    }
    if let Some(glow) = e.outer_glow {
        margin = margin.max(glow.size * 3.0);
    }
    margin.ceil() as u32 + 2
}

/// `LayerEffectsRenderer.placed`: the layer's transform grown about its center by the effects'
/// margin, so the `width` x `height` effects image lands where the layer is.
pub fn placed(t: &comp_format::Transform, width: u32, height: u32, inset: u32) -> comp_format::Transform {
    let (w, h, inset) = (width as f64, height as f64, inset as f64);
    if w <= inset * 2.0 || h <= inset * 2.0 {
        return *t;
    }
    let mut grown = *t;
    grown.size = [t.size[0] * w / (w - inset * 2.0), t.size[1] * h / (h - inset * 2.0)];
    let center = [t.origin[0] + t.size[0] / 2.0, t.origin[1] + t.size[1] / 2.0];
    grown.origin = [center[0] - grown.size[0] / 2.0, center[1] - grown.size[1] / 2.0];
    grown
}

/// `ShadowEffect.offset`: where the shadow falls, in layer pixels, y down. The light comes from
/// `angle` degrees counterclockwise from the right, and the shadow falls away from it.
fn offset(angle: f64, distance: f64) -> (f32, f32) {
    let radians = angle * std::f64::consts::PI / 180.0;
    ((-radians.cos() * distance) as f32, (radians.sin() * distance) as f32)
}

/// The Gaussian's sigma for a blur or glow `size`, as Metal gets it: `Float(size / 2)`.
fn sigma(size: f64) -> f32 {
    (size / 2.0) as f32
}

/// The weights `effects_blur_*` computes for every tap, exp(-k² / (2σ·σ)) in `Float`, for
/// k = -radius…radius, with radius = max(1, round(3σ)).
fn weights(sigma: f32) -> Vec<f32> {
    let radius = ((sigma * 3.0).round() as i64).max(1);
    let spread = 2.0f32 * sigma * sigma;
    (-radius..=radius).map(|k| ((-((k * k) as f32) / spread) as f64).exp() as f32).collect()
}

fn words(values: &[u32]) -> Vec<u8> {
    bytemuck::cast_slice(values).to_vec()
}

struct Run<'a> {
    gpu: &'a Gpu,
    width: u32,
    height: u32,
}

impl Run<'_> {
    /// A float per pixel.
    fn floats(&self) -> wgpu::Buffer {
        self.gpu.image(self.width, self.height).buffer
    }

    fn kernel(&self, name: &'static str, source: &str, params: &[u32], buffers: &[&wgpu::Buffer]) {
        let pipeline = self.gpu.pipeline(name, &format!("{FLOAT}\n{source}"));
        let mut all = vec![f32::INFINITY.to_bits(), self.width, self.height];
        all.extend_from_slice(params);
        self.gpu.dispatch(&pipeline, &words(&all), buffers, self.width, self.height);
    }

    fn spread(&self, source: &wgpu::Buffer, result: &wgpu::Buffer, reach: u32, smallest: bool, axis: u32) {
        self.kernel("effects.spread", include_str!("spread.wgsl"), &[reach, smallest as u32, axis], &[source, result]);
    }

    fn shift(&self, source: &wgpu::Buffer, result: &wgpu::Buffer, (dx, dy): (f32, f32)) {
        self.kernel("effects.shift", include_str!("shift.wgsl"), &[dx.to_bits(), dy.to_bits()], &[source, result]);
    }

    fn blur_pass(&self, source: &wgpu::Buffer, result: &wgpu::Buffer, weights: &wgpu::Buffer, radius: u32, axis: u32) {
        self.kernel("effects.blur", include_str!("blur.wgsl"), &[radius, axis], &[source, result, weights]);
    }

    /// The row pass from `source` into `scratch`, then the column pass into `result`.
    fn blur(&self, sigma: f32, source: &wgpu::Buffer, scratch: &wgpu::Buffer, result: &wgpu::Buffer) {
        let weights = weights(sigma);
        let radius = (weights.len() / 2) as u32;
        let weights = self.gpu.bytes(bytemuck::cast_slice(&weights));
        self.blur_pass(source, scratch, &weights, radius, 0);
        self.blur_pass(scratch, result, &weights, radius, 1);
    }

    /// Step 0 is `effects_ring`, step 1 `effects_inside`.
    fn combine(&self, step: u32, smallest: bool, shape: &wgpu::Buffer, moved: &wgpu::Buffer, result: &wgpu::Buffer) {
        self.kernel("effects.combine", include_str!("combine.wgsl"), &[step, smallest as u32], &[shape, moved, result]);
    }
}

fn color(rgb: [f64; 3], opacity: f64) -> [f32; 4] {
    [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, opacity as f32]
}

/// `LayerEffectsRenderer.render` through `MetalLayerEffects.render`. `pixels` are the layer's
/// straight PNG pixels and `mask` its own mask as coverage per layer pixel; `effects` come from
/// [`shown`].
pub fn render(gpu: &Gpu, pixels: &GpuImage, mask: Option<&wgpu::Buffer>, effects: &Effects) -> Built {
    let inset = margin(effects);
    let (width, height) = (pixels.width + inset * 2, pixels.height + inset * 2);
    let run = Run { gpu, width, height };

    // The pixels with room around them, and first: the shape's own coverage.
    let input = gpu.image(width, height);
    let first = run.floats();
    let placeholder = gpu.bytes(&[0u8; 4]);
    run.kernel(
        "effects.alpha",
        include_str!("alpha.wgsl"),
        &[pixels.width, pixels.height, inset, mask.is_some() as u32],
        &[&pixels.buffer, mask.unwrap_or(&placeholder), &input.buffer, &first],
    );
    let second = run.floats();
    let third = run.floats();

    let stroke = effects.stroke.filter(|s| s.size > 0.0 && s.opacity > 0.0);
    if let Some(stroke) = stroke {
        // second: the shape reached out (or pulled in) by the stroke's size; third: the ring.
        let reach = (stroke.size.round() as i64).max(1) as u32;
        run.spread(&first, &third, reach, stroke.inside, 0);
        run.spread(&third, &second, reach, stroke.inside, 1);
        run.combine(0, stroke.inside, &first, &second, &third);
    }
    let shadow = effects.shadow.filter(|s| s.opacity > 0.0);
    if let Some(shadow) = shadow {
        // second: the shape moved and softened.
        run.shift(&first, &second, offset(shadow.angle, shadow.distance));
        let sigma = sigma(shadow.blur);
        if sigma > 0.01 {
            run.blur(sigma, &second, &run.floats(), &second);
        }
    }
    let overlay = effects.color_overlay.filter(|o| o.opacity > 0.0);
    let inner_shadow = effects.inner_shadow.filter(|s| s.opacity > 0.0);
    let inner = run.floats();
    if let Some(inner_shadow) = inner_shadow {
        // What lies outside the layer, moved and softened, kept to the layer's own shape.
        let moved = run.floats();
        run.shift(&first, &moved, offset(inner_shadow.angle, inner_shadow.distance));
        let sigma = sigma(inner_shadow.blur);
        if sigma > 0.01 {
            run.blur(sigma, &moved, &run.floats(), &moved);
        }
        run.combine(1, false, &first, &moved, &inner);
    }
    let glow = effects.outer_glow.filter(|g| g.size > 0.0 && g.opacity > 0.0);
    let glow_output = run.floats();
    if let Some(glow) = glow {
        let sigma = sigma(glow.size);
        if sigma > 0.01 {
            run.blur(sigma, &first, &run.floats(), &glow_output);
        } else {
            run.shift(&first, &glow_output, (0.0, 0.0));
        }
    }
    let inner_glow = effects.inner_glow.filter(|g| g.size > 0.0 && g.opacity > 0.0);
    let inner_glow_output = run.floats();
    if let Some(inner_glow) = inner_glow {
        let blurred = run.floats();
        let sigma = sigma(inner_glow.size);
        if sigma > 0.01 {
            run.blur(sigma, &first, &run.floats(), &blurred);
        } else {
            run.shift(&first, &blurred, (0.0, 0.0));
        }
        run.combine(1, false, &first, &blurred, &inner_glow_output);
    }

    let none = [0.0f32; 4];
    let flags = stroke.is_some() as u32
        | (stroke.is_some_and(|s| s.inside) as u32) << 1
        | (shadow.is_some() as u32) << 2
        | (inner_shadow.is_some() as u32) << 3
        | (overlay.is_some() as u32) << 4
        | (glow.is_some() as u32) << 5
        | (inner_glow.is_some() as u32) << 6;
    let colors = [
        stroke.map_or(none, |s| color([s.red, s.green, s.blue], s.opacity)),
        shadow.map_or(none, |s| color([s.red, s.green, s.blue], s.opacity)),
        overlay.map_or(none, |o| color([o.red, o.green, o.blue], o.opacity)),
        inner_shadow.map_or(none, |s| color([s.red, s.green, s.blue], s.opacity)),
        glow.map_or(none, |g| color([g.red, g.green, g.blue], g.opacity)),
        inner_glow.map_or(none, |g| color([g.red, g.green, g.blue], g.opacity)),
    ];
    let mut params = vec![flags];
    params.extend(colors.iter().flatten().map(|c| c.to_bits()));
    let output = gpu.image(width, height);
    run.kernel(
        "effects.compose",
        include_str!("compose.wgsl"),
        &params,
        &[&input.buffer, &third, &second, &output.buffer, &inner, &first, &glow_output, &inner_glow_output],
    );
    Built { image: output, inset }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margin_counts_outside_reach_only() {
        let mut e = Effects::default();
        e.stroke = Some(StrokeEffect { enabled: None, size: 4.5, red: 0.0, green: 0.0, blue: 0.0, opacity: 1.0, inside: true });
        assert_eq!(margin(&e), 2);
        e.stroke.as_mut().unwrap().inside = false;
        assert_eq!(margin(&e), 7);
        e.shadow = Some(ShadowEffect { enabled: None, angle: 90.0, distance: 20.0, blur: 20.0, red: 0.0, green: 0.0, blue: 0.0, opacity: 0.0 });
        assert_eq!(margin(&e), 82);
    }

    #[test]
    fn disabled_effects_are_not_shown() {
        let mut e = Effects::default();
        e.stroke = Some(StrokeEffect { enabled: Some(false), size: 4.0, red: 0.0, green: 0.0, blue: 0.0, opacity: 1.0, inside: false });
        assert!(shown(&e).is_none());
        e.stroke.as_mut().unwrap().enabled = None;
        assert!(shown(&e).is_some());
        e.stroke.as_mut().unwrap().opacity = 2.0;
        assert!(shown(&e).is_none());
    }

    #[test]
    fn blur_taps_reach_three_sigma() {
        assert_eq!(weights(sigma(20.0)).len(), 61);
        assert_eq!(weights(sigma(0.2)).len(), 3);
        assert_eq!(weights(sigma(3.0)).len(), 11); // 3 × 1.5 = 4.5 rounds away from zero.
    }
}
