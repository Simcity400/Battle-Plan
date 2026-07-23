use std::collections::{BTreeMap, BTreeSet};

pub mod deployment_queue;
pub use deployment_queue::{
    decode_deployment_queue_frame, decode_deployment_queue_payload,
    encode_deployment_queue_payload, DeploymentConveyorRow, DeploymentCountryRow,
    DeploymentEquipmentRow, DeploymentLineRow, DeploymentQueueFrame, DeploymentQueueSummary,
};

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const RECORD_TYPE_FRAME_END: u16 = 2;
pub const RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const DOMAIN_FUEL: u16 = 10;
pub const DOMAIN_PRODUCTION: u16 = 11;
pub const DOMAIN_STOCKPILE_VARIANT_DEFS: u16 = 37;
pub const DOMAIN_STOCKPILE_VARIANT_UPGRADES: u16 = 40;
pub const DOMAIN_GAP_BADGES: u16 = 611;
pub const DOMAIN_FRAME: u16 = 1000;
const RC_PROBE_DOMAIN_ID: u16 = 250;

pub const PROPOSED_DOMAIN_PRODUCTION_LINE_DEMAND: u16 = 612;
pub const PROPOSED_DOMAIN_COUNTRY_RESOURCE_FLOWS: u16 = 613;
pub const PROPOSED_DOMAIN_DEPLOYMENT_QUEUES: u16 = 614;
pub const PROPOSED_DOMAIN_COUNTRY_ECONOMY_EFFECTS: u16 = 615;
pub const PROPOSED_DOMAIN_MIO_DESIGN_JOINS: u16 = 616;

pub const STATUS_WORKER_PANIC: u32 = compact_contract::COMPACT_STATUS_WORKER_PANIC;
pub const STATUS_BRIDGE_UNAVAILABLE: u32 = compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE;
pub const STATUS_PARTIAL_DATA: u32 = compact_contract::COMPACT_STATUS_PARTIAL_DATA;
pub const STATUS_DELTA_DATA: u32 = compact_contract::COMPACT_STATUS_DELTA_DATA;

pub const FUEL_SCALE_RAW: u32 = 32768;
pub const PRODUCTION_SCALE_RAW: u32 = 100000;
pub const FUEL_ROW_LEN: usize = 40;
pub const PRODUCTION_STOCKPILE_ROW_LEN: usize = 16;
pub const PRODUCTION_LINE_ROW_LEN: usize = 32;
pub const PRODUCTION_EFFICIENCY_ROW_LEN: usize = 36;
pub const PRODUCTION_MIO_JOIN_ROW_LEN: usize = 16;
/// Native production-efficiency helpers return fixed-point percentage values:
/// 100_000 raw units are one percentage point, so 10_000_000 is 100%.
pub const PRODUCTION_EFFICIENCY_RAW_PER_PERCENT: i64 = 100_000;
pub const PRODUCTION_EFFICIENCY_FULL_SCALE_RAW: i64 = 10_000_000;
pub const PRODUCTION_CONSTRUCTION_ROW_LEN: usize = 32;
pub const VARIANT_DEF_ROW_LEN: usize = 24;
pub const VARIANT_DEF_MODULE_ROW_LEN: usize = 8;
pub const VARIANT_UPGRADE_ROW_LEN: usize = 16;
pub const VARIANT_UPGRADE_ENTRY_ROW_LEN: usize = 8;
pub const GAP_BADGE_ROW_LEN: usize = 32;
pub const GAP_BADGE_SCHEMA_VERSION: u32 = 1;

pub const GAP_STATUS_SCHEMA_BLOCKED: u32 = 1;
pub const GAP_STATUS_SOURCE_BLOCKED: u32 = 2;
pub const GAP_STATUS_OWNED_ELSEWHERE: u32 = 3;
pub const GAP_STATUS_BADGED_FAIL_CLOSED: u32 = 4;

pub const GAP_EVIDENCE_SAVE_ORACLE: u32 = 1;
pub const GAP_EVIDENCE_CURRENT_COMPACT_SCHEMA: u32 = 1 << 1;
pub const GAP_EVIDENCE_CURRENT_COLLECTOR_ROOTS: u32 = 1 << 2;
pub const GAP_EVIDENCE_COVERAGE_AUDIT: u32 = 1 << 3;
pub const GAP_EVIDENCE_OWNER_BOUNDARY: u32 = 1 << 4;
pub const GAP_EVIDENCE_NO_HOT_LOOP_ROOT: u32 = 1 << 5;

pub const GAP_CURRENT_SUPPORT_NONE: u32 = 0;
pub const GAP_CURRENT_SUPPORT_PARTIAL: u32 = 1;
pub const GAP_CURRENT_SUPPORT_OWNED_ELSEWHERE: u32 = 2;

const MAX_TAG_DB_ID: u32 = compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
const MAX_VARIANT_ID: u32 = i32::MAX as u32;
const MAX_FACTORIES: i32 = 100_000;
const MAX_SLOT_HANDLE: u32 = 1_000_000;
const MAX_STRING_LEN: usize = 160;
const MAX_UPGRADE_INDEX: u32 = 256;
const MAX_UPGRADE_LEVEL: u32 = 255;

