use super::*;
/// Hourly stored scalar/vector operands of active raid tooltips, keyed by the
/// full raid reference and native occurrence ordinal. Definition rows use the
/// null reference-type arm (`raid_tooltip_operands::definition_key`); the
/// global subunit rows use raid key zero. The engine reader owns the family
/// codes; `key` is a native offset or index within the family.
/// Selected active-raid wing names use the WING_NAME family: byte length at
/// key u32::MAX and little-endian eight-byte chunks at zero-based keys (last
/// chunk zero padded), like existing name-group/custom operands. A negative
/// length is an engine_native::raid_tooltip_operands::NameReadError code, with no
/// chunks. Empty names retain zero length. Engine owns the family/error codes.
/// Lost naming provenance requires the shared stored-text language rule; see
/// the viewer's raid tooltip README for capture behavior and oracle limits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidTooltipOperandRow {
    pub raid: u64,
    pub ordinal: u32,
    pub kind: u32,
    pub key: u32,
    pub raw: i64,
}
typed_codec!(RaidTooltipOperandRow, 96, |row| (row.raid as u128) << 32 | row.ordinal as u128,
 |key| Self { raid: (key >> 32) as u64, ordinal: key as u32, ..Default::default() },
 {0 => (kind): U32, 1 => (key): U32, 2 => (raw): I64}, |_row| Ok(()));

/// Hourly target-tooltip program roster in each country's stored order.
/// Lookup selects the first province match and reads its active-project
/// token. Daily ProjectPrograms cannot supply this hover.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidFacilityRow {
    pub country: u32,
    pub ordinal: u32,
    pub id: u32,
    pub owner: u32,
    pub province: u32,
    pub active_project: Option<u32>,
    pub project_definition: Option<u32>,
}
typed_codec!(RaidFacilityRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (id): U32, 1 => (owner): U32, 2 => (province): U32, 3 => (active_project): OptionalU32, 4 => (project_definition): OptionalU32 }, |_row| Ok(()));

/// ProjectPrograms: key id (32 bits), 8 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProjectProgramRow {
    pub id: u32,
    pub owner: u32,
    pub province: u32,
    pub active_project: Option<u32>,
    pub scientist: u64,
    pub unread_rewards: i32,
    pub building: Vec<u8>,
    /// Primary assignment followed by all supports, preserving empty references
    /// and inactive timers. None is an independently refused acquisition.
    pub assignments: Recorded<Option<Vec<ProjectScientistAssignment>>>,
    /// Dated daily scalar speed sources and specialization contributions.
    pub facility: Recorded<Option<Vec<ProjectFacilityOperand>>>,
}
typed_codec!(ProjectProgramRow, 32, |row| row.id as u128, |key| Self { id: key as u32, ..Default::default() },
 { 0 => (owner): U32, 1 => (province): U32, 2 => (active_project): OptionalU32, 3 => (scientist): U64, 4 => (unread_rewards): I32, 5 => (building): Bytes, 6 => (assignments): Opaque, 7 => (facility): Opaque }, |_row| Ok(()));

/// Projects: key owner:local_id (64 bits), 15 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProjectRow {
    pub owner: u32,
    pub local_id: u32,
    pub definition_id: u32,
    pub program_reference: u64,
    pub started: u8,
    pub completed: u8,
    pub state: ProjectState,
    pub runtime: Recorded<ProjectRuntime>,
}
typed_codec!(ProjectRow, 64, |row| (row.owner as u128) << 32 | row.local_id as u128, |key| Self { owner: (key >> 32) as u32, local_id: key as u32, ..Default::default() },
 { 0 => (definition_id): U32, 1 => (program_reference): U64, 2 => (started): U8, 3 => (completed): U8, 4 => (state.current_offset): U32, 5 => (state.state_enums[0]): U32, 6 => (state.state_enums[1]): U32, 7 => (state.state_enums[2]): U32, 8 => (state.state_enums[3]): U32, 9 => (state.not_completed): U8, 10 => (state.phase_progress): I64, 11 => (state.phase_target): I32, 12 => (state.prototype_progress): I32, 13 => (state.iterations): I32, 14 => (runtime): Opaque }, |_row| Ok(()));

