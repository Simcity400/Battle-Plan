use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_ID: u16 = 58;
pub const DOMAIN_NAME: &str = "country_economy_counters_live";

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const RECORD_TYPE_FRAME_END: u16 = 2;
pub const RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const DOMAIN_FUEL_LIVE: u16 = 10;
pub const DOMAIN_PRODUCTION_LIVE: u16 = 11;
pub const DOMAIN_MAP_DETAIL_LIVE: u16 = 23;
pub const DOMAIN_MARKET_TRADE_LIVE: u16 = 43;
pub const DOMAIN_FRAME: u16 = 1000;

pub const STATUS_WORKER_PANIC: u32 = 1 << 6;
pub const STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const STATUS_LANE_LOCAL_PROOF: u32 = 1 << 5;

pub const COUNTER_PAYLOAD_HEADER_LEN: usize = 52;
pub const COUNTER_ROW_LEN: usize = 48;
pub const COUNTER_SCHEMA_VERSION: u32 = 1;

pub const COUNTER_STATUS_VALIDATED_SLICE: u32 = 1;
pub const COUNTER_STATUS_SCHEMA_BLOCKED: u32 = 2;
pub const COUNTER_STATUS_SOURCE_BLOCKED: u32 = 3;
pub const COUNTER_STATUS_OWNED_ELSEWHERE: u32 = 4;

pub const COUNTER_EVIDENCE_CURRENT_COMPACT_SCHEMA: u32 = 1;
pub const COUNTER_EVIDENCE_CURRENT_COLLECTOR_ROOTS: u32 = 1 << 1;
pub const COUNTER_EVIDENCE_SPEED_FIVE_PROOF: u32 = 1 << 2;
pub const COUNTER_EVIDENCE_SAVE_ORACLE: u32 = 1 << 3;
pub const COUNTER_EVIDENCE_OWNER_BOUNDARY: u32 = 1 << 4;
pub const COUNTER_EVIDENCE_MARKET_EXCLUDED: u32 = 1 << 5;
pub const COUNTER_EVIDENCE_NO_HOT_LOOP_ROOT: u32 = 1 << 6;
pub const COUNTER_EVIDENCE_REJECTED_CANDIDATES: u32 = 1 << 7;

pub const COUNTER_SUPPORT_NONE: u32 = 0;
pub const COUNTER_SUPPORT_PARTIAL: u32 = 1;
pub const COUNTER_SUPPORT_VALIDATED_SLICE: u32 = 2;
pub const COUNTER_SUPPORT_OWNED_ELSEWHERE: u32 = 3;

pub const ROOT_STATUS_VALIDATED: u32 = 1;
pub const ROOT_STATUS_MISSING: u32 = 2;
pub const ROOT_STATUS_SCHEMA_ONLY: u32 = 3;
pub const ROOT_STATUS_OWNED_ELSEWHERE: u32 = 4;
pub const ROOT_STATUS_REJECTED_CANDIDATE: u32 = 5;

pub const PROOF_STATUS_SPEED5_REPLAY_VERIFIED: u32 = 1;
pub const PROOF_STATUS_LANE_LOCAL_BADGE: u32 = 2;
pub const PROOF_STATUS_FIXTURE_REQUIRED: u32 = 3;
pub const PROOF_STATUS_NO_RETRY_WITHOUT_NEW_EVIDENCE: u32 = 4;

pub const EXCLUSION_NONE: u32 = 0;
pub const EXCLUSION_MARKET_TRADE: u32 = 1;
pub const EXCLUSION_STATE_DEPOSITS: u32 = 1 << 1;
pub const EXCLUSION_POLITICS_LAWS: u32 = 1 << 2;

