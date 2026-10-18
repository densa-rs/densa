//! Command-line parsing, pin resolution and the CPU table.

use densa_platform::{CpuId, LogicalCpu};
use densa_poc::{resolve_pin, write_cpu_table, Args, Mode, PinRequest};

fn cpu(index: u8, core: u8, efficiency_class: u8) -> LogicalCpu {
    LogicalCpu {
        id: CpuId { group: 0, index },
        core,
        efficiency_class,
    }
}

fn hybrid() -> Vec<LogicalCpu> {
    vec![
        cpu(0, 0, 1),
        cpu(1, 0, 1),
        cpu(2, 2, 1),
        cpu(3, 2, 1),
        cpu(4, 4, 0),
        cpu(5, 5, 0),
    ]
}

#[test]
fn pin_option_variants() {
    assert_eq!(parse(&["--pin", "auto"]).unwrap().pin, PinRequest::Auto);
    assert_eq!(parse(&["--pin", "none"]).unwrap().pin, PinRequest::None);
    assert_eq!(parse(&["--pin", "5"]).unwrap().pin, PinRequest::Index(5));
    assert_eq!(
        parse(&["--pin", "fast"]).unwrap_err(),
        "`--pin` expects `auto`, `none` or a CPU index (0-255), got `fast`"
    );
    assert_eq!(
        parse(&["--pin", "256"]).unwrap_err(),
        "`--pin` expects `auto`, `none` or a CPU index (0-255), got `256`"
    );
}

mod priority_option {
    use super::*;
    use densa_platform::ThreadPriority;

    #[test]
    fn accepts_every_level() {
        let levels = [
            ("normal", ThreadPriority::Normal),
            ("above", ThreadPriority::AboveNormal),
            ("high", ThreadPriority::Highest),
            ("critical", ThreadPriority::TimeCritical),
        ];
        for (flag, expected) in levels {
            let args = parse(&["--priority", flag]).unwrap();
            assert_eq!(args.worker.priority, expected, "--priority {flag}");
        }
    }

    #[test]
    fn defaults_to_time_critical_for_the_pinned_worker() {
        // scripts/priority_experiment.ps1: next to a busy normal-priority neighbor on the
        // same core, only time-critical kept snapshot age p99 under 1 ms (normal: 32 ms).
        assert_eq!(
            parse(&[]).unwrap().worker.priority,
            ThreadPriority::TimeCritical
        );
    }

    #[test]
    fn rejects_unknown_levels_with_the_valid_choices() {
        assert_eq!(
            parse(&["--priority", "max"]).unwrap_err(),
            "`--priority` expects `normal`, `above`, `high` or `critical`, got `max`"
        );
    }
}

#[test]
fn list_cpus_flag() {
    assert!(parse(&["--list-cpus"]).unwrap().list_cpus);
    assert!(!parse(&[]).unwrap().list_cpus);
}

mod resolve_pin {
    use super::*;
    use densa_engine::Pin;

    #[test]
    fn none_leaves_the_worker_unpinned() {
        assert_eq!(resolve_pin(PinRequest::None, &hybrid()), Ok(Pin::None));
    }

    #[test]
    fn auto_defers_core_choice_to_the_engine_registry() {
        // The engine picks among cores not already claimed by other workers, so the CLI
        // must not pre-pick one (that is how two workers ended up sharing CPU 14).
        assert_eq!(resolve_pin(PinRequest::Auto, &hybrid()), Ok(Pin::Auto));
    }

    #[test]
    fn index_selects_that_cpu() {
        assert_eq!(
            resolve_pin(PinRequest::Index(5), &hybrid()),
            Ok(Pin::Cpu(cpu(5, 5, 0)))
        );
    }

    #[test]
    fn unknown_index_is_an_error_pointing_at_list_cpus() {
        let err = resolve_pin(PinRequest::Index(9), &hybrid()).unwrap_err();
        assert_eq!(err.to_string(), "CPU 9 does not exist (see --list-cpus)");
    }
}

mod describe_pin {
    use super::*;
    use densa_poc::describe_pin;

    #[test]
    fn pinned_worker_names_its_cpu_and_core_kind() {
        let note = describe_pin(Some(cpu(2, 2, 1)), PinRequest::Auto, &hybrid());
        assert_eq!(note, "worker pinned to cpu 2 (performance)");
    }

