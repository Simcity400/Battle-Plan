use super::*;

/// Retired: only create_wargoal's effect-description hover read it. Kept for
/// older recordings' framing.
///
/// A country's owned war goals in native order. Recipient is the war goal's +0x3C
/// reference; goal_type is the definition's lexer token (+0x40 -> +8), not a database
/// ordinal.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OwnedWargoalRow {
    pub country: u32,
    pub ordinal: u32,
    pub recipient: u32,
    pub goal_type: i32,
    /// Stored state IDs in native order, including zero for null entries.
    pub states: Vec<u32>,
}
typed_codec!(OwnedWargoalRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (recipient): U32, 1 => (goal_type): I32, 2 => (states): U32List }, |_row| Ok(()));

/// Research modifier operands, refreshed with the country slice.
/// index==u32::MAX is shared country data.
/// Values are native operands; analytics owns eligibility, clamping and sums.
/// Ordinal references below stay within the same country/slot scope.
///
/// Slot rows identify the exact technology. Modifier operands retain speed
/// components, source text, sharing witnesses, category filters, eligibility
/// metadata and loaded define values as native values; kind tags live in the
/// codec.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
    /// raw=base, auxiliary=country bonus, count=member roster length,
    /// source_id=native group ID, name=group token.
    pub const SHARING: u8 = 5;
    pub const LICENSE: u8 = 6;
    /// raw=number of selected scientist categories, not a computed speed total.
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
    /// raw=definition flags, auxiliary=native feature eligibility flags,
    /// count=feature selector, source_id=modified value ordinal.
    pub const MODIFIER_METADATA: u8 = 17;
    /// raw=native cached category value (zero if absent), source_id=category.
    pub const SCIENTIST_CATEGORY: u8 = 18;
    /// raw=current level, auxiliary=template maximum, count=group ordinal,
    /// source_id=raw member country reference. Each roster occurrence is retained.
    pub const SHARING_MEMBER: u8 = 19;
    /// raw=native classifier, count=member ordinal. An empty set remains empty.
    pub const SHARING_PREDECESSOR: u8 = 20;
    /// count=group ordinal, source_id=category hash, name=exact category string.
    pub const SHARING_FILTER: u8 = 21;
    /// source_id=technology category hash, name=exact category string.
    pub const SHARING_CATEGORY: u8 = 22;
    /// raw=native doctrine byte. Nonzero bypasses member/category dependencies.
    pub const SHARING_DOCTRINE: u8 = 23;
    /// Same identity/maximum as SHARING_MEMBER, but no technology instance.
    pub const SHARING_MISSING_MEMBER: u8 = 24;
}
typed_codec!(ResearchModifierRow, 96, |row| (row.country as u128) << 64 | (row.index as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 64) as u32, index: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (technology_handle): U32, 1 => (kind): U8, 2 => (raw): I64, 3 => (auxiliary): I64, 4 => (count): U32, 5 => (source_id): U32, 6 => (name): Bytes }, |row| if row.kind > Self::SHARING_MISSING_MEMBER { Err(invalid("invalid research modifier kind")) } else { Ok(()) });

/// Slot-local research capture, refreshed one country slice per hour.
///
/// Inputs are native values for the exact slot technology.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
    /// Exact template occurrence; historical absence cannot select a repeated handle.
    pub database_index: Recorded<i32>,
}
typed_codec!(ResearchInputRow, 64, |row| (row.country as u128) << 32 | row.index as u128, |key| Self { country: (key >> 32) as u32, index: key as u32, ..Default::default() },
 { 0 => (technology_handle): U32, 1 => (technology_present): U8, 2 => (token): Bytes, 3 => (classifier): U32, 4 => (level): I32, 5 => (progress_raw): I64, 6 => (cost_raw): I64, 7 => (cost_scale_raw): I64, 8 => (cost_debug): U8, 9 => (research_speed_raw): I64, 10 => (banked_days_raw): I64, 11 => (applied_days_raw): I64, 12 => (ahead_reduction_raw): I64, 13 => (speed_bonus_raw): I64, 14 => (bonus_count): U32, 15 => (database_index): I32 }, |_row| Ok(()));

/// Slot-local applied research bonuses, refreshed with their country slice.
///
/// Applied-bonus attribution retains native values for the exact slot
/// technology.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// SliceReads: the hour each owner's rows of a sliced table were read at,
/// key table:owner (64 bits). One row per sliced owned table and owner with
/// rows, and per owner a complete read covered that returned none (its
/// fingerprint scan saw it); an owner without rows and without a row here has
/// no known read. The recorder updates only the owners read at each hour, so
/// unchanged rows of the sliced tables themselves stay unchanged.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SliceReadRow {
    /// `TableId` index of the sliced table.
    pub table: u32,
    /// Native country ID, canonical country slot or native state ID owning
    /// the rows (the table's owner kind).
    pub owner: u32,
    /// Hour whose boundary this owner's rows were read at.
    pub sample_hour: i32,
}
typed_codec!(SliceReadRow, 64, |row| (row.table as u128) << 32 | row.owner as u128,
 |key| Self { table: (key >> 32) as u32, owner: key as u32, ..Default::default() },
 { 0 => (sample_hour): I32 }, |_row| Ok(()));

