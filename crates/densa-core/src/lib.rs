//! Primitive types shared by every densa crate.
//!
//! Dependency-free on purpose: text layout, rendering and application code all agree on
//! these types without depending on each other.

mod color;

pub use color::Color;
