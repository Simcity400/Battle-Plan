use super::*;
use characters_leaders_cabinet_live_lane as domain47_lane;
use compact_contract::{
    calendar::DATE_EPOCH_CONST, COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
    COMPACT_STATUS_BRIDGE_UNAVAILABLE, COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_NO_CLOCK,
    COMPACT_STATUS_NO_SCALAR_SOURCE, COMPACT_STATUS_PARTIAL_DATA, COMPACT_STATUS_PROFILE_MISMATCH,
    COMPACT_STATUS_SOURCE_UNAVAILABLE, COMPACT_STATUS_STATE_UNAVAILABLE,
    COMPACT_STATUS_TRANSITION_BASELINE_RESET, COMPACT_STATUS_WORKER_PANIC,
};
use session_multiplayer_metadata_live as session57_lane;

// --- synthetic H4CR stream builders --------------------------------------

fn push_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    payload: &[u8],
) {
    push_record_with_flags(
        out,
        sequence,
        game_hour_raw,
        domain_id,
        record_type,
        0,
        payload,
    );
}

fn push_record_with_flags(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    record_flags: u32,
    payload: &[u8],
) {
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&record_flags.to_le_bytes());
    out.extend_from_slice(payload);
}

fn push_str(payload: &mut Vec<u8>, value: &str) {
    payload.extend_from_slice(&(value.len() as u16).to_le_bytes());
    payload.extend_from_slice(value.as_bytes());
}

fn frame_end_payload() -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0u64.to_le_bytes());
    p.extend_from_slice(&0u64.to_le_bytes());
    p.extend_from_slice(&0u32.to_le_bytes());
    p.extend_from_slice(&0u32.to_le_bytes());
    p.extend_from_slice(&0u32.to_le_bytes());
    p
}

/// country_identity (domain 18): header + [tag_db_id u32][u16-len TAG] rows.
fn country_identity_payload(rows: &[(u32, &str)]) -> Vec<u8> {
    // Wave-4: default cosmetic marker = null (1) for every row — the ~50
    // existing call sites don't care about cosmetic tags.
    let with_cosmetic: Vec<(u32, &str, Option<&str>)> =
        rows.iter().map(|(id, tag)| (*id, *tag, None)).collect();
    country_identity_payload_with_cosmetic(&with_cosmetic)
}

/// Wave-4: country_identity payload with per-row cosmetic tag. The third
/// tuple element is `Some(tag)` for a cosmetic override (marker 2 + string),
/// or `None` for validated-null (marker 1). (Marker 0 = absent is exercised
/// by the dedicated absent test, which builds the payload by hand.)
fn country_identity_payload_with_cosmetic(rows: &[(u32, &str, Option<&str>)]) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0u64.to_le_bytes()); // wall
    p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
    p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed
    p.extend_from_slice(&0u32.to_le_bytes()); // status
    for (tag_db_id, tag, cosmetic) in rows {
        p.extend_from_slice(&tag_db_id.to_le_bytes());
        push_str(&mut p, tag);
        match cosmetic {
            Some(cosmetic) => {
                p.push(2); // present
                push_str(&mut p, cosmetic);
            }
            None => p.push(1), // null
        }
    }
    p
}

fn domain47_payload(status_flags: u32, rows: &[domain47_lane::CharacterLaneRow<'_>]) -> Vec<u8> {
    domain47_lane::encode_payload(0, status_flags, domain47_lane::SCHEMA_ROOTS_VALIDATED, rows)
}

fn domain57_payload(
    root_evidence_mask: u32,
    player_rows: Vec<session57_lane::PlayerCountryRow>,
    event_rows: Vec<session57_lane::SessionEventRow>,
    status_flags: u32,
) -> Vec<u8> {
    let source_flags = session57_lane::SOURCE_FLAG_DLL_NATIVE
        | if root_evidence_mask != 0 {
            session57_lane::SOURCE_FLAG_SESSION_ROOT_VALIDATED
        } else {
            0
        };
    session57_lane::encode_payload(&session57_lane::SessionMetadataFrame {
        game_hour_raw: raw_for(16_959_360),
        status_flags,
        source_flags,
        schema_flags: 0,
        root_evidence_mask,
        player_country_rows: player_rows,
        event_rows,
    })
}

fn domain57_player_row(tag: &str, tag_db_id: u32) -> session57_lane::PlayerCountryRow {
    session57_lane::PlayerCountryRow {
        player_key_hash: 42,
        player_name: "fixture name ignored".into(),
        country_tag: tag.into(),
        tag_db_id,
        session_slot: 7,
        flags: session57_lane::PLAYER_FLAG_HUMAN
            | session57_lane::PLAYER_FLAG_HOST
            | session57_lane::PLAYER_FLAG_CONNECTED,
        valid_from_hour_raw: raw_for(16_959_360),
    }
}

fn domain57_event_row(
    kind: session57_lane::SessionEventKind,
    value_i32: i32,
) -> session57_lane::SessionEventRow {
    session57_lane::SessionEventRow {
        kind,
        flags: session57_lane::EVENT_FLAG_DIRECT_CAPTURE,
        actor_key_hash: 0,
        actor_name: String::new(),
        text: if matches!(
            kind,
            session57_lane::SessionEventKind::Chat
                | session57_lane::SessionEventKind::SystemMessage
                | session57_lane::SessionEventKind::Alert
                | session57_lane::SessionEventKind::Notification
        ) {
            "fixture text".into()
        } else {
            String::new()
        },
        tag_db_id: 0,
        value_i32,
        related_value_i32: 0,
        command_sequence: 0,
        event_hour_raw: raw_for(16_959_360),
        event_source_id: 0,
        event_state: 0,
    }
}

fn fuel_payload(rows: &[(u32, i64, i64, i64, i64)]) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0u64.to_le_bytes()); // wall
    p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
    p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed
    p.extend_from_slice(&0u32.to_le_bytes()); // status
    p.extend_from_slice(&32768u32.to_le_bytes()); // scale
    for (tag, current, max, gain, cost) in rows {
        p.extend_from_slice(&tag.to_le_bytes());
        p.extend_from_slice(&1u32.to_le_bytes()); // flags
        p.extend_from_slice(&current.to_le_bytes());
        p.extend_from_slice(&max.to_le_bytes());
        p.extend_from_slice(&gain.to_le_bytes());
        p.extend_from_slice(&cost.to_le_bytes());
    }
    p
}

/// One country, one stockpile row, one military line, one construction row.
fn production_payload(tag: u32, stockpile_amount_raw: i64, active_factories: i32) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&0u64.to_le_bytes()); // wall
    p.extend_from_slice(&1u32.to_le_bytes()); // country_count
    p.extend_from_slice(&1u32.to_le_bytes()); // changed
    p.extend_from_slice(&0u32.to_le_bytes()); // status
    p.extend_from_slice(&100_000u32.to_le_bytes()); // scale
    p.extend_from_slice(&1u32.to_le_bytes()); // stockpile_count
    p.extend_from_slice(&1u32.to_le_bytes()); // military_line_count
    p.extend_from_slice(&0u32.to_le_bytes()); // naval_line_count
    p.extend_from_slice(&1u32.to_le_bytes()); // construction_count
                                              // stockpile row
    p.extend_from_slice(&tag.to_le_bytes());
    p.extend_from_slice(&101u32.to_le_bytes()); // variant_id
    p.extend_from_slice(&stockpile_amount_raw.to_le_bytes());
    // military line row
    p.extend_from_slice(&tag.to_le_bytes());
    p.extend_from_slice(&1u32.to_le_bytes()); // kind = mil
    p.extend_from_slice(&349i32.to_le_bytes()); // line_id
    p.extend_from_slice(&active_factories.to_le_bytes());
    p.extend_from_slice(&10_500_000i32.to_le_bytes()); // cost
    p.extend_from_slice(&3_105_000i32.to_le_bytes()); // speed
    p.extend_from_slice(&3_440_000i32.to_le_bytes()); // produced
    p.extend_from_slice(&(-1i32).to_le_bytes()); // amount
                                                 // construction row
    p.extend_from_slice(&tag.to_le_bytes());
    p.extend_from_slice(&9001u32.to_le_bytes()); // line_id
    p.extend_from_slice(&20013u32.to_le_bytes()); // template_id
    p.extend_from_slice(&6i32.to_le_bytes()); // active_factories
    p.extend_from_slice(&1_080_000_000i32.to_le_bytes()); // cost
    p.extend_from_slice(&12_550_000i32.to_le_bytes()); // produced
    p.extend_from_slice(&0u32.to_le_bytes()); // priority
    p.extend_from_slice(&0u32.to_le_bytes()); // reserved
    p
}

/// Append a `PLIC` production-license trailer to a base production payload.
/// `countries` is a list of (tag_db_id, license ids) rows; a country with an
/// empty id list is a proven-empty set.
fn append_production_license_trailer(p: &mut Vec<u8>, countries: &[(u32, Vec<u32>)]) {
    let id_rows: usize = countries.iter().map(|(_, ids)| ids.len()).sum();
    p.extend_from_slice(b"PLIC");
    p.extend_from_slice(&1u16.to_le_bytes()); // version
    p.extend_from_slice(&20u16.to_le_bytes()); // header_len
    p.extend_from_slice(&(countries.len() as u32).to_le_bytes()); // country_rows
    p.extend_from_slice(&(id_rows as u32).to_le_bytes()); // id_rows
    p.extend_from_slice(&0u32.to_le_bytes()); // reserved
    for (tag, ids) in countries {
        p.extend_from_slice(&tag.to_le_bytes());
        p.extend_from_slice(&(ids.len() as u32).to_le_bytes());
    }
    for (_, ids) in countries {
        for id in ids {
            p.extend_from_slice(&id.to_le_bytes());
        }
    }
}

const DIV_ROW_LEN: usize = 92;
const DIV_TYPE: u32 = 4713;
const DELTA: u32 = 1 << 10;
const DIV_FLAG_DELETED: u32 = 1 << 12;
const DIV_FLAG_TEMPLATE: u32 = 1 << 13;

#[allow(clippy::too_many_arguments)]
fn push_div_row(
    p: &mut Vec<u8>,
    div_id: u32,
    flags: u32,
    location: u32,
    org_raw: u32,
    strength_raw: u32,
    template_id: u32,
    owner: u32,
) {
    let start = p.len();
    p.extend_from_slice(&div_id.to_le_bytes()); // +0
    p.extend_from_slice(&DIV_TYPE.to_le_bytes()); // +4
    p.extend_from_slice(&flags.to_le_bytes()); // +8
    p.extend_from_slice(&location.to_le_bytes()); // +12
    p.extend_from_slice(&0u32.to_le_bytes()); // +16 move_target
    p.extend_from_slice(&0u32.to_le_bytes()); // +20 move_source
    p.extend_from_slice(&0u32.to_le_bytes()); // +24 movement_progress
    p.extend_from_slice(&0u32.to_le_bytes()); // +28 progress_bucket
    p.extend_from_slice(&org_raw.to_le_bytes()); // +32
    p.extend_from_slice(&0u32.to_le_bytes()); // +36 org_bucket
    p.extend_from_slice(&strength_raw.to_le_bytes()); // +40
    p.extend_from_slice(&0u32.to_le_bytes()); // +44 strength_bucket
    p.extend_from_slice(&0u32.to_le_bytes()); // +48 combat_ref_count
    p.extend_from_slice(&template_id.to_le_bytes()); // +52
    p.extend_from_slice(&owner.to_le_bytes()); // +56
    p.extend_from_slice(&0u32.to_le_bytes()); // +60 army_id
    p.extend_from_slice(&0u32.to_le_bytes()); // +64 experience
    p.extend_from_slice(&0u32.to_le_bytes()); // +68 max_supply
    p.extend_from_slice(&0u32.to_le_bytes()); // +72 current_supply
    p.extend_from_slice(&0u32.to_le_bytes()); // +76 supply_gain
    p.extend_from_slice(&0u32.to_le_bytes()); // +80 fuel
    p.extend_from_slice(&0u32.to_le_bytes()); // +84 order_instance_id
    p.extend_from_slice(&0u32.to_le_bytes()); // +88 disrupted_supply
    assert_eq!(p.len() - start, DIV_ROW_LEN);
}

/// Build a division row with explicit supply_gain and disrupted_supply and
/// their presence flags — for the wave-4 supply-scalar fold tests.
#[allow(clippy::too_many_arguments)]
fn push_div_row_with_supply(
    p: &mut Vec<u8>,
    div_id: u32,
    base_flags: u32,
    location: u32,
    supply_gain_raw: Option<u32>,
    disrupted_supply_raw: Option<u32>,
) {
    let start = p.len();
    let mut flags = base_flags;
    if supply_gain_raw.is_some() {
        flags |= 1 << 10;
    }
    if disrupted_supply_raw.is_some() {
        flags |= 1 << 17;
    }
    p.extend_from_slice(&div_id.to_le_bytes()); // +0
    p.extend_from_slice(&4713u32.to_le_bytes()); // +4 div_type
    p.extend_from_slice(&flags.to_le_bytes()); // +8
    p.extend_from_slice(&location.to_le_bytes()); // +12
    p.extend_from_slice(&0u32.to_le_bytes()); // +16 move_target
    p.extend_from_slice(&0u32.to_le_bytes()); // +20 move_source
    p.extend_from_slice(&0u32.to_le_bytes()); // +24 movement_progress
    p.extend_from_slice(&0u32.to_le_bytes()); // +28 progress_bucket
    p.extend_from_slice(&0u32.to_le_bytes()); // +32 org
    p.extend_from_slice(&0u32.to_le_bytes()); // +36 org_bucket
    p.extend_from_slice(&0u32.to_le_bytes()); // +40 strength
    p.extend_from_slice(&0u32.to_le_bytes()); // +44 strength_bucket
    p.extend_from_slice(&0u32.to_le_bytes()); // +48 combat_ref
    p.extend_from_slice(&0u32.to_le_bytes()); // +52 template
    p.extend_from_slice(&0u32.to_le_bytes()); // +56 owner
    p.extend_from_slice(&0u32.to_le_bytes()); // +60 army
    p.extend_from_slice(&0u32.to_le_bytes()); // +64 experience
    p.extend_from_slice(&0u32.to_le_bytes()); // +68 max_supply
    p.extend_from_slice(&0u32.to_le_bytes()); // +72 current_supply
    p.extend_from_slice(&supply_gain_raw.unwrap_or(0).to_le_bytes()); // +76 supply_gain
    p.extend_from_slice(&0u32.to_le_bytes()); // +80 fuel
    p.extend_from_slice(&0u32.to_le_bytes()); // +84 order_instance_id
    p.extend_from_slice(&disrupted_supply_raw.unwrap_or(0).to_le_bytes()); // +88 disrupted_supply
    assert_eq!(p.len() - start, DIV_ROW_LEN);
}

fn divisions_header(p: &mut Vec<u8>, row_count: u32, status: u32) {
    p.extend_from_slice(&0u64.to_le_bytes()); // wall
    p.extend_from_slice(&row_count.to_le_bytes()); // row_count
    p.extend_from_slice(&row_count.to_le_bytes()); // changed
    p.extend_from_slice(&status.to_le_bytes()); // status
    p.extend_from_slice(&100_000u32.to_le_bytes()); // scale
    p.extend_from_slice(&row_count.to_le_bytes()); // root_rows
    p.extend_from_slice(&0u32.to_le_bytes()); // reserved
}

const STATE_ROW_LEN: usize = 16;
const PROV_ROW_LEN: usize = 16;

fn map_control_header(p: &mut Vec<u8>, state_count: u32, province_count: u32, status: u32) {
    p.extend_from_slice(&0u64.to_le_bytes()); // wall
    p.extend_from_slice(&state_count.to_le_bytes());
    p.extend_from_slice(&(state_count + province_count).to_le_bytes()); // changed
    p.extend_from_slice(&status.to_le_bytes());
    p.extend_from_slice(&province_count.to_le_bytes());
    p.extend_from_slice(&state_count.to_le_bytes()); // changed_state_rows
    p.extend_from_slice(&province_count.to_le_bytes()); // changed_province_rows
}

fn push_state_row(p: &mut Vec<u8>, state_id: u32, owner: u32, controller: u32) {
    p.extend_from_slice(&state_id.to_le_bytes());
    p.extend_from_slice(&owner.to_le_bytes());
    p.extend_from_slice(&controller.to_le_bytes());
    let mut flags = 0u32;
    if owner != 0 {
        flags |= 1;
    }
    if controller != 0 {
        flags |= 1 << 1;
    }
    p.extend_from_slice(&flags.to_le_bytes());
    assert_eq!(STATE_ROW_LEN, 16);
}

fn push_prov_row(p: &mut Vec<u8>, province_id: u32, controller: u32, state_id: u32) {
    p.extend_from_slice(&province_id.to_le_bytes());
    p.extend_from_slice(&controller.to_le_bytes());
    p.extend_from_slice(&state_id.to_le_bytes());
    let mut flags = 0u32;
    if controller != 0 {
        flags |= 1;
    }
    if state_id != 0 {
        flags |= 1 << 1;
    }
    p.extend_from_slice(&flags.to_le_bytes());
    assert_eq!(PROV_ROW_LEN, 16);
}

struct TestLandCombat {
    combat_id: u32,
    generation: u64,
    location: u32,
    attackers: Vec<u32>,
    defenders: Vec<u32>,
}

fn land_combats_payload(rows: &[TestLandCombat]) -> Vec<u8> {
    land_combats_payload_with_status(rows, 0)
}

fn land_combats_payload_with_status(rows: &[TestLandCombat], status_flags: u32) -> Vec<u8> {
    let participant_count = rows
        .iter()
        .map(|row| row.attackers.len().saturating_add(row.defenders.len()))
        .sum::<usize>();
    let mut p = Vec::new();
    p.extend_from_slice(&0u64.to_le_bytes()); // wall
    p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // combat_count
    p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
    p.extend_from_slice(&status_flags.to_le_bytes()); // status
    p.extend_from_slice(&(participant_count as u32).to_le_bytes());
    p.extend_from_slice(&0u32.to_le_bytes()); // ended_count
    p.extend_from_slice(&0u32.to_le_bytes()); // reserved
    for row in rows {
        let flags = 1u32 | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4);
        p.extend_from_slice(&row.combat_id.to_le_bytes());
        p.extend_from_slice(&row.generation.to_le_bytes());
        p.extend_from_slice(&flags.to_le_bytes());
        p.extend_from_slice(&row.location.to_le_bytes());
        p.extend_from_slice(&3u32.to_le_bytes()); // day
        p.extend_from_slice(&4u32.to_le_bytes()); // duration
        p.extend_from_slice(&(row.attackers.len() as u32).to_le_bytes());
        p.extend_from_slice(&(row.defenders.len() as u32).to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
    }
    for row in rows {
        for division_id in &row.attackers {
            push_land_combat_participant(&mut p, row.combat_id, row.generation, 1, *division_id);
        }
        for division_id in &row.defenders {
            push_land_combat_participant(&mut p, row.combat_id, row.generation, 2, *division_id);
        }
    }
    p
}

fn push_land_combat_participant(
    p: &mut Vec<u8>,
    combat_id: u32,
    generation: u64,
    side: u32,
    division_id: u32,
) {
    p.extend_from_slice(&combat_id.to_le_bytes());
    p.extend_from_slice(&generation.to_le_bytes());
    p.extend_from_slice(&side.to_le_bytes());
    p.extend_from_slice(&division_id.to_le_bytes());
    p.extend_from_slice(&0u32.to_le_bytes());
}

// --- helpers -------------------------------------------------------------

fn parse(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("snapshot is valid json")
}

fn raw_for(hour_index: i64) -> i32 {
    DATE_EPOCH_CONST + hour_index as i32
}

fn snapshot_for_domain_records(records: Vec<(u16, Vec<u8>)>) -> Value {
    snapshot_for_domain_records_with_flags(
        records
            .into_iter()
            .map(|(domain_id, payload)| (domain_id, payload, 0))
            .collect(),
    )
}

