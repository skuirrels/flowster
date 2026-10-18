//! Derived-state determinism probe (SPEC §20.3, risk R8).
//!
//! Every peer recomputes derived state from replicated inputs, so the native
//! and WASM builds must agree bit for bit on every OS. This module computes a
//! small but representative slice of derived state (transforms, ShapeSheet-style
//! trigonometry, an ellipse path, a flattened Bézier connector with arc length)
//! and hashes the raw f64 bits. CI compares the hash across native and WASM on
//! all three OSes, and against [`GOLDEN_HASH`].
//!
//! Rules this code follows, and that all derived-state code must follow:
//! * transcendental functions come from `libm`, never `f64::sin` & co.
//!   (`clippy.toml` bans the std methods);
//! * no `mul_add` and no `powi` (the result depends on the platform);
//! * iteration order is explicit (vectors, sorted keys), never hash-map order.

use libm::{atan2, cos, exp, hypot, log, pow, sin, sqrt};
use serde::Serialize;

use crate::rng::SplitMix64;

/// Hash of [`derived_state`] with [`DEFAULT_SHAPES`] shapes and [`DEFAULT_SEED`].
/// If this changes, derived state changed for every existing document: treat it
/// like a file-format change.
pub const GOLDEN_HASH: &str = "7e3e5284ff3c4418";
pub const DEFAULT_SHAPES: usize = 2_000;
pub const DEFAULT_SEED: u64 = 0x5EED_F10E;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DerivedStateReport {
    /// FNV-1a 64 over every derived f64's bit pattern, hex.
    pub hash: String,
    /// Number of f64 values hashed.
    pub values: u64,
    pub shapes: usize,
    pub seed: u64,
}

/// Inputs of one synthetic shape (what the CRDT would replicate).
#[derive(Clone, Copy, Debug)]
struct Inputs {
    pin_x: f64,
    pin_y: f64,
    width: f64,
    height: f64,
    angle: f64,
    flip_x: bool,
}

struct Hasher {
    state: u64,
    values: u64,
}

impl Hasher {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Hasher { state: Self::OFFSET, values: 0 }
    }

    fn f(&mut self, v: f64) {
        for b in v.to_bits().to_le_bytes() {
            self.state ^= u64::from(b);
            self.state = self.state.wrapping_mul(Self::PRIME);
        }
        self.values += 1;
    }

    fn pt(&mut self, (x, y): (f64, f64)) {
        self.f(x);
        self.f(y);
    }
}

/// `parent = Pin + R(Angle) · F(FlipX) · (local − LocPin)` (§6.4).
fn local_to_parent(s: &Inputs, (lx, ly): (f64, f64)) -> (f64, f64) {
    let (loc_x, loc_y) = (s.width * 0.5, s.height * 0.5);
    let mut dx = lx - loc_x;
    let dy = ly - loc_y;
    if s.flip_x {
        dx = -dx;
    }
    let (sa, ca) = (sin(s.angle), cos(s.angle));
    (s.pin_x + ca * dx - sa * dy, s.pin_y + sa * dx + ca * dy)
}

/// Flattened cubic Bézier, de Casteljau, fixed step count.
fn cubic(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), steps: u32) -> Vec<(f64, f64)> {
    let lerp = |a: (f64, f64), b: (f64, f64), t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    (0..=steps)
        .map(|i| {
            let t = f64::from(i) / f64::from(steps);
            let (a, b, c) = (lerp(p0, p1, t), lerp(p1, p2, t), lerp(p2, p3, t));
            let (d, e) = (lerp(a, b, t), lerp(b, c, t));
            lerp(d, e, t)
        })
        .collect()
}

fn polyline_length(pts: &[(f64, f64)]) -> f64 {
    pts.windows(2).map(|w| hypot(w[1].0 - w[0].0, w[1].1 - w[0].1)).sum()
}

fn inputs(n: usize, seed: u64) -> Vec<Inputs> {
    let mut rng = SplitMix64::new(seed);
    (0..n)
        .map(|_| Inputs {
            pin_x: rng.unit() * 40.0,
            pin_y: rng.unit() * 30.0,
            width: 0.25 + rng.unit() * 3.0,
            height: 0.25 + rng.unit() * 2.0,
            // Includes angles well outside [-π, π] to exercise argument reduction.
            angle: (rng.unit() - 0.5) * 40.0,
            flip_x: rng.next_u64() & 1 == 1,
        })
        .collect()
}

/// Computes the derived-state slice and its hash.
pub fn derived_state(n_shapes: usize, seed: u64) -> DerivedStateReport {
    let shapes = inputs(n_shapes, seed);
    let mut h = Hasher::new();

    for (i, s) in shapes.iter().enumerate() {
        // Transform: the four corners of the shape's box in page coordinates.
        for c in [(0.0, 0.0), (s.width, 0.0), (s.width, s.height), (0.0, s.height)] {
            h.pt(local_to_parent(s, c));
        }

        // ShapeSheet-style formulas.
        let next = &shapes[(i + 1) % shapes.len()];
        let (dx, dy) = (next.pin_x - s.pin_x, next.pin_y - s.pin_y);
        h.f(atan2(dy, dx)); // ANG-like
        h.f(hypot(dx, dy)); // DIST-like
        h.f(sqrt(s.width * s.width + s.height * s.height));
        h.f(pow(s.width / (s.width + s.height), 2.2)); // gamma-style colour math
        h.f(log(1.0 + s.height) * exp(-s.width / 4.0));
        h.f(atan2(sin(s.angle), cos(s.angle))); // angle normalisation

        // Path: an ellipse inscribed in the box, 32 segments, in page coordinates.
        let ellipse: Vec<(f64, f64)> = (0..=32)
            .map(|k| {
                let t = f64::from(k) * (2.0 * core::f64::consts::PI / 32.0);
                let local = (s.width * 0.5 * (1.0 + cos(t)), s.height * 0.5 * (1.0 + sin(t)));
                local_to_parent(s, local)
            })
            .collect();
        for p in &ellipse {
            h.pt(*p);
        }
        h.f(polyline_length(&ellipse));

        // Connector to the next shape: a cubic leaving along each shape's angle.
        let start = local_to_parent(s, (s.width, s.height * 0.5));
        let end = local_to_parent(next, (0.0, next.height * 0.5));
        let reach = hypot(end.0 - start.0, end.1 - start.1) / 3.0;
        let c1 = (start.0 + cos(s.angle) * reach, start.1 + sin(s.angle) * reach);
        let c2 = (end.0 - cos(next.angle) * reach, end.1 - sin(next.angle) * reach);
        let route = cubic(start, c1, c2, end, 16);
        for p in &route {
            h.pt(*p);
        }
        h.f(polyline_length(&route));
    }

    DerivedStateReport { hash: format!("{:016x}", h.state), values: h.values, shapes: n_shapes, seed }
}

/// [`derived_state`] with the default parameters used by CI.
pub fn default_report() -> DerivedStateReport {
    derived_state(DEFAULT_SHAPES, DEFAULT_SEED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeatable_in_process() {
        assert_eq!(derived_state(100, 1), derived_state(100, 1));
        assert_ne!(derived_state(100, 1).hash, derived_state(100, 2).hash);
    }

    /// Fails if this platform computes different derived state from the
    /// reference. CI runs it on Windows, macOS (arm64) and Linux.
    #[test]
    fn matches_golden_hash() {
        let r = default_report();
        assert_eq!(
            r.hash, GOLDEN_HASH,
            "derived state differs from the reference on this platform ({} values)",
            r.values
        );
    }
}
