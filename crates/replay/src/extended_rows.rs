//! Native state payloads. Fixed arrays and records retain independent field-mask bits.
use super::*;
use crate::value::value_struct;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProjectState {
    pub current_offset: u32,
    pub state_enums: [u32; 4],
    pub not_completed: u8,
    pub phase_progress: i64,
    pub phase_target: i32,
    pub prototype_progress: i32,
    pub iterations: i32,
}
value_struct!(ProjectState {
    current_offset: u32,
    state_enums: [u32; 4],
    not_completed: u8,
    phase_progress: i64,
    phase_target: i32,
    prototype_progress: i32,
    iterations: i32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProjectContext {
    pub province: i32,
    pub progress: i32,
    pub character_reference: u64,
    pub country_present: u8,
    pub country_reference: Option<u32>,
    pub scientist_present: u8,
    pub scientist_reference: Option<u32>,
    pub state_present: u8,
    pub state_reference: Option<u32>,
}
value_struct!(ProjectContext {province: i32, progress: i32, character_reference: u64, country_present: u8, country_reference: Option<u32>, scientist_present: u8, scientist_reference: Option<u32>, state_present: u8, state_reference: Option<u32>});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RaidSource {
    pub kind: Option<u32>,
    pub kind_present: u8,
    pub province: u32,
    pub ship_reference_type: u32,
    pub ship_reference_id: u32,
    pub country: i32,
}
value_struct!(RaidSource {kind: Option<u32>, kind_present: u8, province: u32, ship_reference_type: u32, ship_reference_id: u32, country: i32});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScriptVariableEntry {
    pub name: Vec<u8>,
    pub value_raw: i64,
}
value_struct!(ScriptVariableEntry {name: Vec<u8>, value_raw: i64});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AceDeath {
    pub killer_ace: u64,
    pub killer_country: u32,
}
value_struct!(AceDeath {
    killer_ace: u64,
    killer_country: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirHistoryEntry {
    pub side: u8,
    pub date: i32,
    pub wing_id: u32,
    pub classification: u8,
    pub region_owner: u32,
    pub planes: i32,
    pub mission_mask: u32,
    pub selection: u8,
    pub copied: u8,
    pub attributed_country: u32,
}
value_struct!(AirHistoryEntry {
    side: u8,
    date: i32,
    wing_id: u32,
    classification: u8,
    region_owner: u32,
    planes: i32,
    mission_mask: u32,
    selection: u8,
    copied: u8,
    attributed_country: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirDamageEntry {
    pub side: u8,
    pub date: i32,
    pub wing_id: u32,
    pub classification: u8,
    pub channel: u8,
    pub cause: u32,
    pub amount: i64,
}
value_struct!(AirDamageEntry {
    side: u8,
    date: i32,
    wing_id: u32,
    classification: u8,
    channel: u8,
    cause: u32,
    amount: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SunkShipEntry {
    pub ship_name: Vec<u8>,
    pub attribution_name: Vec<u8>,
    pub class_name: Vec<u8>,
    pub attribution_class_name: Vec<u8>,
    pub victim_country: u32,
    pub attacker_country: u32,
    pub sink_hour: i32,
    pub class_token: u32,
    pub attribution_class_token: u32,
    pub attribution_class_present: u8,
    pub model_level: i32,
    pub victim_equipment_variant: u64,
    pub damage_source_reference: u64,
    pub province: u32,
    pub province_present: u8,
    pub battle: u64,
    pub cause_record: u8,
    pub assisted_kill: u8,
}
value_struct!(SunkShipEntry {ship_name: Vec<u8>, attribution_name: Vec<u8>, class_name: Vec<u8>, attribution_class_name: Vec<u8>, victim_country: u32, attacker_country: u32, sink_hour: i32, class_token: u32, attribution_class_token: u32, attribution_class_present: u8, model_level: i32, victim_equipment_variant: u64, damage_source_reference: u64, province: u32, province_present: u8, battle: u64, cause_record: u8, assisted_kill: u8});

/// Focus: key country (32 bits), 6 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FocusRow {
    pub country: u32,
    pub tree: Option<Vec<u8>>,
    pub current: Option<Vec<u8>>,
    pub continuous: Option<Vec<u8>>,
    pub progress: i64,
    pub paused: u8,
    pub completed_count: i32,
}
typed_codec!(FocusRow, 32, |row| row.country as u128, |key| Self { country: key as u32, tree: Default::default(), current: Default::default(), continuous: Default::default(), progress: Default::default(), paused: Default::default(), completed_count: Default::default() },
 |key, input| Ok(Self { country: key as u32, tree: read_value(Kind::Opaque, input)?, current: read_value(Kind::Opaque, input)?, continuous: read_value(Kind::Opaque, input)?, progress: read_value(Kind::I64, input)?, paused: read_value(Kind::U8, input)?, completed_count: read_value(Kind::I32, input)? }),
 { 0 => (tree): Opaque, 1 => (current): Opaque, 2 => (continuous): Opaque, 3 => (progress): I64, 4 => (paused): U8, 5 => (completed_count): I32 }, |_row| Ok(()));

/// CompletedFocuses: key country:ordinal (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CompletedFocusRow {
    pub country: u32,
    pub ordinal: u32,
    pub token: Vec<u8>,
}
typed_codec!(CompletedFocusRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, token: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, ordinal: key as u32, token: read_value(Kind::Bytes, input)? }),
 { 0 => (token): Bytes }, |_row| Ok(()));

/// ResearchTechnologies: key country:technology_handle (64 bits), 5 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResearchTechnologyRow {
    pub country: u32,
    pub technology_handle: u32,
    pub classifier: u32,
    pub level: i32,
    pub researched: u8,
    pub progress_raw: i64,
    pub ahead_reduction_years_raw: i64,
}
typed_codec!(ResearchTechnologyRow, 64, |row| (row.country as u128) << 32 | row.technology_handle as u128, |key| Self { country: (key >> 32) as u32, technology_handle: key as u32, classifier: Default::default(), level: Default::default(), researched: Default::default(), progress_raw: Default::default(), ahead_reduction_years_raw: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, technology_handle: key as u32, classifier: read_value(Kind::U32, input)?, level: read_value(Kind::I32, input)?, researched: read_value(Kind::U8, input)?, progress_raw: read_value(Kind::I64, input)?, ahead_reduction_years_raw: read_value(Kind::I64, input)? }),
 { 0 => (classifier): U32, 1 => (level): I32, 2 => (researched): U8, 3 => (progress_raw): I64, 4 => (ahead_reduction_years_raw): I64 }, |_row| Ok(()));

/// ResearchSlots: key country:index (64 bits), 5 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResearchSlotRow {
    pub country: u32,
    pub index: u32,
    pub technology_handle: u32,
    pub technology_present: u8,
    pub research_speed_raw: i64,
    pub banked_days_raw: i64,
    pub applied_days_raw: i64,
}
typed_codec!(ResearchSlotRow, 64, |row| (row.country as u128) << 32 | row.index as u128, |key| Self { country: (key >> 32) as u32, index: key as u32, technology_handle: Default::default(), technology_present: Default::default(), research_speed_raw: Default::default(), banked_days_raw: Default::default(), applied_days_raw: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, index: key as u32, technology_handle: read_value(Kind::U32, input)?, technology_present: read_value(Kind::U8, input)?, research_speed_raw: read_value(Kind::I64, input)?, banked_days_raw: read_value(Kind::I64, input)?, applied_days_raw: read_value(Kind::I64, input)? }),
 { 0 => (technology_handle): U32, 1 => (technology_present): U8, 2 => (research_speed_raw): I64, 3 => (banked_days_raw): I64, 4 => (applied_days_raw): I64 }, |_row| Ok(()));

/// TechnologyDefinitions: key technology_handle (32 bits), 5 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TechnologyDefinitionRow {
    pub technology_handle: u32,
    pub database_index: i32,
    pub year: i32,
    pub max_level: i32,
    pub research_cost_raw: i64,
    pub id: Vec<u8>,
}
typed_codec!(TechnologyDefinitionRow, 32, |row| row.technology_handle as u128, |key| Self { technology_handle: key as u32, database_index: Default::default(), year: Default::default(), max_level: Default::default(), research_cost_raw: Default::default(), id: Default::default() },
 |key, input| Ok(Self { technology_handle: key as u32, database_index: read_value(Kind::I32, input)?, year: read_value(Kind::I32, input)?, max_level: read_value(Kind::I32, input)?, research_cost_raw: read_value(Kind::I64, input)?, id: read_value(Kind::Bytes, input)? }),
 { 0 => (database_index): I32, 1 => (year): I32, 2 => (max_level): I32, 3 => (research_cost_raw): I64, 4 => (id): Bytes }, |_row| Ok(()));

/// Politics: key country (32 bits), 5 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PoliticsRow {
    pub country: u32,
    pub last_election: i32,
    pub election_frequency: i32,
    pub elections_allowed: u8,
    pub capital: i32,
    pub ruling_party: Option<u32>,
}
typed_codec!(PoliticsRow, 32, |row| row.country as u128, |key| Self { country: key as u32, last_election: Default::default(), election_frequency: Default::default(), elections_allowed: Default::default(), capital: Default::default(), ruling_party: Default::default() },
 |key, input| Ok(Self { country: key as u32, last_election: read_value(Kind::I32, input)?, election_frequency: read_value(Kind::I32, input)?, elections_allowed: read_value(Kind::U8, input)?, capital: read_value(Kind::I32, input)?, ruling_party: read_value(Kind::OptionalU32, input)? }),
 { 0 => (last_election): I32, 1 => (election_frequency): I32, 2 => (elections_allowed): U8, 3 => (capital): I32, 4 => (ruling_party): OptionalU32 }, |_row| Ok(()));

/// PoliticalParties: key country:id (64 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PoliticalPartyRow {
    pub country: u32,
    pub id: u32,
    pub popularity: i64,
    pub is_ruling: u8,
    pub token: Vec<u8>,
}
typed_codec!(PoliticalPartyRow, 64, |row| (row.country as u128) << 32 | row.id as u128, |key| Self { country: (key >> 32) as u32, id: key as u32, popularity: Default::default(), is_ruling: Default::default(), token: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, id: key as u32, popularity: read_value(Kind::I64, input)?, is_ruling: read_value(Kind::U8, input)?, token: read_value(Kind::Bytes, input)? }),
 { 0 => (popularity): I64, 1 => (is_ruling): U8, 2 => (token): Bytes }, |_row| Ok(()));

/// ActiveIdeas: key country:ordinal (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ActiveIdeaRow {
    pub country: u32,
    pub ordinal: u32,
    pub token: Vec<u8>,
}
typed_codec!(ActiveIdeaRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, token: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, ordinal: key as u32, token: read_value(Kind::Bytes, input)? }),
 { 0 => (token): Bytes }, |_row| Ok(()));

/// DecisionCounts: key country (32 bits), 6 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DecisionCountsRow {
    pub country: u32,
    pub taken: u32,
    pub re_enable: u32,
    pub remove: u32,
    pub timed: u32,
    pub targeted_timed: u32,
    pub targeted: u32,
}
typed_codec!(DecisionCountsRow, 32, |row| row.country as u128, |key| Self { country: key as u32, taken: Default::default(), re_enable: Default::default(), remove: Default::default(), timed: Default::default(), targeted_timed: Default::default(), targeted: Default::default() },
 |key, input| Ok(Self { country: key as u32, taken: read_value(Kind::U32, input)?, re_enable: read_value(Kind::U32, input)?, remove: read_value(Kind::U32, input)?, timed: read_value(Kind::U32, input)?, targeted_timed: read_value(Kind::U32, input)?, targeted: read_value(Kind::U32, input)? }),
 { 0 => (taken): U32, 1 => (re_enable): U32, 2 => (remove): U32, 3 => (timed): U32, 4 => (targeted_timed): U32, 5 => (targeted): U32 }, |_row| Ok(()));

/// Decisions: key country:container:ordinal (72 bits), 7 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DecisionRow {
    pub country: u32,
    pub container: u8,
    pub ordinal: u32,
    pub token: Vec<u8>,
    pub days: i32,
    pub lifecycle: i32,
    pub target_kind: u8,
    pub country_target: u32,
    pub numeric_target: u32,
    pub ignore: u8,
}
typed_codec!(DecisionRow, 72, |row| (row.country as u128) << 40 | (row.container as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 40) as u32, container: (key >> 32) as u8, ordinal: key as u32, token: Default::default(), days: Default::default(), lifecycle: Default::default(), target_kind: Default::default(), country_target: Default::default(), numeric_target: Default::default(), ignore: Default::default() },
 |key, input| Ok(Self { country: (key >> 40) as u32, container: (key >> 32) as u8, ordinal: key as u32, token: read_value(Kind::Bytes, input)?, days: read_value(Kind::I32, input)?, lifecycle: read_value(Kind::I32, input)?, target_kind: read_value(Kind::U8, input)?, country_target: read_value(Kind::U32, input)?, numeric_target: read_value(Kind::U32, input)?, ignore: read_value(Kind::U8, input)? }),
 { 0 => (token): Bytes, 1 => (days): I32, 2 => (lifecycle): I32, 3 => (target_kind): U8, 4 => (country_target): U32, 5 => (numeric_target): U32, 6 => (ignore): U8 }, |_row| Ok(()));

/// DynamicModifiers: key country:ordinal (64 bits), 6 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DynamicModifierRow {
    pub country: u32,
    pub ordinal: u32,
    pub scope_country: u32,
    pub scope_state: i32,
    pub days_remaining: i32,
    pub enabled: u8,
    pub token: Vec<u8>,
    pub values: Vec<i64>,
}
typed_codec!(DynamicModifierRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, scope_country: Default::default(), scope_state: Default::default(), days_remaining: Default::default(), enabled: Default::default(), token: Default::default(), values: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, ordinal: key as u32, scope_country: read_value(Kind::U32, input)?, scope_state: read_value(Kind::I32, input)?, days_remaining: read_value(Kind::I32, input)?, enabled: read_value(Kind::U8, input)?, token: read_value(Kind::Bytes, input)?, values: read_value(Kind::Opaque, input)? }),
 { 0 => (scope_country): U32, 1 => (scope_state): I32, 2 => (days_remaining): I32, 3 => (enabled): U8, 4 => (token): Bytes, 5 => (values): Opaque }, |_row| Ok(()));

/// PreferredTactics: key country (32 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PreferredTacticRow {
    pub country: u32,
    pub tactic_id: i32,
    pub token: Vec<u8>,
}
typed_codec!(PreferredTacticRow, 32, |row| row.country as u128, |key| Self { country: key as u32, tactic_id: Default::default(), token: Default::default() },
 |key, input| Ok(Self { country: key as u32, tactic_id: read_value(Kind::I32, input)?, token: read_value(Kind::Bytes, input)? }),
 { 0 => (tactic_id): I32, 1 => (token): Bytes }, |_row| Ok(()));

/// PreferredTacticOperands: key country (32 bits), 7 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PreferredTacticOperandsRow {
    pub country: u32,
    pub tactic_id: i32,
    pub modifier_key: u32,
    pub metadata_flags: u32,
    pub metadata_gate: i32,
    pub country_factor_raw: i64,
    pub accumulator_raw: i64,
    pub accumulator_entry_present: u8,
}
typed_codec!(PreferredTacticOperandsRow, 32, |row| row.country as u128, |key| Self { country: key as u32, tactic_id: Default::default(), modifier_key: Default::default(), metadata_flags: Default::default(), metadata_gate: Default::default(), country_factor_raw: Default::default(), accumulator_raw: Default::default(), accumulator_entry_present: Default::default() },
 |key, input| Ok(Self { country: key as u32, tactic_id: read_value(Kind::I32, input)?, modifier_key: read_value(Kind::U32, input)?, metadata_flags: read_value(Kind::U32, input)?, metadata_gate: read_value(Kind::I32, input)?, country_factor_raw: read_value(Kind::I64, input)?, accumulator_raw: read_value(Kind::I64, input)?, accumulator_entry_present: read_value(Kind::U8, input)? }),
 { 0 => (tactic_id): I32, 1 => (modifier_key): U32, 2 => (metadata_flags): U32, 3 => (metadata_gate): I32, 4 => (country_factor_raw): I64, 5 => (accumulator_raw): I64, 6 => (accumulator_entry_present): U8 }, |_row| Ok(()));

/// TechSharingGroups: key id (32 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TechSharingGroupRow {
    pub id: u32,
    pub base_bonus: i64,
    pub member_count: u32,
}
typed_codec!(TechSharingGroupRow, 32, |row| row.id as u128, |key| Self { id: key as u32, base_bonus: Default::default(), member_count: Default::default() },
 |key, input| Ok(Self { id: key as u32, base_bonus: read_value(Kind::I64, input)?, member_count: read_value(Kind::U32, input)? }),
 { 0 => (base_bonus): I64, 1 => (member_count): U32 }, |_row| Ok(()));

/// TechSharingMembers: key group:ordinal (64 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TechSharingMemberRow {
    pub group: u32,
    pub ordinal: u32,
    pub country: u32,
    pub bonus: i64,
}
typed_codec!(TechSharingMemberRow, 64, |row| (row.group as u128) << 32 | row.ordinal as u128, |key| Self { group: (key >> 32) as u32, ordinal: key as u32, country: Default::default(), bonus: Default::default() },
 |key, input| Ok(Self { group: (key >> 32) as u32, ordinal: key as u32, country: read_value(Kind::U32, input)?, bonus: read_value(Kind::I64, input)? }),
 { 0 => (country): U32, 1 => (bonus): I64 }, |_row| Ok(()));

/// PowerBalances: key ordinal (32 bits), 6 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PowerBalanceRow {
    pub ordinal: u32,
    pub value_raw: i64,
    pub balance: Vec<u8>,
    pub left: Vec<u8>,
    pub right: Vec<u8>,
    pub trending: Vec<u8>,
    pub active_range: Vec<u8>,
}
typed_codec!(PowerBalanceRow, 32, |row| row.ordinal as u128, |key| Self { ordinal: key as u32, value_raw: Default::default(), balance: Default::default(), left: Default::default(), right: Default::default(), trending: Default::default(), active_range: Default::default() },
 |key, input| Ok(Self { ordinal: key as u32, value_raw: read_value(Kind::I64, input)?, balance: read_value(Kind::Bytes, input)?, left: read_value(Kind::Bytes, input)?, right: read_value(Kind::Bytes, input)?, trending: read_value(Kind::Bytes, input)?, active_range: read_value(Kind::Bytes, input)? }),
 { 0 => (value_raw): I64, 1 => (balance): Bytes, 2 => (left): Bytes, 3 => (right): Bytes, 4 => (trending): Bytes, 5 => (active_range): Bytes }, |_row| Ok(()));

/// PowerBalanceMembers: key record:ordinal (64 bits), 7 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PowerBalanceMemberRow {
    pub record: u32,
    pub ordinal: u32,
    pub country: u32,
    pub daily_raw: i64,
    pub weekly_raw: i64,
    pub daily_present: bool,
    pub weekly_present: bool,
    pub daily_flags: u32,
    pub weekly_flags: u32,
}
typed_codec!(PowerBalanceMemberRow, 64, |row| (row.record as u128) << 32 | row.ordinal as u128, |key| Self { record: (key >> 32) as u32, ordinal: key as u32, country: Default::default(), daily_raw: Default::default(), weekly_raw: Default::default(), daily_present: Default::default(), weekly_present: Default::default(), daily_flags: Default::default(), weekly_flags: Default::default() },
 |key, input| Ok(Self { record: (key >> 32) as u32, ordinal: key as u32, country: read_value(Kind::U32, input)?, daily_raw: read_value(Kind::I64, input)?, weekly_raw: read_value(Kind::I64, input)?, daily_present: read_value(Kind::Bool, input)?, weekly_present: read_value(Kind::Bool, input)?, daily_flags: read_value(Kind::U32, input)?, weekly_flags: read_value(Kind::U32, input)? }),
 { 0 => (country): U32, 1 => (daily_raw): I64, 2 => (weekly_raw): I64, 3 => (daily_present): Bool, 4 => (weekly_present): Bool, 5 => (daily_flags): U32, 6 => (weekly_flags): U32 }, |_row| Ok(()));

/// PowerBalanceRanges: key record:ordinal (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PowerBalanceRangeRow {
    pub record: u32,
    pub ordinal: u32,
    pub identity: Vec<u8>,
}
typed_codec!(PowerBalanceRangeRow, 64, |row| (row.record as u128) << 32 | row.ordinal as u128, |key| Self { record: (key >> 32) as u32, ordinal: key as u32, identity: Default::default() },
 |key, input| Ok(Self { record: (key >> 32) as u32, ordinal: key as u32, identity: read_value(Kind::Bytes, input)? }),
 { 0 => (identity): Bytes }, |_row| Ok(()));

/// PowerBalanceOverrides: key record:ordinal (64 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PowerBalanceOverrideRow {
    pub record: u32,
    pub ordinal: u32,
    pub side: Vec<u8>,
    pub gfx: Vec<u8>,
}
typed_codec!(PowerBalanceOverrideRow, 64, |row| (row.record as u128) << 32 | row.ordinal as u128, |key| Self { record: (key >> 32) as u32, ordinal: key as u32, side: Default::default(), gfx: Default::default() },
 |key, input| Ok(Self { record: (key >> 32) as u32, ordinal: key as u32, side: read_value(Kind::Bytes, input)?, gfx: read_value(Kind::Bytes, input)? }),
 { 0 => (side): Bytes, 1 => (gfx): Bytes }, |_row| Ok(()));

