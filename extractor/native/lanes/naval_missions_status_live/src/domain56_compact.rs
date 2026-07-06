use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::process;

pub const DOMAIN_ID: u16 = 56;
pub const DOMAIN_NAME: &str = "naval_missions_status_live";

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const COMPACT_RECORD_TYPE_FRAME_END: u16 = 2;
pub const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const SCHEMA_VERSION: u32 = 1;
pub const HEADER_LEN: usize = 44;
pub const ROW_LEN: usize = 64;
pub const NAVAL_SUPREMACY_TRAILER_MAGIC: &[u8; 4] = b"NSUP";
pub const NAVAL_SUPREMACY_TRAILER_VERSION: u16 = 1;
pub const NAVAL_SUPREMACY_TRAILER_HEADER_LEN: usize = 12;
pub const NAVAL_SUPREMACY_TRAILER_ROW_LEN: usize = 32;
pub const MAX_OOB_ID: u32 = 4_000_000;
pub const MAX_OWNER_TAG_DB_ID: u32 = compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
pub const MAX_PROVINCE_ID: u32 = 1_000_000;
pub const MAX_REGION_ID: u32 = 100_000;
pub const MAX_MISSION_ENUM: u32 = 64;
pub const MAX_MISSION_STATE: u32 = 256;
pub const MAX_RETREAT_STATE: u32 = 256;
pub const MAX_REPAIR_PROGRESS_MILLI: u32 = 1_000;
pub const MAX_SUNK_CONVOYS: u32 = 1_000_000;
pub const MAX_NAVAL_SUPREMACY_ROWS: usize = 20_000;
pub const NAVAL_SUPREMACY_FIXED_SCALE_RAW: u32 = 100_000;

pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const STATUS_DELTA_DATA: u32 = 1 << 10;
pub const STATUS_MISSION_FIXTURE_REQUIRED: u32 = 1 << 16;
pub const STATUS_HOME_BASE_FIXTURE_REQUIRED: u32 = 1 << 17;
pub const STATUS_REPAIR_RETREAT_FIXTURE_REQUIRED: u32 = 1 << 18;
pub const STATUS_CONVOY_CONTEXT_FIXTURE_REQUIRED: u32 = 1 << 19;
pub const FAILURE_STATUS_MASK: u32 = STATUS_SOURCE_UNAVAILABLE | STATUS_PARTIAL_DATA;
pub const KNOWN_SOURCE_STATUS_MASK: u32 = STATUS_SOURCE_UNAVAILABLE
    | STATUS_PARTIAL_DATA
    | STATUS_DELTA_DATA
    | STATUS_MISSION_FIXTURE_REQUIRED
    | STATUS_HOME_BASE_FIXTURE_REQUIRED
    | STATUS_REPAIR_RETREAT_FIXTURE_REQUIRED
    | STATUS_CONVOY_CONTEXT_FIXTURE_REQUIRED;

pub const FLAG_HAS_MISSION: u32 = 1 << 0;
pub const FLAG_HAS_MISSION_STATE: u32 = 1 << 1;
pub const FLAG_HAS_LOCATION: u32 = 1 << 2;
pub const FLAG_HAS_REGION: u32 = 1 << 3;
pub const FLAG_HAS_HOME_BASE: u32 = 1 << 4;
pub const FLAG_HAS_REPAIR_BASE: u32 = 1 << 5;
pub const FLAG_HAS_REPAIR_PROGRESS: u32 = 1 << 6;
pub const FLAG_HAS_RETREAT_STATE: u32 = 1 << 7;
pub const FLAG_HAS_CONVOY_ROUTE_CONTEXT: u32 = 1 << 8;
pub const FLAG_HAS_SUNK_CONVOY_COUNT: u32 = 1 << 9;
pub const FLAG_VALIDATED_DIRECT: u32 = 1 << 10;
pub const FLAG_DERIVED_REGION: u32 = 1 << 11;
pub const FLAG_FIXTURE_REQUIRED: u32 = 1 << 12;
pub const FLAG_ACTIVE_BATTLE_FIXTURE_REQUIRED: u32 = 1 << 13;
pub const FLAG_FIELD_REJECTED: u32 = 1 << 14;

