use std::collections::{BTreeMap, BTreeSet};

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const DOMAIN_LAND_COMBAT_DETAIL_EVENTS: u16 = 55;
pub const DOMAIN_LAND_COMBAT_DETAIL_EVENTS_NAME: &str = "land_combat_detail_events_live";

pub const SCHEMA_VERSION: u16 = 1;
pub const PAYLOAD_HEADER_LEN: usize = 28;
pub const DETAIL_EVENT_ROW_LEN: usize = 52;

pub const STATUS_DELTA_DATA: u32 = 1 << 10;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const FAILURE_STATUS_MASK: u32 = STATUS_SOURCE_UNAVAILABLE | STATUS_PARTIAL_DATA;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LandCombatSnapshot {
    pub combat_id: u32,
    pub generation: u64,
    pub location: Option<u32>,
    pub day: Option<u32>,
    pub duration: Option<u32>,
    pub attacker_divisions: Vec<u32>,
    pub defender_divisions: Vec<u32>,
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
}

pub fn build_transition_capture_frame(
    previous: &[LandCombatSnapshot],
    current: &[LandCombatSnapshot],
    sequence: u64,
    game_hour_raw: i32,
    elapsed_wall_ms: u64,
) -> Result<CaptureFrame, Vec<String>> {
    let rows = build_detail_event_rows(previous, current)?;
    let payload = encode_detail_events_payload(elapsed_wall_ms, STATUS_DELTA_DATA, &rows);
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

pub fn encode_detail_events_payload(
    elapsed_wall_ms: u64,
    status_flags: u32,
    rows: &[DetailEventRow],
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(PAYLOAD_HEADER_LEN + rows.len() * DETAIL_EVENT_ROW_LEN);
    payload.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    payload.extend_from_slice(&(DETAIL_EVENT_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
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
    payload
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
            decode_detail_events_payload(
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

fn decode_detail_events_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut GateSummary,
    errors: &mut Vec<String>,
) {
    if payload.len() < PAYLOAD_HEADER_LEN {
        errors.push(format!("offset {offset}: detail payload too short"));
        return;
    }
    let version = read_u16(payload, 0);
    let row_len = read_u16(payload, 2) as usize;
    let status_flags = read_u32(payload, 12);
    let row_count = read_u32(payload, 16) as usize;
    let string_count = read_u32(payload, 20);
    let reserved = read_u32(payload, 24);
    summary.status_flags_or |= status_flags;
    if version != SCHEMA_VERSION {
        errors.push(format!(
            "offset {offset}: detail schema version {version} != {SCHEMA_VERSION}"
        ));
    }
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
    let expected_len = PAYLOAD_HEADER_LEN.saturating_add(row_count.saturating_mul(row_len));
    if payload.len() != expected_len {
        errors.push(format!(
            "offset {offset}: detail payload len {} != expected {expected_len}",
            payload.len()
        ));
        return;
    }
    summary.row_count = summary.row_count.saturating_add(row_count as u64);
    let mut seen = BTreeSet::new();
    let mut cursor = PAYLOAD_HEADER_LEN;
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
        validate_decoded_row(offset, row_index, &row, &mut seen, errors);
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
        }
    }
}
