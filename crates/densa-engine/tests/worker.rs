//! Ingestion worker: config validation, publishing, viewport, pacing, pinning, shutdown.

use std::time::{Duration, Instant};

use densa_data::ViewportSnapshot;
use densa_engine::{spawn_worker, Clock, ConfigError, Pin, SpawnError, WorkerConfig};
use densa_sync::Reader;

const ROWS: usize = 8;

fn config(rate: Option<u64>) -> WorkerConfig {
    WorkerConfig {
        symbols: 1_000,
        seed: 7,
        batch_size: 64,
        rate,
        publish_interval_ns: 0,
        pin: Pin::None,
    }
}

#[cfg(windows)]
#[test]
fn pinned_worker_stays_on_its_cpu() {
    use densa_platform::{logical_cpus, pick_performance_cpu};
    let target = pick_performance_cpu(&logical_cpus().unwrap()).unwrap();
    let cfg = WorkerConfig {
        pin: Pin::Cpu(target),
        ..config(None)
    };
    let (handle, _reader) = spawn_worker::<ROWS>(cfg, Clock::new()).unwrap();
    assert_eq!(handle.pinned_cpu(), Some(target));
    std::thread::sleep(Duration::from_millis(300));
    let metrics = handle.stop().unwrap();
    assert_eq!(metrics.last_cpu, Some(target.id), "{metrics:?}");
    // At most the one hop from wherever the thread started onto the target.
    assert!(metrics.cpu_migrations <= 1, "{metrics:?}");
}

#[cfg(windows)]
#[test]
fn concurrent_auto_workers_get_separate_physical_cores() {
    let auto = || WorkerConfig {
        pin: Pin::Auto,
        ..config(None)
    };
    let (first, _r1) = spawn_worker::<ROWS>(auto(), Clock::new()).unwrap();
    let (second, _r2) = spawn_worker::<ROWS>(auto(), Clock::new()).unwrap();
    let (a, b) = (first.pinned_cpu().unwrap(), second.pinned_cpu().unwrap());
    assert_ne!(a.core, b.core, "both workers landed on core {}", a.core);
    first.stop().unwrap();
    second.stop().unwrap();
}

#[cfg(windows)]
#[test]
fn stopping_a_worker_releases_its_cpu_claim() {
    use densa_platform::cpu_registry;
    let cfg = WorkerConfig {
        pin: Pin::Auto,
        ..config(None)
    };
    let (handle, _reader) = spawn_worker::<ROWS>(cfg, Clock::new()).unwrap();
    let cpu = handle.pinned_cpu().unwrap();
    let held = |cpu| {
        cpu_registry()
            .claimed()
            .iter()
            .filter(|c| **c == cpu)
            .count()
    };
    let while_running = held(cpu);
    handle.stop().unwrap();
    assert_eq!(held(cpu), while_running - 1);
}

#[test]
fn dropping_the_handle_stops_the_worker() {
    let (handle, mut reader) = spawn_worker::<ROWS>(config(None), Clock::new()).unwrap();
    wait_for(&mut reader, Duration::from_secs(5), |s| s.seq >= 1);
    drop(handle);
    reader.update(); // Drain whatever was published before the worker exited.
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        !reader.has_update(),
        "worker kept publishing after its handle was dropped"
    );
}

#[cfg(windows)]
#[test]
fn unpinned_worker_reports_its_cpu() {
    let (handle, _reader) = spawn_worker::<ROWS>(config(None), Clock::new()).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let metrics = handle.stop().unwrap();
    assert!(metrics.last_cpu.is_some(), "{metrics:?}");
}

#[cfg(windows)]
#[test]
fn pinning_failure_stops_the_worker_and_reports_why() {
    use densa_platform::{CpuId, LogicalCpu};
    let bogus = LogicalCpu {
        id: CpuId {
            group: 0,
            index: 63,
        },
        core: 63,
        efficiency_class: 0,
    };
    let cfg = WorkerConfig {
        pin: Pin::Cpu(bogus),
        ..config(None)
    };
    match spawn_worker::<ROWS>(cfg, Clock::new()) {
        Err(SpawnError::Pin(e)) => {
            assert!(e.to_string().contains("SetThreadGroupAffinity"), "{e}");
        }
        other => panic!("expected pin error, got {:?}", other.map(|_| ())),
    }
}

/// Polls until `pred` holds for a fresh snapshot, or panics after `timeout`.
fn wait_for(
    reader: &mut Reader<ViewportSnapshot<ROWS>>,
    timeout: Duration,
    mut pred: impl FnMut(&ViewportSnapshot<ROWS>) -> bool,
) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        reader.update();
        if reader.read(&mut pred) {
            return;
        }
        std::thread::yield_now();
    }
    panic!("condition not reached within {timeout:?}");
}

