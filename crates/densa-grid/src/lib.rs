//! A data grid over your own data, drawn onto a `densa_draw::Canvas`.
//!
//! Domain-agnostic: an application describes its [`Column`]s and implements [`GridSource`]
//! (one call per visible cell, formatting into a stack buffer); [`GridPainter`] lays the
//! columns out from font metrics, aligns text, fills overflowing numbers with `#` like a
//! spreadsheet, and draws cell styles and glyphs. No GPU dependency: hand the canvas's
//! instances to `densa-render` (or any other backend) to show them.
//!
//! A widget showing a grid asks its data through [`GridModel`] instead: the same cells, plus
//! the rows to scroll through and sorting by column.
//!
//! Painting allocates nothing once the frame's characters are cached.
//!
//! ```
//! use densa_draw::{Canvas, Color, InstanceBuffer};
//! use densa_grid::{Align, CellStyle, CellText, Column, GridPainter, GridSource, Palette};
//! use densa_text::{AtlasConfig, Font, GlyphCache};
//!
//! /// Your data, as the grid asks for it: a cell at a time, into a stack buffer.
//! struct Hosts(Vec<(&'static str, f64)>);
//!
//! impl GridSource for Hosts {
//!     fn rows(&self) -> usize {
//!         self.0.len()
//!     }
//!
//!     fn cell(&self, row: usize, column: usize, text: &mut CellText) -> CellStyle {
//!         let (name, load) = self.0[row];
//!         // A cell that does not fit its buffer stays empty rather than failing the frame.
//!         let _ = match column {
//!             0 => text.push_str(name),
//!             _ => densa_fmt::write_fixed(text, load, 1),
//!         };
//!         CellStyle::new(Color::WHITE)
//!     }
//! }
//!
//! let columns = vec![Column::new("host", 12, Align::Left), Column::new("load", 6, Align::Right)];
//! let mut painter = GridPainter::new(columns, 20, Font::bundled_mono()?, 13.0, Palette::new());
//!
//! let mut cache = GlyphCache::new(AtlasConfig::default());
//! let mut instances = InstanceBuffer::with_capacity(4096);
//! let mut canvas = Canvas::begin(&mut cache, &mut instances);
//! painter.paint(&Hosts(vec![("web-1", 0.42), ("db-1", 3.5)]), &mut canvas);
//! assert!(instances.len() > 0, "titles, cells and their glyphs");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod grid;
mod layout;
mod model;
mod palette;

pub use densa_core::{Color, Point, Rect};
pub use grid::{CellStyle, CellText, GridPainter, GridSource, CELL_BYTES};
pub use layout::{Align, Column, GridLayout};
pub use model::{GridModel, SortOrder};
pub use palette::Palette;
