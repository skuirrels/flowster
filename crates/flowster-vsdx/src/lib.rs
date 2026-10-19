//! `flowster-vsdx`: Open Packaging Conventions (ECMA-376 Part 2) reader for
//! Visio packages (`.vsdx .vssx .vstx .vsdm .vssm .vstm`).
//!
//! M0 lists what a package contains: every part with its content type and
//! sizes, and every relationship resolved to an absolute part name. Reading
//! parts into the document model is M1.
//!
//! Hardening (SPEC §23): part-name validation (no `..`, no absolute or drive
//! paths, no backslashes), zip-bomb caps (part count, part size, total size,
//! compression ratio) and XML with DTDs rejected.

#![forbid(unsafe_code)]

mod content_types;
mod dom;
pub mod fixture;
mod part_name;
mod rels;
mod theme;
pub mod visio;
pub mod writer;

use std::collections::HashMap;
use std::io::{Cursor, Read, Seek};
use std::sync::Mutex;

use serde::Serialize;

pub use content_types::ContentTypes;
pub use part_name::{normalize_part_name, rels_part_for, source_part_for_rels};
pub use rels::Relationship;

/// Relationship type of the Visio main document part.
pub const REL_VISIO_DOCUMENT: &str = "http://schemas.microsoft.com/visio/2010/relationships/document";

#[derive(Debug, thiserror::Error)]
pub enum OpcError {
    #[error("not a ZIP/OPC package: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed XML in {part}: {message}")]
    Xml { part: String, message: String },
    #[error("unsafe part name {0:?}")]
    UnsafePartName(String),
    #[error("package limit exceeded: {0}")]
    Limit(String),
    #[error("missing [Content_Types].xml")]
    MissingContentTypes,
    #[error("no part named {0}")]
    MissingPart(String),
    #[error("not a Visio drawing: {0}")]
    NotVisio(String),
}

/// Caps applied while reading untrusted packages.
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_parts: usize,
    pub max_part_size: u64,
    pub max_total_size: u64,
    /// Max uncompressed/compressed ratio for any part larger than 1 MiB.
    pub max_ratio: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_parts: 20_000, max_part_size: 512 << 20, max_total_size: 2 << 30, max_ratio: 200 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartInfo {
    /// Absolute part name, e.g. `/visio/pages/page1.xml`.
    pub name: String,
    pub content_type: Option<String>,
    pub size: u64,
    pub compressed_size: u64,
    pub compression: String,
    /// `true` for `_rels/*.rels` parts.
    pub is_relationships: bool,
}

/// What kind of Visio package this is, from the main part's content type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PackageKind {
    Drawing,
    Stencil,
    Template,
    DrawingMacro,
    StencilMacro,
    TemplateMacro,
    Unknown,
}

