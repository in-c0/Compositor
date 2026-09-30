//! Object Selection (the Magic tool in Object mode, `Document/ObjectSelection.swift`) and
//! Select > Subject (`EditorSession.selectSubject` in `Document/SubjectRemoval.swift`).
//!
//! On the Mac both start from Apple's Vision (`VNGenerateForegroundInstanceMaskRequest`), which
//! only runs on Apple's systems. The port stands in U²-Netp ([`crate::ml`]), the same model
//! Remove Background uses. U²-Netp gives one saliency map rather than Vision's numbered
//! instances, so [`Instances`] derives them from it: each connected region of the map at 50% or
//! more is an instance, and the fainter pixels around it belong to the instance they reach first.
//! What that gets wrong against Vision is measured in `parity/features.toml`.
//!
//! Everything after the model is the Swift's own arithmetic: picking the instance under the click
//! on the model's grid, the edge-preserving upsample onto the image, the 50% threshold, Edge's
//! erosion or dilation one pixel at a time, the trace, and Anti-alias's simplify-and-Chaikin
//! smoothing of the outline.

use super::geom::{self, Region};
use crate::ml::Saliency;
use image::RgbaImage;

/// Where an instance's own pixels start, as Vision's instance mask draws them.
const INSTANCE_LEVEL: f32 = 0.5;
/// How faint a pixel can be and still count toward the instance its halo surrounds.
const HALO_LEVEL: f32 = 1.0 / 255.0;
/// Below this peak (the model's own confidence, before rembg's stretch) the model found nothing,
/// as Vision returns no instances for an image without a subject.
const PEAK_LEVEL: f32 = 0.5;

/// Why a region at the frame's edge is left pending.
pub const AT_FRAME: &str = "a subject that runs into the image's edge, where U²-Netp can't tell whether Vision sees an object";

/// `SubjectRemoval.Failure.noSubject`'s message.
pub const NO_SUBJECT: &str = "No foreground subject was detected in this layer. Try an image with a more distinct subject.";

/// The model's grid split into instances: 0 is background, 1… an instance's own pixels
/// (`labels`), and each instance's reach including its halo (`reach`).
pub struct Instances {
    pub side: usize,
    pub labels: Vec<u32>,
    pub reach: Vec<u32>,
    pub count: u32,
}

