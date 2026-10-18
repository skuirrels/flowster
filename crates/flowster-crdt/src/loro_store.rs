//! SPEC §20.3 schema on Loro.
//!
//! * `shapes/<page>` is a `LoroTree` with fractional-index ordering; each node's
//!   meta map holds the shape uuid under `s`.
//! * `shape` is a root map of uuid → shape map (props, `sheet`, `text`).
//! * Atomic cell groups are plain `LoroValue::Map` values (one LWW register each).

use std::cell::RefCell;
use std::collections::HashMap;

use loro::{
    Container, ExpandType, ExportMode, LoroDoc, LoroMap, LoroMovableList, LoroText, LoroTree, LoroValue, StyleConfig,
    StyleConfigMap, TreeID, ValueOrContainer, VersionVector,
};

use crate::schema;
use crate::store::{Digest, SchemaStore, add_bits};
use crate::synth::{Pv, Section, SynthDoc, SynthShape};

pub struct LoroStore {
    pub doc: LoroDoc,
    /// Derived index: uuid → (page id, tree node). Rebuilt by `traverse`.
    index: RefCell<HashMap<String, (String, TreeID)>>,
}

pub fn pv_to_loro(pv: &Pv) -> LoroValue {
    match pv {
        Pv::Null => LoroValue::Null,
        Pv::Bool(b) => (*b).into(),
        Pv::Int(i) => (*i).into(),
        Pv::Num(n) => (*n).into(),
        Pv::Str(s) => s.as_str().into(),
        Pv::Map(entries) => {
            let m: HashMap<String, LoroValue> = entries.iter().map(|(k, v)| (k.clone(), pv_to_loro(v))).collect();
            m.into()
        }
    }
}

fn configure(doc: &LoroDoc) {
    let mut styles = StyleConfigMap::new();
    styles.insert("bold".into(), StyleConfig { expand: ExpandType::After });
    doc.config_text_style(styles);
}

fn child_map(m: &LoroMap, key: &str) -> Option<LoroMap> {
    match m.get(key)? {
        ValueOrContainer::Container(Container::Map(c)) => Some(c),
        _ => None,
    }
}

fn child_text(m: &LoroMap, key: &str) -> Option<LoroText> {
    match m.get(key)? {
        ValueOrContainer::Container(Container::Text(c)) => Some(c),
        _ => None,
    }
}

fn value(m: &LoroMap, key: &str) -> Option<LoroValue> {
    match m.get(key)? {
        ValueOrContainer::Value(v) => Some(v),
        _ => None,
    }
}

/// Reads `group.<cell>.v` as a number from an atomic group value.
fn group_num(group: &LoroValue, cell: &str) -> Option<f64> {
    let LoroValue::Map(m) = group else { return None };
    let LoroValue::Map(c) = m.get(cell)? else { return None };
    match c.get("v")? {
        LoroValue::Double(d) => Some(*d),
        LoroValue::I64(i) => Some(*i as f64),
        _ => None,
    }
}

