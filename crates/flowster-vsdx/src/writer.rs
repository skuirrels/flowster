//! [`flowster_core::model::Document`] → a VSDX package (SPEC §19.2).
//!
//! Writes the parts Flowster models: the document part (settings, colours,
//! style sheets), pages and masters with their shapes, the theme part as
//! stored, media, and a minimal windows part (Visio requires one). Parts Flowster does not model yet (windows, comments,
//! data recordsets, VBA) are not written, so a re-saved file loses them.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Cursor, Write};

use flowster_core::model::{Cell, Document, Row, Section, Shape, ShapeType, Sheet, TextPiece};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const NS_VISIO: &str = "http://schemas.microsoft.com/office/visio/2012/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL_DOCUMENT: &str = "http://schemas.microsoft.com/visio/2010/relationships/document";
const REL_PAGES: &str = "http://schemas.microsoft.com/visio/2010/relationships/pages";
const REL_PAGE: &str = "http://schemas.microsoft.com/visio/2010/relationships/page";
const REL_MASTERS: &str = "http://schemas.microsoft.com/visio/2010/relationships/masters";
const REL_MASTER: &str = "http://schemas.microsoft.com/visio/2010/relationships/master";
const REL_WINDOWS: &str = "http://schemas.microsoft.com/visio/2010/relationships/windows";
const REL_THEME: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";
const REL_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const REL_CORE: &str = "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
const REL_APP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";

/// XML-escapes text and attribute values.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // Characters XML 1.0 cannot carry are dropped.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

