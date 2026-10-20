//! Engine ↔ UI protocol (SPEC §5.4). Mirrors `ui/src/engine/protocol.ts`.
//!
//! Requests travel UI → worker as `{ id, req }`; the worker answers with a
//! stream of [`EngineEvent`]s, ending with `reply` or `error` for that id.
//! `open` carries the file bytes out of band (a transferable `ArrayBuffer`).

use serde::{Deserialize, Serialize};

use crate::command::{ChangeSummary, Command};

pub type DocId = String;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EngineRequest {
    #[serde(rename_all = "camelCase")]
    Open {
        file_name: String,
    },
    /// A new, blank drawing.
    New {
        #[serde(default)]
        title: Option<String>,
    },
    Exec {
        doc: DocId,
        cmd: Command,
    },
    /// Pointer stream for tools (M2). Accepted and ignored in M0.
    Gesture {
        doc: DocId,
        g: serde_json::Value,
    },
    Query {
        doc: DocId,
        q: Query,
    },
    Save {
        doc: DocId,
        format: SaveFormat,
    },
    Export {
        doc: DocId,
        format: String,
        opts: serde_json::Value,
    },
    Close {
        doc: DocId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Query {
    /// OPC parts, content types and relationships of the opened package.
    PackageParts,
    /// Document properties as stored in the CRDT.
    DocInfo,
    /// Hash of a slice of derived state (determinism check in the UI).
    DerivedStateHash,
    /// Pages with their paper sizes.
    Pages,
    /// Display list of one page (index into `Pages`), backgrounds included.
    PageDisplayList { page: usize },
    /// The built-in shape library with preview outlines.
    Library,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SaveFormat {
    Vsdx,
    Vssx,
    Vstx,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncStateKind {
    Local,
    Connecting,
    Live,
    Offline,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EngineEvent {
    /// Binary display-list patches arrive in M1; the variant exists so the UI can switch on it.
    #[serde(rename_all = "camelCase")]
    DisplayListPatch {
        doc: DocId,
        page: u32,
        patch_len: usize,
    },
    SelectionChanged {
        doc: DocId,
        sel: Vec<ShapeRef>,
    },
    ModelChanged {
        doc: DocId,
        changes: ChangeSummary,
    },
    Overlay {
        doc: DocId,
        items: Vec<serde_json::Value>,
    },
    #[serde(rename_all = "camelCase")]
    UndoState {
        doc: DocId,
        can_undo: bool,
        can_redo: bool,
        labels: Vec<String>,
    },
    Presence {
        doc: DocId,
        peers: Vec<serde_json::Value>,
    },
    #[serde(rename_all = "camelCase")]
    SyncState {
        doc: DocId,
        state: SyncStateKind,
        pending_ops: u32,
    },
    /// Successful answer to request `id`.
    Reply {
        id: u32,
        result: serde_json::Value,
    },
    /// Failed request `id` (`None` for errors not tied to a request).
    Error {
        id: Option<u32>,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapeRef {
    pub page: u32,
    pub id: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requests_use_the_ts_wire_shape() {
        let r: EngineRequest = serde_json::from_value(json!({"kind":"open","fileName":"a.vsdx"})).unwrap();
        assert_eq!(r, EngineRequest::Open { file_name: "a.vsdx".into() });
        let r: EngineRequest =
            serde_json::from_value(json!({"kind":"query","doc":"d1","q":{"kind":"packageParts"}})).unwrap();
        assert_eq!(r, EngineRequest::Query { doc: "d1".into(), q: Query::PackageParts });
        let r: EngineRequest = serde_json::from_value(
            json!({"kind":"exec","doc":"d1","cmd":{"kind":"setDocProp","key":"title","value":"x"}}),
        )
        .unwrap();
        assert!(matches!(r, EngineRequest::Exec { cmd: Command::SetDocProp { .. }, .. }));
    }

    #[test]
    fn events_use_the_ts_wire_shape() {
        let e = EngineEvent::UndoState { doc: "d1".into(), can_undo: true, can_redo: false, labels: vec![] };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({"kind":"undoState","doc":"d1","canUndo":true,"canRedo":false,"labels":[]})
        );
        let e = EngineEvent::SyncState { doc: "d1".into(), state: SyncStateKind::Local, pending_ops: 0 };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            json!({"kind":"syncState","doc":"d1","state":"local","pendingOps":0})
        );
    }
}