/// ProductionLines: key handle (64 bits), 11 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
    pub produced_raw: i64,
    pub priority: i32,
    pub amount: i32,
}
typed_codec!(ProductionLineRow, 64, |row| row.handle as u128, |key| Self { handle: key as u64, country: Default::default(), kind: Default::default(), variant: Default::default(), mio: Default::default(), active_factories: Default::default(), requested_factories: Default::default(), cost_raw: Default::default(), speed_raw: Default::default(), produced_raw: Default::default(), priority: Default::default(), amount: Default::default() },
 |key, input| Ok(Self { handle: key as u64, country: read_value(Kind::U32, input)?, kind: read_value(Kind::U8, input)?, variant: read_value(Kind::U64, input)?, mio: read_value(Kind::U64, input)?, active_factories: read_value(Kind::I32, input)?, requested_factories: read_value(Kind::I32, input)?, cost_raw: read_value(Kind::I64, input)?, speed_raw: read_value(Kind::I64, input)?, produced_raw: read_value(Kind::I64, input)?, priority: read_value(Kind::I32, input)?, amount: read_value(Kind::I32, input)? }),
 { 0 => (country): U32, 1 => (kind): U8, 2 => (variant): U64, 3 => (mio): U64, 4 => (active_factories): I32, 5 => (requested_factories): I32, 6 => (cost_raw): I64, 7 => (speed_raw): I64, 8 => (produced_raw): I64, 9 => (priority): I32, 10 => (amount): I32 }, |_row| Ok(()));

/// Stockpiles: key country:ordinal (64 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StockpileRow {
    pub country: u32,
    pub ordinal: u32,
    pub variant_handle: u64,
    pub amount_raw: i64,
}
typed_codec!(StockpileRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, variant_handle: Default::default(), amount_raw: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, ordinal: key as u32, variant_handle: read_value(Kind::U64, input)?, amount_raw: read_value(Kind::I64, input)? }),
 { 0 => (variant_handle): U64, 1 => (amount_raw): I64 }, |_row| Ok(()));

/// DivisionEquipment: key division:ordinal (96 bits), 4 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionEquipmentRow {
    pub division: u64,
    pub template: u64,
    pub country: u32,
    pub ordinal: u32,
    pub variant_handle: u64,
    pub amount_raw: i64,
}
typed_codec!(DivisionEquipmentRow, 96, |row| (row.division as u128) << 32 | row.ordinal as u128, |key| Self { division: (key >> 32) as u64, ordinal: key as u32, template: Default::default(), country: Default::default(), variant_handle: Default::default(), amount_raw: Default::default() },
 |key, input| Ok(Self { division: (key >> 32) as u64, ordinal: key as u32, template: read_value(Kind::U64, input)?, country: read_value(Kind::U32, input)?, variant_handle: read_value(Kind::U64, input)?, amount_raw: read_value(Kind::I64, input)? }),
 { 0 => (template): U64, 1 => (country): U32, 2 => (variant_handle): U64, 3 => (amount_raw): I64 }, |_row| Ok(()));

/// VariantDefinitions: key id (64 bits), 10 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
    pub equipment_name: Vec<u8>,
}
typed_codec!(VariantDefinitionRow, 64, |row| row.id as u128, |key| Self { id: key as u64, equipment_token: Default::default(), creator_link: Default::default(), origin_link: Default::default(), append_variant_name: Default::default(), obsolete: Default::default(), classifier_flags: Default::default(), mission_mask: Default::default(), display_name: Default::default(), variant_name: Default::default(), equipment_name: Default::default() },
 |key, input| Ok(Self { id: key as u64, equipment_token: read_value(Kind::U32, input)?, creator_link: read_value(Kind::U32, input)?, origin_link: read_value(Kind::U32, input)?, append_variant_name: read_value(Kind::U8, input)?, obsolete: read_value(Kind::U8, input)?, classifier_flags: read_value(Kind::U64, input)?, mission_mask: read_value(Kind::U32, input)?, display_name: read_value(Kind::Bytes, input)?, variant_name: read_value(Kind::Bytes, input)?, equipment_name: read_value(Kind::Bytes, input)? }),
 { 0 => (equipment_token): U32, 1 => (creator_link): U32, 2 => (origin_link): U32, 3 => (append_variant_name): U8, 4 => (obsolete): U8, 5 => (classifier_flags): U64, 6 => (mission_mask): U32, 7 => (display_name): Bytes, 8 => (variant_name): Bytes, 9 => (equipment_name): Bytes }, |_row| Ok(()));

/// VariantModules: key variant:source_ordinal (96 bits), 4 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VariantModuleRow {
    pub variant: u64,
    pub source_ordinal: u32,
    pub slot: u32,
    pub definition_token: u32,
    pub definition_active: u8,
    pub definition_name: Vec<u8>,
}
typed_codec!(VariantModuleRow, 96, |row| (row.variant as u128) << 32 | row.source_ordinal as u128, |key| Self { variant: (key >> 32) as u64, source_ordinal: key as u32, slot: Default::default(), definition_token: Default::default(), definition_active: Default::default(), definition_name: Default::default() },
 |key, input| Ok(Self { variant: (key >> 32) as u64, source_ordinal: key as u32, slot: read_value(Kind::U32, input)?, definition_token: read_value(Kind::U32, input)?, definition_active: read_value(Kind::U8, input)?, definition_name: read_value(Kind::Bytes, input)? }),
 { 0 => (slot): U32, 1 => (definition_token): U32, 2 => (definition_active): U8, 3 => (definition_name): Bytes }, |_row| Ok(()));

/// VariantStats: key variant (64 bits), 10 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VariantStatsRow {
    pub variant: u64,
    pub values: [i64; 10],
}
typed_codec!(VariantStatsRow, 64, |row| row.variant as u128, |key| Self { variant: key as u64, values: Default::default() },
 |key, input| Ok(Self { variant: key as u64, values: <[i64;10] as Value>::decode(input)? }),
 { 0 => (values[0]): I64, 1 => (values[1]): I64, 2 => (values[2]): I64, 3 => (values[3]): I64, 4 => (values[4]): I64, 5 => (values[5]): I64, 6 => (values[6]): I64, 7 => (values[7]): I64, 8 => (values[8]): I64, 9 => (values[9]): I64 }, |_row| Ok(()));

/// VariantUpgrades: key variant:source_ordinal (96 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VariantUpgradeRow {
    pub variant: u64,
    pub source_ordinal: u32,
    pub definition_token: i32,
    pub level: u8,
    pub definition_name: Vec<u8>,
}
typed_codec!(VariantUpgradeRow, 96, |row| (row.variant as u128) << 32 | row.source_ordinal as u128, |key| Self { variant: (key >> 32) as u64, source_ordinal: key as u32, definition_token: Default::default(), level: Default::default(), definition_name: Default::default() },
 |key, input| Ok(Self { variant: (key >> 32) as u64, source_ordinal: key as u32, definition_token: read_value(Kind::I32, input)?, level: read_value(Kind::U8, input)?, definition_name: read_value(Kind::Bytes, input)? }),
 { 0 => (definition_token): I32, 1 => (level): U8, 2 => (definition_name): Bytes }, |_row| Ok(()));

/// ArmyUpgradeGroups: key army (64 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ArmyUpgradeGroupRow {
    pub country: u32,
    pub army: u64,
    pub priority: u32,
}
typed_codec!(ArmyUpgradeGroupRow, 64, |row| row.army as u128, |key| Self { army: key as u64, country: Default::default(), priority: Default::default() },
 |key, input| Ok(Self { army: key as u64, country: read_value(Kind::U32, input)?, priority: read_value(Kind::U32, input)? }),
 { 0 => (country): U32, 1 => (priority): U32 }, |_row| Ok(()));

/// ArmyUpgradeRequests: key army:ordinal (96 bits), 4 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ArmyUpgradeRequestRow {
    pub country: u32,
    pub army: u64,
    pub ordinal: u32,
    pub equipment: u64,
    pub request_raw: i64,
    pub total_raw: i64,
}
typed_codec!(ArmyUpgradeRequestRow, 96, |row| (row.army as u128) << 32 | row.ordinal as u128, |key| Self { army: (key >> 32) as u64, ordinal: key as u32, country: Default::default(), equipment: Default::default(), request_raw: Default::default(), total_raw: Default::default() },
 |key, input| Ok(Self { army: (key >> 32) as u64, ordinal: key as u32, country: read_value(Kind::U32, input)?, equipment: read_value(Kind::U64, input)?, request_raw: read_value(Kind::I64, input)?, total_raw: read_value(Kind::I64, input)? }),
 { 0 => (country): U32, 1 => (equipment): U64, 2 => (request_raw): I64, 3 => (total_raw): I64 }, |_row| Ok(()));

/// ArmyUpgradeDeliveries: key army:ordinal (96 bits), 5 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
 |key, input| Ok(Self { army: (key >> 32) as u64, ordinal: key as u32, country: read_value(Kind::U32, input)?, status: read_value(Kind::U32, input)?, progress_raw: read_value(Kind::OptionalI64, input)?, total_progress_raw: read_value(Kind::I64, input)?, allow_zero_entries: read_value(Kind::U8, input)? }),
 { 0 => (country): U32, 1 => (status): U32, 2 => (progress_raw): OptionalI64, 3 => (total_progress_raw): I64, 4 => (allow_zero_entries): U8 }, |_row| Ok(()));

/// ArmyUpgradeProduced: key army:delivery_ordinal:ordinal (128 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ArmyUpgradeProducedRow {
    pub country: u32,
    pub army: u64,
    pub delivery_ordinal: u32,
    pub ordinal: u32,
    pub equipment: u64,
    pub amount_raw: i64,
}
typed_codec!(ArmyUpgradeProducedRow, 128, |row| (row.army as u128) << 64 | (row.delivery_ordinal as u128) << 32 | row.ordinal as u128, |key| Self { army: (key >> 64) as u64, delivery_ordinal: (key >> 32) as u32, ordinal: key as u32, country: Default::default(), equipment: Default::default(), amount_raw: Default::default() },
 |key, input| Ok(Self { army: (key >> 64) as u64, delivery_ordinal: (key >> 32) as u32, ordinal: key as u32, country: read_value(Kind::U32, input)?, equipment: read_value(Kind::U64, input)?, amount_raw: read_value(Kind::I64, input)? }),
 { 0 => (country): U32, 1 => (equipment): U64, 2 => (amount_raw): I64 }, |_row| Ok(()));

/// CountryFuel: key country (32 bits), 4 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryFuelRow {
    pub country: u32,
    pub current_raw: i64,
    pub maximum_raw: i64,
    pub gain_raw: i64,
    pub cost_raw: i64,
}
typed_codec!(CountryFuelRow, 32, |row| row.country as u128, |key| Self { country: key as u32, current_raw: Default::default(), maximum_raw: Default::default(), gain_raw: Default::default(), cost_raw: Default::default() },
 |key, input| Ok(Self { country: key as u32, current_raw: read_value(Kind::I64, input)?, maximum_raw: read_value(Kind::I64, input)?, gain_raw: read_value(Kind::I64, input)?, cost_raw: read_value(Kind::I64, input)? }),
 { 0 => (current_raw): I64, 1 => (maximum_raw): I64, 2 => (gain_raw): I64, 3 => (cost_raw): I64 }, |_row| Ok(()));

/// ResourceDefinitions: key id (32 bits), 4 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResourceDefinitionRow {
    pub id: u32,
    pub token: u32,
    pub active: u8,
    pub factory_cost: i64,
}
typed_codec!(ResourceDefinitionRow, 32, |row| row.id as u128, |key| Self { id: key as u32, token: Default::default(), active: Default::default(), factory_cost: Default::default() },
 |key, input| Ok(Self { id: key as u32, token: read_value(Kind::U32, input)?, active: read_value(Kind::U8, input)?, factory_cost: read_value(Kind::I64, input)? }),
 { 0 => (token): U32, 1 => (active): U8, 2 => (factory_cost): I64 }, |_row| Ok(()));

/// CountryResources: key country:resource (64 bits), 9 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
 |key, input| Ok(Self { country: (key >> 32) as u32, resource: key as u32, produced: read_value(Kind::I64, input)?, adjustment: read_value(Kind::I64, input)?, imported: read_value(Kind::I64, input)?, used: read_value(Kind::I64, input)?, special_total: read_value(Kind::I64, input)?, production_demand: read_value(Kind::I64, input)?, to_export: read_value(Kind::I64, input)?, base_export: read_value(Kind::I64, input)?, exported: read_value(Kind::I64, input)? }),
 { 0 => (produced): I64, 1 => (adjustment): I64, 2 => (imported): I64, 3 => (used): I64, 4 => (special_total): I64, 5 => (production_demand): I64, 6 => (to_export): I64, 7 => (base_export): I64, 8 => (exported): I64 }, |_row| Ok(()));

/// ResourceExchanges: key id (64 bits), 14 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
 |key, input| Ok(Self { id: key as u64, exporter: read_value(Kind::U32, input)?, importer: read_value(Kind::U32, input)?, resource: read_value(Kind::U32, input)?, import_ordinal: read_value(Kind::U32, input)?, export_ordinal: read_value(Kind::U32, input)?, convoys_assigned: read_value(Kind::I32, input)?, convoys_required: read_value(Kind::I32, input)?, efficiency_a: read_value(Kind::I64, input)?, efficiency_b: read_value(Kind::I64, input)?, delivered: read_value(Kind::I64, input)?, required_factories: read_value(Kind::I32, input)?, assigned_factories: read_value(Kind::I32, input)?, no_trade_influence: read_value(Kind::U8, input)?, no_resources: read_value(Kind::U8, input)? }),
 { 0 => (exporter): U32, 1 => (importer): U32, 2 => (resource): U32, 3 => (import_ordinal): U32, 4 => (export_ordinal): U32, 5 => (convoys_assigned): I32, 6 => (convoys_required): I32, 7 => (efficiency_a): I64, 8 => (efficiency_b): I64, 9 => (delivered): I64, 10 => (required_factories): I32, 11 => (assigned_factories): I32, 12 => (no_trade_influence): U8, 13 => (no_resources): U8 }, |_row| Ok(()));

/// PurchaseContracts: key id (64 bits), 11 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
 |key, input| Ok(Self { id: key as u64, seller: read_value(Kind::U32, input)?, buyer: read_value(Kind::U32, input)?, speed: read_value(Kind::I32, input)?, convoys_assigned: read_value(Kind::I32, input)?, convoys_total: read_value(Kind::I32, input)?, route_efficiency_sum_raw: read_value(Kind::I64, input)?, route_days: read_value(Kind::I32, input)?, factory_cic_progress_raw: read_value(Kind::I64, input)?, equipment_transfer_progress_raw: read_value(Kind::I64, input)?, collected_raw: read_value(Kind::I64, input)?, completed_deliveries: read_value(Kind::I32, input)? }),
 { 0 => (seller): U32, 1 => (buyer): U32, 2 => (speed): I32, 3 => (convoys_assigned): I32, 4 => (convoys_total): I32, 5 => (route_efficiency_sum_raw): I64, 6 => (route_days): I32, 7 => (factory_cic_progress_raw): I64, 8 => (equipment_transfer_progress_raw): I64, 9 => (collected_raw): I64, 10 => (completed_deliveries): I32 }, |_row| Ok(()));

/// ProvinceSupply: key province (32 bits), 9 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProvinceSupplyRow {
    pub province: u32,
    pub supply_factor_raw: i64,
    pub railway_health_raw: i64,
    pub controller_index: u16,
    pub capital_country_index: u16,
    pub supply_node_level: u8,
    pub naval_base_level: u8,
    pub non_damaged_infra_level: u8,
    pub infra_level: u8,
    pub flags: u8,
}
typed_codec!(ProvinceSupplyRow, 32, |row| row.province as u128, |key| Self { province: key as u32, supply_factor_raw: Default::default(), railway_health_raw: Default::default(), controller_index: Default::default(), capital_country_index: Default::default(), supply_node_level: Default::default(), naval_base_level: Default::default(), non_damaged_infra_level: Default::default(), infra_level: Default::default(), flags: Default::default() },
 |key, input| Ok(Self { province: key as u32, supply_factor_raw: read_value(Kind::I64, input)?, railway_health_raw: read_value(Kind::I64, input)?, controller_index: read_value(Kind::U16, input)?, capital_country_index: read_value(Kind::U16, input)?, supply_node_level: read_value(Kind::U8, input)?, naval_base_level: read_value(Kind::U8, input)?, non_damaged_infra_level: read_value(Kind::U8, input)?, infra_level: read_value(Kind::U8, input)?, flags: read_value(Kind::U8, input)? }),
 { 0 => (supply_factor_raw): I64, 1 => (railway_health_raw): I64, 2 => (controller_index): U16, 3 => (capital_country_index): U16, 4 => (supply_node_level): U8, 5 => (naval_base_level): U8, 6 => (non_damaged_infra_level): U8, 7 => (infra_level): U8, 8 => (flags): U8 }, |_row| Ok(()));

/// HubMotorization: key owner:province:node_type:actor (128 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HubMotorizationRow {
    pub owner: u32,
    pub actor: u32,
    pub node_index: u32,
    pub map_entry_index: u32,
    pub province: i32,
    pub node_type: i32,
    pub level: u8,
}
typed_codec!(HubMotorizationRow, 128, |row| (row.owner as u128) << 96 | (row.province as u32 as u128) << 64 | (row.node_type as u32 as u128) << 32 | row.actor as u128, |key| Self { owner: (key >> 96) as u32, province: (key >> 64) as u32 as i32, node_type: (key >> 32) as u32 as i32, actor: key as u32, node_index: Default::default(), map_entry_index: Default::default(), level: Default::default() },
 |key, input| Ok(Self { owner: (key >> 96) as u32, province: (key >> 64) as u32 as i32, node_type: (key >> 32) as u32 as i32, actor: key as u32, node_index: read_value(Kind::U32, input)?, map_entry_index: read_value(Kind::U32, input)?, level: read_value(Kind::U8, input)? }),
 { 0 => (node_index): U32, 1 => (map_entry_index): U32, 2 => (level): U8 }, |_row| Ok(()));

/// DisruptedSupply: key country:province:node_type (96 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DisruptedSupplyRow {
    pub country: u32,
    pub province: u32,
    pub node_type: u32,
    pub value: i64,
}
typed_codec!(DisruptedSupplyRow, 96, |row| (row.country as u128) << 64 | (row.province as u128) << 32 | row.node_type as u128, |key| Self { country: (key >> 64) as u32, province: (key >> 32) as u32, node_type: key as u32, value: Default::default() },
 |key, input| Ok(Self { country: (key >> 64) as u32, province: (key >> 32) as u32, node_type: key as u32, value: read_value(Kind::I64, input)? }),
 { 0 => (value): I64 }, |_row| Ok(()));

/// ForeignHomebases: key country:ordinal (64 bits), 10 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ForeignHomebaseRow {
    pub country: u32,
    pub ordinal: u32,
    pub province: u32,
    pub supply: i64,
    pub start: i64,
    pub penalty: i64,
    pub added_penalty: i64,
    pub duration: i64,
    pub hours: i64,
    pub decay: i64,
    pub base: u8,
    pub source: Vec<u8>,
}
typed_codec!(ForeignHomebaseRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, province: Default::default(), supply: Default::default(), start: Default::default(), penalty: Default::default(), added_penalty: Default::default(), duration: Default::default(), hours: Default::default(), decay: Default::default(), base: Default::default(), source: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, ordinal: key as u32, province: read_value(Kind::U32, input)?, supply: read_value(Kind::I64, input)?, start: read_value(Kind::I64, input)?, penalty: read_value(Kind::I64, input)?, added_penalty: read_value(Kind::I64, input)?, duration: read_value(Kind::I64, input)?, hours: read_value(Kind::I64, input)?, decay: read_value(Kind::I64, input)?, base: read_value(Kind::U8, input)?, source: read_value(Kind::Bytes, input)? }),
 { 0 => (province): U32, 1 => (supply): I64, 2 => (start): I64, 3 => (penalty): I64, 4 => (added_penalty): I64, 5 => (duration): I64, 6 => (hours): I64, 7 => (decay): I64, 8 => (base): U8, 9 => (source): Bytes }, |_row| Ok(()));