pub const RECORD_KIND_FUEL_COUNTERS: u32 = 1;
pub const RECORD_KIND_PRODUCTION_LINE_COUNTERS: u32 = 2;
pub const RECORD_KIND_FACTORY_CAPACITY_COUNTERS: u32 = 3;
pub const RECORD_KIND_RESOURCE_FLOW_COUNTERS: u32 = 4;
pub const RECORD_KIND_COUNTER_BADGE: u32 = 255;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CounterBadgeRow {
    pub counter_id: u32,
    pub status: u32,
    pub evidence_mask: u32,
    pub current_support: u32,
    pub current_domain_id: u32,
    pub proposed_record_kind: u32,
    pub current_row_len: u32,
    pub root_status: u32,
    pub proof_status: u32,
    pub owner_domain_id: u32,
    pub exclusion_mask: u32,
    pub source_ref_id: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EconomyCounterBadge {
    pub counter_id: u32,
    pub name: &'static str,
    pub status: u32,
    pub status_label: &'static str,
    pub evidence_mask: u32,
    pub current_support: u32,
    pub current_compact_evidence: &'static str,
    pub source_evidence: &'static str,
    pub fail_closed_badge: &'static str,
    pub current_domain_id: u32,
    pub proposed_record_kind: u32,
    pub proposed_record: &'static str,
    pub current_row_len: u32,
    pub root_status: u32,
    pub proof_status: u32,
    pub owner_domain_id: u32,
    pub owner_boundary: &'static str,
    pub exclusion_mask: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CounterPayloadSummary {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub schema_version: u32,
    pub row_len: u32,
    pub validated_slice_rows: u32,
    pub schema_blocked_rows: u32,
    pub source_blocked_rows: u32,
    pub owned_elsewhere_rows: u32,
    pub market_excluded_rows: u32,
    pub rows: Vec<CounterBadgeRow>,
    pub parse_errors: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DomainRecordSummary {
    pub domain_id: u16,
    pub name: &'static str,
    pub perf_records: u64,
    pub data_records: u64,
    pub row_count_total: u64,
    pub changed_rows_total: u64,
    pub unit_count_total: u64,
    pub status_flags_or: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompactStreamSummary {
    pub total_bytes: usize,
    pub record_count: u64,
    pub domain_data_records: u64,
    pub frame_end_records: u64,
    pub first_game_hour_raw: Option<i32>,
    pub last_game_hour_raw: Option<i32>,
    pub unique_game_hours: usize,
    pub skipped_game_hours: Vec<i32>,
    pub status_flags_or: u32,
    pub parse_errors: Vec<String>,
    pub ordering_errors: Vec<String>,
    pub coverage_errors: Vec<String>,
    pub per_domain: BTreeMap<u16, DomainRecordSummary>,
    pub payloads: Vec<CounterPayloadSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CounterGate {
    pub ok: bool,
    pub expected_domains: Vec<u16>,
    pub summary: CompactStreamSummary,
}

pub fn domain_name(domain_id: u16) -> &'static str {
    match domain_id {
        DOMAIN_ID => DOMAIN_NAME,
        DOMAIN_FUEL_LIVE => "fuel_live",
        DOMAIN_PRODUCTION_LIVE => "production_live",
        DOMAIN_MAP_DETAIL_LIVE => "map_detail_live",
        DOMAIN_MARKET_TRADE_LIVE => "market_trade_live",
        DOMAIN_FRAME => "frame",
        _ => "unknown",
    }
}

pub fn economy_counter_badges() -> Vec<EconomyCounterBadge> {
    vec![
        EconomyCounterBadge {
            counter_id: 1,
            name: "fuel_capacity_gain_use",
            status: COUNTER_STATUS_VALIDATED_SLICE,
            status_label: "validated_slice",
            evidence_mask: COUNTER_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | COUNTER_EVIDENCE_CURRENT_COLLECTOR_ROOTS
                | COUNTER_EVIDENCE_SPEED_FIVE_PROOF,
            current_support: COUNTER_SUPPORT_VALIDATED_SLICE,
            current_compact_evidence: "fuel_live domain 10 rows carry tag_db_id, current fuel, max fuel, gain base, and fuel cost at fixed scale 32768",
            source_evidence: "validated current roots: CCountry+0x1580 -> CCountryFuelStatus; fields +0x08 current, +0x10 max, +0x18 gain base, +0x38 fuel cost",
            fail_closed_badge: "domain 58 does not duplicate fuel rows; it badges fuel capacity/gain/use as already captured by domain 10",
            current_domain_id: DOMAIN_FUEL_LIVE as u32,
            proposed_record_kind: RECORD_KIND_FUEL_COUNTERS,
            proposed_record: "fuel_capacity_gain_use",
            current_row_len: 40,
            root_status: ROOT_STATUS_VALIDATED,
            proof_status: PROOF_STATUS_SPEED5_REPLAY_VERIFIED,
            owner_domain_id: DOMAIN_FUEL_LIVE as u32,
            owner_boundary: "domain 10 owns fuel payloads; domain 58 may only gate/report coverage",
            exclusion_mask: EXCLUSION_NONE,
        },
        EconomyCounterBadge {
            counter_id: 2,
            name: "production_line_active_output_counters",
            status: COUNTER_STATUS_VALIDATED_SLICE,
            status_label: "validated_slice",
            evidence_mask: COUNTER_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | COUNTER_EVIDENCE_CURRENT_COLLECTOR_ROOTS
                | COUNTER_EVIDENCE_SPEED_FIVE_PROOF,
            current_support: COUNTER_SUPPORT_VALIDATED_SLICE,
            current_compact_evidence: "production_live domain 11 production line rows are 32 bytes: tag_db_id, kind, line_id, active_factories, cost, speed, produced, amount",
            source_evidence: "validated current roots: CCountry+0x0F68 -> CProductionManager; line vector +0x58/+0x60/+0x64; line fields +0x08 type, +0x0C id, +0x18 active factories, +0x28 cost, +0x30 speed, +0x38 produced, +0x48 amount",
            fail_closed_badge: "domain 58 does not recapture domain 11 rows; missing residual line fields stay badged below",
            current_domain_id: DOMAIN_PRODUCTION_LIVE as u32,
            proposed_record_kind: RECORD_KIND_PRODUCTION_LINE_COUNTERS,
            proposed_record: "production_line_active_output_counters",
            current_row_len: 32,
            root_status: ROOT_STATUS_VALIDATED,
            proof_status: PROOF_STATUS_SPEED5_REPLAY_VERIFIED,
            owner_domain_id: DOMAIN_PRODUCTION_LIVE as u32,
            owner_boundary: "domain 11 owns production line, stockpile, and construction compact rows",
            exclusion_mask: EXCLUSION_NONE,
        },
        EconomyCounterBadge {
            counter_id: 3,
            name: "requested_factory_counts",
            status: COUNTER_STATUS_SCHEMA_BLOCKED,
            status_label: "schema_blocked",
            evidence_mask: COUNTER_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | COUNTER_EVIDENCE_CURRENT_COLLECTOR_ROOTS
                | COUNTER_EVIDENCE_SAVE_ORACLE,
            current_support: COUNTER_SUPPORT_NONE,
            current_compact_evidence: "domain 11 line rows do not carry requested_factories; only active_factories is encoded",
            source_evidence: "save_oracle/parse_production.py reads countries.<TAG>.production.*_lines.requested_factories",
            fail_closed_badge: "requested factories omitted until a live CEquipmentProductionLine requested-factory offset is validated and added to a schema bump",
            current_domain_id: DOMAIN_PRODUCTION_LIVE as u32,
            proposed_record_kind: RECORD_KIND_PRODUCTION_LINE_COUNTERS,
            proposed_record: "production_line_requested_factory_counts",
            current_row_len: 32,
            root_status: ROOT_STATUS_SCHEMA_ONLY,
            proof_status: PROOF_STATUS_LANE_LOCAL_BADGE,
            owner_domain_id: DOMAIN_ID as u32,
            owner_boundary: "domain 58 owns residual factory-demand counters keyed by tag_db_id, kind, line_id",
            exclusion_mask: EXCLUSION_NONE,
        },
        EconomyCounterBadge {
            counter_id: 4,
            name: "factory_capacity_totals_damaged_free_consumer_goods",
            status: COUNTER_STATUS_SOURCE_BLOCKED,
            status_label: "source_blocked",
            evidence_mask: COUNTER_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | COUNTER_EVIDENCE_SAVE_ORACLE
                | COUNTER_EVIDENCE_NO_HOT_LOOP_ROOT,
            current_support: COUNTER_SUPPORT_NONE,
            current_compact_evidence: "accepted production/fuel streams do not carry civilian, military, naval, damaged, used/free, or consumer-goods total country counters",
            source_evidence: "replay schema and save economy surfaces reserve country economy.factories and consumer-goods style counters for 1:1 replay",
            fail_closed_badge: "factory capacity totals and consumer goods remain null/badged until a validated numeric country root is found",
            current_domain_id: 0,
            proposed_record_kind: RECORD_KIND_FACTORY_CAPACITY_COUNTERS,
            proposed_record: "country_factory_capacity_counters",
            current_row_len: 0,
            root_status: ROOT_STATUS_MISSING,
            proof_status: PROOF_STATUS_FIXTURE_REQUIRED,
            owner_domain_id: DOMAIN_ID as u32,
            owner_boundary: "domain 58 owns numeric factory capacity counters; politics/law tokens are joins only",
            exclusion_mask: EXCLUSION_POLITICS_LAWS,
        },
        EconomyCounterBadge {
            counter_id: 5,
            name: "production_line_efficiency_shortage_conversion_history",
            status: COUNTER_STATUS_SOURCE_BLOCKED,
            status_label: "source_blocked",
            evidence_mask: COUNTER_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | COUNTER_EVIDENCE_SAVE_ORACLE
                | COUNTER_EVIDENCE_NO_HOT_LOOP_ROOT,
            current_support: COUNTER_SUPPORT_PARTIAL,
            current_compact_evidence: "domain 11 carries line active_factories, cost, speed, produced, and amount; it does not carry efficiency, resource shortage, conversion, or line history fields",
            source_evidence: "target contract row explicitly requires residual military/naval production efficiency, resource shortage, conversion, and line-history closure",
            fail_closed_badge: "residual line counters omitted until exact CEquipmentProductionLine fields are fixture-validated by line_id",
            current_domain_id: DOMAIN_PRODUCTION_LIVE as u32,
            proposed_record_kind: RECORD_KIND_PRODUCTION_LINE_COUNTERS,
            proposed_record: "production_line_residual_counters",
            current_row_len: 32,
            root_status: ROOT_STATUS_MISSING,
            proof_status: PROOF_STATUS_FIXTURE_REQUIRED,
            owner_domain_id: DOMAIN_ID as u32,
            owner_boundary: "domain 58 owns only residual numeric counters; domain 11 remains the base line-row owner",
            exclusion_mask: EXCLUSION_NONE,
        },
        EconomyCounterBadge {
            counter_id: 6,
            name: "non_market_resource_flows_shortages",
            status: COUNTER_STATUS_SOURCE_BLOCKED,
            status_label: "source_blocked",
            evidence_mask: COUNTER_EVIDENCE_SAVE_ORACLE
                | COUNTER_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | COUNTER_EVIDENCE_MARKET_EXCLUDED
                | COUNTER_EVIDENCE_REJECTED_CANDIDATES,
            current_support: COUNTER_SUPPORT_PARTIAL,
            current_compact_evidence: "map/state lanes cover state resource deposits; accepted country streams do not carry country produced/imported/exported/used/deficit flow rows",
            source_evidence: "market_trade_live rejected CCountry+0x5B8 and CCountry+0x1218 as full country resource-flow roots; domain 43 owns market/trade contracts and stockpile/licensing residuals",
            fail_closed_badge: "non-market country resource flow rows omitted until a new live root is validated; do not retry rejected market lane candidates without new evidence",
            current_domain_id: 0,
            proposed_record_kind: RECORD_KIND_RESOURCE_FLOW_COUNTERS,
            proposed_record: "country_non_market_resource_flow_counters",
            current_row_len: 0,
            root_status: ROOT_STATUS_REJECTED_CANDIDATE,
            proof_status: PROOF_STATUS_NO_RETRY_WITHOUT_NEW_EVIDENCE,
            owner_domain_id: DOMAIN_ID as u32,
            owner_boundary: "domain 58 owns only non-market aggregate country resource counters; market contracts/licensing stay domain 43",
            exclusion_mask: EXCLUSION_MARKET_TRADE | EXCLUSION_STATE_DEPOSITS,
        },
        EconomyCounterBadge {
            counter_id: 7,
            name: "market_contract_factory_trade_effects",
            status: COUNTER_STATUS_OWNED_ELSEWHERE,
            status_label: "owned_elsewhere",
            evidence_mask: COUNTER_EVIDENCE_OWNER_BOUNDARY
                | COUNTER_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | COUNTER_EVIDENCE_MARKET_EXCLUDED,
            current_support: COUNTER_SUPPORT_OWNED_ELSEWHERE,
            current_compact_evidence: "market_trade_live domain 43 owns purchase contracts, price/detail trailers, lend-lease mapping, market stockpile, licensing, and trade residual blockers",
            source_evidence: "target instructions explicitly require avoiding market contract/licensing rows already owned by domain 43",
            fail_closed_badge: "domain 58 excludes market/licensing rows except future orchestrator-owned aggregate joins",
            current_domain_id: DOMAIN_MARKET_TRADE_LIVE as u32,
            proposed_record_kind: RECORD_KIND_COUNTER_BADGE,
            proposed_record: "market_trade_boundary_badge",
            current_row_len: 0,
            root_status: ROOT_STATUS_OWNED_ELSEWHERE,
            proof_status: PROOF_STATUS_LANE_LOCAL_BADGE,
            owner_domain_id: DOMAIN_MARKET_TRADE_LIVE as u32,
            owner_boundary: "domain 43 owns market_trade_live rows; domain 58 must not duplicate them",
            exclusion_mask: EXCLUSION_MARKET_TRADE,
        },
        EconomyCounterBadge {
            counter_id: 8,
            name: "state_resource_deposit_context",
            status: COUNTER_STATUS_OWNED_ELSEWHERE,
            status_label: "owned_elsewhere",
            evidence_mask: COUNTER_EVIDENCE_OWNER_BOUNDARY
                | COUNTER_EVIDENCE_CURRENT_COMPACT_SCHEMA,
            current_support: COUNTER_SUPPORT_OWNED_ELSEWHERE,
            current_compact_evidence: "state/map detail slices own resource deposits; domain 58 needs aggregate country flows, not duplicate state deposit rows",
            source_evidence: "master resources row cites map_detail_live plus market_trade residuals; this lane closes only missing country-level counters",
            fail_closed_badge: "state deposits excluded; domain 58 must join or aggregate only after orchestrator registration",
            current_domain_id: DOMAIN_MAP_DETAIL_LIVE as u32,
            proposed_record_kind: RECORD_KIND_COUNTER_BADGE,
            proposed_record: "state_resource_owner_boundary_badge",
            current_row_len: 0,
            root_status: ROOT_STATUS_OWNED_ELSEWHERE,
            proof_status: PROOF_STATUS_LANE_LOCAL_BADGE,
            owner_domain_id: DOMAIN_MAP_DETAIL_LIVE as u32,
            owner_boundary: "map/detail lanes own per-state resources; domain 58 owns country aggregate counters only",
            exclusion_mask: EXCLUSION_STATE_DEPOSITS,
        },
    ]
}

pub fn counter_badge_rows(badges: &[EconomyCounterBadge]) -> Vec<CounterBadgeRow> {
    badges
        .iter()
        .map(|badge| CounterBadgeRow {
            counter_id: badge.counter_id,
            status: badge.status,
            evidence_mask: badge.evidence_mask,
            current_support: badge.current_support,
            current_domain_id: badge.current_domain_id,
            proposed_record_kind: badge.proposed_record_kind,
            current_row_len: badge.current_row_len,
            root_status: badge.root_status,
            proof_status: badge.proof_status,
            owner_domain_id: badge.owner_domain_id,
            exclusion_mask: badge.exclusion_mask,
            source_ref_id: badge.counter_id,
        })
        .collect()
}

pub fn encode_counter_payload(
    elapsed_wall_ms: u64,
    status_flags: u32,
    rows: &[CounterBadgeRow],
) -> Vec<u8> {
    let row_count = rows.len().min(u32::MAX as usize) as u32;
    let changed_rows = row_count;
    let validated_slice_rows = count_status(rows, COUNTER_STATUS_VALIDATED_SLICE);
    let schema_blocked_rows = count_status(rows, COUNTER_STATUS_SCHEMA_BLOCKED);
    let source_blocked_rows = count_status(rows, COUNTER_STATUS_SOURCE_BLOCKED);
    let owned_elsewhere_rows = count_status(rows, COUNTER_STATUS_OWNED_ELSEWHERE);
    let market_excluded_rows = rows
        .iter()
        .filter(|row| row.exclusion_mask & EXCLUSION_MARKET_TRADE != 0)
        .count()
        .min(u32::MAX as usize) as u32;

    let mut payload = Vec::with_capacity(COUNTER_PAYLOAD_HEADER_LEN + rows.len() * COUNTER_ROW_LEN);
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&changed_rows.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&COUNTER_SCHEMA_VERSION.to_le_bytes());
    payload.extend_from_slice(&(COUNTER_ROW_LEN as u32).to_le_bytes());
    payload.extend_from_slice(&validated_slice_rows.to_le_bytes());
    payload.extend_from_slice(&schema_blocked_rows.to_le_bytes());
    payload.extend_from_slice(&source_blocked_rows.to_le_bytes());
    payload.extend_from_slice(&owned_elsewhere_rows.to_le_bytes());
    payload.extend_from_slice(&market_excluded_rows.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.counter_id.to_le_bytes());
        payload.extend_from_slice(&row.status.to_le_bytes());
        payload.extend_from_slice(&row.evidence_mask.to_le_bytes());
        payload.extend_from_slice(&row.current_support.to_le_bytes());
        payload.extend_from_slice(&row.current_domain_id.to_le_bytes());
        payload.extend_from_slice(&row.proposed_record_kind.to_le_bytes());
        payload.extend_from_slice(&row.current_row_len.to_le_bytes());
        payload.extend_from_slice(&row.root_status.to_le_bytes());
        payload.extend_from_slice(&row.proof_status.to_le_bytes());
        payload.extend_from_slice(&row.owner_domain_id.to_le_bytes());
        payload.extend_from_slice(&row.exclusion_mask.to_le_bytes());
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

pub fn gate_counter_stream(raw: &[u8], min_game_hours: usize) -> CounterGate {
    let mut summary = summarize_compact_stream(raw);
    if summary.domain_data_records == 0 {
        summary
            .coverage_errors
            .push("no_country_economy_counters_live_data_records".to_string());
    }
    if summary.unique_game_hours < min_game_hours {
        summary.coverage_errors.push(format!(
            "unique_game_hours {} < required {min_game_hours}",
            summary.unique_game_hours
        ));
    }
    let ok = summary.parse_errors.is_empty()
        && summary.ordering_errors.is_empty()
        && summary.coverage_errors.is_empty()
        && summary.skipped_game_hours.is_empty()
        && summary.status_flags_or
            & (STATUS_WORKER_PANIC
                | STATUS_BRIDGE_UNAVAILABLE
                | STATUS_SOURCE_UNAVAILABLE
                | STATUS_PARTIAL_DATA)
            == 0;
    CounterGate {
        ok,
        expected_domains: vec![DOMAIN_ID],
        summary,
    }
}

pub fn summarize_source_stream(raw: &[u8], expected_domains: &[u16]) -> CounterGate {
    let mut summary = summarize_compact_stream(raw);
    let mut hours_by_domain: BTreeMap<u16, BTreeSet<i32>> = BTreeMap::new();
    let mut frame_hours = BTreeSet::new();
    let mut offset = 0usize;
    while offset + COMPACT_RECORD_HEADER_LEN <= raw.len() {
        let header = &raw[offset..offset + COMPACT_RECORD_HEADER_LEN];
        let payload_len = read_u32_le(header, 24) as usize;
        let record_len = COMPACT_RECORD_HEADER_LEN.saturating_add(payload_len);
        if offset.saturating_add(record_len) > raw.len() {
            break;
        }
        let hour = read_i32_le(header, 16);
        let domain_id = read_u16_le(header, 20);
        let record_type = read_u16_le(header, 22);
        if record_type == RECORD_TYPE_FRAME_END {
            frame_hours.insert(hour);
        } else if record_type == RECORD_TYPE_DOMAIN_DATA && expected_domains.contains(&domain_id) {
            hours_by_domain.entry(domain_id).or_default().insert(hour);
        }
        offset += record_len;
    }

    if !summary.skipped_game_hours.is_empty() {
        summary.coverage_errors.push(format!(
            "skipped_game_hours {}",
            summary.skipped_game_hours.len()
        ));
    }
    for domain_id in expected_domains {
        let domain = summary.per_domain.get(domain_id);
        if domain.map_or(0, |d| d.data_records) == 0 {
            summary.coverage_errors.push(format!(
                "expected source domain {} had no data records",
                domain_name(*domain_id)
            ));
        }
        let source_hours = hours_by_domain.get(domain_id).cloned().unwrap_or_default();
        for hour in &frame_hours {
            if !source_hours.contains(hour) {
                summary.coverage_errors.push(format!(
                    "game_hour_raw {hour}: missing {} source data record",
                    domain_name(*domain_id)
                ));
            }
        }
    }
    let ok = summary.parse_errors.is_empty()
        && summary.ordering_errors.is_empty()
        && summary.coverage_errors.is_empty()
        && summary.status_flags_or
            & (STATUS_WORKER_PANIC
                | STATUS_BRIDGE_UNAVAILABLE
                | STATUS_SOURCE_UNAVAILABLE
                | STATUS_PARTIAL_DATA)
            == 0;
    CounterGate {
        ok,
        expected_domains: expected_domains.to_vec(),
        summary,
    }
}

pub fn summarize_compact_stream(raw: &[u8]) -> CompactStreamSummary {
    let mut summary = CompactStreamSummary {
        total_bytes: raw.len(),
        ..CompactStreamSummary::default()
    };
    let mut offset = 0usize;
    let mut previous_sequence = None::<u64>;
    let mut hours = BTreeSet::new();
    let mut data_hours = BTreeSet::new();

    while offset < raw.len() {
        if raw.len().saturating_sub(offset) < COMPACT_RECORD_HEADER_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: trailing bytes shorter than compact header"
            ));
            break;
        }
        let header = &raw[offset..offset + COMPACT_RECORD_HEADER_LEN];
        if &header[0..4] != COMPACT_RECORD_MAGIC {
            summary
                .parse_errors
                .push(format!("offset {offset}: bad compact magic"));
            break;
        }
        let version = read_u16_le(header, 4);
        let header_len = read_u16_le(header, 6) as usize;
        let sequence = read_u64_le(header, 8);
        let game_hour_raw = read_i32_le(header, 16);
        let domain_id = read_u16_le(header, 20);
        let record_type = read_u16_le(header, 22);
        let payload_len = read_u32_le(header, 24) as usize;
        if version != COMPACT_RECORD_VERSION || header_len != COMPACT_RECORD_HEADER_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: unsupported compact header version {version} len {header_len}"
            ));
            break;
        }
        if let Some(prev) = previous_sequence {
            if sequence <= prev {
                summary.ordering_errors.push(format!(
                    "offset {offset}: sequence {sequence} after {prev}"
                ));
            }
        }
        previous_sequence = Some(sequence);
        let record_len = COMPACT_RECORD_HEADER_LEN.saturating_add(payload_len);
        if offset.saturating_add(record_len) > raw.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: record length {record_len} exceeds stream length"
            ));
            break;
        }
        let payload_start = offset + COMPACT_RECORD_HEADER_LEN;
        let payload = &raw[payload_start..payload_start + payload_len];
        summary.record_count = summary.record_count.saturating_add(1);
        hours.insert(game_hour_raw);
        let domain = summary
            .per_domain
            .entry(domain_id)
            .or_insert_with(|| DomainRecordSummary {
                domain_id,
                name: domain_name(domain_id),
                ..DomainRecordSummary::default()
            });

        match record_type {
            RECORD_TYPE_DOMAIN_PERF => {
                domain.perf_records = domain.perf_records.saturating_add(1);
                if payload.len() >= 32 {
                    domain.row_count_total =
                        domain.row_count_total.saturating_add(read_u32_le(payload, 16) as u64);
                    domain.changed_rows_total = domain
                        .changed_rows_total
                        .saturating_add(read_u32_le(payload, 20) as u64);
                    domain.unit_count_total = domain
                        .unit_count_total
                        .saturating_add(read_u32_le(payload, 24) as u64);
                    domain.status_flags_or |= read_u32_le(payload, 28);
                    summary.status_flags_or |= read_u32_le(payload, 28);
                }
            }
            RECORD_TYPE_FRAME_END => {
                summary.frame_end_records = summary.frame_end_records.saturating_add(1);
                if payload.len() >= 28 {
                    summary.status_flags_or |= read_u32_le(payload, 24);
                }
            }
            RECORD_TYPE_DOMAIN_DATA => {
                domain.data_records = domain.data_records.saturating_add(1);
                data_hours.insert(game_hour_raw);
                if domain_id == DOMAIN_ID {
                    summary.domain_data_records = summary.domain_data_records.saturating_add(1);
                    let payload_summary = decode_counter_payload(offset, payload);
                    summary.status_flags_or |= payload_summary.status_flags;
                    domain.row_count_total = domain
                        .row_count_total
                        .saturating_add(payload_summary.row_count as u64);
                    domain.changed_rows_total = domain
                        .changed_rows_total
                        .saturating_add(payload_summary.changed_rows as u64);
                    domain.status_flags_or |= payload_summary.status_flags;
                    summary
                        .parse_errors
                        .extend(payload_summary.parse_errors.iter().cloned());
                    summary.payloads.push(payload_summary);
                }
            }
            other => summary.parse_errors.push(format!(
                "offset {offset}: unknown compact record type {other}"
            )),
        }
        offset += record_len;
    }

    let all_hours: BTreeSet<i32> = hours.union(&data_hours).copied().collect();
    summary.first_game_hour_raw = all_hours.first().copied();
    summary.last_game_hour_raw = all_hours.last().copied();
    summary.unique_game_hours = all_hours.len();
    summary.skipped_game_hours = skipped_hours(&all_hours);
    summary
}