const VARIANT_DEF_FLAG_DISPLAY_NAME: u32 = 1;
const VARIANT_DEF_FLAG_VARIANT_NAME: u32 = 1 << 1;
const VARIANT_DEF_FLAG_MODULES: u32 = 1 << 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FuelRow {
    pub tag_db_id: u32,
    pub current_raw: i64,
    pub max_raw: i64,
    pub gain_raw: i64,
    pub cost_raw: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductionStockpileRow {
    pub tag_db_id: u32,
    pub variant_id: u32,
    pub amount_raw: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductionLineRow {
    pub tag_db_id: u32,
    pub kind: u32,
    pub line_id: i32,
    pub active_factories: i32,
    pub cost_raw: i32,
    pub speed_raw: i32,
    pub produced_raw: i32,
    pub amount: i32,
}

/// Exact X4-P4 outputs from the three a729 line producers. These are producer
/// results, not values reconstructed from line speed, factory counts, or
/// global defines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductionEfficiencyRow {
    pub tag_db_id: u32,
    pub kind: u32,
    pub line_id: i32,
    pub efficiency_raw: i64,
    pub efficiency_cap_raw: i64,
    pub efficiency_growth_raw: i64,
}

/// Exact X4-P5 line-owned industrial-organisation identity. `mio_id == 0`
/// represents the serializer-proven absent `industrial_manufacturer` field;
/// positive values represent `{ id, type = 79 }` after the live type gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductionMioJoinRow {
    pub tag_db_id: u32,
    pub kind: u32,
    pub line_id: i32,
    pub mio_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConstructionQueueRow {
    pub tag_db_id: u32,
    pub line_id: u32,
    pub template_id: Option<u32>,
    pub active_factories: i32,
    pub cost_raw: i32,
    pub produced_raw: i32,
    pub priority: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GapBadgeRow {
    pub gap_id: u32,
    pub status: u32,
    pub evidence_mask: u32,
    pub current_support: u32,
    pub current_domain_id: u32,
    pub proposed_domain_id: u32,
    pub current_row_len: u32,
    pub source_ref_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemainingGapBadge {
    pub gap_id: u32,
    pub name: &'static str,
    pub status: u32,
    pub status_label: &'static str,
    pub evidence_mask: u32,
    pub current_support: u32,
    pub current_compact_evidence: &'static str,
    pub source_evidence: &'static str,
    pub fail_closed_badge: &'static str,
    pub proposed_domain_id: u16,
    pub proposed_domain_name: &'static str,
    pub owner_boundary: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DomainSummary {
    pub domain_id: u16,
    pub name: &'static str,
    pub record_count: u64,
    pub perf_record_count: u64,
    pub data_record_count: u64,
    pub row_count_total: u64,
    pub changed_rows_total: u64,
    pub unit_count_total: u64,
    pub changed_unit_count_total: u64,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub status_flags_or: u32,
    pub data_units_total: BTreeMap<&'static str, u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompactStreamSummary {
    pub total_bytes: usize,
    pub total_payload_bytes: usize,
    pub max_payload_len: usize,
    pub record_count: u64,
    pub frame_end_record_count: u64,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
    pub unique_hours: Vec<i32>,
    pub skipped_hours: i64,
    pub status_flags_or: u32,
    pub failure_status_flags_or: u32,
    pub ordered: bool,
    pub parse_errors: Vec<String>,
    pub ordering_errors: Vec<String>,
    pub coverage_errors: Vec<String>,
    pub frame_hour_counts: BTreeMap<i32, u64>,
    pub per_hour_perf_domain_counts: BTreeMap<i32, BTreeMap<u16, u64>>,
    pub per_hour_data_domain_counts: BTreeMap<i32, BTreeMap<u16, u64>>,
    pub per_hour_perf_row_cardinality: BTreeMap<i32, BTreeMap<u16, (u64, u64)>>,
    pub per_hour_data_row_cardinality: BTreeMap<i32, BTreeMap<u16, (u64, u64)>>,
    pub per_domain: BTreeMap<u16, DomainSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneGate {
    pub ok: bool,
    pub expected_domains: Vec<u16>,
    pub summary: CompactStreamSummary,
}

pub fn domain_name(domain_id: u16) -> &'static str {
    match domain_id {
        DOMAIN_FUEL => "fuel_live",
        DOMAIN_PRODUCTION => "production_live",
        DOMAIN_STOCKPILE_VARIANT_DEFS => "stockpile_variant_defs",
        DOMAIN_STOCKPILE_VARIANT_UPGRADES => "stockpile_variant_upgrades",
        DOMAIN_GAP_BADGES => "economy_production_gap_badges",
        PROPOSED_DOMAIN_PRODUCTION_LINE_DEMAND => "proposed_production_line_demand",
        PROPOSED_DOMAIN_COUNTRY_RESOURCE_FLOWS => "proposed_country_resource_flows",
        PROPOSED_DOMAIN_DEPLOYMENT_QUEUES => "proposed_deployment_queues",
        PROPOSED_DOMAIN_COUNTRY_ECONOMY_EFFECTS => "proposed_country_economy_effects",
        PROPOSED_DOMAIN_MIO_DESIGN_JOINS => "proposed_mio_design_joins",
        DOMAIN_FRAME => "frame",
        _ => "unknown",
    }
}

pub fn domain_id(name: &str) -> Option<u16> {
    match name {
        "fuel_live" | "fuel" => Some(DOMAIN_FUEL),
        "production_live" | "production" => Some(DOMAIN_PRODUCTION),
        "stockpile_variant_defs" | "variant_defs" => Some(DOMAIN_STOCKPILE_VARIANT_DEFS),
        "stockpile_variant_upgrades" | "variant_upgrades" => {
            Some(DOMAIN_STOCKPILE_VARIANT_UPGRADES)
        }
        "economy_production_gap_badges" | "gap_badges" => Some(DOMAIN_GAP_BADGES),
        _ => None,
    }
}

pub fn default_expected_domains() -> Vec<u16> {
    vec![
        DOMAIN_FUEL,
        DOMAIN_PRODUCTION,
        DOMAIN_STOCKPILE_VARIANT_DEFS,
        DOMAIN_STOCKPILE_VARIANT_UPGRADES,
    ]
}

fn is_lane_owned_data_domain(domain_id: u16) -> bool {
    matches!(
        domain_id,
        DOMAIN_FUEL
            | DOMAIN_PRODUCTION
            | DOMAIN_STOCKPILE_VARIANT_DEFS
            | DOMAIN_STOCKPILE_VARIANT_UPGRADES
            | DOMAIN_GAP_BADGES
    )
}

fn is_owned_perf_authority_domain(domain_id: u16) -> bool {
    is_lane_owned_data_domain(domain_id)
        || compact_contract::assigned_descriptor_by_id(domain_id).is_some()
}

pub fn encode_fuel_payload(elapsed_wall_ms: u64, status_flags: u32, rows: &[FuelRow]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(24usize.saturating_add(rows.len() * FUEL_ROW_LEN));
    let row_count = rows.len().min(u32::MAX as usize) as u32;
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&FUEL_SCALE_RAW.to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&row.current_raw.to_le_bytes());
        payload.extend_from_slice(&row.max_raw.to_le_bytes());
        payload.extend_from_slice(&row.gain_raw.to_le_bytes());
        payload.extend_from_slice(&row.cost_raw.to_le_bytes());
    }
    payload
}

pub fn encode_production_payload(
    elapsed_wall_ms: u64,
    status_flags: u32,
    country_count: u32,
    stockpile_rows: &[ProductionStockpileRow],
    military_rows: &[ProductionLineRow],
    naval_rows: &[ProductionLineRow],
    construction_rows: &[ConstructionQueueRow],
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(
        40usize
            .saturating_add(stockpile_rows.len() * PRODUCTION_STOCKPILE_ROW_LEN)
            .saturating_add(
                military_rows
                    .len()
                    .saturating_add(naval_rows.len())
                    .saturating_mul(PRODUCTION_LINE_ROW_LEN),
            )
            .saturating_add(construction_rows.len() * PRODUCTION_CONSTRUCTION_ROW_LEN),
    );
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&country_count.to_le_bytes());
    payload.extend_from_slice(&country_count.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&PRODUCTION_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&(stockpile_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&(military_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&(naval_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload
        .extend_from_slice(&(construction_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    for row in stockpile_rows {
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.variant_id.to_le_bytes());
        payload.extend_from_slice(&row.amount_raw.to_le_bytes());
    }
    for row in military_rows.iter().chain(naval_rows.iter()) {
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.kind.to_le_bytes());
        payload.extend_from_slice(&row.line_id.to_le_bytes());
        payload.extend_from_slice(&row.active_factories.to_le_bytes());
        payload.extend_from_slice(&row.cost_raw.to_le_bytes());
        payload.extend_from_slice(&row.speed_raw.to_le_bytes());
        payload.extend_from_slice(&row.produced_raw.to_le_bytes());
        payload.extend_from_slice(&row.amount.to_le_bytes());
    }
    for row in construction_rows {
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.line_id.to_le_bytes());
        payload.extend_from_slice(&row.template_id.unwrap_or(u32::MAX).to_le_bytes());
        payload.extend_from_slice(&row.active_factories.to_le_bytes());
        payload.extend_from_slice(&row.cost_raw.to_le_bytes());
        payload.extend_from_slice(&row.produced_raw.to_le_bytes());
        payload.extend_from_slice(&row.priority.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    payload
}

/// Append the strict X4-P4 `PEF1` producer-output segment. The caller must
/// supply one exact helper-result row for every emitted base line; otherwise
/// the segment is rejected instead of publishing partial/default efficiency.
pub fn append_production_efficiency_trailer(
    payload: &mut Vec<u8>,
    military_rows: &[ProductionLineRow],
    naval_rows: &[ProductionLineRow],
    efficiency_rows: &[ProductionEfficiencyRow],
) -> Result<(), String> {
    let base_keys: BTreeSet<_> = military_rows
        .iter()
        .chain(naval_rows.iter())
        .map(|row| (row.tag_db_id, row.kind, row.line_id))
        .collect();
    if base_keys.len() != military_rows.len().saturating_add(naval_rows.len()) {
        return Err("PEF1 base production-line keys are not unique".to_string());
    }

    let mut efficiency_keys = BTreeSet::new();
    for row in efficiency_rows {
        if row.tag_db_id == 0 || (row.kind != 1 && row.kind != 2) {
            return Err("PEF1 row has invalid tag/kind".to_string());
        }
        if row.efficiency_raw < 0 || row.efficiency_cap_raw < 0 || row.efficiency_growth_raw < 0 {
            return Err("PEF1 row has a negative producer result".to_string());
        }
        if !efficiency_keys.insert((row.tag_db_id, row.kind, row.line_id)) {
            return Err("PEF1 producer keys are not unique".to_string());
        }
    }
    if efficiency_keys != base_keys {
        return Err(format!(
            "PEF1 line coverage is not bijective: producer_rows={} base_lines={}",
            efficiency_keys.len(),
            base_keys.len()
        ));
    }
    if efficiency_rows.is_empty() {
        return Ok(());
    }

    let row_count = u32::try_from(efficiency_rows.len())
        .map_err(|_| "PEF1 row count exceeds u32".to_string())?;
    let segment_len = 20u32
        .checked_add(
            row_count
                .checked_mul(PRODUCTION_EFFICIENCY_ROW_LEN as u32)
                .ok_or_else(|| "PEF1 segment length overflow".to_string())?,
        )
        .ok_or_else(|| "PEF1 segment length overflow".to_string())?;
    payload.extend_from_slice(b"PEF1");
    payload.extend_from_slice(&1u16.to_le_bytes());
    payload.extend_from_slice(&20u16.to_le_bytes());
    payload.extend_from_slice(&segment_len.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for row in efficiency_rows {
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.kind.to_le_bytes());
        payload.extend_from_slice(&row.line_id.to_le_bytes());
        payload.extend_from_slice(&row.efficiency_raw.to_le_bytes());
        payload.extend_from_slice(&row.efficiency_cap_raw.to_le_bytes());
        payload.extend_from_slice(&row.efficiency_growth_raw.to_le_bytes());
    }
    Ok(())
}

/// Append the strict X4-P5 `PMI1` line-owned MIO join. The segment is only
/// emitted when the caller supplies exactly one assigned-or-proven-null value
/// for every admitted base production line.
pub fn append_production_mio_join_trailer(
    payload: &mut Vec<u8>,
    military_rows: &[ProductionLineRow],
    naval_rows: &[ProductionLineRow],
    mio_rows: &[ProductionMioJoinRow],
) -> Result<(), String> {
    let base_keys: BTreeSet<_> = military_rows
        .iter()
        .chain(naval_rows.iter())
        .map(|row| (row.tag_db_id, row.kind, row.line_id))
        .collect();
    if base_keys.len() != military_rows.len().saturating_add(naval_rows.len()) {
        return Err("PMI1 base production-line keys are not unique".to_string());
    }

    let mut mio_keys = BTreeSet::new();
    for row in mio_rows {
        if row.tag_db_id == 0 || (row.kind != 1 && row.kind != 2) {
            return Err("PMI1 row has invalid tag/kind".to_string());
        }
        if !mio_keys.insert((row.tag_db_id, row.kind, row.line_id)) {
            return Err("PMI1 producer keys are not unique".to_string());
        }
    }
    if mio_keys != base_keys {
        return Err(format!(
            "PMI1 line coverage is not bijective: producer_rows={} base_lines={}",
            mio_keys.len(),
            base_keys.len()
        ));
    }
    if mio_rows.is_empty() {
        return Ok(());
    }

    let row_count =
        u32::try_from(mio_rows.len()).map_err(|_| "PMI1 row count exceeds u32".to_string())?;
    let segment_len = 20u32
        .checked_add(
            row_count
                .checked_mul(PRODUCTION_MIO_JOIN_ROW_LEN as u32)
                .ok_or_else(|| "PMI1 segment length overflow".to_string())?,
        )
        .ok_or_else(|| "PMI1 segment length overflow".to_string())?;
    payload.extend_from_slice(b"PMI1");
    payload.extend_from_slice(&1u16.to_le_bytes());
    payload.extend_from_slice(&20u16.to_le_bytes());
    payload.extend_from_slice(&segment_len.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for row in mio_rows {
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.kind.to_le_bytes());
        payload.extend_from_slice(&row.line_id.to_le_bytes());
        payload.extend_from_slice(&row.mio_id.to_le_bytes());
    }
    Ok(())
}

pub fn remaining_gap_badges() -> Vec<RemainingGapBadge> {
    vec![
        RemainingGapBadge {
            gap_id: 2,
            name: "consumer_goods",
            status: GAP_STATUS_SOURCE_BLOCKED,
            status_label: "source_blocked",
            evidence_mask: GAP_EVIDENCE_COVERAGE_AUDIT
                | GAP_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | GAP_EVIDENCE_NO_HOT_LOOP_ROOT,
            current_support: GAP_CURRENT_SUPPORT_NONE,
            current_compact_evidence: "accepted compact domains carry fuel, production lines/stockpiles/construction, and stockpile variant metadata; no consumer-goods country row exists",
            source_evidence: "coverage_audit recursive country paths expose scripted consumer-goods variables, but no validated numeric country consumer-goods root is present in the live compact path",
            fail_closed_badge: "consumer_goods_effective omitted/badged; do not derive from law tokens or scripted variables in hot-loop capture",
            proposed_domain_id: PROPOSED_DOMAIN_COUNTRY_ECONOMY_EFFECTS,
            proposed_domain_name: domain_name(PROPOSED_DOMAIN_COUNTRY_ECONOMY_EFFECTS),
            owner_boundary: "lane-owned only for numeric economy effect; character/politics owns law/idea token extraction",
        },
        RemainingGapBadge {
            gap_id: 3,
            name: "resource_flows_shortages",
            status: GAP_STATUS_SOURCE_BLOCKED,
            status_label: "source_blocked",
            evidence_mask: GAP_EVIDENCE_SAVE_ORACLE
                | GAP_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | GAP_EVIDENCE_OWNER_BOUNDARY
                | GAP_EVIDENCE_NO_HOT_LOOP_ROOT,
            current_support: GAP_CURRENT_SUPPORT_PARTIAL,
            current_compact_evidence: "map-detail domains cover per-state resource deposits elsewhere; accepted lane domains do not carry per-country produced/exported/to_export/shortage rows",
            source_evidence: "save_oracle/parse_economy.py reads countries.<TAG>.resources.{produced,exported,to_export} and derives available=produced-exported",
            fail_closed_badge: "country resource flow/shortage rows omitted until a CCountry resource-flow root is validated; do not duplicate state-deposit or market/trade ownership",
            proposed_domain_id: PROPOSED_DOMAIN_COUNTRY_RESOURCE_FLOWS,
            proposed_domain_name: domain_name(PROPOSED_DOMAIN_COUNTRY_RESOURCE_FLOWS),
            owner_boundary: "state deposits belong to map/detail construction-state lanes; market/trade contracts belong to market owner",
        },
        RemainingGapBadge {
            gap_id: 6,
            name: "economy_law_derived_state",
            status: GAP_STATUS_OWNED_ELSEWHERE,
            status_label: "owned_elsewhere_badged",
            evidence_mask: GAP_EVIDENCE_SAVE_ORACLE
                | GAP_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | GAP_EVIDENCE_OWNER_BOUNDARY,
            current_support: GAP_CURRENT_SUPPORT_OWNED_ELSEWHERE,
            current_compact_evidence: "this lane has no law-token compact domain; economy derivations are unsafe without validated numeric effect roots",
            source_evidence: "save_oracle/parse_economy.py classifies politics.ideas into laws; shared collector has politics law JSON support outside this lane",
            fail_closed_badge: "law tokens stay characters/politics-owned; this lane will only consume them as joins or emit validated numeric economy effects",
            proposed_domain_id: PROPOSED_DOMAIN_COUNTRY_ECONOMY_EFFECTS,
            proposed_domain_name: domain_name(PROPOSED_DOMAIN_COUNTRY_ECONOMY_EFFECTS),
            owner_boundary: "characters/politics owns law and idea identity; this lane owns numeric factory/resource effects only",
        },
        RemainingGapBadge {
            gap_id: 7,
            name: "trade_factory_impact",
            status: GAP_STATUS_OWNED_ELSEWHERE,
            status_label: "owned_elsewhere_badged",
            evidence_mask: GAP_EVIDENCE_SAVE_ORACLE
                | GAP_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | GAP_EVIDENCE_OWNER_BOUNDARY,
            current_support: GAP_CURRENT_SUPPORT_OWNED_ELSEWHERE,
            current_compact_evidence: "production/fuel/stockpile streams do not carry market contract delivery or factory_cic rows",
            source_evidence: "save_oracle/parse_market.py reads delivery_state.factory_cic_progress; market_trade_live lane owns market/trade contracts",
            fail_closed_badge: "trade-factory impact omitted from this lane except future aggregate join rows granted by market/trade owner",
            proposed_domain_id: PROPOSED_DOMAIN_COUNTRY_ECONOMY_EFFECTS,
            proposed_domain_name: domain_name(PROPOSED_DOMAIN_COUNTRY_ECONOMY_EFFECTS),
            owner_boundary: "market/trade owner owns contract-level trade-factory impact; this lane may later consume aggregate tag_db_id impacts only",
        },
    ]
}

pub fn gap_badge_rows(gaps: &[RemainingGapBadge]) -> Vec<GapBadgeRow> {
    gaps.iter()
        .map(|gap| GapBadgeRow {
            gap_id: gap.gap_id,
            status: gap.status,
            evidence_mask: gap.evidence_mask,
            current_support: gap.current_support,
            current_domain_id: 0,
            proposed_domain_id: gap.proposed_domain_id as u32,
            current_row_len: 0,
            source_ref_id: gap.gap_id,
        })
        .collect()
}

pub fn encode_gap_badges_payload(
    elapsed_wall_ms: u64,
    status_flags: u32,
    rows: &[GapBadgeRow],
) -> Vec<u8> {
    let source_blocked = rows
        .iter()
        .filter(|row| row.status == GAP_STATUS_SOURCE_BLOCKED)
        .count()
        .min(u32::MAX as usize) as u32;
    let schema_blocked = rows
        .iter()
        .filter(|row| row.status == GAP_STATUS_SCHEMA_BLOCKED)
        .count()
        .min(u32::MAX as usize) as u32;
    let owned_elsewhere = rows
        .iter()
        .filter(|row| row.status == GAP_STATUS_OWNED_ELSEWHERE)
        .count()
        .min(u32::MAX as usize) as u32;
    let mut payload = Vec::with_capacity(44usize.saturating_add(rows.len() * GAP_BADGE_ROW_LEN));
    let row_count = rows.len().min(u32::MAX as usize) as u32;
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&GAP_BADGE_SCHEMA_VERSION.to_le_bytes());
    payload.extend_from_slice(&source_blocked.to_le_bytes());
    payload.extend_from_slice(&schema_blocked.to_le_bytes());
    payload.extend_from_slice(&owned_elsewhere.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.gap_id.to_le_bytes());
        payload.extend_from_slice(&row.status.to_le_bytes());
        payload.extend_from_slice(&row.evidence_mask.to_le_bytes());
        payload.extend_from_slice(&row.current_support.to_le_bytes());
        payload.extend_from_slice(&row.current_domain_id.to_le_bytes());
        payload.extend_from_slice(&row.proposed_domain_id.to_le_bytes());
        payload.extend_from_slice(&row.current_row_len.to_le_bytes());
        payload.extend_from_slice(&row.source_ref_id.to_le_bytes());
    }
    payload
}

pub fn append_compact_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
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
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
}

pub fn gate_compact_stream(raw: &[u8], expected_domains: &[u16]) -> LaneGate {
    let mut summary = summarize_compact_stream(raw);
    summary.coverage_errors = frame_shape_errors(
        &summary.unique_hours,
        &summary.frame_hour_counts,
        &summary.per_hour_perf_domain_counts,
        &summary.per_hour_data_domain_counts,
        &summary.per_hour_perf_row_cardinality,
        &summary.per_hour_data_row_cardinality,
        expected_domains,
    );
    let ok = summary.parse_errors.is_empty()
        && summary.ordering_errors.is_empty()
        && summary.coverage_errors.is_empty()
        && summary.failure_status_flags_or == 0;
    LaneGate {
        ok,
        expected_domains: expected_domains.to_vec(),
        summary,
    }
}

pub fn summarize_compact_stream(raw: &[u8]) -> CompactStreamSummary {
    let mut summary = CompactStreamSummary {
        total_bytes: raw.len(),
        ordered: true,
        ..CompactStreamSummary::default()
    };
    let mut offset = 0usize;
    let mut previous_sequence: Option<u64> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut open_frame_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut final_record_is_clean_frame_end = false;
    let mut observed_perf_status_or_by_hour = BTreeMap::<i32, u32>::new();
    let mut observed_owned_perf_roster_by_hour = BTreeMap::<i32, BTreeSet<u16>>::new();
    let mut observed_lane_perf_status_by_hour_domain = BTreeMap::<i32, BTreeMap<u16, u32>>::new();

    while offset < raw.len() {
        let remaining = raw.len().saturating_sub(offset);
        let header = match compact_contract::parse_compact_record_header(&raw[offset..]) {
            Ok(header) => header,
            Err(compact_contract::RecordFramingError::TruncatedHeader(_)) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: truncated compact header"));
                break;
            }
            Err(compact_contract::RecordFramingError::BadMagic) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: invalid compact magic"));
                break;
            }
            Err(error) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: {error}"));
                break;
            }
        };
        let sequence = header.sequence;
        let game_hour_raw = header.game_hour_raw;
        let domain_id = header.domain_id;
        let record_type = header.record_type;
        let payload_len = header.payload_len;
        let record_len = header.record_len();
        if record_len > remaining {
            summary.parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
            ));
            break;
        }
        match previous_sequence {
            None if sequence != 1 => {
                summary.ordered = false;
                summary.ordering_errors.push(format!(
                    "offset {offset}: sequence {sequence} does not start at 1 (stream prefix is missing)"
                ));
            }
            Some(previous) => match previous.checked_add(1) {
                Some(expected) if sequence != expected => {
                    summary.ordered = false;
                    summary.ordering_errors.push(format!(
                        "offset {offset}: sequence discontinuity {previous} -> {sequence} (expected {expected})"
                    ));
                }
                None => {
                    summary.ordered = false;
                    summary.ordering_errors.push(format!(
                        "offset {offset}: sequence space exhausted after {previous}; got {sequence} (wrapping is not allowed)"
                    ));
                }
                _ => {}
            },
            _ => {}
        }
        previous_sequence = Some(sequence);
        final_record_is_clean_frame_end = false;

        let payload_start = offset + COMPACT_RECORD_HEADER_LEN;
        let payload_end = payload_start + payload_len;
        let payload = &raw[payload_start..payload_end];
        let record_semantics_ok = match compact_contract::validate_compact_record_semantics(
            domain_id,
            record_type,
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
        // Domain 250 carries instrumentation-only PERF records. The recorder
        // excludes it from stream/domain/frame cardinality, while sequence and
        // envelope semantics still cover it above.
        if record_type == RECORD_TYPE_DOMAIN_PERF && domain_id == RC_PROBE_DOMAIN_ID {
            offset = offset.saturating_add(record_len);
            continue;
        }
        summary.record_count = summary.record_count.saturating_add(1);
        summary.total_payload_bytes = summary.total_payload_bytes.saturating_add(payload_len);
        summary.max_payload_len = summary.max_payload_len.max(payload_len);

        match record_type {
            RECORD_TYPE_DOMAIN_PERF => {
                let owned_perf_authority = is_owned_perf_authority_domain(domain_id);
                if !owned_perf_authority {
                    summary.parse_errors.push(format!(
                        "offset {offset}: unassigned domain {domain_id} DOMAIN_PERF cannot contribute frame authority"
                    ));
                }
                if record_semantics_ok {
                    let status_flags = read_u32_le(payload, 32);
                    if owned_perf_authority {
                        let inserted = observed_owned_perf_roster_by_hour
                            .entry(game_hour_raw)
                            .or_default()
                            .insert(domain_id);
                        if !inserted {
                            summary.parse_errors.push(format!(
                                "offset {offset}: duplicate owned DOMAIN_PERF for domain {domain_id} in hour {game_hour_raw}"
                            ));
                        }
                        *observed_perf_status_or_by_hour
                            .entry(game_hour_raw)
                            .or_default() |= status_flags;
                    }
                    if is_lane_owned_data_domain(domain_id) {
                        observed_lane_perf_status_by_hour_domain
                            .entry(game_hour_raw)
                            .or_default()
                            .entry(domain_id)
                            .or_insert(status_flags);
                    }
                    decode_domain_perf(offset, game_hour_raw, domain_id, payload, &mut summary);
                    increment_hour_domain_cardinality(
                        &mut summary.per_hour_perf_row_cardinality,
                        game_hour_raw,
                        domain_id,
                        read_u32_le(payload, 16) as u64,
                        read_u32_le(payload, 20) as u64,
                    );
                }
                increment_hour_domain(
                    &mut summary.per_hour_perf_domain_counts,
                    game_hour_raw,
                    domain_id,
                );
                observe_perf_hour(
                    &mut summary,
                    &mut pending_perf_hour,
                    open_frame_hour,
                    last_closed_hour,
                    game_hour_raw,
                    offset,
                );
            }
            RECORD_TYPE_DOMAIN_DATA => {
                if record_semantics_ok && is_lane_owned_data_domain(domain_id) {
                    match observed_lane_perf_status_by_hour_domain
                        .get(&game_hour_raw)
                        .and_then(|domains| domains.get(&domain_id))
                        .copied()
                    {
                        Some(perf_status_flags) if payload.len() >= 20 => {
                            let data_status_flags = read_u32_le(payload, 16);
                            if perf_status_flags != data_status_flags {
                                summary.parse_errors.push(format!(
                                    "offset {offset}: domain {domain_id} DOMAIN_DATA status {data_status_flags:#010x} != preceding same-hour DOMAIN_PERF status {perf_status_flags:#010x}"
                                ));
                            }
                        }
                        Some(_) => {}
                        None => summary.parse_errors.push(format!(
                            "offset {offset}: lane-owned domain {domain_id} DOMAIN_DATA has no preceding same-hour DOMAIN_PERF authority"
                        )),
                    }
                }
                observe_data_hour(
                    &mut summary,
                    pending_perf_hour,
                    &mut open_frame_hour,
                    last_closed_hour,
                    game_hour_raw,
                    offset,
                );
                let domain = domain_entry(&mut summary, domain_id);
                domain.record_count = domain.record_count.saturating_add(1);
                domain.data_record_count = domain.data_record_count.saturating_add(1);
                increment_hour_domain(
                    &mut summary.per_hour_data_domain_counts,
                    game_hour_raw,
                    domain_id,
                );
                if record_semantics_ok {
                    if matches!(
                        domain_id,
                        DOMAIN_FUEL
                            | DOMAIN_PRODUCTION
                            | DOMAIN_STOCKPILE_VARIANT_DEFS
                            | DOMAIN_STOCKPILE_VARIANT_UPGRADES
                            | DOMAIN_GAP_BADGES
                    ) && payload.len() >= 16
                    {
                        increment_hour_domain_cardinality(
                            &mut summary.per_hour_data_row_cardinality,
                            game_hour_raw,
                            domain_id,
                            read_u32_le(payload, 8) as u64,
                            read_u32_le(payload, 12) as u64,
                        );
                    }
                    match domain_id {
                        DOMAIN_FUEL => decode_fuel_payload(offset, payload, &mut summary),
                        DOMAIN_PRODUCTION => {
                            decode_production_payload(offset, payload, &mut summary)
                        }
                        DOMAIN_STOCKPILE_VARIANT_DEFS => {
                            decode_variant_defs_payload(offset, payload, domain_id, &mut summary)
                        }
                        DOMAIN_STOCKPILE_VARIANT_UPGRADES => decode_variant_upgrades_payload(
                            offset,
                            payload,
                            domain_id,
                            &mut summary,
                        ),
                        DOMAIN_GAP_BADGES => {
                            decode_gap_badges_payload(offset, payload, domain_id, &mut summary)
                        }
                        _ => {}
                    }
                }
            }
            RECORD_TYPE_FRAME_END => {
                summary.frame_end_record_count = summary.frame_end_record_count.saturating_add(1);
                *summary.frame_hour_counts.entry(game_hour_raw).or_default() += 1;
                let mut clean = record_semantics_ok;
                if record_semantics_ok {
                    let emitted_domains = read_u32_le(payload, 16);
                    let scheduled_domains = read_u32_le(payload, 20);
                    let status_flags = read_u32_le(payload, 24);
                    let reserved = read_u32_le(payload, 28);
                    clean &= observe_status_flags(
                        &mut summary,
                        DOMAIN_FRAME,
                        "FRAME_END",
                        offset,
                        status_flags,
                    );
                    let observed_perf_status_or = observed_perf_status_or_by_hour
                        .get(&game_hour_raw)
                        .copied()
                        .unwrap_or(0);
                    if status_flags != observed_perf_status_or {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_END status {status_flags:#010x} != OR of independently observed owned DOMAIN_PERF statuses {observed_perf_status_or:#010x}"
                        ));
                    }
                    let observed_owned_perf_domains = observed_owned_perf_roster_by_hour
                        .get(&game_hour_raw)
                        .map(|domains| domains.len())
                        .unwrap_or(0);
                    if usize::try_from(emitted_domains).ok() != Some(observed_owned_perf_domains) {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_END emitted_domains {emitted_domains} != independently observed owned DOMAIN_PERF roster count {observed_owned_perf_domains}"
                        ));
                    }
                    if scheduled_domains != emitted_domains {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_END scheduled_domains {scheduled_domains} != emitted_domains {emitted_domains}"
                        ));
                    }
                    if reserved != 0 {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_END reserved word is nonzero ({reserved:#x})"
                        ));
                    }
                }
                match open_frame_hour {
                    Some(open) if open == game_hour_raw => {}
                    Some(open) => {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_END hour {game_hour_raw} does not close open DATA frame hour {open}"
                        ));
                    }
                    None => {
                        clean = false;
                        summary.parse_errors.push(format!(
                            "offset {offset}: FRAME_END hour {game_hour_raw} has no open DATA frame"
                        ));
                    }
                }
                if let Some(closed) = last_closed_hour {
                    match closed.checked_add(1) {
                        Some(expected) if game_hour_raw == expected => {}
                        Some(expected) => {
                            clean = false;
                            summary.parse_errors.push(format!(
                                "offset {offset}: FRAME_END hour {game_hour_raw} does not follow closed frame hour {closed} (expected {expected})"
                            ));
                        }
                        None => {
                            clean = false;
                            summary.parse_errors.push(format!(
                                "offset {offset}: closed frame hour {closed} exhausted; another frame is forbidden"
                            ));
                        }
                    }
                }
                if clean {
                    open_frame_hour = None;
                    pending_perf_hour = None;
                    last_closed_hour = Some(game_hour_raw);
                    final_record_is_clean_frame_end = true;
                }
            }
            other => summary
                .parse_errors
                .push(format!("offset {offset}: unknown record type {other}")),
        }
        offset = offset.saturating_add(record_len);
    }

    if let Some(open) = open_frame_hour {
        summary
            .parse_errors
            .push(format!("stream ended with open DATA frame hour {open}"));
    }
    if !final_record_is_clean_frame_end {
        summary
            .parse_errors
            .push("stream is not closed by a final canonical FRAME_END record".to_string());
    }

    summary.unique_hours = summary.frame_hour_counts.keys().copied().collect();
    summary.first_hour = summary.unique_hours.first().copied();
    summary.last_hour = summary.unique_hours.last().copied();
    summary.skipped_hours = skipped_hours(&summary.unique_hours);
    summary.coverage_errors.clear();
    summary
}

