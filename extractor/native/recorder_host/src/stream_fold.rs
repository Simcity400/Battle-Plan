//! spilled to disk the moment its `FRAME_END` closes and then released â€” so peak

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use compact_contract::campaign_identity::ValidatedCid1;
use compact_contract::stream_validator::{
    AcceptedRecordKind, StatefulStreamValidator, StreamIdentityMode,
};
use compact_contract::{
    parse_compact_record_header, validate_compact_record_semantics, KnownRecordType,
    RecordFramingError, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC,
    COMPACT_RECORD_TYPE_CAPTURE_TERMINATED, COMPACT_RECORD_TYPE_CLOCK_UNREADABLE_INTERVAL,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_GAP,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_DEGRADED,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_TYPE_FRAME_REFUSED,
    COMPACT_RECORD_TYPE_FREEZE_CENSUS, COMPACT_RECORD_TYPE_UNTRUSTED_CLOCK_RECOVERY,
    COMPACT_STATUS_FAILURE_MASK,
};
use snapshot::sha256::{to_hex, Sha256};

use crate::domain_data_authority::{summarize_domain_data_payload, DomainDataContext};
use crate::world_fold::{FoldedSnapshotFile, WorldFold};

const CHUNK: usize = 8 * 1024 * 1024;

pub const MAX_RESYNC_WINDOW_BYTES: usize = 4 * 1024 * 1024;

pub const MAX_RESYNC_CLAIMED_HOURS: usize = 512;

const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;