pub fn decode_counter_payload(offset: usize, payload: &[u8]) -> CounterPayloadSummary {
    let mut summary = CounterPayloadSummary::default();
    if payload.len() < COUNTER_PAYLOAD_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: counter payload shorter than header"
        ));
        return summary;
    }
    summary.elapsed_wall_ms = read_u64_le(payload, 0);
    summary.row_count = read_u32_le(payload, 8);
    summary.changed_rows = read_u32_le(payload, 12);
    summary.status_flags = read_u32_le(payload, 16);
    summary.schema_version = read_u32_le(payload, 20);
    summary.row_len = read_u32_le(payload, 24);
    summary.validated_slice_rows = read_u32_le(payload, 28);
    summary.schema_blocked_rows = read_u32_le(payload, 32);
    summary.source_blocked_rows = read_u32_le(payload, 36);
    summary.owned_elsewhere_rows = read_u32_le(payload, 40);
    summary.market_excluded_rows = read_u32_le(payload, 44);
    if summary.schema_version != COUNTER_SCHEMA_VERSION {
        summary.parse_errors.push(format!(
            "offset {offset}: unsupported counter schema {}",
            summary.schema_version
        ));
    }
    if summary.row_len != COUNTER_ROW_LEN as u32 {
        summary.parse_errors.push(format!(
            "offset {offset}: counter row_len {} != {}",
            summary.row_len, COUNTER_ROW_LEN
        ));
    }
    let expected_len =
        COUNTER_PAYLOAD_HEADER_LEN.saturating_add(summary.row_count as usize * COUNTER_ROW_LEN);
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: counter payload length {} != expected {expected_len}",
            payload.len()
        ));
        return summary;
    }

    let mut seen = BTreeSet::new();
    let mut validated = 0u32;
    let mut schema = 0u32;
    let mut source = 0u32;
    let mut owned = 0u32;
    let mut market_excluded = 0u32;
    let mut cursor = COUNTER_PAYLOAD_HEADER_LEN;
    for row_index in 0..summary.row_count {
        let row = CounterBadgeRow {
            counter_id: read_u32_le(payload, cursor),
            status: read_u32_le(payload, cursor + 4),
            evidence_mask: read_u32_le(payload, cursor + 8),
            current_support: read_u32_le(payload, cursor + 12),
            current_domain_id: read_u32_le(payload, cursor + 16),
            proposed_record_kind: read_u32_le(payload, cursor + 20),
            current_row_len: read_u32_le(payload, cursor + 24),
            root_status: read_u32_le(payload, cursor + 28),
            proof_status: read_u32_le(payload, cursor + 32),
            owner_domain_id: read_u32_le(payload, cursor + 36),
            exclusion_mask: read_u32_le(payload, cursor + 40),
            source_ref_id: read_u32_le(payload, cursor + 44),
        };
        cursor += COUNTER_ROW_LEN;
        validate_counter_row(offset, row_index, &row, &mut summary.parse_errors, &mut seen);
        match row.status {
            COUNTER_STATUS_VALIDATED_SLICE => validated = validated.saturating_add(1),
            COUNTER_STATUS_SCHEMA_BLOCKED => schema = schema.saturating_add(1),
            COUNTER_STATUS_SOURCE_BLOCKED => source = source.saturating_add(1),
            COUNTER_STATUS_OWNED_ELSEWHERE => owned = owned.saturating_add(1),
            _ => {}
        }
        if row.exclusion_mask & EXCLUSION_MARKET_TRADE != 0 {
            market_excluded = market_excluded.saturating_add(1);
        }
        summary.rows.push(row);
    }

    if validated != summary.validated_slice_rows
        || schema != summary.schema_blocked_rows
        || source != summary.source_blocked_rows
        || owned != summary.owned_elsewhere_rows
        || market_excluded != summary.market_excluded_rows
    {
        summary.parse_errors.push(format!(
            "offset {offset}: counter status/exclusion counters do not match rows"
        ));
    }
    summary
}

