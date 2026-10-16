use super::*;

/// Hourly companion of Countries, keyed by its raw country link ID: the stored
/// map colour and the name operands the ruling party and subject tables lack.
/// Components retain the current primary color's f32 bits without conversion.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryAppearanceRow {
    pub country: u32,
    /// Ordered raw subject roster; empty at zero count.
    pub subjects: Recorded<Vec<i32>>,
    pub red: u32,
    pub green: u32,
    pub blue: u32,
    pub alpha: u32,
    /// Native string bytes; an empty tag means no cosmetic tag.
    pub cosmetic_tag: Vec<u8>,
    /// The original tag a dynamic country is named after; empty otherwise.
    pub name_tag: Vec<u8>,
    /// The ruling party's first country leader's ideology token; empty without one.
    pub leader_ideology: Vec<u8>,
    /// Raw overlord link; positive when the country is a subject.
    pub overlord: i32,
    /// The runtime 3D graphical culture used as a prefix for unit entities;
    /// empty when unset.
    pub graphical_culture: Vec<u8>,
    /// The runtime 2D graphical culture used as a prefix for technology
    /// sprites in the equipment icon fallback; empty when unset.
    pub graphical_culture_2d: Vec<u8>,
    /// The character id of the same first country leader. `None` when `leader_ideology`
    /// is empty (no leader), or when the leader's character could not be read; never zero
    /// for missing.
    pub leader_character: Option<u32>,
    /// The name, adjective and definite name before the last rebuild that
    /// changed them, exact bytes in the recording client's language. Display
    /// uses the current text when the old text is empty. The three are written
    /// together; `None` when they were unreadable or one exceeded 256 bytes.
    pub old_name: Option<Vec<u8>>,
    pub old_adjective: Option<Vec<u8>>,
    pub old_definite_name: Option<Vec<u8>>,
    /// The current UI colour's f32 bits (`color_ui` with COUNTRY_UI_COLOR_*
    /// modifiers): counter plates in country-colour mode take it through the
    /// subject-colour chain.
    pub ui_red: u32,
    pub ui_green: u32,
    pub ui_blue: u32,
    pub ui_alpha: u32,
    /// Original-tag bytes, without the generator's dynamic-slot gate. Positive
    /// empty strings remain empty; original_tag_reference distinguishes their
    /// presence from absence. None is a counted read/arena refusal; older
    /// formats use NotRecorded.
    pub original_tag: Recorded<Option<Vec<u8>>>,
    /// Exact signed original-country reference, preserving alias identity.
    /// Positive selects original_tag, including an empty string; nonpositive
    /// selects the own tag. None is unreadable; NotRecorded is a historical gap.
    pub original_tag_reference: Recorded<Option<i32>>,
    /// Retired selected transport convoy pool and reservations.
    pub transport_convoys: Recorded<Retired>,
}
impl CountryAppearanceRow {
    /// The stored primary colour as sRGB bytes. The floats already carry the
    /// political map's HSV modifiers; conversion truncates `f * 255`.
    pub fn map_color(&self) -> [u8; 3] {
        [self.red, self.green, self.blue].map(|component| (f32::from_bits(component) * 255.0) as u8)
    }
}

typed_codec!(CountryAppearanceRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (red): U32, 1 => (green): U32, 2 => (blue): U32, 3 => (alpha): U32, 4 => (cosmetic_tag): Bytes, 5 => (name_tag): Bytes, 6 => (leader_ideology): Bytes, 7 => (overlord): I32, 8 => (graphical_culture): Bytes, 9 => (leader_character): OptionalU32, 10 => (old_name): Opaque, 11 => (old_adjective): Opaque, 12 => (old_definite_name): Opaque, 13 => (ui_red): U32, 14 => (ui_green): U32, 15 => (ui_blue): U32, 16 => (ui_alpha): U32, 17 => (graphical_culture_2d): Bytes, 18 => (subjects): Opaque, 19 => (original_tag): Opaque, 20 => (original_tag_reference): OptionalI32, 21 => (transport_convoys): Opaque }, |_row| Ok(()));

/// Hourly stack-icon inputs, keyed by the full Divisions handle.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ArmyIconInputRow {
    pub division: u64,
    /// Raw CanAdd gate byte; retained without boolean normalization.
    pub separate_counter: u8,
    /// Full alternate-template handle, or zero for a null native pointer.
    pub alternate_template: u64,
}
typed_codec!(ArmyIconInputRow, 64, |row| row.division as u128, |key| Self { division: key as u64, ..Default::default() },
 { 0 => (separate_counter): U8, 1 => (alternate_template): U64 }, |_row| Ok(()));

/// Additional hourly native counter operands. None only when decoding formats20..22.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ShipCounterInputs {
    pub roster_ordinal: u32,
    pub definition_valid: u8,
    pub carrier_air_base: u64,
    pub maximum_strength: i64,
    /// Raw signed fixed5 AA base, before the ship's 0x88/0x89 modifiers.
    pub anti_air_base: i64,
    pub maximum_organisation: i64,
    /// OR of full variant flag words for strictly positive equipment amounts.
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

/// Current selected-variant operands of the native composition comparator. The general
/// definition ordinal resolves through recorded Equipment content, including generated
/// definitions; daily variant rows do not substitute here.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
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
opaque_value!(Option<HullVariantInputs>);

/// Hourly entity-model and counter operands, keyed by the full Ships handle.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ShipModelRow {
    pub ship: u64,
    pub variant: u64,
    pub hull_sprite: Vec<u8>,
    pub fallback_sprite: Vec<u8>,
    pub model_number: i32,
    /// Full native formation-flags word.
    pub formation_flags: u64,
    pub counter: Option<ShipCounterInputs>,
    /// Exact signed ship-definition index. The recorded content roster
    /// resolves the original name even when multiple definitions have token0.
    /// None is a field-local guarded refusal; old formats use NotRecorded.
    pub subunit_index: Recorded<Option<i32>>,
    /// Retired strike operands of the port strike-force readiness hover.
    pub strike: Recorded<Retired>,
    /// Fresh current tuple for the already selected variant; a null variant
    /// demands none. Individual later failures never erase an earlier field.
    pub hull_variant: Recorded<Option<HullVariantInputs>>,
}
typed_codec!(ShipModelRow, 64, |row| row.ship as u128, |key| Self { ship: key as u64, ..Default::default() },
 { 0 => (variant): U64, 1 => (hull_sprite): Bytes, 2 => (fallback_sprite): Bytes, 3 => (model_number): I32, 4 => (formation_flags): U64, 5 => (counter): Opaque, 6 => (subunit_index): OptionalI32, 7 => (strike): Opaque, 8 => (hull_variant): Opaque }, |_row| Ok(()));

