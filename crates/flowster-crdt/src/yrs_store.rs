//! SPEC §20.3 schema on yrs (Y-CRDT), the evaluated fallback.
//!
//! yrs has no tree and no movable list, so:
//! * hierarchy and z-order are emulated with `parent` + `z` (fractional key)
//!   registers on each shape map. Cycle prevention and sibling ordering become
//!   our own read-time code (the Loro tree gives both for free);
//! * indexed sections use `ArrayRef`, and a row reorder has to be delete + insert.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use yrs::types::ToJson;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Any, Array, ArrayPrelim, ArrayRef, Doc, GetString, Map, MapPrelim, MapRef, Number, Out, ReadTxn, StateVector, Text,
    TextPrelim, TextRef, Transact, TransactionMut, Update,
};

use crate::schema;
use crate::store::{Digest, SchemaStore, add_bits};
use crate::synth::{Pv, Section, SynthDoc, SynthShape};

pub struct YrsStore {
    pub doc: Doc,
    /// Root refs are fetched once: `get_or_insert_*` takes the doc's write lock
    /// and would deadlock inside an open transaction.
    shapes: MapRef,
    /// Derived: page id → highest z key in use. Rebuilt by `traverse`.
    z_max: RefCell<HashMap<String, String>>,
}

pub fn pv_to_any(pv: &Pv) -> Any {
    match pv {
        Pv::Null => Any::Null,
        Pv::Bool(b) => Any::Bool(*b),
        Pv::Int(i) => Any::Number(Number::Int(*i)),
        Pv::Num(n) => Any::Number(Number::Float(*n)),
        Pv::Str(s) => Any::String(Arc::from(s.as_str())),
        Pv::Map(entries) => Any::Map(Arc::new(entries.iter().map(|(k, v)| (k.clone(), pv_to_any(v))).collect())),
    }
}

fn child_map<T: ReadTxn>(m: &MapRef, txn: &T, key: &str) -> Option<MapRef> {
    match m.get(txn, key)? {
        Out::YMap(c) => Some(c),
        _ => None,
    }
}

fn child_text<T: ReadTxn>(m: &MapRef, txn: &T, key: &str) -> Option<TextRef> {
    match m.get(txn, key)? {
        Out::YText(c) => Some(c),
        _ => None,
    }
}

fn any<T: ReadTxn>(m: &MapRef, txn: &T, key: &str) -> Option<Any> {
    match m.get(txn, key)? {
        Out::Any(a) => Some(a),
        _ => None,
    }
}

fn group_num(group: &Any, cell: &str) -> Option<f64> {
    let Any::Map(m) = group else { return None };
    let Any::Map(c) = m.get(cell)? else { return None };
    match c.get("v")? {
        Any::Number(Number::Float(n)) => Some(*n),
        Any::Number(Number::Int(i)) => Some(*i as f64),
        _ => None,
    }
}

fn group_with(group: &Any, updates: &[(&str, f64)]) -> Any {
    let mut out: HashMap<String, Any> = match group {
        Any::Map(m) => (**m).clone(),
        _ => HashMap::new(),
    };
    for (cell, v) in updates {
        let mut c: HashMap<String, Any> = match out.get(*cell) {
            Some(Any::Map(m)) => (**m).clone(),
            _ => HashMap::new(),
        };
        c.insert("v".into(), Any::Number(Number::Float(*v)));
        out.insert((*cell).into(), Any::Map(Arc::new(c)));
    }
    Any::Map(Arc::new(out))
}

/// Counts `CellEntry`-shaped maps (those holding a `v`) in a deep value.
fn count_cells(v: &Any) -> u64 {
    match v {
        Any::Map(m) => u64::from(m.contains_key("v")) + m.values().map(count_cells).sum::<u64>(),
        Any::Array(a) => a.iter().map(count_cells).sum(),
        _ => 0,
    }
}

/// Fractional key strictly greater than `k` (append-only strategy; fine for a spike).
fn key_after(k: &str) -> String {
    format!("{k}V")
}

