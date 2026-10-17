//! Runs the real `densa-poc` binary: exit codes, messages, and a short headless session.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_densa-poc"))
        .args(args)
        .output()
        .expect("failed to launch densa-poc")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn help_prints_usage_and_succeeds() {
    let out = run(&["--help"]);
    assert!(out.status.success());
    assert!(text(&out.stdout).starts_with("Usage: densa-poc"));
}

#[test]
fn bad_argument_exits_2_with_message_and_usage() {
    let out = run(&["--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    let err = text(&out.stderr);
    assert!(err.contains("error: unknown argument `--bogus`"), "{err}");
    assert!(err.contains("Usage: densa-poc"), "{err}");
}

#[test]
fn window_mode_is_reported_as_not_implemented() {
    let out = run(&[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("window mode is not implemented yet"));
}

#[test]
fn headless_session_reports_every_second_with_zero_heap_ops() {
    let out = run(&["--headless", "--seconds", "2", "--symbols", "1000"]);
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "stderr: {}", text(&out.stderr));
    assert!(
        text(&out.stderr).is_empty(),
        "unexpected warning: {}",
        text(&out.stderr)
    );

    let per_second: Vec<&str> = stdout.lines().filter(|l| l.starts_with('[')).collect();
    assert!(!per_second.is_empty(), "no per-second report:\n{stdout}");
    for line in &per_second {
        assert!(line.contains("heap ops: worker 0, frame loop 0"), "{line}");
    }
    let summary = stdout.lines().last().unwrap_or_default();
    assert!(summary.starts_with("summary:"), "{stdout}");
    assert!(summary.ends_with("worker 0, frame loop 0"), "{summary}");
}