impl PackageKind {
    pub fn from_content_type(ct: &str) -> Self {
        match ct {
            "application/vnd.ms-visio.drawing.main+xml" => PackageKind::Drawing,
            "application/vnd.ms-visio.stencil.main+xml" => PackageKind::Stencil,
            "application/vnd.ms-visio.template.main+xml" => PackageKind::Template,
            "application/vnd.ms-visio.drawing.macroEnabled.main+xml" => PackageKind::DrawingMacro,
            "application/vnd.ms-visio.stencil.macroEnabled.main+xml" => PackageKind::StencilMacro,
            "application/vnd.ms-visio.template.macroEnabled.main+xml" => PackageKind::TemplateMacro,
            _ => PackageKind::Unknown,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    pub kind: PackageKind,
    /// The part targeted by the package-level Visio document relationship.
    pub main_part: Option<String>,
    pub parts: Vec<PartInfo>,
    pub relationships: Vec<Relationship>,
    pub content_types: ContentTypes,
    pub total_size: u64,
    /// Non-fatal problems: dangling relationship targets, parts without a content type, …
    pub warnings: Vec<String>,
}

impl Package {
    pub fn part(&self, name: &str) -> Option<&PartInfo> {
        self.parts.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }
}

/// An opened package: the directory plus on-demand access to part bytes.
pub struct OpcPackage {
    pub info: Package,
    limits: Limits,
    zip: Mutex<zip::ZipArchive<Cursor<Vec<u8>>>>,
    /// Lower-cased part name → ZIP entry index (part names are case-insensitive).
    index: HashMap<String, usize>,
}

impl OpcPackage {
    pub fn open(bytes: Vec<u8>) -> Result<Self, OpcError> {
        Self::open_with(bytes, Limits::default())
    }

    pub fn open_with(bytes: Vec<u8>, limits: Limits) -> Result<Self, OpcError> {
        let info = read_package_with(Cursor::new(bytes.as_slice()), &limits)?;
        let zip = zip::ZipArchive::new(Cursor::new(bytes))?;
        let mut index = HashMap::with_capacity(zip.len());
        for i in 0..zip.len() {
            if let Some(Ok(raw)) = zip.name_for_index(i)
                && let Some(name) = normalize_part_name(&raw)
            {
                index.insert(name.to_ascii_lowercase(), i);
            }
        }
        Ok(OpcPackage { info, limits, zip: Mutex::new(zip), index })
    }

    /// Bytes of a part, capped at the package limits.
    pub fn read_part(&self, name: &str) -> Result<Vec<u8>, OpcError> {
        let i = *self.index.get(&name.to_ascii_lowercase()).ok_or_else(|| OpcError::MissingPart(name.to_owned()))?;
        let mut zip = self.zip.lock().map_err(|_| OpcError::MissingPart(name.to_owned()))?;
        let mut entry = zip.by_index(i)?;
        let mut buf = Vec::with_capacity(entry.size().min(self.limits.max_part_size) as usize);
        (&mut entry).take(self.limits.max_part_size + 1).read_to_end(&mut buf)?;
        if buf.len() as u64 > self.limits.max_part_size {
            return Err(OpcError::Limit(format!("{name} exceeds {} bytes", self.limits.max_part_size)));
        }
        Ok(buf)
    }

    /// Internal target part of relationship `rid` from `source`.
    pub fn target(&self, source: &str, rid: &str) -> Option<String> {
        self.info
            .relationships
            .iter()
            .find(|r| r.source.eq_ignore_ascii_case(source) && r.id == rid && !r.external)
            .and_then(|r| r.target_part.clone())
    }

    /// Target of the first relationship of `rel_type` from `source`.
    pub fn target_of_type(&self, source: &str, rel_type: &str) -> Option<String> {
        self.info
            .relationships
            .iter()
            .find(|r| r.source.eq_ignore_ascii_case(source) && r.rel_type == rel_type && !r.external)
            .and_then(|r| r.target_part.clone())
    }
}

/// Reads the package directory from in-memory bytes (the WASM path).
pub fn read_package(bytes: &[u8]) -> Result<Package, OpcError> {
    read_package_with(Cursor::new(bytes), &Limits::default())
}

pub fn read_package_with<R: Read + Seek>(reader: R, limits: &Limits) -> Result<Package, OpcError> {
    let mut zip = zip::ZipArchive::new(reader)?;
    if zip.len() > limits.max_parts {
        return Err(OpcError::Limit(format!("{} entries (max {})", zip.len(), limits.max_parts)));
    }

    let mut parts = Vec::with_capacity(zip.len());
    let mut total: u64 = 0;
    let mut content_types_xml: Option<Vec<u8>> = None;
    let mut rels_xml: Vec<(String, Vec<u8>)> = Vec::new();

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let raw = entry.name()?.into_owned();
        let name = normalize_part_name(&raw).ok_or_else(|| OpcError::UnsafePartName(raw.clone()))?;
        let (size, csize) = (entry.size(), entry.compressed_size());
        if size > limits.max_part_size {
            return Err(OpcError::Limit(format!("{name} is {size} bytes (max {})", limits.max_part_size)));
        }
        if size > (1 << 20) && size / csize.max(1) > limits.max_ratio {
            return Err(OpcError::Limit(format!(
                "{name} compression ratio {} (max {})",
                size / csize.max(1),
                limits.max_ratio
            )));
        }
        total += size;
        if total > limits.max_total_size {
            return Err(OpcError::Limit(format!("package expands past {} bytes", limits.max_total_size)));
        }

        let is_ct = name == "/[Content_Types].xml";
        let is_rels = part_name::is_rels_part(&name);
        if is_ct || is_rels {
            // Read through a hard cap: the declared size can lie.
            let mut buf = Vec::with_capacity(size.min(1 << 20) as usize);
            (&mut entry).take(limits.max_part_size + 1).read_to_end(&mut buf)?;
            if buf.len() as u64 > limits.max_part_size {
                return Err(OpcError::Limit(format!("{name} exceeds its declared size")));
            }
            if is_ct {
                content_types_xml = Some(buf);
                continue; // not a part in the OPC sense
            }
            rels_xml.push((name.clone(), buf));
        }
        parts.push(PartInfo {
            name,
            content_type: None,
            size,
            compressed_size: csize,
            compression: format!("{:?}", entry.compression()),
            is_relationships: is_rels,
        });
    }

    let content_types = ContentTypes::parse(&content_types_xml.ok_or(OpcError::MissingContentTypes)?)?;
    let mut warnings = Vec::new();
    for p in &mut parts {
        p.content_type = content_types.resolve(&p.name).map(str::to_owned);
        if p.content_type.is_none() {
            warnings.push(format!("{} has no content type", p.name));
        }
    }

    let mut relationships = Vec::new();
    rels_xml.sort_by(|a, b| a.0.cmp(&b.0));
    for (rels_name, xml) in &rels_xml {
        let source = source_part_for_rels(rels_name).unwrap_or_else(|| "/".into());
        relationships.extend(rels::parse(&source, rels_name, xml)?);
    }
    for r in &mut relationships {
        if let Some(t) = &r.target_part {
            r.target_exists = parts.iter().any(|p| p.name.eq_ignore_ascii_case(t));
            if !r.target_exists {
                warnings.push(format!("{} {} → {} does not exist", r.source, r.id, t));
            }
        }
    }

    let main_part = relationships
        .iter()
        .find(|r| r.source == "/" && r.rel_type == REL_VISIO_DOCUMENT)
        .and_then(|r| r.target_part.clone());
    let kind = main_part
        .as_deref()
        .and_then(|m| content_types.resolve(m))
        .map(PackageKind::from_content_type)
        .unwrap_or(PackageKind::Unknown);

    parts.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Package { kind, main_part, parts, relationships, content_types, total_size: total, warnings })
}

