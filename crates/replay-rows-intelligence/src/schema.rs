use super::*;
/// Characters: key id (32 bits), 12 payload mask bits.
///
/// The template columns identify the character's retained template, changed
/// only by construction and save loading. `template_present` is whether it
/// is non-null; without it the other template columns are zero and empty.
/// `template_tag` is the template's lexer token, saved as `template`.
/// `template_file` and `template_line` are the reader's exact file name and
/// 1-based line when template reading began: an `instance` uses the line of
/// the `}` closing its leading `allowed`. An unread template keeps an empty
/// file and line -1. `template_generated` marks templates made by `generate_character`.
///
/// The viewer resolves the content template by file, line and tag; the save
/// writes only the tag.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CharacterRow {
    pub id: u32,
    pub country: u32,
    pub name_token_id: i32,
    /// Stored gender enum; only 2 selects feminine pronouns.
    pub gender: u32,
    pub roster: u8,
    pub ordinal: u32,
    pub name: Vec<u8>,
    pub token: Vec<u8>,
    pub template_present: u8,
    pub template_tag: u32,
    pub template_file: Vec<u8>,
    pub template_line: i32,
    pub template_generated: u8,
}
typed_codec!(CharacterRow, 32, |row| row.id as u128, |key| Self { id: key as u32, ..Default::default() },
 { 0 => (country): U32, 1 => (name_token_id): I32, 2 => (roster): U8, 3 => (ordinal): U32, 4 => (name): Bytes, 5 => (token): Bytes, 6 => (template_present): U8, 7 => (template_tag): U32, 8 => (template_file): Bytes, 9 => (template_line): I32, 10 => (template_generated): U8, 11 => (gender): U32 }, |_row| Ok(()));

/// CountryLeaderRoles: key character:map ideology (64 bits).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryLeaderRoleRow {
    pub character: u32,
    pub country: u32,
    pub ideology: i32,
    /// The role's ideology token can change without rekeying the role map.
    /// Outer None is a counted capture refusal; Some(None) is a known null
    /// pointer, distinct from every token.
    pub current_ideology: Recorded<Option<Option<u32>>>,
}
typed_codec!(CountryLeaderRoleRow, 64, |row| (row.character as u128) << 32 | row.ideology as u32 as u128, |key| Self { character: (key >> 32) as u32, ideology: key as u32 as i32, ..Default::default() },
 { 0 => (country): U32, 1 => (current_ideology): Opaque }, |_row| Ok(()));

/// AdvisorRoles: key character:ordinal (64 bits).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AdvisorRoleRow {
    pub character: u32,
    pub country: u32,
    pub ordinal: u32,
    pub key: Vec<u8>,
    pub idea: Vec<u8>,
    pub slot: Vec<u8>,
    /// Stored advisor cache; None is a counted capture refusal.
    pub modifier_cache: Recorded<Option<AdvisorModifierCache>>,
    /// The stored string set: exact occupied-slot bytes in slot order.
    /// None is a counted capture refusal.
    pub tooltip_keys: Recorded<Option<Vec<Vec<u8>>>>,
}
typed_codec!(AdvisorRoleRow, 64, |row| (row.character as u128) << 32 | row.ordinal as u128, |key| Self { character: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (country): U32, 1 => (key): Bytes, 2 => (idea): Bytes, 3 => (slot): Bytes, 4 => (modifier_cache): Opaque, 5 => (tooltip_keys): Opaque }, |_row| Ok(()));

/// UnitLeaders: key id:reference_type (64 bits).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct UnitLeaderRow {
    pub reference_type: u32,
    pub id: u32,
    pub character_type: u32,
    pub character_id: u32,
    pub category: i32,
    /// Signed residual experience in fixed5, independent of skill level.
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
    /// The raw country reference (resolve through CountryLinks); capture
    /// leaves it unchanged.
    pub owner: u32,
    /// The army leader's captured byte; zero for the other kinds. Saved as `captured=yes`.
    pub captured: u8,
    /// Visibility operands: the leader's own visibility choice is non-null
    /// (evaluated first), and the byte that makes the character template's
    /// visibility choice apply when there is no leader choice. Neither is saved.
    pub visibility_choice: u8,
    pub template_visibility: u8,
    /// Stored original/exile country.
    pub exile_country: Recorded<i32>,
    /// Stored operative codename operands; None is outside scope or a counted refusal.
    pub codename: Recorded<Option<NameOperands>>,
    /// Formation reference, read for an army leader whose unsigned category is
    /// below two. Some(0) is observed null; None is outside that branch or a
    /// counted independent refusal. Older formats use NotRecorded.
    pub formation_reference: Recorded<Option<u64>>,
}
typed_codec!(UnitLeaderRow, 64, |row| (row.id as u128) << 32 | row.reference_type as u128, |key| Self { id: (key >> 32) as u32, reference_type: key as u32, ..Default::default() },
 { 0 => (character_type): U32, 1 => (character_id): U32, 2 => (category): I32, 3 => (experience): I64, 4 => (kind): U8, 5 => (skill_status): U8, 6 => (skill_valid): U8, 7 => (skill_level): I32, 8 => (skill_required_experience): I32, 9 => (skill_category): I32, 10 => (components[0]): I32, 11 => (components[1]): I32, 12 => (components[2]): I32, 13 => (components[3]): I32, 14 => (inherited_text_a): Bytes, 15 => (inherited_text_b): Bytes, 16 => (owner): U32, 17 => (captured): U8, 18 => (visibility_choice): U8, 19 => (template_visibility): U8, 20 => (exile_country): I32, 21 => (codename): Opaque, 22 => (formation_reference): OptionalU64 }, |row| {
    if matches!(row.codename, Recorded::Value(Some(_))) && row.kind != 3 {
        return Err(invalid("codename requires an operative leader"));
    }
    Ok(())
});

/// UnitLeaderTraits: key leader_reference:ordinal (96 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct UnitLeaderTraitRow {
    pub leader_reference: u64,
    pub ordinal: u32,
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub token: Vec<u8>,
}
typed_codec!(UnitLeaderTraitRow, 96, |row| (row.leader_reference as u128) << 32 | row.ordinal as u128, |key| Self { leader_reference: (key >> 32) as u64, ordinal: key as u32, token_id: Default::default(), definition_ordinal: Default::default(), token: Default::default() },
 { 0 => (token_id): I32, 1 => (definition_ordinal): I32, 2 => (token): Bytes }, |_row| Ok(()));

