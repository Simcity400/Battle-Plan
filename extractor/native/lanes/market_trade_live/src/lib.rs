use std::collections::{BTreeMap, BTreeSet};

use compact_contract::{
    parse_compact_record_header, validate_compact_record_semantics, RecordFramingError,
};

pub const DOMAIN_ID: u16 = 43;
pub const DOMAIN_NAME: &str = "market_trade_live";

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const RECORD_TYPE_FRAME_END: u16 = 2;
pub const RECORD_TYPE_DOMAIN_DATA: u16 = 3;

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
/// v1 exported-only slice and v2 six-kind matrix both pin the per-row value
/// vector to seven `i64`s and the header field at `+24` to the seven-bit mask
/// `0x7F`, which is why an eighth strategic resource could not be stated. v3 is
/// the same six-kind matrix with a count-prefixed, variable-width vector.
pub const MARKET_TRADE_RESOURCE_FLOW_LEGACY_SCHEMA_VERSION: u16 = 1;
pub const MARKET_TRADE_RESOURCE_FLOW_FIXED7_SCHEMA_VERSION: u16 = 2;
pub const MARKET_TRADE_RESOURCE_FLOW_SCHEMA_VERSION: u16 = 3;
pub const MARKET_TRADE_RESOURCE_FLOW_HEADER_LEN: usize = 32;
/// v1/v2 only: 16 bytes of row header plus seven `i64`s.
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
/// v1 pinned the header's `resource_count` to seven; v2 admits the count the
/// live resource database reported, under the shared admission ceiling; v3 adds
/// the identity block that states WHICH resources those are.
pub const MARKET_TRADE_RESOURCE_ROW_LEGACY_SCHEMA_VERSION: u16 = 1;
pub const MARKET_TRADE_RESOURCE_ROW_COUNT_ONLY_SCHEMA_VERSION: u16 = 2;
pub const MARKET_TRADE_RESOURCE_ROW_SCHEMA_VERSION: u16 = 3;
/// v4 is v3 plus one extra admitted ROW shape: an oil row whose needed-trio the
/// producer could not derive states that, instead of being dropped and leaving
/// the trailer's dense `country_count * resource_count` cross product short --
/// which is what made every D43 record of live capture t17 unreadable.
pub const MARKET_TRADE_RESOURCE_ROW_NEEDED_UNAVAILABLE_SCHEMA_VERSION: u16 = 4;
pub const MARKET_TRADE_RESOURCE_ROW_HEADER_LEN: usize = 32;
pub const MARKET_TRADE_RESOURCE_ROW_LEN: usize = 64;
/// One `MTRR` v3 identity entry: `u16 token_len` then the token bytes,
/// zero-padded to the bound.
pub const STRATEGIC_RESOURCE_IDENTITY_STRIDE: usize =
    compact_contract::build_profile::COMPACT_STRATEGIC_RESOURCE_IDENTITY_STRIDE as usize;
pub const MAX_STRATEGIC_RESOURCE_TOKEN_LEN: usize =
    compact_contract::build_profile::MAX_COMPACT_STRATEGIC_RESOURCE_TOKEN_LEN;
/// The width v1 `MTRR` and v1/v2 `MTRF` are pinned to.
pub const MARKET_TRADE_FIXED7_RESOURCE_COUNT: u32 = 7;
/// The one shared admission ceiling for a count-prefixed resource vector: the
/// same contract constant the collector and the fold bound their counts with.
pub const MAX_STRATEGIC_RESOURCE_COUNT: u32 =
    compact_contract::build_profile::MAX_COMPACT_STRATEGIC_RESOURCE_COUNT;
pub const MARKET_TRADE_RESOURCE_NEEDED_SURPLUS: u32 = 1;
pub const MARKET_TRADE_RESOURCE_NEEDED_DEFICIT: u32 = 2;
pub const MARKET_TRADE_RESOURCE_NEEDED_CURRENT_MAX: u32 = 3;
/// The oil row's needed-trio was not observed. `v4` only; the trio is zeroed on
/// the wire and every reader must treat it as absent rather than as `0`.
pub const MARKET_TRADE_RESOURCE_NEEDED_UNAVAILABLE: u32 = 4;
pub const MARKET_TRADE_RESOURCE_FLAG_CURRENT: u32 = 1;
pub const MARKET_TRADE_RESOURCE_FLAG_MAX: u32 = 1 << 1;
pub const MARKET_TRADE_RESOURCE_FLAG_NEEDED_UNAVAILABLE: u32 = 1 << 2;
pub const MARKET_TRADE_RESOURCE_FLAGS_KNOWN: u32 =
    MARKET_TRADE_RESOURCE_FLAG_CURRENT | MARKET_TRADE_RESOURCE_FLAG_MAX;
pub const MARKET_TRADE_CLASSIC_DEAL_TRAILER_MAGIC: &[u8; 4] = b"MTRD";
pub const MARKET_TRADE_CLASSIC_DEAL_LEGACY_SCHEMA_VERSION: u16 = 1;
pub const MARKET_TRADE_CLASSIC_DEAL_SCHEMA_VERSION: u16 = 2;
/// v3: the header's last word is the INCOMPLETE-OWNER COUNT and that many
/// census keys sit between the witness block and the rows, ascending and a
/// subset of the witnesses. An owner named there published rows that are true
/// but not exhaustive; every other witness's list is exact. v1/v2 hold the word
/// at zero and state no per-owner claim -- their producers withheld the whole
/// trailer instead of publishing a short list.
pub const MARKET_TRADE_CLASSIC_DEAL_PER_OWNER_SCHEMA_VERSION: u16 = 3;
/// v4: the DEAL IDENTITY is carried at full width — `deal_exporter_tag_db_id`
/// `u32` at `+0x20`, `deal_importer_tag_db_id` `u32` at `+0x24`, and a one-based
/// `deal_ordinal` `u32` in the word v1-v3 hold at zero (`+0x5C`) — in place of
/// v1-v3's single packed `u64` at `+0x20`.
///
/// v1-v3 gave each country id SIXTEEN BITS and the ordinal EIGHT, so the
/// producer dropped any deal whose exporter or importer id exceeded `u16::MAX`,
/// and any deal past the 254th in one export bucket. A country cap and a deal
/// cap, both expressed as field widths rather than as constants. The header,
/// witness block and incomplete block are v3's unchanged.
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
/// B1 exact per-country equipment-market stockpile snapshot. `MTS1` carries
/// every country-owner witness, including measured-empty nested iterators.
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
    /// v1/v2 echo the seven-bit mask here; v3 echoes the resource count.
    pub resource_mask_or_count: u32,
    /// As wide as the resource count the trailer stated.
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

/// One classic deal's identity, at the country id space's full structural
/// width. Ordered so a `BTreeMap` keyed on it walks deals in a stable,
/// content-derived order.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct ClassicDealIdentity {
    pub exporter_tag_db_id: u32,
    pub importer_tag_db_id: u32,
    pub resource_index: u32,
    /// One-based on every version: v4 reads it directly, v1-v3's packed low
    /// byte was already one-based.
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
    /// Exact route `+0xE8/+0xE9` state in MTRD v2. Legacy v1 rows retain
    /// `None` so downstream code cannot mistake an absent observation for
    /// two measured false values.
    pub route_status_flags: Option<u32>,
    /// The deal's identity, normalised across wire versions: v4 reads three
    /// full-width words, v1-v3 unpack it from their `u64`. Both halves of one
    /// deal carry the same value, which is what makes it the join key —
    /// `owner`/`counterpart` swap with `direction` and it does not.
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
    /// Raw `+24` header field: the seven-bit mask in v1/v2, the count in v3.
    pub resource_flow_resource_mask: u32,
    /// Resource-set width the trailer states, whichever version declared it.
    pub resource_flow_resource_count: u32,
    pub resource_flow_kind_mask: u32,
    pub factory_header_trailer_present: bool,
    pub factory_header_rows: u32,
    pub resource_row_trailer_present: bool,
    pub resource_rows: u32,
    pub resource_row_country_count: u32,
    pub resource_row_resource_count: u32,
    /// The resource set's own tokens, in slot order, from an `MTRR` v3 identity
    /// block. Empty for v1/v2: those streams state a width and no identity.
    pub resource_tokens: Vec<String>,
    pub classic_deal_trailer_present: bool,
    pub classic_deal_rows: u32,
    pub classic_deal_country_count: u32,
    /// How many of those witnesses the payload marked NOT exhaustive. Zero for
    /// every v1/v2 trailer.
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
    /// The witnesses the payload marked NOT exhaustive, in wire order.
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
                // The badge quotes the shipped seven-resource row width; a
                // v3 row is `16 + 8 * resource_count` and has no one number.
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

/// Row width of an `MTRF` v3 row for a given resource count. The encoder and
/// `decode_resource_flow_trailer` derive the width from the same function, so a
/// producer cannot write a row length the decoder would not compute.
pub fn resource_flow_row_len(resource_count: u32) -> usize {
    MARKET_TRADE_RESOURCE_FLOW_ROW_HEADER_LEN
        .saturating_add((resource_count as usize).saturating_mul(8))
}

