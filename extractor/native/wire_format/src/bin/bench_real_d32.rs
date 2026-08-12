//! Measure the wire format against the shipping compact stream on REAL d32
//! `air_wing_loadouts` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d32-scheduled capture
//! (`runs/batch2/deepwar_t1`). The stream's own d32 DATA records are the shipping
//! format's cost for the domain: a 44-byte header, a fixed 32-byte wing row per
//! changed wing and a fixed 16-byte entry row per equipment line, exactly what
//! the recorder stores today. This tool replays those records into per-hour
//! states — the same fold `world_fold::fold_air_wing_loadouts` performs — and
//! re-encodes every hour with this crate's grammar: a BASELINE at each 48-hour
//! block start, field-granular stored-minimal DELTAs between, over BOTH
//! collections.
//!
//! **Nothing here re-derives a contract it can call.** Header, row and frame
//! decoding go through `wire_format::d32`'s public boundary — including
//! `d32::State::fold_payload`, because one d32 record is not one d32 state:
//! `replace_or_sparse_after_first` clears only on the first non-delta frame and
//! every frame after that is a sparse upsert into the retained accumulator
//! (`world_fold.rs:14895`, `:2927`). H4CR framing, record semantics and sequence
//! continuity go through `compact_contract`'s own parser and stateful validator.
//! A harness that reimplements either drifts from the recorder without anything
//! failing.
//!
//! **The whole stream is validated before any artifact is written.** A stream
//! that is truncated, spliced, or corrupt would otherwise yield a smaller but
//! entirely plausible measurement — a silently wrong number, which is worse than
//! a crash. `scan_stream` therefore enforces, fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`
//!   (magic, version, header length, payload bound);
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`
//!   (sequence continuity from the stream's first record, record-type and
//!   record-flag semantics, FRAME_END shape);
//! - the hour boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`), its DOMAIN_PERF hour binding
//!   (`:328`-`:365`), and the DATA arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal validation (`:46`-`:89`) and its distinct non-probe PERF
//!   domain-count cross-check (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. This crate cannot validate other domains' payloads, but it
//! replays d32's full status-independent payload authority.
//!
//! `decode_compact_air_wing_loadouts_data_payload`
//! (`domain_data_authority.rs:6579`-`:6823`) walks both row tables on every
//! record, regardless of status. The pre-exclusion `d32::peek_header` already
//! checks the shared header predicates. [`check_authority_payload`] adds the
//! authority's remaining non-delta count bound, exact table shape, bounded
//! identities, per-frame uniqueness, delete/live-field rules, declared equipment
//! coverage, entry bounds and uniqueness, and amount/reserved checks. It calls
//! d32's public row and range boundaries wherever those expose the same rule.
//!
//! The five-check sweep also made identity mode a stated input. Legacy remains
//! the default, preserving the existing real-capture measurement.
//!
//! **Two exclusion paths, at the fold's own position in its call order.**
//! `fold_air_wing_loadouts` validates its header counts, then returns without
//! folding on a failure status (`:14828`) and on a delta-before-baseline
//! (`:14837`), and only then checks the payload length and reads a row. So this
//! harness calls `d32::peek_header` first, excludes on
//! `compact_status_failure_flags_after_expected_partial_tolerance` — the
//! recorder's own predicate, never a hand-copied mask — and only then folds; the
//! delta-before-baseline path is `d32::FoldOutcome::DeltaBeforeBaseline`, decided
//! inside the shared fold. Hour uniqueness is checked over EVERY d32 DATA record
//! and BEFORE either exclusion, so a success + failure pair inside one hour
//! cannot slip through.
//!
//! **The cross-domain d30 orphan prune is asserted inert, not assumed inert.**
//! At FRAME_END, `prune_air_wing_loadout_orphans_for_current_hour`
//! (`world_fold.rs:5102`-`:5127`) removes every d32 key registered this hour that
//! d30's `air_wings` accumulator does not hold — a second writer of this domain's
//! accumulator. Modelling it fully would mean porting d30, which this assignment
//! does not; instead [`AirWingRoster`] reads only the d30 key columns the check
//! needs and the harness **fails** if the prune would ever drop a row, rather
//! than reporting a number for a state the recorder would not have held. The
//! model deliberately ignores the prune's three early-return guards
//! (`:5103`-`:5108`), which makes the assertion strictly stronger than the
//! recorder's own behaviour.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d32.bin   — the d32 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows per hour at stored
//! precision) for both collections.
//!
//! **Only d32 DATA payloads — plus the d30 subset below — are validated.**
//! Production runs `validate_domain_data_payload_at` on the DATA payload of
//! EVERY domain (`stream_fold.rs:593`); this harness cannot, because validating
//! another domain's payload means owning that domain's authority, and
//! `wire_format` must not depend on `recorder_host`. The one exception is d30,
//! whose key columns the orphan-prune guard must read: [`AirWingRoster`]
//! validates every d30 DATA record's header length, exact row-count length and
//! row flag words — on records of every status, mirroring the
//! status-independence of production's d30 authority
//! (`domain_data_authority.rs:6257`) — while everything d30 carries beyond
//! those columns (counts, scale, reserved words, per-row value fields) passes
//! unread. Every DATA record of any other domain passes with its H4CR header,
//! sequence continuity, hour-boundary, PERF-binding and FRAME_END rules checked
//! but its **payload bytes unread**.
//! The concrete hazard — a d32 record relabelled to another domain id, or
//! dropped outright, silently shrinking the measurement — is closed by an
//! explicit completeness check in `scan_stream`: every closed game hour must
//! carry exactly one d32 DATA record, of any status. That is a per-capture
//! assertion, not a grammar rule; a capture where d32 is not scheduled every
//! hour fails loudly there and a human adjusts.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_unknown_flags_v1,
    parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d32;

const D30: u16 = 30;
const D32: u16 = 32;
const BLOCK_HOURS: usize = 48;

/// Whether the wire arm opens a BASELINE for this measured hour.
///
/// Two reasons, and the second is not a cadence choice: the 48-hour block
/// boundary, or a measured hour that does not immediately follow the previous
/// measured one. An hour this domain did not successfully sample is coverage
/// the domain lost, which is §8's gap, and `WIRE_FORMAT.md:187`-`:190` is
/// explicit: "A gap affecting the state part forces the next state observation
/// to be a `BASELINE`. State may have moved and returned while nobody was
/// looking, so neither a delta nor an `UNCHANGED` can be justified across
/// unobserved time." Delta-encoding across the gap would produce bytes that
/// decode — the reader's prior is never invalidated, so the local replay
/// cannot notice — while asserting a continuity the capture does not have. The
/// delta chain restarts at the forced baseline: every later delta chains from
/// it, exactly as after a scheduled block start.
fn opens_baseline(hours_seen: usize, previous_measured_hour: Option<i32>, game_hour: i32) -> bool {
    if hours_seen % BLOCK_HOURS == 0 {
        return true;
    }
    match previous_measured_hour {
        Some(previous) => game_hour != previous.saturating_add(1),
        None => true,
    }
}

/// One usable d32 hour: its game hour, both folded collection states, and the
/// byte span of the record that produced it (the shipping arm is those bytes
/// verbatim, header included).
#[derive(Debug)]
struct D32Hour {
    game_hour: i32,
    wings: KeyedState,
    equipment: KeyedState,
    ship_span: Range<usize>,
}

#[derive(Debug)]
struct StreamScan {
    hours: Vec<D32Hour>,
    /// d32 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// d32 records carrying `DELTA_DATA` before any baseline landed.
    delta_before_baseline_records: usize,
    /// Measured d32 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Total wing rows and entry rows the shipping records carried.
    wing_rows: usize,
    entry_rows: usize,
    delete_rows: usize,
}

// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;
const PROBE_DOMAIN_ID: u16 = 250;

fn check_frame_end_seal(payload: &[u8], at_byte: usize) -> Result<(u32, u32), String> {
    if payload.len() != COMPACT_FRAME_END_PAYLOAD_LEN {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: payload length {} is not \
             {COMPACT_FRAME_END_PAYLOAD_LEN}",
            payload.len()
        ));
    }
    let emitted_domains = u32::from_le_bytes(payload[16..20].try_into().expect("4 bytes"));
    let scheduled_domains = u32::from_le_bytes(payload[20..24].try_into().expect("4 bytes"));
    let status_flags = u32::from_le_bytes(payload[24..28].try_into().expect("4 bytes"));
    let reserved = u32::from_le_bytes(payload[28..32].try_into().expect("4 bytes"));
    if reserved != 0 {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: reserved word is nonzero \
             ({reserved:#010x})"
        ));
    }
    if emitted_domains != scheduled_domains {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: emitted domain count \
             {emitted_domains} does not match scheduled domain count {scheduled_domains}"
        ));
    }
    let unknown = status_flags & !FRAME_END_KNOWN_STATUS_MASK;
    if unknown != 0 {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: status flags {status_flags:#010x} \
             contain unknown bits {unknown:#010x}"
        ));
    }
    let fatal = status_flags & COMPACT_STATUS_FAILURE_MASK;
    if fatal != 0 {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: status flags {status_flags:#010x} \
             contain fatal bits {fatal:#010x}"
        ));
    }
    Ok((emitted_domains, scheduled_domains))
}

