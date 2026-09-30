//! Smudge and Liquify: `WarpStroke` on the GPU (`MetalWarp`), then `finishWarp`, which paints the
//! result into the layer's pixels along the stroke with a hard tip a little wider than the brush.

use super::geometry::Point;
use super::stroke::{self, Stroke};
use super::{Result, Tip};
use crate::RenderError;
use crate::gpu::Gpu;
use comp_format::Project;

struct Warp<'a> {
    gpu: &'a Gpu,
    smudge: bool,
    diameter: f64,
    hardness: f64,
    strength: f64,
    width: i32,
    height: i32,
    canvas: wgpu::Buffer,
    carried: Option<wgpu::Buffer>,
    /// Liquify: the layer as the stroke found it, and each pixel's offset.
    original: Option<(wgpu::Buffer, wgpu::Buffer)>,
    scratch: Option<wgpu::Buffer>,
    last: Option<Point>,
    /// Every dab's center.
    points: Vec<Point>,
}

/// One `MetalWarp.dispatch`.
#[derive(Default, Clone, Copy)]
struct Dab {
    kind: u32,
    radius: i32,
    inverse_radius: f32,
    center: [i32; 2],
    origin: [i32; 2],
    area: [i32; 2],
    hardness: f32,
    keep: f32,
    moved: [f32; 2],
}

impl Warp<'_> {
    /// The dab's reach in whole pixels.
    fn radius(&self) -> i32 {
        (self.diameter / 2.0).ceil() as i32
    }

    fn buffer(&self, bytes: u64) -> wgpu::Buffer {
        self.gpu.image(bytes.div_ceil(4).max(1) as u32, 1).buffer
    }

    fn dispatch(&self, dab: Dab, threads: i32) {
        let source = [include_str!("../adjust/float.wgsl"), include_str!("exact.wgsl"), include_str!("warp.wgsl")].join("\n");
        let pipeline = self.gpu.pipeline("paint_warp", &source);
        let mut p = Vec::with_capacity(64);
        p.extend_from_slice(&f32::INFINITY.to_le_bytes());
        p.extend_from_slice(&dab.kind.to_le_bytes());
        p.extend_from_slice(&dab.radius.to_le_bytes());
        p.extend_from_slice(&dab.inverse_radius.to_le_bytes());
        for v in [dab.center, [self.width, self.height], dab.origin, dab.area].concat() {
            p.extend_from_slice(&v.to_le_bytes());
        }
        for v in [dab.hardness, dab.keep, dab.moved[0], dab.moved[1]] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        // Each placeholder is a buffer of its own: writable bindings can't alias.
        let spare: Vec<wgpu::Buffer> = (0..4).map(|_| self.buffer(16)).collect();
        let carried = self.carried.as_ref().unwrap_or(&spare[0]);
        let (original, offsets) = self.original.as_ref().map_or((&spare[1], &spare[2]), |(o, f)| (o, f));
        let scratch = self.scratch.as_ref().unwrap_or(&spare[3]);
        let threads = threads.max(0) as u32;
        self.gpu.dispatch(&pipeline, &p, &[&self.canvas, carried, original, offsets, scratch], threads, threads);
    }

    fn rounded(p: Point) -> [i32; 2] {
        [p[0].round() as i32, p[1].round() as i32]
    }

    fn pick_up(&mut self, at: Point) {
        let r = self.radius();
        let side = 2 * r + 1;
        self.carried = Some(self.buffer(side as u64 * side as u64 * 16));
        self.dispatch(Dab { kind: 0, radius: r, center: Self::rounded(at), ..Default::default() }, side);
    }

    fn smudge_at(&self, at: Point) {
        let r = self.radius();
        let dab = Dab {
            kind: 1,
            radius: r,
            center: Self::rounded(at),
            inverse_radius: 1.0 / (self.diameter / 2.0) as f32,
            hardness: self.hardness as f32,
            keep: self.strength as f32,
            ..Default::default()
        };
        self.dispatch(dab, 2 * r + 1);
    }

    /// `MetalWarp.push`.
    fn push(&mut self, a: Point, b: Point) {
        let r = self.radius();
        let moved = [(b[0] - a[0]) as f32 * self.strength as f32, (b[1] - a[1]) as f32 * self.strength as f32];
        let margin = moved[0].abs().max(moved[1].abs()).ceil() as i32 + 2;
        let [cx, cy] = Self::rounded(b);
        let (x0, x1) = ((cx - r - margin).max(0), (cx + r + margin).min(self.width - 1));
        let (y0, y1) = ((cy - r - margin).max(0), (cy + r + margin).min(self.height - 1));
        if x0 > x1 || y0 > y1 {
            return;
        }
        let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);
        if self.original.is_none() {
            // The first push keeps the layer as it is, and starts every offset at nothing.
            let n = self.width as u64 * self.height as u64;
            self.original = Some((self.buffer(n * 4), self.buffer(n * 8)));
            self.dispatch(Dab { kind: 3, ..Default::default() }, self.width.max(self.height));
        }
        self.scratch = Some(self.buffer(cw as u64 * ch as u64 * 8));
        let dab = Dab {
            kind: 2,
            radius: r,
            center: [cx, cy],
            origin: [x0, y0],
            area: [cw, ch],
            inverse_radius: 1.0 / (self.diameter / 2.0) as f32,
            hardness: self.hardness as f32,
            keep: 0.0,
            moved,
        };
        self.dispatch(dab, cw.max(ch));
        self.dispatch(Dab { kind: 4, ..dab }, 2 * r + 1);
    }

    /// `WarpStroke.append`.
    fn append(&mut self, point: Point) {
        let Some(from) = self.last else {
            self.last = Some(point);
            if self.smudge {
                self.pick_up(point);
            }
            return;
        };
        let distance = (point[0] - from[0]).hypot(point[1] - from[1]);
        let spacing = 1f64.max(self.diameter * if self.smudge { 0.005 } else { 0.025 });
        if distance < spacing {
            return;
        }
        let steps = (distance / spacing).ceil() as i64;
        let mut previous = from;
        for step in 1..=steps {
            let t = step as f64 / steps as f64;
            let next = [from[0] + (point[0] - from[0]) * t, from[1] + (point[1] - from[1]) * t];
            if self.smudge {
                self.smudge_at(next);
            } else {
                self.push(previous, next);
            }
            self.points.push(next);
            previous = next;
        }
        self.last = Some(point);
    }
}

