//! Bounded-memory streaming fold over an on-disk compact record stream.
//!
//! This is the production replacement for `WorldFold::ingest_stream(&whole_buffer)`:
//! instead of holding the entire (multi-GB) compact stream in RAM, it reads the
//! stream file in fixed chunks, frames each 32-byte `H4CR` record against a small
//! carry buffer, and drives the SAME single production fold
//! ([`WorldFold::ingest_record`]). Because the fold is constructed file-backed
//! ([`WorldFold::new_file_backed`]), each per-game-hour snapshot is serialized and
//! spilled to disk the moment its `FRAME_END` closes and then released — so peak
//! memory is O(current world-fold state + one hour's snapshot), independent of the
//! stream's length.
//!
//! The framing loop is byte-for-byte the same contract the `fold_check`
//! fold-equivalence gate (G1) walks, so a stream folded here produces per-hour
//! snapshots hash-identical to `fold_check`'s reference fold. This module is used
//! by the recorder host during finalization and by the `stream_fold` offline
//! tool (which also re-folds old recordings and measures peak memory).

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use compact_contract::campaign_identity::ValidatedCid1;
use compact_contract::stream_validator::{
    AcceptedRecordKind, StatefulStreamValidator, StreamIdentityMode,
};
use compact_contract::{
    parse_compact_record_header, validate_compact_record_semantics, RecordFramingError,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_GAP,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_DEGRADED,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_TYPE_FRAME_REFUSED, COMPACT_STATUS_FAILURE_MASK,
};
use snapshot::sha256::{to_hex, Sha256};

use crate::domain_data_authority::{summarize_domain_data_payload, DomainDataContext};
use crate::world_fold::{FoldedSnapshotFile, WorldFold};

/// Read granularity for the streaming fold. Bounded and independent of stream size.
const CHUNK: usize = 8 * 1024 * 1024;

/// Every globally-defined compact status currently occupies one of bits 0..=13.
/// FRAME_END carries the frame-wide aggregate, so lane-local interpretations do
/// not apply there and any higher bit is an unknown wire-contract extension.
const FRAME_END_KNOWN_STATUS_MASK: u32 = (1 << 14) - 1;

fn validate_frame_end_seal_fields(payload: &[u8]) -> Result<(u32, u32), String> {
    if payload.len() != compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN {
        return Err(format!(
            "FRAME_END payload length {} is not {}",
            payload.len(),
            compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN
        ));
    }

    // Wire layout: elapsed_wall_ms:u64, frame_time_us:u64,
    // emitted_domains:u32, scheduled_domains:u32, status_flags:u32,
    // reserved:u32. The timing fields are decoded by their fixed offsets but
    // carry no seal-time predicate; the remaining fields are authoritative.
    let _elapsed_wall_ms = u64::from_le_bytes(payload[0..8].try_into().unwrap());
    let _frame_time_us = u64::from_le_bytes(payload[8..16].try_into().unwrap());
    let emitted_domains = u32::from_le_bytes(payload[16..20].try_into().unwrap());
    let scheduled_domains = u32::from_le_bytes(payload[20..24].try_into().unwrap());
    let status_flags = u32::from_le_bytes(payload[24..28].try_into().unwrap());
    let reserved = u32::from_le_bytes(payload[28..32].try_into().unwrap());

    if reserved != 0 {
        return Err(format!(
            "FRAME_END reserved word is nonzero ({reserved:#010x})"
        ));
    }
    if emitted_domains != scheduled_domains {
        return Err(format!(
            "FRAME_END emitted domain count {emitted_domains} does not match scheduled domain count {scheduled_domains}"
        ));
    }
    let unknown = status_flags & !FRAME_END_KNOWN_STATUS_MASK;
    if unknown != 0 {
        return Err(format!(
            "FRAME_END status flags {status_flags:#010x} contain unknown bits {unknown:#010x}"
        ));
    }
    let fatal = status_flags & COMPACT_STATUS_FAILURE_MASK;
    if fatal != 0 {
        return Err(format!(
            "FRAME_END status flags {status_flags:#010x} contain fatal bits {fatal:#010x}"
        ));
    }
    Ok((emitted_domains, scheduled_domains))
}

/// Accounting for one streaming fold pass (mirrors the `fold_check` totals so the
/// two can be cross-checked). Never grows with stream length.
#[derive(Debug, Default, Clone)]
pub struct FoldStreamStats {
    /// Total decoded H4CR bytes (independent of raw vs zstd on-disk storage).
    pub stream_bytes: u64,
    /// Bytes consumed as whole, well-framed records.
    pub bytes_folded: u64,
    /// Trailing bytes that did not form a complete record (mid-record EOF).
    pub truncated_tail_bytes: u64,
    /// Accepted ordinary records. The sequence-zero CID1 protocol header is
    /// deliberately excluded to match fold-check/replay-decoder accounting.
    pub records_total: u64,
    pub data_records: u64,
    /// DOMAIN_DATA records whose payload failed stream-boundary exact-consumption
    /// validation and were degraded per-domain (owner zero-data-loss ruling) rather
    /// than failing the whole fold. The record is framed-valid; only its payload is bad.
    pub malformed_payload_records: u64,
    /// Bounded diagnostic breakdown for malformed DATA records. There can be at
    /// most one entry per registered domain, so retaining the first exact
    /// decoder reason cannot grow with capture duration.
    pub malformed_payload_domains: BTreeMap<u16, MalformedPayloadDomainStats>,
    pub frame_end_records: u64,
    pub perf_records: u64,
    /// Ordinary non-DATA/non-PERF/non-FRAME_END records; excludes CID1.
    pub other_records: u64,
    pub hours_emitted: u64,
    /// Set when framing stopped early on a malformed record (bad magic/version).
    pub stop_reason: Option<String>,
    /// True when the stream ends at a CLEAN CLOSED boundary per the shared
    /// BoundaryTracker (no residual bytes, no open hour, no hour-interleave /
    /// mismatched-FRAME_END / sequence-gap violation, >=1 closed hour). A raw
    /// crash tail (tolerated on read) makes this false; no seal path may pack
    /// such a source as a clean compressed container.
    pub ends_at_frame_end: bool,
    /// SHA-256 of the decoded stream bytes — parallel workers must agree on
    /// CONTENT, not just shape (round-3 finding 3); sha256 (round 4) so even
    /// deliberately-collided cheap-hash source versions cannot merge into a
    /// mixed fold. Hex, empty when never computed.
    pub decoded_sha256: String,
}

#[derive(Debug, Default, Clone)]
pub struct MalformedPayloadDomainStats {
    pub records: u64,
    pub first_reason: String,
    pub first_stream_offset: u64,
    pub first_game_hour_raw: i32,
}

/// Pointer-free identity telemetry retained by fold/seal walkers. This is
/// derived only from `ValidatedCid1`; syntax parsing alone cannot construct it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedCampaignIdentity {
    pub campaign_id: String,
    pub game_unique_id: String,
    pub game_unique_seed: i32,
    pub source_build_profile: String,
    pub observed_game_hour_raw: i32,
    pub cid1_payload_sha256: String,
}

/// Result of an identity-mode-explicit fold. The legacy APIs continue to return
/// `FoldStreamStats` unchanged; explicit callers additionally receive the CID1
/// authority retained by the validator.
#[derive(Debug, Clone)]
pub struct IdentityAwareFoldStats {
    pub stats: FoldStreamStats,
    pub campaign_identity: Option<FoldedCampaignIdentity>,
    validated_cid1: Option<ValidatedCid1>,
}

impl IdentityAwareFoldStats {
    /// Exact recorder-manifest object derived from the placement-authoritative
    /// CID1 token retained by this complete fold. No parallel fields can mint it.
    pub fn validated_campaign_identity_manifest(&self) -> Option<serde_json::Value> {
        self.validated_cid1.as_ref().map(|identity| {
            crate::campaign_identity::to_manifest_json(
                &crate::campaign_identity::resolve_validated_cid1(identity),
            )
        })
    }
}

impl From<&compact_contract::campaign_identity::ValidatedCid1> for FoldedCampaignIdentity {
    fn from(identity: &compact_contract::campaign_identity::ValidatedCid1) -> Self {
        Self {
            campaign_id: identity.campaign_id_hex(),
            game_unique_id: identity.game_unique_id().to_owned(),
            game_unique_seed: identity.game_unique_seed(),
            source_build_profile: identity.source_build_profile().to_owned(),
            observed_game_hour_raw: identity.game_hour_raw(),
            cid1_payload_sha256: identity.payload_sha256_hex(),
        }
    }
}