fn validate_next_frame_hour(
    summary: &mut CompactStreamSummary,
    last_closed_hour: Option<i32>,
    game_hour_raw: i32,
    record_label: &str,
    offset: usize,
) {
    let Some(closed) = last_closed_hour else {
        return;
    };
    match closed.checked_add(1) {
        Some(expected) if game_hour_raw == expected => {}
        Some(expected) => summary.parse_errors.push(format!(
            "offset {offset}: {record_label} hour {game_hour_raw} follows closed frame hour {closed}; expected {expected}"
        )),
        None => summary.parse_errors.push(format!(
            "offset {offset}: closed frame hour {closed} exhausted; another record is forbidden"
        )),
    }
}

fn observe_perf_hour(
    summary: &mut CompactStreamSummary,
    pending_perf_hour: &mut Option<i32>,
    open_frame_hour: Option<i32>,
    last_closed_hour: Option<i32>,
    game_hour_raw: i32,
    offset: usize,
) {
    match open_frame_hour {
        Some(open) if open != game_hour_raw => summary.parse_errors.push(format!(
            "offset {offset}: DOMAIN_PERF hour {game_hour_raw} does not match open DATA frame hour {open}"
        )),
        Some(_) => {}
        None => {
            validate_next_frame_hour(
                summary,
                last_closed_hour,
                game_hour_raw,
                "DOMAIN_PERF",
                offset,
            );
            match *pending_perf_hour {
                Some(pending) if pending != game_hour_raw => summary.parse_errors.push(format!(
                    "offset {offset}: DOMAIN_PERF hour {game_hour_raw} conflicts with pending frame hour {pending}"
                )),
                Some(_) => {}
                None => *pending_perf_hour = Some(game_hour_raw),
            }
        }
    }
}

fn observe_data_hour(
    summary: &mut CompactStreamSummary,
    pending_perf_hour: Option<i32>,
    open_frame_hour: &mut Option<i32>,
    last_closed_hour: Option<i32>,
    game_hour_raw: i32,
    offset: usize,
) {
    match *open_frame_hour {
        Some(open) if open != game_hour_raw => summary.parse_errors.push(format!(
            "offset {offset}: DOMAIN_DATA hour {game_hour_raw} encountered before FRAME_END closed open frame hour {open}"
        )),
        Some(_) => {}
        None => {
            validate_next_frame_hour(
                summary,
                last_closed_hour,
                game_hour_raw,
                "DOMAIN_DATA",
                offset,
            );
            if let Some(pending) = pending_perf_hour {
                if pending != game_hour_raw {
                    summary.parse_errors.push(format!(
                        "offset {offset}: DOMAIN_DATA hour {game_hour_raw} conflicts with pending PERF hour {pending}"
                    ));
                }
            }
            *open_frame_hour = Some(game_hour_raw);
        }
    }
}

fn observe_status_flags(
    summary: &mut CompactStreamSummary,
    domain_id: u16,
    origin: &str,
    offset: usize,
    status_flags: u32,
) -> bool {
    summary.status_flags_or |= status_flags;

    let unknown = compact_contract::compact_status_unknown_flags_v1(status_flags);
    if unknown != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: domain {domain_id} {origin} status {status_flags:#010x} contains unknown canonical bits {unknown:#010x}"
        ));
    }

    let fatal = compact_contract::compact_status_failure_flags_after_expected_partial_tolerance(
        domain_id,
        status_flags,
    );
    summary.failure_status_flags_or |= fatal;
    if fatal != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: domain {domain_id} {origin} carries fatal status flags {fatal:#010x} after canonical lane-local normalization"
        ));
    }

    unknown == 0 && fatal == 0
}

fn decode_domain_perf(
    offset: usize,
    _game_hour_raw: i32,
    domain_id: u16,
    payload: &[u8],
    summary: &mut CompactStreamSummary,
) {
    if payload.len() < compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN {
        return;
    }
    let read_time_us = read_u64_le(payload, 8);
    let row_count = read_u32_le(payload, 16) as u64;
    let changed_rows = read_u32_le(payload, 20) as u64;
    let unit_count = read_u32_le(payload, 24) as u64;
    let changed_unit_count = read_u32_le(payload, 28) as u64;
    let status_flags = read_u32_le(payload, 32);
    let reserved = read_u32_le(payload, 36);
    observe_status_flags(summary, domain_id, "DOMAIN_PERF", offset, status_flags);
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: domain {domain_id} PERF changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if changed_unit_count > unit_count {
        summary.parse_errors.push(format!(
            "offset {offset}: domain {domain_id} PERF changed_unit_count {changed_unit_count} > unit_count {unit_count}"
        ));
    }
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: domain {domain_id} PERF reserved word is nonzero ({reserved:#x})"
        ));
    }
    let domain = domain_entry(summary, domain_id);
    domain.record_count = domain.record_count.saturating_add(1);
    domain.perf_record_count = domain.perf_record_count.saturating_add(1);
    domain.row_count_total = domain.row_count_total.saturating_add(row_count);
    domain.changed_rows_total = domain.changed_rows_total.saturating_add(changed_rows);
    domain.unit_count_total = domain.unit_count_total.saturating_add(unit_count);
    domain.changed_unit_count_total = domain
        .changed_unit_count_total
        .saturating_add(changed_unit_count);
    domain.status_flags_or |= status_flags;
    add_unit(domain, "perf_read_time_us_total", read_time_us);
}

fn decode_fuel_payload(offset: usize, payload: &[u8], summary: &mut CompactStreamSummary) {
    if payload.len() < 24 {
        summary.parse_errors.push(format!(
            "offset {offset}: fuel payload shorter than 24 bytes"
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    observe_status_flags(summary, DOMAIN_FUEL, "DOMAIN_DATA", offset, status_flags);
    if scale_raw != FUEL_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: fuel scale {scale_raw} != expected {FUEL_SCALE_RAW}"
        ));
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: fuel changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    let expected_len = 24usize.saturating_add(row_count.saturating_mul(FUEL_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: fuel payload length {} != expected {expected_len}",
            payload.len()
        ));
        return;
    }
    let domain = domain_entry(summary, DOMAIN_FUEL);
    domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
    domain.data_changed_rows_total = domain
        .data_changed_rows_total
        .saturating_add(changed_rows as u64);
    domain.status_flags_or |= status_flags;
    add_unit(domain, "fuel_country_rows", row_count as u64);

    let mut seen = BTreeSet::new();
    for row_index in 0..row_count {
        let cursor = 24 + row_index * FUEL_ROW_LEN;
        let tag_db_id = read_u32_le(payload, cursor);
        let row_flags = read_u32_le(payload, cursor + 4);
        let current_raw = read_i64_le(payload, cursor + 8);
        let max_raw = read_i64_le(payload, cursor + 16);
        let _gain_raw = read_i64_le(payload, cursor + 24);
        let _cost_raw = read_i64_le(payload, cursor + 32);
        if tag_db_id == 0 || tag_db_id > MAX_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: fuel row {row_index} invalid tag_db_id {tag_db_id}"
            ));
        } else if !seen.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: fuel row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if row_flags & !1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: fuel row {row_index} unknown flags {row_flags}"
            ));
        }
        if current_raw < 0 || max_raw < 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: fuel row {row_index} has negative current/max raw values"
            ));
        }
    }
}

fn decode_production_payload(offset: usize, payload: &[u8], summary: &mut CompactStreamSummary) {
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: production payload shorter than 40 bytes"
        ));
        return;
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    let stockpile_count = read_u32_le(payload, 24) as usize;
    let military_count = read_u32_le(payload, 28) as usize;
    let naval_count = read_u32_le(payload, 32) as usize;
    let construction_count = read_u32_le(payload, 36) as usize;
    observe_status_flags(
        summary,
        DOMAIN_PRODUCTION,
        "DOMAIN_DATA",
        offset,
        status_flags,
    );
    if scale_raw != PRODUCTION_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: production scale {scale_raw} != expected {PRODUCTION_SCALE_RAW}"
        ));
    }
    if changed_rows > country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: production changed_rows {changed_rows} > country_count {country_count}"
        ));
    }
    let line_count = military_count.saturating_add(naval_count);
    let expected_len = 40usize
        .saturating_add(stockpile_count.saturating_mul(PRODUCTION_STOCKPILE_ROW_LEN))
        .saturating_add(line_count.saturating_mul(PRODUCTION_LINE_ROW_LEN))
        .saturating_add(construction_count.saturating_mul(PRODUCTION_CONSTRUCTION_ROW_LEN));
    if payload.len() < expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: production payload length {} < expected {expected_len}",
            payload.len()
        ));
        return;
    }
    let domain = domain_entry(summary, DOMAIN_PRODUCTION);
    domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
    domain.data_changed_rows_total = domain
        .data_changed_rows_total
        .saturating_add(changed_rows as u64);
    domain.status_flags_or |= status_flags;
    add_unit(domain, "production_country_rows", country_count as u64);
    add_unit(domain, "stockpile_rows", stockpile_count as u64);
    add_unit(domain, "military_line_rows", military_count as u64);
    add_unit(domain, "naval_line_rows", naval_count as u64);
    add_unit(domain, "construction_queue_rows", construction_count as u64);

    let mut cursor = 40usize;
    let mut seen_stockpile = BTreeSet::new();
    for row_index in 0..stockpile_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let variant_id = read_u32_le(payload, cursor + 4);
        let _amount_raw = read_i64_le(payload, cursor + 8);
        if tag_db_id == 0 || tag_db_id > MAX_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: stockpile row {row_index} invalid tag_db_id {tag_db_id}"
            ));
        }
        if variant_id == 0 || variant_id > MAX_VARIANT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: stockpile row {row_index} invalid variant_id {variant_id}"
            ));
        }
        if !seen_stockpile.insert((tag_db_id, variant_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: stockpile row {row_index} duplicates tag_db_id {tag_db_id} variant_id {variant_id}"
            ));
        }
        cursor += PRODUCTION_STOCKPILE_ROW_LEN;
    }

    let mut seen_lines = BTreeSet::new();
    for row_index in 0..line_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let kind = read_u32_le(payload, cursor + 4);
        let line_id = read_i32_le(payload, cursor + 8);
        let active_factories = read_i32_le(payload, cursor + 12);
        let _cost_raw = read_i32_le(payload, cursor + 16);
        let _speed_raw = read_i32_le(payload, cursor + 20);
        let _produced_raw = read_i32_le(payload, cursor + 24);
        let _amount = read_i32_le(payload, cursor + 28);
        if tag_db_id == 0 || tag_db_id > MAX_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: production line row {row_index} invalid tag_db_id {tag_db_id}"
            ));
        }
        if kind != 1 && kind != 2 {
            summary.parse_errors.push(format!(
                "offset {offset}: production line row {row_index} invalid kind {kind}"
            ));
        }
        if !(0..=MAX_FACTORIES).contains(&active_factories) {
            summary.parse_errors.push(format!(
                "offset {offset}: production line row {row_index} invalid active_factories {active_factories}"
            ));
        }
        if !seen_lines.insert((tag_db_id, kind, line_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: production line row {row_index} duplicates tag_db_id {tag_db_id} kind {kind} line_id {line_id}"
            ));
        }
        cursor += PRODUCTION_LINE_ROW_LEN;
    }

    let mut seen_construction = BTreeSet::new();
    for row_index in 0..construction_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let line_id = read_u32_le(payload, cursor + 4);
        let _template_id = read_u32_le(payload, cursor + 8);
        let active_factories = read_i32_le(payload, cursor + 12);
        let cost_raw = read_i32_le(payload, cursor + 16);
        let produced_raw = read_i32_le(payload, cursor + 20);
        let priority = read_u32_le(payload, cursor + 24);
        let reserved = read_u32_le(payload, cursor + 28);
        if tag_db_id == 0 || tag_db_id > MAX_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: construction row {row_index} invalid tag_db_id {tag_db_id}"
            ));
        }
        if !(0..=MAX_FACTORIES).contains(&active_factories) {
            summary.parse_errors.push(format!(
                "offset {offset}: construction row {row_index} invalid active_factories {active_factories}"
            ));
        }
        if cost_raw < 0 || produced_raw < 0 || reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: construction row {row_index} invalid progress/reserved fields"
            ));
        }
        if !seen_construction.insert((tag_db_id, line_id, priority)) {
            summary.parse_errors.push(format!(
                "offset {offset}: construction row {row_index} duplicates tag_db_id {tag_db_id} line_id {line_id} priority {priority}"
            ));
        }
        cursor += PRODUCTION_CONSTRUCTION_ROW_LEN;
    }
    if cursor != payload.len() {
        decode_production_trailer_chain(
            offset,
            payload,
            cursor,
            country_count,
            &seen_stockpile,
            &seen_lines,
            summary,
        );
    }
}

