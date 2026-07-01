use std::collections::{BTreeMap, BTreeSet};

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

pub const PROPOSED_DOMAIN_PRODUCTION_LINE_DEMAND: u16 = 612;
pub const PROPOSED_DOMAIN_COUNTRY_RESOURCE_FLOWS: u16 = 613;
pub const PROPOSED_DOMAIN_DEPLOYMENT_QUEUES: u16 = 614;
pub const PROPOSED_DOMAIN_COUNTRY_ECONOMY_EFFECTS: u16 = 615;
pub const PROPOSED_DOMAIN_MIO_DESIGN_JOINS: u16 = 616;

pub const STATUS_WORKER_PANIC: u32 = 1 << 6;
pub const STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const STATUS_DELTA_DATA: u32 = 1 << 10;

pub const FUEL_SCALE_RAW: u32 = 32768;
pub const PRODUCTION_SCALE_RAW: u32 = 100000;
pub const FUEL_ROW_LEN: usize = 40;
pub const PRODUCTION_STOCKPILE_ROW_LEN: usize = 16;
pub const PRODUCTION_LINE_ROW_LEN: usize = 32;
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

const MAX_TAG_DB_ID: u32 = 2_048;
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
    pub ordered: bool,
    pub parse_errors: Vec<String>,
    pub ordering_errors: Vec<String>,
    pub coverage_errors: Vec<String>,
    pub frame_hour_counts: BTreeMap<i32, u64>,
    pub per_hour_perf_domain_counts: BTreeMap<i32, BTreeMap<u16, u64>>,
    pub per_hour_data_domain_counts: BTreeMap<i32, BTreeMap<u16, u64>>,
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

