//! Durable append-only journal (`auto_attach_journal.jsonl`), design §8.
//!
//! One flushed record is written BEFORE each externally-visible action, so a
//! hard kill at any point leaves a replayable trail. On startup the journal is
//! replayed to reconcile every nonterminal identity with live processes /
//! worker leases (design §8, gate 10). Raw SID and pipe tokens are never
//! written — only their hashes (design §8).

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use snapshot::sha256::{to_hex, Sha256};

use super::identity::ProcessKey;
use super::state::{CandidateState, ReasonCode};

/// One journal record. Fields mirror the design §8 schema. Optional fields are
/// omitted (not null) when absent so a naive reader never mistakes a missing
/// value for a real one.
#[derive(Clone, Debug, PartialEq)]
pub struct JournalRecord {
    pub schema: &'static str,
    pub event_seq: u64,
    pub observed_at_utc_ms: u64,
    pub coordinator_instance_id: String,
    pub pid: u32,
    pub process_creation_filetime: u64,
    pub process_session_id: Option<u32>,
    /// SHA-256 hex of the user SID. NEVER the raw SID (design §8).
    pub process_user_sid_hash: Option<String>,
    pub exe_path: Option<String>,
    pub exe_sha256: Option<String>,
    pub attempt_id: Option<String>,
    pub state_from: Option<CandidateState>,
    pub state_to: CandidateState,
    pub reason_code: ReasonCode,
    pub reason_detail: Option<String>,
    pub staging_dir: Option<String>,
    pub run_dir: Option<String>,
    pub session_id: Option<String>,
    /// SHA-256 hex of the pipe token. NEVER the raw token (design §8).
    pub pipe_token_hash: Option<String>,
    pub first_observed_hour: Option<i64>,
    pub last_attempted_hour: Option<i64>,
    pub last_confirmed_hour: Option<i64>,
    pub artifact_status: Option<String>,
    pub replay_path: Option<String>,
    /// Hash of the preceding record, or all-zero for the genesis record.
    pub prev_record_hash: String,
    /// SHA-256 over `prev_record_hash || canonical_record_bytes`.
    pub record_hash: String,
}

impl JournalRecord {
    // v2 makes the predecessor/head hashes mandatory. Legacy v1 recovery fails
    // closed instead of silently treating an unauthenticated history as proof.
    pub const SCHEMA: &'static str = "auto_attach_journal.v2";

