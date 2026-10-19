//! The engine that lives in the Web Worker (SPEC §5.1): owns open documents and
//! turns [`EngineRequest`]s into [`EngineEvent`]s. Plain Rust, so it is tested natively.

use std::collections::BTreeMap;

use flowster_core::command::{ChangeSummary, CommandTarget, OpenedSummary};
use flowster_core::model::Document as Model;
use flowster_core::protocol::{DocId, EngineEvent, EngineRequest, Query, SyncStateKind};
use flowster_crdt::document::{Document, PartEntry};
use flowster_vsdx::Package;
use serde_json::{Value, json};

struct OpenDoc {
    file_name: String,
    /// The package the document was opened from (`None` for a new drawing).
    package: Option<Package>,
    /// The Visio model as the CRDT holds it (`None` if the file is not a Visio drawing).
    model: Option<Model>,
    /// Problems met reading the file, kept across re-reads of the model.
    warnings: Vec<String>,
    doc: Document,
}

impl OpenDoc {
    /// Re-reads the model from the CRDT after a change (SPEC §20.6).
    fn refresh(&mut self) {
        if self.model.is_some() {
            let mut m = self.doc.read_model(flowster_vsdx::visio::parse_theme_xml);
            m.warnings = self.warnings.clone();
            self.model = Some(m);
        }
    }
}

#[derive(Default)]
pub struct Engine {
    docs: BTreeMap<DocId, OpenDoc>,
    next_doc: u32,
}

fn undo_event(doc: &DocId, d: &Document) -> EngineEvent {
    let u = d.undo_state();
    EngineEvent::UndoState { doc: doc.clone(), can_undo: u.can_undo, can_redo: u.can_redo, labels: u.labels }
}

fn error(id: u32, message: impl Into<String>) -> Vec<EngineEvent> {
    vec![EngineEvent::Error { id: Some(id), message: message.into() }]
}

