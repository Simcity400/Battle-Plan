//! Allocation-free status snapshots; unavailable OS profiling remains explicit.
use crate::status::{sequence, status};
use std::{
    cell::Cell,
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::Threading::{GetCurrentProcess, GetCurrentThread, GetThreadPriority},
};

// SDK layouts stay local so optional profiling does not widen every unified test dependency.
#[repr(C)]
#[derive(Default)]
struct DispatchData {
    size: u16,
    version: u8,
    counters: u8,
    context_switches: u32,
    wait_reasons: u64,
    cycles: u64,
    retries: u32,
    reserved: u32,
    hardware: [[u64; 2]; 16],
}
#[repr(C)]
#[derive(Default)]
struct MemoryCounters {
    size: u32,
    page_faults: u32,
    memory: [usize; 8],
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn EnableThreadProfiling(
        thread: HANDLE,
        flags: u32,
        counters: u64,
        profile: *mut HANDLE,
    ) -> u32;
    fn DisableThreadProfiling(profile: HANDLE) -> u32;
    fn ReadThreadProfilingData(profile: HANDLE, flags: u32, data: *mut DispatchData) -> u32;
    fn K32GetProcessMemoryInfo(process: HANDLE, counters: *mut MemoryCounters, size: u32) -> i32;
}

thread_local! { static PROFILE: Cell<HANDLE> = const { Cell::new(std::ptr::null_mut()) }; }
static ENABLED: AtomicU64 = AtomicU64::new(0);
static ENABLE_FAILURES: AtomicU64 = AtomicU64::new(0);
static ENABLE_ERROR: AtomicU32 = AtomicU32::new(0);
static READ_FAILURES: AtomicU64 = AtomicU64::new(0);
static FAULT_FAILURES: AtomicU64 = AtomicU64::new(0);
static READS: AtomicU64 = AtomicU64::new(0);
static READ_TICKS: AtomicU64 = AtomicU64::new(0);
static SERVICE: [[AtomicU64; 4]; 4] = [const { [const { AtomicU64::new(0) }; 4] }; 4];

pub(crate) fn record_service(begin: Snapshot, end: Snapshot, cycles: Option<u64>, ticks: i64) {
    let bucket = match (begin.priority, end.priority) {
        (Some(0), Some(0)) => 0,
        (Some(1), Some(1)) => 1,
        (Some(2), Some(2)) => 2,
        _ => 3,
    };
    SERVICE[bucket][0].fetch_add(1, Ordering::Relaxed);
    if let Some(cycles) = cycles {
        SERVICE[bucket][1].fetch_add(cycles, Ordering::Relaxed);
        SERVICE[bucket][3].fetch_add(1, Ordering::Relaxed);
    }
    SERVICE[bucket][2].fetch_add(ticks.max(0) as u64, Ordering::Relaxed);
}