    /// Minimal constructor for a state-transition record; optional evidence is
    /// filled by the builder methods.
    pub fn transition(
        event_seq: u64,
        observed_at_utc_ms: u64,
        coordinator_instance_id: impl Into<String>,
        key: ProcessKey,
        state_from: Option<CandidateState>,
        state_to: CandidateState,
        reason_code: ReasonCode,
    ) -> Self {
        Self {
            schema: Self::SCHEMA,
            event_seq,
            observed_at_utc_ms,
            coordinator_instance_id: coordinator_instance_id.into(),
            pid: key.pid,
            process_creation_filetime: key.creation_filetime.0,
            process_session_id: None,
            process_user_sid_hash: None,
            exe_path: None,
            exe_sha256: None,
            attempt_id: None,
            state_from,
            state_to,
            reason_code,
            reason_detail: None,
            staging_dir: None,
            run_dir: None,
            session_id: None,
            pipe_token_hash: None,
            first_observed_hour: None,
            last_attempted_hour: None,
            last_confirmed_hour: None,
            artifact_status: None,
            replay_path: None,
            prev_record_hash: ZERO_HASH.to_string(),
            record_hash: String::new(),
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.reason_detail = Some(detail.into());
        self
    }

    pub fn with_attempt(mut self, attempt_id: impl Into<String>) -> Self {
        self.attempt_id = Some(attempt_id.into());
        self
    }

    pub fn key(&self) -> ProcessKey {
        ProcessKey::new(self.pid, self.process_creation_filetime)
    }

    fn canonical_json(&self) -> Value {
        let mut obj = json!({
            "schema": self.schema,
            "event_seq": self.event_seq,
            "observed_at_utc_ms": self.observed_at_utc_ms,
            "coordinator_instance_id": self.coordinator_instance_id,
            "pid": self.pid,
            "process_creation_filetime": self.process_creation_filetime,
            "state_to": format!("{:?}", self.state_to),
            "reason_code": self.reason_code.as_str(),
        });
        let map = obj.as_object_mut().expect("object");
        macro_rules! put_opt {
            ($field:expr, $key:literal) => {
                if let Some(v) = &$field {
                    map.insert($key.to_string(), json!(v));
                }
            };
        }
        put_opt!(self.process_session_id, "process_session_id");
        put_opt!(self.process_user_sid_hash, "process_user_sid_hash");
        put_opt!(self.exe_path, "exe_path");
        put_opt!(self.exe_sha256, "exe_sha256");
        put_opt!(self.attempt_id, "attempt_id");
        if let Some(s) = self.state_from {
            map.insert("state_from".to_string(), json!(format!("{s:?}")));
        }
        put_opt!(self.reason_detail, "reason_detail");
        put_opt!(self.staging_dir, "staging_dir");
        put_opt!(self.run_dir, "run_dir");
        put_opt!(self.session_id, "session_id");
        put_opt!(self.pipe_token_hash, "pipe_token_hash");
        put_opt!(self.first_observed_hour, "first_observed_hour");
        put_opt!(self.last_attempted_hour, "last_attempted_hour");
        put_opt!(self.last_confirmed_hour, "last_confirmed_hour");
        put_opt!(self.artifact_status, "artifact_status");
        put_opt!(self.replay_path, "replay_path");
        obj
    }

    fn computed_record_hash(&self) -> Result<String, String> {
        validate_hash("prev_record_hash", &self.prev_record_hash)?;
        let canonical = self.canonical_json().to_string();
        let mut hash = Sha256::new();
        hash.update(self.prev_record_hash.as_bytes());
        hash.update(canonical.as_bytes());
        Ok(to_hex(&hash.finalize()))
    }

    fn to_json(&self) -> Value {
        let mut value = self.canonical_json();
        let map = value.as_object_mut().expect("object");
        map.insert("prev_record_hash".to_string(), json!(self.prev_record_hash));
        let record_hash = if self.record_hash.is_empty() {
            self.computed_record_hash()
                .expect("constructor supplies a valid previous hash")
        } else {
            self.record_hash.clone()
        };
        map.insert("record_hash".to_string(), json!(record_hash));
        value
    }

    /// One JSONL line (no trailing newline).
    pub fn to_jsonl(&self) -> String {
        self.to_json().to_string()
    }

    /// Parse one JSONL line back to a record for recovery replay. Only the
    /// fields the coordinator needs to reconcile are strictly required; the
    /// rest are best-effort.
    pub fn from_jsonl(line: &str) -> Result<Self, String> {
        let v: Value = serde_json::from_str(line).map_err(|e| format!("bad journal line: {e}"))?;
        let object = v
            .as_object()
            .ok_or_else(|| "journal record is not a JSON object".to_string())?;
        let schema = object
            .get("schema")
            .and_then(Value::as_str)
            .ok_or_else(|| "journal record missing string schema".to_string())?;
        if schema != Self::SCHEMA {
            return Err(format!("unsupported journal schema {schema:?}"));
        }
        let get_u64 = |k: &str| -> Result<u64, String> {
            v.get(k)
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("journal record missing u64 {k:?}"))
        };
        let state_to = parse_state(
            v.get("state_to")
                .and_then(Value::as_str)
                .ok_or("journal record missing state_to")?,
        )?;
        let reason_code = v
            .get("reason_code")
            .and_then(Value::as_str)
            .ok_or("journal record missing reason_code")?;
        Ok(Self {
            schema: Self::SCHEMA,
            event_seq: get_u64("event_seq")?,
            observed_at_utc_ms: get_u64("observed_at_utc_ms")?,
            coordinator_instance_id: v
                .get("coordinator_instance_id")
                .and_then(Value::as_str)
                .ok_or("journal record missing coordinator_instance_id")?
                .to_string(),
            pid: u32::try_from(get_u64("pid")?)
                .map_err(|_| "journal pid exceeds u32".to_string())?,
            process_creation_filetime: get_u64("process_creation_filetime")?,
            process_session_id: opt_u32(&v, "process_session_id")?,
            process_user_sid_hash: opt_str(&v, "process_user_sid_hash")?,
            exe_path: opt_str(&v, "exe_path")?,
            exe_sha256: opt_str(&v, "exe_sha256")?,
            attempt_id: opt_str(&v, "attempt_id")?,
            state_from: match v.get("state_from") {
                None => None,
                Some(value) => {
                    Some(parse_state(value.as_str().ok_or_else(|| {
                        "journal state_from is not a string".to_string()
                    })?)?)
                }
            },
            state_to,
            reason_code: parse_reason(reason_code)?,
            reason_detail: opt_str(&v, "reason_detail")?,
            staging_dir: opt_str(&v, "staging_dir")?,
            run_dir: opt_str(&v, "run_dir")?,
            session_id: opt_str(&v, "session_id")?,
            pipe_token_hash: opt_str(&v, "pipe_token_hash")?,
            first_observed_hour: opt_i64(&v, "first_observed_hour")?,
            last_attempted_hour: opt_i64(&v, "last_attempted_hour")?,
            last_confirmed_hour: opt_i64(&v, "last_confirmed_hour")?,
            artifact_status: opt_str(&v, "artifact_status")?,
            replay_path: opt_str(&v, "replay_path")?,
            prev_record_hash: required_hash(&v, "prev_record_hash")?,
            record_hash: required_hash(&v, "record_hash")?,
        })
    }
}