fn validate_counter_row(
    offset: usize,
    row_index: u32,
    row: &CounterBadgeRow,
    errors: &mut Vec<String>,
    seen: &mut BTreeSet<u32>,
) {
    if row.counter_id == 0 || !seen.insert(row.counter_id) {
        errors.push(format!(
            "offset {offset}: counter row {row_index} has zero or duplicate counter_id {}",
            row.counter_id
        ));
    }
    if row.source_ref_id != row.counter_id {
        errors.push(format!(
            "offset {offset}: counter row {row_index} source_ref_id {} != counter_id {}",
            row.source_ref_id, row.counter_id
        ));
    }
    if !matches!(
        row.status,
        COUNTER_STATUS_VALIDATED_SLICE
            | COUNTER_STATUS_SCHEMA_BLOCKED
            | COUNTER_STATUS_SOURCE_BLOCKED
            | COUNTER_STATUS_OWNED_ELSEWHERE
    ) {
        errors.push(format!(
            "offset {offset}: counter row {row_index} invalid status {}",
            row.status
        ));
    }
    if row.evidence_mask == 0 || row.proposed_record_kind == 0 {
        errors.push(format!(
            "offset {offset}: counter row {row_index} lacks evidence or record kind"
        ));
    }
    if !matches!(
        row.current_support,
        COUNTER_SUPPORT_NONE
            | COUNTER_SUPPORT_PARTIAL
            | COUNTER_SUPPORT_VALIDATED_SLICE
            | COUNTER_SUPPORT_OWNED_ELSEWHERE
    ) {
        errors.push(format!(
            "offset {offset}: counter row {row_index} invalid current_support {}",
            row.current_support
        ));
    }
    if row.current_domain_id != 0 && domain_name(row.current_domain_id as u16) == "unknown" {
        errors.push(format!(
            "offset {offset}: counter row {row_index} unknown current_domain_id {}",
            row.current_domain_id
        ));
    }
    if row.owner_domain_id != 0 && domain_name(row.owner_domain_id as u16) == "unknown" {
        errors.push(format!(
            "offset {offset}: counter row {row_index} unknown owner_domain_id {}",
            row.owner_domain_id
        ));
    }
}

