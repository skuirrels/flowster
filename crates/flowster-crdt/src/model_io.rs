//! The Visio model in the CRDT (SPEC §20.3): writing a model read from a file
//! into a Loro document, and reading it back for the engine to render.
//!
//! Layout (root containers):
//!
//! * `pages` lists page uuids; `page/<uuid>` holds the page's props and page
//!   sheet; `shapes/<uuid>` is the page's shape tree (child order = z-order).
//! * `shape` maps shape uuid → shape map (props, `sheet`, `text`, `foreign`),
//!   for page and master shapes alike; `mshapes/<uuid>` is a master's tree.
//! * `masters` and `styles` map uuid → map; `themes` maps uuid → theme asset.
//! * `meta` holds the colour table, default styles and the media index.
//!
//! A sheet is `{ cells, groups, sections }`. The atomic cell groups (Position,
//! Size, …) are each ONE value, so concurrent edits of a group cannot mix.
//! Indexed rows are movable lists, named rows (User, Prop, …) maps keyed by
//! name. Cells are `[v, f, u]` arrays (one LWW register each).
//!
//! Shape text is a rich-text container whose `cp`/`pp`/`tp` marks point at the
//! Character/Paragraph/Tabs rows still in the sheet, and whose fields carry an
//! `fld` mark over their cached text. (Turning row references into inline
//! formatting marks comes with text editing in M2.)
//!
//! Identity: every page, shape, master and style gets a uuid derived from an
//! import seed and its legacy id, and keeps the legacy id for VSDX round trips.

use std::collections::{BTreeMap, HashMap};

use flowster_core::model::{
    Cell, Connect, DefaultStyles, Document as Model, ForeignData, Master, Media, Page, Row, Section, Shape, ShapeType,
    Sheet, StyleSheet, TextPiece,
};
use flowster_core::theme::Theme;
use loro::{
    Container, ExpandType, LoroDoc, LoroMap, LoroMovableList, LoroText, LoroTree, LoroValue, StyleConfig,
    StyleConfigMap, TreeID, ValueOrContainer,
};

use crate::schema;

/// Cells stored together as one value (SPEC §20.3 "atomic cell groups").
pub const GROUPS: &[(&str, &[&str])] = &[
    ("Position", &["PinX", "PinY"]),
    ("Size", &["Width", "Height", "LocPinX", "LocPinY"]),
    ("Rotation", &["Angle"]),
    ("Flip", &["FlipX", "FlipY"]),
    ("Endpoints", &["BeginX", "BeginY", "EndX", "EndY"]),
    ("TextTransform", &["TxtPinX", "TxtPinY", "TxtWidth", "TxtHeight", "TxtLocPinX", "TxtLocPinY", "TxtAngle"]),
];

/// Content-addressed payloads kept beside the CRDT (SPEC §20.3 `assets`):
/// images and the theme part. The CRDT holds only their hashes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Assets {
    pub blobs: BTreeMap<String, Media>,
}

impl Assets {
    fn put(&mut self, content_type: &str, bytes: &[u8]) -> String {
        let key = hex128(hash128(0, bytes));
        self.blobs
            .entry(key.clone())
            .or_insert_with(|| Media { content_type: content_type.to_owned(), bytes: bytes.to_vec() });
        key
    }
}

/// Text marks: formatting row references extend over typed text, fields do not.
pub fn configure(doc: &LoroDoc) {
    let mut styles = StyleConfigMap::new();
    for key in ["cp", "pp", "tp"] {
        styles.insert(key.into(), StyleConfig { expand: ExpandType::After });
    }
    styles.insert("fld".into(), StyleConfig { expand: ExpandType::None });
    doc.config_text_style(styles);
}

// ---------------------------------------------------------------- identity

fn fnv(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ seed;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn hash128(seed: u64, bytes: &[u8]) -> u128 {
    (u128::from(fnv(seed, bytes)) << 64) | u128::from(fnv(seed ^ 0x9e37_79b9_7f4a_7c15, bytes))
}

fn hex128(v: u128) -> String {
    format!("{v:032x}")
}

/// A version-8 (custom) uuid for an imported object.
fn uuid(seed: u64, kind: u8, a: u32, b: u32) -> String {
    let mut key = [0u8; 9];
    key[0] = kind;
    key[1..5].copy_from_slice(&a.to_le_bytes());
    key[5..9].copy_from_slice(&b.to_le_bytes());
    let v = hash128(seed, &key);
    let v = (v & !(0xF << 76) & !(0x3 << 62)) | (0x8 << 76) | (0x2 << 62);
    let h = hex128(v);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// A seed for [`write_model`] from the file's bytes.
pub fn seed_of(bytes: &[u8]) -> u64 {
    fnv(0, bytes)
}

// ------------------------------------------------------------------ writing

struct Writer<'a> {
    doc: &'a LoroDoc,
    seed: u64,
}

/// A `CellEntry` as a positional `[v, f, u]` array, trailing nulls dropped
/// (smaller in snapshots and updates than a `{v, f, u}` map; ADR 0001).
fn cell_value(c: &Cell) -> LoroValue {
    let mut parts: Vec<LoroValue> =
        [&c.v, &c.f, &c.u].iter().map(|v| v.as_deref().map_or(LoroValue::Null, LoroValue::from)).collect();
    while parts.last() == Some(&LoroValue::Null) {
        parts.pop();
    }
    parts.into()
}

fn group_of(cell: &str) -> Option<&'static str> {
    GROUPS.iter().find(|(_, cells)| cells.contains(&cell)).map(|(g, _)| *g)
}

