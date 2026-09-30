//! Selection outlines as polygons in document pixels, and the path operations the Mac runs on
//! them (`CGPath.union`, `intersection`, `subtracting` and `copy(strokingWithWidth:)`), through
//! iOverlay. Curves are flattened as Core Graphics flattens them to fill, which shows in the
//! coverage.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;

pub type Point = [f64; 2];
/// One closed loop; the last point joins the first.
pub type Contour = Vec<Point>;
/// Closed loops filled by the nonzero winding rule.
pub type Region = Vec<Contour>;

/// How far a cubic's control points may bend (their second differences, along x or y) before
/// Core Graphics splits it to fill, in pixels: fitted to the Mac's ellipses, where any value from
/// 0.0658 to 0.0699 draws the same.
const FLATNESS: f64 = 1.0 / 15.0;

pub fn rect(x: f64, y: f64, w: f64, h: f64) -> Contour {
    vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
}

/// `CGPath.addEllipse(in:)`: four cubic Béziers through the midpoints of the rectangle's sides,
/// with control points at 0.5522847498 of each half axis, as Core Graphics builds it.
pub fn ellipse(x: f64, y: f64, w: f64, h: f64) -> Contour {
    const K: f64 = 0.552_284_749_830_793_4;
    let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
    let (ox, oy) = (rx * K, ry * K);
    let quarters = [
        [[cx + rx, cy], [cx + rx, cy + oy], [cx + ox, cy + ry], [cx, cy + ry]],
        [[cx, cy + ry], [cx - ox, cy + ry], [cx - rx, cy + oy], [cx - rx, cy]],
        [[cx - rx, cy], [cx - rx, cy - oy], [cx - ox, cy - ry], [cx, cy - ry]],
        [[cx, cy - ry], [cx + ox, cy - ry], [cx + rx, cy - oy], [cx + rx, cy]],
    ];
    let mut out = Vec::new();
    for q in quarters {
        flatten_cubic(q, &mut out);
    }
    out
}

/// Appends a cubic's points after its first, as Core Graphics flattens it to fill: halved until
/// both second differences of its control points are within `FLATNESS` along x and along y.
fn flatten_cubic(p: [Point; 4], out: &mut Contour) {
    flatten_piece(p, out, 0);
}

fn flatten_piece(p: [Point; 4], out: &mut Contour, depth: u32) {
    let second = |a: Point, b: Point, c: Point| (a[0] - 2.0 * b[0] + c[0]).abs().max((a[1] - 2.0 * b[1] + c[1]).abs());
    if depth >= 16 || second(p[0], p[1], p[2]).max(second(p[1], p[2], p[3])) <= FLATNESS {
        out.push(p[3]);
        return;
    }
    let mid = |a: Point, b: Point| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let (ab, bc, cd) = (mid(p[0], p[1]), mid(p[1], p[2]), mid(p[2], p[3]));
    let (abc, bcd) = (mid(ab, bc), mid(bc, cd));
    let m = mid(abc, bcd);
    flatten_piece([p[0], ab, abc, m], out, depth + 1);
    flatten_piece([m, bcd, cd, p[3]], out, depth + 1);
}

