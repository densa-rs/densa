use std::error::Error;
use std::fmt;
use std::io::Write;

use densa_engine::Pin;
use densa_platform::{find_cpu, logical_cpus, pick_performance_cpu, CoreKind, LogicalCpu};

/// What `--pin` asked for, before it is checked against the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinRequest {
    /// Leave placement to the OS (`--pin none`).
    None,
    /// Let the engine claim a free performance core.
    Auto,
    /// Logical processor index in group 0.
    Index(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Why a `--pin` request cannot be satisfied.
pub enum PinError {
    /// The requested CPU index does not exist.
    NoSuchCpu(u8),
}

impl fmt::Display for PinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchCpu(index) => write!(f, "CPU {index} does not exist (see --list-cpus)"),
        }
    }
}

impl std::error::Error for PinError {}

/// Turns a `--pin` request into the engine's placement policy.
///
/// `auto` is passed through rather than resolved here: the engine claims a core no other
/// worker in this process holds, which a one-off choice made here could not guarantee.
///
/// # Errors
///
/// [`PinError::NoSuchCpu`] when an explicit index is not one of `cpus`.
pub fn resolve_pin(request: PinRequest, cpus: &[LogicalCpu]) -> Result<Pin, PinError> {
    match request {
        PinRequest::None => Ok(Pin::None),
        PinRequest::Auto => Ok(Pin::Auto),
        PinRequest::Index(index) => find_cpu(cpus, index)
            .map(Pin::Cpu)
            .ok_or(PinError::NoSuchCpu(index)),
    }
}

/// [`resolve_pin`] against the live topology, queried only for an explicit index.
pub(crate) fn worker_pin(request: PinRequest) -> Result<Pin, Box<dyn Error>> {
    let cpus = match request {
        PinRequest::Index(_) => logical_cpus()?,
        PinRequest::None | PinRequest::Auto => Vec::new(),
    };
    Ok(resolve_pin(request, &cpus)?)
}

/// One-line description of where the worker ended up, for run headers.
pub fn describe_pin(
    pinned: Option<LogicalCpu>,
    request: PinRequest,
    cpus: &[LogicalCpu],
) -> String {
    match (pinned, request) {
        (Some(cpu), _) => format!(
            "worker pinned to cpu {} ({})",
            cpu.id,
            CoreKind::of(&cpu, cpus)
        ),
        (None, PinRequest::Auto) => {
            "worker not pinned (auto: no free core, or no CPU topology on this OS)".to_owned()
        }
        (None, _) => "worker not pinned".to_owned(),
    }
}

/// `--list-cpus` output: one row per logical CPU, then the `--pin auto` choice.
pub fn write_cpu_table(out: &mut impl Write, cpus: &[LogicalCpu]) -> std::io::Result<()> {
    writeln!(out, "cpu  core  class  kind")?;
    for cpu in cpus {
        writeln!(
            out,
            "{:>3}  {:>4}  {:>5}  {}",
            cpu.id.to_string(),
            cpu.core,
            cpu.efficiency_class,
            CoreKind::of(cpu, cpus)
        )?;
    }
    if let Some(choice) = pick_performance_cpu(cpus) {
        writeln!(
            out,
            "--pin auto would choose cpu {} (first worker)",
            choice.id
        )?;
    }
    Ok(())
}
