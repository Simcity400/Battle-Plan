//! The resident auto-attach coordinator (design §1, §4, §5).
//!
//! One coordinator per Windows session drives every detected the game identity
//! through the attach state machine. It is written as a deterministic
//! `step()`-driven loop over injected trait objects (clock, probe, event
//! source, environment, stager, spawner) so the whole control flow — dual-order
//! discovery, the subscribe/snapshot race, PID reuse, WMI loss, identity
//! fail-close, the no-double-inject guard, multi-instance, and crash recovery —
//! is exercised by fakes with zero live game (design §9 abstraction rationale;
//! design §10 deterministic gates).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::identity::{name_is_hoi4, ProcessIdentity, ProcessKey};
#[cfg(test)]
use super::journal::read_all;
use super::journal::{recover, Journal, JournalRecord};
use super::policy;
use super::reducer::{CandidateRegistry, ObserveOutcome};
use super::sources::{
    AttachEnvironment, Clock, PayloadStager, ProcessEventSource, ProcessProbe, RawProcessEvent,
    WorkerHandle, WorkerSpawner, WorkerSpec, WorkerStatus,
};
use super::staging::{generate_attempt_id, generate_pipe_token, generate_secret_128};
use super::state::{CandidateState, ReasonCode};

/// Static configuration for a coordinator instance. No correctness policy is
/// user-tunable (design §7); these are identity/location facts resolved at boot.
pub struct CoordinatorConfig {
    pub coordinator_instance_id: String,
    /// Output root for per-attempt run directories.
    pub out_root: PathBuf,
    /// The coordinator's own Windows session id and token-SID hash, used to
    /// prove a candidate is same-user/same-session (design §3). A candidate that
    /// does not match is never admitted.
    pub own_session_id: u32,
    pub own_sid_hash: String,
}

/// The dependency bundle. Boxed trait objects so production wires real adapters
/// and tests wire fakes.
pub struct CoordinatorDeps {
    pub clock: Box<dyn Clock>,
    pub probe: Box<dyn ProcessProbe>,
    pub events: Box<dyn ProcessEventSource>,
    pub env: Box<dyn AttachEnvironment>,
    pub stager: Box<dyn PayloadStager>,
    pub spawner: Box<dyn WorkerSpawner>,
}

/// User-visible lifecycle facts emitted only after their authoritative journal
/// transition succeeds. Capture correctness never depends on a UI consumer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SupervisorEvent {
    RecordingStarted { pid: u32, run_dir: PathBuf },
    RecordingSucceeded { pid: u32, replay_path: PathBuf },
    RecordingFailed {
        pid: u32,
        reason: ReasonCode,
        detail: Option<String>,
        run_dir: Option<PathBuf>,
    },
}

pub struct Coordinator {
    config: CoordinatorConfig,
    deps: CoordinatorDeps,
    registry: CandidateRegistry,
    journal: Journal,
    workers: BTreeMap<ProcessKey, Box<dyn WorkerHandle>>,
    last_reconcile_ms: Option<u64>,
    force_reconcile: bool,
    started: bool,
    supervisor_events: Vec<SupervisorEvent>,
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        // `run_supervisor` propagates coordinator errors with `?`. Ownership is
        // therefore the final safety net on every error and shutdown path: no
        // worker may outlive the coordinator that authorizes it.
        for worker in self.workers.values_mut() {
            worker.request_stop();
        }
    }
}

impl Coordinator {
    pub fn new(
        config: CoordinatorConfig,
        deps: CoordinatorDeps,
        journal_path: impl Into<PathBuf>,
    ) -> Result<Self, String> {
        let journal = Journal::open(journal_path.into())?;
        Ok(Self {
            config,
            deps,
            registry: CandidateRegistry::new(),
            journal,
            workers: BTreeMap::new(),
            last_reconcile_ms: None,
            force_reconcile: false,
            started: false,
            supervisor_events: Vec::new(),
        })
    }

    pub fn registry(&self) -> &CandidateRegistry {
        &self.registry
    }

    /// Drain product-facing notifications after their transitions are durable.
    pub fn drain_supervisor_events(&mut self) -> Vec<SupervisorEvent> {
        std::mem::take(&mut self.supervisor_events)
    }