/// ProjectHistory: key owner:project:ordinal (96 bits), 12 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
 { 0 => (prototype_reward_handle): U32, 1 => (option_handle): U32, 2 => (date): I32, 3 => (context.province): I32, 4 => (context.progress): I32, 5 => (context.character_reference): U64, 6 => (context.country_present): U8, 7 => (context.country_reference): OptionalU32, 8 => (context.scientist_present): U8, 9 => (context.scientist_reference): OptionalU32, 10 => (context.state_present): U8, 11 => (context.state_reference): OptionalU32 }, |_row| Ok(()));

/// ProjectBreakthroughs: key owner:specialization (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProjectBreakthroughRow {
    pub owner: u32,
    pub specialization: u32,
    pub points: i32,
}
typed_codec!(ProjectBreakthroughRow, 64, |row| (row.owner as u128) << 32 | row.specialization as u128, |key| Self { owner: (key >> 32) as u32, specialization: key as u32, points: Default::default() },
 { 0 => (points): I32 }, |_row| Ok(()));

/// Nukes: key country:nuke_type (64 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NukeRow {
    pub country: u32,
    pub nuke_type: u32,
    pub amount_raw: i64,
    pub ready: i32,
}
typed_codec!(NukeRow, 64, |row| (row.country as u128) << 32 | row.nuke_type as u128, |key| Self { country: (key >> 32) as u32, nuke_type: key as u32, amount_raw: Default::default(), ready: Default::default() },
 { 0 => (amount_raw): I64, 1 => (ready): I32 }, |_row| Ok(()));

impl NukeRow {
    /// `amount_raw` as the top bar shows it: whole bombs, sentinel and machine-width arithmetic
    /// included.
    pub fn shown(amount_raw: i64) -> i32 {
        if (amount_raw as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd { (amount_raw.wrapping_mul(100_000) / 10_000_000_000_000) as i32 } else { 0xa7c5 }
    }
}

/// RaidCountries: one raid-status record per canonical slot.
///
/// `eligible_types` is the stored `allowed` result, rebuilt only at load,
/// new game and database reload; offers read `RaidPlayerAllowedTypes`
/// instead. `used_types` is the saved `fire_only_once` history. Both hold
/// raid type lexer tokens in stored order; name them through `LexicalTokens`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidCountryRow {
    pub canonical_slot: u32,
    pub country: i32,
    pub eligible_types: Vec<u32>,
    pub used_types: Vec<u32>,
}
typed_codec!(RaidCountryRow, 32, |row| row.canonical_slot as u128, |key| Self { canonical_slot: key as u32, ..Default::default() },
 { 0 => (country): I32, 1 => (eligible_types): U32List, 2 => (used_types): U32List }, |_row| Ok(()));

/// RaidPlayerAllowedTypes: key country (32 bits). At most one row: the
/// player-status cache whose list the game's raid offers use.
///
/// `country` is the raw country reference it was built for, zero before its
/// first rebuild. `allowed` contains raid type lexer tokens in stored order,
/// resolved from u16 database indices. Rebuild evaluates each type's
/// `allowed` only while the dirty byte is set: construction on load and
/// reset sets it, as does an update when the cached country is no longer
/// the playing country (else observer country). Not saved: a save
/// comparison cannot check it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidPlayerAllowedTypeRow {
    pub country: u32,
    pub allowed: Vec<u32>,
}
typed_codec!(RaidPlayerAllowedTypeRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (allowed): U32List }, |_row| Ok(()));

