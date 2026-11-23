use std::sync::atomic::{AtomicU64, Ordering};
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
};

static PRIORITY_FAILURES: AtomicU64 = AtomicU64::new(0);
static ECO_QOS_FAILURES: AtomicU64 = AtomicU64::new(0);
static CONFINED: AtomicU64 = AtomicU64::new(0);
static CONFINEMENT_CLEARED: AtomicU64 = AtomicU64::new(0);
static CONFINEMENT_FAILURES: AtomicU64 = AtomicU64::new(0);
static CONFINEMENT_CPU_SETS: AtomicU64 = AtomicU64::new(0);
static CONFINEMENT_AVAILABILITY: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(ConfinementAvailability::NotQueried as u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum ConfinementAvailability {
    NotQueried,
    Selected,
    TooFewCores,
    SeveralProcessorGroups,
    QueryFailed,
    AllocationFailed,
}

#[cfg(feature = "test-support")]
fn confinement_availability() -> &'static str {
    match CONFINEMENT_AVAILABILITY.load(Ordering::Relaxed) {
        code if code == ConfinementAvailability::NotQueried as u8 => "not queried",
        code if code == ConfinementAvailability::Selected as u8 => "selected",
        code if code == ConfinementAvailability::TooFewCores as u8 => "too few cores",
        code if code == ConfinementAvailability::SeveralProcessorGroups as u8 => {
            "several processor groups"
        }
        code if code == ConfinementAvailability::QueryFailed as u8 => "query failed",
        code if code == ConfinementAvailability::AllocationFailed as u8 => "allocation failed",
        _ => unreachable!("only confinement availability codes are stored"),
    }
}

fn background_cpu_sets() -> &'static [u32] {
    background_cpu_sets_with(|buffer, length, required| unsafe {
        windows_sys::Win32::System::SystemInformation::GetSystemCpuSetInformation(
            buffer.cast(),
            length,
            required,
            std::ptr::null_mut(),
            0,
        ) != 0
    })
}

fn background_cpu_sets_with(
    mut query: impl FnMut(*mut u8, u32, &mut u32) -> bool,
) -> &'static [u32] {
    use windows_sys::Win32::System::SystemInformation::{
        CpuSetInformation, SYSTEM_CPU_SET_INFORMATION,
    };
    static SETS: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();
    SETS.get_or_init(|| {
        let result = engine_allocation::advisory(|| {
            let mut length = 0u32;
            query(std::ptr::null_mut(), 0, &mut length);
            if length == 0 {
                return Err(ConfinementAvailability::QueryFailed);
            }
            let words = (length as usize).div_ceil(8);
            let mut buffer = engine_allocation::vec::<u64>(words)
                .map_err(|_| ConfinementAvailability::AllocationFailed)?;
            buffer.resize(words, 0);
            let read = query(buffer.as_mut_ptr().cast(), length, &mut length);
            if length == 0 || !read {
                return Err(ConfinementAvailability::QueryFailed);
            }
            let entry_size = size_of::<SYSTEM_CPU_SET_INFORMATION>();
            let mut sets = engine_allocation::vec(length as usize / entry_size)
                .map_err(|_| ConfinementAvailability::AllocationFailed)?;
            let bytes = buffer.as_ptr().cast::<u8>();
            let mut offset = 0usize;
            while offset + entry_size <= length as usize && sets.len() < sets.capacity() {
                let entry = unsafe { &*bytes.add(offset).cast::<SYSTEM_CPU_SET_INFORMATION>() };
                if (entry.Size as usize) < entry_size {
                    break;
                }
                if entry.Type == CpuSetInformation {
                    let set = unsafe { entry.Anonymous.CpuSet };
                    let class = unsafe { set.Anonymous2.SchedulingClass };
                    sets.push(CpuSet {
                        group: set.Group,
                        rank: (class, set.EfficiencyClass, std::cmp::Reverse(set.CoreIndex)),
                        id: set.Id,
                    });
                }
                offset += entry.Size as usize;
            }
            least_preferred_cores(&sets)
        });
        let (sets, availability) = match result {
            Ok(sets) => (sets, ConfinementAvailability::Selected),
            Err(reason) => (Vec::new(), reason),
        };
        CONFINEMENT_CPU_SETS.store(sets.len() as u64, Ordering::Relaxed);
        CONFINEMENT_AVAILABILITY.store(availability as u8, Ordering::Relaxed);
        sets
    })
}

#[derive(Clone, Copy)]
struct CpuSet {
    group: u16,
    rank: (u8, u8, std::cmp::Reverse<u8>),
    id: u32,
}

