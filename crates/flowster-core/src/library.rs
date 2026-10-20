//! Flowster's built-in shape library (SPEC §16, own P0 content): the shapes
//! the drawing tools and the Shapes panel create, as plain Visio shapes.
//!
//! Outlines use Visio's relative geometry rows (`RelMoveTo`, `RelLineTo`,
//! `RelCubBezTo`: coordinates as fractions of the shape's width and height), so
//! a shape keeps its form when resized and saves as standard VSDX geometry.
//! Cached values of formula cells are filled in by recalculation after the
//! shape is written.

use crate::calc::format_num;
use crate::model::{Cell, Row, Section, Shape, Sheet};

/// Visio's look for drawn shapes under the Office theme.
const FILL: &str = "#4472c4";
const OUTLINE: &str = "#2f528f";
const LINE_WEIGHT: &str = "0.01041666666666667";
const FONT_SIZE: &str = "0.1666666666666667";
/// Swimlane colours: a darker title band, lighter lane headers.
const POOL_BAND: &str = "#b4c7e7";
const LANE_HEADER: &str = "#dae3f3";
const LANE_LINE: &str = "#8497b0";
const DARK_TEXT: &str = "#1f2937";
/// Lane header / pool band thickness, inches.
const BAND: f64 = 0.4;
/// Bézier constant for quarter ellipses.
const K: f64 = 0.552_284_749_830_793_4;

/// A library entry as the Shapes panel lists it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    /// Default size in inches (a click drops it at this size).
    pub width: f64,
    pub height: f64,
}

const fn e(id: &'static str, name: &'static str, category: &'static str, width: f64, height: f64) -> Entry {
    Entry { id, name, category, width, height }
}

/// Every entry, in panel order.
pub const ENTRIES: &[Entry] = &[
    e("rectangle", "Rectangle", "Basic", 1.5, 1.0),
    e("rounded", "Rounded rectangle", "Basic", 1.5, 1.0),
    e("ellipse", "Ellipse", "Basic", 1.5, 1.0),
    e("circle", "Circle", "Basic", 1.0, 1.0),
    e("triangle", "Triangle", "Basic", 1.2, 1.0),
    e("right-triangle", "Right triangle", "Basic", 1.2, 1.0),
    e("diamond", "Diamond", "Basic", 1.2, 1.2),
    e("pentagon", "Pentagon", "Basic", 1.2, 1.15),
    e("hexagon", "Hexagon", "Basic", 1.3, 1.1),
    e("octagon", "Octagon", "Basic", 1.2, 1.2),
    e("parallelogram", "Parallelogram", "Basic", 1.6, 0.9),
    e("trapezoid", "Trapezoid", "Basic", 1.5, 0.9),
    e("star", "Star", "Basic", 1.2, 1.15),
    e("cross", "Cross", "Basic", 1.0, 1.0),
    e("arrow", "Block arrow", "Basic", 1.6, 0.9),
    e("process", "Process", "Flowchart", 1.5, 0.9),
    e("decision", "Decision", "Flowchart", 1.5, 1.1),
    e("terminator", "Start / End", "Flowchart", 1.5, 0.6),
    e("data", "Data", "Flowchart", 1.6, 0.9),
    e("document", "Document", "Flowchart", 1.5, 1.0),
    e("predefined", "Predefined process", "Flowchart", 1.6, 0.9),
    e("manual-input", "Manual input", "Flowchart", 1.5, 0.9),
    e("preparation", "Preparation", "Flowchart", 1.6, 0.9),
    e("database", "Database", "Flowchart", 1.1, 1.2),
    e("off-page", "Off-page reference", "Flowchart", 0.8, 0.8),
    e("on-page", "On-page reference", "Flowchart", 0.5, 0.5),
    e("delay", "Delay", "Flowchart", 1.4, 0.9),
    e("manual-operation", "Manual operation", "Flowchart", 1.5, 0.9),
    e("note", "Note", "Flowchart", 1.5, 1.1),
    e("text", "Text", "Text", 1.5, 0.4),
    e("pool", "Swimlane pool (3 lanes)", "Swimlanes", 8.0, 4.9),
    e("lane", "Swimlane lane", "Swimlanes", 8.0, 1.5),
];

