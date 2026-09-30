//! Selection outlines in document pixels, and the path operations the Mac runs on them
//! (`CGPath.union`, `intersection`, `subtracting` and `copy(strokingWithWidth:)`).
//!
//! An outline is lines and cubic Béziers, as a `CGPath` is. The path operations run on polygons
//! through iOverlay, with each cubic flattened as Core Graphics flattens it; afterwards every run
//! of chords that came from one cubic becomes that cubic again, or the piece of it that survived.
//! That is what Core Graphics does too, and its pieces show how: a cubic is cut where its
//! flattened chords cross the other outline, split at the parameter interpolated along the chord,
//! and the piece's end moved onto the crossing. The pieces are then flattened afresh to fill,
//! which the coverage shows.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use std::collections::HashMap;

pub type Point = [f64; 2];

/// One piece of an outline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Line(Point, Point),
    Cubic([Point; 4]),
}

impl Seg {
    pub fn start(&self) -> Point {
        match self {
            Seg::Line(a, _) => *a,
            Seg::Cubic(p) => p[0],
        }
    }

    fn map(&self, f: impl Fn(Point) -> Point) -> Seg {
        match self {
            Seg::Line(a, b) => Seg::Line(f(*a), f(*b)),
            Seg::Cubic(p) => Seg::Cubic(p.map(f)),
        }
    }
}

/// One closed loop: each piece starts where the last one ended, and the last ends at the start.
pub type Contour = Vec<Seg>;
/// Closed loops filled by the nonzero winding rule.
pub type Region = Vec<Contour>;

/// How far a cubic's control points may bend (their second differences, along x or y) before
/// Core Graphics splits it to fill, in pixels: fitted to the Mac's ellipses, where any value from
/// 0.0658 to 0.0699 draws the same.
const FLATNESS: f64 = 1.0 / 15.0;

/// Straight lines through `points`, closed.
pub fn polygon(points: &[Point]) -> Contour {
    (0..points.len()).map(|i| Seg::Line(points[i], points[(i + 1) % points.len()])).collect()
}

pub fn rect(x: f64, y: f64, w: f64, h: f64) -> Contour {
    polygon(&[[x, y], [x + w, y], [x + w, y + h], [x, y + h]])
}

/// `CGPath.addEllipse(in:)`: four cubic Béziers through the midpoints of the rectangle's sides,
/// with control points at 0.5522847498 of each half axis, as Core Graphics builds it.
pub fn ellipse(x: f64, y: f64, w: f64, h: f64) -> Contour {
    const K: f64 = 0.552_284_749_830_793_4;
    let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
    let (ox, oy) = (rx * K, ry * K);
    vec![
        Seg::Cubic([[cx + rx, cy], [cx + rx, cy + oy], [cx + ox, cy + ry], [cx, cy + ry]]),
        Seg::Cubic([[cx, cy + ry], [cx - ox, cy + ry], [cx - rx, cy + oy], [cx - rx, cy]]),
        Seg::Cubic([[cx - rx, cy], [cx - rx, cy - oy], [cx - ox, cy - ry], [cx, cy - ry]]),
        Seg::Cubic([[cx, cy - ry], [cx + ox, cy - ry], [cx + rx, cy - oy], [cx + rx, cy]]),
    ]
}

/// The contour as Core Graphics fills it: each piece's start, then the points a cubic is
/// flattened to.
pub fn flatten(contour: &Contour) -> Vec<Point> {
    let mut out = Vec::new();
    for seg in contour {
        out.push(seg.start());
        if let Seg::Cubic(p) = seg {
            let mut points = Vec::new();
            flatten_piece(*p, 0.0, 1.0, &mut points, 0);
            points.pop();
            out.extend(points.into_iter().map(|(p, _)| p));
        }
    }
    out
}