pub fn remaining_gap_badges() -> Vec<RemainingGapBadge> {
    vec![
        RemainingGapBadge {
            gap_id: 1,
            name: "requested_factory_counts",
            status: GAP_STATUS_SCHEMA_BLOCKED,
            status_label: "schema_blocked",
            evidence_mask: GAP_EVIDENCE_SAVE_ORACLE
                | GAP_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | GAP_EVIDENCE_CURRENT_COLLECTOR_ROOTS,
            current_support: GAP_CURRENT_SUPPORT_NONE,
            current_compact_evidence: "domain 11 production line rows are 32 bytes: tag_db_id, kind, line_id, active_factories, cost, speed, produced, amount",
            source_evidence: "save_oracle/parse_production.py reads countries.<TAG>.production.*_lines.requested_factories",
            fail_closed_badge: "requested_factories omitted from replay until a live CEquipmentProductionLine requested-factory offset is validated",
            proposed_domain_id: PROPOSED_DOMAIN_PRODUCTION_LINE_DEMAND,
            proposed_domain_name: domain_name(PROPOSED_DOMAIN_PRODUCTION_LINE_DEMAND),
            owner_boundary: "lane-owned production-line demand; joins by (tag_db_id, kind, line_id)",
        },
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
            gap_id: 4,
            name: "deployment_queues",
            status: GAP_STATUS_SOURCE_BLOCKED,
            status_label: "source_blocked",
            evidence_mask: GAP_EVIDENCE_SAVE_ORACLE
                | GAP_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | GAP_EVIDENCE_NO_HOT_LOOP_ROOT,
            current_support: GAP_CURRENT_SUPPORT_NONE,
            current_compact_evidence: "domain 11 construction_queue rows are civil construction only and do not encode military_deployment_conveyor rows",
            source_evidence: "save_oracle/parse_deployment.py reads countries.<TAG>.deployment.military_deployment_conveyor[] and equipment-in-transit",
            fail_closed_badge: "deployment conveyor omitted until conveyor root/vector/type offsets are validated live",
            proposed_domain_id: PROPOSED_DOMAIN_DEPLOYMENT_QUEUES,
            proposed_domain_name: domain_name(PROPOSED_DOMAIN_DEPLOYMENT_QUEUES),
            owner_boundary: "lane-owned deployment queue state; deployed unit/oob state remains unit-lane owned",
        },
        RemainingGapBadge {
            gap_id: 5,
            name: "mio_design_variant_production_joins",
            status: GAP_STATUS_SCHEMA_BLOCKED,
            status_label: "schema_blocked",
            evidence_mask: GAP_EVIDENCE_SAVE_ORACLE
                | GAP_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | GAP_EVIDENCE_CURRENT_COLLECTOR_ROOTS,
            current_support: GAP_CURRENT_SUPPORT_PARTIAL,
            current_compact_evidence: "stockpile variant domains carry variant ids, names, modules, and upgrades; production line rows do not carry equipment_variant_index or industrial_organisation ids",
            source_evidence: "save_oracle/parse_designs.py reads production.industrial_organisations, scheduled_equipment_variants, and industrial_organisation design links",
            fail_closed_badge: "MIO/design production joins omitted until production-line variant pointer and MIO id roots are validated live",
            proposed_domain_id: PROPOSED_DOMAIN_MIO_DESIGN_JOINS,
            proposed_domain_name: domain_name(PROPOSED_DOMAIN_MIO_DESIGN_JOINS),
            owner_boundary: "join keys stay pointer-free: variant_id and (tag_db_id, kind, line_id)",
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
            current_domain_id: match gap.name {
                "requested_factory_counts"
                | "mio_design_variant_production_joins"
                | "deployment_queues" => DOMAIN_PRODUCTION as u32,
                _ => 0,
            },
            proposed_domain_id: gap.proposed_domain_id as u32,
            current_row_len: match gap.name {
                "requested_factory_counts" | "mio_design_variant_production_joins" => {
                    PRODUCTION_LINE_ROW_LEN as u32
                }
                "deployment_queues" => PRODUCTION_CONSTRUCTION_ROW_LEN as u32,
                _ => 0,
            },
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
        expected_domains,
    );
    let ok = summary.parse_errors.is_empty()
        && summary.ordering_errors.is_empty()
        && summary.coverage_errors.is_empty()
        && summary.status_flags_or
            & (STATUS_WORKER_PANIC
                | STATUS_BRIDGE_UNAVAILABLE
                | STATUS_SOURCE_UNAVAILABLE
                | STATUS_PARTIAL_DATA)
            == 0;
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
    let mut previous_sequence = None;

    while offset < raw.len() {
        let remaining = raw.len().saturating_sub(offset);
        if remaining < COMPACT_RECORD_HEADER_LEN {
            summary
                .parse_errors
                .push(format!("offset {offset}: truncated compact header"));
            break;
        }
        let header = &raw[offset..offset + COMPACT_RECORD_HEADER_LEN];
        if &header[0..4] != COMPACT_RECORD_MAGIC {
            summary
                .parse_errors
                .push(format!("offset {offset}: invalid compact magic"));
            break;
        }
        let version = read_u16_le(header, 4);
        let header_len = read_u16_le(header, 6) as usize;
        if version != COMPACT_RECORD_VERSION || header_len != COMPACT_RECORD_HEADER_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: unsupported compact header version {version} length {header_len}"
            ));
            break;
        }
        let sequence = read_u64_le(header, 8);
        let game_hour_raw = read_i32_le(header, 16);
        let domain_id = read_u16_le(header, 20);
        let record_type = read_u16_le(header, 22);
        let payload_len = read_u32_le(header, 24) as usize;
        let record_len = COMPACT_RECORD_HEADER_LEN.saturating_add(payload_len);
        if record_len > remaining {
            summary.parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
            ));
            break;
        }
        if let Some(previous) = previous_sequence {
            if sequence <= previous {
                summary.ordered = false;
                summary.ordering_errors.push(format!(
                    "offset {offset}: sequence {sequence} did not advance past {previous}"
                ));
            }
        }
        previous_sequence = Some(sequence);

        let payload_start = offset + COMPACT_RECORD_HEADER_LEN;
        let payload_end = payload_start + payload_len;
        let payload = &raw[payload_start..payload_end];
        summary.record_count = summary.record_count.saturating_add(1);
        summary.total_payload_bytes = summary.total_payload_bytes.saturating_add(payload_len);
        summary.max_payload_len = summary.max_payload_len.max(payload_len);

        match record_type {
            RECORD_TYPE_DOMAIN_PERF => {
                decode_domain_perf(offset, game_hour_raw, domain_id, payload, &mut summary);
                increment_hour_domain(
                    &mut summary.per_hour_perf_domain_counts,
                    game_hour_raw,
                    domain_id,
                );
            }
            RECORD_TYPE_DOMAIN_DATA => {
                let domain = domain_entry(&mut summary, domain_id);
                domain.record_count = domain.record_count.saturating_add(1);
                domain.data_record_count = domain.data_record_count.saturating_add(1);
                increment_hour_domain(
                    &mut summary.per_hour_data_domain_counts,
                    game_hour_raw,
                    domain_id,
                );
                match domain_id {
                    DOMAIN_FUEL => decode_fuel_payload(offset, payload, &mut summary),
                    DOMAIN_PRODUCTION => decode_production_payload(offset, payload, &mut summary),
                    DOMAIN_STOCKPILE_VARIANT_DEFS => {
                        decode_variant_defs_payload(offset, payload, domain_id, &mut summary)
                    }
                    DOMAIN_STOCKPILE_VARIANT_UPGRADES => {
                        decode_variant_upgrades_payload(offset, payload, domain_id, &mut summary)
                    }
                    DOMAIN_GAP_BADGES => {
                        decode_gap_badges_payload(offset, payload, domain_id, &mut summary)
                    }
                    _ => {}
                }
            }
            RECORD_TYPE_FRAME_END => {
                summary.frame_end_record_count = summary.frame_end_record_count.saturating_add(1);
                *summary.frame_hour_counts.entry(game_hour_raw).or_default() += 1;
                if payload.len() >= 28 {
                    summary.status_flags_or |= read_u32_le(payload, 24);
                }
            }
            other => summary
                .parse_errors
                .push(format!("offset {offset}: unknown record type {other}")),
        }
        offset = offset.saturating_add(record_len);
    }

    summary.unique_hours = summary.frame_hour_counts.keys().copied().collect();
    summary.first_hour = summary.unique_hours.first().copied();
    summary.last_hour = summary.unique_hours.last().copied();
    summary.skipped_hours = skipped_hours(&summary.unique_hours);
    summary.coverage_errors.clear();
    summary
}

