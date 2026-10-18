//! Geometry sections → paths (SPEC §8.1).
//!
//! Every row type becomes lines and cubic Béziers in the shape's local
//! coordinates. Circular and elliptical arcs are emitted as exact affine images
//! of circle arcs (≤ 90° per cubic). NURBS and splines are flattened.
//! All transcendental maths uses `libm` (SPEC §20.3).

use libm::{atan2, cos, hypot, sin, tan};
use serde::Serialize;

use crate::resolve::{MergedRow, MergedSection};
use crate::value::parse_numeric_call;

const TAU: f64 = core::f64::consts::TAU;
const EPS: f64 = 1e-9;

/// A path segment. Points are `(x, y)` pairs.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "k")]
pub enum Seg {
    M { x: f64, y: f64 },
    L { x: f64, y: f64 },
    C { x1: f64, y1: f64, x2: f64, y2: f64, x: f64, y: f64 },
    Z,
}

impl Seg {
    pub fn end(&self) -> Option<(f64, f64)> {
        match *self {
            Seg::M { x, y } | Seg::L { x, y } | Seg::C { x, y, .. } => Some((x, y)),
            Seg::Z => None,
        }
    }

    pub fn map(&self, f: impl Fn((f64, f64)) -> (f64, f64)) -> Seg {
        match *self {
            Seg::M { x, y } => {
                let (x, y) = f((x, y));
                Seg::M { x, y }
            }
            Seg::L { x, y } => {
                let (x, y) = f((x, y));
                Seg::L { x, y }
            }
            Seg::C { x1, y1, x2, y2, x, y } => {
                let (x1, y1) = f((x1, y1));
                let (x2, y2) = f((x2, y2));
                let (x, y) = f((x, y));
                Seg::C { x1, y1, x2, y2, x, y }
            }
            Seg::Z => Seg::Z,
        }
    }
}

/// One Geometry section as a path.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SectionPath {
    pub segs: Vec<Seg>,
    /// Every sub-path ends where it starts (fillable).
    pub closed: bool,
    pub no_fill: bool,
    pub no_line: bool,
    pub no_show: bool,
}

struct Builder {
    segs: Vec<Seg>,
    cur: (f64, f64),
    start: (f64, f64),
    has_start: bool,
    all_closed: bool,
    open_subpath: bool,
}

impl Builder {
    fn new() -> Self {
        Builder {
            segs: Vec::new(),
            cur: (0.0, 0.0),
            start: (0.0, 0.0),
            has_start: false,
            all_closed: true,
            open_subpath: false,
        }
    }

    fn finish_subpath(&mut self) {
        if self.open_subpath {
            let closed = near(self.cur, self.start);
            if closed {
                self.segs.push(Seg::Z);
            } else {
                self.all_closed = false;
            }
            self.open_subpath = false;
        }
    }

    fn move_to(&mut self, p: (f64, f64)) {
        self.finish_subpath();
        self.segs.push(Seg::M { x: p.0, y: p.1 });
        self.cur = p;
        self.start = p;
        self.has_start = true;
    }

    fn ensure_start(&mut self) {
        if !self.has_start {
            let p = self.cur;
            self.move_to(p);
        }
    }

    fn line_to(&mut self, p: (f64, f64)) {
        self.ensure_start();
        self.segs.push(Seg::L { x: p.0, y: p.1 });
        self.cur = p;
        self.open_subpath = true;
    }

    fn cubic_to(&mut self, c1: (f64, f64), c2: (f64, f64), p: (f64, f64)) {
        self.ensure_start();
        self.segs.push(Seg::C { x1: c1.0, y1: c1.1, x2: c2.0, y2: c2.1, x: p.0, y: p.1 });
        self.cur = p;
        self.open_subpath = true;
    }

