use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_ID: u16 = 43;
pub const DOMAIN_NAME: &str = "market_trade_live";

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const RECORD_TYPE_FRAME_END: u16 = 2;
pub const RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const STATUS_WORKER_PANIC: u32 = 1 << 6;
pub const STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;

pub const MARKET_TRADE_HEADER_LEN: usize = 48;
pub const MARKET_TRADE_ROW_LEN: usize = 48;
pub const MARKET_TRADE_CONTRACT_TYPE: u32 = 81;
pub const MARKET_TRADE_MAX_TAG_DB_ID: u32 = 2_048;
pub const MARKET_TRADE_MAX_CONTRACT_ID: u32 = i32::MAX as u32;
pub const MARKET_TRADE_MAX_VARIANT_ID: u32 = i32::MAX as u32;
pub const MARKET_TRADE_MAX_AMOUNT_RAW: i64 = 1_000_000_000_000_000;
pub const MARKET_TRADE_MAX_PROGRESS_RAW: i32 = i32::MAX;
pub const MARKET_TRADE_SCALE_RAW: u32 = 100_000;

pub const MARKET_TRADE_RESIDUAL_TRAILER_MAGIC: &[u8; 4] = b"MTRB";
pub const MARKET_TRADE_RESIDUAL_SCHEMA_VERSION: u16 = 1;
pub const MARKET_TRADE_RESIDUAL_HEADER_LEN: usize = 32;
pub const MARKET_TRADE_RESIDUAL_ROW_LEN: usize = 48;
pub const MARKET_TRADE_DETAIL_TRAILER_MAGIC: &[u8; 4] = b"MTD1";
pub const MARKET_TRADE_DETAIL_SCHEMA_VERSION: u16 = 1;
pub const MARKET_TRADE_DETAIL_HEADER_LEN: usize = 32;
pub const MARKET_TRADE_DETAIL_ROW_LEN: usize = 64;
pub const MARKET_TRADE_LEND_LEASE_TRAILER_MAGIC: &[u8; 4] = b"MTLL";
pub const MARKET_TRADE_LEND_LEASE_SCHEMA_VERSION: u16 = 1;
pub const MARKET_TRADE_LEND_LEASE_HEADER_LEN: usize = 32;
pub const MARKET_TRADE_LEND_LEASE_ROW_LEN: usize = 48;

pub const ROW_FLAG_BUYER_PTR_VALID: u32 = 1;
pub const ROW_FLAG_EQUIPMENT_SUBBUF_VALID: u32 = 1 << 1;
pub const ROW_FLAG_DELIVERY_SUBBUF_VALID: u32 = 1 << 2;
pub const ROW_FLAG_VARIANT_VALID: u32 = 1 << 3;
pub const ROW_FLAG_AMOUNT_VALID: u32 = 1 << 4;
pub const ROW_FLAGS_KNOWN: u32 = ROW_FLAG_BUYER_PTR_VALID
    | ROW_FLAG_EQUIPMENT_SUBBUF_VALID
    | ROW_FLAG_DELIVERY_SUBBUF_VALID
    | ROW_FLAG_VARIANT_VALID
    | ROW_FLAG_AMOUNT_VALID;

pub const FIELD_STATUS_INVALID: u32 = 0;
pub const FIELD_STATUS_VALID: u32 = 1;
pub const FIELD_STATUS_UNREADABLE: u32 = 2;

pub const DETAIL_FLAG_PRICE_VALID: u32 = 1;
pub const DETAIL_FLAG_AMOUNT_VALID: u32 = 1 << 1;
pub const DETAIL_FLAG_SPEED_VALID: u32 = 1 << 2;
pub const DETAIL_FLAG_COMPLETED_VALID: u32 = 1 << 3;
pub const DETAIL_FLAG_DAYS_VALID: u32 = 1 << 4;
pub const DETAIL_FLAG_DELIVERY_COUNTRY_VALID: u32 = 1 << 5;
pub const DETAIL_FLAG_CONVOYS_VALID: u32 = 1 << 6;
pub const DETAIL_FLAG_EFFICIENCY_VALID: u32 = 1 << 7;
pub const DETAIL_FLAG_FACTORY_PROGRESS_VALID: u32 = 1 << 8;
pub const DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID: u32 = 1 << 9;
pub const DETAIL_FLAGS_KNOWN: u32 = DETAIL_FLAG_PRICE_VALID
    | DETAIL_FLAG_AMOUNT_VALID
    | DETAIL_FLAG_SPEED_VALID
    | DETAIL_FLAG_COMPLETED_VALID
    | DETAIL_FLAG_DAYS_VALID
    | DETAIL_FLAG_DELIVERY_COUNTRY_VALID
    | DETAIL_FLAG_CONVOYS_VALID
    | DETAIL_FLAG_EFFICIENCY_VALID
    | DETAIL_FLAG_FACTORY_PROGRESS_VALID
    | DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID;

pub const LEND_LEASE_DIRECTION_OUTGOING: u32 = 1;
pub const LEND_LEASE_DIRECTION_INCOMING: u32 = 2;
pub const LEND_LEASE_FLAG_AMOUNT_VALID: u32 = 1;
pub const LEND_LEASE_FLAG_PRICE_VALID: u32 = 1 << 1;
pub const LEND_LEASE_FLAGS_KNOWN: u32 = LEND_LEASE_FLAG_AMOUNT_VALID | LEND_LEASE_FLAG_PRICE_VALID;
pub const LEND_LEASE_SOURCE_PURCHASE_CONTRACT: u32 = 1;

pub const RESIDUAL_STATUS_VALIDATED_SLICE: u32 = 1;
pub const RESIDUAL_STATUS_SCHEMA_BLOCKED: u32 = 2;
pub const RESIDUAL_STATUS_SOURCE_BLOCKED: u32 = 3;
pub const RESIDUAL_STATUS_OWNED_ELSEWHERE_CONTEXT: u32 = 4;
pub const RESIDUAL_STATUS_SHARED_PATCH_BLOCKED: u32 = 5;

pub const RESIDUAL_EVIDENCE_SAVE_ORACLE: u32 = 1;
pub const RESIDUAL_EVIDENCE_CURRENT_COMPACT_SCHEMA: u32 = 1 << 1;
pub const RESIDUAL_EVIDENCE_CURRENT_ROOT_PROOF: u32 = 1 << 2;
pub const RESIDUAL_EVIDENCE_D04_DYNAMIC_TAG_PROOF: u32 = 1 << 3;
pub const RESIDUAL_EVIDENCE_DIPLOMACY_LANE_ROOTS: u32 = 1 << 4;
pub const RESIDUAL_EVIDENCE_COVERAGE_AUDIT: u32 = 1 << 5;
pub const RESIDUAL_EVIDENCE_OWNER_BOUNDARY: u32 = 1 << 6;
pub const RESIDUAL_EVIDENCE_NO_HOT_LOOP_ROOT: u32 = 1 << 7;

pub const RESIDUAL_SUPPORT_NONE: u32 = 0;
pub const RESIDUAL_SUPPORT_PARTIAL: u32 = 1;
pub const RESIDUAL_SUPPORT_VALIDATED_SLICE: u32 = 2;
pub const RESIDUAL_SUPPORT_OWNED_ELSEWHERE: u32 = 3;
pub const RESIDUAL_SUPPORT_SHARED_PATCH_PENDING: u32 = 4;

pub const RESIDUAL_RECORD_KIND_PURCHASE_CONTRACT_CORE: u32 = 1;
pub const RESIDUAL_RECORD_KIND_PURCHASE_CONTRACT_DELIVERY: u32 = 2;
pub const RESIDUAL_RECORD_KIND_MARKET_STOCKPILE: u32 = 3;
pub const RESIDUAL_RECORD_KIND_COUNTRY_RESOURCE_FLOW: u32 = 4;
pub const RESIDUAL_RECORD_KIND_DIPLOMATIC_TRADE_CONTEXT: u32 = 5;
pub const RESIDUAL_RECORD_KIND_LICENSE_PRODUCTION: u32 = 6;
pub const RESIDUAL_RECORD_KIND_LEND_LEASE_MAPPING: u32 = 7;
pub const RESIDUAL_RECORD_KIND_RESIDUAL_BADGE: u32 = 255;

pub const RESIDUAL_OWNER_MARKET_TRADE: u32 = 1;
pub const RESIDUAL_OWNER_SHARED_COUNTRY_IDENTITY: u32 = 2;
pub const RESIDUAL_OWNER_DIPLOMACY_INTEL: u32 = 3;
pub const RESIDUAL_OWNER_ECONOMY_PRODUCTION: u32 = 4;

pub const RESIDUAL_ROOT_VALIDATED: u32 = 1;
pub const RESIDUAL_ROOT_UNVALIDATED: u32 = 2;
pub const RESIDUAL_ROOT_SCHEMA_ONLY: u32 = 3;
pub const RESIDUAL_ROOT_OWNED_ELSEWHERE: u32 = 4;
pub const RESIDUAL_ROOT_REJECTED_CANDIDATE: u32 = 5;