fn format_of(file_name: &str) -> String {
    file_name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    /// `open` carries the file bytes out of band, so it has its own entry point.
    pub fn open(&mut self, id: u32, file_name: &str, bytes: &[u8]) -> Vec<EngineEvent> {
        let format = format_of(file_name);
        match format.as_str() {
            "vsdx" | "vssx" | "vstx" | "vsdm" | "vssm" | "vstm" => {}
            "vsd" | "vss" | "vst" | "vsw" => {
                return error(id, "Legacy .vsd import arrives with flowster-vsd (SPEC §19.5)");
            }
            _ => return error(id, format!("Unsupported file type: {file_name}")),
        }
        let pkg = match flowster_vsdx::OpcPackage::open(bytes.to_vec()) {
            Ok(p) => p,
            Err(e) => return error(id, format!("Could not open {file_name}: {e}")),
        };
        let mut package = pkg.info.clone();
        let model = match flowster_vsdx::visio::read_document(&pkg) {
            Ok(m) => Some(m),
            Err(e) => {
                package.warnings.push(format!("drawing not readable: {e}"));
                None
            }
        };

        let mut doc = Document::new();
        let entries: Vec<PartEntry<'_>> = package
            .parts
            .iter()
            .map(|p| PartEntry { name: &p.name, content_type: p.content_type.as_deref(), size: p.size })
            .collect();
        doc.import_package(file_name, &format, &entries);
        // The CRDT is the source of truth (SPEC §20.6): write the drawing into
        // it and render what reads back.
        let warnings = model.as_ref().map(|m| m.warnings.clone()).unwrap_or_default();
        let model = model.map(|m| {
            let theme = flowster_vsdx::visio::theme_xml(&pkg);
            doc.import_model(&m, theme.as_deref(), flowster_crdt::model_io::seed_of(bytes));
            let mut back = doc.read_model(flowster_vsdx::visio::parse_theme_xml);
            back.warnings = m.warnings;
            back
        });

        self.next_doc += 1;
        let doc_id = format!("doc-{}", self.next_doc);
        let opened = OpenedSummary { file_name: file_name.to_owned(), format, part_count: package.parts.len() };
        let result = json!({
            "doc": doc_id,
            "fileName": file_name,
            "kind": package.kind,
            "mainPart": package.main_part,
            "partCount": package.parts.len(),
            "pageCount": model.as_ref().map_or(0, |m| m.pages.len()),
        });
        let events = vec![
            EngineEvent::ModelChanged {
                doc: doc_id.clone(),
                changes: ChangeSummary { opened: Some(opened), ..Default::default() },
            },
            undo_event(&doc_id, &doc),
            EngineEvent::SyncState { doc: doc_id.clone(), state: SyncStateKind::Local, pending_ops: 0 },
            EngineEvent::Reply { id, result },
        ];
        self.docs
            .insert(doc_id, OpenDoc { file_name: file_name.to_owned(), package: Some(package), model, warnings, doc });
        events
    }

    /// A new, blank drawing (File > New).
    fn new_drawing(&mut self, id: u32, title: Option<String>) -> Vec<EngineEvent> {
        self.next_doc += 1;
        let doc_id = format!("doc-{}", self.next_doc);
        let title = title.unwrap_or_else(|| format!("Drawing{}", self.next_doc));
        let doc = Document::new_drawing(&title);
        let model = Some(doc.read_model(flowster_vsdx::visio::parse_theme_xml));
        let file_name = format!("{title}.vsdx");
        let result = json!({
            "doc": doc_id,
            "fileName": file_name,
            "kind": "drawing",
            "mainPart": null,
            "partCount": 0,
            "pageCount": 1,
        });
        let opened = OpenedSummary { file_name: file_name.clone(), format: "vsdx".into(), part_count: 0 };
        let events = vec![
            EngineEvent::ModelChanged {
                doc: doc_id.clone(),
                changes: ChangeSummary { opened: Some(opened), ..Default::default() },
            },
            undo_event(&doc_id, &doc),
            EngineEvent::SyncState { doc: doc_id.clone(), state: SyncStateKind::Local, pending_ops: 0 },
            EngineEvent::Reply { id, result },
        ];
        self.docs.insert(doc_id, OpenDoc { file_name, package: None, model, warnings: Vec::new(), doc });
        events
    }

    pub fn handle(&mut self, id: u32, req: EngineRequest) -> Vec<EngineEvent> {
        match req {
            EngineRequest::Open { .. } => error(id, "open needs file bytes; use Engine::open"),
            EngineRequest::New { title } => self.new_drawing(id, title),
            EngineRequest::Query { doc, q } => {
                let Some(d) = self.docs.get(&doc) else { return error(id, format!("no document {doc}")) };
                let result = match q {
                    Query::PackageParts => match &d.package {
                        Some(p) => serde_json::to_value(p).unwrap_or(Value::Null),
                        None => json!({
                            "kind": "drawing", "mainPart": null, "parts": [], "relationships": [],
                            "contentTypes": {"defaults": [], "overrides": []}, "totalSize": 0, "warnings": [],
                        }),
                    },
                    Query::DocInfo => json!({
                        "fileName": d.file_name,
                        "meta": d.doc.meta(),
                        "unknownParts": d.doc.unknown_count(),
                        "shapes": d.doc.shape_count(),
                        "snapshotBytes": d.doc.snapshot().len(),
                    }),
                    Query::DerivedStateHash => {
                        serde_json::to_value(flowster_core::determinism::default_report()).unwrap_or(Value::Null)
                    }
                    Query::Pages => match &d.model {
                        Some(m) => serde_json::to_value(flowster_render::pages(m)).unwrap_or(Value::Null),
                        None => json!([]),
                    },
                    Query::PageDisplayList { page } => {
                        match d.model.as_ref().and_then(|m| flowster_render::build_page(m, page)) {
                            Some(dl) => serde_json::to_value(dl).unwrap_or(Value::Null),
                            None => return error(id, format!("no page {page}")),
                        }
                    }
                };
                vec![EngineEvent::Reply { id, result }]
            }
            EngineRequest::Exec { doc, cmd } => {
                let Some(d) = self.docs.get_mut(&doc) else { return error(id, format!("no document {doc}")) };
                match d.doc.exec(&cmd) {
                    Ok(changes) => {
                        d.refresh();
                        let result = serde_json::to_value(&changes).unwrap_or(Value::Null);
                        vec![
                            EngineEvent::ModelChanged { doc: doc.clone(), changes },
                            undo_event(&doc, &d.doc),
                            EngineEvent::Reply { id, result },
                        ]
                    }
                    Err(e) => error(id, e.to_string()),
                }
            }
            EngineRequest::Gesture { .. } => vec![EngineEvent::Reply { id, result: Value::Null }],
            EngineRequest::Save { .. } => error(id, "Saving arrives in M2 (SPEC §19.2)"),
            EngineRequest::Export { .. } => error(id, "Export arrives in M1 (SPEC §19.3)"),
            EngineRequest::Close { doc } => {
                self.docs.remove(&doc);
                vec![EngineEvent::Reply { id, result: Value::Null }]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowster_core::command::Command;

    fn reply(events: &[EngineEvent]) -> &Value {
        match events.last() {
            Some(EngineEvent::Reply { result, .. }) => result,
            other => panic!("expected reply, got {other:?}"),
        }
    }

    #[test]
    fn open_query_exec_close() {
        let mut e = Engine::new();
        let ev = e.open(1, "minimal.vsdx", &flowster_vsdx::fixture::minimal_vsdx());
        assert!(matches!(ev[0], EngineEvent::ModelChanged { .. }));
        let doc = reply(&ev)["doc"].as_str().unwrap().to_owned();
        assert_eq!(reply(&ev)["kind"], "drawing");

        let ev = e.handle(2, EngineRequest::Query { doc: doc.clone(), q: Query::PackageParts });
        let parts = reply(&ev)["parts"].as_array().unwrap();
        assert!(parts.iter().any(|p| p["name"] == "/visio/pages/page1.xml"));
        assert!(reply(&ev)["relationships"].as_array().unwrap().len() >= 5);

        let ev = e.handle(
            3,
            EngineRequest::Exec {
                doc: doc.clone(),
                cmd: Command::SetDocProp { key: "title".into(), value: "T".into() },
            },
        );
        assert!(matches!(ev[1], EngineEvent::UndoState { can_undo: true, .. }));
        let ev = e.handle(4, EngineRequest::Query { doc: doc.clone(), q: Query::DocInfo });
        assert_eq!(reply(&ev)["meta"]["title"], "T");
        // The drawing lives in the CRDT: the fixture's one shape.
        assert_eq!(reply(&ev)["shapes"], 1);

        let ev = e.handle(5, EngineRequest::Close { doc: doc.clone() });
        assert_eq!(reply(&ev), &Value::Null);
        let ev = e.handle(6, EngineRequest::Query { doc, q: Query::DocInfo });
        assert!(matches!(ev[0], EngineEvent::Error { id: Some(6), .. }));
    }

    #[test]
    fn pages_and_display_list() {
        let mut e = Engine::new();
        let ev = e.open(1, "minimal.vsdx", &flowster_vsdx::fixture::minimal_vsdx());
        assert_eq!(reply(&ev)["pageCount"], 1);
        let doc = reply(&ev)["doc"].as_str().unwrap().to_owned();
        let ev = e.handle(2, EngineRequest::Query { doc: doc.clone(), q: Query::Pages });
        assert_eq!(reply(&ev)[0]["name"], "Page-1");
        let ev = e.handle(3, EngineRequest::Query { doc: doc.clone(), q: Query::PageDisplayList { page: 0 } });
        let items = reply(&ev)["items"].as_array().unwrap();
        assert_eq!(items[0]["kind"], "path");
        assert_eq!(items[1]["kind"], "text");
        let ev = e.handle(4, EngineRequest::Query { doc, q: Query::PageDisplayList { page: 7 } });
        assert!(matches!(ev[0], EngineEvent::Error { id: Some(4), .. }));
    }

    #[test]
    fn new_drawing_then_draw_and_undo() {
        use flowster_core::command::ShapeKind;
        let mut e = Engine::new();
        let ev = e.handle(1, EngineRequest::New { title: None });
        let doc = reply(&ev)["doc"].as_str().unwrap().to_owned();
        let add = Command::AddShape { page: 0, shape: ShapeKind::Rectangle, x: 1.0, y: 1.0, w: 2.0, h: 1.0 };
        let ev = e.handle(2, EngineRequest::Exec { doc: doc.clone(), cmd: add });
        assert_eq!(reply(&ev)["created"], serde_json::json!([0, 1]));
        let ev = e.handle(3, EngineRequest::Query { doc: doc.clone(), q: Query::PageDisplayList { page: 0 } });
        assert_eq!(reply(&ev)["shapes"][0]["id"], 1);
        assert_eq!(reply(&ev)["shapes"][0]["width"], 2.0);
        e.handle(4, EngineRequest::Exec { doc: doc.clone(), cmd: Command::Undo });
        let ev = e.handle(5, EngineRequest::Query { doc, q: Query::PageDisplayList { page: 0 } });
        assert_eq!(reply(&ev)["shapes"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn bad_files_are_errors_not_panics() {
        let mut e = Engine::new();
        assert!(matches!(e.open(1, "x.vsdx", b"nope")[0], EngineEvent::Error { .. }));
        assert!(matches!(e.open(2, "x.vsd", b"")[0], EngineEvent::Error { .. }));
        assert!(matches!(e.open(3, "x.png", b"")[0], EngineEvent::Error { .. }));
    }
}