    fn push_all(&mut self, segs: Vec<Seg>) {
        for s in segs {
            match s {
                Seg::L { x, y } => self.line_to((x, y)),
                Seg::C { x1, y1, x2, y2, x, y } => self.cubic_to((x1, y1), (x2, y2), (x, y)),
                Seg::M { x, y } => self.move_to((x, y)),
                Seg::Z => {}
            }
        }
    }
}

fn near(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() <= 1e-7 && (a.1 - b.1).abs() <= 1e-7
}

/// Builds the path of every Geometry section. `w`, `h` are the shape's Width/Height.
pub fn sections_to_paths(sections: &[MergedSection], w: f64, h: f64) -> Vec<SectionPath> {
    sections
        .iter()
        .map(|sec| {
            let mut b = Builder::new();
            let mut is_ellipse_only = true;
            let mut spline: Option<SplineState> = None;
            for row in &sec.rows {
                let t = row.t.as_deref().unwrap_or("");
                if t != "Ellipse" {
                    is_ellipse_only = false;
                }
                if !matches!(t, "SplineKnot")
                    && let Some(sp) = spline.take()
                {
                    b.push_all(sp.finish());
                }
                emit_row(&mut b, row, t, w, h, &mut spline);
            }
            if let Some(sp) = spline.take() {
                b.push_all(sp.finish());
            }
            b.finish_subpath();
            let closed = (b.all_closed && !b.segs.is_empty()) || (is_ellipse_only && !b.segs.is_empty());
            SectionPath {
                segs: b.segs,
                closed,
                no_fill: sec.flag("NoFill"),
                no_line: sec.flag("NoLine"),
                no_show: sec.flag("NoShow"),
            }
        })
        .collect()
}

struct SplineState {
    points: Vec<(f64, f64)>,
}

impl SplineState {
    /// Splines are drawn through their control polygon until M1's spline evaluator lands.
    fn finish(self) -> Vec<Seg> {
        self.points.into_iter().skip(1).map(|(x, y)| Seg::L { x, y }).collect()
    }
}

