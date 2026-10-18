//! OS services for low-latency threads: CPU topology, thread pinning, current CPU.
//!
//! On hybrid CPUs (Intel P-cores + E-cores) the scheduler may move a busy-spinning worker
//! between core types, changing its throughput by 2x or more. These helpers let the engine
//! pick a performance core, pin to it, and detect migrations.
//!
//! Implemented for Windows. Elsewhere, queries return [`CpuError::Unsupported`] and
//! [`current_cpu`] returns `None`, so callers degrade to "not pinned".

use std::cmp::Reverse;
use std::fmt;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as os;

#[cfg(not(windows))]
mod fallback;
#[cfg(not(windows))]
use fallback as os;

mod registry;
pub use registry::{cpu_registry, CpuClaim, CpuRegistry};

/// A logical processor: processor group plus index within the group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CpuId {
    /// Processor group (Windows groups CPUs in sets of up to 64).
    pub group: u16,
    /// Index within the group.
    pub index: u8,
}

impl fmt::Display for CpuId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.group == 0 {
            write!(f, "{}", self.index)
        } else {
            write!(f, "{}:{}", self.group, self.index)
        }
    }
}

/// One logical processor as reported by the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LogicalCpu {
    /// Which logical processor this is.
    pub id: CpuId,
    /// Physical core; hyper-threads of one core share it.
    pub core: u8,
    /// Higher is faster. Equal across all CPUs on non-hybrid machines.
    pub efficiency_class: u8,
}

/// Where a CPU sits relative to the rest of this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreKind {
    /// In the fastest efficiency class (a P-core on hybrid CPUs).
    Performance,
    /// In a slower efficiency class (an E-core).
    Efficiency,
    /// Non-hybrid machine: every CPU has the same efficiency class.
    Uniform,
}

impl CoreKind {
    /// Classifies `cpu` relative to every CPU in `all`.
    pub fn of(cpu: &LogicalCpu, all: &[LogicalCpu]) -> Self {
        let classes = all.iter().map(|c| c.efficiency_class);
        let (min, max) = (classes.clone().min(), classes.max());
        match (min, max) {
            (Some(min), Some(max)) if min != max => {
                if cpu.efficiency_class == max {
                    Self::Performance
                } else {
                    Self::Efficiency
                }
            }
            _ => Self::Uniform,
        }
    }
}

impl fmt::Display for CoreKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Performance => "performance",
            Self::Efficiency => "efficiency",
            Self::Uniform => "uniform",
        })
    }
}

/// Why a CPU topology query or pinning request failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CpuError {
    /// No implementation for this OS.
    Unsupported,
    /// The CPU index does not fit in an affinity mask on this target.
    InvalidCpu(CpuId),
    /// An OS call failed.
    Os {
        /// The failing OS function.
        call: &'static str,
        /// The OS error code (`GetLastError` on Windows).
        code: u32,
    },
}

impl fmt::Display for CpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("CPU affinity is not supported on this platform"),
            Self::InvalidCpu(id) => write!(f, "CPU {id} cannot be expressed in an affinity mask"),
            Self::Os { call, code } => write!(f, "{call} failed (Windows error {code})"),
        }
    }
}

impl std::error::Error for CpuError {}

/// All logical processors available to this process.
pub fn logical_cpus() -> Result<Vec<LogicalCpu>, CpuError> {
    os::logical_cpus()
}

/// Pins the calling thread to `cpu`.
pub fn pin_current_thread(cpu: LogicalCpu) -> Result<(), CpuError> {
    os::pin_current_thread(cpu)
}

/// Pins another thread to `cpu` through its join handle.
pub fn pin_thread<T>(thread: &std::thread::JoinHandle<T>, cpu: LogicalCpu) -> Result<(), CpuError> {
    os::pin_thread(thread, cpu)
}

/// Scheduling priority of a thread, relative to its process's priority class.
///
/// Pinning decides *where* a thread runs; priority decides who wins when another thread
/// wants the same core. A pinned busy-spinning worker at `Normal` time-slices with any other
/// normal thread scheduled there and can stall for a full quantum (10+ ms).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThreadPriority {
    /// The OS default.
    #[default]
    Normal,
    /// One step above normal.
    AboveNormal,
    /// The highest non-critical level: preempts normal threads of every normal process.
    Highest,
    /// Above every non-realtime thread. Starves anything else scheduled on the same core,
    /// so only for a thread pinned to a core it effectively owns.
    TimeCritical,
}

/// Sets the priority of another thread through its join handle.
///
/// # Errors
///
/// [`CpuError::Os`] if the OS rejects the request; [`CpuError::Unsupported`] off Windows.
pub fn set_thread_priority<T>(
    thread: &std::thread::JoinHandle<T>,
    priority: ThreadPriority,
) -> Result<(), CpuError> {
    os::set_thread_priority(thread, priority)
}

/// Sets the priority of the calling thread.
///
/// # Errors
///
/// [`CpuError::Os`] if the OS rejects the request; [`CpuError::Unsupported`] off Windows.
pub fn set_current_thread_priority(priority: ThreadPriority) -> Result<(), CpuError> {
    os::set_current_thread_priority(priority)
}

/// The OS's current priority for another thread, or `None` if it cannot be read or is a
/// level this enum does not model. A system call: not for hot loops.
pub fn thread_priority<T>(thread: &std::thread::JoinHandle<T>) -> Option<ThreadPriority> {
    os::thread_priority(thread)
}

/// The OS's current priority for the calling thread (see [`thread_priority`]).
pub fn current_thread_priority() -> Option<ThreadPriority> {
    os::current_thread_priority()
}

/// The logical processor the calling thread is running on right now.
///
/// Cheap (no syscall on Windows x64) and allocation-free, so it is safe to call per batch.
#[inline]
pub fn current_cpu() -> Option<CpuId> {
    os::current_cpu()
}

/// The CPU a latency-critical thread should be pinned to.
///
/// Fastest efficiency class first; among those the highest-numbered physical core, which
/// keeps clear of core 0 where the OS tends to route interrupts; then that core's first
/// logical processor. Deterministic regardless of enumeration order.
pub fn pick_performance_cpu(cpus: &[LogicalCpu]) -> Option<LogicalCpu> {
    pick_performance_cpu_excluding(cpus, &[])
}

/// Like [`pick_performance_cpu`], skipping every physical core already used by `taken`,
/// so two latency-critical threads never share a core through hyper-threading.
pub fn pick_performance_cpu_excluding(
    cpus: &[LogicalCpu],
    taken: &[LogicalCpu],
) -> Option<LogicalCpu> {
    let core_taken = |c: &LogicalCpu| {
        taken
            .iter()
            .any(|t| t.id.group == c.id.group && t.core == c.core)
    };
    cpus.iter()
        .filter(|c| !core_taken(c))
        .max_by_key(|c| {
            (
                c.efficiency_class,
                Reverse(c.id.group),
                c.core,
                Reverse(c.id.index),
            )
        })
        .copied()
}

/// The CPU with logical index `index` in processor group 0.
pub fn find_cpu(cpus: &[LogicalCpu], index: u8) -> Option<LogicalCpu> {
    cpus.iter()
        .find(|c| c.id == CpuId { group: 0, index })
        .copied()
}