pub const KNOWN_ROW_FLAGS: u32 = FLAG_HAS_MISSION
    | FLAG_HAS_MISSION_STATE
    | FLAG_HAS_LOCATION
    | FLAG_HAS_REGION
    | FLAG_HAS_HOME_BASE
    | FLAG_HAS_REPAIR_BASE
    | FLAG_HAS_REPAIR_PROGRESS
    | FLAG_HAS_RETREAT_STATE
    | FLAG_HAS_CONVOY_ROUTE_CONTEXT
    | FLAG_HAS_SUNK_CONVOY_COUNT
    | FLAG_VALIDATED_DIRECT
    | FLAG_DERIVED_REGION
    | FLAG_FIXTURE_REQUIRED
    | FLAG_ACTIVE_BATTLE_FIXTURE_REQUIRED
    | FLAG_FIELD_REJECTED;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavalMissionStatusHeader {
    pub elapsed_wall_ms: u64,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_task_forces: u32,
    pub validated_location_rows: u32,
    pub derived_region_rows: u32,
    pub fixture_required_rows: u32,
    pub convoy_sunk_count_rows: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavalMissionStatusRow {
    pub task_force_id: u32,
    pub fleet_id: u32,
    pub owner_tag_db_id: u32,
    pub flags: u32,
    pub mission_enum: u32,
    pub mission_state: u32,
    pub location_province_id: u32,
    pub strategic_region_id: u32,
    pub home_base_province_id: u32,
    pub repair_base_province_id: u32,
    pub repair_progress_milli: u32,
    pub retreat_state: u32,
    pub sunk_convoys: u32,
    pub convoy_route_context_id: u32,
    pub source_status: u32,
    pub reserved0: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavalSupremacyRow {
    pub region_id: u32,
    pub controller_tag_db_id: u32,
    pub current_raw: u32,
    pub target_raw: u32,
    pub base_target_raw: u32,
    pub previous_raw: u32,
    pub decline_from_raw: u32,
    pub individual_ratio_raw: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain56GateSummary {
    pub record_count: u64,
    pub data_record_count: u64,
    pub frame_end_record_count: u64,
    pub domains_seen: BTreeSet<u16>,
    pub status_flags_or: u32,
    pub row_count_total: u64,
    pub changed_rows_total: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub parse_errors: Vec<String>,
    pub ordering_errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedNavalMissionStatusPayload {
    pub header: NavalMissionStatusHeader,
    pub rows: Vec<NavalMissionStatusRow>,
    pub naval_supremacy_rows: Vec<NavalSupremacyRow>,
}

impl Domain56GateSummary {
    pub fn gate_passed(&self) -> bool {
        self.parse_errors.is_empty()
            && self.ordering_errors.is_empty()
            && self.data_record_count > 0
            && self.status_flags_or & FAILURE_STATUS_MASK == 0
    }
}

pub fn decode_status_payload(
    offset: usize,
    payload: &[u8],
) -> Result<DecodedNavalMissionStatusPayload, Vec<String>> {
    let mut errors = Vec::new();
    if payload.len() < HEADER_LEN {
        errors.push(format!(
            "offset {offset}: domain 56 payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        ));
        return Err(errors);
    }

    let row_count = read_u32(payload, 8) as usize;
    let header = NavalMissionStatusHeader {
        elapsed_wall_ms: read_u64(payload, 0),
        changed_rows: read_u32(payload, 12),
        status_flags: read_u32(payload, 16),
        observed_task_forces: read_u32(payload, 20),
        validated_location_rows: read_u32(payload, 24),
        derived_region_rows: read_u32(payload, 28),
        fixture_required_rows: read_u32(payload, 32),
        convoy_sunk_count_rows: read_u32(payload, 36),
    };
    let schema_version = read_u32(payload, 40);
    let base_len = HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));

    if schema_version != SCHEMA_VERSION {
        errors.push(format!(
            "offset {offset}: domain 56 schema version {schema_version} != {SCHEMA_VERSION}"
        ));
    }
    if payload.len() < base_len {
        errors.push(format!(
            "offset {offset}: domain 56 payload is {} byte(s), expected at least {base_len}",
            payload.len()
        ));
        return Err(errors);
    }
    if header.changed_rows > row_count as u32 {
        errors.push(format!(
            "offset {offset}: changed_rows {} exceeds row_count {row_count}",
            header.changed_rows
        ));
    }
    if header.validated_location_rows > row_count as u32
        || header.derived_region_rows > row_count as u32
        || header.fixture_required_rows > row_count as u32
    {
        errors.push(format!(
            "offset {offset}: header row counters exceed row_count {row_count}"
        ));
    }

    let mut seen = BTreeSet::new();
    let mut rows = Vec::with_capacity(row_count);
    for index in 0..row_count {
        let cursor = HEADER_LEN + index * ROW_LEN;
        let row = NavalMissionStatusRow {
            task_force_id: read_u32(payload, cursor),
            fleet_id: read_u32(payload, cursor + 4),
            owner_tag_db_id: read_u32(payload, cursor + 8),
            flags: read_u32(payload, cursor + 12),
            mission_enum: read_u32(payload, cursor + 16),
            mission_state: read_u32(payload, cursor + 20),
            location_province_id: read_u32(payload, cursor + 24),
            strategic_region_id: read_u32(payload, cursor + 28),
            home_base_province_id: read_u32(payload, cursor + 32),
            repair_base_province_id: read_u32(payload, cursor + 36),
            repair_progress_milli: read_u32(payload, cursor + 40),
            retreat_state: read_u32(payload, cursor + 44),
            sunk_convoys: read_u32(payload, cursor + 48),
            convoy_route_context_id: read_u32(payload, cursor + 52),
            source_status: read_u32(payload, cursor + 56),
            reserved0: read_u32(payload, cursor + 60),
        };
        if !seen.insert(row.task_force_id) {
            errors.push(format!(
                "offset {offset}: duplicate task_force_id {}",
                row.task_force_id
            ));
        }
        if let Err(err) = validate_row(&row, index) {
            errors.push(format!("offset {offset}: {err}"));
        }
        rows.push(row);
    }

    let naval_supremacy_rows = decode_domain56_trailers(offset, payload, base_len, &mut errors);

    if errors.is_empty() {
        Ok(DecodedNavalMissionStatusPayload {
            header,
            rows,
            naval_supremacy_rows,
        })
    } else {
        Err(errors)
    }
}

pub fn encode_payload(
    header: &NavalMissionStatusHeader,
    rows: &[NavalMissionStatusRow],
) -> Vec<u8> {
    encode_payload_with_naval_supremacy(header, rows, &[])
}

pub fn encode_payload_with_naval_supremacy(
    header: &NavalMissionStatusHeader,
    rows: &[NavalMissionStatusRow],
    naval_supremacy_rows: &[NavalSupremacyRow],
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(HEADER_LEN + rows.len() * ROW_LEN);
    push_u64(&mut payload, header.elapsed_wall_ms);
    push_u32(&mut payload, rows.len() as u32);
    push_u32(&mut payload, header.changed_rows);
    push_u32(&mut payload, header.status_flags);
    push_u32(&mut payload, header.observed_task_forces);
    push_u32(&mut payload, header.validated_location_rows);
    push_u32(&mut payload, header.derived_region_rows);
    push_u32(&mut payload, header.fixture_required_rows);
    push_u32(&mut payload, header.convoy_sunk_count_rows);
    push_u32(&mut payload, SCHEMA_VERSION);

    for row in rows {
        validate_row(row, 0).expect("invalid naval mission/status row");
        push_u32(&mut payload, row.task_force_id);
        push_u32(&mut payload, row.fleet_id);
        push_u32(&mut payload, row.owner_tag_db_id);
        push_u32(&mut payload, row.flags);
        push_u32(&mut payload, row.mission_enum);
        push_u32(&mut payload, row.mission_state);
        push_u32(&mut payload, row.location_province_id);
        push_u32(&mut payload, row.strategic_region_id);
        push_u32(&mut payload, row.home_base_province_id);
        push_u32(&mut payload, row.repair_base_province_id);
        push_u32(&mut payload, row.repair_progress_milli);
        push_u32(&mut payload, row.retreat_state);
        push_u32(&mut payload, row.sunk_convoys);
        push_u32(&mut payload, row.convoy_route_context_id);
        push_u32(&mut payload, row.source_status);
        push_u32(&mut payload, row.reserved0);
    }
    append_naval_supremacy_trailer(&mut payload, naval_supremacy_rows);
    payload
}

fn append_naval_supremacy_trailer(payload: &mut Vec<u8>, rows: &[NavalSupremacyRow]) {
    if rows.is_empty() {
        return;
    }
    payload.extend_from_slice(NAVAL_SUPREMACY_TRAILER_MAGIC);
    payload.extend_from_slice(&NAVAL_SUPREMACY_TRAILER_VERSION.to_le_bytes());
    payload.extend_from_slice(&(NAVAL_SUPREMACY_TRAILER_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    for row in rows {
        validate_naval_supremacy_row(row, 0).expect("invalid naval supremacy row");
        push_u32(&mut *payload, row.region_id);
        push_u32(&mut *payload, row.controller_tag_db_id);
        push_u32(&mut *payload, row.current_raw);
        push_u32(&mut *payload, row.target_raw);
        push_u32(&mut *payload, row.base_target_raw);
        push_u32(&mut *payload, row.previous_raw);
        push_u32(&mut *payload, row.decline_from_raw);
        push_u32(&mut *payload, row.individual_ratio_raw);
    }
}

fn decode_domain56_trailers(
    offset: usize,
    payload: &[u8],
    mut cursor: usize,
    errors: &mut Vec<String>,
) -> Vec<NavalSupremacyRow> {
    let mut naval_supremacy_rows = Vec::new();
    let mut seen_nsup = false;
    while cursor < payload.len() {
        if payload.get(cursor..cursor + 4) == Some(NAVAL_SUPREMACY_TRAILER_MAGIC) {
            if seen_nsup {
                errors.push(format!(
                    "offset {offset}: duplicate naval-supremacy trailer"
                ));
                break;
            }
            let (rows, next) = decode_naval_supremacy_trailer(offset, payload, cursor, errors);
            naval_supremacy_rows = rows;
            cursor = next;
            seen_nsup = true;
        } else {
            errors.push(format!(
                "offset {offset}: domain 56 unknown trailer magic at byte {cursor}"
            ));
            break;
        }
    }
    naval_supremacy_rows
}

fn decode_naval_supremacy_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    errors: &mut Vec<String>,
) -> (Vec<NavalSupremacyRow>, usize) {
    let Some(trailer) = payload.get(trailer_offset..) else {
        errors.push(format!(
            "offset {offset}: naval-supremacy trailer offset {trailer_offset} beyond payload"
        ));
        return (Vec::new(), payload.len());
    };
    if trailer.len() < NAVAL_SUPREMACY_TRAILER_HEADER_LEN {
        errors.push(format!(
            "offset {offset}: naval-supremacy trailer is {} byte(s), expected at least {}",
            trailer.len(),
            NAVAL_SUPREMACY_TRAILER_HEADER_LEN
        ));
        return (Vec::new(), payload.len());
    }
    let version = read_u16(trailer, 4);
    let header_len = read_u16(trailer, 6) as usize;
    let row_count = read_u32(trailer, 8) as usize;
    if version != NAVAL_SUPREMACY_TRAILER_VERSION
        || header_len != NAVAL_SUPREMACY_TRAILER_HEADER_LEN
    {
        errors.push(format!(
            "offset {offset}: naval-supremacy trailer header shape invalid (version {version}, header_len {header_len})"
        ));
    }
    if row_count > MAX_NAVAL_SUPREMACY_ROWS {
        errors.push(format!(
            "offset {offset}: naval-supremacy row_count {row_count} exceeds {MAX_NAVAL_SUPREMACY_ROWS}"
        ));
        return (Vec::new(), payload.len());
    }
    let expected = NAVAL_SUPREMACY_TRAILER_HEADER_LEN
        .saturating_add(row_count.saturating_mul(NAVAL_SUPREMACY_TRAILER_ROW_LEN));
    if trailer.len() < expected {
        errors.push(format!(
            "offset {offset}: naval-supremacy trailer len {} < {expected}",
            trailer.len()
        ));
        return (Vec::new(), payload.len());
    }

    // NSUP is a faithful occurrence list of the game's per-strategic-region
    // naval-dominance array (CStrategicRegion naval_dominance). That array
    // legitimately holds MULTIPLE entries for the same (region, controller):
    // validated live at D04 1944, region 29 carries dominance tags [2,22,39,22]
    // (tag 22 twice, distinct value payloads), and 5 (region, controller) pairs
    // in the acceptance fixture repeat with differing supremacy values. There is
    // no per-entry discriminator field in the 32-byte row beyond the value
    // payload, so (region, controller) is NOT a uniqueness key. Do not reject on
    // it — that would drop real data. Preserve array order (the collector emits
    // in a stable order) and validate each row's field bounds. A byte-identical
    // full-row repeat can legitimately occur for zero/steady-state entries, so it
    // is not treated as corruption either.
    let mut rows = Vec::with_capacity(row_count);
    let mut cursor = NAVAL_SUPREMACY_TRAILER_HEADER_LEN;
    for index in 0..row_count {
        let row = NavalSupremacyRow {
            region_id: read_u32(trailer, cursor),
            controller_tag_db_id: read_u32(trailer, cursor + 4),
            current_raw: read_u32(trailer, cursor + 8),
            target_raw: read_u32(trailer, cursor + 12),
            base_target_raw: read_u32(trailer, cursor + 16),
            previous_raw: read_u32(trailer, cursor + 20),
            decline_from_raw: read_u32(trailer, cursor + 24),
            individual_ratio_raw: read_u32(trailer, cursor + 28),
        };
        cursor += NAVAL_SUPREMACY_TRAILER_ROW_LEN;
        if let Err(err) = validate_naval_supremacy_row(&row, index) {
            errors.push(format!("offset {offset}: {err}"));
        }
        rows.push(row);
    }
    (rows, trailer_offset + expected)
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> Domain56GateSummary {
    let mut summary = Domain56GateSummary::default();
    if payload.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: domain 56 payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        ));
        return summary;
    }

    let row_count = read_u32(payload, 8) as usize;
    let changed_rows = read_u32(payload, 12) as usize;
    let status_flags = read_u32(payload, 16);
    let observed_task_forces = read_u32(payload, 20) as usize;
    let validated_location_rows = read_u32(payload, 24) as usize;
    let derived_region_rows = read_u32(payload, 28) as usize;
    let fixture_required_rows = read_u32(payload, 32) as usize;
    let convoy_sunk_count_rows = read_u32(payload, 36) as usize;
    let schema_version = read_u32(payload, 40);
    let base_len = HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));

    summary.status_flags_or |= status_flags;
    summary.row_count_total = row_count as u64;
    summary.changed_rows_total = changed_rows as u64;
    add_unit(&mut summary, "naval_mission_status_rows", row_count);
    add_unit(
        &mut summary,
        "naval_mission_observed_task_forces",
        observed_task_forces,
    );
    add_unit(
        &mut summary,
        "naval_mission_validated_location_rows",
        validated_location_rows,
    );
    add_unit(
        &mut summary,
        "naval_mission_derived_region_rows",
        derived_region_rows,
    );
    add_unit(
        &mut summary,
        "naval_mission_fixture_required_rows",
        fixture_required_rows,
    );
    add_unit(
        &mut summary,
        "naval_mission_sunk_convoy_count_rows",
        convoy_sunk_count_rows,
    );

    if schema_version != SCHEMA_VERSION {
        summary.parse_errors.push(format!(
            "offset {offset}: domain 56 schema version {schema_version} != {SCHEMA_VERSION}"
        ));
    }
    if payload.len() < base_len {
        summary.parse_errors.push(format!(
            "offset {offset}: domain 56 payload is {} byte(s), expected at least {base_len}",
            payload.len()
        ));
        return summary;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: changed_rows {changed_rows} exceeds row_count {row_count}"
        ));
    }
    if validated_location_rows > row_count
        || derived_region_rows > row_count
        || fixture_required_rows > row_count
    {
        summary.parse_errors.push(format!(
            "offset {offset}: header row counters exceed row_count {row_count}"
        ));
    }

    let mut seen = BTreeSet::new();
    let mut location_rows = 0usize;
    let mut region_rows = 0usize;
    let mut mission_rows = 0usize;
    let mut home_base_rows = 0usize;
    let mut repair_rows = 0usize;
    let mut retreat_rows = 0usize;
    let mut convoy_route_rows = 0usize;
    let mut sunk_convoy_rows = 0usize;
    let mut fixture_rows = 0usize;

    for index in 0..row_count {
        let cursor = HEADER_LEN + index * ROW_LEN;
        let row = NavalMissionStatusRow {
            task_force_id: read_u32(payload, cursor),
            fleet_id: read_u32(payload, cursor + 4),
            owner_tag_db_id: read_u32(payload, cursor + 8),
            flags: read_u32(payload, cursor + 12),
            mission_enum: read_u32(payload, cursor + 16),
            mission_state: read_u32(payload, cursor + 20),
            location_province_id: read_u32(payload, cursor + 24),
            strategic_region_id: read_u32(payload, cursor + 28),
            home_base_province_id: read_u32(payload, cursor + 32),
            repair_base_province_id: read_u32(payload, cursor + 36),
            repair_progress_milli: read_u32(payload, cursor + 40),
            retreat_state: read_u32(payload, cursor + 44),
            sunk_convoys: read_u32(payload, cursor + 48),
            convoy_route_context_id: read_u32(payload, cursor + 52),
            source_status: read_u32(payload, cursor + 56),
            reserved0: read_u32(payload, cursor + 60),
        };
        if !seen.insert(row.task_force_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: duplicate task_force_id {}",
                row.task_force_id
            ));
        }
        if let Err(err) = validate_row(&row, index) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
        }
        if row.flags & FLAG_HAS_LOCATION != 0 {
            location_rows += 1;
        }
        if row.flags & FLAG_HAS_REGION != 0 {
            region_rows += 1;
        }
        if row.flags & FLAG_HAS_MISSION != 0 {
            mission_rows += 1;
        }
        if row.flags & FLAG_HAS_HOME_BASE != 0 {
            home_base_rows += 1;
        }
        if row.flags & (FLAG_HAS_REPAIR_BASE | FLAG_HAS_REPAIR_PROGRESS) != 0 {
            repair_rows += 1;
        }
        if row.flags & FLAG_HAS_RETREAT_STATE != 0 {
            retreat_rows += 1;
        }
        if row.flags & FLAG_HAS_CONVOY_ROUTE_CONTEXT != 0 {
            convoy_route_rows += 1;
        }
        if row.flags & FLAG_HAS_SUNK_CONVOY_COUNT != 0 {
            sunk_convoy_rows += 1;
        }
        if row.flags & (FLAG_FIXTURE_REQUIRED | FLAG_ACTIVE_BATTLE_FIXTURE_REQUIRED) != 0 {
            fixture_rows += 1;
        }
    }

    add_unit(&mut summary, "naval_mission_location_rows", location_rows);
    add_unit(&mut summary, "naval_mission_region_rows", region_rows);
    add_unit(
        &mut summary,
        "naval_mission_rows_with_mission",
        mission_rows,
    );
    add_unit(&mut summary, "naval_mission_home_base_rows", home_base_rows);
    add_unit(&mut summary, "naval_mission_repair_rows", repair_rows);
    add_unit(&mut summary, "naval_mission_retreat_rows", retreat_rows);
    add_unit(
        &mut summary,
        "naval_mission_convoy_route_rows",
        convoy_route_rows,
    );
    add_unit(
        &mut summary,
        "naval_mission_sunk_convoy_rows",
        sunk_convoy_rows,
    );
    add_unit(&mut summary, "naval_mission_fixture_rows", fixture_rows);
    let mut trailer_errors = Vec::new();
    let naval_supremacy_rows =
        decode_domain56_trailers(offset, payload, base_len, &mut trailer_errors);
    add_unit(
        &mut summary,
        "naval_supremacy_rows",
        naval_supremacy_rows.len(),
    );
    summary.parse_errors.extend(trailer_errors);
    summary
}

