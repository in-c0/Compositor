//! The Move and Crop tools' geometry, as `LayerTransform`, `TransformDrag`, `TransformSnap`,
//! `CropGeometry` and `CropDrag` (Document/LayerTransform.swift, Document/Crop.swift) compute it.
//! Points are document pixels, y down.

use comp_format::Transform;

pub type Point = [f64; 2];

/// `LayerTransform.handles`: corners and edge midpoints clockwise from the top left, in the unit square.
pub const HANDLES: [Point; 8] = [[0.0, 0.0], [0.5, 0.0], [1.0, 0.0], [1.0, 0.5], [1.0, 1.0], [0.5, 1.0], [0.0, 1.0], [0.0, 0.5]];

pub fn center(t: &Transform) -> Point {
    [t.origin[0] + t.size[0] / 2.0, t.origin[1] + t.size[1] / 2.0]
}

pub fn radians(t: &Transform) -> f64 {
    (t.rotation % 360.0).to_radians()
}

/// Where the unit-square point `unit` lands on the document.
pub fn point(t: &Transform, unit: Point) -> Point {
    let c = center(t);
    let (x, y) = ((unit[0] - 0.5) * t.size[0], (unit[1] - 0.5) * t.size[1]);
    let r = radians(t);
    [c[0] + x * r.cos() - y * r.sin(), c[1] + x * r.sin() + y * r.cos()]
}

pub fn contains(t: &Transform, p: Point) -> bool {
    let c = center(t);
    let (x, y) = (p[0] - c[0], p[1] - c[1]);
    let r = radians(t);
    (x * r.cos() + y * r.sin()).abs() <= t.size[0] / 2.0 && (-x * r.sin() + y * r.cos()).abs() <= t.size[1] / 2.0
}

pub fn is_valid(t: &Transform) -> bool {
    [t.origin[0], t.origin[1], t.size[0], t.size[1], t.rotation].iter().all(|v| v.is_finite())
        && (1.0..=300_000.0).contains(&t.size[0])
        && (1.0..=300_000.0).contains(&t.size[1])
        && t.origin[0].abs() <= 1_000_000.0
        && t.origin[1].abs() <= 1_000_000.0
}

/// Whole pixels and whole degrees: what dragging, scaling and rotating leave behind.
pub fn rounded(t: &Transform) -> Transform {
    let mut r = *t;
    r.origin = [t.origin[0].round(), t.origin[1].round()];
    r.size = [t.size[0].round().max(1.0), t.size[1].round().max(1.0)];
    r.rotation = t.rotation.round();
    r
}

/// Both sides set to `percent` of `pixels`, keeping the center.
pub fn scaled_to_percent(t: &Transform, percent: f64, pixels: [f64; 2]) -> Transform {
    let c = center(t);
    let mut r = *t;
    r.size = [pixels[0] * percent / 100.0, pixels[1] * percent / 100.0];
    r.origin = [c[0] - r.size[0] / 2.0, c[1] - r.size[1] / 2.0];
    r
}

/// The four corners on the document, handle order (top left, top right, bottom right, bottom left).
pub fn corners(t: &Transform) -> [Point; 4] {
    [point(t, [0.0, 0.0]), point(t, [1.0, 0.0]), point(t, [1.0, 1.0]), point(t, [0.0, 1.0])]
}

/// The upright box around the corners: [min x, min y, max x, max y].
pub fn bounds(t: &Transform) -> [f64; 4] {
    let c = corners(t);
    let xs = c.iter().map(|p| p[0]);
    let ys = c.iter().map(|p| p[1]);
    [xs.clone().fold(f64::INFINITY, f64::min), ys.clone().fold(f64::INFINITY, f64::min), xs.fold(f64::NEG_INFINITY, f64::max), ys.fold(f64::NEG_INFINITY, f64::max)]
}

/// An affine map `[a, b, c, d, tx, ty]` (Core Graphics order: x' = a x + c y + tx, y' = b x + d y + ty).
pub type Affine = [f64; 6];

pub fn concat(m: Affine, n: Affine) -> Affine {
    // m then n.
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

pub fn invert(m: Affine) -> Affine {
    let det = m[0] * m[3] - m[1] * m[2];
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    [a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)]
}

pub fn apply(m: Affine, p: Point) -> Point {
    [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]]
}

