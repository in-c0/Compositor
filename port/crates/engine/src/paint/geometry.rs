//! `CGRect` arithmetic as `BrushStroke` uses it, and the stroke's path: samples turned into the
//! segments `MetalBrushCoverage` sweeps the tip along (`appendContinuous`, `continuousCurve`).

/// A `CGRect` in the stroke's pixel grid or in document pixels. `None` stands for `CGRect.null`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }
    pub fn max_x(&self) -> f64 {
        self.x + self.w
    }
    pub fn max_y(&self) -> f64 {
        self.y + self.h
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
    /// `CGRect.intersection`: `None` when they don't meet.
    pub fn intersection(&self, o: &Rect) -> Option<Rect> {
        let (x0, y0) = (self.x.max(o.x), self.y.max(o.y));
        let (x1, y1) = (self.max_x().min(o.max_x()), self.max_y().min(o.max_y()));
        (x1 >= x0 && y1 >= y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let (x0, y0) = (self.x.min(o.x), self.y.min(o.y));
        Rect::new(x0, y0, self.max_x().max(o.max_x()) - x0, self.max_y().max(o.max_y()) - y0)
    }
    /// `CGRect.integral`.
    pub fn integral(&self) -> Rect {
        let (x0, y0) = (self.x.floor(), self.y.floor());
        Rect::new(x0, y0, self.max_x().ceil() - x0, self.max_y().ceil() - y0)
    }
    pub fn inset(&self, dx: f64, dy: f64) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w - 2.0 * dx, self.h - 2.0 * dy)
    }
    pub fn offset(&self, dx: f64, dy: f64) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }
}

pub type Point = [f64; 2];

/// One segment of the tip's path, in document pixels, as the kernel reads it.
pub type Segment = [f32; 4];

fn segment(a: Point, b: Point) -> Segment {
    [a[0] as f32, a[1] as f32, b[0] as f32, b[1] as f32]
}

/// The stroke's samples and the kernel calls they make. `BrushStroke.append` and `flush` on the
/// GPU path: each call sweeps its settled segments into the permanent coverage and draws a
/// provisional tail, which the next call replaces.
#[derive(Default)]
pub struct Path {
    samples: Vec<Point>,
    /// Every kernel call's settled segments and tail, in order.
    pub calls: Vec<(Vec<Segment>, Vec<Segment>)>,
}

impl Path {
    /// `BrushStroke.append`.
    pub fn append(&mut self, point: Point) {
        if !point[0].is_finite() || !point[1].is_finite() || point[0].abs() > 10_000_000.0 || point[1].abs() > 10_000_000.0 {
            return;
        }
        if self.samples.last() == Some(&point) {
            return;
        }
        self.samples.push(point);
        if self.samples.len() > 4 {
            self.samples.remove(0);
        }
        let s = &self.samples;
        let n = s.len();
        let settled = if n == 1 {
            vec![segment(point, point)]
        } else if n >= 3 {
            continuous_curve(s[n - 3], s[n - 2], s[if n >= 4 { n - 4 } else { 0 }], point)
        } else {
            vec![]
        };
        let tail = if n >= 2 { vec![segment(s[n - 2], point)] } else { vec![] };
        self.calls.push((settled, tail));
    }

    /// `BrushStroke.flush`: the last curve piece, settled. Safe to repeat.
    pub fn flush(&mut self) {
        let n = self.samples.len();
        if n < 2 {
            return;
        }
        let s = &self.samples;
        let settled = continuous_curve(s[n - 2], s[n - 1], s[if n >= 3 { n - 3 } else { 0 }], s[n - 1]);
        self.calls.push((settled, vec![]));
        self.samples = vec![s[n - 1]];
    }

    /// Every settled segment and how many each call adds, for the coverage kernel.
    pub fn settled(&self) -> (Vec<Segment>, Vec<u32>) {
        let mut segments = Vec::new();
        let mut ends = Vec::new();
        for (settled, _) in &self.calls {
            segments.extend_from_slice(settled);
            ends.push(segments.len() as u32);
        }
        (segments, ends)
    }
}

/// `continuousCurve`: a centripetal Catmull–Rom piece from `start` to `end`, split into chords
/// until each stays within 0.2 document pixels of the curve.
fn continuous_curve(start: Point, end: Point, before: Point, after: Point) -> Vec<Segment> {
    let knot = |t: f64, a: Point, b: Point| t + 0.0001f64.max((b[0] - a[0]).hypot(b[1] - a[1]).sqrt());
    let mix = |a: Point, b: Point, ta: f64, tb: f64, t: f64| {
        let (wa, wb) = ((tb - t) / (tb - ta), (t - ta) / (tb - ta));
        [a[0] * wa + b[0] * wb, a[1] * wa + b[1] * wb]
    };
    let t0 = 0.0;
    let t1 = knot(t0, before, start);
    let t2 = knot(t1, start, end);
    let t3 = knot(t2, end, after);
    let point = |u: f64| -> Point {
        if u == 0.0 {
            return start;
        }
        if u == 1.0 {
            return end;
        }
        let t = t1 + (t2 - t1) * u;
        let (a, b, c) = (mix(before, start, t0, t1, t), mix(start, end, t1, t2, t), mix(end, after, t2, t3, t));
        mix(mix(a, b, t0, t2, t), mix(b, c, t1, t3, t), t1, t2, t)
    };
    let mut result = Vec::new();
    fn subdivide(point: &dyn Fn(f64) -> Point, a: Point, b: Point, lo: f64, hi: f64, depth: u32, result: &mut Vec<Segment>) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let length_squared = dx * dx + dy * dy;
        let error = |p: Point| {
            let t = if length_squared > 0.0 { 1f64.min(0f64.max(((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length_squared)) } else { 0.0 };
            (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
        };
        let mid = (lo + hi) / 2.0;
        let m = point(mid);
        let deviation = error(m).max(error(point((lo + mid) / 2.0))).max(error(point((mid + hi) / 2.0)));
        if deviation <= 0.2 || depth >= 10 {
            result.push(segment(a, b));
            return;
        }
        subdivide(point, a, m, lo, mid, depth + 1, result);
        subdivide(point, m, b, mid, hi, depth + 1, result);
    }
    subdivide(&point, start, end, 0.0, 1.0, 0, &mut result);
    result
}