/// Complete hourly province census, including empty ordered occupant arrays.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProvinceUnitOrderRow {
    pub province: u32,
    pub mixed: Vec<u64>,
    pub naval: Vec<u64>,
}
typed_codec!(ProvinceUnitOrderRow,32,|r| r.province as u128,|k| Self { province:k as u32,..Default::default() },
 {0 => (mixed): U64List,1 => (naval): U64List},|_r| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CharacterPortrait {
    pub category: u32,
    pub size: u32,
    pub is_set: u8,
    pub kind: u32,
    pub identity: Vec<u8>,
}
value_struct!(CharacterPortrait {category:u32,size:u32,is_set:u8,kind:u32,identity:Vec<u8>});
/// Portraits for the full character roster, independently of the orders-group
/// HQ leader census.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CharacterPortraitRow {
    pub character: u32,
    pub portraits: Vec<CharacterPortrait>,
}

/// Portrait role `civilian`: its token 0x4b26 maps to role 0.
pub const PORTRAIT_CIVILIAN: u32 = 0;
/// Portrait size `large`: token 0x4c28 maps to size 1; the `_small` path
/// is built only for size 0.
pub const PORTRAIT_LARGE: u32 = 1;

/// The holder's lower-bound search for `(size, category)` over entries
/// ordered by size then category, falling back to the first entry of the
/// same size; the caller tries only the returned entry.
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
/// Stored strings and character reference for every distinct leader of an
/// OrdersGroup with a nonnull HQ provider and leader, across all countries.
///
/// The census is independent of the recording client's country and camera. A
/// registered leader outside this HQ scope has no row; that does not prove an
/// empty portrait.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LeaderPortraitRow {
    pub leader: u64,
    pub character: u64,
    pub portrait_c0: Vec<u8>,
    pub portrait_e0: Vec<u8>,
    pub portrait_100: Vec<u8>,
}
typed_codec!(LeaderPortraitRow,64,|r| r.leader as u128,|k| Self {leader:k as u64,..Default::default()},
 {0 => (character): U64,1 => (portrait_c0): Bytes,2 => (portrait_e0): Bytes,3 => (portrait_100): Bytes},|_r| Ok(()));

/// Naval counter operands keyed by task-force handle. The embedded mission
/// supplies `is_in_regions`, `running_away` (unsaved; set when sent to repair
/// or refit, cleared when that action is cancelled), `engagement_rule`,
/// `mission_region_count` and `bombardment_region` (zero for null).
/// `repair_parent` is a full reference; `detached_activity` (saved spelling
/// 1..4) and `repair_last_mission` (saved when nonzero) describe detached activity.
///
/// received_supply_ratio is signed fixed5, read by the low-supply child of the
/// port and at-sea counters.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NavalSearcher {
    /// A failed raw slot stays unknown without erasing earlier ordered entries.
    pub reference: Option<u64>,
    /// None is resolver/identity refusal; false is a known dangling reference.
    pub resolves: Option<bool>,
    /// Generic unit roster owner, independent of logical owner and unit class.
    pub roster_owner: Option<u32>,
}
value_struct!(NavalSearcher { reference: Option<u64>, resolves: Option<bool>, roster_owner: Option<u32> });

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NavalEngagementTarget {
    pub reference: u64,
    /// Native raw resolution only; no invented class, vtable or owner filter.
    pub resolves: Option<bool>,
}
value_struct!(NavalEngagementTarget { reference: u64, resolves: Option<bool> });

/// Ordered genuine port/strike mission inputs for a callback hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalPortMissionInputs {
    pub sample_hour: PacketHour,
    pub searching: Option<Vec<NavalSearcher>>,
    pub engagement_target: Option<NavalEngagementTarget>,
    /// Raw bytes and native order. A later unknown string cannot invalidate
    /// an earlier decisive match; an unknown before a match refuses that arm.
    pub avoided_ship_types: Option<Vec<Option<Vec<u8>>>>,
}
value_struct!(NavalPortMissionInputs {
    sample_hour: PacketHour, searching: Option<Vec<NavalSearcher>>,
    engagement_target: Option<NavalEngagementTarget>, avoided_ship_types: Option<Vec<Option<Vec<u8>>>>
});
opaque_value!(Option<NavalPortMissionInputs>);

/// Hourly naval counter operands, including field-local optional supplements.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
    /// The received-supply ratio (signed fixed5) behind the low-supply child.
    pub received_supply_ratio: i64,
    /// The attached naval-HQ count behind the combat icon's HQ frame.
    pub attached_headquarters: i32,
    /// Exact retained unit-name bytes, including empty text. None is an
    /// independent guarded-read/name-arena refusal; older formats use
    /// NotRecorded. Display keeps the recording-language guard.
    pub name: Recorded<Option<Vec<u8>>>,
    /// Stored supply-consumer inputs for the detailed at-sea supply footer. None is an
    /// independent scalar-read refusal; an absent origins vector preserves the scalar
    /// pack and records its own read/arena refusal. Older formats use NotRecorded. No
    /// task-force save path is established.
    pub supply: Recorded<Option<UnitSupplyInputs>>,
    /// Ordered port operands; unrelated name, supply and motion failures do
    /// not withhold this packet, or vice versa. Older formats use NotRecorded.
    pub port_mission: Recorded<Option<NavalPortMissionInputs>>,
}
typed_codec!(TaskForceCounterInputRow, 64, |r| r.task_force as u128, |k| Self { task_force: k as u64, ..Default::default() },
 { 0 => (is_in_regions): U8, 1 => (running_away): U8, 2 => (engagement_rule): I32, 3 => (mission_region_count): I32, 4 => (bombardment_region): U32, 5 => (repair_parent): U64, 6 => (detached_activity): I32, 7 => (repair_last_mission): I32, 8 => (received_supply_ratio): I64, 9 => (attached_headquarters): I32, 10 => (name): Opaque, 11 => (supply): Opaque, 12 => (port_mission): Opaque }, |_r| Ok(()));