fn stream_validator(mode: StreamIdentityMode) -> StatefulStreamValidator {
    match mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedSnapshotReportRow {
    pub hour_index: i64,
    pub date: String,
    pub snapshot_hash: String,
    pub snapshot_bytes: usize,
}

/// Fold an on-disk compact stream file into per-game-hour snapshot files under
/// `snapshot_dir`, in bounded memory. Returns the spilled snapshot descriptors
/// (in emit order) plus the pass accounting.
pub fn fold_compact_stream_file(
    stream_path: &Path,
    game_id: &str,
    snapshot_dir: &Path,
) -> io::Result<(Vec<FoldedSnapshotFile>, FoldStreamStats)> {
    fold_compact_stream_file_with_identity_mode(
        stream_path,
        game_id,
        snapshot_dir,
        StreamIdentityMode::Legacy,
    )
    .map(|(files, outcome)| (files, outcome.stats))
}

/// Identity-mode-explicit form of [`fold_compact_stream_file`]. Protocol mode
/// is selected before opening the stream and is never sniffed or downgraded.
pub fn fold_compact_stream_file_with_identity_mode(
    stream_path: &Path,
    game_id: &str,
    snapshot_dir: &Path,
    identity_mode: StreamIdentityMode,
) -> io::Result<(Vec<FoldedSnapshotFile>, IdentityAwareFoldStats)> {
    let reader = compact_stream::open(stream_path)?;
    let compressed = reader.compression() == compact_stream::StreamCompression::Zstd;
    let total_len = if compressed { 0 } else { reader.encoded_len() };
    let mut fold = WorldFold::new_file_backed(game_id, snapshot_dir)?;
    let outcome = drive_fold_compressed_with_identity_mode(
        reader,
        total_len,
        &mut fold,
        compressed,
        identity_mode,
    )?;
    let files = fold
        .finish_file_backed()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    Ok((files, outcome))
}

/// ONE shared logical-boundary state machine (container gate round 3: two
/// copies of this invariant are a permanent drift hazard — the fold driver
/// and the seal-time walker now both drive THIS). Legal production streams
/// are strictly hour-sequential: every record of hour h, then FRAME_END(h).
/// Anything else is corruption and must never be silently normalized:
/// - DOMAIN_DATA opens its hour; a DATA for a DIFFERENT hour while one is
///   open is a VIOLATION (round-3 finding 1: a single-slot tracker that just
///   overwrites forgets the earlier open hour).
/// - Only a FRAME_END whose hour MATCHES the open hour closes it; a
///   mismatched FRAME_END is a VIOLATION.
/// - Record sequence starts at 1 and must be CONSECUTIVE (+1) without wrap —
///   an inflated-but-under-cap payload_len that swallows whole records leaves
///   a sequence gap (round-3 finding 2); framing alone cannot see the swallowed
///   bytes, the counter can.
/// - PERF may precede DATA, but it binds the pending hour; every later PERF and
///   DATA before FRAME_END must use that same hour. PERF inside an open DATA
///   frame must match the open hour.
/// - Closed hours are strictly increasing. Forward gaps remain visible as
///   coverage gaps to higher-level duration checks; same/backward hours are
///   corruption. The final physical record at EOF must be FRAME_END.
#[derive(Debug, Default)]
pub struct BoundaryTracker {
    open_hour: Option<i32>,
    pending_perf_hour: Option<i32>,
    last_closed_hour: Option<i32>,
    last_record_kind: Option<u16>,
    terminal_outcome_domains: BTreeSet<u16>,
    gap_domains: BTreeSet<u16>,
    data_domains: BTreeSet<u16>,
    prev_sequence: Option<u64>,
    violation: Option<String>,
}

impl BoundaryTracker {
    pub fn observe(
        &mut self,
        header: &compact_contract::CompactRecordHeader,
        payload: &[u8],
        at_byte: u64,
    ) {
        if self.violation.is_some() {
            return; // first violation is the diagnostic; state is already dirty
        }
        match self.prev_sequence {
            None if header.sequence != 1 => {
                self.violation = Some(format!(
                    "stream starts with sequence {} at byte {at_byte}; expected 1",
                    header.sequence
                ));
                return;
            }
            Some(prev) => {
                let Some(expected) = prev.checked_add(1) else {
                    self.violation = Some(format!(
                        "sequence overflow at byte {at_byte}: {prev} cannot be followed \
                         without wrapping"
                    ));
                    return;
                };
                if header.sequence != expected {
                    self.violation = Some(format!(
                        "sequence discontinuity at byte {at_byte}: {prev} -> {} \
                         (expected {expected}; a record was swallowed or reordered)",
                        header.sequence
                    ));
                    return;
                }
            }
            None => {}
        }
        self.prev_sequence = Some(header.sequence);

        if let Err(reason) = validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            payload,
        ) {
            self.violation = Some(format!(
                "record semantic violation at byte {at_byte}: {reason}"
            ));
            return;
        }

        self.last_record_kind = Some(header.record_type);
        match header.record_type {
            COMPACT_RECORD_TYPE_DOMAIN_PERF | COMPACT_RECORD_TYPE_DOMAIN_GAP => {
                let is_probe = header.record_type == COMPACT_RECORD_TYPE_DOMAIN_PERF
                    && header.domain_id == 250;
                if !is_probe && !self.terminal_outcome_domains.insert(header.domain_id) {
                    self.violation = Some(format!(
                        "duplicate terminal outcome for domain {} in hour {} at byte {at_byte}",
                        header.domain_id, header.game_hour_raw
                    ));
                    return;
                }
                if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_GAP {
                    if self.data_domains.contains(&header.domain_id) {
                        self.violation = Some(format!(
                            "DOMAIN_GAP for domain {} in hour {} coexists with DOMAIN_DATA at byte {at_byte}",
                            header.domain_id, header.game_hour_raw
                        ));
                        return;
                    }
                    self.gap_domains.insert(header.domain_id);
                }
                match self.open_hour {
                    Some(open) if open != header.game_hour_raw => {
                        self.violation = Some(format!(
                        "hour interleaving at byte {at_byte}: DOMAIN_PERF for hour {} while hour {open} is still open",
                        header.game_hour_raw
                    ));
                    }
                    Some(_) => {}
                    None => {
                        if let Some(last_closed) = self.last_closed_hour {
                            if header.game_hour_raw <= last_closed {
                                self.violation = Some(format!(
                                "non-monotonic DOMAIN_PERF at byte {at_byte}: closed hour {last_closed}, then {}",
                                header.game_hour_raw
                            ));
                                return;
                            }
                        }
                        match self.pending_perf_hour {
                            Some(pending) if pending != header.game_hour_raw => {
                                self.violation = Some(format!(
                                "PERF hour conflict at byte {at_byte}: pending hour {pending}, then {}",
                                header.game_hour_raw
                            ));
                            }
                            Some(_) => {}
                            None => self.pending_perf_hour = Some(header.game_hour_raw),
                        }
                    }
                }
            }
            COMPACT_RECORD_TYPE_DOMAIN_DATA => {
                if self.gap_domains.contains(&header.domain_id) {
                    self.violation = Some(format!(
                        "DOMAIN_DATA for domain {} in hour {} coexists with DOMAIN_GAP at byte {at_byte}",
                        header.domain_id, header.game_hour_raw
                    ));
                    return;
                }
                self.data_domains.insert(header.domain_id);
                match self.open_hour {
                    Some(open) if open != header.game_hour_raw => {
                        self.violation = Some(format!(
                            "hour interleaving at byte {at_byte}: DOMAIN_DATA for hour {} \
                         while hour {open} is still open",
                            header.game_hour_raw
                        ));
                    }
                    Some(_) => {}
                    None => {
                        if let Some(pending) = self.pending_perf_hour {
                            if pending != header.game_hour_raw {
                                self.violation = Some(format!(
                                    "DATA hour conflict at byte {at_byte}: pending PERF hour {pending}, then DATA hour {}",
                                    header.game_hour_raw
                                ));
                                return;
                            }
                        }
                        if let Some(last_closed) = self.last_closed_hour {
                            if header.game_hour_raw <= last_closed {
                                self.violation = Some(format!(
                                    "non-monotonic DOMAIN_DATA at byte {at_byte}: closed hour {last_closed}, then {}",
                                    header.game_hour_raw
                                ));
                                return;
                            }
                        }
                        self.open_hour = Some(header.game_hour_raw);
                    }
                }
            }
            COMPACT_RECORD_TYPE_FRAME_END => {
                let (emitted_domains, scheduled_domains) =
                    match validate_frame_end_seal_fields(payload) {
                        Ok(counts) => counts,
                        Err(reason) => {
                            self.violation = Some(format!(
                                "FRAME_END seal violation at byte {at_byte}: {reason}"
                            ));
                            return;
                        }
                    };
                let observed_terminal_outcomes =
                    self.terminal_outcome_domains.len().min(u32::MAX as usize) as u32;
                if observed_terminal_outcomes != emitted_domains
                    || observed_terminal_outcomes != scheduled_domains
                {
                    self.violation = Some(format!(
                        "FRAME_END domain counts at byte {at_byte} claim emitted={emitted_domains} scheduled={scheduled_domains}, but observed {observed_terminal_outcomes} unique non-probe terminal outcomes (DOMAIN_PERF or DOMAIN_GAP)"
                    ));
                    return;
                }
                match self.open_hour.or(self.pending_perf_hour) {
                    Some(open) if open == header.game_hour_raw => {}
                    Some(open) => {
                        self.violation = Some(format!(
                            "mismatched FRAME_END at byte {at_byte}: closes hour {} \
                             while hour {open} is open",
                            header.game_hour_raw
                        ));
                        return;
                    }
                    None => {
                        self.violation = Some(format!(
                            "FRAME_END at byte {at_byte} closes hour {} without a terminal domain outcome",
                            header.game_hour_raw
                        ));
                        return;
                    }
                }
                if let Some(last_closed) = self.last_closed_hour {
                    if header.game_hour_raw <= last_closed {
                        self.violation = Some(format!(
                            "non-monotonic FRAME_END at byte {at_byte}: closed hour \
                             {last_closed}, then {}",
                            header.game_hour_raw
                        ));
                        return;
                    }
                }
                self.open_hour = None;
                self.pending_perf_hour = None;
                self.terminal_outcome_domains.clear();
                self.gap_domains.clear();
                self.data_domains.clear();
                self.last_closed_hour = Some(header.game_hour_raw);
            }
            COMPACT_RECORD_TYPE_FRAME_REFUSED => {
                // A refused frame IS a closed frame -- it closed without
                // publishing. It therefore clears the per-hour sets exactly as
                // FRAME_END does. Leaving them populated (what the old `_ => {}`
                // did) made hour N's outcomes bleed into hour N+1 and fired a
                // FALSE "duplicate terminal outcome" violation on the first real
                // refusal stream.
                //
                // Two things FRAME_END requires are deliberately NOT required
                // here: there is no seal to validate, because nothing was
                // published; and the hour needs no separate terminal domain
                // outcome, because the refusal record IS that outcome and it
                // names every domain responsible.
                //
                // What IS required, exactly as for FRAME_END: a refusal may only
                // close the hour that is actually open. Without this a refusal
                // naming hour C while hour B was still open silently discarded
                // B -- the stream certified while an hour that had already
                // published DOMAIN_DATA was neither closed nor accounted for.
                // `None` is admissible here and is not for FRAME_END: an hour
                // that published nothing at all is the ORDINARY shape of a
                // refusal, and there is nothing to abandon.
                match self.open_hour.or(self.pending_perf_hour) {
                    Some(open) if open == header.game_hour_raw => {}
                    Some(open) => {
                        self.violation = Some(format!(
                            "mismatched FRAME_REFUSED at byte {at_byte}: refuses hour {} \
                             while hour {open} is open",
                            header.game_hour_raw
                        ));
                        return;
                    }
                    None => {}
                }
                // And the record must refuse the hour its own envelope names.
                // The payload's `refused_hour` is what a consumer reads to learn
                // WHICH hour was refused; the envelope hour is what this fold
                // accounts. Nothing else compares them, so a record disagreeing
                // with itself would account for one hour and report another.
                // Either refusal version: version 2 is what the current
                // collector writes, version 1 is what retained streams hold.
                // Decoding only one would report every record of the other
                // version as a violation and abandon the fold.
                match compact_contract::frame_refusal::decode_frame_refusal_any_version(payload) {
                    Ok(record) if record.refused_hour == header.game_hour_raw => {}
                    Ok(record) => {
                        self.violation = Some(format!(
                            "FRAME_REFUSED at byte {at_byte} is framed for hour {} but its \
                             payload refuses hour {}",
                            header.game_hour_raw, record.refused_hour
                        ));
                        return;
                    }
                    Err(reason) => {
                        self.violation = Some(format!(
                            "FRAME_REFUSED payload at byte {at_byte} is unreadable: {reason}"
                        ));
                        return;
                    }
                }
                if let Some(last_closed) = self.last_closed_hour {
                    if header.game_hour_raw <= last_closed {
                        self.violation = Some(format!(
                            "non-monotonic FRAME_REFUSED at byte {at_byte}: closed hour \
                             {last_closed}, then {}",
                            header.game_hour_raw
                        ));
                        return;
                    }
                }
                self.open_hour = None;
                self.pending_perf_hour = None;
                self.terminal_outcome_domains.clear();
                self.gap_domains.clear();
                self.data_domains.clear();
                self.last_closed_hour = Some(header.game_hour_raw);
            }
            COMPACT_RECORD_TYPE_FRAME_DEGRADED => {
                // NOT a boundary record. A degraded frame still ends with a
                // canonical FRAME_END, because it PUBLISHED -- the notice only
                // says which of its domains are unknown. So this arm closes
                // nothing and clears nothing; doing either would close the hour
                // early and orphan the DATA and FRAME_END that follow.
                //
                // What it does enforce is that the notice belongs to the hour it
                // rides in. The payload's `degraded_hour` is what a consumer
                // reads to learn WHICH hour is degraded, and the envelope hour is
                // what the surrounding records are accounted under. Nothing else
                // compares them, so a record disagreeing with itself would mark
                // one hour degraded while a different hour carried the data --
                // and an hour wrongly believed clean is the exact failure this
                // record exists to prevent.
                let open = self.open_hour.or(self.pending_perf_hour);
                match open {
                    Some(open) if open == header.game_hour_raw => {}
                    Some(open) => {
                        self.violation = Some(format!(
                            "mismatched FRAME_DEGRADED at byte {at_byte}: degrades hour {} \
                             while hour {open} is open",
                            header.game_hour_raw
                        ));
                        return;
                    }
                    None => {
                        // Unlike a refusal, a degraded notice cannot stand alone:
                        // it annotates a frame that published, so that frame's
                        // records must already have opened the hour.
                        self.violation = Some(format!(
                            "FRAME_DEGRADED at byte {at_byte} degrades hour {} but no hour is open",
                            header.game_hour_raw
                        ));
                        return;
                    }
                }
                match compact_contract::frame_degraded::FrameDegradedRecordV1::decode(payload) {
                    Ok(record) if record.degraded_hour == header.game_hour_raw => {}
                    Ok(record) => {
                        self.violation = Some(format!(
                            "FRAME_DEGRADED at byte {at_byte} is framed for hour {} but its \
                             payload degrades hour {}",
                            header.game_hour_raw, record.degraded_hour
                        ));
                    }
                    Err(reason) => {
                        self.violation = Some(format!(
                            "FRAME_DEGRADED payload at byte {at_byte} is unreadable: {reason}"
                        ));
                    }
                }
            }
            // Not a frame-boundary record: it carries campaign identity and is
            // validated where identity is folded, not here.
            compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER => {}
            // A record type this fold has not been taught is a violation, not a
            // skip. Silence here is how a consumer drifts out of date while
            // still reporting success.
            other => {
                self.violation = Some(format!(
                    "unknown compact record type {other} at byte {at_byte}: this fold \
                     has not been taught to close, count, or ignore it"
                ));
            }
        }
    }

    /// Ok(()) when the stream ended at a CLEAN closed boundary; Err(reason).
    pub fn eof_check(&self, residual_carry: usize) -> Result<(), String> {
        if let Some(v) = &self.violation {
            return Err(v.clone());
        }
        if residual_carry != 0 {
            return Err(format!(
                "stream ends mid-record ({residual_carry} residual bytes; partial \
                 header or incomplete payload)"
            ));
        }
        if let Some(open) = self.open_hour {
            return Err(format!("game hour {open} left open at EOF"));
        }
        if self.last_closed_hour.is_none() {
            return Err("stream closes no game hour (no FRAME_END or FRAME_REFUSED records)".to_string());
        }
        // A stream may legitimately end on a refusal: that hour was closed, it
        // simply closed without publishing. Requiring FRAME_END here would call
        // an honestly-refused stream malformed.
        if !matches!(
            self.last_record_kind,
            Some(COMPACT_RECORD_TYPE_FRAME_END) | Some(COMPACT_RECORD_TYPE_FRAME_REFUSED)
        ) {
            return Err(format!(
                "stream ends after record type {}; final record must be a canonical FRAME_END or FRAME_REFUSED boundary",
                self.last_record_kind
                    .map_or_else(|| "none".to_string(), |kind| kind.to_string())
            ));
        }
        Ok(())
    }
}

/// One fail-closed certification verdict shared by extractor-side stream
/// consumers. Diagnostic/raw recovery may still yield complete prefix hours,
/// but no certification gate may accept framing stops, torn tails, malformed
/// domain payloads, or a stream that does not end on a clean closed-hour
/// boundary.
pub fn strict_stream_certification_verdict(
    stop_reason: Option<&str>,
    truncated_tail_bytes: u64,
    malformed_payload_records: u64,
    ends_at_frame_end: bool,
    boundary_error: Option<&str>,
) -> Result<(), String> {
    let mut failures = Vec::new();
    if let Some(reason) = stop_reason {
        failures.push(format!("framing stopped: {reason}"));
    }
    if truncated_tail_bytes != 0 {
        failures.push(format!(
            "truncated/corrupt tail contains {truncated_tail_bytes} byte(s)"
        ));
    }
    if malformed_payload_records != 0 {
        failures.push(format!(
            "{malformed_payload_records} DOMAIN_DATA payload record(s) failed exact validation"
        ));
    }
    if let Some(reason) = boundary_error {
        failures.push(format!("stream boundary is not closed: {reason}"));
    } else if !ends_at_frame_end {
        failures.push(
            "stream iteration is not finalized at a certified closed-hour boundary".to_string(),
        );
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "stream is not certifiable: {}",
            failures.join("; ")
        ))
    }
}

/// Exact-DATA and logical-boundary check of an on-disk compact stream — the
/// SEAL-TIME gate for containers packed WITHOUT a host fold (skip-fold /
/// fast-turnaround), so the production writer can never publish a container
/// that is guaranteed unreadable under the sealed-source strictness
/// (container re-gate round 2, finding 3). Runs the same hour-aware state
/// machine as `drive_fold_compressed` but folds nothing: hard-errors on any
/// framing error, malformed domain payload, residual tail bytes, an hour left
/// open at EOF, or a stream that closes no hour.
pub fn verify_stream_logical_boundary(stream_path: &Path) -> io::Result<()> {
    verify_stream_logical_boundary_with_identity_mode(stream_path, StreamIdentityMode::Legacy)
}

/// Identity-mode-explicit seal walker. CID1 is consumed by the shared validator
/// and retained as protocol state; it is never treated as an ordinary boundary
/// record. No first-record sniffing or protocol fallback is permitted.
pub fn verify_stream_logical_boundary_with_identity_mode(
    stream_path: &Path,
    identity_mode: StreamIdentityMode,
) -> io::Result<()> {
    let mut reader = compact_stream::open(stream_path)?;
    let mut carry: Vec<u8> = Vec::with_capacity(CHUNK * 2);
    let mut chunk = vec![0u8; CHUNK];
    let mut processed: u64 = 0;
    let mut tracker = BoundaryTracker::default();
    let mut validator = stream_validator(identity_mode);
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        carry.extend_from_slice(&chunk[..n]);
        let mut off = 0usize;
        loop {
            let remaining = carry.len() - off;
            if remaining < COMPACT_RECORD_HEADER_LEN {
                break;
            }
            let header = parse_compact_record_header(&carry[off..]).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "seal-time boundary check: {e} at byte {}",
                        processed + off as u64
                    ),
                )
            })?;
            let record_len = header.record_len();
            if record_len > remaining {
                break;
            }
            let payload = &carry[off + COMPACT_RECORD_HEADER_LEN..off + record_len];
            let accepted = validator
                .validate_record(&header, payload)
                .map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "seal-time stream validation failed at byte {}: {error}",
                            processed + off as u64
                        ),
                    )
                })?;
            if accepted == AcceptedRecordKind::CampaignIdentity {
                off += record_len;
                continue;
            }
            if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
                validate_domain_data_payload_at(
                    header.domain_id,
                    payload,
                    header.record_flags,
                    header.sequence,
                    header.game_hour_raw,
                    processed + off as u64,
                )
                    .map_err(|reason| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "seal-time boundary check: domain {} DATA payload failed exact validation at byte {}: {reason}",
                                header.domain_id,
                                processed + off as u64
                            ),
                        )
                    })?;
            }
            tracker.observe(&header, payload, processed + off as u64);
            off += record_len;
        }
        processed += off as u64;
        carry.drain(0..off);
    }
    tracker.eof_check(carry.len()).map_err(|reason| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("seal-time boundary check: {reason}"),
        )
    })
}