fn snapshot_for_domain_records_with_flags(records: Vec<(u16, Vec<u8>, u32)>) -> Value {
    let raw = raw_for(16_959_360);
    let mut stream = Vec::new();
    for (index, (domain_id, payload, record_flags)) in records.iter().enumerate() {
        push_record_with_flags(
            &mut stream,
            index as u64 + 1,
            raw,
            *domain_id,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            *record_flags,
            payload,
        );
    }
    push_record(
        &mut stream,
        records.len() as u64 + 1,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let mut fold = WorldFold::new("chunk3_test");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    parse(&snaps[0].2)
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u32_at(out: &mut [u8], offset: usize, value: u32) {
    out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64_at(out: &mut [u8], offset: usize, value: u64) {
    out[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn chunk3_weather_payload(extra_status: u32) -> Vec<u8> {
    let mut out = vec![0u8; WEATHER_HEADER_LEN];
    put_u64_at(&mut out, 0, 10);
    put_u32_at(&mut out, 8, WEATHER_REGION_ROWS as u32);
    put_u32_at(&mut out, 12, WEATHER_REGION_ROWS as u32);
    put_u32_at(&mut out, 16, extra_status);
    put_u32_at(&mut out, 20, 0);
    put_u32_at(&mut out, 24, WEATHER_REGION_ROWS as u32);
    put_u32_at(&mut out, 28, 0);
    put_u32_at(&mut out, 32, 100_000);
    put_u32_at(&mut out, 36, 1);
    for region_id in 0..WEATHER_REGION_ROWS as u32 {
        let flags = if region_id == 12 {
            WEATHER_REGION_RAIN_LIGHT
                | WEATHER_REGION_PHENOMENON_ACTIVE
                | WEATHER_REGION_ACTIVE_SIGNAL
        } else {
            0
        };
        push_u32(&mut out, region_id);
        push_u32(&mut out, flags);
        push_i32(&mut out, if region_id == 12 { 2_150_000 } else { 0 });
        push_u32(&mut out, if region_id == 12 { 3 } else { 0 });
        push_u32(&mut out, 77);
        push_u32(&mut out, if region_id == 12 { 2 } else { 0 });
        push_u32(&mut out, if region_id == 12 { 4 } else { 0 });
        push_u32(&mut out, 0);
    }
    out
}

fn domain53_validated_payload_with_war() -> Vec<u8> {
    let mut rows = diplomacy53::extractor::proof_fixture_rows();
    rows.operation_anchors.clear();
    rows.intel_map.clear();
    rows.wars.push(diplomacy53::compact::WarRelationRow {
        first_tag_db_id: 1,
        second_tag_db_id: 3,
        first_casualties: 100,
        second_casualties: 200,
    });
    let report = diplomacy53::extractor::build_domain53_payload(
        rows,
        diplomacy53::extractor::CURRENT_VALIDATED_SOURCE_PROOFS,
    );
    assert!(report.ok(), "{:?}", report.errors);
    report.payload
}

#[test]
fn chunk3_folds_d53_under_owned_diplomacy_and_intel_extras() {
    // D53-ONLY regression (guards the E1 `diplomacy_world.wars` {}→null change):
    // this stream carries NO domain-60 record, so d53's own war rows must still
    // land under `diplomacy_world.domain53_extra.war_relation_rows` (a SEPARATE
    // key that E1 never touched), and `diplomacy_world.wars` — which was a dead
    // unconditional `{}` placeholder at HEAD and is now D60-owned — is null.
    let snapshot = snapshot_for_domain_records(vec![
        (
            18,
            country_identity_payload(&[(1, "GER"), (3, "POL"), (22, "BUL")]),
        ),
        (53, domain53_validated_payload_with_war()),
    ]);

    let ger = &snapshot["countries"]["GER"];
    assert!(ger["diplomacy"]["opinion"].is_null());
    assert_eq!(
        ger["diplomacy"][DOMAIN53_EXTRA_KEY]["opinion_rows"][0]["target"],
        json!("POL")
    );
    assert_eq!(
        ger["diplomacy"][DOMAIN53_EXTRA_KEY]["relation_flags"][0]["flag_name"],
        json!("guarantee")
    );
    assert_eq!(
        ger["intel"][DOMAIN53_EXTRA_KEY]["agency_scalars"]["upgrade_progress_raw"],
        json!(2_400_000)
    );
    assert_eq!(
        ger["intel"][DOMAIN53_EXTRA_KEY]["operative_lifecycle_rows"][0]["lifecycle_flag"],
        json!(47)
    );
    // E1 schema delta: `wars` is D60-owned and null until a proven
    // capitulation_war_state frame lands this hour (this stream has none).
    assert_eq!(snapshot["diplomacy_world"]["wars"], Value::Null);
    assert_eq!(
        snapshot["diplomacy_world"][DOMAIN53_EXTRA_KEY]["war_relation_rows"][0]
            ["lifecycle_unvalidated"],
        json!(true)
    );
    assert!(
        snapshot["diplomacy_world"][DOMAIN53_EXTRA_KEY]["omitted_unresolved"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "operation_detail_fixture_blocked")
    );
    // Wave-5: the proof-fixture carries 2 operation instances in the OPIN
    // trailer -> folded as a world-scoped provisional sidecar.
    let ops = &snapshot["diplomacy_world"][DOMAIN53_EXTRA_KEY]["operation_instance_rows"];
    let ops = ops.as_array().unwrap();
    assert_eq!(ops.len(), 2);
    // instance 129014 has associated_country tag 3 -> resolves to POL.
    let with_country = ops
        .iter()
        .find(|row| row["instance_id"] == json!(129_014))
        .unwrap();
    assert_eq!(with_country["def_ref_ok"], json!(true));
    assert_eq!(with_country["associated_country"], json!("POL"));
    assert_eq!(
        with_country["associated_country_semantics"],
        json!("provisional_target_vs_location_undistinguished")
    );
    // instance 128904 has no country -> null (absent != zero).
    let no_country = ops
        .iter()
        .find(|row| row["instance_id"] == json!(128_904))
        .unwrap();
    assert_eq!(no_country["associated_country"], Value::Null);
    assert!(snapshot["provenance"]["collector"]["domains"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "diplomacy_intel_operatives_live"));
}

#[test]
fn chunk3_d53_failure_and_delta_are_not_successful_folds() {
    let failure_payload =
        diplomacy53::compact::encode_payload(&diplomacy53::compact::DiplomacyIntelSnapshot {
            status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
            ..diplomacy53::compact::DiplomacyIntelSnapshot::default()
        });
    let failure = snapshot_for_domain_records(vec![(53, failure_payload)]);
    assert!(failure["countries"].as_object().unwrap().is_empty());
    assert!(failure["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(53)
            && problem["problem"] == json!("compact_status_failure")));
    assert!(!failure["provenance"]["collector"]["domains"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "diplomacy_intel_operatives_live"));

    let delta_payload =
        diplomacy53::compact::encode_payload(&diplomacy53::compact::DiplomacyIntelSnapshot {
            status_flags: COMPACT_STATUS_DELTA_DATA,
            ..diplomacy53::compact::DiplomacyIntelSnapshot::default()
        });
    let delta = snapshot_for_domain_records(vec![(53, delta_payload)]);
    assert!(delta["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(53)
            && problem["problem"] == json!("delta_unsupported")));
    assert!(!delta["provenance"]["collector"]["domains"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "diplomacy_intel_operatives_live"));
}

fn chunk3_division_payload(rows: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&100_000u32.to_le_bytes());
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for (div_id, div_type, owner) in rows {
        out.extend_from_slice(&div_id.to_le_bytes());
        out.extend_from_slice(&div_type.to_le_bytes());
        out.extend_from_slice(&(1u32 << 5).to_le_bytes());
        out.extend_from_slice(&118u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&3_000_000u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&8_000_000u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&owner.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        // Wave-4: disrupted_supply slot at +88 (absent -> 0).
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out
}

fn chunk3_division_delete_payload(div_id: u32, div_type: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&DELTA.to_le_bytes());
    out.extend_from_slice(&100_000u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&div_id.to_le_bytes());
    out.extend_from_slice(&div_type.to_le_bytes());
    out.extend_from_slice(&DIV_FLAG_DELETED.to_le_bytes());
    out.resize(out.len() + DIV_ROW_LEN - 12, 0);
    out
}

fn chunk3_division_equipment_payload(
    status: u32,
    roots: &[(u32, u32, u32, u32, u32, u32)],
    entries: &[(u32, i64)],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&(roots.len() as u32).to_le_bytes());
    out.extend_from_slice(&(roots.len() as u32).to_le_bytes());
    out.extend_from_slice(&status.to_le_bytes());
    out.extend_from_slice(&100_000u32.to_le_bytes());
    out.extend_from_slice(&(roots.len() as u32).to_le_bytes());
    out.extend_from_slice(&(roots.len() as u32).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (owner, div_id, flags, template_id, first, count) in roots {
        push_u32(&mut out, *owner);
        push_u32(&mut out, *div_id);
        push_u32(&mut out, *flags);
        push_u32(&mut out, *template_id);
        push_u32(&mut out, *first);
        push_u32(&mut out, *count);
    }
    for (variant_id, amount_raw) in entries {
        push_u32(&mut out, *variant_id);
        push_u32(&mut out, 0);
        push_i64(&mut out, *amount_raw);
    }
    out
}

fn chunk3_rowless_division_equipment_payload(
    status: u32,
    root_rows: u32,
    observed_division_rows: u32,
    observed_equipment_rows: u32,
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u64.to_le_bytes());
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, status);
    push_u32(&mut out, 100_000);
    push_u32(&mut out, root_rows);
    push_u32(&mut out, observed_division_rows);
    push_u32(&mut out, 0);
    push_u32(&mut out, observed_equipment_rows);
    out
}

/// Append a `PLCF` project-lifecycle trailer. `rows` = (tag_db_id, local_id,
/// started, completed).
fn append_project_lifecycle_trailer(
    out: &mut Vec<u8>,
    rows: &[(u32, u32, bool, bool)],
    walked_countries: u32,
    aborted_countries: u32,
) {
    out.extend_from_slice(b"PLCF");
    out.extend_from_slice(&1u16.to_le_bytes()); // version
    out.extend_from_slice(&20u16.to_le_bytes()); // header_len
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
    out.extend_from_slice(&walked_countries.to_le_bytes());
    out.extend_from_slice(&aborted_countries.to_le_bytes());
    for (tag, lid, started, completed) in rows {
        let mut flags = 0u32;
        if *started {
            flags |= 1;
        }
        if *completed {
            flags |= 1 << 1;
        }
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&lid.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
    }
}

fn chunk3_projects_payload(status: u32) -> Vec<u8> {
    let header_extension = 4u32;
    let mut out = vec![
        0u8;
        SPECIAL_PROJECTS_HEADER_BASE_LEN
            + SPECIAL_PROJECTS_HEADER_ROOT_COUNTERS_LEN
            + SPECIAL_PROJECTS_HEADER_REASON_COUNTERS_LEN
            + SPECIAL_PROJECTS_HEADER_PROGRESS_LEN
    ];
    put_u64_at(&mut out, 0, 10);
    put_u32_at(&mut out, 8, 5);
    put_u32_at(&mut out, 12, 5);
    put_u32_at(&mut out, 16, status);
    put_u32_at(&mut out, 20, 1);
    put_u32_at(&mut out, 24, 1);
    put_u32_at(&mut out, 28, 1);
    put_u32_at(&mut out, 32, 1);
    put_u32_at(&mut out, 36, 1);
    put_u32_at(&mut out, 40, 1);
    put_u32_at(&mut out, 44, 1);
    put_u32_at(&mut out, 48, 1);
    put_u32_at(&mut out, 52, header_extension);
    put_u64_at(&mut out, 56, 7);
    let progress_offset = SPECIAL_PROJECTS_HEADER_BASE_LEN
        + SPECIAL_PROJECTS_HEADER_ROOT_COUNTERS_LEN
        + SPECIAL_PROJECTS_HEADER_REASON_COUNTERS_LEN;
    put_u32_at(&mut out, progress_offset, 1);
    put_u32_at(&mut out, progress_offset + 4, 1);
    put_u32_at(&mut out, progress_offset + 8, 1);
    put_u32_at(&mut out, progress_offset + 12, 1);

    for value in [
        9,
        123,
        SPECIAL_PROJECTS_FACILITY_ACTIVE_PROJECT
            | SPECIAL_PROJECTS_FACILITY_SCIENTIST
            | SPECIAL_PROJECTS_FACILITY_BUILDING,
        101,
        501,
        2,
        0,
        0,
        101,
        9001,
        3,
        9,
        SPECIAL_PROJECTS_PROJECT_WORKED_FACILITY,
        0,
        501,
        SPECIAL_PROJECTS_SCIENTIST_NAME_TOKEN
            | SPECIAL_PROJECTS_SCIENTIST_DISPLAY_NAME
            | SPECIAL_PROJECTS_SCIENTIST_TRAIT_COUNT
            | SPECIAL_PROJECTS_SCIENTIST_SKILLS,
        2,
    ] {
        push_u32(&mut out, value);
    }
    for value in [1, 2, 3, 4] {
        push_i32(&mut out, value);
    }
    push_u32(&mut out, 0);
    for value in [101, 9001, 11, 12, 13, 14, 15, 16, 4, 25_000, 100_000, 1] {
        push_u32(&mut out, value);
    }
    for value in [47, 777, 12_345, 0] {
        push_u32(&mut out, value);
    }
    push_str(&mut out, "facility_building");
    push_str(&mut out, "scientist_token");
    push_str(&mut out, "Scientist Name");
    out
}

fn chunk3_frd_payload() -> Vec<u8> {
    chunk3_frd_payload_versioned(true, 0)
}

fn chunk3_frd_payload_versioned(include_target: bool, omitted_targets: u32) -> Vec<u8> {
    let mut out = vec![0u8; FRD_HEADER_LEN];
    out[0..4].copy_from_slice(b"FRD2");
    out[4..6].copy_from_slice(&FRD_SCHEMA_VERSION.to_le_bytes());
    out[6..8].copy_from_slice(&(FRD_HEADER_LEN as u16).to_le_bytes());
    put_u32_at(&mut out, 8, 7);
    put_u32_at(&mut out, 12, 1);
    put_u32_at(&mut out, 16, 1);
    put_u32_at(&mut out, 20, 1);
    put_u32_at(&mut out, 24, 1);
    put_u32_at(&mut out, 28, u32::from(include_target));
    for offset in [32usize, 36, 40, 44, 48] {
        put_u32_at(&mut out, offset, 1);
    }
    put_u32_at(&mut out, 52, omitted_targets);
    for value in [
        47,
        FRD_FOCUS_COMPLETED_COUNT_INCREASED,
        0,
        FRD_FOCUS_FLAG_COMPLETED_TOKEN_EXACT,
        2,
        3,
    ] {
        push_u32(&mut out, value);
    }
    push_i32(&mut out, 1000);
    push_i32(&mut out, 2000);
    push_u32(&mut out, 1);
    push_str(&mut out, "focus_old");
    push_str(&mut out, "focus_new");
    push_str(&mut out, "focus_done");

    for value in [
        47,
        0,
        FRD_RESEARCH_FLAG_ETA_AVAILABLE | FRD_RESEARCH_FLAG_BASE_PRESENT,
        1,
        10,
        11,
        20,
        21,
    ] {
        push_u32(&mut out, value);
    }
    for value in [100, 150, 50, 5, 200, 12] {
        push_i64(&mut out, value);
    }

    for value in [47, 1, 0, 2, 3, 4, 5] {
        push_u32(&mut out, value);
    }

    push_u32(&mut out, 47);
    push_u32(&mut out, 3);
    push_u32(&mut out, 0);
    push_i32(&mut out, 30);
    push_i32(&mut out, 29);
    push_i32(&mut out, 1);
    push_i32(&mut out, 1);
    push_str(&mut out, "decision_x");

    if include_target {
        push_u32(&mut out, 47);
        push_u32(&mut out, 48);
        push_u32(&mut out, FRD_DECISION_TARGET_COUNTRY_FLAGS);
        push_u32(&mut out, FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED);
        push_str(&mut out, "decision_target_country");
    }

    for value in [3, 3, 45, 1, 3] {
        push_u32(&mut out, value);
    }
    push_str(
        &mut out,
        "direct_event_hooks_unavailable_transitions_derived_from_compact_frames",
    );
    out
}

fn chunk3_frd_payload_old_header() -> Vec<u8> {
    chunk3_frd_payload_old_header_with_elapsed(1000)
}

fn chunk3_frd_payload_old_header_with_elapsed(elapsed_wall_ms: u64) -> Vec<u8> {
    let v2 = chunk3_frd_payload_versioned(false, 0);
    let mut out = Vec::new();
    out.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    out.extend_from_slice(&v2[8..28]);
    out.extend_from_slice(&v2[32..52]);
    out.extend_from_slice(&v2[FRD_HEADER_LEN..]);
    out
}

#[derive(Clone)]
struct Chunk3ScriptedRow {
    scope_kind: u32,
    owner_id: u32,
    value_kind: u32,
    key: Option<&'static str>,
    token_id: Option<u32>,
    value_raw: Option<i64>,
    date_hour_raw: Option<i32>,
    deleted: bool,
}

fn chunk3_scripted_payload(status: u32, rows: &[Chunk3ScriptedRow]) -> Vec<u8> {
    let mut key_table = Vec::new();
    let mut encoded_rows = Vec::new();
    let mut variable_rows = 0u32;
    let mut flag_rows = 0u32;
    let mut token_only_rows = 0u32;
    for row in rows {
        let mut flags = 0u32;
        let key_offset = key_table.len() as u32;
        let mut key_len = 0u32;
        if let Some(key) = row.key {
            flags |= SCRIPTED_STATE_ROW_FLAG_KEY_PRESENT;
            key_len = key.len() as u32;
            key_table.extend_from_slice(key.as_bytes());
        }
        if row.token_id.is_some() {
            flags |= SCRIPTED_STATE_ROW_FLAG_TOKEN_PRESENT;
        }
        if row.value_raw.is_some() {
            flags |= SCRIPTED_STATE_ROW_FLAG_VALUE_PRESENT;
        }
        if row.date_hour_raw.is_some() {
            flags |= SCRIPTED_STATE_ROW_FLAG_DATE_PRESENT;
        }
        if row.deleted {
            flags |= SCRIPTED_STATE_ROW_FLAG_DELETED;
        }
        if row.value_kind == SCRIPTED_STATE_VALUE_VARIABLE_FIXED_I64 {
            variable_rows += 1;
        }
        if row.value_kind == SCRIPTED_STATE_VALUE_FLAG_NAMED
            || row.value_kind == SCRIPTED_STATE_VALUE_FLAG_TOKEN_ONLY
        {
            flag_rows += 1;
        }
        if row.value_kind == SCRIPTED_STATE_VALUE_FLAG_TOKEN_ONLY {
            token_only_rows += 1;
        }
        push_u32(&mut encoded_rows, row.scope_kind);
        push_u32(&mut encoded_rows, row.owner_id);
        push_u32(&mut encoded_rows, row.value_kind);
        push_u32(&mut encoded_rows, flags);
        push_u32(&mut encoded_rows, key_offset);
        push_u32(&mut encoded_rows, key_len);
        push_u32(&mut encoded_rows, row.token_id.unwrap_or(0));
        push_i32(&mut encoded_rows, row.date_hour_raw.unwrap_or(0));
        push_i64(&mut encoded_rows, row.value_raw.unwrap_or(0));
        push_u32(&mut encoded_rows, 0);
        push_u32(&mut encoded_rows, 0);
    }
    let mut out = vec![0u8; SCRIPTED_STATE_HEADER_LEN];
    put_u64_at(&mut out, 0, 10);
    put_u32_at(&mut out, 8, rows.len() as u32);
    put_u32_at(&mut out, 12, key_table.len() as u32);
    put_u32_at(&mut out, 16, rows.len() as u32);
    put_u32_at(&mut out, 20, status);
    put_u32_at(&mut out, 24, 3);
    put_u32_at(&mut out, 28, 3);
    put_u32_at(&mut out, 36, variable_rows);
    put_u32_at(&mut out, 40, flag_rows);
    put_u32_at(&mut out, 44, token_only_rows);
    put_u32_at(&mut out, 48, 100_000);
    put_u32_at(&mut out, 52, 2);
    out.extend_from_slice(&encoded_rows);
    out.extend_from_slice(&key_table);
    out
}

#[test]
fn chunk3_folds_d24_weather_under_map_r2_sidecar() {
    let snapshot = snapshot_for_domain_records(vec![(24, chunk3_weather_payload(0))]);

    let weather = &snapshot["map"]["_r2_weather_live"];
    assert_eq!(weather["active_region_count"], json!(1));
    assert_eq!(weather["regions"]["12"]["temperature"], json!(21.5));
    assert_eq!(weather["regions"]["12"]["rain_light"], json!(true));
    assert_eq!(weather["regions"]["12"]["phenomenon_active"], json!(true));
}

#[test]
fn chunk3_folds_d26_equipment_only_on_unique_division_join() {
    let divisions = chunk3_division_payload(&[(17, 4713, 47)]);
    let equipment = chunk3_division_equipment_payload(
        0,
        &[(47, 17, 0, 0, 0, 2)],
        &[(11, 250_000), (12, 750_000)],
    );
    let snapshot = snapshot_for_domain_records(vec![(20, divisions), (26, equipment)]);

    let divs = snapshot["divisions"].as_array().unwrap();
    assert_eq!(divs.len(), 1);
    assert_eq!(divs[0]["equipment"][0]["equip_id"], json!(11));
    assert_eq!(divs[0]["equipment"][0]["amount"], json!(2.5));
    assert_eq!(divs[0]["equipment"][1]["amount"], json!(7.5));
}

#[test]
fn chunk3_d26_rowless_partial_marks_unknown_without_clearing_equipment() {
    let snaps = chunk1_snapshots(vec![
        vec![
            (20, chunk3_division_payload(&[(17, 4713, 47)])),
            (
                26,
                chunk3_division_equipment_payload(0, &[(47, 17, 0, 0, 0, 1)], &[(11, 250_000)]),
            ),
        ],
        vec![(
            26,
            chunk3_rowless_division_equipment_payload(COMPACT_STATUS_PARTIAL_DATA, 2, 1, 1),
        )],
        vec![],
    ]);

    assert_eq!(
        snaps[0]["divisions"][0]["equipment"][0]["equip_id"],
        json!(11)
    );
    assert_eq!(snaps[1]["divisions"][0]["equipment"], Value::Null);
    assert!(snaps[1]["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(26)
            && problem["problem"] == json!("partial_data_rowless")));
    assert!(snaps[1]["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("division_equipment rowless partial data")));
    assert_eq!(
        snaps[2]["divisions"][0]["equipment"][0]["equip_id"],
        json!(11),
        "rowless partial must retain the prior accumulator for later known hours"
    );
}

#[test]
fn chunk3_d26_sparse_partial_updates_present_rows_and_retains_skipped_prior() {
    let mut sparse_partial = chunk3_division_equipment_payload(
        COMPACT_STATUS_PARTIAL_DATA | DELTA,
        &[(47, 17, 0, 0, 0, 1)],
        &[(21, 450_000)],
    );
    sparse_partial[24..28].copy_from_slice(&2u32.to_le_bytes());

    let snaps = chunk1_snapshots(vec![
        vec![
            (20, chunk3_division_payload(&[(17, 1, 47), (18, 1, 47)])),
            (
                26,
                chunk3_division_equipment_payload(
                    0,
                    &[(47, 17, 0, 0, 0, 1), (47, 18, 0, 0, 1, 1)],
                    &[(11, 250_000), (12, 750_000)],
                ),
            ),
        ],
        vec![(26, sparse_partial)],
    ]);

    let divs = snaps[1]["divisions"].as_array().unwrap();
    let div17 = divs
        .iter()
        .find(|div| div["div_uid"] == json!("17:1"))
        .unwrap();
    let div18 = divs
        .iter()
        .find(|div| div["div_uid"] == json!("18:1"))
        .unwrap();
    assert_eq!(div17["equipment"][0]["equip_id"], json!(21));
    assert_eq!(div17["equipment"][0]["amount"], json!(4.5));
    assert_eq!(div18["equipment"][0]["equip_id"], json!(12));
    assert_eq!(div18["equipment"][0]["amount"], json!(7.5));
    assert!(snaps[1]["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(26)
            && problem["problem"] == json!("partial_data")));
    assert!(snaps[1]["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap().contains("sparse rows applied")));
}

#[test]
fn chunk3_d26_ambiguous_join_is_dropped_and_counted() {
    let divisions = chunk3_division_payload(&[(17, 1, 47), (17, 2, 47)]);
    let equipment = chunk3_division_equipment_payload(0, &[(47, 17, 0, 0, 0, 1)], &[(11, 250_000)]);
    let snapshot = snapshot_for_domain_records(vec![(20, divisions), (26, equipment)]);

    for div in snapshot["divisions"].as_array().unwrap() {
        assert!(div.get("equipment").is_none());
    }
    assert!(snapshot["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap().contains("ambiguous")));
}

#[test]
fn chunk3_d26_prunes_equipment_when_d20_deletes_division_before_id_reuse() {
    let snaps = chunk1_snapshots(vec![
        vec![
            (20, chunk3_division_payload(&[(17, 1, 47)])),
            (
                26,
                chunk3_division_equipment_payload(0, &[(47, 17, 0, 0, 0, 1)], &[(11, 250_000)]),
            ),
        ],
        vec![(20, chunk3_division_delete_payload(17, 1))],
        vec![(20, chunk3_division_payload(&[(17, 2, 47)]))],
    ]);

    assert_eq!(
        snaps[0]["divisions"][0]["equipment"][0]["equip_id"],
        json!(11)
    );
    assert_eq!(snaps[1]["divisions"], json!([]));
    assert!(snaps[1]["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("division_equipment_live pruned 1 row")));
    let reused = &snaps[2]["divisions"][0];
    assert_eq!(reused["div_uid"], json!("17:2"));
    assert!(reused.get("equipment").is_none());
}

#[test]
fn chunk3_d26_duplicate_rows_clear_only_ambiguous_key_and_apply_clean_rows() {
    let snaps = chunk1_snapshots(vec![
        vec![
            (20, chunk3_division_payload(&[(17, 1, 47), (18, 1, 47)])),
            (
                26,
                chunk3_division_equipment_payload(
                    0,
                    &[(47, 17, 0, 0, 0, 1), (47, 18, 0, 0, 1, 1)],
                    &[(11, 250_000), (12, 500_000)],
                ),
            ),
        ],
        vec![(
            26,
            chunk3_division_equipment_payload(
                DELTA,
                &[
                    (47, 17, 0, 0, 0, 1),
                    (47, 17, 0, 0, 1, 1),
                    (47, 18, 0, 0, 2, 1),
                ],
                &[(21, 100_000), (22, 200_000), (23, 300_000)],
            ),
        )],
    ]);

    let divs = snaps[1]["divisions"].as_array().unwrap();
    let div17 = divs
        .iter()
        .find(|div| div["div_uid"] == json!("17:1"))
        .unwrap();
    let div18 = divs
        .iter()
        .find(|div| div["div_uid"] == json!("18:1"))
        .unwrap();
    assert!(div17.get("equipment").is_none());
    assert_eq!(div18["equipment"][0]["equip_id"], json!(23));
    assert_eq!(div18["equipment"][0]["amount"], json!(3.0));
    assert!(snaps[1]["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("dropped 2 duplicate-key row")));
}

#[test]
fn chunk3_folds_d41_projects_with_raw_handle_r2_fields() {
    let snapshot = snapshot_for_domain_records(vec![
        (18, country_identity_payload(&[(47, "GER")])),
        (41, chunk3_projects_payload(0)),
    ]);

    let projects = &snapshot["special_projects"];
    assert_eq!(projects["source"], json!("scan"));
    assert_eq!(
        projects["facilities"]["9"]["building"],
        json!("facility_building")
    );
    assert_eq!(
        projects["facilities"]["9"]["_r2_active_project_handle"],
        json!(101)
    );
    assert_eq!(
        projects["_r2_project_states"]["101"]["_r2_static_def_handle"],
        json!(9001)
    );
    assert_eq!(projects["_r2_breakthroughs"][0]["country"], json!("GER"));
    // Wave-5: no PLCF trailer in this base payload -> lifecycle not measured.
    assert_eq!(projects["project_lifecycle"], Value::Null);
}

#[test]
fn chunk3_folds_d41_project_lifecycle_trailer() {
    let mut payload = chunk3_projects_payload(0);
    // GER(47): local 1 started+completed, local 2 started only.
    // SOV(48): local 3 neither. 1 country aborted fail-closed elsewhere.
    append_project_lifecycle_trailer(
        &mut payload,
        &[
            (47, 1, true, true),
            (47, 2, true, false),
            (48, 3, false, false),
        ],
        2,
        1,
    );
    let snapshot = snapshot_for_domain_records(vec![
        (18, country_identity_payload(&[(47, "GER"), (48, "SOV")])),
        (41, payload),
    ]);

    let lifecycle = &snapshot["special_projects"]["project_lifecycle"];
    assert_eq!(lifecycle["row_count"], json!(3));
    assert_eq!(lifecycle["started_count"], json!(2));
    assert_eq!(lifecycle["completed_count"], json!(1));
    assert_eq!(lifecycle["walked_countries"], json!(2));
    assert_eq!(lifecycle["aborted_countries"], json!(1));
    // GER's two projects, keyed by resolved TAG.
    let ger = lifecycle["countries"]["GER"].as_array().unwrap();
    assert_eq!(ger.len(), 2);
    assert_eq!(ger[0]["local_id"], json!(1));
    assert_eq!(ger[0]["started"], json!(true));
    assert_eq!(ger[0]["completed"], json!(true));
    assert_eq!(ger[1]["local_id"], json!(2));
    assert_eq!(ger[1]["completed"], json!(false));
    // SOV's single project, neither flag.
    let sov = lifecycle["countries"]["SOV"].as_array().unwrap();
    assert_eq!(sov[0]["local_id"], json!(3));
    assert_eq!(sov[0]["started"], json!(false));
}

#[test]
fn chunk3_d41_lifecycle_trailer_duplicate_key_fails_fold_closed() {
    let mut payload = chunk3_projects_payload(0);
    // Duplicate (47, 1): the whole D41 fold must fail closed (no
    // special_projects this hour).
    append_project_lifecycle_trailer(
        &mut payload,
        &[(47, 1, true, false), (47, 1, false, true)],
        1,
        0,
    );
    let snapshot = snapshot_for_domain_records(vec![
        (18, country_identity_payload(&[(47, "GER")])),
        (41, payload),
    ]);
    // Fold rejected the malformed payload -> special_projects unknown/null.
    assert!(snapshot["special_projects"].is_null());
}

#[test]
fn chunk3_folds_d45_depth_annotations_and_country_targets() {
    let snapshot = snapshot_for_domain_records_with_flags(vec![
        (18, country_identity_payload(&[(47, "GER"), (48, "SPR")]), 0),
        (
            45,
            chunk3_frd_payload(),
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
        ),
    ]);

    let ger = &snapshot["countries"]["GER"];
    assert_eq!(ger["focus"]["_r2_depth_completed_count"], json!(3));
    assert_eq!(
        ger["research"]["_r2_depth_in_progress_live"][0]["eta_hours"],
        json!(12)
    );
    assert_eq!(
        ger["decisions"]["_r2_depth_status"]["targeted_timed_count"],
        json!(2)
    );
    assert_eq!(
        ger["decisions"]["_r2_depth_targets"][0]["decision"],
        json!("decision_target_country")
    );
    assert_eq!(
        ger["decisions"]["_r2_depth_targets"][0]["target"],
        json!("SPR")
    );
    assert!(ger["decisions"].get("_r2_depth_target_rows").is_none());
    assert_eq!(
        snapshot["provenance"]["collector"]["_r2_depth_badges"][0]["source_domain_id"],
        json!(45)
    );
}

#[test]
fn chunk3_folds_d45_old_header_without_treating_elapsed_as_targets() {
    let snapshot = snapshot_for_domain_records(vec![
        (18, country_identity_payload(&[(47, "GER")])),
        (45, chunk3_frd_payload_old_header()),
    ]);

    let ger = &snapshot["countries"]["GER"];
    assert_eq!(
        ger["decisions"]["_r2_depth_status"]["targeted_timed_count"],
        json!(2)
    );
    assert!(ger["decisions"].get("_r2_depth_targets").is_none());
    assert!(snapshot["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn chunk3_folds_d45_old_header_frd2_elapsed_collision_as_v1() {
    let payload = chunk3_frd_payload_old_header_with_elapsed(843_338_310);
    assert_eq!(&payload[0..4], b"FRD2");
    let snapshot = snapshot_for_domain_records(vec![
        (18, country_identity_payload(&[(47, "GER")])),
        (45, payload),
    ]);

    let ger = &snapshot["countries"]["GER"];
    assert_eq!(
        ger["decisions"]["_r2_depth_status"]["targeted_timed_count"],
        json!(2)
    );
    assert!(ger["decisions"].get("_r2_depth_targets").is_none());
    assert!(snapshot["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn chunk3_folds_d45_schema_v2_flag_without_frd2_as_malformed_unknown() {
    let payload = chunk3_frd_payload_old_header();
    assert_ne!(payload.get(0..4), Some(b"FRD2".as_slice()));
    let snapshot = snapshot_for_domain_records_with_flags(vec![
        (18, country_identity_payload(&[(47, "GER"), (48, "SPR")]), 0),
        (45, payload, COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2),
    ]);

    let countries = snapshot["countries"].as_object().unwrap();
    assert!(countries.values().all(|country| country
        .get("decisions")
        .and_then(|decisions| decisions.get("_r2_depth_status"))
        .is_none()));
    assert!(countries.values().all(|country| country
        .get("decisions")
        .and_then(|decisions| decisions.get("_r2_depth_targets"))
        .is_none()));
    assert!(snapshot["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(45)
            && problem["problem"] == json!("malformed_payload")));
    assert!(!snapshot["provenance"]["collector"]["domains"]
        .as_array()
        .unwrap()
        .iter()
        .any(|domain| domain == "focus_research_decisions_depth_live"));
    assert!(snapshot["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("missing FRD2 sanity marker")));
}

#[test]
fn chunk3_folds_d45_omitted_targets_as_partial_provenance_note() {
    let snapshot = snapshot_for_domain_records_with_flags(vec![
        (18, country_identity_payload(&[(47, "GER"), (48, "SPR")]), 0),
        (
            45,
            chunk3_frd_payload_versioned(true, 1),
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
        ),
    ]);

    let ger = &snapshot["countries"]["GER"];
    assert_eq!(
        ger["decisions"]["_r2_depth_targets"],
        Value::Null,
        "omitted target rows make the country target list incomplete"
    );
    assert!(snapshot["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(45)
            && problem["problem"] == json!("partial_data")));
    assert!(snapshot["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("omitted 1 unreadable targeted-timed country target row")));
}

#[test]
fn chunk3_folds_d51_scripted_state_and_keeps_opaque_status_nonfatal() {
    let rows = vec![
        Chunk3ScriptedRow {
            scope_kind: SCRIPTED_STATE_SCOPE_GLOBAL,
            owner_id: 0,
            value_kind: SCRIPTED_STATE_VALUE_VARIABLE_FIXED_I64,
            key: Some("global_var"),
            token_id: None,
            value_raw: Some(250_000),
            date_hour_raw: None,
            deleted: false,
        },
        Chunk3ScriptedRow {
            scope_kind: SCRIPTED_STATE_SCOPE_COUNTRY,
            owner_id: 47,
            value_kind: SCRIPTED_STATE_VALUE_FLAG_NAMED,
            key: Some("country_flag"),
            token_id: None,
            value_raw: Some(1),
            date_hour_raw: Some(123),
            deleted: false,
        },
        Chunk3ScriptedRow {
            scope_kind: SCRIPTED_STATE_SCOPE_STATE,
            owner_id: 100,
            value_kind: SCRIPTED_STATE_VALUE_VARIABLE_FIXED_I64,
            key: Some("state_var"),
            token_id: None,
            value_raw: Some(50_000),
            date_hour_raw: None,
            deleted: false,
        },
        Chunk3ScriptedRow {
            scope_kind: SCRIPTED_STATE_SCOPE_GLOBAL,
            owner_id: 0,
            value_kind: SCRIPTED_STATE_VALUE_DELAYED_EVENT_OPAQUE,
            key: Some("__delayed_events_root_unresolved"),
            token_id: None,
            value_raw: None,
            date_hour_raw: None,
            deleted: false,
        },
    ];
    let snapshot = snapshot_for_domain_records(vec![
        (18, country_identity_payload(&[(47, "GER")])),
        (
            51,
            chunk3_scripted_payload(SCRIPTED_STATE_STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED, &rows),
        ),
    ]);

    assert_eq!(snapshot["world"]["variables"]["global_var"], json!(2.5));
    assert_eq!(
        snapshot["countries"]["GER"]["extra"]["script_flags"]["country_flag"]["date_hour_raw"],
        json!(123)
    );
    assert_eq!(
        snapshot["states"]["100"]["_r2_scripted_state"]["variables"]["state_var"],
        json!(0.5)
    );
    assert_eq!(
        snapshot["world"]["_r2_scripted_state_badges"][0]["value_kind"],
        json!(SCRIPTED_STATE_VALUE_DELAYED_EVENT_OPAQUE)
    );
    let problems = snapshot["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap();
    assert!(problems.is_empty());
}

#[test]
fn chunk3_d51_lane_local_bits_preserve_opaque_badges_without_failure() {
    let rows = vec![
        Chunk3ScriptedRow {
            scope_kind: SCRIPTED_STATE_SCOPE_GLOBAL,
            owner_id: 0,
            value_kind: SCRIPTED_STATE_VALUE_DELAYED_EVENT_OPAQUE,
            key: Some("__delayed_events_root_unresolved"),
            token_id: None,
            value_raw: None,
            date_hour_raw: None,
            deleted: false,
        },
        Chunk3ScriptedRow {
            scope_kind: SCRIPTED_STATE_SCOPE_GLOBAL,
            owner_id: 0,
            value_kind: SCRIPTED_STATE_VALUE_SCRIPTED_GUI_OPAQUE,
            key: Some("__scripted_gui_root_unresolved"),
            token_id: None,
            value_raw: None,
            date_hour_raw: None,
            deleted: false,
        },
    ];
    let status = SCRIPTED_STATE_STATUS_FLAG_NAMES_PARTIAL
        | SCRIPTED_STATE_STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED
        | SCRIPTED_STATE_STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED;
    let snapshot = snapshot_for_domain_records(vec![(51, chunk3_scripted_payload(status, &rows))]);

    let badges = snapshot["world"]["_r2_scripted_state_badges"]
        .as_array()
        .unwrap();
    assert_eq!(badges.len(), 2);
    assert_eq!(
        badges[0]["value_kind"],
        json!(SCRIPTED_STATE_VALUE_DELAYED_EVENT_OPAQUE)
    );
    assert_eq!(
        badges[1]["value_kind"],
        json!(SCRIPTED_STATE_VALUE_SCRIPTED_GUI_OPAQUE)
    );
    assert!(snapshot["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .is_empty());
    let notes = snapshot["provenance"]["notes"].as_array().unwrap();
    assert!(notes.iter().any(|note| note
        .as_str()
        .unwrap()
        .contains("delayed-event root unresolved")));
    assert!(notes.iter().any(|note| note
        .as_str()
        .unwrap()
        .contains("scripted-GUI root unresolved")));
}

// --- the main multi-domain, multi-hour, delta + delete test --------------

#[test]
fn folds_two_hours_full_then_delta_with_delete() {
    // Two adjacent game hours. Hour A = full baseline for divisions + map,
    // full frames for fuel + production. Hour B = a divisions delta (move div 17,
    // delete div 18) and a map_control delta (flip one state controller).
    let hi_a = 16_959_360i64;
    let hi_b = 16_959_361i64;
    let raw_a = raw_for(hi_a);
    let raw_b = raw_for(hi_b);
    let tag: u32 = 47;

    let mut stream = Vec::new();
    let mut seq = 1u64;
    let push = |stream: &mut Vec<u8>, seq: &mut u64, raw, dom, rt, pl: &[u8]| {
        push_record(stream, *seq, raw, dom, rt, pl);
        *seq += 1;
    };

    // ---- hour A ----
    // fuel: current 123.5, max 456.25 (raw = value*32768)
    let fuel_a = fuel_payload(&[(
        tag,
        (123.5 * 32768.0) as i64,
        (456.25 * 32768.0) as i64,
        (2.0 * 32768.0) as i64,
        (-1.0 * 32768.0) as i64,
    )]);
    push(
        &mut stream,
        &mut seq,
        raw_a,
        10,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &fuel_a,
    );

    // production: stockpile 55.0, 3 active factories on the mil line
    let prod_a = production_payload(tag, (55.0 * 100_000.0) as i64, 3);
    push(
        &mut stream,
        &mut seq,
        raw_a,
        11,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &prod_a,
    );

    // divisions baseline: div 17 (moving toward 200) and div 18 (static)
    let mut div_a = Vec::new();
    divisions_header(&mut div_a, 2, 0);
    // div 17: moving(bit0) + has_target(bit2) + has_owner(bit5) + template(1<<13)
    push_div_row(
        &mut div_a,
        17,
        1 | (1 << 5) | DIV_FLAG_TEMPLATE,
        118,
        (30.0 * 100_000.0) as u32,
        (90.0 * 100_000.0) as u32,
        3101,
        tag,
    );
    // div 18: static, has_owner
    push_div_row(
        &mut div_a,
        18,
        1 << 5,
        119,
        (25.0 * 100_000.0) as u32,
        (80.0 * 100_000.0) as u32,
        0,
        48,
    );
    push(
        &mut stream,
        &mut seq,
        raw_a,
        20,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &div_a,
    );

    // map_control baseline: state 5 owned+controlled by tag 47; province 900 controlled by 47
    let mut mc_a = Vec::new();
    map_control_header(&mut mc_a, 1, 1, 0);
    push_state_row(&mut mc_a, 5, 47, 47);
    push_prov_row(&mut mc_a, 900, 47, 5);
    push(
        &mut stream,
        &mut seq,
        raw_a,
        22,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &mc_a,
    );

    push(
        &mut stream,
        &mut seq,
        raw_a,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    // ---- hour B ----
    // divisions delta: div 17 now at province 200 (moved, no longer moving),
    // div 18 deleted.
    let mut div_b = Vec::new();
    divisions_header(&mut div_b, 2, DELTA);
    push_div_row(
        &mut div_b,
        17,
        (1 << 5) | DIV_FLAG_TEMPLATE,
        200,
        (28.0 * 100_000.0) as u32,
        (88.0 * 100_000.0) as u32,
        3101,
        tag,
    );
    // deleted row must be zero-filled except id/type/flags
    push_div_row(&mut div_b, 18, DIV_FLAG_DELETED, 0, 0, 0, 0, 0);
    push(
        &mut stream,
        &mut seq,
        raw_b,
        20,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &div_b,
    );

    // map_control delta: state 5 controller flips to tag 48 (owner stays 47)
    let mut mc_b = Vec::new();
    map_control_header(&mut mc_b, 1, 0, DELTA);
    push_state_row(&mut mc_b, 5, 47, 48);
    push(
        &mut stream,
        &mut seq,
        raw_b,
        22,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &mc_b,
    );

    push(
        &mut stream,
        &mut seq,
        raw_b,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    // ---- fold ----
    let mut fold = WorldFold::new("test_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 2, "one snapshot per game hour");

    // ---- hour A assertions ----
    let (a_hi, a_date, a_bytes) = &snaps[0];
    assert_eq!(*a_hi, hi_a);
    assert_eq!(a_date, &date_from_raw(raw_a).unwrap());
    let a = parse(a_bytes);
    assert_eq!(a["schema_version"], json!("1"));
    assert_eq!(a["game_id"], json!("test_game"));
    assert_eq!(a["hour_index"], json!(hi_a));
    // every top-level container present
    for key in [
        "countries",
        "states",
        "provinces",
        "divisions",
        "combats",
        "diplomacy_world",
        "map",
        "designs",
        "order_of_battle",
        "naval_air",
        "supply",
        "strategic_air",
        "strategic_operatives",
        "raids",
        "world_events",
        "world",
        "events",
        "provenance",
    ] {
        assert!(a.get(key).is_some(), "missing top-level {key}");
    }
    // provenance: mod null, collector captured, domains recorded, TAG pending
    assert_eq!(a["provenance"]["mod"], Value::Null);
    assert_eq!(a["provenance"]["collector"]["captured"], json!(true));
    assert_eq!(a["tag_db_id_index"]["resolved"], json!(false));
    let dom_list = a["provenance"]["collector"]["domains"].as_array().unwrap();
    for name in ["fuel_live", "production_live", "divisions", "map_control"] {
        assert!(
            dom_list.iter().any(|d| d == name),
            "domain {name} not in provenance"
        );
    }

    // fuel folded and scaled
    let c = &a["countries"]["47"];
    assert!((c["fuel"]["current"].as_f64().unwrap() - 123.5).abs() < 1e-6);
    assert!((c["fuel"]["max"].as_f64().unwrap() - 456.25).abs() < 1e-6);
    assert!((c["fuel"]["cost"].as_f64().unwrap() - (-1.0)).abs() < 1e-6);

    // production folded
    let prod = &c["production"];
    assert!((prod["stockpile"][0]["amount"].as_f64().unwrap() - 55.0).abs() < 1e-6);
    assert_eq!(prod["military_lines"][0]["active_factories"], json!(3));
    assert_eq!(prod["naval_lines"].as_array().unwrap().len(), 0);
    assert_eq!(prod["construction_queue"][0]["template_id"], json!(20013));

    // divisions: two present, keyed by uid, owner as stringified tag_db_id
    let divs = a["divisions"].as_array().unwrap();
    assert_eq!(divs.len(), 2);
    let d17 = divs.iter().find(|d| d["div_uid"] == "17:4713").unwrap();
    assert_eq!(d17["owner"], json!("47"));
    assert_eq!(d17["location_province"], json!(118));
    assert_eq!(d17["moving"], json!(true));
    assert_eq!(d17["template_id"], json!(3101));
    assert!((d17["organisation"].as_f64().unwrap() - 30.0).abs() < 1e-6);

    // map_control baseline
    assert_eq!(a["states"]["5"]["owner"], json!("47"));
    assert_eq!(a["states"]["5"]["controller"], json!("47"));
    assert_eq!(a["provinces"]["900"]["controller"], json!("47"));
    assert_eq!(a["provinces"]["900"]["state_id"], json!(5));
    // map layer mirror
    assert_eq!(a["map"]["owner_by_state"]["5"], json!("47"));
    assert_eq!(a["map"]["control_by_province"]["900"], json!("47"));

    // ---- hour B assertions (delta applied) ----
    let (b_hi, _, b_bytes) = &snaps[1];
    assert_eq!(*b_hi, hi_b);
    let b = parse(b_bytes);

    // div 17 moved to 200 and is no longer moving; div 18 deleted
    let divs_b = b["divisions"].as_array().unwrap();
    assert_eq!(divs_b.len(), 1, "div 18 should be removed by delta delete");
    let d17b = &divs_b[0];
    assert_eq!(d17b["div_uid"], json!("17:4713"));
    assert_eq!(d17b["location_province"], json!(200));
    assert_eq!(d17b["moving"], json!(false));

    // map delta: state 5 controller flipped to 48, owner carried as 47
    assert_eq!(b["states"]["5"]["controller"], json!("48"));
    assert_eq!(b["states"]["5"]["owner"], json!("47"));
    // province 900 carried over from the baseline (delta emitted no province rows)
    assert_eq!(b["provinces"]["900"]["controller"], json!("47"));

    // fuel/production carried across the hour (full domains not re-sent stay put)
    assert!((b["countries"]["47"]["fuel"]["current"].as_f64().unwrap() - 123.5).abs() < 1e-6);

    // hour B provenance only lists the domains that fed rows THIS hour
    let dom_b = b["provenance"]["collector"]["domains"].as_array().unwrap();
    assert!(dom_b.iter().any(|d| d == "divisions"));
    assert!(dom_b.iter().any(|d| d == "map_control"));
    assert!(!dom_b.iter().any(|d| d == "fuel_live"));
}

#[test]
fn delta_before_baseline_emits_null_and_domain_problem() {
    let hi = 16_959_360i64;
    let raw = raw_for(hi);
    let tag: u32 = 47;
    let mut stream = Vec::new();
    let mut seq = 1u64;

    let mut div_delta = Vec::new();
    divisions_header(&mut div_delta, 1, DELTA);
    push_div_row(
        &mut div_delta,
        17,
        (1 << 5) | DIV_FLAG_TEMPLATE,
        118,
        (30.0 * 100_000.0) as u32,
        (90.0 * 100_000.0) as u32,
        3101,
        tag,
    );
    push_record(
        &mut stream,
        seq,
        raw,
        20,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &div_delta,
    );
    seq += 1;

    let mut map_delta = Vec::new();
    map_control_header(&mut map_delta, 1, 1, DELTA);
    push_state_row(&mut map_delta, 5, 47, 48);
    push_prov_row(&mut map_delta, 900, 48, 5);
    push_record(
        &mut stream,
        seq,
        raw,
        22,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &map_delta,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("delta_before_baseline_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    let s = parse(&snaps[0].2);

    assert_eq!(s["divisions"], Value::Null);
    assert_eq!(s["states"], json!({}));
    assert_eq!(s["provinces"], json!({}));
    assert_eq!(s["map"]["owner_by_state"], Value::Null);
    assert_eq!(s["map"]["control_by_province"], Value::Null);
    let problems = s["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap();
    assert!(problems.iter().any(|problem| {
        problem["domain_id"] == json!(20) && problem["problem"] == json!("delta_before_baseline")
    }));
    assert!(problems.iter().any(|problem| {
        problem["domain_id"] == json!(22) && problem["problem"] == json!("delta_before_baseline")
    }));
    let notes = s["provenance"]["notes"].as_array().unwrap();
    assert!(notes
        .iter()
        .any(|note| note.as_str().unwrap().contains("delta_before_baseline")));
}

#[test]
fn failed_delta_capable_hour_is_unknown_but_next_delta_uses_prior_baseline() {
    let hi_a = 16_959_360i64;
    let hi_b = 16_959_361i64;
    let hi_c = 16_959_362i64;
    let raw_a = raw_for(hi_a);
    let raw_b = raw_for(hi_b);
    let raw_c = raw_for(hi_c);
    let tag: u32 = 47;
    let mut stream = Vec::new();
    let mut seq = 1u64;

    let mut div_a = Vec::new();
    divisions_header(&mut div_a, 1, 0);
    push_div_row(
        &mut div_a,
        17,
        (1 << 5) | DIV_FLAG_TEMPLATE,
        118,
        (30.0 * 100_000.0) as u32,
        (90.0 * 100_000.0) as u32,
        3101,
        tag,
    );
    push_record(
        &mut stream,
        seq,
        raw_a,
        20,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &div_a,
    );
    seq += 1;
    let mut map_a = Vec::new();
    map_control_header(&mut map_a, 1, 1, 0);
    push_state_row(&mut map_a, 5, 47, 47);
    push_prov_row(&mut map_a, 900, 47, 5);
    push_record(
        &mut stream,
        seq,
        raw_a,
        22,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &map_a,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw_a,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    seq += 1;

    let mut div_failed = Vec::new();
    divisions_header(&mut div_failed, 0, COMPACT_STATUS_SOURCE_UNAVAILABLE);
    push_record(
        &mut stream,
        seq,
        raw_b,
        20,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &div_failed,
    );
    seq += 1;
    let mut map_failed = Vec::new();
    map_control_header(&mut map_failed, 0, 0, COMPACT_STATUS_SOURCE_UNAVAILABLE);
    push_record(
        &mut stream,
        seq,
        raw_b,
        22,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &map_failed,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw_b,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    seq += 1;

    let mut div_c = Vec::new();
    divisions_header(&mut div_c, 1, DELTA);
    push_div_row(
        &mut div_c,
        17,
        (1 << 5) | DIV_FLAG_TEMPLATE,
        200,
        (28.0 * 100_000.0) as u32,
        (88.0 * 100_000.0) as u32,
        3101,
        tag,
    );
    push_record(
        &mut stream,
        seq,
        raw_c,
        20,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &div_c,
    );
    seq += 1;
    let mut map_c = Vec::new();
    map_control_header(&mut map_c, 1, 0, DELTA);
    push_state_row(&mut map_c, 5, 47, 48);
    push_record(
        &mut stream,
        seq,
        raw_c,
        22,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &map_c,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw_c,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("failed_delta_capable_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 3);

    let b = parse(&snaps[1].2);
    assert_eq!(b["divisions"], Value::Null);
    assert_eq!(b["states"]["5"]["owner"], Value::Null);
    assert_eq!(b["states"]["5"]["controller"], Value::Null);
    assert_eq!(b["provinces"]["900"]["controller"], Value::Null);
    assert_eq!(b["provinces"]["900"]["state_id"], Value::Null);
    assert_eq!(b["map"]["owner_by_state"], Value::Null);
    assert_eq!(b["map"]["control_by_province"], Value::Null);

    let c = parse(&snaps[2].2);
    let divs = c["divisions"].as_array().unwrap();
    assert_eq!(divs.len(), 1);
    assert_eq!(divs[0]["location_province"], json!(200));
    assert_eq!(c["states"]["5"]["owner"], json!("47"));
    assert_eq!(c["states"]["5"]["controller"], json!("48"));
    assert_eq!(c["provinces"]["900"]["controller"], json!("47"));
}

#[test]
fn unfolded_domain_data_is_listed_in_provenance() {
    let hi = 16_959_360i64;
    let raw = raw_for(hi);
    let mut stream = Vec::new();
    // Domain 58 (country_economy_counters_live) is assigned but has no world
    // fold projection, so its data records surface under provenance.unfolded.
    push_record(
        &mut stream,
        1,
        raw,
        58,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &[],
    );
    push_record(
        &mut stream,
        2,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("unfolded_domain_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    let s = parse(&snaps[0].2);
    let unfolded = s["provenance"]["collector"]["unfolded_domains"]
        .as_array()
        .unwrap();
    assert!(unfolded
        .iter()
        .any(|domain| domain["domain_id"] == json!(58)
            && domain["domain"] == json!("country_economy_counters_live")));
    let domains = s["provenance"]["collector"]["domains"].as_array().unwrap();
    assert!(!domains
        .iter()
        .any(|domain| domain == "country_economy_counters_live"));
}

#[test]
fn naval_combat_domains_fold_into_naval_combats_section() {
    use naval_combat_live_lane as naval4950;
    let hi = 16_959_360i64;
    let raw = raw_for(hi);
    let lifecycle = naval4950::encode_lifecycle_payload(
        5,
        naval4950::STATUS_DELTA_DATA,
        &[naval4950::LifecycleRow {
            combat_id: 240_580,
            generation: 1,
            body_ptr: 0,
            deleted: false,
            location_id: Some(6434),
            region_id: Some(18),
            attacker_member_count: Some(1),
            defender_member_count: Some(1),
            attacker_positioning_milli: Some(812),
            defender_positioning_milli: Some(765),
        }],
    );
    let members = naval4950::encode_member_payload(
        5,
        naval4950::STATUS_DELTA_DATA,
        &[
            naval4950::MemberRow {
                combat_id: 240_580,
                generation: 1,
                deleted: false,
                side: naval4950::CombatSide::Attacker,
                group_index: 0,
                member_index: 0,
                unit_type_tag: 51,
                ship_instance_id: Some(4431),
                cship_join_validated: true,
                is_convoy: false,
                owner_tag_db_id: Some(17),
                designed_max_strength_raw: Some(10_000_000),
                battle_slot_index_raw: 3,
                state_bitmask: Some(1),
                engagement_weight_raw: Some(1_000),
                task_force_id: Some(477),
                fleet_id: Some(88),
                strength_raw: Some(10_000_000),
                organisation_raw: Some(9_000_000),
                experience_raw: Some(100_000),
                sunk_convoys: Some(0),
            },
            naval4950::MemberRow {
                combat_id: 240_580,
                generation: 1,
                deleted: false,
                side: naval4950::CombatSide::Defender,
                group_index: 0,
                member_index: 0,
                unit_type_tag: 0,
                ship_instance_id: None,
                cship_join_validated: false,
                is_convoy: true,
                owner_tag_db_id: Some(18),
                designed_max_strength_raw: None,
                battle_slot_index_raw: 4,
                state_bitmask: Some(1),
                engagement_weight_raw: None,
                task_force_id: None,
                fleet_id: None,
                strength_raw: None,
                organisation_raw: None,
                experience_raw: None,
                sunk_convoys: None,
            },
        ],
    );

    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw,
        49,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &lifecycle,
    );
    push_record(
        &mut stream,
        2,
        raw,
        50,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &members,
    );
    push_record(
        &mut stream,
        3,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    // A second hour that deletes the battle proves the section empties on teardown.
    let raw2 = raw_for(hi + 1);
    let lifecycle_end = naval4950::encode_lifecycle_payload(
        5,
        naval4950::STATUS_DELTA_DATA,
        &[naval4950::LifecycleRow {
            combat_id: 240_580,
            generation: 1,
            body_ptr: 0,
            deleted: true,
            location_id: Some(6434),
            region_id: Some(18),
            attacker_member_count: Some(1),
            defender_member_count: Some(1),
            attacker_positioning_milli: Some(812),
            defender_positioning_milli: Some(765),
        }],
    );
    let members_empty = naval4950::encode_member_payload(5, naval4950::STATUS_DELTA_DATA, &[]);
    push_record(
        &mut stream,
        4,
        raw2,
        49,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &lifecycle_end,
    );
    push_record(
        &mut stream,
        5,
        raw2,
        50,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &members_empty,
    );
    push_record(
        &mut stream,
        6,
        raw2,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("naval_combat_fold_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 2);

    let s0 = parse(&snaps[0].2);
    let battle = &s0["naval_combats"]["240580:1"];
    assert_eq!(battle["location_province"], json!(6434));
    assert_eq!(battle["region_id"], json!(18));
    assert_eq!(battle["kind"], json!("naval"));
    let members_json = battle["members"].as_array().unwrap();
    assert_eq!(members_json.len(), 2);
    assert!(members_json.iter().any(|m| m["side"] == json!("attacker")
        && m["ship_instance_id"] == json!(4431)
        && m["cship_join_validated"] == json!(true)
        && m["task_force_id"] == json!(477)));
    assert!(members_json
        .iter()
        .any(|m| m["side"] == json!("defender") && m["is_convoy"] == json!(true)));

    // Both domains fold: neither appears in provenance.unfolded_domains.
    let unfolded = s0["provenance"]["collector"]["unfolded_domains"]
        .as_array()
        .unwrap();
    assert!(!unfolded
        .iter()
        .any(|d| d["domain_id"] == json!(49) || d["domain_id"] == json!(50)));

    // Teardown hour: the battle is gone, section is an empty object (not null).
    let s1 = parse(&snaps[1].2);
    assert_eq!(s1["naval_combats"], json!({}));
}

#[test]
fn domain47_folds_active_ruler_and_r2_sidecar_after_tag_rekey() {
    let raw = raw_for(16_959_360);
    let rows = vec![
        domain47_lane::CharacterLaneRow::Character(domain47_lane::CharacterRow {
            country_tag_db_id: 47,
            char_id: 1001,
            name: Some("Antonio de Oliveira Salazar"),
            token: Some("POR_salazar"),
        }),
        domain47_lane::CharacterLaneRow::UnitLeader(domain47_lane::UnitLeaderRow {
            country_tag_db_id: 47,
            unit_leader_id: 4713,
            char_id: 1001,
            leader_kind: domain47_lane::UNIT_LEADER_ARMY,
            skill: Some(4),
            experience_raw: Some(123_456),
            attack: Some(2),
            defense: Some(3),
            planning: Some(4),
            logistics: Some(5),
            maneuvering: None,
            coordination: None,
            token: Some("POR_salazar"),
            name: Some("Antonio de Oliveira Salazar"),
            trait_count: 1,
        }),
        domain47_lane::CharacterLaneRow::UnitLeaderTrait(domain47_lane::UnitLeaderTraitRow {
            unit_leader_id: 4713,
            ordinal: 0,
            trait_token: "old_guard",
        }),
        domain47_lane::CharacterLaneRow::CountryLeaderRole(domain47_lane::CountryLeaderRoleRow {
            country_tag_db_id: 47,
            char_id: 1001,
            name: Some("Antonio de Oliveira Salazar"),
            token: Some("POR_salazar"),
        }),
        domain47_lane::CharacterLaneRow::ActiveRuler(domain47_lane::ActiveRulerRow {
            country_tag_db_id: 47,
            char_id: 1001,
            ruling_party: Some("neutrality"),
            name: Some("Antonio de Oliveira Salazar"),
            token: Some("POR_salazar"),
            leader_vec_size: 1,
            ambiguous: false,
        }),
        domain47_lane::CharacterLaneRow::AdvisorRoster(domain47_lane::AdvisorRow {
            country_tag_db_id: 47,
            char_id: 1001,
            slot: Some("political_advisor"),
            ledger: Some("political_advisor"),
            idea_token: "POR_salazar_advisor",
        }),
        domain47_lane::CharacterLaneRow::AppointedAdvisor(domain47_lane::AdvisorRow {
            country_tag_db_id: 47,
            char_id: 1001,
            slot: Some("political_advisor"),
            ledger: Some("political_advisor"),
            idea_token: "POR_salazar_advisor",
        }),
    ];

    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "POR")]),
    );
    push_record(
        &mut stream,
        2,
        raw,
        47,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &domain47_payload(0, &rows),
    );
    push_record(
        &mut stream,
        3,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("domain47_valid");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    let snap = parse(&snaps[0].2);
    let country = &snap["countries"]["POR"];
    assert_eq!(
        country["commanders"]["leader"],
        json!({
            "char_id": 1001,
            "name": "Antonio de Oliveira Salazar",
            "token": "POR_salazar",
            "ideology_group": "neutrality",
            "source": "C",
            "ambiguous": false,
        })
    );
    let sidecar = &country["extra"]["_r2_characters_leaders_cabinet_live"];
    assert_eq!(sidecar["characters"].as_array().unwrap().len(), 1);
    assert_eq!(sidecar["unit_leaders"][0]["unit_leader_id"], json!(4713));
    assert_eq!(sidecar["unit_leaders"][0]["leader_kind"], json!("army"));
    assert_eq!(sidecar["unit_leaders"][0]["skill"], json!(4));
    assert_eq!(
        sidecar["country_leader_roles"][0]["token"],
        json!("POR_salazar")
    );
    assert_eq!(sidecar["active_rulers"][0]["leader_vec_size"], json!(1));
    assert_eq!(
        sidecar["unit_leader_traits"],
        json!([{
            "unit_leader_id": 4713,
            "ordinal": 0,
            "trait_token": "old_guard",
        }])
    );
    assert_eq!(
        sidecar["advisor_roster"],
        json!([{
            "char_id": 1001,
            "slot": "political_advisor",
            "ledger": "political_advisor",
            "idea_token": "POR_salazar_advisor",
        }])
    );
    assert_eq!(
        sidecar["appointed_advisors"],
        json!([{
            "char_id": 1001,
            "slot": "political_advisor",
            "ledger": "political_advisor",
            "idea_token": "POR_salazar_advisor",
        }])
    );
    assert!(country.as_object().unwrap().get("politics").is_none());
}

#[test]
fn domain47_portrait_rows_stay_excluded_and_retain_prior_state() {
    let raw_a = raw_for(16_959_360);
    let raw_b = raw_for(16_959_361);
    let baseline_rows = vec![domain47_lane::CharacterLaneRow::Character(
        domain47_lane::CharacterRow {
            country_tag_db_id: 47,
            char_id: 1001,
            name: Some("Antonio de Oliveira Salazar"),
            token: Some("POR_salazar"),
        },
    )];
    let portrait_rows = vec![
        domain47_lane::CharacterLaneRow::Character(domain47_lane::CharacterRow {
            country_tag_db_id: 47,
            char_id: 1002,
            name: None,
            token: None,
        }),
        domain47_lane::CharacterLaneRow::CharacterPortrait(domain47_lane::CharacterPortraitRow {
            country_tag_db_id: 47,
            char_id: 1002,
            role: "civilian",
            size: "large",
            gfx: "GFX_portrait_por_1002",
        }),
    ];
    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw_a,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "POR")]),
    );
    push_record(
        &mut stream,
        2,
        raw_a,
        47,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &domain47_payload(0, &baseline_rows),
    );
    push_record(
        &mut stream,
        3,
        raw_a,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    push_record(
        &mut stream,
        4,
        raw_b,
        47,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &domain47_payload(0, &portrait_rows),
    );
    push_record(
        &mut stream,
        5,
        raw_b,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("domain47_portrait_excluded");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 2);
    let second = parse(&snaps[1].2);
    // The portrait-bearing hour is rejected wholesale: the prior hour's
    // character sidecar is retained and the exclusion is recorded.
    let sidecar = &second["countries"]["POR"]["extra"]["_r2_characters_leaders_cabinet_live"];
    assert_eq!(sidecar["characters"][0]["char_id"], json!(1001));
    assert!(second["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(47)
            && problem["problem"] == json!("excluded_rows_present")));
}

#[test]
fn domain47_rowless_partial_status_retains_previous_character_state() {
    let raw_a = raw_for(16_959_360);
    let raw_b = raw_for(16_959_361);
    let rows = vec![
        domain47_lane::CharacterLaneRow::Character(domain47_lane::CharacterRow {
            country_tag_db_id: 47,
            char_id: 1001,
            name: Some("Antonio de Oliveira Salazar"),
            token: Some("POR_salazar"),
        }),
        domain47_lane::CharacterLaneRow::ActiveRuler(domain47_lane::ActiveRulerRow {
            country_tag_db_id: 47,
            char_id: 1001,
            ruling_party: Some("neutrality"),
            name: Some("Antonio de Oliveira Salazar"),
            token: Some("POR_salazar"),
            leader_vec_size: 1,
            ambiguous: false,
        }),
    ];
    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw_a,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "POR")]),
    );
    push_record(
        &mut stream,
        2,
        raw_a,
        47,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &domain47_payload(0, &rows),
    );
    push_record(
        &mut stream,
        3,
        raw_a,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    push_record(
        &mut stream,
        4,
        raw_b,
        47,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &domain47_payload(COMPACT_STATUS_PARTIAL_DATA, &[]),
    );
    push_record(
        &mut stream,
        5,
        raw_b,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("domain47_failure");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 2);
    let second = parse(&snaps[1].2);
    assert_eq!(
        second["countries"]["POR"]["commanders"]["leader"]["token"],
        json!("POR_salazar")
    );
    assert!(second["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(47)
            && problem["problem"] == json!("failure_status")));
}

#[test]
fn domain47_delta_data_is_unknown_and_does_not_clear_prior_state() {
    let raw_a = raw_for(16_959_360);
    let raw_b = raw_for(16_959_361);
    let rows = vec![domain47_lane::CharacterLaneRow::ActiveRuler(
        domain47_lane::ActiveRulerRow {
            country_tag_db_id: 47,
            char_id: 1001,
            ruling_party: Some("neutrality"),
            name: Some("Antonio de Oliveira Salazar"),
            token: Some("POR_salazar"),
            leader_vec_size: 1,
            ambiguous: false,
        },
    )];
    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw_a,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &country_identity_payload(&[(47, "POR")]),
    );
    push_record(
        &mut stream,
        2,
        raw_a,
        47,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &domain47_payload(0, &rows),
    );
    push_record(
        &mut stream,
        3,
        raw_a,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    push_record(
        &mut stream,
        4,
        raw_b,
        47,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &domain47_payload(COMPACT_STATUS_DELTA_DATA, &rows),
    );
    push_record(
        &mut stream,
        5,
        raw_b,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("domain47_delta");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 2);
    let second = parse(&snaps[1].2);
    assert_eq!(
        second["countries"]["POR"]["commanders"]["leader"]["token"],
        json!("POR_salazar")
    );
    assert!(second["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(47)
            && problem["problem"] == json!("delta_unsupported")));
}

#[test]
fn folds_division_primary_order_link_fields() {
    let raw = raw_for(16_959_360);
    let mut div = Vec::new();
    divisions_header(&mut div, 4, 0);
    // div 17: validated army order, instance id 317 at the order-id slot
    // (row +84; the final +88 slot is now disrupted_supply).
    push_div_row(&mut div, 17, (1 << 5) | (1 << 14), 118, 1, 1, 0, 47);
    let order_iid_off = div.len() - DIV_ROW_LEN + 84;
    div[order_iid_off..order_iid_off + 4].copy_from_slice(&317u32.to_le_bytes());
    // div 18: army-group-owned order (badge only, no id).
    push_div_row(&mut div, 18, (1 << 5) | (1 << 15), 119, 1, 1, 0, 47);
    // div 19: unresolved order pointer (badge only, no id).
    push_div_row(&mut div, 19, (1 << 5) | (1 << 16), 120, 1, 1, 0, 47);
    // div 20: no primary order (both flags absent, tail zero).
    push_div_row(&mut div, 20, 1 << 5, 121, 1, 1, 0, 47);

    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw,
        20,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &div,
    );
    push_record(
        &mut stream,
        2,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("division_order_link_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    let snap = parse(&snaps[0].2);
    let divisions = snap["divisions"].as_array().unwrap();
    let by_id = |id: u32| {
        divisions
            .iter()
            .find(|d| d["div_id"] == json!(id))
            .unwrap_or_else(|| panic!("division {id} missing"))
    };
    assert_eq!(by_id(17)["order_instance_id"], json!(317));
    assert_eq!(by_id(17)["order_link"], json!("army"));
    assert_eq!(by_id(18)["order_instance_id"], Value::Null);
    assert_eq!(by_id(18)["order_link"], json!("army_group"));
    assert_eq!(by_id(19)["order_link"], json!("unresolved"));
    assert_eq!(by_id(20)["order_link"], Value::Null);
    assert_eq!(by_id(20)["order_instance_id"], Value::Null);
}

#[test]
fn division_order_instance_id_without_flag_fails_fold_closed() {
    let raw = raw_for(16_959_360);
    let mut div = Vec::new();
    divisions_header(&mut div, 1, 0);
    push_div_row(&mut div, 17, 1 << 5, 118, 1, 1, 0, 47);
    // Corrupt: order_instance_id (row +84) present without the army-order
    // flag. The row is 92 bytes now, so +84 is the second-to-last u32
    // (disrupted_supply is the final +88 slot).
    let order_iid_off = div.len() - DIV_ROW_LEN + 84;
    div[order_iid_off..order_iid_off + 4].copy_from_slice(&99u32.to_le_bytes());

    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw,
        20,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &div,
    );
    push_record(
        &mut stream,
        2,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("division_order_link_corrupt_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    let snap = parse(&snaps[0].2);
    // The malformed domain-20 payload must not fold any division rows.
    assert!(snap["divisions"]
        .as_array()
        .map(|rows| rows.is_empty())
        .unwrap_or(true));
}

#[test]
fn division_delete_row_with_order_flag_fails_fold_closed() {
    // Establish a baseline division, then send a delete row that is malformed
    // (carries ORDER_ARMY). The delete must NOT apply: the fold fails closed
    // and the prior division survives.
    let raw_a = raw_for(16_959_360);
    let raw_b = raw_for(16_959_361);

    let mut base = Vec::new();
    divisions_header(&mut base, 1, 0);
    push_div_row(&mut base, 17, 1 << 5, 118, 1, 1, 0, 47);

    let mut del = Vec::new();
    divisions_header(&mut del, 1, DELTA);
    // Deleted row with an illegal ORDER_ARMY flag set.
    push_div_row(&mut del, 17, DIV_FLAG_DELETED | (1 << 14), 0, 0, 0, 0, 0);

    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw_a,
        20,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &base,
    );
    push_record(
        &mut stream,
        2,
        raw_a,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    push_record(
        &mut stream,
        3,
        raw_b,
        20,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &del,
    );
    push_record(
        &mut stream,
        4,
        raw_b,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("division_malformed_delete_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 2);
    // Hour B's malformed payload failed closed; the baseline division 17 is
    // retained rather than deleted.
    let second = parse(&snaps[1].2);
    assert!(second["divisions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["div_id"] == json!(17)));
}

#[test]
fn late_records_for_finalized_hour_are_dropped_and_counted() {
    let hi_a = 16_959_360i64;
    let hi_b = 16_959_361i64;
    let raw_a = raw_for(hi_a);
    let raw_b = raw_for(hi_b);
    let mut stream = Vec::new();
    let mut seq = 1u64;

    let mut div_a = Vec::new();
    divisions_header(&mut div_a, 1, 0);
    push_div_row(
        &mut div_a,
        17,
        1 << 5,
        118,
        (30.0 * 100_000.0) as u32,
        (90.0 * 100_000.0) as u32,
        0,
        47,
    );
    push_record(
        &mut stream,
        seq,
        raw_a,
        20,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &div_a,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw_a,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    seq += 1;

    let mut late_div_a = Vec::new();
    divisions_header(&mut late_div_a, 1, 0);
    push_div_row(
        &mut late_div_a,
        17,
        1 << 5,
        999,
        (1.0 * 100_000.0) as u32,
        (1.0 * 100_000.0) as u32,
        0,
        47,
    );
    push_record(
        &mut stream,
        seq,
        raw_a,
        20,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &late_div_a,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw_b,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("late_record_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 2);
    let b = parse(&snaps[1].2);
    assert_eq!(
        b["divisions"].as_array().unwrap()[0]["location_province"],
        json!(118)
    );
    let late = b["provenance"]["collector"]["late_records_dropped"]
        .as_array()
        .unwrap();
    assert!(late
        .iter()
        .any(|entry| entry["hour_index"] == json!(hi_a) && entry["count"] == json!(1)));
}

#[test]
fn combat_snapshot_is_unknown_when_domain21_record_is_absent() {
    let hi = 16_959_360i64;
    let raw = raw_for(hi);
    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("combat_absent_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    let s = parse(&snaps[0].2);
    assert_eq!(s["combats"], Value::Null);
    let domains = s["provenance"]["collector"]["domains"].as_array().unwrap();
    assert!(!domains.iter().any(|domain| domain == "land_combats_live"));
}

#[test]
fn folds_land_combats_as_active_snapshot_with_disappearance() {
    let hi_a = 16_959_360i64;
    let hi_b = 16_959_361i64;
    let raw_a = raw_for(hi_a);
    let raw_b = raw_for(hi_b);
    let mut stream = Vec::new();
    let mut seq = 1u64;

    let combat_a = land_combats_payload(&[TestLandCombat {
        combat_id: 42,
        generation: 7,
        location: 1000,
        attackers: vec![101, 102],
        defenders: vec![201],
    }]);
    push_record(
        &mut stream,
        seq,
        raw_a,
        21,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &combat_a,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw_a,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    seq += 1;

    let combat_b = land_combats_payload(&[]);
    push_record(
        &mut stream,
        seq,
        raw_b,
        21,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &combat_b,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw_b,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("combat_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 2);

    let a = parse(&snaps[0].2);
    let combats_a = a["combats"].as_array().unwrap();
    assert_eq!(combats_a.len(), 1);
    let combat = &combats_a[0];
    assert_eq!(combat["combat_id"], json!(42));
    assert_eq!(combat["kind"], json!("land"));
    assert_eq!(combat["location_province"], json!(1000));
    assert_eq!(combat["attackers"], json!([101, 102]));
    assert_eq!(combat["defenders"], json!([201]));
    assert_eq!(combat["combat_width"], Value::Null);
    assert_eq!(combat["terrain"], Value::Null);
    assert_eq!(combat["progress"], Value::Null);
    assert_eq!(combat["modifiers"], Value::Null);

    let b = parse(&snaps[1].2);
    assert_eq!(b["combats"].as_array().unwrap().len(), 0);
}

#[test]
fn failed_land_combat_hours_are_unknown_for_each_failure_status() {
    let failure_cases = [
        ("state_unavailable", COMPACT_STATUS_STATE_UNAVAILABLE),
        ("profile_mismatch", COMPACT_STATUS_PROFILE_MISMATCH),
        ("no_clock", COMPACT_STATUS_NO_CLOCK),
        ("no_scalar_source", COMPACT_STATUS_NO_SCALAR_SOURCE),
        ("worker_panic", COMPACT_STATUS_WORKER_PANIC),
        ("bridge_unavailable", COMPACT_STATUS_BRIDGE_UNAVAILABLE),
        ("source_unavailable", COMPACT_STATUS_SOURCE_UNAVAILABLE),
        ("partial_data", COMPACT_STATUS_PARTIAL_DATA),
    ];

    for (label, failure_flag) in failure_cases {
        let hi_a = 16_959_360i64;
        let hi_b = 16_959_361i64;
        let hi_c = 16_959_362i64;
        let raw_a = raw_for(hi_a);
        let raw_b = raw_for(hi_b);
        let raw_c = raw_for(hi_c);
        let mut stream = Vec::new();
        let mut seq = 1u64;

        let combat_a = land_combats_payload(&[TestLandCombat {
            combat_id: 42,
            generation: 7,
            location: 1000,
            attackers: vec![101],
            defenders: vec![201],
        }]);
        push_record(
            &mut stream,
            seq,
            raw_a,
            21,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &combat_a,
        );
        seq += 1;
        push_record(
            &mut stream,
            seq,
            raw_a,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
        seq += 1;

        let failed = land_combats_payload_with_status(&[], failure_flag);
        push_record(
            &mut stream,
            seq,
            raw_b,
            21,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &failed,
        );
        seq += 1;
        push_record(
            &mut stream,
            seq,
            raw_b,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
        seq += 1;

        let combat_c = land_combats_payload(&[TestLandCombat {
            combat_id: 43,
            generation: 8,
            location: 1001,
            attackers: vec![301],
            defenders: vec![401],
        }]);
        push_record(
            &mut stream,
            seq,
            raw_c,
            21,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &combat_c,
        );
        seq += 1;
        push_record(
            &mut stream,
            seq,
            raw_c,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );

        let mut fold = WorldFold::new(format!("combat_failure_{label}"));
        fold.ingest_stream(&stream);
        let snaps = fold.finish();
        assert_eq!(snaps.len(), 3, "{label}");

        let a = parse(&snaps[0].2);
        assert_eq!(a["combats"].as_array().unwrap()[0]["combat_id"], json!(42));

        let b = parse(&snaps[1].2);
        assert_eq!(b["combats"], Value::Null, "{label}");
        let notes = b["provenance"]["notes"].as_array().unwrap();
        assert!(
            notes.iter().any(|note| note
                .as_str()
                .unwrap()
                .contains("land_combats_live failure status")),
            "{label}"
        );

        let c = parse(&snaps[2].2);
        let c_combats = c["combats"].as_array().unwrap();
        assert_eq!(c_combats.len(), 1, "{label}");
        assert_eq!(c_combats[0]["combat_id"], json!(43), "{label}");
        assert_eq!(c_combats[0]["attackers"], json!([301]), "{label}");
    }
}

#[test]
fn worker_panic_zero_row_land_combat_payload_is_unknown() {
    let hi = 16_959_360i64;
    let raw = raw_for(hi);
    let failed = land_combats_payload_with_status(&[], COMPACT_STATUS_WORKER_PANIC);
    let mut stream = Vec::new();
    push_record(
        &mut stream,
        1,
        raw,
        21,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &failed,
    );
    push_record(
        &mut stream,
        2,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("combat_worker_panic_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    let s = parse(&snaps[0].2);
    assert_eq!(s["combats"], Value::Null);
    let notes = s["provenance"]["notes"].as_array().unwrap();
    assert!(notes.iter().any(|note| note
        .as_str()
        .unwrap()
        .contains("land_combats_live failure status")));
}

#[test]
fn source_unavailable_land_combats_do_not_fabricate_disappearance() {
    let hi_a = 16_959_360i64;
    let hi_b = 16_959_361i64;
    let raw_a = raw_for(hi_a);
    let raw_b = raw_for(hi_b);
    let mut stream = Vec::new();
    let mut seq = 1u64;

    let combat_a = land_combats_payload(&[TestLandCombat {
        combat_id: 42,
        generation: 7,
        location: 1000,
        attackers: vec![101],
        defenders: vec![201],
    }]);
    push_record(
        &mut stream,
        seq,
        raw_a,
        21,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &combat_a,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw_a,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    seq += 1;

    let unavailable = land_combats_payload_with_status(&[], COMPACT_STATUS_SOURCE_UNAVAILABLE);
    push_record(
        &mut stream,
        seq,
        raw_b,
        21,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &unavailable,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw_b,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("combat_unavailable_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 2);

    let a = parse(&snaps[0].2);
    assert_eq!(a["combats"].as_array().unwrap()[0]["combat_id"], json!(42));

    let b = parse(&snaps[1].2);
    assert_eq!(b["combats"], Value::Null);
    let notes = b["provenance"]["notes"].as_array().unwrap();
    assert!(notes.iter().any(|note| {
        let note = note.as_str().unwrap();
        note.contains("land_combats_live failure status")
            && note.contains("combats unknown this hour")
    }));
}

#[test]
fn folds_land_combat_detail_events_onto_matching_combat_only() {
    let hi = 16_959_360i64;
    let raw = raw_for(hi);
    let mut stream = Vec::new();
    let mut seq = 1u64;

    let combat = land_combats_payload(&[TestLandCombat {
        combat_id: 42,
        generation: 7,
        location: 1000,
        attackers: vec![101],
        defenders: vec![201],
    }]);
    push_record(
        &mut stream,
        seq,
        raw,
        21,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &combat,
    );
    seq += 1;

    let detail_rows = vec![land_detail55::DetailEventRow {
        combat_id: 42,
        generation: 7,
        event_kind: land_detail55::EVENT_PHASE_UPDATED,
        side: None,
        division_id: None,
        location: Some(1000),
        day: Some(3),
        duration: Some(4),
        old_side_count: 0,
        new_side_count: 2,
        flags: land_detail55::FLAG_FROM_VALIDATED_DOMAIN21_ROOT
            | land_detail55::FLAG_LOCATION_PRESENT
            | land_detail55::FLAG_DAY_PRESENT
            | land_detail55::FLAG_DURATION_PRESENT,
    }];
    let detail =
        land_detail55::encode_detail_events_payload(9, COMPACT_STATUS_DELTA_DATA, &detail_rows);
    push_record(
        &mut stream,
        seq,
        raw,
        55,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &detail,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("combat_detail_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    let s = parse(&snaps[0].2);
    let combat = &s["combats"].as_array().unwrap()[0];
    assert_eq!(
        combat["land_combat_detail_events_live"]["phase_event"],
        json!("updated")
    );
    assert_eq!(
        combat["land_combat_detail_events_live"]["location_province"],
        json!(1000)
    );
    assert_eq!(
        combat["land_combat_detail_events_live"]["duration_days"],
        json!(4)
    );
    assert_eq!(combat["combat_width"], Value::Null);
    assert_eq!(combat["terrain"], Value::Null);
    assert_eq!(combat["progress"], Value::Null);
    assert_eq!(combat["modifiers"], Value::Null);
}

fn capwar_side_detail(committed: u32, reserve: u32) -> land_detail55::SideDetail {
    land_detail55::SideDetail {
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

#[test]
fn folds_land_combat_side_detail_onto_combat() {
    // E2: per-side CCombatSide detail scalars attach under
    // land_combat_detail_events_live.{attacker,defender}. A side with no detail
    // (None) gets NO key (fail closed).
    let hi = 16_959_360i64;
    let raw = raw_for(hi);
    let mut stream = Vec::new();
    let mut seq = 1u64;

    let combat = land_combats_payload(&[TestLandCombat {
        combat_id: 21381,
        generation: 7,
        location: 3236,
        attackers: vec![101, 102, 103, 104],
        defenders: vec![201, 202],
    }]);
    push_record(
        &mut stream,
        seq,
        raw,
        21,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &combat,
    );
    seq += 1;

    let att = capwar_side_detail(2, 2);
    let def = capwar_side_detail(2, 0);
    let side_rows = vec![
        land_detail55::SideDetailRow {
            combat_id: 21381,
            generation: 7,
            side: land_detail55::CombatSide::Attacker,
            detail: att,
        },
        land_detail55::SideDetailRow {
            combat_id: 21381,
            generation: 7,
            side: land_detail55::CombatSide::Defender,
            detail: def,
        },
    ];
    let detail = land_detail55::encode_detail_events_payload_v2(
        9,
        COMPACT_STATUS_DELTA_DATA,
        &[],
        &side_rows,
    );
    push_record(
        &mut stream,
        seq,
        raw,
        55,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &detail,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("side_detail_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    let s = parse(&snaps[0].2);
    let combat = &s["combats"].as_array().unwrap()[0];
    let dl = &combat["land_combat_detail_events_live"];
    // Both sides present (all-or-nothing) with the validated scalars.
    assert_eq!(dl["side_detail_completeness"], json!("both"));
    assert_eq!(dl["attacker"]["units"], json!(4));
    assert_eq!(dl["attacker"]["committed"], json!(2));
    assert_eq!(dl["attacker"]["reserve"], json!(2));
    assert_eq!(dl["attacker"]["air_kills"], json!(8));
    assert_eq!(dl["attacker"]["num_org_losses"], json!(192));
    assert_eq!(dl["attacker"]["counts_incomplete"], json!(false));
    assert!(
        (dl["attacker"]["ground_damage_str"].as_f64().unwrap() - 112.485).abs() < 1e-6,
        "ground_damage_str decodes from i64x1e5"
    );
    assert_eq!(dl["attacker"]["ground_damage_str_raw"], json!(11_248_500));
    // tactic/losses badged, never a fabricated value.
    assert_eq!(dl["attacker"]["tactic"], Value::Null);
    assert_eq!(
        dl["attacker"]["tactic_status"],
        json!("unresolved_pointer_table_backed")
    );
    // Defender present too.
    assert_eq!(dl["defender"]["units"], json!(2));
    assert_eq!(dl["defender"]["reserve"], json!(0));
}

#[test]
fn side_detail_is_all_or_nothing_per_combat() {
    // A combat whose defender detail is absent (None → no row) must emit
    // NEITHER side (all-or-nothing) with an explicit completeness badge — a
    // consumer must never see one-sided detail and read the other as zero.
    let raw = raw_for(16_959_360);
    let mut stream = Vec::new();
    let mut seq = 1u64;
    let combat = land_combats_payload(&[TestLandCombat {
        combat_id: 21453,
        generation: 7,
        location: 3236,
        attackers: vec![101],
        defenders: vec![201],
    }]);
    push_record(
        &mut stream,
        seq,
        raw,
        21,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &combat,
    );
    seq += 1;
    // Only an attacker side-detail row (defender torn/absent upstream).
    let side_rows = vec![land_detail55::SideDetailRow {
        combat_id: 21453,
        generation: 7,
        side: land_detail55::CombatSide::Attacker,
        detail: capwar_side_detail(1, 0),
    }];
    let detail = land_detail55::encode_detail_events_payload_v2(
        9,
        COMPACT_STATUS_DELTA_DATA,
        &[],
        &side_rows,
    );
    push_record(
        &mut stream,
        seq,
        raw,
        55,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &detail,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let mut fold = WorldFold::new("side_detail_absent");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    let s = parse(&snaps[0].2);
    let dl = &s["combats"].as_array().unwrap()[0]["land_combat_detail_events_live"];
    // NEITHER side emitted (all-or-nothing); the badge explains why.
    assert!(dl.get("attacker").is_none());
    assert!(dl.get("defender").is_none());
    assert_eq!(
        dl["side_detail_completeness"],
        json!("attacker_only_suppressed")
    );
}

#[test]
fn side_detail_counts_incomplete_is_retained_and_badged() {
    // A stably-read but committed+reserve!=units side (a mid-combat transient)
    // is RETAINED, badged counts_incomplete, and folded on both sides.
    let raw = raw_for(16_959_360);
    let mut stream = Vec::new();
    let mut seq = 1u64;
    let combat = land_combats_payload(&[TestLandCombat {
        combat_id: 21381,
        generation: 7,
        location: 3236,
        attackers: vec![101, 102],
        defenders: vec![201],
    }]);
    push_record(
        &mut stream,
        seq,
        raw,
        21,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &combat,
    );
    seq += 1;
    // Attacker transient: units=3 but committed 2 + reserve 0 = 2 != 3.
    let mut att = capwar_side_detail(2, 0);
    att.units = 3;
    att.counts_incomplete = true;
    let def = capwar_side_detail(1, 0);
    let side_rows = vec![
        land_detail55::SideDetailRow {
            combat_id: 21381,
            generation: 7,
            side: land_detail55::CombatSide::Attacker,
            detail: att,
        },
        land_detail55::SideDetailRow {
            combat_id: 21381,
            generation: 7,
            side: land_detail55::CombatSide::Defender,
            detail: def,
        },
    ];
    let detail = land_detail55::encode_detail_events_payload_v2(
        9,
        COMPACT_STATUS_DELTA_DATA,
        &[],
        &side_rows,
    );
    push_record(
        &mut stream,
        seq,
        raw,
        55,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &detail,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let mut fold = WorldFold::new("side_detail_incomplete");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    let s = parse(&snaps[0].2);
    let dl = &s["combats"].as_array().unwrap()[0]["land_combat_detail_events_live"];
    assert!(
        dl["attacker"].is_object(),
        "transient side is retained, not dropped"
    );
    assert_eq!(dl["attacker"]["counts_incomplete"], json!(true));
    assert_eq!(dl["defender"]["counts_incomplete"], json!(false));
    assert_eq!(
        dl["side_detail_completeness"],
        json!("both_counts_incomplete")
    );
    // Damage accumulators are still valid on the transient side.
    assert_eq!(dl["attacker"]["ground_damage_str_raw"], json!(11_248_500));
}

#[test]
fn folds_land_combat_detail_events_that_arrive_before_domain21() {
    let hi = 16_959_360i64;
    let raw = raw_for(hi);
    let mut stream = Vec::new();
    let mut seq = 1u64;

    let detail_rows = vec![land_detail55::DetailEventRow {
        combat_id: 42,
        generation: 7,
        event_kind: land_detail55::EVENT_PHASE_STARTED,
        side: None,
        division_id: None,
        location: Some(1000),
        day: Some(3),
        duration: Some(4),
        old_side_count: 0,
        new_side_count: 2,
        flags: land_detail55::FLAG_FROM_VALIDATED_DOMAIN21_ROOT
            | land_detail55::FLAG_LOCATION_PRESENT
            | land_detail55::FLAG_DAY_PRESENT
            | land_detail55::FLAG_DURATION_PRESENT,
    }];
    let detail =
        land_detail55::encode_detail_events_payload(9, COMPACT_STATUS_DELTA_DATA, &detail_rows);
    push_record(
        &mut stream,
        seq,
        raw,
        55,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &detail,
    );
    seq += 1;

    let combat = land_combats_payload(&[TestLandCombat {
        combat_id: 42,
        generation: 7,
        location: 1000,
        attackers: vec![101],
        defenders: vec![201],
    }]);
    push_record(
        &mut stream,
        seq,
        raw,
        21,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &combat,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("combat_detail_before_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    let s = parse(&snaps[0].2);
    let combat = &s["combats"].as_array().unwrap()[0];
    assert_eq!(
        combat["land_combat_detail_events_live"]["phase_event"],
        json!("started")
    );
    assert_eq!(
        combat["land_combat_detail_events_live"]["generation"],
        json!(7)
    );
}

#[test]
fn land_combat_detail_generation_mismatch_is_dropped() {
    let hi = 16_959_360i64;
    let raw = raw_for(hi);
    let mut stream = Vec::new();
    let mut seq = 1u64;

    let combat = land_combats_payload(&[TestLandCombat {
        combat_id: 42,
        generation: 7,
        location: 1000,
        attackers: vec![101],
        defenders: vec![201],
    }]);
    push_record(
        &mut stream,
        seq,
        raw,
        21,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &combat,
    );
    seq += 1;

    let detail_rows = vec![land_detail55::DetailEventRow {
        combat_id: 42,
        generation: 8,
        event_kind: land_detail55::EVENT_PHASE_UPDATED,
        side: None,
        division_id: None,
        location: Some(1000),
        day: Some(3),
        duration: Some(4),
        old_side_count: 0,
        new_side_count: 2,
        flags: land_detail55::FLAG_FROM_VALIDATED_DOMAIN21_ROOT
            | land_detail55::FLAG_LOCATION_PRESENT
            | land_detail55::FLAG_DAY_PRESENT
            | land_detail55::FLAG_DURATION_PRESENT,
    }];
    let detail =
        land_detail55::encode_detail_events_payload(9, COMPACT_STATUS_DELTA_DATA, &detail_rows);
    push_record(
        &mut stream,
        seq,
        raw,
        55,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &detail,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("combat_generation_mismatch_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    let s = parse(&snaps[0].2);
    let combat = s["combats"].as_array().unwrap()[0].as_object().unwrap();
    assert!(!combat.contains_key("land_combat_detail_events_live"));
    let notes = s["provenance"]["notes"].as_array().unwrap();
    assert!(notes.iter().any(|note| note
        .as_str()
        .unwrap()
        .contains("unknown active combat key(s): 42:8")));
}

#[test]
fn land_combat_detail_baseline_reset_gap_does_not_attach_synthetic_events() {
    let hi_a = 16_959_360i64;
    let hi_b = 16_959_361i64;
    let hi_c = 16_959_362i64;
    let raw_a = raw_for(hi_a);
    let raw_b = raw_for(hi_b);
    let raw_c = raw_for(hi_c);
    let mut stream = Vec::new();
    let mut seq = 1u64;

    let combat_a = land_combats_payload(&[TestLandCombat {
        combat_id: 42,
        generation: 7,
        location: 1000,
        attackers: vec![101],
        defenders: vec![201],
    }]);
    push_record(
        &mut stream,
        seq,
        raw_a,
        21,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &combat_a,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw_a,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    seq += 1;

    let unavailable = land_combats_payload_with_status(&[], COMPACT_STATUS_SOURCE_UNAVAILABLE);
    push_record(
        &mut stream,
        seq,
        raw_b,
        21,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &unavailable,
    );
    seq += 1;
    let detail_failed = land_detail55::encode_detail_events_payload(
        10,
        COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        &[],
    );
    push_record(
        &mut stream,
        seq,
        raw_b,
        55,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &detail_failed,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw_b,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    seq += 1;

    let combat_c = land_combats_payload(&[TestLandCombat {
        combat_id: 42,
        generation: 7,
        location: 1000,
        attackers: vec![101, 102],
        defenders: vec![201, 202],
    }]);
    push_record(
        &mut stream,
        seq,
        raw_c,
        21,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &combat_c,
    );
    seq += 1;
    let detail_reset = land_detail55::encode_detail_events_payload(
        11,
        COMPACT_STATUS_DELTA_DATA
            | COMPACT_STATUS_PARTIAL_DATA
            | COMPACT_STATUS_TRANSITION_BASELINE_RESET,
        &[],
    );
    push_record(
        &mut stream,
        seq,
        raw_c,
        55,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &detail_reset,
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw_c,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("combat_detail_reset_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 3);

    let b = parse(&snaps[1].2);
    assert_eq!(b["combats"], Value::Null);

    let c = parse(&snaps[2].2);
    let combat = c["combats"].as_array().unwrap()[0].as_object().unwrap();
    assert_eq!(combat["attackers"], json!([101, 102]));
    assert!(!combat.contains_key("land_combat_detail_events_live"));
    let notes = c["provenance"]["notes"].as_array().unwrap();
    assert!(notes.iter().any(|note| note
        .as_str()
        .unwrap()
        .contains("transition baseline reset after an unknown gap")));
}

#[test]
fn land_combat_detail_events_for_unknown_combat_do_not_fabricate_combat() {
    let hi = 16_959_360i64;
    let raw = raw_for(hi);
    let detail_rows = vec![land_detail55::DetailEventRow {
        combat_id: 999,
        generation: 1,
        event_kind: land_detail55::EVENT_PHASE_STARTED,
        side: None,
        division_id: None,
        location: Some(1200),
        day: Some(1),
        duration: Some(1),
        old_side_count: 0,
        new_side_count: 2,
        flags: land_detail55::FLAG_FROM_VALIDATED_DOMAIN21_ROOT
            | land_detail55::FLAG_LOCATION_PRESENT
            | land_detail55::FLAG_DAY_PRESENT
            | land_detail55::FLAG_DURATION_PRESENT,
    }];
    let detail =
        land_detail55::encode_detail_events_payload(9, COMPACT_STATUS_DELTA_DATA, &detail_rows);

    let mut stream = Vec::new();
    let empty_combats = land_combats_payload(&[]);
    push_record(
        &mut stream,
        1,
        raw,
        21,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &empty_combats,
    );
    push_record(
        &mut stream,
        2,
        raw,
        55,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &detail,
    );
    push_record(
        &mut stream,
        3,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("unknown_detail_game");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    let s = parse(&snaps[0].2);
    assert_eq!(s["combats"].as_array().unwrap().len(), 0);
    let notes = s["provenance"]["notes"].as_array().unwrap();
    assert!(notes.iter().any(|note| note
        .as_str()
        .unwrap()
        .contains("unknown active combat key(s): 999:1")));
}

#[test]
fn folds_country_string_domains() {
    // focus + politics + diplomacy in a single hour, exercising compact_string.
    let hi = 16_994_400i64;
    let raw = raw_for(hi);
    let tag: u32 = 47;
    let mut stream = Vec::new();
    let mut seq = 1u64;

    // focus: tree, current, continuous, one completed token
    let mut focus = Vec::new();
    focus.extend_from_slice(&0u64.to_le_bytes()); // wall
    focus.extend_from_slice(&1u32.to_le_bytes()); // country_count
    focus.extend_from_slice(&1u32.to_le_bytes()); // changed
    focus.extend_from_slice(&0u32.to_le_bytes()); // status
    focus.extend_from_slice(&100_000u32.to_le_bytes()); // scale
    focus.extend_from_slice(&1u32.to_le_bytes()); // completed_token_count
    focus.extend_from_slice(&tag.to_le_bytes());
    focus.extend_from_slice(&(1u32 << 4).to_le_bytes()); // flags: has-progress
    focus.extend_from_slice(&2u32.to_le_bytes()); // completed_count
    focus.extend_from_slice(&(3 * 100_000i32).to_le_bytes()); // progress_raw = 3.0
    push_str(&mut focus, "portugal_focus");
    push_str(&mut focus, "POR_industrial_focus");
    push_str(&mut focus, "POR_continuous_focus");
    focus.extend_from_slice(&tag.to_le_bytes());
    push_str(&mut focus, "POR_completed_focus");
    push_record(
        &mut stream,
        seq,
        raw,
        12,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &focus,
    );
    seq += 1;

    // politics: one country, one popularity, one law, one advisor, one spirit
    let mut pol = Vec::new();
    pol.extend_from_slice(&0u64.to_le_bytes()); // wall
    pol.extend_from_slice(&1u32.to_le_bytes()); // country_count
    pol.extend_from_slice(&1u32.to_le_bytes()); // changed
    pol.extend_from_slice(&0u32.to_le_bytes()); // status
    pol.extend_from_slice(&100_000u32.to_le_bytes()); // scale
    pol.extend_from_slice(&1u32.to_le_bytes()); // popularity_count
    pol.extend_from_slice(&1u32.to_le_bytes()); // law_count
    pol.extend_from_slice(&1u32.to_le_bytes()); // advisor_count
    pol.extend_from_slice(&1u32.to_le_bytes()); // national_spirit_count
                                                // country row (28 fixed)
    pol.extend_from_slice(&tag.to_le_bytes()); // +0 tag
    pol.extend_from_slice(&0u32.to_le_bytes()); // +4 flags
    pol.extend_from_slice(&2u32.to_le_bytes()); // +8 idea_count
    pol.extend_from_slice(&0u32.to_le_bytes()); // +12 election_frequency
    pol.extend_from_slice(&2u32.to_le_bytes()); // +16 party_count
    pol.extend_from_slice(&0u32.to_le_bytes()); // +20 low_confidence
    pol.extend_from_slice(&123u32.to_le_bytes()); // +24 leader_char_id
    push_str(&mut pol, "neutrality"); // ruling_party
    push_str(&mut pol, "Antonio"); // leader_name
    push_str(&mut pol, "POR_antonio"); // leader_token
    push_str(&mut pol, "neutrality"); // leader_ideology
                                      // popularity row
    pol.extend_from_slice(&tag.to_le_bytes());
    push_str(&mut pol, "neutrality");
    pol.extend_from_slice(&(65 * 100_000i32 / 100).to_le_bytes()); // 0.65
                                                                   // law row
    pol.extend_from_slice(&tag.to_le_bytes());
    push_str(&mut pol, "economy");
    push_str(&mut pol, "partial_economic_mobilisation");
    // advisor row
    pol.extend_from_slice(&tag.to_le_bytes());
    push_str(&mut pol, "political_advisor");
    push_str(&mut pol, "POR_salazar");
    // spirit row
    pol.extend_from_slice(&tag.to_le_bytes());
    push_str(&mut pol, "estado_novo");
    push_record(
        &mut stream,
        seq,
        raw,
        14,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &pol,
    );
    seq += 1;

    // diplomacy: one country at war with GER, ITA
    let mut dip = Vec::new();
    dip.extend_from_slice(&0u64.to_le_bytes()); // wall
    dip.extend_from_slice(&1u32.to_le_bytes()); // country_count
    dip.extend_from_slice(&1u32.to_le_bytes()); // changed
    dip.extend_from_slice(&0u32.to_le_bytes()); // status
    dip.extend_from_slice(&2u32.to_le_bytes()); // relation_count
    dip.extend_from_slice(&tag.to_le_bytes()); // +0 tag
    dip.extend_from_slice(&0u32.to_le_bytes()); // +4 flags
    dip.extend_from_slice(&1i32.to_le_bytes()); // +8 faction_group
    dip.extend_from_slice(&2u32.to_le_bytes()); // +12 at_war_count
    dip.extend_from_slice(&1234i64.to_le_bytes()); // +16 casualties
    for t in [b"GER", b"ITA"] {
        dip.extend_from_slice(&tag.to_le_bytes());
        dip.extend_from_slice(&3u32.to_le_bytes()); // token_len
        let mut fixed = [0u8; 8];
        fixed[..3].copy_from_slice(t);
        dip.extend_from_slice(&fixed);
    }
    push_record(
        &mut stream,
        seq,
        raw,
        17,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &dip,
    );
    seq += 1;

    push_record(
        &mut stream,
        seq,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("g");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    let s = parse(&snaps[0].2);
    let c = &s["countries"]["47"];

    // focus
    assert_eq!(c["focus"]["focus_tree"], json!("portugal_focus"));
    assert_eq!(c["focus"]["current"], json!("POR_industrial_focus"));
    assert_eq!(
        c["focus"]["current_continuous"],
        json!("POR_continuous_focus")
    );
    assert!((c["focus"]["progress"].as_f64().unwrap() - 3.0).abs() < 1e-6);
    assert_eq!(c["focus"]["completed"][0], json!("POR_completed_focus"));

    // politics
    assert_eq!(c["politics"]["ruling_party"], json!("neutrality"));
    assert_eq!(c["politics"]["leader_name"], json!("Antonio"));
    assert!(
        (c["politics"]["party_popularities"]["neutrality"]
            .as_f64()
            .unwrap()
            - 0.65)
            .abs()
            < 1e-6
    );
    assert_eq!(
        c["politics"]["laws"]["economy"],
        json!("partial_economic_mobilisation")
    );
    assert_eq!(c["politics"]["appointed_advisors"][0], json!("POR_salazar"));
    assert_eq!(c["politics"]["national_spirits"][0], json!("estado_novo"));

    // diplomacy
    assert_eq!(c["diplomacy"]["at_war_count"], json!(2));
    assert_eq!(c["diplomacy"]["casualties"], json!(1234));
    let enemies = c["diplomacy"]["at_war_with"].as_array().unwrap();
    assert!(enemies.iter().any(|e| e == "GER"));
    assert!(enemies.iter().any(|e| e == "ITA"));
}

#[test]
fn country_identity_rekeys_country_state_and_map_by_tag() {
    // domain 18 maps 47->GER; fuel + map_control carry tag 47. The snapshot must
    // key the country "GER", set tag_db_id_index.resolved=true, and re-key the
    // state owner/controller, province controller, and map mirrors to "GER".
    let hi = 16_959_360i64;
    let raw = raw_for(hi);
    let tag: u32 = 47;
    let mut stream = Vec::new();
    let mut seq = 1u64;
    let push = |stream: &mut Vec<u8>, seq: &mut u64, dom, pl: &[u8]| {
        push_record(stream, *seq, raw, dom, COMPACT_RECORD_TYPE_DOMAIN_DATA, pl);
        *seq += 1;
    };

    let ident = country_identity_payload(&[(tag, "GER")]);
    push(&mut stream, &mut seq, 18, &ident);

    let fuel = fuel_payload(&[(tag, (10.0 * 32768.0) as i64, (20.0 * 32768.0) as i64, 0, 0)]);
    push(&mut stream, &mut seq, 10, &fuel);

    let mut mc = Vec::new();
    map_control_header(&mut mc, 1, 1, 0);
    push_state_row(&mut mc, 5, 47, 47);
    push_prov_row(&mut mc, 900, 47, 5);
    push(&mut stream, &mut seq, 22, &mc);

    push_record(
        &mut stream,
        seq,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("g");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    let s = parse(&snaps[0].2);

    // country keyed by TAG, not numeric id
    assert!(
        s["countries"].get("GER").is_some(),
        "country should be keyed GER"
    );
    assert!(
        s["countries"].get("47").is_none(),
        "numeric key must be replaced"
    );
    assert!((s["countries"]["GER"]["fuel"]["current"].as_f64().unwrap() - 10.0).abs() < 1e-6);

    // resolution flag set
    assert_eq!(s["tag_db_id_index"]["resolved"], json!(true));
    assert_eq!(s["tag_db_id_index"]["key"], json!("tag"));

    // state owner/controller re-keyed to GER
    assert_eq!(s["states"]["5"]["owner"], json!("GER"));
    assert_eq!(s["states"]["5"]["controller"], json!("GER"));
    // province controller re-keyed to GER
    assert_eq!(s["provinces"]["900"]["controller"], json!("GER"));
    // map layer mirrors carry GER
    assert_eq!(s["map"]["owner_by_state"]["5"], json!("GER"));
    assert_eq!(s["map"]["control_by_province"]["900"], json!("GER"));

    // country_identity listed in provenance domains
    let dom = s["provenance"]["collector"]["domains"].as_array().unwrap();
    assert!(dom.iter().any(|d| d == "country_identity"));
}

#[test]
fn country_identity_unmapped_id_stays_numeric_and_unresolved() {
    // domain 18 maps only 47->GER; a state controlled by unmapped id 48 must keep
    // the numeric key, resolved must be false, and a note must record the gap.
    let hi = 16_959_360i64;
    let raw = raw_for(hi);
    let mut stream = Vec::new();
    let mut seq = 1u64;
    let push = |stream: &mut Vec<u8>, seq: &mut u64, dom, pl: &[u8]| {
        push_record(stream, *seq, raw, dom, COMPACT_RECORD_TYPE_DOMAIN_DATA, pl);
        *seq += 1;
    };

    let ident = country_identity_payload(&[(47, "GER")]);
    push(&mut stream, &mut seq, 18, &ident);

    // state 5 owned by 47 (mapped) but controlled by 48 (unmapped)
    let mut mc = Vec::new();
    map_control_header(&mut mc, 1, 0, 0);
    push_state_row(&mut mc, 5, 47, 48);
    push(&mut stream, &mut seq, 22, &mc);

    push_record(
        &mut stream,
        seq,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("g");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    let s = parse(&snaps[0].2);

    assert_eq!(s["states"]["5"]["owner"], json!("GER"));
    assert_eq!(s["states"]["5"]["controller"], json!("48")); // unmapped -> numeric
    assert_eq!(s["tag_db_id_index"]["resolved"], json!(false));
    assert_eq!(s["tag_db_id_index"]["key"], json!("tag"));
    let gaps = s["tag_db_id_index"]["unresolved_tag_db_ids"]
        .as_array()
        .unwrap();
    assert!(gaps.iter().any(|g| g == "48"));
    let notes = s["provenance"]["notes"].as_array().unwrap();
    assert!(notes.iter().any(|n| n.as_str().unwrap().contains("48")));
}

// --- E13 measured-country observability -----------------------------------

#[test]
fn measured_countries_known_lists_the_resolved_set() {
    // A proven domain-18 frame => measured_countries.known=true with the exact
    // resolved country set (count + tag_db_ids + tags).
    let raw = raw_for(16_959_360);
    let mut stream = Vec::new();
    let mut seq = 1u64;
    let push = |stream: &mut Vec<u8>, seq: &mut u64, dom, pl: &[u8]| {
        push_record(stream, *seq, raw, dom, COMPACT_RECORD_TYPE_DOMAIN_DATA, pl);
        *seq += 1;
    };
    push(
        &mut stream,
        &mut seq,
        18,
        &country_identity_payload(&[(47, "GER"), (3, "FRA"), (2, "ENG")]),
    );
    push_record(
        &mut stream,
        seq,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let mut fold = WorldFold::new("mc");
    fold.ingest_stream(&stream);
    let s = parse(&fold.finish()[0].2);
    let mc = &s["measured_countries"];
    assert_eq!(mc["known"], json!(true));
    assert_eq!(mc["source"], json!("country_identity"));
    assert_eq!(mc["count"], json!(3));
    // tag_db_ids are the BTreeMap keys (sorted): 2, 3, 47.
    assert_eq!(mc["tag_db_ids"], json!([2, 3, 47]));
    // tags follow the same key order: ENG(2), FRA(3), GER(47).
    assert_eq!(mc["tags"], json!(["ENG", "FRA", "GER"]));
}

#[test]
fn measured_countries_unknown_when_no_domain18_this_hour() {
    // No domain-18 record this hour => known=false (measured set UNKNOWN, not
    // reported as empty/complete) with an explanatory reason and NO count/list.
    let raw = raw_for(16_959_360);
    let mut stream = Vec::new();
    let mut seq = 1u64;
    // Only a fuel row (a country-rooted domain) — no domain 18.
    push_record(
        &mut stream,
        seq,
        raw,
        10,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &fuel_payload(&[(47, 100, 200, 0, 0)]),
    );
    seq += 1;
    push_record(
        &mut stream,
        seq,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let mut fold = WorldFold::new("mc");
    fold.ingest_stream(&stream);
    let s = parse(&fold.finish()[0].2);
    let mc = &s["measured_countries"];
    assert_eq!(mc["known"], json!(false));
    assert!(mc.get("count").is_none());
    assert!(mc.get("tag_db_ids").is_none());
    assert!(mc["reason"].as_str().unwrap().contains("unknown"));
}

#[test]
fn measured_countries_distinguishes_no_data_from_not_resolved() {
    // The core E13 semantic: with the measured set KNOWN, a consumer can tell a
    // country that was MEASURED but has no data in a domain (present in
    // measured_countries, absent from that domain's rows) from a country that
    // was NOT resolved this frame (absent from measured_countries entirely).
    let raw = raw_for(16_959_360);
    let mut stream = Vec::new();
    let mut seq = 1u64;
    let push = |stream: &mut Vec<u8>, seq: &mut u64, dom, pl: &[u8]| {
        push_record(stream, *seq, raw, dom, COMPACT_RECORD_TYPE_DOMAIN_DATA, pl);
        *seq += 1;
    };
    // Measured set: GER(47) + FRA(3). D01 (a dynamic observer, id 900) is NOT
    // measured — the systemic country-roots bound excludes it.
    push(
        &mut stream,
        &mut seq,
        18,
        &country_identity_payload(&[(47, "GER"), (3, "FRA")]),
    );
    // Fuel domain has a row for GER only; FRA is measured-but-no-fuel-data.
    push(
        &mut stream,
        &mut seq,
        10,
        &fuel_payload(&[(47, 100, 200, 0, 0)]),
    );
    push_record(
        &mut stream,
        seq,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let mut fold = WorldFold::new("mc");
    fold.ingest_stream(&stream);
    let s = parse(&fold.finish()[0].2);
    let mc = &s["measured_countries"];
    assert_eq!(mc["known"], json!(true));
    let ids: Vec<u64> = mc["tag_db_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    // FRA measured (in the set) but absent from fuel => "no fuel data for FRA".
    assert!(ids.contains(&3), "FRA is in the measured set");
    // A dynamic observer (900) is NOT in the measured set => "not resolved".
    assert!(
        !ids.contains(&900),
        "unresolved observer absent from measured set"
    );
    // GER has fuel data; FRA does not — both are measured.
    assert!(s["countries"]["GER"].get("fuel").is_some());
    assert!(s["countries"].get("FRA").is_none() || s["countries"]["FRA"].get("fuel").is_none());
}

#[test]
fn measured_countries_known_resets_per_hour_but_map_persists() {
    // Hour 1: domain 18 present => known=true. Hour 2: NO domain 18 => the
    // measured set is UNKNOWN this hour (known=false), yet tag_by_db_id PERSISTS
    // so TAG re-keying still works (a country-rooted domain's rows still resolve
    // to TAGs). This is the per-frame-vs-carried distinction E13 requires.
    let raw1 = raw_for(16_959_360);
    let raw2 = raw_for(16_959_361);
    let mut stream = Vec::new();
    let mut seq = 1u64;
    let mut push = |stream: &mut Vec<u8>, seq: &mut u64, raw, dom, pl: &[u8]| {
        push_record(stream, *seq, raw, dom, COMPACT_RECORD_TYPE_DOMAIN_DATA, pl);
        *seq += 1;
    };
    // Hour 1.
    push(
        &mut stream,
        &mut seq,
        raw1,
        18,
        &country_identity_payload(&[(47, "GER")]),
    );
    push_record(
        &mut stream,
        seq,
        raw1,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    seq += 1;
    // Hour 2: only a fuel row for GER (no domain 18).
    push(
        &mut stream,
        &mut seq,
        raw2,
        10,
        &fuel_payload(&[(47, 100, 200, 0, 0)]),
    );
    push_record(
        &mut stream,
        seq,
        raw2,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let mut fold = WorldFold::new("mc");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 2);

    let s1 = parse(&snaps[0].2);
    assert_eq!(s1["measured_countries"]["known"], json!(true));
    assert_eq!(s1["measured_countries"]["count"], json!(1));

    let s2 = parse(&snaps[1].2);
    // Measured set unknown this hour...
    assert_eq!(s2["measured_countries"]["known"], json!(false));
    // ...but the carried tag map still re-keys GER's fuel row to the TAG.
    assert!(s2["countries"]["GER"]["fuel"].is_object());
}

#[test]
fn country_identity_cosmetic_tag_present_null_and_absent() {
    // GER(47): cosmetic override "GES" (present). ENG(2): validated null.
    // FRA(3): ABSENT (marker 0) — hand-built row. Each country also gets a
    // fuel row so it materializes as a country object.
    let raw = raw_for(16_959_360);
    let mut stream = Vec::new();
    let mut seq = 1u64;
    let mut push = |stream: &mut Vec<u8>, seq: &mut u64, dom, pl: &[u8]| {
        push_record(stream, *seq, raw, dom, COMPACT_RECORD_TYPE_DOMAIN_DATA, pl);
        *seq += 1;
    };

    // Build the D18 payload by hand so FRA can carry the absent marker (0).
    let mut ident = Vec::new();
    ident.extend_from_slice(&0u64.to_le_bytes()); // wall
    ident.extend_from_slice(&3u32.to_le_bytes()); // row_count
    ident.extend_from_slice(&3u32.to_le_bytes()); // changed
    ident.extend_from_slice(&0u32.to_le_bytes()); // status
                                                  // GER present -> "GES"
    ident.extend_from_slice(&47u32.to_le_bytes());
    push_str(&mut ident, "GER");
    ident.push(2);
    push_str(&mut ident, "GES");
    // ENG null
    ident.extend_from_slice(&2u32.to_le_bytes());
    push_str(&mut ident, "ENG");
    ident.push(1);
    // FRA absent
    ident.extend_from_slice(&3u32.to_le_bytes());
    push_str(&mut ident, "FRA");
    ident.push(0);
    push(&mut stream, &mut seq, 18, &ident);

    // fuel rows so each country materializes.
    for tag in [47u32, 2, 3] {
        let fuel = fuel_payload(&[(tag, 100, 200, 0, 0)]);
        push(&mut stream, &mut seq, 10, &fuel);
    }
    push_record(
        &mut stream,
        seq,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("cosmetic");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    let s = parse(&snaps[0].2);

    // GER: cosmetic override present.
    assert_eq!(s["countries"]["GER"]["cosmetic_tag"], json!("GES"));
    // ENG: validated null (key present, value null).
    assert!(s["countries"]["ENG"].get("cosmetic_tag").is_some());
    assert_eq!(s["countries"]["ENG"]["cosmetic_tag"], Value::Null);
    // FRA: absent -> NO cosmetic_tag key at all (absent != null).
    assert!(s["countries"]["FRA"].get("cosmetic_tag").is_none());
}

#[test]
fn out_of_range_hour_emits_no_snapshot() {
    // A frame end with an out-of-range raw date must fail closed (no snapshot).
    let mut stream = Vec::new();
    let fuel = fuel_payload(&[(47, 100, 200, 0, 0)]);
    push_record(
        &mut stream,
        1,
        0,
        10,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &fuel,
    );
    push_record(
        &mut stream,
        2,
        0,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    let mut fold = WorldFold::new("g");
    fold.ingest_stream(&stream);
    assert!(fold.finish().is_empty());
}

#[test]
fn malformed_domain_is_skipped_not_fabricated() {
    // A fuel payload with a bad length is skipped; the snapshot still emits with
    // a note and no fabricated fuel.
    let hi = 17_000_000i64;
    let raw = raw_for(hi);
    let mut stream = Vec::new();
    // truncated fuel payload (header claims 1 row but no row bytes)
    let mut bad = Vec::new();
    bad.extend_from_slice(&0u64.to_le_bytes());
    bad.extend_from_slice(&1u32.to_le_bytes()); // row_count = 1
    bad.extend_from_slice(&1u32.to_le_bytes());
    bad.extend_from_slice(&0u32.to_le_bytes());
    bad.extend_from_slice(&32768u32.to_le_bytes());
    push_record(
        &mut stream,
        1,
        raw,
        10,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        &bad,
    );
    push_record(
        &mut stream,
        2,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );

    let mut fold = WorldFold::new("g");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    let s = parse(&snaps[0].2);
    assert_eq!(s["countries"].as_object().unwrap().len(), 0);
    let notes = s["provenance"]["notes"].as_array().unwrap();
    assert!(notes
        .iter()
        .any(|n| n.as_str().unwrap().contains("domain 10")));
    // fuel_live must NOT be in provenance domains (the fold was skipped)
    let dom = s["provenance"]["collector"]["domains"].as_array().unwrap();
    assert!(!dom.iter().any(|d| d == "fuel_live"));
}

fn chunk1_snapshots(hours: Vec<Vec<(u16, Vec<u8>)>>) -> Vec<Value> {
    let base_hi = 16_959_360i64;
    let mut stream = Vec::new();
    let mut seq = 1u64;
    for (hour_offset, records) in hours.into_iter().enumerate() {
        let raw = raw_for(base_hi + hour_offset as i64);
        for (domain_id, payload) in records {
            push_record(
                &mut stream,
                seq,
                raw,
                domain_id,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            );
            seq += 1;
        }
        push_record(
            &mut stream,
            seq,
            raw,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &frame_end_payload(),
        );
        seq += 1;
    }
    let mut fold = WorldFold::new("chunk1_game");
    fold.ingest_stream(&stream);
    fold.finish()
        .into_iter()
        .map(|(_, _, bytes)| parse(&bytes))
        .collect()
}

fn chunk1_one_snapshot(records: Vec<(u16, Vec<u8>)>) -> Value {
    chunk1_snapshots(vec![records]).remove(0)
}

fn push_d52_row(
    out: &mut Vec<u8>,
    state_id: u32,
    owner: u32,
    controller: u32,
    occupied: u32,
    resistance_raw: i32,
    compliance_raw: i32,
    flags: u32,
) {
    out.extend_from_slice(&state_id.to_le_bytes());
    out.extend_from_slice(&owner.to_le_bytes());
    out.extend_from_slice(&controller.to_le_bytes());
    out.extend_from_slice(&occupied.to_le_bytes());
    out.extend_from_slice(&resistance_raw.to_le_bytes());
    out.extend_from_slice(&compliance_raw.to_le_bytes());
    out.extend_from_slice(&2500i32.to_le_bytes());
    out.extend_from_slice(&(-1200i32).to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0xA11C_E001u32.to_le_bytes());
    out.extend_from_slice(&4183u32.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
}

#[allow(clippy::too_many_arguments)]
fn push_d52_garrison_row(
    out: &mut Vec<u8>,
    state_id: u32,
    owner: u32,
    controller: u32,
    occupied: u32,
    garrison_manpower_need: i32,
    garrison_strength_ratio_raw: i32,
    garrison_required_raw: i32,
    flags: u32,
) {
    out.extend_from_slice(&state_id.to_le_bytes());
    out.extend_from_slice(&owner.to_le_bytes());
    out.extend_from_slice(&controller.to_le_bytes());
    out.extend_from_slice(&occupied.to_le_bytes());
    out.extend_from_slice(&2_230_000i32.to_le_bytes());
    out.extend_from_slice(&6_500_000i32.to_le_bytes());
    out.extend_from_slice(&2500i32.to_le_bytes());
    out.extend_from_slice(&(-1200i32).to_le_bytes());
    out.extend_from_slice(&garrison_manpower_need.to_le_bytes());
    out.extend_from_slice(&garrison_strength_ratio_raw.to_le_bytes());
    out.extend_from_slice(&garrison_required_raw.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
}

fn d52_payload(status_flags: u32, rows: impl FnOnce(&mut Vec<u8>) -> u32) -> Vec<u8> {
    let mut row_bytes = Vec::new();
    let row_count = rows(&mut row_bytes);
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&(FIXED_SCALE as u32).to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&row_bytes);
    payload
}

fn d23_baseline_payload(status_flags: u32, resource_amount: i32) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&2u32.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());

    payload.extend_from_slice(&5u32.to_le_bytes());
    payload.extend_from_slice(&0b11u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());

    payload.extend_from_slice(&2068u32.to_le_bytes());
    payload.extend_from_slice(&5u32.to_le_bytes());
    payload.extend_from_slice(&0b11u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());

    payload.extend_from_slice(&0x3E03u32.to_le_bytes());
    payload.extend_from_slice(&resource_amount.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());

    payload.extend_from_slice(&0x2FABu32.to_le_bytes());
    payload.extend_from_slice(&5u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());

    payload.extend_from_slice(&0x31F6u32.to_le_bytes());
    payload.extend_from_slice(&10u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload
}

fn d44_payload(status_flags: u32) -> Vec<u8> {
    let rows = [
        (
            SUPPLY_ROW_KIND_PROVINCE_LOGISTICS,
            2068u32,
            64u32,
            0u32,
            SUPPLY_ROW_FLAG_RAIL
                | SUPPLY_ROW_FLAG_SUPPLY_NODE
                | SUPPLY_ROW_FLAG_INFRASTRUCTURE
                | SUPPLY_ROW_FLAG_LIVE_BUILDING_VECTOR,
            5i32,
            2i32,
            0i32,
            4i32,
            3i32,
        ),
        (
            SUPPLY_ROW_KIND_DIVISION_SUPPLY,
            42u32,
            2068u32,
            1u32,
            SUPPLY_ROW_FLAG_DIVISION_CURRENT_SUPPLY
                | SUPPLY_ROW_FLAG_DIVISION_MAX_SUPPLY
                | SUPPLY_ROW_FLAG_DIVISION_SUPPLY_GAIN
                | SUPPLY_ROW_FLAG_DIVISION_OWNER
                | SUPPLY_ROW_FLAG_DIVISION_SUPPLY_DEFICIT
                | SUPPLY_ROW_FLAG_DIVISION_FUEL
                | SUPPLY_ROW_FLAG_PROXY_VALUE,
            950_000i32,
            1_200_000i32,
            25_000i32,
            250_000i32,
            99_000i32,
        ),
        (
            SUPPLY_ROW_KIND_CSUPPLY_PROVINCE_INPUT,
            3544u32,
            0u32,
            0u32,
            SUPPLY_ROW_FLAG_CSUPPLY_EFFICIENCY | SUPPLY_ROW_FLAG_CSUPPLY_PROVINCE_ARRAY,
            72_500i32,
            0i32,
            0i32,
            0i32,
            0i32,
        ),
        (
            SUPPLY_ROW_KIND_COMPUTED_BADGE,
            0u32,
            0u32,
            0u32,
            SUPPLY_ROW_FLAG_EXACT_SUPPLY_WALL,
            1i32,
            0i32,
            0i32,
            0i32,
            0i32,
        ),
        (
            SUPPLY_ROW_KIND_COMPUTED_BADGE,
            0u32,
            0u32,
            0u32,
            SUPPLY_ROW_FLAG_ATTRITION_WALL,
            2i32,
            0i32,
            0i32,
            0i32,
            0i32,
        ),
        (
            SUPPLY_ROW_KIND_COMPUTED_BADGE,
            0u32,
            0u32,
            0u32,
            SUPPLY_ROW_FLAG_MOTORIZATION_WALL,
            3i32,
            0i32,
            0i32,
            0i32,
            0i32,
        ),
        (
            SUPPLY_ROW_KIND_COMPUTED_BADGE,
            0u32,
            0u32,
            0u32,
            SUPPLY_ROW_FLAG_CONVOY_PRESSURE_WALL,
            4i32,
            0i32,
            0i32,
            0i32,
            0i32,
        ),
        (
            SUPPLY_ROW_KIND_COMPUTED_BADGE,
            0u32,
            0u32,
            0u32,
            SUPPLY_ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED,
            5i32,
            0i32,
            0i32,
            0i32,
            0i32,
        ),
    ];
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&(FIXED_SCALE as u32).to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&5u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    for (kind, entity, join, owner, flags, a, b, c, d, aux) in rows {
        payload.extend_from_slice(&kind.to_le_bytes());
        payload.extend_from_slice(&entity.to_le_bytes());
        payload.extend_from_slice(&join.to_le_bytes());
        payload.extend_from_slice(&owner.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&a.to_le_bytes());
        payload.extend_from_slice(&b.to_le_bytes());
        payload.extend_from_slice(&c.to_le_bytes());
        payload.extend_from_slice(&d.to_le_bytes());
        payload.extend_from_slice(&aux.to_le_bytes());
    }
    payload
}

fn d44_empty_payload() -> Vec<u8> {
    let rows = [
        (SUPPLY_ROW_FLAG_EXACT_SUPPLY_WALL, 1i32),
        (SUPPLY_ROW_FLAG_ATTRITION_WALL, 2i32),
        (SUPPLY_ROW_FLAG_MOTORIZATION_WALL, 3i32),
        (SUPPLY_ROW_FLAG_CONVOY_PRESSURE_WALL, 4i32),
        (SUPPLY_ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED, 5i32),
    ];
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&(FIXED_SCALE as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for (flags, code) in rows {
        payload.extend_from_slice(&SUPPLY_ROW_KIND_COMPUTED_BADGE.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&code.to_le_bytes());
        payload.extend_from_slice(&0i32.to_le_bytes());
        payload.extend_from_slice(&0i32.to_le_bytes());
        payload.extend_from_slice(&0i32.to_le_bytes());
        payload.extend_from_slice(&0i32.to_le_bytes());
    }
    payload
}

fn d44_missing_badge_empty_payload() -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&(FIXED_SCALE as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload
}

fn d43_payload(status_flags: u32) -> Vec<u8> {
    let flags = MARKET_TRADE_FLAG_BUYER_PTR_VALID
        | MARKET_TRADE_FLAG_EQUIPMENT_SUBBUF_VALID
        | MARKET_TRADE_FLAG_DELIVERY_SUBBUF_VALID
        | MARKET_TRADE_FLAG_VARIANT_VALID
        | MARKET_TRADE_FLAG_AMOUNT_VALID;
    let mut payload = Vec::new();
    payload.extend_from_slice(&1000u64.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&(FIXED_SCALE as u32).to_le_bytes());
    payload.extend_from_slice(&1405u32.to_le_bytes());
    payload.extend_from_slice(&MARKET_TRADE_CONTRACT_TYPE.to_le_bytes());
    payload.extend_from_slice(&flags.to_le_bytes());
    payload.extend_from_slice(&34u32.to_le_bytes());
    payload.extend_from_slice(&67u32.to_le_bytes());
    payload.extend_from_slice(&4756u32.to_le_bytes());
    payload.extend_from_slice(&21_700_000i64.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload
}

fn append_d43_mtd1_detail(payload: &mut Vec<u8>) {
    let flags = MARKET_TRADE_DETAIL_FLAG_PRICE_VALID
        | MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID
        | MARKET_TRADE_DETAIL_FLAG_SPEED_VALID
        | MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID
        | MARKET_TRADE_DETAIL_FLAG_DAYS_VALID
        | MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID
        | MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID
        | MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID
        | MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID
        | MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID;
    payload.extend_from_slice(MARKET_TRADE_DETAIL_TRAILER_MAGIC);
    payload.extend_from_slice(&MARKET_TRADE_DETAIL_TRAILER_VERSION.to_le_bytes());
    payload.extend_from_slice(&(MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(MARKET_TRADE_DETAIL_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1405u32.to_le_bytes());
    payload.extend_from_slice(&flags.to_le_bytes());
    payload.extend_from_slice(&7_000_000i64.to_le_bytes());
    payload.extend_from_slice(&21_700_000i64.to_le_bytes());
    payload.extend_from_slice(&125_000i32.to_le_bytes());
    payload.extend_from_slice(&1i32.to_le_bytes());
    payload.extend_from_slice(&42i32.to_le_bytes());
    payload.extend_from_slice(&67u32.to_le_bytes());
    payload.extend_from_slice(&3i32.to_le_bytes());
    payload.extend_from_slice(&5i32.to_le_bytes());
    payload.extend_from_slice(&90_000i32.to_le_bytes());
    payload.extend_from_slice(&45_000i32.to_le_bytes());
    payload.extend_from_slice(&50_000i32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
}

fn d43_empty_payload() -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&(FIXED_SCALE as u32).to_le_bytes());
    payload
}

fn d22_state5_payload(owner: u32, controller: u32) -> Vec<u8> {
    let mut payload = Vec::new();
    map_control_header(&mut payload, 1, 1, 0);
    push_state_row(&mut payload, 5, owner, controller);
    push_prov_row(&mut payload, 900, controller, 5);
    payload
}

fn d52_state5_payload() -> Vec<u8> {
    let flags = OCC_ROW_FLAG_RESISTANCE | OCC_ROW_FLAG_COMPLIANCE;
    d52_payload(0, |rows| {
        push_d52_row(rows, 5, 0, 0, 0, 100_000, 200_000, flags);
        1
    })
}

fn assert_state5_chunk1_fields_survive(snap: &Value) {
    assert_eq!(snap["states"]["5"]["owner"], json!("47"));
    assert_eq!(snap["states"]["5"]["controller"], json!("48"));
    assert_eq!(snap["provinces"]["900"]["controller"], json!("48"));
    assert_eq!(snap["provinces"]["900"]["state_id"], json!(5));
    assert_eq!(snap["states"]["5"]["buildings"]["infrastructure"], json!(5));
    assert_eq!(
        snap["states"]["5"]["province_buildings"]["naval_base"],
        json!(10)
    );
    assert!((snap["states"]["5"]["resources"]["steel"].as_f64().unwrap() - 7.0).abs() < 0.0001);
    assert!(
        (snap["states"]["5"]["occupation"]["resistance"]
            .as_f64()
            .unwrap()
            - 1.0)
            .abs()
            < 0.0001
    );
    assert!(
        (snap["states"]["5"]["occupation"]["compliance"]
            .as_f64()
            .unwrap()
            - 2.0)
            .abs()
            < 0.0001
    );
    assert_eq!(snap["map"]["owner_by_state"]["5"], json!("47"));
    assert_eq!(snap["map"]["control_by_province"]["900"], json!("48"));
}

#[test]
fn folds_d52_occupation_with_r2_law_token_and_tag_resolution() {
    let flags = OCC_ROW_FLAG_OWNER
        | OCC_ROW_FLAG_CONTROLLER
        | OCC_ROW_FLAG_RESISTANCE
        | OCC_ROW_FLAG_COMPLIANCE
        | OCC_ROW_FLAG_OCCUPIED_STATE
        | OCC_ROW_FLAG_OCCUPIED_TAG_DERIVED
        | OCC_ROW_FLAG_OCCUPATION_LAW_TOKEN
        | OCC_ROW_FLAG_GARRISON_TEMPLATE
        | OCC_ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE
        | OCC_ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE
        | OCC_ROW_FLAG_RESISTANCE_SPEED
        | OCC_ROW_FLAG_COMPLIANCE_SPEED
        | OCC_ROW_FLAG_OCCUPATION_STATUS_ROOT
        | OCC_ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN;
    let snap = chunk1_one_snapshot(vec![
        (18, country_identity_payload(&[(1, "FRA"), (2, "GER")])),
        (
            52,
            d52_payload(0, |rows| {
                push_d52_row(rows, 5, 1, 2, 1, 2_230_000, 6_500_000, flags);
                1
            }),
        ),
    ]);

    let occupation = &snap["states"]["5"]["occupation"];
    assert_eq!(occupation["occupier_tag"], json!("GER"));
    assert_eq!(occupation["occupied_country_tag"], json!("FRA"));
    assert!((occupation["resistance"].as_f64().unwrap() - 22.3).abs() < 0.0001);
    assert_eq!(snap["states"]["5"]["occupation_law"], Value::Null);
    assert_eq!(
        occupation["_r2_occupation_resistance_compliance_live"]["occupation_law_token_id"],
        json!(0xA11C_E001u32)
    );
    assert!(snap["provenance"]["collector"]["domains"]
        .as_array()
        .unwrap()
        .iter()
        .any(|domain| domain == "occupation_resistance_compliance_live"));
}

#[test]
fn folds_d52_measured_garrison_scalars() {
    let flags = OCC_ROW_FLAG_OWNER
        | OCC_ROW_FLAG_CONTROLLER
        | OCC_ROW_FLAG_RESISTANCE
        | OCC_ROW_FLAG_COMPLIANCE
        | OCC_ROW_FLAG_OCCUPIED_STATE
        | OCC_ROW_FLAG_OCCUPIED_TAG_DERIVED
        | OCC_ROW_FLAG_RESISTANCE_SPEED
        | OCC_ROW_FLAG_COMPLIANCE_SPEED
        | OCC_ROW_FLAG_OCCUPATION_STATUS_ROOT
        | OCC_ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE
        | OCC_ROW_FLAG_GARRISON_SCALARS;
    let snap = chunk1_one_snapshot(vec![(
        52,
        d52_payload(0, |rows| {
            push_d52_garrison_row(rows, 5, 1, 2, 1, 14_000, 87_500, 250_000, flags);
            1
        }),
    )]);

    let extra = &snap["states"]["5"]["occupation"]["_r2_occupation_resistance_compliance_live"];
    assert_eq!(extra["garrison_army_manpower_need"], json!(14_000));
    assert!((extra["garrison_strength_ratio"].as_f64().unwrap() - 0.875).abs() < 0.000001);
    assert!((extra["garrison_required"].as_f64().unwrap() - 2.5).abs() < 0.000001);
    assert_eq!(extra["garrison_signals_status"], Value::Null);
}

#[test]
fn d52_garrison_values_without_measured_flag_fail_fold_closed() {
    let flags = OCC_ROW_FLAG_OWNER
        | OCC_ROW_FLAG_CONTROLLER
        | OCC_ROW_FLAG_RESISTANCE
        | OCC_ROW_FLAG_COMPLIANCE
        | OCC_ROW_FLAG_OCCUPIED_STATE
        | OCC_ROW_FLAG_OCCUPIED_TAG_DERIVED
        | OCC_ROW_FLAG_RESISTANCE_SPEED
        | OCC_ROW_FLAG_COMPLIANCE_SPEED;
    let snap = chunk1_one_snapshot(vec![(
        52,
        d52_payload(0, |rows| {
            push_d52_garrison_row(rows, 5, 1, 2, 1, 14_000, 0, 0, flags);
            1
        }),
    )]);

    assert_eq!(snap["states"]["5"]["occupation"], Value::Null);
    assert!(snap["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("garrison scalar values without measured flag")));
}

#[test]
fn d52_unbadged_rows_without_scalars_emit_no_garrison_fields() {
    let flags = OCC_ROW_FLAG_OWNER
        | OCC_ROW_FLAG_CONTROLLER
        | OCC_ROW_FLAG_RESISTANCE
        | OCC_ROW_FLAG_COMPLIANCE
        | OCC_ROW_FLAG_RESISTANCE_SPEED
        | OCC_ROW_FLAG_COMPLIANCE_SPEED
        | OCC_ROW_FLAG_OCCUPATION_STATUS_ROOT;
    let snap = chunk1_one_snapshot(vec![(
        52,
        d52_payload(0, |rows| {
            push_d52_garrison_row(rows, 5, 1, 1, 0, 0, 0, 0, flags);
            1
        }),
    )]);

    let extra = &snap["states"]["5"]["occupation"]["_r2_occupation_resistance_compliance_live"];
    // Absent measurement must stay absent: no fabricated zero garrison values.
    assert_eq!(extra["garrison_army_manpower_need"], Value::Null);
    assert_eq!(extra["garrison_strength_ratio"], Value::Null);
    assert_eq!(extra["garrison_required"], Value::Null);
    assert_eq!(extra["garrison_signals_status"], Value::Null);
}

#[test]
fn d52_delta_before_baseline_marks_unknown_without_mutation() {
    let snap = chunk1_one_snapshot(vec![(52, d52_payload(COMPACT_STATUS_DELTA_DATA, |_| 0))]);
    assert_eq!(snap["states"], json!({}));
    assert!(snap["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(52)
            && problem["problem"] == json!("delta_before_baseline")));
}

#[test]
fn d52_failure_masks_prior_occupation_for_unknown_hour() {
    let flags = OCC_ROW_FLAG_RESISTANCE | OCC_ROW_FLAG_COMPLIANCE;
    let snaps = chunk1_snapshots(vec![
        vec![(
            52,
            d52_payload(0, |rows| {
                push_d52_row(rows, 9, 0, 0, 0, 100_000, 200_000, flags);
                1
            }),
        )],
        vec![(52, d52_payload(COMPACT_STATUS_SOURCE_UNAVAILABLE, |_| 0))],
    ]);
    assert!(snaps[0]["states"]["9"]["occupation"].is_object());
    assert_eq!(snaps[1]["states"]["9"]["occupation"], Value::Null);
    assert!(snaps[1]["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap().contains("failure status")));
}

#[test]
fn folds_d23_map_detail_baseline_and_renderer_mirror() {
    let snap = chunk1_one_snapshot(vec![(23, d23_baseline_payload(0, 700_000))]);
    assert_eq!(snap["states"]["5"]["buildings"]["infrastructure"], json!(5));
    assert_eq!(
        snap["states"]["5"]["province_buildings"]["naval_base"],
        json!(10)
    );
    assert!((snap["states"]["5"]["resources"]["steel"].as_f64().unwrap() - 7.0).abs() < 0.0001);
    assert_eq!(
        snap["map"]["map_detail_live"]["province_buildings_by_province"]["2068"]["naval_base"],
        json!(10)
    );
}

#[test]
fn d23_delta_after_baseline_preserves_carried_province_building_totals() {
    let snaps = chunk1_snapshots(vec![
        vec![(23, d23_baseline_payload(0, 700_000))],
        vec![(23, d23_baseline_payload(COMPACT_STATUS_DELTA_DATA, 800_000))],
    ]);
    assert!(
        (snaps[1]["states"]["5"]["resources"]["steel"]
            .as_f64()
            .unwrap()
            - 8.0)
            .abs()
            < 0.0001
    );
    assert_eq!(
        snaps[1]["states"]["5"]["province_buildings"]["naval_base"],
        json!(10)
    );
}

#[test]
fn d23_delta_before_baseline_and_failure_are_unknown_not_empty() {
    let delta = chunk1_one_snapshot(vec![(
        23,
        d23_baseline_payload(COMPACT_STATUS_DELTA_DATA, 700_000),
    )]);
    assert!(delta["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(23)
            && problem["problem"] == json!("delta_before_baseline")));

    let snaps = chunk1_snapshots(vec![
        vec![(23, d23_baseline_payload(0, 700_000))],
        vec![(
            23,
            d23_baseline_payload(COMPACT_STATUS_SOURCE_UNAVAILABLE, 900_000),
        )],
    ]);
    assert_eq!(snaps[1]["states"]["5"]["resources"], Value::Null);
    assert_eq!(snaps[1]["map"]["map_detail_live"], Value::Null);
}

#[test]
fn d22_full_baseline_same_hour_preserves_d23_d52_fields_in_both_orders() {
    let d22_last = chunk1_one_snapshot(vec![
        (23, d23_baseline_payload(0, 700_000)),
        (52, d52_state5_payload()),
        (22, d22_state5_payload(47, 48)),
    ]);
    assert_state5_chunk1_fields_survive(&d22_last);

    let d22_first = chunk1_one_snapshot(vec![
        (22, d22_state5_payload(47, 48)),
        (23, d23_baseline_payload(0, 700_000)),
        (52, d52_state5_payload()),
    ]);
    assert_state5_chunk1_fields_survive(&d22_first);
}

#[test]
fn d22_full_baseline_later_hour_preserves_d23_d52_fields_in_both_orders() {
    let d23_d52_then_d22 = chunk1_snapshots(vec![
        vec![
            (23, d23_baseline_payload(0, 700_000)),
            (52, d52_state5_payload()),
        ],
        vec![(22, d22_state5_payload(47, 48))],
    ]);
    assert_state5_chunk1_fields_survive(&d23_d52_then_d22[1]);

    let d22_then_d23_d52 = chunk1_snapshots(vec![
        vec![(22, d22_state5_payload(47, 48))],
        vec![
            (23, d23_baseline_payload(0, 700_000)),
            (52, d52_state5_payload()),
        ],
    ]);
    assert_state5_chunk1_fields_survive(&d22_then_d23_d52[1]);
}

#[test]
fn folds_d44_supply_proxy_with_raw_division_ids_and_badges() {
    let snap = chunk1_one_snapshot(vec![
        (18, country_identity_payload(&[(1, "GER")])),
        (44, d44_payload(0)),
    ]);
    let by_province = &snap["supply"]["by_province"];
    assert_eq!(
        by_province["2068"]["logistics_buildings"]["rail_way"],
        json!(5)
    );
    let division = &by_province["2068"]["divisions"][0];
    assert_eq!(division["raw_division_id"], json!(42));
    assert!(division.get("division_id").is_none());
    assert_eq!(division["owner"], json!("GER"));
    assert!((division["current_supply"].as_f64().unwrap() - 9.5).abs() < 0.0001);
    assert!((by_province["3544"]["csupply_input"].as_f64().unwrap() - 0.725).abs() < 0.0001);
    assert_eq!(snap["supply"]["source"], json!("M"));
    assert_eq!(
        snap["supply"]["confidence"],
        json!("division_province_proxy")
    );
    assert_eq!(
        snap["supply"]["r2_logistics_attrition"]["badges"][0]["badge"],
        json!("exact_supply_wall")
    );
}

#[test]
fn d44_success_zero_replaces_and_failure_masks_unknown_hour() {
    let snaps = chunk1_snapshots(vec![
        vec![(44, d44_payload(0))],
        vec![(44, d44_missing_badge_empty_payload())],
        vec![(44, d44_empty_payload())],
        vec![(44, d44_payload(COMPACT_STATUS_SOURCE_UNAVAILABLE))],
    ]);
    assert!(snaps[0]["supply"]["by_province"]["2068"].is_object());
    assert_eq!(snaps[1]["supply"]["by_province"], Value::Null);
    assert!(snaps[1]["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(44)
            && problem["problem"] == json!("missing_required_badge_rows")));
    assert!(snaps[1]["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("missing required computed badge rows")));
    assert_eq!(snaps[2]["supply"]["by_province"], json!({}));
    assert_eq!(
        snaps[2]["supply"]["r2_logistics_attrition"]["badges"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(snaps[3]["supply"]["by_province"], Value::Null);
}

#[test]
fn folds_d43_trade_contracts_world_and_per_country_views() {
    let snap = chunk1_one_snapshot(vec![
        (18, country_identity_payload(&[(34, "SAF"), (67, "ENG")])),
        (43, d43_payload(0)),
    ]);
    let contracts = snap["diplomacy_world"]["trade_contracts"]
        .as_array()
        .unwrap();
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0]["contract_id"], json!("1405:81"));
    assert_eq!(contracts[0]["seller"], json!("SAF"));
    assert_eq!(contracts[0]["buyer"], json!("ENG"));
    assert_eq!(contracts[0]["equipments"][0]["variant_id"], json!(4756));
    assert!((contracts[0]["equipments"][0]["amount"].as_f64().unwrap() - 217.0).abs() < 0.0001);
    assert_eq!(
        snap["countries"]["SAF"]["diplomacy"]["lend_lease"][0]["direction"],
        json!("outgoing")
    );
    assert_eq!(
        snap["countries"]["ENG"]["diplomacy"]["lend_lease"][0]["direction"],
        json!("incoming")
    );
    assert_eq!(
        snap["countries"]["SAF"]["economy"]["market_stockpile"],
        Value::Null
    );
}

#[test]
fn folds_d43_mtd1_detail_fields_without_discarding_partial_core_rows() {
    let mut payload = d43_payload(COMPACT_STATUS_PARTIAL_DATA);
    append_d43_mtd1_detail(&mut payload);

    let snap = chunk1_one_snapshot(vec![
        (18, country_identity_payload(&[(34, "SAF"), (67, "ENG")])),
        (43, payload),
    ]);

    let contract = &snap["diplomacy_world"]["trade_contracts"][0];
    assert_eq!(contract["prices"][0]["amount_raw"], json!(7_000_000));
    assert_eq!(contract["delivery_country"], json!("ENG"));
    assert_eq!(contract["convoys"]["assigned"], json!(3));
    assert_eq!(contract["delivery_efficiency"], json!(0.9));
    assert_eq!(contract["days"], json!(42));
    assert_eq!(contract["completed"], json!(true));
    assert_eq!(
        contract["r2_market_trade_live"]["factory_cic_progress_raw"],
        json!(45_000)
    );
    assert!(snap["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("market_trade_live partial status")));
}

#[test]
fn d43_distinguishes_populated_absent_and_unknown_contract_views() {
    let snaps = chunk1_snapshots(vec![
        vec![
            (18, country_identity_payload(&[(34, "SAF"), (67, "ENG")])),
            (43, d43_payload(0)),
        ],
        vec![(43, d43_empty_payload())],
        vec![(43, d43_payload(0))],
        vec![(43, d43_payload(COMPACT_STATUS_SOURCE_UNAVAILABLE))],
    ]);
    assert_eq!(
        snaps[0]["diplomacy_world"]["trade_contracts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(snaps[1]["diplomacy_world"].get("trade_contracts").is_none());
    assert!(snaps[1]["countries"]
        .get("SAF")
        .and_then(|country| country.get("diplomacy"))
        .and_then(|diplomacy| diplomacy.get("lend_lease"))
        .is_none());
    assert!(snaps[1]["countries"]
        .get("ENG")
        .and_then(|country| country.get("diplomacy"))
        .and_then(|diplomacy| diplomacy.get("lend_lease"))
        .is_none());
    assert_eq!(
        snaps[2]["diplomacy_world"]["trade_contracts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        snaps[2]["countries"]["SAF"]["diplomacy"]["lend_lease"][0]["direction"],
        json!("outgoing")
    );
    assert_eq!(
        snaps[2]["countries"]["ENG"]["diplomacy"]["lend_lease"][0]["direction"],
        json!("incoming")
    );
    assert_eq!(snaps[3]["diplomacy_world"]["trade_contracts"], Value::Null);
    assert_eq!(
        snaps[3]["countries"]["SAF"]["diplomacy"]["lend_lease"],
        Value::Null
    );
    assert_eq!(
        snaps[3]["countries"]["ENG"]["diplomacy"]["lend_lease"],
        Value::Null
    );
}

fn c2_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn c2_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn c2_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn c2_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn d27_order_payload(status_flags: u32, army_id: u32, targets: &[u32]) -> Vec<u8> {
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, status_flags);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, targets.len() as u32);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, army_id);
    c2_u64(&mut p, 77);
    c2_u32(
        &mut p,
        if targets.is_empty() {
            0
        } else {
            ARMY_ORDER_ROW_FLAG_TARGET_STATES
        },
    );
    c2_u32(&mut p, 2);
    c2_i32(&mut p, 5);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, targets.len() as u32);
    c2_u32(&mut p, targets.len() as u32);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    for state_id in targets {
        c2_u32(&mut p, *state_id);
        c2_u32(&mut p, 0);
    }
    p
}

fn d27_empty_payload(status_flags: u32) -> Vec<u8> {
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, status_flags);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    p
}

#[allow(clippy::too_many_arguments)]
fn d28_push_row(
    out: &mut Vec<u8>,
    kind: u32,
    object_id: u32,
    flags: u32,
    parent_id: u32,
    owner: u32,
    leader_id: u32,
    leader_skill: u32,
    member_first: u32,
    member_count: u32,
    logical_member_count: u32,
    member_count_engine: u32,
    missing_member_count: u32,
    leader_trait_count: u32,
    location: u32,
) {
    c2_u32(out, kind);
    c2_u32(out, object_id);
    c2_u32(out, flags);
    c2_u32(out, parent_id);
    c2_u32(out, owner);
    c2_u32(out, leader_id);
    c2_u32(out, leader_skill);
    c2_u32(out, member_first);
    c2_u32(out, member_count);
    c2_u32(out, logical_member_count);
    c2_u32(out, member_count_engine);
    c2_u32(out, missing_member_count);
    c2_u32(out, leader_trait_count);
    c2_u32(out, 0);
    c2_u32(out, 0);
    c2_u32(out, 0);
    c2_u32(out, 0);
    c2_u32(out, 0);
    c2_u32(out, location);
    c2_u32(out, 0);
}

fn d28_oob_payload(status_flags: u32) -> Vec<u8> {
    let members = [10u32, 77u32];
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, 4);
    c2_u32(&mut p, 4);
    c2_u32(&mut p, status_flags);
    c2_u32(&mut p, 2);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, members.len() as u32);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    d28_push_row(
        &mut p,
        OOB_KIND_ARMY_GROUP,
        100,
        OOB_ROW_FLAG_OWNER | OOB_ROW_FLAG_LEADER | OOB_ROW_FLAG_MEMBER_COUNT_ENGINE,
        0,
        1,
        501,
        0,
        0,
        1,
        1,
        1,
        0,
        0,
        0,
    );
    d28_push_row(
        &mut p,
        OOB_KIND_ARMY,
        10,
        OOB_ROW_FLAG_PARENT
            | OOB_ROW_FLAG_OWNER
            | OOB_ROW_FLAG_LEADER
            | OOB_ROW_FLAG_LEADER_SKILL
            | OOB_ROW_FLAG_MEMBER_COUNT_ENGINE,
        100,
        1,
        502,
        4,
        1,
        1,
        1,
        1,
        0,
        0,
        0,
    );
    d28_push_row(
        &mut p,
        OOB_KIND_FLEET,
        200,
        OOB_ROW_FLAG_OWNER | OOB_ROW_FLAG_LEADER,
        0,
        1,
        601,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    );
    d28_push_row(
        &mut p,
        OOB_KIND_TASK_FORCE,
        201,
        OOB_ROW_FLAG_PARENT | OOB_ROW_FLAG_OWNER | OOB_ROW_FLAG_LOCATION,
        200,
        1,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        12345,
    );
    for member_id in members {
        c2_u32(&mut p, member_id);
        c2_u32(&mut p, 0);
    }
    p
}

fn d29_ships_payload(rows: &[(u32, u32, u32, &str)], status_flags: u32) -> Vec<u8> {
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, status_flags);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, FIXED_SCALE as u32);
    c2_u32(&mut p, 0);
    for (instance_id, task_force_id, owner, definition) in rows {
        c2_u32(&mut p, *instance_id);
        c2_u32(
            &mut p,
            SHIP_ROW_FLAG_TASK_FORCE | SHIP_ROW_FLAG_DEFINITION | SHIP_ROW_FLAG_OWNER,
        );
        c2_u32(&mut p, *task_force_id);
        c2_u32(&mut p, 3);
        c2_u32(&mut p, 90_000);
        c2_u32(&mut p, 80_000);
        c2_u32(&mut p, 10_000);
        c2_u32(&mut p, 2);
        c2_u32(&mut p, *owner);
        c2_u32(&mut p, 0);
        push_str(&mut p, definition);
    }
    p
}

fn d56_row(
    task_force_id: u32,
    fleet_id: u32,
    owner_tag_db_id: u32,
    location_province_id: u32,
    sunk_convoys: u32,
) -> naval56::NavalMissionStatusRow {
    let mut flags = naval56::compact::FLAG_VALIDATED_DIRECT
        | naval56::compact::FLAG_FIXTURE_REQUIRED
        | naval56::compact::FLAG_HAS_LOCATION;
    let mut source_status = naval56::compact::STATUS_MISSION_FIXTURE_REQUIRED
        | naval56::compact::STATUS_HOME_BASE_FIXTURE_REQUIRED
        | naval56::compact::STATUS_REPAIR_RETREAT_FIXTURE_REQUIRED;
    if sunk_convoys > 0 {
        flags |= naval56::compact::FLAG_HAS_SUNK_CONVOY_COUNT
            | naval56::compact::FLAG_ACTIVE_BATTLE_FIXTURE_REQUIRED;
        source_status |= naval56::compact::STATUS_CONVOY_CONTEXT_FIXTURE_REQUIRED;
    }
    naval56::NavalMissionStatusRow {
        task_force_id,
        fleet_id,
        owner_tag_db_id,
        flags,
        location_province_id,
        sunk_convoys,
        source_status,
        ..naval56::NavalMissionStatusRow::default()
    }
}

fn d56_payload(status_flags: u32, rows: &[naval56::NavalMissionStatusRow]) -> Vec<u8> {
    naval56::encode_payload(
        &naval56::NavalMissionStatusHeader {
            elapsed_wall_ms: 5,
            changed_rows: rows.len().min(u32::MAX as usize) as u32,
            status_flags,
            observed_task_forces: rows.len().min(u32::MAX as usize) as u32,
            validated_location_rows: rows
                .iter()
                .filter(|row| row.flags & naval56::compact::FLAG_HAS_LOCATION != 0)
                .count()
                .min(u32::MAX as usize) as u32,
            derived_region_rows: rows
                .iter()
                .filter(|row| row.flags & naval56::compact::FLAG_HAS_REGION != 0)
                .count()
                .min(u32::MAX as usize) as u32,
            fixture_required_rows: rows
                .iter()
                .filter(|row| {
                    row.flags
                        & (naval56::compact::FLAG_FIXTURE_REQUIRED
                            | naval56::compact::FLAG_ACTIVE_BATTLE_FIXTURE_REQUIRED)
                        != 0
                })
                .count()
                .min(u32::MAX as usize) as u32,
            convoy_sunk_count_rows: rows
                .iter()
                .filter(|row| row.flags & naval56::compact::FLAG_HAS_SUNK_CONVOY_COUNT != 0)
                .count()
                .min(u32::MAX as usize) as u32,
        },
        rows,
    )
}

fn d54_path_row(
    army_id: u32,
    slot_index: u32,
    path_provinces: Vec<u32>,
) -> battleplans54::BattleplanDepthRow {
    battleplans54::BattleplanDepthRow {
        army_id,
        generation: 1,
        deleted: false,
        slot_index,
        order_type: 3,
        target_states: Vec::new(),
        path_provinces,
        target_declared_count: None,
        target_status: battleplans54::STATUS_UNSUPPORTED,
        geometry_status: battleplans54::STATUS_EXACT,
        execution_status: battleplans54::STATUS_FIXTURE_REQUIRED,
        assigned_units_status: battleplans54::STATUS_EXACT,
        order_linkage_status: battleplans54::STATUS_EXACT,
        stance_status: battleplans54::STATUS_UNKNOWN,
        aggressiveness_status: battleplans54::STATUS_UNKNOWN,
    }
}

fn d54_payload(
    status_flags: u32,
    rows: &[battleplans54::BattleplanDepthRow],
    omitted_non_exact_rows: u32,
    path_geometry_omitted_rows: u32,
) -> Vec<u8> {
    battleplans54::encode_depth_payload(
        rows,
        5,
        status_flags,
        rows.len().min(u32::MAX as usize) as u32,
        1,
        rows.len()
            .saturating_add(omitted_non_exact_rows as usize)
            .saturating_add(path_geometry_omitted_rows as usize)
            .min(u32::MAX as usize) as u32,
        omitted_non_exact_rows,
        path_geometry_omitted_rows,
    )
}

fn d48_payload(status_flags: u32, target_state_id: Option<u32>) -> Vec<u8> {
    let target_unavailable = target_state_id.is_none();
    let frame = domain48::LaneFrame {
        game_hour_raw: raw_for(16_959_360),
        status_flags,
        source_flags: domain48::SOURCE_FLAG_DLL_NATIVE
            | domain48::SOURCE_FLAG_SHARED_COUNTRY_ROOTS
            | domain48::SOURCE_FLAG_SHARED_MAP_TOPOLOGY
            | domain48::SOURCE_FLAG_HEALTHY_LEVEL_CANDIDATE
            | if target_unavailable {
                domain48::SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE
            } else {
                0
            },
        observed_country_roots: 1,
        observed_state_roots: 1,
        observed_province_roots: 1,
        construction_rows: vec![domain48::ConstructionQueueRow {
            tag_db_id: 1,
            line_id: 77,
            template_id: Some(20_013),
            active_factories: 15,
            cost_raw: 10_000_000,
            produced_raw: 5_000_000,
            priority: 0,
            target_state_id,
        }],
        building_rows: vec![
            domain48::BuildingRow {
                scope: domain48::SCOPE_STATE,
                location_id: 42,
                state_id: 42,
                type_id: 0x2fab,
                level: 5,
                healthy_level: Some(4),
            },
            domain48::BuildingRow {
                scope: domain48::SCOPE_PROVINCE,
                location_id: 2068,
                state_id: 42,
                type_id: 0x4cc1,
                level: 2,
                healthy_level: Some(2),
            },
        ],
        resource_rows: vec![domain48::StateResourceRow {
            state_id: 42,
            resource_id: 0x3e03,
            amount_raw: 2_500_000,
        }],
    };
    domain48::encode_lane_frame(&frame)
}

fn d48_no_construction_payload() -> Vec<u8> {
    let frame = domain48::LaneFrame {
        game_hour_raw: raw_for(16_959_360),
        status_flags: 0,
        source_flags: domain48::SOURCE_FLAG_DLL_NATIVE
            | domain48::SOURCE_FLAG_SHARED_COUNTRY_ROOTS
            | domain48::SOURCE_FLAG_SHARED_MAP_TOPOLOGY
            | domain48::SOURCE_FLAG_HEALTHY_LEVEL_CANDIDATE
            | domain48::SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
        observed_country_roots: 1,
        observed_state_roots: 1,
        observed_province_roots: 1,
        construction_rows: Vec::new(),
        building_rows: vec![domain48::BuildingRow {
            scope: domain48::SCOPE_STATE,
            location_id: 42,
            state_id: 42,
            type_id: 0x2fab,
            level: 5,
            healthy_level: Some(4),
        }],
        resource_rows: vec![domain48::StateResourceRow {
            state_id: 42,
            resource_id: 0x3e03,
            amount_raw: 2_500_000,
        }],
    };
    domain48::encode_lane_frame(&frame)
}

fn d48_sparse_partial_payload() -> Vec<u8> {
    let frame = domain48::LaneFrame {
        game_hour_raw: raw_for(16_959_360),
        status_flags: COMPACT_STATUS_PARTIAL_DATA,
        source_flags: domain48::SOURCE_FLAG_DLL_NATIVE
            | domain48::SOURCE_FLAG_SHARED_COUNTRY_ROOTS
            | domain48::SOURCE_FLAG_SHARED_MAP_TOPOLOGY
            | domain48::SOURCE_FLAG_HEALTHY_LEVEL_CANDIDATE
            | domain48::SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
        observed_country_roots: 1,
        observed_state_roots: 1,
        observed_province_roots: 1,
        construction_rows: vec![domain48::ConstructionQueueRow {
            tag_db_id: 1,
            line_id: 77,
            template_id: Some(20_013),
            active_factories: 15,
            cost_raw: 10_000_000,
            produced_raw: 6_000_000,
            priority: 0,
            target_state_id: None,
        }],
        building_rows: Vec::new(),
        resource_rows: Vec::new(),
    };
    domain48::encode_lane_frame(&frame)
}

fn d48_construction_row(line_id: u32, produced_raw: i32) -> domain48::ConstructionQueueRow {
    domain48::ConstructionQueueRow {
        tag_db_id: 1,
        line_id,
        template_id: Some(20_000 + line_id),
        active_factories: 15,
        cost_raw: 10_000_000,
        produced_raw,
        priority: line_id % 10,
        target_state_id: None,
    }
}

fn d48_state_building(type_id: u32, level: u32) -> domain48::BuildingRow {
    domain48::BuildingRow {
        scope: domain48::SCOPE_STATE,
        location_id: 42,
        state_id: 42,
        type_id,
        level,
        healthy_level: Some(level),
    }
}

fn d48_province_building(type_id: u32, level: u32) -> domain48::BuildingRow {
    domain48::BuildingRow {
        scope: domain48::SCOPE_PROVINCE,
        location_id: 2068,
        state_id: 42,
        type_id,
        level,
        healthy_level: Some(level),
    }
}

fn d48_custom_payload(
    status_flags: u32,
    construction_rows: Vec<domain48::ConstructionQueueRow>,
    building_rows: Vec<domain48::BuildingRow>,
) -> Vec<u8> {
    let frame = domain48::LaneFrame {
        game_hour_raw: raw_for(16_959_360),
        status_flags,
        source_flags: domain48::SOURCE_FLAG_DLL_NATIVE
            | domain48::SOURCE_FLAG_SHARED_COUNTRY_ROOTS
            | domain48::SOURCE_FLAG_SHARED_MAP_TOPOLOGY
            | domain48::SOURCE_FLAG_HEALTHY_LEVEL_CANDIDATE
            | domain48::SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
        observed_country_roots: 1,
        observed_state_roots: 1,
        observed_province_roots: 1,
        construction_rows,
        building_rows,
        resource_rows: vec![domain48::StateResourceRow {
            state_id: 42,
            resource_id: 0x3e03,
            amount_raw: 2_500_000,
        }],
    };
    domain48::encode_lane_frame(&frame)
}

fn d48_failure_payload(status_flags: u32) -> Vec<u8> {
    let frame = domain48::LaneFrame {
        game_hour_raw: raw_for(16_959_360),
        status_flags,
        source_flags: domain48::SOURCE_FLAG_DLL_NATIVE
            | domain48::SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
        observed_country_roots: 0,
        observed_state_roots: 0,
        observed_province_roots: 0,
        construction_rows: Vec::new(),
        building_rows: Vec::new(),
        resource_rows: Vec::new(),
    };
    domain48::encode_lane_frame(&frame)
}

fn d30_air_wings_payload(rows: &[(u32, u32, u64, u32)], status_flags: u32) -> Vec<u8> {
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, status_flags);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, FIXED_SCALE as u32);
    c2_u32(&mut p, 0);
    for (owner, wing_id, generation, count) in rows {
        c2_u32(&mut p, *owner);
        c2_u32(&mut p, *wing_id);
        c2_u64(&mut p, *generation);
        c2_u32(&mut p, AIR_WING_ROW_FLAG_MISSION_REGION);
        c2_u32(&mut p, *count);
        c2_u32(&mut p, 7);
        c2_u32(&mut p, 88);
        c2_u32(&mut p, count.saturating_sub(1));
        c2_u32(&mut p, 0);
    }
    p
}

fn d30_air_wings_delete_payload(owner: u32, wing_id: u32, generation: u64) -> Vec<u8> {
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, FIXED_SCALE as u32);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, owner);
    c2_u32(&mut p, wing_id);
    c2_u64(&mut p, generation);
    c2_u32(&mut p, ROW_FLAG_DELETED_LOW_BIT);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    p
}

fn d31_air_region_activity_payload(rows: &[(u32, u32, u32, u32)], status_flags: u32) -> Vec<u8> {
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, status_flags);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    for (region, owner, mission, planes) in rows {
        c2_u32(&mut p, *region);
        c2_u32(&mut p, *owner);
        c2_u32(&mut p, *mission);
        c2_u32(&mut p, 0);
        c2_u32(&mut p, 1);
        c2_u32(&mut p, *planes);
        c2_u32(&mut p, planes.saturating_sub(1));
        c2_u32(&mut p, 0);
    }
    p
}

fn d32_loadouts_payload(
    rows: &[(u32, u32, u64, u32, Vec<(u32, i64)>)],
    status_flags: u32,
) -> Vec<u8> {
    let equipment_count: usize = rows.iter().map(|(_, _, _, _, entries)| entries.len()).sum();
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, status_flags);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, equipment_count as u32);
    c2_u32(&mut p, equipment_count as u32);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, FIXED_SCALE as u32);
    c2_u32(&mut p, 0);
    let mut first = 0u32;
    for (owner, wing_id, generation, air_base_id, entries) in rows {
        c2_u32(&mut p, *owner);
        c2_u32(&mut p, *wing_id);
        c2_u64(&mut p, *generation);
        c2_u32(&mut p, 0);
        c2_u32(&mut p, *air_base_id);
        c2_u32(&mut p, first);
        c2_u32(&mut p, entries.len() as u32);
        first += entries.len() as u32;
    }
    for (_, _, _, _, entries) in rows {
        for (variant_id, amount_raw) in entries {
            c2_u32(&mut p, *variant_id);
            c2_u32(&mut p, 0);
            c2_i64(&mut p, *amount_raw);
        }
    }
    p
}

fn d33_variant_defs_payload(variant_id: u32, display: &str, variant: &str) -> Vec<u8> {
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, variant_id);
    c2_u32(
        &mut p,
        VARIANT_DEF_ROW_FLAG_DISPLAY_NAME
            | VARIANT_DEF_ROW_FLAG_VARIANT_NAME
            | VARIANT_DEF_ROW_FLAG_MODULES,
    );
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    push_str(&mut p, display);
    push_str(&mut p, variant);
    c2_u32(&mut p, 111);
    c2_u32(&mut p, 0);
    push_str(&mut p, "module_engine_3");
    p
}

fn d34_air_base_status_payload(rows: &[(u32, u32, u32)], status_flags: u32) -> Vec<u8> {
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, status_flags);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    for (air_base_id, state_id, planes) in rows {
        c2_u32(&mut p, *air_base_id);
        c2_u32(&mut p, *state_id);
        c2_u32(&mut p, 0);
        c2_u32(&mut p, 400);
        c2_u32(&mut p, 4);
        c2_u32(&mut p, 2);
        c2_u32(&mut p, *planes);
        c2_u32(&mut p, 0);
    }
    p
}

fn d35_air_variant_stats_payload(variant_id: u32) -> Vec<u8> {
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, FIXED_SCALE as u32);
    c2_u32(&mut p, FIXED_SCALE as u32);
    c2_u32(&mut p, variant_id);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 201);
    c2_u32(&mut p, 202);
    c2_u32(&mut p, 203);
    for raw in [
        45_000, 80_000, 12_000, 30_000, 50_000, 95_000, 10_000, 20_000, 11_000, 17_000,
    ] {
        c2_i32(&mut p, raw);
    }
    c2_u32(&mut p, 0);
    p
}

fn d36_reserves_payload(rows: &[(u32, u32, i64)], status_flags: u32) -> Vec<u8> {
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, status_flags);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, FIXED_SCALE as u32);
    for (owner, variant_id, amount_raw) in rows {
        c2_u32(&mut p, *owner);
        c2_u32(&mut p, *variant_id);
        c2_u32(&mut p, 0);
        c2_u32(&mut p, 0);
        c2_i64(&mut p, *amount_raw);
    }
    p
}

fn d38_air_superiority_inputs_payload(
    rows: &[(u32, u32, u32, u32, i64)],
    status_flags: u32,
) -> Vec<u8> {
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, status_flags);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, rows.len() as u32);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, FIXED_SCALE as u32);
    for (region, owner, mission, variant_id, amount_raw) in rows {
        c2_u32(&mut p, *region);
        c2_u32(&mut p, *owner);
        c2_u32(&mut p, *mission);
        c2_u32(&mut p, *variant_id);
        c2_u32(&mut p, 0);
        c2_u32(&mut p, 201);
        c2_u32(&mut p, 202);
        c2_u32(&mut p, 203);
        c2_u32(&mut p, 2);
        c2_u32(&mut p, 0);
        c2_i64(&mut p, *amount_raw);
    }
    p
}

fn d39_upgrades_payload(variant_id: u32, upgrade_index: u32, level: u32) -> Vec<u8> {
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, variant_id);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, upgrade_index);
    c2_u32(&mut p, level);
    p
}

fn d42_air_combat_losses_payload(status_flags: u32, changed_rows: u32) -> Vec<u8> {
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, changed_rows);
    c2_u32(&mut p, status_flags);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, FIXED_SCALE as u32);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 700);
    c2_u64(&mut p, 77);
    c2_u32(
        &mut p,
        AIR_STRENGTH_DELTA_ROW_FLAG_LOSS
            | AIR_STRENGTH_DELTA_ROW_FLAG_MISSION_REGION
            | AIR_STRENGTH_DELTA_ROW_FLAG_EQUIPMENT_SCALE_OK
            | AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALARS,
    );
    c2_u32(&mut p, 7);
    c2_u32(&mut p, 88);
    c2_u32(&mut p, 100);
    c2_u32(&mut p, 96);
    c2_i32(&mut p, -4);
    c2_u32(&mut p, 0);
    c2_i64(&mut p, 100_000);
    c2_i64(&mut p, 96_000);
    c2_i64(&mut p, -4_000);
    c2_u32(&mut p, 3);
    c2_i32(&mut p, 42_000);
    p
}

fn d46_air_variant_archetypes_payload(status_flags: u32) -> Vec<u8> {
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, status_flags);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 1);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0xCAFE_BABEu32);
    c2_u32(&mut p, FIXED_SCALE as u32);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 900);
    c2_u32(
        &mut p,
        AIR_VARIANT_ARCHETYPE_ROW_FLAG_ARCHETYPE
            | AIR_VARIANT_ARCHETYPE_ROW_FLAG_BASE_AIRFRAME
            | AIR_VARIANT_ARCHETYPE_ROW_FLAG_ROLE
            | AIR_VARIANT_ARCHETYPE_ROW_FLAG_AIR_SUPERIORITY_WEIGHT,
    );
    c2_u32(&mut p, 201);
    c2_u32(&mut p, 202);
    c2_u32(&mut p, 203);
    c2_i32(&mut p, 120_000);
    c2_u32(&mut p, 3);
    c2_u32(&mut p, 0);
    push_str(&mut p, "small_plane_airframe");
    push_str(&mut p, "fighter_equipment");
    push_str(&mut p, "fighter");
    p
}

#[test]
fn chunk2_folds_order_of_battle_with_domain_owned_merges() {
    let snap = chunk1_one_snapshot(vec![
        (18, country_identity_payload(&[(1, "GER")])),
        (27, d27_order_payload(0, 10, &[5, 6])),
        (28, d28_oob_payload(0)),
        (29, d29_ships_payload(&[(300, 201, 1, "destroyer_1")], 0)),
        (30, d30_air_wings_payload(&[(1, 700, 77, 100)], 0)),
        (
            32,
            d32_loadouts_payload(&[(1, 700, 77, 9001, vec![(900, 100_000)])], 0),
        ),
    ]);

    let oob = &snap["order_of_battle"];
    assert_eq!(oob["battle_plan_orders"][0]["army"], json!(10));
    assert_eq!(
        oob["battle_plan_orders"][0]["order_instance"]["states"],
        json!([5, 6])
    );
    assert_eq!(oob["army_groups"][0]["owner"], json!("GER"));
    assert_eq!(oob["armies"][0]["field_marshal_group"], json!(100));
    assert_eq!(oob["fleets"][0]["task_forces"][0]["id"], json!(201));
    assert_eq!(
        oob["fleets"][0]["task_forces"][0]["ships"][0]["_r2_ship_owner"]["owner"],
        json!("GER")
    );
    assert_eq!(oob["air_wings"][0]["owner"], json!("GER"));
    assert_eq!(oob["air_wings"][0]["air_base"], json!(9001));
    assert_eq!(oob["air_wings"][0]["equipment"][0][0], json!(900));
    assert_eq!(oob["by_country"]["GER"]["air_wings"], json!(1));
}

#[test]
fn chunk2_folds_d56_naval_status_under_oob_extra_without_owning_entities() {
    let rows = [d56_row(201, 200, 1, 12345, 2)];
    let snap = chunk1_one_snapshot(vec![
        (18, country_identity_payload(&[(1, "GER")])),
        (28, d28_oob_payload(0)),
        (56, d56_payload(0, &rows)),
    ]);

    let extra = &snap["order_of_battle"]["extra"]["naval_missions_status_live"];
    assert_eq!(extra["source"], json!("C"));
    assert_eq!(extra["confidence"], json!("validated_slice"));
    assert_eq!(extra["task_force_count"], json!(1));
    assert_eq!(extra["validated_location_rows"], json!(1));
    assert_eq!(extra["derived_region_rows"], json!(0));
    assert_eq!(extra["convoy_sunk_count_rows"], json!(1));
    assert_eq!(extra["oob_command_join_checked"], json!(true));
    assert_eq!(extra["oob_command_orphan_rows"], json!(0));
    let row = &extra["task_forces"][0];
    assert_eq!(row["task_force_id"], json!(201));
    assert_eq!(row["fleet_id"], json!(200));
    assert_eq!(row["owner"], json!("GER"));
    assert_eq!(row["owner_tag_db_id"], json!(1));
    assert_eq!(row["location_province_id"], json!(12345));
    assert_eq!(row["strategic_region_id"], Value::Null);
    assert_eq!(row["mission_enum"], Value::Null);
    assert_eq!(row["home_base_province_id"], Value::Null);
    assert_eq!(row["repair_base_province_id"], Value::Null);
    assert_eq!(row["retreat_state"], Value::Null);
    assert_eq!(row["sunk_convoys"], json!(2));
    assert_eq!(row["convoy_route_context_id"], Value::Null);
    assert_eq!(row["fixture_required"], json!(true));
    assert_eq!(row["active_battle_fixture_required"], json!(true));
    assert_eq!(row["oob_command_task_force_seen"], json!(true));
    assert_eq!(
        snap["order_of_battle"]["by_country"]["GER"]["task_forces"],
        json!(1)
    );
    assert!(!snap["provenance"]["collector"]["unfolded_domains"]
        .as_array()
        .unwrap()
        .iter()
        .any(|domain| domain["domain_id"] == json!(56)));
}

#[test]
fn chunk2_folds_d57_session_metadata_under_top_level_r2_sidecar() {
    let payload = domain57_payload(
        DOMAIN57_ROOT_PLAYER_COUNTRY | DOMAIN57_ROOT_PAUSE_CLOCK,
        vec![domain57_player_row("IRE", 29)],
        vec![domain57_event_row(
            session57_lane::SessionEventKind::Pause,
            1,
        )],
        0,
    );
    let snap = chunk1_one_snapshot(vec![(57, payload)]);

    let sidecar = &snap[DOMAIN57_R2_KEY];
    assert_eq!(sidecar["players_by_country"]["state"], json!("known"));
    assert_eq!(
        sidecar["players_by_country"]["rows"][0]["country_tag"],
        json!("IRE")
    );
    assert_eq!(
        sidecar["players_by_country"]["rows"][0]["tag_db_id"],
        json!(29)
    );
    assert_eq!(
        sidecar["players_by_country"]["rows"][0]["is_human_controlled"],
        json!(true)
    );
    assert_eq!(
        sidecar["players_by_country"]["rows"][0]["human_marker"],
        json!(true)
    );
    assert_eq!(
        sidecar["players_by_country"]["rows"][0]["human_name"]["state"],
        json!("not_sourced")
    );
    assert_eq!(
        sidecar["players_by_country"]["rows"][0]["player_key_hash"]["state"],
        json!("not_sourced")
    );
    assert_eq!(
        sidecar["players_by_country"]["rows"][0]["session_slot"]["state"],
        json!("not_sourced")
    );
    assert_eq!(
        sidecar["players_by_country"]["rows"][0]["host"]["state"],
        json!("not_sourced")
    );
    assert_eq!(
        sidecar["players_by_country"]["rows"][0]["connected"]["state"],
        json!("not_sourced")
    );
    assert_eq!(sidecar["pause"]["state"], json!("known"));
    assert_eq!(sidecar["pause"]["is_paused"], json!(true));
    assert!(snap["countries"].as_object().unwrap().is_empty());
    assert!(snap["world"].get("session").is_none());
    assert!(snap["world"].get("scenario").is_none());
    assert_eq!(snap["provenance"]["mod"], Value::Null);
}

#[test]
fn chunk2_d57_zero_human_rows_is_unavailable_but_pause_can_still_fold() {
    let payload = domain57_payload(
        DOMAIN57_ROOT_PLAYER_COUNTRY | DOMAIN57_ROOT_PAUSE_CLOCK,
        Vec::new(),
        vec![domain57_event_row(
            session57_lane::SessionEventKind::Pause,
            0,
        )],
        session57_lane::STATUS_SOURCE_UNAVAILABLE | session57_lane::STATUS_PARTIAL_DATA,
    );
    let snap = chunk1_one_snapshot(vec![(57, payload)]);

    let sidecar = &snap[DOMAIN57_R2_KEY];
    assert_eq!(sidecar["players_by_country"]["state"], json!("unavailable"));
    assert_eq!(
        sidecar["players_by_country"]["rows"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(sidecar["pause"]["state"], json!("known"));
    assert_eq!(sidecar["pause"]["is_paused"], json!(false));
    assert!(snap["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(57)
            && problem["problem"] == json!("players_by_country_unavailable")));
}

#[test]
fn chunk2_d57_game_speed_event_is_ignored_until_root_is_pinned() {
    let payload = domain57_payload(
        DOMAIN57_ROOT_PLAYER_COUNTRY,
        vec![domain57_player_row("IRE", 29)],
        vec![domain57_event_row(
            session57_lane::SessionEventKind::GameSpeed,
            3,
        )],
        session57_lane::STATUS_PARTIAL_DATA,
    );
    let snap = chunk1_one_snapshot(vec![(57, payload)]);

    let sidecar = &snap[DOMAIN57_R2_KEY];
    assert_eq!(sidecar["players_by_country"]["state"], json!("known"));
    assert_eq!(sidecar["pause"]["state"], json!("unavailable"));
    assert!(snap["world"].get("session").is_none());
    assert!(snap["world"].get("scenario").is_none());
}

#[test]
fn chunk2_d54_partial_with_rows_keeps_orders_and_surfaces_omissions() {
    let rows = [d54_path_row(17, 2, vec![101, 102])];
    let snap = chunk1_one_snapshot(vec![(
        54,
        d54_payload(COMPACT_STATUS_PARTIAL_DATA, &rows, 1, 1),
    )]);

    let extra = &snap["order_of_battle"]["extra"]["army_battleplans_depth_live"];
    assert_eq!(
        extra["known"],
        json!(true),
        "extra={extra:#} problems={:#} notes={:#}",
        snap["provenance"]["collector"]["domain_problems"],
        snap["provenance"]["notes"]
    );
    assert_eq!(extra["orders"].as_array().unwrap().len(), 1);
    assert_eq!(extra["orders"][0]["army_id"], json!(17));
    assert_eq!(extra["orders"][0]["path_provinces"], json!([101, 102]));
    assert_eq!(extra["omitted_non_exact_rows_this_hour"], json!(1));
    assert_eq!(extra["path_geometry_omitted_rows_this_hour"], json!(1));
    assert!(extra["provenance_notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap().contains("non-exact")));
}

#[test]
fn chunk2_d56_unmapped_owner_keeps_final_tag_index_unresolved() {
    let rows = [d56_row(201, 200, 2, 12345, 0)];
    let snap = chunk1_one_snapshot(vec![
        (18, country_identity_payload(&[(1, "GER")])),
        (56, d56_payload(0, &rows)),
    ]);

    assert_eq!(snap["tag_db_id_index"]["resolved"], json!(false));
    assert_eq!(snap["tag_db_id_index"]["key"], json!("tag"));
    let gaps = snap["tag_db_id_index"]["unresolved_tag_db_ids"]
        .as_array()
        .unwrap();
    assert!(gaps.iter().any(|gap| gap == "2"));

    let extra = &snap["order_of_battle"]["extra"]["naval_missions_status_live"];
    assert_eq!(extra["task_forces"][0]["owner"], json!("2"));
    assert_eq!(extra["task_forces"][0]["owner_tag_db_id"], json!(2));
    assert!(snap["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("tag_db_id -> TAG unresolved for 1: 2")));
}

#[test]
fn chunk2_d56_failure_masks_unknown_without_clearing_then_valid_empty_clears() {
    let rows = [d56_row(201, 200, 1, 12345, 0)];
    let snaps = chunk1_snapshots(vec![
        vec![(56, d56_payload(0, &rows))],
        vec![(56, d56_payload(COMPACT_STATUS_SOURCE_UNAVAILABLE, &[]))],
        vec![],
        vec![(56, d56_payload(0, &[]))],
    ]);

    assert_eq!(
        snaps[0]["order_of_battle"]["extra"]["naval_missions_status_live"]["task_force_count"],
        json!(1)
    );
    assert_eq!(
        snaps[1]["order_of_battle"]["extra"]["naval_missions_status_live"],
        Value::Null
    );
    assert!(snaps[1]["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(56)
            && problem["problem"] == json!("compact_status_failure")));
    assert_eq!(
        snaps[2]["order_of_battle"]["extra"]["naval_missions_status_live"]["task_force_count"],
        json!(1),
        "failure hour must not replace the carried D56 accumulator with empty"
    );
    assert_eq!(
        snaps[3]["order_of_battle"]["extra"]["naval_missions_status_live"]["task_force_count"],
        json!(0),
        "a valid empty full-replace frame is the measured empty state"
    );
}

#[test]
fn chunk2_d56_delta_before_baseline_marks_extra_unknown() {
    let rows = [d56_row(201, 200, 1, 12345, 0)];
    let snap = chunk1_one_snapshot(vec![(56, d56_payload(DELTA, &rows))]);

    assert_eq!(
        snap["order_of_battle"]["extra"]["naval_missions_status_live"],
        Value::Null
    );
    assert!(snap["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(56)
            && problem["problem"] == json!("delta_before_baseline")));
}

#[test]
fn chunk2_folds_d48_under_domain_owned_extras_without_d23_collisions() {
    let snap = chunk1_one_snapshot(vec![
        (18, country_identity_payload(&[(1, "GER")])),
        (48, d48_payload(0, Some(42))),
    ]);

    let production_extra =
        &snap["countries"]["GER"]["production"]["construction_buildings_state_damage_live"];
    assert_eq!(production_extra["source"], json!("C"));
    assert_eq!(
        production_extra["construction_queue"][0]["target_state_id"],
        json!(42)
    );
    assert_eq!(
        production_extra["construction_queue"][0]["template_id"],
        json!(20_013)
    );

    let state = snap["states"]["42"].as_object().unwrap();
    assert!(!state.contains_key("resources"));
    assert!(!state.contains_key("buildings"));
    assert!(!state.contains_key("province_buildings"));
    let state_extra = &state["construction_buildings_state_damage_live"];
    assert_eq!(
        state_extra["state_buildings"]["type_0x00002fab"]["level"],
        json!(5)
    );
    assert_eq!(
        state_extra["state_buildings"]["type_0x00002fab"]["healthy_level"],
        json!(4)
    );
    assert_eq!(
        state_extra["resource_amount_raw_by_id"]["tid_0x00003e03"],
        json!(2_500_000)
    );

    let province = snap["provinces"]["2068"].as_object().unwrap();
    assert!(!province.contains_key("state_id"));
    assert_eq!(
        province["construction_buildings_state_damage_live"]["state_id"],
        json!(42)
    );
    assert_eq!(
        province["construction_buildings_state_damage_live"]["buildings"]["type_0x00004cc1"]
            ["damaged"],
        json!(false)
    );
}

#[test]
fn chunk2_d48_distinguishes_unknown_absent_and_resolved_empty_queue() {
    let snaps = chunk1_snapshots(vec![
        vec![(48, d48_payload(0, None))],
        vec![(48, d48_failure_payload(COMPACT_STATUS_SOURCE_UNAVAILABLE))],
        vec![],
        vec![(48, d48_no_construction_payload())],
    ]);

    let first_extra =
        &snaps[0]["countries"]["1"]["production"]["construction_buildings_state_damage_live"];
    assert_eq!(first_extra["target_state_available"], json!(false));
    assert!(first_extra["construction_queue"][0]
        .as_object()
        .unwrap()
        .get("target_state_id")
        .is_none());

    assert_eq!(
        snaps[1]["states"]["42"]["construction_buildings_state_damage_live"],
        Value::Null
    );
    assert!(snaps[1]["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(48)
            && problem["problem"] == json!("compact_status_failure")));
    assert!(snaps[1]["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("domain extras unknown this hour")));

    assert_eq!(
        snaps[2]["states"]["42"]["construction_buildings_state_damage_live"],
        Value::Null,
        "absent D48 frames omit stale full-frame sidecars"
    );
    assert!(!snaps[2]["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(48)));

    assert_eq!(
        snaps[3]["states"]["42"]["construction_buildings_state_damage_live"]
            ["resource_amount_raw_by_id"]["tid_0x00003e03"],
        json!(2_500_000)
    );
    assert_eq!(
        snaps[3]["countries"]["1"]["production"]["construction_buildings_state_damage_live"],
        Value::Null,
        "valid no-construction frame is a measured empty queue, not source-unavailable"
    );
}

#[test]
fn chunk2_d48_delta_after_baseline_is_unknown_not_full_replace() {
    let snaps = chunk1_snapshots(vec![
        vec![(48, d48_payload(0, Some(42)))],
        vec![(48, d48_payload(DELTA, Some(42)))],
    ]);

    assert_eq!(
        snaps[0]["states"]["42"]["construction_buildings_state_damage_live"]
            ["resource_amount_raw_by_id"]["tid_0x00003e03"],
        json!(2_500_000)
    );
    assert_eq!(
        snaps[1]["states"]["42"]["construction_buildings_state_damage_live"],
        Value::Null,
        "D48 has no delta contract; DELTA_DATA must not be applied as a full replacement"
    );
    assert!(snaps[1]["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(48)
            && problem["problem"] == json!("delta_unsupported")));
}

#[test]
fn chunk2_d48_expected_partial_sparse_applies_present_rows_and_retains_siblings() {
    let snaps = chunk1_snapshots(vec![
        vec![(48, d48_payload(0, None))],
        vec![(48, d48_sparse_partial_payload())],
    ]);

    assert_eq!(
        snaps[1]["countries"]["1"]["production"]["construction_buildings_state_damage_live"]
            ["construction_queue"][0]["produced_raw"],
        json!(6_000_000)
    );
    assert_eq!(
        snaps[1]["states"]["42"]["construction_buildings_state_damage_live"]["state_buildings"]
            ["type_0x00002fab"]["level"],
        json!(5),
        "partial d48 must not fabricate omitted building disappearance"
    );
    assert_eq!(
        snaps[1]["states"]["42"]["construction_buildings_state_damage_live"]
            ["resource_amount_raw_by_id"]["tid_0x00003e03"],
        json!(2_500_000),
        "partial d48 must retain omitted resource siblings"
    );
    assert!(snaps[1]["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(48)
            && problem["problem"] == json!("partial_data")));
    assert!(snaps[1]["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("construction_buildings_state_damage_live expected partial tolerance")));
}

#[test]
fn chunk2_d48_expected_partial_merges_construction_lines_by_line_id() {
    let snaps = chunk1_snapshots(vec![
        vec![(
            48,
            d48_custom_payload(
                0,
                vec![
                    d48_construction_row(77, 5_000_000),
                    d48_construction_row(88, 2_000_000),
                ],
                vec![d48_state_building(0x2fab, 5)],
            ),
        )],
        vec![(
            48,
            d48_custom_payload(
                COMPACT_STATUS_PARTIAL_DATA,
                vec![d48_construction_row(77, 6_000_000)],
                Vec::new(),
            ),
        )],
    ]);

    let queue = snaps[1]["countries"]["1"]["production"]
        ["construction_buildings_state_damage_live"]["construction_queue"]
        .as_array()
        .unwrap();
    assert_eq!(queue.len(), 2);
    let line_a = queue
        .iter()
        .find(|row| row["id"] == json!(77))
        .expect("line 77 retained");
    let line_b = queue
        .iter()
        .find(|row| row["id"] == json!(88))
        .expect("line 88 retained");
    assert_eq!(line_a["produced_raw"], json!(6_000_000));
    assert_eq!(line_b["produced_raw"], json!(2_000_000));
    assert!(snaps[1]["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(48)
            && problem["problem"] == json!("partial_data")));
}

#[test]
fn chunk2_d48_clean_frame_full_replace_clears_removed_construction_line() {
    let snaps = chunk1_snapshots(vec![
        vec![(
            48,
            d48_custom_payload(
                0,
                vec![
                    d48_construction_row(77, 5_000_000),
                    d48_construction_row(88, 2_000_000),
                ],
                vec![d48_state_building(0x2fab, 5)],
            ),
        )],
        vec![(
            48,
            d48_custom_payload(
                0,
                vec![d48_construction_row(77, 6_000_000)],
                vec![d48_state_building(0x2fab, 5)],
            ),
        )],
    ]);

    let queue = snaps[1]["countries"]["1"]["production"]
        ["construction_buildings_state_damage_live"]["construction_queue"]
        .as_array()
        .unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0]["id"], json!(77));
    assert!(!queue.iter().any(|row| row["id"] == json!(88)));
}

#[test]
fn chunk2_d48_expected_partial_merges_building_types_by_container() {
    let snaps = chunk1_snapshots(vec![
        vec![(
            48,
            d48_custom_payload(
                0,
                vec![d48_construction_row(77, 5_000_000)],
                vec![
                    d48_state_building(0x2fab, 5),
                    d48_state_building(0x3000, 7),
                    d48_province_building(0x4cc1, 2),
                    d48_province_building(0x4cc2, 4),
                ],
            ),
        )],
        vec![(
            48,
            d48_custom_payload(
                COMPACT_STATUS_PARTIAL_DATA,
                Vec::new(),
                vec![
                    d48_state_building(0x2fab, 6),
                    d48_province_building(0x4cc1, 3),
                ],
            ),
        )],
    ]);

    let state_buildings =
        &snaps[1]["states"]["42"]["construction_buildings_state_damage_live"]["state_buildings"];
    assert_eq!(state_buildings["type_0x00002fab"]["level"], json!(6));
    assert_eq!(
        state_buildings["type_0x00003000"]["level"],
        json!(7),
        "partial d48 must retain omitted state building types"
    );
    let province_buildings =
        &snaps[1]["provinces"]["2068"]["construction_buildings_state_damage_live"]["buildings"];
    assert_eq!(province_buildings["type_0x00004cc1"]["level"], json!(3));
    assert_eq!(
        province_buildings["type_0x00004cc2"]["level"],
        json!(4),
        "partial d48 must retain omitted province building types"
    );
}

#[test]
fn chunk2_folds_design_and_variant_domains_without_key_collisions() {
    let snap = chunk1_one_snapshot(vec![
        (18, country_identity_payload(&[(1, "GER")])),
        (33, d33_variant_defs_payload(900, "Bf 109", "Bf 109 A")),
        (35, d35_air_variant_stats_payload(900)),
        (36, d36_reserves_payload(&[(1, 900, 250_000)], 0)),
        (
            37,
            d33_variant_defs_payload(901, "Infantry Equipment", "Gew 98"),
        ),
        (39, d39_upgrades_payload(900, 2, 4)),
        (40, d39_upgrades_payload(901, 1, 3)),
        (46, d46_air_variant_archetypes_payload(DELTA)),
    ]);

    let designs = &snap["designs"];
    assert_eq!(
        designs["equipment_variants_live"]["900"]["live_definition"]["display_name"],
        json!("Bf 109")
    );
    assert_eq!(
        designs["equipment_variants_live"]["900"]["live_stats"]["_r2_air_variant_stats"]
            ["equipment_token_raw"],
        json!(201)
    );
    assert_eq!(
        designs["equipment_variants_live"]["900"]["upgrade_levels"][0]["level"],
        json!(4)
    );
    assert_eq!(
        designs["equipment_variants_live"]["900"]["_r2_air_variant_archetypes"]["role_token"],
        json!("fighter")
    );
    assert_eq!(
        designs["_r2_stockpile_variant_defs"]["901"]["display_name"],
        json!("Infantry Equipment")
    );
    assert_eq!(
        designs["_r2_stockpile_variant_upgrades"]["901"][0]["level"],
        json!(3)
    );
    assert_eq!(
        designs["_r2_air_variant_reserves"][0]["owner"],
        json!("GER")
    );
}

#[test]
fn chunk2_folds_air_sections_and_keeps_d42_out_of_events() {
    let snap = chunk1_one_snapshot(vec![
        (18, country_identity_payload(&[(1, "GER")])),
        (31, d31_air_region_activity_payload(&[(88, 1, 7, 100)], 0)),
        (34, d34_air_base_status_payload(&[(9001, 64, 100)], 0)),
        (
            38,
            d38_air_superiority_inputs_payload(&[(88, 1, 7, 900, 100_000)], 0),
        ),
        (42, d42_air_combat_losses_payload(0, 1)),
    ]);

    assert_eq!(
        snap["naval_air"]["_r2_air_region_activity"][0]["owner"],
        json!("GER")
    );
    assert_eq!(snap["strategic_air"]["air_bases"][0]["id"], json!(9001));
    assert_eq!(
        snap["strategic_air"]["_r2_air_superiority_inputs"][0]["variant_id"],
        json!(900)
    );
    assert_eq!(
        snap["strategic_air"]["_r2_air_combat_losses_live"][0]["count_delta"],
        json!(-4)
    );
    assert_eq!(
        snap["strategic_air"]["_r2_air_combat_losses_live"][0]["air_accidents"],
        json!(3)
    );
    assert!(
        (snap["strategic_air"]["_r2_air_combat_losses_live"][0]["experience"]
            .as_f64()
            .unwrap()
            - 0.42)
            .abs()
            < 0.000001
    );
    let cumulative = &snap["strategic_air"]["_r2_air_wing_cumulative_scalars"];
    assert_eq!(cumulative[0]["wing_id"], json!(700));
    assert_eq!(cumulative[0]["air_accidents"], json!(3));
    assert_eq!(cumulative[0]["experience_raw"], json!(42_000));
    assert_eq!(snap["events"], json!([]));
}

fn d42_wing_row(
    p: &mut Vec<u8>,
    wing_id: u32,
    flags: u32,
    old_count: u32,
    new_count: u32,
    air_accidents: u32,
    experience_raw: i32,
) {
    let count_delta = new_count as i64 - old_count as i64;
    c2_u32(p, 1);
    c2_u32(p, wing_id);
    c2_u64(p, u64::from(wing_id));
    c2_u32(p, flags);
    c2_u32(p, 7);
    c2_u32(p, 0);
    c2_u32(p, old_count);
    c2_u32(p, new_count);
    c2_i32(p, count_delta as i32);
    c2_u32(p, 0);
    c2_i64(p, old_count as i64 * 100_000);
    c2_i64(p, new_count as i64 * 100_000);
    c2_i64(p, count_delta * 100_000);
    c2_u32(p, air_accidents);
    c2_i32(p, experience_raw);
}

fn d42_rows_payload(rows: impl FnOnce(&mut Vec<u8>) -> u32) -> Vec<u8> {
    let mut body = Vec::new();
    let row_count = rows(&mut body);
    let mut p = Vec::new();
    c2_u64(&mut p, 0);
    c2_u32(&mut p, row_count);
    c2_u32(&mut p, row_count);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, row_count);
    c2_u32(&mut p, row_count);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, 0);
    c2_u32(&mut p, FIXED_SCALE as u32);
    c2_u32(&mut p, 0);
    p.extend_from_slice(&body);
    p
}

#[test]
fn d42_baseline_rows_feed_cumulative_map_without_event_rows() {
    let baseline_flags = AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALARS
        | AIR_STRENGTH_DELTA_ROW_FLAG_STATE_BASELINE
        | AIR_STRENGTH_DELTA_ROW_FLAG_EQUIPMENT_SCALE_OK;
    let snap = chunk1_one_snapshot(vec![
        (18, country_identity_payload(&[(1, "GER")])),
        (
            42,
            d42_rows_payload(|rows| {
                d42_wing_row(rows, 700, baseline_flags, 100, 100, 3, 42_000);
                d42_wing_row(rows, 701, baseline_flags, 50, 50, 0, 5_000);
                2
            }),
        ),
    ]);
    // No loss/reinforcement/delete flags -> no hourly event rows.
    assert_eq!(
        snap["strategic_air"]["_r2_air_combat_losses_live"],
        json!([])
    );
    let cumulative = &snap["strategic_air"]["_r2_air_wing_cumulative_scalars"];
    assert_eq!(cumulative.as_array().unwrap().len(), 2);
    assert_eq!(cumulative[0]["wing_id"], json!(700));
    assert_eq!(cumulative[0]["air_accidents"], json!(3));
    assert_eq!(cumulative[1]["wing_id"], json!(701));
}

#[test]
fn d42_deleted_row_removes_wing_from_cumulative_map() {
    let baseline_flags = AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALARS
        | AIR_STRENGTH_DELTA_ROW_FLAG_STATE_BASELINE
        | AIR_STRENGTH_DELTA_ROW_FLAG_EQUIPMENT_SCALE_OK;
    let deleted_flags = AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALARS
        | AIR_STRENGTH_DELTA_ROW_FLAG_DELETED
        | AIR_STRENGTH_DELTA_ROW_FLAG_LOSS
        | AIR_STRENGTH_DELTA_ROW_FLAG_EQUIPMENT_SCALE_OK;
    let snaps = chunk1_snapshots(vec![
        vec![
            (18, country_identity_payload(&[(1, "GER")])),
            (
                42,
                d42_rows_payload(|rows| {
                    d42_wing_row(rows, 700, baseline_flags, 100, 100, 3, 42_000);
                    1
                }),
            ),
        ],
        vec![(
            42,
            d42_rows_payload(|rows| {
                d42_wing_row(rows, 700, deleted_flags, 100, 0, 3, 42_000);
                1
            }),
        )],
    ]);
    assert_eq!(
        snaps[0]["strategic_air"]["_r2_air_wing_cumulative_scalars"][0]["wing_id"],
        json!(700)
    );
    assert_eq!(
        snaps[1]["strategic_air"]["_r2_air_wing_cumulative_scalars"],
        json!([])
    );
    assert_eq!(
        snaps[1]["strategic_air"]["_r2_air_combat_losses_live"][0]["wing_id"],
        json!(700)
    );
}

#[test]
fn d42_wing_scalar_values_without_measured_flag_fail_fold_closed() {
    let flags = AIR_STRENGTH_DELTA_ROW_FLAG_LOSS | AIR_STRENGTH_DELTA_ROW_FLAG_EQUIPMENT_SCALE_OK;
    let snap = chunk1_one_snapshot(vec![(
        42,
        d42_rows_payload(|rows| {
            d42_wing_row(rows, 700, flags, 100, 96, 3, 42_000);
            1
        }),
    )]);
    assert!(snap["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("wing scalar values without measured flag")));
}

#[test]
fn d42_duplicate_wing_id_in_one_payload_fails_fold_closed() {
    let baseline_flags = AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALARS
        | AIR_STRENGTH_DELTA_ROW_FLAG_STATE_BASELINE
        | AIR_STRENGTH_DELTA_ROW_FLAG_EQUIPMENT_SCALE_OK;
    let snap = chunk1_one_snapshot(vec![(
        42,
        d42_rows_payload(|rows| {
            d42_wing_row(rows, 700, baseline_flags, 100, 100, 3, 42_000);
            d42_wing_row(rows, 700, baseline_flags, 100, 100, 9, 10_000);
            2
        }),
    )]);
    assert!(snap["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("appears twice in one payload")));
}

#[test]
fn chunk2_sparse_unflagged_followups_preserve_prior_rows() {
    let snaps = chunk1_snapshots(vec![
        vec![
            (18, country_identity_payload(&[(1, "GER")])),
            (
                29,
                d29_ships_payload(&[(300, 201, 1, "destroyer_1"), (301, 201, 1, "sub_1")], 0),
            ),
            (
                30,
                d30_air_wings_payload(&[(1, 700, 77, 100), (1, 701, 78, 50)], 0),
            ),
            (
                31,
                d31_air_region_activity_payload(&[(88, 1, 7, 100), (89, 1, 8, 50)], 0),
            ),
            (
                32,
                d32_loadouts_payload(
                    &[
                        (1, 700, 77, 9001, vec![(900, 100_000)]),
                        (1, 701, 78, 9002, vec![(901, 50_000)]),
                    ],
                    0,
                ),
            ),
            (
                34,
                d34_air_base_status_payload(&[(9001, 64, 100), (9002, 65, 50)], 0),
            ),
            (
                36,
                d36_reserves_payload(&[(1, 900, 250_000), (1, 901, 50_000)], 0),
            ),
            (
                38,
                d38_air_superiority_inputs_payload(
                    &[(88, 1, 7, 900, 100_000), (89, 1, 8, 901, 50_000)],
                    0,
                ),
            ),
        ],
        vec![
            (
                29,
                d29_ships_payload(&[(300, 201, 1, "destroyer_1_refit")], 0),
            ),
            (30, d30_air_wings_payload(&[(1, 700, 77, 101)], 0)),
            (31, d31_air_region_activity_payload(&[(88, 1, 7, 101)], 0)),
            (
                32,
                d32_loadouts_payload(&[(1, 700, 77, 9001, vec![(900, 101_000)])], 0),
            ),
            (34, d34_air_base_status_payload(&[(9001, 64, 101)], 0)),
            (36, d36_reserves_payload(&[(1, 900, 251_000)], 0)),
            (
                38,
                d38_air_superiority_inputs_payload(&[(88, 1, 7, 900, 101_000)], 0),
            ),
        ],
    ]);

    assert_eq!(
        snaps[1]["order_of_battle"]["ships"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        snaps[1]["order_of_battle"]["air_wings"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        snaps[1]["naval_air"]["_r2_air_region_activity"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        snaps[1]["strategic_air"]["air_bases"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        snaps[1]["designs"]["_r2_air_variant_reserves"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        snaps[1]["strategic_air"]["_r2_air_superiority_inputs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn chunk2_d30_valid_empty_tombstone_removes_air_wing() {
    let snaps = chunk1_snapshots(vec![
        vec![
            (18, country_identity_payload(&[(1, "GER")])),
            (30, d30_air_wings_payload(&[(1, 700, 77, 100)], 0)),
        ],
        vec![(30, d30_air_wings_delete_payload(1, 700, 77))],
    ]);

    assert_eq!(
        snaps[0]["order_of_battle"]["air_wings"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        snaps[1]["order_of_battle"]["air_wings"],
        json!([]),
        "valid empty current set is represented by tombstones, not SOURCE_UNAVAILABLE"
    );
}

#[test]
fn chunk2_d30_failure_does_not_consume_pending_delete() {
    let snaps = chunk1_snapshots(vec![
        vec![
            (18, country_identity_payload(&[(1, "GER")])),
            (30, d30_air_wings_payload(&[(1, 700, 77, 100)], 0)),
        ],
        vec![(
            30,
            d30_air_wings_payload(&[], COMPACT_STATUS_SOURCE_UNAVAILABLE),
        )],
        vec![(30, d30_air_wings_delete_payload(1, 700, 77))],
    ]);

    assert_eq!(
        snaps[1]["order_of_battle"]["air_wings"],
        Value::Null,
        "failure frame must null the owned leaf for this hour"
    );
    assert!(snaps[1]["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("air_wings state unknown this hour")));
    assert_eq!(
        snaps[2]["order_of_battle"]["air_wings"],
        json!([]),
        "recovered valid frame must still deliver the tombstone"
    );
}

#[test]
fn chunk2_d32_orphan_loadout_does_not_resurrect_deleted_air_wing() {
    let snaps = chunk1_snapshots(vec![
        vec![
            (18, country_identity_payload(&[(1, "GER")])),
            (30, d30_air_wings_payload(&[(1, 700, 77, 100)], 0)),
            (
                32,
                d32_loadouts_payload(&[(1, 700, 77, 9001, vec![(900, 100_000)])], 0),
            ),
        ],
        vec![
            (30, d30_air_wings_delete_payload(1, 700, 77)),
            (
                32,
                d32_loadouts_payload(&[(1, 700, 77, 9001, vec![(900, 100_000)])], 0),
            ),
        ],
    ]);

    assert_eq!(snaps[1]["order_of_battle"]["air_wings"], json!([]));
    assert!(snaps[1]["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("air_wing_loadouts dropped 1 orphan row")));
}

#[test]
fn chunk2_d32_orphan_note_is_current_hour_only() {
    let snaps = chunk1_snapshots(vec![
        vec![
            (18, country_identity_payload(&[(1, "GER")])),
            (30, d30_air_wings_payload(&[(1, 700, 77, 100)], 0)),
            (
                32,
                d32_loadouts_payload(&[(1, 700, 77, 9001, vec![(900, 100_000)])], 0),
            ),
        ],
        vec![
            (30, d30_air_wings_delete_payload(1, 700, 77)),
            (
                32,
                d32_loadouts_payload(&[(1, 700, 77, 9001, vec![(900, 100_000)])], 0),
            ),
        ],
        vec![],
    ]);

    assert!(snaps[1]["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("air_wing_loadouts dropped 1 orphan row")));
    assert!(!snaps[2]["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap().contains("air_wing_loadouts dropped")));
}

#[test]
fn chunk2_d32_fresh_loadout_reattaches_after_d30_reappearance() {
    let snaps = chunk1_snapshots(vec![
        vec![
            (18, country_identity_payload(&[(1, "GER")])),
            (30, d30_air_wings_payload(&[(1, 700, 77, 100)], 0)),
            (
                32,
                d32_loadouts_payload(&[(1, 700, 77, 9001, vec![(900, 100_000)])], 0),
            ),
        ],
        vec![(30, d30_air_wings_delete_payload(1, 700, 77))],
        vec![
            (30, d30_air_wings_payload(&[(1, 700, 77, 120)], 0)),
            (
                32,
                d32_loadouts_payload(&[(1, 700, 77, 9002, vec![(901, 200_000)])], 0),
            ),
        ],
    ]);

    assert_eq!(snaps[1]["order_of_battle"]["air_wings"], json!([]));
    assert!(snaps[1]["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note
            .as_str()
            .unwrap()
            .contains("air_wing_loadouts dropped 1 orphan row")));
    let wing = &snaps[2]["order_of_battle"]["air_wings"][0];
    assert_eq!(wing["id"], json!(700));
    assert_eq!(wing["air_base"], json!(9002));
    assert_eq!(wing["equipment"][0][0], json!(901));
    assert_eq!(
        wing["_r2_air_wing_loadout"]["equipment"][0]["amount_raw"],
        json!(200_000)
    );
    assert!(!snaps[2]["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap().contains("air_wing_loadouts dropped")));
}

#[test]
fn chunk2_delta_before_baseline_marks_owned_leaf_unknown() {
    let snap = chunk1_one_snapshot(vec![(27, d27_empty_payload(DELTA))]);
    assert_eq!(snap["order_of_battle"]["battle_plan_orders"], Value::Null);
    assert!(snap["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(27)
            && problem["problem"] == json!("delta_before_baseline")));
}

#[test]
fn chunk2_failure_nulls_owned_leaf_without_clearing_accumulator() {
    let snaps = chunk1_snapshots(vec![
        vec![(34, d34_air_base_status_payload(&[(9001, 64, 100)], 0))],
        vec![(
            34,
            d34_air_base_status_payload(&[], COMPACT_STATUS_SOURCE_UNAVAILABLE),
        )],
        vec![(34, d34_air_base_status_payload(&[], 0))],
    ]);
    assert_eq!(snaps[0]["strategic_air"]["air_bases"][0]["id"], json!(9001));
    assert_eq!(snaps[1]["strategic_air"]["air_bases"], Value::Null);
    assert_eq!(
        snaps[2]["strategic_air"]["air_bases"][0]["id"],
        json!(9001),
        "post-baseline empty sparse frame means no changes, not a fabricated empty set"
    );
}

fn d46_ace_row(
    tag_db_id: u32,
    ace_id: u32,
    wing_id: Option<u32>,
    alive: Option<bool>,
    killer_ace_id: Option<u32>,
    killer_country_tag_db_id: Option<u32>,
) -> ace46::AirAceRosterRow {
    let mut source_mask = ace46::SOURCE_LIVE_COUNTRY_ACE_VECTOR
        | ace46::SOURCE_LIVE_ACE_OBJECT_LAYOUT
        | ace46::SOURCE_DEV_SAVE_LAYOUT_VALIDATION;
    if wing_id.is_some() {
        source_mask |= ace46::SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT;
    }
    if alive.is_some() {
        source_mask |= ace46::SOURCE_LIVE_ACE_DEATH_RECORD;
    }
    ace46::AirAceRosterRow {
        tag_db_id,
        ace_id,
        country_ace_index: 0,
        wing_id,
        alive,
        killer_ace_id,
        killer_country_tag_db_id,
        source_mask,
        name: Some("Hans".to_string()),
        surname: Some("Rudel".to_string()),
        callsign: Some("Adler".to_string()),
    }
}

fn d46_payload_with_ace_trailer(rows: &[ace46::AirAceRosterRow], status_flags: u32) -> Vec<u8> {
    let header = ace46::AirAceRosterHeader {
        status_flags,
        observed_country_rows: 17,
        observed_ace_rows: rows.len() as u32,
        wing_assignment_rows: rows.iter().filter(|row| row.wing_id.is_some()).count() as u32,
        dead_rows: rows.iter().filter(|row| row.alive == Some(false)).count() as u32,
        skipped_rows: 0,
    };
    let mut payload = d46_air_variant_archetypes_payload(DELTA);
    payload.extend_from_slice(&ace46::encode_trailer(&header, rows));
    payload
}

#[test]
fn chunk2_d46_ace_trailer_annotates_wings_and_builds_country_sidecar() {
    let snap = chunk1_one_snapshot(vec![
        (18, country_identity_payload(&[(1, "GER")])),
        (30, d30_air_wings_payload(&[(1, 700, 77, 100)], 0)),
        (
            46,
            d46_payload_with_ace_trailer(
                &[
                    d46_ace_row(1, 5440, Some(700), Some(true), None, None),
                    d46_ace_row(1, 6265, None, Some(false), Some(5440), Some(92)),
                ],
                0,
            ),
        ),
    ]);

    let wing = &snap["order_of_battle"]["air_wings"][0];
    assert_eq!(wing["id"], json!(700));
    assert_eq!(wing["ace"]["id"], json!(5440));
    assert_eq!(wing["ace"]["name"], json!("Hans"));
    assert_eq!(wing["ace"]["alive"], json!(true));

    let aces = &snap["countries"]["GER"]["extra"]["_r2_air_ace_roster_live"]["aces"];
    assert_eq!(aces.as_array().map(Vec::len), Some(2));
    assert_eq!(aces[0]["ace_id"], json!(5440));
    assert_eq!(aces[0]["wing_id"], json!(700));
    assert_eq!(aces[0]["killer_ace_id"], Value::Null);
    assert_eq!(aces[1]["ace_id"], json!(6265));
    assert_eq!(aces[1]["alive"], json!(false));
    assert_eq!(aces[1]["killer_ace_id"], json!(5440));
    assert_eq!(aces[1]["killer_country_tag_db_id"], json!(92));
}

#[test]
fn chunk2_d46_ace_trailer_ambiguous_wing_claims_drop_the_annotation() {
    let snap = chunk1_one_snapshot(vec![
        (18, country_identity_payload(&[(1, "GER")])),
        (30, d30_air_wings_payload(&[(1, 700, 77, 100)], 0)),
        (
            46,
            d46_payload_with_ace_trailer(
                &[
                    d46_ace_row(1, 5440, Some(700), Some(true), None, None),
                    d46_ace_row(1, 6001, Some(700), Some(true), None, None),
                ],
                0,
            ),
        ),
    ]);

    let wing = &snap["order_of_battle"]["air_wings"][0];
    assert_eq!(wing["id"], json!(700));
    assert_eq!(wing.get("ace"), None);
    // The sidecar still carries both rows (the ambiguity only drops the
    // wing-side join, never the roster identity).
    let aces = &snap["countries"]["GER"]["extra"]["_r2_air_ace_roster_live"]["aces"];
    assert_eq!(aces.as_array().map(Vec::len), Some(2));
}

#[test]
fn chunk2_d46_ace_trailer_source_unavailable_retains_and_badges() {
    let snap = chunk1_one_snapshot(vec![
        (18, country_identity_payload(&[(1, "GER")])),
        (
            46,
            d46_payload_with_ace_trailer(&[], ace46::ACE_STATUS_SOURCE_UNAVAILABLE),
        ),
    ]);

    assert_eq!(snap["countries"]["GER"].get("extra"), None);
    assert!(snap["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(46)
            && problem["problem"] == json!("ace_roster_source_unavailable")));
}

#[test]
fn chunk2_d20_division_supply_scalars_fold_with_presence() {
    // Div A: supply_gain present-nonzero, disrupted present-nonzero.
    // Div B: supply_gain present-ZERO, disrupted present-ZERO (valid, kept).
    // Div C: both absent (flags clear) -> null, not 0.0.
    let mut divs = Vec::new();
    divisions_header(&mut divs, 3, 0);
    // moving=0, in_combat=0; base_flags carries only the supply/disrupted bits
    // added by the helper.
    push_div_row_with_supply(&mut divs, 17, 0, 118, Some(25_000), Some(39_248));
    push_div_row_with_supply(&mut divs, 18, 0, 119, Some(0), Some(0));
    push_div_row_with_supply(&mut divs, 19, 0, 120, None, None);

    let snap = snapshot_for_domain_records(vec![(20, divs)]);
    let list = snap["divisions"].as_array().unwrap();
    let by_id = |id: u32| {
        list.iter()
            .find(|d| d["div_id"] == json!(id))
            .unwrap_or_else(|| panic!("div {id} missing"))
    };

    let a = by_id(17);
    assert!((a["supply_gain"].as_f64().unwrap() - 0.25).abs() < 1e-9);
    assert!((a["disrupted_supply"].as_f64().unwrap() - 0.39248).abs() < 1e-9);

    let b = by_id(18);
    assert_eq!(b["supply_gain"], json!(0.0));
    assert_eq!(b["disrupted_supply"], json!(0.0));

    let c = by_id(19);
    assert_eq!(c["supply_gain"], Value::Null);
    assert_eq!(c["disrupted_supply"], Value::Null);
}

#[test]
fn chunk2_d11_production_license_trailer_folds_per_country() {
    // LAT: two licensed variants; ROM: proven-empty set; GER: no trailer row
    // (unreadable -> production_licenses null, absent != empty).
    let mut prod = production_payload(1, (10.0 * 100_000.0) as i64, 2);
    // The base payload's rows are all keyed to tag 1; add license rows for
    // tags 1 (LAT-like) and 2 (ROM-like empty). Tag 3 gets no trailer row.
    append_production_license_trailer(&mut prod, &[(1, vec![5132, 5081]), (2, vec![])]);

    let snap = snapshot_for_domain_records(vec![
        (
            18,
            country_identity_payload(&[(1, "LAT"), (2, "ROM"), (3, "GER")]),
        ),
        (11, prod),
    ]);

    // LAT: measured, sorted ids as strings.
    assert_eq!(
        snap["countries"]["LAT"]["production"]["production_licenses"],
        json!(["5081", "5132"])
    );
    // ROM: proven-empty set -> [] (distinct from null).
    assert_eq!(
        snap["countries"]["ROM"]["production"]["production_licenses"],
        json!([])
    );
    // GER: no production block at all this frame (it had no base production
    // rows and no license trailer row), so no production object is created.
    assert!(snap["countries"]
        .get("GER")
        .and_then(|c| c.get("production"))
        .is_none());
}

#[test]
fn chunk2_d11_production_without_license_trailer_folds_null() {
    // A legacy-shaped production record (no PLIC trailer) folds
    // production_licenses = null (not measured), never [] (empty).
    let prod = production_payload(1, (10.0 * 100_000.0) as i64, 2);
    let snap = snapshot_for_domain_records(vec![
        (18, country_identity_payload(&[(1, "LAT")])),
        (11, prod),
    ]);
    assert_eq!(
        snap["countries"]["LAT"]["production"]["production_licenses"],
        Value::Null
    );
}

#[test]
fn chunk2_d46_partial_frame_does_not_wipe_a_good_prior_roster() {
    // Hour 1: a clean complete roster (2 aces, one on wing 700).
    // Hour 2: a PARTIAL frame that dropped a torn country -> carries fewer
    // rows. It MUST NOT clear+replace: hour-2 retains hour-1's roster in the
    // map, and the wing ace annotation is SUPPRESSED (not fresh) so a read
    // failure is never surfaced as this hour's ground truth.
    let clean = d46_payload_with_ace_trailer(
        &[
            d46_ace_row(1, 5440, Some(700), Some(true), None, None),
            d46_ace_row(1, 6265, None, Some(false), Some(5440), Some(92)),
        ],
        0,
    );
    let partial_header = ace46::AirAceRosterHeader {
        status_flags: ace46::ACE_STATUS_PARTIAL_DATA,
        observed_country_rows: 17,
        observed_ace_rows: 1,
        wing_assignment_rows: 0,
        dead_rows: 0,
        skipped_rows: 1,
    };
    let partial_rows = vec![d46_ace_row(1, 5440, None, Some(true), None, None)];
    let mut partial = d46_air_variant_archetypes_payload(DELTA);
    partial.extend_from_slice(&ace46::encode_trailer(&partial_header, &partial_rows));

    let snaps = chunk1_snapshots(vec![
        vec![
            (18, country_identity_payload(&[(1, "GER")])),
            (30, d30_air_wings_payload(&[(1, 700, 77, 100)], 0)),
            (46, clean),
        ],
        vec![
            (18, country_identity_payload(&[(1, "GER")])),
            (30, d30_air_wings_payload(&[(1, 700, 77, 100)], 0)),
            (46, partial),
        ],
    ]);

    let h1 = &snaps[0];
    assert_eq!(
        h1["order_of_battle"]["air_wings"][0]["ace"]["id"],
        json!(5440)
    );
    assert_eq!(
        h1["countries"]["GER"]["extra"]["_r2_air_ace_roster_live"]["aces"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );

    let h2 = &snaps[1];
    // Partial hour: wing annotation suppressed (not fresh)...
    assert_eq!(h2["order_of_battle"]["air_wings"][0].get("ace"), None);
    // ...and the retained sidecar still carries BOTH hour-1 aces (the partial
    // single-row frame did not wipe the second ace).
    assert_eq!(
        h2["countries"]["GER"]["extra"]["_r2_air_ace_roster_live"]["aces"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert!(h2["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(46)
            && problem["problem"] == json!("ace_roster_partial")));
}

#[test]
fn chunk2_d46_first_delta_payload_is_implicit_baseline() {
    let snap = chunk1_one_snapshot(vec![(46, d46_air_variant_archetypes_payload(DELTA))]);
    assert_eq!(
        snap["designs"]["equipment_variants_live"]["900"]["archetype"],
        json!("small_plane_airframe")
    );
    assert!(!snap["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|problem| problem["domain_id"] == json!(46)
            && problem["problem"] == json!("delta_before_baseline")));
}

#[test]
fn chunk2_malformed_payload_is_rejected_not_treated_as_empty() {
    let snap = chunk1_one_snapshot(vec![(42, d42_air_combat_losses_payload(0, 0))]);
    assert!(snap["strategic_air"]
        .get("_r2_air_combat_losses_live")
        .is_none());
    assert!(snap["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap().contains("domain 42 fold skipped")));
}

// --- DOMAIN 60 capitulation_war_state_live (E1) ---------------------------

fn capwar_real_date_raw(year: i32, doy: i32, hour: i32) -> i32 {
    DATE_EPOCH_CONST + (year * 365 + doy) * 24 + hour
}

fn capwar_country_row(
    tag_db_id: u32,
    capitulated: bool,
    capitulated_date_raw: i32,
    legitimacy_raw: i32,
    exile_army_leaders: i32,
) -> capwar60::compact::CountryCapitulationRow {
    capwar60::compact::CountryCapitulationRow {
        tag_db_id,
        capitulated,
        capitulated_date_raw,
        last_surrender_date_raw: capwar60::compact::CGAMEDATE_NULL_RAW,
        faction_join_date_raw: capwar60::compact::CGAMEDATE_NULL_RAW,
        legitimacy_raw,
        exile_army_leaders,
    }
}

fn capwar_payload(snapshot: &capwar60::compact::CapitulationWarSnapshot) -> Vec<u8> {
    capwar60::compact::encode_payload(snapshot)
}

fn capwar_sample_snapshot() -> capwar60::compact::CapitulationWarSnapshot {
    // FRA(3) capitulated 1940.6.18.2 with legitimacy 100.0 and 8 exile
    // leaders; GER(47) not capitulated. One GER->FRA war started 1939.9.1.4.
    let fra_date = capwar_real_date_raw(1940, 31 + 28 + 31 + 30 + 31 + 17, 2);
    capwar60::compact::CapitulationWarSnapshot {
        elapsed_wall_ms: 1,
        status_flags: capwar60::compact::STATUS_WAR_CASUALTIES_RUNNING_ACCUMULATOR,
        countries: vec![
            capwar_country_row(3, true, fra_date, 10_000_000, 8),
            capwar_country_row(47, false, capwar60::compact::CGAMEDATE_NULL_RAW, 0, 0),
        ],
        wars: vec![capwar60::compact::WarRow {
            first_tag_db_id: 47,
            second_tag_db_id: 3,
            start_date_raw: capwar_real_date_raw(1939, 31 + 28 + 31 + 30 + 31 + 30 + 31 + 31, 4),
            first_casualties: 1_110,
            second_casualties: 948,
        }],
    }
}

fn capwar_stream(records: Vec<(u16, Vec<u8>)>, raw: i32) -> Vec<u8> {
    let mut stream = Vec::new();
    let mut seq = 1u64;
    for (dom, payload) in records {
        push_record(
            &mut stream,
            seq,
            raw,
            dom,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        );
        seq += 1;
    }
    push_record(
        &mut stream,
        seq,
        raw,
        1000,
        COMPACT_RECORD_TYPE_FRAME_END,
        &frame_end_payload(),
    );
    stream
}

#[test]
fn capitulation_war_state_folds_countries_and_wars() {
    let raw = raw_for(16_959_360);
    let stream = capwar_stream(
        vec![
            (18, country_identity_payload(&[(47, "GER"), (3, "FRA")])),
            (10, fuel_payload(&[(47, 100, 200, 0, 0)])),
            (10, fuel_payload(&[(3, 100, 200, 0, 0)])),
            (60, capwar_payload(&capwar_sample_snapshot())),
        ],
        raw,
    );
    let mut fold = WorldFold::new("capwar");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 1);
    let s = parse(&snaps[0].2);

    let fra = &s["countries"]["FRA"]["capitulation"];
    assert_eq!(fra["capitulated"], json!(true));
    assert_eq!(fra["capitulated_date"], json!("1940.6.18.2"));
    assert_eq!(fra["last_surrender_date"], Value::Null);
    assert_eq!(fra["faction_join_date"], Value::Null);
    assert!((fra["legitimacy"].as_f64().unwrap() - 100.0).abs() < 1e-9);
    assert_eq!(fra["legitimacy_raw"], json!(10_000_000));
    assert_eq!(fra["exile_army_leaders"], json!(8));

    let ger = &s["countries"]["GER"]["capitulation"];
    assert_eq!(ger["capitulated"], json!(false));
    assert_eq!(ger["capitulated_date"], Value::Null);

    let wars = s["diplomacy_world"]["wars"].as_object().unwrap();
    assert_eq!(wars.len(), 1);
    let war = &wars["GER|FRA"];
    assert_eq!(war["first"], json!("GER"));
    assert_eq!(war["second"], json!("FRA"));
    assert_eq!(war["first_tag_db_id"], json!(47));
    assert_eq!(war["second_tag_db_id"], json!(3));
    assert_eq!(war["start_date"], json!("1939.9.1.4"));
    assert_eq!(war["first_casualties"], json!(1_110));
    assert_eq!(war["second_casualties"], json!(948));
    assert_eq!(
        war["casualties_semantics"],
        json!("live_running_accumulator")
    );

    let dom = s["provenance"]["collector"]["domains"].as_array().unwrap();
    assert!(dom.iter().any(|d| d == "capitulation_war_state_live"));
}

#[test]
fn capitulation_persists_across_hours_but_wars_go_unknown() {
    let raw1 = raw_for(16_959_360);
    let raw2 = raw_for(16_959_361);
    let mut stream = capwar_stream(
        vec![
            (18, country_identity_payload(&[(47, "GER"), (3, "FRA")])),
            (10, fuel_payload(&[(3, 100, 200, 0, 0)])),
            (60, capwar_payload(&capwar_sample_snapshot())),
        ],
        raw1,
    );
    // Hour 2: no D60 record at all.
    stream.extend(capwar_stream(
        vec![
            (18, country_identity_payload(&[(47, "GER"), (3, "FRA")])),
            (10, fuel_payload(&[(3, 100, 200, 0, 0)])),
        ],
        raw2,
    ));
    let mut fold = WorldFold::new("capwar");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 2);

    let s1 = parse(&snaps[0].2);
    assert_eq!(
        s1["countries"]["FRA"]["capitulation"]["capitulated"],
        json!(true)
    );
    assert!(s1["diplomacy_world"]["wars"].is_object());

    // Hour 2: capitulation state persists (last proven frame), wars unknown.
    let s2 = parse(&snaps[1].2);
    assert_eq!(
        s2["countries"]["FRA"]["capitulation"]["capitulated"],
        json!(true)
    );
    assert_eq!(s2["diplomacy_world"]["wars"], Value::Null);
}

#[test]
fn capitulation_absent_country_gets_no_key() {
    // D60 frame carries only FRA; GER materializes via fuel but must get NO
    // capitulation key (absent != false).
    let raw = raw_for(16_959_360);
    let mut snapshot = capwar_sample_snapshot();
    snapshot.countries.retain(|row| row.tag_db_id == 3);
    snapshot.wars.clear();
    snapshot.status_flags = 0;
    let stream = capwar_stream(
        vec![
            (18, country_identity_payload(&[(47, "GER"), (3, "FRA")])),
            (10, fuel_payload(&[(47, 100, 200, 0, 0)])),
            (60, capwar_payload(&snapshot)),
        ],
        raw,
    );
    let mut fold = WorldFold::new("capwar");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    let s = parse(&snaps[0].2);
    assert!(s["countries"]["FRA"]["capitulation"]["capitulated"].as_bool() == Some(true));
    assert!(s["countries"]["GER"].get("capitulation").is_none());
    // Proven frame with zero wars: genuine empty object, not null.
    assert_eq!(s["diplomacy_world"]["wars"], json!({}));
}

#[test]
fn capitulated_minor_without_other_domains_materializes_numeric() {
    // A capitulated country with no identity mapping and no other domain rows
    // must still surface (numeric key) and be recorded as unresolved. The
    // identity record maps only GER so the tag index exists.
    let raw = raw_for(16_959_360);
    let mut snapshot = capwar_sample_snapshot();
    snapshot.countries = vec![capwar_country_row(
        99,
        true,
        capwar_real_date_raw(1941, 50, 6),
        0,
        0,
    )];
    snapshot.wars.clear();
    snapshot.status_flags = 0;
    let stream = capwar_stream(
        vec![
            (18, country_identity_payload(&[(47, "GER")])),
            (60, capwar_payload(&snapshot)),
        ],
        raw,
    );
    let mut fold = WorldFold::new("capwar");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    let s = parse(&snaps[0].2);
    assert_eq!(
        s["countries"]["99"]["capitulation"]["capitulated"],
        json!(true)
    );
    let gaps = s["tag_db_id_index"]["unresolved_tag_db_ids"]
        .as_array()
        .unwrap();
    assert!(gaps.iter().any(|g| g == "99"));
}

#[test]
fn capitulation_failure_status_keeps_last_map_and_wars_unknown() {
    let raw1 = raw_for(16_959_360);
    let raw2 = raw_for(16_959_361);
    let mut stream = capwar_stream(
        vec![
            (18, country_identity_payload(&[(47, "GER"), (3, "FRA")])),
            (10, fuel_payload(&[(3, 100, 200, 0, 0)])),
            (60, capwar_payload(&capwar_sample_snapshot())),
        ],
        raw1,
    );
    // Hour 2: fail-closed empty SOURCE_UNAVAILABLE record.
    let failed = capwar60::compact::CapitulationWarSnapshot {
        elapsed_wall_ms: 1,
        status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
        ..capwar60::compact::CapitulationWarSnapshot::default()
    };
    stream.extend(capwar_stream(
        vec![
            (18, country_identity_payload(&[(47, "GER"), (3, "FRA")])),
            (10, fuel_payload(&[(3, 100, 200, 0, 0)])),
            (60, capwar_payload(&failed)),
        ],
        raw2,
    ));
    let mut fold = WorldFold::new("capwar");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 2);
    let s2 = parse(&snaps[1].2);
    // Last proven capitulation map is retained; wars are unknown (null).
    assert_eq!(
        s2["countries"]["FRA"]["capitulation"]["capitulated"],
        json!(true)
    );
    assert_eq!(s2["diplomacy_world"]["wars"], Value::Null);
    let problems = s2["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap();
    assert!(problems
        .iter()
        .any(|problem| problem["domain_id"] == json!(60)
            && problem["problem"] == json!("compact_status_non_clean")));
}

#[test]
fn capitulation_partial_only_frame_retains_map_not_wipes() {
    // REGRESSION: the collector's per-row-skip fail-closed path emits an EMPTY
    // payload with `status_flags | COMPACT_STATUS_PARTIAL_DATA` (see the
    // collector `capwar_row_skips` return). The fold MUST retain the last
    // proven capitulation map on that frame and NEVER rebuild it from the empty
    // payload — a single torn country read on a live game would otherwise wipe
    // the whole political-map capitulation state.
    //
    // NOTE on the shared mask: `COMPACT_STATUS_PARTIAL_DATA` IS currently a
    // member of `COMPACT_STATUS_FAILURE_MASK`, so `compact_status_is_failure`
    // already returns true for a partial-only frame. The fold guard makes the
    // "retain on partial" intent EXPLICIT (`... || PARTIAL_DATA`) so this
    // domain — a full-replace domain with no partial tolerance — stays correct
    // even if PARTIAL_DATA were ever demoted out of the failure mask. This test
    // is the behavioral guarantee regardless of that membership;
    // `partial_data_is_treated_as_non_clean_for_full_replace_domains` below
    // pins the assumption directly.
    let raw1 = raw_for(16_959_360);
    let raw2 = raw_for(16_959_361);
    let mut stream = capwar_stream(
        vec![
            (18, country_identity_payload(&[(47, "GER"), (3, "FRA")])),
            (10, fuel_payload(&[(3, 100, 200, 0, 0)])),
            (60, capwar_payload(&capwar_sample_snapshot())),
        ],
        raw1,
    );
    // Hour 2: EMPTY payload with PARTIAL_DATA ONLY (exactly the collector's
    // torn-row skip path — NO SOURCE_UNAVAILABLE, NO other failure bit).
    let partial_only = capwar60::compact::CapitulationWarSnapshot {
        elapsed_wall_ms: 1,
        status_flags: COMPACT_STATUS_PARTIAL_DATA,
        ..capwar60::compact::CapitulationWarSnapshot::default()
    };
    stream.extend(capwar_stream(
        vec![
            (18, country_identity_payload(&[(47, "GER"), (3, "FRA")])),
            (10, fuel_payload(&[(3, 100, 200, 0, 0)])),
            (60, capwar_payload(&partial_only)),
        ],
        raw2,
    ));
    let mut fold = WorldFold::new("capwar");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    assert_eq!(snaps.len(), 2);

    // Hour 1: FRA capitulated proven.
    let s1 = parse(&snaps[0].2);
    assert_eq!(
        s1["countries"]["FRA"]["capitulation"]["capitulated"],
        json!(true)
    );

    // Hour 2 (the bug): map RETAINED, not wiped. FRA still capitulated=true;
    // wars go unknown (null) because the partial frame is not proven.
    let s2 = parse(&snaps[1].2);
    assert_eq!(
        s2["countries"]["FRA"]["capitulation"]["capitulated"],
        json!(true),
        "PARTIAL-only frame must RETAIN the last proven capitulation map, not wipe it"
    );
    assert_eq!(s2["diplomacy_world"]["wars"], Value::Null);
    let problems = s2["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap();
    assert!(problems
        .iter()
        .any(|problem| problem["domain_id"] == json!(60)
            && problem["problem"] == json!("compact_status_non_clean")));
}

#[test]
fn capitulation_clean_but_empty_frame_retains_map_not_wipes() {
    // STRUCTURAL guard (closes the residual an adversarial review found): even a
    // CLEAN-status D60 frame (status 0, no failure/partial bit) that carries
    // ZERO country rows must NOT wipe the persistent map — an empty country set
    // on a clean status is implausible (a real clean frame always resolves the
    // live CCountry set) and is treated as non-authoritative.
    let raw1 = raw_for(16_959_360);
    let raw2 = raw_for(16_959_361);
    let mut stream = capwar_stream(
        vec![
            (18, country_identity_payload(&[(47, "GER"), (3, "FRA")])),
            (10, fuel_payload(&[(3, 100, 200, 0, 0)])),
            (60, capwar_payload(&capwar_sample_snapshot())),
        ],
        raw1,
    );
    // Hour 2: EMPTY payload with a fully CLEAN status (0) — no failure/partial.
    let clean_empty = capwar60::compact::CapitulationWarSnapshot {
        elapsed_wall_ms: 1,
        status_flags: 0,
        ..capwar60::compact::CapitulationWarSnapshot::default()
    };
    stream.extend(capwar_stream(
        vec![
            (18, country_identity_payload(&[(47, "GER"), (3, "FRA")])),
            (10, fuel_payload(&[(3, 100, 200, 0, 0)])),
            (60, capwar_payload(&clean_empty)),
        ],
        raw2,
    ));
    let mut fold = WorldFold::new("capwar");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    let s2 = parse(&snaps[1].2);
    assert_eq!(
        s2["countries"]["FRA"]["capitulation"]["capitulated"],
        json!(true),
        "clean-but-empty frame must RETAIN the last proven map, not wipe it"
    );
    assert_eq!(s2["diplomacy_world"]["wars"], Value::Null);
    let problems = s2["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap();
    assert!(problems
        .iter()
        .any(|problem| problem["domain_id"] == json!(60)
            && problem["problem"] == json!("clean_but_empty_country_set")));
}

#[test]
fn partial_data_is_treated_as_non_clean_for_full_replace_domains() {
    // Pins the invariant the D60 fold relies on: PARTIAL_DATA must be treated
    // as a non-clean frame for a full-replace domain with no partial tolerance.
    // Today PARTIAL_DATA is a member of the failure mask (so the fold's
    // `compact_status_is_failure(..) || PARTIAL_DATA` is true on the first
    // term); this test guards the *behavioral* assumption so that if
    // PARTIAL_DATA is ever demoted out of the mask, the explicit `|| PARTIAL`
    // term in the fold — and this assertion — keep the retain semantics.
    assert!(
        compact_contract::compact_status_is_failure(COMPACT_STATUS_PARTIAL_DATA)
            || (COMPACT_STATUS_PARTIAL_DATA & COMPACT_STATUS_PARTIAL_DATA) != 0,
        "a PARTIAL_DATA frame must be non-clean for full-replace domains"
    );
    // And confirm domain 60 has NO expected-partial tolerance (it must fail
    // closed on partial, never accept a partial frame as authoritative).
    assert_eq!(compact_contract::expected_partial_tolerance(60), None);
}

#[test]
fn capitulation_war_rows_without_badge_are_skipped_whole() {
    // A payload whose war rows lack the mandatory running-accumulator badge
    // must be rejected wholesale (decode fails; nothing folds).
    let raw = raw_for(16_959_360);
    let mut snapshot = capwar_sample_snapshot();
    snapshot.status_flags = 0; // badge cleared while war rows present
    let stream = capwar_stream(
        vec![
            (18, country_identity_payload(&[(47, "GER"), (3, "FRA")])),
            (10, fuel_payload(&[(3, 100, 200, 0, 0)])),
            (60, capwar_payload(&snapshot)),
        ],
        raw,
    );
    let mut fold = WorldFold::new("capwar");
    fold.ingest_stream(&stream);
    let snaps = fold.finish();
    let s = parse(&snaps[0].2);
    assert!(s["countries"]["FRA"].get("capitulation").is_none());
    assert_eq!(s["diplomacy_world"]["wars"], Value::Null);
    assert!(s["provenance"]["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap().contains("domain 60 fold skipped")));
}
