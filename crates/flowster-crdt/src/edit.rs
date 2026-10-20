//! Editing commands on the CRDT (SPEC §5.5, §20.6): each runs in the caller's
//! transaction, writes atomic cell groups, then recalculates the shapes it
//! touched so cached values follow their formulas in the same undo step.

use std::collections::HashMap;

use flowster_core::calc::{Addr, RowKey, format_num, stale_cells};
use flowster_core::command::{CellEdit, ChangeSummary, CommandError};
use flowster_core::library::{connector, glue_cells, glue_target};
use flowster_core::model::{Cell, Document as Model, Page, Row, Shape, ShapeType, Sheet};
use flowster_core::transform::Affine;
use loro::{Container, LoroDoc, LoroMap, LoroMovableList, LoroText, LoroTree, LoroValue, TreeID, ValueOrContainer};

use crate::model_io::{
    self, cell_from, cell_value, child_map, entries, group_of, int, lazy_map, string, uuid, write_new_shape, write_row,
    write_sheet,
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

fn bit(b: bool) -> Cell {
    v(if b { "1" } else { "0" })
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
        let moves: Vec<(u32, f64, f64)> = ids.iter().map(|&id| (id, dx, dy)).collect();
        self.translate(page, &moves)
    }

    /// Moves each shape by its own offset, then re-routes their connectors.
    fn translate(&mut self, page: usize, moves: &[(u32, f64, f64)]) -> Result<ChangeSummary> {
        self.shift(page, moves)?;
        let ids: Vec<u32> = moves.iter().map(|m| m.0).collect();
        self.recalc(page, &ids)?;
        self.reglue(page, Some(&ids))?;
        Ok(ChangeSummary { shapes: ids.iter().map(u32::to_string).collect(), ..Default::default() })
    }

    /// Writes moved positions: a 1-D shape's ends, a 2-D shape's pin.
    fn shift(&self, page: usize, moves: &[(u32, f64, f64)]) -> Result<()> {
        let page_uuid = self.page_uuid(page)?;
        let model = model_io::read_model(self.doc, &model_io::Assets::default(), |_| None);
        let calc = flowster_core::calc::Calc::for_page(&model, &model.pages[page]);
        for &(id, dx, dy) in moves {
            let (_, _, m) = self.find(&page_uuid, id)?;
            let sheet = Self::sheet(&m);
            if let Some((bx, by, ex, ey)) = Self::ends(&sheet) {
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
        Ok(())
    }

    /// A 1-D shape's local end points.
    fn ends(sheet: &LoroMap) -> Option<(f64, f64, f64, f64)> {
        let num = |name: &str| Self::cell(sheet, name).and_then(|c| c.v).and_then(|s| s.parse::<f64>().ok());
        Some((num("BeginX")?, num("BeginY")?, num("EndX")?, num("EndY")?))
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
        // A glue target inside a group moves with its top-level group.
        let moved = |ids: &[u32], t: u32| ids.contains(&t) || top_of(p, t).is_some_and(|a| ids.contains(&a));
        let mut touched = Vec::new();
        for s in &p.shapes {
            let (from, to) = (glue_target(&s.sheet, "BegTrigger"), glue_target(&s.sheet, "EndTrigger"));
            if from.is_none() && to.is_none() {
                continue;
            }
            if let Some(ids) = changed
                && !ids.contains(&s.id)
                && !from.is_some_and(|f| moved(ids, f))
                && !to.is_some_and(|t| moved(ids, t))
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
    // ---------------------------------------------------------------- arrange

    fn rects(&self, model: &Model, page: usize, ids: &[u32]) -> Result<Vec<Rect>> {
        ids.iter().map(|&id| shape_rect(model, page, id).ok_or_else(|| failed(format!("no shape {id}")))).collect()
    }

    /// Lines shapes up with the first one's edge or centre.
    pub fn align(&mut self, page: usize, ids: &[u32], how: &str) -> Result<ChangeSummary> {
        if ids.len() < 2 {
            return Err(failed("select two or more shapes to align"));
        }
        let model = model_io::read_model(self.doc, &model_io::Assets::default(), |_| None);
        let rects = self.rects(&model, page, ids)?;
        let p = rects[0];
        let mut moves = Vec::new();
        for (&id, r) in ids.iter().zip(&rects).skip(1) {
            let (dx, dy) = match how {
                "left" => ((p.cx - p.hw) - (r.cx - r.hw), 0.0),
                "center" => (p.cx - r.cx, 0.0),
                "right" => ((p.cx + p.hw) - (r.cx + r.hw), 0.0),
                "top" => (0.0, (p.cy + p.hh) - (r.cy + r.hh)),
                "middle" => (0.0, p.cy - r.cy),
                "bottom" => (0.0, (p.cy - p.hh) - (r.cy - r.hh)),
                other => return Err(failed(format!("unknown alignment {other}"))),
            };
            moves.push((id, dx, dy));
        }
        self.translate(page, &moves)
    }

    /// Equal gaps between shapes along an axis; the outermost two stay put.
    pub fn distribute(&mut self, page: usize, ids: &[u32], axis: &str) -> Result<ChangeSummary> {
        if ids.len() < 3 {
            return Err(failed("select three or more shapes to distribute"));
        }
        let horizontal = match axis {
            "horizontal" => true,
            "vertical" => false,
            other => return Err(failed(format!("unknown axis {other}"))),
        };
        let model = model_io::read_model(self.doc, &model_io::Assets::default(), |_| None);
        let rects = self.rects(&model, page, ids)?;
        // Each shape's centre and half extent along the axis.
        let mut items: Vec<(u32, f64, f64)> = ids
            .iter()
            .zip(&rects)
            .map(|(&id, r)| if horizontal { (id, r.cx, r.hw) } else { (id, r.cy, r.hh) })
            .collect();
        items.sort_by(|a, b| a.1.total_cmp(&b.1));
        let (first, last) = (items[0], items[items.len() - 1]);
        let (start, end) = (first.1 - first.2, last.1 + last.2);
        let sizes: f64 = items.iter().map(|i| 2.0 * i.2).sum();
        let gap = (end - start - sizes) / (items.len() - 1) as f64;
        let mut at = start;
        let mut moves = Vec::new();
        for (id, c, h) in items {
            let d = at + h - c;
            moves.push(if horizontal { (id, d, 0.0) } else { (id, 0.0, d) });
            at += 2.0 * h + gap;
        }
        self.translate(page, &moves)
    }

    /// A quarter turn about the selection's centre.
    pub fn rotate(&mut self, page: usize, ids: &[u32], direction: &str) -> Result<ChangeSummary> {
        let turn = match direction {
            "left" => std::f64::consts::FRAC_PI_2,
            "right" => -std::f64::consts::FRAC_PI_2,
            other => return Err(failed(format!("unknown direction {other}"))),
        };
        self.orient(page, ids, Affine::rotate(turn), |angle, fx, fy| (angle + turn, fx, fy))
    }

    /// A mirror image about the selection's centre line.
    pub fn flip(&mut self, page: usize, ids: &[u32], axis: &str) -> Result<ChangeSummary> {
        match axis {
            "horizontal" => self.orient(page, ids, Affine::scale(-1.0, 1.0), |angle, fx, fy| (-angle, !fx, fy)),
            "vertical" => self.orient(page, ids, Affine::scale(1.0, -1.0), |angle, fx, fy| (-angle, fx, !fy)),
            other => Err(failed(format!("unknown axis {other}"))),
        }
    }

    /// Applies `turn` about the selection's centre: a 1-D shape's ends move; a
    /// 2-D shape's pin moves and `cells` gives its new Angle and flips.
    fn orient(
        &mut self,
        page: usize,
        ids: &[u32],
        turn: Affine,
        cells: impl Fn(f64, bool, bool) -> (f64, bool, bool),
    ) -> Result<ChangeSummary> {
        if ids.is_empty() {
            return Ok(ChangeSummary::default());
        }
        let page_uuid = self.page_uuid(page)?;
        let model = model_io::read_model(self.doc, &model_io::Assets::default(), |_| None);
        let (x0, y0, x1, y1) = union(&self.rects(&model, page, ids)?);
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let t = Affine::translate(cx, cy).then(turn).then(Affine::translate(-cx, -cy));
        let calc = flowster_core::calc::Calc::for_page(&model, &model.pages[page]);
        for &id in ids {
            let (_, _, m) = self.find(&page_uuid, id)?;
            let sheet = Self::sheet(&m);
            if let Some((bx, by, ex, ey)) = Self::ends(&sheet) {
                Self::put_ends(&sheet, t.apply((bx, by)), t.apply((ex, ey)))?;
            } else {
                let n = |name: &str| calc.cell(id, name).num().ok();
                let pin = t.apply((n("PinX").unwrap_or(0.0), n("PinY").unwrap_or(0.0)));
                let flag = |name: &str| n(name).is_some_and(|v| v != 0.0);
                let (angle, fx, fy) = cells(n("Angle").unwrap_or(0.0), flag("FlipX"), flag("FlipY"));
                Self::put_cells(
                    &sheet,
                    &[
                        ("PinX", v(format_num(pin.0))),
                        ("PinY", v(format_num(pin.1))),
                        ("Angle", v(format_num(normal_angle(angle)))),
                        ("FlipX", bit(fx)),
                        ("FlipY", bit(fy)),
                    ],
                )?;
            }
        }
        self.recalc(page, ids)?;
        self.reglue(page, Some(ids))?;
        Ok(ChangeSummary { shapes: ids.iter().map(u32::to_string).collect(), ..Default::default() })
    }

    fn put_ends(sheet: &LoroMap, b: (f64, f64), e: (f64, f64)) -> Result<()> {
        Self::put_cells(
            sheet,
            &[
                ("BeginX", v(format_num(b.0))),
                ("BeginY", v(format_num(b.1))),
                ("EndX", v(format_num(e.0))),
                ("EndY", v(format_num(e.1))),
            ],
        )
    }

    /// The legacy id of a tree node's shape.
    fn legacy_of(&self, tree: &LoroTree, node: TreeID) -> Option<u32> {
        let s = tree.get_meta(node).ok().and_then(|m| string(&m, "s"))?;
        self.shape_map(&s).and_then(|m| int(&m, "legacyId")).map(|i| i as u32)
    }

    /// The page's top-level nodes among `ids`, back to front.
    fn picked(&self, tree: &LoroTree, ids: &[u32]) -> Vec<(TreeID, u32)> {
        tree.roots()
            .into_iter()
            .filter_map(|n| self.legacy_of(tree, n).filter(|id| ids.contains(id)).map(|id| (n, id)))
            .collect()
    }

    /// Stacking order: to the front or back, or one step either way.
    pub fn reorder(&mut self, page: usize, ids: &[u32], to: &str) -> Result<ChangeSummary> {
        let page_uuid = self.page_uuid(page)?;
        let tree = model_io::tree(self.doc, &schema::shapes_tree_key(&page_uuid));
        let picked: Vec<TreeID> = self.picked(&tree, ids).into_iter().map(|p| p.0).collect();
        let pos = |n: TreeID, roots: &[TreeID]| roots.iter().position(|r| *r == n);
        match to {
            "front" => {
                for &n in &picked {
                    if let Some(&last) = tree.roots().last()
                        && last != n
                    {
                        tree.mov_after(n, last).map_err(failed)?;
                    }
                }
            }
            "back" => {
                for &n in picked.iter().rev() {
                    if let Some(&first) = tree.roots().first()
                        && first != n
                    {
                        tree.mov_before(n, first).map_err(failed)?;
                    }
                }
            }
            "forward" => {
                for &n in picked.iter().rev() {
                    let roots = tree.roots();
                    if let Some(&next) = pos(n, &roots).and_then(|i| roots.get(i + 1))
                        && !picked.contains(&next)
                    {
                        tree.mov_after(n, next).map_err(failed)?;
                    }
                }
            }
            "backward" => {
                for &n in &picked {
                    let roots = tree.roots();
                    if let Some(&prev) = pos(n, &roots).and_then(|i| i.checked_sub(1)).and_then(|i| roots.get(i))
                        && !picked.contains(&prev)
                    {
                        tree.mov_before(n, prev).map_err(failed)?;
                    }
                }
            }
            other => return Err(failed(format!("unknown order {other}"))),
        }
        Ok(ChangeSummary { shapes: ids.iter().map(u32::to_string).collect(), ..Default::default() })
    }

    /// A new group around top-level shapes, stacked where the topmost one was.
    /// Members keep their look: their positions become group-local.
    pub fn group(&mut self, page: usize, ids: &[u32]) -> Result<ChangeSummary> {
        if ids.len() < 2 {
            return Err(failed("select two or more shapes to group"));
        }
        let page_uuid = self.page_uuid(page)?;
        let tree = model_io::tree(self.doc, &schema::shapes_tree_key(&page_uuid));
        let members = self.picked(&tree, ids);
        if members.len() != ids.len() {
            return Err(failed("only shapes on the page itself can be grouped"));
        }
        let model = model_io::read_model(self.doc, &model_io::Assets::default(), |_| None);
        let (x0, y0, x1, y1) = union(&self.rects(&model, page, ids)?);
        let (w, h) = (x1 - x0, y1 - y0);
        // The group's local origin is its bottom-left corner.
        let moves: Vec<(u32, f64, f64)> = members.iter().map(|&(_, id)| (id, -x0, -y0)).collect();
        self.shift(page, &moves)?;

        let gid = self.next_id(&page_uuid);
        let mut sheet = Sheet::default();
        for (name, value) in
            [("PinX", x0 + w / 2.0), ("PinY", y0 + h / 2.0), ("Width", w), ("Height", h), ("Angle", 0.0)]
        {
            sheet.cells.insert(name.into(), v(format_num(value)));
        }
        for (name, formula, value) in [("LocPinX", "Width*0.5", w / 2.0), ("LocPinY", "Height*0.5", h / 2.0)] {
            sheet.cells.insert(name.into(), Cell { v: Some(format_num(value)), f: Some(formula.into()), u: None });
        }
        let shape = Shape {
            id: gid,
            kind: ShapeType::Group,
            name_u: Some(format!("Sheet.{gid}")),
            name: Some(format!("Sheet.{gid}")),
            sheet,
            ..Default::default()
        };
        let uuid = self.new_uuid();
        write_new_shape(self.doc, &page_uuid, &shape, &uuid);
        let group = *tree.roots().last().ok_or_else(|| failed("the group was not created"))?;
        tree.mov_after(group, members[members.len() - 1].0).map_err(failed)?;
        for (i, &(node, _)) in members.iter().enumerate() {
            tree.mov_to(node, Some(group), i).map_err(failed)?;
        }
        let mut touched = ids.to_vec();
        touched.push(gid);
        self.recalc(page, &touched)?;
        self.reglue(page, Some(&[gid]))?;
        Ok(ChangeSummary {
            shapes: touched.iter().map(u32::to_string).collect(),
            created: Some((page, gid)),
            ..Default::default()
        })
    }

    /// Breaks groups apart: members return to the page where the group was
    /// stacked, in page coordinates; the group shapes go.
    pub fn ungroup(&mut self, page: usize, ids: &[u32]) -> Result<ChangeSummary> {
        let page_uuid = self.page_uuid(page)?;
        let model = model_io::read_model(self.doc, &model_io::Assets::default(), |_| None);
        let p = model.pages.get(page).ok_or_else(|| failed(format!("no page {page}")))?;
        let calc = flowster_core::calc::Calc::for_page(&model, p);
        let n = |id: u32, name: &str| calc.cell(id, name).num().ok();
        let flag = |id: u32, name: &str| n(id, name).is_some_and(|v| v != 0.0);
        let tree = model_io::tree(self.doc, &schema::shapes_tree_key(&page_uuid));
        let shapes = self.doc.get_map(schema::SHAPE);
        let mut freed = Vec::new();
        for &gid in ids {
            let Some(g) = p.shapes.iter().find(|s| s.id == gid) else { continue };
            if g.kind != ShapeType::Group || g.shapes.is_empty() {
                continue;
            }
            if g.master.is_some() {
                return Err(failed("ungrouping a master shape is not supported yet"));
            }
            let (t, _, _) = page_frame(&model, page, gid).ok_or_else(|| failed("the group has no size"))?;
            let (g_angle, g_fx, g_fy) = (n(gid, "Angle").unwrap_or(0.0), flag(gid, "FlipX"), flag(gid, "FlipY"));
            let (gnode, guuid, _) = self.find(&page_uuid, gid)?;
            for child in &g.shapes {
                let id = child.id;
                let (node, _, m) = self.find(&page_uuid, id)?;
                let sheet = Self::sheet(&m);
                if let Some((bx, by, ex, ey)) = Self::ends(&sheet) {
                    Self::put_ends(&sheet, t.apply((bx, by)), t.apply((ex, ey)))?;
                } else {
                    let pin = t.apply((n(id, "PinX").unwrap_or(0.0), n(id, "PinY").unwrap_or(0.0)));
                    // R(g)·F·R(a) = R(g ± a)·F: a single flip mirrors the turn.
                    let a = n(id, "Angle").unwrap_or(0.0);
                    let angle = if g_fx != g_fy { g_angle - a } else { g_angle + a };
                    // Values replace formulas that referred to the group.
                    Self::put_cells(
                        &sheet,
                        &[
                            ("PinX", v(format_num(pin.0))),
                            ("PinY", v(format_num(pin.1))),
                            ("Width", v(format_num(n(id, "Width").unwrap_or(0.0)))),
                            ("Height", v(format_num(n(id, "Height").unwrap_or(0.0)))),
                            ("Angle", v(format_num(normal_angle(angle)))),
                            ("FlipX", bit(g_fx != flag(id, "FlipX"))),
                            ("FlipY", bit(g_fy != flag(id, "FlipY"))),
                        ],
                    )?;
                }
                tree.mov_before(node, gnode).map_err(failed)?;
                freed.push(id);
            }
            shapes.delete(&guuid).map_err(failed)?;
            tree.delete(gnode).map_err(failed)?;
        }
        if freed.is_empty() {
            return Err(failed("select a group to ungroup"));
        }
        self.recalc(page, &freed)?;
        self.reglue(page, Some(&freed))?;
        Ok(ChangeSummary { shapes: freed.iter().map(u32::to_string).collect(), ..Default::default() })
    }

    // ------------------------------------------------------------------ pages

    /// A blank foreground page with the page setup of the page before it.
    pub fn add_page(&mut self, after: Option<usize>) -> Result<ChangeSummary> {
        let model = model_io::read_model(self.doc, &model_io::Assets::default(), |_| None);
        let at = match after {
            Some(i) if i < model.pages.len() => i + 1,
            _ => model.pages.iter().rposition(|p| !p.background).map_or(model.pages.len(), |i| i + 1),
        };
        let before = at.checked_sub(1);
        let source = before.and_then(|i| model.pages.get(i)).or(model.pages.first());
        let mut cells = source.map_or_else(|| blank_model().pages[0].sheet.cells.clone(), |p| p.sheet.cells.clone());
        cells.remove("UIVisibility");
        // A new foreground page shares the background of the page before it.
        let back_page = match before {
            Some(i) if !model.pages[i].background => {
                string(&self.doc.get_map(schema::page_key(&self.page_uuid(i)?)), "backPage")
            }
            _ => None,
        };
        let id = model.pages.iter().map(|p| p.id).max().map_or(0, |m| m + 1);
        let taken = |name: &str| {
            model.pages.iter().any(|p| p.name.as_deref() == Some(name) || p.name_u.as_deref() == Some(name))
        };
        let mut n = model.pages.iter().filter(|p| !p.background).count() + 1;
        while taken(&format!("Page-{n}")) {
            n += 1;
        }
        let name = format!("Page-{n}");

        let uuid = self.new_uuid();
        self.doc.get_movable_list(schema::PAGES).insert(at, uuid.as_str()).map_err(failed)?;
        let m = self.doc.get_map(schema::page_key(&uuid));
        m.insert("legacyId", i64::from(id)).map_err(failed)?;
        m.insert("name", name.as_str()).map_err(failed)?;
        m.insert("nameU", name.as_str()).map_err(failed)?;
        m.insert("isBackground", false).map_err(failed)?;
        if let Some(b) = back_page {
            m.insert("backPage", b).map_err(failed)?;
        }
        write_sheet(
            &m.insert_container("pageSheet", LoroMap::new()).map_err(failed)?,
            &Sheet { cells, ..Default::default() },
        );
        m.insert("connects", Vec::<LoroValue>::new()).map_err(failed)?;
        Ok(ChangeSummary { page: Some(at), ..Default::default() })
    }

    /// Removes a page and its shapes; pages that used it as their background lose it.
    pub fn delete_page(&mut self, page: usize) -> Result<ChangeSummary> {
        let model = model_io::read_model(self.doc, &model_io::Assets::default(), |_| None);
        let target = model.pages.get(page).ok_or_else(|| failed(format!("no page {page}")))?;
        if !target.background && model.pages.iter().filter(|p| !p.background).count() == 1 {
            return Err(failed("a drawing needs at least one foreground page"));
        }
        let page_uuid = self.page_uuid(page)?;
        let pages = self.doc.get_movable_list(schema::PAGES);
        for i in 0..pages.len() {
            let other = self.doc.get_map(schema::page_key(&self.page_uuid(i)?));
            if string(&other, "backPage").as_deref() == Some(page_uuid.as_str()) {
                other.delete("backPage").map_err(failed)?;
            }
        }
        let shapes = self.doc.get_map(schema::SHAPE);
        for (_, u) in self.nodes(&page_uuid) {
            shapes.delete(&u).map_err(failed)?;
        }
        let tree = self.doc.get_tree(schema::shapes_tree_key(&page_uuid));
        for root in tree.roots() {
            tree.delete(root).map_err(failed)?;
        }
        pages.delete(page, 1).map_err(failed)?;
        self.doc.get_map(schema::page_key(&page_uuid)).clear().map_err(failed)?;
        // The nearest visible page; those after `page` move down one place.
        let next = visible_page(&model, page).map_or(0, |i| if i > page { i - 1 } else { i });
        Ok(ChangeSummary { page: Some(next), ..Default::default() })
    }

    pub fn rename_page(&mut self, page: usize, name: &str) -> Result<ChangeSummary> {
        let name = name.trim();
        if name.is_empty() {
            return Err(failed("a page needs a name"));
        }
        let model = model_io::read_model(self.doc, &model_io::Assets::default(), |_| None);
        if model.pages.iter().enumerate().any(|(i, p)| i != page && p.name.as_deref() == Some(name)) {
            return Err(failed(format!("there is already a page named {name}")));
        }
        self.doc.get_map(schema::page_key(&self.page_uuid(page)?)).insert("name", name).map_err(failed)?;
        Ok(ChangeSummary { page: Some(page), ..Default::default() })
    }

    /// Hides a page from the tabs (Visio's `UIVisibility`), or shows it again.
    pub fn set_page_hidden(&mut self, page: usize, hidden: bool) -> Result<ChangeSummary> {
        let model = model_io::read_model(self.doc, &model_io::Assets::default(), |_| None);
        let target = model.pages.get(page).ok_or_else(|| failed(format!("no page {page}")))?;
        let next = if hidden { visible_page(&model, page) } else { Some(page) };
        if hidden && !target.background && next.is_none() {
            return Err(failed("at least one foreground page must stay visible"));
        }
        let m = self.doc.get_map(schema::page_key(&self.page_uuid(page)?));
        Self::put_cells(&lazy_map(&m, "pageSheet"), &[("UIVisibility", bit(hidden))])?;
        Ok(ChangeSummary { page: Some(next.unwrap_or(page)), ..Default::default() })
    }
    // -------------------------------------------------------------- clipboard

    /// Pastes copied shapes: new ids throughout, moved by `(dx, dy)`, glue
    /// kept between pasted shapes and dropped to anything else.
    pub fn insert_shapes(&mut self, page: usize, shapes: &[Shape], dx: f64, dy: f64) -> Result<ChangeSummary> {
        if shapes.is_empty() {
            return Ok(ChangeSummary::default());
        }
        let page_uuid = self.page_uuid(page)?;
        // Old id → new id, for every shape and sub-shape.
        let mut ids: HashMap<u32, u32> = HashMap::new();
        let mut next = self.next_id(&page_uuid);
        fn number(list: &[Shape], ids: &mut HashMap<u32, u32>, next: &mut u32) {
            for s in list {
                ids.insert(s.id, *next);
                *next += 1;
                number(&s.shapes, ids, next);
            }
        }
        number(shapes, &mut ids, &mut next);
        fn renumber(s: &mut Shape, ids: &HashMap<u32, u32>) {
            s.id = ids[&s.id];
            // `Sheet.5` → `Sheet.12`, `Process` → `Process.12`.
            let rename = |n: &Option<String>| {
                n.as_ref().map(|n| {
                    let base = match n.rsplit_once('.') {
                        Some((b, d)) if !d.is_empty() && d.bytes().all(|c| c.is_ascii_digit()) => b,
                        _ => n.as_str(),
                    };
                    format!("{base}.{}", s.id)
                })
            };
            (s.name_u, s.name) = (rename(&s.name_u), rename(&s.name));
            for c in &mut s.shapes {
                renumber(c, ids);
            }
        }
        let mut pasted: Vec<Shape> = shapes.to_vec();
        let num = |sheet: &Sheet, name: &str| {
            sheet.cells.get(name).and_then(|c| c.v.as_deref()).and_then(|v| v.parse::<f64>().ok())
        };
        for s in &mut pasted {
            let glue = (glue_target(&s.sheet, "BegTrigger"), glue_target(&s.sheet, "EndTrigger"));
            renumber(s, &ids);
            if glue.0.is_some() || glue.1.is_some() {
                glue_cells(
                    &mut s.sheet,
                    glue.0.and_then(|t| ids.get(&t).copied()),
                    glue.1.and_then(|t| ids.get(&t).copied()),
                );
            }
            let ends = (num(&s.sheet, "BeginX"), num(&s.sheet, "BeginY"), num(&s.sheet, "EndX"), num(&s.sheet, "EndY"));
            let moves: Vec<(&str, f64)> = match ends {
                (Some(_), Some(_), Some(_), Some(_)) => {
                    vec![("BeginX", dx), ("BeginY", dy), ("EndX", dx), ("EndY", dy)]
                }
                _ => vec![("PinX", dx), ("PinY", dy)],
            };
            for (name, d) in moves {
                if let Some(value) = num(&s.sheet, name) {
                    // A moved value replaces its formula, as dragging does.
                    let cell = s.sheet.cells.entry(name.into()).or_default();
                    let keep = matches!(name, "BeginX" | "BeginY" | "EndX" | "EndY")
                        && cell.f.as_deref().is_some_and(|f| f.starts_with("_WALKGLUE"));
                    cell.v = Some(format_num(value + d));
                    if !keep {
                        cell.f = None;
                    }
                }
            }
        }
        let uuids: HashMap<u32, String> = ids.values().map(|&id| (id, self.new_uuid())).collect();
        model_io::write_new_shapes(self.doc, &page_uuid, &pasted, &|id| uuids[&id].clone());
        let top: Vec<u32> = pasted.iter().map(|s| s.id).collect();
        let all: Vec<u32> = ids.values().copied().collect();
        self.recalc(page, &all)?;
        self.reglue(page, Some(&top))?;
        Ok(ChangeSummary { shapes: top.iter().map(u32::to_string).collect(), ..Default::default() })
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
    let (t, w, h) = page_frame(model, page, id)?;
    let pts = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|q| t.apply(q));
    let (x0, x1) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (y0, y1) = pts.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
    Some(Rect { cx: (x0 + x1) / 2.0, cy: (y0 + y1) / 2.0, hw: (x1 - x0) / 2.0, hh: (y1 - y0) / 2.0 })
}

/// A shape's local → parent transform and size.
fn local_frame(calc: &flowster_core::calc::Calc, id: u32) -> Option<(Affine, f64, f64)> {
    let n = |name: &str| calc.cell(id, name).num().ok();
    let (w, h) = (n("Width")?, n("Height")?);
    let flip = |name: &str| n(name).is_some_and(|v| v != 0.0);
    let t = Affine::shape_local_to_parent(
        (n("PinX")?, n("PinY")?),
        (n("LocPinX").unwrap_or(w / 2.0), n("LocPinY").unwrap_or(h / 2.0)),
        n("Angle").unwrap_or(0.0),
        flip("FlipX"),
        flip("FlipY"),
    );
    Some((t, w, h))
}

/// A shape's local → page transform (through its groups) and size.
fn page_frame(model: &Model, page: usize, id: u32) -> Option<(Affine, f64, f64)> {
    fn path(shapes: &[Shape], id: u32, out: &mut Vec<u32>) -> bool {
        for s in shapes {
            out.push(s.id);
            if s.id == id || path(&s.shapes, id, out) {
                return true;
            }
            out.pop();
        }
        false
    }
    let p = model.pages.get(page)?;
    let mut ids = Vec::new();
    if !path(&p.shapes, id, &mut ids) {
        return None;
    }
    let calc = flowster_core::calc::Calc::for_page(model, p);
    let mut t = Affine::IDENTITY;
    let mut size = (0.0, 0.0);
    for i in ids {
        let (local, w, h) = local_frame(&calc, i)?;
        t = t.then(local);
        size = (w, h);
    }
    Some((t, size.0, size.1))
}

/// The top-level shape holding `id` (itself when on the page).
fn top_of(page: &Page, id: u32) -> Option<u32> {
    page.shapes.iter().find(|s| s.find(id).is_some()).map(|s| s.id)
}

/// The union of shapes' page bounds: x0, y0, x1, y1.
fn union(rects: &[Rect]) -> (f64, f64, f64, f64) {
    rects.iter().fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |(x0, y0, x1, y1), r| {
        (x0.min(r.cx - r.hw), y0.min(r.cy - r.hh), x1.max(r.cx + r.hw), y1.max(r.cy + r.hh))
    })
}

/// An angle in (-π, π].
fn normal_angle(a: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    let r = a.rem_euclid(tau);
    let r = if r > std::f64::consts::PI { r - tau } else { r };
    if r.abs() < 1e-12 { 0.0 } else { r }
}

/// The first visible foreground page other than `except`, preferring those after it.
fn visible_page(model: &Model, except: usize) -> Option<usize> {
    let ok = |i: &usize| *i != except && !model.pages[*i].background && !model.pages[*i].hidden();
    (except..model.pages.len()).find(ok).or_else(|| (0..except).rev().find(ok))
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
