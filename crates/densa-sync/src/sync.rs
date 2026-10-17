//! Synchronization shim: std primitives normally, loom primitives under `--cfg loom`.
//!
//! Every concurrent data structure in this crate must go through this module so the
//! exact same code can be exhaustively model-checked by loom.

#[cfg(loom)]
pub(crate) use loom::{
    cell::UnsafeCell,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
};

#[cfg(not(loom))]
pub(crate) use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

#[cfg(not(loom))]
pub(crate) use self::cell::UnsafeCell;

#[cfg(not(loom))]
mod cell {
    /// `std::cell::UnsafeCell` exposed through loom's closure-based API.
    ///
    /// Raw pointers never escape the closure, which lets loom track every access.
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct UnsafeCell<T>(std::cell::UnsafeCell<T>);

    impl<T> UnsafeCell<T> {
        pub(crate) const fn new(value: T) -> Self {
            Self(std::cell::UnsafeCell::new(value))
        }

        #[inline(always)]
        pub(crate) fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
            f(self.0.get())
        }

        #[inline(always)]
        pub(crate) fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
            f(self.0.get())
        }
    }
}
