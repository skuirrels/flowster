//! The live, CRDT-backed document the engine runs commands against (SPEC §5.5, §20.6).
//!
//! Holds the Visio model in the §20.3 schema ([`crate::model_io`]), document
//! properties in `meta`, the package inventory in `unknown`, and per-user undo.
//! Payloads (images, theme parts) live beside the CRDT in [`Assets`].

use std::collections::BTreeMap;

use flowster_core::command::{ChangeSummary, Command, CommandError, CommandTarget, UndoState};
use loro::{Container, ExportMode, LoroDoc, LoroMap, LoroValue, UndoManager, ValueOrContainer};

use crate::model_io::{self, Assets};
use crate::schema;
use flowster_core::model::Document as Model;
use flowster_core::theme::Theme;

/// Commits with this origin (file import, remote sync) are not undoable.
const IMPORT_ORIGIN: &str = "import";
const MAX_UNDO_STEPS: usize = 100;

pub struct Document {
    doc: LoroDoc,
    undo: UndoManager,
    undo_labels: Vec<String>,
    redo_labels: Vec<String>,
    assets: Assets,
    /// Counter behind the uuids of shapes this peer creates.
    next_uuid: u64,
}

/// One package part as recorded in `unknown` (§20.3: `partPath → BlobRef`).
pub struct PartEntry<'a> {
    pub name: &'a str,
    pub content_type: Option<&'a str>,
    pub size: u64,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    pub fn new() -> Self {
        let doc = LoroDoc::new();
        model_io::configure(&doc);
        let mut undo = UndoManager::new(&doc);
        undo.set_max_undo_steps(MAX_UNDO_STEPS);
        undo.add_exclude_origin_prefix(IMPORT_ORIGIN);
        Document {
            doc,
            undo,
            undo_labels: Vec::new(),
            redo_labels: Vec::new(),
            assets: Assets::default(),
            next_uuid: 0,
        }
    }

    /// A new, blank drawing (one page), as File > New makes.
    pub fn new_drawing(title: &str) -> Self {
        let mut d = Self::new();
        d.doc.get_map(schema::META).insert("title", title).unwrap();
        d.import_model(&crate::edit::blank_model(), None, model_io::seed_of(title.as_bytes()));
        d
    }

    /// A replica loaded from a snapshot, with the assets it refers to.
    pub fn from_snapshot(bytes: &[u8], assets: Assets) -> Result<Self, String> {
        let mut d = Self::new();
        d.doc.import(bytes).map_err(|e| e.to_string())?;
        d.assets = assets;
        Ok(d)
    }

    pub fn assets(&self) -> &Assets {
        &self.assets
    }

    /// Writes a model read from a file into the CRDT, in one transaction that
    /// is not undoable. `theme_xml` is the file's theme part; `seed` derives
    /// the uuids ([`model_io::seed_of`] of the file's bytes).
    pub fn import_model(&mut self, model: &Model, theme_xml: Option<&[u8]>, seed: u64) {
        let assets = model_io::write_model(&self.doc, model, theme_xml, seed);
        self.assets.blobs.extend(assets.blobs);
        self.doc.set_next_commit_origin(IMPORT_ORIGIN);
        self.doc.commit();
    }

    /// The stored theme part, if the drawing has one.
    pub fn theme_xml(&self) -> Option<&[u8]> {
        model_io::theme_xml(&self.doc, &self.assets)
    }

    /// The model as the CRDT holds it now (what the engine renders).
    pub fn read_model(&self, parse_theme: impl Fn(&[u8]) -> Option<Theme>) -> Model {
        model_io::read_model(&self.doc, &self.assets, parse_theme)
    }

    pub fn loro(&self) -> &LoroDoc {
        &self.doc
    }

    /// Records an opened package: title, source file and its part inventory.
    pub fn import_package(&mut self, file_name: &str, format: &str, parts: &[PartEntry<'_>]) {
        let meta = self.doc.get_map(schema::META);
        let title = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
        let title = title.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(title);
        meta.insert("title", title).unwrap();
        meta.insert("sourceFile", file_name).unwrap();
        meta.insert("sourceFormat", format).unwrap();

        let unknown = self.doc.get_map(schema::UNKNOWN);
        for p in parts {
            let entry = unknown.insert_container(p.name, LoroMap::new()).unwrap();
            entry.insert("contentType", p.content_type.unwrap_or("")).unwrap();
            entry.insert("size", p.size as i64).unwrap();
            // Blob storage arrives with the asset store; until then the entry is a placeholder.
            entry.insert("blob", LoroValue::Null).unwrap();
        }
        self.doc.set_next_commit_origin(IMPORT_ORIGIN);
        self.doc.commit();
    }

    pub fn meta(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        self.doc.get_map(schema::META).for_each(|k, v| {
            if let ValueOrContainer::Value(LoroValue::String(s)) = v {
                out.insert(k.to_owned(), s.to_string());
            }
        });
        out
    }

    /// Number of entries in `unknown`.
    pub fn unknown_count(&self) -> usize {
        let mut n = 0;
        self.doc.get_map(schema::UNKNOWN).for_each(|_, v| {
            if matches!(v, ValueOrContainer::Container(Container::Map(_))) {
                n += 1;
            }
        });
        n
    }

    /// Shapes (page and master) stored in the CRDT.
    pub fn shape_count(&self) -> usize {
        self.doc.get_map(schema::SHAPE).len()
    }

    pub fn snapshot(&self) -> Vec<u8> {
        self.doc.export(ExportMode::Snapshot).unwrap()
    }
}