/// Raids: key id:reference_type (64 bits).
///
/// `show_for` holds the raw country handles an ended raid's result is shown
/// to, saved as `show_for`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidRow {
    pub reference_type: u32,
    pub id: u32,
    pub parent_slot: u32,
    pub ordinal: u32,
    pub phase: i32,
    pub outcome: i32,
    pub source: RaidSource,
    pub show_for: Vec<u32>,
    /// Assigned army and air wing, full references.
    pub unit: u64,
    pub air_wing: u64,
    /// Fixed5 distance flown, read in every phase. Each phase-4 hour adds raid
    /// unit speed (RaidSpeedInputs holds its operands); the save writes it as
    /// `distance` when nonzero. `None` only where the reader did not record it.
    pub travelled: Option<i64>,
    /// Hours prepared, incremented in phase 2 and used as that phase's elapsed
    /// hours; saved as `prep_time` when nonzero. The active raid icon's hover
    /// tooltip consumes this value.
    pub preparation_hours: i32,
}
typed_codec!(RaidRow, 64, |row| (row.id as u128) << 32 | row.reference_type as u128, |key| Self { id: (key >> 32) as u32, reference_type: key as u32, ..Default::default() },
 { 0 => (parent_slot): U32, 1 => (ordinal): U32, 2 => (phase): I32, 3 => (outcome): I32, 4 => (source.kind): OptionalU32, 5 => (source.kind_present): U8, 6 => (source.province): U32, 7 => (source.ship_reference_type): U32, 8 => (source.ship_reference_id): U32, 9 => (source.country): I32, 10 => (source.building_type): U32, 11 => (source.building_location): I32, 12 => (source.building_country): I32, 13 => (show_for): U32List, 14 => (unit): U64, 15 => (air_wing): U64, 16 => (travelled): OptionalI64, 17 => (preparation_hours): I32 }, |_row| Ok(()));

/// RaidTargets: key id:reference_type (64 bits).
///
/// The twin of `Raids`, carrying the raid map icon's extra operands:
/// location scope, raid type definition's lexer token (name through
/// `LexicalTokens`), and the stored `detected` byte.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidTargetRow {
    pub reference_type: u32,
    pub id: u32,
    pub scope: RaidScope,
    /// The raid type definition's lexer token, zero when the pointer is null.
    pub raid_type: u32,
    pub detected: u8,
}
typed_codec!(RaidTargetRow, 64, |row| (row.id as u128) << 32 | row.reference_type as u128, |key| Self { id: (key >> 32) as u32, reference_type: key as u32, ..Default::default() },
 { 0 => (scope.province): U32, 1 => (scope.state): U32, 2 => (scope.leader_reference): U64, 3 => (scope.leader_province): U32, 4 => (scope.building_type): U32, 5 => (scope.building_location): U32, 6 => (scope.building_flags): U32, 7 => (scope.building_country): I32, 8 => (scope.leader_country): OptionalI32, 9 => (raid_type): U32, 10 => (detected): U8 }, |_row| Ok(()));

/// RaidTargetStatuses: key native ordinal (32 bits).
///
/// The raid system's saved target statuses (`raids.targets`), independent
/// of any viewer. An invalid `valid` byte offers nothing. `detected` is
/// every country's persistent detection: byte `slot / 8`, bit `slot % 8`
/// of canonical country `slot`; a slot past the end is undetected.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidTargetStatusRow {
    pub ordinal: u32,
    pub valid: u8,
    pub scope: RaidScope,
    pub detected: Vec<u8>,
}
typed_codec!(RaidTargetStatusRow, 32, |row| row.ordinal as u128,
 |key| Self { ordinal: key as u32, ..Default::default() },
 { 0 => (valid): U8, 1 => (scope.province): U32, 2 => (scope.state): U32, 3 => (scope.leader_reference): U64, 4 => (scope.leader_province): U32, 5 => (scope.building_type): U32, 6 => (scope.building_location): U32, 7 => (scope.building_flags): U32, 8 => (scope.building_country): I32, 9 => (scope.leader_country): OptionalI32, 10 => (detected): Bytes }, |_row| Ok(()));

