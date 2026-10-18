use std::fmt::Write as _;

use densa_fmt::{civil_from_days, write_fixed, FmtBuf, FmtError};

/// What an axis's values are, which decides where its ticks go and how they read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AxisKind {
    /// Plain numbers: ticks at 1, 2 or 5 times a power of ten.
    Number,
    /// Seconds since 1970-01-01 UTC: ticks at whole seconds, minutes, hours or days,
    /// labelled as times of day or dates (in UTC; there is no time-zone database).
    Time,
}

/// The values one axis of a plot spans, `start` below `end`.
///
/// Built from any two values: they are put in order, a single value is widened around
/// itself (a flat series still gets an axis), and non-finite ones give `0..1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Axis {
    start: f64,
    end: f64,
    kind: AxisKind,
}

/// Seconds in a day.
const DAY: f64 = 86_400.0;

/// Tick steps of a time axis, in seconds, from one second to a day; finer and coarser
/// steps are 1, 2 or 5 times a power of ten (of seconds, or of days).
const TIME_STEPS: [f64; 18] = [
    1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1_800.0, 3_600.0, 7_200.0,
    10_800.0, 21_600.0, 43_200.0, DAY,
];

/// Tick indices past this cannot be told apart in an `f64` value.
const MAX_TICK_INDEX: f64 = 9_007_199_254_740_992.0;

impl Axis {
    /// An axis of numbers from `start` to `end`.
    pub fn number(start: f64, end: f64) -> Self {
        Self::new(start, end, AxisKind::Number)
    }

    /// An axis of times from `start` to `end`, in seconds since 1970-01-01 UTC.
    pub fn time(start: f64, end: f64) -> Self {
        Self::new(start, end, AxisKind::Time)
    }

    /// An axis of `kind` from `start` to `end`.
    pub fn new(start: f64, end: f64, kind: AxisKind) -> Self {
        let (start, end) = if start.is_finite() && end.is_finite() {
            // Halved so the span itself stays finite.
            let limit = f64::MAX / 2.0;
            let (low, high) = (start.min(end), start.max(end));
            (low.clamp(-limit, limit), high.clamp(-limit, limit))
        } else {
            (0.0, 1.0)
        };
        let (start, end) = if end > start {
            (start, end)
        } else {
            let half = if start == 0.0 {
                1.0
            } else {
                (start.abs() * 0.01).max(f64::MIN_POSITIVE)
            };
            (start - half, start + half)
        };
        Self { start, end, kind }
    }

    /// The first value.
    pub fn start(&self) -> f64 {
        self.start
    }

    /// The last value.
    pub fn end(&self) -> f64 {
        self.end
    }

    /// `end - start`, always above zero.
    pub fn span(&self) -> f64 {
        self.end - self.start
    }

    /// What the values are.
    pub fn kind(&self) -> AxisKind {
        self.kind
    }

    /// The axis grown by `fraction` of its span at each end: room around a series so it
    /// does not touch the plot's edges.
    #[must_use]
    pub fn padded(self, fraction: f64) -> Self {
        let pad = self.span() * fraction.max(0.0);
        Self::new(self.start - pad, self.end + pad, self.kind)
    }

    /// The axis moved by `delta` (panning).
    #[must_use]
    pub fn shifted(self, delta: f64) -> Self {
        Self::new(self.start + delta, self.end + delta, self.kind)
    }

    /// The axis scaled by `factor` around `anchor`, which stays where it is: below 1 zooms
    /// in, above 1 out. A factor that is not above zero changes nothing.
    #[must_use]
    pub fn zoomed(self, factor: f64, anchor: f64) -> Self {
        if !(factor > 0.0 && factor.is_finite() && anchor.is_finite()) {
            return self;
        }
        Self::new(
            anchor - (anchor - self.start) * factor,
            anchor + (self.end - anchor) * factor,
            self.kind,
        )
    }

    /// Where `value` falls: 0 at `start`, 1 at `end`, beyond those outside the axis.
    pub fn fraction(&self, value: f64) -> f64 {
        (value - self.start) / self.span()
    }

    /// The value at `fraction` of the way from `start` to `end`.
    pub fn value_at(&self, fraction: f64) -> f64 {
        self.start + fraction * self.span()
    }

    /// Evenly spaced round values within the axis, at most `max + 1` of them, ready to be
    /// labelled ([`Ticks::write_label`]).
    pub fn ticks(&self, max: usize) -> Ticks {
        // Lossless enough: a tick count is small.
        let raw = self.span() / max.max(1) as f64;
        let step = match self.kind {
            AxisKind::Number => nice_step(raw),
            AxisKind::Time if raw < 1.0 => nice_step(raw),
            AxisKind::Time if raw > DAY => nice_step(raw / DAY) * DAY,
            AxisKind::Time => TIME_STEPS
                .iter()
                .copied()
                .find(|&step| step >= raw)
                .unwrap_or(DAY),
        };
        let first = (self.start / step).ceil();
        let last = (self.end / step).floor();
        let usable = step.is_finite() && step > 0.0;
        let (next, last) = if usable && first.abs() < MAX_TICK_INDEX && last.abs() < MAX_TICK_INDEX
        {
            // Lossless: both are whole numbers below 2^53.
            (first as i64, last as i64)
        } else {
            (1, 0)
        };
        Ticks {
            next,
            last,
            step,
            kind: self.kind,
        }
    }
}

