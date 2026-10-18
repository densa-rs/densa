//! CPU selection that skips physical cores already in use.

use densa_platform::{pick_performance_cpu_excluding, CpuId, LogicalCpu};

fn cpu(index: u8, core: u8, efficiency_class: u8) -> LogicalCpu {
    LogicalCpu {
        id: CpuId { group: 0, index },
        core,
        efficiency_class,
    }
}

#[test]
fn second_pick_lands_on_a_different_physical_core() {
    // Two hyper-threaded P-cores and one E-core.
    let cpus = [
        cpu(0, 0, 1),
        cpu(1, 0, 1),
        cpu(2, 2, 1),
        cpu(3, 2, 1),
        cpu(4, 4, 0),
    ];
    let first = pick_performance_cpu_excluding(&cpus, &[]).unwrap();
    assert_eq!(first.id.index, 2);
    let second = pick_performance_cpu_excluding(&cpus, &[first]).unwrap();
    assert_eq!(second.id.index, 0, "not the hyper-thread sibling 3");
    let third = pick_performance_cpu_excluding(&cpus, &[first, second]).unwrap();
    assert_eq!(
        third.id.index, 4,
        "falls back to the next class once P-cores are taken"
    );
    assert_eq!(
        pick_performance_cpu_excluding(&cpus, &[first, second, third]),
        None
    );
}