/// CountryExperience: key country (32 bits), 9 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryExperienceRow {
    pub country: u32,
    pub army_raw: i64,
    pub army_daily_raw: i64,
    pub army_daily_training_raw: i64,
    pub navy_raw: i64,
    pub navy_daily_raw: i64,
    pub navy_daily_training_raw: i64,
    pub air_raw: i64,
    pub air_daily_raw: i64,
    pub armies_for_training_raw: i64,
}
typed_codec!(CountryExperienceRow, 32, |row| row.country as u128, |key| Self { country: key as u32, army_raw: Default::default(), army_daily_raw: Default::default(), army_daily_training_raw: Default::default(), navy_raw: Default::default(), navy_daily_raw: Default::default(), navy_daily_training_raw: Default::default(), air_raw: Default::default(), air_daily_raw: Default::default(), armies_for_training_raw: Default::default() },
 |key, input| Ok(Self { country: key as u32, army_raw: read_value(Kind::I64, input)?, army_daily_raw: read_value(Kind::I64, input)?, army_daily_training_raw: read_value(Kind::I64, input)?, navy_raw: read_value(Kind::I64, input)?, navy_daily_raw: read_value(Kind::I64, input)?, navy_daily_training_raw: read_value(Kind::I64, input)?, air_raw: read_value(Kind::I64, input)?, air_daily_raw: read_value(Kind::I64, input)?, armies_for_training_raw: read_value(Kind::I64, input)? }),
 { 0 => (army_raw): I64, 1 => (army_daily_raw): I64, 2 => (army_daily_training_raw): I64, 3 => (navy_raw): I64, 4 => (navy_daily_raw): I64, 5 => (navy_daily_training_raw): I64, 6 => (air_raw): I64, 7 => (air_daily_raw): I64, 8 => (armies_for_training_raw): I64 }, |_row| Ok(()));

/// StateBuildings: key location:building_type (64 bits), 7 payload mask bits.
/// ProvinceBuildings: key location:building_type (64 bits), 7 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BuildingRow {
    pub location: u32,
    pub state: u32,
    pub building_type: u32,
    pub ordinal: u32,
    pub status_flags: u32,
    pub country: u32,
    pub level: i16,
    pub healthy_level: i16,
    pub partial_health: i64,
}
typed_codec!(BuildingRow, 64, |row| (row.location as u128) << 32 | row.building_type as u128, |key| Self { location: (key >> 32) as u32, building_type: key as u32, state: Default::default(), ordinal: Default::default(), status_flags: Default::default(), country: Default::default(), level: Default::default(), healthy_level: Default::default(), partial_health: Default::default() },
 |key, input| Ok(Self { location: (key >> 32) as u32, building_type: key as u32, state: read_value(Kind::U32, input)?, ordinal: read_value(Kind::U32, input)?, status_flags: read_value(Kind::U32, input)?, country: read_value(Kind::U32, input)?, level: read_value(Kind::I16, input)?, healthy_level: read_value(Kind::I16, input)?, partial_health: read_value(Kind::I64, input)? }),
 { 0 => (state): U32, 1 => (ordinal): U32, 2 => (status_flags): U32, 3 => (country): U32, 4 => (level): I16, 5 => (healthy_level): I16, 6 => (partial_health): I64 }, |_row| Ok(()));

/// StateBaseResources: key state:resource (64 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StateBaseResourceRow {
    pub state: u32,
    pub resource: u32,
    pub ordinal: u32,
    pub amount: i64,
}
typed_codec!(StateBaseResourceRow, 64, |row| (row.state as u128) << 32 | row.resource as u128, |key| Self { state: (key >> 32) as u32, resource: key as u32, ordinal: Default::default(), amount: Default::default() },
 |key, input| Ok(Self { state: (key >> 32) as u32, resource: key as u32, ordinal: read_value(Kind::U32, input)?, amount: read_value(Kind::I64, input)? }),
 { 0 => (ordinal): U32, 1 => (amount): I64 }, |_row| Ok(()));

/// BuildingConstruction: key id (64 bits), 15 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
 |key, input| Ok(Self { id: key as u64, country: read_value(Kind::U32, input)?, queue_ordinal: read_value(Kind::U32, input)?, active_factories: read_value(Kind::I32, input)?, cost: read_value(Kind::I64, input)?, speed: read_value(Kind::I64, input)?, produced: read_value(Kind::I64, input)?, priority: read_value(Kind::I32, input)?, amount: read_value(Kind::I32, input)?, created_hour: read_value(Kind::I32, input)?, building_type: read_value(Kind::U32, input)?, target_flags: read_value(Kind::U32, input)?, target_location: read_value(Kind::U32, input)?, target_state: read_value(Kind::U32, input)?, target_country: read_value(Kind::U32, input)?, repair_levels: read_value(Kind::I32, input)? }),
 { 0 => (country): U32, 1 => (queue_ordinal): U32, 2 => (active_factories): I32, 3 => (cost): I64, 4 => (speed): I64, 5 => (produced): I64, 6 => (priority): I32, 7 => (amount): I32, 8 => (created_hour): I32, 9 => (building_type): U32, 10 => (target_flags): U32, 11 => (target_location): U32, 12 => (target_state): U32, 13 => (target_country): U32, 14 => (repair_levels): I32 }, |_row| Ok(()));

/// RailwayConstruction: key id (64 bits), 15 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
 |key, input| Ok(Self { id: key as u64, country: read_value(Kind::U32, input)?, queue_ordinal: read_value(Kind::U32, input)?, active_factories: read_value(Kind::I32, input)?, cost: read_value(Kind::I64, input)?, speed: read_value(Kind::I64, input)?, produced: read_value(Kind::I64, input)?, priority: read_value(Kind::I32, input)?, amount: read_value(Kind::I32, input)?, created_hour: read_value(Kind::I32, input)?, base_level: read_value(Kind::I32, input)?, target_level: read_value(Kind::I32, input)?, cancelled: read_value(Kind::U8, input)?, current_segment: read_value(Kind::I32, input)?, remaining_cost: read_value(Kind::I64, input)?, route: read_value(Kind::U32List, input)? }),
 { 0 => (country): U32, 1 => (queue_ordinal): U32, 2 => (active_factories): I32, 3 => (cost): I64, 4 => (speed): I64, 5 => (produced): I64, 6 => (priority): I32, 7 => (amount): I32, 8 => (created_hour): I32, 9 => (base_level): I32, 10 => (target_level): I32, 11 => (cancelled): U8, 12 => (current_segment): I32, 13 => (remaining_cost): I64, 14 => (route): U32List }, |_row| Ok(()));

/// OccupationSettings: key country (32 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OccupationSettingsRow {
    pub country: u32,
    pub garrison_template: Option<u64>,
    pub default_law_present: u8,
    pub law: Vec<u8>,
}
typed_codec!(OccupationSettingsRow, 32, |row| row.country as u128, |key| Self { country: key as u32, garrison_template: Default::default(), default_law_present: Default::default(), law: Default::default() },
 |key, input| Ok(Self { country: key as u32, garrison_template: read_value(Kind::OptionalU64, input)?, default_law_present: read_value(Kind::U8, input)?, law: read_value(Kind::Bytes, input)? }),
 { 0 => (garrison_template): OptionalU64, 1 => (default_law_present): U8, 2 => (law): Bytes }, |_row| Ok(()));

/// StateOccupationLaws: key controller:occupied:state (96 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StateOccupationLawRow {
    pub controller: u32,
    pub occupied: u32,
    pub state: u32,
    pub name: Vec<u8>,
}
typed_codec!(StateOccupationLawRow, 96, |row| (row.controller as u128) << 64 | (row.occupied as u128) << 32 | row.state as u128, |key| Self { controller: (key >> 64) as u32, occupied: (key >> 32) as u32, state: key as u32, name: Default::default() },
 |key, input| Ok(Self { controller: (key >> 64) as u32, occupied: (key >> 32) as u32, state: key as u32, name: read_value(Kind::Bytes, input)? }),
 { 0 => (name): Bytes }, |_row| Ok(()));

/// StateGarrisons: key controller:state (64 bits), 5 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StateGarrisonRow {
    pub controller: u32,
    pub occupied: u32,
    pub state: u32,
    pub required_manpower: i32,
    pub strength_ratio: i64,
    pub garrison_required: i64,
    pub serialized_flag: u8,
}
typed_codec!(StateGarrisonRow, 64, |row| (row.controller as u128) << 32 | row.state as u128, |key| Self { controller: (key >> 32) as u32, state: key as u32, occupied: Default::default(), required_manpower: Default::default(), strength_ratio: Default::default(), garrison_required: Default::default(), serialized_flag: Default::default() },
 |key, input| Ok(Self { controller: (key >> 32) as u32, state: key as u32, occupied: read_value(Kind::U32, input)?, required_manpower: read_value(Kind::I32, input)?, strength_ratio: read_value(Kind::I64, input)?, garrison_required: read_value(Kind::I64, input)?, serialized_flag: read_value(Kind::U8, input)? }),
 { 0 => (occupied): U32, 1 => (required_manpower): I32, 2 => (strength_ratio): I64, 3 => (garrison_required): I64, 4 => (serialized_flag): U8 }, |_row| Ok(()));

/// StateAssignedManpower: key controller:state:ordinal (96 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StateAssignedManpowerRow {
    pub controller: u32,
    pub occupied: u32,
    pub state: u32,
    pub ordinal: u32,
    pub source_country: u32,
    pub amount: i32,
}
typed_codec!(StateAssignedManpowerRow, 96, |row| (row.controller as u128) << 64 | (row.state as u128) << 32 | row.ordinal as u128, |key| Self { controller: (key >> 64) as u32, state: (key >> 32) as u32, ordinal: key as u32, occupied: Default::default(), source_country: Default::default(), amount: Default::default() },
 |key, input| Ok(Self { controller: (key >> 64) as u32, state: (key >> 32) as u32, ordinal: key as u32, occupied: read_value(Kind::U32, input)?, source_country: read_value(Kind::U32, input)?, amount: read_value(Kind::I32, input)? }),
 { 0 => (occupied): U32, 1 => (source_country): U32, 2 => (amount): I32 }, |_row| Ok(()));

/// CountryOccupationReserveManpower: key country:ordinal (64 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryOccupationReserveManpowerRow {
    pub country: u32,
    pub ordinal: u32,
    pub source_country: u32,
    pub amount: i32,
}
typed_codec!(CountryOccupationReserveManpowerRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, source_country: Default::default(), amount: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, ordinal: key as u32, source_country: read_value(Kind::U32, input)?, amount: read_value(Kind::I32, input)? }),
 { 0 => (source_country): U32, 1 => (amount): I32 }, |_row| Ok(()));

/// StateResistance: key state (32 bits), 6 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StateResistanceRow {
    pub state: u32,
    pub country: u32,
    pub level: i64,
    pub speed: i64,
    pub compliance: i64,
    pub compliance_speed: i64,
    pub active: u8,
}
typed_codec!(StateResistanceRow, 32, |row| row.state as u128, |key| Self { state: key as u32, country: Default::default(), level: Default::default(), speed: Default::default(), compliance: Default::default(), compliance_speed: Default::default(), active: Default::default() },
 |key, input| Ok(Self { state: key as u32, country: read_value(Kind::U32, input)?, level: read_value(Kind::I64, input)?, speed: read_value(Kind::I64, input)?, compliance: read_value(Kind::I64, input)?, compliance_speed: read_value(Kind::I64, input)?, active: read_value(Kind::U8, input)? }),
 { 0 => (country): U32, 1 => (level): I64, 2 => (speed): I64, 3 => (compliance): I64, 4 => (compliance_speed): I64, 5 => (active): U8 }, |_row| Ok(()));

/// CountryCollaboration: key source:target (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryCollaborationRow {
    pub source: u32,
    pub target: u32,
    pub progress: i64,
}
typed_codec!(CountryCollaborationRow, 64, |row| (row.source as u128) << 32 | row.target as u128, |key| Self { source: (key >> 32) as u32, target: key as u32, progress: Default::default() },
 |key, input| Ok(Self { source: (key >> 32) as u32, target: key as u32, progress: read_value(Kind::I64, input)? }),
 { 0 => (progress): I64 }, |_row| Ok(()));

/// Characters: key id (32 bits), 6 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CharacterRow {
    pub id: u32,
    pub country: u32,
    pub name_token_id: i32,
    pub roster: u8,
    pub ordinal: u32,
    pub name: Vec<u8>,
    pub token: Vec<u8>,
}
typed_codec!(CharacterRow, 32, |row| row.id as u128, |key| Self { id: key as u32, country: Default::default(), name_token_id: Default::default(), roster: Default::default(), ordinal: Default::default(), name: Default::default(), token: Default::default() },
 |key, input| Ok(Self { id: key as u32, country: read_value(Kind::U32, input)?, name_token_id: read_value(Kind::I32, input)?, roster: read_value(Kind::U8, input)?, ordinal: read_value(Kind::U32, input)?, name: read_value(Kind::Bytes, input)?, token: read_value(Kind::Bytes, input)? }),
 { 0 => (country): U32, 1 => (name_token_id): I32, 2 => (roster): U8, 3 => (ordinal): U32, 4 => (name): Bytes, 5 => (token): Bytes }, |_row| Ok(()));

/// CountryLeaderRoles: key character:ideology (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryLeaderRoleRow {
    pub character: u32,
    pub country: u32,
    pub ideology: i32,
}
typed_codec!(CountryLeaderRoleRow, 64, |row| (row.character as u128) << 32 | row.ideology as u32 as u128, |key| Self { character: (key >> 32) as u32, ideology: key as u32 as i32, country: Default::default() },
 |key, input| Ok(Self { character: (key >> 32) as u32, ideology: key as u32 as i32, country: read_value(Kind::U32, input)? }),
 { 0 => (country): U32 }, |_row| Ok(()));

/// AdvisorRoles: key character:ordinal (64 bits), 4 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AdvisorRoleRow {
    pub character: u32,
    pub country: u32,
    pub ordinal: u32,
    pub key: Vec<u8>,
    pub idea: Vec<u8>,
    pub slot: Vec<u8>,
}
typed_codec!(AdvisorRoleRow, 64, |row| (row.character as u128) << 32 | row.ordinal as u128, |key| Self { character: (key >> 32) as u32, ordinal: key as u32, country: Default::default(), key: Default::default(), idea: Default::default(), slot: Default::default() },
 |key, input| Ok(Self { character: (key >> 32) as u32, ordinal: key as u32, country: read_value(Kind::U32, input)?, key: read_value(Kind::Bytes, input)?, idea: read_value(Kind::Bytes, input)?, slot: read_value(Kind::Bytes, input)? }),
 { 0 => (country): U32, 1 => (key): Bytes, 2 => (idea): Bytes, 3 => (slot): Bytes }, |_row| Ok(()));

/// UnitLeaders: key id:reference_type (64 bits), 16 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UnitLeaderRow {
    pub reference_type: u32,
    pub id: u32,
    pub character_type: u32,
    pub character_id: u32,
    pub category: i32,
    pub experience: i64,
    pub kind: u8,
    pub skill_status: u8,
    pub skill_valid: u8,
    pub skill_level: i32,
    pub skill_required_experience: i32,
    pub skill_category: i32,
    pub components: [i32; 4],
    pub inherited_text_a: Vec<u8>,
    pub inherited_text_b: Vec<u8>,
}
typed_codec!(UnitLeaderRow, 64, |row| (row.id as u128) << 32 | row.reference_type as u128, |key| Self { id: (key >> 32) as u32, reference_type: key as u32, character_type: Default::default(), character_id: Default::default(), category: Default::default(), experience: Default::default(), kind: Default::default(), skill_status: Default::default(), skill_valid: Default::default(), skill_level: Default::default(), skill_required_experience: Default::default(), skill_category: Default::default(), components: Default::default(), inherited_text_a: Default::default(), inherited_text_b: Default::default() },
 |key, input| Ok(Self { id: (key >> 32) as u32, reference_type: key as u32, character_type: read_value(Kind::U32, input)?, character_id: read_value(Kind::U32, input)?, category: read_value(Kind::I32, input)?, experience: read_value(Kind::I64, input)?, kind: read_value(Kind::U8, input)?, skill_status: read_value(Kind::U8, input)?, skill_valid: read_value(Kind::U8, input)?, skill_level: read_value(Kind::I32, input)?, skill_required_experience: read_value(Kind::I32, input)?, skill_category: read_value(Kind::I32, input)?, components: <[i32;4] as Value>::decode(input)?, inherited_text_a: read_value(Kind::Bytes, input)?, inherited_text_b: read_value(Kind::Bytes, input)? }),
 { 0 => (character_type): U32, 1 => (character_id): U32, 2 => (category): I32, 3 => (experience): I64, 4 => (kind): U8, 5 => (skill_status): U8, 6 => (skill_valid): U8, 7 => (skill_level): I32, 8 => (skill_required_experience): I32, 9 => (skill_category): I32, 10 => (components[0]): I32, 11 => (components[1]): I32, 12 => (components[2]): I32, 13 => (components[3]): I32, 14 => (inherited_text_a): Bytes, 15 => (inherited_text_b): Bytes }, |_row| Ok(()));

/// UnitLeaderTraits: key leader_reference:ordinal (96 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UnitLeaderTraitRow {
    pub leader_reference: u64,
    pub ordinal: u32,
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub token: Vec<u8>,
}
typed_codec!(UnitLeaderTraitRow, 96, |row| (row.leader_reference as u128) << 32 | row.ordinal as u128, |key| Self { leader_reference: (key >> 32) as u64, ordinal: key as u32, token_id: Default::default(), definition_ordinal: Default::default(), token: Default::default() },
 |key, input| Ok(Self { leader_reference: (key >> 32) as u64, ordinal: key as u32, token_id: read_value(Kind::I32, input)?, definition_ordinal: read_value(Kind::I32, input)?, token: read_value(Kind::Bytes, input)? }),
 { 0 => (token_id): I32, 1 => (definition_ordinal): I32, 2 => (token): Bytes }, |_row| Ok(()));

/// UnitLeaderTraitExperience: key leader_reference:ordinal (96 bits), 4 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UnitLeaderTraitExperienceRow {
    pub leader_reference: u64,
    pub ordinal: u32,
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub token: Vec<u8>,
    pub experience: i64,
}
typed_codec!(UnitLeaderTraitExperienceRow, 96, |row| (row.leader_reference as u128) << 32 | row.ordinal as u128, |key| Self { leader_reference: (key >> 32) as u64, ordinal: key as u32, token_id: Default::default(), definition_ordinal: Default::default(), token: Default::default(), experience: Default::default() },
 |key, input| Ok(Self { leader_reference: (key >> 32) as u64, ordinal: key as u32, token_id: read_value(Kind::I32, input)?, definition_ordinal: read_value(Kind::I32, input)?, token: read_value(Kind::Bytes, input)?, experience: read_value(Kind::I64, input)? }),
 { 0 => (token_id): I32, 1 => (definition_ordinal): I32, 2 => (token): Bytes, 3 => (experience): I64 }, |_row| Ok(()));

/// ActiveAbilities: key country:ordinal (64 bits), 12 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ActiveAbilityRow {
    pub country: u32,
    pub ordinal: u32,
    pub scope_country: u32,
    pub token: Vec<u8>,
    pub caster: Option<u64>,
    pub strategic_region: Option<u32>,
    pub division_count: i32,
    pub activation_hour: i32,
    pub phase: u8,
    pub allocate_cost: u8,
    pub allocated_command_power: i64,
    pub duration_hours: i32,
    pub cooldown_hours: i32,
    pub target_kind: i32,
}
typed_codec!(ActiveAbilityRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, scope_country: Default::default(), token: Default::default(), caster: Default::default(), strategic_region: Default::default(), division_count: Default::default(), activation_hour: Default::default(), phase: Default::default(), allocate_cost: Default::default(), allocated_command_power: Default::default(), duration_hours: Default::default(), cooldown_hours: Default::default(), target_kind: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, ordinal: key as u32, scope_country: read_value(Kind::U32, input)?, token: read_value(Kind::Bytes, input)?, caster: read_value(Kind::OptionalU64, input)?, strategic_region: read_value(Kind::OptionalU32, input)?, division_count: read_value(Kind::I32, input)?, activation_hour: read_value(Kind::I32, input)?, phase: read_value(Kind::U8, input)?, allocate_cost: read_value(Kind::U8, input)?, allocated_command_power: read_value(Kind::I64, input)?, duration_hours: read_value(Kind::I32, input)?, cooldown_hours: read_value(Kind::I32, input)?, target_kind: read_value(Kind::I32, input)? }),
 { 0 => (scope_country): U32, 1 => (token): Bytes, 2 => (caster): OptionalU64, 3 => (strategic_region): OptionalU32, 4 => (division_count): I32, 5 => (activation_hour): I32, 6 => (phase): U8, 7 => (allocate_cost): U8, 8 => (allocated_command_power): I64, 9 => (duration_hours): I32, 10 => (cooldown_hours): I32, 11 => (target_kind): I32 }, |_row| Ok(()));

/// Capitulation: key country (32 bits), 6 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CapitulationRow {
    pub country: u32,
    pub capitulated: u8,
    pub capitulated_hour: i32,
    pub last_surrender_hour: i32,
    pub faction_join_hour: i32,
    pub legitimacy_raw: i64,
    pub exile_army_leaders: i32,
}
typed_codec!(CapitulationRow, 32, |row| row.country as u128, |key| Self { country: key as u32, capitulated: Default::default(), capitulated_hour: Default::default(), last_surrender_hour: Default::default(), faction_join_hour: Default::default(), legitimacy_raw: Default::default(), exile_army_leaders: Default::default() },
 |key, input| Ok(Self { country: key as u32, capitulated: read_value(Kind::U8, input)?, capitulated_hour: read_value(Kind::I32, input)?, last_surrender_hour: read_value(Kind::I32, input)?, faction_join_hour: read_value(Kind::I32, input)?, legitimacy_raw: read_value(Kind::I64, input)?, exile_army_leaders: read_value(Kind::I32, input)? }),
 { 0 => (capitulated): U8, 1 => (capitulated_hour): I32, 2 => (last_surrender_hour): I32, 3 => (faction_join_hour): I32, 4 => (legitimacy_raw): I64, 5 => (exile_army_leaders): I32 }, |_row| Ok(()));