/// RaidCooldowns: key owner slot:ordinal (64 bits).
///
/// One stored target cooldown: disables an offer of this raid type (lexer
/// token) against this scope. Remaining days feed `tooltip_raid_on_cooldown`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidCooldownRow {
    pub owner_slot: u32,
    pub ordinal: u32,
    pub raid_type: u32,
    pub scope: RaidScope,
    /// Stored signed cooldown days.
    pub days: i32,
}
typed_codec!(RaidCooldownRow, 64, |row| (row.owner_slot as u128) << 32 | row.ordinal as u128,
 |key| Self { owner_slot: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (raid_type): U32, 1 => (scope.province): U32, 2 => (scope.state): U32, 3 => (scope.leader_reference): U64, 4 => (scope.leader_province): U32, 5 => (scope.building_type): U32, 6 => (scope.building_location): U32, 7 => (scope.building_flags): U32, 8 => (scope.building_country): I32, 9 => (scope.leader_country): OptionalI32, 10 => (days): I32 }, |_row| Ok(()));

/// The dispatcher is part of the key; failed searches stay in their own memo.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidPathDistanceRow {
    pub dispatcher: RaidPathDispatcher,
    pub source: u32,
    pub target: u32,
    pub distance: i64,
    pub ok: u8,
}
typed_codec!(RaidPathDistanceRow, 65, |row| (row.dispatcher as u128) << 64 | (row.source as u128) << 32 | row.target as u128,
 |key| Self { dispatcher: if key >> 64 == 0 { RaidPathDispatcher::Land } else { RaidPathDispatcher::Naval }, source: (key >> 32) as u32, target: key as u32, ..Default::default() },
 { 0 => (distance): I64, 1 => (ok): U8 }, |row| if row.ok <= 1 { Ok(()) } else { Err(invalid("raid path memo ok byte invalid")) });
/// RaidSpeedInputs: key id:reference_type (64 bits), as Raids. Hourly.
///
/// Stored raid unit speed operands, added to `RaidRow::travelled` every
/// phase-4 hour, for every raid in phase 4 and every ended, not cancelled
/// raid whose `end_hour` is at most an hour before the frame (its arrow's
/// unit may still be flying; the viewer applies the render rule). The raid
/// type's essential equipment chooses the branch: transport planes use the
/// largest `transport_speeds` (stat 0x3e of each raid pool entry with nonzero
/// amount and `air_transport` category, in pool order; the owner's stockpile
/// when `transport_from_country`; 100000 when `transport_fallback`), convoys
/// use NNavy defines, otherwise the wing (`wing_max_speed` times 1 + ace
/// terms), else the army (`army_max_speed`). Scale by
/// `NMilitary.LAND_SPEED_MODIFIER` (not for convoys), divide by 100000,
/// then by `NRaids.RAID_UNIT_SPEED_MULTIPLIER` and the raid type's `speed_multiplier`.
///
/// Ace terms apply only to a living ace: `ace_speed` (key 2 on the ace)
/// times wing-size scale (`ACE_WING_SIZE` over `ace_wing_planes`, capped at
/// `ACE_WING_SIZE_MAX_BONUS`; 100000 when the wing does not resolve), times
/// 1 + the ace owner's `ace_effectiveness_factor` (key 0x25a), plus the
/// wing country's `air_ace_bonuses_factor` (key 0x72) only when
/// `ace_type_speed` (key 2 on its type) is nonzero. Country terms come from
/// [`CountryCommandPowerRow`], joined at the same frame by canonical slots
/// `ace_owner` and `wing_country` (positive wing override, else its pool country).
///
/// `transport_speeds` is read here although VariantStats records stat 0x3e:
/// that table is daily, the game reads the stat every phase-4 hour, and its
/// writers are not proven daily-only.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidSpeedInputRow {
    pub reference_type: u32,
    pub id: u32,
    pub phase: i32,
    pub outcome: i32,
    /// Stored end hour, present only when the raid's end-date flag is set.
    pub end_hour: Option<i32>,
    pub wing_max_speed: Option<i64>,
    pub army_max_speed: Option<i64>,
    pub transport_speeds: Vec<i64>,
    pub transport_from_country: u8,
    pub transport_fallback: u8,
    pub ace_present: u8,
    pub ace_alive: u8,
    pub ace_speed: i64,
    pub ace_type_speed: i64,
    pub ace_wing_planes: Option<i32>,
    pub ace_owner: Option<u32>,
    pub wing_country: Option<u32>,
}
typed_codec!(RaidSpeedInputRow, 64, |row| (row.id as u128) << 32 | row.reference_type as u128, |key| Self { id: (key >> 32) as u32, reference_type: key as u32, ..Default::default() },
 { 0 => (phase): I32, 1 => (outcome): I32, 2 => (end_hour): OptionalI32, 3 => (wing_max_speed): OptionalI64, 4 => (army_max_speed): OptionalI64, 5 => (transport_speeds): Opaque, 6 => (transport_from_country): U8, 7 => (transport_fallback): U8, 8 => (ace_present): U8, 9 => (ace_alive): U8, 10 => (ace_speed): I64, 11 => (ace_type_speed): I64, 12 => (ace_wing_planes): OptionalI32, 13 => (ace_owner): OptionalU32, 14 => (wing_country): OptionalU32 }, |_row| Ok(()));