pub const RESIDUAL_PROOF_LIVE_870EBF: u32 = 1;
pub const RESIDUAL_PROOF_STATIC_ORACLE: u32 = 2;
pub const RESIDUAL_PROOF_REGISTRATION_PENDING: u32 = 3;
pub const RESIDUAL_PROOF_NOT_PROVEN_LIVE: u32 = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketTradeRow {
    pub contract_id: u32,
    pub contract_type: u32,
    pub flags: u32,
    pub seller_tag_db_id: u32,
    pub buyer_tag_db_id: u32,
    pub equipment_variant_id: u32,
    pub amount_raw: i64,
    pub equipment_subbuf_status: u32,
    pub delivery_subbuf_status: u32,
    pub buyer_ptr_status: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MarketTradeResidualBadgeRow {
    pub gap_id: u32,
    pub status: u32,
    pub evidence_mask: u32,
    pub current_support: u32,
    pub current_domain_id: u32,
    pub proposed_record_kind: u32,
    pub current_row_len: u32,
    pub source_ref_id: u32,
    pub owner_boundary: u32,
    pub root_status: u32,
    pub proof_status: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PurchaseContractDetailRow {
    pub contract_id: u32,
    pub flags: u32,
    pub price_raw: i64,
    pub amount_raw: i64,
    pub speed_raw: i32,
    pub completed_raw: i32,
    pub days: i32,
    pub delivery_country_tag_db_id: u32,
    pub convoys_assigned: i32,
    pub convoys_total: i32,
    pub efficiency_raw: i32,
    pub factory_cic_progress_raw: i32,
    pub equipment_transfer_progress_raw: i32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LendLeaseMappingRow {
    pub contract_id: u32,
    pub perspective_tag_db_id: u32,
    pub counterpart_tag_db_id: u32,
    pub direction: u32,
    pub equipment_variant_id: u32,
    pub flags: u32,
    pub amount_raw: i64,
    pub price_raw: i64,
    pub source_record_kind: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketTradeResidualBadge {
    pub gap_id: u32,
    pub name: &'static str,
    pub status: u32,
    pub status_label: &'static str,
    pub evidence_mask: u32,
    pub current_support: u32,
    pub current_compact_evidence: &'static str,
    pub source_evidence: &'static str,
    pub fail_closed_badge: &'static str,
    pub proposed_record_kind: u32,
    pub proposed_record: &'static str,
    pub owner_boundary: u32,
    pub owner_label: &'static str,
    pub root_status: u32,
    pub proof_status: u32,
}

impl MarketTradeRow {
    pub fn complete(contract_id: u32, seller_tag_db_id: u32, buyer_tag_db_id: u32) -> Self {
        Self {
            contract_id,
            contract_type: MARKET_TRADE_CONTRACT_TYPE,
            flags: ROW_FLAGS_KNOWN,
            seller_tag_db_id,
            buyer_tag_db_id,
            equipment_variant_id: 10_000 + contract_id,
            amount_raw: 25 * MARKET_TRADE_SCALE_RAW as i64,
            equipment_subbuf_status: FIELD_STATUS_VALID,
            delivery_subbuf_status: FIELD_STATUS_VALID,
            buyer_ptr_status: FIELD_STATUS_VALID,
        }
    }

    pub fn has_complete_extractable_fields(&self) -> bool {
        self.flags & ROW_FLAGS_KNOWN == ROW_FLAGS_KNOWN
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MarketTradePayloadSummary {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_contract_rows: u32,
    pub emitted_contract_rows: u32,
    pub omitted_candidate_rows: u32,
    pub buyer_ptr_valid_rows: u32,
    pub variant_valid_rows: u32,
    pub amount_valid_rows: u32,
    pub partial_field_rows: u32,
    pub scale_raw: u32,
    pub residual_trailer_present: bool,
    pub residual_badge_rows: u32,
    pub residual_schema_blocked_rows: u32,
    pub residual_source_blocked_rows: u32,
    pub residual_owned_elsewhere_rows: u32,
    pub residual_shared_patch_blocked_rows: u32,
    pub detail_trailer_present: bool,
    pub detail_rows: u32,
    pub detail_price_valid_rows: u32,
    pub detail_speed_valid_rows: u32,
    pub detail_completed_valid_rows: u32,
    pub detail_days_valid_rows: u32,
    pub detail_delivery_country_valid_rows: u32,
    pub detail_convoys_valid_rows: u32,
    pub detail_efficiency_valid_rows: u32,
    pub detail_delivery_route_valid_rows: u32,
    pub detail_progress_valid_rows: u32,
    pub lend_lease_trailer_present: bool,
    pub lend_lease_mapping_rows: u32,
    pub lend_lease_outgoing_rows: u32,
    pub lend_lease_incoming_rows: u32,
    pub lend_lease_amount_valid_rows: u32,
    pub lend_lease_price_valid_rows: u32,
    pub rows: Vec<MarketTradeRow>,
    pub residual_badges: Vec<MarketTradeResidualBadgeRow>,
    pub detail_contracts: Vec<PurchaseContractDetailRow>,
    pub lend_lease_mappings: Vec<LendLeaseMappingRow>,
    pub parse_errors: Vec<String>,
}

impl MarketTradePayloadSummary {
    pub fn missing_extractable_rows(&self) -> u32 {
        self.partial_field_rows
    }

    pub fn ok(&self) -> bool {
        self.parse_errors.is_empty()
            && self.status_flags
                & (STATUS_WORKER_PANIC
                    | STATUS_BRIDGE_UNAVAILABLE
                    | STATUS_SOURCE_UNAVAILABLE
                    | STATUS_PARTIAL_DATA)
                == 0
            && self.partial_field_rows == 0
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MarketTradeStreamSummary {
    pub total_bytes: usize,
    pub record_count: u64,
    pub domain_data_records: u64,
    pub frame_end_records: u64,
    pub first_game_hour_raw: Option<i32>,
    pub last_game_hour_raw: Option<i32>,
    pub unique_game_hours: usize,
    pub skipped_game_hours: Vec<i32>,
    pub status_flags_or: u32,
    pub row_count_total: u64,
    pub observed_contract_rows_total: u64,
    pub omitted_candidate_rows_total: u64,
    pub partial_field_rows_total: u64,
    pub residual_badge_records: u64,
    pub residual_badge_rows_total: u64,
    pub residual_schema_blocked_rows_total: u64,
    pub residual_source_blocked_rows_total: u64,
    pub residual_owned_elsewhere_rows_total: u64,
    pub residual_shared_patch_blocked_rows_total: u64,
    pub detail_records: u64,
    pub detail_rows_total: u64,
    pub detail_price_valid_rows_total: u64,
    pub detail_speed_valid_rows_total: u64,
    pub detail_completed_valid_rows_total: u64,
    pub detail_days_valid_rows_total: u64,
    pub detail_delivery_country_valid_rows_total: u64,
    pub detail_convoys_valid_rows_total: u64,
    pub detail_efficiency_valid_rows_total: u64,
    pub detail_delivery_route_valid_rows_total: u64,
    pub detail_progress_valid_rows_total: u64,
    pub lend_lease_mapping_records: u64,
    pub lend_lease_mapping_rows_total: u64,
    pub lend_lease_outgoing_rows_total: u64,
    pub lend_lease_incoming_rows_total: u64,
    pub lend_lease_amount_valid_rows_total: u64,
    pub lend_lease_price_valid_rows_total: u64,
    pub payloads: Vec<MarketTradePayloadSummary>,
    pub parse_errors: Vec<String>,
    pub ordering_errors: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketTradeGate {
    pub ok: bool,
    pub problems: Vec<&'static str>,
    pub summary: MarketTradeStreamSummary,
}

pub fn encode_payload(
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_contract_rows: u32,
    changed_rows: u32,
    rows: &[MarketTradeRow],
) -> Vec<u8> {
    let row_count = rows.len().min(u32::MAX as usize) as u32;
    let buyer_ptr_valid_rows = rows
        .iter()
        .filter(|row| row.flags & ROW_FLAG_BUYER_PTR_VALID != 0)
        .count()
        .min(u32::MAX as usize) as u32;
    let variant_valid_rows = rows
        .iter()
        .filter(|row| row.flags & ROW_FLAG_VARIANT_VALID != 0)
        .count()
        .min(u32::MAX as usize) as u32;
    let amount_valid_rows = rows
        .iter()
        .filter(|row| row.flags & ROW_FLAG_AMOUNT_VALID != 0)
        .count()
        .min(u32::MAX as usize) as u32;
    let partial_field_rows = rows
        .iter()
        .filter(|row| !row.has_complete_extractable_fields())
        .count()
        .min(u32::MAX as usize) as u32;

    let mut payload =
        Vec::with_capacity(MARKET_TRADE_HEADER_LEN + rows.len() * MARKET_TRADE_ROW_LEN);
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&changed_rows.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&observed_contract_rows.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&buyer_ptr_valid_rows.to_le_bytes());
    payload.extend_from_slice(&variant_valid_rows.to_le_bytes());
    payload.extend_from_slice(&amount_valid_rows.to_le_bytes());
    payload.extend_from_slice(&partial_field_rows.to_le_bytes());
    payload.extend_from_slice(&MARKET_TRADE_SCALE_RAW.to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.contract_id.to_le_bytes());
        payload.extend_from_slice(&row.contract_type.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.seller_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.buyer_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.equipment_variant_id.to_le_bytes());
        payload.extend_from_slice(&row.amount_raw.to_le_bytes());
        payload.extend_from_slice(&row.equipment_subbuf_status.to_le_bytes());
        payload.extend_from_slice(&row.delivery_subbuf_status.to_le_bytes());
        payload.extend_from_slice(&row.buyer_ptr_status.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    payload
}

pub fn encode_payload_with_residual_badges(
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_contract_rows: u32,
    changed_rows: u32,
    rows: &[MarketTradeRow],
    residual_badges: &[MarketTradeResidualBadgeRow],
) -> Vec<u8> {
    let mut payload = encode_payload(
        elapsed_wall_ms,
        status_flags,
        observed_contract_rows,
        changed_rows,
        rows,
    );
    payload.extend_from_slice(&encode_residual_trailer(residual_badges));
    payload
}

pub fn encode_payload_with_detail_rows(
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_contract_rows: u32,
    changed_rows: u32,
    rows: &[MarketTradeRow],
    detail_rows: &[PurchaseContractDetailRow],
) -> Vec<u8> {
    let mut payload = encode_payload(
        elapsed_wall_ms,
        status_flags,
        observed_contract_rows,
        changed_rows,
        rows,
    );
    payload.extend_from_slice(&encode_detail_trailer(detail_rows));
    payload
}

pub fn encode_payload_with_lend_lease_mappings(
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_contract_rows: u32,
    changed_rows: u32,
    rows: &[MarketTradeRow],
    mappings: &[LendLeaseMappingRow],
) -> Vec<u8> {
    let mut payload = encode_payload(
        elapsed_wall_ms,
        status_flags,
        observed_contract_rows,
        changed_rows,
        rows,
    );
    payload.extend_from_slice(&encode_lend_lease_trailer(mappings));
    payload
}

pub fn lend_lease_mapping_rows_from_contracts(
    rows: &[MarketTradeRow],
    price_by_contract: &BTreeMap<u32, i64>,
) -> Vec<LendLeaseMappingRow> {
    let mut out = Vec::with_capacity(rows.len().saturating_mul(2));
    for row in rows {
        let mut flags = 0u32;
        if row.flags & ROW_FLAG_AMOUNT_VALID != 0 && row.amount_raw > 0 {
            flags |= LEND_LEASE_FLAG_AMOUNT_VALID;
        }
        let price_raw = price_by_contract
            .get(&row.contract_id)
            .copied()
            .unwrap_or(0);
        if price_raw > 0 {
            flags |= LEND_LEASE_FLAG_PRICE_VALID;
        }
        out.push(LendLeaseMappingRow {
            contract_id: row.contract_id,
            perspective_tag_db_id: row.seller_tag_db_id,
            counterpart_tag_db_id: row.buyer_tag_db_id,
            direction: LEND_LEASE_DIRECTION_OUTGOING,
            equipment_variant_id: row.equipment_variant_id,
            flags,
            amount_raw: row.amount_raw,
            price_raw,
            source_record_kind: LEND_LEASE_SOURCE_PURCHASE_CONTRACT,
        });
        out.push(LendLeaseMappingRow {
            contract_id: row.contract_id,
            perspective_tag_db_id: row.buyer_tag_db_id,
            counterpart_tag_db_id: row.seller_tag_db_id,
            direction: LEND_LEASE_DIRECTION_INCOMING,
            equipment_variant_id: row.equipment_variant_id,
            flags,
            amount_raw: row.amount_raw,
            price_raw,
            source_record_kind: LEND_LEASE_SOURCE_PURCHASE_CONTRACT,
        });
    }
    out
}

pub fn market_trade_residual_badges() -> Vec<MarketTradeResidualBadge> {
    vec![
        MarketTradeResidualBadge {
            gap_id: 1,
            name: "purchase_contract_dynamic_tag_buyer_identity",
            status: RESIDUAL_STATUS_VALIDATED_SLICE,
            status_label: "validated_slice",
            evidence_mask: RESIDUAL_EVIDENCE_CURRENT_ROOT_PROOF
                | RESIDUAL_EVIDENCE_D04_DYNAMIC_TAG_PROOF
                | RESIDUAL_EVIDENCE_OWNER_BOUNDARY,
            current_support: RESIDUAL_SUPPORT_VALIDATED_SLICE,
            current_compact_evidence: "market_trade_live_dtag_shared_20260701T174028 decoded 71 domain-43 records with 0 partial rows after the shared dynamic-tag identity patch",
            source_evidence: "market_trade_live_d04_identity_20260701_current proves tag_table[368]=D04, country_table[368] equals the buyer pointer, and the pointer has the current CCountry vtable",
            fail_closed_badge: "dynamic D-tag buyer rows are accepted only through the shared bounded country_table slot fallback; ordinary CCountry+0x08 validation remains primary",
            proposed_record_kind: RESIDUAL_RECORD_KIND_PURCHASE_CONTRACT_CORE,
            proposed_record: "purchase_contract_core",
            owner_boundary: RESIDUAL_OWNER_SHARED_COUNTRY_IDENTITY,
            owner_label: "shared_country_identity",
            root_status: RESIDUAL_ROOT_VALIDATED,
            proof_status: RESIDUAL_PROOF_LIVE_870EBF,
        },
        MarketTradeResidualBadge {
            gap_id: 2,
            name: "omitted_candidate_classification",
            status: RESIDUAL_STATUS_VALIDATED_SLICE,
            status_label: "validated_slice",
            evidence_mask: RESIDUAL_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | RESIDUAL_EVIDENCE_CURRENT_ROOT_PROOF,
            current_support: RESIDUAL_SUPPORT_VALIDATED_SLICE,
            current_compact_evidence: "domain 43 v1 header separates observed_contract_rows, emitted_contract_rows, omitted_candidate_rows, and partial_field_rows",
            source_evidence: "the lane decoder rejects missing extractable rows as partial while allowing only non-contract/invalid/duplicate candidates as omitted-not-partial",
            fail_closed_badge: "candidate rows with unresolved country, variant, delivery amount, or buyer pointer are structural partials, not harmless omissions",
            proposed_record_kind: RESIDUAL_RECORD_KIND_PURCHASE_CONTRACT_CORE,
            proposed_record: "purchase_contract_core",
            owner_boundary: RESIDUAL_OWNER_MARKET_TRADE,
            owner_label: "market_trade_live",
            root_status: RESIDUAL_ROOT_VALIDATED,
            proof_status: RESIDUAL_PROOF_LIVE_870EBF,
        },
        MarketTradeResidualBadge {
            gap_id: 3,
            name: "purchase_contract_delivery_route_progress",
            status: RESIDUAL_STATUS_VALIDATED_SLICE,
            status_label: "validated_slice",
            evidence_mask: RESIDUAL_EVIDENCE_SAVE_ORACLE
                | RESIDUAL_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | RESIDUAL_EVIDENCE_CURRENT_ROOT_PROOF,
            current_support: RESIDUAL_SUPPORT_VALIDATED_SLICE,
            current_compact_evidence: "MTD1 same-tick v3 emits 63/63 delivery-country, days, efficiency, factory-progress, and equipment-transfer-progress rows plus 25 convoy rows",
            source_evidence: "purchase_contract_same_tick_probe v3 validates CPurchaseContract+0x5C delivery_country_tag_db_id, +0x1D0 days, +0x118/+0x11C convoys, +0x1C0 efficiency_raw, +0x1E0 factory_cic_progress_raw, and +0x1E8 equipment_transfer_progress_raw against 1943.11.9.17 save rows",
            fail_closed_badge: "delivery detail fields are emitted only under MTD1 valid flags; route/path fields not represented by MTD1 remain outside this validated slice until separately rooted",
            proposed_record_kind: RESIDUAL_RECORD_KIND_PURCHASE_CONTRACT_DELIVERY,
            proposed_record: "purchase_contract_delivery",
            owner_boundary: RESIDUAL_OWNER_MARKET_TRADE,
            owner_label: "market_trade_live",
            root_status: RESIDUAL_ROOT_VALIDATED,
            proof_status: RESIDUAL_PROOF_LIVE_870EBF,
        },
        MarketTradeResidualBadge {
            gap_id: 4,
            name: "purchase_contract_price_detail",
            status: RESIDUAL_STATUS_VALIDATED_SLICE,
            status_label: "validated_slice",
            evidence_mask: RESIDUAL_EVIDENCE_SAVE_ORACLE
                | RESIDUAL_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | RESIDUAL_EVIDENCE_CURRENT_ROOT_PROOF,
            current_support: RESIDUAL_SUPPORT_VALIDATED_SLICE,
            current_compact_evidence: "the MTD1 detail trailer encodes contract_id, price_raw, and amount_raw with anti-fabrication flags; live proof captured 50/50 price rows",
            source_evidence: "CPurchaseContract+0x38 equipment_subbuf points at CEquipmentVariant payloads whose i32 amount is the contract price raw value",
            fail_closed_badge: "price is emitted only when DETAIL_FLAG_PRICE_VALID is set; missing price rows fail closed instead of using amount or defaults",
            proposed_record_kind: RESIDUAL_RECORD_KIND_PURCHASE_CONTRACT_DELIVERY,
            proposed_record: "purchase_contract_detail",
            owner_boundary: RESIDUAL_OWNER_MARKET_TRADE,
            owner_label: "market_trade_live",
            root_status: RESIDUAL_ROOT_VALIDATED,
            proof_status: RESIDUAL_PROOF_LIVE_870EBF,
        },
        MarketTradeResidualBadge {
            gap_id: 5,
            name: "purchase_contract_speed_completed",
            status: RESIDUAL_STATUS_VALIDATED_SLICE,
            status_label: "validated_slice",
            evidence_mask: RESIDUAL_EVIDENCE_SAVE_ORACLE
                | RESIDUAL_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | RESIDUAL_EVIDENCE_CURRENT_ROOT_PROOF,
            current_support: RESIDUAL_SUPPORT_VALIDATED_SLICE,
            current_compact_evidence: "MTD1 same-tick v3 emits 63/63 speed_raw and completed_raw rows; Rust gate and replay pack verify zero skipped hours and zero partial rows",
            source_evidence: "purchase_contract_same_tick_probe v2 validates CPurchaseContract+0xC0 speed and +0x25C completed against 63 same-tick 1943.11.9.17 save rows keyed by contract_id",
            fail_closed_badge: "speed/completed are emitted only when DETAIL_FLAG_SPEED_VALID and DETAIL_FLAG_COMPLETED_VALID are set; missing or unaccepted roots stay zero and unflagged",
            proposed_record_kind: RESIDUAL_RECORD_KIND_PURCHASE_CONTRACT_DELIVERY,
            proposed_record: "purchase_contract_detail",
            owner_boundary: RESIDUAL_OWNER_MARKET_TRADE,
            owner_label: "market_trade_live",
            root_status: RESIDUAL_ROOT_VALIDATED,
            proof_status: RESIDUAL_PROOF_LIVE_870EBF,
        },
        MarketTradeResidualBadge {
            gap_id: 6,
            name: "market_stockpile_sellable_equipment",
            status: RESIDUAL_STATUS_SOURCE_BLOCKED,
            status_label: "source_blocked",
            evidence_mask: RESIDUAL_EVIDENCE_SAVE_ORACLE
                | RESIDUAL_EVIDENCE_COVERAGE_AUDIT
                | RESIDUAL_EVIDENCE_CURRENT_ROOT_PROOF
                | RESIDUAL_EVIDENCE_NO_HOT_LOOP_ROOT,
            current_support: RESIDUAL_SUPPORT_NONE,
            current_compact_evidence: "domain 43 v1 emits live purchase contracts only and has no per-country market_stockpile, price_levels, subsidies, or automation rows",
            source_evidence: "market_stockpile_root_probe v2 found no current RTTI vtable for CMarketStockpile/CEquipmentMarketStatus/CMarketRequestAutomation; stockpile_save_compare v4 rejects CCountry+0x1230 against a same-tick 1943.11.9.17 save oracle",
            fail_closed_badge: "sellable market stockpile and automation flags remain omitted; CCountry+0x1230 is rejected as the market_stockpile root and a different equipment_market root must be pinned",
            proposed_record_kind: RESIDUAL_RECORD_KIND_MARKET_STOCKPILE,
            proposed_record: "market_stockpile",
            owner_boundary: RESIDUAL_OWNER_MARKET_TRADE,
            owner_label: "market_trade_live",
            root_status: RESIDUAL_ROOT_REJECTED_CANDIDATE,
            proof_status: RESIDUAL_PROOF_LIVE_870EBF,
        },
        MarketTradeResidualBadge {
            gap_id: 7,
            name: "country_resource_import_export_flows",
            status: RESIDUAL_STATUS_SOURCE_BLOCKED,
            status_label: "source_blocked",
            evidence_mask: RESIDUAL_EVIDENCE_SAVE_ORACLE
                | RESIDUAL_EVIDENCE_COVERAGE_AUDIT
                | RESIDUAL_EVIDENCE_CURRENT_ROOT_PROOF
                | RESIDUAL_EVIDENCE_NO_HOT_LOOP_ROOT,
            current_support: RESIDUAL_SUPPORT_NONE,
            current_compact_evidence: "state resource deposits are covered by map/detail lanes, but domain 43 has no per-country produced/exported/imported/to_export/shortage rows",
            source_evidence: "save_oracle/parse_economy.py reads countries.<TAG>.resources.{produced,exported,to_export}; resource_flow_root_probe v1 rejects CCountry+0x5B8; resource_flow_state_shape_probe v1 rejects the stable CCountry+0x1218 vector as the full flow root",
            fail_closed_badge: "country resource import/export economy is omitted until a live country resource-flow root other than rejected +0x5B8/+0x1218 candidates is validated",
            proposed_record_kind: RESIDUAL_RECORD_KIND_COUNTRY_RESOURCE_FLOW,
            proposed_record: "country_resource_flow",
            owner_boundary: RESIDUAL_OWNER_MARKET_TRADE,
            owner_label: "market_trade_live",
            root_status: RESIDUAL_ROOT_REJECTED_CANDIDATE,
            proof_status: RESIDUAL_PROOF_LIVE_870EBF,
        },
        MarketTradeResidualBadge {
            gap_id: 8,
            name: "license_production_contract_detail",
            status: RESIDUAL_STATUS_SOURCE_BLOCKED,
            status_label: "source_blocked",
            evidence_mask: RESIDUAL_EVIDENCE_DIPLOMACY_LANE_ROOTS
                | RESIDUAL_EVIDENCE_CURRENT_ROOT_PROOF
                | RESIDUAL_EVIDENCE_OWNER_BOUNDARY
                | RESIDUAL_EVIDENCE_NO_HOT_LOOP_ROOT,
            current_support: RESIDUAL_SUPPORT_OWNED_ELSEWHERE,
            current_compact_evidence: "diplomacy domain 53 pins CLicenceProductionAccessRelation as a relation flag, but domain 43 has no licensed-equipment/factory-cost/detail records",
            source_evidence: "license_vector_candidate_probe v1 rejects CCountry+0x1070/+0x10A0: 15 hits all reference one shared CLicensedProductionConsumer object with no nested license/status/equipment children",
            fail_closed_badge: "license relation context may be joined from diplomacy, but license production detail remains omitted until a different market/economy-owned root or active license fixture is validated",
            proposed_record_kind: RESIDUAL_RECORD_KIND_LICENSE_PRODUCTION,
            proposed_record: "license_production",
            owner_boundary: RESIDUAL_OWNER_MARKET_TRADE,
            owner_label: "market_trade_live",
            root_status: RESIDUAL_ROOT_REJECTED_CANDIDATE,
            proof_status: RESIDUAL_PROOF_LIVE_870EBF,
        },
        MarketTradeResidualBadge {
            gap_id: 9,
            name: "market_access_diplomatic_context",
            status: RESIDUAL_STATUS_OWNED_ELSEWHERE_CONTEXT,
            status_label: "owned_elsewhere_context",
            evidence_mask: RESIDUAL_EVIDENCE_DIPLOMACY_LANE_ROOTS
                | RESIDUAL_EVIDENCE_OWNER_BOUNDARY,
            current_support: RESIDUAL_SUPPORT_OWNED_ELSEWHERE,
            current_compact_evidence: "market_trade_live should join relation context by tag pair; it should not duplicate diplomacy relation-flag roots",
            source_evidence: "diplomacy_intel_operatives_live pins CMarketAccessRightsRelation and CEquipmentPurchaseRelation current-build relation flags",
            fail_closed_badge: "market access/equipment-purchase relation context is consumed from diplomacy domain 53 until a shared join is registered",
            proposed_record_kind: RESIDUAL_RECORD_KIND_DIPLOMATIC_TRADE_CONTEXT,
            proposed_record: "diplomatic_trade_context_join",
            owner_boundary: RESIDUAL_OWNER_DIPLOMACY_INTEL,
            owner_label: "diplomacy_intel_operatives_live",
            root_status: RESIDUAL_ROOT_OWNED_ELSEWHERE,
            proof_status: RESIDUAL_PROOF_LIVE_870EBF,
        },
        MarketTradeResidualBadge {
            gap_id: 10,
            name: "lend_lease_market_contract_mapping",
            status: RESIDUAL_STATUS_VALIDATED_SLICE,
            status_label: "validated_slice",
            evidence_mask: RESIDUAL_EVIDENCE_SAVE_ORACLE
                | RESIDUAL_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | RESIDUAL_EVIDENCE_CURRENT_ROOT_PROOF,
            current_support: RESIDUAL_SUPPORT_VALIDATED_SLICE,
            current_compact_evidence: "MTLL maps every emitted purchase contract into one seller-outgoing and one buyer-incoming row keyed by contract_id, perspective tag, counterpart tag, equipment variant, amount, and optional price",
            source_evidence: "lend_lease_mapping_probe v1 derives MTLL rows from validated live CPurchaseContract fields and MTD1 price extraction; save_oracle/parse_market.py and replay/store.py normalize purchase_contract rows into diplomacy.lend_lease entries",
            fail_closed_badge: "do not claim full lend-lease replay until delivery progress, market stockpile, price/speed, and relation context joins are registered",
            proposed_record_kind: RESIDUAL_RECORD_KIND_LEND_LEASE_MAPPING,
            proposed_record: "lend_lease_market_contract_mapping",
            owner_boundary: RESIDUAL_OWNER_MARKET_TRADE,
            owner_label: "market_trade_live",
            root_status: RESIDUAL_ROOT_VALIDATED,
            proof_status: RESIDUAL_PROOF_LIVE_870EBF,
        },
    ]
}

pub fn residual_badge_rows(
    badges: &[MarketTradeResidualBadge],
) -> Vec<MarketTradeResidualBadgeRow> {
    badges
        .iter()
        .map(|badge| MarketTradeResidualBadgeRow {
            gap_id: badge.gap_id,
            status: badge.status,
            evidence_mask: badge.evidence_mask,
            current_support: badge.current_support,
            current_domain_id: match badge.name {
                "purchase_contract_dynamic_tag_buyer_identity"
                | "omitted_candidate_classification"
                | "purchase_contract_price_detail"
                | "lend_lease_market_contract_mapping" => DOMAIN_ID as u32,
                "market_access_diplomatic_context" => 53,
                "license_production_contract_detail" => 53,
                _ => 0,
            },
            proposed_record_kind: badge.proposed_record_kind,
            current_row_len: match badge.name {
                "purchase_contract_dynamic_tag_buyer_identity"
                | "omitted_candidate_classification"
                | "purchase_contract_price_detail" => MARKET_TRADE_ROW_LEN as u32,
                "lend_lease_market_contract_mapping" => MARKET_TRADE_LEND_LEASE_ROW_LEN as u32,
                _ => 0,
            },
            source_ref_id: badge.gap_id,
            owner_boundary: badge.owner_boundary,
            root_status: badge.root_status,
            proof_status: badge.proof_status,
        })
        .collect()
}

pub fn encode_residual_trailer(rows: &[MarketTradeResidualBadgeRow]) -> Vec<u8> {
    let row_count = rows.len().min(u32::MAX as usize) as u32;
    let source_blocked = rows
        .iter()
        .filter(|row| row.status == RESIDUAL_STATUS_SOURCE_BLOCKED)
        .count()
        .min(u32::MAX as usize) as u32;
    let schema_blocked = rows
        .iter()
        .filter(|row| row.status == RESIDUAL_STATUS_SCHEMA_BLOCKED)
        .count()
        .min(u32::MAX as usize) as u32;
    let owned_elsewhere = rows
        .iter()
        .filter(|row| row.status == RESIDUAL_STATUS_OWNED_ELSEWHERE_CONTEXT)
        .count()
        .min(u32::MAX as usize) as u32;
    let shared_patch_blocked = rows
        .iter()
        .filter(|row| row.status == RESIDUAL_STATUS_SHARED_PATCH_BLOCKED)
        .count()
        .min(u32::MAX as usize) as u32;

    let mut payload = Vec::with_capacity(
        MARKET_TRADE_RESIDUAL_HEADER_LEN + rows.len() * MARKET_TRADE_RESIDUAL_ROW_LEN,
    );
    payload.extend_from_slice(MARKET_TRADE_RESIDUAL_TRAILER_MAGIC);
    payload.extend_from_slice(&MARKET_TRADE_RESIDUAL_SCHEMA_VERSION.to_le_bytes());
    payload.extend_from_slice(&(MARKET_TRADE_RESIDUAL_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(MARKET_TRADE_RESIDUAL_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&source_blocked.to_le_bytes());
    payload.extend_from_slice(&schema_blocked.to_le_bytes());
    payload.extend_from_slice(&owned_elsewhere.to_le_bytes());
    payload.extend_from_slice(&shared_patch_blocked.to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.gap_id.to_le_bytes());
        payload.extend_from_slice(&row.status.to_le_bytes());
        payload.extend_from_slice(&row.evidence_mask.to_le_bytes());
        payload.extend_from_slice(&row.current_support.to_le_bytes());
        payload.extend_from_slice(&row.current_domain_id.to_le_bytes());
        payload.extend_from_slice(&row.proposed_record_kind.to_le_bytes());
        payload.extend_from_slice(&row.current_row_len.to_le_bytes());
        payload.extend_from_slice(&row.source_ref_id.to_le_bytes());
        payload.extend_from_slice(&row.owner_boundary.to_le_bytes());
        payload.extend_from_slice(&row.root_status.to_le_bytes());
        payload.extend_from_slice(&row.proof_status.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    payload
}

pub fn encode_detail_trailer(rows: &[PurchaseContractDetailRow]) -> Vec<u8> {
    let row_count = rows.len().min(u32::MAX as usize) as u32;
    let price_valid = rows
        .iter()
        .filter(|row| row.flags & DETAIL_FLAG_PRICE_VALID != 0)
        .count()
        .min(u32::MAX as usize) as u32;
    let speed_valid = rows
        .iter()
        .filter(|row| row.flags & DETAIL_FLAG_SPEED_VALID != 0)
        .count()
        .min(u32::MAX as usize) as u32;
    let delivery_route_valid = rows
        .iter()
        .filter(|row| {
            row.flags
                & (DETAIL_FLAG_DELIVERY_COUNTRY_VALID
                    | DETAIL_FLAG_CONVOYS_VALID
                    | DETAIL_FLAG_EFFICIENCY_VALID)
                != 0
        })
        .count()
        .min(u32::MAX as usize) as u32;
    let progress_valid = rows
        .iter()
        .filter(|row| {
            row.flags & (DETAIL_FLAG_FACTORY_PROGRESS_VALID | DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID)
                != 0
        })
        .count()
        .min(u32::MAX as usize) as u32;

    let mut payload = Vec::with_capacity(
        MARKET_TRADE_DETAIL_HEADER_LEN + rows.len() * MARKET_TRADE_DETAIL_ROW_LEN,
    );
    payload.extend_from_slice(MARKET_TRADE_DETAIL_TRAILER_MAGIC);
    payload.extend_from_slice(&MARKET_TRADE_DETAIL_SCHEMA_VERSION.to_le_bytes());
    payload.extend_from_slice(&(MARKET_TRADE_DETAIL_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(MARKET_TRADE_DETAIL_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&price_valid.to_le_bytes());
    payload.extend_from_slice(&speed_valid.to_le_bytes());
    payload.extend_from_slice(&delivery_route_valid.to_le_bytes());
    payload.extend_from_slice(&progress_valid.to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.contract_id.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.price_raw.to_le_bytes());
        payload.extend_from_slice(&row.amount_raw.to_le_bytes());
        payload.extend_from_slice(&row.speed_raw.to_le_bytes());
        payload.extend_from_slice(&row.completed_raw.to_le_bytes());
        payload.extend_from_slice(&row.days.to_le_bytes());
        payload.extend_from_slice(&row.delivery_country_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.convoys_assigned.to_le_bytes());
        payload.extend_from_slice(&row.convoys_total.to_le_bytes());
        payload.extend_from_slice(&row.efficiency_raw.to_le_bytes());
        payload.extend_from_slice(&row.factory_cic_progress_raw.to_le_bytes());
        payload.extend_from_slice(&row.equipment_transfer_progress_raw.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    payload
}

pub fn encode_lend_lease_trailer(rows: &[LendLeaseMappingRow]) -> Vec<u8> {
    let row_count = rows.len().min(u32::MAX as usize) as u32;
    let outgoing = rows
        .iter()
        .filter(|row| row.direction == LEND_LEASE_DIRECTION_OUTGOING)
        .count()
        .min(u32::MAX as usize) as u32;
    let incoming = rows
        .iter()
        .filter(|row| row.direction == LEND_LEASE_DIRECTION_INCOMING)
        .count()
        .min(u32::MAX as usize) as u32;
    let amount_valid = rows
        .iter()
        .filter(|row| row.flags & LEND_LEASE_FLAG_AMOUNT_VALID != 0)
        .count()
        .min(u32::MAX as usize) as u32;
    let price_valid = rows
        .iter()
        .filter(|row| row.flags & LEND_LEASE_FLAG_PRICE_VALID != 0)
        .count()
        .min(u32::MAX as usize) as u32;

    let mut payload = Vec::with_capacity(
        MARKET_TRADE_LEND_LEASE_HEADER_LEN + rows.len() * MARKET_TRADE_LEND_LEASE_ROW_LEN,
    );
    payload.extend_from_slice(MARKET_TRADE_LEND_LEASE_TRAILER_MAGIC);
    payload.extend_from_slice(&MARKET_TRADE_LEND_LEASE_SCHEMA_VERSION.to_le_bytes());
    payload.extend_from_slice(&(MARKET_TRADE_LEND_LEASE_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(MARKET_TRADE_LEND_LEASE_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&outgoing.to_le_bytes());
    payload.extend_from_slice(&incoming.to_le_bytes());
    payload.extend_from_slice(&amount_valid.to_le_bytes());
    payload.extend_from_slice(&price_valid.to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.contract_id.to_le_bytes());
        payload.extend_from_slice(&row.perspective_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.counterpart_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.direction.to_le_bytes());
        payload.extend_from_slice(&row.equipment_variant_id.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.amount_raw.to_le_bytes());
        payload.extend_from_slice(&row.price_raw.to_le_bytes());
        payload.extend_from_slice(&row.source_record_kind.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    payload
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> MarketTradePayloadSummary {
    let mut summary = MarketTradePayloadSummary::default();
    if payload.len() < MARKET_TRADE_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: market_trade_live payload is {} byte(s), expected at least {MARKET_TRADE_HEADER_LEN}",
            payload.len()
        ));
        return summary;
    }

    summary.elapsed_wall_ms = read_u64_le(payload, 0);
    summary.row_count = read_u32_le(payload, 8);
    summary.changed_rows = read_u32_le(payload, 12);
    summary.status_flags = read_u32_le(payload, 16);
    summary.observed_contract_rows = read_u32_le(payload, 20);
    summary.emitted_contract_rows = read_u32_le(payload, 24);
    summary.omitted_candidate_rows = summary
        .observed_contract_rows
        .saturating_sub(summary.emitted_contract_rows);
    summary.buyer_ptr_valid_rows = read_u32_le(payload, 28);
    summary.variant_valid_rows = read_u32_le(payload, 32);
    summary.amount_valid_rows = read_u32_le(payload, 36);
    summary.partial_field_rows = read_u32_le(payload, 40);
    summary.scale_raw = read_u32_le(payload, 44);

    if summary.changed_rows > summary.row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: changed_rows {} > row_count {}",
            summary.changed_rows, summary.row_count
        ));
    }
    if summary.emitted_contract_rows != summary.row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: emitted_contract_rows {} != row_count {}",
            summary.emitted_contract_rows, summary.row_count
        ));
    }
    if summary.observed_contract_rows < summary.row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: observed_contract_rows {} < row_count {}",
            summary.observed_contract_rows, summary.row_count
        ));
    }
    if summary.scale_raw != MARKET_TRADE_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: scale_raw {} != {MARKET_TRADE_SCALE_RAW}",
            summary.scale_raw
        ));
    }
    for (name, count) in [
        ("buyer_ptr_valid_rows", summary.buyer_ptr_valid_rows),
        ("variant_valid_rows", summary.variant_valid_rows),
        ("amount_valid_rows", summary.amount_valid_rows),
        ("partial_field_rows", summary.partial_field_rows),
    ] {
        if count > summary.row_count {
            summary.parse_errors.push(format!(
                "offset {offset}: {name} {count} > row_count {}",
                summary.row_count
            ));
        }
    }

    let expected_rows_len = MARKET_TRADE_HEADER_LEN
        .saturating_add((summary.row_count as usize).saturating_mul(MARKET_TRADE_ROW_LEN));
    if payload.len() < expected_rows_len {
        summary.parse_errors.push(format!(
            "offset {offset}: market_trade_live payload is {} byte(s), expected at least {expected_rows_len}",
            payload.len()
        ));
        return summary;
    }

    let mut seen_contracts = BTreeSet::new();
    let mut computed_buyer = 0u32;
    let mut computed_variant = 0u32;
    let mut computed_amount = 0u32;
    let mut computed_partial = 0u32;
    let mut cursor = MARKET_TRADE_HEADER_LEN;
    for row_index in 0..summary.row_count {
        let row = MarketTradeRow {
            contract_id: read_u32_le(payload, cursor),
            contract_type: read_u32_le(payload, cursor + 4),
            flags: read_u32_le(payload, cursor + 8),
            seller_tag_db_id: read_u32_le(payload, cursor + 12),
            buyer_tag_db_id: read_u32_le(payload, cursor + 16),
            equipment_variant_id: read_u32_le(payload, cursor + 20),
            amount_raw: read_i64_le(payload, cursor + 24),
            equipment_subbuf_status: read_u32_le(payload, cursor + 32),
            delivery_subbuf_status: read_u32_le(payload, cursor + 36),
            buyer_ptr_status: read_u32_le(payload, cursor + 40),
        };
        let reserved = read_u32_le(payload, cursor + 44);
        cursor += MARKET_TRADE_ROW_LEN;

        let buyer_valid = row.flags & ROW_FLAG_BUYER_PTR_VALID != 0;
        let equipment_valid = row.flags & ROW_FLAG_EQUIPMENT_SUBBUF_VALID != 0;
        let delivery_valid = row.flags & ROW_FLAG_DELIVERY_SUBBUF_VALID != 0;
        let variant_valid = row.flags & ROW_FLAG_VARIANT_VALID != 0;
        let amount_valid = row.flags & ROW_FLAG_AMOUNT_VALID != 0;
        computed_buyer += u32::from(buyer_valid);
        computed_variant += u32::from(variant_valid);
        computed_amount += u32::from(amount_valid);
        computed_partial += u32::from(
            !(buyer_valid && equipment_valid && delivery_valid && variant_valid && amount_valid),
        );

        validate_row(
            offset,
            row_index as usize,
            &row,
            reserved,
            &mut summary.parse_errors,
        );
        if !seen_contracts.insert(row.contract_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade row {row_index} duplicates contract_id {}",
                row.contract_id
            ));
        }
        summary.rows.push(row);
    }

    if payload.len() > expected_rows_len {
        decode_optional_trailer(offset, payload, expected_rows_len, &mut summary);
    }

    for (name, expected, computed) in [
        (
            "buyer_ptr_valid_rows",
            summary.buyer_ptr_valid_rows,
            computed_buyer,
        ),
        (
            "variant_valid_rows",
            summary.variant_valid_rows,
            computed_variant,
        ),
        (
            "amount_valid_rows",
            summary.amount_valid_rows,
            computed_amount,
        ),
        (
            "partial_field_rows",
            summary.partial_field_rows,
            computed_partial,
        ),
    ] {
        if expected != computed {
            summary.parse_errors.push(format!(
                "offset {offset}: {name} header {expected} != decoded {computed}"
            ));
        }
    }
    summary
}

fn decode_optional_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut MarketTradePayloadSummary,
) {
    if payload.len().saturating_sub(trailer_offset) < 4 {
        summary.parse_errors.push(format!(
            "offset {offset}: optional trailer is too short to contain magic"
        ));
        return;
    }
    let magic = &payload[trailer_offset..trailer_offset + 4];
    if magic == MARKET_TRADE_RESIDUAL_TRAILER_MAGIC {
        decode_residual_trailer(offset, payload, trailer_offset, summary);
    } else if magic == MARKET_TRADE_DETAIL_TRAILER_MAGIC {
        decode_detail_trailer(offset, payload, trailer_offset, summary);
    } else if magic == MARKET_TRADE_LEND_LEASE_TRAILER_MAGIC {
        decode_lend_lease_trailer(offset, payload, trailer_offset, summary);
    } else {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade optional trailer has unknown magic"
        ));
    }
}

