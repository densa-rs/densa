//! A raised priority needs a core of its own: with `Pin::Auto` and no free core, the worker
//! stays at normal priority instead of starving the machine.
//!
//! A separate test binary (its own process, so its own CPU registry): exhausting the cores
//! here cannot disturb the pinning tests in `worker.rs`.

use densa_platform::ThreadPriority;
use trading_demo::engine::{spawn_worker, Clock, Pin, WorkerConfig};

const ROWS: usize = 4;

fn auto_critical() -> WorkerConfig {
    WorkerConfig {
        symbols: 100,
        rate: Some(1_000),
        pin: Pin::Auto,
        priority: ThreadPriority::TimeCritical,
        ..WorkerConfig::default()
    }
}

#[test]
fn an_auto_worker_without_a_core_of_its_own_stays_at_normal_priority() {
    // Claim every free core first (on systems without topology there is none to claim).
    let cpus = densa_platform::logical_cpus().unwrap_or_default();
    let registry = densa_platform::cpu_registry();
    let mut held = Vec::new();
    while let Some(claim) = registry.claim_auto(&cpus) {
        held.push(claim);
    }

    let (worker, _reader) = spawn_worker::<ROWS>(auto_critical(), Clock::new())
        .expect("no free core is not an error: the worker runs unpinned");
    assert_eq!(worker.pinned_cpu(), None);
    if cfg!(windows) {
        assert_eq!(worker.priority(), Some(ThreadPriority::Normal));
    }
    worker.stop().unwrap();
    drop(held);
}

#[cfg(windows)]
#[test]
fn an_auto_worker_with_a_core_gets_the_requested_priority_and_can_change_it() {
    let (worker, _reader) = spawn_worker::<ROWS>(auto_critical(), Clock::new()).unwrap();
    if worker.pinned_cpu().is_some() {
        assert_eq!(worker.priority(), Some(ThreadPriority::TimeCritical));
    }
    worker.set_priority(ThreadPriority::AboveNormal).unwrap();
    assert_eq!(worker.priority(), Some(ThreadPriority::AboveNormal));
    worker.stop().unwrap();
}
