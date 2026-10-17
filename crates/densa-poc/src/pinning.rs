use densa_platform::{find_cpu, logical_cpus, pick_performance_cpu, CoreKind, LogicalCpu};
use std::error::Error;
use std::fmt;
use std::io::Write;

/// What `--pin` asked for, before it is checked against the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinRequest {
    None,
    /// Let `densa_platform::pick_performance_cpu` choose.
    Auto,
    /// Logical processor index in group 0.
    Index(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinError {
    NoSuchCpu(u8),
    NoCpus,
}

impl fmt::Display for PinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchCpu(index) => write!(f, "CPU {index} does not exist (see --list-cpus)"),
            Self::NoCpus => f.write_str("the OS reported no CPUs to pin to"),
        }
    }
}

impl std::error::Error for PinError {}

/// Turns a `--pin` request into a concrete CPU of this machine.
pub fn resolve_pin(
    request: PinRequest,
    cpus: &[LogicalCpu],
) -> Result<Option<LogicalCpu>, PinError> {
    match request {
        PinRequest::None => Ok(None),
        PinRequest::Auto => pick_performance_cpu(cpus).map(Some).ok_or(PinError::NoCpus),
        PinRequest::Index(index) => find_cpu(cpus, index)
            .map(Some)
            .ok_or(PinError::NoSuchCpu(index)),
    }
}

const UNPINNED: &str = "worker not pinned";

/// Resolves `--pin` against the live topology; also returns a human-readable note.
///
/// `auto` degrades to unpinned (with a warning on `out`) where the OS offers no affinity
/// control; an explicit CPU is a hard error.
pub(crate) fn resolve_worker_pin(
    request: PinRequest,
    out: &mut impl Write,
) -> Result<(Option<LogicalCpu>, String), Box<dyn Error>> {
    if request == PinRequest::None {
        return Ok((None, UNPINNED.to_owned()));
    }
    let cpus = match logical_cpus() {
        Ok(cpus) => cpus,
        Err(e) if request == PinRequest::Auto => {
            writeln!(
                out,
                "warning: cannot pin the worker ({e}); running unpinned"
            )?;
            return Ok((None, UNPINNED.to_owned()));
        }
        Err(e) => return Err(e.into()),
    };
    let pin = resolve_pin(request, &cpus)?;
    let note = pin.map_or_else(
        || UNPINNED.to_owned(),
        |cpu| {
            format!(
                "worker pinned to cpu {} ({})",
                cpu.id,
                CoreKind::of(&cpu, &cpus)
            )
        },
    );
    Ok((pin, note))
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
        writeln!(out, "--pin auto would choose cpu {}", choice.id)?;
    }
    Ok(())
}
