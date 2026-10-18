//! Dates and times of day in ISO 8601 form, without a time-zone database.

use std::fmt::Write as _;

use densa_render::CellText;

pub(crate) const SECONDS_PER_DAY: i64 = 86_400;

/// Year, month (1-12) and day (1-31) of `days` since 1970-01-01, proleptic Gregorian
/// (Howard Hinnant's `civil_from_days`, exact for every `i64` day count this crate sees).
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    // Lossless: day is 1..=31 and month 1..=12.
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `YYYY-MM-DD` for `days` since the epoch.
pub(crate) fn write_date(text: &mut CellText, days: i64) -> std::fmt::Result {
    let (year, month, day) = civil_from_days(days);
    write!(text, "{year:04}-{month:02}-{day:02}")
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
