use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(test))]
use std::env;
#[cfg(not(test))]
use std::fs;

#[allow(dead_code)]
#[path = "../../compact_contract/src/build_profile.rs"]
mod compact_build_profile;
#[path = "../../compact_contract/src/record_header.rs"]
mod compact_record_header;
#[path = "../../compact_contract/src/record_semantics.rs"]
mod compact_record_semantics;
#[allow(dead_code)]
#[path = "../../compact_contract/src/status_flags_base.rs"]
mod compact_status_flags;
use compact_record_header::{parse_compact_record_header, RecordFramingError};
pub use compact_record_header::{
    COMPACT_RECORD_HEADER_LEN as STREAM_HEADER_LEN, COMPACT_RECORD_MAGIC as STREAM_MAGIC,
    COMPACT_RECORD_VERSION as STREAM_VERSION,
};
use compact_record_semantics::validate_compact_record_semantics;
#[cfg(test)]
use compact_record_semantics::{
    COMPACT_FRAME_END_PAYLOAD_LEN as FRAME_END_PAYLOAD_LEN,
    COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER as RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
};

pub const DOMAIN_ID: u16 = 20;
pub const DOMAIN_NAME: &str = "divisions";
pub const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const RECORD_TYPE_FRAME_END: u16 = 2;
pub const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
pub const DOMAIN_FRAME: u16 = 1000;
/// Reserved instrumentation PERF domain. It is physically framed but never a
/// scheduled/emitted coverage domain and never contributes FRAME_END status.
pub const PROBE_DOMAIN_ID: u16 = 250;

pub const PAYLOAD_HEADER_LEN: usize = 32;
pub const ROW_LEN: usize = 120;
pub const LEGACY_ROW_LEN: usize = 104;
pub const FIXED_SCALE_RAW: u32 = 100_000;
pub const DIVISION_TYPE: u32 = 4_713;

pub use compact_status_flags::{
    COMPACT_STATUS_BRIDGE_UNAVAILABLE as STATUS_BRIDGE_UNAVAILABLE,
    COMPACT_STATUS_CONTROL_INVALID as STATUS_CONTROL_INVALID,
    COMPACT_STATUS_DELTA_DATA as STATUS_DELTA_DATA, COMPACT_STATUS_NO_CLOCK as STATUS_NO_CLOCK,
    COMPACT_STATUS_NO_SCALAR_SOURCE as STATUS_NO_SCALAR_SOURCE,
    COMPACT_STATUS_PARTIAL_DATA as STATUS_PARTIAL_DATA,
    COMPACT_STATUS_PROFILE_MISMATCH as STATUS_PROFILE_MISMATCH,
    COMPACT_STATUS_SOURCE_UNAVAILABLE as STATUS_SOURCE_UNAVAILABLE,
    COMPACT_STATUS_STATE_UNAVAILABLE as STATUS_STATE_UNAVAILABLE,
    COMPACT_STATUS_WORKER_PANIC as STATUS_WORKER_PANIC,
};
pub const FAILURE_STATUS_MASK: u32 = STATUS_STATE_UNAVAILABLE
    | STATUS_PROFILE_MISMATCH
    | STATUS_NO_CLOCK
    | STATUS_NO_SCALAR_SOURCE
    | STATUS_WORKER_PANIC
    | STATUS_BRIDGE_UNAVAILABLE
    | STATUS_SOURCE_UNAVAILABLE
    | STATUS_PARTIAL_DATA
    | STATUS_CONTROL_INVALID;

const fn lane_local_status_mask(domain_id: u16) -> u32 {
    match domain_id {
        49 => 1 << 11,
        51 => compact_status_flags::DOMAIN51_LANE_LOCAL_STATUS_MASK,
        _ => 0,
    }
}

const fn status_for_global_interpretation(domain_id: u16, status_flags: u32) -> u32 {
    status_flags & !lane_local_status_mask(domain_id)
}

pub const ROW_FLAG_MOVING: u32 = 1;
pub const ROW_FLAG_PROGRESS: u32 = 1 << 1;
pub const ROW_FLAG_MOVE_TARGET: u32 = 1 << 2;
pub const ROW_FLAG_MOVE_SOURCE: u32 = 1 << 3;
pub const ROW_FLAG_IN_COMBAT: u32 = 1 << 4;
pub const ROW_FLAG_OWNER: u32 = 1 << 5;
pub const ROW_FLAG_ARMY: u32 = 1 << 6;
pub const ROW_FLAG_EXPERIENCE: u32 = 1 << 7;
pub const ROW_FLAG_MAX_SUPPLY: u32 = 1 << 8;
pub const ROW_FLAG_CURRENT_SUPPLY: u32 = 1 << 9;
pub const ROW_FLAG_SUPPLY_GAIN: u32 = 1 << 10;
pub const ROW_FLAG_FUEL: u32 = 1 << 11;
pub const ROW_FLAG_DELETED: u32 = 1 << 12;
pub const ROW_FLAG_TEMPLATE: u32 = 1 << 13;
/// Wave-3 primary-order linkage flags (mutually exclusive). ORDER_ARMY
/// carries the COrderInstance+0x244 instance id in the row tail u32 at +84
/// (previously reserved-zero); ARMY_GROUP/UNRESOLVED badge without an id.
pub const ROW_FLAG_ORDER_ARMY: u32 = 1 << 14;
pub const ROW_FLAG_ORDER_ARMY_GROUP: u32 = 1 << 15;
pub const ROW_FLAG_ORDER_UNRESOLVED: u32 = 1 << 16;
pub const ROW_FLAG_DISRUPTED_SUPPLY: u32 = 1 << 17;
pub const ROW_FLAG_STRATEGIC_REDEPLOYMENT: u32 = 1 << 18;
pub const ROW_FLAG_CURRENT_MANPOWER: u32 = 1 << 19;
pub const ROW_FLAG_CUMULATIVE_LOSSES: u32 = 1 << 20;
pub const ROW_FLAG_MAX_ORGANISATION: u32 = 1 << 21;
pub const ROW_FLAG_COUNTER_SUPPLY_STATUS: u32 = 1 << 22;
pub const ROW_FLAG_DIG_IN: u32 = 1 << 23;
pub const ROW_FLAG_DIG_IN_CAP: u32 = 1 << 24;
pub const ROW_FLAG_TRAINING_OBSERVED: u32 = 1 << 25;
pub const ROW_FLAG_TRAINING_VISIBLE: u32 = 1 << 26;
pub const ROW_FLAG_DIG_IN_UI_OBSERVED: u32 = 1 << 27;
pub const ROW_FLAG_DIG_IN_UI_VISIBLE: u32 = 1 << 28;
pub const ROW_FLAG_DIG_IN_UI_CAPPED: u32 = 1 << 29;
pub const ROW_FLAG_INVALID_ORDERS_OBSERVED: u32 = 1 << 30;
pub const ROW_FLAG_INVALID_ORDERS_VISIBLE: u32 = 1 << 31;
pub const KNOWN_ROW_FLAGS: u32 = ROW_FLAG_MOVING
    | ROW_FLAG_PROGRESS
    | ROW_FLAG_MOVE_TARGET
    | ROW_FLAG_MOVE_SOURCE
    | ROW_FLAG_IN_COMBAT
    | ROW_FLAG_OWNER
    | ROW_FLAG_ARMY
    | ROW_FLAG_EXPERIENCE
    | ROW_FLAG_MAX_SUPPLY
    | ROW_FLAG_CURRENT_SUPPLY
    | ROW_FLAG_SUPPLY_GAIN
    | ROW_FLAG_FUEL
    | ROW_FLAG_DELETED
    | ROW_FLAG_TEMPLATE
    | ROW_FLAG_ORDER_ARMY
    | ROW_FLAG_ORDER_ARMY_GROUP
    | ROW_FLAG_ORDER_UNRESOLVED
    | ROW_FLAG_DISRUPTED_SUPPLY
    | ROW_FLAG_STRATEGIC_REDEPLOYMENT
    | ROW_FLAG_CURRENT_MANPOWER
    | ROW_FLAG_CUMULATIVE_LOSSES
    | ROW_FLAG_MAX_ORGANISATION
    | ROW_FLAG_COUNTER_SUPPLY_STATUS
    | ROW_FLAG_DIG_IN
    | ROW_FLAG_DIG_IN_CAP
    | ROW_FLAG_TRAINING_OBSERVED
    | ROW_FLAG_TRAINING_VISIBLE
    | ROW_FLAG_DIG_IN_UI_OBSERVED
    | ROW_FLAG_DIG_IN_UI_VISIBLE
    | ROW_FLAG_DIG_IN_UI_CAPPED
    | ROW_FLAG_INVALID_ORDERS_OBSERVED
    | ROW_FLAG_INVALID_ORDERS_VISIBLE;
/// Shared ceiling for the primary-order instance id (collector, recorder
/// summary decoder, world_fold, and this standalone gate all enforce it).
pub const MAX_ORDER_INSTANCE_ID: u32 = 100_000_000;

/// The province id space's STRUCTURAL width (lane IA). Was a guess --
/// `65_535` or `1_000_000`, depending on which copy you read; the tree carried
/// four different ceilings for one id space. Proven from the engine's signed-int
/// subscript law at `ai_area.cpp:0xcc` (`claim_347234c20a542ca79032`); dossier
/// `extractor/native/runs/ia_province_caps/SWEEP.md`.
pub const MAX_PROVINCE_ID: u32 = (i32::MAX as u32) - 1;
pub const MAX_COMBAT_REF_COUNT: u32 = 1_000;
pub const MAX_ORGANISATION_RAW: u32 = 100_000_000;
pub const MAX_STRENGTH_RAW: u32 = 1_000_000_000;
// Path-imported from the dependency-free production build profile so the
// documented direct-rustc gate cannot drift from collector/recorder authority.
pub const MAX_OWNER_TAG_DB_ID: u32 = compact_build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
pub const MAX_ARMY_ID: u32 = i32::MAX as u32;
pub const MAX_TEMPLATE_ID: u32 = i32::MAX as u32;
pub const MAX_SUPPLY_RAW: u32 = 100_000_000;
pub const MAX_SUPPLY_GAIN_RAW: u32 = 500_000;
pub const MAX_DISRUPTED_SUPPLY_RAW: u32 = 1_000_000;
pub const MAX_CURRENT_MANPOWER: u32 = 10_000_000;
pub const MAX_CUMULATIVE_LOSSES: u32 = 100_000_000;
pub const MAX_COUNTER_SUPPLY_STATUS_RAW: u32 = 100_000;
pub const MAX_DIG_IN_RAW: u32 = 100_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DivisionStateHeader {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub changed_rows: u32,
    pub root_rows: u32,
}

