//! Primitive types shared by every densa crate.
//!
//! Dependency-free on purpose: text layout, rendering and application code all agree on
//! these types without depending on each other.

mod color;
mod geometry;

pub use color::Color;
pub use geometry::{Point, Rect, Size};