/// RelationLinks: directed diplomacy slots the map and raid gate test for
/// presence, one row per status with a bit set: the status `country`'s
/// matrix holds toward canonical slot `target_index`. Bit 1 is the attache
/// link for shared fog, bit 2 air-base access for its icon, bit 8 military
/// access for province movement and the naval-base icon, and bit 16 docking
/// rights for the naval-base icon's other access test. Bit 4 is not
/// recorded: the co-belligerent byte changes only the raid army search's
/// edge cost, never whether it reaches its source. Absent pairs have no bit set.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RelationLinkRow {
    /// Raw reference of the country whose matrix holds the status.
    pub country: u32,
    /// Matrix index: the target's canonical country slot.
    pub target_index: u32,
    pub links: u8,
    /// Exact current subject-access inputs; self-keyed complete_owner rows
    /// certify absent pairs. Historical recordings carry NotRecorded.
    pub transport_subject: Recorded<Option<TransportSubjectAccess>>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportSubjectAccess {
    pub sample_hour: PacketHour,
    pub complete_owner: bool,
    pub target_raw: Option<u32>,
    pub subject: Option<ZeroSubjectInputs>,
}
value_struct!(TransportSubjectAccess {
    sample_hour: PacketHour, complete_owner: bool, target_raw: Option<u32>, subject: Option<ZeroSubjectInputs>,
});
opaque_value!(Option<TransportSubjectAccess>);
impl RelationLinkRow {
    pub const ATTACHE: u8 = 1;
    pub const AIR_BASE_ACCESS: u8 = 2;
    /// Bit 4 is unused: the co-belligerent byte is not recorded.
    pub const MILITARY_ACCESS: u8 = 8;
    pub const DOCKING_RIGHTS: u8 = 16;
}
typed_codec!(RelationLinkRow, 64, |row| (row.country as u128) << 32 | row.target_index as u128,
 |key| Self { country: (key >> 32) as u32, target_index: key as u32, ..Default::default() },
 { 0 => (links): U8, 1 => (transport_subject): Opaque }, |_row| Ok(()));

/// BorderConflicts: key province id (32 bits); one row per province with
/// a nonempty border-conflict set, exactly the provinces with
/// `ProvinceRow::MOVE_BORDER_CONFLICT`; no row otherwise.
///
/// `groups` are the orders-group handles its combats admit, in stored order
/// without repeats: for each border-war combat, defending then attacking
/// side's reference, kept when it resolves to a group with a nonzero
/// border-war operand. Each handle has `Divisions.orders_group` encoding.
/// An army may enter only when its location province is also in a conflict
/// or its `Divisions.orders_group` is in `groups`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BorderConflictRow {
    pub province: u32,
    pub groups: Vec<u64>,
}
typed_codec!(BorderConflictRow, 32, |row| row.province as u128,
 |key| Self { province: key as u32, ..Default::default() },
 { 0 => (groups): U64List }, |_row| Ok(()));

/// CountryWars: key raw country reference (32 bits); one row per country whose
/// `can_occupy_non_war` byte is nonzero or whose enemy vector was read. No row
/// means a zero byte and a derived enemy set.
///
/// `can_occupy_non_war` is the stored rule-16 byte (`CAN_OCCUPY_NON_WAR`).
/// When nonzero and no live war relation joins the countries, neutral
/// occupation passes iff their enemy sets share a country.
///
/// As a set, the enemy vector holds the other endpoint of every active war
/// relation the country owns. Its only runtime writer runs for both
/// endpoints of every war added or removed; faction pruning and third
/// parties' wars change only its order, while the occupation gate compares
/// sets. A reader derives it from `WarRelations`. `enemies_read` marks rows
/// where the vector is read instead, as raw links in stored order: a
/// civil-war revolter whose list stays unrebuilt until its own declaration,
/// and a country owning a retired war relation that could linger in a side list.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// Hourly companion of States. All states are present, including false flags.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StateFlagRow {
    pub state: u32,
    /// Native byte, not normalized; zero is false and nonzero is true.
    pub demilitarized: u8,
    /// Stored border-conflict active byte, sampled for every state as the selection gate.
    pub border_conflict_active: Recorded<u8>,
    /// Active states only: attacker/defender raw signed country references from
    /// every tree member, in in-order traversal order. Empty is a successful
    /// empty tree; inactive states have no pairs. The province BorderConflicts
    /// table is separate.
    pub border_conflict_countries: Recorded<Vec<[i32; 2]>>,
}
opaque_value!(Vec<[i32; 2]>);
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
pub struct ScriptVariableEntry {
    pub name: Vec<u8>,
    pub value_raw: i64,
}
value_struct!(ScriptVariableEntry {name: Vec<u8>, value_raw: i64});

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
 { 0 => (tree): Opaque, 1 => (current): Opaque, 2 => (continuous): Opaque, 3 => (progress): I64, 4 => (paused): U8, 5 => (completed_count): I32 }, |_row| Ok(()));

/// CompletedFocuses: key country:ordinal (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CompletedFocusRow {
    pub country: u32,
    pub ordinal: u32,
    pub token: Vec<u8>,
}
typed_codec!(CompletedFocusRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, token: Default::default() },
 { 0 => (token): Bytes }, |_row| Ok(()));

/// ResearchTechnologies: key country:database_index (64 bits), 7 payload
/// mask bits. Refreshed one country slice per hour; the read hour is in
/// SliceReads.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResearchTechnologyRow {
    pub country: u32,
    pub technology_handle: u32,
    /// Exact indexed template occurrence.
    pub database_index: i32,
    /// A nonzero applied-bonus count includes this instance in the save.
    pub applied_bonus_count: i32,
    pub classifier: u32,
    pub level: i32,
    pub researched: u8,
    pub progress_raw: i64,
    pub ahead_reduction_years_raw: i64,
}
typed_codec!(ResearchTechnologyRow, 64, |row| (row.country as u128) << 32 | row.database_index as u32 as u128, |key| Self { country: (key >> 32) as u32, database_index: key as u32 as i32, ..Default::default() },
 { 0 => (classifier): U32, 1 => (level): I32, 2 => (researched): U8, 3 => (progress_raw): I64, 4 => (ahead_reduction_years_raw): I64, 5 => (technology_handle): U32, 6 => (applied_bonus_count): I32 }, |_row| Ok(()));

/// ResearchSlots: key country:index (64 bits), 6 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResearchSlotRow {
    pub country: u32,
    pub index: u32,
    pub technology_handle: u32,
    pub technology_present: u8,
    pub research_speed_raw: i64,
    pub banked_days_raw: i64,
    pub applied_days_raw: i64,
    /// Exact template occurrence; historical absence cannot select a repeated handle.
    pub database_index: Recorded<i32>,
}
typed_codec!(ResearchSlotRow, 64, |row| (row.country as u128) << 32 | row.index as u128, |key| Self { country: (key >> 32) as u32, index: key as u32, technology_handle: Default::default(), technology_present: Default::default(), research_speed_raw: Default::default(), banked_days_raw: Default::default(), applied_days_raw: Default::default(), database_index: Default::default() },
 { 0 => (technology_handle): U32, 1 => (technology_present): U8, 2 => (research_speed_raw): I64, 3 => (banked_days_raw): I64, 4 => (applied_days_raw): I64, 5 => (database_index): I32 }, |_row| Ok(()));

