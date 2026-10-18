//! M0 CRDT spike: memory, snapshot load time and update sizes on a synthetic document.
//!
//! The same code runs natively (`flowster bench crdt`) and in WASM
//! (`scripts/crdt-spike.mjs`); only the [`Probe`] (clock + heap counter) differs.

use serde::Serialize;

use crate::store::SchemaStore;
use crate::synth::{self, SynthDoc};

/// Platform services the spike needs.
pub trait Probe {
    /// Monotonic milliseconds (sub-ms resolution where available).
    fn now_ms(&self) -> f64;
    /// Bytes currently allocated on the Rust heap.
    fn live_bytes(&self) -> usize;
    /// Resets the high-water mark to the current live size.
    fn reset_peak(&self);
    fn peak_bytes(&self) -> usize;
}

#[derive(Clone, Debug)]
pub struct SpikeConfig {
    pub shapes: usize,
    pub pages: usize,
    pub seed: u64,
    /// Repetitions for the load measurement (median is reported).
    pub load_runs: usize,
    /// Updates generated per kind for the remote-apply timing.
    pub apply_samples: usize,
}

impl Default for SpikeConfig {
    fn default() -> Self {
        SpikeConfig { shapes: 10_000, pages: 4, seed: 0xF10_5735, load_runs: 3, apply_samples: 100 }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct UpdateSizes {
    /// One shape moved (Position group rewritten).
    pub move_one: usize,
    /// Drag commit of a 10-shape selection, one transaction.
    pub move_ten: usize,
    pub resize: usize,
    /// One character typed into existing shape text.
    pub text_insert_char: usize,
    /// A word typed as one transaction (8 chars).
    pub text_insert_word: usize,
    pub text_bold: usize,
    pub z_order: usize,
    pub create_shape: usize,
    pub delete_shape: usize,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SpikeReport {
    pub library: String,
    pub target: String,
    pub shapes: usize,
    pub pages: usize,
    /// Heap used by the library-neutral synthetic model (the "plain model" baseline).
    pub plain_model_bytes: usize,
    pub build_ms: f64,
    /// Heap held by the CRDT doc built from scratch (includes history).
    pub built_heap_bytes: usize,
    pub snapshot_bytes: usize,
    pub snapshot_export_ms: f64,
    /// Median time to decode the snapshot into a fresh replica.
    pub load_import_ms: f64,
    /// Median time to decode and read every shape (what "open" needs before first paint).
    pub load_open_ms: f64,
    /// Heap held by a replica loaded from the snapshot and fully read.
    pub loaded_heap_bytes: usize,
    pub loaded_peak_bytes: usize,
    /// `loaded_heap_bytes / plain_model_bytes`; SPEC §20.10 budget is ≤ 1.5.
    pub memory_ratio_vs_plain: f64,
    pub update_bytes: UpdateSizes,
    /// Mean time for a loaded remote replica to apply one move update.
    pub apply_move_us: f64,
    /// Mean time for a loaded remote replica to apply one keystroke update.
    pub apply_text_us: f64,
    /// Remote replica matches the author after all updates.
    pub converged: bool,
    /// Cells read by the deep traversal (same for both libraries if the schema matches).
    pub cells: u64,
    pub extras: Vec<(String, f64)>,
    pub notes: Vec<String>,
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Runs the spike for one library. `target` is a label ("native", "wasm32").
pub fn run<S: SchemaStore>(cfg: &SpikeConfig, probe: &dyn Probe, target: &str) -> SpikeReport {
    let mut r = SpikeReport {
        library: S::LIBRARY.into(),
        target: target.into(),
        shapes: cfg.shapes,
        pages: cfg.pages,
        ..Default::default()
    };

    let base = probe.live_bytes();
    let synth: SynthDoc = synth::generate(cfg.shapes, cfg.pages, cfg.seed);
    r.plain_model_bytes = probe.live_bytes().saturating_sub(base);

    // Build peer A from scratch.
    let before = probe.live_bytes();
    let t0 = probe.now_ms();
    let a = S::new(1);
    a.ingest(&synth);
    r.build_ms = probe.now_ms() - t0;
    r.built_heap_bytes = probe.live_bytes().saturating_sub(before);
    let digest_a = a.traverse();
    r.cells = digest_a.cells;
    r.extras = a.extras();

    let t0 = probe.now_ms();
    let snapshot = a.snapshot();
    r.snapshot_export_ms = probe.now_ms() - t0;
    r.snapshot_bytes = snapshot.len();

    // Load timings (fresh replica each run).
    let mut import_ms = Vec::new();
    let mut open_ms = Vec::new();
    for i in 0..cfg.load_runs.max(1) {
        let t0 = probe.now_ms();
        let b = S::load(100 + i as u64, &snapshot);
        let t1 = probe.now_ms();
        let d = b.traverse();
        let t2 = probe.now_ms();
        assert_eq!(d, digest_a, "snapshot round trip must preserve content");
        import_ms.push(t1 - t0);
        open_ms.push(t2 - t0);
    }
    r.load_import_ms = median(import_ms);
    r.load_open_ms = median(open_ms);

    // Memory of a loaded + fully read replica (this one stays as the remote peer).
    let before = probe.live_bytes();
    probe.reset_peak();
    let b = S::load(2, &snapshot);
    b.traverse();
    r.loaded_heap_bytes = probe.live_bytes().saturating_sub(before);
    r.loaded_peak_bytes = probe.peak_bytes().saturating_sub(before);
    r.memory_ratio_vs_plain = r.loaded_heap_bytes as f64 / r.plain_model_bytes.max(1) as f64;

    // Update sizes, authored on A.
    let ids: Vec<&str> = synth.shapes.iter().filter(|s| !s.text.is_empty()).map(|s| s.uuid.as_str()).collect();
    let mut pending: Vec<Vec<u8>> = Vec::new();
    let mut measure = |f: &dyn Fn()| -> usize {
        let v = a.version();
        f();
        let u = a.updates_since(&v);
        let n = u.len();
        pending.push(u);
        n
    };
    let s0 = ids[0];
    r.update_bytes.move_one = measure(&|| a.move_shapes(&[(s0, 5.25, 7.5)]));
    let ten: Vec<(&str, f64, f64)> = ids[1..11].iter().enumerate().map(|(k, u)| (*u, 3.0 + k as f64, 4.0)).collect();
    r.update_bytes.move_ten = measure(&|| a.move_shapes(&ten));
    r.update_bytes.resize = measure(&|| a.resize_shape(ids[12], 2.0, 1.25));
    r.update_bytes.text_insert_char = measure(&|| a.insert_text(ids[13], 5, "x"));
    r.update_bytes.text_insert_word = measure(&|| a.insert_text(ids[14], 0, "Approve "));
    r.update_bytes.text_bold = measure(&|| a.bold_text(ids[15], 0, 4));
    r.update_bytes.z_order = measure(&|| a.bring_to_front(ids[16]));
    let mut extra = synth::generate(1, 1, cfg.seed ^ 0xABCD).shapes.remove(0);
    extra.uuid = "01900000-ffff-7fff-8fff-ffffffffffff".into();
    let page0 = synth.pages[0].id.clone();
    r.update_bytes.create_shape = measure(&|| a.create_shape(&page0, &extra));
    r.update_bytes.delete_shape = measure(&|| a.delete_shape(ids[17]));
    drop(measure);
    for u in &pending {
        b.apply(u);
    }

    // Remote apply timings: N single-shape moves, then N keystrokes.
    let n = cfg.apply_samples.max(1);
    let mut moves = Vec::with_capacity(n);
    for k in 0..n {
        let v = a.version();
        a.move_shapes(&[(ids[20 + k % (ids.len() - 20)], 1.0 + k as f64 * 0.01, 2.0)]);
        moves.push(a.updates_since(&v));
    }
    let t0 = probe.now_ms();
    for u in &moves {
        b.apply(u);
    }
    r.apply_move_us = (probe.now_ms() - t0) * 1000.0 / n as f64;

    let mut keys = Vec::with_capacity(n);
    for k in 0..n {
        let v = a.version();
        a.insert_text(ids[30 + k % 50], 0, "k");
        keys.push(a.updates_since(&v));
    }
    let t0 = probe.now_ms();
    for u in &keys {
        b.apply(u);
    }
    r.apply_text_us = (probe.now_ms() - t0) * 1000.0 / n as f64;

    r.converged = a.traverse() == b.traverse() && a.z_order(&page0) == b.z_order(&page0);
    r
}