pub fn entry(id: &str) -> Option<&'static Entry> {
    ENTRIES.iter().find(|e| e.id == id)
}

fn v(value: impl Into<String>) -> Cell {
    Cell { v: Some(value.into()), f: None, u: None }
}

fn f(formula: &str) -> Cell {
    Cell { v: None, f: Some(formula.into()), u: None }
}

/// A geometry row with constant cells.
fn row(t: &str, ix: u32, cells: &[(&str, f64)]) -> Row {
    Row {
        t: Some(t.into()),
        ix: Some(ix),
        cells: cells.iter().map(|(k, x)| ((*k).to_owned(), v(format_num(*x)))).collect(),
        ..Default::default()
    }
}

/// A geometry row of formulas.
fn frow(t: &str, ix: u32, cells: &[(&str, &str)]) -> Row {
    Row {
        t: Some(t.into()),
        ix: Some(ix),
        cells: cells.iter().map(|(k, x)| ((*k).to_owned(), f(x))).collect(),
        ..Default::default()
    }
}

/// Path segments in the unit box (y up), turned into relative rows.
enum P {
    M(f64, f64),
    L(f64, f64),
    /// Cubic Bézier: control points then end.
    C(f64, f64, f64, f64, f64, f64),
}

fn rel_section(ix: u32, path: &[P], no_fill: bool, no_line: bool) -> Section {
    let rows = path
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let n = i as u32 + 1;
            match *p {
                P::M(x, y) => row("RelMoveTo", n, &[("X", x), ("Y", y)]),
                P::L(x, y) => row("RelLineTo", n, &[("X", x), ("Y", y)]),
                P::C(a, b, c, d, x, y) => {
                    row("RelCubBezTo", n, &[("X", x), ("Y", y), ("A", a), ("B", b), ("C", c), ("D", d)])
                }
            }
        })
        .collect();
    let mut cells = std::collections::BTreeMap::new();
    if no_fill {
        cells.insert("NoFill".to_owned(), v("1"));
    }
    if no_line {
        cells.insert("NoLine".to_owned(), v("1"));
    }
    Section { n: "Geometry".into(), ix: Some(ix), cells, rows, ..Default::default() }
}

/// A closed polygon through unit-box points.
fn polygon(points: &[(f64, f64)]) -> Vec<P> {
    let mut path: Vec<P> = Vec::with_capacity(points.len() + 1);
    path.push(P::M(points[0].0, points[0].1));
    for &(x, y) in &points[1..] {
        path.push(P::L(x, y));
    }
    path.push(P::L(points[0].0, points[0].1));
    path
}

fn ellipse_section() -> Section {
    Section {
        n: "Geometry".into(),
        ix: Some(0),
        rows: vec![frow(
            "Ellipse",
            1,
            &[
                ("X", "Width*0.5"),
                ("Y", "Height*0.5"),
                ("A", "Width*1"),
                ("B", "Height*0.5"),
                ("C", "Width*0.5"),
                ("D", "Height*1"),
            ],
        )],
        ..Default::default()
    }
}

