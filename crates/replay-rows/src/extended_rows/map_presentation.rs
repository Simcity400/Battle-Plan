use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryAppearanceRow {
    pub country: u32,
    pub subjects: Recorded<Vec<i32>>,
    pub red: u32,
    pub green: u32,
    pub blue: u32,
    pub alpha: u32,
    pub cosmetic_tag: Vec<u8>,
    pub name_tag: Vec<u8>,
    pub leader_ideology: Vec<u8>,
    pub overlord: i32,
    pub graphical_culture: Vec<u8>,
    pub graphical_culture_2d: Vec<u8>,
    pub leader_character: Option<u32>,
    pub old_name: Option<Vec<u8>>,
    pub old_adjective: Option<Vec<u8>>,
    pub old_definite_name: Option<Vec<u8>>,
    pub ui_red: u32,
    pub ui_green: u32,
    pub ui_blue: u32,
    pub ui_alpha: u32,
    pub original_tag: Recorded<Option<Vec<u8>>>,
    pub original_tag_reference: Recorded<Option<i32>>,
    pub transport_convoys: Recorded<Retired>,
}
impl CountryAppearanceRow {
    pub fn map_color(&self) -> [u8; 3] {
        [self.red, self.green, self.blue].map(|component| (f32::from_bits(component) * 255.0) as u8)
    }
}

