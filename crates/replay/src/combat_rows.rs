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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatDivisionCommander {
    pub handle: u64,
    pub skill: i32,
    pub name: Vec<u8>,
    pub traits: Vec<LandCombatCommanderTrait>,
}
value_struct!(LandCombatDivisionCommander { handle: u64, skill: i32, name: Vec<u8>, traits: Vec<LandCombatCommanderTrait> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatSideDetails {
    pub flags: u32,
    pub committed: Vec<u64>,
    pub reserves: Vec<u64>,
    pub tactic_id: i32,
    pub tactic_kind: u8,
    pub tactic_counter_id: u32,
    pub tactic_token: Vec<u8>,
    pub air_counts: Vec<i32>,
    pub anti_air_attack: i64,
    pub air_kills: u32,
    pub damage: [i64; 6],
    pub organisation_loss: [i64; 24],
    pub organisation_loss_index: u32,
    pub organisation_loss_count: u32,
    pub border_modifier: i64,
    pub border_dig_in_factor: i64,
    pub border_terrain_factor: i64,
    pub commander: u64,
    pub commander_skill: i32,
    pub commander_category: i32,
    pub commander_country: u32,
    pub commander_experience: i64,
    pub commander_name: Vec<u8>,
    pub commander_description: Vec<u8>,
    pub presentation: CombatCommanderPresentation,
    pub commander_next_skill: CombatCommanderSkillDefinition,
    pub parent_commander: u64,
    pub parent_commander_name: Vec<u8>,
    pub parent_command_capacity: i32,
    pub parent_formation_kind: u8,
    pub parent_command_count: i32,
    pub portrait_modifiers: Vec<LandCombatCommanderPortraitModifier>,
    pub portrait_tooltips: Vec<LandCombatCommanderPortraitTooltip>,
    pub commander_modifiers: [i64; 4],
    pub commander_modifier_present: [u8; 4],
    pub traits: Vec<LandCombatCommanderTrait>,
    pub learning_traits: Vec<LandCombatCommanderLearningTrait>,
    /// Resolved HQ unit handle shown in the combat window; zero is hidden.
    pub hq_unit: u64,
}
value_struct!(LandCombatSideDetails { flags: u32, committed: Vec<u64>, reserves: Vec<u64>, tactic_id: i32, tactic_kind: u8, tactic_counter_id: u32, tactic_token: Vec<u8>, air_counts: Vec<i32>, anti_air_attack: i64, air_kills: u32, damage: [i64; 6], organisation_loss: [i64; 24], organisation_loss_index: u32, organisation_loss_count: u32, border_modifier: i64, border_dig_in_factor: i64, border_terrain_factor: i64, commander: u64, commander_skill: i32, commander_category: i32, commander_country: u32, commander_experience: i64, commander_name: Vec<u8>, commander_description: Vec<u8>, presentation: CombatCommanderPresentation, commander_next_skill: CombatCommanderSkillDefinition, parent_commander: u64, parent_commander_name: Vec<u8>, parent_command_capacity: i32, parent_formation_kind: u8, parent_command_count: i32, portrait_modifiers: Vec<LandCombatCommanderPortraitModifier>, portrait_tooltips: Vec<LandCombatCommanderPortraitTooltip>, commander_modifiers: [i64; 4], commander_modifier_present: [u8; 4], traits: Vec<LandCombatCommanderTrait>, learning_traits: Vec<LandCombatCommanderLearningTrait>, hq_unit: u64 });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatTacticWeightTerm {
    pub tactic_id: i32,
    pub ordinal: u32,
    pub factor: i64,
    pub add: i64,
}
value_struct!(LandCombatTacticWeightTerm {
    tactic_id: i32,
    ordinal: u32,
    factor: i64,
    add: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatTacticUnlock {
    pub country: u32,
    pub ordinal: u32,
    pub unlock_id: u32,
}
value_struct!(LandCombatTacticUnlock {
    country: u32,
    ordinal: u32,
    unlock_id: u32
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatTacticCandidate {
    pub ordinal: u32,
    pub tactic_id: i32,
    pub unlock_id: u32,
    pub only_show_for: u32,
    pub active: u8,
    pub base_weight: i64,
    pub token: Vec<u8>,
    pub weights: Vec<LandCombatTacticWeightTerm>,
}
value_struct!(LandCombatTacticCandidate { ordinal: u32, tactic_id: i32, unlock_id: u32, only_show_for: u32, active: u8, base_weight: i64, token: Vec<u8>, weights: Vec<LandCombatTacticWeightTerm> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatTacticInputs {
    /// Attacker, defender and viewpoint country. A side's null tag 0 means its
    /// country set was empty (its last unit had left before teardown); that
    /// side's country preference, weight and unlocks are then absent.
    pub countries: [u32; 3],
    pub preferred_tactic_ids: [[i32; 3]; 2],
    pub commander_handles: [[u64; 2]; 2],
    pub preference_factors: [i64; 3],
    pub country_weights: [LandCombatTacticCountryWeight; 2],
    pub country_weight_present: [u8; 2],
    pub unlocks: [Vec<LandCombatTacticUnlock>; 3],
}
value_struct!(LandCombatTacticInputs { countries: [u32; 3], preferred_tactic_ids: [[i32; 3]; 2], commander_handles: [[u64; 2]; 2], preference_factors: [i64; 3], country_weights: [LandCombatTacticCountryWeight; 2], country_weight_present: [u8; 2], unlocks: [Vec<LandCombatTacticUnlock>;3] });

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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandDivisionManpowerTerm {
    pub source: u32,
    pub amount: i32,
}
value_struct!(LandDivisionManpowerTerm {
    source: u32,
    amount: i32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandDivisionContext {
    pub template: u64,
    pub effective_owner: u32,
    pub leader_modifier_kind: u8,
    pub country_modifiers: [LandDivisionModifier; 3],
    pub leader_modifiers: [LandDivisionModifier; 3],
    pub modifier_metadata: [[u32; 2]; 3],
    pub feature_flags: u32,
    pub eligibility_callback: u8,
    pub base_speed: i64,
    pub defines: [i64; 8],
}
value_struct!(LandDivisionContext {
    template: u64,
    effective_owner: u32,
    leader_modifier_kind: u8,
    country_modifiers: [LandDivisionModifier; 3],
    leader_modifiers: [LandDivisionModifier; 3],
    modifier_metadata: [[u32; 2]; 3],
    feature_flags: u32,
    eligibility_callback: u8,
    base_speed: i64,
    defines: [i64; 8]
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
    pub is_attacker: u8,
    pub countries: Vec<u32>,
    pub modifiers: [[NavalCombatSummaryModifier; 5]; 5],
    pub modifier_metadata: [[u32; 2]; 5],
    pub feature_flags: u32,
    pub eligibility_callback: u8,
    pub fixed_defines: [i64; 18],
    pub integer_defines: [i32; 3],
}
value_struct!(NavalCombatSummaryInputs { recent_join_penalty: i64, escort_bonus: i64, is_attacker: u8, countries: Vec<u32>, modifiers: [[NavalCombatSummaryModifier; 5]; 5], modifier_metadata: [[u32; 2]; 5], feature_flags: u32, eligibility_callback: u8, fixed_defines: [i64; 18], integer_defines: [i32; 3] });

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
    pub carrier: u64,
    pub first_wing: u64,
    pub pool_type_flags: u64,
    pub task_force: u64,
    pub mission: i32,
    pub mission_factor: i64,
    pub is_attacker: u8,
    pub country: u32,
    pub container_present: u8,
    pub base_crowding: i64,
    pub carrier_organisation: i64,
    pub carrier_base_organisation: i64,
    pub night: i64,
    pub combat_duration: i32,
    pub base_efficiency: i64,
    pub mission_duration_threshold: i32,
    pub modifiers: [NavalCombatAirEfficiencyModifier; 3],
    pub modifier_metadata: [[u32; 2]; 2],
    pub feature_flags: u32,
    pub eligibility_callback: u8,
}
value_struct!(NavalCombatAirEfficiency {
    source: u8,
    carrier: u64,
    first_wing: u64,
    pool_type_flags: u64,
    task_force: u64,
    mission: i32,
    mission_factor: i64,
    is_attacker: u8,
    country: u32,
    container_present: u8,
    base_crowding: i64,
    carrier_organisation: i64,
    carrier_base_organisation: i64,
    night: i64,
    combat_duration: i32,
    base_efficiency: i64,
    mission_duration_threshold: i32,
    modifiers: [NavalCombatAirEfficiencyModifier; 3],
    modifier_metadata: [[u32; 2]; 2],
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
    pub source: u8,
    pub group_index: u32,
    pub index: u32,
    pub carrier_slot: i32,
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
    pub name: Vec<u8>,
    pub variant: u64,
    pub cached_values: [i64; 6],
    pub naval_hits: Vec<NavalCombatHit>,
    pub air_hits: [Vec<NavalCombatAirHit>; 2],
    pub names: Vec<Vec<u8>>,
    pub kills: Vec<NavalCombatAirWingKill>,
    pub efficiency: NavalCombatAirEfficiency,
}
value_struct!(NavalCombatPlane { source: u8, group_index: u32, index: u32, carrier_slot: i32, carrier_state: u32, air_base: u64, wings: [Vec<u64>;2], initial_planes: i32, current_planes: i32, casualties: i32, time_duration: i32, country: u32, date: i32, flags: [u8; 2], name: Vec<u8>, variant: u64, cached_values: [i64; 6], naval_hits: Vec<NavalCombatHit>, air_hits: [Vec<NavalCombatAirHit>;2], names: Vec<Vec<u8>>, kills: Vec<NavalCombatAirWingKill>, efficiency: NavalCombatAirEfficiency });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatSideDetails {
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
}
value_struct!(NavalCombatSideDetails { commander: u64, engaged: u8, retreating: u8, capital_screening: i64, carrier_screening: i64, positioning: i64, anti_air: i64, planes: Vec<NavalCombatPlane>, summary: NavalCombatSummaryInputs, commander_inputs: Option<NavalCommanderInputs> });

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
    pub dynamic_keys: [u32; 2],
    pub terms: Vec<NavalShipModifierTerm>,
}
value_struct!(NavalShipModifierSource { scope: u8, index: u32, selector: u32, reference: u64, weights: [i64; 2], dynamic_keys: [u32; 2], terms: Vec<NavalShipModifierTerm> });

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
    pub indicator: u8,
    pub phase: i32,
    pub width_factor: i64,
    pub attacker_bonus: i64,
    pub defender_bonus: i64,
    pub terrain_name: Vec<u8>,
    pub stored_terrain_valid: u8,
    pub stored_terrain_name: Vec<u8>,
    pub terrain_width: i64,
    pub terrain_additional_width: i64,
    pub air_defines: [i64; 5],
    pub armour_icon_defines: [u32; 4],
    pub piercing_thresholds: Vec<i64>,
    pub piercing_damage: Vec<i64>,
    pub commander_learning_threshold: i64,
    pub parent_commander_factor: i64,
    pub portrait_feature_flags: u32,
    pub portrait_callback_kind: u8,
    pub recon_skill_impact: i32,
    pub sides: [LandCombatSideDetails; 2],
    pub tactic_inputs: LandCombatTacticInputs,
    pub tactic_candidates: Vec<LandCombatTacticCandidate>,
}
typed_codec!(LandCombatDetailsRow, 64, |row| row.combat as u128,
 |key| Self { combat: key as u64, ..Default::default() },
 |key, input| Ok(Self { combat: key as u64, province: read_value(Kind::U32, input)?, indicator: read_value(Kind::U8, input)?, phase: read_value(Kind::I32, input)?, width_factor: read_value(Kind::I64, input)?, attacker_bonus: read_value(Kind::I64, input)?, defender_bonus: read_value(Kind::I64, input)?, terrain_name: read_value(Kind::Bytes, input)?, stored_terrain_valid: read_value(Kind::U8, input)?, stored_terrain_name: read_value(Kind::Bytes, input)?, terrain_width: read_value(Kind::I64, input)?, terrain_additional_width: read_value(Kind::I64, input)?, air_defines: read_value(Kind::Opaque, input)?, armour_icon_defines: read_value(Kind::Opaque, input)?, piercing_thresholds: read_value(Kind::Opaque, input)?, piercing_damage: read_value(Kind::Opaque, input)?, commander_learning_threshold: read_value(Kind::I64, input)?, parent_commander_factor: read_value(Kind::I64, input)?, portrait_feature_flags: read_value(Kind::U32, input)?, portrait_callback_kind: read_value(Kind::U8, input)?, recon_skill_impact: read_value(Kind::I32, input)?, sides: read_value(Kind::Opaque, input)?, tactic_inputs: read_value(Kind::Opaque, input)?, tactic_candidates: read_value(Kind::Opaque, input)? }),
  { 0 => (province): U32, 1 => (indicator): U8, 2 => (phase): I32, 3 => (width_factor): I64, 4 => (attacker_bonus): I64, 5 => (defender_bonus): I64, 6 => (terrain_name): Bytes, 7 => (stored_terrain_valid): U8, 8 => (stored_terrain_name): Bytes, 9 => (terrain_width): I64, 10 => (terrain_additional_width): I64, 11 => (air_defines): Opaque, 12 => (armour_icon_defines): Opaque, 13 => (piercing_thresholds): Opaque, 14 => (piercing_damage): Opaque, 15 => (commander_learning_threshold): I64, 16 => (parent_commander_factor): I64, 17 => (portrait_feature_flags): U32, 18 => (portrait_callback_kind): U8, 19 => (recon_skill_impact): I32, 20 => (sides): Opaque, 21 => (tactic_inputs): Opaque, 22 => (tactic_candidates): Opaque }, |row| { let _ = row; Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatDivisionRow {
    pub combat: u64,
    pub side: u8,
    pub ordinal: u32,
    pub role: u8,
    pub province: u32,
    pub division: u64,
    pub name: Vec<u8>,
    pub strength: i64,
    pub strength_combat_factor: i64,
    pub fuel_inputs: [i64; 3],
    pub organisation: i64,
    pub experience: i64,
    pub entrenchment: i64,
    pub entrenchment_cap: i64,
    pub retreat: u8,
    pub stat_values: [i64; 5],
    pub max_organisation: i64,
    pub max_strength: i64,
    pub armour: i64,
    pub piercing: i64,
    pub air_attack: i64,
    pub reconnaissance: i64,
    pub initiative: i64,
    pub commander: u64,
    pub scales: [i64; 4],
    pub context: LandDivisionContext,
    pub width_terms: Vec<LandDivisionWidthTerm>,
    pub manpower: Vec<LandDivisionManpowerTerm>,
    pub commander_inputs: Option<LandCombatDivisionCommander>,
    pub reinforcement: LandReinforcementInputs,
    pub reinforcement_modifiers: Vec<LandReinforcementModifier>,
    pub reinforcement_supply: Vec<LandReinforcementSupplyTerm>,
    pub reinforcement_acclimatization: Vec<LandReinforcementAcclimatization>,
    pub reinforcement_transport: Vec<LandReinforcementTransportTerm>,
    /// Resolved province id the committed division points at.
    pub direction: u32,
    /// Raw CUnit word backing the direction arrow.
    pub direction_group: i32,
    /// Template icon gates: bit0 frame14, bit1 frame15.
    pub template_frames: u8,
}
typed_codec!(LandCombatDivisionRow, 97, |row| (row.combat as u128) << 33 | (row.side as u128) << 32 | row.ordinal as u128,
 |key| Self { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, ordinal: key as u32, ..Default::default() },
 |key, input| Ok(Self { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, ordinal: key as u32, role: read_value(Kind::U8, input)?, province: read_value(Kind::U32, input)?, division: read_value(Kind::U64, input)?, name: read_value(Kind::Bytes, input)?, strength: read_value(Kind::I64, input)?, strength_combat_factor: read_value(Kind::I64, input)?, fuel_inputs: read_value(Kind::Opaque, input)?, organisation: read_value(Kind::I64, input)?, experience: read_value(Kind::I64, input)?, entrenchment: read_value(Kind::I64, input)?, entrenchment_cap: read_value(Kind::I64, input)?, retreat: read_value(Kind::U8, input)?, stat_values: read_value(Kind::Opaque, input)?, max_organisation: read_value(Kind::I64, input)?, max_strength: read_value(Kind::I64, input)?, armour: read_value(Kind::I64, input)?, piercing: read_value(Kind::I64, input)?, air_attack: read_value(Kind::I64, input)?, reconnaissance: read_value(Kind::I64, input)?, initiative: read_value(Kind::I64, input)?, commander: read_value(Kind::U64, input)?, scales: read_value(Kind::Opaque, input)?, context: read_value(Kind::Opaque, input)?, width_terms: read_value(Kind::Opaque, input)?, manpower: read_value(Kind::Opaque, input)?, commander_inputs: read_value(Kind::Opaque, input)?, reinforcement: read_value(Kind::Opaque, input)?, reinforcement_modifiers: read_value(Kind::Opaque, input)?, reinforcement_supply: read_value(Kind::Opaque, input)?, reinforcement_acclimatization: read_value(Kind::Opaque, input)?, reinforcement_transport: read_value(Kind::Opaque, input)?, direction: read_value(Kind::U32, input)?, direction_group: read_value(Kind::I32, input)?, template_frames: read_value(Kind::U8, input)? }),
 { 0 => (role): U8, 1 => (province): U32, 2 => (division): U64, 3 => (name): Bytes, 4 => (strength): I64, 5 => (strength_combat_factor): I64, 6 => (fuel_inputs): Opaque, 7 => (organisation): I64, 8 => (experience): I64, 9 => (entrenchment): I64, 10 => (entrenchment_cap): I64, 11 => (retreat): U8, 12 => (stat_values): Opaque, 13 => (max_organisation): I64, 14 => (max_strength): I64, 15 => (armour): I64, 16 => (piercing): I64, 17 => (air_attack): I64, 18 => (reconnaissance): I64, 19 => (initiative): I64, 20 => (commander): U64, 21 => (scales): Opaque, 22 => (context): Opaque, 23 => (width_terms): Opaque, 24 => (manpower): Opaque, 25 => (commander_inputs): Opaque, 26 => (reinforcement): Opaque, 27 => (reinforcement_modifiers): Opaque, 28 => (reinforcement_supply): Opaque, 29 => (reinforcement_acclimatization): Opaque, 30 => (reinforcement_transport): Opaque, 31 => (direction): U32, 32 => (direction_group): I32, 33 => (template_frames): U8 }, |row| { let _ = row; if row.side > 1 { return Err(invalid("invalid combat side")); } Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatModifierRow {
    pub combat: u64,
    pub side: u8,
    pub flags: u32,
    pub indicator: u8,
    pub border_values: Option<[i64; 3]>,
    pub divisions: Vec<DivisionCombatModifiers>,
}
typed_codec!(LandCombatModifierRow, 65, |row| (row.combat as u128) << 1 | row.side as u128,
 |key| Self { combat: (key >> 1) as u64, side: (key & 1) as u8, ..Default::default() },
 |key, input| Ok(Self { combat: (key >> 1) as u64, side: (key & 1) as u8, flags: read_value(Kind::U32, input)?, indicator: read_value(Kind::U8, input)?, border_values: read_value(Kind::Opaque, input)?, divisions: read_value(Kind::Opaque, input)? }),
 { 0 => (flags): U32, 1 => (indicator): U8, 2 => (border_values): Opaque, 3 => (divisions): Opaque }, |row| { let _ = row; if row.side > 1 { return Err(invalid("invalid combat side")); } Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatDetailsRow {
    pub combat: u64,
    pub province: u32,
    pub day: i32,
    pub duration: i32,
    pub progress: i64,
    pub sides: [NavalCombatSideDetails; 2],
}
typed_codec!(NavalCombatDetailsRow, 64, |row| row.combat as u128,
 |key| Self { combat: key as u64, ..Default::default() },
 |key, input| Ok(Self { combat: key as u64, province: read_value(Kind::U32, input)?, day: read_value(Kind::I32, input)?, duration: read_value(Kind::I32, input)?, progress: read_value(Kind::I64, input)?, sides: read_value(Kind::Opaque, input)? }),
 { 0 => (province): U32, 1 => (day): I32, 2 => (duration): I32, 3 => (progress): I64, 4 => (sides): Opaque }, |row| { let _ = row; Ok(()) });

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
 |key, input| Ok(Self { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, battle_slot: key as u32, ship: read_value(Kind::U64, input)?, owner: read_value(Kind::U32, input)?, state: read_value(Kind::U32, input)?, member_source: read_value(Kind::U8, input)?, group_index: read_value(Kind::U32, input)?, member_index: read_value(Kind::U32, input)?, convoy: read_value(Kind::U64, input)?, subunit_index: read_value(Kind::I32, input)?, formation_type: read_value(Kind::I32, input)?, build_cost_ic: read_value(Kind::I64, input)?, cached_strength_ratio: read_value(Kind::I64, input)?, pride: read_value(Kind::U8, input)?, is_convoy: read_value(Kind::U8, input)?, name: read_value(Kind::Bytes, input)?, type_name: read_value(Kind::Bytes, input)?, sprite: read_value(Kind::Bytes, input)?, sunk_by: read_value(Kind::Bytes, input)?, variant: read_value(Kind::U64, input)?, arrival_hours: read_value(Kind::I32, input)?, received_criticals: read_value(Kind::I32, input)?, evacuation_state: read_value(Kind::I32, input)?, detection_state: read_value(Kind::I32, input)?, escape_progress: read_value(Kind::I64, input)?, last_target: read_value(Kind::Opaque, input)?, received_damage: read_value(Kind::Opaque, input)?, live_ship: read_value(Kind::U8, input)?, live_convoy: read_value(Kind::U8, input)?, strength: read_value(Kind::I64, input)?, organisation: read_value(Kind::I64, input)?, base_organisation: read_value(Kind::I64, input)?, maximum_hit_points: read_value(Kind::I64, input)?, formation_flags: read_value(Kind::U64, input)?, weapon_context: read_value(Kind::Opaque, input)?, damage_entries: read_value(Kind::Opaque, input)?, air_hits: read_value(Kind::Opaque, input)?, naval_hits: read_value(Kind::Opaque, input)? }),
 { 0 => (ship): U64, 1 => (owner): U32, 2 => (state): U32, 3 => (member_source): U8, 4 => (group_index): U32, 5 => (member_index): U32, 6 => (convoy): U64, 7 => (subunit_index): I32, 8 => (formation_type): I32, 9 => (build_cost_ic): I64, 10 => (cached_strength_ratio): I64, 11 => (pride): U8, 12 => (is_convoy): U8, 13 => (name): Bytes, 14 => (type_name): Bytes, 15 => (sprite): Bytes, 16 => (sunk_by): Bytes, 17 => (variant): U64, 18 => (arrival_hours): I32, 19 => (received_criticals): I32, 20 => (evacuation_state): I32, 21 => (detection_state): I32, 22 => (escape_progress): I64, 23 => (last_target): Opaque, 24 => (received_damage): Opaque, 25 => (live_ship): U8, 26 => (live_convoy): U8, 27 => (strength): I64, 28 => (organisation): I64, 29 => (base_organisation): I64, 30 => (maximum_hit_points): I64, 31 => (formation_flags): U64, 32 => (weapon_context): Opaque, 33 => (damage_entries): Opaque, 34 => (air_hits): Opaque, 35 => (naval_hits): Opaque }, |row| { let _ = row; if row.side > 1 { return Err(invalid("invalid combat side")); } Ok(()) });

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
opaque_value!(Vec<DivisionCombatModifiers>);
opaque_value!(Vec<LandCombatTacticCandidate>);
opaque_value!(Vec<LandDivisionManpowerTerm>);
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
opaque_value!([i64; 3]);
opaque_value!([i64; 4]);
opaque_value!([i64; 5]);
opaque_value!([i64; 6]);
opaque_value!([u32; 4]);