pub(crate) struct Profile {
    handle: HANDLE,
    _same_thread: PhantomData<Rc<()>>,
}
impl Profile {
    pub(crate) fn start() -> Self {
        let mut handle = std::ptr::null_mut();
        // SAFETY: enables dispatch counters only on this collector worker, without hardware counters.
        let error = unsafe { EnableThreadProfiling(GetCurrentThread(), 1, 0, &mut handle) };
        if error == 0 {
            PROFILE.with(|slot| {
                assert!(slot.get().is_null());
                slot.set(handle);
            });
            ENABLED.fetch_add(1, Ordering::Relaxed);
        } else {
            handle = std::ptr::null_mut();
            ENABLE_FAILURES.fetch_add(1, Ordering::Relaxed);
            ENABLE_ERROR.store(error, Ordering::Relaxed);
        }
        Self {
            handle,
            _same_thread: PhantomData,
        }
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            PROFILE.with(|slot| slot.set(std::ptr::null_mut()));
            // SAFETY: the opaque profiling handle belongs to this thread and has not been disabled.
            unsafe { DisableThreadProfiling(self.handle) };
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Snapshot {
    pub(crate) context_switches: Option<u32>,
    pub(crate) wait_reasons: Option<u64>,
    pub(crate) process_faults: Option<u32>,
    pub(crate) priority: Option<i32>,
}
impl Snapshot {
    pub(crate) fn now(faults: bool) -> Self {
        let begin = crate::sampling::qpc();
        let mut snapshot = Self::default();
        PROFILE.with(|slot| {
            let handle = slot.get();
            if handle.is_null() {
                return;
            }
            let mut data = DispatchData {
                size: size_of::<DispatchData>() as u16,
                version: 1,
                ..Default::default()
            };
            // SAFETY: this worker owns the profiling handle and the initialized output buffer.
            if unsafe { ReadThreadProfilingData(handle, 1, &mut data) } == 0 {
                snapshot.context_switches = Some(data.context_switches);
                snapshot.wait_reasons = Some(data.wait_reasons);
            } else {
                READ_FAILURES.fetch_add(1, Ordering::Relaxed);
            }
        });
        if faults {
            let mut memory = MemoryCounters {
                size: size_of::<MemoryCounters>() as u32,
                ..Default::default()
            };
            // SAFETY: the current process pseudo-handle and this fixed-size output stay valid.
            if unsafe {
                K32GetProcessMemoryInfo(
                    GetCurrentProcess(),
                    &mut memory,
                    size_of::<MemoryCounters>() as u32,
                )
            } != 0
            {
                snapshot.process_faults = Some(memory.page_faults);
            } else {
                FAULT_FAILURES.fetch_add(1, Ordering::Relaxed);
            }
        }
        // SAFETY: the pseudo-handle names only this thread.
        let priority = unsafe { GetThreadPriority(GetCurrentThread()) };
        snapshot.priority = (priority != i32::MAX).then_some(priority);
        READS.fetch_add(1, Ordering::Relaxed);
        READ_TICKS.fetch_add(
            (crate::sampling::qpc() - begin).max(0) as u64,
            Ordering::Relaxed,
        );
        snapshot
    }
    pub(crate) fn switches_to(self, end: Self) -> Option<u32> {
        self.context_switches
            .zip(end.context_switches)
            .map(|(begin, end)| end.wrapping_sub(begin))
    }
    pub(crate) fn faults_to(self, end: Self) -> Option<u32> {
        self.process_faults
            .zip(end.process_faults)
            .map(|(begin, end)| end.wrapping_sub(begin))
    }
}

pub(crate) fn status(frequency: u64) -> impl serde::Serialize {
    status!({
        "service_by_boundary_priority": sequence(SERVICE.iter().zip(["normal", "above_normal", "highest", "mixed_or_unavailable"]).map(move |(cost, priority)| status!({
            "priority": priority,
            "spans": cost[0].load(Ordering::Relaxed),
            "cycle_samples": cost[3].load(Ordering::Relaxed),
            "cycles": (cost[3].load(Ordering::Relaxed) != 0).then(|| cost[1].load(Ordering::Relaxed)),
            "wall_ns": (u128::from(cost[2].load(Ordering::Relaxed)) * 1_000_000_000 / u128::from(frequency.max(1))) as u64,
        }))),
        "profiling_enabled": ENABLED.load(Ordering::Relaxed),
        "enable_failures": ENABLE_FAILURES.load(Ordering::Relaxed),
        "last_enable_error": (ENABLE_FAILURES.load(Ordering::Relaxed) != 0).then(|| ENABLE_ERROR.load(Ordering::Relaxed)),
        "read_failures": READ_FAILURES.load(Ordering::Relaxed),
        "process_fault_failures": FAULT_FAILURES.load(Ordering::Relaxed),
        "snapshot_calls": READS.load(Ordering::Relaxed),
        "snapshot_wall_ns": (u128::from(READ_TICKS.load(Ordering::Relaxed)) * 1_000_000_000 / u128::from(frequency.max(1))) as u64,
        "scope": "status-only plain-release diagnostics on collector workers: dispatch profiling is enabled once, with no hardware counters, and disabled before thread exit; unsupported or denied APIs are null, never zero; context_switches are per-thread modulo-u32 deltas, not preemption durations; wait_reason_bitmap is the OS KWAIT_REASON mask since the previous successful read, not a count or time; spool spans expose coverage and union masks; FIFO boundaries additionally sample whole-game-process PageFaultCount (soft and hard faults combined, including other game threads), so process_faults cannot attribute paging to decode or give I/O latency; priorities are numeric thread offsets within the unchanged game process class; service_by_boundary_priority partitions completed FIFO phases and non-idle spool/helper/sizing spans by their two observed priority offsets, not time actually spent at each priority (interior changes may be invisible); compare with collector_thread_cpu and frame_priority.refresh_cost to weigh interference with game work; snapshot_wall_ns measures query overhead including descheduling; ETW ready-thread/context-switch, hard-fault and disk events are still needed for precise off-CPU causality",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deltas_preserve_unavailable_values_and_counter_wraps() {
        let begin = Snapshot {
            context_switches: Some(u32::MAX - 1),
            process_faults: Some(10),
            ..Default::default()
        };
        let end = Snapshot {
            context_switches: Some(2),
            process_faults: Some(15),
            ..Default::default()
        };
        assert_eq!(begin.switches_to(end), Some(4));
        assert_eq!(begin.faults_to(end), Some(5));
        assert_eq!(begin.switches_to(Snapshot::default()), None);
        assert_eq!(Snapshot::default().faults_to(end), None);
    }
    #[test]
    fn a_snapshot_without_a_profiler_allocates_nothing_and_does_not_invent_switches() {
        crate::test_support::with_allocations_denied(|| {
            let snapshot = Snapshot::now(true);
            assert_eq!(snapshot.context_switches, None);
            assert_eq!(snapshot.wait_reasons, None);
        });
    }
}