pub fn compact_record(
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(COMPACT_RECORD_HEADER_LEN + payload.len());
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn gate_stream(raw: &[u8]) -> Domain56GateSummary {
    let mut summary = Domain56GateSummary::default();
    let mut offset = 0usize;
    let mut previous_sequence = None;
    while offset < raw.len() {
        if raw.len() - offset < COMPACT_RECORD_HEADER_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: trailing {} byte(s), expected compact record header",
                raw.len() - offset
            ));
            break;
        }
        let header = &raw[offset..offset + COMPACT_RECORD_HEADER_LEN];
        if &header[0..4] != COMPACT_RECORD_MAGIC {
            summary
                .parse_errors
                .push(format!("offset {offset}: invalid record magic"));
            break;
        }
        let version = read_u16(header, 4);
        let header_len = read_u16(header, 6) as usize;
        let sequence = read_u64(header, 8);
        let domain_id = read_u16(header, 20);
        let record_type = read_u16(header, 22);
        let payload_len = read_u32(header, 24) as usize;
        if version != COMPACT_RECORD_VERSION || header_len != COMPACT_RECORD_HEADER_LEN {
            summary
                .parse_errors
                .push(format!("offset {offset}: unsupported compact header"));
            break;
        }
        if let Some(previous) = previous_sequence {
            if sequence <= previous {
                summary.ordering_errors.push(format!(
                    "offset {offset}: sequence {sequence} did not advance past {previous}"
                ));
            }
        }
        previous_sequence = Some(sequence);
        let payload_start = offset + COMPACT_RECORD_HEADER_LEN;
        let payload_end = payload_start.saturating_add(payload_len);
        if payload_end > raw.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
            ));
            break;
        }
        let payload = &raw[payload_start..payload_end];
        summary.record_count += 1;
        summary.domains_seen.insert(domain_id);
        match record_type {
            COMPACT_RECORD_TYPE_DOMAIN_DATA if domain_id == DOMAIN_ID => {
                let decoded = decode_payload(offset, payload);
                summary.data_record_count += 1;
                summary.status_flags_or |= decoded.status_flags_or;
                summary.row_count_total += decoded.row_count_total;
                summary.changed_rows_total += decoded.changed_rows_total;
                for (key, value) in decoded.data_units_total {
                    *summary.data_units_total.entry(key).or_default() += value;
                }
                summary.parse_errors.extend(decoded.parse_errors);
            }
            COMPACT_RECORD_TYPE_DOMAIN_DATA => {}
            COMPACT_RECORD_TYPE_DOMAIN_PERF => {}
            COMPACT_RECORD_TYPE_FRAME_END => summary.frame_end_record_count += 1,
            _ => summary.parse_errors.push(format!(
                "offset {offset}: unsupported record type {record_type}"
            )),
        }
        offset = payload_end;
    }
    summary
}

