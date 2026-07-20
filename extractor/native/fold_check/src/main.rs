//! `fold_check` — the fold-equivalence gate (G1) for the extraction pipeline.
//!
//! Given a collector compact record stream (`collector_stream.bin`) or a
//! `.battleplan` container, it decodes every record and drives the **existing
//! production fold** ([`recorder_host::world_fold::WorldFold`]) to produce the
//! per-game-hour world snapshots, then emits a canonical SHA-256 plus normalized
//! status/completeness evidence per hour.
//!
//! There is exactly ONE decode+fold in the codebase. This tool now reuses it
//! through the shared [`replay_decoder`] facade: `replay_decoder` frames the
//! `H4CR` records (via the single `compact_contract::parse_compact_record_header`
//! primitive), drives the production `recorder_host::world_fold::WorldFold`, and
//! yields each game-hour's canonical snapshot bytes in bounded memory. `fold_check`
//! just hashes those bytes. Folding under the fixed [`FOLD_CHECK_GAME_ID`] makes a
//! raw stream and the same stream packed in a container hash identically, so an
//! optimized collector can be certified byte-identical (Extraction Optimization
//! Plan, gate G1).
//!
//! Usage:
//!   fold_check hash    <input> --expected-profile production [--identity-mode legacy|identity-v1] [--out report.json]
//!   fold_check compare <a> <b> --expected-profile production [--identity-mode legacy|identity-v1] [--out report.json] [--sample N]
//!
//! `<input>` is auto-detected: a stream begins with the `H4CR` magic; a container
//! begins with the ZIP local-file magic and its `collector_compact_stream` entry
//! is extracted and folded.
//!
//! `hash` prints a deterministic JSON report (per-hour hash + per-domain record
//! counts + totals + a chain hash) to stdout. `compare` exits 0 iff both inputs
//! fold to the identical hour range with identical per-hour hashes and
//! authoritative completeness; on divergence it reports the first divergent
//! hour with a bounded per-section drill-down.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;

use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance,
    compact_status_for_global_interpretation, compact_status_is_failure, lane_local_status_mask,
    parse_compact_record_header, validate_compact_record_semantics, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END,
};
use compact_contract::stream_validator::{
    AcceptedRecordKind, StatefulStreamValidator, StreamIdentityMode, StreamValidationError,
};
use replay_decoder::{DecodeStats, OpenOptions, Source};
use serde_json::{json, Map, Value};
use snapshot::container::{Container, ExpectedIdentity};
use snapshot::sha256::{to_hex, Sha256};

/// Fixed synthetic game id so the folded snapshot's `game_id` field (and thus the
/// hash) depends only on the record stream, never on the file path. Two files
/// with identical stream bytes therefore hash identically — the whole point.
const FOLD_CHECK_GAME_ID: &str = "fold_check";

/// The decode options every `fold_check` run uses: fold under the fixed
/// [`FOLD_CHECK_GAME_ID`] regardless of source, so a raw stream and the same
/// stream packed in a container hash identically.
fn fold_check_opts() -> OpenOptions {
    OpenOptions {
        game_id: Some(FOLD_CHECK_GAME_ID.to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let rc = match args.get(1).map(String::as_str) {
        Some("hash") => cmd_hash(&args[2..]),
        Some("compare") => cmd_compare(&args[2..]),
        Some("-h") | Some("--help") | None => {
            print_usage();
            Ok(0)
        }
        Some(other) => {
            eprintln!("fold_check: unknown subcommand {other:?}\n");
            print_usage();
            Ok(2)
        }
    };
    match rc {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("fold_check: error: {e}");
            ExitCode::from(1)
        }
    }
}

fn print_usage() {
    eprintln!(
        "fold_check — fold-equivalence gate (G1)\n\
         \n\
         USAGE:\n\
         \x20 fold_check hash    <input> --expected-profile production [--identity-mode legacy|identity-v1] [--out report.json]\n\
         \x20 fold_check compare <a> <b> --expected-profile production [--identity-mode legacy|identity-v1] [--out report.json] [--sample N]\n\
         \n\
         <input> is a collector_stream.bin OR a .battleplan container.\n\
         hash    prints a deterministic per-hour hash report.\n\
         compare exits 0 iff the two inputs fold identically.\n\
         --expected-profile is mandatory: certification never trusts the\n\
         stream's own FRAME_END counts as its coverage authority.\n\
         --identity-mode defaults to legacy. identity-v1 explicitly requires\n\
         one placement-valid CID1 sequence-zero header before ordinary records."
    );
}

fn stream_identity_mode_name(mode: StreamIdentityMode) -> &'static str {
    match mode {
        StreamIdentityMode::Legacy => "legacy",
        StreamIdentityMode::IdentityV1 => "identity-v1",
    }
}