fn decode_production_trailer_chain(
    offset: usize,
    payload: &[u8],
    mut cursor: usize,
    country_count: usize,
    base_stockpile: &BTreeSet<(u32, u32)>,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut CompactStreamSummary,
) {
    let mut plic_seen = false;
    let mut pvj_seen = false;
    let mut pfr_seen = false;
    let mut ppr_seen = false;
    let mut pls_seen = false;
    let mut pef_seen = false;
    let mut pmi_seen = false;
    let mut pcp_seen = false;
    while cursor < payload.len() {
        let Some(magic) = payload.get(cursor..cursor.saturating_add(4)) else {
            summary.parse_errors.push(format!(
                "offset {offset}: production trailer at {cursor} has truncated magic"
            ));
            return;
        };
        if magic == b"PLIC" {
            if plic_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PLIC trailer"
                ));
                return;
            }
            plic_seen = true;
            let Some(next) = decode_production_plic_segment(offset, payload, cursor, summary)
            else {
                return;
            };
            cursor = next;
        } else if magic == b"PVJ1" {
            if pvj_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PVJ1 trailer"
                ));
                return;
            }
            pvj_seen = true;
            let Some(next) =
                decode_production_pvj1_segment(offset, payload, cursor, base_lines, summary)
            else {
                return;
            };
            cursor = next;
        } else if magic == b"PFR1" {
            if pfr_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PFR1 trailer"
                ));
                return;
            }
            pfr_seen = true;
            let Some(next) =
                decode_production_pfr1_segment(offset, payload, cursor, base_lines, summary)
            else {
                return;
            };
            cursor = next;
        } else if magic == b"PPR1" {
            if ppr_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PPR1 trailer"
                ));
                return;
            }
            ppr_seen = true;
            let Some(next) =
                decode_production_ppr1_segment(offset, payload, cursor, base_lines, summary)
            else {
                return;
            };
            cursor = next;
        } else if magic == b"PLS1" {
            if pls_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PLS1 trailer"
                ));
                return;
            }
            pls_seen = true;
            let Some(next) =
                decode_production_pls1_segment(offset, payload, cursor, base_lines, summary)
            else {
                return;
            };
            cursor = next;
        } else if magic == b"PCP1" {
            if pcp_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PCP1 trailer"
                ));
                return;
            }
            pcp_seen = true;
            let Some(next) = decode_production_pcp1_segment(
                offset,
                payload,
                cursor,
                country_count,
                base_stockpile,
                base_lines,
                summary,
            ) else {
                return;
            };
            cursor = next;
        } else if magic == b"PEF1" {
            if pef_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PEF1 trailer"
                ));
                return;
            }
            pef_seen = true;
            let Some(next) =
                decode_production_pef1_segment(offset, payload, cursor, base_lines, summary)
            else {
                return;
            };
            cursor = next;
        } else if magic == b"PMI1" {
            if pmi_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PMI1 trailer"
                ));
                return;
            }
            pmi_seen = true;
            let Some(next) =
                decode_production_pmi1_segment(offset, payload, cursor, base_lines, summary)
            else {
                return;
            };
            cursor = next;
        } else {
            summary.parse_errors.push(format!(
                "offset {offset}: production trailing bytes are not a known D11 trailer"
            ));
            return;
        }
    }
}

fn decode_production_pmi1_segment(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut CompactStreamSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    let trailer = payload.get(trailer_offset..)?;
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PMI1 trailer shorter than {HEADER_LEN} bytes"
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let row_count = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(PRODUCTION_MIO_JOIN_ROW_LEN));
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary
            .parse_errors
            .push(format!("offset {offset}: PMI1 header/length is invalid"));
        return None;
    }

    let mut cursor = HEADER_LEN;
    let mut seen = BTreeSet::new();
    let mut assigned_count = 0u64;
    for row_index in 0..row_count {
        let tag = read_u32_le(trailer, cursor);
        let kind = read_u32_le(trailer, cursor + 4);
        let line_id = read_i32_le(trailer, cursor + 8);
        let mio_id = read_u32_le(trailer, cursor + 12);
        cursor += PRODUCTION_MIO_JOIN_ROW_LEN;
        if tag == 0 || (kind != 1 && kind != 2) {
            summary.parse_errors.push(format!(
                "offset {offset}: PMI1 row {row_index} has invalid tag/kind"
            ));
        }
        if mio_id != 0 {
            assigned_count += 1;
        }
        if !seen.insert((tag, kind, line_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: PMI1 row {row_index} duplicates line key"
            ));
        }
    }
    if seen != *base_lines {
        summary.parse_errors.push(format!(
            "offset {offset}: PMI1 line coverage is not bijective: trailer_rows={} base_lines={}",
            seen.len(),
            base_lines.len()
        ));
    }
    let domain = domain_entry(summary, DOMAIN_PRODUCTION);
    add_unit(domain, "production_mio_join_rows", row_count as u64);
    add_unit(domain, "production_mio_assigned_rows", assigned_count);
    add_unit(
        domain,
        "production_mio_proven_null_rows",
        row_count as u64 - assigned_count,
    );
    Some(trailer_offset + segment_len)
}

fn decode_production_pcp1_segment(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    country_count: usize,
    base_stockpile: &BTreeSet<(u32, u32)>,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut CompactStreamSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    const KNOWN_FLAGS: u32 = 0b111;
    let trailer = payload.get(trailer_offset..)?;
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PCP1 trailer shorter than {HEADER_LEN} bytes"
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let row_count = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(8));
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || row_count != country_count
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary.parse_errors.push(format!(
            "offset {offset}: PCP1 header/count/length is invalid"
        ));
        return None;
    }
    let mut cursor = HEADER_LEN;
    let mut seen = BTreeSet::new();
    let mut complete_counts = [0u64; 3];
    for row_index in 0..row_count {
        let tag_db_id = read_u32_le(trailer, cursor);
        let flags = read_u32_le(trailer, cursor + 4);
        cursor += 8;
        if tag_db_id == 0 || tag_db_id > MAX_TAG_DB_ID || !seen.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: PCP1 row {row_index} has invalid/duplicate tag_db_id {tag_db_id}"
            ));
        }
        if flags & !KNOWN_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: PCP1 row {row_index} has unknown flags {flags:#x}"
            ));
        }
        for (bit, count) in complete_counts.iter_mut().enumerate() {
            if flags & (1 << bit) != 0 {
                *count += 1;
            }
        }
    }
    for &(tag_db_id, _) in base_stockpile {
        if trailer_flags(&seen, trailer, tag_db_id) & 1 == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: PCP1 marks stockpile unreadable for tag {tag_db_id} with base rows"
            ));
        }
    }
    for &(tag_db_id, kind, _) in base_lines {
        let required = if kind == 2 { 1 << 2 } else { 1 << 1 };
        if trailer_flags(&seen, trailer, tag_db_id) & required == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: PCP1 marks production line kind {kind} unreadable for tag {tag_db_id} with base rows"
            ));
        }
    }
    let domain = domain_entry(summary, DOMAIN_PRODUCTION);
    add_unit(domain, "production_completeness_rows", row_count as u64);
    add_unit(domain, "stockpile_complete_rows", complete_counts[0]);
    add_unit(domain, "military_lines_complete_rows", complete_counts[1]);
    add_unit(domain, "naval_lines_complete_rows", complete_counts[2]);
    Some(trailer_offset + segment_len)
}

fn trailer_flags(seen: &BTreeSet<u32>, trailer: &[u8], tag_db_id: u32) -> u32 {
    if !seen.contains(&tag_db_id) {
        return 0;
    }
    let row_count = read_u32_le(trailer, 12) as usize;
    for row_index in 0..row_count {
        let cursor = 20 + row_index * 8;
        if read_u32_le(trailer, cursor) == tag_db_id {
            return read_u32_le(trailer, cursor + 4);
        }
    }
    0
}

fn decode_production_pef1_segment(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut CompactStreamSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    let trailer = payload.get(trailer_offset..)?;
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PEF1 trailer shorter than {HEADER_LEN} bytes"
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let row_count = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let expected =
        HEADER_LEN.saturating_add(row_count.saturating_mul(PRODUCTION_EFFICIENCY_ROW_LEN));
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary
            .parse_errors
            .push(format!("offset {offset}: PEF1 header/length is invalid"));
        return None;
    }

    let mut cursor = HEADER_LEN;
    let mut seen = BTreeSet::new();
    for row_index in 0..row_count {
        let tag = read_u32_le(trailer, cursor);
        let kind = read_u32_le(trailer, cursor + 4);
        let line_id = read_i32_le(trailer, cursor + 8);
        let efficiency_raw = read_i64_le(trailer, cursor + 12);
        let efficiency_cap_raw = read_i64_le(trailer, cursor + 20);
        let efficiency_growth_raw = read_i64_le(trailer, cursor + 28);
        cursor += PRODUCTION_EFFICIENCY_ROW_LEN;
        if tag == 0 || (kind != 1 && kind != 2) {
            summary.parse_errors.push(format!(
                "offset {offset}: PEF1 row {row_index} has invalid tag/kind"
            ));
        }
        if efficiency_raw < 0 || efficiency_cap_raw < 0 || efficiency_growth_raw < 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: PEF1 row {row_index} has a negative producer result"
            ));
        }
        if !seen.insert((tag, kind, line_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: PEF1 row {row_index} duplicates line key"
            ));
        }
    }
    if seen != *base_lines {
        summary.parse_errors.push(format!(
            "offset {offset}: PEF1 line coverage is not bijective: trailer_rows={} base_lines={}",
            seen.len(),
            base_lines.len()
        ));
    }
    let domain = domain_entry(summary, DOMAIN_PRODUCTION);
    add_unit(
        domain,
        "production_efficiency_producer_rows",
        row_count as u64,
    );
    Some(trailer_offset + segment_len)
}

fn decode_production_pls1_segment(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut CompactStreamSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    let trailer = payload.get(trailer_offset..)?;
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PLS1 trailer shorter than {HEADER_LEN} bytes"
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let row_count = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(16));
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary
            .parse_errors
            .push(format!("offset {offset}: PLS1 header/length is invalid"));
        return None;
    }
    let mut cursor = HEADER_LEN;
    let mut seen = BTreeSet::new();
    let mut subtype_counts = [0u64; 4];
    for row_index in 0..row_count {
        let tag = read_u32_le(trailer, cursor);
        let kind = read_u32_le(trailer, cursor + 4);
        let line_id = read_i32_le(trailer, cursor + 8);
        let subtype = read_u32_le(trailer, cursor + 12);
        cursor += 16;
        let coherent = matches!((kind, subtype), (1, 1 | 4) | (2, 2 | 3));
        if tag == 0 || !coherent {
            summary.parse_errors.push(format!(
                "offset {offset}: PLS1 row {row_index} has invalid tag/kind/subtype"
            ));
        }
        if (1..=4).contains(&subtype) {
            subtype_counts[(subtype - 1) as usize] += 1;
        }
        if !seen.insert((tag, kind, line_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: PLS1 row {row_index} duplicates line key"
            ));
        }
    }
    if seen != *base_lines {
        summary.parse_errors.push(format!(
            "offset {offset}: PLS1 line coverage is not bijective: trailer_rows={} base_lines={}",
            seen.len(),
            base_lines.len()
        ));
    }
    let domain = domain_entry(summary, DOMAIN_PRODUCTION);
    add_unit(domain, "production_line_subtype_rows", row_count as u64);
    for (label, count) in [
        ("production_line_subtype_military_rows", subtype_counts[0]),
        ("production_line_subtype_naval_rows", subtype_counts[1]),
        ("production_line_subtype_ship_refit_rows", subtype_counts[2]),
        (
            "production_line_subtype_railway_gun_rows",
            subtype_counts[3],
        ),
    ] {
        add_unit(domain, label, count);
    }
    Some(trailer_offset + segment_len)
}

fn decode_production_plic_segment(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut CompactStreamSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    let trailer = payload.get(trailer_offset..)?;
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PLIC trailer shorter than {HEADER_LEN} bytes"
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let country_rows = read_u32_le(trailer, 8) as usize;
    let id_rows = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let expected = HEADER_LEN
        .saturating_add(country_rows.saturating_mul(8))
        .saturating_add(id_rows.saturating_mul(4));
    if version != 1 || header_len != HEADER_LEN || reserved != 0 || trailer.len() < expected {
        summary
            .parse_errors
            .push(format!("offset {offset}: PLIC header/length is invalid"));
        return None;
    }
    let mut cursor = HEADER_LEN;
    let mut counts = Vec::with_capacity(country_rows);
    let mut seen_tags = BTreeSet::new();
    let mut total_ids = 0usize;
    for row_index in 0..country_rows {
        let tag = read_u32_le(trailer, cursor);
        let count = read_u32_le(trailer, cursor + 4) as usize;
        cursor += 8;
        if tag == 0 || !seen_tags.insert(tag) {
            summary.parse_errors.push(format!(
                "offset {offset}: PLIC country row {row_index} has invalid/duplicate tag {tag}"
            ));
        }
        total_ids = total_ids.saturating_add(count);
        counts.push((tag, count));
    }
    if total_ids != id_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: PLIC id total {total_ids} != id_rows {id_rows}"
        ));
        return None;
    }
    for (tag, count) in counts {
        let mut seen_ids = BTreeSet::new();
        for _ in 0..count {
            let variant_id = read_u32_le(trailer, cursor);
            cursor += 4;
            if variant_id == 0 || variant_id >= (1 << 24) || !seen_ids.insert(variant_id) {
                summary.parse_errors.push(format!(
                    "offset {offset}: PLIC tag {tag} has invalid/duplicate variant_id {variant_id}"
                ));
            }
        }
    }
    let domain = domain_entry(summary, DOMAIN_PRODUCTION);
    add_unit(
        domain,
        "production_license_country_rows",
        country_rows as u64,
    );
    add_unit(domain, "production_license_id_rows", id_rows as u64);
    Some(trailer_offset + expected)
}