/// `unitToDocument`: the unit square (y down) where `t` places a layer, flips included.
pub fn unit_to_document(t: &Transform) -> Affine {
    let (sx, sy) = (if t.flip_x { -1.0 } else { 1.0 }, if t.flip_y { -1.0 } else { 1.0 });
    let r = radians(t);
    let c = center(t);
    // Center the unit square, flip, scale to size, rotate, move to the center.
    let m = [sx * t.size[0], 0.0, 0.0, sy * t.size[1], -0.5 * sx * t.size[0], -0.5 * sy * t.size[1]];
    concat(m, [r.cos(), r.sin(), -r.sin(), r.cos(), c[0], c[1]])
}

/// `LayerTransform.placing`: a transform placing the unit square as `map` does (shear dropped).
pub fn placing(t: &Transform, map: Affine) -> Transform {
    let sign = if t.flip_x { -1.0 } else { 1.0 };
    let angle = (map[1] * sign).atan2(map[0] * sign);
    let along = -map[2] * angle.sin() + map[3] * angle.cos();
    let middle = apply(map, [0.5, 0.5]);
    let mut r = *t;
    r.size = [map[0].hypot(map[1]), along.abs()];
    let degrees = angle.to_degrees();
    r.rotation = degrees + ((t.rotation - degrees) / 360.0).round() * 360.0;
    r.flip_y = along < 0.0;
    r.origin = [middle[0] - r.size[0] / 2.0, middle[1] - r.size[1] / 2.0];
    r
}

/// `LayerTransform.following`: `t` carried along as a layer moves from `old` to `new`.
pub fn following(t: &Transform, old: &Transform, new: &Transform) -> Transform {
    if old == new {
        return *t;
    }
    if old.size == new.size && old.rotation == new.rotation && old.flip_x == new.flip_x && old.flip_y == new.flip_y {
        let mut moved = *t;
        moved.origin[0] += new.origin[0] - old.origin[0];
        moved.origin[1] += new.origin[1] - old.origin[1];
        return moved;
    }
    placing(t, concat(concat(unit_to_document(t), invert(unit_to_document(old))), unit_to_document(new)))
}

pub fn same_placement(a: &Transform, b: &Transform) -> bool {
    let mut copy = *a;
    copy.sampling = b.sampling;
    copy == *b
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Move,
    Resize(usize),
    Rotate,
}

/// `TransformDrag`.
#[derive(Clone, Copy, Debug)]
pub struct TransformDrag {
    pub original: Transform,
    pub start: Point,
    pub mode: Mode,
}

impl TransformDrag {
    pub fn updated(&self, p: Point, lock_ratio: bool, shift: bool, option: bool) -> Transform {
        let o = &self.original;
        let mut result = *o;
        match self.mode {
            Mode::Move => {
                let (mut dx, mut dy) = (p[0] - self.start[0], p[1] - self.start[1]);
                if shift {
                    if dx.abs() >= dy.abs() {
                        dy = 0.0
                    } else {
                        dx = 0.0
                    }
                }
                result.origin[0] += dx;
                result.origin[1] += dy;
            }
            Mode::Rotate => {
                let c = center(o);
                let delta = (p[1] - c[1]).atan2(p[0] - c[0]) - (self.start[1] - c[1]).atan2(self.start[0] - c[0]);
                result.rotation += delta.to_degrees();
                if shift {
                    result.rotation = (result.rotation / 15.0).round() * 15.0;
                }
            }
            Mode::Resize(index) => {
                let handle = HANDLES[index];
                let anchor_unit = if option { [0.5, 0.5] } else { [1.0 - handle[0], 1.0 - handle[1]] };
                let anchor = point(o, anchor_unit);
                let initial = point(o, handle);
                let dx = initial[0] + p[0] - self.start[0] - anchor[0];
                let dy = initial[1] + p[1] - self.start[1] - anchor[1];
                let span = if option { 2.0 } else { 1.0 };
                let r = radians(o);
                let local_x = (dx * r.cos() + dy * r.sin()) * span;
                let local_y = (-dx * r.sin() + dy * r.cos()) * span;
                let (sx, sy) = (handle[0] * 2.0 - 1.0, handle[1] * 2.0 - 1.0);
                let raw_w = if sx == 0.0 { o.size[0] } else { local_x * sx };
                let raw_h = if sy == 0.0 { o.size[1] } else { local_y * sy };
                let (mirror_x, mirror_y) = (raw_w < 0.0, raw_h < 0.0);
                let mut width = raw_w.abs().max(1.0);
                let mut height = raw_h.abs().max(1.0);
                if lock_ratio != shift {
                    let factor = if sx == 0.0 {
                        height / o.size[1]
                    } else if sy == 0.0 {
                        width / o.size[0]
                    } else {
                        (1.0 / o.size[0].min(o.size[1])).max(
                            (local_x * sx * o.size[0] + local_y * sy * o.size[1]) / (o.size[0] * o.size[0] + o.size[1] * o.size[1]),
                        )
                    };
                    width = o.size[0] * factor;
                    height = o.size[1] * factor;
                }
                result.size = [width, height];
                if mirror_x {
                    result.flip_x = !result.flip_x;
                }
                if mirror_y {
                    result.flip_y = !result.flip_y;
                }
                let ox = (0.5 - anchor_unit[0]) * width * if mirror_x { -1.0 } else { 1.0 };
                let oy = (0.5 - anchor_unit[1]) * height * if mirror_y { -1.0 } else { 1.0 };
                let c = [anchor[0] + ox * r.cos() - oy * r.sin(), anchor[1] + ox * r.sin() + oy * r.cos()];
                result.origin = [c[0] - width / 2.0, c[1] - height / 2.0];
            }
        }
        if is_valid(&result) { result } else { *o }
    }
}

