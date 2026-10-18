//! Per-thread allocation counting, to *prove* hot paths never touch the heap.
//!
//! Install [`CountingAllocator`] as the global allocator of a binary or test crate:
//!
//! ```ignore
//! #[global_allocator]
//! static ALLOCATOR: densa_alloc::CountingAllocator = densa_alloc::CountingAllocator::system();
//! ```
//!
//! then wrap the code under test in [`measure`]. Counters are per thread, so other threads
//! (test harness, workers) never pollute a measurement. Without the allocator installed all
//! counters stay at zero; always check [`is_active`] first so a zero-allocation assertion
//! cannot pass vacuously.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

/// Heap operations observed on one thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AllocStats {
    /// `alloc` + `alloc_zeroed` calls.
    pub allocs: u64,
    /// `dealloc` calls.
    pub deallocs: u64,
    /// `realloc` calls.
    pub reallocs: u64,
    /// Bytes requested by `alloc`, `alloc_zeroed` and `realloc` (new size).
    pub bytes_allocated: u64,
}

impl AllocStats {
    /// No operations: the starting point for counters and deltas.
    pub const ZERO: Self = Self {
        allocs: 0,
        deallocs: 0,
        reallocs: 0,
        bytes_allocated: 0,
    };

    /// `true` if no heap operation of any kind happened.
    pub fn is_zero(&self) -> bool {
        self.allocs == 0 && self.deallocs == 0 && self.reallocs == 0
    }

    /// Operations that happened between `earlier` and `self`.
    pub fn since(self, earlier: Self) -> Self {
        Self {
            allocs: self.allocs.wrapping_sub(earlier.allocs),
            deallocs: self.deallocs.wrapping_sub(earlier.deallocs),
            reallocs: self.reallocs.wrapping_sub(earlier.reallocs),
            bytes_allocated: self.bytes_allocated.wrapping_sub(earlier.bytes_allocated),
        }
    }
}

thread_local! {
    // `const` init + a type without `Drop`: lives in static TLS, so touching it from inside
    // the allocator never allocates and never recurses into the allocator.
    static STATS: Cell<AllocStats> = const { Cell::new(AllocStats::ZERO) };
}

#[inline]
fn record(update: impl FnOnce(&mut AllocStats)) {
    // `try_with` instead of `with`: never panic inside the allocator, even during
    // thread teardown.
    let _ = STATS.try_with(|cell| {
        let mut stats = cell.get();
        update(&mut stats);
        cell.set(stats);
    });
}

/// Snapshot of the calling thread's cumulative counters.
pub fn thread_stats() -> AllocStats {
    STATS.try_with(Cell::get).unwrap_or(AllocStats::ZERO)
}

/// Runs `f` and returns its result with the heap operations it performed on this thread.
pub fn measure<R>(f: impl FnOnce() -> R) -> (R, AllocStats) {
    let before = thread_stats();
    let result = f();
    (result, thread_stats().since(before))
}

/// `true` if [`CountingAllocator`] is the active global allocator.
///
/// Performs one tiny allocation and checks that it was counted.
pub fn is_active() -> bool {
    let ((), stats) = measure(|| drop(black_box(Box::new(0u8))));
    stats.allocs > 0
}

/// A [`GlobalAlloc`] wrapper that counts every heap operation per thread.
#[derive(Debug, Default)]
pub struct CountingAllocator<A = System> {
    inner: A,
}

impl CountingAllocator<System> {
    /// Counting wrapper around the system allocator.
    pub const fn system() -> Self {
        Self { inner: System }
    }
}

impl<A> CountingAllocator<A> {
    /// Counting wrapper around any other allocator.
    pub const fn new(inner: A) -> Self {
        Self { inner }
    }
}

#[inline]
fn bytes(size: usize) -> u64 {
    // Lossless on every supported target (usize <= 64 bits).
    size as u64
}

// SAFETY: every method forwards its arguments unchanged to `inner`, so `inner`'s
// `GlobalAlloc` guarantees carry over. The bookkeeping only updates a const-initialised,
// `Drop`-free thread-local `Cell`, which neither allocates nor re-enters the allocator.
unsafe impl<A: GlobalAlloc> GlobalAlloc for CountingAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(|s| {
            s.allocs += 1;
            s.bytes_allocated += bytes(layout.size());
        });
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract for `layout`.
        unsafe { self.inner.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(|s| {
            s.allocs += 1;
            s.bytes_allocated += bytes(layout.size());
        });
        // SAFETY: the caller upholds `GlobalAlloc::alloc_zeroed`'s contract for `layout`.
        unsafe { self.inner.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record(|s| s.deallocs += 1);
        // SAFETY: the caller guarantees `ptr` was allocated by this allocator (hence by
        // `inner`, since we forward every allocation) with `layout`.
        unsafe { self.inner.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(|s| {
            s.reallocs += 1;
            s.bytes_allocated += bytes(new_size);
        });
        // SAFETY: the caller upholds `GlobalAlloc::realloc`'s contract; `ptr` came from
        // `inner` because every allocation is forwarded to it.
        unsafe { self.inner.realloc(ptr, layout, new_size) }
    }
}
