//! The edits an editor can undo and redo.

use std::collections::VecDeque;

/// Undo steps kept; the oldest go first.
pub(crate) const MAX_STEPS: usize = 100;

/// What made an edit, which decides whether the next edit joins it in one undo step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Typed text: a run of it is one step per word.
    Typing,
    /// Characters deleted before the caret, one after another.
    Backspace,
    /// Characters deleted after the caret, one after another.
    ForwardDelete,
    /// Anything else (a paste, a cut, a selection deleted): a step of its own.
    Other,
}

/// One undo step: `removed` at byte `at` was replaced by `inserted`.
#[derive(Debug, Clone)]
pub(crate) struct Edit {
    pub(crate) at: usize,
    pub(crate) removed: String,
    pub(crate) inserted: String,
    /// The selection (anchor, focus) before the edit, given back by undo.
    pub(crate) before: (usize, usize),
    /// The selection after it, given back by redo.
    pub(crate) after: (usize, usize),
    pub(crate) kind: Kind,
}

impl Edit {
    /// Whether `next`, made right after this edit, continues it.
    fn continued_by(&self, next: &Edit) -> bool {
        match (self.kind, next.kind) {
            (Kind::Typing, Kind::Typing) => {
                // A new word starts a new step.
                let word_ended = self.inserted.ends_with(char::is_whitespace)
                    && !next.inserted.starts_with(char::is_whitespace);
                next.removed.is_empty() && next.at == self.at + self.inserted.len() && !word_ended
            }
            (Kind::Backspace, Kind::Backspace) => next.at + next.removed.len() == self.at,
            (Kind::ForwardDelete, Kind::ForwardDelete) => next.at == self.at,
            _ => false,
        }
    }

    /// Takes `next` into this edit (see [`continued_by`](Self::continued_by)).
    fn absorb(&mut self, next: Edit) {
        match next.kind {
            Kind::Typing => self.inserted.push_str(&next.inserted),
            Kind::Backspace => {
                self.removed.insert_str(0, &next.removed);
                self.at = next.at;
            }
            _ => self.removed.push_str(&next.removed),
        }
        self.after = next.after;
    }
}

/// Undo and redo steps.
#[derive(Debug, Clone, Default)]
pub(crate) struct History {
    done: VecDeque<Edit>,
    undone: Vec<Edit>,
    /// Whether the next edit may join the last step (no caret move or undo since).
    open: bool,
}

impl History {
    /// Records `edit`, joining it to the last step if it continues it. Redo steps are lost.
    pub(crate) fn record(&mut self, edit: Edit) {
        self.undone.clear();
        match self.done.back_mut() {
            Some(last) if self.open && last.continued_by(&edit) => last.absorb(edit),
            _ => {
                if self.done.len() == MAX_STEPS {
                    self.done.pop_front();
                }
                self.done.push_back(edit);
            }
        }
        self.open = true;
    }

    /// Ends the last step: the next edit starts a new one.
    pub(crate) fn seal(&mut self) {
        self.open = false;
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// The step to undo, taken off the history: once applied it goes to the redo steps
    /// ([`push_redo`](Self::push_redo)), or back ([`push_undo`](Self::push_undo)) if it failed.
    pub(crate) fn take_undo(&mut self) -> Option<Edit> {
        self.open = false;
        self.done.pop_back()
    }

    /// The step to redo, taken off the history: once applied it goes to the undo steps
    /// ([`push_undo`](Self::push_undo)), or back ([`push_redo`](Self::push_redo)) if it failed.
    pub(crate) fn take_redo(&mut self) -> Option<Edit> {
        self.open = false;
        self.undone.pop()
    }

    /// Puts `edit` on the undo steps as a step of its own, keeping the redo steps.
    pub(crate) fn push_undo(&mut self, edit: Edit) {
        self.done.push_back(edit);
    }

    pub(crate) fn push_redo(&mut self, edit: Edit) {
        self.undone.push(edit);
    }
}