#[test]
fn rejects_invalid_config() {
    let cases = [
        (
            WorkerConfig {
                symbols: 0,
                ..config(None)
            },
            ConfigError::NoSymbols,
        ),
        (
            WorkerConfig {
                batch_size: 0,
                ..config(None)
            },
            ConfigError::EmptyBatch,
        ),
        (config(Some(0)), ConfigError::ZeroRate),
    ];
    for (cfg, expected) in cases {
        assert_eq!(cfg.validate(), Err(expected.clone()));
        match spawn_worker::<ROWS>(cfg, Clock::new()) {
            Err(SpawnError::Config(e)) => assert_eq!(e, expected),
            other => panic!("expected config error, got {:?}", other.map(|_| ())),
        }
    }
}

#[test]
fn errors_have_readable_messages_and_sources() {
    use std::error::Error;
    assert_eq!(
        ConfigError::NoSymbols.to_string(),
        "symbol count must be at least 1"
    );
    assert_eq!(
        ConfigError::EmptyBatch.to_string(),
        "batch size must be at least 1"
    );
    let spawn = SpawnError::Config(ConfigError::ZeroRate);
    assert_eq!(
        spawn.to_string(),
        "invalid worker config: tick rate must be positive (omit it to run unbounded)"
    );
    assert!(spawn.source().is_some());
    let io = SpawnError::Thread(std::io::Error::other("no threads left"));
    assert_eq!(
        io.to_string(),
        "failed to spawn worker thread: no threads left"
    );
    assert!(io.source().is_some());
    assert_eq!(
        densa_engine::WorkerPanicked.to_string(),
        "ingestion worker thread panicked"
    );
}

#[test]
fn default_config_is_valid_and_unpinned() {
    assert_eq!(WorkerConfig::default().validate(), Ok(()));
    assert_eq!(WorkerConfig::default().pin, Pin::None);
}

#[test]
fn publishes_increasing_snapshots_and_stops_cleanly() {
    let (handle, mut reader) = spawn_worker::<ROWS>(config(None), Clock::new()).unwrap();

    wait_for(&mut reader, Duration::from_secs(5), |s| s.seq >= 1);
    let first = reader.read(|s| s.seq);
    wait_for(&mut reader, Duration::from_secs(5), |s| s.seq > first + 10);

    let metrics = handle.stop().expect("worker panicked");
    assert!(metrics.publishes > 10);
    assert_eq!(metrics.ticks_applied, metrics.batches * 64);
    assert_eq!(metrics.ticks_rejected, 0);
}

#[test]
fn follows_the_requested_viewport() {
    let (handle, mut reader) = spawn_worker::<ROWS>(config(None), Clock::new()).unwrap();
    handle.set_viewport(500);
    wait_for(&mut reader, Duration::from_secs(5), |s| s.first_row == 500);
    reader.read(|s| {
        assert_eq!(s.len as usize, ROWS);
        assert_eq!(s.visible()[0].symbol_id, 500);
    });

    // Past the end: the worker clamps to the last full page.
    handle.set_viewport(u32::MAX);
    wait_for(&mut reader, Duration::from_secs(5), |s| {
        s.first_row == 1_000 - ROWS as u32
    });
    handle.stop().unwrap();
}

#[test]
fn rate_limited_worker_stays_near_target_rate() {
    let rate = 100_000;
    let started = Instant::now();
    let (handle, _reader) = spawn_worker::<ROWS>(config(Some(rate)), Clock::new()).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    let elapsed = started.elapsed().as_secs_f64();
    let applied = handle.metrics().ticks_applied as f64;
    handle.stop().unwrap();

    let expected = rate as f64 * elapsed;
    // Generous bounds: this checks pacing works, not scheduler precision.
    assert!(
        applied <= expected * 1.1 + 64.0,
        "too fast: {applied} vs {expected}"
    );
    assert!(
        applied >= expected * 0.5,
        "too slow: {applied} vs {expected}"
    );
}

#[test]
fn stop_is_prompt_even_while_waiting_on_a_slow_pacer() {
    // 1 tick/s with batch 64 => next batch deadline is ~64 s away.
    let (handle, _reader) = spawn_worker::<ROWS>(
        WorkerConfig {
            rate: Some(1),
            ..config(None)
        },
        Clock::new(),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let started = Instant::now();
    handle.stop().unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn publish_interval_throttles_publishes() {
    let cfg = WorkerConfig {
        publish_interval_ns: 50_000_000, // 20 Hz
        ..config(None)
    };
    let (handle, _reader) = spawn_worker::<ROWS>(cfg, Clock::new()).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let metrics = handle.stop().unwrap();
    assert!(metrics.batches > metrics.publishes * 10, "{metrics:?}");
    assert!(metrics.publishes <= 8, "{metrics:?}");
}
