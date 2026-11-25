use super::*;
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OwnedWargoalRow {
    pub country: u32,
    pub ordinal: u32,
    pub recipient: u32,
    pub goal_type: i32,
    pub states: Vec<u32>,
}
typed_codec!(OwnedWargoalRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (recipient): U32, 1 => (goal_type): I32, 2 => (states): U32List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResearchModifierRow {
    pub country: u32,
    pub index: u32,
    pub ordinal: u32,
    pub technology_handle: u32,
    pub kind: u8,
    pub raw: i64,
    pub auxiliary: i64,
    pub count: u32,
    pub source_id: u32,
    pub name: Vec<u8>,
}
impl ResearchModifierRow {
    pub const TECHNOLOGY: u8 = 0;
    pub const COUNTRY_SPEED: u8 = 1;
    pub const CATEGORY_SPEED: u8 = 2;
    pub const DESIGN_TEAM: u8 = 3;
    pub const EXPERIENCE: u8 = 4;
    pub const SHARING: u8 = 5;
    pub const LICENSE: u8 = 6;
    pub const SCIENTIST: u8 = 7;
    pub const YEAR: u8 = 8;
    pub const AHEAD_FACTOR: u8 = 9;
    pub const MINIMUM_SPEED: u8 = 10;
    pub const COUNTRY_SOURCE: u8 = 11;
    pub const IDEA_SOURCE: u8 = 12;
    pub const SCRIPT_SOURCE: u8 = 13;
    pub const SCIENTIST_SOURCE: u8 = 14;
    pub const SHARING_MULTIPLIER: u8 = 15;
    pub const SHARING_MAXIMUM: u8 = 16;
    pub const MODIFIER_METADATA: u8 = 17;
    pub const SCIENTIST_CATEGORY: u8 = 18;
    pub const SHARING_MEMBER: u8 = 19;
    pub const SHARING_PREDECESSOR: u8 = 20;
    pub const SHARING_FILTER: u8 = 21;
    pub const SHARING_CATEGORY: u8 = 22;
    pub const SHARING_DOCTRINE: u8 = 23;
    pub const SHARING_MISSING_MEMBER: u8 = 24;
    pub const GRANT_COUNTER: u8 = 25;
    pub const SPEED_GRANT: u8 = 26;
    pub const COST_GRANT: u8 = 27;
    pub const GRANT_USES: u8 = 28;
    pub const GRANT_TECHNOLOGY: u8 = 29;
    pub const GRANT_CATEGORY: u8 = 30;
    pub const SHARING_ROSTER: u8 = 31;
}
typed_codec!(ResearchModifierRow, 96, |row| (row.country as u128) << 64 | (row.index as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 64) as u32, index: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (technology_handle): U32, 1 => (kind): U8, 2 => (raw): I64, 3 => (auxiliary): I64, 4 => (count): U32, 5 => (source_id): U32, 6 => (name): Bytes }, |row| if row.kind > Self::SHARING_ROSTER { Err(invalid("invalid research modifier kind")) } else { Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResearchInputRow {
    pub country: u32,
    pub index: u32,
    pub technology_handle: u32,
    pub technology_present: u8,
    pub token: Vec<u8>,
    pub classifier: u32,
    pub level: i32,
    pub progress_raw: i64,
    pub cost_raw: i64,
    pub cost_scale_raw: i64,
    pub cost_debug: u8,
    pub research_speed_raw: i64,
    pub banked_days_raw: i64,
    pub applied_days_raw: i64,
    pub ahead_reduction_raw: i64,
    pub speed_bonus_raw: i64,
    pub bonus_count: u32,
    pub database_index: Recorded<i32>,
}
typed_codec!(ResearchInputRow, 64, |row| (row.country as u128) << 32 | row.index as u128, |key| Self { country: (key >> 32) as u32, index: key as u32, ..Default::default() },
 { 0 => (technology_handle): U32, 1 => (technology_present): U8, 2 => (token): Bytes, 3 => (classifier): U32, 4 => (level): I32, 5 => (progress_raw): I64, 6 => (cost_raw): I64, 7 => (cost_scale_raw): I64, 8 => (cost_debug): U8, 9 => (research_speed_raw): I64, 10 => (banked_days_raw): I64, 11 => (applied_days_raw): I64, 12 => (ahead_reduction_raw): I64, 13 => (speed_bonus_raw): I64, 14 => (bonus_count): U32, 15 => (database_index): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResearchAppliedBonusRow {
    pub country: u32,
    pub index: u32,
    pub ordinal: u32,
    pub technology_handle: u32,
    pub id: i32,
    pub resolved: u8,
    pub name: Vec<u8>,
    pub ahead_reduction_raw: i64,
    pub speed_bonus_raw: i64,
    pub cost_reduction_raw: Option<i64>,
}
typed_codec!(ResearchAppliedBonusRow, 96, |row| (row.country as u128) << 64 | (row.index as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 64) as u32, index: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (technology_handle): U32, 1 => (id): I32, 2 => (resolved): U8, 3 => (name): Bytes, 4 => (ahead_reduction_raw): I64, 5 => (speed_bonus_raw): I64, 6 => (cost_reduction_raw): OptionalI64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct SliceReadRow {
    pub table: u32,
    pub owner: u32,
    pub sample_hour: i32,
}
typed_codec!(SliceReadRow, 64, |row| (row.table as u128) << 32 | row.owner as u128,
 |key| Self { table: (key >> 32) as u32, owner: key as u32, ..Default::default() },
 { 0 => (sample_hour): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RelationLinkRow {
    pub country: u32,
    pub target_index: u32,
    pub links: u8,
    pub transport_subject: Recorded<Option<TransportSubjectAccess>>,
}
impl RelationLinkRow {
    pub const ATTACHE: u8 = 1;
    pub const AIR_BASE_ACCESS: u8 = 2;
    pub const MILITARY_ACCESS: u8 = 8;
    pub const DOCKING_RIGHTS: u8 = 16;
}
typed_codec!(RelationLinkRow, 64, |row| (row.country as u128) << 32 | row.target_index as u128,
 |key| Self { country: (key >> 32) as u32, target_index: key as u32, ..Default::default() },
 { 0 => (links): U8, 1 => (transport_subject): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct BorderConflictRow {
    pub province: u32,
    pub groups: Vec<u64>,
}
typed_codec!(BorderConflictRow, 32, |row| row.province as u128,
 |key| Self { province: key as u32, ..Default::default() },
 { 0 => (groups): U64List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryWarRow {
    pub country: u32,
    pub can_occupy_non_war: u8,
    pub enemies_read: u8,
    pub enemies: Vec<u32>,
    pub ordered: Recorded<Option<OrderedEnemyInputs>>,
}
typed_codec!(CountryWarRow, 32, |row| row.country as u128,
 |key| Self { country: key as u32, ..Default::default() },
 { 0 => (can_occupy_non_war): U8, 1 => (enemies_read): U8, 2 => (enemies): U32List, 3 => (ordered): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateFlagRow {
    pub state: u32,
    pub demilitarized: u8,
    pub border_conflict_active: Recorded<u8>,
    pub border_conflict_countries: Recorded<Vec<[i32; 2]>>,
}
typed_codec!(StateFlagRow, 32, |row| row.state as u128,
 |key| Self { state: key as u32, ..Default::default() },
 { 0 => (demilitarized): U8, 1 => (border_conflict_active): U8, 2 => (border_conflict_countries): Opaque }, |row| {
    if let (Recorded::Value(0), Recorded::Value(countries)) = (&row.border_conflict_active, &row.border_conflict_countries)
        && !countries.is_empty() {
        return Err(invalid("inactive state has border conflict countries"));
    }
    Ok(())
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
 { 0 => (tree): Opaque, 1 => (current): Opaque, 2 => (continuous): Opaque, 3 => (progress): I64, 4 => (paused): U8, 5 => (completed_count): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CompletedFocusRow {
    pub country: u32,
    pub ordinal: u32,
    pub token: Vec<u8>,
}
typed_codec!(CompletedFocusRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, token: Default::default() },
 { 0 => (token): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResearchTechnologyRow {
    pub country: u32,
    pub technology_handle: u32,
    pub database_index: i32,
    pub applied_bonus_count: i32,
    pub classifier: u32,
    pub level: i32,
    pub researched: u8,
    pub progress_raw: i64,
    pub ahead_reduction_years_raw: i64,
}
typed_codec!(ResearchTechnologyRow, 64, |row| (row.country as u128) << 32 | row.database_index as u32 as u128, |key| Self { country: (key >> 32) as u32, database_index: key as u32 as i32, ..Default::default() },
 { 0 => (classifier): U32, 1 => (level): I32, 2 => (researched): U8, 3 => (progress_raw): I64, 4 => (ahead_reduction_years_raw): I64, 5 => (technology_handle): U32, 6 => (applied_bonus_count): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResearchSlotRow {
    pub country: u32,
    pub index: u32,
    pub technology_handle: u32,
    pub technology_present: u8,
    pub research_speed_raw: i64,
    pub banked_days_raw: i64,
    pub applied_days_raw: i64,
    pub database_index: Recorded<i32>,
}
typed_codec!(ResearchSlotRow, 64, |row| (row.country as u128) << 32 | row.index as u128, |key| Self { country: (key >> 32) as u32, index: key as u32, technology_handle: Default::default(), technology_present: Default::default(), research_speed_raw: Default::default(), banked_days_raw: Default::default(), applied_days_raw: Default::default(), database_index: Default::default() },
 { 0 => (technology_handle): U32, 1 => (technology_present): U8, 2 => (research_speed_raw): I64, 3 => (banked_days_raw): I64, 4 => (applied_days_raw): I64, 5 => (database_index): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TechnologyDefinitionRow {
    pub technology_handle: u32,
    pub database_index: i32,
    pub year: i32,
    pub max_level: i32,
    pub research_cost_raw: i64,
    pub id: Vec<u8>,
}
typed_codec!(TechnologyDefinitionRow, 32, |row| row.database_index as u32 as u128, |key| Self { database_index: key as u32 as i32, technology_handle: Default::default(), year: Default::default(), max_level: Default::default(), research_cost_raw: Default::default(), id: Default::default() },
 { 0 => (technology_handle): U32, 1 => (year): I32, 2 => (max_level): I32, 3 => (research_cost_raw): I64, 4 => (id): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PoliticsRow {
    pub country: u32,
    pub last_election: i32,
    pub election_frequency: RetiredI32,
    pub elections_allowed: RetiredU8,
    pub capital: i32,
    pub ruling_party: Option<u32>,
    pub original_capital: i32,
    pub has_elections: Recorded<bool>,
    pub election_interval: Recorded<i32>,
    pub next_election_cache: Recorded<Option<i32>>,
}
typed_codec!(PoliticsRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (last_election): I32, 1 => (election_frequency): I32, 2 => (elections_allowed): U8, 3 => (capital): I32, 4 => (ruling_party): OptionalU32, 5 => (original_capital): I32, 6 => (has_elections): Bool, 7 => (election_interval): I32, 8 => (next_election_cache): OptionalI32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PoliticalPartyRow {
    pub country: u32,
    pub id: u32,
    pub popularity: i64,
    pub is_ruling: u8,
    pub token: Vec<u8>,
    pub tag: u32,
    pub name_default: u8,
    pub name: Vec<u8>,
    pub long_name: Vec<u8>,
}
typed_codec!(PoliticalPartyRow, 64, |row| (row.country as u128) << 32 | row.id as u128, |key| Self { country: (key >> 32) as u32, id: key as u32, ..Default::default() },
 { 0 => (popularity): I64, 1 => (is_ruling): U8, 2 => (token): Bytes, 3 => (tag): U32, 4 => (name_default): U8, 5 => (name): Bytes, 6 => (long_name): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ActiveIdeaRow {
    pub country: u32,
    pub ordinal: u32,
    pub token: Vec<u8>,
}
typed_codec!(ActiveIdeaRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, token: Default::default() },
 { 0 => (token): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TimedIdeaRow {
    pub country: u32,
    pub ordinal: u32,
    pub token: Vec<u8>,
    pub days: i32,
}
typed_codec!(TimedIdeaRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (token): Bytes, 1 => (days): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DecisionRandomItemRow {
    pub country: u32,
    pub ordinal: u32,
    pub token: Vec<u8>,
    pub count: i32,
    pub target: u32,
}
typed_codec!(DecisionRandomItemRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (token): Bytes, 1 => (count): I32, 2 => (target): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
 { 0 => (taken): U32, 1 => (re_enable): U32, 2 => (remove): U32, 3 => (timed): U32, 4 => (targeted_timed): U32, 5 => (targeted): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
    pub description_seed: Recorded<Option<Option<i32>>>,
}
typed_codec!(DecisionRow, 72, |row| (row.country as u128) << 40 | (row.container as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 40) as u32, container: (key >> 32) as u8, ordinal: key as u32, token: Default::default(), days: Default::default(), lifecycle: Default::default(), target_kind: Default::default(), country_target: Default::default(), numeric_target: Default::default(), ignore: Default::default(), description_seed: Default::default() },
{ 0 => (token): Bytes, 1 => (days): I32, 2 => (lifecycle): I32, 3 => (target_kind): U8, 4 => (country_target): U32, 5 => (numeric_target): U32, 6 => (ignore): U8, 7 => (description_seed): Opaque }, |row| {
   if matches!(row.description_seed, Recorded::Value(Some(_))) && row.container < 3 {
       Err(invalid("decision description seed on a non-targeted container"))
   } else {
       Ok(())
   }
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
 { 0 => (scope_country): U32, 1 => (scope_state): I32, 2 => (days_remaining): I32, 3 => (enabled): U8, 4 => (token): Bytes, 5 => (values): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TechSharingGroupRow {
    pub id: u32,
    pub base_bonus: i64,
    pub member_count: u32,
}
typed_codec!(TechSharingGroupRow, 32, |row| row.id as u128, |key| Self { id: key as u32, base_bonus: Default::default(), member_count: Default::default() },
 { 0 => (base_bonus): I64, 1 => (member_count): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TechSharingMemberRow {
    pub group: u32,
    pub ordinal: u32,
    pub country: u32,
    pub bonus: i64,
}
typed_codec!(TechSharingMemberRow, 64, |row| (row.group as u128) << 32 | row.ordinal as u128, |key| Self { group: (key >> 32) as u32, ordinal: key as u32, country: Default::default(), bonus: Default::default() },
 { 0 => (country): U32, 1 => (bonus): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
 { 0 => (value_raw): I64, 1 => (balance): Bytes, 2 => (left): Bytes, 3 => (right): Bytes, 4 => (trending): Bytes, 5 => (active_range): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
 { 0 => (country): U32, 1 => (daily_raw): I64, 2 => (weekly_raw): I64, 3 => (daily_present): Bool, 4 => (weekly_present): Bool, 5 => (daily_flags): U32, 6 => (weekly_flags): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PowerBalanceRangeRow {
    pub record: u32,
    pub ordinal: u32,
    pub identity: Vec<u8>,
}
typed_codec!(PowerBalanceRangeRow, 64, |row| (row.record as u128) << 32 | row.ordinal as u128, |key| Self { record: (key >> 32) as u32, ordinal: key as u32, identity: Default::default() },
 { 0 => (identity): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PowerBalanceOverrideRow {
    pub record: u32,
    pub ordinal: u32,
    pub side: Vec<u8>,
    pub gfx: Vec<u8>,
}
typed_codec!(PowerBalanceOverrideRow, 64, |row| (row.record as u128) << 32 | row.ordinal as u128, |key| Self { record: (key >> 32) as u32, ordinal: key as u32, side: Default::default(), gfx: Default::default() },
 { 0 => (side): Bytes, 1 => (gfx): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryExperienceGainRow {
    pub country: u32,
    pub instance: u32,
    pub complete: u8,
    pub army_raw: i64,
    pub navy_raw: i64,
    pub air_raw: i64,
}
typed_codec!(CountryExperienceGainRow, 64, |row| (row.country as u128) << 32 | row.instance as u128,
    |key| Self { country: (key >> 32) as u32, instance: key as u32, ..Default::default() },
    { 0 => (army_raw): I64, 1 => (navy_raw): I64, 2 => (air_raw): I64, 3 => (complete): U8 },
    |row| if row.complete > 1 {
        Err(invalid("experience gain interval completeness is not a boolean"))
    } else if row.army_raw < 0 || row.navy_raw < 0 || row.air_raw < 0 {
        Err(invalid("experience gross gain must be nonnegative"))
    } else { Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
 { 0 => (army_raw): I64, 1 => (army_daily_raw): I64, 2 => (army_daily_training_raw): I64, 3 => (navy_raw): I64, 4 => (navy_daily_raw): I64, 5 => (navy_daily_training_raw): I64, 6 => (air_raw): I64, 7 => (air_daily_raw): I64, 8 => (armies_for_training_raw): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OccupationSettingsRow {
    pub country: u32,
    pub garrison_template: Option<u64>,
    pub default_law_present: u8,
    pub law: Vec<u8>,
    pub occupied: Recorded<Option<Vec<u32>>>,
    pub group_laws: Recorded<Option<Vec<OccupationGroupLaw>>>,
    pub fallback_law: Recorded<Option<Vec<u8>>>,
}
typed_codec!(OccupationSettingsRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (garrison_template): OptionalU64, 1 => (default_law_present): U8, 2 => (law): Bytes, 3 => (occupied): Opaque, 4 => (group_laws): Opaque, 5 => (fallback_law): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateOccupationLawRow {
    pub controller: u32,
    pub occupied: u32,
    pub state: u32,
    pub name: Vec<u8>,
}
typed_codec!(StateOccupationLawRow, 96, |row| (row.controller as u128) << 64 | (row.occupied as u128) << 32 | row.state as u128, |key| Self { controller: (key >> 64) as u32, occupied: (key >> 32) as u32, state: key as u32, name: Default::default() },
 { 0 => (name): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateGarrisonRow {
    pub controller: u32,
    pub occupied: u32,
    pub state: u32,
    pub required_manpower: i32,
    pub strength_ratio: i64,
    pub garrison_required: i64,
    pub serialized_flag: u8,
    pub equipment: Recorded<GarrisonEquipmentInputs>,
}
typed_codec!(StateGarrisonRow, 64, |row| (row.controller as u128) << 32 | row.state as u128, |key| Self { controller: (key >> 32) as u32, state: key as u32, ..Default::default() },
 { 0 => (occupied): U32, 1 => (required_manpower): I32, 2 => (strength_ratio): I64, 3 => (garrison_required): I64, 4 => (serialized_flag): U8, 5 => (equipment): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateAssignedManpowerRow {
    pub controller: u32,
    pub occupied: u32,
    pub state: u32,
    pub ordinal: u32,
    pub source_country: u32,
    pub amount: i32,
}
typed_codec!(StateAssignedManpowerRow, 96, |row| (row.controller as u128) << 64 | (row.state as u128) << 32 | row.ordinal as u128, |key| Self { controller: (key >> 64) as u32, state: (key >> 32) as u32, ordinal: key as u32, occupied: Default::default(), source_country: Default::default(), amount: Default::default() },
 { 0 => (occupied): U32, 1 => (source_country): U32, 2 => (amount): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryOccupationReserveManpowerRow {
    pub country: u32,
    pub ordinal: u32,
    pub source_country: u32,
    pub amount: i32,
}
typed_codec!(CountryOccupationReserveManpowerRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, source_country: Default::default(), amount: Default::default() },
 { 0 => (source_country): U32, 1 => (amount): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateResistanceRow {
    pub state: u32,
    pub country: u32,
    pub level: i64,
    pub speed: i64,
    pub compliance: i64,
    pub compliance_speed: i64,
    pub active: u8,
    pub hover: Recorded<Option<OccupationHover>>,
    pub modifier_sources: Recorded<Vec<OccupationModifierSource>>,
    pub map: Recorded<Option<OccupationMap>>,
}
typed_codec!(StateResistanceRow, 32, |row| row.state as u128, |key| Self { state: key as u32, ..Default::default() },
 { 0 => (country): U32, 1 => (level): I64, 2 => (speed): I64, 3 => (compliance): I64, 4 => (compliance_speed): I64, 5 => (active): U8, 6 => (hover): Opaque, 7 => (modifier_sources): Opaque, 8 => (map): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryCollaborationRow {
    pub source: u32,
    pub target: u32,
    pub progress: i64,
}
typed_codec!(CountryCollaborationRow, 64, |row| (row.source as u128) << 32 | row.target as u128, |key| Self { source: (key >> 32) as u32, target: key as u32, progress: Default::default() },
 { 0 => (progress): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CapitulationRow {
    pub country: u32,
    pub capitulated: u8,
    pub capitulated_hour: i32,
    pub last_surrender_hour: RetiredI32,
    pub faction_join_hour: RetiredI32,
    pub legitimacy_raw: RetiredI64,
    pub exile_army_leaders: RetiredI32,
    pub war_goal_recipients: Recorded<Option<Vec<u32>>>,
}
typed_codec!(CapitulationRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (capitulated): U8, 1 => (capitulated_hour): I32, 2 => (last_surrender_hour): I32, 3 => (faction_join_hour): I32, 4 => (legitimacy_raw): I64, 5 => (exile_army_leaders): I32, 6 => (war_goal_recipients): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
 { 0 => (start_hour): I32, 1 => (end_hour): I32, 2 => (cancel): U8, 3 => (autonomy_ratio): I64, 4 => (state): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
 { 0 => (score): I64, 1 => (lower_score): I64, 2 => (upper_score): I64, 3 => (last_modified_bits): U32, 4 => (state): Bytes, 5 => (previous): Opaque, 6 => (next): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ScriptFlagRow {
    pub scope_kind: u8,
    pub scope_id: u32,
    pub token: u32,
    pub set_hour: i32,
    pub value: i16,
    pub remaining_days: i16,
}
typed_codec!(ScriptFlagRow, 72, |row| (row.scope_kind as u128) << 64 | (row.scope_id as u128) << 32 | row.token as u128, |key| Self { scope_kind: (key >> 64) as u8, scope_id: (key >> 32) as u32, token: key as u32, set_hour: Default::default(), value: Default::default(), remaining_days: Default::default() },
 { 0 => (set_hour): I32, 1 => (value): I16, 2 => (remaining_days): I16 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ScriptVariablesRow {
    pub scope_kind: u8,
    pub scope_id: u32,
    pub entries: Vec<ScriptVariableEntry>,
    pub seed: Recorded<Option<u32>>,
}
typed_codec!(ScriptVariablesRow, 40, |row| (row.scope_kind as u128) << 32 | row.scope_id as u128, |key| Self { scope_kind: (key >> 32) as u8, scope_id: key as u32, ..Default::default() },
 { 0 => (entries): Opaque, 1 => (seed): OptionalU32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ScriptVariableNameRow {
    pub id: u32,
    pub name: Vec<u8>,
}
typed_codec!(ScriptVariableNameRow, 32, |row| row.id as u128,
 |key| Self { id: key as u32, ..Default::default() },
 { 0 => (name): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ScriptVariableValueRow {
    pub scope_kind: u8,
    pub scope_id: u32,
    pub var_id: u32,
    pub value_raw: i64,
}
typed_codec!(ScriptVariableValueRow, 72,
 |row| (row.scope_kind as u128) << 64 | (row.scope_id as u128) << 32 | row.var_id as u128,
 |key| Self { scope_kind: (key >> 64) as u8, scope_id: (key >> 32) as u32, var_id: key as u32, ..Default::default() },
 { 0 => (value_raw): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GlobalSavedEventTargetRow {
    pub ordinal: u32,
    pub name: Vec<u8>,
    pub state: i32,
    pub country: i32,
    pub character: u32,
    pub live: u16,
    pub character_leader: Option<u64>,
}
impl GlobalSavedEventTargetRow {
    pub const CHARACTER: u16 = 1;
    pub const OPERATION: u16 = 1 << 1;
    pub const ACE: u16 = 1 << 2;
    pub const UNIT: u16 = 1 << 3;
    pub const INDUSTRIAL_ORGANISATION: u16 = 1 << 4;
    pub const PURCHASE_CONTRACT: u16 = 1 << 5;
    pub const RAID_INSTANCE: u16 = 1 << 6;
    pub const PROJECT: u16 = 1 << 7;
    pub const FACTION: u16 = 1 << 8;
    pub const REGION: u16 = 1 << 9;
    pub const COMBATANT: u16 = 1 << 10;
    pub const UNSAVED_NAME: u16 = 1 << 11;
    pub const KINDS: [(u16, &'static str); 11] = [
        (Self::CHARACTER, "character"),
        (Self::OPERATION, "operation"),
        (Self::ACE, "ace"),
        (Self::UNIT, "unit"),
        (Self::INDUSTRIAL_ORGANISATION, "industrial_organisation"),
        (Self::PURCHASE_CONTRACT, "purchase_contract"),
        (Self::RAID_INSTANCE, "raid_instance"),
        (Self::PROJECT, "project"),
        (Self::FACTION, "faction"),
        (Self::REGION, "strategic_region"),
        (Self::COMBATANT, "combatant"),
    ];
    pub fn kind_name(mask: u16) -> &'static str {
        Self::KINDS
            .iter()
            .find(|(bit, _)| *bit == mask)
            .map(|(_, name)| *name)
            .expect("a defined kind bit")
    }
}
typed_codec!(GlobalSavedEventTargetRow, 32, |row| row.ordinal as u128,
 |key| Self { ordinal: key as u32, ..Default::default() },
 { 0 => (name): Bytes, 1 => (state): I32, 2 => (country): I32, 3 => (character): U32, 4 => (live): U16, 5 => (character_leader): OptionalU64 },
 |row| if row.live & !0xfff != 0 { Err(invalid("invalid saved event target live mask")) } else { Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PartyLeaderRow {
    pub country: u32,
    pub party: u32,
    pub ordinal: u32,
    pub character: Option<u32>,
    pub role: Option<i32>,
    pub leader_id: i32,
}
typed_codec!(PartyLeaderRow, 96, |row| (row.country as u128) << 64 | (row.party as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 64) as u32, party: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (character): OptionalU32, 1 => (leader_id): I32, 2 => (role): OptionalI32 }, |row| {
    if row.character.is_some() != row.role.is_some() {
        return Err(invalid("party leader character and role presence differ"));
    }
    Ok(())
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryCivilWarRow {
    pub country: u32,
    pub ordinal: u32,
    pub target: u32,
}
typed_codec!(CountryCivilWarRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (target): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateCoreRow {
    pub state: u32,
    pub cores: Vec<u32>,
    pub claims: Recorded<Vec<u32>>,
    pub strategic_locations: Recorded<Vec<u64>>,
    pub province_strategic_locations: Recorded<Vec<u64>>,
}
typed_codec!(StateCoreRow, 32, |row| row.state as u128, |key| Self { state: key as u32, ..Default::default() },
 { 0 => (cores): U32List, 1 => (claims): U32List, 2 => (strategic_locations): U64List, 3 => (province_strategic_locations): U64List }, |_row| Ok(()));

impl StateCoreRow {
    pub fn state_locations(&self) -> Option<impl Iterator<Item = (u32, u32)> + '_> {
        match &self.strategic_locations {
            Recorded::Value(pairs) => Some(pairs.iter().map(|&pair| split(pair))),
            Recorded::NotRecorded => None,
        }
    }

    pub fn province_locations(&self) -> Option<impl Iterator<Item = (u32, u32)> + '_> {
        match &self.province_strategic_locations {
            Recorded::Value(pairs) => Some(pairs.iter().map(|&pair| split(pair))),
            Recorded::NotRecorded => None,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateDynamicModifierRow {
    pub state: u32,
    pub ordinal: u32,
    pub token: Vec<u8>,
    pub scope_country: u32,
    pub scope_state: i32,
}
typed_codec!(StateDynamicModifierRow, 64, |row| (row.state as u128) << 32 | row.ordinal as u128,
 |key| Self { state: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (token): Bytes, 1 => (scope_country): U32, 2 => (scope_state): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryCommandPowerRow {
    pub canonical_slot: u32,
    pub allocated_command_power_raw: i64,
    pub max_command_power: Option<i64>,
    pub max_command_power_factor: Option<i64>,
    pub ace_effectiveness_factor: Option<i64>,
    pub air_ace_bonuses_factor: Option<i64>,
}
typed_codec!(CountryCommandPowerRow, 32, |row| row.canonical_slot as u128, |key| Self { canonical_slot: key as u32, ..Default::default() },
 { 0 => (allocated_command_power_raw): I64, 1 => (max_command_power): OptionalI64, 2 => (max_command_power_factor): OptionalI64, 3 => (ace_effectiveness_factor): OptionalI64, 4 => (air_ace_bonuses_factor): OptionalI64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PeaceConferenceRow {
    pub id: u64,
    pub ordinal: u32,
    pub current_turn: i32,
    pub finalized: u8,
}
typed_codec!(PeaceConferenceRow, 64, |row| row.id as u128, |key| Self { id: key as u64, ordinal: Default::default(), current_turn: Default::default(), finalized: Default::default() },
 { 0 => (ordinal): U32, 1 => (current_turn): I32, 2 => (finalized): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LexicalTokenRow {
    pub token: u32,
    pub literal: Vec<u8>,
}
typed_codec!(LexicalTokenRow, 32, |row| row.token as u128, |key| Self { token: key as u32, literal: Default::default() },
 { 0 => (literal): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ModifierDefinitionRow {
    pub index: u32,
    pub token: u32,
    pub name_key: Vec<u8>,
    pub postfix_key: Vec<u8>,
    pub argument: Vec<u8>,
    pub flags: u32,
    pub category_mask: u32,
    pub feature_selector: u32,
    pub precision: i32,
    pub eligibility: u8,
}
typed_codec!(ModifierDefinitionRow, 32, |row| row.index as u128, |key| Self { index: key as u32, ..Default::default() },
 { 0 => (token): U32, 1 => (name_key): Bytes, 2 => (postfix_key): Bytes, 3 => (argument): Bytes, 4 => (flags): U32, 5 => (category_mask): U32, 6 => (feature_selector): U32, 7 => (precision): I32, 8 => (eligibility): U8 },
 |row| if row.eligibility > 1 { Err(invalid("modifier definition eligibility callback state is neither null nor the getter")) } else { Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryStabilityInputRow {
    pub country: u32,
    pub recruitment_ratio: Recorded<Option<i64>>,
    pub exile_host: Recorded<Option<i32>>,
    pub exile_legitimacy: Recorded<Option<i64>>,
    pub exile_leaders: Recorded<Option<i32>>,
    pub exile_manpower: Recorded<Option<i32>>,
    pub exile_manpower_cap: Recorded<Option<i32>>,

    pub stability_raw: i64,
    pub war_support_raw: i64,
    pub at_war: u8,
    pub offensive_war: u8,
    pub defensive_war: u8,
    pub ruling_party_popularity_raw: i64,
    pub propaganda_stability_q15: i64,
    pub coastal_protection_stability_raw: i64,
    pub bombing_war_support_q15: i64,
    pub heroes_war_support_q15: i64,
    pub convoys_war_support_q15: i64,
    pub propaganda_war_support_q15: i64,
    pub interception_sum_raw: i64,
    pub world_tension_raw: i64,
    pub base_stability_war_factor: i64,
    pub base_stability_party_popularity_factor: i64,
    pub min_stability: i64,
    pub max_stability: i64,
    pub war_support_offensive_war: i64,
    pub war_support_defensive_war: i64,
    pub war_support_tension_impact: i64,
    pub min_war_support: i64,
    pub max_war_support: i64,
    pub interception_war_support_scale_q15: i64,
    pub interception_bombing_war_support_impact: i64,
    pub base_max_command_power: i64,
    pub stability_factor: Option<i64>,
    pub party_popularity_stability_factor: Option<i64>,
    pub war_stability_factor: Option<i64>,
    pub offensive_war_stability_factor: Option<i64>,
    pub defensive_war_stability_factor: Option<i64>,
    pub war_support_factor: Option<i64>,
    pub stability_weekly: Option<i64>,
    pub war_support_weekly: Option<i64>,
    pub keys: Vec<u32>,
    pub children: u32,
    pub language: Vec<u8>,
}
impl CountryStabilityInputRow {
    pub const KEY_NAMES: [&'static [u8]; 10] = [
        b"stability_factor",
        b"party_popularity_stability_factor",
        b"war_stability_factor",
        b"offensive_war_stability_factor",
        b"defensive_war_stability_factor",
        b"war_support_factor",
        b"stability_weekly",
        b"war_support_weekly",
        b"max_command_power",
        b"max_command_power_mult",
    ];
    pub fn totals(&self) -> [Option<i64>; 8] {
        [
            self.stability_factor,
            self.party_popularity_stability_factor,
            self.war_stability_factor,
            self.offensive_war_stability_factor,
            self.defensive_war_stability_factor,
            self.war_support_factor,
            self.stability_weekly,
            self.war_support_weekly,
        ]
    }
}
typed_codec!(CountryStabilityInputRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (stability_raw): I64, 1 => (war_support_raw): I64, 2 => (at_war): U8, 3 => (offensive_war): U8, 4 => (defensive_war): U8, 5 => (ruling_party_popularity_raw): I64, 6 => (propaganda_stability_q15): I64, 7 => (coastal_protection_stability_raw): I64, 8 => (bombing_war_support_q15): I64, 9 => (heroes_war_support_q15): I64, 10 => (convoys_war_support_q15): I64, 11 => (propaganda_war_support_q15): I64, 12 => (interception_sum_raw): I64, 13 => (world_tension_raw): I64, 14 => (base_stability_war_factor): I64, 15 => (base_stability_party_popularity_factor): I64, 16 => (min_stability): I64, 17 => (max_stability): I64, 18 => (war_support_offensive_war): I64, 19 => (war_support_defensive_war): I64, 20 => (war_support_tension_impact): I64, 21 => (min_war_support): I64, 22 => (max_war_support): I64, 23 => (interception_war_support_scale_q15): I64, 24 => (interception_bombing_war_support_impact): I64, 25 => (base_max_command_power): I64, 26 => (stability_factor): OptionalI64, 27 => (party_popularity_stability_factor): OptionalI64, 28 => (war_stability_factor): OptionalI64, 29 => (offensive_war_stability_factor): OptionalI64, 30 => (defensive_war_stability_factor): OptionalI64, 31 => (war_support_factor): OptionalI64, 32 => (stability_weekly): OptionalI64, 33 => (war_support_weekly): OptionalI64, 34 => (keys): U32List, 35 => (children): U32, 36 => (language): Bytes, 37 => (recruitment_ratio): OptionalI64, 38 => (exile_host): OptionalI32, 39 => (exile_legitimacy): OptionalI64, 40 => (exile_leaders): OptionalI32, 41 => (exile_manpower): OptionalI32, 42 => (exile_manpower_cap): OptionalI32 },
 |row| if row.keys.len() != CountryStabilityInputRow::KEY_NAMES.len() || row.at_war > 1 || row.offensive_war > 1 || row.defensive_war > 1 || row.keys.iter().zip(row.totals()).any(|(key, total)| (*key == u32::MAX) != total.is_none()) { Err(invalid("invalid country stability inputs")) } else { Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryStabilityLineRow {
    pub country: u32,
    pub line: u32,
    pub key: u32,
    pub kind: u8,
    pub identity: Vec<u8>,
    pub identity_id: u32,
    pub secondary: i64,
    pub value: i64,
    pub text: Vec<u8>,
}
impl CountryStabilityLineRow {
    pub const UNPAIRED: u8 = 0;
    pub const IDEA: u8 = 1;
    pub const COUNTRY_LEADER: u8 = 2;
    pub const IDEOLOGY: u8 = 3;
    pub const ADVISOR: u8 = 4;
    pub const STATIC: u8 = 5;
    pub const STATIC_SCALED: u8 = 6;
    pub const DYNAMIC_MODIFIER: u8 = 7;
    pub const DYNAMIC_MODIFIER_VALUES: u8 = 8;
}
typed_codec!(CountryStabilityLineRow, 96, |row| (row.country as u128) << 64 | (row.line as u128) << 32 | row.key as u128, |key| Self { country: (key >> 64) as u32, line: (key >> 32) as u32, key: key as u32, ..Default::default() },
 { 0 => (kind): U8, 1 => (identity): Bytes, 2 => (identity_id): U32, 3 => (secondary): I64, 4 => (value): I64, 5 => (text): Bytes },
 |row| if row.kind > Self::DYNAMIC_MODIFIER_VALUES || (row.kind != Self::UNPAIRED && !row.text.is_empty()) { Err(invalid("invalid country stability line")) } else { Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WorldTensionRow {
    pub total_raw: i64,
    pub source_count: i32,
    pub country_aggregate_count: i32,
}
typed_codec!(WorldTensionRow, 0, |_row| 0, |_key| Self { total_raw: Default::default(), source_count: Default::default(), country_aggregate_count: Default::default() },
 { 0 => (total_raw): I64, 1 => (source_count): I32, 2 => (country_aggregate_count): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ThreatSourceRow {
    pub ordinal: u32,
    pub country: u32,
    pub second_country: u32,
    pub current_raw: i64,
    pub final_raw: i64,
    pub daily_raw: i64,
    pub date: i32,
    pub reason: Vec<u8>,
}
typed_codec!(ThreatSourceRow, 32, |row| row.ordinal as u128, |key| Self { ordinal: key as u32, country: Default::default(), second_country: Default::default(), current_raw: Default::default(), final_raw: Default::default(), daily_raw: Default::default(), date: Default::default(), reason: Default::default() },
 { 0 => (country): U32, 1 => (second_country): U32, 2 => (current_raw): I64, 3 => (final_raw): I64, 4 => (daily_raw): I64, 5 => (date): I32, 6 => (reason): Bytes }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ThreatCountryAggregateRow {
    pub country_slot: u32,
    pub total_raw: i64,
}
typed_codec!(ThreatCountryAggregateRow, 32, |row| row.country_slot as u128, |key| Self { country_slot: key as u32, total_raw: Default::default() },
 { 0 => (total_raw): I64 }, |_row| Ok(()));
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IncomingDiplomaticActionRow {
    pub country: u32,
    pub ordinal: u32,
    pub handle: u64,
    pub status: u8,
    pub action_type: i32,
    pub actor: u32,
    pub original_actor: u32,
    pub recipient: u32,
    pub original_recipient: u32,
    pub hour: i32,
    pub last_command_hour: i32,
}
typed_codec!(IncomingDiplomaticActionRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, handle: Default::default(), status: Default::default(), action_type: Default::default(), actor: Default::default(), original_actor: Default::default(), recipient: Default::default(), original_recipient: Default::default(), hour: Default::default(), last_command_hour: Default::default() },
 { 0 => (handle): U64, 1 => (status): U8, 2 => (action_type): I32, 3 => (actor): U32, 4 => (original_actor): U32, 5 => (recipient): U32, 6 => (original_recipient): U32, 7 => (hour): I32, 8 => (last_command_hour): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RelationOpinionRow {
    pub owner_country: u32,
    pub target_country: u32,
    pub opinion: i32,
}
typed_codec!(RelationOpinionRow, 64, |row| (row.owner_country as u128) << 32 | row.target_country as u128, |key| Self { owner_country: (key >> 32) as u32, target_country: key as u32, opinion: Default::default() },
 { 0 => (opinion): I32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RelationTupleRow {
    pub owner_country: u32,
    pub target_country: u32,
    pub type_id: u32,
    pub ordinal: u16,
    pub first_country: u32,
    pub second_country: u32,
    pub start_hour: i32,
    pub end_hour: i32,
    pub cancel: u8,
    pub retired: u8,
}
typed_codec!(RelationTupleRow, 112, |row| (row.owner_country as u128) << 80 | (row.target_country as u128) << 48 | (row.type_id as u128) << 16 | row.ordinal as u128, |key| Self { owner_country: (key >> 80) as u32, target_country: (key >> 48) as u32, type_id: (key >> 16) as u32, ordinal: key as u16, first_country: Default::default(), second_country: Default::default(), start_hour: Default::default(), end_hour: Default::default(), cancel: Default::default(), retired: Default::default() },
 { 0 => (first_country): U32, 1 => (second_country): U32, 2 => (start_hour): I32, 3 => (end_hour): I32, 4 => (cancel): U8, 5 => (retired): U8 }, |_row| Ok(()));

fn split(pair: u64) -> (u32, u32) {
    ((pair >> 32) as u32, pair as u32)
}
