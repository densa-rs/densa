//! wgpu renderer for dense data grids.
//!
//! Split into a GPU-free half and a thin GPU half so most logic is testable without a GPU:
//!
//! - [`GridPainter`] turns a `ViewportSnapshot` into [`Instance`]s (layout, alignment,
//!   number formatting, flashes). Allocation-free per frame.
//! - [`GpuGrid`] uploads instances and draws them all with one instanced draw call: every
//!   quad is either a solid rectangle or a glyph sampled from the [`GlyphAtlas`].

mod atlas;
mod color;
mod gpu;
mod grid;
mod instance;

pub use atlas::{GlyphAtlas, ATLAS_HEIGHT, ATLAS_WIDTH, GLYPH_PX};
pub use color::{srgb, Palette};
pub use gpu::GpuGrid;
pub use grid::{flash_alpha, Align, Column, GridLayout, GridPainter, COLUMNS, FLASH_NS};
pub use instance::{Instance, InstanceBuffer};