/// Wave-3 primary-order linkage (CDivision+0xC8). Only the primary order is
/// represented; ArmyGroup/Unresolved badge distinctly without an id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DivisionOrderLink {
    #[default]
    None,
    ArmyOrder(u32),
    ArmyGroup,
    Unresolved,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DivisionStateRow {
    pub div_id: u32,
    pub deleted: bool,
    pub moving: bool,
    pub location: u32,
    pub move_target_province: Option<u32>,
    pub movement_source_province: Option<u32>,
    pub raw_movement_progress: Option<u32>,
    pub progress_bucket: Option<u32>,
    pub raw_organisation: u32,
    pub raw_max_organisation: Option<u32>,
    pub organisation_bucket: u32,
    pub raw_strength: u32,
    pub strength_bucket: u32,
    pub combat_ref_count: u32,
    pub template_id: Option<u32>,
    pub owner_tag_db_id: Option<u32>,
    pub army_id: Option<u32>,
    pub raw_experience: Option<u32>,
    pub raw_max_supply: Option<u32>,
    pub raw_counter_supply_status: Option<u32>,
    pub raw_current_supply: Option<u32>,
    pub raw_supply_gain: Option<u32>,
    pub raw_fuel: Option<u32>,
    pub order_link: DivisionOrderLink,
    pub raw_disrupted_supply: Option<u32>,
    pub raw_dig_in: Option<u32>,
    pub raw_dig_in_cap: Option<u32>,
    pub counter_training_visible: Option<bool>,
    /// 0 hidden, 1 visible/not-all-capped, 2 visible/all-positive capped.
    pub counter_dig_in_frame: Option<u8>,
    /// Exact post-update `invalid_orders` GUI control visibility.
    pub counter_invalid_orders_visible: Option<bool>,
    pub strategic_redeployment: Option<bool>,
    pub current_manpower: Option<u32>,
    pub cumulative_losses: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain20GateSummary {
    pub stream_bytes: usize,
    pub record_count: u64,
    pub frame_end_record_count: u64,
    pub domain_perf_record_count: u64,
    pub domain_data_record_count: u64,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub status_flags_or: u32,
    pub domains_seen: BTreeSet<u16>,
    pub unique_hours: BTreeSet<i32>,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub max_payload_len: usize,
    pub max_read_time_us: u64,
    pub minimum_game_hour_advancements: Option<i64>,
    pub observed_game_hour_advancements: Option<i64>,
    pub required_closed_frame_count: Option<u64>,
    pub production_certified: bool,
    pub ordering_errors: Vec<String>,
    pub parse_errors: Vec<String>,
}

#[derive(Default)]
struct LifecycleTracker {
    initialized: bool,
    live_divisions: BTreeSet<u32>,
    baseline_rows: u64,
    created_rows: u64,
    updated_rows: u64,
    deleted_rows: u64,
}

impl Domain20GateSummary {
    pub fn merge_domain_payload(&mut self, other: Domain20GateSummary) {
        self.data_rows_total = self.data_rows_total.saturating_add(other.data_rows_total);
        self.data_changed_rows_total = self
            .data_changed_rows_total
            .saturating_add(other.data_changed_rows_total);
        self.status_flags_or |= other.status_flags_or;
        for (key, value) in other.data_units_total {
            add_unit(self, key, value as usize);
        }
        self.parse_errors.extend(other.parse_errors);
    }

    pub fn has_failures(&self) -> bool {
        !self.parse_errors.is_empty()
            || !self.ordering_errors.is_empty()
            || (self.status_flags_or & FAILURE_STATUS_MASK) != 0
    }

    pub fn to_json(&self) -> String {
        let units = self
            .data_units_total
            .iter()
            .map(|(key, value)| format!("\"{}\":{}", json_escape(key), value))
            .collect::<Vec<_>>()
            .join(",");
        let domains = self
            .domains_seen
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let errors = self
            .parse_errors
            .iter()
            .chain(self.ordering_errors.iter())
            .map(|value| format!("\"{}\"", json_escape(value)))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            concat!(
                "{{",
                "\"domain_id\":{},",
                "\"domain_name\":\"{}\",",
                "\"stream_bytes\":{},",
                "\"record_count\":{},",
                "\"frame_end_record_count\":{},",
                "\"domain_perf_record_count\":{},",
                "\"domain_data_record_count\":{},",
                "\"data_rows_total\":{},",
                "\"data_changed_rows_total\":{},",
                "\"status_flags_or\":{},",
                "\"has_failures\":{},",
                "\"domains_seen\":[{}],",
                "\"unique_game_hour_count\":{},",
                "\"max_payload_len\":{},",
                "\"max_read_time_us\":{},",
                "\"minimum_game_hour_advancements\":{},",
                "\"observed_game_hour_advancements\":{},",
                "\"required_closed_frame_count\":{},",
                "\"production_certified\":{},",
                "\"data_units_total\":{{{}}},",
                "\"errors\":[{}]",
                "}}"
            ),
            DOMAIN_ID,
            DOMAIN_NAME,
            self.stream_bytes,
            self.record_count,
            self.frame_end_record_count,
            self.domain_perf_record_count,
            self.domain_data_record_count,
            self.data_rows_total,
            self.data_changed_rows_total,
            self.status_flags_or,
            self.has_failures(),
            domains,
            self.unique_hours.len(),
            self.max_payload_len,
            self.max_read_time_us,
            option_i64_json(self.minimum_game_hour_advancements),
            option_i64_json(self.observed_game_hour_advancements),
            option_u64_json(self.required_closed_frame_count),
            self.production_certified,
            units,
            errors
        )
    }
}