// ---------------------------------------------------------------------------
// Shared: fold-run accounting
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DomainCompleteness {
    data_records: u64,
    perf_records: u64,
    normalized_status_flags_or: u32,
    lane_local_status_flags_or: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct HourCompleteness {
    domains: BTreeMap<u16, DomainCompleteness>,
    frame_emitted_domains: u32,
    frame_scheduled_domains: u32,
    normalized_frame_status_flags: u32,
    normalized_status_flags_or: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpectedDomain {
    wire_name: String,
    requires_data_record: bool,
}

/// A caller-selected authority that is independent of the stream being
/// certified. The stream may report its own scheduled/emitted counts, but it
/// cannot define what production was expected to contain.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ExpectedRoster {
    profile: String,
    domains: BTreeMap<u16, ExpectedDomain>,
}

impl ExpectedRoster {
    fn production() -> Self {
        let domains = compact_contract::production_profile_domains()
            .into_iter()
            .map(|descriptor| {
                (
                    descriptor.id,
                    ExpectedDomain {
                        wire_name: descriptor.wire_name.to_string(),
                        requires_data_record: descriptor.requires_data_record,
                    },
                )
            })
            .collect();
        Self {
            profile: "production".to_string(),
            domains,
        }
    }

    #[cfg(test)]
    fn test(domains: &[(u16, bool)]) -> Self {
        Self {
            profile: "test".to_string(),
            domains: domains
                .iter()
                .map(|(domain_id, requires_data_record)| {
                    (
                        *domain_id,
                        ExpectedDomain {
                            wire_name: compact_contract::wire_name_for_id(*domain_id)
                                .unwrap_or("test_domain")
                                .to_string(),
                            requires_data_record: *requires_data_record,
                        },
                    )
                })
                .collect(),
        }
    }
}

fn verify_expected_roster(
    hour_index: i64,
    completeness: &HourCompleteness,
    expected: &ExpectedRoster,
) -> Result<(), String> {
    let expected_count = u32::try_from(expected.domains.len())
        .map_err(|_| "expected domain roster is larger than u32".to_string())?;
    if completeness.frame_scheduled_domains != expected_count {
        return Err(format!(
            "hour {hour_index} FRAME_END scheduled-domain count {} does not match externally authoritative {} profile roster count {expected_count}",
            completeness.frame_scheduled_domains, expected.profile
        ));
    }
    if completeness.frame_emitted_domains != expected_count {
        return Err(format!(
            "hour {hour_index} FRAME_END emitted-domain count {} does not match externally authoritative {} profile roster count {expected_count}",
            completeness.frame_emitted_domains, expected.profile
        ));
    }

    for (domain_id, expected_domain) in &expected.domains {
        let observed = completeness.domains.get(domain_id);
        let perf_records = observed.map_or(0, |domain| domain.perf_records);
        if perf_records != 1 {
            return Err(format!(
                "hour {hour_index} expected {} profile domain {} ({domain_id}) PERF record count 1, observed {perf_records}",
                expected.profile, expected_domain.wire_name
            ));
        }
        let data_records = observed.map_or(0, |domain| domain.data_records);
        let expected_data_records = u64::from(expected_domain.requires_data_record);
        if data_records != expected_data_records {
            return Err(format!(
                "hour {hour_index} expected {} profile domain {} ({domain_id}) DATA record count {expected_data_records}, observed {data_records}",
                expected.profile, expected_domain.wire_name
            ));
        }
    }

    for (domain_id, observed) in &completeness.domains {
        if (observed.data_records != 0 || observed.perf_records != 0)
            && !expected.domains.contains_key(domain_id)
        {
            return Err(format!(
                "hour {hour_index} observed domain {domain_id} outside externally authoritative {} profile roster (DATA={}, PERF={})",
                expected.profile, observed.data_records, observed.perf_records
            ));
        }
    }
    Ok(())
}

#[derive(Default)]
struct CompletenessAudit {
    hours: BTreeMap<i64, HourCompleteness>,
    current_domains: BTreeMap<u16, DomainCompleteness>,
}

impl CompletenessAudit {
    fn observe(
        &mut self,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) -> Result<(), String> {
        validate_compact_record_semantics(domain_id, record_type, record_flags, payload)?;
        match record_type {
            COMPACT_RECORD_TYPE_DOMAIN_DATA => {
                self.current_domains
                    .entry(domain_id)
                    .or_default()
                    .data_records += 1;
            }
            COMPACT_RECORD_TYPE_DOMAIN_PERF => {
                let status_flags = read_u32_at(payload, 32, "DOMAIN_PERF status")?;
                let normalized = compact_status_for_global_interpretation(domain_id, status_flags);
                let residual = compact_status_failure_flags_after_expected_partial_tolerance(
                    domain_id,
                    status_flags,
                );
                if residual != 0 {
                    return Err(format!(
                        "domain {domain_id} PERF status {status_flags:#010x} has non-tolerated failure flags {residual:#010x}"
                    ));
                }
                // Unassigned PERF-only ids (currently diagnostic probes such as
                // 250) are instrumentation, not extraction completeness. Their
                // timing/unit payloads remain intentionally outside G1 identity.
                if compact_contract::assigned_descriptor_by_id(domain_id).is_some() {
                    let domain = self.current_domains.entry(domain_id).or_default();
                    domain.perf_records += 1;
                    domain.normalized_status_flags_or |= normalized;
                    domain.lane_local_status_flags_or |=
                        status_flags & lane_local_status_mask(domain_id);
                }
            }
            COMPACT_RECORD_TYPE_FRAME_END => {
                let emitted = read_u32_at(payload, 16, "FRAME_END emitted-domain count")?;
                let scheduled = read_u32_at(payload, 20, "FRAME_END scheduled-domain count")?;
                let status_flags = read_u32_at(payload, 24, "FRAME_END status")?;
                let normalized = compact_status_for_global_interpretation(domain_id, status_flags);
                if compact_status_is_failure(normalized) {
                    return Err(format!(
                        "FRAME_END status {status_flags:#010x} contains failure flags"
                    ));
                }
                if emitted != scheduled {
                    return Err(format!(
                        "FRAME_END emitted-domain count {emitted} does not match scheduled-domain count {scheduled}"
                    ));
                }
                let hour_index = compact_contract::calendar::hour_index_from_raw(game_hour_raw)
                    .ok_or_else(|| format!("FRAME_END game hour {game_hour_raw} is invalid"))?;
                let aggregate = self
                    .current_domains
                    .values()
                    .fold(normalized, |status, domain| {
                        status | domain.normalized_status_flags_or
                    });
                let completeness = HourCompleteness {
                    domains: std::mem::take(&mut self.current_domains),
                    frame_emitted_domains: emitted,
                    frame_scheduled_domains: scheduled,
                    normalized_frame_status_flags: normalized,
                    normalized_status_flags_or: aggregate,
                };
                if self.hours.insert(hour_index, completeness).is_some() {
                    return Err(format!(
                        "more than one FRAME_END completeness record for hour {hour_index}"
                    ));
                }
            }
            _ => unreachable!("record semantics rejected unknown type"),
        }
        Ok(())
    }
}

fn read_u32_at(payload: &[u8], offset: usize, label: &str) -> Result<u32, String> {
    let bytes = payload
        .get(offset..offset + 4)
        .ok_or_else(|| format!("{label} is truncated"))?;
    Ok(u32::from_le_bytes(
        bytes.try_into().expect("four-byte status/count slice"),
    ))
}

/// Audit normalized status and completeness directly from decoded H4CR bytes.
/// This is deliberately separate from the world fold: PERF and FRAME_END carry
/// certification evidence but are not folded state. The shared stateful stream
/// validator owns first-record identity placement and ordinary sequence
/// continuity before any record reaches completeness accounting.
fn audit_compact_reader_with_identity_mode(
    mut reader: impl Read,
    identity_mode: StreamIdentityMode,
) -> Result<BTreeMap<i64, HourCompleteness>, String> {
    const CHUNK: usize = 1024 * 1024;
    let mut audit = CompletenessAudit::default();
    let mut stream_validator = match identity_mode {
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
    };
    let mut carry = Vec::with_capacity(CHUNK * 2);
    let mut chunk = vec![0u8; CHUNK];
    let mut offset = 0u64;
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|e| format!("read compact stream for completeness audit: {e}"))?;
        if n != 0 {
            carry.extend_from_slice(&chunk[..n]);
        }
        let mut cursor = 0usize;
        while carry.len().saturating_sub(cursor) >= COMPACT_RECORD_HEADER_LEN {
            let header = parse_compact_record_header(&carry[cursor..]).map_err(|e| {
                format!(
                    "completeness framing at byte {}: {e}",
                    offset + cursor as u64
                )
            })?;
            let record_len = header.record_len();
            if carry.len().saturating_sub(cursor) < record_len {
                break;
            }
            let payload_start = cursor + COMPACT_RECORD_HEADER_LEN;
            let payload = &carry[payload_start..cursor + record_len];
            let accepted = stream_validator
                .validate_record(&header, payload)
                .map_err(|error| {
                    // Pre-2.22 diagnostic containers can contain the old
                    // domain-zero FRAME_END envelope. Domain zero remains a
                    // hard protocol failure (and is never accepted as an
                    // ordinary Legacy record), but retain the current record
                    // semantics error as context so compatibility diagnostics
                    // still identify the historical FRAME_END shape that
                    // prevented certification.
                    let semantics_context = if identity_mode == StreamIdentityMode::Legacy
                        && matches!(&error, StreamValidationError::ReservedDomainZero)
                    {
                        validate_compact_record_semantics(
                            header.domain_id,
                            header.record_type,
                            header.record_flags,
                            payload,
                        )
                        .err()
                        .map(|error| format!("; current record semantics also reject it: {error}"))
                        .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    format!(
                        "{} stream validation at byte {}: {error}{semantics_context}",
                        stream_identity_mode_name(identity_mode),
                        offset + cursor as u64
                    )
                })?;
            if accepted == AcceptedRecordKind::CampaignIdentity {
                cursor += record_len;
                continue;
            }
            audit
                .observe(
                    header.game_hour_raw,
                    header.domain_id,
                    header.record_type,
                    header.record_flags,
                    payload,
                )
                .map_err(|e| {
                    format!(
                        "completeness record at byte {}: {e}",
                        offset + cursor as u64
                    )
                })?;
            cursor += record_len;
        }
        if cursor != 0 {
            carry.drain(..cursor);
            offset += cursor as u64;
        }
        if n == 0 {
            break;
        }
    }
    if !carry.is_empty() {
        return Err(format!(
            "truncated/corrupt tail: completeness audit ended with {} truncated byte(s) at byte {offset}; stream boundary is not closed",
            carry.len()
        ));
    }
    if !audit.current_domains.is_empty() {
        return Err(
            "stream hour left open at EOF; final record must be canonical FRAME_END".to_string(),
        );
    }
    if identity_mode == StreamIdentityMode::IdentityV1 && stream_validator.identity().is_none() {
        return Err("identity-v1 stream ended before its mandatory CID1 header".to_string());
    }
    Ok(audit.hours)
}

