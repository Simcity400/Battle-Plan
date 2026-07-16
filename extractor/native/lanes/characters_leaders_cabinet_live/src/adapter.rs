use std::collections::{BTreeMap, BTreeSet};

use crate::domain47_characters_leaders_cabinet_live as compact;
use crate::extractor::{
    advisor_bucket, read_active_ruler_from_country, read_advisor_raw_fields,
    read_character_raw_fields, read_unit_leader_skill, read_unit_leader_trait_vector,
    AdvisorRecord, CharacterPortraitRecord, CharacterRecord, CountryLeaderRoleRecord, ExtractError,
    LaneFrame, LaneMemory, ResolvedTypeRoots, UnitLeaderRecord, UnitLeaderTraitRecord,
    CCOUNTRY_LEADER_CHARACTER_PTR_OFF, CUNIT_LEADER_CHARACTER_ID_COPY_OFF, CUNIT_LEADER_ID_OFF,
    CUNIT_LEADER_TYPE_ID, CUNIT_LEADER_TYPE_OFF,
};

use compact::{UNIT_LEADER_ARMY, UNIT_LEADER_NAVY};

pub const CUNIT_LEADER_EXPERIENCE_RAW_OFF: u64 = 0xE68;
pub const CARMY_LEADER_ATTACK_OFF: u64 = 0xF58;
pub const CARMY_LEADER_DEFENSE_OFF: u64 = 0xF68;
pub const CARMY_LEADER_PLANNING_OFF: u64 = 0xF78;
pub const CARMY_LEADER_LOGISTICS_OFF: u64 = 0xF88;
pub const CNAVY_LEADER_ATTACK_OFF: u64 = 0xF68;
pub const CNAVY_LEADER_DEFENSE_OFF: u64 = 0xF78;
pub const CNAVY_LEADER_MANEUVERING_OFF: u64 = 0xF88;
pub const CNAVY_LEADER_COORDINATION_OFF: u64 = 0xF98;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdapterInput {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub skipped_input_rows: u32,
    pub characters: Vec<CharacterRoot>,
    pub unit_leaders: Vec<UnitLeaderRoot>,
    pub unit_leader_trait_tokens: Vec<TraitDefinitionRoot>,
    pub country_leader_roles: Vec<CountryLeaderRoleRoot>,
    pub countries: Vec<CountryPoliticsRoot>,
    pub advisors: Vec<AdvisorRoot>,
    pub portraits: Vec<CharacterPortraitInput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterRoot {
    pub character_addr: u64,
    pub name: Option<String>,
    pub token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitLeaderRoot {
    pub leader_addr: u64,
    pub country_tag_db_id: u32,
    pub leader_kind: u8,
    pub name: Option<String>,
    pub token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitDefinitionRoot {
    pub trait_addr: u64,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryLeaderRoleRoot {
    pub country_leader_addr: u64,
    pub country_tag_db_id: u32,
    pub name: Option<String>,
    pub token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryPoliticsRoot {
    pub country_addr: u64,
    pub country_tag_db_id: u32,
    pub ruling_party: Option<String>,
    pub ruler_name: Option<String>,
    pub ruler_token: Option<String>,
    pub active_idea_tokens: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdvisorRoot {
    pub advisor_addr: u64,
    pub idea_token: String,
    pub slot: Option<String>,
    pub ledger: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterPortraitInput {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub role: String,
    pub size: String,
    pub gfx: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdapterBuildStats {
    pub character_roots_seen: u32,
    pub character_rows: u32,
    pub unit_leader_roots_seen: u32,
    pub unit_leader_rows: u32,
    pub unit_leader_trait_rows: u32,
    pub country_leader_roots_seen: u32,
    pub country_leader_role_rows: u32,
    pub country_roots_seen: u32,
    pub active_ruler_rows: u32,
    /// Countries whose active-ruler chain did not resolve this frame
    /// (structurally empty ruling-party leader vector — e.g. rebels — or a
    /// malformed vector). The ruler row for such a country is ABSENT, never
    /// fabricated, and this is NOT a frame-failing rejection: every emitted
    /// ruler row is individually vtable/type/tag-validated, and the live
    /// 870ebf D04 re-probe resolved 435/437 countries with two expected
    /// structural outliers.
    pub active_ruler_unresolved_roots: u32,
    pub advisor_roots_seen: u32,
    pub advisor_roster_rows: u32,
    pub appointed_advisor_rows: u32,
    pub portrait_inputs_seen: u32,
    pub portrait_rows: u32,
    pub rejected_roots: u32,
    pub missing_trait_tokens: u32,
    /// Roots whose frame-walk read failed STABLY (an immediate re-read failed
    /// bit-identically), as opposed to TORN (`rejected_roots`, the re-read
    /// disagreed). DIAGNOSTIC ATTRIBUTION ONLY — stable failures still reject
    /// the frame fail-closed, exactly like torn ones (see `has_rejections`).
    /// The 2026-07-11 adversarial gate REFUTED the earlier claim that a
    /// stable failure proves the ROOT is dead: a NESTED read failure (skill
    /// source, trait vector, referenced trait/character) can be stable while
    /// the parent root lives, and omitting a live root from a CLEAN
    /// full-replacement frame silently DELETES it downstream. Positive
    /// absence is only established when a COMPLETE sweep + prewarm rebuild no
    /// longer yields the root — never by a walk read failure. The collector's
    /// rejection-triggered invalidation fires on this counter so the root
    /// cache converges for the genuinely-dead case; row restoration under
    /// per-frame churn is the PARTIAL-upsert fold design (queued), not
    /// clean-emission-with-omissions.
    pub dead_roots: u32,
}

impl AdapterBuildStats {
    pub fn has_rejections(&self) -> bool {
        self.rejected_roots != 0 || self.missing_trait_tokens != 0 || self.dead_roots != 0
    }
}

/// Classification of a root whose frame-walk read failed.
enum RootFailure {
    /// The re-read failed bit-identically: the object is stably gone.
    Dead,
    /// The re-read disagreed (different error, or success): mid-read mutation.
    Torn,
}

/// Dead-vs-torn: re-run the SAME read after a failure and compare. Matches the
/// prewarm scan's `(Err(a), Err(b)) if a == b => stable exclusion` discipline,
/// so both layers agree on what "dead" means.
fn classify_root_failure<T, E: PartialEq>(first: &E, second: &Result<T, E>) -> RootFailure {
    match second {
        Err(e) if e == first => RootFailure::Dead,
        _ => RootFailure::Torn,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterBuildResult {
    pub frame: LaneFrame,
    pub stats: AdapterBuildStats,
}

pub fn build_lane_frame_from_adapter_input<M: LaneMemory>(
    mem: &M,
    roots: &ResolvedTypeRoots,
    input: &AdapterInput,
) -> AdapterBuildResult {
    let mut frame = LaneFrame::new(input.elapsed_wall_ms);
    frame.status_flags = input.status_flags;
    let mut stats = AdapterBuildStats::default();
    let trait_tokens = input
        .unit_leader_trait_tokens
        .iter()
        .map(|row| (row.trait_addr, row.token.as_str()))
        .collect::<BTreeMap<_, _>>();

    let mut known_characters = BTreeSet::new();
    let mut character_identity = BTreeMap::<(u32, u32), (Option<String>, Option<String>)>::new();
    stats.character_roots_seen = input.characters.len().min(u32::MAX as usize) as u32;
    for root in &input.characters {
        match read_character_raw_fields(mem, roots, root.character_addr) {
            Ok(raw) => {
                let key = (raw.country_tag_db_id, raw.char_id);
                known_characters.insert(key);
                character_identity.insert(key, (root.name.clone(), root.token.clone()));
                frame.characters.push(CharacterRecord {
                    country_tag_db_id: raw.country_tag_db_id,
                    char_id: raw.char_id,
                    name: root.name.clone(),
                    token: root.token.clone(),
                });
            }
            Err(first) => match classify_root_failure(
                &first,
                &read_character_raw_fields(mem, roots, root.character_addr),
            ) {
                RootFailure::Dead => stats.dead_roots = stats.dead_roots.saturating_add(1),
                RootFailure::Torn => stats.rejected_roots = stats.rejected_roots.saturating_add(1),
            },
        }
    }
    stats.character_rows = frame.characters.len().min(u32::MAX as usize) as u32;

    stats.unit_leader_roots_seen = input.unit_leaders.len().min(u32::MAX as usize) as u32;
    for root in &input.unit_leaders {
        match read_unit_leader(mem, roots, root, &trait_tokens) {
            Ok((leader, traits, missing_trait_tokens)) => {
                known_characters.insert((leader.country_tag_db_id, leader.char_id));
                frame.unit_leader_traits.extend(traits);
                frame.unit_leaders.push(leader);
                stats.missing_trait_tokens = stats
                    .missing_trait_tokens
                    .saturating_add(missing_trait_tokens);
            }
            Err(first) => match classify_root_failure(
                &first,
                &read_unit_leader(mem, roots, root, &trait_tokens),
            ) {
                RootFailure::Dead => stats.dead_roots = stats.dead_roots.saturating_add(1),
                RootFailure::Torn => stats.rejected_roots = stats.rejected_roots.saturating_add(1),
            },
        }
    }
    stats.unit_leader_rows = frame.unit_leaders.len().min(u32::MAX as usize) as u32;
    stats.unit_leader_trait_rows = frame.unit_leader_traits.len().min(u32::MAX as usize) as u32;

    stats.country_leader_roots_seen =
        input.country_leader_roles.len().min(u32::MAX as usize) as u32;
    for root in &input.country_leader_roles {
        match read_country_leader_role(mem, roots, root) {
            Ok(row) => {
                known_characters.insert((row.country_tag_db_id, row.char_id));
                frame.country_leader_roles.push(row);
            }
            Err(first) => {
                match classify_root_failure(&first, &read_country_leader_role(mem, roots, root)) {
                    RootFailure::Dead => stats.dead_roots = stats.dead_roots.saturating_add(1),
                    RootFailure::Torn => {
                        stats.rejected_roots = stats.rejected_roots.saturating_add(1)
                    }
                }
            }
        }
    }
    stats.country_leader_role_rows = frame.country_leader_roles.len().min(u32::MAX as usize) as u32;

    stats.country_roots_seen = input.countries.len().min(u32::MAX as usize) as u32;
    for root in &input.countries {
        match read_active_ruler_from_country(
            mem,
            roots,
            root.country_tag_db_id,
            root.country_addr,
            root.ruling_party.clone(),
            root.ruler_name.clone(),
            root.ruler_token.clone(),
        ) {
            Ok(mut row) => {
                if let Some((name, token)) =
                    character_identity.get(&(row.country_tag_db_id, row.char_id))
                {
                    if row.name.is_none() {
                        row.name = name.clone();
                    }
                    if row.token.is_none() {
                        row.token = token.clone();
                    }
                }
                known_characters.insert((row.country_tag_db_id, row.char_id));
                frame.active_rulers.push(row);
            }
            Err(_) => {
                stats.active_ruler_unresolved_roots =
                    stats.active_ruler_unresolved_roots.saturating_add(1)
            }
        }
    }
    stats.active_ruler_rows = frame.active_rulers.len().min(u32::MAX as usize) as u32;

    stats.advisor_roots_seen = input.advisors.len().min(u32::MAX as usize) as u32;
    for root in &input.advisors {
        match read_advisor_raw_fields(mem, roots, root.advisor_addr) {
            Ok(raw) => {
                known_characters.insert((raw.country_tag_db_id, raw.char_id));
                let bucket = advisor_bucket(
                    root.slot.as_deref(),
                    root.ledger.as_deref(),
                    &root.idea_token,
                );
                frame.advisor_roster.push(AdvisorRecord {
                    country_tag_db_id: raw.country_tag_db_id,
                    char_id: raw.char_id,
                    slot: root.slot.clone(),
                    ledger: Some(bucket.as_str().to_string()),
                    idea_token: root.idea_token.clone(),
                });
            }
            Err(first) => match classify_root_failure(
                &first,
                &read_advisor_raw_fields(mem, roots, root.advisor_addr),
            ) {
                RootFailure::Dead => stats.dead_roots = stats.dead_roots.saturating_add(1),
                RootFailure::Torn => stats.rejected_roots = stats.rejected_roots.saturating_add(1),
            },
        }
    }
    stats.advisor_roster_rows = frame.advisor_roster.len().min(u32::MAX as usize) as u32;
    // Appointed (currently-hired) advisors are a KNOWN-OPEN sub-scope for this
    // lane: the validated CAdvisor scan yields the full available ROSTER
    // (roster + slot + idea token, oracle-validated 9549/9549 rows, slot
    // 9547/9547 this build), but "currently appointed" needs the CAdvisor
    // hired bool, which the lane's INTEGRATION_PACKET.md keeps explicitly
    // out-of-scope (domain 14 owns politics.appointed_advisors). The exact
    // active-idea intersection below is a conservative proxy that is correct
    // when hired advisors surface as literal active-idea tokens (vanilla) and
    // legitimately empty on overhaul-mod fixtures whose hired advisors use
    // leveled variant tokens (D04: 8615 roster rows, 0 intersection). It is
    // fail-closed: it never fabricates an appointment, only under-reports.
    // The clean deliverable is roster + traits + slot; no new schema bit is
    // emitted so real captures stay at the established schema value.
    // Fail-closed (orchestrator wave-3 review): appointment needs the CAdvisor
    // hired bool (domain 14 owns politics.appointed_advisors). The active-idea
    // intersection is an UNVALIDATED proxy that yields 0 on the D04 fixture but
    // could emit unproven appointed rows on other saves and break the
    // appointed_advisor_rows==0 merge-ready/integration validators. Do NOT emit
    // it in production. `appointed_advisors_from_country_active_ideas` + its unit
    // test are retained so it can be enabled once the hired bool is validated.
    frame.appointed_advisors = Vec::new();
    stats.appointed_advisor_rows = 0;

    stats.portrait_inputs_seen = input.portraits.len().min(u32::MAX as usize) as u32;
    for portrait in &input.portraits {
        if known_characters.contains(&(portrait.country_tag_db_id, portrait.char_id)) {
            frame.character_portraits.push(CharacterPortraitRecord {
                country_tag_db_id: portrait.country_tag_db_id,
                char_id: portrait.char_id,
                role: portrait.role.clone(),
                size: portrait.size.clone(),
                gfx: portrait.gfx.clone(),
            });
        } else {
            stats.rejected_roots = stats.rejected_roots.saturating_add(1);
        }
    }
    stats.portrait_rows = frame.character_portraits.len().min(u32::MAX as usize) as u32;

    if stats.has_rejections() {
        frame.status_flags |= compact::STATUS_PARTIAL_DATA;
    }
    if input.skipped_input_rows != 0 {
        frame.status_flags |= compact::STATUS_PARTIAL_DATA;
    }
    if input.countries.is_empty() {
        frame.status_flags |= compact::STATUS_BRIDGE_UNAVAILABLE;
    }
    if input.characters.is_empty()
        && input.unit_leaders.is_empty()
        && input.country_leader_roles.is_empty()
        && input.advisors.is_empty()
    {
        frame.status_flags |= compact::STATUS_SOURCE_UNAVAILABLE;
    }

    AdapterBuildResult { frame, stats }
}

pub fn appointed_advisors_from_country_active_ideas(
    roster: &[AdvisorRecord],
    countries: &[CountryPoliticsRoot],
) -> Vec<AdvisorRecord> {
    let active_by_country = countries
        .iter()
        .map(|country| {
            (
                country.country_tag_db_id,
                country
                    .active_idea_tokens
                    .iter()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    roster
        .iter()
        .filter(|row| {
            active_by_country
                .get(&row.country_tag_db_id)
                .map(|tokens| tokens.contains(row.idea_token.as_str()))
                .unwrap_or(false)
        })
        .filter(|row| seen.insert((row.country_tag_db_id, row.char_id, row.idea_token.clone())))
        .cloned()
        .collect()
}

fn read_country_leader_role<M: LaneMemory>(
    mem: &M,
    roots: &ResolvedTypeRoots,
    root: &CountryLeaderRoleRoot,
) -> Result<CountryLeaderRoleRecord, ExtractError> {
    require_vtable(
        mem,
        root.country_leader_addr,
        roots.ccountry_leader_vtable,
        "CCountryLeader",
    )?;
    let character_addr = read_nonzero_u64(
        mem,
        root.country_leader_addr,
        CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
        "CCountryLeader.character",
    )?;
    let character = read_character_raw_fields(mem, roots, character_addr)?;
    Ok(CountryLeaderRoleRecord {
        country_tag_db_id: root.country_tag_db_id,
        char_id: character.char_id,
        name: root.name.clone(),
        token: root.token.clone(),
    })
}

fn read_unit_leader<M: LaneMemory>(
    mem: &M,
    roots: &ResolvedTypeRoots,
    root: &UnitLeaderRoot,
    trait_tokens: &BTreeMap<u64, &str>,
) -> Result<(UnitLeaderRecord, Vec<UnitLeaderTraitRecord>, u32), ExtractError> {
    let expected_vtable = match root.leader_kind {
        UNIT_LEADER_ARMY => roots.carmy_leader_vtable,
        UNIT_LEADER_NAVY => roots.cnavy_leader_vtable,
        _ => 0,
    };
    if expected_vtable == 0 {
        return Err(ExtractError::BadTypeId {
            addr: root.leader_addr,
            field: "CUnitLeader.kind",
            expected: UNIT_LEADER_ARMY as u32,
            observed: root.leader_kind as u32,
        });
    }
    require_vtable(mem, root.leader_addr, expected_vtable, "CUnitLeader")?;
    require_type_id(
        mem,
        root.leader_addr
            .checked_add(CUNIT_LEADER_TYPE_OFF)
            .ok_or(ExtractError::AddressOverflow("CUnitLeader.type"))?,
        CUNIT_LEADER_TYPE_ID,
        "CUnitLeader.type",
    )?;
    let unit_leader_id = read_u32(mem, root.leader_addr, CUNIT_LEADER_ID_OFF, "CUnitLeader.id")?;
    let char_id = read_u32(
        mem,
        root.leader_addr,
        CUNIT_LEADER_CHARACTER_ID_COPY_OFF,
        "CUnitLeader.character_id_copy",
    )?;
    let skill = read_unit_leader_skill(mem, root.leader_addr)?;
    let experience_raw = mem
        .read_i32_le(
            root.leader_addr
                .checked_add(CUNIT_LEADER_EXPERIENCE_RAW_OFF)
                .ok_or(ExtractError::AddressOverflow("CUnitLeader.experience_raw"))?,
        )
        .ok_or(ExtractError::MissingRead {
            addr: root.leader_addr + CUNIT_LEADER_EXPERIENCE_RAW_OFF,
            field: "CUnitLeader.experience_raw",
        })?;
    let mut trait_rows = Vec::new();
    let mut missing_trait_tokens = 0u32;
    if roots.cunit_leader_trait_vtable != 0 && !trait_tokens.is_empty() {
        let trait_vector = read_unit_leader_trait_vector(mem, root.leader_addr)?;
        for (ordinal, trait_addr) in trait_vector.trait_ptrs.iter().copied().enumerate() {
            require_vtable(
                mem,
                trait_addr,
                roots.cunit_leader_trait_vtable,
                "CUnitLeaderTrait",
            )?;
            if let Some(token) = trait_tokens.get(&trait_addr) {
                trait_rows.push(UnitLeaderTraitRecord {
                    unit_leader_id,
                    ordinal: ordinal.min(u16::MAX as usize) as u16,
                    trait_token: (*token).to_string(),
                });
            } else {
                missing_trait_tokens = missing_trait_tokens.saturating_add(1);
            }
        }
    }
    let stat_offsets = stat_offsets(root.leader_kind);
    let leader = UnitLeaderRecord {
        country_tag_db_id: root.country_tag_db_id,
        unit_leader_id,
        char_id,
        leader_kind: root.leader_kind,
        skill,
        experience_raw: Some(experience_raw),
        attack: read_optional_u8(mem, root.leader_addr, stat_offsets.attack),
        defense: read_optional_u8(mem, root.leader_addr, stat_offsets.defense),
        planning: read_optional_u8(mem, root.leader_addr, stat_offsets.planning),
        logistics: read_optional_u8(mem, root.leader_addr, stat_offsets.logistics),
        maneuvering: read_optional_u8(mem, root.leader_addr, stat_offsets.maneuvering),
        coordination: read_optional_u8(mem, root.leader_addr, stat_offsets.coordination),
        token: root.token.clone(),
        name: root.name.clone(),
        trait_count: trait_rows.len().min(u16::MAX as usize) as u16,
    };
    Ok((leader, trait_rows, missing_trait_tokens))
}

#[derive(Clone, Copy)]
struct StatOffsets {
    attack: Option<u64>,
    defense: Option<u64>,
    planning: Option<u64>,
    logistics: Option<u64>,
    maneuvering: Option<u64>,
    coordination: Option<u64>,
}

fn stat_offsets(leader_kind: u8) -> StatOffsets {
    match leader_kind {
        UNIT_LEADER_ARMY => StatOffsets {
            attack: Some(CARMY_LEADER_ATTACK_OFF),
            defense: Some(CARMY_LEADER_DEFENSE_OFF),
            planning: Some(CARMY_LEADER_PLANNING_OFF),
            logistics: Some(CARMY_LEADER_LOGISTICS_OFF),
            maneuvering: None,
            coordination: None,
        },
        UNIT_LEADER_NAVY => StatOffsets {
            attack: Some(CNAVY_LEADER_ATTACK_OFF),
            defense: Some(CNAVY_LEADER_DEFENSE_OFF),
            planning: None,
            logistics: None,
            maneuvering: Some(CNAVY_LEADER_MANEUVERING_OFF),
            coordination: Some(CNAVY_LEADER_COORDINATION_OFF),
        },
        _ => StatOffsets {
            attack: None,
            defense: None,
            planning: None,
            logistics: None,
            maneuvering: None,
            coordination: None,
        },
    }
}

fn read_optional_u8<M: LaneMemory>(mem: &M, base: u64, offset: Option<u64>) -> Option<u8> {
    mem.read_u8(base.checked_add(offset?)?)
}

fn require_vtable<M: LaneMemory>(
    mem: &M,
    addr: u64,
    expected: u64,
    field: &'static str,
) -> Result<(), ExtractError> {
    let observed = mem
        .read_u64_le(addr)
        .ok_or(ExtractError::MissingRead { addr, field })?;
    if observed != expected {
        return Err(ExtractError::BadVtable {
            addr,
            field,
            expected,
            observed,
        });
    }
    Ok(())
}

fn require_type_id<M: LaneMemory>(
    mem: &M,
    addr: u64,
    expected: u32,
    field: &'static str,
) -> Result<(), ExtractError> {
    let observed = mem
        .read_u32_le(addr)
        .ok_or(ExtractError::MissingRead { addr, field })?;
    if observed != expected {
        return Err(ExtractError::BadTypeId {
            addr,
            field,
            expected,
            observed,
        });
    }
    Ok(())
}

fn read_u32<M: LaneMemory>(
    mem: &M,
    base: u64,
    offset: u64,
    field: &'static str,
) -> Result<u32, ExtractError> {
    let addr = base
        .checked_add(offset)
        .ok_or(ExtractError::AddressOverflow(field))?;
    mem.read_u32_le(addr)
        .ok_or(ExtractError::MissingRead { addr, field })
}

fn read_nonzero_u64<M: LaneMemory>(
    mem: &M,
    base: u64,
    offset: u64,
    field: &'static str,
) -> Result<u64, ExtractError> {
    let addr = base
        .checked_add(offset)
        .ok_or(ExtractError::AddressOverflow(field))?;
    let value = mem
        .read_u64_le(addr)
        .ok_or(ExtractError::MissingRead { addr, field })?;
    if value == 0 {
        return Err(ExtractError::NullPointer(field));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractor::{
        self, CADVISOR_CHARACTER_PTR_OFF, CCHARACTER_COUNTRY_TAG_DB_ID_OFF, CCHARACTER_ID_OFF,
        CCHARACTER_TYPE_ID, CCHARACTER_TYPE_OFF, CCHARACTER_UNIT_LEADER_PTR_OFF,
        CCOUNTRY_POLITICAL_STATUS_OFF, CPOLITICAL_PARTY_LEADER_VEC_CAP_OFF,
        CPOLITICAL_PARTY_LEADER_VEC_OFF, CPOLITICAL_PARTY_LEADER_VEC_SIZE_OFF,
        CPOLITICAL_STATUS_RULING_PARTY_OFF, CUNIT_LEADER_SKILL_SOURCE_PTR_OFF,
        CUNIT_LEADER_SKILL_VALUE_OFF, CUNIT_LEADER_TRAIT_CAPACITY_OFF, CUNIT_LEADER_TRAIT_DATA_OFF,
        CUNIT_LEADER_TRAIT_SIZE_OFF,
    };

    const CHAR_VT: u64 = 0x1000_1000;
    const ARMY_VT: u64 = 0x1000_2000;
    const NAVY_VT: u64 = 0x1000_3000;
    const COUNTRY_LEADER_VT: u64 = 0x1000_4000;
    const ADVISOR_VT: u64 = 0x1000_5000;
    const TRAIT_VT: u64 = 0x1000_6000;

    #[derive(Default)]
    struct FakeMemory {
        bytes: BTreeMap<u64, u8>,
    }

    impl FakeMemory {
        fn write_u8(&mut self, addr: u64, value: u8) {
            self.bytes.insert(addr, value);
        }

        fn write_u32(&mut self, addr: u64, value: u32) {
            self.write_bytes(addr, &value.to_le_bytes());
        }

        fn write_i32(&mut self, addr: u64, value: i32) {
            self.write_bytes(addr, &value.to_le_bytes());
        }

        fn write_u64(&mut self, addr: u64, value: u64) {
            self.write_bytes(addr, &value.to_le_bytes());
        }

        fn write_bytes(&mut self, addr: u64, bytes: &[u8]) {
            for (idx, byte) in bytes.iter().enumerate() {
                self.bytes.insert(addr + idx as u64, *byte);
            }
        }
    }

    impl LaneMemory for FakeMemory {
        fn read_u8(&self, addr: u64) -> Option<u8> {
            self.bytes.get(&addr).copied()
        }
    }

    fn roots() -> ResolvedTypeRoots {
        ResolvedTypeRoots {
            ccharacter_vtable: CHAR_VT,
            carmy_leader_vtable: ARMY_VT,
            cnavy_leader_vtable: NAVY_VT,
            ccountry_leader_vtable: COUNTRY_LEADER_VT,
            cadvisor_vtable: ADVISOR_VT,
            cunit_leader_trait_vtable: TRAIT_VT,
        }
    }

    fn write_character(mem: &mut FakeMemory, addr: u64, char_id: u32, tag_db_id: u32) {
        mem.write_u64(addr, CHAR_VT);
        mem.write_u32(addr + CCHARACTER_TYPE_OFF, CCHARACTER_TYPE_ID);
        mem.write_u32(addr + CCHARACTER_ID_OFF, char_id);
        mem.write_u32(addr + CCHARACTER_COUNTRY_TAG_DB_ID_OFF, tag_db_id);
        mem.write_u64(addr + CCHARACTER_UNIT_LEADER_PTR_OFF, 0x2000);
    }

    #[test]
    fn adapter_builds_full_lane_frame_from_valid_roots() {
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        let leader = 0x2000;
        let skill_source = 0x2100;
        let trait_data = 0x2200;
        let trait_def = 0x2300;
        let country_leader = 0x3000;
        let country = 0x4000;
        let status = 0x4100;
        let party = 0x4200;
        let leader_vec = 0x4300;
        let advisor = 0x5000;

        write_character(&mut mem, character, 100, 2);
        mem.write_u64(leader, ARMY_VT);
        mem.write_u32(leader + CUNIT_LEADER_TYPE_OFF, CUNIT_LEADER_TYPE_ID);
        mem.write_u32(leader + CUNIT_LEADER_ID_OFF, 500);
        mem.write_u32(leader + CUNIT_LEADER_CHARACTER_ID_COPY_OFF, 100);
        mem.write_i32(leader + CUNIT_LEADER_EXPERIENCE_RAW_OFF, 42_000);
        mem.write_u64(leader + CUNIT_LEADER_SKILL_SOURCE_PTR_OFF, skill_source);
        mem.write_u32(skill_source + CUNIT_LEADER_SKILL_VALUE_OFF, 4);
        mem.write_u8(leader + CARMY_LEADER_ATTACK_OFF, 6);
        mem.write_u8(leader + CARMY_LEADER_DEFENSE_OFF, 7);
        mem.write_u8(leader + CARMY_LEADER_PLANNING_OFF, 5);
        mem.write_u8(leader + CARMY_LEADER_LOGISTICS_OFF, 8);
        mem.write_u64(leader + CUNIT_LEADER_TRAIT_DATA_OFF, trait_data);
        mem.write_u32(leader + CUNIT_LEADER_TRAIT_CAPACITY_OFF, 1);
        mem.write_u32(leader + CUNIT_LEADER_TRAIT_SIZE_OFF, 1);
        mem.write_u64(trait_data, trait_def);
        mem.write_u64(trait_def, TRAIT_VT);

        mem.write_u64(country_leader, COUNTRY_LEADER_VT);
        mem.write_u64(
            country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
            character,
        );
        mem.write_u64(country + CCOUNTRY_POLITICAL_STATUS_OFF, status);
        mem.write_u64(status + CPOLITICAL_STATUS_RULING_PARTY_OFF, party);
        mem.write_u64(party + CPOLITICAL_PARTY_LEADER_VEC_OFF, leader_vec);
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_SIZE_OFF, 1);
        mem.write_u32(party + CPOLITICAL_PARTY_LEADER_VEC_CAP_OFF, 1);
        mem.write_u64(leader_vec, country_leader);

        mem.write_u64(advisor, ADVISOR_VT);
        mem.write_u64(advisor + CADVISOR_CHARACTER_PTR_OFF, character);

        let input = AdapterInput {
            elapsed_wall_ms: 77,
            characters: vec![CharacterRoot {
                character_addr: character,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            unit_leaders: vec![UnitLeaderRoot {
                leader_addr: leader,
                country_tag_db_id: 2,
                leader_kind: UNIT_LEADER_ARMY,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            unit_leader_trait_tokens: vec![TraitDefinitionRoot {
                trait_addr: trait_def,
                token: "old_guard".to_string(),
            }],
            country_leader_roles: vec![CountryLeaderRoleRoot {
                country_leader_addr: country_leader,
                country_tag_db_id: 2,
                name: Some("Maxwell Aitken".to_string()),
                token: Some("ENG_maxwell_aitken".to_string()),
            }],
            countries: vec![
                CountryPoliticsRoot {
                    country_addr: country,
                    country_tag_db_id: 2,
                    ruling_party: Some("democratic".to_string()),
                    ruler_name: Some("Maxwell Aitken".to_string()),
                    ruler_token: Some("ENG_maxwell_aitken".to_string()),
                    active_idea_tokens: vec!["ENG_maxwell_aitken".to_string()],
                },
                CountryPoliticsRoot {
                    country_addr: country,
                    country_tag_db_id: 3,
                    ruling_party: Some("communism".to_string()),
                    ruler_name: None,
                    ruler_token: None,
                    active_idea_tokens: vec!["OTHER_shared_token".to_string()],
                },
                // A country whose ruler chain does not resolve (nothing
                // mapped at this address): the ruler row must be ABSENT and
                // counted unresolved without failing the frame.
                CountryPoliticsRoot {
                    country_addr: 0xDEAD_0000,
                    country_tag_db_id: 4,
                    ruling_party: None,
                    ruler_name: None,
                    ruler_token: None,
                    active_idea_tokens: vec![],
                },
            ],
            advisors: vec![AdvisorRoot {
                advisor_addr: advisor,
                idea_token: "ENG_maxwell_aitken".to_string(),
                slot: Some("political_advisor".to_string()),
                ledger: None,
            }],
            portraits: vec![CharacterPortraitInput {
                country_tag_db_id: 2,
                char_id: 100,
                role: "civilian".to_string(),
                size: "large".to_string(),
                gfx: "GFX_portrait_ENG_maxwell_aitken".to_string(),
            }],
            ..AdapterInput::default()
        };

        let result = build_lane_frame_from_adapter_input(&mem, &roots(), &input);
        assert_eq!(result.frame.status_flags, 0);
        assert_eq!(result.stats.character_rows, 1);
        assert_eq!(result.stats.unit_leader_rows, 1);
        assert_eq!(result.stats.unit_leader_trait_rows, 1);
        assert_eq!(result.stats.country_leader_role_rows, 1);
        // Country 2 resolves; country 3 shares country 2's object so its
        // ruler fails the tag-echo ownership guard; country 4 has no memory.
        assert_eq!(result.stats.active_ruler_rows, 1);
        assert_eq!(result.stats.active_ruler_unresolved_roots, 2);
        assert_eq!(result.stats.rejected_roots, 0);
        assert!(!result.stats.has_rejections());
        assert_eq!(result.stats.advisor_roster_rows, 1);
        // Appointed is fail-closed off in production (see build path); the
        // intersection derivation is validated directly in
        // appointed_intersection_is_country_scoped below.
        assert_eq!(result.stats.appointed_advisor_rows, 0);
        assert_eq!(result.stats.portrait_rows, 1);

        let payload = extractor::encode_frame_payload(&result.frame);
        let summary = extractor::decode_frame_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.character_portrait_rows, 1);
        assert_eq!(summary.appointed_advisor_rows, 0);
    }

    /// Memory that misses ONE address exactly once, then serves the backing
    /// bytes: models a mid-read mutation the dead-vs-torn double read must
    /// classify as TORN (the two reads disagree).
    struct HealingMemory {
        inner: FakeMemory,
        target: u64,
        miss_pending: std::cell::Cell<bool>,
    }

    impl LaneMemory for HealingMemory {
        fn read_u8(&self, addr: u64) -> Option<u8> {
            if addr == self.target && self.miss_pending.replace(false) {
                return None;
            }
            self.inner.read_u8(addr)
        }
    }

    #[test]
    fn stably_failing_root_still_rejects_the_frame_but_is_attributed_dead() {
        // One valid character plus one root with NO memory behind it: both
        // walk reads fail bit-identically. That is ATTRIBUTED as a stable
        // (dead-looking) failure — but the frame still fails closed: the
        // 2026-07-11 adversarial gate proved a stable failure does NOT prove
        // the root is dead (a nested object can stably fail while the parent
        // lives), and omitting a live root from a CLEAN full-replacement
        // frame would silently delete it downstream.
        let mut mem = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut mem, character, 100, 2);

        let input = AdapterInput {
            elapsed_wall_ms: 5,
            characters: vec![
                CharacterRoot {
                    character_addr: character,
                    name: None,
                    token: None,
                },
                CharacterRoot {
                    character_addr: 0xDEAD_0000,
                    name: None,
                    token: None,
                },
            ],
            ..AdapterInput::default()
        };

        let result = build_lane_frame_from_adapter_input(&mem, &roots(), &input);
        assert_eq!(result.stats.character_rows, 1);
        assert_eq!(result.stats.dead_roots, 1);
        assert_eq!(result.stats.rejected_roots, 0);
        assert!(
            result.stats.has_rejections(),
            "stable failures must still fail the frame closed"
        );
        assert_ne!(result.frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
    }

    #[test]
    fn torn_root_read_still_rejects_the_frame_fail_closed() {
        // The root's first read fails, the re-read succeeds (memory "healed"
        // mid-frame): the two observations disagree -> TORN -> fail-closed
        // frame rejection, exactly as before the dead-vs-torn split.
        let mut inner = FakeMemory::default();
        let character = 0x1000;
        write_character(&mut inner, character, 100, 2);
        let mem = HealingMemory {
            inner,
            target: character,
            miss_pending: std::cell::Cell::new(true),
        };

        let input = AdapterInput {
            elapsed_wall_ms: 5,
            characters: vec![CharacterRoot {
                character_addr: character,
                name: None,
                token: None,
            }],
            ..AdapterInput::default()
        };

        let result = build_lane_frame_from_adapter_input(&mem, &roots(), &input);
        assert_eq!(result.stats.dead_roots, 0);
        assert_eq!(result.stats.rejected_roots, 1);
        assert!(result.stats.has_rejections());
        assert_ne!(result.frame.status_flags & compact::STATUS_PARTIAL_DATA, 0);
    }

    #[test]
    fn appointed_intersection_is_country_scoped() {
        let roster = vec![AdvisorRecord {
            country_tag_db_id: 2,
            char_id: 100,
            slot: Some("political_advisor".to_string()),
            ledger: Some("political_advisor".to_string()),
            idea_token: "shared_token".to_string(),
        }];
        let countries = vec![CountryPoliticsRoot {
            country_addr: 0,
            country_tag_db_id: 3,
            ruling_party: None,
            ruler_name: None,
            ruler_token: None,
            active_idea_tokens: vec!["shared_token".to_string()],
        }];
        assert!(appointed_advisors_from_country_active_ideas(&roster, &countries).is_empty());
    }
}