/// Encodes `MTRF` v3, or `None` when the input could only produce a trailer a
/// decoder must refuse.
///
/// The encoder applies the SAME admission predicate the decoders apply, for two
/// reasons: an encoder that can emit an out-of-bound count is a second opinion
/// about what is legal, and `Vec::with_capacity` here is sized from
/// `8 * resource_count`, so an unbounded count sizes an allocation before
/// anything has refused it.
pub fn encode_resource_flow_trailer(
    rows: &[ResourceFlowRow],
    resource_count: u32,
) -> Option<Vec<u8>> {
    if !(1..=MAX_STRATEGIC_RESOURCE_COUNT).contains(&resource_count) {
        return None;
    }
    // Every field the decoder validates per row, the encoder validates before
    // writing it. Checking only `values_raw.len()` left two ways to emit a
    // stream every decoder refuses: a row echoing a different count at `+8`,
    // and a row carrying a flow kind outside the six-kind matrix.
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
    mut trailer_offset: usize,
    summary: &mut MarketTradePayloadSummary,
) {
    while trailer_offset < payload.len() {
        if payload.len().saturating_sub(trailer_offset) < 4 {
            summary.parse_errors.push(format!(
                "offset {offset}: optional trailer is too short to contain magic"
            ));
            return;
        }
        let magic = &payload[trailer_offset..trailer_offset + 4];
        let next = if magic == MARKET_TRADE_RESIDUAL_TRAILER_MAGIC {
            decode_residual_trailer(offset, payload, trailer_offset, summary)
        } else if magic == MARKET_TRADE_DETAIL_TRAILER_MAGIC {
            decode_detail_trailer(offset, payload, trailer_offset, summary)
        } else if magic == MARKET_TRADE_LEND_LEASE_TRAILER_MAGIC {
            decode_lend_lease_trailer(offset, payload, trailer_offset, summary)
        } else if magic == MARKET_TRADE_RESOURCE_FLOW_TRAILER_MAGIC {
            decode_resource_flow_trailer(offset, payload, trailer_offset, summary)
        } else if magic == MARKET_TRADE_FACTORY_HEADER_TRAILER_MAGIC {
            decode_factory_header_trailer(offset, payload, trailer_offset, summary)
        } else if magic == MARKET_TRADE_RESOURCE_ROW_TRAILER_MAGIC {
            decode_resource_row_trailer(offset, payload, trailer_offset, summary)
        } else if magic == MARKET_TRADE_CLASSIC_DEAL_TRAILER_MAGIC {
            decode_classic_deal_trailer(offset, payload, trailer_offset, summary)
        } else if magic == MARKET_TRADE_ACTIVATION_TRAILER_MAGIC {
            decode_activation_trailer(offset, payload, trailer_offset, summary)
        } else if magic == MARKET_TRADE_STOCKPILE_TRAILER_MAGIC {
            if summary.stockpile_trailer_present {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: duplicate MTS1 stockpile trailer"));
                return;
            }
            decode_market_stockpile_trailer(offset, payload, trailer_offset, summary)
        } else {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade optional trailer has unknown magic"
            ));
            return;
        };
        let Some(next_offset) = next else {
            return;
        };
        if next_offset <= trailer_offset {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade optional trailer did not advance"
            ));
            return;
        }
        trailer_offset = next_offset;
    }
}

fn decode_activation_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut MarketTradePayloadSummary,
) -> Option<usize> {
    if summary.activation_trailer_present {
        summary.parse_errors.push(format!(
            "offset {offset}: duplicate MTRA activation trailer"
        ));
        return None;
    }
    summary.activation_trailer_present = true;
    if payload.len().saturating_sub(trailer_offset) < MARKET_TRADE_ACTIVATION_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: MTRA activation trailer is too short"
        ));
        return None;
    }
    let trailer = &payload[trailer_offset..trailer_offset + MARKET_TRADE_ACTIVATION_HEADER_LEN];
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let flags = read_u32_le(trailer, 8);
    let country_tag_db_id = read_u32_le(trailer, 12);
    let observation_sequence = read_u64_le(trailer, 16);
    let root_kind = read_u32_le(trailer, 24);
    let open_rva = read_u32_le(trailer, 28);
    let active = flags & MARKET_TRADE_ACTIVATION_FLAG_ACTIVE != 0;
    if version != MARKET_TRADE_ACTIVATION_SCHEMA_VERSION
        || header_len != MARKET_TRADE_ACTIVATION_HEADER_LEN
        || flags & !MARKET_TRADE_ACTIVATION_FLAGS_KNOWN != 0
        || flags & MARKET_TRADE_ACTIVATION_FLAG_ROOT_EXACT == 0
        || root_kind != MARKET_TRADE_ACTIVATION_ROOT_COUNTRYTRADEVIEW
        || open_rva != MARKET_TRADE_ACTIVATION_OPEN_RVA
        || active != (country_tag_db_id != 0)
        || (active && observation_sequence == 0)
        || country_tag_db_id > MARKET_TRADE_MAX_TAG_DB_ID
    {
        summary.parse_errors.push(format!(
            "offset {offset}: MTRA activation identity/shape is invalid"
        ));
        return None;
    }
    summary.activation = Some(MarketTradeActivation {
        active,
        country_tag_db_id: active.then_some(country_tag_db_id),
        observation_sequence,
    });
    Some(trailer_offset + MARKET_TRADE_ACTIVATION_HEADER_LEN)
}

fn decode_market_stockpile_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut MarketTradePayloadSummary,
) -> Option<usize> {
    summary.stockpile_trailer_present = true;
    if payload.len().saturating_sub(trailer_offset) < MARKET_TRADE_STOCKPILE_HEADER_LEN {
        summary
            .parse_errors
            .push(format!("offset {offset}: MTS1 trailer is too short"));
        return None;
    }
    let trailer = &payload[trailer_offset..];
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let country_row_len = read_u16_le(trailer, 8) as usize;
    let item_row_len = read_u16_le(trailer, 10) as usize;
    let segment_len = read_u32_le(trailer, 12) as usize;
    let country_count = read_u32_le(trailer, 16);
    let item_count = read_u32_le(trailer, 20);
    let expected_country_count = read_u32_le(trailer, 24);
    let scale_raw = read_u32_le(trailer, 28);
    let flags = read_u32_le(trailer, 32);
    let reserved = read_u32_le(trailer, 36);
    summary.stockpile_country_rows = country_count;
    summary.stockpile_item_rows = item_count;
    summary.stockpile_expected_country_count = expected_country_count;
    let expected_len = MARKET_TRADE_STOCKPILE_HEADER_LEN
        .saturating_add(
            (country_count as usize).saturating_mul(MARKET_TRADE_STOCKPILE_COUNTRY_ROW_LEN),
        )
        .saturating_add((item_count as usize).saturating_mul(MARKET_TRADE_STOCKPILE_ITEM_ROW_LEN));
    if version != MARKET_TRADE_STOCKPILE_SCHEMA_VERSION
        || header_len != MARKET_TRADE_STOCKPILE_HEADER_LEN
        || country_row_len != MARKET_TRADE_STOCKPILE_COUNTRY_ROW_LEN
        || item_row_len != MARKET_TRADE_STOCKPILE_ITEM_ROW_LEN
        || segment_len != expected_len
        || country_count == 0
        || country_count != expected_country_count
        || scale_raw != MARKET_TRADE_SCALE_RAW
        || flags != MARKET_TRADE_STOCKPILE_GLOBAL_FLAGS_KNOWN
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary.parse_errors.push(format!(
            "offset {offset}: invalid MTS1 header shape version {version} header_len {header_len} country_row_len {country_row_len} item_row_len {item_row_len} segment_len {segment_len}/{expected_len} countries {country_count}/{expected_country_count} scale {scale_raw} flags {flags:#x} reserved {reserved}"
        ));
        return None;
    }

    let item_base =
        header_len.saturating_add((country_count as usize).saturating_mul(country_row_len));
    let errors_before_country_rows = summary.parse_errors.len();
    let mut seen_tags = BTreeSet::new();
    let mut expected_first = 0u32;
    for row_index in 0..country_count {
        let cursor = header_len + row_index as usize * country_row_len;
        let row = MarketStockpileCountryRow {
            tag_db_id: read_u32_le(trailer, cursor),
            item_first: read_u32_le(trailer, cursor + 4),
            item_count: read_u32_le(trailer, cursor + 8),
        };
        let row_flags = read_u32_le(trailer, cursor + 12);
        let row_reserved = read_u32_le(trailer, cursor + 16);
        let item_end = row.item_first.saturating_add(row.item_count);
        if row.tag_db_id == 0
            || row.tag_db_id > MARKET_TRADE_MAX_TAG_DB_ID
            || !seen_tags.insert(row.tag_db_id)
            || row.item_first != expected_first
            || item_end > item_count
            || row_flags != MARKET_TRADE_STOCKPILE_COUNTRY_COMPLETE
            || row_reserved != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: MTS1 country row {row_index} has invalid tag/range/completeness"
            ));
        }
        expected_first = item_end;
        summary.stockpile_countries.push(row);
    }
    if expected_first != item_count {
        summary.parse_errors.push(format!(
            "offset {offset}: MTS1 country ranges cover {expected_first} item(s), expected {item_count}"
        ));
    }
    if summary.parse_errors.len() != errors_before_country_rows {
        return Some(trailer_offset + segment_len);
    }

    let mut per_country_keys = BTreeSet::new();
    for country in &summary.stockpile_countries {
        for ordinal in 0..country.item_count {
            let item_index = country.item_first.saturating_add(ordinal);
            let cursor = item_base + item_index as usize * item_row_len;
            let row = MarketStockpileItemRow {
                equipment_variant_id: read_u32_le(trailer, cursor),
                equipment_type: read_u32_le(trailer, cursor + 4),
                ordinal: read_u32_le(trailer, cursor + 8),
                amount_raw: read_i64_le(trailer, cursor + 16),
            };
            let item_reserved = read_u32_le(trailer, cursor + 12);
            if row.equipment_variant_id == 0
                || row.equipment_variant_id > MARKET_TRADE_MAX_VARIANT_ID
                || row.equipment_type != MARKET_TRADE_STOCKPILE_EQUIPMENT_TYPE
                || row.ordinal != ordinal
                || row.amount_raw < 0
                || row.amount_raw > MARKET_TRADE_MAX_AMOUNT_RAW
                || item_reserved != 0
                || !per_country_keys.insert((
                    country.tag_db_id,
                    row.equipment_variant_id,
                    row.equipment_type,
                ))
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: MTS1 item {item_index} has invalid identity/order/amount"
                ));
            }
            summary.stockpile_items.push(row);
        }
    }
    Some(trailer_offset + segment_len)
}

