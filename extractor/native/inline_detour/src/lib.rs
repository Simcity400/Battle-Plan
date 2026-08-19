//! still patched in is not a released hook; restoring the target is a separate

#[cfg(windows)]
mod freeze;

#[cfg(windows)]
pub mod thread_census;

#[cfg(windows)]
pub mod freeze_census;

#[cfg(windows)]
pub mod release_ledger;

#[cfg(all(test, windows))]
mod fault_tests;

#[cfg(all(test, windows))]
#[global_allocator]
static ALLOCATION_PROBE: freeze_window_audit::AllocationProbe =
    freeze_window_audit::AllocationProbe;

#[cfg(windows)]
mod windows_impl {
    use super::freeze::{freeze_other_threads, FreezeRefusal};
    use crate::freeze_census::FreezeEntryPoint;
    use crate::release_ledger::ReleaseOperation;
    use core::ffi::c_void;
    use minhook::{MinHook, MinHookLock, MH_STATUS};
    use std::fmt;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum InlineDetourEnableError {
        ModulePinRefused(u32),
        FreezeRefused(FreezeRefusal),
        MinHook(MH_STATUS),
        BatchApplyRefused {
            status: MH_STATUS,
            still_live: bool,
            rolled_back: bool,
        },
    }

    impl fmt::Display for InlineDetourEnableError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::ModulePinRefused(code) => write!(
                    f,
                    "inline detour refused: containing module could not be pinned \
                     against unload (GetLastError {code})"
                ),
                Self::FreezeRefused(refusal) => write!(f, "inline detour {refusal}"),
                Self::MinHook(status) => write!(f, "{status}"),
                Self::BatchApplyRefused {
                    status,
                    still_live: false,
                    rolled_back: true,
                } => write!(
                    f,
                    "inline detour batch refused: the batch apply did not complete \
                     ({status}), and it names neither the target it stopped at nor \
                     whether that target's bytes were written before it did; the \
                     batch was rolled back and this target is not live"
                ),
                Self::BatchApplyRefused {
                    status,
                    still_live: false,
                    rolled_back: false,
                } => write!(
                    f,
                    "inline detour batch refused: the batch apply did not complete \
                     ({status}), and it names neither the target it stopped at nor \
                     whether that target's bytes were written before it did; this \
                     target is not live, but the rollback did not complete -- a \
                     target whose restore was written and not published stays \
                     unremovable until an enable and a disable complete over it"
                ),
                Self::BatchApplyRefused {
                    status,
                    still_live: true,
                    ..
                } => write!(
                    f,
                    "inline detour batch refused: the batch apply did not complete \
                     ({status}), and it names neither the target it stopped at nor \
                     whether that target's bytes were written before it did; the \
                     rollback did not take this target back down and it is STILL \
                     LIVE"
                ),
            }
        }
    }

    impl std::error::Error for InlineDetourEnableError {}

    impl From<MH_STATUS> for InlineDetourEnableError {
        fn from(status: MH_STATUS) -> Self {
            Self::MinHook(status)
        }
    }

    impl From<FreezeRefusal> for InlineDetourEnableError {
        fn from(refusal: FreezeRefusal) -> Self {
            Self::FreezeRefused(refusal)
        }
    }

    impl InlineDetourEnableError {
        pub fn minhook_status(self) -> Option<MH_STATUS> {
            match self {
                Self::MinHook(status) | Self::BatchApplyRefused { status, .. } => Some(status),
                Self::ModulePinRefused(_) | Self::FreezeRefused(_) => None,
            }
        }

        pub fn freeze_refusal(self) -> Option<FreezeRefusal> {
            match self {
                Self::FreezeRefused(refusal) => Some(refusal),
                Self::ModulePinRefused(_) | Self::MinHook(_) | Self::BatchApplyRefused { .. } => {
                    None
                }
            }
        }

        fn with_rollback_facts(self, live: bool, undone: bool) -> Self {
            match self {
                Self::BatchApplyRefused { status, .. } => Self::BatchApplyRefused {
                    status,
                    still_live: live,
                    rolled_back: undone,
                },
                other => other,
            }
        }
    }

    /// Why an inline detour was not released.
    /// release leaves the detour INSTALLED AND LIVE. That is the safe
    /// still runs real code -- but the owning unit is no longer released and
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum InlineDetourReleaseError {
        FreezeRefused(FreezeRefusal),
        MinHook(MH_STATUS),
        StillEnabled,
    }

    impl fmt::Display for InlineDetourReleaseError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::FreezeRefused(refusal) => {
                    write!(f, "inline detour retained rather than restored: {refusal}")
                }
                Self::MinHook(status) => write!(f, "{status}"),
                Self::StillEnabled => write!(
                    f,
                    "inline detour retained rather than removed: the target is \
                     still enabled, so its detour body cannot have been drained \
                     and freeing the trampoline would strand a callback in it"
                ),
            }
        }
    }

    impl std::error::Error for InlineDetourReleaseError {}

    impl InlineDetourReleaseError {
        pub fn minhook_status(self) -> Option<MH_STATUS> {
            match self {
                Self::MinHook(status) => Some(status),
                Self::FreezeRefused(_) | Self::StillEnabled => None,
            }
        }

        /// treat a release as idempotent stop here.
        pub fn already_released(self) -> bool {
            matches!(
                self,
                Self::MinHook(MH_STATUS::MH_ERROR_DISABLED)
                    | Self::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)
            )
        }

        /// could not be frozen. The owning unit is NOT released.
        pub fn detour_retained(self) -> bool {
            matches!(self, Self::FreezeRefused(_) | Self::StillEnabled)
        }
    }

    static MODULE_PINNED: AtomicBool = AtomicBool::new(false);
    static ENABLED_DETOURS: AtomicU64 = AtomicU64::new(0);
    static RETAINED_DETOURS: AtomicU64 = AtomicU64::new(0);

    #[inline(never)]
    extern "system" fn residency_anchor() {}

    fn pin_containing_module() -> Result<(), u32> {
        if MODULE_PINNED.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut module = ptr::null_mut();
        let pinned = unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
                residency_anchor as *const () as *const u16,
                &mut module,
            )
        };
        if pinned == 0 || module.is_null() {
            return Err(unsafe { GetLastError() });
        }
        MODULE_PINNED.store(true, Ordering::Release);
        Ok(())
    }

    pub(crate) fn run_pinned<T, E>(
        pin: impl FnOnce() -> Result<(), u32>,
        op: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, InlineDetourEnableError>
    where
        E: Into<InlineDetourEnableError>,
    {
        pin().map_err(InlineDetourEnableError::ModulePinRefused)?;
        op().map_err(Into::into)
    }

    pub unsafe fn enable(target: *mut c_void) -> Result<(), InlineDetourEnableError> {
        run_pinned(pin_containing_module, || {
            let mut frozen = freeze_other_threads()?;
            frozen.note_entry(FreezeEntryPoint::Enable, "", 1);
            let outcome = match frozen.refuse_if_a_thread_is_in(core::iter::once(target), false) {
                Ok(()) => frozen
                    .time_minhook(|lock| unsafe { MinHook::enable_hook(lock, target) })
                    .map_err(InlineDetourEnableError::MinHook),
                Err(refusal) => Err(InlineDetourEnableError::FreezeRefused(refusal)),
            };
            drop(frozen);
            outcome
        })?;
        ENABLED_DETOURS.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    pub fn pin_module_now() -> Result<(), InlineDetourEnableError> {
        pin_containing_module().map_err(InlineDetourEnableError::ModulePinRefused)
    }

    pub unsafe fn queue_enable(target: *mut c_void) -> Result<(), InlineDetourEnableError> {
        run_pinned(pin_containing_module, || {
            let lock = MinHookLock::acquire()?;
            unsafe { MinHook::queue_enable_hook(&lock, target) }
        })
    }

    pub unsafe fn apply_queued() -> Result<(), InlineDetourEnableError> {
        apply_queued_without_counting()?;
        ENABLED_DETOURS.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn apply_queued_without_counting() -> Result<(), InlineDetourEnableError> {
        run_pinned(pin_containing_module, || {
            let mut frozen = freeze_other_threads()?;
            frozen.note_entry(FreezeEntryPoint::ApplyQueued, "", 0);
            let outcome = match frozen.refuse_if_a_thread_is_in_a_queued_patch() {
                Ok(()) => frozen
                    .time_minhook(|lock| unsafe { MinHook::apply_queued(lock) })
                    .map_err(InlineDetourEnableError::MinHook),
                Err(refusal) => Err(InlineDetourEnableError::FreezeRefused(refusal)),
            };
            drop(frozen);
            outcome
        })
    }

    pub type BatchOutcome<E> = (&'static str, Result<(), E>);

    #[derive(Debug)]
    pub struct BatchEnableReport {
        outcomes: Vec<BatchOutcome<InlineDetourEnableError>>,
        apply_refusal: Option<InlineDetourEnableError>,
        live: Vec<bool>,
        rolled_back: bool,
        freezes: u64,
    }

    impl BatchEnableReport {
        pub fn rolled_back(&self) -> bool {
            self.rolled_back
        }

        pub fn refusal(&self) -> Option<(&'static str, InlineDetourEnableError)> {
            if let Some(cause @ InlineDetourEnableError::BatchApplyRefused { still_live, .. }) =
                self.apply_refusal
            {
                let named = still_live
                    .then(|| {
                        self.outcomes
                            .iter()
                            .zip(self.live.iter())
                            .find(|(_, live)| **live)
                            .map(|((name, _), _)| *name)
                    })
                    .flatten()
                    .or_else(|| {
                        self.outcomes.iter().find_map(|(name, result)| {
                            matches!(
                                result,
                                Err(InlineDetourEnableError::BatchApplyRefused { .. })
                            )
                            .then_some(*name)
                        })
                    })
                    .or_else(|| self.outcomes.first().map(|(name, _)| *name));
                if let Some(name) = named {
                    return Some((name, cause));
                }
            }
            self.outcomes
                .iter()
                .find_map(|(name, result)| result.err().map(|error| (*name, error)))
        }

        pub fn all_succeeded(&self) -> bool {
            self.refusal().is_none()
        }

        pub fn still_live(&self) -> &[bool] {
            &self.live
        }

        fn sealed(
            mut outcomes: Vec<BatchOutcome<InlineDetourEnableError>>,
            apply_refusal: Option<InlineDetourEnableError>,
            live_now: &[bool],
            rolled_back: bool,
            freezes: u64,
        ) -> Self {
            let live: Vec<bool> = (0..outcomes.len())
                .map(|index| live_now.get(index).copied().unwrap_or(false))
                .collect();
            if let Some(refusal) = apply_refusal {
                for (index, (_, result)) in outcomes.iter_mut().enumerate() {
                    if result.is_ok() {
                        *result = Err(refusal.with_rollback_facts(live[index], rolled_back));
                    }
                }
            }
            let apply_refusal = apply_refusal
                .map(|refusal| refusal.with_rollback_facts(live.contains(&true), rolled_back));
            Self {
                outcomes,
                apply_refusal,
                live,
                rolled_back,
                freezes,
            }
        }
    }

    /// What a batched release did, target by target. The release twin of
    #[derive(Debug)]
    pub struct BatchReleaseReport {
        outcomes: Vec<BatchOutcome<InlineDetourReleaseError>>,
        apply_refusal: Option<InlineDetourReleaseError>,
        already_released: Vec<&'static str>,
        incoherent_cleared: Vec<&'static str>,
        incoherent_retained: Vec<&'static str>,
        live_at_entry: Vec<bool>,
        live_at_exit: Vec<bool>,
        freezes: u64,
    }

    impl BatchReleaseReport {
        /// The targets MinHook reported as ALREADY released -- disabled
        /// Their outcome is `Ok(())`, because the postcondition a release asks
        pub fn already_released(&self) -> &[&'static str] {
            &self.already_released
        }

        pub fn incoherent_cleared(&self) -> &[&'static str] {
            &self.incoherent_cleared
        }

        pub fn incoherent_retained(&self) -> &[&'static str] {
            &self.incoherent_retained
        }

        pub fn live_at_entry(&self) -> &[bool] {
            &self.live_at_entry
        }

        pub fn was_taken_down(&self, index: usize) -> bool {
            self.live_at_entry.get(index).copied().unwrap_or(false)
                && !self.live_at_exit.get(index).copied().unwrap_or(false)
        }

        pub fn live_at_exit(&self) -> &[bool] {
            &self.live_at_exit
        }

        /// Was this target one MinHook reported as already released?
        pub fn was_already_released(&self, name: &str) -> bool {
            self.already_released.iter().any(|entry| *entry == name)
        }

        /// A release batch keeps this spelling where [`BatchEnableReport`] has
        /// cosmetic: a release does NOT undo itself, so a partial teardown is a
        pub fn first_failure(&self) -> Option<(&'static str, InlineDetourReleaseError)> {
            self.outcomes
                .iter()
                .find_map(|(name, result)| result.err().map(|error| (*name, error)))
        }

        /// True when every target in the batch was released AND the batch
        /// -- which is the postcondition a release asks for -- while
        /// See `runs/lt_apply_refusal_rollback/FINDINGS.txt`, "the release
        pub fn all_succeeded(&self) -> bool {
            self.apply_refusal.is_none() && self.outcomes.iter().all(|(_, result)| result.is_ok())
        }
    }

    macro_rules! batch_report_accessors {
        ($report:ty, $error:ty) => {
            impl $report {
                pub fn apply_refusal(&self) -> Option<$error> {
                    self.apply_refusal
                }

                pub fn outcomes(&self) -> &[BatchOutcome<$error>] {
                    &self.outcomes
                }

                pub fn freezes(&self) -> u64 {
                    self.freezes
                }

                pub fn failures(&self) -> Vec<(&'static str, $error)> {
                    self.outcomes
                        .iter()
                        .filter_map(|(name, result)| result.err().map(|error| (*name, error)))
                        .collect()
                }

                pub fn len(&self) -> usize {
                    self.outcomes.len()
                }

                pub fn is_empty(&self) -> bool {
                    self.outcomes.is_empty()
                }
            }
        };
    }

    batch_report_accessors!(BatchEnableReport, InlineDetourEnableError);
    batch_report_accessors!(BatchReleaseReport, InlineDetourReleaseError);

    /// timed from freeze acquisition to release): `MinHook::apply_queued` is
    /// freeze was OBSERVED at 1.85-2.13 ms IN THE SHIPPED RELEASE BUILD and
    const PARKED_RETRIES: u32 = 3;

    /// game is running throughout. It exists because a retry that reacquires
    const PARKED_RETRY_YIELDS: u32 = 64;

    fn refused_because_a_thread_was_parked(refusal: &Option<InlineDetourEnableError>) -> bool {
        matches!(
            refusal,
            Some(InlineDetourEnableError::FreezeRefused(
                super::freeze::FreezeRefusal::TargetExecuting { .. }
            ))
        )
    }

    pub unsafe fn enable_batch(targets: &[(&'static str, *mut c_void)]) -> BatchEnableReport {
        let mut attempts = PARKED_RETRIES;
        let mut freezes = 0u64;
        loop {
            let mut report = enable_batch_once(targets);
            attempts -= 1;
            freezes = freezes.saturating_add(report.freezes);
            report.freezes = freezes;
            if attempts == 0 || !refused_because_a_thread_was_parked(&report.apply_refusal) {
                return report;
            }
            // thread can reacquire both locks and suspend everything again
            for _ in 0..PARKED_RETRY_YIELDS {
                std::thread::yield_now();
            }
        }
    }

    fn enable_batch_once(targets: &[(&'static str, *mut c_void)]) -> BatchEnableReport {
        let freezes_before = super::freeze::freeze_attempts_on_this_thread();
        let mut outcomes: Vec<BatchOutcome<InlineDetourEnableError>> =
            Vec::with_capacity(targets.len());
        if targets.is_empty() {
            return BatchEnableReport::sealed(outcomes, None, &[], false, 0);
        }
        if first_duplicate_target(targets).is_some() {
            for &(name, _) in targets {
                outcomes.push((
                    name,
                    Err(InlineDetourEnableError::MinHook(
                        MH_STATUS::MH_ERROR_ALREADY_CREATED,
                    )),
                ));
            }
            return BatchEnableReport::sealed(outcomes, None, &[], false, 0);
        }
        if let Err(pin) = pin_module_now() {
            for &(name, _) in targets {
                outcomes.push((name, Err(pin)));
            }
            return BatchEnableReport::sealed(outcomes, Some(pin), &[], false, 0);
        }
        // thread holding the allocator's lock makes that a permanent deadlock.
        let mut statuses: Vec<Result<(), MH_STATUS>> = Vec::with_capacity(targets.len());
        let mut rollback_failed: Vec<bool> = Vec::with_capacity(targets.len());
        let mut queued_ok: Vec<bool> = Vec::with_capacity(targets.len());
        let mut live_now: Vec<bool> = Vec::with_capacity(targets.len());
        let mut apply_refusal = None;
        let mut parked_refusal = None;
        let rolled_back;
        let mut attempted_rollback = false;
        match freeze_other_threads() {
            Ok(mut frozen) => {
                frozen.note_entry(
                    FreezeEntryPoint::EnableBatch,
                    targets[0].0,
                    targets.len() as u32,
                );
                for &(_, target) in targets {
                    statuses
                        .push(unsafe { MinHook::queue_enable_hook(frozen.minhook_lock(), target) });
                }
                for status in statuses.iter() {
                    queued_ok.push(status.is_ok());
                }
                let mut queued_any = queued_ok.iter().any(|queued| *queued);
                if queued_any {
                    if let Err(refusal) = frozen.refuse_if_a_thread_is_in_a_queued_patch() {
                        parked_refusal = Some(refusal);
                        apply_refusal = Some(InlineDetourEnableError::FreezeRefused(refusal));
                        queued_any = false;
                    }
                }
                if queued_any {
                    if let Err(apply) =
                        frozen.time_minhook(|lock| unsafe { MinHook::apply_queued(lock) })
                    {
                        apply_refusal = Some(InlineDetourEnableError::BatchApplyRefused {
                            status: apply,
                            still_live: false,
                            rolled_back: false,
                        });
                        for (index, &(_, target)) in targets.iter().enumerate() {
                            if statuses[index].is_ok() {
                                statuses[index] = match unsafe {
                                    MinHook::enable_hook(frozen.minhook_lock(), target)
                                } {
                                    Err(MH_STATUS::MH_ERROR_ENABLED) => Ok(()),
                                    other => other,
                                };
                            }
                        }
                    }
                }
                let batch_refused =
                    statuses.iter().any(|status| status.is_err()) || apply_refusal.is_some();
                for (index, &(_, target)) in targets.iter().enumerate() {
                    let undo = queued_ok[index] && queued_any && batch_refused;
                    let mut failed_undo = false;
                    if undo {
                        attempted_rollback = true;
                        failed_undo = !matches!(
                            unsafe { MinHook::disable_hook(frozen.minhook_lock(), target) },
                            Ok(())
                                | Err(MH_STATUS::MH_ERROR_DISABLED)
                                | Err(MH_STATUS::MH_ERROR_NOT_CREATED)
                        );
                    }
                    rollback_failed.push(failed_undo);
                    let enabled_now = if undo && failed_undo {
                        matches!(
                            MinHook::hook_is_enabled(frozen.minhook_lock(), target),
                            Ok(true)
                        )
                    } else {
                        queued_ok[index] && queued_any && !undo
                    };
                    live_now.push(enabled_now);
                    if !enabled_now {
                        let _ =
                            unsafe { MinHook::queue_disable_hook(frozen.minhook_lock(), target) };
                    }
                }
                rolled_back = attempted_rollback && !rollback_failed.iter().any(|failed| *failed);
                drop(frozen);
            }
            Err(refusal) => {
                let refused = InlineDetourEnableError::FreezeRefused(refusal);
                apply_refusal = Some(refused);
                for &(name, _) in targets {
                    outcomes.push((name, Err(refused)));
                }
                return BatchEnableReport::sealed(
                    outcomes,
                    apply_refusal,
                    &[],
                    false,
                    super::freeze::freeze_attempts_on_this_thread().saturating_sub(freezes_before),
                );
            }
        }
        let mut enabled = 0u64;
        for (index, &(name, _)) in targets.iter().enumerate() {
            let outcome = match parked_refusal {
                Some(refusal) => Err(InlineDetourEnableError::FreezeRefused(refusal)),
                None => statuses[index].map_err(InlineDetourEnableError::MinHook),
            };
            if live_now[index] {
                enabled += 1;
            }
            outcomes.push((name, outcome));
        }
        ENABLED_DETOURS.fetch_add(enabled, Ordering::AcqRel);
        BatchEnableReport::sealed(
            outcomes,
            apply_refusal,
            &live_now,
            rolled_back,
            super::freeze::freeze_attempts_on_this_thread().saturating_sub(freezes_before),
        )
    }

    fn first_duplicate_target(targets: &[(&'static str, *mut c_void)]) -> Option<usize> {
        for (index, &(_, target)) in targets.iter().enumerate() {
            if targets[..index]
                .iter()
                .any(|&(_, earlier)| earlier == target)
            {
                return Some(index);
            }
        }
        None
    }

    /// Release every target under ONE process freeze, reporting each one
    /// separately. The release twin of [`enable_batch`]; see it for what is
    /// detour the caller asked to release would fight the teardown, and the
    /// SUCCESS and named in `already_released`: nothing is patched, which is
    /// the postcondition a release asks for, and it is what every per-target
    pub unsafe fn disable_batch(targets: &[(&'static str, *mut c_void)]) -> BatchReleaseReport {
        let freezes_before = super::freeze::freeze_attempts_on_this_thread();
        let mut outcomes: Vec<BatchOutcome<InlineDetourReleaseError>> =
            Vec::with_capacity(targets.len());
        let mut already_released: Vec<&'static str> = Vec::new();
        if targets.is_empty() {
            return BatchReleaseReport {
                outcomes,
                apply_refusal: None,
                already_released,
                incoherent_cleared: Vec::new(),
                incoherent_retained: Vec::new(),
                live_at_entry: Vec::new(),
                live_at_exit: Vec::new(),
                freezes: 0,
            };
        }
        for &(name, target) in targets {
            crate::release_ledger::name_release_target(target as usize, name);
        }
        let mut statuses: Vec<Result<(), MH_STATUS>> = Vec::with_capacity(targets.len());
        let mut live_at_entry: Vec<bool> = Vec::with_capacity(targets.len());
        // deadlocks the teardown for good if one of them holds the allocator
        let mut live_at_exit: Vec<bool> = Vec::with_capacity(targets.len());
        let mut incoherent_at_entry: Vec<usize> = Vec::with_capacity(targets.len());
        let mut unhealed_incoherent: Vec<usize> = Vec::with_capacity(targets.len());
        let mut heal_failures: Vec<(usize, MH_STATUS)> = Vec::with_capacity(targets.len());
        let mut apply_refusal = None;
        // Set when a held thread was stopped inside bytes this release would
        let mut parked_refusal = None;
        let mut frozen = match freeze_other_threads() {
            Ok(frozen) => frozen,
            Err(refusal) => {
                let retained = InlineDetourReleaseError::FreezeRefused(refusal);
                RETAINED_DETOURS.fetch_add(targets.len() as u64, Ordering::AcqRel);
                for &(name, target) in targets {
                    crate::release_ledger::record_release_attempt(
                        target as usize,
                        ReleaseOperation::Disable,
                        Err(retained),
                    );
                    outcomes.push((name, Err(retained)));
                }
                return BatchReleaseReport {
                    outcomes,
                    apply_refusal: Some(retained),
                    already_released,
                    incoherent_cleared: Vec::new(),
                    incoherent_retained: Vec::new(),
                    live_at_entry: vec![true; targets.len()],
                    live_at_exit: vec![true; targets.len()],
                    freezes: super::freeze::freeze_attempts_on_this_thread()
                        .saturating_sub(freezes_before),
                };
            }
        };
        frozen.note_entry(
            FreezeEntryPoint::DisableBatch,
            targets[0].0,
            targets.len() as u32,
        );
        for &(_, target) in targets {
            live_at_entry.push(matches!(
                MinHook::hook_is_enabled(frozen.minhook_lock(), target),
                Ok(true)
            ));
        }
        for &(_, target) in targets {
            statuses.push(unsafe { MinHook::queue_disable_hook(frozen.minhook_lock(), target) });
        }
        let mut queued_any = statuses.iter().any(|status| status.is_ok());
        if queued_any {
            if let Err(refusal) = frozen.refuse_if_a_thread_is_in_a_queued_patch() {
                parked_refusal = Some(refusal);
                apply_refusal = Some(InlineDetourReleaseError::FreezeRefused(refusal));
                queued_any = false;
            }
        }
        if queued_any {
            if let Err(apply) = frozen.time_minhook(|lock| unsafe { MinHook::apply_queued(lock) }) {
                apply_refusal = Some(InlineDetourReleaseError::MinHook(apply));
                // whatever the apply released after it.
                // so it can release a later site while an earlier one is still
                let mut refused_at = None;
                for (index, &(_, target)) in targets.iter().enumerate() {
                    if !statuses[index].is_ok() {
                        continue;
                    }
                    if refused_at.is_some() {
                        // know about. Its release postcondition already holds
                        // already-released it is.
                        if !live_at_entry[index] {
                            statuses[index] = Err(MH_STATUS::MH_ERROR_DISABLED);
                            continue;
                        }
                        let restored =
                            unsafe { MinHook::enable_hook(frozen.minhook_lock(), target) };
                        statuses[index] = match restored {
                            Ok(()) | Err(MH_STATUS::MH_ERROR_ENABLED) => Err(apply),
                            Err(status) => Err(status),
                        };
                        continue;
                    }
                    statuses[index] =
                        unsafe { MinHook::disable_hook(frozen.minhook_lock(), target) };
                    if let Err(status) = statuses[index] {
                        if !InlineDetourReleaseError::MinHook(status).already_released() {
                            refused_at = Some(index);
                        }
                    }
                }
            }
        }
        // "still enabled" off the target's status: `Ok` or an already-released
        // release already holds -- which is why it is here and not in a caller.
        for (index, &(_, target)) in targets.iter().enumerate() {
            if !matches!(
                MinHook::hook_is_incoherent(frozen.minhook_lock(), target),
                Ok(true)
            ) {
                continue;
            }
            incoherent_at_entry.push(index);
            let reinstated = unsafe { MinHook::enable_hook(frozen.minhook_lock(), target) };
            let outcome = match reinstated {
                Ok(()) => unsafe { MinHook::disable_hook(frozen.minhook_lock(), target) },
                Err(status) => Err(status),
            };
            if let Err(status) = outcome {
                heal_failures.push((index, status));
            }
            if matches!(
                MinHook::hook_is_incoherent(frozen.minhook_lock(), target),
                Ok(true) | Err(_)
            ) {
                unhealed_incoherent.push(index);
            }
        }
        for &(_, target) in targets.iter() {
            match MinHook::hook_is_enabled(frozen.minhook_lock(), target) {
                Ok(true) => {
                    let _ = unsafe { MinHook::queue_enable_hook(frozen.minhook_lock(), target) };
                }
                Ok(false) => {
                    let _ = unsafe { MinHook::queue_disable_hook(frozen.minhook_lock(), target) };
                }
                Err(_) => {}
            }
        }
        for &(_, target) in targets.iter() {
            live_at_exit.push(matches!(
                MinHook::hook_is_enabled(frozen.minhook_lock(), target),
                Ok(true)
            ));
        }
        drop(frozen);
        let mut retained = 0u64;
        for (index, &(name, target)) in targets.iter().enumerate() {
            let raw = match parked_refusal {
                Some(refusal) => Err(InlineDetourReleaseError::FreezeRefused(refusal)),
                None => statuses[index].map_err(InlineDetourReleaseError::MinHook),
            };
            crate::release_ledger::record_release_attempt(
                target as usize,
                ReleaseOperation::Disable,
                raw,
            );
            let healed = heal_failures
                .iter()
                .find(|(healed_index, _)| *healed_index == index)
                .map(|(_, status)| *status);
            if let Some(status) = healed {
                crate::release_ledger::record_release_attempt(
                    target as usize,
                    ReleaseOperation::Disable,
                    Err(InlineDetourReleaseError::MinHook(status)),
                );
            }
            // AN UNPUBLISHED RESTORE IS NOT A RELEASE, whatever the earlier
            let raw = if unhealed_incoherent.contains(&index) {
                Err(InlineDetourReleaseError::MinHook(
                    MH_STATUS::MH_ERROR_MEMORY_PROTECT,
                ))
            } else if let Some(status) = healed {
                Err(InlineDetourReleaseError::MinHook(status))
            } else {
                raw
            };
            match raw {
                Ok(()) => outcomes.push((name, Ok(()))),
                Err(error) if error.already_released() => {
                    already_released.push(name);
                    outcomes.push((name, Ok(())));
                }
                Err(error) => {
                    retained += 1;
                    outcomes.push((name, Err(error)));
                }
            }
        }
        RETAINED_DETOURS.fetch_add(retained, Ordering::AcqRel);
        BatchReleaseReport {
            outcomes,
            apply_refusal,
            already_released,
            incoherent_cleared: incoherent_at_entry
                .iter()
                .filter(|index| !unhealed_incoherent.contains(index))
                .map(|index| targets[*index].0)
                .collect(),
            incoherent_retained: unhealed_incoherent
                .iter()
                .map(|index| targets[*index].0)
                .collect(),
            live_at_entry,
            live_at_exit,
            freezes: super::freeze::freeze_attempts_on_this_thread().saturating_sub(freezes_before),
        }
    }

    pub unsafe fn remove_batch(targets: &[(&'static str, *mut c_void)]) -> BatchReleaseReport {
        let freezes_before = super::freeze::freeze_attempts_on_this_thread();
        if targets.is_empty() {
            return BatchReleaseReport {
                outcomes: Vec::new(),
                apply_refusal: None,
                already_released: Vec::new(),
                incoherent_cleared: Vec::new(),
                incoherent_retained: Vec::new(),
                live_at_entry: Vec::new(),
                live_at_exit: Vec::new(),
                freezes: 0,
            };
        }
        for &(name, target) in targets {
            crate::release_ledger::name_release_target(target as usize, name);
        }
        // deadlock, not a slow path.
        let mut statuses: Vec<Result<(), InlineDetourReleaseError>> =
            Vec::with_capacity(targets.len());
        let mut frozen = match freeze_other_threads() {
            Ok(frozen) => frozen,
            Err(refusal) => {
                let retained = InlineDetourReleaseError::FreezeRefused(refusal);
                RETAINED_DETOURS.fetch_add(targets.len() as u64, Ordering::AcqRel);
                let outcomes = targets
                    .iter()
                    .map(|&(name, target)| {
                        crate::release_ledger::record_release_attempt(
                            target as usize,
                            ReleaseOperation::Remove,
                            Err(retained),
                        );
                        (name, Err(retained))
                    })
                    .collect();
                return BatchReleaseReport {
                    outcomes,
                    apply_refusal: Some(retained),
                    already_released: Vec::new(),
                    incoherent_cleared: Vec::new(),
                    incoherent_retained: Vec::new(),
                    live_at_entry: vec![true; targets.len()],
                    live_at_exit: vec![true; targets.len()],
                    freezes: super::freeze::freeze_attempts_on_this_thread()
                        .saturating_sub(freezes_before),
                };
            }
        };
        frozen.note_entry(
            FreezeEntryPoint::RemoveBatch,
            targets[0].0,
            targets.len() as u32,
        );
        let parked_refusal = frozen
            .refuse_if_a_thread_is_in(targets.iter().map(|&(_, target)| target), true)
            .err();
        if parked_refusal.is_none() {
            let statuses_ref = &mut statuses;
            frozen.time_minhook(|lock| {
                for &(_, target) in targets {
                    // still admitting callbacks is the same use-after-free one
                    // itself, because the batch's `already_released` roster is
                    if matches!(MinHook::hook_is_enabled(lock, target), Ok(true)) {
                        statuses_ref.push(Err(InlineDetourReleaseError::StillEnabled));
                        continue;
                    }
                    statuses_ref.push(
                        unsafe { MinHook::remove_hook(lock, target) }
                            .map_err(InlineDetourReleaseError::MinHook),
                    );
                }
            });
        }
        drop(frozen);
        RETAINED_DETOURS.fetch_add(
            statuses
                .iter()
                .filter(|status| matches!(status, Err(InlineDetourReleaseError::StillEnabled)))
                .count() as u64,
            Ordering::AcqRel,
        );
        if let Some(refusal) = parked_refusal {
            let retained = InlineDetourReleaseError::FreezeRefused(refusal);
            RETAINED_DETOURS.fetch_add(targets.len() as u64, Ordering::AcqRel);
            let outcomes = targets
                .iter()
                .map(|&(name, target)| {
                    crate::release_ledger::record_release_attempt(
                        target as usize,
                        ReleaseOperation::Remove,
                        Err(retained),
                    );
                    (name, Err(retained))
                })
                .collect();
            return BatchReleaseReport {
                outcomes,
                apply_refusal: Some(retained),
                already_released: Vec::new(),
                incoherent_cleared: Vec::new(),
                incoherent_retained: Vec::new(),
                live_at_entry: vec![true; targets.len()],
                live_at_exit: vec![true; targets.len()],
                freezes: super::freeze::freeze_attempts_on_this_thread()
                    .saturating_sub(freezes_before),
            };
        }
        let mut already_released = Vec::new();
        let outcomes = targets
            .iter()
            .zip(statuses)
            .map(|(&(name, target), status)| {
                crate::release_ledger::record_release_attempt(
                    target as usize,
                    ReleaseOperation::Remove,
                    status,
                );
                // used. It is still named in `already_released` for the two
                match status {
                    Err(error) if error.already_released() => {
                        already_released.push(name);
                        (name, Ok(()))
                    }
                    other => (name, other),
                }
            })
            .collect();
        BatchReleaseReport {
            outcomes,
            apply_refusal: None,
            already_released,
            // REMOVAL, not release. MinHook already refuses to remove an
            incoherent_cleared: Vec::new(),
            incoherent_retained: Vec::new(),
            live_at_entry: Vec::new(),
            live_at_exit: Vec::new(),
            freezes: super::freeze::freeze_attempts_on_this_thread().saturating_sub(freezes_before),
        }
    }

    /// [`InlineDetourReleaseError::detour_retained`].
    pub unsafe fn disable(target: *mut c_void) -> Result<(), InlineDetourReleaseError> {
        let mut frozen = match freeze_other_threads() {
            Ok(frozen) => frozen,
            Err(refusal) => {
                RETAINED_DETOURS.fetch_add(1, Ordering::AcqRel);
                return record_release(
                    target,
                    ReleaseOperation::Disable,
                    Err(InlineDetourReleaseError::FreezeRefused(refusal)),
                );
            }
        };
        frozen.note_entry(FreezeEntryPoint::Disable, "", 1);
        let outcome = match frozen.refuse_if_a_thread_is_in(core::iter::once(target), false) {
            Ok(()) => frozen
                .time_minhook(|lock| unsafe { MinHook::disable_hook(lock, target) })
                .map_err(InlineDetourReleaseError::MinHook),
            Err(refusal) => Err(InlineDetourReleaseError::FreezeRefused(refusal)),
        };
        drop(frozen);
        record_release(target, ReleaseOperation::Disable, outcome)
    }

    /// Record one release attempt in the process-wide ledger and hand the
    fn record_release(
        target: *mut c_void,
        operation: ReleaseOperation,
        result: Result<(), InlineDetourReleaseError>,
    ) -> Result<(), InlineDetourReleaseError> {
        crate::release_ledger::record_release_attempt(target as usize, operation, result);
        result
    }

    ///    both are established under the same lock this release runs under.
    /// [`InlineDetourReleaseError::StillEnabled`] if the target is not already
    pub unsafe fn remove(target: *mut c_void) -> Result<(), InlineDetourReleaseError> {
        let mut frozen = match freeze_other_threads() {
            Ok(frozen) => frozen,
            Err(refusal) => {
                RETAINED_DETOURS.fetch_add(1, Ordering::AcqRel);
                return record_release(
                    target,
                    ReleaseOperation::Remove,
                    Err(InlineDetourReleaseError::FreezeRefused(refusal)),
                );
            }
        };
        frozen.note_entry(FreezeEntryPoint::Remove, "", 1);
        // go stale (`MH_HookIsEnabled` checks lock ownership, walks the
        // idempotent release depends on it. Any other status is one
        let enabled = matches!(
            MinHook::hook_is_enabled(frozen.minhook_lock(), target),
            Ok(true)
        );
        let outcome = if enabled {
            Err(InlineDetourReleaseError::StillEnabled)
        } else {
            match frozen.refuse_if_a_thread_is_in(core::iter::once(target), true) {
                Ok(()) => frozen
                    .time_minhook(|lock| unsafe { MinHook::remove_hook(lock, target) })
                    .map_err(InlineDetourReleaseError::MinHook),
                Err(refusal) => Err(InlineDetourReleaseError::FreezeRefused(refusal)),
            }
        };
        drop(frozen);
        if enabled {
            RETAINED_DETOURS.fetch_add(1, Ordering::AcqRel);
        }
        record_release(target, ReleaseOperation::Remove, outcome)
    }

    /// How many releases were refused, leaving a live detour installed.
    /// Non-zero means some unit believes it released a hook that is in fact
    pub fn retained_detour_count() -> u64 {
        RETAINED_DETOURS.load(Ordering::Acquire)
    }

    pub fn module_pinned_against_unload() -> bool {
        MODULE_PINNED.load(Ordering::Acquire)
    }

    pub fn enabled_detour_count() -> u64 {
        ENABLED_DETOURS.load(Ordering::Acquire)
    }
}

#[cfg(windows)]
pub use release_ledger::{
    any_release_needed_a_retry, last_release_attempt, name_release_target, release_attempt_history,
    release_attempts_dropped_for_capacity, release_is_unproven, render_release_attempt_history,
    render_release_attempts_for, targets_with_a_refused_last_release_attempt,
    unproven_releases_among, ReleaseAttempt, ReleaseAttemptOutcome, ReleaseOperation,
};

#[cfg(windows)]
pub use freeze::{
    freeze_attempts, freeze_attempts_on_this_thread, freeze_passes_spent, freeze_process,
    freeze_refusals, suspend_failures_deferred_to_a_later_pass,
    suspend_failures_excused_as_terminated, FreezeRefusal, ProcessFreeze,
};

#[cfg(windows)]
pub(crate) use windows_impl::run_pinned;
#[cfg(windows)]
pub use windows_impl::{
    apply_queued, disable, disable_batch, enable, enable_batch, enabled_detour_count,
    module_pinned_against_unload, pin_module_now, queue_enable, remove, remove_batch,
    retained_detour_count, BatchEnableReport, BatchOutcome, BatchReleaseReport,
    InlineDetourEnableError, InlineDetourReleaseError,
};

#[cfg(test)]
const BANNED_CALL_FRAGMENTS: &[(&str, &str)] = &[
    ("MinHook::", "enable_hook("),
    ("MinHook::", "enable_all_hooks("),
    ("MinHook::", "queue_enable_hook("),
    ("MinHook::", "disable_hook("),
    ("MinHook::", "disable_all_hooks("),
    ("MinHook::", "queue_disable_hook("),
    ("MinHook::", "remove_hook("),
    ("MinHook::", "apply_queued("),
];

#[cfg(test)]
fn normalize_for_scan(text: &str) -> String {
    let mut out = text.to_string();
    while out.contains(" ::") || out.contains(":: ") {
        out = out.replace(" ::", "::").replace(":: ", "::");
    }
    let mut aliases = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        for marker in ["use minhook::MinHook as ", "MinHook as "] {
            if !line.starts_with("use ") && marker.starts_with("use ") {
                continue;
            }
            if marker == "MinHook as " && !line.starts_with("use minhook::") {
                continue;
            }
            let Some(at) = line.find(marker) else {
                continue;
            };
            let rest = &line[at + marker.len()..];
            if let Some(alias) = rest
                .split(|c: char| c == ';' || c == ',' || c == '}' || c.is_whitespace())
                .next()
            {
                if !alias.is_empty() {
                    aliases.push(alias.to_string());
                }
            }
        }
    }
    for alias in aliases {
        out = out.replace(&format!("{alias}::"), "MinHook::");
    }
    out
}

#[cfg(test)]
fn scan_source_for_direct_enable(text: &str) -> Vec<usize> {
    let normalized = normalize_for_scan(text);
    let banned: Vec<String> = BANNED_CALL_FRAGMENTS
        .iter()
        .map(|(prefix, call)| format!("{prefix}{call}"))
        .collect();
    normalized
        .lines()
        .enumerate()
        .filter(|(_, line)| banned.iter().any(|b| line.contains(b.as_str())))
        .map(|(index, _)| index + 1)
        .collect()
}

#[cfg(test)]
const BANNED_SUSPENDER_FRAGMENTS: &[(&str, &str)] = &[
    ("Suspend", "Thread("),
    ("Wow64Suspend", "Thread("),
    ("NtSuspend", "Thread("),
    ("ZwSuspend", "Thread("),
    ("Open", "Thread("),
    ("Thread32", "First("),
    ("Thread32", "Next("),
    ("GetThread", "Context("),
    ("NtGetNext", "Thread("),
    ("ZwGetNext", "Thread("),
];

#[cfg(test)]
fn strip_line_comments(text: &str) -> String {
    text.lines()
        .map(|line| match line.find("//") {
            Some(index) => &line[..index],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
fn scan_source_for_private_suspender(text: &str) -> Vec<usize> {
    let code = strip_line_comments(&normalize_for_scan(text));
    let banned: Vec<String> = BANNED_SUSPENDER_FRAGMENTS
        .iter()
        .map(|(prefix, call)| format!("{prefix}{call}"))
        .collect();
    code.lines()
        .enumerate()
        .filter(|(_, line)| banned.iter().any(|b| line.contains(b.as_str())))
        .map(|(index, _)| index + 1)
        .collect()
}

#[cfg(test)]
mod source_ban_tests {
    use super::{
        scan_source_for_direct_enable, scan_source_for_private_suspender, BANNED_CALL_FRAGMENTS,
        BANNED_SUSPENDER_FRAGMENTS,
    };
    use std::path::{Path, PathBuf};

    fn workspace_sources() -> Vec<PathBuf> {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("inline_detour sits three levels below the repository root")
            .to_path_buf();
        let roots: Vec<PathBuf> = ["extractor/native", "replay_interface/native"]
            .iter()
            .map(|rel| repo.join(rel))
            .collect();
        for root in &roots {
            assert!(
                root.join("Cargo.toml").is_file(),
                "expected a native workspace root at {}",
                root.display()
            );
        }
        let mut sources = Vec::new();
        let mut stack = roots;
        let skip = ["target", ".cargo-target", ".git", "runs", "fixtures"];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if !skip.contains(&name) {
                        stack.push(path);
                    }
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    sources.push(path);
                }
            }
        }
        assert!(
            sources.len() > 100,
            "the scan found only {} sources; it is not reaching the workspace",
            sources.len()
        );
        sources
    }

    fn is_this_crate(path: &Path) -> bool {
        path.starts_with(env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn no_direct_minhook_enable_outside_this_crate() {
        let mut violations = Vec::new();
        for path in workspace_sources() {
            if is_this_crate(&path) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in scan_source_for_direct_enable(&text) {
                violations.push(format!("{}:{}", path.display(), line));
            }
        }
        assert!(
            violations.is_empty(),
            "inline detours must be enabled through `inline_detour::enable`, which pins this module against unload first. Enabling directly leaves a detour live in a module the collector can still unload, and a thread poised in that detour body then resumes on unmapped memory.\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn the_scanner_catches_direct_aliased_and_whitespace_split_calls() {
        let (prefix, call) = BANNED_CALL_FRAGMENTS[0];
        let direct = format!("fn f() {{ unsafe {{ {prefix}{call}t) }}; }}");
        assert_eq!(
            scan_source_for_direct_enable(&direct),
            vec![1],
            "the plain call must be caught"
        );

        let spaced = format!("fn f() {{ unsafe {{ MinHook :: {call}t) }}; }}");
        assert_eq!(
            scan_source_for_direct_enable(&spaced),
            vec![1],
            "whitespace around :: must not defeat the scan"
        );

        let aliased =
            format!("use minhook::MinHook as MH;\nfn f() {{ unsafe {{ MH::{call}t) }}; }}");
        assert_eq!(
            scan_source_for_direct_enable(&aliased),
            vec![2],
            "an aliased import must not defeat the scan"
        );

        let grouped = format!(
            "use minhook::{{MinHook as MH, MH_STATUS}};\nfn f() {{ unsafe {{ MH::{call}t) }}; }}"
        );
        assert_eq!(
            scan_source_for_direct_enable(&grouped),
            vec![2],
            "a grouped aliased import must not defeat the scan either"
        );

        let clean = "fn f() { unsafe { inline_detour::enable(t) }; }";
        assert!(
            scan_source_for_direct_enable(clean).is_empty(),
            "the wrapper call is not a violation"
        );
    }

    #[test]
    fn every_minhook_entry_point_that_rewrites_a_target_is_banned() {
        for call in [
            "enable_hook",
            "enable_all_hooks",
            "queue_enable_hook",
            "disable_hook",
            "disable_all_hooks",
            "queue_disable_hook",
            "remove_hook",
            "apply_queued",
        ] {
            let planted = format!("fn f() {{ unsafe {{ MinHook::{call}(t) }}; }}");
            assert_eq!(
                scan_source_for_direct_enable(&planted),
                vec![1],
                "MinHook::{call} rewrites a live target and must be banned outside this crate"
            );
        }
    }

    #[test]
    fn creating_a_hook_is_not_banned_because_it_rewrites_nothing() {
        let planted = "fn f() { unsafe { MinHook::create_hook(t, d) }; }";
        assert!(
            scan_source_for_direct_enable(planted).is_empty(),
            "MinHook::create_hook patches no live bytes and must stay callable"
        );
    }

    #[test]
    fn no_private_thread_suspender_outside_this_crate() {
        let mut violations = Vec::new();
        for path in workspace_sources() {
            if is_this_crate(&path) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in scan_source_for_private_suspender(&text) {
                violations.push(format!("{}:{}", path.display(), line));
            }
        }
        assert!(
            violations.is_empty(),
            "code that rewrites live bytes must freeze the process through `inline_detour` -- `enable`/`disable`/`remove` when MinHook owns the patch, `freeze_process` when the caller writes its own bytes. A private suspender reads a broken `Thread32Next` as the end of the thread list and enumerates once, so it certifies a truncated list and never sees a thread created after its snapshot; both are what the shared freeze exists to refuse.\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn the_suspender_scanner_catches_calls_and_extern_declarations() {
        for (prefix, call) in BANNED_SUSPENDER_FRAGMENTS {
            let called = format!("fn f() {{ unsafe {{ {prefix}{call}h) }}; }}");
            assert_eq!(
                scan_source_for_private_suspender(&called),
                vec![1],
                "a call to {prefix}{call} must be caught"
            );
            let declared = format!("extern \"system\" {{ fn {prefix}{call}h: Handle) -> u32; }}");
            assert_eq!(
                scan_source_for_private_suspender(&declared),
                vec![1],
                "an extern declaration of {prefix}{call} must be caught"
            );
        }

        let commented = "// the old suspender called SuspendThread(handle) here";
        assert!(
            scan_source_for_private_suspender(commented).is_empty(),
            "prose about the removed suspenders is not a violation"
        );

        let shared = "let freeze = inline_detour::freeze_process()?;";
        assert!(
            scan_source_for_private_suspender(shared).is_empty(),
            "taking the shared freeze is the whole point and must not be flagged"
        );

        let launcher = "unsafe { ResumeThread(process.main_thread) };";
        assert!(
            scan_source_for_private_suspender(launcher).is_empty(),
            "resuming a process created suspended is recorder_host's job and \
             touches no thread of this process"
        );
    }

    #[test]
    fn the_four_migrated_patchers_are_still_named_and_still_clean() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("inline_detour has a parent directory")
            .to_path_buf();
        for unit in [
            "lanes/fired_event_popups_live/src/a729_inline_hook.rs",
            "lanes/focus_research_decisions_depth_live/src/a729_focus_item_inline.rs",
            "lanes/naval_combat_live/src/convoy_terminal_hook_windows.rs",
            "lanes/supply_logistics_attrition_live/src/a729_attrition_hook.rs",
        ] {
            let path = root.join(unit);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{unit} must still exist: {error}"));
            assert!(
                scan_source_for_private_suspender(&text).is_empty(),
                "{unit} suspends threads itself again at line(s) {:?}",
                scan_source_for_private_suspender(&text)
            );
            let code = super::strip_line_comments(&text);
            assert!(
                code.contains("inline_detour::freeze_process()"),
                "{unit} rewrites live bytes and must take the shared freeze"
            );
            assert!(
                code.contains("any_instruction_pointer_in"),
                "{unit} must ask where the frozen threads are stopped before it \
                 rewrites a range; suspension alone does not prove the range is \
                 clear"
            );
        }
    }

    #[test]
    fn the_scan_reaches_the_crates_that_own_detours() {
        let sources = workspace_sources();
        for owner in [
            "fow_visibility_live",
            "land_combat_detail_events_live",
            "dynamic_collection_trace_live",
            "collector",
            "terrain_capture",
        ] {
            assert!(
                sources.iter().any(|p| p.to_string_lossy().contains(owner)),
                "the scan must reach {owner}, which owns inline detours"
            );
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// a build that dropped it would silently reopen the use-after-free the
    #[test]
    fn enabling_a_detour_pins_the_containing_module_first() {
        // this order, everywhere, so two tests cannot deadlock on them.
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _serialised = crate::freeze::os_suspend_state_guard();
        let refusal = unsafe { enable(0x1000 as *mut core::ffi::c_void) }
            .expect_err("no hook was ever created at this address");
        assert!(
            refusal.minhook_status().is_some(),
            "the refusal must come from MinHook, not from the pin: {refusal}"
        );
        assert!(
            module_pinned_against_unload(),
            "the module must be pinned before any patch is attempted"
        );
    }

    #[test]
    fn every_entry_point_funnels_through_the_ordering_helper() {
        let source = include_str!("lib.rs");
        let needle = concat!("run_pinned(", "pin_containing_module");
        let funnelled = source.matches(needle).count();
        assert_eq!(
            funnelled, 3,
            "enable, queue_enable and apply_queued must each go through              run_pinned; found {funnelled} call(s), so one of them patches              without the pin ordering"
        );
    }

    #[test]
    fn a_refused_pin_means_the_patch_never_runs() {
        use std::cell::Cell;
        let ran = Cell::new(false);
        let outcome = super::run_pinned(
            || Err(5u32),
            || {
                ran.set(true);
                Ok::<(), minhook::MH_STATUS>(())
            },
        );
        assert!(
            !ran.get(),
            "the patch must not run when the pin was refused"
        );
        assert_eq!(
            outcome.unwrap_err(),
            InlineDetourEnableError::ModulePinRefused(5),
            "and the refusal must name the pin, not MinHook"
        );

        let ran = Cell::new(false);
        let outcome = super::run_pinned(
            || Ok(()),
            || {
                ran.set(true);
                Ok::<(), minhook::MH_STATUS>(())
            },
        );
        assert!(ran.get(), "an accepted pin must let the patch run");
        assert!(outcome.is_ok());
    }

    #[test]
    fn the_queued_path_pins_before_it_applies() {
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _serialised = crate::freeze::os_suspend_state_guard();
        let queued = unsafe { queue_enable(0x3000 as *mut core::ffi::c_void) };
        assert!(
            queued.is_err() && queued.unwrap_err().minhook_status().is_some(),
            "no hook exists at this address, so MinHook must be the refuser"
        );
        assert!(
            module_pinned_against_unload(),
            "queuing must pin: a queue that is later applied patches the target"
        );

        let applied = unsafe { apply_queued() };
        let _ = applied;
        assert!(module_pinned_against_unload());
    }

    #[test]
    fn the_standalone_pin_reports_only_what_the_loader_accepted() {
        assert!(pin_module_now().is_ok());
        assert!(module_pinned_against_unload());
    }

    #[test]
    fn a_refused_patch_is_not_counted_as_an_enabled_detour() {
        // this order, everywhere, so two tests cannot deadlock on them.
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _serialised = crate::freeze::os_suspend_state_guard();
        let before = enabled_detour_count();
        let _ = unsafe { enable(0x2000 as *mut core::ffi::c_void) };
        assert_eq!(
            enabled_detour_count(),
            before,
            "a detour MinHook refused to patch was never enabled"
        );
    }

    #[test]
    fn the_two_refusals_are_distinguishable() {
        let pin = InlineDetourEnableError::ModulePinRefused(5);
        assert!(pin.minhook_status().is_none());
        assert!(pin.to_string().contains("could not be pinned"));
        assert!(pin.to_string().contains('5'));
        let hook = InlineDetourEnableError::MinHook(minhook::MH_STATUS::MH_ERROR_NOT_CREATED);
        assert!(hook.minhook_status().is_some());
        assert!(!hook.to_string().contains("could not be pinned"));
        assert_ne!(pin.to_string(), hook.to_string());
    }

    #[test]
    fn every_patching_wrapper_freezes_the_process_before_it_calls_minhook() {
        // The release LEDGER first, then the suspend state -- the order every
        // record release attempts into a process-global ledger whose capacity
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _serialised = crate::freeze::os_suspend_state_guard();
        let target = 0x3000 as *mut core::ffi::c_void;

        let before = freeze_attempts_on_this_thread();
        let _ = unsafe { enable(target) };
        assert_eq!(
            freeze_attempts_on_this_thread(),
            before + 1,
            "enable must freeze the process exactly once before patching"
        );

        let before = freeze_attempts_on_this_thread();
        let _ = unsafe { disable(target) };
        assert_eq!(
            freeze_attempts_on_this_thread(),
            before + 1,
            "disable must freeze the process exactly once before restoring bytes"
        );

        let before = freeze_attempts_on_this_thread();
        let _ = unsafe { remove(target) };
        assert_eq!(
            freeze_attempts_on_this_thread(),
            before + 1,
            "remove must freeze the process exactly once before restoring bytes \
             and freeing the trampoline"
        );
    }

    /// Falsifiable: fails if a release that could not freeze is ever reported
    /// The distinction is the whole point of the release error existing
    /// separately. `already_released` means the hook was gone and there is
    /// patched bytes and the owning unit must not claim it released anything.
    #[test]
    fn a_release_that_could_not_freeze_reports_a_retained_detour() {
        let retained = InlineDetourReleaseError::FreezeRefused(FreezeRefusal::ThreadSuspend {
            thread: 4242,
            os_error: 5,
        });
        assert!(retained.detour_retained());
        assert!(!retained.already_released());
        assert!(retained.minhook_status().is_none());
        assert!(retained
            .to_string()
            .contains("retained rather than restored"));
        assert!(retained.to_string().contains("4242"));

        for absent in [
            InlineDetourReleaseError::MinHook(minhook::MH_STATUS::MH_ERROR_DISABLED),
            InlineDetourReleaseError::MinHook(minhook::MH_STATUS::MH_ERROR_NOT_CREATED),
        ] {
            assert!(absent.already_released());
            assert!(!absent.detour_retained());
        }

        let refused =
            InlineDetourReleaseError::MinHook(minhook::MH_STATUS::MH_ERROR_MEMORY_PROTECT);
        assert!(!refused.already_released());
        assert!(!refused.detour_retained());
    }

    #[test]
    fn an_enable_refused_by_the_freeze_is_not_a_minhook_status() {
        let refusal = InlineDetourEnableError::FreezeRefused(FreezeRefusal::NotQuiescent {
            passes: 16,
            unaccounted: 3,
        });
        assert!(refusal.minhook_status().is_none());
        assert!(refusal.freeze_refusal().is_some());
        assert!(refusal.to_string().contains("no target byte was touched"));
        assert!(InlineDetourEnableError::ModulePinRefused(5)
            .freeze_refusal()
            .is_none());
    }
}

#[cfg(all(test, windows))]
mod batch_tests {
    use super::*;
    use core::ffi::c_void;
    use minhook::{MinHook, MH_STATUS};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// modules cannot deadlock against each other.
    static BATCH_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    static VICTIM_CALLS: [AtomicU64; 4] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];
    static DETOUR_CALLS: [AtomicU64; 4] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];

    #[inline(never)]
    extern "system" fn victim_0(x: u64) -> u64 {
        VICTIM_CALLS[0].fetch_add(1, Ordering::AcqRel);
        x.wrapping_mul(3).wrapping_add(0x1000)
    }
    #[inline(never)]
    extern "system" fn victim_1(x: u64) -> u64 {
        VICTIM_CALLS[1].fetch_add(1, Ordering::AcqRel);
        x.wrapping_mul(5).wrapping_add(0x2000)
    }
    #[inline(never)]
    extern "system" fn victim_2(x: u64) -> u64 {
        VICTIM_CALLS[2].fetch_add(1, Ordering::AcqRel);
        x.wrapping_mul(7).wrapping_add(0x3000)
    }
    #[inline(never)]
    extern "system" fn victim_3(x: u64) -> u64 {
        VICTIM_CALLS[3].fetch_add(1, Ordering::AcqRel);
        x.wrapping_mul(11).wrapping_add(0x4000)
    }

    #[inline(never)]
    extern "system" fn detour_0(_x: u64) -> u64 {
        DETOUR_CALLS[0].fetch_add(1, Ordering::AcqRel);
        0xD0
    }
    #[inline(never)]
    extern "system" fn detour_1(_x: u64) -> u64 {
        DETOUR_CALLS[1].fetch_add(1, Ordering::AcqRel);
        0xD1
    }
    #[inline(never)]
    extern "system" fn detour_2(_x: u64) -> u64 {
        DETOUR_CALLS[2].fetch_add(1, Ordering::AcqRel);
        0xD2
    }
    #[inline(never)]
    extern "system" fn detour_3(_x: u64) -> u64 {
        DETOUR_CALLS[3].fetch_add(1, Ordering::AcqRel);
        0xD3
    }

    type Victim = extern "system" fn(u64) -> u64;

    fn victims() -> [(Victim, Victim); 4] {
        [
            (victim_0, detour_0),
            (victim_1, detour_1),
            (victim_2, detour_2),
            (victim_3, detour_3),
        ]
    }

    const ABSENT_TARGET: *mut c_void = 0x5000 as *mut c_void;

    fn create_victims(count: usize) -> Vec<(&'static str, *mut c_void)> {
        const NAMES: [&str; 4] = ["victim.0", "victim.1", "victim.2", "victim.3"];
        let mut targets = Vec::new();
        for (index, (victim, detour)) in victims().into_iter().enumerate().take(count) {
            let target = victim as *mut c_void;
            let _ = unsafe { remove(target) };
            unsafe { MinHook::create_hook(target, detour as *mut c_void) }
                .unwrap_or_else(|status| panic!("{} must be hookable: {status}", NAMES[index]));
            targets.push((NAMES[index], target));
        }
        targets
    }

    fn destroy_victims(targets: &[(&'static str, *mut c_void)]) {
        for &(_, target) in targets {
            let _ = unsafe { disable(target) };
            let _ = unsafe { remove(target) };
        }
    }

    fn call(index: usize) -> u64 {
        victims()[index].0(1)
    }

    #[test]
    fn one_refused_hook_in_a_batch_is_named_and_the_others_are_not_blamed() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The release LEDGER is process-global and every one of these
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();
        let real = create_victims(2);
        let batch = [real[0], ("absent.hook", ABSENT_TARGET), real[1]];

        let report = unsafe { enable_batch(&batch) };

        assert_eq!(report.len(), 3);
        assert!(report.outcomes()[0].1.is_ok(), "{:?}", report.outcomes()[0]);
        assert!(report.outcomes()[2].1.is_ok(), "{:?}", report.outcomes()[2]);
        let (name, error) = report
            .refusal()
            .expect("the absent target must be reported as a failure");
        assert_eq!(name, "absent.hook");
        assert_eq!(
            error.minhook_status(),
            Some(MH_STATUS::MH_ERROR_NOT_CREATED),
            "the refusal must be the one MinHook gave for THAT target: {error}"
        );
        assert_eq!(
            report.failures().len(),
            1,
            "one target's refusal must not be charged to the others: {:?}",
            report.outcomes()
        );
        assert!(!report.all_succeeded());

        assert!(report.rolled_back(), "a refused target undoes the batch");
        VICTIM_CALLS[0].store(0, Ordering::Release);
        VICTIM_CALLS[1].store(0, Ordering::Release);
        assert_ne!(call(0), 0xD0, "victim.0 must not be left detoured");
        assert_ne!(call(1), 0xD1, "victim.1 must not be left detoured");
        assert_eq!(VICTIM_CALLS[0].load(Ordering::Acquire), 1);
        assert_eq!(VICTIM_CALLS[1].load(Ordering::Acquire), 1);

        let _ = unsafe { apply_queued() };
        VICTIM_CALLS[0].store(0, Ordering::Release);
        assert_ne!(
            call(0),
            0xD0,
            "an unrelated apply must not patch what this batch rolled back"
        );
        assert_eq!(VICTIM_CALLS[0].load(Ordering::Acquire), 1);

        destroy_victims(&real);
    }

    #[test]
    fn a_batch_of_four_takes_one_freeze_where_four_singles_take_four() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The release LEDGER is process-global and every one of these
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();
        let targets = create_victims(4);

        let before_singles = freeze_attempts_on_this_thread();
        for &(_, target) in &targets {
            unsafe { enable(target) }.expect("each single enable must succeed");
        }
        let singles = freeze_attempts_on_this_thread() - before_singles;
        assert_eq!(
            singles, 4,
            "the per-hook path must cost one freeze per hook; that is the cost being removed"
        );

        for &(_, target) in &targets {
            unsafe { disable(target) }.expect("each single disable must succeed");
        }

        let report = unsafe { enable_batch(&targets) };
        assert!(
            report.all_succeeded(),
            "the batch must enable all four: {:?}",
            report.outcomes()
        );
        assert_eq!(
            report.freezes(),
            1,
            "one freeze must serve the whole batch, not {} of them",
            report.freezes()
        );
        assert!(
            report.freezes() < singles,
            "the batch must cost strictly fewer freezes than the per-hook path"
        );
        assert!(report.apply_refusal().is_none());
        assert!(!report.rolled_back());

        for index in 0..4 {
            DETOUR_CALLS[index].store(0, Ordering::Release);
        }
        for index in 0..4 {
            assert_eq!(call(index), 0xD0 + index as u64, "victim.{index} detoured");
        }

        let release = unsafe { disable_batch(&targets) };
        assert!(
            release.all_succeeded(),
            "the batched release must restore all four: {:?}",
            release.outcomes()
        );
        assert_eq!(
            release.freezes(),
            1,
            "one freeze must serve the whole release batch too"
        );
        for index in 0..4 {
            VICTIM_CALLS[index].store(0, Ordering::Release);
        }
        for index in 0..4 {
            assert_ne!(
                call(index),
                0xD0 + index as u64,
                "victim.{index} must be restored"
            );
            assert_eq!(VICTIM_CALLS[index].load(Ordering::Acquire), 1);
        }

        destroy_victims(&targets);
    }

    #[test]
    fn an_empty_batch_never_freezes_and_a_broken_roster_freezes_at_most_once() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The release LEDGER is process-global and every one of these
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();

        let empty = unsafe { enable_batch(&[]) };
        assert_eq!(empty.freezes(), 0, "nothing to patch, nothing to freeze");
        assert!(empty.is_empty());
        let empty_release = unsafe { disable_batch(&[]) };
        assert_eq!(empty_release.freezes(), 0);
        let empty_removal = unsafe { remove_batch(&[]) };
        assert_eq!(empty_removal.freezes(), 0);

        let all_absent = unsafe { enable_batch(&[("absent.a", ABSENT_TARGET)]) };
        assert_eq!(all_absent.freezes(), 1);
        assert_eq!(all_absent.failures().len(), 1);
        assert_eq!(all_absent.refusal().map(|(name, _)| name), Some("absent.a"));

        let released = unsafe { disable_batch(&[("absent.b", ABSENT_TARGET)]) };
        assert_eq!(released.freezes(), 1);
        // is the postcondition a release asks for.
        assert!(released.all_succeeded());
        assert!(released.was_already_released("absent.b"));
    }

    #[test]
    fn the_batch_counts_every_target_it_enabled_and_no_target_it_refused() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The release LEDGER is process-global and every one of these
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();
        let targets = create_victims(3);

        let before = enabled_detour_count();
        let clean = unsafe { enable_batch(&targets) };
        assert!(clean.all_succeeded(), "{:?}", clean.outcomes());
        assert_eq!(
            enabled_detour_count() - before,
            3,
            "three targets were patched, so three detours are live"
        );
        let released = unsafe { disable_batch(&targets) };
        assert!(released.all_succeeded(), "{:?}", released.outcomes());

        let batch = [
            targets[0],
            targets[1],
            ("absent.c", ABSENT_TARGET),
            targets[2],
        ];
        let before = enabled_detour_count();
        let report = unsafe { enable_batch(&batch) };
        assert_eq!(report.failures().len(), 1);
        assert!(report.rolled_back());
        assert_eq!(
            enabled_detour_count(),
            before,
            "a batch that rolled itself back left no detour to count"
        );

        destroy_victims(&targets);
    }

    #[test]
    fn a_batch_that_lists_the_same_target_twice_is_refused_outright() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The release LEDGER is process-global and every one of these
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();
        let targets = create_victims(2);

        let before = enabled_detour_count();
        let freezes = freeze_attempts_on_this_thread();
        let report = unsafe { enable_batch(&[targets[0], targets[1], targets[0]]) };
        assert_eq!(report.failures().len(), 3, "{:?}", report.outcomes());
        assert_eq!(report.freezes(), 0, "a refused roster patches nothing");
        assert_eq!(
            freeze_attempts_on_this_thread(),
            freezes,
            "and freezes nothing"
        );
        assert_eq!(enabled_detour_count(), before, "and counts nothing");

        let clean = unsafe { enable_batch(&targets) };
        assert!(clean.all_succeeded(), "{:?}", clean.outcomes());
        let _ = unsafe { disable_batch(&targets) };
        destroy_victims(&targets);
    }

    #[test]
    fn every_patching_window_refuses_before_it_patches() {
        let source = include_str!("lib.rs").replace(
            "
", "
",
        );
        let marker = concat!("// FROZEN WI", "NDOW:");
        let closer = concat!("drop(fro", "zen);");
        let patching = [
            concat!("MinHook::", "apply_queued("),
            concat!("MinHook::", "enable_hook("),
            concat!("MinHook::", "disable_hook("),
            concat!("MinHook::", "remove_hook("),
        ];
        let checks = [
            "refuse_if_a_thread_is_in(",
            "refuse_if_a_thread_is_in_a_queued_patch(",
        ];
        let mut windows = 0;
        let mut cursor = 0usize;
        while let Some(at) = source[cursor..].find(closer) {
            let end = cursor + at;
            let opened = source[..end]
                .rfind(marker)
                .expect("every freeze drop has a window marker");
            let window = &source[opened..end];
            cursor = end + closer.len();
            let first_patch = patching.iter().filter_map(|call| window.find(call)).min();
            let Some(first_patch) = first_patch else {
                continue;
            };
            windows += 1;
            let first_check = checks
                .iter()
                .filter_map(|call| window.find(call))
                .min()
                .unwrap_or_else(|| {
                    panic!(
                        "a window patches live bytes without asking whether a                          thread is stopped inside them:
{window}"
                    )
                });
            assert!(
                first_check < first_patch,
                "the check must come BEFORE the patch, not after it:
{window}"
            );
        }
        assert!(
            windows >= 5,
            "the scan found only {windows} patching window(s); it is not              reaching them"
        );
    }

    /// thread might hold, deadlocks the process permanently -- it is not a
    /// slow path, there is nobody left to release the lock. A `.collect()`
    /// formatting an error, writing to the release ledger, or collecting an
    #[test]
    fn no_held_freeze_window_allocates_or_takes_a_lock() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        let banned = [
            concat!(".coll", "ect()"),
            concat!("Vec::with_cap", "acity"),
            concat!("form", "at!("),
            concat!("to_st", "ring()"),
            concat!("release_ledger::", "record_release_attempt"),
            concat!(".lo", "ck()"),
            concat!("MinHookLock::", "acquire"),
        ];
        let marker = concat!("// FROZEN WI", "NDOW:");
        let closer = concat!("drop(fro", "zen);");
        let mut windows = 0;
        let mut cursor = 0usize;
        while let Some(at) = source[cursor..].find(closer) {
            let end = cursor + at;
            let opened = source[..end]
                .rfind(marker)
                .unwrap_or_else(|| panic!("a held freeze at byte {end} has no window marker"));
            let window = &source[opened..end];
            assert!(
                !window.contains(closer),
                "window markers and freeze drops must alternate"
            );
            windows += 1;
            for fragment in banned {
                assert!(
                    !window.contains(fragment),
                    "a held freeze window contains `{fragment}`, which can block or \
                     allocate while every other thread in the process is suspended:\n{window}"
                );
            }
            cursor = end + closer.len();
        }
        assert!(
            windows >= 7,
            "the scan found only {windows} held-freeze window(s); it is not reaching them"
        );
    }

    /// OBSERVED: FAILED, `install, release and removal must each cost ONE
    #[test]
    fn twelve_hooks_cost_twelve_freezes_one_at_a_time_and_one_as_a_batch() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The release LEDGER is process-global and every one of these
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();
        let targets = create_roster_victims();
        assert_eq!(targets.len(), 12, "the staged roster is D45-sized");

        let before_singles = freeze_attempts_on_this_thread();
        for &(name, target) in &targets {
            unsafe { enable(target) }.unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let singles_enable = freeze_attempts_on_this_thread() - before_singles;

        let before_single_release = freeze_attempts_on_this_thread();
        for &(name, target) in &targets {
            unsafe { disable(target) }.unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let singles_disable = freeze_attempts_on_this_thread() - before_single_release;

        let enabled = unsafe { enable_batch(&targets) };
        assert!(enabled.all_succeeded(), "{:?}", enabled.outcomes());
        let released = unsafe { disable_batch(&targets) };
        assert!(released.all_succeeded(), "{:?}", released.outcomes());
        let removed = unsafe { remove_batch(&targets) };
        assert!(removed.all_succeeded(), "{:?}", removed.outcomes());

        assert_eq!(
            (singles_enable, singles_disable),
            (targets.len() as u64, targets.len() as u64),
            "the per-hook path costs one freeze per hook in each direction"
        );
        assert_eq!(
            (enabled.freezes(), released.freezes(), removed.freezes()),
            (1, 1, 1),
            "install, release and removal must each cost ONE freeze for the whole unit"
        );
        assert_eq!(
            singles_enable + singles_disable,
            (enabled.freezes() + released.freezes()) * 12,
            "12x on the install-plus-release span"
        );
    }

    #[test]
    fn a_batch_install_leaves_a_named_row_in_the_production_freeze_census() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();
        let targets = create_roster_victims();
        let before = crate::freeze_census::freeze_windows().len();

        let enabled = unsafe { enable_batch(&targets) };
        assert!(enabled.all_succeeded(), "{:?}", enabled.outcomes());
        let disabled = unsafe { disable_batch(&targets) };
        assert!(disabled.all_succeeded(), "{:?}", disabled.outcomes());
        let released = unsafe { remove_batch(&targets) };
        assert!(released.all_succeeded(), "{:?}", released.outcomes());

        let rows = crate::freeze_census::freeze_windows();
        assert!(
            rows.len() > before,
            "a real batch install recorded no census row at all; the recording \
             would carry the same silence lane t54's leg 5 named"
        );
        let batch_row = rows
            .iter()
            .rev()
            .find(|row| {
                row.entry_point == Some(crate::freeze_census::FreezeEntryPoint::EnableBatch)
                    && row.label == targets[0].0
            })
            .expect("the batch install's window is named by its first target");
        assert_eq!(
            batch_row.target_count,
            targets.len() as u32,
            "the row must say how many targets the window covered"
        );
        assert!(
            batch_row.window_us > 0,
            "the window has zero duration, which means no clock was read"
        );
        assert!(
            batch_row.minhook_us > 0,
            "the row says MH_ApplyQueued took no time; the apply is what this \
             census exists to size and an unmarked span is indistinguishable \
             from a window that never called it"
        );
        assert!(
            batch_row.acquire_us + batch_row.minhook_us <= batch_row.window_us,
            "the parts exceed the whole: acquire {} + minhook {} > window {}",
            batch_row.acquire_us,
            batch_row.minhook_us,
            batch_row.window_us
        );
        assert!(
            batch_row.passes >= 2,
            "a clean freeze spends two enumeration passes; {} says the row's \
             pass count was never carried across from the freeze",
            batch_row.passes
        );
        assert!(
            rows.iter().rev().any(|row| {
                row.entry_point == Some(crate::freeze_census::FreezeEntryPoint::RemoveBatch)
                    && row.label == targets[0].0
            }),
            "the release half of the capture records no window; a teardown that \
             stops the game is a stopped window too"
        );
    }

    fn create_roster_victims() -> Vec<(&'static str, *mut c_void)> {
        macro_rules! roster {
            ($($name:ident = $salt:expr),+ $(,)?) => {{
                $(
                    #[inline(never)]
                    extern "system" fn $name(x: u64) -> u64 {
                        ROSTER_CALLS.fetch_add($salt, Ordering::AcqRel);
                        x.wrapping_mul($salt).wrapping_add($salt)
                    }
                )+
                let entries: Vec<(&'static str, *mut c_void)> = vec![
                    $((stringify!($name), $name as *mut c_void)),+
                ];
                entries
            }};
        }
        let entries = roster!(
            roster_0 = 3,
            roster_1 = 5,
            roster_2 = 7,
            roster_3 = 11,
            roster_4 = 13,
            roster_5 = 17,
            roster_6 = 19,
            roster_7 = 23,
            roster_8 = 29,
            roster_9 = 31,
            roster_10 = 37,
            roster_11 = 41,
        );
        let mut distinct: Vec<usize> = entries.iter().map(|(_, t)| *t as usize).collect();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            entries.len(),
            "the staged roster must be twelve DISTINCT targets, not one folded body"
        );
        for &(name, target) in &entries {
            let _ = unsafe { remove(target) };
            unsafe { MinHook::create_hook(target, roster_detour as *mut c_void) }
                .unwrap_or_else(|status| panic!("{name} must be hookable: {status}"));
        }
        entries
    }

    static ROSTER_CALLS: AtomicU64 = AtomicU64::new(0);

    #[inline(never)]
    extern "system" fn roster_detour(_x: u64) -> u64 {
        ROSTER_CALLS.fetch_add(1, Ordering::AcqRel);
        0xE0
    }

    #[test]
    fn a_batched_removal_frees_every_trampoline_under_one_freeze() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The release LEDGER is process-global and every one of these
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();
        let targets = create_victims(4);
        let installed = unsafe { enable_batch(&targets) };
        assert!(installed.all_succeeded(), "{:?}", installed.outcomes());
        let released = unsafe { disable_batch(&targets) };
        assert!(released.all_succeeded(), "{:?}", released.outcomes());

        let mut batch = targets.clone();
        batch.push(("absent.removal", ABSENT_TARGET));
        let before = freeze_attempts_on_this_thread();
        let removed = unsafe { remove_batch(&batch) };
        assert_eq!(
            freeze_attempts_on_this_thread() - before,
            1,
            "one freeze must cover every removal in the batch"
        );
        assert_eq!(removed.freezes(), 1);
        for index in 0..4 {
            assert!(
                removed.outcomes()[index].1.is_ok(),
                "{:?}",
                removed.outcomes()[index]
            );
            assert!(
                !removed.was_already_released(targets[index].0),
                "a trampoline this batch actually freed is not 'already released'"
            );
        }
        assert!(
            removed.all_succeeded(),
            "an absent target is nothing-to-do, not a refusal: {:?}",
            removed.outcomes()
        );
        assert!(removed.was_already_released("absent.removal"));
        assert_eq!(removed.already_released(), ["absent.removal"]);
        assert_eq!(removed.failures().len(), 0);

        assert_eq!(
            unsafe { remove(targets[0].1) }
                .expect_err("the hook was removed")
                .minhook_status(),
            Some(MH_STATUS::MH_ERROR_NOT_CREATED)
        );
    }

    /// named in `already_released`.
    /// MUTATION RUN: the already-released arm of the queue phase disabled, so
    /// MUTATION RUN: `already_released.push(name)` removed from
    /// failed: removed.was_already_released("absent.remove")` -- and the
    #[test]
    fn a_release_batch_reports_nothing_to_do_as_success_and_still_names_it() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The release LEDGER is process-global and every one of these
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();

        let disabled = unsafe { disable_batch(&[("absent.disable", ABSENT_TARGET)]) };
        assert!(
            disabled.all_succeeded(),
            "a hook that does not exist cannot be left patched: {:?}",
            disabled.outcomes()
        );
        assert!(disabled.was_already_released("absent.disable"));
        assert_eq!(disabled.freezes(), 1);

        let removed = unsafe { remove_batch(&[("absent.remove", ABSENT_TARGET)]) };
        assert!(removed.all_succeeded(), "{:?}", removed.outcomes());
        assert!(removed.was_already_released("absent.remove"));

        // A target that really WAS released by the batch is not reported as
        // already-released, or the distinction would be worthless.
        let live = create_victims(1);
        let enabled = unsafe { enable_batch(&live) };
        assert!(enabled.all_succeeded(), "{:?}", enabled.outcomes());
        let report = unsafe { disable_batch(&live) };
        assert!(report.all_succeeded(), "{:?}", report.outcomes());
        assert!(
            !report.was_already_released("victim.0"),
            "this batch restored the bytes itself"
        );
        destroy_victims(&live);
    }

    /// Falsifiable: fails if a batched release stops writing its attempts into
    /// the release ledger, which is where a partial teardown names itself.
    /// `record_release_attempt` calls from `disable_batch`/`remove_batch`, or
    /// dropping `name_release_target` so the record cannot say WHICH hook.
    #[test]
    fn a_batched_release_writes_every_attempt_into_the_ledger_by_name() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();
        // instance first, and that removal is itself a release attempt this
        let targets = create_victims(2);
        crate::release_ledger::reset_for_tests();
        let installed = unsafe { enable_batch(&targets) };
        assert!(installed.all_succeeded(), "{:?}", installed.outcomes());

        let released = unsafe { disable_batch(&targets) };
        assert!(released.all_succeeded(), "{:?}", released.outcomes());
        let removed = unsafe { remove_batch(&targets) };
        assert!(removed.all_succeeded(), "{:?}", removed.outcomes());

        for (expected_name, target) in &targets {
            let written = crate::release_ledger::release_attempts_for(*target as usize);
            assert_eq!(
                written.len(),
                2,
                "{expected_name} must have one disable and one removal recorded: {written:?}"
            );
            assert_eq!(written[0].operation, crate::ReleaseOperation::Disable);
            assert_eq!(written[1].operation, crate::ReleaseOperation::Remove);
            for entry in &written {
                assert_eq!(
                    entry.name,
                    Some(*expected_name),
                    "every attempt must name its hook: {entry:?}"
                );
                assert!(entry.outcome.released(), "{entry:?}");
            }
        }
        assert!(
            crate::release_ledger::last_release_attempt(targets[0].1 as usize)
                .is_some_and(|outcome| outcome.released())
        );
        assert!(!crate::release_ledger::release_is_unproven(
            targets[0].1 as usize
        ));
    }

    /// Falsifiable: fails if a batched release stops restoring the original
    /// release path.
    #[test]
    fn a_batched_release_restores_every_target_it_was_given() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The release LEDGER is process-global and every one of these
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();
        let targets = create_victims(3);

        let installed = unsafe { enable_batch(&targets) };
        assert!(installed.all_succeeded(), "{:?}", installed.outcomes());
        for index in 0..3 {
            assert_eq!(call(index), 0xD0 + index as u64);
        }

        let released = unsafe { disable_batch(&targets) };
        assert!(released.all_succeeded(), "{:?}", released.outcomes());
        for index in 0..3 {
            VICTIM_CALLS[index].store(0, Ordering::Release);
            let answer = call(index);
            assert_ne!(answer, 0xD0 + index as u64, "victim.{index} still detoured");
            assert_eq!(
                VICTIM_CALLS[index].load(Ordering::Acquire),
                1,
                "victim.{index}'s own body must have run"
            );
        }

        destroy_victims(&targets);
    }
}