/// Replay d32's status-independent production payload authority
/// (`domain_data_authority.rs:6579`-`:6823`).
fn check_authority_payload(
    payload: &[u8],
    header: d32::PayloadHeader,
    at_byte: usize,
) -> Result<(), String> {
    // `domain_data_authority::COMPACT_AIR_WING_MAX_OWNER_TAG_DB_ID`. Was
    // `1_024`, a guess at the vanilla-plus-DLC country count; now the family's
    // engine-structural id-space width, the same derivation `MAX_WING_ID`
    // below already uses.
    const MAX_OWNER_TAG_DB_ID: u32 = (i32::MAX as u32) - 1;
    const MAX_WING_ID: u32 = i32::MAX as u32;
    const MAX_AIR_BASE_ID: u32 = i32::MAX as u32;
    const MAX_VARIANT_ID: u32 = i32::MAX as u32;

    if header.status_flags & COMPACT_STATUS_DELTA_DATA == 0
        && header.row_count > header.observed_air_wing_rows
    {
        return Err(format!(
            "d32 payload at byte {at_byte}: row_count > observed_air_wing_rows"
        ));
    }
    let wings_len = (header.row_count as usize)
        .checked_mul(d32::WING_ROW_LEN)
        .ok_or_else(|| format!("d32 payload at byte {at_byte}: wing row count overflows"))?;
    let entries_len = (header.equipment_count as usize)
        .checked_mul(d32::ENTRY_ROW_LEN)
        .ok_or_else(|| format!("d32 payload at byte {at_byte}: entry row count overflows"))?;
    let expected = d32::HEADER_LEN
        .checked_add(wings_len)
        .and_then(|n| n.checked_add(entries_len))
        .ok_or_else(|| format!("d32 payload at byte {at_byte}: payload length overflows"))?;
    if payload.len() != expected {
        return Err(format!(
            "d32 payload at byte {at_byte}: length {} != {expected}",
            payload.len()
        ));
    }
    let wings = &payload[d32::HEADER_LEN..d32::HEADER_LEN + wings_len];
    let entries = &payload[d32::HEADER_LEN + wings_len..];

    let mut seen_wings = BTreeSet::new();
    let mut declared_entries = 0usize;
    let mut live_ranges = Vec::new();
    for (index, row) in wings.chunks_exact(d32::WING_ROW_LEN).enumerate() {
        let parsed = d32::parse_wing_row(row)
            .map_err(|e| format!("d32 wing row {index} at byte {at_byte}: {e}"))?;
        if parsed.owner_tag_db_id == 0 || parsed.owner_tag_db_id > MAX_OWNER_TAG_DB_ID {
            return Err(format!(
                "d32 wing row {index} at byte {at_byte}: invalid owner_tag_db_id {}",
                parsed.owner_tag_db_id
            ));
        }
        if parsed.wing_id == 0 || parsed.wing_id > MAX_WING_ID {
            return Err(format!(
                "d32 wing row {index} at byte {at_byte}: invalid wing_id {}",
                parsed.wing_id
            ));
        }
        if !seen_wings.insert((
            parsed.owner_tag_db_id,
            parsed.wing_id,
            parsed.generation,
            parsed.deleted,
        )) {
            return Err(format!(
                "d32 wing row {index} at byte {at_byte}: duplicate owner/wing/generation"
            ));
        }
        if parsed.deleted {
            if parsed.air_base_id.is_some() || parsed.entry_first != 0 || parsed.entry_count != 0 {
                return Err(format!(
                    "d32 deleted wing row {index} at byte {at_byte} carries live fields"
                ));
            }
            continue;
        }
        let Some(air_base_id) = parsed.air_base_id else {
            return Err(format!(
                "d32 wing row {index} at byte {at_byte}: zero air_base_id"
            ));
        };
        if air_base_id > MAX_AIR_BASE_ID {
            return Err(format!(
                "d32 wing row {index} at byte {at_byte}: invalid air_base_id {air_base_id}"
            ));
        }
        let range = d32::entry_index_range(&parsed, header.equipment_count as usize)
            .map_err(|e| format!("d32 wing row {index} at byte {at_byte}: {e}"))?;
        declared_entries = declared_entries.saturating_add(parsed.entry_count);
        live_ranges.push((
            parsed.owner_tag_db_id,
            parsed.wing_id,
            parsed.generation,
            range,
        ));
    }
    if declared_entries != header.equipment_count as usize {
        return Err(format!(
            "d32 payload at byte {at_byte}: declared entries {declared_entries} != {}",
            header.equipment_count
        ));
    }

    let mut seen_entries = BTreeSet::new();
    for (owner, wing, generation, range) in live_ranges {
        for child_index in range {
            let start = child_index * d32::ENTRY_ROW_LEN;
            let row = &entries[start..start + d32::ENTRY_ROW_LEN];
            let entry = d32::parse_equipment_row(row)
                .map_err(|e| format!("d32 equipment row {child_index} at byte {at_byte}: {e}"))?;
            if entry.variant_id == 0 || entry.variant_id > MAX_VARIANT_ID {
                return Err(format!(
                    "d32 equipment row {child_index} at byte {at_byte}: invalid variant_id {}",
                    entry.variant_id
                ));
            }
            if !(0..=d32::PRODUCER_MAX_AMOUNT_RAW).contains(&entry.amount_raw) {
                return Err(format!(
                    "d32 equipment row {child_index} at byte {at_byte}: invalid amount_raw {}",
                    entry.amount_raw
                ));
            }
            if !seen_entries.insert((owner, wing, generation, entry.variant_id)) {
                return Err(format!(
                    "d32 wing at byte {at_byte} repeats variant_id {}",
                    entry.variant_id
                ));
            }
        }
    }
    Ok(())
}

/// The d30 `air_wings` key set, and nothing else.
///
/// **d30 is not ported.** This reads the three key columns of its rows and its
/// baseline-vs-sparse rule, which is the whole of what
/// `prune_air_wing_loadout_orphans_for_current_hour` consults
/// (`world_fold.rs:5102`-`:5127`). Every other field of a d30 row is skipped by
/// stride. It is a deliberately minimal mirror of `fold_air_wings`
/// (`world_fold.rs:14587`-`:14674`) whose only purpose is to let the harness
/// prove the prune inert on this capture; it is not a d32-style boundary and
/// nothing but the assertion below may be built on it.
#[derive(Default)]
struct AirWingRoster {
    keys: BTreeSet<(u32, u32, u64)>,
    baseline_seen: bool,
    /// Keys d30's own delete path registered for the orphan check this hour
    /// (`world_fold.rs:14657`-`:14658`).
    deleted_this_hour: BTreeSet<(u32, u32, u64)>,
}

impl AirWingRoster {
    /// `AIR_WING_HEADER_LEN`, `world_fold.rs:975`.
    const HEADER_LEN: usize = 36;
    /// `AIR_WING_ROW_LEN`, `world_fold.rs:976`.
    const ROW_LEN: usize = 40;
    /// `AIR_WING_KNOWN_FLAGS`, `world_fold.rs:1047`.
    const KNOWN_FLAGS: u32 = 1 | (1 << 1);

    fn fold(&mut self, payload: &[u8]) -> Result<(), String> {
        self.deleted_this_hour.clear();
        if payload.len() < Self::HEADER_LEN {
            return Err("air_wings payload shorter than its header".to_string());
        }
        let row_count = u32_at(payload, 8) as usize;
        let status_flags = u32_at(payload, 16);
        // VALIDITY BEFORE CLASSIFICATION, on d30's mirror as much as on d32's own
        // pass. The two predicates below classify the v1 status vocabulary and are
        // not validity checks (`compact_contract/src/lib.rs:121`-`:123`), so a
        // status word carrying a bit version 1 does not assign is refused before
        // either runs — bits 15..=31 are unassigned (`status_flags_base.rs:40`-
        // `:24`) and an unassigned bit "is not evidence of a future informational
        // status". Without this, an unassigned bit reads as "not a failure", the
        // roster folds the frame, and the orphan-prune assertion below is made
        // against a roster built from a record with no defined meaning.
        let unknown_status = compact_status_unknown_flags_v1(status_flags);
        if unknown_status != 0 {
            return Err(format!(
                "air_wings DATA carries unassigned v1 status bits {unknown_status:#x}; an \
                 unassigned bit has no defined classification"
            ));
        }
        // STRUCTURE BEFORE CLASSIFICATION, the rest of the way: production's
        // d30 payload authority (`decode_compact_air_wings_data_payload`,
        // `domain_data_authority.rs:6257`) re-checks the payload length and
        // walks EVERY row whatever the status word says — its authority is
        // status-independent. A mirror that returned on a failure status before
        // these checks would accept, on exactly the failure-status records, a
        // malformed d30 payload the recorder itself refuses (the d94 lesson).
        // So the length and the full row walk run on every record; only the
        // STATE mutation below is gated on the fold's own classification.
        let expected = row_count
            .checked_mul(Self::ROW_LEN)
            .and_then(|n| n.checked_add(Self::HEADER_LEN))
            .ok_or_else(|| "air_wings row_count overflows".to_string())?;
        if payload.len() != expected {
            return Err(format!("air_wings len {} != {expected}", payload.len()));
        }
        let mut rows: Vec<((u32, u32, u64), bool)> = Vec::with_capacity(row_count);
        for index in 0..row_count {
            let base = Self::HEADER_LEN + index * Self::ROW_LEN;
            let key = (
                u32_at(payload, base),
                u32_at(payload, base + 4),
                u64_at(payload, base + 8),
            );
            let flags = u32_at(payload, base + 16);
            if flags & !Self::KNOWN_FLAGS != 0 {
                return Err("air_wings row has unknown flags".to_string());
            }
            rows.push((key, flags & 1 != 0));
        }
        // `world_fold.rs:14620`: a failure status folds nothing and retains.
        if compact_status_failure_flags_after_expected_partial_tolerance(D30, status_flags) != 0 {
            return Ok(());
        }
        let delta = status_flags & COMPACT_STATUS_DELTA_DATA != 0;
        // `world_fold.rs:14624`: a delta before any baseline folds nothing.
        if delta && !self.baseline_seen {
            return Ok(());
        }
        // `replace_or_sparse_after_first`, `world_fold.rs:14636`.
        let sparse = delta || self.baseline_seen;
        if !sparse {
            self.keys.clear();
            self.baseline_seen = true;
        }
        for (key, deleted) in rows {
            if deleted {
                self.keys.remove(&key);
                self.deleted_this_hour.insert(key);
            } else {
                self.keys.insert(key);
            }
        }
        Ok(())
    }
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        b[off],
        b[off + 1],
        b[off + 2],
        b[off + 3],
        b[off + 4],
        b[off + 5],
        b[off + 6],
        b[off + 7],
    ])
}

