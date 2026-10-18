use std::ops::Range;

use densa_core::{Color, Point, Rect};
use parley::{BoundingBox, Cursor, Layout, Selection};

use crate::Paragraph;

/// A way to move a caret or extend a selection.
///
/// `Left`/`Right` and their word forms move on screen (arrow keys): in right-to-left text
/// `Left` moves forward through the text. `Previous`/`Next` and their word forms move in text
/// order, as deleting does (Backspace removes the logically previous character, wherever it
/// is drawn).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Motion {
    /// One character left on screen.
    Left,
    /// One character right on screen.
    Right,
    /// To the next word boundary left on screen.
    WordLeft,
    /// To the next word boundary right on screen.
    WordRight,
    /// One character back in text order.
    Previous,
    /// One character forward in text order.
    Next,
    /// To the previous word boundary in text order.
    PreviousWord,
    /// To the next word boundary in text order.
    NextWord,
    /// One line up, keeping the horizontal position (the start of the text from the first
    /// line).
    Up,
    /// One line down, keeping the horizontal position (the end of the text from the last
    /// line).
    Down,
    /// To the start of the visual line.
    LineStart,
    /// To the end of the visual line.
    LineEnd,
    /// To the start of the text.
    TextStart,
    /// To the end of the text.
    TextEnd,
}

/// Which character a caret at a position belongs to, where that matters: at a line wrap
/// (end of one line or start of the next) and at a change of text direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Affinity {
    /// With the character after the position (start of the next line at a wrap).
    #[default]
    Downstream,
    /// With the character before the position (end of the line at a wrap): where a caret
    /// stays after typing at the end of a wrapped line.
    Upstream,
}

impl From<Affinity> for parley::Affinity {
    fn from(affinity: Affinity) -> Self {
        match affinity {
            Affinity::Downstream => parley::Affinity::Downstream,
            Affinity::Upstream => parley::Affinity::Upstream,
        }
    }
}

fn from_parley(affinity: parley::Affinity) -> Affinity {
    match affinity {
        parley::Affinity::Upstream => Affinity::Upstream,
        parley::Affinity::Downstream => Affinity::Downstream,
    }
}

/// A caret (collapsed) or a selected range within a [`Paragraph`].
///
/// Positions are byte indices into the paragraph's text, on character (grapheme cluster)
/// boundaries. A selection belongs to the paragraph it was made from: indices read from it
/// ([`range`](Self::range), [`anchor`](Self::anchor), [`focus`](Self::focus)) are
/// meaningless for another text, and after [`Paragraph::reflow`] (or to carry positions to a
/// new paragraph) call [`refreshed`](Self::refreshed). Everything here is allocation-free.
///
/// Equality compares anchor and focus positions with their affinities; the sticky
/// horizontal position used by `Up`/`Down` and the word/line granularity of a drag are not
/// compared.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextSelection {
    inner: Selection,
}

impl TextSelection {
    /// A caret before the character at byte `index` (clamped to the text).
    pub fn caret_at(paragraph: &Paragraph, index: usize) -> Self {
        Self::caret_with(paragraph, index, Affinity::Downstream)
    }

    /// A caret at byte `index` (clamped to the text) with the given `affinity`.
    pub fn caret_with(paragraph: &Paragraph, index: usize, affinity: Affinity) -> Self {
        let cursor = cursor(paragraph, index, affinity);
        Self {
            inner: Selection::new(cursor, cursor),
        }
    }

    /// The range from `anchor` to `focus` (byte indices, clamped to the text; `focus` may
    /// come first): a programmatic selection, such as a search result or an undo restore.
    pub fn from_range(paragraph: &Paragraph, anchor: usize, focus: usize) -> Self {
        let anchor = cursor(paragraph, anchor, Affinity::Downstream);
        let focus = cursor(paragraph, focus, Affinity::Upstream);
        Self {
            inner: Selection::new(anchor, focus),
        }
    }

    /// The whole text, focus at the end.
    pub fn select_all(paragraph: &Paragraph) -> Self {
        Self::from_range(paragraph, 0, paragraph.text_len())
    }

    /// A caret at the text position nearest `point` (relative to the paragraph's top-left
    /// corner): what a click places.
    pub fn at_point(paragraph: &Paragraph, point: Point) -> Self {
        Self {
            inner: Selection::from_point(paragraph.layout(), point.x, point.y),
        }
    }

    /// The word under `point` (a double click).
    pub fn word_at_point(paragraph: &Paragraph, point: Point) -> Self {
        Self {
            inner: Selection::word_from_point(paragraph.layout(), point.x, point.y),
        }
    }

    /// The visual line under `point` (a triple click).
    pub fn line_at_point(paragraph: &Paragraph, point: Point) -> Self {
        Self {
            inner: Selection::line_from_point(paragraph.layout(), point.x, point.y),
        }
    }

    /// The selection with its focus dragged to the position nearest `point`. A selection
    /// begun by word or line keeps extending by whole words or lines.
    #[must_use]
    pub fn extend_to_point(self, paragraph: &Paragraph, point: Point) -> Self {
        Self {
            inner: self
                .inner
                .extend_to_point(paragraph.layout(), point.x, point.y),
        }
    }

    /// The selection extended to `point` by a shift+click: the focus lands where clicked,
    /// and a word or line selection keeps the far side of its word or line as the anchor.
    #[must_use]
    pub fn shift_click(self, paragraph: &Paragraph, point: Point) -> Self {
        Self {
            inner: self
                .inner
                .shift_click_extension(paragraph.layout(), point.x, point.y),
        }
    }

