//! Runs the real `trading-demo` binary: exit codes, messages, and a short headless session.

use std::process::{Command, Output};

/// One child process at a time: sessions run busy-spinning workers, and separate processes
/// cannot see each other's CPU claims, so on a 2-CPU machine two sessions starve each other.
static CHILD: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run(args: &[&str]) -> Output {
    let _one_at_a_time = CHILD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Command::new(env!("CARGO_BIN_EXE_trading-demo"))
        .args(args)
        .output()
        .expect("failed to launch trading-demo")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn help_prints_usage_and_succeeds() {
    let out = run(&["--help"]);
    assert!(out.status.success());
    assert!(text(&out.stdout).starts_with("Usage: trading-demo"));
}

#[test]
fn bad_argument_exits_2_with_message_and_usage() {
    let out = run(&["--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    let err = text(&out.stderr);
    assert!(err.contains("error: unknown argument `--bogus`"), "{err}");
    assert!(err.contains("Usage: trading-demo"), "{err}");
}

/// Opens a real window for 2 s. Needs a desktop session and a GPU; set
/// `DENSA_SKIP_GPU_TESTS=1` to skip where neither exists.
#[test]
fn window_session_renders_at_display_rate_with_zero_densa_heap_ops() {
    if std::env::var_os("DENSA_SKIP_GPU_TESTS").is_some() {
        return;
    }
    let out = run(&["--seconds", "2", "--symbols", "2000"]);
    let stdout = text(&out.stdout);
    assert!(
        out.status.success(),
        "stderr: {}\nstdout: {stdout}",
        text(&out.stderr)
    );
    assert!(stdout.starts_with("densa window: "), "{stdout}");
    assert!(stdout.contains("Fifo"), "V-Sync present mode: {stdout}");

    let summary = stdout.lines().last().unwrap_or_default();
    assert!(summary.starts_with("summary:"), "{stdout}");
    assert!(
        summary.contains("heap ops after startup: worker 0, paint 0, gpu submit"),
        "{summary}"
    );
    let fps: f64 = summary
        .split_once(" frames (")
        .and_then(|(_, rest)| rest.split_once(" fps"))
        .and_then(|(fps, _)| fps.parse().ok())
        .unwrap_or_else(|| panic!("no fps in {summary}"));
    assert!(fps > 30.0, "only {fps} fps: {summary}");
}

#[cfg(windows)]
#[test]
fn list_cpus_prints_the_topology() {
    let out = run(&["--list-cpus"]);
    assert!(out.status.success());
    let stdout = text(&out.stdout);
    assert!(stdout.starts_with("cpu  core  class  kind"), "{stdout}");
    assert!(stdout.contains("--pin auto would choose cpu"), "{stdout}");
}

/// Distinct physical cores of this machine.
#[cfg(windows)]
fn physical_cores() -> usize {
    let cpus = densa_platform::logical_cpus().unwrap();
    let mut cores: Vec<(u16, u8)> = cpus.iter().map(|c| (c.id.group, c.core)).collect();
    cores.sort_unstable();
    cores.dedup();
    cores.len()
}

#[cfg(windows)]
#[test]
fn pinned_headless_session_stays_on_one_cpu() {
    let out = run(&[
        "--headless",
        "--seconds",
        "2",
        "--symbols",
        "1000",
        "--pin",
        "auto",
    ]);
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "stderr: {}", text(&out.stderr));
    let header = stdout.lines().next().unwrap_or_default();
    if physical_cores() < 2 {
        // One physical core (a 2-vCPU CI runner): automatic placement keeps the last free
        // core for everything else, so the worker runs unpinned at normal priority.
        assert!(header.contains("worker not pinned"), "{stdout}");
        assert!(header.contains("priority normal"), "{stdout}");
        return;
    }
    assert!(header.contains("worker pinned to cpu"), "{stdout}");
    let summary = stdout.lines().last().unwrap_or_default();
    // 0, or 1 if the first batch ran before the OS applied the affinity.
    assert!(
        summary.contains("migrations 0 ") || summary.contains("migrations 1 "),
        "{summary}"
    );
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
