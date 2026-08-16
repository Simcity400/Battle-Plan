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
    pub frame_hour_counts: BTreeMap<i32, u64>,
    pub per_hour_perf_domain_counts: BTreeMap<i32, BTreeMap<u16, u64>>,
    pub per_hour_data_domain_counts: BTreeMap<i32, BTreeMap<u16, u64>>,
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
            source_evidence: "2026-07-03 v3 derived-oracle and v4 raw-total probes validated 0 factory/consumer scalar offsets across IRE/ENG/GER/USA/SOV/JAP/ITA/FRA; CCountry+0x1750/0x17E0 are rejected, CCountry+0xF08 is unmapped delta-fixture-only evidence",
            fail_closed_badge: "no domain-58 factory rows promoted; orchestrator must choose derived state+offmap totals or a before/after delta fixture",
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
            source_evidence: "2026-07-03 oracle probe reported 240 production-line residual oracle matches, but the section remained CANDIDATES_ONLY_UNTIL_CONTROLLED_DELTA and did not prove a unique residual field offset",
            fail_closed_badge: "residual line counters omitted until exact CEquipmentProductionLine fields are controlled-delta validated by line_id",
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
            source_evidence: "2026-07-03 oracle probe resource_flow_vector_candidates.oracle_matches.validated_cross_country_count=0; market_trade_live also rejected CCountry+0x5B8 and CCountry+0x1218 as full country resource-flow roots",
            fail_closed_badge: "non-market country resource flow rows omitted until a new live root validates produced/imported/exported/used/deficit across sampled countries",
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
    add_frame_shape_errors(&mut summary, &[DOMAIN_ID]);
    if summary.unique_game_hours < min_game_hours {
        summary.coverage_errors.push(format!(
            "unique_game_hours {} < required {min_game_hours}",
            summary.unique_game_hours
        ));
    }
    let ok = gate_summary_ok(&summary);
    CounterGate {
        ok,
        expected_domains: vec![DOMAIN_ID],
        summary,
    }
}