fn decode_domain_perf(
    offset: usize,
    _game_hour_raw: i32,
    domain_id: u16,
    payload: &[u8],
    summary: &mut CompactStreamSummary,
) {
    if payload.len() < 40 {
        summary
            .parse_errors
            .push(format!("offset {offset}: truncated domain perf payload"));
        return;
    }
    let read_time_us = read_u64_le(payload, 8);
    let row_count = read_u32_le(payload, 16) as u64;
    let changed_rows = read_u32_le(payload, 20) as u64;
    let unit_count = read_u32_le(payload, 24) as u64;
    let changed_unit_count = read_u32_le(payload, 28) as u64;
    let status_flags = read_u32_le(payload, 32);
    summary.status_flags_or |= status_flags;
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
    summary.status_flags_or |= status_flags;
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
    summary.status_flags_or |= status_flags;
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
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: production payload length {} != expected {expected_len}",
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
    summary.status_flags_or |= status_flags;
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
    summary.status_flags_or |= status_flags;
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
    summary.status_flags_or |= status_flags;
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
                32,
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
            &encode_production_payload(10, 32, 1, &[], &[], &[], &[]),
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
                "requested_factory_counts",
                "consumer_goods",
                "resource_flows_shortages",
                "deployment_queues",
                "mio_design_variant_production_joins",
                "economy_law_derived_state",
                "trade_factory_impact",
            ]
        );
        assert!(gaps
            .iter()
            .all(|gap| gap.evidence_mask != 0 && gap.proposed_domain_id != 0));
        assert!(gaps
            .iter()
            .any(|gap| gap.status == GAP_STATUS_SCHEMA_BLOCKED));
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
        assert_eq!(domain.data_rows_total, 7);
        assert_eq!(
            domain
                .data_units_total
                .get("remaining_gap_badge_rows")
                .copied(),
            Some(7)
        );
        assert_eq!(
            domain
                .data_units_total
                .get("source_blocked_gap_rows")
                .copied(),
            Some(3)
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
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&read_time_us.to_le_bytes());
        payload.extend_from_slice(&rows.to_le_bytes());
        payload.extend_from_slice(&rows.to_le_bytes());
        payload.extend_from_slice(&units.to_le_bytes());
        payload.extend_from_slice(&units.to_le_bytes());
        payload.extend_from_slice(&32u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn frame_payload(domains: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&100u64.to_le_bytes());
        payload.extend_from_slice(&domains.to_le_bytes());
        payload.extend_from_slice(&domains.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn append_string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u16).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }
}
