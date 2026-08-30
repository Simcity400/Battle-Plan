use std::collections::{BTreeMap, BTreeSet};

use compact_contract::{
    parse_compact_record_header, validate_compact_record_semantics, RecordFramingError,
};

mod leaf_decode;

pub const DOMAIN_ID: u16 = 43;
pub const DOMAIN_NAME: &str = "market_trade_live";

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = compact_contract::COMPACT_RECORD_MAGIC;
pub const COMPACT_RECORD_VERSION: u16 = compact_contract::COMPACT_RECORD_VERSION;
pub const COMPACT_RECORD_HEADER_LEN: usize = compact_contract::COMPACT_RECORD_HEADER_LEN;
pub const RECORD_TYPE_DOMAIN_PERF: u16 = compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF;
pub const RECORD_TYPE_FRAME_END: u16 = compact_contract::COMPACT_RECORD_TYPE_FRAME_END;
pub const RECORD_TYPE_DOMAIN_DATA: u16 = compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA;

pub const STATUS_WORKER_PANIC: u32 = compact_contract::COMPACT_STATUS_WORKER_PANIC;
pub const STATUS_BRIDGE_UNAVAILABLE: u32 = compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE;
pub const STATUS_PARTIAL_DATA: u32 = compact_contract::COMPACT_STATUS_PARTIAL_DATA;

pub const MARKET_TRADE_HEADER_LEN: usize = 48;
pub const MARKET_TRADE_ROW_LEN: usize = 48;
pub const MARKET_TRADE_CONTRACT_TYPE: u32 = 81;
pub const MARKET_TRADE_MAX_TAG_DB_ID: u32 =
    compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
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
pub const MARKET_TRADE_RESOURCE_FLOW_TRAILER_MAGIC: &[u8; 4] = b"MTRF";
pub const MARKET_TRADE_RESOURCE_FLOW_LEGACY_SCHEMA_VERSION: u16 = 1;
pub const MARKET_TRADE_RESOURCE_FLOW_FIXED7_SCHEMA_VERSION: u16 = 2;
pub const MARKET_TRADE_RESOURCE_FLOW_SCHEMA_VERSION: u16 = 3;
pub const MARKET_TRADE_RESOURCE_FLOW_HEADER_LEN: usize = 32;
pub const MARKET_TRADE_RESOURCE_FLOW_FIXED7_ROW_LEN: usize = 72;
pub const MARKET_TRADE_RESOURCE_FLOW_ROW_HEADER_LEN: usize = 16;
pub const MARKET_TRADE_RESOURCE_FLOW_KIND_PRODUCED: u32 = 1;
pub const MARKET_TRADE_RESOURCE_FLOW_KIND_IMPORTED: u32 = 2;
pub const MARKET_TRADE_RESOURCE_FLOW_KIND_EXPORTED: u32 = 3;
pub const MARKET_TRADE_RESOURCE_FLOW_KIND_USED: u32 = 4;
pub const MARKET_TRADE_RESOURCE_FLOW_KIND_TO_EXPORT: u32 = 5;
pub const MARKET_TRADE_RESOURCE_FLOW_KIND_BASE_EXPORT: u32 = 6;
pub const MARKET_TRADE_RESOURCE_FLOW_KIND_MASK: u32 = 0x3F;
pub const MARKET_TRADE_RESOURCE_FLOW_RESOURCE_MASK: u32 = 0x7F;
pub const MARKET_TRADE_FACTORY_HEADER_TRAILER_MAGIC: &[u8; 4] = b"MTRH";
pub const MARKET_TRADE_FACTORY_HEADER_SCHEMA_VERSION: u16 = 1;
pub const MARKET_TRADE_FACTORY_HEADER_HEADER_LEN: usize = 24;
pub const MARKET_TRADE_FACTORY_HEADER_ROW_LEN: usize = 16;
pub const MARKET_TRADE_RESOURCE_ROW_TRAILER_MAGIC: &[u8; 4] = b"MTRR";
pub const MARKET_TRADE_RESOURCE_ROW_LEGACY_SCHEMA_VERSION: u16 = 1;
pub const MARKET_TRADE_RESOURCE_ROW_COUNT_ONLY_SCHEMA_VERSION: u16 = 2;
pub const MARKET_TRADE_RESOURCE_ROW_SCHEMA_VERSION: u16 = 3;
pub const MARKET_TRADE_RESOURCE_ROW_NEEDED_UNAVAILABLE_SCHEMA_VERSION: u16 = 4;
pub const MARKET_TRADE_RESOURCE_ROW_HEADER_LEN: usize = 32;
pub const MARKET_TRADE_RESOURCE_ROW_LEN: usize = 64;
pub const STRATEGIC_RESOURCE_IDENTITY_STRIDE: usize =
    compact_contract::build_profile::COMPACT_STRATEGIC_RESOURCE_IDENTITY_STRIDE as usize;
pub const MAX_STRATEGIC_RESOURCE_TOKEN_LEN: usize =
    compact_contract::build_profile::MAX_COMPACT_STRATEGIC_RESOURCE_TOKEN_LEN;
pub const MARKET_TRADE_FIXED7_RESOURCE_COUNT: u32 = 7;
pub const MAX_STRATEGIC_RESOURCE_COUNT: u32 =
    compact_contract::build_profile::MAX_COMPACT_STRATEGIC_RESOURCE_COUNT;
pub const MARKET_TRADE_RESOURCE_NEEDED_SURPLUS: u32 = 1;
pub const MARKET_TRADE_RESOURCE_NEEDED_DEFICIT: u32 = 2;
pub const MARKET_TRADE_RESOURCE_NEEDED_CURRENT_MAX: u32 = 3;
pub const MARKET_TRADE_RESOURCE_NEEDED_UNAVAILABLE: u32 = 4;
pub const MARKET_TRADE_RESOURCE_FLAG_CURRENT: u32 = 1;
pub const MARKET_TRADE_RESOURCE_FLAG_MAX: u32 = 1 << 1;
pub const MARKET_TRADE_RESOURCE_FLAG_NEEDED_UNAVAILABLE: u32 = 1 << 2;
pub const MARKET_TRADE_RESOURCE_FLAGS_KNOWN: u32 =
    MARKET_TRADE_RESOURCE_FLAG_CURRENT | MARKET_TRADE_RESOURCE_FLAG_MAX;
pub const MARKET_TRADE_CLASSIC_DEAL_TRAILER_MAGIC: &[u8; 4] = b"MTRD";
pub const MARKET_TRADE_CLASSIC_DEAL_LEGACY_SCHEMA_VERSION: u16 = 1;
pub const MARKET_TRADE_CLASSIC_DEAL_SCHEMA_VERSION: u16 = 2;
pub const MARKET_TRADE_CLASSIC_DEAL_PER_OWNER_SCHEMA_VERSION: u16 = 3;
pub const MARKET_TRADE_CLASSIC_DEAL_WIDE_IDENTITY_SCHEMA_VERSION: u16 = 4;
pub const MARKET_TRADE_CLASSIC_DEAL_HEADER_LEN: usize = 32;
pub const MARKET_TRADE_CLASSIC_DEAL_ROW_LEN: usize = 96;
pub const MARKET_TRADE_CLASSIC_DEAL_DIRECTION_IMPORT: u32 = 1;
pub const MARKET_TRADE_CLASSIC_DEAL_DIRECTION_EXPORT: u32 = 2;
pub const MARKET_TRADE_CLASSIC_DEAL_ROUTE_LAND: u32 = 1;
pub const MARKET_TRADE_CLASSIC_DEAL_ROUTE_SEA: u32 = 2;
pub const MARKET_TRADE_CLASSIC_DEAL_ROUTE_LOCKED: u32 = 3;
pub const MARKET_TRADE_CLASSIC_DEAL_FLAG_EXACT: u32 = 1;
pub const MARKET_TRADE_CLASSIC_DEAL_STATUS_NO_TRADE_INFLUENCE: u32 = 1;
pub const MARKET_TRADE_CLASSIC_DEAL_STATUS_NO_RESOURCES: u32 = 1 << 1;
pub const MARKET_TRADE_CLASSIC_DEAL_STATUS_KNOWN: u32 =
    MARKET_TRADE_CLASSIC_DEAL_STATUS_NO_TRADE_INFLUENCE
        | MARKET_TRADE_CLASSIC_DEAL_STATUS_NO_RESOURCES;
pub const MARKET_TRADE_ACTIVATION_TRAILER_MAGIC: &[u8; 4] = b"MTRA";
pub const MARKET_TRADE_ACTIVATION_SCHEMA_VERSION: u16 = 1;
pub const MARKET_TRADE_ACTIVATION_HEADER_LEN: usize = 32;
pub const MARKET_TRADE_ACTIVATION_FLAG_ACTIVE: u32 = 1;
pub const MARKET_TRADE_ACTIVATION_FLAG_ROOT_EXACT: u32 = 1 << 1;
pub const MARKET_TRADE_ACTIVATION_FLAGS_KNOWN: u32 =
    MARKET_TRADE_ACTIVATION_FLAG_ACTIVE | MARKET_TRADE_ACTIVATION_FLAG_ROOT_EXACT;
pub const MARKET_TRADE_ACTIVATION_ROOT_COUNTRYTRADEVIEW: u32 = 1;
pub const MARKET_TRADE_ACTIVATION_OPEN_RVA: u32 = 0x015e_c520;
pub const MARKET_TRADE_STOCKPILE_TRAILER_MAGIC: &[u8; 4] = b"MTS1";
pub const MARKET_TRADE_STOCKPILE_SCHEMA_VERSION: u16 = 1;
pub const MARKET_TRADE_STOCKPILE_HEADER_LEN: usize = 40;
pub const MARKET_TRADE_STOCKPILE_COUNTRY_ROW_LEN: usize = 20;
pub const MARKET_TRADE_STOCKPILE_ITEM_ROW_LEN: usize = 24;
pub const MARKET_TRADE_STOCKPILE_GLOBAL_OWNER_ITERATOR_COMPLETE: u32 = 1;
pub const MARKET_TRADE_STOCKPILE_GLOBAL_NESTED_ITERATORS_COMPLETE: u32 = 1 << 1;
pub const MARKET_TRADE_STOCKPILE_GLOBAL_FLAGS_KNOWN: u32 =
    MARKET_TRADE_STOCKPILE_GLOBAL_OWNER_ITERATOR_COMPLETE
        | MARKET_TRADE_STOCKPILE_GLOBAL_NESTED_ITERATORS_COMPLETE;
