//! Typed H4CT status proof shared by the supervisor and its worker child.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::control_transport::{
    claim_session_capability, ControlCapabilities, ControlChannel, ControlIo, MessageKind,
    CONTROL_VERSION,
};

const PAYLOAD_SCHEMA: u16 = 1;
// Pinned to the shared codec's constants so the recorder's admission lengths
// and the collector client's shared encoder can never drift.
const COLLECTOR_HELLO_LEN: usize =
    compact_contract::control_transport::payloads::COLLECTOR_HELLO_LEN;
const COLLECTOR_HELLO_WITH_CAPABILITIES_LEN: usize =
    compact_contract::control_transport::payloads::COLLECTOR_HELLO_WITH_CAPABILITIES_LEN;
const HEARTBEAT_PROOF_LEN: usize = 32;
const HEARTBEAT_STATE_PRODUCING: u16 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofExpectation {
    pub pid: u32,
    pub creation_filetime: u64,
    pub h4cr_version: u16,
    pub dll_sha256: [u8; 32],
    pub challenge: [u8; 16],
}

pub fn decode_hex_16(value: &str) -> Result<[u8; 16], String> {
    let bytes = decode_hex(value, 16)?;
    Ok(bytes.try_into().expect("length checked"))
}

pub fn decode_hex_32(value: &str) -> Result<[u8; 32], String> {
    let bytes = decode_hex(value, 32)?;
    Ok(bytes.try_into().expect("length checked"))
}

pub fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn decode_hex(value: &str, byte_len: usize) -> Result<Vec<u8>, String> {
    if value.len() != byte_len * 2 {
        return Err(format!("expected {} hex characters", byte_len * 2));
    }
    let raw = value.as_bytes();
    let mut out = Vec::with_capacity(byte_len);
    for pair in raw.chunks_exact(2) {
        let high = hex_nibble(pair[0]).ok_or_else(|| "invalid hex digit".to_string())?;
        let low = hex_nibble(pair[1]).ok_or_else(|| "invalid hex digit".to_string())?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub fn collector_hello_payload(expectation: &ProofExpectation) -> Vec<u8> {
    // Single implementation of the H4CT payload layout lives in the shared
    // `compact_contract` codec; this stays a thin, byte-identical adapter over
    // the recorder's `ProofExpectation` so the collector and recorder can never
    // encode a different CollectorHello.
    compact_contract::control_transport::payloads::encode_collector_hello(
        &compact_contract::control_transport::payloads::CollectorHelloV1 {
            pid: expectation.pid,
            creation_filetime: expectation.creation_filetime,
            h4cr_version: expectation.h4cr_version,
            control_version: CONTROL_VERSION,
            dll_sha256: expectation.dll_sha256,
            challenge: expectation.challenge,
        },
    )
}

pub fn validate_collector_hello(
    payload: &[u8],
    expectation: &ProofExpectation,
) -> Result<(), String> {
    if payload != collector_hello_payload(expectation) {
        return Err("collector hello did not exactly match expected attempt".to_string());
    }
    Ok(())
}

/// Encode the duplex hello form whose final word is the collector's closed H4CT
/// capability claim. The durable supervisor-status adapter uses the historical
/// 68-byte forwarded proof; the combined H4CR/H4CT worker consumes this exact
/// 72-byte product-admission form.
pub fn collector_hello_with_capabilities_payload(
    expectation: &ProofExpectation,
    capabilities: ControlCapabilities,
) -> Vec<u8> {
    // Byte-identical adapter over the shared H4CT codec: the collector client
    // encodes its hello through the exact same function, so the two endpoints
    // cannot produce different admission bytes.
    compact_contract::control_transport::payloads::encode_collector_hello_with_capabilities(
        &compact_contract::control_transport::payloads::CollectorHelloV1 {
            pid: expectation.pid,
            creation_filetime: expectation.creation_filetime,
            h4cr_version: expectation.h4cr_version,
            control_version: CONTROL_VERSION,
            dll_sha256: expectation.dll_sha256,
            challenge: expectation.challenge,
        },
        capabilities.bits(),
    )
}

/// Decode and bind a collector-originated hello to the retained attempt facts.
/// Capabilities are never accepted as an out-of-band caller argument: the
/// exact bytes authenticated by H4CT framing are the only source of the claim.
pub fn decode_collector_hello_capabilities(
    payload: &[u8],
    expectation: &ProofExpectation,
) -> Result<ControlCapabilities, String> {
    if payload.len() != COLLECTOR_HELLO_WITH_CAPABILITIES_LEN {
        return Err(format!(
            "collector hello length {} != {}",
            payload.len(),
            COLLECTOR_HELLO_WITH_CAPABILITIES_LEN
        ));
    }
    if payload[..COLLECTOR_HELLO_LEN] != collector_hello_payload(expectation) {
        return Err("collector hello did not exactly match expected attempt".to_string());
    }
    let bits = u32::from_le_bytes(
        payload[COLLECTOR_HELLO_LEN..COLLECTOR_HELLO_WITH_CAPABILITIES_LEN]
            .try_into()
            .expect("fixed capability word"),
    );
    ControlCapabilities::from_bits(bits).map_err(|error| error.to_string())
}

pub fn heartbeat_proof_payload(expectation: &ProofExpectation, sequence: u64) -> Vec<u8> {
    // Byte-identical adapter over the shared H4CT codec (see above).
    compact_contract::control_transport::payloads::encode_heartbeat_proof(
        &compact_contract::control_transport::payloads::HeartbeatProofV1 {
            challenge: expectation.challenge,
            last_h4cr_sequence: sequence,
            state: compact_contract::control_transport::payloads::HEARTBEAT_STATE_PRODUCING,
        },
    )
}

pub fn validate_heartbeat_proof(
    payload: &[u8],
    expectation: &ProofExpectation,
) -> Result<u64, String> {
    if payload.len() != HEARTBEAT_PROOF_LEN
        || payload[0..2] != PAYLOAD_SCHEMA.to_le_bytes()
        || payload[2..4] != [0, 0]
        || payload[4..20] != expectation.challenge
        || payload[28..30] != HEARTBEAT_STATE_PRODUCING.to_le_bytes()
        || payload[30..32] != [0, 0]
    {
        return Err("invalid heartbeat/first-frame proof".to_string());
    }
    Ok(u64::from_le_bytes(payload[20..28].try_into().unwrap()))
}

pub struct ReadControlIo<R>(pub R);

impl<R: Read> ControlIo for ReadControlIo<R> {
    fn send_all(&mut self, _bytes: &[u8]) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "read-only control endpoint",
        ))
    }
    fn recv_exact(&mut self, bytes: &mut [u8]) -> io::Result<()> {
        self.0.read_exact(bytes)
    }
}

pub struct WriteControlIo<W>(pub W);

impl<W: Write> ControlIo for WriteControlIo<W> {
    fn send_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes)?;
        self.0.flush()
    }
    fn recv_exact(&mut self, _bytes: &mut [u8]) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "write-only control endpoint",
        ))
    }
}

pub fn recorder_channel<R: Read>(
    reader: R,
    session_id: [u8; 16],
) -> Result<ControlChannel<ReadControlIo<R>>, String> {
    let capability = claim_session_capability(session_id).map_err(|e| e.to_string())?;
    Ok(ControlChannel::new(
        ReadControlIo(reader),
        crate::control_transport::EndpointRole::Recorder,
        capability,
    ))
}

pub struct WorkerProofEmitter<W: Write> {
    channel: ControlChannel<WriteControlIo<W>>,
    expectation: ProofExpectation,
    hello_emitted: bool,
    first_record_emitted: bool,
    last_reported_sequence: Option<u64>,
}

fn monotonic_worker_proof_sequence(previous: Option<u64>, observed: u64) -> u64 {
    previous.map_or(observed, |previous| previous.max(observed))
}

impl<W: Write> WorkerProofEmitter<W> {
    pub fn new(
        writer: W,
        session_id: [u8; 16],
        expectation: ProofExpectation,
    ) -> Result<Self, String> {
        let capability = claim_session_capability(session_id).map_err(|e| e.to_string())?;
        Ok(Self {
            channel: ControlChannel::new(
                WriteControlIo(writer),
                crate::control_transport::EndpointRole::Collector,
                capability,
            ),
            expectation,
            hello_emitted: false,
            first_record_emitted: false,
            last_reported_sequence: None,
        })
    }

