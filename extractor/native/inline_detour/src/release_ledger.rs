//! One mutex acquisition per release attempt, on a teardown path that has just
//! deadlocks a game.

use crate::windows_impl::InlineDetourReleaseError;
use std::sync::Mutex;

pub const HISTORY_CAPACITY: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseOperation {
    Disable,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseAttemptOutcome {
    Succeeded,
    AlreadyReleased,
    Refused(InlineDetourReleaseError),
}

impl ReleaseAttemptOutcome {
    /// Classify a release result exactly as every lane's own teardown does:
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReleaseAttempt {
    pub target: usize,
    pub name: Option<&'static str>,
    pub operation: ReleaseOperation,
    pub attempt: u32,
    pub outcome: ReleaseAttemptOutcome,
}

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
    let mut guard = LEDGER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    op(guard.get_or_insert_with(Ledger::default))
}

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
            // successful release, so mark it as having no attempts yet by
            // `last_release_attempt`, which requires a non-zero attempt count.
        }
    });
}

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

pub fn release_attempt_history() -> Vec<ReleaseAttempt> {
    with_ledger(|ledger| ledger.history.clone())
}

pub fn release_attempts_dropped_for_capacity() -> u64 {
    with_ledger(|ledger| ledger.dropped)
}

/// release it has ever been recorded.
/// `None` is deliberately NOT "released". A caller asking about a target that
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

/// THE predicate a release receipt consults before it claims a hook is proven
pub fn release_is_unproven(target: usize) -> bool {
    matches!(
        last_release_attempt(target),
        Some(ReleaseAttemptOutcome::Refused(_))
    )
}

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

pub fn any_release_needed_a_retry() -> bool {
    with_ledger(|ledger| {
        ledger
            .summaries
            .iter()
            .any(|s| s.refusals != 0 || s.disable_attempts > 1 || s.remove_attempts > 1)
    })
}

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

/// The scoped form of [`render_release_attempt_history`], for a lane teardown
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
/// The scoped form of [`targets_with_a_refused_last_release_attempt`], and the
/// question a lane asks before it mints a release receipt: a receipt that says
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

#[cfg(test)]
pub(crate) static TEST_LEDGER_SERIAL: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    let mut guard = LEDGER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some(Ledger::default());
}

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

    /// MUTATION RUN: `release_is_unproven` redefined over `refusals != 0`,
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
        assert_eq!(
            history[0].outcome,
            ReleaseAttemptOutcome::Refused(retained())
        );
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
        assert!(
            rendered.contains("77"),
            "the error must survive: {rendered}"
        );

        assert!(!release_is_unproven(0x4000));
        assert!(targets_with_a_refused_last_release_attempt().is_empty());
        assert!(
            any_release_needed_a_retry(),
            "a teardown that needed a retry must still say so after it healed"
        );
    }

    /// reported as proven released.
    /// `release_is_unproven` over "any attempt failed" (which never clears and
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
    #[test]
    fn a_target_that_was_never_attempted_answers_nothing_at_all() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        assert_eq!(last_release_attempt(0x6000), None);
        assert!(!release_is_unproven(0x6000));

        name_release_target(0x6000, "named.but.untried");
        assert_eq!(last_release_attempt(0x6000), None);
        assert!(!release_is_unproven(0x6000));
        assert!(targets_with_a_refused_last_release_attempt().is_empty());
        assert!(!any_release_needed_a_retry());
    }

    /// `ReleaseAttemptOutcome::of` as `Err(_) => Refused` (which would report
    /// `Err(_) => AlreadyReleased` (which would report a live detour as gone).
    #[test]
    fn the_outcome_classifier_matches_what_the_lane_teardowns_already_do() {
        assert_eq!(
            ReleaseAttemptOutcome::of(Ok(())),
            ReleaseAttemptOutcome::Succeeded
        );
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

        record_release_attempt(0x7000, ReleaseOperation::Disable, Err(retained()));
        assert!(
            release_is_unproven(0x7000),
            "a refusal past the history cap must still change the per-target verdict"
        );
    }
}
