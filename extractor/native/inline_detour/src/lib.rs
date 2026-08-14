//! The one way an inline detour is enabled in this process.
//!
//! # The invariant
//!
//! **No inline detour is ever enabled in a module that could still be
//! unloaded.**
//!
//! A detour body lives in this DLL. Once a target is patched, a game thread can
//! branch into that body and be preempted before it executes a single
//! instruction, and nothing observable distinguishes that thread from no
//! thread -- no counter reading, no flag inside the body, because the thread is
//! already committed. If the module is then unloaded, that thread resumes on
//! unmapped memory and the game faults. Worse, if a later injection reuses the
//! image base first, the thread runs the NEW mapping's detour against its
//! freshly zeroed state and returns without calling the engine's own function,
//! which changes what the game computes.
//!
//! Retaining a MinHook trampoline does not help with any of that: the
//! trampoline is a separate allocation, and the code the poised thread is
//! actually inside is ours.
//!
//! So [`enable`] pins the containing module BEFORE it patches anything, and
//! refuses to patch at all if the pin fails. `GET_MODULE_HANDLE_EX_FLAG_PIN`
//! makes the loader reference permanent: `FreeLibrary` on any other reference
//! cannot bring the count to zero, so the mapping survives for the life of the
//! process whatever the collector or the host subsequently do.
//!
//! # Why this is a crate and not a helper
//!
//! Twenty-two crates own inline detours here, and only ten of them route
//! through `production_hook_receipt`. An invariant enforced in the receipt path
//! would cover under a third of the exposure while reading as though it covered
//! all of it -- which is worse than not having it, because the gap is invisible.
//! This crate is the single choke point, it is cheap enough for every lane to
//! depend on, and `no_direct_minhook_enable_outside_this_crate` fails the suite
//! if any crate reaches past it.
//!
//! # What this does NOT establish
//!
//! It does not prove no thread is resident in a detour. Nothing available here
//! can; that is the premise the whole design starts from. What it does is make
//! residency SURVIVABLE instead of fatal -- the code stays mapped, so a thread
//! that resumes late runs the real body against real state.
//!
//! It says nothing about the target's bytes. A pinned module with a detour
//! still patched in is not a released hook; restoring the target is a separate
//! obligation each lane keeps, and the host's teardown proof checks it
//! independently.
//!
//! That the pinned mapping actually survives a `FreeLibrary` is a property of
//! the Windows loader, asserted here only as "the PIN flag was accepted". No
//! offline test in this crate can observe a real unload attempt; that needs a
//! live patched `hoi4.exe`.
//!
//! # Cost
//!
//! One `GetModuleHandleExW` on the first enable of the process, and one relaxed
//! atomic load per enable after that. `enable` is an install-time call; no
//! producer path reaches it. The permanent cost is the mapping itself, which is
//! the deliberate trade this module exists to make.

#[cfg(windows)]
mod freeze;

#[cfg(windows)]
pub mod release_ledger;