    /// A successful connection to the coordinator-minted, per-attempt data pipe
    /// is the residency hello. `loaded_dll_sha256` is the retained backing-file
    /// hash and may be emitted only after direct target-memory immutable-image
    /// proof matched the locally loaded module; it is never a cached assertion.
    /// No hook return or child liveness alone can call this.
    pub fn report_connected(&mut self, loaded_dll_sha256: [u8; 32]) -> Result<(), String> {
        if self.hello_emitted {
            return Err("collector hello already emitted".to_string());
        }
        if loaded_dll_sha256 != self.expectation.dll_sha256 {
            return Err(
                "observed target DLL hash does not match the retained expectation".to_string(),
            );
        }
        let mut observed = self.expectation.clone();
        observed.dll_sha256 = loaded_dll_sha256;
        self.channel
            .send(
                MessageKind::CollectorHello,
                &collector_hello_payload(&observed),
            )
            .map_err(|e| e.to_string())?;
        self.expectation = observed;
        self.hello_emitted = true;
        Ok(())
    }

    /// Report only after the complete first H4CR header plus payload was read.
    pub fn report_first_complete_record(&mut self, sequence: u64) -> Result<(), String> {
        if !self.hello_emitted {
            return Err("first-record proof cannot precede collector hello".to_string());
        }
        if self.first_record_emitted {
            return Err("first-record proof already emitted".to_string());
        }
        self.channel
            .send(
                MessageKind::HeartbeatProof,
                &heartbeat_proof_payload(&self.expectation, sequence),
            )
            .map_err(|e| e.to_string())?;
        self.first_record_emitted = true;
        self.last_reported_sequence = Some(sequence);
        Ok(())
    }

    /// Forward a recorder-authenticated recurring H4CT heartbeat to the
    /// supervisor after Recording admission. The durable file is only a
    /// parent/child status transport; the proof's authority still comes from
    /// the admitted collector H4CT exchange.
    pub fn report_recurring_heartbeat(&mut self, sequence: u64) -> Result<(), String> {
        if !self.first_record_emitted {
            return Err("recurring heartbeat cannot precede first-record proof".to_string());
        }
        // The first-world data reader can outrun the collector's frame-level
        // progress publication, and an authenticated logical-stream rollover
        // intentionally resets the exact H4CR sequence to zero. H4CT validates
        // those exact per-stream values before they reach this bridge. The
        // supervisor's durable status stream has no logical-stream-id field, so
        // publish a process-lifetime high-water mark rather than making either
        // legitimate reset look like a forged regression.
        let sequence = monotonic_worker_proof_sequence(self.last_reported_sequence, sequence);
        self.channel
            .send(
                MessageKind::HeartbeatProof,
                &heartbeat_proof_payload(&self.expectation, sequence),
            )
            .map_err(|error| error.to_string())?;
        self.last_reported_sequence = Some(sequence);
        Ok(())
    }
}

/// Per-attempt, recorder-owned status stream used between the worker process
/// and the resident supervisor.  H4CT remains the authority for the contained
/// hello/heartbeat facts; this file makes their delivery durable across a
/// supervisor crash and removes stdout from the product protocol.
pub const WORKER_PROOF_FILE_NAME: &str = "worker_status.h4ct";
pub const WORKER_PROOF_PATH_ENV: &str = "BATTLE_PLAN_AUTO_ATTACH_PROOF_PATH";

pub struct DurableProofWriter {
    file: File,
}

impl DurableProofWriter {
    /// Open only the exact recorder-created file inside this attempt's output
    /// directory.  The supervisor retains a no-delete handle from before child
    /// creation, so this open cannot be redirected by path replacement.
    pub fn open_existing(path: &Path, out_dir: &Path) -> Result<Self, String> {
        let expected = out_dir.join(WORKER_PROOF_FILE_NAME);
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("stat worker proof file failed: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != 0 {
            return Err(
                "worker proof file must be the empty regular file created by the supervisor"
                    .to_string(),
            );
        }
        let canonical_out = fs::canonicalize(out_dir)
            .map_err(|error| format!("canonicalize worker output directory failed: {error}"))?;
        let canonical_path = fs::canonicalize(path)
            .map_err(|error| format!("canonicalize worker proof file failed: {error}"))?;
        let canonical_expected = fs::canonicalize(&expected)
            .map_err(|error| format!("canonicalize expected worker proof file failed: {error}"))?;
        if canonical_path != canonical_expected
            || canonical_path.parent() != Some(canonical_out.as_path())
        {
            return Err("worker proof file escaped its attempt output directory".to_string());
        }
        let mut options = OpenOptions::new();
        options.append(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Share read/write with the retained supervisor handle, but deny
            // delete/rename for the lifetime of this writer.
            const FILE_SHARE_READ: u32 = 0x0000_0001;
            const FILE_SHARE_WRITE: u32 = 0x0000_0002;
            options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
        }
        let file = options
            .open(&canonical_path)
            .map_err(|error| format!("open durable worker proof sink failed: {error}"))?;
        Ok(Self { file })
    }
}

impl Write for DurableProofWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.file.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()?;
        self.file.sync_data()
    }
}

/// Tail reader for the retained supervisor handle.  Regular-file EOF means
/// "not appended yet", not terminal EOF.  The owning child handle sets `stop`
/// only after process exit or teardown; already-appended bytes are consumed
/// before the reader returns EOF, so exit cannot race the final proof frame.
pub struct DurableProofTail {
    file: File,
    stop: Arc<AtomicBool>,
}

impl DurableProofTail {
    pub fn new(file: File, stop: Arc<AtomicBool>) -> Self {
        Self { file, stop }
    }
}

