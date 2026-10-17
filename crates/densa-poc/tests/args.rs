use densa_poc::{Args, Mode};

fn parse(args: &[&str]) -> Result<Args, String> {
    Args::parse(args.iter().map(|s| s.to_string())).map_err(|e| e.to_string())
}

#[test]
fn defaults_to_window_mode_with_sensible_settings() {
    let args = parse(&[]).unwrap();
    assert_eq!(args.mode, Mode::Window);
    assert!(!args.help);
    assert_eq!(args.seconds, 10);
    assert_eq!(args.fps, 120);
    assert_eq!(args.worker.symbols, 10_000);
    assert_eq!(args.worker.rate, Some(200_000));
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
    assert_eq!(args.seconds, 3);
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
        "--help",
    ] {
        assert!(densa_poc::USAGE.contains(flag), "usage is missing {flag}");
    }
}
