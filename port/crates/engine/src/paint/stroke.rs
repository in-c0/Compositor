//! `BrushStroke` on the GPU path: the stroke's pixel grid, its 256-pixel tiles, the tip's coverage
//! (`MetalBrushCoverage`), and the paint composited through it into the layer's pixels or mask,
//! then `paintSnapshot` and `commitPaintSnapshot`.

use super::geometry::{Path, Point, Rect};
use super::{Result, failed, heal};
use crate::RenderError;
use crate::gpu::Gpu;
use comp_format::{LayerRecord, Project, Transform};
use image::{GrayImage, RgbaImage};

pub struct Settings {
    pub diameter: f64,
    pub hardness: f64,
    pub opacity: f64,
    /// 0...1 per channel; a mask paints the first.
    pub color: [f64; 3],
    pub erasing: bool,
    pub healing: bool,
    pub healing_mode: i32,
}

/// Whether the port's Core Image Gaussian blur matches the Mac's to the byte. It doesn't yet
/// (see `adjust/blur.rs`), so Blur strokes report as not supported.
const BLUR_IS_EXACT: bool = false;

/// `BrushStroke.tileSize`.
const TILE: i64 = 256;

/// What Clone Stamp or Blur paints through the tip, laid over the stroke's grid: a premultiplied
/// pixel (or mask value) per grid pixel, and whether the sample reaches it.
struct Sample {
    pixels: Vec<u32>,
    reaches: Vec<u32>,
}

pub struct Stroke {
    layer: String,
    mask: bool,
    settings: Settings,
    /// The grid: `width` x `height` pixels, pixel (x, y) at document (x + tx, y + ty).
    width: i64,
    height: i64,
    tx: f64,
    ty: f64,
    canvas: Rect,
    /// Where the layer's (or mask's) own pixels sit in the grid.
    source_rect: Rect,
    /// The transform the grid's pixels have on the document (`paintTransform`).
    paint_transform: Transform,
    /// The grid before painting: premultiplied RGBA, or mask values, one `u32` per pixel.
    base: Vec<u32>,
    /// A mask's value past its pixels (`LayerMask.background`), 0 or 255.
    mask_background: u32,
    pub path: Path,
    sample: Option<Sample>,
    /// Smudge and Liquify's result replaces what's under the tip rather than drawing over it.
    pub replaces: bool,
}

fn unsupported<T>(what: &str) -> Result<T> {
    Err(RenderError::Unsupported(what.to_string()))
}

/// A transform the port paints through: upright, unflipped, one layer pixel per document pixel,
/// on whole pixels.
fn plain(t: &Transform, width: u32, height: u32) -> bool {
    t.rotation == 0.0
        && !t.flip_x
        && !t.flip_y
        && t.size == [width as f64, height as f64]
        && t.origin[0].fract() == 0.0
        && t.origin[1].fract() == 0.0
}

/// `ImageIO`'s premultiply on decode: rounded to nearest.
pub fn premultiply(rgba: &[u8]) -> Vec<u32> {
    rgba.chunks_exact(4)
        .map(|p| {
            let a = p[3] as u32;
            let c = |v: u8| (v as u32 * a + 127) / 255;
            c(p[0]) | c(p[1]) << 8 | c(p[2]) << 16 | a << 24
        })
        .collect()
}

/// The PNG writer's unpremultiply, `(c × 255 + a / 2) / a`.
fn unpremultiply(p: u32) -> [u8; 4] {
    let a = p >> 24;
    if a == 0 {
        return [0; 4];
    }
    let c = |v: u32| ((v * 255 + a / 2) / a).min(255) as u8;
    [c(p & 255), c(p >> 8 & 255), c(p >> 16 & 255), a as u8]
}

/// `LayerMask.background(of:)`: white when the mask's edge pixels average at least half.
fn mask_background(mask: &GrayImage) -> u32 {
    let (w, h) = mask.dimensions();
    if w > 96 || h > 96 {
        // The Mac reads the 96-pixel thumbnail's edge, which the port doesn't make.
        return u32::MAX;
    }
    let (mut total, mut count) = (0u64, 0u64);
    for y in 0..h {
        for x in 0..w {
            if y == 0 || y == h - 1 || x == 0 || x == w - 1 {
                total += mask.get_pixel(x, y)[0] as u64;
                count += 1;
            }
        }
    }
    if total * 2 >= count * 255 { 255 } else { 0 }
}

