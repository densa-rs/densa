//! Win32 implementation: CPU Sets for topology, group affinity for pinning.

use crate::{CpuError, CpuId, LogicalCpu, ThreadPriority};
use std::mem::size_of;
use std::os::windows::io::AsRawHandle;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{GetLastError, HANDLE};
use windows_sys::Win32::System::Kernel::PROCESSOR_NUMBER;
use windows_sys::Win32::System::SystemInformation::{
    CpuSetInformation, GetSystemCpuSetInformation, GROUP_AFFINITY, SYSTEM_CPU_SET_INFORMATION,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessorNumberEx, GetCurrentThread, GetThreadPriority,
    SetThreadGroupAffinity, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
    THREAD_PRIORITY_HIGHEST, THREAD_PRIORITY_NORMAL, THREAD_PRIORITY_TIME_CRITICAL,
};

fn last_error(call: &'static str) -> CpuError {
    // SAFETY: GetLastError has no preconditions.
    let code = unsafe { GetLastError() };
    CpuError::Os { call, code }
}

pub(crate) fn logical_cpus() -> Result<Vec<LogicalCpu>, CpuError> {
    const CALL: &str = "GetSystemCpuSetInformation";
    // SAFETY: GetCurrentProcess returns a pseudo-handle; it cannot fail and needs no close.
    let process = unsafe { GetCurrentProcess() };

    let mut needed = 0u32;
    // SAFETY: a null buffer with length 0 is the documented way to query the required size;
    // `needed` is a valid out-pointer. The call "fails" with ERROR_INSUFFICIENT_BUFFER.
    unsafe { GetSystemCpuSetInformation(null_mut(), 0, &mut needed, process, 0) };
    if needed == 0 {
        return Err(last_error(CALL));
    }

    // u64 storage gives the 8-byte alignment SYSTEM_CPU_SET_INFORMATION requires.
    let mut storage = vec![0u64; (needed as usize).div_ceil(size_of::<u64>())];
    let mut returned = 0u32;
    // SAFETY: `storage` provides at least `needed` writable, 8-aligned bytes, and the
    // length passed is exactly `needed`; `returned` is a valid out-pointer.
    let ok = unsafe {
        GetSystemCpuSetInformation(
            storage.as_mut_ptr().cast(),
            needed,
            &mut returned,
            process,
            0,
        )
    };
    if ok == 0 {
        return Err(last_error(CALL));
    }

    // The buffer holds variable-size records; each states its own size. Treat sizes as
    // untrusted: stop on zero or on a record that would run past the returned length.
    let base = storage.as_ptr().cast::<u8>();
    let end = (returned as usize).min(storage.len() * size_of::<u64>());
    let mut cpus = Vec::new();
    let mut offset = 0usize;
    while offset + size_of::<SYSTEM_CPU_SET_INFORMATION>() <= end {
        // SAFETY: the full record lies within the `end` bytes the OS initialised in
        // `storage`; `read_unaligned` imposes no alignment requirement; the record is
        // plain integers (and a union of integers), so every bit pattern is valid.
        let record = unsafe {
            base.add(offset)
                .cast::<SYSTEM_CPU_SET_INFORMATION>()
                .read_unaligned()
        };
        if record.Size == 0 {
            break;
        }
        if record.Type == CpuSetInformation {
            // SAFETY: `Type == CpuSetInformation` identifies `CpuSet` as the active field.
            let set = unsafe { record.Anonymous.CpuSet };
            cpus.push(LogicalCpu {
                id: CpuId {
                    group: set.Group,
                    index: set.LogicalProcessorIndex,
                },
                core: set.CoreIndex,
                efficiency_class: set.EfficiencyClass,
            });
        }
        offset += record.Size as usize;
    }
    Ok(cpus)
}

fn pin_handle(thread: HANDLE, cpu: LogicalCpu) -> Result<(), CpuError> {
    let mask = 1usize
        .checked_shl(u32::from(cpu.id.index))
        .ok_or(CpuError::InvalidCpu(cpu.id))?;
    let affinity = GROUP_AFFINITY {
        Mask: mask,
        Group: cpu.id.group,
        Reserved: [0; 3],
    };
    // SAFETY: `thread` is a live thread handle (or the current-thread pseudo-handle) for
    // the duration of the call; `affinity` is fully initialised; the previous-affinity
    // out-pointer is documented as optional (null).
    let ok = unsafe { SetThreadGroupAffinity(thread, &affinity, null_mut()) };
    if ok == 0 {
        return Err(last_error("SetThreadGroupAffinity"));
    }
    Ok(())
}

