use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

use super::identity::{name_is_hoi4, ProcessIdentity, ProcessKey};
#[cfg(test)]
use super::journal::read_all;
use super::journal::{recover, Journal, JournalRecord};
use super::policy;
use super::reducer::{CandidateRegistry, ObserveOutcome};
use super::sources::{
    AttachEnvironment, Clock, CollectorModuleResidency, GameBuildAdmission, PayloadStager,
    ProcessEventSource, ProcessProbe, RawProcessEvent, WorkerHandle, WorkerSpawner, WorkerSpec,
    WorkerStatus,
};
use super::staging::{generate_attempt_id, generate_pipe_token, generate_secret_128};
use super::state::{CandidateState, ReasonCode};

pub struct CoordinatorConfig {
    pub coordinator_instance_id: String,
    pub out_root: PathBuf,
    pub own_session_id: u32,
    pub own_sid_hash: String,
    pub require_visible_window: bool,
}

pub struct CoordinatorDeps {
    pub clock: Box<dyn Clock>,
    pub probe: Box<dyn ProcessProbe>,
    pub events: Box<dyn ProcessEventSource>,
    pub env: Box<dyn AttachEnvironment>,
    pub stager: Box<dyn PayloadStager>,
    pub spawner: Box<dyn WorkerSpawner>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SupervisorEvent {
    RecordingStarted {
        pid: u32,
        run_dir: PathBuf,
    },
    RecordingSucceeded {
        pid: u32,
        replay_path: PathBuf,
    },
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
    recovered_rearm: BTreeMap<ProcessKey, PathBuf>,
    last_reconcile_ms: Option<u64>,
    force_reconcile: bool,
    started: bool,
    supervisor_events: Vec<SupervisorEvent>,
    readiness_diagnostics: BTreeMap<ProcessKey, String>,
    build_admitted: BTreeSet<ProcessKey>,
    unsupported_game_build: Option<String>,
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        // `run_supervisor` propagates coordinator errors with `?`. Ownership is
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
            recovered_rearm: BTreeMap::new(),
            last_reconcile_ms: None,
            force_reconcile: false,
            started: false,
            supervisor_events: Vec::new(),
            readiness_diagnostics: BTreeMap::new(),
            build_admitted: BTreeSet::new(),
            unsupported_game_build: None,
        })
    }

    pub fn registry(&self) -> &CandidateRegistry {
        &self.registry
    }

    pub fn drain_supervisor_events(&mut self) -> Vec<SupervisorEvent> {
        std::mem::take(&mut self.supervisor_events)
    }

    pub fn take_unsupported_game_build(&mut self) -> Option<String> {
        self.unsupported_game_build.take()
    }

    /// Transfer every active candidate into explicit-stop ownership. Recording
    pub fn begin_explicit_stop(&mut self) -> Result<(), String> {
        let now = self.deps.clock.now_ms();
        let keys = self.registry.active_keys();
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
                        Some("explicit app stop; draining and finalizing capture".into()),
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

    pub fn drive_shutdown(&mut self) -> Result<bool, String> {
        let now = self.deps.clock.now_ms();
        let keys: Vec<_> = self.workers.keys().copied().collect();
        for key in keys {
            self.drive_worker(key, now)?;
        }
        Ok(self.workers.is_empty())
    }

    pub fn recover_from_journal(&mut self) -> Result<usize, String> {
        // release authority between validation and the recovery action.
        let records = self.journal.recovered_records().to_vec();
        let recovered = recover(&records);
        let now = self.deps.clock.now_ms();
        let mut poisoned = 0;
        for r in recovered {
            if r.injection_committed {
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
                if r.last_state.is_terminal() {
                    if let Some(staging_dir) = r.staging_dir {
                        self.recovered_rearm
                            .insert(r.key, staging_dir.join("collector.dll"));
                    }
                }
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
        }
        Ok(poisoned)
    }

    pub fn step(&mut self) -> Result<(), String> {
        let now = self.deps.clock.now_ms();
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

    fn ingest_snapshot(&mut self, now: u64) -> Result<(), String> {
        let snapshot = self.deps.probe.snapshot()?;
        let mut live: BTreeSet<ProcessKey> = BTreeSet::new();
        let mut unresolved_pids: BTreeSet<u32> = BTreeSet::new();
        let mut new_keys: Vec<ProcessKey> = Vec::new();
        for e in snapshot.entries {
            if !name_is_hoi4(&e.name) {
                continue;
            }
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
            if self.try_rearm_recovered(id.clone(), now)? {
                continue;
            }
            if self.registry.observe_present(id, now) == ObserveOutcome::NewCandidate {
                new_keys.push(key);
            }
        }
        for key in new_keys {
            self.journal_transition(
                key,
                None,
                CandidateState::Detected,
                ReasonCode::Detected,
                None,
            )?;
        }
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
                let readiness = self.registry.get(&key).unwrap();
                if let Some(last) = readiness.last_readiness_probe_ms {
                    let delay =
                        policy::readiness_delay_ms(readiness.readiness_attempts.saturating_sub(1));
                    if now.saturating_sub(last) < delay {
                        return Ok(());
                    }
                }
                let id = self.registry.get(&key).unwrap().identity.clone();
                if !self.build_admitted.contains(&key) {
                    if let GameBuildAdmission::Unsupported(refusal) =
                        self.deps.env.game_build_admission(&id)
                    {
                        let message = refusal.message();
                        let _ = crate::game_build::write_refusal_receipt(
                            &self.config.out_root,
                            refusal.as_ref(),
                        );
                        self.unsupported_game_build
                            .get_or_insert_with(|| message.clone());
                        crate::game_build::arm_refusal_exit_code();
                        self.fail(key, ReasonCode::UnsupportedGameBuild, now, Some(message))?;
                        return Ok(());
                    }
                    self.build_admitted.insert(key);
                }
                let modset = self.deps.env.resolve_modset(&id);
                if modset.resolved {
                    self.readiness_diagnostics.remove(&key);
                    if self.config.require_visible_window {
                        self.transition(
                            key,
                            CandidateState::WaitingForWindow,
                            ReasonCode::NoVisibleWindow,
                            now,
                            None,
                        )?;
                    } else {
                        self.transition(
                            key,
                            CandidateState::Preparing,
                            ReasonCode::Preparing,
                            now,
                            Some(
                                "headless product admission: exact same-user process identity; visible-window gate disabled"
                                    .into(),
                            ),
                        )?;
                    }
                } else {
                    let mut detail = modset.reasons.join("; ");
                    if detail.len() > 2048 {
                        let mut boundary = 2048;
                        while !detail.is_char_boundary(boundary) {
                            boundary -= 1;
                        }
                        detail.truncate(boundary);
                    }
                    if self.readiness_diagnostics.get(&key) != Some(&detail) {
                        self.journal_transition(
                            key,
                            Some(CandidateState::WaitingForIdentity),
                            CandidateState::WaitingForIdentity,
                            ReasonCode::IdentityUnresolved,
                            Some(detail.clone()),
                        )?;
                        self.readiness_diagnostics.insert(key, detail);
                    }
                    self.registry.bump_readiness(&key, now);
                }
            }
            CandidateState::WaitingForWindow => {
                let readiness = self.registry.get(&key).unwrap();
                if let Some(last) = readiness.last_readiness_probe_ms {
                    let delay =
                        policy::readiness_delay_ms(readiness.readiness_attempts.saturating_sub(1));
                    if now.saturating_sub(last) < delay {
                        return Ok(());
                    }
                }
                if self.deps.env.has_visible_window(key) {
                    self.transition(
                        key,
                        CandidateState::Preparing,
                        ReasonCode::Preparing,
                        now,
                        None,
                    )?;
                } else {
                    self.registry.bump_readiness(&key, now);
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
        if !self.registry.may_start_injection(&key) {
            self.fail(
                key,
                ReasonCode::IdentityMismatchAtInject,
                now,
                Some("no-double-inject guard refused".into()),
            )?;
            return Ok(());
        }
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
            || (self.config.require_visible_window && !self.deps.env.has_visible_window(key))
        {
            self.fail(
                key,
                ReasonCode::IdentityMismatchAtInject,
                now,
                Some(if self.config.require_visible_window {
                    "pre-injection identity/window-owner comparison failed".into()
                } else {
                    "pre-injection exact process identity comparison failed".into()
                }),
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

    fn drive_worker(&mut self, key: ProcessKey, now: u64) -> Result<(), String> {
        let (state, entered, gone) = {
            let c = self.registry.get(&key).unwrap();
            (c.state, c.state_entered_ms, c.process_gone_after_commit)
        };

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
            self.fail(
                key,
                ReasonCode::ProcessExited,
                now,
                Some("process exited before first valid frame".into()),
            )?;
            return Ok(());
        }

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
            (CandidateState::Injecting, WorkerStatus::Handshaking) => {
                self.transition(
                    key,
                    CandidateState::Handshaking,
                    ReasonCode::Injected,
                    now,
                    None,
                )?;
            }
            (CandidateState::Injecting, WorkerStatus::Attached) => {
                self.transition(
                    key,
                    CandidateState::Handshaking,
                    ReasonCode::Injected,
                    now,
                    None,
                )?;
                self.transition(
                    key,
                    CandidateState::WaitingForWorld,
                    ReasonCode::CollectorAttached,
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
            (_, WorkerStatus::UnsupportedGameBuild { message }) => {
                if let Some(worker) = self.workers.get_mut(&key) {
                    worker.request_stop();
                }
                self.unsupported_game_build
                    .get_or_insert_with(|| message.clone());
                crate::game_build::arm_refusal_exit_code();
                self.fail(key, ReasonCode::UnsupportedGameBuild, now, Some(message))?;
            }
            (_, WorkerStatus::Failed { reason }) => {
                if let Some(worker) = self.workers.get_mut(&key) {
                    worker.request_stop();
                }
                self.fail(key, ReasonCode::FinalizeVerifyFailed, now, Some(reason))?;
            }
            _ => {}
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

    fn try_rearm_recovered(&mut self, identity: ProcessIdentity, now: u64) -> Result<bool, String> {
        let key = identity.key;
        let Some(dll_path) = self.recovered_rearm.get(&key).cloned() else {
            return Ok(false);
        };
        let terminal = self
            .registry
            .get(&key)
            .is_some_and(|candidate| candidate.state.is_terminal());
        if !terminal || self.workers.contains_key(&key) {
            return Ok(false);
        }
        match self
            .deps
            .probe
            .collector_module_residency(key.pid, &dll_path)
        {
            Ok(CollectorModuleResidency::Loaded) | Err(_) => Ok(false),
            Ok(CollectorModuleResidency::Absent) => {
                self.deps.stager.rearm_after_proven_unload(key, &dll_path)?;
                let previous = self.registry.rearm_after_proven_unload(identity, now)?;
                self.recovered_rearm.remove(&key);
                self.journal_transition(
                    key,
                    Some(previous),
                    CandidateState::Detected,
                    ReasonCode::CollectorUnloadedRearmed,
                    Some(format!(
                        "prior collector module is absent; rearming same process identity from {}",
                        dll_path.display()
                    )),
                )?;
                Ok(true)
            }
        }
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
            let event = SupervisorEvent::RecordingFailed {
                pid: key.pid,
                reason,
                detail: detail.clone(),
                run_dir: self.run_dir_for(key),
            };
            let host_sentence_first = reason == ReasonCode::UnsupportedGameBuild;
            if host_sentence_first {
                self.supervisor_events.push(event.clone());
            }
            self.journal_transition(key, Some(from), CandidateState::Failed, reason, detail)?;
            if !host_sentence_first {
                self.supervisor_events.push(event);
            }
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
    unsafe impl Send for FakeClock {}

    #[derive(Clone)]
    struct FakeProbe {
        table: Rc<RefCell<BTreeMap<u32, ProcessIdentity>>>,
        live: Rc<RefCell<Vec<u32>>>,
        fail_identify: Rc<RefCell<BTreeSet<u32>>>,
        snapshot_error: Rc<RefCell<Option<String>>>,
        snapshot_complete: Rc<RefCell<bool>>,
        identify_script: Rc<RefCell<BTreeMap<u32, VecDeque<ProcessIdentity>>>>,
        module_residency: Rc<RefCell<BTreeMap<(u32, String), CollectorModuleResidency>>>,
        module_probe_error: Rc<RefCell<Option<String>>>,
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
                module_residency: Rc::new(RefCell::new(BTreeMap::new())),
                module_probe_error: Rc::new(RefCell::new(None)),
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
        fn set_module_loaded(&self, pid: u32, path: &Path, loaded: bool) {
            let key = (pid, path.to_string_lossy().to_ascii_lowercase());
            self.module_residency.borrow_mut().insert(
                key,
                if loaded {
                    CollectorModuleResidency::Loaded
                } else {
                    CollectorModuleResidency::Absent
                },
            );
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
        fn collector_module_residency(
            &self,
            pid: u32,
            dll_path: &Path,
        ) -> Result<CollectorModuleResidency, String> {
            if let Some(error) = self.module_probe_error.borrow_mut().take() {
                return Err(error);
            }
            let key = (pid, dll_path.to_string_lossy().to_ascii_lowercase());
            self.module_residency
                .borrow()
                .get(&key)
                .copied()
                .ok_or_else(|| "forced ambiguous module residency".to_string())
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
        unsupported_build: Rc<RefCell<Option<crate::game_build::ObservedGameBuild>>>,
        build_queries: Rc<RefCell<usize>>,
    }
    impl Default for FakeEnv {
        fn default() -> Self {
            Self {
                window_ready: Rc::new(RefCell::new(true)),
                modset_resolved: Rc::new(RefCell::new(true)),
                unsupported_build: Rc::new(RefCell::new(None)),
                build_queries: Rc::new(RefCell::new(0)),
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
        fn game_build_admission(&self, _id: &ProcessIdentity) -> GameBuildAdmission {
            *self.build_queries.borrow_mut() += 1;
            match self.unsupported_build.borrow().clone() {
                None => GameBuildAdmission::Supported,
                Some(observed) => GameBuildAdmission::Unsupported(Box::new(
                    crate::game_build::UnsupportedGameBuild { observed },
                )),
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
        fn rearm_after_proven_unload(
            &self,
            _key: ProcessKey,
            _prior_dll_path: &Path,
        ) -> Result<(), String> {
            Ok(())
        }

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
                rearmed_after_proven_unload: false,
            })
        }
    }
    unsafe impl Send for FakeStager {}

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

    fn build_with_window_requirement(h: &Harness, require_visible_window: bool) -> Coordinator {
        build_with_window_requirement_and_out_root(h, require_visible_window, PathBuf::from("out"))
    }

    fn build_with_window_requirement_and_out_root(
        h: &Harness,
        require_visible_window: bool,
        out_root: PathBuf,
    ) -> Coordinator {
        *h.spawner.journal_path.borrow_mut() = Some(h.journal_path.clone());
        let config = CoordinatorConfig {
            coordinator_instance_id: "coord-test".into(),
            out_root,
            own_session_id: OWN_SESSION,
            own_sid_hash: OWN_SID.to_string(),
            require_visible_window,
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

    fn build(h: &Harness) -> Coordinator {
        build_with_window_requirement(h, true)
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
        let start = clock.now_ms();
        for i in 0..n {
            clock.set(start + (i as u64 + 1) * 10);
            coord.step().unwrap();
        }
    }

    fn state_of(coord: &Coordinator, key: ProcessKey) -> Option<CandidateState> {
        coord.registry().get(&key).map(|c| c.state)
    }

    fn unknown_build() -> crate::game_build::ObservedGameBuild {
        crate::game_build::ObservedGameBuild {
            display_version: "Operation Bombers v1.20.0.b100 (d300)".to_string(),
            version_source: crate::game_build::VersionSource::LauncherSettingsVersion,
            exe_sha256: Some([0x5a; 32]),
            exe_read_error: None,
            exe_path: PathBuf::from(r"D:\Steam\common\the game\hoi4.exe"),
        }
    }

    fn patch_day_out_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "mk_patch_day_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn an_unknown_game_build_is_refused_before_anything_is_staged_or_injected() {
        let h = harness("patch_day_refuse", full_success_script());
        *h.env.unsupported_build.borrow_mut() = Some(unknown_build());
        let out_root = patch_day_out_root("refuse");
        h.probe.add(hoi4_identity(6100, 909));
        let mut coord = build_with_window_requirement_and_out_root(&h, true, out_root.clone());
        run_steps(&mut coord, &h.clock, 12);

        assert_eq!(
            state_of(&coord, ProcessKey::new(6100, 909)),
            Some(CandidateState::Failed),
            "an unknown build is terminal, not a holding state"
        );
        assert_eq!(
            *h.spawner.spawn_count.borrow(),
            0,
            "no worker may be spawned: the worker is what injects"
        );
        assert!(
            h.stager.staged.borrow().is_empty(),
            "nothing may be staged before the build is admitted: {:?}",
            h.stager.staged.borrow()
        );

        let events = coord.drain_supervisor_events();
        let expected = crate::game_build::UnsupportedGameBuild {
            observed: unknown_build(),
        }
        .message();
        assert!(
            matches!(
                events.as_slice(),
                [SupervisorEvent::RecordingFailed { pid: 6100, reason, detail, .. }]
                    if *reason == ReasonCode::UnsupportedGameBuild
                        && detail.as_deref() == Some(expected.as_str())
            ),
            "expected exactly the unsupported-build message, got {events:?}"
        );

        let records = read_all(&h.journal_path).unwrap();
        let terminal = records
            .iter()
            .find(|record| record.state_to == CandidateState::Failed)
            .expect("a terminal journal record");
        assert_eq!(terminal.reason_code, ReasonCode::UnsupportedGameBuild);
        assert_eq!(terminal.reason_detail.as_deref(), Some(expected.as_str()));

        let receipt = out_root.join(crate::game_build::UNSUPPORTED_GAME_BUILD_RECEIPT_FILE);
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&receipt).expect("receipt written"))
                .expect("receipt parses");
        assert_eq!(value["message"], expected);
        assert_eq!(
            value["terminal_outcome"],
            crate::game_build::UnsupportedGameBuild::TERMINAL_OUTCOME
        );
        assert_eq!(value["recording_started"], false);
        assert_eq!(value["game_modified"], false);

        let taken = coord.take_unsupported_game_build();
        assert_eq!(
            taken.as_deref(),
            Some(expected.as_str()),
            "the supervisor must be handed the same sentence to stop on"
        );
        assert!(
            coord.take_unsupported_game_build().is_none(),
            "the supervisor-terminal fact is handed over once"
        );

        let asked_once = *h.env.build_queries.borrow();
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(*h.env.build_queries.borrow(), asked_once);
        assert_eq!(*h.spawner.spawn_count.borrow(), 0);
        assert!(coord.drain_supervisor_events().is_empty());

        let _ = std::fs::remove_dir_all(&out_root);
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn a_known_game_build_takes_the_ordinary_path() {
        let h = harness("patch_day_known", full_success_script());
        h.probe.add(hoi4_identity(6200, 910));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(
            state_of(&coord, ProcessKey::new(6200, 910)),
            Some(CandidateState::Complete)
        );
        assert_eq!(*h.spawner.spawn_count.borrow(), 1);
        assert_eq!(
            *h.env.build_queries.borrow(),
            1,
            "the supported build is asked about exactly once"
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn a_workers_unsupported_build_refusal_is_not_a_finalize_failure() {
        let message = crate::game_build::UnsupportedGameBuild {
            observed: unknown_build(),
        }
        .message();
        let h = harness(
            "patch_day_worker",
            vec![WorkerStatus::UnsupportedGameBuild {
                message: message.clone(),
            }],
        );
        h.probe.add(hoi4_identity(6500, 913));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 12);

        assert_eq!(
            state_of(&coord, ProcessKey::new(6500, 913)),
            Some(CandidateState::Failed)
        );
        let events = coord.drain_supervisor_events();
        assert!(
            matches!(
                events.as_slice(),
                [
                    SupervisorEvent::RecordingStarted { .. },
                    SupervisorEvent::RecordingFailed { reason, detail, .. },
                ] | [SupervisorEvent::RecordingFailed { reason, detail, .. }]
                    if *reason == ReasonCode::UnsupportedGameBuild
                        && detail.as_deref() == Some(message.as_str())
            ),
            "the worker's own sentence must survive whole, under its own reason \
             code, got {events:?}"
        );
        assert_eq!(
            coord.take_unsupported_game_build().as_deref(),
            Some(message.as_str())
        );
        let records = read_all(&h.journal_path).unwrap();
        assert!(
            records
                .iter()
                .all(|record| record.reason_code != ReasonCode::FinalizeVerifyFailed),
            "nothing was recorded, so nothing failed to finalize: {records:?}"
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn a_held_candidate_is_not_re_asked_about_its_build() {
        let h = harness("patch_day_held", full_success_script());
        *h.env.modset_resolved.borrow_mut() = false;
        h.probe.add(hoi4_identity(6400, 912));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 40);
        assert_eq!(
            state_of(&coord, ProcessKey::new(6400, 912)),
            Some(CandidateState::WaitingForIdentity),
            "an unresolved modset holds the candidate across many probes"
        );
        assert_eq!(
            *h.env.build_queries.borrow(),
            1,
            "the build was proved supported once and remembered"
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn the_build_gate_outranks_an_unresolved_modset() {
        let h = harness("patch_day_before_modset", full_success_script());
        *h.env.unsupported_build.borrow_mut() = Some(unknown_build());
        *h.env.modset_resolved.borrow_mut() = false;
        let out_root = patch_day_out_root("before_modset");
        h.probe.add(hoi4_identity(6300, 911));
        let mut coord = build_with_window_requirement_and_out_root(&h, true, out_root.clone());
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(
            state_of(&coord, ProcessKey::new(6300, 911)),
            Some(CandidateState::Failed)
        );
        let events = coord.drain_supervisor_events();
        assert!(
            matches!(
                events.as_slice(),
                [SupervisorEvent::RecordingFailed { reason, .. }]
                    if *reason == ReasonCode::UnsupportedGameBuild
            ),
            "the version sentence outranks a modset diagnostic, got {events:?}"
        );
        let _ = std::fs::remove_dir_all(&out_root);
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn gate1_game_first_startup_snapshot_attaches() {
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
        let h = harness("g1_appfirst", full_success_script());
        let mut coord = build(&h);
        h.clock.set(10);
        coord.step().unwrap();
        assert!(coord.registry().is_empty());
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

    #[test]
    fn gate2_snapshot_and_wmi_produce_one_attempt() {
        let h = harness("g2_race", full_success_script());
        h.probe.add(hoi4_identity(3000, 333));
        let mut coord = build(&h);
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

    #[test]
    fn gate4_reconcile_cadence_tracks_health() {
        let h = harness("g4_health", full_success_script());
        let mut coord = build(&h);
        h.clock.set(10);
        coord.step().unwrap();
        assert!(!coord.reconcile_due(20_000));
        assert!(coord.reconcile_due(40_000), "due after 30s healthy");
        h.events.set_health(WatcherHealth::Degraded);
        assert!(coord.reconcile_due(10 + 2_000));
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn gate4_degraded_still_discovers_game_first() {
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

    #[test]
    fn gate5_wrong_session_never_admitted() {
        let h = harness("g5_session", full_success_script());
        let mut bad = hoi4_identity(5000, 55);
        bad.session_id = OWN_SESSION + 1;
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
        *h.env.window_ready.borrow_mut() = true;
        h.clock
            .set(h.clock.now_ms() + policy::readiness_delay_ms(0));
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(
            state_of(&coord, ProcessKey::new(5300, 58)),
            Some(CandidateState::Complete)
        );
        let _ = std::fs::remove_file(&h.journal_path);
    }

    #[test]
    fn headless_product_injects_before_any_visible_window() {
        let h = harness("headless_pre_window", full_success_script());
        *h.env.window_ready.borrow_mut() = false;
        h.probe.add(hoi4_identity(5350, 585));
        let mut coord = build_with_window_requirement(&h, false);
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(
            state_of(&coord, ProcessKey::new(5350, 585)),
            Some(CandidateState::Complete)
        );
        assert_eq!(*h.spawner.spawn_count.borrow(), 1);
        let records = read_all(&h.journal_path).unwrap();
        assert!(
            records
                .iter()
                .all(|record| record.state_to != CandidateState::WaitingForWindow),
            "headless product must not wait for a window: {records:?}"
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

    #[test]
    fn gate6_no_heartbeat_never_reaches_recording() {
        let h = harness("g6_hb", vec![WorkerStatus::Handshaking]);
        h.probe.add(hoi4_identity(6000, 66));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 12);
        let s = state_of(&coord, ProcessKey::new(6000, 66)).unwrap();
        assert!(matches!(
            s,
            CandidateState::Handshaking | CandidateState::Injecting
        ));
        assert_eq!(*h.spawner.spawn_count.borrow(), 1);
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
    fn gate6_attached_before_first_poll_preserves_the_waiting_for_world_proof() {
        let h = harness("g6_fast_attached", vec![WorkerStatus::Attached]);
        h.probe.add(hoi4_identity(6001, 67));
        let mut coord = build(&h);
        run_steps(&mut coord, &h.clock, 8);
        assert_eq!(
            state_of(&coord, ProcessKey::new(6001, 67)),
            Some(CandidateState::WaitingForWorld)
        );
        assert_eq!(*h.spawner.spawn_count.borrow(), 1);
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

    #[test]
    fn gate10_recovery_poisons_committed_identity() {
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

    #[test]
    fn recovered_terminal_identity_rearms_only_after_exact_module_unload() {
        let path = journal_path("rearm_after_unload");
        let key = ProcessKey::new(11_100, 210);
        let old_staging = PathBuf::from(r"C:\stage\old-attempt");
        let old_dll = old_staging.join("collector.dll");
        {
            let mut journal = Journal::open(&path).unwrap();
            let mut injecting = JournalRecord::transition(
                0,
                1,
                "old",
                key,
                None,
                CandidateState::Injecting,
                ReasonCode::Injected,
            );
            injecting.exe_path = Some(r"D:\Steam\common\the game\hoi4.exe".to_string());
            injecting.staging_dir = Some(old_staging.display().to_string());
            journal.append(injecting).unwrap();
            journal
                .append(JournalRecord::transition(
                    0,
                    2,
                    "old",
                    key,
                    Some(CandidateState::Finalizing),
                    CandidateState::Failed,
                    ReasonCode::FinalizeVerifyFailed,
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
        h.probe.add(hoi4_identity(key.pid, key.creation_filetime.0));
        h.probe.set_module_loaded(key.pid, &old_dll, true);
        let mut coord = build(&h);
        coord.recover_from_journal().unwrap();

        h.clock.set(1);
        coord.step().unwrap();
        assert_eq!(state_of(&coord, key), Some(CandidateState::Failed));
        assert_eq!(*h.spawner.spawn_count.borrow(), 0);

        h.probe.set_module_loaded(key.pid, &old_dll, false);
        coord.force_reconcile = true;
        run_steps(&mut coord, &h.clock, 12);
        assert_eq!(*h.spawner.spawn_count.borrow(), 1);
        let records = read_all(&path).unwrap();
        assert!(records.iter().any(|record| {
            record.key() == key
                && record.reason_code == ReasonCode::CollectorUnloadedRearmed
                && record.state_to == CandidateState::Detected
        }));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn gate11_recording_process_exit_finalizes_prefix() {
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
        run_steps(&mut coord, &h.clock, 10);
        assert_eq!(
            state_of(&coord, ProcessKey::new(12_000, 120)),
            Some(CandidateState::Recording)
        );
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
            require_visible_window: true,
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
