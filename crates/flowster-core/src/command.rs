//! The command bus (SPEC §5.4–§5.5).
//!
//! Every edit is a [`Command`]. A [`CommandTarget`] (the CRDT-backed document)
//! runs each one as a single transaction tagged with the local origin, so undo,
//! macros, plugins and collaboration share one path. v1 grows this enum toward
//! the ~300 `VisUICmds`-style variants; M2 starts with drawing, moving,
//! resizing, deleting and text.

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
    /// Draws a new shape on a page. 2-D shapes fill the box `(x, y)`–`(x + w,
    /// y + h)` (page inches, +Y up); a line runs from `(x, y)` to `(x + w, y + h)`.
    #[serde(rename_all = "camelCase")]
    AddShape {
        page: usize,
        shape: ShapeKind,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    },
    /// Moves top-level shapes by `(dx, dy)` inches.
    #[serde(rename_all = "camelCase")]
    MoveShapes {
        page: usize,
        shapes: Vec<u32>,
        dx: f64,
        dy: f64,
    },
    /// Resizes a 2-D shape: its new pin (page inches) and size.
    #[serde(rename_all = "camelCase")]
    SetBounds {
        page: usize,
        shape: u32,
        pin_x: f64,
        pin_y: f64,
        width: f64,
        height: f64,
    },
    /// Moves the ends of a 1-D shape (page inches).
    #[serde(rename_all = "camelCase")]
    SetEndpoints {
        page: usize,
        shape: u32,
        begin_x: f64,
        begin_y: f64,
        end_x: f64,
        end_y: f64,
    },
    #[serde(rename_all = "camelCase")]
    DeleteShapes {
        page: usize,
        shapes: Vec<u32>,
    },
    /// Replaces a shape's text (plain text; formatting comes from its rows).
    #[serde(rename_all = "camelCase")]
    SetText {
        page: usize,
        shape: u32,
        text: String,
    },
    /// Per-user undo of this peer's last command.
    Undo,
    Redo,
}

/// What the drawing tools create.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
    Line,
}

impl Command {
    /// Label shown in the Undo/Redo menu.
    pub fn label(&self) -> &'static str {
        match self {
            Command::SetDocProp { .. } => "Change Document Property",
            Command::AddShape { shape: ShapeKind::Rectangle, .. } => "Draw Rectangle",
            Command::AddShape { shape: ShapeKind::Ellipse, .. } => "Draw Ellipse",
            Command::AddShape { shape: ShapeKind::Line, .. } => "Draw Line",
            Command::MoveShapes { .. } => "Move",
            Command::SetBounds { .. } | Command::SetEndpoints { .. } => "Resize",
            Command::DeleteShapes { .. } => "Delete",
            Command::SetText { .. } => "Edit Text",
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
    /// The shape a command created (`page`, legacy id), for selecting it.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created: Option<(usize, u32)>,
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