/// Appends the cubic's flattened points after its first, with their parameters: halved until both
/// second differences of its control points are within `FLATNESS` along x and along y.
fn flatten_piece(p: [Point; 4], t0: f64, t1: f64, out: &mut Vec<(Point, f64)>, depth: u32) {
    let second = |a: Point, b: Point, c: Point| (a[0] - 2.0 * b[0] + c[0]).abs().max((a[1] - 2.0 * b[1] + c[1]).abs());
    if depth >= 16 || second(p[0], p[1], p[2]).max(second(p[1], p[2], p[3])) <= FLATNESS {
        out.push((p[3], t1));
        return;
    }
    let (a, b) = halves(p);
    let m = (t0 + t1) / 2.0;
    flatten_piece(a, t0, m, out, depth + 1);
    flatten_piece(b, m, t1, out, depth + 1);
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn halves(p: [Point; 4]) -> ([Point; 4], [Point; 4]) {
    split(p, 0.5)
}

/// de Casteljau at `t`.
fn split(p: [Point; 4], t: f64) -> ([Point; 4], [Point; 4]) {
    let (ab, bc, cd) = (lerp(p[0], p[1], t), lerp(p[1], p[2], t), lerp(p[2], p[3], t));
    let (abc, bcd) = (lerp(ab, bc, t), lerp(bc, cd, t));
    let m = lerp(abc, bcd, t);
    ([p[0], ab, abc, m], [m, bcd, cd, p[3]])
}

/// The part of `p` between parameters `t0` < `t1`.
fn piece(p: [Point; 4], t0: f64, t1: f64) -> [Point; 4] {
    let right = if t0 > 0.0 { split(p, t0).1 } else { p };
    if t1 >= 1.0 { right } else { split(right, (t1 - t0) / (1.0 - t0)).0 }
}

/// A cubic of an operand, flattened, with each point's parameter.
struct Curve {
    cubic: [Point; 4],
    points: Vec<Point>,
    params: Vec<f64>,
}

/// Where points of the flattened operands came from, to find them again in iOverlay's output.
#[derive(Default)]
struct Origins {
    curves: Vec<Curve>,
    /// Flattened points on a coarse grid, with (curve, parameter) for each.
    at: HashMap<(i64, i64), Vec<(Point, usize, f64)>>,
}

const MATCH: f64 = 1e-6;

impl Origins {
    fn key(p: Point) -> (i64, i64) {
        ((p[0] * 64.0).floor() as i64, (p[1] * 64.0).floor() as i64)
    }

    /// The operand as polygons for iOverlay.
    fn polygons(&mut self, region: &Region) -> Vec<Vec<Point>> {
        let mut out = Vec::new();
        for contour in region {
            let mut points = Vec::new();
            for seg in contour {
                points.push(seg.start());
                if let Seg::Cubic(p) = seg {
                    let mut flat = vec![(p[0], 0.0)];
                    flatten_piece(*p, 0.0, 1.0, &mut flat, 0);
                    let id = self.curves.len();
                    for &(q, t) in &flat {
                        self.at.entry(Self::key(q)).or_default().push((q, id, t));
                    }
                    points.extend(flat[1..flat.len() - 1].iter().map(|&(q, _)| q));
                    self.curves.push(Curve { cubic: *p, points: flat.iter().map(|f| f.0).collect(), params: flat.iter().map(|f| f.1).collect() });
                }
            }
            if points.len() >= 3 {
                out.push(points);
            }
        }
        out
    }

    /// The curves (and parameters) an output point lies on: a flattened point itself, or a
    /// crossing on one of the chords, its parameter interpolated along the chord.
    fn tags(&self, p: Point) -> Vec<(usize, f64)> {
        let (kx, ky) = Self::key(p);
        let mut found = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &(q, id, t) in self.at.get(&(kx + dx, ky + dy)).into_iter().flatten() {
                    if (q[0] - p[0]).abs() <= MATCH && (q[1] - p[1]).abs() <= MATCH {
                        found.push((id, t));
                    }
                }
            }
        }
        if !found.is_empty() {
            return found;
        }
        for (id, curve) in self.curves.iter().enumerate() {
            for k in 0..curve.points.len() - 1 {
                let (a, b) = (curve.points[k], curve.points[k + 1]);
                if p[0] < a[0].min(b[0]) - MATCH || p[0] > a[0].max(b[0]) + MATCH || p[1] < a[1].min(b[1]) - MATCH || p[1] > a[1].max(b[1]) + MATCH {
                    continue;
                }
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let length2 = dx * dx + dy * dy;
                if length2 == 0.0 {
                    continue;
                }
                let f = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length2;
                let off = ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / length2.sqrt();
                if off <= MATCH && (-1e-9..=1.0 + 1e-9).contains(&f) {
                    found.push((id, curve.params[k] + (curve.params[k + 1] - curve.params[k]) * f.clamp(0.0, 1.0)));
                }
            }
        }
        found
    }

    /// Whether parameters `a` and `b` of curve `id` lie on one chord.
    fn same_chord(&self, id: usize, a: f64, b: f64) -> bool {
        let (lo, hi) = (a.min(b), a.max(b));
        let params = &self.curves[id].params;
        params.windows(2).any(|w| w[0] <= lo + 1e-12 && hi <= w[1] + 1e-12) && hi > lo
    }

    /// iOverlay's output polygon as an outline, with its runs of curve chords made curves again.
    fn outline(&self, points: &[Point]) -> Contour {
        let n = points.len();
        let tags: Vec<Vec<(usize, f64)>> = points.iter().map(|&p| self.tags(p)).collect();
        // Each edge on a curve: (curve, parameter at its start, at its end).
        let on: Vec<Option<(usize, f64, f64)>> = (0..n)
            .map(|i| {
                let j = (i + 1) % n;
                tags[i].iter().find_map(|&(id, a)| {
                    tags[j].iter().find(|&&(other, b)| other == id && self.same_chord(id, a, b)).map(|&(_, b)| (id, a, b))
                })
            })
            .collect();
        let continues = |i: usize| {
            let prev = (i + n - 1) % n;
            match (on[prev], on[i]) {
                (Some((c0, a0, b0)), Some((c1, a1, b1))) => c0 == c1 && b0 == a1 && (b0 - a0).signum() == (b1 - a1).signum(),
                _ => false,
            }
        };
        // Start at an edge that begins a run, so no run wraps around.
        let first = (0..n).find(|&i| !continues(i)).unwrap_or(0);
        let mut out = Vec::new();
        let mut i = 0;
        while i < n {
            let e = (first + i) % n;
            let start = points[e];
            match on[e] {
                None => {
                    out.push(Seg::Line(start, points[(e + 1) % n]));
                    i += 1;
                }
                Some((id, t_start, _)) => {
                    let mut last = e;
                    i += 1;
                    while i < n && continues((first + i) % n) {
                        last = (first + i) % n;
                        i += 1;
                    }
                    let t_end = on[last].unwrap().2;
                    let end = points[(last + 1) % n];
                    let cubic = self.curves[id].cubic;
                    let mut part = if t_start < t_end {
                        piece(cubic, t_start, t_end)
                    } else {
                        let p = piece(cubic, t_end, t_start);
                        [p[3], p[2], p[1], p[0]]
                    };
                    // The piece ends where the chords were cut, not on the curve.
                    part[0] = start;
                    part[3] = end;
                    out.push(Seg::Cubic(part));
                }
            }
        }
        out
    }
}

