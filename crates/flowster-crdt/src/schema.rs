//! Root keys of the SPEC §20.3 CRDT document.

pub const META: &str = "meta";
pub const PAGES: &str = "pages";
/// uuid → shape map (`type, nameU, legacyId, master, …, sheet, text`).
pub const SHAPE: &str = "shape";
pub const MASTERS: &str = "masters";
pub const STYLES: &str = "styles";
pub const THEMES: &str = "themes";
pub const DATA: &str = "data";
pub const VALIDATION: &str = "validation";
pub const COMMENTS: &str = "comments";
pub const UNKNOWN: &str = "unknown";
pub const ASSETS: &str = "assets";

pub const SHEET: &str = "sheet";
pub const TEXT: &str = "text";

pub fn page_key(page_id: &str) -> String {
    format!("page/{page_id}")
}

pub fn shapes_tree_key(page_id: &str) -> String {
    format!("shapes/{page_id}")
}

/// A master's shape tree.
pub fn master_tree_key(master_id: &str) -> String {
    format!("mshapes/{master_id}")
}
