//! Editing commands on the CRDT (SPEC §5.5, §20.6): each runs in the caller's
//! transaction, writes atomic cell groups, then recalculates the shapes it
//! touched so cached values follow their formulas in the same undo step.

use std::collections::HashMap;

use flowster_core::calc::{Addr, RowKey, format_num, stale_cells};
use flowster_core::command::{ChangeSummary, CommandError, ShapeKind};
use flowster_core::model::{Cell, Document as Model, Page, Row, Section, Shape, Sheet};
use loro::{Container, LoroDoc, LoroMap, LoroMovableList, LoroText, LoroValue, TreeID, ValueOrContainer};

use crate::model_io::{
    self, cell_from, cell_value, child_map, entries, group_of, int, lazy_map, string, uuid, write_new_shape,
};
use crate::schema;

type Result<T> = std::result::Result<T, CommandError>;

fn failed(e: impl std::fmt::Display) -> CommandError {
    CommandError::Failed(e.to_string())
}

/// Visio's look for drawn shapes under the Office theme: accent-1 fill, a
/// darker outline, white 12 pt Calibri text, centred.
const FILL: &str = "#4472c4";
const OUTLINE: &str = "#2f528f";
const LINE_WEIGHT: &str = "0.01041666666666667";

/// A new, blank drawing: one US Letter landscape page.
pub fn blank_model() -> Model {
    let mut sheet = Sheet::default();
    for (name, v) in [("PageWidth", "11"), ("PageHeight", "8.5"), ("PageScale", "1"), ("DrawingScale", "1")] {
        sheet.cells.insert(name.into(), Cell { v: Some(v.into()), f: None, u: Some("IN".into()) });
    }
    Model {
        pages: vec![Page {
            id: 0,
            name_u: Some("Page-1".into()),
            name: Some("Page-1".into()),
            sheet,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn v(value: impl Into<String>) -> Cell {
    Cell { v: Some(value.into()), f: None, u: None }
}

fn f(formula: &str) -> Cell {
    Cell { v: None, f: Some(formula.into()), u: None }
}

fn row(t: &str, ix: u32, cells: &[(&str, &str)]) -> Row {
    Row {
        t: Some(t.into()),
        ix: Some(ix),
        cells: cells.iter().map(|(k, x)| ((*k).to_owned(), f(x))).collect(),
        ..Default::default()
    }
}

/// The model of a newly drawn shape (cached values are filled in by recalc).
pub fn new_shape(kind: ShapeKind, id: u32, x: f64, y: f64, w: f64, h: f64) -> Shape {
    let mut sheet = Sheet::default();
    let mut put = |name: &str, c: Cell| {
        sheet.cells.insert(name.into(), c);
    };
    put("LineWeight", v(LINE_WEIGHT));
    put("LinePattern", v("1"));
    let geometry = match kind {
        ShapeKind::Line => {
            put("BeginX", v(format_num(x)));
            put("BeginY", v(format_num(y)));
            put("EndX", v(format_num(x + w)));
            put("EndY", v(format_num(y + h)));
            put("PinX", f("(BeginX+EndX)/2"));
            put("PinY", f("(BeginY+EndY)/2"));
            put("Width", f("SQRT((EndX-BeginX)^2+(EndY-BeginY)^2)"));
            put("Height", v("0"));
            put("Angle", f("ATAN2(EndY-BeginY,EndX-BeginX)"));
            put("LineColor", v(FILL));
            Section {
                n: "Geometry".into(),
                ix: Some(0),
                cells: [("NoFill".to_owned(), v("1"))].into_iter().collect(),
                rows: vec![
                    row("MoveTo", 1, &[("X", "Width*0"), ("Y", "Height*0.5")]),
                    row("LineTo", 2, &[("X", "Width*1"), ("Y", "Height*0.5")]),
                ],
                ..Default::default()
            }
        }
        _ => {
            put("PinX", v(format_num(x + w / 2.0)));
            put("PinY", v(format_num(y + h / 2.0)));
            put("Width", v(format_num(w)));
            put("Height", v(format_num(h)));
            put("Angle", v("0"));
            put("LineColor", v(OUTLINE));
            put("FillForegnd", v(FILL));
            put("FillPattern", v("1"));
            put("VerticalAlign", v("1"));
            let rows = if kind == ShapeKind::Ellipse {
                vec![row(
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
                )]
            } else {
                vec![
                    row("MoveTo", 1, &[("X", "Width*0"), ("Y", "Height*0")]),
                    row("LineTo", 2, &[("X", "Width*1"), ("Y", "Height*0")]),
                    row("LineTo", 3, &[("X", "Width*1"), ("Y", "Height*1")]),
                    row("LineTo", 4, &[("X", "Width*0"), ("Y", "Height*1")]),
                    row("LineTo", 5, &[("X", "Width*0"), ("Y", "Height*0")]),
                ]
            };
            Section { n: "Geometry".into(), ix: Some(0), rows, ..Default::default() }
        }
    };
    put("LocPinX", f("Width*0.5"));
    put("LocPinY", f("Height*0.5"));
    let character = Section {
        n: "Character".into(),
        rows: vec![Row {
            ix: Some(0),
            cells: [("Color", "#ffffff"), ("Size", "0.1666666666666667"), ("Font", "Calibri")]
                .into_iter()
                .map(|(k, x)| (k.to_owned(), v(x)))
                .collect(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let paragraph = Section {
        n: "Paragraph".into(),
        rows: vec![Row { ix: Some(0), cells: [("HorzAlign".to_owned(), v("1"))].into(), ..Default::default() }],
        ..Default::default()
    };
    let mut sections = vec![geometry];
    if kind != ShapeKind::Line {
        sections.push(character);
        sections.push(paragraph);
    }
    sheet.sections = sections;
    Shape { id, name_u: Some(format!("Sheet.{id}")), name: Some(format!("Sheet.{id}")), sheet, ..Default::default() }
}

/// Edits run against one document.
pub struct Editor<'a> {
    pub doc: &'a LoroDoc,
    /// Peer-unique counter for new uuids.
    pub next: &'a mut u64,
}

impl Editor<'_> {
    fn page_uuid(&self, page: usize) -> Result<String> {
        match self.doc.get_movable_list(schema::PAGES).get(page) {
            Some(ValueOrContainer::Value(LoroValue::String(s))) => Ok(s.to_string()),
            _ => Err(failed(format!("no page {page}"))),
        }
    }

    /// Every live node of a page tree with its shape uuid, depth-first.
    fn nodes(&self, page_uuid: &str) -> Vec<(TreeID, String)> {
        let tree = self.doc.get_tree(schema::shapes_tree_key(page_uuid));
        let mut out = Vec::new();
        let mut stack: Vec<TreeID> = tree.children(None).unwrap_or_default().into_iter().rev().collect();
        while let Some(node) = stack.pop() {
            if let Some(s) = tree.get_meta(node).ok().and_then(|m| string(&m, "s")) {
                out.push((node, s));
            }
            stack.extend(tree.children(node).unwrap_or_default().into_iter().rev());
        }
        out
    }

    fn shape_map(&self, uuid: &str) -> Option<LoroMap> {
        child_map(&self.doc.get_map(schema::SHAPE), uuid)
    }

    fn find(&self, page_uuid: &str, id: u32) -> Result<(TreeID, String, LoroMap)> {
        for (node, uuid) in self.nodes(page_uuid) {
            if let Some(m) = self.shape_map(&uuid)
                && int(&m, "legacyId") == Some(i64::from(id))
            {
                return Ok((node, uuid, m));
            }
        }
        Err(failed(format!("no shape {id}")))
    }

    fn sheet(m: &LoroMap) -> LoroMap {
        lazy_map(m, schema::SHEET)
    }

    /// A single-row cell as stored locally.
    fn cell(sheet: &LoroMap, name: &str) -> Option<Cell> {
        match group_of(name) {
            Some(g) => match child_map(sheet, "groups").and_then(|gm| model_io::value(&gm, g)) {
                Some(LoroValue::Map(m)) => m.get(name).map(cell_from),
                _ => None,
            },
            None => child_map(sheet, "cells").and_then(|c| model_io::value(&c, name)).map(|x| cell_from(&x)),
        }
    }

    /// Writes single-row cells; cells of one atomic group are rewritten together.
    fn put_cells(sheet: &LoroMap, cells: &[(&str, Cell)]) -> Result<()> {
        let mut groups: HashMap<&str, HashMap<String, LoroValue>> = HashMap::new();
        for (name, c) in cells {
            match group_of(name) {
                Some(g) => {
                    let group = groups.entry(g).or_insert_with(|| {
                        match child_map(sheet, "groups").and_then(|gm| model_io::value(&gm, g)) {
                            Some(LoroValue::Map(m)) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                            _ => HashMap::new(),
                        }
                    });
                    group.insert((*name).to_owned(), cell_value(c));
                }
                None => lazy_map(sheet, "cells").insert(name, cell_value(c)).map_err(failed)?,
            }
        }
        for (g, value) in groups {
            lazy_map(sheet, "groups").insert(g, value).map_err(failed)?;
        }
        Ok(())
    }

    /// Writes a cell of a section row, creating the local section/row when the
    /// shape so far inherited it.
    fn put_row_cell(sheet: &LoroMap, section: &str, ix: u32, row: &RowKey, cell: &str, c: Cell) -> Result<()> {
        let sections = lazy_map(sheet, "sections");
        let found = entries(&sections).into_iter().find_map(|(_, v)| match v {
            ValueOrContainer::Container(Container::Map(m))
                if string(&m, "@n").as_deref() == Some(section) && int(&m, "@ix").unwrap_or(0) == i64::from(ix) =>
            {
                Some(m)
            }
            _ => None,
        });
        let sm = match found {
            Some(m) => m,
            None => {
                let pos = entries(&sections).len() as i64;
                let m = sections.insert_container(&format!("{section}.{ix}"), LoroMap::new()).map_err(failed)?;
                m.insert("@pos", pos).map_err(failed)?;
                m.insert("@n", section).map_err(failed)?;
                m.insert("@ix", i64::from(ix)).map_err(failed)?;
                m
            }
        };
        let row_map = match (row, sm.get("rows")) {
            (RowKey::Name(n), Some(ValueOrContainer::Container(Container::Map(rows)))) => match child_map(&rows, n) {
                Some(r) => r,
                None => new_named_row(&rows, n)?,
            },
            (RowKey::Name(n), _) => new_named_row(&sm.insert_container("rows", LoroMap::new()).map_err(failed)?, n)?,
            (RowKey::Ix(rix), rows) => {
                let list = match rows {
                    Some(ValueOrContainer::Container(Container::MovableList(l))) => l,
                    _ => sm.insert_container("rows", LoroMovableList::new()).map_err(failed)?,
                };
                let existing = (0..list.len()).find_map(|i| match list.get(i) {
                    Some(ValueOrContainer::Container(Container::Map(m))) if int(&m, "@ix") == Some(i64::from(*rix)) => {
                        Some(m)
                    }
                    _ => None,
                });
                match existing {
                    Some(m) => m,
                    None => {
                        let m = list.push_container(LoroMap::new()).map_err(failed)?;
                        m.insert("@pos", list.len() as i64).map_err(failed)?;
                        m.insert("@ix", i64::from(*rix)).map_err(failed)?;
                        m
                    }
                }
            }
        };
        row_map.insert(cell, cell_value(&c)).map_err(failed)
    }

    /// Rewrites the cached values of `shapes` that no longer match their formulas.
    fn recalc(&self, page: usize, shapes: &[u32]) -> Result<()> {
        let model = model_io::read_model(self.doc, &model_io::Assets::default(), |_| None);
        let page_uuid = self.page_uuid(page)?;
        for &id in shapes {
            let Ok((_, _, m)) = self.find(&page_uuid, id) else { continue };
            let sheet = Self::sheet(&m);
            for (addr, value) in stale_cells(&model, page, id) {
                match addr {
                    Addr::Single(name) => {
                        // Keep a local formula; an inherited one stays inherited.
                        let local = Self::cell(&sheet, &name);
                        let f = match local.as_ref().and_then(|c| c.f.clone()) {
                            Some(f) => Some(f),
                            None => Some("Inh".to_owned()),
                        };
                        let u = local.and_then(|c| c.u);
                        Self::put_cells(&sheet, &[(name.as_str(), Cell { v: Some(value), f, u })])?;
                    }
                    Addr::Row { section, section_ix, row: Some(row), cell } => {
                        let local = local_row_cell(&sheet, &section, section_ix, &row, &cell);
                        let f = local.as_ref().and_then(|c| c.f.clone());
                        let u = local.and_then(|c| c.u);
                        Self::put_row_cell(&sheet, &section, section_ix, &row, &cell, Cell { v: Some(value), f, u })?;
                    }
                    Addr::Row { row: None, .. } => {}
                }
            }
        }
        Ok(())
    }

    fn next_id(&self, page_uuid: &str) -> u32 {
        self.nodes(page_uuid)
            .iter()
            .filter_map(|(_, u)| self.shape_map(u).and_then(|m| int(&m, "legacyId")))
            .max()
            .map_or(1, |m| m as u32 + 1)
    }

    fn new_uuid(&mut self) -> String {
        *self.next += 1;
        let n = *self.next;
        uuid(self.doc.peer_id(), b'n', (n >> 32) as u32, n as u32)
    }

    pub fn add_shape(&mut self, page: usize, kind: ShapeKind, x: f64, y: f64, w: f64, h: f64) -> Result<ChangeSummary> {
        let page_uuid = self.page_uuid(page)?;
        let id = self.next_id(&page_uuid);
        let uuid = self.new_uuid();
        write_new_shape(self.doc, &page_uuid, &new_shape(kind, id, x, y, w, h), &uuid);
        self.recalc(page, &[id])?;
        Ok(ChangeSummary { shapes: vec![id.to_string()], created: Some((page, id)), ..Default::default() })
    }

    pub fn move_shapes(&mut self, page: usize, ids: &[u32], dx: f64, dy: f64) -> Result<ChangeSummary> {
        let page_uuid = self.page_uuid(page)?;
        let model = model_io::read_model(self.doc, &model_io::Assets::default(), |_| None);
        let calc = flowster_core::calc::Calc::for_page(&model, &model.pages[page]);
        for &id in ids {
            let (_, _, m) = self.find(&page_uuid, id)?;
            let sheet = Self::sheet(&m);
            let num = |name: &str| Self::cell(&sheet, name).and_then(|c| c.v).and_then(|s| s.parse::<f64>().ok());
            if let (Some(bx), Some(by), Some(ex), Some(ey)) = (num("BeginX"), num("BeginY"), num("EndX"), num("EndY")) {
                // 1-D: move the ends; the pin follows through its formulas.
                Self::put_cells(
                    &sheet,
                    &[
                        ("BeginX", v(format_num(bx + dx))),
                        ("BeginY", v(format_num(by + dy))),
                        ("EndX", v(format_num(ex + dx))),
                        ("EndY", v(format_num(ey + dy))),
                    ],
                )?;
            } else {
                let pin = |name: &str| calc.cell(id, name).num().ok();
                let (px, py) = (pin("PinX").unwrap_or(0.0), pin("PinY").unwrap_or(0.0));
                Self::put_cells(&sheet, &[("PinX", v(format_num(px + dx))), ("PinY", v(format_num(py + dy)))])?;
            }
        }
        self.recalc(page, ids)?;
        Ok(ChangeSummary { shapes: ids.iter().map(u32::to_string).collect(), ..Default::default() })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_bounds(
        &mut self,
        page: usize,
        id: u32,
        pin_x: f64,
        pin_y: f64,
        w: f64,
        h: f64,
    ) -> Result<ChangeSummary> {
        let page_uuid = self.page_uuid(page)?;
        let (_, _, m) = self.find(&page_uuid, id)?;
        let sheet = Self::sheet(&m);
        // Explicit values replace any formula (as resizing in Visio does).
        Self::put_cells(
            &sheet,
            &[
                ("PinX", v(format_num(pin_x))),
                ("PinY", v(format_num(pin_y))),
                ("Width", v(format_num(w.max(0.0)))),
                ("Height", v(format_num(h.max(0.0)))),
            ],
        )?;
        self.recalc(page, &[id])?;
        Ok(ChangeSummary { shapes: vec![id.to_string()], ..Default::default() })
    }

    pub fn set_endpoints(&mut self, page: usize, id: u32, b: (f64, f64), e: (f64, f64)) -> Result<ChangeSummary> {
        let page_uuid = self.page_uuid(page)?;
        let (_, _, m) = self.find(&page_uuid, id)?;
        Self::put_cells(
            &Self::sheet(&m),
            &[
                ("BeginX", v(format_num(b.0))),
                ("BeginY", v(format_num(b.1))),
                ("EndX", v(format_num(e.0))),
                ("EndY", v(format_num(e.1))),
            ],
        )?;
        self.recalc(page, &[id])?;
        Ok(ChangeSummary { shapes: vec![id.to_string()], ..Default::default() })
    }

    pub fn delete_shapes(&mut self, page: usize, ids: &[u32]) -> Result<ChangeSummary> {
        let page_uuid = self.page_uuid(page)?;
        let tree = self.doc.get_tree(schema::shapes_tree_key(&page_uuid));
        let shapes = self.doc.get_map(schema::SHAPE);
        for &id in ids {
            let (node, _, _) = self.find(&page_uuid, id)?;
            // The node's subtree: its own and its children's shape maps go too.
            let mut stack = vec![node];
            while let Some(n) = stack.pop() {
                if let Some(u) = tree.get_meta(n).ok().and_then(|m| string(&m, "s")) {
                    shapes.delete(&u).map_err(failed)?;
                }
                stack.extend(tree.children(n).unwrap_or_default());
            }
            tree.delete(node).map_err(failed)?;
        }
        Ok(ChangeSummary { shapes: ids.iter().map(u32::to_string).collect(), ..Default::default() })
    }

    pub fn set_text(&mut self, page: usize, id: u32, text: &str) -> Result<ChangeSummary> {
        let page_uuid = self.page_uuid(page)?;
        let (_, _, m) = self.find(&page_uuid, id)?;
        let t = match m.get(schema::TEXT) {
            Some(ValueOrContainer::Container(Container::Text(t))) => t,
            _ => m.insert_container(schema::TEXT, LoroText::new()).map_err(failed)?,
        };
        let len = t.len_unicode();
        if len > 0 {
            t.delete(0, len).map_err(failed)?;
        }
        if !text.is_empty() {
            t.insert(0, text).map_err(failed)?;
        }
        Ok(ChangeSummary { shapes: vec![id.to_string()], ..Default::default() })
    }
}

fn new_named_row(rows: &LoroMap, name: &str) -> Result<LoroMap> {
    let pos = entries(rows).len() as i64;
    let m = rows.insert_container(name, LoroMap::new()).map_err(failed)?;
    m.insert("@pos", pos).map_err(failed)?;
    m.insert("@n", name).map_err(failed)?;
    Ok(m)
}

fn local_row_cell(sheet: &LoroMap, section: &str, ix: u32, row: &RowKey, cell: &str) -> Option<Cell> {
    let sections = child_map(sheet, "sections")?;
    let sm = entries(&sections).into_iter().find_map(|(_, v)| match v {
        ValueOrContainer::Container(Container::Map(m))
            if string(&m, "@n").as_deref() == Some(section) && int(&m, "@ix").unwrap_or(0) == i64::from(ix) =>
        {
            Some(m)
        }
        _ => None,
    })?;
    let rm = match (row, sm.get("rows")?) {
        (RowKey::Name(n), ValueOrContainer::Container(Container::Map(rows))) => child_map(&rows, n)?,
        (RowKey::Ix(rix), ValueOrContainer::Container(Container::MovableList(list))) => {
            (0..list.len()).find_map(|i| match list.get(i) {
                Some(ValueOrContainer::Container(Container::Map(m))) if int(&m, "@ix") == Some(i64::from(*rix)) => {
                    Some(m)
                }
                _ => None,
            })?
        }
        _ => return None,
    };
    model_io::value(&rm, cell).map(|x| cell_from(&x))
}
