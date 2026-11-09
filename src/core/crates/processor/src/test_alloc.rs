// SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Thread-local allocation measurements exclude other tests and fixture setup.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Counts {
    pub calls: usize,
    pub bytes: usize,
}

thread_local! { static COUNTS: Cell<Option<Counts>> = const { Cell::new(None) }; }

fn record(bytes: usize) {
    let _ = COUNTS.try_with(|slot| {
        if let Some(mut counts) = slot.get() {
            counts.calls += 1;
            counts.bytes += bytes;
            slot.set(Some(counts));
        }
    });
}

struct CountingAllocator;
// SAFETY: allocation and deallocation are delegated unchanged to System.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(size);
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

pub(super) fn measure<T>(f: impl FnOnce() -> T) -> (T, Counts) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            COUNTS.with(|slot| slot.set(None));
        }
    }
    COUNTS.with(|slot| {
        assert!(slot.get().is_none(), "allocation measurements cannot nest");
        slot.set(Some(Counts::default()));
    });
    let reset = Reset;
    let result = f();
    let counts = COUNTS.with(|slot| slot.get().unwrap());
    drop(reset);
    (result, counts)
}