/// The wing keys a d32 payload inserts, which is what the fold registers for the
/// orphan check (`world_fold.rs:14910`-`:14911`). Delete rows register nothing
/// here — they register on d30's side.
fn inserted_wing_keys(payload: &[u8]) -> Result<Vec<(u32, u32, u64)>, String> {
    let (_, wing_rows, _) = d32::split_payload(payload)?;
    let mut out = Vec::new();
    for row in wing_rows.chunks_exact(d32::WING_ROW_LEN) {
        let parsed = d32::parse_wing_row(row)?;
        if !parsed.deleted {
            out.push((parsed.owner_tag_db_id, parsed.wing_id, parsed.generation));
        }
    }
    Ok(out)
}

/// Validate the whole stream against the canonical contract and fold its usable
/// d32 DATA records into per-hour states. Every rejection path is an `Err`, never
/// a silently shorter measurement and never a panic — see the module docs for
/// what is enforced and what the recorder's own validator additionally checks.
fn scan_stream(
    schema: &wire_format::Schema,
    stream: &[u8],
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    let wings_collection = &schema.collections[0];
    let equipment_collection = &schema.collections[1];

    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D32Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut delta_before_baseline_records = 0usize;
    let mut non_contiguous = 0usize;
    let mut wing_rows = 0usize;
    let mut entry_rows = 0usize;
    let mut delete_rows = 0usize;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d32 DATA record of ANY status — the uniqueness rule.
    let mut last_d32_hour: Option<i32> = None;
    // Hour of the last d32 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    // The retained accumulator. d32 is sparse-after-first, so this lives across
    // the whole scan and each hour's state is a snapshot of it.
    let mut state = d32::State::new();
    let mut roster = AirWingRoster::default();
    // Keys d32 inserted this hour, for the FRAME_END orphan check.
    let mut d32_registered_this_hour: Vec<(u32, u32, u64)> = Vec::new();
    // Whether the hour currently open produced a measurable d32 state.
    let mut pending_hour: Option<D32Hour> = None;

    // Closed game hours and d32 DATA records of ANY status — the
    // completeness rule at the end of the scan.
    let mut closed_hours = 0usize;
    let mut d32_records = 0usize;
    let mut pos = 0usize;
    while pos < stream.len() {
        let header = parse_compact_record_header(&stream[pos..])
            .map_err(|e| format!("record at byte {pos}: {e}"))?;
        let payload_start = pos + COMPACT_RECORD_HEADER_LEN;
        let record_end = payload_start
            .checked_add(header.payload_len)
            .ok_or_else(|| format!("record at byte {pos}: length overflow"))?;
        if record_end > stream.len() {
            // `eof_check`: a stream ending mid-record is not a short stream, it
            // is a corrupt one.
            return Err(format!(
                "stream ends mid-record at byte {pos}: {} byte(s) present, record needs {}",
                stream.len() - pos,
                record_end - pos
            ));
        }
        let payload = &stream[payload_start..record_end];

        validator
            .validate_record(&header, payload)
            .map_err(|e| format!("record at byte {pos}: {e:?}"))?;

        // The hour-boundary subset of `BoundaryTracker::observe`.
        match header.record_type {
            COMPACT_RECORD_TYPE_DOMAIN_PERF => {
                if header.domain_id != PROBE_DOMAIN_ID
                    && !non_probe_perf_domains.insert(header.domain_id)
                {
                    return Err(format!(
                        "duplicate DOMAIN_PERF for domain {} in hour {} at byte {pos}",
                        header.domain_id, header.game_hour_raw
                    ));
                }
                match open_hour {
                    Some(open) if open != header.game_hour_raw => {
                        return Err(format!(
                            "hour interleaving at byte {pos}: DOMAIN_PERF for hour {} while \
                             hour {open} is still open",
                            header.game_hour_raw
                        ))
                    }
                    Some(_) => {}
                    None => {
                        if let Some(last_closed) = last_closed_hour {
                            if header.game_hour_raw <= last_closed {
                                return Err(format!(
                                    "non-monotonic DOMAIN_PERF at byte {pos}: closed hour \
                                     {last_closed}, then {}",
                                    header.game_hour_raw
                                ));
                            }
                        }
                        match pending_perf_hour {
                            Some(pending) if pending != header.game_hour_raw => {
                                return Err(format!(
                                    "PERF hour conflict at byte {pos}: pending hour {pending}, \
                                     then {}",
                                    header.game_hour_raw
                                ))
                            }
                            Some(_) => {}
                            None => pending_perf_hour = Some(header.game_hour_raw),
                        }
                    }
                }
            }
            COMPACT_RECORD_TYPE_DOMAIN_DATA => match open_hour {
                Some(open) if open != header.game_hour_raw => {
                    return Err(format!(
                        "DOMAIN_DATA at byte {pos} carries hour {} while hour {open} is open",
                        header.game_hour_raw
                    ))
                }
                Some(_) => {}
                None => {
                    if let Some(pending) = pending_perf_hour {
                        if pending != header.game_hour_raw {
                            return Err(format!(
                                "DATA hour conflict at byte {pos}: pending PERF hour \
                                 {pending}, then DATA hour {}",
                                header.game_hour_raw
                            ));
                        }
                    }
                    if let Some(last_closed) = last_closed_hour {
                        if header.game_hour_raw <= last_closed {
                            return Err(format!(
                                "DOMAIN_DATA at byte {pos} reopens hour {} after hour \
                                 {last_closed} closed",
                                header.game_hour_raw
                            ));
                        }
                    }
                    open_hour = Some(header.game_hour_raw);
                }
            },
            COMPACT_RECORD_TYPE_FRAME_END => {
                let (emitted_domains, scheduled_domains) = check_frame_end_seal(payload, pos)?;
                let observed_perf_domains =
                    non_probe_perf_domains.len().min(u32::MAX as usize) as u32;
                if observed_perf_domains != emitted_domains
                    || observed_perf_domains != scheduled_domains
                {
                    return Err(format!(
                        "FRAME_END domain counts at byte {pos} claim emitted={emitted_domains} \
                         scheduled={scheduled_domains}, but observed {observed_perf_domains} \
                         unique non-probe DOMAIN_PERF domains"
                    ));
                }
                match open_hour {
                    Some(open) if open == header.game_hour_raw => {}
                    Some(open) => {
                        return Err(format!(
                            "FRAME_END at byte {pos} closes hour {} while hour {open} is open",
                            header.game_hour_raw
                        ))
                    }
                    None => {
                        return Err(format!(
                            "FRAME_END at byte {pos} closes hour {} without an open \
                             DOMAIN_DATA hour",
                            header.game_hour_raw
                        ))
                    }
                }
                if let Some(last_closed) = last_closed_hour {
                    if header.game_hour_raw <= last_closed {
                        return Err(format!(
                            "non-monotonic FRAME_END at byte {pos}: closed hour \
                             {last_closed}, then {}",
                            header.game_hour_raw
                        ));
                    }
                }

                // `finish_hour` (`world_fold.rs:5004`-`:5020`) runs the
                // cross-domain prune here, after every domain fold of the hour.
                // Modelled as an assertion — see the module docs.
                let mut orphan_candidates: BTreeSet<(u32, u32, u64)> =
                    d32_registered_this_hour.iter().copied().collect();
                orphan_candidates.extend(roster.deleted_this_hour.iter().copied());
                for candidate in &orphan_candidates {
                    if roster.keys.contains(candidate) {
                        continue;
                    }
                    let wing = d32::wing_key(candidate.0, candidate.1, candidate.2);
                    if state.wings.contains_key(&wing) {
                        return Err(format!(
                            "hour {}: the d30 orphan prune would drop d32 wing \
                             (owner {}, wing {}, generation {}); this harness does not model \
                             the prune and must not report a number for a state the recorder \
                             would not have held",
                            header.game_hour_raw, candidate.0, candidate.1, candidate.2
                        ));
                    }
                }
                d32_registered_this_hour.clear();
                roster.deleted_this_hour.clear();
                // The hour's d32 state is taken AFTER the prune check, so what is
                // measured is what the recorder would have held at snapshot time.
                if let Some(mut hour) = pending_hour.take() {
                    hour.wings = state.wings.clone();
                    hour.equipment = state.equipment.clone();
                    hours.push(hour);
                }

                open_hour = None;
                pending_perf_hour = None;
                non_probe_perf_domains.clear();
                last_closed_hour = Some(header.game_hour_raw);
                closed_hours += 1;
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA && header.domain_id == D30 {
            roster
                .fold(payload)
                .map_err(|e| format!("d30 payload at byte {pos}: {e}"))?;
        }

        if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA && header.domain_id == D32 {
            d32_records += 1;
            // One d32 DATA record per game hour, checked over EVERY d32 record
            // and BEFORE any status-based exclusion: gating this on the record
            // being usable would let a success + failure pair through the rule
            // entirely, and two records in one hour would fold twice into a
            // state measured once.
            if let Some(previous) = last_d32_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d32 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d32_hour = Some(header.game_hour_raw);

            // The fold's own call order: header checks, then the status check
            // (`world_fold.rs:14828`), then everything else.
            let d32_header =
                d32::peek_header(payload).map_err(|e| format!("d32 payload at byte {pos}: {e}"))?;

            // VALIDITY BEFORE CLASSIFICATION. Everything below that reads the
            // status word — the failure predicate here, the baseline/delta
            // decision inside `fold_payload` — classifies the v1 vocabulary; none
            // of it is a validity predicate. `compact_contract/src/lib.rs:121`-
            // `:123` says so in as many words: "Callers handling untrusted status
            // words must first reject a non-zero `compact_status_unknown_flags_v1`
            // result; otherwise an unassigned bit has no defined classification."
            // Bits 15..=31 are unassigned (`status_flags_base.rs:40`-`:44`), and
            // an unassigned bit "is not evidence of a future informational
            // status".
            //
            // Without this, a record with `status_flags = 1 << 14` reads as "not
            // a failure" — the failure helper masks with the *known* failure set
            // and returns zero for a bit it does not know — and the hour is
            // measured as healthy. The production lane rejects first
            // (`lanes/division_state_movement_live/domain20_compact.rs:419`);
            // this harness does the same rather than re-deriving a lenient rule.
            // No capture carries an unassigned bit, so no measured number moves.
            let unknown_status = compact_status_unknown_flags_v1(d32_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d32 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            check_authority_payload(payload, d32_header, pos)?;

            // `handle_compact_failure` (`world_fold.rs:2843`, called at `:14828`)
            // marks the domain-hour unknown and RETAINS the prior accumulator,
            // folding nothing from this record. The predicate is
            // `compact_contract`'s own, not a copied bit mask: it already applies
            // d32's lane-local mask (none) and expected partial tolerance (none,
            // `compact_contract/src/lib.rs:136`), so it cannot drift.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D32,
                d32_header.status_flags,
            ) != 0
            {
                failure_records += 1;
                pos = record_end;
                continue;
            }

            // Now reproduce the FOLD's own outcome order. Its
            // `reject_delta_before_baseline` (`world_fold.rs:14837`) precedes the
            // fold's structural pass, even though production's independent
            // payload authority above has already validated the record for stream
            // certification.
            let outcome = state
                .fold_payload(wings_collection, equipment_collection, payload)
                .map_err(|e| format!("d32 payload at byte {pos}: {e}"))?;
            if outcome == d32::FoldOutcome::DeltaBeforeBaseline {
                // `reject_delta_before_baseline` (`:14837`): nothing folded, the
                // accumulator retained, the domain-hour unknown. Not d32 data.
                delta_before_baseline_records += 1;
                pos = record_end;
                continue;
            }
            // Only now is the body known to be structurally sound, because the
            // fold that just accepted it ran `split_payload` itself.
            let registered = inserted_wing_keys(payload)
                .map_err(|e| format!("d32 payload at byte {pos}: {e}"))?;

            wing_rows += d32_header.row_count as usize;
            entry_rows += d32_header.equipment_count as usize;
            delete_rows += d32_header.deleted_rows as usize;
            d32_registered_this_hour.extend(registered);

            if let Some(previous) = last_measured_hour {
                if header.game_hour_raw != previous + 1 {
                    non_contiguous += 1;
                }
            }
            last_measured_hour = Some(header.game_hour_raw);
            // The state itself is filled in at FRAME_END, after the prune check.
            pending_hour = Some(D32Hour {
                game_hour: header.game_hour_raw,
                wings: BTreeMap::new(),
                equipment: BTreeMap::new(),
                ship_span: pos..record_end,
            });
        }

        pos = record_end;
    }

    // `BoundaryTracker::eof_check` (`recorder_host/src/stream_fold.rs:459`).
    if let Some(open) = open_hour {
        return Err(format!("game hour {open} left open at EOF"));
    }
    if last_closed_hour.is_none() {
        return Err("stream closes no game hour (no FRAME_END records)".to_string());
    }
    if last_record_type != Some(COMPACT_RECORD_TYPE_FRAME_END) {
        return Err(format!(
            "stream ends after record type {}; final record must be a canonical FRAME_END",
            last_record_type.map_or_else(|| "none".to_string(), |k| k.to_string())
        ));
    }
    if hours.is_empty() {
        return Err("stream carries no usable d32 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d32 DATA
    // record, of any status. Production validates the payload of EVERY domain's
    // DATA record (`stream_fold.rs:593`); this harness cannot (module docs), so
    // a d32 record relabelled to another domain id — or lost outright — would
    // otherwise be silently ignored and the run would report a plausible
    // measurement over fewer hours. Per-hour uniqueness is already enforced
    // above, so equality of the two counts is exactly-one per closed hour. A
    // per-capture assertion, not a grammar rule: a capture that does not
    // schedule d32 every hour fails here loudly and a human adjusts, instead
    // of a number quietly shrinking.
    if d32_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d32_records} d32 DATA \
             record(s); every closed hour of a d32-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(StreamScan {
        hours,
        failure_records,
        delta_before_baseline_records,
        non_contiguous,
        wing_rows,
        entry_rows,
        delete_rows,
    })
}

fn main() {
    const USAGE: &str =
        "usage: bench_real_d32 <collector_stream.bin> <out dir> [legacy|identity-v1]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d32::schema();

    // The one constant this crate's library half mirrors rather than imports
    // (`d32.rs`'s `STATUS_DELTA_DATA`). This bin does depend on
    // `compact_contract`, so it is the natural place to prove the mirror right.
    assert_eq!(
        d32::STATUS_DELTA_DATA,
        COMPACT_STATUS_DELTA_DATA,
        "d32's mirrored DELTA_DATA bit must equal the shipping contract's"
    );

    // ---- Pass 1: validate the WHOLE stream and fold its d32 DATA records. ---
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result.
    let scan = match scan_stream(&schema, &stream, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: encode both arms. Nothing here reads stream content again. --

    let wings_collection = &schema.collections[0];
    let equipment_collection = &schema.collections[1];

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut prior: Option<(&KeyedState, &KeyedState)> = None;
    let mut reader: Option<(KeyedState, KeyedState)> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    // Baselines the §1.5 gap rule forced outside a scheduled block start.
    let mut gap_forced_baselines = 0usize;
    let mut previous_measured_hour: Option<i32> = None;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    let mut changed_wings = 0usize;
    let mut changed_equipment = 0usize;
    let mut tombstoned_wings = 0usize;
    let mut tombstoned_equipment = 0usize;
    let mut wings_unchanged_in_delta_hour = 0usize;
    let mut equipment_unchanged_in_delta_hour = 0usize;
    let mut wings_total = 0usize;
    let mut equipment_total = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        let state = (&hour.wings, &hour.equipment);
        wings_total += state.0.len();
        equipment_total += state.1.len();

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        let hour_start = wire.len();
        let baseline_here = opens_baseline(hours_seen, previous_measured_hour, hour.game_hour);
        if baseline_here && hours_seen % BLOCK_HOURS != 0 {
            gap_forced_baselines += 1;
        }
        let mut emitted = true;
        if baseline_here || prior.is_none() {
            encode_baseline(&schema, &[state.0.clone(), state.1.clone()], &mut wire)
                .expect("baseline encodes");
            baselines += 1;
            baseline_bytes += wire.len() - hour_start;
        } else {
            let prev = prior.unwrap();
            let dw = diff(wings_collection, prev.0, state.0);
            let de = diff(equipment_collection, prev.1, state.1);
            if dw.is_empty() && de.is_empty() {
                empty_hours += 1;
                emitted = false;
            } else {
                changed_wings += dw.changed.len();
                changed_equipment += de.changed.len();
                tombstoned_wings += dw.tombstones.len();
                tombstoned_equipment += de.tombstones.len();
                if dw.is_empty() {
                    wings_unchanged_in_delta_hour += 1;
                }
                if de.is_empty() {
                    equipment_unchanged_in_delta_hour += 1;
                }
                // §7.1: a collection that did not change contributes `None` (its
                // bitmap bit stays clear), never an empty contribution — the
                // encoder rejects the latter outright. With two collections the
                // two are genuinely independent: a placement can move while every
                // equipment amount holds, and far more often the reverse.
                encode_delta(
                    &schema,
                    &[prev.0.clone(), prev.1.clone()],
                    &[
                        (!dw.is_empty()).then_some(dw),
                        (!de.is_empty()).then_some(de),
                    ],
                    &mut wire,
                )
                .expect("delta encodes");
                delta_bytes += wire.len() - hour_start;
            }
        }
        wire_offsets.push(wire.len());

        // Reader replay: decode the bytes just written against the reader's OWN
        // reconstructed prior, exactly as a conforming reader would, and require
        // the result to be stored-equal to this hour's state. This is what makes
        // the size claim a claim about decodable bytes. An hour that emitted
        // nothing is still checked: the reader's retained state must already
        // equal the fold's, which is precisely what "nothing changed" asserts.
        if emitted {
            let hour_bytes = &wire[hour_start..];
            if baseline_here || reader.is_none() {
                let (states, consumed) =
                    decode_baseline(&schema, hour_bytes, 0).expect("baseline decodes");
                assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
                let mut it = states.into_iter();
                let w = it.next().unwrap();
                let e = it.next().unwrap();
                reader = Some((w, e));
            } else {
                let (rw, re) = reader.take().unwrap();
                let (deltas, consumed) =
                    decode_delta(&schema, &[rw.clone(), re.clone()], hour_bytes, 0)
                        .expect("delta decodes");
                assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
                let mut next_w = rw;
                let mut next_e = re;
                if let Some(d) = deltas[0].as_ref() {
                    apply(&mut next_w, d);
                }
                if let Some(d) = deltas[1].as_ref() {
                    apply(&mut next_e, d);
                }
                reader = Some((next_w, next_e));
            }
        }
        let replayed = reader.as_ref().unwrap();
        assert_eq!(
            replayed.0.len(),
            state.0.len(),
            "game hour {}: wing count",
            hour.game_hour
        );
        assert_eq!(
            replayed.1.len(),
            state.1.len(),
            "game hour {}: equipment count",
            hour.game_hour
        );
        assert!(
            diff(wings_collection, &replayed.0, state.0).is_empty(),
            "game hour {}: reader wings are not stored-equal to the fold",
            hour.game_hour
        );
        assert!(
            diff(equipment_collection, &replayed.1, state.1).is_empty(),
            "game hour {}: reader equipment is not stored-equal to the fold",
            hour.game_hour
        );

        prior = Some(state);
        previous_measured_hour = Some(hour.game_hour);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d32.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d32.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let delta_hours = hours_seen - baselines - empty_hours;
    println!(
        "hours                {hours_seen} ({} non-contiguous steps)",
        scan.non_contiguous
    );
    println!(
        "failure-status hours {} (excluded from both arms)",
        scan.failure_records
    );
    println!(
        "delta-before-base    {} (excluded from both arms)",
        scan.delta_before_baseline_records
    );
    println!(
        "shipping rows        {} wing rows ({} deletes), {} entry rows",
        scan.wing_rows, scan.delete_rows, scan.entry_rows
    );
    println!(
        "state avg            {:.1} wings, {:.1} equipment lines",
        wings_total as f64 / hours_seen as f64,
        equipment_total as f64 / hours_seen as f64
    );
    println!(
        "baselines            {baselines} ({} every {BLOCK_HOURS} h, {gap_forced_baselines} \
         forced by a gap in measured hours — WIRE_FORMAT §1.5)",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn           wings {:.2} changed/h, {:.3} tombstones/h; equipment {:.2} changed/h, \
         {:.3} tombstones/h (stored precision)",
        changed_wings as f64 / delta_hours.max(1) as f64,
        tombstoned_wings as f64 / delta_hours.max(1) as f64,
        changed_equipment as f64 / delta_hours.max(1) as f64,
        tombstoned_equipment as f64 / delta_hours.max(1) as f64
    );
    println!(
        "unchanged halves     {wings_unchanged_in_delta_hour} delta hours changed no wing, \
         {equipment_unchanged_in_delta_hour} changed no equipment"
    );
    println!("ship_d32.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
    println!("wire raw split       baseline {baseline_bytes} bytes, delta {delta_bytes} bytes");
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN};
    use wire_format::body::Key;

    fn schema() -> wire_format::Schema {
        d32::schema()
    }

    /// Build one H4CR record the way the recorder frames it
    /// (`compact_contract/src/record_header.rs:60`).
    fn record(sequence: u64, game_hour: i32, domain: u16, kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(COMPACT_RECORD_HEADER_LEN + payload.len());
        out.extend_from_slice(b"H4CR");
        out.extend_from_slice(&1u16.to_le_bytes()); // version
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // record flags
        out.extend_from_slice(payload);
        out
    }

    fn frame_end(sequence: u64, game_hour: i32) -> Vec<u8> {
        frame_end_sealed(sequence, game_hour, 0, 0, 0, 0)
    }

    fn frame_end_sealed(
        sequence: u64,
        game_hour: i32,
        emitted: u32,
        scheduled: u32,
        status: u32,
        reserved: u32,
    ) -> Vec<u8> {
        let mut payload = vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN];
        payload[16..20].copy_from_slice(&emitted.to_le_bytes());
        payload[20..24].copy_from_slice(&scheduled.to_le_bytes());
        payload[24..28].copy_from_slice(&status.to_le_bytes());
        payload[28..32].copy_from_slice(&reserved.to_le_bytes());
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    fn perf(sequence: u64, game_hour: i32, domain: u16) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &vec![0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN],
        )
    }

    fn wing_row(
        owner: u32,
        wing: u32,
        generation: u64,
        flags: u32,
        air_base_id: u32,
        entry_first: u32,
        entry_count: u32,
    ) -> Vec<u8> {
        let mut row = Vec::new();
        row.extend_from_slice(&owner.to_le_bytes());
        row.extend_from_slice(&wing.to_le_bytes());
        row.extend_from_slice(&generation.to_le_bytes());
        for w in [flags, air_base_id, entry_first, entry_count] {
            row.extend_from_slice(&w.to_le_bytes());
        }
        row
    }

    fn entry_row(variant_id: u32, amount_raw: i64) -> Vec<u8> {
        let mut row = Vec::new();
        row.extend_from_slice(&variant_id.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes());
        row.extend_from_slice(&amount_raw.to_le_bytes());
        row
    }

    fn d32_payload(status_flags: u32, rows: &[Vec<u8>], entries: &[Vec<u8>]) -> Vec<u8> {
        let row_count = rows.len() as u32;
        let entry_count = entries.len() as u32;
        let deleted = rows
            .iter()
            .filter(|r| u32::from_le_bytes(r[16..20].try_into().unwrap()) & 1 != 0)
            .count() as u32;
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes());
        p.extend_from_slice(&row_count.to_le_bytes());
        p.extend_from_slice(&row_count.to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&(row_count - deleted).to_le_bytes());
        p.extend_from_slice(&entry_count.to_le_bytes());
        p.extend_from_slice(&entry_count.to_le_bytes());
        p.extend_from_slice(&deleted.to_le_bytes());
        p.extend_from_slice(&100_000u32.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        for r in rows {
            p.extend_from_slice(r);
        }
        for r in entries {
            p.extend_from_slice(r);
        }
        p
    }

    /// A d30 `air_wings` payload carrying the given keys as live rows.
    fn d30_payload(status_flags: u32, keys: &[(u32, u32, u64)]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes());
        p.extend_from_slice(&(keys.len() as u32).to_le_bytes());
        p.extend_from_slice(&(keys.len() as u32).to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&(keys.len() as u32).to_le_bytes()); // observed
        p.extend_from_slice(&0u32.to_le_bytes()); // deleted
        p.extend_from_slice(&100_000u32.to_le_bytes()); // scale
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved0
        assert_eq!(p.len(), AirWingRoster::HEADER_LEN);
        for (owner, wing, generation) in keys {
            let mut row = vec![0u8; AirWingRoster::ROW_LEN];
            row[0..4].copy_from_slice(&owner.to_le_bytes());
            row[4..8].copy_from_slice(&wing.to_le_bytes());
            row[8..16].copy_from_slice(&generation.to_le_bytes());
            p.extend_from_slice(&row);
        }
        p
    }

    /// Two closed hours, each carrying a d30 roster and a d32 record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D30,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d30_payload(0, &[(7, 11, 3)]),
        ));
        s.extend(record(
            2,
            100,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(
                0,
                &[wing_row(7, 11, 3, 0, 408, 0, 1)],
                &[entry_row(900, 100_000)],
            ),
        ));
        s.extend(frame_end(3, 100));
        s.extend(record(
            4,
            101,
            D30,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d30_payload(0, &[(7, 11, 3)]),
        ));
        s.extend(record(
            5,
            101,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(
                0,
                &[wing_row(7, 11, 3, 0, 408, 0, 1)],
                &[entry_row(900, 250_000)],
            ),
        ));
        s.extend(frame_end(6, 101));
        s
    }

    /// The byte offset of the Nth record boundary in a stream of framed records.
    fn boundary(stream: &[u8], records: usize) -> usize {
        let mut pos = 0usize;
        for _ in 0..records {
            let h = parse_compact_record_header(&stream[pos..]).expect("framed");
            pos += COMPACT_RECORD_HEADER_LEN + h.payload_len;
        }
        pos
    }

    #[test]
    fn a_well_formed_stream_yields_its_d32_hours() {
        let scan =
            scan_stream(&schema(), &good_stream(), StreamIdentityMode::Legacy).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[1].game_hour, 101);
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.delta_before_baseline_records, 0);
        assert_eq!(scan.non_contiguous, 0);
        assert_eq!(scan.hours[0].wings.len(), 1);
        assert_eq!(scan.hours[0].equipment.len(), 1);
        // The second hour is a sparse frame over the retained accumulator, so the
        // state is still one wing — not a fresh full replace.
        assert_eq!(scan.hours[1].wings.len(), 1);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helpers classify
        // the v1 vocabulary and are not validity predicates, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure", gets a baseline/delta meaning, and the hour is measured
        // as healthy. Both status readers in this bin are covered: d32's own pass
        // and the d30 roster mirror the orphan-prune assertion is built on.
        // Bit 15 is ASSIGNED (`COMPACT_STATUS_BASELINE_CLEAR`, the stream's deliberate
        // resync primitive), so the UNASSIGNED run starts at 16.
        for bit in 16..32u32 {
            let status = 1u32 << bit;
            // Alone, combined with a known failure bit, and combined with the
            // informational DELTA_DATA bit. The rejection must not depend on it.
            for extra in [0u32, 1, COMPACT_STATUS_DELTA_DATA] {
                let mut on_d32 = Vec::new();
                on_d32.extend(record(
                    1,
                    100,
                    D30,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d30_payload(0, &[(7, 11, 3)]),
                ));
                on_d32.extend(record(
                    2,
                    100,
                    D32,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d32_payload(
                        status | extra,
                        &[wing_row(7, 11, 3, 0, 408, 0, 1)],
                        &[entry_row(900, 100_000)],
                    ),
                ));
                on_d32.extend(frame_end(3, 100));
                assert!(
                    scan_stream(&schema(), &on_d32, StreamIdentityMode::Legacy).is_err(),
                    "d32 status bit {bit} (+{extra:#x}) must be refused"
                );

                let mut on_d30 = Vec::new();
                on_d30.extend(record(
                    1,
                    100,
                    D30,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d30_payload(status | extra, &[(7, 11, 3)]),
                ));
                on_d30.extend(record(
                    2,
                    100,
                    D32,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d32_payload(
                        0,
                        &[wing_row(7, 11, 3, 0, 408, 0, 1)],
                        &[entry_row(900, 100_000)],
                    ),
                ));
                on_d30.extend(frame_end(3, 100));
                assert!(
                    scan_stream(&schema(), &on_d30, StreamIdentityMode::Legacy).is_err(),
                    "d30 roster status bit {bit} (+{extra:#x}) must be refused"
                );
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted
        // on both readers: the check must reject unassigned bits and nothing else.
        // DELTA_DATA is the one both domains read, and after a baseline it is a
        // sparse update.
        for informational in [
            1u32 << 4,
            1 << 5,
            COMPACT_STATUS_DELTA_DATA,
            1 << 11,
            1 << 13,
        ] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D30,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d30_payload(0, &[(7, 11, 3)]),
            ));
            s.extend(record(
                2,
                100,
                D32,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d32_payload(
                    0,
                    &[wing_row(7, 11, 3, 0, 408, 0, 1)],
                    &[entry_row(900, 100_000)],
                ),
            ));
            s.extend(frame_end(3, 100));
            s.extend(record(
                4,
                101,
                D30,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d30_payload(informational, &[(7, 11, 3)]),
            ));
            s.extend(record(
                5,
                101,
                D32,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d32_payload(
                    informational,
                    &[wing_row(7, 11, 3, 0, 408, 0, 1)],
                    &[entry_row(900, 250_000)],
                ),
            ));
            s.extend(frame_end(6, 101));
            let scan = scan_stream(&schema(), &s, StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 2, "bit {informational:#x}");
            assert_eq!(scan.failure_records, 0, "bit {informational:#x}");
            assert_eq!(
                scan.delta_before_baseline_records, 0,
                "bit {informational:#x}"
            );
        }
    }

    #[test]
    fn the_state_is_snapshotted_after_the_hour_closes_not_at_the_record() {
        // The prune runs at FRAME_END, so an hour's measured state must be the
        // post-prune one. A stream whose FRAME_END never arrives contributes no
        // hour at all.
        let full = good_stream();
        let cut = boundary(&full, 2); // after hour 100's d32 record, before its FRAME_END
        assert!(
            scan_stream(&schema(), &full[..cut], StreamIdentityMode::Legacy).is_err(),
            "an hour left open"
        );
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        let full = good_stream();
        for records in [1usize, 2, 4, 5] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&schema(), &full[..cut], StreamIdentityMode::Legacy).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream
        // and `eof_check` accepts it. What matters is that it reports the hours it
        // actually closed.
        for (records, expected_hours) in [(3usize, 1usize), (6, 2)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&schema(), &full[..cut], StreamIdentityMode::Legacy)
                .expect("closed stream is well-formed");
            assert_eq!(scan.hours.len(), expected_hours, "cut after {records}");
        }
    }

    #[test]
    fn a_stream_ending_mid_record_is_rejected() {
        let full = good_stream();
        let boundaries: Vec<usize> = (0..=6).map(|n| boundary(&full, n)).collect();
        for cut in 1..full.len() {
            if boundaries.contains(&cut) {
                continue;
            }
            assert!(
                scan_stream(&schema(), &full[..cut], StreamIdentityMode::Legacy).is_err(),
                "mid-record cut at {cut}"
            );
        }
        assert!(
            scan_stream(&schema(), &[], StreamIdentityMode::Legacy).is_err(),
            "an empty stream is not a measurement"
        );
    }

    #[test]
    fn a_header_the_canonical_parser_rejects_is_rejected_here() {
        for records in 0..6 {
            let at = boundary(&good_stream(), records);
            for (offset, patch) in [
                (0usize, vec![b'X']),              // magic
                (4, 2u16.to_le_bytes().to_vec()),  // unsupported version
                (6, 33u16.to_le_bytes().to_vec()), // unsupported header length
            ] {
                let mut s = good_stream();
                s[at + offset..at + offset + patch.len()].copy_from_slice(&patch);
                assert!(
                    scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err(),
                    "record {records} patched at +{offset} must be refused"
                );
            }
        }
    }

    #[test]
    fn a_sequence_discontinuity_is_rejected() {
        for records in 0..6 {
            let at = boundary(&good_stream(), records);
            for wrong in [0u64, 9] {
                let mut s = good_stream();
                s[at + 8..at + 16].copy_from_slice(&wrong.to_le_bytes());
                assert!(
                    scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err(),
                    "record {records} sequence {wrong}"
                );
            }
        }
    }

    #[test]
    fn a_frame_end_that_does_not_close_its_open_hour_is_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(0, &[], &[]),
        ));
        s.extend(frame_end(2, 999));
        assert!(scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err());

        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            101,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(0, &[], &[]),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(0, &[], &[]),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_second_d32_record_in_one_hour_is_rejected() {
        // Checked over EVERY d32 DATA record and before status-based exclusion,
        // so all four success/failure orderings are refused — gating it on the
        // record being usable would let a success + failure pair through.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D32,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d32_payload(first, &[], &[]),
            ));
            s.extend(record(
                2,
                100,
                D32,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d32_payload(second, &[], &[]),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err(),
                "two d32 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the
        // fold retains the accumulator and folds nothing, so the hour is not d32
        // data and must not enter either arm.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D32,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d32_payload(1 << bit, &[], &[]),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D32,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d32_payload(0, &[wing_row(7, 11, 3, 0, 408, 0, 0)], &[]),
            ));
            s.extend(record(
                4,
                101,
                D30,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d30_payload(0, &[(7, 11, 3)]),
            ));
            s.extend(frame_end(5, 101));
            let scan = scan_stream(&schema(), &s, StreamIdentityMode::Legacy)
                .expect("the stream itself is well-formed");
            assert_eq!(scan.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
        }
        // And DELTA_DATA (1<<10) is NOT a failure bit — after a baseline it is an
        // ordinary sparse frame.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(0, &[], &[]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(COMPACT_STATUS_DELTA_DATA, &[], &[]),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&schema(), &s, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.delta_before_baseline_records, 0);
        assert_eq!(scan.hours.len(), 2);
    }

    #[test]
    fn a_delta_before_any_baseline_is_excluded_rather_than_measured() {
        // `reject_delta_before_baseline` (`world_fold.rs:14837`) folds nothing and
        // marks the domain-hour unknown, so the hour carries no d32 data.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(
                COMPACT_STATUS_DELTA_DATA,
                &[wing_row(7, 11, 3, 0, 408, 0, 0)],
                &[],
            ),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(0, &[wing_row(7, 11, 3, 0, 408, 0, 0)], &[]),
        ));
        s.extend(record(
            4,
            101,
            D30,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d30_payload(0, &[(7, 11, 3)]),
        ));
        s.extend(frame_end(5, 101));
        let scan = scan_stream(&schema(), &s, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.delta_before_baseline_records, 1);
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].game_hour, 101);
        assert_eq!(scan.hours[0].wings.len(), 1);
    }

    #[test]
    fn a_delta_before_baseline_is_still_checked_by_payload_authority() {
        // The fold's delta-before-baseline path returns before its own body read,
        // but production's second payload authority independently reads every
        // DOMAIN_DATA payload (`domain_data_authority.rs:6579`-`:6823`).
        //
        // The reviewer's exact input: a header declaring one wing row and no row
        // bytes, DELTA_DATA set, arriving before any baseline.
        let mut truncated = d32_payload(COMPACT_STATUS_DELTA_DATA, &[], &[]);
        truncated[8..12].copy_from_slice(&1u32.to_le_bytes()); // row_count
        truncated[12..16].copy_from_slice(&1u32.to_le_bytes()); // changed_rows
        truncated[20..24].copy_from_slice(&1u32.to_le_bytes()); // observed rows
        assert_eq!(truncated.len(), 44, "header only, no row bytes");

        // The whole class, not the one input: every body defect the fold does not
        // reach on this path.
        let mut bodies: Vec<Vec<u8>> = vec![truncated];
        // A body one byte short of its declared length.
        let mut short = d32_payload(
            COMPACT_STATUS_DELTA_DATA,
            &[wing_row(7, 11, 3, 0, 408, 0, 0)],
            &[],
        );
        short.pop();
        bodies.push(short);
        // An unknown wing flag bit.
        bodies.push(d32_payload(
            COMPACT_STATUS_DELTA_DATA,
            &[wing_row(7, 11, 3, 1 << 5, 0, 0, 0)],
            &[],
        ));
        // An entry range past the frame's entries.
        bodies.push(d32_payload(
            COMPACT_STATUS_DELTA_DATA,
            &[wing_row(7, 11, 3, 0, 408, 0, 2)],
            &[entry_row(900, 1)],
        ));
        // A duplicated variant inside one wing's range.
        bodies.push(d32_payload(
            COMPACT_STATUS_DELTA_DATA,
            &[wing_row(7, 11, 3, 0, 408, 0, 2)],
            &[entry_row(900, 1), entry_row(900, 2)],
        ));

        for (i, body) in bodies.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(1, 100, D32, COMPACT_RECORD_TYPE_DOMAIN_DATA, &body));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err(),
                "body {i} is malformed to the status-independent authority"
            );
        }

        // The header checks that PRECEDE the status checks in the fold
        // (`:14796`-`:14827`) still apply on this path, so a bad scale or a
        // non-zero reserved word is still a refused stream.
        for (offset, word) in [(36usize, 32_768u32), (40, 1)] {
            let mut body = d32_payload(COMPACT_STATUS_DELTA_DATA, &[], &[]);
            body[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
            let mut s = Vec::new();
            s.extend(record(1, 100, D32, COMPACT_RECORD_TYPE_DOMAIN_DATA, &body));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err(),
                "a header defect at +{offset} precedes the status checks in the fold"
            );
        }
    }

    #[test]
    fn a_stream_with_no_d32_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; 8],
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_malformed_d32_payload_is_refused_not_skipped() {
        // A short or inconsistent payload is a corrupt stream, not an hour to
        // drop quietly.
        let mut payload = d32_payload(0, &[wing_row(7, 11, 3, 0, 408, 0, 0)], &[]);
        payload.truncate(payload.len() - 1);
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err());

        // A row or header the FOLD rejects must make the SCAN fail, not be
        // accepted and then unwind while measuring.
        let mut bad: Vec<Vec<u8>> = Vec::new();
        // an unknown wing flag bit (`world_fold.rs:14858`)
        bad.push(d32_payload(0, &[wing_row(7, 11, 3, 1 << 5, 0, 0, 0)], &[]));
        // an entry range past the frame's entries (`:14866`)
        bad.push(d32_payload(
            0,
            &[wing_row(7, 11, 3, 0, 408, 0, 2)],
            &[entry_row(900, 1)],
        ));
        // a non-zero entry reserved word (`:14885`)
        let mut reserved = entry_row(900, 1);
        reserved[4..8].copy_from_slice(&1u32.to_le_bytes());
        bad.push(d32_payload(
            0,
            &[wing_row(7, 11, 3, 0, 408, 0, 1)],
            &[reserved],
        ));
        // a wrong scale (`:14824`)
        let mut scale = d32_payload(0, &[], &[]);
        scale[36..40].copy_from_slice(&32_768u32.to_le_bytes());
        bad.push(scale);
        // a duplicated variant inside one wing's range — this layer's own check
        bad.push(d32_payload(
            0,
            &[wing_row(7, 11, 3, 0, 408, 0, 2)],
            &[entry_row(900, 1), entry_row(900, 2)],
        ));
        for (i, payload) in bad.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D32,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err(),
                "invalid payload {i} must make the scan fail"
            );
        }
    }

    #[test]
    fn a_prune_that_would_drop_a_row_fails_the_scan() {
        // The cross-domain d30 orphan prune is asserted inert, not assumed
        // inert: a d32 wing with no d30 `air_wings` row would be removed by
        // `prune_air_wing_loadout_orphans_for_current_hour` at FRAME_END, and a
        // harness that measured the un-pruned state would report a number for a
        // state the recorder never held.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D30,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d30_payload(0, &[(7, 99, 1)]),
        ));
        s.extend(record(
            2,
            100,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(0, &[wing_row(7, 11, 3, 0, 408, 0, 0)], &[]),
        ));
        s.extend(frame_end(3, 100));
        let err = scan_stream(&schema(), &s, StreamIdentityMode::Legacy)
            .expect_err("the prune would bite");
        assert!(err.contains("orphan prune"), "{err}");

        // With the matching d30 row present it is inert and the hour measures.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D30,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d30_payload(0, &[(7, 11, 3)]),
        ));
        s.extend(record(
            2,
            100,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(0, &[wing_row(7, 11, 3, 0, 408, 0, 0)], &[]),
        ));
        s.extend(frame_end(3, 100));
        assert_eq!(
            scan_stream(&schema(), &s, StreamIdentityMode::Legacy)
                .expect("inert")
                .hours
                .len(),
            1
        );
    }

    #[test]
    fn a_d30_delete_registers_its_key_for_the_prune() {
        // d30's delete path writes the same orphan set (`world_fold.rs:14657`),
        // so a wing deleted in d30 while d32 still holds it must be caught even
        // though d32 mentioned nothing this hour.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D30,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d30_payload(0, &[(7, 11, 3)]),
        ));
        s.extend(record(
            2,
            100,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(0, &[wing_row(7, 11, 3, 0, 408, 0, 0)], &[]),
        ));
        s.extend(frame_end(3, 100));
        // Hour 101: d30 deletes the wing, d32 says nothing.
        let mut deleted = d30_payload(0, &[(7, 11, 3)]);
        let flags_at = AirWingRoster::HEADER_LEN + 16;
        deleted[flags_at..flags_at + 4].copy_from_slice(&1u32.to_le_bytes());
        deleted[24..28].copy_from_slice(&1u32.to_le_bytes()); // deleted_rows
        s.extend(record(
            4,
            101,
            D30,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &deleted,
        ));
        s.extend(record(
            5,
            101,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(0, &[], &[]),
        ));
        s.extend(frame_end(6, 101));
        let err = scan_stream(&schema(), &s, StreamIdentityMode::Legacy)
            .expect_err("the prune would bite");
        assert!(err.contains("orphan prune"), "{err}");
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D32,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d32_payload(0, &[], &[]),
            )
        };
        for (emitted, scheduled, reserved) in [(0, 1, 0), (1, 0, 0), (0, 0, 1)] {
            let mut s = data(1, 100);
            s.extend(frame_end_sealed(2, 100, emitted, scheduled, 0, reserved));
            assert!(scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err());
        }
        // Bit 15 is ASSIGNED (`COMPACT_STATUS_BASELINE_CLEAR`, the stream's deliberate
        // resync primitive), so the UNASSIGNED run starts at 16.
        for bit in 16..32u32 {
            let mut s = data(1, 100);
            s.extend(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
            assert!(scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err());
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = data(1, 100);
            s.extend(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
            assert!(scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err());
        }
        for len in [0usize, 1, 16, 31, 33, 64] {
            let mut s = data(1, 100);
            s.extend(record(
                2,
                100,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &vec![0; len],
            ));
            assert!(scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err());
        }
        for status in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = data(1, 100);
            s.extend(frame_end_sealed(2, 100, 0, 0, status, 0));
            assert!(scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_ok());
        }
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D32,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d32_payload(0, &[], &[]),
            )
        };
        let mut ok = perf(1, 100, 20);
        ok.extend(perf(2, 100, D32));
        ok.extend(perf(3, 100, PROBE_DOMAIN_ID));
        ok.extend(data(4, 100));
        ok.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert!(scan_stream(&schema(), &ok, StreamIdentityMode::Legacy).is_ok());

        let mut probe_twice = perf(1, 100, PROBE_DOMAIN_ID);
        probe_twice.extend(perf(2, 100, PROBE_DOMAIN_ID));
        probe_twice.extend(data(3, 100));
        probe_twice.extend(frame_end(4, 100));
        assert!(scan_stream(&schema(), &probe_twice, StreamIdentityMode::Legacy).is_ok());

        let mut cases = Vec::new();
        let mut duplicate = perf(1, 100, 20);
        duplicate.extend(perf(2, 100, 20));
        duplicate.extend(data(3, 100));
        duplicate.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        cases.push(duplicate);
        let mut interleaved = data(1, 100);
        interleaved.extend(perf(2, 101, 20));
        interleaved.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        cases.push(interleaved);
        let mut stale = data(1, 100);
        stale.extend(frame_end(2, 100));
        stale.extend(perf(3, 100, 20));
        stale.extend(data(4, 101));
        stale.extend(frame_end_sealed(5, 101, 1, 1, 0, 0));
        cases.push(stale);
        let mut conflict = perf(1, 100, 20);
        conflict.extend(perf(2, 101, 21));
        conflict.extend(data(3, 100));
        conflict.extend(frame_end_sealed(4, 100, 2, 2, 0, 0));
        cases.push(conflict);
        for (emitted, scheduled) in [(1, 1), (3, 3), (2, 3), (3, 2)] {
            let mut counts = perf(1, 100, 20);
            counts.extend(perf(2, 100, D32));
            counts.extend(data(3, 100));
            counts.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            cases.push(counts);
        }
        for s in cases {
            assert!(scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err());
        }

        let mut two_hours = perf(1, 100, 20);
        two_hours.extend(data(2, 100));
        two_hours.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        two_hours.extend(perf(4, 101, 20));
        two_hours.extend(data(5, 101));
        two_hours.extend(frame_end_sealed(6, 101, 1, 1, 0, 0));
        assert!(scan_stream(&schema(), &two_hours, StreamIdentityMode::Legacy).is_ok());
    }

    #[test]
    fn a_data_record_must_open_the_hour_the_pending_perf_records_bound() {
        for (perf_hour, data_hour) in [(100, 101), (101, 100), (100, 200), (200, 100)] {
            let mut s = perf(1, perf_hour, 20);
            s.extend(record(
                2,
                data_hour,
                D32,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d32_payload(0, &[], &[]),
            ));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err());
        }
        let mut agreeing = perf(1, 100, 20);
        agreeing.extend(record(
            2,
            100,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(0, &[], &[]),
        ));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        scan_stream(&schema(), &agreeing, StreamIdentityMode::Legacy)
            .unwrap_or_else(|e| panic!("matching pending PERF hour must be accepted: {e}"));
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        let failed = 1u32;
        let mut bad = Vec::new();
        bad.push(d32_payload(
            failed,
            &[wing_row(7, 11, 3, d32::WING_FLAG_DELETED, 0, 0, 0)],
            &[],
        ));
        let mut short = d32_payload(failed, &[wing_row(7, 11, 3, 0, 408, 0, 0)], &[]);
        short.pop();
        bad.push(short);
        bad.push(d32_payload(
            failed,
            &[wing_row(0, 11, 3, 0, 408, 0, 0)],
            &[],
        ));
        bad.push(d32_payload(
            failed,
            // Was owner `1_025`, one past the old `1_024` content guess; that
            // is an ordinary country id now, so the probe moves to the real
            // boundary.
            &[wing_row(i32::MAX as u32, 11, 3, 0, 408, 0, 0)],
            &[],
        ));
        bad.push(d32_payload(failed, &[wing_row(7, 0, 3, 0, 408, 0, 0)], &[]));
        bad.push(d32_payload(
            failed,
            &[wing_row(7, u32::MAX, 3, 0, 408, 0, 0)],
            &[],
        ));
        bad.push(d32_payload(
            failed,
            &[
                wing_row(7, 11, 3, 0, 408, 0, 0),
                wing_row(7, 11, 3, 0, 408, 0, 0),
            ],
            &[],
        ));
        bad.push(d32_payload(
            failed,
            &[wing_row(7, 11, 3, 1 << 5, 408, 0, 0)],
            &[],
        ));
        bad.push(d32_payload(
            failed,
            &[wing_row(7, 11, 3, d32::WING_FLAG_DELETED, 408, 0, 0)],
            &[],
        ));
        bad.push(d32_payload(failed, &[wing_row(7, 11, 3, 0, 0, 0, 0)], &[]));
        bad.push(d32_payload(
            failed,
            &[wing_row(7, 11, 3, 0, u32::MAX, 0, 0)],
            &[],
        ));
        bad.push(d32_payload(
            failed,
            &[wing_row(7, 11, 3, 0, 408, 0, 2)],
            &[entry_row(900, 1)],
        ));
        bad.push(d32_payload(
            failed,
            &[
                wing_row(7, 11, 3, 0, 408, 0, 1),
                wing_row(7, 12, 3, 0, 408, 0, 1),
            ],
            &[entry_row(900, 1)],
        ));
        bad.push(d32_payload(
            failed,
            &[wing_row(7, 11, 3, 0, 408, 0, 1)],
            &[entry_row(0, 1)],
        ));
        bad.push(d32_payload(
            failed,
            &[wing_row(7, 11, 3, 0, 408, 0, 1)],
            &[entry_row(u32::MAX, 1)],
        ));
        bad.push(d32_payload(
            failed,
            &[wing_row(7, 11, 3, 0, 408, 0, 1)],
            &[entry_row(900, -1)],
        ));
        bad.push(d32_payload(
            failed,
            &[wing_row(7, 11, 3, 0, 408, 0, 1)],
            &[entry_row(900, d32::PRODUCER_MAX_AMOUNT_RAW + 1)],
        ));
        let mut reserved = d32_payload(
            failed,
            &[wing_row(7, 11, 3, 0, 408, 0, 1)],
            &[entry_row(900, 1)],
        );
        let at = d32::HEADER_LEN + d32::WING_ROW_LEN + 4;
        reserved[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
        bad.push(reserved);
        bad.push(d32_payload(
            failed,
            &[wing_row(7, 11, 3, 0, 408, 0, 2)],
            &[entry_row(900, 1), entry_row(900, 2)],
        ));
        // Every bad payload rides in a stream that WOULD otherwise measure: a
        // second, well-formed hour follows it. Without that trailing hour the
        // scan refuses for an unrelated reason — "no usable d32 DATA records" —
        // and the assertion would hold with `check_authority_payload` deleted,
        // which is a test that cannot fail for its own cause.
        for payload in bad {
            let mut s = record(1, 100, D32, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload);
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D32,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d32_payload(0, &[], &[]),
            ));
            s.extend(frame_end(4, 101));
            assert!(scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err());
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let payload = d32_payload(1 << bit, &[wing_row(0, 11, 3, 0, 408, 0, 0)], &[]);
            let mut s = record(1, 100, D32, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload);
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D32,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d32_payload(0, &[], &[]),
            ));
            s.extend(frame_end(4, 101));
            assert!(scan_stream(&schema(), &s, StreamIdentityMode::Legacy).is_err());
        }

        // The authority walks only equipment rows reached from live ranges
        // (`domain_data_authority.rs:6776`-`:6815`). The fold checks every entry,
        // but failure exclusion prevents that fold-only pass. Keep one unreferenced
        // entry malformed to prove this harness stops at production's authority.
        let rows = [
            wing_row(7, 11, 3, 0, 408, 0, 1),
            wing_row(7, 12, 3, 0, 408, 0, 1),
        ];
        let mut allowed = d32_payload(failed, &rows, &[entry_row(900, 1), entry_row(901, 2)]);
        let unreferenced_reserved =
            d32::HEADER_LEN + 2 * d32::WING_ROW_LEN + d32::ENTRY_ROW_LEN + 4;
        allowed[unreferenced_reserved..unreferenced_reserved + 4]
            .copy_from_slice(&1u32.to_le_bytes());
        let mut s = record(1, 100, D32, COMPACT_RECORD_TYPE_DOMAIN_DATA, &allowed);
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(0, &[], &[]),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&schema(), &s, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let legacy = good_stream();
        assert!(scan_stream(&schema(), &legacy, StreamIdentityMode::IdentityV1).is_err());
        assert!(scan_stream(&schema(), &legacy, StreamIdentityMode::Legacy).is_ok());
        let mut identity = record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        );
        identity.extend(good_stream());
        assert!(scan_stream(&schema(), &identity, StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: the harness reads a file from disk and must
        // return a diagnosis, never unwind.
        let schema = schema();
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..4_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 400;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            if len >= 4 && state & 1 == 0 {
                bytes[..4].copy_from_slice(b"H4CR");
            }
            let _ = scan_stream(&schema, &bytes, StreamIdentityMode::Legacy);
        }
        // And corruption of a genuinely well-formed stream, byte by byte.
        let good = good_stream();
        for i in 0..good.len() {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = scan_stream(&schema, &s, StreamIdentityMode::Legacy);
        }
    }

    #[test]
    fn the_mirrored_delta_bit_matches_the_shipping_contract() {
        assert_eq!(d32::STATUS_DELTA_DATA, COMPACT_STATUS_DELTA_DATA);
    }

    #[test]
    fn the_roster_mirrors_the_air_wings_baseline_rule() {
        // d30 is sparse-after-first too, so a later frame must not be read as a
        // second full replace — that would empty the roster and make the prune
        // assertion fire on a healthy stream.
        let mut roster = AirWingRoster::default();
        roster.fold(&d30_payload(0, &[(7, 11, 3)])).expect("folds");
        assert_eq!(roster.keys.len(), 1);
        roster.fold(&d30_payload(0, &[(7, 12, 4)])).expect("folds");
        assert_eq!(
            roster.keys.len(),
            2,
            "the first wing survives a later frame"
        );
        // A failure status folds nothing.
        roster.fold(&d30_payload(1, &[])).expect("folds");
        assert_eq!(roster.keys.len(), 2);
        // A malformed payload is an error, not a silent skip.
        assert!(roster.fold(&[]).is_err());
        let mut short = d30_payload(0, &[(7, 11, 3)]);
        short.pop();
        assert!(roster.fold(&short).is_err());
    }

    #[test]
    fn a_failure_status_d30_record_is_still_structurally_validated() {
        // Production's d30 payload authority (`decode_compact_air_wings_data_payload`,
        // `domain_data_authority.rs:6257`) re-checks the payload length and walks
        // EVERY row whatever the status word says — the authority is
        // status-independent. A mirror that returned on the failure status before
        // those checks would accept, on exactly the failure-status records, a
        // malformed d30 payload the recorder itself refuses (the d94 lesson:
        // a fold that returns early on failure status must not skip authority
        // validation).
        let mut roster = AirWingRoster::default();

        // Truncated body under a failure status: refused, not skipped.
        let mut short = d30_payload(1 << 0, &[(1, 2, 3)]);
        short.pop();
        assert!(
            roster.fold(&short).is_err(),
            "the length is validated before the status classifies the record"
        );

        // An unknown row flag under a failure status: refused, not skipped.
        let mut bad_flags = d30_payload(1 << 0, &[(1, 2, 3)]);
        let flags_at = AirWingRoster::HEADER_LEN + 16;
        bad_flags[flags_at..flags_at + 4].copy_from_slice(&(1u32 << 7).to_le_bytes());
        assert!(
            roster.fold(&bad_flags).is_err(),
            "row flags are validated before the status classifies the record"
        );

        // And a WELL-FORMED failure record still folds nothing and retains.
        let mut roster = AirWingRoster::default();
        roster.fold(&d30_payload(0, &[(1, 2, 3)])).expect("baseline folds");
        roster
            .fold(&d30_payload(1 << 0, &[(9, 9, 9)]))
            .expect("a well-formed failure record is excluded, not refused");
        assert!(roster.keys.contains(&(1, 2, 3)), "the accumulator is retained");
        assert!(
            !roster.keys.contains(&(9, 9, 9)),
            "the failure frame folded nothing"
        );
    }

    #[test]
    fn a_key_is_never_silently_dropped_by_the_wire_key_arity() {
        // The Key type carries the generation as a full u64; a truncation to u32
        // would silently merge two generations of one wing.
        let mut s = Vec::new();
        let big = 0x1_0000_0003u64;
        s.extend(record(
            1,
            100,
            D30,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d30_payload(0, &[(7, 11, 3), (7, 11, big)]),
        ));
        s.extend(record(
            2,
            100,
            D32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d32_payload(
                0,
                &[
                    wing_row(7, 11, 3, 0, 408, 0, 1),
                    wing_row(7, 11, big, 0, 409, 1, 1),
                ],
                &[entry_row(900, 1), entry_row(900, 2)],
            ),
        ));
        s.extend(frame_end(3, 100));
        let scan = scan_stream(&schema(), &s, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours[0].wings.len(), 2, "two generations, two rows");
        assert_eq!(scan.hours[0].equipment.len(), 2);
        let keys: Vec<&Key> = scan.hours[0].equipment.keys().collect();
        assert_eq!(*keys[0], d32::equipment_key(7, 11, 3, 900));
        assert_eq!(*keys[1], d32::equipment_key(7, 11, big, 900));
    }

    #[test]
    fn a_gap_between_measured_hours_forces_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
        // the next state observation to be a `BASELINE`." An excluded or missing
        // hour between two measured ones is such a gap; delta-encoding across it
        // would decode cleanly while asserting a continuity the capture does not
        // have. The decision function, over its whole input space:
        for hours_seen in 0..(2 * BLOCK_HOURS) {
            assert!(
                opens_baseline(hours_seen, None, 100),
                "the first measured hour at position {hours_seen} opens a baseline"
            );
            assert!(
                opens_baseline(hours_seen, Some(100), 102),
                "a skipped hour at position {hours_seen} must open a baseline"
            );
            assert!(
                opens_baseline(hours_seen, Some(100), 100),
                "a non-advancing hour at position {hours_seen} must open a baseline"
            );
            assert_eq!(
                opens_baseline(hours_seen, Some(100), 101),
                hours_seen % BLOCK_HOURS == 0,
                "a contiguous hour at position {hours_seen} is cadence-only"
            );
        }
        // The clock edge cannot wrap into a false "contiguous".
        assert!(opens_baseline(1, Some(i32::MAX), i32::MIN));
    }

    #[test]
    fn an_hour_closed_without_a_d32_record_refuses_the_stream() {
        // The completeness bound on what this harness does NOT validate (module
        // docs): it cannot check other domains' payloads, so a d32 record
        // relabelled to another domain id — or dropped — must be caught by
        // counting, not silently measured as fewer hours.
        let mut s = good_stream();
        // Find the LAST d32 DATA record and relabel it to an unclaimed domain
        // id. The stream stays well formed — sequence, hour boundaries and
        // FRAME_END are untouched — so the completeness rule is the only rule
        // that can catch it.
        let mut pos = 0usize;
        let mut target = None;
        while pos < s.len() {
            let header = parse_compact_record_header(&s[pos..]).expect("well-formed");
            if header.domain_id == D32 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            {
                target = Some(pos);
            }
            pos += COMPACT_RECORD_HEADER_LEN + header.payload_len;
        }
        let target = target.expect("good_stream carries a d32 DATA record");
        // domain_id sits at +20: magic(4) + version(2) + header_len(2) +
        // sequence(8) + game_hour(4).
        s[target + 20..target + 22].copy_from_slice(&99u16.to_le_bytes());
        let err = match scan_stream(&schema(), &s, StreamIdentityMode::Legacy) {
            Err(e) => e,
            Ok(_) => panic!("an hour with no d32 record must refuse the stream"),
        };
        assert!(
            err.contains("game hour(s) but carries"),
            "the completeness check must be what rejects it: {err}"
        );
    }
}