/// Complete ordered native commander training progress, independent of assigned traits.
/// Tokens resolve through LexicalTokens and the recorded content stack.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct UnitLeaderTraitExperienceRow {
    pub leader_reference: u64,
    /// Native progress (0) or per-trait XP-factor overrides (1).
    pub list: u8,
    pub ordinal: u32,
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub experience: i64,
}
typed_codec!(UnitLeaderTraitExperienceRow, 97, |row| (row.leader_reference as u128) << 33 | (row.list as u128) << 32 | row.ordinal as u128, |key| Self { leader_reference: (key >> 33) as u64, list: ((key >> 32) & 1) as u8, ordinal: key as u32, ..Default::default() },
 { 0 => (token_id): I32, 1 => (definition_ordinal): I32, 2 => (experience): I64 }, |_row| Ok(()));

/// ActiveAbilities: key country:ordinal (64 bits), 12 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
 { 0 => (scope_country): U32, 1 => (token): Bytes, 2 => (caster): OptionalU64, 3 => (strategic_region): OptionalU32, 4 => (division_count): I32, 5 => (activation_hour): I32, 6 => (phase): U8, 7 => (allocate_cost): U8, 8 => (allocated_command_power): I64, 9 => (duration_hours): I32, 10 => (cooldown_hours): I32, 11 => (target_kind): I32 }, |_row| Ok(()));

/// IntelligenceAgencies: key country (32 bits), 7 payload mask bits.
///
/// Name bytes are in the recording client's language, recorded in the content
/// identity. Names refresh with the hour; they are not reconstructed from a
/// localisation key.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IntelligenceAgencyRow {
    pub country: u32,
    pub upgrade_progress_raw: i64,
    pub defense_raw: i64,
    pub active_count: i32,
    pub admission_ceiling: i32,
    pub unknown_f0: u32,
    /// Exact stored agency name bytes, used for display only for a created agency.
    pub name: Vec<u8>,
    /// The raw created byte; zero makes the display localise `AGENCY_NAME` instead.
    pub created: u8,
}
typed_codec!(IntelligenceAgencyRow, 32, |row| row.country as u128, |key| Self { country: key as u32, upgrade_progress_raw: Default::default(), defense_raw: Default::default(), active_count: Default::default(), admission_ceiling: Default::default(), unknown_f0: Default::default(), name: Default::default(), created: Default::default() },
 { 0 => (upgrade_progress_raw): I64, 1 => (defense_raw): I64, 2 => (active_count): I32, 3 => (admission_ceiling): I32, 4 => (unknown_f0): U32, 5 => (name): Bytes, 6 => (created): U8 }, |_row| Ok(()));

/// AgencyUpgradeLevels: key country:ordinal (64 bits), 6 payload mask bits.
///
/// Every acquired agency upgrade entry, including duplicates and zero or
/// negative levels, in stored array order. The save writes the upgrade token
/// and signed level from each 16-byte entry. The ordinal is this read's
/// position, not a persistent definition identity.
///
/// Upgrade and branch tokens name the authenticated branch-owned definition,
/// never an assumed global token winner. Database and branch membership
/// authenticate their pointers. Raw pointers and levels survive identity
/// refusals; absent tokens do not certify identity.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AgencyUpgradeLevelRow {
    pub country: u32,
    pub ordinal: u32,
    pub definition_pointer: u64,
    pub branch_pointer: u64,
    pub upgrade_token: Option<u32>,
    pub branch_token: Option<u32>,
    pub level: i32,
    /// Zero authenticates both identities; other values name a counted refusal
    /// in the engine reader. A refused row must not become an acquired count.
    pub identity_status: u8,
}
typed_codec!(AgencyUpgradeLevelRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, definition_pointer: Default::default(), branch_pointer: Default::default(), upgrade_token: Default::default(), branch_token: Default::default(), level: Default::default(), identity_status: Default::default() },
 { 0 => (definition_pointer): U64, 1 => (branch_pointer): U64, 2 => (upgrade_token): OptionalU32, 3 => (branch_token): OptionalU32, 4 => (level): I32, 5 => (identity_status): U8 }, |_row| Ok(()));

/// OperativeMemberships: key id:roster_country:roster_kind (104 bits), 15 payload mask bits.
///
/// The operative has no universal stored detection/exposure byte. Mission
/// detection dispatches an `on_action`; the save serializes assignment,
/// capturer and mission instead. The description string's length is not detection.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OperativeMembershipRow {
    pub id: u64,
    pub owner_country: u32,
    pub roster_country: u32,
    pub roster_kind: u8,
    /// Historical description-length wire slot; no native detection byte exists.
    pub legacy_description_length: u8,
    pub assignment: u32,
    pub experience_raw: i64,
    pub skill: Option<i32>,
    pub capturer_country: u32,
    pub captured_hour: i32,
    pub action_variant: Option<u32>,
    /// Native agency roster position.
    pub ordinal: Recorded<u32>,
    /// Active assignment target copied with this membership.
    pub target_country: Recorded<Option<u32>>,
    /// Own packed operation reference, including reservations.
    pub operation: Recorded<Option<u64>>,
    /// Saved literal/localisation key, never the localised UI cache.
    pub name: Recorded<Vec<u8>>,
    /// Assignment status operands; older layouts supply no value.
    pub cooldown_kind: Recorded<Option<u32>>,
    pub cooldown_start_hour: Recorded<Option<i32>>,
    pub cooldown_end_hour: Recorded<Option<i32>>,
}
typed_codec!(OperativeMembershipRow, 104, |row| (row.id as u128) << 40 | (row.roster_country as u128) << 8 | row.roster_kind as u128, |key| Self { id: (key >> 40) as u64, roster_country: (key >> 8) as u32, roster_kind: key as u8, owner_country: Default::default(), legacy_description_length: Default::default(), assignment: Default::default(), experience_raw: Default::default(), skill: Default::default(), capturer_country: Default::default(), captured_hour: Default::default(), action_variant: Default::default(), ordinal: Default::default(), target_country: Default::default(), operation: Default::default(), name: Default::default(), cooldown_kind: Default::default(), cooldown_start_hour: Default::default(), cooldown_end_hour: Default::default() },
 { 0 => (owner_country): U32, 1 => (legacy_description_length): U8, 2 => (assignment): U32, 3 => (experience_raw): I64, 4 => (skill): OptionalI32, 5 => (capturer_country): U32, 6 => (captured_hour): I32, 7 => (action_variant): OptionalU32, 8 => (ordinal): U32, 9 => (target_country): OptionalU32, 10 => (operation): OptionalU64, 11 => (name): Bytes, 12 => (cooldown_kind): OptionalU32, 13 => (cooldown_start_hour): OptionalI32, 14 => (cooldown_end_hour): OptionalI32 }, |_row| Ok(()));

