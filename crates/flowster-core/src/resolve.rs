//! Cell resolution with inheritance (SPEC §6.3):
//! local cell → master shape (recursively) → style chain for the cell's
//! category (`LineStyle`/`FillStyle`/`TextStyle`, following `BasedOn`) → none.
//!
//! Sections and rows inherit too: an instance's `Geometry` row `IX=1` with only
//! a `Y` cell keeps the master's `X`, and `Del='1'` removes a row or section.
//!
//! Cached values of `Themed` mean "the theme decides". [`Resolved::lookup`]
//! reports them as [`Lookup::Themed`] when the document has a theme; without
//! one they are skipped and resolution continues down the chain, which ends at
//! the application defaults in the "No Style" style (what Visio shows too).

use std::collections::BTreeMap;

use crate::model::{Document, Master, Shape, StyleSheet, TextPiece};
use crate::value::{is_placeholder, parse_bool, parse_num};

/// Which style chain a cell inherits from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Line,
    Fill,
    Text,
}

/// The style category of a single-row-section cell, if it has one.
pub fn category_of(cell: &str) -> Option<Category> {
    const LINE: &[&str] = &[
        "LineWeight",
        "LineColor",
        "LinePattern",
        "Rounding",
        "EndArrowSize",
        "BeginArrow",
        "EndArrow",
        "LineCap",
        "BeginArrowSize",
        "LineColorTrans",
        "CompoundType",
        "LineGradientEnabled",
    ];
    const FILL: &[&str] = &[
        "FillForegnd",
        "FillBkgnd",
        "FillPattern",
        "ShdwForegnd",
        "ShdwPattern",
        "FillForegndTrans",
        "FillBkgndTrans",
        "ShdwForegndTrans",
        "ShapeShdwType",
        "ShapeShdwOffsetX",
        "ShapeShdwOffsetY",
        "ShapeShdwShow",
        "FillGradientEnabled",
    ];
    const TEXT: &[&str] = &[
        "LeftMargin",
        "RightMargin",
        "TopMargin",
        "BottomMargin",
        "VerticalAlign",
        "TextBkgnd",
        "DefaultTabStop",
        "TextDirection",
        "TextBkgndTrans",
    ];
    if LINE.contains(&cell) {
        Some(Category::Line)
    } else if FILL.contains(&cell) {
        Some(Category::Fill)
    } else if TEXT.contains(&cell) {
        Some(Category::Text)
    } else {
        None
    }
}

/// Result of looking a cell up through the chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup<'d> {
    Value(&'d str),
    /// The nearest definition is `THEMEVAL()`: evaluate against the theme.
    Themed,
    Missing,
}

/// A row after inheritance: cached values and formulas by cell name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MergedRow {
    pub t: Option<String>,
    pub ix: Option<u32>,
    pub n: Option<String>,
    pub v: BTreeMap<String, String>,
    pub f: BTreeMap<String, String>,
}