/// The outline sections of a library shape.
fn geometry(id: &str) -> Option<Vec<Section>> {
    let one = |path: Vec<P>| vec![rel_section(0, &path, false, false)];
    let rect = || polygon(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
    Some(match id {
        "rectangle" | "rounded" | "process" | "terminator" | "text" => one(rect()),
        "ellipse" | "circle" | "on-page" => vec![ellipse_section()],
        "triangle" => one(polygon(&[(0.0, 0.0), (1.0, 0.0), (0.5, 1.0)])),
        "right-triangle" => one(polygon(&[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)])),
        "diamond" | "decision" => one(polygon(&[(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)])),
        "pentagon" => one(polygon(&[(0.5, 1.0), (1.0, 0.618), (0.809, 0.0), (0.191, 0.0), (0.0, 0.618)])),
        "hexagon" => one(polygon(&[(0.25, 0.0), (0.75, 0.0), (1.0, 0.5), (0.75, 1.0), (0.25, 1.0), (0.0, 0.5)])),
        "preparation" => one(polygon(&[(0.0, 0.5), (0.15, 0.0), (0.85, 0.0), (1.0, 0.5), (0.85, 1.0), (0.15, 1.0)])),
        "octagon" => {
            // Equal sides: each corner cut is 1 - 1/√2 of the side.
            let (a, b) = (1.0 - std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2);
            one(polygon(&[(a, 0.0), (b, 0.0), (1.0, a), (1.0, b), (b, 1.0), (a, 1.0), (0.0, b), (0.0, a)]))
        }
        "parallelogram" => one(polygon(&[(0.0, 0.0), (0.8, 0.0), (1.0, 1.0), (0.2, 1.0)])),
        "data" => one(polygon(&[(0.0, 0.0), (0.85, 0.0), (1.0, 1.0), (0.15, 1.0)])),
        "trapezoid" => one(polygon(&[(0.0, 0.0), (1.0, 0.0), (0.8, 1.0), (0.2, 1.0)])),
        "manual-operation" => one(polygon(&[(0.2, 0.0), (0.8, 0.0), (1.0, 1.0), (0.0, 1.0)])),
        "manual-input" => one(polygon(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 0.75)])),
        "off-page" => one(polygon(&[(0.0, 1.0), (1.0, 1.0), (1.0, 0.35), (0.5, 0.0), (0.0, 0.35)])),
        "star" => {
            let pts: Vec<(f64, f64)> = (0..10)
                .map(|i| {
                    let a = std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::PI / 5.0;
                    let r = if i % 2 == 0 { 0.5 } else { 0.2 };
                    (0.5 + r * libm::cos(a), 0.5 + r * libm::sin(a) * 1.05 - 0.02)
                })
                .collect();
            one(polygon(&pts))
        }
        "cross" => one(polygon(&[
            (0.33, 0.0),
            (0.67, 0.0),
            (0.67, 0.33),
            (1.0, 0.33),
            (1.0, 0.67),
            (0.67, 0.67),
            (0.67, 1.0),
            (0.33, 1.0),
            (0.33, 0.67),
            (0.0, 0.67),
            (0.0, 0.33),
            (0.33, 0.33),
        ])),
        "arrow" => {
            one(polygon(&[(0.0, 0.3), (0.65, 0.3), (0.65, 0.0), (1.0, 0.5), (0.65, 1.0), (0.65, 0.7), (0.0, 0.7)]))
        }
        "document" => one(vec![
            P::M(0.0, 0.12),
            P::L(0.0, 1.0),
            P::L(1.0, 1.0),
            P::L(1.0, 0.12),
            P::C(0.6, 0.32, 0.4, -0.08, 0.0, 0.12),
        ]),
        "predefined" => vec![
            rel_section(0, &rect(), false, false),
            rel_section(1, &[P::M(0.1, 0.0), P::L(0.1, 1.0)], true, false),
            rel_section(2, &[P::M(0.9, 0.0), P::L(0.9, 1.0)], true, false),
        ],
        "database" => {
            let (t, b) = (0.85, 0.15);
            vec![
                rel_section(
                    0,
                    &[
                        P::M(0.0, t),
                        P::L(0.0, b),
                        P::C(0.0, b - b * K, 0.5 - 0.5 * K, 0.0, 0.5, 0.0),
                        P::C(0.5 + 0.5 * K, 0.0, 1.0, b - b * K, 1.0, b),
                        P::L(1.0, t),
                        P::C(1.0, t + b * K, 0.5 + 0.5 * K, 1.0, 0.5, 1.0),
                        P::C(0.5 - 0.5 * K, 1.0, 0.0, t + b * K, 0.0, t),
                    ],
                    false,
                    false,
                ),
                rel_section(
                    1,
                    &[
                        P::M(0.0, t),
                        P::C(0.0, t - b * K, 0.5 - 0.5 * K, t - b, 0.5, t - b),
                        P::C(0.5 + 0.5 * K, t - b, 1.0, t - b * K, 1.0, t),
                    ],
                    true,
                    false,
                ),
            ]
        }
        "delay" => one(vec![
            P::M(0.0, 0.0),
            P::L(0.6, 0.0),
            P::C(0.6 + 0.4 * K, 0.0, 1.0, 0.5 - 0.5 * K, 1.0, 0.5),
            P::C(1.0, 0.5 + 0.5 * K, 0.6 + 0.4 * K, 1.0, 0.6, 1.0),
            P::L(0.0, 1.0),
            P::L(0.0, 0.0),
        ]),
        "note" => vec![
            rel_section(0, &polygon(&[(0.0, 0.0), (1.0, 0.0), (1.0, 0.78), (0.82, 1.0), (0.0, 1.0)]), false, false),
            rel_section(1, &[P::M(0.82, 1.0), P::L(0.82, 0.78), P::L(1.0, 0.78)], true, false),
        ],
        _ => return None,
    })
}