impl YrsStore {
    fn shapes(&self) -> MapRef {
        self.shapes.clone()
    }

    fn xform<T: ReadTxn>(&self, txn: &T, uuid: &str) -> Option<MapRef> {
        let sm = child_map(&self.shapes(), txn, uuid)?;
        child_map(&child_map(&sm, txn, schema::SHEET)?, txn, "XForm")
    }

    fn write_shape(&self, txn: &mut TransactionMut, page_id: &str, parent: &str, z: &str, shape: &SynthShape) {
        let sm: MapRef = self.shapes().insert(txn, shape.uuid.as_str(), MapPrelim::default());
        sm.insert(txn, "page", page_id);
        sm.insert(txn, "parent", parent);
        sm.insert(txn, "z", z);
        for (k, v) in &shape.props {
            sm.insert(txn, *k, pv_to_any(v));
        }
        let sheet: MapRef = sm.insert(txn, schema::SHEET, MapPrelim::default());
        for (key, section) in &shape.sheet {
            match section {
                Section::Single(cells) => {
                    let m: MapRef = sheet.insert(txn, key.as_str(), MapPrelim::default());
                    for (c, v) in cells {
                        m.insert(txn, c.as_str(), pv_to_any(v));
                    }
                }
                Section::Named(rows) => {
                    let m: MapRef = sheet.insert(txn, key.as_str(), MapPrelim::default());
                    for (name, cells) in rows {
                        let r: MapRef = m.insert(txn, name.as_str(), MapPrelim::default());
                        for (c, v) in cells {
                            r.insert(txn, c.as_str(), pv_to_any(v));
                        }
                    }
                }
                Section::Indexed(rows) => {
                    let l: ArrayRef = sheet.insert(txn, key.as_str(), ArrayPrelim::default());
                    for cells in rows {
                        let r: MapRef = l.push_back(txn, MapPrelim::default());
                        for (c, v) in cells {
                            r.insert(txn, c.as_str(), pv_to_any(v));
                        }
                    }
                }
            }
        }
        if !shape.text.is_empty() {
            let t: TextRef = sm.insert(txn, schema::TEXT, TextPrelim::new(shape.text.as_str()));
            if let Some(r) = &shape.bold {
                let attrs = HashMap::from([(Arc::<str>::from("bold"), Any::Bool(true))]);
                t.format(txn, r.start as u32, (r.end - r.start) as u32, attrs);
            }
        }
    }
}

impl SchemaStore for YrsStore {
    const LIBRARY: &'static str = "yrs";

    fn new(peer: u64) -> Self {
        let doc = Doc::with_client_id(peer);
        let shapes = doc.get_or_insert_map(schema::SHAPE);
        YrsStore { doc, shapes, z_max: RefCell::new(HashMap::new()) }
    }

    fn ingest(&self, synth: &SynthDoc) {
        let meta = self.doc.get_or_insert_map(schema::META);
        let pages = self.doc.get_or_insert_array(schema::PAGES);
        let masters = self.doc.get_or_insert_map(schema::MASTERS);
        let styles = self.doc.get_or_insert_map(schema::STYLES);
        let page_maps: Vec<MapRef> =
            synth.pages.iter().map(|p| self.doc.get_or_insert_map(schema::page_key(&p.id))).collect();

        let mut txn = self.doc.transact_mut();
        meta.insert(&mut txn, "title", "Synthetic 10k");
        meta.insert(&mut txn, "creator", "flowster-crdt spike");
        for (p, pm) in synth.pages.iter().zip(&page_maps) {
            pages.push_back(&mut txn, p.id.as_str());
            pm.insert(&mut txn, "name", p.name.as_str());
            pm.insert(&mut txn, "isBackground", false);
            let ps: MapRef = pm.insert(&mut txn, "pageSheet", MapPrelim::default());
            let props: MapRef = ps.insert(&mut txn, "PageProperties", MapPrelim::default());
            props.insert(&mut txn, "PageWidth", pv_to_any(&crate::synth::cell_u(Pv::Num(34.0), "IN")));
            props.insert(&mut txn, "PageHeight", pv_to_any(&crate::synth::cell_u(Pv::Num(22.0), "IN")));
        }
        for (id, name) in &synth.masters {
            let m: MapRef = masters.insert(&mut txn, id.as_str(), MapPrelim::default());
            m.insert(&mut txn, "nameU", name.as_str());
        }
        for (id, name) in &synth.styles {
            let m: MapRef = styles.insert(&mut txn, id.as_str(), MapPrelim::default());
            m.insert(&mut txn, "name", name.as_str());
        }
        let mut z_max = self.z_max.borrow_mut();
        for (i, shape) in synth.shapes.iter().enumerate() {
            let page_id = &synth.pages[shape.page].id;
            let parent = shape.parent.map(|g| synth.shapes[g].uuid.as_str()).unwrap_or("");
            let z = format!("a{i:06}");
            self.write_shape(&mut txn, page_id, parent, &z, shape);
            z_max.insert(page_id.clone(), z);
        }
    }