fn run(a: &Region, b: &Region, rule: OverlayRule) -> Region {
    if a.is_empty() && b.is_empty() {
        return Vec::new();
    }
    a.overlay(b, rule, FillRule::NonZero).into_iter().flatten().filter(|c| c.len() >= 3).collect()
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
/// every corner. Core Graphics draws each join as one cubic arc, however sharp the corner, and
/// the arc is flattened as any cubic is.
pub fn stroke_band(region: &Region, half: f64) -> Region {
    let mut pieces: Region = Vec::new();
    let mut add = |mut piece: Contour| {
        if signed_area(&piece) < 0.0 {
            piece.reverse();
        }
        if piece.len() >= 3 {
            pieces.push(piece);
        }
    };
    for contour in region {
        let n = contour.len();
        for i in 0..n {
            let (a, b) = (contour[i], contour[(i + 1) % n]);
            let length = (b[0] - a[0]).hypot(b[1] - a[1]);
            if length == 0.0 {
                continue;
            }
            let (nx, ny) = (-(b[1] - a[1]) / length * half, (b[0] - a[0]) / length * half);
            add(vec![[a[0] + nx, a[1] + ny], [b[0] + nx, b[1] + ny], [b[0] - nx, b[1] - ny], [a[0] - nx, a[1] - ny]]);
        }
        for i in 0..n {
            let (p, v, q) = (contour[(i + n - 1) % n], contour[i], contour[(i + 1) % n]);
            let incoming = (v[1] - p[1]).atan2(v[0] - p[0]);
            let outgoing = (q[1] - v[1]).atan2(q[0] - v[0]);
            let turn = wrap(outgoing - incoming);
            if turn == 0.0 || v == p || v == q {
                continue;
            }
            // From the incoming edge's normal to the outgoing edge's, on the outside of the turn.
            let side = if turn > 0.0 { -1.0 } else { 1.0 };
            let start = incoming + side * std::f64::consts::FRAC_PI_2;
            let sweep = wrap(outgoing - incoming);
            let end = start + sweep;
            let handle = 4.0 / 3.0 * (sweep / 4.0).tan() * half;
            let (s0, c0, s1, c1) = (start.sin(), start.cos(), end.sin(), end.cos());
            let p0 = [v[0] + half * c0, v[1] + half * s0];
            let p3 = [v[0] + half * c1, v[1] + half * s1];
            let arc = [p0, [p0[0] - handle * s0, p0[1] + handle * c0], [p3[0] + handle * s1, p3[1] - handle * c1], p3];
            let mut sector = vec![v, p0];
            flatten_cubic(arc, &mut sector);
            add(sector);
        }
    }
    run(&pieces, &Vec::new(), OverlayRule::Union)
}

/// An angle wrapped into (-π, π].
fn wrap(a: f64) -> f64 {
    let t = std::f64::consts::TAU;
    let w = a - t * (a / t).round();
    if w <= -std::f64::consts::PI { w + t } else { w }
}

/// `CGPath.boundingBoxOfPath` is empty or null: nothing is selected.
pub fn is_empty(region: &Region) -> bool {
    region.iter().all(|c| c.len() < 3 || signed_area(c) == 0.0)
}

pub fn signed_area(c: &Contour) -> f64 {
    (0..c.len()).map(|i| {
        let (a, b) = (c[i], c[(i + 1) % c.len()]);
        a[0] * b[1] - b[0] * a[1]
    }).sum::<f64>()
        / 2.0
}

/// The region's points mapped through `m` ([a, b, c, d, tx, ty], as a `CGAffineTransform`).
pub fn transformed(region: &Region, m: [f64; 6]) -> Region {
    region.iter().map(|c| c.iter().map(|p| [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]]).collect()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_of_overlapping_squares() {
        let a = vec![rect(0.0, 0.0, 4.0, 4.0)];
        let b = vec![rect(2.0, 2.0, 4.0, 4.0)];
        let area: f64 = union(&a, &b).iter().map(signed_area).sum();
        assert!((area.abs() - 28.0).abs() < 1e-6);
        let cut: f64 = subtracting(&a, &b).iter().map(signed_area).sum();
        assert!((cut.abs() - 12.0).abs() < 1e-6);
    }

    #[test]
    fn ellipse_area_is_close_to_pi_ab() {
        let e = ellipse(0.0, 0.0, 40.0, 20.0);
        let area = signed_area(&e).abs();
        // Chords cut a little off the curve.
        assert!(area < std::f64::consts::PI * 200.0 && area > std::f64::consts::PI * 200.0 - 2.0, "{area}");
    }

    #[test]
    fn a_band_rounds_the_outside_corners() {
        let square = vec![rect(0.0, 0.0, 10.0, 10.0)];
        let area = |r: &Region| r.iter().map(signed_area).sum::<f64>().abs();
        // 14×14 with rounded corners of radius 2, less the 6×6 inside; chords cut a little off.
        let expected = 14.0 * 14.0 - (4.0 - std::f64::consts::PI) * 4.0 - 36.0;
        let band = stroke_band(&square, 2.0);
        assert!(area(&band) < expected && area(&band) > expected - 0.5, "{}", area(&band));
        // A band wider than the shape covers it with no holes.
        let tiny = vec![vec![[0.0, 0.0], [3.0, 0.5], [1.0, 3.0]]];
        assert_eq!(union(&tiny, &stroke_band(&tiny, 20.0)).len(), 1);
    }
}