fn run(a: &Region, b: &Region, rule: OverlayRule) -> Region {
    let mut origins = Origins::default();
    let (subject, clip) = (origins.polygons(a), origins.polygons(b));
    if subject.is_empty() && clip.is_empty() {
        return Vec::new();
    }
    subject
        .overlay(&clip, rule, FillRule::NonZero)
        .into_iter()
        .flatten()
        .filter(|c| c.len() >= 3)
        .map(|c| origins.outline(&c))
        .collect()
}

/// `a.union(b, using: .winding)`.
pub fn union(a: &Region, b: &Region) -> Region {
    run(a, b, OverlayRule::Union)
}

/// `a.intersection(b, using: .winding)`.
pub fn intersection(a: &Region, b: &Region) -> Region {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    run(a, b, OverlayRule::Intersect)
}

/// `a.subtracting(b, using: .winding)`.
pub fn subtracting(a: &Region, b: &Region) -> Region {
    if a.is_empty() {
        return Vec::new();
    }
    run(a, b, OverlayRule::Difference)
}

/// `path.copy(strokingWithWidth: 2 * half, lineCap: .round, lineJoin: .round, miterLimit: 10)`
/// for a closed outline: the band within `half` of each edge, with a round join on the outside of
/// every corner. Core Graphics draws each join as one cubic arc, however sharp the corner. Curves
/// are stroked along their flattened chords.
pub fn stroke_band(region: &Region, half: f64) -> Region {
    let mut pieces: Region = Vec::new();
    for contour in region {
        let points = flatten(contour);
        let n = points.len();
        for i in 0..n {
            let (a, b) = (points[i], points[(i + 1) % n]);
            let length = (b[0] - a[0]).hypot(b[1] - a[1]);
            if length == 0.0 {
                continue;
            }
            let (nx, ny) = (-(b[1] - a[1]) / length * half, (b[0] - a[0]) / length * half);
            pieces.push(oriented(polygon(&[[a[0] + nx, a[1] + ny], [b[0] + nx, b[1] + ny], [b[0] - nx, b[1] - ny], [a[0] - nx, a[1] - ny]])));
        }
        for i in 0..n {
            let (p, v, q) = (points[(i + n - 1) % n], points[i], points[(i + 1) % n]);
            if v == p || v == q {
                continue;
            }
            let incoming = (v[1] - p[1]).atan2(v[0] - p[0]);
            let outgoing = (q[1] - v[1]).atan2(q[0] - v[0]);
            let sweep = wrap(outgoing - incoming);
            if sweep == 0.0 {
                continue;
            }
            // From the incoming edge's normal to the outgoing edge's, on the outside of the turn.
            let side = if sweep > 0.0 { -1.0 } else { 1.0 };
            let start = incoming + side * std::f64::consts::FRAC_PI_2;
            let end = start + sweep;
            let handle = 4.0 / 3.0 * (sweep / 4.0).tan() * half;
            let (s0, c0, s1, c1) = (start.sin(), start.cos(), end.sin(), end.cos());
            let p0 = [v[0] + half * c0, v[1] + half * s0];
            let p3 = [v[0] + half * c1, v[1] + half * s1];
            let arc = [p0, [p0[0] - handle * s0, p0[1] + handle * c0], [p3[0] + handle * s1, p3[1] - handle * c1], p3];
            pieces.push(oriented(vec![Seg::Line(v, p0), Seg::Cubic(arc), Seg::Line(p3, v)]));
        }
    }
    run(&pieces, &Vec::new(), OverlayRule::Union)
}

