//! The command bus (SPEC §5.4–§5.5).
//!
//! Every edit is a [`Command`]. A [`CommandTarget`] (the CRDT-backed document)
//! runs each one as a single transaction tagged with the local origin, so undo,
//! macros, plugins and collaboration share one path. v1 grows this enum toward
//! the ~300 `VisUICmds`-style variants; M0 has just enough to prove the path.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Command {
    /// Sets a document property (`meta` in the CRDT), e.g. `title`.
    #[serde(rename_all = "camelCase")]
    SetDocProp {
        key: String,
        value: String,
    },
    /// Per-user undo of this peer's last command.
    Undo,
    Redo,
}

impl Command {
    /// Label shown in the Undo/Redo menu.
    pub fn label(&self) -> &'static str {
        match self {
            Command::SetDocProp { .. } => "Change Document Property",
            Command::Undo => "Undo",
            Command::Redo => "Redo",
        }
    }
}

/// What a command changed, used to drive panes (`modelChanged`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSummary {
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub meta: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub shapes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub opened: Option<OpenedSummary>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedSummary {
    pub file_name: String,
    pub format: String,
    pub part_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandError {
    NotSupported(&'static str),
    Failed(String),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::NotSupported(what) => write!(f, "not supported yet: {what}"),
            CommandError::Failed(why) => write!(f, "command failed: {why}"),
        }
    }
}

impl std::error::Error for CommandError {}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoState {
    pub can_undo: bool,
    pub can_redo: bool,
    pub labels: Vec<String>,
}

/// Something commands run against: one transaction per command.
pub trait CommandTarget {
    fn exec(&mut self, cmd: &Command) -> Result<ChangeSummary, CommandError>;
    fn undo_state(&self) -> UndoState;
}