fn decode_factory_header_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut MarketTradePayloadSummary,
) -> Option<usize> {
    summary.factory_header_trailer_present = true;
    if payload.len().saturating_sub(trailer_offset) < MARKET_TRADE_FACTORY_HEADER_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: factory-header trailer is too short"
        ));
        return None;
    }
    let trailer = &payload[trailer_offset..];
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_len = read_u16_le(trailer, 8) as usize;
    let reserved0 = read_u16_le(trailer, 10);
    let row_count = read_u32_le(trailer, 12);
    let country_count = read_u32_le(trailer, 16);
    let reserved1 = read_u32_le(trailer, 20);
    summary.factory_header_rows = row_count;
    if version != MARKET_TRADE_FACTORY_HEADER_SCHEMA_VERSION
        || header_len != MARKET_TRADE_FACTORY_HEADER_HEADER_LEN
        || row_len != MARKET_TRADE_FACTORY_HEADER_ROW_LEN
        || reserved0 != 0
        || reserved1 != 0
        || row_count != country_count
    {
        summary.parse_errors.push(format!(
            "offset {offset}: invalid factory-header shape version {version} header_len {header_len} row_len {row_len} rows {row_count} countries {country_count}"
        ));
        return None;
    }
    let expected_len = header_len.saturating_add((row_count as usize).saturating_mul(row_len));
    if trailer.len() < expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: factory-header trailer is {} byte(s), expected at least {expected_len}",
            trailer.len()
        ));
        return None;
    }
    let mut cursor = header_len;
    let mut seen = BTreeSet::new();
    for row_index in 0..row_count {
        let row = FactoryHeaderRow {
            tag_db_id: read_u32_le(trailer, cursor),
            total: read_i32_le(trailer, cursor + 4),
            used: read_i32_le(trailer, cursor + 8),
            available: read_i32_le(trailer, cursor + 12),
        };
        cursor += row_len;
        if row.tag_db_id == 0 || row.tag_db_id > MARKET_TRADE_MAX_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: factory-header row {row_index} invalid tag_db_id {}",
                row.tag_db_id
            ));
        }
        if !seen.insert(row.tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: factory-header row {row_index} duplicates tag_db_id {}",
                row.tag_db_id
            ));
        }
        summary.factory_headers.push(row);
    }
    Some(trailer_offset + expected_len)
}

/// Decode one `MTRR` v3 identity entry, fail-closed on every field.
fn decode_strategic_resource_identity(
    trailer: &[u8],
    cursor: usize,
    slot: usize,
) -> Result<String, String> {
    let token_len = read_u16_le(trailer, cursor) as usize;
    if token_len == 0 || token_len > MAX_STRATEGIC_RESOURCE_TOKEN_LEN {
        return Err(format!(
            "MTRR identity {slot} token length {token_len} is outside 1..={MAX_STRATEGIC_RESOURCE_TOKEN_LEN}"
        ));
    }
    let body_start = cursor + 2;
    let body = trailer
        .get(body_start..body_start.saturating_add(MAX_STRATEGIC_RESOURCE_TOKEN_LEN))
        .ok_or_else(|| format!("MTRR identity {slot} is truncated"))?;
    // `split_at`, not two index slices: the length guard above already makes
    // both in range, and this keeps a future edit to that guard a refusal
    // rather than a panic.
    let (token_bytes, padding) = body.split_at(token_len);
    if padding.iter().any(|byte| *byte != 0) {
        return Err(format!("MTRR identity {slot} has non-zero padding"));
    }
    let token = std::str::from_utf8(token_bytes)
        .map_err(|_| format!("MTRR identity {slot} is not UTF-8"))?;
    // A charset rule, not a name list: which resources exist is the mod's to
    // decide, and a decoder that knows the answer in advance is the defect this
    // block removes.
    if !token.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' || byte == b'.' || byte == b':'
    }) {
        return Err(format!("MTRR identity {slot} is not a resource token"));
    }
    Ok(token.to_string())
}