impl Instances {
    pub fn new(map: &Saliency) -> Instances {
        let side = map.side;
        let n = side * side;
        let mut labels = vec![0u32; n];
        let mut count = 0;
        if map.peak >= PEAK_LEVEL {
            // Connected regions at 50% or more, 8-connected as a mask's outline joins at corners.
            let mut stack = Vec::new();
            for start in 0..n {
                if labels[start] != 0 || map.values[start] < INSTANCE_LEVEL {
                    continue;
                }
                count += 1;
                labels[start] = count;
                stack.push(start);
                while let Some(i) = stack.pop() {
                    let (x, y) = ((i % side) as isize, (i / side) as isize);
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let (nx, ny) = (x + dx, y + dy);
                            if nx < 0 || ny < 0 || nx >= side as isize || ny >= side as isize {
                                continue;
                            }
                            let j = ny as usize * side + nx as usize;
                            if labels[j] == 0 && map.values[j] >= INSTANCE_LEVEL {
                                labels[j] = count;
                                stack.push(j);
                            }
                        }
                    }
                }
            }
        }
        // Each instance's halo: the fainter pixels it reaches first, breadth first.
        let mut reach = labels.clone();
        let mut frontier: std::collections::VecDeque<usize> = (0..n).filter(|&i| reach[i] != 0).collect();
        while let Some(i) = frontier.pop_front() {
            let (x, y) = (i % side, i / side);
            let neighbors = [(x > 0).then(|| i - 1), (x + 1 < side).then(|| i + 1), (y > 0).then(|| i - side), (y + 1 < side).then(|| i + side)];
            for j in neighbors.into_iter().flatten() {
                if reach[j] == 0 && map.values[j] >= HALO_LEVEL {
                    reach[j] = reach[i];
                    frontier.push_back(j);
                }
            }
        }
        Instances { side, labels, reach, count }
    }

    /// `ObjectSelection.instanceIndex`: the instance at `point` on the model's grid, or `None` on
    /// background.
    pub fn at(&self, point: [f64; 2], width: usize, height: usize) -> Option<u32> {
        if self.side == 0 || width == 0 || height == 0 {
            return None;
        }
        let side = self.side as f64;
        let x = ((point[0] / width as f64 * side) as i64).clamp(0, self.side as i64 - 1) as usize;
        let y = ((point[1] / height as f64 * side) as i64).clamp(0, self.side as i64 - 1) as usize;
        Some(self.labels[y * self.side + x]).filter(|&v| v != 0)
    }

    /// Whether an instance's own pixels reach the edge of the frame. U²-Netp marks a region there
    /// as salient where Vision may see no object at all (the corpus's photo on its own, which
    /// Vision leaves unselected and U²-Netp calls 39% subject), so the port doesn't guess.
    pub fn touches_frame(&self, instance: u32) -> bool {
        let side = self.side;
        (0..side).any(|i| {
            [i, (side - 1) * side + i, i * side, i * side + side - 1].iter().any(|&j| self.labels[j] == instance)
        })
    }

    /// `VNInstanceMaskObservation.generateMask(forInstances:)`: the instance's soft mask on the
    /// model's grid, zero elsewhere.
    pub fn mask(&self, map: &Saliency, instance: u32) -> Vec<f32> {
        map.values.iter().zip(&self.reach).map(|(&v, &r)| if r == instance { v } else { 0.0 }).collect()
    }
}

/// `ObjectSelection.edgePreservedBinaryMask`: `CIEdgePreserveUpsampleFilter` (spatial sigma 5,
/// luma sigma 0.15) lays the model's `side` × `side` mask onto the image with the image's own
/// luminance as the guide, and pixels at 50% or more are kept. Core Image doesn't document the
/// filter; this is the joint bilateral upsampling it is named for (Kopf et al. 2007), with the
/// spatial sigma in the small image's pixels and the guide reduced to the small image's grid.
/// `guide` is the premultiplied sample, `width` × `height`.
pub fn edge_preserved_binary_mask(small: &[f32], side: usize, guide: &[u8], width: usize, height: usize) -> Vec<u8> {
    const SPATIAL_SIGMA: f64 = 5.0;
    const LUMA_SIGMA: f64 = 0.15;
    let luma: Vec<f32> = guide
        .chunks_exact(4)
        .map(|p| (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0)
        .collect();
    let small_luma = crate::ml::resample(&luma, 1, (width, height), (side, side));
    let radius = (SPATIAL_SIGMA * 2.0).ceil() as i64;
    let spatial: Vec<f64> = (0..=radius * radius * 2).map(|d2| (-(d2 as f64) / (2.0 * SPATIAL_SIGMA * SPATIAL_SIGMA)).exp()).collect();
    let (sx, sy) = (side as f64 / width as f64, side as f64 / height as f64);
    let mut out = vec![0u8; width * height];
    for y in 0..height {
        let v = (y as f64 + 0.5) * sy - 0.5;
        let cy = v.round() as i64;
        for x in 0..width {
            let u = (x as f64 + 0.5) * sx - 0.5;
            let cx = u.round() as i64;
            let here = luma[y * width + x] as f64;
            let (mut sum, mut total) = (0.0f64, 0.0f64);
            for qy in (cy - radius).max(0)..=(cy + radius).min(side as i64 - 1) {
                for qx in (cx - radius).max(0)..=(cx + radius).min(side as i64 - 1) {
                    let (dx, dy) = (qx - cx, qy - cy);
                    let q = qy as usize * side + qx as usize;
                    let dl = here - small_luma[q] as f64;
                    let w = spatial[(dx * dx + dy * dy) as usize] * (-(dl * dl) / (2.0 * LUMA_SIGMA * LUMA_SIGMA)).exp();
                    sum += w * small[q] as f64;
                    total += w;
                }
            }
            let level = if total > 0.0 { sum / total } else { 0.0 };
            out[y * width + x] = if crate::ml::to_byte(level as f32) >= 128 { 255 } else { 0 };
        }
    }
    out
}

/// `ObjectSelection.adjusted`: positive `edge` erodes the mask that many pixels, negative dilates
/// it, one 3 × 3 step at a time, at most 10.
pub fn adjusted(mask: &[u8], width: usize, height: usize, edge: i64) -> Vec<u8> {
    let mut mask = mask.to_vec();
    let steps = edge.unsigned_abs().min(10);
    if steps == 0 || width == 0 || height == 0 {
        return mask;
    }
    for _ in 0..steps {
        let mut result = mask.clone();
        for y in 0..height {
            for x in 0..width {
                let here = mask[y * width + x] != 0;
                // Erosion keeps a pixel only if its whole neighborhood is set; dilation sets one
                // with any neighbor set.
                if (edge > 0) != here {
                    continue;
                }
                let mut touched = false;
                for ny in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                    for nx in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                        if (mask[ny * width + nx] == 0) == (edge > 0) {
                            touched = true;
                        }
                    }
                }
                if edge > 0 {
                    result[y * width + x] = if touched { 0 } else { 255 };
                } else if touched {
                    result[y * width + x] = 255;
                }
            }
        }
        mask = result;
    }
    mask
}