fn decode_residual_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut MarketTradePayloadSummary,
) {
    summary.residual_trailer_present = true;
    if payload.len().saturating_sub(trailer_offset) < MARKET_TRADE_RESIDUAL_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: residual trailer is {} byte(s), expected at least {MARKET_TRADE_RESIDUAL_HEADER_LEN}",
            payload.len().saturating_sub(trailer_offset)
        ));
        return;
    }
    let trailer = &payload[trailer_offset..];
    if &trailer[0..4] != MARKET_TRADE_RESIDUAL_TRAILER_MAGIC {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade residual trailer has invalid magic"
        ));
        return;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_len = read_u16_le(trailer, 8) as usize;
    let reserved0 = read_u16_le(trailer, 10);
    let row_count = read_u32_le(trailer, 12);
    let source_blocked = read_u32_le(trailer, 16);
    let schema_blocked = read_u32_le(trailer, 20);
    let owned_elsewhere = read_u32_le(trailer, 24);
    let shared_patch_blocked = read_u32_le(trailer, 28);

    summary.residual_badge_rows = row_count;
    summary.residual_source_blocked_rows = source_blocked;
    summary.residual_schema_blocked_rows = schema_blocked;
    summary.residual_owned_elsewhere_rows = owned_elsewhere;
    summary.residual_shared_patch_blocked_rows = shared_patch_blocked;

    if version != MARKET_TRADE_RESIDUAL_SCHEMA_VERSION
        || header_len != MARKET_TRADE_RESIDUAL_HEADER_LEN
        || row_len != MARKET_TRADE_RESIDUAL_ROW_LEN
    {
        summary.parse_errors.push(format!(
            "offset {offset}: unsupported residual trailer version {version} header_len {header_len} row_len {row_len}"
        ));
        return;
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: residual trailer reserved header field is nonzero"
        ));
    }
    let expected_len = MARKET_TRADE_RESIDUAL_HEADER_LEN
        .saturating_add((row_count as usize).saturating_mul(MARKET_TRADE_RESIDUAL_ROW_LEN));
    if trailer.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: residual trailer is {} byte(s), expected {expected_len}",
            trailer.len()
        ));
        return;
    }

    let mut cursor = MARKET_TRADE_RESIDUAL_HEADER_LEN;
    let mut seen = BTreeSet::new();
    let mut computed_source = 0u32;
    let mut computed_schema = 0u32;
    let mut computed_owned = 0u32;
    let mut computed_shared = 0u32;
    for row_index in 0..row_count {
        let row = MarketTradeResidualBadgeRow {
            gap_id: read_u32_le(trailer, cursor),
            status: read_u32_le(trailer, cursor + 4),
            evidence_mask: read_u32_le(trailer, cursor + 8),
            current_support: read_u32_le(trailer, cursor + 12),
            current_domain_id: read_u32_le(trailer, cursor + 16),
            proposed_record_kind: read_u32_le(trailer, cursor + 20),
            current_row_len: read_u32_le(trailer, cursor + 24),
            source_ref_id: read_u32_le(trailer, cursor + 28),
            owner_boundary: read_u32_le(trailer, cursor + 32),
            root_status: read_u32_le(trailer, cursor + 36),
            proof_status: read_u32_le(trailer, cursor + 40),
        };
        let reserved = read_u32_le(trailer, cursor + 44);
        cursor += MARKET_TRADE_RESIDUAL_ROW_LEN;

        validate_residual_badge_row(offset, row_index as usize, &row, reserved, summary);
        if row.gap_id != 0 && !seen.insert(row.gap_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: residual badge row {row_index} duplicates gap_id {}",
                row.gap_id
            ));
        }
        match row.status {
            RESIDUAL_STATUS_SOURCE_BLOCKED => computed_source += 1,
            RESIDUAL_STATUS_SCHEMA_BLOCKED => computed_schema += 1,
            RESIDUAL_STATUS_OWNED_ELSEWHERE_CONTEXT => computed_owned += 1,
            RESIDUAL_STATUS_SHARED_PATCH_BLOCKED => computed_shared += 1,
            _ => {}
        }
        summary.residual_badges.push(row);
    }
    for (name, expected, computed) in [
        (
            "residual_source_blocked_rows",
            source_blocked,
            computed_source,
        ),
        (
            "residual_schema_blocked_rows",
            schema_blocked,
            computed_schema,
        ),
        (
            "residual_owned_elsewhere_rows",
            owned_elsewhere,
            computed_owned,
        ),
        (
            "residual_shared_patch_blocked_rows",
            shared_patch_blocked,
            computed_shared,
        ),
    ] {
        if expected != computed {
            summary.parse_errors.push(format!(
                "offset {offset}: {name} header {expected} != decoded {computed}"
            ));
        }
    }
}