pub fn encode_payload(header: &DivisionStateHeader, rows: &[DivisionStateRow]) -> Vec<u8> {
    let mut payload =
        Vec::with_capacity(PAYLOAD_HEADER_LEN.saturating_add(rows.len().saturating_mul(ROW_LEN)));
    payload.extend_from_slice(&header.elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&header.changed_rows.to_le_bytes());
    payload.extend_from_slice(&header.status_flags.to_le_bytes());
    payload.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&header.root_rows.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for row in rows {
        let flags = row_flags(row);
        payload.extend_from_slice(&row.div_id.to_le_bytes());
        payload.extend_from_slice(&DIVISION_TYPE.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.location.to_le_bytes());
        payload.extend_from_slice(&row.move_target_province.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.movement_source_province.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_movement_progress.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.progress_bucket.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_organisation.to_le_bytes());
        payload.extend_from_slice(&row.organisation_bucket.to_le_bytes());
        payload.extend_from_slice(&row.raw_strength.to_le_bytes());
        payload.extend_from_slice(&row.strength_bucket.to_le_bytes());
        payload.extend_from_slice(&row.combat_ref_count.to_le_bytes());
        payload.extend_from_slice(&row.template_id.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.army_id.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_experience.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_max_supply.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_current_supply.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_supply_gain.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_fuel.unwrap_or(0).to_le_bytes());
        let order_instance_id = match row.order_link {
            DivisionOrderLink::ArmyOrder(instance_id) if !row.deleted => instance_id,
            _ => 0,
        };
        payload.extend_from_slice(&order_instance_id.to_le_bytes());
        payload.extend_from_slice(&row.raw_disrupted_supply.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(
            &(match row.strategic_redeployment {
                Some(true) => 1u32,
                Some(false) | None => 0u32,
            })
            .to_le_bytes(),
        );
        payload.extend_from_slice(&row.current_manpower.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.cumulative_losses.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_max_organisation.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_counter_supply_status.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_dig_in.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_dig_in_cap.unwrap_or(0).to_le_bytes());
    }
    payload
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> Domain20GateSummary {
    let mut summary = Domain20GateSummary::default();
    if payload.len() < PAYLOAD_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions payload is {} byte(s), expected at least {PAYLOAD_HEADER_LEN}",
            payload.len()
        ));
        return summary;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    let root_rows = read_u32_le(payload, 24) as usize;
    let reserved0 = read_u32_le(payload, 28);
    summary.status_flags_or |= status_for_global_interpretation(DOMAIN_ID, status_flags);
    let unknown_status = compact_status_flags::compact_status_unknown_flags_v1(status_flags);
    if unknown_status != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions DATA status flags {status_flags:#010x} contain unknown bits {unknown_status:#010x}"
        ));
    }
    if scale_raw != FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions scale {scale_raw} != expected {FIXED_SCALE_RAW}"
        ));
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions header reserved field is nonzero"
        ));
    }
    let legacy_expected =
        PAYLOAD_HEADER_LEN.saturating_add(row_count.saturating_mul(LEGACY_ROW_LEN));
    let extended_expected = PAYLOAD_HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    let row_len = if payload.len() == extended_expected {
        ROW_LEN
    } else if payload.len() == legacy_expected {
        LEGACY_ROW_LEN
    } else {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions payload is {} byte(s), expected legacy {legacy_expected} or extended {extended_expected}",
            payload.len()
        ));
        return summary;
    };
    let extended = row_len == ROW_LEN;
    let delta_data = (status_flags & STATUS_DELTA_DATA) != 0;
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if delta_data && changed_rows != row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions delta changed_rows {changed_rows} != row_count {row_count}"
        ));
    }
    if !delta_data && row_count > root_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions row_count {row_count} > root_rows {root_rows}"
        ));
    }

    summary.data_rows_total = row_count as u64;
    summary.data_changed_rows_total = changed_rows as u64;
    add_unit(&mut summary, "division_rows", row_count);
    add_unit(&mut summary, "division_root_rows", root_rows);

    let mut cursor = PAYLOAD_HEADER_LEN;
    let mut seen_divisions = BTreeSet::new();
    let mut deleted_rows = 0usize;
    let mut moving_rows = 0usize;
    let mut in_combat_rows = 0usize;
    let mut owner_rows = 0usize;
    let mut army_rows = 0usize;
    let mut template_rows = 0usize;
    let mut org_strength_rows = 0usize;
    let mut experience_rows = 0usize;
    let mut max_supply_rows = 0usize;
    let mut current_supply_rows = 0usize;
    let mut supply_gain_rows = 0usize;
    let mut fuel_rows = 0usize;
    let mut delta_upsert_rows = 0usize;
    let mut reserved_tail_zero_rows = 0usize;
    let mut order_army_rows = 0usize;
    let mut order_army_group_rows = 0usize;
    let mut order_unresolved_rows = 0usize;
    let mut disrupted_supply_rows = 0usize;
    let mut strategic_redeployment_rows = 0usize;
    let mut current_manpower_rows = 0usize;
    let mut cumulative_losses_rows = 0usize;
    let mut max_organisation_rows = 0usize;
    let mut counter_supply_status_rows = 0usize;
    let mut dig_in_rows = 0usize;
    let mut dig_in_cap_rows = 0usize;
    let mut training_ui_rows = 0usize;
    let mut training_visible_rows = 0usize;
    let mut dig_in_ui_rows = 0usize;
    let mut dig_in_ui_visible_rows = 0usize;

    for row_index in 0..row_count {
        let div_id = read_u32_le(payload, cursor);
        let div_type = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let location = read_u32_le(payload, cursor + 12);
        let move_target = read_u32_le(payload, cursor + 16);
        let move_source = read_u32_le(payload, cursor + 20);
        let movement_progress = read_u32_le(payload, cursor + 24);
        let progress_bucket = read_u32_le(payload, cursor + 28);
        let organisation_raw = read_u32_le(payload, cursor + 32);
        let organisation_bucket = read_u32_le(payload, cursor + 36);
        let strength_raw = read_u32_le(payload, cursor + 40);
        let strength_bucket = read_u32_le(payload, cursor + 44);
        let combat_ref_count = read_u32_le(payload, cursor + 48);
        let template_id = read_u32_le(payload, cursor + 52);
        let owner_tag_db_id = read_u32_le(payload, cursor + 56);
        let army_id = read_u32_le(payload, cursor + 60);
        let experience_raw = read_u32_le(payload, cursor + 64);
        let max_supply_raw = read_u32_le(payload, cursor + 68);
        let current_supply_raw = read_u32_le(payload, cursor + 72);
        let supply_gain_raw = read_u32_le(payload, cursor + 76);
        let fuel_raw = read_u32_le(payload, cursor + 80);
        let reserved1 = read_u32_le(payload, cursor + 84);
        let disrupted_supply_raw = read_u32_le(payload, cursor + 88);
        let strategic_redeployment_raw = read_u32_le(payload, cursor + 92);
        let current_manpower = read_u32_le(payload, cursor + 96);
        let cumulative_losses = read_u32_le(payload, cursor + 100);
        let max_organisation_raw = extended.then(|| read_u32_le(payload, cursor + 104)).unwrap_or(0);
        let counter_supply_status_raw = extended.then(|| read_u32_le(payload, cursor + 108)).unwrap_or(0);
        let dig_in_raw = extended.then(|| read_u32_le(payload, cursor + 112)).unwrap_or(0);
        let dig_in_cap_raw = extended.then(|| read_u32_le(payload, cursor + 116)).unwrap_or(0);
        cursor += row_len;
        reserved_tail_zero_rows = reserved_tail_zero_rows.saturating_add((reserved1 == 0) as usize);

        if div_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has zero div_id"
            ));
        } else if !seen_divisions.insert(div_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} duplicates div_id {div_id}"
            ));
        }
        if div_type != DIVISION_TYPE {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has div_type {div_type}"
            ));
        }
        if flags & !KNOWN_ROW_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has unknown flags {flags}"
            ));
        }

        let deleted = has_flag(flags, ROW_FLAG_DELETED);
        let moving = has_flag(flags, ROW_FLAG_MOVING);
        let has_progress = has_flag(flags, ROW_FLAG_PROGRESS);
        let has_target = has_flag(flags, ROW_FLAG_MOVE_TARGET);
        let has_source = has_flag(flags, ROW_FLAG_MOVE_SOURCE);
        let in_combat = has_flag(flags, ROW_FLAG_IN_COMBAT);
        let has_owner = has_flag(flags, ROW_FLAG_OWNER);
        let has_army = has_flag(flags, ROW_FLAG_ARMY);
        let has_experience = has_flag(flags, ROW_FLAG_EXPERIENCE);
        let has_max_supply = has_flag(flags, ROW_FLAG_MAX_SUPPLY);
        let has_current_supply = has_flag(flags, ROW_FLAG_CURRENT_SUPPLY);
        let has_supply_gain = has_flag(flags, ROW_FLAG_SUPPLY_GAIN);
        let has_fuel = has_flag(flags, ROW_FLAG_FUEL);
        let has_template = has_flag(flags, ROW_FLAG_TEMPLATE);
        let has_disrupted_supply = has_flag(flags, ROW_FLAG_DISRUPTED_SUPPLY);
        let has_strategic_redeployment = has_flag(flags, ROW_FLAG_STRATEGIC_REDEPLOYMENT);
        let has_current_manpower = has_flag(flags, ROW_FLAG_CURRENT_MANPOWER);
        let has_cumulative_losses = has_flag(flags, ROW_FLAG_CUMULATIVE_LOSSES);
        let has_max_organisation = has_flag(flags, ROW_FLAG_MAX_ORGANISATION);
        let has_counter_supply_status = has_flag(flags, ROW_FLAG_COUNTER_SUPPLY_STATUS);
        let has_dig_in = has_flag(flags, ROW_FLAG_DIG_IN);
        let has_dig_in_cap = has_flag(flags, ROW_FLAG_DIG_IN_CAP);
        let training_observed = has_flag(flags, ROW_FLAG_TRAINING_OBSERVED);
        let training_visible = has_flag(flags, ROW_FLAG_TRAINING_VISIBLE);
        let dig_in_ui_observed = has_flag(flags, ROW_FLAG_DIG_IN_UI_OBSERVED);
        let dig_in_ui_visible = has_flag(flags, ROW_FLAG_DIG_IN_UI_VISIBLE);
        let dig_in_ui_capped = has_flag(flags, ROW_FLAG_DIG_IN_UI_CAPPED);
        let invalid_orders_observed = has_flag(flags, ROW_FLAG_INVALID_ORDERS_OBSERVED);
        let invalid_orders_visible = has_flag(flags, ROW_FLAG_INVALID_ORDERS_VISIBLE);
        let a15_flags = ROW_FLAG_MAX_ORGANISATION
            | ROW_FLAG_COUNTER_SUPPLY_STATUS
            | ROW_FLAG_DIG_IN
            | ROW_FLAG_DIG_IN_CAP
            | ROW_FLAG_TRAINING_OBSERVED
            | ROW_FLAG_TRAINING_VISIBLE
            | ROW_FLAG_DIG_IN_UI_OBSERVED
            | ROW_FLAG_DIG_IN_UI_VISIBLE
            | ROW_FLAG_DIG_IN_UI_CAPPED
            | ROW_FLAG_INVALID_ORDERS_OBSERVED
            | ROW_FLAG_INVALID_ORDERS_VISIBLE;
        if !extended && flags & a15_flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: legacy divisions row {row_index} carries extended A15 flags"
            ));
        }

        if deleted {
            deleted_rows = deleted_rows.saturating_add(1);
            if flags != ROW_FLAG_DELETED {
                summary.parse_errors.push(format!(
                    "offset {offset}: divisions delete row {row_index} has non-delete flags"
                ));
            }
            if !zero_filled_deleted_row(
                location,
                move_target,
                move_source,
                movement_progress,
                progress_bucket,
                organisation_raw,
                organisation_bucket,
                strength_raw,
                strength_bucket,
                combat_ref_count,
                template_id,
                owner_tag_db_id,
                army_id,
                experience_raw,
                max_supply_raw,
                current_supply_raw,
                supply_gain_raw,
                fuel_raw,
                reserved1,
                disrupted_supply_raw,
                strategic_redeployment_raw,
                current_manpower,
                cumulative_losses,
                max_organisation_raw,
                counter_supply_status_raw,
                dig_in_raw,
                dig_in_cap_raw,
            ) {
                summary.parse_errors.push(format!(
                    "offset {offset}: divisions delete row {row_index} is not zero-filled"
                ));
            }
            continue;
        }

        if training_visible && !training_observed {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has visible training without an evaluated UI observation"
            ));
        }
        if invalid_orders_visible && !invalid_orders_observed {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has visible invalid-orders without an evaluated UI observation"
            ));
        }
        if dig_in_ui_visible && !dig_in_ui_observed {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has visible dig-in without an evaluated UI observation"
            ));
        }
        if dig_in_ui_capped && (!dig_in_ui_observed || !dig_in_ui_visible) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has capped dig-in without a visible evaluated UI observation"
            ));
        }
        training_ui_rows = training_ui_rows.saturating_add(training_observed as usize);
        training_visible_rows =
            training_visible_rows.saturating_add(training_visible as usize);
        dig_in_ui_rows = dig_in_ui_rows.saturating_add(dig_in_ui_observed as usize);
        dig_in_ui_visible_rows =
            dig_in_ui_visible_rows.saturating_add(dig_in_ui_visible as usize);

        if delta_data {
            delta_upsert_rows = delta_upsert_rows.saturating_add(1);
        }
        org_strength_rows = org_strength_rows.saturating_add(1);
        moving_rows = moving_rows.saturating_add(moving as usize);
        in_combat_rows = in_combat_rows.saturating_add(in_combat as usize);
        owner_rows = owner_rows.saturating_add(has_owner as usize);
        army_rows = army_rows.saturating_add(has_army as usize);
        template_rows = template_rows.saturating_add(has_template as usize);
        experience_rows = experience_rows.saturating_add(has_experience as usize);
        max_supply_rows = max_supply_rows.saturating_add(has_max_supply as usize);
        current_supply_rows = current_supply_rows.saturating_add(has_current_supply as usize);
        supply_gain_rows = supply_gain_rows.saturating_add(has_supply_gain as usize);
        fuel_rows = fuel_rows.saturating_add(has_fuel as usize);

        if !valid_province_id(location) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has invalid location {location}"
            ));
        }
        validate_optional_province(
            &mut summary,
            offset,
            row_index,
            has_target,
            move_target,
            "move_target",
        );
        validate_optional_province(
            &mut summary,
            offset,
            row_index,
            has_source,
            move_source,
            "move_source",
        );
        if !moving && (has_progress || has_target || has_source) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has movement fields while not moving"
            ));
        }
        if has_progress && movement_progress == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has zero movement progress"
            ));
        }
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_progress,
            progress_bucket,
            0,
            u32::MAX,
            "progress_bucket",
        );
        validate_required_u32(
            &mut summary,
            offset,
            row_index,
            organisation_raw,
            0,
            MAX_ORGANISATION_RAW,
            "organisation_raw",
        );
        validate_required_u32(
            &mut summary,
            offset,
            row_index,
            strength_raw,
            0,
            MAX_STRENGTH_RAW,
            "strength_raw",
        );
        validate_required_u32(
            &mut summary,
            offset,
            row_index,
            combat_ref_count,
            0,
            MAX_COMBAT_REF_COUNT,
            "combat_ref_count",
        );
        if in_combat != (combat_ref_count > 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} combat flag does not match combat_ref_count"
            ));
        }
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_template,
            template_id,
            1,
            MAX_TEMPLATE_ID,
            "template_id",
        );
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_owner,
            owner_tag_db_id,
            1,
            MAX_OWNER_TAG_DB_ID,
            "owner_tag_db_id",
        );
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_army,
            army_id,
            1,
            MAX_ARMY_ID,
            "army_id",
        );
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_experience,
            experience_raw,
            0,
            u32::MAX,
            "experience_raw",
        );
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_max_supply,
            max_supply_raw,
            0,
            MAX_SUPPLY_RAW,
            "max_supply_raw",
        );
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_current_supply,
            current_supply_raw,
            0,
            MAX_SUPPLY_RAW,
            "current_supply_raw",
        );
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_supply_gain,
            supply_gain_raw,
            0,
            MAX_SUPPLY_GAIN_RAW,
            "supply_gain_raw",
        );
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_fuel,
            fuel_raw,
            0,
            u32::MAX,
            "fuel_raw",
        );
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_disrupted_supply,
            disrupted_supply_raw,
            0,
            MAX_DISRUPTED_SUPPLY_RAW,
            "disrupted_supply_raw",
        );
        disrupted_supply_rows = disrupted_supply_rows.saturating_add(has_disrupted_supply as usize);
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_strategic_redeployment,
            strategic_redeployment_raw,
            0,
            1,
            "strategic_redeployment_raw",
        );
        strategic_redeployment_rows =
            strategic_redeployment_rows.saturating_add(has_strategic_redeployment as usize);
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_current_manpower,
            current_manpower,
            0,
            MAX_CURRENT_MANPOWER,
            "current_manpower",
        );
        current_manpower_rows = current_manpower_rows.saturating_add(has_current_manpower as usize);
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_cumulative_losses,
            cumulative_losses,
            0,
            MAX_CUMULATIVE_LOSSES,
            "cumulative_losses",
        );
        cumulative_losses_rows =
            cumulative_losses_rows.saturating_add(has_cumulative_losses as usize);
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_max_organisation,
            max_organisation_raw,
            100,
            MAX_ORGANISATION_RAW,
            "max_organisation_raw",
        );
        max_organisation_rows =
            max_organisation_rows.saturating_add(has_max_organisation as usize);
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_counter_supply_status,
            counter_supply_status_raw,
            0,
            MAX_COUNTER_SUPPLY_STATUS_RAW,
            "counter_supply_status_raw",
        );
        counter_supply_status_rows = counter_supply_status_rows
            .saturating_add(has_counter_supply_status as usize);
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_dig_in,
            dig_in_raw,
            0,
            MAX_DIG_IN_RAW,
            "dig_in_raw",
        );
        dig_in_rows = dig_in_rows.saturating_add(has_dig_in as usize);
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_dig_in_cap,
            dig_in_cap_raw,
            0,
            MAX_DIG_IN_RAW,
            "dig_in_cap_raw",
        );
        dig_in_cap_rows = dig_in_cap_rows.saturating_add(has_dig_in_cap as usize);
        let has_order_army = has_flag(flags, ROW_FLAG_ORDER_ARMY);
        let has_order_group = has_flag(flags, ROW_FLAG_ORDER_ARMY_GROUP);
        let has_order_unresolved = has_flag(flags, ROW_FLAG_ORDER_UNRESOLVED);
        if (has_order_army as u32 + has_order_group as u32 + has_order_unresolved as u32) > 1 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has conflicting order-link flags"
            ));
        }
        if has_order_army {
            order_army_rows = order_army_rows.saturating_add(1);
            if reserved1 == 0 || reserved1 > MAX_ORDER_INSTANCE_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: divisions row {row_index} has invalid order instance id {reserved1}"
                ));
            }
        } else {
            order_army_group_rows = order_army_group_rows.saturating_add(has_order_group as usize);
            order_unresolved_rows =
                order_unresolved_rows.saturating_add(has_order_unresolved as usize);
            if reserved1 != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: divisions row {row_index} has order instance id without army-order flag"
                ));
            }
        }
    }

    add_unit(&mut summary, "division_deleted_rows", deleted_rows);
    add_unit(&mut summary, "division_moving_rows", moving_rows);
    add_unit(&mut summary, "division_in_combat_rows", in_combat_rows);
    add_unit(&mut summary, "division_owner_rows", owner_rows);
    add_unit(&mut summary, "division_army_rows", army_rows);
    add_unit(&mut summary, "division_template_rows", template_rows);
    add_unit(
        &mut summary,
        "division_org_strength_rows",
        org_strength_rows,
    );
    add_unit(&mut summary, "division_experience_rows", experience_rows);
    add_unit(&mut summary, "division_max_supply_rows", max_supply_rows);
    add_unit(
        &mut summary,
        "division_current_supply_rows",
        current_supply_rows,
    );
    add_unit(&mut summary, "division_supply_gain_rows", supply_gain_rows);
    add_unit(&mut summary, "division_fuel_rows", fuel_rows);
    add_unit(
        &mut summary,
        "division_max_organisation_rows",
        max_organisation_rows,
    );
    add_unit(
        &mut summary,
        "division_counter_supply_status_rows",
        counter_supply_status_rows,
    );
    add_unit(&mut summary, "division_dig_in_rows", dig_in_rows);
    add_unit(&mut summary, "division_dig_in_cap_rows", dig_in_cap_rows);
    add_unit(&mut summary, "division_training_ui_rows", training_ui_rows);
    add_unit(
        &mut summary,
        "division_training_visible_rows",
        training_visible_rows,
    );
    add_unit(&mut summary, "division_dig_in_ui_rows", dig_in_ui_rows);
    add_unit(
        &mut summary,
        "division_dig_in_ui_visible_rows",
        dig_in_ui_visible_rows,
    );
    add_unit(
        &mut summary,
        "division_delta_upsert_rows",
        delta_upsert_rows,
    );
    add_unit(
        &mut summary,
        "division_reserved_tail_zero_rows",
        reserved_tail_zero_rows,
    );
    add_unit(&mut summary, "division_order_army_rows", order_army_rows);
    add_unit(
        &mut summary,
        "division_order_army_group_rows",
        order_army_group_rows,
    );
    add_unit(
        &mut summary,
        "division_order_unresolved_rows",
        order_unresolved_rows,
    );
    add_unit(
        &mut summary,
        "division_disrupted_supply_rows",
        disrupted_supply_rows,
    );
    add_unit(
        &mut summary,
        "division_strategic_redeployment_rows",
        strategic_redeployment_rows,
    );
    add_unit(
        &mut summary,
        "division_current_manpower_rows",
        current_manpower_rows,
    );
    add_unit(
        &mut summary,
        "division_cumulative_losses_rows",
        cumulative_losses_rows,
    );
    summary
}

