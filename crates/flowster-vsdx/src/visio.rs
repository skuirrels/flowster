//! VSDX parts → [`flowster_core::model::Document`] (SPEC §19.1).
//!
//! Reads document settings, the colour table, style sheets, pages (with their
//! page sheets, shapes and connects), masters and the media used by Foreign
//! shapes. Unknown elements are skipped here; preserving them for write-back is M2.

use std::collections::BTreeMap;

use flowster_core::model::{
    Cell, Connect, DefaultStyles, Document, ForeignData, Master, Media, Page, Row, Section, Shape, ShapeType, Sheet,
    StyleSheet, TextPiece,
};

use crate::dom::{self, El, Node};
use crate::{OpcError, OpcPackage};

const REL_PAGES: &str = "http://schemas.microsoft.com/visio/2010/relationships/pages";
const REL_MASTERS: &str = "http://schemas.microsoft.com/visio/2010/relationships/masters";
const REL_THEME: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";
/// Media larger than this stay unloaded (drawn as a placeholder).
const MAX_MEDIA_BYTES: u64 = 32 << 20;

/// The raw theme parts of a drawing, in relationship order, kept as assets so
/// a document stored in the CRDT can re-read them (`themes` holds each part
/// whole, SPEC §20.3). The first is the document theme.
pub fn theme_xmls(pkg: &OpcPackage) -> Vec<Vec<u8>> {
    let Some(main) = pkg.info.main_part.clone() else { return Vec::new() };
    pkg.targets_of_type(&main, REL_THEME).iter().filter_map(|p| pkg.read_part(p).ok()).collect()
}

/// Parses a theme part read by [`theme_xml`].
pub fn parse_theme_xml(bytes: &[u8]) -> Option<flowster_core::theme::Theme> {
    dom::parse(bytes, "/visio/theme/theme1.xml").ok().map(|root| crate::theme::parse_theme(&root))
}

/// Reads the Visio document model from an opened package.
pub fn read_document(pkg: &OpcPackage) -> Result<Document, OpcError> {
    let main = pkg.info.main_part.clone().ok_or_else(|| OpcError::NotVisio("no Visio document part".into()))?;
    let root = dom::parse(&pkg.read_part(&main)?, &main)?;
    if root.name != "VisioDocument" {
        return Err(OpcError::NotVisio(format!("{main} is <{}>, not <VisioDocument>", root.name)));
    }

    let mut doc = Document::default();
    if let Some(ds) = root.child("DocumentSettings") {
        doc.default_styles = DefaultStyles {
            line: ds.attr_u32("DefaultLineStyle"),
            fill: ds.attr_u32("DefaultFillStyle"),
            text: ds.attr_u32("DefaultTextStyle"),
        };
    }
    if let Some(colors) = root.child("Colors") {
        for c in colors.children_named("ColorEntry") {
            if let (Some(ix), Some(rgb)) = (c.attr_u32("IX"), c.attr("RGB")) {
                doc.colors.insert(ix, rgb.to_owned());
            }
        }
    }
    if let Some(styles) = root.child("StyleSheets") {
        for s in styles.children_named("StyleSheet") {
            let Some(id) = s.attr_u32("ID") else { continue };
            doc.styles.insert(
                id,
                StyleSheet {
                    id,
                    name_u: s.attr("NameU").map(str::to_owned),
                    line_style: s.attr_u32("LineStyle"),
                    fill_style: s.attr_u32("FillStyle"),
                    text_style: s.attr_u32("TextStyle"),
                    sheet: read_sheet(s),
                },
            );
        }
    }

    for theme_part in pkg.targets_of_type(&main, REL_THEME) {
        match pkg.read_part(&theme_part).and_then(|b| dom::parse(&b, &theme_part)) {
            Ok(root) => doc.themes.push(crate::theme::parse_theme(&root)),
            Err(e) => doc.warnings.push(format!("theme: {e}")),
        }
    }
    doc.theme = doc.themes.first().cloned();

    let mut reader = PartReader { pkg, doc: &mut doc };
    if let Some(masters_part) = pkg.target_of_type(&main, REL_MASTERS) {
        reader.read_masters(&masters_part)?;
    }
    if let Some(pages_part) = pkg.target_of_type(&main, REL_PAGES) {
        reader.read_pages(&pages_part)?;
    }
    Ok(doc)
}

struct PartReader<'a> {
    pkg: &'a OpcPackage,
    doc: &'a mut Document,
}