fn decode_detail_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut MarketTradePayloadSummary,
) {
    summary.detail_trailer_present = true;
    if payload.len().saturating_sub(trailer_offset) < MARKET_TRADE_DETAIL_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: detail trailer is {} byte(s), expected at least {MARKET_TRADE_DETAIL_HEADER_LEN}",
            payload.len().saturating_sub(trailer_offset)
        ));
        return;
    }
    let trailer = &payload[trailer_offset..];
    if &trailer[0..4] != MARKET_TRADE_DETAIL_TRAILER_MAGIC {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade detail trailer has invalid magic"
        ));
        return;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_len = read_u16_le(trailer, 8) as usize;
    let reserved0 = read_u16_le(trailer, 10);
    let row_count = read_u32_le(trailer, 12);
    let price_valid = read_u32_le(trailer, 16);
    let speed_valid = read_u32_le(trailer, 20);
    let delivery_route_valid = read_u32_le(trailer, 24);
    let progress_valid = read_u32_le(trailer, 28);

    summary.detail_rows = row_count;
    summary.detail_price_valid_rows = price_valid;
    summary.detail_speed_valid_rows = speed_valid;
    summary.detail_delivery_route_valid_rows = delivery_route_valid;
    summary.detail_progress_valid_rows = progress_valid;

    if version != MARKET_TRADE_DETAIL_SCHEMA_VERSION
        || header_len != MARKET_TRADE_DETAIL_HEADER_LEN
        || row_len != MARKET_TRADE_DETAIL_ROW_LEN
    {
        summary.parse_errors.push(format!(
            "offset {offset}: unsupported detail trailer version {version} header_len {header_len} row_len {row_len}"
        ));
        return;
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: detail trailer reserved header field is nonzero"
        ));
    }
    let expected_len = MARKET_TRADE_DETAIL_HEADER_LEN
        .saturating_add((row_count as usize).saturating_mul(MARKET_TRADE_DETAIL_ROW_LEN));
    if trailer.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: detail trailer is {} byte(s), expected {expected_len}",
            trailer.len()
        ));
        return;
    }

    let mut cursor = MARKET_TRADE_DETAIL_HEADER_LEN;
    let mut seen = BTreeSet::new();
    let mut computed_price = 0u32;
    let mut computed_speed = 0u32;
    let mut computed_completed = 0u32;
    let mut computed_days = 0u32;
    let mut computed_delivery_country = 0u32;
    let mut computed_convoys = 0u32;
    let mut computed_efficiency = 0u32;
    let mut computed_route = 0u32;
    let mut computed_progress = 0u32;
    for row_index in 0..row_count {
        let row = PurchaseContractDetailRow {
            contract_id: read_u32_le(trailer, cursor),
            flags: read_u32_le(trailer, cursor + 4),
            price_raw: read_i64_le(trailer, cursor + 8),
            amount_raw: read_i64_le(trailer, cursor + 16),
            speed_raw: read_i32_le(trailer, cursor + 24),
            completed_raw: read_i32_le(trailer, cursor + 28),
            days: read_i32_le(trailer, cursor + 32),
            delivery_country_tag_db_id: read_u32_le(trailer, cursor + 36),
            convoys_assigned: read_i32_le(trailer, cursor + 40),
            convoys_total: read_i32_le(trailer, cursor + 44),
            efficiency_raw: read_i32_le(trailer, cursor + 48),
            factory_cic_progress_raw: read_i32_le(trailer, cursor + 52),
            equipment_transfer_progress_raw: read_i32_le(trailer, cursor + 56),
        };
        let reserved = read_u32_le(trailer, cursor + 60);
        cursor += MARKET_TRADE_DETAIL_ROW_LEN;

        validate_detail_row(offset, row_index as usize, &row, reserved, summary);
        if row.contract_id != 0 && !seen.insert(row.contract_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: detail row {row_index} duplicates contract_id {}",
                row.contract_id
            ));
        }
        computed_price += u32::from(row.flags & DETAIL_FLAG_PRICE_VALID != 0);
        computed_speed += u32::from(row.flags & DETAIL_FLAG_SPEED_VALID != 0);
        computed_completed += u32::from(row.flags & DETAIL_FLAG_COMPLETED_VALID != 0);
        computed_days += u32::from(row.flags & DETAIL_FLAG_DAYS_VALID != 0);
        computed_delivery_country += u32::from(row.flags & DETAIL_FLAG_DELIVERY_COUNTRY_VALID != 0);
        computed_convoys += u32::from(row.flags & DETAIL_FLAG_CONVOYS_VALID != 0);
        computed_efficiency += u32::from(row.flags & DETAIL_FLAG_EFFICIENCY_VALID != 0);
        computed_route += u32::from(
            row.flags
                & (DETAIL_FLAG_DELIVERY_COUNTRY_VALID
                    | DETAIL_FLAG_CONVOYS_VALID
                    | DETAIL_FLAG_EFFICIENCY_VALID)
                != 0,
        );
        computed_progress += u32::from(
            row.flags & (DETAIL_FLAG_FACTORY_PROGRESS_VALID | DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID)
                != 0,
        );
        summary.detail_contracts.push(row);
    }
    summary.detail_completed_valid_rows = computed_completed;
    summary.detail_days_valid_rows = computed_days;
    summary.detail_delivery_country_valid_rows = computed_delivery_country;
    summary.detail_convoys_valid_rows = computed_convoys;
    summary.detail_efficiency_valid_rows = computed_efficiency;
    for (name, expected, computed) in [
        ("detail_price_valid_rows", price_valid, computed_price),
        ("detail_speed_valid_rows", speed_valid, computed_speed),
        (
            "detail_delivery_route_valid_rows",
            delivery_route_valid,
            computed_route,
        ),
        (
            "detail_progress_valid_rows",
            progress_valid,
            computed_progress,
        ),
    ] {
        if expected != computed {
            summary.parse_errors.push(format!(
                "offset {offset}: {name} header {expected} != decoded {computed}"
            ));
        }
    }
}