/// Returns a copy of an atomic group with `cell.v` replaced.
fn group_with(group: &LoroValue, updates: &[(&str, f64)]) -> LoroValue {
    let mut out: HashMap<String, LoroValue> = match group {
        LoroValue::Map(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => HashMap::new(),
    };
    for (cell, v) in updates {
        let mut c: HashMap<String, LoroValue> = match out.get(*cell) {
            Some(LoroValue::Map(m)) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            _ => HashMap::new(),
        };
        c.insert("v".into(), (*v).into());
        out.insert((*cell).into(), c.into());
    }
    out.into()
}

/// Counts `CellEntry`-shaped maps (those holding a `v`) in a deep value.
fn count_cells(v: &LoroValue) -> u64 {
    match v {
        LoroValue::Map(m) => u64::from(m.contains_key("v")) + m.values().map(count_cells).sum::<u64>(),
        LoroValue::List(l) => l.iter().map(count_cells).sum(),
        _ => 0,
    }
}

impl LoroStore {
    fn shapes(&self) -> LoroMap {
        self.doc.get_map(schema::SHAPE)
    }

    fn shape(&self, uuid: &str) -> Option<LoroMap> {
        child_map(&self.shapes(), uuid)
    }

    fn xform(&self, uuid: &str) -> Option<LoroMap> {
        child_map(&child_map(&self.shape(uuid)?, schema::SHEET)?, "XForm")
    }

    fn tree(&self, page_id: &str) -> LoroTree {
        let t = self.doc.get_tree(schema::shapes_tree_key(page_id));
        if !t.is_fractional_index_enabled() {
            t.enable_fractional_index(0);
        }
        t
    }

    fn write_shape(&self, page_id: &str, parent: Option<TreeID>, shape: &SynthShape) -> TreeID {
        let tree = self.tree(page_id);
        let node = tree.create(parent).expect("tree create");
        tree.get_meta(node).expect("meta").insert("s", shape.uuid.as_str()).unwrap();

        let sm = self.shapes().insert_container(&shape.uuid, LoroMap::new()).unwrap();
        for (k, v) in &shape.props {
            sm.insert(k, pv_to_loro(v)).unwrap();
        }
        let sheet = sm.insert_container(schema::SHEET, LoroMap::new()).unwrap();
        for (key, section) in &shape.sheet {
            match section {
                Section::Single(cells) => {
                    let m = sheet.insert_container(key, LoroMap::new()).unwrap();
                    for (c, v) in cells {
                        m.insert(c, pv_to_loro(v)).unwrap();
                    }
                }
                Section::Named(rows) => {
                    let m = sheet.insert_container(key, LoroMap::new()).unwrap();
                    for (name, cells) in rows {
                        let r = m.insert_container(name, LoroMap::new()).unwrap();
                        for (c, v) in cells {
                            r.insert(c, pv_to_loro(v)).unwrap();
                        }
                    }
                }
                Section::Indexed(rows) => {
                    let l = sheet.insert_container(key, LoroMovableList::new()).unwrap();
                    for (i, cells) in rows.iter().enumerate() {
                        let r = l.insert_container(i, LoroMap::new()).unwrap();
                        for (c, v) in cells {
                            r.insert(c, pv_to_loro(v)).unwrap();
                        }
                    }
                }
            }
        }
        if !shape.text.is_empty() {
            let t = sm.insert_container(schema::TEXT, LoroText::new()).unwrap();
            t.insert(0, &shape.text).unwrap();
            if let Some(r) = &shape.bold {
                t.mark(r.clone(), "bold", true).unwrap();
            }
        }
        self.index.borrow_mut().insert(shape.uuid.clone(), (page_id.to_owned(), node));
        node
    }
}

impl SchemaStore for LoroStore {
    const LIBRARY: &'static str = "loro";

    fn new(peer: u64) -> Self {
        let doc = LoroDoc::new();
        doc.set_peer_id(peer).unwrap();
        configure(&doc);
        LoroStore { doc, index: RefCell::new(HashMap::new()) }
    }

    fn ingest(&self, synth: &SynthDoc) {
        let meta = self.doc.get_map(schema::META);
        meta.insert("title", "Synthetic 10k").unwrap();
        meta.insert("creator", "flowster-crdt spike").unwrap();

        let pages = self.doc.get_movable_list(schema::PAGES);
        for p in &synth.pages {
            pages.push(p.id.as_str()).unwrap();
            let pm = self.doc.get_map(schema::page_key(&p.id));
            pm.insert("name", p.name.as_str()).unwrap();
            pm.insert("isBackground", false).unwrap();
            let ps = pm.insert_container("pageSheet", LoroMap::new()).unwrap();
            let props = ps.insert_container("PageProperties", LoroMap::new()).unwrap();
            props.insert("PageWidth", pv_to_loro(&crate::synth::cell_u(Pv::Num(34.0), "IN"))).unwrap();
            props.insert("PageHeight", pv_to_loro(&crate::synth::cell_u(Pv::Num(22.0), "IN"))).unwrap();
        }
        let masters = self.doc.get_map(schema::MASTERS);
        for (id, name) in &synth.masters {
            let m = masters.insert_container(id, LoroMap::new()).unwrap();
            m.insert("nameU", name.as_str()).unwrap();
        }
        let styles = self.doc.get_map(schema::STYLES);
        for (id, name) in &synth.styles {
            let m = styles.insert_container(id, LoroMap::new()).unwrap();
            m.insert("name", name.as_str()).unwrap();
        }

        let mut nodes: Vec<TreeID> = Vec::with_capacity(synth.shapes.len());
        for shape in &synth.shapes {
            let parent = shape.parent.map(|g| nodes[g]);
            nodes.push(self.write_shape(&synth.pages[shape.page].id, parent, shape));
        }
        self.doc.commit();
    }

    fn snapshot(&self) -> Vec<u8> {
        self.doc.export(ExportMode::Snapshot).unwrap()
    }

    fn load(peer: u64, bytes: &[u8]) -> Self {
        let s = Self::new(peer);
        s.doc.import(bytes).unwrap();
        s
    }

    fn traverse(&self) -> Digest {
        let mut d = Digest::default();
        let mut index = self.index.borrow_mut();
        index.clear();
        let pages = self.doc.get_movable_list(schema::PAGES);
        for i in 0..pages.len() {
            let Some(ValueOrContainer::Value(LoroValue::String(pid))) = pages.get(i) else { continue };
            let tree = self.doc.get_tree(schema::shapes_tree_key(&pid));
            for node in tree.nodes() {
                d.tree_nodes += 1;
                let meta = tree.get_meta(node).unwrap();
                if let Some(LoroValue::String(uuid)) = value(&meta, "s") {
                    index.insert(uuid.to_string(), (pid.to_string(), node));
                }
            }
        }
        self.shapes().for_each(|_, v| {
            let ValueOrContainer::Container(Container::Map(sm)) = v else { return };
            d.shapes += 1;
            d.cells += count_cells(&sm.get_deep_value());
            if let Some(xf) = child_map(&sm, schema::SHEET).and_then(|s| child_map(&s, "XForm")) {
                if let Some(p) = value(&xf, "Position") {
                    add_bits(&mut d.geometry_bits, group_num(&p, "PinX"));
                    add_bits(&mut d.geometry_bits, group_num(&p, "PinY"));
                }
                if let Some(sz) = value(&xf, "Size") {
                    add_bits(&mut d.geometry_bits, group_num(&sz, "Width"));
                    add_bits(&mut d.geometry_bits, group_num(&sz, "Height"));
                }
            }
            if let Some(t) = child_text(&sm, schema::TEXT) {
                d.text_chars += t.to_string().chars().count() as u64;
            }
        });
        d
    }

    fn version(&self) -> Vec<u8> {
        self.doc.oplog_vv().encode()
    }

    fn updates_since(&self, version: &[u8]) -> Vec<u8> {
        let vv = VersionVector::decode(version).unwrap();
        self.doc.export(ExportMode::updates(&vv)).unwrap()
    }

    fn apply(&self, update: &[u8]) {
        self.doc.import(update).unwrap();
    }

    fn move_shapes(&self, moves: &[(&str, f64, f64)]) {
        for (uuid, x, y) in moves {
            let xf = self.xform(uuid).expect("shape");
            let cur = value(&xf, "Position").unwrap_or(LoroValue::Null);
            xf.insert("Position", group_with(&cur, &[("PinX", *x), ("PinY", *y)])).unwrap();
        }
        self.doc.commit();
    }

    fn resize_shape(&self, uuid: &str, w: f64, h: f64) {
        let xf = self.xform(uuid).expect("shape");
        let cur = value(&xf, "Size").unwrap_or(LoroValue::Null);
        let next = group_with(&cur, &[("Width", w), ("Height", h), ("LocPinX", w * 0.5), ("LocPinY", h * 0.5)]);
        xf.insert("Size", next).unwrap();
        self.doc.commit();
    }

    fn insert_text(&self, uuid: &str, pos: usize, text: &str) {
        let sm = self.shape(uuid).expect("shape");
        let t = match child_text(&sm, schema::TEXT) {
            Some(t) => t,
            None => sm.insert_container(schema::TEXT, LoroText::new()).unwrap(),
        };
        t.insert(pos, text).unwrap();
        self.doc.commit();
    }

    fn bold_text(&self, uuid: &str, start: usize, end: usize) {
        let t = child_text(&self.shape(uuid).expect("shape"), schema::TEXT).expect("text");
        t.mark(start..end, "bold", true).unwrap();
        self.doc.commit();
    }

    fn bring_to_front(&self, uuid: &str) {
        let (page, node) = self.index.borrow().get(uuid).cloned().expect("indexed");
        let tree = self.tree(&page);
        let parent = tree.parent(node).expect("parent");
        tree.mov(node, parent).unwrap();
        self.doc.commit();
    }

    fn create_shape(&self, page_id: &str, shape: &SynthShape) {
        self.write_shape(page_id, None, shape);
        self.doc.commit();
    }

    fn delete_shape(&self, uuid: &str) {
        if let Some((page, node)) = self.index.borrow_mut().remove(uuid) {
            self.tree(&page).delete(node).unwrap();
        }
        self.shapes().delete(uuid).unwrap();
        self.doc.commit();
    }

    fn position(&self, uuid: &str) -> Option<(f64, f64)> {
        let p = value(&self.xform(uuid)?, "Position")?;
        Some((group_num(&p, "PinX")?, group_num(&p, "PinY")?))
    }

    fn size(&self, uuid: &str) -> Option<(f64, f64)> {
        let p = value(&self.xform(uuid)?, "Size")?;
        Some((group_num(&p, "Width")?, group_num(&p, "Height")?))
    }

    fn text(&self, uuid: &str) -> Option<String> {
        Some(child_text(&self.shape(uuid)?, schema::TEXT)?.to_string())
    }

    fn extras(&self) -> Vec<(String, f64)> {
        // Shallow snapshot: current state without history (§20.8 compaction).
        let frontiers = self.doc.oplog_frontiers();
        let shallow = self.doc.export(ExportMode::shallow_snapshot(&frontiers)).unwrap();
        vec![("shallow_snapshot_bytes".into(), shallow.len() as f64)]
    }

    fn z_order(&self, page_id: &str) -> Vec<String> {
        let tree = self.doc.get_tree(schema::shapes_tree_key(page_id));
        tree.children(None)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|n| match value(&tree.get_meta(n).ok()?, "s")? {
                LoroValue::String(s) => Some(s.to_string()),
                _ => None,
            })
            .collect()
    }
}
