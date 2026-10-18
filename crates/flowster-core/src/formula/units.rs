//! Unit literals in formulas (SPEC §6.4). Everything converts to internal
//! units: inches for lengths, radians for angles, days for durations.

use core::f64::consts::PI;

/// Factor that converts a value written in `unit` to internal units.
pub fn unit_factor(unit: &str) -> Option<f64> {
    let u = unit.to_ascii_uppercase();
    Some(match u.as_str() {
        // Lengths.
        "IN" | "IN." | "INCH" | "INCHES" => 1.0,
        "FT" | "FOOT" | "FEET" => 12.0,
        "YD" | "YARD" | "YARDS" => 36.0,
        "MI" | "MILE" | "MILES" => 63_360.0,
        "NM" => 72_913.385_826_771_65,
        "MM" => 1.0 / 25.4,
        "CM" => 1.0 / 2.54,
        "M" => 1.0 / 0.0254,
        "KM" => 1000.0 / 0.0254,
        "PT" => 1.0 / 72.0,
        // Picas, ciceros, didots.
        "P" => 1.0 / 6.0,
        "C" | "CI" => 12.0 * 0.376_065 / 25.4,
        "D" | "DI" => 0.376_065 / 25.4,
        // Default drawing/page/text lengths: values are already internal.
        "DL" | "DP" | "DT" | "DU" => 1.0,
        // Angles.
        "DEG" | "°" | "DA" => PI / 180.0,
        "RAD" => 1.0,
        "MIN" => PI / (180.0 * 60.0),
        "SEC" => PI / (180.0 * 3600.0),
        // Durations (elapsed): days.
        "ED" => 1.0,
        "EH" => 1.0 / 24.0,
        "EM" => 1.0 / 1440.0,
        "ES" => 1.0 / 86_400.0,
        "EW" => 7.0,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(unit_factor("in"), Some(1.0));
        assert!((unit_factor("MM").unwrap() * 25.4 - 1.0).abs() < 1e-15);
        assert!((unit_factor("deg").unwrap() * 180.0 - PI).abs() < 1e-15);
        assert_eq!(unit_factor("Width"), None);
    }
}
