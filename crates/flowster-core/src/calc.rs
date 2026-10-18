//! Recalculation over the document model (SPEC §7.4).
//!
//! [`Calc`] evaluates any cell of a page's shapes from its formula, resolving
//! references through inheritance: an inherited formula is evaluated in the
//! context of the inheriting shape (§6.3). Results are memoised per evaluation
//! session; a cycle, or a function Flowster cannot evaluate yet, falls back to
//! the cached value the file stores.
//!
//! [`recalc_check`] recomputes every formula of a document and compares the
//! results with Visio's cached values: the engine's agreement metric on real files.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use crate::formula::{CellRef, Env, ErrKind, Evaluator, Expr, SheetRef, Value, parse};
use crate::model::{Document, Master, Page, Shape, TextPiece};
use crate::resolve::{MergedSection, Resolved};
use crate::transform::Affine;
use crate::value::{parse_color, parse_num};

/// Where a cell lives inside a sheet.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Addr {
    /// A single-row cell: `Width`, `LineColor`, `EventXFMod`.
    Single(String),
    /// A cell of a section row, or of the section itself (`row: None`).
    Row { section: String, section_ix: u32, row: Option<RowKey>, cell: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RowKey {
    Ix(u32),
    Name(String),
}

/// Splits a trailing number off a cell name: `X3` → (`X`, 3).
fn split_index(s: &str) -> Option<(&str, u32)> {
    let digits = s.bytes().rev().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits == s.len() {
        return None;
    }
    let (name, num) = s.split_at(s.len() - digits);
    Some((name, num.parse().ok()?))
}

/// Maps a formula cell reference to a sheet address (SPEC §7.3 `cellref`).
pub fn parse_addr(name: &str) -> Option<Addr> {
    let parts: Vec<&str> = name.split('.').collect();
    let first = parts[0].to_ascii_lowercase();
    let row = |section: &str, ix0: u32, rest: &[&str], default_cell: &str| -> Option<Addr> {
        match rest {
            // Section.Cell<n>: indexed row n (offset by the section's first IX).
            [c] => match split_index(c) {
                Some((cell, n)) => Some(Addr::Row {
                    section: section.into(),
                    section_ix: 0,
                    row: Some(RowKey::Ix(n - 1 + ix0)),
                    cell: cell.into(),
                }),
                // Section.RowName → its default cell.
                None => Some(Addr::Row {
                    section: section.into(),
                    section_ix: 0,
                    row: Some(RowKey::Name((*c).into())),
                    cell: default_cell.into(),
                }),
            },
            [r, c] => Some(Addr::Row {
                section: section.into(),
                section_ix: 0,
                row: Some(RowKey::Name((*r).into())),
                cell: (*c).into(),
            }),
            _ => None,
        }
    };
    match (first.as_str(), &parts[1..]) {
        (_, []) => Some(Addr::Single(name.to_owned())),
        ("char", [c]) => {
            Some(Addr::Row { section: "Character".into(), section_ix: 0, row: Some(RowKey::Ix(0)), cell: (*c).into() })
        }
        ("para", [c]) => {
            Some(Addr::Row { section: "Paragraph".into(), section_ix: 0, row: Some(RowKey::Ix(0)), cell: (*c).into() })
        }
        ("prop", rest) => row("Property", 0, rest, "Value"),
        ("user", rest) => row("User", 0, rest, "Value"),
        ("scratch", rest) => row("Scratch", 0, rest, "X"),
        ("connections", rest) => row("Connection", 0, rest, "X"),
        ("controls", rest) => row("Control", 0, rest, "X"),
        ("actions", rest) => row("Actions", 0, rest, "Action"),
        ("hyperlink", rest) => row("Hyperlink", 0, rest, "Address"),
        (g, [c]) if g.starts_with("geometry") => {
            let n: u32 = g["geometry".len()..].parse().ok()?;
            let section_ix = n.checked_sub(1)?;
            Some(match split_index(c) {
                // Geometry rows are numbered from 1 in both the reference and VSDX IX.
                Some((cell, r)) => {
                    Addr::Row { section: "Geometry".into(), section_ix, row: Some(RowKey::Ix(r)), cell: cell.into() }
                }
                None => Addr::Row { section: "Geometry".into(), section_ix, row: None, cell: (*c).into() },
            })
        }
        _ => None,
    }
}

/// Converts a cached `V` to a value: number, colour or text.
pub fn cached_value(v: &str) -> Value {
    if let Some(n) = parse_num(v) {
        return Value::Num(n);
    }
    if v.starts_with('#')
        && let Some(c) = parse_color(v, &BTreeMap::new())
    {
        return Value::Color(c);
    }
    Value::Str(v.to_owned())
}

/// Sheet key for memoisation: shape ids, or the page sheet.
const PAGE_SHEET: u32 = u32::MAX;

struct Entry<'d> {
    shape: &'d Shape,
    parent_master: Option<&'d Master>,
    parent: Option<u32>,
}