/// TechnologyDefinitions: key database_index (32 bits), 5 payload mask bits.
///
/// The keyed sliced table has no owner or read hour. A complete nonempty read
/// replaces it; an incomplete read replaces only returned keys, and an empty
/// read keeps it (sliced::replaces_whole).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TechnologyDefinitionRow {
    pub technology_handle: u32,
    /// Distinct stored template occurrence identity, even when handles coincide.
    pub database_index: i32,
    pub year: i32,
    pub max_level: i32,
    pub research_cost_raw: i64,
    pub id: Vec<u8>,
}
typed_codec!(TechnologyDefinitionRow, 32, |row| row.database_index as u32 as u128, |key| Self { database_index: key as u32 as i32, technology_handle: Default::default(), year: Default::default(), max_level: Default::default(), research_cost_raw: Default::default(), id: Default::default() },
 { 0 => (technology_handle): U32, 1 => (year): I32, 2 => (max_level): I32, 3 => (research_cost_raw): I64, 4 => (id): Bytes }, |_row| Ok(()));

/// Politics: key country (32 bits), 7 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PoliticsRow {
    pub country: u32,
    pub last_election: i32,
    /// Retired: only the save comparison read these two; `has_elections`
    /// records what the trigger tests of them.
    pub election_frequency: RetiredI32,
    pub elections_allowed: RetiredU8,
    pub capital: i32,
    pub ruling_party: Option<u32>,
    /// Stored research-location province, preferred to the capital unless zero.
    pub original_capital: i32,
    /// What the `has_elections` trigger tests: the elections byte `+0xEC` is nonzero and
    /// the frequency `+0xE8` is positive
    /// ([`crate::rows::compatibility::HAS_ELECTIONS_FORMAT`]).
    pub has_elections: Recorded<bool>,
}
typed_codec!(PoliticsRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (last_election): I32, 1 => (election_frequency): I32, 2 => (elections_allowed): U8, 3 => (capital): I32, 4 => (ruling_party): OptionalU32, 5 => (original_capital): I32, 6 => (has_elections): Bool }, |_row| Ok(()));

/// PoliticalParties: key country:id (64 bits), 7 payload mask bits.
///
/// With name_default set, the game tries `<tag>_<ideology group>_party` (and
/// `_party_long` for the long name when present), then the original tag, then
/// the ideology group's own name. Otherwise it localises the stored keys.
/// Consumers resolve these through the recorded content stack.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PoliticalPartyRow {
    pub country: u32,
    pub id: u32,
    pub popularity: i64,
    pub is_ruling: u8,
    pub token: Vec<u8>,
    /// The party's own tag, as an index in the game's tag array: the owner's
    /// raw link id (`Countries.id`), which for a dynamic country is not its slot.
    pub tag: u32,
    /// 1 when the game names the party by its default rule, 0 when it localises
    /// `name` and `long_name` (`set_party_name` clears it).
    pub name_default: u8,
    /// Stored name localisation key; meaningful only when `name_default` is 0.
    pub name: Vec<u8>,
    /// Stored long-name localisation key; meaningful only when `name_default` is 0.
    pub long_name: Vec<u8>,
}
typed_codec!(PoliticalPartyRow, 64, |row| (row.country as u128) << 32 | row.id as u128, |key| Self { country: (key >> 32) as u32, id: key as u32, ..Default::default() },
 { 0 => (popularity): I64, 1 => (is_ruling): U8, 2 => (token): Bytes, 3 => (tag): U32, 4 => (name_default): U8, 5 => (name): Bytes, 6 => (long_name): Bytes }, |_row| Ok(()));

/// ActiveIdeas: key country:ordinal (64 bits), 1 payload mask bits.
///
/// Read hourly because effects can change active ideas at any hour and triggers
/// see those changes in that hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ActiveIdeaRow {
    pub country: u32,
    pub ordinal: u32,
    pub token: Vec<u8>,
}
typed_codec!(ActiveIdeaRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, token: Default::default() },
 { 0 => (token): Bytes }, |_row| Ok(()));

/// Stored political-action roster order; remaining days are decremented
/// rather than stored as an expiry date.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TimedIdeaRow {
    pub country: u32,
    pub ordinal: u32,
    pub token: Vec<u8>,
    pub days: i32,
}
typed_codec!(TimedIdeaRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (token): Bytes, 1 => (days): I32 }, |_row| Ok(()));

/// Stored decision random-count roster order. An absent entry in a complete
/// table means no stored count, not a read failure.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DecisionRandomItemRow {
    pub country: u32,
    pub ordinal: u32,
    pub token: Vec<u8>,
    pub count: i32,
    /// Raw numeric target bits at entry+0x14, including signed sentinels.
    pub target: u32,
}
typed_codec!(DecisionRandomItemRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (token): Bytes, 1 => (count): I32, 2 => (target): U32 }, |_row| Ok(()));

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
 { 0 => (taken): U32, 1 => (re_enable): U32, 2 => (remove): U32, 3 => (timed): U32, 4 => (targeted_timed): U32, 5 => (targeted): U32 }, |_row| Ok(()));

/// Decisions: key country:container:ordinal (72 bits), 8 payload mask bits.
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
    /// Description seed: None outside demand; Some(None) means lookup found no
    /// status seed; Some(Some(n)) preserves the stored seed. The definition's
    /// random seed is derived from its token.
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
 { 0 => (scope_country): U32, 1 => (scope_state): I32, 2 => (days_remaining): I32, 3 => (enabled): U8, 4 => (token): Bytes, 5 => (values): Opaque }, |_row| Ok(()));