/// Hourly simulation operands of every class-13 railway gun, keyed by its full handle. Owners are
/// raw country tags; `province` is its authenticated location (zero when its roster holds it
/// unlocated). The mixed province occupant order is already in ProvinceUnitOrder; nothing here is a
/// client's rendered icon, selected country or camera state. `railway_building_level` is the signed
/// level of the railway building at the gun's province (`None` for a null rail info or building
/// pointer), the location warning's input. `combat_role` is 0 without a combat, 1 when the logical
/// owner attacks in its first combat and 2 otherwise; `combat_province` is that combat's province.
/// `combat_count` is the stored unit combat-vector count, and `attacking` whether the owner attacks
/// in any of those combats (the unit-action query's combat branch). `reference_entry_count` is the
/// stored count of the gun's own combat reference array, the save's `combat` list.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
    /// The stored count of the gun's own combat references, which the save writes as its `combat`
    /// list.
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
    /// railway gun's stored unit name (on its primary unit), exact bytes. The getter/setter retain
    /// no authored-name provenance; display only in the recording's language. `None` is a guarded
    /// acquisition refusal.
    pub name: Recorded<Option<Vec<u8>>>,
    /// Hourly first-edge native duration inputs, present for a positive
    /// ordinary route. None is outside demand or a counted capture failure;
    /// older formats use NotRecorded. Static edge distance comes from content.
    pub eta: Recorded<Option<RailwayGunEtaOperands>>,
    /// Shared native low-supply footer, reached by class-three counters too.
    /// Scalar failure is field-local; origins retain an independent refusal.
    pub supply: Recorded<Option<UnitSupplyInputs>>,
    /// Raw saved `withdraw` byte behind the withdrawal arrow.
    /// None is field-local acquisition refusal; historical NotRecorded is distinct.
    pub withdrawal: Recorded<Option<u8>>,
    /// Current callback-hour common operands for a freshly searched class13
    /// route. Each failed domain stays absent; a transfer skips land inputs.
    pub fresh: Recorded<Option<FreshRailwayCounterEta>>,
    pub combat_references: Recorded<Option<UnitCombatReferences>>,
    /// Retired selected transport weight and transfer operands.
    pub transport: Recorded<Retired>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RailwayGunEtaOperands {
    /// Raw fixed5 movement progress.
    pub numerator: i64,
    /// Passively observed railway-gun movement speed for its actual location
    /// and first ordinary province, sampled every hour.
    pub current_speed: i64,
}
value_struct!(RailwayGunEtaOperands {
    numerator: i64,
    current_speed: i64
});
opaque_value!(Option<RailwayGunEtaOperands>);

/// Fresh class13 route inputs. Require `sample_hour.fresh_at(frame.hour)`; carried first-edge
/// inputs never substitute for an absent common domain. Native absence of the genuine unit's orders
/// group is `Some(0)`, independently of the railway assignee group already recorded by the counter
/// prefix.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
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
opaque_value!(Option<FreshRailwayCounterEta>);
typed_codec!(RailwayGunCounterInputRow, 64, |r| r.gun as u128, |k| Self { gun: k as u64, ..Default::default() },
 { 0 => (roster_owner): U32, 1 => (expeditionary_owner): U32, 2 => (logical_owner): U32, 3 => (province): U32, 4 => (naval_transfer): U8, 5 => (transfer_action_status): U8, 6 => (exiled): U8, 7 => (retreat): U8, 8 => (strategic_redeployment): U8, 9 => (reference_entry_count): I32, 10 => (assignee_group): U64, 11 => (subunit_token): U32, 12 => (current_strength): I64, 13 => (maximum_strength): I64, 14 => (received_supply_ratio): I64, 15 => (railway_building_level): Opaque, 16 => (target): U32, 17 => (combat_role): U8, 18 => (combat_province): U32, 19 => (combat_count): U32, 20 => (attacking): U8, 21 => (ordinary_path): U32List, 22 => (alternate_path): U32List, 23 => (name): Opaque, 24 => (eta): Opaque, 25 => (supply): Opaque, 26 => (withdrawal): Opaque, 27 => (fresh): Opaque, 28 => (combat_references): Opaque, 29 => (transport): Opaque }, |row| check_unit_combat_references(&row.combat_references, row.combat_count));

#[cfg(test)]
mod fresh_railway_eta_tests {
    use super::*;

    #[test]
    fn fresh_packet_keeps_refused_zero_and_signed_extreme_operands_distinct() {
        for inputs in [
            FreshRailwayCounterEta {
                sample_hour: PacketHour::Host,
                maximum_speed: Some(i64::MIN),
                land_supply_factor: Some(i64::MAX),
                orders_group: Some(u64::MAX),
            },
            FreshRailwayCounterEta {
                sample_hour: PacketHour::Host,
                maximum_speed: Some(0),
                land_supply_factor: None,
                orders_group: Some(0),
            },
            FreshRailwayCounterEta::default(),
        ] {
            let mut bytes = Vec::new();
            inputs.encode(&mut bytes).unwrap();
            assert_eq!(
                bytes.len(),
                crate::value::Value::encoded_bytes(&inputs).unwrap()
            );
            let mut input = Decoder::new(&bytes);
            assert_eq!(FreshRailwayCounterEta::decode(&mut input).unwrap(), inputs);
            input.finish().unwrap();
            assert!(
                FreshRailwayCounterEta::decode(&mut Decoder::new(&bytes[..bytes.len() - 1]))
                    .is_err()
            );
        }
        assert!(FreshRailwayCounterEta::decode(&mut Decoder::new(&[2])).is_err());
    }

