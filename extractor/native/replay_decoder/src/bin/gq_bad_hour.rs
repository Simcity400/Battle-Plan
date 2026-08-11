//! Lane GQ — bad-hour fault-injection harness.
//!
//! # The question
//!
//! Owner directive (2026-08-11): *"one bad hour doesn't destroy the data for the
//! entire run … if we're going to have bad hours, it shouldn't just make the
//! entire recording get bricked."* This instrument answers it by measurement
//! rather than by reading: it injects one fault at a time into an otherwise
//! clean H4CR stream and reports, for every cell, exactly which game hours the
//! real production paths still deliver, which they lose, which they deliver
//! CHANGED, and whether the loss is NAMED anywhere a consumer can read.
//!
//! # What is real here and what is not
//!
//! * **Real**: every decision path. The fold is `recorder_host::world_fold`
//!   through `stream_fold::drive_fold_*` (the production streaming fold) and
//!   `replay_decoder`'s `HourCursor` (the production replay decoder). The
//!   certification verdict is `strict_stream_certification_verdict`. The
//!   seal-time gates are `validate_excluded_domain_data_accounting` +
//!   `strict_stream_certification_verdict` (the `fold_stats` arm) and
//!   `verify_stream_logical_boundary_with_identity_mode` (the skip-fold arm) —
//!   the same two calls `collector_record`'s seal makes, in the same order.
//!   The container gate is `snapshot::container::Container::verify_all`.
//! * **Not real**: the base stream BYTES. No captured H4CR spool survives on
//!   this machine — the four retained captures (`runs/live_unlock/t46..t49`) are
//!   cache-only containers, and a drive-wide search finds no `*.h4cr` anywhere
//!   (spool trim is on by default). The harness therefore synthesizes a stream
//!   with the same record framing the collector writes
//!   (`collector::attach_log::append_compact_frame_end_record`) over two real
//!   production domains (18 `country_identity`, 13 `tech_live`), and checks that
//!   the clean base stream certifies and seals before injecting anything. The
//!   REAL captures are used for the container cells, which need no stream.
//!
//! # Reading the matrix
//!
//! A cell is PASS only when all of the following hold:
//!   * no hour outside the injury is LOST,
//!   * no hour outside the injury is CHANGED,
//!   * the injury itself is named by some accounting output, and
//!   * the seal gate still publishes the recording.
//! Anything else is FAIL, and the row says which of the four failed.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use compact_contract::calendar::DATE_EPOCH_CONST;
use compact_contract::campaign_identity::{
    encode_cid1, Cid1Fields, Cid1Observation, Cid1Witnesses, COMPACT_DOMAIN_STREAM_HEADER,
    COMPACT_RECORD_TYPE_STREAM_HEADER,
};
use compact_contract::stream_validator::StreamIdentityMode;
use compact_contract::{
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_GAP, COMPACT_RECORD_TYPE_FRAME_END,
    COMPACT_RECORD_TYPE_FRAME_REFUSED, COMPACT_RECORD_VERSION, COMPACT_STATUS_DELTA_DATA,
};
use recorder_host::stream_fold::{
    drive_fold_with_identity_mode, strict_stream_certification_verdict,
    validate_excluded_domain_data_accounting, verify_stream_logical_boundary_with_identity_mode,
};
use recorder_host::world_fold::WorldFold;

// ---------------------------------------------------------------------------
// Stream construction — the same 24-byte H4CR record envelope the collector
// writes. Positions are recorded as they are emitted so an injection can target
// one game hour precisely.
// ---------------------------------------------------------------------------

const DOMAIN_FRAME: u16 = 1000;
const D18_COUNTRY_IDENTITY: u16 = 18;
const D13_TECH_LIVE: u16 = 13;
const CAMPAIGN_UUID: &str = "a4b17fed-ed12-4361-9064-fb6576638110";
const BASE_HOUR: i64 = 16_959_360;
/// Nine hours: three injection positions (early=1, middle=4, late=7) with clean
/// hours on both sides of each, so collateral in either direction is visible.
const HOURS: usize = 9;
const EARLY: usize = 1;
const MIDDLE: usize = 4;
const LATE: usize = 7;

fn raw_for(hour_index: i64) -> i32 {
    DATE_EPOCH_CONST + hour_index as i32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RecordSpan {
    /// Ordinal of the game hour this record belongs to (0-based).
    hour: usize,
    offset: usize,
    len: usize,
    domain: u16,
    ty: u16,
}

/// A base stream plus the map of where each record landed.
#[derive(Debug, Clone)]
struct BuiltStream {
    bytes: Vec<u8>,
    spans: Vec<RecordSpan>,
}

impl BuiltStream {
    fn records_of(&self, hour: usize) -> Vec<RecordSpan> {
        self.spans.iter().copied().filter(|s| s.hour == hour).collect()
    }

    fn first_data_of(&self, hour: usize, domain: u16) -> RecordSpan {
        self.spans
            .iter()
            .copied()
            .find(|s| s.hour == hour && s.domain == domain && s.ty == COMPACT_RECORD_TYPE_DOMAIN_DATA)
            .unwrap_or_else(|| panic!("hour {hour} has no DOMAIN_DATA record for domain {domain}"))
    }

    fn frame_end_of(&self, hour: usize) -> RecordSpan {
        self.spans
            .iter()
            .copied()
            .find(|s| s.hour == hour && s.ty == COMPACT_RECORD_TYPE_FRAME_END)
            .unwrap_or_else(|| panic!("hour {hour} has no FRAME_END"))
    }
}

struct StreamWriter {
    bytes: Vec<u8>,
    spans: Vec<RecordSpan>,
    seq: u64,
}

impl StreamWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            spans: Vec::new(),
            seq: 0,
        }
    }

    fn push(&mut self, hour: usize, game_hour_raw: i32, domain: u16, ty: u16, payload: &[u8]) {
        let seq = self.seq;
        self.seq += 1;
        self.push_with_seq(hour, seq, game_hour_raw, domain, ty, payload);
    }

    fn push_with_seq(
        &mut self,
        hour: usize,
        seq: u64,
        game_hour_raw: i32,
        domain: u16,
        ty: u16,
        payload: &[u8],
    ) {
        let offset = self.bytes.len();
        write_record(&mut self.bytes, seq, game_hour_raw, domain, ty, payload);
        self.spans.push(RecordSpan {
            hour,
            offset,
            len: self.bytes.len() - offset,
            domain,
            ty,
        });
    }

    fn finish(self) -> BuiltStream {
        BuiltStream {
            bytes: self.bytes,
            spans: self.spans,
        }
    }
}