fn validate_row(row: &NavalMissionStatusRow, index: usize) -> Result<(), String> {
    validate_oob_id(row.task_force_id, "task_force_id", index)?;
    validate_oob_id(row.fleet_id, "fleet_id", index)?;
    if row.owner_tag_db_id > MAX_OWNER_TAG_DB_ID {
        return Err(format!(
            "row {index} owner_tag_db_id {} exceeds {MAX_OWNER_TAG_DB_ID}",
            row.owner_tag_db_id
        ));
    }
    if row.flags & !KNOWN_ROW_FLAGS != 0 {
        return Err(format!(
            "row {index} has unknown flags 0x{:X}",
            row.flags & !KNOWN_ROW_FLAGS
        ));
    }
    // Mission enum 0 is a VALID value (the "hold/do-nothing" mission; the melt
    // oracle carries all 7 distinct values 0,1,2,3,4,8,9). Presence is carried
    // solely by FLAG_HAS_MISSION, so 0-when-present must NOT be rejected.
    validate_optional_enum(
        row.flags & FLAG_HAS_MISSION != 0,
        row.mission_enum,
        MAX_MISSION_ENUM,
        "mission_enum",
        index,
    )?;
    // mission_state is not emitted or proven in this packet; keep the strict
    // validator (present-zero rejected) until a zero value is oracle-proven.
    validate_optional_max(
        row.flags & FLAG_HAS_MISSION_STATE != 0,
        row.mission_state,
        MAX_MISSION_STATE,
        "mission_state",
        index,
    )?;
    validate_optional_max(
        row.flags & FLAG_HAS_LOCATION != 0,
        row.location_province_id,
        MAX_PROVINCE_ID,
        "location_province_id",
        index,
    )?;
    validate_optional_max(
        row.flags & FLAG_HAS_REGION != 0,
        row.strategic_region_id,
        MAX_REGION_ID,
        "strategic_region_id",
        index,
    )?;
    validate_optional_max(
        row.flags & FLAG_HAS_HOME_BASE != 0,
        row.home_base_province_id,
        MAX_PROVINCE_ID,
        "home_base_province_id",
        index,
    )?;
    validate_optional_max(
        row.flags & FLAG_HAS_REPAIR_BASE != 0,
        row.repair_base_province_id,
        MAX_PROVINCE_ID,
        "repair_base_province_id",
        index,
    )?;
    validate_optional_max(
        row.flags & FLAG_HAS_REPAIR_PROGRESS != 0,
        row.repair_progress_milli,
        MAX_REPAIR_PROGRESS_MILLI,
        "repair_progress_milli",
        index,
    )?;
    validate_optional_max(
        row.flags & FLAG_HAS_RETREAT_STATE != 0,
        row.retreat_state,
        MAX_RETREAT_STATE,
        "retreat_state",
        index,
    )?;
    validate_optional_max(
        row.flags & FLAG_HAS_SUNK_CONVOY_COUNT != 0,
        row.sunk_convoys,
        MAX_SUNK_CONVOYS,
        "sunk_convoys",
        index,
    )?;
    validate_optional_max(
        row.flags & FLAG_HAS_CONVOY_ROUTE_CONTEXT != 0,
        row.convoy_route_context_id,
        u32::MAX,
        "convoy_route_context_id",
        index,
    )?;
    if row.reserved0 != 0 {
        return Err(format!("row {index} reserved0 must be zero"));
    }
    if row.source_status & !KNOWN_SOURCE_STATUS_MASK != 0 {
        return Err(format!(
            "row {index} source_status has unknown bits 0x{:X}",
            row.source_status & !KNOWN_SOURCE_STATUS_MASK
        ));
    }
    Ok(())
}