fn decode_production_pvj1_segment(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut CompactStreamSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    let trailer = payload.get(trailer_offset..)?;
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PVJ1 trailer shorter than {HEADER_LEN} bytes"
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let row_count = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(16));
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary
            .parse_errors
            .push(format!("offset {offset}: PVJ1 header/length is invalid"));
        return None;
    }
    let mut cursor = HEADER_LEN;
    let mut seen = BTreeSet::new();
    for row_index in 0..row_count {
        let tag = read_u32_le(trailer, cursor);
        let kind = read_u32_le(trailer, cursor + 4);
        let line_id = read_i32_le(trailer, cursor + 8);
        let variant_id = read_u32_le(trailer, cursor + 12);
        cursor += 16;
        if tag == 0 || (kind != 1 && kind != 2) || variant_id == 0 || variant_id > MAX_VARIANT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: PVJ1 row {row_index} has invalid tag/kind/variant"
            ));
        }
        if !seen.insert((tag, kind, line_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: PVJ1 row {row_index} duplicates line key"
            ));
        }
    }
    if seen != *base_lines {
        summary.parse_errors.push(format!(
            "offset {offset}: PVJ1 line coverage is not bijective: trailer_rows={} base_lines={}",
            seen.len(),
            base_lines.len()
        ));
    }
    let domain = domain_entry(summary, DOMAIN_PRODUCTION);
    add_unit(domain, "production_variant_join_rows", row_count as u64);
    Some(trailer_offset + segment_len)
}

fn decode_production_pfr1_segment(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut CompactStreamSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    let trailer = payload.get(trailer_offset..)?;
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PFR1 trailer shorter than {HEADER_LEN} bytes"
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let row_count = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(16));
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary
            .parse_errors
            .push(format!("offset {offset}: PFR1 header/length is invalid"));
        return None;
    }
    let mut cursor = HEADER_LEN;
    let mut seen = BTreeSet::new();
    for row_index in 0..row_count {
        let tag = read_u32_le(trailer, cursor);
        let kind = read_u32_le(trailer, cursor + 4);
        let line_id = read_i32_le(trailer, cursor + 8);
        let requested_factories = read_i32_le(trailer, cursor + 12);
        cursor += 16;
        if tag == 0
            || (kind != 1 && kind != 2)
            || !(0..=MAX_FACTORIES).contains(&requested_factories)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: PFR1 row {row_index} has invalid tag/kind/requested_factories"
            ));
        }
        if !seen.insert((tag, kind, line_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: PFR1 row {row_index} duplicates line key"
            ));
        }
    }
    if seen != *base_lines {
        summary.parse_errors.push(format!(
            "offset {offset}: PFR1 line coverage is not bijective: trailer_rows={} base_lines={}",
            seen.len(),
            base_lines.len()
        ));
    }
    let domain = domain_entry(summary, DOMAIN_PRODUCTION);
    add_unit(
        domain,
        "production_requested_factories_rows",
        row_count as u64,
    );
    Some(trailer_offset + segment_len)
}

/// Decode the exact a729 line+0x44 priority/order witness. PPR1 must cover the
/// frozen D11 equipment-line key set bijectively and never accepts a fabricated
/// ordinal outside the collector's bounded production vector.
fn decode_production_ppr1_segment(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut CompactStreamSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    const ROW_LEN: usize = 16;
    const MAX_PRIORITY: i32 = 4095;
    let Some(trailer) = payload.get(trailer_offset..) else {
        summary
            .parse_errors
            .push(format!("offset {offset}: PPR1 offset beyond payload"));
        return None;
    };
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PPR1 trailer shorter than {HEADER_LEN} bytes"
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let row_count = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary
            .parse_errors
            .push(format!("offset {offset}: PPR1 header/length is invalid"));
        return None;
    }
    let mut cursor = HEADER_LEN;
    let mut seen = BTreeSet::new();
    for row_index in 0..row_count {
        let tag_db_id = read_u32_le(trailer, cursor);
        let kind = read_u32_le(trailer, cursor + 4);
        let line_id = read_i32_le(trailer, cursor + 8);
        let priority = read_i32_le(trailer, cursor + 12);
        cursor += ROW_LEN;
        let key = (tag_db_id, kind, line_id);
        if tag_db_id == 0 || (kind != 1 && kind != 2) || !(0..=MAX_PRIORITY).contains(&priority) {
            summary.parse_errors.push(format!(
                "offset {offset}: PPR1 row {row_index} has invalid tag/kind/priority"
            ));
        }
        if !seen.insert(key) {
            summary.parse_errors.push(format!(
                "offset {offset}: PPR1 row {row_index} duplicates line key"
            ));
        }
    }
    if seen != *base_lines {
        summary.parse_errors.push(format!(
            "offset {offset}: PPR1 line coverage is not bijective: trailer_rows={} base_lines={}",
            seen.len(),
            base_lines.len()
        ));
    }
    let domain = domain_entry(summary, DOMAIN_PRODUCTION);
    add_unit(domain, "production_priority_rows", row_count as u64);
    Some(trailer_offset + segment_len)
}

fn decode_variant_defs_payload(
    offset: usize,
    payload: &[u8],
    domain_id: u16,
    summary: &mut CompactStreamSummary,
) {
    if payload.len() < 44 {
        summary.parse_errors.push(format!(
            "offset {offset}: variant defs payload shorter than 44 bytes"
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_variant_rows = read_u32_le(payload, 20) as usize;
    let observed_module_rows = read_u32_le(payload, 24) as usize;
    let emitted_module_rows = read_u32_le(payload, 28) as usize;
    let invalid_variant_rows = read_u32_le(payload, 32) as usize;
    let reserved0 = read_u32_le(payload, 36);
    let reserved1 = read_u32_le(payload, 40);
    observe_status_flags(summary, domain_id, "DOMAIN_DATA", offset, status_flags);
    if changed_rows > row_count || invalid_variant_rows > observed_variant_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: variant defs impossible row counters"
        ));
    }
    if emitted_module_rows > observed_module_rows && (status_flags & STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: variant defs emitted_module_rows exceed observed_module_rows"
        ));
    }
    if reserved0 != 0 || reserved1 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: variant defs reserved header is nonzero"
        ));
    }
    let rows_end = 44usize.saturating_add(row_count.saturating_mul(VARIANT_DEF_ROW_LEN));
    if payload.len() < rows_end {
        summary.parse_errors.push(format!(
            "offset {offset}: variant defs payload length {} shorter than rows end {rows_end}",
            payload.len()
        ));
        return;
    }
    let domain = domain_entry(summary, domain_id);
    domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
    domain.data_changed_rows_total = domain
        .data_changed_rows_total
        .saturating_add(changed_rows as u64);
    domain.status_flags_or |= status_flags;
    add_unit(domain, "stockpile_variant_def_rows", row_count as u64);
    add_unit(
        domain,
        "stockpile_variant_def_observed_variant_rows",
        observed_variant_rows as u64,
    );
    add_unit(
        domain,
        "stockpile_variant_def_observed_module_rows",
        observed_module_rows as u64,
    );
    add_unit(
        domain,
        "stockpile_variant_def_emitted_module_rows",
        emitted_module_rows as u64,
    );
    add_unit(
        domain,
        "stockpile_variant_def_invalid_variant_rows",
        invalid_variant_rows as u64,
    );

    let mut cursor = 44usize;
    let mut seen_variants = BTreeSet::new();
    let mut rows = Vec::with_capacity(row_count);
    let mut declared_modules = 0usize;
    for row_index in 0..row_count {
        let variant_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let module_first = read_u32_le(payload, cursor + 8) as usize;
        let module_count = read_u32_le(payload, cursor + 12) as usize;
        let row_reserved0 = read_u32_le(payload, cursor + 16);
        let row_reserved1 = read_u32_le(payload, cursor + 20);
        cursor += VARIANT_DEF_ROW_LEN;
        if variant_id == 0 || variant_id > MAX_VARIANT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: variant def row {row_index} invalid variant_id {variant_id}"
            ));
        } else if !seen_variants.insert(variant_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: variant def row {row_index} duplicates variant_id {variant_id}"
            ));
        }
        let known = VARIANT_DEF_FLAG_DISPLAY_NAME
            | VARIANT_DEF_FLAG_VARIANT_NAME
            | VARIANT_DEF_FLAG_MODULES;
        if flags & !known != 0 || row_reserved0 != 0 || row_reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: variant def row {row_index} invalid flags/reserved"
            ));
        }
        if module_count == 0 && flags & VARIANT_DEF_FLAG_MODULES != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: variant def row {row_index} has empty module flag"
            ));
        }
        if module_count > 0 && flags & VARIANT_DEF_FLAG_MODULES == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: variant def row {row_index} missing module flag"
            ));
        }
        let Some(module_end) = module_first.checked_add(module_count) else {
            summary.parse_errors.push(format!(
                "offset {offset}: variant def row {row_index} module range overflow"
            ));
            continue;
        };
        if module_end > emitted_module_rows {
            summary.parse_errors.push(format!(
                "offset {offset}: variant def row {row_index} module range exceeds emitted modules"
            ));
        }
        declared_modules = declared_modules.saturating_add(module_count);
        rows.push((flags, module_first, module_count));
    }
    if declared_modules != emitted_module_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: variant defs emitted modules {emitted_module_rows} != declared {declared_modules}"
        ));
    }
    for (row_index, (flags, _, _)) in rows.iter().enumerate() {
        let display_len = match skip_compact_string_len(payload, &mut cursor, "display name") {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        };
        if display_len > MAX_STRING_LEN
            || (display_len == 0 && flags & VARIANT_DEF_FLAG_DISPLAY_NAME != 0)
            || (display_len > 0 && flags & VARIANT_DEF_FLAG_DISPLAY_NAME == 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: variant def row {row_index} invalid display string"
            ));
        }
        let variant_len = match skip_compact_string_len(payload, &mut cursor, "variant name") {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        };
        if variant_len > MAX_STRING_LEN
            || (variant_len == 0 && flags & VARIANT_DEF_FLAG_VARIANT_NAME != 0)
            || (variant_len > 0 && flags & VARIANT_DEF_FLAG_VARIANT_NAME == 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: variant def row {row_index} invalid variant string"
            ));
        }
    }

    let mut covered_modules = BTreeSet::new();
    for (row_index, (_, first, count)) in rows.iter().enumerate() {
        for module_index in *first..first.saturating_add(*count) {
            if !covered_modules.insert(module_index) {
                summary.parse_errors.push(format!(
                    "offset {offset}: variant def row {row_index} overlaps module {module_index}"
                ));
            }
        }
    }
    let mut module_token_rows = 0usize;
    for module_index in 0..emitted_module_rows {
        if cursor.saturating_add(VARIANT_DEF_MODULE_ROW_LEN) > payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: variant def module row {module_index} truncated"
            ));
            return;
        }
        let slot_handle = read_u32_le(payload, cursor);
        let reserved = read_u32_le(payload, cursor + 4);
        cursor += VARIANT_DEF_MODULE_ROW_LEN;
        if slot_handle == 0 || slot_handle > MAX_SLOT_HANDLE || reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: variant def module row {module_index} invalid slot/reserved"
            ));
        }
        let token_len = match skip_compact_string_len(payload, &mut cursor, "module token") {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        };
        if token_len == 0 || token_len > MAX_STRING_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: variant def module row {module_index} invalid token length {token_len}"
            ));
        } else {
            module_token_rows = module_token_rows.saturating_add(1);
        }
    }
    if covered_modules.len() != emitted_module_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: variant defs covered modules {} != emitted {emitted_module_rows}",
            covered_modules.len()
        ));
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: variant defs payload has {} trailing bytes",
            payload.len().saturating_sub(cursor)
        ));
    }
    let domain = domain_entry(summary, domain_id);
    add_unit(
        domain,
        "stockpile_variant_def_module_token_rows",
        module_token_rows as u64,
    );
}

fn decode_variant_upgrades_payload(
    offset: usize,
    payload: &[u8],
    domain_id: u16,
    summary: &mut CompactStreamSummary,
) {
    if payload.len() < 44 {
        summary.parse_errors.push(format!(
            "offset {offset}: variant upgrades payload shorter than 44 bytes"
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_variant_rows = read_u32_le(payload, 20) as usize;
    let observed_upgrade_rows = read_u32_le(payload, 24) as usize;
    let emitted_upgrade_rows = read_u32_le(payload, 28) as usize;
    let invalid_variant_rows = read_u32_le(payload, 32) as usize;
    let reserved0 = read_u32_le(payload, 36);
    let reserved1 = read_u32_le(payload, 40);
    observe_status_flags(summary, domain_id, "DOMAIN_DATA", offset, status_flags);
    if changed_rows > row_count
        || invalid_variant_rows > observed_variant_rows
        || reserved0 != 0
        || reserved1 != 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: variant upgrades invalid header counters"
        ));
    }
    if emitted_upgrade_rows > observed_upgrade_rows && (status_flags & STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: variant upgrades emitted upgrades exceed observed upgrades"
        ));
    }
    let expected_len = 44usize
        .saturating_add(row_count.saturating_mul(VARIANT_UPGRADE_ROW_LEN))
        .saturating_add(emitted_upgrade_rows.saturating_mul(VARIANT_UPGRADE_ENTRY_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: variant upgrades payload length {} != expected {expected_len}",
            payload.len()
        ));
        return;
    }
    let domain = domain_entry(summary, domain_id);
    domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
    domain.data_changed_rows_total = domain
        .data_changed_rows_total
        .saturating_add(changed_rows as u64);
    domain.status_flags_or |= status_flags;
    add_unit(domain, "stockpile_variant_upgrade_rows", row_count as u64);
    add_unit(
        domain,
        "stockpile_variant_upgrade_observed_variant_rows",
        observed_variant_rows as u64,
    );
    add_unit(
        domain,
        "stockpile_variant_upgrade_observed_upgrade_rows",
        observed_upgrade_rows as u64,
    );
    add_unit(
        domain,
        "stockpile_variant_upgrade_emitted_upgrade_rows",
        emitted_upgrade_rows as u64,
    );
    add_unit(
        domain,
        "stockpile_variant_upgrade_invalid_variant_rows",
        invalid_variant_rows as u64,
    );

    let mut cursor = 44usize;
    let mut rows = Vec::with_capacity(row_count);
    let mut seen_variants = BTreeSet::new();
    let mut declared_upgrades = 0usize;
    for row_index in 0..row_count {
        let variant_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let first = read_u32_le(payload, cursor + 8) as usize;
        let count = read_u32_le(payload, cursor + 12) as usize;
        cursor += VARIANT_UPGRADE_ROW_LEN;
        if variant_id == 0 || variant_id > MAX_VARIANT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: variant upgrade row {row_index} invalid variant_id {variant_id}"
            ));
        } else if !seen_variants.insert(variant_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: variant upgrade row {row_index} duplicates variant_id {variant_id}"
            ));
        }
        if flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: variant upgrade row {row_index} unknown flags {flags}"
            ));
        }
        if first.saturating_add(count) > emitted_upgrade_rows {
            summary.parse_errors.push(format!(
                "offset {offset}: variant upgrade row {row_index} range exceeds emitted rows"
            ));
        }
        declared_upgrades = declared_upgrades.saturating_add(count);
        rows.push((first, count));
    }
    if declared_upgrades != emitted_upgrade_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: variant upgrades emitted rows {emitted_upgrade_rows} != declared {declared_upgrades}"
        ));
    }
    let mut entries = Vec::with_capacity(emitted_upgrade_rows);
    let mut level_sum = 0u64;
    for entry_index in 0..emitted_upgrade_rows {
        let upgrade_index = read_u32_le(payload, cursor);
        let level = read_u32_le(payload, cursor + 4);
        cursor += VARIANT_UPGRADE_ENTRY_ROW_LEN;
        if upgrade_index >= MAX_UPGRADE_INDEX || level > MAX_UPGRADE_LEVEL {
            summary.parse_errors.push(format!(
                "offset {offset}: variant upgrade entry {entry_index} invalid index/level"
            ));
        }
        level_sum = level_sum.saturating_add(level as u64);
        entries.push((upgrade_index, level));
    }
    for (row_index, (first, count)) in rows.iter().enumerate() {
        let mut seen = BTreeSet::new();
        for entry_index in *first..first.saturating_add(*count) {
            if let Some((upgrade_index, _)) = entries.get(entry_index) {
                if !seen.insert(*upgrade_index) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: variant upgrade row {row_index} duplicates upgrade_index {upgrade_index}"
                    ));
                }
            }
        }
    }
    let domain = domain_entry(summary, domain_id);
    add_unit(
        domain,
        "stockpile_variant_upgrade_entry_rows",
        emitted_upgrade_rows as u64,
    );
    add_unit(domain, "stockpile_variant_upgrade_level_sum", level_sum);
}