/// TechSharingGroups: key id (32 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TechSharingGroupRow {
    pub id: u32,
    pub base_bonus: i64,
    pub member_count: u32,
}
typed_codec!(TechSharingGroupRow, 32, |row| row.id as u128, |key| Self { id: key as u32, base_bonus: Default::default(), member_count: Default::default() },
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
 { 0 => (country): U32, 1 => (bonus): I64 }, |_row| Ok(()));

/// PowerBalances: key ordinal (32 bits), 6 payload mask bits.
///
/// Balance-of-power presentation operands refresh with the hour; cadence.rs
/// governs sampling.
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
 { 0 => (country): U32, 1 => (daily_raw): I64, 2 => (weekly_raw): I64, 3 => (daily_present): Bool, 4 => (weekly_present): Bool, 5 => (daily_flags): U32, 6 => (weekly_flags): U32 }, |_row| Ok(()));

/// PowerBalanceRanges: key record:ordinal (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PowerBalanceRangeRow {
    pub record: u32,
    pub ordinal: u32,
    pub identity: Vec<u8>,
}
typed_codec!(PowerBalanceRangeRow, 64, |row| (row.record as u128) << 32 | row.ordinal as u128, |key| Self { record: (key >> 32) as u32, ordinal: key as u32, identity: Default::default() },
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
 { 0 => (side): Bytes, 1 => (gfx): Bytes }, |_row| Ok(()));

/// Accepted positive native experience awards during the preceding hourly interval.
/// Instance is the native country identity word, not a tag. The bank cap and spending
/// never reduce these awards. `complete` also needs the same authenticated owner at both
/// hourly boundaries. Every current country is present, even at zero or in its first
/// incomplete interval.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
 { 0 => (army_raw): I64, 1 => (army_daily_raw): I64, 2 => (army_daily_training_raw): I64, 3 => (navy_raw): I64, 4 => (navy_daily_raw): I64, 5 => (navy_daily_training_raw): I64, 6 => (air_raw): I64, 7 => (air_daily_raw): I64, 8 => (armies_for_training_raw): I64 }, |_row| Ok(()));

/// Each native occupation-data member; a null group law still proves membership.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OccupationGroupLaw {
    pub occupied: u32,
    /// Current id through the retained +0x18 country pointer; None is a capture fault.
    pub scope_country: Option<u32>,
    pub law: Option<Vec<u8>>,
}
value_struct!(OccupationGroupLaw { occupied: u32, scope_country: Option<u32>, law: Option<Vec<u8>> });
opaque_value!(Option<Vec<OccupationGroupLaw>>);

/// OccupationSettings: key country (32 bits), with its native group-law inventory.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OccupationSettingsRow {
    pub country: u32,
    pub garrison_template: Option<u64>,
    pub default_law_present: u8,
    pub law: Vec<u8>,
    /// Occupation-group order and each group's raw country reference. None is
    /// a counted capture refusal.
    pub occupied: Recorded<Option<Vec<u32>>>,
    /// Older recordings lack this inventory; None is a failed new capture.
    pub group_laws: Recorded<Option<Vec<OccupationGroupLaw>>>,
    /// The actual database fallback token (+0x48), independent of authored ordering.
    pub fallback_law: Recorded<Option<Vec<u8>>>,
}
typed_codec!(OccupationSettingsRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (garrison_template): OptionalU64, 1 => (default_law_present): U8, 2 => (law): Bytes, 3 => (occupied): Opaque, 4 => (group_laws): Opaque, 5 => (fallback_law): Opaque }, |_row| Ok(()));

/// StateOccupationLaws: key controller:occupied:state (96 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StateOccupationLawRow {
    pub controller: u32,
    pub occupied: u32,
    pub state: u32,
    pub name: Vec<u8>,
}
typed_codec!(StateOccupationLawRow, 96, |row| (row.controller as u128) << 64 | (row.occupied as u128) << 32 | row.state as u128, |key| Self { controller: (key >> 64) as u32, occupied: (key >> 32) as u32, state: key as u32, name: Default::default() },
 { 0 => (name): Bytes }, |_row| Ok(()));

/// One ordered required-pool entry and the strength getter's lookups.
/// Strength weights the required amount; quantity uses active pointer
/// identity and archetype lookup takes the first ordered signed-ID match.
/// No addresses persist.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GarrisonEquipment {
    pub equipment_token: u32,
    pub archetype_token: u32,
    pub archetype_active: u8,
    pub required_raw: i64,
    pub required_lookup_raw: i64,
    pub allocated_raw: i64,
    pub weight_raw: i64,
}
value_struct!(GarrisonEquipment {
    equipment_token: u32,
    archetype_token: u32,
    archetype_active: u8,
    required_raw: i64,
    required_lookup_raw: i64,
    allocated_raw: i64,
    weight_raw: i64
});
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GarrisonEquipmentInputs {
    pub rows: Vec<GarrisonEquipment>,
    /// A local acquisition refusal, rather than a fabricated empty pool.
    pub failure: Option<Vec<u8>>,
}
value_struct!(GarrisonEquipmentInputs { rows: Vec<GarrisonEquipment>, failure: Option<Vec<u8>> });
opaque_value!(GarrisonEquipmentInputs);

/// StateGarrisons: key controller:state (64 bits), appended strength operands.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
 { 0 => (country): U32, 1 => (level): I64, 2 => (speed): I64, 3 => (compliance): I64, 4 => (compliance_speed): I64, 5 => (active): U8 }, |_row| Ok(()));

/// CountryCollaboration: key source:target (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryCollaborationRow {
    pub source: u32,
    pub target: u32,
    pub progress: i64,
}
typed_codec!(CountryCollaborationRow, 64, |row| (row.source as u128) << 32 | row.target as u128, |key| Self { source: (key >> 32) as u32, target: key as u32, progress: Default::default() },
 { 0 => (progress): I64 }, |_row| Ok(()));