impl Stroke {
    /// `BrushStroke.init`. `grows_mask`: a brush on a mask can paint anywhere on the canvas.
    pub fn new(project: &Project, layer_id: &str, mask: bool, settings: Settings, grows_mask: bool) -> Result<Stroke> {
        let m = &project.manifest;
        let layer: &LayerRecord = m.layers.iter().find(|l| l.id == layer_id).expect("the layer exists");
        if layer.is_group() || layer.adjustment.is_some() && !mask {
            return failed("the layer has no pixels to paint");
        }
        let Some(asset) = project.images.get(layer_id) else { return unsupported("painting a layer without pixels") };
        let (iw, ih) = asset.pixels.dimensions();
        let mask_image = project.masks.get(layer_id).map(|a| &a.pixels);
        // A mask on its own placement is painted in its own pixel grid; otherwise the grid is the layer's.
        let (base, ow, oh) = match (mask, &layer.mask_placement, mask_image) {
            (true, Some(placement), Some(img)) => {
                if img.width() <= 2 && img.height() <= 2 {
                    return unsupported("painting a solid placed mask");
                }
                (*placement, img.width(), img.height())
            }
            _ => (layer.transform, iw, ih),
        };
        if !plain(&base, ow, oh) {
            return unsupported("painting a transformed layer or mask");
        }
        if !(1.0..=2100.0).contains(&settings.diameter) || !(0.0..=1.0).contains(&settings.hardness) || !(0.01..=1.0).contains(&settings.opacity) {
            return failed("brush settings out of range");
        }
        let canvas = Rect::new(0.0, 0.0, m.width as f64, m.height as f64);
        let (ox, oy) = (base.origin[0], base.origin[1]);
        let original = Rect::new(0.0, 0.0, ow as f64, oh as f64);
        let extent = if mask && !grows_mask { original } else { original.union(&canvas.offset(-ox, -oy).integral()) };
        let (width, height) = (extent.w as i64, extent.h as i64);
        let source_rect = original.offset(-extent.x, -extent.y);
        let mut paint_transform = base;
        paint_transform.size = [width as f64 * base.size[0] / ow as f64, height as f64 * base.size[1] / oh as f64];
        let center = [extent.x + extent.w / 2.0 + ox, extent.y + extent.h / 2.0 + oy];
        paint_transform.origin = [center[0] - paint_transform.size[0] / 2.0, center[1] - paint_transform.size[1] / 2.0];
        let n = (width * height) as usize;
        let (sx, sy) = (source_rect.x as i64, source_rect.y as i64);
        let (mut base_pixels, background) = if mask {
            let img = mask_image.expect("a mask");
            let background = mask_background(img);
            if background == u32::MAX {
                return unsupported("painting a mask over 96 pixels wide");
            }
            let mut grid = vec![background; n];
            let uniform = img.width() == 1 && img.height() == 1;
            if !uniform && img.dimensions() != (ow, oh) {
                return unsupported("painting a resampled mask");
            }
            for y in 0..oh as i64 {
                for x in 0..ow as i64 {
                    let v = if uniform { img.as_raw()[0] } else { img.get_pixel(x as u32, y as u32)[0] };
                    grid[((y + sy) * width + x + sx) as usize] = v as u32;
                }
            }
            (grid, background)
        } else {
            (vec![0u32; n], 0)
        };
        if !mask {
            let pixels = premultiply(asset.pixels.as_raw());
            for y in 0..ih as i64 {
                let row = (y * iw as i64) as usize;
                let at = ((y + sy) * width + sx) as usize;
                base_pixels[at..at + iw as usize].copy_from_slice(&pixels[row..row + iw as usize]);
            }
        }
        Ok(Stroke {
            layer: layer_id.to_string(),
            mask,
            settings,
            width,
            height,
            tx: extent.x + ox,
            ty: extent.y + oy,
            canvas,
            source_rect,
            paint_transform,
            base: base_pixels,
            mask_background: background,
            path: Path::default(),
            sample: None,
            replaces: false,
        })
    }

