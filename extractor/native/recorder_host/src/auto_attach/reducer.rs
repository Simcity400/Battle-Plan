//! The idempotent candidate registry and reducer shared by all discovery
//! sources.
//!
//! WMI start events, the startup snapshot, and every reconciliation snapshot
//! all feed the SAME reducer keyed by `(pid, creation_filetime)`. A game that
//! starts before we subscribe appears only in the snapshot; one that starts
//! after appears in WMI and may appear in both — full-identity dedup collapses
//! them to one candidate. A reused PID with a new FILETIME is a new identity
//! with no inherited state.
//!
//! This module is the heart of the SINGLE-INJECTION LAW: the registry is the
//! one authority on whether an identity has (or may have) been injected, and
//! [`CandidateRegistry::may_start_injection`] is the guard every attach path
//! must pass through.

use std::collections::{BTreeMap, BTreeSet};

use super::identity::{ProcessIdentity, ProcessKey};
use super::sources::StagedPayload;
use super::state::CandidateState;

/// One tracked process identity and its live bookkeeping.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub key: ProcessKey,
    /// Resolved identity once probed. Always present after `observe_present`.
    pub identity: ProcessIdentity,
    pub state: CandidateState,
    pub first_observed_ms: u64,
    pub last_seen_ms: u64,
    /// Wall-ms the candidate entered its current state (drives the handshake
    /// deadline and readiness backoff).
    pub state_entered_ms: u64,
    /// How many readiness re-checks have run (drives the backoff ladder).
    pub readiness_attempts: usize,
    /// Wall-ms of the last expensive identity/window readiness probe.
    pub last_readiness_probe_ms: Option<u64>,
    /// The current attach attempt id, once one is staged.
    pub attempt_id: Option<String>,
    /// Coordinator-owned session identifier actually passed to the worker.
    pub session_id: Option<String>,
    /// The staged collector payload (unique DLL dir, control path, hashes) once
    /// `Preparing` succeeds.
    pub payload: Option<StagedPayload>,
    /// Last game hour a valid frame confirmed, for the status card.
    pub last_confirmed_hour: Option<i64>,
    /// Set when a reconciliation proves the process gone AFTER injection was
    /// committed — the coordinator must finalize/preserve the prefix, not treat
    /// it as a clean pre-attach exit.
    pub process_gone_after_commit: bool,
    /// A stop hint was received; the coordinator should reconcile promptly.
    pub stop_hinted: bool,
}

impl Candidate {
    fn new(identity: ProcessIdentity, now_ms: u64) -> Self {
        Self {
            key: identity.key,
            identity,
            state: CandidateState::Detected,
            first_observed_ms: now_ms,
            last_seen_ms: now_ms,
            state_entered_ms: now_ms,
            readiness_attempts: 0,
            last_readiness_probe_ms: None,
            attempt_id: None,
            session_id: None,
            payload: None,
            last_confirmed_hour: None,
            process_gone_after_commit: false,
            stop_hinted: false,
        }
    }
}

/// Registry of all known identities this coordinator instance has seen.
#[derive(Default)]
pub struct CandidateRegistry {
    by_key: BTreeMap<ProcessKey, Candidate>,
}

/// What `observe_present` did, so the coordinator knows whether to journal a
/// fresh `Detected`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObserveOutcome {
    /// A brand-new identity was inserted.
    NewCandidate,
    /// An already-known identity was refreshed (idempotent no-op for dedup).
    AlreadyKnown,
}