pub const MARKET_TRADE_STOCKPILE_COUNTRY_COMPLETE: u32 = 1;
pub const MARKET_TRADE_STOCKPILE_EQUIPMENT_TYPE: u32 = 70;

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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourceFlowRow {
    pub tag_db_id: u32,
    pub flow_kind: u32,
    pub resource_mask_or_count: u32,
    pub values_raw: Vec<i64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FactoryHeaderRow {
    pub tag_db_id: u32,
    pub total: i32,
    pub used: i32,
    pub available: i32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResourceViewRow {
    pub tag_db_id: u32,
    pub resource_index: u32,
    pub visible_order: u32,
    pub needed_type: u32,
    pub balance: i32,
    pub produced: i32,
    pub imported: i32,
    pub exported: i32,
    pub special_project: i32,
    pub production_line: i32,
    pub needed: i32,
    pub needed_current: i32,
    pub needed_max: i32,
    pub flags: u32,
    pub descriptor_db_index: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct ClassicDealIdentity {
    pub exporter_tag_db_id: u32,
    pub importer_tag_db_id: u32,
    pub resource_index: u32,
    pub ordinal: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClassicDealRow {
    pub owner_tag_db_id: u32,
    pub counterpart_tag_db_id: u32,
    pub resource_index: u32,
    pub visible_order: u32,
    pub direction: u32,
    pub route_kind: u32,
    pub flags: u32,
    pub route_status_flags: Option<u32>,
    pub identity: ClassicDealIdentity,
    pub requested_raw: i64,
    pub delivered_raw: i64,
    pub available_export_raw: i64,
    pub factory_cost_raw: i64,
    pub efficiency_raw: i64,
    pub civilian_factories: i32,
    pub convoys_assigned: i32,
    pub convoys_required: i32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MarketTradeActivation {
    pub active: bool,
    pub country_tag_db_id: Option<u32>,
    pub observation_sequence: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MarketStockpileCountryRow {
    pub tag_db_id: u32,
    pub item_first: u32,
    pub item_count: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MarketStockpileItemRow {
    pub equipment_variant_id: u32,
    pub equipment_type: u32,
    pub ordinal: u32,
    pub amount_raw: i64,
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
    pub resource_flow_trailer_present: bool,
    pub resource_flow_rows: u32,
    pub resource_flow_country_rows: u32,
    pub resource_flow_exported_rows: u32,
    pub resource_flow_scale_raw: u32,
    pub resource_flow_resource_mask: u32,
    pub resource_flow_resource_count: u32,
    pub resource_flow_kind_mask: u32,
    pub factory_header_trailer_present: bool,
    pub factory_header_rows: u32,
    pub resource_row_trailer_present: bool,
    pub resource_rows: u32,
    pub resource_row_country_count: u32,
    pub resource_row_resource_count: u32,
    pub resource_tokens: Vec<String>,
    pub classic_deal_trailer_present: bool,
    pub classic_deal_rows: u32,
    pub classic_deal_country_count: u32,
    pub classic_deal_incomplete_owner_count: u32,
    pub activation_trailer_present: bool,
    pub stockpile_trailer_present: bool,
    pub stockpile_country_rows: u32,
    pub stockpile_item_rows: u32,
    pub stockpile_expected_country_count: u32,
    pub rows: Vec<MarketTradeRow>,
    pub residual_badges: Vec<MarketTradeResidualBadgeRow>,
    pub detail_contracts: Vec<PurchaseContractDetailRow>,
    pub lend_lease_mappings: Vec<LendLeaseMappingRow>,
    pub resource_flow_rows_decoded: Vec<ResourceFlowRow>,
    pub factory_headers: Vec<FactoryHeaderRow>,
    pub resource_view_rows: Vec<ResourceViewRow>,
    pub classic_deal_country_witnesses: Vec<u32>,
    pub classic_deal_incomplete_owners: Vec<u32>,
    pub classic_deals: Vec<ClassicDealRow>,
    pub activation: Option<MarketTradeActivation>,
    pub stockpile_countries: Vec<MarketStockpileCountryRow>,
    pub stockpile_items: Vec<MarketStockpileItemRow>,
    pub parse_errors: Vec<String>,
}

impl MarketTradePayloadSummary {
    pub fn missing_extractable_rows(&self) -> u32 {
        self.partial_field_rows
    }

    pub fn ok(&self) -> bool {
        self.parse_errors.is_empty()
            && compact_contract::compact_status_failure_flags_after_expected_partial_tolerance(
                DOMAIN_ID,
                self.status_flags,
            ) == 0
            && self.partial_field_rows == 0
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MarketTradeStreamSummary {
    pub total_bytes: usize,
    pub record_count: u64,
    pub domain_perf_records: u64,
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
    pub resource_flow_records: u64,
    pub resource_flow_rows_total: u64,
    pub resource_flow_country_rows_total: u64,
    pub resource_flow_exported_rows_total: u64,
    pub factory_header_records: u64,
    pub factory_header_rows_total: u64,
    pub resource_row_records: u64,
    pub resource_rows_total: u64,
    pub classic_deal_records: u64,
    pub classic_deal_rows_total: u64,
    pub activation_records: u64,
    pub activation_active_records: u64,
    pub activation_closed_records: u64,
    pub activation_state_transitions: u64,
    pub activation_first_active: Option<bool>,
    pub activation_last_active: Option<bool>,
    pub activation_last_sequence: Option<u64>,
    pub activation_saw_closed_before_active: bool,
    pub activation_saw_active_after_closed: bool,
    pub activation_saw_closed_after_active: bool,
    pub activation_sequence_errors: Vec<String>,
    pub payloads: Vec<MarketTradePayloadSummary>,
    pub parse_errors: Vec<String>,
    pub ordering_errors: Vec<String>,
    pub boundary_errors: Vec<String>,
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
                | RESIDUAL_EVIDENCE_CURRENT_COMPACT_SCHEMA
                | RESIDUAL_EVIDENCE_CURRENT_ROOT_PROOF
                | RESIDUAL_EVIDENCE_NO_HOT_LOOP_ROOT,
            current_support: RESIDUAL_SUPPORT_PARTIAL,
            current_compact_evidence: "MTS1 carries the exact per-country owner census, complete nested type-70 variant order, raw amounts, and measured-empty owner witnesses through the recorder fold and typed replay; the production route remains source-blocked until the fresh live root-validation fixture passes",
            source_evidence: "exact a729 static RE proves CCountry+0xFB8 -> CEquipmentMarketStatus+0x00 -> CMarketStockpile+0x38/CEquipmentVariantPool+0x20, owner echo +0x20, vtable 0x1427330A0, and serializer 0x140FFDB00; CCountry+0x1230 remains rejected",
            fail_closed_badge: "MTS1 is admitted only with complete owner and nested-iterator witnesses, canonical ranges/order, type 70, and nonnegative amounts; until the live owner-chain fixture validates, absence remains unavailable and no stockpile is fabricated",
            proposed_record_kind: RESIDUAL_RECORD_KIND_MARKET_STOCKPILE,
            proposed_record: "market_stockpile",
            owner_boundary: RESIDUAL_OWNER_MARKET_TRADE,
            owner_label: "market_trade_live",
            root_status: RESIDUAL_ROOT_UNVALIDATED,
            proof_status: RESIDUAL_PROOF_STATIC_ORACLE,
        },
        MarketTradeResidualBadge {
            gap_id: 7,
            name: "country_resource_import_export_flows",
            status: RESIDUAL_STATUS_VALIDATED_SLICE,
            status_label: "validated_slice",
            evidence_mask: RESIDUAL_EVIDENCE_SAVE_ORACLE
                | RESIDUAL_EVIDENCE_COVERAGE_AUDIT
                | RESIDUAL_EVIDENCE_CURRENT_ROOT_PROOF
                | RESIDUAL_EVIDENCE_D04_DYNAMIC_TAG_PROOF,
            current_support: RESIDUAL_SUPPORT_VALIDATED_SLICE,
            current_compact_evidence: "MTRF v2 emits all six exact seven-resource pools (produced, imported, exported, used, to_export, base_export) with a complete country witness; the fold rejects incomplete or mixed-version pool sets",
            source_evidence: "CCountry+0x11F8 owns CCountryResources; exact pools are +0x28 produced, +0x188 imported, +0x238 used, +0x450 to_export, +0x500 base_export, and +0x5B0 exported in oil/aluminium/rubber/tungsten/steel/chromium/coal order",
            fail_closed_badge: "MTRF v2 requires exactly six unique flow kinds for every witnessed country and the full seven-resource mask; legacy v1 remains an exported-only compatibility slice and never promotes complete=true",
            proposed_record_kind: RESIDUAL_RECORD_KIND_COUNTRY_RESOURCE_FLOW,
            proposed_record: "country_resource_flow",
            owner_boundary: RESIDUAL_OWNER_MARKET_TRADE,
            owner_label: "market_trade_live",
            root_status: RESIDUAL_ROOT_VALIDATED,
            proof_status: RESIDUAL_PROOF_LIVE_870EBF,
        },
        MarketTradeResidualBadge {
            gap_id: 8,
            name: "license_production_contract_detail",
            status: RESIDUAL_STATUS_VALIDATED_SLICE,
            status_label: "validated_slice",
            evidence_mask: RESIDUAL_EVIDENCE_DIPLOMACY_LANE_ROOTS
                | RESIDUAL_EVIDENCE_CURRENT_ROOT_PROOF
                | RESIDUAL_EVIDENCE_OWNER_BOUNDARY
                | RESIDUAL_EVIDENCE_NO_HOT_LOOP_ROOT,
            current_support: RESIDUAL_SUPPORT_PARTIAL,
            current_compact_evidence: "PLIC v2 carries exact ordered licensed variants plus creator/origin identity; the fold joins them by exact country pair to D53 CLicenceProductionAccessRelation first/second/start/end lifecycle with an explicit completeness witness",
            source_evidence: "CCountry+0xF68 -> manager+0x198/+0x1A0/+0x1A4 is the exact ordered type-70 CEquipmentVariant pointer vector; variant +0x0C/+0x1C/+0x20 supplies id/creator/optional origin. The rejected shared CCountry+0x1070/+0x10A0 CLicensedProductionConsumer table remains rejected",
            fail_closed_badge: "legacy PLIC v1, torn vectors, stale/partial D11 or D53, missing lifecycle, and unmatched/ambiguous pairs emit complete=false and null agreements; per-contract royalty/factory-cost rates remain omitted because no exact authoritative field was proven",
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
            status: RESIDUAL_STATUS_VALIDATED_SLICE,
            status_label: "validated_slice",
            evidence_mask: RESIDUAL_EVIDENCE_DIPLOMACY_LANE_ROOTS
                | RESIDUAL_EVIDENCE_OWNER_BOUNDARY,
            current_support: RESIDUAL_SUPPORT_VALIDATED_SLICE,
            current_compact_evidence: "every current D43 MTLL purchase contract carries its exact global contract order and a fail-closed D53 CEquipmentPurchaseRelation pair/start/end context join; D53 direction is preserved independently from D43 seller/buyer roles",
            source_evidence: "diplomacy_intel_operatives_live pins CMarketAccessRightsRelation and CEquipmentPurchaseRelation current-build relation flags",
            fail_closed_badge: "missing/partial D53, malformed lifecycle, or unmatched/ambiguous pair leaves equipment_purchase_relation_complete=false and the relation null without suppressing the independently measured D43 contract; no duplicate relation scan is scheduled in d43",
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
                | "purchase_contract_delivery_route_progress"
                | "purchase_contract_price_detail"
                | "purchase_contract_speed_completed"
                | "market_stockpile_sellable_equipment"
                | "lend_lease_market_contract_mapping"
                | "country_resource_import_export_flows" => DOMAIN_ID as u32,
                "market_access_diplomatic_context" => 53,
                "license_production_contract_detail" => 11,
                _ => 0,
            },
            proposed_record_kind: badge.proposed_record_kind,
            current_row_len: match badge.name {
                "purchase_contract_dynamic_tag_buyer_identity"
                | "omitted_candidate_classification" => MARKET_TRADE_ROW_LEN as u32,
                "purchase_contract_delivery_route_progress"
                | "purchase_contract_price_detail"
                | "purchase_contract_speed_completed" => MARKET_TRADE_DETAIL_ROW_LEN as u32,
                "market_stockpile_sellable_equipment" => MARKET_TRADE_STOCKPILE_ITEM_ROW_LEN as u32,
                "lend_lease_market_contract_mapping" => MARKET_TRADE_LEND_LEASE_ROW_LEN as u32,
                "country_resource_import_export_flows" => {
                    MARKET_TRADE_RESOURCE_FLOW_FIXED7_ROW_LEN as u32
                }
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

pub fn resource_flow_row_len(resource_count: u32) -> usize {
    MARKET_TRADE_RESOURCE_FLOW_ROW_HEADER_LEN
        .saturating_add((resource_count as usize).saturating_mul(8))
}

pub fn encode_resource_flow_trailer(
    rows: &[ResourceFlowRow],
    resource_count: u32,
) -> Option<Vec<u8>> {
    if !(1..=MAX_STRATEGIC_RESOURCE_COUNT).contains(&resource_count) {
        return None;
    }
    if rows.iter().any(|row| {
        row.values_raw.len() != resource_count as usize
            || row.resource_mask_or_count != resource_count
            || !(1..=6).contains(&row.flow_kind)
    }) {
        return None;
    }
    let row_count = rows.len().min(u32::MAX as usize) as u32;
    let country_count = rows
        .iter()
        .map(|row| row.tag_db_id)
        .collect::<BTreeSet<_>>()
        .len()
        .min(u32::MAX as usize) as u32;
    let row_len = resource_flow_row_len(resource_count);

    let mut payload =
        Vec::with_capacity(MARKET_TRADE_RESOURCE_FLOW_HEADER_LEN + rows.len() * row_len);
    payload.extend_from_slice(MARKET_TRADE_RESOURCE_FLOW_TRAILER_MAGIC);
    payload.extend_from_slice(&MARKET_TRADE_RESOURCE_FLOW_SCHEMA_VERSION.to_le_bytes());
    payload.extend_from_slice(&(MARKET_TRADE_RESOURCE_FLOW_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(row_len.min(u16::MAX as usize) as u16).to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&country_count.to_le_bytes());
    payload.extend_from_slice(&MARKET_TRADE_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&resource_count.to_le_bytes());
    payload.extend_from_slice(&MARKET_TRADE_RESOURCE_FLOW_KIND_MASK.to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.flow_kind.to_le_bytes());
        payload.extend_from_slice(&row.resource_mask_or_count.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        for value in &row.values_raw {
            payload.extend_from_slice(&value.to_le_bytes());
        }
    }
    Some(payload)
}

pub fn encode_factory_header_trailer(rows: &[FactoryHeaderRow]) -> Vec<u8> {
    let row_count = rows.len().min(u32::MAX as usize) as u32;
    let mut payload = Vec::with_capacity(
        MARKET_TRADE_FACTORY_HEADER_HEADER_LEN + rows.len() * MARKET_TRADE_FACTORY_HEADER_ROW_LEN,
    );
    payload.extend_from_slice(MARKET_TRADE_FACTORY_HEADER_TRAILER_MAGIC);
    payload.extend_from_slice(&MARKET_TRADE_FACTORY_HEADER_SCHEMA_VERSION.to_le_bytes());
    payload.extend_from_slice(&(MARKET_TRADE_FACTORY_HEADER_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(MARKET_TRADE_FACTORY_HEADER_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.total.to_le_bytes());
        payload.extend_from_slice(&row.used.to_le_bytes());
        payload.extend_from_slice(&row.available.to_le_bytes());
    }
    payload
}

pub fn encode_activation_trailer(activation: MarketTradeActivation) -> Result<Vec<u8>, String> {
    if activation.active != activation.country_tag_db_id.is_some()
        || activation
            .country_tag_db_id
            .is_some_and(|tag| tag == 0 || tag > MARKET_TRADE_MAX_TAG_DB_ID)
    {
        return Err("MTRA active/country identity is inconsistent".into());
    }
    let flags = MARKET_TRADE_ACTIVATION_FLAG_ROOT_EXACT
        | if activation.active {
            MARKET_TRADE_ACTIVATION_FLAG_ACTIVE
        } else {
            0
        };
    let mut payload = Vec::with_capacity(MARKET_TRADE_ACTIVATION_HEADER_LEN);
    payload.extend_from_slice(MARKET_TRADE_ACTIVATION_TRAILER_MAGIC);
    payload.extend_from_slice(&MARKET_TRADE_ACTIVATION_SCHEMA_VERSION.to_le_bytes());
    payload.extend_from_slice(&(MARKET_TRADE_ACTIVATION_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&flags.to_le_bytes());
    payload.extend_from_slice(&activation.country_tag_db_id.unwrap_or(0).to_le_bytes());
    payload.extend_from_slice(&activation.observation_sequence.to_le_bytes());
    payload.extend_from_slice(&MARKET_TRADE_ACTIVATION_ROOT_COUNTRYTRADEVIEW.to_le_bytes());
    payload.extend_from_slice(&MARKET_TRADE_ACTIVATION_OPEN_RVA.to_le_bytes());
    Ok(payload)
}

pub fn encode_market_stockpile_trailer(
    expected_country_count: u32,
    countries: &[MarketStockpileCountryRow],
    items: &[MarketStockpileItemRow],
) -> Vec<u8> {
    let country_count = countries.len().min(u32::MAX as usize) as u32;
    let item_count = items.len().min(u32::MAX as usize) as u32;
    let segment_len = MARKET_TRADE_STOCKPILE_HEADER_LEN
        .saturating_add(
            countries
                .len()
                .saturating_mul(MARKET_TRADE_STOCKPILE_COUNTRY_ROW_LEN),
        )
        .saturating_add(
            items
                .len()
                .saturating_mul(MARKET_TRADE_STOCKPILE_ITEM_ROW_LEN),
        );
    let mut payload = Vec::with_capacity(segment_len);
    payload.extend_from_slice(MARKET_TRADE_STOCKPILE_TRAILER_MAGIC);
    payload.extend_from_slice(&MARKET_TRADE_STOCKPILE_SCHEMA_VERSION.to_le_bytes());
    payload.extend_from_slice(&(MARKET_TRADE_STOCKPILE_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(MARKET_TRADE_STOCKPILE_COUNTRY_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(MARKET_TRADE_STOCKPILE_ITEM_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(segment_len as u32).to_le_bytes());
    payload.extend_from_slice(&country_count.to_le_bytes());
    payload.extend_from_slice(&item_count.to_le_bytes());
    payload.extend_from_slice(&expected_country_count.to_le_bytes());
    payload.extend_from_slice(&MARKET_TRADE_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&MARKET_TRADE_STOCKPILE_GLOBAL_FLAGS_KNOWN.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for row in countries {
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.item_first.to_le_bytes());
        payload.extend_from_slice(&row.item_count.to_le_bytes());
        payload.extend_from_slice(&MARKET_TRADE_STOCKPILE_COUNTRY_COMPLETE.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    for row in items {
        payload.extend_from_slice(&row.equipment_variant_id.to_le_bytes());
        payload.extend_from_slice(&row.equipment_type.to_le_bytes());
        payload.extend_from_slice(&row.ordinal.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&row.amount_raw.to_le_bytes());
    }
    payload
}

// @d43-payload-reader-region begin
// @leaf-source-authority crate=d43_grammar root=extractor/native/d43_grammar files=8 sha256=1c09d4e7cfb0eb8d9f6f15f471b36197031c998521e6a715747308ea59134748
pub fn decode_payload(offset: usize, payload: &[u8]) -> MarketTradePayloadSummary {
    let report = d43_grammar::decode::decode_record(payload, leaf_decode::LANE_DECODE_CONTEXT);
    leaf_decode::project_decoded_payload(offset, &report)
}
// @d43-payload-reader-region end

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
    let mut repeat_baselines = compact_contract::DomainRepeatBaselines::new();
    if let Err(error) =
        compact_contract::stream_validator::validate_freeze_census_stream_placement(raw)
    {
        summary
            .parse_errors
            .push(format!("stateful stream placement: {error}"));
    }
    let mut offset = 0usize;
    let mut previous_sequence: Option<u64> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut open_frame_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut clean_closed_hours = BTreeSet::new();
    let mut observed_closure_hours = BTreeSet::new();
    let mut final_record_is_clean_frame_end = false;
    let mut release_epilogue_seen = false;
    let mut per_hour_perf = BTreeMap::<i32, u32>::new();
    let mut per_hour_data = BTreeMap::<i32, u32>::new();

    while offset < raw.len() {
        let previous_record_closed_world = final_record_is_clean_frame_end;
        final_record_is_clean_frame_end = false;
        let header = match parse_compact_record_header(&raw[offset..]) {
            Ok(header) => header,
            Err(RecordFramingError::TruncatedHeader(_)) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: truncated compact record header"));
                break;
            }
            Err(RecordFramingError::BadMagic) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: invalid compact record magic"));
                break;
            }
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                break;
            }
        };
        match previous_sequence {
            None if header.sequence != 1 => summary.ordering_errors.push(format!(
                "offset {offset}: sequence {} does not start at 1 (stream prefix is missing)",
                header.sequence
            )),
            Some(previous) => match previous.checked_add(1) {
                Some(expected) if header.sequence != expected => {
                    summary.ordering_errors.push(format!(
                        "offset {offset}: sequence discontinuity {previous} -> {} (expected {expected})",
                        header.sequence
                    ));
                }
                None => summary.ordering_errors.push(format!(
                    "offset {offset}: sequence space exhausted after {previous}; got {} (wrapping is not allowed)",
                    header.sequence
                )),
                _ => {}
            },
            _ => {}
        }
        previous_sequence = Some(header.sequence);

        let record_len = header.record_len();
        if record_len > raw.len().saturating_sub(offset) {
            summary.parse_errors.push(format!(
                "offset {offset}: payload length {} exceeds remaining stream bytes",
                header.payload_len
            ));
            break;
        }
        let payload_start = offset + COMPACT_RECORD_HEADER_LEN;
        let payload_end = payload_start + header.payload_len;
        let payload = &raw[payload_start..payload_end];
        let mut repeat_reconstruction: Option<Vec<u8>> = None;
        let record_semantics_ok = match validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            payload,
        ) {
            Ok(()) => {
                match repeat_baselines.observe(header.domain_id, header.record_type, payload) {
                    Ok(reconstructed) => repeat_reconstruction = reconstructed,
                    Err(error) => {
                        summary
                            .parse_errors
                            .push(format!("offset {offset}: {error}"));
                    }
                }
                true
            }
            Err(error) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: {error}"));
                false
            }
        };
        summary.record_count = summary.record_count.saturating_add(1);

        let known_record_type =
            match compact_contract::KnownRecordType::try_from(header.record_type) {
                Ok(record_type) => record_type,
                Err(unknown) => {
                    summary.parse_errors.push(format!(
                        "offset {offset}: unknown compact record type {}",
                        unknown.0
                    ));
                    offset += record_len;
                    continue;
                }
            };
        match known_record_type {
            compact_contract::KnownRecordType::DomainPerf => {
                observe_perf_hour(
                    &mut summary,
                    &mut pending_perf_hour,
                    open_frame_hour,
                    last_closed_hour,
                    header.game_hour_raw,
                    offset,
                );
                if header.domain_id == DOMAIN_ID {
                    summary.domain_perf_records = summary.domain_perf_records.saturating_add(1);
                    *per_hour_perf.entry(header.game_hour_raw).or_default() += 1;
                    if record_semantics_ok {
                        summary.status_flags_or |= read_u32_le(payload, 32);
                    }
                }
            }
            compact_contract::KnownRecordType::DomainData if header.domain_id == DOMAIN_ID => {
                observe_data_hour(
                    &mut summary,
                    pending_perf_hour,
                    &mut open_frame_hour,
                    last_closed_hour,
                    header.game_hour_raw,
                    offset,
                );
                summary.domain_data_records = summary.domain_data_records.saturating_add(1);
                *per_hour_data.entry(header.game_hour_raw).or_default() += 1;
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
                if payload_summary.resource_flow_trailer_present {
                    summary.resource_flow_records = summary.resource_flow_records.saturating_add(1);
                }
                summary.resource_flow_rows_total = summary
                    .resource_flow_rows_total
                    .saturating_add(payload_summary.resource_flow_rows as u64);
                summary.resource_flow_country_rows_total = summary
                    .resource_flow_country_rows_total
                    .saturating_add(payload_summary.resource_flow_country_rows as u64);
                summary.resource_flow_exported_rows_total = summary
                    .resource_flow_exported_rows_total
                    .saturating_add(payload_summary.resource_flow_exported_rows as u64);
                if payload_summary.factory_header_trailer_present {
                    summary.factory_header_records =
                        summary.factory_header_records.saturating_add(1);
                }
                summary.factory_header_rows_total = summary
                    .factory_header_rows_total
                    .saturating_add(payload_summary.factory_header_rows as u64);
                if payload_summary.resource_row_trailer_present {
                    summary.resource_row_records = summary.resource_row_records.saturating_add(1);
                }
                summary.resource_rows_total = summary
                    .resource_rows_total
                    .saturating_add(payload_summary.resource_rows as u64);
                if payload_summary.classic_deal_trailer_present {
                    summary.classic_deal_records = summary.classic_deal_records.saturating_add(1);
                }
                summary.classic_deal_rows_total = summary
                    .classic_deal_rows_total
                    .saturating_add(payload_summary.classic_deal_rows as u64);
                if payload_summary.activation_trailer_present {
                    summary.activation_records = summary.activation_records.saturating_add(1);
                    match payload_summary.activation {
                        Some(activation) => {
                            if summary.activation_first_active.is_none() {
                                summary.activation_first_active = Some(activation.active);
                            }
                            if let Some(previous_active) = summary.activation_last_active {
                                if previous_active != activation.active {
                                    summary.activation_state_transitions =
                                        summary.activation_state_transitions.saturating_add(1);
                                    if summary.activation_last_sequence.is_some_and(|sequence| {
                                        activation.observation_sequence <= sequence
                                    }) {
                                        summary.activation_sequence_errors.push(format!(
                                            "offset {offset}: MTRA state changed without a strictly increasing lifecycle sequence"
                                        ));
                                    }
                                } else if summary.activation_last_sequence.is_some_and(|sequence| {
                                    activation.observation_sequence < sequence
                                }) {
                                    summary.activation_sequence_errors.push(format!(
                                        "offset {offset}: MTRA lifecycle sequence regressed"
                                    ));
                                }
                            }
                            if !activation.active && !summary.activation_saw_active_after_closed {
                                summary.activation_saw_closed_before_active = true;
                            } else if activation.active
                                && summary.activation_saw_closed_before_active
                            {
                                summary.activation_saw_active_after_closed = true;
                            } else if !activation.active
                                && summary.activation_saw_active_after_closed
                            {
                                summary.activation_saw_closed_after_active = true;
                            }
                            summary.activation_last_active = Some(activation.active);
                            summary.activation_last_sequence =
                                Some(activation.observation_sequence);
                            if activation.active {
                                summary.activation_active_records =
                                    summary.activation_active_records.saturating_add(1);
                            } else {
                                summary.activation_closed_records =
                                    summary.activation_closed_records.saturating_add(1);
                            }
                        }
                        None => {}
                    }
                }
                summary
                    .parse_errors
                    .extend(payload_summary.parse_errors.iter().cloned());
                summary.payloads.push(payload_summary);
            }
            compact_contract::KnownRecordType::DomainData => {
                observe_data_hour(
                    &mut summary,
                    pending_perf_hour,
                    &mut open_frame_hour,
                    last_closed_hour,
                    header.game_hour_raw,
                    offset,
                );
            }
            // A repeat covers its hour like DATA and satisfies the frame's
            // data authority: the reconstruction above is verified against
            // this walker's own baseline, so the d43 grammar runs on exactly
            // the bytes the original full record carried.
            compact_contract::KnownRecordType::DomainDataRepeat => {
                observe_data_hour(
                    &mut summary,
                    pending_perf_hour,
                    &mut open_frame_hour,
                    last_closed_hour,
                    header.game_hour_raw,
                    offset,
                );
                if header.domain_id == DOMAIN_ID {
                    summary.domain_data_records = summary.domain_data_records.saturating_add(1);
                    *per_hour_data.entry(header.game_hour_raw).or_default() += 1;
                    let Some(reconstructed) = repeat_reconstruction.as_ref() else {
                        summary
                            .parse_errors
                            .push("DOMAIN_DATA_REPEAT resolved to no payload".to_string());
                        break;
                    };
                    let payload_summary =
                        decode_payload(offset + COMPACT_RECORD_HEADER_LEN, reconstructed);
                    summary.status_flags_or |= payload_summary.status_flags;
                    summary.row_count_total = summary
                        .row_count_total
                        .saturating_add(payload_summary.row_count as u64);
                }
            }
            compact_contract::KnownRecordType::FrameEnd => {
                summary.frame_end_records = summary.frame_end_records.saturating_add(1);
                let mut clean = record_semantics_ok;
                if record_semantics_ok {
                    summary.status_flags_or |= read_u32_le(payload, 24);
                }
                match open_frame_hour {
                    Some(open) if open == header.game_hour_raw => {
                        if record_semantics_ok {
                            observed_closure_hours.insert(header.game_hour_raw);
                        }
                    }
                    Some(open) => {
                        clean = false;
                        summary.boundary_errors.push(format!(
                            "offset {offset}: FRAME_END hour {} does not close open DATA frame hour {open}",
                            header.game_hour_raw
                        ));
                    }
                    None => {
                        clean = false;
                        summary.boundary_errors.push(format!(
                            "offset {offset}: FRAME_END hour {} has no open DATA frame",
                            header.game_hour_raw
                        ));
                    }
                }
                if let Some(closed) = last_closed_hour {
                    match closed.checked_add(1) {
                        Some(expected) if header.game_hour_raw == expected => {}
                        Some(expected) => {
                            clean = false;
                            summary.boundary_errors.push(format!(
                                "offset {offset}: FRAME_END hour {} does not follow closed frame hour {closed} (expected {expected})",
                                header.game_hour_raw
                            ));
                        }
                        None => {
                            clean = false;
                            summary.boundary_errors.push(format!(
                                "offset {offset}: closed frame hour {closed} exhausted; another frame is forbidden"
                            ));
                        }
                    }
                }
                if clean {
                    clean_closed_hours.insert(header.game_hour_raw);
                    open_frame_hour = None;
                    pending_perf_hour = None;
                    last_closed_hour = Some(header.game_hour_raw);
                    final_record_is_clean_frame_end = true;
                }
            }
            compact_contract::KnownRecordType::FreezeCensus => {
                if let Ok(census) =
                    compact_contract::freeze_census_record::FreezeCensusRecordV1::decode(payload)
                {
                    final_record_is_clean_frame_end = compact_contract::freeze_census_record::release_epilogue_closes_stream_policy(previous_record_closed_world, census.phase(), release_epilogue_seen);
                    release_epilogue_seen |= census.phase()
                        == compact_contract::freeze_census_record::FreezeCensusPhase::Release;
                }
            }
            compact_contract::KnownRecordType::ContentIdentityCensus
            | compact_contract::KnownRecordType::StateTopologyCensus
            | compact_contract::KnownRecordType::StreamHeader => {}
            compact_contract::KnownRecordType::DomainGap
            | compact_contract::KnownRecordType::FrameRefused
            | compact_contract::KnownRecordType::FrameDegraded
            | compact_contract::KnownRecordType::CaptureTerminated
            | compact_contract::KnownRecordType::ClockUnreadableInterval
            | compact_contract::KnownRecordType::UntrustedClockRecovery => {
                summary.parse_errors.push(format!(
                    "offset {offset}: record type {} is outside the market proof grammar",
                    known_record_type.canonical_name()
                ))
            }
        }
        offset += record_len;
    }

    if let Some(open) = open_frame_hour {
        summary
            .boundary_errors
            .push(format!("stream ended with open DATA frame hour {open}"));
    } else if let Some(pending) = pending_perf_hour {
        summary.boundary_errors.push(format!(
            "stream ended with pending PERF frame hour {pending} without DATA/FRAME_END"
        ));
    }
    if !final_record_is_clean_frame_end {
        summary
            .boundary_errors
            .push("stream is not closed by a final canonical FRAME_END record".to_string());
    }

    let mut certified_hours = BTreeSet::new();
    for hour in clean_closed_hours {
        let perf_count = per_hour_perf.get(&hour).copied().unwrap_or(0);
        let data_count = per_hour_data.get(&hour).copied().unwrap_or(0);
        if perf_count != 1 {
            summary.boundary_errors.push(format!(
                "game_hour_raw {hour}: market_trade_live PERF record count {perf_count} != 1"
            ));
        }
        if data_count != 1 {
            summary.boundary_errors.push(format!(
                "game_hour_raw {hour}: market_trade_live DATA record count {data_count} != 1"
            ));
        }
        if perf_count == 1 && data_count == 1 {
            certified_hours.insert(hour);
        }
    }
    summary.first_game_hour_raw = certified_hours.first().copied();
    summary.last_game_hour_raw = certified_hours.last().copied();
    summary.unique_game_hours = certified_hours.len();
    summary.skipped_game_hours = skipped_hours(&observed_closure_hours);
    summary
}

fn validate_next_frame_hour(
    summary: &mut MarketTradeStreamSummary,
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
        Some(expected) => summary.boundary_errors.push(format!(
            "offset {offset}: {record_label} hour {game_hour_raw} follows closed frame hour {closed}; expected {expected}"
        )),
        None => summary.boundary_errors.push(format!(
            "offset {offset}: closed frame hour {closed} exhausted; another record is forbidden"
        )),
    }
}

fn observe_perf_hour(
    summary: &mut MarketTradeStreamSummary,
    pending_perf_hour: &mut Option<i32>,
    open_frame_hour: Option<i32>,
    last_closed_hour: Option<i32>,
    game_hour_raw: i32,
    offset: usize,
) {
    match open_frame_hour {
        Some(open) if open != game_hour_raw => summary.boundary_errors.push(format!(
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
                Some(pending) if pending != game_hour_raw => {
                    summary.boundary_errors.push(format!(
                        "offset {offset}: DOMAIN_PERF hour {game_hour_raw} conflicts with pending frame hour {pending}"
                    ));
                }
                Some(_) => {}
                None => *pending_perf_hour = Some(game_hour_raw),
            }
        }
    }
}

fn observe_data_hour(
    summary: &mut MarketTradeStreamSummary,
    pending_perf_hour: Option<i32>,
    open_frame_hour: &mut Option<i32>,
    last_closed_hour: Option<i32>,
    game_hour_raw: i32,
    offset: usize,
) {
    match *open_frame_hour {
        Some(open) if open != game_hour_raw => summary.boundary_errors.push(format!(
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
                    summary.boundary_errors.push(format!(
                        "offset {offset}: DOMAIN_DATA hour {game_hour_raw} conflicts with pending PERF hour {pending}"
                    ));
                }
            }
            *open_frame_hour = Some(game_hour_raw);
        }
    }
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
    if !summary.boundary_errors.is_empty() {
        problems.push("boundary_errors");
    }
    if !summary.activation_sequence_errors.is_empty() {
        problems.push("activation_sequence_errors");
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
    let rejected_status =
        compact_contract::compact_status_failure_flags_after_expected_partial_tolerance(
            DOMAIN_ID,
            summary.status_flags_or,
        );
    if rejected_status & compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE != 0 {
        problems.push("state_unavailable");
    }
    if rejected_status & compact_contract::COMPACT_STATUS_PROFILE_MISMATCH != 0 {
        problems.push("profile_mismatch");
    }
    if rejected_status & compact_contract::COMPACT_STATUS_NO_CLOCK != 0 {
        problems.push("no_clock");
    }
    if rejected_status & compact_contract::COMPACT_STATUS_NO_SCALAR_SOURCE != 0 {
        problems.push("no_scalar_source");
    }
    if rejected_status & STATUS_WORKER_PANIC != 0 {
        problems.push("worker_panic");
    }
    if rejected_status & STATUS_BRIDGE_UNAVAILABLE != 0 {
        problems.push("bridge_unavailable");
    }
    if rejected_status & STATUS_SOURCE_UNAVAILABLE != 0 {
        problems.push("source_unavailable");
    }
    if rejected_status & STATUS_PARTIAL_DATA != 0 {
        problems.push("partial_data_status");
    }
    if rejected_status & compact_contract::COMPACT_STATUS_CONTROL_INVALID != 0 {
        problems.push("control_invalid");
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

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append_test_record_with_flags(
        stream: &mut Vec<u8>,
        sequence: u64,
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) {
        let record_start = stream.len();
        append_compact_record(stream, sequence, 100, domain_id, record_type, payload);
        stream[record_start + 28..record_start + 32].copy_from_slice(&record_flags.to_le_bytes());
    }

    fn canonical_perf_payload() -> Vec<u8> {
        vec![0; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN]
    }

    fn canonical_perf_payload_with_status(status_flags: u32) -> Vec<u8> {
        let mut payload = canonical_perf_payload();
        payload[32..36].copy_from_slice(&status_flags.to_le_bytes());
        payload
    }

    fn canonical_perf_payload_with_pdu1() -> Vec<u8> {
        let name = b"rows";
        let mut payload = canonical_perf_payload();
        payload.extend_from_slice(compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC);
        payload.extend_from_slice(
            &compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes(),
        );
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&(name.len() as u16).to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&7u64.to_le_bytes());
        payload.extend_from_slice(name);
        payload
    }

    fn canonical_frame_end_payload() -> [u8; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN] {
        [0; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN]
    }

    fn canonical_frame_end_payload_with_status(
        status_flags: u32,
    ) -> [u8; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN] {
        let mut payload = canonical_frame_end_payload();
        payload[24..28].copy_from_slice(&status_flags.to_le_bytes());
        payload
    }

    #[derive(Clone, Copy, Debug)]
    enum TestStatusSource {
        Data,
        Perf,
        FrameEnd,
    }

    fn gate_with_status(source: TestStatusSource, status_flags: u32) -> MarketTradeGate {
        let perf_status = matches!(source, TestStatusSource::Perf)
            .then_some(status_flags)
            .unwrap_or(0);
        let data_status = matches!(source, TestStatusSource::Data)
            .then_some(status_flags)
            .unwrap_or(0);
        let frame_status = matches!(source, TestStatusSource::FrameEnd)
            .then_some(status_flags)
            .unwrap_or(0);
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &canonical_perf_payload_with_status(perf_status),
        );
        append_compact_record(
            &mut stream,
            2,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_payload(0, data_status, 1, 1, &[MarketTradeRow::complete(1, 2, 3)]),
        );
        append_compact_record(
            &mut stream,
            3,
            100,
            compact_contract::COMPACT_DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &canonical_frame_end_payload_with_status(frame_status),
        );
        gate_compact_stream(&stream, 1)
    }

    fn complete_market_payload(seed: u32) -> Vec<u8> {
        encode_payload(
            seed as u64,
            0,
            1,
            1,
            &[MarketTradeRow::complete(seed, 2, 3)],
        )
    }

    fn append_test_mtrr(payload: &mut Vec<u8>, countries: &[u32]) {
        append_test_mtrr_with_count(
            payload,
            countries,
            MARKET_TRADE_FIXED7_RESOURCE_COUNT,
            MARKET_TRADE_RESOURCE_ROW_SCHEMA_VERSION,
        );
    }

    fn append_test_mtrr_with_count(
        payload: &mut Vec<u8>,
        countries: &[u32],
        resource_count: u32,
        version: u16,
    ) {
        let tokens = (0..resource_count)
            .map(|slot| format!("res_{slot}"))
            .collect::<Vec<_>>();
        append_test_mtrr_with_identity(payload, countries, resource_count, &tokens, version);
    }

    fn append_test_mtrr_with_identity(
        payload: &mut Vec<u8>,
        countries: &[u32],
        resource_count: u32,
        tokens: &[String],
        version: u16,
    ) {
        append_test_mtrr_with_oil_slot(payload, countries, resource_count, tokens, version, 0);
    }

    fn append_test_mtrr_with_oil_slot(
        payload: &mut Vec<u8>,
        countries: &[u32],
        resource_count: u32,
        tokens: &[String],
        version: u16,
        oil_slot: u32,
    ) {
        let carries_identity = version == MARKET_TRADE_RESOURCE_ROW_SCHEMA_VERSION;
        payload.extend_from_slice(MARKET_TRADE_RESOURCE_ROW_TRAILER_MAGIC);
        payload.extend_from_slice(&version.to_le_bytes());
        payload.extend_from_slice(&(MARKET_TRADE_RESOURCE_ROW_HEADER_LEN as u16).to_le_bytes());
        payload.extend_from_slice(&(MARKET_TRADE_RESOURCE_ROW_LEN as u16).to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&((countries.len() as u32) * resource_count).to_le_bytes());
        payload.extend_from_slice(&(countries.len() as u32).to_le_bytes());
        payload.extend_from_slice(&resource_count.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(
            &(if carries_identity {
                STRATEGIC_RESOURCE_IDENTITY_STRIDE as u32
            } else {
                0
            })
            .to_le_bytes(),
        );
        if carries_identity {
            for token in tokens {
                let bytes = token.as_bytes();
                payload.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
                let mut body = vec![0u8; MAX_STRATEGIC_RESOURCE_TOKEN_LEN];
                let copied = bytes.len().min(MAX_STRATEGIC_RESOURCE_TOKEN_LEN);
                body[..copied].copy_from_slice(&bytes[..copied]);
                payload.extend_from_slice(&body);
            }
        }
        for &tag_db_id in countries {
            for resource_index in 0..resource_count {
                let oil = resource_index == oil_slot;
                let balance = if oil { -10 } else { resource_index as i32 };
                for value in [
                    tag_db_id,
                    resource_index,
                    resource_index,
                    if oil {
                        MARKET_TRADE_RESOURCE_NEEDED_CURRENT_MAX
                    } else {
                        MARKET_TRADE_RESOURCE_NEEDED_SURPLUS
                    },
                ] {
                    payload.extend_from_slice(&value.to_le_bytes());
                }
                for value in [
                    balance,
                    100 + resource_index as i32,
                    10,
                    -5,
                    2,
                    3,
                    if oil { -12 } else { balance },
                    if oil { -12 } else { 0 },
                    if oil { -8 } else { 0 },
                ] {
                    payload.extend_from_slice(&value.to_le_bytes());
                }
                payload.extend_from_slice(
                    &(if oil {
                        MARKET_TRADE_RESOURCE_FLAGS_KNOWN
                    } else {
                        0
                    })
                    .to_le_bytes(),
                );
                payload.extend_from_slice(&(resource_index + 1).to_le_bytes());
                payload.extend_from_slice(&0u32.to_le_bytes());
            }
        }
    }

    fn append_test_mtrd(payload: &mut Vec<u8>, mismatch_second_half: bool) {
        append_test_mtrd_versioned(
            payload,
            mismatch_second_half,
            MARKET_TRADE_CLASSIC_DEAL_SCHEMA_VERSION,
            (47, 51),
        );
    }

    fn append_test_mtrd_versioned(
        payload: &mut Vec<u8>,
        mismatch_second_half: bool,
        version: u16,
        identity: (u32, u32),
    ) {
        let witnesses = [47u32, 51, 99];
        payload.extend_from_slice(MARKET_TRADE_CLASSIC_DEAL_TRAILER_MAGIC);
        payload.extend_from_slice(&version.to_le_bytes());
        payload.extend_from_slice(&(MARKET_TRADE_CLASSIC_DEAL_HEADER_LEN as u16).to_le_bytes());
        payload.extend_from_slice(&(MARKET_TRADE_CLASSIC_DEAL_ROW_LEN as u16).to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&(witnesses.len() as u32).to_le_bytes());
        payload.extend_from_slice(&MARKET_TRADE_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&(witnesses.len() as u32).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        for witness in witnesses {
            payload.extend_from_slice(&witness.to_le_bytes());
        }
        let wide_identity = version == MARKET_TRADE_CLASSIC_DEAL_WIDE_IDENTITY_SCHEMA_VERSION;
        let (identity_bytes, ordinal_word): ([u8; 8], u32) = if wide_identity {
            let mut bytes = [0u8; 8];
            bytes[..4].copy_from_slice(&identity.0.to_le_bytes());
            bytes[4..].copy_from_slice(&identity.1.to_le_bytes());
            (bytes, 1)
        } else {
            let packed =
                (u64::from(identity.0) << 32) | (u64::from(identity.1) << 16) | (4u64 << 8) | 1;
            (packed.to_le_bytes(), 0)
        };
        for (index, owner, counterpart, direction) in [
            (
                0usize,
                47u32,
                51u32,
                MARKET_TRADE_CLASSIC_DEAL_DIRECTION_EXPORT,
            ),
            (
                1usize,
                51u32,
                47u32,
                MARKET_TRADE_CLASSIC_DEAL_DIRECTION_IMPORT,
            ),
        ] {
            for value in [
                owner,
                counterpart,
                4,
                0,
                direction,
                MARKET_TRADE_CLASSIC_DEAL_ROUTE_SEA,
            ] {
                payload.extend_from_slice(&value.to_le_bytes());
            }
            payload.extend_from_slice(&MARKET_TRADE_CLASSIC_DEAL_FLAG_EXACT.to_le_bytes());
            payload.extend_from_slice(
                &(MARKET_TRADE_CLASSIC_DEAL_STATUS_NO_TRADE_INFLUENCE
                    | MARKET_TRADE_CLASSIC_DEAL_STATUS_NO_RESOURCES)
                    .to_le_bytes(),
            );
            payload.extend_from_slice(&identity_bytes);
            payload.extend_from_slice(&3_200_000i64.to_le_bytes());
            payload.extend_from_slice(
                &(if mismatch_second_half && index == 1 {
                    2_700_001i64
                } else {
                    2_700_000i64
                })
                .to_le_bytes(),
            );
            payload.extend_from_slice(&9_600_000i64.to_le_bytes());
            payload.extend_from_slice(&25_000i64.to_le_bytes());
            payload.extend_from_slice(&84_375i64.to_le_bytes());
            payload.extend_from_slice(&8i32.to_le_bytes());
            payload.extend_from_slice(&14i32.to_le_bytes());
            payload.extend_from_slice(&18i32.to_le_bytes());
            payload.extend_from_slice(&ordinal_word.to_le_bytes());
        }
    }

    fn append_complete_market_frame(stream: &mut Vec<u8>, next_sequence: &mut u64, hour: i32) {
        append_compact_record(
            stream,
            *next_sequence,
            hour,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &canonical_perf_payload(),
        );
        *next_sequence += 1;
        append_compact_record(
            stream,
            *next_sequence,
            hour,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &complete_market_payload(hour as u32),
        );
        *next_sequence += 1;
        append_compact_record(
            stream,
            *next_sequence,
            hour,
            compact_contract::COMPACT_DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &canonical_frame_end_payload(),
        );
        *next_sequence += 1;
    }

    fn summarize_sequences(sequences: &[u64]) -> MarketTradeStreamSummary {
        let mut stream = Vec::new();
        for &sequence in sequences {
            append_compact_record(
                &mut stream,
                sequence,
                100,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_PERF,
                &canonical_perf_payload(),
            );
        }
        summarize_compact_stream(&stream)
    }

    fn summarize_perf_record(record_flags: u32, payload: &[u8]) -> MarketTradeStreamSummary {
        let mut stream = Vec::new();
        append_test_record_with_flags(
            &mut stream,
            1,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            record_flags,
            payload,
        );
        summarize_compact_stream(&stream)
    }

    #[test]
    fn compact_stream_rejects_missing_sequence_prefix() {
        let summary = summarize_sequences(&[2]);

        assert!(summary
            .ordering_errors
            .iter()
            .any(|error| error.contains("sequence 2 does not start at 1")));
    }

    #[test]
    fn compact_stream_rejects_sequence_gap() {
        let summary = summarize_sequences(&[1, 3]);

        assert!(summary
            .ordering_errors
            .iter()
            .any(|error| error.contains("sequence discontinuity 1 -> 3 (expected 2)")));
    }

    #[test]
    fn compact_stream_rejects_repeated_sequence() {
        let summary = summarize_sequences(&[1, 1]);

        assert!(summary
            .ordering_errors
            .iter()
            .any(|error| error.contains("sequence discontinuity 1 -> 1 (expected 2)")));
    }

    #[test]
    fn compact_stream_rejects_sequence_regression() {
        let summary = summarize_sequences(&[1, 2, 1]);

        assert!(summary
            .ordering_errors
            .iter()
            .any(|error| error.contains("sequence discontinuity 2 -> 1 (expected 3)")));
    }

    #[test]
    fn compact_stream_rejects_sequence_wrap_after_max() {
        let summary = summarize_sequences(&[u64::MAX, 0]);

        assert!(summary.ordering_errors.iter().any(|error| {
            error.contains("sequence space exhausted after 18446744073709551615")
                && error.contains("wrapping is not allowed")
        }));
    }

    #[test]
    fn record_flag_policy_accepts_canonical_schema_and_perf_flags() {
        let mut stream = Vec::new();
        append_test_record_with_flags(
            &mut stream,
            1,
            45,
            RECORD_TYPE_DOMAIN_DATA,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &[],
        );
        append_test_record_with_flags(
            &mut stream,
            2,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &canonical_perf_payload_with_pdu1(),
        );

        let summary = summarize_compact_stream(&stream);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn record_flag_policy_rejects_wrong_placement_and_unknown_bits() {
        let mut stream = Vec::new();
        append_test_record_with_flags(
            &mut stream,
            1,
            45,
            RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &canonical_perf_payload(),
        );
        append_test_record_with_flags(
            &mut stream,
            2,
            compact_contract::COMPACT_DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            1 << 29,
            &canonical_frame_end_payload(),
        );

        let summary = summarize_compact_stream(&stream);
        let flag_errors = summary
            .parse_errors
            .iter()
            .filter(|error| error.contains("record flags"))
            .collect::<Vec<_>>();
        assert_eq!(flag_errors.len(), 2, "{:?}", summary.parse_errors);
        assert!(flag_errors.iter().any(|error| error.contains("0x80000000")));
        assert!(flag_errors.iter().any(|error| error.contains("0x20000000")));
    }

    #[test]
    fn record_semantics_accepts_exact_pdu1_trailer() {
        let summary = summarize_perf_record(
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &canonical_perf_payload_with_pdu1(),
        );

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn record_semantics_rejects_perf_trailer_flag_without_trailer() {
        let summary = summarize_perf_record(
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &canonical_perf_payload(),
        );

        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("diagnostic trailer header is truncated")));
    }

    #[test]
    fn record_semantics_rejects_unannounced_perf_trailer() {
        let summary = summarize_perf_record(0, &canonical_perf_payload_with_pdu1());

        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("unannounced trailer byte")));
    }

    #[test]
    fn record_semantics_rejects_malformed_pdu1_trailers() {
        let mut bad_magic = canonical_perf_payload_with_pdu1();
        bad_magic[compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN] = b'X';

        let mut truncated_header = canonical_perf_payload_with_pdu1();
        truncated_header.truncate(compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN + 4);

        let mut nonzero_unit_reserved = canonical_perf_payload_with_pdu1();
        nonzero_unit_reserved[compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN + 10] = 1;

        for (payload, expected_error) in [
            (bad_magic, "diagnostic trailer magic is invalid"),
            (truncated_header, "diagnostic trailer header is truncated"),
            (
                nonzero_unit_reserved,
                "diagnostic unit 0 reserved field is nonzero",
            ),
        ] {
            let summary = summarize_perf_record(
                compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
                &payload,
            );
            assert!(
                summary
                    .parse_errors
                    .iter()
                    .any(|error| error.contains(expected_error)),
                "expected {expected_error:?}, got {:?}",
                summary.parse_errors
            );
        }
    }

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
        assert_eq!(decoded.residual_source_blocked_rows, 1);
        assert_eq!(decoded.residual_schema_blocked_rows, 0);
        assert_eq!(decoded.residual_owned_elsewhere_rows, 0);
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
            .any(|badge| badge.name == "country_resource_import_export_flows"
                && badge.status == RESIDUAL_STATUS_VALIDATED_SLICE
                && badge.current_support == RESIDUAL_SUPPORT_VALIDATED_SLICE));
        assert!(badges
            .iter()
            .any(|badge| badge.name == "market_access_diplomatic_context"
                && badge.status == RESIDUAL_STATUS_VALIDATED_SLICE
                && badge.current_support == RESIDUAL_SUPPORT_VALIDATED_SLICE));
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
            RECORD_TYPE_DOMAIN_PERF,
            &canonical_perf_payload(),
        );
        append_compact_record(
            &mut stream,
            2,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload,
        );
        append_compact_record(
            &mut stream,
            3,
            100,
            compact_contract::COMPACT_DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &canonical_frame_end_payload(),
        );

        let gate = gate_compact_stream(&stream, 1);

        assert!(gate.ok, "{:?}", gate.problems);
        assert_eq!(gate.summary.residual_badge_records, 1);
        assert_eq!(gate.summary.residual_badge_rows_total, badges.len() as u64);
        assert_eq!(gate.summary.residual_source_blocked_rows_total, 1);
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
    fn resource_flow_trailer_decodes_after_detail_trailer() {
        let detail = PurchaseContractDetailRow {
            contract_id: 985,
            flags: DETAIL_FLAG_PRICE_VALID | DETAIL_FLAG_AMOUNT_VALID,
            price_raw: 250_000,
            amount_raw: 30_000_000,
            ..PurchaseContractDetailRow::default()
        };
        let resource_rows = six_kind_flow_rows(
            67,
            &[800_000, 0, 0, 1_900_000, 15_499_941, 800_000, 12_000_000],
        );
        let mut payload = encode_payload_with_detail_rows(0, 0, 0, 0, &[], &[detail]);
        payload.extend_from_slice(
            &encode_resource_flow_trailer(&resource_rows, 7).expect("seven-wide rows encode"),
        );

        let decoded = decode_payload(0, &payload);

        assert!(decoded.ok(), "{:?}", decoded.parse_errors);
        assert!(decoded.detail_trailer_present);
        assert!(decoded.resource_flow_trailer_present);
        assert_eq!(decoded.resource_flow_rows, 6);
        assert_eq!(decoded.resource_flow_country_rows, 1);
        assert_eq!(decoded.resource_flow_exported_rows, 1);
        assert_eq!(
            decoded.resource_flow_kind_mask,
            MARKET_TRADE_RESOURCE_FLOW_KIND_MASK
        );
        assert_eq!(decoded.resource_flow_rows_decoded, resource_rows);

        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload,
        );
        append_compact_record(
            &mut stream,
            2,
            100,
            compact_contract::COMPACT_DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &canonical_frame_end_payload(),
        );
        let summary = summarize_compact_stream(&stream);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.detail_records, 1);
        assert_eq!(summary.resource_flow_records, 1);
        assert_eq!(summary.resource_flow_rows_total, 6);
        assert_eq!(summary.resource_flow_country_rows_total, 1);
        assert_eq!(summary.resource_flow_exported_rows_total, 1);
    }

    fn six_kind_flow_rows(tag_db_id: u32, exported: &[i64]) -> Vec<ResourceFlowRow> {
        let resource_count = exported.len() as u32;
        (1..=6)
            .map(|flow_kind| ResourceFlowRow {
                tag_db_id,
                flow_kind,
                resource_mask_or_count: resource_count,
                values_raw: if flow_kind == MARKET_TRADE_RESOURCE_FLOW_KIND_EXPORTED {
                    exported.to_vec()
                } else {
                    vec![0; exported.len()]
                },
            })
            .collect()
    }

    #[test]
    fn resource_flow_trailer_preserves_complete_all_zero_country() {
        let resource_rows = six_kind_flow_rows(67, &[0; 7]);
        let mut payload = encode_payload(0, 0, 0, 0, &[]);
        payload.extend_from_slice(
            &encode_resource_flow_trailer(&resource_rows, 7).expect("seven-wide rows encode"),
        );

        let decoded = decode_payload(0, &payload);

        assert!(decoded.ok(), "{:?}", decoded.parse_errors);
        assert_eq!(decoded.resource_flow_rows_decoded, resource_rows);
    }

    #[test]
    fn factory_header_trailer_decodes_complete_country_snapshot() {
        let rows = vec![FactoryHeaderRow {
            tag_db_id: 67,
            total: 128,
            used: 41,
            available: 73,
        }];
        let mut payload = encode_payload(0, STATUS_SOURCE_UNAVAILABLE, 0, 0, &[]);
        payload.extend_from_slice(&encode_factory_header_trailer(&rows));

        let decoded = decode_payload(0, &payload);

        assert!(decoded.factory_header_trailer_present);
        assert_eq!(decoded.factory_header_rows, 1);
        assert_eq!(decoded.factory_headers, rows);
        assert!(
            decoded.parse_errors.is_empty(),
            "{:?}",
            decoded.parse_errors
        );
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
    fn canonical_failure_bits_fail_from_data_perf_and_frame_end_status() {
        let fatal_cases = [
            (
                compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE,
                "state_unavailable",
            ),
            (
                compact_contract::COMPACT_STATUS_PROFILE_MISMATCH,
                "profile_mismatch",
            ),
            (compact_contract::COMPACT_STATUS_NO_CLOCK, "no_clock"),
            (
                compact_contract::COMPACT_STATUS_NO_SCALAR_SOURCE,
                "no_scalar_source",
            ),
            (
                compact_contract::COMPACT_STATUS_WORKER_PANIC,
                "worker_panic",
            ),
            (
                compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE,
                "bridge_unavailable",
            ),
            (
                compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
                "source_unavailable",
            ),
            (
                compact_contract::COMPACT_STATUS_PARTIAL_DATA,
                "partial_data_status",
            ),
            (
                compact_contract::COMPACT_STATUS_CONTROL_INVALID,
                "control_invalid",
            ),
        ];
        assert_eq!(
            fatal_cases.iter().fold(0u32, |mask, (flag, _)| mask | flag),
            compact_contract::COMPACT_STATUS_FAILURE_MASK
        );

        for source in [
            TestStatusSource::Data,
            TestStatusSource::Perf,
            TestStatusSource::FrameEnd,
        ] {
            for (flag, expected_problem) in fatal_cases {
                let gate = gate_with_status(source, flag);
                assert!(
                    !gate.ok && gate.problems.contains(&expected_problem),
                    "{source:?} flag {flag:#x}: {:?}",
                    gate.problems
                );
                assert_eq!(gate.summary.status_flags_or, flag, "{source:?}");
            }
        }
    }

    #[test]
    fn informational_status_is_nonfatal_and_d43_has_no_partial_tolerance() {
        assert_eq!(compact_contract::lane_local_status_mask(DOMAIN_ID), 0);
        assert_eq!(
            compact_contract::expected_partial_tolerance(DOMAIN_ID),
            None
        );
        assert_eq!(
            compact_contract::compact_status_failure_flags_after_expected_partial_tolerance(
                DOMAIN_ID,
                compact_contract::COMPACT_STATUS_PARTIAL_DATA,
            ),
            compact_contract::COMPACT_STATUS_PARTIAL_DATA
        );

        let informational = compact_contract::COMPACT_STATUS_WAR_BRIDGE_APPLIED
            | compact_contract::COMPACT_STATUS_EXTRACTION_PERF_GATE
            | compact_contract::COMPACT_STATUS_DELTA_DATA
            | compact_contract::COMPACT_STATUS_TRANSITION_BASELINE_RESET
            | compact_contract::COMPACT_STATUS_WAR_SEGMENT_ABSENT;
        assert_eq!(
            informational & compact_contract::COMPACT_STATUS_FAILURE_MASK,
            0
        );
        for source in [
            TestStatusSource::Data,
            TestStatusSource::Perf,
            TestStatusSource::FrameEnd,
        ] {
            let gate = gate_with_status(source, informational);
            assert!(gate.ok, "{source:?}: {:?}", gate.problems);
            assert_eq!(gate.summary.status_flags_or, informational, "{source:?}");
        }
    }

    #[test]
    fn compact_stream_gate_rejects_market_data_without_frame_end() {
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            20,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &canonical_perf_payload(),
        );
        append_compact_record(
            &mut stream,
            2,
            20,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &complete_market_payload(1),
        );

        let gate = gate_compact_stream(&stream, 1);

        assert!(!gate.ok);
        assert!(gate.problems.contains(&"boundary_errors"));
        assert_eq!(gate.summary.unique_game_hours, 0);
        assert!(gate
            .summary
            .boundary_errors
            .iter()
            .any(|error| error.contains("open DATA frame hour 20")));
        assert!(gate
            .summary
            .boundary_errors
            .iter()
            .any(|error| error.contains("final canonical FRAME_END")));
    }

    #[test]
    fn bare_and_unrelated_frames_cannot_inflate_market_duration() {
        let mut bare = Vec::new();
        append_compact_record(
            &mut bare,
            1,
            20,
            compact_contract::COMPACT_DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &canonical_frame_end_payload(),
        );
        let bare_gate = gate_compact_stream(&bare, 1);
        assert!(!bare_gate.ok);
        assert_eq!(bare_gate.summary.unique_game_hours, 0);
        assert!(bare_gate
            .summary
            .boundary_errors
            .iter()
            .any(|error| error.contains("has no open DATA frame")));

        let mut unrelated = Vec::new();
        append_compact_record(
            &mut unrelated,
            1,
            20,
            18,
            RECORD_TYPE_DOMAIN_PERF,
            &canonical_perf_payload(),
        );
        append_compact_record(
            &mut unrelated,
            2,
            20,
            18,
            RECORD_TYPE_DOMAIN_DATA,
            &[0xde, 0xad, 0xbe, 0xef],
        );
        append_compact_record(
            &mut unrelated,
            3,
            20,
            compact_contract::COMPACT_DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &canonical_frame_end_payload(),
        );
        let unrelated_gate = gate_compact_stream(&unrelated, 1);
        assert!(!unrelated_gate.ok);
        assert_eq!(unrelated_gate.summary.unique_game_hours, 0);
        assert!(unrelated_gate
            .summary
            .boundary_errors
            .iter()
            .any(|error| { error.contains("market_trade_live PERF record count 0 != 1") }));
        assert!(unrelated_gate
            .summary
            .boundary_errors
            .iter()
            .any(|error| { error.contains("market_trade_live DATA record count 0 != 1") }));
    }

    #[test]
    fn compact_stream_gate_requires_exactly_one_market_perf_and_data_per_frame() {
        for (name, perf_count, data_count, expected) in [
            ("missing_perf", 0usize, 1usize, "PERF record count 0 != 1"),
            ("duplicate_perf", 2, 1, "PERF record count 2 != 1"),
            ("missing_data", 1, 0, "DATA record count 0 != 1"),
            ("duplicate_data", 1, 2, "DATA record count 2 != 1"),
        ] {
            let mut stream = Vec::new();
            let mut sequence = 1u64;
            for _ in 0..perf_count {
                append_compact_record(
                    &mut stream,
                    sequence,
                    30,
                    DOMAIN_ID,
                    RECORD_TYPE_DOMAIN_PERF,
                    &canonical_perf_payload(),
                );
                sequence += 1;
            }
            for data_index in 0..data_count {
                append_compact_record(
                    &mut stream,
                    sequence,
                    30,
                    DOMAIN_ID,
                    RECORD_TYPE_DOMAIN_DATA,
                    &complete_market_payload(data_index as u32 + 1),
                );
                sequence += 1;
            }
            if data_count == 0 {
                append_compact_record(
                    &mut stream,
                    sequence,
                    30,
                    18,
                    RECORD_TYPE_DOMAIN_DATA,
                    &[0xaa],
                );
                sequence += 1;
            }
            append_compact_record(
                &mut stream,
                sequence,
                30,
                compact_contract::COMPACT_DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &canonical_frame_end_payload(),
            );

            let gate = gate_compact_stream(&stream, 1);
            assert!(!gate.ok, "{name} unexpectedly passed");
            assert_eq!(gate.summary.unique_game_hours, 0, "{name}");
            assert!(
                gate.summary
                    .boundary_errors
                    .iter()
                    .any(|error| error.contains(expected)),
                "{name}: expected {expected:?}, got {:?}",
                gate.summary.boundary_errors
            );
        }
    }

    #[test]
    fn compact_stream_gate_rejects_wrong_hour_perf() {
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            40,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &canonical_perf_payload(),
        );
        append_compact_record(
            &mut stream,
            2,
            41,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &complete_market_payload(1),
        );
        append_compact_record(
            &mut stream,
            3,
            41,
            compact_contract::COMPACT_DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &canonical_frame_end_payload(),
        );

        let gate = gate_compact_stream(&stream, 1);

        assert!(!gate.ok);
        assert!(gate
            .summary
            .boundary_errors
            .iter()
            .any(|error| error.contains("conflicts with pending PERF hour 40")));
        assert_eq!(gate.summary.unique_game_hours, 0);
    }

    #[test]
    fn compact_stream_gate_rejects_skipped_and_reordered_closures() {
        let mut skipped = Vec::new();
        let mut sequence = 1;
        append_complete_market_frame(&mut skipped, &mut sequence, 50);
        append_complete_market_frame(&mut skipped, &mut sequence, 52);
        let skipped_gate = gate_compact_stream(&skipped, 2);
        assert!(!skipped_gate.ok);
        assert_eq!(skipped_gate.summary.skipped_game_hours, vec![51]);
        assert!(skipped_gate.summary.boundary_errors.iter().any(|error| {
            error.contains("FRAME_END hour 52 does not follow closed frame hour 50")
        }));

        let mut reordered = Vec::new();
        append_compact_record(
            &mut reordered,
            1,
            60,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &canonical_perf_payload(),
        );
        append_compact_record(
            &mut reordered,
            2,
            60,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &complete_market_payload(1),
        );
        append_compact_record(
            &mut reordered,
            3,
            59,
            compact_contract::COMPACT_DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &canonical_frame_end_payload(),
        );
        let reordered_gate = gate_compact_stream(&reordered, 1);
        assert!(!reordered_gate.ok);
        assert!(reordered_gate.summary.boundary_errors.iter().any(|error| {
            error.contains("FRAME_END hour 59 does not close open DATA frame hour 60")
        }));
    }

    #[test]
    fn compact_stream_gate_rejects_trailing_record_after_clean_frame() {
        let mut stream = Vec::new();
        let mut sequence = 1;
        append_complete_market_frame(&mut stream, &mut sequence, 70);
        append_compact_record(
            &mut stream,
            sequence,
            71,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &canonical_perf_payload(),
        );

        let gate = gate_compact_stream(&stream, 1);

        assert!(!gate.ok);
        assert!(gate
            .summary
            .boundary_errors
            .iter()
            .any(|error| error.contains("pending PERF frame hour 71")));
        assert!(gate
            .summary
            .boundary_errors
            .iter()
            .any(|error| error.contains("final canonical FRAME_END")));

        let mut trailing_bytes = Vec::new();
        let mut sequence = 1;
        append_complete_market_frame(&mut trailing_bytes, &mut sequence, 70);
        trailing_bytes.extend_from_slice(&[0xaa, 0xbb]);
        let byte_tail_gate = gate_compact_stream(&trailing_bytes, 1);
        assert!(!byte_tail_gate.ok);
        assert!(byte_tail_gate
            .summary
            .parse_errors
            .iter()
            .any(|error| error.contains("truncated compact record header")));
        assert!(byte_tail_gate
            .summary
            .boundary_errors
            .iter()
            .any(|error| error.contains("final canonical FRAME_END")));
    }

    #[test]
    fn unowned_domain_payload_is_ignored_inside_valid_market_frame() {
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            80,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &canonical_perf_payload(),
        );
        append_compact_record(
            &mut stream,
            2,
            80,
            18,
            RECORD_TYPE_DOMAIN_DATA,
            &[0xde, 0xad, 0xbe, 0xef],
        );
        append_compact_record(
            &mut stream,
            3,
            80,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &complete_market_payload(1),
        );
        append_compact_record(
            &mut stream,
            4,
            80,
            compact_contract::COMPACT_DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &canonical_frame_end_payload(),
        );

        let gate = gate_compact_stream(&stream, 1);

        assert!(gate.ok, "{:?}", gate.summary);
        assert_eq!(gate.summary.unique_game_hours, 1);
        assert_eq!(gate.summary.domain_perf_records, 1);
        assert_eq!(gate.summary.domain_data_records, 1);
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
            RECORD_TYPE_DOMAIN_PERF,
            &canonical_perf_payload(),
        );
        append_compact_record(
            &mut stream,
            2,
            10,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload_a,
        );
        append_compact_record(
            &mut stream,
            3,
            10,
            compact_contract::COMPACT_DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &canonical_frame_end_payload(),
        );
        append_compact_record(
            &mut stream,
            4,
            12,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &canonical_perf_payload(),
        );
        append_compact_record(
            &mut stream,
            5,
            12,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload_b,
        );
        append_compact_record(
            &mut stream,
            6,
            12,
            compact_contract::COMPACT_DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &canonical_frame_end_payload(),
        );

        let gate = gate_compact_stream(&stream, 2);

        assert!(!gate.ok);
        assert!(gate.problems.contains(&"skipped_game_hours"));
        assert_eq!(gate.summary.skipped_game_hours, vec![11]);
    }

    #[test]
    fn compact_stream_gate_accepts_complete_consecutive_hours() {
        let mut stream = Vec::new();
        for (sequence, hour) in [(1, 20), (4, 21), (7, 22)] {
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
                RECORD_TYPE_DOMAIN_PERF,
                &canonical_perf_payload(),
            );
            append_compact_record(
                &mut stream,
                sequence + 1,
                hour,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_DATA,
                &payload,
            );
            append_compact_record(
                &mut stream,
                sequence + 2,
                hour,
                compact_contract::COMPACT_DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &canonical_frame_end_payload(),
            );
        }

        let gate = gate_compact_stream(&stream, 3);

        assert!(gate.ok, "{:?}", gate.problems);
        assert_eq!(gate.summary.domain_data_records, 3);
        assert_eq!(gate.summary.unique_game_hours, 3);
        assert_eq!(gate.summary.partial_field_rows_total, 0);
    }

    #[test]
    fn x15_mtrd_reads_the_wide_identity_and_still_reads_the_packed_one() {
        let deals = |version: u16, identity: (u32, u32)| {
            let mut payload = complete_market_payload(1);
            append_test_mtrr(&mut payload, &[47, 51, 99]);
            append_test_mtrd_versioned(&mut payload, false, version, identity);
            decode_payload(0, &payload)
        };

        for version in [
            MARKET_TRADE_CLASSIC_DEAL_SCHEMA_VERSION,
            MARKET_TRADE_CLASSIC_DEAL_PER_OWNER_SCHEMA_VERSION,
            MARKET_TRADE_CLASSIC_DEAL_WIDE_IDENTITY_SCHEMA_VERSION,
        ] {
            let decoded = deals(version, (47, 51));
            assert!(
                decoded.parse_errors.is_empty(),
                "MTRD v{version} must parse clean: {:?}",
                decoded.parse_errors
            );
            assert_eq!(
                decoded.classic_deals.len(),
                2,
                "MTRD v{version} carries a deal's two halves"
            );
            assert!(
                decoded
                    .classic_deals
                    .iter()
                    .all(|row| row.identity.exporter_tag_db_id == 47
                        && row.identity.importer_tag_db_id == 51
                        && row.identity.ordinal == 1),
                "MTRD v{version} normalises to one identity: {:?}",
                decoded.classic_deals
            );
        }

        let torn = deals(
            MARKET_TRADE_CLASSIC_DEAL_WIDE_IDENTITY_SCHEMA_VERSION,
            (47, 99),
        );
        assert!(
            torn.parse_errors
                .iter()
                .any(|error| error.contains("invalid shape")),
            "a v4 identity naming a country the row does not must be refused: {:?}",
            torn.parse_errors
        );
    }

    #[test]
    fn x15_mtrr_mtrd_decode_complete_rows_and_zero_deal_witness() {
        let mut payload = complete_market_payload(1);
        append_test_mtrr(&mut payload, &[47, 51, 99]);
        append_test_mtrd(&mut payload, false);

        let decoded = decode_payload(0, &payload);

        assert!(
            decoded.parse_errors.is_empty(),
            "{:?}",
            decoded.parse_errors
        );
        assert!(decoded.resource_row_trailer_present);
        assert_eq!(decoded.resource_row_country_count, 3);
        assert_eq!(decoded.resource_view_rows.len(), 21);
        assert_eq!(
            decoded.resource_tokens,
            (0..7).map(|slot| format!("res_{slot}")).collect::<Vec<_>>()
        );
        assert!(decoded.classic_deal_trailer_present);
        assert_eq!(decoded.classic_deal_country_count, 3);
        assert_eq!(decoded.classic_deals.len(), 2);
        assert!(decoded.classic_deals.iter().all(|row| {
            row.route_status_flags
                == Some(
                    MARKET_TRADE_CLASSIC_DEAL_STATUS_NO_TRADE_INFLUENCE
                        | MARKET_TRADE_CLASSIC_DEAL_STATUS_NO_RESOURCES,
                )
        }));
        assert!(decoded.classic_deal_country_witnesses.contains(&99));
        assert!(!decoded
            .classic_deals
            .iter()
            .any(|row| row.owner_tag_db_id == 99));
    }

    #[test]
    fn x15_mtrd_v1_retains_route_flags_as_unavailable() {
        let mut payload = complete_market_payload(1);
        let trailer = payload.len();
        append_test_mtrd(&mut payload, false);
        payload[trailer + 4..trailer + 6]
            .copy_from_slice(&MARKET_TRADE_CLASSIC_DEAL_LEGACY_SCHEMA_VERSION.to_le_bytes());
        let first_row = trailer + MARKET_TRADE_CLASSIC_DEAL_HEADER_LEN + 3 * 4;
        for row in [first_row, first_row + MARKET_TRADE_CLASSIC_DEAL_ROW_LEN] {
            payload[row + 28..row + 32].copy_from_slice(&0u32.to_le_bytes());
        }

        let decoded = decode_payload(0, &payload);

        assert!(
            decoded.parse_errors.is_empty(),
            "{:?}",
            decoded.parse_errors
        );
        assert!(decoded
            .classic_deals
            .iter()
            .all(|row| row.route_status_flags.is_none()));
    }

    #[test]
    fn x15_mtrd_v2_rejects_unknown_route_status_bits() {
        let mut payload = complete_market_payload(1);
        let trailer = payload.len();
        append_test_mtrd(&mut payload, false);
        let first_row = trailer + MARKET_TRADE_CLASSIC_DEAL_HEADER_LEN + 3 * 4;
        payload[first_row + 28..first_row + 32].copy_from_slice(&4u32.to_le_bytes());

        let decoded = decode_payload(0, &payload);

        assert!(decoded
            .parse_errors
            .iter()
            .any(|error| error.contains("MTRD row 0 has invalid shape")));
    }

    #[test]
    fn x15_mtrd_rejects_reciprocal_half_value_disagreement() {
        let mut payload = complete_market_payload(1);
        append_test_mtrd(&mut payload, true);

        let decoded = decode_payload(0, &payload);

        assert!(decoded
            .parse_errors
            .iter()
            .any(|error| error.contains("reciprocal halves disagree")));
    }

    #[test]
    fn x15_mtrr_rejects_incomplete_visible_order() {
        let mut payload = complete_market_payload(1);
        let trailer = payload.len();
        append_test_mtrr(&mut payload, &[47]);
        let final_row_visible_order = trailer
            + MARKET_TRADE_RESOURCE_ROW_HEADER_LEN
            + 7 * STRATEGIC_RESOURCE_IDENTITY_STRIDE
            + 6 * MARKET_TRADE_RESOURCE_ROW_LEN
            + 8;
        payload[final_row_visible_order..final_row_visible_order + 4]
            .copy_from_slice(&0u32.to_le_bytes());

        let decoded = decode_payload(0, &payload);

        assert!(decoded
            .parse_errors
            .iter()
            .any(|error| error.contains("visible order is not complete")));
    }

    #[test]
    fn b1_mts1_preserves_ordered_rows_and_measured_empty_country() {
        let countries = [
            MarketStockpileCountryRow {
                tag_db_id: 34,
                item_first: 0,
                item_count: 2,
            },
            MarketStockpileCountryRow {
                tag_db_id: 67,
                item_first: 2,
                item_count: 0,
            },
        ];
        let items = [
            MarketStockpileItemRow {
                equipment_variant_id: 90,
                equipment_type: 70,
                ordinal: 0,
                amount_raw: 76_700_000,
            },
            MarketStockpileItemRow {
                equipment_variant_id: 91,
                equipment_type: 70,
                ordinal: 1,
                amount_raw: 0,
            },
        ];
        let mut payload = complete_market_payload(1);
        payload.extend_from_slice(&encode_market_stockpile_trailer(2, &countries, &items));

        let decoded = decode_payload(0, &payload);
        assert!(
            decoded.parse_errors.is_empty(),
            "{:?}",
            decoded.parse_errors
        );
        assert!(decoded.stockpile_trailer_present);
        assert_eq!(decoded.stockpile_country_rows, 2);
        assert_eq!(decoded.stockpile_item_rows, 2);
        assert_eq!(decoded.stockpile_countries, countries);
        assert_eq!(decoded.stockpile_items, items);
        assert_eq!(decoded.stockpile_countries[1].item_count, 0);
    }

    #[test]
    fn b1_mts1_rejects_incomplete_coverage_and_noncontiguous_nested_ranges() {
        let countries = [
            MarketStockpileCountryRow {
                tag_db_id: 34,
                item_first: 0,
                item_count: 1,
            },
            MarketStockpileCountryRow {
                tag_db_id: 67,
                item_first: 1,
                item_count: 0,
            },
        ];
        let items = [MarketStockpileItemRow {
            equipment_variant_id: 90,
            equipment_type: 70,
            ordinal: 0,
            amount_raw: 100_000,
        }];
        let mut wrong_count = complete_market_payload(1);
        wrong_count.extend_from_slice(&encode_market_stockpile_trailer(3, &countries, &items));
        let decoded = decode_payload(0, &wrong_count);
        assert!(decoded
            .parse_errors
            .iter()
            .any(|error| error.contains("invalid MTS1 header shape")));

        let mut noncontiguous = complete_market_payload(1);
        let mut trailer = encode_market_stockpile_trailer(2, &countries, &items);
        let second_first =
            MARKET_TRADE_STOCKPILE_HEADER_LEN + MARKET_TRADE_STOCKPILE_COUNTRY_ROW_LEN + 4;
        trailer[second_first..second_first + 4].copy_from_slice(&0u32.to_le_bytes());
        noncontiguous.extend_from_slice(&trailer);
        let decoded = decode_payload(0, &noncontiguous);
        assert!(decoded.parse_errors.iter().any(|error| {
            error.contains("MTS1 country row 1 has invalid tag/range/completeness")
        }));
    }

    #[test]
    fn d43_mtrf_v3_round_trips_an_eight_resource_vector() {
        let exported = [
            800_000i64, 0, 0, 1_900_000, 15_499_941, 800_000, 12_000_000, -4_250_000,
        ];
        let rows = six_kind_flow_rows(67, &exported);
        let mut payload = encode_payload(0, 0, 0, 0, &[]);
        payload.extend_from_slice(
            &encode_resource_flow_trailer(&rows, 8).expect("eight-wide rows encode"),
        );

        let decoded = decode_payload(0, &payload);

        assert!(decoded.ok(), "{:?}", decoded.parse_errors);
        assert_eq!(decoded.resource_flow_resource_count, 8);
        assert_eq!(decoded.resource_flow_rows, 6);
        assert_eq!(decoded.resource_flow_country_rows, 1);
        assert_eq!(decoded.resource_flow_rows_decoded, rows);
        let exported_row = decoded
            .resource_flow_rows_decoded
            .iter()
            .find(|row| row.flow_kind == MARKET_TRADE_RESOURCE_FLOW_KIND_EXPORTED)
            .expect("exported row");
        assert_eq!(exported_row.values_raw, exported.to_vec());
        assert_eq!(exported_row.resource_mask_or_count, 8);
    }

    #[test]
    fn d43_mtrf_v3_round_trips_the_seven_resource_vector() {
        let exported = [800_000i64, 0, 0, 1_900_000, 15_499_941, 800_000, 12_000_000];
        let rows = six_kind_flow_rows(67, &exported);
        let mut payload = encode_payload(0, 0, 0, 0, &[]);
        payload.extend_from_slice(
            &encode_resource_flow_trailer(&rows, 7).expect("seven-wide rows encode"),
        );

        let decoded = decode_payload(0, &payload);

        assert!(decoded.ok(), "{:?}", decoded.parse_errors);
        assert_eq!(decoded.resource_flow_resource_count, 7);
        assert_eq!(decoded.resource_flow_rows_decoded, rows);
        assert_eq!(
            resource_flow_row_len(7),
            MARKET_TRADE_RESOURCE_FLOW_FIXED7_ROW_LEN,
            "a seven-wide v3 row is the same width the fixed row always was"
        );
    }

    #[test]
    fn d43_mtrf_v2_fixed_seven_stream_is_still_read() {
        let mut payload = encode_payload(0, 0, 0, 0, &[]);
        payload.extend_from_slice(&encode_fixed7_resource_flow_trailer(
            67,
            MARKET_TRADE_RESOURCE_FLOW_FIXED7_SCHEMA_VERSION,
        ));

        let decoded = decode_payload(0, &payload);

        assert!(decoded.ok(), "{:?}", decoded.parse_errors);
        assert_eq!(decoded.resource_flow_resource_count, 7);
        assert_eq!(
            decoded.resource_flow_resource_mask,
            MARKET_TRADE_RESOURCE_FLOW_RESOURCE_MASK
        );
        assert_eq!(decoded.resource_flow_rows, 6);
        for row in &decoded.resource_flow_rows_decoded {
            assert_eq!(row.values_raw.len(), 7);
        }
    }

    #[test]
    fn d43_mtrr_v1_fixed_seven_stream_is_still_read() {
        let mut payload = complete_market_payload(1);
        append_test_mtrr_with_count(
            &mut payload,
            &[47],
            MARKET_TRADE_FIXED7_RESOURCE_COUNT,
            MARKET_TRADE_RESOURCE_ROW_LEGACY_SCHEMA_VERSION,
        );

        let decoded = decode_payload(0, &payload);

        assert!(decoded.ok(), "{:?}", decoded.parse_errors);
        assert_eq!(decoded.resource_row_resource_count, 7);
        assert_eq!(decoded.resource_view_rows.len(), 7);
    }

    #[test]
    fn d43_mtrr_resolves_oil_at_its_real_slot_when_a_mod_reorders_the_table() {
        let tokens: Vec<String> = [
            "aether", "bauxite", "cobalt", "oil", "steel", "rubber", "tungsten",
        ]
        .iter()
        .map(|token| (*token).to_string())
        .collect();
        let mut payload = complete_market_payload(1);
        append_test_mtrr_with_oil_slot(
            &mut payload,
            &[47, 51],
            7,
            &tokens,
            MARKET_TRADE_RESOURCE_ROW_SCHEMA_VERSION,
            3,
        );

        let decoded = decode_payload(0, &payload);

        assert!(decoded.ok(), "{:?}", decoded.parse_errors);
        assert_eq!(decoded.resource_tokens, tokens);
        assert_eq!(decoded.resource_view_rows.len(), 14);
        let oil_rows: Vec<&ResourceViewRow> = decoded
            .resource_view_rows
            .iter()
            .filter(|row| row.needed_type == MARKET_TRADE_RESOURCE_NEEDED_CURRENT_MAX)
            .collect();
        assert_eq!(oil_rows.len(), 2, "one oil row per country");
        for row in oil_rows {
            assert_eq!(row.resource_index, 3);
            assert_eq!(row.flags, MARKET_TRADE_RESOURCE_FLAGS_KNOWN);
            assert_eq!(row.needed_current, -12);
            assert_eq!(row.needed_max, -8);
        }
        for row in decoded
            .resource_view_rows
            .iter()
            .filter(|row| row.resource_index == 0)
        {
            assert_eq!(row.flags, 0);
            assert_eq!(row.needed_current, 0);
            assert_eq!(row.needed_max, 0);
        }
    }

    #[test]
    fn d43_mtrr_refuses_a_trailer_that_states_two_oil_slots() {
        let tokens: Vec<String> = (0..7).map(|slot| format!("res_{slot}")).collect();
        let mut payload = complete_market_payload(1);
        let mtrr_at = payload.len();
        append_test_mtrr_with_oil_slot(
            &mut payload,
            &[47],
            7,
            &tokens,
            MARKET_TRADE_RESOURCE_ROW_SCHEMA_VERSION,
            2,
        );
        assert!(
            decode_payload(0, &payload).ok(),
            "the single-oil-slot fixture is accepted"
        );

        let rows_at =
            mtrr_at + MARKET_TRADE_RESOURCE_ROW_HEADER_LEN + 7 * STRATEGIC_RESOURCE_IDENTITY_STRIDE;
        let second_oil = rows_at + 5 * MARKET_TRADE_RESOURCE_ROW_LEN;
        payload[second_oil + 12..second_oil + 16]
            .copy_from_slice(&MARKET_TRADE_RESOURCE_NEEDED_CURRENT_MAX.to_le_bytes());
        payload[second_oil + 40..second_oil + 44].copy_from_slice(&(-12i32).to_le_bytes());
        payload[second_oil + 44..second_oil + 48].copy_from_slice(&(-12i32).to_le_bytes());
        payload[second_oil + 48..second_oil + 52].copy_from_slice(&(-8i32).to_le_bytes());
        payload[second_oil + 52..second_oil + 56]
            .copy_from_slice(&MARKET_TRADE_RESOURCE_FLAGS_KNOWN.to_le_bytes());

        let decoded = decode_payload(0, &payload);

        assert!(!decoded.ok());
        assert!(
            decoded
                .parse_errors
                .iter()
                .any(|error| error.contains("MTRR states 2 oil slots")),
            "{:?}",
            decoded.parse_errors
        );
    }

    #[test]
    fn d43_mtrf_refuses_a_version_it_does_not_know_and_never_reinterprets_it() {
        let rows = six_kind_flow_rows(67, &[0; 7]);
        let mut trailer = encode_resource_flow_trailer(&rows, 7).expect("seven-wide rows encode");
        let future = MARKET_TRADE_RESOURCE_FLOW_SCHEMA_VERSION + 1;
        trailer[4..6].copy_from_slice(&future.to_le_bytes());
        let mut payload = encode_payload(0, 0, 0, 0, &[]);
        payload.extend_from_slice(&trailer);

        let decoded = decode_payload(0, &payload);

        assert!(!decoded.ok());
        assert!(
            decoded.parse_errors.iter().any(|error| {
                error.contains("unsupported resource-flow trailer version")
                    && error.contains(&future.to_string())
            }),
            "refusal must name the version: {:?}",
            decoded.parse_errors
        );
        assert!(
            decoded.resource_flow_rows_decoded.is_empty(),
            "an unknown version must decode no rows at all"
        );
    }

    #[test]
    fn d43_mtrr_refuses_a_version_it_does_not_know() {
        let mut payload = complete_market_payload(1);
        append_test_mtrr_with_count(
            &mut payload,
            &[47],
            MARKET_TRADE_FIXED7_RESOURCE_COUNT,
            MARKET_TRADE_RESOURCE_ROW_SCHEMA_VERSION + 1,
        );

        let decoded = decode_payload(0, &payload);

        assert!(!decoded.ok());
        assert!(
            decoded
                .parse_errors
                .iter()
                .any(|error| error.contains("invalid MTRR shape version")),
            "{:?}",
            decoded.parse_errors
        );
        assert!(decoded.resource_view_rows.is_empty());
    }

    #[test]
    fn d43_mtrf_v3_refuses_a_resource_count_above_the_admission_bound() {
        let rows = six_kind_flow_rows(67, &[0; 7]);
        let mut trailer = encode_resource_flow_trailer(&rows, 7).expect("seven-wide rows encode");
        let over = MAX_STRATEGIC_RESOURCE_COUNT + 1;
        trailer[24..28].copy_from_slice(&over.to_le_bytes());
        let mut payload = encode_payload(0, 0, 0, 0, &[]);
        payload.extend_from_slice(&trailer);

        let decoded = decode_payload(0, &payload);

        assert!(!decoded.ok());
        assert!(
            decoded.parse_errors.iter().any(|error| {
                error.contains("resource_flow_resource_count")
                    && error.contains(&format!("outside 1..={MAX_STRATEGIC_RESOURCE_COUNT}"))
            }),
            "{:?}",
            decoded.parse_errors
        );
        assert!(decoded.resource_flow_rows_decoded.is_empty());
    }

    #[test]
    fn d43_mtrr_v2_refuses_a_resource_count_outside_the_admission_bound() {
        for count in [0, MAX_STRATEGIC_RESOURCE_COUNT + 1] {
            let mut payload = complete_market_payload(1);
            append_test_mtrr_with_count(
                &mut payload,
                &[47],
                count,
                MARKET_TRADE_RESOURCE_ROW_SCHEMA_VERSION,
            );

            let decoded = decode_payload(0, &payload);

            assert!(!decoded.ok(), "count {count} must be refused");
            assert!(
                decoded
                    .parse_errors
                    .iter()
                    .any(|error| error.contains("invalid MTRR shape")),
                "count {count}: {:?}",
                decoded.parse_errors
            );
            assert!(decoded.resource_view_rows.is_empty());
        }
    }

    #[test]
    fn d43_resource_flow_encoder_refuses_a_count_outside_the_admission_bound() {
        for count in [0, MAX_STRATEGIC_RESOURCE_COUNT + 1] {
            let rows = six_kind_flow_rows(67, &vec![0i64; count as usize]);
            assert!(
                encode_resource_flow_trailer(&rows, count).is_none(),
                "count {count} must not encode"
            );
        }
    }

    #[test]
    fn d43_resource_flow_encoder_refuses_rows_that_disagree_with_the_count() {
        let mut rows = six_kind_flow_rows(67, &[0; 7]);
        rows[3].values_raw.push(0);
        assert!(encode_resource_flow_trailer(&rows, 7).is_none());
        assert!(encode_resource_flow_trailer(&rows, 8).is_none());
    }

    #[test]
    fn d43_resource_flow_encoder_refuses_rows_the_decoder_would_refuse() {
        let mut echo_disagrees = six_kind_flow_rows(67, &[0; 7]);
        echo_disagrees[2].resource_mask_or_count = 6;
        assert!(
            encode_resource_flow_trailer(&echo_disagrees, 7).is_none(),
            "a row echoing a count the header does not state must not encode"
        );

        for bad_kind in [0u32, 7] {
            let mut bad_flow_kind = six_kind_flow_rows(67, &[0; 7]);
            bad_flow_kind[4].flow_kind = bad_kind;
            assert!(
                encode_resource_flow_trailer(&bad_flow_kind, 7).is_none(),
                "flow_kind {bad_kind} must not encode"
            );
        }
    }

    #[test]
    fn d43_mtrf_v3_refuses_a_row_whose_count_disagrees_with_the_header() {
        let rows = six_kind_flow_rows(67, &[0; 7]);
        let mut trailer = encode_resource_flow_trailer(&rows, 7).expect("seven-wide rows encode");
        let row2 = MARKET_TRADE_RESOURCE_FLOW_HEADER_LEN + 2 * resource_flow_row_len(7);
        trailer[row2 + 8..row2 + 12].copy_from_slice(&6u32.to_le_bytes());
        let mut payload = encode_payload(0, 0, 0, 0, &[]);
        payload.extend_from_slice(&trailer);

        let decoded = decode_payload(0, &payload);

        assert!(!decoded.ok());
        assert!(
            decoded
                .parse_errors
                .iter()
                .any(|error| error.contains("resource-flow row 2 states")),
            "{:?}",
            decoded.parse_errors
        );
    }

    fn encode_fixed7_resource_flow_trailer(tag_db_id: u32, version: u16) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(MARKET_TRADE_RESOURCE_FLOW_TRAILER_MAGIC);
        payload.extend_from_slice(&version.to_le_bytes());
        payload.extend_from_slice(&(MARKET_TRADE_RESOURCE_FLOW_HEADER_LEN as u16).to_le_bytes());
        payload
            .extend_from_slice(&(MARKET_TRADE_RESOURCE_FLOW_FIXED7_ROW_LEN as u16).to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&6u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&MARKET_TRADE_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&MARKET_TRADE_RESOURCE_FLOW_RESOURCE_MASK.to_le_bytes());
        payload.extend_from_slice(&MARKET_TRADE_RESOURCE_FLOW_KIND_MASK.to_le_bytes());
        for flow_kind in 1u32..=6 {
            payload.extend_from_slice(&tag_db_id.to_le_bytes());
            payload.extend_from_slice(&flow_kind.to_le_bytes());
            payload.extend_from_slice(&MARKET_TRADE_RESOURCE_FLOW_RESOURCE_MASK.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            for slot in 0..7i64 {
                payload.extend_from_slice(&(slot * 1_000).to_le_bytes());
            }
        }
        payload
    }
}