const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn validate_hash(field: &str, value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("journal {field} is not lowercase SHA-256 hex"));
    }
    Ok(())
}

fn required_hash(v: &Value, key: &str) -> Result<String, String> {
    let value = v
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("journal record missing string {key}"))?;
    validate_hash(key, value)?;
    Ok(value.to_string())
}

fn opt_str(v: &Value, key: &str) -> Result<Option<String>, String> {
    match v.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(|s| Some(s.to_string()))
            .ok_or_else(|| format!("journal field {key:?} is not a string")),
    }
}

fn opt_u32(v: &Value, key: &str) -> Result<Option<u32>, String> {
    match v.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_u64()
            .ok_or_else(|| format!("journal field {key:?} is not a u32"))
            .and_then(|n| {
                u32::try_from(n)
                    .map(Some)
                    .map_err(|_| format!("journal field {key:?} exceeds u32"))
            }),
    }
}

fn opt_i64(v: &Value, key: &str) -> Result<Option<i64>, String> {
    match v.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("journal field {key:?} is not an i64")),
    }
}

fn parse_state(s: &str) -> Result<CandidateState, String> {
    use CandidateState::*;
    Ok(match s {
        "Detected" => Detected,
        "WaitingForIdentity" => WaitingForIdentity,
        "WaitingForWindow" => WaitingForWindow,
        "Preparing" => Preparing,
        "Injecting" => Injecting,
        "Handshaking" => Handshaking,
        "WaitingForWorld" => WaitingForWorld,
        "Recording" => Recording,
        "Finalizing" => Finalizing,
        "Complete" => Complete,
        "Failed" => Failed,
        "ProcessExitedBeforeAttach" => ProcessExitedBeforeAttach,
        other => return Err(format!("unknown state {other:?}")),
    })
}

fn parse_reason(s: &str) -> Result<ReasonCode, String> {
    use ReasonCode::*;
    Ok(match s {
        "detected" => Detected,
        "identity_unresolved" => IdentityUnresolved,
        // Spelled from the same constant the reason code writes, so the writer
        // and this reader cannot drift into a journal nobody can recover.
        other if other == crate::game_build::UnsupportedGameBuild::TERMINAL_OUTCOME => {
            UnsupportedGameBuild
        }
        "modset_unresolved" => ModsetUnresolved,
        "user_dir_ambiguous" => UserDirAmbiguous,
        "wrong_user_or_session" => WrongUserOrSession,
        "identity_mismatch_at_inject" => IdentityMismatchAtInject,
        "no_visible_window" => NoVisibleWindow,
        "preparing" => Preparing,
        "payload_hash_mismatch" => PayloadHashMismatch,
        "pipe_barrier_timeout" => PipeBarrierTimeout,
        "injected" => Injected,
        "heartbeat_timeout" => HeartbeatTimeout,
        "heartbeat_mismatch" => HeartbeatMismatch,
        "collector_attached" => CollectorAttached,
        "first_frame_valid" => FirstFrameValid,
        "recording_started" => RecordingStarted,
        "process_exited" => ProcessExited,
        "pipe_broken_while_alive" => PipeBrokenWhileAlive,
        "disk_failure" => DiskFailure,
        "finalize_verify_failed" => FinalizeVerifyFailed,
        "replay_complete" => ReplayComplete,
        "recovered_nonterminal_reinjection_blocked" => RecoveredNonterminalReinjectionBlocked,
        "collector_unloaded_rearmed" => CollectorUnloadedRearmed,
        "explicit_stop" => ExplicitStop,
        other => return Err(format!("unknown journal reason {other:?}")),
    })
}