pub fn decode_stream(raw: &[u8]) -> Domain20GateSummary {
    let mut summary = Domain20GateSummary {
        stream_bytes: raw.len(),
        ..Domain20GateSummary::default()
    };
    let mut offset = 0usize;
    let mut previous_sequence: Option<u64> = None;
    let mut open_hour: Option<i32> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut saw_frame_end = false;
    let mut last_record_was_clean_frame_end = false;
    let mut lifecycle = LifecycleTracker::default();
    while offset < raw.len() {
        let remaining = raw.len().saturating_sub(offset);
        let header = match parse_compact_record_header(&raw[offset..]) {
            Ok(header) => header,
            Err(RecordFramingError::TruncatedHeader(_)) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: truncated stream header"));
                break;
            }
            Err(RecordFramingError::BadMagic) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: invalid stream magic"));
                break;
            }
            Err(RecordFramingError::UnsupportedVersion(version)) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: unsupported stream header version {version} len {STREAM_HEADER_LEN}"
                ));
                break;
            }
            Err(RecordFramingError::UnsupportedHeaderLen(header_len)) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: unsupported stream header version {STREAM_VERSION} len {header_len}"
                ));
                break;
            }
            Err(err @ RecordFramingError::PayloadTooLarge(payload_len)) => {
                let payload_len = payload_len as usize;
                if STREAM_HEADER_LEN.saturating_add(payload_len) > remaining {
                    summary.parse_errors.push(format!(
                        "offset {offset}: payload length {payload_len} exceeds remaining bytes {remaining}"
                    ));
                } else {
                    summary.parse_errors.push(format!("offset {offset}: {err}"));
                }
                break;
            }
        };
        let payload_len = header.payload_len;
        let record_len = header.record_len();
        if record_len > remaining {
            summary.parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining bytes {remaining}"
            ));
            break;
        }
        match previous_sequence {
            None if header.sequence != 1 => summary.ordering_errors.push(format!(
                "offset {offset}: first sequence is {}, expected 1",
                header.sequence
            )),
            Some(previous) => match previous.checked_add(1) {
                Some(expected) if header.sequence != expected => {
                    summary.ordering_errors.push(format!(
                        "offset {offset}: sequence discontinuity: expected {expected}, got {}",
                        header.sequence
                    ))
                }
                None => summary.ordering_errors.push(format!(
                    "offset {offset}: sequence cannot advance after {previous} without overflow"
                )),
                _ => {}
            },
            _ => {}
        }
        previous_sequence = Some(header.sequence);
        summary.record_count = summary.record_count.saturating_add(1);
        summary.domains_seen.insert(header.domain_id);
        summary.max_payload_len = summary.max_payload_len.max(payload_len);
        last_record_was_clean_frame_end = false;

        let payload = &raw[offset + STREAM_HEADER_LEN..offset + record_len];
        let semantics_valid = match validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            payload,
        ) {
            Ok(()) => true,
            Err(error) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: {error}"));
                false
            }
        };
        match header.record_type {
            RECORD_TYPE_FRAME_END if semantics_valid => {
                let open_matches = match open_hour {
                    Some(open) if open == header.game_hour_raw => true,
                    Some(open) => {
                        summary.parse_errors.push(format!(
                            "offset {offset}: frame-end hour {} does not close open hour {open}",
                            header.game_hour_raw
                        ));
                        false
                    }
                    None => {
                        summary.parse_errors.push(format!(
                            "offset {offset}: frame-end hour {} has no open data hour",
                            header.game_hour_raw
                        ));
                        false
                    }
                };
                if let Some(pending) = pending_perf_hour {
                    summary.parse_errors.push(format!(
                        "offset {offset}: frame-end hour {} encountered with pending perf-only hour {pending}",
                        header.game_hour_raw
                    ));
                }
                let successor_matches = closed_hour_is_successor(
                    offset,
                    last_closed_hour,
                    header.game_hour_raw,
                    &mut summary.ordering_errors,
                );
                if open_matches && pending_perf_hour.is_none() {
                    open_hour = None;
                    if successor_matches {
                        saw_frame_end = true;
                        last_closed_hour = Some(header.game_hour_raw);
                        last_record_was_clean_frame_end = true;
                        summary.frame_end_record_count =
                            summary.frame_end_record_count.saturating_add(1);
                        summary.unique_hours.insert(header.game_hour_raw);
                        summary.status_flags_or |= status_for_global_interpretation(
                            DOMAIN_FRAME,
                            read_u32_le(payload, 24),
                        );
                    }
                }
            }
            RECORD_TYPE_DOMAIN_PERF if semantics_valid => {
                match (open_hour, pending_perf_hour) {
                    (Some(open), _) if open == header.game_hour_raw => {}
                    (Some(open), _) => summary.parse_errors.push(format!(
                        "offset {offset}: domain perf hour {} does not match open hour {open}",
                        header.game_hour_raw
                    )),
                    (None, Some(pending)) if pending == header.game_hour_raw => {}
                    (None, Some(pending)) => summary.parse_errors.push(format!(
                        "offset {offset}: domain perf hour {} does not match pending perf hour {pending}",
                        header.game_hour_raw
                    )),
                    (None, None) => pending_perf_hour = Some(header.game_hour_raw),
                }
                if header.domain_id == DOMAIN_ID {
                    summary.domain_perf_record_count =
                        summary.domain_perf_record_count.saturating_add(1);
                    decode_domain_perf(offset, payload, &mut summary);
                }
            }
            RECORD_TYPE_DOMAIN_DATA if semantics_valid => {
                match open_hour {
                    Some(open) if open != header.game_hour_raw => summary.parse_errors.push(format!(
                        "offset {offset}: data hour {} encountered before frame-end closed hour {open}",
                        header.game_hour_raw
                    )),
                    None => match pending_perf_hour {
                        Some(pending) if pending != header.game_hour_raw => summary
                            .parse_errors
                            .push(format!(
                                "offset {offset}: data hour {} does not match pending perf hour {pending}",
                                header.game_hour_raw
                            )),
                        _ => {
                            pending_perf_hour = None;
                            open_hour = Some(header.game_hour_raw);
                        }
                    },
                    _ => {}
                }
                if header.domain_id == DOMAIN_ID {
                    summary.domain_data_record_count =
                        summary.domain_data_record_count.saturating_add(1);
                    let payload_summary = decode_payload(offset, payload);
                    let payload_valid = payload_summary.parse_errors.is_empty();
                    summary.merge_domain_payload(payload_summary);
                    if payload_valid {
                        update_lifecycle_from_payload(
                            offset,
                            payload,
                            &mut lifecycle,
                            &mut summary,
                        );
                    }
                }
            }
            RECORD_TYPE_FRAME_END | RECORD_TYPE_DOMAIN_DATA | RECORD_TYPE_DOMAIN_PERF => {}
            other => summary
                .parse_errors
                .push(format!("offset {offset}: unsupported record type {other}")),
        }
        offset = offset.saturating_add(record_len);
    }
    if !raw.is_empty() {
        if !saw_frame_end {
            summary
                .parse_errors
                .push("stream has no frame-end record".to_string());
        }
        if let Some(open) = open_hour {
            summary
                .parse_errors
                .push(format!("stream ended with open data hour {open}"));
        }
        if !last_record_was_clean_frame_end {
            summary
                .parse_errors
                .push("stream does not end at a clean frame-end boundary".to_string());
        }
    }
    add_unit(
        &mut summary,
        "division_lifecycle_baseline_rows",
        lifecycle.baseline_rows.min(usize::MAX as u64) as usize,
    );
    add_unit(
        &mut summary,
        "division_lifecycle_created_rows",
        lifecycle.created_rows.min(usize::MAX as u64) as usize,
    );
    add_unit(
        &mut summary,
        "division_lifecycle_updated_rows",
        lifecycle.updated_rows.min(usize::MAX as u64) as usize,
    );
    add_unit(
        &mut summary,
        "division_lifecycle_deleted_rows",
        lifecycle.deleted_rows.min(usize::MAX as u64) as usize,
    );
    summary
}

/// Certify a production domain-20 proof for an explicitly requested number of
/// game-hour advancements. Because both endpoints are closed frames, `N`
/// advancements require at least `N + 1` consecutive closed frames.
pub fn gate_production_stream(
    raw: &[u8],
    minimum_game_hour_advancements: i64,
) -> Domain20GateSummary {
    let mut summary = decode_stream(raw);
    summary.minimum_game_hour_advancements = Some(minimum_game_hour_advancements);

    let required_closed_frames = if minimum_game_hour_advancements <= 0 {
        summary.parse_errors.push(format!(
            "minimum game-hour advancements must be explicitly positive, got {minimum_game_hour_advancements}"
        ));
        None
    } else {
        match minimum_game_hour_advancements.checked_add(1) {
            Some(required) => {
                summary.required_closed_frame_count = u64::try_from(required).ok();
                Some(required)
            }
            None => {
                summary.parse_errors.push(format!(
                    "minimum game-hour advancements {minimum_game_hour_advancements} cannot be converted to an inclusive closed-frame requirement"
                ));
                None
            }
        }
    };

    let observed_advancements = match (
        summary.unique_hours.first().copied(),
        summary.unique_hours.last().copied(),
    ) {
        (Some(first), Some(last)) => Some(i64::from(last) - i64::from(first)),
        _ => None,
    };
    summary.observed_game_hour_advancements = observed_advancements;
    if let Some(required_closed_frames) = required_closed_frames {
        let observed_advancements = observed_advancements.unwrap_or(-1);
        let observed_closed_frames = i64::try_from(summary.unique_hours.len()).unwrap_or(i64::MAX);
        if observed_advancements < minimum_game_hour_advancements
            || observed_closed_frames < required_closed_frames
        {
            summary.parse_errors.push(format!(
                "production proof duration is {observed_advancements} advancement(s) across {observed_closed_frames} closed frame(s), below required {minimum_game_hour_advancements} advancement(s) / {required_closed_frames} closed frame(s)"
            ));
        }
    }

    certify_production_frames(raw, &mut summary);
    summary.production_certified = minimum_game_hour_advancements > 0
        && summary.domain_data_record_count > 0
        && summary.domain_perf_record_count > 0
        && summary.frame_end_record_count > 0
        && !summary.has_failures();
    summary
}