    /// The selection after `motion`: the caret moves, or with `extend` the focus moves and
    /// the anchor stays (shift + navigation keys).
    #[must_use]
    pub fn moved(self, paragraph: &Paragraph, motion: Motion, extend: bool) -> Self {
        let layout = paragraph.layout();
        let s = self.inner;
        let inner = match motion {
            Motion::Left => s.previous_visual(layout, extend),
            Motion::Right => s.next_visual(layout, extend),
            Motion::WordLeft => s.previous_visual_word(layout, extend),
            Motion::WordRight => s.next_visual_word(layout, extend),
            Motion::Up => s.previous_line(layout, extend),
            Motion::Down => s.next_line(layout, extend),
            Motion::LineStart => s.line_start(layout, extend),
            Motion::LineEnd => s.line_end(layout, extend),
            Motion::Previous => to(s, previous_logical(layout, s.focus()), extend),
            Motion::Next => to(s, next_logical(layout, s.focus()), extend),
            Motion::PreviousWord => to(s, s.focus().previous_logical_word(layout), extend),
            Motion::NextWord => to(s, s.focus().next_logical_word(layout), extend),
            Motion::TextStart => to(s, cursor(paragraph, 0, Affinity::Downstream), extend),
            Motion::TextEnd => {
                let end = cursor(paragraph, paragraph.text_len(), Affinity::Downstream);
                to(s, end, extend)
            }
        };
        Self { inner }
    }

    /// The same anchor and focus (clamped to `paragraph`'s text), with everything derived
    /// from the layout recomputed: call after [`Paragraph::reflow`] or to carry a selection to
    /// a new paragraph. Pixel-based state (the sticky column of `Up`/`Down`, a word or line
    /// drag's granularity) is dropped, since the lines it was measured on are gone.
    #[must_use]
    pub fn refreshed(self, paragraph: &Paragraph) -> Self {
        let clamp = |c: Cursor| cursor(paragraph, c.index(), from_parley(c.affinity()));
        let anchor = clamp(self.inner.anchor());
        let focus = clamp(self.inner.focus());
        Self {
            inner: Selection::new(anchor, focus).refresh(paragraph.layout()),
        }
    }

    /// A caret at the focus.
    #[must_use]
    pub fn collapsed(self) -> Self {
        Self {
            inner: self.inner.collapse(),
        }
    }

    /// Whether nothing is selected (a caret).
    pub fn is_caret(&self) -> bool {
        self.inner.is_collapsed()
    }

    /// The selected byte range, in text order.
    pub fn range(&self) -> Range<usize> {
        self.inner.text_range()
    }

    /// Where the selection started (stays put while extending).
    pub fn anchor(&self) -> usize {
        self.inner.anchor().index()
    }

    /// Where the selection ends: the caret's position.
    pub fn focus(&self) -> usize {
        self.inner.focus().index()
    }

    /// The caret's affinity at the focus.
    pub fn focus_affinity(&self) -> Affinity {
        from_parley(self.inner.focus().affinity())
    }

    /// The caret's rectangle at the focus, `width` pixels wide, relative to the paragraph's
    /// top-left corner.
    pub fn caret_rect(&self, paragraph: &Paragraph, width: f32) -> Rect {
        to_rect(self.inner.focus().geometry(paragraph.layout(), width))
    }

    /// Calls `f` with each rectangle covering the selection (one or more per line), relative
    /// to the paragraph's top-left corner. Nothing for a caret.
    pub fn for_each_rect(&self, paragraph: &Paragraph, mut f: impl FnMut(Rect)) {
        self.inner
            .geometry_with(paragraph.layout(), |bounds, _line| f(to_rect(bounds)));
    }
}

/// Where Backspace with the caret at byte `index` starts deleting: the previous character
/// (code point), so a combining accent comes off its letter as in other editors, except that
/// a line break or an emoji is deleted whole.
pub(crate) fn backspace_start(paragraph: &Paragraph, text: &str, index: usize) -> usize {
    let at = cursor(paragraph, index, Affinity::Downstream);
    let [upstream, _] = at.logical_clusters(paragraph.layout());
    let Some(cluster) = upstream else {
        return index;
    };
    if cluster.is_hard_line_break() || cluster.is_emoji() {
        return cluster.text_range().start;
    }
    text.get(..index)
        .and_then(|before| before.char_indices().next_back())
        .map_or(index, |(start, _)| start)
}

/// A cursor at byte `index` clamped to the text.
fn cursor(paragraph: &Paragraph, index: usize, affinity: Affinity) -> Cursor {
    let index = index.min(paragraph.text_len());
    Cursor::from_byte_index(paragraph.layout(), index, affinity.into())
}

/// `selection` moved to `target`: a caret there, or the focus there if `extend`.
fn to(selection: Selection, target: Cursor, extend: bool) -> Selection {
    if extend {
        selection.extend(target)
    } else {
        Selection::new(target, target)
    }
}

/// The start of the character (grapheme cluster) logically before `at`.
fn previous_logical(layout: &Layout<Color>, at: Cursor) -> Cursor {
    let [upstream, _] = at.logical_clusters(layout);
    upstream.map_or(at, |cluster| {
        let start = cluster.text_range().start;
        Cursor::from_byte_index(layout, start, parley::Affinity::Downstream)
    })
}

/// The end of the character (grapheme cluster) logically after `at`.
fn next_logical(layout: &Layout<Color>, at: Cursor) -> Cursor {
    let [_, downstream] = at.logical_clusters(layout);
    downstream.map_or(at, |cluster| {
        let end = cluster.text_range().end;
        Cursor::from_byte_index(layout, end, parley::Affinity::Upstream)
    })
}

fn to_rect(b: BoundingBox) -> Rect {
    // Narrowing: parley widens the f32 layout coordinates to f64; converting back is exact
    // for those values.
    Rect::from_edges(b.x0 as f32, b.y0 as f32, b.x1 as f32, b.y1 as f32)
}
