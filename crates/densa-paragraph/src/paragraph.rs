use std::ops::Range;

use densa_text::{Font, GlyphId, GlyphRun, PositionedGlyph};
use parley::{Alignment, AlignmentOptions, FontData, Layout, PositionedLayoutItem};

use crate::style::{check_width, ParagraphError, TextAlign};
use densa_core::Color;

/// One laid-out line, in pixels relative to the paragraph's top-left corner.
#[derive(Debug, Clone, PartialEq)]
pub struct LineBox {
    /// Baseline, a whole pixel.
    pub baseline: f32,
    /// Top of the line box.
    pub top: f32,
    /// Bottom of the line box.
    pub bottom: f32,
    /// Left edge of the line's content after alignment.
    pub left: f32,
    /// Advance of the line's content.
    pub width: f32,
    /// Byte range of the text on this line.
    pub text_range: Range<usize>,
}

/// Identifies a font face by its bytes (address and length) and face index.
///
/// Not by parley's blob id: the same bytes registered twice, or reloaded under a new blob,
/// are still one face, sharing one `densa_text::Font` and its cached glyphs. Holders keep
/// the bytes alive (each `Font` owns an `Arc` of them), so an address is never reused for
/// other data while its key is in use.
pub(crate) type FaceKey = (usize, usize, u32);

/// The [`FaceKey`] of `bytes` at face `index`.
pub(crate) fn bytes_key(bytes: &[u8], index: u32) -> FaceKey {
    (bytes.as_ptr() as usize, bytes.len(), index)
}

/// The [`FaceKey`] of a face parley chose.
pub(crate) fn face_key(data: &FontData) -> FaceKey {
    bytes_key(data.data.data(), data.index)
}

/// A font face a paragraph uses.
#[derive(Debug)]
pub(crate) struct Face {
    pub(crate) key: FaceKey,
    pub(crate) font: Font,
}

#[derive(Debug, Clone, Copy)]
struct RunRecord {
    face: usize,
    px: f32,
    color: Color,
    glyphs: (u32, u32),
}

/// Shaped, line-broken and aligned text, ready to draw as [`GlyphRun`]s.
///
/// Built by `TextSystem::paragraph`. Changing the width ([`reflow`](Self::reflow)) re-breaks
/// the shaped text without shaping it again; changing the text or styles needs a new
/// paragraph. Iterating [`runs`](Self::runs) never allocates, so drawing a paragraph every
/// frame is allocation-free.
///
/// Not yet supported: variable-font axes and synthetic bold/italic (glyphs are drawn from
/// the face's default instance), color glyphs (emoji draw as coverage masks).
pub struct Paragraph {
    layout: Layout<Color>,
    text_len: usize,
    align: TextAlign,
    max_width: Option<f32>,
    faces: Vec<Face>,
    glyphs: Vec<PositionedGlyph>,
    runs: Vec<RunRecord>,
    lines: Vec<LineBox>,
}

impl std::fmt::Debug for Paragraph {
    // Hand-written: parley's layout does not implement Debug.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Paragraph")
            .field("align", &self.align)
            .field("max_width", &self.max_width)
            .field("faces", &self.faces.len())
            .field("glyphs", &self.glyphs.len())
            .field("runs", &self.runs.len())
            .field("lines", &self.lines)
            .finish_non_exhaustive()
    }
}

impl Paragraph {
    /// Breaks and aligns `layout` (of a `text_len`-byte text), resolving each font face it
    /// uses once through `resolve`.
    pub(crate) fn new(
        layout: Layout<Color>,
        text_len: usize,
        align: TextAlign,
        max_width: Option<f32>,
        resolve: &mut dyn FnMut(&FontData) -> Option<Font>,
    ) -> Result<Self, ParagraphError> {
        let mut paragraph = Self {
            layout,
            text_len,
            align,
            max_width,
            faces: Vec::new(),
            glyphs: Vec::new(),
            runs: Vec::new(),
            lines: Vec::new(),
        };
        paragraph.break_and_flatten(resolve)?;
        Ok(paragraph)
    }