    #[test]
    fn unpinned_by_request() {
        assert_eq!(
            describe_pin(None, PinRequest::None, &hybrid()),
            "worker not pinned"
        );
    }

    #[test]
    fn auto_that_found_no_free_core_says_so() {
        assert_eq!(
            describe_pin(None, PinRequest::Auto, &hybrid()),
            "worker not pinned (auto: no free core, or no CPU topology on this OS)"
        );
    }
}

#[test]
fn cpu_table_labels_core_kinds() {
    let mut out = Vec::new();
    write_cpu_table(&mut out, &hybrid()).unwrap();
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "cpu  core  class  kind");
    assert_eq!(lines[1], "  0     0      1  performance");
    assert_eq!(lines[5], "  4     4      0  efficiency");
    assert!(text.contains("--pin auto would choose cpu 2"), "{text}");
}

fn parse(args: &[&str]) -> Result<Args, String> {
    Args::parse(args.iter().map(|s| s.to_string())).map_err(|e| e.to_string())
}

#[test]
fn defaults_to_window_mode_with_sensible_settings() {
    let args = parse(&[]).unwrap();
    assert_eq!(args.mode, Mode::Window);
    assert!(!args.help);
    assert_eq!(
        args.seconds, None,
        "headless defaults to 10 s; a window runs until closed"
    );
    assert_eq!(args.fps, 120);
    assert_eq!(args.worker.symbols, 10_000);
    assert_eq!(args.worker.rate, Some(200_000));
    assert_eq!(args.pin, PinRequest::Auto);
}

#[test]
fn parses_every_option() {
    let args = parse(&[
        "--headless",
        "--seconds",
        "3",
        "--rate",
        "50000",
        "--symbols",
        "500",
        "--batch",
        "32",
        "--publish-interval-us",
        "250",
        "--fps",
        "60",
        "--seed",
        "9",
    ])
    .unwrap();
    assert_eq!(args.mode, Mode::Headless);
    assert_eq!(args.seconds, Some(3));
    assert_eq!(args.fps, 60);
    assert_eq!(args.worker.rate, Some(50_000));
    assert_eq!(args.worker.symbols, 500);
    assert_eq!(args.worker.batch_size, 32);
    assert_eq!(args.worker.publish_interval_ns, 250_000);
    assert_eq!(args.worker.seed, 9);
}

#[test]
fn unbounded_clears_the_rate() {
    let args = parse(&["--headless", "--unbounded"]).unwrap();
    assert_eq!(args.worker.rate, None);
}

#[test]
fn help_flags() {
    assert!(parse(&["--help"]).unwrap().help);
    assert!(parse(&["-h"]).unwrap().help);
}

#[test]
fn rejects_bad_input_with_clear_messages() {
    let cases: &[(&[&str], &str)] = &[
        (&["--bogus"], "unknown argument `--bogus`"),
        (&["--seconds"], "`--seconds` needs a value"),
        (
            &["--seconds", "abc"],
            "`--seconds` expects a non-negative integer, got `abc`",
        ),
        (
            &["--seconds", "-1"],
            "`--seconds` expects a non-negative integer, got `-1`",
        ),
        (&["--seconds", "0"], "`--seconds` must be at least 1"),
        (&["--fps", "0"], "`--fps` must be at least 1"),
        (
            &["--rate", "0"],
            "tick rate must be positive (omit it to run unbounded)",
        ),
        (&["--symbols", "0"], "symbol count must be at least 1"),
        (
            &["--symbols", "5000000000"],
            "`--symbols` is too large (max 4294967295)",
        ),
        (
            &["--rate", "10", "--unbounded"],
            "`--rate` and `--unbounded` are mutually exclusive",
        ),
        (
            &["--unbounded", "--rate", "10"],
            "`--rate` and `--unbounded` are mutually exclusive",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(parse(input).unwrap_err(), *expected, "input: {input:?}");
    }
}

#[test]
fn usage_mentions_every_option() {
    for flag in [
        "--headless",
        "--seconds",
        "--rate",
        "--unbounded",
        "--symbols",
        "--batch",
        "--publish-interval-us",
        "--fps",
        "--seed",
        "--pin",
        "--priority",
        "--list-cpus",
        "--help",
    ] {
        assert!(densa_poc::USAGE.contains(flag), "usage is missing {flag}");
    }
}