fn write_record(out: &mut Vec<u8>, seq: u64, game_hour_raw: i32, domain: u16, ty: u16, payload: &[u8]) {
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

/// Domain 18 `country_identity`: 20-byte header (`row_count` at 8,
/// `status_flags` at 16) then `tag_db_id: u32`, length-prefixed tag, cosmetic
/// marker.
fn d18_payload(rows: &[(u32, &str)], status_flags: u32) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0u64.to_le_bytes());
    p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    p.extend_from_slice(&status_flags.to_le_bytes());
    for (id, tag) in rows {
        p.extend_from_slice(&id.to_le_bytes());
        p.extend_from_slice(&(tag.len() as u16).to_le_bytes());
        p.extend_from_slice(tag.as_bytes());
        p.push(1); // cosmetic marker: null
    }
    p
}

/// Domain 13 `tech_live`: 32-byte header (`country_count` at 8, `status_flags`
/// at 16, `active_count` at 28) then 16-byte country rows. A DELTA record
/// carries only the countries whose bytes changed — which is exactly what makes
/// a dropped delta a chain break rather than a one-hour hole.
fn d13_payload(countries: &[(u32, u32, u32)], status_flags: u32) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0u64.to_le_bytes()); //  0.. 8 elapsed_wall_ms
    p.extend_from_slice(&(countries.len() as u32).to_le_bytes()); //  8..12 country_count
    p.extend_from_slice(&(countries.len() as u32).to_le_bytes()); // 12..16 changed_rows
    p.extend_from_slice(&status_flags.to_le_bytes()); // 16..20 status_flags
    p.extend_from_slice(&recorder_host::domain_data_authority::COMPACT_TECH_PROGRESS_SCALE_RAW.to_le_bytes());
    p.extend_from_slice(
        &recorder_host::domain_data_authority::COMPACT_TECH_LIVE_PROGRESS_SCALE_RAW.to_le_bytes(),
    );
    p.extend_from_slice(&0u32.to_le_bytes()); // 28..32 active_count
    for (tag, completed, slots) in countries {
        p.extend_from_slice(&tag.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // row flags (0b111 vocabulary)
        p.extend_from_slice(&completed.to_le_bytes());
        p.extend_from_slice(&slots.to_le_bytes());
    }
    p
}

/// The record the production collector emits when a scheduled domain produced
/// no payload for an hour — including the `ReaderSilent` gap the frame seal
/// mints for a domain that reported nothing at all. This is what a bad hour
/// looks like in production, as opposed to a record vanishing in transport.
fn domain_gap_payload(game_hour_raw: i32) -> Vec<u8> {
    compact_contract::domain_gap::DomainGapRecordV1 {
        part: compact_contract::domain_gap::DomainGapPart::Both,
        reason: compact_contract::domain_gap::DomainGapReason::SourceUnavailable,
        target_hour_raw: game_hour_raw,
        range_start_hour_raw: game_hour_raw,
        range_end_hour_raw: game_hour_raw,
        tick_before: 0,
        tick_after: 0,
        queue_capacity: 0,
        queue_depth: 0,
        queue_dropped: 0,
        outcome_ordinal: 0,
    }
    .encode()
    .expect("canonical domain gap encodes")
}

fn frame_end_payload() -> Vec<u8> {
    frame_end_payload_with_terminal_outcomes(0)
}

/// A frame seal declaring how many unique non-probe TERMINAL OUTCOMES the hour
/// carried. `DOMAIN_GAP` is one, so an hour that gaps a domain must say so here
/// or the boundary tracker refuses the seal.
fn frame_end_payload_with_terminal_outcomes(count: u32) -> Vec<u8> {
    let mut p = vec![0u8; 32];
    p[16..20].copy_from_slice(&count.to_le_bytes());
    p[20..24].copy_from_slice(&count.to_le_bytes());
    p
}

fn cid1_record_payload() -> Vec<u8> {
    let observation = Cid1Observation {
        game_unique_id: CAMPAIGN_UUID.to_string(),
        game_unique_seed: 1_026_046_527,
        game_hour_raw: raw_for(BASE_HOUR),
    };
    let fields = Cid1Fields::from_stable_observations(
        observation.clone(),
        observation,
        "vanilla_latest_2026_07_20".to_string(),
        Cid1Witnesses::default(),
    )
    .expect("CID1 fields from stable observations");
    encode_cid1(&fields).expect("encode CID1")
}

