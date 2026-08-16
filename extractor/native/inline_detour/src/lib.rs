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

/// What every stopped window cost, recorded in production rather than in a test
/// program. See the module for the gap it closes.
#[cfg(windows)]
pub mod freeze_census;

#[cfg(windows)]
pub mod release_ledger;

/// THE FAILURE PATHS, DRIVEN. Test-only, and the seams it uses are absent from
/// every build that is not this crate's own test harness. See the module.
#[cfg(all(test, windows))]
mod fault_tests;

/// The allocation probe, installed as THIS CRATE'S TEST BINARY's allocator and
/// nowhere else.
///
/// It is a counting pass-through to `std::alloc::System`, so it changes no
/// behaviour; what it buys is that
/// [`fault_tests::the_probe_sees_an_allocation_inside_a_held_window`] can prove
/// the instrument the freeze-window audit rests on is still counting. Without
/// the attribute, `freeze_window_audit::observed` returns zero for every
/// window whatever it does, and every `assert_eq!(allocations, 0)` in the
/// workspace becomes vacuous on the same day.
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
        /// THE BATCH'S OWN APPLY REFUSED, so this target is refused with it.
        ///
        /// `MH_ApplyQueued` stops at the first target it cannot patch, and it
        /// does not say which target that was or how far it got. In this fork
        /// `EnableHookLL` can refuse in EITHER of two phases and the status is
        /// the same `MH_ERROR_MEMORY_PROTECT` for both: before it writes
        /// anything, when the first `VirtualProtect` cannot make the page
        /// writable; or after the bytes are written, when the protection
        /// restore or the instruction-cache flush did not complete -- and only
        /// the second latches `incoherent`. So a refused apply means "one of
        /// these targets may be patched with its last write unpublished, and
        /// MinHook names neither the target nor the phase".
        ///
        /// The whole batch is therefore refused and undone -- which is safe in
        /// both phases, because a disable of a target that was never patched
        /// answers `MH_ERROR_DISABLED` without touching a byte -- and every
        /// target carries this rather than the `Ok` its own completion-pass
        /// enable answered.
        ///
        /// TWO FACTS AN OPERATOR NEEDS NEXT, and they are NOT the same fact.
        /// `still_live` says whether this target is patched into the running
        /// game. `rolled_back` says whether the undo COMPLETED. A rollback's
        /// own disable can restore the bytes and then fail to publish, which
        /// leaves the target not live AND not cleanly undone: `incoherent` is
        /// latched, so its removal will refuse by name until a completed
        /// enable-then-disable re-derives the latch. Reporting that as "rolled
        /// back and not live" would be a false all-clear, so the two are
        /// carried separately (review round 4, MED).
        ///
        /// It is a variant of its own rather than [`Self::MinHook`] so the
        /// wire says WHICH failure this was. `MH_ERROR_MEMORY_PROTECT` charged
        /// to a named hook reads as "that hook's own enable refused", which is
        /// a different event with a different recovery.
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
        /// The underlying MinHook status, when the refusal came from MinHook
        /// rather than from the pin or the freeze. Call sites that inspect
        /// specific statuses use this; call sites that only render the failure
        /// use `Display`.
        pub fn minhook_status(self) -> Option<MH_STATUS> {
            match self {
                Self::MinHook(status) | Self::BatchApplyRefused { status, .. } => Some(status),
                Self::ModulePinRefused(_) | Self::FreezeRefused(_) => None,
            }
        }

        /// The freeze refusal, when that is what stopped the patch.
        pub fn freeze_refusal(self) -> Option<FreezeRefusal> {
            match self {
                Self::FreezeRefused(refusal) => Some(refusal),
                Self::ModulePinRefused(_) | Self::MinHook(_) | Self::BatchApplyRefused { .. } => {
                    None
                }
            }
        }

        /// The same refusal, told whether the target it is charged to is still
        /// patched into the running process and whether the undo completed.
        ///
        /// Only a batch-apply refusal carries those facts, and only the batch
        /// that performed the rollback knows them, so they are stamped when the
        /// report is sealed rather than where the refusal is raised. Every
        /// other refusal answers itself unchanged: a refused pin and a refused
        /// freeze patched nothing, and a target's own MinHook refusal is
        /// already about that target.
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
        /// [`remove`] was asked to free the trampoline of a target that is
        /// STILL ENABLED. NOTHING WAS FREED and nothing was restored; the
        /// detour is retained.
        ///
        /// An enabled target is still admitting callbacks, so no drain of the
        /// caller's detour body can have been stable -- and a body in flight
        /// returns THROUGH the trampoline this call would have freed. The
        /// freeze's instruction-pointer check cannot see that thread, because
        /// a detour body is the caller's own function at an address this crate
        /// is never told. See [`remove`]'s contract.
        StillEnabled,
    }

    impl fmt::Display for InlineDetourReleaseError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::FreezeRefused(refusal) => write!(
                    f,
                    "inline detour retained rather than restored: {refusal}"
                ),
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
        /// The underlying MinHook status, when MinHook is what refused.
        pub fn minhook_status(self) -> Option<MH_STATUS> {
            match self {
                Self::MinHook(status) => Some(status),
                Self::FreezeRefused(_) | Self::StillEnabled => None,
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
            matches!(self, Self::FreezeRefused(_) | Self::StillEnabled)
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
    /// MinHook no longer suspends anything -- `minhook_h4` deleted that, and the
    /// instruction-pointer relocation with it -- so the freeze is not a
    /// narrowing of somebody else's fail-open any more. It is the whole of the
    /// protection: this call refuses unless every other thread of the process is
    /// held, and refuses AGAIN if any held thread is stopped inside the exact
    /// bytes the patch would rewrite. What is left, and why, is documented on
    /// [`freeze`]: a thread another process inserts after the last pass is
    /// outside any user-mode fixed point.
    ///
    /// # Safety
    ///
    /// `target` must name a hook already created with MinHook, and the caller
    /// keeps every obligation `MH_EnableHook` has: the detour must remain a
    /// valid callable for as long as the patch is live.
    pub unsafe fn enable(target: *mut c_void) -> Result<(), InlineDetourEnableError> {
        run_pinned(pin_containing_module, || {
            let mut frozen = freeze_other_threads()?;
            // FROZEN WINDOW: nothing below this line until `drop(frozen)` may allocate or block.
            frozen.note_entry(FreezeEntryPoint::Enable, "", 1);
            // REFUSE BEFORE A BYTE MOVES if a held thread is stopped inside the
            // bytes this enable rewrites. MinHook used to answer that by moving
            // the thread's instruction pointer and patching regardless of
            // whether it could; the fork deleted that and this stands in its
            // place.
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
        run_pinned(pin_containing_module, || {
            // NO FREEZE, because queueing patches nothing -- but it does touch
            // MinHook's table, so it takes MinHook's lock, and it takes it here
            // with every thread of the process still RUNNING. That is the only
            // place an acquisition is safe: inside a frozen window it could
            // wait forever on a thread that cannot run.
            let lock = MinHookLock::acquire()?;
            unsafe { MinHook::queue_enable_hook(&lock, target) }
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
            let mut frozen = freeze_other_threads()?;
            // FROZEN WINDOW: nothing below this line until `drop(frozen)` may allocate or block.
            // The batch size is MinHook's internal state and is not reported
            // back, so this window records no target count rather than a wrong
            // one; the census reads a zero here as "not stated" for exactly the
            // reason this function's own doc gives.
            frozen.note_entry(FreezeEntryPoint::ApplyQueued, "", 0);
            // The QUEUED set is what this patches, and it is exactly the set the
            // check asks about -- including any entry an earlier caller left
            // queued, which this call would otherwise patch with nobody having
            // checked it.
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
        live: Vec<bool>,
        rolled_back: bool,
        freezes: u64,
    }

    impl BatchEnableReport {
        /// True when this batch undid itself: something refused, so every
        /// target the batch had queued was disabled again before the freeze
        /// was dropped, and NOTHING it was given is live.
        ///
        /// The per-target outcomes still say what each target's own enable
        /// answered -- that is what names the refuser -- so this is the flag
        /// that says an `Ok` outcome does not mean "currently patched".
        pub fn rolled_back(&self) -> bool {
            self.rolled_back
        }

        /// THE BATCH'S ONE VERDICT, and the only gate an enable consumer needs:
        /// `None` when every target this batch was given is patched and live,
        /// and otherwise the first refusal, named.
        ///
        /// It replaced `first_failure()` on this side because that name invited
        /// the reading the defect below lived in -- "the first target that
        /// refused, and the rest are fine" -- when what a caller must act on is
        /// "this batch did not install".
        ///
        /// # The state this cannot be in, and used to be
        ///
        /// `MH_ApplyQueued` can refuse AFTER writing a target's bytes, and the
        /// completion pass then re-enables every queued target: MinHook answers
        /// `MH_ERROR_ENABLED` for the one already patched, which is mapped to
        /// `Ok` because that target IS patched. A batch could therefore report
        /// `apply_refusal() = Some(..)`, `all_succeeded() = false`, and yet no
        /// named failure at all -- so every one of the fifteen production
        /// installs, all of which gate on the named failure, accepted a roster
        /// with one detour live and its last write incomplete
        /// (`runs/ln_fault_inject/FINDINGS.txt`).
        ///
        /// That state is now unrepresentable rather than merely not produced:
        /// [`Self::sealed`] charges a batch-level refusal to every target that
        /// has no refusal of its own, and [`Self::all_succeeded`] is DEFINED as
        /// this returning `None`. There is one source of truth, so "no failures
        /// but not all succeeded" has nowhere left to live.
        /// THE BATCH'S OWN CAUSE COMES FIRST when there is one, and that is not
        /// a tie-break. `MH_ApplyQueued` refusing is the reason NOTHING is
        /// installed; a target's own `MH_ERROR_MEMORY_PROTECT` alongside it is
        /// a detail of one hook inside a roster that was going to be undone
        /// anyway. Taking whichever came first in slice order would put a bare
        /// per-hook status on the wire for a whole-roster refusal, which reads
        /// as "that hook refused" -- a different event with a different
        /// recovery. Both are still in [`Self::failures`] and
        /// [`Self::outcomes`], in slice order, so nothing is lost.
        ///
        /// THE CAUSE COMES FROM THE BATCH-LEVEL FIELD, not from whichever
        /// target happens to carry a copy of it, and the difference is
        /// load-bearing on the `still_live` half. The per-target copies each
        /// answer for THEIR target; the batch-level one answers for the batch
        /// -- true when ANY target the rollback could not take down is still
        /// patched. Reading a copy would let a roster with one live target
        /// report "rolled back, not live" because some OTHER target's copy
        /// said so, which is a false reassurance about the running game. It
        /// also survives the case where every target ended up with a refusal
        /// of its own, so no copy exists to find (review round 3, MED).
        ///
        /// The NAME points at something an operator can act on: the first
        /// target still patched when any is, else the first target carrying
        /// the batch cause, else the first of the roster.
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
                // `None` only for an EMPTY roster, which cannot carry a batch
                // refusal: the empty batch returns before anything is queued.
                if let Some(name) = named {
                    return Some((name, cause));
                }
            }
            // A refused pin and a refused freeze are already EVERY target's own
            // outcome, so slice order returns the batch cause for those too.
            self.outcomes
                .iter()
                .find_map(|(name, result)| result.err().map(|error| (*name, error)))
        }

        /// True when every target in the batch is patched and live.
        ///
        /// Definitionally [`Self::refusal`] answering `None` -- not a second,
        /// parallel computation that could disagree with it. See `refusal`.
        pub fn all_succeeded(&self) -> bool {
            self.refusal().is_none()
        }

        /// Which targets are PATCHED INTO THE PROCESS, by the position they
        /// were listed in. Same length as [`Self::outcomes`].
        ///
        /// An outcome is not this and cannot be made into it. A target the
        /// batch rolled back reports a refusal and is not live; a target whose
        /// undo was itself refused reports one and IS. A caller that keeps its
        /// own per-site custody mask -- "which of my hooks does my teardown
        /// have to take down" -- must read THIS, because both of the other
        /// readings are wrong in one of those two directions: from the
        /// outcomes it claims custody of detours the rollback removed, and
        /// from `rolled_back()` alone it disclaims custody of the one detour a
        /// refused rollback left in the game.
        pub fn still_live(&self) -> &[bool] {
            &self.live
        }

        /// Build the report, charging a batch-level refusal to every target
        /// that has no refusal of its own.
        ///
        /// EVERY construction of this type goes through here, which is what
        /// makes the invariant structural rather than a rule each return path
        /// has to remember. Four of the five paths are already consistent --
        /// a refused duplicate roster, a refused pin and a refused freeze all
        /// give every target its own `Err`, and an empty batch has no refusal
        /// -- and the fifth is the one the defect lived in.
        ///
        /// `live_now[i]` says whether target `i` is patched into the process at
        /// the moment the freeze is dropped, which is what stamps `still_live`
        /// on the refusal that reaches the wire. A shorter slice than
        /// `outcomes` means "nothing is live", which is exactly true of every
        /// path that returns before the freeze is taken.
        fn sealed(
            mut outcomes: Vec<BatchOutcome<InlineDetourEnableError>>,
            apply_refusal: Option<InlineDetourEnableError>,
            live_now: &[bool],
            rolled_back: bool,
            freezes: u64,
        ) -> Self {
            // Materialised to the roster's own length so `still_live()` can be
            // indexed beside `outcomes()` on EVERY path, including the ones
            // that return before the freeze is taken and are all-false.
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
            // The batch-level copy answers for the batch: it is still live if
            // ANY target the rollback could not take down is.
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

        /// The first refused target, named.
        ///
        /// A release batch keeps this spelling where [`BatchEnableReport`] has
        /// a single `refusal()`, and the difference is behavioural rather than
        /// cosmetic: a release does NOT undo itself, so a partial teardown is a
        /// real, reportable state and each target's own outcome is the account
        /// of what happened to IT. The lanes restore explicitly from those
        /// outcomes. An enable that refuses has nothing partial to report,
        /// because it rolled the whole batch back.
        pub fn first_failure(&self) -> Option<(&'static str, InlineDetourReleaseError)> {
            self.outcomes
                .iter()
                .find_map(|(name, result)| result.err().map(|error| (*name, error)))
        }

        /// True when every target in the batch was released AND the batch
        /// itself was not refused.
        ///
        /// THE SECOND HALF IS LOAD-BEARING and was missing. A target's own
        /// outcome is `Ok` when MinHook answered that it was already disabled
        /// -- which is the postcondition a release asks for -- while
        /// `apply_refusal` carries a failure that belongs to the whole batch,
        /// such as a protection restore or an instruction-cache flush that did
        /// not complete. Reporting "all succeeded" over that is how a teardown
        /// claims a roster is gone whose bytes were restored but not coherently
        /// published (review round 2, HIGH).
        ///
        /// See `runs/lt_apply_refusal_rollback/FINDINGS.txt`, "the release
        /// twin": on this side the gap between this and `first_failure()` is
        /// still reachable, and closing it is a change to the retained census
        /// rather than to the report.
        pub fn all_succeeded(&self) -> bool {
            self.apply_refusal.is_none()
                && self.outcomes.iter().all(|(_, result)| result.is_ok())
        }
    }

    macro_rules! batch_report_accessors {
        ($report:ty, $error:ty) => {
            impl $report {
                /// The refusal `MH_ApplyQueued` returned for the batch as a
                /// whole, when it returned one.
                ///
                /// It is INFORMATIONAL on the enable side and no longer the
                /// only carrier of anything: a batch-level refusal is charged
                /// to every target when the report is sealed, so
                /// `BatchEnableReport::refusal()` already names it and no
                /// caller has to know this accessor exists to fail closed. It
                /// stays because it tells a refusal that belongs to the BATCH
                /// from one that belongs to a target, which is what says
                /// whether the roster or one hook is the thing that is wrong.
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

                /// Every refused target, named.
                pub fn failures(&self) -> Vec<(&'static str, $error)> {
                    self.outcomes
                        .iter()
                        .filter_map(|(name, result)| result.err().map(|error| (*name, error)))
                        .collect()
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
    /// [`enable`] freezes the whole process per call, and a freeze is at least
    /// two walks of this process's thread list plus a suspend/resume of every
    /// other thread, followed by MinHook's own work inside the same window.
    ///
    /// THAT WORK DOMINATES THE WINDOW, MEASURED
    /// (`runs/ju_ix_residuals/freeze_cost_by_build_profile.txt`, paired samples
    /// timed from freeze acquisition to release): `MinHook::apply_queued` is
    /// 74.7-95.3% of every production frozen window sampled, and acquiring the
    /// freeze is 4.6-24.9% of it.
    ///
    /// THIS COMMENT USED TO CALL THAT REMAINDER "MinHook's own system-wide
    /// snapshot" AS A FACT. It is an INFERENCE and is labelled as one now:
    /// MinHook's source was not read here and its internals were not timed.
    /// What IS measured is that the call dominates, shows no per-target trend
    /// across batches of 1/3/6/12 targets, and is the same order as the
    /// system-wide snapshot pass `freeze.rs` replaced. Confirming step: time
    /// MinHook's `Freeze()`/`Unfreeze()` directly, or read the vendored
    /// `minhook 0.9.0` C source.
    ///
    /// THE PER-FREEZE COST HAS FALLEN and the reason this exists has not. When
    /// it was written a freeze was one `CreateToolhelp32Snapshot` per pass over
    /// every thread on the machine, OBSERVED at >=87-111 ms for a clean
    /// two-pass freeze and identical in both build profiles because none of the
    /// work was ours. The freeze now enumerates with `NtGetNextThread` and
    /// confirms the fixed point once against a materialised roster, and a clean
    /// freeze was OBSERVED at 1.85-2.13 ms IN THE SHIPPED RELEASE BUILD and
    /// 14.6-19.2 ms in debug, on one machine holding ten threads
    /// (`runs/ju_ix_residuals/freeze_cost_by_build_profile.txt`; the
    /// 15.4-25.1 ms this comment used to quote from
    /// `runs/ix_batch_freeze/enumeration_cost_measured.txt` is the debug row).
    /// That is a sample and not a bound, and the two halves scale differently:
    /// the walk, the granted-access query, the suspend and the context read are
    /// PER THREAD and hoi4.exe holds far more than ten, while the roster is per
    /// FREEZE and tracks the MACHINE's process table rather than ours. Batching
    /// therefore still buys milliseconds -- one roster instead of one per detour
    /// -- though far less than the ~350 ms the debug figure implied, and the
    /// install's freeze count is itself unverified on this tree
    /// (`runs/ju_ix_residuals/freeze_cost_by_build_profile.txt`: lane IX's
    /// census said fourteen, `PRODUCTION_INSTALL_UNITS` is now declared with
    /// twelve, and a fresh census is the named step). What it buys that is
    /// not measured in milliseconds is ATOMICITY, which is the reason it was
    /// written when the milliseconds had gone: the whole set becomes live inside
    /// ONE window, so no game thread
    /// runs between one detour being patched and the next -- see "Atomic in EFFECT" below -- and
    /// `MinHook::apply_queued`, whose cost dominates the window and which this
    /// module has never been shown able to make cheaper, is paid ONCE instead of
    /// once per target. (This sentence used to name that cost as "MinHook's own
    /// snapshot". It is measured to be `apply_queued`; that the cost INSIDE it
    /// is a thread snapshot is an inference, per the note above.)
    ///
    /// The shipped roster is every non-`bootstrap.` entry of
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
    /// If ANYTHING ends refused -- a queue call, a target's patch, or
    /// `MH_ApplyQueued` itself -- every target this batch QUEUED is disabled
    /// again before the freeze is dropped, and `rolled_back` says so. The
    /// per-target loops this replaced returned on the first refusal, so a later
    /// target was never enabled and the nested-before-outer orders could not be
    /// observed inverted; the completion pass CAN produce that inversion, so it
    /// is undone rather than published.
    ///
    /// THE ROLLBACK SET IS WHAT WAS QUEUED, not what is known to be patched,
    /// and the difference is the whole of lane LT's fix. A refused apply names
    /// no target and does not say whether it reached that target's bytes; a
    /// refused completion-pass enable may have written its target's bytes too,
    /// since `MH_ERROR_MEMORY_PROTECT` is the answer both when the first
    /// `VirtualProtect` refuses (nothing written) and when the protection
    /// restore or the cache flush does not complete (written, unpublished).
    /// Undoing only the targets whose status reads `Ok` therefore skips
    /// exactly the ones most likely to be live. A disable of a target that is
    /// not in fact enabled answers
    /// `MH_ERROR_DISABLED` without touching a byte, so the wider set costs one
    /// refused call and heals the incoherent latch that would otherwise refuse
    /// every later removal of that target by name.
    ///
    /// A rollback that itself fails is not hidden -- `rolled_back` stays false
    /// -- and it is not assumed to have left the target patched either, which
    /// is the same two-phase trap one step further on. `MH_DisableHook`
    /// answers `MH_ERROR_MEMORY_PROTECT` both when its first `VirtualProtect`
    /// refuses, leaving the detour in the game, and when it HAS restored the
    /// original bytes and only the publication failed, leaving the target down
    /// and `incoherent`. So the target's liveness is ASKED of MinHook rather
    /// than inferred: `isEnabled` is set after the write and not at all when
    /// the write never happened, which makes it exactly the question. It is
    /// counted in the enabled census, queued as enabled, and told to say
    /// `STILL LIVE` on the wire only when the answer is yes.
    ///
    /// The other answer is the one this cannot heal: a target left down and
    /// incoherent stays unremovable, because nothing short of re-enabling it
    /// and disabling it again re-derives the latch. That is reported -- a
    /// named refusal, `rolled_back` false -- and is a residual, not a claim
    /// (`runs/lt_apply_refusal_rollback/FINDINGS.txt`, section 5).
    ///
    /// The REPORT stays per target -- which hook failed, with which status --
    /// because that is the fidelity a whole-roster census needs. What is NOT
    /// per target is the VERDICT: `refusal()` is the one gate, and a batch
    /// that did not install every target names a refusal for every target it
    /// cannot vouch for.
    ///
    /// # Safety
    ///
    /// As [`enable`], for every target, plus the precondition above.
    /// How many times a batch is re-attempted when a thread was parked in the
    /// bytes it was about to rewrite.
    ///
    /// # Why a retry exists at all, and why it is this small
    ///
    /// The refusal is a POSITION, not an error: a game thread happened to be
    /// stopped inside a five-byte prologue at the instant of the freeze. Across
    /// a roster of ~55 targets that is not vanishingly rare, and a single-shot
    /// refusal would throw away an otherwise valid attach because of it. Each
    /// attempt is a WHOLE new freeze, so between attempts the game RUNS and the
    /// thread leaves the prologue -- which is the only thing that can change the
    /// answer. Three attempts is therefore not a timeout dressed as a loop; it
    /// is "give the game a moment to move, twice".
    ///
    /// It is bounded because the alternative is an engine wait: a thread parked
    /// in a prologue because it is BLOCKED there will never move, and retrying
    /// it forever is a capture that never starts and never says why.
    const PARKED_RETRIES: u32 = 3;

    /// How many times this thread yields between parked attempts.
    ///
    /// Not a duration and not a wait: the freeze is already dropped, so the
    /// game is running throughout. It exists because a retry that reacquires
    /// the locks and re-suspends everything before the scheduler has run the
    /// parked thread observes the same instruction pointer and learns nothing.
    const PARKED_RETRY_YIELDS: u32 = 64;

    /// Is this report a refusal that a later attempt might not repeat?
    fn refused_because_a_thread_was_parked(refusal: &Option<InlineDetourEnableError>) -> bool {
        matches!(
            refusal,
            Some(InlineDetourEnableError::FreezeRefused(
                super::freeze::FreezeRefusal::TargetExecuting { .. }
            ))
        )
    }

    /// Enable every target under ONE process freeze, retrying a batch that was
    /// refused because a thread was parked in the bytes it would rewrite.
    ///
    /// # Safety
    ///
    /// As [`enable_batch_once`].
    pub unsafe fn enable_batch(
        targets: &[(&'static str, *mut c_void)],
    ) -> BatchEnableReport {
        let mut attempts = PARKED_RETRIES;
        let mut freezes = 0u64;
        loop {
            let mut report = enable_batch_once(targets);
            attempts -= 1;
            // EVERY ATTEMPT'S FREEZES ARE CARRIED. Each attempt measures only
            // its own, and reporting the last one would tell a three-attempt
            // refusal it cost one freeze -- which is what the census reads
            // (review round 2, LOW).
            freezes = freezes.saturating_add(report.freezes);
            report.freezes = freezes;
            if attempts == 0 || !refused_because_a_thread_was_parked(&report.apply_refusal) {
                return report;
            }
            // AND THE GAME IS GIVEN A CHANCE TO MOVE. Dropping the freeze makes
            // the parked thread RUNNABLE; it does not make it RUN, and this
            // thread can reacquire both locks and suspend everything again
            // before the scheduler has looked at it -- so the same instruction
            // pointer is observed three times and the retry bought nothing
            // (review round 2, MED). Yielding is the whole of the fix: it is
            // not a wait, the game is already running, and nothing here is
            // holding a freeze.
            for _ in 0..PARKED_RETRY_YIELDS {
                std::thread::yield_now();
            }
        }
    }

    fn enable_batch_once(
        targets: &[(&'static str, *mut c_void)],
    ) -> BatchEnableReport {
        let freezes_before = super::freeze::freeze_attempts_on_this_thread();
        let mut outcomes: Vec<BatchOutcome<InlineDetourEnableError>> =
            Vec::with_capacity(targets.len());
        if targets.is_empty() {
            return BatchEnableReport::sealed(outcomes, None, &[], false, 0);
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
            return BatchEnableReport::sealed(outcomes, None, &[], false, 0);
        }
        // The module is pinned BEFORE anything is patched, exactly as
        // [`enable`] does it, and a refused pin means nothing is attempted.
        if let Err(pin) = pin_module_now() {
            for &(name, _) in targets {
                outcomes.push((name, Err(pin)));
            }
            return BatchEnableReport::sealed(outcomes, Some(pin), &[], false, 0);
        }
        // Both buffers are reserved here, before the freeze exists. Filling
        // them inside the window must not call the allocator: a suspended
        // thread holding the allocator's lock makes that a permanent deadlock.
        let mut statuses: Vec<Result<(), MH_STATUS>> = Vec::with_capacity(targets.len());
        let mut rollback_failed: Vec<bool> = Vec::with_capacity(targets.len());
        // WHETHER THE QUEUE CALL TOOK IT, snapshotted before the completion
        // pass overwrites `statuses`.
        //
        // THIS IS WHAT THE ROLLBACK IS SELECTED FROM, and it must not be
        // `statuses`. A completion-pass `MH_EnableHook` that refuses has, in
        // this fork, quite possibly WRITTEN the target's bytes already -- the
        // status reports the restore/flush, not the write -- so selecting the
        // undo from the post-pass status skips exactly the target most likely
        // to be patched. "This batch queued it" is the widest true statement
        // about what this batch may have patched, and a disable of a target
        // that is not in fact enabled answers `MH_ERROR_DISABLED` without
        // touching a byte, so the wider set costs nothing and heals more.
        let mut queued_ok: Vec<bool> = Vec::with_capacity(targets.len());
        // Whether each target is patched into the process at the moment the
        // freeze is dropped. The census and the wire's `still_live` both read
        // it, and neither may be derived from an OUTCOME any more: a rolled
        // back target reports a refusal and is not live, and a target whose
        // undo was refused reports one and IS.
        let mut live_now: Vec<bool> = Vec::with_capacity(targets.len());
        let mut apply_refusal = None;
        // Set when a held thread was stopped inside bytes the apply would have
        // rewritten. It is carried out of the window so every target's outcome
        // can name it: nothing was patched, so no target's own MinHook status
        // describes what happened to it.
        let mut parked_refusal = None;
        let rolled_back;
        let mut attempted_rollback = false;
        match freeze_other_threads() {
            Ok(mut frozen) => {
                // FROZEN WINDOW: nothing below this line until `drop(frozen)` may allocate or block.
                // Named by its FIRST target, which is how a window is attributed
                // to the lane that took it: every production caller passes the
                // `&'static str` names the pinned roster already uses.
                frozen.note_entry(
                    FreezeEntryPoint::EnableBatch,
                    targets[0].0,
                    targets.len() as u32,
                );
                for &(_, target) in targets {
                    statuses.push(unsafe {
                        MinHook::queue_enable_hook(frozen.minhook_lock(), target)
                    });
                }
                for status in statuses.iter() {
                    queued_ok.push(status.is_ok());
                }
                let mut queued_any = queued_ok.iter().any(|queued| *queued);
                // NOTHING IS PATCHED UNTIL THIS PASSES. A held thread stopped
                // inside any byte the apply would rewrite refuses the whole
                // batch: `queued_any` goes false, so the normalisation loop
                // below un-queues every target, the freeze is dropped with the
                // game exactly as it was found, and every target reports the
                // refusal. Retrying is the caller's -- by then the game has run
                // on and the thread has left the prologue.
                if queued_any {
                    if let Err(refusal) = frozen.refuse_if_a_thread_is_in_a_queued_patch() {
                        parked_refusal = Some(refusal);
                        apply_refusal = Some(InlineDetourEnableError::FreezeRefused(refusal));
                        queued_any = false;
                    }
                }
                if queued_any {
                    if let Err(apply) = frozen.time_minhook(|lock| unsafe {
                        MinHook::apply_queued(lock)
                    }) {
                        // Charged to the BATCH, not to a target: the apply
                        // stopped at a target it does not name, and it does
                        // not say whether it reached that target's bytes
                        // before it stopped. `still_live` is stamped when the
                        // report is sealed, once the rollback below has
                        // answered.
                        apply_refusal = Some(InlineDetourEnableError::BatchApplyRefused {
                            status: apply,
                            // Both stamped when the report is sealed, once the
                            // rollback below has answered.
                            still_live: false,
                            rolled_back: false,
                        });
                        // FINISH THE PATCHING ANYWAY, THEN UNDO ALL OF IT.
                        //
                        // This looks like wasted work and is not: MinHook does
                        // not say where the apply stopped, so after a refusal
                        // nobody knows which targets are patched. The pass
                        // ATTEMPTS completion over every queued target -- it
                        // cannot guarantee it, because a completion enable
                        // whose own first `VirtualProtect` refuses writes
                        // nothing and leaves that target down -- which is
                        // precisely why the rollback below covers the whole
                        // QUEUED set rather than the ones this pass says it
                        // patched. Undoing a target that was never patched
                        // writes nothing; undoing one that was is what
                        // RE-COMPLETES the incoherent one (`MH_DisableHook`
                        // rewrites its bytes and re-derives the latch, and
                        // until that happens `MH_RemoveHook` refuses it by
                        // name forever).
                        for (index, &(_, target)) in targets.iter().enumerate() {
                            if statuses[index].is_ok() {
                                statuses[index] = match unsafe {
                                    MinHook::enable_hook(frozen.minhook_lock(), target)
                                } {
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
                // Anything refused -- at queue time, at patch time, OR BY THE
                // APPLY ITSELF -- undoes the whole batch, so no game thread
                // sees a partial set in an order MinHook's creation table
                // chose.
                //
                // THE `apply_refusal` HALF IS THE FIX. It used to be absent,
                // and the state that produced was the one lane LN drove: the
                // apply refuses after writing a target's bytes, the completion
                // pass re-enables every queued target, MinHook answers
                // `MH_ERROR_ENABLED` for the one already patched, that maps to
                // `Ok`, so `any_refusal` was FALSE and the whole roster went
                // live with one detour's last write unpublished
                // (`runs/ln_fault_inject/FINDINGS.txt`). A refused apply means
                // one target MAY be patched with its write unpublished, and
                // MinHook names neither the target nor whether it got that far
                // -- so all of them come back down. Undoing one that was never
                // patched costs a refused `MH_ERROR_DISABLED` and writes
                // nothing, which is why the conservative set is the safe one.
                let batch_refused =
                    statuses.iter().any(|status| status.is_err()) || apply_refusal.is_some();
                for (index, &(_, target)) in targets.iter().enumerate() {
                    let undo = queued_ok[index] && queued_any && batch_refused;
                    let mut failed_undo = false;
                    if undo {
                        attempted_rollback = true;
                        // ALREADY DISABLED IS A SUCCESSFUL UNDO. The rollback
                        // set is every target this batch QUEUED, which is
                        // wider than the set it is known to have patched, so a
                        // target the apply never reached answers
                        // `MH_ERROR_DISABLED` here -- nothing was patched and
                        // nothing is left patched, which is precisely what the
                        // undo asked for. Counting it as a failed undo would
                        // make `rolled_back` false over a batch that IS fully
                        // rolled back.
                        failed_undo = !matches!(
                            unsafe { MinHook::disable_hook(frozen.minhook_lock(), target) },
                            Ok(())
                                | Err(MH_STATUS::MH_ERROR_DISABLED)
                                | Err(MH_STATUS::MH_ERROR_NOT_CREATED)
                        );
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
                    // that was QUEUED AND NEVER PATCHED: `queueEnable` true,
                    // `isEnabled` false, divergent, waiting for somebody
                    // else's apply to patch it inside a freeze nobody
                    // accounted for.
                    //
                    // Two things leave that shape, and neither can be driven
                    // here. A queue call that succeeded for a target the apply
                    // never reached -- which the completion pass now closes,
                    // because it patches every queued target before the
                    // rollback takes them down. And an `EnableHookLL` whose
                    // FIRST `VirtualProtect` fails: it returns before touching
                    // either flag, so the target keeps this batch's
                    // `queueEnable` over an `isEnabled` of false, and the
                    // rollback's `MH_DisableHook` answers `MH_ERROR_DISABLED`
                    // without writing anything, leaving it divergent. THIS
                    // line is what equalises it. Nothing available here can
                    // make `VirtualProtect` fail on a page this process just
                    // wrote, so it stands on MinHook's own source and on
                    // review rather than on a test.
                    // A REFUSED UNDO IS NOT THE SAME AS A LIVE TARGET, and the
                    // status cannot tell them apart. `MH_DisableHook` answers
                    // `MH_ERROR_MEMORY_PROTECT` both when its first
                    // `VirtualProtect` refuses -- nothing restored, the detour
                    // is still in the game -- and when it HAS restored the
                    // original bytes and the protection restore or the cache
                    // flush did not publish. Inferring "still live" from the
                    // refusal is right in the first case and wrong in the
                    // second, and it is the wrong one that a custody mask acts
                    // on: it would send a teardown to disable a target that is
                    // already down.
                    //
                    // So ask. `isEnabled` is set by `EnableHookLL` AFTER the
                    // write and not at all when the write never happened, which
                    // makes it exactly "are this target's bytes the patch right
                    // now" in all four of those corners. Asked only on the one
                    // path where the inference is uncertain, so the clean
                    // install pays nothing for it.
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
                return BatchEnableReport::sealed(
                    outcomes,
                    apply_refusal,
                    &[],
                    false,
                    super::freeze::freeze_attempts_on_this_thread()
                        .saturating_sub(freezes_before),
                );
            }
        }
        let mut enabled = 0u64;
        for (index, &(name, _)) in targets.iter().enumerate() {
            let outcome = match parked_refusal {
                // The batch was refused before it patched anything, so no
                // target's queue status is an account of what happened to it.
                Some(refusal) => Err(InlineDetourEnableError::FreezeRefused(refusal)),
                None => statuses[index].map_err(InlineDetourEnableError::MinHook),
            };
            // FROM THE LIVE STATE, NOT FROM THE OUTCOME. The completion pass
            // calls MinHook directly and does not bump the census itself, so
            // this is the only place any of them is counted and none is
            // counted twice -- but what it counts has to be "patched into the
            // process right now", and an outcome is no longer that. A batch
            // rolled back after a refused apply reports a refusal for every
            // target and none of them is live; a target whose undo was refused
            // reports one too and IS live, and it is the one the census may
            // not miss.
            //
            // Decided per POSITION, because a mixed rollback -- some undos
            // accepted, one refused -- leaves only the refused one live, and a
            // batch-wide flag would count the ones that came back down with
            // it.
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
        // WHICH TARGETS THIS BATCH FOUND PATCHED, read before anything is
        // queued and reserved before the freeze exists.
        //
        // A teardown is routinely handed its WHOLE roster when part of it is
        // already down -- every failed-install cleanup in this workspace does
        // exactly that. The restore-behind-a-refusal arm below then re-enables
        // targets to undo an apply that had taken them down, and for a target
        // that was ALREADY down it does not undo anything: it INSTALLS a
        // detour, from inside a teardown, and the caller's custody mask does
        // not name it. This is what stops that.
        let mut live_at_entry: Vec<bool> = Vec::with_capacity(targets.len());
        let mut apply_refusal = None;
        // Set when a held thread was stopped inside bytes this release would
        // have restored. Nothing is unpatched then, so no target's own MinHook
        // status is an account of what happened to it.
        let mut parked_refusal = None;
        let mut frozen = match freeze_other_threads() {
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
            statuses.push(unsafe {
                MinHook::queue_disable_hook(frozen.minhook_lock(), target)
            });
        }
        let mut queued_any = statuses.iter().any(|status| status.is_ok());
        // THE SAME REFUSAL AS THE INSTALL DIRECTION, and it matters just as
        // much: restoring a target's original bytes underneath a thread stopped
        // inside the patch tears its instruction stream exactly as installing
        // does. A refusal here unpatches nothing, so every target is still live
        // and every one is reported retained.
        if queued_any {
            if let Err(refusal) = frozen.refuse_if_a_thread_is_in_a_queued_patch() {
                parked_refusal = Some(refusal);
                apply_refusal = Some(InlineDetourReleaseError::FreezeRefused(refusal));
                queued_any = false;
                // THE QUEUE IS PUT BACK BY THE NORMALISATION LOOP BELOW, which
                // this used to duplicate. Nothing was applied, so every target
                // this batch queued is still in whatever state it was found in,
                // and a `queueEnable` of FALSE against an `isEnabled` of TRUE
                // is a divergent entry that some later, unrelated
                // `apply_queued` performs inside a freeze nobody accounted for
                // (review round 1, MED). The loop below asks MinHook for
                // `isEnabled` and equalises against it for EVERY target in
                // BOTH directions, which is strictly what the copy that stood
                // here did and also covers the case it did not (review round
                // 4). One mechanism, not two.
            }
        }
        if queued_any {
            if let Err(apply) = frozen.time_minhook(|lock| unsafe {
                MinHook::apply_queued(lock)
            }) {
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
                        // AND ONLY IF THIS BATCH FOUND IT PATCHED. Re-enabling
                        // is how a target the apply took down is put back; for
                        // one that was already down when the batch started it
                        // is not a restore at all, it is a teardown installing
                        // a detour the caller never asked for and does not
                        // know about. Its release postcondition already holds
                        // -- nothing is patched -- so it is reported as the
                        // already-released it is.
                        if !live_at_entry[index] {
                            statuses[index] = Err(MH_STATUS::MH_ERROR_DISABLED);
                            continue;
                        }
                        // Behind the refusal: restore it if the apply took it
                        // down. `MH_ERROR_ENABLED` means it never did.
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
        // Leave `queueEnable` equal to `isEnabled` for EVERY target, so no
        // divergent entry is left for an unrelated `MH_ApplyQueued` to act on.
        //
        // ASKED, NOT INFERRED, and that is the whole of it. This used to read
        // "still enabled" off the target's status: `Ok` or an already-released
        // status meant down, anything else meant up. `MH_ERROR_MEMORY_PROTECT`
        // breaks that in both directions, because `EnableHookLL` answers it
        // before it writes anything AND after it has written and failed only
        // to publish. The reachable one is a restore-behind-a-refusal whose
        // own first `VirtualProtect` fails: nothing is written, the target
        // stays DOWN, the status says refused, and the inference queued it
        // ENABLED -- a divergent entry that the next unrelated apply installs,
        // inside a freeze nobody accounted for, during a teardown (review
        // round 4, HIGH).
        //
        // `isEnabled` is what `MH_ApplyQueued` compares `queueEnable` against,
        // so asking for it is asking the exact question this loop exists to
        // answer. A target MinHook cannot find has no entry to leave divergent.
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
        drop(frozen);
        let mut retained = 0u64;
        for (index, &(name, target)) in targets.iter().enumerate() {
            let raw = match parked_refusal {
                // Refused before anything was unpatched: the target's queue
                // status says nothing about what happened to it, and what
                // happened to it is that it is still live.
                Some(refusal) => Err(InlineDetourReleaseError::FreezeRefused(refusal)),
                None => statuses[index].map_err(InlineDetourReleaseError::MinHook),
            };
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
    /// `MAXIMUM_SUSPEND_COUNT`: `MH_RemoveHook` no longer suspends anything at
    /// all, so N removals under one freeze add nothing to any thread's suspend
    /// depth beyond the one this freeze took. The sentence this replaces
    /// described the vendored library before `minhook_h4` deleted its
    /// suspension.
    ///
    /// EVERY TARGET'S TRAMPOLINE IS IN THE PRE-PATCH CHECK, because a removal
    /// FREES it: a thread stopped inside a slot this call would free resumes
    /// into freed memory, and that refuses the whole batch before anything is
    /// removed.
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
                    freezes: super::freeze::freeze_attempts_on_this_thread()
                        .saturating_sub(freezes_before),
                };
            }
        };
        // FROZEN WINDOW: nothing below this line until `drop(frozen)` may allocate or block.
        frozen.note_entry(
            FreezeEntryPoint::RemoveBatch,
            targets[0].0,
            targets.len() as u32,
        );
        // TIMED AROUND THE WHOLE LOOP, not around one call, and the difference
        // is stated because it changes what the row means: `remove_batch` has no
        // apply -- `MH_RemoveHook` is per target and there is no queued form --
        // so this window's MinHook cost is N removals rather than one apply. The
        // target count is on the row beside it, so a reader divides rather than
        // being told a per-call figure that was never measured.
        // BOTH RANGES, because a remove frees the trampoline as well as
        // restoring the target: a thread stopped in either resumes into torn
        // bytes or into freed memory. There is no queue to ask about -- removal
        // is per target and has no queued form -- so the caller's own targets
        // are the set, which is exact here for the same reason it would have
        // been wrong for an apply.
        let parked_refusal = frozen
            .refuse_if_a_thread_is_in(targets.iter().map(|&(_, target)| target), true)
            .err();
        if parked_refusal.is_none() {
            let statuses_ref = &mut statuses;
            frozen.time_minhook(|lock| {
                for &(_, target) in targets {
                    // THE SAME QUIESCENCE REFUSAL [`remove`] MAKES, per target.
                    // This frees every trampoline in the batch, so a target
                    // still admitting callbacks is the same use-after-free one
                    // step wider: the caller cannot have drained a detour body
                    // that is still being entered. Only `Ok(true)` refuses --
                    // `MH_ERROR_NOT_CREATED` must still reach the caller as
                    // itself, because the batch's `already_released` roster is
                    // built from it.
                    //
                    // Refusing one target does not refuse the batch. The others
                    // are genuinely quiescent and their trampolines are freed;
                    // this one is reported retained, by name, in `failures()`.
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
        // Counted after the resume; the loop above only recorded the answer.
        RETAINED_DETOURS.fetch_add(
            statuses
                .iter()
                .filter(|status| {
                    matches!(status, Err(InlineDetourReleaseError::StillEnabled))
                })
                .count() as u64,
            Ordering::AcqRel,
        );
        if let Some(refusal) = parked_refusal {
            // Nothing was removed and nothing was freed. Every target keeps its
            // trampoline and its patch, which is what `detour_retained` means.
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
        // FROZEN WINDOW: nothing below this line until `drop(frozen)` may allocate or block.
        frozen.note_entry(FreezeEntryPoint::Disable, "", 1);
        // A disable rewrites the same bytes an enable does, so it refuses on the
        // same question. The trampoline is NOT in the check: a disable leaves it
        // allocated, and a thread inside it stays valid.
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
    /// # Contract: THE CALLER MUST PROVE ITS DETOUR BODY IS QUIESCENT
    ///
    /// This is the half of the question the freeze cannot answer, and it is
    /// stated here because until now it was only implied.
    ///
    /// `refuse_if_a_thread_is_in(.., true)` asks whether any frozen thread's
    /// instruction pointer is inside the TARGET or the TRAMPOLINE. A thread
    /// executing the caller's own detour body is inside NEITHER -- the detour
    /// is the caller's function, at an address this crate is never told -- and
    /// yet it is the thread most likely to be there, because that is where a
    /// detour spends its time. When it finishes it RETURNS THROUGH THE
    /// TRAMPOLINE, into memory `MH_RemoveHook` has by then freed.
    ///
    /// So this call is sound only when the caller has already established, for
    /// this target:
    ///
    /// 1. ADMISSION IS CLOSED. No new callback may enter the detour body.
    ///    Closing it AFTER measuring is not closing it: a count taken while
    ///    entries are still admitted is stale before it is read.
    /// 2. THE BODIES IN FLIGHT HAVE DRAINED TO ZERO, measured after (1), by a
    ///    counter the detour itself increments on entry and decrements after
    ///    its call through the original returns -- not before it.
    /// 3. NEITHER (1) NOR (2) HAS BEEN UNDONE since, which in practice means
    ///    both are established under the same lock this release runs under.
    ///
    /// [`disable`] is the ordinary way to get (1), and this call REFUSES with
    /// [`InlineDetourReleaseError::StillEnabled`] if the target is not already
    /// disabled, because a caller that has not disabled cannot have satisfied
    /// (1) at all -- its detour is still live and still admitting. That refusal
    /// is necessary, not sufficient: (2) and (3) are the caller's, and nothing
    /// here can check them.
    ///
    /// A hook that was CREATED but never ENABLED has no detour body to drain --
    /// nothing was ever patched into the game, so no thread can have entered
    /// it -- and this refusal passes it, which is why rollback paths need no
    /// drain at all.
    ///
    /// `supply_logistics_attrition_live`'s tooltip teardown is the worked
    /// example -- an `ADMISSION_OPEN` flag cleared before any drain begins, a
    /// bounded drain of `CALLBACKS_IN_FLIGHT`, and a refusal that reports the
    /// target as retained rather than freeing it.
    ///
    /// # Safety
    ///
    /// As `MH_RemoveHook`, plus the contract above. On a retained refusal
    /// nothing was freed and nothing was restored, so the caller's trampoline
    /// and detour must both stay valid for the life of the process.
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
        // FROZEN WINDOW: nothing below this line until `drop(frozen)` may allocate or block.
        frozen.note_entry(FreezeEntryPoint::Remove, "", 1);
        // STEP 1 OF THE CONTRACT, AS FAR AS THIS CRATE CAN SEE IT, AND IT
        // FAILS CLOSED. A target still enabled is still admitting callbacks, so
        // the caller cannot have drained its detour body -- whatever count it
        // took is stale, and the trampoline this call would free is one a body
        // in flight will return through.
        //
        // A REFUSAL AND NOT AN ASSERTION, for two reasons the first spelling of
        // this got wrong. A `debug_assert!` checks nothing in the shipped
        // build, which is the build that runs inside a player's game; and it
        // would unwind from INSIDE THE HELD FREEZE, where the panic machinery
        // itself formats and allocates -- the exact hang this whole audit
        // exists to prevent, reached through the guard against it.
        //
        // So the question is asked while frozen, where it is cheap and cannot
        // go stale (`MH_HookIsEnabled` checks lock ownership, walks the
        // existing hook table and reads a boolean; it allocates nothing and
        // waits for nothing, and this thread already holds the lock), and the
        // ANSWER IS ACTED ON AFTER THE RESUME.
        //
        // ONLY `Ok(true)` IS "STILL ENABLED", and the other answers are not
        // this refusal's business. `MH_ERROR_NOT_CREATED` means MinHook has no
        // hook here at all -- no trampoline to free and no body to strand --
        // and it MUST reach the caller as itself, because every per-target
        // teardown in the workspace reads it as nothing-to-do and a second,
        // idempotent release depends on it. Any other status is one
        // `MH_RemoveHook` will refuse on its own, in its own vocabulary.
        let enabled = matches!(
            MinHook::hook_is_enabled(frozen.minhook_lock(), target),
            Ok(true)
        );
        // THE TRAMPOLINE IS IN THE CHECK HERE, and that is the difference from
        // [`disable`]: `MH_RemoveHook` frees the slot, so a thread stopped
        // inside it resumes into freed memory.
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
    freeze_refusals, suspend_failures_deferred_to_a_later_pass,
    suspend_failures_excused_as_terminated, FreezeRefusal, ProcessFreeze,
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
/// and stop them -- without a thread walk there is no list to walk, and without
/// `SuspendThread` there is nothing to stop it with.
///
/// `NtGetNextThread` IS covered, and used not to be. The note here said it was
/// out of scope because reaching for an undocumented ntdll export was an edit a
/// reviewer would see -- which was true while nothing in the tree did it. The
/// shared freeze now enumerates with exactly that call, so the pattern is
/// demonstrated in-tree and copying it is no longer an unusual thing for a lane
/// to write. A ban list that omits the primitive its own crate uses is the one
/// shape this scan cannot afford.
///
/// STILL NOT COVERED, in those words: a raw `syscall` instruction, a suspender
/// written in a `.c` file, and a fourth-party crate. Each needs an edit that is
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
    ("NtGetNext", "Thread("),
    ("ZwGetNext", "Thread("),
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
        // The release LEDGER first, then the suspend state -- the order every
        // other test in this crate takes them in. `disable` and `remove` below
        // record release attempts into a process-global ledger whose capacity
        // test asserts an exact dropped count under this lock, so a writer that
        // skipped it could land inside that assertion's window.
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
        // The release LEDGER is process-global and every one of these
        // tests writes into it -- `create_victims` removes any prior
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        // count under this same lock, so a batch test that skipped it
        // could add an entry inside that assertion's window.
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
    /// `freeze_other_threads` call, which is one sizing walk of this process's
    /// thread list plus at least two fixed-point walks each.
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
        // The release LEDGER is process-global and every one of these
        // tests writes into it -- `create_victims` removes any prior
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        // count under this same lock, so a batch test that skipped it
        // could add an entry inside that assertion's window.
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
    /// The empty case is the one that must cost nothing: a freeze walks this
    /// process's thread list and suspends every game thread, and paying that for
    /// a batch with no work is pure loss. The all-absent case is the
    /// deliberate price of putting the queue calls inside the freeze, and it
    /// is bounded here so that price stays one freeze and not one per target.
    ///
    /// What production change would make this fail? Removing the empty-batch
    /// early return, or moving the queue calls back outside the freeze and
    /// taking a second one to apply them.
    #[test]
    fn an_empty_batch_never_freezes_and_a_broken_roster_freezes_at_most_once() {
        let _serial = BATCH_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The release LEDGER is process-global and every one of these
        // tests writes into it -- `create_victims` removes any prior
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        // count under this same lock, so a batch test that skipped it
        // could add an entry inside that assertion's window.
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

        // A roster whose every target is absent still takes the ONE freeze,
        // because the queue calls are inside it -- that is the deliberate
        // price of a queue that cannot leak. It must not take more than one,
        // and every target must still be named.
        let all_absent = unsafe { enable_batch(&[("absent.a", ABSENT_TARGET)]) };
        assert_eq!(all_absent.freezes(), 1);
        assert_eq!(all_absent.failures().len(), 1);
        assert_eq!(all_absent.refusal().map(|(name, _)| name), Some("absent.a"));

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
        // The release LEDGER is process-global and every one of these
        // tests writes into it -- `create_victims` removes any prior
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        // count under this same lock, so a batch test that skipped it
        // could add an entry inside that assertion's window.
        let _ledger = crate::release_ledger::TEST_LEDGER_SERIAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
        // The release LEDGER is process-global and every one of these
        // tests writes into it -- `create_victims` removes any prior
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        // count under this same lock, so a batch test that skipped it
        // could add an entry inside that assertion's window.
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

    /// EVERY PATH THAT PATCHES ASKS FIRST, asserted against the source.
    ///
    /// The behavioural proof of the refusal itself lives in `freeze.rs`
    /// (`a_thread_inside_the_bytes_a_patch_would_rewrite_refuses_it`), driven
    /// with a range that certainly contains a held thread. What that test
    /// CANNOT show is that production is wired to it -- deleting the check from
    /// `enable` would leave it green (review round 1, MED). This is that half:
    /// every window that calls a MinHook operation which REWRITES BYTES must
    /// first call one of the two refusals, and the call must come before it.
    ///
    /// What production change would make this fail? Removing
    /// `refuse_if_a_thread_is_in`/`refuse_if_a_thread_is_in_a_queued_patch`
    /// from any patch path, or moving it after the patch -- which is where the
    /// rejected first design put it.
    #[test]
    fn every_patching_window_refuses_before_it_patches() {
        let source = include_str!("lib.rs").replace("
", "
");
        let marker = concat!("// FROZEN WI", "NDOW:");
        let closer = concat!("drop(fro", "zen);");
        // The operations that rewrite live bytes. Queueing does not.
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
            let first_patch = patching
                .iter()
                .filter_map(|call| window.find(call))
                .min();
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
            // THE ONE ADDED BY THE MINHOOK FORK. Acquiring MinHook's lock is
            // the only call in that library that can wait, and a wait inside a
            // held freeze is the hang this lane closed. Every window takes the
            // lock from the freeze that already holds it
            // (`frozen.minhook_lock()`), and an acquisition written here would
            // be the defect coming back.
            concat!("MinHookLock::", "acquire"),
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
    /// the batch costs one for the whole unit. Each freeze walks this process's
    /// thread list and suspends and resumes every other thread in it, and it is
    /// the COUNT -- not a wall time taken on a machine running five other build
    /// lanes -- that this API exists to move.
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
        // The release LEDGER is process-global and every one of these
        // tests writes into it -- `create_victims` removes any prior
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        // count under this same lock, so a batch test that skipped it
        // could add an entry inside that assertion's window.
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
        // Stated as the ratio the migration claims, so a partial regression --
        // say, a batch that freezes twice -- fails here rather than passing a
        // "fewer than before" comparison.
        assert_eq!(
            singles_enable + singles_disable,
            (enabled.freezes() + released.freezes()) * 12,
            "12x on the install-plus-release span"
        );
    }

    /// Falsifiable: fails if a real batch install leaves the production freeze
    /// census with nothing to say about it.
    ///
    /// THE DEFECT CLASS THIS CLOSES is not a wrong number, it is a missing one.
    /// `FREEZE_ATTEMPTS` existed for months with zero consumers, and every
    /// stopped-window timer lived in a test module, so a recording could prove
    /// forty-five hooks installed and say NOTHING about how many times the game
    /// was stopped to install them or for how long. The adversarial review of
    /// lane t54's leg 5 is that finding. A census that silently recorded nothing
    /// would reproduce it exactly.
    ///
    /// What production change would make this fail? Removing the `record` call
    /// from `ProcessFreeze::drop`; dropping the `timing` field so it is never
    /// filled; removing `note_entry` from `enable_batch`, which would leave a
    /// real batch attributed to `freeze_process`; or removing `time_minhook`,
    /// which would leave `minhook_us` at zero on a window that really did call
    /// `MH_ApplyQueued`.
    ///
    /// It does NOT assert a duration THRESHOLD. What a freeze costs is the
    /// question the instrument exists to answer on a real attach, and a test
    /// that pinned a millisecond figure on a build machine would be asserting
    /// the machine. What is asserted is that the window is bounded by nothing
    /// and STRICTLY POSITIVE, which a clock read that never happened cannot be,
    /// and that the parts do not exceed the whole.
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
        // DISABLED BEFORE REMOVED, which is now the contract rather than a
        // courtesy. This test used to go straight from `enable_batch` to
        // `remove_batch`, and that is exactly the shape `remove`'s quiescence
        // contract refuses: a target still enabled is still admitting, so no
        // drain of its detour body can have been stable, and removal frees the
        // trampoline a body in flight returns through. Its sibling
        // `twelve_hooks_cost_twelve_freezes_one_at_a_time_and_one_as_a_batch`
        // always did it in this order.
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
        // The batch's own row, found by the name the caller passed. Searched by
        // LABEL rather than taken as the last row, because these tests run
        // beside others that freeze.
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
        // And the removal's row, which is the teardown half of the census and is
        // the one that proves the instrument is not install-only.
        assert!(
            rows.iter().rev().any(|row| {
                row.entry_point == Some(crate::freeze_census::FreezeEntryPoint::RemoveBatch)
                    && row.label == targets[0].0
            }),
            "the release half of the capture records no window; a teardown that \
             stops the game is a stopped window too"
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
        // The release LEDGER is process-global and every one of these
        // tests writes into it -- `create_victims` removes any prior
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        // count under this same lock, so a batch test that skipped it
        // could add an entry inside that assertion's window.
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
        // The release LEDGER is process-global and every one of these
        // tests writes into it -- `create_victims` removes any prior
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        // count under this same lock, so a batch test that skipped it
        // could add an entry inside that assertion's window.
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
        // The release LEDGER is process-global and every one of these
        // tests writes into it -- `create_victims` removes any prior
        // instance, and the batch releases below record every attempt.
        // `release_ledger`'s own capacity test asserts an exact dropped
        // count under this same lock, so a batch test that skipped it
        // could add an entry inside that assertion's window.
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
