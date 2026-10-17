use densa_fmt::{write_fixed, write_grouped, FmtBuf, FmtError};
use std::fmt::Write as _;

/// Deterministic xorshift so the oracle sweep is reproducible.
fn values(count: usize) -> impl Iterator<Item = f64> {
    let mut s = 0x9e37_79b9_7f4a_7c15u64;
    (0..count).map(move |i| {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let mantissa = (s >> 11) as f64 / (1u64 << 53) as f64; // [0, 1)
        let exponent = (i % 19) as i32 - 6; // 1e-6 ..= 1e12
        let sign = if s & 1 == 0 { 1.0 } else { -1.0 };
        sign * mantissa * 10f64.powi(exponent)
    })
}

/// Reference implementation for thousands grouping (allocates; test-only).
fn grouped_oracle(value: u64, sep: char) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(ch);
    }
    out
}

#[test]
fn fixed_matches_std_format_exactly() {
    let mut buf = FmtBuf::<64>::new();
    for value in values(20_000) {
        for decimals in 0..=6 {
            buf.clear();
            write_fixed(&mut buf, value, decimals).unwrap();
            assert_eq!(
                buf.as_str(),
                format!("{value:.decimals$}"),
                "value={value:e}"
            );
        }
    }
}

#[test]
fn fixed_rounds_ties_like_std() {
    // 0.125 is exact in binary: std rounds the exact value half-to-even -> "0.12".
    // A naive `(x * 100).round()` would print "0.13".
    let mut buf = FmtBuf::<16>::new();
    for (value, expected) in [
        (0.125, "0.12"),
        (0.375, "0.38"),
        (2.5, "2"),
        (1.005, "1.00"),
    ] {
        buf.clear();
        let decimals = if value == 2.5 { 0 } else { 2 };
        write_fixed(&mut buf, value, decimals).unwrap();
        assert_eq!(buf.as_str(), expected);
    }
}

#[test]
fn fixed_rejects_non_finite_and_leaves_buffer_untouched() {
    let mut buf = FmtBuf::<16>::new();
    buf.push_str("px=").unwrap();
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(write_fixed(&mut buf, value, 2), Err(FmtError::NotFinite));
        assert_eq!(buf.as_str(), "px=");
    }
}

#[test]
fn fixed_overflow_rolls_back_to_previous_content() {
    let mut buf = FmtBuf::<8>::new();
    buf.push_str("ab").unwrap();
    assert_eq!(
        write_fixed(&mut buf, 123_456.789, 2),
        Err(FmtError::Overflow)
    );
    assert_eq!(buf.as_str(), "ab");
    write_fixed(&mut buf, 1.5, 1).unwrap();
    assert_eq!(buf.as_str(), "ab1.5");
}

#[test]
fn grouped_matches_oracle() {
    let mut buf = FmtBuf::<32>::new();
    let samples = [
        0,
        7,
        999,
        1_000,
        12_345,
        999_999,
        1_000_000,
        1_234_567_890,
        u64::MAX,
    ];
    for value in samples
        .into_iter()
        .chain((0..64).map(|shift| 1u64 << shift))
    {
        buf.clear();
        write_grouped(&mut buf, value, ',').unwrap();
        assert_eq!(buf.as_str(), grouped_oracle(value, ','), "value={value}");
    }
}

#[test]
fn grouped_supports_non_ascii_separators() {
    let mut buf = FmtBuf::<32>::new();
    write_grouped(&mut buf, 1_234_567, '\u{2009}').unwrap(); // thin space
    assert_eq!(buf.as_str(), "1\u{2009}234\u{2009}567");
}

#[test]
fn grouped_overflow_rolls_back() {
    let mut buf = FmtBuf::<6>::new();
    buf.push_str("v=").unwrap();
    assert_eq!(
        write_grouped(&mut buf, 1_234_567, ','),
        Err(FmtError::Overflow)
    );
    assert_eq!(buf.as_str(), "v=");
}

#[test]
fn buffer_basics() {
    let mut buf = FmtBuf::<4>::default();
    assert!(buf.is_empty());
    assert_eq!(buf.capacity(), 4);
    buf.push_str("abcd").unwrap();
    assert_eq!(buf.len(), 4);
    assert_eq!(buf.as_bytes(), b"abcd");
    assert_eq!(buf.push_str("e"), Err(FmtError::Overflow));
    assert_eq!(buf.as_str(), "abcd");
    buf.clear();
    assert!(buf.is_empty());
}

#[test]
fn push_str_is_all_or_nothing_for_multibyte_text() {
    let mut buf = FmtBuf::<3>::new();
    buf.push_str("a").unwrap();
    // "€" is 3 bytes; it must not be split to fill the remaining 2.
    assert_eq!(buf.push_str("€"), Err(FmtError::Overflow));
    assert_eq!(buf.as_str(), "a");
}

#[test]
fn implements_fmt_write() {
    let mut buf = FmtBuf::<16>::new();
    write!(buf, "{}-{:>3}", 12, 'x').unwrap();
    assert_eq!(buf.as_str(), "12-  x");
    assert!(buf.write_str("this does not fit").is_err());
}

#[test]
fn errors_have_readable_messages() {
    assert_eq!(
        FmtError::Overflow.to_string(),
        "formatted text exceeds buffer capacity"
    );
    assert_eq!(FmtError::NotFinite.to_string(), "value is NaN or infinite");
}
