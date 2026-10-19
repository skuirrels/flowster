//! Arrowhead geometry for `BeginArrow` / `EndArrow` 1–45, measured from Visio's
//! rendering (oracle exports at every size and several line weights).
//!
//! Heads are drawn in a local frame with the line's end at the origin, +X
//! pointing out of the line and lengths in head units `u`. Visio scales a head
//! with both the size cell and the line weight:
//! `u = F[size] × 0.97 × (weight + 2.7 pt)`, where F is 0.71, 0.77, 1, 1.18,
//! 1.37, 2.78 and 5.28 for sizes 0 (very small) to 6 (colossal). Circles, ticks
//! and crow's feet share a second unit `V = 0.76 u`.

use flowster_core::geom::Seg;

/// The round-figure unit used by circles, squares, ticks and crow's feet.
const V: f64 = 0.76;

/// Head units in inches for a size cell (0–6) and line weight (inches).
pub fn unit(size: f64, weight: f64) -> f64 {
    const F: [f64; 7] = [0.71, 0.77, 1.0, 1.18, 1.37, 2.78, 5.28];
    let f = F[size.clamp(0.0, 6.0) as usize];
    f * 0.97 * (weight * 72.0 + 2.7) / 72.0
}

/// How a part of a head is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Stroked with the line (open arrows, ticks).
    Open,
    /// Filled with the line colour.
    Filled,
    /// Outlined with the line, nothing inside.
    Hollow,
}

/// One arrowhead: its parts and how far back from the end the line stops.
pub struct Head {
    pub parts: Vec<(Style, Vec<Seg>)>,
    /// Distance in `u` from the end at which the line itself stops.
    pub trim: f64,
}

fn poly(points: &[(f64, f64)], closed: bool) -> Vec<Seg> {
    let mut d: Vec<Seg> =
        points.iter().enumerate().map(|(i, &(x, y))| if i == 0 { Seg::M { x, y } } else { Seg::L { x, y } }).collect();
    if closed {
        d.push(Seg::Z);
    }
    d
}

/// A quadratic Bézier from the current point, as a cubic.
fn quad(from: (f64, f64), c: (f64, f64), to: (f64, f64)) -> Seg {
    Seg::C {
        x1: from.0 + 2.0 / 3.0 * (c.0 - from.0),
        y1: from.1 + 2.0 / 3.0 * (c.1 - from.1),
        x2: to.0 + 2.0 / 3.0 * (c.0 - to.0),
        y2: to.1 + 2.0 / 3.0 * (c.1 - to.1),
        x: to.0,
        y: to.1,
    }
}

/// Triangle with its tip at `apex`.
fn tri(apex: f64, len: f64, half: f64) -> Vec<Seg> {
    poly(&[(apex, 0.0), (apex - len, half), (apex - len, -half)], true)
}

/// Open "V" with its point at `apex`.
fn vee(apex: f64, len: f64, half: f64) -> Vec<Seg> {
    poly(&[(apex - len, half), (apex, 0.0), (apex - len, -half)], false)
}

fn tick(x: f64) -> Vec<Seg> {
    poly(&[(x, V), (x, -V)], false)
}

fn segment(x0: f64, x1: f64) -> Vec<Seg> {
    poly(&[(x0, 0.0), (x1, 0.0)], false)
}

fn crow() -> Vec<Seg> {
    poly(&[(0.0, V), (-1.8 * V, 0.0), (0.0, -V)], false)
}

fn circle(cx: f64, r: f64) -> Vec<Seg> {
    const K: f64 = 0.552_284_749_830_793_4;
    let k = K * r;
    vec![
        Seg::M { x: cx + r, y: 0.0 },
        Seg::C { x1: cx + r, y1: k, x2: cx + k, y2: r, x: cx, y: r },
        Seg::C { x1: cx - k, y1: r, x2: cx - r, y2: k, x: cx - r, y: 0.0 },
        Seg::C { x1: cx - r, y1: -k, x2: cx - k, y2: -r, x: cx, y: -r },
        Seg::C { x1: cx + k, y1: -r, x2: cx + r, y2: -k, x: cx + r, y: 0.0 },
        Seg::Z,
    ]
}

