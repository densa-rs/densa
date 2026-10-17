//! Allocation-free number formatting into fixed-capacity stack buffers.
//!
//! Floats go through `core::fmt` (exact, allocation-free, identical output to `format!`);
//! only digit grouping, which `core::fmt` lacks, is hand-written. All writes append
//! all-or-nothing: on error the buffer is left exactly as it was.

use std::fmt::{self, Write as _};

/// Why a value could not be formatted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FmtError {
    /// The text does not fit in the remaining capacity.
    Overflow,
    /// `NaN` or infinite input; callers render their own placeholder.
    NotFinite,
}

impl fmt::Display for FmtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Overflow => "formatted text exceeds buffer capacity",
            Self::NotFinite => "value is NaN or infinite",
        })
    }
}

impl std::error::Error for FmtError {}

/// Fixed-capacity UTF-8 text buffer living entirely on the stack.
#[derive(Debug, Clone)]
pub struct FmtBuf<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> FmtBuf<N> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(self.as_bytes())
            .expect("FmtBuf only ever appends whole `str` values, so it is valid UTF-8")
    }

    /// Appends `s` entirely, or nothing at all if it does not fit.
    pub fn push_str(&mut self, s: &str) -> Result<(), FmtError> {
        let end = self.len + s.len();
        if end > N {
            return Err(FmtError::Overflow);
        }
        self.bytes[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

impl<const N: usize> Default for FmtBuf<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> fmt::Write for FmtBuf<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_str(s).map_err(|_| fmt::Error)
    }
}

/// Appends `value` with exactly `decimals` fractional digits (same output as `format!`).
pub fn write_fixed<const N: usize>(
    buf: &mut FmtBuf<N>,
    value: f64,
    decimals: usize,
) -> Result<(), FmtError> {
    if !value.is_finite() {
        return Err(FmtError::NotFinite);
    }
    let mark = buf.len;
    write!(buf, "{value:.decimals$}").map_err(|_| {
        buf.len = mark;
        FmtError::Overflow
    })
}

/// Appends `value` with `separator` between groups of three digits (e.g. `1,234,567`).
pub fn write_grouped<const N: usize>(
    buf: &mut FmtBuf<N>,
    value: u64,
    separator: char,
) -> Result<(), FmtError> {
    // u64::MAX has 20 decimal digits. Collected least-significant first.
    let mut digits = [0u8; 20];
    let mut count = 0;
    let mut rest = value;
    loop {
        // Lossless: `rest % 10` is a single decimal digit.
        digits[count] = b'0' + (rest % 10) as u8;
        count += 1;
        rest /= 10;
        if rest == 0 {
            break;
        }
    }

    let mut sep_bytes = [0u8; 4];
    let sep: &str = separator.encode_utf8(&mut sep_bytes);
    let mark = buf.len;
    let result = (0..count).rev().try_for_each(|i| {
        let digit = std::str::from_utf8(&digits[i..=i]).expect("ASCII digit");
        buf.push_str(digit)?;
        if i > 0 && i % 3 == 0 {
            buf.push_str(sep)?;
        }
        Ok(())
    });
    if result.is_err() {
        buf.len = mark;
    }
    result
}
