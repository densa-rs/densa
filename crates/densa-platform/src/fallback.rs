//! Platforms without an implementation yet: report "unsupported", never fail silently.

use crate::{CpuError, CpuId, LogicalCpu, ThreadPriority};

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

pub(crate) fn set_thread_priority<T>(
    _thread: &std::thread::JoinHandle<T>,
    _priority: ThreadPriority,
) -> Result<(), CpuError> {
    Err(CpuError::Unsupported)
}

pub(crate) fn set_current_thread_priority(_priority: ThreadPriority) -> Result<(), CpuError> {
    Err(CpuError::Unsupported)
}

pub(crate) fn thread_priority<T>(_thread: &std::thread::JoinHandle<T>) -> Option<ThreadPriority> {
    None
}

pub(crate) fn current_thread_priority() -> Option<ThreadPriority> {
    None
}
