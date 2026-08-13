//! Every attempt to take a detour back out, recorded where it cannot be lost.
//!
//! # The overstatement this exists to name
//!
//! A lane teardown disables its targets in a loop and clears its `INSTALLED`
//! latch only after the whole loop is clean. One refused disable therefore
//! returns `Err` with the latch still set — which is the RIGHT direction (some
//! targets really are still patched) and is deliberately retryable, because a
//! refused freeze is usually transient and the next attempt heals it.
//!
//! What it was not, until this module, is RECORDED. `INSTALLED == true` after
//! a partial disable overstates how much is installed — some targets came off,
//! some did not — and nothing anywhere said which, or that an attempt had been
//! made at all. A retry that then succeeded erased even the fact that anything
//! had gone wrong. An operator reading a capture could not tell a teardown that
//! worked first time from one that failed four times and worked on the fifth.
//!
//! # What this is NOT
//!
//! It is NOT a one-way "removal unproved" latch. The owner's ruling
//! (2026-08-05) is explicit that the retry path stays: a transient failure can
//! heal, and a latch that refused to un-set would turn every healed teardown
//! into a permanently failed capture. What is recorded is the ATTEMPT SEQUENCE,
//! so `attempt -> failure -> retry -> success` is readable in that order and
//! the success does not overwrite the failure.
//!
//! # Why it lives in this crate
//!
//! Because this crate is the only way bytes are restored. `disable` and
//! `remove` are the choke point the source ban keeps closed, so a ledger kept
//! here covers every lane with no lane opt-in and no lane able to forget. A
//! ledger kept in each lane would cover whichever lanes remembered to write to
//! it, which is the failure mode `inline_detour` was extracted to end.
//!
//! # Cost and bounds
//!
//! One mutex acquisition per release attempt, on a teardown path that has just
//! frozen and unfrozen the entire process — unmeasurable beside that. The
//! history is capped at [`HISTORY_CAPACITY`] entries with an overflow counter,
//! so a pathological retry loop cannot grow it without bound; the PER-TARGET
//! summary is never dropped, so the question "did this target's last attempt
//! fail" stays answerable however long the history is.
//!
//! NOTHING HERE RUNS UNDER A FREEZE. Every recording call sits after the
//! `ProcessFreeze` guard has been dropped, which matters: taking a mutex or
//! allocating while every other thread is suspended is how an injected DLL
//! deadlocks a game.

use crate::windows_impl::InlineDetourReleaseError;
use std::sync::Mutex;

/// How many individual attempts are kept in order before the history starts
/// counting overflow instead of storing it.
///
/// The shipped roster is every non-`bootstrap.` entry of
/// `compact_contract::passive_ready::PINNED_PRODUCTION_HOOK_SPECS_V1` -- it was
/// 55 detours when this constant was chosen and moves whenever a domain is
/// scheduled or descheduled -- and each is disabled and then removed, so a
/// clean teardown writes two entries per detour. This leaves room for several
/// full retry rounds on top of that before anything is dropped, and a teardown
/// that has retried more than that has a per-target summary saying so.
pub const HISTORY_CAPACITY: usize = 512;

/// Which byte-restoring operation an attempt was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseOperation {
    /// `MH_DisableHook`: restore the target's bytes, keep the trampoline.
    Disable,
    /// `MH_RemoveHook`: restore the bytes if still patched, then free the
    /// trampoline.
    Remove,
}

impl ReleaseOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Remove => "remove",
        }
    }
}

/// How one attempt ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseAttemptOutcome {
    /// The operation ran and MinHook accepted it.
    Succeeded,
    /// There was nothing to do: MinHook reports the hook already disabled or
    /// never created. Not a failure, and deliberately distinguished from
    /// `Succeeded` so a reader can tell a teardown that restored bytes from one
    /// that found them already restored.
    AlreadyReleased,
    /// The operation was refused and the detour may still be patched in.
    Refused(InlineDetourReleaseError),
}

impl ReleaseAttemptOutcome {
    /// Classify a release result exactly as every lane's own teardown does:
    /// `MH_ERROR_DISABLED` and `MH_ERROR_NOT_CREATED` are nothing-to-do, and
    /// everything else — including a refused FREEZE, which means the detour is
    /// still live — is a refusal.
    pub fn of(result: Result<(), InlineDetourReleaseError>) -> Self {
        match result {
            Ok(()) => Self::Succeeded,
            Err(error) if error.already_released() => Self::AlreadyReleased,
            Err(error) => Self::Refused(error),
        }
    }