fn section_key(s: &Section) -> String {
    match s.ix {
        Some(ix) => format!("{}.{ix}", s.n),
        None => s.n.clone(),
    }
}

fn write_row(m: &LoroMap, row: &Row, pos: usize) {
    m.insert("@pos", pos as i64).unwrap();
    if let Some(t) = &row.t {
        m.insert("@t", t.as_str()).unwrap();
    }
    if let Some(n) = &row.n {
        m.insert("@n", n.as_str()).unwrap();
    }
    if let Some(ix) = row.ix {
        m.insert("@ix", i64::from(ix)).unwrap();
    }
    if row.del {
        m.insert("@del", true).unwrap();
    }
    for (name, c) in &row.cells {
        m.insert(name, cell_value(c)).unwrap();
    }
}

/// `m[key]` as a map container, created on first use (empty containers cost
/// snapshot space, so sheets create only the parts they have).
fn lazy_map(m: &LoroMap, key: &str) -> LoroMap {
    child_map(m, key).unwrap_or_else(|| m.insert_container(key, LoroMap::new()).unwrap())
}

fn write_sheet(m: &LoroMap, sheet: &Sheet) {
    let mut groups: BTreeMap<&str, HashMap<String, LoroValue>> = BTreeMap::new();
    for (name, c) in &sheet.cells {
        match group_of(name) {
            Some(g) => {
                groups.entry(g).or_default().insert(name.clone(), cell_value(c));
            }
            None => lazy_map(m, "cells").insert(name, cell_value(c)).unwrap(),
        }
    }
    for (g, v) in groups {
        lazy_map(m, "groups").insert(g, v).unwrap();
    }
    if sheet.sections.is_empty() {
        return;
    }
    let sections = m.insert_container("sections", LoroMap::new()).unwrap();
    for (pos, s) in sheet.sections.iter().enumerate() {
        let mut key = section_key(s);
        if sections.get(&key).is_some() {
            key = format!("{key}#{pos}");
        }
        let sm = sections.insert_container(&key, LoroMap::new()).unwrap();
        sm.insert("@pos", pos as i64).unwrap();
        sm.insert("@n", s.n.as_str()).unwrap();
        if let Some(ix) = s.ix {
            sm.insert("@ix", i64::from(ix)).unwrap();
        }
        if s.del {
            sm.insert("@del", true).unwrap();
        }
        for (name, c) in &s.cells {
            lazy_map(&sm, "cells").insert(name, cell_value(c)).unwrap();
        }
        let mut names: Vec<&str> = s.rows.iter().filter_map(|r| r.n.as_deref()).collect();
        names.sort_unstable();
        names.dedup();
        let named = !s.rows.is_empty() && names.len() == s.rows.len();
        if s.rows.is_empty() {
            continue;
        }
        if named {
            let rows = sm.insert_container("rows", LoroMap::new()).unwrap();
            for (i, r) in s.rows.iter().enumerate() {
                let n = r.n.as_deref().unwrap_or_default();
                write_row(&rows.insert_container(n, LoroMap::new()).unwrap(), r, i);
            }
        } else {
            let rows = sm.insert_container("rows", LoroMovableList::new()).unwrap();
            for (i, r) in s.rows.iter().enumerate() {
                write_row(&rows.push_container(LoroMap::new()).unwrap(), r, i);
            }
        }
    }
}

