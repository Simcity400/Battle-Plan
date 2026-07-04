use std::collections::{BTreeMap, BTreeSet};

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const DOMAIN_LAND_COMBAT_DETAIL_EVENTS: u16 = 55;
pub const DOMAIN_LAND_COMBAT_DETAIL_EVENTS_NAME: &str = "land_combat_detail_events_live";

// Schema v2 (E2) appends a per-side combat DETAIL row family AFTER the event
// rows, extending the header with a side-detail count. v1 payloads (no side
// detail) remain decodable: a v1 header omits the trailing count field, so the
// version gate selects the header/parse shape (never a payload-byte sniff).
pub const SCHEMA_VERSION: u16 = 2;
pub const SCHEMA_VERSION_V1_NO_SIDE_DETAIL: u16 = 1;
/// v1 header (event rows only). Kept for decoding older streams.
pub const PAYLOAD_HEADER_LEN_V1: usize = 28;
/// v2 header adds a u32 side_detail_row_count after the event-row count block.
pub const PAYLOAD_HEADER_LEN: usize = 32;
pub const DETAIL_EVENT_ROW_LEN: usize = 52;

// --- Per-side combat DETAIL row (E2, CCombatSide scalars) -----------------
//
// Current-state (NOT delta): one row per active combat per side, emitted every
// frame for every live combat, so the fold always has the latest per-side
// scalars regardless of whether the participant/phase delta changed this hour.
// Offsets validated 50/50 on both sides vs the melt (FINAL_SWEEP §T2). Damage
// fields are i64 fixed-point ×100000; counts are u32. `tactic` is PARTIAL
// (pointer/table-backed, no inline offset) and `losses` is not disambiguable
// on this all-zero-loss save — both are DELIBERATELY OMITTED (badged).
/// Side-detail row wire length. Layout (all little-endian):
///   combat_id u32 | generation u64 | side u32 | flags u32           (20)
///   units u32 | committed u32 | reserve u32 | air_kills u32
///     | org_loss_summary_index u32 | num_org_losses u32             (+24 = 44)
///   7 × i64 damage raws: anti_air, air_str, air_org, ground_str,
///     ground_org, prevented_str, prevented_org                      (+56 = 100)
pub const SIDE_DETAIL_ROW_LEN: usize = 100;
pub const SIDE_DETAIL_FIXED_SCALE: i64 = 100_000;

pub const SIDE_DETAIL_FLAG_ATTACKER: u32 = 1 << 0;
pub const SIDE_DETAIL_FLAG_DEFENDER: u32 = 1 << 1;
/// The CCombatSide vtable + registry back-ref revalidated for this side.
pub const SIDE_DETAIL_FLAG_SIDE_VALIDATED: u32 = 1 << 2;
/// tactic is unresolved (pointer/table-backed) — never emitted as a value.
pub const SIDE_DETAIL_FLAG_TACTIC_UNRESOLVED: u32 = 1 << 3;
/// losses is not disambiguable on this save (all-zero oracle) — omitted.
pub const SIDE_DETAIL_FLAG_LOSSES_OMITTED: u32 = 1 << 4;
/// The count triple was read STABLY (each field double-read agreed) but the
/// `committed + reserve == units` invariant did not hold — i.e. a genuine
/// mid-combat transient (a division joining/leaving between the engine's own
/// bookkeeping), NOT a torn read. The row is retained (dropping it would lose
/// the side exactly at the interesting transition) but badged so a consumer
/// treats the count triple as momentarily inconsistent, not authoritative.
pub const SIDE_DETAIL_FLAG_COUNTS_INCOMPLETE: u32 = 1 << 5;
pub const SIDE_DETAIL_KNOWN_FLAG_MASK: u32 = SIDE_DETAIL_FLAG_ATTACKER
    | SIDE_DETAIL_FLAG_DEFENDER
    | SIDE_DETAIL_FLAG_SIDE_VALIDATED
    | SIDE_DETAIL_FLAG_TACTIC_UNRESOLVED
    | SIDE_DETAIL_FLAG_LOSSES_OMITTED
    | SIDE_DETAIL_FLAG_COUNTS_INCOMPLETE;

/// Sanity ceiling on a fixed-point damage accumulator (±~9.2e12 raw = ±92M
/// after the /1e5 scale — generous for a single combat, still rejects garbage).
pub const MAX_SIDE_DETAIL_DAMAGE_RAW: i64 = 9_200_000_000_000;
pub const MAX_SIDE_DETAIL_COUNT: u32 = 100_000;

pub const STATUS_DELTA_DATA: u32 = compact_contract::COMPACT_STATUS_DELTA_DATA;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE;
pub const STATUS_PARTIAL_DATA: u32 = compact_contract::COMPACT_STATUS_PARTIAL_DATA;
pub const STATUS_TRANSITION_BASELINE_RESET: u32 =
    compact_contract::COMPACT_STATUS_TRANSITION_BASELINE_RESET;
pub const FAILURE_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_FAILURE_MASK;

pub const EVENT_PHASE_STARTED: u32 = 1;
pub const EVENT_PHASE_UPDATED: u32 = 2;
pub const EVENT_PHASE_ENDED: u32 = 3;
pub const EVENT_PARTICIPANT_JOINED: u32 = 4;
pub const EVENT_PARTICIPANT_LEFT_UNCLASSIFIED: u32 = 5;

pub const SIDE_NONE: u32 = 0;
pub const SIDE_ATTACKER: u32 = 1;
pub const SIDE_DEFENDER: u32 = 2;

pub const FLAG_LOCATION_PRESENT: u32 = 1 << 0;
pub const FLAG_DAY_PRESENT: u32 = 1 << 1;
pub const FLAG_DURATION_PRESENT: u32 = 1 << 2;
pub const FLAG_SIDE_PRESENT: u32 = 1 << 3;
pub const FLAG_DIVISION_PRESENT: u32 = 1 << 4;
pub const FLAG_FROM_VALIDATED_DOMAIN21_ROOT: u32 = 1 << 5;
pub const FLAG_SEMANTIC_UNCLASSIFIED: u32 = 1 << 6;
pub const KNOWN_FLAG_MASK: u32 = FLAG_LOCATION_PRESENT
    | FLAG_DAY_PRESENT
    | FLAG_DURATION_PRESENT
    | FLAG_SIDE_PRESENT
    | FLAG_DIVISION_PRESENT
    | FLAG_FROM_VALIDATED_DOMAIN21_ROOT
    | FLAG_SEMANTIC_UNCLASSIFIED;

pub const MAX_COMBAT_ID: u32 = 0x7fff_ffff;
pub const MAX_DIVISION_ID: u32 = 0x7fff_ffff;
pub const MAX_LOCATION_ID: u32 = 1_000_000;
pub const MAX_AGE_DAYS: u32 = 100_000;
pub const MAX_SIDE_UNITS: usize = 128;
/// Two sides per combat; bound the side-detail family the same as MAX_LAND_COMBATS.
pub const MAX_LAND_COMBATS_SIDE_DETAIL: usize = 8_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CombatSide {
    Attacker,
    Defender,
}