const ANCHOR_SCHEMA: &str = "auto_attach_journal_anchor.v1";

#[derive(Clone, Debug, PartialEq, Eq)]
struct AnchorRecord {
    event_seq: u64,
    head_hash: String,
}

impl AnchorRecord {
    fn to_jsonl(&self) -> String {
        json!({
            "schema": ANCHOR_SCHEMA,
            "event_seq": self.event_seq,
            "head_hash": self.head_hash,
        })
        .to_string()
    }

    fn from_jsonl(line: &str) -> Result<Self, String> {
        let value: Value =
            serde_json::from_str(line).map_err(|e| format!("bad anchor line: {e}"))?;
        let object = value
            .as_object()
            .ok_or_else(|| "anchor record is not a JSON object".to_string())?;
        if object.get("schema").and_then(Value::as_str) != Some(ANCHOR_SCHEMA) {
            return Err("anchor record has unsupported schema".to_string());
        }
        let event_seq = object
            .get("event_seq")
            .and_then(Value::as_u64)
            .ok_or_else(|| "anchor record missing u64 event_seq".to_string())?;
        let head_hash = object
            .get("head_hash")
            .and_then(Value::as_str)
            .ok_or_else(|| "anchor record missing string head_hash".to_string())?;
        validate_hash("anchor head_hash", head_hash)?;
        Ok(Self {
            event_seq,
            head_hash: head_hash.to_string(),
        })
    }
}

/// The anchor is deliberately a different filesystem object. Journal-only
/// rollback/replacement therefore cannot erase an already-synced injection
/// commitment. Production adds two independent corroborators before any hook:
/// the stager's create-new PID+FILETIME commitment and target-module residency.
/// Rewriting both journal files alone therefore cannot authorize a second
/// production attempt; uncertainty in any layer is a refusal.
pub(crate) fn anchor_path(path: &Path) -> PathBuf {
    let mut anchored = path.as_os_str().to_os_string();
    anchored.push(".anchor");
    PathBuf::from(anchored)
}

fn strict_lines<'a>(text: &'a str, label: &str) -> Result<Vec<&'a str>, String> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    for (index, segment) in lines.iter().enumerate() {
        let line = segment.strip_suffix('\r').unwrap_or(segment);
        if line.trim().is_empty() {
            return Err(format!(
                "{label} line {} is empty/whitespace trailing non-record data",
                index + 1
            ));
        }
    }
    Ok(lines)
}

fn parse_records(bytes: &[u8], path: &Path) -> Result<Vec<JournalRecord>, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| format!("journal {} is not UTF-8: {e}", path.display()))?;
    let mut out = Vec::new();
    let mut expected_seq = 0u64;
    let mut expected_prev = ZERO_HASH.to_string();
    for (index, segment) in strict_lines(text, "journal")?.into_iter().enumerate() {
        let line = segment.strip_suffix('\r').unwrap_or(segment);
        let record = JournalRecord::from_jsonl(line)
            .map_err(|e| format!("journal line {} is invalid: {e}", index + 1))?;
        if record.to_jsonl() != line {
            return Err(format!(
                "journal line {} is non-canonical or contains unauthenticated bytes",
                index + 1
            ));
        }
        if record.event_seq != expected_seq {
            return Err(format!(
                "journal line {} sequence {} != expected {}",
                index + 1,
                record.event_seq,
                expected_seq
            ));
        }
        if record.prev_record_hash != expected_prev {
            return Err(format!(
                "journal line {} predecessor hash does not match retained chain head",
                index + 1
            ));
        }
        let computed = record.computed_record_hash()?;
        if record.record_hash != computed {
            return Err(format!("journal line {} record hash mismatch", index + 1));
        }
        expected_prev = record.record_hash.clone();
        expected_seq = expected_seq
            .checked_add(1)
            .ok_or_else(|| "journal sequence exhausted".to_string())?;
        out.push(record);
    }
    Ok(out)
}

