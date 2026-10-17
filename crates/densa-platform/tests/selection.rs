//! Platform-independent CPU selection logic.

use densa_platform::{find_cpu, pick_performance_cpu, CoreKind, CpuId, LogicalCpu};

fn cpu(index: u8, core: u8, efficiency_class: u8) -> LogicalCpu {
    LogicalCpu {
        id: CpuId { group: 0, index },
        core,
        efficiency_class,
    }
}

/// Shaped like an i7-14700KF: 8 hyper-threaded P-cores (class 1) + E-cores (class 0).
fn hybrid() -> Vec<LogicalCpu> {
    let mut cpus = Vec::new();
    for core in 0..8u8 {
        cpus.push(cpu(core * 2, core * 2, 1));
        cpus.push(cpu(core * 2 + 1, core * 2, 1));
    }
    for e in 16..28u8 {
        cpus.push(cpu(e, e, 0));
    }
    cpus
}

#[test]
fn picks_a_performance_core_away_from_core_zero() {
    let picked = pick_performance_cpu(&hybrid()).unwrap();
    assert_eq!(picked.efficiency_class, 1);
    // Highest P-core (logical 14/15), first logical processor of it.
    assert_eq!(
        picked.id,
        CpuId {
            group: 0,
            index: 14
        }
    );
}

#[test]
fn uniform_machines_still_avoid_core_zero() {
    let cpus: Vec<_> = (0..4).map(|i| cpu(i, i, 0)).collect();
    assert_eq!(pick_performance_cpu(&cpus).unwrap().id.index, 3);
}

#[test]
fn selection_does_not_depend_on_enumeration_order() {
    let mut cpus = hybrid();
    cpus.reverse();
    assert_eq!(pick_performance_cpu(&cpus).unwrap().id.index, 14);
}

#[test]
fn no_cpus_means_no_choice() {
    assert_eq!(pick_performance_cpu(&[]), None);
}

#[test]
fn core_kind_is_relative_to_the_machine() {
    let cpus = hybrid();
    assert_eq!(CoreKind::of(&cpus[0], &cpus), CoreKind::Performance);
    assert_eq!(CoreKind::of(&cpus[20], &cpus), CoreKind::Efficiency);
    let uniform = [cpu(0, 0, 0), cpu(1, 1, 0)];
    assert_eq!(CoreKind::of(&uniform[0], &uniform), CoreKind::Uniform);
    assert_eq!(CoreKind::Performance.to_string(), "performance");
    assert_eq!(CoreKind::Efficiency.to_string(), "efficiency");
    assert_eq!(CoreKind::Uniform.to_string(), "uniform");
}

#[test]
fn finds_cpus_by_index_in_group_zero() {
    let cpus = hybrid();
    assert_eq!(find_cpu(&cpus, 20).unwrap().efficiency_class, 0);
    assert_eq!(find_cpu(&cpus, 99), None);
}

#[test]
fn ids_display_compactly() {
    assert_eq!(CpuId { group: 0, index: 7 }.to_string(), "7");
    assert_eq!(CpuId { group: 1, index: 3 }.to_string(), "1:3");
}