fn audit_source(
    source: &Source,
    identity_mode: StreamIdentityMode,
) -> Result<BTreeMap<i64, HourCompleteness>, String> {
    match source {
        Source::StreamFile(path) => {
            let reader = compact_stream::open(path)
                .map_err(|e| format!("open {} for completeness audit: {e}", path.display()))?;
            audit_compact_reader_with_identity_mode(reader, identity_mode)
        }
        Source::Container(path) => {
            let container = Container::open(path, &ExpectedIdentity::default())
                .map_err(|e| format!("open {} for completeness audit: {e}", path.display()))?;
            let entries = container.entries_by_role(replay_decoder::STREAM_ROLE);
            let entry = match entries.as_slice() {
                [entry] => entry,
                [] => {
                    return Err(format!(
                        "{} has no entry with role {:?}",
                        path.display(),
                        replay_decoder::STREAM_ROLE
                    ))
                }
                many => {
                    return Err(format!(
                        "{} has {} entries with role {:?}; expected exactly one",
                        path.display(),
                        many.len(),
                        replay_decoder::STREAM_ROLE
                    ))
                }
            };
            let bytes = container
                .read_entry_bytes(&entry.path)
                .map_err(|e| format!("read {} for completeness audit: {e}", entry.path))?;
            audit_compact_reader_with_identity_mode(std::io::Cursor::new(bytes), identity_mode)
        }
    }
}