    #[test]
    fn only_fresh_suffix_changes_and_an_explicit_refusal_clears_it() {
        let previous = RailwayGunCounterInputRow {
            gun: u64::MAX,
            name: Some(vec![0xff]).into(),
            eta: Some(RailwayGunEtaOperands {
                numerator: i64::MIN,
                current_speed: 7,
            })
            .into(),
            supply: None.into(),
            withdrawal: Some(0).into(),
            fresh: None.into(),
            combat_count: 1,
            combat_references: Some(UnitCombatReferences {
                sample_hour: PacketHour::Host,
                entries: vec![UnitCombatReference {
                    combat: u64::MAX,
                    kind: 0,
                }],
            })
            .into(),
            ..Default::default()
        };
        let row = RailwayGunCounterInputRow {
            fresh: Some(FreshRailwayCounterEta {
                sample_hour: PacketHour::Host,
                maximum_speed: Some(0),
                land_supply_factor: None,
                orders_group: Some(u64::MAX),
            })
            .into(),
            ..previous.clone()
        };
        assert_eq!(RailwayGunCounterInputRow::SCHEMA.len(), 30);
        assert_eq!(TableId::RailwayGunCounterInputs.mask_width(), 4);
        let mut bytes = Vec::new();
        row.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            RailwayGunCounterInputRow::decode_fields(row.key(), &mut input).unwrap(),
            row
        );
        input.finish().unwrap();
        for (old, new) in [(&previous, &row), (&row, &previous)] {
            let delta = new.delta(Some(old)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 27);
            assert_eq!(delta.apply(Some(old.clone())).unwrap(), *new);
        }
        let historical = RailwayGunCounterInputRow {
            fresh: Recorded::NotRecorded,
            ..row
        };
        assert_ne!(historical.fresh, previous.fresh);
        assert!(historical.encode_fields(&mut Vec::new()).is_err());
    }
}

/// Fleet counter operands keyed by fleet handle. RGBA are the stored colour's
/// component f32 bit patterns; `mission_region_count` is the stored count;
/// `task_forces` is the ordered roster as full handles.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FleetCounterInputRow {
    pub fleet: u64,
    pub red: u32,
    pub green: u32,
    pub blue: u32,
    pub alpha: u32,
    pub mission_region_count: i32,
    pub task_forces: Vec<u64>,
}
typed_codec!(FleetCounterInputRow, 64, |r| r.fleet as u128, |k| Self { fleet: k as u64, ..Default::default() },
 { 0 => (red): U32, 1 => (green): U32, 2 => (blue): U32, 3 => (alpha): U32, 4 => (mission_region_count): I32, 5 => (task_forces): U64List }, |_r| Ok(()));

/// Slot-zero subject pointer/type/parties for diplomacy access.
/// `relation_type: None` is an observed null pointer. A non-subject type has
/// no parties because they were not read. Type30D1 retains exact known
/// words: first zero leaves second unreached; a refused first with known
/// second zero retains that independently proved truth. No absent word
/// becomes zero. The enclosing recorded Option distinguishes an unknown OR from null.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ZeroSubjectInputs {
    pub relation_type: Option<u32>,
    pub parties: Option<[Option<u32>; 2]>,
}
value_struct!(ZeroSubjectInputs { relation_type: Option<u32>, parties: Option<[Option<u32>; 2]> });
opaque_value!(Option<ZeroSubjectInputs>);

/// Country counter and serializer operands keyed by raw country link id.
/// `pride_of_the_fleet` is the full ship reference (zero for the null
/// sentinel's zero bits, otherwise raw); `owned_state_count` is the
/// owned-state array count tested before writing a supply block.
/// `controlled_state_count` is the array count returned by
/// `num_of_controlled_states`; controller changes maintain this array,
/// adding only states with provinces.
///
/// The oracle compares controlled_state_count with the saved states the country
/// controls, and first_division with its first saved division: the save keeps
/// the roster order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryCounterInputRow {
    pub country: u32,
    pub pride_of_the_fleet: u64,
    pub owned_state_count: i32,
    pub controlled_state_count: i32,
    /// Full handle of the first division in the country roster, zero when
    /// empty: the invasion route's last unit fallback.
    pub first_division: u64,
    /// Stored owned, controlled and core state order. None is a counted
    /// capture refusal; counts remain independently recorded.
    pub owned_states: Recorded<Option<Vec<u32>>>,
    pub controlled_states: Recorded<Option<Vec<u32>>>,
    pub core_states: Recorded<Option<Vec<u32>>>,
    /// Stored army and navy leader order, full handles; null is zero.
    /// None is a counted capture refusal; older formats use NotRecorded.
    pub army_leaders: Recorded<Option<Vec<u64>>>,
    pub navy_leaders: Recorded<Option<Vec<u64>>>,
    /// Production policy lexer-token IDs in stored order. None is null or a
    /// counted refusal.
    pub mio_policies: Recorded<Option<Vec<u32>>>,
    /// Raw neighbor references in stored order. None is a counted capture
    /// refusal; rebuilt on load, not saved.
    pub neighbors: Recorded<Option<Vec<u32>>>,
    /// Complete sorted unique canonical targets of this country's stored
    /// co-belligerent truth, including zero. Direction is owner to target;
    /// no active-war veto or faction reconstruction is applied. None is a
    /// counted independent read/arena refusal, not an empty set.
    pub co_belligerents: Recorded<Option<Vec<u32>>>,
    /// Slot-zero war-pointer presence (0/1), then the pointed relation's exact retired byte (zero
    /// for a null pointer). Independent of the cache set; these native runtime operands have no
    /// text-save counterpart.
    pub zero_war: Recorded<Option<[u8; 2]>>,
    /// Retired owner-shared transport speeds of the selected transport
    /// preview; the river, naval and theatre domains retired before them.
    pub movement: Recorded<Retired>,
    /// Exact slot-zero status's subject pointer, type and parties, independent of zero_war and the
    /// directed cache. None is a guarded refusal; text saves omit slot0.
    pub zero_subject: Recorded<Option<ZeroSubjectInputs>>,
    /// Retained cached ship-name bytes for this row's full `pride_of_the_fleet`
    /// handle. None is unused for a zero or stale reference, otherwise a
    /// guarded read/name-arena refusal. Some(empty) is a known empty name.
    /// Older formats use NotRecorded; display keeps the recording-language guard.
    pub pride_name: Recorded<Option<Vec<u8>>>,
    /// Current-hour positive receiver's actual diplomacy matrix[0] operands,
    /// including the military-access flag-one arm. The nested target is the
    /// cell's exact raw word, which may be a positive alias; established
    /// exact-zero `zero_war`/`zero_subject` fields are not substituted. None is
    /// a counted source/status refusal; nested None values retain their
    /// independent earlier skips or guarded refusals. Source faction and
    /// reached target faction use same-hour country/faction evidence. Older
    /// recordings retain NotRecorded.
    pub forward_visibility: Recorded<Option<ReverseZeroVisibilityInputs>>,
    /// The selected-attack scalars recordings through format 101 captured
    /// ([`CountryAttackInputs`], effective values); current writers leave it
    /// empty ([`Witnessed`]) and record [`Self::attack_modifiers`] instead.
    pub attack: Recorded<Witnessed<CountryAttackInputs>>,
    /// The direct-war packet recordings through format 101 captured. Current
    /// writers store None: playback derives every cell from the hour's
    /// WarRelations ([`crate::directed_war_derive`]), and an older packet,
    /// where present, still answers for its receiver.
    pub directed_wars: Recorded<Option<DirectedWars>>,
    /// The country accumulator's raw `land_night_attack` and
    /// `land_bunker_effectiveness_factor` entries, which the selected
    /// army's attack list reads for the division's owner and the target's
    /// controller during playback. None is a counted read refusal;
    /// older formats use NotRecorded.
    pub attack_modifiers: Recorded<Option<CountryAttackModifiers>>,
    /// The same accumulator's raw movement and attrition entries
    /// ([`CountryMovementModifiers`]), read and carried with the row. None is a counted
    /// read refusal; older formats use NotRecorded.
    pub movement_modifiers: Recorded<Option<CountryMovementModifiers>>,
}
typed_codec!(CountryCounterInputRow, 32, |r| r.country as u128, |k| Self { country: k as u32, ..Default::default() },
 { 0 => (pride_of_the_fleet): U64, 1 => (owned_state_count): I32, 2 => (controlled_state_count): I32, 3 => (first_division): U64, 4 => (owned_states): Opaque, 5 => (controlled_states): Opaque, 6 => (core_states): Opaque, 7 => (army_leaders): Opaque, 8 => (navy_leaders): Opaque, 9 => (mio_policies): Opaque, 10 => (neighbors): Opaque, 11 => (co_belligerents): Opaque, 12 => (zero_war): Opaque, 13 => (movement): Opaque, 14 => (zero_subject): Opaque, 15 => (pride_name): Opaque, 16 => (forward_visibility): Opaque, 17 => (attack): Opaque, 18 => (directed_wars): Opaque, 19 => (attack_modifiers): Opaque, 20 => (movement_modifiers): Opaque }, |_row| {
    Ok(())
});