/// OperativeMissions: key roster_country:id (96 bits), 18 payload mask bits.
///
/// Hourly: every operative in each agency's active roster, in roster order,
/// with what the Operatives map's icons read: the assignment and its
/// cooldown, the active mission and its targets, the mission an operative
/// relocating carries until it arrives, the leader's own portrait, and a
/// root-out-resistance mission's effectiveness operands. Commands,
/// recruitment, capture and operations change these between daily ticks;
/// OperativeMemberships keeps the daily census of every holding vector.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OperativeMissionRow {
    pub active_effect: Recorded<Option<std::sync::Arc<OperativeEffect>>>,
    pub pending_effect: Recorded<Option<std::sync::Arc<OperativeEffect>>>,
    pub map_selected: Recorded<Option<u8>>,
    pub map_warning: Recorded<Option<u8>>,
    pub roster_country: u32,
    pub id: u64,
    /// Native active-roster position.
    pub ordinal: u32,
    pub owner_country: u32,
    pub assignment: u32,
    /// Cooldown operands, read only for the cooldown assignment.
    pub cooldown_kind: Option<u32>,
    pub cooldown_start_hour: Option<i32>,
    pub cooldown_end_hour: Option<i32>,
    /// The active mission's nonzero variant (1..8), whatever the assignment.
    pub variant: Option<u32>,
    /// The active mission's raw target country reference, present with the
    /// variant; the save omits a nonpositive one.
    pub target_country: Option<u32>,
    /// The state-targeted missions' target state id; absent for a null state
    /// and for the country-targeted variants.
    pub target_state: Option<i32>,
    /// The boost-ideology mission's ideology, as the save writes it; empty
    /// for none.
    pub ideology: Vec<u8>,
    /// The pending mission's variant; absent when no mission is pending.
    pub pending_variant: Option<u32>,
    pub pending_country: Option<u32>,
    pub pending_state: Option<i32>,
    pub pending_ideology: Vec<u8>,
    /// The leader's own portrait sprite name, shown when it has no character.
    pub portrait: Vec<u8>,
    /// Root-out-resistance operands, present only for that variant (4): the
    /// effective `root_out_resistance_effectiveness_factor` (fixed5) of the
    /// owner country's and of the operative's own modifier accumulator, and
    /// 1 when the operative's nationalities contain the mission's context
    /// country (the target state's active resistance country when positive,
    /// else the target country), else 0. Runtime-only: the save writes none.
    pub root_out_country_factor: Option<i64>,
    pub root_out_operative_factor: Option<i64>,
    pub root_out_nationality: Option<u8>,
}
typed_codec!(OperativeMissionRow, 96, |row| (row.roster_country as u128) << 64 | row.id as u128, |key| Self { roster_country: (key >> 64) as u32, id: key as u64, ..Default::default() },
 { 0 => (ordinal): U32, 1 => (owner_country): U32, 2 => (assignment): U32, 3 => (cooldown_kind): OptionalU32, 4 => (cooldown_start_hour): OptionalI32, 5 => (cooldown_end_hour): OptionalI32, 6 => (variant): OptionalU32, 7 => (target_country): OptionalU32, 8 => (target_state): OptionalI32, 9 => (ideology): Bytes, 10 => (pending_variant): OptionalU32, 11 => (pending_country): OptionalU32, 12 => (pending_state): OptionalI32, 13 => (pending_ideology): Bytes, 14 => (portrait): Bytes, 15 => (root_out_country_factor): OptionalI64, 16 => (root_out_operative_factor): OptionalI64, 17 => (root_out_nationality): Opaque, 18 => (map_selected): Opaque, 19 => (map_warning): Opaque, 20 => (active_effect): Opaque, 21 => (pending_effect): Opaque }, |_row| Ok(()));

/// StrategicOperatives: key country (32 bits), 5 payload mask bits.
///
/// Hourly: each country's strategic operative, keyed by its raw owner, with
/// its index in the manager's vector (the country slot the save keys it by),
/// the mission and operation activity raised against the country (saved
/// `subversive_activity_level` and `danger_level`, fixed5), and the
/// control-trade influence and diplomatic pressure other countries hold over
/// it, keyed by the influencing country, in native table order. Missions,
/// operations and their daily decay change them, and the Operatives map
/// reads them every hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StrategicOperativeRow {
    pub mission_rosters: Recorded<Option<std::sync::Arc<[Vec<u64>; 5]>>>,
    pub country: u32,
    pub slot: u32,
    pub mission_activity: i64,
    pub operation_activity: i64,
    pub trade_influence: Vec<TradeInfluence>,
    pub diplomatic_pressure: Vec<DiplomaticPressure>,
}
typed_codec!(StrategicOperativeRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (slot): U32, 1 => (mission_activity): I64, 2 => (operation_activity): I64, 3 => (trade_influence): Opaque, 4 => (diplomatic_pressure): Opaque, 5 => (mission_rosters): Opaque }, |_row| Ok(()));

/// Logical saved propaganda entries. Real sequences retain their keys across
/// daily appends/removals. The final sequence is an owner header: it preserves
/// the independent loaded weekly cache until `derived_drift` becomes one, when
/// the weekly drift is exactly the wrapping Q15 sum of the retained entries.
/// A missing header is unobserved, not an observed empty history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PropagandaEffortRow {
    pub slot: u32,
    pub country: u32,
    pub sequence: u64,
    pub stability: i64,
    pub war_support: i64,
    pub derived_drift: u8,
}
typed_codec!(PropagandaEffortRow, 96, |row| (row.slot as u128) << 64 | row.sequence as u128,
|key| Self { slot: (key >> 64) as u32, sequence: key as u64, ..Default::default() },
{ 0 => (stability): I64, 1 => (war_support): I64, 2 => (derived_drift): U8, 3 => (country): U32 }, |row| {
   if row.derived_drift > 1 || (row.sequence != u64::MAX && row.derived_drift != 0)
       || (row.derived_drift == 1 && (row.stability != 0 || row.war_support != 0)) {
       return Err(replay_rows::codec::invalid("invalid propaganda effort header"));
   }
   Ok(())
});