fn read_records_from(file: &mut File, path: &Path) -> Result<Vec<JournalRecord>, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|e| format!("seek journal {}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("read journal {}: {e}", path.display()))?;
    parse_records(&bytes, path)
}

fn read_anchors_from(file: &mut File, path: &Path) -> Result<Vec<AnchorRecord>, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|e| format!("seek anchor {}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("read anchor {}: {e}", path.display()))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|e| format!("anchor {} is not UTF-8: {e}", path.display()))?;
    strict_lines(text, "anchor")?
        .into_iter()
        .enumerate()
        .map(|(index, segment)| {
            let line = segment.strip_suffix('\r').unwrap_or(segment);
            let anchor = AnchorRecord::from_jsonl(line)
                .map_err(|e| format!("anchor line {} is invalid: {e}", index + 1))?;
            if anchor.to_jsonl() != line {
                return Err(format!(
                    "anchor line {} is non-canonical or contains unauthenticated bytes",
                    index + 1
                ));
            }
            Ok(anchor)
        })
        .collect()
}

fn validate_anchor_pair(records: &[JournalRecord], anchors: &[AnchorRecord]) -> Result<(), String> {
    if records.len() != anchors.len() {
        return Err(format!(
            "journal/anchor length mismatch ({} records, {} anchors): rollback or incomplete commit",
            records.len(),
            anchors.len()
        ));
    }
    for (record, anchor) in records.iter().zip(anchors) {
        if anchor.event_seq != record.event_seq || anchor.head_hash != record.record_hash {
            return Err(format!(
                "journal/anchor mismatch at sequence {}: rollback or replacement detected",
                record.event_seq
            ));
        }
    }
    Ok(())
}

fn open_retained_append(path: &Path, label: &str) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).append(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        options.share_mode(FILE_SHARE_READ);
    }
    options
        .open(path)
        .map_err(|e| format!("open {label} {}: {e}", path.display()))
}

/// Append-only JSONL journal writer. Every append flushes to disk before the
/// caller performs the external action it describes.
pub struct Journal {
    path: PathBuf,
    file: File,
    anchor_path: PathBuf,
    anchor_file: File,
    next_seq: u64,
    head_hash: String,
    recovered_records: Vec<JournalRecord>,
    healthy: bool,
}

impl Journal {
    /// Open (creating if needed) the journal, scanning any existing content to
    /// continue the `event_seq` monotonically.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        let anchor_path = anchor_path(&path);
        // Open both objects with write/delete sharing denied BEFORE sampling
        // either. The validated bytes and their authority handles are retained
        // for the coordinator lifetime; recovery never re-opens a bare path.
        let mut file = open_retained_append(&path, "journal")?;
        let mut anchor_file = open_retained_append(&anchor_path, "journal anchor")?;
        let existing = read_records_from(&mut file, &path)?;
        let anchors = read_anchors_from(&mut anchor_file, &anchor_path)?;
        validate_anchor_pair(&existing, &anchors)?;
        let next_seq = existing.len() as u64;
        let head_hash = existing
            .last()
            .map(|record| record.record_hash.clone())
            .unwrap_or_else(|| ZERO_HASH.to_string());
        Ok(Self {
            path,
            file,
            anchor_path,
            anchor_file,
            next_seq,
            head_hash,
            recovered_records: existing,
            healthy: true,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub fn recovered_records(&self) -> &[JournalRecord] {
        &self.recovered_records
    }

    /// Append one record (its `event_seq` is overwritten with the next
    /// monotonic value), then flush. Returns the assigned seq.
    pub fn append(&mut self, mut record: JournalRecord) -> Result<u64, String> {
        if !self.healthy {
            return Err("journal writer is poisoned after a prior durability failure".to_string());
        }
        record.event_seq = self.next_seq;
        record.prev_record_hash = self.head_hash.clone();
        record.record_hash = record.computed_record_hash()?;
        let line = record.to_jsonl();
        let anchor = AnchorRecord {
            event_seq: record.event_seq,
            head_hash: record.record_hash.clone(),
        }
        .to_jsonl();
        let durable = (|| -> Result<(), String> {
            writeln!(self.file, "{line}").map_err(|e| format!("journal write: {e}"))?;
            self.file
                .flush()
                .map_err(|e| format!("journal flush: {e}"))?;
            self.file
                .sync_data()
                .map_err(|e| format!("journal sync_data: {e}"))?;
            // Journal first, anchor second. A crash/error between them yields a
            // loud length mismatch on restart, never a re-injection authority.
            writeln!(self.anchor_file, "{anchor}")
                .map_err(|e| format!("journal anchor write: {e}"))?;
            self.anchor_file
                .flush()
                .map_err(|e| format!("journal anchor flush: {e}"))?;
            self.anchor_file
                .sync_data()
                .map_err(|e| format!("journal anchor sync_data: {e}"))?;
            Ok(())
        })();
        if let Err(error) = durable {
            self.healthy = false;
            return Err(format!(
                "{error}; journal writer poisoned (anchor {})",
                self.anchor_path.display()
            ));
        }
        let assigned = self.next_seq;
        self.next_seq = self
            .next_seq
            .checked_add(1)
            .ok_or_else(|| "journal sequence exhausted".to_string())?;
        self.head_hash = record.record_hash.clone();
        self.recovered_records.push(record);
        Ok(assigned)
    }
}

/// Read every record from a journal file (for recovery). Missing file => empty.
pub fn read_all(path: impl AsRef<Path>) -> Result<Vec<JournalRecord>, String> {
    let path = path.as_ref();
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("read journal {}: {e}", path.display())),
    };
    parse_records(&bytes, path)
}

