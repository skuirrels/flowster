//! OPC part names (ECMA-376 Part 2 §9.1.1) and `.rels` naming.

/// Turns a ZIP entry name into an absolute part name (`/a/b.xml`), or `None`
/// if the name is unsafe or not a valid part name: empty segments, `.`/`..`
/// segments, backslashes, drive letters, NUL, or a trailing slash.
pub fn normalize_part_name(zip_name: &str) -> Option<String> {
    let name = zip_name.strip_prefix('/').unwrap_or(zip_name);
    if name.is_empty() || name.ends_with('/') || name.contains('\\') || name.contains('\0') {
        return None;
    }
    if name.len() >= 2 && name.as_bytes()[1] == b':' {
        return None;
    }
    for seg in name.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." || seg.ends_with('.') {
            return None;
        }
    }
    Some(format!("/{name}"))
}

pub(crate) fn is_rels_part(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".rels") && (lower.contains("/_rels/") || lower.starts_with("/_rels/"))
}

/// `/visio/_rels/document.xml.rels` → `/visio/document.xml`; `/_rels/.rels` → `/`.
pub fn source_part_for_rels(rels_name: &str) -> Option<String> {
    let (dir, file) = rels_name.rsplit_once("/_rels/")?;
    let file = file.strip_suffix(".rels")?;
    if file.is_empty() {
        return Some(if dir.is_empty() { "/".into() } else { format!("{dir}/") });
    }
    Some(format!("{dir}/{file}"))
}

/// `/visio/document.xml` → `/visio/_rels/document.xml.rels`; `/` → `/_rels/.rels`.
pub fn rels_part_for(source: &str) -> String {
    if source == "/" {
        return "/_rels/.rels".into();
    }
    let (dir, file) = source.rsplit_once('/').unwrap_or(("", source));
    format!("{dir}/_rels/{file}.rels")
}

/// Resolves a relative relationship target against its source part.
pub(crate) fn resolve_target(source: &str, target: &str) -> Option<String> {
    let target = target.split('#').next().unwrap_or(target);
    let joined = if let Some(abs) = target.strip_prefix('/') {
        abs.to_owned()
    } else {
        let base = match source.rsplit_once('/') {
            Some((dir, _)) => dir.trim_start_matches('/'),
            None => "",
        };
        if base.is_empty() { target.to_owned() } else { format!("{base}/{target}") }
    };
    let mut out: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            s => out.push(s),
        }
    }
    if out.is_empty() {
        return None;
    }
    normalize_part_name(&out.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(normalize_part_name("visio/document.xml").as_deref(), Some("/visio/document.xml"));
        assert_eq!(normalize_part_name("/visio/document.xml").as_deref(), Some("/visio/document.xml"));
        for bad in ["../x", "a/../b", "a//b", "a\\b", "C:/x", "a/", "", "a/./b"] {
            assert_eq!(normalize_part_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn rels_names() {
        assert_eq!(source_part_for_rels("/_rels/.rels").as_deref(), Some("/"));
        assert_eq!(source_part_for_rels("/visio/_rels/document.xml.rels").as_deref(), Some("/visio/document.xml"));
        assert_eq!(rels_part_for("/"), "/_rels/.rels");
        assert_eq!(rels_part_for("/visio/pages/pages.xml"), "/visio/pages/_rels/pages.xml.rels");
    }

    #[test]
    fn targets() {
        assert_eq!(resolve_target("/", "visio/document.xml").as_deref(), Some("/visio/document.xml"));
        assert_eq!(resolve_target("/visio/document.xml", "pages/pages.xml").as_deref(), Some("/visio/pages/pages.xml"));
        assert_eq!(
            resolve_target("/visio/pages/page1.xml", "../masters/m.xml").as_deref(),
            Some("/visio/masters/m.xml")
        );
        assert_eq!(resolve_target("/visio/document.xml", "/docProps/app.xml").as_deref(), Some("/docProps/app.xml"));
        assert_eq!(resolve_target("/a.xml", "../../x"), None);
    }
}
