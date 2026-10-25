//! The system allocator, asking for transparent huge pages on big blocks.
//!
//! A 45 MP frame is a 136 MB buffer, and the export path allocates several
//! (decode, turn, grade, resize). With 4 KB pages every fresh one costs about
//! 33,000 page faults: measured 54 ms to touch 136 MB, against 9 ms with
//! MADV_HUGEPAGE (this kernel serves huge pages on request). Allocations
//! under 4 MB are untouched.
use std::alloc::{GlobalAlloc, Layout, System};

pub struct HugeAlloc;

const BIG: usize = 4 << 20;
const HUGE: usize = 2 << 20;

/// Ask for huge pages on the 2 MB-aligned part of a block.
unsafe fn advise(p: *mut u8, size: usize) {
    let start = (p as usize + HUGE - 1) & !(HUGE - 1);
    let end = (p as usize + size) & !(HUGE - 1);
    if end > start {
        libc::madvise(start as *mut libc::c_void, end - start, libc::MADV_HUGEPAGE);
    }
}

unsafe impl GlobalAlloc for HugeAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() && l.size() >= BIG {
            advise(p, l.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(l);
        if !p.is_null() && l.size() >= BIG {
            advise(p, l.size());
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = System.realloc(p, l, new);
        if !q.is_null() && new >= BIG {
            advise(q, new);
        }
        q
    }
}
