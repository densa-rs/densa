//! Win32 implementation: CPU Sets for topology, group affinity for pinning.

use crate::{CpuError, CpuId, LogicalCpu};
use std::mem::size_of;
use std::os::windows::io::AsRawHandle;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{GetLastError, HANDLE};
use windows_sys::Win32::System::Kernel::PROCESSOR_NUMBER;
use windows_sys::Win32::System::SystemInformation::{
    CpuSetInformation, GetSystemCpuSetInformation, GROUP_AFFINITY, SYSTEM_CPU_SET_INFORMATION,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessorNumberEx, GetCurrentThread, SetThreadGroupAffinity,
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
