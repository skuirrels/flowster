//! `flowster-render`: turns a page of the document model into a display list
//! (SPEC §8.5): paths in page coordinates (inches, +Y up) with fill and stroke,
//! text blocks with their transform and styled runs, and images.
//!
//! The canvas renderer in the UI, the SVG exporter ([`svg`]) and golden tests
//! all consume the same list. Text is laid out by the consumer for now; core
//! text layout (parley) replaces that later in M1 so `TEXTWIDTH` and wrapping
//! are identical everywhere.

#![forbid(unsafe_code)]

pub mod svg;

use base64::Engine as _;
use flowster_core::calc::Calc;
use flowster_core::formula::Value;
use flowster_core::geom::{Seg, round_corners, sections_to_paths};
use flowster_core::model::{Document, Master, Page, Shape, ShapeType, TextPiece};
use flowster_core::resolve::{Lookup, MergedSection, Resolved};
use flowster_core::theme::{EMU_PER_INCH, FillStyle, FontStyle, LineStyle, Slot, StyleSet, Theme};
use flowster_core::transform::Affine;
use flowster_core::value::{Rgba, parse_color, parse_num};
use libm::{atan2, cos, hypot, sin};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub index: usize,
    pub id: u32,
    pub name: String,
    pub background: bool,
    pub back_page: Option<u32>,
    /// Paper size in inches.
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayList {
    pub page: PageInfo,
    pub items: Vec<Item>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stroke {
    pub color: Rgba,
    /// Inches.
    pub width: f64,
    /// Dash lengths in inches; empty for solid.
    pub dash: Vec<f64>,
    pub cap: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub text: String,
    pub font: String,
    /// Inches.
    pub size: f64,
    pub color: Rgba,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Paragraph {
    /// 0 left, 1 centre, 2 right, 3 justify, 4 distributed.
    pub align: u8,
    /// Line spacing: negative = multiple of the font size (Visio `SpLine`), positive = inches.
    pub line_spacing: f64,
    pub space_before: f64,
    pub space_after: f64,
    pub indent_left: f64,
    pub indent_first: f64,
    pub runs: Vec<Run>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Item {
    #[serde(rename_all = "camelCase")]
    Path { shape: u32, d: Vec<Seg>, fill: Option<Rgba>, stroke: Option<Stroke> },
    /// A text block. `transform` maps block-local inches (+Y up, origin at the
    /// block's bottom-left) to page inches.
    #[serde(rename_all = "camelCase")]
    Text {
        shape: u32,
        transform: Affine,
        width: f64,
        height: f64,
        /// Left, right, top, bottom.
        margins: [f64; 4],
        /// 0 top, 1 middle, 2 bottom.
        valign: u8,
        background: Option<Rgba>,
        paragraphs: Vec<Paragraph>,
    },
    /// An image filling `[x, x+w] × [y, y+h]` in shape-local inches mapped by `transform`.
    #[serde(rename_all = "camelCase")]
    Image {
        shape: u32,
        transform: Affine,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        mime: String,
        /// Base64 payload, or `None` when the browser cannot decode the format (EMF/WMF/OLE).
        data: Option<String>,
    },
}

pub fn page_info(doc: &Document, index: usize) -> Option<PageInfo> {
    let p = doc.pages.get(index)?;
    let num = |n: &str| p.sheet.cell(n).and_then(|c| c.v.as_deref()).and_then(parse_num);
    Some(PageInfo {
        index,
        id: p.id,
        name: p.name.clone().or_else(|| p.name_u.clone()).unwrap_or_else(|| format!("Page-{}", index + 1)),
        background: p.background,
        back_page: p.back_page,
        width: num("PageWidth").unwrap_or(8.5),
        height: num("PageHeight").unwrap_or(11.0),
    })
}

pub fn pages(doc: &Document) -> Vec<PageInfo> {
    (0..doc.pages.len()).filter_map(|i| page_info(doc, i)).collect()
}

/// Builds the display list of page `index`, its background pages first.
pub fn build_page(doc: &Document, index: usize) -> Option<DisplayList> {
    let info = page_info(doc, index)?;
    let mut b =
        Builder { doc, items: Vec::new(), warnings: Vec::new(), layers: Vec::new(), variation: (0, 0), calc: None };
    let mut chain: Vec<&Page> = Vec::new();
    let mut next = doc.pages[index].back_page;
    while let Some(id) = next {
        match doc.page(id) {
            Some(bp) if chain.len() < 8 && !chain.iter().any(|p| p.id == id) => {
                chain.push(bp);
                next = bp.back_page;
            }
            _ => break,
        }
    }
    for bp in chain.iter().rev() {
        b.page(bp);
    }
    b.page(&doc.pages[index]);
    let mut warnings = doc.warnings.clone();
    warnings.append(&mut b.warnings);
    Some(DisplayList { page: info, items: b.items, warnings })
}

struct Builder<'d> {
    doc: &'d Document,
    items: Vec<Item>,
    warnings: Vec<String>,
    /// Visibility of the current page's layers by row index.
    layers: Vec<bool>,
    /// The current page's theme variation (colour, style).
    variation: (usize, usize),
    /// Formula engine for the current page: computes cells the file has no cached value for.
    calc: Option<Calc<'d>>,
}

/// Evaluates `THEMEVAL()` cells for one shape (SPEC §11).
struct Themer<'t> {
    theme: &'t Theme,
    set: &'t StyleSet,
    variation: (usize, usize),
}

impl Themer<'_> {
    fn row(&self, r: &Resolved<'_>, matrix_cell: &str, slot: Slot) -> usize {
        self.theme.matrix_row(r.num(matrix_cell).unwrap_or(100.0), slot, self.variation.1)
    }
    fn placeholder(&self, r: &Resolved<'_>, color_cell: &str) -> Option<Rgba> {
        self.theme.quick_style_color(r.num(color_cell).unwrap_or(100.0), self.variation.0)
    }
    fn fill(&self, r: &Resolved<'_>) -> Option<&FillStyle> {
        self.set.fills.get(self.row(r, "QuickStyleFillMatrix", Slot::Fill))
    }
    fn line(&self, r: &Resolved<'_>) -> Option<&LineStyle> {
        self.set.lines.get(self.row(r, "QuickStyleLineMatrix", Slot::Line))
    }
    fn font(&self, r: &Resolved<'_>) -> Option<&FontStyle> {
        self.set.fonts.get(self.row(r, "QuickStyleFontMatrix", Slot::Font))
    }
    fn fill_color(&self, r: &Resolved<'_>) -> Option<Rgba> {
        let ph = self.placeholder(r, "QuickStyleFillColor");
        match self.fill(r).and_then(|f| f.color.as_ref()) {
            Some(spec) => Some(self.theme.resolve(spec, ph)),
            None => ph,
        }
    }
    fn line_color(&self, r: &Resolved<'_>) -> Option<Rgba> {
        let ph = self.placeholder(r, "QuickStyleLineColor");
        match self.line(r).and_then(|l| l.color.as_ref()) {
            Some(spec) => Some(self.theme.resolve(spec, ph)),
            None => ph,
        }
    }
    fn font_color(&self, r: &Resolved<'_>) -> Option<Rgba> {
        let ph = self.placeholder(r, "QuickStyleFontColor");
        match self.font(r).and_then(|f| f.color.as_ref()) {
            Some(spec) => Some(self.theme.resolve(spec, ph)),
            None => ph.or(Some(Rgba::BLACK)),
        }
    }
}

