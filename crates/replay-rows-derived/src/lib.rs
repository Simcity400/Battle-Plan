use engine_allocation as allocation;
use replay_rows::codec::{Decoder, invalid_static as invalid, put_varint};
use replay_rows::rows::*;
use replay_rows::row_codec;
use std::io;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TagHolderRow {
    pub number: u32,
    pub epoch: u32,
    pub slot: u32,
    pub id: u32,
    pub tag: Vec<u8>,
}

impl TagHolderRow {
    pub const UNKNOWN: u32 = u32::MAX;
    pub const NONE: u32 = u32::MAX;
    pub const REASONS: [&'static str; 3] = [
        "Countries was not complete at these hours",
        "CountryLinks was not complete at these hours",
        "Countries and CountryLinks were not complete at these hours",
    ];
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GroupSpanRow {
    pub kind: u8,
    pub group: u32,
    pub value: u32,
    pub epoch: u32,
    pub ordinal: u32,
}

impl GroupSpanRow {
    pub const MEMBER: u8 = 0;
    pub const LEADER: u8 = 1;
    pub const NAME: u8 = 2;
    pub const PRESENT: u8 = 3;
    pub const INCOMPLETE: u8 = 4;
    pub const WAR: u8 = 5;
    pub const UNOBSERVED: u8 = 6;
    pub const HUMANS: u32 = 0;
    pub const FACTIONS: u32 = 1;
    pub const WARS: u32 = 2;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GroupRow {
    pub number: u32,
    pub faction: u64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GroupNameRow {
    pub number: u32,
    pub literal: Vec<u8>,
    pub template_token: Option<u32>,
    pub spelling: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WarEventRow {
    pub kind: u8,
    pub hour: i32,
    pub first: u32,
    pub second: u32,
    pub read: u8,
}

impl WarEventRow {
    pub const START: u8 = 0;
    pub const CAPITULATION: u8 = 1;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LookSpanRow {
    pub number: u32,
    pub fact: u32,
    pub epoch: u32,
    pub read: u8,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LookFactRow {
    pub number: u32,
    pub look: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct BalanceDropRow {
    pub kind: u8,
    pub country: u32,
    pub hour: i32,
    pub before: i64,
    pub after: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TagNameRow {
    pub number: u32,
    pub tag: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyTotalRow {
    pub slot: u32,
    pub owner_divisions: Recorded<i64>,
    pub owner_strength_raw: Recorded<i64>,
    pub host_divisions: Recorded<i64>,
    pub host_strength_raw: Recorded<i64>,
    pub owner_battalions: Recorded<i64>,
    pub host_battalions: Recorded<i64>,
    pub owner_manpower: Recorded<i64>,
    pub host_manpower: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TerritoryTotalRow {
    pub slot: u32,
    pub owned_states: Recorded<i64>,
    pub controlled_states: Recorded<i64>,
    pub occupied_states: Recorded<i64>,
    pub owned_provinces: Recorded<i64>,
    pub controlled_provinces: Recorded<i64>,
    pub occupied_provinces: Recorded<i64>,
    pub owned_victory_points: Recorded<i64>,
    pub controlled_victory_points: Recorded<i64>,
    pub occupied_victory_points: Recorded<i64>,
    pub owned_area: Recorded<i64>,
    pub controlled_area: Recorded<i64>,
    pub occupied_area: Recorded<i64>,
}

impl TerritoryTotalRow {
    pub const WORLD: u32 = u32::MAX;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryStabilityRow {
    pub country: u32,
    pub stability: Recorded<i64>,
    pub war_support: Recorded<i64>,
    pub max_command_power: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyMixRow {
    pub slot: u32,
    pub attribution: u8,
    pub token: u32,
    pub battalions: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirForceTotalRow {
    pub slot: u32,
    pub roster_planes: Recorded<i64>,
    pub roster_wings: Recorded<i64>,
    pub effective_planes: Recorded<i64>,
    pub effective_wings: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirForcePartRow {
    pub slot: u32,
    pub ownership: u8,
    pub variant: Option<u64>,
    pub planes: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirForceWingKindRow {
    pub slot: u32,
    pub ownership: u8,
    pub signature: u32,
    pub wings: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirForceWingSignatureRow {
    pub number: u32,
    pub variants: Vec<u8>,
}

impl AirForceWingSignatureRow {
    pub fn encode(variants: &[u64]) -> Vec<u8> {
        variants.iter().flat_map(|variant| variant.to_le_bytes()).collect()
    }

    pub fn variants(&self) -> io::Result<Vec<u64>> {
        if self.variants.len() % 8 != 0 {
            return Err(invalid("a wing signature is not whole variants"));
        }
        Ok(self.variants.chunks_exact(8).map(|bytes| u64::from_le_bytes(bytes.try_into().expect("eight bytes"))).collect())
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavyTotalRow {
    pub slot: u32,
    pub roster_ships: Recorded<i64>,
    pub roster_strength_raw: Recorded<i64>,
    pub roster_max_strength_raw: Recorded<i64>,
    pub logical_ships: Recorded<i64>,
    pub logical_strength_raw: Recorded<i64>,
    pub logical_max_strength_raw: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavyClassRow {
    pub slot: u32,
    pub ownership: u8,
    pub class: u32,
    pub ships: i64,
    pub strength_raw: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryDeployedManpowerRow {
    pub slot: u32,
    pub army: Recorded<i32>,
    pub navy: Recorded<i32>,
    pub navy_required: Recorded<i32>,
    pub air: Recorded<i32>,
    pub exile_air: Recorded<i32>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct MilitaryAidRow {
    pub sender: u32,
    pub host: u32,
    pub channel: u8,
    pub divisions: Recorded<i64>,
}

impl MilitaryAidRow {
    pub const VOLUNTEERS: u8 = 0;
    pub const EXPEDITIONARIES: u8 = 1;
    pub const TALLIES: u8 = 2;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct SupplyHealthRow {
    pub slot: u32,
    pub no_supply: Recorded<i64>,
    pub strained: Recorded<i64>,
    pub in_supply: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct EquipmentLossTotalRow {
    pub slot: u32,
    pub country: u32,
    pub source: u8,
    pub variant: Option<u32>,
    pub total: Recorded<i64>,
    pub incomplete_from: Option<i32>,
    pub freed: Recorded<i64>,
}

impl EquipmentLossTotalRow {
    pub const COMBAT_LOG: u8 = 0;
    pub const SUNK_CONVOYS: u8 = 32;
    pub const LEND_LEASE: u8 = 48;
    pub const DESTRUCTION: u8 = 64;

    pub fn variant_id(handle: u64) -> Option<u32> {
        (handle as u32 == engine_reference_values::native_values::equipment_variant::REFERENCE_TYPE).then_some((handle >> 32) as u32)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CombatLogDroppedRow {
    pub country: u32,
    pub group: u32,
    pub kind: u8,
    pub month: u16,
    pub variant: u32,
    pub date: Recorded<i32>,
    pub amount: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ShipLossRow {
    pub slot: u32,
    pub country: u32,
    pub hour: i32,
    pub signature: u64,
    pub sink_hour: i32,
    pub variant: u64,
    pub count: u32,
    pub check: u64,
}

impl ShipLossRow {
    pub const FIELD: u32 = (1 << 24) - 1;
    pub const SIGNATURE: u64 = (1 << 48) - 1;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct EquipmentLossCoverageRow {
    pub slot: u32,
    pub country: u32,
    pub land: Recorded<u8>,
    pub ungrouped: Recorded<u32>,
    pub resolved: Recorded<u8>,
    pub convoy_drops: Recorded<u32>,
    pub drain: Recorded<Vec<u8>>,
    pub unread: Recorded<u8>,
    pub counted_from: Recorded<Option<i32>>,
    pub withheld_from: Recorded<Option<i32>>,
    pub incomplete_from: Recorded<Option<i32>>,
    pub convoys_from: Recorded<Option<i32>>,
    pub lend_from: Recorded<Option<i32>>,
    pub exchanges_ended: Recorded<u32>,
    pub ended_from: Recorded<Option<i32>>,
    pub unresolved: Recorded<u32>,
    pub unresolved_all: Recorded<u32>,
    pub drain_state: Recorded<u8>,
}

impl EquipmentLossCoverageRow {
    pub const RUN: u32 = u32::MAX;
    pub const LOGGED: u8 = 0;
    pub const NOT_LOGGED: u8 = 1;
    pub const PARTIAL: u8 = 2;
    pub const UNKNOWN: u8 = 3;
    pub const PRUNE_UNOBSERVED: u8 = 4;
    pub const UNREAD_COMBAT_LOG: u8 = 1;
    pub const UNREAD_SHIPS: u8 = 2;
    pub const UNREAD_CONVOYS: u8 = 4;
    pub const UNREAD_LEND_LEASE: u8 = 8;
    pub const DRAIN_STATE_COMPLETE: u8 = 0;
    pub const DRAIN_STATE_PARTIAL_INTERVAL: u8 = 1;
    pub const DRAIN_STATE_NOT_READ: u8 = 2;
    pub const DRAIN_STATE_LOST: u8 = 4;
    pub const DRAIN_STATE_OWNER_UNKNOWN: u8 = 8;
    pub const DRAIN_STATE_SUPPLEMENTAL_UNAVAILABLE: u8 = 16;
    pub const DRAIN_STATE_WITHHELD: u8 = 32;
    pub const DRAIN_STATE_MASK: u8 = Self::DRAIN_STATE_PARTIAL_INTERVAL
        | Self::DRAIN_STATE_NOT_READ
        | Self::DRAIN_STATE_LOST
        | Self::DRAIN_STATE_OWNER_UNKNOWN
        | Self::DRAIN_STATE_SUPPLEMENTAL_UNAVAILABLE
        | Self::DRAIN_STATE_WITHHELD;
    pub const DRAIN_WITHHELD: &'static str = "Equipment destruction drain is withheld for the run";
    pub const DRAIN_TRUNCATED: &'static str = "Equipment destruction drain is truncated; withheld for the run";
    pub const DRAIN_SUPPLEMENTAL_NOT_RECORDED: &'static str = "EquipmentDestruction supplemental coverage not recorded";
    pub const DRAIN_OWNER_UNKNOWN: &'static str = "Equipment destruction ownership is unknown: raw country references have no CountryLink";
    pub const DRAIN_NOT_RECORDED: &'static str = "EquipmentDestruction is not recorded in this recording";
    pub const DRAIN_NOT_READ: &'static str = "Equipment destruction drain was not read at this hour";
    pub const DRAIN_LOST: &'static str = "Equipment destruction drain is missing at hours the writer did not run";
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StockByVariantRow {
    pub slot: u32,
    pub variant: u64,
    pub amount: Recorded<i64>,
    pub positions: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionEquipmentCountRow {
    pub slot: u32,
    pub side: u8,
    pub divisions: Recorded<i64>,
    pub short: Recorded<i64>,
    pub unknown: Recorded<i64>,
    pub unpooled: Recorded<i64>,
    pub unclassified: Recorded<i64>,
    pub unresolved: i64,
    pub pool_hour: Option<i32>,
    pub stock_hour: Option<i32>,
}

impl DivisionEquipmentCountRow {
    pub const OWNER: u8 = 0;
    pub const HOST: u8 = 1;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionEquipmentTypeRow {
    pub slot: u32,
    pub side: u8,
    pub archetype: Option<u32>,
    pub held: Recorded<i64>,
    pub needed: Recorded<i64>,
    pub filled: Recorded<i64>,
    pub missing: Recorded<i64>,
    pub short: Recorded<i64>,
    pub stock: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionEquipmentDesignRow {
    pub slot: u32,
    pub side: u8,
    pub archetype: Option<u32>,
    pub variant: Option<u32>,
    pub held: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionEquipmentTemplateRow {
    pub slot: u32,
    pub side: u8,
    pub template: u64,
    pub divisions: Recorded<i64>,
    pub unknown: Recorded<i64>,
    pub types_short: Recorded<i64>,
    pub manpower: Recorded<i64>,
    pub manpower_lacking: Recorded<i64>,
    pub required: Recorded<i64>,
    pub hp: Recorded<i64>,
    pub hp_max: Recorded<i64>,
    pub hp_max_lacking: Recorded<i64>,
    pub needed: Recorded<i64>,
    pub held: Recorded<i64>,
    pub need_missing: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionTotalRow {
    pub country: u32,
    pub variant: u64,
    pub purpose: u8,
    pub units: i64,
    pub inexact_rows: u32,
    pub spent: Recorded<i64>,
    pub spent_reason: u8,
    pub unproven: Recorded<i64>,
    pub equipment_token: Recorded<Vec<u8>>,
    pub archetype: u32,
}

impl ProductionTotalRow {
    pub const KNOWN: u8 = 0;
    pub const NOT_RECORDED: u8 = 1;
    pub const SPENDING_UNAVAILABLE: u8 = 2;
    pub const LOST_OUTPUT: u8 = 3;
    pub const MISSED_PUBLICATION: u8 = 4;
    pub const ROCKET_INPUT_INEXACT: u8 = 5;
    pub const ROCKET_OWNER_UNREADABLE: u8 = 6;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionCountryTotalRow {
    pub country: u32,
    pub units: i64,
    pub units_inexact: u32,
    pub spent: Recorded<i64>,
    pub units_without_refits: i64,
    pub units_without_refits_inexact: u32,
    pub spent_without_refits: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionTypeTotalRow {
    pub country: u32,
    pub archetype: u32,
    pub units: i64,
    pub units_inexact: u32,
    pub spent: Recorded<i64>,
    pub units_without_refits: i64,
    pub units_without_refits_inexact: u32,
    pub spent_without_refits: Recorded<i64>,
    pub archetype_spelling: Recorded<Vec<u8>>,
}

impl ProductionTypeTotalRow {
    pub const WORLD: u32 = u32::MAX;
    pub const NO_ARCHETYPE: u32 = 0;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionCoverageRow {
    pub country: u32,
    pub source: u8,
    pub observation_since: Option<i32>,
    pub units_unknown_from: Option<i32>,
    pub spending_unknown_from: Option<i32>,
    pub spending_reason: u8,
}

impl ProductionCoverageRow {
    pub const RUN: u32 = u32::MAX;
    pub const LINES: u8 = 0;
    pub const ROCKET: u8 = 1;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DockyardRepairRateRow {
    pub country: u32,
    pub part: u8,
    pub design: u64,
    pub rate: Recorded<i64>,
    pub used_dockyards: Recorded<i32>,
    pub output: Recorded<i64>,
    pub split_known: Recorded<bool>,
}

impl DockyardRepairRateRow {
    pub const TOTAL: u8 = 0;
    pub const UNATTRIBUTED: u8 = 1;
    pub const DESIGN: u8 = 2;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandBattleRow {
    pub combat: u64,
    pub start_hour: i32,
    pub start_exact: bool,
    pub counter_base: i32,
    pub peak_total: Recorded<u32>,
    pub peak_total_hour: Recorded<i32>,
    pub peak_attackers: Recorded<u32>,
    pub peak_attackers_hour: Recorded<i32>,
    pub peak_defenders: Recorded<u32>,
    pub peak_defenders_hour: Recorded<i32>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandBattleEndRow {
    pub combat: u64,
    pub start_hour: i32,
    pub result_only: bool,
    pub kind: u8,
    pub end_hour: i32,
    pub start_exact: bool,
    pub counter_base: i32,
    pub province: u32,
    pub last_read: Option<i32>,
    pub last_duration: Recorded<i32>,
    pub peak_total: Recorded<u32>,
    pub peak_total_hour: Recorded<i32>,
    pub peak_attackers: Recorded<u32>,
    pub peak_attackers_hour: Recorded<i32>,
    pub peak_defenders: Recorded<u32>,
    pub peak_defenders_hour: Recorded<i32>,
    pub attacker_manpower: Recorded<i32>,
    pub defender_manpower: Recorded<i32>,
    pub attackers: Recorded<Vec<u32>>,
    pub defenders: Recorded<Vec<u32>>,
    pub countries_unknown: u8,
    pub result: Option<u64>,
    pub result_read: bool,
    pub winners: Recorded<u8>,
    pub result_duration: Recorded<i32>,
    pub attacker_terminal: Recorded<i32>,
    pub defender_terminal: Recorded<i32>,
    pub result_attackers: Recorded<Vec<u32>>,
    pub result_defenders: Recorded<Vec<u32>>,
    pub result_unknown: u8,
    pub attacker_terminal_equipment: Recorded<Vec<LandEquipmentLoss>>,
    pub defender_terminal_equipment: Recorded<Vec<LandEquipmentLoss>>,
}

impl LandBattleEndRow {
    pub const ABSENT: u8 = 0;
    pub const REUSED: u8 = 1;
    pub const FINALIZED: u8 = 2;
    pub const UNKNOWN: u8 = 3;
    pub const RESULT_ONLY: u8 = 4;
    pub const ATTACKERS_UNKNOWN: u8 = 1;
    pub const DEFENDERS_UNKNOWN: u8 = 2;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalBattleRow {
    pub combat: u64,
    pub first_hour: i32,
    pub peak_ships: Recorded<u32>,
    pub peak_hour: Recorded<i32>,
    pub attacker_sunk: Recorded<u32>,
    pub defender_sunk: Recorded<u32>,
    pub attacker_convoys_sunk: Recorded<u32>,
    pub defender_convoys_sunk: Recorded<u32>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalResultObservationRow {
    pub result: u64,
    pub refusal: u8,
    pub combat: u64,
    pub province: Recorded<u32>,
    pub first_hour: Recorded<i32>,
    pub last_hour: Recorded<i32>,
    pub peak_ships: Recorded<u32>,
    pub peak_hour: Recorded<i32>,
    pub attacker_ships: Recorded<u32>,
    pub attacker_convoys: Recorded<u32>,
    pub attacker_sunk: Recorded<u32>,
    pub attacker_convoys_sunk: Recorded<u32>,
    pub defender_ships: Recorded<u32>,
    pub defender_convoys: Recorded<u32>,
    pub defender_sunk: Recorded<u32>,
    pub defender_convoys_sunk: Recorded<u32>,
}

impl NavalResultObservationRow {
    pub const OBSERVED: u8 = 0;
    pub const INVALID_SOURCE: u8 = 1;
    pub const DUPLICATE: u8 = 2;
    pub const MISMATCH: u8 = 3;
    pub const NOT_OBSERVED: u8 = 4;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalLossTotalRow {
    pub slot: u32,
    pub country: u32,
    pub attribution: u8,
    pub kind: u8,
    pub total: Recorded<i64>,
    pub unknown: Recorded<u32>,
    pub counted_from: Recorded<i32>,
    pub known: Recorded<bool>,
    pub lost_from: Recorded<Option<i32>>,
}

impl NavalLossTotalRow {
    pub const NONE: u32 = u32::MAX - 1;
    pub const RUN: u32 = u32::MAX;
    pub const LOST: u8 = 0;
    pub const SUNK_BY: u8 = 1;
    pub const SHIPS: u8 = 0;
    pub const CONVOYS: u8 = 1;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PowerSpendTotalRow {
    pub holder: u32,
    pub currency: u8,
    pub ordinal: u32,
    pub known: Recorded<i64>,
    pub payments: Recorded<i64>,
    pub entries: Recorded<i64>,
    pub unknown: Recorded<i64>,
    pub uncovered: Recorded<i64>,
    pub latest_hour: Recorded<Option<i32>>,
    pub latest_sequence: Recorded<Option<u64>>,
    pub latest_ordinal: Recorded<Option<u32>>,
    pub latest_read: Recorded<Option<i32>>,
    pub days: Recorded<i64>,
    pub day_payments: Recorded<i64>,
    pub day_entries: Recorded<i64>,
    pub day_charge: Recorded<i64>,
    pub day_unknown: Recorded<i64>,
    pub days_uncovered: Recorded<i64>,
    pub realized_gained: Recorded<i64>,
    pub realized_lost: Recorded<i64>,
    pub realized_unknown: Recorded<i64>,
    pub first_day: Recorded<Option<i32>>,
    pub last_day: Recorded<Option<i32>>,
    pub last_lag: Recorded<Option<i32>>,
    pub last_hour: Recorded<Option<i32>>,
    pub last_sequence: Recorded<Option<u64>>,
    pub last_ordinal: Recorded<Option<u32>>,
    pub last_covered: Recorded<bool>,
    pub balances: Recorded<i64>,
    pub credits: Recorded<i64>,
    pub gained: Recorded<i64>,
    pub capped: Recorded<i64>,
    pub gain_days: Recorded<i64>,
    pub gain_unknown_days: Recorded<i64>,
    pub unmatched: Recorded<i64>,
}

impl PowerSpendTotalRow {
    pub const GAINS: u32 = u32::MAX;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PowerSpendDayRow {
    pub holder: u32,
    pub hour: i32,
    pub currency: u8,
    pub entry: u32,
    pub ordinal: u32,
    pub day: i32,
    pub days: i64,
    pub payments: i64,
    pub charge: Recorded<i64>,
    pub unknown: i64,
    pub uncovered: i64,
    pub realized_gained: Recorded<i64>,
    pub realized_lost: Recorded<i64>,
    pub realized_unknown: i64,
    pub credits: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PowerItemRow {
    pub holder: u32,
    pub ordinal: u32,
    pub kind: u8,
    pub currency: PowerCurrency,
    pub cause: PowerCause,
    pub component: Option<u32>,
    pub charged: bool,
    pub site: PowerSite,
    pub action: PowerAction,
    pub item_token: Option<u32>,
    pub item_key: Vec<u8>,
    pub slot_key: Vec<u8>,
    pub character: Option<u32>,
    pub object_kind: PowerObjectKind,
    pub object_reference: Option<u64>,
    pub target_country: Option<u32>,
    pub target_state: Option<u32>,
    pub target_region: Option<u32>,
    pub origin_kind: PowerOriginKind,
    pub origin_token: Option<u32>,
    pub origin_key: Vec<u8>,
    pub first_hour: i32,
    pub first_read: i32,
    pub first_country: u32,
    pub first_epoch: u64,
    pub first_sequence: u64,
}

impl PowerItemRow {
    pub const CHANGE: u8 = 0;
    pub const COST: u8 = 1;
    pub const ABILITY_UPKEEP: u8 = 2;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PowerDayPartRow {
    pub holder: u32,
    pub currency: u8,
    pub ordinal: u32,
    pub day: i32,
    pub spent: Recorded<i64>,
    pub payments: i64,
    pub covered: bool,
    pub sequence: Option<u64>,
    pub cost_ordinal: Option<u32>,
    pub read: i32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PowerHoldRow {
    pub holder: u32,
    pub hold: u64,
    pub capture_country: u32,
    pub capture_epoch: u64,
    pub amount: i64,
    pub check: u64,
    pub cause: PowerCause,
    pub object_kind: PowerObjectKind,
    pub object_reference: Option<u64>,
    pub ability: bool,
    pub item_token: Option<u32>,
    pub item_key: Vec<u8>,
    pub slot_key: Vec<u8>,
    pub character: Option<u32>,
    pub target_country: Option<u32>,
    pub target_state: Option<u32>,
    pub target_region: Option<u32>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PowerSpendCoverageRow {
    pub holder: u32,
    pub political: bool,
    pub command: bool,
    pub reason: u8,
    pub political_since: Option<i32>,
    pub command_since: Option<i32>,
    pub daily_mismatch: u8,
    pub input_reason: Vec<u8>,
}

impl PowerSpendCoverageRow {
    pub const STEADY_MISMATCH: u8 = 1;
    pub const GAIN_UNMATCHED: u8 = 2;
    pub const KNOWN: u8 = 0;
    pub const CHANGES_UNREAD: u8 = 1;
    pub const DAILY_COSTS_UNREAD: u8 = 2;
    pub const POLITICAL_OPERAND: u8 = 3;
    pub const COMMAND_OPERAND: u8 = 4;
    pub const BOTH_OPERANDS: u8 = 5;
    pub const NO_CAPTURE: u8 = 6;
    pub const STALE: u8 = 7;
    pub const COST_UNMATCHED: u8 = 8;
    pub const NEGATIVE_COST: u8 = 9;
    pub const STREAMS: u8 = 10;
    pub const BACKLOG: u8 = 11;
    pub const REASONS: [&'static str; 12] = [
        "",
        "Spending events were not recorded at this read",
        "Daily political power costs were not recorded at this read",
        "A political power adjustment has no recorded evaluated operand",
        "A command power adjustment has no recorded evaluated operand",
        "Political and command power adjustments have no recorded evaluated operands",
        "country identity or politics coverage unavailable",
        "politics coverage is stale at this read",
        "Daily political power costs have no matched payment at this read",
        "Negative daily political power cost components are not included in spending",
        "the politics capture did not account for every balance change at this read",
        "the run's first hour lists what the game held when the run began: spending is counted from the next hour",
    ];
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FeedEventRow {
    pub generation: u64,
    pub sequence: u64,
    pub hour: i32,
    pub read_hour: i32,
    pub kind: u8,
    pub actor_reference: u32,
    pub actor_id: Recorded<u32>,
    pub actor_instance: Recorded<u32>,
    pub actor_slot: Recorded<u32>,
    pub target_reference: u32,
    pub target_id: Recorded<u32>,
    pub target_instance: Recorded<u32>,
    pub target_slot: Recorded<u32>,
    pub route: Recorded<u8>,
    pub within: Recorded<u8>,
    pub conference: Recorded<Option<u64>>,
    pub state: Recorded<Option<u32>>,
    pub ally_reference: Recorded<Option<u32>>,
    pub ally_id: Recorded<Option<u32>>,
    pub ally_instance: Recorded<Option<u32>>,
    pub ally_slot: Recorded<Option<u32>>,
    pub reason: Recorded<Option<u32>>,
    pub flags: Recorded<u8>,
    pub token: Recorded<Vec<u8>>,
    pub native_event_kind: Option<u32>,
    pub native_event_id: Option<u32>,
    pub instance: Option<u32>,
    pub option: Option<i32>,
    pub country_target: Option<u32>,
    pub numeric_target: Option<i32>,
    pub lifecycle: Option<i32>,
    pub days: Option<i32>,
    pub scopes: Recorded<Vec<replay_rows::event_feed_values::EventFeedScope>>,
    pub opener: Option<u64>,
    pub opener_hour: Option<i32>,
    pub dated: Option<i32>,
    pub political_power: Recorded<Option<i64>>,
    pub command_power: Recorded<Option<i64>>,
    pub coverage: Option<u32>,
    pub outstanding: Option<u64>,
    pub refusals: Option<u32>,
    pub failure: Vec<u8>,
    pub backlog: bool,
    pub incomplete: Option<u32>,
}

impl FeedEventRow {
    pub const REFUSALS_SHIFT: u32 = 8;

    pub const fn incomplete_of(coverage: u8, refusals: u8) -> u32 {
        coverage as u32 | (refusals as u32) << Self::REFUSALS_SHIFT
    }

    pub const DERIVED: u64 = u64::MAX;
    pub const CEASED_TO_EXIST: u8 = 128;
    pub const GUARANTEE_GRANTED: u8 = 129;
    pub const READ_INCOMPLETE: u8 = 130;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FeedOpenEventRow {
    pub kind: u32,
    pub id: u32,
    pub instance: u32,
    pub receiver: u32,
    pub generation: u64,
    pub sequence: u64,
    pub read_hour: i32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FeedOpenDecisionRow {
    pub generation: u64,
    pub sequence: u64,
    pub country: u32,
    pub token: Vec<u8>,
    pub country_target: Option<u32>,
    pub numeric_target: Option<i32>,
    pub read_hour: i32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FeedGuaranteeRow {
    pub first: u32,
    pub second: u32,
    pub start: i32,
    pub hour: Option<i32>,
    pub rows: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalRegionControlRow {
    pub viewer: u32,
    pub region: u32,
    pub friendly: Recorded<i64>,
    pub enemy: Recorded<i64>,
    pub ratio_friendly: Recorded<i64>,
    pub ratio_enemy: Recorded<i64>,
    pub own: Recorded<i64>,
    pub control_refusal: Vec<u8>,
    pub share_refusal: Vec<u8>,
}

impl NavalRegionControlRow {
    pub const EVERY_VIEWER: u32 = u32::MAX;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryNukeTotalRow {
    pub country: u32,
    pub total: Recorded<i64>,
    pub ready: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ThreatTotalRow {
    pub sum: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionTotalRow {
    pub faction: u64,
    pub owned_states: Recorded<i64>,
    pub controlled_states: Recorded<i64>,
    pub occupied_states: Recorded<i64>,
    pub owned_provinces: Recorded<i64>,
    pub controlled_provinces: Recorded<i64>,
    pub occupied_provinces: Recorded<i64>,
    pub owned_victory_points: Recorded<i64>,
    pub controlled_victory_points: Recorded<i64>,
    pub occupied_victory_points: Recorded<i64>,
    pub owned_area: Recorded<i64>,
    pub controlled_area: Recorded<i64>,
    pub occupied_area: Recorded<i64>,
    pub free1: Recorded<i64>,
    pub free2: Recorded<i64>,
    pub free3: Recorded<i64>,
    pub free4: Recorded<i64>,
    pub free5: Recorded<i64>,
    pub free6: Recorded<i64>,
    pub free7: Recorded<i64>,
    pub built1: Recorded<i64>,
    pub built2: Recorded<i64>,
    pub built3: Recorded<i64>,
    pub built4: Recorded<i64>,
    pub built5: Recorded<i64>,
    pub built6: Recorded<i64>,
    pub built7: Recorded<i64>,
    pub top_bar1: Recorded<i64>,
    pub top_bar2: Recorded<i64>,
    pub top_bar3: Recorded<i64>,
    pub top_bar4: Recorded<i64>,
    pub top_bar5: Recorded<i64>,
    pub top_bar6: Recorded<i64>,
    pub top_bar7: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidEndRow {
    pub hour: i32,
    pub id: u32,
    pub reference_type: u32,
    pub kind: u8,
    pub previous_read: i32,
    pub first_hour: Option<i32>,
    pub first_exact: bool,
    pub launched_hour: Option<i32>,
    pub raider: Recorded<u32>,
    pub phase: Recorded<i32>,
    pub outcome: Recorded<i32>,
    pub travelled: Recorded<Option<i64>>,
    pub preparation_hours: Recorded<i32>,
    pub province: Recorded<u32>,
    pub state: Recorded<u32>,
    pub leader_reference: Recorded<u64>,
    pub leader_province: Recorded<u32>,
    pub building_type: Recorded<u32>,
    pub building_location: Recorded<u32>,
    pub building_flags: Recorded<u32>,
    pub building_country: Recorded<i32>,
    pub leader_country: Recorded<Option<i32>>,
    pub raid_type: Recorded<u32>,
    pub target_state: Recorded<Option<u32>>,
    pub victim: Recorded<Option<u32>>,
    pub victim_slot: u32,
    pub victim_unknown: u8,
    pub end_hour: Recorded<Option<i32>>,
    pub completion_country: Recorded<Option<u32>>,
    pub completion_country_slot: Recorded<u32>,
}

impl RaidEndRow {
    pub const ENDED: u8 = 0;
    pub const DELETED: u8 = 1;
    pub const VICTIM_KNOWN: u8 = 0;
    pub const TARGET_UNKNOWN: u8 = 1;
    pub const LOCATION_UNKNOWN: u8 = 2;
    pub const VICTIM_UNLINKED: u8 = 3;

    pub fn key_of(hour: i32, raid: u128) -> u128 {
        u128::from(hour as u32) << 64 | (raid & u128::from(u64::MAX))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirMissionWingRow {
    pub slot: u32,
    pub region: u32,
    pub wing: u64,
    pub ordinal: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LeaderBattleTotalRow {
    pub leader: u64,
    pub fought: u32,
    pub won: u32,
    pub lost: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LeaderTraitDateRow {
    pub leader: u64,
    pub token: u32,
    pub first_hour: i32,
    pub first_class: u8,
    pub gained_hour: Option<i32>,
    pub spelling: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LeaderCommandSpanRow {
    pub source: u8,
    pub formation: u64,
    pub leader: u64,
    pub first_hour: i32,
    pub kind: u8,
    pub name: Recorded<Option<Vec<u8>>>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LeaderCommandRow {
    pub leader: u64,
    pub hour: i32,
    pub ordinal: u32,
    pub source: u8,
    pub formation: u64,
    pub kind: u8,
    pub first_hour: i32,
    pub last_hour: i32,
    pub name: Recorded<Option<Vec<u8>>>,
}

impl LeaderCommandRow {
    pub const ORDERS_GROUP: u8 = 0;
    pub const FLEET: u8 = 1;
    pub const ARMY: u8 = 0;
    pub const ARMY_GROUP: u8 = 1;
    pub const NAVY: u8 = 2;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryExperienceDailyRow {
    pub country: u32,
    pub army_change: Recorded<i64>,
    pub navy_change: Recorded<i64>,
    pub air_change: Recorded<i64>,
    pub army_gain: Recorded<i64>,
    pub navy_gain: Recorded<i64>,
    pub air_gain: Recorded<i64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryGameScoreRow {
    pub country: u32,
    pub army: Recorded<i64>,
    pub navy: Recorded<i64>,
    pub air: Recorded<i64>,
    pub industry: Recorded<i64>,
    pub land: Recorded<i64>,
    pub total: Recorded<i64>,
    pub army_reason: u8,
    pub navy_reason: u8,
    pub air_reason: u8,
    pub industry_reason: u8,
    pub land_reason: u8,
    pub total_reason: u8,
}

impl CountryGameScoreRow {
    pub const KNOWN: u8 = 0;
    pub const INPUTS_INCOMPLETE: u8 = 1;
    pub const NO_INPUT_ROW: u8 = 2;
    pub const COUNT_UNREAD: u8 = 3;
    pub const MULTIPLIER: u8 = 4;
    pub const AIR_ENTRY: u8 = 5;
    pub const BASE_ACCESS: u8 = 6;
    pub const NO_LINK: u8 = 7;
    pub const REGISTRATIONS_INCOMPLETE: u8 = 8;
    pub const NO_REGISTRATION: u8 = 9;
    pub const WINGS_INCOMPLETE: u8 = 10;
    pub const NO_WING: u8 = 11;
    pub const PLANES: u8 = 12;
    pub const STAT: u8 = 13;
    pub const TOTAL_PARTS: u8 = 14;
    pub const FACTORIES_INCOMPLETE: u8 = 15;
    pub const NO_FACTORIES_ROW: u8 = 16;
    pub const REASONS: [&'static str; 17] = [
        "",
        "CountryGameScoreInputs was not complete",
        "CountryGameScoreInputs held no row for this country",
        "the game's count was not read (the country's CountryGameScoreInputs row names why)",
        "the score multiplier was not read (GameScoreMultipliers names why)",
        "the strategic-air entry was not read (the country's CountryGameScoreInputs row names why)",
        "an air base's access was not read (the country's CountryGameScoreInputs row names why)",
        "no link resolves the country, so its air bases' registrations cannot be joined",
        "AirBaseRegistrations was not complete",
        "an accessible air base holds no registration for the country",
        "AirWingGameScoreStats was not complete",
        "a registered air wing has no AirWingGameScoreStats row",
        "an air wing's plane count was not read (its AirWingGameScoreStats row names why)",
        "an air wing's mission stat was not read (its AirWingGameScoreStats row names why)",
        "the total needs army, navy, air and the total multiplier",
        "CountryFactories was not complete",
        "CountryFactories held no row for this country",
    ];
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StabilityPartRow {
    pub country: u32,
    pub measure: u8,
    pub handle: u32,
    pub value: Recorded<i64>,
}

impl StabilityPartRow {
    pub const STABILITY: u8 = 0;
    pub const WAR_SUPPORT: u8 = 1;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StabilityPartKeyRow {
    pub handle: u32,
    pub kind: u8,
    pub identity: Vec<u8>,
    pub identity_id: u32,
    pub secondary: i64,
    pub script: Recorded<Vec<u8>>,
    pub saved: Recorded<Vec<u8>>,
    pub template_present: Recorded<u8>,
    pub template_tag: Recorded<Vec<u8>>,
    pub template_file: Recorded<Vec<u8>>,
    pub template_line: Recorded<i32>,
    pub template_generated: Recorded<u8>,
}

impl StabilityPartKeyRow {
    pub const TERM: u8 = 9;
    pub const REMAINDER: u8 = 10;
    pub const CLAMP: u8 = 11;
    pub const TOTAL: u8 = 12;
    pub const REFUSED_LABEL: u8 = 13;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ThreatEventRow {
    pub kind: u8,
    pub slot: u32,
    pub hour: i32,
    pub ordinal: u32,
    pub country: u32,
    pub second_country: u32,
    pub date: i32,
    pub reason: Vec<u8>,
    pub final_raw: i64,
    pub place: u32,
}

impl ThreatEventRow {
    pub const LISTED: u8 = 0;
    pub const CHANGED: u8 = 1;
    pub const DROPPED: u8 = 2;
}

pub const KEY_FIELD_NONE: u32 = (1 << 24) - 1;

pub const fn key_field(value: u32) -> u128 {
    (if value == TagHolderRow::NONE { KEY_FIELD_NONE } else { value & KEY_FIELD_NONE }) as u128
}

pub const fn field_of_key(key: u128, shift: u32) -> u32 {
    let value = (key >> shift) as u32 & KEY_FIELD_NONE;
    if value == KEY_FIELD_NONE { TagHolderRow::NONE } else { value }
}

pub const fn fits_key_field(value: u32) -> bool {
    value < KEY_FIELD_NONE || value == TagHolderRow::NONE
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelDayRow {
    pub country: u32,
    pub read_hour: i32,
    pub gained: Recorded<i64>,
    pub burned: Recorded<i64>,
    pub received_army: Recorded<i64>,
    pub received_air: Recorded<i64>,
    pub received_navy: Recorded<i64>,
    pub other: Recorded<i64>,
    pub gain_base: Recorded<i64>,
    pub gain_states: Recorded<i64>,
    pub gain_oil: Recorded<i64>,
    pub gain_negative_cost: Recorded<i64>,
    pub own_gain_quantity: Recorded<i64>,
    pub incoming: Recorded<i64>,
    pub outgoing: Recorded<i64>,
    pub flat: Recorded<i64>,
    pub left_out: u8,
    pub awards_reason: Vec<u8>,
    pub continuity_start_hour: Option<i32>,
    pub overlapped: Option<i32>,
}

impl FuelDayRow {
    pub const KEPT: u8 = 0;
    pub const OVERLAPS: u8 = 1;
    pub const NOT_UPDATED: u8 = 2;
    pub const OWNERSHIP_NOT_RECORDED: u8 = 3;
    pub const RAW: [&'static str; 14] = [
        "gained",
        "burned",
        "received_army",
        "received_air",
        "received_navy",
        "other",
        "gain_base",
        "gain_states",
        "gain_oil",
        "gain_negative_cost",
        "own_gain_quantity",
        "incoming",
        "outgoing",
        "flat",
    ];

    pub fn measures(raw: [Option<i64>; 14]) -> [Option<i64>; 15] {
        let quantity = i128::from(engine_reference_values::native_values::fuel_status::QUANTITY_SCALE);
        let gain = i128::from(engine_reference_values::native_values::fuel_status::MODIFIER_SCALE);
        let wide = raw.map(|cell| cell.map(i128::from));
        let [gained, burned, army, air, navy, other, base, states, oil, negative, converted, incoming, outgoing, flat] = wide;
        let received = [army, air, navy];
        let received_sum = received.iter().try_fold(0_i128, |sum, value| value.map(|value| sum + value));
        let mut values: [Option<i128>; 15] = [None; 15];
        values[0] = gained;
        values[1] = burned;
        for (at, value) in received.into_iter().enumerate() {
            values[12 + at] = value.map(|value| (value * quantity).max(0));
        }
        if converted.is_some() {
            for (at, value) in [base, states, oil, negative].into_iter().enumerate() {
                values[2 + at] = value.map(|value| value.max(0));
            }
            values[7] = incoming.map(|value| value.max(0));
            values[8] = outgoing.map(|value| (value * gain).max(0));
            values[9] = flat.map(|value| (value * gain).max(0));
            values[10] = other.zip(outgoing.zip(flat)).map(|(other, (outgoing, flat))| ((other - outgoing - flat) * gain).max(0));
        } else {
            values[6] = gained.map(|value| value.max(0));
            values[10] = other.map(|value| (value * gain).max(0));
        }
        values[11] = burned.zip(other).zip(received_sum).map(|((burned, other), received)| (burned * gain - other * gain - received * quantity).max(0));
        values.map(|value| value.and_then(|value| i64::try_from(value).ok()))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelDemandTotalRow {
    pub country: u32,
    pub native_army: Recorded<i64>,
    pub native_air: Recorded<i64>,
    pub native_navy: Recorded<i64>,
    pub known_army: Recorded<i64>,
    pub known_air: Recorded<i64>,
    pub known_navy: Recorded<i64>,
    pub positive_army: Recorded<i64>,
    pub positive_air: Recorded<i64>,
    pub positive_navy: Recorded<i64>,
    pub army_reason: Vec<u8>,
    pub air_reason: Vec<u8>,
    pub navy_reason: Vec<u8>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TradeFlowRow {
    pub seller_slot: u32,
    pub seller: u32,
    pub buyer_slot: u32,
    pub buyer: u32,
    pub resource: u32,
    pub delivered: Recorded<i64>,
    pub factories: Recorded<i64>,
    pub convoys_assigned: Recorded<i64>,
    pub convoys_required: Recorded<i64>,
    pub deals: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ConstructionProjectRow {
    pub line: u64,
    pub kind: u8,
    pub output_alone: bool,
    pub country: u32,
    pub building_type: Option<u32>,
    pub target_state: Option<u32>,
    pub to_state: Option<u32>,
    pub province: Option<u32>,
    pub route_provinces: Option<u32>,
    pub base_level: Option<i32>,
    pub target_level: Option<i32>,
    pub cost: Option<i64>,
    pub created_hour: Option<i32>,
    pub first_read: i32,
    pub census_read: Option<i32>,
    pub queue_ordinal: Option<u32>,
    pub earlier_unknown: bool,
    pub factories: Recorded<i32>,
    pub remaining: Recorded<i32>,
    pub repair_levels: Recorded<i32>,
    pub built: i64,
    pub built_rises: Vec<ConstructionRise>,
    pub repaired: i64,
    pub repaired_rises: Vec<ConstructionRise>,
    pub added_new: i64,
    pub added_repair: i64,
    pub possible: i64,
    pub unaccounted_new: i64,
    pub unaccounted_repair: i64,
    pub end: u8,
    pub end_reason: u8,
    pub end_hour: Option<i32>,
    pub end_known_at: Option<i32>,
    pub construction_raw: Recorded<i64>,
    pub repair_raw: Recorded<i64>,
    pub output_since: Option<i32>,
    pub observed_since_creation: bool,
    pub output_reason: u8,
    pub first_repair_tick: Option<i32>,
}


#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ConstructionLineEndRow {
    pub hour: i32,
    pub line: u64,
    pub kind: u8,
    pub output_alone: bool,
    pub country: u32,
    pub building_type: Option<u32>,
    pub target_state: Option<u32>,
    pub to_state: Option<u32>,
    pub first_read: i32,
    pub new_work: bool,
    pub repair_work: bool,
    pub before_observation: bool,
    pub had_output: bool,
    pub output_reason: u8,
}

impl ConstructionLineEndRow {
    pub const fn key_of(hour: i32, line: u64) -> u128 {
        (hour as u32 as u128) << 64 | line as u128
    }

    pub fn of(hour: i32, row: &ConstructionProjectRow) -> Self {
        let positive = |cell: &Recorded<i32>| matches!(cell, Recorded::Value(count) if *count > 0);
        let wide = |cell: &Recorded<i64>| matches!(cell, Recorded::Value(amount) if *amount > 0);
        Self {
            hour,
            line: row.line,
            kind: row.kind,
            output_alone: row.output_alone,
            country: row.country,
            building_type: row.building_type,
            target_state: row.target_state,
            to_state: row.to_state,
            first_read: row.first_read,
            new_work: [row.built, row.added_new, row.possible, row.unaccounted_new].into_iter().any(|count| count > 0) || positive(&row.remaining) || wide(&row.construction_raw),
            repair_work: [row.repaired, row.added_repair, row.unaccounted_repair].into_iter().any(|count| count > 0)
                || positive(&row.repair_levels)
                || wide(&row.repair_raw)
                || row.first_repair_tick.is_some(),
            before_observation: row.output_since.is_some() && !row.observed_since_creation,
            had_output: row.output_since.is_some(),
            output_reason: row.output_reason,
        }
    }
}

impl ConstructionProjectRow {
    pub const BUILDING: u8 = 0;
    pub const RAILWAY: u8 = 1;
    pub const OPEN: u8 = 0;
    pub const FINISHED: u8 = 1;
    pub const REMOVED: u8 = 2;
    pub const UNKNOWN_END: u8 = 3;
    pub const NOT_RECORDED: u8 = 1;
    pub const SHARED_TARGET: u8 = 2;
    pub const NOTHING_LEFT: u8 = 3;
    pub const COMPLETION_NOT_RECORDED: u8 = 4;
    pub const OUTPUT_KNOWN: u8 = 0;
    pub const OUTPUT_NOT_RECORDED: u8 = 1;
    pub const OUTPUT_LINKED: u8 = 2;
    pub const OUTPUT_INEXACT: u8 = 3;
    pub const OUTPUT_RESTARTED: u8 = 4;
    pub const OUTPUT_FINAL_MISSING: u8 = 5;
    pub const OUTPUT_OWNER_UNKNOWN: u8 = 6;
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ConstructionSpendRow {
    pub slot: u32,
    pub country: u32,
    pub repair: bool,
    pub railway: bool,
    pub building_type: u32,
    pub state: u32,
    pub to_state: u32,
    pub total: Recorded<i64>,
    pub unknown: i64,
}

impl Default for ConstructionSpendRow {
    fn default() -> Self {
        Self {
            slot: 0,
            country: 0,
            repair: false,
            railway: false,
            building_type: 0,
            state: 0,
            to_state: TagHolderRow::NONE,
            total: Recorded::default(),
            unknown: 0,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionEventRow {
    pub kind: u8,
    pub hour: i32,
    pub division: u64,
    pub disposition: Option<u32>,
    pub cause: Option<u32>,
    pub owner: u32,
    pub slot: u32,
    pub native_hour: i32,
    pub exact: bool,
    pub after: Option<i32>,
    pub province: Recorded<u32>,
    pub place_hour: Recorded<i32>,
    pub template: Recorded<u64>,
    pub template_hour: Recorded<i32>,
    pub name: Recorded<Option<Vec<u8>>>,
    pub naming: Recorded<Option<NameOperands>>,
    pub name_hour: Recorded<i32>,
    pub icon_unit: Recorded<u32>,
    pub icon_custom: Recorded<i32>,
    pub icon_name: Recorded<Vec<u8>>,
    pub icon_hour: Recorded<i32>,
    pub killer_country: Option<u32>,
    pub killer_slot: Option<u32>,
    pub killer_division: Option<u64>,
    pub template_id: Recorded<u32>,
    pub matched_deaths: Option<u32>,
    pub template_literal: Recorded<Vec<u8>>,
    pub template_key: Recorded<Vec<u8>>,
    pub template_country: Recorded<i32>,
}

impl DivisionEventRow {
    pub const CREATED: u8 = 0;
    pub const LOST: u8 = 1;
    pub const TRANSFER: u8 = 2;
    pub const VANISHED: u8 = 3;
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LeaderBattleRow {
    pub leader: u64,
    pub hour: i32,
    pub ordinal: u32,
    pub entry: u8,
    pub kind: u8,
    pub combat: u64,
    pub sides: u8,
    pub start_hour: i32,
    pub start_unknown: bool,
    pub winners: Recorded<u8>,
    pub result_hour: Recorded<i32>,
    pub outcome: Recorded<u8>,
}

impl Default for LeaderBattleRow {
    fn default() -> Self {
        Self {
            leader: 0,
            hour: 0,
            ordinal: 0,
            entry: Self::PARTICIPATION,
            kind: Self::LAND,
            combat: 0,
            sides: 0,
            start_hour: 0,
            start_unknown: false,
            winners: Recorded::default(),
            result_hour: Recorded::default(),
            outcome: Recorded::default(),
        }
    }
}

impl LeaderBattleRow {
    pub const PARTICIPATION: u8 = 0;
    pub const RESULT: u8 = 1;
    pub const LAND: u8 = 1;
    pub const NAVAL: u8 = 2;
    pub const WON: u8 = 1;
    pub const LOST: u8 = 2;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LeaderTraitGainRow {
    pub leader: u64,
    pub hour: i32,
    pub token: u32,
    pub class: u8,
    pub spelling: Vec<u8>,
}

impl LeaderTraitGainRow {
    pub const HELD_BEFORE: u8 = 0;
    pub const GAINED: u8 = 1;
    pub const FIRST_OBSERVED: u8 = 2;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResearchAssignmentRow {
    pub holder_slot: u32,
    pub holder_id: u32,
    pub index: u32,
    pub lane: u16,
    pub technology_handle: u32,
    pub database_index: i32,
    pub token: Vec<u8>,
    pub level: i32,
    pub continuity_start_hour: i32,
    pub first_observed_hour: i32,
    pub started_after_hour: Option<i32>,
    pub last_observed_hour: Option<i32>,
    pub ended_by_hour: Option<i32>,
    pub outcome: u8,
    pub outcome_observed_hour: Option<i32>,
    pub awaiting: u8,
}

impl ResearchAssignmentRow {
    pub const ACTIVE: u8 = 0;
    pub const COMPLETED: u8 = 1;
    pub const SWITCHED: u8 = 2;
    pub const UNKNOWN: u8 = 3;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResearchCoverageRow {
    pub holder_slot: u32,
    pub holder_id: u32,
    pub member: u32,
    pub since: i32,
}

impl ResearchCoverageRow {
    pub const HOLDER: u32 = u32::MAX;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionAccountOwnerRow {
    pub account: u32,
    pub faction: u64,
    pub slot: u32,
    pub id: u32,
    pub member: u8,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PeaceOutcomeRow {
    pub after_hour: i32,
    pub ordinal: u32,
    pub observation: u8,
    pub conference: Option<u64>,
    pub before_hour: i32,
    pub change: u8,
    pub group: u32,
    pub state: u32,
    pub from_slot: u32,
    pub from_id: u32,
    pub to_slot: u32,
    pub to_id: u32,
    pub victory_points: Option<i64>,
}

impl PeaceOutcomeRow {
    pub const RECORDED: u8 = 0;
    pub const NOT_RECORDED: u8 = 1;
    pub const UNKNOWN: u8 = 2;
    pub const TRANSFER: u8 = 0;
    pub const NEW_SUBJECT: u8 = 1;
    pub const GOVERNMENT: u8 = 2;
    pub const RELEASED: u8 = 3;
    pub const NONE: u32 = u32::MAX;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PeaceConferenceSpanRow {
    pub conference: u64,
    pub hour: i32,
    pub first_observed_hour: i32,
    pub last_observed_hour: i32,
    pub current_turn: i32,
    pub finalized: u8,
    pub first_finalized_hour: Option<i32>,
    pub first_absent_hour: Option<i32>,
    pub comparison: u8,
    pub before_hour: Option<i32>,
    pub compared_hour: Option<i32>,
}

impl PeaceConferenceSpanRow {
    pub const WAITING: u8 = 0;
    pub const COMPARED: u8 = 1;
    pub const NO_BEFORE: u8 = 2;
    pub const OVERLAPPED: u8 = 3;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CasualtyLifetimeRow {
    pub slot_a: u32,
    pub id_a: u32,
    pub slot_b: u32,
    pub id_b: u32,
    pub start_hour: i32,
    pub taken_a: i64,
    pub taken_b: i64,
    pub unknown_a: i64,
    pub unknown_b: i64,
}

impl CasualtyLifetimeRow {
    pub const FIELD: u32 = (1 << 24) - 1;
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CasualtyTotalRow {
    pub slot: u32,
    pub id: u32,
    pub dealt: Recorded<i64>,
    pub taken: Recorded<i64>,
    pub unknown: Recorded<i64>,
    pub partial: u8,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OccupierTotalRow {
    pub slot: u32,
    pub id: u32,
    pub resistance_sum: Recorded<i64>,
    pub compliance_sum: Recorded<i64>,
    pub states: Recorded<i64>,
    pub held_states: Recorded<i64>,
    pub held_complete: u8,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirMissionCountRow {
    pub slot: u32,
    pub region: u32,
    pub wings: i64,
    pub mission_0: Recorded<i32>,
    pub mission_1: Recorded<i32>,
    pub mission_2: Recorded<i32>,
    pub mission_3: Recorded<i32>,
    pub mission_4: Recorded<i32>,
    pub mission_5: Recorded<i32>,
    pub mission_6: Recorded<i32>,
    pub mission_7: Recorded<i32>,
    pub mission_8: Recorded<i32>,
    pub mission_9: Recorded<i32>,
    pub mission_10: Recorded<i32>,
    pub mission_11: Recorded<i32>,
    pub mission_12: Recorded<i32>,
    pub mission_13: Recorded<i32>,
    pub mission_14: Recorded<i32>,
    pub mission_15: Recorded<i32>,
    pub mission_16: Recorded<i32>,
    pub mission_17: Recorded<i32>,
}

impl AirMissionCountRow {
    pub const MISSIONS: usize = 18;

    pub fn count(totals: &mut [i32; Self::MISSIONS], mask: u32, planes: i32) {
        if mask as i32 <= 0 {
            return;
        }
        for (bit, total) in totals.iter_mut().enumerate() {
            if mask & (1 << bit) != 0 {
                *total = (*total).max(0).wrapping_add(planes);
            }
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirRegionWingRow {
    pub region: u32,
    pub wings: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirRegionDayRow {
    pub region: u32,
    pub day: i32,
    pub activity: i64,
    pub activity_total: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TerritoryRegionTotalRow {
    pub holder: u32,
    pub region: u32,
    pub controlled_provinces: Recorded<i64>,
    pub controlled_victory_points: Recorded<i64>,
    pub controlled_area: Recorded<i64>,
    pub controlled_states: Recorded<i64>,
    pub controlled_states_denominator: Recorded<i64>,
    pub owned_provinces: Recorded<i64>,
    pub owned_victory_points: Recorded<i64>,
    pub owned_area: Recorded<i64>,
    pub owned_states: Recorded<i64>,
    pub owned_states_denominator: Recorded<i64>,
}

impl TerritoryRegionTotalRow {
    pub const NO_COUNTRY: u32 = u32::MAX;
    pub const NO_REGION: u32 = u32::MAX;

    pub const fn key_of(holder: u32, region: u32) -> u128 {
        (holder as u128) << 32 | region as u128
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProducedValueRow {
    pub country: u32,
    pub archetype: u32,
    pub units: Recorded<i64>,
    pub units_reason: u8,
    pub units_without_refits: Recorded<i64>,
    pub units_without_refits_reason: u8,
    pub ic: Recorded<i64>,
    pub ic_reason: u8,
    pub ic_without_refits: Recorded<i64>,
    pub ic_without_refits_reason: u8,
    pub since: Option<i32>,
    pub as_of: Option<i32>,
}

impl ProducedValueRow {
    pub const WHOLE: u32 = u32::MAX;
    pub const REST: u32 = u32::MAX - 1;
    pub const CLOCK: u32 = u32::MAX;

    pub const KNOWN: u8 = 0;
    pub const NOT_RECORDED: u8 = 1;
    pub const UNITS_INEXACT: u8 = 2;
    pub const SPENDING_UNAVAILABLE: u8 = 3;
    pub const LOST_OUTPUT: u8 = 4;
    pub const MISSED_PUBLICATION: u8 = 5;
    pub const TYPE_NOT_RECORDED: u8 = 6;
    pub const ROCKET_TABLE_UNAVAILABLE: u8 = 7;
    pub const ROCKET_INPUT_INEXACT: u8 = 8;
    pub const ROCKET_OWNER_UNREADABLE: u8 = 9;
    pub const OUT_OF_RANGE: u8 = 10;

    pub const fn key_of(country: u32, archetype: u32) -> u128 {
        (country as u128) << 32 | archetype as u128
    }

    pub const fn clock() -> u128 {
        Self::key_of(Self::CLOCK, 0)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirLossTotalRow {
    pub slot: u32,
    pub id: u32,
    pub region: u32,
    pub cause: u32,
    pub dealt_known: Recorded<i64>,
    pub dealt_unknown: Recorded<i64>,
    pub taken_known: Recorded<i64>,
    pub taken_unknown: Recorded<i64>,
    pub late: i32,
}

impl AirLossTotalRow {
    pub const NONE: u32 = u32::MAX;

    pub const fn key_of(slot: u32, id: u32, region: u32, cause: u32) -> u128 {
        (slot as u128) << 96 | (id as u128) << 64 | (region as u128) << 32 | cause as u128
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct BombingTotalRow {
    pub slot: u32,
    pub id: u32,
    pub cause: u8,
    pub dealt: Recorded<i64>,
    pub taken: Recorded<i64>,
    pub late: i32,
}

impl BombingTotalRow {
    pub const NONE: u32 = u32::MAX;

    pub const fn key_of(slot: u32, id: u32, cause: u8) -> u128 {
        (slot as u128) << 40 | (id as u128) << 8 | cause as u128
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GarrisonStrengthRow {
    pub controller: u32,
    pub state: u32,
    pub strength: Recorded<i64>,
    pub reason: u8,
}

impl GarrisonStrengthRow {
    pub const KNOWN: u8 = 0;
    pub const UNRESOLVED: u8 = 1;
    pub const ASSIGNED: u8 = 2;
    pub const OPERANDS: u8 = 3;
    pub const FAILED: u8 = 4;

    pub const fn key_of(controller: u32, state: u32) -> u128 {
        (controller as u128) << 32 | state as u128
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirOwnerDayRow {
    pub country: u32,
    pub region: u32,
    pub day: i32,
    pub friendly: i64,
    pub enemy: i64,
    pub friendly_total: i64,
    pub enemy_total: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirTheatreRegionRow {
    pub theatre: u64,
    pub region: u32,
    pub wings: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirRegionActivityRow {
    pub region: u32,
    pub day: i32,
    pub friendly: i64,
    pub enemy: i64,
    pub friendly_total: i64,
    pub enemy_total: i64,
}

row_codec!(TagHolderRow, 128, |row| (row.number as u128) << 96 | (row.epoch as u128) << 64 | (row.slot as u128) << 32 | row.id as u128, |key| { number: (key >> 96) as u32, epoch: (key >> 64) as u32, slot: (key >> 32) as u32, id: key as u32 }, {0 => tag: Bytes}, |_row| Ok(()));
row_codec!(GroupSpanRow, 104, |row| (row.kind as u128) << 96 | (row.group as u128) << 64 | (row.value as u128) << 32 | row.epoch as u128, |key| { kind: (key >> 96) as u8, group: (key >> 64) as u32, value: (key >> 32) as u32, epoch: key as u32 }, {0 => ordinal: U32}, |_row| Ok(()));
row_codec!(GroupRow, 32, |row| row.number as u128, |key| { number: key as u32 }, {0 => faction: U64}, |_row| Ok(()));
row_codec!(GroupNameRow, 32, |row| row.number as u128, |key| { number: key as u32 }, {0 => literal: Bytes, 1 => template_token: OptionalU32, 2 => spelling: Bytes}, |_row| Ok(()));
row_codec!(WarEventRow, 104, |row| (row.kind as u128) << 96 | (row.hour as u32 as u128) << 64 | (row.first as u128) << 32 | row.second as u128, |key| { kind: (key >> 96) as u8, hour: (key >> 64) as u32 as i32, first: (key >> 32) as u32, second: key as u32 }, {0 => read: U8}, |_row| Ok(()));
row_codec!(LookSpanRow, 96, |row| (row.number as u128) << 64 | (row.fact as u128) << 32 | row.epoch as u128, |key| { number: (key >> 64) as u32, fact: (key >> 32) as u32, epoch: key as u32 }, {0 => read: U8}, |_row| Ok(()));
row_codec!(LookFactRow, 32, |row| row.number as u128, |key| { number: key as u32 }, {0 => look: Bytes}, |_row| Ok(()));
row_codec!(BalanceDropRow, 72, |row| (row.kind as u128) << 64 | (row.country as u128) << 32 | row.hour as u32 as u128, |key| { kind: (key >> 64) as u8, country: (key >> 32) as u32, hour: key as u32 as i32 }, {0 => before: I64, 1 => after: I64}, |_row| Ok(()));
row_codec!(TagNameRow, 32, |row| row.number as u128, |key| { number: key as u32 }, {0 => tag: Bytes}, |_row| Ok(()));
row_codec!(TerritoryTotalRow, 32, |row| row.slot as u128, |key| { slot: key as u32 }, {0 => owned_states: I64, 1 => controlled_states: I64, 2 => occupied_states: I64, 3 => owned_provinces: I64, 4 => controlled_provinces: I64, 5 => occupied_provinces: I64, 6 => owned_victory_points: I64, 7 => controlled_victory_points: I64, 8 => occupied_victory_points: I64, 9 => owned_area: I64, 10 => controlled_area: I64, 11 => occupied_area: I64}, |_row| Ok(()));
row_codec!(CountryStabilityRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => stability: I64, 1 => war_support: I64, 2 => max_command_power: I64}, |_row| Ok(()));
row_codec!(ArmyTotalRow, 32, |row| row.slot as u128, |key| { slot: key as u32 }, {0 => owner_divisions: I64, 1 => owner_strength_raw: I64, 2 => host_divisions: I64, 3 => host_strength_raw: I64, 4 => owner_battalions: I64, 5 => host_battalions: I64, 6 => owner_manpower: I64, 7 => host_manpower: I64}, |_row| Ok(()));
row_codec!(ArmyMixRow, 72, |row| ((row.slot as u128) << 40) | ((row.attribution as u128) << 32) | row.token as u128, |key| { slot: (key >> 40) as u32, attribution: (key >> 32) as u8, token: key as u32 }, {0 => battalions: I64}, |_row| Ok(()));
row_codec!(AirForceTotalRow, 32, |row| row.slot as u128, |key| { slot: key as u32 }, {0 => roster_planes: I64, 1 => roster_wings: I64, 2 => effective_planes: I64, 3 => effective_wings: I64}, |_row| Ok(()));
row_codec!(AirForcePartRow, 105, |row| ((row.slot as u128) << 73) | ((row.ownership as u128) << 65) | match row.variant { Some(variant) => (1 << 64) | variant as u128, None => 0 }, |key| { slot: (key >> 73) as u32, ownership: (key >> 65) as u8, variant: if (key >> 64) & 1 == 1 { Some(key as u64) } else { None } }, {0 => planes: I64}, |_row| Ok(()));
row_codec!(AirForceWingKindRow, 72, |row| ((row.slot as u128) << 40) | ((row.ownership as u128) << 32) | row.signature as u128, |key| { slot: (key >> 40) as u32, ownership: (key >> 32) as u8, signature: key as u32 }, {0 => wings: I64}, |_row| Ok(()));
row_codec!(AirForceWingSignatureRow, 32, |row| row.number as u128, |key| { number: key as u32 }, {0 => variants: Bytes}, |_row| Ok(()));
row_codec!(NavyTotalRow, 32, |row| row.slot as u128, |key| { slot: key as u32 }, {0 => roster_ships: I64, 1 => roster_strength_raw: I64, 2 => roster_max_strength_raw: I64, 3 => logical_ships: I64, 4 => logical_strength_raw: I64, 5 => logical_max_strength_raw: I64}, |_row| Ok(()));
row_codec!(NavyClassRow, 72, |row| ((row.slot as u128) << 40) | ((row.ownership as u128) << 32) | row.class as u128, |key| { slot: (key >> 40) as u32, ownership: (key >> 32) as u8, class: key as u32 }, {0 => ships: I64, 1 => strength_raw: I64}, |_row| Ok(()));
row_codec!(CountryDeployedManpowerRow, 32, |row| row.slot as u128, |key| { slot: key as u32 }, {0 => army: I32, 1 => navy: I32, 2 => navy_required: I32, 3 => air: I32, 4 => exile_air: I32}, |_row| Ok(()));
row_codec!(MilitaryAidRow, 72, |row| (row.sender as u128) << 40 | (row.host as u128) << 8 | row.channel as u128, |key| { sender: (key >> 40) as u32, host: (key >> 8) as u32, channel: key as u8 }, {0 => divisions: I64}, |row| if row.channel > MilitaryAidRow::TALLIES { Err(invalid("a military aid row's channel is past tallies")) } else { Ok(()) });
row_codec!(SupplyHealthRow, 32, |row| row.slot as u128, |key| { slot: key as u32 }, {0 => no_supply: I64, 1 => strained: I64, 2 => in_supply: I64}, |_row| Ok(()));
row_codec!(CountryExperienceDailyRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => army_change: I64, 1 => navy_change: I64, 2 => air_change: I64, 3 => army_gain: I64, 4 => navy_gain: I64, 5 => air_gain: I64}, |_row| Ok(()));
row_codec!(CountryGameScoreRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => army: I64, 1 => navy: I64, 2 => air: I64, 3 => industry: I64, 4 => land: I64, 5 => total: I64, 6 => army_reason: U8, 7 => navy_reason: U8, 8 => air_reason: U8, 9 => industry_reason: U8, 10 => land_reason: U8, 11 => total_reason: U8}, |_row| Ok(()));
row_codec!(StabilityPartRow, 72, |row| (row.country as u128) << 40 | (row.measure as u128) << 32 | row.handle as u128, |key| { country: (key >> 40) as u32, measure: (key >> 32) as u8, handle: key as u32 }, {0 => value: I64}, |row| if row.measure > StabilityPartRow::WAR_SUPPORT { Err(invalid("a stability part's measure is past war support")) } else { Ok(()) });
row_codec!(StabilityPartKeyRow, 32, |row| row.handle as u128, |key| { handle: key as u32 }, {0 => kind: U8, 1 => identity: Bytes, 2 => identity_id: U32, 3 => secondary: I64, 4 => script: Bytes, 5 => saved: Bytes, 6 => template_present: U8, 7 => template_tag: Bytes, 8 => template_file: Bytes, 9 => template_line: I32, 10 => template_generated: U8}, |row| if row.kind > StabilityPartKeyRow::REFUSED_LABEL { Err(invalid("a stability part's kind is past a refused label")) } else { Ok(()) });
row_codec!(ThreatEventRow, 104, |row| (row.kind as u128) << 96 | (row.slot as u128) << 64 | (row.hour as u32 as u128) << 32 | row.ordinal as u128, |key| { kind: (key >> 96) as u8, slot: (key >> 64) as u32, hour: (key >> 32) as u32 as i32, ordinal: key as u32 }, {0 => country: U32, 1 => second_country: U32, 2 => date: I32, 3 => reason: Bytes, 4 => final_raw: I64, 5 => place: U32}, |_row| Ok(()));
row_codec!(EquipmentLossTotalRow, 105, |row| (row.slot as u128) << 73 | (row.country as u128) << 41 | (row.source as u128) << 33 | match row.variant { Some(variant) => 1 << 32 | variant as u128, None => 0 }, |key| { slot: (key >> 73) as u32, country: (key >> 41) as u32, source: (key >> 33) as u8, variant: if (key >> 32) & 1 == 1 { Some(key as u32) } else { None } }, {0 => total: I64, 1 => incomplete_from: OptionalI32, 2 => freed: I64}, |_row| Ok(()));
row_codec!(CombatLogDroppedRow, 120, |row| (row.country as u128) << 88 | (row.group as u128) << 56 | (row.kind as u128) << 48 | (row.month as u128) << 32 | row.variant as u128, |key| { country: (key >> 88) as u32, group: (key >> 56) as u32, kind: (key >> 48) as u8, month: (key >> 32) as u16, variant: key as u32 }, {0 => date: I32, 1 => amount: I64}, |_row| Ok(()));
row_codec!(ShipLossRow, 128, |row| (row.slot as u128) << 104 | (row.country as u128) << 80 | (row.hour as u32 as u128) << 48 | row.signature as u128, |key| { slot: (key >> 104) as u32 & ShipLossRow::FIELD, country: (key >> 80) as u32 & ShipLossRow::FIELD, hour: (key >> 48) as u32 as i32, signature: key as u64 & ShipLossRow::SIGNATURE }, {0 => sink_hour: I32, 1 => variant: U64, 2 => count: U32, 3 => check: U64}, |row| if row.slot > ShipLossRow::FIELD || row.country > ShipLossRow::FIELD || row.signature > ShipLossRow::SIGNATURE { Err(invalid("a ship loss's slot or country passes 24 bits, or its signature 48")) } else { Ok(()) });
row_codec!(EquipmentLossCoverageRow, 64, |row| (row.slot as u128) << 32 | row.country as u128, |key| { slot: (key >> 32) as u32, country: key as u32 }, {0 => land: U8, 1 => ungrouped: U32, 2 => resolved: U8, 3 => convoy_drops: U32, 4 => drain: Bytes, 5 => unread: U8, 6 => counted_from: OptionalI32, 7 => withheld_from: OptionalI32, 8 => incomplete_from: OptionalI32, 9 => convoys_from: OptionalI32, 10 => lend_from: OptionalI32, 11 => exchanges_ended: U32, 12 => ended_from: OptionalI32, 13 => unresolved: U32, 14 => unresolved_all: U32, 15 => drain_state: U8}, |row| if matches!(row.drain_state, Recorded::Value(state) if state & !EquipmentLossCoverageRow::DRAIN_STATE_MASK != 0) { Err(invalid("equipment loss coverage drain state has unsupported bits")) } else { Ok(()) });
row_codec!(StockByVariantRow, 96, |row| (row.slot as u128) << 64 | row.variant as u128, |key| { slot: (key >> 64) as u32, variant: key as u64 }, {0 => amount: I64, 1 => positions: I64}, |_row| Ok(()));
row_codec!(DivisionEquipmentCountRow, 40, |row| (row.slot as u128) << 8 | row.side as u128, |key| { slot: (key >> 8) as u32, side: key as u8 }, {0 => divisions: I64, 1 => short: I64, 2 => unknown: I64, 3 => unpooled: I64, 4 => unclassified: I64, 5 => unresolved: I64, 6 => pool_hour: OptionalI32, 7 => stock_hour: OptionalI32}, |row| if row.side > DivisionEquipmentCountRow::HOST { Err(invalid("a division equipment row's side is past host")) } else { Ok(()) });
row_codec!(DivisionEquipmentTypeRow, 73, |row| (row.slot as u128) << 41 | (row.side as u128) << 33 | match row.archetype { Some(token) => 1 << 32 | token as u128, None => 0 }, |key| { slot: (key >> 41) as u32, side: (key >> 33) as u8, archetype: if (key >> 32) & 1 == 1 { Some(key as u32) } else { None } }, {0 => held: I64, 1 => needed: I64, 2 => filled: I64, 3 => missing: I64, 4 => short: I64, 5 => stock: I64}, |row| if row.side > DivisionEquipmentCountRow::HOST { Err(invalid("a division equipment row's side is past host")) } else { Ok(()) });
row_codec!(DivisionEquipmentDesignRow, 106, |row| (row.slot as u128) << 74 | (row.side as u128) << 66 | (match row.archetype { Some(token) => 1 << 32 | token as u128, None => 0 }) << 33 | match row.variant { Some(id) => 1 << 32 | id as u128, None => 0 }, |key| { slot: (key >> 74) as u32, side: (key >> 66) as u8, archetype: if (key >> 65) & 1 == 1 { Some((key >> 33) as u32) } else { None }, variant: if (key >> 32) & 1 == 1 { Some(key as u32) } else { None } }, {0 => held: I64}, |row| if row.side > DivisionEquipmentCountRow::HOST { Err(invalid("a division equipment row's side is past host")) } else { Ok(()) });
row_codec!(DivisionEquipmentTemplateRow, 104, |row| (row.slot as u128) << 72 | (row.side as u128) << 64 | row.template as u128, |key| { slot: (key >> 72) as u32, side: (key >> 64) as u8, template: key as u64 }, {0 => divisions: I64, 1 => unknown: I64, 2 => types_short: I64, 3 => manpower: I64, 4 => manpower_lacking: I64, 5 => required: I64, 6 => hp: I64, 7 => hp_max: I64, 8 => hp_max_lacking: I64, 9 => needed: I64, 10 => held: I64, 11 => need_missing: Bytes}, |row| if row.side > DivisionEquipmentCountRow::HOST { Err(invalid("a division equipment row's side is past host")) } else { Ok(()) });
row_codec!(FuelDayRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => read_hour: I32, 1 => gained: I64, 2 => burned: I64, 3 => received_army: I64, 4 => received_air: I64, 5 => received_navy: I64, 6 => other: I64, 7 => gain_base: I64, 8 => gain_states: I64, 9 => gain_oil: I64, 10 => gain_negative_cost: I64, 11 => own_gain_quantity: I64, 12 => incoming: I64, 13 => outgoing: I64, 14 => flat: I64, 15 => left_out: U8, 16 => awards_reason: Bytes, 17 => continuity_start_hour: OptionalI32, 18 => overlapped: OptionalI32}, |row| if row.left_out > FuelDayRow::OWNERSHIP_NOT_RECORDED { Err(invalid("a fuel day's left-out code is past ownership not recorded")) } else { Ok(()) });
row_codec!(FuelDemandTotalRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => native_army: I64, 1 => native_air: I64, 2 => native_navy: I64, 3 => known_army: I64, 4 => known_air: I64, 5 => known_navy: I64, 6 => positive_army: I64, 7 => positive_air: I64, 8 => positive_navy: I64, 9 => army_reason: Bytes, 10 => air_reason: Bytes, 11 => navy_reason: Bytes}, |_row| Ok(()));
row_codec!(TradeFlowRow, 128, |row| key_field(row.seller_slot) << 104 | key_field(row.seller) << 80 | key_field(row.buyer_slot) << 56 | key_field(row.buyer) << 32 | row.resource as u128, |key| { seller_slot: field_of_key(key, 104), seller: field_of_key(key, 80), buyer_slot: field_of_key(key, 56), buyer: field_of_key(key, 32), resource: key as u32 }, {0 => delivered: I64, 1 => factories: I64, 2 => convoys_assigned: I64, 3 => convoys_required: I64, 4 => deals: I64}, |row| if [row.seller_slot, row.seller, row.buyer_slot, row.buyer].into_iter().all(fits_key_field) { Ok(()) } else { Err(invalid("a trade flow's country is past its 24-bit key field")) });
row_codec!(ConstructionProjectRow, 64, |row| row.line as u128, |key| { line: key as u64 }, {0 => kind: U8, 1 => output_alone: Bool, 2 => country: U32, 3 => building_type: OptionalU32, 4 => target_state: OptionalU32, 5 => to_state: OptionalU32, 6 => province: OptionalU32, 7 => route_provinces: OptionalU32, 8 => base_level: OptionalI32, 9 => target_level: OptionalI32, 10 => cost: OptionalI64, 11 => created_hour: OptionalI32, 12 => first_read: I32, 13 => census_read: OptionalI32, 14 => queue_ordinal: OptionalU32, 15 => earlier_unknown: Bool, 16 => factories: I32, 17 => remaining: I32, 18 => repair_levels: I32, 19 => built: I64, 20 => built_rises: Opaque, 21 => repaired: I64, 22 => repaired_rises: Opaque, 23 => added_new: I64, 24 => added_repair: I64, 25 => possible: I64, 26 => unaccounted_new: I64, 27 => unaccounted_repair: I64, 28 => end: U8, 29 => end_reason: U8, 30 => end_hour: OptionalI32, 31 => end_known_at: OptionalI32, 32 => construction_raw: I64, 33 => repair_raw: I64, 34 => output_since: OptionalI32, 35 => observed_since_creation: Bool, 36 => output_reason: U8, 37 => first_repair_tick: OptionalI32}, |row| if row.kind > ConstructionProjectRow::RAILWAY || row.end > ConstructionProjectRow::UNKNOWN_END { Err(invalid("a construction project's kind or ending is past its codes")) } else { Ok(()) });
row_codec!(ConstructionLineEndRow, 96, |row| ConstructionLineEndRow::key_of(row.hour, row.line), |key| { hour: (key >> 64) as u32 as i32, line: key as u64 }, {0 => kind: U8, 1 => output_alone: Bool, 2 => country: U32, 3 => building_type: OptionalU32, 4 => target_state: OptionalU32, 5 => to_state: OptionalU32, 6 => first_read: I32, 7 => new_work: Bool, 8 => repair_work: Bool, 9 => before_observation: Bool, 10 => had_output: Bool, 11 => output_reason: U8}, |row| if row.kind > ConstructionProjectRow::RAILWAY { Err(invalid("a construction line end's kind is past its codes")) } else { Ok(()) });
row_codec!(ConstructionSpendRow, 114, |row| key_field(row.slot) << 90 | key_field(row.country) << 66 | (row.repair as u128) << 65 | (row.railway as u128) << 64 | (row.state as u128) << 32 | (if row.railway { row.to_state } else { row.building_type }) as u128, |key| { slot: field_of_key(key, 90), country: field_of_key(key, 66), repair: (key >> 65) & 1 == 1, railway: (key >> 64) & 1 == 1, building_type: if (key >> 64) & 1 == 1 { TagHolderRow::NONE } else { key as u32 }, state: (key >> 32) as u32, to_state: if (key >> 64) & 1 == 1 { key as u32 } else { TagHolderRow::NONE } }, {0 => total: I64, 1 => unknown: I64}, |row| if [row.slot, row.country].into_iter().all(fits_key_field) && (row.railway || row.to_state == TagHolderRow::NONE) && (!row.railway || row.building_type == TagHolderRow::NONE) { Ok(()) } else { Err(invalid("a construction spend key holds a slot or country past 24 bits, or a line both a building type and a railway's last state")) });
row_codec!(ProductionTotalRow, 104, |row| (row.country as u128) << 72 | (row.variant as u128) << 8 | row.purpose as u128, |key| { country: (key >> 72) as u32, variant: (key >> 8) as u64, purpose: key as u8 }, {0 => units: I64, 1 => inexact_rows: U32, 2 => spent: I64, 3 => spent_reason: U8, 4 => unproven: I64, 5 => equipment_token: Bytes, 6 => archetype: U32}, |row| if OutputPurpose::from_raw(row.purpose).is_none() { Err(invalid("a production total's purpose is invalid")) } else { Ok(()) });
row_codec!(ProductionCountryTotalRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => units: I64, 1 => units_inexact: U32, 2 => spent: I64, 3 => units_without_refits: I64, 4 => units_without_refits_inexact: U32, 5 => spent_without_refits: I64}, |_row| Ok(()));
row_codec!(ProductionTypeTotalRow, 64, |row| (row.country as u128) << 32 | row.archetype as u128, |key| { country: (key >> 32) as u32, archetype: key as u32 }, {0 => units: I64, 1 => units_inexact: U32, 2 => spent: I64, 3 => units_without_refits: I64, 4 => units_without_refits_inexact: U32, 5 => spent_without_refits: I64, 6 => archetype_spelling: Bytes}, |_row| Ok(()));
row_codec!(ProductionCoverageRow, 40, |row| (row.country as u128) << 8 | row.source as u128, |key| { country: (key >> 8) as u32, source: key as u8 }, {0 => observation_since: OptionalI32, 1 => units_unknown_from: OptionalI32, 2 => spending_unknown_from: OptionalI32, 3 => spending_reason: U8}, |row| if row.source > ProductionCoverageRow::ROCKET { Err(invalid("a production coverage row's source is past Rocket")) } else { Ok(()) });
row_codec!(DockyardRepairRateRow, 104, |row| (row.country as u128) << 72 | (row.part as u128) << 64 | row.design as u128, |key| { country: (key >> 72) as u32, part: (key >> 64) as u8, design: key as u64 }, {0 => rate: I64, 1 => used_dockyards: I32, 2 => output: I64, 3 => split_known: Bool}, |row| if row.part > DockyardRepairRateRow::DESIGN { Err(invalid("a dockyard repair rate's part is past a design")) } else { Ok(()) });
row_codec!(LandBattleRow, 64, |row| row.combat as u128, |key| { combat: key as u64 }, {0 => start_hour: I32, 1 => start_exact: Bool, 2 => counter_base: I32, 3 => peak_total: U32, 4 => peak_total_hour: I32, 5 => peak_attackers: U32, 6 => peak_attackers_hour: I32, 7 => peak_defenders: U32, 8 => peak_defenders_hour: I32}, |_row| Ok(()));
row_codec!(LandBattleEndRow, 97, |row| (row.combat as u128) << 33 | (row.result_only as u128) << 32 | row.start_hour as u32 as u128, |key| { combat: (key >> 33) as u64, result_only: (key >> 32) & 1 == 1, start_hour: key as u32 as i32 }, {0 => kind: U8, 1 => end_hour: I32, 2 => start_exact: Bool, 3 => counter_base: I32, 4 => province: U32, 5 => last_read: OptionalI32, 6 => last_duration: I32, 7 => peak_total: U32, 8 => peak_total_hour: I32, 9 => peak_attackers: U32, 10 => peak_attackers_hour: I32, 11 => peak_defenders: U32, 12 => peak_defenders_hour: I32, 13 => attacker_manpower: I32, 14 => defender_manpower: I32, 15 => attackers: U32List, 16 => defenders: U32List, 17 => countries_unknown: U8, 18 => result: OptionalU64, 19 => result_read: Bool, 20 => winners: U8, 21 => result_duration: I32, 22 => attacker_terminal: I32, 23 => defender_terminal: I32, 24 => result_attackers: U32List, 25 => result_defenders: U32List, 26 => result_unknown: U8, 27 => attacker_terminal_equipment: Opaque, 28 => defender_terminal_equipment: Opaque}, |row| if row.kind > LandBattleEndRow::RESULT_ONLY { Err(invalid("land battle end kind invalid")) } else { Ok(()) });
row_codec!(NavalBattleRow, 64, |row| row.combat as u128, |key| { combat: key as u64 }, {0 => first_hour: I32, 1 => peak_ships: U32, 2 => peak_hour: I32, 3 => attacker_sunk: U32, 4 => defender_sunk: U32, 5 => attacker_convoys_sunk: U32, 6 => defender_convoys_sunk: U32}, |_row| Ok(()));
row_codec!(NavalResultObservationRow, 64, |row| row.result as u128, |key| { result: key as u64 }, {0 => refusal: U8, 1 => combat: U64, 2 => province: U32, 3 => first_hour: I32, 4 => last_hour: I32, 5 => peak_ships: U32, 6 => peak_hour: I32, 7 => attacker_ships: U32, 8 => attacker_convoys: U32, 9 => attacker_sunk: U32, 10 => attacker_convoys_sunk: U32, 11 => defender_ships: U32, 12 => defender_convoys: U32, 13 => defender_sunk: U32, 14 => defender_convoys_sunk: U32}, |row| if row.refusal > NavalResultObservationRow::NOT_OBSERVED { Err(invalid("naval result observation refusal invalid")) } else { Ok(()) });
row_codec!(NavalLossTotalRow, 80, |row| (row.slot as u128) << 48 | (row.country as u128) << 16 | (row.attribution as u128) << 8 | row.kind as u128, |key| { slot: (key >> 48) as u32, country: (key >> 16) as u32, attribution: (key >> 8) as u8, kind: key as u8 }, {0 => total: I64, 1 => unknown: U32, 2 => counted_from: I32, 3 => known: Bool, 4 => lost_from: OptionalI32}, |row| if row.attribution > NavalLossTotalRow::SUNK_BY || row.kind > NavalLossTotalRow::CONVOYS { Err(invalid("naval loss total attribution or kind invalid")) } else { Ok(()) });
row_codec!(DivisionEventRow, 104, |row| (row.kind as u128) << 96 | (row.hour as u32 as u128) << 64 | row.division as u128, |key| { kind: (key >> 96) as u8, hour: (key >> 64) as u32 as i32, division: key as u64 }, {0 => disposition: OptionalU32, 1 => cause: OptionalU32, 2 => owner: U32, 3 => slot: U32, 4 => native_hour: I32, 5 => exact: Bool, 6 => after: OptionalI32, 7 => province: U32, 8 => place_hour: I32, 9 => template: U64, 10 => template_hour: I32, 11 => name: Opaque, 12 => naming: Opaque, 13 => name_hour: I32, 14 => icon_unit: U32, 15 => icon_custom: I32, 16 => icon_name: Bytes, 17 => icon_hour: I32, 18 => killer_country: OptionalU32, 19 => killer_slot: OptionalU32, 20 => killer_division: OptionalU64, 21 => template_id: U32, 22 => matched_deaths: OptionalU32, 23 => template_literal: Bytes, 24 => template_key: Bytes, 25 => template_country: I32}, |row| if row.kind > DivisionEventRow::VANISHED { Err(invalid("a division event's kind is past vanished")) } else { Ok(()) });
row_codec!(LeaderBattleRow, 128, |row| (row.leader as u128) << 64 | (row.hour as u32 as u128) << 32 | row.ordinal as u128, |key| { leader: (key >> 64) as u64, hour: (key >> 32) as u32 as i32, ordinal: key as u32 }, {0 => entry: U8, 1 => kind: U8, 2 => combat: U64, 3 => sides: U8, 4 => start_hour: I32, 5 => start_unknown: Bool, 6 => winners: U8, 7 => result_hour: I32, 8 => outcome: U8}, |row| if row.entry > LeaderBattleRow::RESULT || !matches!(row.kind, LeaderBattleRow::LAND | LeaderBattleRow::NAVAL) { Err(invalid("a leader battle's entry or kind is unknown")) } else { Ok(()) });
row_codec!(LeaderTraitGainRow, 128, |row| (row.leader as u128) << 64 | (row.hour as u32 as u128) << 32 | row.token as u128, |key| { leader: (key >> 64) as u64, hour: (key >> 32) as u32 as i32, token: key as u32 }, {0 => class: U8, 1 => spelling: Bytes}, |row| if row.class > LeaderTraitGainRow::FIRST_OBSERVED { Err(invalid("a trait gain's class is past first observed")) } else { Ok(()) });
row_codec!(PowerSpendTotalRow, 72, |row| (row.holder as u128) << 40 | (row.currency as u128) << 32 | row.ordinal as u128, |key| { holder: (key >> 40) as u32, currency: (key >> 32) as u8, ordinal: key as u32 }, {0 => known: I64, 1 => payments: I64, 2 => entries: I64, 3 => unknown: I64, 4 => uncovered: I64, 5 => latest_hour: OptionalI32, 6 => latest_sequence: OptionalU64, 7 => latest_ordinal: OptionalU32, 8 => latest_read: OptionalI32, 9 => days: I64, 10 => day_payments: I64, 11 => day_entries: I64, 12 => day_charge: I64, 13 => day_unknown: I64, 14 => days_uncovered: I64, 15 => realized_gained: I64, 16 => realized_lost: I64, 17 => realized_unknown: I64, 18 => first_day: OptionalI32, 19 => last_day: OptionalI32, 20 => last_lag: OptionalI32, 21 => last_hour: OptionalI32, 22 => last_sequence: OptionalU64, 23 => last_ordinal: OptionalU32, 24 => last_covered: Bool, 25 => balances: I64, 26 => credits: I64, 27 => gained: I64, 28 => capped: I64, 29 => gain_days: I64, 30 => gain_unknown_days: I64, 31 => unmatched: I64}, |row| if row.currency > PowerCurrency::Command as u8 { Err(invalid("a power total's currency is past command")) } else { Ok(()) });
row_codec!(PowerSpendDayRow, 104, |row| (row.holder as u128) << 72 | (row.hour as u32 as u128) << 40 | (row.currency as u128) << 32 | row.entry as u128, |key| { holder: (key >> 72) as u32, hour: (key >> 40) as u32 as i32, currency: (key >> 32) as u8, entry: key as u32 }, {0 => ordinal: U32, 1 => day: I32, 2 => days: I64, 3 => payments: I64, 4 => charge: I64, 5 => unknown: I64, 6 => uncovered: I64, 7 => realized_gained: I64, 8 => realized_lost: I64, 9 => realized_unknown: I64, 10 => credits: I64}, |row| if row.currency > PowerCurrency::Command as u8 || row.day.rem_euclid(24) != 0 { Err(invalid("a power spend day's currency is past command or its day is not a midnight")) } else { Ok(()) });
row_codec!(PowerItemRow, 64, |row| (row.holder as u128) << 32 | row.ordinal as u128, |key| { holder: (key >> 32) as u32, ordinal: key as u32 }, {0 => kind: U8, 1 => currency: U8, 2 => cause: U16, 3 => component: OptionalU32, 4 => charged: Bool, 5 => site: U16, 6 => action: U8, 7 => item_token: OptionalU32, 8 => item_key: Bytes, 9 => slot_key: Bytes, 10 => character: OptionalU32, 11 => object_kind: U8, 12 => object_reference: OptionalU64, 13 => target_country: OptionalU32, 14 => target_state: OptionalU32, 15 => target_region: OptionalU32, 16 => origin_kind: U8, 17 => origin_token: OptionalU32, 18 => origin_key: Bytes, 19 => first_hour: I32, 20 => first_read: I32, 21 => first_country: U32, 22 => first_epoch: U64, 23 => first_sequence: U64}, |row| if row.kind > PowerItemRow::ABILITY_UPKEEP { Err(invalid("a power item's kind is past ability upkeep")) } else { Ok(()) });
row_codec!(PowerDayPartRow, 72, |row| (row.holder as u128) << 40 | (row.currency as u128) << 32 | row.ordinal as u128, |key| { holder: (key >> 40) as u32, currency: (key >> 32) as u8, ordinal: key as u32 }, {0 => day: I32, 1 => spent: I64, 2 => payments: I64, 3 => covered: Bool, 4 => sequence: OptionalU64, 5 => cost_ordinal: OptionalU32, 6 => read: I32}, |row| if row.day.rem_euclid(24) != 0 { Err(invalid("a power day part's day is not a midnight")) } else { Ok(()) });
row_codec!(PowerHoldRow, 96, |row| (row.holder as u128) << 64 | row.hold as u128, |key| { holder: (key >> 64) as u32, hold: key as u64 }, {0 => amount: I64, 1 => check: U64, 2 => cause: U16, 3 => object_kind: U8, 4 => object_reference: OptionalU64, 5 => ability: Bool, 6 => item_token: OptionalU32, 7 => item_key: Bytes, 8 => slot_key: Bytes, 9 => character: OptionalU32, 10 => target_country: OptionalU32, 11 => target_state: OptionalU32, 12 => target_region: OptionalU32, 13 => capture_country: U32, 14 => capture_epoch: U64}, |_row| Ok(()));
row_codec!(PowerSpendCoverageRow, 32, |row| row.holder as u128, |key| { holder: key as u32 }, {0 => political: Bool, 1 => command: Bool, 2 => reason: U8, 3 => political_since: OptionalI32, 4 => command_since: OptionalI32, 5 => daily_mismatch: U8, 6 => input_reason: Bytes}, |row| if usize::from(row.reason) >= PowerSpendCoverageRow::REASONS.len() { Err(invalid("a power spend coverage reason is past the reasons")) } else { Ok(()) });
row_codec!(ResearchAssignmentRow, 112, |row| (row.holder_slot as u128) << 80 | (row.holder_id as u128) << 48 | (row.index as u128) << 16 | row.lane as u128, |key| { holder_slot: (key >> 80) as u32, holder_id: (key >> 48) as u32, index: (key >> 16) as u32, lane: key as u16 }, {0 => technology_handle: U32, 1 => database_index: I32, 2 => token: Bytes, 3 => level: I32, 4 => continuity_start_hour: I32, 5 => first_observed_hour: I32, 6 => started_after_hour: OptionalI32, 7 => last_observed_hour: OptionalI32, 8 => ended_by_hour: OptionalI32, 9 => outcome: U8, 10 => outcome_observed_hour: OptionalI32, 11 => awaiting: U8}, |row| if row.outcome > ResearchAssignmentRow::UNKNOWN { Err(invalid("a research assignment's outcome is past unknown")) } else { Ok(()) });
row_codec!(ResearchCoverageRow, 96, |row| (row.holder_slot as u128) << 64 | (row.holder_id as u128) << 32 | row.member as u128, |key| { holder_slot: (key >> 64) as u32, holder_id: (key >> 32) as u32, member: key as u32 }, {0 => since: I32}, |_row| Ok(()));
row_codec!(FactionAccountOwnerRow, 32, |row| row.account as u128, |key| { account: key as u32 }, {0 => faction: U64, 1 => slot: U32, 2 => id: U32, 3 => member: U8}, |_row| Ok(()));
row_codec!(PeaceOutcomeRow, 64, |row| (row.after_hour as u32 as u128) << 32 | row.ordinal as u128, |key| { after_hour: (key >> 32) as u32 as i32, ordinal: key as u32 }, {0 => observation: U8, 1 => conference: OptionalU64, 2 => before_hour: I32, 3 => change: U8, 4 => group: U32, 5 => state: U32, 6 => from_slot: U32, 7 => from_id: U32, 8 => to_slot: U32, 9 => to_id: U32, 10 => victory_points: OptionalI64}, |row| if row.observation > PeaceOutcomeRow::UNKNOWN || row.change > PeaceOutcomeRow::RELEASED { Err(invalid("a peace outcome's observation or change is past the last")) } else { Ok(()) });
row_codec!(PeaceConferenceSpanRow, 96, |row| (row.conference as u128) << 32 | row.hour as u32 as u128, |key| { conference: (key >> 32) as u64, hour: key as u32 as i32 }, {0 => first_observed_hour: I32, 1 => last_observed_hour: I32, 2 => current_turn: I32, 3 => finalized: U8, 4 => first_finalized_hour: OptionalI32, 5 => first_absent_hour: OptionalI32, 6 => comparison: U8, 7 => before_hour: OptionalI32, 8 => compared_hour: OptionalI32}, |row| if row.comparison > PeaceConferenceSpanRow::OVERLAPPED { Err(invalid("a peace conference's comparison is past overlapped")) } else { Ok(()) });
row_codec!(CasualtyLifetimeRow, 128, |row| (row.slot_a as u128) << 104 | (row.id_a as u128) << 80 | (row.slot_b as u128) << 56 | (row.id_b as u128) << 32 | row.start_hour as u32 as u128, |key| { slot_a: (key >> 104) as u32 & CasualtyLifetimeRow::FIELD, id_a: (key >> 80) as u32 & CasualtyLifetimeRow::FIELD, slot_b: (key >> 56) as u32 & CasualtyLifetimeRow::FIELD, id_b: (key >> 32) as u32 & CasualtyLifetimeRow::FIELD, start_hour: key as u32 as i32 }, {0 => taken_a: I64, 1 => taken_b: I64, 2 => unknown_a: I64, 3 => unknown_b: I64}, |row| if [row.slot_a, row.id_a, row.slot_b, row.id_b].iter().any(|&field| field > CasualtyLifetimeRow::FIELD) { Err(invalid("a casualty lifetime's slot or id passes 24 bits")) } else { Ok(()) });
row_codec!(CasualtyTotalRow, 64, |row| (row.slot as u128) << 32 | row.id as u128, |key| { slot: (key >> 32) as u32, id: key as u32 }, {0 => dealt: I64, 1 => taken: I64, 2 => unknown: I64, 3 => partial: U8}, |_row| Ok(()));
row_codec!(OccupierTotalRow, 64, |row| (row.slot as u128) << 32 | row.id as u128, |key| { slot: (key >> 32) as u32, id: key as u32 }, {0 => resistance_sum: I64, 1 => compliance_sum: I64, 2 => states: I64, 3 => held_states: I64, 4 => held_complete: U8}, |_row| Ok(()));
row_codec!(AirMissionCountRow, 64, |row| (row.slot as u128) << 32 | row.region as u128, |key| { slot: (key >> 32) as u32, region: key as u32 }, {0 => wings: I64, 1 => mission_0: I32, 2 => mission_1: I32, 3 => mission_2: I32, 4 => mission_3: I32, 5 => mission_4: I32, 6 => mission_5: I32, 7 => mission_6: I32, 8 => mission_7: I32, 9 => mission_8: I32, 10 => mission_9: I32, 11 => mission_10: I32, 12 => mission_11: I32, 13 => mission_12: I32, 14 => mission_13: I32, 15 => mission_14: I32, 16 => mission_15: I32, 17 => mission_16: I32, 18 => mission_17: I32}, |_row| Ok(()));
row_codec!(AirRegionWingRow, 32, |row| row.region as u128, |key| { region: key as u32 }, {0 => wings: I64}, |_row| Ok(()));
row_codec!(FeedEventRow, 128, |row| (row.generation as u128) << 64 | row.sequence as u128, |key| { generation: (key >> 64) as u64, sequence: key as u64 }, {0 => hour: I32, 1 => read_hour: I32, 2 => kind: U8, 3 => actor_reference: U32, 4 => actor_id: U32, 5 => actor_instance: U32, 6 => actor_slot: U32, 7 => target_reference: U32, 8 => target_id: U32, 9 => target_instance: U32, 10 => target_slot: U32, 11 => route: U8, 12 => within: U8, 13 => conference: OptionalU64, 14 => state: OptionalU32, 15 => ally_reference: OptionalU32, 16 => ally_id: OptionalU32, 17 => ally_instance: OptionalU32, 18 => ally_slot: OptionalU32, 19 => reason: OptionalU32, 20 => flags: U8, 21 => token: Bytes, 22 => native_event_kind: OptionalU32, 23 => native_event_id: OptionalU32, 24 => instance: OptionalU32, 25 => option: OptionalI32, 26 => country_target: OptionalU32, 27 => numeric_target: OptionalI32, 28 => lifecycle: OptionalI32, 29 => days: OptionalI32, 30 => scopes: Opaque, 31 => opener: OptionalU64, 32 => opener_hour: OptionalI32, 33 => dated: OptionalI32, 34 => political_power: OptionalI64, 35 => command_power: OptionalI64, 36 => coverage: OptionalU32, 37 => outstanding: OptionalU64, 38 => refusals: OptionalU32, 39 => failure: Bytes, 40 => backlog: Bool, 41 => incomplete: OptionalU32}, |_row| Ok(()));
row_codec!(FeedOpenEventRow, 128, |row| (row.kind as u128) << 96 | (row.id as u128) << 64 | (row.instance as u128) << 32 | row.receiver as u128, |key| { kind: (key >> 96) as u32, id: (key >> 64) as u32, instance: (key >> 32) as u32, receiver: key as u32 }, {0 => generation: U64, 1 => sequence: U64, 2 => read_hour: I32}, |_row| Ok(()));
row_codec!(FeedOpenDecisionRow, 128, |row| (row.generation as u128) << 64 | row.sequence as u128, |key| { generation: (key >> 64) as u64, sequence: key as u64 }, {0 => country: U32, 1 => token: Bytes, 2 => country_target: OptionalU32, 3 => numeric_target: OptionalI32, 4 => read_hour: I32}, |_row| Ok(()));
row_codec!(FeedGuaranteeRow, 96, |row| (row.first as u128) << 64 | (row.second as u128) << 32 | row.start as u32 as u128, |key| { first: (key >> 64) as u32, second: (key >> 32) as u32, start: key as u32 as i32 }, {0 => hour: OptionalI32, 1 => rows: U32}, |_row| Ok(()));
row_codec!(NavalRegionControlRow, 64, |row| (row.viewer as u128) << 32 | row.region as u128, |key| { viewer: (key >> 32) as u32, region: key as u32 }, {0 => friendly: I64, 1 => enemy: I64, 2 => ratio_friendly: I64, 3 => ratio_enemy: I64, 4 => own: I64, 5 => control_refusal: Bytes, 6 => share_refusal: Bytes}, |_row| Ok(()));
row_codec!(CountryNukeTotalRow, 32, |row| row.country as u128, |key| { country: key as u32 }, {0 => total: I64, 1 => ready: I64}, |_row| Ok(()));
row_codec!(RaidEndRow, 96, |row| (row.hour as u32 as u128) << 64 | (row.id as u128) << 32 | row.reference_type as u128, |key| { hour: (key >> 64) as u32 as i32, id: (key >> 32) as u32, reference_type: key as u32 }, {0 => kind: U8, 1 => previous_read: I32, 2 => first_hour: OptionalI32, 3 => first_exact: Bool, 4 => launched_hour: OptionalI32, 5 => raider: U32, 6 => phase: I32, 7 => outcome: I32, 8 => travelled: OptionalI64, 9 => preparation_hours: I32, 10 => province: U32, 11 => state: U32, 12 => leader_reference: U64, 13 => leader_province: U32, 14 => building_type: U32, 15 => building_location: U32, 16 => building_flags: U32, 17 => building_country: I32, 18 => leader_country: OptionalI32, 19 => raid_type: U32, 20 => target_state: OptionalU32, 21 => victim: OptionalU32, 22 => victim_slot: U32, 23 => victim_unknown: U8, 24 => end_hour: OptionalI32, 25 => completion_country: OptionalU32, 26 => completion_country_slot: U32}, |row| if row.kind > RaidEndRow::DELETED || row.victim_unknown > RaidEndRow::VICTIM_UNLINKED { Err(invalid("a raid end's kind or victim reason is past the last")) } else { Ok(()) });
row_codec!(AirMissionWingRow, 128, |row| (row.slot as u128) << 96 | (row.region as u128) << 64 | row.wing as u128, |key| { slot: (key >> 96) as u32, region: (key >> 64) as u32, wing: key as u64 }, {0 => ordinal: U32}, |_row| Ok(()));
row_codec!(LeaderBattleTotalRow, 64, |row| row.leader as u128, |key| { leader: key as u64 }, {0 => fought: U32, 1 => won: U32, 2 => lost: U32}, |_row| Ok(()));
row_codec!(LeaderTraitDateRow, 96, |row| (row.leader as u128) << 32 | row.token as u128, |key| { leader: (key >> 32) as u64, token: key as u32 }, {0 => first_hour: I32, 1 => first_class: U8, 2 => gained_hour: OptionalI32, 3 => spelling: Bytes}, |row| if row.first_class > LeaderTraitGainRow::FIRST_OBSERVED { Err(invalid("a trait date's class is past first observed")) } else { Ok(()) });
row_codec!(LeaderCommandSpanRow, 72, |row| (row.source as u128) << 64 | row.formation as u128, |key| { source: (key >> 64) as u8, formation: key as u64 }, {0 => leader: U64, 1 => first_hour: I32, 2 => kind: U8, 3 => name: Opaque}, |row| if row.source > LeaderCommandRow::FLEET || row.kind > LeaderCommandRow::NAVY { Err(invalid("a command span's source or kind is past the last")) } else { Ok(()) });
row_codec!(LeaderCommandRow, 128, |row| (row.leader as u128) << 64 | (row.hour as u32 as u128) << 32 | row.ordinal as u128, |key| { leader: (key >> 64) as u64, hour: (key >> 32) as u32 as i32, ordinal: key as u32 }, {0 => source: U8, 1 => formation: U64, 2 => kind: U8, 3 => first_hour: I32, 4 => last_hour: I32, 5 => name: Opaque}, |row| if row.source > LeaderCommandRow::FLEET || row.kind > LeaderCommandRow::NAVY { Err(invalid("a command's source or kind is past the last")) } else { Ok(()) });
row_codec!(ThreatTotalRow, 0, |_row| 0, |_key| {  }, {0 => sum: I64}, |_row| Ok(()));
row_codec!(FactionTotalRow, 64, |row| row.faction as u128, |key| { faction: key as u64 }, {0 => owned_states: I64, 1 => controlled_states: I64, 2 => occupied_states: I64, 3 => owned_provinces: I64, 4 => controlled_provinces: I64, 5 => occupied_provinces: I64, 6 => owned_victory_points: I64, 7 => controlled_victory_points: I64, 8 => occupied_victory_points: I64, 9 => owned_area: I64, 10 => controlled_area: I64, 11 => occupied_area: I64, 12 => free1: I64, 13 => free2: I64, 14 => free3: I64, 15 => free4: I64, 16 => free5: I64, 17 => free6: I64, 18 => free7: I64, 19 => built1: I64, 20 => built2: I64, 21 => built3: I64, 22 => built4: I64, 23 => built5: I64, 24 => built6: I64, 25 => built7: I64, 26 => top_bar1: I64, 27 => top_bar2: I64, 28 => top_bar3: I64, 29 => top_bar4: I64, 30 => top_bar5: I64, 31 => top_bar6: I64, 32 => top_bar7: I64}, |_row| Ok(()));
row_codec!(AirRegionActivityRow, 32, |row| row.region as u128, |key| { region: key as u32 }, {0 => day: I32, 1 => friendly: I64, 2 => enemy: I64, 3 => friendly_total: I64, 4 => enemy_total: I64}, |_row| Ok(()));
row_codec!(AirTheatreRegionRow, 96, |row| (row.theatre as u128) << 32 | row.region as u128, |key| { theatre: (key >> 32) as u64, region: key as u32 }, {0 => wings: I64}, |_row| Ok(()));
row_codec!(AirRegionDayRow, 32, |row| row.region as u128, |key| { region: key as u32 }, {0 => day: I32, 1 => activity: I64, 2 => activity_total: I64}, |_row| Ok(()));
row_codec!(AirOwnerDayRow, 64, |row| (row.country as u128) << 32 | row.region as u128, |key| { country: (key >> 32) as u32, region: key as u32 }, {0 => day: I32, 1 => friendly: I64, 2 => enemy: I64, 3 => friendly_total: I64, 4 => enemy_total: I64}, |_row| Ok(()));
row_codec!(TerritoryRegionTotalRow, 64, |row| (row.holder as u128) << 32 | row.region as u128, |key| { holder: (key >> 32) as u32, region: key as u32 }, {0 => controlled_provinces: I64, 1 => controlled_victory_points: I64, 2 => controlled_area: I64, 3 => controlled_states: I64, 4 => controlled_states_denominator: I64, 5 => owned_provinces: I64, 6 => owned_victory_points: I64, 7 => owned_area: I64, 8 => owned_states: I64, 9 => owned_states_denominator: I64}, |_row| Ok(()));
row_codec!(ProducedValueRow, 64, |row| (row.country as u128) << 32 | row.archetype as u128, |key| { country: (key >> 32) as u32, archetype: key as u32 }, {0 => units: I64, 1 => units_reason: U8, 2 => units_without_refits: I64, 3 => units_without_refits_reason: U8, 4 => ic: I64, 5 => ic_reason: U8, 6 => ic_without_refits: I64, 7 => ic_without_refits_reason: U8, 8 => since: OptionalI32, 9 => as_of: OptionalI32}, |_row| Ok(()));
row_codec!(AirLossTotalRow, 128, |row| (row.slot as u128) << 96 | (row.id as u128) << 64 | (row.region as u128) << 32 | row.cause as u128, |key| { slot: (key >> 96) as u32, id: (key >> 64) as u32, region: (key >> 32) as u32, cause: key as u32 }, {0 => dealt_known: I64, 1 => dealt_unknown: I64, 2 => taken_known: I64, 3 => taken_unknown: I64, 4 => late: I32}, |_row| Ok(()));
row_codec!(BombingTotalRow, 72, |row| (row.slot as u128) << 40 | (row.id as u128) << 8 | row.cause as u128, |key| { slot: (key >> 40) as u32, id: (key >> 8) as u32, cause: key as u8 }, {0 => dealt: I64, 1 => taken: I64, 2 => late: I32}, |_row| Ok(()));
row_codec!(GarrisonStrengthRow, 64, |row| (row.controller as u128) << 32 | row.state as u128, |key| { controller: (key >> 32) as u32, state: key as u32 }, {0 => strength: I64, 1 => reason: U8}, |_row| Ok(()));