/// Ordered naval launch occurrence from actual Country0's strategic navy.
/// `base_province` is the base's exact province word, interpreted as signed.
/// `country` is its holder navy's raw country word, copied without
/// country-link resolution.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LaunchNavalBase {
    pub base_province: u32,
    pub country: u32,
}
value_struct!(LaunchNavalBase {
    base_province: u32,
    country: u32
});

/// Actual Country0's launch provider. NativeNull records a successfully read
/// null manager slot, never an empty roster or an acquisition refusal.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
        // Preserve the unreleased packet's previous None/Some list encodings.
        match input.u8()? {
            0 => Ok(Self::Unavailable),
            1 => Vec::<LaunchNavalBase>::decode(input).map(Self::Bases),
            2 => Ok(Self::NativeNull),
            _ => Err(invalid("launch naval provider discriminator invalid")),
        }
    }
}

/// Current-hour actual Country0 launch operands shared by all faction callers.
/// Vectors retain native order and repeated occurrences. Unavailable and
/// owned_states None are independent refusals; Bases(empty) observes a list.
/// Base and state IDs retain their exact bits; no persistent handle is inferred.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
opaque_value!(Option<ReservedLaunchSources>);

/// One directed diplomacy-status cell reached by the access query. The enclosing row or packet
/// assigns its receiver and matrix ordinal: actual Country0's reverse cell or a positive receiver's
/// matrix[0] cell. `target` is the cell's exact raw target word, never its matrix ordinal or a
/// resolved alias. `war` retains the war pointer's presence and the pointed retired byte; [0, 0] is
/// native null, [1, 0] is an active-war veto. Each None is an independent refused or unreached
/// operand. A present subject with no type is native null; ordered optional parties retain the lazy
/// OR reads.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReverseZeroVisibilityInputs {
    pub target: Option<u32>,
    pub war: Option<[u8; 2]>,
    pub subject: Option<ZeroSubjectInputs>,
    /// The independent military-access gate: presence 0/1 from the exact
    /// eight-byte pointer, without a pointed-object read. None is an earlier
    /// skip or guarded refusal. Flag-zero queries do not demand this operand;
    /// flag-one queries admit only presence 0/1.
    pub military_access: Option<u8>,
}
value_struct!(ReverseZeroVisibilityInputs {
    target: Option<u32>,
    war: Option<[u8; 2]>,
    subject: Option<ZeroSubjectInputs>,
    military_access: Option<u8>,
});
opaque_value!(Option<ReverseZeroVisibilityInputs>);

/// Current-hour actual Country0 receiver operands, in native matrix order.
/// The complete vector length is the observed matrix count, including slot0;
/// no country census count is substituted. An absent cell is uncertified,
/// distinct from a present cell's native null or independently skipped reads.
/// Consumers require `sample_hour.fresh_at` the queried branch frame's hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReservedReverseVisibilityInputs {
    pub sample_hour: PacketHour,
    pub cells: Vec<Option<ReverseZeroVisibilityInputs>>,
}
value_struct!(ReservedReverseVisibilityInputs {
    sample_hour: PacketHour,
    cells: Vec<Option<ReverseZeroVisibilityInputs>>,
});
opaque_value!(Option<ReservedReverseVisibilityInputs>);

/// Admission of the complete actual-owner0 CountryIntel sample, paired with
/// the same branch-scoped rows. The hour is their acquisition hour and stays
/// unchanged on a carried daily sample. Count0 certifies a known empty vector;
/// native null or refusal has no certificate. Consumers validate the signed
/// native count domain and the complete sparse row section before lookup.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
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
    #[cfg(test)]
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
opaque_value!(Option<ReservedCountryIntelSample>);