impl PropagandaEffortRow {
    /// `rows` is the table's ordinary key-ordered complete snapshot.
    pub fn weekly_drift(rows: &[Self], slot: u32) -> Option<(i64, i64)> {
        let begin = rows.partition_point(|row| row.slot < slot);
        let end = rows.partition_point(|row| row.slot <= slot);
        let group = rows.get(begin..end)?;
        let header = group.last().filter(|row| row.sequence == u64::MAX)?;
        if header.derived_drift == 0 {
            return Some((header.stability, header.war_support));
        }
        Some(group[..group.len() - 1].iter().fold(
            (0_i64, 0_i64),
            |(stability, war_support), row| {
                (
                    stability.wrapping_add(row.stability),
                    war_support.wrapping_add(row.war_support),
                )
            },
        ))
    }
}

/// Cryptology: key owner_country:target_country (64 bits), 4 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CryptologyRow {
    pub owner_country: u32,
    pub target_country: u32,
    pub amount_raw: i64,
    pub bonus_days: i32,
    pub bonus_active: u8,
    pub decrypting: u8,
    /// The cipher list's order; formats before 135 did not record it.
    pub ordinal: Recorded<u32>,
}
typed_codec!(CryptologyRow, 64, |row| (row.owner_country as u128) << 32 | row.target_country as u128, |key| Self { owner_country: (key >> 32) as u32, target_country: key as u32, ..Self::default() },
 { 0 => (amount_raw): I64, 1 => (bonus_days): I32, 2 => (bonus_active): U8, 3 => (decrypting): U8, 4 => (ordinal): U32 }, |_row| Ok(()));

/// IntelNetworks: key owner_country:target_country:state_id (96 bits), 12 payload mask bits.
///
/// Fixed-point values are scaled by 100000. The Operatives map mode's network
/// layer reads the local strength and quiet byte; its state tooltip reads the
/// gains and the selected subnetwork's values.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IntelNetworkRow {
    pub owner_country: u32,
    pub target_country: u32,
    pub state_id: i32,
    /// Absent when the native subnetwork selector is negative.
    pub strength_raw: Option<i64>,
    /// Exact signed selector, including all native unassigned values.
    pub subnetwork_selector: i32,
    /// Exact graph-row state strength, including zero and negative values.
    pub local_strength_raw: i64,
    /// The state's daily gain and its parts from the network's operatives and
    /// from adjacent states (saved `gain` and `gain_breakdown`).
    pub gain_raw: i64,
    pub gain_from_operatives_raw: i64,
    pub gain_from_adjacency_raw: i64,
    /// The network's quiet byte for the selector (the local
    /// `sub_network_is_quiet` list); zero for a selector outside the list.
    pub local_quiet: u8,
    /// The selected subnetwork's gain factor, strength target and the
    /// target's parts from operatives and from counterintelligence; absent,
    /// like the strength, for a negative selector.
    pub gain_factor_raw: Option<i64>,
    pub strength_target_raw: Option<i64>,
    pub target_from_operatives_raw: Option<i64>,
    pub target_from_counterintelligence_raw: Option<i64>,
    /// The selected subnetwork's own quiet byte; zero for a negative selector,
    /// which selects no subnetwork.
    pub subnetwork_quiet: u8,
}
typed_codec!(IntelNetworkRow, 96, |row| (row.owner_country as u128) << 64 | (row.target_country as u128) << 32 | row.state_id as u32 as u128, |key| Self { owner_country: (key >> 64) as u32, target_country: (key >> 32) as u32, state_id: key as u32 as i32, ..Default::default() },
 { 0 => (strength_raw): OptionalI64, 1 => (subnetwork_selector): I32, 2 => (local_strength_raw): I64, 3 => (gain_raw): I64, 4 => (gain_from_operatives_raw): I64, 5 => (gain_from_adjacency_raw): I64, 6 => (local_quiet): U8, 7 => (gain_factor_raw): OptionalI64, 8 => (strength_target_raw): OptionalI64, 9 => (target_from_operatives_raw): OptionalI64, 10 => (target_from_counterintelligence_raw): OptionalI64, 11 => (subnetwork_quiet): U8 }, |_row| Ok(()));

/// Independent retained national coverage and strength caches, one row per
/// actual network, including an empty network. These are not recomputed from
/// state rows: the native caches can be stale. Acquired with IntelNetworks at
/// its daily/reset/growth boundary, carried otherwise; ordinary row deltas
/// write only changed fields. Country references retain the census identities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IntelNetworkSummaryRow {
    pub owner_country: u32,
    pub target_country: u32,
    pub ordinal: u32,
    pub covered_core_states: i32,
    pub covered_controlled_states: i32,
    pub covered_worth: i32,
    pub controlled_strength_sum: i64,
    pub core_strength_sum: i64,
    pub max_state_strength: i64,
}
typed_codec!(IntelNetworkSummaryRow, 64, |row| (row.owner_country as u128) << 32 | row.target_country as u128,
 |key| Self { owner_country: (key >> 32) as u32, target_country: key as u32, ..Default::default() },
 { 0 => (covered_core_states): I32, 1 => (covered_controlled_states): I32, 2 => (covered_worth): I32,
   3 => (controlled_strength_sum): I64, 4 => (core_strength_sum): I64, 5 => (max_state_strength): I64, 6 => (ordinal): U32 }, |_row| Ok(()));

/// Operations: key id (64 bits), 21 payload mask bits.
///
/// Hourly: operation creation and cancellation are commands outside the
/// daily tick, and the map icons read the preparation every hour. Visible
/// outcomes are snapshots. OperationCompletions supplies the events.
/// The daily manager can destroy an AI owner's instance in the same call
/// that completes it. Other instances can be deleted by command or
/// invalidation. There is no fixed minimum completed lifetime, and even
/// hourly snapshots cannot guarantee every outcome.
///
/// `preparation_current` and `preparation_required` are saved as
/// `civilian_factories` and `total`, each omitted when zero. `phases` are the
/// selected phase definitions' lexer tokens in order with repeats (saved
/// `phases`). `requirements` are the requirement records in order with
/// repeats (saved `resources`). `equipment` is the inline equipment pool's
/// variant entries (saved `equipment`, zero amounts only with
/// `allow_zero_entries`) and `equipment_totals` its per-archetype totals,
/// which the requirements are checked against and the save does not write.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
    /// The stored bonus trial result; completion selects the definition's
    /// bonus effect with this byte.
    pub bonus: Recorded<u8>,
    /// The stored risk-trial detection result; the completion icon reads
    /// this separately from terminal status.
    pub detected: Recorded<u8>,
    /// Exact retained effect-description bytes, built during completion and
    /// displayed in the recording client's language. They are not
    /// re-localisation inputs.
    pub effect_description: Recorded<Vec<u8>>,
    pub phases: Vec<u32>,
    pub requirements: Vec<OperationRequirement>,
    pub equipment: Vec<OperationEquipment>,
    pub equipment_totals: Vec<OperationEquipmentTotal>,
    /// The equipment pool's raw allow-zero byte (`allow_zero_entries`).
    pub equipment_allow_zero: u8,
}
typed_codec!(OperationRow, 64, |row| row.id as u128, |key| Self { id: key as u64, ..Default::default() },
 { 0 => (owner): U32, 1 => (target): U32, 2 => (target_province): OptionalU32, 3 => (definition_token): U32, 4 => (preparation_current): I64, 5 => (preparation_required): I64, 6 => (flag_40): U8, 7 => (auto_repeat): U8, 8 => (terminal): U8, 9 => (start_hour): I32, 10 => (duration_days): I32, 11 => (phase_count): U32, 12 => (assigned_slots): U32, 13 => (bonus): U8, 14 => (detected): U8, 15 => (effect_description): Bytes, 16 => (phases): U32List, 17 => (requirements): Opaque, 18 => (equipment): Opaque, 19 => (equipment_totals): Opaque, 20 => (equipment_allow_zero): U8 }, |_row| Ok(()));