fn certify_production_frames(raw: &[u8], summary: &mut Domain20GateSummary) {
    let mut offset = 0usize;
    let mut open_hour: Option<i32> = None;
    let mut owned_data_count = 0u64;
    let mut owned_perf_count = 0u64;
    let mut owned_data_status: Option<u32> = None;
    let mut owned_perf_status: Option<u32> = None;
    let mut perf_status_or = 0u32;
    let mut data_record_count = 0u64;
    let mut perf_record_count = 0u64;
    let mut data_domains = BTreeSet::new();
    let mut perf_domains = BTreeSet::new();

    while offset < raw.len() {
        let header = match parse_compact_record_header(&raw[offset..]) {
            Ok(header) => header,
            Err(_) => break,
        };
        let record_len = header.record_len();
        if record_len > raw.len().saturating_sub(offset) {
            break;
        }
        let payload = &raw[offset + STREAM_HEADER_LEN..offset + record_len];
        let semantics_valid = validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            payload,
        )
        .is_ok();

        match header.record_type {
            RECORD_TYPE_DOMAIN_DATA | RECORD_TYPE_DOMAIN_PERF if semantics_valid => {
                if open_hour.is_none() {
                    open_hour = Some(header.game_hour_raw);
                }
                if header.record_type == RECORD_TYPE_DOMAIN_DATA {
                    if header.domain_id != PROBE_DOMAIN_ID {
                        data_record_count = data_record_count.saturating_add(1);
                        data_domains.insert(header.domain_id);
                    }
                    if header.domain_id == DOMAIN_ID {
                        owned_data_count = owned_data_count.saturating_add(1);
                        if payload.len() >= PAYLOAD_HEADER_LEN {
                            owned_data_status = Some(read_u32_le(payload, 16));
                        }
                    }
                } else {
                    if header.domain_id != PROBE_DOMAIN_ID {
                        perf_record_count = perf_record_count.saturating_add(1);
                        perf_domains.insert(header.domain_id);
                        // Canonical PERF semantics guarantee the 40-byte base.
                        perf_status_or |= status_for_global_interpretation(
                            header.domain_id,
                            read_u32_le(payload, 32),
                        );
                    }
                    if header.domain_id == DOMAIN_ID {
                        owned_perf_count = owned_perf_count.saturating_add(1);
                        owned_perf_status = Some(read_u32_le(payload, 32));
                    }
                }
            }
            RECORD_TYPE_FRAME_END if semantics_valid => {
                if open_hour == Some(header.game_hour_raw) {
                    if owned_data_count != 1 {
                        summary.parse_errors.push(format!(
                            "offset {offset}: closed hour {} has {owned_data_count} divisions DOMAIN_DATA records, expected exactly 1",
                            header.game_hour_raw
                        ));
                    }
                    if owned_perf_count != 1 {
                        summary.parse_errors.push(format!(
                            "offset {offset}: closed hour {} has {owned_perf_count} divisions DOMAIN_PERF records, expected exactly 1",
                            header.game_hour_raw
                        ));
                    }
                    if owned_data_count == 1
                        && owned_perf_count == 1
                        && owned_data_status != owned_perf_status
                    {
                        summary.parse_errors.push(format!(
                            "offset {offset}: closed hour {} divisions DATA status {:?} does not match PERF status {:?}",
                            header.game_hour_raw, owned_data_status, owned_perf_status
                        ));
                    }
                    validate_production_frame_end(
                        offset,
                        payload,
                        &data_domains,
                        &perf_domains,
                        data_record_count,
                        perf_record_count,
                        perf_status_or,
                        &mut summary.parse_errors,
                    );
                }
                open_hour = None;
                owned_data_count = 0;
                owned_perf_count = 0;
                owned_data_status = None;
                owned_perf_status = None;
                perf_status_or = 0;
                data_record_count = 0;
                perf_record_count = 0;
                data_domains.clear();
                perf_domains.clear();
            }
            _ => {}
        }
        offset = offset.saturating_add(record_len);
    }
}

fn validate_production_frame_end(
    offset: usize,
    payload: &[u8],
    data_domains: &BTreeSet<u16>,
    perf_domains: &BTreeSet<u16>,
    data_record_count: u64,
    perf_record_count: u64,
    perf_status_or: u32,
    errors: &mut Vec<String>,
) {
    let emitted_domains = read_u32_le(payload, 16);
    let scheduled_domains = read_u32_le(payload, 20);
    let status_flags = read_u32_le(payload, 24);
    let reserved = read_u32_le(payload, 28);
    if reserved != 0 {
        errors.push(format!(
            "offset {offset}: FRAME_END reserved word is nonzero ({reserved:#010x})"
        ));
    }
    if emitted_domains != scheduled_domains {
        errors.push(format!(
            "offset {offset}: FRAME_END emitted domain count {emitted_domains} does not match scheduled domain count {scheduled_domains}"
        ));
    }
    let data_domain_count = u32::try_from(data_domains.len()).unwrap_or(u32::MAX);
    let perf_domain_count = u32::try_from(perf_domains.len()).unwrap_or(u32::MAX);
    if emitted_domains != data_domain_count || scheduled_domains != perf_domain_count {
        errors.push(format!(
            "offset {offset}: FRAME_END authority {emitted_domains} emitted/{scheduled_domains} scheduled does not match {data_domain_count} DATA/{perf_domain_count} PERF domain(s) in the closed frame"
        ));
    }
    if data_domains != perf_domains {
        errors.push(format!(
            "offset {offset}: closed-frame DATA domains {data_domains:?} do not match PERF domains {perf_domains:?}"
        ));
    }
    if data_record_count != u64::from(emitted_domains)
        || perf_record_count != u64::from(scheduled_domains)
    {
        errors.push(format!(
            "offset {offset}: FRAME_END authority {emitted_domains} emitted/{scheduled_domains} scheduled does not match {data_record_count} non-probe DATA/{perf_record_count} non-probe PERF record(s)"
        ));
    }
    let unknown_status = compact_status_flags::compact_status_unknown_flags_v1(status_flags);
    if unknown_status != 0 {
        errors.push(format!(
            "offset {offset}: FRAME_END status flags {status_flags:#010x} contain unknown bits {unknown_status:#010x}"
        ));
    }
    let fatal_status = status_flags & FAILURE_STATUS_MASK;
    if fatal_status != 0 {
        errors.push(format!(
            "offset {offset}: FRAME_END status flags {status_flags:#010x} contain fatal bits {fatal_status:#010x}"
        ));
    }
    if status_flags != perf_status_or {
        errors.push(format!(
            "offset {offset}: FRAME_END status {status_flags:#010x} does not equal closed-frame PERF status aggregate {perf_status_or:#010x}"
        ));
    }
}

fn closed_hour_is_successor(
    offset: usize,
    previous: Option<i32>,
    current: i32,
    errors: &mut Vec<String>,
) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    match previous.checked_add(1) {
        Some(expected) if current == expected => true,
        Some(expected) => {
            errors.push(format!(
                "offset {offset}: closed hour discontinuity: expected {expected}, got {current}"
            ));
            false
        }
        None => {
            errors.push(format!(
                "offset {offset}: closed hour cannot advance after {previous} without overflow"
            ));
            false
        }
    }
}

fn update_lifecycle_from_payload(
    offset: usize,
    payload: &[u8],
    tracker: &mut LifecycleTracker,
    summary: &mut Domain20GateSummary,
) {
    if payload.len() < PAYLOAD_HEADER_LEN {
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let legacy_expected =
        PAYLOAD_HEADER_LEN.saturating_add(row_count.saturating_mul(LEGACY_ROW_LEN));
    let extended_expected = PAYLOAD_HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    let row_len = if payload.len() == extended_expected {
        ROW_LEN
    } else if payload.len() == legacy_expected {
        LEGACY_ROW_LEN
    } else {
        return;
    };
    let mut cursor = PAYLOAD_HEADER_LEN;
    for row_index in 0..row_count {
        let div_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 8);
        cursor += row_len;
        if div_id == 0 {
            continue;
        }
        let deleted = has_flag(flags, ROW_FLAG_DELETED);
        if !tracker.initialized {
            if deleted {
                summary.parse_errors.push(format!(
                    "offset {offset}: baseline divisions row {row_index} is a delete row"
                ));
            } else if tracker.live_divisions.insert(div_id) {
                tracker.baseline_rows = tracker.baseline_rows.saturating_add(1);
            }
            continue;
        }
        if deleted {
            if tracker.live_divisions.remove(&div_id) {
                tracker.deleted_rows = tracker.deleted_rows.saturating_add(1);
            } else {
                summary.parse_errors.push(format!(
                    "offset {offset}: divisions row {row_index} deletes unknown div_id {div_id}"
                ));
            }
        } else if tracker.live_divisions.insert(div_id) {
            tracker.created_rows = tracker.created_rows.saturating_add(1);
        } else {
            tracker.updated_rows = tracker.updated_rows.saturating_add(1);
        }
    }
    tracker.initialized = true;
}

fn decode_domain_perf(offset: usize, payload: &[u8], summary: &mut Domain20GateSummary) {
    let read_time_us = read_u64_le(payload, 8);
    let status_flags = read_u32_le(payload, 32);
    summary.status_flags_or |= status_for_global_interpretation(DOMAIN_ID, status_flags);
    let unknown_status = compact_status_flags::compact_status_unknown_flags_v1(status_flags);
    if unknown_status != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions PERF status flags {status_flags:#010x} contain unknown bits {unknown_status:#010x}"
        ));
    }
    summary.max_read_time_us = summary.max_read_time_us.max(read_time_us);
}

fn row_flags(row: &DivisionStateRow) -> u32 {
    if row.deleted {
        return ROW_FLAG_DELETED;
    }
    let mut flags = 0u32;
    if row.moving {
        flags |= ROW_FLAG_MOVING;
    }
    if row.raw_movement_progress.is_some() {
        flags |= ROW_FLAG_PROGRESS;
    }
    if row.move_target_province.is_some() {
        flags |= ROW_FLAG_MOVE_TARGET;
    }
    if row.movement_source_province.is_some() {
        flags |= ROW_FLAG_MOVE_SOURCE;
    }
    if row.combat_ref_count > 0 {
        flags |= ROW_FLAG_IN_COMBAT;
    }
    if row.owner_tag_db_id.is_some() {
        flags |= ROW_FLAG_OWNER;
    }
    if row.army_id.is_some() {
        flags |= ROW_FLAG_ARMY;
    }
    if row.raw_experience.is_some() {
        flags |= ROW_FLAG_EXPERIENCE;
    }
    if row.raw_max_supply.is_some() {
        flags |= ROW_FLAG_MAX_SUPPLY;
    }
    if row.raw_current_supply.is_some() {
        flags |= ROW_FLAG_CURRENT_SUPPLY;
    }
    if row.raw_supply_gain.is_some() {
        flags |= ROW_FLAG_SUPPLY_GAIN;
    }
    if row.raw_fuel.is_some() {
        flags |= ROW_FLAG_FUEL;
    }
    if row.template_id.is_some() {
        flags |= ROW_FLAG_TEMPLATE;
    }
    if row.raw_disrupted_supply.is_some() {
        flags |= ROW_FLAG_DISRUPTED_SUPPLY;
    }
    if row.strategic_redeployment.is_some() {
        flags |= ROW_FLAG_STRATEGIC_REDEPLOYMENT;
    }
    if row.current_manpower.is_some() {
        flags |= ROW_FLAG_CURRENT_MANPOWER;
    }
    if row.cumulative_losses.is_some() {
        flags |= ROW_FLAG_CUMULATIVE_LOSSES;
    }
    if row.raw_max_organisation.is_some() {
        flags |= ROW_FLAG_MAX_ORGANISATION;
    }
    if row.raw_counter_supply_status.is_some() {
        flags |= ROW_FLAG_COUNTER_SUPPLY_STATUS;
    }
    if row.raw_dig_in.is_some() {
        flags |= ROW_FLAG_DIG_IN;
    }
    if row.raw_dig_in_cap.is_some() {
        flags |= ROW_FLAG_DIG_IN_CAP;
    }
    if let Some(visible) = row.counter_training_visible {
        flags |= ROW_FLAG_TRAINING_OBSERVED;
        if visible {
            flags |= ROW_FLAG_TRAINING_VISIBLE;
        }
    }
    if let Some(frame) = row.counter_dig_in_frame {
        flags |= ROW_FLAG_DIG_IN_UI_OBSERVED;
        if frame != 0 {
            flags |= ROW_FLAG_DIG_IN_UI_VISIBLE;
        }
        if frame == 2 {
            flags |= ROW_FLAG_DIG_IN_UI_CAPPED;
        }
    }
    if let Some(visible) = row.counter_invalid_orders_visible {
        flags |= ROW_FLAG_INVALID_ORDERS_OBSERVED;
        if visible {
            flags |= ROW_FLAG_INVALID_ORDERS_VISIBLE;
        }
    }
    match row.order_link {
        DivisionOrderLink::None => {}
        DivisionOrderLink::ArmyOrder(_) => flags |= ROW_FLAG_ORDER_ARMY,
        DivisionOrderLink::ArmyGroup => flags |= ROW_FLAG_ORDER_ARMY_GROUP,
        DivisionOrderLink::Unresolved => flags |= ROW_FLAG_ORDER_UNRESOLVED,
    }
    flags
}

fn zero_filled_deleted_row(
    values: u32,
    a: u32,
    b: u32,
    c: u32,
    d: u32,
    e: u32,
    f: u32,
    g: u32,
    h: u32,
    i: u32,
    j: u32,
    k: u32,
    l: u32,
    m: u32,
    n: u32,
    o: u32,
    p: u32,
    q: u32,
    r: u32,
    s: u32,
    t: u32,
    u: u32,
    v: u32,
    w: u32,
    x: u32,
    y: u32,
    z: u32,
) -> bool {
    [
        values, a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p, q, r, s, t, u, v, w, x, y, z,
    ]
    .into_iter()
    .all(|value| value == 0)
}

