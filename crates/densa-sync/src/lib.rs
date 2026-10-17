//! Lock-free synchronization primitives for low-latency data → UI pipelines.

mod sync;
pub mod triple_buffer;

pub use triple_buffer::{triple_buffer, Reader, Writer};