/// The clean base stream: CID1 header, then `HOURS` game hours, each carrying a
/// d18 full record and a d13 record (hour 0 whole = the delta baseline, later
/// hours DELTA patches naming only the country that moved), closed by FRAME_END.
fn build_base_stream(identity: bool) -> BuiltStream {
    let mut w = StreamWriter::new();
    if identity {
        w.push_with_seq(
            0,
            0,
            raw_for(BASE_HOUR),
            COMPACT_DOMAIN_STREAM_HEADER,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            &cid1_record_payload(),
        );
        // The CID1 header is sequence 0; ordinary records start at 1.
        w.seq = 1;
    } else {
        w.seq = 1;
    }
    for hour in 0..HOURS {
        let raw = raw_for(BASE_HOUR + hour as i64);
        w.push(
            hour,
            raw,
            D18_COUNTRY_IDENTITY,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(&[(47, "GER"), (48, "FRA")], 0),
        );
        let tech = if hour == 0 {
            // The whole baseline both countries are seeded from.
            d13_payload(&[(47, 10, 3), (48, 20, 2)], 0)
        } else {
            // A patch naming ONLY the country that changed this hour.
            d13_payload(&[(47, 10 + hour as u32, 3)], COMPACT_STATUS_DELTA_DATA)
        };
        w.push(hour, raw, D13_TECH_LIVE, COMPACT_RECORD_TYPE_DOMAIN_DATA, &tech);
        w.push(
            hour,
            raw,
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
    }
    w.finish()
}

/// Rebuild the base stream while omitting selected records, RENUMBERING the
/// sequence so the result is otherwise perfectly well-formed. This separates
/// "a record is missing" from "the sequence has a hole": the collector emits a
/// contiguous sequence, so a domain it could not read for an hour produces a
/// stream with no sequence gap at all.
fn build_stream_omitting(identity: bool, omit: &dyn Fn(usize, u16, u16) -> bool) -> BuiltStream {
    let mut w = StreamWriter::new();
    if identity {
        w.push_with_seq(
            0,
            0,
            raw_for(BASE_HOUR),
            COMPACT_DOMAIN_STREAM_HEADER,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            &cid1_record_payload(),
        );
    }
    w.seq = 1;
    for hour in 0..HOURS {
        let raw = raw_for(BASE_HOUR + hour as i64);
        if !omit(hour, D18_COUNTRY_IDENTITY, COMPACT_RECORD_TYPE_DOMAIN_DATA) {
            w.push(
                hour,
                raw,
                D18_COUNTRY_IDENTITY,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d18_payload(&[(47, "GER"), (48, "FRA")], 0),
            );
        }
        if !omit(hour, D13_TECH_LIVE, COMPACT_RECORD_TYPE_DOMAIN_DATA) {
            let tech = if hour == 0 {
                d13_payload(&[(47, 10, 3), (48, 20, 2)], 0)
            } else {
                d13_payload(&[(47, 10 + hour as u32, 3)], COMPACT_STATUS_DELTA_DATA)
            };
            w.push(hour, raw, D13_TECH_LIVE, COMPACT_RECORD_TYPE_DOMAIN_DATA, &tech);
        }
        if !omit(hour, DOMAIN_FRAME, COMPACT_RECORD_TYPE_FRAME_END) {
            w.push(
                hour,
                raw,
                DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &frame_end_payload(),
            );
        }
    }
    w.finish()
}

/// The production-shaped bad hour: hour `gap_hour` reports a canonical
/// `DOMAIN_GAP` for d13 in place of its `DOMAIN_DATA`, exactly as the collector
/// does when a reader produces no payload. Sequence stays contiguous.
fn build_stream_with_domain_gap(identity: bool, gap_hour: usize) -> Vec<u8> {
    let mut w = StreamWriter::new();
    if identity {
        w.push_with_seq(
            0,
            0,
            raw_for(BASE_HOUR),
            COMPACT_DOMAIN_STREAM_HEADER,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            &cid1_record_payload(),
        );
    }
    w.seq = 1;
    for hour in 0..HOURS {
        let raw = raw_for(BASE_HOUR + hour as i64);
        w.push(
            hour,
            raw,
            D18_COUNTRY_IDENTITY,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(&[(47, "GER"), (48, "FRA")], 0),
        );
        let gapped = hour == gap_hour;
        if gapped {
            w.push(
                hour,
                raw,
                D13_TECH_LIVE,
                COMPACT_RECORD_TYPE_DOMAIN_GAP,
                &domain_gap_payload(raw),
            );
        } else {
            let tech = if hour == 0 {
                d13_payload(&[(47, 10, 3), (48, 20, 2)], 0)
            } else {
                d13_payload(&[(47, 10 + hour as u32, 3)], COMPACT_STATUS_DELTA_DATA)
            };
            w.push(hour, raw, D13_TECH_LIVE, COMPACT_RECORD_TYPE_DOMAIN_DATA, &tech);
        }
        w.push(
            hour,
            raw,
            DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload_with_terminal_outcomes(if gapped { 1 } else { 0 }),
        );
    }
    w.finish().bytes
}

// ---------------------------------------------------------------------------
// Running the real production paths
// ---------------------------------------------------------------------------

/// Everything the chain says about one stream.
struct Observation {
    /// hour_index -> canonical snapshot SHA-256, as the decoder published them.
    hours: BTreeMap<i64, String>,
    /// hour_index -> the canonical snapshot bytes, kept so a cell can show WHY
    /// an hour changed instead of only that it did.
    snapshots: BTreeMap<i64, Vec<u8>>,
    /// Per-domain DOMAIN_DATA record counts the fold actually accepted.
    per_domain: BTreeMap<u16, u64>,
    decoder_stop_reason: Option<String>,
    decoder_boundary_error: Option<String>,
    decoder_truncated_tail: u64,
    decoder_certification: Result<(), String>,
    /// The production streaming fold's own verdict (the `fold_stats` seal arm).
    fold_seal_gate: Result<(), String>,
    /// The skip-fold seal arm.
    boundary_walk: Result<(), String>,
    /// Every accounting string a consumer could read the injury out of.
    named_reasons: Vec<String>,
}

fn observe(bytes: &[u8], identity: bool, scratch: &Path, tag: &str) -> Observation {
    let mode = if identity {
        StreamIdentityMode::IdentityV1
    } else {
        StreamIdentityMode::Legacy
    };

    // --- production replay decoder -----------------------------------------
    let mut cursor = replay_decoder::open_reader_with_identity_mode(
        Cursor::new(bytes.to_vec()),
        bytes.len() as u64,
        "gq_bad_hour",
        mode,
    );
    let mut hours = BTreeMap::new();
    let mut snapshots = BTreeMap::new();
    let mut decode_io_error = None;
    loop {
        match cursor.next_hour() {
            None => break,
            Some(Ok(hour)) => {
                hours.insert(hour.hour_index, hour.snapshot_sha256());
                snapshots.insert(hour.hour_index, hour.snapshot_bytes().to_vec());
            }
            Some(Err(e)) => {
                decode_io_error = Some(e.to_string());
                break;
            }
        }
    }
    let stats = cursor.stats().clone();

    let mut named_reasons = Vec::new();
    if let Some(e) = &decode_io_error {
        named_reasons.push(format!("decoder_error={e}"));
    }
    for excluded in &stats.excluded_domain_data_records {
        named_reasons.push(format!(
            "excluded_domain_data hour_raw={} domain={} reason={}",
            excluded.game_hour_raw, excluded.domain_id, excluded.validation_reason
        ));
    }

    // --- production streaming fold (the seal's `fold_stats` source) ---------
    let mut fold = WorldFold::new("gq_bad_hour".to_string());
    let fold_outcome =
        drive_fold_with_identity_mode(Cursor::new(bytes.to_vec()), bytes.len() as u64, &mut fold, mode);
    let fold_seal_gate = match fold_outcome {
        Err(e) => Err(format!("fold driver io error: {e}")),
        Ok(outcome) => {
            let s = &outcome.stats;
            for (domain, m) in &s.malformed_payload_domains {
                named_reasons.push(format!(
                    "malformed_payload domain={domain} hour_raw={} reason={}",
                    m.first_game_hour_raw, m.first_reason
                ));
            }
            validate_excluded_domain_data_accounting(
                s.malformed_payload_records,
                &s.excluded_domain_data_records,
            )
            .and_then(|()| {
                strict_stream_certification_verdict(
                    s.stop_reason.as_deref(),
                    s.truncated_tail_bytes,
                    s.ends_at_frame_end,
                    None,
                )
            })
        }
    };

    // --- the skip-fold seal arm, over a real file --------------------------
    let path = scratch.join(format!("{tag}.h4cr"));
    fs::write(&path, bytes).expect("write injected stream");
    let boundary_walk = match verify_stream_logical_boundary_with_identity_mode(&path, mode) {
        Ok(excluded) => {
            for e in &excluded {
                named_reasons.push(format!(
                    "seal_walk_excluded hour_raw={} domain={} reason={}",
                    e.game_hour_raw, e.domain_id, e.validation_reason
                ));
            }
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    };

    if let Some(r) = &stats.stop_reason {
        named_reasons.push(format!("stop_reason={r}"));
    }
    if let Some(r) = &stats.boundary_error {
        named_reasons.push(format!("boundary_error={r}"));
    }

    Observation {
        hours,
        snapshots,
        per_domain: stats.per_domain_data_records.clone(),
        decoder_stop_reason: stats.stop_reason.clone(),
        decoder_boundary_error: stats.boundary_error.clone(),
        decoder_truncated_tail: stats.truncated_tail_bytes,
        decoder_certification: stats.certification_verdict(),
        fold_seal_gate,
        boundary_walk,
        named_reasons,
    }
}

// ---------------------------------------------------------------------------
// Verdicts
// ---------------------------------------------------------------------------

struct Row {
    cell: String,
    position: String,
    /// Did the injection change ANYTHING about the injured hours? A cell whose
    /// injury is invisible is measuring nothing, and is reported as INERT
    /// rather than counted as a pass or a fail.
    injury_observable: bool,
    injured: BTreeSet<i64>,
    delivered: usize,
    lost_collateral: Vec<i64>,
    changed_collateral: Vec<i64>,
    injury_named: bool,
    seal: String,
    verdict: String,
    detail: String,
}

fn hour_of(ordinal: usize) -> i64 {
    BASE_HOUR + ordinal as i64
}

fn judge(
    cell: &str,
    position: &str,
    injured_ordinals: &[usize],
    base: &Observation,
    injected: &Observation,
) -> Row {
    let injured: BTreeSet<i64> = injured_ordinals.iter().map(|o| hour_of(*o)).collect();

    // Was the injury visible at all? Either an injured hour vanished, or it was
    // published with different bytes, or the chain refused something it did not
    // refuse on the clean base.
    let injury_observable = injured.iter().any(|hour| {
        match (base.hours.get(hour), injected.hours.get(hour)) {
            (Some(_), None) => true,
            (Some(a), Some(b)) => a != b,
            _ => false,
        }
    }) || injected.decoder_certification.is_err()
        || injected.fold_seal_gate.is_err()
        || injected.boundary_walk.is_err();

    let mut lost_collateral = Vec::new();
    // An uninjured hour whose bytes changed splits in two, and the difference
    // is the whole argument. If the hour also gained a complaint in its own
    // `provenance` block, it is a hour that KNOWS it is degraded — fail-closed,
    // which the directive asks for. If it changed while still claiming to be
    // clean, a consumer cannot tell it apart from good data: that is the silent
    // corruption the directive forbids.
    let mut changed_collateral = Vec::new();
    let mut degraded_named_collateral = Vec::new();
    for (hour, sha) in &base.hours {
        if injured.contains(hour) {
            continue;
        }
        match injected.hours.get(hour) {
            None => lost_collateral.push(*hour),
            Some(other) if other != sha => {
                let before = base
                    .snapshots
                    .get(hour)
                    .map(|b| provenance_complaint(b))
                    .unwrap_or_default();
                let after = injected
                    .snapshots
                    .get(hour)
                    .map(|b| provenance_complaint(b))
                    .unwrap_or_default();
                if !after.is_empty() && after != before {
                    degraded_named_collateral.push(*hour);
                } else {
                    changed_collateral.push(*hour);
                }
            }
            Some(_) => {}
        }
    }

    // The injury is NAMED when some accounting string mentions one of the
    // injured hours (by raw date) or when the injured hour was published with
    // an explicitly different — i.e. degraded/refused — snapshot that the fold
    // recorded a reason for. A bare absence with no reason anywhere is not
    // named: that is the "silent loss" the directive forbids.
    let injured_raws: Vec<String> = injured
        .iter()
        .map(|h| raw_for(*h).to_string())
        .collect();
    let named_in_stream_accounting = injected.named_reasons.iter().any(|reason| {
        injured_raws.iter().any(|raw| reason.contains(raw.as_str()))
    });
    // The fold's OWN per-hour channel counts too: `provenance.collector`
    // carries `domain_problems`, `unfolded_domains` and `notes`, and a replay
    // consumer reads them out of the very hour they describe. An injured hour
    // that ships with a complaint there is named, even when nothing at the
    // stream level mentions it.
    let named_in_published_hour = injured.iter().any(|hour| {
        let Some(bytes) = injected.snapshots.get(hour) else {
            return false;
        };
        let base_complaint = base
            .snapshots
            .get(hour)
            .map(|b| provenance_complaint(b))
            .unwrap_or_default();
        let complaint = provenance_complaint(bytes);
        !complaint.is_empty() && complaint != base_complaint
    });
    let injury_named = named_in_stream_accounting || named_in_published_hour;

    let seal = match (&injected.fold_seal_gate, &injected.boundary_walk) {
        (Ok(()), Ok(())) => "PUBLISHES".to_string(),
        (Err(e), _) => format!("REFUSES ({})", first_clause(e)),
        (_, Err(e)) => format!("REFUSES-walk ({})", first_clause(e)),
    };

    let mut failures = Vec::new();
    if !lost_collateral.is_empty() {
        failures.push(format!("collateral loss {lost_collateral:?}"));
    }
    if !changed_collateral.is_empty() {
        failures.push(format!(
            "SILENT collateral change {changed_collateral:?}"
        ));
    }
    if !injury_named {
        failures.push("injury unnamed".to_string());
    }
    if !matches!(
        (&injected.fold_seal_gate, &injected.boundary_walk),
        (Ok(()), Ok(()))
    ) {
        failures.push("whole-run seal refusal".to_string());
    }
    let verdict = if !injury_observable {
        "INERT".to_string()
    } else if failures.is_empty() {
        "PASS".to_string()
    } else {
        "FAIL".to_string()
    };

    let mut detail = failures.join("; ");
    if detail.is_empty() {
        detail = "loss confined to the injured hour and named".to_string();
    }
    let injured_outcome: Vec<String> = injured
        .iter()
        .map(|hour| {
            let ord = hour - BASE_HOUR;
            match (base.hours.get(hour), injected.hours.get(hour)) {
                (Some(_), None) => format!("h{ord}:absent"),
                (Some(a), Some(b)) if a != b => format!("h{ord}:changed"),
                (Some(_), Some(_)) => format!("h{ord}:UNCHANGED"),
                _ => format!("h{ord}:?"),
            }
        })
        .collect();
    detail.push_str(&format!(" | injured hours {}", injured_outcome.join(",")));
    if !degraded_named_collateral.is_empty() {
        detail.push_str(&format!(
            " | fail-closed degraded (named) {:?}",
            degraded_named_collateral
                .iter()
                .map(|h| h - BASE_HOUR)
                .collect::<Vec<_>>()
        ));
    }
    if let Some(stop) = &injected.decoder_stop_reason {
        detail.push_str(&format!(" | decoder stop: {}", first_clause(stop)));
    }
    if let Some(b) = &injected.decoder_boundary_error {
        detail.push_str(&format!(" | boundary: {}", first_clause(b)));
    }
    if injected.decoder_truncated_tail != 0 {
        detail.push_str(&format!(
            " | torn tail {} B",
            injected.decoder_truncated_tail
        ));
    }
    if let Err(e) = &injected.decoder_certification {
        detail.push_str(&format!(" | certification: {}", first_clause(e)));
    }

    Row {
        cell: cell.to_string(),
        position: position.to_string(),
        injury_observable,
        injured,
        delivered: injected.hours.len(),
        lost_collateral,
        changed_collateral,
        injury_named,
        seal,
        verdict,
        detail,
    }
}

/// The fold's per-hour complaint channel, as a comparable string: the
/// `provenance.collector` fields a replay consumer reads to learn that an hour
/// is degraded. Empty means the hour claims to be clean.
fn provenance_complaint(snapshot: &[u8]) -> String {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(snapshot) else {
        return String::new();
    };
    let collector = value
        .get("provenance")
        .and_then(|p| p.get("collector"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let mut parts = Vec::new();
    for field in ["domain_problems", "unfolded_domains", "late_records_dropped"] {
        if let Some(arr) = collector.get(field).and_then(|v| v.as_array()) {
            if !arr.is_empty() {
                parts.push(format!("{field}={arr:?}"));
            }
        }
    }
    for field in ["notes"] {
        if let Some(arr) = value
            .get("provenance")
            .and_then(|p| p.get(field))
            .and_then(|v| v.as_array())
        {
            if !arr.is_empty() {
                parts.push(format!("{field}={arr:?}"));
            }
        }
    }
    parts.join("; ")
}

fn first_clause(s: &str) -> String {
    let trimmed = s.trim();
    let cut = trimmed.find(';').unwrap_or(trimmed.len());
    let clause = &trimmed[..cut];
    if clause.len() > 120 {
        format!("{}…", &clause[..120])
    } else {
        clause.to_string()
    }
}

// ---------------------------------------------------------------------------
// Injections
// ---------------------------------------------------------------------------

fn inject_truncate_mid_record(base: &BuiltStream, hour: usize) -> Vec<u8> {
    let span = base.first_data_of(hour, D18_COUNTRY_IDENTITY);
    base.bytes[..span.offset + COMPACT_RECORD_HEADER_LEN + 3].to_vec()
}

/// Cut the file at a whole-record boundary, immediately BEFORE the hour's
/// `FRAME_END`. Every record is intact; the hour is simply left open at EOF —
/// the shape a recorder that died between the last domain read and the frame
/// seal leaves behind.
fn inject_truncate_at_boundary(base: &BuiltStream, hour: usize) -> Vec<u8> {
    let span = base.frame_end_of(hour);
    base.bytes[..span.offset].to_vec()
}

fn inject_payload_bitflip(base: &BuiltStream, hour: usize) -> Vec<u8> {
    let span = base.first_data_of(hour, D18_COUNTRY_IDENTITY);
    let mut bytes = base.bytes.clone();
    // The row's tag-length field: a flip here makes the payload structurally
    // impossible to consume exactly, which is what a bit rot in a body looks
    // like to the exact decoder.
    let at = span.offset + COMPACT_RECORD_HEADER_LEN + 24;
    bytes[at] ^= 0x40;
    bytes
}

fn inject_framing_corruption(base: &BuiltStream, hour: usize) -> Vec<u8> {
    let span = base.first_data_of(hour, D18_COUNTRY_IDENTITY);
    let mut bytes = base.bytes.clone();
    bytes[span.offset + 1] ^= 0x20; // record magic
    bytes
}

fn inject_length_corruption(base: &BuiltStream, hour: usize) -> Vec<u8> {
    let span = base.first_data_of(hour, D18_COUNTRY_IDENTITY);
    let mut bytes = base.bytes.clone();
    // payload_len lives at header offset 18..22.
    let at = span.offset + 18;
    let corrupted = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) ^ 0x0000_0010;
    bytes[at..at + 4].copy_from_slice(&corrupted.to_le_bytes());
    bytes
}

fn inject_missing_hour(base: &BuiltStream, hour: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    let spans = base.records_of(hour);
    let start = spans.first().expect("hour has records").offset;
    let end = spans.last().map(|s| s.offset + s.len).expect("hour has records");
    bytes.extend_from_slice(&base.bytes[..start]);
    bytes.extend_from_slice(&base.bytes[end..]);
    bytes
}

fn inject_refused_frame(_base: &BuiltStream, identity: bool, hour: usize) -> Vec<u8> {
    // Rebuild so the sequence stays contiguous: the collector that refuses a
    // frame still writes one boundary record for the hour.
    let mut w = StreamWriter::new();
    if identity {
        w.push_with_seq(
            0,
            0,
            raw_for(BASE_HOUR),
            COMPACT_DOMAIN_STREAM_HEADER,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            &cid1_record_payload(),
        );
    }
    w.seq = 1;
    for h in 0..HOURS {
        let raw = raw_for(BASE_HOUR + h as i64);
        w.push(
            h,
            raw,
            D18_COUNTRY_IDENTITY,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(&[(47, "GER"), (48, "FRA")], 0),
        );
        let tech = if h == 0 {
            d13_payload(&[(47, 10, 3), (48, 20, 2)], 0)
        } else {
            d13_payload(&[(47, 10 + h as u32, 3)], COMPACT_STATUS_DELTA_DATA)
        };
        w.push(h, raw, D13_TECH_LIVE, COMPACT_RECORD_TYPE_DOMAIN_DATA, &tech);
        if h == hour {
            w.push(
                h,
                raw,
                DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_REFUSED,
                &frame_refused_payload(raw),
            );
        } else {
            w.push(
                h,
                raw,
                DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &frame_end_payload(),
            );
        }
    }
    w.finish().bytes
}

fn frame_refused_payload(game_hour_raw: i32) -> Vec<u8> {
    use compact_contract::diagnostic_channel::{Authority, Predicate, TruncationAccount};
    use compact_contract::frame_refusal::{FrameRefusalOffenderV2, FrameRefusalRecordV2};
    FrameRefusalRecordV2 {
        refused_hour: game_hour_raw,
        frame_findings: Vec::new(),
        offenders: vec![FrameRefusalOffenderV2 {
            domain_id: D13_TECH_LIVE,
            status_flags: compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
            non_tolerated_flags: compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
            predicate: Predicate::undeclared(),
            authority: Authority::SelfDomain,
            locator: Vec::new(),
            units: Vec::new(),
            truncation: TruncationAccount::complete(0),
            off_frame: None,
            reason: "d13_source_unavailable=1".to_string(),
            reason_units_named: 0,
        }],
    }
    .encode()
    .expect("canonical v2 refusal encodes")
}

// ---------------------------------------------------------------------------
// Container cells, over the real retained captures
// ---------------------------------------------------------------------------

fn container_cells(scratch: &Path, out: &mut Vec<String>) {
    let candidates = [
        "runs/live_unlock/t48_authoritative/native_session.battleplan",
        "runs/live_unlock/t49_pricing/native_session.battleplan",
    ];
    let native_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("native root")
        .to_path_buf();
    let source = candidates
        .iter()
        .map(|c| native_root.join(c))
        .find(|p| p.exists());
    let Some(source) = source else {
        out.push("| (g) container entry corruption | n/a | NOT RUN — no retained capture container in this worktree |".to_string());
        return;
    };

    let clean = scratch.join("container_clean.battleplan");
    fs::copy(&source, &clean).expect("copy real container");
    let clean_verdict = verify_container(&clean);
    out.push(format!(
        "| (g0) real container, untouched | — | {} |",
        describe(&clean_verdict)
    ));

    for (label, mutate) in [
        (
            "(g1) one payload entry byte corrupted",
            corrupt_container_entry as fn(&Path, &Path) -> Result<String, String>,
        ),
        ("(g2) manifest digest mismatch", corrupt_container_manifest),
    ] {
        let target = scratch.join(format!("container_{}.battleplan", label.len()));
        fs::copy(&source, &target).expect("copy real container");
        match mutate(&target, &target) {
            Err(e) => out.push(format!("| {label} | — | NOT RUN — {e} |")),
            Ok(what) => {
                let verdict = verify_container(&target);
                out.push(format!("| {label} ({what}) | — | {} |", describe(&verdict)));
            }
        }
    }
}

fn verify_container(path: &Path) -> Result<(), String> {
    match snapshot::container::Container::open(path, &snapshot::container::ExpectedIdentity::default()) {
        Err(e) => Err(format!("open: {e}")),
        Ok(container) => container
            .verify_all_streaming()
            .map_err(|e| format!("verify_all: {e}")),
    }
}

fn describe(result: &Result<(), String>) -> String {
    match result {
        Ok(()) => "container gate ACCEPTS".to_string(),
        Err(e) => format!("container gate REFUSES — {}", first_clause(e)),
    }
}

/// Flip one byte inside the LARGEST payload entry's compressed bytes, in place,
/// leaving the ZIP structure intact. This is bit rot in one entry.
fn corrupt_container_entry(path: &Path, _out: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    // Corrupt deep inside the file, well past the first local header, so the
    // damage lands in the largest stored entry's data.
    let at = bytes.len() / 2;
    let mut bytes = bytes;
    bytes[at] ^= 0xFF;
    fs::write(path, &bytes).map_err(|e| e.to_string())?;
    Ok(format!("byte {at}"))
}

/// Flip one byte inside `manifest.json`'s bytes (it is STORED, not deflated, so
/// the flip lands in readable JSON).
fn corrupt_container_manifest(path: &Path, _out: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let needle = b"\"sha256\"";
    let at = bytes
        .windows(needle.len())
        .rposition(|w| w == needle)
        .ok_or_else(|| "no sha256 field found in the tail of the container".to_string())?;
    // Move onto the first hex digit of the value and change it.
    let value = bytes[at..]
        .iter()
        .position(|b| b.is_ascii_hexdigit() && *b != b'2' && *b != b'5' && *b != b'6')
        .map(|o| at + o)
        .ok_or_else(|| "no digest value after the sha256 key".to_string())?;
    let mut bytes = bytes;
    bytes[value] = if bytes[value] == b'a' { b'b' } else { b'a' };
    fs::write(path, &bytes).map_err(|e| e.to_string())?;
    Ok(format!("digest byte {value}"))
}

// ---------------------------------------------------------------------------

fn main() {
    let scratch = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: gq_bad_hour <scratch-dir>");
    fs::create_dir_all(&scratch).expect("scratch dir");

    let identity = true;
    let base = build_base_stream(identity);
    let base_obs = observe(&base.bytes, identity, &scratch, "base");

    let mut lines = Vec::new();
    lines.push(format!(
        "Base stream: {} bytes, {} hours, {} records. Decoder delivered {} hours.",
        base.bytes.len(),
        HOURS,
        base.spans.len(),
        base_obs.hours.len()
    ));
    lines.push(format!(
        "Base certification: {:?}; seal gate: {:?}; boundary walk: {:?}",
        base_obs.decoder_certification, base_obs.fold_seal_gate, base_obs.boundary_walk
    ));
    lines.push(format!(
        "Base per-domain DOMAIN_DATA records accepted: {:?}",
        base_obs.per_domain
    ));
    for (hour, bytes) in &base_obs.snapshots {
        fs::write(scratch.join(format!("base_h{}.json", hour - BASE_HOUR)), bytes)
            .expect("dump base snapshot");
    }
    for reason in &base_obs.named_reasons {
        lines.push(format!("Base accounting: {reason}"));
    }
    // Prove the two domains actually reach the published world state, so a cell
    // that changes them is changing something a consumer can see.
    for (probe, label) in [
        ("\"tech_count\"", "d13 tech_live"),
        ("\"GER\"", "d18 country_identity"),
    ] {
        let hits = base_obs
            .snapshots
            .values()
            .filter(|bytes| {
                String::from_utf8_lossy(bytes).contains(probe)
            })
            .count();
        lines.push(format!(
            "Base snapshots containing {label} evidence ({probe}): {hits}/{}",
            base_obs.snapshots.len()
        ));
    }
    // A base with ANY accounting complaint is not a control: every "injury
    // named" answer below would be contaminated by a reason that was there
    // before the injection.
    if base_obs.hours.len() != HOURS
        || base_obs.decoder_certification.is_err()
        || base_obs.fold_seal_gate.is_err()
        || base_obs.boundary_walk.is_err()
        || !base_obs.named_reasons.is_empty()
    {
        lines.push(
            "BASE STREAM IS NOT CLEAN — every row below is uninterpretable. Reasons:".to_string(),
        );
        for r in &base_obs.named_reasons {
            lines.push(format!("  {r}"));
        }
        println!("{}", lines.join("\n"));
        std::process::exit(2);
    }

    let mut rows: Vec<Row> = Vec::new();
    let positions = [("early", EARLY), ("middle", MIDDLE), ("late", LATE)];

    for (name, hour) in positions {
        // (a) truncated hour — the stream ends mid-record inside the hour.
        let injured: Vec<usize> = (hour..HOURS).collect();
        let bytes = inject_truncate_mid_record(&base, hour);
        rows.push(judge(
            "(a) truncated hour (ends mid-record)",
            name,
            &injured,
            &base_obs,
            &observe(&bytes, identity, &scratch, &format!("a_{name}")),
        ));

        // (b) corrupt record body.
        let bytes = inject_payload_bitflip(&base, hour);
        rows.push(judge(
            "(b) corrupt record body (payload bit flip)",
            name,
            &[hour],
            &base_obs,
            &observe(&bytes, identity, &scratch, &format!("b_{name}")),
        ));

        // (c1) corrupt record magic.
        let bytes = inject_framing_corruption(&base, hour);
        rows.push(judge(
            "(c1) corrupt record framing (magic)",
            name,
            &[hour],
            &base_obs,
            &observe(&bytes, identity, &scratch, &format!("c1_{name}")),
        ));

        // (c2) corrupt record length.
        let bytes = inject_length_corruption(&base, hour);
        rows.push(judge(
            "(c2) corrupt record framing (length)",
            name,
            &[hour],
            &base_obs,
            &observe(&bytes, identity, &scratch, &format!("c2_{name}")),
        ));

        // (d) missing hour entirely.
        let bytes = inject_missing_hour(&base, hour);
        rows.push(judge(
            "(d) missing hour (records absent, sequence hole)",
            name,
            &[hour],
            &base_obs,
            &observe(&bytes, identity, &scratch, &format!("d_{name}")),
        ));

        // (e) refused frame hour.
        let bytes = inject_refused_frame(&base, identity, hour);
        rows.push(judge(
            "(e) refused frame hour (FRAME_REFUSED v2)",
            name,
            &[hour],
            &base_obs,
            &observe(&bytes, identity, &scratch, &format!("e_{name}")),
        ));

        // (f1) delta-chain break: one delta dropped, sequence renumbered.
        let stream = build_stream_omitting(identity, &|h, d, _t| h == hour && d == D13_TECH_LIVE);
        rows.push(judge(
            "(f1) delta dropped (later deltas depend on it)",
            name,
            &[hour],
            &base_obs,
            &observe(&stream.bytes, identity, &scratch, &format!("f1_{name}")),
        ));

        // (f3) THE PRODUCTION-SHAPED BAD HOUR: the domain reports a canonical
        // DOMAIN_GAP for this hour instead of data.
        let bytes = build_stream_with_domain_gap(identity, hour);
        rows.push(judge(
            "(f3) domain reports DOMAIN_GAP for one hour",
            name,
            &[hour],
            &base_obs,
            &observe(&bytes, identity, &scratch, &format!("f3_{name}")),
        ));

        // (h) abrupt end at a whole-record boundary: the hour is left OPEN.
        // Only that hour is injured; hours before it are complete and sealed.
        let bytes = inject_truncate_at_boundary(&base, hour);
        rows.push(judge(
            "(h) abrupt end, hour left open at EOF",
            name,
            &[hour],
            &base_obs,
            &observe(&bytes, identity, &scratch, &format!("h_{name}")),
        ));
    }

    // Dump the (f1) middle-position hours so the stale-hour claim can be read
    // out of the actual published bytes rather than inferred.
    {
        let stream = build_stream_omitting(identity, &|h, d, _t| h == MIDDLE && d == D13_TECH_LIVE);
        let obs = observe(&stream.bytes, identity, &scratch, "f1_dump");
        for (hour, bytes) in &obs.snapshots {
            fs::write(
                scratch.join(format!("f1_h{}.json", hour - BASE_HOUR)),
                bytes,
            )
            .expect("dump f1 snapshot");
        }
    }

    {
        let bytes = build_stream_with_domain_gap(identity, MIDDLE);
        let obs = observe(&bytes, identity, &scratch, "f3_dump");
        for (hour, b) in &obs.snapshots {
            fs::write(scratch.join(format!("f3_h{}.json", hour - BASE_HOUR)), b)
                .expect("dump f3 snapshot");
        }
    }

    // (f2) the keyframe itself is dropped — position-independent (hour 0).
    let stream = build_stream_omitting(identity, &|h, d, _t| h == 0 && d == D13_TECH_LIVE);
    rows.push(judge(
        "(f2) delta baseline (keyframe) dropped",
        "hour 0",
        &[0],
        &base_obs,
        &observe(&stream.bytes, identity, &scratch, "f2"),
    ));

    // --- render ------------------------------------------------------------
    lines.push(String::new());
    lines.push("| cell | position | injured | hours delivered | collateral lost | collateral changed | injury named | seal | verdict | detail |".to_string());
    lines.push("|---|---|---|---:|---|---|---|---|---|---|".to_string());
    for r in &rows {
        lines.push(format!(
            "| {} | {} | {:?} | {}/{} | {:?} | {:?} | {} | {} | **{}** | {} |",
            r.cell,
            r.position,
            r.injured.iter().map(|h| h - BASE_HOUR).collect::<Vec<_>>(),
            r.delivered,
            HOURS,
            r.lost_collateral
                .iter()
                .map(|h| h - BASE_HOUR)
                .collect::<Vec<_>>(),
            r.changed_collateral
                .iter()
                .map(|h| h - BASE_HOUR)
                .collect::<Vec<_>>(),
            if r.injury_named { "yes" } else { "NO" },
            r.seal,
            r.verdict,
            r.detail,
        ));
    }

    lines.push(String::new());
    lines.push("### Container cells (real retained capture)".to_string());
    lines.push(String::new());
    lines.push("| cell | position | result |".to_string());
    lines.push("|---|---|---|".to_string());
    let mut container_rows = Vec::new();
    container_cells(&scratch, &mut container_rows);
    lines.extend(container_rows);

    let pass = rows.iter().filter(|r| r.verdict == "PASS").count();
    lines.push(String::new());
    lines.push(format!("Stream cells: {pass} PASS / {} FAIL", rows.len() - pass));

    println!("{}", lines.join("\n"));
}
