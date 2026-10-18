//! Paragraph layout for densa: shaping, line breaking, bidi and alignment.
//!
//! [`TextSystem`] (fonts plus layout scratch space, one per application) turns text and a
//! [`ParagraphStyle`], with optional [`Span`]s, into a [`Paragraph`]: shaped by parley,
//! broken into lines at a maximum width and aligned. A paragraph hands its glyphs out as
//! `densa_text::GlyphRun`s, which a renderer draws (densa-draw's `Canvas::draw_run`), so
//! layout and rendering stay independent of each other.
//!
//! [`TextSelection`] adds what editing needs on top: hit-testing a point to a text position,
//! caret and selection rectangles, and keyboard motions (by character, word and line, in
//! visual order for mixed-direction text).
//!
//! [`TextEditor`] makes text editable: typing, deletion by character, word or line, and
//! input-method (IME) composition, keeping a selection and a laid-out paragraph in step.
//!
//! Laying out allocates and happens when text, style or width change; drawing a laid-out
//! paragraph, hit-testing and moving a selection or a caret do not allocate.
//!
//! # Fonts and scripts
//!
//! [`TextSystem::new`] draws the default `monospace` family with the bundled JetBrains Mono
//! and every character it lacks (Korean, Japanese, Chinese, symbols, ...) with the operating
//! system's fonts; [`TextSystem::bundled`] uses the bundled font alone, the same on every
//! machine. Features:
//!
//! - `system-fonts` (default): find the operating system's fonts. On Linux, fontconfig is
//!   loaded at run time; without it, text falls back to the bundled font.
//! - `complex-scripts`: dictionaries for word breaks in Chinese and Japanese and line breaks
//!   in Thai, Lao, Khmer and Burmese (several MB). Without it, a double-click in Japanese
//!   selects one character.

mod editor;
mod paragraph;
mod selection;
mod style;
mod system;

pub use densa_core::{Color, Point, Rect};
pub use editor::TextEditor;
pub use paragraph::{LineBox, Paragraph};
pub use selection::{Affinity, Motion, TextSelection};
pub use style::{ParagraphError, ParagraphStyle, Span, TextAlign, MAX_FONT_PX, MAX_LINE_HEIGHT};
pub use system::TextSystem;
