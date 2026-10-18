//! Deterministic synthetic documents shaped like the SPEC §20.3 schema.
//!
//! The generator is library-neutral: it produces plain values that each CRDT
//! backend ingests the same way, so their sizes and timings are comparable.
//! The content is invented for the spike. Nothing here comes from Visio content.

use std::ops::Range;

use flowster_core::rng::SplitMix64;

/// A plain value. `Map` is what an atomic LWW group (`CellEntry`, Position, Size, …) is written as.
#[derive(Clone, Debug, PartialEq)]
pub enum Pv {
    Null,
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(String),
    Map(Vec<(String, Pv)>),
}

impl Pv {
    pub fn get(&self, key: &str) -> Option<&Pv> {
        match self {
            Pv::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Pv::Num(n) => Some(*n),
            Pv::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
}

/// One ShapeSheet section in its §20.3 storage shape.
#[derive(Clone, Debug)]
pub enum Section {
    /// Single-row section: `Map<cellName, CellEntry>` (atomic groups are one entry).
    Single(Vec<(String, Pv)>),
    /// Named-row section (`User`, `Prop`, …): `Map<rowName, Row>`.
    Named(Vec<(String, Vec<(String, Pv)>)>),
    /// Indexed section (`Geometry n`, `Character`, …): `MovableList<Row>`.
    Indexed(Vec<Vec<(String, Pv)>>),
}

#[derive(Clone, Debug)]
pub struct SynthPage {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct SynthShape {
    pub uuid: String,
    pub page: usize,
    /// Index of the parent group shape in [`SynthDoc::shapes`], if any.
    pub parent: Option<usize>,
    pub props: Vec<(&'static str, Pv)>,
    pub sheet: Vec<(String, Section)>,
    pub text: String,
    pub bold: Option<Range<usize>>,
}

#[derive(Clone, Debug)]
pub struct SynthDoc {
    pub pages: Vec<SynthPage>,
    pub masters: Vec<(String, String)>,
    pub styles: Vec<(String, String)>,
    pub shapes: Vec<SynthShape>,
}

/// Deterministic generator for synthetic content.
pub struct Rng(SplitMix64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(SplitMix64::new(seed))
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }
    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        self.0.unit()
    }
    /// UUIDv7-shaped id (fixed epoch + counter, random tail).
    pub fn uuid7(&mut self, counter: u64) -> String {
        let ms: u64 = 0x0190_0000_0000 + counter;
        let r = self.next_u64();
        let r2 = self.next_u64();
        format!(
            "{:08x}-{:04x}-7{:03x}-{:04x}-{:012x}",
            (ms >> 16) as u32,
            (ms & 0xffff) as u16,
            (r & 0xfff) as u16,
            (0x8000 | ((r >> 12) & 0x3fff)) as u16,
            r2 & 0xffff_ffff_ffff
        )
    }
}

pub fn cell(v: Pv) -> Pv {
    Pv::Map(vec![("v".into(), v)])
}
pub fn cell_f(f: &str, v: Pv) -> Pv {
    Pv::Map(vec![("f".into(), Pv::Str(f.into())), ("v".into(), v)])
}
pub fn cell_u(v: Pv, u: &str) -> Pv {
    Pv::Map(vec![("v".into(), v), ("u".into(), Pv::Str(u.into()))])
}
fn s(x: &str) -> Pv {
    Pv::Str(x.into())
}
fn kv(k: &str, v: Pv) -> (String, Pv) {
    (k.into(), v)
}

/// The Position atomic group (§20.3): `PinX, PinY + parentTag`.
pub fn position_group(pin_x: f64, pin_y: f64, parent_tag: &str) -> Pv {
    Pv::Map(vec![
        kv("PinX", cell_u(Pv::Num(pin_x), "IN")),
        kv("PinY", cell_u(Pv::Num(pin_y), "IN")),
        kv("parentTag", s(parent_tag)),
    ])
}

/// The Size atomic group (§20.3): `Width, Height, LocPinX, LocPinY`.
pub fn size_group(w: f64, h: f64) -> Pv {
    Pv::Map(vec![
        kv("Width", cell_u(Pv::Num(w), "IN")),
        kv("Height", cell_u(Pv::Num(h), "IN")),
        kv("LocPinX", cell_f("Width*0.5", Pv::Num(w * 0.5))),
        kv("LocPinY", cell_f("Height*0.5", Pv::Num(h * 0.5))),
    ])
}

/// Generates `n_shapes` shapes over `n_pages` pages.
///
/// Mix: every 50th shape is a group holding the next 4 shapes; every 10th
/// shape is a 1-D connector glued to two neighbours (canonical `{s:uuid}`
/// references, §20.4); one shape in 5 has a bold run in its text.
pub fn generate(n_shapes: usize, n_pages: usize, seed: u64) -> SynthDoc {
    let mut rng = Rng::new(seed);
    let mut counter = 0u64;
    let mut next_id = |rng: &mut Rng| {
        counter += 1;
        rng.uuid7(counter)
    };

    let n_pages = n_pages.max(1);
    let pages: Vec<SynthPage> =
        (0..n_pages).map(|i| SynthPage { id: next_id(&mut rng), name: format!("Page-{}", i + 1) }).collect();
    let masters: Vec<(String, String)> = ["Process", "Decision", "Start/End", "Document", "Data", "Dynamic connector"]
        .iter()
        .map(|n| (next_id(&mut rng), n.to_string()))
        .collect();
    let styles: Vec<(String, String)> =
        ["Normal", "Connector", "Text Only"].iter().map(|n| (next_id(&mut rng), n.to_string())).collect();

    let mut shapes: Vec<SynthShape> = Vec::with_capacity(n_shapes);
    let per_page = n_shapes.div_ceil(n_pages).max(1);
    let mut group_open: Option<(usize, usize)> = None; // (group index, children left)

    for i in 0..n_shapes {
        let page = (i / per_page).min(n_pages - 1);
        let uuid = next_id(&mut rng);
        let legacy_id = (i % per_page + 1) as i64;

        let parent = match group_open {
            Some((g, left)) if left > 0 && shapes[g].page == page => {
                group_open = if left > 1 { Some((g, left - 1)) } else { None };
                Some(g)
            }
            _ => {
                group_open = None;
                None
            }
        };
        let parent_tag = match parent {
            Some(g) => shapes[g].uuid.clone(),
            None => pages[page].id.clone(),
        };

        let is_group = parent.is_none() && i % 50 == 0;
        let is_connector = !is_group && i % 10 == 9 && i >= 2 && shapes[i - 2].page == page;

        let px = 1.0 + rng.unit() * 30.0;
        let py = 1.0 + rng.unit() * 20.0;
        let (w, h) = (0.75 + rng.unit() * 1.5, 0.5 + rng.unit());

        let (kind, master) = if is_group {
            ("Group", None)
        } else if is_connector {
            ("Shape", Some(5))
        } else {
            ("Shape", Some((rng.next_u64() % 5) as usize))
        };

        let mut props: Vec<(&'static str, Pv)> = vec![
            ("type", s(kind)),
            ("nameU", s(&format!("{}.{}", master.map(|m| masters[m].1.as_str()).unwrap_or("Sheet"), legacy_id))),
            ("legacyId", Pv::Int(legacy_id)),
        ];
        if let Some(m) = master {
            props.push(("master", s(&masters[m].0)));
            props.push(("masterShape", Pv::Int(5)));
        }
        let style = if is_connector { &styles[1].0 } else { &styles[0].0 };
        props.push(("lineStyle", s(style)));
        props.push(("fillStyle", s(style)));
        props.push(("textStyle", s(style)));

        let mut sheet: Vec<(String, Section)> = Vec::new();
        if is_connector {
            let a = &shapes[i - 1].uuid;
            let b = &shapes[i - 2].uuid;
            let glue = |sh: &str, c: &str| format!("PAR(PNT({{s:{sh}}}!Connections.{c}1,{{s:{sh}}}!Connections.{c}1))");
            let (bx, by, ex, ey) = (px, py, px + 2.0, py + 1.0);
            sheet.push((
                "XForm".into(),
                Section::Single(vec![
                    kv(
                        "Endpoints",
                        Pv::Map(vec![
                            kv("BeginX", cell_f(&glue(a, "X"), Pv::Num(bx))),
                            kv("BeginY", cell_f(&glue(a, "Y"), Pv::Num(by))),
                            kv("EndX", cell_f(&glue(b, "X"), Pv::Num(ex))),
                            kv("EndY", cell_f(&glue(b, "Y"), Pv::Num(ey))),
                            kv(
                                "lastWorld",
                                Pv::Map(vec![
                                    kv("bx", Pv::Num(bx)),
                                    kv("by", Pv::Num(by)),
                                    kv("ex", Pv::Num(ex)),
                                    kv("ey", Pv::Num(ey)),
                                ]),
                            ),
                        ]),
                    ),
                    kv("Position", position_group(px + 1.0, py + 0.5, &parent_tag)),
                ]),
            ));
            sheet.push((
                "Line".into(),
                Section::Single(vec![
                    kv("LineWeight", cell_u(Pv::Num(0.0104), "PT")),
                    kv("LineColor", cell_f("THEMEVAL(\"LineColor\")", s("#41719c"))),
                    kv("EndArrow", cell(Pv::Int(13))),
                ]),
            ));
            sheet.push((
                "Geometry1".into(),
                Section::Indexed(vec![
                    vec![kv("rowType", s("MoveTo")), kv("X", cell(Pv::Num(0.0))), kv("Y", cell(Pv::Num(0.0)))],
                    vec![
                        kv("rowType", s("LineTo")),
                        kv("X", cell_f("Width", Pv::Num(2.23))),
                        kv("Y", cell(Pv::Num(0.0))),
                    ],
                ]),
            ));
        } else {
            sheet.push((
                "XForm".into(),
                Section::Single(vec![
                    kv("Position", position_group(px, py, &parent_tag)),
                    kv("Size", size_group(w, h)),
                    kv("Rotation", Pv::Map(vec![kv("Angle", cell_u(Pv::Num(0.0), "DEG"))])),
                    kv("Flip", Pv::Map(vec![kv("FlipX", cell(Pv::Bool(false))), kv("FlipY", cell(Pv::Bool(false)))])),
                ]),
            ));
            sheet.push((
                "Line".into(),
                Section::Single(vec![
                    kv("LineWeight", cell_u(Pv::Num(0.0104), "PT")),
                    kv("LineColor", cell_f("THEMEVAL(\"LineColor\")", s("#41719c"))),
                    kv("LinePattern", cell(Pv::Int(1))),
                ]),
            ));
            sheet.push((
                "Fill".into(),
                Section::Single(vec![
                    kv("FillForegnd", cell_f("THEMEVAL(\"FillColor\")", s("#5b9bd5"))),
                    kv("FillPattern", cell(Pv::Int(1))),
                ]),
            ));
            sheet.push((
                "Character".into(),
                Section::Indexed(vec![vec![
                    kv("Font", cell(s("Flowster Sans"))),
                    kv("Size", cell_u(Pv::Num(0.1667), "PT")),
                    kv("Color", cell_f("THEMEVAL(\"TextColor\")", s("#ffffff"))),
                ]]),
            ));
            let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)];
            let mut geo = vec![vec![
                kv("rowType", s("Geometry")),
                kv("NoFill", cell(Pv::Bool(false))),
                kv("NoLine", cell(Pv::Bool(false))),
            ]];
            for (k, (gx, gy)) in corners.iter().enumerate() {
                geo.push(vec![
                    kv("rowType", s(if k == 0 { "MoveTo" } else { "LineTo" })),
                    kv("X", cell_f(&format!("Width*{gx}"), Pv::Num(w * gx))),
                    kv("Y", cell_f(&format!("Height*{gy}"), Pv::Num(h * gy))),
                ]);
            }
            sheet.push(("Geometry1".into(), Section::Indexed(geo)));
            sheet.push((
                "Prop".into(),
                Section::Named(vec![
                    (
                        "Cost".into(),
                        vec![
                            kv("Value", cell_u(Pv::Num((rng.unit() * 1000.0).floor()), "CUR")),
                            kv("Label", cell(s("Cost"))),
                            kv("Type", cell(Pv::Int(7))),
                        ],
                    ),
                    (
                        "Owner".into(),
                        vec![
                            kv("Value", cell(s("Team A"))),
                            kv("Label", cell(s("Owner"))),
                            kv("Type", cell(Pv::Int(0))),
                        ],
                    ),
                ]),
            ));
        }

        let text = if is_connector || is_group { String::new() } else { format!("Step {} of the process", i + 1) };
        let bold = if !text.is_empty() && i % 5 == 0 { Some(0..4) } else { None };

        if is_group {
            group_open = Some((i, 4));
        }
        shapes.push(SynthShape { uuid, page, parent, props, sheet, text, bold });
    }

    SynthDoc { pages, masters, styles, shapes }
}
