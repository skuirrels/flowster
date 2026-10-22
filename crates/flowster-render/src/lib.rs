//! `flowster-render`: turns a page of the document model into a display list
//! (SPEC §8.5): paths in page coordinates (inches, +Y up) with fill and stroke,
//! text blocks with their transform and styled runs, and images.
//!
//! The canvas renderer in the UI, the SVG exporter ([`svg`]) and golden tests
//! all consume the same list. Text is laid out by the consumer for now; core
//! text layout (parley) replaces that later in M1 so `TEXTWIDTH` and wrapping
//! are identical everywhere.

#![forbid(unsafe_code)]

pub mod arrows;
pub mod emf;
pub mod paint;
pub mod svg;
pub mod text_layout;

use base64::Engine as _;
use flowster_core::calc::Calc;
use flowster_core::formula::Value;
use flowster_core::geom::{Seg, round_corners, sections_to_paths};
use flowster_core::model::{Document, Master, Page, Shape, ShapeType, TextPiece};
use flowster_core::resolve::{Lookup, MergedSection, Resolved};
use flowster_core::theme::{EMU_PER_INCH, FillStyle, FontStyle, LineStyle, Slot, StyleSet, Theme};
use flowster_core::transform::Affine;
use flowster_core::value::{Rgba, parse_color, parse_num};
use libm::{cos, hypot, sin};
pub use paint::{Paint, Stop};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub index: usize,
    pub id: u32,
    pub name: String,
    pub background: bool,
    pub back_page: Option<u32>,
    /// Hidden from the page tabs.
    pub hidden: bool,
    /// Paper size in inches.
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayList {
    pub page: PageInfo,
    pub items: Vec<Item>,
    /// The page's top-level shapes, back to front.
    pub shapes: Vec<ShapeBox>,
    pub warnings: Vec<String>,
}

/// Where a top-level shape is, for selecting, moving and resizing it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeBox {
    pub id: u32,
    /// Shape-local inches (origin bottom-left, +Y up) → page inches.
    pub transform: Affine,
    pub width: f64,
    pub height: f64,
    /// Page positions of a 1-D shape's ends.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub begin: Option<(f64, f64)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<(f64, f64)>,
    /// The shape's text, for editing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Shapes a connector's ends are glued to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub begin_glue: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_glue: Option<u32>,
    /// A dynamic connector (`ObjType` 2).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub connector: bool,
    /// A group (it can be ungrouped).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub group: bool,
    /// The `Angle` cell, radians counter-clockwise.
    #[serde(skip_serializing_if = "is_zero")]
    pub angle: f64,
    /// How the shape looks now (resolved through styles, masters and theme).
    pub format: ShapeFormat,
}

/// A shape's resolved formatting, for the format controls.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeFormat {
    /// Representative fill colour; `None` when the shape has no fill.
    pub fill: Option<Rgba>,
    /// Line colour; `None` when the shape has no line.
    pub line: Option<Rgba>,
    /// Points.
    pub line_weight: f64,
    pub line_pattern: u32,
    /// Corner rounding, inches.
    pub rounding: f64,
    pub font: String,
    /// Points.
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub text_color: Rgba,
    /// 0 left, 1 centre, 2 right, 3 justify.
    pub align: u8,
    /// 0 top, 1 middle, 2 bottom.
    pub valign: u8,
    /// Visio arrowhead kinds at the line's begin and end (0 = none).
    pub begin_arrow: u32,
    pub end_arrow: u32,
}