pub fn summarize_source_stream(raw: &[u8], expected_domains: &[u16]) -> CounterGate {
    let mut summary = summarize_compact_stream(raw);
    add_frame_shape_errors(&mut summary, expected_domains);
    let ok = gate_summary_ok(&summary);
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
    if let Err(error) = compact_contract::stream_validator::validate_freeze_census_stream_placement(raw) {
        summary.parse_errors.push(format!("stateful stream placement: {error}"));
    }
    let mut offset = 0usize;
    let mut previous_sequence = None::<u64>;
    let mut pending_perf_hour = None::<i32>;
    let mut open_frame_hour = None::<i32>;
    let mut last_closed_hour = None::<i32>;
    let mut final_record_is_clean_frame_end = false;
    let mut release_epilogue_seen = false;

    while offset < raw.len() {
        let header = match compact_contract::parse_compact_record_header(&raw[offset..]) {
            Ok(header) => header,
            Err(compact_contract::RecordFramingError::TruncatedHeader(_)) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: trailing bytes shorter than compact header"
                ));
                break;
            }
            Err(compact_contract::RecordFramingError::BadMagic) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: bad compact magic"));
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
        match previous_sequence {
            None if sequence != 1 => summary.ordering_errors.push(format!(
                "offset {offset}: sequence {sequence} does not start at 1 (stream prefix is missing)"
            )),
            Some(previous) => match previous.checked_add(1) {
                Some(expected) if sequence != expected => summary.ordering_errors.push(format!(
                    "offset {offset}: sequence discontinuity {previous} -> {sequence} (expected {expected})"
                )),
                None => summary.ordering_errors.push(format!(
                    "offset {offset}: sequence space exhausted after {previous}; got {sequence} (wrapping is not allowed)"
                )),
                _ => {}
            },
            _ => {}
        }
        previous_sequence = Some(sequence);
        let previous_record_closed_world = final_record_is_clean_frame_end;
        final_record_is_clean_frame_end = false;
        let record_len = header.record_len();
        if offset.saturating_add(record_len) > raw.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: record length {record_len} exceeds stream length"
            ));
            break;
        }
        let payload_start = offset + COMPACT_RECORD_HEADER_LEN;
        let payload = &raw[payload_start..payload_start + payload_len];
        let semantics_ok = match compact_contract::validate_compact_record_semantics(
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
        // The single release epilogue the owner ruling admits after a closed-hour
        // boundary. It uses the SHARED policy so this gate cannot drift into a
        // laxer rule than the stateful validator run above.
        if semantics_ok && record_type == compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS {
            if let Ok(census) =
                compact_contract::freeze_census_record::FreezeCensusRecordV1::decode(payload)
            {
                final_record_is_clean_frame_end =
                    compact_contract::freeze_census_record::release_epilogue_closes_stream_policy(
                        previous_record_closed_world,
                        census.phase(),
                        release_epilogue_seen,
                    );
                release_epilogue_seen |= census.phase()
                    == compact_contract::freeze_census_record::FreezeCensusPhase::Release;
            }
        }
        summary.record_count = summary.record_count.saturating_add(1);

        match record_type {
            RECORD_TYPE_DOMAIN_PERF => {
                if semantics_ok {
                    observe_perf_hour(
                        &mut summary,
                        &mut pending_perf_hour,
                        open_frame_hour,
                        last_closed_hour,
                        game_hour_raw,
                        offset,
                    );
                    increment_hour_domain(
                        &mut summary.per_hour_perf_domain_counts,
                        game_hour_raw,
                        domain_id,
                    );
                }
                // Payloads for domains this lane does not own remain opaque.
                let status_flags =
                    (semantics_ok && domain_id == DOMAIN_ID).then(|| read_u32_le(payload, 32));
                if let Some(status_flags) = status_flags {
                    validate_status_flags(
                        offset,
                        "country_economy_counters_live PERF",
                        status_flags,
                        &mut summary.parse_errors,
                    );
                    summary.status_flags_or |= status_flags;
                }
                let domain = domain_summary_entry(&mut summary, domain_id);
                domain.perf_records = domain.perf_records.saturating_add(1);
                if semantics_ok && domain_id == DOMAIN_ID {
                    domain.row_count_total = domain
                        .row_count_total
                        .saturating_add(read_u32_le(payload, 16) as u64);
                    domain.changed_rows_total = domain
                        .changed_rows_total
                        .saturating_add(read_u32_le(payload, 20) as u64);
                    domain.unit_count_total = domain
                        .unit_count_total
                        .saturating_add(read_u32_le(payload, 24) as u64);
                    domain.status_flags_or |= status_flags.expect("target PERF status");
                }
            }
            RECORD_TYPE_FRAME_END => {
                let _ = domain_summary_entry(&mut summary, domain_id);
                summary.frame_end_records = summary.frame_end_records.saturating_add(1);
                let mut clean = semantics_ok;
                if semantics_ok {
                    let status_flags = read_u32_le(payload, 24);
                    validate_status_flags(
                        offset,
                        "FRAME_END",
                        status_flags,
                        &mut summary.parse_errors,
                    );
                    summary.status_flags_or |= status_flags;
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
                    *summary.frame_hour_counts.entry(game_hour_raw).or_default() += 1;
                    open_frame_hour = None;
                    pending_perf_hour = None;
                    last_closed_hour = Some(game_hour_raw);
                    final_record_is_clean_frame_end = true;
                }
            }
            RECORD_TYPE_DOMAIN_DATA => {
                if semantics_ok {
                    observe_data_hour(
                        &mut summary,
                        pending_perf_hour,
                        &mut open_frame_hour,
                        last_closed_hour,
                        game_hour_raw,
                        offset,
                    );
                    increment_hour_domain(
                        &mut summary.per_hour_data_domain_counts,
                        game_hour_raw,
                        domain_id,
                    );
                }
                // Payloads for domains this lane does not own remain opaque.
                let payload_summary = if semantics_ok && domain_id == DOMAIN_ID {
                    Some(decode_counter_payload(offset, payload))
                } else {
                    None
                };
                if let Some(payload_summary) = payload_summary.as_ref() {
                    summary.status_flags_or |= payload_summary.status_flags;
                }
                if semantics_ok && domain_id == DOMAIN_ID {
                    summary.domain_data_records = summary.domain_data_records.saturating_add(1);
                    let payload_summary = payload_summary.expect("target DATA payload");
                    summary
                        .parse_errors
                        .extend(payload_summary.parse_errors.iter().cloned());
                    {
                        let domain = domain_summary_entry(&mut summary, domain_id);
                        domain.data_records = domain.data_records.saturating_add(1);
                        domain.row_count_total = domain
                            .row_count_total
                            .saturating_add(payload_summary.row_count as u64);
                        domain.changed_rows_total = domain
                            .changed_rows_total
                            .saturating_add(payload_summary.changed_rows as u64);
                        domain.status_flags_or |= payload_summary.status_flags;
                    }
                    summary.payloads.push(payload_summary);
                } else {
                    let domain = domain_summary_entry(&mut summary, domain_id);
                    domain.data_records = domain.data_records.saturating_add(1);
                }
            }
            _ => {
                let _ = domain_summary_entry(&mut summary, domain_id);
            }
        }
        offset += record_len;
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

    let closed_hours = summary
        .frame_hour_counts
        .keys()
        .copied()
        .collect::<BTreeSet<_>>();
    summary.first_game_hour_raw = closed_hours.first().copied();
    summary.last_game_hour_raw = closed_hours.last().copied();
    summary.unique_game_hours = closed_hours.len();
    summary.skipped_game_hours = skipped_hours(&closed_hours);
    summary
}

