use core::time::Duration;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::Kernel::PROCESSOR_NUMBER;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, GetProcessAffinityMask,
    GetThreadIdealProcessorEx, OpenThread, THREAD_QUERY_LIMITED_INFORMATION,
};

#[link(name = "kernel32")]
extern "system" {
    fn QueryThreadCycleTime(thread: HANDLE, cycles: *mut u64) -> i32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotThread {
    pub thread_id: u32,
    pub cycles: u64,
    pub ideal_processor: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadCensus {
    pub process_affinity_mask: usize,
    pub threads_sampled: usize,
    pub hottest: Vec<HotThread>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadCensusRefusal {
    ProcessAffinityUnreadable { last_error: u32 },
    SnapshotUnavailable { last_error: u32 },
    WalkBroken { last_error: u32 },
}

pub fn sample_other_threads(
    window: Duration,
    top: usize,
) -> Result<ThreadCensus, ThreadCensusRefusal> {
    // SAFETY: plain Win32 calls with documented arguments; every handle
    unsafe {
        let mut process_mask = 0usize;
        let mut system_mask = 0usize;
        if GetProcessAffinityMask(GetCurrentProcess(), &mut process_mask, &mut system_mask) == 0
            || process_mask == 0
        {
            return Err(ThreadCensusRefusal::ProcessAffinityUnreadable {
                last_error: GetLastError(),
            });
        }
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
            return Err(ThreadCensusRefusal::SnapshotUnavailable {
                last_error: GetLastError(),
            });
        }
        let own_pid = GetCurrentProcessId();
        let own_tid = GetCurrentThreadId();
        let mut entry: THREADENTRY32 = core::mem::zeroed();
        entry.dwSize = core::mem::size_of::<THREADENTRY32>() as u32;
        let mut handles: Vec<(u32, HANDLE)> = Vec::new();
        let mut stepped = Thread32First(snapshot, &mut entry);
        let walk = loop {
            if stepped == 0 {
                let last_error = GetLastError();
                break if last_error == ERROR_NO_MORE_FILES {
                    Ok(())
                } else {
                    Err(ThreadCensusRefusal::WalkBroken { last_error })
                };
            }
            if entry.th32OwnerProcessID == own_pid && entry.th32ThreadID != own_tid {
                let handle = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, entry.th32ThreadID);
                if !handle.is_null() {
                    handles.push((entry.th32ThreadID, handle));
                }
            }
            entry.dwSize = core::mem::size_of::<THREADENTRY32>() as u32;
            stepped = Thread32Next(snapshot, &mut entry);
        };
        CloseHandle(snapshot);
        if let Err(refusal) = walk {
            for &(_, handle) in &handles {
                CloseHandle(handle);
            }
            return Err(refusal);
        }

        let cycles_of = |handle: HANDLE| -> u64 {
            let mut value = 0u64;
            if QueryThreadCycleTime(handle, &mut value) != 0 {
                value
            } else {
                0
            }
        };
        let before: Vec<u64> = handles
            .iter()
            .map(|&(_, handle)| cycles_of(handle))
            .collect();
        std::thread::sleep(window);
        let mut sampled: Vec<HotThread> = handles
            .iter()
            .enumerate()
            .map(|(index, &(thread_id, handle))| {
                let mut ideal = PROCESSOR_NUMBER {
                    Group: 0,
                    Number: 0,
                    Reserved: 0,
                };
                let ideal_processor =
                    if GetThreadIdealProcessorEx(handle, &mut ideal) != 0 && ideal.Group == 0 {
                        Some(ideal.Number)
                    } else {
                        None
                    };
                HotThread {
                    thread_id,
                    cycles: cycles_of(handle).saturating_sub(before[index]),
                    ideal_processor,
                }
            })
            .collect();
        for &(_, handle) in &handles {
            CloseHandle(handle);
        }
        let threads_sampled = sampled.len();
        sampled
            .sort_unstable_by(|a, b| b.cycles.cmp(&a.cycles).then(a.thread_id.cmp(&b.thread_id)));
        sampled.truncate(top);
        Ok(ThreadCensus {
            process_affinity_mask: process_mask,
            threads_sampled,
            hottest: sampled,
        })
    }
}
