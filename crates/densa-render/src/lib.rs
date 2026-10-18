//! wgpu renderer for dense data grids.
//!
//! Domain-agnostic: an app describes its columns and implements [`GridSource`]; the
//! renderer does layout, text and drawing. Split into a GPU-free half and a thin GPU half
//! so most logic is testable without a GPU:
//!
//! - [`GridPainter`] turns a [`GridSource`] into [`Instance`]s: layout from font metrics,
//!   alignment, overflow, cell styles and glyphs from a cached atlas (`densa_text`).
//!   Allocation-free per frame once the frame's glyphs are cached.
//! - [`GpuGrid`] keeps the glyph atlas texture in sync with the painter's uploads and draws
//!   every instance with one instanced draw call: each quad is either a solid rectangle or
//!   a glyph sampled from the atlas.

mod color;
mod gpu;
mod grid;
mod instance;
mod layout;

pub use color::{srgb, Palette};
pub use gpu::GpuGrid;
pub use grid::{CellStyle, CellText, GridPainter, GridSource, ATLAS_SIZE, CELL_BYTES};
pub use instance::{Instance, InstanceBuffer};
pub use layout::{Align, Column, GridLayout};
