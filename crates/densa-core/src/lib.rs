//! Primitive types shared by every densa crate.
//!
//! Dependency-free on purpose: text layout, rendering and application code all agree on
//! these types without depending on each other.
//!
//! ```
//! use densa_core::{Color, Point, Rect};
//!
//! // x, y, width, height. Stored as edges: neighbours built from the same edge value
//! // (`Rect::from_edges`, or `translate`) tile exactly.
//! let cell = Rect::new(10.0, 20.0, 100.0, 18.0);
//! assert!(cell.contains(Point::new(10.0, 20.0)));
//! assert!(!cell.contains(Point::new(110.0, 20.0)), "the right edge is outside");
//! assert_eq!(cell.intersect(Rect::new(0.0, 0.0, 50.0, 50.0)).width(), 40.0);
//!
//! // Colors are linear RGB; `srgb` reads the hex code a design tool gives.
//! let accent = Color::srgb(0x3B82F6);
//! assert_eq!(accent.to_srgb_hex(), 0x3B82F6);
//! let faint = accent.with_alpha(0.25);
//! assert_eq!(faint.a, 0.25);
//! ```

mod color;
mod geometry;

pub use color::Color;
pub use geometry::{Point, Rect, Size};