impl MergedRow {
    pub fn num(&self, cell: &str) -> Option<f64> {
        self.v.get(cell).and_then(|v| parse_num(v))
    }
    /// The value, or the formula when the producer left the value empty.
    pub fn raw(&self, cell: &str) -> Option<&str> {
        self.v.get(cell).filter(|v| !v.trim().is_empty()).or_else(|| self.f.get(cell)).map(String::as_str)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MergedSection {
    pub ix: Option<u32>,
    pub v: BTreeMap<String, String>,
    pub rows: Vec<MergedRow>,
}

impl MergedSection {
    pub fn flag(&self, cell: &str) -> bool {
        self.v.get(cell).and_then(|v| parse_bool(v)).unwrap_or(false)
    }
    pub fn row(&self, ix: u32) -> Option<&MergedRow> {
        self.rows.iter().find(|r| r.ix == Some(ix))
    }
}

/// A shape seen through its inheritance chain.
pub struct Resolved<'d> {
    doc: &'d Document,
    /// The shape first, then the master shapes it inherits from.
    chain: Vec<&'d Shape>,
    /// Master that children's `MasterShape` ids refer to.
    master: Option<&'d Master>,
}

const MAX_DEPTH: usize = 16;

impl<'d> Resolved<'d> {
    /// `parent_master` is the master of the enclosing group instance, if any.
    pub fn new(doc: &'d Document, shape: &'d Shape, parent_master: Option<&'d Master>) -> Self {
        let mut chain = vec![shape];
        let mut master = parent_master;
        let mut cur = shape;
        for _ in 0..MAX_DEPTH {
            let next = if let Some(mid) = cur.master {
                match doc.masters.get(&mid) {
                    Some(m) => {
                        master = Some(m);
                        match cur.master_shape {
                            Some(ms) => m.find(ms),
                            None => m.shapes.first(),
                        }
                    }
                    None => None,
                }
            } else if let (Some(ms), Some(m)) = (cur.master_shape, master) {
                m.find(ms).filter(|s| !std::ptr::eq(*s, cur))
            } else {
                None
            };
            match next {
                Some(n) if !chain.iter().any(|c| std::ptr::eq(*c, n)) => {
                    chain.push(n);
                    cur = n;
                }
                _ => break,
            }
        }
        Resolved { doc, chain, master }
    }

    pub fn shape(&self) -> &'d Shape {
        self.chain[0]
    }

