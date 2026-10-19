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
        Document { doc, undo, undo_labels: Vec::new(), redo_labels: Vec::new(), assets: Assets::default() }
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
