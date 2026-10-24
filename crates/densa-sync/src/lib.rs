//! Lock-free synchronization primitives for low-latency data → UI pipelines.
//!
//! ```
//! use densa_sync::triple_buffer;
//!
//! // A worker thread publishes whole snapshots; the UI thread reads the newest one without
//! // waiting for the worker, and the worker never waits for the UI.
//! let (mut writer, mut reader) = triple_buffer(vec![0.0_f64; 4]);
//! let worker = std::thread::spawn(move || {
//!     let mut latest = vec![0.0_f64; 4]; // the worker's own, always current
//!     latest[2] = 101.5;
//!     // The slot handed out holds an older snapshot: overwrite all of it, never a part.
//!     writer.write(|prices| prices.copy_from_slice(&latest));
//!     writer.publish();
//! });
//! worker.join().unwrap();
//!
//! assert!(reader.update(), "a snapshot was published since the last update");
//! assert_eq!(reader.read(|prices| prices[2]), 101.5);
//! assert!(!reader.update(), "nothing newer");
//! ```

mod sync;
pub mod triple_buffer;

pub use triple_buffer::{triple_buffer, Reader, Writer};