fn decode_gap_badges_payload(
    offset: usize,
    payload: &[u8],
    domain_id: u16,
    summary: &mut CompactStreamSummary,
) {
    if payload.len() < 44 {
        summary.parse_errors.push(format!(
            "offset {offset}: gap badges payload shorter than 44 bytes"
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let schema_version = read_u32_le(payload, 20);
    let source_blocked = read_u32_le(payload, 24) as usize;
    let schema_blocked = read_u32_le(payload, 28) as usize;
    let owned_elsewhere = read_u32_le(payload, 32) as usize;
    let reserved0 = read_u32_le(payload, 36);
    let reserved1 = read_u32_le(payload, 40);
    observe_status_flags(summary, domain_id, "DOMAIN_DATA", offset, status_flags);
    if schema_version != GAP_BADGE_SCHEMA_VERSION {
        summary.parse_errors.push(format!(
            "offset {offset}: gap badges schema {schema_version} != expected {GAP_BADGE_SCHEMA_VERSION}"
        ));
    }
    if changed_rows > row_count || reserved0 != 0 || reserved1 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: gap badges invalid header counters/reserved fields"
        ));
    }
    let expected_len = 44usize.saturating_add(row_count.saturating_mul(GAP_BADGE_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: gap badges payload length {} != expected {expected_len}",
            payload.len()
        ));
        return;
    }

    let domain = domain_entry(summary, domain_id);
    domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
    domain.data_changed_rows_total = domain
        .data_changed_rows_total
        .saturating_add(changed_rows as u64);
    domain.status_flags_or |= status_flags;
    add_unit(domain, "remaining_gap_badge_rows", row_count as u64);
    add_unit(domain, "source_blocked_gap_rows", source_blocked as u64);
    add_unit(domain, "schema_blocked_gap_rows", schema_blocked as u64);
    add_unit(domain, "owned_elsewhere_gap_rows", owned_elsewhere as u64);

    let mut cursor = 44usize;
    let mut seen_gaps = BTreeSet::new();
    let mut counted_source_blocked = 0usize;
    let mut counted_schema_blocked = 0usize;
    let mut counted_owned_elsewhere = 0usize;
    for row_index in 0..row_count {
        let gap_id = read_u32_le(payload, cursor);
        let status = read_u32_le(payload, cursor + 4);
        let evidence_mask = read_u32_le(payload, cursor + 8);
        let current_support = read_u32_le(payload, cursor + 12);
        let current_domain_id = read_u32_le(payload, cursor + 16);
        let proposed_domain_id = read_u32_le(payload, cursor + 20);
        let current_row_len = read_u32_le(payload, cursor + 24);
        let source_ref_id = read_u32_le(payload, cursor + 28);
        cursor += GAP_BADGE_ROW_LEN;

        if gap_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: gap badge row {row_index} has zero gap_id"
            ));
        } else if !seen_gaps.insert(gap_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: gap badge row {row_index} duplicates gap_id {gap_id}"
            ));
        }
        if !matches!(
            status,
            GAP_STATUS_SCHEMA_BLOCKED
                | GAP_STATUS_SOURCE_BLOCKED
                | GAP_STATUS_OWNED_ELSEWHERE
                | GAP_STATUS_BADGED_FAIL_CLOSED
        ) {
            summary.parse_errors.push(format!(
                "offset {offset}: gap badge row {row_index} invalid status {status}"
            ));
        }
        if evidence_mask == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: gap badge row {row_index} has no evidence bits"
            ));
        }
        if !matches!(
            current_support,
            GAP_CURRENT_SUPPORT_NONE
                | GAP_CURRENT_SUPPORT_PARTIAL
                | GAP_CURRENT_SUPPORT_OWNED_ELSEWHERE
        ) {
            summary.parse_errors.push(format!(
                "offset {offset}: gap badge row {row_index} invalid current_support {current_support}"
            ));
        }
        if current_domain_id != 0 && domain_name(current_domain_id as u16) == "unknown" {
            summary.parse_errors.push(format!(
                "offset {offset}: gap badge row {row_index} unknown current_domain_id {current_domain_id}"
            ));
        }
        if domain_name(proposed_domain_id as u16) == "unknown" {
            summary.parse_errors.push(format!(
                "offset {offset}: gap badge row {row_index} unknown proposed_domain_id {proposed_domain_id}"
            ));
        }
        if current_row_len > 512 {
            summary.parse_errors.push(format!(
                "offset {offset}: gap badge row {row_index} implausible current_row_len {current_row_len}"
            ));
        }
        if source_ref_id == 0 || source_ref_id != gap_id {
            summary.parse_errors.push(format!(
                "offset {offset}: gap badge row {row_index} invalid source_ref_id {source_ref_id}"
            ));
        }
        match status {
            GAP_STATUS_SOURCE_BLOCKED => {
                counted_source_blocked = counted_source_blocked.saturating_add(1)
            }
            GAP_STATUS_SCHEMA_BLOCKED => {
                counted_schema_blocked = counted_schema_blocked.saturating_add(1)
            }
            GAP_STATUS_OWNED_ELSEWHERE => {
                counted_owned_elsewhere = counted_owned_elsewhere.saturating_add(1)
            }
            _ => {}
        }
    }
    if counted_source_blocked != source_blocked
        || counted_schema_blocked != schema_blocked
        || counted_owned_elsewhere != owned_elsewhere
    {
        summary.parse_errors.push(format!(
            "offset {offset}: gap badge status counters do not match rows"
        ));
    }
}

fn frame_shape_errors(
    unique_hours: &[i32],
    frame_counts: &BTreeMap<i32, u64>,
    perf_counts: &BTreeMap<i32, BTreeMap<u16, u64>>,
    data_counts: &BTreeMap<i32, BTreeMap<u16, u64>>,
    perf_row_cardinality: &BTreeMap<i32, BTreeMap<u16, (u64, u64)>>,
    data_row_cardinality: &BTreeMap<i32, BTreeMap<u16, (u64, u64)>>,
    expected_domains: &[u16],
) -> Vec<String> {
    let mut errors = Vec::new();
    if frame_counts.is_empty() && perf_counts.is_empty() && data_counts.is_empty() {
        return errors;
    }
    let mut all_hours: BTreeSet<i32> = unique_hours.iter().copied().collect();
    all_hours.extend(perf_counts.keys().copied());
    all_hours.extend(data_counts.keys().copied());
    for hour in &all_hours {
        let frame_count = frame_counts.get(hour).copied().unwrap_or(0);
        if frame_count != 1 {
            errors.push(format!(
                "game_hour_raw {hour}: frame_end_count {frame_count} != 1"
            ));
        }
    }
    for hour in unique_hours {
        for domain_id in expected_domains {
            let perf_count = perf_counts
                .get(hour)
                .and_then(|domains| domains.get(domain_id))
                .copied()
                .unwrap_or(0);
            if perf_count != 1 {
                errors.push(format!(
                    "game_hour_raw {hour}: compact domain {} perf_record_count {perf_count} != 1",
                    domain_name(*domain_id)
                ));
            }
            let data_count = data_counts
                .get(hour)
                .and_then(|domains| domains.get(domain_id))
                .copied()
                .unwrap_or(0);
            if data_count != 1 {
                errors.push(format!(
                    "game_hour_raw {hour}: compact domain {} data_record_count {data_count} != 1",
                    domain_name(*domain_id)
                ));
            }
            if perf_count == 1 && data_count == 1 {
                let perf_cardinality = perf_row_cardinality
                    .get(hour)
                    .and_then(|domains| domains.get(domain_id))
                    .copied();
                let data_cardinality = data_row_cardinality
                    .get(hour)
                    .and_then(|domains| domains.get(domain_id))
                    .copied();
                match (perf_cardinality, data_cardinality) {
                    (Some((perf_rows, perf_changed)), Some((data_rows, data_changed))) => {
                        if perf_rows < data_rows {
                            errors.push(format!(
                                "game_hour_raw {hour}: compact domain {} PERF observed row_count {perf_rows} < DATA emitted row_count {data_rows}",
                                domain_name(*domain_id)
                            ));
                        }
                        if perf_changed != data_rows || perf_changed != data_changed {
                            errors.push(format!(
                                "game_hour_raw {hour}: compact domain {} PERF changed_rows {perf_changed} != DATA emitted/changed rows {data_rows}/{data_changed}",
                                domain_name(*domain_id)
                            ));
                        }
                    }
                    _ => errors.push(format!(
                        "game_hour_raw {hour}: compact domain {} lacks independently decoded PERF/DATA row cardinality",
                        domain_name(*domain_id)
                    )),
                }
            }
        }
    }
    for (hour, domains) in perf_counts {
        if unique_hours.contains(hour) {
            continue;
        }
        for domain_id in expected_domains {
            if domains.get(domain_id).copied().unwrap_or(0) != 0 {
                errors.push(format!(
                    "game_hour_raw {hour}: compact domain {} perf on non-frame hour",
                    domain_name(*domain_id)
                ));
            }
        }
    }
    for (hour, domains) in data_counts {
        if unique_hours.contains(hour) {
            continue;
        }
        for domain_id in expected_domains {
            if domains.get(domain_id).copied().unwrap_or(0) != 0 {
                errors.push(format!(
                    "game_hour_raw {hour}: compact domain {} data on non-frame hour",
                    domain_name(*domain_id)
                ));
            }
        }
    }
    errors
}

fn increment_hour_domain_cardinality(
    counts: &mut BTreeMap<i32, BTreeMap<u16, (u64, u64)>>,
    hour: i32,
    domain_id: u16,
    rows: u64,
    changed_rows: u64,
) {
    let cardinality = counts
        .entry(hour)
        .or_default()
        .entry(domain_id)
        .or_default();
    cardinality.0 = cardinality.0.saturating_add(rows);
    cardinality.1 = cardinality.1.saturating_add(changed_rows);
}

fn increment_hour_domain(
    counts: &mut BTreeMap<i32, BTreeMap<u16, u64>>,
    hour: i32,
    domain_id: u16,
) {
    *counts
        .entry(hour)
        .or_default()
        .entry(domain_id)
        .or_default() += 1;
}

fn domain_entry(summary: &mut CompactStreamSummary, domain_id: u16) -> &mut DomainSummary {
    summary
        .per_domain
        .entry(domain_id)
        .or_insert_with(|| DomainSummary {
            domain_id,
            name: domain_name(domain_id),
            ..DomainSummary::default()
        })
}

fn add_unit(domain: &mut DomainSummary, key: &'static str, value: u64) {
    *domain.data_units_total.entry(key).or_default() = domain
        .data_units_total
        .get(key)
        .copied()
        .unwrap_or(0)
        .saturating_add(value);
}

fn skipped_hours(hours: &[i32]) -> i64 {
    if hours.len() < 2 {
        return 0;
    }
    let mut skipped = 0i64;
    for pair in hours.windows(2) {
        let delta = pair[1].saturating_sub(pair[0]) as i64;
        if delta > 1 {
            skipped = skipped.saturating_add(delta - 1);
        }
    }
    skipped
}

fn skip_compact_string_len(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<usize, String> {
    if cursor.saturating_add(2) > bytes.len() {
        return Err(format!("{label}: missing compact string length"));
    }
    let len = read_u16_le(bytes, *cursor) as usize;
    *cursor += 2;
    if cursor.saturating_add(len) > bytes.len() {
        return Err(format!(
            "{label}: compact string length {len} exceeds payload"
        ));
    }
    *cursor += len;
    Ok(len)
}

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("u16 range"))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("u32 range"))
}

fn read_i32_le(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("i32 range"))
}

fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("u64 range"))
}