fn emit_row(b: &mut Builder, row: &MergedRow, t: &str, w: f64, h: f64, spline: &mut Option<SplineState>) {
    let n = |c: &str| row.num(c);
    let xy = |rel: bool| -> Option<(f64, f64)> {
        let (x, y) = (n("X")?, n("Y")?);
        Some(if rel { (x * w, y * h) } else { (x, y) })
    };
    match t {
        "MoveTo" | "RelMoveTo" => {
            if let Some(p) = xy(t == "RelMoveTo") {
                b.move_to(p);
            }
        }
        "LineTo" | "RelLineTo" => {
            if let Some(p) = xy(t == "RelLineTo") {
                b.line_to(p);
            }
        }
        "ArcTo" => {
            if let Some(p) = xy(false) {
                let bow = n("A").unwrap_or(0.0);
                let segs = arc_bow(b.cur, p, bow);
                b.push_all(segs);
            }
        }
        "EllipticalArcTo" | "RelEllipticalArcTo" => {
            let rel = t == "RelEllipticalArcTo";
            if let (Some(p), Some(ax), Some(ay)) = (xy(rel), n("A"), n("B")) {
                let ctrl = if rel { (ax * w, ay * h) } else { (ax, ay) };
                let angle = n("C").unwrap_or(0.0);
                let ratio = n("D").unwrap_or(1.0);
                let segs = elliptical_arc(b.cur, ctrl, p, angle, ratio);
                b.push_all(segs);
            }
        }
        "Ellipse" => {
            if let (Some(c), Some(a), Some(bb), Some(cc), Some(d)) = (xy(false), n("A"), n("B"), n("C"), n("D")) {
                let segs = ellipse(c, (a, bb), (cc, d));
                b.move_to(segs_start(&segs).unwrap_or(c));
                b.push_all(segs.into_iter().skip(1).collect());
                b.finish_subpath();
                b.has_start = false;
            }
        }
        "RelCubBezTo" => {
            if let (Some(p), Some(a), Some(bb), Some(c), Some(d)) = (xy(true), n("A"), n("B"), n("C"), n("D")) {
                b.cubic_to((a * w, bb * h), (c * w, d * h), p);
            }
        }
        "RelQuadBezTo" => {
            if let (Some(p), Some(a), Some(bb)) = (xy(true), n("A"), n("B")) {
                let q = (a * w, bb * h);
                let p0 = b.cur;
                let c1 = (p0.0 + 2.0 / 3.0 * (q.0 - p0.0), p0.1 + 2.0 / 3.0 * (q.1 - p0.1));
                let c2 = (p.0 + 2.0 / 3.0 * (q.0 - p.0), p.1 + 2.0 / 3.0 * (q.1 - p.1));
                b.cubic_to(c1, c2, p);
            }
        }
        "PolylineTo" => {
            if let Some(p) = xy(false) {
                if let Some(args) = row.raw("A").and_then(|a| parse_numeric_call(a, "POLYLINE"))
                    && args.len() >= 2
                {
                    let (xrel, yrel) = (args[0] == 0.0, args[1] == 0.0);
                    let (pairs, _) = args[2..].as_chunks::<2>();
                    for [x, y] in pairs {
                        b.line_to((if xrel { x * w } else { *x }, if yrel { y * h } else { *y }));
                    }
                }
                b.line_to(p);
            }
        }
        "NURBSTo" => {
            if let Some(p) = xy(false) {
                let args = row.raw("E").and_then(|e| parse_numeric_call(e, "NURBS"));
                match args.and_then(|a| nurbs_to(b.cur, p, row, &a, w, h)) {
                    Some(points) => {
                        for q in points {
                            b.line_to(q);
                        }
                    }
                    None => b.line_to(p),
                }
            }
        }
        "SplineStart" => {
            if let Some(p) = xy(false) {
                *spline = Some(SplineState { points: vec![b.cur, p] });
            }
        }
        "SplineKnot" => {
            if let Some(p) = xy(false) {
                match spline {
                    Some(sp) => sp.points.push(p),
                    None => b.line_to(p),
                }
            }
        }
        // InfiniteLine and unknown rows draw nothing.
        _ => {}
    }
}

fn segs_start(segs: &[Seg]) -> Option<(f64, f64)> {
    segs.first().and_then(Seg::end)
}

/// Circle arc from `p0` through `pm` to `p1`, as cubics (no leading MoveTo).
pub fn arc_through(p0: (f64, f64), pm: (f64, f64), p1: (f64, f64)) -> Vec<Seg> {
    let (ax, ay) = p0;
    let (bx, by) = pm;
    let (cx, cy) = p1;
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    if d.abs() < EPS {
        return vec![Seg::L { x: p1.0, y: p1.1 }];
    }
    let a2 = ax * ax + ay * ay;
    let b2 = bx * bx + by * by;
    let c2 = cx * cx + cy * cy;
    let ox = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / d;
    let oy = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / d;
    let r = hypot(ax - ox, ay - oy);
    let t0 = atan2(ay - oy, ax - ox);
    let tm = atan2(by - oy, bx - ox);
    let t1 = atan2(cy - oy, cx - ox);
    // Counter-clockwise sweep from t0 to t1; flip if the middle point is not on it.
    let ccw = |from: f64, to: f64| (to - from).rem_euclid(TAU);
    let sweep_ccw = ccw(t0, t1);
    let sweep = if ccw(t0, tm) <= sweep_ccw { sweep_ccw } else { sweep_ccw - TAU };
    circle_arc((ox, oy), r, t0, sweep, p1)
}