    /// Transfer every active candidate into explicit-stop ownership. Recording
    /// workers retain their valid prefix and enter Finalizing; candidates that
    /// never proved Recording fail closed. The caller must continue
    /// [`Self::drive_shutdown`] until it returns true so completion/failure is
    /// durably journalled rather than discarded by Drop.
    pub fn begin_explicit_stop(&mut self) -> Result<(), String> {
        let now = self.deps.clock.now_ms();
        let keys = self.registry.active_keys();
        // Signal every child before any terminal transition removes a handle.
        // `ChildWorkerHandle::drop` may wait through the graceful deadline, so
        // this two-pass form makes all worker drain windows overlap instead of
        // serializing one 30-second window per active game.
        for key in &keys {
            if let Some(worker) = self.workers.get_mut(key) {
                worker.request_stop();
            }
        }
        for key in keys {
            let state = match self.registry.get(&key) {
                Some(candidate) => candidate.state,
                None => continue,
            };
            match state {
                CandidateState::Recording => {
                    self.transition(
                        key,
                        CandidateState::Finalizing,
                        ReasonCode::ExplicitStop,
                        now,
                        Some("explicit app stop; draining and verifying replay".into()),
                    )?;
                }
                CandidateState::Finalizing => {}
                _ => {
                    self.fail(
                        key,
                        ReasonCode::ExplicitStop,
                        now,
                        Some("explicit app stop before Recording admission".into()),
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Poll only already-owned finalizers; discovery/injection is permanently
    /// out of scope once explicit shutdown begins.
    pub fn drive_shutdown(&mut self) -> Result<bool, String> {
        let now = self.deps.clock.now_ms();
        let keys: Vec<_> = self.workers.keys().copied().collect();
        for key in keys {
            self.drive_worker(key, now)?;
        }
        Ok(self.workers.is_empty())
    }

    /// Recover from a prior journal (design §8, gate 10). Every nonterminal
    /// identity that had COMMITTED an injection but whose completion is not
    /// proven is poisoned as `Failed` so it can never be re-injected — the
    /// SINGLE-INJECTION LAW under supervisor restart. If its live process is
    /// gone we still record it terminal; if it is alive we still refuse to
    /// touch it (we cannot prove the old module unloaded).
    pub fn recover_from_journal(&mut self) -> Result<usize, String> {
        // Use the chain+anchor-validated bytes retained behind the still-open,
        // deny-write/delete journal handles. Re-reading the pathname here would
        // release authority between validation and the recovery action.
        let records = self.journal.recovered_records().to_vec();
        let recovered = recover(&records);
        let now = self.deps.clock.now_ms();
        let mut poisoned = 0;
        for r in recovered {
            if r.injection_committed {
                // Fail-closed: never reinject a live process whose prior module
                // we cannot prove unloaded.
                let image = records
                    .iter()
                    .rev()
                    .find(|rec| rec.key() == r.key && rec.exe_path.is_some())
                    .and_then(|rec| rec.exe_path.clone())
                    .unwrap_or_else(|| "hoi4.exe".to_string());
                let recovered_state = if r.last_state.is_terminal() {
                    r.last_state
                } else {
                    CandidateState::Failed
                };
                self.registry
                    .seed_recovered(r.key, recovered_state, image, now);
                if !r.last_state.is_terminal() {
                    self.journal_transition(
                        r.key,
                        Some(r.last_state),
                        CandidateState::Failed,
                        ReasonCode::RecoveredNonterminalReinjectionBlocked,
                        Some("recovered mid-injection identity; refusing reinjection".to_string()),
                    )?;
                }
                poisoned += 1;
                continue;
            }
            if r.last_state.is_terminal() {
                continue;
            }
            // Nonterminal, NOT committed (e.g. WaitingForWindow at crash): safe
            // to re-detect fresh via the normal snapshot path — no seed needed.
        }
        Ok(poisoned)
    }

    /// One coordinator iteration. Deterministic: does exactly the work implied
    /// by the current clock and injected signals, then returns.
    pub fn step(&mut self) -> Result<(), String> {
        let now = self.deps.clock.now_ms();
        // First step: subscribe (already done by the event source ctor) THEN
        // enumerate a complete snapshot and drain queued events — the exact
        // startup order (design §2). We enumerate first so game-first is closed
        // even if no push events ever arrive.
        if !self.started {
            self.ingest_snapshot(now)?;
            self.started = true;
        }
        self.ingest_events(now)?;
        if self.force_reconcile || self.reconcile_due(now) {
            self.ingest_snapshot(now)?;
            self.force_reconcile = false;
        }
        self.drive_all(now)?;
        Ok(())
    }

    fn reconcile_due(&self, now: u64) -> bool {
        match self.last_reconcile_ms {
            None => true,
            Some(last) => {
                let interval = policy::reconcile_interval_ms(self.deps.events.health());
                now.saturating_sub(last) >= interval
            }
        }
    }

    fn same_user(&self, id: &ProcessIdentity) -> bool {
        id.session_id == self.config.own_session_id && id.user_sid_hash == self.config.own_sid_hash
    }

    /// Enumerate live processes, admit new same-user `hoi4.exe` identities, and
    /// reconcile exits against the authoritative live set (design §2).
    fn ingest_snapshot(&mut self, now: u64) -> Result<(), String> {
        let snapshot = self.deps.probe.snapshot()?;
        let mut live: BTreeSet<ProcessKey> = BTreeSet::new();
        let mut unresolved_pids: BTreeSet<u32> = BTreeSet::new();
        let mut new_keys: Vec<ProcessKey> = Vec::new();
        for e in snapshot.entries {
            if !name_is_hoi4(&e.name) {
                continue;
            }
            // Authority-grade identity from the live handle. A the game pid we cannot
            // identify (access denied / different session) is NOT admitted —
            // fail closed (design §3, gate 5).
            let id = match self.deps.probe.identify(e.pid) {
                Ok(id) => id,
                Err(_) => {
                    unresolved_pids.insert(e.pid);
                    continue;
                }
            };
            if !id.is_hoi4() || !self.same_user(&id) {
                continue;
            }
            let key = id.key;
            live.insert(key);
            if self.registry.observe_present(id, now) == ObserveOutcome::NewCandidate {
                new_keys.push(key);
            }
        }
        // Journal freshly detected identities.
        for key in new_keys {
            self.journal_transition(
                key,
                None,
                CandidateState::Detected,
                ReasonCode::Detected,
                None,
            )?;
        }
        // Reconcile exits. A candidate absent from the authoritative live set has
        // gone: pre-attach => clean terminal, post-commit => finalize flag.
        let changed =
            self.registry
                .reconcile_snapshot(&live, &unresolved_pids, snapshot.complete, now);
        for key in changed {
            let cand = self.registry.get(&key).expect("changed key exists");
            let to = cand.state;
            self.journal_transition(
                key,
                None,
                to,
                ReasonCode::ProcessExited,
                Some("absent from live set".into()),
            )?;
        }
        self.last_reconcile_ms = Some(now);
        Ok(())
    }

    /// Handle low-latency push events (design §2). Start events add via probe;
    /// stop events are hints that force a prompt reconcile.
    fn ingest_events(&mut self, now: u64) -> Result<(), String> {
        let events = self.deps.events.drain_events();
        for ev in events {
            match ev {
                RawProcessEvent::Start { pid, name } => {
                    if !name_is_hoi4(&name) {
                        continue;
                    }
                    let id = match self.deps.probe.identify(pid) {
                        Ok(id) => id,
                        Err(_) => continue,
                    };
                    if !id.is_hoi4() || !self.same_user(&id) {
                        continue;
                    }
                    let key = id.key;
                    if self.registry.observe_present(id, now) == ObserveOutcome::NewCandidate {
                        self.journal_transition(
                            key,
                            None,
                            CandidateState::Detected,
                            ReasonCode::Detected,
                            None,
                        )?;
                    }
                }
                RawProcessEvent::Stop { pid, .. } => {
                    self.registry.stop_hint(pid);
                    self.force_reconcile = true;
                }
            }
        }
        Ok(())
    }

    fn drive_all(&mut self, now: u64) -> Result<(), String> {
        for key in self.registry.active_keys() {
            self.drive(key, now)?;
        }
        Ok(())
    }

    /// Advance one candidate by at most one meaningful step, using injected
    /// signals. Never advances into or past the injection commit line except via
    /// the registry's guarded `advance` (SINGLE-INJECTION LAW).
    fn drive(&mut self, key: ProcessKey, now: u64) -> Result<(), String> {
        let state = match self.registry.get(&key) {
            Some(c) if !c.state.is_terminal() => c.state,
            _ => return Ok(()),
        };
        match state {
            CandidateState::Detected => {
                self.transition(
                    key,
                    CandidateState::WaitingForIdentity,
                    ReasonCode::IdentityUnresolved,
                    now,
                    None,
                )?;
            }
            CandidateState::WaitingForIdentity => {
                let id = self.registry.get(&key).unwrap().identity.clone();
                let modset = self.deps.env.resolve_modset(&id);
                if modset.resolved {
                    self.transition(
                        key,
                        CandidateState::WaitingForWindow,
                        ReasonCode::NoVisibleWindow,
                        now,
                        None,
                    )?;
                } else {
                    // Hold, fail-closed, backing off. Not journalled every tick.
                    self.registry.bump_readiness(&key);
                }
            }
            CandidateState::WaitingForWindow => {
                if self.deps.env.has_visible_window(key) {
                    self.transition(
                        key,
                        CandidateState::Preparing,
                        ReasonCode::Preparing,
                        now,
                        None,
                    )?;
                } else {
                    self.registry.bump_readiness(&key);
                }
            }
            CandidateState::Preparing => {
                self.prepare_and_inject(key, now)?;
            }
            CandidateState::Injecting
            | CandidateState::Handshaking
            | CandidateState::WaitingForWorld
            | CandidateState::Recording
            | CandidateState::Finalizing => {
                self.drive_worker(key, now)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// `Preparing`: stage a fresh unique DLL, verify its hash, then — guarded by
    /// the no-double-inject registry — spawn exactly one worker and commit to
    /// `Injecting` (design §5). Any failure fails the attempt closed (loud); we
    /// never leave a half-committed key that could be re-injected.
    fn prepare_and_inject(&mut self, key: ProcessKey, now: u64) -> Result<(), String> {
        let attempt_id = generate_attempt_id(&self.config.coordinator_instance_id, key, now);
        let (pipe_token, pipe_token_hash) = match generate_pipe_token() {
            Ok(value) => value,
            Err(e) => {
                self.fail(key, ReasonCode::DiskFailure, now, Some(e))?;
                return Ok(());
            }
        };
        let control_challenge = match generate_secret_128() {
            Ok(value) => value,
            Err(e) => {
                self.fail(key, ReasonCode::DiskFailure, now, Some(e))?;
                return Ok(());
            }
        };
        let payload = match self
            .deps
            .stager
            .stage(key, &attempt_id, &pipe_token, &pipe_token_hash)
        {
            Ok(p) => p,
            Err(e) => {
                self.fail(
                    key,
                    ReasonCode::DiskFailure,
                    now,
                    Some(format!("stage failed: {e}")),
                )?;
                return Ok(());
            }
        };
        if payload.dll_sha256 != payload.source_sha256 {
            self.fail(
                key,
                ReasonCode::PayloadHashMismatch,
                now,
                Some("staged dll hash mismatch".into()),
            )?;
            return Ok(());
        }
        // Guard BEFORE spawning: if the registry will not permit injection, stop.
        if !self.registry.may_start_injection(&key) {
            self.fail(
                key,
                ReasonCode::IdentityMismatchAtInject,
                now,
                Some("no-double-inject guard refused".into()),
            )?;
            return Ok(());
        }
        // Re-probe every authority field and the PID-owned visible window at the
        // last possible point. Cached registry identity is never sufficient for
        // a live PID because the PID may have been reused while staging.
        let expected_identity = self
            .registry
            .get(&key)
            .map(|candidate| candidate.identity.clone())
            .ok_or_else(|| format!("pre-injection identity {key:?} disappeared"))?;
        let reprobed = match self.deps.probe.identify(key.pid) {
            Ok(identity) => identity,
            Err(e) => {
                self.fail(
                    key,
                    ReasonCode::IdentityMismatchAtInject,
                    now,
                    Some(format!("pre-injection identity re-probe failed: {e}")),
                )?;
                return Ok(());
            }
        };
        if reprobed != expected_identity
            || !reprobed.is_hoi4()
            || !self.same_user(&reprobed)
            || !self.deps.env.has_visible_window(key)
        {
            self.fail(
                key,
                ReasonCode::IdentityMismatchAtInject,
                now,
                Some("pre-injection identity/window-owner comparison failed".into()),
            )?;
            return Ok(());
        }
        let out_dir = self.config.out_root.join(format!("run_{attempt_id}"));
        let session_id = session_uuid_from_pipe_token(&pipe_token)?;
        let spec = WorkerSpec {
            key,
            attempt_id: attempt_id.clone(),
            payload: payload.clone(),
            out_dir,
            session_id: session_id.clone(),
            control_challenge,
        };
        // This synced journal record is the commit line. It precedes child
        // creation, so a crash cannot erase knowledge of a possibly-started
        // injector. Every post-intent failure stays poisoned for this key.
        self.registry
            .advance(&key, CandidateState::Injecting, now)
            .map_err(|e| format!("commit injection intent: {e}"))?;
        if let Some(c) = self.registry.get_mut(&key) {
            c.attempt_id = Some(attempt_id.clone());
            c.session_id = Some(session_id);
            c.payload = Some(payload.clone());
        }
        self.journal_transition(
            key,
            Some(CandidateState::Preparing),
            CandidateState::Injecting,
            ReasonCode::Injected,
            Some(format!("durable committed intent for {attempt_id}")),
        )?;
        let handle = match self.deps.spawner.spawn(&spec) {
            Ok(h) => h,
            Err(e) => {
                // The child never started, so no hook was installed. Fail closed
                // and do NOT retry in the same process identity (conservative;
                // design §5 self-refutation "retry is not automatically safe").
                self.fail(
                    key,
                    ReasonCode::DiskFailure,
                    now,
                    Some(format!("spawn failed: {e}")),
                )?;
                return Ok(());
            }
        };
        self.workers.insert(key, handle);
        Ok(())
    }

    /// Advance a committed candidate by polling its worker (design §5 ladder:
    /// heartbeat for residency, then a valid session frame for recording). Also
    /// enforces the handshake deadline and the post-commit process-exit
    /// finalization.
    fn drive_worker(&mut self, key: ProcessKey, now: u64) -> Result<(), String> {
        let (state, entered, gone) = {
            let c = self.registry.get(&key).unwrap();
            (c.state, c.state_entered_ms, c.process_gone_after_commit)
        };

        // Post-commit process exit: finalize the valid prefix (design §6).
        if gone
            && matches!(
                state,
                CandidateState::Recording
                    | CandidateState::WaitingForWorld
                    | CandidateState::Handshaking
                    | CandidateState::Injecting
            )
        {
            if let Some(w) = self.workers.get_mut(&key) {
                w.request_stop();
            }
            if state == CandidateState::Recording {
                self.transition(
                    key,
                    CandidateState::Finalizing,
                    ReasonCode::ProcessExited,
                    now,
                    Some("process exited; finalizing prefix".into()),
                )?;
                return Ok(());
            }
            // Exited before a valid frame: no recording to finalize -> Failed
            // (retains diagnostics; nothing to promote).
            self.fail(
                key,
                ReasonCode::ProcessExited,
                now,
                Some("process exited before first valid frame".into()),
            )?;
            return Ok(());
        }

        // Handshake deadline (design §4): loud timeout, no second DLL.
        if state == CandidateState::Handshaking
            && now.saturating_sub(entered) > policy::HANDSHAKE_DEADLINE_MS
        {
            if let Some(w) = self.workers.get_mut(&key) {
                w.request_stop();
            }
            self.fail(
                key,
                ReasonCode::HeartbeatTimeout,
                now,
                Some("heartbeat deadline exceeded".into()),
            )?;
            return Ok(());
        }

        let status = match self.workers.get_mut(&key) {
            Some(w) => w.poll(),
            None => {
                // No worker but in a committed state: inconsistent -> fail closed.
                self.fail(
                    key,
                    ReasonCode::HeartbeatMismatch,
                    now,
                    Some("committed state with no worker handle".into()),
                )?;
                return Ok(());
            }
        };

        match (state, status) {
            (CandidateState::Injecting, WorkerStatus::Handshaking)
            | (CandidateState::Injecting, WorkerStatus::Attached) => {
                self.transition(
                    key,
                    CandidateState::Handshaking,
                    ReasonCode::Injected,
                    now,
                    None,
                )?;
            }
            (CandidateState::Handshaking, WorkerStatus::Attached) => {
                self.transition(
                    key,
                    CandidateState::WaitingForWorld,
                    ReasonCode::CollectorAttached,
                    now,
                    None,
                )?;
            }
            (
                CandidateState::WaitingForWorld,
                WorkerStatus::Recording {
                    last_confirmed_hour,
                },
            )
            | (
                CandidateState::Handshaking,
                WorkerStatus::Recording {
                    last_confirmed_hour,
                },
            ) => {
                // A worker may report Recording directly from Handshaking if the
                // heartbeat and first frame arrive between polls; still route
                // through the ladder via WaitingForWorld first if needed.
                if state == CandidateState::Handshaking {
                    self.transition(
                        key,
                        CandidateState::WaitingForWorld,
                        ReasonCode::CollectorAttached,
                        now,
                        None,
                    )?;
                }
                if let Some(c) = self.registry.get_mut(&key) {
                    c.last_confirmed_hour = last_confirmed_hour;
                }
                self.transition(
                    key,
                    CandidateState::Recording,
                    ReasonCode::RecordingStarted,
                    now,
                    None,
                )?;
            }
            (
                CandidateState::Recording,
                WorkerStatus::Recording {
                    last_confirmed_hour,
                },
            ) => {
                if let Some(c) = self.registry.get_mut(&key) {
                    c.last_confirmed_hour = last_confirmed_hour;
                }
            }
            (CandidateState::Recording, WorkerStatus::Finalizing) => {
                self.transition(
                    key,
                    CandidateState::Finalizing,
                    ReasonCode::ReplayComplete,
                    now,
                    None,
                )?;
            }
            (CandidateState::Finalizing, WorkerStatus::Complete { replay_path }) => {
                self.complete(key, now, replay_path)?;
            }
            (CandidateState::Recording, WorkerStatus::Complete { replay_path }) => {
                // Worker jumped straight to complete: route through Finalizing.
                self.transition(
                    key,
                    CandidateState::Finalizing,
                    ReasonCode::ReplayComplete,
                    now,
                    None,
                )?;
                self.complete(key, now, replay_path)?;
            }
            (
                CandidateState::Injecting
                | CandidateState::Handshaking
                | CandidateState::WaitingForWorld,
                WorkerStatus::Complete { .. },
            ) => {
                self.fail(
                    key,
                    ReasonCode::HeartbeatMismatch,
                    now,
                    Some("worker completed before matched hello/first-frame proof".into()),
                )?;
            }
            (_, WorkerStatus::Failed { reason }) => {
                if let Some(worker) = self.workers.get_mut(&key) {
                    worker.request_stop();
                }
                self.fail(key, ReasonCode::FinalizeVerifyFailed, now, Some(reason))?;
            }
            _ => { /* no change this tick */ }
        }
        Ok(())
    }

    fn transition(
        &mut self,
        key: ProcessKey,
        next: CandidateState,
        reason: ReasonCode,
        now: u64,
        detail: Option<String>,
    ) -> Result<(), String> {
        let from = self.registry.advance(&key, next, now)?;
        self.journal_transition(key, Some(from), next, reason, detail)?;
        if next == CandidateState::Recording && from != CandidateState::Recording {
            if let Some(run_dir) = self.run_dir_for(key) {
                self.supervisor_events
                    .push(SupervisorEvent::RecordingStarted {
                        pid: key.pid,
                        run_dir,
                    });
            }
        }
        Ok(())
    }

    fn complete(&mut self, key: ProcessKey, now: u64, replay_path: String) -> Result<(), String> {
        let from = self.registry.advance(&key, CandidateState::Complete, now)?;
        let mut rec = self.base_record(
            key,
            Some(from),
            CandidateState::Complete,
            ReasonCode::ReplayComplete,
        );
        rec.replay_path = Some(replay_path.clone());
        rec.artifact_status = Some("verified".to_string());
        self.journal.append(rec)?;
        self.supervisor_events
            .push(SupervisorEvent::RecordingSucceeded {
                pid: key.pid,
                replay_path: PathBuf::from(replay_path),
            });
        self.workers.remove(&key);
        Ok(())
    }

    fn fail(
        &mut self,
        key: ProcessKey,
        reason: ReasonCode,
        now: u64,
        detail: Option<String>,
    ) -> Result<(), String> {
        if let Some(from) = self.registry.fail(&key) {
            let _ = now;
            self.journal_transition(
                key,
                Some(from),
                CandidateState::Failed,
                reason,
                detail.clone(),
            )?;
            self.supervisor_events.push(SupervisorEvent::RecordingFailed {
                pid: key.pid,
                reason,
                detail,
                run_dir: self.run_dir_for(key),
            });
        }
        self.workers.remove(&key);
        Ok(())
    }

    fn run_dir_for(&self, key: ProcessKey) -> Option<PathBuf> {
        self.registry
            .get(&key)
            .and_then(|candidate| candidate.attempt_id.as_ref())
            .map(|attempt_id| self.config.out_root.join(format!("run_{attempt_id}")))
    }

    fn base_record(
        &self,
        key: ProcessKey,
        from: Option<CandidateState>,
        to: CandidateState,
        reason: ReasonCode,
    ) -> JournalRecord {
        let now = self.deps.clock.now_ms();
        let mut rec = JournalRecord::transition(
            0,
            now,
            self.config.coordinator_instance_id.clone(),
            key,
            from,
            to,
            reason,
        );
        if let Some(c) = self.registry.get(&key) {
            rec.exe_path = Some(c.identity.image_path.clone());
            rec.process_session_id = Some(c.identity.session_id);
            if !c.identity.user_sid_hash.is_empty() {
                rec.process_user_sid_hash = Some(c.identity.user_sid_hash.clone());
            }
            rec.attempt_id = c.attempt_id.clone();
            rec.session_id = c.session_id.clone();
            rec.last_confirmed_hour = c.last_confirmed_hour;
            if let Some(p) = &c.payload {
                rec.staging_dir = Some(p.staging_dir.display().to_string());
                rec.pipe_token_hash = Some(p.pipe_token_hash.clone());
                rec.exe_sha256 = None;
            }
        }
        rec
    }

    fn journal_transition(
        &mut self,
        key: ProcessKey,
        from: Option<CandidateState>,
        to: CandidateState,
        reason: ReasonCode,
        detail: Option<String>,
    ) -> Result<(), String> {
        let mut rec = self.base_record(key, from, to, reason);
        rec.reason_detail = detail;
        self.journal.append(rec)?;
        Ok(())
    }
}

fn session_uuid_from_pipe_token(pipe_token: &str) -> Result<String, String> {
    if pipe_token.len() != 32
        || !pipe_token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("auto-attach pipe token is not canonical 128-bit lowercase hex".to_string());
    }
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &pipe_token[0..8],
        &pipe_token[8..12],
        &pipe_token[12..16],
        &pipe_token[16..20],
        &pipe_token[20..32],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_attach::policy::WatcherHealth;
    use crate::auto_attach::sources::{
        ModsetResolution, ProcessSnapshot, SnapshotEntry, StagedPayload,
    };
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    // ---- Fakes -------------------------------------------------------

    #[derive(Clone, Default)]
    struct FakeClock {
        now: Rc<RefCell<u64>>,
    }
    impl FakeClock {
        fn set(&self, v: u64) {
            *self.now.borrow_mut() = v;
        }
    }
    impl Clock for FakeClock {
        fn now_ms(&self) -> u64 {
            *self.now.borrow()
        }
    }
    // SAFETY: single-threaded test use only; Rc is not Send but the trait
    // requires Send. We wrap in a newtype asserting Send for the test harness.
    unsafe impl Send for FakeClock {}

    #[derive(Clone)]
    struct FakeProbe {
        // pid -> identity
        table: Rc<RefCell<BTreeMap<u32, ProcessIdentity>>>,
        // live pids present in the snapshot
        live: Rc<RefCell<Vec<u32>>>,
        fail_identify: Rc<RefCell<BTreeSet<u32>>>,
        snapshot_error: Rc<RefCell<Option<String>>>,
        snapshot_complete: Rc<RefCell<bool>>,
        identify_script: Rc<RefCell<BTreeMap<u32, VecDeque<ProcessIdentity>>>>,
    }
    impl Default for FakeProbe {
        fn default() -> Self {
            Self {
                table: Rc::new(RefCell::new(BTreeMap::new())),
                live: Rc::new(RefCell::new(Vec::new())),
                fail_identify: Rc::new(RefCell::new(BTreeSet::new())),
                snapshot_error: Rc::new(RefCell::new(None)),
                snapshot_complete: Rc::new(RefCell::new(true)),
                identify_script: Rc::new(RefCell::new(BTreeMap::new())),
            }
        }
    }
    impl FakeProbe {
        fn add(&self, id: ProcessIdentity) {
            self.table.borrow_mut().insert(id.key.pid, id.clone());
            self.live.borrow_mut().push(id.key.pid);
        }
        fn remove_live(&self, pid: u32) {
            self.live.borrow_mut().retain(|p| *p != pid);
        }
        fn script_identities(&self, pid: u32, identities: Vec<ProcessIdentity>) {
            self.identify_script
                .borrow_mut()
                .insert(pid, identities.into());
        }
        fn fail_next_snapshot(&self, reason: &str) {
            *self.snapshot_error.borrow_mut() = Some(reason.to_string());
        }
    }
    impl ProcessProbe for FakeProbe {
        fn snapshot(&self) -> Result<ProcessSnapshot, String> {
            if let Some(error) = self.snapshot_error.borrow_mut().take() {
                return Err(error);
            }
            let table = self.table.borrow();
            let entries = self
                .live
                .borrow()
                .iter()
                .filter_map(|pid| {
                    table.get(pid).map(|id| SnapshotEntry {
                        pid: *pid,
                        name: id.image_basename_lower(),
                    })
                })
                .collect();
            if *self.snapshot_complete.borrow() {
                Ok(ProcessSnapshot::complete(entries))
            } else {
                Ok(ProcessSnapshot::incomplete(entries, "forced truncation"))
            }
        }
        fn identify(&self, pid: u32) -> Result<ProcessIdentity, String> {
            if self.fail_identify.borrow().contains(&pid) {
                return Err(format!("forced identity failure for {pid}"));
            }
            if let Some(identity) = self
                .identify_script
                .borrow_mut()
                .get_mut(&pid)
                .and_then(VecDeque::pop_front)
            {
                return Ok(identity);
            }
            self.table
                .borrow()
                .get(&pid)
                .cloned()
                .ok_or_else(|| format!("no identity for {pid}"))
        }
    }
    unsafe impl Send for FakeProbe {}

    #[derive(Clone)]
    struct FakeEvents {
        queue: Rc<RefCell<Vec<RawProcessEvent>>>,
        health: Rc<RefCell<WatcherHealth>>,
    }
    impl FakeEvents {
        fn healthy() -> Self {
            Self {
                queue: Rc::new(RefCell::new(Vec::new())),
                health: Rc::new(RefCell::new(WatcherHealth::Healthy)),
            }
        }
        fn push(&self, ev: RawProcessEvent) {
            self.queue.borrow_mut().push(ev);
        }
        fn set_health(&self, h: WatcherHealth) {
            *self.health.borrow_mut() = h;
        }
    }
    impl ProcessEventSource for FakeEvents {
        fn drain_events(&mut self) -> Vec<RawProcessEvent> {
            std::mem::take(&mut *self.queue.borrow_mut())
        }
        fn health(&self) -> WatcherHealth {
            *self.health.borrow()
        }
    }
    unsafe impl Send for FakeEvents {}

    #[derive(Clone)]
    struct FakeEnv {
        window_ready: Rc<RefCell<bool>>,
        modset_resolved: Rc<RefCell<bool>>,
    }
    impl Default for FakeEnv {
        fn default() -> Self {
            Self {
                window_ready: Rc::new(RefCell::new(true)),
                modset_resolved: Rc::new(RefCell::new(true)),
            }
        }
    }
    impl AttachEnvironment for FakeEnv {
        fn has_visible_window(&self, _key: ProcessKey) -> bool {
            *self.window_ready.borrow()
        }
        fn resolve_modset(&self, _id: &ProcessIdentity) -> ModsetResolution {
            ModsetResolution {
                resolved: *self.modset_resolved.borrow(),
                reasons: vec![],
            }
        }
    }
    unsafe impl Send for FakeEnv {}

    #[derive(Clone, Default)]
    struct FakeStager {
        fail: Rc<RefCell<bool>>,
        hash_mismatch: Rc<RefCell<bool>>,
        staged: Rc<RefCell<Vec<String>>>,
    }
    impl PayloadStager for FakeStager {
        fn stage(
            &self,
            _key: ProcessKey,
            attempt_id: &str,
            pipe_token: &str,
            pipe_token_hash: &str,
        ) -> Result<StagedPayload, String> {
            if *self.fail.borrow() {
                return Err("forced stage failure".into());
            }
            self.staged.borrow_mut().push(attempt_id.to_string());
            let (src, dll) = if *self.hash_mismatch.borrow() {
                ("aaa".to_string(), "bbb".to_string())
            } else {
                ("same".to_string(), "same".to_string())
            };
            Ok(StagedPayload {
                staging_dir: PathBuf::from(format!("stage/{attempt_id}")),
                dll_path: PathBuf::from(format!("stage/{attempt_id}/collector.dll")),
                control_path: PathBuf::from(format!("stage/{attempt_id}/control.json")),
                dll_sha256: dll,
                source_sha256: src,
                pipe_token: pipe_token.to_string(),
                pipe_token_hash: pipe_token_hash.to_string(),
            })
        }
    }
    unsafe impl Send for FakeStager {}

    // A scripted worker: yields a sequence of statuses on successive polls,
    // holding the last one forever. Records spawn count globally for the
    // no-double-inject assertions.
    #[derive(Clone)]
    struct FakeSpawner {
        script: Rc<RefCell<Vec<WorkerStatus>>>,
        spawn_count: Rc<RefCell<usize>>,
        fail_spawn: Rc<RefCell<bool>>,
        spawned_specs: Rc<RefCell<Vec<WorkerSpec>>>,
        journal_path: Rc<RefCell<Option<PathBuf>>>,
        durable_intent_seen_at_spawn: Rc<RefCell<Option<bool>>>,
        stop_count: Rc<RefCell<usize>>,
        drop_count: Rc<RefCell<usize>>,
    }
    impl FakeSpawner {
        fn new(script: Vec<WorkerStatus>) -> Self {
            Self {
                script: Rc::new(RefCell::new(script)),
                spawn_count: Rc::new(RefCell::new(0)),
                fail_spawn: Rc::new(RefCell::new(false)),
                spawned_specs: Rc::new(RefCell::new(Vec::new())),
                journal_path: Rc::new(RefCell::new(None)),
                durable_intent_seen_at_spawn: Rc::new(RefCell::new(None)),
                stop_count: Rc::new(RefCell::new(0)),
                drop_count: Rc::new(RefCell::new(0)),
            }
        }
    }
    impl WorkerSpawner for FakeSpawner {
        fn spawn(&self, spec: &WorkerSpec) -> Result<Box<dyn WorkerHandle>, String> {
            self.spawned_specs.borrow_mut().push(spec.clone());
            if let Some(path) = self.journal_path.borrow().clone() {
                let seen = read_all(path)
                    .ok()
                    .and_then(|records| records.last().cloned())
                    .map(|record| {
                        record.key() == spec.key
                            && record.state_to == CandidateState::Injecting
                            && record.attempt_id.as_deref() == Some(spec.attempt_id.as_str())
                            && record.session_id.as_deref() == Some(spec.session_id.as_str())
                            && record.pipe_token_hash.as_deref()
                                == Some(spec.payload.pipe_token_hash.as_str())
                    })
                    .unwrap_or(false);
                *self.durable_intent_seen_at_spawn.borrow_mut() = Some(seen);
            }
            if *self.fail_spawn.borrow() {
                return Err("forced spawn failure".into());
            }
            *self.spawn_count.borrow_mut() += 1;
            Ok(Box::new(FakeWorker {
                script: self.script.borrow().clone(),
                idx: 0,
                stop_count: self.stop_count.clone(),
                drop_count: self.drop_count.clone(),
            }))
        }
    }
    unsafe impl Send for FakeSpawner {}

    struct FakeWorker {
        script: Vec<WorkerStatus>,
        idx: usize,
        stop_count: Rc<RefCell<usize>>,
        drop_count: Rc<RefCell<usize>>,
    }
    impl WorkerHandle for FakeWorker {
        fn poll(&mut self) -> WorkerStatus {
            let s = self.script.get(self.idx).cloned().unwrap_or_else(|| {
                self.script
                    .last()
                    .cloned()
                    .unwrap_or(WorkerStatus::Handshaking)
            });
            if self.idx + 1 < self.script.len() {
                self.idx += 1;
            }
            s
        }
        fn request_stop(&mut self) {
            *self.stop_count.borrow_mut() += 1;
        }
    }
    unsafe impl Send for FakeWorker {}

    impl Drop for FakeWorker {
        fn drop(&mut self) {
            *self.drop_count.borrow_mut() += 1;
        }
    }

    // ---- Harness -----------------------------------------------------

    const OWN_SESSION: u32 = 7;
    const OWN_SID: &str = "own-sid-hash";

    fn hoi4_identity(pid: u32, ft: u64) -> ProcessIdentity {
        ProcessIdentity {
            key: ProcessKey::new(pid, ft),
            image_path: r"D:\Steam\common\the game\hoi4.exe".to_string(),
            session_id: OWN_SESSION,
            user_sid_hash: OWN_SID.to_string(),
        }
    }

    struct Harness {
        clock: FakeClock,
        probe: FakeProbe,
        events: FakeEvents,
        env: FakeEnv,
        stager: FakeStager,
        spawner: FakeSpawner,
        journal_path: PathBuf,
    }

    fn journal_path(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "aa_coord_{tag}_{}_{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&p);
        p
    }

    fn build(h: &Harness) -> Coordinator {
        *h.spawner.journal_path.borrow_mut() = Some(h.journal_path.clone());
        let config = CoordinatorConfig {
            coordinator_instance_id: "coord-test".into(),
            out_root: PathBuf::from("out"),
            own_session_id: OWN_SESSION,
            own_sid_hash: OWN_SID.to_string(),
        };
        let deps = CoordinatorDeps {
            clock: Box::new(h.clock.clone()),
            probe: Box::new(h.probe.clone()),
            events: Box::new(h.events.clone()),
            env: Box::new(h.env.clone()),
            stager: Box::new(h.stager.clone()),
            spawner: Box::new(h.spawner.clone()),
        };
        Coordinator::new(config, deps, h.journal_path.clone()).unwrap()
    }

    fn harness(tag: &str, worker_script: Vec<WorkerStatus>) -> Harness {
        Harness {
            clock: FakeClock::default(),
            probe: FakeProbe::default(),
            events: FakeEvents::healthy(),
            env: FakeEnv::default(),
            stager: FakeStager::default(),
            spawner: FakeSpawner::new(worker_script),
            journal_path: journal_path(tag),
        }
    }

    fn full_success_script() -> Vec<WorkerStatus> {
        vec![
            WorkerStatus::Handshaking,
            WorkerStatus::Attached,
            WorkerStatus::Recording {
                last_confirmed_hour: Some(1936_010101),
            },
            WorkerStatus::Finalizing,
            WorkerStatus::Complete {
                replay_path: "out/replay.battleplan".into(),
            },
        ]
    }

    fn run_steps(coord: &mut Coordinator, clock: &FakeClock, n: usize) {
        for i in 0..n {
            clock.set((i as u64 + 1) * 10);
            coord.step().unwrap();
        }
    }

    fn state_of(coord: &Coordinator, key: ProcessKey) -> Option<CandidateState> {
        coord.registry().get(&key).map(|c| c.state)
    }

    // ---- Gate 1: dual order ------------------------------------------

    #[test]
    fn gate1_game_first_startup_snapshot_attaches() {
        // Game already running before the coordinator starts (game-first).
        let h = harness("g1_gamefirst", full_success_script());
        h.probe.add(hoi4_identity(1000, 111));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(
            state_of(&coord, ProcessKey::new(1000, 111)),
            Some(CandidateState::Complete)
        );
        let events = coord.drain_supervisor_events();
        assert!(matches!(
            events.as_slice(),
            [
                SupervisorEvent::RecordingStarted { pid: 1000, run_dir },
                SupervisorEvent::RecordingSucceeded { pid: 1000, replay_path }
            ] if run_dir.starts_with("out")
                && run_dir.file_name().is_some_and(|name| name.to_string_lossy().starts_with("run_"))
                && replay_path == &PathBuf::from("out/replay.battleplan")
        ));
        assert_eq!(*h.spawner.spawn_count.borrow(), 1, "exactly one injection");
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn gate1_app_first_wmi_start_attaches() {
        // App started first; the game launches later and is seen via a WMI
        // start event (app-first).
        let h = harness("g1_appfirst", full_success_script());
        let mut coord = build(&h);
        // First step: empty snapshot, nothing detected.
        h.clock.set(10);
        coord.step().unwrap();
        assert!(coord.registry().is_empty());
        // Game launches: probe learns it, WMI start fires.
        h.probe.add(hoi4_identity(2000, 222));
        h.events.push(RawProcessEvent::Start {
            pid: 2000,
            name: "hoi4.exe".into(),
        });
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(
            state_of(&coord, ProcessKey::new(2000, 222)),
            Some(CandidateState::Complete)
        );
        assert_eq!(*h.spawner.spawn_count.borrow(), 1);
        let _ = std::fs::remove_file(&h.journal_path);
    }

    // ---- Gate 2: subscribe race / dedup ------------------------------

    #[test]
    fn gate2_snapshot_and_wmi_produce_one_attempt() {
        // The same game appears in BOTH the startup snapshot and a WMI start
        // event; exactly one candidate, one injection.
        let h = harness("g2_race", full_success_script());
        h.probe.add(hoi4_identity(3000, 333));
        let mut coord = build(&h);
        // Also deliver a duplicate WMI start for the same pid.
        h.events.push(RawProcessEvent::Start {
            pid: 3000,
            name: "HOI4.EXE".into(),
        });
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(
            coord.registry().len(),
            1,
            "one candidate despite two sources"
        );
        assert_eq!(
            *h.spawner.spawn_count.borrow(),
            1,
            "one injection despite two sources"
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    // ---- Gate 3: PID reuse -------------------------------------------

    #[test]
    fn gate3_reused_pid_is_new_attempt() {
        let h = harness("g3_reuse", full_success_script());
        h.probe.add(hoi4_identity(4000, 444));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(
            state_of(&coord, ProcessKey::new(4000, 444)),
            Some(CandidateState::Complete)
        );
        // Old process exits; a NEW process reuses pid 4000 with a new FILETIME.
        // Discovered via its WMI start event (app-first low-latency path) so we
        // don't wait for the 30 s healthy reconcile in the test.
        h.probe.remove_live(4000);
        h.probe.add(hoi4_identity(4000, 999));
        h.events.push(RawProcessEvent::Start {
            pid: 4000,
            name: "hoi4.exe".into(),
        });
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(
            coord.registry().len(),
            2,
            "reused pid is a distinct identity"
        );
        assert_eq!(
            state_of(&coord, ProcessKey::new(4000, 999)),
            Some(CandidateState::Complete)
        );
        assert_eq!(
            *h.spawner.spawn_count.borrow(),
            2,
            "two independent injections"
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    // ---- Gate 4: WMI loss / recovery cadence -------------------------

    #[test]
    fn gate4_reconcile_cadence_tracks_health() {
        let h = harness("g4_health", full_success_script());
        let mut coord = build(&h);
        // Healthy: 30s cadence. At t=10 first snapshot ran; not due again at 20s.
        h.clock.set(10);
        coord.step().unwrap();
        assert!(!coord.reconcile_due(20_000));
        assert!(coord.reconcile_due(40_000), "due after 30s healthy");
        // Degraded: 2s cadence.
        h.events.set_health(WatcherHealth::Degraded);
        assert!(coord.reconcile_due(10 + 2_000));
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn gate4_degraded_still_discovers_game_first() {
        // With WMI degraded (no events), pure snapshot polling still attaches.
        let h = harness("g4_degraded", full_success_script());
        h.events.set_health(WatcherHealth::Degraded);
        h.probe.add(hoi4_identity(4200, 42));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(
            state_of(&coord, ProcessKey::new(4200, 42)),
            Some(CandidateState::Complete)
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    // ---- Gate 5: identity fail-close ---------------------------------

    #[test]
    fn gate5_wrong_session_never_admitted() {
        let h = harness("g5_session", full_success_script());
        let mut bad = hoi4_identity(5000, 55);
        bad.session_id = OWN_SESSION + 1; // different session
        h.probe.add(bad);
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 12);
        assert!(
            coord.registry().is_empty(),
            "wrong-session the game never becomes a candidate"
        );
        assert_eq!(*h.spawner.spawn_count.borrow(), 0);
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn gate5_wrong_user_never_admitted() {
        let h = harness("g5_user", full_success_script());
        let mut bad = hoi4_identity(5100, 56);
        bad.user_sid_hash = "someone-else".into();
        h.probe.add(bad);
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 12);
        assert!(coord.registry().is_empty());
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn gate5_unresolved_modset_blocks_injection() {
        let h = harness("g5_modset", full_success_script());
        *h.env.modset_resolved.borrow_mut() = false;
        h.probe.add(hoi4_identity(5200, 57));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 20);
        assert_eq!(
            state_of(&coord, ProcessKey::new(5200, 57)),
            Some(CandidateState::WaitingForIdentity),
            "unresolved modset holds before injection, fail-closed"
        );
        assert_eq!(*h.spawner.spawn_count.borrow(), 0, "never injected");
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn gate5_no_window_holds_before_injection() {
        let h = harness("g5_window", full_success_script());
        *h.env.window_ready.borrow_mut() = false;
        h.probe.add(hoi4_identity(5300, 58));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 20);
        assert_eq!(
            state_of(&coord, ProcessKey::new(5300, 58)),
            Some(CandidateState::WaitingForWindow)
        );
        assert_eq!(*h.spawner.spawn_count.borrow(), 0);
        // Once the window appears, it proceeds.
        *h.env.window_ready.borrow_mut() = true;
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(
            state_of(&coord, ProcessKey::new(5300, 58)),
            Some(CandidateState::Complete)
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn gate5_stage_hash_mismatch_fails_closed() {
        let h = harness("g5_hash", full_success_script());
        *h.stager.hash_mismatch.borrow_mut() = true;
        h.probe.add(hoi4_identity(5400, 59));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(
            state_of(&coord, ProcessKey::new(5400, 59)),
            Some(CandidateState::Failed)
        );
        assert_eq!(
            *h.spawner.spawn_count.borrow(),
            0,
            "hash mismatch => no injection"
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    // ---- Gate 6/7: pipe barrier + injection uncertainty --------------

    #[test]
    fn gate6_no_heartbeat_never_reaches_recording() {
        // Worker stuck in Handshaking forever: never reaches WaitingForWorld or
        // Recording, and the deadline eventually fails it — with NO second DLL.
        let h = harness("g6_hb", vec![WorkerStatus::Handshaking]);
        h.probe.add(hoi4_identity(6000, 66));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 12);
        // It injected once and is Handshaking, not Recording.
        let s = state_of(&coord, ProcessKey::new(6000, 66)).unwrap();
        assert!(matches!(
            s,
            CandidateState::Handshaking | CandidateState::Injecting
        ));
        assert_eq!(*h.spawner.spawn_count.borrow(), 1);
        // Advance well past the handshake deadline.
        h.clock.set(policy::HANDSHAKE_DEADLINE_MS + 1_000_000);
        coord.step().unwrap();
        assert_eq!(
            state_of(&coord, ProcessKey::new(6000, 66)),
            Some(CandidateState::Failed)
        );
        assert_eq!(
            *h.spawner.spawn_count.borrow(),
            1,
            "timeout does NOT trigger a second DLL"
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn gate7_worker_failure_no_second_injection() {
        let h = harness(
            "g7_fail",
            vec![
                WorkerStatus::Handshaking,
                WorkerStatus::Failed {
                    reason: "hook rejected".into(),
                },
            ],
        );
        h.probe.add(hoi4_identity(7000, 77));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(
            state_of(&coord, ProcessKey::new(7000, 77)),
            Some(CandidateState::Failed)
        );
        assert_eq!(
            *h.spawner.spawn_count.borrow(),
            1,
            "a failed attempt is never retried in-process"
        );
        let events = coord.drain_supervisor_events();
        assert!(matches!(
            events.as_slice(),
            [SupervisorEvent::RecordingFailed {
                pid: 7000,
                reason: ReasonCode::FinalizeVerifyFailed,
                detail: Some(detail),
                run_dir: Some(run_dir),
            }] if detail == "hook rejected" && run_dir.starts_with("out")
        ));
        // Even if it stays live and we keep stepping, no re-injection.
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(*h.spawner.spawn_count.borrow(), 1);
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn gate7_spawn_failure_fails_closed() {
        let h = harness("g7_spawn", full_success_script());
        *h.spawner.fail_spawn.borrow_mut() = true;
        h.probe.add(hoi4_identity(7100, 78));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(
            state_of(&coord, ProcessKey::new(7100, 78)),
            Some(CandidateState::Failed)
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    // ---- Gate 9: multi-instance --------------------------------------

    #[test]
    fn gate9_two_games_attach_independently() {
        let h = harness("g9_multi", full_success_script());
        h.probe.add(hoi4_identity(9000, 90));
        h.probe.add(hoi4_identity(9001, 91));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 16);
        assert_eq!(
            state_of(&coord, ProcessKey::new(9000, 90)),
            Some(CandidateState::Complete)
        );
        assert_eq!(
            state_of(&coord, ProcessKey::new(9001, 91)),
            Some(CandidateState::Complete)
        );
        assert_eq!(
            *h.spawner.spawn_count.borrow(),
            2,
            "one injection per game, no merge"
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    // ---- Gate 10: crash recovery -------------------------------------

    #[test]
    fn gate10_recovery_poisons_committed_identity() {
        // Simulate a prior run that committed an injection then crashed: the
        // journal ends with the identity in Injecting. On restart the same live
        // process must NEVER be re-injected.
        let path = journal_path("g10");
        {
            let mut j = Journal::open(&path).unwrap();
            let key = ProcessKey::new(10_000, 100);
            j.append(JournalRecord::transition(
                0,
                1,
                "old",
                key,
                None,
                CandidateState::Detected,
                ReasonCode::Detected,
            ))
            .unwrap();
            j.append(JournalRecord::transition(
                0,
                2,
                "old",
                key,
                Some(CandidateState::WaitingForWindow),
                CandidateState::Injecting,
                ReasonCode::Injected,
            ))
            .unwrap();
        }
        let h = Harness {
            clock: FakeClock::default(),
            probe: FakeProbe::default(),
            events: FakeEvents::healthy(),
            env: FakeEnv::default(),
            stager: FakeStager::default(),
            spawner: FakeSpawner::new(full_success_script()),
            journal_path: path.clone(),
        };
        // The same live process is still running.
        h.probe.add(hoi4_identity(10_000, 100));
        let mut coord = build(&h);
        let poisoned = coord.recover_from_journal().unwrap();
        assert_eq!(poisoned, 1);
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(
            state_of(&coord, ProcessKey::new(10_000, 100)),
            Some(CandidateState::Failed),
            "recovered committed identity stays Failed"
        );
        assert_eq!(
            *h.spawner.spawn_count.borrow(),
            0,
            "no reinjection of a possibly-hooked live process"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn gate10_recovery_blocks_all_committed_and_ignores_uncommitted() {
        let path = journal_path("g10b");
        let committed = ProcessKey::new(11_000, 110);
        let done = ProcessKey::new(11_001, 111);
        let precommit = ProcessKey::new(11_002, 112);
        {
            let mut j = Journal::open(&path).unwrap();
            // Completed identity.
            j.append(JournalRecord::transition(
                0,
                1,
                "old",
                done,
                None,
                CandidateState::Complete,
                ReasonCode::ReplayComplete,
            ))
            .unwrap();
            // Pre-commit nonterminal (WaitingForWindow) — safe to re-detect.
            j.append(JournalRecord::transition(
                0,
                2,
                "old",
                precommit,
                None,
                CandidateState::WaitingForWindow,
                ReasonCode::NoVisibleWindow,
            ))
            .unwrap();
            // Committed nonterminal.
            j.append(JournalRecord::transition(
                0,
                3,
                "old",
                committed,
                None,
                CandidateState::Recording,
                ReasonCode::RecordingStarted,
            ))
            .unwrap();
        }
        let h = Harness {
            clock: FakeClock::default(),
            probe: FakeProbe::default(),
            events: FakeEvents::healthy(),
            env: FakeEnv::default(),
            stager: FakeStager::default(),
            spawner: FakeSpawner::new(full_success_script()),
            journal_path: path.clone(),
        };
        let mut coord = build(&h);
        let poisoned = coord.recover_from_journal().unwrap();
        assert_eq!(poisoned, 2, "all committed identities stay blocked");
        assert_eq!(state_of(&coord, committed), Some(CandidateState::Failed));
        assert_eq!(state_of(&coord, done), Some(CandidateState::Complete));
        assert!(
            coord.registry().get(&precommit).is_none(),
            "uncommitted not seeded, re-detected fresh"
        );
        let _ = std::fs::remove_file(&path);
    }

    // ---- Gate 11 (offline slice): process exit finalization ----------

    #[test]
    fn gate11_recording_process_exit_finalizes_prefix() {
        // Worker recording; the script ENDS at Recording so the worker holds
        // there (it never self-finalizes). The game then exits and the
        // coordinator must finalize the prefix on the process-gone signal.
        let h = harness(
            "g11_exit",
            vec![
                WorkerStatus::Handshaking,
                WorkerStatus::Attached,
                WorkerStatus::Recording {
                    last_confirmed_hour: Some(1936_010112),
                },
            ],
        );
        h.probe.add(hoi4_identity(12_000, 120));
        let mut coord = build(&h);
        // Drive to Recording (7 transitions from detection; the worker holds
        // Recording thereafter).
        run_steps(&mut coord, &h.clock, 10);
        assert_eq!(
            state_of(&coord, ProcessKey::new(12_000, 120)),
            Some(CandidateState::Recording)
        );
        // Game exits: removed from the live set AND a WMI stop hint forces a
        // prompt reconcile (so we don't wait for the 30 s healthy cadence).
        h.probe.remove_live(12_000);
        h.events.push(RawProcessEvent::Stop {
            pid: 12_000,
            name: "hoi4.exe".into(),
        });
        run_steps(&mut coord, &h.clock, 4);
        let s = state_of(&coord, ProcessKey::new(12_000, 120)).unwrap();
        assert!(
            matches!(s, CandidateState::Finalizing | CandidateState::Complete),
            "prefix finalized after process exit, got {s:?}"
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn journal_is_written_for_lifecycle() {
        let h = harness("journal", full_success_script());
        h.probe.add(hoi4_identity(13_000, 130));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 12);
        let recs = read_all(&h.journal_path).unwrap();
        assert!(recs.iter().any(|r| r.state_to == CandidateState::Detected));
        assert!(recs.iter().any(|r| r.state_to == CandidateState::Injecting));
        assert!(recs.iter().any(|r| r.state_to == CandidateState::Recording));
        assert!(recs.iter().any(|r| r.state_to == CandidateState::Complete));
        // No raw secrets in the journal file.
        let text = std::fs::read_to_string(&h.journal_path).unwrap();
        assert!(!text.contains("\"user_sid\":"));
        assert!(!text.contains("\"pipe_token\":"));
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn durable_intent_is_synced_before_worker_spawn() {
        let h = harness("intent_before_spawn", full_success_script());
        h.probe.add(hoi4_identity(14_000, 140));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 8);
        assert_eq!(
            *h.spawner.durable_intent_seen_at_spawn.borrow(),
            Some(true),
            "spawn must observe the synced Injecting intent and exact correlation fields"
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn spawn_failure_after_intent_stays_poisoned_and_recovers_as_committed() {
        let h = harness("spawn_failure_poisoned", full_success_script());
        h.probe.add(hoi4_identity(14_001, 141));
        *h.spawner.fail_spawn.borrow_mut() = true;
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 8);
        let records = read_all(&h.journal_path).unwrap();
        let states: Vec<_> = records.iter().map(|record| record.state_to).collect();
        let injecting = states
            .iter()
            .position(|state| *state == CandidateState::Injecting);
        let failed = states
            .iter()
            .position(|state| *state == CandidateState::Failed);
        assert!(
            injecting.is_some() && injecting < failed,
            "states={states:?}"
        );
        drop(coord);

        let restarted = harness("unused", full_success_script());
        let config = CoordinatorConfig {
            coordinator_instance_id: "restart".into(),
            out_root: PathBuf::from("out"),
            own_session_id: OWN_SESSION,
            own_sid_hash: OWN_SID.to_string(),
        };
        let deps = CoordinatorDeps {
            clock: Box::new(restarted.clock.clone()),
            probe: Box::new(restarted.probe.clone()),
            events: Box::new(restarted.events.clone()),
            env: Box::new(restarted.env.clone()),
            stager: Box::new(restarted.stager.clone()),
            spawner: Box::new(restarted.spawner.clone()),
        };
        let mut recovered = Coordinator::new(config, deps, h.journal_path.clone()).unwrap();
        restarted.probe.add(hoi4_identity(14_001, 141));
        assert_eq!(recovered.recover_from_journal().unwrap(), 1);
        assert_eq!(
            state_of(&recovered, ProcessKey::new(14_001, 141)),
            Some(CandidateState::Failed),
            "terminal failed intent must be seeded so detection cannot retry"
        );
        run_steps(&mut recovered, &restarted.clock, 8);
        assert_eq!(*restarted.spawner.spawn_count.borrow(), 0);
        let _ = std::fs::remove_file(&h.journal_path);
        let _ = std::fs::remove_file(&restarted.journal_path);
    }

    #[test]
    fn pid_reuse_between_staging_and_spawn_fails_before_intent() {
        let h = harness("pid_reuse_pre_spawn", full_success_script());
        let old = hoi4_identity(14_002, 142);
        let replacement = hoi4_identity(14_002, 999_142);
        h.probe.add(old.clone());
        h.probe.script_identities(14_002, vec![old, replacement]);
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 8);
        assert_eq!(
            state_of(&coord, ProcessKey::new(14_002, 142)),
            Some(CandidateState::Failed)
        );
        assert_eq!(*h.spawner.spawn_count.borrow(), 0);
        assert!(read_all(&h.journal_path)
            .unwrap()
            .iter()
            .all(|record| record.state_to != CandidateState::Injecting));
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn transient_identity_failure_does_not_infer_exit() {
        let h = harness("identity_unresolved_live", full_success_script());
        *h.env.window_ready.borrow_mut() = false;
        h.probe.add(hoi4_identity(14_003, 143));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 3);
        h.probe.fail_identify.borrow_mut().insert(14_003);
        h.events.push(RawProcessEvent::Stop {
            pid: 14_003,
            name: "hoi4.exe".into(),
        });
        h.clock.set(100);
        coord.step().unwrap();
        assert_eq!(
            state_of(&coord, ProcessKey::new(14_003, 143)),
            Some(CandidateState::WaitingForWindow)
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn incomplete_snapshot_cannot_emit_process_exited() {
        let h = harness("incomplete_snapshot", full_success_script());
        *h.env.window_ready.borrow_mut() = false;
        h.probe.add(hoi4_identity(14_004, 144));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 3);
        h.probe.remove_live(14_004);
        *h.probe.snapshot_complete.borrow_mut() = false;
        h.events.push(RawProcessEvent::Stop {
            pid: 14_004,
            name: "hoi4.exe".into(),
        });
        h.clock.set(100);
        coord.step().unwrap();
        assert_eq!(
            state_of(&coord, ProcessKey::new(14_004, 144)),
            Some(CandidateState::WaitingForWindow)
        );
        *h.probe.snapshot_complete.borrow_mut() = true;
        h.events.push(RawProcessEvent::Stop {
            pid: 14_004,
            name: "hoi4.exe".into(),
        });
        h.clock.set(110);
        coord.step().unwrap();
        assert_eq!(
            state_of(&coord, ProcessKey::new(14_004, 144)),
            Some(CandidateState::ProcessExitedBeforeAttach)
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn coordinator_token_session_and_journal_hash_are_same_attempt() {
        let h = harness("correlation_exact", full_success_script());
        h.probe.add(hoi4_identity(14_005, 145));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 8);
        let specs = h.spawner.spawned_specs.borrow();
        let spec = specs.first().expect("spawned spec");
        let mut hash = snapshot::sha256::Sha256::new();
        hash.update(spec.payload.pipe_token.as_bytes());
        assert_eq!(
            snapshot::sha256::to_hex(&hash.finalize()),
            spec.payload.pipe_token_hash
        );
        let intent = read_all(&h.journal_path)
            .unwrap()
            .into_iter()
            .find(|record| record.state_to == CandidateState::Injecting)
            .unwrap();
        assert_eq!(intent.session_id.as_deref(), Some(spec.session_id.as_str()));
        assert_eq!(
            intent.pipe_token_hash.as_deref(),
            Some(spec.payload.pipe_token_hash.as_str())
        );
        let journal_text = std::fs::read_to_string(&h.journal_path).unwrap();
        assert!(!journal_text.contains(&spec.payload.pipe_token));
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn complete_before_proof_fails_terminal_instead_of_sticking() {
        let h = harness(
            "complete_before_proof",
            vec![WorkerStatus::Complete {
                replay_path: "out/unproven.battleplan".into(),
            }],
        );
        h.probe.add(hoi4_identity(14_006, 146));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 8);
        assert_eq!(
            state_of(&coord, ProcessKey::new(14_006, 146)),
            Some(CandidateState::Failed)
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn live_worker_failure_is_stopped_before_handle_is_removed() {
        let h = harness(
            "worker_failure_stopped",
            vec![WorkerStatus::Failed {
                reason: "proof channel failed".into(),
            }],
        );
        h.probe.add(hoi4_identity(14_007, 147));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 8);
        assert_eq!(
            *h.spawner.stop_count.borrow(),
            1,
            "a proof failure must terminate the already-spawned injector worker"
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn coordinator_drop_after_error_stops_and_waits_live_worker() {
        let h = harness("coordinator_error_cleanup", vec![WorkerStatus::Handshaking]);
        h.probe.add(hoi4_identity(14_008, 148));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 8);
        assert_eq!(*h.spawner.spawn_count.borrow(), 1, "worker must be live");
        assert_eq!(*h.spawner.drop_count.borrow(), 0, "worker still supervised");

        // Force the actual `coord.step()?` propagation path used by
        // `run_supervisor`: a stop hint requests an immediate authoritative
        // snapshot, and that snapshot fails while the worker is still live.
        h.events.push(RawProcessEvent::Stop {
            pid: 14_008,
            name: "hoi4.exe".to_string(),
        });
        h.probe
            .fail_next_snapshot("forced coordinator snapshot failure");
        let error = coord.step().expect_err("coordinator step must fail");
        assert!(
            error.contains("forced coordinator snapshot failure"),
            "{error}"
        );

        // `run_supervisor` now returns via `?`, dropping the coordinator. Its
        // Drop requests stop; the concrete ChildWorkerHandle Drop synchronously
        // waits/reaps (covered by the live subprocess test in auto_attach.rs).
        drop(coord);
        assert_eq!(*h.spawner.stop_count.borrow(), 1, "worker was not stopped");
        assert_eq!(
            *h.spawner.drop_count.borrow(),
            1,
            "worker was not waited/dropped"
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }
}
