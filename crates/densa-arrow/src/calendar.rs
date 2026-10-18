//! Dates and times of day in ISO 8601 form, without a time-zone database.

use std::fmt::Write as _;

use densa_fmt::civil_from_days;
use densa_grid::CellText;

pub(crate) const SECONDS_PER_DAY: i64 = 86_400;

/// `YYYY-MM-DD` for `days` since the epoch. Years before year 0 take a minus sign and keep
/// four digits, as ISO 8601 writes them (`-0001`).
pub(crate) fn write_date(text: &mut CellText, days: i64) -> std::fmt::Result {
    let (year, month, day) = civil_from_days(days);
    let sign = if year < 0 { "-" } else { "" };
    let digits = year.unsigned_abs();
    write!(text, "{sign}{digits:04}-{month:02}-{day:02}")
}

/// `HH:MM:SS` for `seconds` since midnight (`0..86_400`), then `.` and `fraction` padded to
/// `digits` digits when `digits` is not zero.
pub(crate) fn write_time(
    text: &mut CellText,
    seconds: i64,
    fraction: i64,
    digits: usize,
) -> std::fmt::Result {
    let (hours, minutes, secs) = (seconds / 3_600, seconds / 60 % 60, seconds % 60);
    write!(text, "{hours:02}:{minutes:02}:{secs:02}")?;
    if digits > 0 {
        write!(text, ".{fraction:0digits$}")?;
    }
    Ok(())
}

/// `YYYY-MM-DD HH:MM:SS[.fraction]` for `value` counted in `1 / per_second` seconds since
/// the epoch; `digits` is the fraction's width (0, 3, 6 or 9).
pub(crate) fn write_timestamp(
    text: &mut CellText,
    value: i64,
    per_second: i64,
    digits: usize,
) -> std::fmt::Result {
    let seconds = value.div_euclid(per_second);
    let fraction = value.rem_euclid(per_second);
    write_date(text, seconds.div_euclid(SECONDS_PER_DAY))?;
    text.write_char(' ')?;
    write_time(text, seconds.rem_euclid(SECONDS_PER_DAY), fraction, digits)
}
