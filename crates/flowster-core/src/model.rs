//! The document model as read from a package (SPEC §6.1): ShapeSheets with
//! their cells' formulas (`F`), cached values (`V`) and units (`U`), exactly as
//! stored. Inheritance (§6.3) is applied on read by [`crate::resolve`], never by
//! copying values, so a later write can tell local cells from inherited ones.

use std::collections::BTreeMap;

/// One ShapeSheet cell.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Cell {
    /// Cached value in internal units (inches, radians, …), as written by the producer.
    pub v: Option<String>,
    pub f: Option<String>,
    pub u: Option<String>,
}

impl Cell {
    pub fn value(v: impl Into<String>) -> Self {
        Cell { v: Some(v.into()), f: None, u: None }
    }
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Row {
    /// Row type for geometry rows (`MoveTo`, `LineTo`, `NURBSTo`, …).
    pub t: Option<String>,
    /// Name for named rows (`User.Foo`, `Prop.Cost`, `Control.TextPosition`).
    pub n: Option<String>,
    pub ix: Option<u32>,
    pub del: bool,
    pub cells: BTreeMap<String, Cell>,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Section {
    pub n: String,
    /// Index for multi-instance sections (`Geometry` 0, 1, …).
    pub ix: Option<u32>,
    pub del: bool,
    /// Section-level cells (e.g. `NoFill`, `NoLine`, `NoShow` on Geometry).
    pub cells: BTreeMap<String, Cell>,
    pub rows: Vec<Row>,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Sheet {
    pub cells: BTreeMap<String, Cell>,
    pub sections: Vec<Section>,
}

impl Sheet {
    pub fn cell(&self, name: &str) -> Option<&Cell> {
        self.cells.get(name)
    }
    pub fn sections_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Section> + 'a {
        self.sections.iter().filter(move |s| s.n == name)
    }
    pub fn section(&self, name: &str, ix: Option<u32>) -> Option<&Section> {
        self.sections.iter().find(|s| s.n == name && (ix.is_none() || s.ix == ix))
    }
}

/// Shape text with its run markers (`<cp/>`, `<pp/>`, `<tp/>`, `<fld/>`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TextPiece {
    Text(String),
    /// Following characters use Character row `IX`.
    Cp(u32),
    /// Following paragraph uses Paragraph row `IX`.
    Pp(u32),
    Tp(u32),
    /// A field; its cached display text follows as a `Text` piece.
    Fld(u32),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ShapeType {
    #[default]
    Shape,
    Group,
    Guide,
    Foreign,
}

/// Embedded image or object of a `Foreign` shape.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ForeignData {
    pub foreign_type: String,
    pub compression: Option<String>,
    /// Part name of the payload (`/visio/media/image1.png`), if stored as a part.
    pub part: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Shape {
    pub id: u32,
    pub kind: ShapeType,
    pub name_u: Option<String>,
    pub name: Option<String>,
    pub master: Option<u32>,
    pub master_shape: Option<u32>,
    pub line_style: Option<u32>,
    pub fill_style: Option<u32>,
    pub text_style: Option<u32>,
    pub del: bool,
    pub sheet: Sheet,
    pub text: Option<Vec<TextPiece>>,
    pub foreign: Option<ForeignData>,
    pub shapes: Vec<Shape>,
}

impl Shape {
    /// Depth-first search among this shape and its descendants.
    pub fn find(&self, id: u32) -> Option<&Shape> {
        if self.id == id {
            return Some(self);
        }
        self.shapes.iter().find_map(|s| s.find(id))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleSheet {
    pub id: u32,
    pub name_u: Option<String>,
    /// `BasedOn` styles per category.
    pub line_style: Option<u32>,
    pub fill_style: Option<u32>,
    pub text_style: Option<u32>,
    pub sheet: Sheet,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Master {
    pub id: u32,
    pub name_u: Option<String>,
    pub page_sheet: Sheet,
    pub shapes: Vec<Shape>,
}

impl Master {
    pub fn find(&self, id: u32) -> Option<&Shape> {
        self.shapes.iter().find_map(|s| s.find(id))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Connect {
    pub from_sheet: u32,
    pub from_cell: Option<String>,
    pub to_sheet: u32,
    pub to_cell: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub id: u32,
    pub name_u: Option<String>,
    pub name: Option<String>,
    pub background: bool,
    pub back_page: Option<u32>,
    pub sheet: Sheet,
    pub shapes: Vec<Shape>,
    pub connects: Vec<Connect>,
}

impl Page {
    /// Hidden from the page tabs (PageSheet `UIVisibility` = 1).
    pub fn hidden(&self) -> bool {
        self.sheet.cell("UIVisibility").and_then(|c| c.v.as_deref()).and_then(|v| v.parse::<f64>().ok()) == Some(1.0)
    }
}

/// Default style ids from `DocumentSettings`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DefaultStyles {
    pub line: Option<u32>,
    pub fill: Option<u32>,
    pub text: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    /// Pages in file order.
    pub pages: Vec<Page>,
    pub masters: BTreeMap<u32, Master>,
    pub styles: BTreeMap<u32, StyleSheet>,
    /// Document colour table entries beyond the built-in palette (`IX` → `#rrggbb`).
    pub colors: BTreeMap<u32, String>,
    pub default_styles: DefaultStyles,
    /// The document theme (`None` for "No theme" drawings): the first of `themes`.
    pub theme: Option<crate::theme::Theme>,
    /// Every theme part. Pages and shapes pick one with `ThemeIndex` ([`Document::theme_by_index`]).
    pub themes: Vec<crate::theme::Theme>,
    /// Media payloads by part name (images referenced by Foreign shapes).
    pub media: BTreeMap<String, Media>,
    /// Non-fatal problems met while reading.
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Media {
    pub content_type: String,
    pub bytes: Vec<u8>,
}

impl Document {
    pub fn page(&self, id: u32) -> Option<&Page> {
        self.pages.iter().find(|p| p.id == id)
    }

    /// The theme a `ThemeIndex` value selects: 0 is "no theme"; an index no
    /// theme part declares falls back to the document theme.
    pub fn theme_by_index(&self, index: u32) -> Option<&crate::theme::Theme> {
        if index == 0 {
            return None;
        }
        self.themes.iter().find(|t| t.ids.contains(&index)).or(self.theme.as_ref())
    }
}

/// `ThemeIndex` and the other scheme indices: 65534 means "not set here".
pub const THEME_INDEX_UNSET: u32 = 65534;