fn decode_lend_lease_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut MarketTradePayloadSummary,
) {
    summary.lend_lease_trailer_present = true;
    if payload.len().saturating_sub(trailer_offset) < MARKET_TRADE_LEND_LEASE_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: lend-lease trailer is {} byte(s), expected at least {MARKET_TRADE_LEND_LEASE_HEADER_LEN}",
            payload.len().saturating_sub(trailer_offset)
        ));
        return;
    }
    let trailer = &payload[trailer_offset..];
    if &trailer[0..4] != MARKET_TRADE_LEND_LEASE_TRAILER_MAGIC {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade lend-lease trailer has invalid magic"
        ));
        return;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_len = read_u16_le(trailer, 8) as usize;
    let reserved0 = read_u16_le(trailer, 10);
    let row_count = read_u32_le(trailer, 12);
    let outgoing = read_u32_le(trailer, 16);
    let incoming = read_u32_le(trailer, 20);
    let amount_valid = read_u32_le(trailer, 24);
    let price_valid = read_u32_le(trailer, 28);

    summary.lend_lease_mapping_rows = row_count;
    summary.lend_lease_outgoing_rows = outgoing;
    summary.lend_lease_incoming_rows = incoming;
    summary.lend_lease_amount_valid_rows = amount_valid;
    summary.lend_lease_price_valid_rows = price_valid;

    if version != MARKET_TRADE_LEND_LEASE_SCHEMA_VERSION
        || header_len != MARKET_TRADE_LEND_LEASE_HEADER_LEN
        || row_len != MARKET_TRADE_LEND_LEASE_ROW_LEN
    {
        summary.parse_errors.push(format!(
            "offset {offset}: unsupported lend-lease trailer version {version} header_len {header_len} row_len {row_len}"
        ));
        return;
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: lend-lease trailer reserved header field is nonzero"
        ));
    }
    let expected_len = MARKET_TRADE_LEND_LEASE_HEADER_LEN
        .saturating_add((row_count as usize).saturating_mul(MARKET_TRADE_LEND_LEASE_ROW_LEN));
    if trailer.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: lend-lease trailer is {} byte(s), expected {expected_len}",
            trailer.len()
        ));
        return;
    }

    let mut cursor = MARKET_TRADE_LEND_LEASE_HEADER_LEN;
    let mut seen_keys = BTreeSet::new();
    let mut per_contract = BTreeMap::<u32, (u32, u32)>::new();
    let mut computed_outgoing = 0u32;
    let mut computed_incoming = 0u32;
    let mut computed_amount = 0u32;
    let mut computed_price = 0u32;
    for row_index in 0..row_count {
        let row = LendLeaseMappingRow {
            contract_id: read_u32_le(trailer, cursor),
            perspective_tag_db_id: read_u32_le(trailer, cursor + 4),
            counterpart_tag_db_id: read_u32_le(trailer, cursor + 8),
            direction: read_u32_le(trailer, cursor + 12),
            equipment_variant_id: read_u32_le(trailer, cursor + 16),
            flags: read_u32_le(trailer, cursor + 20),
            amount_raw: read_i64_le(trailer, cursor + 24),
            price_raw: read_i64_le(trailer, cursor + 32),
            source_record_kind: read_u32_le(trailer, cursor + 40),
        };
        let reserved = read_u32_le(trailer, cursor + 44);
        cursor += MARKET_TRADE_LEND_LEASE_ROW_LEN;

        validate_lend_lease_row(offset, row_index as usize, &row, reserved, summary);
        if row.contract_id != 0
            && !seen_keys.insert((row.contract_id, row.perspective_tag_db_id, row.direction))
        {
            summary.parse_errors.push(format!(
                "offset {offset}: lend-lease row {row_index} duplicates contract/perspective/direction key"
            ));
        }
        match row.direction {
            LEND_LEASE_DIRECTION_OUTGOING => {
                computed_outgoing += 1;
                per_contract.entry(row.contract_id).or_default().0 += 1;
            }
            LEND_LEASE_DIRECTION_INCOMING => {
                computed_incoming += 1;
                per_contract.entry(row.contract_id).or_default().1 += 1;
            }
            _ => {}
        }
        computed_amount += u32::from(row.flags & LEND_LEASE_FLAG_AMOUNT_VALID != 0);
        computed_price += u32::from(row.flags & LEND_LEASE_FLAG_PRICE_VALID != 0);
        summary.lend_lease_mappings.push(row);
    }
    for (contract_id, (out_rows, in_rows)) in per_contract {
        if out_rows != 1 || in_rows != 1 {
            summary.parse_errors.push(format!(
                "offset {offset}: lend-lease contract_id {contract_id} has outgoing {out_rows} incoming {in_rows}, expected 1/1"
            ));
        }
    }
    for (name, expected, computed) in [
        ("lend_lease_outgoing_rows", outgoing, computed_outgoing),
        ("lend_lease_incoming_rows", incoming, computed_incoming),
        (
            "lend_lease_amount_valid_rows",
            amount_valid,
            computed_amount,
        ),
        ("lend_lease_price_valid_rows", price_valid, computed_price),
    ] {
        if expected != computed {
            summary.parse_errors.push(format!(
                "offset {offset}: {name} header {expected} != decoded {computed}"
            ));
        }
    }
}