fn validate_required_u32(
    summary: &mut Domain20GateSummary,
    offset: usize,
    row_index: usize,
    value: u32,
    min_value: u32,
    max_value: u32,
    label: &str,
) {
    if value < min_value || value > max_value {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions row {row_index} {label} {value} outside {min_value}..{max_value}"
        ));
    }
}

fn validate_optional_u32(
    summary: &mut Domain20GateSummary,
    offset: usize,
    row_index: usize,
    has_value: bool,
    value: u32,
    min_value: u32,
    max_value: u32,
    label: &str,
) {
    if has_value {
        validate_required_u32(
            summary, offset, row_index, value, min_value, max_value, label,
        );
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions row {row_index} has {label} without flag"
        ));
    }
}

fn validate_optional_province(
    summary: &mut Domain20GateSummary,
    offset: usize,
    row_index: usize,
    has_value: bool,
    value: u32,
    label: &str,
) {
    if has_value {
        if !valid_province_id(value) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has invalid {label} {value}"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions row {row_index} has {label} without flag"
        ));
    }
}

fn valid_province_id(value: u32) -> bool {
    value > 0 && value <= MAX_PROVINCE_ID
}

fn has_flag(flags: u32, flag: u32) -> bool {
    (flags & flag) != 0
}

fn add_unit(summary: &mut Domain20GateSummary, key: &'static str, value: usize) {
    let entry = summary.data_units_total.entry(key).or_default();
    *entry = entry.saturating_add(value as u64);
}

fn read_u32_le(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(payload[offset..offset + 4].try_into().expect("u32 range"))
}