/// Capitulation: key country (32 bits), 6 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CapitulationRow {
    pub country: u32,
    pub capitulated: u8,
    pub capitulated_hour: i32,
    /// Retired fields 2-5: only the save comparison read them.
    pub last_surrender_hour: RetiredI32,
    pub faction_join_hour: RetiredI32,
    pub legitimacy_raw: RetiredI64,
    pub exile_army_leaders: RetiredI32,
}
typed_codec!(CapitulationRow, 32, |row| row.country as u128, |key| Self { country: key as u32, capitulated: Default::default(), capitulated_hour: Default::default(), last_surrender_hour: Default::default(), faction_join_hour: Default::default(), legitimacy_raw: Default::default(), exile_army_leaders: Default::default() },
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
 { 0 => (score): I64, 1 => (lower_score): I64, 2 => (upper_score): I64, 3 => (last_modified_bits): U32, 4 => (state): Bytes, 5 => (previous): Opaque, 6 => (next): Opaque }, |_row| Ok(()));

/// ScriptFlags: key scope_kind:scope_id:token (72 bits), 3 payload mask bits.
///
/// Scope kinds: 0 global (scope_id 0), 1 country (raw country reference),
/// 2 state (state id), 3 character (Characters id and its own flag manager).
/// Every complete table holds every scope; a character without flags has
/// no rows.
///
/// Flags are read hourly: effects set or clear them between ticks, and triggers
/// see the change in that hour. ActiveIdeas has the same requirement.
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
 { 0 => (set_hour): I32, 1 => (value): I16, 2 => (remaining_days): I16 }, |_row| Ok(()));

/// ScriptVariables: key scope_kind:scope_id (40 bits).
///
/// Scope kinds: 0 global (scope_id 0), 1 country (raw country reference),
/// 2 state (state id), 3 character (Characters id and its embedded store).
/// Every complete table holds a row for every walked scope, empty stores
/// included; `entries` are sorted by their exact key bytes.
///
/// Persistent keys and fixed5 values stay as stored. Read hourly because
/// effects can change them at any hour. Built-in game variables are computed on
/// demand, not stored or recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ScriptVariablesRow {
    pub scope_kind: u8,
    pub scope_id: u32,
    pub entries: Vec<ScriptVariableEntry>,
    /// Raw scoped-variable random seed. Construction sets 1; leaving a scoped
    /// variable context writes it.
    pub seed: Recorded<Option<u32>>,
}
typed_codec!(ScriptVariablesRow, 40, |row| (row.scope_kind as u128) << 32 | row.scope_id as u128, |key| Self { scope_kind: (key >> 32) as u8, scope_id: key as u32, ..Default::default() },
 { 0 => (entries): Opaque, 1 => (seed): OptionalU32 }, |_row| Ok(()));

/// Saved-global registry in first-match order. The mask records all nine
/// live references, region and combatant presence, plus whether the writer
/// omits the intern-ID-0 name. Unit-leader resolution falls back to the
/// character's reference, which UnitLeaders does not capture. None is no
/// leader; Some(0) refuses that row's null leader identity.
///
/// Registry ordinals are the keys: duplicate names, stored selectors and
/// character IDs stay recorded. Clearing a name deletes its record rather than
/// retaining the previous target.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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
    /// The first interned string has ID 0; the save omits its name.
    pub const UNSAVED_NAME: u16 = 1 << 11;
    /// Reference validity order, then the two pointer kinds. Combatant has no
    /// save field.
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
    /// The save key and refusal name of one kind bit.
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

/// PartyLeaders: key country:party:ordinal (96 bits). Hourly.
///
/// Each political party's country leaders in party-array then leader-vector
/// order, used by `has_country_leader`; the ruling party's first leader
/// supplies its display. `country` is the raw owner id, `party` the party's
/// ideology ID (PoliticalParties' `id`; join `is_ruling` there), `ordinal`
/// the leader's vector position. `character` is the leader's parent character
/// id, `None` for a null parent; `leader_id` is the role's script `id`
/// (-1 unless set), which a nonzero `has_country_leader` `id` operand matches.
///
/// The signed role key is found by leader pointer in the character's role map,
/// so it locates CharacterRoleTraits even after an ideology change.
/// `has_country_leader_with_trait` compares the recorded definition ordinals,
/// as the game compares pointers. The party writer saves leaders in order
/// without leader_id; the oracle takes that id from the leader's saved role.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PartyLeaderRow {
    pub country: u32,
    pub party: u32,
    pub ordinal: u32,
    pub character: Option<u32>,
    /// Signed key of the exact leader pointer in the character's role map, not its current ideology.
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

/// CountryCivilWars: key country:ordinal (64 bits). Hourly.
///
/// A country's civil-war target countries: raw tag references as stored
/// (`target`), in stored order, kept sorted and unique by insertion. An
/// empty array has no rows. `has_civil_war` holds when one of them, or an
/// enemy that lists the country, exists. `country` is the raw owner id.
/// The save writes one `civil_war_target` per entry, in order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryCivilWarRow {
    pub country: u32,
    pub ordinal: u32,
    pub target: u32,
}
typed_codec!(CountryCivilWarRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (target): U32 }, |_row| Ok(()));

/// StateCores: key state (32 bits). The state's raw core country references
/// in stored order. `is_core_of` compares raw references or canonical slots.
/// Resolve through CountryLinks. A state read with no cores has a row with
/// an empty list.
///
/// Sliced, with `StateResourceInputs` (family `raid_state`, owner: the state): core
/// insertion and removal are hooked, so the list equals the game's at every hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StateCoreRow {
    pub state: u32,
    pub cores: Vec<u32>,
}
typed_codec!(StateCoreRow, 32, |row| row.state as u128, |key| Self { state: key as u32, ..Default::default() },
 { 0 => (cores): U32List }, |_row| Ok(()));

