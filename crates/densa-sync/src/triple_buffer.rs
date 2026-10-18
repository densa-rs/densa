//! Wait-free single-producer / single-consumer triple buffer.
//!
//! Three slots are owned, at any instant, by exactly one role each:
//!
//! - **back**   — owned by the [`Writer`], being filled;
//! - **middle** — owned by nobody, holds the latest published snapshot;
//! - **front**  — owned by the [`Reader`], being read.
//!
//! Ownership moves only through one `AtomicU8` (`middle`), which packs the middle slot
//! index (bits 0..=1) and a `DIRTY` flag (bit 2) meaning "middle is newer than front".
//! Both sides exchange their slot with `middle` via a single `swap`, so neither side
//! ever waits for the other: `publish` and `update` are wait-free (one atomic RMW each).
//!
//! # Memory ordering
//!
//! Both swaps use `AcqRel`; neither half alone is sufficient:
//!
//! - Writer `Release` + reader `Acquire` order the writer's stores into a slot before
//!   the reader's loads from it (no read-after-write tearing).
//! - Reader `Release` + writer `Acquire` order the reader's loads from a slot it hands
//!   back before the writer's next stores into it (no write-after-read hazard). A
//!   "Release on publish, Acquire on fetch" scheme omits this edge and is unsound.
//!
//! # Stale back buffer
//!
//! After `publish`, the writer receives whichever slot was middle — its content is an
//! *older* snapshot, not the one just published. Writers must fully overwrite the parts
//! of `T` they care about on every cycle (e.g. copy a complete viewport).

use crate::sync::{Arc, AtomicU8, Ordering, UnsafeCell};
use crossbeam_utils::CachePadded;

const INDEX_MASK: u8 = 0b011;
const DIRTY_BIT: u8 = 0b100;

struct Shared<T> {
    /// Each slot on its own cache line so writer and reader never false-share.
    slots: [CachePadded<UnsafeCell<T>>; 3],
    /// Middle slot index | optional `DIRTY_BIT`.
    middle: CachePadded<AtomicU8>,
}

// SAFETY: `Shared` is reachable from two threads, but each slot is accessed by at most
// one thread at a time: a slot index is held by exactly one of {writer.back, middle,
// reader.front}, and ownership transfers happen through `AcqRel` swaps on `middle`, which
// establish happens-before between the previous owner's accesses and the next owner's.
// Values of `T` thus move between threads but are never shared, so `T: Send` suffices.
unsafe impl<T: Send> Sync for Shared<T> {}

// SAFETY: `&Reader` exposes `read`, which hands out `&T`; sharing a `&Reader` across
// threads therefore shares `&T`, which is sound only for `T: Sync`. This explicit impl
// replaces the auto impl, which would otherwise require only `T: Send` (via `Arc<Shared>`).
// `Writer` needs no such bound: every method that touches a slot takes `&mut self`.
unsafe impl<T: Send + Sync> Sync for Reader<T> {}

/// Creates a connected writer/reader pair, with all three slots set to `initial`.
///
/// This is the only point that allocates; `write`, `publish`, `update`, `read` never do.
pub fn triple_buffer<T: Clone + Send>(initial: T) -> (Writer<T>, Reader<T>) {
    let shared = Arc::new(Shared {
        slots: [
            CachePadded::new(UnsafeCell::new(initial.clone())),
            CachePadded::new(UnsafeCell::new(initial.clone())),
            CachePadded::new(UnsafeCell::new(initial)),
        ],
        middle: CachePadded::new(AtomicU8::new(1)),
    });
    let writer = Writer {
        shared: Arc::clone(&shared),
        back: 0,
    };
    let reader = Reader { shared, front: 2 };
    (writer, reader)
}

/// Producer half. Not `Clone`: the single-producer invariant is enforced by ownership.
pub struct Writer<T> {
    shared: Arc<Shared<T>>,
    back: u8,
}

/// Consumer half. Not `Clone`: the single-consumer invariant is enforced by ownership.
///
/// `Reader<T>` is `Sync` only when `T: Sync`, because `read(&self)` hands out `&T`:
///
/// ```compile_fail
/// fn assert_sync<S: Sync>() {}
/// assert_sync::<densa_sync::Reader<std::cell::Cell<u32>>>();
/// ```
pub struct Reader<T> {
    shared: Arc<Shared<T>>,
    front: u8,
}

impl<T: Send> Writer<T> {
    /// Gives mutable access to the back slot. Changes are invisible until [`publish`].
    ///
    /// The slot holds an older snapshot; see the module docs on stale back buffers.
    ///
    /// [`publish`]: Writer::publish
    #[inline]
    pub fn write<R>(&mut self, f: impl FnOnce(&mut T) -> R) -> R {
        self.shared.slots[usize::from(self.back)].with_mut(|ptr| {
            // SAFETY: `self.back` is owned exclusively by this writer (it is neither the
            // middle index nor the reader's front), and the `Acquire` half of the swap
            // that handed it to us ordered all prior reader accesses before this point.
            // `&mut self` prevents re-entrant aliasing, and the reference cannot escape `f`.
            f(unsafe { &mut *ptr })
        })
    }

    /// Publishes the back slot as the newest snapshot. Wait-free; never blocks.
    #[inline]
    pub fn publish(&mut self) {
        // ORDERING: AcqRel. Release: our writes to `back` happen-before the reader's
        // Acquire swap that takes this slot. Acquire: the reader's reads of the slot we
        // receive (released by its swap) happen-before our next `write` into it.
        let previous = self
            .shared
            .middle
            .swap(self.back | DIRTY_BIT, Ordering::AcqRel);
        self.back = previous & INDEX_MASK;
    }
}