impl Read for DurableProofTail {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            let read = self.file.read(bytes)?;
            if read != 0 {
                return Ok(read);
            }
            if self.stop.load(Ordering::Acquire) {
                return Ok(0);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Create the durable status file before the worker process exists and retain
/// the read handle with delete sharing denied. `create_new` makes stale residue
/// fail closed instead of appending to another attempt's transcript.
pub fn create_durable_proof_file(path: PathBuf) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_SHARE_WRITE: u32 = 0x0000_0002;
        options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    }
    let file = options
        .open(&path)
        .map_err(|error| format!("create durable worker proof file failed: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("sync durable worker proof file failed: {error}"))?;
    Ok(file)
}

/// The generic PERF envelope immediately preceding a candidate world DATA
/// record. PERF alone is telemetry and can never prove Recording.
#[derive(Clone, Copy)]
struct DomainPerfEvidence {
    header: compact_contract::CompactRecordHeader,
    elapsed_wall_ms: u64,
    row_count: u32,
    changed_rows: u32,
    unit_count: u32,
    changed_unit_count: u32,
    status_flags: u32,
}

/// Opaque authority that a complete H4CR PERF/DATA pair passed the production
/// first-world-record gate. It is deliberately non-`Clone`: the duplex state
/// transition consumes the one proof produced by the stream validator.
#[derive(Debug, PartialEq, Eq)]
pub struct ValidatedCompactRecordProof {
    header: compact_contract::CompactRecordHeader,
    confirmed_hour: i64,
}

impl ValidatedCompactRecordProof {
    pub const fn sequence(&self) -> u64 {
        self.header.sequence
    }

    pub const fn game_hour_raw(&self) -> i32 {
        self.header.game_hour_raw
    }

    pub const fn confirmed_hour(&self) -> i64 {
        self.confirmed_hour
    }

    pub const fn domain_id(&self) -> u16 {
        self.header.domain_id
    }

    #[cfg(test)]
    pub(crate) fn for_test(game_hour_raw: i32) -> Self {
        Self {
            header: compact_contract::CompactRecordHeader {
                sequence: 1,
                game_hour_raw,
                domain_id: 10,
                record_type: compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
                payload_len: 0,
                record_flags: 0,
            },
            confirmed_hour: i64::from(game_hour_raw),
        }
    }
}

const FIRST_PROOF_KNOWN_STATUS_MASK: u32 = (1 << 14) - 1;

/// Incremental, bounded gate for the first semantically valid world record.
/// A proof requires a clean full-baseline PERF -> DATA pair for an actually
/// scheduled domain, a plausible Clausewitz clock, an explicit domain-correct
/// real-material predicate, and a payload accepted by the production world
/// fold. Domains without a vetted predicate are insufficient proof and leave
/// the gate waiting.
///
/// Threat-model boundary: arbitrary code already executing inside the retained
/// the game process can read the staged session secret and originate the pipe
/// stream, while truly simultaneous equal-rights external injection can race
/// the final module-absence check. Those residuals are explicitly out of the
/// proof's threat model and require an architectural trust boundary or OS code
/// integrity policy; they are not described as fail-closed or repairable with
/// another in-process nonce or HMAC.
pub struct FirstCompactRecordGate {
    buffered: Vec<u8>,
    expected_domain_ids: BTreeSet<u16>,
    last_sequence: Option<u64>,
    /// The collector emits the complete PERF roster before the complete DATA
    /// roster for an hour. Retain at most one clean PERF record per expected
    /// domain until its matching DATA record or the canonical FRAME_END.
    pending_perf: BTreeMap<u16, DomainPerfEvidence>,
}

impl Default for FirstCompactRecordGate {
    fn default() -> Self {
        Self::new(
            compact_contract::ASSIGNED_COMPACT_DOMAINS
                .iter()
                .filter(|descriptor| descriptor.schedulable && descriptor.requires_data_record)
                .map(|descriptor| descriptor.id),
        )
    }
}

impl FirstCompactRecordGate {
    pub fn new(expected_domain_ids: impl IntoIterator<Item = u16>) -> Self {
        Self {
            buffered: Vec::new(),
            expected_domain_ids: expected_domain_ids.into_iter().collect(),
            last_sequence: None,
            pending_perf: BTreeMap::new(),
        }
    }

    pub fn observe(&mut self, chunk: &[u8]) -> Result<Option<ValidatedCompactRecordProof>, String> {
        let buffer_limit = compact_contract::COMPACT_MAX_PAYLOAD_LEN as usize
            + compact_contract::COMPACT_RECORD_HEADER_LEN
            + 65_536;
        if self.buffered.len().saturating_add(chunk.len()) > buffer_limit {
            return Err(format!(
                "first world-record proof exceeded the bounded {}-byte buffer",
                buffer_limit
            ));
        }
        self.buffered.extend_from_slice(chunk);

        loop {
            if self.buffered.len() < compact_contract::COMPACT_RECORD_HEADER_LEN {
                return Ok(None);
            }
            let header = compact_contract::parse_compact_record_header(&self.buffered)
                .map_err(|e| format!("first world-record proof rejected framing: {e}"))?;
            let record_len = header.record_len();
            if self.buffered.len() < record_len {
                return Ok(None);
            }
            if compact_contract::calendar::hour_index_from_raw(header.game_hour_raw).is_none() {
                return Err(format!(
                    "first world-record proof rejected implausible game clock {}",
                    header.game_hour_raw
                ));
            }
            match self.last_sequence {
                None if header.sequence != 1 => {
                    return Err(format!(
                        "first world-record proof rejected initial sequence {}; expected 1",
                        header.sequence
                    ));
                }
                Some(previous) => {
                    let expected = previous.checked_add(1).ok_or_else(|| {
                        "first world-record proof rejected exhausted sequence space".to_string()
                    })?;
                    if header.sequence != expected {
                        return Err(format!(
                            "first world-record proof rejected sequence {} after {}",
                            header.sequence, previous
                        ));
                    }
                }
                None => {}
            }
            let remaining = self.buffered.split_off(record_len);
            let record = std::mem::replace(&mut self.buffered, remaining);
            let payload = &record[compact_contract::COMPACT_RECORD_HEADER_LEN..];
            compact_contract::validate_compact_record_semantics(
                header.domain_id,
                header.record_type,
                header.record_flags,
                payload,
            )
            .map_err(|error| {
                format!("first world-record proof rejected record semantics: {error}")
            })?;
            let proof = self.observe_complete_record(header, payload)?;
            self.last_sequence = Some(header.sequence);
            if proof {
                let confirmed_hour =
                    compact_contract::calendar::hour_index_from_raw(header.game_hour_raw)
                        .expect("plausible clock checked before proof construction");
                return Ok(Some(ValidatedCompactRecordProof {
                    header,
                    confirmed_hour,
                }));
            }
        }
    }

    fn observe_complete_record(
        &mut self,
        header: compact_contract::CompactRecordHeader,
        payload: &[u8],
    ) -> Result<bool, String> {
        match header.record_type {
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF => {
                // Always discard an older candidate for this domain first. An
                // invalid/non-baseline PERF record must never leave a previous
                // clean candidate available for a later DATA record.
                self.pending_perf.remove(&header.domain_id);
                if let Some(evidence) =
                    parse_domain_perf_evidence(header, payload, &self.expected_domain_ids)?
                {
                    self.pending_perf.insert(header.domain_id, evidence);
                }
                Ok(false)
            }
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA => {
                let evidence = self.pending_perf.remove(&header.domain_id);
                let Some(evidence) = evidence else {
                    return Ok(false);
                };
                if evidence.header.domain_id != header.domain_id
                    || evidence.header.game_hour_raw != header.game_hour_raw
                    || evidence.header.sequence >= header.sequence
                {
                    return Ok(false);
                }
                if !data_record_matches_perf(header, payload, evidence)? {
                    return Ok(false);
                }
                world_fold_proves_material_record(header, payload, evidence.elapsed_wall_ms)
            }
            compact_contract::COMPACT_RECORD_TYPE_FRAME_END => {
                self.pending_perf.clear();
                Ok(false)
            }
            other => Err(format!(
                "first world-record proof rejected unknown record type {other}"
            )),
        }
    }
}

fn parse_domain_perf_evidence(
    header: compact_contract::CompactRecordHeader,
    payload: &[u8],
    expected_domain_ids: &BTreeSet<u16>,
) -> Result<Option<DomainPerfEvidence>, String> {
    const BASE_LEN: usize = 40;
    const TRAILER_HEADER_LEN: usize = 8;
    const TRAILER_UNIT_HEADER_LEN: usize = 12;

    if payload.len() < BASE_LEN {
        return Err(format!(
            "first world-record proof rejected domain {} PERF length {}",
            header.domain_id,
            payload.len()
        ));
    }
    let allowed_record_flags = compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER;
    if header.record_flags & !allowed_record_flags != 0 {
        return Err(format!(
            "first world-record proof rejected domain {} PERF record flags {:#x}",
            header.domain_id, header.record_flags
        ));
    }
    let has_trailer =
        header.record_flags & compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER != 0;
    if !has_trailer && payload.len() != BASE_LEN {
        return Err(format!(
            "first world-record proof rejected unannounced domain {} PERF trailer",
            header.domain_id
        ));
    }
    if read_u32(payload, 36) != 0 {
        return Err(format!(
            "first world-record proof rejected domain {} PERF reserved word",
            header.domain_id
        ));
    }
    if has_trailer {
        if payload.len() < BASE_LEN + TRAILER_HEADER_LEN
            || &payload[BASE_LEN..BASE_LEN + 4] != b"PDU1"
            || read_u16(payload, BASE_LEN + 4) != 1
        {
            return Err(format!(
                "first world-record proof rejected domain {} PERF diagnostic trailer",
                header.domain_id
            ));
        }
        let unit_count = read_u16(payload, BASE_LEN + 6) as usize;
        let mut cursor = BASE_LEN + TRAILER_HEADER_LEN;
        for _ in 0..unit_count {
            if payload.len().saturating_sub(cursor) < TRAILER_UNIT_HEADER_LEN {
                return Err(format!(
                    "first world-record proof rejected torn domain {} PERF diagnostic unit",
                    header.domain_id
                ));
            }
            let name_len = read_u16(payload, cursor) as usize;
            if name_len == 0 || read_u16(payload, cursor + 2) != 0 {
                return Err(format!(
                    "first world-record proof rejected malformed domain {} PERF diagnostic unit",
                    header.domain_id
                ));
            }
            cursor = cursor
                .checked_add(TRAILER_UNIT_HEADER_LEN)
                .and_then(|value| value.checked_add(name_len))
                .ok_or_else(|| "first world-record proof PERF trailer overflow".to_string())?;
            if cursor > payload.len() {
                return Err(format!(
                    "first world-record proof rejected torn domain {} PERF diagnostic name",
                    header.domain_id
                ));
            }
        }
        if cursor != payload.len() {
            return Err(format!(
                "first world-record proof rejected trailing domain {} PERF bytes",
                header.domain_id
            ));
        }
    }

    let descriptor = compact_contract::assigned_descriptor_by_id(header.domain_id);
    let scheduled_data_domain = descriptor
        .map(|descriptor| descriptor.schedulable && descriptor.requires_data_record)
        .unwrap_or(false)
        && expected_domain_ids.contains(&header.domain_id);
    let row_count = read_u32(payload, 16);
    let changed_rows = read_u32(payload, 20);
    let unit_count = read_u32(payload, 24);
    let changed_unit_count = read_u32(payload, 28);
    let status_flags = read_u32(payload, 32);
    let global_status =
        compact_contract::compact_status_for_global_interpretation(header.domain_id, status_flags);
    let clean_baseline = status_flags & !FIRST_PROOF_KNOWN_STATUS_MASK == 0
        && !compact_contract::compact_status_is_failure(global_status)
        && global_status & compact_contract::COMPACT_STATUS_DELTA_DATA == 0;
    // These counters are domain-specific (some count roots, others child
    // units), so cross-counter ordering is not a valid generic invariant.
    // Requiring each to be nonzero rejects empty/degenerate telemetry while
    // the production fold below owns the domain-specific structural checks.
    let plausible_counts =
        row_count > 0 && changed_rows > 0 && unit_count > 0 && changed_unit_count > 0;

    if !scheduled_data_domain || !clean_baseline || !plausible_counts {
        return Ok(None);
    }
    Ok(Some(DomainPerfEvidence {
        header,
        elapsed_wall_ms: read_u64(payload, 0),
        row_count,
        changed_rows,
        unit_count,
        changed_unit_count,
        status_flags,
    }))
}

fn data_record_matches_perf(
    header: compact_contract::CompactRecordHeader,
    payload: &[u8],
    perf: DomainPerfEvidence,
) -> Result<bool, String> {
    let Some(allowed_record_flags) = proof_allowed_data_record_flags(header.domain_id) else {
        // Structural default-deny: a scheduled domain is not proof-capable
        // merely because its DATA prefix resembles another domain's layout.
        return Ok(false);
    };
    if header.record_flags & !allowed_record_flags != 0 {
        // Invalid DATA flags make this pair insufficient; the stream remains
        // available for capture and a later qualifying pair may still prove.
        return Ok(false);
    }
    if payload.len() < 8 || read_u64(payload, 0) != perf.elapsed_wall_ms {
        return Ok(false);
    }

    let matches = match header.domain_id {
        10 => d10_fuel_data_matches_perf(payload, perf),
        44 => d44_supply_data_matches_perf(payload, perf),
        68 => d68_session_header_data_matches_perf(payload, perf),
        // Keep this arm even though the whitelist above already denies it:
        // adding a flag allowance alone must never silently make a new domain
        // proof-capable without an explicit real-material predicate.
        _ => false,
    };
    Ok(matches)
}

fn proof_allowed_data_record_flags(domain_id: u16) -> Option<u32> {
    match domain_id {
        // These production writers emit DATA with record_flags == 0. PERF's
        // diagnostic-trailer flag and d45's schema flag are invalid here.
        10 | 44 | 68 => Some(0),
        _ => None,
    }
}

fn clean_full_data_status(
    domain_id: u16,
    data_status_flags: u32,
    perf: DomainPerfEvidence,
) -> bool {
    let global_status =
        compact_contract::compact_status_for_global_interpretation(domain_id, data_status_flags);
    data_status_flags == perf.status_flags
        && data_status_flags & !FIRST_PROOF_KNOWN_STATUS_MASK == 0
        && !compact_contract::compact_status_is_failure(global_status)
        && global_status & compact_contract::COMPACT_STATUS_DELTA_DATA == 0
}

fn d10_fuel_data_matches_perf(payload: &[u8], perf: DomainPerfEvidence) -> bool {
    const HEADER_LEN: usize = 24;
    const ROW_LEN: usize = 40;
    const SCALE_RAW: u32 = 32_768;
    // Mirrors the production collector read_country_fuel emission bound.
    const MAX_FUEL_MEASURE_RAW: i64 = 1_000_000_000i64 * SCALE_RAW as i64;

    if payload.len() < HEADER_LEN {
        return false;
    }
    let row_count = read_u32(payload, 8);
    let changed_rows = read_u32(payload, 12);
    let status_flags = read_u32(payload, 16);
    let Some(expected_len) = (row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|rows_len| HEADER_LEN.checked_add(rows_len))
    else {
        return false;
    };
    if payload.len() != expected_len
        || read_u32(payload, 20) != SCALE_RAW
        || row_count == 0
        || changed_rows != row_count
        || !clean_full_data_status(10, status_flags, perf)
    {
        return false;
    }
    let mut seen_tag_db_ids = BTreeSet::new();
    let mut has_real_fuel_material = false;
    for row_index in 0..row_count as usize {
        let base = HEADER_LEN + row_index * ROW_LEN;
        let tag_db_id = read_u32(payload, base);
        let row_flags = read_u32(payload, base + 4);
        let current_raw = read_i64(payload, base + 8);
        let max_raw = read_i64(payload, base + 16);
        let gain_raw = read_i64(payload, base + 24);
        let cost_raw = read_i64(payload, base + 32);
        if tag_db_id == 0
            || tag_db_id > compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID
            || !seen_tag_db_ids.insert(tag_db_id)
            // The production d10 encoder always sets the fuel-present bit.
            || row_flags != 1
            || !(0..=MAX_FUEL_MEASURE_RAW).contains(&current_raw)
            || !(0..=MAX_FUEL_MEASURE_RAW).contains(&max_raw)
            || !(-MAX_FUEL_MEASURE_RAW..=MAX_FUEL_MEASURE_RAW).contains(&gain_raw)
            || !(-MAX_FUEL_MEASURE_RAW..=MAX_FUEL_MEASURE_RAW).contains(&cost_raw)
        {
            return false;
        }
        has_real_fuel_material |=
            current_raw != 0 || max_raw != 0 || gain_raw != 0 || cost_raw != 0;
    }

    // Production d10 PERF counts all resolved roots, while DATA carries only
    // countries with a fuel row. Its unit counters bind to emitted DATA rows.
    has_real_fuel_material
        && perf.row_count >= row_count
        && perf.changed_rows == changed_rows
        && perf.unit_count == row_count
        && perf.changed_unit_count == changed_rows
}

fn d44_supply_data_matches_perf(payload: &[u8], perf: DomainPerfEvidence) -> bool {
    const HEADER_LEN: usize = 48;
    const ROW_LEN: usize = 40;
    const SCALE_RAW: u32 = 100_000;

    if payload.len() < HEADER_LEN {
        return false;
    }
    let row_count = read_u32(payload, 8);
    let changed_rows = read_u32(payload, 12);
    let status_flags = read_u32(payload, 16);
    let province_rows = read_u32(payload, 24);
    let division_rows = read_u32(payload, 28);
    let badge_rows = read_u32(payload, 32);
    let csupply_input_rows = read_u32(payload, 44);
    let Some(real_entity_rows) = province_rows
        .checked_add(division_rows)
        .and_then(|value| value.checked_add(csupply_input_rows))
    else {
        return false;
    };
    let Some(emitted_rows) = province_rows
        .checked_add(division_rows)
        .and_then(|value| value.checked_add(badge_rows))
    else {
        return false;
    };
    let Some(all_data_rows) = emitted_rows.checked_add(csupply_input_rows) else {
        return false;
    };
    let Some(observed_roots) = read_u32(payload, 36).checked_add(read_u32(payload, 40)) else {
        return false;
    };
    let Some(expected_len) = (row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|rows_len| HEADER_LEN.checked_add(rows_len))
    else {
        return false;
    };
    if payload.len() != expected_len
        || read_u32(payload, 20) != SCALE_RAW
        || all_data_rows != row_count
        || real_entity_rows == 0
        || !clean_full_data_status(44, status_flags, perf)
    {
        return false;
    }
    let mut actual_province_rows = 0u32;
    let mut actual_division_rows = 0u32;
    let mut actual_badge_rows = 0u32;
    let mut actual_csupply_rows = 0u32;
    let mut live_entity_rows = 0u32;
    for row_index in 0..row_count as usize {
        let base = HEADER_LEN + row_index * ROW_LEN;
        let row_kind = read_u32(payload, base);
        let entity_id = read_u32(payload, base + 4);
        let counter = match row_kind {
            1 => &mut actual_province_rows,
            2 => &mut actual_division_rows,
            3 => &mut actual_badge_rows,
            4 | 5 => &mut actual_csupply_rows,
            _ => return false,
        };
        *counter = counter.saturating_add(1);
        if matches!(row_kind, 1 | 2 | 4 | 5) && entity_id & (1 << 31) == 0 {
            live_entity_rows = live_entity_rows.saturating_add(1);
        }
    }
    if actual_province_rows != province_rows
        || actual_division_rows != division_rows
        || actual_badge_rows != badge_rows
        || actual_csupply_rows != csupply_input_rows
        || live_entity_rows == 0
    {
        return false;
    }

    // The five mandatory computed-badge rows are diagnostics. Only province,
    // division, or csupply entity rows can satisfy real material.
    observed_roots == perf.row_count
        && perf.changed_rows == changed_rows
        && emitted_rows == perf.unit_count
        && perf.changed_unit_count == changed_rows
}

fn d68_session_header_data_matches_perf(payload: &[u8], perf: DomainPerfEvidence) -> bool {
    const HEADER_LEN: usize = 16;
    if payload.len() < HEADER_LEN {
        return false;
    }
    let status_flags = read_u32(payload, 12);
    if !clean_full_data_status(68, status_flags, perf) {
        return false;
    }

    // Production emits exactly one top-level session-header unit in PERF.
    // DATA has no generic row counters: offset 8 is field-presence bits and
    // offset 12 is the real status consumed by fold_session_header.
    perf.row_count == 1
        && perf.changed_rows == 1
        && perf.unit_count == 1
        && perf.changed_unit_count == 1
}

fn world_fold_proves_material_record(
    header: compact_contract::CompactRecordHeader,
    payload: &[u8],
    elapsed_wall_ms: u64,
) -> Result<bool, String> {
    if payload.len() < 8 || read_u64(payload, 0) != elapsed_wall_ms {
        return Ok(false);
    }
    let candidate = folded_snapshot(header.game_hour_raw, Some((header, payload)))?;
    let expected_name = compact_contract::wire_name_for_id(header.domain_id).ok_or_else(|| {
        format!(
            "world proof domain {} lacks a registry name",
            header.domain_id
        )
    })?;
    let folded_domain = candidate
        .pointer("/provenance/collector/domains")
        .and_then(serde_json::Value::as_array)
        .map(|domains| {
            domains
                .iter()
                .any(|domain| domain.as_str() == Some(expected_name))
        })
        .unwrap_or(false);
    if !folded_domain {
        return Ok(false);
    }
    Ok(match header.domain_id {
        10 => folded_fuel_has_real_material(&candidate),
        44 => folded_supply_has_real_material(&candidate),
        // d68 is last-value-wins, so a present scalar zero is preserved by the
        // fold but is not material proof. Inspect the production-folded
        // session_header and require a real nonzero/nonempty field.
        68 => folded_session_header_has_real_material(&candidate),
        _ => false,
    })
}

fn folded_fuel_has_real_material(snapshot: &serde_json::Value) -> bool {
    snapshot
        .get("countries")
        .and_then(serde_json::Value::as_object)
        .map(|countries| {
            countries.values().any(|country| {
                country
                    .get("fuel")
                    .and_then(serde_json::Value::as_object)
                    .map(|fuel| {
                        fuel.len() == 4
                            && fuel.values().all(|value| value.as_f64().is_some())
                            && fuel
                                .values()
                                .any(|value| value.as_f64().is_some_and(|value| value != 0.0))
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn folded_supply_has_real_material(snapshot: &serde_json::Value) -> bool {
    snapshot
        .pointer("/supply/by_province")
        .and_then(serde_json::Value::as_object)
        .map(|by_province| !by_province.is_empty())
        .unwrap_or(false)
}

fn folded_session_header_has_real_material(snapshot: &serde_json::Value) -> bool {
    let Some(header) = snapshot
        .get("session_header")
        .and_then(serde_json::Value::as_object)
    else {
        return false;
    };
    let player_present = header
        .get("player")
        .and_then(serde_json::Value::as_object)
        .and_then(|player| player.get("tag_db_id"))
        .and_then(serde_json::Value::as_u64)
        .map(|value| value != 0)
        .unwrap_or(false);
    let numeric_present = ["dlcs", "start_date_raw", "save_version", "session"]
        .iter()
        .any(|field| {
            header
                .get(*field)
                .and_then(serde_json::Value::as_u64)
                .map(|value| value != 0)
                .unwrap_or(false)
        });
    let string_present = ["ideology", "cosmetic_tag", "version"].iter().any(|field| {
        header
            .get(*field)
            .and_then(serde_json::Value::as_str)
            .map(|value| !value.is_empty())
            .unwrap_or(false)
    });
    player_present || numeric_present || string_present
}

fn folded_snapshot(
    game_hour_raw: i32,
    record: Option<(compact_contract::CompactRecordHeader, &[u8])>,
) -> Result<serde_json::Value, String> {
    let mut fold = crate::world_fold::WorldFold::new("auto_attach_first_world_record_proof");
    if let Some((header, payload)) = record {
        fold.ingest_record(
            header.sequence,
            header.game_hour_raw,
            header.domain_id,
            header.record_type,
            payload,
            header.record_flags,
        );
    }
    fold.ingest_record(
        0,
        game_hour_raw,
        1000,
        compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
        &[],
        0,
    );
    let snapshots = fold.finish();
    let (_, _, bytes) = snapshots
        .into_iter()
        .next()
        .ok_or_else(|| "world proof fold emitted no snapshot".to_string())?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("world proof snapshot was not JSON: {error}"))
}

#[cfg(test)]
fn prune_absent_world_values(value: serde_json::Value) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::Array(values) => {
            let values = values
                .into_iter()
                .filter_map(prune_absent_world_values)
                .collect::<Vec<_>>();
            (!values.is_empty()).then_some(serde_json::Value::Array(values))
        }
        serde_json::Value::Object(values) => {
            let values = values
                .into_iter()
                .filter_map(|(key, value)| {
                    prune_absent_world_values(value).map(|value| (key, value))
                })
                .collect::<serde_json::Map<_, _>>();
            (!values.is_empty()).then_some(serde_json::Value::Object(values))
        }
        scalar => Some(scalar),
    }
}

fn read_u16(payload: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        payload[offset..offset + 2]
            .try_into()
            .expect("bounds checked"),
    )
}

fn read_u32(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        payload[offset..offset + 4]
            .try_into()
            .expect("bounds checked"),
    )
}

fn read_u64(payload: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        payload[offset..offset + 8]
            .try_into()
            .expect("bounds checked"),
    )
}

fn read_i64(payload: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(
        payload[offset..offset + 8]
            .try_into()
            .expect("bounds checked"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durable_worker_status_is_create_new_synced_and_tailed_to_terminal_eof() {
        let root = std::env::temp_dir().join(format!(
            "hoi4_durable_worker_proof_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join(WORKER_PROOF_FILE_NAME);
        let retained = create_durable_proof_file(path.clone()).unwrap();
        assert!(create_durable_proof_file(path.clone()).is_err());

        let mut writer = DurableProofWriter::open_existing(&path, &root).unwrap();
        writer.write_all(b"durable-status").unwrap();
        writer.flush().unwrap();
        drop(writer);

        let stop = Arc::new(AtomicBool::new(true));
        let mut tail = DurableProofTail::new(retained, stop);
        let mut observed = [0u8; 14];
        tail.read_exact(&mut observed).unwrap();
        assert_eq!(&observed, b"durable-status");
        assert_eq!(tail.read(&mut [0u8; 1]).unwrap(), 0);
        drop(tail);
        fs::remove_dir_all(root).unwrap();
    }

    fn expectation() -> ProofExpectation {
        ProofExpectation {
            pid: 42,
            creation_filetime: 9001,
            h4cr_version: compact_contract::COMPACT_RECORD_VERSION,
            dll_sha256: [3; 32],
            challenge: [4; 16],
        }
    }

    #[test]
    fn proof_payloads_reject_mismatch_and_trailing_bytes() {
        let expected = expectation();
        let hello = collector_hello_payload(&expected);
        validate_collector_hello(&hello, &expected).unwrap();
        let mut bad = hello;
        bad.push(0);
        assert!(validate_collector_hello(&bad, &expected).is_err());
        let heartbeat = heartbeat_proof_payload(&expected, 77);
        assert_eq!(validate_heartbeat_proof(&heartbeat, &expected).unwrap(), 77);
    }

    #[test]
    fn cached_hash_cannot_be_emitted_as_loaded_module_proof() {
        let expected = expectation();
        let mut emitter = WorkerProofEmitter::new(Vec::<u8>::new(), [0x91; 16], expected).unwrap();
        let error = emitter
            .report_connected([0x55; 32])
            .expect_err("a different observed target image must abort the hello");
        assert!(error.contains("observed target DLL hash"), "{error}");
    }

    #[test]
    fn worker_status_bridge_preserves_monotonicity_across_progress_lag_and_rollover() {
        assert_eq!(monotonic_worker_proof_sequence(None, 2), 2);
        assert_eq!(monotonic_worker_proof_sequence(Some(2), 0), 2);
        assert_eq!(monotonic_worker_proof_sequence(Some(2), 1), 2);
        assert_eq!(monotonic_worker_proof_sequence(Some(2), 3), 3);
    }

    #[test]
    fn torn_first_record_never_proves_recording() {
        let hour = plausible_game_hour();
        let perf = compact_record(
            1,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(1, 1, 1, 1, 0),
        );
        let data = compact_record(
            2,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &fuel_payload(&[47], 0),
        );
        let mut gate = FirstCompactRecordGate::default();
        assert!(gate.observe(&perf).unwrap().is_none());
        assert!(gate.observe(&data[..data.len() - 1]).unwrap().is_none());
        assert_eq!(
            gate.observe(&data[data.len() - 1..])
                .unwrap()
                .unwrap()
                .sequence(),
            2
        );
    }

    #[test]
    fn unknown_zero_length_envelope_never_proves_recording() {
        let mut header = [0u8; compact_contract::COMPACT_RECORD_HEADER_LEN];
        header[0..4].copy_from_slice(compact_contract::COMPACT_RECORD_MAGIC);
        header[4..6].copy_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
        header[6..8]
            .copy_from_slice(&(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        header[8..16].copy_from_slice(&1u64.to_le_bytes());
        header[16..20].copy_from_slice(&plausible_game_hour().to_le_bytes());
        let err = FirstCompactRecordGate::default()
            .observe(&header)
            .expect_err("record_type=0 and payload_len=0 must fail closed");
        assert!(
            err.contains("record type") || err.contains("payload"),
            "{err}"
        );
    }

    #[test]
    fn domain_perf_telemetry_never_proves_recording() {
        let mut record = vec![0u8; compact_contract::COMPACT_RECORD_HEADER_LEN + 40];
        record[0..4].copy_from_slice(compact_contract::COMPACT_RECORD_MAGIC);
        record[4..6].copy_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
        record[6..8]
            .copy_from_slice(&(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        record[8..16].copy_from_slice(&1u64.to_le_bytes());
        record[16..20].copy_from_slice(&plausible_game_hour().to_le_bytes());
        record[20..22].copy_from_slice(&10u16.to_le_bytes());
        record[22..24]
            .copy_from_slice(&compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF.to_le_bytes());
        record[24..28].copy_from_slice(&40u32.to_le_bytes());

        assert!(FirstCompactRecordGate::default()
            .observe(&record)
            .unwrap()
            .is_none());
    }

    #[test]
    fn real_fuel_baseline_with_world_root_proves_recording() {
        let hour = plausible_game_hour();
        let mut stream = compact_record(
            1,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(2, 2, 2, 2, 0),
        );
        stream.extend_from_slice(&compact_record(
            2,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &fuel_payload(&[1, 47], 0),
        ));

        let proof = FirstCompactRecordGate::default()
            .observe(&stream)
            .unwrap()
            .expect("a real rooted world baseline must prove recording");
        assert_eq!(proof.sequence(), 2);
        assert_eq!(proof.domain_id(), 10);
    }

    #[test]
    fn round6_d10_out_of_range_all_zero_fuel_does_not_prove_recording() {
        let stream = d10_stream_with_single_row(fuel_payload_with_single_row(2_049, 0, 0, 0, 0, 0));
        assert!(FirstCompactRecordGate::new([10])
            .observe(&stream)
            .unwrap()
            .is_none());
    }

    #[test]
    fn round6_d10_in_range_all_zero_fuel_does_not_prove_recording() {
        let stream = d10_stream_with_single_row(fuel_payload_with_single_row(47, 1, 0, 0, 0, 0));
        assert!(FirstCompactRecordGate::new([10])
            .observe(&stream)
            .unwrap()
            .is_none());
    }

    #[test]
    fn round6_d10_out_of_range_nonzero_fuel_does_not_prove_recording() {
        let stream = d10_stream_with_single_row(fuel_payload_with_single_row(2_049, 1, 1, 0, 0, 0));
        assert!(FirstCompactRecordGate::new([10])
            .observe(&stream)
            .unwrap()
            .is_none());
    }

    #[test]
    fn round6_d10_absent_row_flag_with_nonzero_fuel_does_not_prove_recording() {
        let stream = d10_stream_with_single_row(fuel_payload_with_single_row(47, 0, 1, 0, 0, 0));
        assert!(FirstCompactRecordGate::new([10])
            .observe(&stream)
            .unwrap()
            .is_none());
    }

    #[test]
    fn round6_d10_out_of_production_fuel_bounds_does_not_prove_recording() {
        let over = 1_000_000_000i64 * 32_768 + 1;
        for payload in [
            fuel_payload_with_single_row(47, 1, over, 0, 0, 0),
            fuel_payload_with_single_row(47, 1, 0, over, 0, 0),
            fuel_payload_with_single_row(47, 1, 0, 0, over, 0),
            fuel_payload_with_single_row(47, 1, 0, 0, 0, -over),
        ] {
            let stream = d10_stream_with_single_row(payload);
            assert!(FirstCompactRecordGate::new([10])
                .observe(&stream)
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn round6_d10_in_range_nonzero_fuel_still_proves_recording() {
        let stream = d10_stream_with_single_row(fuel_payload_with_single_row(
            compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID,
            1,
            1,
            0,
            0,
            0,
        ));
        let proof = FirstCompactRecordGate::new([10])
            .observe(&stream)
            .unwrap()
            .unwrap();
        assert_eq!(proof.sequence(), 2);
        assert_eq!(proof.domain_id(), 10);
    }

    #[test]
    fn round5_d68_delta_data_does_not_prove_recording() {
        let hour = plausible_game_hour();
        let mut stream = compact_record(
            1,
            hour,
            68,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            // The old generic DATA parser read d68 presence=DLCS as row_count,
            // the real DATA status as changed_rows, and player=0 as status.
            &domain_perf_payload(
                2,
                compact_contract::COMPACT_STATUS_DELTA_DATA,
                2,
                compact_contract::COMPACT_STATUS_DELTA_DATA,
                0,
            ),
        );
        stream.extend_from_slice(&compact_record(
            2,
            hour,
            68,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &session_header_dlcs_payload(compact_contract::COMPACT_STATUS_DELTA_DATA, 7),
        ));

        assert!(
            FirstCompactRecordGate::new([68])
                .observe(&stream)
                .unwrap()
                .is_none(),
            "a d68 delta update is not a full material recording baseline"
        );
    }

    #[test]
    fn round5_d44_badge_only_data_does_not_prove_recording() {
        let hour = plausible_game_hour();
        let mut stream = compact_record(
            1,
            hour,
            44,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(2, 5, 5, 5, 0),
        );
        stream.extend_from_slice(&compact_record(
            2,
            hour,
            44,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &supply_badge_only_payload(),
        ));

        assert!(
            FirstCompactRecordGate::new([44])
                .observe(&stream)
                .unwrap()
                .is_none(),
            "mandatory d44 computed badges are diagnostics, not real entity rows"
        );
    }

    #[test]
    fn round5_d10_invalid_data_record_flags_do_not_prove_recording() {
        let hour = plausible_game_hour();
        let mut stream = compact_record(
            1,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(1, 1, 1, 1, 0),
        );
        stream.extend_from_slice(&compact_record(
            2,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &fuel_payload(&[47], 0),
        ));

        let error = FirstCompactRecordGate::new([10])
            .observe(&stream)
            .expect_err("a PERF-only record flag is invalid on d10 DATA");
        assert!(error.contains("record flags"), "{error}");
    }

    #[test]
    fn round5_domain_without_explicit_material_predicate_is_default_denied() {
        let hour = plausible_game_hour();
        let mut stream = compact_record(
            1,
            hour,
            11,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(1, 1, 1, 1, 0),
        );
        stream.extend_from_slice(&compact_record(
            2,
            hour,
            11,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &production_stockpile_payload(),
        ));

        assert!(
            FirstCompactRecordGate::new([11])
                .observe(&stream)
                .unwrap()
                .is_none(),
            "a scheduled domain without a vetted real-material predicate must keep waiting"
        );
    }

    #[test]
    fn round5_d68_full_nonzero_material_still_proves_recording() {
        let hour = plausible_game_hour();
        let mut stream = compact_record(
            1,
            hour,
            68,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(1, 1, 1, 1, 0),
        );
        stream.extend_from_slice(&compact_record(
            2,
            hour,
            68,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &session_header_dlcs_payload(0, 7),
        ));

        let proof = FirstCompactRecordGate::new([68])
            .observe(&stream)
            .unwrap()
            .expect("a clean full d68 record with a real nonzero DLC field must prove recording");
        assert_eq!(proof.sequence(), 2);
        assert_eq!(proof.domain_id(), 68);
    }

    #[test]
    fn d68_present_but_zero_scalar_does_not_prove_recording() {
        let hour = plausible_game_hour();
        let mut stream = compact_record(
            1,
            hour,
            68,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(1, 1, 1, 1, 0),
        );
        stream.extend_from_slice(&compact_record(
            2,
            hour,
            68,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &session_header_dlcs_payload(0, 0),
        ));
        assert!(FirstCompactRecordGate::new([68])
            .observe(&stream)
            .unwrap()
            .is_none());
    }

    #[test]
    fn d44_badges_cannot_be_relabelled_as_entity_rows_for_proof() {
        let hour = plausible_game_hour();
        let mut stream = compact_record(
            1,
            hour,
            44,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(2, 5, 5, 5, 0),
        );
        stream.extend_from_slice(&compact_record(
            2,
            hour,
            44,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &supply_badges_relabelled_as_province_payload(),
        ));
        assert!(FirstCompactRecordGate::new([44])
            .observe(&stream)
            .unwrap()
            .is_none());
    }

    #[test]
    fn d44_tombstone_only_entity_rows_do_not_prove_recording() {
        let hour = plausible_game_hour();
        let mut stream = compact_record(
            1,
            hour,
            44,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(2, 6, 6, 6, 0),
        );
        stream.extend_from_slice(&compact_record(
            2,
            hour,
            44,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &supply_tombstone_only_payload(),
        ));
        assert!(FirstCompactRecordGate::new([44])
            .observe(&stream)
            .unwrap()
            .is_none());
    }

    #[test]
    fn unscheduled_world_domain_cannot_prove_recording() {
        let stream = fuel_world_stream(plausible_game_hour(), 0, &[1]);
        assert!(FirstCompactRecordGate::new([11])
            .observe(&stream)
            .unwrap()
            .is_none());
    }

    #[test]
    fn implausible_game_clock_fails_closed() {
        let stream = fuel_world_stream(0, 0, &[1]);
        let error = FirstCompactRecordGate::new([10])
            .observe(&stream)
            .expect_err("an out-of-range Clausewitz clock must abort proof");
        assert!(error.contains("implausible game clock"), "{error}");
    }

    #[test]
    fn first_record_sequence_is_exactly_one() {
        let hour = plausible_game_hour();
        for sequence in [0, 2, u64::MAX] {
            let record = compact_record(
                sequence,
                hour,
                10,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
                0,
                &domain_perf_payload(1, 1, 1, 1, 0),
            );
            let error = FirstCompactRecordGate::new([10])
                .observe(&record)
                .expect_err("an arbitrary initial sequence must fail closed");
            assert!(error.contains("initial sequence"), "{error}");
        }
    }

    #[test]
    fn exhausted_sequence_space_never_saturates_to_an_adjacent_duplicate() {
        let hour = plausible_game_hour();
        let record = compact_record(
            u64::MAX,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(1, 1, 1, 1, 0),
        );
        let mut gate = FirstCompactRecordGate::new([10]);
        gate.last_sequence = Some(u64::MAX);
        let error = gate
            .observe(&record)
            .expect_err("u64::MAX must exhaust sequence space, not self-adjoin");
        assert!(error.contains("exhausted sequence space"), "{error}");
    }

    #[test]
    fn forward_sequence_gap_fails_before_perf_data_pairing() {
        let hour = plausible_game_hour();
        let mut gate = FirstCompactRecordGate::new([10]);
        let perf = compact_record(
            1,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(1, 1, 1, 1, 0),
        );
        assert!(gate.observe(&perf).unwrap().is_none());
        let data = compact_record(
            3,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &fuel_payload(&[47], 0),
        );
        let error = gate
            .observe(&data)
            .expect_err("a forward sequence gap must fail closed");
        assert!(error.contains("sequence 3 after 1"), "{error}");
    }

    #[test]
    fn frame_end_must_pass_canonical_record_semantics() {
        let payload = [0; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN];
        for (domain_id, record_flags, payload) in [
            (
                compact_contract::COMPACT_DOMAIN_FRAME,
                0,
                &payload[..payload.len() - 1],
            ),
            (999, 0, &payload[..]),
            (
                compact_contract::COMPACT_DOMAIN_FRAME,
                compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
                &payload[..],
            ),
        ] {
            let record = compact_record(
                1,
                plausible_game_hour(),
                domain_id,
                compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
                record_flags,
                payload,
            );
            let error = FirstCompactRecordGate::new([10])
                .observe(&record)
                .expect_err("a noncanonical FRAME_END must fail before state dispatch");
            assert!(error.contains("record semantics"), "{error}");
        }
    }

    #[test]
    fn failure_or_delta_status_cannot_seed_recording_proof() {
        for status in [
            compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_PARTIAL_DATA,
            compact_contract::COMPACT_STATUS_DELTA_DATA,
            1 << 31,
        ] {
            let stream = fuel_world_stream(plausible_game_hour(), status, &[1]);
            assert!(
                FirstCompactRecordGate::new([10])
                    .observe(&stream)
                    .unwrap()
                    .is_none(),
                "status {status:#x} advanced Recording"
            );
        }
    }

    #[test]
    fn zero_root_or_malformed_world_payload_cannot_prove_recording() {
        let hour = plausible_game_hour();
        for payload in [vec![0; 64], fuel_payload(&[], 0), fuel_payload(&[0], 0), {
            let mut malformed = fuel_payload(&[1], 0);
            malformed[8..12].copy_from_slice(&2u32.to_le_bytes());
            malformed
        }] {
            let mut stream = compact_record(
                1,
                hour,
                10,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
                0,
                &domain_perf_payload(1, 1, 1, 1, 0),
            );
            stream.extend_from_slice(&compact_record(
                2,
                hour,
                10,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                &payload,
            ));
            assert!(FirstCompactRecordGate::new([10])
                .observe(&stream)
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn perf_and_data_must_be_an_ordered_same_domain_pair() {
        let hour = plausible_game_hour();
        let mut stream = compact_record(
            1,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(1, 1, 1, 1, 0),
        );
        stream.extend_from_slice(&compact_record(
            2,
            hour,
            11,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &fuel_payload(&[1], 0),
        ));
        assert!(FirstCompactRecordGate::new([10, 11])
            .observe(&stream)
            .unwrap()
            .is_none());
    }

    #[test]
    fn perf_roster_then_data_roster_proves_the_matching_domain() {
        let hour = plausible_game_hour();
        let mut stream = compact_record(
            1,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(1, 1, 1, 1, 0),
        );
        stream.extend_from_slice(&compact_record(
            2,
            hour,
            68,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(1, 1, 1, 1, 0),
        ));
        stream.extend_from_slice(&compact_record(
            3,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &fuel_payload(&[47], 0),
        ));

        let proof = FirstCompactRecordGate::new([10, 68])
            .observe(&stream)
            .unwrap()
            .expect("the collector's PERF-roster/DATA-roster order must prove recording");
        assert_eq!(proof.sequence(), 3);
        assert_eq!(proof.domain_id(), 10);
    }

    #[test]
    fn source_unavailable_supply_data_does_not_prove_recording() {
        let hour = plausible_game_hour();
        let mut stream = compact_record(
            1,
            hour,
            44,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(2, 1, 1, 1, 0),
        );
        stream.extend_from_slice(&compact_record(
            2,
            hour,
            44,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &supply_payload(
                compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
                0,
                0,
                0,
                0,
                1,
                1,
            ),
        ));

        assert!(FirstCompactRecordGate::new([44])
            .observe(&stream)
            .unwrap()
            .is_none());
    }

    #[test]
    fn empty_land_combat_snapshot_does_not_prove_recording() {
        let hour = plausible_game_hour();
        let mut stream = compact_record(
            1,
            hour,
            21,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(1, 1, 1, 1, 0),
        );
        stream.extend_from_slice(&compact_record(
            2,
            hour,
            21,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &land_combat_payload(0, 1, 0, 0, 1),
        ));

        assert!(FirstCompactRecordGate::new([21])
            .observe(&stream)
            .unwrap()
            .is_none());
    }

    #[test]
    fn contradictory_data_counters_are_skipped_until_a_genuine_pair_arrives() {
        let hour = plausible_game_hour();
        let mut stream = compact_record(
            1,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(2, 2, 2, 2, 0),
        );
        stream.extend_from_slice(&compact_record(
            2,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &fuel_payload(&[47], 0),
        ));
        stream.extend_from_slice(&compact_record(
            3,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(1, 1, 1, 1, 0),
        ));
        stream.extend_from_slice(&compact_record(
            4,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &fuel_payload(&[47], 0),
        ));

        let proof = FirstCompactRecordGate::new([10])
            .observe(&stream)
            .unwrap()
            .expect("the later genuine PERF/DATA pair must still prove recording");
        assert_eq!(proof.sequence(), 4);
    }

    #[test]
    fn material_projection_drops_empty_and_unknown_world_values() {
        assert!(prune_absent_world_values(serde_json::json!([])).is_none());
        assert!(prune_absent_world_values(serde_json::json!({
            "by_province": null,
            "source": null,
            "confidence": null,
            "r2_logistics_attrition": null,
        }))
        .is_none());
        assert_eq!(
            prune_absent_world_values(serde_json::json!([{"combat_id": 7}])),
            Some(serde_json::json!([{"combat_id": 7}]))
        );
    }

    fn plausible_game_hour() -> i32 {
        compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24
    }

    fn compact_record(
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut record = vec![0u8; compact_contract::COMPACT_RECORD_HEADER_LEN];
        record[0..4].copy_from_slice(compact_contract::COMPACT_RECORD_MAGIC);
        record[4..6].copy_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
        record[6..8]
            .copy_from_slice(&(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        record[8..16].copy_from_slice(&sequence.to_le_bytes());
        record[16..20].copy_from_slice(&game_hour_raw.to_le_bytes());
        record[20..22].copy_from_slice(&domain_id.to_le_bytes());
        record[22..24].copy_from_slice(&record_type.to_le_bytes());
        record[24..28].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        record[28..32].copy_from_slice(&record_flags.to_le_bytes());
        record.extend_from_slice(payload);
        record
    }

    fn fuel_world_stream(game_hour_raw: i32, status_flags: u32, roots: &[u32]) -> Vec<u8> {
        let row_count = roots.len() as u32;
        let mut stream = compact_record(
            1,
            game_hour_raw,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(row_count, row_count, row_count, row_count, status_flags),
        );
        stream.extend_from_slice(&compact_record(
            2,
            game_hour_raw,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &fuel_payload(roots, status_flags),
        ));
        stream
    }

    fn domain_perf_payload(
        row_count: u32,
        changed_rows: u32,
        unit_count: u32,
        changed_unit_count: u32,
        status_flags: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::with_capacity(40);
        payload.extend_from_slice(&99u64.to_le_bytes());
        payload.extend_from_slice(&7u64.to_le_bytes());
        payload.extend_from_slice(&row_count.to_le_bytes());
        payload.extend_from_slice(&changed_rows.to_le_bytes());
        payload.extend_from_slice(&unit_count.to_le_bytes());
        payload.extend_from_slice(&changed_unit_count.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn d10_stream_with_single_row(payload: Vec<u8>) -> Vec<u8> {
        let hour = plausible_game_hour();
        let mut stream = compact_record(
            1,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(1, 1, 1, 1, 0),
        );
        stream.extend_from_slice(&compact_record(
            2,
            hour,
            10,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        ));
        stream
    }

    fn fuel_payload(tag_db_ids: &[u32], status_flags: u32) -> Vec<u8> {
        let row_count = tag_db_ids.len() as u32;
        let mut payload = Vec::with_capacity(24 + tag_db_ids.len() * 40);
        payload.extend_from_slice(&99u64.to_le_bytes());
        payload.extend_from_slice(&row_count.to_le_bytes());
        payload.extend_from_slice(&row_count.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&32_768u32.to_le_bytes());
        for (index, tag_db_id) in tag_db_ids.iter().enumerate() {
            payload.extend_from_slice(&tag_db_id.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&(100 + index as i64).to_le_bytes());
            payload.extend_from_slice(&(200 + index as i64).to_le_bytes());
            payload.extend_from_slice(&(10 + index as i64).to_le_bytes());
            payload.extend_from_slice(&(-5 - index as i64).to_le_bytes());
        }
        payload
    }

    fn fuel_payload_with_single_row(
        tag_db_id: u32,
        row_flags: u32,
        current_raw: i64,
        max_raw: i64,
        gain_raw: i64,
        cost_raw: i64,
    ) -> Vec<u8> {
        let mut payload = Vec::with_capacity(64);
        payload.extend_from_slice(&99u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&32_768u32.to_le_bytes());
        payload.extend_from_slice(&tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row_flags.to_le_bytes());
        payload.extend_from_slice(&current_raw.to_le_bytes());
        payload.extend_from_slice(&max_raw.to_le_bytes());
        payload.extend_from_slice(&gain_raw.to_le_bytes());
        payload.extend_from_slice(&cost_raw.to_le_bytes());
        payload
    }

    fn supply_payload(
        status_flags: u32,
        province_rows: u32,
        division_rows: u32,
        badge_rows: u32,
        csupply_input_rows: u32,
        observed_province_roots: u32,
        observed_division_roots: u32,
    ) -> Vec<u8> {
        let row_count = province_rows
            .saturating_add(division_rows)
            .saturating_add(badge_rows)
            .saturating_add(csupply_input_rows);
        let mut payload = Vec::with_capacity(48);
        payload.extend_from_slice(&99u64.to_le_bytes());
        payload.extend_from_slice(&row_count.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&32_768u32.to_le_bytes());
        payload.extend_from_slice(&province_rows.to_le_bytes());
        payload.extend_from_slice(&division_rows.to_le_bytes());
        payload.extend_from_slice(&badge_rows.to_le_bytes());
        payload.extend_from_slice(&observed_province_roots.to_le_bytes());
        payload.extend_from_slice(&observed_division_roots.to_le_bytes());
        payload.extend_from_slice(&csupply_input_rows.to_le_bytes());
        payload
    }

    fn supply_badge_only_payload() -> Vec<u8> {
        let badge_flags: [u32; 5] = [1 << 9, 1 << 10, 1 << 14, 1 << 15, 1 << 16];
        let mut payload = supply_payload(0, 0, 0, badge_flags.len() as u32, 0, 1, 1);
        payload[12..16].copy_from_slice(&(badge_flags.len() as u32).to_le_bytes());
        payload[20..24].copy_from_slice(&100_000u32.to_le_bytes());
        for (code, flags) in badge_flags.into_iter().enumerate() {
            payload.extend_from_slice(&3u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&(code as i32 + 1).to_le_bytes());
            payload.extend_from_slice(&0i32.to_le_bytes());
            payload.extend_from_slice(&0i32.to_le_bytes());
            payload.extend_from_slice(&0i32.to_le_bytes());
            payload.extend_from_slice(&0i32.to_le_bytes());
        }
        payload
    }

    fn supply_badges_relabelled_as_province_payload() -> Vec<u8> {
        let mut payload = supply_badge_only_payload();
        payload[24..28].copy_from_slice(&1u32.to_le_bytes());
        payload[32..36].copy_from_slice(&4u32.to_le_bytes());
        payload
    }

    fn supply_tombstone_only_payload() -> Vec<u8> {
        let mut payload = supply_badge_only_payload();
        let mut tombstone = Vec::with_capacity(40);
        tombstone.extend_from_slice(&1u32.to_le_bytes());
        tombstone.extend_from_slice(&((1u32 << 31) | 42).to_le_bytes());
        tombstone.extend_from_slice(&0u32.to_le_bytes());
        tombstone.extend_from_slice(&0u32.to_le_bytes());
        tombstone.extend_from_slice(&0u32.to_le_bytes());
        tombstone.extend_from_slice(&0i32.to_le_bytes());
        tombstone.extend_from_slice(&0i32.to_le_bytes());
        tombstone.extend_from_slice(&0i32.to_le_bytes());
        tombstone.extend_from_slice(&0i32.to_le_bytes());
        tombstone.extend_from_slice(&0i32.to_le_bytes());
        drop(payload.splice(48..48, tombstone));
        payload[8..12].copy_from_slice(&6u32.to_le_bytes());
        payload[12..16].copy_from_slice(&6u32.to_le_bytes());
        payload[24..28].copy_from_slice(&1u32.to_le_bytes());
        payload
    }

    fn session_header_dlcs_payload(status_flags: u32, dlcs: u32) -> Vec<u8> {
        let mut payload = Vec::with_capacity(42);
        payload.extend_from_slice(&99u64.to_le_bytes());
        payload.extend_from_slice(&(1u32 << 1).to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&dlcs.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload
    }

    fn production_stockpile_payload() -> Vec<u8> {
        let mut payload = Vec::with_capacity(56);
        payload.extend_from_slice(&99u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&100_000u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&7u32.to_le_bytes());
        payload.extend_from_slice(&100_000i64.to_le_bytes());
        payload
    }

    fn land_combat_payload(
        combat_count: u32,
        changed_rows: u32,
        status_flags: u32,
        participant_count: u32,
        ended_count: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::with_capacity(32);
        payload.extend_from_slice(&99u64.to_le_bytes());
        payload.extend_from_slice(&combat_count.to_le_bytes());
        payload.extend_from_slice(&changed_rows.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&participant_count.to_le_bytes());
        payload.extend_from_slice(&ended_count.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }
}
