use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;
use core::mem;
use core::ops::Range;

use crate::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeContext {
    pub accepted_families: D43AcceptedFamilies,
    pub max_resource_count: u32,
    pub identity_stride: usize,
    pub max_owner_tag_db_id: u32,
    pub max_strategic_resource_token_len: usize,
    pub max_contract_id: u32,
    pub max_equipment_variant_id: u32,
    pub max_deal_identity_ordinal: u32,
    pub max_amount_raw: i64,
}

impl DecodeContext {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        accepted_families: D43AcceptedFamilies,
        max_resource_count: u32,
        identity_stride: usize,
        max_owner_tag_db_id: u32,
        max_strategic_resource_token_len: usize,
        max_contract_id: u32,
        max_equipment_variant_id: u32,
        max_deal_identity_ordinal: u32,
        max_amount_raw: i64,
    ) -> Self {
        Self {
            accepted_families,
            max_resource_count,
            identity_stride,
            max_owner_tag_db_id,
            max_strategic_resource_token_len,
            max_contract_id,
            max_equipment_variant_id,
            max_deal_identity_ordinal,
            max_amount_raw,
        }
    }

    pub const fn limits(self) -> Limits {
        Limits::new(self.max_resource_count, self.identity_stride)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct D43AcceptedFamilies(u16);

impl D43AcceptedFamilies {
    pub const SHIPPING: Self = Self(0x007f);
    pub const LANE_ALL: Self = Self(0x01ff);

    pub const fn contains(self, trailer: Trailer) -> bool {
        self.0 & family_bit(trailer) != 0
    }
}

const fn family_bit(trailer: Trailer) -> u16 {
    match trailer {
        Trailer::Detail => 1 << 0,
        Trailer::ResourceFlow => 1 << 1,
        Trailer::FactoryHeader => 1 << 2,
        Trailer::ResourceRow => 1 << 3,
        Trailer::ClassicDeal => 1 << 4,
        Trailer::Activation => 1 << 5,
        Trailer::Stockpile => 1 << 6,
        Trailer::Residual => 1 << 7,
        Trailer::LendLease => 1 << 8,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct D43DecodedRow<T> {
    pub index: usize,
    pub at: usize,
    pub row: T,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct D43DecodedFlowRow {
    pub index: usize,
    pub at: usize,
    pub row: FlowRow,
    pub values: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct D43DecodedIdentity {
    pub index: usize,
    pub at: usize,
    pub declared_len: usize,
    pub token: Option<Range<usize>>,
    pub token_text: Option<String>,
    pub padding: Option<Range<usize>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeDocument {
    pub core_header: Option<CoreHeader>,
    pub core_rows: Vec<D43DecodedRow<CoreRow>>,
    pub detail_header: Option<DetailHeader>,
    pub detail_rows: Vec<D43DecodedRow<DetailRow>>,
    pub flow_header: Option<FlowTrailer>,
    pub flow_rows: Vec<D43DecodedFlowRow>,
    pub flow_values: Vec<i64>,
    pub factory_header: Option<FactoryHeader>,
    pub factory_rows: Vec<D43DecodedRow<FactoryRow>>,
    pub resource_header: Option<ResourceRowTrailer>,
    pub resource_identities: Vec<D43DecodedIdentity>,
    pub resource_rows: Vec<D43DecodedRow<ResourceRow>>,
    pub deal_header: Option<DealTrailer>,
    pub deal_witnesses: Vec<D43DecodedRow<u32>>,
    pub deal_incomplete_owners: Vec<D43DecodedRow<u32>>,
    pub deal_rows: Vec<D43DecodedRow<DealRow>>,
    pub activation_header: Option<ActivationTrailer>,
    pub stockpile_header: Option<StockpileTrailer>,
    pub stockpile_countries: Vec<D43DecodedRow<StockpileCountryRow>>,
    pub stockpile_items: Vec<D43DecodedRow<StockpileItemRow>>,
    pub residual_header: Option<ResidualHeader>,
    pub residual_rows: Vec<D43DecodedRow<ResidualRow>>,
    pub lend_lease_header: Option<LendLeaseHeader>,
    pub lend_lease_rows: Vec<D43DecodedRow<LendLeaseRow>>,
    pub consumed: usize,
}

impl DecodeDocument {
    fn empty() -> Self {
        Self {
            core_header: None,
            core_rows: Vec::new(),
            detail_header: None,
            detail_rows: Vec::new(),
            flow_header: None,
            flow_rows: Vec::new(),
            flow_values: Vec::new(),
            factory_header: None,
            factory_rows: Vec::new(),
            resource_header: None,
            resource_identities: Vec::new(),
            resource_rows: Vec::new(),
            deal_header: None,
            deal_witnesses: Vec::new(),
            deal_incomplete_owners: Vec::new(),
            deal_rows: Vec::new(),
            activation_header: None,
            stockpile_header: None,
            stockpile_countries: Vec::new(),
            stockpile_items: Vec::new(),
            residual_header: None,
            residual_rows: Vec::new(),
            lend_lease_header: None,
            lend_lease_rows: Vec::new(),
            consumed: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct D43UnitCounts {
    pub market_trade_contract_rows: Option<usize>,
    pub market_trade_observed_contract_rows: Option<usize>,
    pub market_trade_omitted_candidate_rows: Option<usize>,
    pub market_trade_buyer_ptr_valid_rows: Option<usize>,
    pub market_trade_variant_valid_rows: Option<usize>,
    pub market_trade_amount_valid_rows: Option<usize>,
    pub market_trade_partial_field_rows: Option<usize>,
    pub market_trade_detail_rows: Option<usize>,
    pub market_trade_detail_omitted_rows: Option<usize>,
    pub market_trade_detail_price_valid_rows: Option<usize>,
    pub market_trade_detail_speed_valid_rows: Option<usize>,
    pub market_trade_detail_completed_valid_rows: Option<usize>,
    pub market_trade_detail_days_valid_rows: Option<usize>,
    pub market_trade_detail_delivery_country_valid_rows: Option<usize>,
    pub market_trade_detail_convoy_valid_rows: Option<usize>,
    pub market_trade_detail_efficiency_valid_rows: Option<usize>,
    pub market_trade_detail_delivery_route_valid_rows: Option<usize>,
    pub market_trade_detail_progress_valid_rows: Option<usize>,
    pub market_trade_resource_flow_rows: Option<usize>,
    pub market_trade_factory_header_rows: Option<usize>,
    pub market_trade_resource_rows: Option<usize>,
    pub market_trade_classic_deal_country_witnesses: Option<usize>,
    pub market_trade_classic_deal_incomplete_owners: Option<usize>,
    pub market_trade_classic_deal_rows: Option<usize>,
    pub market_trade_activation_rows: Option<usize>,
    pub market_trade_stockpile_country_rows: Option<usize>,
    pub market_trade_stockpile_item_rows: Option<usize>,
    pub fold_contract_keys: usize,
    pub fold_detail_contract_keys: usize,
    pub fold_resource_pool_countries: usize,
    pub fold_factory_header_countries: usize,
    pub fold_resource_row_countries: usize,
    pub fold_classic_deal_owner_countries: usize,
    pub fold_stockpile_countries: usize,
}

impl D43UnitCounts {
    fn zero() -> Self {
        Self {
            market_trade_contract_rows: None,
            market_trade_observed_contract_rows: None,
            market_trade_omitted_candidate_rows: None,
            market_trade_buyer_ptr_valid_rows: None,
            market_trade_variant_valid_rows: None,
            market_trade_amount_valid_rows: None,
            market_trade_partial_field_rows: None,
            market_trade_detail_rows: None,
            market_trade_detail_omitted_rows: None,
            market_trade_detail_price_valid_rows: None,
            market_trade_detail_speed_valid_rows: None,
            market_trade_detail_completed_valid_rows: None,
            market_trade_detail_days_valid_rows: None,
            market_trade_detail_delivery_country_valid_rows: None,
            market_trade_detail_convoy_valid_rows: None,
            market_trade_detail_efficiency_valid_rows: None,
            market_trade_detail_delivery_route_valid_rows: None,
            market_trade_detail_progress_valid_rows: None,
            market_trade_resource_flow_rows: None,
            market_trade_factory_header_rows: None,
            market_trade_resource_rows: None,
            market_trade_classic_deal_country_witnesses: None,
            market_trade_classic_deal_incomplete_owners: None,
            market_trade_classic_deal_rows: None,
            market_trade_activation_rows: None,
            market_trade_stockpile_country_rows: None,
            market_trade_stockpile_item_rows: None,
            fold_contract_keys: 0,
            fold_detail_contract_keys: 0,
            fold_resource_pool_countries: 0,
            fold_factory_header_countries: 0,
            fold_resource_row_countries: 0,
            fold_classic_deal_owner_countries: 0,
            fold_stockpile_countries: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum D43DecodeRule {
    D43FindingsSuppressed,
    D43DocumentCapacity,
    D43StructuralViolation,
    D43FamilyNotAccepted,
    D43CoreScale,
    D43CoreChangedRows,
    D43CoreEmittedRows,
    D43CoreObservedRows,
    D43CoreHeaderCountBound,
    D43CoreContractIdZero,
    D43CoreContractIdAuthorityRange,
    D43CoreDuplicateContractPair,
    D43CoreDuplicateContractId,
    D43CoreContractType,
    D43CoreZeroParty,
    D43CoreSellerAuthorityRange,
    D43CoreBuyerAuthorityRange,
    D43CoreUnknownFlags,
    D43CoreReserved,
    D43CoreBuyerStatusRange,
    D43CoreBuyerFlagStatus,
    D43CoreEquipmentStatusRange,
    D43CoreDeliveryStatusRange,
    D43CoreEquipmentFlagStatus,
    D43CoreDeliveryFlagStatus,
    D43CoreVariantRange,
    D43CoreVariantWithoutEquipment,
    D43CoreVariantWithoutFlag,
    D43CoreAmountRange,
    D43CoreAmountWithoutDelivery,
    D43CoreAmountWithoutFlag,
    D43CoreComputedCountMismatch,
    D43DetailContractIdZero,
    D43DetailContractIdAuthorityRange,
    D43DetailMissingCoreContract,
    D43DetailDuplicateContract,
    D43DetailInvalidFlags,
    D43DetailReserved,
    D43DetailPresentValueRange,
    D43DetailAbsentValueNonzero,
    D43DetailDeliveryCountryRange,
    D43DetailDeliveryCountryWithoutFlag,
    D43DetailConvoyRange,
    D43DetailConvoyWithoutFlag,
    D43DetailHeaderCountMismatch,
    D43SizedHeaderReserved0,
    D43FlowHeaderResourceCount,
    D43FlowHeaderResourceMask,
    D43FlowHeaderKindMask,
    D43FlowHeaderScale,
    D43FlowHeaderRowCount,
    D43FlowTagRange,
    D43FlowKind,
    D43FlowRowMaskOrCount,
    D43FlowReserved,
    D43FlowLegacyValuesAllZero,
    D43FlowDuplicateCountryKind,
    D43FlowDecodedCountryCount,
    D43FlowCountryKindsIncomplete,
    D43FactoryHeaderShape,
    D43FactoryTagRange,
    D43FactoryDuplicateTag,
    D43IdentityLength,
    D43IdentityPadding,
    D43IdentityUtf8,
    D43IdentityCharset,
    D43IdentityDuplicateToken,
    D43ResourceHeaderShape,
    D43ResourceTagRange,
    D43ResourceIndexRange,
    D43ResourceVisibleOrderRange,
    D43ResourceIndexOrderMismatch,
    D43ResourceDescriptorZero,
    D43ResourceReserved,
    D43ResourceUnavailableVersion,
    D43ResourceUnavailableShape,
    D43ResourceOilShape,
    D43ResourceNonOilFlags,
    D43ResourceNonOilCurrentMax,
    D43ResourceNonOilNeededBalance,
    D43ResourceNonOilNeededType,
    D43ResourceNonOilSign,
    D43ResourceDuplicateCountrySlot,
    D43ResourceDescriptorChanged,
    D43ResourceOilSlotCount,
    D43ResourceOilRowCount,
    D43ResourceOilWithoutCountry,
    D43ResourceDescriptorCoverage,
    D43ResourceCountryCount,
    D43ResourceCountryCoverage,
    D43DealHeaderShape,
    D43DealWitnessRange,
    D43DealWitnessDuplicate,
    D43DealIncompleteNotSubset,
    D43DealIncompleteNotAscending,
    D43DealOwnerNotWitnessed,
    D43DealCounterpartNotWitnessed,
    D43DealSameParty,
    D43DealResourceIndexRange,
    D43DealDirection,
    D43DealRoute,
    D43DealFlags,
    D43DealLegacyReservedStatus,
    D43DealStatusBits,
    D43DealPackedReserved,
    D43DealPackedResourceEcho,
    D43DealIdentityOrdinal,
    D43DealIdentityParties,
    D43DealRequestedScale,
    D43DealEfficiencyRange,
    D43DealReservedOrdinal,
    D43DealDuplicateVisibleOrder,
    D43DealHalfCount,
    D43DealHalfMismatch,
    D43DealVisibleOrderCoverage,
    D43ActivationFlags,
    D43ActivationRootExact,
    D43ActivationActiveCountry,
    D43ActivationSequence,
    D43ActivationCountryRange,
    D43ActivationRootKind,
    D43ActivationOpenRva,
    D43StockpileSegmentLength,
    D43StockpileCountryCount,
    D43StockpileExpectedCountryCount,
    D43StockpileScale,
    D43StockpileGlobalFlags,
    D43StockpileReserved,
    D43StockpileCountryTagRange,
    D43StockpileCountryDuplicate,
    D43StockpileCountryFirst,
    D43StockpileCountryRange,
    D43StockpileCountryFlags,
    D43StockpileCountryReserved,
    D43StockpileCountryCoverage,
    D43StockpileVariantRange,
    D43StockpileEquipmentType,
    D43StockpileOrdinal,
    D43StockpileItemReserved,
    D43StockpileAmountRange,
    D43StockpileDuplicateVariant,
    D43ResidualHeaderCounts,
    D43ResidualReserved,
    D43LendLeaseHeaderCounts,
    D43LendLeaseRowShape,
    D43CrossResourceCountAgreement,
    D43CrossDealResourceSlot,
    D43CrossCountrySetAgreement,
    D43CrossFlowV3NeedsIdentityResourceRows,
}

/// shipping formatters do not acquire an unreachable formatting surface. Each
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum D43LaneRule {
    D43LaneResidualHeaderCountSum,
    D43LaneResidualGapIdZero,
    D43LaneResidualStatusUnknown,
    D43LaneResidualEvidenceEmpty,
    D43LaneResidualCurrentSupportUnknown,
    D43LaneResidualCurrentDomainUnknown,
    D43LaneResidualProposedRecordKindUnknown,
    D43LaneResidualCurrentRowLenRange,
    D43LaneResidualSourceRefInvalid,
    D43LaneResidualOwnerBoundaryUnknown,
    D43LaneResidualRootStatusUnknown,
    D43LaneResidualProofStatusUnknown,
    D43LaneResidualReserved,
    D43LaneResidualDuplicateGapId,
    D43LaneResidualSourceBlockedCount,
    D43LaneResidualSchemaBlockedCount,
    D43LaneResidualOwnedElsewhereCount,
    D43LaneResidualSharedPatchBlockedCount,
    D43LaneDetailSpeedRange,
    D43LaneDetailCompletedRange,
    D43LaneDetailDaysRange,
    D43LaneDetailConvoysAssignedRange,
    D43LaneDetailConvoysTotalRange,
    D43LaneDetailEfficiencyRange,
    D43LaneDetailFactoryProgressRange,
    D43LaneDetailEquipmentProgressRange,
    D43LaneFlowCountryKindMask,
    D43LaneFlowDecodedCountryCount,
    D43LaneFlowLegacyExportedDecodedCount,
    D43LaneResourceUnknownFlags,
    D43LaneLendLeaseDirectionCountSum,
    D43LaneLendLeaseContractIdRange,
    D43LaneLendLeasePerspectiveRange,
    D43LaneLendLeaseCounterpartRange,
    D43LaneLendLeaseSameParty,
    D43LaneLendLeaseDirection,
    D43LaneLendLeaseVariantRange,
    D43LaneLendLeaseUnknownFlags,
    D43LaneLendLeaseAmountRange,
    D43LaneLendLeaseAmountWithoutFlag,
    D43LaneLendLeasePriceRange,
    D43LaneLendLeasePriceWithoutFlag,
    D43LaneLendLeaseSourceRecordKind,
    D43LaneLendLeaseReserved,
    D43LaneLendLeaseDuplicateKey,
    D43LaneLendLeaseContractHalfCount,
    D43LaneLendLeaseOutgoingCount,
    D43LaneLendLeaseIncomingCount,
    D43LaneLendLeaseAmountCount,
    D43LaneLendLeasePriceCount,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum D43RuleFatality {
    Fatal,
    NonFatal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum D43ValueField {
    D43PriceRaw,
    D43AmountRaw,
    D43SpeedRaw,
    D43CompletedRaw,
    D43Days,
    D43EfficiencyRaw,
    D43FactoryCicProgressRaw,
    D43EquipmentTransferProgressRaw,
}

impl D43ValueField {
    pub const fn offset(self) -> usize {
        match self {
            D43ValueField::D43PriceRaw => 8,
            D43ValueField::D43AmountRaw => 16,
            D43ValueField::D43SpeedRaw => 24,
            D43ValueField::D43CompletedRaw => 28,
            D43ValueField::D43Days => 32,
            D43ValueField::D43EfficiencyRaw => 48,
            D43ValueField::D43FactoryCicProgressRaw => 52,
            D43ValueField::D43EquipmentTransferProgressRaw => 56,
        }
    }
}

impl D43DecodeRule {
    pub const fn fatality(self) -> D43RuleFatality {
        match self {
            D43DecodeRule::D43FindingsSuppressed
            | D43DecodeRule::D43CoreChangedRows
            | D43DecodeRule::D43CoreEmittedRows
            | D43DecodeRule::D43CoreObservedRows
            | D43DecodeRule::D43CoreHeaderCountBound
            | D43DecodeRule::D43CoreContractIdAuthorityRange
            | D43DecodeRule::D43CoreDuplicateContractId
            | D43DecodeRule::D43CoreSellerAuthorityRange
            | D43DecodeRule::D43CoreBuyerAuthorityRange
            | D43DecodeRule::D43CoreBuyerStatusRange
            | D43DecodeRule::D43CoreBuyerFlagStatus
            | D43DecodeRule::D43CoreEquipmentStatusRange
            | D43DecodeRule::D43CoreDeliveryStatusRange
            | D43DecodeRule::D43CoreEquipmentFlagStatus
            | D43DecodeRule::D43CoreDeliveryFlagStatus
            | D43DecodeRule::D43CoreVariantRange
            | D43DecodeRule::D43CoreVariantWithoutEquipment
            | D43DecodeRule::D43CoreVariantWithoutFlag
            | D43DecodeRule::D43CoreAmountRange
            | D43DecodeRule::D43CoreAmountWithoutDelivery
            | D43DecodeRule::D43CoreAmountWithoutFlag
            | D43DecodeRule::D43CoreComputedCountMismatch
            | D43DecodeRule::D43DetailContractIdAuthorityRange
            | D43DecodeRule::D43DetailHeaderCountMismatch
            | D43DecodeRule::D43ResidualHeaderCounts
            | D43DecodeRule::D43ResidualReserved
            | D43DecodeRule::D43LendLeaseHeaderCounts
            | D43DecodeRule::D43LendLeaseRowShape => D43RuleFatality::NonFatal,
            _ => D43RuleFatality::Fatal,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct D43PolicyOrder {
    pub phase: u8,
    pub family_ordinal: u16,
    pub block_ordinal: u8,
    pub scope_ordinal: u8,
    pub row_or_slot: usize,
    pub rule_ordinal: u16,
    pub at: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum D43DecodeBlock {
    D43CoreHeader,
    D43CoreRows,
    D43TrailerMagic,
    D43TrailerHeader,
    D43Rows,
    D43Identities,
    D43Witnesses,
    D43IncompleteOwners,
    D43Countries,
    D43Items,
    D43CrossFamily,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct D43DealIdentityOperands {
    pub exporter_tag_db_id: u32,
    pub importer_tag_db_id: u32,
    pub resource_index: u32,
    pub ordinal: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Finding {
    pub rule: D43DecodeRule,
    pub family: Option<Trailer>,
    pub block: D43DecodeBlock,
    pub at: usize,
    pub row_or_slot: Option<usize>,
    pub operand_a: i64,
    pub operand_b: i64,
    pub operand_c: i64,
    pub operand_d: i64,
    pub other_family: Option<Trailer>,
    pub value_field: Option<D43ValueField>,
    pub deal_identity: Option<D43DealIdentityOperands>,
    pub lane_rule: Option<D43LaneRule>,
    pub suppressed_rule: Option<D43DecodeRule>,
    pub header_preview: Option<HeaderPreview>,
    pub fatality: D43RuleFatality,
    pub policy_order: D43PolicyOrder,
    pub structural: Option<Violation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum D43DecodeStop {
    D43Grammar(Violation),
    D43RejectedFamily { at: usize, trailer: Trailer },
    D43Allocation { at: usize, rows: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeReport {
    pub document: DecodeDocument,
    pub findings: Vec<Finding>,
    pub structural_stop: Option<D43DecodeStop>,
    pub units: D43UnitCounts,
    finding_tallies: BTreeMap<(D43DecodeRule, Option<D43LaneRule>), D43FindingTally>,
}

pub const D43_MAX_FINDINGS_PER_RULE: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct D43FindingTally {
    retained: usize,
    suppressed: usize,
    first_suppressed: Option<Finding>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum D43VerdictBoundary {
    D43TrailersAndCrossFamily,
    D43WholeRecord,
}

fn finding(
    rule: D43DecodeRule,
    family: Option<Trailer>,
    block: Option<TrailerBlock>,
    at: usize,
    row_or_slot: Option<usize>,
    operands: [i64; 4],
    other_family: Option<Trailer>,
    phase: u8,
    family_ordinal: u16,
    rule_ordinal: u16,
) -> Finding {
    let decoded_block = decode_block(rule, family, block, phase);
    Finding {
        rule,
        family,
        block: decoded_block,
        at,
        row_or_slot,
        operand_a: operands[0],
        operand_b: operands[1],
        operand_c: operands[2],
        operand_d: operands[3],
        other_family,
        value_field: None,
        deal_identity: None,
        lane_rule: None,
        suppressed_rule: None,
        header_preview: None,
        fatality: rule.fatality(),
        policy_order: D43PolicyOrder {
            phase,
            family_ordinal,
            block_ordinal: policy_block_ordinal(rule, family, decoded_block, phase),
            scope_ordinal: policy_scope(rule, row_or_slot),
            row_or_slot: row_or_slot.unwrap_or(usize::MAX),
            rule_ordinal,
            at,
        },
        structural: None,
    }
}

fn policy_block_ordinal(
    rule: D43DecodeRule,
    family: Option<Trailer>,
    block: D43DecodeBlock,
    phase: u8,
) -> u8 {
    if phase != 1 {
        if phase == 0 && rule == D43DecodeRule::D43CoreScale {
            return 2;
        }
        return match block {
            D43DecodeBlock::D43CoreHeader | D43DecodeBlock::D43CrossFamily => 0,
            D43DecodeBlock::D43CoreRows => 1,
            D43DecodeBlock::D43Rows => 2,
            _ => 0,
        };
    }
    if policy_scope(rule, None) == 2 {
        return 5;
    }
    match block {
        D43DecodeBlock::D43TrailerMagic => 0,
        D43DecodeBlock::D43TrailerHeader => 1,
        D43DecodeBlock::D43Identities
        | D43DecodeBlock::D43Witnesses
        | D43DecodeBlock::D43Countries => 2,
        D43DecodeBlock::D43IncompleteOwners | D43DecodeBlock::D43Items => 3,
        D43DecodeBlock::D43Rows => match family {
            Some(Trailer::ResourceRow) => 3,
            Some(Trailer::ClassicDeal) => 4,
            _ => 2,
        },
        _ => 0,
    }
}

fn decode_block(
    rule: D43DecodeRule,
    family: Option<Trailer>,
    block: Option<TrailerBlock>,
    phase: u8,
) -> D43DecodeBlock {
    match block {
        Some(TrailerBlock::Rows) => D43DecodeBlock::D43Rows,
        Some(TrailerBlock::Identities) => D43DecodeBlock::D43Identities,
        Some(TrailerBlock::Witnesses) => D43DecodeBlock::D43Witnesses,
        Some(TrailerBlock::IncompleteOwners) => D43DecodeBlock::D43IncompleteOwners,
        Some(TrailerBlock::Countries) => D43DecodeBlock::D43Countries,
        Some(TrailerBlock::Items) => D43DecodeBlock::D43Items,
        None if phase == 0 => D43DecodeBlock::D43CoreHeader,
        None if phase == 2 => D43DecodeBlock::D43CrossFamily,
        None if phase == 1 && policy_scope(rule, None) == 2 => D43DecodeBlock::D43Rows,
        None if phase == 3 && core_header_rule(rule) => D43DecodeBlock::D43CoreHeader,
        None if phase == 3 => D43DecodeBlock::D43CoreRows,
        None if rule == D43DecodeRule::D43FamilyNotAccepted || family.is_none() => {
            D43DecodeBlock::D43TrailerMagic
        }
        None => D43DecodeBlock::D43TrailerHeader,
    }
}

fn core_header_rule(rule: D43DecodeRule) -> bool {
    matches!(
        rule,
        D43DecodeRule::D43CoreChangedRows
            | D43DecodeRule::D43CoreEmittedRows
            | D43DecodeRule::D43CoreObservedRows
            | D43DecodeRule::D43CoreHeaderCountBound
            | D43DecodeRule::D43CoreComputedCountMismatch
    )
}

fn policy_scope(rule: D43DecodeRule, row_or_slot: Option<usize>) -> u8 {
    if matches!(
        rule,
        D43DecodeRule::D43DetailHeaderCountMismatch
            | D43DecodeRule::D43FlowDecodedCountryCount
            | D43DecodeRule::D43FlowCountryKindsIncomplete
            | D43DecodeRule::D43ResourceOilSlotCount
            | D43DecodeRule::D43ResourceOilRowCount
            | D43DecodeRule::D43ResourceOilWithoutCountry
            | D43DecodeRule::D43ResourceDescriptorCoverage
            | D43DecodeRule::D43ResourceCountryCount
            | D43DecodeRule::D43ResourceCountryCoverage
            | D43DecodeRule::D43DealHalfCount
            | D43DecodeRule::D43DealHalfMismatch
            | D43DecodeRule::D43DealVisibleOrderCoverage
            | D43DecodeRule::D43StockpileCountryCoverage
            | D43DecodeRule::D43CoreComputedCountMismatch
            | D43DecodeRule::D43DetailMissingCoreContract
    ) {
        2
    } else if row_or_slot.is_some() {
        1
    } else {
        0
    }
}

fn push_rule(
    report: &mut DecodeReport,
    rule: D43DecodeRule,
    family: Option<Trailer>,
    block: Option<TrailerBlock>,
    at: usize,
    row_or_slot: Option<usize>,
    operands: [i64; 4],
    other_family: Option<Trailer>,
    phase: u8,
    family_ordinal: u16,
    rule_ordinal: u16,
) {
    retain_finding(
        report,
        finding(
            rule,
            family,
            block,
            at,
            row_or_slot,
            operands,
            other_family,
            phase,
            family_ordinal,
            rule_ordinal,
        ),
    );
}

fn retain_finding(report: &mut DecodeReport, item: Finding) {
    let key = (item.rule, item.lane_rule);
    let retain = {
        let tally = report
            .finding_tallies
            .entry(key)
            .or_insert(D43FindingTally {
                retained: 0,
                suppressed: 0,
                first_suppressed: None,
            });
        if tally.retained < D43_MAX_FINDINGS_PER_RULE {
            tally.retained += 1;
            true
        } else {
            tally.suppressed = tally.suppressed.saturating_add(1);
            if tally.first_suppressed.is_none() {
                tally.first_suppressed = Some(item);
            }
            false
        }
    };
    if retain {
        report.findings.push(item);
    }
}

fn finalize_suppressed_findings(report: &mut DecodeReport) {
    let tallies = mem::take(&mut report.finding_tallies);
    for ((rule, lane_rule), tally) in tallies {
        if tally.suppressed == 0 {
            continue;
        }
        if let Some(mut item) = tally.first_suppressed {
            item.rule = D43DecodeRule::D43FindingsSuppressed;
            item.suppressed_rule = Some(rule);
            item.lane_rule = lane_rule;
            item.operand_a = tally.suppressed as i64;
            item.operand_b = 0;
            item.operand_c = 0;
            item.operand_d = 0;
            item.fatality = D43RuleFatality::NonFatal;
            report.findings.push(item);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn push_lane_rule(
    report: &mut DecodeReport,
    rule: D43DecodeRule,
    lane_rule: D43LaneRule,
    family: Trailer,
    at: usize,
    row_or_slot: Option<usize>,
    operands: [i64; 4],
    family_ordinal: u16,
    rule_ordinal: u16,
) {
    let mut item = finding(
        rule,
        Some(family),
        Some(TrailerBlock::Rows),
        at,
        row_or_slot,
        operands,
        None,
        1,
        family_ordinal,
        rule_ordinal,
    );
    item.lane_rule = Some(lane_rule);
    item.fatality = D43RuleFatality::NonFatal;
    retain_finding(report, item);
}

fn push_deal_rule(
    report: &mut DecodeReport,
    rule: D43DecodeRule,
    at: usize,
    identity: D43DealIdentityOperands,
    identity_order: usize,
    half_count: usize,
    family_ordinal: u16,
    rule_ordinal: u16,
) {
    let mut item = finding(
        rule,
        Some(Trailer::ClassicDeal),
        Some(TrailerBlock::Rows),
        at,
        Some(identity_order),
        [half_count as i64, 0, 0, 0],
        None,
        1,
        family_ordinal,
        rule_ordinal,
    );
    item.deal_identity = Some(identity);
    retain_finding(report, item);
}

fn stop_on_violation(
    report: &mut DecodeReport,
    violation: Violation,
    preview: Option<HeaderPreview>,
    phase: u8,
    family_ordinal: u16,
) {
    if matches!(
        violation.kind,
        ViolationKind::ResourceCount {
            trailer: Trailer::ResourceFlow,
            ..
        }
    ) && report.findings.iter().any(|item| {
        item.rule == D43DecodeRule::D43FlowHeaderResourceCount
            && item.policy_order.family_ordinal == family_ordinal
    }) {
        report.structural_stop = Some(D43DecodeStop::D43Grammar(violation));
        return;
    }
    let rule_ordinal = match violation.kind {
        ViolationKind::HeaderLen {
            trailer: Trailer::ResourceFlow,
            ..
        } => 2,
        _ => 0,
    };
    let mut item = finding(
        D43DecodeRule::D43StructuralViolation,
        violation_family(violation),
        violation_block(violation),
        violation.at,
        None,
        structural_operands(preview),
        None,
        phase,
        family_ordinal,
        rule_ordinal,
    );
    item.structural = Some(violation);
    item.header_preview = preview;
    item.block = violation_decode_block(violation, item.block);
    item.policy_order.block_ordinal =
        policy_block_ordinal(item.rule, item.family, item.block, item.policy_order.phase);
    retain_finding(report, item);
    report.structural_stop = Some(D43DecodeStop::D43Grammar(violation));
}

fn structural_operands(preview: Option<HeaderPreview>) -> [i64; 4] {
    match preview {
        Some(HeaderPreview::ResourceFlow(words)) => [
            words.sized.row_count as i64,
            words.country_count_or_exported as i64,
            words.kind_mask as i64,
            words.mask_or_count as i64,
        ],
        Some(HeaderPreview::ResourceRow(words)) => [
            words.sized.prefix.version as i64,
            words.resource_count as i64,
            words.identity_stride_or_reserved as i64,
            words.sized.row_count as i64,
        ],
        _ => [0; 4],
    }
}

fn violation_decode_block(violation: Violation, fallback: D43DecodeBlock) -> D43DecodeBlock {
    match violation.kind {
        ViolationKind::DuplicateTrailer { .. } => D43DecodeBlock::D43TrailerMagic,
        ViolationKind::Need {
            section: Section::CoreHeader,
            ..
        }
        | ViolationKind::OffsetOverflow {
            section: Section::CoreHeader,
            ..
        } => D43DecodeBlock::D43CoreHeader,
        ViolationKind::Need {
            section: Section::CoreRows,
            ..
        }
        | ViolationKind::OffsetOverflow {
            section: Section::CoreRows,
            ..
        } => D43DecodeBlock::D43CoreRows,
        ViolationKind::Need {
            section: Section::TrailerMagic,
            ..
        }
        | ViolationKind::OffsetOverflow {
            section: Section::TrailerMagic,
            ..
        } => D43DecodeBlock::D43TrailerMagic,
        _ => fallback,
    }
}

fn violation_family(violation: Violation) -> Option<Trailer> {
    match violation.kind {
        ViolationKind::UnknownVersion { trailer, .. }
        | ViolationKind::HeaderLen { trailer, .. }
        | ViolationKind::RowLen { trailer, .. }
        | ViolationKind::ResourceCount { trailer, .. }
        | ViolationKind::IdentityStride { trailer, .. }
        | ViolationKind::AbsentBlockDeclared { trailer, .. }
        | ViolationKind::DuplicateTrailer { trailer, .. } => Some(trailer),
        ViolationKind::Need {
            section: Section::TrailerHeader(t),
            ..
        }
        | ViolationKind::Need {
            section: Section::TrailerBody(t),
            ..
        }
        | ViolationKind::OffsetOverflow {
            section: Section::TrailerHeader(t),
            ..
        }
        | ViolationKind::OffsetOverflow {
            section: Section::TrailerBody(t),
            ..
        } => Some(t),
        ViolationKind::CallerDefect(CallerDefect::BlockNotConsumed { trailer, .. }) => {
            Some(trailer)
        }
        _ => None,
    }
}

fn violation_block(violation: Violation) -> Option<TrailerBlock> {
    match violation.kind {
        ViolationKind::CallerDefect(CallerDefect::BlockNotConsumed { block, .. }) => Some(block),
        _ => None,
    }
}

pub fn decode_record(payload: &[u8], ctx: DecodeContext) -> DecodeReport {
    let mut report = DecodeReport {
        document: DecodeDocument::empty(),
        findings: Vec::new(),
        structural_stop: None,
        units: D43UnitCounts::zero(),
        finding_tallies: BTreeMap::new(),
    };
    let header = match read_core_header(payload) {
        Ok(value) => value,
        Err(err) => {
            stop_on_violation(&mut report, err, None, 0, 0);
            finalize_suppressed_findings(&mut report);
            return report;
        }
    };
    report.document.core_header = Some(header);
    evaluate_core_header(&mut report, header);
    let core_finish = match core_end(payload, header.row_count()) {
        Ok(value) => value,
        Err(err) => {
            stop_on_violation(&mut report, err, None, 0, 0);
            compute_units(&mut report);
            finalize_suppressed_findings(&mut report);
            return report;
        }
    };

    let declared_core_rows = header.row_count() as usize;
    if report
        .document
        .core_rows
        .try_reserve_exact(declared_core_rows)
        .is_err()
    {
        push_rule(
            &mut report,
            D43DecodeRule::D43DocumentCapacity,
            None,
            None,
            CORE_HEADER_LEN,
            None,
            [
                declared_core_rows as i64,
                core::mem::size_of::<D43DecodedRow<CoreRow>>() as i64,
                0,
                0,
            ],
            None,
            0,
            0,
            1,
        );
        report.structural_stop = Some(D43DecodeStop::D43Allocation {
            at: CORE_HEADER_LEN,
            rows: declared_core_rows,
        });
        compute_units(&mut report);
        finalize_suppressed_findings(&mut report);
        return report;
    }

    let mut core = CoreRowCursor::new(header.row_count());
    loop {
        match core.next(payload) {
            CoreRowStep::Row { index, row } => report.document.core_rows.push(D43DecodedRow {
                index,
                at: CORE_HEADER_LEN + index * CORE_ROW_LEN,
                row,
            }),
            CoreRowStep::End(_) => break,
        }
    }
    evaluate_core_rows(&mut report, ctx);

    let mut cursor = TrailerCursor::new(core_finish);
    let mut trailer_ordinal = 0u16;
    loop {
        let step = match cursor.next(payload) {
            Ok(value) => value,
            Err(err) => {
                stop_on_violation(&mut report, err, None, 1, trailer_ordinal);
                break;
            }
        };
        let (magic_at, trailer) = match step {
            TrailerStep::End(at) => {
                report.document.consumed = at;
                if let Err(err) = exact_consumption(at, payload.len()) {
                    stop_on_violation(&mut report, err, None, 1, trailer_ordinal);
                }
                break;
            }
            TrailerStep::Magic { at, trailer } => (at, trailer),
        };
        if !ctx.accepted_families.contains(trailer) {
            push_rule(
                &mut report,
                D43DecodeRule::D43FamilyNotAccepted,
                Some(trailer),
                None,
                magic_at,
                None,
                [0; 4],
                None,
                1,
                trailer_ordinal,
                0,
            );
            report.structural_stop = Some(D43DecodeStop::D43RejectedFamily {
                at: magic_at,
                trailer,
            });
            break;
        }

        // the structural decision and cursor ownership.
        let preview = match cursor.peek_header(payload) {
            Ok(words) => words,
            Err(err) => {
                stop_on_violation(&mut report, err, None, 1, trailer_ordinal);
                break;
            }
        };
        evaluate_header_preview(&mut report, preview, ctx, magic_at, trailer_ordinal);
        let body = match cursor.read_body(payload, ctx.limits()) {
            Ok(value) => value,
            Err(err) => {
                stop_on_violation(&mut report, err, Some(preview), 1, trailer_ordinal);
                break;
            }
        };
        let next = match walk_body(&mut report, payload, ctx, body, trailer_ordinal) {
            Ok(value) => value,
            Err(err) => {
                stop_on_violation(&mut report, err, Some(preview), 1, trailer_ordinal);
                break;
            }
        };
        report.document.consumed = next.offset();
        cursor = next;
        trailer_ordinal = trailer_ordinal.saturating_add(1);
    }

    if report.structural_stop.is_none() {
        evaluate_cross_family(&mut report, ctx);
    }
    compute_units(&mut report);
    finalize_suppressed_findings(&mut report);
    report
}

pub fn select_first_fatal(report: &DecodeReport) -> Option<&Finding> {
    select_first_fatal_through(report, D43VerdictBoundary::D43WholeRecord)
}

pub fn select_first_fatal_through(
    report: &DecodeReport,
    boundary: D43VerdictBoundary,
) -> Option<&Finding> {
    let last_phase = match boundary {
        D43VerdictBoundary::D43TrailersAndCrossFamily => 2,
        D43VerdictBoundary::D43WholeRecord => u8::MAX,
    };
    report
        .findings
        .iter()
        .filter(|item| {
            item.fatality == D43RuleFatality::Fatal && item.policy_order.phase <= last_phase
        })
        .min_by_key(|item| item.policy_order)
}

pub fn identity_token(identity: &D43DecodedIdentity) -> Option<&str> {
    identity.token_text.as_deref()
}

fn walk_body(
    report: &mut DecodeReport,
    payload: &[u8],
    ctx: DecodeContext,
    body: TrailerBody,
    family_ordinal: u16,
) -> Result<TrailerCursor, Violation> {
    match body {
        TrailerBody::Detail(body) => {
            let header = *body.header();
            report.document.detail_header = Some(header);
            let mut rows = body.rows();
            while let RowStep::Row { index, at } = rows.next() {
                report.document.detail_rows.push(D43DecodedRow {
                    index,
                    at,
                    row: detail_row_at(payload, at),
                });
            }
            evaluate_detail(report, ctx, header, family_ordinal);
            Ok(rows.into_end()?.next.resume())
        }
        TrailerBody::ResourceFlow(body) => {
            let header = *body.header();
            report.document.flow_header = Some(header);
            let mut rows = body.rows();
            while let RowStep::Row { index, at } = rows.next() {
                let row = flow_row_at(payload, at);
                let start = report.document.flow_values.len();
                let mut slot = 0usize;
                while slot < header.resource_count as usize {
                    report
                        .document
                        .flow_values
                        .push(flow_value_at(payload, at, slot));
                    slot += 1;
                }
                let end = report.document.flow_values.len();
                report.document.flow_rows.push(D43DecodedFlowRow {
                    index,
                    at,
                    row,
                    values: start..end,
                });
            }
            evaluate_flow(report, ctx, header, family_ordinal);
            Ok(rows.into_end()?.next.resume())
        }
        TrailerBody::FactoryHeader(body) => {
            let header = *body.header();
            report.document.factory_header = Some(header);
            let mut rows = body.rows();
            while let RowStep::Row { index, at } = rows.next() {
                report.document.factory_rows.push(D43DecodedRow {
                    index,
                    at,
                    row: factory_row_at(payload, at),
                });
            }
            evaluate_factory(report, ctx, header, family_ordinal);
            Ok(rows.into_end()?.next.resume())
        }
        TrailerBody::ResourceRow(body) => {
            let header = *body.header();
            report.document.resource_header = Some(header);
            let mut identities = body.identities();
            while let RowStep::Row { index, at } = identities.next() {
                let entry = identity_entry_at(payload, at, header.identity_stride);
                let (declared_len, token, token_text, padding) = match entry {
                    Some(value) => {
                        let token = value.token.map(|bytes| {
                            let start = at + 2;
                            start..start + bytes.len()
                        });
                        let token_text = value
                            .token
                            .and_then(|bytes| core::str::from_utf8(bytes).ok())
                            .map(String::from);
                        let padding = value.padding.map(|bytes| {
                            let start = at + 2 + value.declared_len;
                            start..start + bytes.len()
                        });
                        (value.declared_len, token, token_text, padding)
                    }
                    None => (0, None, None, None),
                };
                report
                    .document
                    .resource_identities
                    .push(D43DecodedIdentity {
                        index,
                        at,
                        declared_len,
                        token,
                        token_text,
                        padding,
                    });
            }
            let identity_end = identities.into_end()?;
            let mut rows = identity_end.next.rows();
            while let RowStep::Row { index, at } = rows.next() {
                report.document.resource_rows.push(D43DecodedRow {
                    index,
                    at,
                    row: resource_row_at(payload, at),
                });
            }
            evaluate_resource(report, payload, ctx, header, family_ordinal);
            Ok(rows.into_end()?.next.resume())
        }
        TrailerBody::ClassicDeal(body) => {
            let header = *body.header();
            report.document.deal_header = Some(header);
            let mut witnesses = body.witnesses();
            while let RowStep::Row { index, at } = witnesses.next() {
                report.document.deal_witnesses.push(D43DecodedRow {
                    index,
                    at,
                    row: owner_word_at(payload, at),
                });
            }
            let witnesses_end = witnesses.into_end()?;
            let mut incomplete = witnesses_end.next.incomplete_owners();
            while let RowStep::Row { index, at } = incomplete.next() {
                report.document.deal_incomplete_owners.push(D43DecodedRow {
                    index,
                    at,
                    row: owner_word_at(payload, at),
                });
            }
            let incomplete_end = incomplete.into_end()?;
            let mut rows = incomplete_end.next.rows();
            while let RowStep::Row { index, at } = rows.next() {
                report.document.deal_rows.push(D43DecodedRow {
                    index,
                    at,
                    row: deal_row_at(payload, at, header.version),
                });
            }
            evaluate_deals(report, ctx, header, family_ordinal);
            Ok(rows.into_end()?.next.resume())
        }
        TrailerBody::Activation(body) => {
            let header = *body.header();
            report.document.activation_header = Some(header);
            evaluate_activation(report, ctx, header, family_ordinal);
            Ok(body.end().next.resume())
        }
        TrailerBody::Stockpile(body) => {
            let header = *body.header();
            report.document.stockpile_header = Some(header);
            let mut countries = body.countries();
            while let RowStep::Row { index, at } = countries.next() {
                report.document.stockpile_countries.push(D43DecodedRow {
                    index,
                    at,
                    row: stockpile_country_row_at(payload, at),
                });
            }
            let countries_end = countries.into_end()?;
            let mut items = countries_end.next.items();
            while let RowStep::Row { index, at } = items.next() {
                report.document.stockpile_items.push(D43DecodedRow {
                    index,
                    at,
                    row: stockpile_item_row_at(payload, at),
                });
            }
            evaluate_stockpile(report, ctx, header, family_ordinal);
            Ok(items.into_end()?.next.resume())
        }
        TrailerBody::Residual(body) => {
            let header = *body.header();
            report.document.residual_header = Some(header);
            let mut rows = body.rows();
            while let RowStep::Row { index, at } = rows.next() {
                report.document.residual_rows.push(D43DecodedRow {
                    index,
                    at,
                    row: residual_row_at(payload, at),
                });
            }
            evaluate_residual(report, header, family_ordinal);
            Ok(rows.into_end()?.next.resume())
        }
        TrailerBody::LendLease(body) => {
            let header = *body.header();
            report.document.lend_lease_header = Some(header);
            let mut rows = body.rows();
            while let RowStep::Row { index, at } = rows.next() {
                report.document.lend_lease_rows.push(D43DecodedRow {
                    index,
                    at,
                    row: lend_lease_row_at(payload, at),
                });
            }
            evaluate_lend_lease(report, ctx, header, family_ordinal);
            Ok(rows.into_end()?.next.resume())
        }
    }
}

fn evaluate_core_header(report: &mut DecodeReport, header: CoreHeader) {
    let row_count = header.word(CoreWord::RowCount);
    let changed = header.word(CoreWord::ChangedRows);
    let observed = header.word(CoreWord::ObservedContractRows);
    let emitted = header.word(CoreWord::EmittedContractRows);
    if changed > row_count {
        push_rule(
            report,
            D43DecodeRule::D43CoreChangedRows,
            None,
            None,
            CoreWord::ChangedRows.offset(),
            None,
            [changed as i64, row_count as i64, 0, 0],
            None,
            3,
            0,
            1,
        );
    }
    if emitted != row_count {
        push_rule(
            report,
            D43DecodeRule::D43CoreEmittedRows,
            None,
            None,
            CoreWord::EmittedContractRows.offset(),
            None,
            [emitted as i64, row_count as i64, 0, 0],
            None,
            3,
            0,
            2,
        );
    }
    if observed < row_count {
        push_rule(
            report,
            D43DecodeRule::D43CoreObservedRows,
            None,
            None,
            CoreWord::ObservedContractRows.offset(),
            None,
            [observed as i64, row_count as i64, 0, 0],
            None,
            3,
            0,
            3,
        );
    }
    for (ordinal, word) in [
        CoreWord::BuyerPtrValidRows,
        CoreWord::VariantValidRows,
        CoreWord::AmountValidRows,
        CoreWord::PartialFieldRows,
    ]
    .into_iter()
    .enumerate()
    {
        let count = header.word(word);
        if count > row_count {
            push_rule(
                report,
                D43DecodeRule::D43CoreHeaderCountBound,
                None,
                None,
                word.offset(),
                None,
                [word.offset() as i64, count as i64, row_count as i64, 0],
                None,
                3,
                0,
                4 + ordinal as u16,
            );
        }
    }
    if header.word(CoreWord::ScaleRaw) != FIXED_SCALE_RAW {
        push_rule(
            report,
            D43DecodeRule::D43CoreScale,
            None,
            None,
            CoreWord::ScaleRaw.offset(),
            None,
            [
                header.word(CoreWord::ScaleRaw) as i64,
                FIXED_SCALE_RAW as i64,
                0,
                0,
            ],
            None,
            0,
            0,
            1,
        );
    }
}

fn evaluate_header_preview(
    report: &mut DecodeReport,
    preview: HeaderPreview,
    ctx: DecodeContext,
    at: usize,
    family_ordinal: u16,
) {
    let (family, sized) = match preview {
        HeaderPreview::Detail(words) => (Trailer::Detail, Some(words.sized)),
        HeaderPreview::ResourceFlow(words) => {
            if words.sized.prefix.version == FlowVersion::Variable.wire()
                && (words.mask_or_count == 0 || words.mask_or_count > ctx.max_resource_count)
            {
                push_rule(
                    report,
                    D43DecodeRule::D43FlowHeaderResourceCount,
                    Some(Trailer::ResourceFlow),
                    None,
                    at + 24,
                    None,
                    [
                        words.mask_or_count as i64,
                        ctx.max_resource_count as i64,
                        0,
                        0,
                    ],
                    None,
                    1,
                    family_ordinal,
                    1,
                );
            }
            (Trailer::ResourceFlow, Some(words.sized))
        }
        HeaderPreview::FactoryHeader(words) => (Trailer::FactoryHeader, Some(words.sized)),
        HeaderPreview::ResourceRow(words) => (Trailer::ResourceRow, Some(words.sized)),
        HeaderPreview::ClassicDeal(words) => (Trailer::ClassicDeal, Some(words.sized)),
        HeaderPreview::Activation(_) => (Trailer::Activation, None),
        HeaderPreview::Stockpile(_) => (Trailer::Stockpile, None),
        HeaderPreview::Residual(words) => (Trailer::Residual, Some(words.sized)),
        HeaderPreview::LendLease(words) => (Trailer::LendLease, Some(words.sized)),
    };
    if let Some(words) = sized {
        if words.reserved0 != 0 {
            push_rule(
                report,
                D43DecodeRule::D43SizedHeaderReserved0,
                Some(family),
                None,
                at + 10,
                None,
                [words.reserved0 as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                2,
            );
        }
    }
}

fn evaluate_core_rows(report: &mut DecodeReport, ctx: DecodeContext) {
    let mut seen_pairs = BTreeSet::new();
    let mut seen_ids = BTreeSet::new();
    let mut computed = [0u32; 4];
    let row_count = report.document.core_rows.len();
    for position in 0..row_count {
        let (row, at, index) = {
            let item = &report.document.core_rows[position];
            (item.row, item.at, item.index)
        };
        let idx = Some(index);
        let buyer = RowFlag::BuyerPtrValid.is_set(row.flags);
        let equipment = RowFlag::EquipmentSubbufValid.is_set(row.flags);
        let delivery = RowFlag::DeliverySubbufValid.is_set(row.flags);
        let variant = RowFlag::VariantValid.is_set(row.flags);
        let amount = RowFlag::AmountValid.is_set(row.flags);
        computed[0] += u32::from(buyer);
        computed[1] += u32::from(variant);
        computed[2] += u32::from(amount);
        computed[3] += u32::from(!(buyer && equipment && delivery && variant && amount));
        if row.contract_id == 0 {
            push_rule(
                report,
                D43DecodeRule::D43CoreContractIdZero,
                None,
                None,
                at,
                idx,
                [0; 4],
                None,
                3,
                0,
                20,
            );
        }
        if row.contract_id == 0 || row.contract_id > ctx.max_contract_id {
            push_rule(
                report,
                D43DecodeRule::D43CoreContractIdAuthorityRange,
                None,
                None,
                at,
                idx,
                [row.contract_id as i64, ctx.max_contract_id as i64, 0, 0],
                None,
                3,
                0,
                21,
            );
        } else if !seen_ids.insert(row.contract_id) {
            push_rule(
                report,
                D43DecodeRule::D43CoreDuplicateContractId,
                None,
                None,
                at,
                idx,
                [row.contract_id as i64, 0, 0, 0],
                None,
                3,
                0,
                22,
            );
        }
        if !seen_pairs.insert((row.contract_id, row.contract_type)) {
            push_rule(
                report,
                D43DecodeRule::D43CoreDuplicateContractPair,
                None,
                None,
                at,
                idx,
                [row.contract_id as i64, row.contract_type as i64, 0, 0],
                None,
                3,
                0,
                23,
            );
        }
        if row.contract_type != CORE_CONTRACT_TYPE {
            push_rule(
                report,
                D43DecodeRule::D43CoreContractType,
                None,
                None,
                at + 4,
                idx,
                [row.contract_type as i64, CORE_CONTRACT_TYPE as i64, 0, 0],
                None,
                3,
                0,
                24,
            );
        }
        if row.seller_tag_db_id == 0 || row.buyer_tag_db_id == 0 {
            push_rule(
                report,
                D43DecodeRule::D43CoreZeroParty,
                None,
                None,
                at + 12,
                idx,
                [
                    row.seller_tag_db_id as i64,
                    row.buyer_tag_db_id as i64,
                    0,
                    0,
                ],
                None,
                3,
                0,
                25,
            );
        }
        if row.seller_tag_db_id == 0 || row.seller_tag_db_id > ctx.max_owner_tag_db_id {
            push_rule(
                report,
                D43DecodeRule::D43CoreSellerAuthorityRange,
                None,
                None,
                at + 12,
                idx,
                [
                    row.seller_tag_db_id as i64,
                    ctx.max_owner_tag_db_id as i64,
                    0,
                    0,
                ],
                None,
                3,
                0,
                26,
            );
        }
        if row.buyer_tag_db_id == 0 || row.buyer_tag_db_id > ctx.max_owner_tag_db_id {
            push_rule(
                report,
                D43DecodeRule::D43CoreBuyerAuthorityRange,
                None,
                None,
                at + 16,
                idx,
                [
                    row.buyer_tag_db_id as i64,
                    ctx.max_owner_tag_db_id as i64,
                    0,
                    0,
                ],
                None,
                3,
                0,
                27,
            );
        }
        if row.flags & !KNOWN_ROW_FLAGS != 0 {
            push_rule(
                report,
                D43DecodeRule::D43CoreUnknownFlags,
                None,
                None,
                at + 8,
                idx,
                [row.flags as i64, KNOWN_ROW_FLAGS as i64, 0, 0],
                None,
                3,
                0,
                28,
            );
        }
        if row.reserved != 0 {
            push_rule(
                report,
                D43DecodeRule::D43CoreReserved,
                None,
                None,
                at + 44,
                idx,
                [row.reserved as i64, 0, 0, 0],
                None,
                3,
                0,
                29,
            );
        }
        core_status_rules(
            report, ctx, row, at, index, buyer, equipment, delivery, variant, amount,
        );
    }
    if let Some(header) = report.document.core_header {
        for (ordinal, (word, actual)) in [
            (CoreWord::BuyerPtrValidRows, computed[0]),
            (CoreWord::VariantValidRows, computed[1]),
            (CoreWord::AmountValidRows, computed[2]),
            (CoreWord::PartialFieldRows, computed[3]),
        ]
        .into_iter()
        .enumerate()
        {
            let expected = header.word(word);
            if expected != actual {
                push_rule(
                    report,
                    D43DecodeRule::D43CoreComputedCountMismatch,
                    None,
                    None,
                    word.offset(),
                    None,
                    [word.offset() as i64, expected as i64, actual as i64, 0],
                    None,
                    3,
                    0,
                    60 + ordinal as u16,
                );
            }
        }
    }
}

fn core_status_rules(
    report: &mut DecodeReport,
    ctx: DecodeContext,
    row: CoreRow,
    at: usize,
    index: usize,
    buyer: bool,
    equipment: bool,
    delivery: bool,
    variant: bool,
    amount: bool,
) {
    let idx = Some(index);
    if row.buyer_ptr_status > 2 {
        push_rule(
            report,
            D43DecodeRule::D43CoreBuyerStatusRange,
            None,
            None,
            at + 40,
            idx,
            [row.buyer_ptr_status as i64, 2, 0, 0],
            None,
            3,
            0,
            30,
        );
    }
    if buyer != (row.buyer_ptr_status == 1) {
        push_rule(
            report,
            D43DecodeRule::D43CoreBuyerFlagStatus,
            None,
            None,
            at + 40,
            idx,
            [buyer as i64, row.buyer_ptr_status as i64, 0, 0],
            None,
            3,
            0,
            31,
        );
    }
    if row.equipment_subbuf_status > 2 {
        push_rule(
            report,
            D43DecodeRule::D43CoreEquipmentStatusRange,
            None,
            None,
            at + 32,
            idx,
            [row.equipment_subbuf_status as i64, 2, 0, 0],
            None,
            3,
            0,
            32,
        );
    }
    if row.delivery_subbuf_status > 2 {
        push_rule(
            report,
            D43DecodeRule::D43CoreDeliveryStatusRange,
            None,
            None,
            at + 36,
            idx,
            [row.delivery_subbuf_status as i64, 2, 0, 0],
            None,
            3,
            0,
            33,
        );
    }
    if equipment != (row.equipment_subbuf_status == 1) {
        push_rule(
            report,
            D43DecodeRule::D43CoreEquipmentFlagStatus,
            None,
            None,
            at + 32,
            idx,
            [equipment as i64, row.equipment_subbuf_status as i64, 0, 0],
            None,
            3,
            0,
            34,
        );
    }
    if delivery != (row.delivery_subbuf_status == 1) {
        push_rule(
            report,
            D43DecodeRule::D43CoreDeliveryFlagStatus,
            None,
            None,
            at + 36,
            idx,
            [delivery as i64, row.delivery_subbuf_status as i64, 0, 0],
            None,
            3,
            0,
            35,
        );
    }
    if variant
        && (row.equipment_variant_id == 0
            || row.equipment_variant_id > ctx.max_equipment_variant_id)
    {
        push_rule(
            report,
            D43DecodeRule::D43CoreVariantRange,
            None,
            None,
            at + 20,
            idx,
            [
                row.equipment_variant_id as i64,
                ctx.max_equipment_variant_id as i64,
                0,
                0,
            ],
            None,
            3,
            0,
            36,
        );
    }
    if variant && !equipment {
        push_rule(
            report,
            D43DecodeRule::D43CoreVariantWithoutEquipment,
            None,
            None,
            at + 20,
            idx,
            [0; 4],
            None,
            3,
            0,
            37,
        );
    }
    if !variant && row.equipment_variant_id != 0 {
        push_rule(
            report,
            D43DecodeRule::D43CoreVariantWithoutFlag,
            None,
            None,
            at + 20,
            idx,
            [row.equipment_variant_id as i64, 0, 0, 0],
            None,
            3,
            0,
            38,
        );
    }
    if amount && (row.amount_raw <= 0 || row.amount_raw > ctx.max_amount_raw) {
        push_rule(
            report,
            D43DecodeRule::D43CoreAmountRange,
            None,
            None,
            at + 24,
            idx,
            [row.amount_raw, ctx.max_amount_raw, 0, 0],
            None,
            3,
            0,
            39,
        );
    }
    if amount && !delivery {
        push_rule(
            report,
            D43DecodeRule::D43CoreAmountWithoutDelivery,
            None,
            None,
            at + 24,
            idx,
            [0; 4],
            None,
            3,
            0,
            40,
        );
    }
    if !amount && row.amount_raw != 0 {
        push_rule(
            report,
            D43DecodeRule::D43CoreAmountWithoutFlag,
            None,
            None,
            at + 24,
            idx,
            [row.amount_raw, 0, 0, 0],
            None,
            3,
            0,
            41,
        );
    }
}

fn evaluate_detail(
    report: &mut DecodeReport,
    ctx: DecodeContext,
    header: DetailHeader,
    family_ordinal: u16,
) {
    let rows = report.document.detail_rows.clone();
    let core_ids = report
        .document
        .core_rows
        .iter()
        .filter_map(|item| (item.row.contract_id != 0).then_some(item.row.contract_id))
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut computed = [0usize; 9];
    for item in rows {
        let r = item.row;
        let idx = Some(item.index);
        if r.contract_id == 0 {
            push_rule(
                report,
                D43DecodeRule::D43DetailContractIdZero,
                Some(Trailer::Detail),
                Some(TrailerBlock::Rows),
                item.at,
                idx,
                [0; 4],
                None,
                1,
                family_ordinal,
                20,
            );
        }
        if r.contract_id == 0 || r.contract_id > ctx.max_contract_id {
            push_rule(
                report,
                D43DecodeRule::D43DetailContractIdAuthorityRange,
                Some(Trailer::Detail),
                Some(TrailerBlock::Rows),
                item.at,
                idx,
                [r.contract_id as i64, ctx.max_contract_id as i64, 0, 0],
                None,
                1,
                family_ordinal,
                21,
            );
        } else if !core_ids.contains(&r.contract_id) {
            push_rule(
                report,
                D43DecodeRule::D43DetailMissingCoreContract,
                Some(Trailer::Detail),
                Some(TrailerBlock::Rows),
                item.at,
                idx,
                [r.contract_id as i64, 0, 0, 0],
                None,
                3,
                0,
                70,
            );
        }
        if !seen.insert(r.contract_id) {
            push_rule(
                report,
                D43DecodeRule::D43DetailDuplicateContract,
                Some(Trailer::Detail),
                Some(TrailerBlock::Rows),
                item.at,
                idx,
                [r.contract_id as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                60,
            );
        }
        if r.flags == 0 || r.flags & !KNOWN_DETAIL_FLAGS != 0 {
            push_rule(
                report,
                D43DecodeRule::D43DetailInvalidFlags,
                Some(Trailer::Detail),
                Some(TrailerBlock::Rows),
                item.at + 4,
                idx,
                [r.flags as i64, KNOWN_DETAIL_FLAGS as i64, 0, 0],
                None,
                1,
                family_ordinal,
                23,
            );
        }
        if r.reserved != 0 {
            push_rule(
                report,
                D43DecodeRule::D43DetailReserved,
                Some(Trailer::Detail),
                Some(TrailerBlock::Rows),
                item.at + 60,
                idx,
                [r.reserved as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                24,
            );
        }
        detail_i64(
            report,
            &item,
            D43ValueField::D43PriceRaw,
            r.price_raw,
            DetailFlag::PriceValid.is_set(r.flags),
            1,
            ctx.max_amount_raw,
            family_ordinal,
            30,
        );
        detail_i64(
            report,
            &item,
            D43ValueField::D43AmountRaw,
            r.amount_raw,
            DetailFlag::AmountValid.is_set(r.flags),
            1,
            ctx.max_amount_raw,
            family_ordinal,
            32,
        );
        detail_i32(
            report,
            &item,
            D43ValueField::D43SpeedRaw,
            r.speed_raw,
            DetailFlag::SpeedValid.is_set(r.flags),
            0,
            1_000_000_000,
            family_ordinal,
            34,
        );
        if DetailFlag::SpeedValid.is_set(r.flags) && !(0..=100).contains(&r.speed_raw) {
            push_lane_rule(
                report,
                D43DecodeRule::D43DetailPresentValueRange,
                D43LaneRule::D43LaneDetailSpeedRange,
                Trailer::Detail,
                item.at + D43ValueField::D43SpeedRaw.offset(),
                idx,
                [r.speed_raw as i64, 0, 100, 0],
                family_ordinal,
                35,
            );
        }
        detail_i32(
            report,
            &item,
            D43ValueField::D43CompletedRaw,
            r.completed_raw,
            DetailFlag::CompletedValid.is_set(r.flags),
            0,
            1,
            family_ordinal,
            36,
        );
        if DetailFlag::CompletedValid.is_set(r.flags) && r.completed_raw < 0 {
            push_lane_rule(
                report,
                D43DecodeRule::D43DetailPresentValueRange,
                D43LaneRule::D43LaneDetailCompletedRange,
                Trailer::Detail,
                item.at + D43ValueField::D43CompletedRaw.offset(),
                idx,
                [r.completed_raw as i64, 0, i32::MAX as i64, 0],
                family_ordinal,
                37,
            );
        }
        detail_i32(
            report,
            &item,
            D43ValueField::D43Days,
            r.days,
            DetailFlag::DaysValid.is_set(r.flags),
            -1,
            100_000,
            family_ordinal,
            38,
        );
        if DetailFlag::DaysValid.is_set(r.flags) && r.days < 0 {
            push_lane_rule(
                report,
                D43DecodeRule::D43DetailPresentValueRange,
                D43LaneRule::D43LaneDetailDaysRange,
                Trailer::Detail,
                item.at + D43ValueField::D43Days.offset(),
                idx,
                [r.days as i64, 0, i32::MAX as i64, 0],
                family_ordinal,
                39,
            );
        }
        if DetailFlag::DeliveryCountryValid.is_set(r.flags) {
            if r.delivery_country_tag_db_id == 0
                || r.delivery_country_tag_db_id > ctx.max_owner_tag_db_id
            {
                push_rule(
                    report,
                    D43DecodeRule::D43DetailDeliveryCountryRange,
                    Some(Trailer::Detail),
                    Some(TrailerBlock::Rows),
                    item.at + 36,
                    idx,
                    [
                        r.delivery_country_tag_db_id as i64,
                        ctx.max_owner_tag_db_id as i64,
                        0,
                        0,
                    ],
                    None,
                    1,
                    family_ordinal,
                    40,
                );
            }
        } else if r.delivery_country_tag_db_id != 0 {
            push_rule(
                report,
                D43DecodeRule::D43DetailDeliveryCountryWithoutFlag,
                Some(Trailer::Detail),
                Some(TrailerBlock::Rows),
                item.at + 36,
                idx,
                [r.delivery_country_tag_db_id as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                41,
            );
        }
        if DetailFlag::ConvoysValid.is_set(r.flags) {
            if r.convoys_assigned < 0 || r.convoys_total < 0 || r.convoys_assigned > r.convoys_total
            {
                push_rule(
                    report,
                    D43DecodeRule::D43DetailConvoyRange,
                    Some(Trailer::Detail),
                    Some(TrailerBlock::Rows),
                    item.at + 40,
                    idx,
                    [r.convoys_assigned as i64, r.convoys_total as i64, 0, 0],
                    None,
                    1,
                    family_ordinal,
                    42,
                );
            }
        } else if r.convoys_assigned != 0 || r.convoys_total != 0 {
            push_rule(
                report,
                D43DecodeRule::D43DetailConvoyWithoutFlag,
                Some(Trailer::Detail),
                Some(TrailerBlock::Rows),
                item.at + 40,
                idx,
                [r.convoys_assigned as i64, r.convoys_total as i64, 0, 0],
                None,
                1,
                family_ordinal,
                43,
            );
        }
        if DetailFlag::ConvoysValid.is_set(r.flags) {
            if !(0..=1_000_000).contains(&r.convoys_assigned) {
                push_lane_rule(
                    report,
                    D43DecodeRule::D43DetailConvoyRange,
                    D43LaneRule::D43LaneDetailConvoysAssignedRange,
                    Trailer::Detail,
                    item.at + 40,
                    idx,
                    [r.convoys_assigned as i64, 0, 1_000_000, 0],
                    family_ordinal,
                    42,
                );
            }
            if !(0..=1_000_000).contains(&r.convoys_total) {
                push_lane_rule(
                    report,
                    D43DecodeRule::D43DetailConvoyRange,
                    D43LaneRule::D43LaneDetailConvoysTotalRange,
                    Trailer::Detail,
                    item.at + 44,
                    idx,
                    [r.convoys_total as i64, 0, 1_000_000, 0],
                    family_ordinal,
                    43,
                );
            }
        }
        detail_i32(
            report,
            &item,
            D43ValueField::D43EfficiencyRaw,
            r.efficiency_raw,
            DetailFlag::EfficiencyValid.is_set(r.flags),
            0,
            FIXED_SCALE_RAW as i32,
            family_ordinal,
            44,
        );
        if DetailFlag::EfficiencyValid.is_set(r.flags) && r.efficiency_raw < 0 {
            push_lane_rule(
                report,
                D43DecodeRule::D43DetailPresentValueRange,
                D43LaneRule::D43LaneDetailEfficiencyRange,
                Trailer::Detail,
                item.at + D43ValueField::D43EfficiencyRaw.offset(),
                idx,
                [r.efficiency_raw as i64, 0, i32::MAX as i64, 0],
                family_ordinal,
                45,
            );
        }
        detail_i32(
            report,
            &item,
            D43ValueField::D43FactoryCicProgressRaw,
            r.factory_cic_progress_raw,
            DetailFlag::FactoryProgressValid.is_set(r.flags),
            0,
            FIXED_SCALE_RAW as i32,
            family_ordinal,
            46,
        );
        if DetailFlag::FactoryProgressValid.is_set(r.flags) && r.factory_cic_progress_raw < 0 {
            push_lane_rule(
                report,
                D43DecodeRule::D43DetailPresentValueRange,
                D43LaneRule::D43LaneDetailFactoryProgressRange,
                Trailer::Detail,
                item.at + D43ValueField::D43FactoryCicProgressRaw.offset(),
                idx,
                [r.factory_cic_progress_raw as i64, 0, i32::MAX as i64, 0],
                family_ordinal,
                47,
            );
        }
        detail_i32(
            report,
            &item,
            D43ValueField::D43EquipmentTransferProgressRaw,
            r.equipment_transfer_progress_raw,
            DetailFlag::EquipmentProgressValid.is_set(r.flags),
            0,
            FIXED_SCALE_RAW as i32,
            family_ordinal,
            48,
        );
        if DetailFlag::EquipmentProgressValid.is_set(r.flags)
            && r.equipment_transfer_progress_raw < 0
        {
            push_lane_rule(
                report,
                D43DecodeRule::D43DetailPresentValueRange,
                D43LaneRule::D43LaneDetailEquipmentProgressRange,
                Trailer::Detail,
                item.at + D43ValueField::D43EquipmentTransferProgressRaw.offset(),
                idx,
                [
                    r.equipment_transfer_progress_raw as i64,
                    0,
                    i32::MAX as i64,
                    0,
                ],
                family_ordinal,
                49,
            );
        }
        computed[0] += usize::from(DetailFlag::PriceValid.is_set(r.flags));
        computed[1] += usize::from(DetailFlag::SpeedValid.is_set(r.flags));
        computed[2] += usize::from(DetailFlag::CompletedValid.is_set(r.flags));
        computed[3] += usize::from(DetailFlag::DaysValid.is_set(r.flags));
        computed[4] += usize::from(DetailFlag::DeliveryCountryValid.is_set(r.flags));
        computed[5] += usize::from(DetailFlag::ConvoysValid.is_set(r.flags));
        computed[6] += usize::from(DetailFlag::EfficiencyValid.is_set(r.flags));
        computed[7] += usize::from(
            r.flags
                & (DetailFlag::DeliveryCountryValid.bit()
                    | DetailFlag::ConvoysValid.bit()
                    | DetailFlag::EfficiencyValid.bit())
                != 0,
        );
        computed[8] += usize::from(
            r.flags
                & (DetailFlag::FactoryProgressValid.bit()
                    | DetailFlag::EquipmentProgressValid.bit())
                != 0,
        );
    }
    for (ordinal, (expected, actual)) in [
        (header.price_valid_rows as usize, computed[0]),
        (header.speed_valid_rows as usize, computed[1]),
        (header.delivery_route_valid_rows as usize, computed[7]),
        (header.progress_valid_rows as usize, computed[8]),
    ]
    .into_iter()
    .enumerate()
    {
        if expected != actual {
            push_rule(
                report,
                D43DecodeRule::D43DetailHeaderCountMismatch,
                Some(Trailer::Detail),
                None,
                header.sized.at + 16 + ordinal * 4,
                None,
                [ordinal as i64, expected as i64, actual as i64, 0],
                None,
                1,
                family_ordinal,
                70 + ordinal as u16,
            );
        }
    }
}

fn detail_i64(
    report: &mut DecodeReport,
    item: &D43DecodedRow<DetailRow>,
    field: D43ValueField,
    value: i64,
    present: bool,
    min: i64,
    max: i64,
    family_ordinal: u16,
    ordinal: u16,
) {
    let rule = if present && (value < min || value > max) {
        Some(D43DecodeRule::D43DetailPresentValueRange)
    } else if !present && value != 0 {
        Some(D43DecodeRule::D43DetailAbsentValueNonzero)
    } else {
        None
    };
    if let Some(rule) = rule {
        let mut item_finding = finding(
            rule,
            Some(Trailer::Detail),
            Some(TrailerBlock::Rows),
            item.at + field.offset(),
            Some(item.index),
            [value, min, max, 0],
            None,
            1,
            family_ordinal,
            ordinal,
        );
        item_finding.value_field = Some(field);
        retain_finding(report, item_finding);
    }
}

fn detail_i32(
    report: &mut DecodeReport,
    item: &D43DecodedRow<DetailRow>,
    field: D43ValueField,
    value: i32,
    present: bool,
    min: i32,
    max: i32,
    family_ordinal: u16,
    ordinal: u16,
) {
    detail_i64(
        report,
        item,
        field,
        value as i64,
        present,
        min as i64,
        max as i64,
        family_ordinal,
        ordinal,
    )
}

fn evaluate_flow(
    report: &mut DecodeReport,
    ctx: DecodeContext,
    header: FlowTrailer,
    family_ordinal: u16,
) {
    if header.scale_raw != FIXED_SCALE_RAW {
        push_rule(
            report,
            D43DecodeRule::D43FlowHeaderScale,
            Some(Trailer::ResourceFlow),
            None,
            header.at + 20,
            None,
            [header.scale_raw as i64, FIXED_SCALE_RAW as i64, 0, 0],
            None,
            1,
            family_ordinal,
            4,
        );
    }
    match header.version {
        FlowVersion::Legacy => {
            if header.mask_or_count != FLOW_RESOURCE_MASK {
                push_rule(
                    report,
                    D43DecodeRule::D43FlowHeaderResourceMask,
                    Some(Trailer::ResourceFlow),
                    None,
                    header.at + 24,
                    None,
                    [header.mask_or_count as i64, FLOW_RESOURCE_MASK as i64, 0, 0],
                    None,
                    1,
                    family_ordinal,
                    1,
                );
            }
            if header.kind_mask != 0 {
                push_rule(
                    report,
                    D43DecodeRule::D43FlowHeaderKindMask,
                    Some(Trailer::ResourceFlow),
                    None,
                    header.at + 28,
                    None,
                    [
                        header.kind_mask as i64,
                        0,
                        header.row_count as i64,
                        header.country_count_or_exported as i64,
                    ],
                    None,
                    1,
                    family_ordinal,
                    3,
                );
            }
            if header.row_count != header.country_count_or_exported {
                push_rule(
                    report,
                    D43DecodeRule::D43FlowHeaderRowCount,
                    Some(Trailer::ResourceFlow),
                    None,
                    header.at + 12,
                    None,
                    [
                        header.row_count as i64,
                        header.country_count_or_exported as i64,
                        0,
                        header.kind_mask as i64,
                    ],
                    None,
                    1,
                    family_ordinal,
                    3,
                );
            }
        }
        FlowVersion::Fixed7 => {
            if header.mask_or_count != FLOW_RESOURCE_MASK {
                push_rule(
                    report,
                    D43DecodeRule::D43FlowHeaderResourceMask,
                    Some(Trailer::ResourceFlow),
                    None,
                    header.at + 24,
                    None,
                    [header.mask_or_count as i64, FLOW_RESOURCE_MASK as i64, 0, 0],
                    None,
                    1,
                    family_ordinal,
                    1,
                );
            }
            if header.kind_mask != FLOW_KIND_MASK {
                push_rule(
                    report,
                    D43DecodeRule::D43FlowHeaderKindMask,
                    Some(Trailer::ResourceFlow),
                    None,
                    header.at + 28,
                    None,
                    [
                        header.kind_mask as i64,
                        FLOW_KIND_MASK as i64,
                        header.row_count as i64,
                        header.country_count_or_exported as i64,
                    ],
                    None,
                    1,
                    family_ordinal,
                    3,
                );
            }
            if header.row_count
                != header
                    .country_count_or_exported
                    .saturating_mul(FlowKind::ALL.len())
            {
                push_rule(
                    report,
                    D43DecodeRule::D43FlowHeaderRowCount,
                    Some(Trailer::ResourceFlow),
                    None,
                    header.at + 12,
                    None,
                    [
                        header.row_count as i64,
                        header.country_count_or_exported as i64,
                        FlowKind::ALL.len() as i64,
                        header.kind_mask as i64,
                    ],
                    None,
                    1,
                    family_ordinal,
                    3,
                );
            }
        }
        FlowVersion::Variable => {
            if header.resource_count == 0 || header.resource_count > ctx.max_resource_count {}
            if header.kind_mask != FLOW_KIND_MASK {
                push_rule(
                    report,
                    D43DecodeRule::D43FlowHeaderKindMask,
                    Some(Trailer::ResourceFlow),
                    None,
                    header.at + 28,
                    None,
                    [
                        header.kind_mask as i64,
                        FLOW_KIND_MASK as i64,
                        header.row_count as i64,
                        header.country_count_or_exported as i64,
                    ],
                    None,
                    1,
                    family_ordinal,
                    3,
                );
            }
            if header.row_count
                != header
                    .country_count_or_exported
                    .saturating_mul(FlowKind::ALL.len())
            {
                push_rule(
                    report,
                    D43DecodeRule::D43FlowHeaderRowCount,
                    Some(Trailer::ResourceFlow),
                    None,
                    header.at + 12,
                    None,
                    [
                        header.row_count as i64,
                        header.country_count_or_exported as i64,
                        FlowKind::ALL.len() as i64,
                        header.kind_mask as i64,
                    ],
                    None,
                    1,
                    family_ordinal,
                    3,
                );
            }
        }
    }
    let rows = report.document.flow_rows.clone();
    let mut seen = BTreeSet::new();
    let mut countries = BTreeMap::<u32, BTreeSet<u32>>::new();
    let mut lane_countries = BTreeMap::<u32, BTreeSet<u32>>::new();
    let mut exported_rows = 0usize;
    for item in rows {
        let r = item.row;
        let idx = Some(item.index);
        if r.tag_db_id == 0 || r.tag_db_id > ctx.max_owner_tag_db_id {
            push_rule(
                report,
                D43DecodeRule::D43FlowTagRange,
                Some(Trailer::ResourceFlow),
                Some(TrailerBlock::Rows),
                item.at,
                idx,
                [r.tag_db_id as i64, ctx.max_owner_tag_db_id as i64, 0, 0],
                None,
                1,
                family_ordinal,
                20,
            );
        }
        if (header.version == FlowVersion::Legacy && r.flow_kind != FlowKind::Exported.wire())
            || (header.version != FlowVersion::Legacy && FlowKind::from_wire(r.flow_kind).is_none())
        {
            push_rule(
                report,
                D43DecodeRule::D43FlowKind,
                Some(Trailer::ResourceFlow),
                Some(TrailerBlock::Rows),
                item.at + 4,
                idx,
                [r.flow_kind as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                21,
            );
        }
        let expected = if header.version == FlowVersion::Variable {
            header.resource_count
        } else {
            FLOW_RESOURCE_MASK
        };
        if r.mask_or_count != expected {
            push_rule(
                report,
                D43DecodeRule::D43FlowRowMaskOrCount,
                Some(Trailer::ResourceFlow),
                Some(TrailerBlock::Rows),
                item.at + 8,
                idx,
                [r.mask_or_count as i64, expected as i64, 0, 0],
                None,
                1,
                family_ordinal,
                22,
            );
        }
        if r.reserved != 0 {
            push_rule(
                report,
                D43DecodeRule::D43FlowReserved,
                Some(Trailer::ResourceFlow),
                Some(TrailerBlock::Rows),
                item.at + 12,
                idx,
                [r.reserved as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                23,
            );
        }
        if header.version == FlowVersion::Legacy {
            let mut all_zero = true;
            let mut n = item.values.start;
            while n < item.values.end {
                if report.document.flow_values.get(n).copied().unwrap_or(0) != 0 {
                    all_zero = false;
                }
                n += 1;
            }
            if all_zero {
                push_rule(
                    report,
                    D43DecodeRule::D43FlowLegacyValuesAllZero,
                    Some(Trailer::ResourceFlow),
                    Some(TrailerBlock::Rows),
                    item.at,
                    idx,
                    [0; 4],
                    None,
                    1,
                    family_ordinal,
                    24,
                );
            }
        }
        if !seen.insert((r.tag_db_id, r.flow_kind)) {
            push_rule(
                report,
                D43DecodeRule::D43FlowDuplicateCountryKind,
                Some(Trailer::ResourceFlow),
                Some(TrailerBlock::Rows),
                item.at,
                idx,
                [r.tag_db_id as i64, r.flow_kind as i64, 0, 0],
                None,
                1,
                family_ordinal,
                25,
            );
        }
        if r.flow_kind == FlowKind::Exported.wire() {
            exported_rows = exported_rows.saturating_add(1);
        }
        countries
            .entry(r.tag_db_id)
            .or_default()
            .insert(r.flow_kind);
        if header.version != FlowVersion::Legacy && FlowKind::from_wire(r.flow_kind).is_some() {
            lane_countries
                .entry(r.tag_db_id)
                .or_default()
                .insert(r.flow_kind);
        }
    }
    if header.version == FlowVersion::Legacy && header.country_count_or_exported != exported_rows {
        push_lane_rule(
            report,
            D43DecodeRule::D43FlowDecodedCountryCount,
            D43LaneRule::D43LaneFlowLegacyExportedDecodedCount,
            Trailer::ResourceFlow,
            header.at + 16,
            None,
            [
                header.country_count_or_exported as i64,
                exported_rows as i64,
                0,
                0,
            ],
            family_ordinal,
            39,
        );
    }
    if header.version != FlowVersion::Legacy {
        if lane_countries.len() != header.country_count_or_exported {
            push_lane_rule(
                report,
                D43DecodeRule::D43FlowDecodedCountryCount,
                D43LaneRule::D43LaneFlowDecodedCountryCount,
                Trailer::ResourceFlow,
                header.at + 16,
                None,
                [
                    lane_countries.len() as i64,
                    header.country_count_or_exported as i64,
                    0,
                    0,
                ],
                family_ordinal,
                40,
            );
        }
        if countries.len() != header.country_count_or_exported {
            push_rule(
                report,
                D43DecodeRule::D43FlowDecodedCountryCount,
                Some(Trailer::ResourceFlow),
                None,
                header.at + 16,
                None,
                [
                    countries.len() as i64,
                    header.country_count_or_exported as i64,
                    0,
                    0,
                ],
                None,
                1,
                family_ordinal,
                40,
            );
        }
        for (tag, kinds) in countries {
            if kinds.len() != FlowKind::ALL.len() {
                push_rule(
                    report,
                    D43DecodeRule::D43FlowCountryKindsIncomplete,
                    Some(Trailer::ResourceFlow),
                    Some(TrailerBlock::Rows),
                    header.rows_at,
                    None,
                    [
                        tag as i64,
                        kinds.len() as i64,
                        FlowKind::ALL.len() as i64,
                        0,
                    ],
                    None,
                    1,
                    family_ordinal,
                    41,
                );
            }
        }
        for (tag, kinds) in lane_countries {
            if kinds.len() != FlowKind::ALL.len() {
                let mut observed_mask = 0u32;
                for kind in &kinds {
                    if (1..=FlowKind::ALL.len() as u32).contains(kind) {
                        observed_mask |= 1 << (*kind - 1);
                    }
                }
                push_lane_rule(
                    report,
                    D43DecodeRule::D43FlowCountryKindsIncomplete,
                    D43LaneRule::D43LaneFlowCountryKindMask,
                    Trailer::ResourceFlow,
                    header.rows_at,
                    None,
                    [tag as i64, observed_mask as i64, FLOW_KIND_MASK as i64, 0],
                    family_ordinal,
                    42,
                );
            }
        }
    }
}

fn evaluate_factory(
    report: &mut DecodeReport,
    ctx: DecodeContext,
    header: FactoryHeader,
    family_ordinal: u16,
) {
    if header.sized.row_count != header.country_count || header.reserved1 != 0 {
        push_rule(
            report,
            D43DecodeRule::D43FactoryHeaderShape,
            Some(Trailer::FactoryHeader),
            None,
            header.sized.at,
            None,
            [
                header.sized.row_count as i64,
                header.country_count as i64,
                header.reserved1 as i64,
                0,
            ],
            None,
            1,
            family_ordinal,
            1,
        );
    }
    let mut seen = BTreeSet::new();
    let rows = report.document.factory_rows.clone();
    for item in rows {
        if item.row.tag_db_id == 0 || item.row.tag_db_id > ctx.max_owner_tag_db_id {
            push_rule(
                report,
                D43DecodeRule::D43FactoryTagRange,
                Some(Trailer::FactoryHeader),
                Some(TrailerBlock::Rows),
                item.at,
                Some(item.index),
                [
                    item.row.tag_db_id as i64,
                    ctx.max_owner_tag_db_id as i64,
                    0,
                    0,
                ],
                None,
                1,
                family_ordinal,
                20,
            );
        }
        if !seen.insert(item.row.tag_db_id) {
            push_rule(
                report,
                D43DecodeRule::D43FactoryDuplicateTag,
                Some(Trailer::FactoryHeader),
                Some(TrailerBlock::Rows),
                item.at,
                Some(item.index),
                [item.row.tag_db_id as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                21,
            );
        }
    }
}

fn evaluate_resource(
    report: &mut DecodeReport,
    payload: &[u8],
    ctx: DecodeContext,
    header: ResourceRowTrailer,
    family_ordinal: u16,
) {
    if header.display_scale != 1
        || header.row_count
            != header
                .country_count
                .saturating_mul(header.resource_count as usize)
    {
        push_rule(
            report,
            D43DecodeRule::D43ResourceHeaderShape,
            Some(Trailer::ResourceRow),
            None,
            header.at,
            None,
            [
                header.display_scale as i64,
                header.row_count as i64,
                header.country_count as i64,
                header.resource_count as i64,
            ],
            None,
            1,
            family_ordinal,
            1,
        );
    }
    let identities = report.document.resource_identities.clone();
    let mut token_set = BTreeSet::<&[u8]>::new();
    for item in identities {
        if item.declared_len == 0
            || item.declared_len > ctx.max_strategic_resource_token_len
            || item.token.is_none()
        {
            push_rule(
                report,
                D43DecodeRule::D43IdentityLength,
                Some(Trailer::ResourceRow),
                Some(TrailerBlock::Identities),
                item.at,
                Some(item.index),
                [
                    item.declared_len as i64,
                    ctx.max_strategic_resource_token_len as i64,
                    0,
                    0,
                ],
                None,
                1,
                family_ordinal,
                10,
            );
        }
        if let Some(range) = item.padding.clone() {
            if payload
                .get(range)
                .map(|bytes| bytes.iter().any(|b| *b != 0))
                .unwrap_or(true)
            {
                push_rule(
                    report,
                    D43DecodeRule::D43IdentityPadding,
                    Some(Trailer::ResourceRow),
                    Some(TrailerBlock::Identities),
                    item.at,
                    Some(item.index),
                    [0; 4],
                    None,
                    1,
                    family_ordinal,
                    11,
                );
            }
        }
        if let Some(range) = item.token.clone() {
            if let Some(bytes) = payload.get(range) {
                if core::str::from_utf8(bytes).is_err() {
                    push_rule(
                        report,
                        D43DecodeRule::D43IdentityUtf8,
                        Some(Trailer::ResourceRow),
                        Some(TrailerBlock::Identities),
                        item.at,
                        Some(item.index),
                        [0; 4],
                        None,
                        1,
                        family_ordinal,
                        12,
                    );
                }
                if !bytes.iter().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-' | b'.' | b':')
                }) {
                    push_rule(
                        report,
                        D43DecodeRule::D43IdentityCharset,
                        Some(Trailer::ResourceRow),
                        Some(TrailerBlock::Identities),
                        item.at,
                        Some(item.index),
                        [0; 4],
                        None,
                        1,
                        family_ordinal,
                        13,
                    );
                }
                if !token_set.insert(bytes) {
                    push_rule(
                        report,
                        D43DecodeRule::D43IdentityDuplicateToken,
                        Some(Trailer::ResourceRow),
                        Some(TrailerBlock::Identities),
                        item.at,
                        Some(item.index),
                        [0; 4],
                        None,
                        1,
                        family_ordinal,
                        14,
                    );
                }
            }
        }
    }
    let rows = report.document.resource_rows.clone();
    let mut seen = BTreeSet::new();
    let mut descriptor = BTreeMap::<u32, u32>::new();
    let mut oil_slots = BTreeSet::new();
    let mut oil_rows = 0usize;
    let mut countries = BTreeMap::<u32, BTreeSet<u32>>::new();
    for item in rows {
        let r = item.row;
        let idx = Some(item.index);
        if r.tag_db_id == 0 || r.tag_db_id > ctx.max_owner_tag_db_id {
            push_rule(
                report,
                D43DecodeRule::D43ResourceTagRange,
                Some(Trailer::ResourceRow),
                Some(TrailerBlock::Rows),
                item.at,
                idx,
                [r.tag_db_id as i64, ctx.max_owner_tag_db_id as i64, 0, 0],
                None,
                1,
                family_ordinal,
                20,
            );
        }
        if r.resource_index >= header.resource_count {
            push_rule(
                report,
                D43DecodeRule::D43ResourceIndexRange,
                Some(Trailer::ResourceRow),
                Some(TrailerBlock::Rows),
                item.at + 4,
                idx,
                [r.resource_index as i64, header.resource_count as i64, 0, 0],
                None,
                1,
                family_ordinal,
                21,
            );
        }
        if r.visible_order >= header.resource_count {
            push_rule(
                report,
                D43DecodeRule::D43ResourceVisibleOrderRange,
                Some(Trailer::ResourceRow),
                Some(TrailerBlock::Rows),
                item.at + 8,
                idx,
                [r.visible_order as i64, header.resource_count as i64, 0, 0],
                None,
                1,
                family_ordinal,
                22,
            );
        }
        if r.resource_index != r.visible_order {
            push_rule(
                report,
                D43DecodeRule::D43ResourceIndexOrderMismatch,
                Some(Trailer::ResourceRow),
                Some(TrailerBlock::Rows),
                item.at + 4,
                idx,
                [r.resource_index as i64, r.visible_order as i64, 0, 0],
                None,
                1,
                family_ordinal,
                23,
            );
        }
        if r.descriptor_db_index == 0 {
            push_rule(
                report,
                D43DecodeRule::D43ResourceDescriptorZero,
                Some(Trailer::ResourceRow),
                Some(TrailerBlock::Rows),
                item.at + 56,
                idx,
                [0; 4],
                None,
                1,
                family_ordinal,
                24,
            );
        }
        if r.reserved != 0 {
            push_rule(
                report,
                D43DecodeRule::D43ResourceReserved,
                Some(Trailer::ResourceRow),
                Some(TrailerBlock::Rows),
                item.at + 60,
                idx,
                [r.reserved as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                25,
            );
        }
        let known_resource_flags = if header.version.admits_needed_unavailable() {
            RESOURCE_FLAGS_OIL | ResourceFlag::NeededUnavailable.bit()
        } else {
            RESOURCE_FLAGS_OIL
        };
        let unknown_resource_flags = r.flags & !known_resource_flags;
        if unknown_resource_flags != 0 {
            push_lane_rule(
                report,
                D43DecodeRule::D43ResourceNonOilFlags,
                D43LaneRule::D43LaneResourceUnknownFlags,
                Trailer::ResourceRow,
                item.at + 52,
                idx,
                [unknown_resource_flags as i64, 0, 0, 0],
                family_ordinal,
                26,
            );
        }
        let unavailable_flag = ResourceFlag::NeededUnavailable.bit();
        let oil_shaped = matches!(
            NeededType::from_wire(r.needed_type),
            Some(NeededType::CurrentMax) | Some(NeededType::Unavailable)
        ) || r.flags & (RESOURCE_FLAGS_OIL | unavailable_flag) != 0;
        if oil_shaped {
            oil_slots.insert(r.resource_index);
            oil_rows += 1;
            let unavailable =
                r.needed_type == NeededType::Unavailable.wire() || r.flags & unavailable_flag != 0;
            if unavailable {
                if !header.version.admits_needed_unavailable() {
                    push_rule(
                        report,
                        D43DecodeRule::D43ResourceUnavailableVersion,
                        Some(Trailer::ResourceRow),
                        Some(TrailerBlock::Rows),
                        item.at + 12,
                        idx,
                        [header.version.wire() as i64, 0, 0, 0],
                        None,
                        1,
                        family_ordinal,
                        30,
                    );
                }
                if r.needed_type != NeededType::Unavailable.wire()
                    || r.flags != unavailable_flag
                    || r.needed != 0
                    || r.needed_current != 0
                    || r.needed_max != 0
                {
                    push_rule(
                        report,
                        D43DecodeRule::D43ResourceUnavailableShape,
                        Some(Trailer::ResourceRow),
                        Some(TrailerBlock::Rows),
                        item.at + 12,
                        idx,
                        [
                            r.needed_type as i64,
                            r.flags as i64,
                            r.needed as i64,
                            r.needed_current as i64,
                        ],
                        None,
                        1,
                        family_ordinal,
                        31,
                    );
                }
            } else if r.needed_type != NeededType::CurrentMax.wire()
                || r.flags != RESOURCE_FLAGS_OIL
                || r.needed != r.needed_current
            {
                push_rule(
                    report,
                    D43DecodeRule::D43ResourceOilShape,
                    Some(Trailer::ResourceRow),
                    Some(TrailerBlock::Rows),
                    item.at + 12,
                    idx,
                    [
                        r.needed_type as i64,
                        r.flags as i64,
                        r.needed as i64,
                        r.needed_current as i64,
                    ],
                    None,
                    1,
                    family_ordinal,
                    32,
                );
            }
        } else {
            if r.flags != 0 {
                push_rule(
                    report,
                    D43DecodeRule::D43ResourceNonOilFlags,
                    Some(Trailer::ResourceRow),
                    Some(TrailerBlock::Rows),
                    item.at + 52,
                    idx,
                    [r.flags as i64, 0, 0, 0],
                    None,
                    1,
                    family_ordinal,
                    33,
                );
            }
            if r.needed_current != 0 || r.needed_max != 0 {
                push_rule(
                    report,
                    D43DecodeRule::D43ResourceNonOilCurrentMax,
                    Some(Trailer::ResourceRow),
                    Some(TrailerBlock::Rows),
                    item.at + 44,
                    idx,
                    [r.needed_current as i64, r.needed_max as i64, 0, 0],
                    None,
                    1,
                    family_ordinal,
                    34,
                );
            }
            if r.needed != r.balance {
                push_rule(
                    report,
                    D43DecodeRule::D43ResourceNonOilNeededBalance,
                    Some(Trailer::ResourceRow),
                    Some(TrailerBlock::Rows),
                    item.at + 16,
                    idx,
                    [r.needed as i64, r.balance as i64, 0, 0],
                    None,
                    1,
                    family_ordinal,
                    35,
                );
            }
            if !matches!(
                NeededType::from_wire(r.needed_type),
                Some(NeededType::Surplus) | Some(NeededType::Deficit)
            ) {
                push_rule(
                    report,
                    D43DecodeRule::D43ResourceNonOilNeededType,
                    Some(Trailer::ResourceRow),
                    Some(TrailerBlock::Rows),
                    item.at + 12,
                    idx,
                    [r.needed_type as i64, 0, 0, 0],
                    None,
                    1,
                    family_ordinal,
                    36,
                );
            }
            if (r.balance < 0) != (r.needed_type == NeededType::Deficit.wire()) {
                push_rule(
                    report,
                    D43DecodeRule::D43ResourceNonOilSign,
                    Some(Trailer::ResourceRow),
                    Some(TrailerBlock::Rows),
                    item.at + 12,
                    idx,
                    [r.balance as i64, r.needed_type as i64, 0, 0],
                    None,
                    1,
                    family_ordinal,
                    37,
                );
            }
        }
        if !seen.insert((r.tag_db_id, r.resource_index)) {
            push_rule(
                report,
                D43DecodeRule::D43ResourceDuplicateCountrySlot,
                Some(Trailer::ResourceRow),
                Some(TrailerBlock::Rows),
                item.at,
                idx,
                [r.tag_db_id as i64, r.resource_index as i64, 0, 0],
                None,
                1,
                family_ordinal,
                38,
            );
        }
        if let Some(old) = descriptor.insert(r.resource_index, r.descriptor_db_index) {
            if old != r.descriptor_db_index {
                push_rule(
                    report,
                    D43DecodeRule::D43ResourceDescriptorChanged,
                    Some(Trailer::ResourceRow),
                    Some(TrailerBlock::Rows),
                    item.at + 56,
                    idx,
                    [
                        r.resource_index as i64,
                        old as i64,
                        r.descriptor_db_index as i64,
                        0,
                    ],
                    None,
                    1,
                    family_ordinal,
                    39,
                );
            }
        }
        countries
            .entry(r.tag_db_id)
            .or_default()
            .insert(r.visible_order);
    }
    if header.country_count > 0 {
        if oil_slots.len() != 1 {
            push_rule(
                report,
                D43DecodeRule::D43ResourceOilSlotCount,
                Some(Trailer::ResourceRow),
                None,
                header.at,
                None,
                [oil_slots.len() as i64, 1, 0, 0],
                None,
                1,
                family_ordinal,
                50,
            );
        }
        if oil_rows != header.country_count {
            push_rule(
                report,
                D43DecodeRule::D43ResourceOilRowCount,
                Some(Trailer::ResourceRow),
                None,
                header.at,
                None,
                [oil_rows as i64, header.country_count as i64, 0, 0],
                None,
                1,
                family_ordinal,
                51,
            );
        }
    } else if oil_rows != 0 {
        push_rule(
            report,
            D43DecodeRule::D43ResourceOilWithoutCountry,
            Some(Trailer::ResourceRow),
            None,
            header.at,
            None,
            [oil_rows as i64, 0, 0, 0],
            None,
            1,
            family_ordinal,
            52,
        );
    }
    let distinct_descriptors = descriptor.values().copied().collect::<BTreeSet<_>>().len();
    if header.country_count > 0
        && (descriptor.len() != header.resource_count as usize
            || distinct_descriptors != header.resource_count as usize)
    {
        push_rule(
            report,
            D43DecodeRule::D43ResourceDescriptorCoverage,
            Some(Trailer::ResourceRow),
            None,
            header.at,
            None,
            [
                descriptor.len() as i64,
                distinct_descriptors as i64,
                header.resource_count as i64,
                0,
            ],
            None,
            1,
            family_ordinal,
            53,
        );
    }
    if countries.len() != header.country_count {
        push_rule(
            report,
            D43DecodeRule::D43ResourceCountryCount,
            Some(Trailer::ResourceRow),
            None,
            header.at + 16,
            None,
            [countries.len() as i64, header.country_count as i64, 0, 0],
            None,
            1,
            family_ordinal,
            54,
        );
    }
    for (tag, slots) in countries {
        let dense = slots.len() == header.resource_count as usize
            && slots
                .iter()
                .copied()
                .enumerate()
                .all(|(i, v)| v == i as u32);
        if !dense {
            push_rule(
                report,
                D43DecodeRule::D43ResourceCountryCoverage,
                Some(Trailer::ResourceRow),
                Some(TrailerBlock::Rows),
                header.rows_at,
                None,
                [
                    tag as i64,
                    slots.len() as i64,
                    header.resource_count as i64,
                    0,
                ],
                None,
                1,
                family_ordinal,
                55,
            );
        }
    }
}

fn evaluate_deals(
    report: &mut DecodeReport,
    ctx: DecodeContext,
    header: DealTrailer,
    family_ordinal: u16,
) {
    if header.country_count != header.witness_count
        || header.incomplete_count > header.witness_count
        || header.scale_raw != FIXED_SCALE_RAW
    {
        push_rule(
            report,
            D43DecodeRule::D43DealHeaderShape,
            Some(Trailer::ClassicDeal),
            None,
            header.at,
            None,
            [
                header.country_count as i64,
                header.witness_count as i64,
                header.incomplete_count as i64,
                header.scale_raw as i64,
            ],
            None,
            1,
            family_ordinal,
            1,
        );
    }
    let witnesses = report.document.deal_witnesses.clone();
    let mut witnessed = BTreeSet::new();
    for item in witnesses {
        if item.row == 0 || item.row > ctx.max_owner_tag_db_id {
            push_rule(
                report,
                D43DecodeRule::D43DealWitnessRange,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Witnesses),
                item.at,
                Some(item.index),
                [item.row as i64, ctx.max_owner_tag_db_id as i64, 0, 0],
                None,
                1,
                family_ordinal,
                10,
            );
        }
        if !witnessed.insert(item.row) {
            push_rule(
                report,
                D43DecodeRule::D43DealWitnessDuplicate,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Witnesses),
                item.at,
                Some(item.index),
                [item.row as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                11,
            );
        }
    }
    let incomplete = report.document.deal_incomplete_owners.clone();
    let mut previous = None;
    for item in incomplete {
        if !witnessed.contains(&item.row) {
            push_rule(
                report,
                D43DecodeRule::D43DealIncompleteNotSubset,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::IncompleteOwners),
                item.at,
                Some(item.index),
                [item.row as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                12,
            );
        }
        if previous.map(|v| item.row <= v).unwrap_or(false) {
            push_rule(
                report,
                D43DecodeRule::D43DealIncompleteNotAscending,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::IncompleteOwners),
                item.at,
                Some(item.index),
                [previous.unwrap_or(0) as i64, item.row as i64, 0, 0],
                None,
                1,
                family_ordinal,
                13,
            );
        }
        previous = Some(item.row);
    }
    let rows = report.document.deal_rows.clone();
    let mut visible = BTreeSet::new();
    let mut halves = BTreeMap::<(u32, u32, u32, u32), Vec<DealRow>>::new();
    let mut owner_orders = BTreeMap::<u32, BTreeSet<u32>>::new();
    for item in rows {
        let r = item.row;
        let idx = Some(item.index);
        if !witnessed.contains(&r.owner_tag_db_id) {
            push_rule(
                report,
                D43DecodeRule::D43DealOwnerNotWitnessed,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                item.at,
                idx,
                [r.owner_tag_db_id as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                20,
            );
        }
        if !witnessed.contains(&r.counterpart_tag_db_id) {
            push_rule(
                report,
                D43DecodeRule::D43DealCounterpartNotWitnessed,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                item.at + 4,
                idx,
                [r.counterpart_tag_db_id as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                21,
            );
        }
        if r.owner_tag_db_id == r.counterpart_tag_db_id {
            push_rule(
                report,
                D43DecodeRule::D43DealSameParty,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                item.at,
                idx,
                [r.owner_tag_db_id as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                22,
            );
        }
        if r.resource_index >= ctx.max_resource_count {
            push_rule(
                report,
                D43DecodeRule::D43DealResourceIndexRange,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                item.at + 8,
                idx,
                [r.resource_index as i64, ctx.max_resource_count as i64, 0, 0],
                None,
                1,
                family_ordinal,
                23,
            );
        }
        if DealDirection::from_wire(r.direction).is_none() {
            push_rule(
                report,
                D43DecodeRule::D43DealDirection,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                item.at + 16,
                idx,
                [r.direction as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                24,
            );
        }
        if DealRoute::from_wire(r.route_kind).is_none() {
            push_rule(
                report,
                D43DecodeRule::D43DealRoute,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                item.at + 20,
                idx,
                [r.route_kind as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                25,
            );
        }
        if r.flags != DEAL_FLAG_EXACT {
            push_rule(
                report,
                D43DecodeRule::D43DealFlags,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                item.at + 24,
                idx,
                [r.flags as i64, DEAL_FLAG_EXACT as i64, 0, 0],
                None,
                1,
                family_ordinal,
                26,
            );
        }
        if !header.version.carries_route_status() && r.route_status_or_reserved != 0 {
            push_rule(
                report,
                D43DecodeRule::D43DealLegacyReservedStatus,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                item.at + 28,
                idx,
                [r.route_status_or_reserved as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                27,
            );
        }
        if header.version.carries_route_status()
            && r.route_status_or_reserved & !KNOWN_DEAL_STATUS != 0
        {
            push_rule(
                report,
                D43DecodeRule::D43DealStatusBits,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                item.at + 28,
                idx,
                [
                    r.route_status_or_reserved as i64,
                    KNOWN_DEAL_STATUS as i64,
                    0,
                    0,
                ],
                None,
                1,
                family_ordinal,
                28,
            );
        }
        if let Some(packed) = r.packed_identity {
            if packed_identity_reserved(packed) != 0 {
                push_rule(
                    report,
                    D43DecodeRule::D43DealPackedReserved,
                    Some(Trailer::ClassicDeal),
                    Some(TrailerBlock::Rows),
                    item.at + 32,
                    idx,
                    [packed_identity_reserved(packed) as i64, 0, 0, 0],
                    None,
                    1,
                    family_ordinal,
                    29,
                );
            }
            if packed_identity_resource(packed) != r.resource_index {
                push_rule(
                    report,
                    D43DecodeRule::D43DealPackedResourceEcho,
                    Some(Trailer::ClassicDeal),
                    Some(TrailerBlock::Rows),
                    item.at + 32,
                    idx,
                    [
                        packed_identity_resource(packed) as i64,
                        r.resource_index as i64,
                        0,
                        0,
                    ],
                    None,
                    1,
                    family_ordinal,
                    30,
                );
            }
        }
        if r.identity_ordinal == 0 || r.identity_ordinal > ctx.max_deal_identity_ordinal {
            push_rule(
                report,
                D43DecodeRule::D43DealIdentityOrdinal,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                item.at + 32,
                idx,
                [
                    r.identity_ordinal as i64,
                    ctx.max_deal_identity_ordinal as i64,
                    0,
                    0,
                ],
                None,
                1,
                family_ordinal,
                31,
            );
        }
        let expected = (
            if r.direction == DealDirection::Import.wire() {
                r.counterpart_tag_db_id
            } else {
                r.owner_tag_db_id
            },
            if r.direction == DealDirection::Import.wire() {
                r.owner_tag_db_id
            } else {
                r.counterpart_tag_db_id
            },
        );
        if (r.identity_exporter_tag_db_id, r.identity_importer_tag_db_id) != expected {
            push_rule(
                report,
                D43DecodeRule::D43DealIdentityParties,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                item.at + 32,
                idx,
                [
                    r.identity_exporter_tag_db_id as i64,
                    r.identity_importer_tag_db_id as i64,
                    expected.0 as i64,
                    expected.1 as i64,
                ],
                None,
                1,
                family_ordinal,
                32,
            );
        }
        if r.requested_raw % (FIXED_SCALE_RAW as i64) != 0 {
            push_rule(
                report,
                D43DecodeRule::D43DealRequestedScale,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                item.at + 40,
                idx,
                [r.requested_raw, FIXED_SCALE_RAW as i64, 0, 0],
                None,
                1,
                family_ordinal,
                33,
            );
        }
        if !(0..=FIXED_SCALE_RAW as i64).contains(&r.efficiency_raw) {
            push_rule(
                report,
                D43DecodeRule::D43DealEfficiencyRange,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                item.at + 72,
                idx,
                [r.efficiency_raw, 0, FIXED_SCALE_RAW as i64, 0],
                None,
                1,
                family_ordinal,
                34,
            );
        }
        if r.reserved_ordinal_word.unwrap_or(0) != 0 {
            push_rule(
                report,
                D43DecodeRule::D43DealReservedOrdinal,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                item.at + 92,
                idx,
                [r.reserved_ordinal_word.unwrap_or(0) as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                35,
            );
        }
        if !visible.insert((r.owner_tag_db_id, r.visible_order)) {
            push_rule(
                report,
                D43DecodeRule::D43DealDuplicateVisibleOrder,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                item.at,
                idx,
                [r.owner_tag_db_id as i64, r.visible_order as i64, 0, 0],
                None,
                1,
                family_ordinal,
                36,
            );
        }
        halves
            .entry((
                r.identity_exporter_tag_db_id,
                r.identity_importer_tag_db_id,
                r.resource_index,
                r.identity_ordinal,
            ))
            .or_default()
            .push(r);
        owner_orders
            .entry(r.owner_tag_db_id)
            .or_default()
            .insert(r.visible_order);
    }
    for (identity_order, ((exporter, importer, resource, ordinal), pair)) in
        halves.into_iter().enumerate()
    {
        let identity = D43DealIdentityOperands {
            exporter_tag_db_id: exporter,
            importer_tag_db_id: importer,
            resource_index: resource,
            ordinal,
        };
        if pair.len() != 2 {
            push_deal_rule(
                report,
                D43DecodeRule::D43DealHalfCount,
                header.rows_at,
                identity,
                identity_order,
                pair.len(),
                family_ordinal,
                50,
            );
        } else if let [first, second] = pair.as_slice() {
            if !deal_halves_match(*first, *second) {
                push_deal_rule(
                    report,
                    D43DecodeRule::D43DealHalfMismatch,
                    header.rows_at,
                    identity,
                    identity_order,
                    pair.len(),
                    family_ordinal,
                    51,
                );
            }
        }
    }
    for (owner, orders) in owner_orders {
        if !orders
            .iter()
            .copied()
            .enumerate()
            .all(|(i, v)| v == i as u32)
        {
            push_rule(
                report,
                D43DecodeRule::D43DealVisibleOrderCoverage,
                Some(Trailer::ClassicDeal),
                Some(TrailerBlock::Rows),
                header.rows_at,
                None,
                [owner as i64, orders.len() as i64, 0, 0],
                None,
                1,
                family_ordinal,
                52,
            );
        }
    }
}

fn deal_halves_match(a: DealRow, b: DealRow) -> bool {
    a.owner_tag_db_id == b.counterpart_tag_db_id
        && a.counterpart_tag_db_id == b.owner_tag_db_id
        && a.resource_index == b.resource_index
        && a.direction != b.direction
        && a.route_kind == b.route_kind
        && a.route_status_or_reserved == b.route_status_or_reserved
        && a.requested_raw == b.requested_raw
        && a.delivered_raw == b.delivered_raw
        && a.available_export_raw == b.available_export_raw
        && a.factory_cost_raw == b.factory_cost_raw
        && a.efficiency_raw == b.efficiency_raw
        && a.civilian_factories == b.civilian_factories
        && a.convoys_assigned == b.convoys_assigned
        && a.convoys_required == b.convoys_required
}

fn evaluate_activation(
    report: &mut DecodeReport,
    ctx: DecodeContext,
    h: ActivationTrailer,
    family_ordinal: u16,
) {
    let active = h.flags & ACTIVATION_FLAG_ACTIVE != 0;
    if h.flags & !KNOWN_ACTIVATION_FLAGS != 0 {
        push_rule(
            report,
            D43DecodeRule::D43ActivationFlags,
            Some(Trailer::Activation),
            None,
            h.at + 8,
            None,
            [h.flags as i64, KNOWN_ACTIVATION_FLAGS as i64, 0, 0],
            None,
            1,
            family_ordinal,
            1,
        );
    }
    if h.flags & ACTIVATION_FLAG_ROOT_EXACT == 0 {
        push_rule(
            report,
            D43DecodeRule::D43ActivationRootExact,
            Some(Trailer::Activation),
            None,
            h.at + 8,
            None,
            [h.flags as i64, ACTIVATION_FLAG_ROOT_EXACT as i64, 0, 0],
            None,
            1,
            family_ordinal,
            2,
        );
    }
    if active != (h.country_tag_db_id != 0) {
        push_rule(
            report,
            D43DecodeRule::D43ActivationActiveCountry,
            Some(Trailer::Activation),
            None,
            h.at + 12,
            None,
            [active as i64, h.country_tag_db_id as i64, 0, 0],
            None,
            1,
            family_ordinal,
            3,
        );
    }
    if active && h.observation_sequence == 0 {
        push_rule(
            report,
            D43DecodeRule::D43ActivationSequence,
            Some(Trailer::Activation),
            None,
            h.at + 16,
            None,
            [0; 4],
            None,
            1,
            family_ordinal,
            4,
        );
    }
    if h.country_tag_db_id > ctx.max_owner_tag_db_id {
        push_rule(
            report,
            D43DecodeRule::D43ActivationCountryRange,
            Some(Trailer::Activation),
            None,
            h.at + 12,
            None,
            [
                h.country_tag_db_id as i64,
                ctx.max_owner_tag_db_id as i64,
                0,
                0,
            ],
            None,
            1,
            family_ordinal,
            5,
        );
    }
    if h.root_kind != ACTIVATION_ROOT_COUNTRYTRADEVIEW {
        push_rule(
            report,
            D43DecodeRule::D43ActivationRootKind,
            Some(Trailer::Activation),
            None,
            h.at + 24,
            None,
            [
                h.root_kind as i64,
                ACTIVATION_ROOT_COUNTRYTRADEVIEW as i64,
                0,
                0,
            ],
            None,
            1,
            family_ordinal,
            6,
        );
    }
    if h.open_rva != ACTIVATION_OPEN_RVA {
        push_rule(
            report,
            D43DecodeRule::D43ActivationOpenRva,
            Some(Trailer::Activation),
            None,
            h.at + 28,
            None,
            [h.open_rva as i64, ACTIVATION_OPEN_RVA as i64, 0, 0],
            None,
            1,
            family_ordinal,
            7,
        );
    }
}

fn evaluate_stockpile(
    report: &mut DecodeReport,
    ctx: DecodeContext,
    h: StockpileTrailer,
    family_ordinal: u16,
) {
    let expected = h
        .header_len
        .saturating_add(h.country_count.saturating_mul(h.country_row_len))
        .saturating_add(h.item_count.saturating_mul(h.item_row_len));
    if h.declared_segment_len != expected {
        push_rule(
            report,
            D43DecodeRule::D43StockpileSegmentLength,
            Some(Trailer::Stockpile),
            None,
            h.at + 12,
            None,
            [h.declared_segment_len as i64, expected as i64, 0, 0],
            None,
            1,
            family_ordinal,
            1,
        );
    }
    if h.country_count == 0 {
        push_rule(
            report,
            D43DecodeRule::D43StockpileCountryCount,
            Some(Trailer::Stockpile),
            None,
            h.at + 16,
            None,
            [0; 4],
            None,
            1,
            family_ordinal,
            2,
        );
    }
    if h.country_count != h.expected_country_count {
        push_rule(
            report,
            D43DecodeRule::D43StockpileExpectedCountryCount,
            Some(Trailer::Stockpile),
            None,
            h.at + 24,
            None,
            [
                h.country_count as i64,
                h.expected_country_count as i64,
                0,
                0,
            ],
            None,
            1,
            family_ordinal,
            3,
        );
    }
    if h.scale_raw != FIXED_SCALE_RAW {
        push_rule(
            report,
            D43DecodeRule::D43StockpileScale,
            Some(Trailer::Stockpile),
            None,
            h.at + 28,
            None,
            [h.scale_raw as i64, FIXED_SCALE_RAW as i64, 0, 0],
            None,
            1,
            family_ordinal,
            4,
        );
    }
    if h.flags != STOCKPILE_GLOBAL_FLAGS {
        push_rule(
            report,
            D43DecodeRule::D43StockpileGlobalFlags,
            Some(Trailer::Stockpile),
            None,
            h.at + 32,
            None,
            [h.flags as i64, STOCKPILE_GLOBAL_FLAGS as i64, 0, 0],
            None,
            1,
            family_ordinal,
            5,
        );
    }
    if h.reserved != 0 {
        push_rule(
            report,
            D43DecodeRule::D43StockpileReserved,
            Some(Trailer::Stockpile),
            None,
            h.at + 36,
            None,
            [h.reserved as i64, 0, 0, 0],
            None,
            1,
            family_ordinal,
            6,
        );
    }
    let countries = report.document.stockpile_countries.clone();
    let items = report.document.stockpile_items.clone();
    let mut seen_tags = BTreeSet::new();
    let mut expected_first = 0usize;
    for country in countries {
        let r = country.row;
        let idx = Some(country.index);
        let end = r.item_first.checked_add(r.item_count);
        if r.tag_db_id == 0 || r.tag_db_id > ctx.max_owner_tag_db_id {
            push_rule(
                report,
                D43DecodeRule::D43StockpileCountryTagRange,
                Some(Trailer::Stockpile),
                Some(TrailerBlock::Countries),
                country.at,
                idx,
                [r.tag_db_id as i64, ctx.max_owner_tag_db_id as i64, 0, 0],
                None,
                1,
                family_ordinal,
                20,
            );
        }
        if !seen_tags.insert(r.tag_db_id) {
            push_rule(
                report,
                D43DecodeRule::D43StockpileCountryDuplicate,
                Some(Trailer::Stockpile),
                Some(TrailerBlock::Countries),
                country.at,
                idx,
                [r.tag_db_id as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                21,
            );
        }
        if r.item_first != expected_first {
            push_rule(
                report,
                D43DecodeRule::D43StockpileCountryFirst,
                Some(Trailer::Stockpile),
                Some(TrailerBlock::Countries),
                country.at + 4,
                idx,
                [r.item_first as i64, expected_first as i64, 0, 0],
                None,
                1,
                family_ordinal,
                22,
            );
        }
        if end.map(|v| v > h.item_count).unwrap_or(true) {
            push_rule(
                report,
                D43DecodeRule::D43StockpileCountryRange,
                Some(Trailer::Stockpile),
                Some(TrailerBlock::Countries),
                country.at + 4,
                idx,
                [
                    r.item_first as i64,
                    r.item_count as i64,
                    h.item_count as i64,
                    0,
                ],
                None,
                1,
                family_ordinal,
                23,
            );
        }
        if r.flags != STOCKPILE_COUNTRY_COMPLETE {
            push_rule(
                report,
                D43DecodeRule::D43StockpileCountryFlags,
                Some(Trailer::Stockpile),
                Some(TrailerBlock::Countries),
                country.at + 12,
                idx,
                [r.flags as i64, STOCKPILE_COUNTRY_COMPLETE as i64, 0, 0],
                None,
                1,
                family_ordinal,
                24,
            );
        }
        if r.reserved != 0 {
            push_rule(
                report,
                D43DecodeRule::D43StockpileCountryReserved,
                Some(Trailer::Stockpile),
                Some(TrailerBlock::Countries),
                country.at + 16,
                idx,
                [r.reserved as i64, 0, 0, 0],
                None,
                1,
                family_ordinal,
                25,
            );
        }
        let mut variants = BTreeSet::new();
        let mut ordinal = 0usize;
        while ordinal < r.item_count {
            let item_index = match r.item_first.checked_add(ordinal) {
                Some(v) => v,
                None => break,
            };
            if let Some(item) = items.get(item_index) {
                let row = item.row;
                let item_idx = Some(item.index);
                if row.equipment_variant_id == 0
                    || row.equipment_variant_id > ctx.max_equipment_variant_id
                {
                    push_rule(
                        report,
                        D43DecodeRule::D43StockpileVariantRange,
                        Some(Trailer::Stockpile),
                        Some(TrailerBlock::Items),
                        item.at,
                        item_idx,
                        [
                            row.equipment_variant_id as i64,
                            ctx.max_equipment_variant_id as i64,
                            0,
                            0,
                        ],
                        None,
                        1,
                        family_ordinal,
                        30,
                    );
                }
                if row.variant_type != STOCKPILE_EQUIPMENT_TYPE {
                    push_rule(
                        report,
                        D43DecodeRule::D43StockpileEquipmentType,
                        Some(Trailer::Stockpile),
                        Some(TrailerBlock::Items),
                        item.at + 4,
                        item_idx,
                        [
                            row.variant_type as i64,
                            STOCKPILE_EQUIPMENT_TYPE as i64,
                            0,
                            0,
                        ],
                        None,
                        1,
                        family_ordinal,
                        31,
                    );
                }
                if row.ordinal != ordinal as u32 {
                    push_rule(
                        report,
                        D43DecodeRule::D43StockpileOrdinal,
                        Some(Trailer::Stockpile),
                        Some(TrailerBlock::Items),
                        item.at + 8,
                        item_idx,
                        [row.ordinal as i64, ordinal as i64, 0, 0],
                        None,
                        1,
                        family_ordinal,
                        32,
                    );
                }
                if row.reserved != 0 {
                    push_rule(
                        report,
                        D43DecodeRule::D43StockpileItemReserved,
                        Some(Trailer::Stockpile),
                        Some(TrailerBlock::Items),
                        item.at + 12,
                        item_idx,
                        [row.reserved as i64, 0, 0, 0],
                        None,
                        1,
                        family_ordinal,
                        33,
                    );
                }
                if row.amount_raw < 0 || row.amount_raw > ctx.max_amount_raw {
                    push_rule(
                        report,
                        D43DecodeRule::D43StockpileAmountRange,
                        Some(Trailer::Stockpile),
                        Some(TrailerBlock::Items),
                        item.at + 16,
                        item_idx,
                        [row.amount_raw, ctx.max_amount_raw, 0, 0],
                        None,
                        1,
                        family_ordinal,
                        34,
                    );
                }
                if !variants.insert((row.equipment_variant_id, row.variant_type)) {
                    push_rule(
                        report,
                        D43DecodeRule::D43StockpileDuplicateVariant,
                        Some(Trailer::Stockpile),
                        Some(TrailerBlock::Items),
                        item.at,
                        item_idx,
                        [
                            row.equipment_variant_id as i64,
                            row.variant_type as i64,
                            0,
                            0,
                        ],
                        None,
                        1,
                        family_ordinal,
                        35,
                    );
                }
            }
            ordinal += 1;
        }
        if let Some(end) = end {
            expected_first = end;
        }
    }
    if expected_first != h.item_count {
        let mut item = finding(
            D43DecodeRule::D43StockpileCountryCoverage,
            Some(Trailer::Stockpile),
            Some(TrailerBlock::Countries),
            h.countries_at,
            None,
            [expected_first as i64, h.item_count as i64, 0, 0],
            None,
            1,
            family_ordinal,
            40,
        );
        item.policy_order.block_ordinal = 3;
        item.policy_order.scope_ordinal = 0;
        retain_finding(report, item);
    }
}

fn evaluate_residual(report: &mut DecodeReport, h: ResidualHeader, family_ordinal: u16) {
    let sum = (h.source_blocked_rows as usize)
        .saturating_add(h.schema_blocked_rows as usize)
        .saturating_add(h.owned_elsewhere_rows as usize)
        .saturating_add(h.shared_patch_blocked_rows as usize);
    if sum != h.sized.row_count {
        push_lane_rule(
            report,
            D43DecodeRule::D43ResidualHeaderCounts,
            D43LaneRule::D43LaneResidualHeaderCountSum,
            Trailer::Residual,
            h.sized.at,
            None,
            [sum as i64, h.sized.row_count as i64, 0, 0],
            family_ordinal,
            1,
        );
    }
    let rows = report.document.residual_rows.clone();
    let mut seen = BTreeSet::new();
    let mut computed = [0u32; 4];
    for item in rows {
        let r = item.row;
        let row = Some(item.index);
        let mut emit = |lane_rule, operand, ordinal| {
            push_lane_rule(
                report,
                D43DecodeRule::D43ResidualReserved,
                lane_rule,
                Trailer::Residual,
                item.at,
                row,
                [operand, 0, 0, 0],
                family_ordinal,
                ordinal,
            );
        };
        if r.gap_id == 0 {
            emit(D43LaneRule::D43LaneResidualGapIdZero, 0, 20);
        }
        if !(1..=5).contains(&r.status) {
            emit(
                D43LaneRule::D43LaneResidualStatusUnknown,
                r.status as i64,
                21,
            );
        }
        if r.evidence_mask == 0 {
            emit(D43LaneRule::D43LaneResidualEvidenceEmpty, 0, 22);
        }
        if r.current_support > 4 {
            emit(
                D43LaneRule::D43LaneResidualCurrentSupportUnknown,
                r.current_support as i64,
                23,
            );
        }
        if !matches!(r.current_domain_id, 0 | 11 | 17 | 43 | 53) {
            emit(
                D43LaneRule::D43LaneResidualCurrentDomainUnknown,
                r.current_domain_id as i64,
                24,
            );
        }
        if !((1..=7).contains(&r.proposed_record_kind) || r.proposed_record_kind == 255) {
            emit(
                D43LaneRule::D43LaneResidualProposedRecordKindUnknown,
                r.proposed_record_kind as i64,
                25,
            );
        }
        if r.current_row_len > 512 {
            emit(
                D43LaneRule::D43LaneResidualCurrentRowLenRange,
                r.current_row_len as i64,
                26,
            );
        }
        if r.source_ref_id == 0 || r.source_ref_id != r.gap_id {
            emit(
                D43LaneRule::D43LaneResidualSourceRefInvalid,
                r.source_ref_id as i64,
                27,
            );
        }
        if !(1..=4).contains(&r.owner_boundary) {
            emit(
                D43LaneRule::D43LaneResidualOwnerBoundaryUnknown,
                r.owner_boundary as i64,
                28,
            );
        }
        if !(1..=5).contains(&r.root_status) {
            emit(
                D43LaneRule::D43LaneResidualRootStatusUnknown,
                r.root_status as i64,
                29,
            );
        }
        if !(1..=4).contains(&r.proof_status) {
            emit(
                D43LaneRule::D43LaneResidualProofStatusUnknown,
                r.proof_status as i64,
                30,
            );
        }
        if r.reserved != 0 {
            emit(D43LaneRule::D43LaneResidualReserved, r.reserved as i64, 31);
        }
        if r.gap_id != 0 && !seen.insert(r.gap_id) {
            emit(
                D43LaneRule::D43LaneResidualDuplicateGapId,
                r.gap_id as i64,
                32,
            );
        }
        match r.status {
            3 => computed[0] = computed[0].saturating_add(1),
            2 => computed[1] = computed[1].saturating_add(1),
            4 => computed[2] = computed[2].saturating_add(1),
            5 => computed[3] = computed[3].saturating_add(1),
            _ => {}
        }
    }
    let expected = [
        h.source_blocked_rows,
        h.schema_blocked_rows,
        h.owned_elsewhere_rows,
        h.shared_patch_blocked_rows,
    ];
    let rules = [
        D43LaneRule::D43LaneResidualSourceBlockedCount,
        D43LaneRule::D43LaneResidualSchemaBlockedCount,
        D43LaneRule::D43LaneResidualOwnedElsewhereCount,
        D43LaneRule::D43LaneResidualSharedPatchBlockedCount,
    ];
    for index in 0..4 {
        if expected[index] != computed[index] {
            push_lane_rule(
                report,
                D43DecodeRule::D43ResidualHeaderCounts,
                rules[index],
                Trailer::Residual,
                h.sized.at,
                None,
                [expected[index] as i64, computed[index] as i64, 0, 0],
                family_ordinal,
                40 + index as u16,
            );
        }
    }
}

fn evaluate_lend_lease(
    report: &mut DecodeReport,
    ctx: DecodeContext,
    h: LendLeaseHeader,
    family_ordinal: u16,
) {
    if (h.outgoing_rows as usize).saturating_add(h.incoming_rows as usize) != h.sized.row_count {
        push_lane_rule(
            report,
            D43DecodeRule::D43LendLeaseHeaderCounts,
            D43LaneRule::D43LaneLendLeaseDirectionCountSum,
            Trailer::LendLease,
            h.sized.at,
            None,
            [
                h.outgoing_rows as i64,
                h.incoming_rows as i64,
                h.sized.row_count as i64,
                0,
            ],
            family_ordinal,
            1,
        );
    }
    let rows = report.document.lend_lease_rows.clone();
    let mut seen = BTreeSet::new();
    let mut per_contract = BTreeMap::<u32, (u32, u32)>::new();
    let mut computed = [0u32; 4];
    for item in rows {
        let r = item.row;
        let amount = LendLeaseFlag::AmountValid.bit() & r.flags != 0;
        let price = LendLeaseFlag::PriceValid.bit() & r.flags != 0;
        let mut emit = |lane_rule, operands, ordinal| {
            push_lane_rule(
                report,
                D43DecodeRule::D43LendLeaseRowShape,
                lane_rule,
                Trailer::LendLease,
                item.at,
                Some(item.index),
                operands,
                family_ordinal,
                ordinal,
            );
        };
        if r.contract_id == 0 || r.contract_id > ctx.max_contract_id {
            emit(
                D43LaneRule::D43LaneLendLeaseContractIdRange,
                [r.contract_id as i64, 0, 0, 0],
                20,
            );
        }
        if r.perspective_tag_db_id == 0 || r.perspective_tag_db_id > ctx.max_owner_tag_db_id {
            emit(
                D43LaneRule::D43LaneLendLeasePerspectiveRange,
                [r.perspective_tag_db_id as i64, 0, 0, 0],
                21,
            );
        }
        if r.counterpart_tag_db_id == 0 || r.counterpart_tag_db_id > ctx.max_owner_tag_db_id {
            emit(
                D43LaneRule::D43LaneLendLeaseCounterpartRange,
                [r.counterpart_tag_db_id as i64, 0, 0, 0],
                22,
            );
        }
        if r.perspective_tag_db_id == r.counterpart_tag_db_id {
            emit(D43LaneRule::D43LaneLendLeaseSameParty, [0; 4], 23);
        }
        if !LendLeaseDirection::ALL
            .iter()
            .any(|v| v.wire() == r.direction)
        {
            emit(
                D43LaneRule::D43LaneLendLeaseDirection,
                [r.direction as i64, 0, 0, 0],
                24,
            );
        }
        if r.equipment_variant_id == 0 || r.equipment_variant_id > ctx.max_equipment_variant_id {
            emit(
                D43LaneRule::D43LaneLendLeaseVariantRange,
                [r.equipment_variant_id as i64, 0, 0, 0],
                25,
            );
        }
        let unknown = r.flags & !KNOWN_LEND_LEASE_FLAGS;
        if unknown != 0 {
            emit(
                D43LaneRule::D43LaneLendLeaseUnknownFlags,
                [unknown as i64, 0, 0, 0],
                26,
            );
        }
        if amount && (r.amount_raw <= 0 || r.amount_raw > ctx.max_amount_raw) {
            emit(
                D43LaneRule::D43LaneLendLeaseAmountRange,
                [r.amount_raw, 0, 0, 0],
                27,
            );
        } else if !amount && r.amount_raw != 0 {
            emit(
                D43LaneRule::D43LaneLendLeaseAmountWithoutFlag,
                [r.amount_raw, 0, 0, 0],
                28,
            );
        }
        if price && (r.price_raw <= 0 || r.price_raw > ctx.max_amount_raw) {
            emit(
                D43LaneRule::D43LaneLendLeasePriceRange,
                [r.price_raw, 0, 0, 0],
                29,
            );
        } else if !price && r.price_raw != 0 {
            emit(
                D43LaneRule::D43LaneLendLeasePriceWithoutFlag,
                [r.price_raw, 0, 0, 0],
                30,
            );
        }
        if r.source_record_kind != 1 {
            emit(
                D43LaneRule::D43LaneLendLeaseSourceRecordKind,
                [r.source_record_kind as i64, 0, 0, 0],
                31,
            );
        }
        if r.reserved != 0 {
            emit(D43LaneRule::D43LaneLendLeaseReserved, [0; 4], 32);
        }
        if r.contract_id != 0 && !seen.insert((r.contract_id, r.perspective_tag_db_id, r.direction))
        {
            emit(D43LaneRule::D43LaneLendLeaseDuplicateKey, [0; 4], 33);
        }
        match r.direction {
            1 => {
                computed[0] = computed[0].saturating_add(1);
                let pair = per_contract.entry(r.contract_id).or_default();
                pair.0 = pair.0.saturating_add(1);
            }
            2 => {
                computed[1] = computed[1].saturating_add(1);
                let pair = per_contract.entry(r.contract_id).or_default();
                pair.1 = pair.1.saturating_add(1);
            }
            _ => {}
        }
        computed[2] = computed[2].saturating_add(u32::from(amount));
        computed[3] = computed[3].saturating_add(u32::from(price));
    }
    for (contract_id, (outgoing, incoming)) in per_contract {
        if outgoing != 1 || incoming != 1 {
            push_lane_rule(
                report,
                D43DecodeRule::D43LendLeaseRowShape,
                D43LaneRule::D43LaneLendLeaseContractHalfCount,
                Trailer::LendLease,
                h.sized.at,
                None,
                [contract_id as i64, outgoing as i64, incoming as i64, 0],
                family_ordinal,
                40,
            );
        }
    }
    let expected = [
        h.outgoing_rows,
        h.incoming_rows,
        h.amount_valid_rows,
        h.price_valid_rows,
    ];
    let rules = [
        D43LaneRule::D43LaneLendLeaseOutgoingCount,
        D43LaneRule::D43LaneLendLeaseIncomingCount,
        D43LaneRule::D43LaneLendLeaseAmountCount,
        D43LaneRule::D43LaneLendLeasePriceCount,
    ];
    for index in 0..4 {
        if expected[index] != computed[index] {
            push_lane_rule(
                report,
                D43DecodeRule::D43LendLeaseHeaderCounts,
                rules[index],
                Trailer::LendLease,
                h.sized.at,
                None,
                [expected[index] as i64, computed[index] as i64, 0, 0],
                family_ordinal,
                50 + index as u16,
            );
        }
    }
}

fn evaluate_cross_family(report: &mut DecodeReport, _ctx: DecodeContext) {
    if let (Some(flow), Some(resource)) =
        (report.document.flow_header, report.document.resource_header)
    {
        if flow.resource_count != resource.resource_count {
            push_rule(
                report,
                D43DecodeRule::D43CrossResourceCountAgreement,
                Some(Trailer::ResourceFlow),
                None,
                flow.at + 24,
                None,
                [
                    flow.resource_count as i64,
                    resource.resource_count as i64,
                    0,
                    0,
                ],
                Some(Trailer::ResourceRow),
                2,
                0,
                1,
            );
        }
    }
    if let Some(resource) = report.document.resource_header {
        let deals = report.document.deal_rows.clone();
        for item in deals {
            if item.row.resource_index >= resource.resource_count {
                push_rule(
                    report,
                    D43DecodeRule::D43CrossDealResourceSlot,
                    Some(Trailer::ClassicDeal),
                    Some(TrailerBlock::Rows),
                    item.at + 8,
                    Some(item.index),
                    [
                        item.row.owner_tag_db_id as i64,
                        item.row.resource_index as i64,
                        resource.resource_count as i64,
                        0,
                    ],
                    Some(Trailer::ResourceRow),
                    2,
                    0,
                    2,
                );
            }
        }
    }
    let flow_set = report.document.flow_header.and_then(|header| {
        if header.version == FlowVersion::Legacy {
            return None;
        }
        let mut kinds = BTreeMap::<u32, BTreeSet<u32>>::new();
        for row in &report.document.flow_rows {
            kinds
                .entry(row.row.tag_db_id)
                .or_default()
                .insert(row.row.flow_kind);
        }
        if kinds.values().all(|set| set.len() == FlowKind::ALL.len()) {
            Some(kinds.keys().copied().collect::<BTreeSet<_>>())
        } else {
            None
        }
    });
    let factory_set = report.document.factory_header.map(|_| {
        report
            .document
            .factory_rows
            .iter()
            .map(|r| r.row.tag_db_id)
            .collect::<BTreeSet<_>>()
    });
    let resource_set = report.document.resource_header.and_then(|_| {
        if report.document.resource_rows.is_empty()
            && !report.document.resource_identities.is_empty()
        {
            None
        } else {
            Some(
                report
                    .document
                    .resource_rows
                    .iter()
                    .map(|r| r.row.tag_db_id)
                    .collect::<BTreeSet<_>>(),
            )
        }
    });
    let deal_set = report.document.deal_header.map(|_| {
        report
            .document
            .deal_witnesses
            .iter()
            .map(|r| r.row)
            .collect::<BTreeSet<_>>()
    });
    let sets = [
        (Trailer::ResourceFlow, flow_set),
        (Trailer::FactoryHeader, factory_set),
        (Trailer::ResourceRow, resource_set),
        (Trailer::ClassicDeal, deal_set),
    ];
    let mut reference: Option<(Trailer, BTreeSet<u32>)> = None;
    for (family, set) in sets {
        if let Some(set) = set {
            if let Some((base, base_set)) = &reference {
                if *base_set != set {
                    push_rule(
                        report,
                        D43DecodeRule::D43CrossCountrySetAgreement,
                        Some(*base),
                        None,
                        0,
                        None,
                        [base_set.len() as i64, set.len() as i64, 0, 0],
                        Some(family),
                        2,
                        0,
                        10,
                    );
                }
            } else {
                reference = Some((family, set));
            }
        }
    }
    if let Some(flow) = report.document.flow_header {
        if flow.version == FlowVersion::Variable {
            let identity = report
                .document
                .resource_header
                .map(|h| h.version.carries_identity())
                .unwrap_or(false)
                && !report.document.resource_identities.is_empty();
            if !identity {
                push_rule(
                    report,
                    D43DecodeRule::D43CrossFlowV3NeedsIdentityResourceRows,
                    Some(Trailer::ResourceFlow),
                    None,
                    flow.at,
                    None,
                    [0; 4],
                    Some(Trailer::ResourceRow),
                    2,
                    0,
                    11,
                );
            }
        }
    }
}

fn compute_units(report: &mut DecodeReport) {
    let mut units = D43UnitCounts::zero();
    let core_header = report.document.core_header;
    let core_is_publishable = core_header.is_some_and(|header| {
        report.document.core_rows.len() == header.row_count() as usize
            && !report
                .findings
                .iter()
                .any(|finding| finding.rule == D43DecodeRule::D43CoreScale)
    });

    if let Some(h) = core_header.filter(|_| core_is_publishable) {
        let contract_rows = h.word(CoreWord::RowCount) as usize;
        let observed_rows = h.word(CoreWord::ObservedContractRows) as usize;
        units.market_trade_contract_rows = Some(contract_rows);
        units.market_trade_observed_contract_rows = Some(observed_rows);
        units.market_trade_omitted_candidate_rows =
            Some(observed_rows.saturating_sub(h.word(CoreWord::EmittedContractRows) as usize));
        units.market_trade_buyer_ptr_valid_rows =
            Some(h.word(CoreWord::BuyerPtrValidRows) as usize);
        units.market_trade_variant_valid_rows = Some(h.word(CoreWord::VariantValidRows) as usize);
        units.market_trade_amount_valid_rows = Some(h.word(CoreWord::AmountValidRows) as usize);
        units.market_trade_partial_field_rows = Some(h.word(CoreWord::PartialFieldRows) as usize);

        let stopped_before_first_detail = match report.structural_stop {
            Some(D43DecodeStop::D43Grammar(violation)) => {
                violation_family(violation) == Some(Trailer::Detail)
                    && report.document.detail_header.is_none()
            }
            Some(D43DecodeStop::D43RejectedFamily { trailer, .. }) => {
                trailer == Trailer::Detail && report.document.detail_header.is_none()
            }
            Some(D43DecodeStop::D43Allocation { .. }) => true,
            None => false,
        };
        if !stopped_before_first_detail {
            let detail_rows = report.document.detail_rows.len();
            units.market_trade_detail_rows = Some(detail_rows);
            units.market_trade_detail_omitted_rows =
                Some(contract_rows.saturating_sub(detail_rows));
        }

        if report.document.detail_header.is_some() {
            let mut price = 0usize;
            let mut speed = 0usize;
            let mut completed = 0usize;
            let mut days = 0usize;
            let mut country = 0usize;
            let mut convoy = 0usize;
            let mut efficiency = 0usize;
            let mut route = 0usize;
            let mut progress = 0usize;
            for item in &report.document.detail_rows {
                let flags = item.row.flags;
                price += usize::from(DetailFlag::PriceValid.is_set(flags));
                speed += usize::from(DetailFlag::SpeedValid.is_set(flags));
                completed += usize::from(DetailFlag::CompletedValid.is_set(flags));
                days += usize::from(DetailFlag::DaysValid.is_set(flags));
                country += usize::from(DetailFlag::DeliveryCountryValid.is_set(flags));
                convoy += usize::from(DetailFlag::ConvoysValid.is_set(flags));
                efficiency += usize::from(DetailFlag::EfficiencyValid.is_set(flags));
                route += usize::from(
                    flags
                        & (DetailFlag::DeliveryCountryValid.bit()
                            | DetailFlag::ConvoysValid.bit()
                            | DetailFlag::EfficiencyValid.bit())
                        != 0,
                );
                progress += usize::from(
                    flags
                        & (DetailFlag::FactoryProgressValid.bit()
                            | DetailFlag::EquipmentProgressValid.bit())
                        != 0,
                );
            }
            units.market_trade_detail_price_valid_rows = Some(price);
            units.market_trade_detail_speed_valid_rows = Some(speed);
            units.market_trade_detail_completed_valid_rows = Some(completed);
            units.market_trade_detail_days_valid_rows = Some(days);
            units.market_trade_detail_delivery_country_valid_rows = Some(country);
            units.market_trade_detail_convoy_valid_rows = Some(convoy);
            units.market_trade_detail_efficiency_valid_rows = Some(efficiency);
            units.market_trade_detail_delivery_route_valid_rows = Some(route);
            units.market_trade_detail_progress_valid_rows = Some(progress);
        }

        if report.document.flow_header.is_some() {
            units.market_trade_resource_flow_rows = Some(report.document.flow_rows.len());
        }
        if report.document.factory_header.is_some() {
            units.market_trade_factory_header_rows = Some(report.document.factory_rows.len());
        }
        if report.document.resource_header.is_some() {
            units.market_trade_resource_rows = Some(report.document.resource_rows.len());
        }
        if report.document.deal_header.is_some() {
            units.market_trade_classic_deal_country_witnesses =
                Some(report.document.deal_witnesses.len());
            units.market_trade_classic_deal_incomplete_owners =
                Some(report.document.deal_incomplete_owners.len());
            units.market_trade_classic_deal_rows = Some(report.document.deal_rows.len());
        }
        if report.document.activation_header.is_some() {
            units.market_trade_activation_rows = Some(1);
        }
        if report.document.stockpile_header.is_some() {
            units.market_trade_stockpile_country_rows =
                Some(report.document.stockpile_countries.len());
            units.market_trade_stockpile_item_rows = Some(report.document.stockpile_items.len());
        }
    }
    units.fold_contract_keys = report
        .document
        .core_rows
        .iter()
        .map(|r| (r.row.contract_id, r.row.contract_type))
        .collect::<BTreeSet<_>>()
        .len();
    units.fold_detail_contract_keys = report
        .document
        .detail_rows
        .iter()
        .map(|r| r.row.contract_id)
        .collect::<BTreeSet<_>>()
        .len();
    units.fold_resource_pool_countries = report
        .document
        .flow_rows
        .iter()
        .map(|r| r.row.tag_db_id)
        .collect::<BTreeSet<_>>()
        .len();
    units.fold_factory_header_countries = report
        .document
        .factory_rows
        .iter()
        .map(|r| r.row.tag_db_id)
        .collect::<BTreeSet<_>>()
        .len();
    units.fold_resource_row_countries = report
        .document
        .resource_rows
        .iter()
        .map(|r| r.row.tag_db_id)
        .collect::<BTreeSet<_>>()
        .len();
    units.fold_classic_deal_owner_countries = report
        .document
        .deal_witnesses
        .iter()
        .map(|r| r.row)
        .collect::<BTreeSet<_>>()
        .len();
    units.fold_stockpile_countries = report
        .document
        .stockpile_countries
        .iter()
        .map(|r| r.row.tag_db_id)
        .collect::<BTreeSet<_>>()
        .len();
    let publication_cut = match report.structural_stop {
        Some(D43DecodeStop::D43Grammar(violation)) => Some(violation.at),
        Some(D43DecodeStop::D43RejectedFamily { at, .. }) => Some(at),
        Some(D43DecodeStop::D43Allocation { at, .. }) => Some(at),
        None => None,
    };
    if let Some(cut) = publication_cut {
        if report
            .document
            .detail_header
            .is_some_and(|header| header.sized.at >= cut)
        {
            units.market_trade_detail_rows = None;
            units.market_trade_detail_omitted_rows = None;
            units.market_trade_detail_price_valid_rows = None;
            units.market_trade_detail_speed_valid_rows = None;
            units.market_trade_detail_completed_valid_rows = None;
            units.market_trade_detail_days_valid_rows = None;
            units.market_trade_detail_delivery_country_valid_rows = None;
            units.market_trade_detail_convoy_valid_rows = None;
            units.market_trade_detail_efficiency_valid_rows = None;
            units.market_trade_detail_delivery_route_valid_rows = None;
            units.market_trade_detail_progress_valid_rows = None;
        }
        if report
            .document
            .flow_header
            .is_some_and(|header| header.at >= cut)
        {
            units.market_trade_resource_flow_rows = None;
        }
        if report
            .document
            .factory_header
            .is_some_and(|header| header.sized.at >= cut)
        {
            units.market_trade_factory_header_rows = None;
        }
        if report
            .document
            .resource_header
            .is_some_and(|header| header.at >= cut)
        {
            units.market_trade_resource_rows = None;
        }
        if report
            .document
            .deal_header
            .is_some_and(|header| header.at >= cut)
        {
            units.market_trade_classic_deal_country_witnesses = None;
            units.market_trade_classic_deal_incomplete_owners = None;
            units.market_trade_classic_deal_rows = None;
        }
        if report
            .document
            .activation_header
            .is_some_and(|header| header.at >= cut)
        {
            units.market_trade_activation_rows = None;
        }
        if report
            .document
            .stockpile_header
            .is_some_and(|header| header.at >= cut)
        {
            units.market_trade_stockpile_country_rows = None;
            units.market_trade_stockpile_item_rows = None;
        }
    }
    report.units = units;
}