fn validate_naval_supremacy_row(row: &NavalSupremacyRow, index: usize) -> Result<(), String> {
    validate_optional_max(true, row.region_id, MAX_REGION_ID, "region_id", index)?;
    validate_optional_max(
        true,
        row.controller_tag_db_id,
        MAX_OWNER_TAG_DB_ID,
        "controller_tag_db_id",
        index,
    )?;
    if row.individual_ratio_raw > NAVAL_SUPREMACY_FIXED_SCALE_RAW {
        return Err(format!(
            "naval-supremacy row {index} individual_ratio_raw {} exceeds {}",
            row.individual_ratio_raw, NAVAL_SUPREMACY_FIXED_SCALE_RAW
        ));
    }
    Ok(())
}

fn validate_oob_id(value: u32, label: &str, index: usize) -> Result<(), String> {
    if value == 0 || value > MAX_OOB_ID {
        Err(format!("row {index} {label} has invalid id {value}"))
    } else {
        Ok(())
    }
}

fn validate_optional_max(
    present: bool,
    value: u32,
    max_value: u32,
    label: &str,
    index: usize,
) -> Result<(), String> {
    if present {
        if value == 0 || value > max_value {
            Err(format!("row {index} {label} has invalid value {value}"))
        } else {
            Ok(())
        }
    } else if value != 0 {
        Err(format!("row {index} {label} has value without flag"))
    } else {
        Ok(())
    }
}

