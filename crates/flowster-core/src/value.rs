//! Interpreting cached cell values (`V`) and a few formula-only cells.

use std::collections::BTreeMap;

/// RGBA, 8 bits per channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const BLACK: Rgba = Rgba(0, 0, 0, 255);
    pub const WHITE: Rgba = Rgba(255, 255, 255, 255);

    pub fn with_transparency(self, trans: f64) -> Rgba {
        let a = (f64::from(self.3) * (1.0 - trans.clamp(0.0, 1.0))).round() as u8;
        Rgba(self.0, self.1, self.2, a)
    }

    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

/// The 24 built-in colour-table entries every Visio document starts with
/// (public file format documentation: indices 0–23 of the colour table).
const BASE_PALETTE: [u32; 24] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000, 0x008000, 0x000080,
    0x808000, 0x800080, 0x008080, 0xC0C0C0, 0xE6E6E6, 0xCDCDCD, 0xB3B3B3, 0x9A9A9A, 0x808080, 0x666666, 0x4D4D4D,
    0x333333, 0x1A1A1A,
];

/// Cached values that mean "computed elsewhere", so inheritance must continue.
pub fn is_placeholder(v: &str) -> bool {
    v.eq_ignore_ascii_case("Themed")
}

pub fn parse_num(v: &str) -> Option<f64> {
    let t = v.trim();
    if t.is_empty() || is_placeholder(t) {
        return None;
    }
    t.parse::<f64>().ok().filter(|n| n.is_finite())
}

pub fn parse_bool(v: &str) -> Option<bool> {
    match v.trim() {
        "1" | "TRUE" | "true" | "-1" => Some(true),
        "0" | "FALSE" | "false" => Some(false),
        other => parse_num(other).map(|n| n != 0.0),
    }
}

fn hex_rgb(s: &str) -> Option<Rgba> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some(Rgba((n >> 16) as u8, (n >> 8) as u8, n as u8, 255))
}

/// A colour cell: `#rrggbb`, a colour-table index, or `RGB(r,g,b)`.
pub fn parse_color(v: &str, doc_colors: &BTreeMap<u32, String>) -> Option<Rgba> {
    let t = v.trim();
    if let Some(c) = hex_rgb(t) {
        return Some(c);
    }
    if let Ok(ix) = t.parse::<u32>() {
        if let Some(&rgb) = BASE_PALETTE.get(ix as usize) {
            return Some(Rgba((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255));
        }
        return doc_colors.get(&ix).and_then(|s| hex_rgb(s));
    }
    let upper = t.to_ascii_uppercase();
    if let Some(args) = upper.strip_prefix("RGB(").and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<u8> = args.split(',').filter_map(|p| p.trim().parse::<u8>().ok()).collect();
        if parts.len() == 3 {
            return Some(Rgba(parts[0], parts[1], parts[2], 255));
        }
    }
    None
}

/// Arguments of a function-call value such as `POLYLINE(0, 0, 1,0, 0,1)` or
/// `NURBS(...)`: all numeric. Returns `None` if `name` does not match or an argument is not a number.
pub fn parse_numeric_call(v: &str, name: &str) -> Option<Vec<f64>> {
    let t = v.trim();
    let open = t.find('(')?;
    if !t[..open].trim().eq_ignore_ascii_case(name) || !t.ends_with(')') {
        return None;
    }
    let inner = &t[open + 1..t.len() - 1];
    if inner.trim().is_empty() {
        return Some(Vec::new());
    }
    inner.split(',').map(|a| a.trim().parse::<f64>().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        let doc = BTreeMap::from([(24, "#7F7F7F".to_string())]);
        assert_eq!(parse_color("#ffc000", &doc), Some(Rgba(255, 192, 0, 255)));
        assert_eq!(parse_color("2", &doc), Some(Rgba(255, 0, 0, 255)));
        assert_eq!(parse_color("24", &doc), Some(Rgba(127, 127, 127, 255)));
        assert_eq!(parse_color("RGB(1, 2, 3)", &doc), Some(Rgba(1, 2, 3, 255)));
        assert_eq!(parse_color("Themed", &doc), None);
    }

    #[test]
    fn calls() {
        assert_eq!(
            parse_numeric_call("POLYLINE(0, 0, 1,0, 0,1)", "POLYLINE"),
            Some(vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0])
        );
        assert_eq!(parse_numeric_call("NURBS(1, 3)", "POLYLINE"), None);
        assert_eq!(parse_num("Themed"), None);
        assert_eq!(parse_num("0.25"), Some(0.25));
    }
}
