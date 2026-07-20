//! Durable filesystem ownership for multi-logical-stream CID1 rollover.
//!
//! The data-pipe reader and H4CT control router use two handles over one state.
//! The data handle is the only writer of H4CR bytes.  The control handle is the
//! only owner allowed to drain, seal, open, and recover logical streams.  A
//! CRA1 receipt is therefore impossible until the old file is contiguous and
//! durable and the next empty file plus transaction ledger are durable.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use compact_contract::campaign_identity::ValidatedCid1;
use compact_contract::campaign_rollover::RolloverProposeV1;
use compact_contract::stream_validator::{AcceptedRecordKind, StatefulStreamValidator};
use compact_contract::{
    parse_compact_record_header, COMPACT_MAX_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::identity_control::{AcknowledgedFirstCompactRecord, ValidatedFirstCompactRecord};
use super::rollover_control::{
    DrainedLogicalStream, OpenedLogicalStream, RecorderRolloverStore,
};
use crate::control_transport::{ControlCancellation, ControlDeadline};

const STATE_FILE: &str = "rollover-state.json";
const REJECTION_DIRECTORY: &str = "rollover-rejections";
pub const SET_DESCRIPTOR_FILE: &str = "logical-stream-set.json";
pub const PACKAGE_SET_DESCRIPTOR_FILE: &str = "logical-stream-package-set.json";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableRolloverPhase {
    Streaming,
    Draining,
    AwaitingCid1,
    Complete,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableRolloverState {
    pub schema: String,
    pub phase: DurableRolloverPhase,
    pub logical_stream_id: u64,
    pub campaign_id_hex: String,
    pub cid1_payload_sha256_hex: String,
    pub last_complete_sequence: Option<u64>,
    pub last_proposal_nonce: u64,
    pub sealed_streams: Vec<DurableSealedStream>,
    #[serde(default)]
    pub transitions: Vec<DurableRolloverTransition>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableSealedStream {
    pub logical_stream_id: u64,
    pub last_complete_sequence: u64,
    pub campaign_id_hex: String,
    pub cid1_payload_sha256_hex: String,
    pub file_sha256_hex: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableRolloverTransition {
    pub current_logical_stream_id: u64,
    pub proposed_logical_stream_id: u64,
    pub last_admitted_h4cr_sequence: u64,
    pub boundary_game_hour_raw: i32,
    pub reason: String,
    pub proposal_nonce: u64,
    pub current_campaign_id_hex: String,
    pub proposed_campaign_id_hex: String,
    pub current_cid1_payload_sha256_hex: String,
    pub proposed_cid1_payload_sha256_hex: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableRolloverPublicationStream {
    pub logical_stream_id: u64,
    pub relative_path: String,
    pub last_complete_sequence: u64,
    pub campaign_id_hex: String,
    pub cid1_payload_sha256_hex: String,
    pub file_bytes: u64,
    pub file_sha256_hex: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableRolloverSetDescriptor {
    pub schema: String,
    pub ordered_streams: Vec<DurableRolloverPublicationStream>,
    pub transitions: Vec<DurableRolloverTransition>,
}

impl DurableRolloverState {
    fn new(identity: &ValidatedCid1) -> Self {
        Self {
            schema: "cid1-rollover-fs-v1".to_string(),
            phase: DurableRolloverPhase::Streaming,
            logical_stream_id: 0,
            campaign_id_hex: hex(&identity.campaign_id()),
            cid1_payload_sha256_hex: hex(&identity.payload_sha256()),
            last_complete_sequence: None,
            last_proposal_nonce: 0,
            sealed_streams: Vec::new(),
            transitions: Vec::new(),
        }
    }
}

struct Inner {
    root: PathBuf,
    state: DurableRolloverState,
    file: Option<File>,
    validator: StatefulStreamValidator,
    expected_campaign_id: [u8; 32],
    expected_cid1_payload_sha256: [u8; 32],
    pending_first_record: Option<Vec<u8>>,
    drain_target_sequence: Option<u64>,
    terminal_error: Option<String>,
}

struct Shared {
    inner: Mutex<Inner>,
    changed: Condvar,
}

/// H4CR data-pipe endpoint.  Complete records are admitted atomically; bytes
/// from two logical streams can never share a file.
#[derive(Clone)]
pub struct FilesystemRolloverDataWriter {
    shared: Arc<Shared>,
}

/// H4CT/store endpoint implementing the durable-before-CRA1 contract.
pub struct FilesystemRolloverStore {
    shared: Arc<Shared>,
}

impl FilesystemRolloverStore {
    /// Create a fresh stream set only from the move-only result of the initial
    /// durable CFA1 transaction. Arbitrary CID1 bytes cannot initialize the
    /// production storage owner.
    pub fn create(
        root: impl Into<PathBuf>,
        initial: &AcknowledgedFirstCompactRecord,
    ) -> Result<(Self, FilesystemRolloverDataWriter), String> {
        let acknowledgement = initial.first_record().acknowledgement();
        if initial.durable_receipt().acknowledgement() != acknowledgement
            || acknowledgement.logical_stream_id != 0
            || acknowledgement.mode
                != compact_contract::stream_validator::StreamIdentityMode::IdentityV1
        {
            return Err("initial rollover durable receipt is inconsistent".to_string());
        }
        Self::create_from_record(root.into(), initial.first_record().complete_record())
    }

    fn create_from_record(
        root: PathBuf,
        initial_record: &[u8],
    ) -> Result<(Self, FilesystemRolloverDataWriter), String> {
        fs::create_dir_all(&root)
            .map_err(|error| format!("create rollover root {}: {error}", root.display()))?;
        if root.join(STATE_FILE).exists() {
            return Err("rollover root already contains durable state".to_string());
        }
        let first = ValidatedFirstCompactRecord::identity_v1(0, initial_record)
            .map_err(|error| format!("initial rollover CID1 is invalid: {error}"))?;
        let identity = first
            .validated_cid1()
            .ok_or("initial rollover record retained no CID1")?
            .clone();
        let path = open_path(&root, 0);
        let mut file = create_new_file(&path)?;
        file.write_all(initial_record)
            .map_err(|error| format!("write initial rollover stream: {error}"))?;
        durable_file(&mut file, &path)?;
        let mut validator = StatefulStreamValidator::identity_v1();
        admit_complete_record(&mut validator, initial_record)?;
        let mut state = DurableRolloverState::new(&identity);
        state.last_complete_sequence = Some(0);
        persist_state(&root, &state)?;
        let shared = Arc::new(Shared {
            inner: Mutex::new(Inner {
                root,
                state,
                file: Some(file),
                validator,
                expected_campaign_id: identity.campaign_id(),
                expected_cid1_payload_sha256: identity.payload_sha256(),
                pending_first_record: None,
                drain_target_sequence: None,
                terminal_error: None,
            }),
            changed: Condvar::new(),
        });
        Ok((
            Self {
                shared: shared.clone(),
            },
            FilesystemRolloverDataWriter { shared },
        ))
    }

    #[cfg(test)]
    fn create_for_test(
        root: impl Into<PathBuf>,
        initial_record: &[u8],
    ) -> Result<(Self, FilesystemRolloverDataWriter), String> {
        Self::create_from_record(root.into(), initial_record)
    }

    /// Reopen a crash-surviving set.  Recovery accepts only a self-consistent
    /// state file and exact file topology; ambiguous partial transactions fail.
    pub fn recover(root: impl Into<PathBuf>) -> Result<(Self, FilesystemRolloverDataWriter), String> {
        let root = root.into();
        let mut state: DurableRolloverState = serde_json::from_slice(
            &fs::read(root.join(STATE_FILE))
                .map_err(|error| format!("read rollover state: {error}"))?,
        )
        .map_err(|error| format!("decode rollover state: {error}"))?;
        validate_state_shape(&state)?;
        if matches!(
            state.phase,
            DurableRolloverPhase::Draining
                | DurableRolloverPhase::Complete
                | DurableRolloverPhase::Failed
        ) {
            return Err(format!("rollover recovery refuses terminal/ambiguous phase {:?}", state.phase));
        }
        for receipt in &state.sealed_streams {
            let id = receipt.logical_stream_id;
            let path = sealed_path(&root, id);
            if !path.is_file() || open_path(&root, id).exists() {
                return Err(format!("sealed logical stream {id} has ambiguous file topology"));
            }
            let mut sealed_validator = StatefulStreamValidator::identity_v1();
            let (last, file_hash, _) =
                scan_complete_stream_file(&path, &mut sealed_validator)
                    .map_err(|error| format!("read sealed logical stream {id}: {error}"))?;
            let identity = sealed_validator
                .identity()
                .ok_or_else(|| format!("sealed logical stream {id} has no CID1"))?;
            if last != Some(receipt.last_complete_sequence)
                || hex(&identity.campaign_id()) != receipt.campaign_id_hex
                || hex(&identity.payload_sha256()) != receipt.cid1_payload_sha256_hex
                || hex(&file_hash) != receipt.file_sha256_hex
            {
                return Err(format!("sealed logical stream {id} receipt mismatch"));
            }
        }
        let current_path = open_path(&root, state.logical_stream_id);
        if !current_path.is_file() || sealed_path(&root, state.logical_stream_id).exists() {
            return Err("current logical stream has ambiguous file topology".to_string());
        }
        let mut validator = StatefulStreamValidator::identity_v1();
        let mut pending_first_record = None;
        let (observed_last, _, only_record) =
            scan_complete_stream_file(&current_path, &mut validator)
                .map_err(|error| format!("read current logical stream: {error}"))?;
        match state.phase {
            DurableRolloverPhase::Streaming => {
                let identity = validator
                    .identity()
                    .ok_or("recovered streaming file has no CID1")?;
                if hex(&identity.campaign_id()) != state.campaign_id_hex
                    || hex(&identity.payload_sha256()) != state.cid1_payload_sha256_hex
                    || observed_last < state.last_complete_sequence
                {
                    return Err("recovered streaming bytes disagree with durable state".to_string());
                }
                // A complete durable file prefix may be newer than the state
                // checkpoint because per-record fsync would make capture cost
                // unbounded. Recompute the exact contiguous frontier from the
                // bytes and publish the recovered checkpoint before reuse.
                state.last_complete_sequence = observed_last;
                persist_state(&root, &state)?;
            }
            DurableRolloverPhase::AwaitingCid1 => {
                if observed_last.is_some() {
                    if observed_last != Some(0) {
                        return Err("awaiting-CID1 recovery contains ordinary records".to_string());
                    }
                    let identity = validator.identity().ok_or("awaiting-CID1 file has no CID1")?;
                    if hex(&identity.campaign_id()) != state.campaign_id_hex
                        || hex(&identity.payload_sha256()) != state.cid1_payload_sha256_hex
                    {
                        return Err("recovered candidate CID1 disagrees with transaction".to_string());
                    }
                    pending_first_record = Some(
                        only_record.ok_or("awaiting-CID1 recovery did not retain its sole record")?,
                    );
                }
            }
            DurableRolloverPhase::Draining
            | DurableRolloverPhase::Complete
            | DurableRolloverPhase::Failed => unreachable!(),
        }
        let file = OpenOptions::new()
            .append(true)
            .read(true)
            .open(&current_path)
            .map_err(|error| format!("reopen current logical stream: {error}"))?;
        let expected_campaign_id = decode_hex32(&state.campaign_id_hex)?;
        let expected_cid1_payload_sha256 = decode_hex32(&state.cid1_payload_sha256_hex)?;
        let shared = Arc::new(Shared {
            inner: Mutex::new(Inner {
                root,
                state,
                file: Some(file),
                validator,
                expected_campaign_id,
                expected_cid1_payload_sha256,
                pending_first_record,
                drain_target_sequence: None,
                terminal_error: None,
            }),
            changed: Condvar::new(),
        });
        Ok((Self { shared: shared.clone() }, FilesystemRolloverDataWriter { shared }))
    }

    pub fn durable_state(&self) -> Result<DurableRolloverState, String> {
        self.shared
            .inner
            .lock()
            .map(|inner| inner.state.clone())
            .map_err(|_| "rollover store mutex poisoned".to_string())
    }

    /// Wait for the data router to deliver the candidate sequence-zero CID1.
    /// The caller then passes these exact bytes to the second CFA1 barrier.
    pub fn wait_for_new_header(
        &self,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<Vec<u8>, String> {
        let mut inner = self.shared.inner.lock().map_err(|_| "rollover store mutex poisoned")?;
        loop {
            preflight(deadline, cancellation)?;
            if let Some(error) = inner.terminal_error.as_ref() {
                return Err(error.clone());
            }
            if let Some(record) = inner.pending_first_record.take() {
                return Ok(record);
            }
            let wait = deadline.remaining().unwrap_or(Duration::ZERO).min(Duration::from_millis(10));
            let (next, _) = self.shared.changed.wait_timeout(inner, wait)
                .map_err(|_| "rollover store mutex poisoned")?;
            inner = next;
        }
    }

    /// Commit only from the move-only result of the durable second CFA1. This
    /// joins storage release to the same acknowledgement that rebound the
    /// identity dispatcher; an integer logical id alone cannot resume writes.
    pub fn commit_acknowledged_new_header(
        &mut self,
        acknowledged: &AcknowledgedFirstCompactRecord,
    ) -> Result<(), String> {
        let acknowledgement = acknowledged.first_record().acknowledgement();
        let identity = acknowledged
            .first_record()
            .validated_cid1()
            .ok_or("acknowledged rollover header retained no CID1")?;
        let mut inner = self.shared.inner.lock().map_err(|_| "rollover store mutex poisoned")?;
        let transition_matches = inner.state.transitions.last().map_or(false, |transition| {
            transition.proposed_logical_stream_id == acknowledgement.logical_stream_id
                && transition.boundary_game_hour_raw == identity.game_hour_raw()
        });
        if inner.state.phase != DurableRolloverPhase::AwaitingCid1
            || inner.state.logical_stream_id != acknowledgement.logical_stream_id
            || inner.validator.identity().is_none()
            || inner.pending_first_record.is_some()
            || acknowledgement.h4cr_sequence != 0
            || acknowledgement.payload_sha256 != inner.expected_cid1_payload_sha256
            || identity.campaign_id() != inner.expected_campaign_id
            || !transition_matches
            || acknowledged.durable_receipt().acknowledgement() != acknowledgement
        {
            let result = terminal(&mut inner, "new-header commit does not match awaited CID1");
            self.shared.changed.notify_all();
            return result;
        }
        // Do not expose Streaming to the waiting data writer until the exact
        // release state is durable.  On a persistence error the shared state
        // remains AwaitingCid1, so no sequence-one record can escape before
        // the caller terminalizes the transaction.
        let mut committed_state = inner.state.clone();
        committed_state.phase = DurableRolloverPhase::Streaming;
        committed_state.last_complete_sequence = Some(0);
        if let Err(error) = persist_state(&inner.root, &committed_state) {
            let reason = format!("persist acknowledged rollover release failed: {error}");
            inner.state.phase = DurableRolloverPhase::Failed;
            inner.terminal_error = Some(reason.clone());
            let _ = persist_state(&inner.root, &inner.state);
            self.shared.changed.notify_all();
            return Err(reason);
        }
        inner.state = committed_state;
        self.shared.changed.notify_all();
        Ok(())
    }

    pub fn fail_pending_header(&mut self, reason: &str) -> Result<(), String> {
        let mut inner = self
            .shared
            .inner
            .lock()
            .map_err(|_| "rollover store mutex poisoned".to_string())?;
        let result = terminal(&mut inner, reason);
        self.shared.changed.notify_all();
        result
    }

    /// Durably close the final logical stream and publish the exact ordered set
    /// descriptor consumed by per-stream folding and packaging. This is the only
    /// transition to `Complete`; an open/current file is never advertised.
    pub fn seal_for_publication(&mut self) -> Result<DurableRolloverSetDescriptor, String> {
        let mut inner = self
            .shared
            .inner
            .lock()
            .map_err(|_| "rollover store mutex poisoned".to_string())?;
        if inner.state.phase != DurableRolloverPhase::Streaming {
            return terminal(&mut inner, "publication seal requested outside Streaming");
        }
        let logical_stream_id = inner.state.logical_stream_id;
        let last_complete_sequence = inner
            .state
            .last_complete_sequence
            .ok_or("publication stream has no admitted CID1")?;
        if last_complete_sequence == 0 {
            return terminal(
                &mut inner,
                "publication stream never admitted a post-CID1 world record",
            );
        }
        let open = open_path(&inner.root, logical_stream_id);
        let sealed = sealed_path(&inner.root, logical_stream_id);
        let mut file = inner
            .file
            .take()
            .ok_or("publication stream file is absent")?;
        durable_file(&mut file, &open)?;
        drop(file);
        fs::rename(&open, &sealed)
            .map_err(|error| format!("seal final logical stream {logical_stream_id}: {error}"))?;
        sync_directory(&inner.root)?;
        let file_sha256 = hash_file(&sealed)?;
        let campaign_id_hex = hex(&inner.expected_campaign_id);
        let cid1_payload_sha256_hex = hex(&inner.expected_cid1_payload_sha256);
        inner.state.sealed_streams.push(DurableSealedStream {
            logical_stream_id,
            last_complete_sequence,
            campaign_id_hex,
            cid1_payload_sha256_hex,
            file_sha256_hex: hex(&file_sha256),
        });
        inner.state.phase = DurableRolloverPhase::Complete;
        persist_state(&inner.root, &inner.state)?;
        let descriptor = publication_descriptor_locked(&inner)?;
        let bytes = serde_json::to_vec_pretty(&descriptor)
            .map_err(|error| format!("encode rollover set descriptor: {error}"))?;
        publish_new_file_atomically(&inner.root, SET_DESCRIPTOR_FILE, &bytes)?;
        Ok(descriptor)
    }

    pub fn publication_descriptor(&self) -> Result<DurableRolloverSetDescriptor, String> {
        let inner = self
            .shared
            .inner
            .lock()
            .map_err(|_| "rollover store mutex poisoned".to_string())?;
        publication_descriptor_locked(&inner)
    }

    /// Re-admit a crash-complete raw set for packaging recovery. The durable
    /// state, every member hash, and the atomically published descriptor must
    /// agree byte-for-byte before any package finalizer receives a path.
    pub fn recover_publication_descriptor(
        root: impl AsRef<Path>,
    ) -> Result<DurableRolloverSetDescriptor, String> {
        let root = root.as_ref();
        let state: DurableRolloverState = serde_json::from_slice(
            &fs::read(root.join(STATE_FILE))
                .map_err(|error| format!("read completed rollover state: {error}"))?,
        )
        .map_err(|error| format!("decode completed rollover state: {error}"))?;
        if state.phase != DurableRolloverPhase::Complete {
            return Err("rollover packaging recovery requires Complete durable state".into());
        }
        validate_state_shape(&state)?;
        let mut observed_identity_hours = Vec::with_capacity(state.sealed_streams.len());
        for receipt in &state.sealed_streams {
            let path = sealed_path(root, receipt.logical_stream_id);
            let (observed_last, observed_campaign_id, observed_cid1_payload, observed_hour, file_hash) =
                validate_complete_stream_file(&path).map_err(|error| {
                    format!(
                        "read completed logical stream {}: {error}",
                        receipt.logical_stream_id
                    )
                })?;
            if observed_last != Some(receipt.last_complete_sequence)
                || hex(&observed_campaign_id) != receipt.campaign_id_hex
                || hex(&observed_cid1_payload) != receipt.cid1_payload_sha256_hex
                || hex(&file_hash) != receipt.file_sha256_hex
            {
                return Err(format!(
                    "completed logical stream {} differs from durable state",
                    receipt.logical_stream_id
                ));
            }
            observed_identity_hours.push(observed_hour);
        }
        for (index, transition) in state.transitions.iter().enumerate() {
            if observed_identity_hours.get(index + 1).copied()
                != Some(transition.boundary_game_hour_raw)
            {
                return Err(format!(
                    "rollover transition {} boundary hour differs from next CID1",
                    transition.proposed_logical_stream_id
                ));
            }
        }
        let inner = Inner {
            root: root.to_path_buf(),
            state,
            file: None,
            validator: StatefulStreamValidator::identity_v1(),
            expected_campaign_id: [0; 32],
            expected_cid1_payload_sha256: [0; 32],
            pending_first_record: None,
            drain_target_sequence: None,
            terminal_error: None,
        };
        let mechanical = publication_descriptor_locked(&inner)?;
        let descriptor_path = root.join(SET_DESCRIPTOR_FILE);
        if !descriptor_path.exists() {
            let bytes = serde_json::to_vec_pretty(&mechanical)
                .map_err(|error| format!("encode recovered rollover set descriptor: {error}"))?;
            publish_new_file_atomically(root, SET_DESCRIPTOR_FILE, &bytes)?;
        }
        let published_bytes = fs::read(&descriptor_path)
            .map_err(|error| format!("read published rollover set descriptor: {error}"))?;
        let published: DurableRolloverSetDescriptor = serde_json::from_slice(&published_bytes)
            .map_err(|error| format!("decode published rollover set descriptor: {error}"))?;
        if published != mechanical {
            return Err("published rollover set descriptor differs from durable members".into());
        }
        Ok(published)
    }

    #[cfg(test)]
    fn commit_new_header_for_test(&mut self, logical_stream_id: u64) -> Result<(), String> {
        let mut inner = self.shared.inner.lock().map_err(|_| "rollover store mutex poisoned")?;
        if inner.state.phase != DurableRolloverPhase::AwaitingCid1
            || inner.state.logical_stream_id != logical_stream_id
            || inner.validator.identity().is_none()
            || inner.pending_first_record.is_some()
        {
            return terminal(&mut inner, "test new-header commit does not match awaited CID1");
        }
        let mut committed_state = inner.state.clone();
        committed_state.phase = DurableRolloverPhase::Streaming;
        committed_state.last_complete_sequence = Some(0);
        persist_state(&inner.root, &committed_state)?;
        inner.state = committed_state;
        self.shared.changed.notify_all();
        Ok(())
    }
}

impl FilesystemRolloverDataWriter {
    pub fn append_complete_record(&self, record: &[u8]) -> Result<(), String> {
        let mut inner = self.shared.inner.lock().map_err(|_| "rollover store mutex poisoned")?;
        if let Some(error) = inner.terminal_error.as_ref() {
            return Err(error.clone());
        }
        let header = parse_compact_record_header(record)
            .map_err(|error| format!("rollover H4CR framing failed: {error}"))?;
        if header.record_len() != record.len() {
            return terminal(&mut inner, "rollover H4CR record is truncated or has trailing bytes");
        }
        while inner.state.phase == DurableRolloverPhase::AwaitingCid1 && header.sequence != 0 {
            if let Some(error) = inner.terminal_error.as_ref() {
                return Err(error.clone());
            }
            let (next, _) = self
                .shared
                .changed
                .wait_timeout(inner, Duration::from_millis(10))
                .map_err(|_| "rollover store mutex poisoned")?;
            inner = next;
        }
        match inner.state.phase {
            DurableRolloverPhase::Streaming => {
                let expected = inner.state.last_complete_sequence.and_then(|value| value.checked_add(1));
                if expected != Some(header.sequence) {
                    return terminal(&mut inner, "rollover H4CR sequence is not contiguous");
                }
            }
            DurableRolloverPhase::Draining => match inner.drain_target_sequence {
                Some(target) if header.sequence <= target => {
                    let expected = inner
                        .state
                        .last_complete_sequence
                        .and_then(|value| value.checked_add(1));
                    if expected != Some(header.sequence) {
                        return terminal(&mut inner, "draining H4CR sequence is not contiguous");
                    }
                }
                _ => {
                    return terminal(
                        &mut inner,
                        "collector wrote H4CR after the advertised drain boundary",
                    )
                }
            },
            DurableRolloverPhase::AwaitingCid1 if header.sequence == 0 => {}
            DurableRolloverPhase::AwaitingCid1 => {
                return terminal(&mut inner, "ordinary H4CR arrived before rollover CID1 acknowledgement")
            }
            DurableRolloverPhase::Complete | DurableRolloverPhase::Failed => {
                return Err("rollover store is terminal".to_string())
            }
        }
        let kind = match admit_complete_record(&mut inner.validator, record) {
            Ok(kind) => kind,
            Err(error) => return terminal(&mut inner, &error),
        };
        if inner.state.phase == DurableRolloverPhase::AwaitingCid1 {
            if kind != AcceptedRecordKind::CampaignIdentity {
                return terminal(&mut inner, "new logical stream did not start with CID1");
            }
            let identity = inner.validator.identity().ok_or("CID1 validator lost identity")?;
            if identity.campaign_id() != inner.expected_campaign_id
                || identity.payload_sha256() != inner.expected_cid1_payload_sha256
            {
                return terminal(&mut inner, "new logical stream CID1 differs from accepted CRP1");
            }
        } else if kind != AcceptedRecordKind::Ordinary {
            return terminal(&mut inner, "streaming logical stream received a duplicate CID1");
        }
        inner.file.as_mut().ok_or("rollover stream file is absent")?
            .write_all(record)
            .map_err(|error| format!("write rollover H4CR: {error}"))?;
        inner.state.last_complete_sequence = Some(header.sequence);
        if inner.state.phase == DurableRolloverPhase::AwaitingCid1 {
            inner.file.as_mut().ok_or("rollover stream file is absent")?
                .sync_all().map_err(|error| format!("sync rollover CID1: {error}"))?;
            inner.pending_first_record = Some(record.to_vec());
        }
        self.shared.changed.notify_all();
        Ok(())
    }
}

impl RecorderRolloverStore for FilesystemRolloverStore {
    fn drain_contiguous_through(
        &mut self,
        proposal: &RolloverProposeV1,
        deadline: ControlDeadline,
        cancellation: &ControlCancellation,
    ) -> Result<DrainedLogicalStream, String> {
        let mut inner = self.shared.inner.lock().map_err(|_| "rollover store mutex poisoned")?;
        if inner.state.phase != DurableRolloverPhase::Streaming {
            return terminal(&mut inner, "drain requested outside Streaming");
        }
        inner.state.phase = DurableRolloverPhase::Draining;
        inner.drain_target_sequence = Some(proposal.last_admitted_h4cr_sequence);
        persist_state(&inner.root, &inner.state)?;
        loop {
            preflight(deadline, cancellation)?;
            let last = inner.state.last_complete_sequence.ok_or("stream has no admitted CID1")?;
            if last > proposal.last_admitted_h4cr_sequence {
                return terminal(&mut inner, "data router passed the proposed drain boundary");
            }
            if last == proposal.last_admitted_h4cr_sequence {
                inner.file.as_mut().ok_or("rollover stream file is absent")?
                    .sync_all().map_err(|error| format!("sync drained stream: {error}"))?;
                return Ok(DrainedLogicalStream {
                    logical_stream_id: inner.state.logical_stream_id,
                    last_h4cr_sequence: last,
                    campaign_id: inner.expected_campaign_id,
                    cid1_payload_sha256: inner.expected_cid1_payload_sha256,
                });
            }
            let wait = deadline.remaining().unwrap_or(Duration::ZERO).min(Duration::from_millis(10));
            let (next, _) = self.shared.changed.wait_timeout(inner, wait)
                .map_err(|_| "rollover store mutex poisoned")?;
            inner = next;
        }
    }

    fn durable_seal_and_open(
        &mut self,
        proposal: &RolloverProposeV1,
        drained: DrainedLogicalStream,
    ) -> Result<OpenedLogicalStream, String> {
        let mut inner = self.shared.inner.lock().map_err(|_| "rollover store mutex poisoned")?;
        if inner.state.phase != DurableRolloverPhase::Draining
            || drained.logical_stream_id != inner.state.logical_stream_id
            || drained.last_h4cr_sequence != inner.state.last_complete_sequence.unwrap_or(u64::MAX)
        {
            return terminal(&mut inner, "seal/open receipt does not match drained owner");
        }
        if proposal.validate().is_err()
            || proposal.proposed_logical_stream_id
                != inner
                    .state
                    .logical_stream_id
                    .checked_add(1)
                    .unwrap_or(u64::MAX)
            || proposal.last_admitted_h4cr_sequence != drained.last_h4cr_sequence
            || proposal.current_campaign_id != drained.campaign_id
            || proposal.current_cid1_payload_sha256 != drained.cid1_payload_sha256
            || proposal.proposal_nonce <= inner.state.last_proposal_nonce
        {
            return terminal(
                &mut inner,
                "seal/open proposal differs from drained transition authority",
            );
        }
        let old_id = inner.state.logical_stream_id;
        let old_open = open_path(&inner.root, old_id);
        let old_sealed = sealed_path(&inner.root, old_id);
        let mut file = inner.file.take().ok_or("rollover stream file is absent")?;
        durable_file(&mut file, &old_open)?;
        drop(file);
        fs::rename(&old_open, &old_sealed)
            .map_err(|error| format!("seal logical stream {old_id}: {error}"))?;
        sync_directory(&inner.root)?;

        let new_id = proposal.proposed_logical_stream_id;
        let new_open = open_path(&inner.root, new_id);
        let mut new_file = create_new_file(&new_open)?;
        durable_file(&mut new_file, &new_open)?;
        let sealed_hash = hash_file(&old_sealed)?;
        inner.state.sealed_streams.push(DurableSealedStream {
            logical_stream_id: old_id,
            last_complete_sequence: drained.last_h4cr_sequence,
            campaign_id_hex: hex(&drained.campaign_id),
            cid1_payload_sha256_hex: hex(&drained.cid1_payload_sha256),
            file_sha256_hex: hex(&sealed_hash),
        });
        inner.state.transitions.push(DurableRolloverTransition {
            current_logical_stream_id: old_id,
            proposed_logical_stream_id: new_id,
            last_admitted_h4cr_sequence: proposal.last_admitted_h4cr_sequence,
            boundary_game_hour_raw: proposal.boundary_game_hour_raw,
            reason: match proposal.reason {
                compact_contract::campaign_rollover::RolloverReason::WholePairChanged => {
                    "whole_pair_changed"
                }
                compact_contract::campaign_rollover::RolloverReason::SamePairTimeRollback => {
                    "same_pair_time_rollback"
                }
            }
            .to_string(),
            proposal_nonce: proposal.proposal_nonce,
            current_campaign_id_hex: hex(&proposal.current_campaign_id),
            proposed_campaign_id_hex: hex(&proposal.proposed_campaign_id),
            current_cid1_payload_sha256_hex: hex(&proposal.current_cid1_payload_sha256),
            proposed_cid1_payload_sha256_hex: hex(&proposal.proposed_cid1_payload_sha256),
        });
        inner.state.logical_stream_id = new_id;
        inner.state.campaign_id_hex = hex(&proposal.proposed_campaign_id);
        inner.state.cid1_payload_sha256_hex = hex(&proposal.proposed_cid1_payload_sha256);
        inner.state.last_complete_sequence = None;
        inner.state.last_proposal_nonce = proposal.proposal_nonce;
        inner.state.phase = DurableRolloverPhase::AwaitingCid1;
        inner.file = Some(new_file);
        inner.validator = StatefulStreamValidator::identity_v1();
        inner.expected_campaign_id = proposal.proposed_campaign_id;
        inner.expected_cid1_payload_sha256 = proposal.proposed_cid1_payload_sha256;
        inner.pending_first_record = None;
        inner.drain_target_sequence = None;
        persist_state(&inner.root, &inner.state)?;
        self.shared.changed.notify_all();
        Ok(OpenedLogicalStream {
            sealed_logical_stream_id: old_id,
            opened_logical_stream_id: new_id,
            expected_campaign_id: proposal.proposed_campaign_id,
            expected_cid1_payload_sha256: proposal.proposed_cid1_payload_sha256,
        })
    }

    fn durable_record_rejection(
        &mut self,
        proposal: &RolloverProposeV1,
        error_code: u16,
        reason: &str,
    ) -> Result<(), String> {
        let inner = self.shared.inner.lock().map_err(|_| "rollover store mutex poisoned")?;
        let dir = inner.root.join(REJECTION_DIRECTORY);
        fs::create_dir_all(&dir).map_err(|error| format!("create rejection journal: {error}"))?;
        let path = dir.join(format!("nonce-{}.json", proposal.proposal_nonce));
        let bytes = serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "cid1-rollover-rejection-v1",
            "proposal_nonce": proposal.proposal_nonce,
            "proposed_logical_stream_id": proposal.proposed_logical_stream_id,
            "error_code": error_code,
            "reason": reason,
            "proposal_sha256": hex(&Sha256::digest(compact_contract::campaign_rollover::encode_rollover_propose(proposal).map_err(|e| e.to_string())?)),
        })).map_err(|error| format!("encode rejection journal: {error}"))?;
        let mut file = create_new_file(&path)?;
        file.write_all(&bytes).map_err(|error| format!("write rejection journal: {error}"))?;
        durable_file(&mut file, &path)?;
        sync_directory(&dir)
    }
}

fn admit_complete_record(
    validator: &mut StatefulStreamValidator,
    record: &[u8],
) -> Result<AcceptedRecordKind, String> {
    let header = parse_compact_record_header(record).map_err(|error| error.to_string())?;
    if header.record_len() != record.len() {
        return Err("H4CR record length mismatch".to_string());
    }
    validator
        .validate_record(&header, &record[COMPACT_RECORD_HEADER_LEN..])
        .map_err(|error| error.to_string())
}

fn validate_complete_stream_prefix(
    validator: &mut StatefulStreamValidator,
    bytes: &[u8],
) -> Result<Option<u64>, String> {
    let mut offset = 0usize;
    let mut last = None;
    while offset < bytes.len() {
        let header = parse_compact_record_header(&bytes[offset..])
            .map_err(|error| format!("recovered stream framing at {offset}: {error}"))?;
        let end = offset.checked_add(header.record_len()).ok_or("recovered stream length overflow")?;
        if end > bytes.len() {
            return Err("recovered stream has a partial terminal record".to_string());
        }
        admit_complete_record(validator, &bytes[offset..end])?;
        last = Some(header.sequence);
        offset = end;
    }
    Ok(last)
}

/// Re-admit a sealed H4CR stream with memory bounded to one record. Recovery
/// runs on production-sized captures, so it must never materialize the whole
/// logical stream merely to reproduce the durable receipt.
fn validate_complete_stream_file(
    path: &Path,
) -> Result<(Option<u64>, [u8; 32], [u8; 32], i32, [u8; 32]), String> {
    let mut validator = StatefulStreamValidator::identity_v1();
    let (last, file_hash, _) = scan_complete_stream_file(path, &mut validator)?;
    let identity = validator
        .identity()
        .ok_or("completed logical stream has no CID1")?;
    Ok((
        last,
        identity.campaign_id(),
        identity.payload_sha256(),
        identity.game_hour_raw(),
        file_hash,
    ))
}

fn scan_complete_stream_file(
    path: &Path,
    validator: &mut StatefulStreamValidator,
) -> Result<(Option<u64>, [u8; 32], Option<Vec<u8>>), String> {
    let mut file = File::open(path)
        .map_err(|error| format!("open completed stream {}: {error}", path.display()))?;
    let mut file_hash = Sha256::new();
    let mut last = None;
    let mut only_record = None;
    let mut offset = 0u64;
    loop {
        let mut header_bytes = [0u8; COMPACT_RECORD_HEADER_LEN];
        let first = file
            .read(&mut header_bytes[..1])
            .map_err(|error| format!("read recovered stream header at {offset}: {error}"))?;
        if first == 0 {
            break;
        }
        file.read_exact(&mut header_bytes[1..]).map_err(|error| {
            format!("recovered stream has a partial terminal header at {offset}: {error}")
        })?;
        let header = parse_compact_record_header(&header_bytes)
            .map_err(|error| format!("recovered stream framing at {offset}: {error}"))?;
        let record_len = header.record_len();
        if header.payload_len > COMPACT_MAX_PAYLOAD_LEN as usize {
            return Err(format!(
                "recovered H4CR payload at {offset} exceeds {COMPACT_MAX_PAYLOAD_LEN} byte bound"
            ));
        }
        let mut payload = vec![0u8; header.payload_len];
        file.read_exact(&mut payload).map_err(|error| {
            format!("recovered stream has a partial terminal record at {offset}: {error}")
        })?;
        validator
            .validate_record(&header, &payload)
            .map_err(|error| format!("recovered stream validation at {offset}: {error}"))?;
        file_hash.update(header_bytes);
        file_hash.update(&payload);
        if last.is_none() {
            let mut record = Vec::with_capacity(record_len);
            record.extend_from_slice(&header_bytes);
            record.extend_from_slice(&payload);
            only_record = Some(record);
        } else {
            only_record = None;
        }
        last = Some(header.sequence);
        offset = offset
            .checked_add(record_len as u64)
            .ok_or("recovered stream offset overflow")?;
    }
    Ok((last, file_hash.finalize().into(), only_record))
}

fn terminal<T>(inner: &mut Inner, reason: &str) -> Result<T, String> {
    inner.state.phase = DurableRolloverPhase::Failed;
    inner.terminal_error = Some(reason.to_string());
    let _ = persist_state(&inner.root, &inner.state);
    Err(reason.to_string())
}

fn preflight(deadline: ControlDeadline, cancellation: &ControlCancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("rollover storage operation cancelled".to_string())
    } else if deadline.is_expired() {
        Err("rollover storage operation deadline expired".to_string())
    } else {
        Ok(())
    }
}

fn validate_state_shape(state: &DurableRolloverState) -> Result<(), String> {
    let expected_sealed_ids: Box<dyn Iterator<Item = u64>> =
        if state.phase == DurableRolloverPhase::Complete {
            Box::new(0..=state.logical_stream_id)
        } else {
            Box::new(0..state.logical_stream_id)
        };
    if state.schema != "cid1-rollover-fs-v1"
        || decode_hex32(&state.campaign_id_hex)? == [0; 32]
        || decode_hex32(&state.cid1_payload_sha256_hex)? == [0; 32]
        || state
            .sealed_streams
            .iter()
            .map(|stream| stream.logical_stream_id)
            .ne(expected_sealed_ids)
        || state.sealed_streams.iter().any(|stream| {
            [
                &stream.campaign_id_hex,
                &stream.cid1_payload_sha256_hex,
                &stream.file_sha256_hex,
            ]
            .iter()
            .any(|digest| decode_hex32(digest).map_or(true, |bytes| bytes == [0; 32]))
        })
    {
        return Err("rollover durable state is non-canonical".to_string());
    }
    if state.transitions.len()
        != usize::try_from(state.logical_stream_id)
            .map_err(|_| "rollover logical stream id exceeds usize".to_string())?
    {
        return Err("rollover transition count does not match logical stream frontier".into());
    }
    let mut previous_nonce = 0u64;
    for (index, transition) in state.transitions.iter().enumerate() {
        let current_id = u64::try_from(index)
            .map_err(|_| "rollover transition count exceeds u64".to_string())?;
        let proposed_id = current_id
            .checked_add(1)
            .ok_or("rollover transition id exhausted")?;
        let current = state
            .sealed_streams
            .get(index)
            .ok_or("rollover transition has no sealed current member")?;
        let (proposed_campaign_id_hex, proposed_payload_hex) =
            match state.sealed_streams.get(index + 1) {
                Some(next) => (&next.campaign_id_hex, &next.cid1_payload_sha256_hex),
                None => (&state.campaign_id_hex, &state.cid1_payload_sha256_hex),
            };
        let reason = match transition.reason.as_str() {
            "whole_pair_changed" => {
                compact_contract::campaign_rollover::RolloverReason::WholePairChanged
            }
            "same_pair_time_rollback" => {
                compact_contract::campaign_rollover::RolloverReason::SamePairTimeRollback
            }
            _ => return Err("rollover transition reason is not canonical".into()),
        };
        let proposal = RolloverProposeV1 {
            proposed_logical_stream_id: transition.proposed_logical_stream_id,
            last_admitted_h4cr_sequence: transition.last_admitted_h4cr_sequence,
            boundary_game_hour_raw: transition.boundary_game_hour_raw,
            reason,
            current_campaign_id: decode_hex32(&transition.current_campaign_id_hex)?,
            proposed_campaign_id: decode_hex32(&transition.proposed_campaign_id_hex)?,
            current_cid1_payload_sha256: decode_hex32(
                &transition.current_cid1_payload_sha256_hex,
            )?,
            proposed_cid1_payload_sha256: decode_hex32(
                &transition.proposed_cid1_payload_sha256_hex,
            )?,
            proposal_nonce: transition.proposal_nonce,
        };
        proposal
            .validate()
            .map_err(|error| format!("rollover transition CRP1 is invalid: {error}"))?;
        if transition.current_logical_stream_id != current_id
            || transition.proposed_logical_stream_id != proposed_id
            || transition.last_admitted_h4cr_sequence != current.last_complete_sequence
            || transition.current_campaign_id_hex != current.campaign_id_hex
            || transition.current_cid1_payload_sha256_hex != current.cid1_payload_sha256_hex
            || &transition.proposed_campaign_id_hex != proposed_campaign_id_hex
            || &transition.proposed_cid1_payload_sha256_hex != proposed_payload_hex
            || transition.proposal_nonce <= previous_nonce
        {
            return Err("rollover transition chain is not adjacent to sealed members".into());
        }
        previous_nonce = transition.proposal_nonce;
    }
    if state.last_proposal_nonce != previous_nonce {
        return Err("rollover durable nonce frontier differs from transition chain".into());
    }
    Ok(())
}

fn publication_descriptor_locked(inner: &Inner) -> Result<DurableRolloverSetDescriptor, String> {
    if inner.state.phase != DurableRolloverPhase::Complete || inner.file.is_some() {
        return Err("rollover stream set is not durably complete".to_string());
    }
    validate_state_shape(&inner.state)?;
    let mut ordered_streams = Vec::with_capacity(inner.state.sealed_streams.len());
    for receipt in &inner.state.sealed_streams {
        let path = sealed_path(&inner.root, receipt.logical_stream_id);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("inspect published logical stream: {error}"))?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(format!(
                "published logical stream {} is not a regular file",
                receipt.logical_stream_id
            ));
        }
        let actual_sha256 = hex(&hash_file(&path)?);
        if actual_sha256 != receipt.file_sha256_hex {
            return Err(format!(
                "published logical stream {} hash changed",
                receipt.logical_stream_id
            ));
        }
        ordered_streams.push(DurableRolloverPublicationStream {
            logical_stream_id: receipt.logical_stream_id,
            relative_path: format!(
                "logical-stream-{}.sealed.h4cr",
                receipt.logical_stream_id
            ),
            last_complete_sequence: receipt.last_complete_sequence,
            campaign_id_hex: receipt.campaign_id_hex.clone(),
            cid1_payload_sha256_hex: receipt.cid1_payload_sha256_hex.clone(),
            file_bytes: metadata.len(),
            file_sha256_hex: receipt.file_sha256_hex.clone(),
        });
    }
    Ok(DurableRolloverSetDescriptor {
        schema: "cid1-rollover-set-v1".to_string(),
        ordered_streams,
        transitions: inner.state.transitions.clone(),
    })
}

fn persist_state(root: &Path, state: &DurableRolloverState) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(state).map_err(|error| format!("encode rollover state: {error}"))?;
    let temp = root.join(format!("{STATE_FILE}.new"));
    let final_path = root.join(STATE_FILE);
    if temp.exists() {
        fs::remove_file(&temp).map_err(|error| format!("remove stale rollover state candidate: {error}"))?;
    }
    let mut file = create_new_file(&temp)?;
    file.write_all(&bytes).map_err(|error| format!("write rollover state: {error}"))?;
    durable_file(&mut file, &temp)?;
    drop(file);
    replace_file(&temp, &final_path)?;
    sync_directory(root)
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination).map_err(|error| format!("publish rollover state: {error}"))
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    if !destination.exists() {
        return fs::rename(source, destination)
            .map_err(|error| format!("publish initial rollover state: {error}"));
    }
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn ReplaceFileW(
            replaced_file_name: *const u16,
            replacement_file_name: *const u16,
            backup_file_name: *const u16,
            replace_flags: u32,
            exclude: *mut std::ffi::c_void,
            reserved: *mut std::ffi::c_void,
        ) -> i32;
    }
    const REPLACEFILE_WRITE_THROUGH: u32 = 0x0000_0001;
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let source_wide: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    if unsafe {
        ReplaceFileW(
            destination_wide.as_ptr(),
            source_wide.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        Err(format!(
            "atomically publish rollover state: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

fn create_new_file(path: &Path) -> Result<File, String> {
    OpenOptions::new().read(true).write(true).create_new(true).open(path)
        .map_err(|error| format!("create {}: {error}", path.display()))
}

/// Atomically publish a new immutable descriptor. A byte-identical final file
/// makes crash recovery idempotent; a differing pre-existing file fails closed.
pub fn publish_immutable_descriptor(
    root: impl AsRef<Path>,
    file_name: &str,
    bytes: &[u8],
) -> Result<(), String> {
    publish_new_file_atomically(root.as_ref(), file_name, bytes)
}

fn publish_new_file_atomically(root: &Path, file_name: &str, bytes: &[u8]) -> Result<(), String> {
    if file_name.is_empty()
        || Path::new(file_name).components().count() != 1
        || file_name == "."
        || file_name == ".."
    {
        return Err("immutable descriptor file name is not a single normal component".into());
    }
    let final_path = root.join(file_name);
    if final_path.exists() {
        let existing = fs::read(&final_path)
            .map_err(|error| format!("read existing {}: {error}", final_path.display()))?;
        return if existing == bytes {
            Ok(())
        } else {
            Err(format!(
                "immutable descriptor {} already exists with different bytes",
                final_path.display()
            ))
        };
    }
    let candidate = root.join(format!("{file_name}.new"));
    if candidate.exists() {
        fs::remove_file(&candidate).map_err(|error| {
            format!("remove stale descriptor candidate {}: {error}", candidate.display())
        })?;
    }
    let mut file = create_new_file(&candidate)?;
    file.write_all(bytes)
        .map_err(|error| format!("write {}: {error}", candidate.display()))?;
    durable_file(&mut file, &candidate)?;
    drop(file);
    fs::rename(&candidate, &final_path).map_err(|error| {
        format!(
            "publish immutable descriptor {}: {error}",
            final_path.display()
        )
    })?;
    sync_directory(root)
}

fn durable_file(file: &mut File, path: &Path) -> Result<(), String> {
    file.flush().map_err(|error| format!("flush {}: {error}", path.display()))?;
    file.sync_all().map_err(|error| format!("sync {}: {error}", path.display()))
}

fn hash_file(path: &Path) -> Result<[u8; 32], String> {
    let mut file = File::open(path).map_err(|error| format!("open {} for hashing: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| format!("hash {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest.finalize().into())
}

fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        File::open(path).and_then(|file| file.sync_all())
            .map_err(|error| format!("sync directory {}: {error}", path.display()))
    }
    #[cfg(windows)]
    {
        // File data and rename metadata are write-through on the supported
        // Windows product path; opening directories portably requires Win32
        // backup semantics and is already covered by the identity journal.
        let _ = path;
        Ok(())
    }
}

fn open_path(root: &Path, id: u64) -> PathBuf {
    root.join(format!("logical-stream-{id}.open.h4cr"))
}

fn sealed_path(root: &Path, id: u64) -> PathBuf {
    root.join(format!("logical-stream-{id}.sealed.h4cr"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_hex32(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) {
        return Err("digest is not canonical lowercase hex".to_string());
    }
    let mut out = [0u8; 32];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| "digest contains invalid hex".to_string())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::calendar::DATE_EPOCH_CONST;
    use compact_contract::campaign_identity::{
        encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses,
        COMPACT_DOMAIN_STREAM_HEADER, COMPACT_RECORD_TYPE_STREAM_HEADER,
    };
    use compact_contract::campaign_rollover::RolloverReason;
    use compact_contract::{
        COMPACT_RECORD_MAGIC, COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_VERSION,
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{sync_channel, RecvTimeoutError};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "hoi4-cid1-rollover-fs-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn hour(offset: i32) -> i32 {
        DATE_EPOCH_CONST + 1936 * 365 * 24 + offset
    }

    fn record(sequence: u64, game_hour_raw: i32, domain: u16, ty: u16, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; COMPACT_RECORD_HEADER_LEN];
        bytes[0..4].copy_from_slice(COMPACT_RECORD_MAGIC);
        bytes[4..6].copy_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        bytes[6..8].copy_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        bytes[8..16].copy_from_slice(&sequence.to_le_bytes());
        bytes[16..20].copy_from_slice(&game_hour_raw.to_le_bytes());
        bytes[20..22].copy_from_slice(&domain.to_le_bytes());
        bytes[22..24].copy_from_slice(&ty.to_le_bytes());
        bytes[24..28].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn ordinary_record(sequence: u64, game_hour_raw: i32) -> Vec<u8> {
        // Domain 18 plus DOMAIN_DATA and a nonempty opaque payload is the
        // canonical ordinary-record fixture also used by the stream validator.
        // These tests exercise durable byte custody, not a domain decoder.
        record(
            sequence,
            game_hour_raw,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            b"DATA",
        )
    }

    fn cid1(seed: i32, observed_hour: i32) -> (Vec<u8>, ValidatedCid1) {
        let observation = Cid1Observation {
            game_unique_id: "a4b17fed-ed12-4361-9064-fb6576638110".to_string(),
            game_unique_seed: seed,
            game_hour_raw: observed_hour,
        };
        let fields = Cid1Fields::from_stable_observations(
            observation.clone(),
            observation,
            "rollover-fs-test".to_string(),
            Cid1Witnesses::default(),
        )
        .unwrap();
        let payload = encode_cid1(&fields).unwrap();
        let complete = record(
            0,
            observed_hour,
            COMPACT_DOMAIN_STREAM_HEADER,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            &payload,
        );
        let validated = ValidatedFirstCompactRecord::identity_v1(0, &complete)
            .unwrap()
            .validated_cid1()
            .unwrap()
            .clone();
        (complete, validated)
    }

    fn proposal(current: &ValidatedCid1, next: &ValidatedCid1) -> RolloverProposeV1 {
        RolloverProposeV1 {
            proposed_logical_stream_id: 1,
            last_admitted_h4cr_sequence: 1,
            boundary_game_hour_raw: hour(2),
            reason: RolloverReason::WholePairChanged,
            current_campaign_id: current.campaign_id(),
            proposed_campaign_id: next.campaign_id(),
            current_cid1_payload_sha256: current.payload_sha256(),
            proposed_cid1_payload_sha256: next.payload_sha256(),
            proposal_nonce: 1,
        }
    }

    #[test]
    fn durable_store_cuts_files_and_withholds_streaming_until_cid1_commit() {
        let root = TestRoot::new();
        let (first, current) = cid1(-1234, hour(0));
        let (mut store, writer) =
            FilesystemRolloverStore::create_for_test(&root.0, &first).unwrap();
        let ordinary = ordinary_record(1, hour(1));
        writer.append_complete_record(&ordinary).unwrap();
        let (next_first, next) = cid1(-1235, hour(2));
        let proposal = proposal(&current, &next);
        let drained = store
            .drain_contiguous_through(
                &proposal,
                ControlDeadline::after(Duration::from_secs(1)).unwrap(),
                &ControlCancellation::new(),
            )
            .unwrap();
        store.durable_seal_and_open(&proposal, drained).unwrap();

        // A sequence-one world record must remain withheld while the control
        // owner is awaiting the new CID1. It becomes admissible only after the
        // exact header has been observed and the second durable barrier commits.
        let blocked_writer = writer.clone();
        let (started_tx, started_rx) = sync_channel(0);
        let (done_tx, done_rx) = sync_channel(1);
        let blocked = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            done_tx
                .send(blocked_writer.append_complete_record(&ordinary_record(1, hour(3))))
                .unwrap();
        });
        started_rx.recv().unwrap();
        assert!(matches!(
            done_rx.recv_timeout(Duration::from_millis(50)),
            Err(RecvTimeoutError::Timeout)
        ));
        writer.append_complete_record(&next_first).unwrap();
        assert_eq!(
            store
                .wait_for_new_header(
                    ControlDeadline::after(Duration::from_secs(1)).unwrap(),
                    &ControlCancellation::new(),
                )
                .unwrap(),
            next_first
        );
        store.commit_new_header_for_test(1).unwrap();
        done_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        blocked.join().unwrap();
        assert_eq!(store.durable_state().unwrap().phase, DurableRolloverPhase::Streaming);
        assert!(sealed_path(&root.0, 0).is_file());
        assert!(open_path(&root.0, 1).is_file());
        let set = store.seal_for_publication().unwrap();
        assert_eq!(set.ordered_streams.len(), 2);
        assert_eq!(set.transitions.len(), 1);
        assert_eq!(set.transitions[0].current_logical_stream_id, 0);
        assert_eq!(set.transitions[0].proposed_logical_stream_id, 1);
        assert_eq!(set.transitions[0].boundary_game_hour_raw, hour(2));
    }

    #[test]
    fn recovery_accepts_only_exact_topology_and_bytes() {
        let root = TestRoot::new();
        let (first, _) = cid1(-1234, hour(0));
        let (store, original_writer) =
            FilesystemRolloverStore::create_for_test(&root.0, &first).unwrap();
        drop(store);
        drop(original_writer);
        let (recovered, writer) = FilesystemRolloverStore::recover(&root.0).unwrap();
        assert_eq!(recovered.durable_state().unwrap().last_complete_sequence, Some(0));
        writer
            .append_complete_record(&ordinary_record(1, hour(1)))
            .unwrap();
        fs::write(sealed_path(&root.0, 0), b"ambiguous").unwrap();
        assert!(FilesystemRolloverStore::recover(&root.0).is_err());
    }

    #[test]
    fn completed_set_is_ordered_and_missing_descriptor_is_recovered_mechanically() {
        let root = TestRoot::new();
        let (first, _) = cid1(-1234, hour(0));
        let (mut store, writer) =
            FilesystemRolloverStore::create_for_test(&root.0, &first).unwrap();
        writer
            .append_complete_record(&ordinary_record(1, hour(1)))
            .unwrap();
        let published = store.seal_for_publication().unwrap();
        assert_eq!(published.ordered_streams.len(), 1);
        assert_eq!(published.ordered_streams[0].logical_stream_id, 0);
        assert_eq!(published.ordered_streams[0].last_complete_sequence, 1);
        assert!(sealed_path(&root.0, 0).is_file());
        assert!(!open_path(&root.0, 0).exists());

        fs::remove_file(root.0.join(SET_DESCRIPTOR_FILE)).unwrap();
        let recovered =
            FilesystemRolloverStore::recover_publication_descriptor(&root.0).unwrap();
        assert_eq!(recovered, published);
        assert!(root.0.join(SET_DESCRIPTOR_FILE).is_file());
    }

    #[test]
    fn immutable_package_descriptor_is_idempotent_but_never_replaceable() {
        let root = TestRoot::new();
        publish_immutable_descriptor(&root.0, PACKAGE_SET_DESCRIPTOR_FILE, b"one").unwrap();
        publish_immutable_descriptor(&root.0, PACKAGE_SET_DESCRIPTOR_FILE, b"one").unwrap();
        assert!(publish_immutable_descriptor(
            &root.0,
            PACKAGE_SET_DESCRIPTOR_FILE,
            b"two"
        )
        .is_err());
        assert_eq!(
            fs::read(root.0.join(PACKAGE_SET_DESCRIPTOR_FILE)).unwrap(),
            b"one"
        );
    }
}