fn square(cx: f64, half: f64) -> Vec<Seg> {
    poly(&[(cx + half, half), (cx - half, half), (cx - half, -half), (cx + half, -half)], true)
}

fn diamond(front: f64, len: f64, half: f64) -> Vec<Seg> {
    poly(&[(front, 0.0), (front - len / 2.0, half), (front - len, 0.0), (front - len / 2.0, -half)], true)
}

/// 2u triangle whose back is an arc: `bulge` < 0 dents it towards the tip.
fn arc_back(bulge: f64) -> Vec<Seg> {
    // A quadratic through the back corners whose midpoint is 2u + bulge back.
    let c = (-2.0 - 2.0 * bulge, 0.0);
    vec![Seg::M { x: 0.0, y: 0.0 }, Seg::L { x: -2.0, y: 1.0 }, quad((-2.0, 1.0), c, (-2.0, -1.0)), Seg::Z]
}

/// 2u head with flared sides (y = x² / 4) and a dented back.
fn flared(closed: bool) -> Vec<Seg> {
    let side_up = quad((0.0, 0.0), (-1.0, 0.0), (-2.0, 1.0));
    let side_down = quad((-2.0, -1.0), (-1.0, 0.0), (0.0, 0.0));
    if closed {
        vec![Seg::M { x: 0.0, y: 0.0 }, side_up, quad((-2.0, 1.0), (-1.5, 0.0), (-2.0, -1.0)), side_down, Seg::Z]
    } else {
        vec![
            Seg::M { x: -2.0, y: 1.0 },
            quad((-2.0, 1.0), (-1.0, 0.0), (0.0, 0.0)),
            quad((0.0, 0.0), (-1.0, 0.0), (-2.0, -1.0)),
        ]
    }
}