fn write_text(t: &LoroText, pieces: &[TextPiece]) {
    let (mut cp, mut pp, mut tp, mut fld) = (None, None, None, None);
    let mut pos = 0usize;
    for piece in pieces {
        match piece {
            TextPiece::Cp(ix) => cp = Some(*ix),
            TextPiece::Pp(ix) => pp = Some(*ix),
            TextPiece::Tp(ix) => tp = Some(*ix),
            TextPiece::Fld(ix) => fld = Some(*ix),
            TextPiece::Text(s) => {
                let n = s.chars().count();
                if n == 0 {
                    continue;
                }
                t.insert(pos, s).unwrap();
                for (key, v) in [("cp", cp), ("pp", pp), ("tp", tp), ("fld", fld.take())] {
                    if let Some(ix) = v {
                        t.mark(pos..pos + n, key, i64::from(ix)).unwrap();
                    }
                }
                pos += n;
            }
        }
    }
}

impl Writer<'_> {
    fn shape_map(&self, uuid: &str) -> LoroMap {
        self.doc.get_map(schema::SHAPE).insert_container(uuid, LoroMap::new()).unwrap()
    }

    fn write_shapes(
        &self,
        tree: &LoroTree,
        parent: Option<TreeID>,
        shapes: &[Shape],
        uuid_of: &dyn Fn(u32) -> String,
        media: &BTreeMap<String, String>,
    ) {
        for s in shapes {
            let node = tree.create(parent).unwrap();
            let id = uuid_of(s.id);
            tree.get_meta(node).unwrap().insert("s", id.as_str()).unwrap();
            let m = self.shape_map(&id);
            let kind = match s.kind {
                ShapeType::Shape => "shape",
                ShapeType::Group => "group",
                ShapeType::Guide => "guide",
                ShapeType::Foreign => "foreign",
            };
            m.insert("type", kind).unwrap();
            m.insert("legacyId", i64::from(s.id)).unwrap();
            if let Some(v) = &s.name_u {
                m.insert("nameU", v.as_str()).unwrap();
            }
            if let Some(v) = &s.name {
                m.insert("name", v.as_str()).unwrap();
            }
            if let Some(v) = s.master {
                m.insert("master", uuid(self.seed, b'm', v, 0)).unwrap();
            }
            if let Some(v) = s.master_shape {
                m.insert("masterShape", i64::from(v)).unwrap();
            }
            for (key, v) in [("lineStyle", s.line_style), ("fillStyle", s.fill_style), ("textStyle", s.text_style)] {
                if let Some(v) = v {
                    m.insert(key, uuid(self.seed, b'y', v, 0)).unwrap();
                }
            }
            if s.del {
                m.insert("del", true).unwrap();
            }
            write_sheet(&m.insert_container(schema::SHEET, LoroMap::new()).unwrap(), &s.sheet);
            if let Some(pieces) = &s.text {
                write_text(&m.insert_container(schema::TEXT, LoroText::new()).unwrap(), pieces);
            }
            if let Some(f) = &s.foreign {
                let mut v: HashMap<String, LoroValue> = HashMap::new();
                v.insert("type".into(), f.foreign_type.as_str().into());
                if let Some(c) = &f.compression {
                    v.insert("compression".into(), c.as_str().into());
                }
                if let Some(p) = &f.part {
                    v.insert("part".into(), p.as_str().into());
                    if let Some(h) = media.get(p) {
                        v.insert("asset".into(), h.as_str().into());
                    }
                }
                m.insert("foreign", v).unwrap();
            }
            if !s.shapes.is_empty() {
                self.write_shapes(tree, Some(node), &s.shapes, uuid_of, media);
            }
        }
    }
}

fn tree(doc: &LoroDoc, key: &str) -> LoroTree {
    let t = doc.get_tree(key);
    if !t.is_fractional_index_enabled() {
        t.enable_fractional_index(0);
    }
    t
}