    fn snapshot(&self) -> Vec<u8> {
        self.doc.transact().encode_state_as_update_v1(&StateVector::default())
    }

    fn load(peer: u64, bytes: &[u8]) -> Self {
        let s = Self::new(peer);
        s.apply(bytes);
        s
    }

    fn traverse(&self) -> Digest {
        let mut d = Digest::default();
        let shapes = self.shapes();
        let txn = self.doc.transact();
        let mut z_max = self.z_max.borrow_mut();
        z_max.clear();
        for (_, v) in shapes.iter(&txn) {
            let Out::YMap(sm) = v else { continue };
            d.shapes += 1;
            d.cells += count_cells(&sm.to_json(&txn));
            // The tree is emulated: every shape is a node.
            d.tree_nodes += 1;
            if let (Some(Any::String(page)), Some(Any::String(z))) = (any(&sm, &txn, "page"), any(&sm, &txn, "z")) {
                let e = z_max.entry(page.to_string()).or_default();
                if z.as_ref() > e.as_str() {
                    *e = z.to_string();
                }
            }
            if let Some(xf) = child_map(&sm, &txn, schema::SHEET).and_then(|s| child_map(&s, &txn, "XForm")) {
                if let Some(p) = any(&xf, &txn, "Position") {
                    add_bits(&mut d.geometry_bits, group_num(&p, "PinX"));
                    add_bits(&mut d.geometry_bits, group_num(&p, "PinY"));
                }
                if let Some(sz) = any(&xf, &txn, "Size") {
                    add_bits(&mut d.geometry_bits, group_num(&sz, "Width"));
                    add_bits(&mut d.geometry_bits, group_num(&sz, "Height"));
                }
            }
            if let Some(t) = child_text(&sm, &txn, schema::TEXT) {
                d.text_chars += t.get_string(&txn).chars().count() as u64;
            }
        }
        d
    }

    fn version(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }

    fn updates_since(&self, version: &[u8]) -> Vec<u8> {
        let sv = StateVector::decode_v1(version).unwrap();
        self.doc.transact().encode_state_as_update_v1(&sv)
    }

    fn apply(&self, update: &[u8]) {
        let u = Update::decode_v1(update).unwrap();
        self.doc.transact_mut().apply_update(u).unwrap();
    }

    fn move_shapes(&self, moves: &[(&str, f64, f64)]) {
        let mut txn = self.doc.transact_mut();
        for (uuid, x, y) in moves {
            let xf = self.xform(&txn, uuid).expect("shape");
            let cur = any(&xf, &txn, "Position").unwrap_or(Any::Null);
            xf.insert(&mut txn, "Position", group_with(&cur, &[("PinX", *x), ("PinY", *y)]));
        }
    }

    fn resize_shape(&self, uuid: &str, w: f64, h: f64) {
        let mut txn = self.doc.transact_mut();
        let xf = self.xform(&txn, uuid).expect("shape");
        let cur = any(&xf, &txn, "Size").unwrap_or(Any::Null);
        let next = group_with(&cur, &[("Width", w), ("Height", h), ("LocPinX", w * 0.5), ("LocPinY", h * 0.5)]);
        xf.insert(&mut txn, "Size", next);
    }