/// The contour turned to run counterclockwise (by the shoelace sum), so pieces unite.
fn oriented(contour: Contour) -> Contour {
    if signed_area(&flatten(&contour)) >= 0.0 {
        return contour;
    }
    contour
        .iter()
        .rev()
        .map(|seg| match seg {
            Seg::Line(a, b) => Seg::Line(*b, *a),
            Seg::Cubic(p) => Seg::Cubic([p[3], p[2], p[1], p[0]]),
        })
        .collect()
}

/// An angle wrapped into (-π, π].
fn wrap(a: f64) -> f64 {
    let t = std::f64::consts::TAU;
    let w = a - t * (a / t).round();
    if w <= -std::f64::consts::PI { w + t } else { w }
}

/// `CGPath.boundingBoxOfPath` is empty or null: nothing is selected.
pub fn is_empty(region: &Region) -> bool {
    region.iter().all(|c| signed_area(&flatten(c)) == 0.0)
}

pub fn signed_area(c: &[Point]) -> f64 {
    (0..c.len()).map(|i| {
        let (a, b) = (c[i], c[(i + 1) % c.len()]);
        a[0] * b[1] - b[0] * a[1]
    }).sum::<f64>()
        / 2.0
}

/// The region mapped through `m` ([a, b, c, d, tx, ty], as a `CGAffineTransform`).
pub fn transformed(region: &Region, m: [f64; 6]) -> Region {
    region.iter().map(|c| c.iter().map(|s| s.map(|p| [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]])).collect()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(r: &Region) -> f64 {
        r.iter().map(|c| signed_area(&flatten(c))).sum::<f64>().abs()
    }

    #[test]
    fn union_of_overlapping_squares() {
        let a = vec![rect(0.0, 0.0, 4.0, 4.0)];
        let b = vec![rect(2.0, 2.0, 4.0, 4.0)];
        assert!((area(&union(&a, &b)) - 28.0).abs() < 1e-6);
        assert!((area(&subtracting(&a, &b)) - 12.0).abs() < 1e-6);
    }

    #[test]
    fn ellipse_area_is_close_to_pi_ab() {
        let e = vec![ellipse(0.0, 0.0, 40.0, 20.0)];
        // Chords cut a little off the curve.
        let a = area(&e);
        assert!(a < std::f64::consts::PI * 200.0 && a > std::f64::consts::PI * 200.0 - 2.0, "{a}");
    }

    #[test]
    fn whole_curves_survive_path_operations() {
        let e = vec![ellipse(8.0, 10.0, 48.0, 40.0)];
        let clipped = intersection(&e, &vec![rect(0.0, 0.0, 64.0, 64.0)]);
        let cubics: Vec<[Point; 4]> = clipped.iter().flatten().filter_map(|s| if let Seg::Cubic(p) = s { Some(*p) } else { None }).collect();
        assert_eq!(cubics.len(), 4);
        for original in &e[0] {
            let Seg::Cubic(p) = original else { unreachable!() };
            let reversed = [p[3], p[2], p[1], p[0]];
            assert!(cubics.iter().any(|c| (0..4).all(|k| (0..2).all(|d| (c[k][d] - p[k][d]).abs() < 1e-5 || (c[k][d] - reversed[k][d]).abs() < 1e-5))));
        }
    }

    #[test]
    fn a_cut_curve_keeps_its_piece() {
        // The right of an ellipse cut away: two curves are cut, two survive.
        let e = vec![ellipse(8.0, 10.0, 48.0, 40.0)];
        let cut = subtracting(&e, &vec![rect(45.0, 0.0, 19.0, 64.0)]);
        let cubics = cut.iter().flatten().filter(|s| matches!(s, Seg::Cubic(_))).count();
        assert_eq!(cubics, 4);
        let lines = cut.iter().flatten().filter(|s| matches!(s, Seg::Line(..))).count();
        assert_eq!(lines, 1);
    }

    #[test]
    fn a_band_rounds_the_outside_corners() {
        let square = vec![rect(0.0, 0.0, 10.0, 10.0)];
        // 14×14 with rounded corners of radius 2, less the 6×6 inside; chords cut a little off.
        let expected = 14.0 * 14.0 - (4.0 - std::f64::consts::PI) * 4.0 - 36.0;
        let band = stroke_band(&square, 2.0);
        assert!(area(&band) < expected && area(&band) > expected - 0.5, "{}", area(&band));
        // A band wider than the shape covers it with no holes.
        let tiny = vec![polygon(&[[0.0, 0.0], [3.0, 0.5], [1.0, 3.0]])];
        assert_eq!(union(&tiny, &stroke_band(&tiny, 20.0)).len(), 1);
    }
}