fn read_i64_le(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("i64 range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_fuel_payload() {
        let payload = encode_fuel_payload(
            7,
            32,
            &[FuelRow {
                tag_db_id: 47,
                current_raw: 4_046_848,
                max_raw: 14_950_400,
                gain_raw: 245_760,
                cost_raw: -73_728,
            }],
        );
        let mut summary = CompactStreamSummary::default();
        decode_fuel_payload(0, &payload, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let domain = summary.per_domain.get(&DOMAIN_FUEL).unwrap();
        assert_eq!(domain.data_rows_total, 1);
        assert_eq!(
            domain.data_units_total.get("fuel_country_rows").copied(),
            Some(1)
        );
    }

    #[test]
    fn decodes_production_payload() {
        let payload = encode_production_payload(
            9,
            32,
            1,
            &[ProductionStockpileRow {
                tag_db_id: 47,
                variant_id: 70,
                amount_raw: 12_350_000,
            }],
            &[ProductionLineRow {
                tag_db_id: 47,
                kind: 1,
                line_id: 349,
                active_factories: 3,
                cost_raw: 10_500_000,
                speed_raw: 3_105_000,
                produced_raw: 3_440_000,
                amount: -1,
            }],
            &[ProductionLineRow {
                tag_db_id: 47,
                kind: 2,
                line_id: 350,
                active_factories: 2,
                cost_raw: 5_500_000,
                speed_raw: 2_105_000,
                produced_raw: 1_440_000,
                amount: 1,
            }],
            &[ConstructionQueueRow {
                tag_db_id: 47,
                line_id: 9001,
                template_id: Some(20013),
                active_factories: 6,
                cost_raw: 1_080_000_000,
                produced_raw: 12_550_000,
                priority: 0,
            }],
        );
        let mut summary = CompactStreamSummary::default();
        decode_production_payload(0, &payload, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let domain = summary.per_domain.get(&DOMAIN_PRODUCTION).unwrap();
        assert_eq!(domain.data_rows_total, 1);
        assert_eq!(
            domain.data_units_total.get("stockpile_rows").copied(),
            Some(1)
        );
        assert_eq!(
            domain
                .data_units_total
                .get("construction_queue_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn x4_p1_decodes_bijective_pvj1_variant_join() {
        let mut payload = encode_production_payload(
            9,
            0,
            1,
            &[],
            &[ProductionLineRow {
                tag_db_id: 47,
                kind: 1,
                line_id: 349,
                active_factories: 3,
                cost_raw: 1,
                speed_raw: 1,
                produced_raw: 1,
                amount: 0,
            }],
            &[],
            &[],
        );
        payload.extend_from_slice(b"PVJ1");
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&20u16.to_le_bytes());
        payload.extend_from_slice(&36u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&349i32.to_le_bytes());
        payload.extend_from_slice(&7316u32.to_le_bytes());

        let mut summary = CompactStreamSummary::default();
        decode_production_payload(0, &payload, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary.per_domain[&DOMAIN_PRODUCTION]
                .data_units_total
                .get("production_variant_join_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn x4_p2_decodes_bijective_pfr1_requested_factories_join() {
        let mut payload = encode_production_payload(
            9,
            0,
            1,
            &[],
            &[ProductionLineRow {
                tag_db_id: 47,
                kind: 1,
                line_id: 349,
                active_factories: 3,
                cost_raw: 1,
                speed_raw: 1,
                produced_raw: 1,
                amount: 0,
            }],
            &[],
            &[],
        );
        payload.extend_from_slice(b"PFR1");
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&20u16.to_le_bytes());
        payload.extend_from_slice(&36u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&349i32.to_le_bytes());
        payload.extend_from_slice(&5i32.to_le_bytes());

        let mut summary = CompactStreamSummary::default();
        decode_production_payload(0, &payload, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary.per_domain[&DOMAIN_PRODUCTION]
                .data_units_total
                .get("production_requested_factories_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn decodes_bijective_ppr1_priority_order_join() {
        let mut payload = encode_production_payload(
            9,
            0,
            1,
            &[],
            &[ProductionLineRow {
                tag_db_id: 47,
                kind: 1,
                line_id: 349,
                active_factories: 3,
                cost_raw: 1,
                speed_raw: 1,
                produced_raw: 1,
                amount: 0,
            }],
            &[],
            &[],
        );
        payload.extend_from_slice(b"PPR1");
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&20u16.to_le_bytes());
        payload.extend_from_slice(&36u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&349i32.to_le_bytes());
        payload.extend_from_slice(&0i32.to_le_bytes());

        let mut summary = CompactStreamSummary::default();
        decode_production_payload(0, &payload, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary.per_domain[&DOMAIN_PRODUCTION]
                .data_units_total
                .get("production_priority_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn x4_p3_decodes_bijective_pls1_line_subtypes() {
        let mut payload = encode_production_payload(
            9,
            0,
            1,
            &[],
            &[ProductionLineRow {
                tag_db_id: 47,
                kind: 1,
                line_id: 349,
                active_factories: 3,
                cost_raw: 1,
                speed_raw: 1,
                produced_raw: 1,
                amount: 0,
            }],
            &[ProductionLineRow {
                tag_db_id: 47,
                kind: 2,
                line_id: 350,
                active_factories: 2,
                cost_raw: 1,
                speed_raw: 1,
                produced_raw: 1,
                amount: 0,
            }],
            &[],
        );
        payload.extend_from_slice(b"PLS1");
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&20u16.to_le_bytes());
        payload.extend_from_slice(&52u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&349i32.to_le_bytes());
        payload.extend_from_slice(&4u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&350i32.to_le_bytes());
        payload.extend_from_slice(&3u32.to_le_bytes());

        let mut summary = CompactStreamSummary::default();
        decode_production_payload(0, &payload, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let units = &summary.per_domain[&DOMAIN_PRODUCTION].data_units_total;
        assert_eq!(units.get("production_line_subtype_rows").copied(), Some(2));
        assert_eq!(
            units
                .get("production_line_subtype_ship_refit_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            units
                .get("production_line_subtype_railway_gun_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn x4_p6_decodes_tag_bearing_proven_empty_and_unreadable_vectors() {
        let mut payload = encode_production_payload(9, 0, 2, &[], &[], &[], &[]);
        payload.extend_from_slice(b"PCP1");
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&20u16.to_le_bytes());
        payload.extend_from_slice(&36u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&0b111u32.to_le_bytes());
        payload.extend_from_slice(&48u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());

        let mut summary = CompactStreamSummary::default();
        decode_production_payload(0, &payload, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let units = &summary.per_domain[&DOMAIN_PRODUCTION].data_units_total;
        assert_eq!(units.get("production_completeness_rows").copied(), Some(2));
        assert_eq!(units.get("stockpile_complete_rows").copied(), Some(1));
        assert_eq!(units.get("military_lines_complete_rows").copied(), Some(1));
        assert_eq!(units.get("naval_lines_complete_rows").copied(), Some(1));
    }

    #[test]
    fn x4_p6_rejects_non_bijective_or_unknown_pcp1_rows() {
        let mut payload = encode_production_payload(9, 0, 2, &[], &[], &[], &[]);
        payload.extend_from_slice(b"PCP1");
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&20u16.to_le_bytes());
        payload.extend_from_slice(&28u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&8u32.to_le_bytes());
        let mut summary = CompactStreamSummary::default();
        decode_production_payload(0, &payload, &mut summary);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("PCP1 header/count/length is invalid")));

        let mut unknown = encode_production_payload(9, 0, 1, &[], &[], &[], &[]);
        unknown.extend_from_slice(b"PCP1");
        unknown.extend_from_slice(&1u16.to_le_bytes());
        unknown.extend_from_slice(&20u16.to_le_bytes());
        unknown.extend_from_slice(&28u32.to_le_bytes());
        unknown.extend_from_slice(&1u32.to_le_bytes());
        unknown.extend_from_slice(&0u32.to_le_bytes());
        unknown.extend_from_slice(&47u32.to_le_bytes());
        unknown.extend_from_slice(&8u32.to_le_bytes());
        let mut summary = CompactStreamSummary::default();
        decode_production_payload(0, &unknown, &mut summary);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("PCP1 row 0 has unknown flags")));

        let mut incoherent = encode_production_payload(
            9,
            0,
            1,
            &[ProductionStockpileRow {
                tag_db_id: 47,
                variant_id: 70,
                amount_raw: 1,
            }],
            &[],
            &[],
            &[],
        );
        incoherent.extend_from_slice(b"PCP1");
        incoherent.extend_from_slice(&1u16.to_le_bytes());
        incoherent.extend_from_slice(&20u16.to_le_bytes());
        incoherent.extend_from_slice(&28u32.to_le_bytes());
        incoherent.extend_from_slice(&1u32.to_le_bytes());
        incoherent.extend_from_slice(&0u32.to_le_bytes());
        incoherent.extend_from_slice(&47u32.to_le_bytes());
        incoherent.extend_from_slice(&0u32.to_le_bytes());
        let mut summary = CompactStreamSummary::default();
        decode_production_payload(0, &incoherent, &mut summary);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("marks stockpile unreadable")));
    }

    #[test]
    fn x4_p4_appends_and_decodes_bijective_pef1_producer_outputs() {
        let military = [ProductionLineRow {
            tag_db_id: 47,
            kind: 1,
            line_id: 349,
            active_factories: 3,
            cost_raw: 1,
            speed_raw: 1,
            produced_raw: 1,
            amount: 0,
        }];
        let mut payload = encode_production_payload(9, 0, 1, &[], &military, &[], &[]);
        append_production_efficiency_trailer(
            &mut payload,
            &military,
            &[],
            &[ProductionEfficiencyRow {
                tag_db_id: 47,
                kind: 1,
                line_id: 349,
                efficiency_raw: 6_459_667,
                efficiency_cap_raw: PRODUCTION_EFFICIENCY_FULL_SCALE_RAW,
                efficiency_growth_raw: 12_345,
            }],
        )
        .unwrap();

        let mut summary = CompactStreamSummary::default();
        decode_production_payload(0, &payload, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary.per_domain[&DOMAIN_PRODUCTION]
                .data_units_total
                .get("production_efficiency_producer_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn x4_p4_refuses_non_bijective_efficiency_carrier() {
        let military = [ProductionLineRow {
            tag_db_id: 47,
            kind: 1,
            line_id: 349,
            active_factories: 3,
            cost_raw: 1,
            speed_raw: 1,
            produced_raw: 1,
            amount: 0,
        }];
        let mut payload = encode_production_payload(9, 0, 1, &[], &military, &[], &[]);
        let base_len = payload.len();
        let error =
            append_production_efficiency_trailer(&mut payload, &military, &[], &[]).unwrap_err();
        assert!(error.contains("not bijective"), "{error}");
        assert_eq!(payload.len(), base_len);
    }

    #[test]
    fn x4_p5_appends_and_decodes_assigned_and_proven_null_mio_joins() {
        let military = [ProductionLineRow {
            tag_db_id: 47,
            kind: 1,
            line_id: 1669,
            active_factories: 3,
            cost_raw: 1,
            speed_raw: 1,
            produced_raw: 1,
            amount: 0,
        }];
        let naval = [ProductionLineRow {
            tag_db_id: 47,
            kind: 2,
            line_id: 1672,
            active_factories: 2,
            cost_raw: 1,
            speed_raw: 1,
            produced_raw: 1,
            amount: 0,
        }];
        let mut payload = encode_production_payload(9, 0, 1, &[], &military, &naval, &[]);
        append_production_mio_join_trailer(
            &mut payload,
            &military,
            &naval,
            &[
                ProductionMioJoinRow {
                    tag_db_id: 47,
                    kind: 1,
                    line_id: 1669,
                    mio_id: 33,
                },
                ProductionMioJoinRow {
                    tag_db_id: 47,
                    kind: 2,
                    line_id: 1672,
                    mio_id: 0,
                },
            ],
        )
        .unwrap();

        let mut summary = CompactStreamSummary::default();
        decode_production_payload(0, &payload, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let units = &summary.per_domain[&DOMAIN_PRODUCTION].data_units_total;
        assert_eq!(units.get("production_mio_join_rows").copied(), Some(2));
        assert_eq!(units.get("production_mio_assigned_rows").copied(), Some(1));
        assert_eq!(
            units.get("production_mio_proven_null_rows").copied(),
            Some(1)
        );
    }

    #[test]
    fn x4_p5_refuses_non_bijective_or_duplicate_mio_carrier() {
        let military = [ProductionLineRow {
            tag_db_id: 47,
            kind: 1,
            line_id: 1669,
            active_factories: 3,
            cost_raw: 1,
            speed_raw: 1,
            produced_raw: 1,
            amount: 0,
        }];
        let mut payload = encode_production_payload(9, 0, 1, &[], &military, &[], &[]);
        let base_len = payload.len();
        let error =
            append_production_mio_join_trailer(&mut payload, &military, &[], &[]).unwrap_err();
        assert!(error.contains("not bijective"), "{error}");
        assert_eq!(payload.len(), base_len);

        let duplicate = ProductionMioJoinRow {
            tag_db_id: 47,
            kind: 1,
            line_id: 1669,
            mio_id: 33,
        };
        let error = append_production_mio_join_trailer(
            &mut payload,
            &military,
            &[],
            &[duplicate, duplicate],
        )
        .unwrap_err();
        assert!(error.contains("not unique"), "{error}");
        assert_eq!(payload.len(), base_len);
    }

    #[test]
    fn rejects_duplicate_stockpile_keys() {
        let payload = encode_production_payload(
            9,
            32,
            1,
            &[
                ProductionStockpileRow {
                    tag_db_id: 47,
                    variant_id: 70,
                    amount_raw: 1,
                },
                ProductionStockpileRow {
                    tag_db_id: 47,
                    variant_id: 70,
                    amount_raw: 2,
                },
            ],
            &[],
            &[],
            &[],
        );
        let mut summary = CompactStreamSummary::default();
        decode_production_payload(10, &payload, &mut summary);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("duplicates tag_db_id 47 variant_id 70")));
    }

    #[test]
    fn gates_two_domain_stream() {
        let mut raw = Vec::new();
        append_compact_record(
            &mut raw,
            1,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(10, 1, 1),
        );
        append_compact_record(
            &mut raw,
            2,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_fuel_payload(
                10,
                0,
                &[FuelRow {
                    tag_db_id: 47,
                    current_raw: 1,
                    max_raw: 2,
                    gain_raw: 0,
                    cost_raw: 0,
                }],
            ),
        );
        append_compact_record(
            &mut raw,
            3,
            100,
            DOMAIN_PRODUCTION,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(11, 1, 1),
        );
        append_compact_record(
            &mut raw,
            4,
            100,
            DOMAIN_PRODUCTION,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_production_payload(10, 0, 1, &[], &[], &[], &[]),
        );
        append_compact_record(
            &mut raw,
            5,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(2),
        );
        let gate = gate_compact_stream(&raw, &[DOMAIN_FUEL, DOMAIN_PRODUCTION]);
        assert!(gate.ok, "{:?}", gate.summary.coverage_errors);
    }

    #[test]
    fn stream_gate_rejects_every_canonical_failure_from_each_status_origin() {
        let fatal_bits = [
            compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_PROFILE_MISMATCH,
            compact_contract::COMPACT_STATUS_NO_CLOCK,
            compact_contract::COMPACT_STATUS_NO_SCALAR_SOURCE,
            compact_contract::COMPACT_STATUS_WORKER_PANIC,
            compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_PARTIAL_DATA,
            compact_contract::COMPACT_STATUS_CONTROL_INVALID,
        ];
        assert_eq!(
            fatal_bits.iter().copied().fold(0, |mask, bit| mask | bit),
            compact_contract::COMPACT_STATUS_FAILURE_MASK
        );

        for bit in fatal_bits {
            for (data_status, perf_status, frame_status) in [(bit, 0, 0), (0, bit, 0), (0, 0, bit)]
            {
                let gate = gate_compact_stream(
                    &fuel_stream_with_statuses(data_status, perf_status, frame_status),
                    &[DOMAIN_FUEL],
                );
                assert!(
                    !gate.ok,
                    "bit={bit:#x}, errors={:?}",
                    gate.summary.parse_errors
                );
                assert_ne!(gate.summary.failure_status_flags_or & bit, 0);
                assert!(gate
                    .summary
                    .parse_errors
                    .iter()
                    .any(|error| error.contains("fatal status flags")));
            }
        }
    }

    #[test]
    fn stream_gate_accepts_aligned_canonical_informational_status_authority() {
        let informational_bits = [
            compact_contract::COMPACT_STATUS_WAR_BRIDGE_APPLIED,
            compact_contract::COMPACT_STATUS_EXTRACTION_PERF_GATE,
            compact_contract::COMPACT_STATUS_DELTA_DATA,
            compact_contract::COMPACT_STATUS_TRANSITION_BASELINE_RESET,
            compact_contract::COMPACT_STATUS_WAR_SEGMENT_ABSENT,
        ];
        assert_eq!(
            informational_bits.iter().copied().fold(
                compact_contract::COMPACT_STATUS_FAILURE_MASK,
                |mask, bit| mask | bit
            ),
            compact_contract::COMPACT_STATUS_V1_KNOWN_MASK
        );

        for bit in informational_bits {
            let gate =
                gate_compact_stream(&fuel_stream_with_statuses(bit, bit, bit), &[DOMAIN_FUEL]);
            assert!(
                gate.ok,
                "bit={bit:#x}, errors={:?}",
                gate.summary.parse_errors
            );
            assert_eq!(gate.summary.failure_status_flags_or, 0);
            assert_ne!(gate.summary.status_flags_or & bit, 0);
        }
    }

    #[test]
    fn stream_gate_rejects_known_informational_status_authority_mismatches() {
        let bit = compact_contract::COMPACT_STATUS_DELTA_DATA;
        for (data_status, perf_status, frame_status) in
            [(bit, 0, 0), (0, bit, bit), (bit, bit, 0), (0, 0, bit)]
        {
            let gate = gate_compact_stream(
                &fuel_stream_with_statuses(data_status, perf_status, frame_status),
                &[DOMAIN_FUEL],
            );
            assert!(
                !gate.ok,
                "statuses=({data_status:#x}, {perf_status:#x}, {frame_status:#x})"
            );
            assert!(gate.summary.parse_errors.iter().any(|error| {
                error.contains("preceding same-hour DOMAIN_PERF status")
                    || error.contains("OR of independently observed owned DOMAIN_PERF statuses")
            }));
        }
    }

    #[test]
    fn stream_gate_rejects_unknown_status_bits_from_each_origin() {
        let unknown = 1u32 << 31;
        for (data_status, perf_status, frame_status) in
            [(unknown, 0, 0), (0, unknown, 0), (0, 0, unknown)]
        {
            let gate = gate_compact_stream(
                &fuel_stream_with_statuses(data_status, perf_status, frame_status),
                &[DOMAIN_FUEL],
            );
            assert!(!gate.ok, "errors={:?}", gate.summary.parse_errors);
            assert!(gate
                .summary
                .parse_errors
                .iter()
                .any(|error| error.contains("unknown canonical bits")));
        }
    }

    #[test]
    fn stream_gate_binds_frame_authority_to_observed_perf_and_reserved_zero() {
        for frame_payload in [
            frame_payload_with_authority(2, 2, 0, 0),
            frame_payload_with_authority(1, 2, 0, 0),
            frame_payload_with_authority(1, 1, 0, 7),
        ] {
            let mut raw = Vec::new();
            append_compact_record(
                &mut raw,
                1,
                100,
                DOMAIN_FUEL,
                RECORD_TYPE_DOMAIN_PERF,
                &perf_payload_with_status(0, 0, 0),
            );
            append_compact_record(
                &mut raw,
                2,
                100,
                DOMAIN_FUEL,
                RECORD_TYPE_DOMAIN_DATA,
                &encode_fuel_payload(0, 0, &[]),
            );
            append_compact_record(
                &mut raw,
                3,
                100,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &frame_payload,
            );
            let gate = gate_compact_stream(&raw, &[DOMAIN_FUEL]);
            assert!(!gate.ok, "errors={:?}", gate.summary.parse_errors);
        }
    }

    #[test]
    fn stream_gate_rejects_unassigned_perf_as_frame_authority() {
        let mut raw = Vec::new();
        append_compact_record(
            &mut raw,
            1,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(0, 0, 0),
        );
        append_compact_record(
            &mut raw,
            2,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_fuel_payload(0, 0, &[]),
        );
        append_compact_record(
            &mut raw,
            3,
            100,
            u16::MAX,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(0, 0, 0),
        );
        append_compact_record(
            &mut raw,
            4,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload_with_authority(2, 2, 0, 0),
        );

        let gate = gate_compact_stream(&raw, &[DOMAIN_FUEL]);
        assert!(!gate.ok);
        assert!(gate.summary.parse_errors.iter().any(|error| {
            error.contains("unassigned domain 65535 DOMAIN_PERF")
                || error.contains("owned DOMAIN_PERF roster count 1")
        }));
    }

    #[test]
    fn stream_gate_cross_checks_perf_and_data_row_cardinality() {
        let mut raw = Vec::new();
        append_compact_record(
            &mut raw,
            1,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(0, 1, 1),
        );
        append_compact_record(
            &mut raw,
            2,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_fuel_payload(0, 0, &[]),
        );
        append_compact_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(1),
        );
        let gate = gate_compact_stream(&raw, &[DOMAIN_FUEL]);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .coverage_errors
            .iter()
            .any(|error| error.contains("PERF changed_rows 1 != DATA emitted/changed rows 0/0")));
    }

    #[test]
    fn stream_gate_accepts_perf_observed_work_above_emitted_data_rows() {
        let row = FuelRow {
            tag_db_id: 47,
            current_raw: 1,
            max_raw: 2,
            gain_raw: 0,
            cost_raw: 0,
        };
        let mut raw = Vec::new();
        append_compact_record(
            &mut raw,
            1,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_full(0, 2, 1, 1, 1, 0, 0),
        );
        append_compact_record(
            &mut raw,
            2,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_fuel_payload(0, 0, &[row]),
        );
        append_compact_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(1),
        );
        let gate = gate_compact_stream(&raw, &[DOMAIN_FUEL]);
        assert!(gate.ok, "{:?}", gate.summary.coverage_errors);
    }

    #[test]
    fn stream_gate_excludes_instrumentation_probe_from_frame_cardinality() {
        let mut raw = Vec::new();
        append_compact_record(
            &mut raw,
            1,
            i32::MIN,
            RC_PROBE_DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(0, 0, 0),
        );
        append_compact_record(
            &mut raw,
            2,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(0, 0, 0),
        );
        append_compact_record(
            &mut raw,
            3,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_fuel_payload(0, 0, &[]),
        );
        append_compact_record(
            &mut raw,
            4,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(1),
        );
        let gate = gate_compact_stream(&raw, &[DOMAIN_FUEL]);
        assert!(gate.ok, "{:?}", gate.summary.parse_errors);
        assert_eq!(gate.summary.record_count, 3);
        assert!(!gate.summary.per_domain.contains_key(&RC_PROBE_DOMAIN_ID));
    }

    #[test]
    fn stream_gate_rejects_impossible_perf_cardinality_and_reserved_word() {
        for perf in [
            perf_payload_full(0, 0, 1, 0, 0, 0, 0),
            perf_payload_full(0, 0, 0, 0, 1, 0, 0),
            perf_payload_full(0, 0, 0, 0, 0, 0, 9),
        ] {
            let mut raw = Vec::new();
            append_compact_record(
                &mut raw,
                1,
                100,
                DOMAIN_FUEL,
                RECORD_TYPE_DOMAIN_PERF,
                &perf,
            );
            append_compact_record(
                &mut raw,
                2,
                100,
                DOMAIN_FUEL,
                RECORD_TYPE_DOMAIN_DATA,
                &encode_fuel_payload(0, 0, &[]),
            );
            append_compact_record(
                &mut raw,
                3,
                100,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &frame_payload(1),
            );
            assert!(!gate_compact_stream(&raw, &[DOMAIN_FUEL]).ok);
        }
    }

    #[test]
    fn stream_gate_rejects_duplicate_perf_and_data_records_independently() {
        let perf = perf_payload_with_status(0, 0, 0);
        let data = encode_fuel_payload(0, 0, &[]);
        for duplicate_perf in [true, false] {
            let mut raw = Vec::new();
            let mut sequence = 1;
            append_compact_record(
                &mut raw,
                sequence,
                100,
                DOMAIN_FUEL,
                RECORD_TYPE_DOMAIN_PERF,
                &perf,
            );
            sequence += 1;
            if duplicate_perf {
                append_compact_record(
                    &mut raw,
                    sequence,
                    100,
                    DOMAIN_FUEL,
                    RECORD_TYPE_DOMAIN_PERF,
                    &perf,
                );
                sequence += 1;
            }
            append_compact_record(
                &mut raw,
                sequence,
                100,
                DOMAIN_FUEL,
                RECORD_TYPE_DOMAIN_DATA,
                &data,
            );
            sequence += 1;
            if !duplicate_perf {
                append_compact_record(
                    &mut raw,
                    sequence,
                    100,
                    DOMAIN_FUEL,
                    RECORD_TYPE_DOMAIN_DATA,
                    &data,
                );
                sequence += 1;
            }
            append_compact_record(
                &mut raw,
                sequence,
                100,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &frame_payload(if duplicate_perf { 2 } else { 1 }),
            );
            let gate = gate_compact_stream(&raw, &[DOMAIN_FUEL]);
            assert!(!gate.ok);
            assert!(gate.summary.coverage_errors.iter().any(|error| {
                error.contains(if duplicate_perf {
                    "perf_record_count 2 != 1"
                } else {
                    "data_record_count 2 != 1"
                })
            }));
        }
    }

    #[test]
    fn stream_gate_enforces_canonical_record_flag_placement() {
        let mut legal = valid_fuel_stream();
        replace_record_payload(&mut legal, 0, &perf_payload_with_empty_diag());
        legal[28..32].copy_from_slice(
            &compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER.to_le_bytes(),
        );
        assert!(gate_compact_stream(&legal, &[DOMAIN_FUEL]).ok);

        let mut illegal = valid_fuel_stream();
        let data_offset =
            COMPACT_RECORD_HEADER_LEN + compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN;
        illegal[data_offset + 28..data_offset + 32].copy_from_slice(
            &compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER.to_le_bytes(),
        );
        let gate = gate_compact_stream(&illegal, &[DOMAIN_FUEL]);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .parse_errors
            .iter()
            .any(|error| error.contains("contain rejected bits")));
    }

    #[test]
    fn stream_gate_rejects_missing_gapped_and_wrapping_sequences() {
        let mut missing = valid_fuel_stream();
        missing[8..16].copy_from_slice(&2u64.to_le_bytes());
        assert!(summarize_compact_stream(&missing)
            .ordering_errors
            .iter()
            .any(|error| error.contains("does not start at 1")));

        let mut gap = valid_fuel_stream();
        let data_offset =
            COMPACT_RECORD_HEADER_LEN + compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN;
        gap[data_offset + 8..data_offset + 16].copy_from_slice(&3u64.to_le_bytes());
        assert!(summarize_compact_stream(&gap)
            .ordering_errors
            .iter()
            .any(|error| error.contains("sequence discontinuity 1 -> 3")));

        let mut wrapping = valid_fuel_stream();
        wrapping[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        wrapping[data_offset + 8..data_offset + 16].copy_from_slice(&0u64.to_le_bytes());
        assert!(summarize_compact_stream(&wrapping)
            .ordering_errors
            .iter()
            .any(|error| error.contains("wrapping is not allowed")));
    }

    #[test]
    fn stream_gate_rejects_missing_and_unannounced_perf_trailers_and_bad_frame_size() {
        let mut long_perf = Vec::new();
        append_compact_record(
            &mut long_perf,
            1,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_PERF,
            &[0; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN + 1],
        );
        append_compact_record(
            &mut long_perf,
            2,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_fuel_payload(0, 32, &[]),
        );
        append_compact_record(
            &mut long_perf,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(1),
        );
        assert!(summarize_compact_stream(&long_perf)
            .parse_errors
            .iter()
            .any(|error| error.contains("unannounced trailer byte")));

        let mut announced_without_trailer = valid_fuel_stream();
        announced_without_trailer[28..32].copy_from_slice(
            &compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER.to_le_bytes(),
        );
        assert!(summarize_compact_stream(&announced_without_trailer)
            .parse_errors
            .iter()
            .any(|error| error.contains("diagnostic trailer header is truncated")));

        let mut short_frame = valid_fuel_stream();
        short_frame.truncate(short_frame.len() - 1);
        let frame_offset = short_frame.len()
            - (compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN - 1)
            - COMPACT_RECORD_HEADER_LEN;
        short_frame[frame_offset + 24..frame_offset + 28].copy_from_slice(
            &((compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN - 1) as u32).to_le_bytes(),
        );
        assert!(summarize_compact_stream(&short_frame)
            .parse_errors
            .iter()
            .any(|error| error.contains("FRAME_END payload length 31 is not 32")));
    }

    #[test]
    fn stream_gate_rejects_cross_hour_data_and_records_after_closed_hour() {
        let mut mismatched = valid_fuel_stream();
        let data_offset =
            COMPACT_RECORD_HEADER_LEN + compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN;
        mismatched[data_offset + 16..data_offset + 20].copy_from_slice(&101i32.to_le_bytes());
        assert!(summarize_compact_stream(&mismatched)
            .parse_errors
            .iter()
            .any(|error| error.contains("conflicts with pending PERF hour 100")));

        let mut trailing = valid_fuel_stream();
        append_compact_record(
            &mut trailing,
            4,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0, 0, 0),
        );
        let summary = summarize_compact_stream(&trailing);
        assert!(summary.parse_errors.iter().any(|error| {
            error.contains("follows closed frame hour 100")
                || error.contains("final canonical FRAME_END")
        }));
    }

    #[test]
    fn stream_gate_rejects_lane_owned_data_before_same_hour_perf() {
        let mut raw = Vec::new();
        append_compact_record(
            &mut raw,
            1,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_fuel_payload(0, 0, &[]),
        );
        append_compact_record(
            &mut raw,
            2,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(0, 0, 0),
        );
        append_compact_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(1),
        );

        let gate = gate_compact_stream(&raw, &[DOMAIN_FUEL]);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .parse_errors
            .iter()
            .any(|error| error.contains("has no preceding same-hour DOMAIN_PERF authority")));
    }

    #[test]
    fn stream_gate_fails_closed_on_malformed_domain_data() {
        let mut raw = Vec::new();
        append_compact_record(
            &mut raw,
            1,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0, 0, 0),
        );
        append_compact_record(&mut raw, 2, 100, DOMAIN_FUEL, RECORD_TYPE_DOMAIN_DATA, &[]);
        append_compact_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(1),
        );
        let gate = gate_compact_stream(&raw, &[DOMAIN_FUEL]);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .parse_errors
            .iter()
            .any(|error| error.contains("fuel payload shorter than 24 bytes")));
    }

    #[test]
    fn decodes_stockpile_variant_defs_payload() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&5u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&32u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1375u32.to_le_bytes());
        payload.extend_from_slice(
            &(VARIANT_DEF_FLAG_DISPLAY_NAME | VARIANT_DEF_FLAG_MODULES).to_le_bytes(),
        );
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        append_string(&mut payload, "Do 17");
        append_string(&mut payload, "");
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        append_string(&mut payload, "fixed_main_weapon_slot");
        let mut summary = CompactStreamSummary::default();
        decode_variant_defs_payload(0, &payload, DOMAIN_STOCKPILE_VARIANT_DEFS, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn remaining_gap_badges_cover_open_backlog() {
        let gaps = remaining_gap_badges();
        let names = gaps.iter().map(|gap| gap.name).collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "consumer_goods",
                "resource_flows_shortages",
                "economy_law_derived_state",
                "trade_factory_impact",
            ]
        );
        assert!(gaps
            .iter()
            .all(|gap| gap.evidence_mask != 0 && gap.proposed_domain_id != 0));
        assert!(gaps
            .iter()
            .all(|gap| gap.status != GAP_STATUS_SCHEMA_BLOCKED));
        assert!(gaps
            .iter()
            .any(|gap| gap.status == GAP_STATUS_SOURCE_BLOCKED));
        assert!(gaps
            .iter()
            .any(|gap| gap.status == GAP_STATUS_OWNED_ELSEWHERE));
    }

    #[test]
    fn decodes_gap_badge_payload() {
        let gaps = remaining_gap_badges();
        let rows = gap_badge_rows(&gaps);
        let payload = encode_gap_badges_payload(13, 32, &rows);
        let mut summary = CompactStreamSummary::default();
        decode_gap_badges_payload(0, &payload, DOMAIN_GAP_BADGES, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let domain = summary.per_domain.get(&DOMAIN_GAP_BADGES).unwrap();
        assert_eq!(domain.data_rows_total, 4);
        assert_eq!(
            domain
                .data_units_total
                .get("remaining_gap_badge_rows")
                .copied(),
            Some(4)
        );
        assert_eq!(
            domain
                .data_units_total
                .get("source_blocked_gap_rows")
                .copied(),
            Some(2)
        );
    }

    #[test]
    fn rejects_duplicate_gap_badge_keys() {
        let mut rows = gap_badge_rows(&remaining_gap_badges());
        rows[1].gap_id = rows[0].gap_id;
        rows[1].source_ref_id = rows[0].source_ref_id;
        let payload = encode_gap_badges_payload(13, 32, &rows);
        let mut summary = CompactStreamSummary::default();
        decode_gap_badges_payload(0, &payload, DOMAIN_GAP_BADGES, &mut summary);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("duplicates gap_id")));
    }

    fn perf_payload(read_time_us: u64, rows: u32, units: u32) -> Vec<u8> {
        perf_payload_full(read_time_us, rows, rows, units, units, 0, 0)
    }

    fn perf_payload_with_status(read_time_us: u64, rows: u32, status_flags: u32) -> Vec<u8> {
        perf_payload_full(read_time_us, rows, rows, rows, rows, status_flags, 0)
    }

    fn perf_payload_full(
        read_time_us: u64,
        rows: u32,
        changed_rows: u32,
        units: u32,
        changed_units: u32,
        status_flags: u32,
        reserved: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&read_time_us.to_le_bytes());
        payload.extend_from_slice(&rows.to_le_bytes());
        payload.extend_from_slice(&changed_rows.to_le_bytes());
        payload.extend_from_slice(&units.to_le_bytes());
        payload.extend_from_slice(&changed_units.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&reserved.to_le_bytes());
        payload
    }

    fn frame_payload(domains: u32) -> Vec<u8> {
        frame_payload_with_authority(domains, domains, 0, 0)
    }

    fn frame_payload_with_authority(
        emitted_domains: u32,
        scheduled_domains: u32,
        status_flags: u32,
        reserved: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&100u64.to_le_bytes());
        payload.extend_from_slice(&emitted_domains.to_le_bytes());
        payload.extend_from_slice(&scheduled_domains.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&reserved.to_le_bytes());
        payload
    }

    fn fuel_stream_with_statuses(data_status: u32, perf_status: u32, frame_status: u32) -> Vec<u8> {
        let mut raw = Vec::new();
        append_compact_record(
            &mut raw,
            1,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(0, 0, perf_status),
        );
        append_compact_record(
            &mut raw,
            2,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_fuel_payload(0, data_status, &[]),
        );
        append_compact_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload_with_authority(1, 1, frame_status, 0),
        );
        raw
    }

    fn perf_payload_with_empty_diag() -> Vec<u8> {
        let mut payload = perf_payload(0, 0, 0);
        payload.extend_from_slice(compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC);
        payload.extend_from_slice(
            &compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes(),
        );
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload
    }

    fn valid_fuel_stream() -> Vec<u8> {
        let mut raw = Vec::new();
        append_compact_record(
            &mut raw,
            1,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0, 0, 0),
        );
        append_compact_record(
            &mut raw,
            2,
            100,
            DOMAIN_FUEL,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_fuel_payload(0, 0, &[]),
        );
        append_compact_record(
            &mut raw,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(1),
        );
        raw
    }

    fn replace_record_payload(raw: &mut Vec<u8>, record_offset: usize, payload: &[u8]) {
        let header = compact_contract::parse_compact_record_header(&raw[record_offset..])
            .expect("test stream header");
        let payload_start = record_offset + COMPACT_RECORD_HEADER_LEN;
        let payload_end = payload_start + header.payload_len;
        raw.splice(payload_start..payload_end, payload.iter().copied());
        raw[record_offset + 24..record_offset + 28]
            .copy_from_slice(&(payload.len() as u32).to_le_bytes());
    }

    fn append_string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u16).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }
}