/// Completion observations keyed by raw owner:sequence (96 bits).
///
/// Each hourly table is the complete drained batch, including player and AI
/// owners. It is not a cumulative census: an empty complete batch proves no
/// observed completions in that interval; missing batches never prove zero.
/// Capture reads stored completion results before the manager can delete
/// the instance. Auto-repeat instances retain their separate full handles.
/// Sequence orders observations on this branch. Invalidation cancels
/// without storing an outcome or completing; it supplies no completion event.
/// Saves retain only still-existing Operations, not this completion history.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OperationCompletionRow {
    pub owner: u32,
    pub sequence: u64,
    /// Full operation instance handle, with no content-type whitelist.
    pub id: u64,
    pub target: u32,
    /// Recorded lexical token; consumers resolve it through the content stack.
    pub definition_token: u32,
    pub completion_hour: i32,
    /// Stored terminal, bonus and detection results after completion returns.
    pub terminal: u8,
    pub bonus: u8,
    pub detected: u8,
    /// Exact description bytes in the recording client's language, including
    /// NUL and non-UTF8 bytes when description_error is zero. The reader checks
    /// the string bound. A refused description leaves the outcome intact.
    pub effect_description: Vec<u8>,
    /// Zero is exact, one denotes a historical budget refusal, two lacks the
    /// current byte-ring capacity. Writers grow from live demand and never emit
    /// the historical refusal. Refusals carry no partial description bytes.
    pub description_error: u8,
    /// Native stored string length, including when its bytes were refused.
    pub description_required: u64,
}
impl OperationCompletionRow {
    pub fn description_refusal(&self) -> Option<&'static str> {
        match self.description_error {
            1 => Some("operation completion description exceeds byte budget"),
            2 => Some("operation completion description byte ring short"),
            _ => None,
        }
    }
}
typed_codec!(OperationCompletionRow, 96, |row| (row.owner as u128) << 64 | row.sequence as u128, |key| Self { owner: (key >> 64) as u32, sequence: key as u64, ..Self::default() },
{ 0 => (id): U64, 1 => (target): U32, 2 => (definition_token): U32, 3 => (completion_hour): I32, 4 => (terminal): U8, 5 => (bonus): U8, 6 => (detected): U8, 7 => (effect_description): Bytes, 8 => (description_error): U8, 9 => (description_required): U64 }, |row| {
   if row.description_error > 2 {
       return Err(invalid("operation completion description error is unknown"));
   }
   if row.description_error == 0 {
       if row.description_required != row.effect_description.len() as u64 {
           return Err(invalid("operation completion description length differs from its stored bytes"));
       }
   } else if !row.effect_description.is_empty() {
       return Err(invalid("operation completion refused description carries bytes"));
   }
   Ok(())
});

/// OperationAssignments: key operation:ordinal (96 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OperationAssignmentRow {
    pub operation: u64,
    pub ordinal: u32,
    pub operative: u64,
    pub selection: u32,
}
typed_codec!(OperationAssignmentRow, 96, |row| (row.operation as u128) << 32 | row.ordinal as u128, |key| Self { operation: (key >> 32) as u64, ordinal: key as u32, operative: Default::default(), selection: Default::default() },
 { 0 => (operative): U64, 1 => (selection): U32 }, |_row| Ok(()));

/// CharacterRoleTraits: key character:kind:role (72 bits). Hourly.
///
/// The role trait arrays a character-scope `has_trait` searches besides
/// its unit leader's: each country-leader role (kind [`Self::COUNTRY_LEADER`],
/// `role` the role-map ideology key, CountryLeaderRoles' `ideology`), each
/// advisor role (kind [`Self::ADVISOR`], `role` its AdvisorRoles `ordinal`),
/// and the scientist role (kind [`Self::SCIENTIST`], `role` 0), whether or
/// not a roster holds the scientist. Country-leader and advisor roles hold
/// country-leader trait definitions; scientist roles hold scientist traits.
/// `tokens` are each trait's lexer token in array order; an empty role array
/// has no row. For a country-leader or advisor row, `definitions` keeps each
/// trait's database ordinal in the same order, excluding the null object.
/// A repeated name is another definition, and `has_trait` compares
/// definitions, so the token alone cannot identify a trait. A scientist row
/// has none because its database is keyed by token. The save writes each
/// role's `traits`: country-leader and scientist traits by token, advisor
/// traits by name.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CharacterRoleTraitRow {
    pub character: u32,
    pub kind: u8,
    pub role: u32,
    pub tokens: Vec<u32>,
    pub definitions: Vec<u32>,
}
impl CharacterRoleTraitRow {
    pub const COUNTRY_LEADER: u8 = 0;
    pub const ADVISOR: u8 = 1;
    pub const SCIENTIST: u8 = 2;
}
typed_codec!(CharacterRoleTraitRow, 72, |row| (row.character as u128) << 40 | (row.kind as u128) << 32 | row.role as u128,
 |key| Self { character: (key >> 40) as u32, kind: (key >> 32) as u8, role: key as u32, ..Default::default() },
 { 0 => (tokens): U32List, 1 => (definitions): U32List }, |row| {
    if row.kind > CharacterRoleTraitRow::SCIENTIST {
        return Err(invalid("character role trait kind must be country leader, advisor or scientist"));
    }
    if row.kind == CharacterRoleTraitRow::SCIENTIST && row.role != 0 {
        return Err(invalid("a character's scientist role key is zero"));
    }
    if row.tokens.is_empty() {
        return Err(invalid("a role with no traits has no CharacterRoleTraits row"));
    }
    let definitions = if row.kind == CharacterRoleTraitRow::SCIENTIST { 0 } else { row.tokens.len() };
    if row.definitions.len() != definitions {
        return Err(invalid("a country-leader or advisor trait has one definition ordinal and a scientist trait none"));
    }
    Ok(())
});

