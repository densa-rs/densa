use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::{pick_performance_cpu_excluding, LogicalCpu};

/// Record of the CPUs claimed by latency-critical threads, so automatic placement never
/// stacks two busy-spinning threads on one physical core.
///
/// Placement policy belongs to the process, not to each caller: without it, every
/// `auto`-pinned worker picks the same "best" core and they time-slice it at half speed
/// each. Use the process-wide [`cpu_registry`]; separate instances exist for tests.
///
/// Claims are released when their [`CpuClaim`] guard is dropped. Only threads within this
/// process are visible; another process may still pick the same core.
#[derive(Debug, Default)]
pub struct CpuRegistry {
    claimed: Mutex<Vec<LogicalCpu>>,
}

static GLOBAL: CpuRegistry = CpuRegistry::new();

/// The process-wide registry.
pub fn cpu_registry() -> &'static CpuRegistry {
    &GLOBAL
}

impl CpuRegistry {
    pub const fn new() -> Self {
        Self {
            claimed: Mutex::new(Vec::new()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<LogicalCpu>> {
        // Poison-tolerant: every critical section is a single push or remove on the Vec,
        // so a panicking holder cannot leave it half-updated.
        self.claimed.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Claims the fastest CPU on a physical core that no current claim uses (see
    /// [`pick_performance_cpu_excluding`]). Falls back to slower core classes once the
    /// fastest are taken; `None` when every core is claimed.
    #[must_use = "the CPU is released as soon as the claim is dropped"]
    pub fn claim_auto(&self, cpus: &[LogicalCpu]) -> Option<CpuClaim<'_>> {
        let mut claimed = self.lock();
        let cpu = pick_performance_cpu_excluding(cpus, &claimed)?;
        claimed.push(cpu);
        Some(CpuClaim {
            registry: self,
            cpu,
        })
    }

    /// Records an explicit choice of `cpu`. Never refused, even if the core is already
    /// claimed: an explicit request is the caller's decision. Automatic claims avoid it.
    #[must_use = "the CPU is released as soon as the claim is dropped"]
    pub fn claim(&self, cpu: LogicalCpu) -> CpuClaim<'_> {
        self.lock().push(cpu);
        CpuClaim {
            registry: self,
            cpu,
        }
    }

    /// Current claims, one entry per live [`CpuClaim`].
    pub fn claimed(&self) -> Vec<LogicalCpu> {
        self.lock().clone()
    }

    fn release(&self, cpu: LogicalCpu) {
        let mut claimed = self.lock();
        if let Some(position) = claimed.iter().position(|c| *c == cpu) {
            claimed.swap_remove(position);
        }
    }
}

/// A claimed CPU; releases the claim when dropped.
#[derive(Debug)]
pub struct CpuClaim<'a> {
    registry: &'a CpuRegistry,
    cpu: LogicalCpu,
}

impl CpuClaim<'_> {
    pub fn cpu(&self) -> LogicalCpu {
        self.cpu
    }
}

impl Drop for CpuClaim<'_> {
    fn drop(&mut self) {
        self.registry.release(self.cpu);
    }
}