/// `beginWarp`, `WarpStroke.append` for every point, and `finishWarp`.
pub fn apply(gpu: &Gpu, project: &mut Project, layer: &str, smudge: bool, tip: Tip, points: &[Point]) -> Result<()> {
    let m = &project.manifest;
    let record = m.layers.iter().find(|l| l.id == layer).expect("the layer exists");
    let Some(asset) = project.images.get(layer) else { return Err(RenderError::Unsupported("smudging a layer without pixels".into())) };
    let t = &record.transform;
    let (iw, ih) = asset.pixels.dimensions();
    if t.rotation != 0.0 || t.flip_x || t.flip_y || t.size != [iw as f64, ih as f64] || t.origin[0].fract() != 0.0 || t.origin[1].fract() != 0.0 {
        return Err(RenderError::Unsupported("smudging a transformed layer".into()));
    }
    let (width, height) = (m.width as i32, m.height as i32);
    // The working copy: the layer drawn on a document-sized surface, as `LayerRenderer.draw` puts it.
    let pixels = stroke::premultiply(asset.pixels.as_raw());
    let mut surface = vec![0u32; (width * height) as usize];
    let (ox, oy) = (t.origin[0] as i32, t.origin[1] as i32);
    for y in 0..ih as i32 {
        for x in 0..iw as i32 {
            let (dx, dy) = (x + ox, y + oy);
            if dx >= 0 && dy >= 0 && dx < width && dy < height {
                surface[(dy * width + dx) as usize] = pixels[(y * iw as i32 + x) as usize];
            }
        }
    }
    let canvas = gpu.upload(width as u32, height as u32, bytemuck::cast_slice(&surface));
    let mut warp = Warp {
        gpu,
        smudge,
        diameter: 2f64.max(tip.diameter),
        hardness: 0.98f64.min(0f64.max(tip.hardness)),
        strength: 1f64.min(0.01f64.max(tip.opacity)),
        width,
        height,
        canvas: canvas.buffer,
        carried: None,
        original: None,
        scratch: None,
        last: None,
        points: Vec::new(),
    };
    for &p in points {
        warp.append(p);
    }
    if warp.points.is_empty() {
        return Ok(());
    }
    let image = crate::gpu::GpuImage { buffer: warp.canvas, width: width as u32, height: height as u32 };
    let result: Vec<u32> = bytemuck::cast_slice(&gpu.download(&image)?).to_vec();
    // `finishWarp`: a hard tip a little wider than the brush, at full opacity, replacing what's
    // under it with the result, through a point every twentieth of the brush's width.
    let settings = stroke::Settings {
        diameter: warp.diameter + 4.0,
        hardness: 1.0,
        opacity: 1.0,
        color: [0.0; 3],
        erasing: false,
        healing: false,
        healing_mode: 0,
    };
    let mut stroke = Stroke::new(project, layer, false, settings, false)?;
    stroke.set_replacement(&result, width as u32, height as u32);
    let spacing = 1f64.max(warp.diameter * 0.05);
    let mut kept: Option<Point> = None;
    let count = warp.points.len();
    for (index, &point) in warp.points.iter().enumerate() {
        if let Some(k) = kept {
            if index < count - 1 && (point[0] - k[0]).hypot(point[1] - k[1]) < spacing {
                continue;
            }
        }
        stroke.path.append(point);
        kept = Some(point);
    }
    stroke.path.flush();
    stroke.finish(gpu, project)
}