    fn insert_text(&self, uuid: &str, pos: usize, text: &str) {
        let mut txn = self.doc.transact_mut();
        let sm = child_map(&self.shapes(), &txn, uuid).expect("shape");
        let t = match child_text(&sm, &txn, schema::TEXT) {
            Some(t) => t,
            None => sm.insert(&mut txn, schema::TEXT, TextPrelim::new("")),
        };
        t.insert(&mut txn, pos as u32, text);
    }

    fn bold_text(&self, uuid: &str, start: usize, end: usize) {
        let mut txn = self.doc.transact_mut();
        let sm = child_map(&self.shapes(), &txn, uuid).expect("shape");
        let t = child_text(&sm, &txn, schema::TEXT).expect("text");
        let attrs = HashMap::from([(Arc::<str>::from("bold"), Any::Bool(true))]);
        t.format(&mut txn, start as u32, (end - start) as u32, attrs);
    }

    fn bring_to_front(&self, uuid: &str) {
        let mut txn = self.doc.transact_mut();
        let sm = child_map(&self.shapes(), &txn, uuid).expect("shape");
        let Some(Any::String(page)) = any(&sm, &txn, "page") else { return };
        let mut z_max = self.z_max.borrow_mut();
        let next = key_after(z_max.get(page.as_ref()).map(String::as_str).unwrap_or("a"));
        sm.insert(&mut txn, "z", next.as_str());
        z_max.insert(page.to_string(), next);
    }

    fn create_shape(&self, page_id: &str, shape: &SynthShape) {
        let mut z_max = self.z_max.borrow_mut();
        let z = key_after(z_max.get(page_id).map(String::as_str).unwrap_or("a"));
        let mut txn = self.doc.transact_mut();
        self.write_shape(&mut txn, page_id, "", &z, shape);
        z_max.insert(page_id.to_owned(), z);
    }

    fn delete_shape(&self, uuid: &str) {
        let mut txn = self.doc.transact_mut();
        self.shapes().remove(&mut txn, uuid);
    }

    fn position(&self, uuid: &str) -> Option<(f64, f64)> {
        let txn = self.doc.transact();
        let p = any(&self.xform(&txn, uuid)?, &txn, "Position")?;
        Some((group_num(&p, "PinX")?, group_num(&p, "PinY")?))
    }

    fn size(&self, uuid: &str) -> Option<(f64, f64)> {
        let txn = self.doc.transact();
        let p = any(&self.xform(&txn, uuid)?, &txn, "Size")?;
        Some((group_num(&p, "Width")?, group_num(&p, "Height")?))
    }

    fn text(&self, uuid: &str) -> Option<String> {
        let txn = self.doc.transact();
        let sm = child_map(&self.shapes(), &txn, uuid)?;
        Some(child_text(&sm, &txn, schema::TEXT)?.get_string(&txn))
    }

    fn extras(&self) -> Vec<(String, f64)> {
        let v2 = self.doc.transact().encode_state_as_update_v2(&StateVector::default());
        vec![("snapshot_v2_bytes".into(), v2.len() as f64)]
    }

    fn z_order(&self, page_id: &str) -> Vec<String> {
        let txn = self.doc.transact();
        let mut kids: Vec<(String, String)> = self
            .shapes()
            .iter(&txn)
            .filter_map(|(uuid, v)| {
                let Out::YMap(sm) = v else { return None };
                let page = any(&sm, &txn, "page")?;
                let parent = any(&sm, &txn, "parent")?;
                let z = any(&sm, &txn, "z")?;
                match (page, parent, z) {
                    (Any::String(p), Any::String(par), Any::String(z)) if p.as_ref() == page_id && par.is_empty() => {
                        Some((z.to_string(), uuid.to_string()))
                    }
                    _ => None,
                }
            })
            .collect();
        // Ties (concurrent identical keys) break by uuid so every peer agrees.
        kids.sort();
        kids.into_iter().map(|(_, u)| u).collect()
    }
}