pub(crate) fn pin_current_thread(cpu: LogicalCpu) -> Result<(), CpuError> {
    // SAFETY: GetCurrentThread returns a pseudo-handle; it cannot fail and needs no close.
    pin_handle(unsafe { GetCurrentThread() }, cpu)
}

pub(crate) fn pin_thread<T>(
    thread: &std::thread::JoinHandle<T>,
    cpu: LogicalCpu,
) -> Result<(), CpuError> {
    // The JoinHandle owns the thread handle and keeps it open until joined.
    pin_handle(thread.as_raw_handle(), cpu)
}

/// `GetThreadPriority`'s failure value (`THREAD_PRIORITY_ERROR_RETURN`, i.e. `MAXLONG`),
/// which windows-sys does not export.
const PRIORITY_ERROR_RETURN: i32 = i32::MAX;

fn to_os_priority(priority: ThreadPriority) -> i32 {
    match priority {
        ThreadPriority::Normal => THREAD_PRIORITY_NORMAL,
        ThreadPriority::AboveNormal => THREAD_PRIORITY_ABOVE_NORMAL,
        ThreadPriority::Highest => THREAD_PRIORITY_HIGHEST,
        ThreadPriority::TimeCritical => THREAD_PRIORITY_TIME_CRITICAL,
    }
}

fn from_os_priority(raw: i32) -> Option<ThreadPriority> {
    match raw {
        THREAD_PRIORITY_NORMAL => Some(ThreadPriority::Normal),
        THREAD_PRIORITY_ABOVE_NORMAL => Some(ThreadPriority::AboveNormal),
        THREAD_PRIORITY_HIGHEST => Some(ThreadPriority::Highest),
        THREAD_PRIORITY_TIME_CRITICAL => Some(ThreadPriority::TimeCritical),
        _ => None, // Error return, or a level this API does not model (e.g. below normal).
    }
}

fn set_priority(thread: HANDLE, priority: ThreadPriority) -> Result<(), CpuError> {
    // SAFETY: `thread` is a live thread handle (or the current-thread pseudo-handle) for
    // the duration of the call, and the priority is one of the documented constants.
    let ok = unsafe { SetThreadPriority(thread, to_os_priority(priority)) };
    if ok == 0 {
        return Err(last_error("SetThreadPriority"));
    }
    Ok(())
}

fn get_priority(thread: HANDLE) -> Option<ThreadPriority> {
    // SAFETY: `thread` is a live thread handle (or the current-thread pseudo-handle) for
    // the duration of the call.
    let raw = unsafe { GetThreadPriority(thread) };
    (raw != PRIORITY_ERROR_RETURN)
        .then_some(raw)
        .and_then(from_os_priority)
}

pub(crate) fn set_thread_priority<T>(
    thread: &std::thread::JoinHandle<T>,
    priority: ThreadPriority,
) -> Result<(), CpuError> {
    set_priority(thread.as_raw_handle(), priority)
}

pub(crate) fn set_current_thread_priority(priority: ThreadPriority) -> Result<(), CpuError> {
    // SAFETY: GetCurrentThread returns a pseudo-handle; it cannot fail and needs no close.
    set_priority(unsafe { GetCurrentThread() }, priority)
}

pub(crate) fn thread_priority<T>(thread: &std::thread::JoinHandle<T>) -> Option<ThreadPriority> {
    get_priority(thread.as_raw_handle())
}

pub(crate) fn current_thread_priority() -> Option<ThreadPriority> {
    // SAFETY: GetCurrentThread returns a pseudo-handle; it cannot fail and needs no close.
    get_priority(unsafe { GetCurrentThread() })
}

#[inline]
pub(crate) fn current_cpu() -> Option<CpuId> {
    let mut number = PROCESSOR_NUMBER {
        Group: 0,
        Number: 0,
        Reserved: 0,
    };
    // SAFETY: `number` is a valid out-pointer; the call has no failure mode.
    unsafe { GetCurrentProcessorNumberEx(&mut number) };
    Some(CpuId {
        group: number.Group,
        index: number.Number,
    })
}
