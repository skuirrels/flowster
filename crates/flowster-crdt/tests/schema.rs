//! Schema behaviour both libraries must provide (SPEC §20.5), on a small document.

use flowster_crdt::SchemaStore;
use flowster_crdt::synth::generate;

fn pair<S: SchemaStore>() -> (S, S, flowster_crdt::synth::SynthDoc) {
    let synth = generate(120, 2, 7);
    let a = S::new(1);
    a.ingest(&synth);
    let b = S::load(2, &a.snapshot());
    assert_eq!(a.traverse(), b.traverse());
    (a, b, synth)
}

fn sync<S: SchemaStore>(a: &S, b: &S) {
    let (va, vb) = (a.version(), b.version());
    let to_b = a.updates_since(&vb);
    let to_a = b.updates_since(&va);
    b.apply(&to_b);
    a.apply(&to_a);
}

/// A move by one user and a resize by another both survive (separate atomic groups).
fn move_vs_resize<S: SchemaStore>() {
    let (a, b, synth) = pair::<S>();
    let id = &synth.shapes[3].uuid;
    a.move_shapes(&[(id, 9.0, 8.0)]);
    b.resize_shape(id, 3.0, 2.0);
    sync(&a, &b);
    for s in [&a, &b] {
        assert_eq!(s.position(id), Some((9.0, 8.0)));
        assert_eq!(s.size(id), Some((3.0, 2.0)));
    }
}

/// Concurrent moves of the same shape: one Position wins whole, never a PinX/PinY mix.
fn concurrent_moves_are_atomic<S: SchemaStore>() {
    let (a, b, synth) = pair::<S>();
    let id = &synth.shapes[3].uuid;
    a.move_shapes(&[(id, 1.0, 1.0)]);
    b.move_shapes(&[(id, 2.0, 2.0)]);
    sync(&a, &b);
    let pa = a.position(id).unwrap();
    assert_eq!(pa, b.position(id).unwrap());
    assert!(pa == (1.0, 1.0) || pa == (2.0, 2.0), "mixed group: {pa:?}");
}

/// Two users typing into the same text merge character-wise.
fn concurrent_typing_merges<S: SchemaStore>() {
    let (a, b, synth) = pair::<S>();
    let id = &synth.shapes[1].uuid;
    let original = a.text(id).unwrap();
    a.insert_text(id, 0, "AA ");
    b.insert_text(id, original.len(), " BB");
    sync(&a, &b);
    let t = a.text(id).unwrap();
    assert_eq!(t, b.text(id).unwrap());
    assert_eq!(t, format!("AA {original} BB"));
}

/// Concurrent z-order changes converge to the same order on both peers.
fn z_order_converges<S: SchemaStore>() {
    let (a, b, synth) = pair::<S>();
    let page = &synth.pages[0].id;
    let top = |s: &S| s.z_order(page).last().cloned().unwrap();
    let ua = a.z_order(page)[1].clone();
    let ub = b.z_order(page)[2].clone();
    a.bring_to_front(&ua);
    b.bring_to_front(&ub);
    sync(&a, &b);
    assert_eq!(a.z_order(page), b.z_order(page));
    assert!(top(&a) == ua || top(&a) == ub);
}

/// Edit vs delete: delete wins.
fn delete_wins<S: SchemaStore>() {
    let (a, b, synth) = pair::<S>();
    let id = &synth.shapes[2].uuid;
    a.delete_shape(id);
    b.move_shapes(&[(id, 4.0, 4.0)]);
    sync(&a, &b);
    assert_eq!(a.position(id), None);
    assert_eq!(b.position(id), None);
    assert_eq!(a.traverse(), b.traverse());
}

macro_rules! suite {
    ($m:ident, $store:ty) => {
        mod $m {
            use super::*;
            #[test]
            fn move_vs_resize_both_apply() {
                move_vs_resize::<$store>()
            }
            #[test]
            fn concurrent_moves_are_atomic_groups() {
                concurrent_moves_are_atomic::<$store>()
            }
            #[test]
            fn concurrent_typing_merges_characterwise() {
                concurrent_typing_merges::<$store>()
            }
            #[test]
            fn z_order_converges_on_both_peers() {
                z_order_converges::<$store>()
            }
            #[test]
            fn delete_wins_over_edit() {
                delete_wins::<$store>()
            }
        }
    };
}

#[cfg(feature = "loro")]
suite!(loro, flowster_crdt::loro_store::LoroStore);
#[cfg(feature = "yrs")]
suite!(yrs, flowster_crdt::yrs_store::YrsStore);

#[test]
fn small_spike_runs_for_both() {
    struct P;
    impl flowster_crdt::spike::Probe for P {
        fn now_ms(&self) -> f64 {
            0.0
        }
        fn live_bytes(&self) -> usize {
            0
        }
        fn reset_peak(&self) {}
        fn peak_bytes(&self) -> usize {
            0
        }
    }
    let cfg = flowster_crdt::spike::SpikeConfig {
        shapes: 300,
        pages: 2,
        load_runs: 1,
        apply_samples: 10,
        ..Default::default()
    };
    #[cfg(feature = "loro")]
    assert!(flowster_crdt::spike::run::<flowster_crdt::loro_store::LoroStore>(&cfg, &P, "test").converged);
    #[cfg(feature = "yrs")]
    assert!(flowster_crdt::spike::run::<flowster_crdt::yrs_store::YrsStore>(&cfg, &P, "test").converged);
}
