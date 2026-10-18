//! Counting global allocator for the CRDT spike's memory figures.
//! Two relaxed atomic adds per allocation.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct Counting;

fn grew(n: usize) {
    let live = LIVE.fetch_add(n, Relaxed) + n;
    PEAK.fetch_max(live, Relaxed);
}

// SAFETY: every method forwards to `System` unchanged and only updates counters.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_sub(layout.size(), Relaxed);
            grew(new_size);
        }
        p
    }
}

// Installed for the whole CLI: the overhead is negligible for a batch tool.
#[global_allocator]
static GLOBAL: Counting = Counting;

pub fn live() -> usize {
    LIVE.load(Relaxed)
}
pub fn peak() -> usize {
    PEAK.load(Relaxed)
}
pub fn reset_peak() {
    PEAK.store(LIVE.load(Relaxed), Relaxed);
}