    pub fn master(&self) -> Option<&'d Master> {
        self.master
    }

    pub fn doc(&self) -> &'d Document {
        self.doc
    }

    /// Children to draw: the instance's own sub-shapes, or the master shape's
    /// when the instance lists none.
    pub fn children(&self) -> &'d [Shape] {
        self.chain.iter().map(|s| s.shapes.as_slice()).find(|c| !c.is_empty()).unwrap_or(&[])
    }

    pub fn text(&self) -> Option<&'d [TextPiece]> {
        self.chain.iter().find_map(|s| s.text.as_deref())
    }

    pub fn foreign(&self) -> Option<&'d crate::model::ForeignData> {
        self.chain.iter().find_map(|s| s.foreign.as_ref())
    }

    fn style_id(&self, cat: Category) -> Option<u32> {
        let pick = |s: &Shape| match cat {
            Category::Line => s.line_style,
            Category::Fill => s.fill_style,
            Category::Text => s.text_style,
        };
        self.chain.iter().find_map(|s| pick(s)).or(match cat {
            Category::Line => self.doc.default_styles.line,
            Category::Fill => self.doc.default_styles.fill,
            Category::Text => self.doc.default_styles.text,
        })
    }

    /// Styles to consult for a category, nearest first.
    pub fn style_chain(&self, cat: Category) -> Vec<&'d StyleSheet> {
        let mut out: Vec<&StyleSheet> = Vec::new();
        let mut next = self.style_id(cat);
        while let Some(id) = next {
            let Some(st) = self.doc.styles.get(&id) else { break };
            if out.iter().any(|s| s.id == id) || out.len() >= MAX_DEPTH {
                break;
            }
            out.push(st);
            next = match cat {
                Category::Line => st.line_style,
                Category::Fill => st.fill_style,
                Category::Text => st.text_style,
            };
        }
        out
    }

    /// A single-row cell after inheritance, distinguishing theme-driven values.
    pub fn lookup(&self, cell: &str) -> Lookup<'d> {
        let themed = self.doc.theme.is_some();
        let check = |v: Option<&'d str>| -> Option<Lookup<'d>> {
            let v = v?;
            if !is_placeholder(v) {
                Some(Lookup::Value(v))
            } else if themed {
                Some(Lookup::Themed)
            } else {
                None
            }
        };
        for s in &self.chain {
            if let Some(l) = check(s.sheet.cells.get(cell).and_then(|c| c.v.as_deref())) {
                return l;
            }
        }
        if let Some(cat) = category_of(cell) {
            for st in self.style_chain(cat) {
                if let Some(l) = check(st.sheet.cells.get(cell).and_then(|c| c.v.as_deref())) {
                    return l;
                }
            }
        }
        Lookup::Missing
    }

    /// Cached value of a single-row cell after inheritance (`None` if missing or themed).
    pub fn v(&self, cell: &str) -> Option<&'d str> {
        match self.lookup(cell) {
            Lookup::Value(v) => Some(v),
            _ => None,
        }
    }

    pub fn num(&self, cell: &str) -> Option<f64> {
        self.v(cell).and_then(parse_num)
    }

    pub fn flag(&self, cell: &str) -> Option<bool> {
        self.v(cell).and_then(parse_bool)
    }

    /// All instances of section `name` (e.g. every `Geometry` section), merged
    /// across the chain and ordered by `IX`.
    pub fn sections(&self, name: &str) -> Vec<MergedSection> {
        let mut by_ix: BTreeMap<u32, MergedSection> = BTreeMap::new();
        for s in self.chain.iter().rev() {
            for (pos, sec) in s.sheet.sections_named(name).enumerate() {
                let key = sec.ix.unwrap_or(pos as u32);
                if sec.del {
                    by_ix.remove(&key);
                    continue;
                }
                let merged = by_ix.entry(key).or_insert_with(|| MergedSection { ix: Some(key), ..Default::default() });
                for (k, c) in &sec.cells {
                    if let Some(v) = &c.v {
                        merged.v.insert(k.clone(), v.clone());
                    }
                }
                for (rpos, row) in sec.rows.iter().enumerate() {
                    let found = merged.rows.iter().position(|r| match (&row.n, row.ix) {
                        (Some(n), _) => r.n.as_ref() == Some(n),
                        (None, Some(ix)) => r.ix == Some(ix),
                        (None, None) => false,
                    });
                    if row.del {
                        if let Some(i) = found {
                            merged.rows.remove(i);
                        }
                        continue;
                    }
                    let i = match found {
                        Some(i) => i,
                        None => {
                            merged.rows.push(MergedRow {
                                ix: row.ix.or(Some(rpos as u32)),
                                n: row.n.clone(),
                                ..Default::default()
                            });
                            merged.rows.len() - 1
                        }
                    };
                    let r = &mut merged.rows[i];
                    if row.t.is_some() {
                        r.t = row.t.clone();
                    }
                    for (k, c) in &row.cells {
                        if let Some(v) = &c.v {
                            r.v.insert(k.clone(), v.clone());
                        }
                        if let Some(f) = &c.f {
                            r.f.insert(k.clone(), f.clone());
                        }
                    }
                }
            }
        }
        let mut out: Vec<MergedSection> = by_ix.into_values().collect();
        for s in &mut out {
            s.rows.sort_by_key(|r| r.ix.unwrap_or(u32::MAX));
        }
        out
    }

    /// A cell of row `ix` in a text section (`Character`, `Paragraph`), falling
    /// back to row 0 of the text style chain. Returns `Themed` like [`Self::lookup`].
    pub fn text_row<'a>(&'a self, merged: &'a [MergedSection], section: &str, ix: u32, cell: &str) -> Lookup<'a>
    where
        'd: 'a,
    {
        let themed = self.doc.theme.is_some();
        if let Some(v) = merged.first().and_then(|s| s.row(ix)).and_then(|r| r.v.get(cell)) {
            if !is_placeholder(v) {
                return Lookup::Value(v.as_str());
            } else if themed {
                return Lookup::Themed;
            }
        }
        for st in self.style_chain(Category::Text) {
            let v = st
                .sheet
                .section(section, None)
                .and_then(|s| s.rows.iter().find(|r| r.ix.unwrap_or(0) == 0))
                .and_then(|r| r.cells.get(cell))
                .and_then(|c| c.v.as_deref());
            match v {
                Some(v) if !is_placeholder(v) => return Lookup::Value(v),
                Some(_) if themed => return Lookup::Themed,
                _ => {}
            }
        }
        Lookup::Missing
    }
}