/// Fill, line and text cells shared by the 2-D shapes.
fn styled(sheet: &mut Sheet, fill: Option<&str>, line: Option<&str>, text: &str) {
    sheet.cells.insert("LineWeight".into(), v(LINE_WEIGHT));
    match line {
        Some(c) => {
            sheet.cells.insert("LinePattern".into(), v("1"));
            sheet.cells.insert("LineColor".into(), v(c));
        }
        None => {
            sheet.cells.insert("LinePattern".into(), v("0"));
        }
    }
    match fill {
        Some(c) => {
            sheet.cells.insert("FillPattern".into(), v("1"));
            sheet.cells.insert("FillForegnd".into(), v(c));
        }
        None => {
            sheet.cells.insert("FillPattern".into(), v("0"));
        }
    }
    sheet.cells.insert("VerticalAlign".into(), v("1"));
    sheet.sections.push(Section {
        n: "Character".into(),
        rows: vec![Row {
            ix: Some(0),
            cells: [("Color", text), ("Size", FONT_SIZE), ("Font", "Calibri")]
                .into_iter()
                .map(|(k, x)| (k.to_owned(), v(x)))
                .collect(),
            ..Default::default()
        }],
        ..Default::default()
    });
    sheet.sections.push(Section {
        n: "Paragraph".into(),
        rows: vec![Row { ix: Some(0), cells: [("HorzAlign".to_owned(), v("1"))].into(), ..Default::default() }],
        ..Default::default()
    });
}

/// A 2-D shape filling the box (x, y)–(x + w, y + h).
fn shape_2d(id: u32, x: f64, y: f64, w: f64, h: f64, sections: Vec<Section>) -> Shape {
    let mut sheet = Sheet::default();
    for (name, value) in [("PinX", x + w / 2.0), ("PinY", y + h / 2.0), ("Width", w), ("Height", h), ("Angle", 0.0)] {
        sheet.cells.insert(name.into(), v(format_num(value)));
    }
    sheet.cells.insert("LocPinX".into(), f("Width*0.5"));
    sheet.cells.insert("LocPinY".into(), f("Height*0.5"));
    sheet.sections = sections;
    Shape { id, name_u: Some(format!("Sheet.{id}")), name: Some(format!("Sheet.{id}")), sheet, ..Default::default() }
}