/// XML parse failure in `part`. Readers also return this for any DTD (§23: no entity expansion).
pub(crate) fn xml_err(part: &str, e: impl std::fmt::Display) -> OpcError {
    OpcError::Xml { part: part.to_owned(), message: e.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_parts_and_relationships_of_minimal_drawing() {
        let pkg = read_package(&fixture::minimal_vsdx()).unwrap();
        assert_eq!(pkg.kind, PackageKind::Drawing);
        assert_eq!(pkg.main_part.as_deref(), Some("/visio/document.xml"));
        let names: Vec<&str> = pkg.parts.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"/visio/pages/page1.xml"));
        assert!(names.contains(&"/_rels/.rels"));
        assert!(!names.contains(&"/[Content_Types].xml"));
        assert_eq!(
            pkg.part("/visio/pages/page1.xml").unwrap().content_type.as_deref(),
            Some("application/vnd.ms-visio.page+xml")
        );
        // Default (by extension) content type.
        assert_eq!(
            pkg.part("/visio/_rels/document.xml.rels").unwrap().content_type.as_deref(),
            Some("application/vnd.openxmlformats-package.relationships+xml")
        );
        let page = pkg.relationships.iter().find(|r| r.source == "/visio/pages/pages.xml" && r.id == "rId1").unwrap();
        assert_eq!(page.target_part.as_deref(), Some("/visio/pages/page1.xml"));
        assert!(page.target_exists);
        let ext = pkg.relationships.iter().find(|r| r.external).unwrap();
        assert_eq!(ext.target, "https://example.com/flowster");
        assert_eq!(ext.target_part, None);
        assert!(pkg.warnings.is_empty(), "{:?}", pkg.warnings);
    }

    #[test]
    fn rejects_traversal_and_missing_content_types() {
        let bad = fixture::build(&[
            ("../evil.xml", b"<x/>".as_slice()),
            ("[Content_Types].xml", fixture::CONTENT_TYPES.as_bytes()),
        ]);
        assert!(matches!(read_package(&bad), Err(OpcError::UnsafePartName(_))));
        let bad = fixture::build(&[("C:/evil.xml", b"<x/>".as_slice())]);
        assert!(matches!(read_package(&bad), Err(OpcError::UnsafePartName(_))));
        let no_ct = fixture::build(&[("visio/document.xml", b"<x/>".as_slice())]);
        assert!(matches!(read_package(&no_ct), Err(OpcError::MissingContentTypes)));
        assert!(read_package(b"not a zip").is_err());
    }

    #[test]
    fn rejects_doctype() {
        let evil = r#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY e SYSTEM "file:///etc/passwd">]><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#;
        let pkg = fixture::build(&[("[Content_Types].xml", evil.as_bytes())]);
        assert!(matches!(read_package(&pkg), Err(OpcError::Xml { .. })));
    }

    #[test]
    fn enforces_size_limits() {
        let big = vec![b' '; 4 << 20];
        let pkg = fixture::build(&[("[Content_Types].xml", fixture::CONTENT_TYPES.as_bytes()), ("big.xml", &big)]);
        let limits = Limits { max_part_size: 1 << 20, ..Limits::default() };
        assert!(matches!(read_package_with(Cursor::new(&pkg), &limits), Err(OpcError::Limit(_))));
        // 4 MiB of spaces deflates far past 200:1.
        assert!(matches!(read_package(&pkg), Err(OpcError::Limit(_))));
    }
}