impl PartReader<'_> {
    fn read_masters(&mut self, part: &str) -> Result<(), OpcError> {
        let root = dom::parse(&self.pkg.read_part(part)?, part)?;
        for m in root.children_named("Master") {
            let Some(id) = m.attr_u32("ID") else { continue };
            let mut master = Master {
                id,
                name_u: m.attr("NameU").map(str::to_owned),
                page_sheet: m.child("PageSheet").map(read_sheet).unwrap_or_default(),
                shapes: Vec::new(),
            };
            if let Some(content) = self.rel_part(part, m) {
                match self.read_contents(&content) {
                    Ok((shapes, _)) => master.shapes = shapes,
                    Err(e) => self.doc.warnings.push(format!("master {id}: {e}")),
                }
            }
            self.doc.masters.insert(id, master);
        }
        Ok(())
    }

    fn read_pages(&mut self, part: &str) -> Result<(), OpcError> {
        let root = dom::parse(&self.pkg.read_part(part)?, part)?;
        for p in root.children_named("Page") {
            let Some(id) = p.attr_u32("ID") else { continue };
            let mut page = Page {
                id,
                name_u: p.attr("NameU").map(str::to_owned),
                name: p.attr("Name").map(str::to_owned),
                background: p.attr_bool("Background"),
                back_page: p.attr_u32("BackPage"),
                sheet: p.child("PageSheet").map(read_sheet).unwrap_or_default(),
                ..Default::default()
            };
            if let Some(content) = self.rel_part(part, p) {
                match self.read_contents(&content) {
                    Ok((shapes, connects)) => {
                        page.shapes = shapes;
                        page.connects = connects;
                    }
                    Err(e) => self.doc.warnings.push(format!("page {id}: {e}")),
                }
            }
            self.doc.pages.push(page);
        }
        Ok(())
    }

    /// The part an element's `<Rel r:id=…/>` child points to.
    fn rel_part(&self, source: &str, el: &El) -> Option<String> {
        let rid = el.child("Rel")?.attr("id")?;
        self.pkg.target(source, rid)
    }

    fn read_contents(&mut self, part: &str) -> Result<(Vec<Shape>, Vec<Connect>), OpcError> {
        let root = dom::parse(&self.pkg.read_part(part)?, part)?;
        let shapes = match root.child("Shapes") {
            Some(s) => s.children_named("Shape").map(|e| self.read_shape(part, e)).collect(),
            None => Vec::new(),
        };
        let connects = root
            .child("Connects")
            .map(|c| {
                c.children_named("Connect")
                    .filter_map(|e| {
                        Some(Connect {
                            from_sheet: e.attr_u32("FromSheet")?,
                            from_cell: e.attr("FromCell").map(str::to_owned),
                            to_sheet: e.attr_u32("ToSheet")?,
                            to_cell: e.attr("ToCell").map(str::to_owned),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok((shapes, connects))
    }

    fn read_shape(&mut self, part: &str, e: &El) -> Shape {
        let kind = match e.attr("Type") {
            Some("Group") => ShapeType::Group,
            Some("Guide") => ShapeType::Guide,
            Some("Foreign") => ShapeType::Foreign,
            _ => ShapeType::Shape,
        };
        let foreign = e.child("ForeignData").map(|fd| {
            let media = fd.child("Rel").and_then(|r| r.attr("id")).and_then(|rid| self.pkg.target(part, rid));
            if let Some(m) = &media {
                self.load_media(m);
            }
            ForeignData {
                foreign_type: fd.attr("ForeignType").unwrap_or("").to_owned(),
                compression: fd.attr("CompressionType").map(str::to_owned),
                part: media,
            }
        });
        Shape {
            id: e.attr_u32("ID").unwrap_or(0),
            kind,
            name_u: e.attr("NameU").map(str::to_owned),
            name: e.attr("Name").map(str::to_owned),
            master: e.attr_u32("Master"),
            master_shape: e.attr_u32("MasterShape"),
            line_style: e.attr_u32("LineStyle"),
            fill_style: e.attr_u32("FillStyle"),
            text_style: e.attr_u32("TextStyle"),
            del: e.attr_bool("Del"),
            sheet: read_sheet(e),
            text: e.child("Text").map(read_text),
            foreign,
            shapes: match e.child("Shapes") {
                Some(s) => s.children_named("Shape").map(|c| self.read_shape(part, c)).collect(),
                None => Vec::new(),
            },
        }
    }

    fn load_media(&mut self, part: &str) {
        if self.doc.media.contains_key(part) {
            return;
        }
        let Some(info) = self.pkg.info.part(part) else { return };
        if info.size > MAX_MEDIA_BYTES {
            self.doc.warnings.push(format!("{part} is too large to display ({} bytes)", info.size));
            return;
        }
        let content_type = info.content_type.clone().unwrap_or_default();
        match self.pkg.read_part(part) {
            Ok(bytes) => {
                self.doc.media.insert(part.to_owned(), Media { content_type, bytes });
            }
            Err(e) => self.doc.warnings.push(format!("{part}: {e}")),
        }
    }
}

fn read_cells(e: &El) -> BTreeMap<String, Cell> {
    e.children_named("Cell")
        .filter_map(|c| {
            Some((
                c.attr("N")?.to_owned(),
                Cell {
                    v: c.attr("V").map(str::to_owned),
                    f: c.attr("F").map(str::to_owned),
                    u: c.attr("U").map(str::to_owned),
                },
            ))
        })
        .collect()
}

/// Cells and sections of a sheet element (Shape, PageSheet, StyleSheet, …).
pub(crate) fn read_sheet(e: &El) -> Sheet {
    let sections = e
        .children_named("Section")
        .map(|s| Section {
            n: s.attr("N").unwrap_or("").to_owned(),
            ix: s.attr_u32("IX"),
            del: s.attr_bool("Del"),
            cells: read_cells(s),
            rows: s
                .children_named("Row")
                .map(|r| Row {
                    t: r.attr("T").map(str::to_owned),
                    n: r.attr("N").map(str::to_owned),
                    ix: r.attr_u32("IX"),
                    del: r.attr_bool("Del"),
                    cells: read_cells(r),
                })
                .collect(),
        })
        .collect();
    Sheet { cells: read_cells(e), sections }
}

fn read_text(e: &El) -> Vec<TextPiece> {
    let mut out = Vec::new();
    for n in &e.kids {
        match n {
            Node::Text(t) => out.push(TextPiece::Text(t.clone())),
            Node::El(el) => {
                let ix = el.attr_u32("IX").unwrap_or(0);
                match el.name.as_str() {
                    "cp" => out.push(TextPiece::Cp(ix)),
                    "pp" => out.push(TextPiece::Pp(ix)),
                    "tp" => out.push(TextPiece::Tp(ix)),
                    "fld" => {
                        out.push(TextPiece::Fld(ix));
                        out.push(TextPiece::Text(el.text()));
                    }
                    _ => out.push(TextPiece::Text(el.text())),
                }
            }
        }
    }
    out
}
