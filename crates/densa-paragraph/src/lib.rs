//! Paragraph layout for densa: shaping, line breaking, bidi and alignment.
//!
//! [`TextSystem`] (fonts plus layout scratch space, one per application) turns text and a
//! [`ParagraphStyle`], with optional [`Span`]s, into a [`Paragraph`]: shaped by parley,
//! broken into lines at a maximum width and aligned. A paragraph hands its glyphs out as
//! `densa_text::GlyphRun`s, which a renderer draws (densa-render's `Canvas::draw_run`), so
//! layout and rendering stay independent of each other.
//!
//! Laying out allocates and happens when text, style or width change; drawing a laid-out
//! paragraph every frame does not allocate.

mod paragraph;
mod style;
mod system;

pub use paragraph::{LineBox, Paragraph};
pub use style::{
    Color, ParagraphError, ParagraphStyle, Span, TextAlign, MAX_FONT_PX, MAX_LINE_HEIGHT,
};
pub use system::TextSystem;