const DEFAULT_LINE_WEIGHT: f64 = 0.01;
const DEFAULT_FONT_SIZE: f64 = 1.0 / 6.0;
const DEFAULT_FONT: &str = "Calibri";
const DEFAULT_MARGIN: f64 = 4.0 / 72.0;

impl<'d> Builder<'d> {
    fn page(&mut self, page: &'d Page) {
        let num = |n: &str| page.sheet.cell(n).and_then(|c| c.v.as_deref()).and_then(parse_num);
        // Shapes are in drawing units; paper is in page units.
        let scale = match (num("PageScale"), num("DrawingScale")) {
            (Some(p), Some(d)) if d.abs() > 1e-12 && p.abs() > 1e-12 => p / d,
            _ => 1.0,
        };
        self.layers = page
            .sheet
            .section("Layer", None)
            .map(|s| {
                let mut v = Vec::new();
                for (pos, r) in s.rows.iter().enumerate() {
                    let ix = r.ix.unwrap_or(pos as u32) as usize;
                    if v.len() <= ix {
                        v.resize(ix + 1, true);
                    }
                    v[ix] = r.cells.get("Visible").and_then(|c| c.v.as_deref()).and_then(parse_num) != Some(0.0);
                }
                v
            })
            .unwrap_or_default();
        // 65534 means "not set" for the variation indices.
        let index = |n: &str| num(n).filter(|v| *v >= 0.0 && *v < 65534.0).unwrap_or(0.0) as usize;
        self.variation = (index("VariationColorIndex"), index("VariationStyleIndex"));
        self.calc = Some(Calc::for_page(self.doc, page));
        let root = Affine::scale(scale, scale);
        for s in &page.shapes {
            self.shape(s, root, None);
        }
    }