/// StateDynamicModifiers: key state:ordinal (64 bits).
///
/// Stored dynamic modifiers in container order: the definition's name and
/// the entry's scope. `has_dynamic_modifier` without `scope=` counts only
/// the first entry of a definition, and only when it is unscoped.
///
/// Sliced, with `StateResourceInputs`: every writer of the list is hooked,
/// so it equals the game's at every hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// CountryCommandPower: key canonical slot. Every country's hourly modifier
/// totals read live by raid calculations, with its command power, read
/// every hour with the raid launch walk: allocated power and the effective
/// values of `max_command_power`, `max_command_power_mult`,
/// `ace_effectiveness_factor` and `air_ace_bonuses_factor`. Raid offers and
/// ace range terms, raid unit speeds (ace terms joined through
/// [`RaidSpeedInputRow`]'s `ace_owner` and `wing_country`), and stability
/// consumers (displayed maximum command power minus allocated power) use
/// the one record of each, joined by canonical slot at the same frame.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryCommandPowerRow {
    pub canonical_slot: u32,
    /// Stored fixed5 command power reserved by allocators.
    pub allocated_command_power_raw: i64,
    /// `max_command_power`: the getter value of the key resolved by its
    /// script name, `None` when the running game registers no key under that
    /// name. CountryStabilityInputs keeps only its lines.
    pub max_command_power: Option<i64>,
    /// `max_command_power_mult`, likewise.
    pub max_command_power_factor: Option<i64>,
    /// `ace_effectiveness_factor`, likewise: the ace owner's factor in its
    /// scaled modifier.
    pub ace_effectiveness_factor: Option<i64>,
    /// `air_ace_bonuses_factor`, likewise: the wing country's ace bonus.
    pub air_ace_bonuses_factor: Option<i64>,
}
typed_codec!(CountryCommandPowerRow, 32, |row| row.canonical_slot as u128, |key| Self { canonical_slot: key as u32, ..Default::default() },
 { 0 => (allocated_command_power_raw): I64, 1 => (max_command_power): OptionalI64, 2 => (max_command_power_factor): OptionalI64, 3 => (ace_effectiveness_factor): OptionalI64, 4 => (air_ace_bonuses_factor): OptionalI64 }, |_row| Ok(()));

/// PeaceConferences: key id (64 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PeaceConferenceRow {
    pub id: u64,
    pub ordinal: u32,
    pub current_turn: i32,
    pub finalized: u8,
}
typed_codec!(PeaceConferenceRow, 64, |row| row.id as u128, |key| Self { id: key as u64, ordinal: Default::default(), current_turn: Default::default(), finalized: Default::default() },
 { 0 => (ordinal): U32, 1 => (current_turn): I32, 2 => (finalized): U8 }, |_row| Ok(()));

/// LexicalTokens: key token (32 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LexicalTokenRow {
    pub token: u32,
    pub literal: Vec<u8>,
}
typed_codec!(LexicalTokenRow, 32, |row| row.token as u128, |key| Self { token: key as u32, literal: Default::default() },
 { 0 => (literal): Bytes }, |_row| Ok(()));

/// ModifierDefinitions: key index (32 bits), 8 payload mask bits.
///
/// One registered row of the modifier definition table: `token` is the
/// script-name lexer id, resolved through LexicalTokens; the three strings
/// are name and postfix localisation keys and the name's `$ARG$`
/// substitution, exact bytes. Flags, category mask, feature selector and
/// precision are stored words; consumers decode them. `eligibility` is the
/// process-wide feature eligibility callback read with the table (0 null,
/// 1 the flags getter), the same on every row.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

/// CountryStabilityInputs: key country (32 bits), 37 payload mask bits.
///
/// One country's raw inputs to displayed stability, war support and maximum
/// command power, read at the owner's slice hour (SliceReads). Values keep
/// their stored representation: `_raw` fields are fixed5 (x100000), `_q15`
/// fields are Q15 words converted to fixed5 by the getters. Defines retain
/// the loaded globals in their loader's storage format. Modifier totals
/// are effective country accumulator values for [`Self::KEY_NAMES`], `None`
/// when the running game registers no key under that name; `keys` holds
/// the resolved indices in the same order (`u32::MAX` when unregistered).
/// Consumers derive displayed values; nothing here is pre-combined.
///
/// Maximum command power joins CountryCommandPower's hourly max_command_power,
/// multiplier and allocated power by canonical slot at the same frame; the
/// displayed and clamped maximum is the maximum command power minus that allocation.
/// This row describes its SliceReads hour; the command-power row describes the
/// frame hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryStabilityInputRow {
    pub country: u32,
    pub stability_raw: i64,
    pub war_support_raw: i64,
    /// An owned war relation whose counterpart's diplomacy war-status operand
    /// is below one. Raw 0 or 1.
    pub at_war: u8,
    /// Whether an offensive war is present. Raw 0 or 1.
    pub offensive_war: u8,
    /// Whether a defensive war is present. Raw 0 or 1.
    pub defensive_war: u8,
    /// The ruling party's popularity, percent in fixed5 (0..10,000,000).
    pub ruling_party_popularity_raw: i64,
    pub propaganda_stability_q15: i64,
    /// The daily cached coastal-protection stability term.
    pub coastal_protection_stability_raw: i64,
    pub bombing_war_support_q15: i64,
    pub heroes_war_support_q15: i64,
    pub convoys_war_support_q15: i64,
    pub propaganda_war_support_q15: i64,
    /// Saturated window-zero channel-one interception sum, fixed5.
    pub interception_sum_raw: i64,
    /// The world-threat manager's stored total.
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
    /// Resolved key index per [`Self::KEY_NAMES`] entry, `u32::MAX` when absent.
    /// The last two, `max_command_power` and `max_command_power_mult`, have
    /// lines (CountryStabilityLines) but no total here: their getter values
    /// are [`CountryCommandPowerRow`]'s, the one record of them.
    pub keys: Vec<u32>,
    /// Children of the country modifier tree.
    pub children: u32,
    /// The game's active language key, for the display text of unpaired lines;
    /// empty when the getter returns null. Allocated command power, subtracted
    /// from the displayed maximum, is recorded once in
    /// [`CountryCommandPowerRow::allocated_command_power_raw`].
    pub language: Vec<u8>,
}
impl CountryStabilityInputRow {
    /// The modifier script names whose totals and lines are recorded, in the
    /// order of the total fields and of `keys`.
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
    /// Totals of the first eight [`Self::KEY_NAMES`], in that order; the
    /// last two keys' totals are [`CountryCommandPowerRow`]'s.
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
 { 0 => (stability_raw): I64, 1 => (war_support_raw): I64, 2 => (at_war): U8, 3 => (offensive_war): U8, 4 => (defensive_war): U8, 5 => (ruling_party_popularity_raw): I64, 6 => (propaganda_stability_q15): I64, 7 => (coastal_protection_stability_raw): I64, 8 => (bombing_war_support_q15): I64, 9 => (heroes_war_support_q15): I64, 10 => (convoys_war_support_q15): I64, 11 => (propaganda_war_support_q15): I64, 12 => (interception_sum_raw): I64, 13 => (world_tension_raw): I64, 14 => (base_stability_war_factor): I64, 15 => (base_stability_party_popularity_factor): I64, 16 => (min_stability): I64, 17 => (max_stability): I64, 18 => (war_support_offensive_war): I64, 19 => (war_support_defensive_war): I64, 20 => (war_support_tension_impact): I64, 21 => (min_war_support): I64, 22 => (max_war_support): I64, 23 => (interception_war_support_scale_q15): I64, 24 => (interception_bombing_war_support_impact): I64, 25 => (base_max_command_power): I64, 26 => (stability_factor): OptionalI64, 27 => (party_popularity_stability_factor): OptionalI64, 28 => (war_stability_factor): OptionalI64, 29 => (offensive_war_stability_factor): OptionalI64, 30 => (defensive_war_stability_factor): OptionalI64, 31 => (war_support_factor): OptionalI64, 32 => (stability_weekly): OptionalI64, 33 => (war_support_weekly): OptionalI64, 34 => (keys): U32List, 35 => (children): U32, 36 => (language): Bytes },
 |row| if row.keys.len() != CountryStabilityInputRow::KEY_NAMES.len() || row.at_war > 1 || row.offensive_war > 1 || row.defensive_war > 1 || row.keys.iter().zip(row.totals()).any(|(key, total)| (*key == u32::MAX) != total.is_none()) { Err(invalid("invalid country stability inputs")) } else { Ok(()) });