/// How close, in screen points, a snap target pulls (`TransformSnap.distance`).
pub const SNAP_DISTANCE: f64 = 10.0;

/// The smallest move that puts one of `guides` on one of `targets`, and the target it met.
fn shift(guides: &[f64], targets: &[f64], tolerance: f64) -> (f64, Option<f64>) {
    let mut best: Option<(f64, f64)> = None;
    for g in guides {
        for t in targets {
            let m = t - g;
            if m.abs() > tolerance {
                continue;
            }
            if best.is_some_and(|(b, _)| b.abs() <= m.abs()) {
                continue;
            }
            best = Some((m, *t));
        }
    }
    (best.map_or(0.0, |b| b.0), best.map(|b| b.1))
}

/// `TransformSnap.offset`: `box` [min x, min y, max x, max y] moved so its nearest edge or center
/// meets a target on each axis within `tolerance`, and the targets met.
pub fn snap_offset(b: [f64; 4], xs: &[f64], ys: &[f64], tolerance: f64) -> ([f64; 2], Option<f64>, Option<f64>) {
    let (mx, tx) = shift(&[b[0], (b[0] + b[2]) / 2.0, b[2]], xs, tolerance);
    let (my, ty) = shift(&[b[1], (b[1] + b[3]) / 2.0, b[3]], ys, tolerance);
    ([mx, my], tx, ty)
}

pub fn nearest(value: f64, lines: &[f64], tolerance: f64) -> Option<f64> {
    lines.iter().copied().filter(|l| (l - value).abs() <= tolerance).min_by(|a, b| (a - value).abs().total_cmp(&(b - value).abs()))
}