/// Native country slot zero's naming operands and original tag getter.
/// Slot zero is excluded from Countries and the positive-only link census.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReservedCountryNamingRow {
    pub tag: Vec<u8>,
    pub name_tag: Vec<u8>,
    pub cosmetic_tag: Vec<u8>,
    pub leader_ideology: Vec<u8>,
    /// None when no political status or ruling party exists.
    pub group: Option<Vec<u8>>,
    pub overlord: i32,
    /// None when no diplomacy/autonomy status exists.
    pub autonomy: Option<Vec<u8>>,
    /// Stored string-length witnesses: ordinary, definite, adjective bits.
    pub built: u8,
    /// Original-tag input, independent of the generator's dynamic-slot gate
    /// on `name_tag`. Positive bytes may be empty; original_tag_reference
    /// preserves presence. None is a counted read/arena refusal. Positive bytes
    /// are exact.
    pub original_tag: Recorded<Option<Vec<u8>>>,
    /// Exact signed original-tag reference word, independent of the generator's slot gate.
    pub original_tag_reference: Recorded<Option<i32>>,
    /// Actual country-link vectors, indexed by canonical bucket and holding
    /// ordered raw country references. Empty buckets, duplicate members and
    /// alias occurrences remain exact; None is a guarded read/arena refusal.
    pub original_tag_buckets: Recorded<Option<Vec<Vec<u32>>>>,
    /// Physical country pointer-array order, with each occurrence's own raw id.
    /// Reserved zero, aliases and duplicate IDs are retained. None is a
    /// guarded read/arena refusal, independent of the bucket cache.
    pub country_array: Recorded<Option<Vec<u32>>>,
    /// The exact signed reserved country's owned-state count word.
    /// None is an independent guarded refusal, never an inferred zero.
    pub owned_state_count: Recorded<Option<i32>>,
    /// Signed country-database boundary compared with the canonical slot.
    /// None is a guarded refusal; NotRecorded preserves the historical prefix.
    pub dynamic_tag_boundary: Recorded<Option<i32>>,
    /// Reserved faction identity: Some(0) is null, otherwise an authenticated
    /// full faction handle. None is a refusal.
    pub faction: Recorded<Option<u64>>,
    /// Retired naval movement packet: physical navy nearest-base caches and
    /// reserved owner theatre membership, read only by fresh counter routes.
    pub movement: Recorded<Retired>,
    /// Fresh actual Country0 theatre count, only after the entire owner-zero
    /// LandTheatres suffix commits with complete regular/army native roster
    /// proof. Some(0) certifies the fresh empty theatre vector; None refuses.
    pub land_theatre_count: Recorded<Option<u32>>,
    /// Current-hour ordered actual Country0 launch lists, independent of its
    /// naming, faction, count and movement operands on a healthy singleton.
    /// None is a current packet refusal; NotRecorded is historical absence.
    pub launch_sources: Recorded<Option<ReservedLaunchSources>>,
    /// Current-hour actual Country0 receiver matrix. None is a whole-vector
    /// root/header/capacity refusal; cells preserve their own certification.
    /// Available only through a complete required naming singleton prefix.
    pub reverse_visibility: Recorded<Option<ReservedReverseVisibilityInputs>>,
    /// Certificate paired with actual-owner0 CountryIntel's latest complete
    /// daily/deferred/forced sample. None is unadmitted; a carried value keeps
    /// its acquisition hour. Historical field absence remains NotRecorded.
    pub country_intel_sample: Recorded<Option<ReservedCountryIntelSample>>,
    /// Actual Country0's selected-attack scalars as recordings through format
    /// 101 captured them ([`CountryAttackInputs`]); current writers leave it
    /// empty ([`Witnessed`]).
    pub attack: Recorded<Witnessed<Option<CountryAttackInputs>>>,
    /// Retired raw mapping and spelling observations of the selected-attack map.
    pub raw_country_observations: Recorded<Retired>,
    /// Retired transport mission packet of the selected transport preview.
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

/// Orders-group counter operands keyed by group handle; `withdrawing` is
/// the raw saved byte tested for the HQ leader-en-route glyph.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OrdersGroupCounterInputRow {
    pub group: u64,
    pub withdrawing: u8,
    /// Stored signed fixed5 HQ distance. None is a counted isolated read
    /// failure; older formats use NotRecorded.
    pub hq_distance: Recorded<Option<i32>>,
    pub leader_enroute_days: Recorded<Option<i32>>,
    pub border_war_presence: Recorded<Option<u8>>,
    /// Stored signed value used by the invasion communications branch. Distinct from the HQ
    /// distance; None is a counted independent refusal.
    pub invasion_source_distance: Recorded<Option<i32>>,
    /// The commander operands recordings through format 101 captured
    /// ([`OrdersGroupAttackInputs`]); current writers leave it empty
    /// ([`Witnessed`]) unless the collector's `attack_witness` diagnostic
    /// sampled one of the group's divisions.
    pub attack: Recorded<Witnessed<OrdersGroupAttackInputs>>,
    /// The direct commander's attack operands (the orders group's direct commander,
    /// [`CommanderAttackStats`]): `Some(None)` for a group without one, None for a counted read
    /// refusal. Older formats use NotRecorded.
    pub commander_attack: Recorded<Option<Option<CommanderAttackStats>>>,
    /// The direct commander's terrain movement ([`CommanderMovementStats`]), read and
    /// refused with [`Self::commander_attack`]. Older formats use NotRecorded.
    pub commander_movement: Recorded<Option<Option<CommanderMovementStats>>>,
}
typed_codec!(OrdersGroupCounterInputRow, 64, |r| r.group as u128, |k| Self { group: k as u64, ..Default::default() },
 { 0 => (withdrawing): U8, 1 => (hq_distance): OptionalI32,
   2 => (leader_enroute_days): OptionalI32, 3 => (border_war_presence): Opaque,
   4 => (invasion_source_distance): OptionalI32, 5 => (attack): Opaque,
   6 => (commander_attack): Opaque, 7 => (commander_movement): Opaque }, |_row| Ok(()));

/// Picture-directory inputs for the same HQ scope as LeaderPortraits,
/// keyed by leader handle. `country` is the raw leader-country reference.
/// `tag_reference` is the positive original-country reference, else the
/// country id; `tag` is its exact tag-string bytes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LeaderPortraitPathRow {
    pub leader: u64,
    pub country: u32,
    pub tag_reference: u32,
    pub tag: Vec<u8>,
}
typed_codec!(LeaderPortraitPathRow, 64, |r| r.leader as u128, |k| Self { leader: k as u64, ..Default::default() },
 { 0 => (country): U32, 1 => (tag_reference): U32, 2 => (tag): Bytes }, |_r| Ok(()));