    /// Width of the widest line, in pixels.
    pub fn width(&self) -> f32 {
        self.layout.width()
    }

    /// Height of all lines, in pixels: the bottom of the last line box.
    ///
    /// Taken from the (pixel-snapped) line boxes rather than parley's unsnapped total, so
    /// stacking paragraphs by height never drifts from where their lines are drawn.
    pub fn height(&self) -> f32 {
        self.lines.last().map_or(0.0, |line| line.bottom)
    }

    /// The width lines wrap at; `None` if they only break at newlines.
    pub fn max_width(&self) -> Option<f32> {
        self.max_width
    }

    /// The lines, top to bottom.
    pub fn lines(&self) -> &[LineBox] {
        &self.lines
    }

    /// Glyphs to draw, in all runs.
    pub fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }

    /// The glyph runs to draw, positioned relative to the paragraph's top-left corner.
    pub fn runs(&self) -> impl ExactSizeIterator<Item = GlyphRun<'_>> + '_ {
        self.runs.iter().map(|run| {
            let (start, end) = run.glyphs;
            GlyphRun {
                font: &self.faces[run.face].font,
                px: run.px,
                color: run.color,
                glyphs: &self.glyphs[start as usize..end as usize],
            }
        })
    }

    /// Re-breaks and re-aligns the lines for a new `max_width` (`None`: no wrapping),
    /// without shaping again.
    ///
    /// # Errors
    /// [`ParagraphError::InvalidWidth`] if `max_width` is negative or not finite.
    pub fn reflow(&mut self, max_width: Option<f32>) -> Result<(), ParagraphError> {
        self.max_width = check_width(max_width)?;
        // Re-breaking reuses the shaped runs, so every face is known already.
        self.break_and_flatten(&mut |_| None)
    }

    fn break_and_flatten(
        &mut self,
        resolve: &mut dyn FnMut(&FontData) -> Option<Font>,
    ) -> Result<(), ParagraphError> {
        self.layout.break_all_lines(self.max_width);
        let options = AlignmentOptions::default();
        self.layout.align(alignment(self.align), options);

        self.glyphs.clear();
        self.runs.clear();
        self.lines.clear();
        for line in self.layout.lines() {
            let metrics = line.metrics();
            self.lines.push(LineBox {
                baseline: metrics.baseline,
                top: metrics.block_min_coord,
                bottom: metrics.block_max_coord,
                left: metrics.offset,
                width: metrics.advance,
                // Clamped: parley gives an empty text's single line the range 0..1.
                text_range: clamp(line.text_range(), self.text_len),
            });
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue; // Inline boxes are not supported yet.
                };
                let data = run.run().font();
                let key = face_key(data);
                let face = match self.faces.iter().position(|f| f.key == key) {
                    Some(face) => face,
                    None => {
                        // Parley read this face to shape with it, so this only fails if the
                        // data cannot be parsed for drawing: an error, not a silent gap.
                        let font = resolve(data).ok_or(ParagraphError::UnreadableFont)?;
                        self.faces.push(Face { key, font });
                        self.faces.len() - 1
                    }
                };
                // Lossless: a paragraph holds far fewer than 2^32 glyphs.
                let start = self.glyphs.len() as u32;
                self.glyphs
                    .extend(run.positioned_glyphs().map(|g| PositionedGlyph {
                        id: GlyphId(g.id),
                        x: g.x,
                        y: g.y,
                    }));
                let end = self.glyphs.len() as u32;
                self.runs.push(RunRecord {
                    face,
                    px: run.run().font_size(),
                    color: run.style().brush,
                    glyphs: (start, end),
                });
            }
        }
        Ok(())
    }
}

fn clamp(range: Range<usize>, len: usize) -> Range<usize> {
    range.start.min(len)..range.end.min(len)
}

fn alignment(align: TextAlign) -> Alignment {
    match align {
        TextAlign::Start => Alignment::Start,
        TextAlign::End => Alignment::End,
        TextAlign::Left => Alignment::Left,
        TextAlign::Center => Alignment::Center,
        TextAlign::Right => Alignment::Right,
        TextAlign::Justify => Alignment::Justify,
    }
}