fn attr(out: &mut String, name: &str, v: Option<&str>) {
    if let Some(v) = v {
        let _ = write!(out, r#" {name}="{}""#, esc(v));
    }
}

fn attr_u32(out: &mut String, name: &str, v: Option<u32>) {
    if let Some(v) = v {
        let _ = write!(out, r#" {name}="{v}""#);
    }
}

fn cells(out: &mut String, cells: &BTreeMap<String, Cell>) {
    for (name, c) in cells {
        out.push_str("<Cell");
        attr(out, "N", Some(name));
        attr(out, "V", c.v.as_deref());
        attr(out, "U", c.u.as_deref());
        attr(out, "F", c.f.as_deref());
        out.push_str("/>");
    }
}

fn row(out: &mut String, r: &Row) {
    out.push_str("<Row");
    attr(out, "T", r.t.as_deref());
    attr(out, "N", r.n.as_deref());
    attr_u32(out, "IX", r.ix);
    if r.del {
        out.push_str(r#" Del="1""#);
    }
    out.push('>');
    cells(out, &r.cells);
    out.push_str("</Row>");
}

fn section(out: &mut String, s: &Section) {
    out.push_str("<Section");
    attr(out, "N", Some(&s.n));
    attr_u32(out, "IX", s.ix);
    if s.del {
        out.push_str(r#" Del="1""#);
    }
    out.push('>');
    cells(out, &s.cells);
    for r in &s.rows {
        row(out, r);
    }
    out.push_str("</Section>");
}

/// A sheet's cells and sections (the element itself is the caller's).
fn sheet(out: &mut String, s: &Sheet) {
    cells(out, &s.cells);
    for sec in &s.sections {
        section(out, sec);
    }
}

fn text(out: &mut String, pieces: &[TextPiece]) {
    out.push_str("<Text>");
    let mut it = pieces.iter().peekable();
    while let Some(p) = it.next() {
        match p {
            TextPiece::Text(t) => out.push_str(&esc(t)),
            TextPiece::Cp(ix) => {
                let _ = write!(out, r#"<cp IX="{ix}"/>"#);
            }
            TextPiece::Pp(ix) => {
                let _ = write!(out, r#"<pp IX="{ix}"/>"#);
            }
            TextPiece::Tp(ix) => {
                let _ = write!(out, r#"<tp IX="{ix}"/>"#);
            }
            TextPiece::Fld(ix) => {
                // A field carries its cached display text inside the element.
                let shown = match it.peek() {
                    Some(TextPiece::Text(t)) => {
                        let t = esc(t);
                        it.next();
                        t
                    }
                    _ => String::new(),
                };
                let _ = write!(out, r#"<fld IX="{ix}">{shown}</fld>"#);
            }
        }
    }
    out.push_str("</Text>");
}

/// Relationships of one part, numbered as they are added.
#[derive(Default)]
struct Rels {
    list: Vec<(String, &'static str, String, bool)>,
}

impl Rels {
    fn add(&mut self, rel_type: &'static str, target: &str) -> String {
        if let Some((id, ..)) = self.list.iter().find(|(_, t, tg, _)| *t == rel_type && tg == target) {
            return id.clone();
        }
        let id = format!("rId{}", self.list.len() + 1);
        self.list.push((id.clone(), rel_type, target.to_owned(), false));
        id
    }

    fn xml(&self) -> String {
        let mut out = String::from(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        );
        for (id, t, target, external) in &self.list {
            let _ = write!(out, r#"<Relationship Id="{id}" Type="{t}" Target="{}""#, esc(target));
            if *external {
                out.push_str(r#" TargetMode="External""#);
            }
            out.push_str("/>");
        }
        out.push_str("</Relationships>");
        out
    }
}

/// Writes shapes, collecting the media they reference as relationships of
/// the part being written (`media` maps part names to their package path).
fn shapes(out: &mut String, list: &[Shape], rels: &mut Rels, media_used: &mut Vec<String>) {
    for s in list {
        out.push_str("<Shape");
        attr_u32(out, "ID", Some(s.id));
        attr(out, "NameU", s.name_u.as_deref());
        attr(out, "Name", s.name.as_deref());
        let kind = match s.kind {
            ShapeType::Shape => "Shape",
            ShapeType::Group => "Group",
            ShapeType::Guide => "Guide",
            ShapeType::Foreign => "Foreign",
        };
        attr(out, "Type", Some(kind));
        attr_u32(out, "Master", s.master);
        attr_u32(out, "MasterShape", s.master_shape);
        attr_u32(out, "LineStyle", s.line_style);
        attr_u32(out, "FillStyle", s.fill_style);
        attr_u32(out, "TextStyle", s.text_style);
        if s.del {
            out.push_str(r#" Del="1""#);
        }
        out.push('>');
        sheet(out, &s.sheet);
        // Schema order: cells, sections, Text, ForeignData, Shapes.
        if let Some(t) = &s.text {
            text(out, t);
        }
        if let Some(f) = &s.foreign {
            out.push_str("<ForeignData");
            attr(out, "ForeignType", Some(&f.foreign_type));
            attr(out, "CompressionType", f.compression.as_deref());
            out.push('>');
            if let Some(part) = &f.part {
                // Media live in /visio/media; pages and masters are one level down.
                let name = part.rsplit('/').next().unwrap_or(part);
                let id = rels.add(REL_IMAGE, &format!("../media/{name}"));
                if !media_used.contains(part) {
                    media_used.push(part.clone());
                }
                let _ = write!(out, r#"<Rel r:id="{id}"/>"#);
            }
            out.push_str("</ForeignData>");
        }
        if !s.shapes.is_empty() {
            out.push_str("<Shapes>");
            shapes(out, &s.shapes, rels, media_used);
            out.push_str("</Shapes>");
        }
        out.push_str("</Shape>");
    }
}

fn header(root: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><{root} xmlns="{NS_VISIO}" xmlns:r="{NS_R}" xml:space="preserve">"#
    )
}

fn media_content_type(name: &str, stored: &str) -> String {
    if !stored.is_empty() {
        return stored.to_owned();
    }
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "emf" => "image/x-emf",
        "wmf" => "image/x-wmf",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        _ => "application/octet-stream",
    }
    .to_owned()
}

/// Writes `doc` as a `.vsdx` package. `theme_xml` is the theme part as stored;
/// `title` goes into the core properties.
pub fn write_vsdx(doc: &Document, theme_xml: Option<&[u8]>, title: &str) -> Result<Vec<u8>, String> {
    let mut parts: Vec<(String, String, Vec<u8>)> = Vec::new(); // (name, content type, bytes)
    let mut doc_rels = Rels::default();
    let mut media_used: Vec<String> = Vec::new();

    // Masters.
    if !doc.masters.is_empty() {
        let mut list = header("Masters");
        let mut rels = Rels::default();
        for (n, m) in doc.masters.values().enumerate() {
            let file = format!("master{}.xml", n + 1);
            let rid = rels.add(REL_MASTER, &file);
            list.push_str("<Master");
            attr_u32(&mut list, "ID", Some(m.id));
            attr(&mut list, "NameU", m.name_u.as_deref());
            attr(&mut list, "Name", m.name_u.as_deref());
            list.push_str("><PageSheet>");
            sheet(&mut list, &m.page_sheet);
            let _ = write!(list, r#"</PageSheet><Rel r:id="{rid}"/></Master>"#);
            let mut content = header("MasterContents");
            let mut content_rels = Rels::default();
            content.push_str("<Shapes>");
            shapes(&mut content, &m.shapes, &mut content_rels, &mut media_used);
            content.push_str("</Shapes></MasterContents>");
            parts.push((format!("visio/masters/{file}"), "application/vnd.ms-visio.master+xml".into(), content.into()));
            if !content_rels.list.is_empty() {
                parts.push((format!("visio/masters/_rels/{file}.rels"), String::new(), content_rels.xml().into()));
            }
        }
        list.push_str("</Masters>");
        parts.push(("visio/masters/masters.xml".into(), "application/vnd.ms-visio.masters+xml".into(), list.into()));
        parts.push(("visio/masters/_rels/masters.xml.rels".into(), String::new(), rels.xml().into()));
        doc_rels.add(REL_MASTERS, "masters/masters.xml");
    }

    // Pages.
    let mut list = header("Pages");
    let mut rels = Rels::default();
    for (n, p) in doc.pages.iter().enumerate() {
        let file = format!("page{}.xml", n + 1);
        let rid = rels.add(REL_PAGE, &file);
        list.push_str("<Page");
        attr_u32(&mut list, "ID", Some(p.id));
        attr(&mut list, "NameU", p.name_u.as_deref());
        attr(&mut list, "Name", p.name.as_deref().or(p.name_u.as_deref()));
        if p.background {
            list.push_str(r#" Background="1""#);
        }
        attr_u32(&mut list, "BackPage", p.back_page);
        list.push_str("><PageSheet>");
        sheet(&mut list, &p.sheet);
        let _ = write!(list, r#"</PageSheet><Rel r:id="{rid}"/></Page>"#);
        let mut content = header("PageContents");
        let mut content_rels = Rels::default();
        if !p.shapes.is_empty() {
            content.push_str("<Shapes>");
            shapes(&mut content, &p.shapes, &mut content_rels, &mut media_used);
            content.push_str("</Shapes>");
        }
        if !p.connects.is_empty() {
            content.push_str("<Connects>");
            for c in &p.connects {
                content.push_str("<Connect");
                attr_u32(&mut content, "FromSheet", Some(c.from_sheet));
                attr(&mut content, "FromCell", c.from_cell.as_deref());
                attr_u32(&mut content, "ToSheet", Some(c.to_sheet));
                attr(&mut content, "ToCell", c.to_cell.as_deref());
                content.push_str("/>");
            }
            content.push_str("</Connects>");
        }
        content.push_str("</PageContents>");
        parts.push((format!("visio/pages/{file}"), "application/vnd.ms-visio.page+xml".into(), content.into()));
        if !content_rels.list.is_empty() {
            parts.push((format!("visio/pages/_rels/{file}.rels"), String::new(), content_rels.xml().into()));
        }
    }
    list.push_str("</Pages>");
    parts.push(("visio/pages/pages.xml".into(), "application/vnd.ms-visio.pages+xml".into(), list.into()));
    parts.push(("visio/pages/_rels/pages.xml.rels".into(), String::new(), rels.xml().into()));
    doc_rels.add(REL_PAGES, "pages/pages.xml");

    // Theme and media.
    if let Some(xml) = theme_xml {
        parts.push((
            "visio/theme/theme1.xml".into(),
            "application/vnd.openxmlformats-officedocument.theme+xml".into(),
            xml.to_vec(),
        ));
        doc_rels.add(REL_THEME, "theme/theme1.xml");
    }
    for part in &media_used {
        let Some(m) = doc.media.get(part) else { continue };
        let name = part.rsplit('/').next().unwrap_or(part);
        parts.push((format!("visio/media/{name}"), media_content_type(name, &m.content_type), m.bytes.clone()));
    }

    // Visio will not open a drawing without a windows part: one drawing
    // window on the first foreground page, zoomed to fit.
    let first = doc.pages.iter().find(|p| !p.background).or(doc.pages.first()).map_or(0, |p| p.id);
    let windows = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Windows ClientWidth="1280" ClientHeight="800" xmlns="{NS_VISIO}" xmlns:r="{NS_R}" xml:space="preserve"><Window ID="0" WindowType="Drawing" WindowState="1073741824" ContainerType="Page" Page="{first}" ViewScale="-1"/></Windows>"#
    );
    parts.push(("visio/windows.xml".into(), "application/vnd.ms-visio.windows+xml".into(), windows.into()));
    doc_rels.add(REL_WINDOWS, "windows.xml");

    // The document part.
    let mut d = header("VisioDocument");
    d.push_str("<DocumentSettings");
    attr_u32(&mut d, "DefaultLineStyle", doc.default_styles.line);
    attr_u32(&mut d, "DefaultFillStyle", doc.default_styles.fill);
    attr_u32(&mut d, "DefaultTextStyle", doc.default_styles.text);
    d.push_str("/>");
    if !doc.colors.is_empty() {
        d.push_str("<Colors>");
        for (ix, rgb) in &doc.colors {
            let _ = write!(d, r#"<ColorEntry IX="{ix}" RGB="{}"/>"#, esc(rgb));
        }
        d.push_str("</Colors>");
    }
    if !doc.styles.is_empty() {
        d.push_str("<StyleSheets>");
        for st in doc.styles.values() {
            d.push_str("<StyleSheet");
            attr_u32(&mut d, "ID", Some(st.id));
            attr(&mut d, "NameU", st.name_u.as_deref());
            attr(&mut d, "Name", st.name_u.as_deref());
            attr_u32(&mut d, "LineStyle", st.line_style);
            attr_u32(&mut d, "FillStyle", st.fill_style);
            attr_u32(&mut d, "TextStyle", st.text_style);
            d.push('>');
            sheet(&mut d, &st.sheet);
            d.push_str("</StyleSheet>");
        }
        d.push_str("</StyleSheets>");
    }
    d.push_str("</VisioDocument>");
    parts.push(("visio/document.xml".into(), "application/vnd.ms-visio.drawing.main+xml".into(), d.into()));
    parts.push(("visio/_rels/document.xml.rels".into(), String::new(), doc_rels.xml().into()));

    // Package-level parts.
    let core = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>{}</dc:title></cp:coreProperties>"#,
        esc(title)
    );
    parts.push((
        "docProps/core.xml".into(),
        "application/vnd.openxmlformats-package.core-properties+xml".into(),
        core.into(),
    ));
    parts.push((
        "docProps/app.xml".into(),
        "application/vnd.openxmlformats-officedocument.extended-properties+xml".into(),
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Flowster</Application></Properties>"#.into(),
    ));
    let mut root_rels = Rels::default();
    root_rels.add(REL_DOCUMENT, "visio/document.xml");
    root_rels.add(REL_CORE, "docProps/core.xml");
    root_rels.add(REL_APP, "docProps/app.xml");
    parts.push(("_rels/.rels".into(), String::new(), root_rels.xml().into()));

    let mut types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>"#,
    );
    for (name, ct, _) in &parts {
        if !ct.is_empty() {
            let _ = write!(types, r#"<Override PartName="/{}" ContentType="{}"/>"#, esc(name), esc(ct));
        }
    }
    types.push_str("</Types>");

    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut entries: Vec<(&str, &[u8])> = vec![("[Content_Types].xml", types.as_bytes())];
    entries.extend(parts.iter().map(|(n, _, b)| (n.as_str(), b.as_slice())));
    for (name, bytes) in entries {
        zip.start_file(name, opts).map_err(|e| e.to_string())?;
        zip.write_all(bytes).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_round_trips_through_write_and_read() {
        let pkg = crate::OpcPackage::open(crate::fixture::minimal_vsdx()).unwrap();
        let doc = crate::visio::read_document(&pkg).unwrap();
        let bytes = write_vsdx(&doc, None, "Round trip").unwrap();
        let back = crate::visio::read_document(&crate::OpcPackage::open(bytes).unwrap()).unwrap();
        assert_eq!(back.pages, doc.pages);
        assert_eq!(back.styles, doc.styles);
        assert_eq!(back.masters, doc.masters);
    }

    #[test]
    fn text_and_fields_are_escaped_and_kept() {
        let mut out = String::new();
        text(
            &mut out,
            &[TextPiece::Cp(0), TextPiece::Text("a < b & c".into()), TextPiece::Fld(2), TextPiece::Text("12".into())],
        );
        assert_eq!(out, r#"<Text><cp IX="0"/>a &lt; b &amp; c<fld IX="2">12</fld></Text>"#);
    }
}