pub struct Calc<'d> {
    doc: &'d Document,
    page: Option<&'d Page>,
    shapes: HashMap<u32, Entry<'d>>,
    parsed: RefCell<HashMap<String, Option<Rc<Expr>>>>,
    memo: RefCell<HashMap<(u32, Addr), Value>>,
    active: RefCell<HashSet<(u32, Addr)>>,
}

impl<'d> Calc<'d> {
    /// A calculator over the shapes of `page`.
    pub fn for_page(doc: &'d Document, page: &'d Page) -> Self {
        let mut c = Self::empty(doc, Some(page));
        c.index(&page.shapes, None, None);
        c
    }

    /// A calculator over the shapes of a master (formulas inside master parts).
    pub fn for_master(doc: &'d Document, master: &'d Master) -> Self {
        let mut c = Self::empty(doc, None);
        c.index(&master.shapes, Some(master), None);
        c
    }

    fn empty(doc: &'d Document, page: Option<&'d Page>) -> Self {
        Calc {
            doc,
            page,
            shapes: HashMap::new(),
            parsed: RefCell::new(HashMap::new()),
            memo: RefCell::new(HashMap::new()),
            active: RefCell::new(HashSet::new()),
        }
    }

    fn index(&mut self, shapes: &'d [Shape], master: Option<&'d Master>, parent: Option<u32>) {
        for s in shapes {
            self.shapes.insert(s.id, Entry { shape: s, parent_master: master, parent });
            let r = Resolved::new(self.doc, s, master);
            let child_master = r.master();
            self.index(&s.shapes, child_master, Some(s.id));
        }
    }