    fn on_visible_layer(&self, r: &Resolved<'_>) -> bool {
        let Some(members) = r.v("LayerMember") else { return true };
        let ids: Vec<usize> = members.split(';').filter_map(|s| s.trim().parse().ok()).collect();
        ids.is_empty() || ids.iter().any(|&i| self.layers.get(i).copied().unwrap_or(true))
    }

    fn shape(&mut self, shape: &'d Shape, parent: Affine, parent_master: Option<&'d Master>) {
        if shape.del || shape.kind == ShapeType::Guide {
            return;
        }
        let r = Resolved::new(self.doc, shape, parent_master);
        if !self.on_visible_layer(&r) {
            return;
        }
        let cell = |name: &str| self.cell_num(&r, name);
        let w = cell("Width").unwrap_or(0.0);
        let h = cell("Height").unwrap_or(0.0);
        let local = Affine::shape_local_to_parent(
            (cell("PinX").unwrap_or(0.0), cell("PinY").unwrap_or(0.0)),
            (cell("LocPinX").unwrap_or(w / 2.0), cell("LocPinY").unwrap_or(h / 2.0)),
            cell("Angle").unwrap_or(0.0),
            cell("FlipX").is_some_and(|v| v != 0.0),
            cell("FlipY").is_some_and(|v| v != 0.0),
        );
        let world = parent.then(local);

        if shape.kind == ShapeType::Foreign || r.foreign().is_some() {
            self.image(&r, world, w, h);
        }
        self.geometry(&r, world, w, h);
        let master = r.master();
        for child in r.children() {
            self.shape(child, world, master);
        }
        self.text(&r, world, w, h);
    }

