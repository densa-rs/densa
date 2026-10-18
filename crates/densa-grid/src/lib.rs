//! A data grid over your own data, drawn onto a `densa_draw::Canvas`.
//!
//! Domain-agnostic: an application describes its [`Column`]s and implements [`GridSource`]
//! (one call per visible cell, formatting into a stack buffer); [`GridPainter`] lays the
//! columns out from font metrics, aligns text, fills overflowing numbers with `#` like a
//! spreadsheet, and draws cell styles and glyphs. No GPU dependency: hand the canvas's
//! instances to `densa-render` (or any other backend) to show them.
//!
//! Painting allocates nothing once the frame's characters are cached.

mod grid;
mod layout;
mod palette;

pub use densa_core::{Color, Point, Rect};
pub use grid::{CellStyle, CellText, GridPainter, GridSource, CELL_BYTES};
pub use layout::{Align, Column, GridLayout};
pub use palette::Palette;