impl CommandTarget for Document {
    fn exec(&mut self, cmd: &Command) -> Result<ChangeSummary, CommandError> {
        let mut changes = ChangeSummary::default();
        match cmd {
            Command::SetDocProp { key, value } => {
                self.doc
                    .get_map(schema::META)
                    .insert(key, value.as_str())
                    .map_err(|e| CommandError::Failed(e.to_string()))?;
                self.doc.commit();
                self.undo_labels.push(cmd.label().to_owned());
                self.redo_labels.clear();
                changes.meta.push(key.clone());
            }
            Command::AddShape { .. }
            | Command::MoveShapes { .. }
            | Command::SetBounds { .. }
            | Command::SetEndpoints { .. }
            | Command::DeleteShapes { .. }
            | Command::SetText { .. }
            | Command::SetCells { .. }
            | Command::AddConnector { .. }
            | Command::AutoConnect { .. }
            | Command::AlignShapes { .. }
            | Command::DistributeShapes { .. }
            | Command::RotateShapes { .. }
            | Command::FlipShapes { .. }
            | Command::ReorderShapes { .. }
            | Command::GroupShapes { .. }
            | Command::UngroupShapes { .. }
            | Command::AddPage { .. }
            | Command::DeletePage { .. }
            | Command::RenamePage { .. }
            | Command::SetPageHidden { .. }
            | Command::InsertShapes { .. } => {
                let mut ed = crate::edit::Editor { doc: &self.doc, next: &mut self.next_uuid };
                changes = match cmd {
                    Command::AddShape { page, shape, x, y, w, h } => ed.add_shape(*page, shape, *x, *y, *w, *h),
                    Command::MoveShapes { page, shapes, dx, dy } => ed.move_shapes(*page, shapes, *dx, *dy),
                    Command::SetBounds { page, shape, pin_x, pin_y, width, height } => {
                        ed.set_bounds(*page, *shape, *pin_x, *pin_y, *width, *height)
                    }
                    Command::SetEndpoints { page, shape, begin_x, begin_y, end_x, end_y, begin_glue, end_glue } => ed
                        .set_endpoints(*page, *shape, (*begin_x, *begin_y), (*end_x, *end_y), (*begin_glue, *end_glue)),
                    Command::AddConnector { page, from, to, x0, y0, x1, y1 } => {
                        ed.add_connector(*page, *from, *to, (*x0, *y0), (*x1, *y1))
                    }
                    Command::AutoConnect { page, shape, direction } => ed.auto_connect(*page, *shape, direction),
                    Command::DeleteShapes { page, shapes } => ed.delete_shapes(*page, shapes),
                    Command::SetText { page, shape, text } => ed.set_text(*page, *shape, text),
                    Command::SetCells { page, shapes, cells } => ed.set_cells(*page, shapes, cells),
                    Command::AlignShapes { page, shapes, align } => ed.align(*page, shapes, align),
                    Command::DistributeShapes { page, shapes, axis } => ed.distribute(*page, shapes, axis),
                    Command::RotateShapes { page, shapes, direction } => ed.rotate(*page, shapes, direction),
                    Command::FlipShapes { page, shapes, axis } => ed.flip(*page, shapes, axis),
                    Command::ReorderShapes { page, shapes, to } => ed.reorder(*page, shapes, to),
                    Command::GroupShapes { page, shapes } => ed.group(*page, shapes),
                    Command::UngroupShapes { page, shapes } => ed.ungroup(*page, shapes),
                    Command::AddPage { after } => ed.add_page(*after),
                    Command::DeletePage { page } => ed.delete_page(*page),
                    Command::RenamePage { page, name } => ed.rename_page(*page, name),
                    Command::SetPageHidden { page, hidden } => ed.set_page_hidden(*page, *hidden),
                    Command::InsertShapes { page, shapes, dx, dy } => ed.insert_shapes(*page, shapes, *dx, *dy),
                    _ => unreachable!(),
                }?;
                self.doc.commit();
                self.undo_labels.push(cmd.label().to_owned());
                self.redo_labels.clear();
            }
            Command::Undo => {
                if self.undo.undo().map_err(|e| CommandError::Failed(e.to_string()))? {
                    if let Some(l) = self.undo_labels.pop() {
                        self.redo_labels.push(l);
                    }
                    changes.meta.push("*".into());
                }
            }
            Command::Redo => {
                if self.undo.redo().map_err(|e| CommandError::Failed(e.to_string()))? {
                    if let Some(l) = self.redo_labels.pop() {
                        self.undo_labels.push(l);
                    }
                    changes.meta.push("*".into());
                }
            }
        }
        Ok(changes)
    }