/// Subjects: key overlord:subject (64 bits), 5 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SubjectRow {
    pub overlord: u32,
    pub subject: u32,
    pub start_hour: i32,
    pub end_hour: i32,
    pub cancel: u8,
    pub autonomy_ratio: i64,
    pub state: Vec<u8>,
}
typed_codec!(SubjectRow, 64, |row| (row.overlord as u128) << 32 | row.subject as u128, |key| Self { overlord: (key >> 32) as u32, subject: key as u32, start_hour: Default::default(), end_hour: Default::default(), cancel: Default::default(), autonomy_ratio: Default::default(), state: Default::default() },
 |key, input| Ok(Self { overlord: (key >> 32) as u32, subject: key as u32, start_hour: read_value(Kind::I32, input)?, end_hour: read_value(Kind::I32, input)?, cancel: read_value(Kind::U8, input)?, autonomy_ratio: read_value(Kind::I64, input)?, state: read_value(Kind::Bytes, input)? }),
 { 0 => (start_hour): I32, 1 => (end_hour): I32, 2 => (cancel): U8, 3 => (autonomy_ratio): I64, 4 => (state): Bytes }, |_row| Ok(()));

/// Autonomy: key subject (32 bits), 7 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AutonomyRow {
    pub subject: u32,
    pub score: i64,
    pub lower_score: i64,
    pub upper_score: i64,
    pub last_modified_bits: u32,
    pub state: Vec<u8>,
    pub previous: Option<Vec<u8>>,
    pub next: Option<Vec<u8>>,
}
typed_codec!(AutonomyRow, 32, |row| row.subject as u128, |key| Self { subject: key as u32, score: Default::default(), lower_score: Default::default(), upper_score: Default::default(), last_modified_bits: Default::default(), state: Default::default(), previous: Default::default(), next: Default::default() },
 |key, input| Ok(Self { subject: key as u32, score: read_value(Kind::I64, input)?, lower_score: read_value(Kind::I64, input)?, upper_score: read_value(Kind::I64, input)?, last_modified_bits: read_value(Kind::U32, input)?, state: read_value(Kind::Bytes, input)?, previous: read_value(Kind::Opaque, input)?, next: read_value(Kind::Opaque, input)? }),
 { 0 => (score): I64, 1 => (lower_score): I64, 2 => (upper_score): I64, 3 => (last_modified_bits): U32, 4 => (state): Bytes, 5 => (previous): Opaque, 6 => (next): Opaque }, |_row| Ok(()));

/// IntelligenceAgencies: key country (32 bits), 5 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IntelligenceAgencyRow {
    pub country: u32,
    pub upgrade_progress_raw: i64,
    pub defense_raw: i64,
    pub active_count: i32,
    pub admission_ceiling: i32,
    pub unknown_f0: u32,
}
typed_codec!(IntelligenceAgencyRow, 32, |row| row.country as u128, |key| Self { country: key as u32, upgrade_progress_raw: Default::default(), defense_raw: Default::default(), active_count: Default::default(), admission_ceiling: Default::default(), unknown_f0: Default::default() },
 |key, input| Ok(Self { country: key as u32, upgrade_progress_raw: read_value(Kind::I64, input)?, defense_raw: read_value(Kind::I64, input)?, active_count: read_value(Kind::I32, input)?, admission_ceiling: read_value(Kind::I32, input)?, unknown_f0: read_value(Kind::U32, input)? }),
 { 0 => (upgrade_progress_raw): I64, 1 => (defense_raw): I64, 2 => (active_count): I32, 3 => (admission_ceiling): I32, 4 => (unknown_f0): U32 }, |_row| Ok(()));

/// OperativeMemberships: key id:roster_country:roster_kind (104 bits), 8 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OperativeMembershipRow {
    pub id: u64,
    pub owner_country: u32,
    pub roster_country: u32,
    pub roster_kind: u8,
    pub detected: u8,
    pub assignment: u32,
    pub experience_raw: i64,
    pub skill: Option<i32>,
    pub capturer_country: u32,
    pub captured_hour: i32,
    pub action_variant: Option<u32>,
}
typed_codec!(OperativeMembershipRow, 104, |row| (row.id as u128) << 40 | (row.roster_country as u128) << 8 | row.roster_kind as u128, |key| Self { id: (key >> 40) as u64, roster_country: (key >> 8) as u32, roster_kind: key as u8, owner_country: Default::default(), detected: Default::default(), assignment: Default::default(), experience_raw: Default::default(), skill: Default::default(), capturer_country: Default::default(), captured_hour: Default::default(), action_variant: Default::default() },
 |key, input| Ok(Self { id: (key >> 40) as u64, roster_country: (key >> 8) as u32, roster_kind: key as u8, owner_country: read_value(Kind::U32, input)?, detected: read_value(Kind::U8, input)?, assignment: read_value(Kind::U32, input)?, experience_raw: read_value(Kind::I64, input)?, skill: read_value(Kind::OptionalI32, input)?, capturer_country: read_value(Kind::U32, input)?, captured_hour: read_value(Kind::I32, input)?, action_variant: read_value(Kind::OptionalU32, input)? }),
 { 0 => (owner_country): U32, 1 => (detected): U8, 2 => (assignment): U32, 3 => (experience_raw): I64, 4 => (skill): OptionalI32, 5 => (capturer_country): U32, 6 => (captured_hour): I32, 7 => (action_variant): OptionalU32 }, |_row| Ok(()));

/// Cryptology: key owner_country:target_country (64 bits), 4 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CryptologyRow {
    pub owner_country: u32,
    pub target_country: u32,
    pub amount_raw: i64,
    pub bonus_days: i32,
    pub bonus_active: u8,
    pub decrypting: u8,
}
typed_codec!(CryptologyRow, 64, |row| (row.owner_country as u128) << 32 | row.target_country as u128, |key| Self { owner_country: (key >> 32) as u32, target_country: key as u32, amount_raw: Default::default(), bonus_days: Default::default(), bonus_active: Default::default(), decrypting: Default::default() },
 |key, input| Ok(Self { owner_country: (key >> 32) as u32, target_country: key as u32, amount_raw: read_value(Kind::I64, input)?, bonus_days: read_value(Kind::I32, input)?, bonus_active: read_value(Kind::U8, input)?, decrypting: read_value(Kind::U8, input)? }),
 { 0 => (amount_raw): I64, 1 => (bonus_days): I32, 2 => (bonus_active): U8, 3 => (decrypting): U8 }, |_row| Ok(()));

/// IntelNetworks: key owner_country:target_country:state_id (96 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IntelNetworkRow {
    pub owner_country: u32,
    pub target_country: u32,
    pub state_id: i32,
    /// Absent when the native subnetwork selector is negative.
    pub strength_raw: Option<i64>,
    /// Exact signed selector, including all native unassigned values.
    pub subnetwork_selector: i32,
}
typed_codec!(IntelNetworkRow, 96, |row| (row.owner_country as u128) << 64 | (row.target_country as u128) << 32 | row.state_id as u32 as u128, |key| Self { owner_country: (key >> 64) as u32, target_country: (key >> 32) as u32, state_id: key as u32 as i32, strength_raw: Default::default(), subnetwork_selector: Default::default() },
 |key, input| Ok(Self { owner_country: (key >> 64) as u32, target_country: (key >> 32) as u32, state_id: key as u32 as i32, strength_raw: read_value(Kind::OptionalI64, input)?, subnetwork_selector: read_value(Kind::I32, input)? }),
 { 0 => (strength_raw): OptionalI64, 1 => (subnetwork_selector): I32 }, |_row| Ok(()));

/// ScriptFlags: key scope_kind:scope_id:token (72 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScriptFlagRow {
    pub scope_kind: u8,
    pub scope_id: u32,
    pub token: u32,
    pub set_hour: i32,
    pub value: i16,
    pub remaining_days: i16,
}
typed_codec!(ScriptFlagRow, 72, |row| (row.scope_kind as u128) << 64 | (row.scope_id as u128) << 32 | row.token as u128, |key| Self { scope_kind: (key >> 64) as u8, scope_id: (key >> 32) as u32, token: key as u32, set_hour: Default::default(), value: Default::default(), remaining_days: Default::default() },
 |key, input| Ok(Self { scope_kind: (key >> 64) as u8, scope_id: (key >> 32) as u32, token: key as u32, set_hour: read_value(Kind::I32, input)?, value: read_value(Kind::I16, input)?, remaining_days: read_value(Kind::I16, input)? }),
 { 0 => (set_hour): I32, 1 => (value): I16, 2 => (remaining_days): I16 }, |_row| Ok(()));

/// ScriptVariables: key scope_kind:scope_id (40 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScriptVariablesRow {
    pub scope_kind: u8,
    pub scope_id: u32,
    pub entries: Vec<ScriptVariableEntry>,
}
typed_codec!(ScriptVariablesRow, 40, |row| (row.scope_kind as u128) << 32 | row.scope_id as u128, |key| Self { scope_kind: (key >> 32) as u8, scope_id: key as u32, entries: Default::default() },
 |key, input| Ok(Self { scope_kind: (key >> 32) as u8, scope_id: key as u32, entries: read_value(Kind::Opaque, input)? }),
 { 0 => (entries): Opaque }, |_row| Ok(()));

/// Operations: key id (64 bits), 13 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OperationRow {
    pub id: u64,
    pub owner: u32,
    pub target: u32,
    pub target_province: Option<u32>,
    pub definition_token: u32,
    pub preparation_current: i64,
    pub preparation_required: i64,
    pub flag_40: u8,
    pub auto_repeat: u8,
    pub terminal: u8,
    pub start_hour: i32,
    pub duration_days: i32,
    pub phase_count: u32,
    pub assigned_slots: u32,
}
typed_codec!(OperationRow, 64, |row| row.id as u128, |key| Self { id: key as u64, owner: Default::default(), target: Default::default(), target_province: Default::default(), definition_token: Default::default(), preparation_current: Default::default(), preparation_required: Default::default(), flag_40: Default::default(), auto_repeat: Default::default(), terminal: Default::default(), start_hour: Default::default(), duration_days: Default::default(), phase_count: Default::default(), assigned_slots: Default::default() },
 |key, input| Ok(Self { id: key as u64, owner: read_value(Kind::U32, input)?, target: read_value(Kind::U32, input)?, target_province: read_value(Kind::OptionalU32, input)?, definition_token: read_value(Kind::U32, input)?, preparation_current: read_value(Kind::I64, input)?, preparation_required: read_value(Kind::I64, input)?, flag_40: read_value(Kind::U8, input)?, auto_repeat: read_value(Kind::U8, input)?, terminal: read_value(Kind::U8, input)?, start_hour: read_value(Kind::I32, input)?, duration_days: read_value(Kind::I32, input)?, phase_count: read_value(Kind::U32, input)?, assigned_slots: read_value(Kind::U32, input)? }),
 { 0 => (owner): U32, 1 => (target): U32, 2 => (target_province): OptionalU32, 3 => (definition_token): U32, 4 => (preparation_current): I64, 5 => (preparation_required): I64, 6 => (flag_40): U8, 7 => (auto_repeat): U8, 8 => (terminal): U8, 9 => (start_hour): I32, 10 => (duration_days): I32, 11 => (phase_count): U32, 12 => (assigned_slots): U32 }, |_row| Ok(()));

/// OperationAssignments: key operation:ordinal (96 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OperationAssignmentRow {
    pub operation: u64,
    pub ordinal: u32,
    pub operative: u64,
    pub selection: u32,
}
typed_codec!(OperationAssignmentRow, 96, |row| (row.operation as u128) << 32 | row.ordinal as u128, |key| Self { operation: (key >> 32) as u64, ordinal: key as u32, operative: Default::default(), selection: Default::default() },
 |key, input| Ok(Self { operation: (key >> 32) as u64, ordinal: key as u32, operative: read_value(Kind::U64, input)?, selection: read_value(Kind::U32, input)? }),
 { 0 => (operative): U64, 1 => (selection): U32 }, |_row| Ok(()));

/// WingEquipment: key wing:ordinal (96 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WingEquipmentRow {
    pub wing: u64,
    pub ordinal: u32,
    pub variant: u64,
    pub amount_raw: i64,
}
typed_codec!(WingEquipmentRow, 96, |row| (row.wing as u128) << 32 | row.ordinal as u128, |key| Self { wing: (key >> 32) as u64, ordinal: key as u32, variant: Default::default(), amount_raw: Default::default() },
 |key, input| Ok(Self { wing: (key >> 32) as u64, ordinal: key as u32, variant: read_value(Kind::U64, input)?, amount_raw: read_value(Kind::I64, input)? }),
 { 0 => (variant): U64, 1 => (amount_raw): I64 }, |_row| Ok(()));

/// AirBases: key id (64 bits), 5 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirBaseRow {
    pub id: u64,
    pub state: Option<u32>,
    pub province: Option<u32>,
    pub host: u64,
    pub capacity: i32,
    pub kind: i32,
}
typed_codec!(AirBaseRow, 64, |row| row.id as u128, |key| Self { id: key as u64, state: Default::default(), province: Default::default(), host: Default::default(), capacity: Default::default(), kind: Default::default() },
 |key, input| Ok(Self { id: key as u64, state: read_value(Kind::OptionalU32, input)?, province: read_value(Kind::OptionalU32, input)?, host: read_value(Kind::U64, input)?, capacity: read_value(Kind::I32, input)?, kind: read_value(Kind::I32, input)? }),
 { 0 => (state): OptionalU32, 1 => (province): OptionalU32, 2 => (host): U64, 3 => (capacity): I32, 4 => (kind): I32 }, |_row| Ok(()));

/// WingCumulative: key id (64 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WingCumulativeRow {
    pub id: u64,
    pub accidents: i32,
    pub experience: i64,
}
typed_codec!(WingCumulativeRow, 64, |row| row.id as u128, |key| Self { id: key as u64, accidents: Default::default(), experience: Default::default() },
 |key, input| Ok(Self { id: key as u64, accidents: read_value(Kind::I32, input)?, experience: read_value(Kind::I64, input)? }),
 { 0 => (accidents): I32, 1 => (experience): I64 }, |_row| Ok(()));

/// AirRegionHistory: key country:region (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirRegionHistoryRow {
    pub country: u32,
    pub region: u32,
    pub entries: Vec<AirHistoryEntry>,
}
typed_codec!(AirRegionHistoryRow, 64, |row| (row.country as u128) << 32 | row.region as u128, |key| Self { country: (key >> 32) as u32, region: key as u32, entries: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, region: key as u32, entries: read_value(Kind::Opaque, input)? }),
 { 0 => (entries): Opaque }, |_row| Ok(()));

/// AirRegionDamage: key country:region (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirRegionDamageRow {
    pub country: u32,
    pub region: u32,
    pub entries: Vec<AirDamageEntry>,
}
typed_codec!(AirRegionDamageRow, 64, |row| (row.country as u128) << 32 | row.region as u128, |key| Self { country: (key >> 32) as u32, region: key as u32, entries: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, region: key as u32, entries: read_value(Kind::Opaque, input)? }),
 { 0 => (entries): Opaque }, |_row| Ok(()));

/// AirAces: key id (64 bits), 11 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirAceRow {
    pub id: u64,
    pub country: u32,
    pub ordinal: u32,
    pub aircraft_type_mask: u32,
    pub assigned_wing: u64,
    pub alive: u8,
    pub handled: u8,
    pub kill_type: u32,
    pub death: Option<AceDeath>,
    pub name: Vec<u8>,
    pub surname: Vec<u8>,
    pub callsign: Vec<u8>,
}
typed_codec!(AirAceRow, 64, |row| row.id as u128, |key| Self { id: key as u64, country: Default::default(), ordinal: Default::default(), aircraft_type_mask: Default::default(), assigned_wing: Default::default(), alive: Default::default(), handled: Default::default(), kill_type: Default::default(), death: Default::default(), name: Default::default(), surname: Default::default(), callsign: Default::default() },
 |key, input| Ok(Self { id: key as u64, country: read_value(Kind::U32, input)?, ordinal: read_value(Kind::U32, input)?, aircraft_type_mask: read_value(Kind::U32, input)?, assigned_wing: read_value(Kind::U64, input)?, alive: read_value(Kind::U8, input)?, handled: read_value(Kind::U8, input)?, kill_type: read_value(Kind::U32, input)?, death: read_value(Kind::Opaque, input)?, name: read_value(Kind::Bytes, input)?, surname: read_value(Kind::Bytes, input)?, callsign: read_value(Kind::Bytes, input)? }),
 { 0 => (country): U32, 1 => (ordinal): U32, 2 => (aircraft_type_mask): U32, 3 => (assigned_wing): U64, 4 => (alive): U8, 5 => (handled): U8, 6 => (kill_type): U32, 7 => (death): Opaque, 8 => (name): Bytes, 9 => (surname): Bytes, 10 => (callsign): Bytes }, |_row| Ok(()));

/// AirStatistics: key country:region (64 bits), 26 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirStatisticsRow {
    pub country: u32,
    pub region: u32,
    pub year_slot: u32,
    pub year_window_wrapped: u8,
    pub channels: [i64; 24],
}
typed_codec!(AirStatisticsRow, 64, |row| (row.country as u128) << 32 | row.region as u128, |key| Self { country: (key >> 32) as u32, region: key as u32, year_slot: Default::default(), year_window_wrapped: Default::default(), channels: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, region: key as u32, year_slot: read_value(Kind::U32, input)?, year_window_wrapped: read_value(Kind::U8, input)?, channels: <[i64;24] as Value>::decode(input)? }),
 { 0 => (year_slot): U32, 1 => (year_window_wrapped): U8, 2 => (channels[0]): I64, 3 => (channels[1]): I64, 4 => (channels[2]): I64, 5 => (channels[3]): I64, 6 => (channels[4]): I64, 7 => (channels[5]): I64, 8 => (channels[6]): I64, 9 => (channels[7]): I64, 10 => (channels[8]): I64, 11 => (channels[9]): I64, 12 => (channels[10]): I64, 13 => (channels[11]): I64, 14 => (channels[12]): I64, 15 => (channels[13]): I64, 16 => (channels[14]): I64, 17 => (channels[15]): I64, 18 => (channels[16]): I64, 19 => (channels[17]): I64, 20 => (channels[18]): I64, 21 => (channels[19]): I64, 22 => (channels[20]): I64, 23 => (channels[21]): I64, 24 => (channels[22]): I64, 25 => (channels[23]): I64 }, |_row| Ok(()));

/// SunkConvoys: key month:killer_country:owner_country (96 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SunkConvoyRow {
    pub month: i32,
    pub killer_country: u32,
    pub owner_country: u32,
    pub convoys: i32,
    pub ordinal: u32,
}
typed_codec!(SunkConvoyRow, 96, |row| (row.month as u32 as u128) << 64 | (row.killer_country as u128) << 32 | row.owner_country as u128, |key| Self { month: (key >> 64) as u32 as i32, killer_country: (key >> 32) as u32, owner_country: key as u32, convoys: Default::default(), ordinal: Default::default() },
 |key, input| Ok(Self { month: (key >> 64) as u32 as i32, killer_country: (key >> 32) as u32, owner_country: key as u32, convoys: read_value(Kind::I32, input)?, ordinal: read_value(Kind::U32, input)? }),
 { 0 => (convoys): I32, 1 => (ordinal): U32 }, |_row| Ok(()));

/// RailCooldowns: key province (32 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RailCooldownRow {
    pub province: u32,
    pub cooldown_ticks: i32,
}
typed_codec!(RailCooldownRow, 32, |row| row.province as u128, |key| Self { province: key as u32, cooldown_ticks: Default::default() },
 |key, input| Ok(Self { province: key as u32, cooldown_ticks: read_value(Kind::I32, input)? }),
 { 0 => (cooldown_ticks): I32 }, |_row| Ok(()));

/// RailEdges: key from:to (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RailEdgeRow {
    pub from: u32,
    pub to: u32,
    pub level: i32,
}
typed_codec!(RailEdgeRow, 64, |row| (row.from as u128) << 32 | row.to as u128, |key| Self { from: (key >> 32) as u32, to: key as u32, level: Default::default() },
 |key, input| Ok(Self { from: (key >> 32) as u32, to: key as u32, level: read_value(Kind::I32, input)? }),
 { 0 => (level): I32 }, |_row| Ok(()));

/// LandTheatres: key id (64 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandTheatreRow {
    pub id: u64,
    pub owner: u32,
    pub ordinal: u32,
    /// Native signed CTheatre +0x110 value, independent of the roster owner.
    pub volunteers_theatre: i32,
}
typed_codec!(LandTheatreRow, 64, |row| row.id as u128, |key| Self { id: key as u64, owner: Default::default(), ordinal: Default::default(), volunteers_theatre: Default::default() },
 |key, input| Ok(Self { id: key as u64, owner: read_value(Kind::U32, input)?, ordinal: read_value(Kind::U32, input)?, volunteers_theatre: read_value(Kind::I32, input)? }),
 { 0 => (owner): U32, 1 => (ordinal): U32, 2 => (volunteers_theatre): I32 }, |_row| Ok(()));