fn gate_summary_ok(summary: &CompactStreamSummary) -> bool {
    summary.parse_errors.is_empty()
        && summary.ordering_errors.is_empty()
        && summary.coverage_errors.is_empty()
        && summary.skipped_game_hours.is_empty()
        && compact_contract::compact_status_failure_flags_after_expected_partial_tolerance(
            DOMAIN_ID,
            summary.status_flags_or,
        ) == 0
}

fn add_frame_shape_errors(summary: &mut CompactStreamSummary, expected_domains: &[u16]) {
    if !summary.skipped_game_hours.is_empty() {
        summary.coverage_errors.push(format!(
            "skipped_game_hours {}",
            summary.skipped_game_hours.len()
        ));
    }

    for (&hour, &frame_count) in &summary.frame_hour_counts {
        if frame_count != 1 {
            summary.coverage_errors.push(format!(
                "game_hour_raw {hour}: frame_end_count {frame_count} != 1"
            ));
        }
        for &domain_id in expected_domains {
            let perf_count =
                hour_domain_count(&summary.per_hour_perf_domain_counts, hour, domain_id);
            if perf_count != 1 {
                summary.coverage_errors.push(format!(
                    "game_hour_raw {hour}: compact domain {} perf_record_count {perf_count} != 1",
                    domain_name(domain_id)
                ));
            }
            let data_count =
                hour_domain_count(&summary.per_hour_data_domain_counts, hour, domain_id);
            if data_count != 1 {
                summary.coverage_errors.push(format!(
                    "game_hour_raw {hour}: compact domain {} data_record_count {data_count} != 1",
                    domain_name(domain_id)
                ));
            }
        }
    }

    for (&hour, domains) in &summary.per_hour_perf_domain_counts {
        if summary.frame_hour_counts.contains_key(&hour) {
            continue;
        }
        for &domain_id in expected_domains {
            let count = domains.get(&domain_id).copied().unwrap_or(0);
            if count != 0 {
                summary.coverage_errors.push(format!(
                    "game_hour_raw {hour}: compact domain {} has {count} PERF record(s) on a non-certified frame hour",
                    domain_name(domain_id)
                ));
            }
        }
    }
    for (&hour, domains) in &summary.per_hour_data_domain_counts {
        if summary.frame_hour_counts.contains_key(&hour) {
            continue;
        }
        for &domain_id in expected_domains {
            let count = domains.get(&domain_id).copied().unwrap_or(0);
            if count != 0 {
                summary.coverage_errors.push(format!(
                    "game_hour_raw {hour}: compact domain {} has {count} DATA record(s) on a non-certified frame hour",
                    domain_name(domain_id)
                ));
            }
        }
    }
}

fn hour_domain_count(counts: &BTreeMap<i32, BTreeMap<u16, u64>>, hour: i32, domain_id: u16) -> u64 {
    counts
        .get(&hour)
        .and_then(|domains| domains.get(&domain_id))
        .copied()
        .unwrap_or(0)
}