/// `snappedResizePoint`: the pointer nudged so the dragged edges land on nearby targets
/// (upright layers only). `update` is the drag's own result for a pointer.
pub fn snapped_resize_point(p: Point, drag: &TransformDrag, proportional: bool, xs: &[f64], ys: &[f64], tolerance: f64, update: impl Fn(Point) -> Transform) -> (Point, Vec<f64>, Vec<f64>) {
    let Mode::Resize(index) = drag.mode else { return (p, vec![], vec![]) };
    if radians(&drag.original) != 0.0 {
        return (p, vec![], vec![]);
    }
    let handle = HANDLES[index];
    let grab = point(&drag.original, handle);
    let at = [grab[0] + p[0] - drag.start[0], grab[1] + p[1] - drag.start[1]];
    let edge = |t: &Transform, horizontal: bool| {
        let (min, max) = if horizontal { (t.origin[0], t.origin[0] + t.size[0]) } else { (t.origin[1], t.origin[1] + t.size[1]) };
        let a = if horizontal { at[0] } else { at[1] };
        if (min - a).abs() <= (max - a).abs() { min } else { max }
    };
    let draft = update(p);
    let mut snaps: Vec<(bool, f64)> = Vec::new();
    if handle[0] != 0.5 {
        if let Some(x) = nearest(edge(&draft, true), xs, tolerance) {
            snaps.push((true, x));
        }
    }
    if handle[1] != 0.5 {
        if let Some(y) = nearest(edge(&draft, false), ys, tolerance) {
            snaps.push((false, y));
        }
    }
    if proportional && snaps.len() == 2 {
        let keep = *snaps.iter().min_by(|a, b| (a.1 - edge(&draft, a.0)).abs().total_cmp(&(b.1 - edge(&draft, b.0)).abs())).unwrap();
        snaps = vec![keep];
    }
    let mut result = p;
    for (horizontal, target) in &snaps {
        let before = edge(&update(result), *horizontal);
        let mut nudged = result;
        if *horizontal {
            nudged[0] += 1.0
        } else {
            nudged[1] += 1.0
        }
        let per_pixel = edge(&update(nudged), *horizontal) - before;
        if per_pixel.abs() <= 0.01 {
            continue;
        }
        let s = (target - before) / per_pixel;
        if *horizontal {
            result[0] += s
        } else {
            result[1] += s
        }
    }
    let gx = snaps.iter().filter(|s| s.0).map(|s| s.1).collect();
    let gy = snaps.iter().filter(|s| !s.0).map(|s| s.1).collect();
    (result, gx, gy)
}

/// A crop frame [x, y, width, height].
pub type CropRect = [f64; 4];

/// `CropGeometry.snapped`: whole pixels, at least 1 × 1.
pub fn crop_snapped(r: CropRect) -> CropRect {
    let (x0, x1) = (r[0].min(r[0] + r[2]), r[0].max(r[0] + r[2]));
    let (y0, y1) = (r[1].min(r[1] + r[3]), r[1].max(r[1] + r[3]));
    let (x, y) = (x0.round(), y0.round());
    [x, y, (x1.round() - x).max(1.0), (y1.round() - y).max(1.0)]
}

pub fn crop_valid(r: CropRect) -> bool {
    r.iter().all(|v| v.is_finite()) && (1.0..=30_000.0).contains(&r[2]) && (1.0..=30_000.0).contains(&r[3]) && r[0].abs() <= 1_000_000.0 && r[1].abs() <= 1_000_000.0
}