/// RaidEquipment: key id:reference_type (64 bits), as Raids. Hourly.
///
/// A raid's equipment pool and nuke count, checked against its type's
/// requirements to colour a prepared arrow READY or PREPARING. Recorded
/// before launch (phases 1 to 3) and while flying (as RaidSpeedInputs).
/// `variants` and `amounts` retain pool entries in order: full variant
/// reference (zero for null) and raw amount. `archetype_tokens` and
/// `archetype_amounts` retain per-archetype totals in stored order
/// (0x18-byte entries): archetype token and raw total. The requirement check
/// reads these totals rather than entries; quantity updates clamp a total
/// and its entry separately to `±0x53E2D620FB40` (`L`), so both remain as
/// stored. The save does
/// not write the totals; the oracle compares the census (both ways, from the
/// saved phase, outcome and `end_date`), the entries, the nukes and each
/// total exactly against its archetype's saved entries summed by
/// `VariantDefinitions.archetype_token`, stating a limitation only where the
/// total, a contributing entry or their sum reaches `L`. The one case it
/// cannot see is a clamp that later unwound: adding `3/4 L` to two variants
/// of one archetype and removing one leaves a total of `1/4 L` below the
/// entries' `3/4 L` with nothing at the bound. That needs the pool to have
/// held at least `L` (about 9.2e8 units) of one archetype, and would surface
/// as a reported difference.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidEquipmentRow {
    pub reference_type: u32,
    pub id: u32,
    pub phase: i32,
    pub end_hour: Option<i32>,
    pub variants: Vec<u64>,
    pub amounts: Vec<i64>,
    pub nukes: i32,
    pub archetype_tokens: Vec<u32>,
    pub archetype_amounts: Vec<i64>,
}
typed_codec!(RaidEquipmentRow, 64, |row| (row.id as u128) << 32 | row.reference_type as u128, |key| Self { id: (key >> 32) as u32, reference_type: key as u32, ..Default::default() },
 { 0 => (phase): I32, 1 => (end_hour): OptionalI32, 2 => (variants): U64List, 3 => (amounts): Opaque, 4 => (nukes): I32, 5 => (archetype_tokens): U32List, 6 => (archetype_amounts): Opaque }, |_row| Ok(()));

