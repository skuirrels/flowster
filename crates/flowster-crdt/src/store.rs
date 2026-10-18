//! The operations the spike measures, implemented once per CRDT library.

use crate::synth::{SynthDoc, SynthShape};

/// Order-independent digest of what the engine reads on open:
/// every shape's Position/Size group and its text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Digest {
    pub shapes: u64,
    pub tree_nodes: u64,
    pub text_chars: u64,
    /// `CellEntry` values found by a deep read of every shape (forces lazy state to load).
    pub cells: u64,
    /// Wrapping sum of the f64 bits of PinX, PinY, Width, Height.
    pub geometry_bits: u64,
}

pub trait SchemaStore: Sized {
    const LIBRARY: &'static str;

    fn new(peer: u64) -> Self;
    /// Imports a whole synthetic document in one transaction, as VSDX import would.
    fn ingest(&self, doc: &SynthDoc);
    fn snapshot(&self) -> Vec<u8>;
    /// Loads a snapshot produced by [`SchemaStore::snapshot`] into a fresh replica.
    fn load(peer: u64, bytes: &[u8]) -> Self;
    /// Reads every shape the way the engine does on open, rebuilding the uuid index.
    fn traverse(&self) -> Digest;

    fn version(&self) -> Vec<u8>;
    fn updates_since(&self, version: &[u8]) -> Vec<u8>;
    fn apply(&self, update: &[u8]);

    /// One transaction that rewrites the Position atomic group of each shape.
    fn move_shapes(&self, moves: &[(&str, f64, f64)]);
    /// One transaction that rewrites the Size atomic group.
    fn resize_shape(&self, uuid: &str, w: f64, h: f64);
    fn insert_text(&self, uuid: &str, pos: usize, text: &str);
    fn bold_text(&self, uuid: &str, start: usize, end: usize);
    /// Z-order change: make the shape the top-most child of its parent.
    fn bring_to_front(&self, uuid: &str);
    fn create_shape(&self, page_id: &str, shape: &SynthShape);
    fn delete_shape(&self, uuid: &str);

    fn position(&self, uuid: &str) -> Option<(f64, f64)>;
    fn size(&self, uuid: &str) -> Option<(f64, f64)>;
    fn text(&self, uuid: &str) -> Option<String>;
    /// Child uuids of the page root, in z-order (back to front).
    fn z_order(&self, page_id: &str) -> Vec<String>;

    /// Library-specific extra measurements (name, value).
    fn extras(&self) -> Vec<(String, f64)> {
        Vec::new()
    }
}

pub(crate) fn add_bits(acc: &mut u64, v: Option<f64>) {
    if let Some(v) = v {
        *acc = acc.wrapping_add(v.to_bits());
    }
}