fn count_status(rows: &[CounterBadgeRow], status: u32) -> u32 {
    rows.iter()
        .filter(|row| row.status == status)
        .count()
        .min(u32::MAX as usize) as u32
}

pub fn skipped_hours(hours: &BTreeSet<i32>) -> Vec<i32> {
    let ordered: Vec<i32> = hours.iter().copied().collect();
    let mut missing = Vec::new();
    for pair in ordered.windows(2) {
        if pair[1] > pair[0] + 1 {
            missing.extend((pair[0] + 1)..pair[1]);
        }
    }
    missing
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_badges_cover_assigned_boundary() {
        let badges = economy_counter_badges();
        let names = badges.iter().map(|badge| badge.name).collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "fuel_capacity_gain_use",
                "production_line_active_output_counters",
                "requested_factory_counts",
                "factory_capacity_totals_damaged_free_consumer_goods",
                "production_line_efficiency_shortage_conversion_history",
                "non_market_resource_flows_shortages",
                "market_contract_factory_trade_effects",
                "state_resource_deposit_context",
            ]
        );
        assert_eq!(
            badges
                .iter()
                .filter(|badge| badge.status == COUNTER_STATUS_VALIDATED_SLICE)
                .count(),
            2
        );
        assert!(badges.iter().any(|badge| {
            badge.name == "market_contract_factory_trade_effects"
                && badge.owner_domain_id == DOMAIN_MARKET_TRADE_LIVE as u32
                && badge.exclusion_mask & EXCLUSION_MARKET_TRADE != 0
        }));
    }

    #[test]
    fn payload_roundtrips_counter_badges() {
        let rows = counter_badge_rows(&economy_counter_badges());
        let payload = encode_counter_payload(9, STATUS_LANE_LOCAL_PROOF, &rows);

        let decoded = decode_counter_payload(0, &payload);

        assert!(decoded.parse_errors.is_empty(), "{:?}", decoded.parse_errors);
        assert_eq!(decoded.row_count, rows.len() as u32);
        assert_eq!(decoded.validated_slice_rows, 2);
        assert_eq!(decoded.schema_blocked_rows, 1);
        assert_eq!(decoded.source_blocked_rows, 3);
        assert_eq!(decoded.owned_elsewhere_rows, 2);
        assert_eq!(decoded.market_excluded_rows, 2);
        assert_eq!(decoded.rows, rows);
    }

    #[test]
    fn gate_accepts_one_hour_counter_stream() {
        let rows = counter_badge_rows(&economy_counter_badges());
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(rows.len() as u32, rows.len() as u32),
        );
        append_compact_record(
            &mut stream,
            2,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_counter_payload(1, STATUS_LANE_LOCAL_PROOF, &rows),
        );
        append_compact_record(&mut stream, 3, 100, DOMAIN_FRAME, RECORD_TYPE_FRAME_END, &[]);

        let gate = gate_counter_stream(&stream, 1);

        assert!(gate.ok, "{:?}", gate.summary);
        assert_eq!(gate.summary.domain_data_records, 1);
        assert_eq!(gate.summary.unique_game_hours, 1);
        assert!(gate.summary.skipped_game_hours.is_empty());
    }

    #[test]
    fn decoder_rejects_duplicate_counter_ids() {
        let mut rows = counter_badge_rows(&economy_counter_badges());
        rows[1].counter_id = rows[0].counter_id;
        rows[1].source_ref_id = rows[0].source_ref_id;
        let payload = encode_counter_payload(0, 0, &rows);

        let decoded = decode_counter_payload(0, &payload);

        assert!(decoded
            .parse_errors
            .iter()
            .any(|err| err.contains("duplicate counter_id")));
    }

    fn perf_payload(rows: u32, units: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u64.to_le_bytes());
        payload.extend_from_slice(&rows.to_le_bytes());
        payload.extend_from_slice(&rows.to_le_bytes());
        payload.extend_from_slice(&units.to_le_bytes());
        payload.extend_from_slice(&units.to_le_bytes());
        payload.extend_from_slice(&STATUS_LANE_LOCAL_PROOF.to_le_bytes());
        payload
    }
}