fn verify_fold_counts(
    hour_index: i64,
    folded: &BTreeMap<u16, u64>,
    completeness: &HourCompleteness,
) -> Result<(), String> {
    let audited: BTreeMap<u16, u64> = completeness
        .domains
        .iter()
        .filter_map(|(domain_id, domain)| {
            (domain.data_records != 0).then_some((*domain_id, domain.data_records))
        })
        .collect();
    if &audited == folded {
        Ok(())
    } else {
        Err(format!(
            "hour {hour_index} DOMAIN_DATA counts disagree between production fold {folded:?} and completeness audit {audited:?}"
        ))
    }
}

/// One closed game-hour: the canonical hash of its folded snapshot plus the
/// normalized record-level status/completeness evidence that fed it.
struct HourReport {
    hour_index: i64,
    date: String,
    snapshot_hash: String,
    snapshot_bytes: usize,
    domain_data_counts: BTreeMap<u16, u64>,
    completeness: HourCompleteness,
}

#[derive(Default)]
struct Totals {
    stream_bytes: u64,
    bytes_folded: u64,
    truncated_tail_bytes: u64,
    records_total: u64,
    data_records: u64,
    malformed_payload_records: u64,
    frame_end_records: u64,
    perf_records: u64,
    other_records: u64,
    hours_emitted: u64,
    per_domain_data_records: BTreeMap<u16, u64>,
    stop_reason: Option<String>,
    boundary_error: Option<String>,
    ends_at_frame_end: bool,
}

struct FoldRun {
    hours: Vec<HourReport>,
    totals: Totals,
}

/// Convert a [`replay_decoder`] pass's totals into the report `Totals`. The two
/// share the same accounting shape (the decoder mirrors what this tool used to
/// count inline); this is a plain field copy.
fn totals_from_stats(s: &DecodeStats) -> Totals {
    Totals {
        stream_bytes: s.stream_bytes,
        bytes_folded: s.bytes_folded,
        truncated_tail_bytes: s.truncated_tail_bytes,
        records_total: s.records_total,
        data_records: s.data_records,
        malformed_payload_records: s.malformed_payload_records,
        frame_end_records: s.frame_end_records,
        perf_records: s.perf_records,
        other_records: s.other_records,
        hours_emitted: s.hours_emitted,
        per_domain_data_records: s.per_domain_data_records.clone(),
        stop_reason: s.stop_reason.clone(),
        boundary_error: s.boundary_error.clone(),
        ends_at_frame_end: s.ends_at_frame_end,
    }
}

/// Fold an input file through the shared [`replay_decoder`] cursor (auto-detecting
/// a raw `collector_stream.bin` vs a `.battleplan` container), invoking `on_hour`
/// as each game-hour closes so the caller can hash-and-release. The decoder folds
/// under [`FOLD_CHECK_GAME_ID`] regardless of source. The container path inherits
/// the in-RAM entry reader's per-entry cap; use a raw stream for multi-GiB checks.
fn fold_input(
    path: &Path,
    expected_roster: &ExpectedRoster,
    identity_mode: StreamIdentityMode,
    mut on_hour: impl FnMut(i64, &str, &[u8], &BTreeMap<u16, u64>, &HourCompleteness),
) -> Result<(Totals, &'static str), String> {
    let source = Source::detect(path).map_err(|e| e.to_string())?;
    let kind = match source {
        Source::StreamFile(_) => "raw_stream",
        Source::Container(_) => "container",
    };
    let mut completeness = audit_source(&source, identity_mode)?;
    let mut cursor = source
        .open_cursor_with_identity_mode(&fold_check_opts(), identity_mode)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    while let Some(hour) = cursor.next_hour() {
        let hour = hour.map_err(|e| format!("fold {}: {e}", path.display()))?;
        let audited = completeness.get(&hour.hour_index).ok_or_else(|| {
            format!(
                "completeness audit has no closed record boundary for folded hour {}",
                hour.hour_index
            )
        })?;
        verify_fold_counts(hour.hour_index, &hour.domain_data_counts, audited)?;
        verify_expected_roster(hour.hour_index, audited, expected_roster)?;
        on_hour(
            hour.hour_index,
            &hour.date,
            &hour.snapshot,
            &hour.domain_data_counts,
            audited,
        );
        completeness.remove(&hour.hour_index);
    }
    cursor
        .stats()
        .certification_verdict()
        .map_err(|reason| format!("certify {}: {reason}", path.display()))?;
    if let Some(hour) = completeness.keys().next() {
        return Err(format!(
            "completeness audit produced hour {hour} that the production fold did not emit"
        ));
    }
    Ok((totals_from_stats(cursor.stats()), kind))
}

/// Record one closed hour into the running `hours` list. The fold's output is
/// canonical by construction — every accumulator is a BTreeMap/BTreeSet and
/// serde_json (no `preserve_order`) writes object keys sorted, so identical
/// input yields byte-identical snapshot bytes; we hash those shipped bytes.
fn push_hour(
    hours: &mut Vec<HourReport>,
    hour_index: i64,
    date: &str,
    bytes: &[u8],
    counts: &BTreeMap<u16, u64>,
    completeness: &HourCompleteness,
) {
    hours.push(HourReport {
        hour_index,
        date: date.to_string(),
        snapshot_hash: to_hex(&Sha256::digest(bytes)),
        snapshot_bytes: bytes.len(),
        domain_data_counts: counts.clone(),
        completeness: completeness.clone(),
    });
}

/// Fold an input file into an in-memory [`FoldRun`] (per-hour hashes + totals).
#[cfg(test)]
fn run_input(
    path: &Path,
    expected_roster: &ExpectedRoster,
) -> Result<(FoldRun, &'static str), String> {
    run_input_with_identity_mode(path, expected_roster, StreamIdentityMode::Legacy)
}