    fn resolved(&self, id: u32) -> Option<Resolved<'d>> {
        let e = self.shapes.get(&id)?;
        Some(Resolved::new(self.doc, e.shape, e.parent_master))
    }

    fn parsed(&self, f: &str) -> Option<Rc<Expr>> {
        if let Some(e) = self.parsed.borrow().get(f) {
            return e.clone();
        }
        let e = parse(f).ok().map(Rc::new);
        self.parsed.borrow_mut().insert(f.to_owned(), e.clone());
        e
    }

    /// Formula and cached value of a cell, after inheritance.
    fn def(&self, sheet: u32, addr: &Addr) -> (Option<String>, Option<String>) {
        let owned = |f: Option<&str>, v: Option<&str>| (f.map(str::to_owned), v.map(str::to_owned));
        if sheet == PAGE_SHEET {
            let Some(page) = self.page else { return (None, None) };
            let Addr::Single(name) = addr else { return (None, None) };
            let c = page.sheet.cells.get(name.as_str());
            return owned(c.and_then(|c| c.f.as_deref()), c.and_then(|c| c.v.as_deref()));
        }
        let Some(r) = self.resolved(sheet) else { return (None, None) };
        match addr {
            Addr::Single(name) => r.cell_def(name).map(|d| owned(d.f, d.v)).unwrap_or((None, None)),
            Addr::Row { section, section_ix, row, cell } => {
                let secs: Vec<MergedSection> = r.sections(section);
                let Some(sec) = secs.iter().find(|s| s.ix == Some(*section_ix)).or(secs.first()) else {
                    return (None, None);
                };
                match row {
                    None => (None, sec.v.get(cell).cloned()),
                    Some(key) => {
                        let found = sec.rows.iter().find(|r| match key {
                            RowKey::Ix(ix) => r.ix == Some(*ix),
                            RowKey::Name(n) => r.n.as_deref().is_some_and(|rn| rn.eq_ignore_ascii_case(n)),
                        });
                        match found {
                            Some(r) => (r.f.get(cell).cloned(), r.v.get(cell).cloned()),
                            None => (None, None),
                        }
                    }
                }
            }
        }
    }

    /// Value of `addr` on sheet `sheet` (a shape id, or the page sheet).
    pub fn value(&self, sheet: u32, addr: &Addr) -> Value {
        let key = (sheet, addr.clone());
        if let Some(v) = self.memo.borrow().get(&key) {
            return v.clone();
        }
        let (f, v) = self.def(sheet, addr);
        let cached = v.as_deref().map(cached_value);
        let result = match f.as_deref().and_then(|f| self.parsed(f)) {
            Some(expr) if !self.active.borrow().contains(&key) => {
                self.active.borrow_mut().insert(key.clone());
                let env = SheetEnv { calc: self, sheet };
                let out = Evaluator { env: &env }.eval(&expr);
                self.active.borrow_mut().remove(&key);
                match out {
                    Value::Err(ErrKind::Unsupported) => cached.unwrap_or(Value::Err(ErrKind::Unsupported)),
                    other => other,
                }
            }
            // A cycle, a constant, or an unparsable formula: the cached value stands.
            _ => cached.unwrap_or(Value::Empty),
        };
        self.memo.borrow_mut().insert(key, result.clone());
        result
    }

    /// Whether `shape` is the shape this calculator knows as `id` (not a master's
    /// sub-shape that happens to share the id).
    pub fn owns(&self, id: u32, shape: &Shape) -> bool {
        self.shapes.get(&id).is_some_and(|e| std::ptr::eq(e.shape, shape))
    }

    /// Evaluates `formula` in the context of shape `sheet`.
    pub fn eval_in(&self, sheet: u32, formula: &str) -> Value {
        match self.parsed(formula) {
            Some(expr) => {
                let env = SheetEnv { calc: self, sheet };
                Evaluator { env: &env }.eval(&expr)
            }
            None => Value::Err(ErrKind::Name),
        }
    }

    /// Value of a cell named as a formula reference (`Width`, `Geometry1.X3`, `Prop.Cost`).
    pub fn cell(&self, shape: u32, reference: &str) -> Value {
        match parse_addr(reference) {
            Some(a) => self.value(shape, &a),
            None => Value::Err(ErrKind::Ref),
        }
    }

    /// A numeric cell; `None` when the cell is absent (so callers apply their default).
    fn num(&self, sheet: u32, cell: &str) -> Option<f64> {
        match self.value(sheet, &Addr::Single(cell.into())) {
            Value::Empty => None,
            v => v.num().ok(),
        }
    }

    /// Local → parent transform of a shape, from its computed cells.
    fn local_to_parent(&self, id: u32) -> Option<Affine> {
        let w = self.num(id, "Width").unwrap_or(0.0);
        let h = self.num(id, "Height").unwrap_or(0.0);
        Some(Affine::shape_local_to_parent(
            (self.num(id, "PinX")?, self.num(id, "PinY")?),
            (self.num(id, "LocPinX").unwrap_or(w / 2.0), self.num(id, "LocPinY").unwrap_or(h / 2.0)),
            self.num(id, "Angle").unwrap_or(0.0),
            self.num(id, "FlipX").unwrap_or(0.0) != 0.0,
            self.num(id, "FlipY").unwrap_or(0.0) != 0.0,
        ))
    }

    /// Local → page transform.
    fn to_world(&self, id: u32) -> Option<Affine> {
        let mut t = self.local_to_parent(id)?;
        let mut cur = self.shapes.get(&id)?.parent;
        let mut guard = 0;
        while let Some(p) = cur {
            t = self.local_to_parent(p)?.then(t);
            cur = self.shapes.get(&p).and_then(|e| e.parent);
            guard += 1;
            if guard > 64 {
                return None;
            }
        }
        Some(t)
    }

    /// Page → the parent frame of shape `id`.
    fn world_to_parent(&self, id: u32) -> Option<Affine> {
        match self.shapes.get(&id)?.parent {
            Some(p) => self.to_world(p)?.invert(),
            None => Some(Affine::IDENTITY),
        }
    }
}