    /// Did this attempt leave the target released?
    pub fn released(self) -> bool {
        matches!(self, Self::Succeeded | Self::AlreadyReleased)
    }
}

/// One recorded attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReleaseAttempt {
    /// Runtime address of the patched target. Process-local; never serialize
    /// it, exactly as `production_hook_receipt` says of its own addresses.
    pub target: usize,
    /// The caller's name for the target when it gave one.
    pub name: Option<&'static str>,
    pub operation: ReleaseOperation,
    /// 1-based, counted per (target, operation). THE field that makes a retry
    /// legible as a retry rather than as a second unrelated event.
    pub attempt: u32,
    pub outcome: ReleaseAttemptOutcome,
}

/// The running per-target summary, which is never dropped for capacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TargetSummary {
    target: usize,
    name: Option<&'static str>,
    disable_attempts: u32,
    remove_attempts: u32,
    refusals: u32,
    last: ReleaseAttemptOutcome,
}

#[derive(Default)]
struct Ledger {
    history: Vec<ReleaseAttempt>,
    dropped: u64,
    summaries: Vec<TargetSummary>,
}

static LEDGER: Mutex<Option<Ledger>> = Mutex::new(None);

fn with_ledger<T>(op: impl FnOnce(&mut Ledger) -> T) -> T {
    let mut guard = LEDGER.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    op(guard.get_or_insert_with(Ledger::default))
}

/// Give `target` a name so later reports can use it instead of an address.
///
/// Idempotent, and the FIRST name wins: two callers naming the same target
/// differently is a bug in the callers, and silently taking the second name
/// would make the report depend on call order.
pub fn name_release_target(target: usize, name: &'static str) {
    with_ledger(|ledger| {
        if let Some(summary) = ledger.summaries.iter_mut().find(|s| s.target == target) {
            summary.name.get_or_insert(name);
        } else {
            ledger.summaries.push(TargetSummary {
                target,
                name: Some(name),
                disable_attempts: 0,
                remove_attempts: 0,
                refusals: 0,
                last: ReleaseAttemptOutcome::Succeeded,
            });
            // A summary that has recorded no attempt must not read as a
            // successful release, so mark it as having no attempts yet by
            // leaving the counters at zero; `last` is only consulted through
            // `last_release_attempt`, which requires a non-zero attempt count.
        }
    });
}

/// Record one attempt and return the attempt ordinal it was given.
pub(crate) fn record_release_attempt(
    target: usize,
    operation: ReleaseOperation,
    result: Result<(), InlineDetourReleaseError>,
) -> u32 {
    let outcome = ReleaseAttemptOutcome::of(result);
    with_ledger(|ledger| {
        let index = match ledger.summaries.iter().position(|s| s.target == target) {
            Some(index) => index,
            None => {
                ledger.summaries.push(TargetSummary {
                    target,
                    name: None,
                    disable_attempts: 0,
                    remove_attempts: 0,
                    refusals: 0,
                    last: outcome,
                });
                ledger.summaries.len() - 1
            }
        };
        let summary = &mut ledger.summaries[index];
        let attempt = match operation {
            ReleaseOperation::Disable => {
                summary.disable_attempts = summary.disable_attempts.saturating_add(1);
                summary.disable_attempts
            }
            ReleaseOperation::Remove => {
                summary.remove_attempts = summary.remove_attempts.saturating_add(1);
                summary.remove_attempts
            }
        };
        if matches!(outcome, ReleaseAttemptOutcome::Refused(_)) {
            summary.refusals = summary.refusals.saturating_add(1);
        }
        summary.last = outcome;
        let name = summary.name;
        if ledger.history.len() < HISTORY_CAPACITY {
            ledger.history.push(ReleaseAttempt {
                target,
                name,
                operation,
                attempt,
                outcome,
            });
        } else {
            ledger.dropped = ledger.dropped.saturating_add(1);
        }
        attempt
    })
}

/// Every attempt this process has recorded, in the order they happened.
pub fn release_attempt_history() -> Vec<ReleaseAttempt> {
    with_ledger(|ledger| ledger.history.clone())
}

/// How many attempts were counted but not stored because the history is full.
pub fn release_attempts_dropped_for_capacity() -> u64 {
    with_ledger(|ledger| ledger.dropped)
}

/// How this target's most recent attempt ended, or `None` if no attempt to
/// release it has ever been recorded.
///
/// `None` is deliberately NOT "released". A caller asking about a target that
/// was never attempted has learned nothing from this ledger and must decide
/// from its own custody.
pub fn last_release_attempt(target: usize) -> Option<ReleaseAttemptOutcome> {
    with_ledger(|ledger| {
        ledger
            .summaries
            .iter()
            .find(|s| s.target == target)
            .filter(|s| s.disable_attempts != 0 || s.remove_attempts != 0)
            .map(|s| s.last)
    })
}

