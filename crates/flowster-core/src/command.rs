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
    /// Draws a library shape (`crate::library`) on a page. 2-D shapes fill the
    /// box `(x, y)`–`(x + w, y + h)` (page inches, +Y up); a line runs from
    /// `(x, y)` to `(x + w, y + h)`.
    #[serde(rename_all = "camelCase")]
    AddShape {
        page: usize,
        shape: String,
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
    /// Moves the ends of a 1-D shape (page inches). For a connector, an end
    /// with a glue target follows that shape; one without comes unglued.
    #[serde(rename_all = "camelCase")]
    SetEndpoints {
        page: usize,
        shape: u32,
        begin_x: f64,
        begin_y: f64,
        end_x: f64,
        end_y: f64,
        #[serde(default)]
        begin_glue: Option<u32>,
        #[serde(default)]
        end_glue: Option<u32>,
    },
    /// Draws a dynamic connector, glued at either end to a shape when given.
    #[serde(rename_all = "camelCase")]
    AddConnector {
        page: usize,
        from: Option<u32>,
        to: Option<u32>,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    },
    /// Copies a shape one step in `direction` (`up`, `down`, `left`, `right`)
    /// and connects the two (Visio's AutoConnect).
    #[serde(rename_all = "camelCase")]
    AutoConnect {
        page: usize,
        shape: u32,
        direction: String,
    },
    #[serde(rename_all = "camelCase")]
    DeleteShapes {
        page: usize,
        shapes: Vec<u32>,
    },
    /// Sets ShapeSheet cells on shapes (formatting: fill, line, font, …).
    #[serde(rename_all = "camelCase")]
    SetCells {
        page: usize,
        shapes: Vec<u32>,
        cells: Vec<CellEdit>,
    },
    /// Replaces a shape's text (plain text; formatting comes from its rows).
    #[serde(rename_all = "camelCase")]
    SetText {
        page: usize,
        shape: u32,
        text: String,
    },
    /// Aligns shapes to the first one (the primary selection): `left`,
    /// `center`, `right`, `top`, `middle` or `bottom`.
    #[serde(rename_all = "camelCase")]
    AlignShapes {
        page: usize,
        shapes: Vec<u32>,
        align: String,
    },
    /// Spaces three or more shapes evenly along `horizontal` or `vertical`;
    /// the outermost two stay put.
    #[serde(rename_all = "camelCase")]
    DistributeShapes {
        page: usize,
        shapes: Vec<u32>,
        axis: String,
    },
    /// Turns shapes a quarter turn about the selection's centre:
    /// `left` (counter-clockwise) or `right`.
    #[serde(rename_all = "camelCase")]
    RotateShapes {
        page: usize,
        shapes: Vec<u32>,
        direction: String,
    },
    /// Mirrors shapes about the selection's centre: `horizontal` or `vertical`.
    #[serde(rename_all = "camelCase")]
    FlipShapes {
        page: usize,
        shapes: Vec<u32>,
        axis: String,
    },
    /// Changes the stacking order: `front`, `back`, `forward` or `backward`.
    #[serde(rename_all = "camelCase")]
    ReorderShapes {
        page: usize,
        shapes: Vec<u32>,
        to: String,
    },
    /// Puts top-level shapes into a new group.
    #[serde(rename_all = "camelCase")]
    GroupShapes {
        page: usize,
        shapes: Vec<u32>,
    },
    /// Breaks groups apart; their members return to the page.
    #[serde(rename_all = "camelCase")]
    UngroupShapes {
        page: usize,
        shapes: Vec<u32>,
    },
    /// Inserts a blank foreground page after `after` (after the last
    /// foreground page when absent), with the page setup of the page it follows.
    #[serde(rename_all = "camelCase")]
    AddPage {
        #[serde(default)]
        after: Option<usize>,
    },
    #[serde(rename_all = "camelCase")]
    DeletePage {
        page: usize,
    },
    #[serde(rename_all = "camelCase")]
    RenamePage {
        page: usize,
        name: String,
    },
    /// Hides a page from the page tabs, or shows it again (`UIVisibility`).
    #[serde(rename_all = "camelCase")]
    SetPageHidden {
        page: usize,
        hidden: bool,
    },
    /// Sets a page's paper size in inches (Design > Size). `paper` names a
    /// standard size by its `PaperKind` (see `crate::paper`). A custom size
    /// leaves the printer paper as it was and turns Auto Size off, so the page
    /// keeps it. Orientation follows the size.
    #[serde(rename_all = "camelCase")]
    SetPageSize {
        page: usize,
        width: f64,
        height: f64,
        #[serde(default)]
        paper: Option<u32>,
    },
    /// Turns a page to portrait or landscape (Design > Orientation): width and
    /// height swap if they need to, and the printer paper turns with them.
    #[serde(rename_all = "camelCase")]
    SetPageOrientation {
        page: usize,
        landscape: bool,
    },
    /// Fits the page to its shapes with a small margin, moving them in by it
    /// (Design > Size > Fit to Drawing). Turns Auto Size off.
    #[serde(rename_all = "camelCase")]
    FitPageToDrawing {
        page: usize,
    },
    /// Design > Auto Size (`DrawingResizeType`): the page grows and shrinks by
    /// whole sheets of its paper to hold its shapes after every edit.
    #[serde(rename_all = "camelCase")]
    SetPageAutoSize {
        page: usize,
        on: bool,
    },
    /// Pastes shapes (from the `copyShapes` query) onto a page, moved by
    /// `(dx, dy)` inches. They get new ids; glue between pasted shapes is kept,
    /// glue to anything else comes undone.
    #[serde(rename_all = "camelCase")]
    InsertShapes {
        page: usize,
        shapes: Vec<crate::model::Shape>,
        dx: f64,
        dy: f64,
    },
    /// Per-user undo of this peer's last command.
    Undo,
    Redo,
}

