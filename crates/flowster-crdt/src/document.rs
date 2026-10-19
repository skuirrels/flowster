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
            | Command::SetText { .. } => {
                let mut ed = crate::edit::Editor { doc: &self.doc, next: &mut self.next_uuid };
                changes = match cmd {
                    Command::AddShape { page, shape, x, y, w, h } => ed.add_shape(*page, *shape, *x, *y, *w, *h),
                    Command::MoveShapes { page, shapes, dx, dy } => ed.move_shapes(*page, shapes, *dx, *dy),
                    Command::SetBounds { page, shape, pin_x, pin_y, width, height } => {
                        ed.set_bounds(*page, *shape, *pin_x, *pin_y, *width, *height)
                    }
                    Command::SetEndpoints { page, shape, begin_x, begin_y, end_x, end_y } => {
                        ed.set_endpoints(*page, *shape, (*begin_x, *begin_y), (*end_x, *end_y))
                    }
                    Command::DeleteShapes { page, shapes } => ed.delete_shapes(*page, shapes),
                    Command::SetText { page, shape, text } => ed.set_text(*page, *shape, text),
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
        use flowster_core::command::ShapeKind;
        let mut d = Document::new_drawing("Drawing1");
        let model = |d: &Document| d.read_model(|_| None);
        assert_eq!(model(&d).pages.len(), 1);
        assert!(!d.undo_state().can_undo);

        let add = |d: &mut Document, shape, x, y, w, h| {
            d.exec(&Command::AddShape { page: 0, shape, x, y, w, h }).unwrap().created.unwrap().1
        };
        let rect = add(&mut d, ShapeKind::Rectangle, 1.0, 1.0, 2.0, 1.0);
        let ellipse = add(&mut d, ShapeKind::Ellipse, 4.0, 1.0, 2.0, 2.0);
        let line = add(&mut d, ShapeKind::Line, 1.0, 5.0, 3.0, 4.0);
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
        assert_eq!(s.sheet.sections[0].rows[2].cells["X"].v.as_deref(), Some("2"));
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
        assert_eq!(s.sheet.sections[0].rows[2].cells["X"].v.as_deref(), Some("4"), "geometry follows Width");

        d.exec(&Command::SetText { page: 0, shape: rect, text: "Hello".into() }).unwrap();
        assert_eq!(model(&d).pages[0].shapes[0].text, Some(vec![TextPiece::Text("Hello".into())]));

        d.exec(&Command::SetEndpoints { page: 0, shape: line, begin_x: 0.0, begin_y: 0.0, end_x: 0.0, end_y: 2.0 })
            .unwrap();
        assert!((cell(&d, line, "Angle") - std::f64::consts::FRAC_PI_2).abs() < 1e-9);

        d.exec(&Command::DeleteShapes { page: 0, shapes: vec![ellipse] }).unwrap();
        assert_eq!(model(&d).pages[0].shapes.len(), 2);
        assert_eq!(d.undo_state().labels[0], "Delete");

        d.exec(&Command::Undo).unwrap();
        assert_eq!(model(&d).pages[0].shapes.len(), 3, "undo restores the ellipse");
        d.exec(&Command::Undo).unwrap();
        d.exec(&Command::Undo).unwrap();
        assert_eq!(model(&d).pages[0].shapes[0].text, None, "undo removes the text");
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
