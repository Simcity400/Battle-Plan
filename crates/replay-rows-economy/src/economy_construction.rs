use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RocketProductionOutputRow {
    pub country: u32,
    pub variant: u64,
    pub spent_raw: i64,
    pub instant_raw: i64,
    pub since_hour: i32,
    pub exact: bool,
    pub ticks: u64,
    pub inexact_ticks: u64,
    pub unknown_design_ticks: u64,
    pub unknown_country_ticks: u64,
    pub units: i64,
    pub units_exact: bool,
    pub deliveries: u64,
    pub inexact_deliveries: u64,
}
typed_codec!(RocketProductionOutputRow, 96, |row| (row.country as u128) << 64 | row.variant as u128, |key| Self { country: (key >> 64) as u32, variant: key as u64, ..Default::default() },
{ 0 => (spent_raw): I64, 1 => (instant_raw): I64, 2 => (since_hour): I32, 3 => (exact): Bool, 4 => (ticks): U64, 5 => (inexact_ticks): U64, 6 => (unknown_design_ticks): U64, 7 => (unknown_country_ticks): U64, 8 => (units): I64, 9 => (units_exact): Bool, 10 => (deliveries): U64, 11 => (inexact_deliveries): U64 }, |row| {
   if row.inexact_ticks > row.ticks
       || row.unknown_design_ticks > row.inexact_ticks
       || row.unknown_country_ticks > row.inexact_ticks
       || row.inexact_deliveries > row.deliveries
       || (row.variant == 0 && (row.spent_raw != 0 || row.instant_raw != 0 || row.units != 0))
       || (row.country != 0 && row.unknown_country_ticks != 0)
       || (row.variant != 0 && row.unknown_design_ticks != 0)
       || (row.country == 0 && row.exact && row.unknown_country_ticks != 0)
       || (row.country != 0 && row.exact && row.inexact_ticks != 0)
       || (row.country != 0 && row.units_exact && row.inexact_deliveries != 0)
       || (row.variant != 0 && row.ticks == 0 && row.deliveries == 0)
       || (row.variant != 0 && row.country == 0 && row.unknown_country_ticks != row.ticks)
       || (row.variant != 0 && row.country == 0
           && (row.inexact_deliveries != row.deliveries || (row.deliveries != 0 && row.units_exact)))
       || (row.ticks == 0 && (row.spent_raw != 0 || row.instant_raw != 0))
       || (row.deliveries == 0 && row.units != 0)
   {
       return Err(invalid("rocket output scope/counts inconsistent"));
   }
   Ok(())
});