fn validate_detail_row(
    offset: usize,
    row_index: usize,
    row: &PurchaseContractDetailRow,
    reserved: u32,
    summary: &mut MarketTradePayloadSummary,
) {
    if row.contract_id == 0 || row.contract_id > MARKET_TRADE_MAX_CONTRACT_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: detail row {row_index} invalid contract_id {}",
            row.contract_id
        ));
    }
    if row.flags & !DETAIL_FLAGS_KNOWN != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: detail row {row_index} unknown flags 0x{:X}",
            row.flags & !DETAIL_FLAGS_KNOWN
        ));
    }
    validate_i64_presence(
        offset,
        row_index,
        "price_raw",
        row.flags & DETAIL_FLAG_PRICE_VALID != 0,
        row.price_raw,
        1,
        MARKET_TRADE_MAX_AMOUNT_RAW,
        summary,
    );
    validate_i64_presence(
        offset,
        row_index,
        "amount_raw",
        row.flags & DETAIL_FLAG_AMOUNT_VALID != 0,
        row.amount_raw,
        1,
        MARKET_TRADE_MAX_AMOUNT_RAW,
        summary,
    );
    validate_i32_presence(
        offset,
        row_index,
        "speed_raw",
        row.flags & DETAIL_FLAG_SPEED_VALID != 0,
        row.speed_raw,
        0,
        100,
        summary,
    );
    validate_i32_presence(
        offset,
        row_index,
        "completed_raw",
        row.flags & DETAIL_FLAG_COMPLETED_VALID != 0,
        row.completed_raw,
        0,
        i32::MAX,
        summary,
    );
    validate_i32_presence(
        offset,
        row_index,
        "days",
        row.flags & DETAIL_FLAG_DAYS_VALID != 0,
        row.days,
        0,
        i32::MAX,
        summary,
    );
    if row.flags & DETAIL_FLAG_DELIVERY_COUNTRY_VALID != 0 {
        if row.delivery_country_tag_db_id == 0
            || row.delivery_country_tag_db_id > MARKET_TRADE_MAX_TAG_DB_ID
        {
            summary.parse_errors.push(format!(
                "offset {offset}: detail row {row_index} invalid delivery_country_tag_db_id {}",
                row.delivery_country_tag_db_id
            ));
        }
    } else if row.delivery_country_tag_db_id != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: detail row {row_index} has delivery country without valid flag"
        ));
    }
    validate_i32_presence(
        offset,
        row_index,
        "convoys_assigned",
        row.flags & DETAIL_FLAG_CONVOYS_VALID != 0,
        row.convoys_assigned,
        0,
        1_000_000,
        summary,
    );
    validate_i32_presence(
        offset,
        row_index,
        "convoys_total",
        row.flags & DETAIL_FLAG_CONVOYS_VALID != 0,
        row.convoys_total,
        0,
        1_000_000,
        summary,
    );
    validate_i32_presence(
        offset,
        row_index,
        "efficiency_raw",
        row.flags & DETAIL_FLAG_EFFICIENCY_VALID != 0,
        row.efficiency_raw,
        0,
        MARKET_TRADE_MAX_PROGRESS_RAW,
        summary,
    );
    validate_i32_presence(
        offset,
        row_index,
        "factory_cic_progress_raw",
        row.flags & DETAIL_FLAG_FACTORY_PROGRESS_VALID != 0,
        row.factory_cic_progress_raw,
        0,
        MARKET_TRADE_MAX_PROGRESS_RAW,
        summary,
    );
    validate_i32_presence(
        offset,
        row_index,
        "equipment_transfer_progress_raw",
        row.flags & DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID != 0,
        row.equipment_transfer_progress_raw,
        0,
        MARKET_TRADE_MAX_PROGRESS_RAW,
        summary,
    );
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: detail row {row_index} reserved field is nonzero"
        ));
    }
}

fn validate_lend_lease_row(
    offset: usize,
    row_index: usize,
    row: &LendLeaseMappingRow,
    reserved: u32,
    summary: &mut MarketTradePayloadSummary,
) {
    if row.contract_id == 0 || row.contract_id > MARKET_TRADE_MAX_CONTRACT_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: lend-lease row {row_index} invalid contract_id {}",
            row.contract_id
        ));
    }
    for (name, tag_db_id) in [
        ("perspective_tag_db_id", row.perspective_tag_db_id),
        ("counterpart_tag_db_id", row.counterpart_tag_db_id),
    ] {
        if tag_db_id == 0 || tag_db_id > MARKET_TRADE_MAX_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: lend-lease row {row_index} invalid {name} {tag_db_id}"
            ));
        }
    }
    if row.perspective_tag_db_id == row.counterpart_tag_db_id {
        summary.parse_errors.push(format!(
            "offset {offset}: lend-lease row {row_index} perspective equals counterpart"
        ));
    }
    if row.direction != LEND_LEASE_DIRECTION_OUTGOING
        && row.direction != LEND_LEASE_DIRECTION_INCOMING
    {
        summary.parse_errors.push(format!(
            "offset {offset}: lend-lease row {row_index} invalid direction {}",
            row.direction
        ));
    }
    if row.equipment_variant_id == 0 || row.equipment_variant_id > MARKET_TRADE_MAX_VARIANT_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: lend-lease row {row_index} invalid equipment_variant_id {}",
            row.equipment_variant_id
        ));
    }
    if row.flags & !LEND_LEASE_FLAGS_KNOWN != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: lend-lease row {row_index} unknown flags 0x{:X}",
            row.flags & !LEND_LEASE_FLAGS_KNOWN
        ));
    }
    validate_i64_presence(
        offset,
        row_index,
        "lend_lease.amount_raw",
        row.flags & LEND_LEASE_FLAG_AMOUNT_VALID != 0,
        row.amount_raw,
        1,
        MARKET_TRADE_MAX_AMOUNT_RAW,
        summary,
    );
    validate_i64_presence(
        offset,
        row_index,
        "lend_lease.price_raw",
        row.flags & LEND_LEASE_FLAG_PRICE_VALID != 0,
        row.price_raw,
        1,
        MARKET_TRADE_MAX_AMOUNT_RAW,
        summary,
    );
    if row.source_record_kind != LEND_LEASE_SOURCE_PURCHASE_CONTRACT {
        summary.parse_errors.push(format!(
            "offset {offset}: lend-lease row {row_index} invalid source_record_kind {}",
            row.source_record_kind
        ));
    }
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: lend-lease row {row_index} reserved field is nonzero"
        ));
    }
}

fn validate_i64_presence(
    offset: usize,
    row_index: usize,
    name: &str,
    present: bool,
    value: i64,
    min_value: i64,
    max_value: i64,
    summary: &mut MarketTradePayloadSummary,
) {
    if present {
        if value < min_value || value > max_value {
            summary.parse_errors.push(format!(
                "offset {offset}: detail row {row_index} invalid {name} {value}"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: detail row {row_index} has {name} without valid flag"
        ));
    }
}

fn validate_i32_presence(
    offset: usize,
    row_index: usize,
    name: &str,
    present: bool,
    value: i32,
    min_value: i32,
    max_value: i32,
    summary: &mut MarketTradePayloadSummary,
) {
    if present {
        if value < min_value || value > max_value {
            summary.parse_errors.push(format!(
                "offset {offset}: detail row {row_index} invalid {name} {value}"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: detail row {row_index} has {name} without valid flag"
        ));
    }
}

fn validate_residual_badge_row(
    offset: usize,
    row_index: usize,
    row: &MarketTradeResidualBadgeRow,
    reserved: u32,
    summary: &mut MarketTradePayloadSummary,
) {
    if row.gap_id == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: residual badge row {row_index} has zero gap_id"
        ));
    }
    if !matches!(
        row.status,
        RESIDUAL_STATUS_VALIDATED_SLICE
            | RESIDUAL_STATUS_SCHEMA_BLOCKED
            | RESIDUAL_STATUS_SOURCE_BLOCKED
            | RESIDUAL_STATUS_OWNED_ELSEWHERE_CONTEXT
            | RESIDUAL_STATUS_SHARED_PATCH_BLOCKED
    ) {
        summary.parse_errors.push(format!(
            "offset {offset}: residual badge row {row_index} has unknown status {}",
            row.status
        ));
    }
    if row.evidence_mask == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: residual badge row {row_index} has no evidence bits"
        ));
    }
    if !matches!(
        row.current_support,
        RESIDUAL_SUPPORT_NONE
            | RESIDUAL_SUPPORT_PARTIAL
            | RESIDUAL_SUPPORT_VALIDATED_SLICE
            | RESIDUAL_SUPPORT_OWNED_ELSEWHERE
            | RESIDUAL_SUPPORT_SHARED_PATCH_PENDING
    ) {
        summary.parse_errors.push(format!(
            "offset {offset}: residual badge row {row_index} has unknown current_support {}",
            row.current_support
        ));
    }
    if !matches!(row.current_domain_id, 0 | 11 | 17 | 43 | 53) {
        summary.parse_errors.push(format!(
            "offset {offset}: residual badge row {row_index} has unknown current_domain_id {}",
            row.current_domain_id
        ));
    }
    if !known_residual_record_kind(row.proposed_record_kind) {
        summary.parse_errors.push(format!(
            "offset {offset}: residual badge row {row_index} has unknown proposed_record_kind {}",
            row.proposed_record_kind
        ));
    }
    if row.current_row_len > 512 {
        summary.parse_errors.push(format!(
            "offset {offset}: residual badge row {row_index} has implausible current_row_len {}",
            row.current_row_len
        ));
    }
    if row.source_ref_id == 0 || row.source_ref_id != row.gap_id {
        summary.parse_errors.push(format!(
            "offset {offset}: residual badge row {row_index} has invalid source_ref_id {}",
            row.source_ref_id
        ));
    }
    if !matches!(
        row.owner_boundary,
        RESIDUAL_OWNER_MARKET_TRADE
            | RESIDUAL_OWNER_SHARED_COUNTRY_IDENTITY
            | RESIDUAL_OWNER_DIPLOMACY_INTEL
            | RESIDUAL_OWNER_ECONOMY_PRODUCTION
    ) {
        summary.parse_errors.push(format!(
            "offset {offset}: residual badge row {row_index} has unknown owner_boundary {}",
            row.owner_boundary
        ));
    }
    if !matches!(
        row.root_status,
        RESIDUAL_ROOT_VALIDATED
            | RESIDUAL_ROOT_UNVALIDATED
            | RESIDUAL_ROOT_SCHEMA_ONLY
            | RESIDUAL_ROOT_OWNED_ELSEWHERE
            | RESIDUAL_ROOT_REJECTED_CANDIDATE
    ) {
        summary.parse_errors.push(format!(
            "offset {offset}: residual badge row {row_index} has unknown root_status {}",
            row.root_status
        ));
    }
    if !matches!(
        row.proof_status,
        RESIDUAL_PROOF_LIVE_870EBF
            | RESIDUAL_PROOF_STATIC_ORACLE
            | RESIDUAL_PROOF_REGISTRATION_PENDING
            | RESIDUAL_PROOF_NOT_PROVEN_LIVE
    ) {
        summary.parse_errors.push(format!(
            "offset {offset}: residual badge row {row_index} has unknown proof_status {}",
            row.proof_status
        ));
    }
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: residual badge row {row_index} reserved field is nonzero"
        ));
    }
}

fn known_residual_record_kind(kind: u32) -> bool {
    matches!(
        kind,
        RESIDUAL_RECORD_KIND_PURCHASE_CONTRACT_CORE
            | RESIDUAL_RECORD_KIND_PURCHASE_CONTRACT_DELIVERY
            | RESIDUAL_RECORD_KIND_MARKET_STOCKPILE
            | RESIDUAL_RECORD_KIND_COUNTRY_RESOURCE_FLOW
            | RESIDUAL_RECORD_KIND_DIPLOMATIC_TRADE_CONTEXT
            | RESIDUAL_RECORD_KIND_LICENSE_PRODUCTION
            | RESIDUAL_RECORD_KIND_LEND_LEASE_MAPPING
            | RESIDUAL_RECORD_KIND_RESIDUAL_BADGE
    )
}

