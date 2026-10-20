//! Editing commands on the CRDT (SPEC §5.5, §20.6): each runs in the caller's
//! transaction, writes atomic cell groups, then recalculates the shapes it
//! touched so cached values follow their formulas in the same undo step.

use std::collections::HashMap;

use flowster_core::calc::{Addr, RowKey, format_num, stale_cells};
use flowster_core::command::{CellEdit, ChangeSummary, CommandError};
use flowster_core::library::{connector, glue_cells, glue_target};
use flowster_core::model::{Cell, Document as Model, Page, Row, Sheet};
use loro::{Container, LoroDoc, LoroMap, LoroMovableList, LoroText, LoroValue, TreeID, ValueOrContainer};

use crate::model_io::{
    self, cell_from, cell_value, child_map, entries, group_of, int, lazy_map, string, uuid, write_new_shape, write_row,
};
use crate::schema;

type Result<T> = std::result::Result<T, CommandError>;

fn failed(e: impl std::fmt::Display) -> CommandError {
    CommandError::Failed(e.to_string())
}

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

    /// Adds library shape `kind` (several shapes for a swimlane pool).
    pub fn add_shape(&mut self, page: usize, kind: &str, x: f64, y: f64, w: f64, h: f64) -> Result<ChangeSummary> {
        let page_uuid = self.page_uuid(page)?;
        let id = self.next_id(&page_uuid);
        let shapes = flowster_core::library::build(kind, id, x, y, w, h)
            .ok_or_else(|| failed(format!("unknown shape {kind}")))?;
        let ids: Vec<u32> = shapes.iter().map(|s| s.id).collect();
        for s in &shapes {
            let uuid = self.new_uuid();
            write_new_shape(self.doc, &page_uuid, s, &uuid);
        }
        if matches!(kind, "pool" | "lane") {
            // Swimlanes go to the back so the process drawn on them stays on top.
            let tree = self.doc.get_tree(schema::shapes_tree_key(&page_uuid));
            let roots = tree.roots();
            for (i, node) in roots[roots.len() - shapes.len()..].iter().enumerate() {
                tree.mov_to(*node, None::<TreeID>, i).map_err(failed)?;
            }
        }
        self.recalc(page, &ids)?;
        Ok(ChangeSummary {
            shapes: ids.iter().map(u32::to_string).collect(),
            created: Some((page, id)),
            ..Default::default()
        })
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
        self.reglue(page, Some(ids))?;
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
        self.reglue(page, Some(&[id]))?;
        Ok(ChangeSummary { shapes: vec![id.to_string()], ..Default::default() })
    }

    pub fn set_endpoints(
        &mut self,
        page: usize,
        id: u32,
        b: (f64, f64),
        e: (f64, f64),
        glue: (Option<u32>, Option<u32>),
    ) -> Result<ChangeSummary> {
        let page_uuid = self.page_uuid(page)?;
        let (_, _, m) = self.find(&page_uuid, id)?;
        let sheet = Self::sheet(&m);
        let is_connector = Self::cell(&sheet, "ObjType").and_then(|c| c.v).as_deref() == Some("2");
        if is_connector {
            // Rewrite the glue: triggers and the end formulas they drive.
            let mut local = Sheet::default();
            for name in ["BeginX", "BeginY", "EndX", "EndY", "BegTrigger", "EndTrigger"] {
                if let Some(c) = Self::cell(&sheet, name) {
                    local.cells.insert(name.into(), c);
                }
            }
            glue_cells(&mut local, glue.0, glue.1);
            for (end, value) in [("BeginX", b.0), ("BeginY", b.1), ("EndX", e.0), ("EndY", e.1)] {
                local.cells.entry(end.into()).or_default().v = Some(format_num(value));
            }
            let cells: Vec<(&str, Cell)> = local.cells.iter().map(|(k, c)| (k.as_str(), c.clone())).collect();
            Self::put_cells(&sheet, &cells)?;
            for trigger in ["BegTrigger", "EndTrigger"] {
                if !local.cells.contains_key(trigger)
                    && let Some(c) = child_map(&sheet, "cells")
                {
                    let _ = c.delete(trigger);
                }
            }
        } else {
            Self::put_cells(
                &sheet,
                &[
                    ("BeginX", v(format_num(b.0))),
                    ("BeginY", v(format_num(b.1))),
                    ("EndX", v(format_num(e.0))),
                    ("EndY", v(format_num(e.1))),
                ],
            )?;
        }
        self.reglue(page, Some(&[id]))?;
        self.recalc(page, &[id])?;
        Ok(ChangeSummary { shapes: vec![id.to_string()], ..Default::default() })
    }

    /// A dynamic connector between two shapes (or free points).
    pub fn add_connector(
        &mut self,
        page: usize,
        from: Option<u32>,
        to: Option<u32>,
        b: (f64, f64),
        e: (f64, f64),
    ) -> Result<ChangeSummary> {
        let page_uuid = self.page_uuid(page)?;
        let id = self.next_id(&page_uuid);
        let uuid = self.new_uuid();
        write_new_shape(self.doc, &page_uuid, &connector(id, b.0, b.1, e.0, e.1, from, to), &uuid);
        self.reglue(page, Some(&[id]))?;
        self.recalc(page, &[id])?;
        Ok(ChangeSummary { shapes: vec![id.to_string()], created: Some((page, id)), ..Default::default() })
    }

    /// Visio's AutoConnect: a copy of `id` one step in `direction`, connected to it.
    pub fn auto_connect(&mut self, page: usize, id: u32, direction: &str) -> Result<ChangeSummary> {
        let page_uuid = self.page_uuid(page)?;
        let model = model_io::read_model(self.doc, &model_io::Assets::default(), |_| None);
        let source =
            model.pages[page].shapes.iter().find(|s| s.id == id).ok_or_else(|| failed(format!("no shape {id}")))?;
        let rect = shape_rect(&model, page, id).ok_or_else(|| failed("the shape has no size"))?;
        if glue_target(&source.sheet, "BegTrigger").is_some() || source.sheet.cells.contains_key("BeginX") {
            return Err(failed("AutoConnect works on 2-D shapes"));
        }
        const GAP: f64 = 0.75;
        let (dx, dy) = match direction {
            "right" => (2.0 * rect.hw + GAP, 0.0),
            "left" => (-(2.0 * rect.hw + GAP), 0.0),
            "up" => (0.0, 2.0 * rect.hh + GAP),
            "down" => (0.0, -(2.0 * rect.hh + GAP)),
            other => return Err(failed(format!("unknown direction {other}"))),
        };
        let new_id = self.next_id(&page_uuid);
        let mut copy = source.clone();
        copy.id = new_id;
        copy.name_u = Some(format!("Sheet.{new_id}"));
        copy.name = Some(format!("Sheet.{new_id}"));
        copy.text = None;
        copy.shapes.clear();
        let calc = flowster_core::calc::Calc::for_page(&model, &model.pages[page]);
        let pin = |name: &str| calc.cell(id, name).num().unwrap_or(0.0);
        copy.sheet.cells.insert("PinX".into(), v(format_num(pin("PinX") + dx)));
        copy.sheet.cells.insert("PinY".into(), v(format_num(pin("PinY") + dy)));
        let uuid = self.new_uuid();
        write_new_shape(self.doc, &page_uuid, &copy, &uuid);
        self.recalc(page, &[new_id])?;
        let (cx, cy) = (rect.cx, rect.cy);
        let conn = self.next_id(&page_uuid);
        let cuuid = self.new_uuid();
        write_new_shape(
            self.doc,
            &page_uuid,
            &connector(conn, cx, cy, cx + dx, cy + dy, Some(id), Some(new_id)),
            &cuuid,
        );
        self.reglue(page, Some(&[conn]))?;
        self.recalc(page, &[conn])?;
        Ok(ChangeSummary {
            shapes: vec![new_id.to_string(), conn.to_string()],
            created: Some((page, new_id)),
            ..Default::default()
        })
    }

    /// Re-routes the connectors glued to any of `changed` (all connectors
    /// when `None`): ends on the facing sides of their shapes, right-angle path.
    fn reglue(&mut self, page: usize, changed: Option<&[u32]>) -> Result<()> {
        let page_uuid = self.page_uuid(page)?;
        let model = model_io::read_model(self.doc, &model_io::Assets::default(), |_| None);
        let Some(p) = model.pages.get(page) else { return Ok(()) };
        let mut touched = Vec::new();
        for s in &p.shapes {
            let (from, to) = (glue_target(&s.sheet, "BegTrigger"), glue_target(&s.sheet, "EndTrigger"));
            if from.is_none() && to.is_none() {
                continue;
            }
            if let Some(ids) = changed
                && !ids.contains(&s.id)
                && !from.is_some_and(|f| ids.contains(&f))
                && !to.is_some_and(|t| ids.contains(&t))
            {
                continue;
            }
            let num = |n: &str| s.sheet.cells.get(n).and_then(|c| c.v.as_deref()).and_then(|v| v.parse::<f64>().ok());
            let (Some(bx), Some(by), Some(ex), Some(ey)) = (num("BeginX"), num("BeginY"), num("EndX"), num("EndY"))
            else {
                continue;
            };
            let a = from.and_then(|t| shape_rect(&model, page, t));
            let b = to.and_then(|t| shape_rect(&model, page, t));
            let points = route(a, b, (bx, by), (ex, ey));
            let (Some(&first), Some(&last)) = (points.first(), points.last()) else { continue };
            let (_, _, m) = self.find(&page_uuid, s.id)?;
            let sheet = Self::sheet(&m);
            let keep = |name: &str, value: f64| {
                let f = s.sheet.cells.get(name).and_then(|c| c.f.clone());
                Cell { v: Some(format_num(value)), f, u: None }
            };
            Self::put_cells(
                &sheet,
                &[
                    ("BeginX", keep("BeginX", first.0)),
                    ("BeginY", keep("BeginY", first.1)),
                    ("EndX", keep("EndX", last.0)),
                    ("EndY", keep("EndY", last.1)),
                ],
            )?;
            let rows: Vec<Row> = points
                .iter()
                .enumerate()
                .map(|(i, &(x, y))| Row {
                    t: Some(if i == 0 { "MoveTo" } else { "LineTo" }.into()),
                    ix: Some(i as u32 + 1),
                    cells: [("X".to_owned(), v(format_num(x - first.0))), ("Y".to_owned(), v(format_num(y - first.1)))]
                        .into(),
                    ..Default::default()
                })
                .collect();
            Self::replace_rows(&sheet, "Geometry", 0, &rows)?;
            touched.push(s.id);
        }
        if !touched.is_empty() {
            self.recalc(page, &touched)?;
        }
        Ok(())
    }

    /// Replaces every row of an indexed section (a connector's path).
    fn replace_rows(sheet: &LoroMap, section: &str, ix: u32, rows: &[Row]) -> Result<()> {
        let sections = lazy_map(sheet, "sections");
        let found = entries(&sections).into_iter().find_map(|(_, v)| match v {
            ValueOrContainer::Container(Container::Map(m))
                if string(&m, "@n").as_deref() == Some(section) && int(&m, "@ix").unwrap_or(0) == i64::from(ix) =>
            {
                Some(m)
            }
            _ => None,
        });
        let Some(sm) = found else { return Ok(()) };
        let list = sm.insert_container("rows", LoroMovableList::new()).map_err(failed)?;
        for (i, r) in rows.iter().enumerate() {
            write_row(&list.push_container(LoroMap::new()).map_err(failed)?, r, i);
        }
        Ok(())
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

    /// Sets cells on shapes (formatting). A row cell without a row index goes
    /// to every row of that section the shape has, so a whole text changes.
    pub fn set_cells(&mut self, page: usize, ids: &[u32], edits: &[CellEdit]) -> Result<ChangeSummary> {
        let page_uuid = self.page_uuid(page)?;
        let model = model_io::read_model(self.doc, &model_io::Assets::default(), |_| None);
        let page_model = model.pages.get(page).ok_or_else(|| failed(format!("no page {page}")))?;
        for &id in ids {
            let (_, _, m) = self.find(&page_uuid, id)?;
            let sheet = Self::sheet(&m);
            let shape = page_model.shapes.iter().find(|s| s.id == id);
            for e in edits {
                let cell = Cell { v: Some(e.value.clone()), f: None, u: None };
                match &e.section {
                    None => Self::put_cells(&sheet, &[(e.cell.as_str(), cell)])?,
                    Some(section) => {
                        let rows: Vec<u32> = match e.row {
                            Some(r) => vec![r],
                            None => {
                                let resolved = shape.map(|s| {
                                    flowster_core::resolve::Resolved::new(&model, s, None)
                                        .sections(section)
                                        .into_iter()
                                        .flat_map(|sec| sec.rows.into_iter().filter_map(|r| r.ix))
                                        .collect::<Vec<_>>()
                                });
                                match resolved {
                                    Some(ixs) if !ixs.is_empty() => ixs,
                                    _ => vec![0],
                                }
                            }
                        };
                        for r in rows {
                            Self::put_row_cell(&sheet, section, 0, &RowKey::Ix(r), &e.cell, cell.clone())?;
                        }
                    }
                }
            }
        }
        self.recalc(page, ids)?;
        self.reglue(page, Some(ids))?;
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

/// A shape's axis-aligned page bounds: centre and half extents.
#[derive(Clone, Copy, Debug)]
struct Rect {
    cx: f64,
    cy: f64,
    hw: f64,
    hh: f64,
}

fn shape_rect(model: &Model, page: usize, id: u32) -> Option<Rect> {
    let p = model.pages.get(page)?;
    p.shapes.iter().find(|s| s.id == id)?;
    let calc = flowster_core::calc::Calc::for_page(model, p);
    let n = |name: &str| calc.cell(id, name).num().ok();
    let (w, h) = (n("Width")?, n("Height")?);
    let t = flowster_core::transform::Affine::shape_local_to_parent(
        (n("PinX")?, n("PinY")?),
        (n("LocPinX").unwrap_or(w / 2.0), n("LocPinY").unwrap_or(h / 2.0)),
        n("Angle").unwrap_or(0.0),
        false,
        false,
    );
    let pts = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|q| t.apply(q));
    let (x0, x1) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (y0, y1) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
    Some(Rect { cx: (x0 + x1) / 2.0, cy: (y0 + y1) / 2.0, hw: (x1 - x0) / 2.0, hh: (y1 - y0) / 2.0 })
}

/// The side midpoint of `r` facing `toward`, and whether that side is left/right.
fn facing_side(r: Rect, toward: (f64, f64)) -> ((f64, f64), bool) {
    let (dx, dy) = (toward.0 - r.cx, toward.1 - r.cy);
    // Compare in units of the rectangle's own extents, so wide shapes prefer
    // their long sides only when the target really lies beyond them.
    if dx.abs() * r.hh.max(1e-6) >= dy.abs() * r.hw.max(1e-6) {
        ((r.cx + r.hw * dx.signum(), r.cy), true)
    } else {
        ((r.cx, r.cy + r.hh * dy.signum()), false)
    }
}

/// A right-angle path between two (optional) glued shapes; free ends stay put.
fn route(a: Option<Rect>, b: Option<Rect>, p0: (f64, f64), p1: (f64, f64)) -> Vec<(f64, f64)> {
    match (a, b) {
        (Some(a), Some(b)) => {
            let (start, horizontal) = facing_side(a, (b.cx, b.cy));
            let (end, _) = facing_side(b, (a.cx, a.cy));
            if horizontal {
                if (start.1 - end.1).abs() < 1e-6 {
                    return vec![start, end];
                }
                let mx = (start.0 + end.0) / 2.0;
                vec![start, (mx, start.1), (mx, end.1), end]
            } else {
                if (start.0 - end.0).abs() < 1e-6 {
                    return vec![start, end];
                }
                let my = (start.1 + end.1) / 2.0;
                vec![start, (start.0, my), (end.0, my), end]
            }
        }
        (Some(a), None) => {
            let (start, horizontal) = facing_side(a, p1);
            let corner = if horizontal { (p1.0, start.1) } else { (start.0, p1.1) };
            vec![start, corner, p1]
        }
        (None, Some(b)) => {
            let (end, horizontal) = facing_side(b, p0);
            let corner = if horizontal { (p0.0, end.1) } else { (end.0, p0.1) };
            vec![p0, corner, end]
        }
        (None, None) => vec![p0, p1],
    }
}