#[cfg(windows)]
mod windows_impl {
    use super::freeze::{freeze_other_threads, FreezeRefusal};
    use crate::release_ledger::ReleaseOperation;
    use core::ffi::c_void;
    use minhook::{MinHook, MH_STATUS};
    use std::fmt;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
    };

    /// Why an inline detour was not enabled.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum InlineDetourEnableError {
        /// The containing module could not be pinned, so nothing was patched.
        /// Carries `GetLastError` from the refused `GetModuleHandleExW`.
        ModulePinRefused(u32),
        /// The process could not be frozen, so nothing was patched. A target
        /// rewritten while a thread may be executing inside it is a torn
        /// instruction stream in that thread.
        FreezeRefused(FreezeRefusal),
        /// The pin and the freeze both succeeded and MinHook refused the patch.
        MinHook(MH_STATUS),
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
        /// The underlying MinHook status, when the refusal came from MinHook
        /// rather than from the pin or the freeze. Call sites that inspect
        /// specific statuses use this; call sites that only render the failure
        /// use `Display`.
        pub fn minhook_status(self) -> Option<MH_STATUS> {
            match self {
                Self::MinHook(status) => Some(status),
                Self::ModulePinRefused(_) | Self::FreezeRefused(_) => None,
            }
        }

        /// The freeze refusal, when that is what stopped the patch.
        pub fn freeze_refusal(self) -> Option<FreezeRefusal> {
            match self {
                Self::FreezeRefused(refusal) => Some(refusal),
                Self::ModulePinRefused(_) | Self::MinHook(_) => None,
            }
        }
    }

    /// Why an inline detour was not released.
    ///
    /// Separate from [`InlineDetourEnableError`] because the consequences
    /// differ: a refused enable leaves the target clean, whereas a refused
    /// release leaves the detour INSTALLED AND LIVE. That is the safe
    /// direction -- the module is pinned, so a thread inside the detour body
    /// still runs real code -- but the owning unit is no longer released and
    /// must report itself so.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum InlineDetourReleaseError {
        /// The process could not be frozen. NOTHING WAS UNPATCHED; the detour
        /// is retained, which is deliberate. Restoring the original bytes
        /// underneath a thread that may be executing them is the unsafe
        /// direction, and refusing keeps the process in the state it is
        /// already running in.
        FreezeRefused(FreezeRefusal),
        /// The freeze succeeded and MinHook refused.
        MinHook(MH_STATUS),
    }

    impl fmt::Display for InlineDetourReleaseError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::FreezeRefused(refusal) => write!(
                    f,
                    "inline detour retained rather than restored: {refusal}"
                ),
                Self::MinHook(status) => write!(f, "{status}"),
            }
        }
    }

    impl std::error::Error for InlineDetourReleaseError {}

    impl InlineDetourReleaseError {
        /// The underlying MinHook status, when MinHook is what refused.
        pub fn minhook_status(self) -> Option<MH_STATUS> {
            match self {
                Self::MinHook(status) => Some(status),
                Self::FreezeRefused(_) => None,
            }
        }

        /// True when this status is MinHook reporting the hook was already
        /// gone. Nothing was patched, nothing is retained, and callers that
        /// treat a release as idempotent stop here.
        pub fn already_released(self) -> bool {
            matches!(
                self,
                Self::MinHook(MH_STATUS::MH_ERROR_DISABLED)
                    | Self::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED)
            )
        }

        /// True when a live detour was left installed because the process
        /// could not be frozen. The owning unit is NOT released.
        pub fn detour_retained(self) -> bool {
            matches!(self, Self::FreezeRefused(_))
        }
    }

    static MODULE_PINNED: AtomicBool = AtomicBool::new(false);
    static ENABLED_DETOURS: AtomicU64 = AtomicU64::new(0);
    static RETAINED_DETOURS: AtomicU64 = AtomicU64::new(0);

    /// A function whose address is inside this module, used only to name the
    /// module to pin. `#[inline(never)]` so it cannot be folded away into a
    /// caller in some other module.
    #[inline(never)]
    extern "system" fn residency_anchor() {}

    /// Take a permanent loader reference on the module containing this code.
    ///
    /// Idempotent and racy-safe: two threads pinning concurrently both succeed,
    /// and the second `GET_MODULE_HANDLE_EX_FLAG_PIN` is a no-op the loader
    /// already satisfied. The flag is what matters, not the returned handle,
    /// which is deliberately discarded.
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

    /// Run `op` ONLY if `pin` succeeded, and never otherwise.
    ///
    /// Every public entry point funnels through here, because the ordering IS
    /// the invariant: patching before pinning leaves a window in which a detour
    /// is live in an unloadable module, and that window is what produced the
    /// recorded 0xc0000005 faults in an unloaded capture mapping.
    ///
    /// Both inputs are parameters so the ordering is testable. It is not
    /// otherwise: by the time any test runs, some earlier test has already
    /// pinned the process, so asserting "the module is pinned afterwards"
    /// cannot tell a pin-first build from a pin-second one. The property that
    /// actually discriminates is that a REFUSED pin means `op` never ran.
    /// `op`'s error is whatever it refuses with -- MinHook's status, or the
    /// freeze refusal for the entry points that hold a freeze across the
    /// patch -- so the ordering helper does not flatten a refused freeze into
    /// a MinHook status it did not come from.
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

    /// Enable an inline detour at `target`: pin this module, freeze the
    /// process, then patch.
    ///
    /// FAIL-CLOSED TWICE, and both orders are the whole point.
    ///
    /// If the module cannot be pinned, the target is not patched, so the state
    /// where a detour is live in an unloadable module never exists. Patching
    /// first and pinning second would leave exactly that window open.
    ///
    /// If the process cannot be frozen, the target is not patched either.
    /// MinHook suspends threads itself but ignores every failure to do so and
    /// rewrites the bytes regardless; a thread executing inside those bytes
    /// then resumes into a half-old, half-new instruction stream. Holding the
    /// freeze across the MinHook call is what makes MinHook's suspension
    /// succeed rather than be skipped.
    ///
    /// # Safety
    ///
    /// `target` must name a hook already created with MinHook, and the caller
    /// keeps every obligation `MH_EnableHook` has: the detour must remain a
    /// valid callable for as long as the patch is live.
    pub unsafe fn enable(target: *mut c_void) -> Result<(), InlineDetourEnableError> {
        run_pinned(pin_containing_module, || {
            let frozen = freeze_other_threads()?;
            // FROZEN WINDOW: nothing below this line until `drop(frozen)` may allocate or block.
            let outcome = unsafe { MinHook::enable_hook(target) };
            drop(frozen);
            outcome.map_err(InlineDetourEnableError::MinHook)
        })?;
        ENABLED_DETOURS.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    /// Take the permanent loader reference now, without enabling anything.
    ///
    /// For callers that pin at a different moment than they patch -- a capture
    /// that disarms its hooks at dwell end and keeps them mapped as
    /// pass-throughs, for instance. Fail-closed like [`enable`]: the flag is
    /// recorded only after the loader accepts the pin, never before it is
    /// attempted. Recording first and proceeding on failure is the exact
    /// inversion of this invariant, and it reads as pinned while the module is
    /// still unloadable.
    pub fn pin_module_now() -> Result<(), InlineDetourEnableError> {
        pin_containing_module().map_err(InlineDetourEnableError::ModulePinRefused)
    }

    /// Queue an inline detour for a batched enable, pinning this module first.
    ///
    /// Queuing does not patch, but it is the point of no return for a caller
    /// that will then apply the batch, and pinning here means a queue that is
    /// never applied still leaves the module retained -- the conservative
    /// direction.
    ///
    /// # Safety
    ///
    /// As [`enable`].
    pub unsafe fn queue_enable(target: *mut c_void) -> Result<(), InlineDetourEnableError> {
        run_pinned(pin_containing_module, || unsafe {
            MinHook::queue_enable_hook(target)
        })
    }

    /// Apply every queued enable, pinning this module first and patching under
    /// a held freeze.
    ///
    /// This is the call that actually patches a queued batch, so both orderings
    /// matter here exactly as they do in [`enable`]. Counted as ONE enable
    /// operation covering the whole batch: the batch size is MinHook's internal
    /// state and is not reported back.
    ///
    /// # Safety
    ///
    /// As [`enable`], for every target in the queue.
    pub unsafe fn apply_queued() -> Result<(), InlineDetourEnableError> {
        apply_queued_without_counting()?;
        ENABLED_DETOURS.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    /// [`apply_queued`] without the census bump, for the batch entry points
    /// that know their own cardinality and count each target individually.
    ///
    /// The census is what the teardown proof reports as the reason the module
    /// is retained, so counting a 12-target batch as one enable understates it
    /// by eleven. Splitting the counter out here is the whole of the
    /// difference; the pin, the freeze and the ordering are identical.
    fn apply_queued_without_counting() -> Result<(), InlineDetourEnableError> {
        run_pinned(pin_containing_module, || {
            let frozen = freeze_other_threads()?;
            // FROZEN WINDOW: nothing below this line until `drop(frozen)` may allocate or block.
            let outcome = unsafe { MinHook::apply_queued() };
            drop(frozen);
            outcome.map_err(InlineDetourEnableError::MinHook)
        })
    }


    /// One target's outcome inside a batch, named as the caller named it.
    pub type BatchOutcome<E> = (&'static str, Result<(), E>);

    /// What a batched enable did, target by target.
    ///
    /// The point of the type is that a batch NEVER collapses into a single
    /// verdict. `PINNED_PRODUCTION_HOOK_SPECS_V1` requires every one of its
    /// rows to install, and an operator reading a refused capture has to be
    /// told WHICH row is missing; a batch that reported only "the batch
    /// failed" would be strictly less usable than the per-hook calls it
    /// replaced, whatever it saved in freezes.
    #[derive(Debug)]
    pub struct BatchEnableReport {
        outcomes: Vec<BatchOutcome<InlineDetourEnableError>>,
        apply_refusal: Option<InlineDetourEnableError>,
        rolled_back: bool,
        freezes: u64,
    }

    impl BatchEnableReport {
        /// True when this batch undid itself: some target refused, so every
        /// target the batch had enabled was disabled again before the freeze
        /// was dropped, and NOTHING it was given is live.
        ///
        /// The per-target outcomes still say what each target's own enable
        /// answered -- that is what names the refuser -- so this is the flag
        /// that says an `Ok` outcome does not mean "currently patched".
        pub fn rolled_back(&self) -> bool {
            self.rolled_back
        }
    }

    /// What a batched release did, target by target. The release twin of
    /// [`BatchEnableReport`].
    #[derive(Debug)]
    pub struct BatchReleaseReport {
        outcomes: Vec<BatchOutcome<InlineDetourReleaseError>>,
        apply_refusal: Option<InlineDetourReleaseError>,
        already_released: Vec<&'static str>,
        freezes: u64,
    }

    impl BatchReleaseReport {
        /// The targets MinHook reported as ALREADY released -- disabled
        /// already, or never created at all.
        ///
        /// Their outcome is `Ok(())`, because the postcondition a release asks
        /// for holds: nothing is patched. That matches what every per-target
        /// teardown in this workspace did with `MH_ERROR_DISABLED` and
        /// `MH_ERROR_NOT_CREATED`, so batching does not tighten any lane into
        /// refusing a second, idempotent teardown.
        ///
        /// It is reported separately because two call sites legitimately care.
        /// A lane that tracks which of its own sites it has already removed
        /// treats an UNEXPECTED `MH_ERROR_NOT_CREATED` as a custody failure --
        /// somebody else removed its hook -- and needs to tell that from a
        /// removal it performed itself. Folding the status into `Ok` without
        /// exposing it here would erase exactly that distinction.
        pub fn already_released(&self) -> &[&'static str] {
            &self.already_released
        }

        /// Was this target one MinHook reported as already released?
        pub fn was_already_released(&self, name: &str) -> bool {
            self.already_released.iter().any(|entry| *entry == name)
        }
    }

    macro_rules! batch_report_accessors {
        ($report:ty, $error:ty) => {
            impl $report {
                /// The refusal `MH_ApplyQueued` returned for the batch as a
                /// whole, when it returned one.
                ///
                /// Kept even when every target ends up `Ok`, because that
                /// combination is a REAL event and not a non-event: the one
                /// freeze was refused and the targets were then driven
                /// individually. Dropping it would turn "the batch failed and
                /// the retry healed it" into a silent success, which is the
                /// same class of erasure a partial teardown commits when it
                /// says nothing.
                pub fn apply_refusal(&self) -> Option<$error> {
                    self.apply_refusal
                }

                /// Every target's outcome, in the order the caller listed them.
                pub fn outcomes(&self) -> &[BatchOutcome<$error>] {
                    &self.outcomes
                }

                /// How many process freezes this batch actually took. THE
                /// number this API exists to move: one for a batch that
                /// applied cleanly, and one per still-queued target only on
                /// the failure path that has to name them individually.
                pub fn freezes(&self) -> u64 {
                    self.freezes
                }

                /// The first refused target, named.
                pub fn first_failure(&self) -> Option<(&'static str, $error)> {
                    self.outcomes
                        .iter()
                        .find_map(|(name, result)| result.err().map(|error| (*name, error)))
                }

                /// Every refused target, named.
                pub fn failures(&self) -> Vec<(&'static str, $error)> {
                    self.outcomes
                        .iter()
                        .filter_map(|(name, result)| result.err().map(|error| (*name, error)))
                        .collect()
                }

                /// True when every target in the batch succeeded.
                pub fn all_succeeded(&self) -> bool {
                    self.outcomes.iter().all(|(_, result)| result.is_ok())
                }

                /// How many targets the batch covered.
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

    /// Enable every target under ONE process freeze, reporting each one
    /// separately.
    ///
    /// # Why this exists
    ///
    /// [`enable`] freezes the whole process per call, and a freeze is two
    /// system-wide thread-list walks plus a suspend/resume of every other
    /// thread, followed by MinHook's own third walk. The shipped roster is every
    /// non-`bootstrap.` entry of
    /// `compact_contract::passive_ready::PINNED_PRODUCTION_HOOK_SPECS_V1` (the
    /// two bootstrap hooks are IAT stores, not detours) -- it was 55 when this
    /// was written and moves whenever a domain is scheduled or descheduled --
    /// so the install path paid one freeze per detour and the teardown path
    /// two. This pays ONE, on every path including the failure path.
    ///
    /// # PRECONDITION: no target may already be enabled
    ///
    /// The caller guarantees that none of `targets` is currently patched in.
    /// Every production caller satisfies it by construction -- a lane creates
    /// its hooks and enables them in the same function, behind its own
    /// `INSTALLED` latch -- and a batch that lists one target twice is refused
    /// below.
    ///
    /// It is a PRECONDITION rather than a check because MinHook has no way to
    /// answer "is this hook enabled": every entry point that would tell you
    /// changes the answer. Three things depend on it and are wrong without it:
    /// the enabled-detour census would count a patch that already existed; the
    /// failure rollback would disable a detour some earlier owner installed;
    /// and a target's post-failure queue flag would be set from a state this
    /// call did not establish.
    ///
    /// # What is inside the freeze
    ///
    /// The QUEUE calls, `MH_ApplyQueued`, and -- only when that refuses -- one
    /// `MH_EnableHook` per queued target, plus the rollback and the queue-flag
    /// normalisation. All of them are field writes under MinHook's own
    /// critical section, or `VirtualProtect` + `memcpy` + a cache flush.
    /// Nothing allocates: the two buffers filled inside the window are
    /// reserved before it opens.
    ///
    /// # The lock this window does NOT introduce
    ///
    /// Every MinHook entry point taken here enters MinHook's process-global
    /// spin lock, so a thread suspended while holding it would spin this
    /// thread forever with the whole game frozen. That hazard is NOT created
    /// by batching and is not made likelier by it: the single-target
    /// [`enable`], [`disable`] and [`remove`] already call MinHook inside
    /// their own freeze, and whether some thread holds the lock is decided at
    /// the instant the freeze is taken, not by how long it is then held.
    /// Serialising every MinHook caller in the process before any freeze is a
    /// change to the freeze design itself and belongs with it.
    ///
    /// Queueing INSIDE the freeze is what makes the queue leak-proof. MinHook's
    /// queue is process-global and `MH_ApplyQueued` applies every divergent
    /// entry it finds, so a target left queued is a patch some later,
    /// unrelated batch performs inside a freeze nobody accounted for. Queue
    /// first and the freeze can still be refused AFTER the queue is dirty,
    /// with no way to know what to restore it to. Queue second and a refused
    /// freeze means nothing was ever queued.
    ///
    /// # Why the completion pass is INSIDE the same freeze
    ///
    /// `MH_ApplyQueued` walks MinHook's own hook table in CREATION order, not
    /// in the order of this slice, and stops at the first target it cannot
    /// patch. A partial apply can therefore leave an outer producer live while
    /// the nested observer that must precede it is still dark -- exactly the
    /// state the callers' reversed slices exist to forbid. Finishing under a
    /// second freeze would let game threads run in that state.
    ///
    /// # Atomic in EFFECT, per-target in REPORTING
    ///
    /// If ANY target ends refused -- at queue time or at patch time -- every
    /// target this batch enabled is disabled again before the freeze is
    /// dropped, and `rolled_back` says so. The per-target loops this replaced
    /// returned on the first refusal, so a later target was never enabled and
    /// the nested-before-outer orders could not be observed inverted; the
    /// completion pass CAN produce that inversion, so it is undone rather than
    /// published.
    ///
    /// A rollback that itself fails is not hidden: `rolled_back` stays false,
    /// the target keeps an `Ok` outcome because its own enable succeeded, and
    /// it is still counted and still queued as enabled -- because it IS.
    ///
    /// The REPORT stays per target -- which hook failed, with which status --
    /// because that is the fidelity a whole-roster census needs.
    ///
    /// # Safety
    ///
    /// As [`enable`], for every target, plus the precondition above.
    pub unsafe fn enable_batch(
        targets: &[(&'static str, *mut c_void)],
    ) -> BatchEnableReport {
        let freezes_before = super::freeze::freeze_attempts_on_this_thread();
        let mut outcomes: Vec<BatchOutcome<InlineDetourEnableError>> =
            Vec::with_capacity(targets.len());
        if targets.is_empty() {
            return BatchEnableReport {
                outcomes,
                apply_refusal: None,
                rolled_back: false,
                freezes: 0,
            };
        }
        if first_duplicate_target(targets).is_some() {
            // A target listed twice would be counted twice in the enabled
            // census for one physical patch, its second entry's outcome would
            // describe the first entry's work, and it would break the
            // not-already-enabled precondition against itself. Refuse the
            // roster rather than report one that does not exist.
            for &(name, _) in targets {
                outcomes.push((
                    name,
                    Err(InlineDetourEnableError::MinHook(
                        MH_STATUS::MH_ERROR_ALREADY_CREATED,
                    )),
                ));
            }
            return BatchEnableReport {
                outcomes,
                apply_refusal: None,
                rolled_back: false,
                freezes: 0,
            };
        }
        // The module is pinned BEFORE anything is patched, exactly as
        // [`enable`] does it, and a refused pin means nothing is attempted.
        if let Err(pin) = pin_module_now() {
            for &(name, _) in targets {
                outcomes.push((name, Err(pin)));
            }
            return BatchEnableReport {
                outcomes,
                apply_refusal: Some(pin),
                rolled_back: false,
                freezes: 0,
            };
        }
        // Both buffers are reserved here, before the freeze exists. Filling
        // them inside the window must not call the allocator: a suspended
        // thread holding the allocator's lock makes that a permanent deadlock.
        let mut statuses: Vec<Result<(), MH_STATUS>> = Vec::with_capacity(targets.len());
        let mut rollback_failed: Vec<bool> = Vec::with_capacity(targets.len());
        let mut apply_refusal = None;
        let rolled_back;
        let mut attempted_rollback = false;
        match freeze_other_threads() {
            Ok(frozen) => {
                // FROZEN WINDOW: nothing below this line until `drop(frozen)` may allocate or block.
                for &(_, target) in targets {
                    statuses.push(unsafe { MinHook::queue_enable_hook(target) });
                }
                let queued_any = statuses.iter().any(|status| status.is_ok());
                if queued_any {
                    if let Err(apply) = unsafe { MinHook::apply_queued() } {
                        apply_refusal =
                            Some(InlineDetourEnableError::MinHook(apply));
                        for (index, &(_, target)) in targets.iter().enumerate() {
                            if statuses[index].is_ok() {
                                statuses[index] = match unsafe { MinHook::enable_hook(target) } {
                                    // The apply reached it before it stopped.
                                    // Under the not-already-enabled
                                    // precondition, that is this batch's own
                                    // patch.
                                    Err(MH_STATUS::MH_ERROR_ENABLED) => Ok(()),
                                    other => other,
                                };
                            }
                        }
                    }
                }
                // Anything refused -- at queue time or at patch time -- undoes
                // the whole batch, so no game thread sees a partial set in an
                // order MinHook's creation table chose.
                let any_refusal = statuses.iter().any(|status| status.is_err());
                for (index, &(_, target)) in targets.iter().enumerate() {
                    let live = statuses[index].is_ok() && queued_any;
                    let undo = live && any_refusal;
                    let mut failed_undo = false;
                    if undo {
                        attempted_rollback = true;
                        failed_undo = unsafe { MinHook::disable_hook(target) }.is_err();
                    }
                    rollback_failed.push(failed_undo);
                    // Leave `queueEnable` equal to `isEnabled` for every
                    // target, so this batch cannot leave a divergent entry for
                    // an unrelated `MH_ApplyQueued` to act on.
                    //
                    // BE EXACT ABOUT WHEN THIS IS LOAD-BEARING, because a
                    // mutation that deletes it SURVIVES the offline suite and
                    // that is not the same as it being dead code.
                    // `EnableHookLL` sets `isEnabled` and `queueEnable`
                    // together, so every target the apply patched -- and every
                    // target the rollback then unpatched -- is already
                    // equalised, and that is the whole of what an offline test
                    // can reach. What is NOT reachable offline is a target
                    // that was queued and never patched, which is exactly what
                    // a refused `MH_ApplyQueued` leaves behind: `queueEnable`
                    // true, `isEnabled` false, divergent, waiting for somebody
                    // else's apply to patch it inside a freeze nobody
                    // accounted for. Nothing available here can make MinHook
                    // refuse an apply it would otherwise accept, so this line
                    // stands on MinHook's own source and on review rather than
                    // on a test.
                    let enabled_now = live && (!undo || failed_undo);
                    if !enabled_now {
                        let _ = unsafe { MinHook::queue_disable_hook(target) };
                    }
                }
                // `rolled_back` is the CLAIM that nothing is live, so it holds
                // only when every undo this batch attempted also succeeded.
                rolled_back =
                    attempted_rollback && !rollback_failed.iter().any(|failed| *failed);
                drop(frozen);
            }
            Err(refusal) => {
                // Nothing was queued and nothing was patched: the queue calls
                // are inside the window this refusal prevented. There is no
                // residue to clean up, which is the whole reason the queue is
                // in there.
                let refused = InlineDetourEnableError::FreezeRefused(refusal);
                apply_refusal = Some(refused);
                for &(name, _) in targets {
                    outcomes.push((name, Err(refused)));
                }
                return BatchEnableReport {
                    outcomes,
                    apply_refusal,
                    rolled_back: false,
                    freezes: super::freeze::freeze_attempts_on_this_thread()
                        .saturating_sub(freezes_before),
                };
            }
        }
        let mut enabled = 0u64;
        for (index, &(name, _)) in targets.iter().enumerate() {
            let outcome = statuses[index].map_err(InlineDetourEnableError::MinHook);
            // Counted when the target is LIVE now, which after a rollback is
            // only the ones the rollback could not take back down. The
            // completion pass calls MinHook directly and does not bump the
            // census itself, so this is the only place any of them is counted
            // and none is counted twice.
            // Counted only when the target is LIVE now: its own enable
            // succeeded AND no rollback took it back down. Decided per
            // POSITION, because a mixed rollback -- some undos accepted, one
            // refused -- leaves only the refused one live, and a batch-wide
            // flag would count the ones that came back down with it.
            if outcome.is_ok() && (!attempted_rollback || rollback_failed[index]) {
                enabled += 1;
            }
            outcomes.push((name, outcome));
        }
        ENABLED_DETOURS.fetch_add(enabled, Ordering::AcqRel);
        BatchEnableReport {
            outcomes,
            apply_refusal,
            rolled_back,
            freezes: super::freeze::freeze_attempts_on_this_thread().saturating_sub(freezes_before),
        }
    }

    /// The first target that appears more than once in `targets`.
    fn first_duplicate_target(targets: &[(&'static str, *mut c_void)]) -> Option<usize> {
        for (index, &(_, target)) in targets.iter().enumerate() {
            if targets[..index].iter().any(|&(_, earlier)| earlier == target) {
                return Some(index);
            }
        }
        None
    }

    /// Release every target under ONE process freeze, reporting each one
    /// separately. The release twin of [`enable_batch`]; see it for what is
    /// inside the freeze, why the queue calls are in there, and why the
    /// completion pass is too.
    ///
    /// FAIL-CLOSED in [`disable`]'s direction: a batch whose freeze was
    /// refused queued nothing and unpatched nothing, and every target is
    /// reported `detour_retained`. The retained census moves by the number of
    /// targets, not by one, because that is how many detours are still live.
    ///
    /// UNLIKE [`enable_batch`], a partial failure is NOT undone. Re-enabling a
    /// detour the caller asked to release would fight the teardown, and the
    /// per-target loops this replaces left partial disables too -- the lanes
    /// that care restore explicitly, from the per-target outcomes reported
    /// here.
    ///
    /// MinHook's already-disabled and never-created statuses are reported as
    /// SUCCESS and named in `already_released`: nothing is patched, which is
    /// the postcondition a release asks for, and it is what every per-target
    /// teardown this replaced already did.
    ///
    /// This does NOT remove hooks or free trampolines; [`remove_batch`] does
    /// that, because freeing a trampoline is a different obligation from
    /// restoring bytes.
    ///
    /// # Safety
    ///
    /// As [`disable`], for every target.
    pub unsafe fn disable_batch(
        targets: &[(&'static str, *mut c_void)],
    ) -> BatchReleaseReport {
        let freezes_before = super::freeze::freeze_attempts_on_this_thread();
        let mut outcomes: Vec<BatchOutcome<InlineDetourReleaseError>> =
            Vec::with_capacity(targets.len());
        let mut already_released: Vec<&'static str> = Vec::new();
        if targets.is_empty() {
            return BatchReleaseReport {
                outcomes,
                apply_refusal: None,
                already_released,
                freezes: 0,
            };
        }
        for &(name, target) in targets {
            // Named before anything is attempted, so every attempt below is
            // reported by the caller's name for the hook rather than by a bare
            // address.
            crate::release_ledger::name_release_target(target as usize, name);
        }
        let mut statuses: Vec<Result<(), MH_STATUS>> = Vec::with_capacity(targets.len());
        let mut apply_refusal = None;
        let frozen = match freeze_other_threads() {
            Ok(frozen) => frozen,
            Err(refusal) => {
                // Nothing queued, nothing unpatched: every detour is still
                // live, which is the one fact a teardown may not misreport.
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
                    freezes: super::freeze::freeze_attempts_on_this_thread()
                        .saturating_sub(freezes_before),
                };
            }
        };
        // FROZEN WINDOW: nothing below this line until `drop(frozen)` may allocate or block.
        for &(_, target) in targets {
            statuses.push(unsafe { MinHook::queue_disable_hook(target) });
        }
        if statuses.iter().any(|status| status.is_ok()) {
            if let Err(apply) = unsafe { MinHook::apply_queued() } {
                apply_refusal = Some(InlineDetourReleaseError::MinHook(apply));
                // STOP AT THE FIRST REFUSAL, IN SLICE ORDER, and put back
                // whatever the apply released after it.
                //
                // The slice order is load-bearing on the way out exactly as it
                // is on the way in: the selected-designer teardown takes the
                // callbacks that READ joins down before the observers that
                // would catch a free, and the peace-conference teardown does
                // the same. `MH_ApplyQueued` walks MinHook's CREATION order,
                // so it can release a later site while an earlier one is still
                // live. Carrying on past a refusal would finish that inversion
                // and drop the freeze on it; the per-target loops this
                // replaces stopped, and so does this.
                let mut refused_at = None;
                for (index, &(_, target)) in targets.iter().enumerate() {
                    if !statuses[index].is_ok() {
                        continue;
                    }
                    if refused_at.is_some() {
                        // Behind the refusal: restore it if the apply took it
                        // down. `MH_ERROR_ENABLED` means it never did.
                        let restored = unsafe { MinHook::enable_hook(target) };
                        statuses[index] = match restored {
                            Ok(()) | Err(MH_STATUS::MH_ERROR_ENABLED) => Err(apply),
                            Err(status) => Err(status),
                        };
                        continue;
                    }
                    statuses[index] = unsafe { MinHook::disable_hook(target) };
                    if let Err(status) = statuses[index] {
                        if !InlineDetourReleaseError::MinHook(status).already_released() {
                            refused_at = Some(index);
                        }
                    }
                }
            }
        }
        // Leave `queueEnable` equal to `isEnabled` for every target this batch
        // could not release, so no divergent entry is left for an unrelated
        // `MH_ApplyQueued`. A target whose disable was refused is still
        // enabled; `MH_ERROR_DISABLED`/`MH_ERROR_NOT_CREATED` are not
        // refusals, and `EnableHookLL` already equalised the ones it disabled.
        for (index, &(_, target)) in targets.iter().enumerate() {
            let still_enabled = match statuses[index] {
                Ok(()) => false,
                Err(status) => !InlineDetourReleaseError::MinHook(status).already_released(),
            };
            if still_enabled {
                let _ = unsafe { MinHook::queue_enable_hook(target) };
            }
        }
        drop(frozen);
        let mut retained = 0u64;
        for (index, &(name, target)) in targets.iter().enumerate() {
            let raw = statuses[index].map_err(InlineDetourReleaseError::MinHook);
            // The RAW status is what the ledger records: it is the account of
            // what MinHook was asked and what it answered, per target and not
            // per batch.
            crate::release_ledger::record_release_attempt(
                target as usize,
                ReleaseOperation::Disable,
                raw,
            );
            match raw {
                Ok(()) => outcomes.push((name, Ok(()))),
                Err(error) if error.already_released() => {
                    already_released.push(name);
                    outcomes.push((name, Ok(())));
                }
                Err(error) => {
                    // EVERY refusal here leaves the detour patched into the
                    // running game -- a refused freeze did not unpatch it, and
                    // a MinHook refusal did not either. `detour_retained` is
                    // narrower than that (it names only the freeze refusal),
                    // so counting on it alone under-reported live detours,
                    // which is the one direction this census may not err in.
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
            freezes: super::freeze::freeze_attempts_on_this_thread().saturating_sub(freezes_before),
        }
    }

    /// Remove every target under ONE process freeze, reporting each one
    /// separately.
    ///
    /// MinHook has no queue for removal, so unlike [`enable_batch`] and
    /// [`disable_batch`] this is a LOOP of `MH_RemoveHook` calls with the
    /// freeze held across all of them rather than one applied batch. The
    /// saving is the same and the protection is unchanged: the guard held here
    /// is the same guard each single [`remove`] takes, held once instead of
    /// once per target.
    ///
    /// Holding it across N MinHook calls does not push any thread past
    /// `MAXIMUM_SUSPEND_COUNT`. `MH_RemoveHook` suspends and RESUMES inside
    /// each call, so the depth never exceeds ours plus the one
    /// `SUSPENSIONS_STILL_TO_COME` already accounts for, however many calls
    /// follow.
    ///
    /// FAIL-CLOSED like [`remove`]: a refused freeze removes NOTHING, every
    /// target is reported retained, and no trampoline is freed.
    ///
    /// # Safety
    ///
    /// As [`remove`], for every target.
    pub unsafe fn remove_batch(
        targets: &[(&'static str, *mut c_void)],
    ) -> BatchReleaseReport {
        let freezes_before = super::freeze::freeze_attempts_on_this_thread();
        if targets.is_empty() {
            return BatchReleaseReport {
                outcomes: Vec::new(),
                apply_refusal: None,
                    already_released: Vec::new(),
                freezes: 0,
            };
        }
        for &(name, target) in targets {
            crate::release_ledger::name_release_target(target as usize, name);
        }
        // Filling this inside the freeze must not ALLOCATE, so the capacity is
        // taken here, before the freeze exists. `.collect()` inside the window
        // would have called the allocator with every other thread suspended --
        // and a suspended thread holding the allocator's lock is a permanent
        // deadlock, not a slow path.
        let mut statuses: Vec<Result<(), InlineDetourReleaseError>> =
            Vec::with_capacity(targets.len());
        let frozen = match freeze_other_threads() {
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
                    freezes: super::freeze::freeze_attempts_on_this_thread()
                        .saturating_sub(freezes_before),
                };
            }
        };
        // FROZEN WINDOW: nothing below this line until `drop(frozen)` may allocate or block.
        for &(_, target) in targets {
            statuses.push(
                unsafe { MinHook::remove_hook(target) }.map_err(InlineDetourReleaseError::MinHook),
            );
        }
        drop(frozen);
        let mut already_released = Vec::new();
        let outcomes = targets
            .iter()
            .zip(statuses)
            .map(|(&(name, target), status)| {
                // The RAW status is what the ledger records: it is the account
                // of what MinHook was asked and what it answered.
                crate::release_ledger::record_release_attempt(
                    target as usize,
                    ReleaseOperation::Remove,
                    status,
                );
                // `MH_ERROR_NOT_CREATED` means there is no hook and no
                // trampoline, so the removal's postcondition already holds --
                // the same reading every per-target teardown here already
                // used. It is still named in `already_released` for the two
                // lanes that must tell a removal they performed from one they
                // did not.
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
            freezes: super::freeze::freeze_attempts_on_this_thread().saturating_sub(freezes_before),
        }
    }






    /// Disable the inline detour at `target`, restoring the original bytes,
    /// under a held process freeze.
    ///
    /// FAIL-CLOSED: if the process cannot be frozen this does NOT unpatch. The
    /// detour stays installed and `Err(FreezeRefused)` says so. That is the
    /// safe direction and it is the direction this workspace already relies
    /// on: the containing module is pinned, so a thread poised in the detour
    /// body keeps running real code for the life of the process, whereas
    /// restoring a target underneath a running thread tears its instruction
    /// stream.
    ///
    /// The trampoline is untouched, exactly as `MH_DisableHook` leaves it.
    ///
    /// # Safety
    ///
    /// As `MH_DisableHook`. The detour must remain a valid callable until this
    /// returns `Ok`, and must remain valid FOREVER if it returns
    /// [`InlineDetourReleaseError::detour_retained`].
    pub unsafe fn disable(target: *mut c_void) -> Result<(), InlineDetourReleaseError> {
        let frozen = match freeze_other_threads() {
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
        // FROZEN WINDOW: nothing below this line until `drop(frozen)` may allocate or block.
        let outcome = unsafe { MinHook::disable_hook(target) };
        drop(frozen);
        record_release(
            target,
            ReleaseOperation::Disable,
            outcome.map_err(InlineDetourReleaseError::MinHook),
        )
    }

    /// Record one release attempt in the process-wide ledger and hand the
    /// result straight back.
    ///
    /// A pass-through on purpose: recording must not be able to change what a
    /// caller sees, or the ledger would be a second source of truth about
    /// whether a detour came off rather than a record of what was tried.
    ///
    /// Called only AFTER the `ProcessFreeze` has been dropped. It takes a
    /// mutex, and taking a mutex with every other thread in the process
    /// suspended is the one way this crate could hang the game.
    fn record_release(
        target: *mut c_void,
        operation: ReleaseOperation,
        result: Result<(), InlineDetourReleaseError>,
    ) -> Result<(), InlineDetourReleaseError> {
        crate::release_ledger::record_release_attempt(target as usize, operation, result);
        result
    }

    /// Remove the hook at `target` under a held process freeze, freeing its
    /// trampoline.
    ///
    /// FAIL-CLOSED in the same direction as [`disable`], and for a sharper
    /// reason. `MH_RemoveHook` restores the target's bytes if the hook is
    /// enabled AND THEN FREES THE TRAMPOLINE. Letting that run without a
    /// freeze risks both halves at once: a torn instruction stream at the
    /// target, and a freed trampoline underneath a thread that is inside it.
    ///
    /// # Safety
    ///
    /// As `MH_RemoveHook`. On a retained refusal nothing was freed and nothing
    /// was restored, so the caller's trampoline and detour must both stay
    /// valid for the life of the process.
    pub unsafe fn remove(target: *mut c_void) -> Result<(), InlineDetourReleaseError> {
        let frozen = match freeze_other_threads() {
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
        // FROZEN WINDOW: nothing below this line until `drop(frozen)` may allocate or block.
        let outcome = unsafe { MinHook::remove_hook(target) };
        drop(frozen);
        record_release(
            target,
            ReleaseOperation::Remove,
            outcome.map_err(InlineDetourReleaseError::MinHook),
        )
    }

    /// How many releases were refused, leaving a live detour installed.
    ///
    /// Non-zero means some unit believes it released a hook that is in fact
    /// still patched into the game. The teardown proof reads this.
    pub fn retained_detour_count() -> u64 {
        RETAINED_DETOURS.load(Ordering::Acquire)
    }

    /// True once this module holds a permanent loader reference because an
    /// inline detour was enabled through [`enable`].
    ///
    /// The collector's unload path consults this. It is deliberately one-way:
    /// there is no clearing it, because a cleared flag would describe a module
    /// that is safe to unload while a thread may still be inside a detour body.
    pub fn module_pinned_against_unload() -> bool {
        MODULE_PINNED.load(Ordering::Acquire)
    }

    /// How many inline detours this module has enabled. Diagnostic; the census
    /// the teardown proof reports alongside the pin.
    pub fn enabled_detour_count() -> u64 {
        ENABLED_DETOURS.load(Ordering::Acquire)
    }
}

#[cfg(windows)]
pub use release_ledger::{
    any_release_needed_a_retry, last_release_attempt, name_release_target,
    release_attempt_history, release_attempts_dropped_for_capacity, release_is_unproven,
    render_release_attempt_history, render_release_attempts_for,
    targets_with_a_refused_last_release_attempt, unproven_releases_among, ReleaseAttempt,
    ReleaseAttemptOutcome, ReleaseOperation,
};

#[cfg(windows)]
pub use freeze::{
    freeze_attempts, freeze_attempts_on_this_thread, freeze_passes_spent, freeze_process,
    freeze_refusals, open_failures_excused_as_no_longer_ours,
    suspend_failures_deferred_to_a_later_pass, suspend_failures_excused_as_terminated,
    FreezeRefusal, ProcessFreeze,
};

#[cfg(windows)]
pub(crate) use windows_impl::run_pinned;
#[cfg(windows)]
pub use windows_impl::{
    apply_queued, disable, disable_batch, enable, enable_batch, enabled_detour_count,
    remove_batch,
    module_pinned_against_unload, pin_module_now, queue_enable, remove, retained_detour_count,
    BatchEnableReport, BatchOutcome, BatchReleaseReport, InlineDetourEnableError,
    InlineDetourReleaseError,
};

/// The banned MinHook entry points, spelled as they appear in a call.
///
/// EVERY entry point that rewrites a live target's bytes is here, not just the
/// ones that install. `MH_DisableHook` and `MH_RemoveHook` route through the
/// same `Freeze()`/`EnableHookLL` pair as `MH_EnableHook` and fail open on a
/// refused suspension in exactly the same way; `MH_RemoveHook` additionally
/// frees the trampoline. `MH_CreateHook` is deliberately absent: it reads the
/// target and allocates, and patches nothing.
///
/// Kept as separate fragments and joined at runtime so that this list does not
/// itself match the scan, which would make the guard trivially self-satisfying.
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

/// Collapse whitespace around `::` so `MinHook :: enable_hook` cannot slip past
/// a substring match, and normalise any local alias of `MinHook` back to the
/// real name so `use minhook::MinHook as MH; MH::enable_hook(..)` is caught too.
///
/// Deliberately crude rather than a real parser: a parser would be a second
/// thing to get wrong, and every bypass this normalisation does not catch --
/// a macro that assembles the path, the raw `minhook::ffi` symbol, a different
/// hooking library -- requires an edit that is visible in review. What is
/// closed here is the reachable-today path of writing the call.
#[cfg(test)]
fn normalize_for_scan(text: &str) -> String {
    let mut out = text.to_string();
    while out.contains(" ::") || out.contains(":: ") {
        out = out.replace(" ::", "::").replace(":: ", "::");
    }
    let mut aliases = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        // Both the bare form and the grouped form, which is the idiomatic one
        // and was a live bypass: `use minhook::{MinHook as MH, MH_STATUS};`
        // aliases just as effectively as `use minhook::MinHook as MH;` and
        // needs no extra dependency, since every crate here already has MinHook.
        for marker in ["use minhook::MinHook as ", "MinHook as "] {
            if !line.starts_with("use ") && marker.starts_with("use ") {
                continue;
            }
            if marker == "MinHook as " && !line.starts_with("use minhook::") {
                continue;
            }
            let Some(at) = line.find(marker) else { continue };
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

/// Every line of `text` that enables an inline detour without the wrapper.
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

/// The Win32 entry points a PRIVATE in-process thread suspender needs, spelled
/// as they appear in a call or an `extern` declaration.
///
/// Four units in this workspace each hand-rolled one of these, and each got the
/// same two things wrong: a broken `Thread32Next` read as the end of the thread
/// list, and a single enumeration pass that a thread created afterwards walks
/// straight through. They are gone; this is what keeps them gone.
///
/// SCOPE, stated because it is narrower than "no thread suspension anywhere".
/// `ResumeThread` and `CreateToolhelp32Snapshot` are deliberately ABSENT:
/// `recorder_host` resumes the main thread of a process it created suspended,
/// and enumerates PROCESSES, and neither reaches a thread of the collector's own
/// process. What is banned is the ability to enumerate this process's threads
/// and stop them -- without `Thread32First`/`Thread32Next` there is no thread
/// list to walk, and without `SuspendThread` there is nothing to stop it with.
///
/// NOT COVERED, in those words: `NtGetNextThread`, a raw syscall, a suspender
/// written in a `.c` file, or a fourth-party crate. Each needs an edit that is
/// visible in review, which is the same bar the MinHook ban above sets.
///
/// Kept as separate fragments and joined at runtime so this list does not
/// itself match the scan.
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
];

/// Strip `//` line comments so a prose reference to a banned name is not a
/// violation.
///
/// Deliberately crude, and deliberately one-directional: it removes comment
/// text, so it can only ever make the scan MISS a call hidden in something that
/// looks like a comment -- which is not writable Rust -- and can never make it
/// invent one. Block comments are not stripped, so a banned call commented out
/// with `/* */` still trips the scan; that is the safe direction.
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

/// Every line of `text` that reaches for a private thread suspender.
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
        scan_source_for_direct_enable, scan_source_for_private_suspender,
        BANNED_CALL_FRAGMENTS, BANNED_SUSPENDER_FRAGMENTS,
    };
    use std::path::{Path, PathBuf};

    /// Every `.rs` file in every native workspace that can inject into the
    /// game, except this crate's own.
    ///
    /// EXPLICITLY MORE THAN ONE WORKSPACE, and it stays that way now that the
    /// crate that motivated it has moved. `terrain_capture` is a separately
    /// injected DLL with its own D3D11 detours; it used to live in
    /// `replay_interface/native`, and a scan that stopped at the extractor
    /// boundary would have declared the invariant enforced while that mapping
    /// patched before pinning. It now sits in `extractor/native` with the rest
    /// of the injected code, but the second root is kept deliberately: an
    /// invariant that stops at a workspace boundary is not one, and nothing
    /// stops a future injected DLL appearing in the replay tree again.
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

    /// THE ENFORCEMENT. Fails if any crate patches a target without going
    /// through [`super::enable`].
    ///
    /// A detour enabled outside this crate is a detour enabled in a module that
    /// was never pinned, so the collector can later unload itself out from under
    /// a thread poised in that detour's body. Twelve of the twenty-two crates
    /// that own detours have no other reason to depend on the receipt crate, so
    /// there is no smaller choke point that covers them.
    ///
    /// Deliberately a source scan: MinHook is a third-party crate any workspace
    /// member can name directly, so nothing in Rust's visibility rules can stop
    /// a new lane calling it. The tree already uses source scanning for ordering
    /// laws it cannot otherwise state (`division_combat_stats_live`).
    ///
    /// SCOPE: whatever [`workspace_sources`] walks, which is BOTH native
    /// workspaces. An earlier version of this comment claimed the scan stopped
    /// at `extractor/native` and that `terrain_capture` was exempt; that was
    /// never what the code did — `workspace_sources` has always included
    /// `replay_interface/native`, and `terrain_capture` has always routed its
    /// D3D11 detours through `inline_detour::queue_enable`. The crate now lives
    /// at `extractor/native/terrain_capture`, so the point is moot for it, but
    /// the comment was describing a hole that did not exist and would have
    /// licensed one that did.
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

    /// THE GUARD ON THE GUARD, and it now runs the real scanner.
    ///
    /// The previous version asserted that a planted string contained the banned
    /// substring, which is a property of the string and not of the scanner:
    /// deleting the violation-collection branch left it green. This feeds
    /// planted sources through `scan_source_for_direct_enable` itself.
    ///
    /// The alias and whitespace cases are here because they were real bypasses:
    /// `use minhook::MinHook as MH; MH::enable_hook(t)` and
    /// `MinHook :: enable_hook(t)` both defeated the plain substring match the
    /// first version used, and every lane already depends on MinHook so neither
    /// needs a new dependency to reach.
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

        let aliased = format!(
            "use minhook::MinHook as MH;\nfn f() {{ unsafe {{ MH::{call}t) }}; }}"
        );
        assert_eq!(
            scan_source_for_direct_enable(&aliased),
            vec![2],
            "an aliased import must not defeat the scan"
        );

        // The GROUPED import, which is the idiomatic form and was a live
        // bypass: the bare-form check alone missed it entirely.
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

    /// Falsifiable: fails if any entry point that rewrites a live target's
    /// bytes drops out of the ban list.
    ///
    /// The list originally covered only the three that install. `MH_DisableHook`
    /// and `MH_RemoveHook` reach the identical `Freeze()`/`EnableHookLL` pair
    /// and fail open on a refused suspension identically, so a lane calling
    /// them directly bypasses the freeze exactly as a lane calling
    /// `MH_EnableHook` directly bypassed the pin. Each planted line here goes
    /// through the real scanner, so deleting a fragment turns this red.
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

    /// Falsifiable: fails if `create_hook` is ever added to the ban list.
    ///
    /// It is the one MinHook entry point lanes legitimately call directly:
    /// it reads the target and allocates a trampoline, and patches nothing, so
    /// no thread can be inside bytes it rewrites. Banning it would push every
    /// lane's hook creation through a wrapper that has nothing to add, and the
    /// resulting churn would obscure the ban that matters.
    #[test]
    fn creating_a_hook_is_not_banned_because_it_rewrites_nothing() {
        let planted = "fn f() { unsafe { MinHook::create_hook(t, d) }; }";
        assert!(
            scan_source_for_direct_enable(planted).is_empty(),
            "MinHook::create_hook patches no live bytes and must stay callable"
        );
    }

    /// THE ENFORCEMENT for the freeze, and the twin of the MinHook ban above.
    /// Fails if any crate suspends this process's threads itself.
    ///
    /// A private suspender is not merely duplication. All four that existed
    /// certified a TRUNCATED thread list -- they read any `Thread32Next` stop as
    /// the end of the list -- and all four enumerated exactly once, so a thread
    /// created after the snapshot was patched around while it ran. Both defects
    /// are invisible at the call site and identical in every copy, which is what
    /// makes a choke point the fix rather than four fixes.
    ///
    /// The two mutations this is here to catch, restated as what a future lane
    /// would actually write: reintroducing an `extern` block with
    /// `SuspendThread`/`Thread32Next` in a lane, or a fresh enumeration loop
    /// beside the shared freeze. Either lands here by file and line.
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

    /// THE GUARD ON THAT GUARD: every planted line goes through the real
    /// scanner, so deleting a fragment or the violation-collection branch turns
    /// this red rather than leaving a scan that finds nothing.
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

    /// Falsifiable: fails if any of the four migrated units grows a private
    /// suspender again, named one by one so the failure says WHICH unit
    /// regressed instead of only that some file did.
    ///
    /// The blanket scan above already covers every crate. This exists because a
    /// blanket scan cannot say the four files it was written for are still
    /// being scanned -- rename one, or move its patcher into a new module, and
    /// the blanket test stays green while the unit it was written for is no
    /// longer named anywhere.
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

    /// The walk must actually reach the crates that own detours; if it ever
    /// stopped, every future violation would pass unnoticed.
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

    /// Several tests here read a delta on the process-global enable counter, so
    /// they are serialized rather than left to interleave. Without this,
    /// `the_queued_path_pins_before_it_applies` could apply an empty queue --
    /// which succeeds, and counts -- inside another test's delta window.
    static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Falsifiable: fails if the pin stops happening, or stops being recorded.
    ///
    /// The pin is what makes every other retention claim in this workspace
    /// unconditional rather than "while the DLL happens to still be mapped", so
    /// a build that dropped it would silently reopen the use-after-free the
    /// whole design is arranged around.
    ///
    /// What this CANNOT show, in those words: that a pinned module actually
    /// survives a `FreeLibrary`. That is the Windows loader's behaviour and
    /// observing it needs a live patched `hoi4.exe` with a real unload attempt.
    /// What is asserted here is that the PIN flag was accepted for this exact
    /// module before any detour was enabled.
    #[test]
    fn enabling_a_detour_pins_the_containing_module_first() {
        // Both globals are in play: `enable` pins AND freezes. Always taken in
        // this order, everywhere, so two tests cannot deadlock on them.
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _serialised = crate::freeze::os_suspend_state_guard();
        // No detour exists in a test process, so `enable` will be refused by
        // MinHook -- AFTER the pin, which is the ordering under test.
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

    /// Falsifiable: fails if any public entry point stops funnelling through
    /// [`super::run_pinned`].
    ///
    /// `run_pinned` is where the ordering lives, and the previous sweep showed
    /// that a bypass is invisible to every behavioural assertion available
    /// here: by the time a test runs, some earlier test has pinned the process,
    /// so "the module is pinned afterwards" holds for a bypassing build too.
    /// The source scan cannot help either, since it deliberately skips this
    /// crate -- this crate is the one place the banned calls are allowed.
    ///
    /// So the wiring is asserted structurally, the way
    /// `division_combat_stats_live` asserts its own rollback ordering: each of
    /// the three entry points must name the helper. A bypass drops the count.
    #[test]
    fn every_entry_point_funnels_through_the_ordering_helper() {
        let source = include_str!("lib.rs");
        // Split so this literal is not itself a match in the scanned source,
        // which would make the count self-satisfying.
        let needle = concat!("run_pinned(", "pin_containing_module");
        let funnelled = source.matches(needle).count();
        assert_eq!(
            funnelled, 3,
            "enable, queue_enable and apply_queued must each go through              run_pinned; found {funnelled} call(s), so one of them patches              without the pin ordering"
        );
    }

    /// THE ORDERING, and the only test that can actually discriminate it.
    ///
    /// Falsifiable: fails if `run_pinned` ever runs the operation when the pin
    /// was refused -- i.e. if a build patched first and pinned second. The
    /// asserting-the-flag-afterwards tests below CANNOT catch that, because by
    /// the time any of them runs some earlier test has already pinned this
    /// process; that is exactly how two mutations survived the first sweep.
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
        assert!(!ran.get(), "the patch must not run when the pin was refused");
        assert_eq!(
            outcome.unwrap_err(),
            InlineDetourEnableError::ModulePinRefused(5),
            "and the refusal must name the pin, not MinHook"
        );

        // The converse: an accepted pin does run it, so the guard is not simply
        // refusing everything.
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

    /// Falsifiable: fails if the QUEUED path stops pinning before it patches.
    ///
    /// `terrain_capture` reaches MinHook only through `queue_enable` +
    /// `apply_queued`, never through `enable`, so a pin that guarded only the
    /// direct path would have left that DLL exactly as exposed as before -- and
    /// it is the one with recorded 0xc0000005 faults in an unloaded mapping.
    #[test]
    fn the_queued_path_pins_before_it_applies() {
        // `apply_queued` patches, so it freezes: this test moves process-global
        // suspend state and must serialise on it too. Same order as everywhere.
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
        // An empty queue applies cleanly; either way the pin already happened.
        let _ = applied;
        assert!(module_pinned_against_unload());
    }

    /// Falsifiable: fails if the standalone pin stops being fail-closed, i.e.
    /// if it reports success the loader did not give. This is the inversion
    /// `terrain_capture` shipped -- it latched its pinned flag BEFORE calling
    /// the API and proceeded on failure, so the flag read pinned over a module
    /// that was still unloadable.
    #[test]
    fn the_standalone_pin_reports_only_what_the_loader_accepted() {
        assert!(pin_module_now().is_ok());
        assert!(module_pinned_against_unload());
    }

    /// Falsifiable: fails if a MinHook refusal is ever counted as an enabled
    /// detour. The count is what the teardown proof reports as the reason the
    /// module is retained, so inflating it would claim residency that never
    /// existed.
    #[test]
    fn a_refused_patch_is_not_counted_as_an_enabled_detour() {
        // Both globals are in play: `enable` pins AND freezes. Always taken in
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

    /// Falsifiable: fails if the pin-refusal and MinHook-refusal arms ever
    /// render identically. They are different facts with different operator
    /// responses -- one means the loader refused us, the other means the target
    /// was not hookable -- and a teardown reading one as the other would look
    /// in the wrong place.
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

    /// Falsifiable: fails if any of the three patching wrappers stops freezing
    /// the process.
    ///
    /// This is THE guard on the fix. Delete the `freeze_other_threads()` call
    /// from `enable`, `disable`, or `remove` and the corresponding attempt
    /// count stops moving, which fails here by name. Each call is driven at an
    /// address MinHook has no hook for, so MinHook refuses AFTER the freeze --
    /// the same shape as the pin-ordering test above.
    #[test]
    fn every_patching_wrapper_freezes_the_process_before_it_calls_minhook() {
        // Process-global suspend state: serialise with every other test
        // that freezes or arranges suspend counts.
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
    /// as anything other than a RETAINED, still-installed detour.
    ///
    /// The distinction is the whole point of the release error existing
    /// separately. `already_released` means the hook was gone and there is
    /// nothing to do; `detour_retained` means the game is still running our
    /// patched bytes and the owning unit must not claim it released anything.
    /// Collapsing the two would let a teardown proof pass over a live detour.
    #[test]
    fn a_release_that_could_not_freeze_reports_a_retained_detour() {
        let retained = InlineDetourReleaseError::FreezeRefused(FreezeRefusal::ThreadSuspend {
            thread: 4242,
            os_error: 5,
        });
        assert!(retained.detour_retained());
        assert!(!retained.already_released());
        assert!(retained.minhook_status().is_none());
        assert!(retained.to_string().contains("retained rather than restored"));
        assert!(retained.to_string().contains("4242"));

        for absent in [
            InlineDetourReleaseError::MinHook(minhook::MH_STATUS::MH_ERROR_DISABLED),
            InlineDetourReleaseError::MinHook(minhook::MH_STATUS::MH_ERROR_NOT_CREATED),
        ] {
            assert!(absent.already_released());
            assert!(!absent.detour_retained());
        }

        let refused = InlineDetourReleaseError::MinHook(minhook::MH_STATUS::MH_ERROR_MEMORY_PROTECT);
        assert!(!refused.already_released());
        assert!(!refused.detour_retained());
    }

    /// Falsifiable: fails if a freeze refusal is ever reported as a MinHook
    /// status on the enable path, which would send an operator looking at the
    /// target instead of at the process.
    #[test]
    fn an_enable_refused_by_the_freeze_is_not_a_minhook_status() {
        let refusal = InlineDetourEnableError::FreezeRefused(FreezeRefusal::NotQuiescent {
            passes: 16,
            still_running: 3,
        });
        assert!(refusal.minhook_status().is_none());
        assert!(refusal.freeze_refusal().is_some());
        assert!(refusal.to_string().contains("no target byte was touched"));
        assert!(InlineDetourEnableError::ModulePinRefused(5)
            .freeze_refusal()
            .is_none());
    }
}

/// The batched entry points, and the freeze count they exist to move.
///
/// These drive REAL MinHook detours on functions in the test binary, because
/// the properties under test are not expressible against a mocked hook: "one
/// freeze covered twelve targets" is a count taken from the freeze module
/// itself, and "the other eleven are live even though the twelfth was refused"
/// is only observable by calling them.
#[cfg(all(test, windows))]
mod batch_tests {
    use super::*;
    use core::ffi::c_void;
    use minhook::{MinHook, MH_STATUS};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Serialises every test in this module against every test in `tests`:
    /// both move the process-global freeze counters and the process-global
    /// MinHook hook table, and a delta read is meaningless if another test is
    /// taking its own freezes in the window. Always taken BEFORE
    /// `os_suspend_state_guard`, the same order `tests` uses, so the two
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

    // Four distinct hookable functions. `#[inline(never)]` and the distinct
    // constants keep the linker from folding them into one body, which would
    // make four hooks into one target and quietly turn every batch here into a
    // batch of one.
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

    /// An address MinHook has no hook for, and never will. The failure this
    /// injects into a batch is `MH_ERROR_NOT_CREATED`, which is exactly what a
    /// production hook whose `create_hook` was never reached looks like.
    const ABSENT_TARGET: *mut c_void = 0x5000 as *mut c_void;

    /// Create `count` real hooks and return their targets, leaving them
    /// DISABLED. Removes any prior instance first so a test that ran before
    /// this one cannot leave a hook behind and turn a `create` into
    /// `MH_ERROR_ALREADY_CREATED`.
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

    /// THE PER-HOOK FAILURE PROOF, and the whole reason this is a report and
    /// not a `Result<(), E>`.
    ///
    /// Falsifiable three ways, each of them a regression the task named:
    ///   * SMEARED BLAME -- if the batch aborted at the first refusal, the
    ///     two real targets would carry the third's status instead of their
    ///     own. Asserted by their outcomes being `Ok`.
    ///   * SILENCE -- if the batch reported only "something failed", the
    ///     absent target's name and its `MH_ERROR_NOT_CREATED` would not be
    ///     recoverable. Asserted by name and by status.
    ///   * COLLATERAL BLAME -- if a batch verdict were smeared over every
    ///     target, the two good ones would read as failures. Asserted by
    ///     `failures().len() == 1`.
    ///
    /// What production change would make this fail? Replacing `enable_batch`'s
    /// per-target outcome vector with a single `Result` for the batch; making
    /// a refused queue abort the remaining queue calls; or dropping the queue
    /// status and reporting every target from `MH_ApplyQueued`'s one verdict.
    ///
    /// It also pins the two things the batch owes beyond reporting: nothing
    /// this batch enabled is left live when any target refused, and no queue
    /// entry is left for an unrelated `MH_ApplyQueued` to act on.
    #[test]
    fn one_refused_hook_in_a_batch_is_named_and_the_others_are_not_blamed() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();
        let real = create_victims(2);
        let batch = [real[0], ("absent.hook", ABSENT_TARGET), real[1]];

        let report = unsafe { enable_batch(&batch) };

        assert_eq!(report.len(), 3);
        assert!(report.outcomes()[0].1.is_ok(), "{:?}", report.outcomes()[0]);
        assert!(report.outcomes()[2].1.is_ok(), "{:?}", report.outcomes()[2]);
        let (name, error) = report
            .first_failure()
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

        // ATOMIC IN EFFECT. The two real targets' OWN enables succeeded and
        // say so, but the batch undid itself because a third target refused,
        // so neither is live -- checked by calling them, not by reading the
        // report that would be the thing under test.
        assert!(report.rolled_back(), "a refused target undoes the batch");
        VICTIM_CALLS[0].store(0, Ordering::Release);
        VICTIM_CALLS[1].store(0, Ordering::Release);
        assert_ne!(call(0), 0xD0, "victim.0 must not be left detoured");
        assert_ne!(call(1), 0xD1, "victim.1 must not be left detoured");
        assert_eq!(VICTIM_CALLS[0].load(Ordering::Acquire), 1);
        assert_eq!(VICTIM_CALLS[1].load(Ordering::Acquire), 1);

        // AND NO QUEUE RESIDUE. MinHook's queue is process-global, so a target
        // left queued is a patch some later, unrelated `MH_ApplyQueued`
        // performs inside a freeze nobody accounted for. Driving one here must
        // change nothing.
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

    /// THE FREEZE COUNT. Falsifiable: fails the moment `enable_batch` goes
    /// back to freezing per target.
    ///
    /// Four real hooks, driven both ways in one test so the comparison is
    /// against the same process in the same state rather than against a
    /// remembered number. `freeze_attempts_on_this_thread` is the counter the
    /// freeze module already keeps and is incremented once per
    /// `freeze_other_threads` call, which is one system-wide thread walk plus
    /// at least one verification walk each.
    ///
    /// What production change would make this fail? Implementing
    /// `enable_batch` as a loop over `enable`; calling `apply_queued` once per
    /// target; or taking a freeze around the queue phase, which patches
    /// nothing and needs none.
    ///
    /// MUTATION RUN: queue phase changed to call `enable` per target instead
    /// of `queue_enable`. OBSERVED: FAILED, `one freeze must serve the whole
    /// batch, not 5 of them / left: 5 / right: 1`.
    #[test]
    fn a_batch_of_four_takes_one_freeze_where_four_singles_take_four() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
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

        // Back to disabled, so the batch has the same work to do.
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

    /// Falsifiable: fails if an empty batch freezes the process, or if a
    /// batch whose targets are all absent freezes more than once.
    ///
    /// The empty case is the one that must cost nothing: a freeze is a
    /// system-wide thread walk and a suspend of every game thread, and paying
    /// it for a batch with no work is pure loss. The all-absent case is the
    /// deliberate price of putting the queue calls inside the freeze, and it
    /// is bounded here so that price stays one freeze and not one per target.
    ///
    /// What production change would make this fail? Removing the empty-batch
    /// early return, or moving the queue calls back outside the freeze and
    /// taking a second one to apply them.
    #[test]
    fn an_empty_batch_never_freezes_and_a_broken_roster_freezes_at_most_once() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();

        let empty = unsafe { enable_batch(&[]) };
        assert_eq!(empty.freezes(), 0, "nothing to patch, nothing to freeze");
        assert!(empty.is_empty());
        let empty_release = unsafe { disable_batch(&[]) };
        assert_eq!(empty_release.freezes(), 0);
        let empty_removal = unsafe { remove_batch(&[]) };
        assert_eq!(empty_removal.freezes(), 0);

        // A roster whose every target is absent still takes the ONE freeze,
        // because the queue calls are inside it -- that is the deliberate
        // price of a queue that cannot leak. It must not take more than one,
        // and every target must still be named.
        let all_absent = unsafe { enable_batch(&[("absent.a", ABSENT_TARGET)]) };
        assert_eq!(all_absent.freezes(), 1);
        assert_eq!(all_absent.failures().len(), 1);
        assert_eq!(all_absent.first_failure().map(|(name, _)| name), Some("absent.a"));

        let released = unsafe { disable_batch(&[("absent.b", ABSENT_TARGET)]) };
        assert_eq!(released.freezes(), 1);
        // Nothing to disable is not a refusal: there is nothing patched, which
        // is the postcondition a release asks for.
        assert!(released.all_succeeded());
        assert!(released.was_already_released("absent.b"));
    }

    /// Falsifiable: fails if the batched enable stops counting each target in
    /// the enabled-detour census.
    ///
    /// The census is what the teardown proof reports as the reason the module
    /// is pinned for the life of the process. `apply_queued` counts a batch as
    /// ONE, which was harmless while nothing in production used it and is not
    /// harmless now: a 12-hook lane would understate its own residency by
    /// eleven.
    ///
    /// What production change would make this fail? Reverting `enable_batch`
    /// to `ENABLED_DETOURS.fetch_add(1, ..)`; counting refused targets; or
    /// counting the targets whose own enable succeeded in a batch that then
    /// rolled itself back, which claims residency for detours that are gone.
    #[test]
    fn the_batch_counts_every_target_it_enabled_and_no_target_it_refused() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();
        let targets = create_victims(3);

        // A clean batch counts every target it made live, once each.
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

        // A batch that undid itself counts NOTHING, because nothing is live.
        // Counting the targets whose own enable succeeded would claim
        // residency for detours the rollback took straight back out.
        let batch = [targets[0], targets[1], ("absent.c", ABSENT_TARGET), targets[2]];
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

    /// Falsifiable: fails if a batch that lists the same target twice is
    /// accepted.
    ///
    /// One physical patch counted as two detours inflates the census the
    /// teardown proof cites, and the duplicate entry's outcome describes the
    /// first entry's work rather than its own. Neither is recoverable after
    /// the fact, so the batch is refused before anything is queued -- and
    /// refused for EVERY entry, because a partially-accepted duplicate roster
    /// is the same problem one step later.
    ///
    /// What production change would make this fail? Deleting the
    /// `first_duplicate_target` guard, or applying it to only the second
    /// occurrence.
    ///
    /// MUTATION RUN: guard replaced with `if let Some(duplicate) =
    /// None::<usize>`. OBSERVED: FAILED, `left: 0 / right: 3` with the
    /// outcomes `[("victim.0", Ok(())), ("victim.1", Ok(())), ("victim.0",
    /// Ok(()))]` -- one physical patch reported and counted twice.
    #[test]
    fn a_batch_that_lists_the_same_target_twice_is_refused_outright() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
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
        assert_eq!(
            enabled_detour_count(),
            before,
            "and counts nothing"
        );

        // The same two targets WITHOUT the duplicate are accepted, so the
        // guard is not simply refusing everything.
        let clean = unsafe { enable_batch(&targets) };
        assert!(clean.all_succeeded(), "{:?}", clean.outcomes());
        let _ = unsafe { disable_batch(&targets) };
        destroy_victims(&targets);
    }

    /// THE FREEZE-WINDOW LAW, asserted against the source because nothing
    /// available offline can observe it.
    ///
    /// Every other thread in the process is suspended for the length of these
    /// windows. A single call into the allocator, or any lock a suspended
    /// thread might hold, deadlocks the process permanently -- it is not a
    /// slow path, there is nobody left to release the lock. A `.collect()`
    /// inside `remove_batch`'s window was exactly that defect, and it read as
    /// harmless.
    ///
    /// So the windows are required to contain no allocating construct. The
    /// buffers they fill are reserved before the freeze is taken, which is why
    /// `push` into reserved capacity is permitted and `Vec::with_capacity`
    /// inside the window is not.
    ///
    /// What production change would make this fail? Building the report,
    /// formatting an error, writing to the release ledger, or collecting an
    /// iterator between `freeze_other_threads()` and `drop(frozen)`.
    ///
    /// MUTATION RUN: `let _leak: Vec<u8> = Vec::with_capacity(8);` added
    /// inside `remove_batch`'s window. OBSERVED: FAILED, `a held freeze window
    /// contains `Vec::with_capacity``. The defect this replaces was real: the
    /// first version of `remove_batch` collected its statuses inside the
    /// window.
    #[test]
    fn no_held_freeze_window_allocates_or_takes_a_lock() {
        let source = include_str!("lib.rs").replace("\r\n", "\n");
        // Split so these fragments do not match themselves in the scanned
        // source, which would make the scan self-satisfying.
        let banned = [
            concat!(".coll", "ect()"),
            concat!("Vec::with_cap", "acity"),
            concat!("form", "at!("),
            concat!("to_st", "ring()"),
            concat!("release_ledger::", "record_release_attempt"),
            concat!(".lo", "ck()"),
        ];
        // Each window is delimited by an explicit marker and its `drop`, and
        // EVERY `drop(frozen)` must have a marker before it -- so a new freeze
        // added without one fails here rather than escaping the scan.
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

    /// THE CLOSING EVIDENCE, at the cardinality of the largest real install
    /// unit.
    ///
    /// D45 installs twelve detours in one unit. This stages twelve real
    /// MinHook hooks and measures both paths in the same process, in the same
    /// state, back to back: the per-hook path costs one freeze per hook and
    /// the batch costs one for the whole unit. Each freeze is a system-wide
    /// thread-list enumeration plus a suspend and resume of every other thread
    /// in the process, and it is the count -- not a wall time taken on a
    /// machine running five other build lanes -- that this API exists to move.
    ///
    /// The numbers are ASSERTED as a law, not recorded: `singles == twelve`
    /// and `batch == 1` for any twelve targets, so the evidence does not go
    /// stale when the roster changes size.
    ///
    /// What production change would make this fail? Any regression that puts a
    /// freeze back inside the batch: reverting `enable_batch` to a loop over
    /// `enable`, applying the queue per target, or freezing around the queue
    /// phase.
    ///
    /// MUTATION RUN: queue phase changed to call `enable` per target.
    /// OBSERVED: FAILED, `install, release and removal must each cost ONE
    /// freeze for the whole unit / left: (13, 1, 1) / right: (1, 1, 1)` --
    /// twelve per-target freezes plus the apply.
    #[test]
    fn twelve_hooks_cost_twelve_freezes_one_at_a_time_and_one_as_a_batch() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
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
        // Stated as the ratio the migration claims, so a partial regression --
        // say, a batch that freezes twice -- fails here rather than passing a
        // "fewer than before" comparison.
        assert_eq!(
            singles_enable + singles_disable,
            (enabled.freezes() + released.freezes()) * 12,
            "12x on the install-plus-release span"
        );
    }

    /// Twelve distinct hookable functions, created and left disabled.
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
        // One shared counter, distinct salts: enough to stop the linker
        // folding twelve identical bodies into one, which would silently turn
        // a twelve-target batch into a one-target batch.
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

    /// Falsifiable: fails if `remove_batch` stops holding ONE freeze across
    /// the whole removal, or stops reporting each target separately.
    ///
    /// Removal has no MinHook queue, so this is the one batch entry point that
    /// is a loop under a held guard rather than an applied batch. The property
    /// is the same and so is the test: four removals, one freeze, and a target
    /// MinHook has no hook for is named rather than smeared over the others.
    ///
    /// What production change would make this fail? Rewriting `remove_batch`
    /// as a loop over `remove`; taking the freeze inside the loop; or
    /// returning early on the first refused target, which would leave the
    /// remaining trampolines allocated with nothing saying so.
    #[test]
    fn a_batched_removal_frees_every_trampoline_under_one_freeze() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
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
        // A target MinHook has no hook for is NOT a failure -- there is
        // nothing patched and no trampoline, so the removal's postcondition
        // holds -- but it is NAMED, because a lane that tracks its own
        // removals has to tell that from one it performed itself.
        assert!(
            removed.all_succeeded(),
            "an absent target is nothing-to-do, not a refusal: {:?}",
            removed.outcomes()
        );
        assert!(removed.was_already_released("absent.removal"));
        assert_eq!(removed.already_released(), ["absent.removal"]);
        assert_eq!(removed.failures().len(), 0);

        // Gone for real: a second remove of a real target finds nothing.
        assert_eq!(
            unsafe { remove(targets[0].1) }
                .expect_err("the hook was removed")
                .minhook_status(),
            Some(MH_STATUS::MH_ERROR_NOT_CREATED)
        );
    }

    /// THE FIDELITY GUARD ON THE NORMALISATION, and it cuts both ways.
    ///
    /// Every per-target teardown this batch API replaced read
    /// `MH_ERROR_DISABLED` and `MH_ERROR_NOT_CREATED` as nothing-to-do, so a
    /// batch that reported them as refusals would make a second, idempotent
    /// teardown fail where the loop it replaced succeeded. And two lanes track
    /// which of their OWN sites they have already removed and treat an
    /// unexpected `MH_ERROR_NOT_CREATED` as a custody failure, so a batch that
    /// merely swallowed the status would erase the fact they decide on.
    ///
    /// Both halves are asserted here: the outcome is `Ok`, AND the target is
    /// named in `already_released`.
    ///
    /// What production change would make this fail? Reporting the raw
    /// `MH_ERROR_NOT_CREATED` as the outcome (breaks idempotent teardown), or
    /// folding it into `Ok` without recording the name (breaks the two custody
    /// lanes).
    ///
    /// MUTATION RUN: the already-released arm of the queue phase disabled, so
    /// the raw status becomes the outcome. OBSERVED: FAILED, `a hook that does
    /// not exist cannot be left patched`.
    ///
    /// MUTATION RUN: `already_released.push(name)` removed from
    /// `remove_batch`, keeping the `Ok` fold. OBSERVED: FAILED, `assertion
    /// failed: removed.was_already_released("absent.remove")` -- and the
    /// removal test failed with it.
    #[test]
    fn a_release_batch_reports_nothing_to_do_as_success_and_still_names_it() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();

        let disabled = unsafe { disable_batch(&[("absent.disable", ABSENT_TARGET)]) };
        assert!(
            disabled.all_succeeded(),
            "a hook that does not exist cannot be left patched: {:?}",
            disabled.outcomes()
        );
        assert!(disabled.was_already_released("absent.disable"));
        // ONE freeze even though there was nothing to do: the queue calls are
        // inside the window, which is what makes the queue leak-proof. Bounded
        // here so that price stays one freeze and never one per target.
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
    ///
    /// The ledger is the whole of Job 2's mechanism on the batched path. A
    /// `disable_batch` that patched correctly but recorded nothing would leave
    /// exactly the silence the ruling is about.
    ///
    /// What production change would make this fail? Dropping the
    /// `record_release_attempt` calls from `disable_batch`/`remove_batch`, or
    /// dropping `name_release_target` so the record cannot say WHICH hook.
    #[test]
    fn a_batched_release_writes_every_attempt_into_the_ledger_by_name() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // Reset and read under the ledger's own serialisation, because the
        // ledger is process-global and another test's teardown writes into it.
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _suspend = crate::freeze::os_suspend_state_guard();
        // Reset AFTER creating the hooks: `create_victims` removes any prior
        // instance first, and that removal is itself a release attempt this
        // test is not asking about.
        let targets = create_victims(2);
        crate::release_ledger::reset_for_tests();
        let installed = unsafe { enable_batch(&targets) };
        assert!(installed.all_succeeded(), "{:?}", installed.outcomes());

        let released = unsafe { disable_batch(&targets) };
        assert!(released.all_succeeded(), "{:?}", released.outcomes());
        let removed = unsafe { remove_batch(&targets) };
        assert!(removed.all_succeeded(), "{:?}", removed.outcomes());

        // Read PER TARGET, not by slicing the whole history: a concurrent
        // test's own hooks write into the same ledger, and an assertion that
        // counted every entry would be measuring them too.
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
    /// bytes of every target it was given.
    ///
    /// The name-level assertion is not enough on its own -- a `disable_batch`
    /// that reported success without queuing anything would pass it -- so this
    /// calls each function and requires the ORIGINAL to answer.
    ///
    /// What production change would make this fail? Queuing only the first
    /// target; dropping the `MH_ApplyQueued` call; or queuing an ENABLE on the
    /// release path.
    #[test]
    fn a_batched_release_restores_every_target_it_was_given() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
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