    /// The page's calculator, if it knows this exact shape.
    fn calc_for(&self, r: &Resolved<'_>) -> Option<&Calc<'d>> {
        self.calc.as_ref().filter(|c| c.owns(r.shape().id, r.shape()))
    }

    /// A numeric single-row cell: the cached value, else computed from its formula.
    fn cell_num(&self, r: &Resolved<'_>, name: &str) -> Option<f64> {
        if let Some(n) = r.num(name) {
            return Some(n);
        }
        let def = r.cell_def(name)?;
        let f = def.f?;
        match self.calc_for(r)?.eval_in(r.shape().id, f) {
            Value::Num(n) => Some(n),
            Value::Bool(b) => Some(if b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    /// Fills geometry cells that have a formula but no cached value.
    fn compute_missing(&self, r: &Resolved<'_>, sections: &mut [MergedSection]) {
        let Some(calc) = self.calc_for(r) else { return };
        for sec in sections {
            for row in &mut sec.rows {
                let missing: Vec<(String, String)> = row
                    .f
                    .iter()
                    .filter(|(k, _)| !row.v.contains_key(*k))
                    .map(|(k, f)| (k.clone(), f.clone()))
                    .collect();
                for (k, f) in missing {
                    if let Value::Num(n) = calc.eval_in(r.shape().id, &f) {
                        row.v.insert(k, n.to_string());
                    }
                }
            }
        }
    }

    fn themer(&self, r: &Resolved<'_>) -> Option<Themer<'d>> {
        let theme = self.doc.theme.as_ref()?;
        // QuickStyleType 2 = connector; 0 (auto) treats 1-D shapes as connectors.
        let connector = match r.num("QuickStyleType").unwrap_or(0.0) as i64 {
            2 => true,
            0 => r.v("BeginX").is_some(),
            _ => false,
        };
        let set = if connector && !theme.connectors.lines.is_empty() { &theme.connectors } else { &theme.shapes };
        Some(Themer { theme, set, variation: self.variation })
    }

    /// A colour cell, evaluating `THEMEVAL()` with `themed`.
    fn color(&self, r: &Resolved<'_>, cell: &str, themed: impl Fn(&Themer<'d>) -> Option<Rgba>, default: Rgba) -> Rgba {
        match r.lookup(cell) {
            Lookup::Value(v) => parse_color(v, &self.doc.colors).unwrap_or(default),
            Lookup::Themed => self.themer(r).and_then(|t| themed(&t)).unwrap_or(default),
            Lookup::Missing => default,
        }
    }

    /// A numeric cell, evaluating `THEMEVAL()` with `themed`.
    fn num(&self, r: &Resolved<'_>, cell: &str, themed: impl Fn(&Themer<'d>) -> Option<f64>, default: f64) -> f64 {
        match r.lookup(cell) {
            Lookup::Value(v) => parse_num(v).unwrap_or(default),
            Lookup::Themed => self.themer(r).and_then(|t| themed(&t)).unwrap_or(default),
            Lookup::Missing => default,
        }
    }

    fn geometry(&mut self, r: &Resolved<'_>, world: Affine, w: f64, h: f64) {
        let mut sections = r.sections("Geometry");
        if sections.is_empty() {
            return;
        }
        self.compute_missing(r, &mut sections);
        let paths = sections_to_paths(&sections, w, h);
        let rounding =
            self.num(r, "Rounding", |t| t.line(r).and_then(|l| l.rounding_emu).map(|e| e / EMU_PER_INCH), 0.0);
        let mut fill_segs: Vec<Seg> = Vec::new();
        let mut stroke_segs: Vec<Seg> = Vec::new();
        for p in paths.iter().filter(|p| !p.no_show) {
            let segs = if rounding > 0.0 { round_corners(&p.segs, rounding) } else { p.segs.clone() };
            if p.closed && !p.no_fill {
                fill_segs.extend(segs.iter().cloned());
            }
            if !p.no_line {
                stroke_segs.extend(segs);
            }
        }

        let fill_pattern = self.num(r, "FillPattern", |t| Some(t.fill(r).and_then(|f| f.pattern).unwrap_or(1.0)), 1.0);
        let fill = if fill_pattern != 0.0 && !fill_segs.is_empty() {
            let color = self.color(r, "FillForegnd", |t| t.fill_color(r), Rgba::WHITE);
            Some(color.with_transparency(self.num(r, "FillForegndTrans", |_| None, 0.0)))
        } else {
            None
        };
        let line_pattern = self.num(r, "LinePattern", |t| Some(t.line(r).and_then(|l| l.pattern).unwrap_or(1.0)), 1.0);
        let weight = self
            .num(
                r,
                "LineWeight",
                |t| t.line(r).and_then(|l| l.width_emu).map(|e| e / EMU_PER_INCH),
                DEFAULT_LINE_WEIGHT,
            )
            .max(0.0)
            * world.mean_scale();
        let stroke = if line_pattern != 0.0 && !stroke_segs.is_empty() {
            let color = self.color(r, "LineColor", |t| t.line_color(r), Rgba::BLACK);
            Some(Stroke {
                color: color.with_transparency(self.num(r, "LineColorTrans", |_| None, 0.0)),
                width: weight,
                dash: dash_pattern(line_pattern, weight),
                cap: match self.num(r, "LineCap", |_| None, 0.0) as i64 {
                    1 => "square",
                    2 => "butt",
                    _ => "round",
                },
            })
        } else {
            None
        };

        let shape = r.shape().id;
        let to_page = |segs: &[Seg]| segs.iter().map(|s| s.map(|p| world.apply(p))).collect::<Vec<_>>();
        // Fill and outline coincide for ordinary closed shapes: one item does both.
        if fill.is_some() && stroke.is_some() && fill_segs == stroke_segs {
            self.items.push(Item::Path { shape, d: to_page(&fill_segs), fill, stroke });
            return;
        }
        if let Some(fill) = fill {
            self.items.push(Item::Path { shape, d: to_page(&fill_segs), fill: Some(fill), stroke: None });
        }
        if let Some(stroke) = stroke {
            let mut d = to_page(&stroke_segs);
            let arrows = self.arrows(r, &mut d, &stroke);
            self.items.push(Item::Path { shape, d, fill: None, stroke: Some(stroke) });
            self.items.extend(arrows);
        }
    }

    /// Arrowheads at the open ends of a stroked path (`BeginArrow`, `EndArrow`).
    fn arrows(&self, r: &Resolved<'_>, d: &mut [Seg], stroke: &Stroke) -> Vec<Item> {
        let mut out = Vec::new();
        if d.iter().any(|s| matches!(s, Seg::Z)) {
            return out;
        }
        let kinds = [
            (
                self.num(r, "BeginArrow", |t| t.line(r).and_then(|l| l.begin_arrow), 0.0) as i64,
                self.num(r, "BeginArrowSize", |t| t.line(r).and_then(|l| l.begin_arrow_size), 2.0),
                true,
            ),
            (
                self.num(r, "EndArrow", |t| t.line(r).and_then(|l| l.end_arrow), 0.0) as i64,
                self.num(r, "EndArrowSize", |t| t.line(r).and_then(|l| l.end_arrow_size), 2.0),
                false,
            ),
        ];
        for (kind, size, at_begin) in kinds {
            if kind <= 0 || d.len() < 2 {
                continue;
            }
            let (tip, from, idx) = if at_begin {
                let tip = d[0].end().unwrap_or_default();
                let from = match d[1] {
                    Seg::C { x1, y1, .. } => (x1, y1),
                    other => other.end().unwrap_or(tip),
                };
                (tip, from, 0)
            } else {
                let n = d.len() - 1;
                let tip = d[n].end().unwrap_or_default();
                let from = match d[n] {
                    Seg::C { x2, y2, .. } => (x2, y2),
                    _ => d[n - 1].end().unwrap_or(tip),
                };
                (tip, from, n)
            };
            let len_dir = hypot(tip.0 - from.0, tip.1 - from.1);
            if len_dir < 1e-9 {
                continue;
            }
            let scale = [0.5, 0.75, 1.0, 1.4, 1.9, 2.6, 4.0][size.clamp(0.0, 6.0) as usize];
            let len = scale * (0.06 + 5.0 * stroke.width);
            let half = len * 0.33;
            let ang = atan2(tip.1 - from.1, tip.0 - from.0);
            let (c, s) = (cos(ang), sin(ang));
            let base = (tip.0 - c * len, tip.1 - s * len);
            let left = (base.0 - s * half, base.1 + c * half);
            let right = (base.0 + s * half, base.1 - c * half);
            let open = kind == 1;
            if open {
                out.push(Item::Path {
                    shape: r.shape().id,
                    d: vec![
                        Seg::M { x: left.0, y: left.1 },
                        Seg::L { x: tip.0, y: tip.1 },
                        Seg::L { x: right.0, y: right.1 },
                    ],
                    fill: None,
                    stroke: Some(Stroke { dash: Vec::new(), ..stroke.clone() }),
                });
            } else {
                // Stop the line inside the head so thick lines do not poke past the tip.
                let back = (tip.0 - c * len * 0.8, tip.1 - s * len * 0.8);
                d[idx] = match d[idx] {
                    Seg::M { .. } => Seg::M { x: back.0, y: back.1 },
                    Seg::L { .. } => Seg::L { x: back.0, y: back.1 },
                    Seg::C { x1, y1, x2, y2, .. } => Seg::C { x1, y1, x2, y2, x: back.0, y: back.1 },
                    Seg::Z => Seg::Z,
                };
                out.push(Item::Path {
                    shape: r.shape().id,
                    d: vec![
                        Seg::M { x: tip.0, y: tip.1 },
                        Seg::L { x: left.0, y: left.1 },
                        Seg::L { x: right.0, y: right.1 },
                        Seg::Z,
                    ],
                    fill: Some(stroke.color),
                    stroke: None,
                });
            }
        }
        out
    }

    fn image(&mut self, r: &Resolved<'_>, world: Affine, w: f64, h: f64) {
        let Some(fd) = r.foreign() else { return };
        let media = fd.part.as_deref().and_then(|p| self.doc.media.get(p));
        let mime = media.map(|m| m.content_type.clone()).unwrap_or_else(|| "application/octet-stream".into());
        let displayable =
            matches!(mime.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/bmp" | "image/svg+xml");
        self.items.push(Item::Image {
            shape: r.shape().id,
            transform: world,
            x: r.num("ImgOffsetX").unwrap_or(0.0),
            y: r.num("ImgOffsetY").unwrap_or(0.0),
            w: r.num("ImgWidth").unwrap_or(w),
            h: r.num("ImgHeight").unwrap_or(h),
            data: media.filter(|_| displayable).map(|m| base64::engine::general_purpose::STANDARD.encode(&m.bytes)),
            mime,
        });
    }

    fn text(&mut self, r: &Resolved<'_>, world: Affine, w: f64, h: f64) {
        let Some(pieces) = r.text() else { return };
        if r.flag("HideText").unwrap_or(false) {
            return;
        }
        let chars = r.sections("Character");
        let paras = r.sections("Paragraph");
        let themer = self.themer(r);
        let font = |ix: u32| -> Run {
            let v = |cell: &str| r.text_row(&chars, "Character", ix, cell);
            let num = |cell: &str| match v(cell) {
                Lookup::Value(s) => parse_num(s),
                _ => None,
            };
            let style = match v("Style") {
                Lookup::Value(s) => parse_num(s).unwrap_or(0.0),
                Lookup::Themed => themer.as_ref().and_then(|t| t.font(r)).and_then(|f| f.style).unwrap_or(0.0),
                Lookup::Missing => 0.0,
            } as i64;
            let font = match v("Font") {
                Lookup::Value(f) if parse_num(f).is_none() && !f.trim().is_empty() => f.to_owned(),
                Lookup::Themed => {
                    themer.as_ref().and_then(|t| t.theme.minor_font.clone()).unwrap_or_else(|| DEFAULT_FONT.into())
                }
                _ => DEFAULT_FONT.to_owned(),
            };
            let color = match v("Color") {
                Lookup::Value(c) => parse_color(c, &self.doc.colors).unwrap_or(Rgba::BLACK),
                Lookup::Themed => themer.as_ref().and_then(|t| t.font_color(r)).unwrap_or(Rgba::BLACK),
                Lookup::Missing => Rgba::BLACK,
            };
            Run {
                text: String::new(),
                font,
                size: num("Size").filter(|s| *s > 0.0).unwrap_or(DEFAULT_FONT_SIZE),
                color: color.with_transparency(num("ColorTrans").unwrap_or(0.0)),
                bold: style & 1 != 0,
                italic: style & 2 != 0,
                underline: style & 4 != 0,
                strike: num("Strikethru").unwrap_or(0.0) != 0.0,
            }
        };
        let para = |ix: u32| -> Paragraph {
            let v = |cell: &str| match r.text_row(&paras, "Paragraph", ix, cell) {
                Lookup::Value(s) => parse_num(s),
                _ => None,
            };
            Paragraph {
                align: v("HorzAlign").unwrap_or(1.0).clamp(0.0, 4.0) as u8,
                line_spacing: v("SpLine").unwrap_or(-1.2),
                space_before: v("SpBefore").unwrap_or(0.0),
                space_after: v("SpAfter").unwrap_or(0.0),
                indent_left: v("IndLeft").unwrap_or(0.0),
                indent_first: v("IndFirst").unwrap_or(0.0),
                runs: Vec::new(),
            }
        };

        let mut paragraphs: Vec<Paragraph> = Vec::new();
        let (mut cp, mut pp) = (0u32, 0u32);
        let mut current = para(0);
        let mut pending_para: Option<u32> = None;
        for piece in pieces {
            match piece {
                TextPiece::Cp(ix) => cp = *ix,
                TextPiece::Pp(ix) => {
                    pp = *ix;
                    if current.runs.is_empty() {
                        current = para(pp);
                    } else {
                        pending_para = Some(pp);
                    }
                }
                TextPiece::Tp(_) | TextPiece::Fld(_) => {}
                TextPiece::Text(t) => {
                    let mut parts = t.split(['\n', '\u{2029}']).peekable();
                    while let Some(part) = parts.next() {
                        let part = part.replace(['\r', '\u{FFFC}'], "").replace('\u{2028}', "\n");
                        if !part.is_empty() {
                            let mut run = font(cp);
                            run.text = part;
                            current.runs.push(run);
                        }
                        if parts.peek().is_some() {
                            let next = para(pending_para.take().unwrap_or(pp));
                            paragraphs.push(std::mem::replace(&mut current, next));
                        }
                    }
                }
            }
        }
        if !current.runs.is_empty() {
            paragraphs.push(current);
        }
        // Visio text ends with a paragraph mark; empty trailing paragraphs draw nothing.
        while paragraphs.last().is_some_and(|p| p.runs.is_empty()) {
            paragraphs.pop();
        }
        if paragraphs.iter().all(|p| p.runs.iter().all(|r| r.text.trim().is_empty())) {
            return;
        }

        let tw = r.num("TxtWidth").unwrap_or(w);
        let th = r.num("TxtHeight").unwrap_or(h);
        let block = world
            .then(Affine::translate(r.num("TxtPinX").unwrap_or(w / 2.0), r.num("TxtPinY").unwrap_or(h / 2.0)))
            .then(Affine::rotate(r.num("TxtAngle").unwrap_or(0.0)))
            .then(Affine::translate(
                -r.num("TxtLocPinX").unwrap_or(tw / 2.0),
                -r.num("TxtLocPinY").unwrap_or(th / 2.0),
            ));
        let background = match r.v("TextBkgnd").map(str::trim) {
            None | Some("0") => None,
            Some(v) => {
                parse_color(v, &self.doc.colors).map(|c| c.with_transparency(r.num("TextBkgndTrans").unwrap_or(0.0)))
            }
        };
        self.items.push(Item::Text {
            shape: r.shape().id,
            transform: block,
            width: tw,
            height: th,
            margins: [
                r.num("LeftMargin").unwrap_or(DEFAULT_MARGIN),
                r.num("RightMargin").unwrap_or(DEFAULT_MARGIN),
                r.num("TopMargin").unwrap_or(DEFAULT_MARGIN),
                r.num("BottomMargin").unwrap_or(DEFAULT_MARGIN),
            ],
            valign: r.num("VerticalAlign").unwrap_or(1.0).clamp(0.0, 2.0) as u8,
            background,
            paragraphs,
        });
    }
}

/// Dash lengths for the built-in line patterns, in multiples of the line weight
/// (with a floor so hairlines still show their pattern).
fn dash_pattern(pattern: f64, weight: f64) -> Vec<f64> {
    let unit = weight.max(0.01);
    let units: &[f64] = match pattern as i64 {
        0 | 1 => &[],
        2 => &[6.0, 4.0],
        3 => &[1.0, 3.0],
        4 => &[6.0, 3.0, 1.0, 3.0],
        5 => &[6.0, 3.0, 1.0, 3.0, 1.0, 3.0],
        6 => &[12.0, 4.0],
        7 => &[12.0, 4.0, 6.0, 4.0],
        8 => &[12.0, 4.0, 6.0, 4.0, 6.0, 4.0],
        9 => &[3.0, 2.0],
        10 => &[1.0, 2.0],
        11 => &[3.0, 2.0, 1.0, 2.0],
        12 => &[3.0, 2.0, 1.0, 2.0, 1.0, 2.0],
        13 => &[6.0, 2.0],
        14 => &[6.0, 2.0, 3.0, 2.0],
        15 => &[6.0, 2.0, 3.0, 2.0, 3.0, 2.0],
        16 => &[24.0, 8.0],
        17 => &[24.0, 8.0, 12.0, 8.0],
        18 => &[24.0, 8.0, 12.0, 8.0, 12.0, 8.0],
        19 => &[1.0, 1.0],
        20 => &[1.0, 6.0],
        21 => &[1.0, 1.0, 1.0, 6.0],
        22 => &[12.0, 2.0, 1.0, 2.0],
        23 => &[2.0, 2.0],
        _ => &[6.0, 4.0],
    };
    units.iter().map(|u| u * unit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_fixture_renders_a_box_with_text() {
        let pkg = flowster_vsdx::OpcPackage::open(flowster_vsdx::fixture::minimal_vsdx()).unwrap();
        let doc = flowster_vsdx::visio::read_document(&pkg).unwrap();
        let dl = build_page(&doc, 0).unwrap();
        assert_eq!((dl.page.width, dl.page.height), (8.5, 11.0));
        let Item::Path { d, fill, stroke, .. } = &dl.items[0] else { panic!("{:?}", dl.items) };
        assert!(fill.is_some() && stroke.is_some());
        // Box of 2 × 1 centred on (4.25, 5.5).
        assert_eq!(d[0], Seg::M { x: 3.25, y: 5.0 });
        let Item::Text { paragraphs, .. } = &dl.items[1] else { panic!() };
        assert_eq!(paragraphs[0].runs[0].text, "Hello from Flowster");
    }
}