/// LandTheatreGroups: key id (64 bits), 4 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandTheatreGroupRow {
    pub id: u64,
    pub theatre: u64,
    pub ordinal: u32,
    pub priority: i32,
    pub name: Vec<u8>,
}
typed_codec!(LandTheatreGroupRow, 64, |row| row.id as u128, |key| Self { id: key as u64, theatre: Default::default(), ordinal: Default::default(), priority: Default::default(), name: Default::default() },
 |key, input| Ok(Self { id: key as u64, theatre: read_value(Kind::U64, input)?, ordinal: read_value(Kind::U32, input)?, priority: read_value(Kind::I32, input)?, name: read_value(Kind::Bytes, input)? }),
 { 0 => (theatre): U64, 1 => (ordinal): U32, 2 => (priority): I32, 3 => (name): Bytes }, |_row| Ok(()));

/// LandTheatreMembers: key theater_group:orders_group (128 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandTheatreMemberRow {
    pub orders_group: u64,
    pub theater_group: u64,
    pub ordinal: u32,
}
typed_codec!(LandTheatreMemberRow, 128, |row| (row.theater_group as u128) << 64 | row.orders_group as u128, |key| Self { theater_group: (key >> 64) as u64, orders_group: key as u64, ordinal: Default::default() },
 |key, input| Ok(Self { theater_group: (key >> 64) as u64, orders_group: key as u64, ordinal: read_value(Kind::U32, input)? }),
 { 0 => (ordinal): U32 }, |_row| Ok(()));

/// AirTheatres: key id (64 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirTheatreRow {
    pub id: u64,
    pub owner: u32,
    pub ordinal: u32,
    pub name: Vec<u8>,
}
typed_codec!(AirTheatreRow, 64, |row| row.id as u128, |key| Self { id: key as u64, owner: Default::default(), ordinal: Default::default(), name: Default::default() },
 |key, input| Ok(Self { id: key as u64, owner: read_value(Kind::U32, input)?, ordinal: read_value(Kind::U32, input)?, name: read_value(Kind::Bytes, input)? }),
 { 0 => (owner): U32, 1 => (ordinal): U32, 2 => (name): Bytes }, |_row| Ok(()));

/// AirTheatreGroups: key id (64 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirTheatreGroupRow {
    pub id: u64,
    pub theatre: u64,
    pub ordinal: u32,
    pub name: Vec<u8>,
}
typed_codec!(AirTheatreGroupRow, 64, |row| row.id as u128, |key| Self { id: key as u64, theatre: Default::default(), ordinal: Default::default(), name: Default::default() },
 |key, input| Ok(Self { id: key as u64, theatre: read_value(Kind::U64, input)?, ordinal: read_value(Kind::U32, input)?, name: read_value(Kind::Bytes, input)? }),
 { 0 => (theatre): U64, 1 => (ordinal): U32, 2 => (name): Bytes }, |_row| Ok(()));

/// AirTheatreMembers: key group:wing (128 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirTheatreMemberRow {
    pub wing: u64,
    pub group: u64,
    pub ordinal: u32,
}
typed_codec!(AirTheatreMemberRow, 128, |row| (row.group as u128) << 64 | row.wing as u128, |key| Self { group: (key >> 64) as u64, wing: key as u64, ordinal: Default::default() },
 |key, input| Ok(Self { group: (key >> 64) as u64, wing: key as u64, ordinal: read_value(Kind::U32, input)? }),
 { 0 => (ordinal): U32 }, |_row| Ok(()));

/// NavalResults: key id (64 bits), 10 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalResultRow {
    pub id: u64,
    pub province: u32,
    pub result_hour: i32,
    pub retire_hour: i32,
    pub category: u32,
    pub flag_a: u8,
    pub flag_b: u8,
    pub subtype: u8,
    pub readiness: u8,
    pub winning_side: u8,
    pub losing_side: u8,
}
typed_codec!(NavalResultRow, 64, |row| row.id as u128, |key| Self { id: key as u64, province: Default::default(), result_hour: Default::default(), retire_hour: Default::default(), category: Default::default(), flag_a: Default::default(), flag_b: Default::default(), subtype: Default::default(), readiness: Default::default(), winning_side: Default::default(), losing_side: Default::default() },
 |key, input| Ok(Self { id: key as u64, province: read_value(Kind::U32, input)?, result_hour: read_value(Kind::I32, input)?, retire_hour: read_value(Kind::I32, input)?, category: read_value(Kind::U32, input)?, flag_a: read_value(Kind::U8, input)?, flag_b: read_value(Kind::U8, input)?, subtype: read_value(Kind::U8, input)?, readiness: read_value(Kind::U8, input)?, winning_side: read_value(Kind::U8, input)?, losing_side: read_value(Kind::U8, input)? }),
 { 0 => (province): U32, 1 => (result_hour): I32, 2 => (retire_hour): I32, 3 => (category): U32, 4 => (flag_a): U8, 5 => (flag_b): U8, 6 => (subtype): U8, 7 => (readiness): U8, 8 => (winning_side): U8, 9 => (losing_side): U8 }, |_row| Ok(()));

/// NavalResultMembers: key result:member (128 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalResultMemberRow {
    pub result: u64,
    pub member: u64,
    pub presence_value: u64,
    pub ordinal: u32,
    pub side: u8,
}
typed_codec!(NavalResultMemberRow, 128, |row| (row.result as u128) << 64 | row.member as u128, |key| Self { result: (key >> 64) as u64, member: key as u64, presence_value: Default::default(), ordinal: Default::default(), side: Default::default() },
 |key, input| Ok(Self { result: (key >> 64) as u64, member: key as u64, presence_value: read_value(Kind::U64, input)?, ordinal: read_value(Kind::U32, input)?, side: read_value(Kind::U8, input)? }),
 { 0 => (presence_value): U64, 1 => (ordinal): U32, 2 => (side): U8 }, |_row| Ok(()));

/// NavalResultCountries: key result:list:country (104 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalResultCountryRow {
    pub result: u64,
    pub country: u32,
    pub ordinal: u32,
    pub list: u8,
}
typed_codec!(NavalResultCountryRow, 104, |row| (row.result as u128) << 40 | (row.list as u128) << 32 | row.country as u128, |key| Self { result: (key >> 40) as u64, list: (key >> 32) as u8, country: key as u32, ordinal: Default::default() },
 |key, input| Ok(Self { result: (key >> 40) as u64, list: (key >> 32) as u8, country: key as u32, ordinal: read_value(Kind::U32, input)? }),
 { 0 => (ordinal): U32 }, |_row| Ok(()));

/// SunkShips: key singleton zero (0 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SunkShipsRow {
    pub entries: Vec<SunkShipEntry>,
}
typed_codec!(SunkShipsRow, 0, |_row| 0, |_key| Self { entries: Default::default() },
 |_key, input| Ok(Self { entries: read_value(Kind::Opaque, input)? }),
 { 0 => (entries): Opaque }, |_row| Ok(()));

/// ProjectPrograms: key id (32 bits), 6 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProjectProgramRow {
    pub id: u32,
    pub owner: u32,
    pub province: u32,
    pub active_project: Option<u32>,
    pub scientist: u64,
    pub unread_rewards: i32,
    pub building: Vec<u8>,
}
typed_codec!(ProjectProgramRow, 32, |row| row.id as u128, |key| Self { id: key as u32, owner: Default::default(), province: Default::default(), active_project: Default::default(), scientist: Default::default(), unread_rewards: Default::default(), building: Default::default() },
 |key, input| Ok(Self { id: key as u32, owner: read_value(Kind::U32, input)?, province: read_value(Kind::U32, input)?, active_project: read_value(Kind::OptionalU32, input)?, scientist: read_value(Kind::U64, input)?, unread_rewards: read_value(Kind::I32, input)?, building: read_value(Kind::Bytes, input)? }),
 { 0 => (owner): U32, 1 => (province): U32, 2 => (active_project): OptionalU32, 3 => (scientist): U64, 4 => (unread_rewards): I32, 5 => (building): Bytes }, |_row| Ok(()));

/// Projects: key owner:local_id (64 bits), 14 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProjectRow {
    pub owner: u32,
    pub local_id: u32,
    pub definition_id: u32,
    pub program_reference: u64,
    pub started: u8,
    pub completed: u8,
    pub state: ProjectState,
}
typed_codec!(ProjectRow, 64, |row| (row.owner as u128) << 32 | row.local_id as u128, |key| Self { owner: (key >> 32) as u32, local_id: key as u32, definition_id: Default::default(), program_reference: Default::default(), started: Default::default(), completed: Default::default(), state: Default::default() },
 |key, input| Ok(Self { owner: (key >> 32) as u32, local_id: key as u32, definition_id: read_value(Kind::U32, input)?, program_reference: read_value(Kind::U64, input)?, started: read_value(Kind::U8, input)?, completed: read_value(Kind::U8, input)?, state: <ProjectState as Value>::decode(input)? }),
 { 0 => (definition_id): U32, 1 => (program_reference): U64, 2 => (started): U8, 3 => (completed): U8, 4 => (state.current_offset): U32, 5 => (state.state_enums[0]): U32, 6 => (state.state_enums[1]): U32, 7 => (state.state_enums[2]): U32, 8 => (state.state_enums[3]): U32, 9 => (state.not_completed): U8, 10 => (state.phase_progress): I64, 11 => (state.phase_target): I32, 12 => (state.prototype_progress): I32, 13 => (state.iterations): I32 }, |_row| Ok(()));

/// ProjectHistory: key owner:project:ordinal (96 bits), 12 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProjectHistoryRow {
    pub owner: u32,
    pub project: u32,
    pub ordinal: u32,
    pub prototype_reward_handle: u32,
    pub option_handle: u32,
    pub date: i32,
    pub context: ProjectContext,
}
typed_codec!(ProjectHistoryRow, 96, |row| (row.owner as u128) << 64 | (row.project as u128) << 32 | row.ordinal as u128, |key| Self { owner: (key >> 64) as u32, project: (key >> 32) as u32, ordinal: key as u32, prototype_reward_handle: Default::default(), option_handle: Default::default(), date: Default::default(), context: Default::default() },
 |key, input| Ok(Self { owner: (key >> 64) as u32, project: (key >> 32) as u32, ordinal: key as u32, prototype_reward_handle: read_value(Kind::U32, input)?, option_handle: read_value(Kind::U32, input)?, date: read_value(Kind::I32, input)?, context: <ProjectContext as Value>::decode(input)? }),
 { 0 => (prototype_reward_handle): U32, 1 => (option_handle): U32, 2 => (date): I32, 3 => (context.province): I32, 4 => (context.progress): I32, 5 => (context.character_reference): U64, 6 => (context.country_present): U8, 7 => (context.country_reference): OptionalU32, 8 => (context.scientist_present): U8, 9 => (context.scientist_reference): OptionalU32, 10 => (context.state_present): U8, 11 => (context.state_reference): OptionalU32 }, |_row| Ok(()));

/// ProjectBreakthroughs: key owner:specialization (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProjectBreakthroughRow {
    pub owner: u32,
    pub specialization: u32,
    pub points: i32,
}
typed_codec!(ProjectBreakthroughRow, 64, |row| (row.owner as u128) << 32 | row.specialization as u128, |key| Self { owner: (key >> 32) as u32, specialization: key as u32, points: Default::default() },
 |key, input| Ok(Self { owner: (key >> 32) as u32, specialization: key as u32, points: read_value(Kind::I32, input)? }),
 { 0 => (points): I32 }, |_row| Ok(()));

/// Nukes: key country:nuke_type (64 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NukeRow {
    pub country: u32,
    pub nuke_type: u32,
    pub amount_raw: i64,
    pub ready: i32,
}
typed_codec!(NukeRow, 64, |row| (row.country as u128) << 32 | row.nuke_type as u128, |key| Self { country: (key >> 32) as u32, nuke_type: key as u32, amount_raw: Default::default(), ready: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, nuke_type: key as u32, amount_raw: read_value(Kind::I64, input)?, ready: read_value(Kind::I32, input)? }),
 { 0 => (amount_raw): I64, 1 => (ready): I32 }, |_row| Ok(()));

/// RaidCountries: key canonical_slot (32 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RaidCountryRow {
    pub canonical_slot: u32,
    pub country: i32,
    pub priority: i32,
    pub child_count: u32,
}
typed_codec!(RaidCountryRow, 32, |row| row.canonical_slot as u128, |key| Self { canonical_slot: key as u32, country: Default::default(), priority: Default::default(), child_count: Default::default() },
 |key, input| Ok(Self { canonical_slot: key as u32, country: read_value(Kind::I32, input)?, priority: read_value(Kind::I32, input)?, child_count: read_value(Kind::U32, input)? }),
 { 0 => (country): I32, 1 => (priority): I32, 2 => (child_count): U32 }, |_row| Ok(()));

/// Raids: key id:reference_type (64 bits), 13 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RaidRow {
    pub reference_type: u32,
    pub id: u32,
    pub parent_slot: u32,
    pub ordinal: u32,
    pub preparation_hours: i32,
    pub phase: i32,
    pub outcome: i32,
    pub end_date_present: u8,
    pub victim_country: i32,
    pub source: RaidSource,
}
typed_codec!(RaidRow, 64, |row| (row.id as u128) << 32 | row.reference_type as u128, |key| Self { id: (key >> 32) as u32, reference_type: key as u32, parent_slot: Default::default(), ordinal: Default::default(), preparation_hours: Default::default(), phase: Default::default(), outcome: Default::default(), end_date_present: Default::default(), victim_country: Default::default(), source: Default::default() },
 |key, input| Ok(Self { id: (key >> 32) as u32, reference_type: key as u32, parent_slot: read_value(Kind::U32, input)?, ordinal: read_value(Kind::U32, input)?, preparation_hours: read_value(Kind::I32, input)?, phase: read_value(Kind::I32, input)?, outcome: read_value(Kind::I32, input)?, end_date_present: read_value(Kind::U8, input)?, victim_country: read_value(Kind::I32, input)?, source: <RaidSource as Value>::decode(input)? }),
 { 0 => (parent_slot): U32, 1 => (ordinal): U32, 2 => (preparation_hours): I32, 3 => (phase): I32, 4 => (outcome): I32, 5 => (end_date_present): U8, 6 => (victim_country): I32, 7 => (source.kind): OptionalU32, 8 => (source.kind_present): U8, 9 => (source.province): U32, 10 => (source.ship_reference_type): U32, 11 => (source.ship_reference_id): U32, 12 => (source.country): I32 }, |_row| Ok(()));

/// PeaceConferences: key id (64 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PeaceConferenceRow {
    pub id: u64,
    pub ordinal: u32,
    pub current_turn: i32,
    pub finalized: u8,
}
typed_codec!(PeaceConferenceRow, 64, |row| row.id as u128, |key| Self { id: key as u64, ordinal: Default::default(), current_turn: Default::default(), finalized: Default::default() },
 |key, input| Ok(Self { id: key as u64, ordinal: read_value(Kind::U32, input)?, current_turn: read_value(Kind::I32, input)?, finalized: read_value(Kind::U8, input)? }),
 { 0 => (ordinal): U32, 1 => (current_turn): I32, 2 => (finalized): U8 }, |_row| Ok(()));

/// WeatherLimits: key singleton zero (0 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WeatherLimitsRow {
    pub max_water: i64,
    pub max_snow: i64,
}
typed_codec!(WeatherLimitsRow, 0, |_row| 0, |_key| Self { max_water: Default::default(), max_snow: Default::default() },
 |_key, input| Ok(Self { max_water: read_value(Kind::I64, input)?, max_snow: read_value(Kind::I64, input)? }),
 { 0 => (max_water): I64, 1 => (max_snow): I64 }, |_row| Ok(()));

/// WeatherRegions: key id (32 bits), 8 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WeatherRegionRow {
    pub id: u32,
    pub temperature: i64,
    pub next_change_hour: i32,
    pub light_rain: u8,
    pub heavy_rain: u8,
    pub snow: u8,
    pub blizzard: u8,
    pub sandstorm: u8,
    pub arctic_water: u8,
}
typed_codec!(WeatherRegionRow, 32, |row| row.id as u128, |key| Self { id: key as u32, temperature: Default::default(), next_change_hour: Default::default(), light_rain: Default::default(), heavy_rain: Default::default(), snow: Default::default(), blizzard: Default::default(), sandstorm: Default::default(), arctic_water: Default::default() },
 |key, input| Ok(Self { id: key as u32, temperature: read_value(Kind::I64, input)?, next_change_hour: read_value(Kind::I32, input)?, light_rain: read_value(Kind::U8, input)?, heavy_rain: read_value(Kind::U8, input)?, snow: read_value(Kind::U8, input)?, blizzard: read_value(Kind::U8, input)?, sandstorm: read_value(Kind::U8, input)?, arctic_water: read_value(Kind::U8, input)? }),
 { 0 => (temperature): I64, 1 => (next_change_hour): I32, 2 => (light_rain): U8, 3 => (heavy_rain): U8, 4 => (snow): U8, 5 => (blizzard): U8, 6 => (sandstorm): U8, 7 => (arctic_water): U8 }, |_row| Ok(()));

/// WeatherProvinces: key id (32 bits), 7 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WeatherProvinceRow {
    pub id: u32,
    pub water: i64,
    pub snow: i64,
    pub temperature: i64,
    pub temperature_change: i64,
    pub mud: u8,
    pub custom_mud: u8,
    pub custom_modifier_count: u32,
}
typed_codec!(WeatherProvinceRow, 32, |row| row.id as u128, |key| Self { id: key as u32, water: Default::default(), snow: Default::default(), temperature: Default::default(), temperature_change: Default::default(), mud: Default::default(), custom_mud: Default::default(), custom_modifier_count: Default::default() },
 |key, input| Ok(Self { id: key as u32, water: read_value(Kind::I64, input)?, snow: read_value(Kind::I64, input)?, temperature: read_value(Kind::I64, input)?, temperature_change: read_value(Kind::I64, input)?, mud: read_value(Kind::U8, input)?, custom_mud: read_value(Kind::U8, input)?, custom_modifier_count: read_value(Kind::U32, input)? }),
 { 0 => (water): I64, 1 => (snow): I64, 2 => (temperature): I64, 3 => (temperature_change): I64, 4 => (mud): U8, 5 => (custom_mud): U8, 6 => (custom_modifier_count): U32 }, |_row| Ok(()));

/// OperationTokens: key owner:target (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OperationTokensRow {
    pub owner: u32,
    pub target: u32,
    pub tokens: Vec<u32>,
}
typed_codec!(OperationTokensRow, 64, |row| (row.owner as u128) << 32 | row.target as u128, |key| Self { owner: (key >> 32) as u32, target: key as u32, tokens: Default::default() },
 |key, input| Ok(Self { owner: (key >> 32) as u32, target: key as u32, tokens: read_value(Kind::U32List, input)? }),
 { 0 => (tokens): U32List }, |_row| Ok(()));

/// LexicalTokens: key token (32 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LexicalTokenRow {
    pub token: u32,
    pub literal: Vec<u8>,
}
typed_codec!(LexicalTokenRow, 32, |row| row.token as u128, |key| Self { token: key as u32, literal: Default::default() },
 |key, input| Ok(Self { token: key as u32, literal: read_value(Kind::Bytes, input)? }),
 { 0 => (literal): Bytes }, |_row| Ok(()));

/// NavalSupremacy: key region:country (64 bits), 7 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalSupremacyRow {
    pub region: u32,
    pub region_slot: u32,
    pub country: u32,
    pub current: i64,
    pub base_target: i64,
    pub target: i64,
    pub individual_ratio: i64,
    pub previous: i64,
    pub decline_from: i64,
}
typed_codec!(NavalSupremacyRow, 64, |row| (row.region as u128) << 32 | row.country as u128, |key| Self { region: (key >> 32) as u32, country: key as u32, region_slot: Default::default(), current: Default::default(), base_target: Default::default(), target: Default::default(), individual_ratio: Default::default(), previous: Default::default(), decline_from: Default::default() },
 |key, input| Ok(Self { region: (key >> 32) as u32, country: key as u32, region_slot: read_value(Kind::U32, input)?, current: read_value(Kind::I64, input)?, base_target: read_value(Kind::I64, input)?, target: read_value(Kind::I64, input)?, individual_ratio: read_value(Kind::I64, input)?, previous: read_value(Kind::I64, input)?, decline_from: read_value(Kind::I64, input)? }),
 { 0 => (region_slot): U32, 1 => (current): I64, 2 => (base_target): I64, 3 => (target): I64, 4 => (individual_ratio): I64, 5 => (previous): I64, 6 => (decline_from): I64 }, |_row| Ok(()));

/// RadarCoverage: key country:ordinal (64 bits), 2 payload mask bits.
///
/// One radar coverage group of `CCountry+0x1120`. `ordinal` is the native vector
/// persistent group; `state` is the group's host CState id at `+0x48`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RadarCoverageRow {
    /// Raw country reference; resolve through CountryLinks to a canonical slot.
    pub country: u32,
    pub ordinal: u32,
    /// Host state id; absent when the native `+0x48` pointer is null.
    pub state: Option<u32>,
    /// Covered province ids in native array order, repeats included.
    pub provinces: Vec<u32>,
}
typed_codec!(RadarCoverageRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, state: Default::default(), provinces: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, ordinal: key as u32, state: read_value(Kind::OptionalU32, input)?, provinces: read_value(Kind::U32List, input)? }),
 { 0 => (state): OptionalU32, 1 => (provinces): U32List }, |_row| Ok(()));