/// Writes `model` into `doc` (uncommitted) and returns the payloads it refers
/// to. `theme_xml` is the raw theme part, `seed` makes the uuids (see [`seed_of`]).
pub fn write_model(doc: &LoroDoc, model: &Model, theme_xml: Option<&[u8]>, seed: u64) -> Assets {
    let w = Writer { doc, seed };
    let mut assets = Assets::default();
    let meta = doc.get_map(schema::META);

    let media: BTreeMap<String, String> =
        model.media.iter().map(|(part, m)| (part.clone(), assets.put(&m.content_type, &m.bytes))).collect();
    let mi = meta.insert_container("media", LoroMap::new()).unwrap();
    for (part, hash) in &media {
        mi.insert(part, hash.as_str()).unwrap();
    }
    let colors = meta.insert_container("colors", LoroMap::new()).unwrap();
    for (ix, rgb) in &model.colors {
        colors.insert(&ix.to_string(), rgb.as_str()).unwrap();
    }
    let d = &model.default_styles;
    for (key, v) in [("defaultLineStyle", d.line), ("defaultFillStyle", d.fill), ("defaultTextStyle", d.text)] {
        if let Some(v) = v {
            meta.insert(key, uuid(seed, b'y', v, 0)).unwrap();
        }
    }
    if let Some(xml) = theme_xml {
        let hash = assets.put("application/vnd.openxmlformats-officedocument.theme+xml", xml);
        let mut v: HashMap<String, LoroValue> = HashMap::new();
        v.insert("asset".into(), hash.into());
        doc.get_map(schema::THEMES).insert(&uuid(seed, b't', 0, 0), v).unwrap();
    }

    let styles = doc.get_map(schema::STYLES);
    for st in model.styles.values() {
        let m = styles.insert_container(&uuid(seed, b'y', st.id, 0), LoroMap::new()).unwrap();
        m.insert("legacyId", i64::from(st.id)).unwrap();
        if let Some(n) = &st.name_u {
            m.insert("nameU", n.as_str()).unwrap();
        }
        for (key, v) in [("lineStyle", st.line_style), ("fillStyle", st.fill_style), ("textStyle", st.text_style)] {
            if let Some(v) = v {
                m.insert(key, uuid(seed, b'y', v, 0)).unwrap();
            }
        }
        write_sheet(&m.insert_container(schema::SHEET, LoroMap::new()).unwrap(), &st.sheet);
    }

    let masters = doc.get_map(schema::MASTERS);
    for ms in model.masters.values() {
        let id = uuid(seed, b'm', ms.id, 0);
        let m = masters.insert_container(&id, LoroMap::new()).unwrap();
        m.insert("legacyId", i64::from(ms.id)).unwrap();
        if let Some(n) = &ms.name_u {
            m.insert("nameU", n.as_str()).unwrap();
        }
        write_sheet(&m.insert_container("pageSheet", LoroMap::new()).unwrap(), &ms.page_sheet);
        let mid = ms.id;
        w.write_shapes(
            &tree(doc, &schema::master_tree_key(&id)),
            None,
            &ms.shapes,
            &|s| uuid(seed, b'M', mid, s),
            &media,
        );
    }

    let pages = doc.get_movable_list(schema::PAGES);
    for p in &model.pages {
        let id = uuid(seed, b'p', p.id, 0);
        pages.push(id.as_str()).unwrap();
        let m = doc.get_map(schema::page_key(&id));
        m.insert("legacyId", i64::from(p.id)).unwrap();
        if let Some(n) = &p.name {
            m.insert("name", n.as_str()).unwrap();
        }
        if let Some(n) = &p.name_u {
            m.insert("nameU", n.as_str()).unwrap();
        }
        m.insert("isBackground", p.background).unwrap();
        if let Some(b) = p.back_page {
            m.insert("backPage", uuid(seed, b'p', b, 0)).unwrap();
        }
        write_sheet(&m.insert_container("pageSheet", LoroMap::new()).unwrap(), &p.sheet);
        // Connects are derived data Visio caches; kept as one value for fidelity.
        let connects: Vec<LoroValue> = p
            .connects
            .iter()
            .map(|c| {
                let mut v: HashMap<String, LoroValue> = HashMap::new();
                v.insert("fromSheet".into(), i64::from(c.from_sheet).into());
                v.insert("toSheet".into(), i64::from(c.to_sheet).into());
                if let Some(x) = &c.from_cell {
                    v.insert("fromCell".into(), x.as_str().into());
                }
                if let Some(x) = &c.to_cell {
                    v.insert("toCell".into(), x.as_str().into());
                }
                v.into()
            })
            .collect();
        m.insert("connects", connects).unwrap();
        let pid = p.id;
        w.write_shapes(
            &tree(doc, &schema::shapes_tree_key(&id)),
            None,
            &p.shapes,
            &|s| uuid(seed, b's', pid, s),
            &media,
        );
    }
    assets
}

// ------------------------------------------------------------------ reading

fn child_map(m: &LoroMap, key: &str) -> Option<LoroMap> {
    match m.get(key)? {
        ValueOrContainer::Container(Container::Map(c)) => Some(c),
        _ => None,
    }
}

fn value(m: &LoroMap, key: &str) -> Option<LoroValue> {
    match m.get(key)? {
        ValueOrContainer::Value(v) => Some(v),
        _ => None,
    }
}

fn string(m: &LoroMap, key: &str) -> Option<String> {
    match value(m, key)? {
        LoroValue::String(s) => Some(s.to_string()),
        _ => None,
    }
}

fn int(m: &LoroMap, key: &str) -> Option<i64> {
    match value(m, key)? {
        LoroValue::I64(i) => Some(i),
        _ => None,
    }
}

fn flag(m: &LoroMap, key: &str) -> bool {
    matches!(value(m, key), Some(LoroValue::Bool(true)))
}