    /// Clone Stamp's sample (`cloneSample`), `offset` document pixels from where it paints: the
    /// layer's own pixels, or with `all`, the canvas as it shows every layer.
    pub fn set_clone(&mut self, gpu: &Gpu, project: &Project, offset: Point, all: bool) -> Result<()> {
        let (pixels, sw, sh, placed) = if all {
            let canvas = crate::composite::Compositor::new(gpu, project).render()?;
            let bytes = gpu.download(&canvas)?;
            let pixels: Vec<u32> = bytemuck::cast_slice(&bytes).to_vec();
            // Placed on the document at -offset, carried into the grid.
            (pixels, canvas.width, canvas.height, [-offset[0] - self.tx, -offset[1] - self.ty])
        } else {
            let asset = &project.images[&self.layer];
            let (w, h) = asset.pixels.dimensions();
            (premultiply(asset.pixels.as_raw()), w, h, [self.source_rect.x - offset[0], self.source_rect.y - offset[1]])
        };
        self.sample = Some(self.lay(&pixels, sw, sh, placed));
        Ok(())
    }

    /// Lays a `sw` x `sh` image over the grid with its top-left pixel at grid `placed`.
    fn lay(&self, pixels: &[u32], sw: u32, sh: u32, placed: [f64; 2]) -> Sample {
        let n = (self.width * self.height) as usize;
        let mut sample = Sample { pixels: vec![0; n], reaches: vec![0; n] };
        let (px, py) = (placed[0] as i64, placed[1] as i64);
        for y in 0..self.height {
            for x in 0..self.width {
                let (u, v) = (x - px, y - py);
                if u >= 0 && v >= 0 && u < sw as i64 && v < sh as i64 {
                    let i = (y * self.width + x) as usize;
                    sample.pixels[i] = pixels[(v * sw as i64 + u) as usize];
                    sample.reaches[i] = 1;
                }
            }
        }
        sample
    }