fn decode_resource_row_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut MarketTradePayloadSummary,
) -> Option<usize> {
    summary.resource_row_trailer_present = true;
    if payload.len().saturating_sub(trailer_offset) < MARKET_TRADE_RESOURCE_ROW_HEADER_LEN {
        summary
            .parse_errors
            .push(format!("offset {offset}: MTRR trailer is too short"));
        return None;
    }
    let trailer = &payload[trailer_offset..];
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_len = read_u16_le(trailer, 8) as usize;
    let reserved0 = read_u16_le(trailer, 10);
    let row_count = read_u32_le(trailer, 12);
    let country_count = read_u32_le(trailer, 16);
    let resource_count = read_u32_le(trailer, 20);
    let display_scale = read_u32_le(trailer, 24);
    let reserved1 = read_u32_le(trailer, 28);
    summary.resource_rows = row_count;
    summary.resource_row_country_count = country_count;
    summary.resource_row_resource_count = resource_count;
    // The count is a wire field, not a constant: `common/resources` is
    // moddable, so the width comes off the stream. v1 keeps its exactly-seven
    // rule; v2 admits the discovered count under the shared ceiling. Any other
    // version -- including a future one -- falls through to `false` and is
    // refused with a reason, never reinterpreted under a grammar it did not
    // declare.
    let version_admits_count = match version {
        MARKET_TRADE_RESOURCE_ROW_LEGACY_SCHEMA_VERSION => {
            resource_count == MARKET_TRADE_FIXED7_RESOURCE_COUNT
        }
        MARKET_TRADE_RESOURCE_ROW_COUNT_ONLY_SCHEMA_VERSION
        | MARKET_TRADE_RESOURCE_ROW_SCHEMA_VERSION
        | MARKET_TRADE_RESOURCE_ROW_NEEDED_UNAVAILABLE_SCHEMA_VERSION => {
            (1..=MAX_STRATEGIC_RESOURCE_COUNT).contains(&resource_count)
        }
        _ => false,
    };
    // Only v4 defines an unobserved needed-trio; under every earlier version the
    // bit and the type are unknown vocabulary and the row is refused.
    let admits_needed_unavailable =
        version == MARKET_TRADE_RESOURCE_ROW_NEEDED_UNAVAILABLE_SCHEMA_VERSION;
    let known_flags = if admits_needed_unavailable {
        MARKET_TRADE_RESOURCE_FLAGS_KNOWN | MARKET_TRADE_RESOURCE_FLAG_NEEDED_UNAVAILABLE
    } else {
        MARKET_TRADE_RESOURCE_FLAGS_KNOWN
    };
    // `+28` is reserved zero in v1/v2 and the identity stride in v3 and v4. The
    // VERSION selects the reading; the value never does.
    let carries_identity = matches!(
        version,
        MARKET_TRADE_RESOURCE_ROW_SCHEMA_VERSION
            | MARKET_TRADE_RESOURCE_ROW_NEEDED_UNAVAILABLE_SCHEMA_VERSION
    );
    let identity_field_valid = if carries_identity {
        reserved1 == STRATEGIC_RESOURCE_IDENTITY_STRIDE as u32
    } else {
        reserved1 == 0
    };
    if !version_admits_count
        || header_len != MARKET_TRADE_RESOURCE_ROW_HEADER_LEN
        || row_len != MARKET_TRADE_RESOURCE_ROW_LEN
        || reserved0 != 0
        || !identity_field_valid
        || display_scale != 1
        || row_count != country_count.saturating_mul(resource_count)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: invalid MTRR shape version {version} header_len {header_len} row_len {row_len} rows {row_count} countries {country_count} resources {resource_count} scale {display_scale}"
        ));
        return None;
    }
    let identity_len = if carries_identity {
        (resource_count as usize).saturating_mul(STRATEGIC_RESOURCE_IDENTITY_STRIDE)
    } else {
        0
    };
    let expected_len = header_len
        .saturating_add(identity_len)
        .saturating_add((row_count as usize).saturating_mul(row_len));
    if trailer.len() < expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: MTRR trailer is {} byte(s), expected at least {expected_len}",
            trailer.len()
        ));
        return None;
    }
    let mut cursor = header_len;
    if carries_identity {
        let mut distinct = BTreeSet::new();
        for slot in 0..resource_count as usize {
            match decode_strategic_resource_identity(trailer, cursor, slot) {
                Ok(token) => {
                    if !distinct.insert(token.clone()) {
                        summary.parse_errors.push(format!(
                            "offset {offset}: MTRR identity {slot} repeats token {token}"
                        ));
                    }
                    summary.resource_tokens.push(token);
                }
                Err(reason) => summary
                    .parse_errors
                    .push(format!("offset {offset}: {reason}")),
            }
            cursor += STRATEGIC_RESOURCE_IDENTITY_STRIDE;
        }
    }
    let mut seen = BTreeSet::new();
    let mut per_country = BTreeMap::<u32, BTreeSet<u32>>::new();
    // WHICH SLOT IS OIL IS READ OFF THE ROW, NEVER OFF ITS POSITION.
    //
    // The producer identifies oil from the engine's own cached oil descriptor
    // (`CStrategicResourceDatabase+0x60`) and stamps the answer on the row it
    // belongs to, as the oil-only needed model: `NEEDED_CURRENT_MAX` with
    // `FLAGS_KNOWN`, or -- from v4 -- `NEEDED_UNAVAILABLE` with
    // `FLAG_NEEDED_UNAVAILABLE`. No other row may carry either shape.
    //
    // This decoder used to ask `resource_index == 0` instead, which assumes oil
    // is the FIRST resource `common/resources` defines. That is true of vanilla
    // and false of any mod that reorders or prepends a resource, and the failure
    // was not a mislabelling but a total refusal: the real oil row would have
    // been judged under the non-oil rule and the slot-0 row under the oil rule,
    // so both would have raised a parse error.
    let mut oil_slots = BTreeSet::new();
    let mut oil_row_count = 0u32;
    for row_index in 0..row_count {
        let row = ResourceViewRow {
            tag_db_id: read_u32_le(trailer, cursor),
            resource_index: read_u32_le(trailer, cursor + 4),
            visible_order: read_u32_le(trailer, cursor + 8),
            needed_type: read_u32_le(trailer, cursor + 12),
            balance: read_i32_le(trailer, cursor + 16),
            produced: read_i32_le(trailer, cursor + 20),
            imported: read_i32_le(trailer, cursor + 24),
            exported: read_i32_le(trailer, cursor + 28),
            special_project: read_i32_le(trailer, cursor + 32),
            production_line: read_i32_le(trailer, cursor + 36),
            needed: read_i32_le(trailer, cursor + 40),
            needed_current: read_i32_le(trailer, cursor + 44),
            needed_max: read_i32_le(trailer, cursor + 48),
            flags: read_u32_le(trailer, cursor + 52),
            descriptor_db_index: read_u32_le(trailer, cursor + 56),
        };
        let reserved = read_u32_le(trailer, cursor + 60);
        cursor += row_len;
        if row.tag_db_id == 0 || row.tag_db_id > MARKET_TRADE_MAX_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: MTRR row {row_index} invalid tag_db_id {}",
                row.tag_db_id
            ));
        }
        if row.resource_index >= resource_count
            || row.visible_order >= resource_count
            || row.descriptor_db_index == 0
            || row.flags & !known_flags != 0
            || reserved != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: MTRR row {row_index} has invalid shape"
            ));
        }
        // Oil-shaped means "carries a vocabulary only the oil row may use".
        // Both the type and the flag word are tested, so a row that sets one
        // without the other is still routed to the oil rule and refused by it
        // rather than slipping through the non-oil rule.
        let oil = matches!(
            row.needed_type,
            MARKET_TRADE_RESOURCE_NEEDED_CURRENT_MAX | MARKET_TRADE_RESOURCE_NEEDED_UNAVAILABLE
        ) || row.flags
            & (MARKET_TRADE_RESOURCE_FLAGS_KNOWN | MARKET_TRADE_RESOURCE_FLAG_NEEDED_UNAVAILABLE)
            != 0;
        if oil {
            oil_slots.insert(row.resource_index);
            oil_row_count += 1;
        }
        // The oil slot's needed-trio is either derived or explicitly unobserved.
        // The unobserved shape is checked EXACTLY -- type, flags and all three
        // zeroed fields -- so a zeroed trio can never pass as a derived one.
        let needed_unavailable = oil
            && admits_needed_unavailable
            && (row.needed_type == MARKET_TRADE_RESOURCE_NEEDED_UNAVAILABLE
                || row.flags & MARKET_TRADE_RESOURCE_FLAG_NEEDED_UNAVAILABLE != 0);
        if needed_unavailable {
            if row.needed_type != MARKET_TRADE_RESOURCE_NEEDED_UNAVAILABLE
                || row.flags != MARKET_TRADE_RESOURCE_FLAG_NEEDED_UNAVAILABLE
                || row.needed != 0
                || row.needed_current != 0
                || row.needed_max != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: MTRR row {row_index} has inconsistent unobserved-needed semantics"
                ));
            }
        } else if (oil
            && (row.needed_type != MARKET_TRADE_RESOURCE_NEEDED_CURRENT_MAX
                || row.flags != MARKET_TRADE_RESOURCE_FLAGS_KNOWN
                || row.needed != row.needed_current))
            || (!oil
                && (row.flags != 0
                    || row.needed_current != 0
                    || row.needed_max != 0
                    || ![
                        MARKET_TRADE_RESOURCE_NEEDED_SURPLUS,
                        MARKET_TRADE_RESOURCE_NEEDED_DEFICIT,
                    ]
                    .contains(&row.needed_type)
                    || row.needed != row.balance
                    || (row.balance < 0)
                        != (row.needed_type == MARKET_TRADE_RESOURCE_NEEDED_DEFICIT)))
        {
            summary.parse_errors.push(format!(
                "offset {offset}: MTRR row {row_index} has inconsistent needed semantics"
            ));
        }
        if !seen.insert((row.tag_db_id, row.resource_index)) {
            summary.parse_errors.push(format!(
                "offset {offset}: MTRR row {row_index} duplicates country/resource key"
            ));
        }
        per_country
            .entry(row.tag_db_id)
            .or_default()
            .insert(row.visible_order);
        summary.resource_view_rows.push(row);
    }
    // Oil is ONE resource, and it is the same one for every country in a record.
    // Reading the slot off the rows is only sound if the rows agree on it, so
    // that agreement is checked rather than assumed: exactly one slot may wear
    // the oil model, and every country must present it exactly once. Combined
    // with the country/resource uniqueness above and the dense visible-order
    // check below, this pins the oil slot to a single discovered value --
    // whatever position the mod's `common/resources` put it in.
    if country_count > 0 {
        if oil_slots.len() != 1 {
            summary.parse_errors.push(format!(
                "offset {offset}: MTRR states {} oil slots, expected exactly 1",
                oil_slots.len()
            ));
        }
        if oil_row_count != country_count {
            summary.parse_errors.push(format!(
                "offset {offset}: MTRR states {oil_row_count} oil rows for {country_count} countries"
            ));
        }
    } else if oil_row_count != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: MTRR states an oil row with no countries"
        ));
    }
    if per_country.len() != country_count as usize {
        summary.parse_errors.push(format!(
            "offset {offset}: MTRR decoded country count {} != header {country_count}",
            per_country.len()
        ));
    }
    for (tag_db_id, orders) in per_country {
        if orders.len() != resource_count as usize || !orders.iter().copied().eq(0..resource_count)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: MTRR country {tag_db_id} visible order is not complete"
            ));
        }
    }
    Some(trailer_offset + expected_len)
}

