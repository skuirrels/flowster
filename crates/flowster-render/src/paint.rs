//! Fill paints (SPEC §8.3): solid colours, gradients (modern `FillGradient`
//! stops and the legacy two-colour `FillPattern` 25–40) and hatch patterns
//! (`FillPattern` 2–24).
//!
//! Gradient geometry is expressed in a *paint space* mapped to the page by
//! `space`, so radial and rectangular gradients follow the shape's box and
//! rotation exactly like Visio's (they are ellipses on non-square shapes).
//! The origins and blend curve were measured from Visio's own renders
//! (`tools/visio-oracle/make-corpus.ps1`, `gradients.vsdx`).

use flowster_core::transform::Affine;
use flowster_core::value::Rgba;
use libm::{cos, hypot, sin};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Stop {
    pub offset: f64,
    pub color: Rgba,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Paint {
    Solid {
        color: Rgba,
    },
    /// Linear gradient from `(x0, y0)` to `(x1, y1)` in paint space.
    Linear {
        space: Affine,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        stops: Vec<Stop>,
    },
    /// Circular in paint space (elliptical on the page when the box is not square).
    Radial {
        space: Affine,
        cx: f64,
        cy: f64,
        r: f64,
        stops: Vec<Stop>,
    },
    /// Square isolines around `(ox, oy)`: offset = Chebyshev distance / `extent`.
    /// Paint space is the unit square of the shape box.
    Rect {
        space: Affine,
        ox: f64,
        oy: f64,
        extent: f64,
        stops: Vec<Stop>,
    },
    /// An 8 × 8 bitmap tiled in paint space (one bit per unit), `fg` on `bg`.
    Pattern {
        space: Affine,
        bits: [u8; 8],
        fg: Rgba,
        bg: Rgba,
    },
}

impl Paint {
    /// A representative colour (for consumers that only draw solids).
    pub fn representative(&self) -> Rgba {
        match self {
            Paint::Solid { color } => *color,
            Paint::Linear { stops, .. } | Paint::Radial { stops, .. } | Paint::Rect { stops, .. } => {
                stops.get(stops.len() / 2).map_or(Rgba::WHITE, |s| s.color)
            }
            Paint::Pattern { fg, .. } => *fg,
        }
    }
}

/// Visio blends gradient colours with the brighter end dominating: a channel
/// moving up follows `1 − (1 − f)²`, one moving down follows `f²`. Stops are
/// expanded here so renderers can interpolate linearly.
pub fn expand_stops(stops: &[Stop]) -> Vec<Stop> {
    const STEPS: usize = 8;
    let mut out = Vec::with_capacity(stops.len() * STEPS);
    for pair in stops.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        out.push(a);
        for k in 1..STEPS {
            let f = k as f64 / STEPS as f64;
            let ch = |x: u8, y: u8| {
                let (x, y) = (f64::from(x), f64::from(y));
                let w = if y >= x { 1.0 - (1.0 - f) * (1.0 - f) } else { f * f };
                (x + (y - x) * w).round().clamp(0.0, 255.0) as u8
            };
            let alpha = (f64::from(a.color.3) + (f64::from(b.color.3) - f64::from(a.color.3)) * f).round() as u8;
            out.push(Stop {
                offset: a.offset + (b.offset - a.offset) * f,
                color: Rgba(ch(a.color.0, b.color.0), ch(a.color.1, b.color.1), ch(a.color.2, b.color.2), alpha),
            });
        }
    }
    if let Some(last) = stops.last() {
        out.push(*last);
    }
    out
}

/// Unit-square corners/centres used by the radial and rectangular presets (+Y up).
const BR: (f64, f64) = (1.0, 0.0);
const BL: (f64, f64) = (0.0, 0.0);
const TR: (f64, f64) = (1.0, 1.0);
const TL: (f64, f64) = (0.0, 1.0);
const C: (f64, f64) = (0.5, 0.5);

fn farthest_corner(o: (f64, f64)) -> f64 {
    [BL, BR, TL, TR].iter().map(|c| hypot(c.0 - o.0, c.1 - o.1)).fold(0.0, f64::max)
}

fn chebyshev_extent(o: (f64, f64)) -> f64 {
    [BL, BR, TL, TR].iter().map(|c| (c.0 - o.0).abs().max((c.1 - o.1).abs())).fold(0.0, f64::max)
}