fn validate_row(
    offset: usize,
    row_index: usize,
    row: &MarketTradeRow,
    reserved: u32,
    errors: &mut Vec<String>,
) {
    if row.contract_id == 0 || row.contract_id > MARKET_TRADE_MAX_CONTRACT_ID {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} invalid contract_id {}",
            row.contract_id
        ));
    }
    if row.contract_type != MARKET_TRADE_CONTRACT_TYPE {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} contract_type {} != {MARKET_TRADE_CONTRACT_TYPE}",
            row.contract_type
        ));
    }
    for (name, tag_db_id) in [
        ("seller_tag_db_id", row.seller_tag_db_id),
        ("buyer_tag_db_id", row.buyer_tag_db_id),
    ] {
        if tag_db_id == 0 || tag_db_id > MARKET_TRADE_MAX_TAG_DB_ID {
            errors.push(format!(
                "offset {offset}: market trade row {row_index} invalid {name} {tag_db_id}"
            ));
        }
    }
    if row.flags & !ROW_FLAGS_KNOWN != 0 {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} unknown flags 0x{:X}",
            row.flags & !ROW_FLAGS_KNOWN
        ));
    }
    if reserved != 0 {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} reserved field is nonzero"
        ));
    }
    validate_status_pair(
        offset,
        row_index,
        "buyer_ptr",
        row.flags & ROW_FLAG_BUYER_PTR_VALID != 0,
        row.buyer_ptr_status,
        errors,
    );
    validate_status_pair(
        offset,
        row_index,
        "equipment_subbuf",
        row.flags & ROW_FLAG_EQUIPMENT_SUBBUF_VALID != 0,
        row.equipment_subbuf_status,
        errors,
    );
    validate_status_pair(
        offset,
        row_index,
        "delivery_subbuf",
        row.flags & ROW_FLAG_DELIVERY_SUBBUF_VALID != 0,
        row.delivery_subbuf_status,
        errors,
    );

    if row.flags & ROW_FLAG_VARIANT_VALID != 0 {
        if row.equipment_variant_id == 0 || row.equipment_variant_id > MARKET_TRADE_MAX_VARIANT_ID {
            errors.push(format!(
                "offset {offset}: market trade row {row_index} invalid equipment_variant_id {}",
                row.equipment_variant_id
            ));
        }
    } else if row.equipment_variant_id != 0 {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} has variant id without valid flag"
        ));
    }
    if row.flags & ROW_FLAG_AMOUNT_VALID != 0 {
        if row.amount_raw <= 0 || row.amount_raw > MARKET_TRADE_MAX_AMOUNT_RAW {
            errors.push(format!(
                "offset {offset}: market trade row {row_index} invalid amount_raw {}",
                row.amount_raw
            ));
        }
    } else if row.amount_raw != 0 {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} has amount without valid flag"
        ));
    }
}

fn validate_status_pair(
    offset: usize,
    row_index: usize,
    name: &str,
    valid_flag: bool,
    status: u32,
    errors: &mut Vec<String>,
) {
    if status > FIELD_STATUS_UNREADABLE {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} invalid {name}_status {status}"
        ));
    }
    if valid_flag != (status == FIELD_STATUS_VALID) {
        errors.push(format!(
            "offset {offset}: market trade row {row_index} {name} flag/status mismatch"
        ));
    }
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

pub fn summarize_compact_stream(raw: &[u8]) -> MarketTradeStreamSummary {
    let mut summary = MarketTradeStreamSummary {
        total_bytes: raw.len(),
        ..MarketTradeStreamSummary::default()
    };
    let mut offset = 0usize;
    let mut previous_sequence = None;
    let mut frame_hours = BTreeSet::new();
    let mut data_hours = BTreeSet::new();
    let mut per_hour_data = BTreeMap::<i32, u32>::new();

    while offset < raw.len() {
        if raw.len().saturating_sub(offset) < COMPACT_RECORD_HEADER_LEN {
            summary
                .parse_errors
                .push(format!("offset {offset}: truncated compact record header"));
            break;
        }
        let header = &raw[offset..offset + COMPACT_RECORD_HEADER_LEN];
        if &header[0..4] != COMPACT_RECORD_MAGIC {
            summary
                .parse_errors
                .push(format!("offset {offset}: invalid compact record magic"));
            break;
        }
        let version = read_u16_le(header, 4);
        let header_len = read_u16_le(header, 6) as usize;
        if version != COMPACT_RECORD_VERSION || header_len != COMPACT_RECORD_HEADER_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: unsupported compact record version {version} header_len {header_len}"
            ));
            break;
        }
        let sequence = read_u64_le(header, 8);
        let game_hour_raw = read_i32_le(header, 16);
        let domain_id = read_u16_le(header, 20);
        let record_type = read_u16_le(header, 22);
        let payload_len = read_u32_le(header, 24) as usize;
        let reserved = read_u32_le(header, 28);
        if reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: compact record reserved field is nonzero"
            ));
        }
        if let Some(previous) = previous_sequence {
            if sequence <= previous {
                summary.ordering_errors.push(format!(
                    "offset {offset}: sequence {sequence} did not advance past {previous}"
                ));
            }
        }
        previous_sequence = Some(sequence);

        let record_len = COMPACT_RECORD_HEADER_LEN.saturating_add(payload_len);
        if record_len > raw.len().saturating_sub(offset) {
            summary.parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
            ));
            break;
        }
        let payload_start = offset + COMPACT_RECORD_HEADER_LEN;
        let payload_end = payload_start + payload_len;
        let payload = &raw[payload_start..payload_end];
        summary.record_count = summary.record_count.saturating_add(1);

        match record_type {
            RECORD_TYPE_DOMAIN_DATA if domain_id == DOMAIN_ID => {
                summary.domain_data_records = summary.domain_data_records.saturating_add(1);
                data_hours.insert(game_hour_raw);
                *per_hour_data.entry(game_hour_raw).or_default() += 1;
                let payload_summary = decode_payload(offset, payload);
                summary.status_flags_or |= payload_summary.status_flags;
                summary.row_count_total = summary
                    .row_count_total
                    .saturating_add(payload_summary.row_count as u64);
                summary.observed_contract_rows_total = summary
                    .observed_contract_rows_total
                    .saturating_add(payload_summary.observed_contract_rows as u64);
                summary.omitted_candidate_rows_total = summary
                    .omitted_candidate_rows_total
                    .saturating_add(payload_summary.omitted_candidate_rows as u64);
                summary.partial_field_rows_total = summary
                    .partial_field_rows_total
                    .saturating_add(payload_summary.partial_field_rows as u64);
                if payload_summary.residual_trailer_present {
                    summary.residual_badge_records =
                        summary.residual_badge_records.saturating_add(1);
                }
                summary.residual_badge_rows_total = summary
                    .residual_badge_rows_total
                    .saturating_add(payload_summary.residual_badge_rows as u64);
                summary.residual_schema_blocked_rows_total = summary
                    .residual_schema_blocked_rows_total
                    .saturating_add(payload_summary.residual_schema_blocked_rows as u64);
                summary.residual_source_blocked_rows_total = summary
                    .residual_source_blocked_rows_total
                    .saturating_add(payload_summary.residual_source_blocked_rows as u64);
                summary.residual_owned_elsewhere_rows_total = summary
                    .residual_owned_elsewhere_rows_total
                    .saturating_add(payload_summary.residual_owned_elsewhere_rows as u64);
                summary.residual_shared_patch_blocked_rows_total = summary
                    .residual_shared_patch_blocked_rows_total
                    .saturating_add(payload_summary.residual_shared_patch_blocked_rows as u64);
                if payload_summary.detail_trailer_present {
                    summary.detail_records = summary.detail_records.saturating_add(1);
                }
                summary.detail_rows_total = summary
                    .detail_rows_total
                    .saturating_add(payload_summary.detail_rows as u64);
                summary.detail_price_valid_rows_total = summary
                    .detail_price_valid_rows_total
                    .saturating_add(payload_summary.detail_price_valid_rows as u64);
                summary.detail_speed_valid_rows_total = summary
                    .detail_speed_valid_rows_total
                    .saturating_add(payload_summary.detail_speed_valid_rows as u64);
                summary.detail_completed_valid_rows_total = summary
                    .detail_completed_valid_rows_total
                    .saturating_add(payload_summary.detail_completed_valid_rows as u64);
                summary.detail_days_valid_rows_total = summary
                    .detail_days_valid_rows_total
                    .saturating_add(payload_summary.detail_days_valid_rows as u64);
                summary.detail_delivery_country_valid_rows_total = summary
                    .detail_delivery_country_valid_rows_total
                    .saturating_add(payload_summary.detail_delivery_country_valid_rows as u64);
                summary.detail_convoys_valid_rows_total = summary
                    .detail_convoys_valid_rows_total
                    .saturating_add(payload_summary.detail_convoys_valid_rows as u64);
                summary.detail_efficiency_valid_rows_total = summary
                    .detail_efficiency_valid_rows_total
                    .saturating_add(payload_summary.detail_efficiency_valid_rows as u64);
                summary.detail_delivery_route_valid_rows_total = summary
                    .detail_delivery_route_valid_rows_total
                    .saturating_add(payload_summary.detail_delivery_route_valid_rows as u64);
                summary.detail_progress_valid_rows_total = summary
                    .detail_progress_valid_rows_total
                    .saturating_add(payload_summary.detail_progress_valid_rows as u64);
                if payload_summary.lend_lease_trailer_present {
                    summary.lend_lease_mapping_records =
                        summary.lend_lease_mapping_records.saturating_add(1);
                }
                summary.lend_lease_mapping_rows_total = summary
                    .lend_lease_mapping_rows_total
                    .saturating_add(payload_summary.lend_lease_mapping_rows as u64);
                summary.lend_lease_outgoing_rows_total = summary
                    .lend_lease_outgoing_rows_total
                    .saturating_add(payload_summary.lend_lease_outgoing_rows as u64);
                summary.lend_lease_incoming_rows_total = summary
                    .lend_lease_incoming_rows_total
                    .saturating_add(payload_summary.lend_lease_incoming_rows as u64);
                summary.lend_lease_amount_valid_rows_total = summary
                    .lend_lease_amount_valid_rows_total
                    .saturating_add(payload_summary.lend_lease_amount_valid_rows as u64);
                summary.lend_lease_price_valid_rows_total = summary
                    .lend_lease_price_valid_rows_total
                    .saturating_add(payload_summary.lend_lease_price_valid_rows as u64);
                summary
                    .parse_errors
                    .extend(payload_summary.parse_errors.iter().cloned());
                summary.payloads.push(payload_summary);
            }
            RECORD_TYPE_FRAME_END => {
                summary.frame_end_records = summary.frame_end_records.saturating_add(1);
                frame_hours.insert(game_hour_raw);
                if payload.len() >= 28 {
                    summary.status_flags_or |= read_u32_le(payload, 24);
                }
            }
            RECORD_TYPE_DOMAIN_PERF | RECORD_TYPE_DOMAIN_DATA => {}
            other => summary.parse_errors.push(format!(
                "offset {offset}: unknown compact record type {other}"
            )),
        }
        offset += record_len;
    }

    let all_hours: BTreeSet<i32> = frame_hours.union(&data_hours).copied().collect();
    summary.first_game_hour_raw = all_hours.first().copied();
    summary.last_game_hour_raw = all_hours.last().copied();
    summary.unique_game_hours = all_hours.len();
    summary.skipped_game_hours = skipped_hours(&all_hours);
    for (hour, count) in per_hour_data {
        if count != 1 {
            summary.parse_errors.push(format!(
                "game_hour_raw {hour}: market_trade_live data record count {count} != 1"
            ));
        }
    }
    summary
}

pub fn gate_compact_stream(raw: &[u8], min_game_hours: usize) -> MarketTradeGate {
    let summary = summarize_compact_stream(raw);
    let mut problems = Vec::new();
    if !summary.parse_errors.is_empty() {
        problems.push("parse_errors");
    }
    if !summary.ordering_errors.is_empty() {
        problems.push("ordering_errors");
    }
    if summary.domain_data_records == 0 {
        problems.push("no_market_trade_data_records");
    }
    if summary.unique_game_hours < min_game_hours {
        problems.push("insufficient_game_hours");
    }
    if !summary.skipped_game_hours.is_empty() {
        problems.push("skipped_game_hours");
    }
    if summary.status_flags_or & STATUS_WORKER_PANIC != 0 {
        problems.push("worker_panic");
    }
    if summary.status_flags_or & STATUS_BRIDGE_UNAVAILABLE != 0 {
        problems.push("bridge_unavailable");
    }
    if summary.status_flags_or & STATUS_SOURCE_UNAVAILABLE != 0 {
        problems.push("source_unavailable");
    }
    if summary.status_flags_or & STATUS_PARTIAL_DATA != 0 {
        problems.push("partial_data_status");
    }
    if summary.partial_field_rows_total != 0 {
        problems.push("missing_extractable_market_trade_rows");
    }
    MarketTradeGate {
        ok: problems.is_empty(),
        problems,
        summary,
    }
}