/// The recording client's stored playing and observer-view country
/// references. Hourly singleton: tag switches, MP controller assignments
/// and observer switches run between ticks, never inside one, so a sample
/// holds the references the tick after it uses. A local tag switch rebuilds
/// every engaged flag at once, so its sample already shows them. A viewing
/// change without that rebuild, such as a new observer view, is rebuilt by
/// the next tick's combat manager and first appears in the next hour's
/// sample. Zero playing country means observer; the observer's viewed
/// country is independent of the human roster. Resolve positive references
/// through the same frame's CountryLinks.
///
/// Players describes the human roster and cannot identify the recording client.
/// Missing or partial LocalPlayer data is neither an observer nor a guessed
/// participant.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LocalPlayerRow {
    pub country: u32,
    pub observer_country: u32,
}

impl LocalPlayerRow {
    /// The viewing choice (the save's `player` tag): signed-positive playing
    /// country, else observer country. `None` when the choice names no country.
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

/// SimulationFeatures: the simulation's DLC feature mask, set by the
/// session's DLC command, the `human_ai` debug byte, and the weather
/// manager's two suspension bytes. Read hourly: the byte can be toggled at
/// runtime. A singleton with key zero; a missing row is unavailable, never
/// a zero mask.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SimulationFeatureRow {
    pub dlc_mask: u32,
    /// The `human_ai` debug byte, ORed into every country's AI-control status.
    pub human_ai: u8,
    /// Weather and day-night suspension bytes, in that order; None is a failed
    /// weather read and does not discard the unrelated feature mask/debug byte.
    pub weather_suspend: Recorded<Option<[u8; 2]>>,
    /// Raw console AI-control byte. None is a counted capture refusal; other
    /// simulation features remain available.
    pub instant_decisions: Recorded<Option<u8>>,
    pub runtime_scalars: Recorded<PoliticalRuntimeScalars>,
    /// The loaded defines of the movement speed's supply, fuel, retreat and air terms
    /// and the attrition hover's supply line ([`MovementDefines`]), read each hour from
    /// their live globals. Older formats use NotRecorded.
    pub movement_defines: Recorded<MovementDefines>,
}
typed_codec!(SimulationFeatureRow, 0, |_row| 0, |_key| Self::default(),
 { 0 => (dlc_mask): U32, 1 => (human_ai): U8, 2 => (weather_suspend): Opaque, 3 => (instant_decisions): Opaque, 4 => (runtime_scalars): Opaque, 5 => (movement_defines): Opaque }, |_row| Ok(()));

/// Loaded defines, raw (fixed5, or a signed 32-bit global sign-extended), each `None`
/// when its own read was refused: the out-of-supply speed penalty at full severity;
/// the retreating term and the withdrawing term; the fuel ratio's floor and the scale
/// of the stat block's fuel word into its capacity; the days out of supply that give the
/// full penalty and the hours of supply grace; the enemy air superiority speed scale
/// and the anti-air reduction's denominator and maximum; and the attrition hover's
/// out-of-supply multiplier.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
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

#[cfg(test)]
#[path = "map_presentation/tests.rs"]
mod weather_tests;

#[cfg(test)]
#[path = "map_presentation/launch_sources_tests.rs"]
mod launch_sources_tests;

#[cfg(test)]
#[path = "map_presentation/reverse_zero_tests.rs"]
mod reverse_zero_tests;

#[cfg(test)]
#[path = "map_presentation/forward_visibility_tests.rs"]
mod forward_visibility_tests;

/// CountryVisibility: each country's effective modifier 0x24B on its own
/// accumulator, which scales every foreign unit's visibility byte when the
/// country is the viewer. Raw fixed5.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryVisibilityRow {
    pub country: u32,
    pub unit_modifier: i64,
}
typed_codec!(CountryVisibilityRow, 32, |row| row.country as u128,
 |key| Self { country: key as u32, ..Default::default() },
 { 0 => (unit_modifier): I64 }, |_row| Ok(()));

/// StateVisibility: one row per state whose raw owner and controller
/// differ, with the owner's state modifier 0x4B, which the fog rebuild adds
/// to the owner's occupied states. Raw fixed5.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StateVisibilityRow {
    pub state: u32,
    /// Raw owner reference the modifier was read for.
    pub country: u32,
    pub network_modifier: i64,
}
typed_codec!(StateVisibilityRow, 32, |row| row.state as u128,
 |key| Self { state: key as u32, ..Default::default() },
 { 0 => (country): U32, 1 => (network_modifier): I64 }, |_row| Ok(()));

/// Hourly: what a unit counter and the 3D model draw from each template.
/// The player can edit a template's icon, composition or model choices at
/// any hour. `priority_unit` is the selected sub-unit, 0 for the null
/// singleton, and `priority_unit_name` its name; `custom_icon` and
/// `custom_icon_nato` are the template's stored icon choice.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionTemplateIconRow {
    pub template: u64,
    pub priority_unit: u32,
    pub priority_unit_name: Vec<u8>,
    pub custom_icon: i32,
    pub custom_icon_nato: u8,
    /// The composition's selected sub-unit, else the priority sub-unit: the
    /// sub-unit the 3D model follows. Zero is the null object, which draws none.
    pub representative: u32,
    /// Nonzero when the representative is a country-local clone.
    pub representative_local: u8,
    /// The composition's (definition, count) entries grouped from its three
    /// grids, in stored order. [`Self::grids`] keeps each grid apart.
    pub subunits: Vec<TemplateSubunit>,
    /// The composition's country, raw: the clone scope of local sub-units and the model fallback
    /// country.
    pub country: i32,
    /// The composition's `foreign_template_tag`, raw.
    pub foreign_country: i32,
    /// Saved model category filter; absent when disengaged.
    pub model_category: Option<u64>,
    /// Saved explicit model entity; empty when unset.
    pub model_entity: Vec<u8>,
    /// Saved excluded equipment variants, full handles in native order.
    pub excluded_variants: Vec<u64>,
    /// Nonzero when this template's model inputs could not be read this hour.
    /// `representative`, `representative_local`, `subunits`, `grids`,
    /// `model_entity` and `excluded_variants` are then empty; the counter's fields, `country`,
    /// `foreign_country` and `model_category` stand.
    pub model_unreadable: u8,
    /// Stored signed composition count used to count formation divisions.
    /// Independent of optional model inputs; None is a counted read refusal.
    pub formation_division_count: Recorded<Option<i32>>,
    /// Fresh display-name operands; SameAs refers to this frame's dictionary.
    pub display_name: Recorded<Option<TemplateDisplayName>>,
    /// Each composition grid's (definition, cell count) entries, from the
    /// same read as `subunits`.
    pub grids: Recorded<TemplateGrids>,
}
typed_codec!(DivisionTemplateIconRow, 64, |row| row.template as u128,
 |key| Self { template: key as u64, ..Default::default() },
 { 0 => (priority_unit): U32, 1 => (priority_unit_name): Bytes, 2 => (custom_icon): I32, 3 => (custom_icon_nato): U8, 4 => (representative): U32, 5 => (representative_local): U8, 6 => (subunits): Opaque, 7 => (country): I32, 8 => (foreign_country): I32, 9 => (model_category): OptionalU64, 10 => (model_entity): Bytes, 11 => (excluded_variants): U64List, 12 => (model_unreadable): U8, 13 => (formation_division_count): OptionalI32, 14 => (display_name): Opaque, 15 => (grids): Opaque }, |_row| Ok(()));