fn increment_hour_domain(
    counts: &mut BTreeMap<i32, BTreeMap<u16, u64>>,
    hour: i32,
    domain_id: u16,
) {
    let count = counts
        .entry(hour)
        .or_default()
        .entry(domain_id)
        .or_default();
    *count = count.saturating_add(1);
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

fn domain_summary_entry(
    summary: &mut CompactStreamSummary,
    domain_id: u16,
) -> &mut DomainRecordSummary {
    summary
        .per_domain
        .entry(domain_id)
        .or_insert_with(|| DomainRecordSummary {
            domain_id,
            name: domain_name(domain_id),
            ..DomainRecordSummary::default()
        })
}

fn validate_status_flags(offset: usize, label: &str, status_flags: u32, errors: &mut Vec<String>) {
    // Re-exported, not re-spelled. This was an OR-list of every bit the
    // contract had assigned when it was written, which is the same local copy
    // as a literal mask and drifts the same way: the day the contract assigned
    // COMPACT_STATUS_SOURCE_EMPTY, this gate began rejecting a valid record as
    // carrying an unknown bit.
    const KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;
    let unknown = status_flags & !KNOWN_STATUS_MASK;
    if unknown != 0 {
        errors.push(format!(
            "offset {offset}: {label} status flags {status_flags:#010x} contain unknown bits {unknown:#010x}"
        ));
    }
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
    validate_status_flags(
        offset,
        "country_economy_counters_live DATA",
        summary.status_flags,
        &mut summary.parse_errors,
    );
    if summary.changed_rows > summary.row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: changed_rows {} exceeds row_count {}",
            summary.changed_rows, summary.row_count
        ));
    }
    if read_u32_le(payload, 48) != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: counter payload reserved word is nonzero"
        ));
    }
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
        validate_counter_row(
            offset,
            row_index,
            &row,
            &mut summary.parse_errors,
            &mut seen,
        );
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

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("u32 range"))
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

        assert!(
            decoded.parse_errors.is_empty(),
            "{:?}",
            decoded.parse_errors
        );
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
        append_compact_record(
            &mut stream,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );

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

    #[test]
    fn gate_rejects_non_one_start_and_sequence_gap() {
        let rows = counter_badge_rows(&economy_counter_badges());
        let mut non_one = Vec::new();
        append_compact_record(
            &mut non_one,
            2,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(rows.len() as u32, rows.len() as u32),
        );
        append_compact_record(
            &mut non_one,
            3,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_counter_payload(1, STATUS_LANE_LOCAL_PROOF, &rows),
        );
        append_compact_record(
            &mut non_one,
            4,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let gate = gate_counter_stream(&non_one, 1);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .ordering_errors
            .iter()
            .any(|error| error.contains("does not start at 1")));

        let mut gap = Vec::new();
        append_compact_record(
            &mut gap,
            1,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(rows.len() as u32, rows.len() as u32),
        );
        append_compact_record(
            &mut gap,
            3,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_counter_payload(1, STATUS_LANE_LOCAL_PROOF, &rows),
        );
        append_compact_record(
            &mut gap,
            4,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let gate = gate_counter_stream(&gap, 1);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .ordering_errors
            .iter()
            .any(|error| error.contains("sequence discontinuity 1 -> 3")));

        let mut wrapping = Vec::new();
        append_compact_record(
            &mut wrapping,
            u64::MAX,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(rows.len() as u32, rows.len() as u32),
        );
        append_compact_record(
            &mut wrapping,
            0,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_counter_payload(1, STATUS_LANE_LOCAL_PROOF, &rows),
        );
        append_compact_record(
            &mut wrapping,
            1,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let gate = gate_counter_stream(&wrapping, 1);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .ordering_errors
            .iter()
            .any(|error| error.contains("sequence space exhausted")));
    }

    #[test]
    fn gate_rejects_data_without_frame_closure_and_trailing_records() {
        let rows = counter_badge_rows(&economy_counter_badges());
        let mut open = Vec::new();
        append_compact_record(
            &mut open,
            1,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(rows.len() as u32, rows.len() as u32),
        );
        append_compact_record(
            &mut open,
            2,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_counter_payload(1, STATUS_LANE_LOCAL_PROOF, &rows),
        );
        let gate = gate_counter_stream(&open, 1);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .parse_errors
            .iter()
            .any(|error| error.contains("open DATA frame hour 100")));

        let mut trailing = Vec::new();
        let next = append_target_frame(&mut trailing, 1, 100, STATUS_LANE_LOCAL_PROOF);
        append_compact_record(
            &mut trailing,
            next,
            101,
            DOMAIN_FUEL_LIVE,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(0, 0, 0),
        );
        let gate = gate_counter_stream(&trailing, 1);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .parse_errors
            .iter()
            .any(|error| error.contains("final canonical FRAME_END")));
    }

    #[test]
    fn unrelated_frame_cannot_inflate_counter_duration() {
        let mut stream = Vec::new();
        let mut sequence = append_target_frame(&mut stream, 1, 100, STATUS_LANE_LOCAL_PROOF);
        append_compact_record(
            &mut stream,
            sequence,
            101,
            DOMAIN_FUEL_LIVE,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(0, 0, 0),
        );
        sequence += 1;
        append_compact_record(
            &mut stream,
            sequence,
            101,
            DOMAIN_FUEL_LIVE,
            RECORD_TYPE_DOMAIN_DATA,
            &[0xde, 0xad, 0xbe, 0xef],
        );
        sequence += 1;
        append_compact_record(
            &mut stream,
            sequence,
            101,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );

        let gate = gate_counter_stream(&stream, 2);
        assert!(!gate.ok);
        assert!(gate.summary.coverage_errors.iter().any(|error| {
            error.contains("game_hour_raw 101") && error.contains("data_record_count 0")
        }));
    }

    #[test]
    fn gate_requires_exactly_one_counter_perf_and_data_per_frame() {
        let rows = counter_badge_rows(&economy_counter_badges());

        let mut duplicate_data = Vec::new();
        append_compact_record(
            &mut duplicate_data,
            1,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(rows.len() as u32, rows.len() as u32),
        );
        for sequence in [2, 3] {
            append_compact_record(
                &mut duplicate_data,
                sequence,
                100,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_DATA,
                &encode_counter_payload(1, STATUS_LANE_LOCAL_PROOF, &rows),
            );
        }
        append_compact_record(
            &mut duplicate_data,
            4,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let gate = gate_counter_stream(&duplicate_data, 1);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .coverage_errors
            .iter()
            .any(|error| error.contains("data_record_count 2 != 1")));

        let mut duplicate_perf = Vec::new();
        for sequence in [1, 2] {
            append_compact_record(
                &mut duplicate_perf,
                sequence,
                100,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_PERF,
                &perf_payload(rows.len() as u32, rows.len() as u32),
            );
        }
        append_compact_record(
            &mut duplicate_perf,
            3,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_counter_payload(1, STATUS_LANE_LOCAL_PROOF, &rows),
        );
        append_compact_record(
            &mut duplicate_perf,
            4,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let gate = gate_counter_stream(&duplicate_perf, 1);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .coverage_errors
            .iter()
            .any(|error| error.contains("perf_record_count 2 != 1")));

        let mut missing_perf = Vec::new();
        append_compact_record(
            &mut missing_perf,
            1,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_counter_payload(1, STATUS_LANE_LOCAL_PROOF, &rows),
        );
        append_compact_record(
            &mut missing_perf,
            2,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let gate = gate_counter_stream(&missing_perf, 1);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .coverage_errors
            .iter()
            .any(|error| error.contains("perf_record_count 0 != 1")));

        let mut missing_data = Vec::new();
        append_compact_record(
            &mut missing_data,
            1,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(rows.len() as u32, rows.len() as u32),
        );
        append_compact_record(
            &mut missing_data,
            2,
            100,
            DOMAIN_FUEL_LIVE,
            RECORD_TYPE_DOMAIN_DATA,
            &[1],
        );
        append_compact_record(
            &mut missing_data,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let gate = gate_counter_stream(&missing_data, 1);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .coverage_errors
            .iter()
            .any(|error| error.contains("data_record_count 0 != 1")));
    }

    #[test]
    fn gate_rejects_wrong_perf_hour_and_nonconsecutive_or_reordered_closures() {
        let rows = counter_badge_rows(&economy_counter_badges());
        let mut wrong_perf = Vec::new();
        append_compact_record(
            &mut wrong_perf,
            1,
            101,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(rows.len() as u32, rows.len() as u32),
        );
        append_compact_record(
            &mut wrong_perf,
            2,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_counter_payload(1, STATUS_LANE_LOCAL_PROOF, &rows),
        );
        append_compact_record(
            &mut wrong_perf,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let gate = gate_counter_stream(&wrong_perf, 1);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .parse_errors
            .iter()
            .any(|error| error.contains("conflicts with pending PERF hour 101")));

        for second_hour in [102, 99] {
            let mut stream = Vec::new();
            let next = append_target_frame(&mut stream, 1, 100, STATUS_LANE_LOCAL_PROOF);
            append_target_frame(&mut stream, next, second_hour, STATUS_LANE_LOCAL_PROOF);
            let gate = gate_counter_stream(&stream, 2);
            assert!(!gate.ok, "second hour {second_hour} was accepted");
            assert!(gate.summary.parse_errors.iter().any(|error| {
                error.contains("closed frame hour 100") && error.contains("expected 101")
            }));
        }
    }

    #[test]
    fn gate_rejects_bad_record_flags_and_statuses() {
        let mut bad_flags = Vec::new();
        append_target_frame(&mut bad_flags, 1, 100, STATUS_LANE_LOCAL_PROOF);
        let data_offset =
            COMPACT_RECORD_HEADER_LEN + compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN;
        bad_flags[data_offset + 28..data_offset + 32].copy_from_slice(
            &compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER.to_le_bytes(),
        );
        let gate = gate_counter_stream(&bad_flags, 1);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .parse_errors
            .iter()
            .any(|error| error.contains("rejected bits")));

        let mut fatal = Vec::new();
        append_target_frame(
            &mut fatal,
            1,
            100,
            compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE,
        );
        let gate = gate_counter_stream(&fatal, 1);
        assert!(!gate.ok);
        assert!(gate.summary.parse_errors.is_empty(), "{:?}", gate.summary);

        let mut unknown = Vec::new();
        append_target_frame(&mut unknown, 1, 100, 1 << 31);
        let gate = gate_counter_stream(&unknown, 1);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .parse_errors
            .iter()
            .any(|error| error.contains("unknown bits")));
    }

    #[test]
    fn source_gate_ignores_unowned_payloads_but_validates_their_envelopes() {
        let mut stream = Vec::new();
        let mut opaque_perf = perf_payload_with_status(0, 0, 1 << 31);
        opaque_perf[36..40].copy_from_slice(&0u32.to_le_bytes());
        append_compact_record(
            &mut stream,
            1,
            100,
            DOMAIN_FUEL_LIVE,
            RECORD_TYPE_DOMAIN_PERF,
            &opaque_perf,
        );
        append_compact_record(
            &mut stream,
            2,
            100,
            DOMAIN_FUEL_LIVE,
            RECORD_TYPE_DOMAIN_DATA,
            &[0xde, 0xad],
        );
        append_compact_record(
            &mut stream,
            3,
            100,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        let gate = summarize_source_stream(&stream, &[DOMAIN_FUEL_LIVE]);
        assert!(gate.ok, "{:?}", gate.summary);

        stream[28..32].copy_from_slice(&0x8000_0000u32.to_le_bytes());
        let gate = summarize_source_stream(&stream, &[DOMAIN_FUEL_LIVE]);
        assert!(!gate.ok);
        assert!(gate
            .summary
            .parse_errors
            .iter()
            .any(|error| error.contains("rejected bits")));
    }

    fn perf_payload(rows: u32, units: u32) -> Vec<u8> {
        perf_payload_with_status(rows, units, STATUS_LANE_LOCAL_PROOF)
    }

    fn perf_payload_with_status(rows: u32, units: u32, status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u64.to_le_bytes());
        payload.extend_from_slice(&rows.to_le_bytes());
        payload.extend_from_slice(&rows.to_le_bytes());
        payload.extend_from_slice(&units.to_le_bytes());
        payload.extend_from_slice(&units.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn frame_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload
    }

    fn append_target_frame(
        stream: &mut Vec<u8>,
        first_sequence: u64,
        hour: i32,
        status_flags: u32,
    ) -> u64 {
        let rows = counter_badge_rows(&economy_counter_badges());
        append_compact_record(
            stream,
            first_sequence,
            hour,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(rows.len() as u32, rows.len() as u32, status_flags),
        );
        append_compact_record(
            stream,
            first_sequence + 1,
            hour,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_counter_payload(1, status_flags, &rows),
        );
        append_compact_record(
            stream,
            first_sequence + 2,
            hour,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_payload(0),
        );
        first_sequence + 3
    }
}