fn decode_classic_deal_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut MarketTradePayloadSummary,
) -> Option<usize> {
    summary.classic_deal_trailer_present = true;
    if payload.len().saturating_sub(trailer_offset) < MARKET_TRADE_CLASSIC_DEAL_HEADER_LEN {
        summary
            .parse_errors
            .push(format!("offset {offset}: MTRD trailer is too short"));
        return None;
    }
    let trailer = &payload[trailer_offset..];
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_len = read_u16_le(trailer, 8) as usize;
    let reserved0 = read_u16_le(trailer, 10);
    let row_count = read_u32_le(trailer, 12);
    let country_count = read_u32_le(trailer, 16);
    let scale_raw = read_u32_le(trailer, 20);
    let witness_count = read_u32_le(trailer, 24);
    // v3 and v4's incomplete-owner count; v1/v2's reserved word, which must be
    // zero. A v3 body labelled v2 therefore fails the shape check rather than
    // reading the completeness block as deal rows.
    let per_owner = version == MARKET_TRADE_CLASSIC_DEAL_PER_OWNER_SCHEMA_VERSION
        || version == MARKET_TRADE_CLASSIC_DEAL_WIDE_IDENTITY_SCHEMA_VERSION;
    let incomplete_count = read_u32_le(trailer, 28);
    summary.classic_deal_rows = row_count;
    summary.classic_deal_country_count = country_count;
    summary.classic_deal_incomplete_owner_count = if per_owner { incomplete_count } else { 0 };
    if ![
        MARKET_TRADE_CLASSIC_DEAL_LEGACY_SCHEMA_VERSION,
        MARKET_TRADE_CLASSIC_DEAL_SCHEMA_VERSION,
        MARKET_TRADE_CLASSIC_DEAL_PER_OWNER_SCHEMA_VERSION,
        MARKET_TRADE_CLASSIC_DEAL_WIDE_IDENTITY_SCHEMA_VERSION,
    ]
    .contains(&version)
        || header_len != MARKET_TRADE_CLASSIC_DEAL_HEADER_LEN
        || row_len != MARKET_TRADE_CLASSIC_DEAL_ROW_LEN
        || reserved0 != 0
        || (!per_owner && incomplete_count != 0)
        || (per_owner && incomplete_count > witness_count)
        || scale_raw != MARKET_TRADE_SCALE_RAW
        || witness_count != country_count
    {
        summary.parse_errors.push(format!(
            "offset {offset}: invalid MTRD shape version {version} header_len {header_len} row_len {row_len} countries {country_count} witnesses {witness_count} incomplete {incomplete_count} scale {scale_raw}"
        ));
        return None;
    }
    let expected_len = header_len
        .saturating_add((witness_count as usize).saturating_mul(4))
        .saturating_add((incomplete_count as usize).saturating_mul(4))
        .saturating_add((row_count as usize).saturating_mul(row_len));
    if trailer.len() < expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: MTRD trailer is {} byte(s), expected at least {expected_len}",
            trailer.len()
        ));
        return None;
    }
    let mut cursor = header_len;
    let mut witnesses = BTreeSet::new();
    for witness_index in 0..witness_count {
        let tag_db_id = read_u32_le(trailer, cursor);
        cursor += 4;
        if tag_db_id == 0 || tag_db_id > MARKET_TRADE_MAX_TAG_DB_ID || !witnesses.insert(tag_db_id)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: MTRD witness {witness_index} is invalid or duplicated"
            ));
        }
        summary.classic_deal_country_witnesses.push(tag_db_id);
    }
    let mut incomplete_seen = BTreeSet::new();
    let mut previous_incomplete: Option<u32> = None;
    for incomplete_index in 0..incomplete_count {
        let tag_db_id = read_u32_le(trailer, cursor);
        cursor += 4;
        if !witnesses.contains(&tag_db_id)
            || previous_incomplete.is_some_and(|previous| previous >= tag_db_id)
            || !incomplete_seen.insert(tag_db_id)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: MTRD incomplete owner {incomplete_index} is not an ascending witness"
            ));
        }
        previous_incomplete = Some(tag_db_id);
        summary.classic_deal_incomplete_owners.push(tag_db_id);
    }
    let mut seen = BTreeSet::new();
    let mut pair_halves = BTreeMap::<ClassicDealIdentity, Vec<ClassicDealRow>>::new();
    let mut per_owner_order = BTreeMap::<u32, BTreeSet<u32>>::new();
    let wide_identity = version == MARKET_TRADE_CLASSIC_DEAL_WIDE_IDENTITY_SCHEMA_VERSION;
    for row_index in 0..row_count {
        // The deal identity, read at whatever width this version spells it. The
        // resource is taken from the row's own `resource_index` on both paths;
        // v1-v3's packed resource BYTE is a repeat of it and is checked as such
        // in `legacy_identity_matches` below.
        let (identity_exporter, identity_importer, identity_ordinal) = if wide_identity {
            (
                read_u32_le(trailer, cursor + 32),
                read_u32_le(trailer, cursor + 36),
                read_u32_le(trailer, cursor + 92),
            )
        } else {
            let packed = read_u64_le(trailer, cursor + 32);
            (
                ((packed >> 32) & 0xFFFF) as u32,
                ((packed >> 16) & 0xFFFF) as u32,
                (packed & 0xFF) as u32,
            )
        };
        let legacy_identity_matches = wide_identity || {
            let packed = read_u64_le(trailer, cursor + 32);
            packed >> 48 == 0 && ((packed >> 8) & 0xFF) as u32 == read_u32_le(trailer, cursor + 8)
        };
        let row = ClassicDealRow {
            owner_tag_db_id: read_u32_le(trailer, cursor),
            counterpart_tag_db_id: read_u32_le(trailer, cursor + 4),
            resource_index: read_u32_le(trailer, cursor + 8),
            visible_order: read_u32_le(trailer, cursor + 12),
            direction: read_u32_le(trailer, cursor + 16),
            route_kind: read_u32_le(trailer, cursor + 20),
            flags: read_u32_le(trailer, cursor + 24),
            // v2 added the exact route-status bytes and v3 inherits them
            // unchanged; only legacy v1 leaves them unavailable. Testing for
            // "not legacy" is what keeps a version bump from silently demoting
            // a measured field to absent.
            route_status_flags: if version != MARKET_TRADE_CLASSIC_DEAL_LEGACY_SCHEMA_VERSION {
                Some(read_u32_le(trailer, cursor + 28))
            } else {
                None
            },
            identity: ClassicDealIdentity {
                exporter_tag_db_id: identity_exporter,
                importer_tag_db_id: identity_importer,
                resource_index: read_u32_le(trailer, cursor + 8),
                ordinal: identity_ordinal,
            },
            requested_raw: read_i64_le(trailer, cursor + 40),
            delivered_raw: read_i64_le(trailer, cursor + 48),
            available_export_raw: read_i64_le(trailer, cursor + 56),
            factory_cost_raw: read_i64_le(trailer, cursor + 64),
            efficiency_raw: read_i64_le(trailer, cursor + 72),
            civilian_factories: read_i32_le(trailer, cursor + 80),
            convoys_assigned: read_i32_le(trailer, cursor + 84),
            convoys_required: read_i32_le(trailer, cursor + 88),
        };
        let status_or_reserved = read_u32_le(trailer, cursor + 28);
        // v1-v3 hold `+0x5C` at zero; v4 spends it on the one-based ordinal,
        // which `identity_shape_ok` below requires to be non-zero and within the
        // export bucket's own transport budget. Whichever version this is,
        // exactly one of the two rules applies and neither admits an arbitrary
        // value.
        let reserved_row1 = if wide_identity {
            0
        } else {
            read_i32_le(trailer, cursor + 92)
        };
        // The identity's intra-row agreement, which is this row's tear
        // detector: `exporter`/`importer` must be the row's own owner and
        // counterpart under `direction`. The two country ids are not
        // range-checked separately — that equality plus the witness rule above
        // already bounds them.
        let identity_shape_ok = legacy_identity_matches
            && row.identity.ordinal != 0
            && row.identity.ordinal
                <= compact_contract::build_profile::MAX_MARKET_TRADE_CLASSIC_DEAL_ORDINAL
            && match row.direction {
                MARKET_TRADE_CLASSIC_DEAL_DIRECTION_EXPORT => {
                    row.owner_tag_db_id == row.identity.exporter_tag_db_id
                        && row.counterpart_tag_db_id == row.identity.importer_tag_db_id
                }
                MARKET_TRADE_CLASSIC_DEAL_DIRECTION_IMPORT => {
                    row.owner_tag_db_id == row.identity.importer_tag_db_id
                        && row.counterpart_tag_db_id == row.identity.exporter_tag_db_id
                }
                _ => false,
            };
        cursor += row_len;
        if !identity_shape_ok
            || !witnesses.contains(&row.owner_tag_db_id)
            || !witnesses.contains(&row.counterpart_tag_db_id)
            || row.owner_tag_db_id == row.counterpart_tag_db_id
            || row.resource_index >= MAX_STRATEGIC_RESOURCE_COUNT
            || ![
                MARKET_TRADE_CLASSIC_DEAL_DIRECTION_IMPORT,
                MARKET_TRADE_CLASSIC_DEAL_DIRECTION_EXPORT,
            ]
            .contains(&row.direction)
            || ![
                MARKET_TRADE_CLASSIC_DEAL_ROUTE_LAND,
                MARKET_TRADE_CLASSIC_DEAL_ROUTE_SEA,
                MARKET_TRADE_CLASSIC_DEAL_ROUTE_LOCKED,
            ]
            .contains(&row.route_kind)
            || row.flags != MARKET_TRADE_CLASSIC_DEAL_FLAG_EXACT
            || (version == MARKET_TRADE_CLASSIC_DEAL_LEGACY_SCHEMA_VERSION
                && status_or_reserved != 0)
            || (version != MARKET_TRADE_CLASSIC_DEAL_LEGACY_SCHEMA_VERSION
                && status_or_reserved & !MARKET_TRADE_CLASSIC_DEAL_STATUS_KNOWN != 0)
            || row.efficiency_raw < 0
            || row.efficiency_raw > MARKET_TRADE_SCALE_RAW as i64
            || reserved_row1 != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: MTRD row {row_index} has invalid shape"
            ));
        }
        if !seen.insert((row.owner_tag_db_id, row.visible_order)) {
            summary.parse_errors.push(format!(
                "offset {offset}: MTRD row {row_index} duplicates owner/visible_order"
            ));
        }
        per_owner_order
            .entry(row.owner_tag_db_id)
            .or_default()
            .insert(row.visible_order);
        pair_halves.entry(row.identity).or_default().push(row);
        summary.classic_deals.push(row);
    }
    for (identity, halves) in pair_halves {
        let deal_id = format!(
            "{}->{} resource {} ordinal {}",
            identity.exporter_tag_db_id,
            identity.importer_tag_db_id,
            identity.resource_index,
            identity.ordinal
        );
        if halves.len() != 2 {
            summary.parse_errors.push(format!(
                "offset {offset}: MTRD deal_id {deal_id} has {} half/halves, expected 2",
                halves.len()
            ));
            continue;
        }
        let a = halves[0];
        let b = halves[1];
        if a.owner_tag_db_id != b.counterpart_tag_db_id
            || b.owner_tag_db_id != a.counterpart_tag_db_id
            || a.direction == b.direction
            || a.resource_index != b.resource_index
            || a.route_kind != b.route_kind
            || a.route_status_flags != b.route_status_flags
            || a.requested_raw != b.requested_raw
            || a.delivered_raw != b.delivered_raw
            || a.available_export_raw != b.available_export_raw
            || a.factory_cost_raw != b.factory_cost_raw
            || a.efficiency_raw != b.efficiency_raw
            || a.civilian_factories != b.civilian_factories
            || a.convoys_assigned != b.convoys_assigned
            || a.convoys_required != b.convoys_required
        {
            summary.parse_errors.push(format!(
                "offset {offset}: MTRD deal_id {deal_id} reciprocal halves disagree"
            ));
        }
    }
    for (owner, orders) in per_owner_order {
        if !orders.iter().copied().eq(0..orders.len() as u32) {
            summary.parse_errors.push(format!(
                "offset {offset}: MTRD owner {owner} visible order is not contiguous"
            ));
        }
    }
    Some(trailer_offset + expected_len)
}

