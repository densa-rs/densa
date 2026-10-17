//! Human-readable metric formatting. Reporting runs outside the measured frame work, so
//! these helpers may allocate. Output is ASCII-only ("us", not "µs") for any console.

/// `850ns`, `45.2us`, `1.25ms`, `3.50s`.
pub fn format_duration_ns(ns: u64) -> String {
    let n = ns as f64;
    match ns {
        0..=999 => format!("{ns}ns"),
        1_000..=999_999 => format!("{:.1}us", n / 1e3),
        1_000_000..=999_999_999 => format!("{:.2}ms", n / 1e6),
        _ => format!("{:.2}s", n / 1e9),
    }
}

/// `950`, `200.0k`, `52.31M`.
pub fn format_rate(per_sec: f64) -> String {
    if per_sec >= 1e6 {
        format!("{:.2}M", per_sec / 1e6)
    } else if per_sec >= 1e3 {
        format!("{:.1}k", per_sec / 1e3)
    } else {
        format!("{per_sec:.0}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_scale_units() {
        assert_eq!(format_rate(950.0), "950");
        assert_eq!(format_rate(200_000.0), "200.0k");
        assert_eq!(format_rate(52_310_000.0), "52.31M");
    }

    #[test]
    fn duration_boundaries() {
        assert_eq!(format_duration_ns(0), "0ns");
        assert_eq!(format_duration_ns(999), "999ns");
        assert_eq!(format_duration_ns(1_000), "1.0us");
        assert_eq!(format_duration_ns(1_000_000), "1.00ms");
        assert_eq!(format_duration_ns(1_000_000_000), "1.00s");
    }
}