/// Default worker count for the parallel post-capture fold. The fold is OFFLINE
/// batch work (post-capture finalization / re-fold tooling), where the
/// weaker-PC law's "don't lean on cores" constraint does not apply — that law
/// governs CAPTURE-time per-frame work.
///
/// cores/2, capped at 8: measured on the deep-war 512 MiB prefix (8-core rig),
/// 4 workers (11.15 MB/s) beats 8 workers (10.2 MB/s) — the fold is
/// memory-bandwidth/allocation-bound, and each fold worker also owns an emit
/// worker thread, so cores/2 fold workers already saturates the machine. The
/// cap also bounds peak memory (each worker holds a full fold-state copy).
pub fn default_fold_workers() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    (cores / 2).clamp(1, 8)
}

/// Parallel bounded-memory fold of an on-disk compact stream: `workers`
/// independent streaming folds each walk the WHOLE stream (fold state is
/// carried across hours, so every worker must apply every record — record
/// application is ~10% of fold cost), but worker `k` BUILDS + spills only the
/// hours whose emission ordinal is `≡ k (mod workers)`. Because
/// `WorldFold::build_snapshot` is `&self`-pure, skipping it on non-owned hours
/// cannot perturb any worker's state, so the union of the workers' outputs is
/// byte-identical to the sequential fold's output — same files, same names,
/// same canonical bytes, same hashes (G1-preserving by construction).
///
/// Returns the merged snapshot descriptors sorted by `hour_index` (identical
/// ordering to the sequential fold) and the pass accounting from worker 0
/// (every worker walks identical bytes, so their `FoldStreamStats` must agree;
/// divergence fails loudly). `workers <= 1` delegates to the sequential fold.
pub fn fold_compact_stream_file_parallel(
    stream_path: &Path,
    game_id: &str,
    snapshot_dir: &Path,
    workers: usize,
) -> io::Result<(Vec<FoldedSnapshotFile>, FoldStreamStats)> {
    fold_compact_stream_file_parallel_with_identity_mode(
        stream_path,
        game_id,
        snapshot_dir,
        workers,
        StreamIdentityMode::Legacy,
    )
    .map(|(files, outcome)| (files, outcome.stats))
}

/// Identity-mode-explicit form of [`fold_compact_stream_file_parallel`]. Each
/// worker independently validates the same declared protocol and must agree on
/// the retained CID1 authority as well as stream accounting.
pub fn fold_compact_stream_file_parallel_with_identity_mode(
    stream_path: &Path,
    game_id: &str,
    snapshot_dir: &Path,
    workers: usize,
    identity_mode: StreamIdentityMode,
) -> io::Result<(Vec<FoldedSnapshotFile>, IdentityAwareFoldStats)> {
    if workers <= 1 {
        return fold_compact_stream_file_with_identity_mode(
            stream_path,
            game_id,
            snapshot_dir,
            identity_mode,
        );
    }
    // A worker that ends up owning zero hours is only wasted CPU, never a
    // correctness problem, so no up-front knowledge of the hour count is needed.
    let results: Vec<io::Result<(Vec<FoldedSnapshotFile>, IdentityAwareFoldStats)>> =
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..workers)
                .map(|k| {
                    scope.spawn(
                        move || -> io::Result<(Vec<FoldedSnapshotFile>, IdentityAwareFoldStats)> {
                            let reader = compact_stream::open(stream_path)?;
                            let compressed =
                                reader.compression() == compact_stream::StreamCompression::Zstd;
                            let total_len = if compressed { 0 } else { reader.encoded_len() };
                            let mut fold = WorldFold::new_file_backed_strided(
                                game_id,
                                snapshot_dir,
                                workers as u64,
                                k as u64,
                            )?;
                            let outcome = drive_fold_compressed_with_identity_mode(
                                reader,
                                total_len,
                                &mut fold,
                                compressed,
                                identity_mode,
                            )?;
                            let files = fold
                                .finish_file_backed()
                                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
                            Ok((files, outcome))
                        },
                    )
                })
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| {
                        Err(io::Error::new(
                            io::ErrorKind::Other,
                            "parallel fold worker panicked",
                        ))
                    })
                })
                .collect()
        });

    let mut merged: Vec<FoldedSnapshotFile> = Vec::new();
    let mut outcome: Option<IdentityAwareFoldStats> = None;
    for result in results {
        let (files, worker_outcome) = result?;
        // Every worker framed identical bytes; any accounting divergence means a
        // non-deterministic walk and MUST fail loudly rather than merge.
        if let Some(reference_outcome) = &outcome {
            let reference = &reference_outcome.stats;
            let worker_stats = &worker_outcome.stats;
            let agree = reference.stream_bytes == worker_stats.stream_bytes
                && reference.bytes_folded == worker_stats.bytes_folded
                && reference.records_total == worker_stats.records_total
                && reference.hours_emitted == worker_stats.hours_emitted
                && reference.stop_reason == worker_stats.stop_reason
                // Boundary facts join the agreement predicate (re-gate round 2
                // finding 4): a worker seeing a different file view must fail
                // the merge, not silently supply the stored stats.
                && reference.truncated_tail_bytes == worker_stats.truncated_tail_bytes
                && reference.ends_at_frame_end == worker_stats.ends_at_frame_end
                // Content digest: same-shape-different-payload source views
                // (e.g. file replaced between worker opens) must fail the
                // merge, not silently mix snapshots from two stream versions.
                && reference.decoded_sha256 == worker_stats.decoded_sha256
                && reference_outcome.campaign_identity == worker_outcome.campaign_identity
                && reference_outcome.validated_cid1 == worker_outcome.validated_cid1;
            if !agree {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "parallel fold workers disagree on stream accounting: \
                         {reference:?} vs {worker_stats:?}"
                    ),
                ));
            }
        } else {
            outcome = Some(worker_outcome);
        }
        merged.extend(files);
    }
    merged.sort_by_key(|f| f.hour_index);
    let outcome = outcome.expect("workers >= 2 always yields stats");
    let stats = &outcome.stats;
    if merged.len() as u64 != stats.hours_emitted {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "parallel fold merged {} snapshot files but the stream closed {} hours",
                merged.len(),
                stats.hours_emitted
            ),
        ));
    }
    Ok((merged, outcome))
}

/// Hash spilled snapshot files for the offline report. When `discard_snapshots`
/// is true, each file is removed only after its hash and byte count are captured.
pub fn hash_folded_snapshot_files(
    files: &[FoldedSnapshotFile],
    discard_snapshots: bool,
) -> io::Result<Vec<FoldedSnapshotReportRow>> {
    let mut rows = Vec::with_capacity(files.len());
    for f in files {
        // Prefer the hash the emit worker already computed over the exact bytes it
        // wrote — avoids re-reading (and re-hashing) every multi-MB snapshot from
        // disk. Fall back to a fresh read only for snapshots without a stored hash.
        let snapshot_hash = match &f.sha256 {
            Some(hash) => hash.clone(),
            None => sha256_file(&f.path)
                .map_err(|e| io::Error::new(e.kind(), format!("hash {}: {e}", f.path.display())))?,
        };
        rows.push(FoldedSnapshotReportRow {
            hour_index: f.hour_index,
            date: f.date.clone(),
            snapshot_hash,
            snapshot_bytes: f.byte_len,
        });
        if discard_snapshots {
            std::fs::remove_file(&f.path).map_err(|e| {
                io::Error::new(e.kind(), format!("remove {}: {e}", f.path.display()))
            })?;
        }
    }
    Ok(rows)
}

pub fn snapshot_chain_hash(rows: &[FoldedSnapshotReportRow]) -> String {
    let mut chain = Sha256::new();
    for row in rows {
        chain.update(row.hour_index.to_le_bytes().as_slice());
        chain.update(row.snapshot_hash.as_bytes());
        chain.update(b"\n");
    }
    to_hex(&chain.finalize())
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finalize()))
}

/// Validate one DOMAIN_DATA payload through the canonical per-domain decoding
/// authority used by the manifest and compact verification gate. The record
/// header's declared payload length is not trusted: exact consumption by the
/// domain grammar is the independent delimiter. Missing fold baselines and
/// cross-domain semantic joins are deliberately irrelevant to this structural
/// boundary verdict.
pub fn validate_domain_data_payload(
    domain_id: u16,
    payload: &[u8],
    record_flags: u32,
) -> Result<(), String> {
    validate_domain_data_payload_at(domain_id, payload, record_flags, 0, 0, 0)
}

fn validate_domain_data_payload_at(
    domain_id: u16,
    payload: &[u8],
    record_flags: u32,
    sequence: u64,
    header_hour_raw: i32,
    stream_offset: u64,
) -> Result<(), String> {
    domain_data_payload_status_at(
        domain_id,
        payload,
        record_flags,
        sequence,
        header_hour_raw,
        stream_offset,
    )
    .map(|_| ())
}

/// The status word a `DOMAIN_DATA` payload carries INSIDE ITSELF, read by the
/// one authority that already decodes every domain's payload.
///
/// This is the same call `validate_domain_data_payload` makes; that wrapper
/// threw the status away with `.map(|_| ())`. Throwing it away is what let the
/// two statuses a frame carries — the one embedded in the DATA payload and the
/// one in the matching `DOMAIN_PERF` record — go unchecked against each other.
/// `WorldFold` decides whether to apply a domain's rows from the EMBEDDED one
/// (35 of its 90 fold functions read it directly, at three different payload
/// offsets), while stream certification reads only the PERF one. A payload
/// declaring a failure while its PERF declares success therefore made the fold
/// silently withhold every row of that domain from an hour that certified as
/// complete.
///
/// `None` means this domain's payload authority does not decode a status word
/// at all, which is a different fact from "the status was clean" and is
/// deliberately not collapsed into one.
pub fn domain_data_payload_status(
    domain_id: u16,
    payload: &[u8],
    record_flags: u32,
) -> Result<Option<u32>, String> {
    domain_data_payload_status_at(domain_id, payload, record_flags, 0, 0, 0)
}

fn domain_data_payload_status_at(
    domain_id: u16,
    payload: &[u8],
    record_flags: u32,
    sequence: u64,
    header_hour_raw: i32,
    stream_offset: u64,
) -> Result<Option<u32>, String> {
    summarize_domain_data_payload(
        domain_id,
        payload,
        DomainDataContext {
            stream_offset: usize::try_from(stream_offset).unwrap_or(usize::MAX),
            record_flags,
            sequence,
            header_hour_raw,
            session_header_identity: None,
        },
    )
    .map(|authority| authority.status_flags)
    .map_err(|error| error.to_string())
}

/// Drive the production fold over a record stream with a bounded carry buffer.
/// Every `DOMAIN_DATA`/`FRAME_END` record is handed to [`WorldFold::ingest_record`]
/// exactly as `WorldFold::ingest_stream` would; `DOMAIN_PERF`/probe records carry
/// no folded state and are counted but ignored. A file-backed fold spills each
/// closed hour to disk inside `ingest_record`, so nothing accumulates here.
pub fn drive_fold<R: Read>(
    mut reader: R,
    total_len: u64,
    fold: &mut WorldFold,
) -> io::Result<FoldStreamStats> {
    drive_fold_with_identity_mode(reader.by_ref(), total_len, fold, StreamIdentityMode::Legacy)
        .map(|outcome| outcome.stats)
}

/// Identity-mode-explicit uncompressed fold driver.
pub fn drive_fold_with_identity_mode<R: Read>(
    mut reader: R,
    total_len: u64,
    fold: &mut WorldFold,
    identity_mode: StreamIdentityMode,
) -> io::Result<IdentityAwareFoldStats> {
    drive_fold_compressed_with_identity_mode(reader.by_ref(), total_len, fold, false, identity_mode)
}

/// `compressed` = the reader is a checksummed zstd stream. On such a source an
/// H4CR framing stop before physical EOF drains the reader to EOF (forcing the
/// zstd footer + content-checksum check) and then hard-errors — corruption of
/// the decoded content fails the checksum, and an intact-storage framing stop
/// is itself garbage. Raw sources keep the tolerant torn-tail behavior the
/// deep-war corpus relies on.
pub fn drive_fold_compressed<R: Read>(
    mut reader: R,
    total_len: u64,
    fold: &mut WorldFold,
    compressed: bool,
) -> io::Result<FoldStreamStats> {
    drive_fold_compressed_with_identity_mode(
        reader.by_ref(),
        total_len,
        fold,
        compressed,
        StreamIdentityMode::Legacy,
    )
    .map(|outcome| outcome.stats)
}

