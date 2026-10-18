//! Human-readable metric formatting. Reporting runs outside the measured frame work, so
//! these helpers may allocate. Output is ASCII-only ("us", not "µs") for any console.

use densa_engine::LatencySummary;
use densa_platform::{CpuId, ThreadPriority};

/// `p50 45.2us p99 1.25ms max 3.50ms`.
pub fn format_latency(s: &LatencySummary) -> String {
    format!(
        "p50 {} p99 {} max {}",
        format_duration_ns(s.p50),
        format_duration_ns(s.p99),
        format_duration_ns(s.max)
    )
}

/// The `--priority` word for a level, or `?` where the OS cannot report it.
pub fn format_priority(priority: Option<ThreadPriority>) -> &'static str {
    match priority {
        Some(ThreadPriority::Normal) => "normal",
        Some(ThreadPriority::AboveNormal) => "above",
        Some(ThreadPriority::Highest) => "high",
        Some(ThreadPriority::TimeCritical) => "critical",
        None => "?",
    }
}

/// `14`, or `?` where the OS cannot report the CPU.
pub fn format_cpu(cpu: Option<CpuId>) -> String {
    cpu.map_or_else(|| "?".to_owned(), |id| id.to_string())
}

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
    fn latency_and_cpu() {
        let s = LatencySummary {
            count: 3,
            p50: 45_200,
            p99: 1_250_000,
            max: 3_500_000,
        };
        assert_eq!(format_latency(&s), "p50 45.2us p99 1.25ms max 3.50ms");
        assert_eq!(format_cpu(None), "?");
        assert_eq!(
            format_cpu(Some(CpuId {
                group: 0,
                index: 14
            })),
            "14"
        );
    }

    #[test]
    fn priority_words_match_the_cli_flag() {
        assert_eq!(format_priority(Some(ThreadPriority::Highest)), "high");
        assert_eq!(
            format_priority(Some(ThreadPriority::TimeCritical)),
            "critical"
        );
        assert_eq!(format_priority(None), "?");
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
