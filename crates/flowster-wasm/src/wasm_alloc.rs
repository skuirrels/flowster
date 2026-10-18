//! Global allocator for the WASM engine: dlmalloc over a system layer that
//! grows linear memory in large chunks, plus live/peak byte counters.
//!
//! Why: the stock wasm32 allocator grows memory 64 KiB at a time, and each
//! `memory.grow` is expensive in V8 on Windows (measured at 0.7 ms per call on a
//! small heap and 4.6 ms at 1 GiB, Node 24). WebView2 runs the same V8. A
//! 10k-shape import needs thousands of grows, which made it 14x slower than
//! native (docs/adr/0001-crdt-library.md). Growing by at least 16 MiB, or 1/8 of the
//! current heap, makes the number of grows logarithmic in heap size.
//!
//! Single-threaded: the engine worker has no wasm threads. Revisit if it ever does.
#![allow(unsafe_code)]

use core::arch::wasm32;
use core::cell::UnsafeCell;
use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use dlmalloc::{Allocator, Dlmalloc};

const PAGE: usize = 64 * 1024;
const MIN_CHUNK_PAGES: usize = 256; // 16 MiB

struct ChunkedGrow;

// SAFETY: hands out fresh, page-aligned regions from `memory.grow`, never reused or freed.
unsafe impl Allocator for ChunkedGrow {
    fn alloc(&self, size: usize) -> (*mut u8, usize, u32) {
        let needed = size.div_ceil(PAGE);
        let chunk = needed.max(MIN_CHUNK_PAGES).max(wasm32::memory_size(0) / 8);
        let mut pages = chunk;
        let mut prev = wasm32::memory_grow(0, pages);
        if prev == usize::MAX && chunk > needed {
            pages = needed;
            prev = wasm32::memory_grow(0, pages);
        }
        if prev == usize::MAX {
            return (core::ptr::null_mut(), 0, 0);
        }
        ((prev * PAGE) as *mut u8, pages * PAGE, 0)
    }
    fn remap(&self, _ptr: *mut u8, _old: usize, _new: usize, _can_move: bool) -> *mut u8 {
        core::ptr::null_mut()
    }
    fn free_part(&self, _ptr: *mut u8, _old: usize, _new: usize) -> bool {
        false
    }
    fn free(&self, _ptr: *mut u8, _size: usize) -> bool {
        false
    }
    fn can_release_part(&self, _flags: u32) -> bool {
        false
    }
    fn allocates_zeros(&self) -> bool {
        true
    }
    fn page_size(&self) -> usize {
        PAGE
    }
}

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(n: usize) {
    let live = LIVE.fetch_add(n, Relaxed) + n;
    PEAK.fetch_max(live, Relaxed);
}

struct Engine(UnsafeCell<Dlmalloc<ChunkedGrow>>);

// SAFETY: wasm32 without the `atomics` feature has exactly one thread.
unsafe impl Sync for Engine {}

// SAFETY: forwards to dlmalloc, which upholds the GlobalAlloc contract; counters only observe.
unsafe impl GlobalAlloc for Engine {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { (*self.0.get()).malloc(layout.size(), layout.align()) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { (*self.0.get()).calloc(layout.size(), layout.align()) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { (*self.0.get()).free(ptr, layout.size(), layout.align()) };
        LIVE.fetch_sub(layout.size(), Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { (*self.0.get()).realloc(ptr, layout.size(), layout.align(), new_size) };
        if !p.is_null() {
            LIVE.fetch_sub(layout.size(), Relaxed);
            grew(new_size);
        }
        p
    }
}

#[global_allocator]
static GLOBAL: Engine = Engine(UnsafeCell::new(Dlmalloc::new_with_allocator(ChunkedGrow)));

/// Bytes currently allocated by Rust code.
pub fn live() -> usize {
    LIVE.load(Relaxed)
}
pub fn peak() -> usize {
    PEAK.load(Relaxed)
}
pub fn reset_peak() {
    PEAK.store(LIVE.load(Relaxed), Relaxed);
}
