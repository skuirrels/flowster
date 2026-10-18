//! 2-D affine transforms (SPEC §6.4). Same component order as canvas/SVG:
//! `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.

use libm::{cos, sin};

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct Affine(pub [f64; 6]);

impl Affine {
    pub const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub fn translate(x: f64, y: f64) -> Affine {
        Affine([1.0, 0.0, 0.0, 1.0, x, y])
    }

    pub fn scale(sx: f64, sy: f64) -> Affine {
        Affine([sx, 0.0, 0.0, sy, 0.0, 0.0])
    }

    /// Counter-clockwise rotation (Visio angles, +Y up).
    pub fn rotate(rad: f64) -> Affine {
        let (s, c) = (sin(rad), cos(rad));
        Affine([c, s, -s, c, 0.0, 0.0])
    }

    /// `self ∘ other`: applies `other` first, then `self`.
    pub fn then(self, other: Affine) -> Affine {
        let [a1, b1, c1, d1, e1, f1] = self.0;
        let [a2, b2, c2, d2, e2, f2] = other.0;
        Affine([
            a1 * a2 + c1 * b2,
            b1 * a2 + d1 * b2,
            a1 * c2 + c1 * d2,
            b1 * c2 + d1 * d2,
            a1 * e2 + c1 * f2 + e1,
            b1 * e2 + d1 * f2 + f1,
        ])
    }

    pub fn apply(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }

    /// Length scale along a direction-agnostic average (for line widths).
    /// The inverse transform, if the matrix is invertible.
    pub fn invert(&self) -> Option<Affine> {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if det.abs() < 1e-15 {
            return None;
        }
        let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
        Some(Affine([ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)]))
    }

    pub fn mean_scale(&self) -> f64 {
        let [a, b, c, d, _, _] = self.0;
        let det = (a * d - b * c).abs();
        det.sqrt()
    }

    /// Local → parent for a shape (§6.4):
    /// `parent = Pin + R(Angle) · F(FlipX, FlipY) · (local − LocPin)`.
    pub fn shape_local_to_parent(
        pin: (f64, f64),
        loc_pin: (f64, f64),
        angle: f64,
        flip_x: bool,
        flip_y: bool,
    ) -> Affine {
        let flip = Affine::scale(if flip_x { -1.0 } else { 1.0 }, if flip_y { -1.0 } else { 1.0 });
        Affine::translate(pin.0, pin.1)
            .then(Affine::rotate(angle))
            .then(flip)
            .then(Affine::translate(-loc_pin.0, -loc_pin.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12
    }

    #[test]
    fn shape_transform() {
        let t = Affine::shape_local_to_parent((5.0, 5.0), (1.0, 0.5), core::f64::consts::FRAC_PI_2, false, false);
        // The local pin maps to the pin; local +X maps to parent +Y after a quarter turn.
        assert!(close(t.apply((1.0, 0.5)), (5.0, 5.0)));
        assert!(close(t.apply((2.0, 0.5)), (5.0, 6.0)));
        let f = Affine::shape_local_to_parent((5.0, 5.0), (1.0, 0.5), 0.0, true, false);
        assert!(close(f.apply((2.0, 0.5)), (4.0, 5.0)));
    }
}