/// VolunteerHosts: key country:position (64 bits), 2 payload mask bits.
///
/// One position of `CCountry+0x1428` and its paired `+0x1440` tally. The position
/// is part of the key because the vector can hold a repeated host.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VolunteerHostRow {
    /// Raw country reference of the country that owns the vector.
    pub country: u32,
    pub position: u32,
    /// Raw country reference exactly as the native vector stores it.
    pub host: u32,
    /// Paired native word; the engine re-derives it from the sender's armies.
    pub deployed_divisions: i32,
}
typed_codec!(VolunteerHostRow, 64, |row| (row.country as u128) << 32 | row.position as u128, |key| Self { country: (key >> 32) as u32, position: key as u32, host: Default::default(), deployed_divisions: Default::default() },
 |key, input| Ok(Self { country: (key >> 32) as u32, position: key as u32, host: read_value(Kind::U32, input)?, deployed_divisions: read_value(Kind::I32, input)? }),
 { 0 => (host): U32, 1 => (deployed_divisions): I32 }, |_row| Ok(()));

/// TheatreAreas: key theatre:ordinal (96 bits), 2 payload mask bits.
///
/// One CControllerArea listed by a volunteer CTheatre at `+0x18`. `representative`
/// is the province behind the area's `+0x28`, which is the only member the save
/// writer emits; `provinces` is the area's whole `+0x30` membership.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TheatreAreaRow {
    /// Full native CTheatre handle; join it to LandTheatres.
    pub theatre: u64,
    pub ordinal: u32,
    /// Representative province id; absent when the native pointer is null.
    pub representative: Option<u32>,
    /// Member province ids in native array order, repeats included.
    pub provinces: Vec<u32>,
}
typed_codec!(TheatreAreaRow, 96, |row| (row.theatre as u128) << 32 | row.ordinal as u128, |key| Self { theatre: (key >> 32) as u64, ordinal: key as u32, representative: Default::default(), provinces: Default::default() },
 |key, input| Ok(Self { theatre: (key >> 32) as u64, ordinal: key as u32, representative: read_value(Kind::OptionalU32, input)?, provinces: read_value(Kind::U32List, input)? }),
 { 0 => (representative): OptionalU32, 1 => (provinces): U32List }, |_row| Ok(()));

/// StationedUnits: key theatre:ordinal (96 bits), 1 payload mask bit.
///
/// One position of a volunteer CTheatre's `+0x68` stationed-unit vector.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StationedUnitRow {
    /// Full native CTheatre handle; join it to LandTheatres.
    pub theatre: u64,
    pub ordinal: u32,
    /// Full native CUnit handle; join it to Divisions or TaskForces.
    pub unit: u64,
}
typed_codec!(StationedUnitRow, 96, |row| (row.theatre as u128) << 32 | row.ordinal as u128, |key| Self { theatre: (key >> 32) as u64, ordinal: key as u32, unit: Default::default() },
 |key, input| Ok(Self { theatre: (key >> 32) as u64, ordinal: key as u32, unit: read_value(Kind::U64, input)? }),
 { 0 => (unit): U64 }, |_row| Ok(()));

/// UnitIntel: key unit:ordinal (96 bits), 2 payload mask bits.
///
/// One entry of a unit's `CUnit+0x278` per-viewer level array, in native order.
/// The entry's `+8` decay accumulator is deliberately not recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UnitIntelRow {
    /// Full native CUnit handle; join it to Divisions or TaskForces.
    pub unit: u64,
    pub ordinal: u32,
    /// Stored viewer key: the country's ordinal in the engine's country array.
    pub viewer: u32,
    /// Native signed level byte; nonpositive entries are recorded as stored.
    pub level: i8,
}
typed_codec!(UnitIntelRow, 96, |row| (row.unit as u128) << 32 | row.ordinal as u128, |key| Self { unit: (key >> 32) as u64, ordinal: key as u32, viewer: Default::default(), level: Default::default() },
 |key, input| Ok(Self { unit: (key >> 32) as u64, ordinal: key as u32, viewer: read_value(Kind::U32, input)?, level: read_value(Kind::I8, input)? }),
 { 0 => (viewer): U32, 1 => (level): I8 }, |_row| Ok(()));

/// AirBaseRegistrations: key base:slot (96 bits), 2 payload mask bits.
///
/// One country's registered pool membership at one air base. `slot` is the index
/// in `CAirBase+0x30`, which the engine fills at the country's resolved ordinal.
/// Wings are flattened across the container's pools in pool order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AirBaseRegistrationRow {
    /// Full native CAirBase handle; join it to AirBases.
    pub base: u64,
    pub slot: u32,
    /// Raw country reference stored by the container itself.
    pub country: u32,
    /// Full native CAirWing handles in native order, repeats included.
    pub wings: Vec<u64>,
}
typed_codec!(AirBaseRegistrationRow, 96, |row| (row.base as u128) << 32 | row.slot as u128, |key| Self { base: (key >> 32) as u64, slot: key as u32, country: Default::default(), wings: Default::default() },
 |key, input| Ok(Self { base: (key >> 32) as u64, slot: key as u32, country: read_value(Kind::U32, input)?, wings: read_value(Kind::U64List, input)? }),
 { 0 => (country): U32, 1 => (wings): U64List }, |_row| Ok(()));

// Variable records borrow their typed values during encoding and allocate only on decode.
opaque_value!(Option<Vec<u8>>);
opaque_value!(Vec<i64>);
opaque_value!(Vec<ScriptVariableEntry>);
opaque_value!(Vec<AirHistoryEntry>);
opaque_value!(Vec<AirDamageEntry>);
opaque_value!(Option<AceDeath>);
opaque_value!(Vec<SunkShipEntry>);

#[cfg(test)]
pub(crate) use fixtures::sample_extended_table;

#[cfg(test)]
mod fixtures {
    use super::*;
    use std::fmt::Debug;
    trait Sample {
        fn sample(tick: usize) -> Self;
    }
    macro_rules! signed_samples {
        ($($ty:ty),+) => { $(impl Sample for $ty {
            fn sample(tick: usize) -> Self { match tick % 3 { 0 => <$ty>::MIN, 1 => -1, _ => <$ty>::MAX } }
        })+ };
    }
    macro_rules! unsigned_samples {
        ($($ty:ty),+) => { $(impl Sample for $ty {
            fn sample(tick: usize) -> Self { match tick % 3 { 0 => 0, 1 => 1, _ => <$ty>::MAX } }
        })+ };
    }
    signed_samples!(i8, i16, i32, i64);
    unsigned_samples!(u8, u16, u32, u64);
    impl Sample for bool {
        fn sample(tick: usize) -> Self {
            tick.is_multiple_of(2)
        }
    }
    impl<T: Sample> Sample for Option<T> {
        fn sample(tick: usize) -> Self {
            if tick.is_multiple_of(4) {
                None
            } else {
                Some(T::sample(tick))
            }
        }
    }
    impl<T: Sample> Sample for Vec<T> {
        fn sample(tick: usize) -> Self {
            (0..tick % 3).map(|index| T::sample(tick + index)).collect()
        }
    }
    impl<T: Sample, const N: usize> Sample for [T; N] {
        fn sample(tick: usize) -> Self {
            std::array::from_fn(|index| T::sample(tick + index))
        }
    }
    macro_rules! sample_struct {
        ($ty:ty { $($field:ident : $field_ty:ty),* $(,)? }) => {
            impl Sample for $ty { fn sample(tick: usize) -> Self { Self { $($field: <$field_ty as Sample>::sample(tick),)* } } }
        };
    }
    trait Fixture: RowCodec + Debug {
        fn fixture(key: u128, tick: usize) -> Self;
    }
    macro_rules! fixture {
        ($ty:ident, |$key:ident| { $($key_field:ident: $key_value:expr),* $(,)? },
         { $($field:ident: $field_ty:ty),+ $(,)? }) => {
            impl Fixture for $ty {
                fn fixture($key: u128, tick: usize) -> Self {
                    Self { $($key_field: $key_value,)* $($field: <$field_ty as Sample>::sample(tick),)+ }
                }
            }
        };
    }
    sample_struct!(ProjectState {
        current_offset: u32,
        state_enums: [u32; 4],
        not_completed: u8,
        phase_progress: i64,
        phase_target: i32,
        prototype_progress: i32,
        iterations: i32
    });
    sample_struct!(ProjectContext { province: i32, progress: i32, character_reference: u64, country_present: u8, country_reference: Option<u32>, scientist_present: u8, scientist_reference: Option<u32>, state_present: u8, state_reference: Option<u32> });
    sample_struct!(RaidSource { kind: Option<u32>, kind_present: u8, province: u32, ship_reference_type: u32, ship_reference_id: u32, country: i32 });
    sample_struct!(ScriptVariableEntry { name: Vec<u8>, value_raw: i64 });
    sample_struct!(AceDeath {
        killer_ace: u64,
        killer_country: u32
    });
    sample_struct!(AirHistoryEntry {
        side: u8,
        date: i32,
        wing_id: u32,
        classification: u8,
        region_owner: u32,
        planes: i32,
        mission_mask: u32,
        selection: u8,
        copied: u8,
        attributed_country: u32
    });
    sample_struct!(AirDamageEntry {
        side: u8,
        date: i32,
        wing_id: u32,
        classification: u8,
        channel: u8,
        cause: u32,
        amount: i64
    });
    sample_struct!(SunkShipEntry { ship_name: Vec<u8>, attribution_name: Vec<u8>, class_name: Vec<u8>, attribution_class_name: Vec<u8>, victim_country: u32, attacker_country: u32, sink_hour: i32, class_token: u32, attribution_class_token: u32, attribution_class_present: u8, model_level: i32, victim_equipment_variant: u64, damage_source_reference: u64, province: u32, province_present: u8, battle: u64, cause_record: u8, assisted_kill: u8 });
    fixture!(FocusRow, |key| {country: key as u32}, {tree: Option<Vec<u8>>, current: Option<Vec<u8>>, continuous: Option<Vec<u8>>, progress: i64, paused: u8, completed_count: i32});
    fixture!(CompletedFocusRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {token: Vec<u8>});
    fixture!(ResearchTechnologyRow, |key| {country: (key >> 32) as u32, technology_handle: key as u32}, {classifier: u32, level: i32, researched: u8, progress_raw: i64, ahead_reduction_years_raw: i64});
    fixture!(ResearchSlotRow, |key| {country: (key >> 32) as u32, index: key as u32}, {technology_handle: u32, technology_present: u8, research_speed_raw: i64, banked_days_raw: i64, applied_days_raw: i64});
    fixture!(TechnologyDefinitionRow, |key| {technology_handle: key as u32}, {database_index: i32, year: i32, max_level: i32, research_cost_raw: i64, id: Vec<u8>});
    fixture!(PoliticsRow, |key| {country: key as u32}, {last_election: i32, election_frequency: i32, elections_allowed: u8, capital: i32, ruling_party: Option<u32>});
    fixture!(PoliticalPartyRow, |key| {country: (key >> 32) as u32, id: key as u32}, {popularity: i64, is_ruling: u8, token: Vec<u8>});
    fixture!(ActiveIdeaRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {token: Vec<u8>});
    fixture!(DecisionCountsRow, |key| {country: key as u32}, {taken: u32, re_enable: u32, remove: u32, timed: u32, targeted_timed: u32, targeted: u32});
    fixture!(DecisionRow, |key| {country: (key >> 40) as u32, container: (key >> 32) as u8, ordinal: key as u32}, {token: Vec<u8>, days: i32, lifecycle: i32, target_kind: u8, country_target: u32, numeric_target: u32, ignore: u8});
    fixture!(DynamicModifierRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {scope_country: u32, scope_state: i32, days_remaining: i32, enabled: u8, token: Vec<u8>, values: Vec<i64>});
    fixture!(PreferredTacticRow, |key| {country: key as u32}, {tactic_id: i32, token: Vec<u8>});
    fixture!(PreferredTacticOperandsRow, |key| {country: key as u32}, {tactic_id: i32, modifier_key: u32, metadata_flags: u32, metadata_gate: i32, country_factor_raw: i64, accumulator_raw: i64, accumulator_entry_present: u8});
    fixture!(TechSharingGroupRow, |key| {id: key as u32}, {base_bonus: i64, member_count: u32});
    fixture!(TechSharingMemberRow, |key| {group: (key >> 32) as u32, ordinal: key as u32}, {country: u32, bonus: i64});
    fixture!(PowerBalanceRow, |key| {ordinal: key as u32}, {value_raw: i64, balance: Vec<u8>, left: Vec<u8>, right: Vec<u8>, trending: Vec<u8>, active_range: Vec<u8>});
    fixture!(PowerBalanceMemberRow, |key| {record: (key >> 32) as u32, ordinal: key as u32}, {country: u32, daily_raw: i64, weekly_raw: i64, daily_present: bool, weekly_present: bool, daily_flags: u32, weekly_flags: u32});
    fixture!(PowerBalanceRangeRow, |key| {record: (key >> 32) as u32, ordinal: key as u32}, {identity: Vec<u8>});
    fixture!(PowerBalanceOverrideRow, |key| {record: (key >> 32) as u32, ordinal: key as u32}, {side: Vec<u8>, gfx: Vec<u8>});
    fixture!(ProductionLineRow, |key| {handle: key as u64}, {country: u32, kind: u8, variant: u64, mio: u64, active_factories: i32, requested_factories: i32, cost_raw: i64, speed_raw: i64, produced_raw: i64, priority: i32, amount: i32});
    fixture!(StockpileRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {variant_handle: u64, amount_raw: i64});
    fixture!(DivisionEquipmentRow, |key| {division: (key >> 32) as u64, ordinal: key as u32}, {template: u64, country: u32, variant_handle: u64, amount_raw: i64});
    fixture!(VariantDefinitionRow, |key| {id: key as u64}, {equipment_token: u32, creator_link: u32, origin_link: u32, append_variant_name: u8, obsolete: u8, classifier_flags: u64, mission_mask: u32, display_name: Vec<u8>, variant_name: Vec<u8>, equipment_name: Vec<u8>});
    fixture!(VariantModuleRow, |key| {variant: (key >> 32) as u64, source_ordinal: key as u32}, {slot: u32, definition_token: u32, definition_active: u8, definition_name: Vec<u8>});
    fixture!(VariantStatsRow, |key| {variant: key as u64}, {values: [i64;10]});
    fixture!(VariantUpgradeRow, |key| {variant: (key >> 32) as u64, source_ordinal: key as u32}, {definition_token: i32, level: u8, definition_name: Vec<u8>});
    fixture!(ArmyUpgradeGroupRow, |key| {army: key as u64}, {country: u32, priority: u32});
    fixture!(ArmyUpgradeRequestRow, |key| {army: (key >> 32) as u64, ordinal: key as u32}, {country: u32, equipment: u64, request_raw: i64, total_raw: i64});
    fixture!(ArmyUpgradeDeliveryRow, |key| {army: (key >> 32) as u64, ordinal: key as u32}, {country: u32, status: u32, progress_raw: Option<i64>, total_progress_raw: i64, allow_zero_entries: u8});
    fixture!(ArmyUpgradeProducedRow, |key| {army: (key >> 64) as u64, delivery_ordinal: (key >> 32) as u32, ordinal: key as u32}, {country: u32, equipment: u64, amount_raw: i64});
    fixture!(CountryFuelRow, |key| {country: key as u32}, {current_raw: i64, maximum_raw: i64, gain_raw: i64, cost_raw: i64});
    fixture!(ResourceDefinitionRow, |key| {id: key as u32}, {token: u32, active: u8, factory_cost: i64});
    fixture!(CountryResourceRow, |key| {country: (key >> 32) as u32, resource: key as u32}, {produced: i64, adjustment: i64, imported: i64, used: i64, special_total: i64, production_demand: i64, to_export: i64, base_export: i64, exported: i64});
    fixture!(ResourceExchangeRow, |key| {id: key as u64}, {exporter: u32, importer: u32, resource: u32, import_ordinal: u32, export_ordinal: u32, convoys_assigned: i32, convoys_required: i32, efficiency_a: i64, efficiency_b: i64, delivered: i64, required_factories: i32, assigned_factories: i32, no_trade_influence: u8, no_resources: u8});
    fixture!(PurchaseContractRow, |key| {id: key as u64}, {seller: u32, buyer: u32, speed: i32, convoys_assigned: i32, convoys_total: i32, route_efficiency_sum_raw: i64, route_days: i32, factory_cic_progress_raw: i64, equipment_transfer_progress_raw: i64, collected_raw: i64, completed_deliveries: i32});
    fixture!(ProvinceSupplyRow, |key| {province: key as u32}, {supply_factor_raw: i64, railway_health_raw: i64, controller_index: u16, capital_country_index: u16, supply_node_level: u8, naval_base_level: u8, non_damaged_infra_level: u8, infra_level: u8, flags: u8});
    fixture!(HubMotorizationRow, |key| {owner: (key >> 96) as u32, province: (key >> 64) as u32 as i32, node_type: (key >> 32) as u32 as i32, actor: key as u32}, {node_index: u32, map_entry_index: u32, level: u8});
    fixture!(DisruptedSupplyRow, |key| {country: (key >> 64) as u32, province: (key >> 32) as u32, node_type: key as u32}, {value: i64});
    fixture!(ForeignHomebaseRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {province: u32, supply: i64, start: i64, penalty: i64, added_penalty: i64, duration: i64, hours: i64, decay: i64, base: u8, source: Vec<u8>});
    fixture!(CountryExperienceRow, |key| {country: key as u32}, {army_raw: i64, army_daily_raw: i64, army_daily_training_raw: i64, navy_raw: i64, navy_daily_raw: i64, navy_daily_training_raw: i64, air_raw: i64, air_daily_raw: i64, armies_for_training_raw: i64});
    fixture!(BuildingRow, |key| {location: (key >> 32) as u32, building_type: key as u32}, {state: u32, ordinal: u32, status_flags: u32, country: u32, level: i16, healthy_level: i16, partial_health: i64});
    fixture!(StateBaseResourceRow, |key| {state: (key >> 32) as u32, resource: key as u32}, {ordinal: u32, amount: i64});
    fixture!(BuildingConstructionRow, |key| {id: key as u64}, {country: u32, queue_ordinal: u32, active_factories: i32, cost: i64, speed: i64, produced: i64, priority: i32, amount: i32, created_hour: i32, building_type: u32, target_flags: u32, target_location: u32, target_state: u32, target_country: u32, repair_levels: i32});
    fixture!(RailwayConstructionRow, |key| {id: key as u64}, {country: u32, queue_ordinal: u32, active_factories: i32, cost: i64, speed: i64, produced: i64, priority: i32, amount: i32, created_hour: i32, base_level: i32, target_level: i32, cancelled: u8, current_segment: i32, remaining_cost: i64, route: Vec<u32>});
    fixture!(OccupationSettingsRow, |key| {country: key as u32}, {garrison_template: Option<u64>, default_law_present: u8, law: Vec<u8>});
    fixture!(StateOccupationLawRow, |key| {controller: (key >> 64) as u32, occupied: (key >> 32) as u32, state: key as u32}, {name: Vec<u8>});
    fixture!(StateGarrisonRow, |key| {controller: (key >> 32) as u32, state: key as u32}, {occupied: u32, required_manpower: i32, strength_ratio: i64, garrison_required: i64, serialized_flag: u8});
    fixture!(StateAssignedManpowerRow, |key| {controller: (key >> 64) as u32, state: (key >> 32) as u32, ordinal: key as u32}, {occupied: u32, source_country: u32, amount: i32});
    fixture!(CountryOccupationReserveManpowerRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {source_country: u32, amount: i32});
    fixture!(StateResistanceRow, |key| {state: key as u32}, {country: u32, level: i64, speed: i64, compliance: i64, compliance_speed: i64, active: u8});
    fixture!(CountryCollaborationRow, |key| {source: (key >> 32) as u32, target: key as u32}, {progress: i64});
    fixture!(CharacterRow, |key| {id: key as u32}, {country: u32, name_token_id: i32, roster: u8, ordinal: u32, name: Vec<u8>, token: Vec<u8>});
    fixture!(CountryLeaderRoleRow, |key| {character: (key >> 32) as u32, ideology: key as u32 as i32}, {country: u32});
    fixture!(AdvisorRoleRow, |key| {character: (key >> 32) as u32, ordinal: key as u32}, {country: u32, key: Vec<u8>, idea: Vec<u8>, slot: Vec<u8>});
    fixture!(UnitLeaderRow, |key| {id: (key >> 32) as u32, reference_type: key as u32}, {character_type: u32, character_id: u32, category: i32, experience: i64, kind: u8, skill_status: u8, skill_valid: u8, skill_level: i32, skill_required_experience: i32, skill_category: i32, components: [i32;4], inherited_text_a: Vec<u8>, inherited_text_b: Vec<u8>});
    fixture!(UnitLeaderTraitRow, |key| {leader_reference: (key >> 32) as u64, ordinal: key as u32}, {token_id: i32, definition_ordinal: i32, token: Vec<u8>});
    fixture!(UnitLeaderTraitExperienceRow, |key| {leader_reference: (key >> 32) as u64, ordinal: key as u32}, {token_id: i32, definition_ordinal: i32, token: Vec<u8>, experience: i64});
    fixture!(ActiveAbilityRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {scope_country: u32, token: Vec<u8>, caster: Option<u64>, strategic_region: Option<u32>, division_count: i32, activation_hour: i32, phase: u8, allocate_cost: u8, allocated_command_power: i64, duration_hours: i32, cooldown_hours: i32, target_kind: i32});
    fixture!(CapitulationRow, |key| {country: key as u32}, {capitulated: u8, capitulated_hour: i32, last_surrender_hour: i32, faction_join_hour: i32, legitimacy_raw: i64, exile_army_leaders: i32});
    fixture!(SubjectRow, |key| {overlord: (key >> 32) as u32, subject: key as u32}, {start_hour: i32, end_hour: i32, cancel: u8, autonomy_ratio: i64, state: Vec<u8>});
    fixture!(AutonomyRow, |key| {subject: key as u32}, {score: i64, lower_score: i64, upper_score: i64, last_modified_bits: u32, state: Vec<u8>, previous: Option<Vec<u8>>, next: Option<Vec<u8>>});
    fixture!(IntelligenceAgencyRow, |key| {country: key as u32}, {upgrade_progress_raw: i64, defense_raw: i64, active_count: i32, admission_ceiling: i32, unknown_f0: u32});
    fixture!(OperativeMembershipRow, |key| {id: (key >> 40) as u64, roster_country: (key >> 8) as u32, roster_kind: key as u8}, {owner_country: u32, detected: u8, assignment: u32, experience_raw: i64, skill: Option<i32>, capturer_country: u32, captured_hour: i32, action_variant: Option<u32>});
    fixture!(CryptologyRow, |key| {owner_country: (key >> 32) as u32, target_country: key as u32}, {amount_raw: i64, bonus_days: i32, bonus_active: u8, decrypting: u8});
    fixture!(IntelNetworkRow, |key| {owner_country: (key >> 64) as u32, target_country: (key >> 32) as u32, state_id: key as u32 as i32}, {strength_raw: Option<i64>, subnetwork_selector: i32});
    fixture!(ScriptFlagRow, |key| {scope_kind: (key >> 64) as u8, scope_id: (key >> 32) as u32, token: key as u32}, {set_hour: i32, value: i16, remaining_days: i16});
    fixture!(ScriptVariablesRow, |key| {scope_kind: (key >> 32) as u8, scope_id: key as u32}, {entries: Vec<ScriptVariableEntry>});
    fixture!(OperationRow, |key| {id: key as u64}, {owner: u32, target: u32, target_province: Option<u32>, definition_token: u32, preparation_current: i64, preparation_required: i64, flag_40: u8, auto_repeat: u8, terminal: u8, start_hour: i32, duration_days: i32, phase_count: u32, assigned_slots: u32});
    fixture!(OperationAssignmentRow, |key| {operation: (key >> 32) as u64, ordinal: key as u32}, {operative: u64, selection: u32});
    fixture!(WingEquipmentRow, |key| {wing: (key >> 32) as u64, ordinal: key as u32}, {variant: u64, amount_raw: i64});
    fixture!(AirBaseRow, |key| {id: key as u64}, {state: Option<u32>, province: Option<u32>, host: u64, capacity: i32, kind: i32});
    fixture!(WingCumulativeRow, |key| {id: key as u64}, {accidents: i32, experience: i64});
    fixture!(AirRegionHistoryRow, |key| {country: (key >> 32) as u32, region: key as u32}, {entries: Vec<AirHistoryEntry>});
    fixture!(AirRegionDamageRow, |key| {country: (key >> 32) as u32, region: key as u32}, {entries: Vec<AirDamageEntry>});
    fixture!(AirAceRow, |key| {id: key as u64}, {country: u32, ordinal: u32, aircraft_type_mask: u32, assigned_wing: u64, alive: u8, handled: u8, kill_type: u32, death: Option<AceDeath>, name: Vec<u8>, surname: Vec<u8>, callsign: Vec<u8>});
    fixture!(AirStatisticsRow, |key| {country: (key >> 32) as u32, region: key as u32}, {year_slot: u32, year_window_wrapped: u8, channels: [i64;24]});
    fixture!(SunkConvoyRow, |key| {month: (key >> 64) as u32 as i32, killer_country: (key >> 32) as u32, owner_country: key as u32}, {convoys: i32, ordinal: u32});
    fixture!(RailCooldownRow, |key| {province: key as u32}, {cooldown_ticks: i32});
    fixture!(RailEdgeRow, |key| {from: (key >> 32) as u32, to: key as u32}, {level: i32});
    fixture!(LandTheatreRow, |key| {id: key as u64}, {owner: u32, ordinal: u32, volunteers_theatre: i32});
    fixture!(LandTheatreGroupRow, |key| {id: key as u64}, {theatre: u64, ordinal: u32, priority: i32, name: Vec<u8>});
    fixture!(LandTheatreMemberRow, |key| {theater_group: (key >> 64) as u64, orders_group: key as u64}, {ordinal: u32});
    fixture!(AirTheatreRow, |key| {id: key as u64}, {owner: u32, ordinal: u32, name: Vec<u8>});
    fixture!(AirTheatreGroupRow, |key| {id: key as u64}, {theatre: u64, ordinal: u32, name: Vec<u8>});
    fixture!(AirTheatreMemberRow, |key| {group: (key >> 64) as u64, wing: key as u64}, {ordinal: u32});
    fixture!(NavalResultRow, |key| {id: key as u64}, {province: u32, result_hour: i32, retire_hour: i32, category: u32, flag_a: u8, flag_b: u8, subtype: u8, readiness: u8, winning_side: u8, losing_side: u8});
    fixture!(NavalResultMemberRow, |key| {result: (key >> 64) as u64, member: key as u64}, {presence_value: u64, ordinal: u32, side: u8});
    fixture!(NavalResultCountryRow, |key| {result: (key >> 40) as u64, list: (key >> 32) as u8, country: key as u32}, {ordinal: u32});
    fixture!(SunkShipsRow, |_key| {}, {entries: Vec<SunkShipEntry>});
    fixture!(ProjectProgramRow, |key| {id: key as u32}, {owner: u32, province: u32, active_project: Option<u32>, scientist: u64, unread_rewards: i32, building: Vec<u8>});
    fixture!(ProjectRow, |key| {owner: (key >> 32) as u32, local_id: key as u32}, {definition_id: u32, program_reference: u64, started: u8, completed: u8, state: ProjectState});
    fixture!(ProjectHistoryRow, |key| {owner: (key >> 64) as u32, project: (key >> 32) as u32, ordinal: key as u32}, {prototype_reward_handle: u32, option_handle: u32, date: i32, context: ProjectContext});
    fixture!(ProjectBreakthroughRow, |key| {owner: (key >> 32) as u32, specialization: key as u32}, {points: i32});
    fixture!(NukeRow, |key| {country: (key >> 32) as u32, nuke_type: key as u32}, {amount_raw: i64, ready: i32});
    fixture!(RaidCountryRow, |key| {canonical_slot: key as u32}, {country: i32, priority: i32, child_count: u32});
    fixture!(RaidRow, |key| {id: (key >> 32) as u32, reference_type: key as u32}, {parent_slot: u32, ordinal: u32, preparation_hours: i32, phase: i32, outcome: i32, end_date_present: u8, victim_country: i32, source: RaidSource});
    fixture!(PeaceConferenceRow, |key| {id: key as u64}, {ordinal: u32, current_turn: i32, finalized: u8});
    fixture!(WeatherLimitsRow, |_key| {}, {max_water: i64, max_snow: i64});
    fixture!(WeatherRegionRow, |key| {id: key as u32}, {temperature: i64, next_change_hour: i32, light_rain: u8, heavy_rain: u8, snow: u8, blizzard: u8, sandstorm: u8, arctic_water: u8});
    fixture!(WeatherProvinceRow, |key| {id: key as u32}, {water: i64, snow: i64, temperature: i64, temperature_change: i64, mud: u8, custom_mud: u8, custom_modifier_count: u32});
    fixture!(OperationTokensRow, |key| {owner: (key >> 32) as u32, target: key as u32}, {tokens: Vec<u32>});
    fixture!(LexicalTokenRow, |key| {token: key as u32}, {literal: Vec<u8>});
    fixture!(NavalSupremacyRow, |key| {region: (key >> 32) as u32, country: key as u32}, {region_slot: u32, current: i64, base_target: i64, target: i64, individual_ratio: i64, previous: i64, decline_from: i64});
    fixture!(RadarCoverageRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {state: Option<u32>, provinces: Vec<u32>});
    fixture!(VolunteerHostRow, |key| {country: (key >> 32) as u32, position: key as u32}, {host: u32, deployed_divisions: i32});
    fixture!(TheatreAreaRow, |key| {theatre: (key >> 32) as u64, ordinal: key as u32}, {representative: Option<u32>, provinces: Vec<u32>});
    fixture!(StationedUnitRow, |key| {theatre: (key >> 32) as u64, ordinal: key as u32}, {unit: u64});
    fixture!(UnitIntelRow, |key| {unit: (key >> 32) as u64, ordinal: key as u32}, {viewer: u32, level: i8});
    fixture!(AirBaseRegistrationRow, |key| {base: (key >> 32) as u64, slot: key as u32}, {country: u32, wings: Vec<u64>});