impl CandidateRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &ProcessKey) -> Option<&Candidate> {
        self.by_key.get(key)
    }

    pub fn get_mut(&mut self, key: &ProcessKey) -> Option<&mut Candidate> {
        self.by_key.get_mut(key)
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ProcessKey, &Candidate)> {
        self.by_key.iter()
    }

    /// Keys currently in a non-terminal state.
    pub fn active_keys(&self) -> Vec<ProcessKey> {
        self.by_key
            .iter()
            .filter(|(_, c)| !c.state.is_terminal())
            .map(|(k, _)| *k)
            .collect()
    }

    /// Fold an observation that a given live identity is present. Idempotent:
    /// the same identity observed via snapshot AND via WMI collapses to one
    /// candidate. Only `hoi4.exe` identities are admitted;
    /// anything else is ignored (defensive — callers already filter).
    pub fn observe_present(&mut self, identity: ProcessIdentity, now_ms: u64) -> ObserveOutcome {
        if !identity.is_hoi4() {
            return ObserveOutcome::AlreadyKnown;
        }
        let key = identity.key;
        match self.by_key.get_mut(&key) {
            Some(existing) => {
                existing.last_seen_ms = now_ms;
                existing.stop_hinted = false; // a fresh sighting clears a stale stop hint
                ObserveOutcome::AlreadyKnown
            }
            None => {
                self.by_key.insert(key, Candidate::new(identity, now_ms));
                ObserveOutcome::NewCandidate
            }
        }
    }

    /// Replace a recovered terminal generation with a clean candidate after the
    /// coordinator has independently proved that generation's exact staged DLL
    /// is absent from the still-live target. The registry does not perform that
    /// OS proof itself; requiring a terminal predecessor prevents this seam
    /// from weakening the in-generation single-injection guard.
    pub fn rearm_after_proven_unload(
        &mut self,
        identity: ProcessIdentity,
        now_ms: u64,
    ) -> Result<CandidateState, String> {
        let key = identity.key;
        let previous = self
            .by_key
            .get(&key)
            .ok_or_else(|| format!("cannot rearm unknown identity {key:?}"))?
            .state;
        if !previous.is_terminal() {
            return Err(format!(
                "cannot rearm nonterminal identity {key:?} in state {previous}"
            ));
        }
        self.by_key.insert(key, Candidate::new(identity, now_ms));
        Ok(previous)
    }

    /// Record a WMI stop hint for a pid. It is only a hint: liveness is proven
    /// by reconciliation. Marks every live candidate carrying that
    /// pid so the coordinator reconciles promptly.
    pub fn stop_hint(&mut self, pid: u32) {
        for cand in self.by_key.values_mut() {
            if cand.key.pid == pid && !cand.state.is_terminal() {
                cand.stop_hinted = true;
            }
        }
    }

    /// Reconcile against the authoritative live set (the keys a fresh snapshot
    /// proved alive). Any non-terminal candidate absent from `live_keys` has
    /// exited. Returns the keys that just transitioned to a terminal/gone state
    /// so the coordinator can journal them.
    ///
    /// Pre-injection exit => `ProcessExitedBeforeAttach` (clean terminal).
    /// Post-commit exit => flagged `process_gone_after_commit` and left in place
    /// for the coordinator to finalize/preserve the prefix.
    pub fn reconcile_live_set(
        &mut self,
        live_keys: &BTreeSet<ProcessKey>,
        now_ms: u64,
    ) -> Vec<ProcessKey> {
        self.reconcile_snapshot(live_keys, &BTreeSet::new(), true, now_ms)
    }

    /// Reconcile only when enumeration completeness is positively witnessed.
    /// `unresolved_pids` were present in the snapshot but could not be identity-
    /// probed; they are therefore not evidence that an existing key exited.
    pub fn reconcile_snapshot(
        &mut self,
        live_keys: &BTreeSet<ProcessKey>,
        unresolved_pids: &BTreeSet<u32>,
        snapshot_complete: bool,
        now_ms: u64,
    ) -> Vec<ProcessKey> {
        if !snapshot_complete {
            return Vec::new();
        }
        let mut changed = Vec::new();
        for (key, cand) in self.by_key.iter_mut() {
            if cand.state.is_terminal() {
                continue;
            }
            if live_keys.contains(key) || unresolved_pids.contains(&key.pid) {
                cand.last_seen_ms = now_ms;
                if live_keys.contains(key) {
                    cand.stop_hinted = false;
                }
                continue;
            }
            // Absent from the live set => gone.
            if let Some(next) = cand.state.on_process_exit() {
                // Clean pre-attach exit.
                cand.state = next;
                changed.push(*key);
            } else if cand.state.injection_committed() && !cand.process_gone_after_commit {
                cand.process_gone_after_commit = true;
                changed.push(*key);
            }
        }
        changed
    }

    /// The SINGLE-INJECTION LAW guard. Returns true ONLY when it is provably
    /// safe to begin an injection for `key`: the candidate exists, is in a
    /// pre-injection state, and has never committed an injection. Fails closed
    /// on an unknown key or any committed/terminal state because a fresh DLL
    /// path does not make automatic retry safe.
    pub fn may_start_injection(&self, key: &ProcessKey) -> bool {
        match self.by_key.get(key) {
            Some(c) => !c.state.injection_committed() && !c.state.is_terminal(),
            None => false,
        }
    }

    /// Advance a candidate to `next` if the transition is legal AND — for a
    /// transition INTO the injection-committed region — the no-double-inject
    /// guard permits it. Returns the prior state on success, or an error string
    /// (fail-closed) describing why the transition was refused.
    pub fn advance(
        &mut self,
        key: &ProcessKey,
        next: CandidateState,
        now_ms: u64,
    ) -> Result<CandidateState, String> {
        // Guard the commit boundary BEFORE mutating.
        let entering_injection = next == CandidateState::Injecting;
        if entering_injection && !self.may_start_injection(key) {
            return Err(format!(
                "refused injection for {key:?}: no-double-inject guard (unknown/committed/terminal)"
            ));
        }
        let cand = self
            .by_key
            .get_mut(key)
            .ok_or_else(|| format!("advance unknown identity {key:?}"))?;
        if !cand.state.can_advance_to(next) {
            return Err(format!(
                "illegal transition {} -> {} for {key:?}",
                cand.state, next
            ));
        }
        let prev = cand.state;
        cand.state = next;
        cand.state_entered_ms = now_ms;
        if next != prev {
            cand.readiness_attempts = 0;
            cand.last_readiness_probe_ms = None;
        }
        Ok(prev)
    }

    /// Seed a recovered identity directly into a given state during crash
    /// recovery. Used to poison a committed-but-unproven identity as
    /// `Failed` so `may_start_injection` stays closed and the live process is
    /// never re-injected (SINGLE-INJECTION LAW under supervisor restart).
    pub fn seed_recovered(
        &mut self,
        key: ProcessKey,
        state: CandidateState,
        image_path: String,
        now_ms: u64,
    ) {
        let identity = ProcessIdentity {
            key,
            image_path,
            session_id: 0,
            user_sid_hash: String::new(),
        };
        let mut cand = Candidate::new(identity, now_ms);
        cand.state = state;
        self.by_key.insert(key, cand);
    }

    /// Force a candidate into `Failed` (a hard failure edge, legal from any
    /// non-terminal state). Idempotent on already-failed. Returns prior state.
    pub fn fail(&mut self, key: &ProcessKey) -> Option<CandidateState> {
        let cand = self.by_key.get_mut(key)?;
        if cand.state.is_terminal() {
            return None;
        }
        let prev = cand.state;
        cand.state = CandidateState::Failed;
        Some(prev)
    }

    /// Record a readiness re-check and return the just-incremented attempt count.
    pub fn bump_readiness(&mut self, key: &ProcessKey, now_ms: u64) -> usize {
        if let Some(c) = self.by_key.get_mut(key) {
            c.readiness_attempts += 1;
            c.last_readiness_probe_ms = Some(now_ms);
            c.readiness_attempts
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(pid: u32, ft: u64) -> ProcessIdentity {
        ProcessIdentity {
            key: ProcessKey::new(pid, ft),
            image_path: r"C:\Games\the game\hoi4.exe".to_string(),
            session_id: 1,
            user_sid_hash: "sidhash".to_string(),
        }
    }

    #[test]
    fn dedup_snapshot_and_wmi_collapse_to_one() {
        // The same identity seen via snapshot then via WMI is
        // ONE candidate, one attempt.
        let mut reg = CandidateRegistry::new();
        assert_eq!(
            reg.observe_present(ident(100, 5), 0),
            ObserveOutcome::NewCandidate
        );
        assert_eq!(
            reg.observe_present(ident(100, 5), 10),
            ObserveOutcome::AlreadyKnown
        );
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn reused_pid_is_a_new_identity_with_no_old_state() {
        // A reused PID with a different creation time is a distinct identity.
        let mut reg = CandidateRegistry::new();
        reg.observe_present(ident(100, 5), 0);
        // Drive first identity to Complete.
        reg.advance(
            &ProcessKey::new(100, 5),
            CandidateState::WaitingForWindow,
            0,
        )
        .unwrap();
        // Same pid, new FILETIME.
        assert_eq!(
            reg.observe_present(ident(100, 9), 100),
            ObserveOutcome::NewCandidate
        );
        assert_eq!(reg.len(), 2);
        let fresh = reg.get(&ProcessKey::new(100, 9)).unwrap();
        assert_eq!(
            fresh.state,
            CandidateState::Detected,
            "new identity starts clean"
        );
    }

    #[test]
    fn no_double_inject_guard_blocks_second_injection() {
        // SINGLE-INJECTION LAW: once Injecting, never again.
        let mut reg = CandidateRegistry::new();
        let key = ProcessKey::new(200, 7);
        reg.observe_present(ident(200, 7), 0);
        reg.advance(&key, CandidateState::WaitingForWindow, 0)
            .unwrap();
        reg.advance(&key, CandidateState::Preparing, 0).unwrap();
        assert!(reg.may_start_injection(&key));
        reg.advance(&key, CandidateState::Injecting, 0).unwrap();
        assert!(
            !reg.may_start_injection(&key),
            "injection committed => guard closes"
        );
        // A second advance-to-Injecting is refused fail-closed.
        let err = reg.advance(&key, CandidateState::Injecting, 0).unwrap_err();
        assert!(err.contains("no-double-inject") || err.contains("illegal transition"));
    }

    #[test]
    fn may_start_injection_fails_closed_on_unknown_key() {
        let reg = CandidateRegistry::new();
        assert!(!reg.may_start_injection(&ProcessKey::new(1, 1)));
    }

    #[test]
    fn proven_unload_rearms_only_a_terminal_generation() {
        let mut reg = CandidateRegistry::new();
        let identity = ident(201, 8);
        let key = identity.key;
        reg.observe_present(identity.clone(), 0);
        assert!(reg
            .rearm_after_proven_unload(identity.clone(), 1)
            .unwrap_err()
            .contains("nonterminal"));
        reg.fail(&key);
        assert_eq!(
            reg.rearm_after_proven_unload(identity, 2).unwrap(),
            CandidateState::Failed
        );
        let fresh = reg.get(&key).unwrap();
        assert_eq!(fresh.state, CandidateState::Detected);
        assert!(fresh.attempt_id.is_none());
        assert!(fresh.payload.is_none());
        assert!(reg.may_start_injection(&key));
    }

    #[test]
    fn reconcile_marks_pre_attach_exit_clean() {
        let mut reg = CandidateRegistry::new();
        let key = ProcessKey::new(300, 3);
        reg.observe_present(ident(300, 3), 0);
        reg.advance(&key, CandidateState::WaitingForWindow, 0)
            .unwrap();
        // Empty live set => it exited.
        let changed = reg.reconcile_live_set(&BTreeSet::new(), 50);
        assert_eq!(changed, vec![key]);
        assert_eq!(
            reg.get(&key).unwrap().state,
            CandidateState::ProcessExitedBeforeAttach
        );
    }

    #[test]
    fn reconcile_flags_post_commit_exit_for_finalize() {
        let mut reg = CandidateRegistry::new();
        let key = ProcessKey::new(400, 4);
        reg.observe_present(ident(400, 4), 0);
        reg.advance(&key, CandidateState::WaitingForWindow, 0)
            .unwrap();
        reg.advance(&key, CandidateState::Preparing, 0).unwrap();
        reg.advance(&key, CandidateState::Injecting, 0).unwrap();
        reg.advance(&key, CandidateState::Handshaking, 0).unwrap();
        let changed = reg.reconcile_live_set(&BTreeSet::new(), 60);
        assert_eq!(changed, vec![key]);
        let c = reg.get(&key).unwrap();
        assert!(
            c.process_gone_after_commit,
            "committed exit needs finalize, not clean-terminal"
        );
        assert_eq!(
            c.state,
            CandidateState::Handshaking,
            "state preserved for the finalize decision"
        );
    }

    #[test]
    fn reconcile_keeps_live_candidates() {
        let mut reg = CandidateRegistry::new();
        let key = ProcessKey::new(500, 5);
        reg.observe_present(ident(500, 5), 0);
        let mut live = BTreeSet::new();
        live.insert(key);
        let changed = reg.reconcile_live_set(&live, 70);
        assert!(changed.is_empty());
        assert_eq!(reg.get(&key).unwrap().last_seen_ms, 70);
    }

    #[test]
    fn two_simultaneous_identities_are_independent() {
        // Two games are tracked independently and never merged.
        let mut reg = CandidateRegistry::new();
        reg.observe_present(ident(600, 1), 0);
        reg.observe_present(ident(601, 2), 0);
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.active_keys().len(), 2);
        // One exits; the other is untouched.
        let mut live = BTreeSet::new();
        live.insert(ProcessKey::new(601, 2));
        reg.reconcile_live_set(&live, 10);
        assert_eq!(
            reg.get(&ProcessKey::new(600, 1)).unwrap().state,
            CandidateState::ProcessExitedBeforeAttach
        );
        assert!(!reg
            .get(&ProcessKey::new(601, 2))
            .unwrap()
            .state
            .is_terminal());
    }

    #[test]
    fn non_hoi4_identity_is_ignored() {
        let mut reg = CandidateRegistry::new();
        let mut bogus = ident(700, 7);
        bogus.image_path = r"C:\Windows\notepad.exe".to_string();
        assert_eq!(reg.observe_present(bogus, 0), ObserveOutcome::AlreadyKnown);
        assert_eq!(reg.len(), 0);
    }
}