fn u32_of(v: Option<i64>) -> Option<u32> {
    v.and_then(|i| u32::try_from(i).ok())
}

fn cell_from(v: &LoroValue) -> Cell {
    let LoroValue::List(l) = v else { return Cell::default() };
    let get = |i: usize| match l.get(i) {
        Some(LoroValue::String(s)) => Some(s.to_string()),
        _ => None,
    };
    Cell { v: get(0), f: get(1), u: get(2) }
}

/// Every `(key, value)` of a map (containers included), sorted by key.
fn entries(m: &LoroMap) -> Vec<(String, ValueOrContainer)> {
    let mut out = Vec::new();
    m.for_each(|k, v| out.push((k.to_owned(), v)));
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn read_row(m: &LoroMap) -> (i64, Row) {
    let mut row = Row {
        t: string(m, "@t"),
        n: string(m, "@n"),
        ix: u32_of(int(m, "@ix")),
        del: flag(m, "@del"),
        cells: BTreeMap::new(),
    };
    for (k, v) in entries(m) {
        if !k.starts_with('@')
            && let ValueOrContainer::Value(v) = v
        {
            row.cells.insert(k, cell_from(&v));
        }
    }
    (int(m, "@pos").unwrap_or(0), row)
}

fn read_sheet(m: Option<LoroMap>) -> Sheet {
    let mut sheet = Sheet::default();
    let Some(m) = m else { return sheet };
    if let Some(cells) = child_map(&m, "cells") {
        for (k, v) in entries(&cells) {
            if let ValueOrContainer::Value(v) = v {
                sheet.cells.insert(k, cell_from(&v));
            }
        }
    }
    if let Some(groups) = child_map(&m, "groups") {
        for (_, g) in entries(&groups) {
            if let ValueOrContainer::Value(LoroValue::Map(cells)) = g {
                for (k, v) in cells.iter() {
                    sheet.cells.insert(k.clone(), cell_from(v));
                }
            }
        }
    }
    let mut sections: Vec<(i64, Section)> = Vec::new();
    if let Some(secs) = child_map(&m, "sections") {
        for (_, s) in entries(&secs) {
            let ValueOrContainer::Container(Container::Map(sm)) = s else { continue };
            let mut section = Section {
                n: string(&sm, "@n").unwrap_or_default(),
                ix: u32_of(int(&sm, "@ix")),
                del: flag(&sm, "@del"),
                cells: BTreeMap::new(),
                rows: Vec::new(),
            };
            if let Some(sc) = child_map(&sm, "cells") {
                for (k, v) in entries(&sc) {
                    if let ValueOrContainer::Value(v) = v {
                        section.cells.insert(k, cell_from(&v));
                    }
                }
            }
            let mut rows: Vec<(i64, Row)> = Vec::new();
            match sm.get("rows") {
                Some(ValueOrContainer::Container(Container::Map(named))) => {
                    for (_, r) in entries(&named) {
                        if let ValueOrContainer::Container(Container::Map(rm)) = r {
                            rows.push(read_row(&rm));
                        }
                    }
                }
                Some(ValueOrContainer::Container(Container::MovableList(list))) => {
                    for i in 0..list.len() {
                        if let Some(ValueOrContainer::Container(Container::Map(rm))) = list.get(i) {
                            // List order is authoritative; `@pos` only orders named rows.
                            let (_, row) = read_row(&rm);
                            rows.push((i as i64, row));
                        }
                    }
                }
                _ => {}
            }
            rows.sort_by_key(|(p, _)| *p);
            section.rows = rows.into_iter().map(|(_, r)| r).collect();
            sections.push((int(&sm, "@pos").unwrap_or(0), section));
        }
    }
    sections.sort_by_key(|(p, _)| *p);
    sheet.sections = sections.into_iter().map(|(_, s)| s).collect();
    sheet
}

fn read_text(t: &LoroText) -> Vec<TextPiece> {
    let mut out = Vec::new();
    let (mut cp, mut pp, mut tp) = (None, None, None);
    for d in t.to_delta() {
        let loro::TextDelta::Insert { insert, attributes } = d else { continue };
        let attr = |k: &str| match attributes.as_ref().and_then(|a| a.get(k)) {
            Some(LoroValue::I64(i)) => u32::try_from(*i).ok(),
            _ => None,
        };
        let mut marked = false;
        for (state, new, make) in [
            (&mut cp, attr("cp"), TextPiece::Cp as fn(u32) -> TextPiece),
            (&mut pp, attr("pp"), TextPiece::Pp),
            (&mut tp, attr("tp"), TextPiece::Tp),
        ] {
            if new != *state {
                if let Some(ix) = new {
                    out.push(make(ix));
                    marked = true;
                }
                *state = new;
            }
        }
        push_text(&mut out, insert, attr("fld"), marked);
    }
    out.retain(|p| !matches!(p, TextPiece::Text(s) if s.is_empty()));
    out
}

/// Appends text after any run markers: a field's text stands alone (an empty
/// text after it stops merging, removed at the end), plain text merges with
/// plain text before it when no marker came between.
fn push_text(out: &mut Vec<TextPiece>, s: String, fld: Option<u32>, marked: bool) {
    if let Some(ix) = fld {
        out.push(TextPiece::Fld(ix));
        out.push(TextPiece::Text(s));
        out.push(TextPiece::Text(String::new()));
    } else if let (false, Some(TextPiece::Text(prev))) = (marked, out.last_mut()) {
        prev.push_str(&s);
    } else {
        out.push(TextPiece::Text(s));
    }
}

/// The form [`read_model`] gives shape text: run markers only where the run
/// changes, in `cp`, `pp`, `tp` order, and adjacent plain text merged. Two
/// texts with equal canonical forms render identically.
pub fn canonical_text(pieces: &[TextPiece]) -> Vec<TextPiece> {
    let mut out = Vec::new();
    let (mut cp, mut pp, mut tp, mut fld) = (None, None, None, None);
    let (mut ecp, mut epp, mut etp) = (None, None, None);
    for piece in pieces {
        match piece {
            TextPiece::Cp(ix) => cp = Some(*ix),
            TextPiece::Pp(ix) => pp = Some(*ix),
            TextPiece::Tp(ix) => tp = Some(*ix),
            TextPiece::Fld(ix) => fld = Some(*ix),
            TextPiece::Text(s) if s.is_empty() => {}
            TextPiece::Text(s) => {
                let mut marked = false;
                for (state, emitted, make) in [
                    (cp, &mut ecp, TextPiece::Cp as fn(u32) -> TextPiece),
                    (pp, &mut epp, TextPiece::Pp),
                    (tp, &mut etp, TextPiece::Tp),
                ] {
                    if state != *emitted {
                        if let Some(ix) = state {
                            out.push(make(ix));
                            marked = true;
                        }
                        *emitted = state;
                    }
                }
                push_text(&mut out, s.clone(), fld.take(), marked);
            }
        }
    }
    out.retain(|p| !matches!(p, TextPiece::Text(s) if s.is_empty()));
    out
}

/// `model` as [`read_model`] gives it back: shape text in canonical form and
/// no read warnings (they describe the file, not the document).
pub fn canonical_model(model: &Model) -> Model {
    fn shapes(list: &mut [Shape]) {
        for s in list {
            if let Some(t) = &s.text {
                s.text = Some(canonical_text(t));
            }
            shapes(&mut s.shapes);
        }
    }
    let mut m = model.clone();
    m.warnings.clear();
    for p in &mut m.pages {
        shapes(&mut p.shapes);
    }
    for ms in m.masters.values_mut() {
        shapes(&mut ms.shapes);
    }
    m
}

struct Reader<'a> {
    doc: &'a LoroDoc,
    masters: BTreeMap<String, u32>,
    styles: BTreeMap<String, u32>,
    media: BTreeMap<String, String>,
}