/// CharacterOperatives: key character (32 bits). Hourly.
///
/// A character's unit-leader reference, resolved through the registry when
/// its direct unit leader (UnitLeaders) is null. Creating or recruiting an
/// operative copies its identity into this reference; only a nonzero
/// reference has a row. `reference` is the stored eight bytes (the type word
/// low, the id high), saved as `operative`. `resolved` is whether the registry
/// holds its object. When it does, the character-scope trigger operands follow:
/// `category` (3 for an operative), `owner` (the raw country reference `tag`
/// compares; 0 is none), `skill_level` (`None` for a null skill object), and
/// `trait_definitions` (assigned traits' database ordinals, each an i32's
/// bits, in array order). Without the object they are zero and empty.
///
/// Character-scope `has_trait`, `skill` and `tag` consume these operands. The
/// save keeps the reference and each role's traits, but not the resolved
/// leader's operands.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CharacterOperativeRow {
    pub character: u32,
    pub reference: u64,
    pub resolved: bool,
    pub category: i32,
    pub owner: u32,
    pub skill_level: Option<i32>,
    pub trait_definitions: Vec<u32>,
    /// Stored operative codename operands; None is outside scope or a counted refusal.
    pub codename: Recorded<Option<NameOperands>>,
    /// Exact retained leader-name bytes; None is a counted capture refusal.
    pub name: Recorded<Option<Vec<u8>>>,
}
typed_codec!(CharacterOperativeRow, 32, |row| row.character as u128,
 |key| Self { character: key as u32, ..Default::default() },
 { 0 => (reference): U64, 1 => (resolved): Bool, 2 => (category): I32, 3 => (owner): U32, 4 => (skill_level): OptionalI32, 5 => (trait_definitions): U32List, 6 => (codename): Opaque, 7 => (name): Opaque }, |row| {
    if row.reference == 0 {
        return Err(invalid("a character with no unit-leader reference has no CharacterOperatives row"));
    }
    if !row.resolved
        && (row.category != 0
            || row.owner != 0
            || row.skill_level.is_some()
            || !row.trait_definitions.is_empty()
            || matches!(row.codename, Recorded::Value(Some(_)))
            || matches!(row.name, Recorded::Value(Some(_))))
    {
        return Err(invalid("an unresolved unit-leader reference has no leader operands"));
    }
    Ok(())
});

/// CountryCharacters: key country:ordinal (64 bits). Hourly.
///
/// A country's active character records in name-token order. Each is a
/// 16-byte record whose first qword points to its character. `country` is
/// the owning country's raw id, `ordinal` the record's position and
/// `character` its character's id, `None` for a null record. The
/// `has_country_leader` `character` operand takes the first record of a
/// token here. A country without a character manager has no rows. The save
/// writes the same records as `character_status`, deduplicated and ordered
/// by character reference rather than this order.
///
/// CountryRetiredCharacters (hourly) keeps each country's retired records
/// the same way: the same 16-byte records in name-token order. Retirement
/// moves an active record here. A character is retired when its country is
/// positive and a retired record's character has the same name token. The
/// save writes them as `retired_character_status`.
///
/// The oracle compares active and retired records as sets, matching the save's
/// map representation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryCharacterRow {
    pub country: u32,
    pub ordinal: u32,
    pub character: Option<u32>,
}
typed_codec!(CountryCharacterRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (character): OptionalU32 }, |_row| Ok(()));

/// AdvisorTokens: key ordinal (32 bits). Hourly.
///
/// The character manager's advisor idea-token string set, which `has_idea`
/// consults first to decide whether a name is an advisor's idea. Advisor
/// availability updates erase and insert the roles' idea tokens; loading a
/// retired character inserts all its roles' tokens. Retiring erases none,
/// and dropping a country's character lists for dynamic reuse does not
/// erase them either. Between loads the set can therefore hold tokens no
/// saved list explains. Keys are stored exactly and sorted by bytes,
/// `ordinal` being the rank: bucket order follows hashes and insertion
/// history. No writer saves the set.
///
/// The oracle reconstructs the set from the advisor-role idea tokens of
/// characters marked `advisor=yes` in any country's saved `character_status`,
/// and all characters in `retired_character_status`. This is exact at a load
/// hour; otherwise unexplained tokens are a runtime-only limitation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AdvisorTokenRow {
    pub ordinal: u32,
    pub token: Vec<u8>,
}
typed_codec!(AdvisorTokenRow, 32, |row| row.ordinal as u128, |key| Self { ordinal: key as u32, ..Default::default() },
 { 0 => (token): Bytes }, |_row| Ok(()));

/// FactionAdvisors: key faction:ordinal (96 bits). Hourly.
///
/// A faction's occupied advisor slots in its upgrade status. Slots are
/// 0x1A8 bytes, with the advisor pointer in the first qword. `faction` is
/// the full reference (Factions' `id`), `ordinal` the slot position (empty
/// slots have no row, so ordinals may skip), and `token` the exact advisor
/// idea token. `has_idea` holds for an advisor token in a slot of the
/// country's faction. The save writes `upgrades.slots` with the advisor's
/// character token, rather than its idea token.
///
/// The oracle compares slot occupancy and the saved character's advisor roles
/// to recover the idea token.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionAdvisorRow {
    pub faction: u64,
    pub ordinal: u32,
    pub token: Vec<u8>,
}
typed_codec!(FactionAdvisorRow, 96, |row| (row.faction as u128) << 32 | row.ordinal as u128, |key| Self { faction: (key >> 32) as u64, ordinal: key as u32, ..Default::default() },
 { 0 => (token): Bytes }, |_row| Ok(()));

/// OperationTokens: key owner:target (64 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OperationTokensRow {
    pub owner: u32,
    pub target: u32,
    pub tokens: Vec<u32>,
}
typed_codec!(OperationTokensRow, 64, |row| (row.owner as u128) << 32 | row.target as u128, |key| Self { owner: (key >> 32) as u32, target: key as u32, tokens: Default::default() },
 { 0 => (tokens): U32List }, |_row| Ok(()));