fn least_preferred_cores(sets: &[CpuSet]) -> Result<Vec<u32>, ConfinementAvailability> {
    if sets.iter().any(|set| set.group != sets[0].group) {
        return Err(ConfinementAvailability::SeveralProcessorGroups);
    }
    let mut cores = engine_allocation::vec(sets.len())
        .map_err(|_| ConfinementAvailability::AllocationFailed)?;
    cores.extend(sets.iter().map(|set| set.rank));
    cores.sort_unstable();
    cores.dedup();
    if cores.len() < 4 {
        return Err(ConfinementAvailability::TooFewCores);
    }
    cores.truncate(2);
    let mut ids = engine_allocation::vec(sets.len())
        .map_err(|_| ConfinementAvailability::AllocationFailed)?;
    ids.extend(
        sets.iter()
            .filter(|set| cores.contains(&set.rank))
            .map(|set| set.id),
    );
    Ok(ids)
}

pub const BACKGROUND_STACK_BYTES: usize = 16 * 1024 * 1024;

pub struct OwnPriority {
    boosts_disabled: bool,
    frame_pipeline: bool,
    lowest: Option<bool>,
    eco_qos: Option<bool>,
    confined: Option<bool>,
}

impl OwnPriority {
    pub const fn new() -> Self {
        Self {
            boosts_disabled: false,
            frame_pipeline: false,
            lowest: None,
            eco_qos: None,
            confined: None,
        }
    }

    /// # Safety
    /// Only the assigned collector worker may apply this session's priority policy to the current
    /// thread.
    pub unsafe fn frame_pipeline() -> Self {
        let mut priority = Self {
            frame_pipeline: true,
            ..Self::new()
        };
        // SAFETY: The owning worker adjusts only its own thread's priority.
        unsafe { priority.follow(false) };
        priority
    }

    /// # Safety
    /// Only the assigned collector worker may apply this session's priority policy to the current
    /// thread.
    pub unsafe fn follow_pipeline(
        &mut self,
        pipeline: &crate::frame_priority::Pipeline,
        role: crate::frame_priority::Role,
    ) {
        // SAFETY: The owning worker adjusts only its own thread's priority.
        unsafe { self.follow(false) };
        if !self.boosts_disabled
            || self.lowest != Some(false)
            || self.eco_qos != Some(false)
            || self.confined != Some(false)
        {
            crate::tick_gate::copy_only(false);
            return;
        }
        // SAFETY: Only the current session's designated thread registers this role.
        unsafe { pipeline.register(role) };
        pipeline.refresh();
        crate::tick_gate::copy_only(
            !matches!(role, crate::frame_priority::Role::Sizing)
                && pipeline.registered(role)
                && !pipeline.failed(),
        );
    }

    /// # Safety
    /// Only the assigned collector worker may apply this session's priority policy to the current
    /// thread.
    pub unsafe fn reconciler(&mut self) -> bool {
        // SAFETY: The owning worker adjusts only its own thread's priority.
        unsafe { self.follow(false) };
        if !self.boosts_disabled
            || self.lowest != Some(false)
            || self.eco_qos != Some(false)
            || self.confined != Some(false)
        {
            return false;
        }
        if unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) } == 0 {
            PRIORITY_FAILURES.fetch_add(1, Ordering::Relaxed);
            false
        } else {
            true
        }
    }

    fn should_lower(&self, backlogged: bool) -> bool {
        !self.frame_pipeline && !backlogged
    }

    /// # Safety
    /// Only the assigned collector worker may apply this session's priority policy to the current
    /// thread.
    pub unsafe fn follow(&mut self, backlogged: bool) {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadInformation, SetThreadPriority, SetThreadPriorityBoost,
            THREAD_POWER_THROTTLING_CURRENT_VERSION, THREAD_POWER_THROTTLING_EXECUTION_SPEED,
            THREAD_POWER_THROTTLING_STATE, THREAD_PRIORITY_LOWEST, THREAD_PRIORITY_NORMAL,
            ThreadPowerThrottling,
        };
        let lowest = self.should_lower(backlogged);
        if !self.boosts_disabled {
            if unsafe { SetThreadPriorityBoost(GetCurrentThread(), 1) } == 0 {
                PRIORITY_FAILURES.fetch_add(1, Ordering::Relaxed);
            } else {
                self.boosts_disabled = true;
            }
        }
        if self.lowest != Some(lowest) {
            let priority = if lowest {
                THREAD_PRIORITY_LOWEST
            } else {
                THREAD_PRIORITY_NORMAL
            };
            if unsafe { SetThreadPriority(GetCurrentThread(), priority) } == 0 {
                PRIORITY_FAILURES.fetch_add(1, Ordering::Relaxed);
            } else {
                self.lowest = Some(lowest);
            }
        }
        if self.eco_qos != Some(lowest) {
            let throttling = THREAD_POWER_THROTTLING_STATE {
                Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
                ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
                StateMask: if lowest {
                    THREAD_POWER_THROTTLING_EXECUTION_SPEED
                } else {
                    0
                },
            };
            if unsafe {
                SetThreadInformation(
                    GetCurrentThread(),
                    ThreadPowerThrottling,
                    (&throttling as *const THREAD_POWER_THROTTLING_STATE).cast(),
                    size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
                )
            } == 0
            {
                ECO_QOS_FAILURES.fetch_add(1, Ordering::Relaxed);
            } else {
                self.eco_qos = Some(lowest);
            }
        }
        let sets = if self.frame_pipeline {
            &[][..]
        } else {
            background_cpu_sets()
        };
        if (self.frame_pipeline || !sets.is_empty()) && self.confined != Some(lowest) {
            use windows_sys::Win32::System::Threading::SetThreadSelectedCpuSets;
            let (list, count) = if lowest {
                (sets.as_ptr(), sets.len() as u32)
            } else {
                (std::ptr::null(), 0)
            };
            if unsafe { SetThreadSelectedCpuSets(GetCurrentThread(), list, count) } == 0 {
                CONFINEMENT_FAILURES.fetch_add(1, Ordering::Relaxed);
            } else {
                if lowest {
                    CONFINED.fetch_add(1, Ordering::Relaxed);
                } else {
                    CONFINEMENT_CLEARED.fetch_add(1, Ordering::Relaxed);
                }
                self.confined = Some(lowest);
            }
        }
    }
}