/// A modern gradient (`FillGradientEnabled`) for a `w × h` shape whose local
/// frame maps to the page by `world`. `dir` is `FillGradientDir`, `angle` is
/// `FillGradientAngle` (radians, clockwise on screen).
pub fn gradient(world: Affine, w: f64, h: f64, dir: f64, angle: f64, stops: Vec<Stop>) -> Paint {
    let stops = expand_stops(&stops);
    let unit = world.then(Affine::scale(w.max(1e-9), h.max(1e-9)));
    let radial =
        |o: (f64, f64)| Paint::Radial { space: unit, cx: o.0, cy: o.1, r: farthest_corner(o), stops: stops.clone() };
    let rect = |o: (f64, f64)| Paint::Rect {
        space: unit,
        ox: o.0,
        oy: o.1,
        extent: chebyshev_extent(o),
        stops: stops.clone(),
    };
    match dir as i64 {
        1 => radial(BR),
        2 => radial(BL),
        3 => radial(C),
        4 => radial((0.5, 1.0)),
        5 => radial((0.5, 0.0)),
        6 => radial(TR),
        7 => radial(TL),
        8 => rect(BR),
        9 => rect(BL),
        10 | 13 => rect(C),
        11 => rect(TR),
        12 => rect(TL),
        _ => {
            // Linear: along `angle`, clockwise on screen, spanning the shape box.
            let (dx, dy) = (cos(angle), -sin(angle));
            let half = ((w * dx).abs() + (h * dy).abs()) / 2.0;
            let (cx, cy) = (w / 2.0, h / 2.0);
            Paint::Linear {
                space: world,
                x0: cx - dx * half,
                y0: cy - dy * half,
                x1: cx + dx * half,
                y1: cy + dy * half,
                stops,
            }
        }
    }
}

/// The legacy two-colour gradients, `FillPattern` 25–40: `fg` at the origin, `bg` away from it.
pub fn legacy_gradient(world: Affine, w: f64, h: f64, pattern: i64, fg: Rgba, bg: Rgba) -> Option<Paint> {
    let unit = world.then(Affine::scale(w.max(1e-9), h.max(1e-9)));
    let two = expand_stops(&[Stop { offset: 0.0, color: fg }, Stop { offset: 1.0, color: bg }]);
    let mirrored = expand_stops(&[
        Stop { offset: 0.0, color: bg },
        Stop { offset: 0.5, color: fg },
        Stop { offset: 1.0, color: bg },
    ]);
    let linear =
        |x0: f64, y0: f64, x1: f64, y1: f64, stops: Vec<Stop>| Paint::Linear { space: unit, x0, y0, x1, y1, stops };
    let rect =
        |o: (f64, f64)| Paint::Rect { space: unit, ox: o.0, oy: o.1, extent: chebyshev_extent(o), stops: two.clone() };
    let radial = |o: (f64, f64), r: f64| Paint::Radial { space: unit, cx: o.0, cy: o.1, r, stops: two.clone() };
    Some(match pattern {
        25 => linear(0.0, 0.5, 1.0, 0.5, two.clone()),
        26 => linear(0.0, 0.5, 1.0, 0.5, mirrored),
        27 => linear(1.0, 0.5, 0.0, 0.5, two.clone()),
        28 => linear(0.5, 1.0, 0.5, 0.0, two.clone()),
        29 => linear(0.5, 1.0, 0.5, 0.0, mirrored),
        30 => linear(0.5, 0.0, 0.5, 1.0, two.clone()),
        31 => rect(TL),
        32 => rect(TR),
        33 => rect(BL),
        34 => rect(BR),
        35 => rect(C),
        36 => radial(TL, 1.0),
        37 => radial(TR, 1.0),
        38 => radial(BL, 1.0),
        39 => radial(BR, 1.0),
        40 => radial(C, 0.5),
        _ => return None,
    })
}