/// Like `validate_optional_max` but 0 is a VALID present value (for enum
/// fields whose zero is a real member, e.g. the naval mission "hold" enum).
/// Presence is carried by the flag, so an absent field must still be 0.
fn validate_optional_enum(
    present: bool,
    value: u32,
    max_value: u32,
    label: &str,
    index: usize,
) -> Result<(), String> {
    if present {
        if value > max_value {
            Err(format!("row {index} {label} has invalid value {value}"))
        } else {
            Ok(())
        }
    } else if value != 0 {
        Err(format!("row {index} {label} has value without flag"))
    } else {
        Ok(())
    }
}

fn add_unit(summary: &mut Domain56GateSummary, key: &'static str, value: usize) {
    *summary.data_units_total.entry(key).or_default() += value as u64;
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ])
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn summary_json(summary: &Domain56GateSummary) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    json_bool(&mut out, "gate_passed", summary.gate_passed(), true);
    json_u64(&mut out, "record_count", summary.record_count, true);
    json_u64(
        &mut out,
        "data_record_count",
        summary.data_record_count,
        true,
    );
    json_u64(
        &mut out,
        "frame_end_record_count",
        summary.frame_end_record_count,
        true,
    );
    json_u64(&mut out, "row_count_total", summary.row_count_total, true);
    json_u64(
        &mut out,
        "changed_rows_total",
        summary.changed_rows_total,
        true,
    );
    json_u64(
        &mut out,
        "status_flags_or",
        summary.status_flags_or as u64,
        true,
    );
    out.push_str("  \"data_units_total\": {");
    for (index, (key, value)) in summary.data_units_total.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push('"');
        out.push_str(key);
        out.push_str("\": ");
        out.push_str(&value.to_string());
    }
    out.push_str("},\n");
    out.push_str("  \"parse_errors\": [");
    for (index, error) in summary.parse_errors.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        push_json_string(&mut out, error);
    }
    out.push_str("],\n");
    out.push_str("  \"ordering_errors\": [");
    for (index, error) in summary.ordering_errors.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        push_json_string(&mut out, error);
    }
    out.push_str("]\n}\n");
    out
}