#[cfg(feature = "test-support")]
/// # Safety
/// Run only in an isolated fixture that owns the collector affinity state and failure injection.
pub unsafe fn background_cpu_sets_failure_exercise(
    ordinal: usize,
    query_failure: Option<usize>,
) -> usize {
    use collector_test_support::{ATTEMPTS, DENIALS, NullGuard};
    use windows_sys::Win32::System::SystemInformation::{
        CpuSetInformation, SYSTEM_CPU_SET_INFORMATION, SYSTEM_CPU_SET_INFORMATION_0,
        SYSTEM_CPU_SET_INFORMATION_0_0,
    };
    assert_eq!(confinement_availability(), "not queried");
    let entries: [_; 8] = std::array::from_fn(|index| SYSTEM_CPU_SET_INFORMATION {
        Size: size_of::<SYSTEM_CPU_SET_INFORMATION>() as u32,
        Type: CpuSetInformation,
        Anonymous: SYSTEM_CPU_SET_INFORMATION_0 {
            CpuSet: SYSTEM_CPU_SET_INFORMATION_0_0 {
                Id: 256 + index as u32,
                CoreIndex: (index / 2) as u8,
                ..Default::default()
            },
        },
    });
    let mut calls = 0;
    let guard = NullGuard::new(ordinal);
    let sets = background_cpu_sets_with(|buffer, length, required| {
        calls += 1;
        if query_failure == Some(calls) {
            if buffer.is_null() {
                *required = 0;
            }
            return false;
        }
        *required = size_of_val(&entries) as u32;
        if buffer.is_null() {
            assert_eq!(length, 0);
            return false;
        }
        assert!(length >= *required);
        unsafe {
            std::ptr::copy_nonoverlapping(
                entries.as_ptr().cast::<u8>(),
                buffer,
                *required as usize,
            );
        }
        true
    });
    let attempts = ATTEMPTS.load(Ordering::Relaxed);
    let expected = if query_failure.is_some() {
        assert_eq!(DENIALS.load(Ordering::Relaxed), 0);
        "query failed"
    } else if ordinal == usize::MAX {
        assert_eq!(DENIALS.load(Ordering::Relaxed), 0);
        assert_eq!(sets, [260, 261, 262, 263]);
        "selected"
    } else {
        assert_eq!(attempts, ordinal);
        assert_eq!(DENIALS.load(Ordering::Relaxed), 1);
        "allocation failed"
    };
    assert_eq!(confinement_availability(), expected);
    assert_eq!(
        CONFINEMENT_CPU_SETS.load(Ordering::Relaxed),
        sets.len() as u64
    );
    assert_eq!(
        background_cpu_sets_with(|_, _, _| panic!("cached topology must not be retried")),
        sets
    );
    if expected != "selected" {
        assert!(sets.is_empty());
        let mut priority = OwnPriority::new();
        // SAFETY: The owning worker adjusts only its own thread's priority.
        unsafe { priority.follow(false) };
        assert!(priority.lowest == Some(true) || PRIORITY_FAILURES.load(Ordering::Relaxed) > 0);
        assert!(priority.eco_qos == Some(true) || ECO_QOS_FAILURES.load(Ordering::Relaxed) > 0);
        let priority_failures = PRIORITY_FAILURES.load(Ordering::Relaxed);
        let eco_qos_failures = ECO_QOS_FAILURES.load(Ordering::Relaxed);
        // SAFETY: The owning worker adjusts only its own thread's priority.
        unsafe { priority.follow(true) };
        assert!(
            priority.lowest == Some(false)
                || PRIORITY_FAILURES.load(Ordering::Relaxed) > priority_failures
        );
        assert!(
            priority.eco_qos == Some(false)
                || ECO_QOS_FAILURES.load(Ordering::Relaxed) > eco_qos_failures
        );
        assert!(priority.confined.is_none());
        assert_eq!(CONFINED.load(Ordering::Relaxed), 0);
        assert_eq!(CONFINEMENT_CLEARED.load(Ordering::Relaxed), 0);
        assert_eq!(CONFINEMENT_FAILURES.load(Ordering::Relaxed), 0);
    }
    assert_eq!(ATTEMPTS.load(Ordering::Relaxed), attempts);
    drop(guard);
    attempts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu_sets(cores: &[(u16, u8, u8, u8)], first: u32) -> Vec<CpuSet> {
        let mut id = first;
        let mut sets = Vec::new();
        for &(group, class, efficiency, core) in cores {
            for _ in 0..2 {
                let rank = (class, efficiency, std::cmp::Reverse(core));
                sets.push(CpuSet { group, rank, id });
                id += 1;
            }
        }
        sets
    }

    #[test]
    fn frame_pipeline_never_lowers_even_with_all_four_frames_free() {
        let pipeline = OwnPriority {
            frame_pipeline: true,
            ..OwnPriority::new()
        };
        let sizing = OwnPriority::new();
        for backlogged in [false, true, false] {
            assert!(!pipeline.should_lower(backlogged));
            assert_eq!(sizing.should_lower(backlogged), !backlogged);
        }
    }

    #[test]
    fn frame_pipeline_applies_normal_policy_before_waiting_and_keeps_it_between_jobs() {
        std::thread::spawn(|| {
            let mut priority =
            // SAFETY: This is the dedicated worker applying its own session's priority policy.
            unsafe { OwnPriority::frame_pipeline() };
            for backlogged in [false, true, false] {
                // SAFETY: The owning worker adjusts only its own thread's priority.
                unsafe { priority.follow(backlogged) };
                assert_eq!(priority.lowest, Some(false));
                assert_eq!(priority.eco_qos, Some(false));
                assert_eq!(priority.confined, Some(false));
            }
        })
        .join()
        .unwrap();
    }

    #[test]
    fn background_threads_take_the_two_least_preferred_physical_cores() {
        let sets = cpu_sets(
            &[
                (0, 6, 0, 0),
                (0, 2, 0, 2),
                (0, 0, 0, 4),
                (0, 0, 0, 6),
                (0, 0, 0, 8),
            ],
            256,
        );
        assert_eq!(least_preferred_cores(&sets).unwrap(), [262, 263, 264, 265]);
        let mut shuffled = sets.clone();
        shuffled.reverse();
        assert_eq!(
            least_preferred_cores(&shuffled).unwrap(),
            [265, 264, 263, 262]
        );
        let hybrid = cpu_sets(
            &[
                (0, 0, 1, 0),
                (0, 0, 1, 2),
                (0, 0, 0, 4),
                (0, 0, 0, 6),
                (0, 0, 1, 8),
            ],
            0,
        );
        assert_eq!(least_preferred_cores(&hybrid).unwrap(), [4, 5, 6, 7]);
        let reversed = cpu_sets(
            &[
                (0, 0, 0, 0),
                (0, 0, 0, 2),
                (0, 0, 1, 4),
                (0, 0, 1, 6),
                (0, 0, 1, 8),
            ],
            0,
        );
        assert_eq!(least_preferred_cores(&reversed).unwrap(), [0, 1, 2, 3]);
        let groups = cpu_sets(
            &[
                (0, 0, 0, 60),
                (0, 0, 0, 62),
                (1, 0, 0, 60),
                (1, 0, 0, 62),
                (1, 0, 0, 0),
            ],
            0,
        );
        assert_eq!(
            least_preferred_cores(&groups),
            Err(ConfinementAvailability::SeveralProcessorGroups)
        );
        assert_eq!(
            least_preferred_cores(&sets[..6]),
            Err(ConfinementAvailability::TooFewCores)
        );
        assert_eq!(
            least_preferred_cores(&[]),
            Err(ConfinementAvailability::TooFewCores)
        );
    }
}