struct SheetEnv<'c, 'd> {
    calc: &'c Calc<'d>,
    sheet: u32,
}

impl SheetEnv<'_, '_> {
    fn target(&self, r: &CellRef) -> Option<u32> {
        match &r.sheet {
            None => Some(self.sheet),
            Some(SheetRef::Id(n)) => Some(*n),
            Some(SheetRef::ThePage) => Some(PAGE_SHEET),
            Some(_) => None,
        }
    }
}

impl Env for SheetEnv<'_, '_> {
    fn cell(&self, r: &CellRef) -> Value {
        let Some(sheet) = self.target(r) else { return Value::Err(ErrKind::Unsupported) };
        if sheet != PAGE_SHEET && !self.calc.shapes.contains_key(&sheet) {
            return Value::Err(ErrKind::Ref);
        }
        match parse_addr(&r.name) {
            Some(a) => match self.calc.value(sheet, &a) {
                Value::Empty if self.calc.def(sheet, &a) == (None, None) => Value::Err(ErrKind::Unsupported),
                v => v,
            },
            None => Value::Err(ErrKind::Unsupported),
        }
    }

    fn sheet_of(&self, r: &CellRef) -> Option<u32> {
        self.target(r).filter(|s| *s != PAGE_SHEET)
    }

    fn par(&self, sheet: Option<u32>, p: (f64, f64)) -> Option<(f64, f64)> {
        let from = sheet.unwrap_or(self.sheet);
        let world = self.calc.to_world(from)?.apply(p);
        Some(self.calc.world_to_parent(self.sheet)?.apply(world))
    }

    fn loc(&self, sheet: Option<u32>, p: (f64, f64)) -> Option<(f64, f64)> {
        let to = sheet.unwrap_or(self.sheet);
        let world = self.calc.world_to_parent(self.sheet)?.invert()?.apply(p);
        Some(self.calc.to_world(to)?.invert()?.apply(world))
    }

