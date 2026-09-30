//! Selection outlines as polygons in document pixels, and the path operations the Mac runs on
//! them (`CGPath.union`, `intersection`, `subtracting` and `copy(strokingWithWidth:)`), through
//! iOverlay. Curves are flattened as Core Graphics flattens them to fill, which shows in the
//! coverage.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};

pub type Point = [f64; 2];
/// One closed loop; the last point joins the first.
pub type Contour = Vec<Point>;
/// Closed loops filled by the nonzero winding rule.
pub type Region = Vec<Contour>;

/// How far a curve's control points may stray from its chord's thirds before it is split, in
/// pixels (fitted to the Mac's ellipses).
const FLATNESS: f64 = 1.0 / 16.0;

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
/// each piece's inner control points lie within 1/16 px, along x and along y, of the points a
/// third and two thirds along its chord.
fn flatten_cubic(p: [Point; 4], out: &mut Contour) {
    let off = |c: Point, a: Point, b: Point| {
        let (x, y) = ((2.0 * a[0] + b[0]) / 3.0, (2.0 * a[1] + b[1]) / 3.0);
        (c[0] - x).abs().max((c[1] - y).abs())
    };
    let flat = off(p[1], p[0], p[3]).max(off(p[2], p[3], p[0])) <= FLATNESS;
    if flat {
        out.push(p[3]);
        return;
    }
    let mid = |a: Point, b: Point| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let (ab, bc, cd) = (mid(p[0], p[1]), mid(p[1], p[2]), mid(p[2], p[3]));
    let (abc, bcd) = (mid(ab, bc), mid(bc, cd));
    let m = mid(abc, bcd);
    flatten_cubic([p[0], ab, abc, m], out);
    flatten_cubic([m, bcd, cd, p[3]], out);
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

/// The region grown (`distance` > 0) or shrunk (< 0) by `distance`, with round corners: what the
/// Mac gets from `path.copy(strokingWithWidth: 2 * |distance|, lineCap: .round, lineJoin: .round)`
/// added to the path (Expand) or taken from it (Contract). Offsetting draws the same outline
/// without the stroke's inner loops, which iOverlay mishandles once the band is wider than the
/// shape.
pub fn offset(region: &Region, distance: f64) -> Region {
    if region.is_empty() || distance == 0.0 {
        return region.clone();
    }
    // Normalize first: the offset needs outer loops counterclockwise and holes clockwise.
    let normalized = run(region, &Vec::new(), OverlayRule::Union);
    // Round corners as chords 1/64 of the radius long.
    let style = OutlineStyle::new(distance).line_join(LineJoin::Round(1.0 / 64.0));
    normalized.outline(&style).into_iter().flatten().filter(|c| c.len() >= 3).collect()
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
    fn offsets_round_the_corners() {
        let square = vec![rect(0.0, 0.0, 10.0, 10.0)];
        let area = |r: &Region| r.iter().map(signed_area).sum::<f64>().abs();
        // 14×14 with rounded corners of radius 2.
        let grown = 14.0 * 14.0 - (4.0 - std::f64::consts::PI) * 4.0;
        assert!((area(&offset(&square, 2.0)) - grown).abs() < 0.01);
        assert!((area(&offset(&square, -2.0)) - 36.0).abs() < 0.01);
        // Growing a small shape a long way fills it in, with no holes.
        let tiny = vec![vec![[0.0, 0.0], [3.0, 0.5], [1.0, 3.0]]];
        let big = offset(&tiny, 20.0);
        assert_eq!(big.len(), 1);
    }
}