/// Sparse native province overrides. Absence from a complete current table means no
/// override; bytes can be a localization key, not rendered text.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProvinceNameRow {
    pub province: u32,
    pub name: Vec<u8>,
}
typed_codec!(ProvinceNameRow, 32, |row| row.province as u128,
 |key| Self { province: key as u32, name: Default::default() },
 { 0 => (name): Bytes }, |_row| Ok(()));

/// NavalBases: key province (32 bits), 4 payload mask bits.
///
/// The province-to-naval-base hash used to look up and draw bases.
/// `countries` is the base's own ordered list, tested against the viewer;
/// a match adds the map icon's red prohibition overlay.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalBaseRow {
    pub province: u32,
    /// The holder's raw country reference.
    pub country: u32,
    pub level: i32,
    pub healthy_level: i32,
    /// The complete country list as raw country references, in native order: every entry up to its
    /// live count. A count that is not positive is empty, as `ccountry_tag::list_contains` reads
    /// it.
    pub countries: Vec<u32>,
    /// The list header held a positive count with a null data pointer, which
    /// the game never writes; `countries` is then unknown rather than empty.
    pub countries_unreadable: bool,
    /// Actual tag-string bytes for each raw country-list word. None is a
    /// current read refusal; NotRecorded is a historical prefix.
    pub country_tags: Recorded<Option<Vec<Vec<u8>>>>,
}
typed_codec!(NavalBaseRow, 32, |row| row.province as u128, |key| Self { province: key as u32, ..Default::default() },
 { 0 => (country): U32, 1 => (level): I32, 2 => (healthy_level): I32, 3 => (countries): U32List, 4 => (countries_unreadable): Bool, 5 => (country_tags): Opaque }, |_row| Ok(()));

/// BattleSounds: key ordinal (32 bits), 5 payload mask bits.
///
/// The battle ambience's battle list as the game holds it at the hourly
/// boundary, in the order its per-frame pass visits the battles: its spatial
/// tree's nodes in pre-order, each node's battles in array order. The game
/// keeps no battle id; `ordinal` is the battle's place in that list. Battles
/// change only where the game adds and removes combats, so the next hour's
/// frames sound this list. A merge keeps the battle sounding for the
/// recording client, so the list is that client's.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BattleSoundRow {
    pub ordinal: u32,
    /// Exact f32 bits of the battle's map x.
    pub center_x: u32,
    /// Exact f32 bits of the battle's map z (the map's y).
    pub center_z: u32,
    /// Exact f32 bits of the battle's radius.
    pub radius: u32,
    /// Member province ids in the battle's member-array order, repeats kept.
    pub provinces: Vec<u32>,
    /// The battle's main sound instance was live for the recording client.
    pub sounding: bool,
    /// The battle object's native address: the game keeps a battle's sounds
    /// on its object, through merges it survives and regroupings it keeps.
    pub object: u64,
}
typed_codec!(BattleSoundRow, 32, |row| row.ordinal as u128, |key| Self { ordinal: key as u32, ..Default::default() },
 { 0 => (center_x): U32, 1 => (center_z): U32, 2 => (radius): U32, 3 => (provinces): U32List, 4 => (sounding): Bool, 5 => (object): U64 }, |_row| Ok(()));

/// Every country supply system's floating-harbour deployment records
/// (stride 0x68), keyed by canonical country slot and array ordinal.
/// Lookup returns the first record whose province matches; `status` is
/// its raw byte tested by the map's floating-harbour slot.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FloatingHarborDeploymentRow {
    pub country: u32,
    pub ordinal: u32,
    pub province: i32,
    pub status: u8,
}
typed_codec!(FloatingHarborDeploymentRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (province): I32, 1 => (status): U8 }, |_row| Ok(()));

/// The recording client's special-facility map flag for every
/// program province where it is nonzero. Only program code sets it, so it is recorded as
/// stored; the viewer draws the facility icon from it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FacilityMapFlagRow {
    pub province: u32,
    pub flag: u8,
}
typed_codec!(FacilityMapFlagRow, 32, |row| row.province as u128, |key| Self { province: key as u32, ..Default::default() },
 { 0 => (flag): U8 }, |_row| Ok(()));

/// RadarCoverage: key country:ordinal (64 bits), 2 payload mask bits.
///
/// One country radar coverage group. `ordinal` is the vector position,
/// which can change by swap-removal, so it names a slot rather than a
/// persistent group; `state` is the group's host state id.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RadarCoverageRow {
    /// Raw country reference; resolve through CountryLinks to a canonical slot.
    pub country: u32,
    pub ordinal: u32,
    /// Host state id; absent when the native host-state pointer is null.
    pub state: Option<u32>,
    /// Covered province ids in native array order, repeats included.
    pub provinces: Vec<u32>,
}
typed_codec!(RadarCoverageRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, state: Default::default(), provinces: Default::default() },
 { 0 => (state): OptionalU32, 1 => (provinces): U32List }, |_row| Ok(()));

opaque_value!(Option<ShipCounterInputs>);
opaque_value!(Vec<CharacterPortrait>);