fn read_u64_le(payload: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(payload[offset..offset + 8].try_into().expect("u64 range"))
}

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn option_i64_json(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn option_u64_json(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

#[cfg(not(test))]
fn main() {
    let args = env::args().collect::<Vec<_>>();
    if !(args.len() == 4 || args.len() == 5)
        || args.get(2).map(String::as_str) != Some("--min-game-hours")
    {
        eprintln!(
            "usage: domain20_compact <collector_stream.bin> --min-game-hours <positive advancements> [proof_summary.json]"
        );
        std::process::exit(2);
    }
    let minimum_game_hour_advancements = args[3].parse::<i64>().unwrap_or_else(|err| {
        eprintln!("invalid --min-game-hours value {}: {err}", args[3]);
        std::process::exit(2);
    });
    let raw = fs::read(&args[1]).unwrap_or_else(|err| {
        eprintln!("failed to read {}: {err}", args[1]);
        std::process::exit(2);
    });
    let summary = gate_production_stream(&raw, minimum_game_hour_advancements);
    let json = summary.to_json();
    if let Some(out_path) = args.get(4) {
        fs::write(out_path, json.as_bytes()).unwrap_or_else(|err| {
            eprintln!("failed to write {out_path}: {err}");
            std::process::exit(2);
        });
    } else {
        println!("{json}");
    }
    if !summary.production_certified {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_len_literal_is_stable() {
        // Guard against stale-constant drift (a prior review found a sibling
        // domain-42 test asserting a stale 68): pin the LITERAL, not a derived
        // value, so a wrong ROW_LEN fails here.
        assert_eq!(ROW_LEN, 120);
    }

    #[test]
    fn retained_104_byte_rows_decode_with_a15_fields_unavailable() {
        let rows = vec![DivisionStateRow {
            div_id: 7,
            location: 12,
            raw_organisation: 5_000_000,
            raw_strength: 100_000_000,
            ..DivisionStateRow::default()
        }];
        let header = DivisionStateHeader {
            elapsed_wall_ms: 0,
            status_flags: 0,
            changed_rows: 1,
            root_rows: 1,
        };
        let mut legacy = encode_payload(&header, &rows);
        legacy.truncate(PAYLOAD_HEADER_LEN + LEGACY_ROW_LEN);
        let summary = decode_payload(0, &legacy);
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
    }

    #[test]
    fn retained_104_byte_rows_reject_extended_a15_presence_bits() {
        let rows = vec![DivisionStateRow {
            div_id: 7,
            location: 12,
            raw_organisation: 5_000_000,
            raw_strength: 100_000_000,
            raw_max_organisation: Some(6_000_000),
            ..DivisionStateRow::default()
        }];
        let header = DivisionStateHeader {
            elapsed_wall_ms: 0,
            status_flags: 0,
            changed_rows: 1,
            root_rows: 1,
        };
        let mut forged_legacy = encode_payload(&header, &rows);
        forged_legacy.truncate(PAYLOAD_HEADER_LEN + LEGACY_ROW_LEN);
        let summary = decode_payload(0, &forged_legacy);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("legacy divisions row 0 carries extended A15 flags")));
    }

    #[test]
    fn encodes_and_decodes_full_state_rows() {
        let rows = vec![
            DivisionStateRow {
                div_id: 101,
                moving: true,
                location: 12_345,
                move_target_province: Some(12_345),
                movement_source_province: Some(12_300),
                raw_movement_progress: Some(42_000),
                progress_bucket: Some(42),
                raw_organisation: 8_500_000,
                raw_max_organisation: Some(10_000_000),
                organisation_bucket: 8_500,
                raw_strength: 96_250_000,
                strength_bucket: 96_250,
                combat_ref_count: 2,
                template_id: Some(3_101),
                owner_tag_db_id: Some(1),
                army_id: Some(77),
                raw_experience: Some(1_234_000),
                raw_max_supply: Some(2_000_000),
                raw_counter_supply_status: Some(34_999),
                raw_current_supply: Some(1_750_000),
                raw_supply_gain: Some(10_000),
                raw_fuel: Some(88_000),
                raw_disrupted_supply: Some(39_248),
                raw_dig_in: Some(1_000_000),
                raw_dig_in_cap: Some(2_000_000),
                counter_training_visible: Some(true),
                counter_dig_in_frame: Some(0),
                strategic_redeployment: Some(true),
                current_manpower: Some(8_777),
                cumulative_losses: Some(1_234),
                ..DivisionStateRow::default()
            },
            DivisionStateRow {
                div_id: 102,
                location: 99,
                raw_organisation: 10_000_000,
                organisation_bucket: 10_000,
                raw_strength: 100_000_000,
                strength_bucket: 100_000,
                owner_tag_db_id: Some(2),
                template_id: Some(4_001),
                raw_experience: Some(0),
                raw_current_supply: Some(2_000_000),
                raw_disrupted_supply: Some(0),
                strategic_redeployment: Some(false),
                current_manpower: Some(0),
                cumulative_losses: Some(0),
                ..DivisionStateRow::default()
            },
        ];
        let header = DivisionStateHeader {
            elapsed_wall_ms: 250,
            status_flags: 0,
            changed_rows: rows.len() as u32,
            root_rows: rows.len() as u32,
        };
        let payload = encode_payload(&header, &rows);
        let row0 = PAYLOAD_HEADER_LEN;
        assert_eq!(read_u32_le(&payload, row0 + 88), 39_248);
        assert_eq!(read_u32_le(&payload, row0 + 92), 1);
        assert_eq!(read_u32_le(&payload, row0 + 96), 8_777);
        assert_eq!(read_u32_le(&payload, row0 + 100), 1_234);
        assert_eq!(read_u32_le(&payload, row0 + 104), 10_000_000);
        assert_eq!(read_u32_le(&payload, row0 + 108), 34_999);
        assert_eq!(read_u32_le(&payload, row0 + 112), 1_000_000);
        assert_eq!(read_u32_le(&payload, row0 + 116), 2_000_000);
        let summary = decode_payload(32, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.data_rows_total, 2);
        assert_eq!(
            summary
                .data_units_total
                .get("division_moving_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_in_combat_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_current_supply_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            summary.data_units_total.get("division_fuel_rows").copied(),
            Some(1)
        );
        for key in [
            "division_max_organisation_rows",
            "division_counter_supply_status_rows",
            "division_dig_in_rows",
            "division_dig_in_cap_rows",
        ] {
            assert_eq!(summary.data_units_total.get(key).copied(), Some(1), "{key}");
        }
        for key in [
            "division_disrupted_supply_rows",
            "division_strategic_redeployment_rows",
            "division_current_manpower_rows",
            "division_cumulative_losses_rows",
        ] {
            assert_eq!(summary.data_units_total.get(key).copied(), Some(2), "{key}");
        }
    }

    #[test]
    fn rejects_corrupt_production_tail_values() {
        let row = DivisionStateRow {
            div_id: 103,
            location: 99,
            raw_organisation: 1,
            raw_strength: 1,
            raw_disrupted_supply: Some(0),
            strategic_redeployment: Some(false),
            current_manpower: Some(0),
            cumulative_losses: Some(0),
            ..DivisionStateRow::default()
        };
        let header = DivisionStateHeader {
            elapsed_wall_ms: 1,
            status_flags: 0,
            changed_rows: 1,
            root_rows: 1,
        };
        for (tail_offset, corrupt, label) in [
            (
                88usize,
                MAX_DISRUPTED_SUPPLY_RAW + 1,
                "disrupted_supply_raw",
            ),
            (92, 2, "strategic_redeployment_raw"),
            (96, MAX_CURRENT_MANPOWER + 1, "current_manpower"),
            (100, MAX_CUMULATIVE_LOSSES + 1, "cumulative_losses"),
        ] {
            let mut payload = encode_payload(&header, std::slice::from_ref(&row));
            let start = PAYLOAD_HEADER_LEN + tail_offset;
            payload[start..start + 4].copy_from_slice(&corrupt.to_le_bytes());
            let summary = decode_payload(0, &payload);
            assert!(
                summary
                    .parse_errors
                    .iter()
                    .any(|error| error.contains(label)),
                "tail +{tail_offset} corruption was not rejected: {:?}",
                summary.parse_errors
            );
        }
    }

    #[test]
    fn rejects_nonzero_production_tail_without_presence_flags() {
        let row = DivisionStateRow {
            div_id: 104,
            location: 99,
            raw_organisation: 1,
            raw_strength: 1,
            ..DivisionStateRow::default()
        };
        let header = DivisionStateHeader {
            elapsed_wall_ms: 1,
            status_flags: 0,
            changed_rows: 1,
            root_rows: 1,
        };
        for (tail_offset, label) in [
            (88usize, "disrupted_supply_raw"),
            (92, "strategic_redeployment_raw"),
            (96, "current_manpower"),
            (100, "cumulative_losses"),
        ] {
            let mut payload = encode_payload(&header, std::slice::from_ref(&row));
            let start = PAYLOAD_HEADER_LEN + tail_offset;
            payload[start..start + 4].copy_from_slice(&1u32.to_le_bytes());
            let summary = decode_payload(0, &payload);
            assert!(
                summary
                    .parse_errors
                    .iter()
                    .any(|error| error.contains(&format!("has {label} without flag"))),
                "tail +{tail_offset} missing-presence corruption was not rejected: {:?}",
                summary.parse_errors
            );
        }
    }

    #[test]
    fn encodes_and_decodes_order_link_rows() {
        let rows = vec![
            DivisionStateRow {
                div_id: 201,
                location: 10,
                raw_organisation: 1,
                raw_strength: 1,
                order_link: DivisionOrderLink::ArmyOrder(317),
                ..DivisionStateRow::default()
            },
            DivisionStateRow {
                div_id: 202,
                location: 11,
                raw_organisation: 1,
                raw_strength: 1,
                order_link: DivisionOrderLink::ArmyGroup,
                ..DivisionStateRow::default()
            },
            DivisionStateRow {
                div_id: 203,
                location: 12,
                raw_organisation: 1,
                raw_strength: 1,
                order_link: DivisionOrderLink::Unresolved,
                ..DivisionStateRow::default()
            },
            DivisionStateRow {
                div_id: 204,
                location: 13,
                raw_organisation: 1,
                raw_strength: 1,
                ..DivisionStateRow::default()
            },
        ];
        let header = DivisionStateHeader {
            elapsed_wall_ms: 1,
            status_flags: 0,
            changed_rows: rows.len() as u32,
            root_rows: rows.len() as u32,
        };
        let summary = decode_payload(0, &encode_payload(&header, &rows));
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_order_army_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_order_army_group_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_order_unresolved_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn rejects_out_of_range_order_instance_id() {
        let rows = vec![DivisionStateRow {
            div_id: 210,
            location: 10,
            raw_organisation: 1,
            raw_strength: 1,
            order_link: DivisionOrderLink::ArmyOrder(MAX_ORDER_INSTANCE_ID + 1),
            ..DivisionStateRow::default()
        }];
        let header = DivisionStateHeader {
            elapsed_wall_ms: 1,
            status_flags: 0,
            changed_rows: 1,
            root_rows: 1,
        };
        let summary = decode_payload(0, &encode_payload(&header, &rows));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("invalid order instance id")));
    }

    #[test]
    fn rejects_order_instance_id_without_army_flag() {
        let rows = vec![DivisionStateRow {
            div_id: 211,
            location: 10,
            raw_organisation: 1,
            raw_strength: 1,
            ..DivisionStateRow::default()
        }];
        let header = DivisionStateHeader {
            elapsed_wall_ms: 1,
            status_flags: 0,
            changed_rows: 1,
            root_rows: 1,
        };
        let mut payload = encode_payload(&header, &rows);
        let order_instance_id = PAYLOAD_HEADER_LEN + 84;
        payload[order_instance_id..order_instance_id + 4].copy_from_slice(&99u32.to_le_bytes());
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("order instance id without army-order flag")));
    }

    #[test]
    fn accepts_zero_filled_deleted_rows() {
        let rows = vec![DivisionStateRow {
            div_id: 404,
            deleted: true,
            ..DivisionStateRow::default()
        }];
        let header = DivisionStateHeader {
            elapsed_wall_ms: 250,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: 1,
            root_rows: 10,
        };
        let summary = decode_payload(0, &encode_payload(&header, &rows));
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_deleted_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn rejects_movement_fields_when_not_moving() {
        let rows = vec![DivisionStateRow {
            div_id: 5,
            location: 55,
            move_target_province: Some(56),
            raw_organisation: 1,
            raw_strength: 1,
            ..DivisionStateRow::default()
        }];
        let header = DivisionStateHeader {
            elapsed_wall_ms: 0,
            status_flags: 0,
            changed_rows: 1,
            root_rows: 1,
        };
        let summary = decode_payload(0, &encode_payload(&header, &rows));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("movement fields while not moving")));
    }

    #[test]
    fn decodes_stream_with_domain20_data_record() {
        let rows = vec![DivisionStateRow {
            div_id: 7,
            moving: true,
            location: 70,
            move_target_province: Some(70),
            raw_movement_progress: Some(1_000),
            progress_bucket: Some(1),
            raw_organisation: 5_000_000,
            organisation_bucket: 5_000,
            raw_strength: 75_000_000,
            strength_bucket: 75_000,
            owner_tag_db_id: Some(1),
            template_id: Some(9),
            raw_current_supply: Some(100_000),
            ..DivisionStateRow::default()
        }];
        let payload = encode_payload(
            &DivisionStateHeader {
                elapsed_wall_ms: 0,
                status_flags: 0,
                changed_rows: 1,
                root_rows: 1,
            },
            &rows,
        );
        let mut stream = Vec::new();
        append_record(
            &mut stream,
            1,
            123,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload,
        );
        append_record(&mut stream, 2, 123, 1000, RECORD_TYPE_FRAME_END, &[0; 32]);
        let summary = decode_stream(&stream);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.record_count, 2);
        assert_eq!(summary.domain_data_record_count, 1);
        assert_eq!(summary.frame_end_record_count, 1);
        assert_eq!(summary.unique_hours.len(), 1);
    }

    #[test]
    fn stream_accepts_unowned_records_and_perf_before_data() {
        let payload = encode_payload(
            &DivisionStateHeader {
                elapsed_wall_ms: 0,
                status_flags: 0,
                changed_rows: 0,
                root_rows: 0,
            },
            &[],
        );
        let mut stream = Vec::new();
        append_record(&mut stream, 1, 100, 51, RECORD_TYPE_DOMAIN_PERF, &[0; 40]);
        append_record(&mut stream, 2, 100, 51, RECORD_TYPE_DOMAIN_DATA, &[0xA5; 7]);
        append_record(
            &mut stream,
            3,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &[0; 40],
        );
        append_record(
            &mut stream,
            4,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload,
        );
        append_record(
            &mut stream,
            5,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; FRAME_END_PAYLOAD_LEN],
        );

        let summary = decode_stream(&stream);
        assert!(!summary.has_failures(), "{:?}", summary.parse_errors);
        assert_eq!(summary.domain_data_record_count, 1);
        assert_eq!(summary.domain_perf_record_count, 1);
    }

    #[test]
    fn stream_rejects_nonconsecutive_physical_closures() {
        let payload = encode_payload(
            &DivisionStateHeader {
                elapsed_wall_ms: 0,
                status_flags: 0,
                changed_rows: 0,
                root_rows: 0,
            },
            &[],
        );
        let mut stream = Vec::new();
        let mut sequence = 1u64;
        for hour in [100i32, 99, 101] {
            append_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_DATA,
                &payload,
            );
            sequence += 1;
            append_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &[0; FRAME_END_PAYLOAD_LEN],
            );
            sequence += 1;
        }

        let summary = decode_stream(&stream);
        assert!(summary
            .ordering_errors
            .iter()
            .any(|error| error.contains("closed hour discontinuity: expected 101, got 99")));
    }

    #[test]
    fn stream_uses_full_canonical_failure_mask_after_normalization() {
        let fatal = (1 << 0)
            | (1 << 1)
            | (1 << 2)
            | (1 << 3)
            | (1 << 6)
            | (1 << 7)
            | (1 << 8)
            | (1 << 9)
            | (1 << 12);
        assert_eq!(fatal, FAILURE_STATUS_MASK);
        assert_eq!(status_for_global_interpretation(51, 1 << 12), 0);
        assert_eq!(
            status_for_global_interpretation(DOMAIN_ID, 1 << 12),
            1 << 12
        );
        let payload = encode_payload(
            &DivisionStateHeader {
                elapsed_wall_ms: 0,
                status_flags: fatal,
                changed_rows: 0,
                root_rows: 0,
            },
            &[],
        );
        let mut stream = Vec::new();
        append_record(
            &mut stream,
            1,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload,
        );
        append_record(
            &mut stream,
            2,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; FRAME_END_PAYLOAD_LEN],
        );
        let summary = decode_stream(&stream);
        assert_eq!(summary.status_flags_or & FAILURE_STATUS_MASK, fatal);
        assert!(summary.has_failures());
    }

    #[test]
    fn derives_lifecycle_from_ordered_domain20_rows() {
        let baseline = vec![
            DivisionStateRow {
                div_id: 7,
                location: 70,
                raw_organisation: 5_000_000,
                organisation_bucket: 5_000,
                raw_strength: 75_000_000,
                strength_bucket: 75_000,
                owner_tag_db_id: Some(1),
                template_id: Some(9),
                ..DivisionStateRow::default()
            },
            DivisionStateRow {
                div_id: 8,
                location: 80,
                raw_organisation: 5_000_000,
                organisation_bucket: 5_000,
                raw_strength: 75_000_000,
                strength_bucket: 75_000,
                owner_tag_db_id: Some(1),
                template_id: Some(9),
                ..DivisionStateRow::default()
            },
        ];
        let delta = vec![
            DivisionStateRow {
                div_id: 7,
                moving: true,
                location: 71,
                move_target_province: Some(71),
                raw_movement_progress: Some(1_000),
                progress_bucket: Some(1),
                raw_organisation: 5_000_000,
                organisation_bucket: 5_000,
                raw_strength: 75_000_000,
                strength_bucket: 75_000,
                owner_tag_db_id: Some(1),
                template_id: Some(9),
                ..DivisionStateRow::default()
            },
            DivisionStateRow {
                div_id: 9,
                location: 90,
                raw_organisation: 5_000_000,
                organisation_bucket: 5_000,
                raw_strength: 75_000_000,
                strength_bucket: 75_000,
                owner_tag_db_id: Some(1),
                template_id: Some(10),
                ..DivisionStateRow::default()
            },
            DivisionStateRow {
                div_id: 8,
                deleted: true,
                ..DivisionStateRow::default()
            },
        ];
        let mut stream = Vec::new();
        append_record(
            &mut stream,
            1,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_payload(
                &DivisionStateHeader {
                    elapsed_wall_ms: 0,
                    status_flags: 0,
                    changed_rows: 2,
                    root_rows: 2,
                },
                &baseline,
            ),
        );
        append_record(
            &mut stream,
            2,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; FRAME_END_PAYLOAD_LEN],
        );
        append_record(
            &mut stream,
            3,
            101,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_payload(
                &DivisionStateHeader {
                    elapsed_wall_ms: 100,
                    status_flags: STATUS_DELTA_DATA,
                    changed_rows: 3,
                    root_rows: 3,
                },
                &delta,
            ),
        );
        append_record(
            &mut stream,
            4,
            101,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; FRAME_END_PAYLOAD_LEN],
        );
        let summary = decode_stream(&stream);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_lifecycle_baseline_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_lifecycle_updated_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_lifecycle_created_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_lifecycle_deleted_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn stream_rejects_sequence_flags_lengths_and_unclean_tail() {
        let payload = encode_payload(
            &DivisionStateHeader {
                elapsed_wall_ms: 0,
                status_flags: 0,
                changed_rows: 0,
                root_rows: 0,
            },
            &[],
        );
        let mut stream = Vec::new();
        append_record_with_flags(
            &mut stream,
            2,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            1 << 31,
            &payload,
        );
        append_record(
            &mut stream,
            3,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &[0; 39],
        );
        append_record(
            &mut stream,
            4,
            101,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; 31],
        );
        append_record(
            &mut stream,
            5,
            101,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &[0; 40],
        );

        let summary = decode_stream(&stream);
        assert!(summary
            .ordering_errors
            .iter()
            .any(|error| error.contains("expected 1")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("record flags")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("shorter than 40")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("is not 32")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("clean frame-end boundary")));
    }

    #[test]
    fn stream_validates_announced_perf_trailer_envelopes() {
        fn stream_with_perf(flags: u32, perf: &[u8]) -> Domain20GateSummary {
            let payload = encode_payload(
                &DivisionStateHeader {
                    elapsed_wall_ms: 0,
                    status_flags: 0,
                    changed_rows: 0,
                    root_rows: 0,
                },
                &[],
            );
            let mut stream = Vec::new();
            append_record(
                &mut stream,
                1,
                100,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_DATA,
                &payload,
            );
            append_record_with_flags(
                &mut stream,
                2,
                100,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_PERF,
                flags,
                perf,
            );
            append_record(
                &mut stream,
                3,
                100,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &[0; FRAME_END_PAYLOAD_LEN],
            );
            decode_stream(&stream)
        }

        let base = vec![0; 40];
        let trailer = empty_pdu1_perf_payload();
        assert!(
            stream_with_perf(RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &trailer)
                .parse_errors
                .is_empty()
        );
        assert!(stream_with_perf(RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &base)
            .parse_errors
            .iter()
            .any(|error| error.contains("trailer header is truncated")));
        assert!(stream_with_perf(0, &trailer)
            .parse_errors
            .iter()
            .any(|error| error.contains("unannounced trailer")));
    }

    #[test]
    fn production_gate_duration_is_advancements_plus_one_closed_frames() {
        let exact =
            gate_production_stream(&valid_production_stream(&[1_000, 1_001, 1_002, 1_003]), 3);
        assert!(exact.production_certified, "{:?}", exact.parse_errors);
        assert_eq!(exact.observed_game_hour_advancements, Some(3));
        assert_eq!(exact.required_closed_frame_count, Some(4));

        let short = gate_production_stream(&valid_production_stream(&[1_000, 1_001, 1_002]), 3);
        assert!(!short.production_certified);
        assert!(short.parse_errors.iter().any(|error| error.contains(
            "2 advancement(s) across 3 closed frame(s), below required 3 advancement(s) / 4 closed frame(s)"
        )));
    }

    #[test]
    fn production_gate_requires_an_explicit_positive_duration() {
        let gate = gate_production_stream(&valid_production_stream(&[100, 101]), 0);
        assert!(!gate.production_certified);
        assert!(gate
            .parse_errors
            .iter()
            .any(|error| error.contains("must be explicitly positive")));
    }

    #[test]
    fn production_gate_requires_exactly_one_owned_data_and_perf_per_hour() {
        let mut duplicate_data = Vec::new();
        let mut sequence = 1;
        append_production_hour(&mut duplicate_data, &mut sequence, 100, 2, 1, 0, 0, 1, 1, 0);
        append_production_hour(&mut duplicate_data, &mut sequence, 101, 1, 1, 0, 0, 1, 1, 0);
        let gate = gate_production_stream(&duplicate_data, 1);
        assert!(!gate.production_certified);
        assert!(
            gate.parse_errors
                .iter()
                .any(|error| error
                    .contains("has 2 divisions DOMAIN_DATA records, expected exactly 1"))
        );

        let mut missing_perf = Vec::new();
        let mut sequence = 1;
        append_production_hour(&mut missing_perf, &mut sequence, 100, 1, 0, 0, 0, 1, 1, 0);
        append_production_hour(&mut missing_perf, &mut sequence, 101, 1, 1, 0, 0, 1, 1, 0);
        let gate = gate_production_stream(&missing_perf, 1);
        assert!(!gate.production_certified);
        assert!(
            gate.parse_errors
                .iter()
                .any(|error| error
                    .contains("has 0 divisions DOMAIN_PERF records, expected exactly 1"))
        );
    }

    #[test]
    fn production_gate_rejects_status_disagreement_and_noncanonical_status_bits() {
        let mut disagreement = Vec::new();
        let mut sequence = 1;
        append_production_hour(
            &mut disagreement,
            &mut sequence,
            100,
            1,
            1,
            0,
            STATUS_DELTA_DATA,
            1,
            1,
            STATUS_DELTA_DATA,
        );
        append_production_hour(&mut disagreement, &mut sequence, 101, 1, 1, 0, 0, 1, 1, 0);
        let gate = gate_production_stream(&disagreement, 1);
        assert!(!gate.production_certified);
        assert!(gate
            .parse_errors
            .iter()
            .any(|error| error.contains("DATA status Some(0) does not match PERF status")));

        let mut unknown = valid_production_stream(&[100, 101]);
        // First DATA payload status word.
        unknown[STREAM_HEADER_LEN + 16..STREAM_HEADER_LEN + 20]
            .copy_from_slice(&(1u32 << 31).to_le_bytes());
        let gate = gate_production_stream(&unknown, 1);
        assert!(!gate.production_certified);
        assert!(gate
            .parse_errors
            .iter()
            .any(|error| error.contains("DATA status flags") && error.contains("unknown bits")));
    }

    #[test]
    fn production_gate_rejects_false_frame_end_authority() {
        let base = valid_production_stream(&[100, 101]);
        let final_payload = base.len() - FRAME_END_PAYLOAD_LEN;

        let mut mismatched_counts = base.clone();
        mismatched_counts[final_payload + 16..final_payload + 20]
            .copy_from_slice(&2u32.to_le_bytes());
        let gate = gate_production_stream(&mismatched_counts, 1);
        assert!(!gate.production_certified);
        assert!(gate
            .parse_errors
            .iter()
            .any(|error| error.contains("does not match scheduled domain count")));

        let mut false_equal_counts = base.clone();
        false_equal_counts[final_payload + 16..final_payload + 24]
            .copy_from_slice(&[2, 0, 0, 0, 2, 0, 0, 0]);
        let gate = gate_production_stream(&false_equal_counts, 1);
        assert!(!gate.production_certified);
        assert!(gate
            .parse_errors
            .iter()
            .any(|error| error.contains("does not match 1 DATA/1 PERF domain(s)")));

        let mut reserved = base.clone();
        reserved[final_payload + 28..final_payload + 32].copy_from_slice(&7u32.to_le_bytes());
        let gate = gate_production_stream(&reserved, 1);
        assert!(!gate.production_certified);
        assert!(gate
            .parse_errors
            .iter()
            .any(|error| error.contains("reserved word is nonzero")));

        let mut omitted_owned_status = base.clone();
        omitted_owned_status[final_payload + 24..final_payload + 28]
            .copy_from_slice(&0u32.to_le_bytes());
        let gate = gate_production_stream(&omitted_owned_status, 1);
        assert!(!gate.production_certified);
        assert!(gate
            .parse_errors
            .iter()
            .any(|error| error.contains("does not equal closed-frame PERF status aggregate")));

        let mut fatal_status = base.clone();
        fatal_status[final_payload + 24..final_payload + 28]
            .copy_from_slice(&STATUS_SOURCE_UNAVAILABLE.to_le_bytes());
        let gate = gate_production_stream(&fatal_status, 1);
        assert!(!gate.production_certified);
        assert!(gate
            .parse_errors
            .iter()
            .any(|error| error.contains("FRAME_END status flags") && error.contains("fatal bits")));

        let mut unknown_status = base;
        unknown_status[final_payload + 24..final_payload + 28]
            .copy_from_slice(&(1u32 << 31).to_le_bytes());
        let gate = gate_production_stream(&unknown_status, 1);
        assert!(!gate.production_certified);
        assert!(gate.parse_errors.iter().any(
            |error| error.contains("FRAME_END status flags") && error.contains("unknown bits")
        ));
    }

    #[test]
    fn production_gate_rejects_duplicate_unowned_records_hidden_by_distinct_domain_counts() {
        let owned_payload = encode_payload(
            &DivisionStateHeader {
                elapsed_wall_ms: 0,
                status_flags: 0,
                changed_rows: 0,
                root_rows: 0,
            },
            &[],
        );
        let mut stream = Vec::new();
        let mut sequence = 1u64;
        for hour in [100, 101] {
            for (domain_id, record_type, payload) in [
                (DOMAIN_ID, RECORD_TYPE_DOMAIN_DATA, owned_payload.as_slice()),
                (DOMAIN_ID, RECORD_TYPE_DOMAIN_PERF, &[0; 40]),
                (77, RECORD_TYPE_DOMAIN_DATA, b"a"),
                (77, RECORD_TYPE_DOMAIN_DATA, b"b"),
                (77, RECORD_TYPE_DOMAIN_PERF, &[0; 40]),
                (77, RECORD_TYPE_DOMAIN_PERF, &[0; 40]),
            ] {
                append_record(&mut stream, sequence, hour, domain_id, record_type, payload);
                sequence += 1;
            }
            append_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &frame_end_payload(2, 2, 0),
            );
            sequence += 1;
        }

        let gate = gate_production_stream(&stream, 1);
        assert!(!gate.production_certified);
        assert!(gate.parse_errors.iter().any(|error| {
            error.contains("does not match 3 non-probe DATA/3 non-probe PERF record(s)")
        }));
    }

    #[test]
    fn production_gate_rejects_noncanonical_sequence_flags_and_trailing_records() {
        let mut bad_sequence = valid_production_stream(&[100, 101]);
        bad_sequence[8..16].copy_from_slice(&2u64.to_le_bytes());
        let gate = gate_production_stream(&bad_sequence, 1);
        assert!(!gate.production_certified);
        assert!(gate
            .ordering_errors
            .iter()
            .any(|error| error.contains("first sequence is 2, expected 1")));

        let mut bad_flags = valid_production_stream(&[100, 101]);
        bad_flags[28..32].copy_from_slice(&(1u32 << 31).to_le_bytes());
        let gate = gate_production_stream(&bad_flags, 1);
        assert!(!gate.production_certified);
        assert!(gate
            .parse_errors
            .iter()
            .any(|error| error.contains("record flags")));

        let mut trailing = valid_production_stream(&[100, 101]);
        append_record(
            &mut trailing,
            7,
            102,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &[0; 40],
        );
        let gate = gate_production_stream(&trailing, 1);
        assert!(!gate.production_certified);
        assert!(gate
            .parse_errors
            .iter()
            .any(|error| error.contains("does not end at a clean frame-end boundary")));
    }

    #[test]
    fn production_gate_excludes_reserved_probe_perf_from_frame_authority() {
        let payload = encode_payload(
            &DivisionStateHeader {
                elapsed_wall_ms: 0,
                status_flags: 0,
                changed_rows: 0,
                root_rows: 0,
            },
            &[],
        );
        let mut stream = Vec::new();
        let mut sequence = 1;
        for hour in [100, 101] {
            append_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_DATA,
                &payload,
            );
            sequence += 1;
            append_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_PERF,
                &[0; 40],
            );
            sequence += 1;
            append_record(
                &mut stream,
                sequence,
                hour,
                PROBE_DOMAIN_ID,
                RECORD_TYPE_DOMAIN_PERF,
                &[0; 40],
            );
            sequence += 1;
            append_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &frame_end_payload(1, 1, 0),
            );
            sequence += 1;
        }

        let gate = gate_production_stream(&stream, 1);
        assert!(gate.production_certified, "{:?}", gate.parse_errors);
    }

    #[test]
    fn production_gate_normalizes_lane_local_perf_status_before_frame_authority() {
        let payload = encode_payload(
            &DivisionStateHeader {
                elapsed_wall_ms: 0,
                status_flags: 0,
                changed_rows: 0,
                root_rows: 0,
            },
            &[],
        );
        let mut domain51_perf = vec![0; 40];
        domain51_perf[32..36].copy_from_slice(&(1u32 << 12).to_le_bytes());
        let mut stream = Vec::new();
        let mut sequence = 1;
        for hour in [100, 101] {
            append_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_DATA,
                &payload,
            );
            sequence += 1;
            append_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_PERF,
                &[0; 40],
            );
            sequence += 1;
            append_record(
                &mut stream,
                sequence,
                hour,
                51,
                RECORD_TYPE_DOMAIN_DATA,
                &[],
            );
            sequence += 1;
            append_record(
                &mut stream,
                sequence,
                hour,
                51,
                RECORD_TYPE_DOMAIN_PERF,
                &domain51_perf,
            );
            sequence += 1;
            append_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &frame_end_payload(2, 2, 0),
            );
            sequence += 1;
        }

        let gate = gate_production_stream(&stream, 1);
        assert!(gate.production_certified, "{:?}", gate.parse_errors);
    }

    fn valid_production_stream(hours: &[i32]) -> Vec<u8> {
        let mut stream = Vec::new();
        let mut sequence = 1;
        for (index, hour) in hours.iter().copied().enumerate() {
            let status = if index == 0 { 0 } else { STATUS_DELTA_DATA };
            append_production_hour(
                &mut stream,
                &mut sequence,
                hour,
                1,
                1,
                status,
                status,
                1,
                1,
                status,
            );
        }
        stream
    }

    #[allow(clippy::too_many_arguments)]
    fn append_production_hour(
        stream: &mut Vec<u8>,
        sequence: &mut u64,
        hour: i32,
        data_count: usize,
        perf_count: usize,
        data_status: u32,
        perf_status: u32,
        emitted_domains: u32,
        scheduled_domains: u32,
        frame_status: u32,
    ) {
        let payload = encode_payload(
            &DivisionStateHeader {
                elapsed_wall_ms: 0,
                status_flags: data_status,
                changed_rows: 0,
                root_rows: 0,
            },
            &[],
        );
        for _ in 0..data_count {
            append_record(
                stream,
                *sequence,
                hour,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_DATA,
                &payload,
            );
            *sequence += 1;
        }
        let mut perf = vec![0; 40];
        perf[32..36].copy_from_slice(&perf_status.to_le_bytes());
        for _ in 0..perf_count {
            append_record(
                stream,
                *sequence,
                hour,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_PERF,
                &perf,
            );
            *sequence += 1;
        }
        append_record(
            stream,
            *sequence,
            hour,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_end_payload(emitted_domains, scheduled_domains, frame_status),
        );
        *sequence += 1;
    }

    fn frame_end_payload(
        emitted_domains: u32,
        scheduled_domains: u32,
        status_flags: u32,
    ) -> Vec<u8> {
        let mut payload = vec![0; FRAME_END_PAYLOAD_LEN];
        payload[16..20].copy_from_slice(&emitted_domains.to_le_bytes());
        payload[20..24].copy_from_slice(&scheduled_domains.to_le_bytes());
        payload[24..28].copy_from_slice(&status_flags.to_le_bytes());
        payload
    }

    fn append_record(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        payload: &[u8],
    ) {
        append_record_with_flags(
            out,
            sequence,
            game_hour_raw,
            domain_id,
            record_type,
            0,
            payload,
        );
    }

    fn append_record_with_flags(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) {
        out.extend_from_slice(STREAM_MAGIC);
        out.extend_from_slice(&STREAM_VERSION.to_le_bytes());
        out.extend_from_slice(&(STREAM_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour_raw.to_le_bytes());
        out.extend_from_slice(&domain_id.to_le_bytes());
        out.extend_from_slice(&record_type.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&record_flags.to_le_bytes());
        out.extend_from_slice(payload);
    }

    fn empty_pdu1_perf_payload() -> Vec<u8> {
        let mut payload = vec![0; 40];
        payload.extend_from_slice(b"PDU1");
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload
    }
}