fn run_input_with_identity_mode(
    path: &Path,
    expected_roster: &ExpectedRoster,
    identity_mode: StreamIdentityMode,
) -> Result<(FoldRun, &'static str), String> {
    let mut hours: Vec<HourReport> = Vec::new();
    let (totals, kind) = fold_input(
        path,
        expected_roster,
        identity_mode,
        |hour_index, date, bytes, counts, completeness| {
            push_hour(&mut hours, hour_index, date, bytes, counts, completeness)
        },
    )?;
    Ok((FoldRun { hours, totals }, kind))
}

/// Fold an already-opened in-memory reader into a [`FoldRun`] through the shared
/// [`replay_decoder`] cursor. Used by the unit tests, which fold an in-memory
/// `Cursor` (under [`FOLD_CHECK_GAME_ID`]) without going through a file.
#[cfg(test)]
fn collect_run(reader: std::io::Cursor<Vec<u8>>, total_len: u64) -> Result<FoldRun, String> {
    collect_run_with_expected_roster(reader, total_len, None)
}

#[cfg(test)]
fn collect_identity_v1_run(
    reader: std::io::Cursor<Vec<u8>>,
    total_len: u64,
) -> Result<FoldRun, String> {
    collect_run_with_identity_mode_and_expected_roster(
        reader,
        total_len,
        StreamIdentityMode::IdentityV1,
        None,
    )
}

#[cfg(test)]
fn collect_certified_run(
    reader: std::io::Cursor<Vec<u8>>,
    total_len: u64,
    expected_roster: &ExpectedRoster,
) -> Result<FoldRun, String> {
    collect_run_with_expected_roster(reader, total_len, Some(expected_roster))
}

#[cfg(test)]
fn collect_run_with_expected_roster(
    reader: std::io::Cursor<Vec<u8>>,
    total_len: u64,
    expected_roster: Option<&ExpectedRoster>,
) -> Result<FoldRun, String> {
    collect_run_with_identity_mode_and_expected_roster(
        reader,
        total_len,
        StreamIdentityMode::Legacy,
        expected_roster,
    )
}

#[cfg(test)]
fn collect_run_with_identity_mode_and_expected_roster(
    reader: std::io::Cursor<Vec<u8>>,
    total_len: u64,
    identity_mode: StreamIdentityMode,
    expected_roster: Option<&ExpectedRoster>,
) -> Result<FoldRun, String> {
    let mut hours: Vec<HourReport> = Vec::new();
    let mut completeness = audit_compact_reader_with_identity_mode(
        std::io::Cursor::new(reader.get_ref().clone()),
        identity_mode,
    )?;
    let mut cursor = replay_decoder::open_reader_with_identity_mode(
        reader,
        total_len,
        FOLD_CHECK_GAME_ID,
        identity_mode,
    );
    while let Some(hour) = cursor.next_hour() {
        let hour = hour.map_err(|e| e.to_string())?;
        let audited = completeness.get(&hour.hour_index).ok_or_else(|| {
            format!(
                "completeness audit has no closed record boundary for folded hour {}",
                hour.hour_index
            )
        })?;
        verify_fold_counts(hour.hour_index, &hour.domain_data_counts, audited)?;
        if let Some(expected_roster) = expected_roster {
            verify_expected_roster(hour.hour_index, audited, expected_roster)?;
        }
        push_hour(
            &mut hours,
            hour.hour_index,
            &hour.date,
            &hour.snapshot,
            &hour.domain_data_counts,
            audited,
        );
        completeness.remove(&hour.hour_index);
    }
    cursor
        .stats()
        .certification_verdict()
        .map_err(|reason| format!("certify in-memory stream: {reason}"))?;
    if let Some(hour) = completeness.keys().next() {
        return Err(format!(
            "completeness audit produced hour {hour} that the production fold did not emit"
        ));
    }
    Ok(FoldRun {
        hours,
        totals: totals_from_stats(cursor.stats()),
    })
}

/// Chain hash over ordered per-hour folded hashes and normalized completeness:
/// a single anchor for the full G1 identity rather than folded state alone.
fn chain_hash(hours: &[HourReport]) -> String {
    let mut h = Sha256::new();
    for hr in hours {
        h.update(hr.hour_index.to_le_bytes().as_slice());
        h.update(hr.snapshot_hash.as_bytes());
        h.update(
            hr.completeness
                .frame_emitted_domains
                .to_le_bytes()
                .as_slice(),
        );
        h.update(
            hr.completeness
                .frame_scheduled_domains
                .to_le_bytes()
                .as_slice(),
        );
        h.update(
            hr.completeness
                .normalized_frame_status_flags
                .to_le_bytes()
                .as_slice(),
        );
        h.update(
            hr.completeness
                .normalized_status_flags_or
                .to_le_bytes()
                .as_slice(),
        );
        h.update(
            (hr.completeness.domains.len() as u64)
                .to_le_bytes()
                .as_slice(),
        );
        for (domain_id, domain) in &hr.completeness.domains {
            h.update(domain_id.to_le_bytes().as_slice());
            h.update(domain.data_records.to_le_bytes().as_slice());
            h.update(domain.perf_records.to_le_bytes().as_slice());
            h.update(domain.normalized_status_flags_or.to_le_bytes().as_slice());
            h.update(domain.lane_local_status_flags_or.to_le_bytes().as_slice());
        }
        h.update(b"\n");
    }
    to_hex(&h.finalize())
}

// ---------------------------------------------------------------------------
// hash
// ---------------------------------------------------------------------------

