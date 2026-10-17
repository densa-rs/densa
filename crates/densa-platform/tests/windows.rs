//! Real OS behaviour. Pinning is verified by observing the thread move, not by trusting
//! the API's return value.
#![cfg(windows)]

use densa_platform::{
    current_cpu, logical_cpus, pick_performance_cpu, pin_current_thread, pin_thread, CpuError,
    CpuId, LogicalCpu,
};
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Spins until the calling thread runs on `target`, or gives up after 2 s.
fn runs_on(target: CpuId) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if current_cpu() == Some(target) {
            return true;
        }
        std::thread::yield_now();
    }
    false
}

#[test]
fn enumerates_every_logical_processor_once() {
    let cpus = logical_cpus().unwrap();
    let parallelism = std::thread::available_parallelism().unwrap().get();
    assert_eq!(cpus.len(), parallelism, "{cpus:?}");
    let unique: HashSet<CpuId> = cpus.iter().map(|c| c.id).collect();
    assert_eq!(unique.len(), cpus.len());
}

#[test]
fn current_cpu_is_one_of_the_enumerated_cpus() {
    let cpus = logical_cpus().unwrap();
    let now = current_cpu().expect("current CPU is always known on Windows");
    assert!(cpus.iter().any(|c| c.id == now));
}

#[test]
fn pinning_the_current_thread_moves_it_to_each_target() {
    let cpus = logical_cpus().unwrap();
    let mut targets: Vec<LogicalCpu> = vec![*cpus.first().unwrap(), *cpus.last().unwrap()];
    targets.dedup();
    // Run on a scratch thread so the test harness thread keeps its affinity.
    std::thread::spawn(move || {
        for target in targets {
            pin_current_thread(target).unwrap();
            assert!(runs_on(target.id), "did not move to {}", target.id);
        }
    })
    .join()
    .unwrap();
}

#[test]
fn pinning_another_thread_by_handle_moves_it() {
    let target = pick_performance_cpu(&logical_cpus().unwrap()).unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let worker = std::thread::spawn(move || {
        rx.recv().unwrap(); // Wait until pinned.
        runs_on(target.id)
    });
    pin_thread(&worker, target).unwrap();
    tx.send(()).unwrap();
    assert!(
        worker.join().unwrap(),
        "pinned thread never ran on {}",
        target.id
    );
}

#[test]
fn pinning_to_a_nonexistent_cpu_is_an_os_error() {
    let bogus = LogicalCpu {
        id: CpuId {
            group: 0,
            index: 63,
        },
        core: 63,
        efficiency_class: 0,
    };
    if logical_cpus().unwrap().iter().any(|c| c.id == bogus.id) {
        return; // A 64-CPU group: no bogus index available in group 0.
    }
    let err = std::thread::spawn(move || pin_current_thread(bogus))
        .join()
        .unwrap()
        .unwrap_err();
    assert!(
        matches!(
            err,
            CpuError::Os {
                call: "SetThreadGroupAffinity",
                ..
            }
        ),
        "{err:?}"
    );
    assert!(err
        .to_string()
        .starts_with("SetThreadGroupAffinity failed (Windows error "));
}