/// Identity-mode-explicit fold driver. The shared stateful validator owns CID1
/// placement before records reach boundary/fold dispatch. Ordinary-record
/// sequence and envelope authority remains with [`BoundaryTracker`], whose
/// diagnostic fold deliberately permits a malformed domain payload to degrade
/// only that domain while the recording continues.
pub fn drive_fold_compressed_with_identity_mode<R: Read>(
    mut reader: R,
    total_len: u64,
    fold: &mut WorldFold,
    compressed: bool,
    identity_mode: StreamIdentityMode,
) -> io::Result<IdentityAwareFoldStats> {
    let mut stats = FoldStreamStats {
        stream_bytes: total_len,
        ..FoldStreamStats::default()
    };
    let mut campaign_identity = None;
    let mut validated_cid1 = None;
    let mut carry: Vec<u8> = Vec::with_capacity(CHUNK * 2);
    let mut chunk = vec![0u8; CHUNK];
    let mut processed: u64 = 0;
    let mut decoded_read: u64 = 0;
    // Shared hour-aware closure state machine (see BoundaryTracker: hour
    // interleaving, mismatched FRAME_END, and sequence discontinuities are
    // sticky violations; only a clean tracker at EOF is a closed boundary).
    let mut tracker = BoundaryTracker::default();
    let mut validator = stream_validator(identity_mode);
    let mut decoded_hasher = Sha256::new();

    'read: loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        decoded_read = decoded_read.saturating_add(n as u64);
        decoded_hasher.update(&chunk[..n]);
        carry.extend_from_slice(&chunk[..n]);

        let mut off = 0usize;
        let mut hard_stop = false;
        loop {
            let remaining = carry.len() - off;
            if remaining < COMPACT_RECORD_HEADER_LEN {
                break;
            }
            // The single H4CR framing primitive (compact_contract); malformed
            // headers stop the walk with the same diagnostic strings as before.
            let header = match parse_compact_record_header(&carry[off..]) {
                Ok(h) => h,
                Err(e) => {
                    let at = processed + off as u64;
                    stats.stop_reason = Some(match e {
                        RecordFramingError::TruncatedHeader(actual) => {
                            format!("truncated record header ({actual} byte(s)) at byte {at}")
                        }
                        RecordFramingError::BadMagic => {
                            format!("invalid record magic at byte {at}")
                        }
                        RecordFramingError::UnsupportedVersion(v) => {
                            format!("unsupported record version {v} at byte {at}")
                        }
                        RecordFramingError::UnsupportedHeaderLen(l) => {
                            format!("unsupported header length {l} at byte {at}")
                        }
                        RecordFramingError::PayloadTooLarge(l) => {
                            format!("payload length {l} exceeds the record bound at byte {at}")
                        }
                    });
                    hard_stop = true;
                    break;
                }
            };
            let record_len = header.record_len();
            if record_len > remaining {
                break; // payload not fully buffered yet
            }
            let payload = &carry[off + COMPACT_RECORD_HEADER_LEN..off + record_len];

            let is_cid1_envelope = header.sequence == 0
                && header.domain_id
                    == compact_contract::campaign_identity::COMPACT_DOMAIN_STREAM_HEADER
                && header.record_type
                    == compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;
            if identity_mode == StreamIdentityMode::Legacy && is_cid1_envelope {
                let error = validator
                    .validate_record(&header, payload)
                    .expect_err("Legacy validator must reject an IdentityV1 CID1 envelope");
                stats.stop_reason = Some(format!(
                    "stream validation failed at byte {}: {error}",
                    processed + off as u64
                ));
                hard_stop = true;
                break;
            }
            if identity_mode == StreamIdentityMode::IdentityV1 && validated_cid1.is_none() {
                match validator.validate_record(&header, payload) {
                    Ok(AcceptedRecordKind::CampaignIdentity) => {
                        validated_cid1 = validator.identity().cloned();
                        campaign_identity = validator.identity().map(FoldedCampaignIdentity::from);
                        off += record_len;
                        continue;
                    }
                    Ok(AcceptedRecordKind::Ordinary) => unreachable!(
                        "IdentityV1 validator cannot accept an ordinary record before CID1"
                    ),
                    Err(error) => {
                        stats.stop_reason = Some(format!(
                            "stream validation failed at byte {}: {error}",
                            processed + off as u64
                        ));
                        hard_stop = true;
                        break;
                    }
                }
            }

            if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
                if let Err(reason) = validate_domain_data_payload_at(
                    header.domain_id,
                    payload,
                    header.record_flags,
                    header.sequence,
                    header.game_hour_raw,
                    processed + off as u64,
                ) {
                    // Zero-data-loss ruling (owner 2026-07-14): a structurally-malformed
                    // DOMAIN_DATA payload degrades ONLY its own domain — never the whole
                    // recording. Frame accounting still counts this well-framed record;
                    // the untrusted payload is not applied. Framing/version/checksum
                    // corruption remains fail-closed below.
                    stats.records_total += 1;
                    stats.data_records += 1;
                    stats.malformed_payload_records += 1;
                    let malformed = stats
                        .malformed_payload_domains
                        .entry(header.domain_id)
                        .or_insert_with(|| MalformedPayloadDomainStats {
                            records: 0,
                            first_reason: reason.clone(),
                            first_stream_offset: processed + off as u64,
                            first_game_hour_raw: header.game_hour_raw,
                        });
                    malformed.records = malformed.records.saturating_add(1);
                    tracker.observe(&header, payload, processed + off as u64);
                    fold.record_malformed_domain_payload(
                        header.game_hour_raw,
                        header.domain_id,
                        payload,
                        &reason,
                    );
                    off += record_len;
                    continue;
                }
            }

            stats.records_total += 1;
            match header.record_type {
                COMPACT_RECORD_TYPE_DOMAIN_DATA => stats.data_records += 1,
                COMPACT_RECORD_TYPE_FRAME_END => stats.frame_end_records += 1,
                COMPACT_RECORD_TYPE_DOMAIN_PERF => stats.perf_records += 1,
                _ => stats.other_records += 1,
            }
            tracker.observe(&header, payload, processed + off as u64);

            fold.ingest_record(
                header.sequence,
                header.game_hour_raw,
                header.domain_id,
                header.record_type,
                payload,
                header.record_flags,
            );
            off += record_len;
        }

        processed += off as u64;
        carry.drain(0..off);
        if hard_stop {
            if compressed {
                // Force zstd footer + content-checksum validation before
                // trusting this stop; a corrupt frame errors here rather than
                // reading as a clean short fold.
                let mut sink = [0u8; 64 * 1024];
                loop {
                    match reader.read(&mut sink)? {
                        0 => break,
                        _ => continue,
                    }
                }
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "H4CR framing or stream validation stopped before physical EOF on a compressed \
                         stream at byte {processed}: {} (storage checksum valid, so the \
                         declared decoded-stream protocol is invalid)",
                        stats
                            .stop_reason
                            .as_deref()
                            .unwrap_or("unknown framing stop")
                    ),
                ));
            }
            break 'read;
        }
    }

    // A file-backed fold spills as it goes; `emitted_hours` is the authoritative
    // count of closed game-hours in either mode.
    if total_len == 0 {
        stats.stream_bytes = decoded_read;
    }
    stats.bytes_folded = processed;
    stats.truncated_tail_bytes = stats.stream_bytes.saturating_sub(processed);
    stats.hours_emitted = fold.emitted_hours_count() as u64;
    stats.ends_at_frame_end = tracker.eof_check(carry.len()).is_ok();
    stats.decoded_sha256 = to_hex(&decoded_hasher.finalize());
    if compressed {
        // A SEALED (checksummed zstd) container must end at a valid LOGICAL
        // boundary, not merely a valid storage boundary: no residual carry,
        // no open hour, no boundary violation (hour interleave / mismatched
        // FRAME_END / sequence gap), at least one closed hour. RAW sources
        // keep the tolerant torn-tail behavior byte-identically.
        if let Err(reason) = tracker.eof_check(carry.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("compressed stream fails the closed-boundary law: {reason}"),
            ));
        }
    }
    Ok(IdentityAwareFoldStats {
        stats,
        campaign_identity,
        validated_cid1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::calendar::DATE_EPOCH_CONST;
    use compact_contract::campaign_identity::{
        encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses, COMPACT_DOMAIN_STREAM_HEADER,
        COMPACT_RECORD_TYPE_STREAM_HEADER,
    };
    use compact_contract::{
        CompactRecordHeader, COMPACT_DOMAIN_FRAME, COMPACT_DOMAIN_PERF_BASE_LEN,
        COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC, COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION,
        COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
        COMPACT_RECORD_MAGIC, COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_FRAME_END,
        COMPACT_RECORD_VERSION, COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_PARTIAL_DATA,
        COMPACT_STATUS_SOURCE_UNAVAILABLE,
    };
    use snapshot::sha256::{to_hex, Sha256};
    use std::io::Cursor;
    use std::path::PathBuf;

    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(tag: &str) -> TmpDir {
            let mut p = std::env::temp_dir();
            p.push(format!(
                "hoi4_stream_fold_{tag}_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A refusal stream must fold cleanly, and a record type this fold has not
    /// been taught must be a violation rather than a skip.
    ///
    /// Both halves reproduce real defects. The fold dispatched on record type
    /// with a bare `_ => {}`, so FRAME_REFUSED was invisible -- and because only
    /// FRAME_END cleared the per-hour sets, hour N's gap for a domain survived
    /// into hour N+1 and tripped "duplicate terminal outcome": a FALSE violation
    /// reported against a perfectly honest stream. This is the exact shape of
    /// the production stream that first carried refusals (348 refusals, 146
    /// gaps, 2 frame ends).
    ///
    /// Mutation proof: put `_ => {}` back in place of the FRAME_REFUSED arm and
    /// the first assertion fails with that duplicate-terminal-outcome message;
    /// put it back in place of the `other =>` arm and the last assertion fails.
    #[test]
    fn a_refusal_closes_its_hour_and_an_untaught_type_is_a_violation() {
        let gap_for = |hour: i32| {
            compact_contract::DomainGapRecordV1 {
                part: compact_contract::DomainGapPart::Both,
                reason: compact_contract::DomainGapReason::ClockUnreadable,
                target_hour_raw: hour,
                range_start_hour_raw: hour,
                range_end_hour_raw: hour,
                tick_before: 0,
                tick_after: 0,
                queue_capacity: 0,
                queue_depth: 0,
                queue_dropped: 0,
                outcome_ordinal: 0,
            }
            .encode()
            .expect("canonical gap fixture")
        };
        let refusal_for = |hour: i32| {
            compact_contract::frame_refusal::FrameRefusalRecordV1 {
                refused_hour: hour,
                offenders: vec![compact_contract::frame_refusal::FrameRefusalOffender {
                    domain_id: 64,
                    status_flags: 0x0000_0100,
                    non_tolerated_flags: 0x0000_0100,
                    reason: "d64_rejected_timed_effect_frames=1".to_string(),
                }],
            }
            .encode()
            .expect("canonical refusal fixture")
        };

        // The SAME domain gapped in two consecutive hours, each hour closed by a
        // refusal rather than a FRAME_END.
        let mut tracker = BoundaryTracker::default();
        let mut seq = 1u64;
        for hour_index in [HOUR_A, HOUR_B] {
            let hour = raw_for(hour_index);
            let gap = gap_for(hour);
            tracker.observe(
                &boundary_header(seq, hour, 64, COMPACT_RECORD_TYPE_DOMAIN_GAP, gap.len(), 0),
                &gap,
                0,
            );
            seq += 1;
            let refusal = refusal_for(hour);
            tracker.observe(
                &boundary_header(
                    seq,
                    hour,
                    COMPACT_DOMAIN_FRAME,
                    COMPACT_RECORD_TYPE_FRAME_REFUSED,
                    refusal.len(),
                    0,
                ),
                &refusal,
                0,
            );
            seq += 1;
        }
        assert!(
            tracker.violation.is_none(),
            "an honest refusal stream must fold clean, got {:?}",
            tracker.violation
        );

        // And it ends at a closed boundary: the hour WAS closed, it simply
        // closed without publishing.
        assert!(
            tracker.eof_check(0).is_ok(),
            "a stream ending on a refusal ended at a closed boundary, got {:?}",
            tracker.eof_check(0)
        );

        // An undefined record type is NOT skipped.
        let mut untaught = BoundaryTracker::default();
        untaught.observe(&boundary_header(1, raw_for(HOUR_A), 64, 61, 0, 0), &[], 0);
        assert!(
            untaught
                .violation
                .as_ref()
                .is_some_and(|v| v.contains("unknown compact record type")),
            "an untaught record type must be a violation, got {:?}",
            untaught.violation
        );
    }

    /// Law: a refusal closes the hour that is OPEN, and refuses the hour its
    /// own envelope names.
    ///
    /// The refusal arm clears `open_hour` unconditionally. On its own that
    /// means a refusal naming a later hour silently discards the open one: the
    /// stream then certifies while an hour that had already published
    /// DOMAIN_DATA is neither closed nor accounted for anywhere. FRAME_END has
    /// carried the matching rule since it existed; this is the same rule, minus
    /// the seal and the terminal-outcome requirement a refusal has no reason to
    /// meet.
    ///
    /// The envelope/payload agreement is the second half. `refused_hour` is
    /// what a consumer reads to learn WHICH hour was refused and the envelope
    /// hour is what this fold accounts; nothing else compares them.
    ///
    /// Mutation proof: delete either added `match` in the FRAME_REFUSED arm and
    /// the corresponding half of this test fails.
    #[test]
    fn a_refusal_closes_the_open_hour_and_refuses_the_hour_it_is_framed_for() {
        let refusal_for = |hour: i32| {
            compact_contract::frame_refusal::FrameRefusalRecordV1 {
                refused_hour: hour,
                offenders: vec![compact_contract::frame_refusal::FrameRefusalOffender {
                    domain_id: 64,
                    status_flags: 0x0000_0100,
                    non_tolerated_flags: 0x0000_0100,
                    reason: "d64_rejected_timed_effect_frames=1".to_string(),
                }],
            }
            .encode()
            .expect("canonical refusal fixture")
        };
        let open = raw_for(HOUR_A);
        let later = raw_for(HOUR_A + 1);

        // An hour with published DOMAIN_DATA may not be abandoned by a refusal
        // that names a different hour.
        let mut abandons = BoundaryTracker::default();
        observe_zero_payload(
            &mut abandons,
            boundary_header(1, open, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
            0,
        );
        let refusal = refusal_for(later);
        abandons.observe(
            &boundary_header(
                2,
                later,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_REFUSED,
                refusal.len(),
                0,
            ),
            &refusal,
            32,
        );
        assert!(
            abandons
                .violation
                .as_ref()
                .is_some_and(|v| v.contains("mismatched FRAME_REFUSED")),
            "a refusal must not silently discard an open hour, got {:?}",
            abandons.violation
        );

        // The ORDINARY production shape: the collector drops the staged frame
        // and writes a refusal-only buffer, so nothing for that hour was ever
        // published and neither `open_hour` nor `pending_perf_hour` is set. The
        // `None` arm exists for exactly this, and it must certify.
        let mut refusal_only = BoundaryTracker::default();
        let refusal = refusal_for(open);
        refusal_only.observe(
            &boundary_header(
                1,
                open,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_REFUSED,
                refusal.len(),
                0,
            ),
            &refusal,
            0,
        );
        assert!(
            refusal_only.violation.is_none(),
            "a refusal for an hour that published nothing is the ordinary shape, got {:?}",
            refusal_only.violation
        );
        assert_eq!(refusal_only.eof_check(0), Ok(()));

        // Refusing the hour that IS open is the other accepted shape.
        let mut closes = BoundaryTracker::default();
        observe_zero_payload(
            &mut closes,
            boundary_header(1, open, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
            0,
        );
        let refusal = refusal_for(open);
        closes.observe(
            &boundary_header(
                2,
                open,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_REFUSED,
                refusal.len(),
                0,
            ),
            &refusal,
            32,
        );
        assert!(
            closes.violation.is_none(),
            "a refusal closing its own open hour is honest, got {:?}",
            closes.violation
        );
        assert_eq!(closes.eof_check(0), Ok(()));

        // And a record whose envelope and payload name different hours is
        // refused even when nothing is open.
        let mut disagrees = BoundaryTracker::default();
        let refusal = refusal_for(later);
        disagrees.observe(
            &boundary_header(
                1,
                open,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_REFUSED,
                refusal.len(),
                0,
            ),
            &refusal,
            0,
        );
        assert!(
            disagrees
                .violation
                .as_ref()
                .is_some_and(|v| v.contains("payload refuses hour")),
            "a refusal must agree with its own envelope, got {:?}",
            disagrees.violation
        );
    }

    fn raw_for(hour_index: i64) -> i32 {
        DATE_EPOCH_CONST + hour_index as i32
    }

    fn push_record(
        out: &mut Vec<u8>,
        seq: u64,
        game_hour_raw: i32,
        domain: u16,
        ty: u16,
        payload: &[u8],
    ) {
        // Historical fixtures passed zero for FRAME_END. Keep their intent but
        // encode the canonical production domain now enforced by record semantics.
        let domain = if ty == COMPACT_RECORD_TYPE_FRAME_END {
            COMPACT_DOMAIN_FRAME
        } else {
            domain
        };
        out.extend_from_slice(COMPACT_RECORD_MAGIC);
        out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&seq.to_le_bytes());
        out.extend_from_slice(&game_hour_raw.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
    }

    fn push_identity_v1(out: &mut Vec<u8>, game_hour_raw: i32) {
        let observation = Cid1Observation {
            game_unique_id: "a4b17fed-ed12-4361-9064-fb6576638110".to_owned(),
            game_unique_seed: -947_102,
            game_hour_raw,
        };
        let fields = Cid1Fields::from_stable_observations(
            observation.clone(),
            observation,
            "vanilla_latest_2026_07_15".to_owned(),
            Cid1Witnesses::default(),
        )
        .unwrap();
        let payload = encode_cid1(&fields).unwrap();
        push_record(
            out,
            0,
            game_hour_raw,
            COMPACT_DOMAIN_STREAM_HEADER,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            &payload,
        );
    }

    fn frame_end_payload(
        emitted_domains: u32,
        scheduled_domains: u32,
        status_flags: u32,
        reserved: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::with_capacity(COMPACT_FRAME_END_PAYLOAD_LEN);
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&emitted_domains.to_le_bytes());
        payload.extend_from_slice(&scheduled_domains.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&reserved.to_le_bytes());
        payload
    }

    /// domain-18 country_identity payload (same wire layout the collector emits).
    fn country_identity_payload(rows: &[(u32, &str)]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes());
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        for (id, tag) in rows {
            p.extend_from_slice(&id.to_le_bytes());
            p.extend_from_slice(&(tag.len() as u16).to_le_bytes());
            p.extend_from_slice(tag.as_bytes());
            p.push(1);
        }
        p
    }

    fn append_complete_domain_record(payload: &mut Vec<u8>, hour: i32) {
        let mut swallowed = Vec::new();
        push_record(
            &mut swallowed,
            99,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(48, "FRA")]),
        );
        payload.extend_from_slice(&swallowed);
    }

    fn expect_domain_boundary_degrades(domain: u16, payload: &[u8]) {
        let hour = raw_for(HOUR_A);
        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            payload,
        );
        push_record(
            &mut stream,
            2,
            hour,
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; 32],
        );
        let mut fold = WorldFold::new("gid");
        let stats = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
            .expect("a swallow payload must degrade, not fail the fold");
        assert_eq!(stats.malformed_payload_records, 1, "domain {domain}");
        assert_eq!(
            stats.data_records, 1,
            "domain {domain}: the swallowed record must NOT be counted as a second framed record"
        );
    }

    fn expect_malformed_domain_degrades(domain: u16, payload: &[u8]) {
        let hour = raw_for(HOUR_A);
        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            payload,
        );
        let mut fold = WorldFold::new("malformed_boundary");
        let stats = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
            .expect("a malformed DATA payload must degrade its domain, not fail the fold");
        assert_eq!(stats.malformed_payload_records, 1, "domain {domain}");
        assert_eq!(stats.data_records, 1, "domain {domain}");
    }

    fn expect_domain_boundary_acceptance(domain: u16, payload: &[u8]) {
        let hour = raw_for(HOUR_A);
        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            payload,
        );
        push_record(
            &mut stream,
            2,
            hour,
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; 32],
        );
        let mut fold = WorldFold::new("production_boundary_acceptance");
        let stats = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
            .unwrap_or_else(|err| panic!("valid production d{domain} payload was rejected: {err}"));
        assert_eq!(stats.data_records, 1, "domain {domain}");
        assert_eq!(stats.bytes_folded, stream.len() as u64, "domain {domain}");
        assert!(stats.stop_reason.is_none(), "domain {domain}");
    }

    const HOUR_A: i64 = 16_959_360;
    const HOUR_B: i64 = 16_959_361;
    const HOUR_C: i64 = 16_959_362;

    fn boundary_header(
        sequence: u64,
        hour: i32,
        domain_id: u16,
        record_type: u16,
        payload_len: usize,
        record_flags: u32,
    ) -> CompactRecordHeader {
        CompactRecordHeader {
            sequence,
            game_hour_raw: hour,
            domain_id,
            record_type,
            payload_len,
            record_flags,
        }
    }

    fn observe_zero_payload(
        tracker: &mut BoundaryTracker,
        header: CompactRecordHeader,
        at_byte: u64,
    ) {
        let payload = vec![0u8; header.payload_len];
        tracker.observe(&header, &payload, at_byte);
    }

    #[test]
    fn identity_v1_fold_consumes_cid1_before_ordinary_boundary_and_fold_dispatch() {
        let hour = raw_for(HOUR_A);
        let mut stream = Vec::new();
        push_identity_v1(&mut stream, hour);
        push_record(
            &mut stream,
            1,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(48, "FRA")]),
        );
        push_record(
            &mut stream,
            2,
            hour,
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(0, 0, 0, 0),
        );

        let mut fold = WorldFold::new("identity_v1");
        let outcome = drive_fold_with_identity_mode(
            Cursor::new(stream.clone()),
            stream.len() as u64,
            &mut fold,
            StreamIdentityMode::IdentityV1,
        )
        .unwrap();
        let stats = &outcome.stats;

        assert_eq!(stats.records_total, 2);
        assert_eq!(stats.data_records, 1);
        assert_eq!(stats.frame_end_records, 1);
        assert_eq!(stats.other_records, 0);
        assert_eq!(stats.hours_emitted, 1);
        assert!(stats.ends_at_frame_end);
        let identity = outcome.campaign_identity.as_ref().unwrap();
        assert_eq!(
            identity.game_unique_id,
            "a4b17fed-ed12-4361-9064-fb6576638110"
        );
        assert_eq!(identity.game_unique_seed, -947_102);
        assert_eq!(identity.observed_game_hour_raw, hour);
        let manifest = outcome
            .validated_campaign_identity_manifest()
            .expect("validated CID1 must derive the exact Known manifest object");
        assert_eq!(manifest["state"], "known");
        assert_eq!(
            manifest["game_unique_id"],
            "a4b17fed-ed12-4361-9064-fb6576638110"
        );
        assert_eq!(manifest["game_unique_seed"], "-947102");
    }

    #[test]
    fn identity_protocol_selection_is_explicit_and_never_falls_back() {
        let hour = raw_for(HOUR_A);
        let mut identity_stream = Vec::new();
        push_identity_v1(&mut identity_stream, hour);
        let mut legacy_fold = WorldFold::new("legacy_rejects_cid1");
        let legacy = drive_fold(
            Cursor::new(identity_stream.clone()),
            identity_stream.len() as u64,
            &mut legacy_fold,
        )
        .unwrap();
        assert_eq!(legacy.bytes_folded, 0);
        assert!(legacy
            .stop_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("next required sequence 1")));

        let mut ordinary_stream = Vec::new();
        push_record(
            &mut ordinary_stream,
            1,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(48, "FRA")]),
        );
        let mut identity_fold = WorldFold::new("identity_rejects_legacy");
        let identity = drive_fold_with_identity_mode(
            Cursor::new(ordinary_stream.clone()),
            ordinary_stream.len() as u64,
            &mut identity_fold,
            StreamIdentityMode::IdentityV1,
        )
        .unwrap();
        assert_eq!(identity.stats.bytes_folded, 0);
        assert!(identity
            .stats
            .stop_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("invalid CID1 stream header")));
    }

    #[test]
    fn identity_v1_seal_walker_consumes_cid1_before_boundary_dispatch() {
        let hour = raw_for(HOUR_A);
        let mut stream = Vec::new();
        push_identity_v1(&mut stream, hour);
        push_record(
            &mut stream,
            1,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(48, "FRA")]),
        );
        push_record(
            &mut stream,
            2,
            hour,
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(0, 0, 0, 0),
        );

        let dir = TmpDir::new("identity_v1_seal");
        let path = dir.0.join("stream.bin");
        std::fs::write(&path, stream).unwrap();
        verify_stream_logical_boundary_with_identity_mode(&path, StreamIdentityMode::IdentityV1)
            .unwrap();
        let legacy = verify_stream_logical_boundary(&path).unwrap_err();
        assert!(legacy.to_string().contains("next required sequence 1"));
    }

    #[test]
    fn boundary_tracker_requires_sequence_one_and_rejects_wraparound() {
        let hour = raw_for(HOUR_A);
        let mut wrong_start = BoundaryTracker::default();
        observe_zero_payload(
            &mut wrong_start,
            boundary_header(2, hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
            0,
        );
        let err = wrong_start.eof_check(0).unwrap_err();
        assert!(err.contains("expected 1"), "{err}");

        let mut wrap = BoundaryTracker {
            prev_sequence: Some(u64::MAX),
            ..BoundaryTracker::default()
        };
        observe_zero_payload(
            &mut wrap,
            boundary_header(0, hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
            32,
        );
        let err = wrap.eof_check(0).unwrap_err();
        assert!(err.contains("sequence overflow"), "{err}");
    }

    #[test]
    fn boundary_tracker_allows_forward_gaps_but_rejects_non_monotonic_hours() {
        let first_hour = raw_for(HOUR_A);
        let forward_gap = raw_for(HOUR_A + 2);
        let mut accepted = BoundaryTracker::default();
        observe_zero_payload(
            &mut accepted,
            boundary_header(1, first_hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
            0,
        );
        observe_zero_payload(
            &mut accepted,
            boundary_header(
                2,
                first_hour,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                COMPACT_FRAME_END_PAYLOAD_LEN,
                0,
            ),
            32,
        );
        observe_zero_payload(
            &mut accepted,
            boundary_header(
                3,
                forward_gap,
                18,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                COMPACT_DOMAIN_PERF_BASE_LEN,
                0,
            ),
            64,
        );
        observe_zero_payload(
            &mut accepted,
            boundary_header(4, forward_gap, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
            96,
        );
        accepted.observe(
            &boundary_header(
                5,
                forward_gap,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                COMPACT_FRAME_END_PAYLOAD_LEN,
                0,
            ),
            &frame_end_payload(1, 1, 0, 0),
            128,
        );
        accepted
            .eof_check(0)
            .expect("a forward coverage gap remains a valid monotonic boundary");

        for next_hour in [first_hour, raw_for(HOUR_A - 1)] {
            let first_hour = raw_for(HOUR_A);
            let mut tracker = BoundaryTracker::default();
            observe_zero_payload(
                &mut tracker,
                boundary_header(1, first_hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
                0,
            );
            observe_zero_payload(
                &mut tracker,
                boundary_header(
                    2,
                    first_hour,
                    COMPACT_DOMAIN_FRAME,
                    COMPACT_RECORD_TYPE_FRAME_END,
                    COMPACT_FRAME_END_PAYLOAD_LEN,
                    0,
                ),
                32,
            );
            observe_zero_payload(
                &mut tracker,
                boundary_header(3, next_hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
                64,
            );
            observe_zero_payload(
                &mut tracker,
                boundary_header(
                    4,
                    next_hour,
                    COMPACT_DOMAIN_FRAME,
                    COMPACT_RECORD_TYPE_FRAME_END,
                    COMPACT_FRAME_END_PAYLOAD_LEN,
                    0,
                ),
                96,
            );
            let err = tracker.eof_check(0).unwrap_err();
            assert!(err.contains("non-monotonic DOMAIN_DATA"), "{err}");
        }
    }

    #[test]
    fn boundary_tracker_requires_frame_end_as_the_final_record() {
        let hour = raw_for(HOUR_A);
        for trailing_domain in [18, 250] {
            let mut tracker = BoundaryTracker::default();
            observe_zero_payload(
                &mut tracker,
                boundary_header(1, hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
                0,
            );
            observe_zero_payload(
                &mut tracker,
                boundary_header(
                    2,
                    hour,
                    COMPACT_DOMAIN_FRAME,
                    COMPACT_RECORD_TYPE_FRAME_END,
                    COMPACT_FRAME_END_PAYLOAD_LEN,
                    0,
                ),
                32,
            );
            observe_zero_payload(
                &mut tracker,
                boundary_header(
                    3,
                    hour + 1,
                    trailing_domain,
                    COMPACT_RECORD_TYPE_DOMAIN_PERF,
                    COMPACT_DOMAIN_PERF_BASE_LEN,
                    0,
                ),
                64,
            );
            let err = tracker.eof_check(0).unwrap_err();
            assert!(
                err.contains("final record must be a canonical FRAME_END or FRAME_REFUSED"),
                "{err}"
            );
        }
    }

    #[test]
    fn boundary_tracker_binds_perf_to_pending_and_open_hours() {
        let hour = raw_for(HOUR_A);

        let mut inside_open = BoundaryTracker::default();
        observe_zero_payload(
            &mut inside_open,
            boundary_header(1, hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
            0,
        );
        observe_zero_payload(
            &mut inside_open,
            boundary_header(
                2,
                hour + 999,
                18,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                COMPACT_DOMAIN_PERF_BASE_LEN,
                0,
            ),
            32,
        );
        let err = inside_open.eof_check(0).unwrap_err();
        assert!(
            err.contains("DOMAIN_PERF") && err.contains("still open"),
            "{err}"
        );

        let mut pending = BoundaryTracker::default();
        observe_zero_payload(
            &mut pending,
            boundary_header(
                1,
                hour,
                18,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                COMPACT_DOMAIN_PERF_BASE_LEN,
                0,
            ),
            0,
        );
        observe_zero_payload(
            &mut pending,
            boundary_header(2, hour + 1, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
            32,
        );
        let err = pending.eof_check(0).unwrap_err();
        assert!(err.contains("pending PERF hour"), "{err}");

        let mut after_close = BoundaryTracker::default();
        observe_zero_payload(
            &mut after_close,
            boundary_header(1, hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
            0,
        );
        observe_zero_payload(
            &mut after_close,
            boundary_header(
                2,
                hour,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                COMPACT_FRAME_END_PAYLOAD_LEN,
                0,
            ),
            32,
        );
        observe_zero_payload(
            &mut after_close,
            boundary_header(
                3,
                hour,
                250,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                COMPACT_DOMAIN_PERF_BASE_LEN,
                0,
            ),
            64,
        );
        let err = after_close.eof_check(0).unwrap_err();
        assert!(err.contains("non-monotonic DOMAIN_PERF"), "{err}");
    }

    #[test]
    fn boundary_tracker_preserves_perf_before_data_for_the_next_hour() {
        let first = raw_for(HOUR_A);
        let second = raw_for(HOUR_A + 1);
        let mut tracker = BoundaryTracker::default();
        for (sequence, hour, domain, record_type, payload_len) in [
            (
                1,
                first,
                18,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                COMPACT_DOMAIN_PERF_BASE_LEN,
            ),
            (2, first, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0),
            (
                3,
                first,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                COMPACT_FRAME_END_PAYLOAD_LEN,
            ),
            (
                4,
                second,
                18,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                COMPACT_DOMAIN_PERF_BASE_LEN,
            ),
            (5, second, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0),
            (
                6,
                second,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                COMPACT_FRAME_END_PAYLOAD_LEN,
            ),
        ] {
            let header = boundary_header(sequence, hour, domain, record_type, payload_len, 0);
            if record_type == COMPACT_RECORD_TYPE_FRAME_END {
                tracker.observe(&header, &frame_end_payload(1, 1, 0, 0), sequence * 32);
            } else {
                observe_zero_payload(&mut tracker, header, sequence * 32);
            }
        }
        tracker
            .eof_check(0)
            .expect("PERF may precede DATA when the next hour closes canonically");
    }

    fn valid_pdu1_payload() -> Vec<u8> {
        let mut payload = vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN];
        payload.extend_from_slice(COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC);
        payload.extend_from_slice(&COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes());
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&3u16.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&123u64.to_le_bytes());
        payload.extend_from_slice(b"cpu");
        payload
    }

    #[test]
    fn boundary_tracker_accepts_valid_announced_pdu1() {
        let hour = raw_for(HOUR_A);
        let payload = valid_pdu1_payload();
        let mut tracker = BoundaryTracker::default();
        tracker.observe(
            &boundary_header(
                1,
                hour,
                18,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                payload.len(),
                COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            ),
            &payload,
            0,
        );
        observe_zero_payload(
            &mut tracker,
            boundary_header(2, hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
            32,
        );
        tracker.observe(
            &boundary_header(
                3,
                hour,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                COMPACT_FRAME_END_PAYLOAD_LEN,
                0,
            ),
            &frame_end_payload(1, 1, 0, 0),
            64,
        );
        tracker
            .eof_check(0)
            .expect("a valid announced PDU1 trailer must pass the shared entrypoint");
    }

    #[test]
    fn boundary_tracker_binds_frame_end_counts_to_unique_terminal_outcomes() {
        let hour = raw_for(HOUR_A);

        let mut duplicate = BoundaryTracker::default();
        for sequence in 1..=2 {
            observe_zero_payload(
                &mut duplicate,
                boundary_header(
                    sequence,
                    hour,
                    18,
                    COMPACT_RECORD_TYPE_DOMAIN_PERF,
                    COMPACT_DOMAIN_PERF_BASE_LEN,
                    0,
                ),
                sequence * 32,
            );
        }
        let err = duplicate.eof_check(0).unwrap_err();
        assert!(
            err.contains("duplicate terminal outcome for domain 18"),
            "{err}"
        );

        let mut missing = BoundaryTracker::default();
        observe_zero_payload(
            &mut missing,
            boundary_header(1, hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
            0,
        );
        missing.observe(
            &boundary_header(
                2,
                hour,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                COMPACT_FRAME_END_PAYLOAD_LEN,
                0,
            ),
            &frame_end_payload(1, 1, 0, 0),
            32,
        );
        let err = missing.eof_check(0).unwrap_err();
        assert!(
            err.contains("observed 0 unique non-probe terminal outcomes"),
            "{err}"
        );

        let mut probe_excluded = BoundaryTracker::default();
        for (sequence, domain) in [(1, 250), (2, 18)] {
            observe_zero_payload(
                &mut probe_excluded,
                boundary_header(
                    sequence,
                    hour,
                    domain,
                    COMPACT_RECORD_TYPE_DOMAIN_PERF,
                    COMPACT_DOMAIN_PERF_BASE_LEN,
                    0,
                ),
                sequence * 32,
            );
        }
        observe_zero_payload(
            &mut probe_excluded,
            boundary_header(3, hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
            96,
        );
        probe_excluded.observe(
            &boundary_header(
                4,
                hour,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                COMPACT_FRAME_END_PAYLOAD_LEN,
                0,
            ),
            &frame_end_payload(1, 1, 0, 0),
            128,
        );
        probe_excluded
            .eof_check(0)
            .expect("reserved probe domain 250 must not inflate FRAME_END counts");

        let gap_payload = compact_contract::DomainGapRecordV1 {
            part: compact_contract::DomainGapPart::Both,
            reason: compact_contract::DomainGapReason::ClockUnreadable,
            target_hour_raw: hour,
            range_start_hour_raw: hour,
            range_end_hour_raw: hour,
            tick_before: 0,
            tick_after: 0,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        }
        .encode()
        .expect("canonical gap fixture");
        let mut gap_only = BoundaryTracker::default();
        gap_only.observe(
            &boundary_header(
                1,
                hour,
                18,
                COMPACT_RECORD_TYPE_DOMAIN_GAP,
                gap_payload.len(),
                0,
            ),
            &gap_payload,
            0,
        );
        gap_only.observe(
            &boundary_header(
                2,
                hour,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                COMPACT_FRAME_END_PAYLOAD_LEN,
                0,
            ),
            &frame_end_payload(1, 1, 0, 0),
            32,
        );
        gap_only
            .eof_check(0)
            .expect("one DOMAIN_GAP terminal outcome must seal a gap-only frame");

        let mut gap_then_data = BoundaryTracker::default();
        gap_then_data.observe(
            &boundary_header(
                1,
                hour,
                18,
                COMPACT_RECORD_TYPE_DOMAIN_GAP,
                gap_payload.len(),
                0,
            ),
            &gap_payload,
            0,
        );
        observe_zero_payload(
            &mut gap_then_data,
            boundary_header(2, hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
            32,
        );
        let error = gap_then_data.eof_check(0).unwrap_err();
        assert!(error.contains("coexists with DOMAIN_GAP"), "{error}");

        let mut data_then_gap = BoundaryTracker::default();
        observe_zero_payload(
            &mut data_then_gap,
            boundary_header(1, hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
            0,
        );
        data_then_gap.observe(
            &boundary_header(
                2,
                hour,
                18,
                COMPACT_RECORD_TYPE_DOMAIN_GAP,
                gap_payload.len(),
                0,
            ),
            &gap_payload,
            32,
        );
        let error = data_then_gap.eof_check(0).unwrap_err();
        assert!(error.contains("coexists with DOMAIN_DATA"), "{error}");
    }

    #[test]
    fn boundary_tracker_rejects_missing_announced_pdu1() {
        let hour = raw_for(HOUR_A);
        let payload = vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN];
        let mut tracker = BoundaryTracker::default();
        tracker.observe(
            &boundary_header(
                1,
                hour,
                18,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                payload.len(),
                COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            ),
            &payload,
            0,
        );
        let err = tracker.eof_check(0).unwrap_err();
        assert!(
            err.contains("diagnostic trailer header is truncated"),
            "{err}"
        );
    }

    #[test]
    fn boundary_tracker_rejects_unannounced_pdu1() {
        let hour = raw_for(HOUR_A);
        let payload = valid_pdu1_payload();
        let mut tracker = BoundaryTracker::default();
        tracker.observe(
            &boundary_header(
                1,
                hour,
                18,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                payload.len(),
                0,
            ),
            &payload,
            0,
        );
        let err = tracker.eof_check(0).unwrap_err();
        assert!(err.contains("unannounced trailer byte"), "{err}");
    }

    #[test]
    fn boundary_tracker_rejects_malformed_announced_pdu1() {
        let hour = raw_for(HOUR_A);
        let mut payload = valid_pdu1_payload();
        payload[COMPACT_DOMAIN_PERF_BASE_LEN] = b'X';
        let mut tracker = BoundaryTracker::default();
        tracker.observe(
            &boundary_header(
                1,
                hour,
                18,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                payload.len(),
                COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            ),
            &payload,
            0,
        );
        let err = tracker.eof_check(0).unwrap_err();
        assert!(err.contains("diagnostic trailer magic is invalid"), "{err}");
    }

    #[test]
    fn boundary_tracker_delegates_flags_and_frame_contract() {
        let hour = raw_for(HOUR_A);
        let mut rejected_flags = BoundaryTracker::default();
        observe_zero_payload(
            &mut rejected_flags,
            boundary_header(1, hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 1 << 29),
            0,
        );
        let err = rejected_flags.eof_check(0).unwrap_err();
        assert!(err.contains("contain rejected bits"), "{err}");

        let mut wrong_frame = BoundaryTracker::default();
        observe_zero_payload(
            &mut wrong_frame,
            boundary_header(
                1,
                hour,
                0,
                COMPACT_RECORD_TYPE_FRAME_END,
                COMPACT_FRAME_END_PAYLOAD_LEN,
                0,
            ),
            0,
        );
        let err = wrong_frame.eof_check(0).unwrap_err();
        assert!(err.contains("FRAME_END domain 0 is not"), "{err}");
    }

    #[test]
    fn d12_truncated_country_row_returns_invalid_data_not_panic() {
        let mut payload = vec![0u8; 28];
        payload[8..12].copy_from_slice(&1u32.to_le_bytes());
        expect_malformed_domain_degrades(12, &payload);
    }

    #[test]
    fn d14_truncated_popularity_row_returns_invalid_data_not_panic() {
        let mut payload = vec![0u8; 40];
        payload[24..28].copy_from_slice(&1u32.to_le_bytes());
        expect_malformed_domain_degrades(14, &payload);
    }

    #[test]
    fn d15_truncated_advisor_detail_returns_invalid_data_not_panic() {
        let mut payload = vec![0u8; 24];
        payload[20..24].copy_from_slice(&1u32.to_le_bytes());
        expect_malformed_domain_degrades(15, &payload);
    }

    #[test]
    fn d12_truncated_completed_tag_read_u32_returns_invalid_data_not_panic() {
        let mut payload = vec![0u8; 31];
        payload[24..28].copy_from_slice(&1u32.to_le_bytes());
        expect_malformed_domain_degrades(12, &payload);
    }

    #[test]
    fn d14_truncated_popularity_value_returns_invalid_data_not_panic() {
        let mut payload = vec![0u8; 40];
        payload[24..28].copy_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        expect_malformed_domain_degrades(14, &payload);
    }

    #[test]
    fn d14_full_popularity_row_remains_boundary_valid() {
        let mut payload = vec![0u8; 40];
        payload[8..12].copy_from_slice(&1u32.to_le_bytes());
        payload[24..28].copy_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&[0u8; 24]);
        for _ in 0..4 {
            payload.extend_from_slice(&0u16.to_le_bytes());
        }
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.push(b'd');
        payload.extend_from_slice(&50_000i32.to_le_bytes());
        expect_domain_boundary_acceptance(14, &payload);
    }

    #[test]
    fn d12_zero_row_payload_cannot_swallow_complete_record() {
        let mut payload = vec![0u8; 28];
        append_complete_domain_record(&mut payload, raw_for(HOUR_A));
        expect_domain_boundary_degrades(12, &payload);
    }

    #[test]
    fn d14_through_d17_trailing_record_residue_is_rejected() {
        for (domain, header_len) in [(14u16, 40usize), (15, 24), (16, 24), (17, 24)] {
            let mut payload = vec![0u8; header_len];
            append_complete_domain_record(&mut payload, raw_for(HOUR_A));
            expect_domain_boundary_degrades(domain, &payload);
        }
    }

    #[test]
    fn d51_delta_and_failure_status_cannot_skip_boundary_validation() {
        for status in [COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_SOURCE_UNAVAILABLE] {
            let mut payload = vec![0u8; 64];
            payload[20..24].copy_from_slice(&status.to_le_bytes());
            payload[48..52].copy_from_slice(&100_000u32.to_le_bytes());
            payload[52..56].copy_from_slice(&2u32.to_le_bytes());
            append_complete_domain_record(&mut payload, raw_for(HOUR_A));
            expect_domain_boundary_degrades(51, &payload);
        }
    }

    #[test]
    fn d69_failure_status_at_byte_12_cannot_skip_boundary_validation() {
        let mut payload = compact_contract::tech_sharing_groups::encode_payload_with_bonuses(
            &[],
            1,
            COMPACT_STATUS_SOURCE_UNAVAILABLE,
        )
        .unwrap();
        append_complete_domain_record(&mut payload, raw_for(HOUR_A));
        expect_domain_boundary_degrades(69, &payload);

        // Refutation attempt: append 192 bytes, exactly divisible by the TSG2
        // 12-byte pair width. Zero group windows still cannot claim the rows.
        let mut aligned_payload =
            compact_contract::tech_sharing_groups::encode_payload_with_bonuses(
                &[],
                1,
                COMPACT_STATUS_SOURCE_UNAVAILABLE,
            )
            .unwrap();
        for sequence in 99..102 {
            append_complete_domain_record(&mut aligned_payload, raw_for(HOUR_A) + sequence);
        }
        expect_domain_boundary_degrades(69, &aligned_payload);

        // The legacy 16-byte zero-row payload is ambiguous without a schema
        // discriminator and must never be admitted as D69 production DATA.
        let mut legacy = vec![0u8; 16];
        legacy[12..16].copy_from_slice(&COMPACT_STATUS_SOURCE_UNAVAILABLE.to_le_bytes());
        expect_domain_boundary_degrades(69, &legacy);
    }

    #[test]
    fn d51_and_d69_production_failure_payloads_pass_stream_boundary() {
        use scripted_state_flags_variables_live_lane::scripted_state_domain51_gate::compact as d51c;

        let d51 = d51c::encode_payload(
            &d51c::ScriptedStateHeader {
                elapsed_wall_ms: 1,
                status_flags: d51c::STATUS_SOURCE_UNAVAILABLE,
                changed_rows: 0,
                observed_scopes: 0,
                measured_scopes: 0,
                rejected_scopes: 0,
            },
            &[],
        );
        expect_domain_boundary_acceptance(51, &d51);

        let d69 = compact_contract::tech_sharing_groups::encode_payload_with_bonuses(
            &[],
            1,
            COMPACT_STATUS_SOURCE_UNAVAILABLE,
        )
        .unwrap();
        expect_domain_boundary_acceptance(69, &d69);
    }

    #[test]
    fn d69_v2_signed_bonus_payload_passes_stream_boundary() {
        use compact_contract::tech_sharing_groups::{
            encode_payload_with_bonuses, TechSharingGroupBonusRow, TechSharingMemberBonus,
        };

        let payload = encode_payload_with_bonuses(
            &[TechSharingGroupBonusRow {
                id_token: 9,
                members: vec![
                    TechSharingMemberBonus {
                        member_tag_db_id: 1,
                        dynamic_bonus_raw_fixed5: -5_000,
                    },
                    TechSharingMemberBonus {
                        member_tag_db_id: 368,
                        dynamic_bonus_raw_fixed5: 10_000,
                    },
                ],
            }],
            1,
            0,
        )
        .unwrap();
        expect_domain_boundary_acceptance(69, &payload);
    }

    #[test]
    fn d53_plain_delta_production_payload_passes_stream_boundary() {
        use diplomacy_intel_operatives_live::compact as d53c;

        let payload = d53c::encode_payload(&d53c::DiplomacyIntelSnapshot {
            status_flags: COMPACT_STATUS_DELTA_DATA,
            opinions: vec![d53c::OpinionRow {
                src_tag_db_id: 1,
                target_tag_db_id: 2,
                opinion: 5,
            }],
            ..Default::default()
        });
        expect_domain_boundary_acceptance(53, &payload);
    }

    #[test]
    fn d53_plain_partial_production_payload_passes_stream_boundary() {
        use diplomacy_intel_operatives_live::compact as d53c;

        let payload = d53c::encode_payload(&d53c::DiplomacyIntelSnapshot {
            status_flags: COMPACT_STATUS_PARTIAL_DATA,
            ..Default::default()
        });
        expect_domain_boundary_acceptance(53, &payload);
    }

    #[test]
    fn d53_facs_v1_absent_production_payload_passes_stream_boundary() {
        use diplomacy_intel_operatives_live::compact as d53c;

        let payload = d53c::encode_payload(&d53c::DiplomacyIntelSnapshot {
            opinions: vec![d53c::OpinionRow {
                src_tag_db_id: 1,
                target_tag_db_id: 2,
                opinion: 5,
            }],
            factions: Vec::new(),
            ..Default::default()
        });
        assert!(
            !payload.windows(4).any(|window| window == b"FACS"),
            "the production encoder must omit FACS v1 for an absent faction set"
        );
        expect_domain_boundary_acceptance(53, &payload);
    }

    #[test]
    fn d53_delta_partial_passes_boundary_validation_and_normal_fold() {
        use diplomacy_intel_operatives_live::compact as d53c;

        // This is the real recorder's empty-first-frame sequence: a tolerated
        // partial full frame seeds an empty baseline, then the next partial read
        // emits a DELTA|PARTIAL diff against that baseline.
        let baseline = d53c::encode_payload(&d53c::DiplomacyIntelSnapshot {
            status_flags: COMPACT_STATUS_PARTIAL_DATA,
            ..Default::default()
        });
        let delta = d53c::encode_payload(&d53c::DiplomacyIntelSnapshot {
            status_flags: COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_PARTIAL_DATA,
            opinions: vec![d53c::OpinionRow {
                src_tag_db_id: 1,
                target_tag_db_id: 2,
                opinion: 5,
            }],
            ..Default::default()
        });

        validate_domain_data_payload(53, &delta, 0)
            .expect("a structurally valid d53 DELTA|PARTIAL must pass boundary validation");

        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            raw_for(HOUR_A),
            53,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &baseline,
        );
        push_record(
            &mut stream,
            2,
            raw_for(HOUR_A),
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; 32],
        );
        push_record(
            &mut stream,
            3,
            raw_for(HOUR_B),
            53,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &delta,
        );
        push_record(
            &mut stream,
            4,
            raw_for(HOUR_B),
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; 32],
        );

        let mut fold = WorldFold::new("gid");
        let stats = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
            .expect("the normal pipeline must fold a valid d53 DELTA|PARTIAL after its baseline");
        assert_eq!(stats.hours_emitted, 2);
    }

    fn three_hour_stream() -> Vec<u8> {
        let mut s = Vec::new();
        let mut seq = 1u64;
        for (h, rows) in [
            (HOUR_A, vec![(47u32, "GER")]),
            (HOUR_B, vec![(47, "GER")]),
            (HOUR_C, vec![(47, "GER"), (48, "FRA")]),
        ] {
            push_record(
                &mut s,
                seq,
                raw_for(h),
                18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &country_identity_payload(&rows),
            );
            seq += 1;
            push_record(
                &mut s,
                seq,
                raw_for(h),
                0,
                COMPACT_RECORD_TYPE_FRAME_END,
                &[0u8; 32], // production FRAME_END payload is exactly 32 bytes
            );
            seq += 1;
        }
        s
    }

    /// Reproduce the audited boundary construction exactly: after the forged
    /// DATA length consumes a complete adjacent record, the only visible H4CR
    /// headers remain PERF/DATA/FRAME_END with consecutive sequence numbers.
    /// Coverage counts and the stream hash are intentionally derived from the
    /// already-corrupt bytes, so neither can reveal the swallowed record.
    #[test]
    fn data_payload_cannot_swallow_complete_adjacent_record() {
        let hour = raw_for(HOUR_A);
        let mut swallowed = Vec::new();
        push_record(
            &mut swallowed,
            3,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(48, "FRA")]),
        );

        let mut forged_payload = country_identity_payload(&[(47, "GER")]);
        forged_payload.extend_from_slice(&swallowed);

        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &[0u8; 40],
        );
        push_record(
            &mut stream,
            2,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &forged_payload,
        );
        push_record(
            &mut stream,
            3,
            hour,
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(1, 1, 0, 0),
        );

        let regenerated_manifest_hash = Sha256::digest(&stream);
        assert_eq!(Sha256::digest(&stream), regenerated_manifest_hash);

        let mut fold = WorldFold::new("gid");
        let stats = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
            .expect("swallow degrades, not fails");
        assert_eq!(stats.malformed_payload_records, 1);
        assert_eq!(
            stats.data_records, 1,
            "the swallowed record must not be independently framed/applied"
        );
    }

    #[test]
    fn well_formed_domain_data_payload_passes_boundary_validation() {
        let hour = raw_for(HOUR_A);
        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &[0u8; 40],
        );
        push_record(
            &mut stream,
            2,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER")]),
        );
        push_record(
            &mut stream,
            3,
            hour,
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(1, 1, 0, 0),
        );

        let mut fold = WorldFold::new("gid");
        let stats = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
            .expect("well-formed DOMAIN_DATA must remain accepted");
        assert_eq!(stats.stream_bytes, stream.len() as u64);
        assert_eq!(stats.bytes_folded, stream.len() as u64);
        assert_eq!(stats.records_total, 3);
        assert_eq!(stats.perf_records, 1);
        assert_eq!(stats.data_records, 1);
        assert_eq!(stats.malformed_payload_records, 0);
        assert_eq!(stats.frame_end_records, 1);
        assert_eq!(stats.hours_emitted, 1);
        assert_eq!(stats.truncated_tail_bytes, 0);
        assert!(stats.stop_reason.is_none());
    }

    fn one_data_hour(payload: &[u8]) -> Vec<u8> {
        let hour = raw_for(HOUR_A);
        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            payload,
        );
        push_record(
            &mut stream,
            2,
            hour,
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; 32],
        );
        stream
    }

    #[test]
    fn domain_data_payload_residue_degrades() {
        let mut payload = country_identity_payload(&[(47, "GER")]);
        payload.push(0xA5);
        let stream = one_data_hour(&payload);
        let mut fold = WorldFold::new("gid");
        let stats = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
            .expect("unconsumed DATA residue must degrade its domain");
        assert_eq!(stats.malformed_payload_records, 1);
    }

    #[test]
    fn domain_data_payload_declared_row_overrun_degrades() {
        let mut payload = country_identity_payload(&[(47, "GER")]);
        payload[8..12].copy_from_slice(&2u32.to_le_bytes());
        let stream = one_data_hour(&payload);
        let mut fold = WorldFold::new("gid");
        let stats = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
            .expect("DATA internal row overrun must degrade its domain");
        assert_eq!(stats.malformed_payload_records, 1);
    }

    /// The streaming fold spills one snapshot file per hour and produces bytes
    /// byte-identical to the in-memory batch fold (`ingest_stream` + `finish`).
    /// This is the unit-level G1 equivalence: streaming path == batch path.
    #[test]
    fn streaming_fold_matches_batch_fold() {
        let dir = TmpDir::new("equiv");
        let stream = three_hour_stream();
        let stream_path = dir.0.join("s.bin");
        std::fs::write(&stream_path, &stream).unwrap();

        let (files, stats) =
            fold_compact_stream_file(&stream_path, "gid", &dir.0.join("snaps")).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(stats.hours_emitted, 3);
        assert_eq!(stats.truncated_tail_bytes, 0);
        assert!(stats.stop_reason.is_none());

        // Batch fold the same bytes in memory.
        let mut batch = WorldFold::new("gid");
        batch.ingest_stream(&stream);
        let batch_out = batch.finish();
        assert_eq!(batch_out.len(), files.len());
        for (f, (hour, _date, bytes)) in files.iter().zip(batch_out.iter()) {
            assert_eq!(f.hour_index, *hour);
            let spilled = std::fs::read(&f.path).unwrap();
            assert_eq!(
                to_hex(&Sha256::digest(&spilled)),
                to_hex(&Sha256::digest(bytes)),
                "streaming hour {} snapshot != batch",
                hour
            );
        }
    }

    /// Net-gate finding 1: a sealed (compressed) stream whose decoded bytes
    /// end mid-record must HARD-ERROR, never return Ok-with-stats. Raw stays
    /// tolerant (separately asserted by the incomplete-tail test).
    #[test]
    fn compressed_stream_ending_mid_record_hard_errors() {
        let mut stream = three_hour_stream();
        // Append 16 bytes of a would-be next header (valid magic, then cut).
        stream.extend_from_slice(COMPACT_RECORD_MAGIC);
        stream.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        stream.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        stream.extend_from_slice(&7u64.to_le_bytes());
        let mut fold = WorldFold::new("gid");
        let err =
            drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, true).unwrap_err();
        assert!(err.to_string().contains("ends mid-record"), "{err}");

        // Same bytes, RAW source: tolerated with the residue accounted.
        let mut fold = WorldFold::new("gid");
        let stats =
            drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, false).unwrap();
        assert_eq!(stats.truncated_tail_bytes, 16);
        assert!(!stats.ends_at_frame_end);
    }

    /// Net-gate finding 2: a sealed stream whose last record is NOT FRAME_END
    /// ends inside an open game hour (a crash tail) and must HARD-ERROR; the
    /// deferred repack independently refuses to seal such a source.
    #[test]
    fn compressed_stream_ending_in_open_hour_hard_errors() {
        let mut stream = three_hour_stream();
        push_record(
            &mut stream,
            7,
            raw_for(HOUR_C + 1),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER")]),
        );
        let mut fold = WorldFold::new("gid");
        let err =
            drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, true).unwrap_err();
        assert!(err.to_string().contains("left open"), "{err}");

        // RAW: tolerated; the boundary fact is exposed for the repack guard.
        let mut fold = WorldFold::new("gid");
        let stats =
            drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, false).unwrap();
        assert!(!stats.ends_at_frame_end);
        // A clean stream reports the closed boundary.
        let mut fold = WorldFold::new("gid");
        let clean = drive_fold_compressed(
            std::io::Cursor::new(three_hour_stream()),
            0,
            &mut fold,
            false,
        )
        .unwrap();
        assert!(clean.ends_at_frame_end);
    }

    /// Re-gate round 2, finding 1: closure is HOUR-AWARE — `DATA(N+1),
    /// FRAME_END(N), EOF` has a trailing FRAME_END but hour N+1 was never
    /// closed; it must hard-error on sealed sources and report an unclosed
    /// boundary on raw ones.
    #[test]
    fn trailing_frame_end_for_the_wrong_hour_does_not_close_the_stream() {
        let mut stream = three_hour_stream();
        push_record(
            &mut stream,
            7,
            raw_for(HOUR_C + 1),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER")]),
        );
        // FRAME_END for the WRONG (already closed) hour.
        push_record(
            &mut stream,
            8,
            raw_for(HOUR_C),
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; 32],
        );
        let mut fold = WorldFold::new("gid");
        let err =
            drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, true).unwrap_err();
        assert!(err.to_string().contains("mismatched FRAME_END"), "{err}");

        let mut fold = WorldFold::new("gid");
        let stats =
            drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, false).unwrap();
        assert!(!stats.ends_at_frame_end);

        // The seal-time walker enforces the same law for skip-fold packs.
        let dir = TmpDir::new("boundary");
        let p = dir.0.join("torn.bin");
        std::fs::write(&p, &stream).unwrap();
        let err = verify_stream_logical_boundary(&p).unwrap_err();
        assert!(err.to_string().contains("mismatched FRAME_END"), "{err}");
        let clean = dir.0.join("clean.bin");
        std::fs::write(&clean, three_hour_stream()).unwrap();
        verify_stream_logical_boundary(&clean).unwrap();
    }

    #[test]
    fn seal_time_boundary_rejects_malformed_domain_data() {
        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            raw_for(HOUR_A),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[],
        );
        push_record(
            &mut stream,
            2,
            raw_for(HOUR_A),
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        );

        let dir = TmpDir::new("malformed_data_boundary");
        let path = dir.0.join("malformed.bin");
        std::fs::write(&path, stream).unwrap();
        let err = verify_stream_logical_boundary(&path).unwrap_err();
        assert!(
            err.to_string()
                .contains("domain 18 DATA payload failed exact validation"),
            "{err}"
        );
    }

    fn expect_frame_end_seal_rejection(tag: &str, frame_payload: &[u8], expected: &str) {
        let hour = raw_for(HOUR_A);
        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER")]),
        );
        push_record(
            &mut stream,
            2,
            hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            frame_payload,
        );
        let dir = TmpDir::new(tag);
        let path = dir.0.join("invalid-frame-end.bin");
        std::fs::write(&path, stream).unwrap();

        let err = verify_stream_logical_boundary(&path).unwrap_err();
        assert!(
            err.to_string().contains(expected),
            "case {tag}: expected {expected:?}, got {err}"
        );
    }

    #[test]
    fn seal_time_boundary_rejects_nonzero_frame_end_reserved_word() {
        expect_frame_end_seal_rejection(
            "frame_end_reserved",
            &frame_end_payload(1, 1, 0, 7),
            "reserved word is nonzero",
        );
    }

    #[test]
    fn seal_time_boundary_rejects_frame_end_domain_count_mismatch() {
        expect_frame_end_seal_rejection(
            "frame_end_count_mismatch",
            &frame_end_payload(1, 2, 0, 0),
            "emitted domain count 1 does not match scheduled domain count 2",
        );
    }

    #[test]
    fn seal_time_boundary_rejects_unknown_frame_end_status_bits() {
        expect_frame_end_seal_rejection(
            "frame_end_unknown_status",
            &frame_end_payload(1, 1, 1 << 31, 0),
            "contain unknown bits 0x80000000",
        );
    }

    #[test]
    fn seal_time_boundary_rejects_fatal_frame_end_status() {
        expect_frame_end_seal_rejection(
            "frame_end_fatal_status",
            &frame_end_payload(1, 1, COMPACT_STATUS_SOURCE_UNAVAILABLE, 0),
            "contain fatal bits",
        );
    }

    fn assert_compressed_protocol_hard_error(err: &io::Error, expected_reason: &str) {
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let message = err.to_string();
        assert!(
            message.contains(
                "H4CR framing or stream validation stopped before physical EOF on a compressed stream"
            ),
            "missing compressed-stream hard-error classification: {message}"
        );
        assert!(
            message.contains(expected_reason),
            "missing framing/validation reason {expected_reason:?}: {message}"
        );
        assert!(
            message.contains("storage checksum valid"),
            "missing storage-integrity classification: {message}"
        );
        assert!(
            message.contains("declared decoded-stream protocol is invalid"),
            "missing decoded-protocol classification: {message}"
        );
    }

    /// Net-gate finding 3: a forged/corrupt payload_len beyond the record
    /// bound is a framing error — hard on compressed, loud stop_reason on raw
    /// — instead of a multi-hundred-MB carry that can swallow later records.
    #[test]
    fn oversized_payload_len_is_a_framing_stop() {
        let mut stream = three_hour_stream();
        stream.extend_from_slice(COMPACT_RECORD_MAGIC);
        stream.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        stream.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        stream.extend_from_slice(&7u64.to_le_bytes());
        stream.extend_from_slice(&raw_for(HOUR_C).to_le_bytes());
        stream.extend_from_slice(&18u16.to_le_bytes());
        stream.extend_from_slice(&COMPACT_RECORD_TYPE_DOMAIN_DATA.to_le_bytes());
        stream.extend_from_slice(&0x2000_0000u32.to_le_bytes()); // 512 MiB > bound
        stream.extend_from_slice(&0u32.to_le_bytes());

        let mut fold = WorldFold::new("gid");
        let err =
            drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, true).unwrap_err();
        assert_compressed_protocol_hard_error(&err, "exceeds the record bound");

        let mut fold = WorldFold::new("gid");
        let stats =
            drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, false).unwrap();
        assert!(
            stats
                .stop_reason
                .as_deref()
                .is_some_and(|r| r.contains("exceeds the record bound")),
            "{:?}",
            stats.stop_reason
        );
        assert_eq!(stats.hours_emitted, 3, "records before the stop still fold");
    }

    /// Hour-boundary release: a file-backed fold NEVER retains a closed hour's
    /// snapshot in memory — `take_finished()` stays empty while every hour is
    /// accounted, so peak memory does not grow with the hour count.
    #[test]
    fn file_backed_fold_releases_each_hour() {
        let dir = TmpDir::new("release");
        let stream = three_hour_stream();
        let mut fold = WorldFold::new_file_backed("gid", &dir.0.join("snaps")).unwrap();
        let stats =
            drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold).unwrap();
        assert_eq!(stats.hours_emitted, 3);
        // Nothing accumulated in the in-memory sink: each hour spilled + released.
        assert!(
            fold.take_finished().is_empty(),
            "file-backed fold must not retain snapshots in memory"
        );
        assert_eq!(fold.emitted_hours_count(), 3);
        let files = fold.finish_file_backed().unwrap();
        assert_eq!(files.len(), 3);
        for f in &files {
            assert!(
                f.path.exists(),
                "hour {} snapshot spilled to disk",
                f.hour_index
            );
        }
    }

    /// A stream that ends mid-record yields only the complete hours and reports the
    /// truncated tail (no partial snapshot, no panic).
    #[test]
    fn truncated_tail_emits_complete_hours_only() {
        let dir = TmpDir::new("trunc");
        let mut stream = three_hour_stream();
        stream.truncate(stream.len() - 5); // chop hour C's frame_end
        let stream_path = dir.0.join("s.bin");
        std::fs::write(&stream_path, &stream).unwrap();
        let (files, stats) =
            fold_compact_stream_file(&stream_path, "gid", &dir.0.join("snaps")).unwrap();
        assert_eq!(files.len(), 2, "only complete hours emit");
        assert_eq!(files[0].hour_index, HOUR_A);
        assert_eq!(files[1].hour_index, HOUR_B);
        assert!(stats.truncated_tail_bytes > 0, "incomplete tail reported");
    }

    #[test]
    fn torn_zstd_frame_at_complete_record_boundary_fails_fold() {
        let dir = TmpDir::new("zstd_trunc");
        let stream = three_hour_stream();
        let raw_path = dir.0.join("raw.bin");
        let zstd_path = dir.0.join("compressed.bin");
        std::fs::write(&raw_path, &stream).unwrap();
        compact_stream::encode_file(
            &raw_path,
            &zstd_path,
            compact_stream::StreamCompression::Zstd,
        )
        .unwrap();

        let mut stored = std::fs::read(&zstd_path).unwrap();
        stored.pop().expect("checksummed zstd frame is non-empty");
        std::fs::write(&zstd_path, stored).unwrap();

        let err = match fold_compact_stream_file(&zstd_path, "gid", &dir.0.join("snaps")) {
            Ok(_) => panic!("stream_fold must reject a torn zstd frame"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert!(err.to_string().contains("incomplete zstd frame"));
    }

    // Review finding (fix round 2): corruption that mangles a record HEADER
    // makes framing stop gracefully BEFORE the zstd footer, bypassing the
    // content checksum. On a compressed source that must now hard-error; on a
    // raw source the tolerant stop-with-reason behavior is preserved exactly.
    #[test]
    fn header_corruption_in_compressed_stream_fails_fold_loudly() {
        let dir = TmpDir::new("zstd_hdr_flip");
        let clean = three_hour_stream();
        let mut corrupted = clean.clone();
        // First record starts at 0; walk one record forward with the public
        // framing primitive and clobber the SECOND record's magic byte, so
        // framing stops mid-stream inside checksummed-valid zstd bytes.
        let h = parse_compact_record_header(&clean).unwrap();
        let second = h.record_len();
        corrupted[second] ^= 0xFF;

        let raw_path = dir.0.join("raw.bin");
        let zstd_src = dir.0.join("src.bin");
        let zstd_path = dir.0.join("compressed.bin");
        std::fs::write(&raw_path, &corrupted).unwrap();
        std::fs::write(&zstd_src, &corrupted).unwrap();
        compact_stream::encode_file(
            &zstd_src,
            &zstd_path,
            compact_stream::StreamCompression::Zstd,
        )
        .unwrap();

        // Compressed: hard error, never a clean short fold.
        let err = match fold_compact_stream_file(&zstd_path, "gid", &dir.0.join("zs")) {
            Ok(_) => panic!("compressed header corruption must fail the fold"),
            Err(err) => err,
        };
        assert_compressed_protocol_hard_error(&err, "invalid record magic");

        // Raw: byte-identical tolerant behavior (stop reason preserved; the
        // corruption sits before hour A's FRAME_END, so no hour completes).
        let (files, stats) =
            fold_compact_stream_file(&raw_path, "gid", &dir.0.join("raws")).unwrap();
        assert!(
            files.is_empty(),
            "corruption before the first FRAME_END emits no hours"
        );
        assert!(stats.stop_reason.is_some(), "raw stop keeps its diagnostic");
    }

    /// Scripted reader discriminating the post-framing-stop drain: serves a
    /// prepared chunk, then either errors (checksum found during drain) or
    /// EOFs (intact storage ⇒ the framing stop is the hard error).
    struct ScriptedReader {
        prefix: Vec<u8>,
        served: bool,
        tail_error: Option<(io::ErrorKind, &'static str)>,
    }

    impl Read for ScriptedReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if !self.served {
                self.served = true;
                let n = self.prefix.len().min(buf.len());
                buf[..n].copy_from_slice(&self.prefix[..n]);
                assert_eq!(n, self.prefix.len(), "test prefix must fit one chunk");
                return Ok(n);
            }
            match self.tail_error {
                Some((kind, msg)) => Err(io::Error::new(kind, msg)),
                None => Ok(0),
            }
        }
    }

    fn one_record_then_garbage() -> Vec<u8> {
        let clean = three_hour_stream();
        let first_len = parse_compact_record_header(&clean).unwrap().record_len();
        let mut bytes = clean[..first_len].to_vec();
        bytes.extend_from_slice(b"XXXXGARBAGEHEADERXXXXGARBAGEHEADER");
        bytes
    }

    #[test]
    fn fold_drain_after_framing_stop_propagates_the_storage_root_error() {
        let reader = ScriptedReader {
            prefix: one_record_then_garbage(),
            served: false,
            tail_error: Some((
                io::ErrorKind::InvalidData,
                "simulated zstd checksum mismatch",
            )),
        };
        let mut fold = WorldFold::new("gid");
        let err = drive_fold_compressed(reader, 0, &mut fold, true)
            .expect_err("checksum error during drain must surface");
        assert!(
            err.to_string().contains("simulated zstd checksum mismatch"),
            "root error must propagate, got: {err}"
        );
    }

    #[test]
    fn fold_framing_stop_with_intact_storage_is_itself_the_hard_error() {
        let reader = ScriptedReader {
            prefix: one_record_then_garbage(),
            served: false,
            tail_error: None,
        };
        let mut fold = WorldFold::new("gid");
        let err = drive_fold_compressed(reader, 0, &mut fold, true)
            .expect_err("compressed framing stop must not end cleanly");
        assert_compressed_protocol_hard_error(&err, "invalid record magic");
    }

    // The PARALLEL fold call site must also thread compressed=true (a review
    // finding: only raw fixtures exercised the parallel path, so flipping its
    // flag to false went undetected).
    #[test]
    fn parallel_fold_rejects_compressed_header_corruption() {
        let dir = TmpDir::new("zstd_par_hdr");
        let clean = three_hour_stream();
        let mut corrupted = clean.clone();
        let first_len = parse_compact_record_header(&clean).unwrap().record_len();
        corrupted[first_len] ^= 0xFF;
        let src = dir.0.join("src.bin");
        let zstd_path = dir.0.join("compressed.bin");
        std::fs::write(&src, &corrupted).unwrap();
        compact_stream::encode_file(&src, &zstd_path, compact_stream::StreamCompression::Zstd)
            .unwrap();
        let err =
            match fold_compact_stream_file_parallel(&zstd_path, "gid", &dir.0.join("snaps"), 2) {
                Ok(_) => panic!("parallel fold must reject compressed header corruption"),
                Err(err) => err,
            };
        assert_compressed_protocol_hard_error(&err, "invalid record magic");
    }

    #[test]
    fn discard_snapshot_reports_match_keep_mode_and_remove_files() {
        let dir = TmpDir::new("discard");
        let stream = three_hour_stream();

        let keep_path = dir.0.join("keep.bin");
        std::fs::write(&keep_path, &stream).unwrap();
        let (keep_files, keep_stats) =
            fold_compact_stream_file(&keep_path, "gid", &dir.0.join("keep_snaps")).unwrap();
        assert_eq!(keep_stats.hours_emitted, 3);
        let keep_rows = hash_folded_snapshot_files(&keep_files, false).unwrap();
        let keep_chain_hash = snapshot_chain_hash(&keep_rows);
        assert_eq!(keep_rows.len(), 3);
        for f in &keep_files {
            assert!(
                f.path.exists(),
                "keep mode should retain {}",
                f.path.display()
            );
        }

        let discard_path = dir.0.join("discard.bin");
        std::fs::write(&discard_path, &stream).unwrap();
        let (discard_files, discard_stats) =
            fold_compact_stream_file(&discard_path, "gid", &dir.0.join("discard_snaps")).unwrap();
        assert_eq!(discard_stats.hours_emitted, 3);
        let discard_rows = hash_folded_snapshot_files(&discard_files, true).unwrap();
        let discard_chain_hash = snapshot_chain_hash(&discard_rows);

        assert_eq!(discard_rows, keep_rows);
        assert_eq!(discard_chain_hash, keep_chain_hash);
        for f in &discard_files {
            assert!(
                !f.path.exists(),
                "discard mode should remove {}",
                f.path.display()
            );
        }
        assert!(
            std::fs::read_dir(dir.0.join("discard_snaps"))
                .unwrap()
                .next()
                .is_none(),
            "discard snapshot directory should be empty"
        );
    }

    /// The emit worker's streamed SHA-256 equals a fresh hash of the exact bytes
    /// it wrote (tee-writer correctness), for every spilled hour.
    #[test]
    fn stored_snapshot_sha_matches_file_bytes() {
        let dir = TmpDir::new("teesha");
        let stream = three_hour_stream();
        let stream_path = dir.0.join("s.bin");
        std::fs::write(&stream_path, &stream).unwrap();
        let (files, _) =
            fold_compact_stream_file(&stream_path, "gid", &dir.0.join("snaps")).unwrap();
        assert_eq!(files.len(), 3);
        for f in &files {
            let bytes = std::fs::read(&f.path).unwrap();
            assert_eq!(bytes.len(), f.byte_len);
            assert_eq!(
                f.sha256.as_deref(),
                Some(to_hex(&Sha256::digest(&bytes)).as_str()),
                "stored sha for hour {} must match spilled bytes",
                f.hour_index
            );
        }
    }

    /// G1 at the unit level for the parallel fold: for several worker counts
    /// (including more workers than hours), the merged strided output is
    /// byte-identical to the sequential fold — same hour set, same per-hour
    /// hashes, same chain hash, same stats accounting.
    #[test]
    fn parallel_fold_matches_sequential_for_all_worker_counts() {
        let dir = TmpDir::new("par");
        let stream = three_hour_stream();
        let stream_path = dir.0.join("s.bin");
        std::fs::write(&stream_path, &stream).unwrap();

        let (seq_files, seq_stats) =
            fold_compact_stream_file(&stream_path, "gid", &dir.0.join("seq")).unwrap();
        let seq_rows = hash_folded_snapshot_files(&seq_files, false).unwrap();
        let seq_chain = snapshot_chain_hash(&seq_rows);
        assert_eq!(seq_stats.hours_emitted, 3);

        for workers in [2usize, 3, 5] {
            let out = dir.0.join(format!("par{workers}"));
            let (par_files, par_stats) =
                fold_compact_stream_file_parallel(&stream_path, "gid", &out, workers).unwrap();
            assert_eq!(
                par_stats.hours_emitted, seq_stats.hours_emitted,
                "workers={workers}"
            );
            assert_eq!(par_stats.records_total, seq_stats.records_total);
            assert_eq!(par_stats.bytes_folded, seq_stats.bytes_folded);
            let par_rows = hash_folded_snapshot_files(&par_files, false).unwrap();
            assert_eq!(par_rows, seq_rows, "workers={workers} rows diverge");
            assert_eq!(
                snapshot_chain_hash(&par_rows),
                seq_chain,
                "workers={workers} chain diverges"
            );
            // Ordering contract: merged descriptors ascend by hour_index exactly
            // like the sequential emit order.
            let hours: Vec<i64> = par_files.iter().map(|f| f.hour_index).collect();
            assert_eq!(hours, vec![HOUR_A, HOUR_B, HOUR_C]);
        }
    }
}