fn cmd_hash(args: &[String]) -> Result<u8, String> {
    let expected_roster = parse_expected_profile(args)?;
    let identity_mode = parse_stream_identity_mode(args)?;
    let (positional, out) = parse_common(args)?;
    if positional.len() != 1 {
        return Err("hash takes exactly one <input>".into());
    }
    let path = Path::new(&positional[0]);
    let (run, kind) = run_input_with_identity_mode(path, &expected_roster, identity_mode)?;
    let report = hash_report(path, kind, &run, &expected_roster, identity_mode);
    emit(&report, out.as_deref())?;
    Ok(0)
}

fn hash_report(
    path: &Path,
    kind: &str,
    run: &FoldRun,
    expected_roster: &ExpectedRoster,
    identity_mode: StreamIdentityMode,
) -> Value {
    let mut per_domain = Map::new();
    for (dom, n) in &run.totals.per_domain_data_records {
        per_domain.insert(dom.to_string(), json!({ "data_records": n }));
    }
    let hours: Vec<Value> = run
        .hours
        .iter()
        .map(|hr| {
            let mut doms = Map::new();
            for (d, c) in &hr.domain_data_counts {
                doms.insert(d.to_string(), json!(c));
            }
            json!({
                "hour_index": hr.hour_index,
                "date": hr.date,
                "snapshot_hash": hr.snapshot_hash,
                "snapshot_bytes": hr.snapshot_bytes,
                "domain_data_counts": Value::Object(doms),
                "normalized_completeness": completeness_json(&hr.completeness),
            })
        })
        .collect();
    let range = hour_range(&run.hours);
    json!({
        "tool": "fold_check",
        "mode": "hash",
        "report_version": 3,
        "input": path.display().to_string(),
        "input_kind": kind,
        "identity_mode": stream_identity_mode_name(identity_mode),
        "game_id": FOLD_CHECK_GAME_ID,
        "expected_roster": expected_roster_json(expected_roster),
        "totals": totals_json(&run.totals),
        "hour_range": range,
        "per_domain": Value::Object(per_domain),
        "hours": hours,
        "chain_hash": chain_hash(&run.hours),
    })
}

fn expected_roster_json(expected: &ExpectedRoster) -> Value {
    let domains: Vec<Value> = expected
        .domains
        .iter()
        .map(|(domain_id, domain)| {
            json!({
                "domain_id": domain_id,
                "wire_name": domain.wire_name,
                "requires_data_record": domain.requires_data_record,
                "requires_perf_record": true,
            })
        })
        .collect();
    json!({
        "profile": expected.profile,
        "domain_count": expected.domains.len(),
        "domains": domains,
    })
}

fn completeness_json(completeness: &HourCompleteness) -> Value {
    let mut domains = Map::new();
    for (domain_id, domain) in &completeness.domains {
        domains.insert(
            domain_id.to_string(),
            json!({
                "data_records": domain.data_records,
                "perf_records": domain.perf_records,
                "normalized_status_flags_or": domain.normalized_status_flags_or,
                "lane_local_status_flags_or": domain.lane_local_status_flags_or,
            }),
        );
    }
    json!({
        "frame_emitted_domains": completeness.frame_emitted_domains,
        "frame_scheduled_domains": completeness.frame_scheduled_domains,
        "normalized_frame_status_flags": completeness.normalized_frame_status_flags,
        "normalized_status_flags_or": completeness.normalized_status_flags_or,
        "domains": Value::Object(domains),
    })
}

fn totals_json(t: &Totals) -> Value {
    let mut v = json!({
        "stream_bytes": t.stream_bytes,
        "bytes_folded": t.bytes_folded,
        "truncated_tail_bytes": t.truncated_tail_bytes,
        "records_total": t.records_total,
        "data_records": t.data_records,
        "malformed_payload_records": t.malformed_payload_records,
        "frame_end_records": t.frame_end_records,
        "perf_records_ignored": t.perf_records,
        "other_records_ignored": t.other_records,
        "hours_emitted": t.hours_emitted,
        "ends_at_frame_end": t.ends_at_frame_end,
    });
    if let Some(reason) = &t.stop_reason {
        v.as_object_mut()
            .unwrap()
            .insert("framing_stopped".into(), json!(reason));
    }
    if let Some(reason) = &t.boundary_error {
        v.as_object_mut()
            .unwrap()
            .insert("boundary_error".into(), json!(reason));
    }
    v
}

fn hour_range(hours: &[HourReport]) -> Value {
    match (hours.first(), hours.last()) {
        (Some(f), Some(l)) => json!([f.hour_index, l.hour_index]),
        _ => Value::Null,
    }
}

// ---------------------------------------------------------------------------
// compare
// ---------------------------------------------------------------------------