/// One cell to set. `section` names a row section (`Character`, `Paragraph`);
/// without `row` the value goes to every row the shape has (or row 0).
/// `value` is in internal units (inches, `#rrggbb`, flags).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellEdit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<u32>,
    pub cell: String,
    pub value: String,
}

impl Command {
    /// Label shown in the Undo/Redo menu.
    pub fn label(&self) -> &'static str {
        match self {
            Command::SetDocProp { .. } => "Change Document Property",
            Command::AddShape { shape, .. } if shape == "line" => "Draw Line",
            Command::AddShape { shape, .. } if shape == "text" => "Add Text",
            Command::AddShape { .. } => "Add Shape",
            Command::MoveShapes { .. } => "Move",
            Command::SetBounds { .. } | Command::SetEndpoints { .. } => "Resize",
            Command::DeleteShapes { .. } => "Delete",
            Command::SetText { .. } => "Edit Text",
            // The rotate handle sets only the angle.
            Command::SetCells { cells, .. } if cells.iter().all(|c| c.cell == "Angle") => "Rotate",
            Command::SetCells { .. } => "Format",
            Command::AddConnector { .. } => "Connect Shapes",
            Command::InsertShapes { .. } => "Paste",
            Command::AutoConnect { .. } => "AutoConnect",
            Command::AlignShapes { .. } => "Align Shapes",
            Command::DistributeShapes { .. } => "Distribute Shapes",
            Command::RotateShapes { .. } => "Rotate",
            Command::FlipShapes { .. } => "Flip",
            Command::ReorderShapes { to, .. } => match to.as_str() {
                "front" => "Bring to Front",
                "back" => "Send to Back",
                "forward" => "Bring Forward",
                _ => "Send Backward",
            },
            Command::GroupShapes { .. } => "Group",
            Command::UngroupShapes { .. } => "Ungroup",
            Command::AddPage { .. } => "Insert Page",
            Command::DeletePage { .. } => "Delete Page",
            Command::RenamePage { .. } => "Rename Page",
            Command::SetPageHidden { hidden: true, .. } => "Hide Page",
            Command::SetPageHidden { .. } => "Unhide Page",
            Command::SetPageSize { .. } => "Page Size",
            Command::SetPageOrientation { .. } => "Page Orientation",
            Command::FitPageToDrawing { .. } => "Fit to Drawing",
            Command::SetPageAutoSize { .. } => "Auto Size",
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
    /// The page list changed; the page to show next.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub page: Option<usize>,
    /// The page grew or shrank on its left or bottom edge, so every shape
    /// moved by `(dx, dy)` paper inches. A view pans the same way to keep them still.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub moved: Option<(f64, f64)>,
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