/// Result of replaying a journal: the last known state per identity and whether
/// it is terminal. Nonterminal identities need live reconciliation on startup.
#[derive(Clone, Debug, PartialEq)]
pub struct RecoveredIdentity {
    pub key: ProcessKey,
    pub last_state: CandidateState,
    pub injection_committed: bool,
    /// Staging directory for the latest attach generation. A recovered
    /// terminal generation may be rearmed only after its exact collector DLL is
    /// proven absent from the still-live target process.
    pub staging_dir: Option<PathBuf>,
}

/// Fold a journal's records into the last-known state per identity (design §8
/// "replay the journal and reconcile every nonterminal identity").
pub fn recover(records: &[JournalRecord]) -> Vec<RecoveredIdentity> {
    use std::collections::BTreeMap;
    let mut recovered: BTreeMap<
        ProcessKey,
        (u64, CandidateState, bool, Option<PathBuf>),
    > = BTreeMap::new();
    for r in records {
        let key = r.key();
        if recovered
            .get(&key)
            .is_some_and(|(seq, _, _, _)| *seq >= r.event_seq)
        {
            continue;
        }
        let previous = recovered.get(&key).cloned();
        let new_generation = r.reason_code == ReasonCode::CollectorUnloadedRearmed;
        let mut committed = previous.as_ref().is_some_and(|(_, _, value, _)| *value);
        let mut staging_dir = previous.and_then(|(_, _, _, path)| path);
        if new_generation {
            committed = false;
            staging_dir = None;
        }
        committed |= r.state_to.injection_committed();
        if let Some(path) = r.staging_dir.as_ref() {
            staging_dir = Some(PathBuf::from(path));
        }
        recovered.insert(
            key,
            (r.event_seq, r.state_to, committed, staging_dir),
        );
    }
    recovered
        .into_iter()
        .map(|(key, (_, last_state, injection_committed, staging_dir))| RecoveredIdentity {
            key,
            last_state,
            injection_committed,
            staging_dir,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_journal(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "auto_attach_journal_test_{name}_{}.jsonl",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(anchor_path(&p));
        p
    }

    #[test]
    fn record_roundtrips_through_jsonl() {
        let rec = JournalRecord::transition(
            0,
            1_700_000_000_000,
            "coord-abc",
            ProcessKey::new(42, 133_000_000_000_000_000),
            Some(CandidateState::WaitingForWindow),
            CandidateState::Preparing,
            ReasonCode::Preparing,
        )
        .with_attempt("att-1")
        .with_detail("staging fresh dll dir");
        let line = rec.to_jsonl();
        let back = JournalRecord::from_jsonl(&line).unwrap();
        assert_eq!(back.key(), rec.key());
        assert_eq!(back.state_to, CandidateState::Preparing);
        assert_eq!(back.state_from, Some(CandidateState::WaitingForWindow));
        assert_eq!(back.attempt_id.as_deref(), Some("att-1"));
    }

    #[test]
    fn never_writes_raw_secret_field_names() {
        // The schema exposes only *_hash fields; assert a populated record's
        // JSON has no bare `user_sid` or `pipe_token` key.
        let mut rec = JournalRecord::transition(
            0,
            0,
            "c",
            ProcessKey::new(1, 5),
            None,
            CandidateState::Detected,
            ReasonCode::Detected,
        );
        rec.process_user_sid_hash = Some("HASHED".to_string());
        rec.pipe_token_hash = Some("HASHED".to_string());
        let line = rec.to_jsonl();
        assert!(line.contains("process_user_sid_hash"));
        assert!(line.contains("pipe_token_hash"));
        assert!(!line.contains("\"user_sid\""));
        assert!(!line.contains("\"pipe_token\""));
        assert!(!line.contains("\"process_user_sid\":"));
    }

    #[test]
    fn append_flush_and_reopen_continues_seq() {
        let path = tmp_journal("seq");
        {
            let mut j = Journal::open(&path).unwrap();
            assert_eq!(j.next_seq(), 0);
            let s0 = j
                .append(JournalRecord::transition(
                    999, // overwritten
                    1,
                    "c",
                    ProcessKey::new(10, 100),
                    None,
                    CandidateState::Detected,
                    ReasonCode::Detected,
                ))
                .unwrap();
            assert_eq!(s0, 0);
            let s1 = j
                .append(JournalRecord::transition(
                    0,
                    2,
                    "c",
                    ProcessKey::new(10, 100),
                    Some(CandidateState::Detected),
                    CandidateState::WaitingForWindow,
                    ReasonCode::NoVisibleWindow,
                ))
                .unwrap();
            assert_eq!(s1, 1);
        }
        // Reopen: seq continues at 2.
        let j2 = portable_test_fs_retry::with_read_guard(&path, || Journal::open(&path))
            .expect("hold the journal open across the scanner's window")
            .unwrap();
        assert_eq!(j2.next_seq(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn recover_takes_last_state_and_tracks_commit() {
        let key = ProcessKey::new(7, 700);
        let recs = vec![
            JournalRecord::transition(
                0,
                1,
                "c",
                key,
                None,
                CandidateState::Detected,
                ReasonCode::Detected,
            ),
            JournalRecord::transition(
                1,
                2,
                "c",
                key,
                Some(CandidateState::Detected),
                CandidateState::WaitingForWindow,
                ReasonCode::NoVisibleWindow,
            ),
            JournalRecord::transition(
                2,
                3,
                "c",
                key,
                Some(CandidateState::WaitingForWindow),
                CandidateState::Injecting,
                ReasonCode::Injected,
            ),
        ];
        let rec = recover(&recs);
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].last_state, CandidateState::Injecting);
        assert!(
            rec[0].injection_committed,
            "Injecting is past the commit line"
        );
    }

    #[test]
    fn recovered_rearm_starts_a_new_injection_generation() {
        let key = ProcessKey::new(8, 800);
        let mut injected = JournalRecord::transition(
            0,
            1,
            "old",
            key,
            None,
            CandidateState::Injecting,
            ReasonCode::Injected,
        );
        injected.staging_dir = Some(r"C:\stage\old".to_string());
        let failed = JournalRecord::transition(
            1,
            2,
            "old",
            key,
            Some(CandidateState::Finalizing),
            CandidateState::Failed,
            ReasonCode::FinalizeVerifyFailed,
        );
        let rearmed = JournalRecord::transition(
            2,
            3,
            "new",
            key,
            Some(CandidateState::Failed),
            CandidateState::Detected,
            ReasonCode::CollectorUnloadedRearmed,
        );
        let recovered = recover(&[injected, failed, rearmed]);
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].last_state, CandidateState::Detected);
        assert!(!recovered[0].injection_committed);
        assert!(recovered[0].staging_dir.is_none());
    }

    #[test]
    fn recover_two_identities_independent() {
        let a = ProcessKey::new(1, 10);
        let b = ProcessKey::new(1, 20); // reused pid, new identity
        let recs = vec![
            JournalRecord::transition(
                0,
                1,
                "c",
                a,
                None,
                CandidateState::Detected,
                ReasonCode::Detected,
            ),
            JournalRecord::transition(
                1,
                2,
                "c",
                a,
                Some(CandidateState::Detected),
                CandidateState::Complete,
                ReasonCode::ReplayComplete,
            ),
            JournalRecord::transition(
                2,
                3,
                "c",
                b,
                None,
                CandidateState::Detected,
                ReasonCode::Detected,
            ),
        ];
        let mut rec = recover(&recs);
        rec.sort_by_key(|r| r.key.creation_filetime.0);
        assert_eq!(rec.len(), 2);
        assert_eq!(rec[0].last_state, CandidateState::Complete);
        assert_eq!(rec[1].last_state, CandidateState::Detected);
    }

    #[test]
    fn nonempty_truncated_final_line_blocks_recovery() {
        let path = tmp_journal("trunc");
        {
            let mut j = Journal::open(&path).unwrap();
            j.append(JournalRecord::transition(
                0,
                1,
                "c",
                ProcessKey::new(3, 30),
                None,
                CandidateState::Detected,
                ReasonCode::Detected,
            ))
            .unwrap();
        }
        // Simulate a crash mid-append: append a partial line.
        {
            use std::io::Write as _;
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            write!(
                f,
                "{{\"schema\":\"auto_attach_journal.v1\",\"event_seq\":1,\"pi"
            )
            .unwrap();
        }
        let err = read_all(&path).unwrap_err();
        assert!(err.contains("line 2 is invalid"), "{err}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn malformed_sole_injecting_record_blocks_open() {
        let path = tmp_journal("sole_injecting_malformed");
        std::fs::write(
            &path,
            b"{\"schema\":\"auto_attach_journal.v1\",\"event_seq\":0,\"state_to\":\"Injecting\"",
        )
        .unwrap();
        let err = portable_test_fs_retry::with_read_guard(&path, || Journal::open(&path))
            .expect("hold the journal open across the scanner's window")
            .err()
            .expect("malformed recovery must fail");
        assert!(err.contains("line 1 is invalid"), "{err}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sequence_gap_blocks_recovery() {
        let path = tmp_journal("sequence_gap");
        let mut record = JournalRecord::transition(
            0,
            1,
            "c",
            ProcessKey::new(1, 2),
            None,
            CandidateState::Detected,
            ReasonCode::Detected,
        );
        record.event_seq = 1;
        std::fs::write(&path, format!("{}\n", record.to_jsonl())).unwrap();
        let err = read_all(&path).unwrap_err();
        assert!(err.contains("sequence 1 != expected 0"), "{err}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn synced_injecting_record_cannot_be_erased_by_valid_prefix_rollback() {
        let path = tmp_journal("rollback_after_injecting");
        let key = ProcessKey::new(70, 700);
        let prefix = {
            let mut journal = Journal::open(&path).unwrap();
            journal
                .append(JournalRecord::transition(
                    0,
                    1,
                    "c",
                    key,
                    None,
                    CandidateState::Detected,
                    ReasonCode::Detected,
                ))
                .unwrap();
            let prefix = portable_test_fs_retry::read(&path).unwrap();
            journal
                .append(JournalRecord::transition(
                    0,
                    2,
                    "c",
                    key,
                    Some(CandidateState::Preparing),
                    CandidateState::Injecting,
                    ReasonCode::Injected,
                ))
                .unwrap();
            prefix
        };

        std::fs::write(&path, prefix).unwrap();
        let err = portable_test_fs_retry::with_read_guard(&path, || Journal::open(&path))
            .expect("hold the journal open across the scanner's window")
            .err()
            .expect("rollback must make restart fail closed");
        assert!(err.contains("anchor") || err.contains("rollback"), "{err}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn whitespace_after_committed_record_is_tampering() {
        let path = tmp_journal("whitespace_after_commit");
        {
            let mut journal = Journal::open(&path).unwrap();
            journal
                .append(JournalRecord::transition(
                    0,
                    1,
                    "c",
                    ProcessKey::new(71, 701),
                    Some(CandidateState::Preparing),
                    CandidateState::Injecting,
                    ReasonCode::Injected,
                ))
                .unwrap();
        }
        let mut file =
            portable_test_fs_retry::open_with(OpenOptions::new().append(true), &path).unwrap();
        write!(file, " \t\r\n").unwrap();
        drop(file);

        let err = portable_test_fs_retry::with_read_guard(&path, || Journal::open(&path))
            .expect("hold the appended journal open across the scanner's window")
            .err()
            .expect("trailing non-record bytes must fail closed");
        assert!(err.contains("trailing") || err.contains("line 2"), "{err}");
        let _ = std::fs::remove_file(&path);
    }
}