fn cmd_compare(args: &[String]) -> Result<u8, String> {
    let expected_roster = parse_expected_profile(args)?;
    let identity_mode = parse_stream_identity_mode(args)?;
    let (positional, out) = parse_common(args)?;
    let sample = parse_sample(args)?.unwrap_or(24);
    if positional.len() != 2 {
        return Err("compare takes exactly two inputs <a> <b>".into());
    }
    let path_a = Path::new(&positional[0]);
    let path_b = Path::new(&positional[1]);
    let (run_a, kind_a) =
        run_input_with_identity_mode(path_a, &expected_roster, identity_mode)?;
    let (run_b, kind_b) =
        run_input_with_identity_mode(path_b, &expected_roster, identity_mode)?;

    let map_a: BTreeMap<i64, &HourReport> = run_a.hours.iter().map(|h| (h.hour_index, h)).collect();
    let map_b: BTreeMap<i64, &HourReport> = run_b.hours.iter().map(|h| (h.hour_index, h)).collect();

    // First divergent hour across the union of hour indices, in order.
    let mut all_hours: Vec<i64> = map_a.keys().chain(map_b.keys()).copied().collect();
    all_hours.sort_unstable();
    all_hours.dedup();

    let mut first_divergence: Option<Value> = None;
    for h in &all_hours {
        match (map_a.get(h), map_b.get(h)) {
            (Some(a), Some(b)) => {
                if a.snapshot_hash != b.snapshot_hash {
                    first_divergence = Some(divergence_hash_mismatch(
                        *h,
                        a,
                        b,
                        path_a,
                        path_b,
                        sample,
                        &expected_roster,
                        identity_mode,
                    )?);
                    break;
                } else if a.completeness != b.completeness {
                    first_divergence = Some(json!({
                        "hour_index": h,
                        "kind": "normalized_completeness_mismatch",
                        "a_hash": a.snapshot_hash,
                        "b_hash": b.snapshot_hash,
                        "a_normalized_completeness": completeness_json(&a.completeness),
                        "b_normalized_completeness": completeness_json(&b.completeness),
                    }));
                    break;
                }
            }
            (Some(a), None) => {
                first_divergence = Some(json!({
                    "hour_index": h,
                    "kind": "missing_in_b",
                    "a_hash": a.snapshot_hash,
                    "b_hash": Value::Null,
                }));
                break;
            }
            (None, Some(b)) => {
                first_divergence = Some(json!({
                    "hour_index": h,
                    "kind": "missing_in_a",
                    "a_hash": Value::Null,
                    "b_hash": b.snapshot_hash,
                }));
                break;
            }
            (None, None) => {}
        }
    }

    let identical = first_divergence.is_none();
    let report = json!({
        "tool": "fold_check",
        "mode": "compare",
        "report_version": 3,
        "identical": identical,
        "identity_mode": stream_identity_mode_name(identity_mode),
        "expected_roster": expected_roster_json(&expected_roster),
        "a": side_summary(path_a, kind_a, &run_a),
        "b": side_summary(path_b, kind_b, &run_b),
        "hour_count_match": run_a.hours.len() == run_b.hours.len(),
        "first_divergence": first_divergence.unwrap_or(Value::Null),
    });
    emit(&report, out.as_deref())?;
    Ok(if identical { 0 } else { 1 })
}

fn side_summary(path: &Path, kind: &str, run: &FoldRun) -> Value {
    json!({
        "input": path.display().to_string(),
        "input_kind": kind,
        "hours": run.hours.len(),
        "hour_range": hour_range(&run.hours),
        "chain_hash": chain_hash(&run.hours),
        "totals": totals_json(&run.totals),
    })
}

/// Build the drill-down for a hash-mismatched hour: re-fold BOTH inputs, capture
/// only that hour's snapshot bytes, and diff the two folded Values.
fn divergence_hash_mismatch(
    hour: i64,
    a: &HourReport,
    b: &HourReport,
    path_a: &Path,
    path_b: &Path,
    sample: usize,
    expected_roster: &ExpectedRoster,
    identity_mode: StreamIdentityMode,
) -> Result<Value, String> {
    let bytes_a = capture_hour(path_a, hour, expected_roster, identity_mode)?;
    let bytes_b = capture_hour(path_b, hour, expected_roster, identity_mode)?;
    let (section_diffs, sections) = match (bytes_a, bytes_b) {
        (Some(ba), Some(bb)) => {
            let va: Value = serde_json::from_slice(&ba)
                .map_err(|e| format!("parse snapshot a hour {hour}: {e}"))?;
            let vb: Value = serde_json::from_slice(&bb)
                .map_err(|e| format!("parse snapshot b hour {hour}: {e}"))?;
            let mut diffs: Vec<Value> = Vec::new();
            diff_values(&va, &vb, "", &mut diffs, sample);
            (Value::Array(diffs), section_summary(&va, &vb))
        }
        _ => (Value::Null, Value::Null),
    };

    let mut domain_diffs = Map::new();
    let doms: std::collections::BTreeSet<u16> = a
        .domain_data_counts
        .keys()
        .chain(b.domain_data_counts.keys())
        .copied()
        .collect();
    for d in doms {
        let ca = a.domain_data_counts.get(&d).copied().unwrap_or(0);
        let cb = b.domain_data_counts.get(&d).copied().unwrap_or(0);
        if ca != cb {
            domain_diffs.insert(d.to_string(), json!({ "a": ca, "b": cb }));
        }
    }

    Ok(json!({
        "hour_index": hour,
        "kind": "hash_mismatch",
        "date": a.date,
        "a_hash": a.snapshot_hash,
        "b_hash": b.snapshot_hash,
        "a_snapshot_bytes": a.snapshot_bytes,
        "b_snapshot_bytes": b.snapshot_bytes,
        "a_normalized_completeness": completeness_json(&a.completeness),
        "b_normalized_completeness": completeness_json(&b.completeness),
        "domain_data_count_diffs": Value::Object(domain_diffs),
        "sections": sections,
        "section_diffs_truncated_at": sample,
        "section_diffs": section_diffs,
    }))
}

/// Re-fold `path`, keeping only the snapshot bytes for `target` hour.
fn capture_hour(
    path: &Path,
    target: i64,
    expected_roster: &ExpectedRoster,
    identity_mode: StreamIdentityMode,
) -> Result<Option<Vec<u8>>, String> {
    let mut captured: Option<Vec<u8>> = None;
    fold_input(
        path,
        expected_roster,
        identity_mode,
        |hour_index, _date, bytes, _counts, _completeness| {
            if hour_index == target && captured.is_none() {
                captured = Some(bytes.to_vec());
            }
        },
    )?;
    Ok(captured)
}

/// Per top-level snapshot section (the fold's output shape): the "row count" in
/// each side and whether they are equal. A concise map of where the divergence
/// lives before the fine-grained `section_diffs`.
fn section_summary(a: &Value, b: &Value) -> Value {
    let empty = Map::new();
    let am = a.as_object().unwrap_or(&empty);
    let bm = b.as_object().unwrap_or(&empty);
    let keys: std::collections::BTreeSet<&String> = am.keys().chain(bm.keys()).collect();
    let mut out = Map::new();
    for k in keys {
        let av = am.get(k);
        let bv = bm.get(k);
        let equal = av == bv;
        if !equal {
            out.insert(
                k.clone(),
                json!({
                    "a_rows": value_len(av),
                    "b_rows": value_len(bv),
                    "equal": false,
                }),
            );
        }
    }
    Value::Object(out)
}