/// Cubic approximation of a circle arc starting at angle `t0` sweeping `sweep` radians.
fn circle_arc(c: (f64, f64), r: f64, t0: f64, sweep: f64, end: (f64, f64)) -> Vec<Seg> {
    let pieces = ((sweep.abs() / (TAU / 4.0)).ceil() as usize).max(1);
    let step = sweep / pieces as f64;
    let k = 4.0 / 3.0 * tan(step / 4.0);
    let mut out = Vec::with_capacity(pieces);
    let mut a = t0;
    for i in 0..pieces {
        let b = a + step;
        let (ca, sa, cb, sb) = (cos(a), sin(a), cos(b), sin(b));
        let p0 = (c.0 + r * ca, c.1 + r * sa);
        let p1 = if i + 1 == pieces { end } else { (c.0 + r * cb, c.1 + r * sb) };
        out.push(Seg::C {
            x1: p0.0 - k * r * sa,
            y1: p0.1 + k * r * ca,
            x2: c.0 + r * cb + k * r * sb,
            y2: c.1 + r * sb - k * r * cb,
            x: p1.0,
            y: p1.1,
        });
        a = b;
    }
    out
}

/// `ArcTo`: circular arc to `p1` whose midpoint is `bow` away from the chord's midpoint.
/// A positive bow bulges to the right of the direction of travel.
pub fn arc_bow(p0: (f64, f64), p1: (f64, f64), bow: f64) -> Vec<Seg> {
    let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
    let len = hypot(dx, dy);
    if bow.abs() < EPS || len < EPS {
        return vec![Seg::L { x: p1.0, y: p1.1 }];
    }
    let mid = ((p0.0 + p1.0) / 2.0, (p0.1 + p1.1) / 2.0);
    let right = (dy / len, -dx / len);
    arc_through(p0, (mid.0 + right.0 * bow, mid.1 + right.1 * bow), p1)
}

/// `EllipticalArcTo`: arc from `p0` through `ctrl` to `p1` on an ellipse whose
/// major axis has angle `angle` and major/minor ratio `ratio`.
pub fn elliptical_arc(p0: (f64, f64), ctrl: (f64, f64), p1: (f64, f64), angle: f64, ratio: f64) -> Vec<Seg> {
    let ratio = if ratio.abs() < EPS { 1.0 } else { ratio };
    let (s, c) = (sin(angle), cos(angle));
    // Into a frame where the ellipse is a circle: rotate by -angle, squash x by 1/ratio.
    let to = |p: (f64, f64)| {
        let (x, y) = (c * p.0 + s * p.1, -s * p.0 + c * p.1);
        (x / ratio, y)
    };
    let from = |p: (f64, f64)| {
        let (x, y) = (p.0 * ratio, p.1);
        (c * x - s * y, s * x + c * y)
    };
    arc_through(to(p0), to(ctrl), to(p1))
        .iter()
        .map(|seg| seg.map(from))
        .collect::<Vec<_>>()
        .into_iter()
        .map(|seg| {
            // Keep the exact end point (rounding through the frame change).
            match seg {
                Seg::C { x1, y1, x2, y2, x, y } if near((x, y), p1) => Seg::C { x1, y1, x2, y2, x: p1.0, y: p1.1 },
                other => other,
            }
        })
        .collect()
}

/// `Ellipse` row: centre `c`, `a` on one axis, `b` on the other. Starts with a MoveTo.
pub fn ellipse(c: (f64, f64), a: (f64, f64), b: (f64, f64)) -> Vec<Seg> {
    let u = (a.0 - c.0, a.1 - c.1);
    let v = (b.0 - c.0, b.1 - c.1);
    let k = 4.0 / 3.0 * tan(TAU / 16.0);
    let p = |t: f64| (c.0 + cos(t) * u.0 + sin(t) * v.0, c.1 + cos(t) * u.1 + sin(t) * v.1);
    let dp = |t: f64| (-sin(t) * u.0 + cos(t) * v.0, -sin(t) * u.1 + cos(t) * v.1);
    let start = p(0.0);
    let mut out = vec![Seg::M { x: start.0, y: start.1 }];
    for q in 0..4 {
        let t0 = f64::from(q) * TAU / 4.0;
        let t1 = f64::from(q + 1) * TAU / 4.0;
        let (p0, d0, p1, d1) = (p(t0), dp(t0), if q == 3 { start } else { p(t1) }, dp(t1));
        out.push(Seg::C {
            x1: p0.0 + k * d0.0,
            y1: p0.1 + k * d0.1,
            x2: p1.0 - k * d1.0,
            y2: p1.1 - k * d1.1,
            x: p1.0,
            y: p1.1,
        });
    }
    out.push(Seg::Z);
    out
}