/// CountryStabilityLines: key country:line:key (96 bits), 6 payload mask bits.
///
/// One child of the country modifier tree and one tracked key it carries
/// a nonzero value for. `line` is the child's tree position: tooltip order
/// and source-addition order. `key` is the modifier key index (see
/// CountryStabilityInputs.keys), and `value` the child's exact entry. The
/// tracked keys are `good` keys, so a country's lines for a key sum to the
/// accumulator's raw total. `kind` and identity fields name the source
/// paired with the child by exact content; an `UNPAIRED` line keeps the
/// child's display text as localised at the last recalculation, in
/// CountryStabilityInputs.language, and is the only kind with `text`.
///
/// Sliced by country with CountryStabilityInputs; values describe the owner's
/// SliceReads hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CountryStabilityLineRow {
    pub country: u32,
    pub line: u32,
    pub key: u32,
    pub kind: u8,
    /// The source's script identity: an idea, advisor idea, ideology group or
    /// static modifier token, or a dynamic modifier's definition id.
    pub identity: Vec<u8>,
    /// A character id (country leader, advisor) or a dynamic modifier's scope
    /// country reference; zero otherwise.
    pub identity_id: u32,
    /// A dynamic modifier's scope state; zero otherwise.
    pub secondary: i64,
    pub value: i64,
    pub text: Vec<u8>,
}
impl CountryStabilityLineRow {
    /// The reader could not prove the child's source; `text` holds its label.
    pub const UNPAIRED: u8 = 0;
    /// A political idea (`political status` +0x50): identity = idea token.
    pub const IDEA: u8 = 1;
    /// The ruling party's first country leader: identity_id = character id.
    pub const COUNTRY_LEADER: u8 = 2;
    /// The ruling party's ideology group: identity = group token.
    pub const IDEOLOGY: u8 = 3;
    /// An appointed advisor: identity = advisor idea token, identity_id = character id.
    pub const ADVISOR: u8 = 4;
    /// A named static modifier added unscaled: identity = static name.
    pub const STATIC: u8 = 5;
    /// A stability- or war-support-scaled static copy: identity = static name.
    pub const STATIC_SCALED: u8 = 6;
    /// A dynamic modifier's constant part: identity = definition id,
    /// identity_id = scope country, secondary = scope state.
    pub const DYNAMIC_MODIFIER: u8 = 7;
    /// A dynamic modifier's evaluated values, with the same identity fields.
    pub const DYNAMIC_MODIFIER_VALUES: u8 = 8;
}
typed_codec!(CountryStabilityLineRow, 96, |row| (row.country as u128) << 64 | (row.line as u128) << 32 | row.key as u128, |key| Self { country: (key >> 64) as u32, line: (key >> 32) as u32, key: key as u32, ..Default::default() },
 { 0 => (kind): U8, 1 => (identity): Bytes, 2 => (identity_id): U32, 3 => (secondary): I64, 4 => (value): I64, 5 => (text): Bytes },
 |row| if row.kind > Self::DYNAMIC_MODIFIER_VALUES || (row.kind != Self::UNPAIRED && !row.text.is_empty()) { Err(invalid("invalid country stability line")) } else { Ok(()) });

/// WorldTension: singleton, 3 payload mask bits.
/// `total_raw` is the resident total, stored as
/// `clamp(sum of source contributions, 0, 10_000_000) / 100`. The pre-clamp
/// sum is the sum of ThreatSources.current_raw; nothing is derived here.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorldTensionRow {
    pub total_raw: i64,
    /// Native source count, independent of how many rows fitted the buffer.
    pub source_count: i32,
    /// Stored aggregate count, set to the country census.
    pub country_aggregate_count: i32,
}
typed_codec!(WorldTensionRow, 0, |_row| 0, |_key| Self { total_raw: Default::default(), source_count: Default::default(), country_aggregate_count: Default::default() },
 { 0 => (total_raw): I64, 1 => (source_count): I32, 2 => (country_aggregate_count): I32 }, |_row| Ok(()));