impl<T: Send> Reader<T> {
    /// Returns `true` if a snapshot newer than the current front has been published.
    #[inline]
    pub fn has_update(&self) -> bool {
        // ORDERING: Relaxed. A pure hint; no slot data is accessed based on it.
        self.shared.middle.load(Ordering::Relaxed) & DIRTY_BIT != 0
    }

    /// Switches to the newest published snapshot, if any. Wait-free; never blocks.
    ///
    /// Returns `true` if the front slot changed.
    #[inline]
    pub fn update(&mut self) -> bool {
        if !self.has_update() {
            return false;
        }
        // ORDERING: AcqRel. Acquire: the writer's writes to the slot we take happen-before
        // our reads. Release: our reads of the old front happen-before the writer's
        // Acquire swap that later hands that slot back to the writer.
        let previous = self.shared.middle.swap(self.front, Ordering::AcqRel);
        // Only the reader clears DIRTY, and it observed DIRTY set above; by coherence of
        // `middle`'s modification order, the swapped-out value must still carry it.
        debug_assert!(previous & DIRTY_BIT != 0, "DIRTY cleared by someone else");
        self.front = previous & INDEX_MASK;
        true
    }

    /// Gives shared access to the current front snapshot.
    #[inline]
    pub fn read<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.shared.slots[usize::from(self.front)].with(|ptr| {
            // SAFETY: `self.front` is owned exclusively by this reader, so the writer
            // never writes it concurrently; the `Acquire` half of the swap that handed it
            // to us made the writer's prior stores visible. `front` can only change via
            // `update(&mut self)`, which cannot run while this `&self` borrow is live.
            f(unsafe { &*ptr })
        })
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    #[test]
    fn halves_are_send_for_send_payloads() {
        assert_send::<Writer<[u64; 4]>>();
        assert_send::<Reader<[u64; 4]>>();
        // Send-but-!Sync payloads can still move to another thread.
        assert_send::<Reader<std::cell::Cell<u32>>>();
    }

    #[test]
    fn reader_is_sync_for_sync_payloads() {
        // The negative case (`T: !Sync` implies `Reader<T>: !Sync`) is a compile_fail doctest.
        assert_sync::<Reader<[u64; 4]>>();
    }

    #[test]
    fn reader_sees_initial_value_without_update() {
        let (_writer, reader) = triple_buffer(7u32);
        assert_eq!(reader.read(|v| *v), 7);
        assert!(!reader.has_update());
    }

    #[test]
    fn update_returns_false_when_nothing_published() {
        let (_writer, mut reader) = triple_buffer(1u32);
        assert!(!reader.update());
        assert_eq!(reader.read(|v| *v), 1);
    }

    #[test]
    fn unpublished_writes_are_invisible() {
        let (mut writer, mut reader) = triple_buffer(1u32);
        writer.write(|v| *v = 2);
        assert!(!reader.update());
        assert_eq!(reader.read(|v| *v), 1);
    }

    #[test]
    fn published_value_becomes_visible_after_update() {
        let (mut writer, mut reader) = triple_buffer(0u32);
        writer.write(|v| *v = 42);
        writer.publish();

        assert!(reader.has_update());
        assert!(reader.update());
        assert_eq!(reader.read(|v| *v), 42);
        assert!(!reader.has_update());
        assert!(!reader.update(), "same snapshot must not be reported twice");
        assert_eq!(reader.read(|v| *v), 42);
    }

    #[test]
    fn reader_skips_to_latest_of_several_publishes() {
        let (mut writer, mut reader) = triple_buffer(0u32);
        for i in 1..=5 {
            writer.write(|v| *v = i);
            writer.publish();
        }
        assert!(reader.update());
        assert_eq!(reader.read(|v| *v), 5);
    }

    #[test]
    fn write_returns_closure_result() {
        let (mut writer, _reader) = triple_buffer(3u32);
        assert_eq!(writer.write(|v| *v * 2), 6);
    }

    #[test]
    fn indices_always_form_a_permutation() {
        let (mut writer, mut reader) = triple_buffer(0u8);
        let check = |w: &Writer<u8>, r: &Reader<u8>| {
            let middle = w.shared.middle.load(Ordering::Relaxed) & INDEX_MASK;
            let mut seen = [false; 3];
            for idx in [w.back, middle, r.front] {
                assert!(idx < 3);
                assert!(!seen[idx as usize], "index {idx} owned twice");
                seen[idx as usize] = true;
            }
        };
        check(&writer, &reader);
        // Interleave publishes and updates in every order pattern.
        for step in 0..32u32 {
            if step % 3 != 2 {
                writer.publish();
                check(&writer, &reader);
            }
            if step.is_multiple_of(2) {
                reader.update();
                check(&writer, &reader);
            }
        }
    }

    #[test]
    fn writer_never_receives_the_readers_front_buffer() {
        // Tag each buffer with the index it lives in, then verify the writer's
        // back buffer is never the reader's front buffer across many cycles.
        let (mut writer, mut reader) = triple_buffer(0usize);
        for round in 0..64usize {
            writer.write(|v| *v = round);
            writer.publish();
            if round.is_multiple_of(3) {
                reader.update();
            }
            assert_ne!(writer.back, reader.front);
        }
    }
}