impl Reader<'_> {
    fn style(&self, m: &LoroMap, key: &str) -> Option<u32> {
        string(m, key).and_then(|u| self.styles.get(&u).copied())
    }

    fn shapes(&self, tree: &LoroTree, parent: Option<TreeID>) -> Vec<Shape> {
        let nodes = tree.children(parent);
        let all = self.doc.get_map(schema::SHAPE);
        let mut out = Vec::new();
        for node in nodes.unwrap_or_default() {
            let Some(uuid) = tree.get_meta(node).ok().and_then(|m| string(&m, "s")) else { continue };
            let Some(m) = child_map(&all, &uuid) else { continue };
            let kind = match string(&m, "type").as_deref() {
                Some("group") => ShapeType::Group,
                Some("guide") => ShapeType::Guide,
                Some("foreign") => ShapeType::Foreign,
                _ => ShapeType::Shape,
            };
            let text = match m.get(schema::TEXT) {
                Some(ValueOrContainer::Container(Container::Text(t))) => Some(read_text(&t)),
                _ => None,
            };
            let foreign = match value(&m, "foreign") {
                Some(LoroValue::Map(f)) => {
                    let get = |k: &str| match f.get(k) {
                        Some(LoroValue::String(s)) => Some(s.to_string()),
                        _ => None,
                    };
                    Some(ForeignData {
                        foreign_type: get("type").unwrap_or_default(),
                        compression: get("compression"),
                        part: get("part"),
                    })
                }
                _ => None,
            };
            out.push(Shape {
                id: u32_of(int(&m, "legacyId")).unwrap_or(0),
                kind,
                name_u: string(&m, "nameU"),
                name: string(&m, "name"),
                master: string(&m, "master").and_then(|u| self.masters.get(&u).copied()),
                master_shape: u32_of(int(&m, "masterShape")),
                line_style: self.style(&m, "lineStyle"),
                fill_style: self.style(&m, "fillStyle"),
                text_style: self.style(&m, "textStyle"),
                del: flag(&m, "del"),
                sheet: read_sheet(child_map(&m, schema::SHEET)),
                text,
                foreign,
                shapes: self.shapes(tree, Some(node)),
            });
        }
        out
    }
}