    fn maximum_key<T: RowCodec>() -> u128 {
        if T::KEY_BITS == 128 {
            u128::MAX
        } else {
            (1_u128 << T::KEY_BITS) - 1
        }
    }
    fn rows<T: Fixture>(tick: usize) -> Vec<T> {
        let mut rows = vec![T::fixture(0, tick)];
        if T::KEY_BITS != 0 {
            rows.push(T::fixture(1, tick + 1));
            if tick % 17 < 5 {
                rows.push(T::fixture(maximum_key::<T>(), tick + 2));
            }
        }
        rows
    }
    pub(crate) fn sample_extended_table(table: TableId, tick: usize) -> Option<TableRows> {
        Some(match table {
            TableId::Focus => TableRows::Focus(rows::<FocusRow>(tick)),
            TableId::CompletedFocuses => {
                TableRows::CompletedFocuses(rows::<CompletedFocusRow>(tick))
            }
            TableId::ResearchTechnologies => {
                TableRows::ResearchTechnologies(rows::<ResearchTechnologyRow>(tick))
            }
            TableId::ResearchSlots => TableRows::ResearchSlots(rows::<ResearchSlotRow>(tick)),
            TableId::TechnologyDefinitions => {
                TableRows::TechnologyDefinitions(rows::<TechnologyDefinitionRow>(tick))
            }
            TableId::Politics => TableRows::Politics(rows::<PoliticsRow>(tick)),
            TableId::PoliticalParties => {
                TableRows::PoliticalParties(rows::<PoliticalPartyRow>(tick))
            }
            TableId::ActiveIdeas => TableRows::ActiveIdeas(rows::<ActiveIdeaRow>(tick)),
            TableId::DecisionCounts => TableRows::DecisionCounts(rows::<DecisionCountsRow>(tick)),
            TableId::Decisions => TableRows::Decisions(rows::<DecisionRow>(tick)),
            TableId::DynamicModifiers => {
                TableRows::DynamicModifiers(rows::<DynamicModifierRow>(tick))
            }
            TableId::PreferredTactics => {
                TableRows::PreferredTactics(rows::<PreferredTacticRow>(tick))
            }
            TableId::PreferredTacticOperands => {
                TableRows::PreferredTacticOperands(rows::<PreferredTacticOperandsRow>(tick))
            }
            TableId::TechSharingGroups => {
                TableRows::TechSharingGroups(rows::<TechSharingGroupRow>(tick))
            }
            TableId::TechSharingMembers => {
                TableRows::TechSharingMembers(rows::<TechSharingMemberRow>(tick))
            }
            TableId::PowerBalances => TableRows::PowerBalances(rows::<PowerBalanceRow>(tick)),
            TableId::PowerBalanceMembers => {
                TableRows::PowerBalanceMembers(rows::<PowerBalanceMemberRow>(tick))
            }
            TableId::PowerBalanceRanges => {
                TableRows::PowerBalanceRanges(rows::<PowerBalanceRangeRow>(tick))
            }
            TableId::PowerBalanceOverrides => {
                TableRows::PowerBalanceOverrides(rows::<PowerBalanceOverrideRow>(tick))
            }
            TableId::ProductionLines => TableRows::ProductionLines(rows::<ProductionLineRow>(tick)),
            TableId::Stockpiles => TableRows::Stockpiles(rows::<StockpileRow>(tick)),
            TableId::DivisionEquipment => {
                TableRows::DivisionEquipment(rows::<DivisionEquipmentRow>(tick))
            }
            TableId::VariantDefinitions => {
                TableRows::VariantDefinitions(rows::<VariantDefinitionRow>(tick))
            }
            TableId::VariantModules => TableRows::VariantModules(rows::<VariantModuleRow>(tick)),
            TableId::VariantStats => TableRows::VariantStats(rows::<VariantStatsRow>(tick)),
            TableId::VariantUpgrades => TableRows::VariantUpgrades(rows::<VariantUpgradeRow>(tick)),
            TableId::ArmyUpgradeGroups => {
                TableRows::ArmyUpgradeGroups(rows::<ArmyUpgradeGroupRow>(tick))
            }
            TableId::ArmyUpgradeRequests => {
                TableRows::ArmyUpgradeRequests(rows::<ArmyUpgradeRequestRow>(tick))
            }
            TableId::ArmyUpgradeDeliveries => {
                TableRows::ArmyUpgradeDeliveries(rows::<ArmyUpgradeDeliveryRow>(tick))
            }
            TableId::ArmyUpgradeProduced => {
                TableRows::ArmyUpgradeProduced(rows::<ArmyUpgradeProducedRow>(tick))
            }
            TableId::CountryFuel => TableRows::CountryFuel(rows::<CountryFuelRow>(tick)),
            TableId::ResourceDefinitions => {
                TableRows::ResourceDefinitions(rows::<ResourceDefinitionRow>(tick))
            }
            TableId::CountryResources => {
                TableRows::CountryResources(rows::<CountryResourceRow>(tick))
            }
            TableId::ResourceExchanges => {
                TableRows::ResourceExchanges(rows::<ResourceExchangeRow>(tick))
            }
            TableId::PurchaseContracts => {
                TableRows::PurchaseContracts(rows::<PurchaseContractRow>(tick))
            }
            TableId::ProvinceSupply => TableRows::ProvinceSupply(rows::<ProvinceSupplyRow>(tick)),
            TableId::HubMotorization => {
                TableRows::HubMotorization(rows::<HubMotorizationRow>(tick))
            }
            TableId::DisruptedSupply => {
                TableRows::DisruptedSupply(rows::<DisruptedSupplyRow>(tick))
            }
            TableId::ForeignHomebases => {
                TableRows::ForeignHomebases(rows::<ForeignHomebaseRow>(tick))
            }
            TableId::CountryExperience => {
                TableRows::CountryExperience(rows::<CountryExperienceRow>(tick))
            }
            TableId::StateBuildings => TableRows::StateBuildings(rows::<BuildingRow>(tick)),
            TableId::ProvinceBuildings => TableRows::ProvinceBuildings(rows::<BuildingRow>(tick)),
            TableId::StateBaseResources => {
                TableRows::StateBaseResources(rows::<StateBaseResourceRow>(tick))
            }
            TableId::BuildingConstruction => {
                TableRows::BuildingConstruction(rows::<BuildingConstructionRow>(tick))
            }
            TableId::RailwayConstruction => {
                TableRows::RailwayConstruction(rows::<RailwayConstructionRow>(tick))
            }
            TableId::OccupationSettings => {
                TableRows::OccupationSettings(rows::<OccupationSettingsRow>(tick))
            }
            TableId::StateOccupationLaws => {
                TableRows::StateOccupationLaws(rows::<StateOccupationLawRow>(tick))
            }
            TableId::StateGarrisons => TableRows::StateGarrisons(rows::<StateGarrisonRow>(tick)),
            TableId::StateAssignedManpower => {
                TableRows::StateAssignedManpower(rows::<StateAssignedManpowerRow>(tick))
            }
            TableId::CountryOccupationReserveManpower => {
                TableRows::CountryOccupationReserveManpower(rows::<
                    CountryOccupationReserveManpowerRow,
                >(tick))
            }
            TableId::StateResistance => {
                TableRows::StateResistance(rows::<StateResistanceRow>(tick))
            }
            TableId::CountryCollaboration => {
                TableRows::CountryCollaboration(rows::<CountryCollaborationRow>(tick))
            }
            TableId::Characters => TableRows::Characters(rows::<CharacterRow>(tick)),
            TableId::CountryLeaderRoles => {
                TableRows::CountryLeaderRoles(rows::<CountryLeaderRoleRow>(tick))
            }
            TableId::AdvisorRoles => TableRows::AdvisorRoles(rows::<AdvisorRoleRow>(tick)),
            TableId::UnitLeaders => TableRows::UnitLeaders(rows::<UnitLeaderRow>(tick)),
            TableId::UnitLeaderTraits => {
                TableRows::UnitLeaderTraits(rows::<UnitLeaderTraitRow>(tick))
            }
            TableId::UnitLeaderTraitExperience => {
                TableRows::UnitLeaderTraitExperience(rows::<UnitLeaderTraitExperienceRow>(tick))
            }
            TableId::ActiveAbilities => TableRows::ActiveAbilities(rows::<ActiveAbilityRow>(tick)),
            TableId::Capitulation => TableRows::Capitulation(rows::<CapitulationRow>(tick)),
            TableId::Subjects => TableRows::Subjects(rows::<SubjectRow>(tick)),
            TableId::Autonomy => TableRows::Autonomy(rows::<AutonomyRow>(tick)),
            TableId::IntelligenceAgencies => {
                TableRows::IntelligenceAgencies(rows::<IntelligenceAgencyRow>(tick))
            }
            TableId::OperativeMemberships => {
                TableRows::OperativeMemberships(rows::<OperativeMembershipRow>(tick))
            }
            TableId::Cryptology => TableRows::Cryptology(rows::<CryptologyRow>(tick)),
            TableId::IntelNetworks => TableRows::IntelNetworks(rows::<IntelNetworkRow>(tick)),
            TableId::ScriptFlags => TableRows::ScriptFlags(rows::<ScriptFlagRow>(tick)),
            TableId::ScriptVariables => {
                TableRows::ScriptVariables(rows::<ScriptVariablesRow>(tick))
            }
            TableId::Operations => TableRows::Operations(rows::<OperationRow>(tick)),
            TableId::OperationAssignments => {
                TableRows::OperationAssignments(rows::<OperationAssignmentRow>(tick))
            }
            TableId::WingEquipment => TableRows::WingEquipment(rows::<WingEquipmentRow>(tick)),
            TableId::AirBases => TableRows::AirBases(rows::<AirBaseRow>(tick)),
            TableId::WingCumulative => TableRows::WingCumulative(rows::<WingCumulativeRow>(tick)),
            TableId::AirRegionHistory => {
                TableRows::AirRegionHistory(rows::<AirRegionHistoryRow>(tick))
            }
            TableId::AirRegionDamage => {
                TableRows::AirRegionDamage(rows::<AirRegionDamageRow>(tick))
            }
            TableId::AirAces => TableRows::AirAces(rows::<AirAceRow>(tick)),
            TableId::AirStatistics => TableRows::AirStatistics(rows::<AirStatisticsRow>(tick)),
            TableId::SunkConvoys => TableRows::SunkConvoys(rows::<SunkConvoyRow>(tick)),
            TableId::RailCooldowns => TableRows::RailCooldowns(rows::<RailCooldownRow>(tick)),
            TableId::RailEdges => TableRows::RailEdges(rows::<RailEdgeRow>(tick)),
            TableId::LandTheatres => TableRows::LandTheatres(rows::<LandTheatreRow>(tick)),
            TableId::LandTheatreGroups => {
                TableRows::LandTheatreGroups(rows::<LandTheatreGroupRow>(tick))
            }
            TableId::LandTheatreMembers => {
                TableRows::LandTheatreMembers(rows::<LandTheatreMemberRow>(tick))
            }
            TableId::AirTheatres => TableRows::AirTheatres(rows::<AirTheatreRow>(tick)),
            TableId::AirTheatreGroups => {
                TableRows::AirTheatreGroups(rows::<AirTheatreGroupRow>(tick))
            }
            TableId::AirTheatreMembers => {
                TableRows::AirTheatreMembers(rows::<AirTheatreMemberRow>(tick))
            }
            TableId::NavalResults => TableRows::NavalResults(rows::<NavalResultRow>(tick)),
            TableId::NavalResultMembers => {
                TableRows::NavalResultMembers(rows::<NavalResultMemberRow>(tick))
            }
            TableId::NavalResultCountries => {
                TableRows::NavalResultCountries(rows::<NavalResultCountryRow>(tick))
            }
            TableId::SunkShips => TableRows::SunkShips(rows::<SunkShipsRow>(tick)),
            TableId::ProjectPrograms => TableRows::ProjectPrograms(rows::<ProjectProgramRow>(tick)),
            TableId::Projects => TableRows::Projects(rows::<ProjectRow>(tick)),
            TableId::ProjectHistory => TableRows::ProjectHistory(rows::<ProjectHistoryRow>(tick)),
            TableId::ProjectBreakthroughs => {
                TableRows::ProjectBreakthroughs(rows::<ProjectBreakthroughRow>(tick))
            }
            TableId::Nukes => TableRows::Nukes(rows::<NukeRow>(tick)),
            TableId::RaidCountries => TableRows::RaidCountries(rows::<RaidCountryRow>(tick)),
            TableId::Raids => TableRows::Raids(rows::<RaidRow>(tick)),
            TableId::PeaceConferences => {
                TableRows::PeaceConferences(rows::<PeaceConferenceRow>(tick))
            }
            TableId::WeatherLimits => TableRows::WeatherLimits(rows::<WeatherLimitsRow>(tick)),
            TableId::WeatherRegions => TableRows::WeatherRegions(rows::<WeatherRegionRow>(tick)),
            TableId::WeatherProvinces => {
                TableRows::WeatherProvinces(rows::<WeatherProvinceRow>(tick))
            }
            TableId::OperationTokens => {
                TableRows::OperationTokens(rows::<OperationTokensRow>(tick))
            }
            TableId::LexicalTokens => TableRows::LexicalTokens(rows::<LexicalTokenRow>(tick)),
            TableId::NavalSupremacy => TableRows::NavalSupremacy(rows::<NavalSupremacyRow>(tick)),
            TableId::RadarCoverage => TableRows::RadarCoverage(rows::<RadarCoverageRow>(tick)),
            TableId::VolunteerHosts => TableRows::VolunteerHosts(rows::<VolunteerHostRow>(tick)),
            TableId::TheatreAreas => TableRows::TheatreAreas(rows::<TheatreAreaRow>(tick)),
            TableId::StationedUnits => TableRows::StationedUnits(rows::<StationedUnitRow>(tick)),
            TableId::UnitIntel => TableRows::UnitIntel(rows::<UnitIntelRow>(tick)),
            TableId::AirBaseRegistrations => {
                TableRows::AirBaseRegistrations(rows::<AirBaseRegistrationRow>(tick))
            }
            _ => return None,
        })
    }
    fn check_table<T: Fixture>(table: TableId) {
        assert_eq!(table.key_bits(), T::KEY_BITS);
        for key in [0, maximum_key::<T>()] {
            let mut previous = T::fixture(key, 0);
            for tick in 1..=12 {
                let next = T::fixture(key, tick);
                assert_eq!(next.key(), key, "{table:?} key");
                let mut bytes = Vec::new();
                encode_row(&next, 0, &mut bytes).unwrap();
                let mut input = Decoder::new(&bytes);
                assert_eq!(decode_row::<T>(0, &mut input).unwrap(), next, "{table:?}");
                input.finish().unwrap();
                for prefix in 0..bytes.len() {
                    assert!(
                        decode_row::<T>(0, &mut Decoder::new(&bytes[..prefix])).is_err(),
                        "{table:?} prefix {prefix}"
                    );
                }
                let full = next.delta(None).unwrap().unwrap();
                assert_eq!(full.mask, table.full_mask(), "{table:?}");
                assert_eq!(full.apply::<T>(None).unwrap(), next, "{table:?} insert");
                if let Some(delta) = next.delta(Some(&previous)).unwrap() {
                    let mut bytes = Vec::new();
                    encode_delta(table, &delta, 0, &mut bytes).unwrap();
                    let mut input = Decoder::new(&bytes);
                    let decoded = decode_delta(table, 0, &mut input).unwrap();
                    input.finish().unwrap();
                    assert_eq!(
                        decoded.apply(Some(previous.clone())).unwrap(),
                        next,
                        "{table:?} delta"
                    );
                    let mut trailing = decoded;
                    trailing.values.push(0);
                    assert!(trailing.apply(Some(previous.clone())).is_err());
                    assert_eq!(
                        previous
                            .delta(Some(&next))
                            .unwrap()
                            .unwrap()
                            .apply(Some(next.clone()))
                            .unwrap(),
                        previous
                    );
                } else {
                    assert_eq!(previous, next);
                }
                assert!(next.delta(Some(&next)).unwrap().is_none());
                previous = next;
            }
        }
    }