/// `CropGeometry.create`.
pub fn crop_create(start: Point, end: Point, ratio: Option<f64>, symmetric: bool) -> CropRect {
    let (mut dx, mut dy) = (end[0] - start[0], end[1] - start[1]);
    if let Some(ratio) = ratio {
        if dx.abs() > dy.abs() * ratio {
            dy = if dy < 0.0 { -1.0 } else { 1.0 } * dx.abs() / ratio;
        } else {
            dx = if dx < 0.0 { -1.0 } else { 1.0 } * dy.abs() * ratio;
        }
    }
    if symmetric {
        return crop_snapped([start[0] - dx.abs(), start[1] - dy.abs(), dx.abs() * 2.0, dy.abs() * 2.0]);
    }
    crop_snapped([start[0].min(start[0] + dx), start[1].min(start[1] + dy), dx.abs(), dy.abs()])
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CropMode {
    Create,
    Move,
    Resize(usize),
}

/// `CropDrag`.
#[derive(Clone, Copy, Debug)]
pub struct CropDrag {
    pub start: Point,
    pub original: CropRect,
    pub mode: CropMode,
}

impl CropDrag {
    pub fn updated(&self, p: Point, ratio: Option<f64>, symmetric: bool) -> CropRect {
        let o = self.original;
        match self.mode {
            CropMode::Create => crop_create(self.start, p, ratio, symmetric),
            CropMode::Move => crop_snapped([o[0] + p[0] - self.start[0], o[1] + p[1] - self.start[1], o[2], o[3]]),
            CropMode::Resize(index) => {
                let t = Transform::at(o[0], o[1], o[2], o[3]);
                let drag = TransformDrag { original: t, start: self.start, mode: Mode::Resize(index) };
                let next = drag.updated(p, ratio.is_some(), false, symmetric);
                crop_snapped([next.origin[0], next.origin[1], next.size[0], next.size[1]])
            }
        }
    }
}

/// `CropSnap.apply`: moving snaps the nearest edges; creating or resizing snaps the dragged ones.
pub fn crop_snap(rect: CropRect, drag: &CropDrag, p: Point, ratio: Option<f64>, symmetric: bool, xs: &[f64], ys: &[f64], tolerance: f64) -> CropRect {
    if tolerance <= 0.0 {
        return rect;
    }
    let (min_x, min_y, max_x, max_y) = (rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]);
    let (horizontal, vertical) = match drag.mode {
        CropMode::Move => {
            let shift = |edges: [f64; 2], targets: &[f64]| {
                edges.iter().filter_map(|e| nearest(*e, targets, tolerance).map(|t| t - e)).min_by(|a, b| a.abs().total_cmp(&b.abs())).unwrap_or(0.0)
            };
            return [rect[0] + shift([min_x, max_x], xs), rect[1] + shift([min_y, max_y], ys), rect[2], rect[3]];
        }
        CropMode::Create => {
            if ratio.is_some() {
                return rect;
            }
            (true, true)
        }
        CropMode::Resize(index) => {
            if ratio.is_some() {
                return rect;
            }
            (HANDLES[index][0] != 0.5, HANDLES[index][1] != 0.5)
        }
    };
    let (mut x0, mut y0, mut x1, mut y1) = (min_x, min_y, max_x, max_y);
    if horizontal {
        if (p[0] - x0).abs() <= (p[0] - x1).abs() {
            if let Some(x) = nearest(x0, xs, tolerance).filter(|x| *x < x1) {
                x0 = x;
            }
        } else if let Some(x) = nearest(x1, xs, tolerance).filter(|x| *x > x0) {
            x1 = x;
        }
    }
    if vertical {
        if (p[1] - y0).abs() <= (p[1] - y1).abs() {
            if let Some(y) = nearest(y0, ys, tolerance).filter(|y| *y < y1) {
                y0 = y;
            }
        } else if let Some(y) = nearest(y1, ys, tolerance).filter(|y| *y > y0) {
            y1 = y;
        }
    }
    if symmetric {
        let c = match drag.mode {
            CropMode::Create => drag.start,
            _ => [drag.original[0] + drag.original[2] / 2.0, drag.original[1] + drag.original[3] / 2.0],
        };
        if horizontal {
            let half = if p[0] >= c[0] { x1 - c[0] } else { c[0] - x0 };
            if half >= 0.5 {
                x0 = c[0] - half;
                x1 = c[0] + half;
            }
        }
        if vertical {
            let half = if p[1] >= c[1] { y1 - c[1] } else { c[1] - y0 };
            if half >= 0.5 {
                y0 = c[1] - half;
                y1 = c[1] + half;
            }
        }
    }
    [x0, y0, x1 - x0, y1 - y0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_drag_scales_proportionally_and_flips_past_the_anchor() {
        let t = Transform::at(10.0, 10.0, 100.0, 50.0);
        let drag = TransformDrag { original: t, start: [110.0, 60.0], mode: Mode::Resize(4) };
        let r = drag.updated([210.0, 110.0], true, false, false);
        assert_eq!((r.origin, r.size), ([10.0, 10.0], [200.0, 100.0]));
        // Unlocked, past the left edge: flipped horizontally.
        let r = drag.updated([-40.0, 60.0], false, false, false);
        assert!(r.flip_x);
        assert_eq!(r.size, [50.0, 50.0]);
        assert_eq!(r.origin, [-40.0, 10.0]);
    }

    #[test]
    fn move_snaps_to_the_canvas_edge() {
        let b = [3.0, 40.0, 53.0, 90.0];
        let (offset, x, y) = snap_offset(b, &[0.0, 100.0], &[0.0, 200.0], 5.0);
        assert_eq!(offset, [-3.0, 0.0]);
        assert_eq!((x, y), (Some(0.0), None));
    }

    #[test]
    fn following_a_move_moves_the_placement() {
        let mask = Transform::at(0.0, 0.0, 10.0, 10.0);
        let old = Transform::at(0.0, 0.0, 20.0, 20.0);
        let mut new = old;
        new.origin = [5.0, 7.0];
        assert_eq!(following(&mask, &old, &new).origin, [5.0, 7.0]);
        let mut scaled = old;
        scaled.size = [40.0, 40.0];
        let f = following(&mask, &old, &scaled);
        assert!((f.size[0] - 20.0).abs() < 1e-9 && f.origin[0].abs() < 1e-9);
    }
}