/// Reads the model back from `doc`. `parse_theme` turns the theme asset into a
/// [`Theme`] (the parser lives in `flowster-vsdx`).
pub fn read_model(doc: &LoroDoc, assets: &Assets, parse_theme: impl Fn(&[u8]) -> Option<Theme>) -> Model {
    let mut model = Model::default();
    let meta = doc.get_map(schema::META);

    let legacy = |map: &LoroMap| -> BTreeMap<String, u32> {
        entries(map)
            .into_iter()
            .filter_map(|(k, v)| match v {
                ValueOrContainer::Container(Container::Map(m)) => u32_of(int(&m, "legacyId")).map(|id| (k, id)),
                _ => None,
            })
            .collect()
    };
    let styles_map = doc.get_map(schema::STYLES);
    let masters_map = doc.get_map(schema::MASTERS);
    let mut r = Reader { doc, styles: legacy(&styles_map), masters: legacy(&masters_map), media: BTreeMap::new() };
    if let Some(mi) = child_map(&meta, "media") {
        for (part, v) in entries(&mi) {
            if let ValueOrContainer::Value(LoroValue::String(h)) = v {
                r.media.insert(part, h.to_string());
            }
        }
    }
    for (part, hash) in &r.media {
        if let Some(m) = assets.blobs.get(hash) {
            model.media.insert(part.clone(), m.clone());
        }
    }
    if let Some(colors) = child_map(&meta, "colors") {
        for (ix, v) in entries(&colors) {
            if let (Ok(ix), ValueOrContainer::Value(LoroValue::String(rgb))) = (ix.parse::<u32>(), v) {
                model.colors.insert(ix, rgb.to_string());
            }
        }
    }
    model.default_styles = DefaultStyles {
        line: r.style(&meta, "defaultLineStyle"),
        fill: r.style(&meta, "defaultFillStyle"),
        text: r.style(&meta, "defaultTextStyle"),
    };
    for (_, v) in entries(&doc.get_map(schema::THEMES)) {
        if let ValueOrContainer::Value(LoroValue::Map(t)) = v
            && let Some(LoroValue::String(h)) = t.get("asset")
            && let Some(blob) = assets.blobs.get(h.as_str())
        {
            model.theme = parse_theme(&blob.bytes);
        }
    }

    for (_, v) in entries(&styles_map) {
        let ValueOrContainer::Container(Container::Map(m)) = v else { continue };
        let Some(id) = u32_of(int(&m, "legacyId")) else { continue };
        model.styles.insert(
            id,
            StyleSheet {
                id,
                name_u: string(&m, "nameU"),
                line_style: r.style(&m, "lineStyle"),
                fill_style: r.style(&m, "fillStyle"),
                text_style: r.style(&m, "textStyle"),
                sheet: read_sheet(child_map(&m, schema::SHEET)),
            },
        );
    }
    for (uuid, v) in entries(&masters_map) {
        let ValueOrContainer::Container(Container::Map(m)) = v else { continue };
        let Some(id) = u32_of(int(&m, "legacyId")) else { continue };
        let shapes = r.shapes(&doc.get_tree(schema::master_tree_key(&uuid)), None);
        model.masters.insert(
            id,
            Master { id, name_u: string(&m, "nameU"), page_sheet: read_sheet(child_map(&m, "pageSheet")), shapes },
        );
    }

    let pages = doc.get_movable_list(schema::PAGES);
    let mut page_ids: BTreeMap<String, u32> = BTreeMap::new();
    let mut back_pages: Vec<Option<String>> = Vec::new();
    for i in 0..pages.len() {
        let Some(ValueOrContainer::Value(LoroValue::String(uuid))) = pages.get(i) else { continue };
        let uuid = uuid.to_string();
        let m = doc.get_map(schema::page_key(&uuid));
        let id = u32_of(int(&m, "legacyId")).unwrap_or(0);
        page_ids.insert(uuid.clone(), id);
        back_pages.push(string(&m, "backPage"));
        let connects = match value(&m, "connects") {
            Some(LoroValue::List(list)) => list
                .iter()
                .filter_map(|c| {
                    let LoroValue::Map(c) = c else { return None };
                    let num = |k: &str| match c.get(k) {
                        Some(LoroValue::I64(i)) => u32::try_from(*i).ok(),
                        _ => None,
                    };
                    let text = |k: &str| match c.get(k) {
                        Some(LoroValue::String(s)) => Some(s.to_string()),
                        _ => None,
                    };
                    Some(Connect {
                        from_sheet: num("fromSheet")?,
                        from_cell: text("fromCell"),
                        to_sheet: num("toSheet")?,
                        to_cell: text("toCell"),
                    })
                })
                .collect(),
            _ => Vec::new(),
        };
        model.pages.push(Page {
            id,
            name_u: string(&m, "nameU"),
            name: string(&m, "name"),
            background: flag(&m, "isBackground"),
            back_page: None,
            sheet: read_sheet(child_map(&m, "pageSheet")),
            shapes: r.shapes(&doc.get_tree(schema::shapes_tree_key(&uuid)), None),
            connects,
        });
    }
    for (page, back) in model.pages.iter_mut().zip(back_pages) {
        page.back_page = back.and_then(|b| page_ids.get(&b).copied());
    }
    model
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_are_stable_and_well_formed() {
        let a = uuid(7, b's', 1, 2);
        assert_eq!(a, uuid(7, b's', 1, 2));
        assert_ne!(a, uuid(7, b's', 1, 3));
        assert_ne!(a, uuid(8, b's', 1, 2));
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "8", "version nibble: {a}");
        assert!(matches!(&a[19..20], "8" | "9" | "a" | "b"), "variant: {a}");
    }

    #[test]
    fn concurrent_move_and_resize_both_survive() {
        let bytes = flowster_vsdx::fixture::minimal_vsdx();
        let pkg = flowster_vsdx::OpcPackage::open(bytes.clone()).unwrap();
        let model = flowster_vsdx::visio::read_document(&pkg).unwrap();
        let a = LoroDoc::new();
        a.set_peer_id(1).unwrap();
        let assets = write_model(&a, &model, None, 1);
        a.commit();
        let b = LoroDoc::new();
        b.set_peer_id(2).unwrap();
        b.import(&a.export(loro::ExportMode::Snapshot).unwrap()).unwrap();

        // Rewrite one atomic group per replica, as a move and a resize command would.
        let set = |doc: &LoroDoc, group: &str, cells: &[(&str, &str)]| {
            let shape = uuid(1, b's', model.pages[0].id, model.pages[0].shapes[0].id);
            let sheet = child_map(&child_map(&doc.get_map(schema::SHAPE), &shape).unwrap(), schema::SHEET).unwrap();
            let v: HashMap<String, LoroValue> =
                cells.iter().map(|(k, v)| ((*k).to_owned(), cell_value(&Cell::value(*v)))).collect();
            lazy_map(&sheet, "groups").insert(group, v).unwrap();
            doc.commit();
        };
        set(&a, "Position", &[("PinX", "1"), ("PinY", "2")]);
        set(&b, "Size", &[("Width", "3"), ("Height", "4"), ("LocPinX", "1.5"), ("LocPinY", "2")]);
        let va = a.oplog_vv();
        a.import(&b.export(loro::ExportMode::updates(&va)).unwrap()).unwrap();
        b.import(&a.export(loro::ExportMode::updates(&b.oplog_vv())).unwrap()).unwrap();

        let ma = read_model(&a, &assets, |_| None);
        assert_eq!(ma, read_model(&b, &assets, |_| None), "replicas converge");
        let cells = &ma.pages[0].shapes[0].sheet.cells;
        let v = |n: &str| cells[n].v.as_deref();
        assert_eq!((v("PinX"), v("PinY"), v("Width"), v("Height")), (Some("1"), Some("2"), Some("3"), Some("4")));
    }

    #[test]
    fn text_marks_round_trip_in_canonical_form() {
        let pieces = vec![
            TextPiece::Cp(0),
            TextPiece::Pp(0),
            TextPiece::Text("Bold ".into()),
            TextPiece::Cp(1),
            TextPiece::Text("plain ".into()),
            TextPiece::Fld(0),
            TextPiece::Text("12/10/2026".into()),
            TextPiece::Text(" end".into()),
            TextPiece::Cp(1),
        ];
        let doc = LoroDoc::new();
        configure(&doc);
        let t = doc.get_text("t");
        write_text(&t, &pieces);
        assert_eq!(read_text(&t), canonical_text(&pieces));
        assert_eq!(t.to_string(), "Bold plain 12/10/2026 end");
    }
}