fn json_bool(out: &mut String, key: &str, value: bool, comma: bool) {
    out.push_str("  \"");
    out.push_str(key);
    out.push_str("\": ");
    out.push_str(if value { "true" } else { "false" });
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn json_u64(out: &mut String, key: &str, value: u64, comma: bool) {
    out.push_str("  \"");
    out.push_str(key);
    out.push_str("\": ");
    out.push_str(&value.to_string());
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
}

fn main() {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: domain56_compact <collector_stream.bin>");
        process::exit(2);
    };
    let raw = match fs::read(&path) {
        Ok(raw) => raw,
        Err(err) => {
            eprintln!("read {path}: {err}");
            process::exit(2);
        }
    };
    let summary = gate_stream(&raw);
    print!("{}", summary_json(&summary));
    if !summary.gate_passed() {
        process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rows() -> Vec<NavalMissionStatusRow> {
        vec![
            NavalMissionStatusRow {
                task_force_id: 617,
                fleet_id: 615,
                owner_tag_db_id: 47,
                flags: FLAG_HAS_LOCATION
                    | FLAG_HAS_REGION
                    | FLAG_VALIDATED_DIRECT
                    | FLAG_DERIVED_REGION
                    | FLAG_FIXTURE_REQUIRED
                    | FLAG_HAS_SUNK_CONVOY_COUNT,
                location_province_id: 12389,
                strategic_region_id: 31,
                sunk_convoys: 2,
                source_status: STATUS_MISSION_FIXTURE_REQUIRED
                    | STATUS_HOME_BASE_FIXTURE_REQUIRED
                    | STATUS_REPAIR_RETREAT_FIXTURE_REQUIRED,
                ..NavalMissionStatusRow::default()
            },
            NavalMissionStatusRow {
                task_force_id: 618,
                fleet_id: 615,
                owner_tag_db_id: 47,
                flags: FLAG_HAS_LOCATION
                    | FLAG_VALIDATED_DIRECT
                    | FLAG_ACTIVE_BATTLE_FIXTURE_REQUIRED,
                location_province_id: 12390,
                source_status: STATUS_CONVOY_CONTEXT_FIXTURE_REQUIRED,
                ..NavalMissionStatusRow::default()
            },
        ]
    }

    fn sample_supremacy_rows() -> Vec<NavalSupremacyRow> {
        vec![
            NavalSupremacyRow {
                region_id: 16,
                controller_tag_db_id: 2,
                current_raw: 67_962_800,
                target_raw: 67_962_800,
                base_target_raw: 67_962_800,
                previous_raw: 67_962_800,
                decline_from_raw: 67_962_800,
                individual_ratio_raw: 87_173,
            },
            NavalSupremacyRow {
                region_id: 18,
                controller_tag_db_id: 44,
                current_raw: 448_057_205,
                target_raw: 448_057_205,
                base_target_raw: 76_245_200,
                previous_raw: 448_057_205,
                decline_from_raw: 448_057_205,
                individual_ratio_raw: 97_816,
            },
        ]
    }

    #[test]
    fn encodes_and_decodes_badged_domain56_payload() {
        let rows = sample_rows();
        let payload = encode_payload(
            &NavalMissionStatusHeader {
                elapsed_wall_ms: 42,
                changed_rows: 2,
                status_flags: STATUS_DELTA_DATA
                    | STATUS_MISSION_FIXTURE_REQUIRED
                    | STATUS_HOME_BASE_FIXTURE_REQUIRED,
                observed_task_forces: 2,
                validated_location_rows: 2,
                derived_region_rows: 1,
                fixture_required_rows: 2,
                convoy_sunk_count_rows: 1,
            },
            &rows,
        );
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.row_count_total, 2);
        assert_eq!(
            summary.data_units_total.get("naval_mission_location_rows"),
            Some(&2)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("naval_mission_sunk_convoy_rows"),
            Some(&1)
        );
    }

    #[test]
    fn compact_stream_gate_passes_for_domain56_data() {
        let rows = sample_rows();
        let payload = encode_payload(
            &NavalMissionStatusHeader {
                elapsed_wall_ms: 1,
                changed_rows: 2,
                status_flags: STATUS_DELTA_DATA,
                observed_task_forces: 2,
                validated_location_rows: 2,
                derived_region_rows: 1,
                fixture_required_rows: 2,
                convoy_sunk_count_rows: 1,
            },
            &rows,
        );
        let mut raw = Vec::new();
        raw.extend_from_slice(&compact_record(
            1,
            60787539,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        raw.extend_from_slice(&compact_record(
            2,
            60787539,
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[],
        ));
        let summary = gate_stream(&raw);
        assert!(summary.gate_passed(), "{:?}", summary.parse_errors);
        assert_eq!(summary.data_record_count, 1);
        assert_eq!(summary.frame_end_record_count, 1);
    }

    #[test]
    fn fixture_required_status_bits_do_not_overlap_shared_failure_mask() {
        const SHARED_FAILURE_MASK: u32 = (1 << 0)
            | (1 << 1)
            | (1 << 2)
            | (1 << 3)
            | (1 << 6)
            | (1 << 7)
            | (1 << 8)
            | (1 << 9)
            | (1 << 12);
        let fixture_bits = STATUS_MISSION_FIXTURE_REQUIRED
            | STATUS_HOME_BASE_FIXTURE_REQUIRED
            | STATUS_REPAIR_RETREAT_FIXTURE_REQUIRED
            | STATUS_CONVOY_CONTEXT_FIXTURE_REQUIRED;
        assert_eq!(fixture_bits & SHARED_FAILURE_MASK, 0);
    }

    #[test]
    fn decodes_payload_with_rows_for_world_fold() {
        let rows = sample_rows();
        let payload = encode_payload(
            &NavalMissionStatusHeader {
                elapsed_wall_ms: 42,
                changed_rows: 2,
                observed_task_forces: 2,
                validated_location_rows: 2,
                derived_region_rows: 1,
                fixture_required_rows: 2,
                convoy_sunk_count_rows: 1,
                ..NavalMissionStatusHeader::default()
            },
            &rows,
        );
        let decoded = decode_status_payload(0, &payload).unwrap();
        assert_eq!(decoded.header.changed_rows, 2);
        assert_eq!(decoded.rows.len(), 2);
        assert_eq!(decoded.rows[0].task_force_id, 617);
        assert_eq!(decoded.rows[0].strategic_region_id, 31);
    }

    #[test]
    fn naval_supremacy_trailer_round_trips() {
        let rows = sample_rows();
        let supremacy_rows = sample_supremacy_rows();
        let payload = encode_payload_with_naval_supremacy(
            &NavalMissionStatusHeader {
                elapsed_wall_ms: 42,
                changed_rows: 2,
                observed_task_forces: 2,
                validated_location_rows: 2,
                derived_region_rows: 1,
                fixture_required_rows: 2,
                convoy_sunk_count_rows: 1,
                ..NavalMissionStatusHeader::default()
            },
            &rows,
            &supremacy_rows,
        );

        let decoded = decode_status_payload(0, &payload).unwrap();
        assert_eq!(decoded.rows.len(), 2);
        assert_eq!(decoded.naval_supremacy_rows, supremacy_rows);

        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary.data_units_total.get("naval_supremacy_rows"),
            Some(&2)
        );
    }

    #[test]
    fn naval_supremacy_trailer_accepts_repeated_region_controller_with_distinct_values() {
        // The game's per-strategic-region naval-dominance array legitimately
        // holds MULTIPLE entries for the same (region, controller) — validated
        // live at D04 1944 (region 29 dominance tags [2,22,39,22]) and in the
        // acceptance fixture (5 repeated pairs with differing supremacy values).
        // Repeating (region, controller) while the six value fields differ must
        // be accepted, and BOTH occurrences must survive the decode.
        let rows = sample_rows();
        let mut supremacy_rows = sample_supremacy_rows();
        supremacy_rows[1].region_id = supremacy_rows[0].region_id;
        supremacy_rows[1].controller_tag_db_id = supremacy_rows[0].controller_tag_db_id;
        // The two rows share (region, controller) but differ in the value fields
        // (sample_supremacy_rows already gives them distinct payloads).
        assert_ne!(supremacy_rows[0], supremacy_rows[1]);
        let payload = encode_payload_with_naval_supremacy(
            &NavalMissionStatusHeader {
                elapsed_wall_ms: 42,
                changed_rows: 2,
                observed_task_forces: 2,
                validated_location_rows: 2,
                derived_region_rows: 1,
                fixture_required_rows: 2,
                convoy_sunk_count_rows: 1,
                ..NavalMissionStatusHeader::default()
            },
            &rows,
            &supremacy_rows,
        );

        let decoded = decode_status_payload(0, &payload).expect("payload decodes");
        assert_eq!(decoded.naval_supremacy_rows, supremacy_rows);
    }

    #[test]
    fn rejects_home_base_value_without_flag() {
        let row = sample_rows().remove(0);
        let mut payload = encode_payload(
            &NavalMissionStatusHeader {
                elapsed_wall_ms: 1,
                changed_rows: 1,
                observed_task_forces: 1,
                validated_location_rows: 1,
                fixture_required_rows: 1,
                ..NavalMissionStatusHeader::default()
            },
            &[row],
        );
        let home_base_offset = HEADER_LEN + 32;
        payload[home_base_offset..home_base_offset + 4].copy_from_slice(&99u32.to_le_bytes());
        let summary = decode_payload(7, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("home_base_province_id")));
    }

    #[test]
    fn rejects_missing_domain56_data_record() {
        let raw = compact_record(1, 1, 0, COMPACT_RECORD_TYPE_FRAME_END, &[]);
        let summary = gate_stream(&raw);
        assert!(!summary.gate_passed());
        assert_eq!(summary.data_record_count, 0);
    }
}