/// RaidCountryInputs: key canonical slot. What raid offers need that `Countries` lacks: the
/// effective getter values of the range modifiers, the autonomous flag (the game state's
/// per-country array, absent past the controller arrays) and the convoy reservations as flattened
/// `(priority, quantity)` pairs. Command power and the ace country modifiers are
/// [`CountryCommandPowerRow`]'s.
///
/// Hourly, every country, in the same country pass as `CountryCommandPower`: the game's
/// offer cache re-checks every offer at least hourly.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidCountryInputRow {
    pub canonical_slot: u32,
    /// Uncapped signed whole-plane stockpile total for volunteer senders.
    pub volunteer_stockpile_planes: Recorded<Option<i32>>,
    /// `air_range_factor`. Each modifier here is the getter value of the key
    /// resolved by its script name, `None` when the running game registers no
    /// key under that name.
    pub air_range_factor: Option<i64>,
    /// `naval_commando_raid_distance`.
    pub naval_commando_range_factor: Option<i64>,
    pub autonomous: Option<u32>,
    pub convoy_reservations: Vec<u32>,
}
typed_codec!(RaidCountryInputRow, 32, |row| row.canonical_slot as u128, |key| Self { canonical_slot: key as u32, ..Default::default() },
 { 0 => (air_range_factor): OptionalI64, 1 => (naval_commando_range_factor): OptionalI64, 2 => (autonomous): OptionalU32, 3 => (convoy_reservations): U32List, 4 => (volunteer_stockpile_planes): OptionalI32 }, |_row| Ok(()));

/// RaidAirSites: key canonical slot:ordinal. A country's air then missile
/// sites, tried as raid sources, with the country's container capacity at
/// each base, absent without a container.
///
/// Sliced, with `StockpileArchetypes`: each country's daily slice and every
/// country a hooked site list or capacity writer marked since the previous
/// hour, so the lists and capacities equal the game's at every hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidAirSiteRow {
    pub canonical_slot: u32,
    pub ordinal: u32,
    pub list: u8,
    /// Full `air base` handle; join it to AirBases.
    pub base: u64,
    pub container_capacity_raw: Option<i64>,
}
typed_codec!(RaidAirSiteRow, 64, |row| (row.canonical_slot as u128) << 32 | row.ordinal as u128, |key| Self { canonical_slot: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (list): U8, 1 => (base): U64, 2 => (container_capacity_raw): OptionalI64 }, |_row| Ok(()));

/// AceRangeInputs: key canonical slot:full ace reference (96 bits), owning
/// country first so its rows are contiguous. Ace wing-range operands:
/// assigned wing, alive byte and effective key 5 values from its own and
/// its type's modifier accumulators.
///
/// Hourly, every country, in the same country pass as `RaidCountryInputs`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AceRangeInputRow {
    pub canonical_slot: u32,
    pub ace: u64,
    pub wing: u64,
    pub alive: u8,
    pub range: i64,
    pub type_range: i64,
    /// Raw night/weather entries for living aces; absent outside that demand.
    pub normalization: Recorded<Option<[Option<i64>; 2]>>,
    /// Living ace's raw fuel entry from that same complete accumulator copy.
    pub fuel_modifier: Recorded<Option<[Option<i64>; 1]>>,
    /// Raw living-ace operands, independently delta encoded. Known plus None
    /// proves an absent accumulator key rather than a failed read.
    pub combat_modifiers_known: Recorded<bool>,
    pub speed_modifier_raw: Recorded<Option<i64>>,
    pub attack_modifier_raw: Recorded<Option<i64>>,
    pub defence_modifier_raw: Recorded<Option<i64>>,
    pub type_speed_modifier_raw: Recorded<Option<i64>>,
    /// Bit zero is known; bits one and two are positive type attack/defence.
    /// Type speed uses its raw entry and recorded modifier metadata in playback.
    pub type_combat_flags: Recorded<u8>,
}
typed_codec!(AceRangeInputRow, 96, |row| (row.canonical_slot as u128) << 64 | row.ace as u128, |key| Self { canonical_slot: (key >> 64) as u32, ace: key as u64, ..Default::default() },
 { 0 => (wing): U64, 1 => (alive): U8, 2 => (range): I64, 3 => (type_range): I64, 4 => (normalization): Opaque, 5 => (fuel_modifier): Opaque, 6 => (combat_modifiers_known): Bool, 7 => (speed_modifier_raw): OptionalI64, 8 => (attack_modifier_raw): OptionalI64, 9 => (defence_modifier_raw): OptionalI64, 10 => (type_speed_modifier_raw): OptionalI64, 11 => (type_combat_flags): U8 }, |_row| Ok(()));