/// The smallest of 1, 2, 5 and 10 times a power of ten that is at least `raw`.
fn nice_step(raw: f64) -> f64 {
    let magnitude = 10f64.powf(raw.log10().floor());
    let norm = raw / magnitude;
    let multiple = if norm <= 1.0 {
        1.0
    } else if norm <= 2.0 {
        2.0
    } else if norm <= 5.0 {
        5.0
    } else {
        10.0
    };
    multiple * magnitude
}

/// The ticks of an [`Axis`], smallest first: whole multiples of [`step`](Self::step).
#[derive(Debug, Clone)]
pub struct Ticks {
    next: i64,
    last: i64,
    step: f64,
    kind: AxisKind,
}

impl Ticks {
    /// The distance between two ticks.
    pub fn step(&self) -> f64 {
        self.step
    }

    /// Appends the label of tick `value`, as precise as the step needs: `0.25` for steps of
    /// 0.05, `14:30` for steps of minutes, `2026-10-19` for steps of days. All or nothing,
    /// like every `densa_fmt` write.
    ///
    /// # Errors
    /// [`FmtError::Overflow`] if the label does not fit, [`FmtError::NotFinite`] for a
    /// value that is not finite.
    pub fn write_label<const N: usize>(
        &self,
        value: f64,
        text: &mut FmtBuf<N>,
    ) -> Result<(), FmtError> {
        if !value.is_finite() {
            return Err(FmtError::NotFinite);
        }
        match self.kind {
            AxisKind::Number => {
                let digits = decimals(self.step);
                // Lossless: at most 15 digits.
                let scale = 10f64.powi(digits as i32);
                // A value that rounds to zero is written without a sign: "0.00", not "-0.00".
                let value = if (value * scale).round() == 0.0 {
                    0.0
                } else {
                    value
                };
                write_fixed(text, value, digits)
            }
            AxisKind::Time => {
                let mark = text.len();
                write_time(text, value, self.step).map_err(|_| {
                    text.truncate(mark);
                    FmtError::Overflow
                })
            }
        }
    }
}

impl Iterator for Ticks {
    type Item = f64;

    fn next(&mut self) -> Option<f64> {
        if self.next > self.last {
            return None;
        }
        // Lossless: tick indices stay below 2^53.
        let value = self.next as f64 * self.step;
        self.next += 1;
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = usize::try_from(self.last - self.next + 1).unwrap_or(0);
        (left, Some(left))
    }
}

impl ExactSizeIterator for Ticks {}

/// Fraction digits a label needs to tell ticks `step` apart (at most 15).
fn decimals(step: f64) -> usize {
    // The epsilon keeps exact powers of ten (0.1) from asking for one digit too many.
    let digits = (-step.log10() - 1e-9).ceil();
    // Lossless: clamped to 0..=15.
    digits.clamp(0.0, 15.0) as usize
}

/// A time label for a tick `step` apart from its neighbours: `HH:MM:SS.fff` below a
/// second, `HH:MM:SS` below a minute, `HH:MM` below a day, `YYYY-MM-DD` from a day.
fn write_time<const N: usize>(text: &mut FmtBuf<N>, value: f64, step: f64) -> std::fmt::Result {
    if step >= DAY {
        // Lossless enough: dates within the range `civil_from_days` handles.
        let days = (value / DAY).floor() as i64;
        let (year, month, day) = civil_from_days(days);
        let sign = if year < 0 { "-" } else { "" };
        let digits = year.unsigned_abs();
        return write!(text, "{sign}{digits:04}-{month:02}-{day:02}");
    }
    let digits = if step < 1.0 { decimals(step).min(9) } else { 0 };
    // Lossless: digits is at most 9.
    let scale = 10i64.pow(digits as u32);
    // Lossless: whole units of the last digit, checked to fit an i64 below.
    let units = (value * scale as f64).round();
    if units.abs() >= 9.0e18 {
        return Err(std::fmt::Error);
    }
    let units = units as i64;
    let seconds = units.div_euclid(scale).rem_euclid(86_400);
    let fraction = units.rem_euclid(scale);
    let (hours, minutes, secs) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
    write!(text, "{hours:02}:{minutes:02}")?;
    if step < 60.0 {
        write!(text, ":{secs:02}")?;
    }
    if digits > 0 {
        write!(text, ".{fraction:0digits$}")?;
    }
    Ok(())
}