impl DisplayList {
    /// The drawing's extents in page inches, `[x0, y0, x1, y1]` (y up), which may
    /// reach past the page: path control points and half the stroke width, text
    /// blocks and image frames. `None` for an empty page.
    pub fn extents(&self) -> Option<[f64; 4]> {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        let mut add = |(x, y): (f64, f64), pad: f64| {
            b = [b[0].min(x - pad), b[1].min(y - pad), b[2].max(x + pad), b[3].max(y + pad)];
        };
        for item in &self.items {
            match item {
                Item::Path { d, stroke, blur, .. } => {
                    let pad = stroke.as_ref().map_or(0.0, |s| s.width / 2.0) + blur;
                    for s in d {
                        if let Seg::C { x1, y1, x2, y2, .. } = *s {
                            add((x1, y1), pad);
                            add((x2, y2), pad);
                        }
                        if let Some(p) = s.end() {
                            add(p, pad);
                        }
                    }
                }
                Item::Text { transform, width, height, .. } => {
                    for p in [(0.0, 0.0), (*width, 0.0), (0.0, *height), (*width, *height)] {
                        add(transform.apply(p), 0.0);
                    }
                }
                Item::Image { transform, x, y, w, h, .. } => {
                    for p in [(*x, *y), (x + w, *y), (*x, y + h), (x + w, y + h)] {
                        add(transform.apply(p), 0.0);
                    }
                }
            }
        }
        (b[0] <= b[2]).then_some(b)
    }
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
    /// The bullet drawn at the first line's indent (`Bullet` / `BulletStr`), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bullet: Option<String>,
    /// Where the first line's text starts after a bullet (`TextPosAfterBullet`,
    /// inches from the bullet); 0: at the left indent, or just past the bullet.
    #[serde(skip_serializing_if = "is_zero")]
    pub text_after_bullet: f64,
    pub runs: Vec<Run>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Item {
    #[serde(rename_all = "camelCase")]
    Path {
        shape: u32,
        d: Vec<Seg>,
        fill: Option<Paint>,
        stroke: Option<Stroke>,
        /// Gaussian blur radius in inches (shadows); 0 for none.
        #[serde(skip_serializing_if = "is_zero", default)]
        blur: f64,
    },
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

fn is_zero(v: &f64) -> bool {
    *v == 0.0
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
        hidden: p.hidden(),
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
    let mut b = Builder {
        doc,
        items: Vec::new(),
        warnings: Vec::new(),
        layers: Vec::new(),
        variation: (0, 0),
        calc: None,
        page_shadow: (0.0, 0.0),
        boxes: None,
        depth: 0,
        theme: None,
    };
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
    // Only the page's own shapes can be selected, not its backgrounds'.
    b.boxes = Some(Vec::new());
    b.page(&doc.pages[index]);
    let mut warnings = doc.warnings.clone();
    warnings.append(&mut b.warnings);
    Some(DisplayList { page: info, items: b.items, shapes: b.boxes.unwrap_or_default(), warnings })
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
    /// The page's default shadow offset (`ShdwOffsetX/Y` in the page sheet).
    page_shadow: (f64, f64),
    /// Selection boxes of the page's top-level shapes (`None` on background pages).
    boxes: Option<Vec<ShapeBox>>,
    /// Group nesting of the shape being built (0 = on the page).
    depth: usize,
    /// The theme of the page or group being built (`None`: no theme).
    theme: Option<&'d flowster_core::theme::Theme>,
}

/// A resolved drop shadow, in page inches.
struct Shadow {
    dx: f64,
    dy: f64,
    color: Rgba,
    blur: f64,
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
    fn effect(&self, r: &Resolved<'_>) -> Option<&flowster_core::theme::ShadowStyle> {
        self.set.effects.get(self.row(r, "QuickStyleEffectsMatrix", Slot::Effect))?.as_ref()
    }
    fn gradient_stops(&self, r: &Resolved<'_>) -> Vec<Stop> {
        let ph = self.placeholder(r, "QuickStyleFillColor");
        self.fill(r)
            .map(|f| {
                f.gradient
                    .iter()
                    .map(|(pos, spec)| Stop { offset: *pos, color: self.theme.resolve(spec, ph) })
                    .collect()
            })
            .unwrap_or_default()
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
        // The page's theme: what its shapes use unless they (or their group) pick
        // another. A page with no ThemeIndex has none, even when the file has
        // theme parts (some shapes may still use them).
        self.theme = num("ThemeIndex")
            .map(|v| v as u32)
            .filter(|&v| v != flowster_core::model::THEME_INDEX_UNSET)
            .and_then(|v| self.doc.theme_by_index(v));
        self.calc = Some(Calc::for_page(self.doc, page));
        self.page_shadow = (num("ShdwOffsetX").unwrap_or(0.125), num("ShdwOffsetY").unwrap_or(-0.125));
        let root = Affine::scale(scale, scale);
        for s in &page.shapes {
            self.shape(s, root, None);
        }
    }

    fn on_visible_layer(&self, r: &Resolved<'d>) -> bool {
        let Some(members) = r.v("LayerMember") else { return true };
        let ids: Vec<usize> = members.split(';').filter_map(|s| s.trim().parse().ok()).collect();
        ids.is_empty() || ids.iter().any(|&i| self.layers.get(i).copied().unwrap_or(true))
    }

    fn shape(&mut self, shape: &'d Shape, parent: Affine, parent_master: Option<&'d Master>) {
        if shape.del || shape.kind == ShapeType::Guide {
            return;
        }
        // The shape's own theme, else its group's (or page's).
        let theme = match Resolved::new(self.doc, shape, parent_master).own_theme_index() {
            Some(ix) => self.doc.theme_by_index(ix),
            None => self.theme,
        };
        let r = Resolved::new(self.doc, shape, parent_master).with_theme(theme);
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

        if self.depth == 0 && self.boxes.is_some() {
            let end = |x: &str, y: &str| Some(parent.apply((cell(x)?, cell(y)?)));
            let text = r.text().map(|pieces| {
                pieces.iter().filter_map(|p| if let TextPiece::Text(t) = p { Some(t.as_str()) } else { None }).collect()
            });
            let format = self.format_of(&r, world, w, h);
            let b = ShapeBox {
                id: shape.id,
                transform: world,
                width: w,
                height: h,
                begin: end("BeginX", "BeginY"),
                end: end("EndX", "EndY"),
                text,
                begin_glue: flowster_core::library::glue_target(&shape.sheet, "BegTrigger"),
                end_glue: flowster_core::library::glue_target(&shape.sheet, "EndTrigger"),
                connector: r.num("ObjType") == Some(2.0),
                group: shape.kind == ShapeType::Group,
                angle: r.num("Angle").unwrap_or(0.0),
                format,
            };
            if let Some(boxes) = &mut self.boxes {
                boxes.push(b);
            }
        }
        if shape.kind == ShapeType::Foreign || r.foreign().is_some() {
            self.image(&r, world, w, h);
        }
        self.geometry(&r, world, w, h);
        let master = r.master();
        self.depth += 1;
        let outer = std::mem::replace(&mut self.theme, theme);
        for child in r.children() {
            self.shape(child, world, master);
        }
        self.theme = outer;
        self.depth -= 1;
        self.text(&r, world, w, h);
    }

    /// The page's calculator, if it knows this exact shape.
    fn calc_for(&self, r: &Resolved<'d>) -> Option<&Calc<'d>> {
        self.calc.as_ref().filter(|c| c.owns(r.shape().id, r.shape()))
    }

    /// A numeric single-row cell: the cached value, else computed from its formula.
    fn cell_num(&self, r: &Resolved<'d>, name: &str) -> Option<f64> {
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
    fn compute_missing(&self, r: &Resolved<'d>, sections: &mut [MergedSection]) {
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

    fn themer(&self, r: &Resolved<'d>) -> Option<Themer<'d>> {
        let theme = r.theme()?;
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
    fn color(&self, r: &Resolved<'d>, cell: &str, themed: impl Fn(&Themer<'d>) -> Option<Rgba>, default: Rgba) -> Rgba {
        match r.lookup(cell) {
            Lookup::Value(v) => parse_color(v, &self.doc.colors).unwrap_or(default),
            Lookup::Themed => self.themer(r).and_then(|t| themed(&t)).unwrap_or(default),
            Lookup::Missing => default,
        }
    }

    /// A numeric cell, evaluating `THEMEVAL()` with `themed`.
    fn num(&self, r: &Resolved<'d>, cell: &str, themed: impl Fn(&Themer<'d>) -> Option<f64>, default: f64) -> f64 {
        match r.lookup(cell) {
            Lookup::Value(v) => parse_num(v).unwrap_or(default),
            Lookup::Themed => self.themer(r).and_then(|t| themed(&t)).unwrap_or(default),
            Lookup::Missing => default,
        }
    }

    fn geometry(&mut self, r: &Resolved<'d>, world: Affine, w: f64, h: f64) {
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

        let fill = if fill_segs.is_empty() { None } else { self.fill_paint(r, world, w, h) };
        let (line_pattern, weight) = self.line_pattern_weight(r);
        let weight = weight * world.mean_scale();
        let stroke = if line_pattern != 0.0 && !stroke_segs.is_empty() {
            let color = self.color(r, "LineColor", |t| t.line_color(r), Rgba::BLACK);
            Some(Stroke {
                color: color.with_transparency(self.num(r, "LineColorTrans", |_| None, 0.0)),
                width: weight,
                dash: dash_pattern(line_pattern, weight),
                // 0 round, 1 flat at the end point, 2 square past it. Dashes
                // always have flat ends.
                cap: match self.num(r, "LineCap", |_| None, 0.0) as i64 {
                    _ if line_pattern >= 2.0 => "butt",
                    0 => "round",
                    2 => "square",
                    _ => "butt",
                },
            })
        } else {
            None
        };

        let shape = r.shape().id;
        let to_page = |segs: &[Seg]| segs.iter().map(|s| s.map(|p| world.apply(p))).collect::<Vec<_>>();
        if let Some(sh) = self.shadow(r) {
            // The silhouette, offset on the page (shadows do not rotate with the shape).
            let offset = |segs: &[Seg]| {
                segs.iter()
                    .map(|s| {
                        s.map(|p| {
                            let q = world.apply(p);
                            (q.0 + sh.dx, q.1 + sh.dy)
                        })
                    })
                    .collect::<Vec<_>>()
            };
            if fill.is_some() {
                self.items.push(Item::Path {
                    shape,
                    d: offset(&fill_segs),
                    fill: Some(Paint::Solid { color: sh.color }),
                    stroke: None,
                    blur: sh.blur,
                });
            }
            if let Some(st) = &stroke
                && (fill.is_none() || fill_segs != stroke_segs)
            {
                self.items.push(Item::Path {
                    shape,
                    d: offset(&stroke_segs),
                    fill: None,
                    stroke: Some(Stroke { color: sh.color, ..st.clone() }),
                    blur: sh.blur,
                });
            }
        }
        // Fill and outline coincide for ordinary closed shapes: one item does both.
        if fill.is_some() && stroke.is_some() && fill_segs == stroke_segs {
            self.items.push(Item::Path { shape, d: to_page(&fill_segs), fill, stroke, blur: 0.0 });
            return;
        }
        if let Some(fill) = fill {
            self.items.push(Item::Path { shape, d: to_page(&fill_segs), fill: Some(fill), stroke: None, blur: 0.0 });
        }
        if let Some(stroke) = stroke {
            let mut d = to_page(&stroke_segs);
            let arrows = self.arrows(r, &mut d, &stroke);
            self.items.push(Item::Path { shape, d, fill: None, stroke: Some(stroke), blur: 0.0 });
            self.items.extend(arrows);
        }
    }

    /// The fill of a closed shape: modern gradient, legacy gradient, hatch or solid.
    /// `LinePattern` and `LineWeight` (inches) after styles and theme.
    fn line_pattern_weight(&self, r: &Resolved<'d>) -> (f64, f64) {
        let pattern = self.num(r, "LinePattern", |t| Some(t.line(r).and_then(|l| l.pattern).unwrap_or(1.0)), 1.0);
        let weight = self
            .num(
                r,
                "LineWeight",
                |t| t.line(r).and_then(|l| l.width_emu).map(|e| e / EMU_PER_INCH),
                DEFAULT_LINE_WEIGHT,
            )
            .max(0.0);
        (pattern, weight)
    }

    /// The resolved format of a shape (what the format controls show).
    fn format_of(&self, r: &Resolved<'d>, world: Affine, w: f64, h: f64) -> ShapeFormat {
        let mut sections = r.sections("Geometry");
        self.compute_missing(r, &mut sections);
        let paths = sections_to_paths(&sections, w, h);
        let closed = paths.iter().any(|p| p.closed && !p.no_fill && !p.no_show);
        let lined = paths.iter().any(|p| !p.no_line && !p.no_show);
        let fill = if closed { self.fill_paint(r, world, w, h).map(|p| p.representative()) } else { None };
        let (pattern, weight) = self.line_pattern_weight(r);
        let line = (pattern != 0.0 && lined).then(|| self.color(r, "LineColor", |t| t.line_color(r), Rgba::BLACK));
        let rounding =
            self.num(r, "Rounding", |t| t.line(r).and_then(|l| l.rounding_emu).map(|e| e / EMU_PER_INCH), 0.0);
        let chars = r.sections("Character");
        let paras = r.sections("Paragraph");
        let themer = self.themer(r);
        let run = self.char_run(r, &chars, themer.as_ref(), 0);
        let align = match r.text_row(&paras, "Paragraph", 0, "HorzAlign") {
            Lookup::Value(v) => parse_num(v).unwrap_or(1.0),
            _ => 1.0,
        };
        ShapeFormat {
            fill,
            line,
            line_weight: weight * 72.0,
            line_pattern: pattern.max(0.0) as u32,
            rounding,
            font: run.font,
            size: run.size * 72.0,
            bold: run.bold,
            italic: run.italic,
            underline: run.underline,
            strike: run.strike,
            text_color: run.color,
            align: align.clamp(0.0, 4.0) as u8,
            valign: r.num("VerticalAlign").unwrap_or(1.0).clamp(0.0, 2.0) as u8,
            begin_arrow: self.num(r, "BeginArrow", |t| t.line(r).and_then(|l| l.begin_arrow), 0.0).max(0.0) as u32,
            end_arrow: self.num(r, "EndArrow", |t| t.line(r).and_then(|l| l.end_arrow), 0.0).max(0.0) as u32,
        }
    }

    /// The character format of Character row `ix` (empty text).
    fn char_run(&self, r: &Resolved<'d>, chars: &[MergedSection], themer: Option<&Themer<'d>>, ix: u32) -> Run {
        let v = |cell: &str| r.text_row(chars, "Character", ix, cell);
        let num = |cell: &str| match v(cell) {
            Lookup::Value(s) => parse_num(s),
            _ => None,
        };
        let style = match v("Style") {
            Lookup::Value(s) => parse_num(s).unwrap_or(0.0),
            Lookup::Themed => themer.and_then(|t| t.font(r)).and_then(|f| f.style).unwrap_or(0.0),
            Lookup::Missing => 0.0,
        } as i64;
        let font = match v("Font") {
            Lookup::Value(f) if parse_num(f).is_none() && !f.trim().is_empty() => f.to_owned(),
            Lookup::Themed => themer.and_then(|t| t.theme.minor_font.clone()).unwrap_or_else(|| DEFAULT_FONT.into()),
            _ => DEFAULT_FONT.to_owned(),
        };
        let color = match v("Color") {
            Lookup::Value(c) => parse_color(c, &self.doc.colors).unwrap_or(Rgba::BLACK),
            Lookup::Themed => themer.and_then(|t| t.font_color(r)).unwrap_or(Rgba::BLACK),
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
    }

    fn fill_paint(&self, r: &Resolved<'d>, world: Affine, w: f64, h: f64) -> Option<Paint> {
        let pattern =
            self.num(r, "FillPattern", |t| Some(t.fill(r).and_then(|f| f.pattern).unwrap_or(1.0)), 1.0) as i64;
        if pattern == 0 {
            return None;
        }
        let fg = self.color(r, "FillForegnd", |t| t.fill_color(r), Rgba::WHITE).with_transparency(self.num(
            r,
            "FillForegndTrans",
            |_| None,
            0.0,
        ));
        let gradient_on = self.num(
            r,
            "FillGradientEnabled",
            |t| Some(if t.fill(r).is_some_and(|f| !f.gradient.is_empty()) { 1.0 } else { 0.0 }),
            0.0,
        ) != 0.0;
        if gradient_on {
            let stops = self.gradient_stops(r);
            if stops.len() >= 2 {
                let dir = self.num(r, "FillGradientDir", |t| t.fill(r).and_then(|f| f.gradient_dir), 0.0);
                let angle = self.num(
                    r,
                    "FillGradientAngle",
                    |t| t.fill(r).and_then(|f| f.gradient_angle),
                    core::f64::consts::FRAC_PI_2,
                );
                return Some(paint::gradient(world, w, h, dir, angle, stops));
            }
            if let Some(s) = stops.first() {
                return Some(Paint::Solid { color: s.color });
            }
        }
        if pattern == 1 {
            return Some(Paint::Solid { color: fg });
        }
        let bg = self.color(r, "FillBkgnd", |_| None, Rgba::WHITE).with_transparency(self.num(
            r,
            "FillBkgndTrans",
            |_| None,
            0.0,
        ));
        paint::hatch(pattern, fg, bg)
            .or_else(|| paint::legacy_gradient(world, w, h, pattern, fg, bg))
            .or(Some(Paint::Solid { color: fg }))
    }

    /// Gradient stops from the `FillGradient` section (shape chain, then the
    /// fill style chain), evaluating themed stops against the theme.
    fn gradient_stops(&self, r: &Resolved<'d>) -> Vec<Stop> {
        let secs = r.sections("FillGradient");
        let rows: Vec<(Option<String>, Option<String>, Option<String>)> = match secs.first() {
            Some(sec) => sec
                .rows
                .iter()
                .map(|row| {
                    (
                        row.v.get("GradientStopColor").cloned(),
                        row.v.get("GradientStopColorTrans").cloned(),
                        row.v.get("GradientStopPosition").cloned(),
                    )
                })
                .collect(),
            None => r
                .style_chain(flowster_core::resolve::Category::Fill)
                .into_iter()
                .find_map(|st| st.sheet.section("FillGradient", None))
                .map(|sec| {
                    sec.rows
                        .iter()
                        .map(|row| {
                            let v = |c: &str| row.cells.get(c).and_then(|c| c.v.clone());
                            (v("GradientStopColor"), v("GradientStopColorTrans"), v("GradientStopPosition"))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };
        let themed = rows.iter().any(|(c, _, _)| c.as_deref().is_some_and(flowster_core::value::is_placeholder));
        if themed || rows.is_empty() {
            return self.themer(r).map(|t| t.gradient_stops(r)).unwrap_or_default();
        }
        let mut stops: Vec<Stop> = rows
            .iter()
            .filter_map(|(c, tr, pos)| {
                let color = parse_color(c.as_deref()?, &self.doc.colors)?;
                let trans = tr.as_deref().and_then(parse_num).unwrap_or(0.0);
                Some(Stop {
                    offset: pos.as_deref().and_then(parse_num).unwrap_or(0.0).clamp(0.0, 1.0),
                    color: color.with_transparency(trans),
                })
            })
            .collect();
        stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        stops
    }

    /// Drop shadow from the shadow cells, the page defaults or the theme's effect style.
    fn shadow(&self, r: &Resolved<'d>) -> Option<Shadow> {
        match r.lookup("ShdwPattern") {
            Lookup::Value(v) if parse_num(v).unwrap_or(0.0) != 0.0 => {
                let (dx, dy) = match self.num(r, "ShapeShdwType", |_| None, 0.0) as i64 {
                    0 => self.page_shadow,
                    _ => {
                        (self.num(r, "ShapeShdwOffsetX", |_| None, 0.0), self.num(r, "ShapeShdwOffsetY", |_| None, 0.0))
                    }
                };
                let color = self.color(r, "ShdwForegnd", |_| None, Rgba(0, 0, 0, 255)).with_transparency(self.num(
                    r,
                    "ShdwForegndTrans",
                    |_| None,
                    0.0,
                ));
                Some(Shadow { dx, dy, color, blur: self.num(r, "ShapeShdwBlur", |_| None, 0.0).max(0.0) })
            }
            Lookup::Themed => {
                let t = self.themer(r)?;
                let e = t.effect(r)?;
                let ph = t.placeholder(r, "QuickStyleShadowColor").or(Some(Rgba(0, 0, 0, 255)));
                let color = e.color.as_ref().map(|c| t.theme.resolve(c, ph)).unwrap_or(Rgba(0, 0, 0, 102));
                let dist = e.dist_emu / EMU_PER_INCH;
                Some(Shadow { dx: dist * cos(e.dir), dy: -dist * sin(e.dir), color, blur: e.blur_emu / EMU_PER_INCH })
            }
            _ => None,
        }
    }

    /// Arrowheads at the open ends of a stroked path (`BeginArrow`, `EndArrow`).
    fn arrows(&self, r: &Resolved<'d>, d: &mut [Seg], stroke: &Stroke) -> Vec<Item> {
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
            let Some(head) = arrows::head(kind) else { continue };
            if len_dir < 1e-9 {
                continue;
            }
            let u = arrows::unit(size, stroke.width);
            let (c, s) = ((tip.0 - from.0) / len_dir, (tip.1 - from.1) / len_dir);
            // Head frame: +X out of the line, +Y to its left, in head units.
            let place = |(x, y): (f64, f64)| (tip.0 + u * (x * c - y * s), tip.1 + u * (x * s + y * c));
            if head.trim > 0.0 {
                let back = place((-head.trim, 0.0));
                d[idx] = match d[idx] {
                    Seg::M { .. } => Seg::M { x: back.0, y: back.1 },
                    Seg::L { .. } => Seg::L { x: back.0, y: back.1 },
                    Seg::C { x1, y1, x2, y2, .. } => Seg::C { x1, y1, x2, y2, x: back.0, y: back.1 },
                    Seg::Z => Seg::Z,
                };
            }
            let outline = Stroke { dash: Vec::new(), cap: "round", ..stroke.clone() };
            for (style, local) in head.parts {
                let d: Vec<Seg> = local.iter().map(|seg| seg.map(place)).collect();
                let (fill, stroke) = match style {
                    arrows::Style::Filled => (Some(Paint::Solid { color: stroke.color }), None),
                    arrows::Style::Open | arrows::Style::Hollow => (None, Some(outline.clone())),
                };
                out.push(Item::Path { shape: r.shape().id, d, fill, stroke, blur: 0.0 });
            }
        }
        out
    }

    fn image(&mut self, r: &Resolved<'d>, world: Affine, w: f64, h: f64) {
        let Some(fd) = r.foreign() else { return };
        let media = fd.part.as_deref().and_then(|p| self.doc.media.get(p));
        let mime = media.map(|m| m.content_type.clone()).unwrap_or_else(|| "application/octet-stream".into());
        let (x, y) = (r.num("ImgOffsetX").unwrap_or(0.0), r.num("ImgOffsetY").unwrap_or(0.0));
        let (iw, ih) = (r.num("ImgWidth").unwrap_or(w), r.num("ImgHeight").unwrap_or(h));
        // Metafiles become vectors: the header frame (in reference-device
        // pixels, y down) fills the image rectangle.
        if let Some(m) = media
            && let Some(hd) = emf::header(&m.bytes)
        {
            let [fx0, fy0, fx1, fy1] = hd.frame;
            let (fw, fh) = ((fx1 - fx0).abs().max(1e-9), (fy1 - fy0).abs().max(1e-9));
            let to_page = world
                .then(Affine::translate(x, y + ih))
                .then(Affine::scale(iw / fw, -ih / fh))
                .then(Affine::translate(-fx0, -fy0));
            if emf::render(&m.bytes, r.shape().id, to_page, &mut self.items) {
                return;
            }
        }
        let displayable =
            matches!(mime.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/bmp" | "image/svg+xml");
        self.items.push(Item::Image {
            shape: r.shape().id,
            transform: world,
            x,
            y,
            w: iw,
            h: ih,
            data: media.filter(|_| displayable).map(|m| base64::engine::general_purpose::STANDARD.encode(&m.bytes)),
            mime,
        });
    }

    fn text(&mut self, r: &Resolved<'d>, world: Affine, w: f64, h: f64) {
        let Some(pieces) = r.text() else { return };
        if r.flag("HideText").unwrap_or(false) {
            return;
        }
        let chars = r.sections("Character");
        let paras = r.sections("Paragraph");
        let themer = self.themer(r);
        let font = |ix: u32| -> Run { self.char_run(r, &chars, themer.as_ref(), ix) };
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
                bullet: {
                    let custom = match r.text_row(&paras, "Paragraph", ix, "BulletStr") {
                        Lookup::Value(s) => Some(s.trim_matches('"').to_owned()).filter(|s| !s.is_empty()),
                        _ => None,
                    };
                    // Visio's bullet styles draw a dot unless BulletStr names the glyph.
                    custom.or_else(|| (v("Bullet").unwrap_or(0.0) >= 1.0).then(|| "•".to_owned()))
                },
                text_after_bullet: v("TextPosAfterBullet").unwrap_or(0.0).max(0.0),
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
/// (with a floor so hairlines still show their pattern), measured from Visio's
/// rendering at 1.5 pt and 6 pt. Visio draws dashes with flat ends whatever
/// the line cap, so the lengths are exact.
fn dash_pattern(pattern: f64, weight: f64) -> Vec<f64> {
    let unit = weight.max(0.01);
    let units: &[f64] = match pattern as i64 {
        0 | 1 => &[],
        2 => &[8.0, 4.0],
        3 => &[1.0, 4.0],
        4 => &[8.0, 4.0, 1.0, 4.0],
        5 => &[8.0, 4.0, 1.0, 4.0, 1.0, 4.0],
        6 => &[8.0, 4.0, 8.0, 4.0, 1.0, 4.0],
        7 => &[20.0, 4.0, 8.0, 4.0],
        8 => &[20.0, 4.0, 8.0, 4.0, 8.0, 4.0],
        9 => &[4.0, 2.0],
        10 => &[1.0, 2.0],
        11 => &[4.0, 2.0, 1.0, 2.0],
        12 => &[4.0, 2.0, 1.0, 2.0, 1.0, 2.0],
        13 => &[4.0, 2.0, 4.0, 2.0, 1.0, 2.0],
        14 => &[10.0, 2.0, 4.0, 2.0],
        15 => &[10.0, 2.0, 4.0, 2.0, 4.0, 2.0],
        16 => &[16.0, 8.0],
        17 => &[1.0, 8.0],
        18 => &[16.0, 8.0, 1.0, 8.0],
        19 => &[16.0, 8.0, 1.0, 8.0, 1.0, 8.0],
        20 => &[16.0, 8.0, 16.0, 8.0, 1.0, 8.0],
        21 => &[40.0, 8.0, 16.0, 8.0],
        22 => &[40.0, 8.0, 16.0, 8.0, 16.0, 8.0],
        23 => &[2.0, 1.0],
        _ => &[8.0, 4.0],
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
        // The drawing's extents: the box plus half its stroke.
        let half = stroke.as_ref().unwrap().width / 2.0;
        let [x0, y0, x1, y1] = dl.extents().unwrap();
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(near(x0, 3.25 - half) && near(y0, 5.0 - half) && near(x1, 5.25 + half) && near(y1, 6.0 + half));
    }

    #[test]
    fn dash_patterns_scale_with_the_weight() {
        assert!(dash_pattern(1.0, 0.02).is_empty());
        assert_eq!(dash_pattern(2.0, 0.02), vec![0.16, 0.08]);
        assert_eq!(dash_pattern(7.0, 0.1), vec![2.0, 0.4, 0.8, 0.4]);
        // Hairlines keep a visible pattern.
        assert_eq!(dash_pattern(3.0, 0.0), vec![0.01, 0.04]);
    }
}