/// `NURBSTo` with `E = NURBS(knotLast, degree, xType, yType, x1, y1, knot1, weight1, …)`.
/// Control points: the current point (knot `C`, weight `D`), the listed points,
/// then the row's `X, Y` (weight `B`, preceded by knot `A`).
fn nurbs_to(
    p0: (f64, f64),
    p_end: (f64, f64),
    row: &MergedRow,
    args: &[f64],
    w: f64,
    h: f64,
) -> Option<Vec<(f64, f64)>> {
    if args.len() < 4 || !(args.len() - 4).is_multiple_of(4) {
        return None;
    }
    let (knot_last, degree, xrel, yrel) = (args[0], args[1] as usize, args[2] == 0.0, args[3] == 0.0);
    let mut ctrl = vec![p0];
    let mut weights = vec![row.num("D").unwrap_or(1.0)];
    let mut knots = vec![row.num("C").unwrap_or(0.0)];
    let (quads, _) = args[4..].as_chunks::<4>();
    for [x, y, knot, weight] in quads {
        ctrl.push((if xrel { x * w } else { *x }, if yrel { y * h } else { *y }));
        knots.push(*knot);
        weights.push(*weight);
    }
    ctrl.push(p_end);
    weights.push(row.num("B").unwrap_or(1.0));
    knots.push(row.num("A").unwrap_or(knot_last));
    let degree = degree.clamp(1, ctrl.len() - 1);
    while knots.len() < ctrl.len() + degree + 1 {
        knots.push(knot_last);
    }
    knots.truncate(ctrl.len() + degree + 1);
    Some(sample_nurbs(&ctrl, &weights, &knots, degree, 64 * (ctrl.len() - degree)))
}

/// Evaluates a rational B-spline at `samples` evenly spaced parameters (de Boor).
pub fn sample_nurbs(ctrl: &[(f64, f64)], weights: &[f64], knots: &[f64], p: usize, samples: usize) -> Vec<(f64, f64)> {
    let n = ctrl.len();
    let (t0, t1) = (knots[p], knots[n]);
    let mut out = Vec::with_capacity(samples);
    for i in 1..=samples {
        let t = if i == samples { t1 } else { t0 + (t1 - t0) * i as f64 / samples as f64 };
        // Span k with knots[k] <= t < knots[k+1], clamped to the valid range.
        let mut k = p;
        while k + 1 < n && knots[k + 1] <= t {
            k += 1;
        }
        let mut d: Vec<(f64, f64, f64)> = (0..=p)
            .map(|j| {
                let idx = k + j - p;
                let wt = weights[idx];
                (ctrl[idx].0 * wt, ctrl[idx].1 * wt, wt)
            })
            .collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let left = knots[k + j - p];
                let right = knots[k + 1 + j - r];
                let denom = right - left;
                let alpha = if denom.abs() < EPS { 0.0 } else { (t - left) / denom };
                d[j] = (
                    (1.0 - alpha) * d[j - 1].0 + alpha * d[j].0,
                    (1.0 - alpha) * d[j - 1].1 + alpha * d[j].1,
                    (1.0 - alpha) * d[j - 1].2 + alpha * d[j].2,
                );
            }
        }
        let (x, y, wt) = d[p];
        out.push(if wt.abs() < EPS { ctrl[n - 1] } else { (x / wt, y / wt) });
    }
    if let Some(last) = out.last_mut() {
        *last = ctrl[n - 1];
    }
    out
}