/// ThreatSources: key stored ordinal (32 bits), 7 payload mask bits.
/// The compacted vector has no engine ids, so ordinal is its only stored
/// identity and its order is itself state.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ThreatSourceRow {
    pub ordinal: u32,
    /// Raw country reference; resolve through CountryLinks.
    pub country: u32,
    /// Second raw country reference at +0x0c; its engine role is not established.
    pub second_country: u32,
    pub current_raw: i64,
    pub final_raw: i64,
    pub daily_raw: i64,
    /// Native engine date hour, not a calendar conversion.
    pub date: i32,
    /// Literal engine reason bytes; an empty reason is valid.
    pub reason: Vec<u8>,
}
typed_codec!(ThreatSourceRow, 32, |row| row.ordinal as u128, |key| Self { ordinal: key as u32, country: Default::default(), second_country: Default::default(), current_raw: Default::default(), final_raw: Default::default(), daily_raw: Default::default(), date: Default::default(), reason: Default::default() },
 { 0 => (country): U32, 1 => (second_country): U32, 2 => (current_raw): I64, 3 => (final_raw): I64, 4 => (daily_raw): I64, 5 => (date): I32, 6 => (reason): Bytes }, |_row| Ok(()));

/// ThreatCountryAggregates: key canonical country slot (32 bits), 1 payload mask bit.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ThreatCountryAggregateRow {
    /// Canonical country slot resolved from the raw reference.
    pub country_slot: u32,
    /// Signed sum stored for this slot, before the getter's clamp.
    pub total_raw: i64,
}
typed_codec!(ThreatCountryAggregateRow, 32, |row| row.country_slot as u128, |key| Self { country_slot: key as u32, total_raw: Default::default() },
 { 0 => (total_raw): I64 }, |_row| Ok(()));
/// IncomingDiplomaticActions: key country:ordinal (64 bits), 9 payload mask bits.
///
/// The ordinal is the stored queue index, also used as the serialized key,
/// so a null slot leaves a gap rather than shifting later rows.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IncomingDiplomaticActionRow {
    pub country: u32,
    pub ordinal: u32,
    /// Full incoming diplomatic action handle from the wrapper's +0x08.
    pub handle: u64,
    /// Raw wrapper byte at +0x10; no writer serializes it.
    pub status: u8,
    pub action_type: i32,
    pub actor: u32,
    pub original_actor: u32,
    pub recipient: u32,
    pub original_recipient: u32,
    /// Raw engine date word; the engine's own representation is preserved.
    pub hour: i32,
    pub last_command_hour: i32,
}
typed_codec!(IncomingDiplomaticActionRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, handle: Default::default(), status: Default::default(), action_type: Default::default(), actor: Default::default(), original_actor: Default::default(), recipient: Default::default(), original_recipient: Default::default(), hour: Default::default(), last_command_hour: Default::default() },
 { 0 => (handle): U64, 1 => (status): U8, 2 => (action_type): I32, 3 => (actor): U32, 4 => (original_actor): U32, 5 => (recipient): U32, 6 => (original_recipient): U32, 7 => (hour): I32, 8 => (last_command_hour): I32 }, |_row| Ok(()));

/// RelationOpinions: key owner_country:target_country (64 bits), 1 payload mask bit.
///
/// One row per directed country pair whose cached opinion dword is nonzero,
/// which is exactly when the native relation-status serializer emits
/// `cached_sum`. Country references stay raw engine references.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RelationOpinionRow {
    pub owner_country: u32,
    pub target_country: u32,
    /// Native signed dword, unscaled.
    pub opinion: i32,
}
typed_codec!(RelationOpinionRow, 64, |row| (row.owner_country as u128) << 32 | row.target_country as u128, |key| Self { owner_country: (key >> 32) as u32, target_country: key as u32, opinion: Default::default() },
 { 0 => (opinion): I32 }, |_row| Ok(()));

/// RelationTuples: key owner_country:target_country:type_id:ordinal (112 bits), 6 payload mask bits.
///
/// One row per element of a relation status's serializable vector, in native order.
/// `type_id` is the engine's own relation type word, which is also the key token the save
/// writes for that block. The ordinal completes the key because nothing in the engine
/// forbids two relations of one type in one pair.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RelationTupleRow {
    pub owner_country: u32,
    pub target_country: u32,
    pub type_id: u32,
    /// Serializable-vector index. The count is a signed i32, but cannot exceed
    /// u16: reset and the sixteen relation-slot setters are its only writers.
    /// Each setter removes its slot's previous relation before appending the
    /// new one, so the vector holds at most one relation per compiled slot,
    /// whatever the country count or content.
    pub ordinal: u16,
    /// Raw relation first-endpoint reference, which may be an alias of the owner.
    pub first_country: u32,
    /// Raw relation second-endpoint reference, which may be an alias of the target.
    pub second_country: u32,
    pub start_hour: i32,
    pub end_hour: i32,
    /// Raw engine byte, retained without boolean normalization.
    pub cancel: u8,
    /// Raw engine byte, retained without boolean normalization.
    pub retired: u8,
}
typed_codec!(RelationTupleRow, 112, |row| (row.owner_country as u128) << 80 | (row.target_country as u128) << 48 | (row.type_id as u128) << 16 | row.ordinal as u128, |key| Self { owner_country: (key >> 80) as u32, target_country: (key >> 48) as u32, type_id: (key >> 16) as u32, ordinal: key as u16, first_country: Default::default(), second_country: Default::default(), start_hour: Default::default(), end_hour: Default::default(), cancel: Default::default(), retired: Default::default() },
 { 0 => (first_country): U32, 1 => (second_country): U32, 2 => (start_hour): I32, 3 => (end_hour): I32, 4 => (cancel): U8, 5 => (retired): U8 }, |_row| Ok(()));

opaque_value!(Vec<ScriptVariableEntry>);