    fn shape_text(&self) -> Option<String> {
        let r = self.calc.resolved(self.sheet)?;
        Some(
            r.text()?
                .iter()
                .filter_map(|p| match p {
                    TextPiece::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect::<String>()
                .trim_end_matches('\n')
                .to_owned(),
        )
    }

    fn master_name(&self) -> Option<String> {
        self.calc.resolved(self.sheet)?.master()?.name_u.clone()
    }
}

/// Outcome of recomputing every formula in a document.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct RecalcReport {
    pub formulas: usize,
    /// Recomputed value equals the cached value.
    pub matched: usize,
    /// Kept the cached value: unsupported function, cycle or unresolvable reference.
    pub kept_cached: usize,
    pub mismatched: Vec<Mismatch>,
    /// Functions that forced a cached value, with counts.
    pub unsupported_functions: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Mismatch {
    pub page: String,
    pub shape: u32,
    pub cell: String,
    pub formula: String,
    pub computed: String,
    pub cached: String,
}

fn same(computed: &Value, cached: &str) -> bool {
    match (computed, cached_value(cached)) {
        (Value::Num(a), Value::Num(b)) => (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0),
        (Value::Bool(a), Value::Num(b)) => (if *a { 1.0 } else { 0.0 } - b).abs() < 1e-12,
        (Value::Color(a), Value::Color(b)) => a.hex() == b.hex(),
        (a, Value::Str(s)) => a.text().ok().is_some_and(|t| t == s),
        _ => false,
    }
}

fn functions_in(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Call(name, args) => {
            out.push(name.clone());
            for a in args {
                functions_in(a, out);
            }
        }
        Expr::Neg(x) | Expr::Percent(x) => functions_in(x, out),
        Expr::Bin(_, a, b) => {
            functions_in(a, out);
            functions_in(b, out);
        }
        _ => {}
    }
}

/// Recomputes every local formula (shape cells and section row cells) on every
/// page and compares the result with the value cached in the file.
pub fn recalc_check(doc: &Document) -> RecalcReport {
    let mut report = RecalcReport::default();
    for page in &doc.pages {
        let calc = Calc::for_page(doc, page);
        let page_name = page.name_u.clone().unwrap_or_default();
        let mut ids: Vec<u32> = calc.shapes.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let shape = calc.shapes[&id].shape;
            let mut cells: Vec<(Addr, &str, Option<&str>)> = Vec::new();
            for (name, c) in &shape.sheet.cells {
                if let Some(f) = c.f.as_deref() {
                    cells.push((Addr::Single(name.clone()), f, c.v.as_deref()));
                }
            }
            for (pos, sec) in shape.sheet.sections.iter().enumerate() {
                for row in &sec.rows {
                    let key = match (&row.n, row.ix) {
                        (Some(n), _) => Some(RowKey::Name(n.clone())),
                        (None, Some(ix)) => Some(RowKey::Ix(ix)),
                        _ => None,
                    };
                    for (name, c) in &row.cells {
                        if let Some(f) = c.f.as_deref() {
                            let addr = Addr::Row {
                                section: sec.n.clone(),
                                section_ix: sec.ix.unwrap_or(pos as u32),
                                row: key.clone(),
                                cell: name.clone(),
                            };
                            cells.push((addr, f, c.v.as_deref()));
                        }
                    }
                }
            }
            for (addr, f, v) in cells {
                if crate::resolve::is_inherit(f) || f.eq_ignore_ascii_case("No Formula") {
                    continue;
                }
                let Some(v) = v else { continue };
                report.formulas += 1;
                let direct = match parse(f) {
                    Ok(e) => {
                        let env = SheetEnv { calc: &calc, sheet: id };
                        let out = Evaluator { env: &env }.eval(&e);
                        if out == Value::Err(ErrKind::Unsupported) {
                            let mut fns = Vec::new();
                            functions_in(&e, &mut fns);
                            for name in fns {
                                if name.starts_with('_') || unsupported_name(&name) {
                                    *report.unsupported_functions.entry(name).or_default() += 1;
                                }
                            }
                        }
                        out
                    }
                    Err(_) => Value::Err(ErrKind::Name),
                };
                if direct == Value::Err(ErrKind::Unsupported) {
                    report.kept_cached += 1;
                } else if same(&direct, v) {
                    report.matched += 1;
                } else {
                    report.mismatched.push(Mismatch {
                        page: page_name.clone(),
                        shape: id,
                        cell: format!("{addr:?}"),
                        formula: f.to_owned(),
                        computed: direct.text().unwrap_or_else(|k| k.label().to_owned()),
                        cached: v.to_owned(),
                    });
                }
            }
        }
    }
    report
}

