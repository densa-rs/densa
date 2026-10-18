//! Viewport scrolling over the virtual grid, independent of the windowing library.

/// What a key or wheel event asks the viewport to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAction {
    /// Move by this many rows (negative is up).
    Rows(i64),
    /// Move by this many viewport pages (negative is up).
    Pages(i64),
    /// Jump to the first row.
    Top,
    /// Jump to the last full page.
    Bottom,
}

/// First visible row of a viewport, kept within `0..=total - visible`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scroll {
    first_row: u32,
    max_first_row: u32,
    visible: u32,
}

impl Scroll {
    /// A viewport of `visible_rows` over `total_rows`, scrolled to the top.
    pub fn new(total_rows: u32, visible_rows: u32) -> Self {
        Self {
            first_row: 0,
            max_first_row: total_rows.saturating_sub(visible_rows),
            visible: visible_rows,
        }
    }

    /// First visible row.
    pub fn first_row(&self) -> u32 {
        self.first_row
    }

    /// Applies `action`; returns `true` if the first visible row changed.
    pub fn apply(&mut self, action: ScrollAction) -> bool {
        let target = match action {
            ScrollAction::Rows(n) => i64::from(self.first_row) + n,
            ScrollAction::Pages(n) => i64::from(self.first_row) + n * i64::from(self.visible),
            ScrollAction::Top => 0,
            ScrollAction::Bottom => i64::from(self.max_first_row),
        };
        // Lossless: clamped into 0..=max_first_row, which came from a u32.
        let clamped = target.clamp(0, i64::from(self.max_first_row)) as u32;
        let changed = clamped != self.first_row;
        self.first_row = clamped;
        changed
    }
}

/// Rows per mouse-wheel notch.
pub const WHEEL_ROWS_PER_LINE: f64 = 3.0;

/// Converts a wheel delta into rows: in lines (`pixels == false`) or in pixels, where
/// `row_height` pixels make a row. Positive deltas scroll toward the top.
pub fn wheel_rows(delta: f64, pixels: bool, row_height: u32) -> i64 {
    let rows = if pixels {
        delta / f64::from(row_height.max(1))
    } else {
        delta * WHEEL_ROWS_PER_LINE
    };
    // Rounded so slow trackpad deltas still move once they add up to half a row.
    -(rows.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_and_pages_clamp_to_the_last_full_page() {
        let mut s = Scroll::new(1_000, 50);
        assert!(s.apply(ScrollAction::Rows(3)));
        assert_eq!(s.first_row(), 3);
        assert!(s.apply(ScrollAction::Pages(2)));
        assert_eq!(s.first_row(), 103);
        s.apply(ScrollAction::Pages(100));
        assert_eq!(s.first_row(), 950);
        assert!(!s.apply(ScrollAction::Rows(1)), "already at the bottom");
        s.apply(ScrollAction::Rows(-10_000));
        assert_eq!(s.first_row(), 0);
    }

    #[test]
    fn top_and_bottom() {
        let mut s = Scroll::new(1_000, 50);
        assert!(s.apply(ScrollAction::Bottom));
        assert_eq!(s.first_row(), 950);
        assert!(s.apply(ScrollAction::Top));
        assert_eq!(s.first_row(), 0);
        assert!(!s.apply(ScrollAction::Top));
    }

    #[test]
    fn tables_smaller_than_the_viewport_never_scroll() {
        let mut s = Scroll::new(10, 50);
        assert!(!s.apply(ScrollAction::Bottom));
        assert_eq!(s.first_row(), 0);
    }

    #[test]
    fn wheel_deltas_scroll_toward_the_top_when_positive() {
        assert_eq!(wheel_rows(1.0, false, 18), -3);
        assert_eq!(wheel_rows(-2.0, false, 18), 6);
        assert_eq!(wheel_rows(36.0, true, 18), -2);
        assert_eq!(wheel_rows(4.0, true, 18), 0, "less than half a row");
        assert_eq!(wheel_rows(10.0, true, 0), -10, "zero row height is guarded");
    }
}