fn decode_residual_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut MarketTradePayloadSummary,
) -> Option<usize> {
    summary.residual_trailer_present = true;
    if payload.len().saturating_sub(trailer_offset) < MARKET_TRADE_RESIDUAL_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: residual trailer is {} byte(s), expected at least {MARKET_TRADE_RESIDUAL_HEADER_LEN}",
            payload.len().saturating_sub(trailer_offset)
        ));
        return None;
    }
    let trailer = &payload[trailer_offset..];
    if &trailer[0..4] != MARKET_TRADE_RESIDUAL_TRAILER_MAGIC {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade residual trailer has invalid magic"
        ));
        return None;
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
        return None;
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: residual trailer reserved header field is nonzero"
        ));
    }
    let expected_len = MARKET_TRADE_RESIDUAL_HEADER_LEN
        .saturating_add((row_count as usize).saturating_mul(MARKET_TRADE_RESIDUAL_ROW_LEN));
    if trailer.len() < expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: residual trailer is {} byte(s), expected at least {expected_len}",
            trailer.len()
        ));
        return None;
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
    Some(trailer_offset + expected_len)
}

fn decode_detail_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut MarketTradePayloadSummary,
) -> Option<usize> {
    summary.detail_trailer_present = true;
    if payload.len().saturating_sub(trailer_offset) < MARKET_TRADE_DETAIL_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: detail trailer is {} byte(s), expected at least {MARKET_TRADE_DETAIL_HEADER_LEN}",
            payload.len().saturating_sub(trailer_offset)
        ));
        return None;
    }
    let trailer = &payload[trailer_offset..];
    if &trailer[0..4] != MARKET_TRADE_DETAIL_TRAILER_MAGIC {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade detail trailer has invalid magic"
        ));
        return None;
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
        return None;
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: detail trailer reserved header field is nonzero"
        ));
    }
    let expected_len = MARKET_TRADE_DETAIL_HEADER_LEN
        .saturating_add((row_count as usize).saturating_mul(MARKET_TRADE_DETAIL_ROW_LEN));
    if trailer.len() < expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: detail trailer is {} byte(s), expected at least {expected_len}",
            trailer.len()
        ));
        return None;
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
    Some(trailer_offset + expected_len)
}

fn decode_lend_lease_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut MarketTradePayloadSummary,
) -> Option<usize> {
    summary.lend_lease_trailer_present = true;
    if payload.len().saturating_sub(trailer_offset) < MARKET_TRADE_LEND_LEASE_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: lend-lease trailer is {} byte(s), expected at least {MARKET_TRADE_LEND_LEASE_HEADER_LEN}",
            payload.len().saturating_sub(trailer_offset)
        ));
        return None;
    }
    let trailer = &payload[trailer_offset..];
    if &trailer[0..4] != MARKET_TRADE_LEND_LEASE_TRAILER_MAGIC {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade lend-lease trailer has invalid magic"
        ));
        return None;
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
        return None;
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: lend-lease trailer reserved header field is nonzero"
        ));
    }
    let expected_len = MARKET_TRADE_LEND_LEASE_HEADER_LEN
        .saturating_add((row_count as usize).saturating_mul(MARKET_TRADE_LEND_LEASE_ROW_LEN));
    if trailer.len() < expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: lend-lease trailer is {} byte(s), expected at least {expected_len}",
            trailer.len()
        ));
        return None;
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
    Some(trailer_offset + expected_len)
}