impl CombatSide {
    pub fn wire_id(self) -> u32 {
        match self {
            Self::Attacker => SIDE_ATTACKER,
            Self::Defender => SIDE_DEFENDER,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SideDetail {
    /// Total units on the side (side+0x2C = committed + reserve). 100/100.
    pub units: u32,
    /// Committed (front-line) count (side+0xD4). 100/100, values 1..6.
    pub committed: u32,
    /// Reserve count (side+0xEC). 100/100.
    pub reserve: u32,
    /// Air-to-ground kills (side+0x140). u32.
    pub air_kills: u32,
    /// Org-loss summary index (side+0x1A8). u32.
    pub org_loss_summary_index: u32,
    /// Number of org losses = combat day count (side+0x1AC). u32.
    pub num_org_losses: u32,
    /// i64 ×1e5 damage accumulators (side+0x138..+0x170).
    pub anti_air_attack_raw: i64,
    pub air_damage_str_raw: i64,
    pub air_damage_org_raw: i64,
    pub ground_damage_str_raw: i64,
    pub ground_damage_org_raw: i64,
    pub prevented_damage_str_raw: i64,
    pub prevented_damage_org_raw: i64,
    /// True when the stably-read count triple violated committed+reserve==units
    /// (a genuine mid-combat transient). The row is still emitted, badged.
    pub counts_incomplete: bool,
}

/// A per-side current-state detail row for one combat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SideDetailRow {
    pub combat_id: u32,
    pub generation: u64,
    pub side: CombatSide,
    pub detail: SideDetail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LandCombatSnapshot {
    pub combat_id: u32,
    pub generation: u64,
    pub location: Option<u32>,
    pub day: Option<u32>,
    pub duration: Option<u32>,
    pub attacker_divisions: Vec<u32>,
    pub defender_divisions: Vec<u32>,
    /// E2: per-side CCombatSide detail scalars. `None` when the side object
    /// failed revalidation (fail closed — absent, never zero-filled).
    pub attacker_detail: Option<SideDetail>,
    pub defender_detail: Option<SideDetail>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetailEventRow {
    pub combat_id: u32,
    pub generation: u64,
    pub event_kind: u32,
    pub side: Option<CombatSide>,
    pub division_id: Option<u32>,
    pub location: Option<u32>,
    pub day: Option<u32>,
    pub duration: Option<u32>,
    pub old_side_count: u32,
    pub new_side_count: u32,
    pub flags: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GateSummary {
    pub record_count: u64,
    pub row_count: u64,
    pub phase_started_rows: u64,
    pub phase_updated_rows: u64,
    pub phase_ended_rows: u64,
    pub participant_joined_rows: u64,
    pub participant_left_unclassified_rows: u64,
    pub unclassified_rows: u64,
    pub side_detail_rows: u64,
    pub domains_seen: BTreeSet<u16>,
    pub status_flags_or: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateResult {
    pub ok: bool,
    pub errors: Vec<String>,
    pub summary: GateSummary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureFrame {
    pub compact_stream: Vec<u8>,
    pub rows: Vec<DetailEventRow>,
    /// E2: per-side current-state detail rows (one per active combat per side).
    pub side_detail_rows: Vec<SideDetailRow>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LandCombatDetailTracker {
    previous: Vec<LandCombatSnapshot>,
    pending_rebaseline: bool,
    last_frame_rebaselined: bool,
}

impl LandCombatDetailTracker {
    pub fn reset_transition_baseline(&mut self) {
        self.previous.clear();
        self.pending_rebaseline = true;
        self.last_frame_rebaselined = false;
    }

    pub fn last_frame_rebaselined(&self) -> bool {
        self.last_frame_rebaselined
    }
}

pub trait LandCombatDetailMemory {
    fn land_combat_snapshots(&self) -> Result<Vec<LandCombatSnapshot>, Vec<String>>;
}

pub fn collect_transition_capture_frame<M: LandCombatDetailMemory>(
    memory: &M,
    tracker: &mut LandCombatDetailTracker,
    sequence: u64,
    game_hour_raw: i32,
    elapsed_wall_ms: u64,
) -> Result<CaptureFrame, Vec<String>> {
    let current = memory.land_combat_snapshots()?;
    if tracker.pending_rebaseline {
        // The transition BASELINE reset zeroes the event-delta history so no
        // synthetic join/leave events are fabricated across the gap. Per-side
        // detail is CURRENT-STATE, not a delta, so it is still emitted on the
        // rebaseline frame — the fold needs the live scalars every hour.
        let side_detail_rows = side_detail_rows_from_snapshots(&current);
        let payload = encode_detail_events_payload_v2(
            elapsed_wall_ms,
            STATUS_DELTA_DATA | STATUS_PARTIAL_DATA | STATUS_TRANSITION_BASELINE_RESET,
            &[],
            &side_detail_rows,
        );
        let mut compact_stream = Vec::new();
        append_compact_record(
            &mut compact_stream,
            sequence,
            game_hour_raw,
            DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        tracker.previous = current;
        tracker.pending_rebaseline = false;
        tracker.last_frame_rebaselined = true;
        return Ok(CaptureFrame {
            compact_stream,
            rows: Vec::new(),
            side_detail_rows,
        });
    }
    let frame = build_transition_capture_frame(
        tracker.previous.as_slice(),
        current.as_slice(),
        sequence,
        game_hour_raw,
        elapsed_wall_ms,
    )?;
    tracker.previous = current;
    tracker.last_frame_rebaselined = false;
    Ok(frame)
}

/// Build the per-side current-state detail rows for every active combat that
/// carries validated CCombatSide detail. A side whose detail failed
/// revalidation contributes NO row (fail closed; absent, not zero).
pub fn side_detail_rows_from_snapshots(current: &[LandCombatSnapshot]) -> Vec<SideDetailRow> {
    let mut rows = Vec::new();
    for snapshot in current {
        if let Some(detail) = snapshot.attacker_detail {
            rows.push(SideDetailRow {
                combat_id: snapshot.combat_id,
                generation: snapshot.generation,
                side: CombatSide::Attacker,
                detail,
            });
        }
        if let Some(detail) = snapshot.defender_detail {
            rows.push(SideDetailRow {
                combat_id: snapshot.combat_id,
                generation: snapshot.generation,
                side: CombatSide::Defender,
                detail,
            });
        }
    }
    rows
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedDetailEventsPayload {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub rows: Vec<DetailEventRow>,
    /// E2: per-side current-state detail rows (empty for v1 streams).
    pub side_detail_rows: Vec<SideDetailRow>,
}

pub fn build_transition_capture_frame(
    previous: &[LandCombatSnapshot],
    current: &[LandCombatSnapshot],
    sequence: u64,
    game_hour_raw: i32,
    elapsed_wall_ms: u64,
) -> Result<CaptureFrame, Vec<String>> {
    let rows = build_detail_event_rows(previous, current)?;
    let side_detail_rows = side_detail_rows_from_snapshots(current);
    let payload = encode_detail_events_payload_v2(
        elapsed_wall_ms,
        STATUS_DELTA_DATA,
        &rows,
        &side_detail_rows,
    );
    let mut compact_stream = Vec::new();
    append_compact_record(
        &mut compact_stream,
        sequence,
        game_hour_raw,
        DOMAIN_LAND_COMBAT_DETAIL_EVENTS,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &payload,
    );
    Ok(CaptureFrame {
        compact_stream,
        rows,
        side_detail_rows,
    })
}

pub fn build_detail_event_rows(
    previous: &[LandCombatSnapshot],
    current: &[LandCombatSnapshot],
) -> Result<Vec<DetailEventRow>, Vec<String>> {
    let mut errors = Vec::new();
    for row in previous.iter().chain(current.iter()) {
        if let Err(err) = validate_snapshot(row) {
            errors.push(err);
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let previous_by_key = previous
        .iter()
        .map(|row| ((row.combat_id, row.generation), row))
        .collect::<BTreeMap<_, _>>();
    let current_by_key = current
        .iter()
        .map(|row| ((row.combat_id, row.generation), row))
        .collect::<BTreeMap<_, _>>();

    let mut rows = Vec::new();
    for (key, now) in &current_by_key {
        match previous_by_key.get(key) {
            None => rows.push(phase_row(EVENT_PHASE_STARTED, now)),
            Some(old) if phase_changed(old, now) => rows.push(phase_row(EVENT_PHASE_UPDATED, now)),
            Some(_) => {}
        }
        if let Some(old) = previous_by_key.get(key).copied() {
            push_side_delta_rows(
                &mut rows,
                old.attacker_divisions.as_slice(),
                &now.attacker_divisions,
                now,
                CombatSide::Attacker,
            );
            push_side_delta_rows(
                &mut rows,
                old.defender_divisions.as_slice(),
                &now.defender_divisions,
                now,
                CombatSide::Defender,
            );
        }
    }
    for (key, old) in &previous_by_key {
        if !current_by_key.contains_key(key) {
            rows.push(phase_row(EVENT_PHASE_ENDED, old));
        }
    }
    Ok(rows)
}

/// v1 encoder (event rows only, no side detail). Retained for the legacy wire
/// shape and older tests; writes SCHEMA_VERSION_V1 and the 28-byte header.
pub fn encode_detail_events_payload(
    elapsed_wall_ms: u64,
    status_flags: u32,
    rows: &[DetailEventRow],
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(PAYLOAD_HEADER_LEN_V1 + rows.len() * DETAIL_EVENT_ROW_LEN);
    payload.extend_from_slice(&SCHEMA_VERSION_V1_NO_SIDE_DETAIL.to_le_bytes());
    payload.extend_from_slice(&(DETAIL_EVENT_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    append_event_rows(&mut payload, rows);
    payload
}

/// v2 encoder (E2): event rows PLUS a per-side detail row family. The 32-byte
/// header appends a u32 side_detail_row_count after the event-row count block;
/// the side-detail rows follow the event rows.
pub fn encode_detail_events_payload_v2(
    elapsed_wall_ms: u64,
    status_flags: u32,
    rows: &[DetailEventRow],
    side_detail_rows: &[SideDetailRow],
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(
        PAYLOAD_HEADER_LEN
            + rows.len() * DETAIL_EVENT_ROW_LEN
            + side_detail_rows.len() * SIDE_DETAIL_ROW_LEN,
    );
    payload.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    payload.extend_from_slice(&(DETAIL_EVENT_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes()); // string_count
    payload
        .extend_from_slice(&(side_detail_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes()); // reserved
    append_event_rows(&mut payload, rows);
    for row in side_detail_rows {
        append_side_detail_row(&mut payload, row);
    }
    payload
}

fn append_event_rows(payload: &mut Vec<u8>, rows: &[DetailEventRow]) {
    for row in rows {
        validate_row(row).expect("invalid detail event row");
        payload.extend_from_slice(&row.combat_id.to_le_bytes());
        payload.extend_from_slice(&row.generation.to_le_bytes());
        payload.extend_from_slice(&row.event_kind.to_le_bytes());
        payload.extend_from_slice(
            &row.side
                .map(CombatSide::wire_id)
                .unwrap_or(SIDE_NONE)
                .to_le_bytes(),
        );
        payload.extend_from_slice(&row.division_id.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.location.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.day.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.duration.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.old_side_count.to_le_bytes());
        payload.extend_from_slice(&row.new_side_count.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
}

/// Encode one side-detail row (SIDE_DETAIL_ROW_LEN bytes; layout on the const).
fn append_side_detail_row(payload: &mut Vec<u8>, row: &SideDetailRow) {
    let mut flags = SIDE_DETAIL_FLAG_SIDE_VALIDATED
        | SIDE_DETAIL_FLAG_TACTIC_UNRESOLVED
        | SIDE_DETAIL_FLAG_LOSSES_OMITTED;
    flags |= match row.side {
        CombatSide::Attacker => SIDE_DETAIL_FLAG_ATTACKER,
        CombatSide::Defender => SIDE_DETAIL_FLAG_DEFENDER,
    };
    if row.detail.counts_incomplete {
        flags |= SIDE_DETAIL_FLAG_COUNTS_INCOMPLETE;
    }
    payload.extend_from_slice(&row.combat_id.to_le_bytes());
    payload.extend_from_slice(&row.generation.to_le_bytes());
    payload.extend_from_slice(&row.side.wire_id().to_le_bytes());
    payload.extend_from_slice(&flags.to_le_bytes());
    payload.extend_from_slice(&row.detail.units.to_le_bytes());
    payload.extend_from_slice(&row.detail.committed.to_le_bytes());
    payload.extend_from_slice(&row.detail.reserve.to_le_bytes());
    payload.extend_from_slice(&row.detail.air_kills.to_le_bytes());
    payload.extend_from_slice(&row.detail.org_loss_summary_index.to_le_bytes());
    payload.extend_from_slice(&row.detail.num_org_losses.to_le_bytes());
    payload.extend_from_slice(&row.detail.anti_air_attack_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.air_damage_str_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.air_damage_org_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.ground_damage_str_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.ground_damage_org_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.prevented_damage_str_raw.to_le_bytes());
    payload.extend_from_slice(&row.detail.prevented_damage_org_raw.to_le_bytes());
}

pub fn gate_stream(data: &[u8]) -> GateResult {
    let mut errors = Vec::new();
    let mut summary = GateSummary::default();
    let mut cursor = 0usize;
    while cursor < data.len() {
        if data.len().saturating_sub(cursor) < COMPACT_RECORD_HEADER_LEN {
            errors.push(format!("offset {cursor}: truncated compact record header"));
            break;
        }
        if &data[cursor..cursor + 4] != COMPACT_RECORD_MAGIC {
            errors.push(format!("offset {cursor}: bad compact record magic"));
            break;
        }
        let version = read_u16(data, cursor + 4);
        let header_len = read_u16(data, cursor + 6) as usize;
        let domain_id = read_u16(data, cursor + 20);
        let record_type = read_u16(data, cursor + 22);
        let payload_len = read_u32(data, cursor + 24) as usize;
        let reserved = read_u32(data, cursor + 28);
        if version != COMPACT_RECORD_VERSION {
            errors.push(format!(
                "offset {cursor}: unsupported compact version {version}"
            ));
        }
        if header_len != COMPACT_RECORD_HEADER_LEN {
            errors.push(format!(
                "offset {cursor}: compact header_len {header_len} != {COMPACT_RECORD_HEADER_LEN}"
            ));
            break;
        }
        if reserved != 0 {
            errors.push(format!(
                "offset {cursor}: compact header reserved is nonzero"
            ));
        }
        let payload_start = cursor + header_len;
        let payload_end = payload_start.saturating_add(payload_len);
        if payload_end > data.len() {
            errors.push(format!("offset {cursor}: payload overruns stream"));
            break;
        }
        summary.record_count = summary.record_count.saturating_add(1);
        summary.domains_seen.insert(domain_id);
        if domain_id == DOMAIN_LAND_COMBAT_DETAIL_EVENTS
            && record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
        {
            decode_detail_events_payload_into_summary(
                cursor,
                &data[payload_start..payload_end],
                &mut summary,
                &mut errors,
            );
        }
        cursor = payload_end;
    }
    if summary.status_flags_or & FAILURE_STATUS_MASK != 0 {
        errors.push(format!(
            "land combat detail events status has failure flags {}",
            summary.status_flags_or & FAILURE_STATUS_MASK
        ));
    }
    GateResult {
        ok: errors.is_empty(),
        errors,
        summary,
    }
}

pub fn result_json(gate: &GateResult) -> String {
    format!(
        concat!(
            "{{\n",
            "  \"ok\": {},\n",
            "  \"errors\": {},\n",
            "  \"record_count\": {},\n",
            "  \"row_count\": {},\n",
            "  \"phase_started_rows\": {},\n",
            "  \"phase_updated_rows\": {},\n",
            "  \"phase_ended_rows\": {},\n",
            "  \"participant_joined_rows\": {},\n",
            "  \"participant_left_unclassified_rows\": {},\n",
            "  \"unclassified_rows\": {},\n",
            "  \"status_flags_or\": {}\n",
            "}}\n"
        ),
        gate.ok,
        json_string_array(&gate.errors),
        gate.summary.record_count,
        gate.summary.row_count,
        gate.summary.phase_started_rows,
        gate.summary.phase_updated_rows,
        gate.summary.phase_ended_rows,
        gate.summary.participant_joined_rows,
        gate.summary.participant_left_unclassified_rows,
        gate.summary.unclassified_rows,
        gate.summary.status_flags_or
    )
}

pub fn decode_detail_events_payload(
    offset: usize,
    payload: &[u8],
) -> Result<DecodedDetailEventsPayload, Vec<String>> {
    let mut errors = Vec::new();
    // The version selects the header/parse shape (framing-level, never a
    // payload-byte sniff). v1 = 28-byte header, event rows only. v2 = 32-byte
    // header (adds side_detail_row_count) + a trailing per-side detail family.
    if payload.len() < 4 {
        return Err(vec![format!("offset {offset}: detail payload too short")]);
    }
    let version = read_u16(payload, 0);
    let (header_len, side_detail_row_count) = match version {
        SCHEMA_VERSION_V1_NO_SIDE_DETAIL => (PAYLOAD_HEADER_LEN_V1, 0usize),
        SCHEMA_VERSION => {
            if payload.len() < PAYLOAD_HEADER_LEN {
                return Err(vec![format!(
                    "offset {offset}: detail v2 payload too short"
                )]);
            }
            (PAYLOAD_HEADER_LEN, read_u32(payload, 24) as usize)
        }
        other => {
            return Err(vec![format!(
                "offset {offset}: detail schema version {other} unsupported"
            )]);
        }
    };
    if payload.len() < header_len {
        return Err(vec![format!("offset {offset}: detail payload too short")]);
    }
    let row_len = read_u16(payload, 2) as usize;
    let elapsed_wall_ms = read_u64(payload, 4);
    let status_flags = read_u32(payload, 12);
    let row_count = read_u32(payload, 16) as usize;
    let string_count = read_u32(payload, 20);
    // The final u32 of each header is a reserved-zero field (v1 @24, v2 @28).
    let reserved = read_u32(payload, header_len - 4);
    if row_len != DETAIL_EVENT_ROW_LEN {
        errors.push(format!(
            "offset {offset}: detail row_len {row_len} != {DETAIL_EVENT_ROW_LEN}"
        ));
    }
    if string_count != 0 || reserved != 0 {
        errors.push(format!(
            "offset {offset}: detail payload header reserved/string fields nonzero"
        ));
    }
    if side_detail_row_count > MAX_LAND_COMBATS_SIDE_DETAIL {
        errors.push(format!(
            "offset {offset}: side_detail_row_count {side_detail_row_count} exceeds max"
        ));
    }
    let expected_len = header_len
        .saturating_add(row_count.saturating_mul(row_len))
        .saturating_add(side_detail_row_count.saturating_mul(SIDE_DETAIL_ROW_LEN));
    if payload.len() != expected_len {
        errors.push(format!(
            "offset {offset}: detail payload len {} != expected {expected_len}",
            payload.len()
        ));
        return Err(errors);
    }
    let mut seen = BTreeSet::new();
    let mut rows = Vec::with_capacity(row_count);
    let mut cursor = header_len;
    for row_index in 0..row_count {
        let row = DecodedRow {
            combat_id: read_u32(payload, cursor),
            generation: read_u64(payload, cursor + 4),
            event_kind: read_u32(payload, cursor + 12),
            side: read_u32(payload, cursor + 16),
            division_id: read_u32(payload, cursor + 20),
            flags: read_u32(payload, cursor + 24),
            location: read_u32(payload, cursor + 28),
            day: read_u32(payload, cursor + 32),
            duration: read_u32(payload, cursor + 36),
            old_side_count: read_u32(payload, cursor + 40),
            new_side_count: read_u32(payload, cursor + 44),
            reserved: read_u32(payload, cursor + 48),
        };
        cursor += row_len;
        validate_decoded_row(offset, row_index, &row, &mut seen, &mut errors);
        rows.push(DetailEventRow {
            combat_id: row.combat_id,
            generation: row.generation,
            event_kind: row.event_kind,
            side: match row.side {
                SIDE_ATTACKER => Some(CombatSide::Attacker),
                SIDE_DEFENDER => Some(CombatSide::Defender),
                _ => None,
            },
            division_id: if row.flags & FLAG_DIVISION_PRESENT != 0 {
                Some(row.division_id)
            } else {
                None
            },
            location: if row.flags & FLAG_LOCATION_PRESENT != 0 {
                Some(row.location)
            } else {
                None
            },
            day: if row.flags & FLAG_DAY_PRESENT != 0 {
                Some(row.day)
            } else {
                None
            },
            duration: if row.flags & FLAG_DURATION_PRESENT != 0 {
                Some(row.duration)
            } else {
                None
            },
            old_side_count: row.old_side_count,
            new_side_count: row.new_side_count,
            flags: row.flags,
        });
    }

    // v2 per-side detail family (empty for v1).
    let mut side_detail_rows = Vec::with_capacity(side_detail_row_count);
    let mut seen_side = BTreeSet::new();
    for row_index in 0..side_detail_row_count {
        match decode_side_detail_row(offset, row_index, payload, cursor, &mut seen_side) {
            Ok(row) => side_detail_rows.push(row),
            Err(err) => errors.push(err),
        }
        cursor += SIDE_DETAIL_ROW_LEN;
    }

    if errors.is_empty() {
        Ok(DecodedDetailEventsPayload {
            elapsed_wall_ms,
            status_flags,
            rows,
            side_detail_rows,
        })
    } else {
        Err(errors)
    }
}

fn decode_side_detail_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut BTreeSet<(u32, u64, u32)>,
) -> Result<SideDetailRow, String> {
    let combat_id = read_u32(payload, cursor);
    let generation = read_u64(payload, cursor + 4);
    let side_wire = read_u32(payload, cursor + 12);
    let flags = read_u32(payload, cursor + 16);
    let units = read_u32(payload, cursor + 20);
    let committed = read_u32(payload, cursor + 24);
    let reserve = read_u32(payload, cursor + 28);
    let air_kills = read_u32(payload, cursor + 32);
    let org_loss_summary_index = read_u32(payload, cursor + 36);
    let num_org_losses = read_u32(payload, cursor + 40);
    let anti_air_attack_raw = read_i64(payload, cursor + 44);
    let air_damage_str_raw = read_i64(payload, cursor + 52);
    let air_damage_org_raw = read_i64(payload, cursor + 60);
    let ground_damage_str_raw = read_i64(payload, cursor + 68);
    let ground_damage_org_raw = read_i64(payload, cursor + 76);
    let prevented_damage_str_raw = read_i64(payload, cursor + 84);
    let prevented_damage_org_raw = read_i64(payload, cursor + 92);

    if combat_id == 0 || combat_id > MAX_COMBAT_ID {
        return Err(format!(
            "offset {offset}: side detail row {row_index} invalid combat_id"
        ));
    }
    if flags & !SIDE_DETAIL_KNOWN_FLAG_MASK != 0 {
        return Err(format!(
            "offset {offset}: side detail row {row_index} unknown flags {}",
            flags & !SIDE_DETAIL_KNOWN_FLAG_MASK
        ));
    }
    if flags & SIDE_DETAIL_FLAG_SIDE_VALIDATED == 0 {
        return Err(format!(
            "offset {offset}: side detail row {row_index} lacks side-validated flag"
        ));
    }
    let side = match side_wire {
        SIDE_ATTACKER => {
            if flags & SIDE_DETAIL_FLAG_ATTACKER == 0 {
                return Err(format!(
                    "offset {offset}: side detail row {row_index} attacker side/flag mismatch"
                ));
            }
            CombatSide::Attacker
        }
        SIDE_DEFENDER => {
            if flags & SIDE_DETAIL_FLAG_DEFENDER == 0 {
                return Err(format!(
                    "offset {offset}: side detail row {row_index} defender side/flag mismatch"
                ));
            }
            CombatSide::Defender
        }
        other => {
            return Err(format!(
                "offset {offset}: side detail row {row_index} invalid side {other}"
            ));
        }
    };
    for (label, value) in [
        ("units", units),
        ("committed", committed),
        ("reserve", reserve),
        ("air_kills", air_kills),
        ("org_loss_summary_index", org_loss_summary_index),
        ("num_org_losses", num_org_losses),
    ] {
        if value > MAX_SIDE_DETAIL_COUNT {
            return Err(format!(
                "offset {offset}: side detail row {row_index} {label} {value} exceeds max"
            ));
        }
    }
    // The committed+reserve==units invariant is REQUIRED unless the row is
    // explicitly badged COUNTS_INCOMPLETE (a genuine mid-combat transient the
    // collector observed with a stable double-read). Without the badge, a
    // violation is a corrupt/torn payload and is rejected.
    let counts_incomplete = flags & SIDE_DETAIL_FLAG_COUNTS_INCOMPLETE != 0;
    if !counts_incomplete && committed.saturating_add(reserve) != units {
        return Err(format!(
            "offset {offset}: side detail row {row_index} committed+reserve != units without incomplete badge"
        ));
    }
    for (label, raw) in [
        ("anti_air_attack", anti_air_attack_raw),
        ("air_damage_str", air_damage_str_raw),
        ("air_damage_org", air_damage_org_raw),
        ("ground_damage_str", ground_damage_str_raw),
        ("ground_damage_org", ground_damage_org_raw),
        ("prevented_damage_str", prevented_damage_str_raw),
        ("prevented_damage_org", prevented_damage_org_raw),
    ] {
        if !(0..=MAX_SIDE_DETAIL_DAMAGE_RAW).contains(&raw) {
            return Err(format!(
                "offset {offset}: side detail row {row_index} {label} raw {raw} out of range"
            ));
        }
    }
    if !seen.insert((combat_id, generation, side_wire)) {
        return Err(format!(
            "offset {offset}: side detail row {row_index} duplicates (combat,generation,side)"
        ));
    }
    Ok(SideDetailRow {
        combat_id,
        generation,
        side,
        detail: SideDetail {
            units,
            committed,
            reserve,
            air_kills,
            org_loss_summary_index,
            num_org_losses,
            anti_air_attack_raw,
            air_damage_str_raw,
            air_damage_org_raw,
            ground_damage_str_raw,
            ground_damage_org_raw,
            prevented_damage_str_raw,
            prevented_damage_org_raw,
            counts_incomplete,
        },
    })
}

fn decode_detail_events_payload_into_summary(
    offset: usize,
    payload: &[u8],
    summary: &mut GateSummary,
    errors: &mut Vec<String>,
) {
    if payload.len() >= 16 {
        summary.status_flags_or |= read_u32(payload, 12);
    }
    let decoded = match decode_detail_events_payload(offset, payload) {
        Ok(decoded) => decoded,
        Err(mut decode_errors) => {
            errors.append(&mut decode_errors);
            return;
        }
    };
    summary.row_count = summary.row_count.saturating_add(decoded.rows.len() as u64);
    summary.side_detail_rows = summary
        .side_detail_rows
        .saturating_add(decoded.side_detail_rows.len() as u64);
    for row in decoded.rows {
        match row.event_kind {
            EVENT_PHASE_STARTED => summary.phase_started_rows += 1,
            EVENT_PHASE_UPDATED => summary.phase_updated_rows += 1,
            EVENT_PHASE_ENDED => summary.phase_ended_rows += 1,
            EVENT_PARTICIPANT_JOINED => summary.participant_joined_rows += 1,
            EVENT_PARTICIPANT_LEFT_UNCLASSIFIED => {
                summary.participant_left_unclassified_rows += 1;
                summary.unclassified_rows += 1;
            }
            _ => {}
        }
    }
}

#[derive(Debug)]
struct DecodedRow {
    combat_id: u32,
    generation: u64,
    event_kind: u32,
    side: u32,
    division_id: u32,
    flags: u32,
    location: u32,
    day: u32,
    duration: u32,
    old_side_count: u32,
    new_side_count: u32,
    reserved: u32,
}

fn validate_decoded_row(
    offset: usize,
    row_index: usize,
    row: &DecodedRow,
    seen: &mut BTreeSet<(u32, u64, u32, u32, u32)>,
    errors: &mut Vec<String>,
) {
    if row.combat_id == 0 || row.combat_id > MAX_COMBAT_ID {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has invalid combat_id"
        ));
    }
    if !matches!(
        row.event_kind,
        EVENT_PHASE_STARTED
            | EVENT_PHASE_UPDATED
            | EVENT_PHASE_ENDED
            | EVENT_PARTICIPANT_JOINED
            | EVENT_PARTICIPANT_LEFT_UNCLASSIFIED
    ) {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has invalid event_kind {}",
            row.event_kind
        ));
    }
    if row.flags & !KNOWN_FLAG_MASK != 0 {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has unknown flags {}",
            row.flags
        ));
    }
    if row.flags & FLAG_FROM_VALIDATED_DOMAIN21_ROOT == 0 {
        errors.push(format!(
            "offset {offset}: detail row {row_index} lacks validated-root provenance"
        ));
    }
    if row.side != SIDE_NONE && row.side != SIDE_ATTACKER && row.side != SIDE_DEFENDER {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has invalid side {}",
            row.side
        ));
    }
    if row.flags & FLAG_SIDE_PRESENT == 0 && row.side != SIDE_NONE {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has side without flag"
        ));
    }
    if row.flags & FLAG_DIVISION_PRESENT == 0 && row.division_id != 0 {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has division without flag"
        ));
    }
    if row.flags & FLAG_LOCATION_PRESENT != 0
        && (row.location == 0 || row.location > MAX_LOCATION_ID)
    {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has invalid location"
        ));
    }
    if row.flags & FLAG_DAY_PRESENT != 0 && row.day > MAX_AGE_DAYS {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has invalid day"
        ));
    }
    if row.flags & FLAG_DURATION_PRESENT != 0 && row.duration > MAX_AGE_DAYS {
        errors.push(format!(
            "offset {offset}: detail row {row_index} has invalid duration"
        ));
    }
    if row.reserved != 0 {
        errors.push(format!(
            "offset {offset}: detail row {row_index} reserved field is nonzero"
        ));
    }
    if row.old_side_count > MAX_SIDE_UNITS as u32 || row.new_side_count > MAX_SIDE_UNITS as u32 {
        errors.push(format!(
            "offset {offset}: detail row {row_index} side counts exceed max"
        ));
    }
    if !seen.insert((
        row.combat_id,
        row.generation,
        row.event_kind,
        row.side,
        row.division_id,
    )) {
        errors.push(format!(
            "offset {offset}: detail row {row_index} duplicates event key"
        ));
    }
}

fn push_side_delta_rows(
    rows: &mut Vec<DetailEventRow>,
    old_units: &[u32],
    new_units: &[u32],
    now: &LandCombatSnapshot,
    side: CombatSide,
) {
    let old = old_units.iter().copied().collect::<BTreeSet<_>>();
    let new = new_units.iter().copied().collect::<BTreeSet<_>>();
    for division_id in new.difference(&old) {
        rows.push(participant_row(
            EVENT_PARTICIPANT_JOINED,
            now,
            side,
            *division_id,
            old.len() as u32,
            new.len() as u32,
        ));
    }
    for division_id in old.difference(&new) {
        rows.push(participant_row(
            EVENT_PARTICIPANT_LEFT_UNCLASSIFIED,
            now,
            side,
            *division_id,
            old.len() as u32,
            new.len() as u32,
        ));
    }
}

fn phase_changed(old: &LandCombatSnapshot, now: &LandCombatSnapshot) -> bool {
    old.location != now.location
        || old.day != now.day
        || old.duration != now.duration
        || old.attacker_divisions.len() != now.attacker_divisions.len()
        || old.defender_divisions.len() != now.defender_divisions.len()
}

fn phase_row(event_kind: u32, snapshot: &LandCombatSnapshot) -> DetailEventRow {
    let mut flags = FLAG_FROM_VALIDATED_DOMAIN21_ROOT;
    if snapshot.location.is_some() {
        flags |= FLAG_LOCATION_PRESENT;
    }
    if snapshot.day.is_some() {
        flags |= FLAG_DAY_PRESENT;
    }
    if snapshot.duration.is_some() {
        flags |= FLAG_DURATION_PRESENT;
    }
    DetailEventRow {
        combat_id: snapshot.combat_id,
        generation: snapshot.generation,
        event_kind,
        side: None,
        division_id: None,
        location: snapshot.location,
        day: snapshot.day,
        duration: snapshot.duration,
        old_side_count: 0,
        new_side_count: snapshot
            .attacker_divisions
            .len()
            .saturating_add(snapshot.defender_divisions.len())
            .min(u32::MAX as usize) as u32,
        flags,
    }
}

fn participant_row(
    event_kind: u32,
    snapshot: &LandCombatSnapshot,
    side: CombatSide,
    division_id: u32,
    old_side_count: u32,
    new_side_count: u32,
) -> DetailEventRow {
    let mut flags = FLAG_FROM_VALIDATED_DOMAIN21_ROOT | FLAG_SIDE_PRESENT | FLAG_DIVISION_PRESENT;
    if event_kind == EVENT_PARTICIPANT_LEFT_UNCLASSIFIED {
        flags |= FLAG_SEMANTIC_UNCLASSIFIED;
    }
    if snapshot.location.is_some() {
        flags |= FLAG_LOCATION_PRESENT;
    }
    if snapshot.day.is_some() {
        flags |= FLAG_DAY_PRESENT;
    }
    if snapshot.duration.is_some() {
        flags |= FLAG_DURATION_PRESENT;
    }
    DetailEventRow {
        combat_id: snapshot.combat_id,
        generation: snapshot.generation,
        event_kind,
        side: Some(side),
        division_id: Some(division_id),
        location: snapshot.location,
        day: snapshot.day,
        duration: snapshot.duration,
        old_side_count,
        new_side_count,
        flags,
    }
}

fn validate_snapshot(row: &LandCombatSnapshot) -> Result<(), String> {
    if row.combat_id == 0 || row.combat_id > MAX_COMBAT_ID {
        return Err(format!("invalid combat_id {}", row.combat_id));
    }
    for (label, units) in [
        ("attacker", row.attacker_divisions.as_slice()),
        ("defender", row.defender_divisions.as_slice()),
    ] {
        if units.len() > MAX_SIDE_UNITS {
            return Err(format!("combat {} {label} units exceed max", row.combat_id));
        }
        let mut seen = BTreeSet::new();
        for unit in units {
            if *unit == 0 || *unit > MAX_DIVISION_ID {
                return Err(format!(
                    "combat {} {label} has invalid division id {unit}",
                    row.combat_id
                ));
            }
            if !seen.insert(*unit) {
                return Err(format!(
                    "combat {} {label} duplicates division id {unit}",
                    row.combat_id
                ));
            }
        }
    }
    if row
        .location
        .is_some_and(|value| value == 0 || value > MAX_LOCATION_ID)
    {
        return Err(format!("combat {} invalid location", row.combat_id));
    }
    if row.day.is_some_and(|value| value > MAX_AGE_DAYS) {
        return Err(format!("combat {} invalid day", row.combat_id));
    }
    if row.duration.is_some_and(|value| value > MAX_AGE_DAYS) {
        return Err(format!("combat {} invalid duration", row.combat_id));
    }
    Ok(())
}

fn validate_row(row: &DetailEventRow) -> Result<(), String> {
    if row.combat_id == 0 || row.combat_id > MAX_COMBAT_ID {
        return Err("invalid combat id".to_string());
    }
    if let Some(division_id) = row.division_id {
        if division_id == 0 || division_id > MAX_DIVISION_ID {
            return Err("invalid division id".to_string());
        }
    }
    Ok(())
}

fn append_compact_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    flags: u32,
    payload: &[u8],
) {
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(payload);
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

fn read_i64(data: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

fn json_str(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization failed")
}

fn json_string_array(values: &[String]) -> String {
    let parts = values
        .iter()
        .map(|value| json_str(value))
        .collect::<Vec<_>>();
    format!("[{}]", parts.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestMemory {
        snapshots: Vec<LandCombatSnapshot>,
    }

    impl LandCombatDetailMemory for TestMemory {
        fn land_combat_snapshots(&self) -> Result<Vec<LandCombatSnapshot>, Vec<String>> {
            Ok(self.snapshots.clone())
        }
    }

    #[test]
    fn transition_emits_phase_and_participant_events() {
        let old = [snapshot(42, 7, 1000, &[1001, 1002], &[2001])];
        let new = [snapshot(42, 7, 1000, &[1001], &[2001, 2002])];
        let rows = build_detail_event_rows(&old, &new).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().any(|row| row.event_kind == EVENT_PHASE_UPDATED));
        assert!(rows
            .iter()
            .any(|row| row.event_kind == EVENT_PARTICIPANT_JOINED));
        assert!(rows
            .iter()
            .any(|row| row.event_kind == EVENT_PARTICIPANT_LEFT_UNCLASSIFIED));
    }

    #[test]
    fn compact_gate_accepts_transition_stream() {
        let start = [snapshot(42, 7, 1000, &[1001, 1002], &[2001])];
        let update = [snapshot(42, 7, 1000, &[1001], &[2001, 2002])];
        let frame1 = build_transition_capture_frame(&[], &start, 1, 60_787_539, 1).unwrap();
        let frame2 = build_transition_capture_frame(&start, &update, 2, 60_787_540, 2).unwrap();
        let frame3 = build_transition_capture_frame(&update, &[], 3, 60_787_541, 3).unwrap();
        let mut stream = frame1.compact_stream;
        stream.extend_from_slice(&frame2.compact_stream);
        stream.extend_from_slice(&frame3.compact_stream);
        let gate = gate_stream(&stream);
        assert!(gate.ok, "{:?}", gate.errors);
        assert_eq!(gate.summary.phase_started_rows, 1);
        assert_eq!(gate.summary.phase_updated_rows, 1);
        assert_eq!(gate.summary.phase_ended_rows, 1);
        assert_eq!(gate.summary.participant_joined_rows, 1);
        assert_eq!(gate.summary.participant_left_unclassified_rows, 1);
    }

    #[test]
    fn reset_transition_baseline_rebaselines_without_synthetic_events() {
        let mut tracker = LandCombatDetailTracker::default();
        let first = TestMemory {
            snapshots: vec![snapshot(42, 7, 1000, &[1001], &[2001])],
        };
        let changed_after_gap = TestMemory {
            snapshots: vec![snapshot(42, 7, 1000, &[1001, 1002], &[2001, 2002])],
        };

        let first_frame =
            collect_transition_capture_frame(&first, &mut tracker, 1, 60_787_539, 1).unwrap();
        assert!(!first_frame.rows.is_empty());

        tracker.reset_transition_baseline();
        let reset_frame =
            collect_transition_capture_frame(&changed_after_gap, &mut tracker, 2, 60_787_541, 2)
                .unwrap();
        assert!(reset_frame.rows.is_empty());
        assert!(tracker.last_frame_rebaselined());
        let payload = &reset_frame.compact_stream[COMPACT_RECORD_HEADER_LEN..];
        let decoded = decode_detail_events_payload(COMPACT_RECORD_HEADER_LEN, payload).unwrap();
        assert_eq!(decoded.rows.len(), 0);
        assert_ne!(decoded.status_flags & STATUS_TRANSITION_BASELINE_RESET, 0);
        assert_ne!(decoded.status_flags & STATUS_PARTIAL_DATA, 0);
    }

    fn snapshot(
        combat_id: u32,
        generation: u64,
        location: u32,
        attackers: &[u32],
        defenders: &[u32],
    ) -> LandCombatSnapshot {
        LandCombatSnapshot {
            combat_id,
            generation,
            location: Some(location),
            day: Some(3),
            duration: Some(3),
            attacker_divisions: attackers.to_vec(),
            defender_divisions: defenders.to_vec(),
            attacker_detail: None,
            defender_detail: None,
        }
    }

    fn sample_side_detail(committed: u32, reserve: u32) -> SideDetail {
        SideDetail {
            units: committed + reserve,
            committed,
            reserve,
            air_kills: 8,
            org_loss_summary_index: 0,
            num_org_losses: 192,
            anti_air_attack_raw: 434_710,
            air_damage_str_raw: 8_961_046,
            air_damage_org_raw: 18_464_611,
            ground_damage_str_raw: 11_248_500,
            ground_damage_org_raw: 15_443_824,
            prevented_damage_str_raw: 111_654,
            prevented_damage_org_raw: 245_433,
            counts_incomplete: false,
        }
    }

    fn snapshot_with_detail(
        combat_id: u32,
        generation: u64,
        attackers: &[u32],
        defenders: &[u32],
        attacker_detail: Option<SideDetail>,
        defender_detail: Option<SideDetail>,
    ) -> LandCombatSnapshot {
        LandCombatSnapshot {
            combat_id,
            generation,
            location: Some(3236),
            day: Some(192),
            duration: Some(192),
            attacker_divisions: attackers.to_vec(),
            defender_divisions: defenders.to_vec(),
            attacker_detail,
            defender_detail,
        }
    }

    #[test]
    fn side_detail_rows_roundtrip_through_v2_payload() {
        let att = sample_side_detail(2, 2);
        let def = sample_side_detail(2, 0);
        let current = vec![snapshot_with_detail(
            21381,
            7,
            &[8001, 8002, 8003, 8004],
            &[9001, 9002],
            Some(att),
            Some(def),
        )];
        let frame = build_transition_capture_frame(&[], &current, 1, 60_787_539, 1).unwrap();
        assert_eq!(frame.side_detail_rows.len(), 2);
        let payload = &frame.compact_stream[COMPACT_RECORD_HEADER_LEN..];
        let decoded = decode_detail_events_payload(COMPACT_RECORD_HEADER_LEN, payload).unwrap();
        assert_eq!(decoded.side_detail_rows.len(), 2);
        let attacker = decoded
            .side_detail_rows
            .iter()
            .find(|row| row.side == CombatSide::Attacker)
            .unwrap();
        assert_eq!(attacker.combat_id, 21381);
        assert_eq!(attacker.detail, att);
        assert_eq!(attacker.detail.units, 4);
        assert_eq!(attacker.detail.committed, 2);
        assert_eq!(attacker.detail.reserve, 2);
        let defender = decoded
            .side_detail_rows
            .iter()
            .find(|row| row.side == CombatSide::Defender)
            .unwrap();
        assert_eq!(defender.detail, def);

        // Gate still accepts the v2 stream and counts side-detail rows.
        let gate = gate_stream(&frame.compact_stream);
        assert!(gate.ok, "{:?}", gate.errors);
        assert_eq!(gate.summary.side_detail_rows, 2);
    }

    #[test]
    fn side_missing_detail_emits_no_row_fail_closed() {
        // A side whose detail failed revalidation (None) must contribute NO
        // row — absent, never zero-filled.
        let current = vec![snapshot_with_detail(
            21453,
            7,
            &[8001],
            &[9001],
            Some(sample_side_detail(1, 0)),
            None,
        )];
        let rows = side_detail_rows_from_snapshots(&current);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].side, CombatSide::Attacker);
    }

    #[test]
    fn v1_payload_still_decodes_with_no_side_detail() {
        // The legacy v1 encoder path (event rows only) must still decode.
        let start = [snapshot(42, 7, 1000, &[1001, 1002], &[2001])];
        let update = [snapshot(42, 7, 1000, &[1001], &[2001, 2002])];
        let rows = build_detail_event_rows(&start, &update).unwrap();
        let payload = encode_detail_events_payload(1, STATUS_DELTA_DATA, &rows);
        assert_eq!(read_u16(&payload, 0), SCHEMA_VERSION_V1_NO_SIDE_DETAIL);
        let decoded = decode_detail_events_payload(0, &payload).unwrap();
        assert!(decoded.side_detail_rows.is_empty());
        assert_eq!(decoded.rows.len(), rows.len());
    }

    #[test]
    fn side_detail_committed_plus_reserve_must_equal_units_without_badge() {
        // An invariant violation WITHOUT the incomplete badge is a corrupt/torn
        // payload and must be rejected.
        let mut bad = sample_side_detail(2, 2);
        bad.units = 5; // committed+reserve = 4 != 5
        bad.counts_incomplete = false;
        let current = vec![snapshot_with_detail(
            21381,
            7,
            &[8001],
            &[9001],
            Some(bad),
            None,
        )];
        let frame = build_transition_capture_frame(&[], &current, 1, 60_787_539, 1).unwrap();
        let payload = &frame.compact_stream[COMPACT_RECORD_HEADER_LEN..];
        let err = decode_detail_events_payload(0, payload).unwrap_err();
        assert!(err
            .iter()
            .any(|e| e.contains("committed+reserve != units without incomplete badge")));
    }

    #[test]
    fn side_detail_invariant_violation_accepted_when_incomplete_badged() {
        // The SAME violation WITH the incomplete badge (a genuine mid-combat
        // transient the collector observed with a stable double-read) is
        // RETAINED and round-trips — the row is not dropped.
        let mut transient = sample_side_detail(2, 0);
        transient.units = 3; // committed 2 + reserve 0 = 2 != 3
        transient.counts_incomplete = true;
        let current = vec![snapshot_with_detail(
            21381,
            7,
            &[8001],
            &[9001],
            Some(transient),
            None,
        )];
        let frame = build_transition_capture_frame(&[], &current, 1, 60_787_539, 1).unwrap();
        assert_eq!(frame.side_detail_rows.len(), 1);
        let payload = &frame.compact_stream[COMPACT_RECORD_HEADER_LEN..];
        let decoded = decode_detail_events_payload(0, payload).expect("incomplete row decodes");
        assert_eq!(decoded.side_detail_rows.len(), 1);
        assert!(decoded.side_detail_rows[0].detail.counts_incomplete);
        assert_eq!(decoded.side_detail_rows[0].detail.units, 3);
        // The flag round-trips in the wire flags.
        let gate = gate_stream(&frame.compact_stream);
        assert!(gate.ok, "{:?}", gate.errors);
    }

    #[test]
    fn side_detail_damage_out_of_range_is_rejected() {
        let mut bad = sample_side_detail(1, 0);
        bad.ground_damage_str_raw = MAX_SIDE_DETAIL_DAMAGE_RAW + 1;
        let current = vec![snapshot_with_detail(
            21381,
            7,
            &[8001],
            &[9001],
            Some(bad),
            None,
        )];
        let frame = build_transition_capture_frame(&[], &current, 1, 60_787_539, 1).unwrap();
        let payload = &frame.compact_stream[COMPACT_RECORD_HEADER_LEN..];
        assert!(decode_detail_events_payload(0, payload).is_err());
    }
}