    fn undo_state(&self) -> UndoState {
        UndoState {
            can_undo: self.undo.can_undo(),
            can_redo: self.undo.can_redo(),
            labels: self.undo_labels.iter().rev().cloned().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowster_core::model::TextPiece;

    #[test]
    fn drawing_moving_resizing_typing_deleting_and_undo() {
        let mut d = Document::new_drawing("Drawing1");
        let model = |d: &Document| d.read_model(|_| None);
        assert_eq!(model(&d).pages.len(), 1);
        assert!(!d.undo_state().can_undo);

        let add = |d: &mut Document, shape: String, x, y, w, h| {
            d.exec(&Command::AddShape { page: 0, shape, x, y, w, h }).unwrap().created.unwrap().1
        };
        let rect = add(&mut d, "rectangle".into(), 1.0, 1.0, 2.0, 1.0);
        let ellipse = add(&mut d, "ellipse".into(), 4.0, 1.0, 2.0, 2.0);
        let line = add(&mut d, "line".into(), 1.0, 5.0, 3.0, 4.0);
        assert_eq!((rect, ellipse, line), (1, 2, 3));

        let cell = |d: &Document, id: u32, name: &str| -> f64 {
            let m = model(d);
            let calc = flowster_core::calc::Calc::for_page(&m, &m.pages[0]);
            calc.cell(id, name).num().unwrap()
        };
        // Cached values were filled in by recalc: the renderer draws them.
        let m = model(&d);
        let s = &m.pages[0].shapes[0];
        assert_eq!(s.sheet.cells["LocPinX"].v.as_deref(), Some("1"));
        // Library outlines are relative rows: they scale with the shape.
        assert_eq!(s.sheet.sections[0].rows[2].t.as_deref(), Some("RelLineTo"));
        assert!((cell(&d, line, "Width") - 5.0).abs() < 1e-9, "3-4-5 line");
        let dl = flowster_render::build_page(&m, 0).unwrap();
        assert_eq!(dl.items.iter().filter(|i| matches!(i, flowster_render::Item::Path { .. })).count(), 3);

        d.exec(&Command::MoveShapes { page: 0, shapes: vec![rect, line], dx: 0.5, dy: 0.25 }).unwrap();
        assert_eq!((cell(&d, rect, "PinX"), cell(&d, rect, "PinY")), (2.5, 1.75));
        assert_eq!((cell(&d, line, "BeginX"), cell(&d, line, "PinY")), (1.5, 7.25));

        d.exec(&Command::SetBounds { page: 0, shape: rect, pin_x: 3.0, pin_y: 2.0, width: 4.0, height: 2.0 }).unwrap();
        let m = model(&d);
        let s = &m.pages[0].shapes[0];
        assert_eq!(s.sheet.cells["LocPinX"].v.as_deref(), Some("2"), "LocPin follows Width");
        let dl = flowster_render::build_page(&m, 0).unwrap();
        let [x0, _, x1, _] =
            flowster_render::DisplayList { items: dl.items[..1].to_vec(), ..dl.clone() }.extents().unwrap();
        assert!((x1 - x0 - 4.0).abs() < 0.02, "geometry follows Width: {}", x1 - x0);

        d.exec(&Command::SetText { page: 0, shape: rect, text: "Hello".into() }).unwrap();
        assert_eq!(model(&d).pages[0].shapes[0].text, Some(vec![TextPiece::Text("Hello".into())]));

        // Formatting: fill, line and the text's font through SetCells.
        let edit = |section: Option<&str>, cell: &str, value: &str| flowster_core::command::CellEdit {
            section: section.map(str::to_owned),
            row: None,
            cell: cell.into(),
            value: value.into(),
        };
        d.exec(&Command::SetCells {
            page: 0,
            shapes: vec![rect],
            cells: vec![
                edit(None, "FillForegnd", "#ff0000"),
                edit(None, "LinePattern", "0"),
                edit(Some("Character"), "Font", "Arial"),
                edit(Some("Character"), "Size", "0.25"),
                edit(Some("Character"), "Style", "1"),
            ],
        })
        .unwrap();
        let dl = flowster_render::build_page(&model(&d), 0).unwrap();
        let fmt = &dl.shapes.iter().find(|b| b.id == rect).unwrap().format;
        assert_eq!(fmt.fill, Some(flowster_core::value::Rgba(255, 0, 0, 255)));
        assert_eq!(fmt.line, None);
        assert_eq!((fmt.font.as_str(), fmt.size, fmt.bold), ("Arial", 18.0, true));
        assert_eq!(d.undo_state().labels[0], "Format");

        d.exec(&Command::SetEndpoints {
            page: 0,
            shape: line,
            begin_x: 0.0,
            begin_y: 0.0,
            end_x: 0.0,
            end_y: 2.0,
            begin_glue: None,
            end_glue: None,
        })
        .unwrap();
        assert!((cell(&d, line, "Angle") - std::f64::consts::FRAC_PI_2).abs() < 1e-9);

        d.exec(&Command::DeleteShapes { page: 0, shapes: vec![ellipse] }).unwrap();
        assert_eq!(model(&d).pages[0].shapes.len(), 2);
        assert_eq!(d.undo_state().labels[0], "Delete");

        d.exec(&Command::Undo).unwrap();
        assert_eq!(model(&d).pages[0].shapes.len(), 3, "undo restores the ellipse");
        d.exec(&Command::Undo).unwrap();
        d.exec(&Command::Undo).unwrap();
        d.exec(&Command::Undo).unwrap();
        assert_eq!(model(&d).pages[0].shapes[0].text, None, "undo removes the text");
    }

    #[test]
    fn connectors_glue_follow_moves_and_autoconnect() {
        let mut d = Document::new_drawing("Flow");
        let add = |d: &mut Document, x: f64| {
            let cmd = Command::AddShape { page: 0, shape: "process".into(), x, y: 1.0, w: 1.0, h: 1.0 };
            d.exec(&cmd).unwrap().created.unwrap().1
        };
        let (a, b) = (add(&mut d, 1.0), add(&mut d, 4.0));
        let conn = d
            .exec(&Command::AddConnector { page: 0, from: Some(a), to: Some(b), x0: 1.5, y0: 1.5, x1: 4.5, y1: 1.5 })
            .unwrap()
            .created
            .unwrap()
            .1;
        let ends = |d: &Document| {
            let m = d.read_model(|_| None);
            let calc = flowster_core::calc::Calc::for_page(&m, &m.pages[0]);
            ["BeginX", "BeginY", "EndX", "EndY"].map(|n| calc.cell(conn, n).num().unwrap())
        };
        // Facing sides: a's right edge to b's left edge.
        assert_eq!(ends(&d), [2.0, 1.5, 4.0, 1.5]);
        // Moving b drags the connector's end along, with a right-angle path.
        d.exec(&Command::MoveShapes { page: 0, shapes: vec![b], dx: 0.0, dy: 2.0 }).unwrap();
        assert_eq!(ends(&d), [2.0, 1.5, 4.0, 3.5]);
        let m = d.read_model(|_| None);
        let s = m.pages[0].shapes.iter().find(|s| s.id == conn).unwrap();
        assert_eq!(s.sheet.sections[0].rows.len(), 4, "Z-shaped route");

        // AutoConnect: a copy below `a`, connected to it.
        let changes = d.exec(&Command::AutoConnect { page: 0, shape: a, direction: "down".into() }).unwrap();
        let copy = changes.created.unwrap().1;
        let m = d.read_model(|_| None);
        let calc = flowster_core::calc::Calc::for_page(&m, &m.pages[0]);
        assert_eq!((calc.cell(copy, "PinX").num().unwrap(), calc.cell(copy, "PinY").num().unwrap()), (1.5, -0.25));
        let connects = flowster_core::library::derive_connects(&m.pages[0]);
        assert_eq!(connects.len(), 4);
        assert!(connects.iter().any(|c| c.to_sheet == copy));
    }

    #[test]
    fn swimlanes_go_behind_the_drawing() {
        let mut d = Document::new_drawing("Lanes");
        let add = |d: &mut Document, shape: &str, w: f64, h: f64| {
            let cmd = Command::AddShape { page: 0, shape: shape.into(), x: 1.0, y: 1.0, w, h };
            d.exec(&cmd).unwrap().created.unwrap().1
        };
        let step = add(&mut d, "process", 1.5, 0.9);
        let pool = add(&mut d, "pool", 8.0, 4.9);
        let m = d.read_model(|_| None);
        let order: Vec<u32> = m.pages[0].shapes.iter().map(|s| s.id).collect();
        assert_eq!(order.len(), 5, "a frame and three lanes, then the step");
        assert_eq!(order[0], pool);
        assert_eq!(*order.last().unwrap(), step);
    }

    #[test]
    fn pages_insert_rename_hide_delete_and_undo() {
        let mut d = Document::new_drawing("Pages");
        let names = |d: &Document| -> Vec<String> {
            d.read_model(|_| None).pages.iter().map(|p| p.name.clone().unwrap_or_default()).collect()
        };
        let added = d.exec(&Command::AddPage { after: None }).unwrap();
        assert_eq!(added.page, Some(1));
        d.exec(&Command::AddPage { after: Some(0) }).unwrap();
        assert_eq!(names(&d), ["Page-1", "Page-3", "Page-2"]);
        let m = d.read_model(|_| None);
        assert_eq!(m.pages[1].sheet.cells["PageWidth"].v.as_deref(), Some("11"), "page setup is copied");
        assert_ne!(m.pages[1].id, m.pages[2].id);

        d.exec(&Command::AddShape { page: 1, shape: "rectangle".into(), x: 1.0, y: 1.0, w: 1.0, h: 1.0 }).unwrap();
        assert_eq!(d.read_model(|_| None).pages[1].shapes.len(), 1);
        assert!(d.read_model(|_| None).pages[0].shapes.is_empty());

        d.exec(&Command::RenamePage { page: 1, name: " Overview ".into() }).unwrap();
        assert!(d.exec(&Command::RenamePage { page: 2, name: "Overview".into() }).is_err());
        assert_eq!(names(&d)[1], "Overview");

        let hidden = d.exec(&Command::SetPageHidden { page: 0, hidden: true }).unwrap();
        assert_eq!(hidden.page, Some(1), "the next visible page shows");
        assert!(d.read_model(|_| None).pages[0].hidden());
        d.exec(&Command::SetPageHidden { page: 1, hidden: true }).unwrap();
        assert!(d.exec(&Command::SetPageHidden { page: 2, hidden: true }).is_err(), "one page stays visible");
        d.exec(&Command::SetPageHidden { page: 0, hidden: false }).unwrap();
        assert!(!d.read_model(|_| None).pages[0].hidden());

        let deleted = d.exec(&Command::DeletePage { page: 1 }).unwrap();
        assert_eq!(names(&d), ["Page-1", "Page-2"]);
        assert_eq!(deleted.page, Some(1));
        d.exec(&Command::DeletePage { page: 0 }).unwrap();
        assert!(d.exec(&Command::DeletePage { page: 0 }).is_err(), "the last page stays");
        d.exec(&Command::Undo).unwrap();
        d.exec(&Command::Undo).unwrap();
        assert_eq!(names(&d), ["Page-1", "Overview", "Page-2"]);
        assert_eq!(d.read_model(|_| None).pages[1].shapes.len(), 1, "undo brings the shapes back");
    }

    #[test]
    fn align_distribute_orient_reorder_and_group() {
        let mut d = Document::new_drawing("Arrange");
        let add = |d: &mut Document, x: f64, y: f64, w: f64| {
            let cmd = Command::AddShape { page: 0, shape: "rectangle".into(), x, y, w, h: 1.0 };
            d.exec(&cmd).unwrap().created.unwrap().1
        };
        let a = add(&mut d, 1.0, 1.0, 2.0);
        let b = add(&mut d, 4.0, 3.0, 1.0);
        let c = add(&mut d, 8.0, 2.0, 1.0);
        let cell = |d: &Document, id: u32, name: &str| -> f64 {
            let m = d.read_model(|_| None);
            let calc = flowster_core::calc::Calc::for_page(&m, &m.pages[0]);
            calc.cell(id, name).num().unwrap()
        };
        let close = |x: f64, y: f64| (x - y).abs() < 1e-9;

        d.exec(&Command::AlignShapes { page: 0, shapes: vec![a, b, c], align: "left".into() }).unwrap();
        assert!(close(cell(&d, b, "PinX"), 1.5) && close(cell(&d, c, "PinX"), 1.5), "left edges at x = 1");
        d.exec(&Command::AlignShapes { page: 0, shapes: vec![a, b, c], align: "middle".into() }).unwrap();
        assert!(close(cell(&d, b, "PinY"), 1.5));
        d.exec(&Command::Undo).unwrap();
        d.exec(&Command::Undo).unwrap();

        // Gaps a→b: 1, b→c: 3; distributed: 2 each.
        d.exec(&Command::DistributeShapes { page: 0, shapes: vec![c, a, b], axis: "horizontal".into() }).unwrap();
        assert!(close(cell(&d, a, "PinX"), 2.0) && close(cell(&d, c, "PinX"), 8.5));
        assert!(close(cell(&d, b, "PinX"), 5.5));

        d.exec(&Command::RotateShapes { page: 0, shapes: vec![a], direction: "left".into() }).unwrap();
        assert!(close(cell(&d, a, "Angle"), std::f64::consts::FRAC_PI_2));
        d.exec(&Command::FlipShapes { page: 0, shapes: vec![a], axis: "horizontal".into() }).unwrap();
        assert!(close(cell(&d, a, "Angle"), -std::f64::consts::FRAC_PI_2));
        assert!(close(cell(&d, a, "FlipX"), 1.0));
        assert!(close(cell(&d, a, "PinX"), 2.0), "a lone shape flips in place");

        let order =
            |d: &Document| -> Vec<u32> { d.read_model(|_| None).pages[0].shapes.iter().map(|s| s.id).collect() };
        d.exec(&Command::ReorderShapes { page: 0, shapes: vec![a], to: "front".into() }).unwrap();
        assert_eq!(order(&d), [b, c, a]);
        d.exec(&Command::ReorderShapes { page: 0, shapes: vec![a], to: "backward".into() }).unwrap();
        assert_eq!(order(&d), [b, a, c]);
        d.exec(&Command::ReorderShapes { page: 0, shapes: vec![c], to: "back".into() }).unwrap();
        assert_eq!(order(&d), [c, b, a]);
        d.exec(&Command::ReorderShapes { page: 0, shapes: vec![c], to: "forward".into() }).unwrap();
        assert_eq!(order(&d), [b, c, a]);

        let page_bounds = |d: &Document| {
            let dl = flowster_render::build_page(&d.read_model(|_| None), 0).unwrap();
            dl.items.len()
        };
        let items = page_bounds(&d);
        let before = (cell(&d, b, "PinX"), cell(&d, b, "PinY"));
        let grouped = d.exec(&Command::GroupShapes { page: 0, shapes: vec![b, c] }).unwrap();
        let g = grouped.created.unwrap().1;
        let m = d.read_model(|_| None);
        assert_eq!(m.pages[0].shapes.iter().map(|s| s.id).collect::<Vec<_>>(), [g, a], "the group stacks where c was");
        assert_eq!(m.pages[0].shapes[0].shapes.len(), 2);
        assert_eq!(page_bounds(&d), items, "grouping draws the same shapes");

        d.exec(&Command::MoveShapes { page: 0, shapes: vec![g], dx: 1.0, dy: 0.0 }).unwrap();
        d.exec(&Command::UngroupShapes { page: 0, shapes: vec![g] }).unwrap();
        assert_eq!(order(&d), [b, c, a]);
        assert!(close(cell(&d, b, "PinX"), before.0 + 1.0) && close(cell(&d, b, "PinY"), before.1));
    }

    #[test]
    fn pasted_shapes_get_new_ids_and_keep_their_glue() {
        let mut d = Document::new_drawing("Paste");
        let add = |d: &mut Document, x: f64| {
            let cmd = Command::AddShape { page: 0, shape: "process".into(), x, y: 1.0, w: 1.0, h: 1.0 };
            d.exec(&cmd).unwrap().created.unwrap().1
        };
        let (a, b) = (add(&mut d, 1.0), add(&mut d, 4.0));
        let c = d
            .exec(&Command::AddConnector { page: 0, from: Some(a), to: Some(b), x0: 1.5, y0: 1.5, x1: 4.5, y1: 1.5 })
            .unwrap()
            .created
            .unwrap()
            .1;
        let m = d.read_model(|_| None);
        // Copy b and the connector only: the connector's glue to a must go.
        let copied: Vec<_> = m.pages[0].shapes.iter().filter(|s| s.id == b || s.id == c).cloned().collect();
        let pasted = d.exec(&Command::InsertShapes { page: 0, shapes: copied.clone(), dx: 0.0, dy: 3.0 }).unwrap();
        let new: Vec<u32> = pasted.shapes.iter().map(|s| s.parse().unwrap()).collect();
        assert_eq!(new, [4, 5]);
        let m = d.read_model(|_| None);
        let calc = flowster_core::calc::Calc::for_page(&m, &m.pages[0]);
        assert!((calc.cell(4, "PinY").num().unwrap() - 4.5).abs() < 1e-9);
        let conn = m.pages[0].shapes.iter().find(|s| s.id == 5).unwrap();
        assert_eq!(flowster_core::library::glue_target(&conn.sheet, "BegTrigger"), None);
        assert_eq!(flowster_core::library::glue_target(&conn.sheet, "EndTrigger"), Some(4));
        assert_eq!(conn.name_u.as_deref(), Some("Dynamic connector.5"));

        // Copy all three to another page: everything stays connected.
        d.exec(&Command::AddPage { after: None }).unwrap();
        let all: Vec<_> = m.pages[0].shapes.iter().filter(|s| [a, b, c].contains(&s.id)).cloned().collect();
        d.exec(&Command::InsertShapes { page: 1, shapes: all, dx: 0.0, dy: 0.0 }).unwrap();
        let m = d.read_model(|_| None);
        let conn = m.pages[1].shapes.iter().find(|s| s.id == 3).unwrap();
        assert_eq!(flowster_core::library::glue_target(&conn.sheet, "BegTrigger"), Some(1));
        assert_eq!(flowster_core::library::glue_target(&conn.sheet, "EndTrigger"), Some(2));
        d.exec(&Command::Undo).unwrap();
        assert!(d.read_model(|_| None).pages[1].shapes.is_empty());
    }

    #[test]
    fn a_drawing_round_trips_through_the_crdt_and_a_snapshot() {
        let bytes = flowster_vsdx::fixture::minimal_vsdx();
        let pkg = flowster_vsdx::OpcPackage::open(bytes.clone()).unwrap();
        let model = flowster_vsdx::visio::read_document(&pkg).unwrap();
        let theme = flowster_vsdx::visio::theme_xml(&pkg);
        let mut d = Document::new();
        d.import_model(&model, theme.as_deref(), model_io::seed_of(&bytes));
        assert!(!d.undo_state().can_undo, "import must not be undoable");

        let parse = |b: &[u8]| flowster_vsdx::visio::parse_theme_xml(b);
        let expected = model_io::canonical_model(&model);
        assert_eq!(d.read_model(parse), expected);
        // A fresh replica from the snapshot reads the same model.
        let copy = Document::from_snapshot(&d.snapshot(), d.assets().clone()).unwrap();
        assert_eq!(copy.read_model(parse), expected);
    }

    #[test]
    fn commands_are_undoable_but_import_is_not() {
        let mut d = Document::new();
        d.import_package(
            "C:\\x\\Plan.vsdx",
            "vsdx",
            &[PartEntry { name: "/visio/document.xml", content_type: None, size: 3 }],
        );
        assert_eq!(d.meta()["title"], "Plan");
        assert_eq!(d.unknown_count(), 1);
        assert!(!d.undo_state().can_undo, "import must not be undoable");

        d.exec(&Command::SetDocProp { key: "title".into(), value: "Renamed".into() }).unwrap();
        assert_eq!(d.meta()["title"], "Renamed");
        assert_eq!(d.undo_state().labels, ["Change Document Property"]);

        d.exec(&Command::Undo).unwrap();
        assert_eq!(d.meta()["title"], "Plan");
        assert!(d.undo_state().can_redo);
        d.exec(&Command::Redo).unwrap();
        assert_eq!(d.meta()["title"], "Renamed");
    }
}