type P = [f64; 2];

/// `ObjectSelection.smoothed`: each traced loop simplified (tolerance 1.6 px) and rounded with
/// three Chaikin passes. Winding and loop order are kept, so holes still subtract.
pub fn smoothed(loops: &[Vec<(i32, i32)>]) -> Region {
    loops
        .iter()
        .filter(|l| l.len() >= 3)
        .filter_map(|l| {
            let points: Vec<P> = l.iter().map(|&(x, y)| [x as f64, y as f64]).collect();
            let points = chaikin(&simplify_closed(&points, 1.6), 3);
            (!points.is_empty()).then(|| geom::polygon(&points))
        })
        .collect()
}

fn simplify_closed(input: &[P], tolerance: f64) -> Vec<P> {
    let mut points = input.to_vec();
    if points.first() == points.last() {
        points.pop();
    }
    if points.len() < 4 {
        return points;
    }
    // Break at a stable extreme (the first leftmost, then topmost) so the open-polyline
    // simplifier keeps the whole closed contour.
    let mut start = 0;
    for i in 1..points.len() {
        let (a, b) = (points[i], points[start]);
        let less = if a[0] == b[0] { a[1] < b[1] } else { a[0] < b[0] };
        if less {
            start = i;
        }
    }
    let mut open: Vec<P> = points[start..].iter().chain(&points[..start]).copied().collect();
    open.push(open[0]);
    let last = open.len() - 1;
    let mut open = simplify_open(&open, 0, last, tolerance);
    if open.first() == open.last() {
        open.pop();
    }
    if open.len() >= 3 { open } else { points }
}

fn simplify_open(points: &[P], first: usize, last: usize, tolerance: f64) -> Vec<P> {
    if last <= first + 1 {
        return vec![points[first], points[last]];
    }
    let mut farthest = first + 1;
    let mut greatest = 0.0;
    for index in first + 1..last {
        let distance = perpendicular_distance(points[index], points[first], points[last]);
        if distance > greatest {
            greatest = distance;
            farthest = index;
        }
    }
    if !(greatest > tolerance) {
        return vec![points[first], points[last]];
    }
    let mut left = simplify_open(points, first, farthest, tolerance);
    let right = simplify_open(points, farthest, last, tolerance);
    left.pop();
    left.extend(right);
    left
}