pub fn skipped_hours(hours: &BTreeSet<i32>) -> Vec<i32> {
    let mut missing = Vec::new();
    let ordered: Vec<i32> = hours.iter().copied().collect();
    for pair in ordered.windows(2) {
        let prev = pair[0];
        let next = pair[1];
        if next > prev + 1 {
            missing.extend((prev + 1)..next);
        }
    }
    missing
}

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_i32_le(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn read_i64_le(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_roundtrip_accepts_complete_purchase_contract_rows() {
        let rows = vec![
            MarketTradeRow::complete(726, 66, 17),
            MarketTradeRow::complete(785, 73, 33),
        ];
        let payload = encode_payload(1234, 0, 2, 2, &rows);

        let decoded = decode_payload(0, &payload);

        assert!(decoded.ok(), "{:?}", decoded.parse_errors);
        assert_eq!(decoded.row_count, 2);
        assert_eq!(decoded.observed_contract_rows, 2);
        assert_eq!(decoded.omitted_candidate_rows, 0);
        assert_eq!(decoded.partial_field_rows, 0);
        assert_eq!(decoded.rows, rows);
    }

    #[test]
    fn omitted_candidates_are_accounted_but_not_partial() {
        let rows = vec![MarketTradeRow::complete(726, 66, 17)];
        let payload = encode_payload(1234, 0, 4, 1, &rows);

        let decoded = decode_payload(0, &payload);

        assert!(decoded.ok(), "{:?}", decoded.parse_errors);
        assert_eq!(decoded.omitted_candidate_rows, 3);
        assert_eq!(decoded.missing_extractable_rows(), 0);
    }

    #[test]
    fn residual_badge_trailer_decodes_without_breaking_contract_rows() {
        let rows = vec![MarketTradeRow::complete(726, 66, 17)];
        let badges = residual_badge_rows(&market_trade_residual_badges());
        let payload = encode_payload_with_residual_badges(1234, 0, 1, 1, &rows, &badges);

        let decoded = decode_payload(0, &payload);

        assert!(decoded.ok(), "{:?}", decoded.parse_errors);
        assert_eq!(decoded.row_count, 1);
        assert!(decoded.residual_trailer_present);
        assert_eq!(decoded.residual_badge_rows as usize, badges.len());
        assert_eq!(decoded.residual_source_blocked_rows, 3);
        assert_eq!(decoded.residual_schema_blocked_rows, 0);
        assert_eq!(decoded.residual_owned_elsewhere_rows, 1);
        assert_eq!(decoded.residual_shared_patch_blocked_rows, 0);
        assert_eq!(decoded.residual_badges, badges);
    }

    #[test]
    fn residual_badges_cover_trade_economy_backlog() {
        let badges = market_trade_residual_badges();
        let names = badges.iter().map(|badge| badge.name).collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "purchase_contract_dynamic_tag_buyer_identity",
                "omitted_candidate_classification",
                "purchase_contract_delivery_route_progress",
                "purchase_contract_price_detail",
                "purchase_contract_speed_completed",
                "market_stockpile_sellable_equipment",
                "country_resource_import_export_flows",
                "license_production_contract_detail",
                "market_access_diplomatic_context",
                "lend_lease_market_contract_mapping",
            ]
        );
        assert!(badges
            .iter()
            .any(|badge| badge.status == RESIDUAL_STATUS_SOURCE_BLOCKED));
        assert!(badges
            .iter()
            .any(|badge| badge.name == "purchase_contract_price_detail"
                && badge.status == RESIDUAL_STATUS_VALIDATED_SLICE));
        assert!(badges
            .iter()
            .any(|badge| badge.status == RESIDUAL_STATUS_OWNED_ELSEWHERE_CONTEXT));
        assert!(badges
            .iter()
            .all(|badge| badge.evidence_mask != 0 && badge.proposed_record_kind != 0));
    }

    #[test]
    fn residual_badge_stream_passes_domain43_gate() {
        let badges = residual_badge_rows(&market_trade_residual_badges());
        let payload = encode_payload_with_residual_badges(0, 0, 0, 0, &[], &badges);
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload,
        );
        append_compact_record(&mut stream, 2, 100, 0, RECORD_TYPE_FRAME_END, &[]);

        let gate = gate_compact_stream(&stream, 1);

        assert!(gate.ok, "{:?}", gate.problems);
        assert_eq!(gate.summary.residual_badge_records, 1);
        assert_eq!(gate.summary.residual_badge_rows_total, badges.len() as u64);
        assert_eq!(gate.summary.residual_source_blocked_rows_total, 3);
    }

    #[test]
    fn detail_trailer_decodes_price_without_fabricating_route_fields() {
        let detail = PurchaseContractDetailRow {
            contract_id: 726,
            flags: DETAIL_FLAG_PRICE_VALID | DETAIL_FLAG_AMOUNT_VALID,
            price_raw: 250_000,
            amount_raw: 3_200_000,
            ..PurchaseContractDetailRow::default()
        };
        let payload = encode_payload_with_detail_rows(0, 0, 0, 0, &[], &[detail]);

        let decoded = decode_payload(0, &payload);

        assert!(decoded.ok(), "{:?}", decoded.parse_errors);
        assert!(decoded.detail_trailer_present);
        assert_eq!(decoded.detail_rows, 1);
        assert_eq!(decoded.detail_price_valid_rows, 1);
        assert_eq!(decoded.detail_speed_valid_rows, 0);
        assert_eq!(decoded.detail_delivery_route_valid_rows, 0);
        assert_eq!(decoded.detail_progress_valid_rows, 0);
        assert_eq!(decoded.detail_contracts, vec![detail]);
    }

    #[test]
    fn detail_decoder_rejects_unflagged_speed() {
        let detail = PurchaseContractDetailRow {
            contract_id: 726,
            flags: DETAIL_FLAG_PRICE_VALID,
            price_raw: 250_000,
            speed_raw: 2,
            ..PurchaseContractDetailRow::default()
        };
        let payload = encode_payload_with_detail_rows(0, 0, 0, 0, &[], &[detail]);

        let decoded = decode_payload(0, &payload);

        assert!(decoded
            .parse_errors
            .iter()
            .any(|err| err.contains("speed_raw without valid flag")));
    }

    #[test]
    fn detail_trailer_decodes_same_tick_route_and_speed_fields() {
        let detail = PurchaseContractDetailRow {
            contract_id: 985,
            flags: DETAIL_FLAG_PRICE_VALID
                | DETAIL_FLAG_AMOUNT_VALID
                | DETAIL_FLAG_SPEED_VALID
                | DETAIL_FLAG_COMPLETED_VALID
                | DETAIL_FLAG_DAYS_VALID
                | DETAIL_FLAG_DELIVERY_COUNTRY_VALID
                | DETAIL_FLAG_CONVOYS_VALID
                | DETAIL_FLAG_EFFICIENCY_VALID
                | DETAIL_FLAG_FACTORY_PROGRESS_VALID
                | DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID,
            price_raw: 250_000,
            amount_raw: 30_000_000,
            speed_raw: 1,
            completed_raw: 8,
            days: 16,
            delivery_country_tag_db_id: 117,
            convoys_assigned: 12,
            convoys_total: 12,
            efficiency_raw: 500_000,
            factory_cic_progress_raw: 15_000_000,
            equipment_transfer_progress_raw: 15_000_000,
            ..PurchaseContractDetailRow::default()
        };
        let payload = encode_payload_with_detail_rows(0, 0, 0, 0, &[], &[detail]);

        let decoded = decode_payload(0, &payload);

        assert!(decoded.ok(), "{:?}", decoded.parse_errors);
        assert_eq!(decoded.detail_speed_valid_rows, 1);
        assert_eq!(decoded.detail_completed_valid_rows, 1);
        assert_eq!(decoded.detail_days_valid_rows, 1);
        assert_eq!(decoded.detail_delivery_country_valid_rows, 1);
        assert_eq!(decoded.detail_convoys_valid_rows, 1);
        assert_eq!(decoded.detail_efficiency_valid_rows, 1);
        assert_eq!(decoded.detail_delivery_route_valid_rows, 1);
        assert_eq!(decoded.detail_progress_valid_rows, 1);
        assert_eq!(decoded.detail_contracts, vec![detail]);
    }

    #[test]
    fn lend_lease_mapping_trailer_decodes_symmetric_contract_views() {
        let rows = vec![MarketTradeRow::complete(726, 66, 17)];
        let mut prices = BTreeMap::new();
        prices.insert(726, 250_000);
        let mappings = lend_lease_mapping_rows_from_contracts(&rows, &prices);
        let payload = encode_payload_with_lend_lease_mappings(0, 0, 1, 1, &rows, &mappings);

        let decoded = decode_payload(0, &payload);

        assert!(decoded.ok(), "{:?}", decoded.parse_errors);
        assert!(decoded.lend_lease_trailer_present);
        assert_eq!(decoded.lend_lease_mapping_rows, 2);
        assert_eq!(decoded.lend_lease_outgoing_rows, 1);
        assert_eq!(decoded.lend_lease_incoming_rows, 1);
        assert_eq!(decoded.lend_lease_amount_valid_rows, 2);
        assert_eq!(decoded.lend_lease_price_valid_rows, 2);
        assert_eq!(decoded.lend_lease_mappings, mappings);
    }

    #[test]
    fn lend_lease_mapping_rejects_missing_counterpart_view() {
        let rows = vec![MarketTradeRow::complete(726, 66, 17)];
        let mut prices = BTreeMap::new();
        prices.insert(726, 250_000);
        let mappings = lend_lease_mapping_rows_from_contracts(&rows, &prices);
        let payload = encode_payload_with_lend_lease_mappings(0, 0, 1, 1, &rows, &mappings[..1]);

        let decoded = decode_payload(0, &payload);

        assert!(decoded.parse_errors.iter().any(|err| err
            .contains("lend-lease contract_id 726 has outgoing 1 incoming 0, expected 1/1")));
    }

    #[test]
    fn residual_decoder_rejects_duplicate_gap_ids() {
        let mut badges = residual_badge_rows(&market_trade_residual_badges());
        badges[1].gap_id = badges[0].gap_id;
        badges[1].source_ref_id = badges[0].source_ref_id;
        let payload = encode_payload_with_residual_badges(0, 0, 0, 0, &[], &badges);

        let decoded = decode_payload(0, &payload);

        assert!(decoded
            .parse_errors
            .iter()
            .any(|err| err.contains("duplicates gap_id")));
    }

    #[test]
    fn partial_row_fails_gate_even_when_payload_decodes() {
        let mut row = MarketTradeRow::complete(785, 73, 368);
        row.flags &= !ROW_FLAG_BUYER_PTR_VALID;
        row.buyer_ptr_status = FIELD_STATUS_INVALID;
        let payload = encode_payload(1234, STATUS_PARTIAL_DATA, 1, 1, &[row]);

        let decoded = decode_payload(0, &payload);

        assert!(
            decoded.parse_errors.is_empty(),
            "{:?}",
            decoded.parse_errors
        );
        assert!(!decoded.ok());
        assert_eq!(decoded.partial_field_rows, 1);
    }

    #[test]
    fn decoder_rejects_fabricated_variant_without_flag() {
        let mut row = MarketTradeRow::complete(1, 2, 3);
        row.flags &= !ROW_FLAG_VARIANT_VALID;
        row.equipment_variant_id = 99;
        let payload = encode_payload(0, 0, 1, 1, &[row]);

        let decoded = decode_payload(0, &payload);

        assert!(decoded
            .parse_errors
            .iter()
            .any(|err| err.contains("variant id without valid flag")));
    }

    #[test]
    fn compact_stream_gate_requires_hourly_unskipped_data() {
        let payload_a = encode_payload(0, 0, 1, 1, &[MarketTradeRow::complete(1, 2, 3)]);
        let payload_b = encode_payload(1, 0, 1, 1, &[MarketTradeRow::complete(2, 2, 3)]);
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            10,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload_a,
        );
        append_compact_record(&mut stream, 2, 10, 0, RECORD_TYPE_FRAME_END, &[]);
        append_compact_record(
            &mut stream,
            3,
            12,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload_b,
        );
        append_compact_record(&mut stream, 4, 12, 0, RECORD_TYPE_FRAME_END, &[]);

        let gate = gate_compact_stream(&stream, 2);

        assert!(!gate.ok);
        assert!(gate.problems.contains(&"skipped_game_hours"));
        assert_eq!(gate.summary.skipped_game_hours, vec![11]);
    }

    #[test]
    fn compact_stream_gate_accepts_complete_consecutive_hours() {
        let mut stream = Vec::new();
        for (sequence, hour) in [(1, 20), (3, 21), (5, 22)] {
            let payload = encode_payload(
                sequence,
                0,
                1,
                1,
                &[MarketTradeRow::complete(sequence as u32, 2, 3)],
            );
            append_compact_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_DATA,
                &payload,
            );
            append_compact_record(
                &mut stream,
                sequence + 1,
                hour,
                0,
                RECORD_TYPE_FRAME_END,
                &[],
            );
        }

        let gate = gate_compact_stream(&stream, 3);

        assert!(gate.ok, "{:?}", gate.problems);
        assert_eq!(gate.summary.domain_data_records, 3);
        assert_eq!(gate.summary.unique_game_hours, 3);
        assert_eq!(gate.summary.partial_field_rows_total, 0);
    }
}