typed_codec!(CountryAppearanceRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (red): U32, 1 => (green): U32, 2 => (blue): U32, 3 => (alpha): U32, 4 => (cosmetic_tag): Bytes, 5 => (name_tag): Bytes, 6 => (leader_ideology): Bytes, 7 => (overlord): I32, 8 => (graphical_culture): Bytes, 9 => (leader_character): OptionalU32, 10 => (old_name): Opaque, 11 => (old_adjective): Opaque, 12 => (old_definite_name): Opaque, 13 => (ui_red): U32, 14 => (ui_green): U32, 15 => (ui_blue): U32, 16 => (ui_alpha): U32, 17 => (graphical_culture_2d): Bytes, 18 => (subjects): Opaque, 19 => (original_tag): Opaque, 20 => (original_tag_reference): OptionalI32, 21 => (transport_convoys): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyIconInputRow {
    pub division: u64,
    pub separate_counter: u8,
    pub alternate_template: u64,
}
typed_codec!(ArmyIconInputRow, 64, |row| row.division as u128, |key| Self { division: key as u64, ..Default::default() },
 { 0 => (separate_counter): U8, 1 => (alternate_template): U64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ShipCounterInputs {
    pub roster_ordinal: u32,
    pub definition_valid: u8,
    pub carrier_air_base: u64,
    pub maximum_strength: i64,
    pub anti_air_base: i64,
    pub maximum_organisation: i64,
    pub positive_equipment_flags: u64,
}
value_struct!(ShipCounterInputs {
    roster_ordinal: u32,
    definition_valid: u8,
    carrier_air_base: u64,
    maximum_strength: i64,
    anti_air_base: i64,
    maximum_organisation: i64,
    positive_equipment_flags: u64
});

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct HullVariantInputs {
    pub definition_index: Option<i32>,
    pub module_count: Option<i32>,
    pub upgrade_level_sum: Option<i32>,
    pub module_priority_sum: Option<i32>,
    pub version: Option<u8>,
    pub organisation_marker: Option<i32>,
}
value_struct!(HullVariantInputs {
    definition_index: Option<i32>, module_count: Option<i32>, upgrade_level_sum: Option<i32>,
    module_priority_sum: Option<i32>, version: Option<u8>, organisation_marker: Option<i32>
});
impl OpaqueValue for HullVariantInputs {}

opaque_value!(Option<[i64; 5]>);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ShipModelRow {
    pub ship: u64,
    pub variant: u64,
    pub hull_sprite: Vec<u8>,
    pub fallback_sprite: Vec<u8>,
    pub model_number: i32,
    pub formation_flags: u64,
    pub counter: Option<ShipCounterInputs>,
    pub subunit_index: Recorded<Option<i32>>,
    pub strike: Recorded<Retired>,
    pub hull_variant: Recorded<Option<HullVariantInputs>>,
    pub has_criticals: Recorded<Option<u32>>,
    pub air_capacity_kind: Recorded<Option<u32>>,
    pub naval_power: Recorded<Option<[i64; 5]>>,
}
typed_codec!(ShipModelRow, 64, |row| row.ship as u128, |key| Self { ship: key as u64, ..Default::default() },
 { 0 => (variant): U64, 1 => (hull_sprite): Bytes, 2 => (fallback_sprite): Bytes, 3 => (model_number): I32, 4 => (formation_flags): U64, 5 => (counter): Opaque, 6 => (subunit_index): OptionalI32, 7 => (strike): Opaque, 8 => (hull_variant): Opaque, 9 => (has_criticals): OptionalU32, 10 => (air_capacity_kind): OptionalU32, 11 => (naval_power): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProvinceUnitOrderRow {
    pub province: u32,
    pub mixed: Vec<u64>,
    pub naval: Vec<u64>,
}
typed_codec!(ProvinceUnitOrderRow,32,|r| r.province as u128,|k| Self { province:k as u32,..Default::default() },
 {0 => (mixed): U64List,1 => (naval): U64List},|_r| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CharacterPortrait {
    pub category: u32,
    pub size: u32,
    pub is_set: u8,
    pub kind: u32,
    pub identity: Vec<u8>,
}
value_struct!(CharacterPortrait {category:u32,size:u32,is_set:u8,kind:u32,identity:Vec<u8>});
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CharacterPortraitRow {
    pub character: u32,
    pub portraits: Vec<CharacterPortrait>,
}

pub const PORTRAIT_CIVILIAN: u32 = 0;
pub const PORTRAIT_LARGE: u32 = 1;

pub fn portrait_entry(
    portraits: &[CharacterPortrait],
    category: u32,
    size: u32,
) -> Option<&CharacterPortrait> {
    let key = (size as i32, category as i32);
    let n = portraits.len();
    let mut p = 0usize;
    let mut u = n;
    while u > 0 {
        let half = u >> 1;
        let entry = &portraits[p + half];
        if (entry.size as i32, entry.category as i32) < key {
            p += (u & 1) + half;
        }
        u = half;
    }
    let exact = portraits
        .get(p)
        .is_some_and(|e| e.size as i32 == key.0 && e.category as i32 == key.1);
    if !exact {
        while p > 0 && portraits[p - 1].size as i32 == key.0 {
            p -= 1;
        }
        if portraits.get(p).is_none_or(|e| e.size as i32 != key.0) {
            return None;
        }
    }
    portraits.get(p)
}
typed_codec!(CharacterPortraitRow,32,|r| r.character as u128,|k| Self {character:k as u32,..Default::default()},
 {0 => (portraits): Opaque},|_r| Ok(()));
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LeaderPortraitRow {
    pub leader: u64,
    pub character: u64,
    pub portrait_c0: Vec<u8>,
    pub portrait_e0: Vec<u8>,
    pub portrait_100: Vec<u8>,
}
typed_codec!(LeaderPortraitRow,64,|r| r.leader as u128,|k| Self {leader:k as u64,..Default::default()},
 {0 => (character): U64,1 => (portrait_c0): Bytes,2 => (portrait_e0): Bytes,3 => (portrait_100): Bytes},|_r| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalSearcher {
    pub reference: Option<u64>,
    pub resolves: Option<bool>,
    pub roster_owner: Option<u32>,
}
value_struct!(NavalSearcher { reference: Option<u64>, resolves: Option<bool>, roster_owner: Option<u32> });

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalEngagementTarget {
    pub reference: u64,
    pub resolves: Option<bool>,
}
value_struct!(NavalEngagementTarget { reference: u64, resolves: Option<bool> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalPortMissionInputs {
    pub sample_hour: PacketHour,
    pub searching: Option<Vec<NavalSearcher>>,
    pub engagement_target: Option<NavalEngagementTarget>,
    pub avoided_ship_types: Option<Vec<Option<Vec<u8>>>>,
}
value_struct!(NavalPortMissionInputs {
    sample_hour: PacketHour, searching: Option<Vec<NavalSearcher>>,
    engagement_target: Option<NavalEngagementTarget>, avoided_ship_types: Option<Vec<Option<Vec<u8>>>>
});
impl OpaqueValue for NavalPortMissionInputs {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TaskForceCounterInputRow {
    pub task_force: u64,
    pub is_in_regions: u8,
    pub running_away: u8,
    pub engagement_rule: i32,
    pub mission_region_count: i32,
    pub bombardment_region: u32,
    pub repair_parent: u64,
    pub detached_activity: i32,
    pub repair_last_mission: i32,
    pub received_supply_ratio: i64,
    pub attached_headquarters: i32,
    pub name: Recorded<Option<Vec<u8>>>,
    pub supply: Recorded<Option<UnitSupplyInputs>>,
    pub port_mission: Recorded<Option<NavalPortMissionInputs>>,
}
typed_codec!(TaskForceCounterInputRow, 64, |r| r.task_force as u128, |k| Self { task_force: k as u64, ..Default::default() },
 { 0 => (is_in_regions): U8, 1 => (running_away): U8, 2 => (engagement_rule): I32, 3 => (mission_region_count): I32, 4 => (bombardment_region): U32, 5 => (repair_parent): U64, 6 => (detached_activity): I32, 7 => (repair_last_mission): I32, 8 => (received_supply_ratio): I64, 9 => (attached_headquarters): I32, 10 => (name): Opaque, 11 => (supply): Opaque, 12 => (port_mission): Opaque }, |_r| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RailwayGunCounterInputRow {
    pub gun: u64,
    pub roster_owner: u32,
    pub expeditionary_owner: u32,
    pub logical_owner: u32,
    pub province: u32,
    pub naval_transfer: u8,
    pub transfer_action_status: u8,
    pub exiled: u8,
    pub retreat: u8,
    pub strategic_redeployment: u8,
    pub reference_entry_count: i32,
    pub assignee_group: u64,
    pub subunit_token: u32,
    pub current_strength: i64,
    pub maximum_strength: i64,
    pub received_supply_ratio: i64,
    pub railway_building_level: Option<i16>,
    pub target: u32,
    pub combat_role: u8,
    pub combat_province: u32,
    pub combat_count: u32,
    pub attacking: u8,
    pub ordinary_path: Vec<u32>,
    pub alternate_path: Vec<u32>,
    pub name: Recorded<Option<Vec<u8>>>,
    pub eta: Recorded<Option<RailwayGunEtaOperands>>,
    pub supply: Recorded<Option<UnitSupplyInputs>>,
    pub withdrawal: Recorded<Option<u8>>,
    pub fresh: Recorded<Option<FreshRailwayCounterEta>>,
    pub combat_references: Recorded<Option<UnitCombatReferences>>,
    pub transport: Recorded<Retired>,
    pub theatre: Recorded<Option<u64>>,
    pub queued_actions: Recorded<Option<i32>>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RailwayGunEtaOperands {
    pub numerator: i64,
    pub current_speed: i64,
}
value_struct!(RailwayGunEtaOperands {
    numerator: i64,
    current_speed: i64
});
impl OpaqueValue for RailwayGunEtaOperands {}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FreshRailwayCounterEta {
    pub sample_hour: PacketHour,
    pub maximum_speed: Option<i64>,
    pub land_supply_factor: Option<i64>,
    pub orders_group: Option<u64>,
}
value_struct!(FreshRailwayCounterEta {
    sample_hour: PacketHour,
    maximum_speed: Option<i64>,
    land_supply_factor: Option<i64>,
    orders_group: Option<u64>,
});
impl OpaqueValue for FreshRailwayCounterEta {}
typed_codec!(RailwayGunCounterInputRow, 64, |r| r.gun as u128, |k| Self { gun: k as u64, ..Default::default() },
 { 0 => (roster_owner): U32, 1 => (expeditionary_owner): U32, 2 => (logical_owner): U32, 3 => (province): U32, 4 => (naval_transfer): U8, 5 => (transfer_action_status): U8, 6 => (exiled): U8, 7 => (retreat): U8, 8 => (strategic_redeployment): U8, 9 => (reference_entry_count): I32, 10 => (assignee_group): U64, 11 => (subunit_token): U32, 12 => (current_strength): I64, 13 => (maximum_strength): I64, 14 => (received_supply_ratio): I64, 15 => (railway_building_level): Opaque, 16 => (target): U32, 17 => (combat_role): U8, 18 => (combat_province): U32, 19 => (combat_count): U32, 20 => (attacking): U8, 21 => (ordinary_path): U32List, 22 => (alternate_path): U32List, 23 => (name): Opaque, 24 => (eta): Opaque, 25 => (supply): Opaque, 26 => (withdrawal): Opaque, 27 => (fresh): Opaque, 28 => (combat_references): Opaque, 29 => (transport): Opaque, 30 => (theatre): OptionalU64, 31 => (queued_actions): OptionalI32 }, |row| check_unit_combat_references(&row.combat_references, row.combat_count));


#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FleetCounterInputRow {
    pub fleet: u64,
    pub red: u32,
    pub green: u32,
    pub blue: u32,
    pub alpha: u32,
    pub mission_region_count: i32,
    pub mission_regions: Recorded<Option<std::sync::Arc<Vec<u32>>>>,
    pub task_forces: Vec<u64>,
}
typed_codec!(FleetCounterInputRow, 64, |r| r.fleet as u128, |k| Self { fleet: k as u64, ..Default::default() },
 { 0 => (red): U32, 1 => (green): U32, 2 => (blue): U32, 3 => (alpha): U32, 4 => (mission_region_count): I32, 5 => (task_forces): U64List, 6 => (mission_regions): Opaque }, |_r| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ZeroSubjectInputs {
    pub relation_type: Option<u32>,
    pub parties: Option<[Option<u32>; 2]>,
}
value_struct!(ZeroSubjectInputs { relation_type: Option<u32>, parties: Option<[Option<u32>; 2]> });
impl OpaqueValue for ZeroSubjectInputs {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ManpowerReferences(Option<std::sync::Arc<Vec<u64>>>);

impl ManpowerReferences {
    pub fn allocated_bytes(&self) -> usize {
        <Self as Value>::heap_bytes(self)
    }
}
impl From<Vec<u64>> for ManpowerReferences {
    fn from(values: Vec<u64>) -> Self {
        Self((!values.is_empty()).then(|| std::sync::Arc::new(values)))
    }
}
impl std::ops::Deref for ManpowerReferences {
    type Target = [u64];
    fn deref(&self) -> &[u64] {
        self.0.as_deref().map_or(&[], Vec::as_slice)
    }
}
impl<'a> IntoIterator for &'a ManpowerReferences {
    type Item = &'a u64;
    type IntoIter = std::slice::Iter<'a, u64>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl Value for ManpowerReferences {
    const MINIMUM_ENCODED_BYTES: usize = 4;
    fn heap_bytes(&self) -> usize {
        self.0.as_ref().map_or(0, |values| {
            size_of::<Vec<u64>>() + 2 * size_of::<usize>() + values.capacity() * size_of::<u64>()
        })
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        engine_allocation::add(
            4,
            self.len()
                .checked_mul(8)
                .ok_or_else(|| invalid("reference list size overflow"))?,
        )
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(self.clone())
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        let count =
            u32::try_from(self.len()).map_err(|_| invalid("reference count exceeds u32"))?;
        output.extend_from_slice(&count.to_le_bytes());
        <u64 as Value>::encode_many(self, output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        <Vec<u64> as Value>::decode(input).map(Self::from)
    }
}
crate::checkpoint::values::wire_value!(ManpowerReferences);
impl OpaqueValue for ManpowerReferences {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryCounterInputRow {
    pub sent_expeditionaries: Recorded<Option<ManpowerReferences>>,
    pub country: u32,
    pub recruitment_modifiers: Recorded<Option<[Option<i64>; 4]>>,
    pub pride_of_the_fleet: u64,
    pub owned_state_count: i32,
    pub controlled_state_count: i32,
    pub first_division: u64,
    pub owned_states: Recorded<Option<Vec<u32>>>,
    pub controlled_states: Recorded<Option<Vec<u32>>>,
    pub core_states: Recorded<Option<Vec<u32>>>,
    pub army_leaders: Recorded<Option<Vec<u64>>>,
    pub navy_leaders: Recorded<Option<Vec<u64>>>,
    pub mio_policies: Recorded<Option<Vec<u32>>>,
    pub neighbors: Recorded<Option<Vec<u32>>>,
    pub co_belligerents: Recorded<Option<Vec<u32>>>,
    pub mines_effect_reduction: Recorded<Option<Option<i64>>>,
    pub naval_detection: Recorded<Option<Option<i64>>>,
    pub zero_war: Recorded<Option<[u8; 2]>>,
    pub movement: Recorded<Retired>,
    pub zero_subject: Recorded<Option<ZeroSubjectInputs>>,
    pub pride_name: Recorded<Option<Vec<u8>>>,
    pub forward_visibility: Recorded<Option<ReverseZeroVisibilityInputs>>,
    pub attack: Recorded<Witnessed<CountryAttackInputs>>,
    pub directed_wars: Recorded<Option<DirectedWars>>,
    pub attack_modifiers: Recorded<Option<CountryAttackModifiers>>,
    pub movement_modifiers: Recorded<Option<CountryMovementModifiers>>,
    pub air_modifiers: Recorded<Option<[Option<i64>; 10]>>,
    pub air_combat_modifiers_known: Recorded<bool>,
    pub air_superiority_efficiency: Recorded<Option<i64>>,
    pub air_intercept_efficiency: Recorded<Option<i64>>,
    pub air_cas_efficiency: Recorded<Option<i64>>,
    pub air_nav_efficiency: Recorded<Option<i64>>,
    pub air_attack_factor: Recorded<Option<i64>>,
    pub air_defence_factor: Recorded<Option<i64>>,
    pub air_strategic_bomber_defence_factor: Recorded<Option<i64>>,
    pub combat_width_factor: Recorded<Option<i64>>,
}
typed_codec!(CountryCounterInputRow, 32, |r| r.country as u128, |k| Self { country: k as u32, ..Default::default() },
 { 0 => (pride_of_the_fleet): U64, 1 => (owned_state_count): I32, 2 => (controlled_state_count): I32, 3 => (first_division): U64, 4 => (owned_states): Opaque, 5 => (controlled_states): Opaque, 6 => (core_states): Opaque, 7 => (army_leaders): Opaque, 8 => (navy_leaders): Opaque, 9 => (mio_policies): Opaque, 10 => (neighbors): Opaque, 11 => (co_belligerents): Opaque, 12 => (zero_war): Opaque, 13 => (movement): Opaque, 14 => (zero_subject): Opaque, 15 => (pride_name): Opaque, 16 => (forward_visibility): Opaque, 17 => (attack): Opaque, 18 => (directed_wars): Opaque, 19 => (attack_modifiers): Opaque, 20 => (movement_modifiers): Opaque, 21 => (recruitment_modifiers): Opaque, 22 => (sent_expeditionaries): Opaque, 23 => (air_modifiers): Opaque, 24 => (mines_effect_reduction): Opaque, 25 => (naval_detection): Opaque, 26 => (air_superiority_efficiency): OptionalI64, 27 => (air_intercept_efficiency): OptionalI64, 28 => (air_cas_efficiency): OptionalI64, 29 => (air_nav_efficiency): OptionalI64, 30 => (air_attack_factor): OptionalI64, 31 => (air_defence_factor): OptionalI64, 32 => (air_strategic_bomber_defence_factor): OptionalI64, 33 => (air_combat_modifiers_known): Bool, 34 => (combat_width_factor): OptionalI64 }, |_row| {
    Ok(())
});
opaque_value!(Option<[Option<i64>; 10]>);

impl CountryCounterInputRow {
    pub fn air_combat_modifier(&self, key: u32) -> Result<Option<i64>, &'static str> {
        use engine_reference_values::modifier_keys as keys;
        if matches!(
            key,
            keys::AIR_HOME_DEFENCE_FACTOR | keys::AIR_POWER_PROJECTION_FACTOR
        ) {
            let Recorded::Value(Some(values)) = &self.air_modifiers else {
                return Err("country Air modifier census is unavailable");
            };
            return Ok(values[if key == keys::AIR_HOME_DEFENCE_FACTOR {
                2
            } else {
                3
            }]);
        }
        if self.air_combat_modifiers_known != Recorded::Value(true) {
            return Err("country Air combat modifier census is unavailable");
        }
        let value = match key {
            keys::AIR_SUPERIORITY_EFFICIENCY => self.air_superiority_efficiency,
            keys::AIR_INTERCEPT_EFFICIENCY => self.air_intercept_efficiency,
            keys::AIR_CAS_EFFICIENCY => self.air_cas_efficiency,
            keys::AIR_NAV_EFFICIENCY => self.air_nav_efficiency,
            keys::AIR_ATTACK_FACTOR => self.air_attack_factor,
            keys::AIR_DEFENCE_FACTOR => self.air_defence_factor,
            keys::AIR_STRATEGIC_BOMBER_DEFENCE_FACTOR => self.air_strategic_bomber_defence_factor,
            _ => return Err("country Air combat modifier key is not recorded"),
        };
        value
            .require("country Air combat modifier")
            .copied()
            .map_err(|_| "country Air combat modifier not recorded")
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LaunchNavalBase {
    pub base_province: u32,
    pub country: u32,
}
value_struct!(LaunchNavalBase {
    base_province: u32,
    country: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum LaunchNavalBases {
    #[default]
    Unavailable,
    Bases(Vec<LaunchNavalBase>),
    NativeNull,
}

impl Value for LaunchNavalBases {
    fn heap_bytes(&self) -> usize {
        match self {
            Self::Bases(bases) => Value::heap_bytes(bases),
            Self::Unavailable | Self::NativeNull => 0,
        }
    }
    const MINIMUM_ENCODED_BYTES: usize = 1;

    fn encoded_bytes(&self) -> io::Result<usize> {
        match self {
            Self::Bases(bases) => allocation::add(1, Value::encoded_bytes(bases)?),
            _ => Ok(1),
        }
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(match self {
            Self::Unavailable => Self::Unavailable,
            Self::Bases(bases) => Self::Bases(Value::try_clone(bases)?),
            Self::NativeNull => Self::NativeNull,
        })
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        match self {
            Self::Unavailable => output.push(0),
            Self::Bases(bases) => {
                output.push(1);
                bases.encode_reserved(output)?;
            }
            Self::NativeNull => output.push(2),
        }
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        match input.u8()? {
            0 => Ok(Self::Unavailable),
            1 => Vec::<LaunchNavalBase>::decode(input).map(Self::Bases),
            2 => Ok(Self::NativeNull),
            _ => Err(invalid("launch naval provider discriminator invalid")),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ReservedLaunchSources {
    pub sample_hour: PacketHour,
    pub naval_bases: LaunchNavalBases,
    pub owned_states: Option<Vec<u32>>,
}
value_struct!(ReservedLaunchSources {
    sample_hour: PacketHour,
    naval_bases: LaunchNavalBases,
    owned_states: Option<Vec<u32>>,
});
impl OpaqueValue for ReservedLaunchSources {}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ReverseZeroVisibilityInputs {
    pub target: Option<u32>,
    pub war: Option<[u8; 2]>,
    pub subject: Option<ZeroSubjectInputs>,
    pub military_access: Option<u8>,
}
value_struct!(ReverseZeroVisibilityInputs {
    target: Option<u32>,
    war: Option<[u8; 2]>,
    subject: Option<ZeroSubjectInputs>,
    military_access: Option<u8>,
});
impl OpaqueValue for ReverseZeroVisibilityInputs {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ReservedReverseVisibilityInputs {
    pub sample_hour: PacketHour,
    pub cells: Vec<Option<ReverseZeroVisibilityInputs>>,
}
value_struct!(ReservedReverseVisibilityInputs {
    sample_hour: PacketHour,
    cells: Vec<Option<ReverseZeroVisibilityInputs>>,
});
impl OpaqueValue for ReservedReverseVisibilityInputs {}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ReservedCountryIntelSample {
    pub sample_hour: i32,
    pub vector_count: u32,
}
impl ReservedCountryIntelSample {
    fn check_native_count(&self) -> io::Result<()> {
        if self.vector_count > i32::MAX as u32 {
            Err(invalid(
                "reserved country intel count exceeds signed native domain",
            ))
        } else {
            Ok(())
        }
    }
}
impl crate::value::Value for ReservedCountryIntelSample {
    fn heap_bytes(&self) -> usize {
        0
    }
    const MINIMUM_ENCODED_BYTES: usize = 8;
    #[cfg(any(test, feature = "test-support"))]
    const STRUCT_FIELDS: usize = 2;

    fn encoded_bytes(&self) -> io::Result<usize> {
        self.check_native_count()?;
        Ok(Self::MINIMUM_ENCODED_BYTES)
    }
    fn try_clone(&self) -> io::Result<Self> {
        self.check_native_count()?;
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        self.check_native_count()?;
        <i32 as crate::value::Value>::encode_reserved(&self.sample_hour, output)?;
        <u32 as crate::value::Value>::encode_reserved(&self.vector_count, output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let sample = Self {
            sample_hour: <i32 as crate::value::Value>::decode(input)?,
            vector_count: <u32 as crate::value::Value>::decode(input)?,
        };
        sample.check_native_count()?;
        Ok(sample)
    }
}
impl OpaqueValue for ReservedCountryIntelSample {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ReservedCountryNamingRow {
    pub tag: Vec<u8>,
    pub name_tag: Vec<u8>,
    pub cosmetic_tag: Vec<u8>,
    pub leader_ideology: Vec<u8>,
    pub group: Option<Vec<u8>>,
    pub overlord: i32,
    pub autonomy: Option<Vec<u8>>,
    pub built: u8,
    pub original_tag: Recorded<Option<Vec<u8>>>,
    pub original_tag_reference: Recorded<Option<i32>>,
    pub original_tag_buckets: Recorded<Option<Vec<Vec<u32>>>>,
    pub country_array: Recorded<Option<Vec<u32>>>,
    pub owned_state_count: Recorded<Option<i32>>,
    pub dynamic_tag_boundary: Recorded<Option<i32>>,
    pub faction: Recorded<Option<u64>>,
    pub movement: Recorded<Retired>,
    pub land_theatre_count: Recorded<Option<u32>>,
    pub launch_sources: Recorded<Option<ReservedLaunchSources>>,
    pub reverse_visibility: Recorded<Option<ReservedReverseVisibilityInputs>>,
    pub country_intel_sample: Recorded<Option<ReservedCountryIntelSample>>,
    pub attack: Recorded<Witnessed<Option<CountryAttackInputs>>>,
    pub raw_country_observations: Recorded<Retired>,
    pub transport_mission_inputs: Recorded<Retired>,
}
typed_codec!(ReservedCountryNamingRow, 0, |_row| 0, |_key| Self::default(),
{ 0 => (tag): Bytes, 1 => (name_tag): Bytes, 2 => (cosmetic_tag): Bytes, 3 => (leader_ideology): Bytes, 4 => (group): Opaque, 5 => (overlord): I32, 6 => (autonomy): Opaque, 7 => (built): U8, 8 => (original_tag): Opaque, 9 => (original_tag_reference): OptionalI32, 10 => (original_tag_buckets): Opaque, 11 => (country_array): Opaque, 12 => (owned_state_count): OptionalI32, 13 => (dynamic_tag_boundary): OptionalI32, 14 => (faction): OptionalU64, 15 => (movement): Opaque, 16 => (land_theatre_count): OptionalU32, 17 => (launch_sources): Opaque, 18 => (reverse_visibility): Opaque, 19 => (country_intel_sample): Opaque, 20 => (attack): Opaque, 21 => (raw_country_observations): Opaque, 22 => (transport_mission_inputs): Opaque }, |row| {
   if row.built & !7 != 0 {
       Err(invalid("reserved country naming witness has unknown bits"))
   } else {
       Ok(())
   }
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OrdersGroupCounterInputRow {
    pub group: u64,
    pub withdrawing: u8,
    pub hq_distance: Recorded<Option<i32>>,
    pub leader_enroute_days: Recorded<Option<i32>>,
    pub border_war_presence: Recorded<Option<u8>>,
    pub invasion_source_distance: Recorded<Option<i32>>,
    pub hq_nearest_front: Recorded<Option<i32>>,
    pub template_context: Recorded<Option<std::sync::Arc<TemplateHqContext>>>,
    pub attack: Recorded<Witnessed<OrdersGroupAttackInputs>>,
    pub commander_attack: Recorded<Option<Option<CommanderAttackStats>>>,
    pub commander_movement: Recorded<Option<Option<CommanderMovementStats>>>,
}
typed_codec!(OrdersGroupCounterInputRow, 64, |r| r.group as u128, |k| Self { group: k as u64, ..Default::default() },
 { 0 => (withdrawing): U8, 1 => (hq_distance): OptionalI32,
   2 => (leader_enroute_days): OptionalI32, 3 => (border_war_presence): Opaque,
   4 => (invasion_source_distance): OptionalI32, 5 => (attack): Opaque,
     6 => (commander_attack): Opaque, 7 => (commander_movement): Opaque,
     8 => (hq_nearest_front): OptionalI32, 9 => (template_context): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LeaderPortraitPathRow {
    pub leader: u64,
    pub country: u32,
    pub tag_reference: u32,
    pub tag: Vec<u8>,
}
typed_codec!(LeaderPortraitPathRow, 64, |r| r.leader as u128, |k| Self { leader: k as u64, ..Default::default() },
 { 0 => (country): U32, 1 => (tag_reference): U32, 2 => (tag): Bytes }, |_r| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LocalPlayerRow {
    pub country: u32,
    pub observer_country: u32,
}

impl LocalPlayerRow {
    pub fn viewing_country(&self) -> Option<u32> {
        let raw = if (self.country as i32) > 0 {
            self.country
        } else {
            self.observer_country
        };
        ((raw as i32) > 0).then_some(raw)
    }
}
typed_codec!(LocalPlayerRow, 0, |_row| 0, |_key| Self::default(),
 { 0 => (country): U32, 1 => (observer_country): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandUnitOrderRow {
    pub units: Recorded<Option<Vec<u64>>>,
}
typed_codec!(LandUnitOrderRow, 0, |_row| 0, |_key| Self::default(),
 { 0 => (units): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct SimulationFeatureRow {
    pub dlc_mask: u32,
    pub human_ai: u8,
    pub state_array_len: Recorded<Option<u32>>,
    pub weather_suspend: Recorded<Option<[u8; 2]>>,
    pub instant_decisions: Recorded<Option<u8>>,
    pub runtime_scalars: Recorded<PoliticalRuntimeScalars>,
    pub movement_defines: Recorded<MovementDefines>,
}
typed_codec!(SimulationFeatureRow, 0, |_row| 0, |_key| Self::default(),
 { 0 => (dlc_mask): U32, 1 => (human_ai): U8, 2 => (weather_suspend): Opaque, 3 => (instant_decisions): Opaque, 4 => (runtime_scalars): Opaque, 5 => (movement_defines): Opaque, 6 => (state_array_len): OptionalU32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct MovementDefines {
    pub out_of_supply_speed: Option<i64>,
    pub retreat_speed: Option<i64>,
    pub withdrawal_speed: Option<i64>,
    pub out_of_fuel_speed: Option<i64>,
    pub fuel_capacity_scale: Option<i64>,
    pub supply_days: Option<i64>,
    pub supply_grace: Option<i64>,
    pub air_speed: Option<i64>,
    pub air_defense_denominator: Option<i64>,
    pub air_defense_reduction: Option<i64>,
    pub out_of_supply_attrition: Option<i64>,
}
value_struct!(MovementDefines { out_of_supply_speed: Option<i64>, retreat_speed: Option<i64>,
    withdrawal_speed: Option<i64>, out_of_fuel_speed: Option<i64>,
    fuel_capacity_scale: Option<i64>, supply_days: Option<i64>, supply_grace: Option<i64>,
    air_speed: Option<i64>, air_defense_denominator: Option<i64>,
    air_defense_reduction: Option<i64>, out_of_supply_attrition: Option<i64> });
opaque_value!(MovementDefines);
opaque_value!(Option<[u8; 2]>);





#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryVisibilityRow {
    pub country: u32,
    pub unit_modifier: i64,
}
typed_codec!(CountryVisibilityRow, 32, |row| row.country as u128,
 |key| Self { country: key as u32, ..Default::default() },
 { 0 => (unit_modifier): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateVisibilityRow {
    pub state: u32,
    pub country: u32,
    pub network_modifier: i64,
}
typed_codec!(StateVisibilityRow, 32, |row| row.state as u128,
 |key| Self { state: key as u32, ..Default::default() },
 { 0 => (country): U32, 1 => (network_modifier): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionTemplateIconRow {
    pub manpower_exile: Recorded<bool>,
    pub template: u64,
    pub base_width: Recorded<Option<i64>>,
    pub priority_unit: u32,
    pub priority_unit_name: Vec<u8>,
    pub custom_icon: i32,
    pub custom_icon_nato: u8,
    pub representative: u32,
    pub representative_local: u8,
    pub subunits: Vec<TemplateSubunit>,
    pub country: i32,
    pub foreign_country: i32,
    pub model_category: Option<u64>,
    pub model_entity: Vec<u8>,
    pub excluded_variants: Vec<u64>,
    pub model_unreadable: u8,
    pub formation_division_count: Recorded<Option<i32>>,
    pub display_name: Recorded<Option<TemplateDisplayName>>,
    pub grids: Recorded<TemplateGrids>,
    pub layout: Recorded<Option<std::sync::Arc<TemplateLayout>>>,
    pub settings: Recorded<Option<std::sync::Arc<TemplateSettings>>>,
    pub speed_keys: Recorded<Option<Vec<u32>>>,
}
typed_codec!(DivisionTemplateIconRow, 64, |row| row.template as u128,
 |key| Self { template: key as u64, ..Default::default() },
 { 0 => (priority_unit): U32, 1 => (priority_unit_name): Bytes, 2 => (custom_icon): I32, 3 => (custom_icon_nato): U8, 4 => (representative): U32, 5 => (representative_local): U8, 6 => (subunits): Opaque, 7 => (country): I32, 8 => (foreign_country): I32, 9 => (model_category): OptionalU64, 10 => (model_entity): Bytes, 11 => (excluded_variants): U64List, 12 => (model_unreadable): U8, 13 => (formation_division_count): OptionalI32, 14 => (display_name): Opaque, 15 => (grids): Opaque, 16 => (manpower_exile): Bool, 17 => (base_width): OptionalI64, 18 => (layout): Opaque, 19 => (settings): Opaque, 20 => (speed_keys): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProvinceNameRow {
    pub province: u32,
    pub name: Vec<u8>,
}
typed_codec!(ProvinceNameRow, 32, |row| row.province as u128,
 |key| Self { province: key as u32, name: Default::default() },
 { 0 => (name): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalBaseRow {
    pub province: u32,
    pub country: u32,
    pub level: i32,
    pub healthy_level: i32,
    pub countries: Vec<u32>,
    pub countries_unreadable: bool,
    pub country_tags: Recorded<Option<Vec<Vec<u8>>>>,
}
typed_codec!(NavalBaseRow, 32, |row| row.province as u128, |key| Self { province: key as u32, ..Default::default() },
 { 0 => (country): U32, 1 => (level): I32, 2 => (healthy_level): I32, 3 => (countries): U32List, 4 => (countries_unreadable): Bool, 5 => (country_tags): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct BattleSoundRow {
    pub ordinal: u32,
    pub center_x: u32,
    pub center_z: u32,
    pub radius: u32,
    pub provinces: Vec<u32>,
    pub sounding: bool,
    pub object: u64,
}
typed_codec!(BattleSoundRow, 32, |row| row.ordinal as u128, |key| Self { ordinal: key as u32, ..Default::default() },
 { 0 => (center_x): U32, 1 => (center_z): U32, 2 => (radius): U32, 3 => (provinces): U32List, 4 => (sounding): Bool, 5 => (object): U64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FloatingHarborDeploymentRow {
    pub country: u32,
    pub ordinal: u32,
    pub province: i32,
    pub status: u8,
}
typed_codec!(FloatingHarborDeploymentRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (province): I32, 1 => (status): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FacilityMapFlagRow {
    pub province: u32,
    pub flag: u8,
}
typed_codec!(FacilityMapFlagRow, 32, |row| row.province as u128, |key| Self { province: key as u32, ..Default::default() },
 { 0 => (flag): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RadarCoverageRow {
    pub country: u32,
    pub ordinal: u32,
    pub state: Option<u32>,
    pub provinces: Vec<u32>,
    pub radius: Option<i64>,
}
typed_codec!(RadarCoverageRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (state): OptionalU32, 1 => (provinces): U32List, 2 => (radius): OptionalI64 }, |_row| Ok(()));

impl OpaqueValue for ShipCounterInputs {}
impl OpaqueValue for CharacterPortrait {}

opaque_value!(Option<[Option<i64>; 4]>);