/// "Row count" of a value: array/object length, 0 for null/absent, 1 for a scalar.
fn value_len(v: Option<&Value>) -> Value {
    match v {
        None | Some(Value::Null) => json!(0),
        Some(Value::Array(a)) => json!(a.len()),
        Some(Value::Object(o)) => json!(o.len()),
        Some(_) => json!(1),
    }
}

/// Recursive bounded structural diff. Collects at most `limit` differing leaves,
/// each with its path and a short preview of both sides.
fn diff_values(a: &Value, b: &Value, path: &str, out: &mut Vec<Value>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    match (a, b) {
        (Value::Object(am), Value::Object(bm)) => {
            let keys: std::collections::BTreeSet<&String> = am.keys().chain(bm.keys()).collect();
            for k in keys {
                if out.len() >= limit {
                    return;
                }
                let child = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}/{k}")
                };
                match (am.get(k), bm.get(k)) {
                    (Some(x), Some(y)) => diff_values(x, y, &child, out, limit),
                    (Some(x), None) => out.push(json!({
                        "path": child, "kind": "only_in_a", "a": preview(x), "b": Value::Null,
                    })),
                    (None, Some(y)) => out.push(json!({
                        "path": child, "kind": "only_in_b", "a": Value::Null, "b": preview(y),
                    })),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(aa), Value::Array(ba)) => {
            if aa.len() != ba.len() {
                out.push(json!({
                    "path": path, "kind": "array_len", "a": aa.len(), "b": ba.len(),
                }));
            }
            let n = aa.len().min(ba.len());
            for i in 0..n {
                if out.len() >= limit {
                    return;
                }
                diff_values(&aa[i], &ba[i], &format!("{path}[{i}]"), out, limit);
            }
        }
        _ => {
            if a != b {
                out.push(json!({
                    "path": path, "kind": "value", "a": preview(a), "b": preview(b),
                }));
            }
        }
    }
}

/// A bounded preview of a value for the drill-down (scalars pass through; large
/// containers/strings are summarized so the report stays small).
fn preview(v: &Value) -> Value {
    match v {
        Value::Array(a) => json!(format!("<array len {}>", a.len())),
        Value::Object(o) => json!(format!("<object len {}>", o.len())),
        Value::String(s) if s.len() > 120 => json!(format!("{}…({} chars)", &s[..117], s.len())),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// arg parsing + output
// ---------------------------------------------------------------------------

/// Extract positional args and an optional `--out <file>`.
fn parse_common(args: &[String]) -> Result<(Vec<String>, Option<String>), String> {
    let mut positional = Vec::new();
    let mut out = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out = Some(args.get(i).ok_or("--out requires a file path")?.clone());
            }
            "--sample" => {
                i += 1; // value validated in parse_sample; skip here
            }
            "--expected-profile" => {
                i += 1; // value validated in parse_expected_profile; skip here
            }
            "--identity-mode" => {
                i += 1; // value validated in parse_stream_identity_mode; skip here
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {other}"));
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }
    Ok((positional, out))
}

fn parse_stream_identity_mode(args: &[String]) -> Result<StreamIdentityMode, String> {
    let mut selected: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--identity-mode" {
            let value = args
                .get(i + 1)
                .ok_or("--identity-mode requires legacy or identity-v1")?;
            if selected.replace(value.as_str()).is_some() {
                return Err("--identity-mode may be specified only once".to_string());
            }
            i += 1;
        }
        i += 1;
    }
    match selected {
        None | Some("legacy") => Ok(StreamIdentityMode::Legacy),
        Some("identity-v1") => Ok(StreamIdentityMode::IdentityV1),
        Some(other) => Err(format!(
            "unsupported stream identity mode {other:?}; expected legacy or identity-v1"
        )),
    }
}

fn parse_expected_profile(args: &[String]) -> Result<ExpectedRoster, String> {
    let mut selected: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--expected-profile" {
            let value = args
                .get(i + 1)
                .ok_or("--expected-profile requires a profile name")?;
            if selected.replace(value.as_str()).is_some() {
                return Err("--expected-profile may be specified only once".to_string());
            }
            i += 1;
        }
        i += 1;
    }
    match selected {
        Some("production") => Ok(ExpectedRoster::production()),
        Some(other) => Err(format!(
            "unsupported expected profile {other:?}; only \"production\" is authoritative"
        )),
        None => Err(
            "missing mandatory --expected-profile production; FRAME_END self-reported counts are not a coverage authority"
                .to_string(),
        ),
    }
}

fn parse_sample(args: &[String]) -> Result<Option<usize>, String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--sample" {
            let v = args.get(i + 1).ok_or("--sample requires a count")?;
            return v
                .parse::<usize>()
                .map(Some)
                .map_err(|_| format!("--sample expects a non-negative integer, got {v:?}"));
        }
        i += 1;
    }
    Ok(None)
}

/// Write a report as deterministic pretty JSON to a file or stdout.
fn emit(report: &Value, out: Option<&str>) -> Result<(), String> {
    let mut text =
        serde_json::to_string_pretty(report).map_err(|e| format!("serialize report: {e}"))?;
    text.push('\n');
    match out {
        Some(path) => {
            File::create(path)
                .and_then(|mut f| f.write_all(text.as_bytes()))
                .map_err(|e| format!("write {path}: {e}"))?;
        }
        None => {
            io::stdout()
                .write_all(text.as_bytes())
                .map_err(|e| format!("write stdout: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