/// Rounds the corners between straight segments (the `Rounding` cell).
pub fn round_corners(segs: &[Seg], radius: f64) -> Vec<Seg> {
    if radius <= EPS {
        return segs.to_vec();
    }
    // Split into sub-paths and only round those made purely of lines.
    let mut out = Vec::with_capacity(segs.len() * 2);
    let mut i = 0;
    while i < segs.len() {
        let mut j = i + 1;
        while j < segs.len() && !matches!(segs[j], Seg::M { .. }) {
            j += 1;
        }
        let sub = &segs[i..j];
        let lines_only = sub.iter().skip(1).all(|s| matches!(s, Seg::L { .. } | Seg::Z));
        if lines_only && matches!(sub.first(), Some(Seg::M { .. })) {
            out.extend(round_polyline(sub, radius));
        } else {
            out.extend_from_slice(sub);
        }
        i = j;
    }
    out
}

fn round_polyline(sub: &[Seg], radius: f64) -> Vec<Seg> {
    let closed = matches!(sub.last(), Some(Seg::Z));
    let mut pts: Vec<(f64, f64)> = sub.iter().filter_map(Seg::end).collect();
    if closed && pts.len() > 1 && near(pts[0], pts[pts.len() - 1]) {
        pts.pop();
    }
    let n = pts.len();
    if n < 3 {
        return sub.to_vec();
    }
    // Tangent points either side of each corner: (before, vertex, after).
    type Corner = ((f64, f64), (f64, f64), (f64, f64));
    let corner = |i: usize| -> Option<Corner> {
        let v = pts[i];
        let prev = pts[(i + n - 1) % n];
        let next = pts[(i + 1) % n];
        let (l1, l2) = (hypot(prev.0 - v.0, prev.1 - v.1), hypot(next.0 - v.0, next.1 - v.1));
        if l1 < EPS || l2 < EPS {
            return None;
        }
        let d = radius.min(l1 / 2.0).min(l2 / 2.0);
        let a = (v.0 + (prev.0 - v.0) / l1 * d, v.1 + (prev.1 - v.1) / l1 * d);
        let b = (v.0 + (next.0 - v.0) / l2 * d, v.1 + (next.1 - v.1) / l2 * d);
        Some((a, v, b))
    };
    let quad = |a: (f64, f64), v: (f64, f64), b: (f64, f64)| Seg::C {
        x1: a.0 + 2.0 / 3.0 * (v.0 - a.0),
        y1: a.1 + 2.0 / 3.0 * (v.1 - a.1),
        x2: b.0 + 2.0 / 3.0 * (v.0 - b.0),
        y2: b.1 + 2.0 / 3.0 * (v.1 - b.1),
        x: b.0,
        y: b.1,
    };
    let mut out = Vec::new();
    if closed {
        let first = corner(0);
        let start = first.map(|c| c.2).unwrap_or(pts[0]);
        out.push(Seg::M { x: start.0, y: start.1 });
        for i in 1..=n {
            let idx = i % n;
            match corner(idx) {
                Some((a, v, b)) => {
                    out.push(Seg::L { x: a.0, y: a.1 });
                    out.push(quad(a, v, b));
                }
                None => out.push(Seg::L { x: pts[idx].0, y: pts[idx].1 }),
            }
        }
        out.push(Seg::Z);
    } else {
        out.push(Seg::M { x: pts[0].0, y: pts[0].1 });
        for i in 1..n - 1 {
            let v = pts[i];
            let (prev, next) = (pts[i - 1], pts[i + 1]);
            let (l1, l2) = (hypot(prev.0 - v.0, prev.1 - v.1), hypot(next.0 - v.0, next.1 - v.1));
            if l1 < EPS || l2 < EPS {
                out.push(Seg::L { x: v.0, y: v.1 });
                continue;
            }
            let d = radius.min(l1 / 2.0).min(l2 / 2.0);
            let a = (v.0 + (prev.0 - v.0) / l1 * d, v.1 + (prev.1 - v.1) / l1 * d);
            let b = (v.0 + (next.0 - v.0) / l2 * d, v.1 + (next.1 - v.1) / l2 * d);
            out.push(Seg::L { x: a.0, y: a.1 });
            out.push(quad(a, v, b));
        }
        out.push(Seg::L { x: pts[n - 1].0, y: pts[n - 1].1 });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(t: &str, cells: &[(&str, &str)]) -> MergedRow {
        MergedRow {
            t: Some(t.into()),
            ix: None,
            n: None,
            v: cells.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            f: Default::default(),
        }
    }

    fn section(rows: Vec<MergedRow>) -> MergedSection {
        MergedSection { ix: Some(0), v: Default::default(), rows }
    }

    #[test]
    fn rectangle_is_closed() {
        let s = section(vec![
            row("MoveTo", &[("X", "0"), ("Y", "0")]),
            row("LineTo", &[("X", "2"), ("Y", "0")]),
            row("LineTo", &[("X", "2"), ("Y", "1")]),
            row("LineTo", &[("X", "0"), ("Y", "1")]),
            row("LineTo", &[("X", "0"), ("Y", "0")]),
        ]);
        let p = &sections_to_paths(&[s], 2.0, 1.0)[0];
        assert!(p.closed);
        assert_eq!(p.segs.last(), Some(&Seg::Z));
    }

    #[test]
    fn line_is_open() {
        let s = section(vec![row("MoveTo", &[("X", "0"), ("Y", "0")]), row("LineTo", &[("X", "2"), ("Y", "1")])]);
        assert!(!sections_to_paths(&[s], 2.0, 1.0)[0].closed);
    }

    #[test]
    fn arc_through_three_points_hits_them() {
        let segs = arc_through((0.0, 0.0), (1.0, 1.0), (2.0, 0.0));
        // Upper half circle of radius 1 around (1, 0): two quarter cubics.
        assert_eq!(segs.len(), 2);
        let Seg::C { x, y, .. } = segs[0] else { panic!() };
        assert!((x - 1.0).abs() < 1e-12 && (y - 1.0).abs() < 1e-12);
        assert_eq!(segs[1].end(), Some((2.0, 0.0)));
    }

    #[test]
    fn polyline_and_nurbs() {
        let s = section(vec![
            row("MoveTo", &[("X", "0"), ("Y", "0")]),
            row("PolylineTo", &[("X", "3"), ("Y", "1"), ("A", "POLYLINE(0, 0, 0.5,1, 0.75,0)")]),
        ]);
        let p = &sections_to_paths(&[s], 4.0, 2.0)[0];
        let ends: Vec<_> = p.segs.iter().filter_map(Seg::end).collect();
        assert_eq!(ends, vec![(0.0, 0.0), (2.0, 2.0), (3.0, 0.0), (3.0, 1.0)]);

        // A clamped cubic NURBS with unit weights is a Bézier; check the end points.
        let s = section(vec![
            row("MoveTo", &[("X", "0"), ("Y", "0")]),
            row(
                "NURBSTo",
                &[
                    ("X", "3"),
                    ("Y", "0"),
                    ("A", "0"),
                    ("B", "1"),
                    ("C", "0"),
                    ("D", "1"),
                    ("E", "NURBS(1, 3, 0, 0, 0.33333333333333,1,0,1, 0.66666666666667,-1,0,1)"),
                ],
            ),
        ]);
        let p = &sections_to_paths(&[s], 3.0, 1.0)[0];
        let ends: Vec<_> = p.segs.iter().filter_map(Seg::end).collect();
        assert_eq!(ends.last(), Some(&(3.0, 0.0)));
        // Midpoint of the Bézier (0,0),(1,1),(2,-1),(3,0) is (1.5, 0).
        let mid = ends[ends.len() / 2];
        assert!((mid.0 - 1.5).abs() < 1e-9 && mid.1.abs() < 1e-9, "{mid:?}");
    }
}
