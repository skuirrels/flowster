//! `[Content_Types].xml` (ECMA-376 Part 2 §10.1.2).

use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use serde::Serialize;

use crate::{OpcError, xml_err};

const PART: &str = "/[Content_Types].xml";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentTypes {
    /// Extension (lower case, no dot) → content type.
    pub defaults: Vec<(String, String)>,
    /// Part name → content type.
    pub overrides: Vec<(String, String)>,
}

impl ContentTypes {
    pub fn parse(xml: &[u8]) -> Result<Self, OpcError> {
        let mut ct = ContentTypes::default();
        let mut reader = Reader::from_reader(xml);
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_event_into(&mut buf).map_err(|e| xml_err(PART, e))? {
                Event::DocType(_) => return Err(xml_err(PART, "DTDs are not allowed")),
                Event::Start(e) | Event::Empty(e) => {
                    let local = e.local_name();
                    let tag = local.as_ref();
                    if tag != "Default" && tag != "Override" {
                        continue;
                    }
                    let mut key = None;
                    let mut value = None;
                    for a in e.attributes() {
                        let a = a.map_err(|e| xml_err(PART, e))?;
                        let v = a.normalized_value(XmlVersion::Implicit1_0).map_err(|e| xml_err(PART, e))?.into_owned();
                        match a.key.local_name().as_ref() {
                            "Extension" | "PartName" => key = Some(v),
                            "ContentType" => value = Some(v),
                            _ => {}
                        }
                    }
                    if let (Some(k), Some(v)) = (key, value) {
                        if tag == "Default" {
                            ct.defaults.push((k.to_ascii_lowercase(), v));
                        } else {
                            ct.overrides.push((k, v));
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }
        Ok(ct)
    }

    /// Override by part name (case-insensitive, §9.1.1.2), else Default by extension.
    pub fn resolve(&self, part_name: &str) -> Option<&str> {
        if let Some((_, v)) = self.overrides.iter().find(|(k, _)| k.eq_ignore_ascii_case(part_name)) {
            return Some(v);
        }
        let file = part_name.rsplit('/').next()?;
        let ext = file.rsplit_once('.')?.1.to_ascii_lowercase();
        self.defaults.iter().find(|(k, _)| *k == ext).map(|(_, v)| v.as_str())
    }
}