fn decode_resource_flow_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut MarketTradePayloadSummary,
) -> Option<usize> {
    summary.resource_flow_trailer_present = true;
    if payload.len().saturating_sub(trailer_offset) < MARKET_TRADE_RESOURCE_FLOW_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: resource-flow trailer is {} byte(s), expected at least {MARKET_TRADE_RESOURCE_FLOW_HEADER_LEN}",
            payload.len().saturating_sub(trailer_offset)
        ));
        return None;
    }
    let trailer = &payload[trailer_offset..];
    if &trailer[0..4] != MARKET_TRADE_RESOURCE_FLOW_TRAILER_MAGIC {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade resource-flow trailer has invalid magic"
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_len = read_u16_le(trailer, 8) as usize;
    let reserved0 = read_u16_le(trailer, 10);
    let row_count = read_u32_le(trailer, 12);
    let country_count_or_exported = read_u32_le(trailer, 16);
    let scale_raw = read_u32_le(trailer, 20);
    // `+24` is a seven-bit resource MASK in v1/v2 and the resource COUNT in v3.
    let mask_or_count = read_u32_le(trailer, 24);
    let kind_mask = read_u32_le(trailer, 28);

    summary.resource_flow_rows = row_count;
    summary.resource_flow_country_rows =
        if version == MARKET_TRADE_RESOURCE_FLOW_LEGACY_SCHEMA_VERSION {
            0
        } else {
            country_count_or_exported
        };
    summary.resource_flow_scale_raw = scale_raw;
    summary.resource_flow_resource_mask = mask_or_count;
    summary.resource_flow_kind_mask = kind_mask;

    // The version decides whether `+24` reads as a mask or as a count. A stream
    // is never read under a grammar it did not declare, and an unknown version
    // is refused with a reason rather than reinterpreted.
    let resource_count = match version {
        MARKET_TRADE_RESOURCE_FLOW_LEGACY_SCHEMA_VERSION
        | MARKET_TRADE_RESOURCE_FLOW_FIXED7_SCHEMA_VERSION => {
            if mask_or_count != MARKET_TRADE_RESOURCE_FLOW_RESOURCE_MASK {
                summary.parse_errors.push(format!(
                    "offset {offset}: resource_flow_resource_mask {mask_or_count:#X} != {MARKET_TRADE_RESOURCE_FLOW_RESOURCE_MASK:#X}"
                ));
            }
            MARKET_TRADE_FIXED7_RESOURCE_COUNT
        }
        MARKET_TRADE_RESOURCE_FLOW_SCHEMA_VERSION => {
            if !(1..=MAX_STRATEGIC_RESOURCE_COUNT).contains(&mask_or_count) {
                // Return: `expected_len` below multiplies by a row length
                // derived from this count, so an out-of-bound count must not be
                // trusted to size anything.
                summary.parse_errors.push(format!(
                    "offset {offset}: resource_flow_resource_count {mask_or_count} is outside 1..={MAX_STRATEGIC_RESOURCE_COUNT}"
                ));
                return None;
            }
            mask_or_count
        }
        _ => {
            summary.parse_errors.push(format!(
                "offset {offset}: unsupported resource-flow trailer version {version}"
            ));
            return None;
        }
    };
    summary.resource_flow_resource_count = resource_count;
    let expected_row_len = if version == MARKET_TRADE_RESOURCE_FLOW_SCHEMA_VERSION {
        MARKET_TRADE_RESOURCE_FLOW_ROW_HEADER_LEN
            .saturating_add((resource_count as usize).saturating_mul(8))
    } else {
        MARKET_TRADE_RESOURCE_FLOW_FIXED7_ROW_LEN
    };
    if header_len != MARKET_TRADE_RESOURCE_FLOW_HEADER_LEN || row_len != expected_row_len {
        summary.parse_errors.push(format!(
            "offset {offset}: resource-flow trailer version {version} header_len {header_len} row_len {row_len} != {expected_row_len}"
        ));
        return None;
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: resource-flow trailer reserved header field is nonzero"
        ));
    }
    if version == MARKET_TRADE_RESOURCE_FLOW_LEGACY_SCHEMA_VERSION && kind_mask != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: legacy resource-flow trailer reserved header field is nonzero"
        ));
    }
    if version != MARKET_TRADE_RESOURCE_FLOW_LEGACY_SCHEMA_VERSION
        && kind_mask != MARKET_TRADE_RESOURCE_FLOW_KIND_MASK
    {
        summary.parse_errors.push(format!(
            "offset {offset}: resource_flow_kind_mask {kind_mask:#X} != {MARKET_TRADE_RESOURCE_FLOW_KIND_MASK:#X}"
        ));
    }
    if version == MARKET_TRADE_RESOURCE_FLOW_LEGACY_SCHEMA_VERSION
        && country_count_or_exported != row_count
    {
        summary.parse_errors.push(format!(
            "offset {offset}: legacy resource_flow_exported_rows {country_count_or_exported} != row_count {row_count}"
        ));
    }
    if version != MARKET_TRADE_RESOURCE_FLOW_LEGACY_SCHEMA_VERSION
        && row_count != country_count_or_exported.saturating_mul(6)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: resource_flow_rows {row_count} != country_count {country_count_or_exported} * 6"
        ));
    }
    if scale_raw != MARKET_TRADE_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: resource_flow_scale_raw {scale_raw} != {MARKET_TRADE_SCALE_RAW}"
        ));
    }
    let expected_len = MARKET_TRADE_RESOURCE_FLOW_HEADER_LEN
        .saturating_add((row_count as usize).saturating_mul(expected_row_len));
    if trailer.len() < expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: resource-flow trailer is {} byte(s), expected at least {expected_len}",
            trailer.len()
        ));
        return None;
    }

    let mut cursor = MARKET_TRADE_RESOURCE_FLOW_HEADER_LEN;
    let mut seen_keys = BTreeSet::new();
    let mut seen_tag_masks = BTreeMap::<u32, u32>::new();
    let mut computed_exported = 0u32;
    let expected_row_mask_or_count = if version == MARKET_TRADE_RESOURCE_FLOW_SCHEMA_VERSION {
        resource_count
    } else {
        MARKET_TRADE_RESOURCE_FLOW_RESOURCE_MASK
    };
    for row_index in 0..row_count {
        let row = ResourceFlowRow {
            tag_db_id: read_u32_le(trailer, cursor),
            flow_kind: read_u32_le(trailer, cursor + 4),
            resource_mask_or_count: read_u32_le(trailer, cursor + 8),
            values_raw: (0..resource_count as usize)
                .map(|resource_index| {
                    read_i64_le(
                        trailer,
                        cursor + MARKET_TRADE_RESOURCE_FLOW_ROW_HEADER_LEN + resource_index * 8,
                    )
                })
                .collect(),
        };
        let reserved = read_u32_le(trailer, cursor + 12);
        cursor += expected_row_len;

        validate_resource_flow_row(
            offset,
            row_index as usize,
            &row,
            reserved,
            expected_row_mask_or_count,
            version == MARKET_TRADE_RESOURCE_FLOW_LEGACY_SCHEMA_VERSION,
            summary,
        );
        if row.flow_kind == MARKET_TRADE_RESOURCE_FLOW_KIND_EXPORTED {
            computed_exported = computed_exported.saturating_add(1);
        }
        if row.tag_db_id != 0 && !seen_keys.insert((row.tag_db_id, row.flow_kind)) {
            summary.parse_errors.push(format!(
                "offset {offset}: resource-flow row {row_index} duplicates tag_db_id {} flow_kind {}",
                row.tag_db_id, row.flow_kind
            ));
        }
        if (1..=6).contains(&row.flow_kind) {
            let mask = seen_tag_masks.entry(row.tag_db_id).or_default();
            *mask |= 1 << (row.flow_kind - 1);
        }
        summary.resource_flow_rows_decoded.push(row);
    }
    summary.resource_flow_exported_rows = computed_exported;
    if version == MARKET_TRADE_RESOURCE_FLOW_LEGACY_SCHEMA_VERSION
        && country_count_or_exported != computed_exported
    {
        summary.parse_errors.push(format!(
            "offset {offset}: legacy resource_flow_exported_rows header {country_count_or_exported} != decoded {computed_exported}"
        ));
    }
    if version != MARKET_TRADE_RESOURCE_FLOW_LEGACY_SCHEMA_VERSION {
        if seen_tag_masks.len() != country_count_or_exported as usize {
            summary.parse_errors.push(format!(
                "offset {offset}: decoded resource-flow country count {} != header {country_count_or_exported}",
                seen_tag_masks.len()
            ));
        }
        for (tag_db_id, observed_mask) in seen_tag_masks {
            if observed_mask != MARKET_TRADE_RESOURCE_FLOW_KIND_MASK {
                summary.parse_errors.push(format!(
                    "offset {offset}: tag_db_id {tag_db_id} resource-flow kind mask {observed_mask:#X} != {MARKET_TRADE_RESOURCE_FLOW_KIND_MASK:#X}"
                ));
            }
        }
    }
    Some(trailer_offset + expected_len)
}