#[cfg(test)]
#[path = "rocket_output/tests.rs"]
mod rocket_output_tests;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ConstructionLineOutputRow {
    pub line: u64,
    pub railway: bool,
    pub construction_raw: i64,
    pub repair_raw: i64,
    pub bank_raw: i64,
    pub instant_raw: i64,
    pub carry_raw: i64,
    pub ticks: i32,
    pub repair_ticks: i32,
    pub since_hour: i32,
    pub observed_since_creation: bool,
    pub linked: bool,
    pub ended: bool,
    pub exact: bool,
    pub completion: Recorded<Option<ConstructionCompletion>>,
    pub count_reset: Recorded<i64>,
    pub country: Recorded<Option<u32>>,
    pub building_type: Recorded<Option<u32>>,
    pub target_flags: Recorded<Option<u32>>,
    pub target_location: Recorded<Option<u32>>,
    pub target_state: Recorded<Option<u32>>,
    pub target_country: Recorded<Option<u32>>,
    pub route_first: Recorded<Option<u32>>,
    pub route_last: Recorded<Option<u32>>,
}
typed_codec!(ConstructionLineOutputRow, 64, |row| row.line as u128, |key| Self { line: key as u64, ..Default::default() },
 { 0 => (railway): Bool, 1 => (construction_raw): I64, 2 => (repair_raw): I64, 3 => (bank_raw): I64, 4 => (instant_raw): I64, 5 => (carry_raw): I64, 6 => (ticks): I32, 7 => (since_hour): I32, 8 => (observed_since_creation): Bool, 9 => (linked): Bool, 10 => (ended): Bool, 11 => (exact): Bool, 12 => (repair_ticks): I32, 13 => (completion): Opaque, 14 => (count_reset): I64,
 15 => (country): OptionalU32, 16 => (building_type): OptionalU32, 17 => (target_flags): OptionalU32, 18 => (target_location): OptionalU32, 19 => (target_state): OptionalU32, 20 => (target_country): OptionalU32, 21 => (route_first): OptionalU32, 22 => (route_last): OptionalU32 }, |row| {
    if row.railway && matches!(row.count_reset, Recorded::Value(count) if count != 0) {
        return Err(invalid("railway output cannot contain a building count reset"));
    }
    if let Recorded::Value(Some(completion)) = row.completion
        && (!row.ended || completion.hour < row.since_hour || (completion.cancelled && !row.railway))
    {
        return Err(invalid("construction completion outcome inconsistent"));
    }
    Ok(())
});

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ConstructionCompletion {
    pub hour: i32,
    pub cancelled: bool,
}
value_struct!(ConstructionCompletion {
    hour: i32,
    cancelled: bool
});
impl OpaqueValue for ConstructionCompletion {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantDefinitionRow {
    pub id: u64,
    pub equipment_token: u32,
    pub creator_link: u32,
    pub origin_link: u32,
    pub append_variant_name: u8,
    pub obsolete: u8,
    pub classifier_flags: u64,
    pub mission_mask: u32,
    pub display_name: Vec<u8>,
    pub variant_name: Vec<u8>,
    pub explicit_entity: Vec<u8>,
    pub archetype_token: u32,
    pub icon_active: u8,
    pub icon_kind: u32,
    pub icon: Vec<u8>,
    pub version: Recorded<u8>,
    pub design: Recorded<VariantDesignState>,
}
typed_codec!(VariantDefinitionRow, 64, |row| row.id as u128, |key| Self { id: key as u64, ..Default::default() },
 { 0 => (equipment_token): U32, 1 => (creator_link): U32, 2 => (origin_link): U32, 3 => (append_variant_name): U8, 4 => (obsolete): U8, 5 => (classifier_flags): U64, 6 => (mission_mask): U32, 7 => (display_name): Bytes, 8 => (variant_name): Bytes, 9 => (explicit_entity): Bytes, 10 => (archetype_token): U32, 11 => (icon_active): U8, 12 => (icon_kind): U32, 13 => (icon): Bytes, 14 => (version): U8, 15 => (design): Opaque }, |row| check_design_state(&row.design));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LocalSubunitRequirement {
    pub archetype_token: u32,
    pub amount_raw: i64,
}
value_struct!(LocalSubunitRequirement {
    archetype_token: u32,
    amount_raw: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct UnitBonusTarget {
    pub category: bool,
    pub token: u32,
    pub combat_width_raw: i64,
    pub stats: Vec<UnitBonusStat>,
    pub grants: Vec<UnitBonusGrant>,
}
value_struct!(UnitBonusTarget {
    category: bool,
    token: u32,
    combat_width_raw: i64,
    stats: Vec<UnitBonusStat>,
    grants: Vec<UnitBonusGrant>
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct UnitBonusGrant {
    pub category_token: u32,
    pub add: u8,
    pub display_as_percentage: u8,
    pub stats: Vec<UnitBonusStat>,
}
value_struct!(UnitBonusGrant {
    category_token: u32,
    add: u8,
    display_as_percentage: u8,
    stats: Vec<UnitBonusStat>
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct UnitBonusStat {
    pub selector: u32,
    pub value_raw: i64,
}
value_struct!(UnitBonusStat {
    selector: u32,
    value_raw: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FuelAwardRecipient {
    pub kind: u32,
    pub reference: u64,
    pub owner: u32,
    pub host: u64,
    pub amount_raw: i64,
    pub name: Option<Vec<u8>>,
    pub name_reason: Vec<u8>,
}
value_struct!(FuelAwardRecipient { kind: u32, reference: u64, owner: u32, host: u64,
    amount_raw: i64, name: Option<Vec<u8>>, name_reason: Vec<u8> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FuelAwardHour {
    pub hour: i32,
    pub gains_raw: [i64; 4],
    pub own_gain_raw: i64,
    pub transfers_raw: [i64; 4],
    pub consumed_raw: i64,
    pub received_raw: [i64; 3],
    pub recipients: Vec<FuelAwardRecipient>,
    pub history_index: i32,
    pub remaining_hours: i32,
    pub pending_raw: [i64; 3],
    pub current_raw: i64,
}
value_struct!(FuelAwardHour { hour: i32, gains_raw: [i64; 4], own_gain_raw: i64,
    transfers_raw: [i64; 4], consumed_raw: i64, received_raw: [i64; 3],
    recipients: Vec<FuelAwardRecipient>, history_index: i32, remaining_hours: i32,
    pending_raw: [i64; 3], current_raw: i64 });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FuelAwardWindow {
    pub country_instance: u32,
    pub continuity_start_hour: Option<i32>,
    pub hours: Vec<FuelAwardHour>,
    pub reason: Vec<u8>,
}
value_struct!(FuelAwardWindow { country_instance: u32, continuity_start_hour: Option<i32>, hours: Vec<FuelAwardHour>, reason: Vec<u8> });
opaque_value!(FuelAwardWindow);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelAwardDayRecipient {
    pub kind: u32,
    pub reference: u64,
    pub owner: u32,
    pub host: u64,
    pub name: Option<Vec<u8>>,
    pub name_reason: Vec<u8>,
    pub first_hour: i32,
    pub last_hour: i32,
    pub amount_raw: i64,
}
value_struct!(FuelAwardDayRecipient { kind: u32, reference: u64, owner: u32, host: u64,
    name: Option<Vec<u8>>, name_reason: Vec<u8>, first_hour: i32, last_hour: i32,
    amount_raw: i64 });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelAwardDay {
    pub country_instance: u32,
    pub continuity_start_hour: Option<i32>,
    pub last_hour: i32,
    pub gains_raw: [i64; 4],
    pub own_gain_raw: i64,
    pub own_gain_quantity_raw: i64,
    pub transfers_raw: [i64; 4],
    pub consumed_raw: i64,
    pub received_raw: [i64; 3],
    pub recipients: Vec<FuelAwardDayRecipient>,
    pub reason: Vec<u8>,
}
value_struct!(FuelAwardDay { country_instance: u32, continuity_start_hour: Option<i32>,
    last_hour: i32, gains_raw: [i64; 4], own_gain_raw: i64, own_gain_quantity_raw: i64,
    transfers_raw: [i64; 4], consumed_raw: i64, received_raw: [i64; 3],
    recipients: Vec<FuelAwardDayRecipient>, reason: Vec<u8> });
opaque_value!(FuelAwardDay);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryFuelRow {
    pub country: u32,
    pub current_raw: i64,
    pub maximum_raw: i64,
    pub gain_raw: RetiredI64,
    pub cost_raw: RetiredI64,
    pub gain_from_states_raw: RetiredI64,
    pub lend_lease_gain_raw: i64,
    pub lend_lease_consumption_raw: RetiredI64,
    pub gain_per_oil_raw: RetiredI64,
    pub history_index: RetiredI32,
    pub remaining_hours: RetiredI32,
    pub ring_gain_raw: i64,
    pub ring_consumed_raw: i64,
    pub ring_other_raw: i64,
    pub ring_requested_0_raw: RetiredI64,
    pub ring_requested_1_raw: RetiredI64,
    pub ring_requested_2_raw: RetiredI64,
    pub ring_received_0_raw: i64,
    pub ring_received_1_raw: i64,
    pub ring_received_2_raw: i64,
    pub country_instance: Recorded<Option<u32>>,
    pub awards: Recorded<Retired>,
    pub demand: Recorded<FuelDemandRead>,
    pub award_day: Recorded<FuelAwardDay>,
}
typed_codec!(CountryFuelRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (current_raw): I64, 1 => (maximum_raw): I64, 2 => (gain_raw): I64, 3 => (cost_raw): I64, 4 => (gain_from_states_raw): I64, 5 => (lend_lease_gain_raw): I64, 6 => (lend_lease_consumption_raw): I64, 7 => (gain_per_oil_raw): I64, 8 => (history_index): I32, 9 => (remaining_hours): I32, 10 => (ring_gain_raw): I64, 11 => (ring_consumed_raw): I64, 12 => (ring_other_raw): I64, 13 => (ring_requested_0_raw): I64, 14 => (ring_requested_1_raw): I64, 15 => (ring_requested_2_raw): I64, 16 => (ring_received_0_raw): I64, 17 => (ring_received_1_raw): I64, 18 => (ring_received_2_raw): I64, 19 => (country_instance): OptionalU32, 20 => (awards): Opaque, 21 => (demand): Opaque, 22 => (award_day): Opaque }, |_row| Ok(()));

impl OpaqueValue for LocalSubunitRequirement {}
impl OpaqueValue for UnitBonusTarget {}

fn check_design_state(value: &Recorded<VariantDesignState>) -> io::Result<()> {
    let Recorded::Value(state) = value else {
        return Ok(());
    };
    if state.design_team_serialized && state.design_team == 0 {
        return Err(invalid("resolved design team has a null reference"));
    }
    if state.bonus_sources.contains(&0) || state.named_sources.contains(&0) {
        return Err(invalid("design has a null bonus snapshot reference"));
    }
    for entry in state.mission_stats.iter().flat_map(|source| &source.stats) {
        if entry.index >= engine_reference_values::native_values::equipment_variant::STAT_COUNT as u32 {
            return Err(invalid("design stat index outside native metadata"));
        }
    }
    let expected = (0..engine_reference_values::native_values::equipment_variant::STAT_COUNT as u32).filter(|index| {
        ![0x2e, 0x2f, 0x30, 0x31, 0x3e, 0x41, 0x43, 0x44, 0x45, 0x4d].contains(index)
    });
    if (state.named_bonuses.is_empty() && !state.unresolved_stats.is_empty())
        || (!state.named_bonuses.is_empty()
            && !state
                .unresolved_stats
                .iter()
                .map(|entry| entry.index)
                .eq(expected))
    {
        return Err(invalid(
            "named bonus stat residuals are incomplete or unordered",
        ));
    }
    Ok(())
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DesignCountryModifierRow {
    pub country: u32,
    pub modifier: u32,
    pub value: Option<i64>,
    pub failure: u8,
}
typed_codec!(DesignCountryModifierRow, 64,
|row| (row.country as u128) << 32 | row.modifier as u128,
|key| Self { country: (key >> 32) as u32, modifier: key as u32, ..Default::default() },
{ 0 => (value): OptionalI64, 1 => (failure): U8 }, |row| {
    if row.failure > 2 || (row.failure == 0) != row.value.is_some() {
        return Err(invalid("design country modifier availability inconsistent"));
    }
    Ok(())
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DesignCountryTemplatesRow {
    pub country: u32,
    pub uses: Vec<i32>,
    pub factors: Vec<i64>,
    pub equipment: Vec<Vec<u32>>,
    pub failure: u8,
}
impl DesignCountryTemplatesRow {
    pub fn entries(&self) -> impl Iterator<Item = (i32, i64, &[u32])> {
        self.uses
            .iter()
            .copied()
            .zip(self.factors.iter().copied())
            .zip(&self.equipment)
            .map(|((uses, factor), equipment)| (uses, factor, equipment.as_slice()))
    }
}
typed_codec!(DesignCountryTemplatesRow, 32, |row| row.country as u128,
|key| Self { country: key as u32, ..Default::default() },
{ 0 => (uses): Opaque, 1 => (factors): Opaque, 2 => (equipment): Opaque, 3 => (failure): U8 }, |row| {
    if row.failure > 2 || row.uses.len() != row.factors.len()
        || row.uses.len() != row.equipment.len() || (row.failure != 0 && !row.uses.is_empty()) {
        return Err(invalid("design country template availability inconsistent"));
    }
    Ok(())
});

pub fn variant_bonus_snapshot_id(source: &VariantBonusSource) -> u64 {
    struct Digest(u64);
    impl Digest {
        fn bytes(&mut self, bytes: &[u8]) {
            for byte in bytes {
                self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
            }
        }
        fn word(&mut self, word: u64) {
            self.bytes(&word.to_le_bytes());
        }
    }
    let mut digest = Digest(0xcbf2_9ce4_8422_2325);
    match &source.named {
        None => digest.word(0),
        Some(named) => {
            digest.word(1);
            digest.word(u64::from(named.owner));
            digest.word(u64::from(named.handle));
            digest.word(named.name.len() as u64);
            digest.bytes(&named.name);
            digest.word(named.description.len() as u64);
            digest.bytes(&named.description);
        }
    }
    match &source.idea {
        None => digest.word(0),
        Some(text) => {
            digest.word(1);
            digest.word(text.len() as u64);
            digest.bytes(text);
        }
    }
    match source.idea_token {
        None => digest.word(0),
        Some(token) => {
            digest.word(1);
            digest.word(u64::from(token));
        }
    }
    digest.word(source.stats.len() as u64);
    for entry in &source.stats {
        digest.word(u64::from(entry.index));
        digest.word(entry.value as u64);
    }
    digest.word(source.terrain_attrition.len() as u64);
    for value in &source.terrain_attrition {
        digest.word(*value as u64);
    }
    digest.0.max(1)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantSnapshotDefinition {
    pub id: u64,
    pub equipment_token: u32,
    pub creator_link: u32,
    pub origin_link: u32,
    pub append_variant_name: u8,
    pub obsolete: u8,
    pub classifier_flags: u64,
    pub mission_mask: u32,
    pub display_name: Vec<u8>,
    pub variant_name: Vec<u8>,
    pub explicit_entity: Vec<u8>,
    pub archetype_token: u32,
    pub icon_active: u8,
    pub icon_kind: u32,
    pub icon: Vec<u8>,
    pub version: Recorded<u8>,
    pub design: Recorded<VariantDesignState>,
}
value_struct!(VariantSnapshotDefinition { id: u64, equipment_token: u32, creator_link: u32, origin_link: u32, append_variant_name: u8, obsolete: u8, classifier_flags: u64, mission_mask: u32, display_name: Vec<u8>, variant_name: Vec<u8>, explicit_entity: Vec<u8>, archetype_token: u32, icon_active: u8, icon_kind: u32, icon: Vec<u8>, version: Recorded<u8>, design: Recorded<VariantDesignState> });
impl From<VariantDefinitionRow> for VariantSnapshotDefinition {
    fn from(value: VariantDefinitionRow) -> Self {
        Self {
            id: value.id,
            equipment_token: value.equipment_token,
            creator_link: value.creator_link,
            origin_link: value.origin_link,
            append_variant_name: value.append_variant_name,
            obsolete: value.obsolete,
            classifier_flags: value.classifier_flags,
            mission_mask: value.mission_mask,
            display_name: value.display_name,
            variant_name: value.variant_name,
            explicit_entity: value.explicit_entity,
            archetype_token: value.archetype_token,
            icon_active: value.icon_active,
            icon_kind: value.icon_kind,
            icon: value.icon,
            version: value.version,
            design: value.design,
        }
    }
}
impl From<VariantSnapshotDefinition> for VariantDefinitionRow {
    fn from(value: VariantSnapshotDefinition) -> Self {
        Self {
            id: value.id,
            equipment_token: value.equipment_token,
            creator_link: value.creator_link,
            origin_link: value.origin_link,
            append_variant_name: value.append_variant_name,
            obsolete: value.obsolete,
            classifier_flags: value.classifier_flags,
            mission_mask: value.mission_mask,
            display_name: value.display_name,
            variant_name: value.variant_name,
            explicit_entity: value.explicit_entity,
            archetype_token: value.archetype_token,
            icon_active: value.icon_active,
            icon_kind: value.icon_kind,
            icon: value.icon,
            version: value.version,
            design: value.design,
        }
    }
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantSnapshotModule {
    pub ordinal: u32,
    pub slot: u32,
    pub definition_token: u32,
    pub active: u8,
}
value_struct!(VariantSnapshotModule {
    ordinal: u32,
    slot: u32,
    definition_token: u32,
    active: u8
});
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantSnapshotUpgrade {
    pub ordinal: u32,
    pub definition_token: i32,
    pub level: u8,
}
value_struct!(VariantSnapshotUpgrade {
    ordinal: u32,
    definition_token: i32,
    level: u8
});
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantSnapshot {
    pub definition: VariantSnapshotDefinition,
    pub modules: Vec<VariantSnapshotModule>,
    pub stats: Vec<Option<i64>>,
    pub upgrades: Vec<VariantSnapshotUpgrade>,
}
value_struct!(VariantSnapshot { definition: VariantSnapshotDefinition, modules: Vec<VariantSnapshotModule>, stats: Vec<Option<i64>>, upgrades: Vec<VariantSnapshotUpgrade> });
impl OpaqueValue for VariantSnapshot {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantSourceReferences {
    pub direct: Vec<u64>,
    pub named: Vec<u64>,
}
value_struct!(VariantSourceReferences { direct: Vec<u64>, named: Vec<u64> });
impl OpaqueValue for VariantSourceReferences {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DesignOrganisationLivenessRow {
    pub organisation: u64,
    pub live: bool,
}
typed_codec!(DesignOrganisationLivenessRow, 64, |row| row.organisation as u128, |key| Self { organisation: key as u64, ..Default::default() },
    { 0 => (live): Bool }, |row| if row.organisation == 0 { Err(invalid("null design organisation liveness")) } else { Ok(()) });
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantOrganisationChange {
    pub organisation: u64,
    pub live: bool,
}
value_struct!(VariantOrganisationChange {
    organisation: u64,
    live: bool
});
impl OpaqueValue for VariantOrganisationChange {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantIconChange {
    pub active: u8,
    pub kind: u32,
    pub text: Vec<u8>,
}
value_struct!(VariantIconChange { active: u8, kind: u32, text: Vec<u8> });
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantScalarChange {
    pub obsolete: Option<u8>,
    pub auto_upgraded: Option<u8>,
    pub highlight: Option<u8>,
    pub max_version: Option<u8>,
    pub role_icon_index: Option<i32>,
    pub display_name: Option<Vec<u8>>,
    pub model: Option<Vec<u8>>,
    pub icon: Option<VariantIconChange>,
    pub names_group: Option<Option<Vec<u8>>>,
    pub niches: Option<Vec<u32>>,
}
value_struct!(VariantScalarChange {
    obsolete: Option<u8>, auto_upgraded: Option<u8>, highlight: Option<u8>,
    max_version: Option<u8>, role_icon_index: Option<i32>, display_name: Option<Vec<u8>>,
    model: Option<Vec<u8>>, icon: Option<VariantIconChange>,
    names_group: Option<Option<Vec<u8>>>, niches: Option<Vec<u32>>
});
impl OpaqueValue for VariantScalarChange {}
impl VariantScalarChange {
    pub fn changes_design(&self) -> bool {
        self.auto_upgraded.is_some()
            || self.highlight.is_some()
            || self.max_version.is_some()
            || self.role_icon_index.is_some()
            || self.names_group.is_some()
            || self.niches.is_some()
    }
    pub fn apply_design(&self, state: &mut VariantDesignState) -> std::io::Result<()> {
        if let Some(value) = self.auto_upgraded {
            state.auto_upgraded = value;
        }
        if let Some(value) = self.highlight {
            state.highlight = value;
        }
        if let Some(value) = self.max_version {
            state.max_version = value;
        }
        if let Some(value) = self.role_icon_index {
            state.role_icon_index = value;
        }
        if let Some(value) = &self.names_group {
            state.division_names_group = replay_rows::value::Value::try_clone(value)?;
        }
        if let Some(value) = &self.niches {
            state.niches = replay_rows::value::Value::try_clone(value)?;
        }
        Ok(())
    }
    pub fn apply_definition(&self, row: &mut VariantDefinitionRow) -> std::io::Result<()> {
        if let Some(value) = self.obsolete {
            row.obsolete = value;
        }
        if let Some(value) = &self.display_name {
            row.display_name = replay_rows::value::Value::try_clone(value)?;
        }
        if let Some(value) = &self.model {
            row.explicit_entity = replay_rows::value::Value::try_clone(value)?;
        }
        if let Some(value) = &self.icon {
            row.icon_active = value.active;
            row.icon_kind = value.kind;
            row.icon = replay_rows::value::Value::try_clone(&value.text)?;
        }
        if self.changes_design() {
            self.apply_design(
                row.design
                    .require_mut("VariantDefinitions.design")
                    .map_err(std::io::Error::other)?,
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantChangeRow {
    pub sequence: u64,
    pub hour: i32,
    pub variant: u64,
    pub flags: Option<[u8; 3]>,
    pub scalar: Option<VariantScalarChange>,
    pub sources: Option<VariantSourceReferences>,
    pub organisation: Option<VariantOrganisationChange>,
    pub removed: bool,
    pub snapshot: Option<VariantSnapshot>,
    pub failure: Vec<u8>,
}
typed_codec!(VariantChangeRow, 64, |row| row.sequence as u128,
|key| Self { sequence: key as u64, ..Default::default() },
{ 0 => (hour): I32, 1 => (removed): Bool, 2 => (snapshot): Opaque, 3 => (failure): Bytes, 4 => (variant): U64, 5 => (flags): Opaque, 6 => (sources): Opaque, 7 => (organisation): Opaque, 8 => (scalar): Opaque }, |row| {
   if let Some(change) = &row.scalar {
       if row.sequence == 0 || row.variant == 0 || row.organisation.is_some() || row.snapshot.is_some() || row.flags.is_some() || row.sources.is_some() || row.removed || !row.failure.is_empty() || change == &VariantScalarChange::default() {
           return Err(invalid("invalid variant scalar change"));
       }
   } else if let Some(change) = &row.organisation {
       if row.sequence == 0 || row.variant != 0 || change.organisation == 0 || row.snapshot.is_some() || row.flags.is_some() || row.sources.is_some() || row.removed || !row.failure.is_empty() { return Err(invalid("invalid design organisation liveness change")); }
   } else if let Some(snapshot) = &row.snapshot {
       if row.sequence == 0 || row.variant != snapshot.definition.id || (row.flags.is_some() || row.sources.is_some()) || snapshot.stats.len() != 10 || !row.failure.is_empty() {
           return Err(invalid("invalid variant change snapshot"));
       }
       check_design_state(&snapshot.definition.design)?;
       if snapshot.modules.iter().enumerate().any(|(index, row)| row.ordinal as usize != index)
           || snapshot.upgrades.iter().enumerate().any(|(index, row)| row.ordinal as usize != index) {
           return Err(invalid("variant snapshot child ordinals are not contiguous"));
       }
   } else if row.sources.is_some() {
       if row.sequence == 0 || row.variant == 0 || row.flags.is_some() || row.removed || !row.failure.is_empty() { return Err(invalid("invalid variant source change")); }
   } else if row.flags.is_some() {
       if row.sequence == 0 || row.variant == 0 || row.removed || !row.failure.is_empty() { return Err(invalid("invalid variant flag change")); }
   } else if row.removed {
       if row.sequence == 0 || row.variant == 0 || !row.failure.is_empty() { return Err(invalid("invalid variant removal")); }
   } else if row.sequence != 0 || row.failure.is_empty() {
       return Err(invalid("variant change lacks its snapshot or observation failure"));
   }
   Ok(())
});

impl VariantSnapshot {
    pub fn try_definition(&self) -> std::io::Result<VariantDefinitionRow> {
        replay_rows::value::Value::try_clone(&self.definition).map(Into::into)
    }
}


impl VariantSnapshotDefinition {
    pub fn matches(&self, row: &VariantDefinitionRow) -> bool {
        self.id == row.id
            && self.equipment_token == row.equipment_token
            && self.creator_link == row.creator_link
            && self.origin_link == row.origin_link
            && self.append_variant_name == row.append_variant_name
            && self.obsolete == row.obsolete
            && self.classifier_flags == row.classifier_flags
            && self.mission_mask == row.mission_mask
            && self.display_name == row.display_name
            && self.variant_name == row.variant_name
            && self.explicit_entity == row.explicit_entity
            && self.archetype_token == row.archetype_token
            && self.icon_active == row.icon_active
            && self.icon_kind == row.icon_kind
            && self.icon == row.icon
            && self.version == row.version
            && self.design == row.design
    }
}

#[cfg(test)]
mod design_contract_tests {
    use super::*;

    #[test]
    fn scalar_edits_round_trip_and_preserve_unwritten_fields() {
        let mut row = VariantChangeRow {
            sequence: 1,
            variant: 70,
            scalar: Some(VariantScalarChange {
                auto_upgraded: Some(1),
                names_group: Some(None),
                ..Default::default()
            }),
            ..Default::default()
        };
        row.check_key_fields().unwrap();
        let delta = row.delta(None).unwrap().unwrap();
        assert_eq!(VariantChangeRow::apply_delta(&delta, None).unwrap(), row);
        let mut definition = VariantDefinitionRow {
            design: Recorded::Value(VariantDesignState {
                manpower: 99,
                division_names_group: Some(b"old".to_vec()),
                ..Default::default()
            }),
            ..Default::default()
        };
        row.scalar
            .as_ref()
            .unwrap()
            .apply_definition(&mut definition)
            .unwrap();
        let state = definition.design.require("design").unwrap();
        assert_eq!(state.auto_upgraded, 1);
        assert_eq!(state.manpower, 99);
        assert_eq!(state.division_names_group, None);
        row.flags = Some([0; 3]);
        assert!(row.check_key_fields().is_err());
        row.flags = None;
        row.scalar = Some(Default::default());
        assert!(row.check_key_fields().is_err());
    }
    #[test]
    fn compact_sources_and_shared_liveness_have_separate_valid_row_contracts() {
        let mut source = VariantChangeRow {
            sequence: 1,
            variant: 7,
            sources: Some(VariantSourceReferences {
                direct: vec![8, 8],
                named: vec![9],
            }),
            ..Default::default()
        };
        source.check_key_fields().unwrap();
        let delta = source.delta(None).unwrap().unwrap();
        assert_eq!(VariantChangeRow::apply_delta(&delta, None).unwrap(), source);
        source.organisation = Some(VariantOrganisationChange {
            organisation: 99,
            live: false,
        });
        assert!(source.check_key_fields().is_err());
        let life = VariantChangeRow {
            sequence: 2,
            organisation: Some(VariantOrganisationChange {
                organisation: 99,
                live: false,
            }),
            ..Default::default()
        };
        life.check_key_fields().unwrap();
        let delta = life.delta(None).unwrap().unwrap();
        assert_eq!(VariantChangeRow::apply_delta(&delta, None).unwrap(), life);
        let row = DesignOrganisationLivenessRow {
            organisation: 99,
            live: false,
        };
        row.check_key_fields().unwrap();
    }

    #[test]
    fn variant_removal_is_an_identity_tombstone_without_a_snapshot() {
        let mut row = VariantChangeRow {
            sequence: 1,
            hour: 7,
            variant: u64::MAX,
            removed: true,
            ..Default::default()
        };
        row.check_key_fields().unwrap();
        let delta = row.delta(None).unwrap().unwrap();
        assert_eq!(VariantChangeRow::apply_delta(&delta, None).unwrap(), row);
        row.variant = 0;
        assert!(row.check_key_fields().is_err());
        row.variant = u64::MAX;
        row.removed = false;
        assert!(row.check_key_fields().is_err());
    }

    #[test]
    fn variant_country_modifier_availability_never_confuses_a_failure_with_zero() {
        let mut row = DesignCountryModifierRow {
            country: 5_000,
            modifier: u32::MAX,
            value: Some(0),
            failure: 0,
        };
        row.check_key_fields().unwrap();
        assert_eq!(row.key(), (5_000_u128 << 32) | u128::from(u32::MAX));
        row.value = None;
        assert!(row.check_key_fields().is_err());
        for failure in [1, 2] {
            row.failure = failure;
            row.check_key_fields().unwrap();
            row.value = Some(0);
            assert!(row.check_key_fields().is_err());
            row.value = None;
        }
        row.failure = 3;
        assert!(row.check_key_fields().is_err());
    }

    #[test]
    fn design_template_consumption_changes_only_uses_and_checks_parallel_rosters() {
        let previous = DesignCountryTemplatesRow {
            country: 5_000,
            uses: vec![2, -1],
            factors: vec![123_457, -1],
            equipment: vec![vec![8, 8], vec![]],
            failure: 0,
        };
        previous.check_key_fields().unwrap();
        let mut next = previous.clone();
        next.uses[0] -= 1;
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1);
        assert_eq!(
            DesignCountryTemplatesRow::apply_delta(&delta, Some(previous)).unwrap(),
            next
        );
        next.factors.pop();
        assert!(next.check_key_fields().is_err());
        let mut empty = DesignCountryTemplatesRow::default();
        empty.check_key_fields().unwrap();
        empty.failure = 2;
        empty.check_key_fields().unwrap();
        empty.uses.push(0);
        assert!(empty.check_key_fields().is_err());
    }

    #[test]
    fn variant_design_contract_refuses_bad_stat_residuals_and_null_live_team() {
        let mut row = VariantDefinitionRow {
            id: 70,
            design: Recorded::Value(VariantDesignState::default()),
            ..Default::default()
        };
        row.check_key_fields().unwrap();
        row.design
            .require_mut("design")
            .unwrap()
            .design_team_serialized = true;
        assert!(row.check_key_fields().is_err());
        let state = row.design.require_mut("design").unwrap();
        state.design_team_serialized = false;
        state.design_team = u64::MAX;
        state.named_bonuses.push(9);
        assert!(row.check_key_fields().is_err());
        let state = row.design.require_mut("design").unwrap();
        state.unresolved_stats = (0..engine_reference_values::native_values::equipment_variant::STAT_COUNT as u32)
            .filter(|index| {
                ![0x2e, 0x2f, 0x30, 0x31, 0x3e, 0x41, 0x43, 0x44, 0x45, 0x4d].contains(index)
            })
            .map(|index| VariantStatEntry {
                index,
                value: i64::MIN,
            })
            .collect();
        row.check_key_fields().unwrap();
        row.design.require_mut("design").unwrap().unresolved_stats[0].index = 1;
        assert!(row.check_key_fields().is_err());
    }
}