fn perpendicular_distance(p: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx.hypot(dy);
    if !(length > 0.0) {
        return (p[0] - a[0]).hypot(p[1] - a[1]);
    }
    (dy * p[0] - dx * p[1] + b[0] * a[1] - b[1] * a[0]).abs() / length
}

fn chaikin(input: &[P], iterations: usize) -> Vec<P> {
    let mut points = input.to_vec();
    if points.first() == points.last() {
        points.pop();
    }
    if points.len() < 3 {
        return points;
    }
    for _ in 0..iterations {
        let mut next = Vec::with_capacity(points.len() * 2);
        for i in 0..points.len() {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            next.push([a[0] * 0.75 + b[0] * 0.25, a[1] * 0.75 + b[1] * 0.25]);
            next.push([a[0] * 0.25 + b[0] * 0.75, a[1] * 0.25 + b[1] * 0.75]);
        }
        points = next;
    }
    points
}

/// The sample, premultiplied, as the straight image the model reads.
pub fn straight(sample: &[u8], width: usize, height: usize) -> RgbaImage {
    RgbaImage::from_fn(width as u32, height as u32, |x, y| {
        let i = (y as usize * width + x as usize) * 4;
        let a = sample[i + 3] as u32;
        let c = |v: u8| if a == 0 { 0 } else { ((v as u32 * 255 + a / 2) / a).min(255) as u8 };
        image::Rgba([c(sample[i]), c(sample[i + 1]), c(sample[i + 2]), sample[i + 3]])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erosion_and_dilation_step_one_pixel() {
        let (w, h) = (7, 7);
        let mut mask = vec![0u8; w * h];
        for y in 2..5 {
            for x in 2..5 {
                mask[y * w + x] = 255;
            }
        }
        let eroded = adjusted(&mask, w, h, 1);
        assert_eq!(eroded.iter().filter(|&&v| v != 0).count(), 1);
        let dilated = adjusted(&mask, w, h, -1);
        assert_eq!(dilated.iter().filter(|&&v| v != 0).count(), 25);
        // At most 10 steps.
        assert_eq!(adjusted(&mask, w, h, -40), adjusted(&mask, w, h, -10));
    }

    #[test]
    fn a_square_smooths_inside_its_corners() {
        let square = vec![(0, 0), (10, 0), (10, 10), (0, 10)];
        let region = smoothed(&[square]);
        assert_eq!(region.len(), 1);
        // Three Chaikin passes of four corners: 32 points, all within the square.
        assert_eq!(region[0].len(), 32);
        let area = geom::signed_area(&geom::flatten(&region[0]));
        assert!(area.abs() > 80.0 && area.abs() < 100.0, "{area}");
    }

    #[test]
    fn instances_split_and_pick_by_click() {
        let side = 8;
        let mut values = vec![0.0f32; side * side];
        for (x, y) in [(1, 1), (2, 1), (1, 2), (6, 6), (5, 6)] {
            values[y * side + x] = 1.0;
        }
        values[3 * side + 1] = 0.2;
        let map = Saliency { side, values, peak: 0.9 };
        let found = Instances::new(&map);
        assert_eq!(found.count, 2);
        let first = found.at([1.5, 1.5], 8, 8).unwrap();
        let second = found.at([6.5, 6.5], 8, 8).unwrap();
        assert_ne!(first, second);
        assert_eq!(found.at([4.0, 0.0], 8, 8), None);
        // The faint pixel below the first instance belongs to it.
        assert_eq!(found.mask(&map, first)[3 * side + 1], 0.2);
        assert_eq!(found.mask(&map, second)[3 * side + 1], 0.0);
        assert!(!found.touches_frame(first));
        let mut edge = map.values.clone();
        edge[7] = 1.0;
        let framed = Instances::new(&Saliency { side, values: edge, peak: 0.9 });
        assert!(framed.touches_frame(framed.at([7.5, 0.5], 8, 8).unwrap()));
        // A model that isn't confident finds nothing.
        assert_eq!(Instances::new(&Saliency { peak: 0.1, ..map }).count, 0);
    }
}
