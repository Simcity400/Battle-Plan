use super::*;

fn multiply(a: i64, b: i64, denominator: i64) -> Option<i64> {
    a.wrapping_mul(b).checked_div(denominator)
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionLineRow {
    pub handle: u64,
    pub country: u32,
    pub kind: u8,
    pub variant: u64,
    pub mio: u64,
    pub active_factories: i32,
    pub requested_factories: i32,
    pub cost_raw: i64,
    pub speed_raw: i64,
    pub produced_raw: RetiredI64,
    pub priority: i32,
    pub amount: i32,
    pub max_factories: i32,
    pub efficiency_raw: i64,
}

impl ProductionLineRow {
    pub fn factory_efficiency(&self, source: &ProductionLineModifiersRow) -> Option<i64> {
        if (
            self.handle,
            self.country,
            self.variant,
            self.mio,
            self.kind,
            self.active_factories,
        ) != (
            source.line,
            source.country,
            source.variant,
            source.organisation,
            source.kind,
            source.active_factories,
        ) {
            return None;
        }
        let count = self.active_factories.min(self.max_factories).max(0);
        let values = source
            .factory_efficiencies
            .get(..usize::try_from(count).ok()?)?;
        let sum = values
            .iter()
            .fold(0_u64, |sum, &value| sum.wrapping_add(value as u64));
        Some(game_model::production::efficiency_average(sum, count))
    }
}
typed_codec!(ProductionLineRow, 64, |row| row.handle as u128, |key| Self { handle: key as u64, ..Default::default() },
 { 0 => (country): U32, 1 => (kind): U8, 2 => (variant): U64, 3 => (mio): U64, 4 => (active_factories): I32, 5 => (requested_factories): I32, 6 => (cost_raw): I64, 7 => (speed_raw): I64, 8 => (produced_raw): I64, 9 => (priority): I32, 10 => (amount): I32, 11 => (max_factories): I32, 12 => (efficiency_raw): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IndustrialOrganisationRow {
    pub organisation: u64,
    pub country: u32,
    pub ordinal: u32,
    pub owner: u32,
    pub token: u32,
    pub name: Vec<u8>,
    pub icon: Vec<u8>,
}
typed_codec!(IndustrialOrganisationRow, 64, |row| row.organisation as u128, |key| Self { organisation: key as u64, ..Default::default() },
 { 0 => (country): U32, 1 => (ordinal): U32, 2 => (owner): U32, 3 => (token): U32, 4 => (name): Bytes, 5 => (icon): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IndustrialOrganisationHistoryRow {
    pub organisation: u64,
    pub variant: u64,
    pub units: i32,
    pub design_hour: Recorded<Option<i32>>,
}
typed_codec!(IndustrialOrganisationHistoryRow, 128, |row| (row.organisation as u128) << 64 | row.variant as u128, |key| Self { organisation: (key >> 64) as u64, variant: key as u64, ..Default::default() },
 { 0 => (units): I32, 1 => (design_hour): OptionalI32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionLineOutputRow {
    pub line: u64,
    pub ordinal: u32,
    pub country: u32,
    pub variant: u64,
    pub mio: u64,
    pub units: i64,
    pub mio_credit: RetiredI64,
    pub since_hour: i32,
    pub observed_since_creation: bool,
    pub ended: bool,
    pub exact: bool,
    pub spent_raw: Recorded<Option<i64>>,
    pub instant_raw: Recorded<RetiredOptionalI64>,
    pub purpose: Recorded<u8>,
}
typed_codec!(ProductionLineOutputRow, 96, |row| (row.line as u128) << 32 | row.ordinal as u128, |key| Self { line: (key >> 32) as u64, ordinal: key as u32, ..Default::default() },
{ 0 => (country): U32, 1 => (variant): U64, 2 => (mio): U64, 3 => (units): I64, 4 => (mio_credit): I64, 5 => (since_hour): I32, 6 => (observed_since_creation): Bool, 7 => (ended): Bool, 8 => (exact): Bool, 9 => (spent_raw): OptionalI64, 10 => (instant_raw): OptionalI64, 11 => (purpose): U8 },
|row| match row.purpose {
    Recorded::Value(raw) if OutputPurpose::from_raw(raw).is_none() => Err(invalid("production output purpose invalid")),
    _ => Ok(()),
});

impl ProductionLineOutputRow {
    pub fn purpose(&self) -> OutputPurpose {
        match self.purpose {
            Recorded::Value(raw) => OutputPurpose::from_raw(raw).unwrap_or_default(),
            Recorded::NotRecorded => OutputPurpose::Unknown,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CombatLogEquipmentRow {
    pub country: u32,
    pub group: u32,
    pub kind: u8,
    pub month: u16,
    pub variant: u32,
    pub date: i32,
    pub amount_raw: i64,
}
typed_codec!(CombatLogEquipmentRow, 120, |row| (row.country as u128) << 88 | (row.group as u128) << 56 | (row.kind as u128) << 48 | (row.month as u128) << 32 | row.variant as u128, |key| Self { country: (key >> 88) as u32, group: (key >> 56) as u32, kind: (key >> 48) as u8, month: (key >> 32) as u16, variant: key as u32, ..Default::default() },
 { 0 => (date): I32, 1 => (amount_raw): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CombatLogStateRow {
    pub viewing_country_raw: i32,
    pub prune_observed: u8,
}
typed_codec!(CombatLogStateRow, 0, |_row| 0, |_key| Self { viewing_country_raw: Default::default(), prune_observed: Default::default() },
 { 0 => (viewing_country_raw): I32, 1 => (prune_observed): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LendLeaseEquipmentRow {
    pub giver: u32,
    pub exchange: u32,
    pub kind: u8,
    pub variant: u32,
    pub receiver: u32,
    pub amount_raw: i64,
}
typed_codec!(LendLeaseEquipmentRow, 104, |row| (row.giver as u128) << 72 | (row.exchange as u128) << 40 | (row.kind as u128) << 32 | row.variant as u128, |key| Self { giver: (key >> 72) as u32, exchange: (key >> 40) as u32, kind: (key >> 32) as u8, variant: key as u32, ..Default::default() },
 { 0 => (receiver): U32, 1 => (amount_raw): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StockpileRow {
    pub country: u32,
    pub ordinal: u32,
    pub variant_handle: u64,
    pub amount_raw: i64,
}
typed_codec!(StockpileRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, variant_handle: Default::default(), amount_raw: Default::default() },
 { 0 => (variant_handle): U64, 1 => (amount_raw): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionEquipmentRow {
    pub division: u64,
    pub template: u64,
    pub country: u32,
    pub ordinal: u32,
    pub variant_handle: u64,
    pub amount_raw: i64,
}
typed_codec!(DivisionEquipmentRow, 96, |row| (row.division as u128) << 32 | row.ordinal as u128, |key| Self { division: (key >> 32) as u64, ordinal: key as u32, template: Default::default(), country: Default::default(), variant_handle: Default::default(), amount_raw: Default::default() },
 { 0 => (template): U64, 1 => (country): U32, 2 => (variant_handle): U64, 3 => (amount_raw): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryLocalSubunitRow {
    pub country: u32,
    pub token: u32,
    pub requirements: Vec<LocalSubunitRequirement>,
}
typed_codec!(CountryLocalSubunitRow, 64, |row| (row.country as u128) << 32 | row.token as u128,
 |key| Self { country: (key >> 32) as u32, token: key as u32, ..Default::default() },
 { 0 => (requirements): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AvailableEquipmentVariantRow {
    pub country: u32,
    pub roster_kind: u8,
    pub ordinal: u32,
    pub variant: u64,
}
typed_codec!(AvailableEquipmentVariantRow, 66, |row| (row.country as u128) << 34 | (row.roster_kind as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 34) as u32, roster_kind: ((key >> 32) & 3) as u8, ordinal: key as u32, ..Default::default() },
 { 0 => (variant): U64 }, |row| { if row.roster_kind > 2 { return Err(invalid("equipment roster kind must be primary, foreign or best")); } Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryUnitBonusRow {
    pub country: u32,
    pub ordinal: u32,
    pub source_kind: u8,
    pub source_id: u32,
    pub source_number: i32,
    pub label: Vec<u8>,
    pub targets: Vec<UnitBonusTarget>,
}
typed_codec!(CountryUnitBonusRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (source_kind): U8, 1 => (source_id): U32, 2 => (source_number): I32, 3 => (label): Bytes, 4 => (targets): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantModuleRow {
    pub variant: u64,
    pub source_ordinal: u32,
    pub slot: u32,
    pub definition_token: u32,
    pub definition_active: u8,
}
typed_codec!(VariantModuleRow, 96, |row| (row.variant as u128) << 32 | row.source_ordinal as u128, |key| Self { variant: (key >> 32) as u64, source_ordinal: key as u32, slot: Default::default(), definition_token: Default::default(), definition_active: Default::default() },
 { 0 => (slot): U32, 1 => (definition_token): U32, 2 => (definition_active): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantStatsRow {
    pub variant: u64,
    pub values: [Option<i64>; 10],
    pub organisation: u64,
    pub organisation_trait_count: i32,
    pub organisation_bonuses: Vec<IndustrialOrganisationBonus>,
}
impl VariantStatsRow {
    pub const AIR_RANGE: usize = 0;
    pub const AIR_DEFENCE: usize = 1;
    pub const AIR_ATTACK: usize = 2;
    pub const AIR_AGILITY: usize = 3;
    pub const MAXIMUM_SPEED: usize = 4;
    pub const RELIABILITY: usize = 5;
    pub const WEIGHT: usize = 6;
    pub const THRUST: usize = 7;
    pub const FUEL_CONSUMPTION: usize = 8;
    pub const BUILD_COST_IC: usize = 9;
    pub const LANE_NAMES: [&'static str; 10] = [
        "air_range",
        "air_defence",
        "air_attack",
        "air_agility",
        "maximum_speed",
        "reliability",
        "weight",
        "thrust",
        "fuel_consumption",
        "build_cost_ic",
    ];

    pub fn stat(&self, lane: usize) -> Result<i64, String> {
        let name = Self::LANE_NAMES.get(lane).copied().unwrap_or("unknown");
        self.values
            .get(lane)
            .copied()
            .flatten()
            .ok_or_else(|| format!("variant {} stat `{name}` was withheld", self.variant))
    }
}
typed_codec!(VariantStatsRow, 64, |row| row.variant as u128, |key| Self { variant: key as u64, ..Default::default() },
 { 0 => (values[0]): OptionalI64, 1 => (values[1]): OptionalI64, 2 => (values[2]): OptionalI64, 3 => (values[3]): OptionalI64, 4 => (values[4]): OptionalI64, 5 => (values[5]): OptionalI64, 6 => (values[6]): OptionalI64, 7 => (values[7]): OptionalI64, 8 => (values[8]): OptionalI64, 9 => (values[9]): OptionalI64, 10 => (organisation): U64, 11 => (organisation_trait_count): I32, 12 => (organisation_bonuses): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantUpgradeRow {
    pub variant: u64,
    pub source_ordinal: u32,
    pub definition_token: i32,
    pub level: u8,
}
typed_codec!(VariantUpgradeRow, 96, |row| (row.variant as u128) << 32 | row.source_ordinal as u128, |key| Self { variant: (key >> 32) as u64, source_ordinal: key as u32, definition_token: Default::default(), level: Default::default() },
 { 0 => (definition_token): I32, 1 => (level): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyUpgradeGroupRow {
    pub country: u32,
    pub army: u64,
    pub priority: u32,
}
typed_codec!(ArmyUpgradeGroupRow, 64, |row| row.army as u128, |key| Self { army: key as u64, country: Default::default(), priority: Default::default() },
 { 0 => (country): U32, 1 => (priority): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyUpgradeRequestRow {
    pub country: u32,
    pub army: u64,
    pub ordinal: u32,
    pub equipment: u64,
    pub request_raw: i64,
    pub total_raw: i64,
}
typed_codec!(ArmyUpgradeRequestRow, 96, |row| (row.army as u128) << 32 | row.ordinal as u128, |key| Self { army: (key >> 32) as u64, ordinal: key as u32, country: Default::default(), equipment: Default::default(), request_raw: Default::default(), total_raw: Default::default() },
 { 0 => (country): U32, 1 => (equipment): U64, 2 => (request_raw): I64, 3 => (total_raw): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyUpgradeDeliveryRow {
    pub country: u32,
    pub army: u64,
    pub ordinal: u32,
    pub status: u32,
    pub progress_raw: Option<i64>,
    pub total_progress_raw: i64,
    pub allow_zero_entries: u8,
}
typed_codec!(ArmyUpgradeDeliveryRow, 96, |row| (row.army as u128) << 32 | row.ordinal as u128, |key| Self { army: (key >> 32) as u64, ordinal: key as u32, country: Default::default(), status: Default::default(), progress_raw: Default::default(), total_progress_raw: Default::default(), allow_zero_entries: Default::default() },
 { 0 => (country): U32, 1 => (status): U32, 2 => (progress_raw): OptionalI64, 3 => (total_progress_raw): I64, 4 => (allow_zero_entries): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyUpgradeProducedRow {
    pub country: u32,
    pub army: u64,
    pub delivery_ordinal: u32,
    pub ordinal: u32,
    pub equipment: u64,
    pub amount_raw: i64,
}
typed_codec!(ArmyUpgradeProducedRow, 128, |row| (row.army as u128) << 64 | (row.delivery_ordinal as u128) << 32 | row.ordinal as u128, |key| Self { army: (key >> 64) as u64, delivery_ordinal: (key >> 32) as u32, ordinal: key as u32, country: Default::default(), equipment: Default::default(), amount_raw: Default::default() },
 { 0 => (country): U32, 1 => (equipment): U64, 2 => (amount_raw): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryFuelConsumerRow {
    pub country: u32,
    pub index: u32,
    pub priority: i32,
    pub requested_raw: i64,
    pub received_raw: i64,
}
typed_codec!(CountryFuelConsumerRow, 64, |row| (row.country as u128) << 32 | row.index as u128, |key| Self { country: (key >> 32) as u32, index: key as u32, ..Default::default() },
 { 0 => (priority): I32, 1 => (requested_raw): I64, 2 => (received_raw): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryFactoriesRow {
    pub country: u32,
    pub military_levels: i32,
    pub military_total_raw: i64,
    pub military_controlled_raw: i64,
    pub military_owned_raw: i64,
    pub military_damaged_raw: i64,
    pub military_used: i32,
    pub military_unavailable: i32,
    pub military_from_subjects: i32,
    pub military_to_overlord: i32,
    pub military_from_others: i32,
    pub military_to_others: i32,
    pub naval_levels: i32,
    pub naval_total_raw: i64,
    pub naval_controlled_raw: i64,
    pub naval_owned_raw: i64,
    pub naval_damaged_raw: i64,
    pub naval_used: i32,
    pub naval_unavailable: i32,
    pub naval_from_subjects: i32,
    pub naval_to_overlord: i32,
    pub naval_from_others: i32,
    pub naval_to_others: i32,
    pub civilian_levels: i32,
    pub civilian_total_raw: i64,
    pub civilian_controlled_raw: i64,
    pub civilian_owned_raw: i64,
    pub civilian_damaged_raw: i64,
    pub civilian_used: i32,
    pub civilian_unavailable: i32,
    pub civilian_from_subjects: i32,
    pub civilian_to_overlord: i32,
    pub civilian_from_others: i32,
    pub civilian_to_others: i32,
    pub civilian_license_exports: i32,
    pub civilian_trade_exports: i32,
    pub consumer_goods: i32,
    pub special_projects: i32,
    pub license_imports: i32,
    pub trade_imports: i32,
    pub contract_payments: i32,
}
typed_codec!(CountryFactoriesRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (military_levels): I32, 1 => (military_total_raw): I64, 2 => (military_controlled_raw): I64, 3 => (military_owned_raw): I64, 4 => (military_damaged_raw): I64, 5 => (military_used): I32, 6 => (military_unavailable): I32, 7 => (military_from_subjects): I32, 8 => (military_to_overlord): I32, 9 => (military_from_others): I32, 10 => (military_to_others): I32,
   11 => (naval_levels): I32, 12 => (naval_total_raw): I64, 13 => (naval_controlled_raw): I64, 14 => (naval_owned_raw): I64, 15 => (naval_damaged_raw): I64, 16 => (naval_used): I32, 17 => (naval_unavailable): I32, 18 => (naval_from_subjects): I32, 19 => (naval_to_overlord): I32, 20 => (naval_from_others): I32, 21 => (naval_to_others): I32,
   22 => (civilian_levels): I32, 23 => (civilian_total_raw): I64, 24 => (civilian_controlled_raw): I64, 25 => (civilian_owned_raw): I64, 26 => (civilian_damaged_raw): I64, 27 => (civilian_used): I32, 28 => (civilian_unavailable): I32, 29 => (civilian_from_subjects): I32, 30 => (civilian_to_overlord): I32, 31 => (civilian_from_others): I32, 32 => (civilian_to_others): I32,
   33 => (civilian_license_exports): I32, 34 => (civilian_trade_exports): I32, 35 => (consumer_goods): I32, 36 => (special_projects): I32, 37 => (license_imports): I32, 38 => (trade_imports): I32, 39 => (contract_payments): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResourceDefinitionRow {
    pub id: u32,
    pub token: u32,
    pub active: u8,
    pub factory_cost: i64,
}
typed_codec!(ResourceDefinitionRow, 32, |row| row.id as u128, |key| Self { id: key as u32, token: Default::default(), active: Default::default(), factory_cost: Default::default() },
 { 0 => (token): U32, 1 => (active): U8, 2 => (factory_cost): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryResourceRow {
    pub country: u32,
    pub resource: u32,
    pub produced: i64,
    pub adjustment: i64,
    pub imported: i64,
    pub used: i64,
    pub special_total: i64,
    pub production_demand: i64,
    pub to_export: i64,
    pub base_export: i64,
    pub exported: i64,
}
typed_codec!(CountryResourceRow, 64, |row| (row.country as u128) << 32 | row.resource as u128, |key| Self { country: (key >> 32) as u32, resource: key as u32, produced: Default::default(), adjustment: Default::default(), imported: Default::default(), used: Default::default(), special_total: Default::default(), production_demand: Default::default(), to_export: Default::default(), base_export: Default::default(), exported: Default::default() },
 { 0 => (produced): I64, 1 => (adjustment): I64, 2 => (imported): I64, 3 => (used): I64, 4 => (special_total): I64, 5 => (production_demand): I64, 6 => (to_export): I64, 7 => (base_export): I64, 8 => (exported): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResourceExchangeRow {
    pub id: u64,
    pub exporter: u32,
    pub importer: u32,
    pub resource: u32,
    pub import_ordinal: u32,
    pub export_ordinal: u32,
    pub convoys_assigned: i32,
    pub convoys_required: i32,
    pub efficiency_a: i64,
    pub efficiency_b: i64,
    pub delivered: i64,
    pub required_factories: i32,
    pub assigned_factories: i32,
    pub no_trade_influence: u8,
    pub no_resources: u8,
}
typed_codec!(ResourceExchangeRow, 64, |row| row.id as u128, |key| Self { id: key as u64, exporter: Default::default(), importer: Default::default(), resource: Default::default(), import_ordinal: Default::default(), export_ordinal: Default::default(), convoys_assigned: Default::default(), convoys_required: Default::default(), efficiency_a: Default::default(), efficiency_b: Default::default(), delivered: Default::default(), required_factories: Default::default(), assigned_factories: Default::default(), no_trade_influence: Default::default(), no_resources: Default::default() },
 { 0 => (exporter): U32, 1 => (importer): U32, 2 => (resource): U32, 3 => (import_ordinal): U32, 4 => (export_ordinal): U32, 5 => (convoys_assigned): I32, 6 => (convoys_required): I32, 7 => (efficiency_a): I64, 8 => (efficiency_b): I64, 9 => (delivered): I64, 10 => (required_factories): I32, 11 => (assigned_factories): I32, 12 => (no_trade_influence): U8, 13 => (no_resources): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PurchaseContractRow {
    pub id: u64,
    pub seller: u32,
    pub buyer: u32,
    pub speed: i32,
    pub convoys_assigned: i32,
    pub convoys_total: i32,
    pub route_efficiency_sum_raw: i64,
    pub route_days: i32,
    pub factory_cic_progress_raw: i64,
    pub equipment_transfer_progress_raw: i64,
    pub collected_raw: i64,
    pub completed_deliveries: i32,
}
typed_codec!(PurchaseContractRow, 64, |row| row.id as u128, |key| Self { id: key as u64, seller: Default::default(), buyer: Default::default(), speed: Default::default(), convoys_assigned: Default::default(), convoys_total: Default::default(), route_efficiency_sum_raw: Default::default(), route_days: Default::default(), factory_cic_progress_raw: Default::default(), equipment_transfer_progress_raw: Default::default(), collected_raw: Default::default(), completed_deliveries: Default::default() },
 { 0 => (seller): U32, 1 => (buyer): U32, 2 => (speed): I32, 3 => (convoys_assigned): I32, 4 => (convoys_total): I32, 5 => (route_efficiency_sum_raw): I64, 6 => (route_days): I32, 7 => (factory_cic_progress_raw): I64, 8 => (equipment_transfer_progress_raw): I64, 9 => (collected_raw): I64, 10 => (completed_deliveries): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct BuildingRow {
    pub location: u32,
    pub building_type: u32,
    pub ordinal: u32,
    pub status_flags: u32,
    pub country: u32,
    pub level: i16,
    pub healthy_level: i16,
    pub partial_health: i64,
    pub active: bool,
}
typed_codec!(BuildingRow, 64, |row| (row.location as u128) << 32 | row.building_type as u128, |key| Self { location: (key >> 32) as u32, building_type: key as u32, ordinal: Default::default(), status_flags: Default::default(), country: Default::default(), level: Default::default(), healthy_level: Default::default(), partial_health: Default::default(), active: Default::default() },
 { 0 => (ordinal): U32, 1 => (status_flags): U32, 2 => (country): U32, 3 => (level): I16, 4 => (healthy_level): I16, 5 => (partial_health): I64, 6 => (active): Bool }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateBaseResourceRow {
    pub state: u32,
    pub resource: u32,
    pub ordinal: u32,
    pub amount: i64,
}
typed_codec!(StateBaseResourceRow, 64, |row| (row.state as u128) << 32 | row.resource as u128, |key| Self { state: (key >> 32) as u32, resource: key as u32, ordinal: Default::default(), amount: Default::default() },
 { 0 => (ordinal): U32, 1 => (amount): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct BuildingConstructionRow {
    pub id: u64,
    pub country: u32,
    pub queue_ordinal: u32,
    pub active_factories: i32,
    pub cost: i64,
    pub speed: i64,
    pub produced: i64,
    pub priority: i32,
    pub amount: i32,
    pub created_hour: i32,
    pub building_type: u32,
    pub target_flags: u32,
    pub target_location: u32,
    pub target_state: u32,
    pub target_country: u32,
    pub repair_levels: i32,
}
typed_codec!(BuildingConstructionRow, 64, |row| row.id as u128, |key| Self { id: key as u64, country: Default::default(), queue_ordinal: Default::default(), active_factories: Default::default(), cost: Default::default(), speed: Default::default(), produced: Default::default(), priority: Default::default(), amount: Default::default(), created_hour: Default::default(), building_type: Default::default(), target_flags: Default::default(), target_location: Default::default(), target_state: Default::default(), target_country: Default::default(), repair_levels: Default::default() },
 { 0 => (country): U32, 1 => (queue_ordinal): U32, 2 => (active_factories): I32, 3 => (cost): I64, 4 => (speed): I64, 5 => (produced): I64, 6 => (priority): I32, 7 => (amount): I32, 8 => (created_hour): I32, 9 => (building_type): U32, 10 => (target_flags): U32, 11 => (target_location): U32, 12 => (target_state): U32, 13 => (target_country): U32, 14 => (repair_levels): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RailwayConstructionRow {
    pub id: u64,
    pub country: u32,
    pub queue_ordinal: u32,
    pub active_factories: i32,
    pub cost: i64,
    pub speed: i64,
    pub produced: i64,
    pub priority: i32,
    pub amount: i32,
    pub created_hour: i32,
    pub base_level: i32,
    pub target_level: i32,
    pub cancelled: u8,
    pub current_segment: i32,
    pub remaining_cost: i64,
    pub route: Vec<u32>,
}
typed_codec!(RailwayConstructionRow, 64, |row| row.id as u128, |key| Self { id: key as u64, country: Default::default(), queue_ordinal: Default::default(), active_factories: Default::default(), cost: Default::default(), speed: Default::default(), produced: Default::default(), priority: Default::default(), amount: Default::default(), created_hour: Default::default(), base_level: Default::default(), target_level: Default::default(), cancelled: Default::default(), current_segment: Default::default(), remaining_cost: Default::default(), route: Default::default() },
 { 0 => (country): U32, 1 => (queue_ordinal): U32, 2 => (active_factories): I32, 3 => (cost): I64, 4 => (speed): I64, 5 => (produced): I64, 6 => (priority): I32, 7 => (amount): I32, 8 => (created_hour): I32, 9 => (base_level): I32, 10 => (target_level): I32, 11 => (cancelled): U8, 12 => (current_segment): I32, 13 => (remaining_cost): I64, 14 => (route): U32List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateResourceInputRow {
    pub local_manpower: Recorded<Option<i64>>,
    pub local_noncore_manpower: Recorded<Option<i64>>,
    pub recruitment_add: Recorded<Option<i64>>,
    pub recruitment_factor: Recorded<Option<i64>>,
    pub state: u32,
    pub gate: u8,
    pub global_factor: i64,
    pub origin: u8,
    pub infrastructure_level: Option<i32>,
    pub infrastructure_modifier: i64,
    pub supply_route: u8,
}
typed_codec!(StateResourceInputRow, 32, |row| row.state as u128,
 |key| Self { state: key as u32, ..Default::default() },
 { 0 => (gate): U8, 1 => (global_factor): I64, 2 => (origin): U8, 3 => (infrastructure_level): OptionalI32, 4 => (infrastructure_modifier): I64, 5 => (supply_route): U8, 6 => (local_manpower): OptionalI64, 7 => (local_noncore_manpower): OptionalI64, 8 => (recruitment_add): OptionalI64, 9 => (recruitment_factor): OptionalI64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateBuildingInputRow {
    pub state: u32,
    pub owner: u32,
    pub controller: u32,
    pub extra_shared_slots: i32,
    pub local_building_slots: Option<i64>,
    pub local_building_slots_factor: Option<i64>,
    pub global_building_slots: Option<i64>,
    pub global_building_slots_factor: Option<i64>,
    pub local_factories: Option<i64>,
    pub local_factory_energy_consumption: Option<i64>,
    pub local_factory_energy_consumption_per_infrastructure: Option<i64>,
    pub factory_energy_consumption: Option<i64>,
    pub factory_energy_factor: Option<i64>,
}
typed_codec!(StateBuildingInputRow, 32, |row| row.state as u128,
 |key| Self { state: key as u32, ..Default::default() },
 { 0 => (owner): U32, 1 => (controller): U32, 2 => (extra_shared_slots): I32, 3 => (local_building_slots): OptionalI64, 4 => (local_building_slots_factor): OptionalI64, 5 => (global_building_slots): OptionalI64, 6 => (global_building_slots_factor): OptionalI64, 7 => (local_factories): OptionalI64, 8 => (local_factory_energy_consumption): OptionalI64, 9 => (local_factory_energy_consumption_per_infrastructure): OptionalI64, 10 => (factory_energy_consumption): OptionalI64, 11 => (factory_energy_factor): OptionalI64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateResourceRow {
    pub state: u32,
    pub resource: u32,
    pub pool: i64,
    pub additive: i64,
    pub factor: i64,
    pub cost: i64,
}
typed_codec!(StateResourceRow, 64, |row| (row.state as u128) << 32 | row.resource as u128,
 |key| Self { state: (key >> 32) as u32, resource: key as u32, ..Default::default() },
 { 0 => (pool): I64, 1 => (additive): I64, 2 => (factor): I64, 3 => (cost): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryBuildingResourceRow {
    pub country: u32,
    pub ordinal: u32,
    pub entry: u32,
    pub building: u32,
    pub count: u32,
    pub resource: u32,
    pub amount: i64,
}
typed_codec!(CountryBuildingResourceRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (entry): U32, 1 => (building): U32, 2 => (count): U32, 3 => (resource): U32, 4 => (amount): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResourceOriginRow {
    pub ordinal: u32,
    pub state: u32,
    pub country: u32,
    pub list: u8,
    pub efficiency: i64,
    pub route_factor: i64,
    pub local_resources_factor: i64,
    pub bad: bool,
    pub convoys_assigned: i32,
    pub convoys_required: i32,
    pub convoys_base: Recorded<i32>,
}
typed_codec!(ResourceOriginRow, 64, |row| (row.state as u128) << 32 | row.country as u128,
 |key| Self { state: (key >> 32) as u32, country: key as u32, ..Default::default() },
 { 0 => (list): U8, 1 => (efficiency): I64, 2 => (route_factor): I64, 3 => (local_resources_factor): I64, 4 => (bad): Bool, 5 => (convoys_assigned): I32, 6 => (convoys_required): I32, 7 => (ordinal): U32, 8 => (convoys_base): I32 },
 |row| if row.list > 1 { Err(invalid("resource origin list is neither own (0) nor rights (1)")) } else { Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResourceRightRow {
    pub ordinal: u32,
    pub state: u32,
    pub resource: u32,
    pub holder: u32,
}
typed_codec!(ResourceRightRow, 64, |row| (row.state as u128) << 32 | row.resource as u128,
 |key| Self { state: (key >> 32) as u32, resource: key as u32, ..Default::default() },
 { 0 => (holder): U32, 1 => (ordinal): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryOilFactorRow {
    pub country: u32,
    pub factor: i64,
}
typed_codec!(CountryOilFactorRow, 32, |row| row.country as u128,
 |key| Self { country: key as u32, ..Default::default() },
 { 0 => (factor): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateTemporaryResourceRow {
    pub state: u32,
    pub resource: u32,
    pub amount: i64,
}
typed_codec!(StateTemporaryResourceRow, 64, |row| (row.state as u128) << 32 | row.resource as u128,
 |key| Self { state: (key >> 32) as u32, resource: key as u32, ..Default::default() },
 { 0 => (amount): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ConvoyClientRow {
    pub owner_ordinal: Option<u32>,
    pub other_ordinal: Option<u32>,
    pub handle: u32,
    pub category: u8,
    pub owner: u32,
    pub other: u32,
    pub route_class: u8,
    pub route_country: u32,
    pub route_other: u32,
    pub bad: bool,
    pub convoys_assigned: i32,
    pub convoys_required: i32,
    pub efficiency_a: Recorded<i64>,
    pub efficiency_b: Recorded<i64>,
    pub convoys_base: Recorded<i32>,
}
typed_codec!(ConvoyClientRow, 32, |row| row.handle as u128,
 |key| Self { handle: key as u32, ..Default::default() },
 { 0 => (category): U8, 1 => (owner): U32, 2 => (other): U32, 3 => (route_class): U8, 4 => (route_country): U32, 5 => (route_other): U32, 6 => (bad): Bool, 7 => (convoys_assigned): I32, 8 => (convoys_required): I32, 9 => (owner_ordinal): OptionalU32, 10 => (other_ordinal): OptionalU32, 11 => (efficiency_a): I64, 12 => (efficiency_b): I64, 13 => (convoys_base): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RegionalConvoyEfficiencyRow {
    pub country: u32,
    pub region: u32,
    pub efficiency: i64,
}
typed_codec!(RegionalConvoyEfficiencyRow, 64, |row| (row.country as u128) << 32 | row.region as u128,
 |key| Self { country: (key >> 32) as u32, region: key as u32, ..Default::default() },
 { 0 => (efficiency): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StrategicRegionWeightRow {
    pub region: u32,
    pub weight: i32,
}
typed_codec!(StrategicRegionWeightRow, 32, |row| row.region as u128,
 |key| Self { region: key as u32, ..Default::default() },
 { 0 => (weight): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TradeRouteRow {
    pub country: u32,
    pub class: u8,
    pub other: u32,
    pub kind: u8,
    pub from: u32,
    pub to: u32,
    pub path: Vec<u32>,
    pub source_state: Option<u32>,
    pub destination_state: Option<u32>,
}
typed_codec!(TradeRouteRow, 72, |row| (row.country as u128) << 40 | (row.class as u128) << 32 | row.other as u128,
 |key| Self { country: (key >> 40) as u32, class: (key >> 32) as u8, other: key as u32, ..Default::default() },
 { 0 => (kind): U8, 1 => (from): U32, 2 => (to): U32, 3 => (path): U32List, 4 => (source_state): OptionalU32, 5 => (destination_state): OptionalU32 }, |_row| Ok(()));
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StockpileArchetypeRow {
    pub canonical_slot: u32,
    pub archetype: u32,
    pub units: i32,
}
typed_codec!(StockpileArchetypeRow, 64, |row| (row.canonical_slot as u128) << 32 | row.archetype as u128, |key| Self { canonical_slot: (key >> 32) as u32, archetype: key as u32, ..Default::default() },
 { 0 => (units): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionLicenseRow {
    pub country: u32,
    pub ordinal: u32,
    pub variant: u64,
}
typed_codec!(ProductionLicenseRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, variant: Default::default() },
 { 0 => (variant): U64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionLicenseRecordRow {
    pub owner: u32,
    pub ordinal: u32,
    pub giver: u32,
    pub lended_cic: i32,
    pub required_cic: i32,
    pub start_date: i32,
    pub parent: u64,
    pub operand_raw: i64,
    pub equipment: Vec<u64>,
}
typed_codec!(ProductionLicenseRecordRow, 64, |row| (row.owner as u128) << 32 | row.ordinal as u128, |key| Self { owner: (key >> 32) as u32, ordinal: key as u32, giver: Default::default(), lended_cic: Default::default(), required_cic: Default::default(), start_date: Default::default(), parent: Default::default(), operand_raw: Default::default(), equipment: Default::default() },
 { 0 => (giver): U32, 1 => (lended_cic): I32, 2 => (required_cic): I32, 3 => (start_date): I32, 4 => (parent): U64, 5 => (operand_raw): I64, 6 => (equipment): U64List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DeploymentConveyorRow {
    pub handle: u64,
    pub country: u32,
    pub ordinal: u32,
    pub template: u64,
    pub amount: i32,
    pub location: u32,
    pub priority: u32,
    pub role_token: u32,
    pub closed: u8,
    pub name: Vec<u8>,
    pub exile_country: Option<u32>,
}
typed_codec!(DeploymentConveyorRow, 64, |row| row.handle as u128, |key| Self { handle: key as u64, country: Default::default(), ordinal: Default::default(), template: Default::default(), amount: Default::default(), location: Default::default(), priority: Default::default(), role_token: Default::default(), closed: Default::default(), name: Default::default(), exile_country: Default::default() },
 { 0 => (country): U32, 1 => (ordinal): U32, 2 => (template): U64, 3 => (amount): I32, 4 => (location): U32, 5 => (priority): U32, 6 => (role_token): U32, 7 => (closed): U8, 8 => (name): Bytes, 9 => (exile_country): OptionalU32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DeploymentLineRow {
    pub handle: u64,
    pub conveyor: u64,
    pub country: u32,
    pub ordinal: u32,
    pub name_order: i32,
    pub amount: i32,
    pub training_raw: i64,
    pub max_training_raw: i64,
    pub max_manpower: i32,
    pub allow_zero_entries: u8,
    pub equipment_allocations: i32,
    pub distributable_priority: i32,
    pub halting_reason_present: u8,
    pub halting_reason: i32,
    pub custom_name: Vec<u8>,
    pub custom_flag_a: u8,
    pub custom_flag_b: u8,
}
typed_codec!(DeploymentLineRow, 64, |row| row.handle as u128, |key| Self { handle: key as u64, conveyor: Default::default(), country: Default::default(), ordinal: Default::default(), name_order: Default::default(), amount: Default::default(), training_raw: Default::default(), max_training_raw: Default::default(), max_manpower: Default::default(), allow_zero_entries: Default::default(), equipment_allocations: Default::default(), distributable_priority: Default::default(), halting_reason_present: Default::default(), halting_reason: Default::default(), custom_name: Default::default(), custom_flag_a: Default::default(), custom_flag_b: Default::default() },
 { 0 => (conveyor): U64, 1 => (country): U32, 2 => (ordinal): U32, 3 => (name_order): I32, 4 => (amount): I32, 5 => (training_raw): I64, 6 => (max_training_raw): I64, 7 => (max_manpower): I32, 8 => (allow_zero_entries): U8, 9 => (equipment_allocations): I32, 10 => (distributable_priority): I32, 11 => (halting_reason_present): U8, 12 => (halting_reason): I32, 13 => (custom_name): Bytes, 14 => (custom_flag_a): U8, 15 => (custom_flag_b): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DeploymentManpowerRow {
    pub line: u64,
    pub side: u8,
    pub ordinal: u32,
    pub source_country: u32,
    pub amount: i32,
}
typed_codec!(DeploymentManpowerRow, 97, |row| (row.line as u128) << 33 | (row.side as u128) << 32 | row.ordinal as u128, |key| Self { line: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, ordinal: key as u32, source_country: Default::default(), amount: Default::default() },
 { 0 => (source_country): U32, 1 => (amount): I32 }, |row| if row.side > 1 { Err(invalid("deployment manpower side exceeds one")) } else { Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DeploymentEquipmentRow {
    pub line: u64,
    pub ordinal: u32,
    pub variant_handle: u64,
    pub amount_raw: i64,
}
typed_codec!(DeploymentEquipmentRow, 96, |row| (row.line as u128) << 32 | row.ordinal as u128, |key| Self { line: (key >> 32) as u64, ordinal: key as u32, variant_handle: Default::default(), amount_raw: Default::default() },
 { 0 => (variant_handle): U64, 1 => (amount_raw): I64 }, |_row| Ok(()));

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LogisticsEquipmentRow {
    pub country: u32,
    pub archetype: u32,
    pub token: u32,
    pub balance_raw: i64,
    pub stock_raw: i64,
    pub available_raw: Recorded<Option<i64>>,
}
typed_codec!(LogisticsEquipmentRow, 64, |row| (row.country as u128) << 32 | row.archetype as u128, |key| Self { country: (key >> 32) as u32, archetype: key as u32, ..Default::default() },
 { 0 => (token): U32, 1 => (balance_raw): I64, 2 => (stock_raw): I64, 3 => (available_raw): OptionalI64 }, |row| {
    if matches!(row.available_raw, Recorded::Value(Some(value)) if value < 0) { return Err(invalid("logistics available operand is negative")) }
    Ok(())
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantBonusSnapshotRow {
    pub id: u64,
    pub source: VariantBonusSource,
}
typed_codec!(VariantBonusSnapshotRow, 64, |row| row.id as u128,
|key| Self { id: key as u64, ..Default::default() }, { 0 => (source): Opaque }, |row| {
    if row.id == 0
        || row.source.stats.iter().any(|entry| entry.index >= engine_reference_values::native_values::equipment_variant::STAT_COUNT as u32) {
        return Err(invalid("invalid applied equipment bonus snapshot"));
    }
    Ok(())
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DesignObserverContextRow {
    pub fog: u8,
}
typed_codec!(DesignObserverContextRow, 0, |_row| 0, |_key| Self::default(),
 { 0 => (fog): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IndustrialOrganisationDetailsRow {
    pub organisation: u64,
    pub country: u32,
    pub state: IndustrialOrganisationState,
    pub unlocked: Vec<u32>,
    pub queued: Vec<u32>,
    pub allowed_policies: Vec<u32>,
    pub tasks: Vec<IndustrialOrganisationTask>,
    pub variables: IndustrialOrganisationVariables,
    pub flags: Vec<game_model::script_flag::Row>,
}
typed_codec!(IndustrialOrganisationDetailsRow, 64, |row| row.organisation as u128,
    |key| Self { organisation: key as u64, ..Default::default() }, {
        0 => (country): U32, 1 => (state.funds_raw): I64,
        2 => (state.size): I32, 3 => (state.points): I32,
        4 => (state.research_bonus_raw): I64, 5 => (state.task_capacity): I32,
        6 => (state.research_assign_cost_raw): I64, 7 => (state.production_assign_cost_raw): I64,
        8 => (state.design_team_change_cost): I32, 9 => (state.funds_gain_factor_raw): I64,
        10 => (state.size_cost_factor_raw): I64, 11 => (state.upgrades): U8,
        12 => (state.queue_index): I32, 13 => (state.policy): U32,
        14 => (state.cooldown_hour): OptionalI32, 15 => (state.source_location): Opaque,
        16 => (unlocked): U32List, 17 => (queued): U32List, 18 => (allowed_policies): U32List,
        19 => (tasks): Opaque, 20 => (variables): Opaque, 21 => (flags): Opaque
    }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IndustrialOrganisationPoliciesRow {
    pub country: u32,
    pub policies: Vec<IndustrialOrganisationPolicy>,
    pub cost_modifier_raw: Recorded<Option<i64>>,
}
typed_codec!(IndustrialOrganisationPoliciesRow, 32, |row| row.country as u128,
    |key| Self { country: key as u32, ..Default::default() },
    { 0 => (policies): Opaque, 1 => (cost_modifier_raw): OptionalI64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryMarketRow {
    pub country: u32,
    pub inventory: SharedMarketPool,
    pub prices: SharedMarketPriceLevels,
    pub subsidies: SharedMarketSubsidies,
    pub subsidy_total_raw: i64,
    pub auto_accept_market_access: RetiredBool,
    pub auto_send_market_access: bool,
    pub auto_accept_purchase: RetiredBool,
}
typed_codec!(CountryMarketRow, 32, |row| row.country as u128,
 |key| Self { country: key as u32, ..Default::default() },
 { 0 => (inventory): Opaque, 1 => (prices): Opaque, 2 => (subsidies): Opaque,
   3 => (subsidy_total_raw): I64, 4 => (auto_accept_market_access): Bool,
   5 => (auto_send_market_access): Bool, 6 => (auto_accept_purchase): Bool }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct MarketContractTermsRow {
    pub runtime: MarketContractRuntime,
    pub variables: SharedMarketVariables,
    pub variable_state: u64,
    pub country: u32,
    pub id: u64,
    pub kind: u8,
    pub buyer: u32,
    pub speed: i32,
    pub equipment: SharedMarketPool,
    pub prices: SharedMarketPool,
    pub price_levels: SharedMarketPriceLevels,
    pub subsidies: SharedMarketSubsidies,
}
typed_codec!(MarketContractTermsRow, 104,
 |row| (row.country as u128) << 72 | (row.id as u128) << 8 | row.kind as u128,
 |key| Self { country: (key >> 72) as u32, id: (key >> 8) as u64, kind: key as u8, ..Default::default() },
 { 0 => (buyer): U32, 1 => (speed): I32, 2 => (equipment): Opaque,
   3 => (prices): Opaque, 4 => (price_levels): Opaque, 5 => (subsidies): Opaque,
   6 => (runtime.route_equipment): Opaque, 7 => (runtime.route_length): I32,
   8 => (runtime.route_type): U8, 9 => (runtime.route_request): I32,
   10 => (runtime.route_sunk_convoys_raw): I64, 11 => (runtime.period_days): I32,
   12 => (runtime.meta_cic_raw): I64, 13 => (runtime.meta_convoys_raw): I64,
   14 => (runtime.meta_efficiency_raw): I64, 15 => (runtime.history_factory_raw): I64,
   16 => (runtime.history_equipment_raw): I64, 17 => (runtime.history_collected_raw): I64,
   18 => (variables): Opaque, 19 => (variable_state): U64 }, |row| if row.kind<=1 {Ok(())} else {Err(replay_rows::codec::invalid("market terms kind unsupported"))});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LendLeaseAgreementRow {
    pub country: u32,
    pub id: u64,
    pub receiver: u32,
    pub revision: u64,
    pub active_since: i32,
    pub last_delivery: i32,
    pub monthly: SharedMarketPool,
    pub percentage: SharedMarketPool,
    pub once: SharedMarketPool,
    pub delivered: SharedMarketPool,
    pub sunk: SharedMarketPool,
    pub fuel_daily_raw: i64,
    pub fuel_percentage_raw: i64,
    pub fuel_sent_raw: i64,
    pub fuel_sunk_raw: i64,
    pub last_fuel_delivered_raw: i64,
}
typed_codec!(LendLeaseAgreementRow, 96,
 |row| (row.country as u128) << 64 | row.id as u128,
 |key| Self { country: (key >> 64) as u32, id: key as u64, ..Default::default() },
 { 0 => (receiver): U32, 1 => (revision): U64, 2 => (active_since): I32,
   3 => (last_delivery): I32, 4 => (monthly): Opaque, 5 => (percentage): Opaque,
   6 => (once): Opaque, 7 => (delivered): Opaque, 8 => (sunk): Opaque,
   9 => (fuel_daily_raw): I64, 10 => (fuel_percentage_raw): I64,
   11 => (fuel_sent_raw): I64, 12 => (fuel_sunk_raw): I64,
   13 => (last_fuel_delivered_raw): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct MarketEventRow {
    pub id: u64,
    pub occurrence: u64,
    pub hour: i32,
    pub agreement: u64,
    pub country: u32,
    pub receiver: u32,
    pub kind: u8,
    pub mode: u8,
    pub ordinal: u32,
    pub variant: u64,
    pub amount_raw: i64,
    pub auxiliary_raw: i64,
    pub total_raw: i64,
    pub lost_total_raw: i64,
    pub date: i32,
    pub flags: u8,
}
impl MarketEventRow {
    pub fn delivery_unavailable_reason(&self) -> Option<&'static str> {
        if self.kind != 34 {
            return None;
        }
        Some(match self.ordinal {
            1 => "Delivery amount unavailable: required native cumulative total unreadable",
            2 => "Delivery amount unavailable: cumulative baseline uncertified",
            3 => "Delivery amount unavailable: final cumulative snapshot incomplete",
            4 => "Delivery amount unavailable: purchase manifest not acquired before removal",
            5 => "Delivery amount unavailable: holder erased without its final totals witness",
            _ => "Delivery amount unavailable: unsupported refusal reason",
        })
    }
}

typed_codec!(MarketEventRow, 64, |row| row.id as u128,
 |key| Self { id: key as u64, ..Default::default() },
 { 0 => (occurrence): U64, 1 => (hour): I32, 2 => (agreement): U64,
   3 => (country): U32, 4 => (receiver): U32, 5 => (kind): U8,
   6 => (mode): U8, 7 => (ordinal): U32, 8 => (variant): U64,
   9 => (amount_raw): I64, 10 => (auxiliary_raw): I64, 11 => (date): I32,
   12 => (flags): U8, 13 => (total_raw): I64, 14 => (lost_total_raw): I64 }, |row| if (row.kind<=30 && row.kind!=19) || row.kind==34 {Ok(())} else {Err(replay_rows::codec::invalid("market event kind unsupported"))});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct EquipmentDestructionRow {
    pub country: u32,
    pub cause: EquipmentDestructionCause,
    pub variant: u64,
    pub amount_raw: i64,
    pub coverage: Recorded<EquipmentDestructionCoverage>,
    pub unavailable_reason: Recorded<String>,
}
typed_codec!(EquipmentDestructionRow, 104,
|row| (row.country as u128) << 72 | (row.cause as u128) << 64 | row.variant as u128,
|key| Self { country: (key >> 72) as u32, cause: EquipmentDestructionCause::try_from((key >> 64) as u8)?, variant: key as u64, ..Default::default() },
{ 0 => (amount_raw): I64, 1 => (coverage): U8, 2 => (unavailable_reason): Bytes }, |row| {
    match (&row.coverage, &row.unavailable_reason) {
        (Recorded::NotRecorded, Recorded::NotRecorded) if row.amount_raw > 0 => Ok(()),
        (Recorded::Value(EquipmentDestructionCoverage::Complete), Recorded::Value(reason)) if row.key() != 0 && row.amount_raw != 0 && reason.is_empty() => Ok(()),
        (Recorded::Value(EquipmentDestructionCoverage::SupplementalUnavailable | EquipmentDestructionCoverage::PartialInterval), Recorded::Value(reason)) if row.key() == 0 && row.amount_raw == 0 && !reason.is_empty() => Ok(()),
        _ => Err(invalid("invalid equipment destruction amount or availability marker")),
    }
});

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryProductionModifiersRow {
    pub country: u32,
    pub power_raw: i64,
    pub base_start: i32,
    pub base_cap: i32,
    pub gain_raw: i64,
    pub balance_raw: i64,
    pub factory_speed: [i64; 4],
    pub retention: [i64; 4],
    pub conversion_raw: i64,
    pub trees: Vec<ProductionModifierTree>,
}
typed_codec!(CountryProductionModifiersRow, 32, |row| row.country as u128,
|key| Self { country: key as u32, ..Default::default() },
{ 0 => (power_raw): I64, 1 => (base_start): I32, 2 => (base_cap): I32,
  3 => (gain_raw): I64, 4 => (balance_raw): I64, 5 => (factory_speed): Opaque,
  6 => (trees): Opaque, 7 => (retention): Opaque, 8 => (conversion_raw): I64 }, |row| {
    if row.trees.len() != engine_reference_values::native_values::production_modifiers::KEYS.len()
        || row.trees.iter().zip(engine_reference_values::native_values::production_modifiers::KEYS).any(|(tree, key)| tree.key != key) {
        return Err(invalid("production modifier keys differ from native request catalogue"));
    }
    for tree in &row.trees {
        if tree.failure.is_empty() { check_tree(&tree.nodes)?; }
        else if !tree.nodes.is_empty() { return Err(invalid("failed production modifier tree retains partial nodes")); }
    }
    Ok(())
});

impl CountryProductionModifiersRow {
    pub fn modifier(&self, key: u32) -> Option<i64> {
        let tree = self.trees.iter().find(|tree| tree.key == key)?;
        if tree.failure.is_empty() { tree.nodes.first().map(|node| node.value) } else { None }
    }

    pub fn blended_speed(&self, power_modifier: i64, naval: bool) -> Option<i64> {
        let power = multiply(power_modifier.wrapping_add(100_000), self.power_raw, 100_000)?.max(0);
        let [unpowered, powered] = if naval { [self.factory_speed[2], self.factory_speed[3]] } else { [self.factory_speed[0], self.factory_speed[1]] };
        Some(unpowered.wrapping_add(multiply(power, powered.wrapping_sub(unpowered), 100_000)?))
    }

    pub fn factory_output(&self, naval: bool) -> Option<i64> {
        use engine_reference_values::modifier_keys as keys;
        let power_modifier = self.modifier(keys::OUT_OF_POWER_IMPACT_FACTOR)?;
        let base = self.blended_speed(power_modifier, naval)?;
        let (output, powered) = if naval {
            (keys::INDUSTRIAL_CAPACITY_DOCKYARD, keys::INDUSTRIAL_CAPACITY_DOCKYARD_POWERED)
        } else {
            (keys::INDUSTRIAL_CAPACITY_FACTORY, keys::INDUSTRIAL_CAPACITY_FACTORY_POWERED)
        };
        let modifier = self.modifier(output)?;
        let mut total = base.wrapping_add(multiply(base, modifier, 100_000)?);
        if self.power_raw > 99_999 {
            total = total.wrapping_add(multiply(base, self.modifier(powered)?, 100_000)?);
        } else if modifier > 0 {
            let power = multiply(power_modifier.wrapping_add(100_000), self.power_raw, 100_000)?.max(0);
            let adjustment = multiply(100_000_i64.wrapping_sub(power), modifier.wrapping_neg(), 100_000)?;
            total = total.wrapping_add(multiply(base, adjustment, 100_000)?);
        }
        Some(total)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionLineModifiersRow {
    pub line: u64,
    pub country: u32,
    pub variant: u64,
    pub organisation: u64,
    pub kind: u8,
    pub active_factories: i32,
    pub resource_penalty_raw: i64,
    pub organisation_dirty: bool,
    pub conversion: RetiredBool,
    pub licensed: bool,
    pub conversion_output: bool,
    pub refit_output: bool,
    pub naval_efficiency: bool,
    pub naval_output: bool,
    pub bonuses: [i64; 7],
    pub factory_efficiencies: Vec<i64>,
    pub converting: Recorded<bool>,
}
typed_codec!(ProductionLineModifiersRow, 64, |row| row.line as u128,
|key| Self { line: key as u64, ..Default::default() },
{ 0 => (country): U32, 1 => (variant): U64, 2 => (organisation): U64, 3 => (kind): U8,
  4 => (active_factories): I32, 5 => (resource_penalty_raw): I64, 6 => (organisation_dirty): Bool,
  7 => (conversion): Bool, 8 => (bonuses): Opaque, 9 => (factory_efficiencies): Opaque,
  10 => (naval_efficiency): Bool, 11 => (naval_output): Bool, 12 => (licensed): Bool,
  13 => (conversion_output): Bool, 14 => (refit_output): Bool, 15 => (converting): Bool },
|row| if row.kind > engine_reference_values::native_values::production_line::KIND_RAILWAY_GUN {
    Err(invalid("production modifier line identity invalid"))
} else { Ok(()) });