fn validate_resource_flow_row(
    offset: usize,
    row_index: usize,
    row: &ResourceFlowRow,
    reserved: u32,
    expected_mask_or_count: u32,
    legacy: bool,
    summary: &mut MarketTradePayloadSummary,
) {
    if row.tag_db_id == 0 || row.tag_db_id > MARKET_TRADE_MAX_TAG_DB_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: resource-flow row {row_index} invalid tag_db_id {}",
            row.tag_db_id
        ));
    }
    if (legacy && row.flow_kind != MARKET_TRADE_RESOURCE_FLOW_KIND_EXPORTED)
        || (!legacy && !(1..=6).contains(&row.flow_kind))
    {
        summary.parse_errors.push(format!(
            "offset {offset}: resource-flow row {row_index} invalid flow_kind {}",
            row.flow_kind
        ));
    }
    if row.resource_mask_or_count != expected_mask_or_count {
        summary.parse_errors.push(format!(
            "offset {offset}: resource-flow row {row_index} states 0x{:X}, header states 0x{expected_mask_or_count:X}",
            row.resource_mask_or_count
        ));
    }
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: resource-flow row {row_index} reserved field is nonzero"
        ));
    }
    if legacy && row.values_raw.iter().all(|value| *value == 0) {
        summary.parse_errors.push(format!(
            "offset {offset}: resource-flow row {row_index} has all-zero exported resources"
        ));
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
    if let Err(error) = compact_contract::stream_validator::validate_freeze_census_stream_placement(raw) {
        summary.parse_errors.push(format!("stateful stream placement: {error}"));
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
        // Any byte after a clean closure means the physical EOF is no longer at
        // that FRAME_END, even when the trailing bytes cannot form a header.
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
        let record_semantics_ok = match validate_compact_record_semantics(
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
                if let Ok(census) = compact_contract::freeze_census_record::FreezeCensusRecordV1::decode(payload) {
                    final_record_is_clean_frame_end = compact_contract::freeze_census_record::release_epilogue_closes_stream_policy(previous_record_closed_world, census.phase(), release_epilogue_seen);
                    release_epilogue_seen |= census.phase() == compact_contract::freeze_census_record::FreezeCensusPhase::Release;
                }
            }
            compact_contract::KnownRecordType::StreamHeader => {}
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
    // Domain 43 has no lane-local aliases and no registered expected-partial
    // tolerance. Applying the shared normalizer here covers owned DATA/PERF
    // status and the FRAME_END aggregate status accumulated above without
    // inventing a lane-local exemption.
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

    /// `resource_count` is a parameter because the wire is count-prefixed: a
    /// test that could only build seven-resource payloads could not exercise
    /// the case the fixed width made impossible.
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

    /// Build an `MTRR` trailer stating an explicit resource set. `tokens` goes
    /// into the v3 identity block verbatim, including bodies the decoder must
    /// refuse. v1/v2 ignore it -- they have nowhere to put it.
    fn append_test_mtrr_with_identity(
        payload: &mut Vec<u8>,
        countries: &[u32],
        resource_count: u32,
        tokens: &[String],
        version: u16,
    ) {
        append_test_mtrr_with_oil_slot(payload, countries, resource_count, tokens, version, 0);
    }

    /// The same trailer with the OIL SLOT as a parameter. Vanilla puts oil
    /// first, so every other fixture here passes 0; a mod that reorders or
    /// prepends a resource in `common/resources` does not, and that is the case
    /// the decoder must resolve from the row's own stamped model rather than
    /// from its position.
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

    /// The same two-half deal under an explicit version word and an explicit
    /// identity pair.
    ///
    /// Both are parameters rather than derived, so a fixture can drive the v4
    /// identity words INDEPENDENTLY of the row's own owner/counterpart — which
    /// is the only way to exercise the intra-row agreement rule that is this
    /// row's tear detector.
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
        // v4 spells the identity as two full-width words at `+0x20`/`+0x24`
        // plus a one-based ordinal in v1-v3's reserved word at `+0x5C`; v1-v3
        // pack all four into one `u64` at `+0x20`, sixteen bits per country id.
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

    /// One country's complete six-kind flow matrix, as wide as `exported`.
    /// The width is a parameter, not seven: that is the whole point of the
    /// count-prefixed vector.
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

    /// The lane decoder reads BOTH `MTRD` identity spellings, and the v4 one is
    /// what the collector actually emits.
    ///
    /// v1-v3 pack the deal identity into a `u64` with sixteen bits per country
    /// id; v4 carries two full `u32`s plus a one-based ordinal in the word
    /// v1-v3 hold at zero. This decoder is a SECOND consumer of that trailer,
    /// independent of the fold's, and before this fixture its v4 branch was
    /// never executed by any test — the whole suite drove version 2.
    ///
    /// kills: deleting the v4 arm of the identity read (the v4 payload's rows
    /// then unpack the two `u32`s as one packed `u64` and fail the intra-row
    /// agreement, so the deal count drops); deleting the `identity_shape_ok`
    /// agreement check itself (the mismatched-identity payload below then
    /// parses clean); and reading the ordinal from the wrong word on v4 (a zero
    /// ordinal is refused, and `+0x5C` is the only word carrying it).
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

        // The intra-row agreement, on v4, where the identity words are separate
        // from the row's own owner/counterpart and so CAN disagree. 99 is a
        // witness, so witness membership does not refuse this on its own.
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
        // The lane decoder reads the set's identity, not just its width.
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
        // Rows start after the v3 identity block, not at `header_len`.
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
        // Second country item_first (country row 1 +4) must equal 1.
        let second_first =
            MARKET_TRADE_STOCKPILE_HEADER_LEN + MARKET_TRADE_STOCKPILE_COUNTRY_ROW_LEN + 4;
        trailer[second_first..second_first + 4].copy_from_slice(&0u32.to_le_bytes());
        noncontiguous.extend_from_slice(&trailer);
        let decoded = decode_payload(0, &noncontiguous);
        assert!(decoded.parse_errors.iter().any(|error| {
            error.contains("MTS1 country row 1 has invalid tag/range/completeness")
        }));
    }

    // ---------------------------------------------------------------------
    // Count-prefixed resource vector: MTRF v3 / MTRR v2.
    //
    // Each of these is the visible half of a mutation. `runs/resource_wire/`
    // records which production edit turns which of them red.
    // ---------------------------------------------------------------------

    /// (a) The case the fixed seven-wide vector made unrepresentable: a mod's
    /// eighth strategic resource, encoded by the producer and read back by the
    /// decoder with every slot intact.
    ///
    /// Reverting the decoder's width handling to the fixed 72-byte row while
    /// leaving the producer widened -- the producer-wider-than-consumer drift
    /// class -- turns this red, because the eighth slot lands past the row the
    /// consumer would read.
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

    /// (b) The vanilla width still round-trips, byte for byte, through the same
    /// count-prefixed path.
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

    /// (c) A stream written before the widening still reads. v2 pins the vector
    /// to seven and puts the seven-bit mask at `+24`; nothing about that
    /// changed, and the new decoder applies exactly the v2 rules to it.
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

    /// (c) The same, for `MTRR` v1.
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

    /// (c2) A REORDERED resource table decodes, with oil resolved at its real
    /// slot rather than at slot 0.
    ///
    /// `common/resources` is an ordinary game file and a mod may prepend to it,
    /// which moves oil off the first slot. The producer already identifies oil
    /// from the database's own cached oil descriptor and stamps the oil
    /// needed-model on that row; this asserts the decoder reads the stamp.
    ///
    /// Reverting `let oil = ...` to `row.resource_index == 0` makes this red:
    /// the real oil row is then judged under the non-oil rule and the slot-0 row
    /// under the oil rule, so BOTH raise parse errors and the record is refused
    /// outright -- a modded game would publish nothing for D43.
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
        // Exactly the slot-3 rows wear the oil model, one per country.
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
        // And slot 0, which a positional decoder would have called oil, is an
        // ordinary non-oil row with no needed pair.
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

    /// (c3) Two slots wearing the oil model is refused.
    ///
    /// Reading the oil slot off the rows is only sound because the rows are made
    /// to agree on it, so the disagreement is checked rather than resolved to
    /// whichever slot the decoder saw first.
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

    /// (d) The decisive one for version safety. A stream one version PAST what
    /// this decoder knows is refused with its version named -- it is never
    /// reinterpreted under the grammar the decoder does know.
    ///
    /// This is the code path an older decoder takes when it meets a v3 stream:
    /// the accepted-version set is the discriminator, so stamping a version
    /// outside the set exercises exactly that branch. It matters because `+24`
    /// is a mask in v1/v2 and a count in v3 -- reading a v3 header under the v2
    /// rules would compare a small count against `0x7F` and mis-size every row.
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

    /// (d) The same for `MTRR`.
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

    /// (e) A count past the shared admission ceiling is refused before anything
    /// is sized from it. Without the bound, `row_count * resource_count * 8` is
    /// two stream-supplied `u32`s driving an allocation.
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

    /// (e) The same for `MTRR` v2, and for a zero count -- an empty resource set
    /// is not a resource set.
    ///
    /// The over-bound trailer is otherwise COMPLETE: it emits all 65 rows per
    /// country, so nothing but the ceiling stands between it and admission. A
    /// fixture that declared 65 and emitted no rows would stay refusable as
    /// truncated even with the ceiling gone, and would prove nothing.
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

    /// The encoder applies the decoders' predicate. An encoder that can emit a
    /// count no decoder admits is a second opinion about what is legal -- and
    /// its `Vec::with_capacity` is sized from `8 * resource_count`, so an
    /// unbounded count sizes an allocation before anything refuses it.
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

    /// A row narrower or wider than the stated count cannot be encoded either:
    /// the header would describe rows the payload does not contain.
    #[test]
    fn d43_resource_flow_encoder_refuses_rows_that_disagree_with_the_count() {
        let mut rows = six_kind_flow_rows(67, &[0; 7]);
        rows[3].values_raw.push(0);
        assert!(encode_resource_flow_trailer(&rows, 7).is_none());
        assert!(encode_resource_flow_trailer(&rows, 8).is_none());
    }

    /// The encoder applies EVERY per-row predicate the decoder applies. A row
    /// echoing a different count, or carrying a flow kind outside the six-kind
    /// matrix, is a stream every decoder refuses -- so the encoder must not be
    /// able to write it.
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

    /// A row that states a different width from its header is refused. Without
    /// this, header and rows could drift and the decoder would read whichever
    /// the header happened to say.
    ///
    /// The mismatch is patched into the BYTES, not built with the encoder: the
    /// encoder applies the decoder's predicate, so it cannot emit this stream,
    /// and a decoder test that needed it to would be testing the wrong thing.
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

    /// A `MTRF` v1/v2 trailer, hand-built: seven fixed `i64` slots and the
    /// seven-bit mask at `+24` and in every row.
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