/// A 1-D line from (x0, y0) to (x1, y1), with Visio's 1-D formulas.
pub fn line(id: u32, x0: f64, y0: f64, x1: f64, y1: f64) -> Shape {
    let mut sheet = Sheet::default();
    for (name, value) in [("BeginX", x0), ("BeginY", y0), ("EndX", x1), ("EndY", y1)] {
        sheet.cells.insert(name.into(), v(format_num(value)));
    }
    for (name, formula) in [
        ("PinX", "(BeginX+EndX)/2"),
        ("PinY", "(BeginY+EndY)/2"),
        ("Width", "SQRT((EndX-BeginX)^2+(EndY-BeginY)^2)"),
        ("Angle", "ATAN2(EndY-BeginY,EndX-BeginX)"),
        ("LocPinX", "Width*0.5"),
        ("LocPinY", "Height*0.5"),
    ] {
        sheet.cells.insert(name.into(), f(formula));
    }
    sheet.cells.insert("Height".into(), v("0"));
    sheet.cells.insert("LineWeight".into(), v(LINE_WEIGHT));
    sheet.cells.insert("LinePattern".into(), v("1"));
    sheet.cells.insert("LineColor".into(), v(FILL));
    sheet.sections.push(Section {
        n: "Geometry".into(),
        ix: Some(0),
        cells: [("NoFill".to_owned(), v("1"))].into(),
        rows: vec![
            frow("MoveTo", 1, &[("X", "Width*0"), ("Y", "Height*0.5")]),
            frow("LineTo", 2, &[("X", "Width*1"), ("Y", "Height*0.5")]),
        ],
        ..Default::default()
    });
    Shape { id, name_u: Some(format!("Sheet.{id}")), name: Some(format!("Sheet.{id}")), sheet, ..Default::default() }
}

/// A dynamic connector from (x0, y0) to (x1, y1), glued to shapes `from` and
/// `to` when given. Glue is Visio's own: trigger formulas naming the shapes,
/// and `_WALKGLUE` end formulas whose cached values the engine maintains. The
/// path rows are written by the engine's router.
pub fn connector(id: u32, x0: f64, y0: f64, x1: f64, y1: f64, from: Option<u32>, to: Option<u32>) -> Shape {
    let mut sheet = Sheet::default();
    let walk = |end: &str| Cell {
        v: Some(format_num(match end {
            "BeginX" => x0,
            "BeginY" => y0,
            "EndX" => x1,
            _ => y1,
        })),
        f: None,
        u: None,
    };
    for name in ["BeginX", "BeginY", "EndX", "EndY"] {
        sheet.cells.insert(name.into(), walk(name));
    }
    glue_cells(&mut sheet, from, to);
    for (name, formula) in [
        ("PinX", "(BeginX+EndX)/2"),
        ("PinY", "(BeginY+EndY)/2"),
        ("Width", "EndX-BeginX"),
        ("Height", "EndY-BeginY"),
        ("LocPinX", "Width*0.5"),
        ("LocPinY", "Height*0.5"),
    ] {
        sheet.cells.insert(name.into(), f(formula));
    }
    sheet.cells.insert("Angle".into(), v("0"));
    sheet.cells.insert("ObjType".into(), v("2"));
    sheet.cells.insert("LineWeight".into(), v(LINE_WEIGHT));
    sheet.cells.insert("LinePattern".into(), v("1"));
    sheet.cells.insert("LineColor".into(), v("#3b4a5c"));
    sheet.cells.insert("EndArrow".into(), v("4"));
    sheet.cells.insert("EndArrowSize".into(), v("2"));
    sheet.cells.insert("Rounding".into(), v("0"));
    sheet.sections.push(Section {
        n: "Geometry".into(),
        ix: Some(0),
        cells: [("NoFill".to_owned(), v("1"))].into(),
        rows: vec![row("MoveTo", 1, &[("X", 0.0), ("Y", 0.0)]), row("LineTo", 2, &[("X", x1 - x0), ("Y", y1 - y0)])],
        ..Default::default()
    });
    Shape {
        id,
        name_u: Some(format!("Dynamic connector.{id}")),
        name: Some(format!("Dynamic connector.{id}")),
        sheet,
        ..Default::default()
    }
}

