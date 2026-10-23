//! Paper sizes for page setup (Design > Size) and Auto Size's sheets.
//!
//! A page sheet names its printer paper with `PaperKind`, a Windows paper
//! code, and `PrintPageOrientation` (1 portrait, 2 landscape). The UI keeps the
//! same list of sizes for its Size menu.

/// The sizes offered, by `PaperKind`: name and portrait width × height in inches.
pub const PAPER: &[(u32, &str, f64, f64)] = &[
    (1, "Letter", 8.5, 11.0),
    (5, "Legal", 8.5, 14.0),
    (3, "Tabloid", 11.0, 17.0),
    (11, "A5", 5.83, 8.27),
    (9, "A4", 8.27, 11.69),
    (8, "A3", 11.69, 16.54),
    (66, "A2", 16.54, 23.39),
];

/// A paper kind's portrait size in inches, if Flowster knows it.
pub fn paper_size(kind: u32) -> Option<(f64, f64)> {
    PAPER.iter().find(|p| p.0 == kind).map(|p| (p.2, p.3))
}

/// One sheet of a page's paper in inches, turned to its orientation: the step
/// Auto Size grows a page by. Unknown or unset paper is Letter; an unset
/// orientation follows the page's own shape.
pub fn sheet(kind: Option<u32>, orientation: Option<u32>, page: (f64, f64)) -> (f64, f64) {
    let (w, h) = kind.and_then(paper_size).unwrap_or((8.5, 11.0));
    let landscape = match orientation {
        Some(2) => true,
        Some(1) => false,
        _ => page.0 > page.1,
    };
    if landscape { (h, w) } else { (w, h) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sheet_turns_with_its_orientation() {
        assert_eq!(sheet(Some(9), Some(1), (1.0, 1.0)), (8.27, 11.69));
        assert_eq!(sheet(Some(9), Some(2), (1.0, 1.0)), (11.69, 8.27));
        assert_eq!(sheet(None, None, (11.0, 8.5)), (11.0, 8.5), "Letter, landscape like the page");
        assert_eq!(sheet(Some(256), Some(1), (1.0, 1.0)), (8.5, 11.0), "a custom paper counts as Letter");
    }
}