/// HiredAdvisors: key country:ordinal (64 bits), 3 payload mask bits.
///
/// The appointed-advisor array in stored order: `country` is the raw owner
/// reference, `character` the advisor's character id, and `idea` and `slot`
/// the advisor's exact stored strings.
///
/// Resolve the raw country reference through CountryLinks.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct HiredAdvisorRow {
    pub country: u32,
    pub ordinal: u32,
    pub character: u32,
    pub idea: Vec<u8>,
    pub slot: Vec<u8>,
}
typed_codec!(HiredAdvisorRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, character: Default::default(), idea: Default::default(), slot: Default::default() },
 { 0 => (character): U32, 1 => (idea): Bytes, 2 => (slot): Bytes }, |_row| Ok(()));

/// UnitIntel: key unit:ordinal (96 bits), 2 payload mask bits.
///
/// One entry of a unit's per-viewer level array, in stored order.
/// The entry's decay accumulator is deliberately not recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct UnitIntelRow {
    /// Full native unit handle; join it to Divisions or TaskForces.
    pub unit: u64,
    pub ordinal: u32,
    /// Stored viewer key: the country's ordinal in the engine's country array.
    pub viewer: u32,
    /// Native signed level byte; nonpositive entries are recorded as stored.
    pub level: i8,
}
typed_codec!(UnitIntelRow, 96, |row| (row.unit as u128) << 32 | row.ordinal as u128, |key| Self { unit: (key >> 32) as u64, ordinal: key as u32, viewer: Default::default(), level: Default::default() },
 { 0 => (viewer): U32, 1 => (level): I8 }, |_row| Ok(()));

/// Scientists: key character (32 bits), 5 payload mask bits.
///
/// `trait_count` and `skill_count` are the live vector counts, not capacities.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ScientistRow {
    pub character: u32,
    /// Raw country reference of the roster that holds this scientist.
    pub country: u32,
    /// Native roster vector position; roster order is engine state.
    pub ordinal: u32,
    pub trait_count: i32,
    pub skill_count: i32,
    /// Raw scientist byte, saved by token 0x385B. None is a counted capture
    /// refusal; membership and skill counts remain available.
    pub active: Recorded<Option<u8>>,
}

typed_codec!(ScientistRow, 32, |row| row.character as u128, |key| Self { character: key as u32, ..Default::default() },
 { 0 => (country): U32, 1 => (ordinal): U32, 2 => (trait_count): I32, 3 => (skill_count): I32, 4 => (active): Opaque }, |_row| Ok(()));

/// Daily scientist operands, dated by this table's supplying read.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ScientistDailyRow {
    pub character: u32,
    pub country: u32,
    /// None is an independent guarded-read refusal.
    pub injury_days: Recorded<Option<i32>>,
    /// Retained conjunction: 0 null, 1 nonempty, 2 empty (native true).
    pub template_visibility: Recorded<Option<u8>>,
    pub template_origin: Recorded<Option<ScientistTemplateOrigin>>,
}
typed_codec!(ScientistDailyRow, 32, |row| row.character as u128, |key| Self { character: key as u32, ..Default::default() },
 { 0 => (country): U32, 1 => (injury_days): OptionalI32, 2 => (template_visibility): Opaque, 3 => (template_origin): Opaque }, |_row| Ok(()));

/// Stored agency roster order. The full reference pair is retained even
/// when unresolved.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CapturedOperativeReferenceRow {
    pub country: u32,
    pub ordinal: u32,
    pub operative: u64,
    /// Resolved leader's raw country reference; None is null, never a guarded
    /// read failure.
    pub owner: Option<u32>,
}
typed_codec!(CapturedOperativeReferenceRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (operative): U64, 1 => (owner): OptionalU32 }, |_row| Ok(()));

/// ScientistSkills: key character:ordinal (64 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ScientistSkillRow {
    pub character: u32,
    pub ordinal: u32,
    /// Full native u32 specialization token, the record's first word.
    pub token: u32,
    /// Native signed level; zero is a real level, not an absence.
    pub level: i32,
    /// Stored signed experience, independent of the skill's level.
    pub experience_raw: i64,
}
typed_codec!(ScientistSkillRow, 64, |row| (row.character as u128) << 32 | row.ordinal as u128, |key| Self { character: (key >> 32) as u32, ordinal: key as u32, token: Default::default(), level: Default::default(), experience_raw: Default::default() },
 { 0 => (token): U32, 1 => (level): I32, 2 => (experience_raw): I64 }, |_row| Ok(()));
/// CountryIntel: key owner_country:target_index (64 bits), 4 payload mask bits.
///
/// The four lanes of a country's native intel array, keyed by the native array
/// index, which is the target's canonical country-link slot rather than a raw
/// reference. Fog reads army and air, the naval-base icon's port detail the navy
/// lane, and raid target intel any lane.
/// Each lane keeps its raw signed i64; the save prints the same value divided
/// by 100,000 and omits a lane only when the raw qword is exactly zero.
///
/// The table is **sparse**. A row is recorded for every record with a nonzero
/// lane, and always for the owner's last record (index `count - 1`), even when
/// every lane is zero. So in a complete table:
///
/// - an owner's native record count is its greatest recorded index plus one;
/// - an index below that count with no row has four zero lanes;
/// - an index at or beyond that count is not in the native array at all;
/// - an owner with no rows has no records (a null table or a count that is not
///   positive, for which the save writes no list).
///
/// The last-record row is the least data that keeps "beyond the count"
/// distinguishable from "zero": it costs nothing when that record is nonzero
/// and one row per owner otherwise, where a separate count would cost one per
/// owner always. A truncated table proves none of this for its missing rows.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryIntelRow {
    /// Raw country reference of the table's owner, as other tables record it.
    pub owner_country: u32,
    /// Canonical country-link slot of the target, which is the native index.
    pub target_index: u32,
    pub army: i64,
    pub air: i64,
    /// Target intel selects these lanes too.
    pub civilian: i64,
    pub navy: i64,
}
typed_codec!(CountryIntelRow, 64, |row| (row.owner_country as u128) << 32 | row.target_index as u128, |key| Self { owner_country: (key >> 32) as u32, target_index: key as u32, army: Default::default(), air: Default::default(), civilian: Default::default(), navy: Default::default() },
 { 0 => (army): I64, 1 => (air): I64, 2 => (civilian): I64, 3 => (navy): I64 }, |_row| Ok(()));

impl CountryIntelRow {
    /// Each recorded owner's native record count, from a complete table: its
    /// greatest recorded index plus one. Owners absent here have no records.
    pub fn native_counts<'a>(
        rows: impl IntoIterator<Item = &'a Self>,
    ) -> std::collections::BTreeMap<u32, u32> {
        let mut counts = std::collections::BTreeMap::new();
        for row in rows {
            let count = row.target_index.saturating_add(1);
            counts
                .entry(row.owner_country)
                .and_modify(|known: &mut u32| *known = (*known).max(count))
                .or_insert(count);
        }
        counts
    }
}

