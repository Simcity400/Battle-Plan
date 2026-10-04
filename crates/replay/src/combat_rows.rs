//! Hourly raw combat window operands. Fixed-point stays integer; handles stay complete.
use super::*;
use crate::value::value_struct;
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CombatModifierValue {
    pub id: u32,
    pub value_a: i64,
    pub value_b: i64,
}
value_struct!(CombatModifierValue {
    id: u32,
    value_a: i64,
    value_b: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionCombatModifiers {
    pub division: u64,
    pub ordinal: u32,
    pub entries: Vec<CombatModifierValue>,
}
value_struct!(DivisionCombatModifiers { division: u64, ordinal: u32, entries: Vec<CombatModifierValue> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatCommanderTrait {
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub kind: u32,
    pub relevant: u8,
    pub token: Vec<u8>,
}
value_struct!(LandCombatCommanderTrait { token_id: i32, definition_ordinal: i32, kind: u32, relevant: u8, token: Vec<u8> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatCommanderLearningTrait {
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub required_experience: i32,
    pub experience: i64,
    pub token: Vec<u8>,
}
value_struct!(LandCombatCommanderLearningTrait { token_id: i32, definition_ordinal: i32, required_experience: i32, experience: i64, token: Vec<u8> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatCommanderPortraitModifier {
    pub source: u8,
    pub id: u32,
    pub value: i64,
    pub metadata_flags: u32,
    pub category_mask: u32,
    pub feature_selector: u32,
}
value_struct!(LandCombatCommanderPortraitModifier {
    source: u8,
    id: u32,
    value: i64,
    metadata_flags: u32,
    category_mask: u32,
    feature_selector: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatCommanderPortraitTooltip {
    pub source: u8,
    pub bucket: u32,
    pub key: u32,
    pub text: Vec<u8>,
}
value_struct!(LandCombatCommanderPortraitTooltip { source: u8, bucket: u32, key: u32, text: Vec<u8> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CombatCommanderSkillDefinition {
    pub present: u8,
    pub valid: u8,
    pub level: i32,
    pub required_experience: i32,
    pub category: i32,
}
value_struct!(CombatCommanderSkillDefinition {
    present: u8,
    valid: u8,
    level: i32,
    required_experience: i32,
    category: i32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CombatCommanderPortraitEntry {
    pub role: u32,
    pub size: u32,
    pub is_set: u8,
    pub kind: u32,
    pub text: Vec<u8>,
}
value_struct!(CombatCommanderPortraitEntry { role: u32, size: u32, is_set: u8, kind: u32, text: Vec<u8> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CombatCommanderPresentation {
    pub character: u64,
    pub character_state: u8,
    pub character_country: u32,
    pub character_gender: u32,
    pub character_role_leader: u64,
    pub character_fallback_leader: u64,
    pub character_role_source: u8,
    pub fallback_text: [Vec<u8>; 3],
    pub portraits: Vec<CombatCommanderPortraitEntry>,
    pub role_traits: Vec<LandCombatCommanderTrait>,
}
value_struct!(CombatCommanderPresentation { character: u64, character_state: u8, character_country: u32, character_gender: u32, character_role_leader: u64, character_fallback_leader: u64, character_role_source: u8, fallback_text: [Vec<u8>;3], portraits: Vec<CombatCommanderPortraitEntry>, role_traits: Vec<LandCombatCommanderTrait> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCommanderInputs {
    pub commander: u64,
    pub country: u32,
    pub category: i32,
    pub experience: i64,
    pub skills: [i32; 5],
    pub skill_definitions: [CombatCommanderSkillDefinition; 5],
    pub next_skill: CombatCommanderSkillDefinition,
    pub name: Vec<u8>,
    pub description: Vec<u8>,
    pub presentation: CombatCommanderPresentation,
    pub traits: Vec<LandCombatCommanderTrait>,
    pub learning_traits: Vec<LandCombatCommanderLearningTrait>,
    pub learning_threshold: i64,
    pub modifiers: Vec<LandCombatCommanderPortraitModifier>,
    pub modifier_feature_flags: u32,
    pub modifier_callback_kind: u8,
    pub tooltips: Vec<LandCombatCommanderPortraitTooltip>,
}
value_struct!(NavalCommanderInputs { commander: u64, country: u32, category: i32, experience: i64, skills: [i32; 5], skill_definitions: [CombatCommanderSkillDefinition;5], next_skill: CombatCommanderSkillDefinition, name: Vec<u8>, description: Vec<u8>, presentation: CombatCommanderPresentation, traits: Vec<LandCombatCommanderTrait>, learning_traits: Vec<LandCombatCommanderLearningTrait>, learning_threshold: i64, modifiers: Vec<LandCombatCommanderPortraitModifier>, modifier_feature_flags: u32, modifier_callback_kind: u8, tooltips: Vec<LandCombatCommanderPortraitTooltip> });

/// A division's own commander; its identity is the row's `commander`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatDivisionCommander {
    pub skill: i32,
    /// The raw saved name (CUnitLeader+0x20); the viewer localises it.
    pub name_token: Vec<u8>,
    pub traits: Vec<LandCombatCommanderTrait>,
}
value_struct!(LandCombatDivisionCommander { skill: i32, name_token: Vec<u8>, traits: Vec<LandCombatCommanderTrait> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatSideDetails {
    pub committed: Vec<u64>,
    pub reserves: Vec<u64>,
    pub tactic_id: i32,
    pub tactic_kind: u8,
    pub tactic_counter_id: u32,
    pub tactic_token: Vec<u8>,
    pub air_counts: Vec<i32>,
    /// CLandCombatant+0x138, the CAS tooltip's AA_EFFECTIVENESS input. Its
    /// consumer is L4c's CAS tooltip (viewer combat/tooltips.rs, the opposing
    /// side's value); L4b records it and the oracle compares it to the save.
    pub anti_air_attack: i64,
    pub air_kills: u32,
    pub damage: [i64; 6],
    pub organisation_loss: [i64; 24],
    pub organisation_loss_count: u32,
    pub commander: u64,
    pub commander_skill: i32,
    pub commander_category: i32,
    pub commander_country: u32,
    /// Raw CUnitLeader experience. Consumed by L4c's commander skill tooltip
    /// (viewer combat/tooltips.rs); the oracle compares it to the save.
    pub commander_experience: i64,
    /// Raw saved name (CUnitLeader+0x20) and description (+0x80) tokens; the
    /// viewer localises both in its own language. The description's consumer
    /// is L4c's portrait tooltip (viewer combat/commander_tooltips.rs).
    pub commander_name_token: Vec<u8>,
    pub commander_description_token: Vec<u8>,
    /// CArmyLeader+0x1048 formation reference of a category 0/1 commander,
    /// for the HQ badge (1415C5F10); zero otherwise.
    pub commander_formation: u64,
    pub presentation: CombatCommanderPresentation,
    pub traits: Vec<LandCombatCommanderTrait>,
    pub timed_traits: Vec<LandCombatCommanderTimedTrait>,
    pub learning_traits: Vec<LandCombatCommanderLearningTrait>,
}
value_struct!(LandCombatSideDetails { committed: Vec<u64>, reserves: Vec<u64>, tactic_id: i32, tactic_kind: u8, tactic_counter_id: u32, tactic_token: Vec<u8>, air_counts: Vec<i32>, anti_air_attack: i64, air_kills: u32, damage: [i64; 6], organisation_loss: [i64; 24], organisation_loss_count: u32, commander: u64, commander_skill: i32, commander_category: i32, commander_country: u32, commander_experience: i64, commander_name_token: Vec<u8>, commander_description_token: Vec<u8>, commander_formation: u64, presentation: CombatCommanderPresentation, traits: Vec<LandCombatCommanderTrait>, timed_traits: Vec<LandCombatCommanderTimedTrait>, learning_traits: Vec<LandCombatCommanderLearningTrait> });

/// A timed trait of the side's commander (`CUnitLeader+0xDE0`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatCommanderTimedTrait {
    pub token_id: i32,
    pub definition_ordinal: i32,
    /// Signed; -1 is not an expiring trait, zero still is (140C0B3C0).
    pub remaining_days: i32,
}
value_struct!(LandCombatCommanderTimedTrait {
    token_id: i32,
    definition_ordinal: i32,
    remaining_days: i32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatTacticCountryWeight {
    pub country: u32,
    pub tactic_id: i32,
    pub modifier_key: u32,
    pub metadata_flags: u32,
    pub metadata_gate: i32,
    pub country_factor_raw: i64,
    pub accumulator_raw: i64,
    pub accumulator_entry_present: u8,
}
value_struct!(LandCombatTacticCountryWeight {
    country: u32,
    tactic_id: i32,
    modifier_key: u32,
    metadata_flags: u32,
    metadata_gate: i32,
    country_factor_raw: i64,
    accumulator_raw: i64,
    accumulator_entry_present: u8
});

/// The per-side simulation operands of 1412A77A0's preferred-tactic weights;
/// the candidates, weights and unlocks come from content and
/// CountryTacticUnlocks. A side whose country set emptied has no country
/// preference or weight.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatTacticInputs {
    /// Country, general and field-marshal preferred tactic ids per side.
    pub preferred_tactic_ids: [[i32; 3]; 2],
    pub country_weights: [LandCombatTacticCountryWeight; 2],
    pub country_weight_present: [u8; 2],
}
value_struct!(LandCombatTacticInputs {
    preferred_tactic_ids: [[i32; 3]; 2],
    country_weights: [LandCombatTacticCountryWeight; 2],
    country_weight_present: [u8; 2]
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandDivisionModifier {
    pub raw: i64,
    pub present: u8,
}
value_struct!(LandDivisionModifier {
    raw: i64,
    present: u8
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandDivisionWidthTerm {
    pub subunit: u32,
    pub count: i32,
    pub width: i64,
}
value_struct!(LandDivisionWidthTerm {
    subunit: u32,
    count: i32,
    width: i64
});

/// 1415CA660's exile-frame operands for a committed division whose template
/// is exiled: origin type (+0x1F8) exactly 2 (`CArmy::IsExiled`, 140C79AC0).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatExileInputs {
    pub origin_type: i32,
    /// Template +0x1F4 when positive, else +0x1EC (140C770A0).
    pub country: i32,
    pub country_fallback: i32,
    /// The location state's core countries and the exile country's occupier
    /// set, raw tags in native order.
    pub location_cores: Vec<u32>,
    pub occupiers: Vec<u32>,
}
value_struct!(LandCombatExileInputs { origin_type: i32, country: i32, country_fallback: i32, location_cores: Vec<u32>, occupiers: Vec<u32> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandDivisionContext {
    pub template: u64,
    pub leader_modifier_kind: u8,
    pub country_modifiers: [LandDivisionModifier; 3],
    pub leader_modifiers: [LandDivisionModifier; 3],
    pub modifier_metadata: [[u32; 2]; 3],
    pub feature_flags: u32,
    pub eligibility_callback: u8,
    pub base_speed: i64,
}
value_struct!(LandDivisionContext {
    template: u64,
    leader_modifier_kind: u8,
    country_modifiers: [LandDivisionModifier; 3],
    leader_modifiers: [LandDivisionModifier; 3],
    modifier_metadata: [[u32; 2]; 3],
    feature_flags: u32,
    eligibility_callback: u8,
    base_speed: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandReinforcementModifier {
    pub scope: u8,
    pub key: u32,
    pub raw: i64,
    pub present: u8,
    pub metadata_flags: u32,
    pub feature_selector: u32,
}
value_struct!(LandReinforcementModifier {
    scope: u8,
    key: u32,
    raw: i64,
    present: u8,
    metadata_flags: u32,
    feature_selector: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandReinforcementSupplyTerm {
    pub subunit: u32,
    pub count: i32,
    pub special: u8,
}
value_struct!(LandReinforcementSupplyTerm {
    subunit: u32,
    count: i32,
    special: u8
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandReinforcementAcclimatization {
    pub weight: i64,
    pub active: u8,
}
value_struct!(LandReinforcementAcclimatization {
    weight: i64,
    active: u8
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandReinforcementTransportTerm {
    pub country: u32,
    pub regional_value: i64,
    pub own_intel: i64,
    pub enemy_intel: i64,
}
value_struct!(LandReinforcementTransportTerm {
    country: u32,
    regional_value: i64,
    own_intel: i64,
    enemy_intel: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandReinforcementInputs {
    pub scalar_values: [i64; 32],
    pub flags: [u8; 12],
    pub ids: [u32; 8],
}
value_struct!(LandReinforcementInputs {
    scalar_values: [i64; 32],
    flags: [u8; 12],
    ids: [u32; 8]
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatSummaryModifier {
    pub raw: i64,
    pub present: u8,
}
value_struct!(NavalCombatSummaryModifier {
    raw: i64,
    present: u8
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatSummaryInputs {
    pub recent_join_penalty: i64,
    pub escort_bonus: i64,
    /// The side's ordered primary country list (`+0x38`), raw references.
    pub countries: Vec<u32>,
    /// Its ordered roster-owner list (`+0x50`), which admits a viewer to the combat's
    /// shared vision (`UpdateCombatSharedVisionFlag@1413CEBE0`).
    pub secondary_countries: Vec<u32>,
    pub modifiers: [[NavalCombatSummaryModifier; 5]; 5],
    pub modifier_metadata: [[u32; 2]; 5],
    pub feature_flags: u32,
    pub eligibility_callback: u8,
}
value_struct!(NavalCombatSummaryInputs { recent_join_penalty: i64, escort_bonus: i64, countries: Vec<u32>, secondary_countries: Vec<u32>, modifiers: [[NavalCombatSummaryModifier; 5]; 5], modifier_metadata: [[u32; 2]; 5], feature_flags: u32, eligibility_callback: u8 });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatAirEfficiencyModifier {
    pub raw: i64,
    pub present: u8,
}
value_struct!(NavalCombatAirEfficiencyModifier {
    raw: i64,
    present: u8
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatAirEfficiency {
    pub source: u8,
    pub pool_type_flags: u64,
    pub mission: i32,
    pub container_present: u8,
    pub base_crowding: i64,
    pub night: i64,
    pub modifiers: [NavalCombatAirEfficiencyModifier; 3],
    /// Definition-table flags and feature selector of keys 0x22, 0x23 and
    /// the static screening sortie keys 0x24, 0x1B6, read whatever holds them.
    pub modifier_metadata: [[u32; 2]; 4],
    pub feature_flags: u32,
    pub eligibility_callback: u8,
}
value_struct!(NavalCombatAirEfficiency {
    source: u8,
    pool_type_flags: u64,
    mission: i32,
    container_present: u8,
    base_crowding: i64,
    night: i64,
    modifiers: [NavalCombatAirEfficiencyModifier; 3],
    modifier_metadata: [[u32; 2]; 4],
    feature_flags: u32,
    eligibility_callback: u8
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatAirWingKill {
    pub wing: u64,
    pub index: u32,
    pub category: u64,
    pub count: i32,
}
value_struct!(NavalCombatAirWingKill {
    wing: u64,
    index: u32,
    category: u64,
    count: i32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatPlane {
    /// CSubUnitDefinition+0x58C indices for the +0x40 wing references,
    /// aligned by occurrence. MAX means the historical wing no longer resolves.
    /// 141963460/140F4A4B0 derive the hover's localized short-name list.
    pub wing_types: Vec<u32>,
    pub source: u8,
    /// Side forward-group ordinal, or MAX for a paired-only group.
    pub group_index: u32,
    pub index: u32,
    pub carrier_slot: i32,
    /// Whether the shared CFEXAir carrier-member pointer is present.
    pub carrier_member_present: u8,
    pub carrier_state: u32,
    pub air_base: u64,
    pub wings: [Vec<u64>; 2],
    pub initial_planes: i32,
    pub current_planes: i32,
    pub casualties: i32,
    pub time_duration: i32,
    pub country: u32,
    pub date: i32,
    pub flags: [u8; 2],
    pub variant: u64,
    pub naval_hits: Vec<NavalCombatHit>,
    pub air_hits: [Vec<NavalCombatAirHit>; 2],
    pub kills: Vec<NavalCombatAirWingKill>,
    pub efficiency: NavalCombatAirEfficiency,
}
value_struct!(NavalCombatPlane { wing_types: Vec<u32>, source: u8, group_index: u32, index: u32, carrier_slot: i32, carrier_member_present: u8, carrier_state: u32, air_base: u64, wings: [Vec<u64>;2], initial_planes: i32, current_planes: i32, casualties: i32, time_duration: i32, country: u32, date: i32, flags: [u8; 2], variant: u64, naval_hits: Vec<NavalCombatHit>, air_hits: [Vec<NavalCombatAirHit>;2], kills: Vec<NavalCombatAirWingKill>, efficiency: NavalCombatAirEfficiency });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatSideDetails {
    pub damage_history: [[i64; 6]; 6],
    pub original_health: i64,
    pub total_damage: i64,
    pub damage_by_type: Vec<NavalDamageSource>,
    pub commander: u64,
    pub engaged: u8,
    pub retreating: u8,
    pub capital_screening: i64,
    pub carrier_screening: i64,
    pub positioning: i64,
    pub anti_air: i64,
    pub planes: Vec<NavalCombatPlane>,
    pub summary: NavalCombatSummaryInputs,
    pub commander_inputs: Option<NavalCommanderInputs>,
    /// The side's task-force handles (`CCombatant+0x20`) in stored order:
    /// `CNavalCombatant::AddUnit@141608EC0` admits only navies, and
    /// `CNavalCombatant::Serialize@14160FF40` saves each as `unit`.
    /// Engaged-flag derivation compares these handles hour over hour.
    pub units: Vec<u64>,
}
value_struct!(NavalCombatSideDetails { damage_history: [[i64; 6]; 6], original_health: i64, total_damage: i64, damage_by_type: Vec<NavalDamageSource>, commander: u64, engaged: u8, retreating: u8, capital_screening: i64, carrier_screening: i64, positioning: i64, anti_air: i64, planes: Vec<NavalCombatPlane>, summary: NavalCombatSummaryInputs, commander_inputs: Option<NavalCommanderInputs>, units: Vec<u64> });

/// One source-token total in the native +0x288 map's iteration order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalDamageSource {
    pub token: u32,
    pub damage: i64,
}
value_struct!(NavalDamageSource {
    token: u32,
    damage: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatDamage {
    pub source: u64,
    pub country: u32,
    pub damage: i64,
}
value_struct!(NavalCombatDamage {
    source: u64,
    country: u32,
    damage: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatAirHit {
    pub variant: u64,
    pub count: i32,
    pub country: u32,
}
value_struct!(NavalCombatAirHit {
    variant: u64,
    count: i32,
    country: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatHit {
    pub target_slot: u32,
    pub target_name: Vec<u8>,
    pub convoy: u8,
    pub damage: i64,
    pub denominator: i64,
    pub last_hit: u8,
}
value_struct!(NavalCombatHit { target_slot: u32, target_name: Vec<u8>, convoy: u8, damage: i64, denominator: i64, last_hit: u8 });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatLastTarget {
    pub present: u8,
    pub is_convoy: u8,
    pub target_group_member_count: i32,
}
value_struct!(NavalCombatLastTarget {
    present: u8,
    is_convoy: u8,
    target_group_member_count: i32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalShipModifierTerm {
    pub key: u32,
    pub raw: i64,
    pub metadata_flags: u32,
    pub feature_selector: u32,
}
value_struct!(NavalShipModifierTerm {
    key: u32,
    raw: i64,
    metadata_flags: u32,
    feature_selector: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalShipModifierSource {
    pub scope: u8,
    pub index: u32,
    pub selector: u32,
    pub reference: u64,
    pub weights: [i64; 2],
    pub movement: i64,
    pub dynamic_keys: [u32; 2],
    pub terms: Vec<NavalShipModifierTerm>,
}
value_struct!(NavalShipModifierSource { scope: u8, index: u32, selector: u32, reference: u64, weights: [i64; 2], movement: i64, dynamic_keys: [u32; 2], terms: Vec<NavalShipModifierTerm> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalShipWeaponContext {
    pub task_force: u64,
    pub country: u32,
    pub commander: u64,
    pub ship_class: u32,
    pub subunit: u32,
    pub weapon_bases: [i64; 5],
    pub weather_factor: i64,
    pub fuel: [i64; 2],
    pub night: i64,
    pub formation_flags: u64,
    pub detection_base: i64,
    pub air_base: u64,
    pub air_base_maximum: i32,
    pub air_detection_base: i64,
    pub terrain_name: Vec<u8>,
    pub sources: Vec<NavalShipModifierSource>,
}
value_struct!(NavalShipWeaponContext { task_force: u64, country: u32, commander: u64, ship_class: u32, subunit: u32, weapon_bases: [i64; 5], weather_factor: i64, fuel: [i64; 2], night: i64, formation_flags: u64, detection_base: i64, air_base: u64, air_base_maximum: i32, air_detection_base: i64, terrain_name: Vec<u8>, sources: Vec<NavalShipModifierSource> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalResultShip {
    pub member: u64,
    pub owner: u32,
    pub subunit_index: i32,
    pub formation_type: i32,
    pub build_cost_ic: i64,
    pub strength_ratio: i64,
    pub pride: u8,
    pub is_convoy: u8,
    pub name: Vec<u8>,
    pub type_name: Vec<u8>,
    pub sprite: Vec<u8>,
    pub sunk_by: Vec<u8>,
    pub variant: u64,
    pub convoy_client: u64,
    pub convoy_index: i32,
    pub naval_hits: Vec<NavalCombatHit>,
    pub air_hits: [Vec<NavalCombatAirHit>; 2],
}
value_struct!(NavalResultShip { member: u64, owner: u32, subunit_index: i32, formation_type: i32, build_cost_ic: i64, strength_ratio: i64, pride: u8, is_convoy: u8, name: Vec<u8>, type_name: Vec<u8>, sprite: Vec<u8>, sunk_by: Vec<u8>, variant: u64, convoy_client: u64, convoy_index: i32, naval_hits: Vec<NavalCombatHit>, air_hits: [Vec<NavalCombatAirHit>;2] });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalResultPlane {
    pub variant: u64,
    pub variant_summary_inputs: [i64; 2],
    pub initial_planes: i32,
    pub current_planes: i32,
    pub lost_planes: i32,
    pub country: u32,
    pub name: Vec<u8>,
    pub naval_hits: Vec<NavalCombatHit>,
    pub air_hits: [Vec<NavalCombatAirHit>; 2],
}
value_struct!(NavalResultPlane { variant: u64, variant_summary_inputs: [i64; 2], initial_planes: i32, current_planes: i32, lost_planes: i32, country: u32, name: Vec<u8>, naval_hits: Vec<NavalCombatHit>, air_hits: [Vec<NavalCombatAirHit>;2] });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalResultSideDetails {
    pub leader: u64,
    pub hq_leader: u64,
    pub hq_influences: Vec<u64>,
    pub ships: Vec<NavalResultShip>,
    pub planes: Vec<NavalResultPlane>,
    pub hq_modifiers: Vec<NavalShipModifierSource>,
    pub commander_context: Option<NavalCommanderInputs>,
    pub hq_commander_context: Option<NavalCommanderInputs>,
}
value_struct!(NavalResultSideDetails { leader: u64, hq_leader: u64, hq_influences: Vec<u64>, ships: Vec<NavalResultShip>, planes: Vec<NavalResultPlane>, hq_modifiers: Vec<NavalShipModifierSource>, commander_context: Option<NavalCommanderInputs>, hq_commander_context: Option<NavalCommanderInputs> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatDetailsRow {
    pub combat: u64,
    pub province: u32,
    pub phase: i32,
    pub width_factor: i64,
    pub attacker_bonus: i64,
    pub defender_bonus: i64,
    pub stored_terrain_valid: u8,
    pub stored_terrain_name: Vec<u8>,
    pub sides: [LandCombatSideDetails; 2],
    pub tactic_inputs: LandCombatTacticInputs,
}
typed_codec!(LandCombatDetailsRow, 64, |row| row.combat as u128,
 |key| Self { combat: key as u64, ..Default::default() },
 |key, input| Ok(Self { combat: key as u64, province: read_value(Kind::U32, input)?, phase: read_value(Kind::I32, input)?, width_factor: read_value(Kind::I64, input)?, attacker_bonus: read_value(Kind::I64, input)?, defender_bonus: read_value(Kind::I64, input)?, stored_terrain_valid: read_value(Kind::U8, input)?, stored_terrain_name: read_value(Kind::Bytes, input)?, sides: read_value(Kind::Opaque, input)?, tactic_inputs: read_value(Kind::Opaque, input)? }),
  { 0 => (province): U32, 1 => (phase): I32, 2 => (width_factor): I64, 3 => (attacker_bonus): I64, 4 => (defender_bonus): I64, 5 => (stored_terrain_valid): U8, 6 => (stored_terrain_name): Bytes, 7 => (sides): Opaque, 8 => (tactic_inputs): Opaque }, |row| { let _ = row; Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatDivisionRow {
    pub combat: u64,
    pub side: u8,
    pub ordinal: u32,
    pub role: u8,
    pub province: u32,
    pub division: u64,
    /// Naming operands (CNameGroupMember): type, order, ordered and
    /// programmatic flags, the stored +0x88 name where 1409B5640 shows it
    /// (+0xA9 clear, or set with a null group), and the selected historical
    /// group key, empty for a null group; the viewer renders the name from
    /// content.
    pub name_type: i32,
    pub name_order: i32,
    pub name_ordered: u8,
    pub name_programmatic: u8,
    pub name_custom: Vec<u8>,
    pub name_group: Vec<u8>,
    pub strength_combat_factor: i64,
    pub fuel_inputs: [i64; 2],
    pub stat_values: [i64; 5],
    pub armour: i64,
    pub piercing: i64,
    pub air_attack: i64,
    pub reconnaissance: i64,
    pub initiative: i64,
    pub commander: u64,
    pub scales: [i64; 4],
    pub context: LandDivisionContext,
    pub width_terms: Vec<LandDivisionWidthTerm>,
    pub commander_inputs: Option<LandCombatDivisionCommander>,
    pub reinforcement: LandReinforcementInputs,
    pub reinforcement_modifiers: Vec<LandReinforcementModifier>,
    pub reinforcement_supply: Vec<LandReinforcementSupplyTerm>,
    pub reinforcement_acclimatization: Vec<LandReinforcementAcclimatization>,
    pub reinforcement_transport: Vec<LandReinforcementTransportTerm>,
    /// Raw CUnit+0x4A8 latch word of the direction count (141298E00).
    pub direction_group: i32,
    pub exile: Option<LandCombatExileInputs>,
    /// The unit stat block's equipment category mask, `*(CUnit+0x138)+0x5A8`
    /// as stored: CEquipmentTypeTrigger::vf23 (141390370) counts the
    /// CLandCombatant units whose mask meets a category trigger's
    /// (140F731B0).
    pub categories: u64,
}
typed_codec!(LandCombatDivisionRow, 97, |row| (row.combat as u128) << 33 | (row.side as u128) << 32 | row.ordinal as u128,
 |key| Self { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, ordinal: key as u32, ..Default::default() },
 |key, input| Ok(Self { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, ordinal: key as u32, role: read_value(Kind::U8, input)?, province: read_value(Kind::U32, input)?, division: read_value(Kind::U64, input)?, name_type: read_value(Kind::I32, input)?, name_order: read_value(Kind::I32, input)?, name_ordered: read_value(Kind::U8, input)?, name_programmatic: read_value(Kind::U8, input)?, name_custom: read_value(Kind::Bytes, input)?, name_group: read_value(Kind::Bytes, input)?, strength_combat_factor: read_value(Kind::I64, input)?, fuel_inputs: read_value(Kind::Opaque, input)?, stat_values: read_value(Kind::Opaque, input)?, armour: read_value(Kind::I64, input)?, piercing: read_value(Kind::I64, input)?, air_attack: read_value(Kind::I64, input)?, reconnaissance: read_value(Kind::I64, input)?, initiative: read_value(Kind::I64, input)?, commander: read_value(Kind::U64, input)?, scales: read_value(Kind::Opaque, input)?, context: read_value(Kind::Opaque, input)?, width_terms: read_value(Kind::Opaque, input)?, commander_inputs: read_value(Kind::Opaque, input)?, reinforcement: read_value(Kind::Opaque, input)?, reinforcement_modifiers: read_value(Kind::Opaque, input)?, reinforcement_supply: read_value(Kind::Opaque, input)?, reinforcement_acclimatization: read_value(Kind::Opaque, input)?, reinforcement_transport: read_value(Kind::Opaque, input)?, direction_group: read_value(Kind::I32, input)?, exile: read_value(Kind::Opaque, input)?, categories: read_value(Kind::U64, input)? }),
 { 0 => (role): U8, 1 => (province): U32, 2 => (division): U64, 3 => (name_type): I32, 4 => (name_order): I32, 5 => (name_ordered): U8, 6 => (name_programmatic): U8, 7 => (name_custom): Bytes, 8 => (name_group): Bytes, 9 => (strength_combat_factor): I64, 10 => (fuel_inputs): Opaque, 11 => (stat_values): Opaque, 12 => (armour): I64, 13 => (piercing): I64, 14 => (air_attack): I64, 15 => (reconnaissance): I64, 16 => (initiative): I64, 17 => (commander): U64, 18 => (scales): Opaque, 19 => (context): Opaque, 20 => (width_terms): Opaque, 21 => (commander_inputs): Opaque, 22 => (reinforcement): Opaque, 23 => (reinforcement_modifiers): Opaque, 24 => (reinforcement_supply): Opaque, 25 => (reinforcement_acclimatization): Opaque, 26 => (reinforcement_transport): Opaque, 27 => (direction_group): I32, 28 => (exile): Opaque, 29 => (categories): U64 }, |row| { let _ = row; if row.side > 1 { return Err(invalid("invalid combat side")); } Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatModifierRow {
    pub combat: u64,
    pub side: u8,
    pub border_values: Option<[i64; 3]>,
    pub divisions: Vec<DivisionCombatModifiers>,
}
typed_codec!(LandCombatModifierRow, 65, |row| (row.combat as u128) << 1 | row.side as u128,
 |key| Self { combat: (key >> 1) as u64, side: (key & 1) as u8, ..Default::default() },
 |key, input| Ok(Self { combat: (key >> 1) as u64, side: (key & 1) as u8, border_values: read_value(Kind::Opaque, input)?, divisions: read_value(Kind::Opaque, input)? }),
 { 0 => (border_values): Opaque, 1 => (divisions): Opaque }, |row| { let _ = row; if row.side > 1 { return Err(invalid("invalid combat side")); } Ok(()) });

/// Hourly side operands of one land combat, for its map marker (1418B4BD0) and
/// window. The country sets keep raw references in native order; the viewer
/// applies the visibility and side rules for any viewing country.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatSideRow {
    pub combat: u64,
    /// CLandCombat +0x98, the marker's frame and the window's indicator icon.
    pub indicator: u8,
    /// Attacker then defender CLandCombatant +0x08 flag words.
    pub flags: [u32; 2],
    pub attacker_primary: Vec<u32>,
    pub attacker_secondary: Vec<u32>,
    pub defender_primary: Vec<u32>,
    pub defender_secondary: Vec<u32>,
}
typed_codec!(LandCombatSideRow, 64, |row| row.combat as u128,
|key| Self { combat: key as u64, ..Default::default() },
|key, input| Ok(Self { combat: key as u64, indicator: read_value(Kind::U8, input)?, flags: read_value(Kind::Opaque, input)?, attacker_primary: read_value(Kind::U32List, input)?, attacker_secondary: read_value(Kind::U32List, input)?, defender_primary: read_value(Kind::U32List, input)?, defender_secondary: read_value(Kind::U32List, input)? }),
{ 0 => (indicator): U8, 1 => (flags): Opaque, 2 => (attacker_primary): U32List, 3 => (attacker_secondary): U32List, 4 => (defender_primary): U32List, 5 => (defender_secondary): U32List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatDetailsRow {
    pub combat: u64,
    pub province: u32,
    pub day: i32,
    pub duration: i32,
    pub progress: i64,
    pub sides: [NavalCombatSideDetails; 2],
    /// SWeatherPerRegion+0x30 active i16 values, little endian, in this
    /// live combat's strategic template order (140F0CF40, 141D8A3B0).
    pub custom_sea: Option<Vec<u8>>,
    /// 0 complete; 1 guarded custom-grid read failed. No diagnostic text is
    /// serialized per combat (140F0CF40, 141D8A3B0).
    pub custom_sea_error: u8,
}
typed_codec!(NavalCombatDetailsRow, 64, |row| row.combat as u128,
 |key| Self { combat: key as u64, ..Default::default() },
 |key, input| Ok(Self { combat: key as u64, province: read_value(Kind::U32, input)?, day: read_value(Kind::I32, input)?, duration: read_value(Kind::I32, input)?, progress: read_value(Kind::I64, input)?, sides: read_value(Kind::Opaque, input)?, custom_sea: read_value(Kind::Opaque, input)?, custom_sea_error: read_value(Kind::U8, input)? }),
 { 0 => (province): U32, 1 => (day): I32, 2 => (duration): I32, 3 => (progress): I64, 4 => (sides): Opaque, 5 => (custom_sea): Opaque, 6 => (custom_sea_error): U8 }, |row| { if row.custom_sea.as_ref().is_some_and(|sea| sea.len() % 2 != 0) { return Err(invalid("odd custom sea i16 bytes")); } if row.custom_sea_error > 1 || (row.custom_sea.is_some() == (row.custom_sea_error != 0)) { return Err(invalid("custom sea capture status conflicts with values")); } Ok(()) });

/// Ordered wing-handle occurrences in the native dense CAirBase+0x48 walk for
/// a base referenced by naval combat. Repeated handles are meaningful. Every
/// resolved referenced base has a row, including one with an empty wing list.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatAirBaseWingHandlesRow {
    pub base: u64,
    pub wings: Vec<u64>,
}
typed_codec!(NavalCombatAirBaseWingHandlesRow, 64, |row| row.base as u128,
 |key| Self { base: key as u64, ..Default::default() },
 |key, input| Ok(Self { base: key as u64, wings: read_value(Kind::U64List, input)? }),
 { 0 => (wings): U64List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatShipRow {
    pub combat: u64,
    pub side: u8,
    pub battle_slot: u32,
    pub ship: u64,
    pub owner: u32,
    pub state: u32,
    pub member_source: u8,
    pub group_index: u32,
    pub member_index: u32,
    pub convoy: u64,
    pub subunit_index: i32,
    pub formation_type: i32,
    pub build_cost_ic: i64,
    pub cached_strength_ratio: i64,
    pub pride: u8,
    pub is_convoy: u8,
    pub name: Vec<u8>,
    pub type_name: Vec<u8>,
    pub sprite: Vec<u8>,
    pub sunk_by: Vec<u8>,
    pub variant: u64,
    pub arrival_hours: i32,
    pub received_criticals: i32,
    pub evacuation_state: i32,
    pub detection_state: i32,
    pub escape_progress: i64,
    pub escape_speed_base: i64,
    pub last_target: NavalCombatLastTarget,
    pub received_damage: [i64; 6],
    pub live_ship: u8,
    pub live_convoy: u8,
    pub strength: i64,
    pub organisation: i64,
    pub base_organisation: i64,
    pub maximum_hit_points: i64,
    pub formation_flags: u64,
    pub weapon_context: NavalShipWeaponContext,
    pub damage_entries: Vec<NavalCombatDamage>,
    pub air_hits: [Vec<NavalCombatAirHit>; 2],
    pub naval_hits: Vec<NavalCombatHit>,
}
typed_codec!(NavalCombatShipRow, 97, |row| (row.combat as u128) << 33 | (row.side as u128) << 32 | row.battle_slot as u128,
 |key| Self { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, battle_slot: key as u32, ..Default::default() },
 |key, input| Ok(Self { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, battle_slot: key as u32, ship: read_value(Kind::U64, input)?, owner: read_value(Kind::U32, input)?, state: read_value(Kind::U32, input)?, member_source: read_value(Kind::U8, input)?, group_index: read_value(Kind::U32, input)?, member_index: read_value(Kind::U32, input)?, convoy: read_value(Kind::U64, input)?, subunit_index: read_value(Kind::I32, input)?, formation_type: read_value(Kind::I32, input)?, build_cost_ic: read_value(Kind::I64, input)?, cached_strength_ratio: read_value(Kind::I64, input)?, pride: read_value(Kind::U8, input)?, is_convoy: read_value(Kind::U8, input)?, name: read_value(Kind::Bytes, input)?, type_name: read_value(Kind::Bytes, input)?, sprite: read_value(Kind::Bytes, input)?, sunk_by: read_value(Kind::Bytes, input)?, variant: read_value(Kind::U64, input)?, arrival_hours: read_value(Kind::I32, input)?, received_criticals: read_value(Kind::I32, input)?, evacuation_state: read_value(Kind::I32, input)?, detection_state: read_value(Kind::I32, input)?, escape_progress: read_value(Kind::I64, input)?, escape_speed_base: read_value(Kind::I64, input)?, last_target: read_value(Kind::Opaque, input)?, received_damage: read_value(Kind::Opaque, input)?, live_ship: read_value(Kind::U8, input)?, live_convoy: read_value(Kind::U8, input)?, strength: read_value(Kind::I64, input)?, organisation: read_value(Kind::I64, input)?, base_organisation: read_value(Kind::I64, input)?, maximum_hit_points: read_value(Kind::I64, input)?, formation_flags: read_value(Kind::U64, input)?, weapon_context: read_value(Kind::Opaque, input)?, damage_entries: read_value(Kind::Opaque, input)?, air_hits: read_value(Kind::Opaque, input)?, naval_hits: read_value(Kind::Opaque, input)? }),
 { 0 => (ship): U64, 1 => (owner): U32, 2 => (state): U32, 3 => (member_source): U8, 4 => (group_index): U32, 5 => (member_index): U32, 6 => (convoy): U64, 7 => (subunit_index): I32, 8 => (formation_type): I32, 9 => (build_cost_ic): I64, 10 => (cached_strength_ratio): I64, 11 => (pride): U8, 12 => (is_convoy): U8, 13 => (name): Bytes, 14 => (type_name): Bytes, 15 => (sprite): Bytes, 16 => (sunk_by): Bytes, 17 => (variant): U64, 18 => (arrival_hours): I32, 19 => (received_criticals): I32, 20 => (evacuation_state): I32, 21 => (detection_state): I32, 22 => (escape_progress): I64, 23 => (escape_speed_base): I64, 24 => (last_target): Opaque, 25 => (received_damage): Opaque, 26 => (live_ship): U8, 27 => (live_convoy): U8, 28 => (strength): I64, 29 => (organisation): I64, 30 => (base_organisation): I64, 31 => (maximum_hit_points): I64, 32 => (formation_flags): U64, 33 => (weapon_context): Opaque, 34 => (damage_entries): Opaque, 35 => (air_hits): Opaque, 36 => (naval_hits): Opaque }, |row| { let _ = row; if row.side > 1 { return Err(invalid("invalid combat side")); } Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatResultDetailsRow {
    pub result: u64,
    pub sides: [NavalResultSideDetails; 2],
}
typed_codec!(NavalCombatResultDetailsRow, 64, |row| row.result as u128,
 |key| Self { result: key as u64, ..Default::default() },
 |key, input| Ok(Self { result: key as u64, sides: read_value(Kind::Opaque, input)? }),
 { 0 => (sides): Opaque }, |row| { let _ = row; Ok(()) });

opaque_value!(LandCombatTacticInputs);
opaque_value!(LandDivisionContext);
opaque_value!(LandReinforcementInputs);
opaque_value!(NavalCombatLastTarget);
opaque_value!(NavalShipWeaponContext);
opaque_value!(Option<LandCombatDivisionCommander>);
opaque_value!(Option<[i64; 3]>);
opaque_value!(Option<LandCombatExileInputs>);
opaque_value!(Vec<DivisionCombatModifiers>);
opaque_value!(Vec<LandDivisionWidthTerm>);
opaque_value!(Vec<LandReinforcementAcclimatization>);
opaque_value!(Vec<LandReinforcementModifier>);
opaque_value!(Vec<LandReinforcementSupplyTerm>);
opaque_value!(Vec<LandReinforcementTransportTerm>);
opaque_value!(Vec<NavalCombatDamage>);
opaque_value!(Vec<NavalCombatHit>);
opaque_value!([LandCombatSideDetails; 2]);
opaque_value!([NavalCombatSideDetails; 2]);
opaque_value!([NavalResultSideDetails; 2]);
opaque_value!([Vec<NavalCombatAirHit>; 2]);
opaque_value!([i64; 2]);
opaque_value!([i64; 3]);
opaque_value!([i64; 4]);
opaque_value!([i64; 5]);
opaque_value!([i64; 6]);
opaque_value!([[i64; 6]; 6]);
opaque_value!([u32; 2]);
opaque_value!([u32; 4]);
