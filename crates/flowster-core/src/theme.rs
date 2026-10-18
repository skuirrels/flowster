//! Document themes (SPEC §11): the DrawingML theme part plus Visio's
//! extensions, enough to evaluate the common `THEMEVAL()` cells: fill, line and
//! text colour, line weight, pattern, rounding, arrows and the text font.
//!
//! A shape picks a theme colour with `QuickStyle*Color` and a style row with
//! `QuickStyle*Matrix`; the row's colour is usually the placeholder `phClr`,
//! replaced by the picked colour, then adjusted by DrawingML modifiers (shade, tint, …).

use std::collections::BTreeMap;

use libm::pow;

use crate::value::Rgba;

#[derive(Clone, Debug, PartialEq)]
pub enum ColorBase {
    Rgb(Rgba),
    /// Scheme colour name: `dk1`, `lt1`, `accent1`…, or `phClr` (the placeholder).
    Scheme(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColorSpec {
    pub base: ColorBase,
    /// DrawingML modifiers in document order: (`shade`, 0.5), (`lumMod`, 0.75), …
    pub mods: Vec<(String, f64)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FillStyle {
    /// Solid colour, or a representative stop of a gradient.
    pub color: Option<ColorSpec>,
    pub pattern: Option<f64>,
    /// Gradient stops `(position 0–1, colour)` when the style is a gradient fill.
    pub gradient: Vec<(f64, ColorSpec)>,
    /// Gradient geometry, in Visio's `FillGradientDir` terms (0 linear, 3 radial
    /// from centre, 10 rectangular from centre).
    pub gradient_dir: Option<f64>,
    /// Linear gradient angle in radians (DrawingML `lin ang`).
    pub gradient_angle: Option<f64>,
}

/// A theme effect's outer shadow (DrawingML `outerShdw`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShadowStyle {
    pub blur_emu: f64,
    pub dist_emu: f64,
    /// Direction in radians, clockwise from +X on screen.
    pub dir: f64,
    pub color: Option<ColorSpec>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineStyle {
    /// Width in EMU (914 400 per inch).
    pub width_emu: Option<f64>,
    pub color: Option<ColorSpec>,
    pub pattern: Option<f64>,
    pub rounding_emu: Option<f64>,
    pub begin_arrow: Option<f64>,
    pub begin_arrow_size: Option<f64>,
    pub end_arrow: Option<f64>,
    pub end_arrow_size: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontStyle {
    pub color: Option<ColorSpec>,
    pub style: Option<f64>,
}

/// One style set: what shapes use, or what connectors use.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleSet {
    pub fills: Vec<FillStyle>,
    pub lines: Vec<LineStyle>,
    pub fonts: Vec<FontStyle>,
    /// Effect styles; `None` for effects without an outer shadow.
    pub effects: Vec<Option<ShadowStyle>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Theme {
    pub name: String,
    /// `dk1, lt1, dk2, lt2, accent1…6, hlink, folHlink, bkgnd`.
    pub scheme: BTreeMap<String, Rgba>,
    pub major_font: Option<String>,
    pub minor_font: Option<String>,
    pub shapes: StyleSet,
    pub connectors: StyleSet,
    /// Variation colour schemes: `varColor1…7` each.
    pub variation_colors: Vec<Vec<ColorSpec>>,
    /// Variation style schemes, each with the `varStyle`s that matrix values
    /// 100–103 select: `[fillIdx, lineIdx, effectIdx, fontIdx]` (1-based).
    pub variation_styles: Vec<Vec<[u32; 4]>>,
}

pub const EMU_PER_INCH: f64 = 914_400.0;

/// What a `QuickStyle*Matrix` selects for one category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Fill = 0,
    Line = 1,
    Effect = 2,
    Font = 3,
}

impl Theme {
    pub fn scheme_color(&self, name: &str) -> Option<Rgba> {
        self.scheme.get(name).copied()
    }

    /// Resolves a `QuickStyle*Color` value: 0 dark, 1 light, 2–7 accents,
    /// 100+ variation colours (default variation: the accents).
    pub fn quick_style_color(&self, value: f64, variation: usize) -> Option<Rgba> {
        let v = value as i64;
        let name = match v {
            0 => "dk1",
            1 => "lt1",
            2..=7 => return self.scheme_color(&format!("accent{}", v - 1)),
            100..=106 => {
                let k = (v - 100) as usize;
                if let Some(spec) = self.variation_colors.get(variation).and_then(|c| c.get(k)) {
                    return Some(self.resolve(spec, None));
                }
                return match k {
                    0..=5 => self.scheme_color(&format!("accent{}", k + 1)),
                    _ => self.scheme_color("dk2"),
                };
            }
            _ => "accent1",
        };
        self.scheme_color(name)
    }

    /// Resolves a `QuickStyle*Matrix` value to a 0-based row of the style list.
    pub fn matrix_row(&self, value: f64, slot: Slot, variation: usize) -> usize {
        let v = value as i64;
        match v {
            1..=6 => (v - 1) as usize,
            100..=103 => self
                .variation_styles
                .get(variation)
                .and_then(|scheme| scheme.get((v - 100) as usize))
                .map(|s| (s[slot as usize].max(1) - 1) as usize)
                .unwrap_or(0),
            _ => 0,
        }
    }

    /// The final colour of a spec; `ph` replaces the `phClr` placeholder.
    pub fn resolve(&self, spec: &ColorSpec, ph: Option<Rgba>) -> Rgba {
        let base = match &spec.base {
            ColorBase::Rgb(c) => *c,
            ColorBase::Scheme(n) if n == "phClr" => ph.unwrap_or(Rgba::BLACK),
            ColorBase::Scheme(n) => self
                .scheme_color(match n.as_str() {
                    "tx1" => "dk1",
                    "bg1" => "lt1",
                    "tx2" => "dk2",
                    "bg2" => "lt2",
                    other => other,
                })
                .unwrap_or(Rgba::BLACK),
        };
        apply_mods(base, &spec.mods)
    }
}

fn to_linear(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { pow((c + 0.055) / 1.055, 2.4) }
}

fn to_srgb(c: f64) -> f64 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * pow(c, 1.0 / 2.4) - 0.055 }
}

fn rgb_to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-12 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    (h, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    if s.abs() < 1e-12 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let hue = |t: f64| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (hue(h + 1.0 / 3.0), hue(h), hue(h - 1.0 / 3.0))
}

/// DrawingML colour transforms (ECMA-376 Part 1 §20.1.2.3): shade and tint
/// work in linear RGB, luminance and saturation modifiers in HSL.
pub fn apply_mods(c: Rgba, mods: &[(String, f64)]) -> Rgba {
    let mut r = f64::from(c.0) / 255.0;
    let mut g = f64::from(c.1) / 255.0;
    let mut b = f64::from(c.2) / 255.0;
    let mut a = f64::from(c.3) / 255.0;
    for (name, v) in mods {
        match name.as_str() {
            "shade" => {
                r = to_srgb(to_linear(r) * v);
                g = to_srgb(to_linear(g) * v);
                b = to_srgb(to_linear(b) * v);
            }
            "tint" => {
                r = to_srgb(to_linear(r) * v + (1.0 - v));
                g = to_srgb(to_linear(g) * v + (1.0 - v));
                b = to_srgb(to_linear(b) * v + (1.0 - v));
            }
            "lumMod" | "lumOff" | "satMod" | "satOff" | "hueOff" => {
                let (mut h, mut s, mut l) = rgb_to_hsl(r, g, b);
                match name.as_str() {
                    "lumMod" => l *= v,
                    "lumOff" => l += v,
                    "satMod" => s *= v,
                    "satOff" => s += v,
                    _ => h += v / 360.0,
                }
                (r, g, b) = hsl_to_rgb(h, s.clamp(0.0, 1.0), l.clamp(0.0, 1.0));
            }
            "alpha" => a = *v,
            "alphaMod" => a *= v,
            "alphaOff" => a += v,
            _ => {}
        }
    }
    let q = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgba(q(r), q(g), q(b), q(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers() {
        let green = Rgba(0xBD, 0xD3, 0x65, 255);
        // Shade darkens, tint lightens, lumMod scales lightness.
        let s = apply_mods(green, &[("shade".into(), 0.5)]);
        assert!(s.0 < green.0 && s.1 < green.1 && s.2 < green.2);
        let t = apply_mods(green, &[("tint".into(), 0.5)]);
        assert!(t.0 > green.0 && t.1 > green.1 && t.2 > green.2);
        assert_eq!(apply_mods(Rgba::WHITE, &[("lumMod".into(), 0.5)]), Rgba(128, 128, 128, 255));
        assert_eq!(apply_mods(green, &[]), green);
    }
}