/// The head for an arrow type, or `None` for 0 and unknown types.
pub fn head(kind: i64) -> Option<Head> {
    use Style::{Filled, Hollow, Open};
    let h = |parts: Vec<(Style, Vec<Seg>)>, trim: f64| Some(Head { parts, trim });
    match kind {
        1 => h(vec![(Open, vee(0.0, 1.0, 1.0))], 0.0),
        2 => h(vec![(Filled, tri(0.0, 1.0, 1.0))], 1.0),
        3 => h(vec![(Open, vee(0.0, 2.0, 1.0))], 0.0),
        4 => h(vec![(Filled, tri(0.0, 2.0, 1.0))], 2.0),
        5 => h(vec![(Filled, arc_back(-0.25))], 1.75),
        6 => h(vec![(Filled, arc_back(0.4))], 2.0),
        7 => h(vec![(Open, flared(false))], 0.0),
        8 => h(vec![(Filled, flared(true))], 1.75),
        9 => h(vec![(Open, poly(&[(-1.0, -1.0), (1.0, 1.0)], false))], 0.0),
        10 => h(vec![(Filled, circle(0.0, V))], 0.0),
        11 => h(vec![(Filled, square(0.0, V))], 0.0),
        12 => h(vec![(Open, vee(0.0, 3.0, 1.0))], 0.0),
        13 => h(vec![(Filled, tri(0.0, 3.0, 1.0))], 3.0),
        14 => h(vec![(Hollow, tri(0.0, 3.0, 1.0))], 3.0),
        15 => h(vec![(Hollow, tri(0.0, 1.0, 1.0))], 1.0),
        16 => h(vec![(Hollow, tri(0.0, 2.0, 1.0))], 2.0),
        17 => h(vec![(Hollow, arc_back(-0.25))], 1.75),
        18 => h(vec![(Hollow, arc_back(0.4))], 2.4),
        19 => h(vec![(Hollow, flared(true))], 1.75),
        20 => h(vec![(Hollow, circle(0.0, V))], V),
        21 => h(vec![(Hollow, square(0.0, V))], V),
        22 => h(vec![(Hollow, diamond(0.0, 4.5, 1.125))], 4.5),
        23 => h(vec![(Open, poly(&[(-1.0, -1.0), (-3.0, 1.0)], false))], 0.0),
        24 => h(vec![(Open, tick(-2.0 * V))], 0.0),
        25 => h(vec![(Open, tick(-2.0 * V)), (Open, tick(-3.0 * V))], 0.0),
        26 => h(vec![(Open, tick(-2.0 * V)), (Open, tick(-3.0 * V)), (Open, tick(-4.0 * V))], 0.0),
        27 => h(vec![(Open, crow())], 0.0),
        28 => h(vec![(Open, crow()), (Open, tick(-3.0 * V))], 0.0),
        29 => h(vec![(Open, crow()), (Open, segment(0.0, -2.0 * V)), (Hollow, circle(-3.0 * V, V))], 4.0 * V),
        30 => h(vec![(Open, tick(-V)), (Open, segment(0.0, -2.0 * V)), (Hollow, circle(-3.0 * V, V))], 4.0 * V),
        31 => h(vec![(Hollow, circle(-V, V)), (Open, tick(-3.0 * V))], 2.0 * V),
        32 => h(vec![(Hollow, circle(-V, V)), (Open, tick(-3.0 * V)), (Open, tick(-4.0 * V))], 2.0 * V),
        33 => h(
            vec![(Hollow, circle(-V, V)), (Open, tick(-3.0 * V)), (Open, tick(-4.0 * V)), (Open, tick(-5.0 * V))],
            2.0 * V,
        ),
        34 => h(vec![(Hollow, circle(-V, V)), (Hollow, diamond(-2.0 * V, 4.5, 1.125))], 0.0),
        35 => h(vec![(Filled, circle(-V, V)), (Open, tick(-3.0 * V))], 0.0),
        36 => h(vec![(Filled, circle(-V, V)), (Open, tick(-3.0 * V)), (Open, tick(-4.0 * V))], 0.0),
        37 => h(
            vec![(Filled, circle(-V, V)), (Open, tick(-3.0 * V)), (Open, tick(-4.0 * V)), (Open, tick(-5.0 * V))],
            0.0,
        ),
        38 => h(vec![(Filled, circle(-V, V)), (Hollow, diamond(-2.0 * V, 4.5, 1.125))], 0.0),
        39 => h(vec![(Filled, tri(0.0, 2.0, 1.0)), (Filled, tri(-2.0, 2.0, 1.0))], 4.0),
        40 => h(vec![(Hollow, tri(0.0, 2.0, 1.0)), (Hollow, tri(-2.0, 2.0, 1.0))], 4.0),
        41 => h(vec![(Hollow, circle(-V, V))], 2.0 * V),
        42 => h(vec![(Filled, circle(-V, V))], 0.0),
        43 => h(vec![(Open, vee(0.0, 2.0, 1.0)), (Open, vee(-2.0, 2.0, 1.0))], 0.0),
        44 => h(vec![(Open, vee(0.0, 2.0, 1.0)), (Open, tick(-2.0 - 1.5 * V))], 0.0),
        45 => h(vec![(Open, vee(0.0, 2.0, 1.0)), (Open, vee(-2.0, 2.0, 1.0)), (Open, tick(-4.0 - 1.5 * V))], 0.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_has_a_head_and_scaling_follows_size_and_weight() {
        for k in 1..=45 {
            let h = head(k).unwrap_or_else(|| panic!("type {k}"));
            assert!(!h.parts.is_empty() && h.trim >= 0.0);
        }
        assert!(head(0).is_none() && head(46).is_none());
        // Medium at 1 pt: 3.6 pt; doubles roughly with jumbo, grows with weight.
        assert!((unit(2.0, 1.0 / 72.0) * 72.0 - 3.589).abs() < 1e-3);
        assert!(unit(5.0, 1.0 / 72.0) > 2.7 * unit(2.0, 1.0 / 72.0));
        assert!(unit(2.0, 6.0 / 72.0) > 2.3 * unit(2.0, 1.0 / 72.0));
    }
}