/// True when this target was attempted and its LAST attempt was refused.
///
/// THE predicate a release receipt consults before it claims a hook is proven
/// removed. It is deliberately about the last attempt and not about any
/// attempt: an earlier failure that a retry healed is history, not an
/// outstanding overstatement, and refusing over it would be the one-way latch
/// the owner ruled against.
pub fn release_is_unproven(target: usize) -> bool {
    matches!(
        last_release_attempt(target),
        Some(ReleaseAttemptOutcome::Refused(_))
    )
}

/// Every target whose last attempt was refused, for a report that has to name
/// them all rather than stop at the first.
pub fn targets_with_a_refused_last_release_attempt() -> Vec<(usize, Option<&'static str>)> {
    with_ledger(|ledger| {
        ledger
            .summaries
            .iter()
            .filter(|s| s.disable_attempts != 0 || s.remove_attempts != 0)
            .filter(|s| matches!(s.last, ReleaseAttemptOutcome::Refused(_)))
            .map(|s| (s.target, s.name))
            .collect()
    })
}

/// Did any target need more than one attempt at anything?
///
/// The cheap question a teardown asks before deciding whether the history is
/// worth reporting at all. A clean teardown answers `false` and says nothing.
pub fn any_release_needed_a_retry() -> bool {
    with_ledger(|ledger| {
        ledger
            .summaries
            .iter()
            .any(|s| s.refusals != 0 || s.disable_attempts > 1 || s.remove_attempts > 1)
    })
}

/// Render the history as one line per attempt, in order.
///
/// The rendering keeps FAILURES AND THE SUCCESSES THAT FOLLOWED THEM, because
/// the sequence is the record: a reader has to be able to see
/// `disable#1 refused ... disable#2 ok` and conclude that the hook came off on
/// the retry, which neither line says on its own.
pub fn render_release_attempt_history() -> String {
    let dropped = release_attempts_dropped_for_capacity();
    let mut rendered = String::new();
    for attempt in release_attempt_history() {
        if !rendered.is_empty() {
            rendered.push_str("; ");
        }
        rendered.push_str(&render_attempt(&attempt));
    }
    if dropped != 0 {
        if !rendered.is_empty() {
            rendered.push_str("; ");
        }
        rendered.push_str(&format!("{dropped} further attempt(s) not stored"));
    }
    rendered
}

/// Render the attempts recorded for `targets` only, in order.
///
/// The scoped form of [`render_release_attempt_history`], for a lane teardown
/// that has to say what happened to ITS hooks without dragging in every other
/// lane's. Targets with no recorded attempt contribute nothing, so a clean
/// teardown renders the empty string and a caller can test for that.
pub fn render_release_attempts_for(targets: &[usize]) -> String {
    let mut rendered = String::new();
    for attempt in release_attempt_history() {
        if !targets.contains(&attempt.target) {
            continue;
        }
        if !rendered.is_empty() {
            rendered.push_str("; ");
        }
        rendered.push_str(&render_attempt(&attempt));
    }
    rendered
}

/// Every one of `targets` whose LAST release attempt was refused, named.
///
/// The scoped form of [`targets_with_a_refused_last_release_attempt`], and the
/// question a lane asks before it mints a release receipt: a receipt that says
/// "proven removed" while one of its own sites' last attempt failed is exactly
/// the overstatement this ledger exists to end.
pub fn unproven_releases_among(targets: &[usize]) -> Vec<(usize, Option<&'static str>)> {
    targets_with_a_refused_last_release_attempt()
        .into_iter()
        .filter(|(target, _)| targets.contains(target))
        .collect()
}

fn render_attempt(attempt: &ReleaseAttempt) -> String {
    let who = match attempt.name {
        Some(name) => name.to_string(),
        None => format!("{:#x}", attempt.target),
    };
    let outcome = match attempt.outcome {
        ReleaseAttemptOutcome::Succeeded => "ok".to_string(),
        ReleaseAttemptOutcome::AlreadyReleased => "already released".to_string(),
        ReleaseAttemptOutcome::Refused(error) => format!("REFUSED: {error}"),
    };
    format!(
        "{who} {}#{} {outcome}",
        attempt.operation.as_str(),
        attempt.attempt
    )
}