fn validate_frame_end_seal_fields(payload: &[u8]) -> Result<(u32, u32), String> {
    if payload.len() != compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN {
        return Err(format!(
            "FRAME_END payload length {} is not {}",
            payload.len(),
            compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN
        ));
    }

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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ExcludedDomainDataRecord {
    pub stream_offset: u64,
    pub game_hour_raw: i32,
    pub domain_id: u16,
    pub validation_reason: String,
}

pub fn excluded_domain_data_json(records: &[ExcludedDomainDataRecord]) -> serde_json::Value {
    let mut per_domain = BTreeMap::<u16, u64>::new();
    for record in records {
        *per_domain.entry(record.domain_id).or_default() += 1;
    }
    serde_json::json!({
        "complete": records.is_empty(),
        "total_records": records.len(),
        "per_domain": per_domain
            .into_iter()
            .map(|(domain_id, count)| (domain_id.to_string(), serde_json::json!(count)))
            .collect::<serde_json::Map<String, serde_json::Value>>(),
        "records": records,
    })
}

pub fn validate_excluded_domain_data_accounting(
    excluded_count: u64,
    records: &[ExcludedDomainDataRecord],
) -> Result<(), String> {
    if excluded_count != records.len() as u64 {
        return Err(format!(
            "incomplete DOMAIN_DATA exclusion accounting: excluded_count={excluded_count} ledger_records={}",
            records.len(),
        ));
    }
    Ok(())
}

#[derive(Debug, Default, Clone)]
pub struct FoldStreamStats {
    pub stream_bytes: u64,
    pub bytes_folded: u64,
    pub truncated_tail_bytes: u64,
    pub records_total: u64,
    pub data_records: u64,
    pub malformed_payload_records: u64,
    pub malformed_payload_domains: BTreeMap<u16, MalformedPayloadDomainStats>,
    pub excluded_domain_data_records: Vec<ExcludedDomainDataRecord>,
    pub frame_end_records: u64,
    pub perf_records: u64,
    pub freeze_census_records: u64,
    pub other_records: u64,
    pub hours_emitted: u64,
    pub stop_reason: Option<String>,
    pub ends_at_frame_end: bool,
    pub decoded_sha256: String,
    pub unframeable_bytes: u64,
    pub hour_coverage: crate::hour_coverage::HourCoverageMap,
}

#[derive(Debug, Default, Clone)]
pub struct MalformedPayloadDomainStats {
    pub records: u64,
    pub first_reason: String,
    pub first_stream_offset: u64,
    pub first_game_hour_raw: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedCampaignIdentity {
    pub campaign_id: String,
    pub game_unique_id: String,
    pub game_unique_seed: i32,
    pub source_build_profile: String,
    pub observed_game_hour_raw: i32,
    pub cid1_payload_sha256: String,
}

#[derive(Debug, Clone)]
pub struct IdentityAwareFoldStats {
    pub stats: FoldStreamStats,
    pub campaign_identity: Option<FoldedCampaignIdentity>,
    validated_cid1: Option<ValidatedCid1>,
}

impl IdentityAwareFoldStats {
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

pub fn fold_compact_stream_file_with_identity_mode(
    stream_path: &Path,
    game_id: &str,
    snapshot_dir: &Path,
    identity_mode: StreamIdentityMode,
) -> io::Result<(Vec<FoldedSnapshotFile>, IdentityAwareFoldStats)> {
    fold_compact_stream_file_with_identity_mode_and_content_manifest(
        stream_path,
        game_id,
        snapshot_dir,
        identity_mode,
        None,
        None,
    )
}

pub fn fold_compact_stream_file_with_identity_mode_and_content_manifest(
    stream_path: &Path,
    game_id: &str,
    snapshot_dir: &Path,
    identity_mode: StreamIdentityMode,
    effective_content_manifest_sha256: Option<&str>,
    effective_vfs_manifest_sha256: Option<&str>,
) -> io::Result<(Vec<FoldedSnapshotFile>, IdentityAwareFoldStats)> {
    let reader = compact_stream::open(stream_path)?;
    let compressed = reader.compression() == compact_stream::StreamCompression::Zstd;
    let total_len = if compressed { 0 } else { reader.encoded_len() };
    let mut fold = WorldFold::new_file_backed(game_id, snapshot_dir)?;
    if let Some(digest) = effective_content_manifest_sha256 {
        fold.set_effective_content_manifest_sha256(digest)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    }
    if let Some(digest) = effective_vfs_manifest_sha256 {
        fold.set_effective_vfs_manifest_sha256(digest);
    }
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

#[derive(Debug, Default)]
pub struct BoundaryTracker {
    open_hour: Option<i32>,
    pending_perf_hour: Option<i32>,
    last_closed_hour: Option<i32>,
    last_record_kind: Option<u16>,
    release_freeze_census_seen: bool,
    content_identity_census_seen: bool,
    state_topology_census_seen: bool,
    terminal_outcome_domains: BTreeSet<u16>,
    gap_domains: BTreeSet<u16>,
    data_domains: BTreeSet<u16>,
    prev_sequence: Option<u64>,
    termination: Option<compact_contract::capture_termination::CaptureTerminationRecordV1>,
    unobserved_intervals: Vec<(i32, i32)>,
    disclaimed_spans: Vec<(i32, i32)>,
    violation: Option<String>,
    coverage: crate::hour_coverage::HourCoverageMap,
    hour_span: Option<OpenHourSpan>,
    resync: Option<ResyncState>,
    records_lost_at_injury: bool,
    framing_damage_seen: bool,
    lost_between_frames: (u64, String),
    abandoned_hour: (u64, i32, String),
}

struct OpenHourSpan {
    game_hour_raw: i32,
    first_byte: u64,
    len: u64,
    hasher: Sha256,
}

impl std::fmt::Debug for OpenHourSpan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenHourSpan")
            .field("game_hour_raw", &self.game_hour_raw)
            .field("first_byte", &self.first_byte)
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

impl OpenHourSpan {
    fn absorb(&mut self, header: &compact_contract::CompactRecordHeader, payload: &[u8]) {
        self.len = self
            .len
            .saturating_add(header.record_len().try_into().unwrap_or(u64::MAX));
        self.hasher.update(&header.sequence.to_le_bytes());
        self.hasher.update(&header.game_hour_raw.to_le_bytes());
        self.hasher.update(&header.domain_id.to_le_bytes());
        self.hasher.update(&header.record_type.to_le_bytes());
        self.hasher.update(&header.record_flags.to_le_bytes());
        self.hasher.update(&(payload.len() as u64).to_le_bytes());
        self.hasher.update(payload);
    }

    fn basis(&self) -> String {
        to_hex(&self.hasher.clone().finalize())
    }
}

#[derive(Debug)]
struct ResyncState {
    reason: String,
    hours: BTreeSet<i32>,
    first_byte: u64,
    len: u64,
    records_absorbed: u64,
    abandoned_signalled: bool,
    hours_overflowed: bool,
    lost_records: bool,
}

impl BoundaryTracker {
    fn unobserved_interval_covering(&self, hour: i32) -> Option<(i32, i32)> {
        self.unobserved_intervals
            .iter()
            .find(|(start, end)| *start <= hour && hour <= *end)
            .copied()
    }

    fn intersecting_unobserved_interval(&self, start: i32, end: i32) -> Option<(i32, i32)> {
        self.unobserved_intervals
            .iter()
            .find(|(other_start, other_end)| start <= *other_end && *other_start <= end)
            .copied()
    }

    fn intersecting_disclaimed_span(&self, start: i32, end: i32) -> Option<(i32, i32)> {
        self.disclaimed_spans
            .iter()
            .find(|(other_start, other_end)| start <= *other_end && *other_start <= end)
            .copied()
    }

    pub fn observe(
        &mut self,
        header: &compact_contract::CompactRecordHeader,
        payload: &[u8],
        at_byte: u64,
    ) {
        self.observe_with_tolerance(header, payload, at_byte, false)
    }

    /// Observe one record the walker has ALREADY decided to tolerate as a
    /// named loss (today: a semantically malformed DOMAIN_DATA_REPEAT whose
    /// refusal is accounted in an exclusion ledger). Framing, hour-span and
    /// sequence accounting still run; the SEMANTIC rules do not, because
    /// their violation is exactly what the walker tolerated — latching it
    /// here would surface as `boundary_error` and fail certification for a
    /// record the walk had already forgiven (Codex review round 16,
    /// finding 4).
    pub fn observe_tolerated(
        &mut self,
        header: &compact_contract::CompactRecordHeader,
        payload: &[u8],
        at_byte: u64,
    ) {
        self.observe_with_tolerance(header, payload, at_byte, true)
    }

    fn observe_with_tolerance(
        &mut self,
        header: &compact_contract::CompactRecordHeader,
        payload: &[u8],
        at_byte: u64,
        tolerate_record_semantics: bool,
    ) {
        let record_len: u64 = header.record_len().try_into().unwrap_or(u64::MAX);
        if self.resync.is_some() {
            self.absorb_into_resync(header, at_byte, record_len);
            return;
        }
        let span_was_open = self.hour_span.is_some();
        self.absorb_into_hour(header, payload, at_byte);
        let latched = self.violation.take();
        self.observe_rules(header, payload, at_byte, tolerate_record_semantics);
        let fired = self.violation.take();
        self.violation = latched.or_else(|| fired.clone());
        match fired {
            Some(reason) if self.records_lost_at_injury && !span_was_open => {
                self.reconcile_hours_lost_between_frames(header, payload, at_byte, reason)
            }
            Some(reason) => self.begin_resync(header, at_byte, reason),
            None => self.close_hour_if_boundary(header),
        }
    }

    fn reconcile_hours_lost_between_frames(
        &mut self,
        header: &compact_contract::CompactRecordHeader,
        payload: &[u8],
        at_byte: u64,
        reason: String,
    ) {
        self.records_lost_at_injury = false;
        if let Some(last_closed) = self.last_closed_hour {
            let mut missing = last_closed.saturating_add(1);
            const MAX_ENUMERATED_MISSING_HOURS: i32 = 512;
            if header.game_hour_raw.saturating_sub(missing) > MAX_ENUMERATED_MISSING_HOURS {
                self.coverage.note_unattributed_loss(format!(
                    "hours {missing}..={} are unaccounted for: {reason}",
                    header.game_hour_raw.saturating_sub(1)
                ));
            } else {
                while missing < header.game_hour_raw {
                    self.coverage.record_excluded(
                        missing,
                        at_byte,
                        0,
                        format!("game hour {missing} was never delivered: {reason}"),
                    );
                    missing = missing.saturating_add(1);
                }
            }
        } else {
            self.coverage.note_unattributed_loss(reason.clone());
        }
        self.coverage.record_excluded(
            header.game_hour_raw,
            at_byte,
            0,
            format!(
                "game hour {} may be missing records of its own: {reason}, and nothing binds \
                 an hour to a record count, so the surviving records cannot be shown to be \
                 all of them",
                header.game_hour_raw
            ),
        );
        self.open_hour = None;
        self.pending_perf_hour = None;
        self.terminal_outcome_domains.clear();
        self.gap_domains.clear();
        self.data_domains.clear();
        self.prev_sequence = header.sequence.checked_sub(1);
        self.lost_between_frames = (self.lost_between_frames.0.saturating_add(1), reason);
        let latched = self.violation.take();
        self.observe_rules(header, payload, at_byte, false);
        let fired = self.violation.take();
        self.violation = latched.or_else(|| fired.clone());
        match fired {
            Some(second) => self.begin_resync(header, at_byte, second),
            None => self.close_hour_if_boundary(header),
        }
    }

    fn absorb_into_hour(
        &mut self,
        header: &compact_contract::CompactRecordHeader,
        payload: &[u8],
        at_byte: u64,
    ) {
        if matches!(
            KnownRecordType::try_from(header.record_type),
            Ok(KnownRecordType::StreamHeader | KnownRecordType::FreezeCensus)
        ) {
            return;
        }
        let span = self.hour_span.get_or_insert_with(|| OpenHourSpan {
            game_hour_raw: header.game_hour_raw,
            first_byte: at_byte,
            len: 0,
            hasher: Sha256::new(),
        });
        span.absorb(header, payload);
    }

    fn close_hour_if_boundary(&mut self, header: &compact_contract::CompactRecordHeader) {
        match header.record_type {
            COMPACT_RECORD_TYPE_FRAME_END => {
                if let Some(span) = self.hour_span.take() {
                    if self.framing_damage_seen {
                        self.coverage.record_excluded(
                            span.game_hour_raw,
                            span.first_byte,
                            span.len,
                            concat!(
                                "this hour was recovered after framing damage earlier in ",
                                "the stream; a resynchronised record boundary is a guess ",
                                "rather than a proof, so its records are readable but not ",
                                "vouched for"
                            ),
                        );
                    } else {
                        let basis = span.basis();
                        self.coverage.record_verified(
                            span.game_hour_raw,
                            span.first_byte,
                            span.len,
                            basis,
                        );
                    }
                }
            }
            COMPACT_RECORD_TYPE_FRAME_REFUSED => {
                if let Some(span) = self.hour_span.take() {
                    self.coverage.record_excluded(
                        span.game_hour_raw,
                        span.first_byte,
                        span.len,
                        format!(
                            "the producer refused this frame (FRAME_REFUSED at byte {}); the \
                             hour was assembled and judged unpublishable",
                            span.first_byte.saturating_add(span.len)
                        ),
                    );
                }
            }
            _ => {}
        }
    }

    fn begin_resync(
        &mut self,
        header: &compact_contract::CompactRecordHeader,
        at_byte: u64,
        reason: String,
    ) {
        let span = self.hour_span.take();
        let (first_byte, len) = span
            .as_ref()
            .map(|span| (span.first_byte, span.len))
            .unwrap_or((at_byte, 0));
        let mut hours = BTreeSet::new();
        if let Some(span) = span.as_ref() {
            hours.insert(span.game_hour_raw);
        }
        if let Some(last_closed) = self.last_closed_hour {
            let next = header.game_hour_raw;
            if next > last_closed {
                let mut missing = last_closed.saturating_add(1);
                const MAX_ENUMERATED_MISSING_HOURS: i32 = 512;
                if next.saturating_sub(missing) > MAX_ENUMERATED_MISSING_HOURS {
                    self.coverage.note_unattributed_loss(format!(
                        "hours {missing}..={} are unaccounted for after {reason}",
                        next.saturating_sub(1)
                    ));
                } else {
                    while missing < next {
                        hours.insert(missing);
                        missing = missing.saturating_add(1);
                    }
                }
            }
        }
        self.resync = Some(ResyncState {
            reason,
            hours,
            first_byte,
            len,
            records_absorbed: 0,
            abandoned_signalled: false,
            hours_overflowed: false,
            lost_records: std::mem::take(&mut self.records_lost_at_injury),
        });
    }

    fn absorb_into_resync(
        &mut self,
        header: &compact_contract::CompactRecordHeader,
        _at_byte: u64,
        record_len: u64,
    ) {
        let closing = matches!(
            header.record_type,
            COMPACT_RECORD_TYPE_FRAME_END | COMPACT_RECORD_TYPE_FRAME_REFUSED
        );
        let mut abandoned = None;
        {
            let Some(state) = self.resync.as_mut() else {
                return;
            };
            state.len = state.len.saturating_add(record_len);
            state.records_absorbed = state.records_absorbed.saturating_add(1);
            if state.hours.len() >= MAX_RESYNC_CLAIMED_HOURS
                && !state.hours.contains(&header.game_hour_raw)
            {
                state.hours_overflowed = true;
                if closing {
                    self.finish_resync(header);
                }
                return;
            }
            if let Some(first) = state.hours.iter().next().copied() {
                if header.game_hour_raw > first && !state.abandoned_signalled {
                    state.abandoned_signalled = true;
                    abandoned = Some((first, state.reason.clone()));
                }
            }
            state.hours.insert(header.game_hour_raw);
        }
        if let Some((hour, reason)) = abandoned {
            self.abandoned_hour = (
                self.abandoned_hour.0.saturating_add(1),
                hour,
                format!(
                    "game hour {hour} lost its own frame boundary to stream damage ({reason}),                      so it is closed here without publishing"
                ),
            );
        }
        if closing {
            self.finish_resync(header);
        }
    }

    fn finish_resync(&mut self, header: &compact_contract::CompactRecordHeader) {
        let Some(state) = self.resync.take() else {
            return;
        };
        for hour in &state.hours {
            self.coverage
                .record_excluded(*hour, state.first_byte, state.len, state.reason.clone());
        }
        if state.hours_overflowed {
            self.coverage.note_unattributed_loss(format!(
                "the damaged span from byte {} claimed more than {MAX_RESYNC_CLAIMED_HOURS} \
                 distinct game hours; those past the bound are not enumerated individually: {}",
                state.first_byte, state.reason
            ));
        }
        self.open_hour = None;
        self.pending_perf_hour = None;
        self.terminal_outcome_domains.clear();
        self.gap_domains.clear();
        self.data_domains.clear();
        self.hour_span = None;
        self.prev_sequence = Some(header.sequence);
        self.last_record_kind = Some(header.record_type);
        self.last_closed_hour = Some(match self.last_closed_hour {
            Some(last) if last >= header.game_hour_raw => last,
            _ => header.game_hour_raw,
        });
    }

    pub fn note_framing_damage(&mut self, at_byte: u64, skipped_bytes: u64, reason: &str) {
        let hour = self.hour_span.as_ref().map(|span| span.game_hour_raw);
        let statement = match hour {
            Some(hour) => format!(
                "framing damage inside game hour {hour}: {skipped_bytes} byte(s) from byte \
                 {at_byte} could not be framed ({reason})"
            ),
            None => format!(
                "framing damage: {skipped_bytes} byte(s) from byte {at_byte} could not be \
                 framed ({reason})"
            ),
        };
        if self.violation.is_none() {
            self.violation = Some(statement.clone());
        }
        self.records_lost_at_injury = true;
        self.framing_damage_seen = true;
        if let Some(state) = self.resync.as_mut() {
            state.len = state.len.saturating_add(skipped_bytes);
            state.lost_records = true;
            self.records_lost_at_injury = false;
            return;
        }
        let span = self.hour_span.take();
        let (first_byte, len) = span
            .as_ref()
            .map(|span| (span.first_byte, span.len))
            .unwrap_or((at_byte, 0));
        let mut hours = BTreeSet::new();
        if let Some(span) = span.as_ref() {
            hours.insert(span.game_hour_raw);
        }
        self.resync = Some(ResyncState {
            reason: statement,
            hours,
            first_byte,
            len: len.saturating_add(skipped_bytes),
            records_absorbed: 0,
            abandoned_signalled: false,
            hours_overflowed: false,
            lost_records: std::mem::take(&mut self.records_lost_at_injury),
        });
    }

    pub fn coverage(&self) -> &crate::hour_coverage::HourCoverageMap {
        &self.coverage
    }

    pub fn open_hour_raw(&self) -> Option<i32> {
        self.hour_span.as_ref().map(|span| span.game_hour_raw)
    }

    pub fn lost_between_frames(&self) -> (u64, &str) {
        (
            self.lost_between_frames.0,
            self.lost_between_frames.1.as_str(),
        )
    }

    pub fn abandoned_hour(&self) -> (u64, i32, &str) {
        (
            self.abandoned_hour.0,
            self.abandoned_hour.1,
            self.abandoned_hour.2.as_str(),
        )
    }

    pub fn resync_hour(&self) -> Option<i32> {
        self.resync
            .as_ref()
            .and_then(|state| state.hours.iter().next().copied())
    }

    pub fn resynchronising(&self) -> Option<&str> {
        self.resync.as_ref().map(|state| state.reason.as_str())
    }

    pub fn resync_lost_records(&self) -> bool {
        self.resync.as_ref().is_some_and(|state| state.lost_records)
    }

    pub fn seal_coverage(&mut self, residual: &[u8]) -> crate::hour_coverage::HourCoverageMap {
        let residual_carry = residual.len();
        if let Some(state) = self.resync.take() {
            for hour in &state.hours {
                self.coverage.record_excluded(
                    *hour,
                    state.first_byte,
                    state.len,
                    state.reason.clone(),
                );
            }
            if state.hours.is_empty() {
                self.coverage.note_unattributed_loss(state.reason.clone());
            }
            if state.hours_overflowed {
                self.coverage.note_unattributed_loss(format!(
                    "the damaged span from byte {} claimed more than \
                     {MAX_RESYNC_CLAIMED_HOURS} distinct game hours; those past the bound \
                     are not enumerated individually: {}",
                    state.first_byte, state.reason
                ));
            }
        }
        if let Some(span) = self.hour_span.take() {
            self.coverage.record_excluded(
                span.game_hour_raw,
                span.first_byte,
                span.len,
                format!(
                    "game hour {} was left open at end of stream: its records were read but \
                     no canonical FRAME_END or FRAME_REFUSED ever closed it, so nothing \
                     vouches for what the hour contained",
                    span.game_hour_raw
                ),
            );
        }
        if residual_carry != 0 {
            match parse_compact_record_header(residual) {
                Ok(header) => self.coverage.record_excluded(
                    header.game_hour_raw,
                    0,
                    residual_carry as u64,
                    format!(
                        "the stream ends mid-record inside game hour {}: {residual_carry} \
                         residual byte(s) form no complete record, so this hour and every \
                         hour after it was never delivered",
                        header.game_hour_raw
                    ),
                ),
                Err(_) => {
                    let last = self
                        .coverage
                        .rows()
                        .last()
                        .map(|row| row.game_hour_raw.to_string())
                        .unwrap_or_else(|| "none".to_string());
                    self.coverage.note_unattributed_loss(format!(
                        "the stream ends mid-record: {residual_carry} residual byte(s) after \
                         the last hour the walk reached (hour {last}) form no complete \
                         record, and no readable header remains to name the hour they were \
                         carrying"
                    ));
                }
            }
        }
        if let Some(termination) = &self.termination {
            self.coverage.note_unattributed_loss(format!(
                "capture terminated abnormally after {} ms: reasons={:?}",
                termination.elapsed_wall_ms,
                termination.reasons.names(),
            ));
        }
        self.coverage.clone()
    }

    fn observe_rules(
        &mut self,
        header: &compact_contract::CompactRecordHeader,
        payload: &[u8],
        at_byte: u64,
        tolerate_record_semantics: bool,
    ) {
        if self.termination.is_some() {
            self.violation = Some(format!(
                "record type {} appears at byte {at_byte} after CAPTURE_TERMINATED",
                header.record_type
            ));
            return;
        }
        if let Err(reason) =
            compact_contract::freeze_census_record::validate_record_allowed_after_release_epilogue(
                self.release_freeze_census_seen,
            )
        {
            self.violation = Some(format!("record at byte {at_byte} is illegal: {reason}"));
            return;
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
                    self.records_lost_at_injury = true;
                    self.violation = Some(format!(
                        "sequence overflow at byte {at_byte}: {prev} cannot be followed \
                         without wrapping"
                    ));
                    return;
                };
                if header.sequence != expected {
                    self.records_lost_at_injury = true;
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

        if !tolerate_record_semantics {
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
        }

        let record_type = match KnownRecordType::try_from(header.record_type) {
            Ok(record_type) => record_type,
            Err(_) => {
                self.violation = Some(format!(
                    "unknown compact record type {} at byte {at_byte}: this fold has not been taught to close, count, or ignore it",
                    header.record_type
                ));
                return;
            }
        };
        let previous_record_type = self
            .last_record_kind
            .and_then(|raw| KnownRecordType::try_from(raw).ok());
        self.last_record_kind = Some(header.record_type);
        match record_type {
            KnownRecordType::DomainPerf | KnownRecordType::DomainGap => {
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
            // A repeat is a data record of a new kind: the owning hour stays
            // covered, and it obeys the same gap-exclusion and
            // hour-monotonicity rules as DOMAIN_DATA. A domain may still carry
            // several data records of either kind in one hour (d53 snapshot +
            // controller carriers), so repeats add no uniqueness rule here.
            KnownRecordType::DomainData | KnownRecordType::DomainDataRepeat => {
                if self.gap_domains.contains(&header.domain_id) {
                    self.violation = Some(format!(
                        "{} for domain {} in hour {} coexists with DOMAIN_GAP at byte {at_byte}",
                        record_type.canonical_name().to_uppercase(),
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
            KnownRecordType::FrameEnd => {
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
                if let Some((start, end)) = self.unobserved_interval_covering(header.game_hour_raw)
                {
                    self.violation = Some(format!(
                        "FRAME_END at byte {at_byte} closes hour {}, which an earlier \
                         CLOCK_UNREADABLE_INTERVAL declared never observed ({start}..={end})",
                        header.game_hour_raw
                    ));
                    return;
                }
                self.open_hour = None;
                self.pending_perf_hour = None;
                self.terminal_outcome_domains.clear();
                self.gap_domains.clear();
                self.data_domains.clear();
                self.last_closed_hour = Some(header.game_hour_raw);
            }
            KnownRecordType::FrameRefused => {
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
                match compact_contract::frame_refusal::FrameRefusalRecordV2::decode(payload) {
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
                if let Some((start, end)) = self.unobserved_interval_covering(header.game_hour_raw)
                {
                    self.violation = Some(format!(
                        "FRAME_REFUSED at byte {at_byte} refuses hour {}, which an earlier \
                         CLOCK_UNREADABLE_INTERVAL declared never observed ({start}..={end})",
                        header.game_hour_raw
                    ));
                    return;
                }
                self.open_hour = None;
                self.pending_perf_hour = None;
                self.terminal_outcome_domains.clear();
                self.gap_domains.clear();
                self.data_domains.clear();
                self.last_closed_hour = Some(header.game_hour_raw);
            }
            KnownRecordType::FrameDegraded => {
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
                        self.violation = Some(format!(
                            "FRAME_DEGRADED at byte {at_byte} degrades hour {} but no hour is open",
                            header.game_hour_raw
                        ));
                        return;
                    }
                }
                match compact_contract::frame_degraded::FrameDegradedRecordV2::decode(payload) {
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
            KnownRecordType::CaptureTerminated => {
                if header.game_hour_raw != 0 {
                    self.violation = Some(format!(
                        "CAPTURE_TERMINATED at byte {at_byte} uses game hour {}; expected zero no-coverage sentinel",
                        header.game_hour_raw
                    ));
                    return;
                }
                if let Some(open) = self.open_hour.or(self.pending_perf_hour) {
                    self.violation = Some(format!(
                        "CAPTURE_TERMINATED at byte {at_byte} leaves game hour {open} open"
                    ));
                    return;
                }
                match compact_contract::capture_termination::CaptureTerminationRecordV1::decode(
                    payload,
                ) {
                    Ok(record) => self.termination = Some(record),
                    Err(reason) => {
                        self.violation = Some(format!(
                            "CAPTURE_TERMINATED payload at byte {at_byte} is unreadable: {reason}"
                        ));
                    }
                }
            }
            KnownRecordType::ClockUnreadableInterval => {
                let record = match compact_contract::clock_unreadable_interval::
                    ClockUnreadableIntervalRecordV1::decode(payload)
                {
                    Ok(record) => record,
                    Err(reason) => {
                        self.violation = Some(format!(
                            "CLOCK_UNREADABLE_INTERVAL payload at byte {at_byte} is \
                             unreadable: {reason}"
                        ));
                        return;
                    }
                };
                if record.resumed_hour_raw != header.game_hour_raw {
                    self.violation = Some(format!(
                        "CLOCK_UNREADABLE_INTERVAL at byte {at_byte} is framed for hour {} but \
                         its payload resumed at hour {}",
                        header.game_hour_raw, record.resumed_hour_raw
                    ));
                    return;
                }
                if let Some(last_closed) = self.last_closed_hour {
                    if record.range_start_hour_raw <= last_closed {
                        self.violation = Some(format!(
                            "CLOCK_UNREADABLE_INTERVAL at byte {at_byte} claims hours \
                             {}..={} were never observed, but hour {last_closed} is already \
                             closed",
                            record.range_start_hour_raw, record.range_end_hour_raw
                        ));
                        return;
                    }
                }
                if let Some((start, end)) = self.intersecting_unobserved_interval(
                    record.range_start_hour_raw,
                    record.range_end_hour_raw,
                ) {
                    self.violation = Some(format!(
                        "CLOCK_UNREADABLE_INTERVAL at byte {at_byte} claims hours {}..={} \
                         were never observed, overlapping the interval {start}..={end} \
                         already declared",
                        record.range_start_hour_raw, record.range_end_hour_raw
                    ));
                    return;
                }
                if let Some((start, end)) = self.intersecting_disclaimed_span(
                    record.range_start_hour_raw,
                    record.range_end_hour_raw,
                ) {
                    self.violation = Some(format!(
                        "CLOCK_UNREADABLE_INTERVAL at byte {at_byte} claims hours {}..={} \
                         were never observed, but an UNTRUSTED_CLOCK_RECOVERY already \
                         disclaimed {start}..={end}",
                        record.range_start_hour_raw, record.range_end_hour_raw
                    ));
                    return;
                }
                self.unobserved_intervals
                    .push((record.range_start_hour_raw, record.range_end_hour_raw));
            }
            KnownRecordType::UntrustedClockRecovery => {
                let record = match compact_contract::untrusted_clock_recovery::
                    UntrustedClockRecoveryRecordV1::decode(payload)
                {
                    Ok(record) => record,
                    Err(reason) => {
                        self.violation = Some(format!(
                            "UNTRUSTED_CLOCK_RECOVERY payload at byte {at_byte} is \
                             unreadable: {reason}"
                        ));
                        return;
                    }
                };
                if record.resumed_hour_raw != header.game_hour_raw {
                    self.violation = Some(format!(
                        "UNTRUSTED_CLOCK_RECOVERY at byte {at_byte} is framed for hour {} but \
                         its payload resumed at hour {}",
                        header.game_hour_raw, record.resumed_hour_raw
                    ));
                    return;
                }
                if let Some(last_closed) = self.last_closed_hour {
                    if record.resumed_hour_raw <= last_closed {
                        self.violation = Some(format!(
                            "UNTRUSTED_CLOCK_RECOVERY at byte {at_byte} resumes at hour {}, \
                             which is not after the already-closed hour {last_closed}",
                            record.resumed_hour_raw
                        ));
                        return;
                    }
                }
                let (disclaimed_start, disclaimed_end) = (
                    record.last_observed_hour_raw.saturating_add(1),
                    record.resumed_hour_raw.saturating_sub(1),
                );
                if let Some((start, end)) =
                    self.intersecting_unobserved_interval(disclaimed_start, disclaimed_end)
                {
                    self.violation = Some(format!(
                        "UNTRUSTED_CLOCK_RECOVERY at byte {at_byte} disclaims hours \
                         {disclaimed_start}..={disclaimed_end} after a \
                         CLOCK_UNREADABLE_INTERVAL already declared {start}..={end} never \
                         observed"
                    ));
                    return;
                }
                self.disclaimed_spans
                    .push((disclaimed_start, disclaimed_end));
            }
            KnownRecordType::FreezeCensus => {
                if header.game_hour_raw != 0 {
                    self.violation = Some(format!(
                        "FREEZE_CENSUS at byte {at_byte} is framed for hour {}, not the zero no-coverage sentinel",
                        header.game_hour_raw
                    ));
                    return;
                }
                let census =
                    match compact_contract::freeze_census_record::FreezeCensusRecordV1::decode(
                        payload,
                    ) {
                        Ok(census) => census,
                        Err(reason) => {
                            self.violation = Some(format!(
                                "FREEZE_CENSUS payload at byte {at_byte} is unreadable: {reason}"
                            ));
                            return;
                        }
                    };
                let ordinary_before = header.sequence.saturating_sub(1);
                match compact_contract::freeze_census_record::
                    validate_freeze_census_placement_policy(
                        census.phase(),
                        header.sequence,
                        ordinary_before,
                        previous_record_type,
                        self.release_freeze_census_seen,
                    )
                {
                    Ok(compact_contract::freeze_census_record::FreezeCensusPlacement::InstallPreamble) => {}
                    Ok(compact_contract::freeze_census_record::FreezeCensusPlacement::ReleaseEpilogue) => {
                        self.release_freeze_census_seen = true;
                    }
                    Err(reason) => {
                self.violation = Some(format!(
                            "FREEZE_CENSUS placement at byte {at_byte} is invalid: {reason}"
                ));
            }
        }
            }
            KnownRecordType::ContentIdentityCensus => {
                if header.game_hour_raw != 0 {
                    self.violation = Some(format!(
                        "CONTENT_IDENTITY_CENSUS at byte {at_byte} is framed for hour {}, not the zero no-coverage sentinel",
                        header.game_hour_raw
                    ));
                    return;
                }
                if let Err(reason) =
                    compact_contract::content_identity_census::ContentIdentityCensusV1::decode(
                        payload,
                    )
                {
                    self.violation = Some(format!(
                        "CONTENT_IDENTITY_CENSUS payload at byte {at_byte} is unreadable: {reason}"
                    ));
                    return;
                }
                let ordinary_before = header.sequence.saturating_sub(1);
                match compact_contract::content_identity_census::
                    validate_content_identity_census_placement(
                        ordinary_before,
                        previous_record_type,
                        self.content_identity_census_seen,
                    )
                {
                    Ok(()) => self.content_identity_census_seen = true,
                    Err(reason) => {
                        self.violation = Some(format!(
                            "CONTENT_IDENTITY_CENSUS placement at byte {at_byte} is invalid: {reason}"
                        ));
                    }
                }
            }
            KnownRecordType::StateTopologyCensus => {
                if header.game_hour_raw != 0 {
                    self.violation = Some(format!(
                        "STATE_TOPOLOGY_CENSUS at byte {at_byte} is framed for hour {}, not the zero no-coverage sentinel",
                        header.game_hour_raw
                    ));
                    return;
                }
                if let Err(reason) =
                    compact_contract::state_topology_census::StateTopologyCensusV1::decode(payload)
                {
                    self.violation = Some(format!(
                        "STATE_TOPOLOGY_CENSUS payload at byte {at_byte} is unreadable: {reason}"
                    ));
                    return;
                }
                let ordinary_before = header.sequence.saturating_sub(1);
                match compact_contract::state_topology_census::
                    validate_state_topology_census_placement(
                        ordinary_before,
                        previous_record_type,
                        self.state_topology_census_seen,
                    )
                {
                    Ok(()) => self.state_topology_census_seen = true,
                    Err(reason) => {
                        self.violation = Some(format!(
                            "STATE_TOPOLOGY_CENSUS placement at byte {at_byte} is invalid: {reason}"
                        ));
                    }
                }
            }
            KnownRecordType::StreamHeader => {}
        }
    }

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
        if let Some(termination) = &self.termination {
            return Err(format!(
                "capture terminated abnormally after {} ms: reasons={:?}; detail={}{}",
                termination.elapsed_wall_ms,
                termination.reasons.names(),
                termination.detail,
                if termination.detail_truncated {
                    " [detail truncated]"
                } else {
                    ""
                }
            ));
        }
        if self.last_closed_hour.is_none() {
            return Err(
                "stream closes no game hour (no FRAME_END or FRAME_REFUSED records)".to_string(),
            );
        }
        let physical_close_is_valid =
            boundary_eof_policy(self.last_record_kind, self.release_freeze_census_seen);
        if !physical_close_is_valid {
            return Err(format!(
                "stream ends after record type {}; final record must be a canonical FRAME_END or FRAME_REFUSED boundary, or the single release FREEZE_CENSUS epilogue behind one",
                self.last_record_kind
                    .map_or_else(|| "none".to_string(), |kind| kind.to_string())
            ));
        }
        Ok(())
    }
}

/// A stream ends on a canonical closed-hour boundary, or on the single release
/// `release_freeze_census_seen` bit is only ever set by
fn boundary_eof_policy(last_record_kind: Option<u16>, release_freeze_census_seen: bool) -> bool {
    matches!(
        last_record_kind,
        Some(COMPACT_RECORD_TYPE_FRAME_END) | Some(COMPACT_RECORD_TYPE_FRAME_REFUSED)
    ) || (release_freeze_census_seen
        && matches!(last_record_kind, Some(COMPACT_RECORD_TYPE_FREEZE_CENSUS)))
}

#[derive(Debug)]
pub enum FoldDamageStep {
    LostRecords { hour: i32, reason: String },
    CloseAbandonedHour { hour: i32, reason: String },
    Clean,
}

pub fn apply_fold_damage_step(step: FoldDamageStep, fold: &mut WorldFold) {
    match step {
        FoldDamageStep::LostRecords { hour, reason } => fold.record_framing_damage(hour, &reason),
        FoldDamageStep::CloseAbandonedHour { hour, reason } => {
            fold.close_damaged_hour(hour, &reason)
        }
        FoldDamageStep::Clean => {}
    }
}

pub struct FoldDamageWatch {
    resync_reason: Option<String>,
    resync_hour: Option<i32>,
    open_hour: Option<i32>,
    lost_between_frames: u64,
    abandoned_hours: u64,
}

impl FoldDamageWatch {
    pub fn before(tracker: &BoundaryTracker) -> Self {
        Self {
            resync_reason: tracker.resynchronising().map(str::to_string),
            resync_hour: tracker.resync_hour(),
            open_hour: tracker.open_hour_raw(),
            lost_between_frames: tracker.lost_between_frames().0,
            abandoned_hours: tracker.abandoned_hour().0,
        }
    }

    pub fn after(self, tracker: &BoundaryTracker, fallback_hour: i32) -> FoldDamageStep {
        match (self.resync_reason, tracker.resynchronising()) {
            (None, Some(reason)) => match (tracker.resync_lost_records(), self.open_hour) {
                (true, Some(hour)) => FoldDamageStep::LostRecords {
                    hour,
                    reason: reason.to_string(),
                },
                _ => FoldDamageStep::Clean,
            },
            _ => {
                let (abandoned_events, abandoned_hour, abandoned_reason) = tracker.abandoned_hour();
                if abandoned_events > self.abandoned_hours {
                    return FoldDamageStep::CloseAbandonedHour {
                        hour: abandoned_hour,
                        reason: abandoned_reason.to_string(),
                    };
                }
                let (events, reason) = tracker.lost_between_frames();
                if events > self.lost_between_frames {
                    FoldDamageStep::LostRecords {
                        hour: fallback_hour,
                        reason: reason.to_string(),
                    }
                } else {
                    FoldDamageStep::Clean
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramingResync {
    At(usize),
    NeedMoreData,
    Exhausted,
}

pub fn find_frame_resync_offset(buf: &[u8], from: usize) -> FramingResync {
    let Some(scan_end) = buf.len().checked_sub(COMPACT_RECORD_HEADER_LEN) else {
        return FramingResync::NeedMoreData;
    };
    let mut candidate = from.saturating_add(1);
    while candidate <= scan_end {
        if &buf[candidate..candidate + COMPACT_RECORD_MAGIC.len()] != COMPACT_RECORD_MAGIC {
            candidate += 1;
            continue;
        }
        let Ok(header) = parse_compact_record_header(&buf[candidate..]) else {
            candidate += 1;
            continue;
        };
        let end = candidate.saturating_add(header.record_len());
        if end > buf.len() {
            return FramingResync::NeedMoreData;
        }
        if end == buf.len() || parse_compact_record_header(&buf[end..]).is_ok() {
            return FramingResync::At(candidate);
        }
        candidate += 1;
    }
    FramingResync::NeedMoreData
}

pub fn framing_stop_text(error: RecordFramingError, at: u64) -> String {
    match error {
        RecordFramingError::TruncatedHeader(actual) => {
            format!("truncated record header ({actual} byte(s)) at byte {at}")
        }
        RecordFramingError::BadMagic => format!("invalid record magic at byte {at}"),
        RecordFramingError::UnsupportedVersion(v) => {
            format!("unsupported record version {v} at byte {at}")
        }
        RecordFramingError::UnsupportedHeaderLen(l) => {
            format!("unsupported header length {l} at byte {at}")
        }
        RecordFramingError::PayloadTooLarge(l) => {
            format!("payload length {l} exceeds the record bound at byte {at}")
        }
    }
}

pub fn strict_stream_certification_verdict(
    stop_reason: Option<&str>,
    truncated_tail_bytes: u64,
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

pub fn verify_stream_logical_boundary(
    stream_path: &Path,
) -> io::Result<Vec<ExcludedDomainDataRecord>> {
    verify_stream_logical_boundary_with_identity_mode(stream_path, StreamIdentityMode::Legacy)
}

pub fn verify_stream_logical_boundary_with_identity_mode(
    stream_path: &Path,
    identity_mode: StreamIdentityMode,
) -> io::Result<Vec<ExcludedDomainDataRecord>> {
    let outcome = walk_stream_hour_coverage(stream_path, identity_mode)?;
    match outcome.boundary_error {
        Some(reason) => Err(io::Error::new(io::ErrorKind::InvalidData, reason)),
        None => Ok(outcome.excluded_domain_data_records),
    }
}

pub struct SealWalkOutcome {
    pub excluded_domain_data_records: Vec<ExcludedDomainDataRecord>,
    pub decoded_sha256: String,
    pub hour_coverage: crate::hour_coverage::HourCoverageMap,
    pub boundary_error: Option<String>,
}

pub fn walk_stream_hour_coverage(
    stream_path: &Path,
    identity_mode: StreamIdentityMode,
) -> io::Result<SealWalkOutcome> {
    let mut reader = compact_stream::open(stream_path)?;
    let mut carry: Vec<u8> = Vec::with_capacity(CHUNK * 2);
    let mut chunk = vec![0u8; CHUNK];
    let mut processed: u64 = 0;
    let mut tracker = BoundaryTracker::default();
    let mut validator = stream_validator(identity_mode);
    let mut excluded = Vec::new();
    let mut hard_error: Option<String> = None;
    let mut decoded_hasher = Sha256::new();
    // The seal walk's own `PSD1` baselines. It is a third INDEPENDENT walker of
    // the same stream -- the fold and the manifest summary are the other two --
    // and each keeps its own store rather than sharing one, because they run at
    // different times and, for the fold, in eight parallel copies.
    let mut span_delta_consumer =
        compact_contract::payload_span_delta::PayloadSpanDeltaConsumer::new();
    // The framing-layer RAW repeat baselines: a semantically valid repeat is
    // COVERED DATA only after this walk verifies it exactly like every other
    // reader does — resolution digest, then PSD1 composition when the flag
    // word says the repeated hour rode enveloped (Codex review round 19,
    // finding 3).
    let mut repeat_baselines = compact_contract::DomainRepeatBaselines::new();
    'read: loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        decoded_hasher.update(&chunk[..n]);
        carry.extend_from_slice(&chunk[..n]);
        let mut off = 0usize;
        loop {
            let remaining = carry.len() - off;
            if remaining < COMPACT_RECORD_HEADER_LEN {
                break;
            }
            let at = processed + off as u64;
            let header = match parse_compact_record_header(&carry[off..]) {
                Ok(header) => header,
                Err(e) => {
                    if hard_error.is_none() {
                        hard_error = Some(format!("seal-time boundary check: {e} at byte {at}"));
                    }
                    match find_frame_resync_offset(&carry, off) {
                        FramingResync::At(next) => {
                            tracker.note_framing_damage(
                                at,
                                (next - off) as u64,
                                &framing_stop_text(e, at),
                            );
                            off = next;
                            continue;
                        }
                        FramingResync::NeedMoreData
                            if carry.len() - off <= MAX_RESYNC_WINDOW_BYTES =>
                        {
                            break;
                        }
                        FramingResync::NeedMoreData | FramingResync::Exhausted => {
                            let keep = COMPACT_RECORD_HEADER_LEN.saturating_sub(1);
                            let discard = (carry.len() - off).saturating_sub(keep);
                            if discard == 0 {
                                break;
                            }
                            tracker.note_framing_damage(
                                at,
                                discard as u64,
                                &framing_stop_text(e, at),
                            );
                            off += discard;
                            continue;
                        }
                    }
                }
            };
            let record_len = header.record_len();
            if record_len > remaining {
                break;
            }
            let payload = &carry[off + COMPACT_RECORD_HEADER_LEN..off + record_len];
            let accepted = match validator.validate_record(&header, payload) {
                Ok(accepted) => accepted,
                // A semantically malformed DOMAIN_DATA_REPEAT is a LOST
                // RECORD, not a seal-breaking violation (Codex review round
                // 14, finding 1): excluded and named like any refused repeat,
                // the hour still seals.
                Err(
                    error @ (compact_contract::stream_validator::StreamValidationError::InvalidRecordPayload(_)
                    | compact_contract::stream_validator::StreamValidationError::PayloadLengthMismatch { .. }
                    | compact_contract::stream_validator::StreamValidationError::ReservedDomainZero),
                ) if header.record_type
                    == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT =>
                {
                    // Only SEMANTIC failures of a type-14 record are
                    // tolerable losses; sequence-level errors stay fatal
                    // (Codex review round 18, finding 2).
                    excluded.push(ExcludedDomainDataRecord {
                        stream_offset: at,
                        game_hour_raw: header.game_hour_raw,
                        domain_id: header.domain_id,
                        validation_reason: format!(
                            "DOMAIN_DATA_REPEAT failed stream validation: {error}"
                        ),
                    });
                    validator.tolerate_failed_record(&header);
                    tracker.observe_tolerated(&header, payload, at);
                    off += record_len;
                    continue;
                }
                Err(error) => {
                    if hard_error.is_none() {
                        hard_error = Some(format!(
                            "seal-time stream validation failed at byte {at}: {error}"
                        ));
                    }
                    tracker.note_framing_damage(
                        at,
                        record_len as u64,
                        &format!("stream validation failed: {error}"),
                    );
                    off += record_len;
                    continue;
                }
            };
            if accepted == AcceptedRecordKind::CampaignIdentity {
                off += record_len;
                continue;
            }
            if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
                // Seed the framing-layer RAW baseline exactly as the
                // collector's gate did when this record published — without
                // it every later honest repeat would refuse "no stored
                // baseline" (Codex review round 20, finding 2).
                let _ = repeat_baselines.observe(
                    header.domain_id,
                    header.record_type,
                    payload,
                );
                // UNWRAP FIRST: a `PSD1` envelope is validated as the grammar it
                // reconstructs to, exactly as it was before the delta existed. A
                // record that cannot be reconstructed is EXCLUDED AND NAMED,
                // which is this walk's existing outcome for a payload it cannot
                // validate -- the hour still seals, and it seals saying which
                // record it is missing and why.
                let reconstructed =
                    match compact_contract::payload_span_delta::reconstruct_if_enveloped(
                        &mut span_delta_consumer,
                        header.record_flags,
                        compact_contract::payload_span_delta::PayloadSpanDeltaIdentity {
                            domain_id: header.domain_id,
                            source_sequence: header.sequence,
                            game_hour_raw: header.game_hour_raw,
                        },
                        payload,
                    ) {
                        Ok(reconstructed) => reconstructed,
                        Err(validation_reason) => {
                            excluded.push(ExcludedDomainDataRecord {
                                stream_offset: at,
                                game_hour_raw: header.game_hour_raw,
                                domain_id: header.domain_id,
                                validation_reason,
                            });
                            tracker.observe(&header, payload, at);
                            off += record_len;
                            continue;
                        }
                    };
                // The envelope flag is CLEARED for the validator: the payload
                // it is being handed is the reconstructed grammar, not an
                // envelope, and the flag describes the payload.
                let downstream_flags = if reconstructed.is_some() {
                    header.record_flags
                        & !compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA
                } else {
                    header.record_flags
                };
                if let Err(validation_reason) = validate_domain_data_payload_at(
                    header.domain_id,
                    reconstructed.as_deref().unwrap_or(payload),
                    downstream_flags,
                    header.sequence,
                    header.game_hour_raw,
                    at,
                ) {
                    excluded.push(ExcludedDomainDataRecord {
                        stream_offset: at,
                        game_hour_raw: header.game_hour_raw,
                        domain_id: header.domain_id,
                        validation_reason,
                    });
                }
            }
            if header.record_type
                == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
            {
                // VERIFY, DON'T TRUST THE BYTES' PRESENCE (Codex review round
                // 19, finding 3): the framing gate resolves the repeated
                // hour's raw on-wire bytes, and — when the preserved flag word
                // says they are an envelope — `repeat_basis` proves them
                // against this walk's own retained output. The RESOLVED
                // payload then passes the same domain-grammar validation a
                // full record's does: digest-valid resolution of malformed
                // bytes is still a lost hour (Codex review round 21).
                // A refusal is excluded and named; the hour's bytes still
                // seal.
                let verification = match repeat_baselines.observe(
                    header.domain_id,
                    header.record_type,
                    payload,
                ) {
                    Err(error) => Err(error),
                    Ok(resolved) => match resolved {
                        Some(bytes) => {
                            let composed = if header.record_flags
                                & compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA
                                != 0
                            {
                                compact_contract::payload_span_delta::repeat_basis(
                                    &span_delta_consumer,
                                    header.domain_id,
                                    &bytes,
                                )
                            } else {
                                Ok(bytes.to_vec())
                            };
                            match composed {
                                Ok(resolved_payload) => {
                                    let validation_flags = if header.record_flags
                                        & compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA
                                        != 0
                                    {
                                        header.record_flags
                                            & !compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA
                                    } else {
                                        header.record_flags
                                    };
                                    validate_domain_data_payload_at(
                                        header.domain_id,
                                        &resolved_payload,
                                        validation_flags,
                                        header.sequence,
                                        header.game_hour_raw,
                                        at,
                                    )
                                    .map_err(|error| {
                                        format!(
                                            "repeated payload failed domain validation: {error}"
                                        )
                                    })
                                }
                                Err(reason) => Err(format!(
                                    "DOMAIN_DATA_REPEAT composition refused: {reason}"
                                )),
                            }
                        }
                        None => Ok(()),
                    },
                };
                if let Err(reason) = verification {
                    excluded.push(ExcludedDomainDataRecord {
                        stream_offset: at,
                        game_hour_raw: header.game_hour_raw,
                        domain_id: header.domain_id,
                        validation_reason: format!("DOMAIN_DATA_REPEAT refused: {reason}"),
                    });
                    tracker.observe_tolerated(&header, payload, at);
                    off += record_len;
                    continue;
                }
            }
            tracker.observe(&header, payload, at);
            off += record_len;
        }
        processed += off as u64;
        carry.drain(0..off);
    }
    let boundary_error = hard_error.or_else(|| {
        tracker
            .eof_check(carry.len())
            .err()
            .map(|reason| format!("seal-time boundary check: {reason}"))
    });
    let hour_coverage = tracker.seal_coverage(&carry);
    Ok(SealWalkOutcome {
        excluded_domain_data_records: excluded,
        decoded_sha256: to_hex(&decoded_hasher.finalize()),
        hour_coverage,
        boundary_error,
    })
}

pub fn default_fold_workers() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    (cores / 2).clamp(1, 8)
}

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

pub fn fold_compact_stream_file_parallel_with_identity_mode(
    stream_path: &Path,
    game_id: &str,
    snapshot_dir: &Path,
    workers: usize,
    identity_mode: StreamIdentityMode,
) -> io::Result<(Vec<FoldedSnapshotFile>, IdentityAwareFoldStats)> {
    fold_compact_stream_file_parallel_with_identity_mode_and_content_manifest(
        stream_path,
        game_id,
        snapshot_dir,
        workers,
        identity_mode,
        None,
        None,
    )
}

pub fn fold_compact_stream_file_parallel_with_identity_mode_and_content_manifest(
    stream_path: &Path,
    game_id: &str,
    snapshot_dir: &Path,
    workers: usize,
    identity_mode: StreamIdentityMode,
    effective_content_manifest_sha256: Option<&str>,
    effective_vfs_manifest_sha256: Option<&str>,
) -> io::Result<(Vec<FoldedSnapshotFile>, IdentityAwareFoldStats)> {
    if workers <= 1 {
        return fold_compact_stream_file_with_identity_mode_and_content_manifest(
            stream_path,
            game_id,
            snapshot_dir,
            identity_mode,
            effective_content_manifest_sha256,
            effective_vfs_manifest_sha256,
        );
    }
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
                            if let Some(digest) = effective_vfs_manifest_sha256 {
                                fold.set_effective_vfs_manifest_sha256(digest);
                            }
                            if let Some(digest) = effective_content_manifest_sha256 {
                                fold.set_effective_content_manifest_sha256(digest).map_err(
                                    |error| io::Error::new(io::ErrorKind::InvalidInput, error),
                                )?;
                            }
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
        if let Some(reference_outcome) = &outcome {
            let reference = &reference_outcome.stats;
            let worker_stats = &worker_outcome.stats;
            let agree = reference.stream_bytes == worker_stats.stream_bytes
                && reference.bytes_folded == worker_stats.bytes_folded
                && reference.records_total == worker_stats.records_total
                && reference.hours_emitted == worker_stats.hours_emitted
                && reference.stop_reason == worker_stats.stop_reason
                && reference.truncated_tail_bytes == worker_stats.truncated_tail_bytes
                && reference.ends_at_frame_end == worker_stats.ends_at_frame_end
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

pub fn hash_folded_snapshot_files(
    files: &[FoldedSnapshotFile],
    discard_snapshots: bool,
) -> io::Result<Vec<FoldedSnapshotReportRow>> {
    let mut rows = Vec::with_capacity(files.len());
    for f in files {
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

/// Validate one DOMAIN_DATA payload against the H4CR header that carries it.
///
/// The header identity is a PARAMETER and never a default. Two domains
/// cross-check their payload against the enclosing record â€”
/// `session_multiplayer_metadata_live` (d57) compares its payload hour and
/// every event row's hour against `header_hour_raw`, and
/// `xmap07_snow_mud_live` (d100) compares its source sequence and hour â€” so a
/// caller that passes a placeholder does not "skip" those checks, it runs them
/// against the placeholder and fails every record.
///
/// That is not hypothetical: the seal gate and the replay decoder both used to
/// call a three-argument wrapper that substituted `0`, which made every d57
/// record in a real capture report
/// `d57 payload hour <hour> != H4CR hour 0`. On lane OB's 393-hour capture that
/// excluded all 366 d57 records as "malformed" and failed the seal gate with
/// `d57_record_count_per_closed_frame_invalid`, downgrading a good recording to
/// a recovery-only replay the viewer refuses to open. The wrapper is gone; pass
/// the real header.
pub fn validate_domain_data_payload_at(
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

/// The status flags one DOMAIN_DATA payload declares, validated against the
/// H4CR header that carries it.
///
/// Same rule as [`validate_domain_data_payload_at`]: the header identity is a
/// parameter, never a default. A placeholder here is worse than a wrong error
/// message, because the only caller maps `Err` to `None` â€” so a d57 record
/// rejected against a placeholder hour does not merely mis-report, it drops the
/// domain's declared status flags out of the completeness audit entirely.
pub fn domain_data_payload_status_at(
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
            // This caller wants only `status_flags`; it never reads a changed-row
            // count, so it does not track the d100 digest.
            xmap07_snow_mud_previous_grid_sha256: None,
        },
    )
    .map(|authority| authority.status_flags)
    .map_err(|error| error.to_string())
}

pub fn drive_fold<R: Read>(
    mut reader: R,
    total_len: u64,
    fold: &mut WorldFold,
) -> io::Result<FoldStreamStats> {
    drive_fold_with_identity_mode(reader.by_ref(), total_len, fold, StreamIdentityMode::Legacy)
        .map(|outcome| outcome.stats)
}

pub fn drive_fold_with_identity_mode<R: Read>(
    mut reader: R,
    total_len: u64,
    fold: &mut WorldFold,
    identity_mode: StreamIdentityMode,
) -> io::Result<IdentityAwareFoldStats> {
    drive_fold_compressed_with_identity_mode(reader.by_ref(), total_len, fold, false, identity_mode)
}

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
            let header = match parse_compact_record_header(&carry[off..]) {
                Ok(h) => h,
                Err(e) => {
                    let at = processed + off as u64;
                    let message = framing_stop_text(e, at);
                    if stats.stop_reason.is_none() {
                        stats.stop_reason = Some(message.clone());
                    }
                    if compressed {
                        hard_stop = true;
                        break;
                    }
                    match find_frame_resync_offset(&carry, off) {
                        FramingResync::At(next) => {
                            let skipped = (next - off) as u64;
                            let before = FoldDamageWatch::before(&tracker);
                            tracker.note_framing_damage(at, skipped, &message);
                            apply_fold_damage_step(before.after(&tracker, 0), fold);
                            stats.unframeable_bytes =
                                stats.unframeable_bytes.saturating_add(skipped);
                            off = next;
                            continue;
                        }
                        FramingResync::NeedMoreData
                            if carry.len() - off <= MAX_RESYNC_WINDOW_BYTES =>
                        {
                            break;
                        }
                        FramingResync::NeedMoreData | FramingResync::Exhausted => {
                            let keep = COMPACT_RECORD_HEADER_LEN.saturating_sub(1);
                            let discard = (carry.len() - off).saturating_sub(keep);
                            if discard == 0 {
                                break;
                            }
                            let before = FoldDamageWatch::before(&tracker);
                            tracker.note_framing_damage(at, discard as u64, &message);
                            apply_fold_damage_step(before.after(&tracker, 0), fold);
                            stats.unframeable_bytes =
                                stats.unframeable_bytes.saturating_add(discard as u64);
                            off += discard;
                            continue;
                        }
                    }
                }
            };
            let record_len = header.record_len();
            if record_len > remaining {
                break;
            }
            let payload = &carry[off + COMPACT_RECORD_HEADER_LEN..off + record_len];

            let is_cid1_envelope = header.sequence == 0
                && header.domain_id
                    == compact_contract::campaign_identity::COMPACT_DOMAIN_STREAM_HEADER
                && header.record_type
                    == compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;
            if identity_mode == StreamIdentityMode::Legacy && is_cid1_envelope {
                let at = processed + off as u64;
                let error = validator
                    .validate_record(&header, payload)
                    .expect_err("Legacy validator must reject an IdentityV1 CID1 envelope");
                let message = format!("stream validation failed at byte {at}: {error}");
                if stats.stop_reason.is_none() {
                    stats.stop_reason = Some(message.clone());
                }
                let before = FoldDamageWatch::before(&tracker);
                tracker.note_framing_damage(at, record_len as u64, &message);
                apply_fold_damage_step(before.after(&tracker, header.game_hour_raw), fold);
                stats.unframeable_bytes = stats.unframeable_bytes.saturating_add(record_len as u64);
                off += record_len;
                continue;
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

            // PROVE, DON'T ADVANCE: the walker validates the reconstruction
            // through the fold's READ-ONLY probe and passes RAW ON-WIRE
            // bytes plus the ORIGINAL flag word to `ingest_record`, whose
            // internal apply performs the ONE true advance (Codex review
            // round 16, finding 2). An advancing walker-side reconstruct
            // would store grammar in the repeat baseline and refuse every
            // honest Delta-resolution repeat.
            let record_flags = header.record_flags;
            let reconstructed = if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
                match fold.probe_domain_data(
                    header.sequence,
                    header.game_hour_raw,
                    header.domain_id,
                    record_flags,
                    payload,
                ) {
                    Ok(reconstructed) => Ok(reconstructed),
                    Err(reason) => Err(reason),
                }
            } else {
                Ok(None)
            };
            let mut validation_flags = record_flags;
            if let Ok(Some(_)) = &reconstructed {
                validation_flags &= !compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA;
            }
            let reconstruction_failure = reconstructed.as_ref().err().cloned();
            let folded_payload = match &reconstructed {
                Ok(Some(bytes)) => bytes.as_slice(),
                _ => payload,
            };
            if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
                if let Err(reason) = reconstruction_failure.map(Err).unwrap_or_else(|| {
                    validate_domain_data_payload_at(
                        header.domain_id,
                        folded_payload,
                        validation_flags,
                        header.sequence,
                        header.game_hour_raw,
                        processed + off as u64,
                    )
                }) {
                    stats.records_total += 1;
                    stats.data_records += 1;
                    stats.malformed_payload_records += 1;
                    stats
                        .excluded_domain_data_records
                        .push(ExcludedDomainDataRecord {
                            stream_offset: processed + off as u64,
                            game_hour_raw: header.game_hour_raw,
                            domain_id: header.domain_id,
                            validation_reason: reason.clone(),
                        });
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
                    // The producer published this record and moved BOTH
                    // baselines with it; the reader must too, or a forged
                    // repeat naming an older envelope passes the stale raw
                    // digest and the un-advanced retained output (Codex
                    // review round 17, finding 1).
                    fold.note_rejected_domain_data(
                        header.sequence,
                        header.game_hour_raw,
                        header.domain_id,
                        header.record_flags,
                        payload,
                    );
                    let before = FoldDamageWatch::before(&tracker);
                    tracker.observe(&header, payload, processed + off as u64);
                    apply_fold_damage_step(before.after(&tracker, header.game_hour_raw), fold);
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
            match KnownRecordType::try_from(header.record_type) {
                Ok(KnownRecordType::DomainData | KnownRecordType::DomainDataRepeat) => {
                    stats.data_records += 1
                }
                Ok(KnownRecordType::FrameEnd) => stats.frame_end_records += 1,
                Ok(KnownRecordType::DomainPerf) => stats.perf_records += 1,
                Ok(KnownRecordType::FreezeCensus) => stats.freeze_census_records += 1,
                Ok(
                    KnownRecordType::StreamHeader
                    | KnownRecordType::ContentIdentityCensus
                    | KnownRecordType::StateTopologyCensus
                    | KnownRecordType::DomainGap
                    | KnownRecordType::FrameRefused
                    | KnownRecordType::FrameDegraded
                    | KnownRecordType::CaptureTerminated
                    | KnownRecordType::ClockUnreadableInterval
                    | KnownRecordType::UntrustedClockRecovery,
                ) => stats.other_records += 1,
                Err(_) => stats.other_records += 1,
            }
            // A refused repeat must reach THIS walker's exclusion ledger with
            // its exact reason, not only the fold's malformed-hour note
            // (Codex review round 10, finding 4): the read-only probe runs
            // every check `ingest_record` would, so a refusal here is THE
            // refusal, named once and counted where the loss is attributed.
            // The boundary tracker is told the record is TOLERATED so its
            // semantic rules do not latch a violation the walk has already
            // forgiven (Codex review round 16, finding 4).
            if header.record_type
                == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
            {
                // The resolved payload is grammar-validated EXACTLY like a
                // full record's: digest-valid resolution of malformed domain
                // bytes is still a lost hour (Codex review round 21).
                let outcome = fold
                    .verify_domain_data_repeat(
                        header.domain_id,
                        record_flags,
                        folded_payload,
                    )
                    .map_err(|error| {
                        format!("DOMAIN_DATA_REPEAT composition refused: {error}")
                    })
                    .and_then(|resolved| {
                        let validation_flags =
                            if record_flags & compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA
                                != 0
                            {
                                record_flags
                                    & !compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA
                            } else {
                                record_flags
                            };
                        validate_domain_data_payload_at(
                            header.domain_id,
                            &resolved,
                            validation_flags,
                            header.sequence,
                            header.game_hour_raw,
                            processed + off as u64,
                        )
                        .map_err(|error| {
                            format!("repeated payload failed domain validation: {error}")
                        })
                    });
                if let Err(reason) = outcome {
                    stats.malformed_payload_records += 1;
                    stats
                        .excluded_domain_data_records
                        .push(ExcludedDomainDataRecord {
                            stream_offset: processed + off as u64,
                            game_hour_raw: header.game_hour_raw,
                            domain_id: header.domain_id,
                            validation_reason: reason.clone(),
                        });
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
                    let before = FoldDamageWatch::before(&tracker);
                    tracker.observe_tolerated(&header, payload, processed + off as u64);
                    apply_fold_damage_step(before.after(&tracker, header.game_hour_raw), fold);
                    fold.refuse_domain_data_repeat(header.game_hour_raw, header.domain_id, &reason);
                    off += record_len;
                    continue;
                }
            }
            let before = FoldDamageWatch::before(&tracker);
            tracker.observe(&header, payload, processed + off as u64);
            apply_fold_damage_step(before.after(&tracker, header.game_hour_raw), fold);
            fold.ingest_record(
                header.sequence,
                header.game_hour_raw,
                header.domain_id,
                header.record_type,
                payload,
                record_flags,
            );
            off += record_len;
        }

        processed += off as u64;
        carry.drain(0..off);
        if hard_stop {
            if compressed {
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

    if total_len == 0 {
        stats.stream_bytes = decoded_read;
    }
    stats.bytes_folded = processed.saturating_sub(stats.unframeable_bytes);
    stats.truncated_tail_bytes = stats.stream_bytes.saturating_sub(processed);
    stats.hours_emitted = fold.emitted_hours_count() as u64;
    stats.ends_at_frame_end = tracker.eof_check(carry.len()).is_ok();
    stats.hour_coverage = tracker.seal_coverage(&carry);
    stats.decoded_sha256 = to_hex(&decoded_hasher.finalize());
    if compressed {
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
            compact_contract::frame_refusal::FrameRefusalRecordV2 {
                refused_hour: hour,
                offenders: vec![compact_contract::frame_refusal::FrameRefusalOffenderV2 {
                    domain_id: 64,
                    status_flags: 0x0000_0100,
                    non_tolerated_flags: 0x0000_0100,
                    predicate: compact_contract::diagnostic_channel::Predicate::new(
                        1,
                        "synthetic_domain_read_is_bound",
                    )
                    .unwrap(),
                    authority: compact_contract::diagnostic_channel::Authority::SelfDomain,
                    locator: Vec::new(),
                    units: Vec::new(),
                    truncation: compact_contract::diagnostic_channel::TruncationAccount::complete(
                        0,
                    ),
                    off_frame: None,
                    reason: "d64_rejected_timed_effect_frames=1".to_string(),
                    reason_units_named: 0,
                }],
                frame_findings: Vec::new(),
            }
            .encode()
            .expect("canonical refusal fixture")
        };

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

        assert!(
            tracker.eof_check(0).is_ok(),
            "a stream ending on a refusal ended at a closed boundary, got {:?}",
            tracker.eof_check(0)
        );

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

    #[test]
    fn an_untrusted_recovery_disclaims_hours_without_declaring_them_unobserved() {
        let recovery = |last_observed: i32, resumed: i32| {
            compact_contract::untrusted_clock_recovery::UntrustedClockRecoveryRecordV1 {
                last_observed_hour_raw: last_observed,
                resumed_hour_raw: resumed,
                unreadable_events: 2,
                observed_delta_hours: i64::from(resumed) - i64::from(last_observed),
                trust_ceiling_hours: 768,
            }
            .encode()
            .expect("canonical recovery fixture")
        };
        let observe_recovery =
            |tracker: &mut BoundaryTracker, seq: u64, resumed: i32, payload: &[u8]| {
                tracker.observe(
                    &boundary_header(
                        seq,
                        resumed,
                        COMPACT_DOMAIN_FRAME,
                        COMPACT_RECORD_TYPE_UNTRUSTED_CLOCK_RECOVERY,
                        payload.len(),
                        0,
                    ),
                    payload,
                    0,
                );
            };

        let base = raw_for(HOUR_A);
        let last_observed = base;
        let resumed = base + 800;

        let payload = recovery(last_observed, resumed);
        let mut clean = BoundaryTracker::default();
        observe_recovery(&mut clean, 1, resumed, &payload);
        assert!(clean.violation.is_none(), "{:?}", clean.violation);

        let refusal_hour = base + 300;
        let refusal = compact_contract::frame_refusal::FrameRefusalRecordV2 {
            refused_hour: refusal_hour,
            offenders: vec![compact_contract::frame_refusal::FrameRefusalOffenderV2 {
                domain_id: 64,
                status_flags: 0x0000_0100,
                non_tolerated_flags: 0x0000_0100,
                predicate: compact_contract::diagnostic_channel::Predicate::new(
                    1,
                    "synthetic_domain_read_is_bound",
                )
                .unwrap(),
                authority: compact_contract::diagnostic_channel::Authority::SelfDomain,
                locator: Vec::new(),
                units: Vec::new(),
                truncation: compact_contract::diagnostic_channel::TruncationAccount::complete(0),
                off_frame: None,
                reason: "d64".to_string(),
                reason_units_named: 0,
            }],
            frame_findings: Vec::new(),
        }
        .encode()
        .expect("canonical refusal fixture");
        let mut inside = BoundaryTracker::default();
        observe_recovery(&mut inside, 1, resumed, &payload);
        inside.observe(
            &boundary_header(
                2,
                refusal_hour,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_REFUSED,
                refusal.len(),
                0,
            ),
            &refusal,
            0,
        );
        assert!(
            inside.violation.is_none(),
            "a recovery declares no hour unobserved, so a later frame inside its span must \
             fold clean, got {:?}",
            inside.violation
        );

        let mut misframed = BoundaryTracker::default();
        observe_recovery(&mut misframed, 1, resumed + 1, &payload);
        assert!(
            misframed
                .violation
                .as_ref()
                .is_some_and(|v| v.contains("is framed for hour")),
            "a recovery framed for the wrong hour must be a violation, got {:?}",
            misframed.violation
        );

        let interval =
            compact_contract::clock_unreadable_interval::ClockUnreadableIntervalRecordV1 {
                range_start_hour_raw: base + 1,
                range_end_hour_raw: base + 4,
                last_observed_hour_raw: base,
                resumed_hour_raw: base + 5,
                unreadable_events: 1,
            }
            .encode()
            .expect("canonical interval fixture");
        let mut contradicted = BoundaryTracker::default();
        contradicted.observe(
            &boundary_header(
                1,
                base + 5,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_CLOCK_UNREADABLE_INTERVAL,
                interval.len(),
                0,
            ),
            &interval,
            0,
        );
        assert!(
            contradicted.violation.is_none(),
            "{:?}",
            contradicted.violation
        );
        observe_recovery(&mut contradicted, 2, resumed, &payload);
        assert!(
            contradicted
                .violation
                .as_ref()
                .is_some_and(|v| v.contains("already declared")),
            "a recovery may not disclaim a span an interval already vouched for, got {:?}",
            contradicted.violation
        );

        let mut reversed = BoundaryTracker::default();
        observe_recovery(&mut reversed, 1, resumed, &payload);
        assert!(reversed.violation.is_none(), "{:?}", reversed.violation);
        let inside_interval =
            compact_contract::clock_unreadable_interval::ClockUnreadableIntervalRecordV1 {
                range_start_hour_raw: base + 101,
                range_end_hour_raw: base + 104,
                last_observed_hour_raw: base + 100,
                resumed_hour_raw: base + 105,
                unreadable_events: 1,
            }
            .encode()
            .expect("canonical interval fixture");
        reversed.observe(
            &boundary_header(
                2,
                base + 105,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_CLOCK_UNREADABLE_INTERVAL,
                inside_interval.len(),
                0,
            ),
            &inside_interval,
            0,
        );
        assert!(
            reversed
                .violation
                .as_ref()
                .is_some_and(|v| v.contains("already disclaimed")),
            "an interval may not vouch for a span a recovery already disclaimed, got {:?}",
            reversed.violation
        );
    }

    #[test]
    fn a_declared_unobserved_interval_excludes_every_later_claim_on_its_hours() {
        let interval = |last_observed: i32, resumed: i32| {
            compact_contract::clock_unreadable_interval::ClockUnreadableIntervalRecordV1 {
                range_start_hour_raw: last_observed + 1,
                range_end_hour_raw: resumed - 1,
                last_observed_hour_raw: last_observed,
                resumed_hour_raw: resumed,
                unreadable_events: 2,
            }
            .encode()
            .expect("canonical interval fixture")
        };
        let observe_interval =
            |tracker: &mut BoundaryTracker, seq: u64, resumed: i32, payload: &[u8]| {
                tracker.observe(
                    &boundary_header(
                        seq,
                        resumed,
                        COMPACT_DOMAIN_FRAME,
                        COMPACT_RECORD_TYPE_CLOCK_UNREADABLE_INTERVAL,
                        payload.len(),
                        0,
                    ),
                    payload,
                    0,
                );
            };
        let base = raw_for(HOUR_A);
        let (last_observed, resumed) = (base, base + 5);

        let payload = interval(last_observed, resumed);
        let mut clean = BoundaryTracker::default();
        observe_interval(&mut clean, 1, resumed, &payload);
        assert!(clean.violation.is_none(), "{:?}", clean.violation);

        let refusal_hour = base + 3;
        let refusal = compact_contract::frame_refusal::FrameRefusalRecordV2 {
            refused_hour: refusal_hour,
            offenders: vec![compact_contract::frame_refusal::FrameRefusalOffenderV2 {
                domain_id: 64,
                status_flags: 0x0000_0100,
                non_tolerated_flags: 0x0000_0100,
                predicate: compact_contract::diagnostic_channel::Predicate::new(
                    1,
                    "synthetic_domain_read_is_bound",
                )
                .unwrap(),
                authority: compact_contract::diagnostic_channel::Authority::SelfDomain,
                locator: Vec::new(),
                units: Vec::new(),
                truncation: compact_contract::diagnostic_channel::TruncationAccount::complete(0),
                off_frame: None,
                reason: "d64".to_string(),
                reason_units_named: 0,
            }],
            frame_findings: Vec::new(),
        }
        .encode()
        .expect("canonical refusal fixture");
        let mut refused = BoundaryTracker::default();
        observe_interval(&mut refused, 1, resumed, &payload);
        refused.observe(
            &boundary_header(
                2,
                refusal_hour,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_REFUSED,
                refusal.len(),
                0,
            ),
            &refusal,
            0,
        );
        assert!(
            refused
                .violation
                .as_ref()
                .is_some_and(|v| v.contains("declared never observed")),
            "a refusal inside a declared-unobserved interval must be a violation, got {:?}",
            refused.violation
        );

        let mut overlapped = BoundaryTracker::default();
        observe_interval(&mut overlapped, 1, resumed, &payload);
        let second = interval(base + 2, base + 8);
        observe_interval(&mut overlapped, 2, base + 8, &second);
        assert!(
            overlapped
                .violation
                .as_ref()
                .is_some_and(|v| v.contains("overlapping the interval")),
            "two intervals claiming the same hours must be a violation, got {:?}",
            overlapped.violation
        );

        let mut disjoint = BoundaryTracker::default();
        observe_interval(&mut disjoint, 1, resumed, &payload);
        let later = interval(base + 6, base + 9);
        observe_interval(&mut disjoint, 2, base + 9, &later);
        assert!(disjoint.violation.is_none(), "{:?}", disjoint.violation);
    }

    #[test]
    fn a_probe_record_beside_a_d81_gap_is_not_a_duplicate_terminal_outcome() {
        const D81: u16 = 81;
        const PROBE: u16 = 250;
        let hour = raw_for(HOUR_A);
        let gap = compact_contract::DomainGapRecordV1 {
            part: compact_contract::DomainGapPart::State,
            reason: compact_contract::DomainGapReason::SourceUnavailable,
            target_hour_raw: hour,
            range_start_hour_raw: hour,
            range_end_hour_raw: hour,
            tick_before: hour as u64,
            tick_after: hour as u64,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        }
        .encode()
        .expect("canonical d81 gap fixture");

        let perf = [0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN];

        let mut admitted = BoundaryTracker::default();
        let mut seq = 1u64;
        for _ in 0..2 {
            admitted.observe(
                &boundary_header(
                    seq,
                    hour,
                    PROBE,
                    COMPACT_RECORD_TYPE_DOMAIN_PERF,
                    perf.len(),
                    0,
                ),
                &perf,
                0,
            );
            seq += 1;
        }
        admitted.observe(
            &boundary_header(seq, hour, D81, COMPACT_RECORD_TYPE_DOMAIN_GAP, gap.len(), 0),
            &gap,
            0,
        );
        assert!(
            admitted.violation.is_none(),
            "the probe carrier must not consume d81's one terminal outcome, got {:?}",
            admitted.violation
        );

        let mut rejected = BoundaryTracker::default();
        rejected.observe(
            &boundary_header(1, hour, D81, COMPACT_RECORD_TYPE_DOMAIN_PERF, perf.len(), 0),
            &perf,
            0,
        );
        rejected.observe(
            &boundary_header(2, hour, D81, COMPACT_RECORD_TYPE_DOMAIN_GAP, gap.len(), 0),
            &gap,
            0,
        );
        assert!(
            rejected
                .violation
                .as_ref()
                .is_some_and(|violation| violation.contains("duplicate terminal outcome")),
            "d81 PERF plus d81 GAP in one hour must still be refused, got {:?}",
            rejected.violation
        );
    }

    #[test]
    fn a_refusal_closes_the_open_hour_and_refuses_the_hour_it_is_framed_for() {
        let refusal_for = |hour: i32| {
            compact_contract::frame_refusal::FrameRefusalRecordV2 {
                refused_hour: hour,
                offenders: vec![compact_contract::frame_refusal::FrameRefusalOffenderV2 {
                    domain_id: 64,
                    status_flags: 0x0000_0100,
                    non_tolerated_flags: 0x0000_0100,
                    predicate: compact_contract::diagnostic_channel::Predicate::new(
                        1,
                        "synthetic_domain_read_is_bound",
                    )
                    .unwrap(),
                    authority: compact_contract::diagnostic_channel::Authority::SelfDomain,
                    locator: Vec::new(),
                    units: Vec::new(),
                    truncation: compact_contract::diagnostic_channel::TruncationAccount::complete(
                        0,
                    ),
                    off_frame: None,
                    reason: "d64_rejected_timed_effect_frames=1".to_string(),
                    reason_units_named: 0,
                }],
                frame_findings: Vec::new(),
            }
            .encode()
            .expect("canonical refusal fixture")
        };
        let open = raw_for(HOUR_A);
        let later = raw_for(HOUR_A + 1);

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

    /// Accepting the STREAM is not the same as decoding the PAYLOAD: a malformed
    /// body degrades its own domain and the fold keeps going, so the framing
    /// assertions alone pass on a payload the decoder REJECTED - such a record
    /// still lands with `data_records == 1`, `bytes_folded == stream.len()` and
    /// no stop reason. Every caller of this helper means "this production body
    /// is well formed", so the decode check lives HERE rather than being an
    /// optional extra at each call site: it used to be returned as stats and
    /// only the d14 caller looked, which left the other SIX call sites - three
    /// domains, d51, d53 and d69 - asserting nothing stronger than "the fold
    /// did not stop".
    ///
    /// kills - the production change each caller now catches that it did not
    /// before. What raises `malformed_payload_records` is any parse error out
    /// of `summarize_domain_data_payload`, which is TWO sources: the domain's
    /// own decoder, and `reject_unassigned_status_bits`, which the authority
    /// applies to every domain's status word after the decoder has run.
    /// - d14 / `d14_full_popularity_row_remains_boundary_valid`: a change to the
    ///   D14 32-byte fixed block's width or to the admissible
    ///   `election_frequency` set that leaves this fixture behind. This site
    ///   already asserted it; the assertion moved here unchanged.
    /// - d51 / `d51_and_d69_production_failure_payloads_pass_stream_boundary`:
    ///   a `scripted_state_domain51_gate::compact` encoder change - header
    ///   width or field order - that its own `decode_payload` can no longer
    ///   parse on the SOURCE_UNAVAILABLE / zero-row path, or a d51 status bit
    ///   published outside the assigned v1 mask and d51's registered lane-local
    ///   mask, which `reject_unassigned_status_bits` turns into a parse error.
    ///   Before, a d51 producer emitting a body its own decoder rejects passed
    ///   this test.
    /// - d69 / `d51_and_d69_production_failure_payloads_pass_stream_boundary`
    ///   and `d69_v2_signed_bonus_payload_passes_stream_boundary`: drift in the
    ///   TSG2 bonus payload that `encode_payload_with_bonuses` emits - the
    ///   24-byte header, the group/member count framing, or the 12-byte signed
    ///   member-bonus row - changed on the encoder side only. The fold reaches
    ///   it through `decode_payload_with_definition_ids`, whose TSG2 branch
    ///   delegates to `decode_payload_with_bonuses`; a mismatch there is a parse
    ///   error the old framing-only assertions ignored. The same assertion also
    ///   catches the policy rejections (group/member counts, member tag_db_id
    ///   ceiling) that decoder raises after a clean parse.
    /// - d53 / `d53_plain_delta_production_payload_passes_stream_boundary`,
    ///   `d53_plain_partial_production_payload_passes_stream_boundary`,
    ///   `d53_facs_v1_absent_production_payload_passes_stream_boundary`:
    ///   `diplomacy_intel_operatives_live::compact` encoder/decoder drift on the
    ///   DELTA, PARTIAL and absent-FACS shapes. The absent-FACS site is the
    ///   sharpest, and not because it is the shortest fixture - it is not; the
    ///   PARTIAL one has no rows at all. It is sharpest because it is the ONLY
    ///   site that pins a trailer's ABSENCE. FACS is a self-describing trailer
    ///   `append_faction_trailer` omits entirely when the roster is empty and
    ///   incomplete, and the site asserted only that the bytes contain no
    ///   `FACS`. That is half a claim: it says the encoder took the omit
    ///   branch, and says nothing about whether the payload it produced can
    ///   still be read. A decoder that starts requiring the trailer, or an
    ///   encoder whose omit branch also drops or mis-sizes something the
    ///   decoder does need, satisfied the old assertion and fails the new one.
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
        assert_eq!(
            stats.malformed_payload_records,
            0,
            "domain {domain}: the boundary-valid body must DECODE, not merely ride an accepted \
             stream; first decoder reason: {:?}",
            stats
                .malformed_payload_domains
                .get(&domain)
                .map(|malformed| malformed.first_reason.as_str())
        );
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

    fn walk_clean_hours(tracker: &mut BoundaryTracker, seq: &mut u64, hours: &[i64], at: &mut u64) {
        for hour_index in hours {
            let hour = raw_for(*hour_index);
            let data = country_identity_payload(&[(47, "GER")]);
            tracker.observe(
                &boundary_header(
                    *seq,
                    hour,
                    18,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    data.len(),
                    0,
                ),
                &data,
                *at,
            );
            *seq += 1;
            *at += (COMPACT_RECORD_HEADER_LEN + data.len()) as u64;
            let seal = frame_end_payload(0, 0, 0, 0);
            tracker.observe(
                &boundary_header(
                    *seq,
                    hour,
                    COMPACT_DOMAIN_FRAME,
                    COMPACT_RECORD_TYPE_FRAME_END,
                    seal.len(),
                    0,
                ),
                &seal,
                *at,
            );
            *seq += 1;
            *at += (COMPACT_RECORD_HEADER_LEN + seal.len()) as u64;
        }
    }

    #[test]
    fn an_injured_hour_is_excluded_and_named_while_its_neighbours_stay_verified() {
        let mut tracker = BoundaryTracker::default();
        let mut seq = 1u64;
        let mut at = 0u64;
        walk_clean_hours(&mut tracker, &mut seq, &[HOUR_A], &mut at);

        let hour_b = raw_for(HOUR_B);
        let data = country_identity_payload(&[(47, "GER")]);
        tracker.observe(
            &boundary_header(
                seq,
                hour_b,
                18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                data.len(),
                0,
            ),
            &data,
            at,
        );
        seq += 1;
        at += (COMPACT_RECORD_HEADER_LEN + data.len()) as u64;
        tracker.observe(
            &boundary_header(
                seq,
                raw_for(HOUR_C),
                18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                data.len(),
                0,
            ),
            &data,
            at,
        );
        seq += 1;
        at += (COMPACT_RECORD_HEADER_LEN + data.len()) as u64;
        let seal = frame_end_payload(0, 0, 0, 0);
        tracker.observe(
            &boundary_header(
                seq,
                hour_b,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                seal.len(),
                0,
            ),
            &seal,
            at,
        );
        seq += 1;
        at += (COMPACT_RECORD_HEADER_LEN + seal.len()) as u64;

        walk_clean_hours(&mut tracker, &mut seq, &[HOUR_C + 1], &mut at);

        let coverage = tracker.seal_coverage(&[]);
        coverage
            .validate()
            .expect("the map the seal publishes must be admissible");
        assert!(
            coverage.is_hour_verified(raw_for(HOUR_A)),
            "the hour BEFORE the injury is verified: {}",
            coverage.summary()
        );
        assert!(
            coverage.is_hour_verified(raw_for(HOUR_C + 1)),
            "the hour AFTER the injury is verified â€” the whole point of the ruling: {}",
            coverage.summary()
        );
        let excluded = coverage.excluded_hours();
        assert!(
            excluded
                .iter()
                .any(|(hour, reason)| *hour == hour_b && reason.contains("hour interleaving")),
            "the injured hour is excluded and NAMED, got {excluded:?}"
        );
        assert!(
            tracker
                .eof_check(0)
                .unwrap_err()
                .contains("hour interleaving"),
            "the latched whole-stream verdict must be untouched by the per-hour arm"
        );
        crate::hour_coverage::per_hour_seal_verdict(&coverage)
            .expect("a recording with verified hours seals");
    }

    #[test]
    fn a_sequence_hole_costs_the_missing_hours_and_the_one_it_resumed_in() {
        let mut tracker = BoundaryTracker::default();
        let mut seq = 1u64;
        let mut at = 0u64;
        walk_clean_hours(&mut tracker, &mut seq, &[HOUR_A], &mut at);
        seq += 2;
        walk_clean_hours(&mut tracker, &mut seq, &[HOUR_C, HOUR_C + 1], &mut at);

        let coverage = tracker.seal_coverage(&[]);
        coverage.validate().expect("admissible");
        assert!(coverage.is_hour_verified(raw_for(HOUR_A)));
        assert!(
            coverage.is_hour_verified(raw_for(HOUR_C + 1)),
            "the walk recovers: the next clean hour is verified, {}",
            coverage.summary()
        );
        let excluded = coverage.excluded_hours();
        assert_eq!(
            excluded.len(),
            2,
            "the missing hour and the hour the stream resumed in, got {excluded:?}"
        );
        assert_eq!(excluded[0].0, raw_for(HOUR_B));
        assert!(
            excluded[0].1.contains("never delivered")
                && excluded[0].1.contains("sequence discontinuity"),
            "the missing hour names why it is missing, got {:?}",
            excluded[0].1
        );
        assert_eq!(excluded[1].0, raw_for(HOUR_C));
        assert!(
            excluded[1].1.contains("may be missing records of its own"),
            "the resumed hour names what cannot be shown about it, got {:?}",
            excluded[1].1
        );
        assert_eq!(
            tracker.lost_between_frames().0,
            1,
            "a hole between frames is reported to the fold exactly once"
        );
    }

    #[test]
    fn a_refused_hour_is_excluded_by_name_rather_than_silently_absent() {
        let refusal = compact_contract::frame_refusal::FrameRefusalRecordV2 {
            refused_hour: raw_for(HOUR_B),
            offenders: vec![compact_contract::frame_refusal::FrameRefusalOffenderV2 {
                domain_id: 64,
                status_flags: 0x0000_0100,
                non_tolerated_flags: 0x0000_0100,
                predicate: compact_contract::diagnostic_channel::Predicate::new(
                    1,
                    "synthetic_domain_read_is_bound",
                )
                .unwrap(),
                authority: compact_contract::diagnostic_channel::Authority::SelfDomain,
                locator: Vec::new(),
                units: Vec::new(),
                truncation: compact_contract::diagnostic_channel::TruncationAccount::complete(0),
                off_frame: None,
                reason: "d64_rejected_timed_effect_frames=1".to_string(),
                reason_units_named: 0,
            }],
            frame_findings: Vec::new(),
        }
        .encode()
        .expect("canonical refusal fixture");

        let mut tracker = BoundaryTracker::default();
        let mut seq = 1u64;
        let mut at = 0u64;
        walk_clean_hours(&mut tracker, &mut seq, &[HOUR_A], &mut at);
        let data = country_identity_payload(&[(47, "GER")]);
        tracker.observe(
            &boundary_header(
                seq,
                raw_for(HOUR_B),
                18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                data.len(),
                0,
            ),
            &data,
            at,
        );
        seq += 1;
        at += (COMPACT_RECORD_HEADER_LEN + data.len()) as u64;
        tracker.observe(
            &boundary_header(
                seq,
                raw_for(HOUR_B),
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_REFUSED,
                refusal.len(),
                0,
            ),
            &refusal,
            at,
        );
        seq += 1;
        at += (COMPACT_RECORD_HEADER_LEN + refusal.len()) as u64;
        walk_clean_hours(&mut tracker, &mut seq, &[HOUR_C], &mut at);

        let coverage = tracker.seal_coverage(&[]);
        assert_eq!(
            tracker.eof_check(0),
            Ok(()),
            "a refusal stream is honest and its whole-stream verdict stays clean"
        );
        let excluded = coverage.excluded_hours();
        assert_eq!(excluded.len(), 1, "got {excluded:?}");
        assert_eq!(excluded[0].0, raw_for(HOUR_B));
        assert!(
            excluded[0].1.contains("refused this frame"),
            "the refused hour says it was refused, got {:?}",
            excluded[0].1
        );
        assert_eq!(coverage.verified_count(), 2);
    }

    #[test]
    fn a_torn_tail_names_the_game_hour_whose_records_it_interrupted() {
        let mut tracker = BoundaryTracker::default();
        let mut seq = 1u64;
        let mut at = 0u64;
        walk_clean_hours(&mut tracker, &mut seq, &[HOUR_A], &mut at);

        let mut residual = Vec::new();
        residual.extend_from_slice(COMPACT_RECORD_MAGIC);
        residual.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        residual.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        residual.extend_from_slice(&seq.to_le_bytes());
        residual.extend_from_slice(&raw_for(HOUR_B).to_le_bytes());
        residual.extend_from_slice(&18u16.to_le_bytes());
        residual.extend_from_slice(&COMPACT_RECORD_TYPE_DOMAIN_DATA.to_le_bytes());
        residual.extend_from_slice(&40u32.to_le_bytes());
        residual.extend_from_slice(&0u32.to_le_bytes());
        residual.extend_from_slice(&[0u8; 3]);

        let coverage = tracker.seal_coverage(&residual);
        coverage.validate().expect("admissible");
        assert!(coverage.is_hour_verified(raw_for(HOUR_A)));
        let excluded = coverage.excluded_hours();
        assert_eq!(excluded.len(), 1, "got {excluded:?}");
        assert_eq!(excluded[0].0, raw_for(HOUR_B));
        assert!(
            excluded[0].1.contains("ends mid-record inside game hour"),
            "got {:?}",
            excluded[0].1
        );
    }

    #[test]
    fn a_framing_resync_refuses_a_record_magic_that_is_really_payload_bytes() {
        let seal = frame_end_payload(0, 0, 0, 0);
        let mut real = Vec::new();
        real.extend_from_slice(COMPACT_RECORD_MAGIC);
        real.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        real.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        real.extend_from_slice(&7u64.to_le_bytes());
        real.extend_from_slice(&raw_for(HOUR_A).to_le_bytes());
        real.extend_from_slice(&COMPACT_DOMAIN_FRAME.to_le_bytes());
        real.extend_from_slice(&COMPACT_RECORD_TYPE_FRAME_END.to_le_bytes());
        real.extend_from_slice(&(seal.len() as u32).to_le_bytes());
        real.extend_from_slice(&0u32.to_le_bytes());
        real.extend_from_slice(&seal);

        let mut decoy = Vec::new();
        decoy.extend_from_slice(COMPACT_RECORD_MAGIC);
        decoy.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        decoy.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        decoy.extend_from_slice(&99u64.to_le_bytes());
        decoy.extend_from_slice(&raw_for(HOUR_A).to_le_bytes());
        decoy.extend_from_slice(&18u16.to_le_bytes());
        decoy.extend_from_slice(&COMPACT_RECORD_TYPE_DOMAIN_DATA.to_le_bytes());
        decoy.extend_from_slice(&8u32.to_le_bytes());
        decoy.extend_from_slice(&0u32.to_le_bytes());
        decoy.extend_from_slice(&[0u8; 8]);

        let mut buf = vec![0xAAu8; 16];
        let decoy_at = buf.len();
        buf.extend_from_slice(&decoy);
        buf.extend_from_slice(&[0xBB, 0xBB]);
        let real_at = buf.len();
        buf.extend_from_slice(&real);

        match find_frame_resync_offset(&buf, 0) {
            FramingResync::At(offset) => {
                assert_ne!(
                    offset, decoy_at,
                    "the decoy header parses but its record lands on nothing â€” it must be rejected"
                );
                assert_eq!(offset, real_at, "the resync must land on the real record");
            }
            other => panic!("expected a resync onto the real record, got {other:?}"),
        }
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
        // The header's fixed-point scale, which the decoder requires to equal
        // the contract's. A zero here is a parse error on its own.
        payload[20..24]
            .copy_from_slice(&crate::domain_data_authority::COMPACT_FIXED_SCALE_RAW.to_le_bytes());
        payload[24..28].copy_from_slice(&1u32.to_le_bytes());
        // The 32-byte fixed block, spelled word by word: tag_db_id, flags,
        // idea_count, election_frequency, party_count, low_confidence,
        // leader.char_id, capital_state. It was 4 + a 24-byte filler when the
        // D14 row was 28 bytes wide; the E28 capital word made it 32 and this
        // fixture kept the old width. The stream still accepted, because a
        // malformed body degrades its own domain rather than stopping the fold -
        // which is why `expect_domain_boundary_acceptance` asserts the body
        // DECODED, the assertion that catches both the stale width and the two
        // zero words below.
        //
        // `election_frequency` is `u32::MAX` because zero is not a legal
        // measured value: the decoder admits either the unmeasured marker or
        // 1..=600, and a zero-filled block quietly fails that.
        for value in [1u32, 0, 0, u32::MAX, 0, 0, 0, 0] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        for _ in 0..4 {
            payload.extend_from_slice(&0u16.to_le_bytes());
        }
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.push(b'd');
        payload.extend_from_slice(&50_000i32.to_le_bytes());
        // The decode assertion this test used to spell out itself now lives
        // inside `expect_domain_boundary_acceptance`, where it polices every
        // domain that calls the helper rather than only this one.
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

        // d53 does not cross-check the enclosing header, so the identity
        // arguments are free here; they are spelled out rather than defaulted
        // so no caller can inherit a placeholder by accident.
        validate_domain_data_payload_at(53, &delta, 0, 0, 0, 0)
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
                &[0u8; 32],
            );
            seq += 1;
        }
        s
    }

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

    #[test]
    fn compressed_stream_ending_mid_record_hard_errors() {
        let mut stream = three_hour_stream();
        stream.extend_from_slice(COMPACT_RECORD_MAGIC);
        stream.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        stream.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        stream.extend_from_slice(&7u64.to_le_bytes());
        let mut fold = WorldFold::new("gid");
        let err =
            drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, true).unwrap_err();
        assert!(err.to_string().contains("ends mid-record"), "{err}");

        let mut fold = WorldFold::new("gid");
        let stats =
            drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, false).unwrap();
        assert_eq!(stats.truncated_tail_bytes, 16);
        assert!(!stats.ends_at_frame_end);
    }

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

        let mut fold = WorldFold::new("gid");
        let stats =
            drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, false).unwrap();
        assert!(!stats.ends_at_frame_end);
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
    fn seal_time_boundary_accounts_for_malformed_domain_data() {
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
        let excluded = verify_stream_logical_boundary(&path)
            .expect("a framed, closed stream remains sealable");
        assert_eq!(excluded.len(), 1);
        assert_eq!(excluded[0].stream_offset, 0);
        assert_eq!(excluded[0].game_hour_raw, raw_for(HOUR_A));
        assert_eq!(excluded[0].domain_id, 18);
        assert!(!excluded[0].validation_reason.is_empty());
    }

    #[test]
    fn structural_certification_classes_remain_fatal() {
        let framing =
            strict_stream_certification_verdict(Some("bad H4CR magic at byte 64"), 0, true, None)
                .expect_err("a framing stop must refuse sealing");
        assert!(framing.contains("framing stopped"), "{framing}");

        let tail = strict_stream_certification_verdict(None, 7, true, None)
            .expect_err("a truncated/corrupt tail must refuse sealing");
        assert!(tail.contains("truncated/corrupt tail"), "{tail}");

        let boundary = strict_stream_certification_verdict(
            None,
            0,
            false,
            Some("game hour 42 left open at EOF"),
        )
        .expect_err("an unclosed boundary must refuse sealing");
        assert!(
            boundary.contains("stream boundary is not closed"),
            "{boundary}"
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
        stream.extend_from_slice(&0x2000_0000u32.to_le_bytes());
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

    #[test]
    fn truncated_tail_emits_complete_hours_only() {
        let dir = TmpDir::new("trunc");
        let mut stream = three_hour_stream();
        stream.truncate(stream.len() - 5);
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

    #[test]
    fn header_corruption_in_compressed_stream_fails_fold_loudly() {
        let dir = TmpDir::new("zstd_hdr_flip");
        let clean = three_hour_stream();
        let mut corrupted = clean.clone();
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

        let err = match fold_compact_stream_file(&zstd_path, "gid", &dir.0.join("zs")) {
            Ok(_) => panic!("compressed header corruption must fail the fold"),
            Err(err) => err,
        };
        assert_compressed_protocol_hard_error(&err, "invalid record magic");

        let (files, stats) =
            fold_compact_stream_file(&raw_path, "gid", &dir.0.join("raws")).unwrap();
        assert!(stats.stop_reason.is_some(), "raw stop keeps its diagnostic");
        assert!(
            !files.is_empty(),
            "the hours after the damage are still delivered: {}",
            stats.hour_coverage.summary()
        );
        let excluded = stats.hour_coverage.excluded_hours();
        assert!(
            excluded
                .iter()
                .any(|(_, reason)| reason.contains("framing damage")),
            "the damaged hour is excluded and NAMED, got {excluded:?}"
        );
        assert_eq!(
            stats.hour_coverage.verified_count(),
            0,
            "nothing read after framing damage may be vouched for: {}",
            stats.hour_coverage.summary()
        );
        assert!(
            crate::hour_coverage::per_hour_seal_verdict(&stats.hour_coverage)
                .unwrap_err()
                .contains("nothing to seal"),
            "a recording with no verified hour has nothing to seal"
        );
    }

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
            let hours: Vec<i64> = par_files.iter().map(|f| f.hour_index).collect();
            assert_eq!(hours, vec![HOUR_A, HOUR_B, HOUR_C]);
        }
    }

    #[test]
    fn boundary_tracker_reports_typed_abnormal_termination_without_closing_an_hour() {
        let reasons = compact_contract::capture_termination::CaptureTerminationReasons::from_bits(
            compact_contract::capture_termination::CAPTURE_TERMINATION_REASON_CAPTURE_LOOP_ABORTED,
        )
        .unwrap();
        let payload =
            compact_contract::capture_termination::CaptureTerminationRecordV1::encode_bounded(
                reasons,
                1_900,
                "H4FR encoder rejected its detail",
            );
        let mut tracker = BoundaryTracker::default();
        tracker.observe(
            &boundary_header(
                1,
                0,
                compact_contract::COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_CAPTURE_TERMINATED,
                payload.len(),
                0,
            ),
            &payload,
            0,
        );
        let error = tracker.eof_check(0).unwrap_err();
        assert!(error.contains("capture terminated abnormally"), "{error}");
        assert!(error.contains("capture_loop_aborted"), "{error}");
        assert!(
            error.contains("H4FR encoder rejected its detail"),
            "{error}"
        );
    }

    fn one_freeze_census_payload(
        phase: compact_contract::freeze_census_record::FreezeCensusPhase,
    ) -> Vec<u8> {
        use compact_contract::freeze_census_record::{
            FreezeCensusCounterSnapshot, FreezeCensusEntryPoint, FreezeCensusRecordV1,
            FreezeCensusRowRef,
        };
        let before = FreezeCensusCounterSnapshot {
            process_attempts: 10,
            producer_thread_attempts: 7,
            process_refusals: 2,
            census_recorded: 10,
            census_dropped: 0,
        };
        let after = FreezeCensusCounterSnapshot {
            process_attempts: 11,
            producer_thread_attempts: 8,
            process_refusals: 2,
            census_recorded: 11,
            census_dropped: 0,
        };
        FreezeCensusRecordV1::encode_bounded(
            phase,
            before,
            after,
            &[FreezeCensusRowRef {
                ordinal: 11,
                refused: false,
                entry_point: Some(FreezeCensusEntryPoint::EnableBatch),
                label: "production hooks",
                target_count: 4,
                window_us: 90,
                acquire_us: 20,
                minhook_us: 30,
                passes: 1,
            }],
        )
        .unwrap()
    }

    #[test]
    fn freeze_census_does_not_open_or_extend_world_hour_coverage() {
        let payload = one_freeze_census_payload(
            compact_contract::freeze_census_record::FreezeCensusPhase::Install,
        );
        let mut tracker = BoundaryTracker::default();
        let census_header = boundary_header(
            1,
            0,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FREEZE_CENSUS,
            payload.len(),
            0,
        );
        tracker.absorb_into_hour(&census_header, &payload, 0);
        tracker.observe(&census_header, &payload, 0);
        assert!(tracker.hour_span.is_none());

        let first_world_byte = (COMPACT_RECORD_HEADER_LEN + payload.len()) as u64;
        let data = country_identity_payload(&[(47, "GER")]);
        let data_header = boundary_header(
            2,
            raw_for(HOUR_A),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            data.len(),
            0,
        );
        tracker.absorb_into_hour(&data_header, &data, first_world_byte);
        assert_eq!(
            tracker.hour_span.as_ref().unwrap().first_byte,
            first_world_byte
        );
    }

    /// One release-phase H4FC payload, bounded and self-consistent.
    fn release_census_payload() -> Vec<u8> {
        use compact_contract::freeze_census_record::{
            FreezeCensusCounterSnapshot, FreezeCensusPhase, FreezeCensusRecordV1,
        };
        FreezeCensusRecordV1::encode_bounded(
            FreezeCensusPhase::Release,
            FreezeCensusCounterSnapshot::default(),
            FreezeCensusCounterSnapshot::default(),
            &[],
        )
        .unwrap()
    }

    fn release_census_header(sequence: u64, payload_len: usize) -> CompactRecordHeader {
        boundary_header(
            sequence,
            0,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FREEZE_CENSUS,
            payload_len,
            0,
        )
    }

    fn tracker_at_a_closed_hour() -> (BoundaryTracker, u64, u64) {
        let mut tracker = BoundaryTracker::default();
        let mut sequence = 1u64;
        let mut at = 0u64;
        walk_clean_hours(&mut tracker, &mut sequence, &[1], &mut at);
        (tracker, sequence, at)
    }

    #[test]
    fn the_fold_accepts_one_release_epilogue_and_it_opens_no_hour() {
        let (mut tracker, sequence, at) = tracker_at_a_closed_hour();
        let closed_before = tracker.last_closed_hour;
        let payload = release_census_payload();
        tracker.observe(
            &release_census_header(sequence, payload.len()),
            &payload,
            at,
        );
        assert!(tracker.violation.is_none(), "{:?}", tracker.violation);
        assert_eq!(tracker.eof_check(0), Ok(()));
        assert_eq!(tracker.open_hour, None);
        assert_eq!(
            tracker.last_closed_hour, closed_before,
            "the epilogue must not close, open, or move a game hour"
        );
    }

    #[test]
    fn the_fold_refuses_every_other_shape_after_the_final_boundary() {
        let payload = release_census_payload();

        let (mut two, sequence, at) = tracker_at_a_closed_hour();
        two.observe(
            &release_census_header(sequence, payload.len()),
            &payload,
            at,
        );
        two.observe(
            &release_census_header(sequence + 1, payload.len()),
            &payload,
            at + 32,
        );
        assert!(two.violation.is_some(), "a second epilogue was accepted");

        let (mut then_data, sequence, at) = tracker_at_a_closed_hour();
        then_data.observe(
            &release_census_header(sequence, payload.len()),
            &payload,
            at,
        );
        observe_zero_payload(
            &mut then_data,
            boundary_header(
                sequence + 1,
                raw_for(2),
                18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                0,
            ),
            at + 32,
        );
        assert!(
            then_data.violation.is_some(),
            "a record after the epilogue was accepted"
        );

        let mut no_boundary = BoundaryTracker::default();
        no_boundary.observe(&release_census_header(1, payload.len()), &payload, 0);
        assert!(
            no_boundary.violation.is_some(),
            "an epilogue with no preceding closed hour was accepted"
        );

        let mut after_data = BoundaryTracker::default();
        observe_zero_payload(
            &mut after_data,
            boundary_header(1, raw_for(1), 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, 0),
            0,
        );
        after_data.observe(&release_census_header(2, payload.len()), &payload, 32);
        assert!(
            after_data.violation.is_some(),
            "an epilogue behind a DOMAIN_DATA record was accepted"
        );

        let (mut hour_bearing, sequence, at) = tracker_at_a_closed_hour();
        let mut header = release_census_header(sequence, payload.len());
        header.game_hour_raw = raw_for(1);
        hour_bearing.observe(&header, &payload, at);
        assert!(
            hour_bearing.violation.is_some(),
            "an epilogue carrying a game hour was accepted"
        );

        let (mut oversize, sequence, at) = tracker_at_a_closed_hour();
        let mut big = payload.clone();
        big.resize(
            compact_contract::freeze_census_record::FREEZE_CENSUS_MAX_PAYLOAD_LEN + 1,
            0,
        );
        oversize.observe(&release_census_header(sequence, big.len()), &big, at);
        assert!(
            oversize.violation.is_some(),
            "an oversize epilogue was accepted"
        );
    }

    /// `boundary_eof_policy` with `release_freeze_census_seen` never set,
    #[test]
    fn an_unchanged_reader_refuses_an_epilogue_carrying_recording() {
        let (mut tracker, sequence, at) = tracker_at_a_closed_hour();
        let payload = release_census_payload();
        tracker.observe(
            &release_census_header(sequence, payload.len()),
            &payload,
            at,
        );
        assert_eq!(tracker.eof_check(0), Ok(()));
        assert_eq!(
            tracker.last_record_kind,
            Some(COMPACT_RECORD_TYPE_FREEZE_CENSUS)
        );
        assert!(!boundary_eof_policy(tracker.last_record_kind, false));
    }

    fn install_census_payload() -> Vec<u8> {
        use compact_contract::freeze_census_record::{
            FreezeCensusCounterSnapshot, FreezeCensusPhase, FreezeCensusRecordV1,
        };
        FreezeCensusRecordV1::encode_bounded(
            FreezeCensusPhase::Install,
            FreezeCensusCounterSnapshot::default(),
            FreezeCensusCounterSnapshot::default(),
            &[],
        )
        .unwrap()
    }

    fn install_census_header(sequence: u64, payload_len: usize) -> CompactRecordHeader {
        boundary_header(
            sequence,
            0,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FREEZE_CENSUS,
            payload_len,
            0,
        )
    }

    fn serialize_records(records: &[(CompactRecordHeader, Vec<u8>)]) -> Vec<u8> {
        let mut raw = Vec::new();
        for (header, payload) in records {
            raw.extend_from_slice(COMPACT_RECORD_MAGIC);
            raw.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
            raw.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
            raw.extend_from_slice(&header.sequence.to_le_bytes());
            raw.extend_from_slice(&header.game_hour_raw.to_le_bytes());
            raw.extend_from_slice(&header.domain_id.to_le_bytes());
            raw.extend_from_slice(&header.record_type.to_le_bytes());
            raw.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            raw.extend_from_slice(&header.record_flags.to_le_bytes());
            raw.extend_from_slice(payload);
        }
        raw
    }

    /// record at sequence 1, then one ordinary hour, then the release epilogue
    fn legacy_capture_records(with_epilogue: bool) -> Vec<(CompactRecordHeader, Vec<u8>)> {
        let install = install_census_payload();
        let hour = raw_for(HOUR_A);
        let data = country_identity_payload(&[(47, "GER")]);
        let seal = frame_end_payload(0, 0, 0, 0);
        let mut records = vec![
            (install_census_header(1, install.len()), install),
            (
                boundary_header(2, hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, data.len(), 0),
                data,
            ),
            (
                boundary_header(
                    3,
                    hour,
                    COMPACT_DOMAIN_FRAME,
                    COMPACT_RECORD_TYPE_FRAME_END,
                    seal.len(),
                    0,
                ),
                seal,
            ),
        ];
        if with_epilogue {
            let release = release_census_payload();
            records.push((release_census_header(4, release.len()), release));
        }
        records
    }

    fn both_verdicts(records: &[(CompactRecordHeader, Vec<u8>)]) -> (Result<(), String>, bool) {
        let mut tracker = BoundaryTracker::default();
        let mut at = 0u64;
        for (header, payload) in records {
            tracker.observe(header, payload, at);
            at += (COMPACT_RECORD_HEADER_LEN + payload.len()) as u64;
        }
        let fold = tracker.eof_check(0);
        let scanner = compact_contract::stream_validator::validate_freeze_census_stream_placement(
            &serialize_records(records),
        );
        (fold, scanner.is_ok())
    }

    #[test]
    fn the_fold_and_the_secondary_scanners_agree_on_a_legacy_capture() {
        for with_epilogue in [false, true] {
            let (fold, scanner) = both_verdicts(&legacy_capture_records(with_epilogue));
            assert_eq!(fold, Ok(()), "fold refused a legacy capture");
            assert!(scanner, "secondary scanners refused a legacy capture");
        }

        let mut identity = legacy_capture_records(true);
        let cid1 = {
            let observation = Cid1Observation {
                game_unique_id: "a4b17fed-ed12-4361-9064-fb6576638110".to_owned(),
                game_unique_seed: -947_102,
                game_hour_raw: raw_for(HOUR_A),
            };
            let fields = Cid1Fields::from_stable_observations(
                observation.clone(),
                observation,
                "vanilla_latest_2026_07_15".to_owned(),
                Cid1Witnesses::default(),
            )
            .unwrap();
            encode_cid1(&fields).unwrap()
        };
        identity.insert(
            0,
            (
                boundary_header(
                    0,
                    raw_for(HOUR_A),
                    COMPACT_DOMAIN_STREAM_HEADER,
                    COMPACT_RECORD_TYPE_STREAM_HEADER,
                    cid1.len(),
                    0,
                ),
                cid1,
            ),
        );
        let (_, scanner) = both_verdicts(&identity);
        assert!(scanner, "secondary scanners refused an identity capture");

        let install = install_census_payload();
        let hour = raw_for(HOUR_A);
        let data = country_identity_payload(&[(47, "GER")]);
        let seal = frame_end_payload(0, 0, 0, 0);
        let misplaced = vec![
            (
                boundary_header(1, hour, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, data.len(), 0),
                data,
            ),
            (install_census_header(2, install.len()), install),
            (
                boundary_header(
                    3,
                    hour,
                    COMPACT_DOMAIN_FRAME,
                    COMPACT_RECORD_TYPE_FRAME_END,
                    seal.len(),
                    0,
                ),
                seal,
            ),
        ];
        let (fold, scanner) = both_verdicts(&misplaced);
        assert!(fold.is_err(), "fold admitted a misplaced install census");
        assert!(
            !scanner,
            "secondary scanners admitted a misplaced install census"
        );
    }

    /// authority having ACCEPTED it as the release epilogue -- i.e. treating
    #[test]
    fn only_an_accepted_release_epilogue_may_close_a_stream_after_the_boundary() {
        assert!(boundary_eof_policy(
            Some(COMPACT_RECORD_TYPE_FREEZE_CENSUS),
            true,
        ));
        assert!(!boundary_eof_policy(
            Some(COMPACT_RECORD_TYPE_FREEZE_CENSUS),
            false,
        ));
        assert!(boundary_eof_policy(
            Some(COMPACT_RECORD_TYPE_FRAME_END),
            false
        ));
        assert!(boundary_eof_policy(
            Some(COMPACT_RECORD_TYPE_FRAME_REFUSED),
            false
        ));
        for kind in [
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            COMPACT_RECORD_TYPE_DOMAIN_GAP,
            COMPACT_RECORD_TYPE_FRAME_DEGRADED,
            COMPACT_RECORD_TYPE_CAPTURE_TERMINATED,
        ] {
            assert!(
                !boundary_eof_policy(Some(kind), true),
                "record type {kind} was allowed to close a stream"
            );
        }
        assert!(!boundary_eof_policy(None, true));
    }

    /// The enclosing H4CR header identity must reach the payload validator.
    ///
    /// d57 cross-checks its payload hour against the record header's hour, so a
    /// caller that substitutes a placeholder does not skip the check â€” it runs
    /// it against the placeholder and rejects a perfectly good record. Lane OB's
    /// live capture hit exactly that: all 366 d57 records were reported
    /// `d57 payload hour <hour> != H4CR hour 0`, which failed the seal gate with
    /// `d57_record_count_per_closed_frame_invalid` and downgraded the recording
    /// to a recovery-only replay the viewer refuses to open.
    ///
    /// kills: reintroducing a header-less wrapper that defaults the hour to 0,
    /// or any caller passing a placeholder instead of `header.game_hour_raw`.
    #[test]
    fn domain_data_validation_uses_the_real_header_hour_not_a_placeholder() {
        use session_multiplayer_metadata_live as session57;

        let hour = 60_832_056i32;
        let frame = session57::SessionMetadataFrame {
            game_hour_raw: hour,
            status_flags: 0,
            source_flags: 0,
            schema_flags: 0,
            root_evidence_mask: 0,
            world_tension_fixed5: None,
            world_tension_details: None,
            player_country_rows: Vec::new(),
            event_rows: Vec::new(),
        };
        let payload = session57::encode_payload(&frame);

        validate_domain_data_payload_at(session57::DOMAIN_ID, &payload, 0, 7, hour, 4_096).expect(
            "a d57 payload whose hour equals its record header hour must pass validation",
        );

        let error = validate_domain_data_payload_at(session57::DOMAIN_ID, &payload, 0, 7, 0, 4_096)
            .expect_err("a placeholder header hour must not silently validate");
        assert!(
            error.contains(&format!("d57 payload hour {hour} != H4CR hour 0")),
            "the placeholder must be reported as the mismatch it is, got: {error}"
        );
    }

    /// A d57 payload that carries its `WT3X` world-tension trailer is LONGER
    /// than its base sections, and the host must admit it.
    ///
    /// The host authority decoder derived the expected payload length from the
    /// four base sections alone and demanded exact equality, so every payload
    /// with a trailer was excluded as malformed. Capture rc_vanilla5 put 692 d57
    /// records on the wire and all 692 carried a trailer, so `domain57_frames`
    /// decoded EMPTY, the seal gate saw zero records against 739 closed frames,
    /// and the recording published as `native_session.failed.battleplan`. Sweep
    /// and raw evidence: `runs/si_d57_frame_gate/`.
    ///
    /// The rule now lives once, in `session57::payload_total_length_error`.
    ///
    /// kills: restoring `base_len == payload.len()` in
    /// `decode_compact_session_multiplayer_metadata_live_data_payload`, or any
    /// length check there that does not consult the declared trailer.
    #[test]
    fn a_d57_payload_carrying_its_world_tension_trailer_is_admitted() {
        use session_multiplayer_metadata_live as session57;

        let hour = 60_832_553i32;
        let details = session57::WorldTensionDetails {
            current_fixed5: 4_200,
            prior_fixed5: 4_100,
            increased: true,
            monthly_decrease_fixed5: 100,
            time_scale_fixed5: 1_000,
            start_date_raw: 43_800_000,
            country_rows: vec![session57::WorldTensionCountryRow {
                tag_db_id: 1,
                flags: session57::WORLD_TENSION_COUNTRY_FLAG_POLICY_CAP_ENABLED,
                generated_threat_fixed: 14_267_212,
                // The exact shape rc_vanilla5 carries: the game ideas and rules push
                // these thresholds below zero.
                generate_wargoal_threshold_fixed5: -25_000,
                join_faction_threshold_fixed5: -10_000,
                lend_lease_threshold_fixed5: 50_000,
                send_volunteers_threshold_fixed5: 0,
                guarantee_threshold_fixed5: 0,
            }],
            ledger_rows: Vec::new(),
        };
        let frame = session57::SessionMetadataFrame {
            game_hour_raw: hour,
            status_flags: 0,
            source_flags: session57::SOURCE_FLAG_DLL_NATIVE
                | session57::SOURCE_FLAG_SESSION_ROOT_VALIDATED,
            schema_flags: 0,
            root_evidence_mask: session57::ROOT_EVIDENCE_WORLD_TENSION,
            world_tension_fixed5: Some(4_200),
            world_tension_details: Some(details),
            player_country_rows: Vec::new(),
            event_rows: Vec::new(),
        };
        let payload = session57::encode_payload(&frame);

        // The base sections alone are the 64-byte header: no player rows, no
        // event rows, no strings. So the trailer is the whole rest of the
        // payload, and the old rule rejected it on length alone.
        let base_len = session57::PAYLOAD_HEADER_LEN;
        assert!(
            payload.len() > base_len,
            "the fixture must actually carry a trailer, got {} bytes",
            payload.len()
        );

        validate_domain_data_payload_at(session57::DOMAIN_ID, &payload, 0, 7, hour, 4_096).expect(
            "a d57 payload carrying a well-formed WT3X trailer must pass host validation",
        );

        // The trailer is admitted, not waved through: corrupting its magic while
        // the header still declares it must be refused.
        let mut corrupted = payload.clone();
        corrupted[base_len] ^= 0xFF;
        let error =
            validate_domain_data_payload_at(session57::DOMAIN_ID, &corrupted, 0, 7, hour, 4_096)
                .expect_err("a declared trailer with a broken magic must not validate");
        assert!(
            error.contains("world-tension extension"),
            "the trailer must be named as the fault, got: {error}"
        );

        // And a payload with trailing bytes it does NOT declare is still refused
        // on length, exactly as before.
        let mut undeclared = session57::encode_payload(&session57::SessionMetadataFrame {
            world_tension_details: None,
            world_tension_fixed5: None,
            root_evidence_mask: 0,
            ..frame
        });
        undeclared.push(0);
        let error =
            validate_domain_data_payload_at(session57::DOMAIN_ID, &undeclared, 0, 7, hour, 4_096)
                .expect_err("undeclared trailing bytes must still be a length fault");
        assert!(
            error.contains("incompatible with base length"),
            "undeclared trailing bytes must be reported as a length fault, got: {error}"
        );
    }
// -+
// DOMAIN_DATA_REPEAT and the per-hour seal.
// -------------------------------------------------------------------------

/// THE SEAL QUESTION THE REPEAT GATE WAS APPROVED ON: an hour whose data is a
/// DOMAIN_DATA_REPEAT record is COVERED, exactly as a full-record hour is â€”
/// the repeat is a data record of a new kind, not a gap.
///
/// kills: counting repeats as "other" records would leave hour B with no data
/// record in the completeness ledger (`data_records == 1`), and treating the
/// unknown type as a violation would exclude hour B from `hour_coverage` â€”
/// both assertions below name which one broke.
#[test]
fn a_repeat_covered_hour_seals_complete() {
    let span = compact_contract::domain_declared_volatile_prefix_len(18);
    let full = country_identity_payload(&[(48, "FRA")]);
    let mut digest = Sha256::digest(&full[span..]).to_vec();
    let repeat_payload = {
        let mut p = Vec::with_capacity(span + 32);
        p.extend_from_slice(&7u64.to_le_bytes());
        p.append(&mut digest);
        p
    };
    let hour_a = raw_for(HOUR_A);
    let hour_b = raw_for(HOUR_B);

    let mut stream = Vec::new();
    let perf = vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN];
    push_record(&mut stream, 1, hour_a, 18, COMPACT_RECORD_TYPE_DOMAIN_PERF, &perf);
    push_record(&mut stream, 2, hour_a, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &full);
    push_record(
        &mut stream,
        3,
        hour_a,
        0,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(1, 1, 0, 0),
    );
    push_record(&mut stream, 4, hour_b, 18, COMPACT_RECORD_TYPE_DOMAIN_PERF, &perf);
    push_record(
        &mut stream,
        5,
        hour_b,
        18,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT,
        &repeat_payload,
    );
    push_record(
        &mut stream,
        6,
        hour_b,
        0,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(1, 1, 0, 0),
    );

    let mut fold = WorldFold::new("pv_repeat_seal");
    let stats = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
        .expect("a stream with a repeat-covered hour folds cleanly");
    assert_eq!(stats.data_records, 2, "the repeat counts as a data record");
    assert_eq!(stats.malformed_payload_records, 0);
    assert_eq!(stats.hours_emitted, 2, "both hours publish");
    assert!(
        stats.ends_at_frame_end,
        "no violation fired on the repeat record: {:?}",
        stats.stop_reason
    );
    assert_eq!(
        stats.hour_coverage.verified_count(),
        2,
        "both hours seal VERIFIED: {}",
        stats.hour_coverage.summary()
    );
    assert_eq!(stats.hour_coverage.excluded_count(), 0);
}

/// The digest is the fail-closed guarantee behind that seal: a corrupted
/// repeat still seals its hour's BYTES (framing was intact) while the fold
/// refuses the domain by name instead of applying unverified state.
#[test]
fn a_corrupted_repeat_refuses_by_name_while_the_hour_bytes_still_seal() {
    let span = compact_contract::domain_declared_volatile_prefix_len(18);
    let full = country_identity_payload(&[(48, "FRA")]);
    let mut digest = Sha256::digest(&full[span..]).to_vec();
    digest[0] ^= 0xFF;
    let mut repeat_payload = Vec::with_capacity(span + 32);
    repeat_payload.extend_from_slice(&7u64.to_le_bytes());
    repeat_payload.extend_from_slice(&digest);
    let hour_a = raw_for(HOUR_A);
    let hour_b = raw_for(HOUR_B);

    let mut stream = Vec::new();
    let perf = vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN];
    push_record(&mut stream, 1, hour_a, 18, COMPACT_RECORD_TYPE_DOMAIN_PERF, &perf);
    push_record(&mut stream, 2, hour_a, 18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &full);
    push_record(
        &mut stream,
        3,
        hour_a,
        0,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(1, 1, 0, 0),
    );
    push_record(&mut stream, 4, hour_b, 18, COMPACT_RECORD_TYPE_DOMAIN_PERF, &perf);
    push_record(
        &mut stream,
        5,
        hour_b,
        18,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT,
        &repeat_payload,
    );
    push_record(
        &mut stream,
        6,
        hour_b,
        0,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(1, 1, 0, 0),
    );

    let mut fold = WorldFold::new("pv_repeat_corrupt");
    let stats = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
        .expect("a corrupted repeat is a named fold refusal, not framing damage");
    // kills: a refused DOMAIN_DATA_REPEAT that surfaced only as the fold's
    // malformed-hour note, so the walker's exclusion ledger held no entry and
    // fold-check loss accounting omitted the refused repeated hour (Codex
    // review round 10, finding 4). The refusal must be counted HERE.
    assert_eq!(
        stats.malformed_payload_records, 1,
        "the refused repeat is a lost DATA record in this walker's ledger"
    );
    assert_eq!(stats.excluded_domain_data_records.len(), 1);
    assert!(
        stats.excluded_domain_data_records[0]
            .validation_reason
            .contains("DOMAIN_DATA_REPEAT composition refused"),
        "{}",
        stats.excluded_domain_data_records[0].validation_reason
    );
    assert_eq!(stats.hour_coverage.verified_count(), 2);
    let snapshots: Vec<serde_json::Value> = fold
        .take_finished()
        .into_iter()
        .map(|(_, _, bytes)| serde_json::from_slice(&bytes).expect("snapshot json"))
        .collect();
    let refused = snapshots.iter().any(|snapshot| {
        snapshot.to_string().contains("invalid_domain_data_repeat")
    });
    assert!(
        refused,
        "the digest mismatch must be named in the published provenance"
    );
}
}
