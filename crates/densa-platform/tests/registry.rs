//! Process-wide CPU claims: `auto` placement must never stack two latency-critical threads
//! on one physical core. Each test uses its own registry so parallel tests don't interfere.

use densa_platform::{CpuId, CpuRegistry, LogicalCpu};

fn cpu(index: u8, core: u8, efficiency_class: u8) -> LogicalCpu {
    LogicalCpu {
        id: CpuId { group: 0, index },
        core,
        efficiency_class,
    }
}

/// Two hyper-threaded P-cores (cores 0 and 2) and one E-core (core 4).
fn machine() -> Vec<LogicalCpu> {
    vec![
        cpu(0, 0, 1),
        cpu(1, 0, 1),
        cpu(2, 2, 1),
        cpu(3, 2, 1),
        cpu(4, 4, 0),
    ]
}

#[test]
fn concurrent_auto_claims_land_on_different_physical_cores() {
    let registry = CpuRegistry::new();
    let cpus = machine();
    let a = registry.claim_auto(&cpus).unwrap();
    let b = registry.claim_auto(&cpus).unwrap();
    assert_ne!(a.cpu().core, b.cpu().core);
    assert_eq!(a.cpu().efficiency_class, 1);
    assert_eq!(b.cpu().efficiency_class, 1);
    assert_eq!(registry.claimed().len(), 2);
}

#[test]
fn dropping_a_claim_frees_its_core() {
    let registry = CpuRegistry::new();
    let cpus = machine();
    let first = registry.claim_auto(&cpus).unwrap().cpu();
    // `first`'s claim was a temporary: already released.
    assert!(registry.claimed().is_empty());
    let again = registry.claim_auto(&cpus).unwrap();
    assert_eq!(again.cpu(), first, "the best core is reused once free");
}

#[test]
fn explicit_claims_are_avoided_by_auto() {
    let registry = CpuRegistry::new();
    let cpus = machine();
    let pinned = registry.claim(cpu(3, 2, 1)); // Sibling of the best P-core.
    let auto = registry.claim_auto(&cpus).unwrap();
    assert_eq!(
        auto.cpu().core,
        0,
        "auto skips core 2, claimed through its sibling"
    );
    drop(pinned);
}

#[test]
fn explicit_claims_may_share_a_core_by_request() {
    let registry = CpuRegistry::new();
    let a = registry.claim(cpu(2, 2, 1));
    let b = registry.claim(cpu(2, 2, 1));
    assert_eq!(
        registry.claimed().len(),
        2,
        "explicit requests are honored, not refused"
    );
    drop(a);
    assert_eq!(
        registry.claimed(),
        vec![cpu(2, 2, 1)],
        "one release frees one claim"
    );
    drop(b);
    assert!(registry.claimed().is_empty());
}

#[test]
fn exhausting_performance_cores_falls_back_then_runs_out() {
    let registry = CpuRegistry::new();
    let cpus = machine();
    let p1 = registry.claim_auto(&cpus).unwrap();
    let p2 = registry.claim_auto(&cpus).unwrap();
    let e = registry.claim_auto(&cpus).unwrap();
    assert_eq!(e.cpu().efficiency_class, 0, "only an E-core is left");
    assert!(registry.claim_auto(&cpus).is_none(), "every core is taken");
    drop((p1, p2, e));
    assert!(registry.claimed().is_empty());
}

#[test]
fn global_registry_is_shared() {
    let a: *const CpuRegistry = densa_platform::cpu_registry();
    let b: *const CpuRegistry = densa_platform::cpu_registry();
    assert_eq!(a, b);
}