fn unsupported_name(name: &str) -> bool {
    struct Probe;
    impl Env for Probe {
        fn cell(&self, _r: &CellRef) -> Value {
            Value::Num(0.0)
        }
    }
    // Ask the evaluator itself whether this function is a known gap.
    let args = vec![Expr::Num(0.0); 1];
    let v = Evaluator { env: &Probe }.eval(&Expr::Call(name.to_owned(), args));
    v == Value::Err(ErrKind::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cell, Row, Section, Sheet};

    fn f(formula: &str, v: &str) -> Cell {
        Cell { v: Some(v.into()), f: Some(formula.into()), u: None }
    }

    fn shape(id: u32, cells: &[(&str, Cell)]) -> Shape {
        Shape {
            id,
            sheet: Sheet {
                cells: cells.iter().map(|(k, c)| (k.to_string(), c.clone())).collect(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn inherited_formulas_evaluate_in_the_instance() {
        // Master: LocPinX = Width*0.5 with the master's cached value (1).
        let master = Master {
            id: 2,
            shapes: vec![shape(5, &[("Width", Cell::value("2")), ("LocPinX", f("Width*0.5", "1"))])],
            ..Default::default()
        };
        // Instance overrides only Width; LocPinX is inherited and has no local value.
        let mut inst = shape(1, &[("Width", Cell::value("4")), ("PinX", Cell::value("3"))]);
        inst.master = Some(2);
        let page = Page { shapes: vec![inst], ..Default::default() };
        let doc = Document { pages: vec![page], masters: BTreeMap::from([(2, master)]), ..Default::default() };
        let calc = Calc::for_page(&doc, &doc.pages[0]);
        assert_eq!(calc.cell(1, "LocPinX"), Value::Num(2.0));
        assert_eq!(calc.cell(1, "PinX"), Value::Num(3.0));
    }

    #[test]
    fn cross_sheet_references_sections_and_glue() {
        let mut a = shape(
            1,
            &[
                ("PinX", Cell::value("2")),
                ("PinY", Cell::value("3")),
                ("Width", Cell::value("2")),
                ("Height", Cell::value("1")),
                ("Angle", Cell::value("0")),
            ],
        );
        a.sheet.sections.push(Section {
            n: "Connection".into(),
            rows: vec![Row {
                ix: Some(0),
                cells: [("X".to_string(), f("Width", "2")), ("Y".to_string(), f("Height*0.5", "0.5"))].into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        a.sheet.sections.push(Section {
            n: "Property".into(),
            rows: vec![Row {
                n: Some("Cost".into()),
                cells: [("Value".to_string(), Cell::value("12.5"))].into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        // A connector glued to Sheet.1's first connection point.
        let b = shape(
            2,
            &[
                ("BeginX", f("PAR(PNT(Sheet.1!Connections.X1,Sheet.1!Connections.Y1))", "0")),
                ("Total", f("Sheet.1!Prop.Cost*2+ThePage!PageWidth", "0")),
            ],
        );
        let page = Page {
            sheet: Sheet { cells: [("PageWidth".to_string(), Cell::value("11"))].into(), ..Default::default() },
            shapes: vec![a, b],
            ..Default::default()
        };
        let doc = Document { pages: vec![page], ..Default::default() };
        let calc = Calc::for_page(&doc, &doc.pages[0]);
        // Connection point (2, 0.5) in Sheet.1's local frame is (3, 3) on the page.
        let Value::Point { x, y, .. } = calc.cell(2, "BeginX") else { panic!("{:?}", calc.cell(2, "BeginX")) };
        assert_eq!((x, y), (3.0, 3.0));
        assert_eq!(calc.cell(2, "Total"), Value::Num(36.0));
        assert_eq!(calc.cell(1, "Connections.Y1"), Value::Num(0.5));
    }

    #[test]
    fn cycles_fall_back_to_cached_values() {
        let s = shape(1, &[("Width", f("Height*2", "4")), ("Height", f("Width/2", "2"))]);
        let page = Page { shapes: vec![s], ..Default::default() };
        let doc = Document { pages: vec![page], ..Default::default() };
        let calc = Calc::for_page(&doc, &doc.pages[0]);
        assert_eq!(calc.cell(1, "Width"), Value::Num(4.0));
    }

    #[test]
    fn addresses() {
        assert_eq!(parse_addr("Width"), Some(Addr::Single("Width".into())));
        assert_eq!(
            parse_addr("Geometry2.X3"),
            Some(Addr::Row { section: "Geometry".into(), section_ix: 1, row: Some(RowKey::Ix(3)), cell: "X".into() })
        );
        assert_eq!(
            parse_addr("Prop.Cost.Label"),
            Some(Addr::Row {
                section: "Property".into(),
                section_ix: 0,
                row: Some(RowKey::Name("Cost".into())),
                cell: "Label".into()
            })
        );
        assert_eq!(
            parse_addr("Scratch.X1"),
            Some(Addr::Row { section: "Scratch".into(), section_ix: 0, row: Some(RowKey::Ix(0)), cell: "X".into() })
        );
    }
}