/// Writes (or clears) a connector's glue: the end follows shape `to` while its
/// trigger names it.
pub fn glue_cells(sheet: &mut Sheet, from: Option<u32>, to: Option<u32>) {
    for (trigger, ends, target) in [("BegTrigger", ["BeginX", "BeginY"], from), ("EndTrigger", ["EndX", "EndY"], to)] {
        match target {
            Some(t) => {
                sheet.cells.insert(
                    trigger.into(),
                    Cell { v: Some("2".into()), f: Some(format!("_XFTRIGGER(Sheet.{t}!EventXFMod)")), u: None },
                );
                for end in ends {
                    let c = sheet.cells.entry(end.into()).or_default();
                    c.f = Some("_WALKGLUE(BegTrigger,EndTrigger,WalkPreference)".into());
                }
            }
            None => {
                sheet.cells.remove(trigger);
                for end in ends {
                    if let Some(c) = sheet.cells.get_mut(end) {
                        c.f = None;
                    }
                }
            }
        }
    }
}

/// Visio's `Connect` records for a page, derived from its connectors' glue
/// formulas (the formulas are the truth; the records are a cache Visio reads).
/// Records for connectors without Flowster-readable glue are kept as they were.
pub fn derive_connects(page: &crate::model::Page) -> Vec<crate::model::Connect> {
    use crate::model::Connect;
    let mut derived = Vec::new();
    let mut glued = std::collections::BTreeSet::new();
    for s in &page.shapes {
        for (trigger, cell) in [("BegTrigger", "BeginX"), ("EndTrigger", "EndX")] {
            if let Some(t) = glue_target(&s.sheet, trigger) {
                glued.insert(s.id);
                derived.push(Connect {
                    from_sheet: s.id,
                    from_cell: Some(cell.into()),
                    to_sheet: t,
                    to_cell: Some("PinX".into()),
                });
            }
        }
    }
    let mut out: Vec<Connect> = page.connects.iter().filter(|c| !glued.contains(&c.from_sheet)).cloned().collect();
    out.extend(derived);
    out
}

/// The shape a connector end is glued to, read from its trigger formula.
pub fn glue_target(sheet: &Sheet, trigger: &str) -> Option<u32> {
    let f = sheet.cells.get(trigger)?.f.as_deref()?;
    let rest = &f[f.find("Sheet.")? + 6..];
    rest[..rest.find('!')?].parse().ok()
}

/// A horizontal swimlane: an unfilled body with a filled header strip on the
/// left whose text reads bottom to top.
fn lane(id: u32, x: f64, y: f64, w: f64, h: f64, title: &str) -> Shape {
    let body = rel_section(0, &polygon(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]), true, false);
    let header = Section {
        n: "Geometry".into(),
        ix: Some(1),
        rows: vec![
            frow("MoveTo", 1, &[("X", "0"), ("Y", "0")]),
            frow("LineTo", 2, &[("X", "0.4"), ("Y", "0")]),
            frow("LineTo", 3, &[("X", "0.4"), ("Y", "Height")]),
            frow("LineTo", 4, &[("X", "0"), ("Y", "Height")]),
            frow("LineTo", 5, &[("X", "0"), ("Y", "0")]),
        ],
        ..Default::default()
    };
    let mut s = shape_2d(id, x, y, w, h, vec![body, header]);
    styled(&mut s.sheet, Some(LANE_HEADER), Some(LANE_LINE), DARK_TEXT);
    for (name, formula) in [
        ("TxtWidth", "Height"),
        ("TxtPinY", "Height*0.5"),
        ("TxtLocPinX", "TxtWidth*0.5"),
        ("TxtLocPinY", "TxtHeight*0.5"),
    ] {
        s.sheet.cells.insert(name.into(), f(formula));
    }
    s.sheet.cells.insert("TxtHeight".into(), v(format_num(BAND)));
    s.sheet.cells.insert("TxtPinX".into(), v(format_num(BAND / 2.0)));
    s.sheet.cells.insert("TxtAngle".into(), v(format_num(std::f64::consts::FRAC_PI_2)));
    s.text = Some(vec![crate::model::TextPiece::Text(title.into())]);
    s
}