/// 8 × 8 approximations of the hatch patterns 2–24 (row 0 at the top, bit 7 left).
fn pattern_bits(pattern: i64) -> Option<[u8; 8]> {
    let diag_up = |step: usize| -> [u8; 8] {
        let mut b = [0u8; 8];
        for (r, row) in b.iter_mut().enumerate() {
            for k in (0..8).step_by(step) {
                *row |= 1 << ((r + k) % 8);
            }
        }
        b
    };
    let diag_down = |step: usize| -> [u8; 8] {
        let mut b = [0u8; 8];
        for (r, row) in b.iter_mut().enumerate() {
            for k in (0..8).step_by(step) {
                *row |= 0x80 >> ((r + k) % 8);
            }
        }
        b
    };
    let or = |a: [u8; 8], b: [u8; 8]| -> [u8; 8] { std::array::from_fn(|i| a[i] | b[i]) };
    let rows =
        |mask: u8, every: usize| -> [u8; 8] { std::array::from_fn(|i| if i % every == 0 { 0xFF } else { 0 } | mask) };
    let cols = |c: u8| -> [u8; 8] { [c; 8] };
    Some(match pattern {
        2 => diag_up(8),
        3 => or(rows(0, 8), cols(0x80)),
        4 => or(diag_up(8), diag_down(8)),
        5 => diag_down(8),
        6 => rows(0, 8),
        7 => cols(0x80),
        8 => [0x77, 0xDD, 0x77, 0xDD, 0x77, 0xDD, 0x77, 0xDD],
        9 => [0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55],
        10 => [0x88, 0x22, 0x88, 0x22, 0x88, 0x22, 0x88, 0x22],
        11 => [0x80, 0x00, 0x08, 0x00, 0x80, 0x00, 0x08, 0x00],
        12 => [0x80, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00],
        13 => [0xFF, 0xFF, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00],
        14 => cols(0xCC),
        15 => diag_down(4),
        16 => diag_up(4),
        17 => or(diag_up(4), diag_down(4)),
        18 => or(rows(0, 2), cols(0xAA)),
        19 => rows(0, 4),
        20 => cols(0x88),
        21 => diag_down(4),
        22 => diag_up(4),
        23 => or(rows(0, 4), cols(0x88)),
        24 => or(diag_up(2), diag_down(2)),
        _ => return None,
    })
}

/// Size of one pattern bit on the page, in inches (measured from Visio's renders).
pub const PATTERN_BIT: f64 = 1.0 / 128.0;

pub fn hatch(pattern: i64, fg: Rgba, bg: Rgba) -> Option<Paint> {
    Some(Paint::Pattern { space: Affine::scale(PATTERN_BIT, PATTERN_BIT), bits: pattern_bits(pattern)?, fg, bg })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_curve_matches_visio_samples() {
        // White → blue, measured on Visio renders: the red channel at the middle is ~186–196, not 128.
        let s = expand_stops(&[
            Stop { offset: 0.0, color: Rgba(255, 255, 255, 255) },
            Stop { offset: 1.0, color: Rgba(0, 0, 255, 255) },
        ]);
        let mid = s.iter().find(|x| (x.offset - 0.5).abs() < 1e-9).unwrap();
        assert!((185..=197).contains(&mid.color.0), "{:?}", mid.color);
        assert_eq!(mid.color.2, 255);
        assert_eq!(s.first().unwrap().color, Rgba(255, 255, 255, 255));
        assert_eq!(s.last().unwrap().color, Rgba(0, 0, 255, 255));
    }

    #[test]
    fn linear_angles() {
        let stops = vec![Stop { offset: 0.0, color: Rgba::WHITE }, Stop { offset: 1.0, color: Rgba::BLACK }];
        // 90° runs top to bottom (+Y up, so y0 > y1); 0° left to right.
        let Paint::Linear { x0, y0, x1, y1, .. } =
            gradient(Affine::IDENTITY, 2.0, 1.0, 0.0, core::f64::consts::FRAC_PI_2, stops.clone())
        else {
            panic!()
        };
        assert!((x0 - 1.0).abs() < 1e-12 && (y0 - 1.0).abs() < 1e-12 && (x1 - 1.0).abs() < 1e-12 && y1.abs() < 1e-12);
        let Paint::Linear { x0, x1, .. } = gradient(Affine::IDENTITY, 2.0, 1.0, 0.0, 0.0, stops) else { panic!() };
        assert!(x0.abs() < 1e-12 && (x1 - 2.0).abs() < 1e-12);
    }

    #[test]
    fn presets() {
        let stops = vec![Stop { offset: 0.0, color: Rgba::WHITE }, Stop { offset: 1.0, color: Rgba::BLACK }];
        let Paint::Radial { cx, cy, r, .. } = gradient(Affine::IDENTITY, 1.0, 1.0, 1.0, 0.0, stops.clone()) else {
            panic!()
        };
        assert_eq!((cx, cy), (1.0, 0.0));
        assert!((r - 2f64.sqrt()).abs() < 1e-12);
        let Paint::Rect { extent, .. } = gradient(Affine::IDENTITY, 1.0, 1.0, 10.0, 0.0, stops) else { panic!() };
        assert_eq!(extent, 0.5);
        assert!(hatch(2, Rgba::BLACK, Rgba::WHITE).is_some());
        assert!(hatch(25, Rgba::BLACK, Rgba::WHITE).is_none());
        assert!(legacy_gradient(Affine::IDENTITY, 1.0, 1.0, 36, Rgba::BLACK, Rgba::WHITE).is_some());
    }
}
