//! Platforms without an implementation yet: report "unsupported", never fail silently.

use crate::{CpuError, CpuId, LogicalCpu};

pub(crate) fn logical_cpus() -> Result<Vec<LogicalCpu>, CpuError> {
    Err(CpuError::Unsupported)
}

pub(crate) fn pin_current_thread(_cpu: LogicalCpu) -> Result<(), CpuError> {
    Err(CpuError::Unsupported)
}

pub(crate) fn pin_thread<T>(
    _thread: &std::thread::JoinHandle<T>,
    _cpu: LogicalCpu,
) -> Result<(), CpuError> {
    Err(CpuError::Unsupported)
}

pub(crate) fn current_cpu() -> Option<CpuId> {
    None
}