    /// Blur's sample (`blurSample`): the layer's pixels (or its mask) softened by Core Image's
    /// Gaussian blur, `radius` canvas pixels, with room around them for the blur to spread.
    pub fn set_blur(&mut self, gpu: &Gpu, radius: f64) -> Result<()> {
        if !BLUR_IS_EXACT {
            return unsupported("the Blur tool (Core Image's Gaussian blur isn't reproduced exactly yet)");
        }
        let (sw, sh) = (self.source_rect.w, self.source_rect.h);
        let sigma = (0.5f64.max(radius).min(50.0)).min(sw.max(sh) / 2.0);
        let margin = (3.0 * sigma).ceil();
        let (w, h) = ((sw + 2.0 * margin) as i64, (sh + 2.0 * margin) as i64);
        let m = margin as i64;
        // A mask is clamped to its extent before blurring, so its edge tone carries on; pixels
        // are blurred as they are, transparent past the context.
        let pad = if self.mask { (3.0 * sigma).floor() as i64 + 2 } else { 0 };
        let (pw, ph) = (w + 2 * pad, h + 2 * pad);
        let mut context = vec![if self.mask { self.mask_background * 0x0001_0101 | 0xff00_0000 } else { 0 }; (w * h) as usize];
        for y in 0..sh as i64 {
            for x in 0..sw as i64 {
                let g = self.base[((y + self.source_rect.y as i64) * self.width + x + self.source_rect.x as i64) as usize];
                context[((y + m) * w + x + m) as usize] = if self.mask { (g & 255) * 0x0001_0101 | 0xff00_0000 } else { g };
            }
        }
        let padded: Vec<u32> = (0..ph)
            .flat_map(|y| (0..pw).map(move |x| (x, y)))
            .map(|(x, y)| context[((y - pad).clamp(0, h - 1) * w + (x - pad).clamp(0, w - 1)) as usize])
            .collect();
        let image = gpu.upload(pw as u32, ph as u32, bytemuck::cast_slice(&padded));
        let soft = crate::adjust::blur::gaussian(gpu, &image, sigma);
        let soft: Vec<u32> = bytemuck::cast_slice(&gpu.download(&soft)?).to_vec();
        let mask = self.mask;
        let cropped: Vec<u32> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| {
                let p = soft[((y + pad) * pw + x + pad) as usize];
                if mask { p & 255 } else { p }
            })
            .collect();
        self.sample = Some(self.lay(&cropped, w as u32, h as u32, [self.source_rect.x - margin, self.source_rect.y - margin]));
        Ok(())
    }

    /// Smudge and Liquify's result, a document-sized premultiplied image, painted over the grid.
    pub fn set_replacement(&mut self, pixels: &[u32], width: u32, height: u32) {
        self.sample = Some(self.lay(pixels, width, height, [-self.tx, -self.ty]));
        self.replaces = true;
    }

    /// `continuousKeys`: the tiles a call's segments reach.
    fn keys(&self, segments: &[[f32; 4]], keys: &mut std::collections::BTreeSet<i64>) {
        let reach = self.settings.diameter / 2.0 + 2.0;
        let columns = (self.width + TILE - 1) / TILE;
        let grid = Rect::new(0.0, 0.0, self.width as f64, self.height as f64);
        for s in segments {
            let (x0, x1) = (s[0].min(s[2]) as f64, s[0].max(s[2]) as f64);
            let (y0, y1) = (s[1].min(s[3]) as f64, s[1].max(s[3]) as f64);
            let bx = Rect::new(x0, y0, (s[2] - s[0]).abs() as f64, (s[3] - s[1]).abs() as f64);
            let _ = (x1, y1);
            let Some(bx) = bx.inset(-reach, -reach).intersection(&self.canvas) else { continue };
            if bx.is_empty() {
                continue;
            }
            let Some(affected) = bx.offset(-self.tx, -self.ty).integral().intersection(&grid) else { continue };
            if affected.is_empty() {
                continue;
            }
            for ty in affected.y as i64 / TILE..=(affected.max_y().ceil() as i64 - 1) / TILE {
                for tx in affected.x as i64 / TILE..=(affected.max_x().ceil() as i64 - 1) / TILE {
                    keys.insert(ty * columns + tx);
                }
            }
        }
    }

    fn tile_rect(&self, key: i64) -> Rect {
        let columns = (self.width + TILE - 1) / TILE;
        let (x, y) = (key % columns * TILE, key / columns * TILE);
        Rect::new(x as f64, y as f64, TILE.min(self.width - x) as f64, TILE.min(self.height - y) as f64)
    }

    /// The tip's coverage over the grid, one byte per pixel.
    fn coverage(&self, gpu: &Gpu) -> Result<Vec<u32>> {
        let (segments, ends) = self.path.settled();
        let pipeline = gpu.pipeline("paint_coverage", include_str!("coverage.wgsl"));
        let s = &self.settings;
        let spacing = 0.25f64.max(s.diameter * if s.hardness >= 1.0 { 0.015 } else { 0.025 });
        let floats: [f32; 12] = [
            1.0,
            0.0,
            0.0,
            1.0,
            self.tx as f32,
            self.ty as f32,
            (s.diameter / 2.0) as f32,
            s.hardness as f32,
            self.canvas.w as f32,
            self.canvas.h as f32,
            1.0,
            spacing as f32,
        ];
        let mut params: Vec<u8> = bytemuck::cast_slice(&floats).to_vec();
        for v in [self.width as u32, self.height as u32, ends.len() as u32, 0] {
            params.extend_from_slice(&v.to_le_bytes());
        }
        let segments = gpu.bytes(bytemuck::cast_slice(&segments));
        let ends = gpu.bytes(bytemuck::cast_slice(&ends));
        let out = gpu.image(self.width as u32, self.height as u32);
        gpu.dispatch(&pipeline, &params, &[&segments, &ends, &out.buffer], self.width as u32, self.height as u32);
        Ok(bytemuck::cast_slice(&gpu.download(&out)?).to_vec())
    }

    /// Paints the stroke into the grid, then commits it to the layer: `flush`'s publish, `heal`,
    /// `paintSnapshot` and `commitPaintSnapshot`.
    pub fn finish(self, gpu: &Gpu, project: &mut Project) -> Result<()> {
        // The tiles `renderContinuous` allocated: every call's settled segments and tail.
        let mut keys = std::collections::BTreeSet::new();
        for (settled, tail) in &self.path.calls {
            self.keys(settled, &mut keys);
            self.keys(tail, &mut keys);
        }
        if keys.is_empty() {
            // No tile, no patch: `finishBrushImmediately` commits nothing.
            return Ok(());
        }
        let mut allocated = self.source_rect;
        for &key in &keys {
            allocated = allocated.union(&self.tile_rect(key));
        }
        let coverage = self.coverage(gpu)?;
        if let Ok(dir) = std::env::var("PAINT_DUMP") {
            // Fitting aid: the grid's size, origin, coverage and base, raw.
            let mut out = Vec::new();
            for v in [self.width as u32, self.height as u32, self.tx as i32 as u32, self.ty as i32 as u32] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(bytemuck::cast_slice(&coverage));
            out.extend_from_slice(bytemuck::cast_slice(&self.base));
            let _ = std::fs::write(std::path::Path::new(&dir).join(format!("{}.bin", self.layer)), out);
        }
        let painted = if self.settings.healing {
            self.heal(&coverage, &keys)
        } else {
            self.composite(gpu, &coverage)?
        };
        self.commit(project, painted, allocated.integral())
    }

    /// `publish` for the last time, through the final coverage: the paint, the eraser or the
    /// sample, drawn over each tile's original pixels.
    fn composite(&self, gpu: &Gpu, coverage: &[u32]) -> Result<Vec<u32>> {
        let s = &self.settings;
        let pipeline = gpu.pipeline("paint_composite", include_str!("composite.wgsl"));
        let mode: u32 = if let Some(_) = &self.sample {
            if self.replaces { 4 } else { 3 }
        } else if self.mask {
            2
        } else if s.erasing {
            1
        } else {
            0
        };
        let mut params = Vec::new();
        for v in [self.width as u32, self.height as u32, mode, self.mask as u32] {
            params.extend_from_slice(&v.to_le_bytes());
        }
        for v in [s.color[0] as f32, s.color[1] as f32, s.color[2] as f32, s.opacity as f32] {
            params.extend_from_slice(&v.to_le_bytes());
        }
        let n = (self.width * self.height) as usize;
        let empty = vec![0u32; n];
        let (sample, reaches) = self.sample.as_ref().map_or((&empty, &empty), |s| (&s.pixels, &s.reaches));
        let base = gpu.bytes(bytemuck::cast_slice(&self.base));
        let coverage = gpu.bytes(bytemuck::cast_slice(coverage));
        let sample = gpu.bytes(bytemuck::cast_slice(sample));
        let reaches = gpu.bytes(bytemuck::cast_slice(reaches));
        let out = gpu.image(self.width as u32, self.height as u32);
        gpu.dispatch(&pipeline, &params, &[&base, &coverage, &sample, &reaches, &out.buffer], self.width as u32, self.height as u32);
        Ok(bytemuck::cast_slice(&gpu.download(&out)?).to_vec())
    }

    /// `BrushStroke.heal`: rebuilds the painted area from nearby texture with `HealPixels.c`.
    fn heal(&self, coverage: &[u32], keys: &std::collections::BTreeSet<i64>) -> Vec<u32> {
        let (w, h) = (self.width, self.height);
        let bytes: Vec<u8> = coverage.iter().map(|&c| c as u8).collect();
        let b = heal::coverage_bounds(&bytes, w as usize, h as usize, w as usize);
        if b[2] <= b[0] {
            return self.base.clone();
        }
        let painted = Rect::new(b[0] as f64, b[1] as f64, (b[2] - b[0]) as f64, (b[3] - b[1]) as f64);
        // Room for the kernel's patch search, which looks up to about three spot-widths away.
        let reach = (painted.w.max(painted.h) + 32.0) * 3.2;
        let Some(region) = painted.inset(-reach, -reach).intersection(&Rect::new(0.0, 0.0, w as f64, h as f64)) else {
            return self.base.clone();
        };
        let region = region.integral();
        let (rx, ry, rw, rh) = (region.x as i64, region.y as i64, region.w as i64, region.h as i64);
        // The layer's own pixels (the base holds nothing else), and the coverage, over the region.
        let mut rgba = vec![0u8; (rw * rh * 4) as usize];
        let mut gray = vec![0u8; (rw * rh) as usize];
        for y in 0..rh {
            for x in 0..rw {
                let g = ((y + ry) * w + x + rx) as usize;
                let r = (y * rw + x) as usize;
                rgba[r * 4..r * 4 + 4].copy_from_slice(&self.base[g].to_le_bytes());
                gray[r] = bytes[g];
            }
        }
        heal::spot_heal(&mut rgba, &gray, rw as usize, rh as usize, rw as usize * 4, self.settings.opacity as f32, self.settings.healing_mode, 0);
        // Written into every tile the stroke touched.
        let mut out = self.base.clone();
        for &key in keys {
            let t = self.tile_rect(key);
            for y in t.y as i64..t.max_y() as i64 {
                for x in t.x as i64..t.max_x() as i64 {
                    let (u, v) = (x - rx, y - ry);
                    if u >= 0 && v >= 0 && u < rw && v < rh {
                        let r = ((v * rw + u) * 4) as usize;
                        out[(y * w + x) as usize] = u32::from_le_bytes([rgba[r], rgba[r + 1], rgba[r + 2], rgba[r + 3]]);
                    }
                }
            }
        }
        out
    }

    /// `paintSnapshot` and `commitPaintSnapshot`.
    fn commit(&self, project: &mut Project, grid: Vec<u32>, committed: Rect) -> Result<()> {
        let w = self.width;
        let crop = if self.mask {
            committed
        } else {
            // Painting only adds alpha: the old bounds, grown to every pixel with any alpha.
            let mut bounds = self.source_rect;
            let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
            for y in 0..self.height {
                for x in 0..w {
                    if grid[(y * w + x) as usize] >> 24 != 0 {
                        x0 = x0.min(x);
                        y0 = y0.min(y);
                        x1 = x1.max(x + 1);
                        y1 = y1.max(y + 1);
                    }
                }
            }
            if x1 > x0 {
                bounds = bounds.union(&Rect::new(x0 as f64, y0 as f64, (x1 - x0) as f64, (y1 - y0) as f64));
            }
            bounds
        };
        let (cx, cy, cw, ch) = (crop.x as i64, crop.y as i64, crop.w as u32, crop.h as u32);
        let transform = self.transform_for(&crop);
        let index = project.manifest.layers.iter().position(|l| l.id == self.layer).expect("the layer exists");
        let at = |x: u32, y: u32| grid[((y as i64 + cy) * w + x as i64 + cx) as usize];
        if self.mask {
            let image = GrayImage::from_fn(cw, ch, |x, y| image::Luma([at(x, y) as u8]));
            project.masks.insert(self.layer.clone(), comp_format::Asset::new(image));
            let layer = &mut project.manifest.layers[index];
            // Grown past its layer, or already placed on its own: the mask keeps its place on the document.
            if layer.mask_placement.is_some() || crop != self.source_rect {
                layer.mask_placement = Some(transform);
            }
        } else {
            let image = RgbaImage::from_fn(cw, ch, |x, y| image::Rgba(unpremultiply(at(x, y))));
            project.images.insert(self.layer.clone(), comp_format::Asset::new(image));
            let layer = &mut project.manifest.layers[index];
            layer.transform = transform;
            // An unplaced mask grows with its layer, white where it had no pixels.
            if crop != self.source_rect && layer.mask_placement.is_none() {
                if let Some(old) = project.masks.get(&self.layer).map(|a| a.pixels.clone()) {
                    let uniform = old.width() == 1 && old.height() == 1;
                    if !uniform && old.dimensions() != (self.source_rect.w as u32, self.source_rect.h as u32) {
                        return unsupported("growing a resampled mask");
                    }
                    let (sx, sy) = ((self.source_rect.x - crop.x) as i64, (self.source_rect.y - crop.y) as i64);
                    let (sw, sh) = (self.source_rect.w as i64, self.source_rect.h as i64);
                    let grown = GrayImage::from_fn(cw, ch, |x, y| {
                        let (u, v) = (x as i64 - sx, y as i64 - sy);
                        if u >= 0 && v >= 0 && u < sw && v < sh {
                            if uniform { old.get_pixel(0, 0).clone() } else { *old.get_pixel(u as u32, v as u32) }
                        } else {
                            image::Luma([255])
                        }
                    });
                    project.masks.insert(self.layer.clone(), comp_format::Asset::new(grown));
                }
            }
        }
        let _ = self.mask_background;
        Ok(())
    }

    /// `BrushStroke.transform(for:)`.
    fn transform_for(&self, bounds: &Rect) -> Transform {
        let center = [bounds.x + bounds.w / 2.0 + self.tx, bounds.y + bounds.h / 2.0 + self.ty];
        let mut result = self.paint_transform;
        result.size = [
            bounds.w * self.paint_transform.size[0] / self.width as f64,
            bounds.h * self.paint_transform.size[1] / self.height as f64,
        ];
        result.origin = [center[0] - result.size[0] / 2.0, center[1] - result.size[1] / 2.0];
        result
    }
}