/// The country-character roster's advisor search order, read once a game day at the country's owner
/// slice; SliceReads holds each country's read hour. The primary roster is independent of hired
/// appointments and has no slot bytes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AvailableAdvisorRow {
    pub country: u32,
    pub ordinal: u32,
    pub character: u32,
    pub idea: Vec<u8>,
}
typed_codec!(AvailableAdvisorRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (character): U32, 1 => (idea): Bytes }, |_row| Ok(()));

/// Shared client agency UI buffers in the order state icons consume them. One row
/// with snapshot key zero also represents four observed empty lists. These are
/// retained UI membership, never reconstructed from saved operation instances or
/// filtered to a newly selected perspective. A tag switch may leave them stale.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AgencyOperationBuffersRow {
    pub snapshot: u8,
    pub completed: Vec<AgencyBufferedOperation>,
    pub running: Vec<AgencyBufferedOperation>,
    pub ready: Vec<AgencyBufferedOperation>,
    pub unready: Vec<AgencyBufferedOperation>,
    pub offers: Recorded<Vec<AgencyBufferedOffer>>,
}
typed_codec!(AgencyOperationBuffersRow, 8, |row| row.snapshot as u128, |key| Self { snapshot: key as u8, ..Default::default() },
 { 0 => (completed): Opaque, 1 => (running): Opaque, 2 => (ready): Opaque, 3 => (unready): Opaque, 4 => (offers): Opaque }, |_row| Ok(()));

/// Independent contribution vectors. Kinds are allied, cryptology, static,
/// dynamic, prior-day dynamic, pending dynamic values and accumulators. A final
/// ordinal header stores the native extent in `target`; the outer contribution
/// extent uses the final pool. Dense missing entries within the extent are zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IntelContributionRow {
    pub owner: u32,
    pub kind: u8,
    pub pool: u32,
    pub ordinal: u32,
    pub target: i32,
    pub civilian: i64,
    pub army: i64,
    pub navy: i64,
    pub air: i64,
}
typed_codec!(IntelContributionRow,104,|row| (row.owner as u128)<<72 | (row.kind as u128)<<64 | (row.pool as u128)<<32 | row.ordinal as u128,
 |key| Self {owner:(key>>72) as u32,kind:(key>>64) as u8,pool:(key>>32) as u32,ordinal:key as u32,..Default::default()},
 {0=>(target):I32,1=>(civilian):I64,2=>(army):I64,3=>(navy):I64,4=>(air):I64},|_row|Ok(()));

/// Static pool/source identities and ordered keyed values. Final entry ordinal
/// is a source header; final source ordinal is a pool header. The all-final key
/// is the root's independently retained discriminator and pool extents.
/// Final pool with source ordinal 0/1/2/3 is the held cheat/radar/token/network
/// source reference, keyed by its target in `ordinal` (zero for country roots).
/// No process pointer is a recording input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IntelSourceRow {
    pub owner: u32,
    pub pool: u32,
    pub source: u32,
    pub ordinal: u32,
    pub target: i32,
    pub civilian: i64,
    pub army: i64,
    pub navy: i64,
    pub air: i64,
    pub id: i32,
    pub removed: u8,
    pub next_id: i32,
    pub discriminant: u32,
    pub count: u32,
    pub other_count: u32,
    pub reference_country: u32,
    pub reference_pool: i32,
}
typed_codec!(IntelSourceRow,128,|row| (row.owner as u128)<<96 | (row.pool as u128)<<64 | (row.source as u128)<<32 | row.ordinal as u128,
 |key| Self {owner:(key>>96) as u32,pool:(key>>64) as u32,source:(key>>32) as u32,ordinal:key as u32,..Default::default()},
 {0=>(target):I32,1=>(civilian):I64,2=>(army):I64,3=>(navy):I64,4=>(air):I64,5=>(id):I32,6=>(removed):U8,7=>(next_id):I32,8=>(discriminant):U32,9=>(count):U32,10=>(other_count):U32,11=>(reference_country):U32,12=>(reference_pool):I32},|_row|Ok(()));

/// Saved network auxiliary occurrences. The stable id is allocated on workers
/// for an actual (kind, subnetwork, ordinal), never a country/state product.
/// National scope is u32::MAX. Kinds: network metadata (0), vertex/state-depth-
/// selector (1), edge endpoints (2), prior source (3), local quiet flag (4),
/// operative handle/selector (5), country record (6), subnetwork presence (7),
/// coverage/coverable core-controlled-worth counts (8/9), controlled/core sums
/// and national coverage (10), detection chance/factor (11), state/strength
/// occurrences (12), core states (13), and otherwise-unselected strength/gain/
/// target terms (14/15) and quiet (16). Existing IntelNetworks supplies selected
/// terms and per-state strengths/gains. Unused carriers are canonical zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IntelNetworkAuxiliaryRow {
    pub owner: u32,
    pub target: u32,
    pub id: u64,
    pub kind: u8,
    pub subnetwork: u32,
    pub ordinal: u64,
    pub country: u32,
    pub count_a: i32,
    pub count_b: i32,
    pub count_c: i32,
    pub first: i64,
    pub second: i64,
    pub third: i64,
}
impl IntelNetworkAuxiliaryRow {
    pub fn label(&self) -> Option<&'static str> {
        Some(match self.kind {
            0 => "Network topology",
            1 => "Vertex",
            2 => "Edge",
            3 => "Previous gain source",
            4 => "Local quiet flag",
            5 => "Operative",
            6 => "Country record",
            7 => "Subnetwork",
            8 => "Coverage",
            9 => "Total coverable",
            10 => "Strength aggregates and national coverage",
            11 => "Operative detection",
            12 => "State",
            13 => "Core state",
            14 => "Unselected subnetwork strength",
            15 => "Unselected subnetwork target",
            16 => "Unselected subnetwork quiet",
            _ => return None,
        })
    }
}
typed_codec!(IntelNetworkAuxiliaryRow,128,|row| (row.owner as u128)<<96 | (row.target as u128)<<64 | row.id as u128,
 |key| Self{owner:(key>>96) as u32,target:(key>>64) as u32,id:key as u64,..Default::default()},
 {0=>(kind):U8,1=>(subnetwork):U32,2=>(ordinal):U64,3=>(country):U32,4=>(count_a):I32,5=>(count_b):I32,6=>(count_c):I32,7=>(first):I64,8=>(second):I64,9=>(third):I64},|_row|Ok(()));