    #[test]
    fn persisted_upgrade_groups_keep_army_identity_and_native_priority() {
        let mut previous = ArmyUpgradeGroupRow {
            country: u32::MAX,
            army: u64::MAX,
            priority: 1,
        };
        assert_eq!(TableId::ArmyUpgradeGroups.full_mask(), 3);
        for priority in [0, u32::MAX, 1] {
            let next = ArmyUpgradeGroupRow {
                priority,
                ..previous.clone()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 2);
            assert_eq!(delta.key, u128::from(u64::MAX));
            assert_eq!(delta.apply(Some(previous)).unwrap(), next);
            let mut bytes = Vec::new();
            encode_row(&next, 0, &mut bytes).unwrap();
            let mut input = Decoder::new(&bytes);
            assert_eq!(input.varint().unwrap(), u128::from(u64::MAX));
            assert_eq!(input.remaining(), 8);
            assert_eq!(input.u32().unwrap(), u32::MAX);
            assert_eq!(input.u32().unwrap(), priority);
            input.finish().unwrap();
            assert_eq!(
                decode_row::<ArmyUpgradeGroupRow>(0, &mut Decoder::new(&bytes)).unwrap(),
                next
            );
            previous = next;
        }
    }

    #[test]
    fn operations_record_native_definition_and_independent_progress_fields() {
        let previous = OperationRow {
            id: 7,
            owner: 1,
            target: 2,
            definition_token: 0x1234_5678,
            preparation_current: i64::MIN,
            preparation_required: i64::MAX,
            ..OperationRow::default()
        };
        assert_eq!(TableId::Operations.field_count(), 13);
        let mut bytes = Vec::new();
        encode_row(&previous, 0, &mut bytes).unwrap();
        assert_eq!(bytes.len(), 49);
        assert_eq!(
            &bytes[..14],
            &[7, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0x78, 0x56, 0x34, 0x12]
        );
        assert_eq!(&bytes[14..22], &i64::MIN.to_le_bytes());
        assert_eq!(&bytes[22..30], &i64::MAX.to_le_bytes());
        let next = OperationRow {
            preparation_current: i64::MAX,
            assigned_slots: u32::MAX,
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, (1 << 4) | (1 << 12));
        assert_eq!(delta.apply(Some(previous)).unwrap(), next);
    }

    #[test]
    fn intel_networks_preserve_unassigned_selectors_and_optional_native_strength() {
        let mut previous = IntelNetworkRow {
            owner_country: u32::MAX,
            target_country: 7,
            state_id: i32::MIN,
            strength_raw: None,
            subnetwork_selector: -1,
        };
        for (subnetwork_selector, strength_raw, mask) in [
            (i32::MIN, None, 2),
            (-2, None, 2),
            (0, Some(0), 3),
            (0, Some(i64::MIN), 1),
            (0, Some(i64::MAX), 1),
            (i32::MAX, Some(i64::MAX), 2),
            (-1, None, 3),
        ] {
            let next = IntelNetworkRow {
                strength_raw,
                subnetwork_selector,
                ..previous.clone()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, mask);
            assert_eq!(delta.apply(Some(previous)).unwrap(), next);
            let mut bytes = Vec::new();
            encode_row(&next, 0, &mut bytes).unwrap();
            let mut input = Decoder::new(&bytes);
            assert_eq!(decode_row::<IntelNetworkRow>(0, &mut input).unwrap(), next);
            input.finish().unwrap();
            previous = next;
        }
    }

    #[test]
    fn theatre_volunteer_tag_preserves_signed_bits_independently_of_owner() {
        let mut previous = LandTheatreRow {
            id: u64::MAX,
            owner: u32::MAX,
            ordinal: 7,
            volunteers_theatre: i32::MIN,
        };
        assert_eq!(TableId::LandTheatres.full_mask(), 7);
        for volunteers_theatre in [i32::MAX, 0, -1, i32::MIN] {
            let next = LandTheatreRow {
                volunteers_theatre,
                ..previous.clone()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1 << 2);
            assert_eq!(delta.apply(Some(previous)).unwrap(), next);
            let mut wire = Vec::new();
            encode_row(&next, 0, &mut wire).unwrap();
            let mut input = Decoder::new(&wire);
            assert_eq!(input.varint().unwrap(), u128::from(u64::MAX));
            assert_eq!(input.u32().unwrap(), u32::MAX);
            assert_eq!(input.u32().unwrap(), 7);
            assert_eq!(input.i32().unwrap(), volunteers_theatre);
            input.finish().unwrap();
            previous = next;
        }
    }

    #[test]
    fn wide_masks_change_one_native_channel_at_a_time() {
        assert_eq!(TableId::AirStatistics.field_count(), 26);
        assert_eq!(TableId::AirStatistics.mask_width(), 4);
        let previous = AirStatisticsRow::default();
        for index in 0..24 {
            let mut next = previous.clone();
            next.channels[index] = i64::MIN;
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1_u64 << (index + 2));
            let mut bytes = Vec::new();
            encode_delta(TableId::AirStatistics, &delta, 0, &mut bytes).unwrap();
            assert_eq!(&bytes[1..5], &(1_u32 << (index + 2)).to_le_bytes());
            assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
        }
        let previous = VariantStatsRow::default();
        for index in 0..10 {
            let mut next = previous.clone();
            next.values[index] = i64::MAX;
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1_u64 << index);
            assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
        }
        let previous = ProjectRow::default();
        let mut next = previous.clone();
        next.state.state_enums[3] = u32::MAX;
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask.count_ones(), 1);
        assert_eq!(delta.apply(Some(previous)).unwrap(), next);
    }

    #[test]
    fn structured_fields_reject_hostile_inner_counts_tags_and_suffixes() {
        fn reject<T: NativeValue>(inner: &[u8]) {
            let mut bytes = Vec::new();
            put_bytes(&mut bytes, inner).unwrap();
            assert!(read_value::<T>(Kind::Opaque, &mut Decoder::new(&bytes)).is_err());
        }
        reject::<Option<Vec<u8>>>(&[2]);
        reject::<Option<Vec<u8>>>(&[1, 255, 255, 255, 255]);
        reject::<Option<Vec<u8>>>(&[0, 0]);
        reject::<Option<AceDeath>>(&[1, 0]);
        reject::<Option<AceDeath>>(&[2]);
        reject::<Vec<i64>>(&u32::MAX.to_le_bytes());
        reject::<Vec<ScriptVariableEntry>>(&u32::MAX.to_le_bytes());
        reject::<Vec<AirHistoryEntry>>(&u32::MAX.to_le_bytes());
        reject::<Vec<AirDamageEntry>>(&u32::MAX.to_le_bytes());
        reject::<Vec<SunkShipEntry>>(&u32::MAX.to_le_bytes());
        let mut nested = 1_u32.to_le_bytes().to_vec();
        nested.extend_from_slice(&u32::MAX.to_le_bytes());
        nested.extend_from_slice(&0_i64.to_le_bytes());
        reject::<Vec<ScriptVariableEntry>>(&nested);
        for kind in [Kind::OptionalI32, Kind::OptionalU64, Kind::Bool] {
            assert!(kind.decode(&mut Decoder::new(&[2])).is_err());
        }
        for (kind, old, difference) in [
            (Kind::I16, Field::I16(i16::MAX), 1),
            (Kind::I16, Field::I16(i16::MIN), -1),
            (Kind::U16, Field::U16(u16::MAX), 1),
            (Kind::U16, Field::U16(0), -1),
            (Kind::Bool, Field::Bool(true), 1),
            (Kind::OptionalI32, Field::OptionalI32(Some(i32::MIN)), -1),
            (Kind::OptionalU64, Field::OptionalU64(Some(u64::MAX)), 1),
        ] {
            assert!(Change::Number(difference).apply(kind, old).is_err());
        }
    }

    #[test]
    fn nested_allocations_move_unchanged_and_absence_differs_from_empty() {
        let previous = AirAceRow {
            name: vec![0, 255, b'N'],
            surname: vec![254],
            callsign: vec![0],
            death: Some(AceDeath {
                killer_ace: u64::MAX,
                killer_country: u32::MAX,
            }),
            ..AirAceRow::default()
        };
        let name = previous.name.as_ptr();
        let mut next = previous.clone();
        next.handled = 255;
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        let restored = delta.apply(Some(previous)).unwrap();
        assert_eq!(restored, next);
        assert_eq!(restored.name.as_ptr(), name);
        let mut previous = FocusRow::default();
        for tree in [Some(vec![]), None, Some(vec![0, 255]), Some(vec![])] {
            let next = FocusRow {
                tree,
                ..FocusRow::default()
            };
            let delta = next.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1);
            assert_eq!(delta.apply(Some(previous)).unwrap(), next);
            previous = next;
        }
    }

    #[test]
    fn every_extended_table_round_trips_extremes_nested_records_and_deltas() {
        check_table::<FocusRow>(TableId::Focus);
        check_table::<CompletedFocusRow>(TableId::CompletedFocuses);
        check_table::<ResearchTechnologyRow>(TableId::ResearchTechnologies);
        check_table::<ResearchSlotRow>(TableId::ResearchSlots);
        check_table::<TechnologyDefinitionRow>(TableId::TechnologyDefinitions);
        check_table::<PoliticsRow>(TableId::Politics);
        check_table::<PoliticalPartyRow>(TableId::PoliticalParties);
        check_table::<ActiveIdeaRow>(TableId::ActiveIdeas);
        check_table::<DecisionCountsRow>(TableId::DecisionCounts);
        check_table::<DecisionRow>(TableId::Decisions);
        check_table::<DynamicModifierRow>(TableId::DynamicModifiers);
        check_table::<PreferredTacticRow>(TableId::PreferredTactics);
        check_table::<PreferredTacticOperandsRow>(TableId::PreferredTacticOperands);
        check_table::<TechSharingGroupRow>(TableId::TechSharingGroups);
        check_table::<TechSharingMemberRow>(TableId::TechSharingMembers);
        check_table::<PowerBalanceRow>(TableId::PowerBalances);
        check_table::<PowerBalanceMemberRow>(TableId::PowerBalanceMembers);
        check_table::<PowerBalanceRangeRow>(TableId::PowerBalanceRanges);
        check_table::<PowerBalanceOverrideRow>(TableId::PowerBalanceOverrides);
        check_table::<ProductionLineRow>(TableId::ProductionLines);
        check_table::<StockpileRow>(TableId::Stockpiles);
        check_table::<DivisionEquipmentRow>(TableId::DivisionEquipment);
        check_table::<VariantDefinitionRow>(TableId::VariantDefinitions);
        check_table::<VariantModuleRow>(TableId::VariantModules);
        check_table::<VariantStatsRow>(TableId::VariantStats);
        check_table::<VariantUpgradeRow>(TableId::VariantUpgrades);
        check_table::<ArmyUpgradeGroupRow>(TableId::ArmyUpgradeGroups);
        check_table::<ArmyUpgradeRequestRow>(TableId::ArmyUpgradeRequests);
        check_table::<ArmyUpgradeDeliveryRow>(TableId::ArmyUpgradeDeliveries);
        check_table::<ArmyUpgradeProducedRow>(TableId::ArmyUpgradeProduced);
        check_table::<CountryFuelRow>(TableId::CountryFuel);
        check_table::<ResourceDefinitionRow>(TableId::ResourceDefinitions);
        check_table::<CountryResourceRow>(TableId::CountryResources);
        check_table::<ResourceExchangeRow>(TableId::ResourceExchanges);
        check_table::<PurchaseContractRow>(TableId::PurchaseContracts);
        check_table::<ProvinceSupplyRow>(TableId::ProvinceSupply);
        check_table::<HubMotorizationRow>(TableId::HubMotorization);
        check_table::<DisruptedSupplyRow>(TableId::DisruptedSupply);
        check_table::<ForeignHomebaseRow>(TableId::ForeignHomebases);
        check_table::<CountryExperienceRow>(TableId::CountryExperience);
        check_table::<BuildingRow>(TableId::StateBuildings);
        check_table::<BuildingRow>(TableId::ProvinceBuildings);
        check_table::<StateBaseResourceRow>(TableId::StateBaseResources);
        check_table::<BuildingConstructionRow>(TableId::BuildingConstruction);
        check_table::<RailwayConstructionRow>(TableId::RailwayConstruction);
        check_table::<OccupationSettingsRow>(TableId::OccupationSettings);
        check_table::<StateOccupationLawRow>(TableId::StateOccupationLaws);
        check_table::<StateGarrisonRow>(TableId::StateGarrisons);
        check_table::<StateAssignedManpowerRow>(TableId::StateAssignedManpower);
        check_table::<CountryOccupationReserveManpowerRow>(
            TableId::CountryOccupationReserveManpower,
        );
        check_table::<StateResistanceRow>(TableId::StateResistance);
        check_table::<CountryCollaborationRow>(TableId::CountryCollaboration);
        check_table::<CharacterRow>(TableId::Characters);
        check_table::<CountryLeaderRoleRow>(TableId::CountryLeaderRoles);
        check_table::<AdvisorRoleRow>(TableId::AdvisorRoles);
        check_table::<UnitLeaderRow>(TableId::UnitLeaders);
        check_table::<UnitLeaderTraitRow>(TableId::UnitLeaderTraits);
        check_table::<UnitLeaderTraitExperienceRow>(TableId::UnitLeaderTraitExperience);
        check_table::<ActiveAbilityRow>(TableId::ActiveAbilities);
        check_table::<CapitulationRow>(TableId::Capitulation);
        check_table::<SubjectRow>(TableId::Subjects);
        check_table::<AutonomyRow>(TableId::Autonomy);
        check_table::<IntelligenceAgencyRow>(TableId::IntelligenceAgencies);
        check_table::<OperativeMembershipRow>(TableId::OperativeMemberships);
        check_table::<CryptologyRow>(TableId::Cryptology);
        check_table::<IntelNetworkRow>(TableId::IntelNetworks);
        check_table::<ScriptFlagRow>(TableId::ScriptFlags);
        check_table::<ScriptVariablesRow>(TableId::ScriptVariables);
        check_table::<OperationRow>(TableId::Operations);
        check_table::<OperationAssignmentRow>(TableId::OperationAssignments);
        check_table::<WingEquipmentRow>(TableId::WingEquipment);
        check_table::<AirBaseRow>(TableId::AirBases);
        check_table::<WingCumulativeRow>(TableId::WingCumulative);
        check_table::<AirRegionHistoryRow>(TableId::AirRegionHistory);
        check_table::<AirRegionDamageRow>(TableId::AirRegionDamage);
        check_table::<AirAceRow>(TableId::AirAces);
        check_table::<AirStatisticsRow>(TableId::AirStatistics);
        check_table::<SunkConvoyRow>(TableId::SunkConvoys);
        check_table::<RailCooldownRow>(TableId::RailCooldowns);
        check_table::<RailEdgeRow>(TableId::RailEdges);
        check_table::<LandTheatreRow>(TableId::LandTheatres);
        check_table::<LandTheatreGroupRow>(TableId::LandTheatreGroups);
        check_table::<LandTheatreMemberRow>(TableId::LandTheatreMembers);
        check_table::<AirTheatreRow>(TableId::AirTheatres);
        check_table::<AirTheatreGroupRow>(TableId::AirTheatreGroups);
        check_table::<AirTheatreMemberRow>(TableId::AirTheatreMembers);
        check_table::<NavalResultRow>(TableId::NavalResults);
        check_table::<NavalResultMemberRow>(TableId::NavalResultMembers);
        check_table::<NavalResultCountryRow>(TableId::NavalResultCountries);
        check_table::<SunkShipsRow>(TableId::SunkShips);
        check_table::<ProjectProgramRow>(TableId::ProjectPrograms);
        check_table::<ProjectRow>(TableId::Projects);
        check_table::<ProjectHistoryRow>(TableId::ProjectHistory);
        check_table::<ProjectBreakthroughRow>(TableId::ProjectBreakthroughs);
        check_table::<NukeRow>(TableId::Nukes);
        check_table::<RaidCountryRow>(TableId::RaidCountries);
        check_table::<RaidRow>(TableId::Raids);
        check_table::<PeaceConferenceRow>(TableId::PeaceConferences);
        check_table::<WeatherLimitsRow>(TableId::WeatherLimits);
        check_table::<WeatherRegionRow>(TableId::WeatherRegions);
        check_table::<WeatherProvinceRow>(TableId::WeatherProvinces);
        check_table::<OperationTokensRow>(TableId::OperationTokens);
        check_table::<LexicalTokenRow>(TableId::LexicalTokens);
        check_table::<NavalSupremacyRow>(TableId::NavalSupremacy);
        check_table::<RadarCoverageRow>(TableId::RadarCoverage);
        check_table::<VolunteerHostRow>(TableId::VolunteerHosts);
        check_table::<TheatreAreaRow>(TableId::TheatreAreas);
        check_table::<StationedUnitRow>(TableId::StationedUnits);
        check_table::<UnitIntelRow>(TableId::UnitIntel);
        check_table::<AirBaseRegistrationRow>(TableId::AirBaseRegistrations);
    }
}