impl AceRangeInputRow {
    fn known_type_combat_flags(&self) -> Result<u8, &'static str> {
        let Recorded::Value(flags) = self.type_combat_flags else {
            return Err("ace type combat flags not recorded");
        };
        (flags & 1 != 0)
            .then_some(flags)
            .ok_or("ace type combat modifiers are unavailable")
    }
    pub fn type_attack_positive(&self) -> Result<bool, &'static str> {
        Ok(self.known_type_combat_flags()? & 2 != 0)
    }
    pub fn type_defence_positive(&self) -> Result<bool, &'static str> {
        Ok(self.known_type_combat_flags()? & 4 != 0)
    }
    pub fn type_speed_modifier(&self) -> Result<Option<i64>, &'static str> {
        self.known_type_combat_flags()?;
        self.type_speed_modifier_raw
            .require("ace type speed modifier")
            .copied()
            .map_err(|_| "ace type speed modifier not recorded")
    }
}

/// Stored per-country regional air-superiority scores (fixed5), used by
/// raid target intel, success and the Air map mode. Every pair with a nonzero
/// weight, active-mission counter, nonzero spotting scale or unknown spotting
/// scale, and the final region
/// of every owner is present. In a complete table, an absent pair for a
/// present owner has zero weights/counters and observed zero spotting scale;
/// an absent owner has no strategic-air object.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidAirScoreRow {
    pub country: u32,
    pub region: u32,
    pub friendly: i64,
    pub enemy: i64,
    /// The entry's land movement impact, rewritten with the pair: the movement speed
    /// adds it times the enemy's share of the pair and the air speed define. Older
    /// formats use NotRecorded.
    pub impact: Recorded<i64>,
    /// The regional active-mission counters, rewritten with the weights. Air map colours
    /// consult these when both weights are zero.
    pub friendly_missions: u16,
    pub enemy_missions: u16,
    /// Raw regional fixed5 spotting normalization, acquired independently of
    /// the score prefix and carried at the same native phase cadence.
    pub air_spotting_scale: Option<i64>,
    /// Shared with the capture's held table: an unchanged menu is never copied.
    pub menu: Recorded<Option<std::sync::Arc<AirPowerMenu>>>,
}
typed_codec!(RaidAirScoreRow, 64, |r| (r.country as u128) << 32 | r.region as u128,
 |key| Self { country: (key >> 32) as u32, region: key as u32, ..Self::default() },
 { 0 => (friendly): I64, 1 => (enemy): I64, 2 => (impact): I64, 3 => (friendly_missions): U16, 4 => (enemy_missions): U16, 5 => (air_spotting_scale): OptionalI64, 6 => (menu): Opaque }, |_row| Ok(()));

/// Hourly raid target-intel operands for live counterintelligence rating
/// and decryption threshold. Modifier totals preserve getter feature and
/// clamp semantics. Operatives retain roster occurrences; the rating
/// deduplicates their objects and stacks contributions.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidIntelInputRow {
    pub country: u32,
    pub strategic_present: u8,
    pub country_defense: i64,
    pub defense_tree: Recorded<Option<std::sync::Arc<Vec<ModifierBreakdownNode>>>>,
    pub crypto_strength: Option<i64>,
    pub operatives: Vec<RaidCounterOperative>,
}
typed_codec!(RaidIntelInputRow, 32, |r| r.country as u128,
 |key| Self { country: key as u32, ..Self::default() },
 { 0 => (strategic_present): U8, 1 => (country_defense): I64, 2 => (crypto_strength): OptionalI64, 3 => (operatives): Opaque, 4 => (defense_tree): Opaque },
 |_r| Ok(()));