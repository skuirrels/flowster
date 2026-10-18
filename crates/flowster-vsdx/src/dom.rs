//! A minimal element tree for one XML part. Parts are size-capped before they
//! get here, so building a tree is simpler than a streaming state machine and
//! keeps mixed content (shape text with `<cp/>` markers) in order.
//! DTDs are rejected (§23); only the five predefined entities and character
//! references are expanded.

use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};

use crate::{OpcError, xml_err};

#[derive(Clone, Debug, Default)]
pub struct El {
    /// Local name (namespace prefix dropped).
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub kids: Vec<Node>,
}

#[derive(Clone, Debug)]
pub enum Node {
    El(El),
    Text(String),
}

impl El {
    /// Attribute by local name (`r:id` → `id`).
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn attr_u32(&self, name: &str) -> Option<u32> {
        self.attr(name).and_then(|v| v.trim().parse().ok())
    }

    pub fn attr_bool(&self, name: &str) -> bool {
        matches!(self.attr(name).map(str::trim), Some("1" | "true"))
    }

    pub fn children(&self) -> impl Iterator<Item = &El> {
        self.kids.iter().filter_map(|n| match n {
            Node::El(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a El> + 'a {
        self.children().filter(move |e| e.name == name)
    }

    pub fn child(&self, name: &str) -> Option<&El> {
        self.children().find(|e| e.name == name)
    }

    /// Concatenated text of this element and its descendants.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for n in &self.kids {
            match n {
                Node::Text(t) => s.push_str(t),
                Node::El(e) => s.push_str(&e.text()),
            }
        }
        s
    }
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_owned()
}

fn start(e: &quick_xml::events::BytesStart<'_>, part: &str) -> Result<El, OpcError> {
    let mut attrs = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|err| xml_err(part, err))?;
        let v = a.normalized_value(XmlVersion::Implicit1_0).map_err(|err| xml_err(part, err))?.into_owned();
        let key = a.key.as_ref();
        // Keep namespace declarations out of the attribute list.
        if key == "xmlns" || key.starts_with("xmlns:") {
            continue;
        }
        attrs.push((local(key), v));
    }
    Ok(El { name: local(e.name().as_ref()), attrs, kids: Vec::new() })
}

fn entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => return None,
    })
}

/// Parses a whole part into its root element.
pub fn parse(xml: &[u8], part: &str) -> Result<El, OpcError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut stack: Vec<El> = Vec::new();
    let mut root: Option<El> = None;
    let push_text = |stack: &mut Vec<El>, t: &str| {
        if let Some(top) = stack.last_mut() {
            match top.kids.last_mut() {
                Some(Node::Text(prev)) => prev.push_str(t),
                _ => top.kids.push(Node::Text(t.to_owned())),
            }
        }
    };
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf).map_err(|e| xml_err(part, e))? {
            Event::DocType(_) => return Err(xml_err(part, "DTDs are not allowed")),
            Event::Start(e) => {
                if stack.len() > 256 {
                    return Err(xml_err(part, "elements nested too deeply"));
                }
                stack.push(start(&e, part)?);
            }
            Event::Empty(e) => {
                let el = start(&e, part)?;
                match stack.last_mut() {
                    Some(top) => top.kids.push(Node::El(el)),
                    None => root = Some(el),
                }
            }
            Event::End(_) => {
                let el = stack.pop().ok_or_else(|| xml_err(part, "unbalanced end tag"))?;
                match stack.last_mut() {
                    Some(top) => top.kids.push(Node::El(el)),
                    None => root = Some(el),
                }
            }
            Event::Text(t) => push_text(&mut stack, &t.xml10_content()),
            Event::CData(c) => push_text(&mut stack, &c),
            Event::GeneralRef(r) => {
                if r.is_char_ref() {
                    if let Some(c) = r.resolve_char_ref().map_err(|e| xml_err(part, e))? {
                        push_text(&mut stack, c.encode_utf8(&mut [0; 4]));
                    }
                } else {
                    let name: &str = &r;
                    let rep = entity(name).ok_or_else(|| xml_err(part, format!("unknown entity &{name};")))?;
                    push_text(&mut stack, rep);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    root.ok_or_else(|| xml_err(part, "no root element"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_content_and_entities() {
        let el = parse(b"<a xmlns='x' xmlns:r='y' r:id='1'><t>A &amp; B<cp IX='1'/>C &#x41;</t></a>", "t").unwrap();
        assert_eq!(el.attr("id"), Some("1"));
        let t = el.child("t").unwrap();
        assert_eq!(t.kids.len(), 3);
        assert_eq!(t.text(), "A & BC A");
        assert!(parse(b"<!DOCTYPE a [<!ENTITY e 'x'>]><a>&e;</a>", "t").is_err());
    }
}