/// The ledger is process-global, so every test that RESETS it or reads its
/// aggregate state has to hold this first.
///
/// Shared with `crate::batch_tests` rather than private to this module: both
/// read the same ledger, and two private mutexes would serialise each module
/// against itself and neither against the other -- which is exactly the shape
/// that made a capacity test in one module empty the history under a
/// contents assertion in the other.
#[cfg(test)]
pub(crate) static TEST_LEDGER_SERIAL: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    let mut guard = LEDGER.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some(Ledger::default());
}

/// Every recorded attempt for one target, for a test that must not be
/// disturbed by whatever another test's hooks are doing to the same ledger.
#[cfg(test)]
pub(crate) fn release_attempts_for(target: usize) -> Vec<ReleaseAttempt> {
    with_ledger(|ledger| {
        ledger
            .history
            .iter()
            .filter(|attempt| attempt.target == target)
            .copied()
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FreezeRefusal;
    use minhook::MH_STATUS;

    use super::TEST_LEDGER_SERIAL as SERIAL;

    fn reset() {
        super::reset_for_tests();
    }

    fn retained() -> InlineDetourReleaseError {
        InlineDetourReleaseError::FreezeRefused(FreezeRefusal::ThreadSuspend {
            thread: 77,
            os_error: 5,
        })
    }

    /// THE JOB-2 PROPERTY. Falsifiable: fails if a refused disable followed by
    /// a successful retry stops being readable as attempt, failure, retry,
    /// success -- in that order, with the failure still present.
    ///
    /// This is the whole of the owner's 2026-08-05 ruling: keep the retry, but
    /// make the failed attempt name itself. A ledger that stored only the
    /// latest outcome would pass every other assertion here and fail this one,
    /// because the failure line would be gone.
    ///
    /// What production change would make this fail? Overwriting the history
    /// entry instead of appending; recording only refusals; recording only the
    /// final outcome; or dropping the attempt ordinal, which is what makes the
    /// second line legible as a RETRY of the first.
    ///
    /// MUTATION RUN: `ledger.history.clear()` before each push, so only the
    /// latest attempt survives. OBSERVED: FAILED, `the failure must survive
    /// the retry / left: 1 / right: 2`.
    ///
    /// MUTATION RUN: `release_is_unproven` redefined over `refusals != 0`,
    /// i.e. over ANY failed attempt rather than the last -- the one-way latch
    /// the owner ruled against. OBSERVED: FAILED,
    /// `assertion failed: !release_is_unproven(0x4000)`.
    #[test]
    fn a_failed_disable_and_its_successful_retry_are_both_recorded_in_order() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        name_release_target(0x4000, "d85.division_order");

        let first = record_release_attempt(0x4000, ReleaseOperation::Disable, Err(retained()));
        let second = record_release_attempt(0x4000, ReleaseOperation::Disable, Ok(()));
        assert_eq!((first, second), (1, 2), "the retry must be attempt 2");

        let history = release_attempt_history();
        assert_eq!(history.len(), 2, "the failure must survive the retry");
        assert_eq!(history[0].outcome, ReleaseAttemptOutcome::Refused(retained()));
        assert_eq!(history[0].attempt, 1);
        assert_eq!(history[1].outcome, ReleaseAttemptOutcome::Succeeded);
        assert_eq!(history[1].attempt, 2);
        assert_eq!(history[0].name, Some("d85.division_order"));

        let rendered = render_release_attempt_history();
        assert!(
            rendered.contains("d85.division_order disable#1 REFUSED"),
            "the failed attempt must name the hook, the attempt and the error: {rendered}"
        );
        assert!(
            rendered.contains("d85.division_order disable#2 ok"),
            "the healing retry must be visible too: {rendered}"
        );
        assert!(rendered.contains("77"), "the error must survive: {rendered}");

        // And the overstatement is over: the last attempt succeeded.
        assert!(!release_is_unproven(0x4000));
        assert!(targets_with_a_refused_last_release_attempt().is_empty());
        assert!(
            any_release_needed_a_retry(),
            "a teardown that needed a retry must still say so after it healed"
        );
    }

    /// Falsifiable: fails if a target whose LAST attempt was refused is ever
    /// reported as proven released.
    ///
    /// What production change would make this fail? Defining
    /// `release_is_unproven` over "any attempt failed" (which never clears and
    /// is the one-way latch the owner ruled out) or over "the first attempt"
    /// (which a retry can never correct).
    #[test]
    fn a_target_whose_last_attempt_was_refused_is_unproven_and_named() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        name_release_target(0x5000, "d45.focus_completed");
        record_release_attempt(0x5000, ReleaseOperation::Disable, Ok(()));
        record_release_attempt(0x5000, ReleaseOperation::Remove, Err(retained()));

        assert!(
            release_is_unproven(0x5000),
            "a refused REMOVE leaves the trampoline live and is not a proven release"
        );
        assert_eq!(
            targets_with_a_refused_last_release_attempt(),
            vec![(0x5000, Some("d45.focus_completed"))]
        );
    }

    /// Falsifiable: fails if a target nobody ever tried to release reads as
    /// released, or as unproven.
    ///
    /// Both directions are wrong and both are reachable: `Option` collapsing
    /// to `Some(Succeeded)` would let a receipt claim a removal that was never
    /// attempted, and collapsing to unproven would refuse every capture whose
    /// hooks are simply not this crate's business.
    ///
    /// What production change would make this fail? Dropping the
    /// `disable_attempts != 0 || remove_attempts != 0` filter, which is the
    /// only thing separating "named" from "attempted".
    #[test]
    fn a_target_that_was_never_attempted_answers_nothing_at_all() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        assert_eq!(last_release_attempt(0x6000), None);
        assert!(!release_is_unproven(0x6000));

        // Naming is not attempting.
        name_release_target(0x6000, "named.but.untried");
        assert_eq!(last_release_attempt(0x6000), None);
        assert!(!release_is_unproven(0x6000));
        assert!(targets_with_a_refused_last_release_attempt().is_empty());
        assert!(!any_release_needed_a_retry());
    }

    /// Falsifiable: fails if MinHook's nothing-to-do statuses are ever recorded
    /// as refusals, or a refused freeze is ever recorded as nothing-to-do.
    ///
    /// The second half is the one that matters: a refused freeze means the
    /// detour is STILL PATCHED, and every lane teardown in this workspace
    /// already treats `MH_ERROR_DISABLED`/`MH_ERROR_NOT_CREATED` as clean. A
    /// ledger that classified differently from the callers would contradict
    /// them.
    ///
    /// What production change would make this fail? Writing
    /// `ReleaseAttemptOutcome::of` as `Err(_) => Refused` (which would report
    /// every idempotent second teardown as a failure) or as
    /// `Err(_) => AlreadyReleased` (which would report a live detour as gone).
    #[test]
    fn the_outcome_classifier_matches_what_the_lane_teardowns_already_do() {
        assert_eq!(ReleaseAttemptOutcome::of(Ok(())), ReleaseAttemptOutcome::Succeeded);
        for clean in [
            InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_DISABLED),
            InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_NOT_CREATED),
        ] {
            assert_eq!(
                ReleaseAttemptOutcome::of(Err(clean)),
                ReleaseAttemptOutcome::AlreadyReleased
            );
            assert!(ReleaseAttemptOutcome::of(Err(clean)).released());
        }
        let live = retained();
        assert_eq!(
            ReleaseAttemptOutcome::of(Err(live)),
            ReleaseAttemptOutcome::Refused(live)
        );
        assert!(!ReleaseAttemptOutcome::of(Err(live)).released());
        let refused = InlineDetourReleaseError::MinHook(MH_STATUS::MH_ERROR_MEMORY_PROTECT);
        assert_eq!(
            ReleaseAttemptOutcome::of(Err(refused)),
            ReleaseAttemptOutcome::Refused(refused)
        );
    }

    /// Falsifiable: fails if the history can grow without bound, or if
    /// overflow becomes silent.
    ///
    /// A teardown driver retries on a backoff and can run for a long time; an
    /// unbounded ledger in an injected DLL is a leak. Dropping entries quietly
    /// would be worse than the leak, so the count is reported and the
    /// PER-TARGET verdict is still exact.
    ///
    /// What production change would make this fail? Removing the capacity
    /// check; removing the `dropped` counter; or dropping the per-target
    /// summary update when the history is full, which would make
    /// `release_is_unproven` stop seeing later attempts.
    #[test]
    fn the_history_is_bounded_and_says_how_much_it_dropped() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        for _ in 0..(HISTORY_CAPACITY + 7) {
            record_release_attempt(0x7000, ReleaseOperation::Disable, Ok(()));
        }
        assert_eq!(release_attempt_history().len(), HISTORY_CAPACITY);
        assert_eq!(release_attempts_dropped_for_capacity(), 7);
        assert!(render_release_attempt_history().contains("7 further attempt(s) not stored"));

        // The verdict is still exact for the attempts nobody stored.
        record_release_attempt(0x7000, ReleaseOperation::Disable, Err(retained()));
        assert!(
            release_is_unproven(0x7000),
            "a refusal past the history cap must still change the per-target verdict"
        );
    }
}
