//! Paragraph layout for densa: shaping, line breaking, bidi and alignment.
//!
//! [`TextSystem`] (fonts plus layout scratch space, one per application) turns text and a
//! [`ParagraphStyle`], with optional [`Span`]s, into a [`Paragraph`]: shaped by parley,
//! broken into lines at a maximum width and aligned. A paragraph hands its glyphs out as
//! `densa_text::GlyphRun`s, which a renderer draws (densa-render's `Canvas::draw_run`), so
//! layout and rendering stay independent of each other.
//!
//! [`TextSelection`] adds what editing needs on top: hit-testing a point to a text position,
//! caret and selection rectangles, and keyboard motions (by character, word and line, in
//! visual order for mixed-direction text).
//!
//! Laying out allocates and happens when text, style or width change; drawing a laid-out
//! paragraph, hit-testing and moving a selection do not allocate.

mod paragraph;
mod selection;
mod style;
mod system;

pub use densa_core::{Color, Point, Rect};
pub use paragraph::{LineBox, Paragraph};
pub use selection::{Affinity, Motion, TextSelection};
pub use style::{ParagraphError, ParagraphStyle, Span, TextAlign, MAX_FONT_PX, MAX_LINE_HEIGHT};
pub use system::TextSystem;