/// The shapes for library entry `id` filling the box (x, y)–(x + w, y + h)
/// (a line runs from (x, y) to (x + w, y + h)), numbered from `first_id`.
pub fn build(id: &str, first_id: u32, x: f64, y: f64, w: f64, h: f64) -> Option<Vec<Shape>> {
    match id {
        "line" => return Some(vec![line(first_id, x, y, x + w, y + h)]),
        "lane" => return Some(vec![lane(first_id, x, y, w, h, "Lane")]),
        "pool" => {
            // A titled frame with three lanes stacked under the band.
            let frame_body = rel_section(0, &polygon(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]), true, false);
            let band = Section {
                n: "Geometry".into(),
                ix: Some(1),
                rows: vec![
                    frow("MoveTo", 1, &[("X", "0"), ("Y", "Height-0.4")]),
                    frow("LineTo", 2, &[("X", "Width"), ("Y", "Height-0.4")]),
                    frow("LineTo", 3, &[("X", "Width"), ("Y", "Height")]),
                    frow("LineTo", 4, &[("X", "0"), ("Y", "Height")]),
                    frow("LineTo", 5, &[("X", "0"), ("Y", "Height-0.4")]),
                ],
                ..Default::default()
            };
            let mut pool = shape_2d(first_id, x, y, w, h, vec![frame_body, band]);
            styled(&mut pool.sheet, Some(POOL_BAND), Some(LANE_LINE), DARK_TEXT);
            for (name, formula) in [
                ("TxtWidth", "Width"),
                ("TxtPinX", "Width*0.5"),
                ("TxtPinY", "Height-0.2"),
                ("TxtLocPinX", "TxtWidth*0.5"),
                ("TxtLocPinY", "TxtHeight*0.5"),
            ] {
                pool.sheet.cells.insert(name.into(), f(formula));
            }
            pool.sheet.cells.insert("TxtHeight".into(), v(format_num(BAND)));
            pool.text = Some(vec![crate::model::TextPiece::Text("Process".into())]);
            let lane_h = (h - BAND) / 3.0;
            let mut out = vec![pool];
            for i in 0..3u32 {
                let ly = y + h - BAND - lane_h * f64::from(i + 1);
                out.push(lane(first_id + 1 + i, x, ly, w, lane_h, &format!("Lane {}", i + 1)));
            }
            return Some(out);
        }
        _ => {}
    }
    let mut s = shape_2d(first_id, x, y, w, h, geometry(id)?);
    if id == "text" {
        styled(&mut s.sheet, None, None, "#000000");
    } else {
        styled(&mut s.sheet, Some(FILL), Some(OUTLINE), "#ffffff");
    }
    match id {
        "rounded" => {
            s.sheet.cells.insert("Rounding".into(), v("0.1"));
        }
        "terminator" => {
            s.sheet.cells.insert("Rounding".into(), f("MIN(Width,Height)*0.5"));
        }
        _ => {}
    }
    Some(vec![s])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_builds() {
        for e in ENTRIES {
            let shapes = build(e.id, 1, 0.0, 0.0, e.width, e.height).unwrap_or_else(|| panic!("{}", e.id));
            assert!(!shapes.is_empty(), "{}", e.id);
            assert!(shapes.iter().all(|s| !s.sheet.sections.is_empty()), "{}", e.id);
        }
        assert_eq!(
            build("pool", 5, 0.0, 0.0, 8.0, 4.9).unwrap().iter().map(|s| s.id).collect::<Vec<_>>(),
            [5, 6, 7, 8]
        );
        assert!(build("nonsense", 1, 0.0, 0.0, 1.0, 1.0).is_none());
    }
}
