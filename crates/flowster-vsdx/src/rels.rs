//! Relationship parts (ECMA-376 Part 2 §9.3).

use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use serde::Serialize;

use crate::part_name::resolve_target;
use crate::{OpcError, xml_err};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Relationship {
    /// Source part name, `/` for package relationships.
    pub source: String,
    pub id: String,
    pub rel_type: String,
    /// Target exactly as written.
    pub target: String,
    /// `TargetMode="External"` (URLs, file links): never resolved to a part.
    pub external: bool,
    /// Absolute part name for internal targets.
    pub target_part: Option<String>,
    pub target_exists: bool,
}

pub(crate) fn parse(source: &str, rels_part: &str, xml: &[u8]) -> Result<Vec<Relationship>, OpcError> {
    let mut out = Vec::new();
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf).map_err(|e| xml_err(rels_part, e))? {
            Event::DocType(_) => return Err(xml_err(rels_part, "DTDs are not allowed")),
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == "Relationship" => {
                let (mut id, mut ty, mut target, mut external) = (String::new(), String::new(), String::new(), false);
                for a in e.attributes() {
                    let a = a.map_err(|e| xml_err(rels_part, e))?;
                    let v =
                        a.normalized_value(XmlVersion::Implicit1_0).map_err(|e| xml_err(rels_part, e))?.into_owned();
                    match a.key.local_name().as_ref() {
                        "Id" => id = v,
                        "Type" => ty = v,
                        "Target" => target = v,
                        "TargetMode" => external = v.eq_ignore_ascii_case("External"),
                        _ => {}
                    }
                }
                let target_part = if external { None } else { resolve_target(source, &target) };
                out.push(Relationship {
                    source: source.to_owned(),
                    id,
                    rel_type: ty,
                    target,
                    external,
                    target_part,
                    target_exists: false,
                });
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}
